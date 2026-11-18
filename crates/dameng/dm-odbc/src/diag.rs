//! 诊断收集（T3.2，设计方案 §6.2，契约 CT-06 / CT-07）。
//!
//! 三条硬要求：
//!
//! 1. **一次失败可能产生多条记录**，必须全部保留、按记录号升序，不能只要第一条；
//! 2. 消息必须走**宽字符**路径取回（`SQLGetDiagRecW`），否则中文是乱码；
//! 3. 分类以**原生码**为准（§6.8），未登记的码原样落回 [`DmErrorKind::Server`]，
//!    绝不猜语义；「找不到驱动」只能靠 `SQLSTATE` 判（那时没有原生码）。
//!
//! 本模块只做两件事：`句柄 → Vec<DiagRecord>` 与 `Vec<DiagRecord> → DmError`，
//! 不碰连接状态机，也不持有任何句柄。

use dm_error::{DiagRecord, DmError, DmErrorKind, ErrorContext, Stage, sqlstate_from_str};
use odbc_api::handles::{DiagnosticResult, Diagnostics, SqlChar, SqlResult, slice_to_cow_utf8};
use odbc_sys::SqlReturn;

/// 单条诊断消息的初始缓冲（`SqlChar` 个数）。
///
/// 达梦的报错很少超过这个长度；超了 [`Diagnostics::diagnostic_record_vec`] 自己会扩容
/// 并重新取一遍，所以这个值只影响稳态分配，不影响正确性。
const MESSAGE_CAPACITY: usize = 512;

/// 收集句柄上的**全部**诊断记录，按记录号升序。
///
/// 这对任何 ODBC 句柄都成立（环境、连接、语句），`DM` 的多记录行为见 §6.2。
/// 驱动一条记录都没给时返回空 `Vec`——**这不是错误**，调用方用
/// [`to_error`] 会得到一个诚实的「无诊断」错误。
pub fn collect<H>(handle: &H) -> Vec<DiagRecord>
where
    H: Diagnostics + ?Sized,
{
    let mut out: Vec<DiagRecord> = Vec::new();
    // 复用同一个 Vec：`diagnostic_record_vec` 每轮都会 `resize` 到实际消息长度，
    // 不会累加上一轮的内容（读源码确认过），所以稳态下这里零分配。
    let mut message: Vec<SqlChar> = Vec::with_capacity(MESSAGE_CAPACITY);
    let mut rec_number: i16 = 1;
    while let Some(result) = handle.diagnostic_record_vec(rec_number, &mut message) {
        out.push(record_from(&result, &message));
        rec_number += 1;
    }
    out
}

/// 把一条 `DiagnosticResult` 与已取回的消息拼成 [`DiagRecord`]。
fn record_from(result: &DiagnosticResult, message: &[SqlChar]) -> DiagRecord {
    let sqlstate = sqlstate_from_str(result.state.as_str());
    DiagRecord::new(sqlstate, result.native_error, decode_message(message))
}

/// 宽字符 → `String`。
///
/// 驱动有时在消息中间塞 NUL（例如把消息截断后没清干净尾部），这里按第一个 NUL 截断，
/// 而不是让 NUL 直接进日志。`slice_to_cow_utf8` 是 `odbc-api` 自己的解码入口，
/// 它处理了「非 UTF-8 字节」的降级，不 panic。
pub(crate) fn decode_message(message: &[SqlChar]) -> String {
    let end = message
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(message.len());
    slice_to_cow_utf8(&message[..end]).into_owned()
}

/// 一组诊断记录 → [`DmError`]，按**第一条**记录分类（后续记录保留在 [`DmError::diags`]）。
///
/// `function` 是失败的 ODBC 调用名（如 `"SQLDriverConnect"`），只用于「驱动一条记录都没给」
/// 时的提示文本。
///
/// SQL 指纹由 T5.4（`dm-query`）负责填，本层先只带阶段。
pub fn to_error(diags: Vec<DiagRecord>, stage: Stage, function: &str) -> DmError {
    let context = ErrorContext::new(stage);
    let kind = match diags.first() {
        Some(first) => {
            DmErrorKind::classify_server(first.sqlstate, first.native_code, &first.message)
        }
        None => DmErrorKind::Server {
            sqlstate: [0u8; dm_error::SQLSTATE_LEN],
            code: 0,
            message: format!("ODBC 调用 {function} 失败，但驱动未提供任何诊断记录"),
        },
    };
    DmError::with_diags(kind, diags, context)
}

/// 把 `odbc-api` 的错误对象转成 [`DmError`]。
///
/// **手里有句柄时优先用 [`collect`]**：它能取回全部记录，而 `odbc_api::Error` 只带
/// **一条**（见 `odbc-api/src/error.rs`：`Diagnostics { record, function }`）。
///
/// 但对**建连失败**这类情况，句柄在 `odbc-api` 内部就随连接一起析构了，拿不到 `collect`
/// 的机会——那时这个函数是唯一能取回 `SQLSTATE` 与原生码的入口。所以它**必须**把
/// `record` 里的三元组原样搬过来，不能用 `err.to_string()` 顶替：真机上那样做会得到
/// `sqlstate=[0,0,0,0,0], code=0`，把 §6.8 的分类表和 §6.2 的「先看原生码」一起废掉。
pub fn from_odbc_error(err: odbc_api::Error, stage: Stage, function: &'static str) -> DmError {
    match err {
        // 这几种变体都携带一条真实记录，逐字段搬。
        odbc_api::Error::Diagnostics { record, .. }
        | odbc_api::Error::UnsupportedOdbcApiVersion(record)
        | odbc_api::Error::UnableToRepresentNull(record)
        | odbc_api::Error::OracleOdbcDriverDoesNotSupport64Bit(record)
        | odbc_api::Error::InvalidRowArraySize { record, .. } => {
            let diag = diag_from_record(&record);
            to_error(vec![diag], stage, function)
        }
        // 没有记录可搬：造一条**诚实**的记录（原生码 0、SQLSTATE 空）并把原文留在消息里，
        // 绝不编一个看起来像真的 `SQLSTATE`。
        other => {
            let diag = DiagRecord::native(0, other.to_string());
            to_error(vec![diag], stage, function)
        }
    }
}

/// `odbc_api::handles::Record` → [`DiagRecord`]（宽字符消息走同一个解码口）。
fn diag_from_record(record: &odbc_api::handles::Record) -> DiagRecord {
    DiagRecord::new(
        record.state.0,
        record.native_error,
        decode_message(&record.message),
    )
}

/// 诊断记录是否表示「驱动没装 / 数据源没配」——只有这几种情况能靠 `SQLSTATE` 判（§6.2）。
///
/// * `IM002` 未发现数据源名称且未指定默认驱动程序
/// * `IM003` 指定的驱动程序无法加载
///
/// 其余 `IMxxx` 是驱动管理器内部故障，不该伪装成「没装驱动」——它们会落回 `Server`/`Connect`。
pub fn is_driver_not_found(diags: &[DiagRecord]) -> bool {
    diags.iter().any(|rec| {
        // 用字节比较，不经过 `String`，避免为一个比较分配。
        matches!(&rec.sqlstate, b"IM002" | b"IM003")
    })
}

/// 诊断链的一行摘要，供 `tracing` 与错误消息使用。
pub fn summarize(diags: &[DiagRecord]) -> String {
    if diags.is_empty() {
        return "（无诊断记录）".to_owned();
    }
    diags
        .iter()
        .map(|rec| {
            let state = rec.sqlstate_str();
            if state.is_empty() {
                format!("[{}] {}", rec.native_code, rec.message)
            } else {
                format!("[{state}/{}] {}", rec.native_code, rec.message)
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// 本 crate 只用宽字符 API（§4.3：启用 `narrow` 会让中文变乱码）。
///
/// 这个断言把「误开 `narrow`」变成编译错误，而不是上线后的乱码。
const _: () = assert!(
    core::mem::size_of::<SqlChar>() == 2,
    "dm-odbc 必须使用 odbc-api 的宽字符 API：不得启用 narrow feature"
);

/// 供调用方一句话拿错的便捷包装：收集 + 分类。
pub fn collect_and_classify<H>(handle: &H, stage: Stage, function: &str) -> DmError
where
    H: Diagnostics + ?Sized,
{
    to_error(collect(handle), stage, function)
}

/// `SqlResult<T>` → `Result<T, DmError>`：**成功（含带警告的成功）取值**，其余一律收集诊断并分类。
///
/// 这条口径全 crate 只有这一处实现（`conn.rs` 与 `stmt.rs` 都调它），因为它编码了两个容易走偏的
/// 判断：
///
/// * `SQL_SUCCESS_WITH_INFO` **不是失败**。它表示「调用成功，但有诊断信息」（截断、驱动提示等）。
///   当成错误会拒绝一批本来能用的语句；但也不能默默丢掉，所以用 `tracing::warn!` 留痕。
///   注意：取数路径的截断**不靠返回码判**，而靠 indicator 与缓冲长度比（§6.4），
///   所以这里把 `SuccessWithInfo` 当成功不会漏掉截断。
/// * 其余四种返回码（`NoData`/`NeedData`/`StillExecuting`/`Error`）在**本 crate 的调用点上都是异常**：
///   每一个调用点该不该出现这几种码是明确的，出现了就说明我们对驱动的假设错了。
///   与其翻译成一个猜测的语义，不如把驱动自己的诊断原样交出去（§6.8 的纪律）。
pub fn sql_ok<T>(
    result: SqlResult<T>,
    handle: &(impl Diagnostics + ?Sized),
    stage: Stage,
    function: &'static str,
) -> Result<T, DmError> {
    match result {
        SqlResult::Success(value) => Ok(value),
        SqlResult::SuccessWithInfo(value) => {
            let diags = collect(handle);
            tracing::warn!(
                function,
                diagnostics = %summarize(&diags),
                "ODBC 调用成功但带诊断信息"
            );
            Ok(value)
        }
        _ => Err(collect_and_classify(handle, stage, function)),
    }
}

/// 把裸 `SqlReturn` 折成 `SqlResult<()>`，好交给 [`sql_ok`] 走统一口径。
///
/// 裸 FFI 调用（`SQLBindParameter`/`SQLGetData`/…）回的是 `SqlReturn` 而不是
/// `SqlResult<T>`。这里**只做形状转换，不做判断**——成败的规矩只有 [`sql_ok`] 一处。
///
/// `SQL_SUCCESS_WITH_INFO` 折成 `SuccessWithInfo`：取数路径的截断靠 indicator 与缓冲
/// 长度比来判（§6.4），不靠返回码，所以这个折叠不会把截断当成成功漏掉。
pub(crate) fn fold_return(ret: SqlReturn, function: &'static str) -> SqlResult<()> {
    if ret == SqlReturn::SUCCESS {
        SqlResult::Success(())
    } else if ret == SqlReturn::SUCCESS_WITH_INFO {
        SqlResult::SuccessWithInfo(())
    } else if ret == SqlReturn::NO_DATA {
        SqlResult::NoData
    } else if ret == SqlReturn::NEED_DATA {
        SqlResult::NeedData
    } else if ret == SqlReturn::STILL_EXECUTING {
        SqlResult::StillExecuting
    } else {
        SqlResult::Error { function }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dm_error::SQLSTATE_LEN;

    /// CT-10（离线侧）：整条链路必须走**宽字符** API。
    ///
    /// 文件顶部那条 `const _` 断言能在编译期挡住「误开 `narrow`」，但它**在真机套件里也看不到**
    /// ——离线门禁跑的是 `cargo test`（编译期断言确实会生效，可一旦有人为了让某个 target 编过而
    /// 把断言删掉/放宽，就没有任何用例会红）。这条用例把同一件事钉进**离线**套件，并且多守一层
    /// 契约里真正要命的宽度关系：`odbc-api` 的 `SqlChar` 与 `odbc-sys` 的 `WChar` 必须同宽，
    /// 否则我们传进去的缓冲会被驱动按另一种宽度解读（中文乱码，或越界读）。
    #[test]
    fn ct10_wide_char_api_is_two_bytes_and_utf16_round_trips() {
        assert_eq!(
            core::mem::size_of::<SqlChar>(),
            2,
            "CT-10：SqlChar 必须是 2 字节（宽字符 API）；启用 odbc-api 的 narrow 后它就是 u8"
        );
        assert_eq!(
            core::mem::size_of::<odbc_sys::WChar>(),
            2,
            "CT-10：ODBC 的 WChar 必须与 SqlChar 同宽，否则缓冲会被按错误宽度解读"
        );
        // 编码往返：BMP 中文与增补平面字符（UTF-16 代理对）都不能被拆坏。
        let text = "中文🦀";
        let units = wide(text);
        assert_eq!(units.len(), text.encode_utf16().count());
        assert_eq!(String::from_utf16(&units).expect("UTF-16 合法"), text);
    }

    /// 一条假诊断记录：`(SQLSTATE, 原生码, 消息)`。
    type Raw = ([u8; SQLSTATE_LEN], i32, String);

    /// 假 ODBC 句柄：按记录号吐出预置记录，模拟驱动行为（含结尾 NUL 与截断）。
    #[derive(Debug, Default)]
    struct FakeHandle {
        records: Vec<Raw>,
    }

    impl FakeHandle {
        fn new(records: Vec<Raw>) -> Self {
            Self { records }
        }

        fn with_state(state: &[u8; SQLSTATE_LEN], code: i32, message: &str) -> Self {
            Self::new(vec![(*state, code, message.to_owned())])
        }
    }

    /// 宽字符化。`SqlChar` 在受支持的目标上是 `u16`（上面有编译期断言）。
    fn wide(s: &str) -> Vec<SqlChar> {
        s.encode_utf16().map(|unit| unit as SqlChar).collect()
    }

    /// 由 `SQLSTATE` 字节造一个 `State`（走 `odbc-api` 的公开构造函数）。
    fn state_of(bytes: &[u8; SQLSTATE_LEN]) -> odbc_api::handles::State {
        let mut code = [0 as SqlChar; SQLSTATE_LEN + 1];
        for (slot, byte) in code.iter_mut().zip(bytes.iter()) {
            *slot = *byte as SqlChar;
        }
        odbc_api::handles::State::from_chars_with_nul(&code)
    }

    impl Diagnostics for FakeHandle {
        fn diagnostic_record(
            &self,
            rec_number: i16,
            message_text: &mut [SqlChar],
        ) -> Option<DiagnosticResult> {
            assert!(rec_number >= 1, "记录号从 1 开始");
            let (state, code, message) = self.records.get(usize::from(rec_number as u16) - 1)?;
            let text = wide(message);
            let text_length = i16::try_from(text.len()).expect("测试消息不会超过 i16");

            // 模拟驱动：写入文本 + 结尾 NUL，缓冲装不下就截断（但仍报完整长度）。
            if !message_text.is_empty() {
                let copy = text.len().min(message_text.len() - 1);
                message_text[..copy].copy_from_slice(&text[..copy]);
                message_text[copy] = 0;
            }

            Some(DiagnosticResult {
                state: state_of(state),
                native_error: *code,
                text_length,
            })
        }
    }

    #[test]
    fn collects_every_record_in_order() {
        let handle = FakeHandle::new(vec![
            (*b"42000", -2007, "第 1 条：语法分析出错".to_owned()),
            (*b"42000", -2111, "第 2 条：无效的列名 [NO_SUCH]".to_owned()),
            (*b"HY000", -70027, "第 3 条：无效的游标状态".to_owned()),
        ]);
        let diags = collect(&handle);
        assert_eq!(diags.len(), 3, "CT-06：多条记录必须全部保留");
        assert_eq!(diags[0].native_code, -2007);
        assert_eq!(diags[1].native_code, -2111);
        assert_eq!(diags[2].native_code, -70027);
        assert_eq!(diags[0].sqlstate_str(), "42000");
        assert_eq!(diags[2].sqlstate_str(), "HY000");
    }

    #[test]
    fn decodes_wide_chinese_message() {
        // CT-07：走宽字符路径，中文不得变成乱码。
        let handle = FakeHandle::with_state(b"HY000", -70005, "字符串截断：列 NAME 的字符数超限");
        let diags = collect(&handle);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "字符串截断：列 NAME 的字符数超限");
        assert!(!diags[0].message.is_ascii());
    }

    #[test]
    fn grows_buffer_for_long_message() {
        // 超过 MESSAGE_CAPACITY，逼 `diagnostic_record_vec` 扩容重取。
        let long = "达".repeat(MESSAGE_CAPACITY * 2);
        let handle = FakeHandle::with_state(b"HY000", -1, &long);
        let diags = collect(&handle);
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].message.chars().count(),
            MESSAGE_CAPACITY * 2,
            "长消息必须完整取回，不能截断"
        );
    }

    #[test]
    fn stops_at_first_missing_record() {
        let handle = FakeHandle::new(vec![(*b"01000", 0, "第一条".to_owned())]);
        let diags = collect(&handle);
        assert_eq!(diags.len(), 1, "记录 2 不存在时必须停下，而不是死循环");
    }

    #[test]
    fn absent_sqlstate_is_not_rendered_as_nul_noise() {
        // 驱动没给 SQLSTATE 时是 5 个 NUL：必须渲染成空串，不能出现 \x00。
        let handle = FakeHandle::new(vec![(
            [0u8; SQLSTATE_LEN],
            -70027,
            "无效的游标状态".to_owned(),
        )]);
        let diags = collect(&handle);
        assert_eq!(diags[0].sqlstate_str(), "");
        let text = summarize(&diags);
        assert_eq!(text, "[-70027] 无效的游标状态");
        assert!(!text.contains('\0'));
    }

    #[test]
    fn classifies_by_native_code_and_keeps_raw_triple() {
        let handle = FakeHandle::new(vec![
            (*b"42000", -2007, "语法分析出错".to_owned()),
            (*b"42000", 0, "附带的一条余信息".to_owned()),
        ]);
        let err = to_error(collect(&handle), Stage::Prepare, "SQLPrepareW");
        assert_eq!(err.kind, DmErrorKind::Syntax);
        assert_eq!(err.diags().len(), 2, "分类不得吞掉后续诊断记录");
        assert_eq!(err.diags()[1].message, "附带的一条余信息");
        assert_eq!(err.server_code_and_message(), Some((-2007, "语法分析出错")));
        assert_eq!(err.context.stage, Stage::Prepare);
    }

    #[test]
    fn unregistered_code_stays_server_error() {
        // 未登记的码必须原样保留三元组，绝不猜语义（§6.8 纪律）。
        let handle = FakeHandle::with_state(b"23000", -2601, "违反唯一约束");
        let err = to_error(collect(&handle), Stage::Execute, "SQLExecute");
        match err.kind {
            DmErrorKind::Server {
                sqlstate,
                code,
                message,
            } => {
                assert_eq!(sqlstate, *b"23000");
                assert_eq!(code, -2601);
                assert_eq!(message, "违反唯一约束");
            }
            other => panic!("未登记的码不该被分类，却得到 {other:?}"),
        }
    }

    #[test]
    fn no_diagnostics_is_still_honest() {
        let err = to_error(Vec::new(), Stage::Connect, "SQLDriverConnectW");
        match &err.kind {
            DmErrorKind::Server { code, message, .. } => {
                assert_eq!(*code, 0);
                assert!(
                    message.contains("SQLDriverConnectW"),
                    "消息里要有失败的调用名"
                );
            }
            other => panic!("无诊断时应落回 Server，却得到 {other:?}"),
        }
        assert_eq!(summarize(err.diags()), "（无诊断记录）");
    }

    #[test]
    fn detects_missing_driver_by_sqlstate_only() {
        let im002 = collect(&FakeHandle::with_state(
            b"IM002",
            0,
            "未发现数据源名称并且未指定默认驱动程序",
        ));
        assert!(is_driver_not_found(&im002));

        let im003 = collect(&FakeHandle::with_state(
            b"IM003",
            0,
            "指定的驱动程序无法加载",
        ));
        assert!(is_driver_not_found(&im003));

        // 有原生码的服务端错误绝不能被当成「没装驱动」。
        let server = collect(&FakeHandle::with_state(b"HY000", -71008, "需要更多信息"));
        assert!(!is_driver_not_found(&server));
        assert!(!is_driver_not_found(&[]));
    }

    #[test]
    fn odbc_error_fallback_keeps_text() {
        let err = from_odbc_error(
            odbc_api::Error::NoDiagnostics {
                function: "SQLGetData",
            },
            Stage::Fetch,
            "SQLGetData",
        );
        assert_eq!(err.diags().len(), 1);
        assert!(
            err.diags()[0].message.contains("SQLGetData"),
            "兜底错误也要带上失败的调用名"
        );
    }

    #[test]
    fn stops_at_embedded_nul() {
        let handle = FakeHandle::with_state(b"HY000", -1, "前半段\0后半段");
        let diags = collect(&handle);
        assert_eq!(diags[0].message, "前半段", "消息里的 NUL 之后不该进日志");
    }

    #[test]
    fn collect_and_classify_wraps_collect() {
        let handle = FakeHandle::with_state(b"22001", -70005, "字符串截断".to_owned().as_str());
        let err = collect_and_classify(&handle, Stage::Bind, "SQLBindParameter");
        assert_eq!(err.kind, DmErrorKind::Encode { column: None });
        assert_eq!(err.diags().len(), 1);
    }
}
