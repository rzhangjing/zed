//! 结果集：描述列与取值（T3.5，设计方案 §6.4 与 §7）。
//!
//! 本模块只做三件事——描述列、取一行、把 ODBC 类型译成 [`DmValue`]。三条纪律：
//!
//! 1. **类型一对一**。每种 `DataType` 有自己的 ODBC C 类型。不搞「全部按
//!    `SQL_C_WCHAR` 取回来再在客户端解析」：那会把「这列其实是二进制」这类错误
//!    从「取值当场拒绝」推迟到「用户拿到一串乱码之后再猜」，而 §7 明确要求
//!    `BLOB` 列 `get::<String>()` 必须被拒绝。
//! 2. **截断必重取**。`SQL_SUCCESS_WITH_INFO` 且指标大于缓冲时，按指标扩容再取，
//!    绝不留半截值。
//! 3. **NULL 与空串严格分开**。`indicator == SQL_NULL_DATA(-1)` 是 NULL，
//!    `indicator == 0` 是空串。两者在服务端也是两回事，客户端不许把它们糊在一起。
//!
//! ## 关于 `length_in_char`
//!
//! `SQL_DESC_OCTET_LENGTH` 的名字就说明单位是**字节**，所以文本/二进制列的
//! `column_size` 按字节解释、`length_in_char = Some(false)`；连接期探测
//! （`DmConfig::length_in_char`，§4.4/D16）若已明确服务端按字符计量，则以该
//! 声明为准——它是真机探测出来的，而列属性是驱动按自己的口径报的。
//!
//! ## 未覆盖的类型
//!
//! 达梦的 `TIMESTAMP WITH [LOCAL] TIME ZONE` 在 `dm_core::DataType` 里**没有
//! 独立变体**（§7 说它们没有标准 ODBC C 类型），因此会落到 `Unknown(code)`：
//! 本模块按原始字节返回并留一条 `warn!`。等真机确认它们到底报哪个码之后，
//! 再决定是否补一条专用路径（已登记为待验证项）。

use std::mem::size_of;
use std::sync::Arc;

use dm_core::backend::{FetchState, RowBuf};
use dm_core::{ColumnMeta, DataType, DmValue, TypeCategory};
use dm_error::{DiagRecord, DmError, DmErrorKind, ErrorContext, Stage};
use odbc_api::handles::{SqlChar, SqlResult, Statement as _};
use odbc_sys::{CDataType, Len, NO_TOTAL, NULL_DATA, Pointer, SQLGetData, SqlReturn};

use crate::diag;
use crate::param::NumericStruct;
use crate::stmt::OdbcStatement;

/// 宽字符首趟试探的**下限**：2048 个码元 = 4096 字节（§6.4 的 4 KiB 试探）。
const WIDE_CHUNK_UNITS: usize = 2048;

/// 有界列首趟试探的**上限**：64 K 码元 = 128 KiB 缓冲。
///
/// 列的声明长度可能很大（`VARCHAR(100000)`），不能照着声明分配；超过上限的列退化成
/// 「首趟 64 KiB + 按指标续取」——**BMP 文本的续取是好的**，这一点有真机证据
/// （400000 字符的 `CLOB` 分几十趟取回逐字一致）。
const WIDE_PROBE_CAP_UNITS: usize = 64 * 1024;

/// 二进制试探块：4096 字节。
const BINARY_CHUNK_BYTES: usize = 4096;

/// 超过这个位数就走文本路径取 `DECIMAL`。
///
/// `rust_decimal` 是 96 位尾数（约 28–29 位十进制）。精度超过它的值用
/// `SQL_C_NUMERIC` 取回来会溢出，只能按文本取。§7 对 `precision > 28` 的要求
/// 就是这个意思，这里把它写成一个常量而不是散在代码里的魔法数。
const DECIMAL_MAX_PRECISION: usize = 28;

/// 单个值的码元/字节上限，超过即判定驱动给了荒谬的长度。
///
/// 这不是业务限制，是**兜底**：没有它，一个坏掉的驱动报出 `i16::MAX` 的指标就能
/// 让我们按指示一路扩容下去。64 M 码元 = 128 MiB 的 UTF-16 文本，任何真实 CLOB 都够。
const MAX_VALUE_UNITS: usize = 64 * 1024 * 1024;

/// 跨行复用的一组临时缓冲。
///
/// 放在调用方手里（而不是每次 `fetch` 内部 `Vec::new()`），是为了让稳态取数
/// 不再一遍遍向分配器要同样大的块——CLOB 那一趟最贵的就是这 4 KiB 的重分配。
#[derive(Debug, Default)]
pub struct FetchScratch {
    /// 宽字符暂存（UTF-16 码元）。
    wide: Vec<u16>,
    /// 二进制暂存。
    binary: Vec<u8>,
}

impl FetchScratch {
    /// 新建一组空缓冲。
    pub fn new() -> Self {
        Self::default()
    }

    /// 预留容量，避免头几行先做几次试探性扩容。
    pub fn with_capacity(wide_units: usize, bytes: usize) -> Self {
        Self {
            wide: Vec::with_capacity(wide_units),
            binary: Vec::with_capacity(bytes),
        }
    }

    /// 当前宽字符暂存容量（码元数）。
    pub fn wide_capacity(&self) -> usize {
        self.wide.capacity()
    }

    /// 当前二进制暂存容量（字节数）。
    pub fn binary_capacity(&self) -> usize {
        self.binary.capacity()
    }
}

/// 描述结果集的全部列（`SQLNumResultCols` + 逐列元数据）。
///
/// `length_in_char` 来自连接配置（§4.4/D16 的真机探测结果）。它为 `Some(true)`
/// 时，文本列的 `column_size` 按**字符**计量。
pub fn describe_columns(
    stmt: &mut OdbcStatement<'_>,
    length_in_char: Option<bool>,
) -> Result<Arc<[ColumnMeta]>, DmError> {
    let count = sql_ok(stmt.statement().num_result_cols(), stmt, "SQLNumResultCols")?;
    // 负数只可能来自坏驱动；当成「没有结果集」而不是「若干万列」。
    let count = u16::try_from(count.max(0)).unwrap_or(u16::MAX);

    let mut columns = Vec::with_capacity(usize::from(count));
    let mut name_buffer: Vec<SqlChar> = Vec::with_capacity(128);

    for index in 1..=count {
        name_buffer.clear();
        sql_ok(
            stmt.statement().col_name(index, &mut name_buffer),
            stmt,
            "SQLColAttribute(SQL_DESC_NAME)",
        )?;
        let name = diag::decode_message(&name_buffer);

        // 用 **concise type**（`SQL_DESC_CONCISE_TYPE`）而不是 `SQL_DESC_TYPE`：
        // 按 ODBC 规范，日期/时间/区间的 `SQL_DESC_TYPE` 报的是**冗长类型**
        // `SQL_DATETIME(9)` / `SQL_INTERVAL`，只有 concise type 才给出
        // `SQL_TYPE_DATE(91)` / `SQL_TYPE_TIME(92)` / `SQL_TYPE_TIMESTAMP(93)`。
        // 真机实测（T3.5）：达梦的 DATE/TIME/TIMESTAMP 三列走 `col_type` 全部报
        // `Unknown(9)`，走 `col_concise_type` 才拿到 91/92/93 —— 这一条正是
        // `datatype.rs` 故意不映射 9 所要「倒逼看见」的东西。
        let code = sql_ok(
            stmt.statement().col_concise_type(index),
            stmt,
            "SQLColAttribute(SQL_DESC_CONCISE_TYPE)",
        )?;
        let data_type = DataType::from_odbc(code.0);

        // `odbc-api` 自己的 `Nullability` 已经把「未知」并进「可能为空」了
        // （`could_be_nullable()` 只在 `NoNulls` 时为 false），正是我们要的保守方向。
        let nullable = match stmt.statement().col_nullability(index) {
            SqlResult::Success(n) | SqlResult::SuccessWithInfo(n) => n.could_be_nullable(),
            // 问不出来不等于「一定有值」：宁可当作可空。用户真取到 NULL 时得到的是
            // 一个明确的 NullViolation，而不是我们替他做的「不可能为空」的承诺。
            _ => true,
        };

        let octet_length = optional_attribute(stmt.statement().col_octet_length(index));
        let precision = optional_attribute(stmt.statement().col_precision(index));
        let scale = optional_attribute(stmt.statement().col_scale(index));

        let mut meta = ColumnMeta::new(name, data_type).with_nullable(nullable);

        match data_type.category() {
            TypeCategory::Text | TypeCategory::Binary => {
                if let Some(len) = octet_length.and_then(positive) {
                    meta = meta.with_column_size(len);
                }
                meta = meta.with_length_in_char(length_in_char.unwrap_or(false));
            }
            TypeCategory::Decimal | TypeCategory::Float => {
                if let Some(p) = precision.and_then(positive) {
                    // 小数位问不出来时按 0 处理：这是数值列最常见的形态，而且
                    // `ColumnMeta` 的精度/小数位是一对，缺一个就没法只报精度。
                    let s = scale.and_then(non_negative_usize).unwrap_or(0);
                    meta = meta.with_precision_scale(p, s);
                }
            }
            // 整数/布尔/时间/区间的「精度」不是位数，报出去只会误导调用方。
            _ => {}
        }

        columns.push(meta);
    }

    let columns: Arc<[ColumnMeta]> = columns.into();
    // 顺手记在语句上：`Backend::columns`/`fetch` 都要在不重新问一遍列属性的前提下拿到它，
    // 而 `Backend::columns` 的签名只给 `&Self::Stmt`（不可变），问驱动是做不到的。
    stmt.set_columns(Arc::clone(&columns));
    Ok(columns)
}

/// 取一行。`into` 会被清空后装入恰好一行；没有下一行时返回
/// [`FetchState::Exhausted`] 且 `into` 保持为空。
///
/// `columns` 必须是 [`describe_columns`] 对这个结果集算出来的同一份元数据，
/// 且长度与 `into` 的列数一致。把它显式传进来（而不是只靠 `RowBuf` 的列数）
/// 是为了拿到每列的真实类型与列名：类型决定用哪个 ODBC C 类型取，列名决定报错
/// 时说的是哪一列。
pub fn fetch(
    stmt: &mut OdbcStatement<'_>,
    columns: &[ColumnMeta],
    into: &mut RowBuf,
    scratch: &mut FetchScratch,
) -> Result<FetchState, DmError> {
    let count = into.columns();
    if columns.len() != count {
        return Err(decode_error(
            "<结果集>",
            format!("列元数据有 {} 列，行缓冲是 {count} 列", columns.len()),
        ));
    }

    into.clear();

    // SAFETY: 本层从不绑定 delayed input 参数（不使用 `bind_delayed_input_parameter`），
    // 所以 `SQLFetch` 不会去解引用任何调用方提供的参数缓冲；`self.stmt` 是
    // `allocate_statement` 成功之后才存在的有效语句句柄。
    let fetched = unsafe { stmt.statement().fetch() };
    match fetched {
        SqlResult::Success(()) | SqlResult::SuccessWithInfo(()) => {}
        SqlResult::NoData => {
            // 取完了。驱动此时已经关了游标，语句对象的状态要跟着更新，否则下一次
            // 执行会白跑一次 `SQLFreeStmt(SQL_CLOSE)`（无害，但状态说真话更省事）。
            stmt.mark_cursor_closed();
            return Ok(FetchState::Exhausted);
        }
        other => {
            let _ = other;
            return Err(diag::collect_and_classify(
                stmt.statement(),
                Stage::Fetch,
                "SQLFetch",
            ));
        }
    }

    let mut row = Vec::with_capacity(count);
    for (index, meta) in columns.iter().enumerate() {
        // ODBC 的列号从 1 起，`columns` 的下标从 0 起。
        let column_index = u16::try_from(index + 1).unwrap_or(u16::MAX);
        row.push(read_value(stmt, column_index, meta, scratch)?);
    }

    into.push_row(&row)?;
    Ok(FetchState::Rows { rows: 1 })
}

/// 把裸 `SqlResult` 的失败翻译成 `DmError`。
///
/// 调用点写成「先求值、再传句柄」两步，是因为 `stmt.statement()` 要可变借用：
/// 把 `stmt.statement().col_x(i)` 直接塞进同一个调用里会和第二个参数打架。
fn sql_ok<T>(
    result: SqlResult<T>,
    stmt: &mut OdbcStatement<'_>,
    function: &'static str,
) -> Result<T, DmError> {
    diag::sql_ok(result, stmt.statement(), Stage::Fetch, function)
}

/// 取一列的值。`column_index` 从 1 起（ODBC 约定）。
fn read_value(
    stmt: &mut OdbcStatement<'_>,
    column_index: u16,
    meta: &ColumnMeta,
    scratch: &mut FetchScratch,
) -> Result<DmValue, DmError> {
    let label = &meta.name;
    match meta.data_type {
        DataType::Char
        | DataType::VarChar
        | DataType::LongVarChar
        | DataType::WChar
        | DataType::WVarChar
        | DataType::WLongVarChar
        | DataType::Guid => Ok(match read_wide(stmt, column_index, meta, scratch)? {
            Some(text) => DmValue::Text(text),
            None => DmValue::Null,
        }),

        DataType::TinyInt => fixed(stmt, column_index, label, CDataType::STinyInt)
            .map(|v| v.map_or(DmValue::Null, DmValue::TinyInt)),
        DataType::SmallInt => fixed(stmt, column_index, label, CDataType::SShort)
            .map(|v| v.map_or(DmValue::Null, DmValue::SmallInt)),
        DataType::Integer => fixed(stmt, column_index, label, CDataType::SLong)
            .map(|v| v.map_or(DmValue::Null, DmValue::Int)),
        DataType::BigInt => fixed(stmt, column_index, label, CDataType::SBigInt)
            .map(|v| v.map_or(DmValue::Null, DmValue::BigInt)),

        DataType::Real => fixed(stmt, column_index, label, CDataType::Float)
            .map(|v| v.map_or(DmValue::Null, DmValue::Real)),
        // ODBC 的 `SQL_FLOAT`(6) 是双精度，`SQL_DOUBLE`(8) 也是；只有 `SQL_REAL`(7)
        // 才是单精度。别被「FLOAT 听起来更小」骗了。
        DataType::Float | DataType::Double => fixed(stmt, column_index, label, CDataType::Double)
            .map(|v| v.map_or(DmValue::Null, DmValue::Double)),

        DataType::Bit => fixed::<u8>(stmt, column_index, label, CDataType::Bit)
            .map(|v| v.map_or(DmValue::Null, |b| DmValue::Bool(b != 0))),

        DataType::Decimal => read_decimal(stmt, column_index, meta, scratch),

        DataType::Date => {
            match fixed::<odbc_sys::Date>(stmt, column_index, label, CDataType::TypeDate)? {
                None => Ok(DmValue::Null),
                Some(raw) => chrono::NaiveDate::from_ymd_opt(
                    i32::from(raw.year),
                    u32::from(raw.month),
                    u32::from(raw.day),
                )
                .map(DmValue::Date)
                .ok_or_else(|| {
                    decode_error(
                        label,
                        format!("日期越界：{}-{}-{}", raw.year, raw.month, raw.day),
                    )
                }),
            }
        }

        DataType::Time => {
            // 与绑定侧同一个理由（见 `param.rs` 的 `type_of`/`encode_one`）：
            // `SQL_TIME_STRUCT` **没有小数秒字段**，达梦 `TIME(6)` 的微秒会被它整段
            // 吃掉。T8.9 真机探针 13c/13d 实测：服务端存的是 `14:30:45.123456`
            // （`CAST(v AS VARCHAR)` 逐字回来），走 `SQL_C_TYPE_TIME` 取回来却只有
            // `14:30:45`。改按 `SQL_C_TYPE_TIMESTAMP` 取——ODBC 的转换规则里
            // `SQL_TYPE_TIME → SQL_C_TYPE_TIMESTAMP` 把日期补成当天、**小数秒保留**，
            // 这里只要时分秒与纳秒，日期部分丢掉。
            match fixed::<odbc_sys::Timestamp>(stmt, column_index, label, CDataType::TypeTimestamp)?
            {
                None => Ok(DmValue::Null),
                Some(raw) => {
                    let nanos = raw.fraction;
                    chrono::NaiveTime::from_hms_nano_opt(
                        u32::from(raw.hour),
                        u32::from(raw.minute),
                        u32::from(raw.second),
                        nanos,
                    )
                    .map(DmValue::Time)
                    .ok_or_else(|| {
                        decode_error(
                            label,
                            format!(
                                "时间越界：{}:{}:{}.{}",
                                raw.hour, raw.minute, raw.second, nanos
                            ),
                        )
                    })
                }
            }
        }

        DataType::Timestamp => {
            match fixed::<odbc_sys::Timestamp>(stmt, column_index, label, CDataType::TypeTimestamp)?
            {
                None => Ok(DmValue::Null),
                Some(raw) => timestamp_to_value(&raw)
                    .map(DmValue::Timestamp)
                    .map_err(|e| decode_error(label, e)),
            }
        }

        DataType::Binary | DataType::VarBinary | DataType::LongVarBinary => {
            Ok(match read_binary(stmt, column_index, label, scratch)? {
                Some(bytes) => DmValue::Bytes(bytes),
                None => DmValue::Null,
            })
        }

        DataType::Unknown(code) => {
            // §7：未知类型码绝不 panic，按原始字节返回并留一条 warn。
            tracing::warn!(
                column = %label,
                odbc_code = code,
                "结果集出现未登记的 ODBC 类型码，按原始字节返回"
            );
            Ok(match read_binary(stmt, column_index, label, scratch)? {
                Some(bytes) => DmValue::Unknown(bytes),
                None => DmValue::Null,
            })
        }

        other => Err(DmError::at(
            DmErrorKind::Unsupported {
                feature: format!("取 {other} 类型的列"),
            },
            Stage::Fetch,
        )),
    }
}

/// 取一个定长值：驱动一次写满结构体。
fn fixed<T>(
    stmt: &mut OdbcStatement<'_>,
    column_index: u16,
    column: &str,
    ctype: CDataType,
) -> Result<Option<T>, DmError> {
    let mut slot = std::mem::MaybeUninit::<T>::zeroed();
    let size = std::mem::size_of::<T>();
    let mut indicator: Len = 0;
    let hstmt = stmt.statement().as_sys();

    // SAFETY: `slot` 是一个活的、可写的 `size` 字节缓冲；`indicator` 是活的 `Len`。
    // 定长 C 类型下驱动只会写 `size` 字节，不会越界。
    let ret = unsafe {
        SQLGetData(
            hstmt,
            column_index,
            ctype,
            slot.as_mut_ptr() as Pointer,
            size as Len,
            &mut indicator,
        )
    };
    sql_ok(diag::fold_return(ret, "SQLGetData"), stmt, "SQLGetData")?;

    if indicator == NULL_DATA {
        return Ok(None);
    }
    if indicator < 0 {
        return Err(decode_error(
            column,
            format!("定长列返回了无法解释的长度指标 {indicator}"),
        ));
    }
    if indicator as usize != size {
        tracing::warn!(
            column,
            expected = size,
            indicator,
            "定长列的指标与该 ODBC C 类型的大小不一致，按结构体大小解释"
        );
    }

    // SAFETY: 成功返回的 `SQLGetData` 已经写满了整个结构体。
    Ok(Some(unsafe { slot.assume_init() }))
}

/// 取 `DECIMAL`：优先 `SQL_C_NUMERIC`，位数超界或数值路径解释不了时退回文本。
///
/// 文本路径的结果交给 [`decimal_text_value`]：装得进 `rust_decimal` 是数值，
/// 装不进（`DECIMAL(38,20)` 的 38 位尾数）是原样文本——**都不是报错**。
fn read_decimal(
    stmt: &mut OdbcStatement<'_>,
    column_index: u16,
    meta: &ColumnMeta,
    scratch: &mut FetchScratch,
) -> Result<DmValue, DmError> {
    let label = meta.name.as_str();

    if meta.precision.is_some_and(|p| p > DECIMAL_MAX_PRECISION) {
        return Ok(match read_wide(stmt, column_index, meta, scratch)? {
            Some(text) => decimal_text_value(&text),
            None => DmValue::Null,
        });
    }

    match fixed::<NumericStruct>(stmt, column_index, label, CDataType::Numeric)? {
        None => Ok(DmValue::Null),
        Some(raw) => match numeric_to_decimal(&raw) {
            Ok(value) => Ok(DmValue::Decimal(value)),
            Err(reason) => {
                // 尾数超出 i128 时退回文本路径：这列本身还是读得出来的，不该因为
                // 「我们先试了数值路径」就报错。
                tracing::debug!(
                    column = label,
                    reason,
                    "SQL_C_NUMERIC 解释失败，回退文本路径"
                );
                match read_wide(stmt, column_index, meta, scratch)? {
                    Some(text) => Ok(decimal_text_value(&text)),
                    None => Ok(DmValue::Null),
                }
            }
        },
    }
}

/// 这一列的宽字符值最长会有多少个 UTF-16 码元——也就是首趟试探该一次要回多少。
///
/// 上界是这么来的：
///
/// * 其它列用 `ColumnMeta::column_size`，它是**声明长度**而不是内容长度：
///   `length_in_char == Some(false)` 时单位是字节，而 UTF-16 码元数不会超过 UTF-8
///   字节数（最坏 1 字节/字符 = 1 码元；增补平面 4 字节 = 2 码元），加 1 收尾就够；
///   单位是字符、或者还没判定出单位时按 2 倍算——一个字符最多 2 个码元。有界列按声明
///   长度一次要回，`VARCHAR(2000)` 就从三趟变一趟。
/// * `TEXT` / `CLOB` / `LONGVARCHAR`（[`DataType::LongVarChar`] / [`DataType::WLongVarChar`]）
///   在 ODBC 层没有可信的声明长度，**故意**只给下限 [`WIDE_CHUNK_UNITS`]：真机实测证明
///   增补平面字符的损坏与「分不分趟」无关（1 次重复的单趟取值也把 `🀄` 读成另一个增补
///   平面字符），给它们 128 KiB 首趟换不来正确性，只会把响亮的 `Decode` 报错换成静默
///   错字，还让每行多付一次 128 KiB 的初始化（见 §6.5 的实测记录与 [`read_wide`]）。
/// * 结果夹在 [`WIDE_CHUNK_UNITS`] 与 [`WIDE_PROBE_CAP_UNITS`] 之间：下限保住「不付大
///   缓冲」，上限防驱动报出天文数字的列宽时照着分配（内存纪律）。
fn wide_probe_units(meta: &ColumnMeta) -> usize {
    let units = match meta.data_type {
        // 声明长度不可信，不猜；首趟就按下限。
        DataType::LongVarChar | DataType::WLongVarChar => WIDE_CHUNK_UNITS,
        _ => match meta.column_size {
            Some(size) if meta.length_in_char == Some(false) => size.saturating_add(1),
            Some(size) => size.saturating_mul(2).saturating_add(1),
            None => WIDE_CHUNK_UNITS,
        },
    };
    units.clamp(WIDE_CHUNK_UNITS, WIDE_PROBE_CAP_UNITS)
}

/// 变长宽字符取值：首趟按「这一列最长能有多长」试探，之后按指标一次要回剩余
/// （单趟上限 64 KiB）。
///
/// ## 首趟为什么按列定，而不是一律 4 KiB
///
/// 有界列（`CHAR(n)`/`VARCHAR(n)`）按声明长度一次要回，省掉续取往返，代价只有几行清零；
/// 没有声明上界的 `TEXT`/`CLOB` 保持 4 KiB——见 [`wide_probe_units`] 里为什么**不**给它们
/// 大试探。续取本身对 BMP 文本是好的：400000 字符的 `CLOB` 就是这样分几十趟取回、
/// 逐字一致的。
///
/// ## 增补平面字符：已知驱动缺陷
///
/// 真机实测（§6.5）：`A🀄B` 写进 `CLOB` 后读回是 `A𖾟B`——码点被换成另一个**合法**的
/// 增补平面字符，与长度无关（1 次重复也一样）；值再长（≥ 40000 码元的非 BMP 文本）时
/// 服务端连写都拒（`-70013 转换失败`，`HY000`）。所以非 BMP 字符不承诺内容保真；要存
/// 非 BMP 文本，用 `VARBINARY`/`BLOB` 存 UTF-8 字节（字节路径逐字节精确，有真机用例）。
///
/// 短值（一趟装得下）没有任何可用信号能识别这种替换，它会被当作正常值交给调用方——
/// 这正是「不承诺内容保真」的含义。超过首趟探测量时续取会把畸形 UTF-16 交回这里，在
/// [`String::from_utf16`] 处报 `Decode`（`invalid utf-16: lone surrogate`）；宁可响亮报错，
/// 也不要把半个码元当值返回。
///
/// 想让大列也精确取数，可以用 ODBC 的「`BufferLength = 0` + 空指针只问长度」那次调用先
/// 拿准确长度，省掉「先给 4 KiB 再按指标续取」的额外往返——但那要额外依赖一条达梦未实测
/// 的驱动行为，留给 §10.4 的落地状态记着（本条优化**没有实现**）。
///
/// ## 为什么扫 NUL 而不是信指标
///
/// ODBC 对 `SQLGetData` 的 `StrLen_or_IndPtr` 有两种合法解释：字符串值的「总长度」
/// 或「本次调用后可返回的剩余长度」。达梦实测给的是**剩余长度**，但依赖这个细节
/// 会把我们绑死在一个驱动的实现选择上。成功收尾的那一次驱动一定会补宽字符 NUL，
/// 被截断的那一次缓冲一定被填满——**扫 NUL 在两种解释下都给出同一个正确边界**，
/// 所以这里用「指标给上界、NUL 定位真实末端」。
fn read_wide(
    stmt: &mut OdbcStatement<'_>,
    column_index: u16,
    meta: &ColumnMeta,
    scratch: &mut FetchScratch,
) -> Result<Option<String>, DmError> {
    let column = meta.name.as_str();
    // 首趟之后的每趟最多要多少码元。上限只是防坏驱动报天文数字时照着分配。
    const WIDE_TAIL_UNITS: usize = 64 * 1024;

    let probe = wide_probe_units(meta);
    let out = &mut scratch.wide;
    out.clear();
    let hstmt = stmt.statement().as_sys();
    let mut indicator: Len = 0;
    // 上一趟指标给出的剩余码元数（`None` = 还没跑过，或驱动回了 `SQL_NO_TOTAL`）。
    let mut pending: Option<usize> = None;

    loop {
        let start = out.len();
        if start > MAX_VALUE_UNITS {
            return Err(decode_error(
                column,
                format!("文本长度超过 {MAX_VALUE_UNITS} 码元的上限"),
            ));
        }
        let want = pending.map_or(probe, |units| units.clamp(1, WIDE_TAIL_UNITS));
        out.resize(start + want, 0);

        // SAFETY: 从 `start` 起有 `want` 个可写码元，长度按字节给出。
        let ret = unsafe {
            SQLGetData(
                hstmt,
                column_index,
                CDataType::WChar,
                out.as_mut_ptr().add(start) as Pointer,
                (want * size_of::<u16>()) as Len,
                &mut indicator,
            )
        };

        if ret == SqlReturn::NO_DATA {
            out.truncate(start);
            break;
        }
        let finished = ret == SqlReturn::SUCCESS;
        sql_ok(diag::fold_return(ret, "SQLGetData"), stmt, "SQLGetData")?;

        if indicator == NULL_DATA {
            out.clear();
            return Ok(None);
        }
        if indicator < 0 && indicator != NO_TOTAL {
            return Err(decode_error(
                column,
                format!("文本列返回了无法解释的长度指标 {indicator}"),
            ));
        }

        // 指标在 ODBC 里有两种合法解释（总长度 / 剩余长度），所以**只拿它当上界**：
        // 真实末端由宽字符 NUL 定（见模块头「扫 NUL 而不是信指标」）。
        pending = (indicator != NO_TOTAL).then(|| indicator as usize / size_of::<u16>());
        let ceiling = pending.map_or(want, |units| units.min(want));
        let written = match out[start..].iter().position(|unit| *unit == 0) {
            Some(position) => position.min(ceiling),
            None => ceiling,
        };
        out.truncate(start + written);

        if finished {
            break;
        }
        if written == 0 {
            return Err(decode_error(
                column,
                "驱动在未取完时写入了 0 个码元，无法推进",
            ));
        }
    }

    String::from_utf16(out)
        .map(Some)
        .map_err(|e| decode_error(column, format!("UTF-16 解码失败：{e}")))
}

/// 本趟 `SQLGetData` 往缓冲里写进了多少**有效字节**（二进制路径，§6.4；契约 CT-12）。
///
/// 指标语义只有这一处判断，因为 ODBC 的指标有三种合法形态、而它们**不能**按同一套解释读：
///
/// * `SQL_NO_TOTAL`（`-4`）——驱动没给长度，只能以「本趟缓冲被填满」为准；
/// * 其余**非负**值——可能是**总长度**也可能是**剩余长度**（两种解释在 ODBC 里都合法，
///   达梦走的是哪一种见 §6.4 的实测）；两种解释下「本趟写入量不超过本趟容量」都成立，
///   所以取 `min(指标, 容量)` 是安全的，剩下的量交给下一趟续取；
/// * 其余**负数**——无法解释的指标，报错而不是猜（`SQL_NULL_DATA` 也该走不到这里：
///   调用方必须先判 NULL，见 [`read_binary`]）。
fn written_units(indicator: Len, capacity: usize, column: &str) -> Result<usize, DmError> {
    if indicator == NO_TOTAL {
        return Ok(capacity);
    }
    if indicator < 0 {
        return Err(decode_error(
            column,
            format!("二进制列返回了无法解释的长度指标 {indicator}"),
        ));
    }
    Ok((indicator as usize).min(capacity))
}

/// 变长二进制取值：同样是试探 + 重取。
///
/// 二进制没有「结尾 NUL」可言，所以只能信指标；指标为 `SQL_NO_TOTAL` 时以本趟
/// 缓冲被填满为准。
fn read_binary(
    stmt: &mut OdbcStatement<'_>,
    column_index: u16,
    column: &str,
    scratch: &mut FetchScratch,
) -> Result<Option<Vec<u8>>, DmError> {
    let out = &mut scratch.binary;
    out.clear();
    let hstmt = stmt.statement().as_sys();
    let mut indicator: Len = 0;

    loop {
        let start = out.len();
        if start > MAX_VALUE_UNITS {
            return Err(decode_error(
                column,
                format!("二进制长度超过 {MAX_VALUE_UNITS} 字节的上限"),
            ));
        }
        out.resize(start + BINARY_CHUNK_BYTES, 0);

        // SAFETY: 从 `start` 起有 `BINARY_CHUNK_BYTES` 个可写字节。
        let ret = unsafe {
            SQLGetData(
                hstmt,
                column_index,
                CDataType::Binary,
                out.as_mut_ptr().add(start) as Pointer,
                BINARY_CHUNK_BYTES as Len,
                &mut indicator,
            )
        };

        if ret == SqlReturn::NO_DATA {
            out.truncate(start);
            break;
        }
        let finished = ret == SqlReturn::SUCCESS;
        sql_ok(diag::fold_return(ret, "SQLGetData"), stmt, "SQLGetData")?;

        if indicator == NULL_DATA {
            out.clear();
            return Ok(None);
        }
        // NULL 已在上一段返回；剩下的指标形态全部交给 `written_units`（CT-12 的唯一判断点）。
        let written = written_units(indicator, BINARY_CHUNK_BYTES, column)?;
        out.truncate(start + written);

        if finished {
            break;
        }
        if written == 0 {
            return Err(decode_error(
                column,
                "驱动在未取完时写入了 0 字节，无法推进",
            ));
        }
    }

    // 复制而不是 `mem::take`：`take` 会把缓冲**连同容量**一起交出去，于是下一行又从
    // 4 KiB 起步、按 `resize` 一格格涨——1 MiB 的 BLOB 每行要几百次重分配与拷贝，
    // 而 `FetchScratch` 存在的全部意义就是「跨行复用同一块缓冲」（§5.4 纪律 2）。
    // 值必须归调用方所有，所以这一次拷贝省不掉；省掉的是**下一次**的分配。
    let bytes = out.clone();
    out.clear();
    Ok(Some(bytes))
}

/// 把 `SQL_C_NUMERIC` 的结构体翻成 [`rust_decimal::Decimal`]。
///
/// `val` 是**绝对值**的小端字节串，正负号单独放在 `sign`（1 正 0 负）。这正是
/// `param.rs` 编码时的约定，取回来也是同一套。
fn numeric_to_decimal(raw: &NumericStruct) -> Result<rust_decimal::Decimal, String> {
    let magnitude = u128::from_le_bytes(raw.val);
    if magnitude > i128::MAX as u128 {
        return Err(format!("尾数 {magnitude} 超出 i128 表示范围"));
    }
    let scale = u32::from(raw.scale.max(0) as u8);
    let mut value = rust_decimal::Decimal::from_i128_with_scale(magnitude as i128, scale);
    if raw.sign == 0 {
        value = -value;
    }
    Ok(value)
}

/// 文本形式的十进制数——**精确**解析，绝不四舍五入。
///
/// 不能用 `str::parse::<Decimal>()`（= `FromStr`）：它把超出 96 位尾数的值**静默舍成**
/// 约 29 位有效数字。真机实测（Round 25 探针 24d）：`DECIMAL(38, 20)` 列里存着
/// `123456789012345678.12345678901234567890`（服务端 `CAST(… AS VARCHAR(64))` 逐字符对得上），
/// 而 `FromStr` 给出的是 `123456789012345678.12345678901`——**数字少了 9 位、调用方却看不出来**。
/// [`decimal_text_value`] 因此改用 `from_str_exact`：装得下就精确还原成数值，装不下就按原文
/// 交回文本，两边的「一个字都不丢」都可保证。
fn parse_decimal(text: &str) -> Result<rust_decimal::Decimal, String> {
    rust_decimal::Decimal::from_str_exact(text.trim())
        .map_err(|e| format!("无法把 {text:?} 解析为 DECIMAL：{e}"))
}

/// 文本形态的 `DECIMAL` → [`DmValue`]：装得进 `rust_decimal` 就还原成数值，装不进就
/// **原样按文本**交回。
///
/// [`DECIMAL_MAX_PRECISION`] 之上的列只能走文本路径（`SQL_C_NUMERIC` 会溢出），而
/// `DECIMAL(38,20)` 这类列真放得下 96 位尾数以外的值——那不是「取得出错」，是
/// 「载体不够宽」。报 `Decode` 会把一列明明读得出来的数据变成读不出来；按文本交回
/// 则**一个字都不丢**，调用方（`toasty-driver-dameng` 的 `BigDecimal` 解码、以及
/// `RawSqlRet::Infer` 的文本回退）都还能拿到精确的十进制字符串。
fn decimal_text_value(text: &str) -> DmValue {
    match parse_decimal(text) {
        Ok(value) => DmValue::Decimal(value),
        Err(reason) => {
            tracing::debug!(
                text,
                reason,
                "DECIMAL 的文本形态超出 rust_decimal，原样按文本返回"
            );
            DmValue::Text(text.trim().to_owned())
        }
    }
}

fn timestamp_to_value(raw: &odbc_sys::Timestamp) -> Result<chrono::NaiveDateTime, String> {
    let date = chrono::NaiveDate::from_ymd_opt(
        i32::from(raw.year),
        u32::from(raw.month),
        u32::from(raw.day),
    )
    .ok_or_else(|| format!("日期越界：{}-{}-{}", raw.year, raw.month, raw.day))?;
    // `SQL_TIMESTAMP_STRUCT.fraction` 的单位是**纳秒**（ODBC 规范：0..=999_999_999），
    // 而 `and_hms_micro_opt` 要的是微秒。真机实测（T3.5）证明达梦按规范填纳秒：
    // 服务端存 `...07.123456` 时该字段是 123_456_000，直接当微秒传会「时间越界」。
    let micros = raw.fraction / 1_000;
    date.and_hms_micro_opt(
        u32::from(raw.hour),
        u32::from(raw.minute),
        u32::from(raw.second),
        micros,
    )
    .ok_or_else(|| {
        format!(
            "时间越界：{}:{}:{} + {micros}µs（ODBC 纳秒字段 {}）",
            raw.hour, raw.minute, raw.second, raw.fraction
        )
    })
}

/// 取不出来的值一律是 `Decode { column }` @ `Stage::Fetch`，细节放进诊断记录：
/// 调用方既能拿到列名（可编程），又能拿到人话（可排障）。
fn decode_error(column: &str, detail: impl std::fmt::Display) -> DmError {
    DmError::with_diags(
        DmErrorKind::Decode {
            column: Some(column.to_owned()),
        },
        vec![DiagRecord::native(0, detail.to_string())],
        ErrorContext::new(Stage::Fetch),
    )
}

fn positive(value: isize) -> Option<usize> {
    usize::try_from(value).ok().filter(|v| *v > 0)
}

/// 列属性问不出来时返回 `None`——「不知道」不该冒充「是 0」。
fn optional_attribute(result: SqlResult<isize>) -> Option<isize> {
    match result {
        SqlResult::Success(v) | SqlResult::SuccessWithInfo(v) => Some(v),
        _ => None,
    }
}

fn non_negative_usize(value: isize) -> Option<usize> {
    usize::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numeric(scale: i8, sign: u8, magnitude: u128) -> NumericStruct {
        NumericStruct {
            precision: 20,
            scale,
            sign,
            val: magnitude.to_le_bytes(),
        }
    }

    #[test]
    fn numeric_sign_is_a_separate_field_not_twos_complement() {
        let positive = numeric(2, 1, 12345);
        let negative = numeric(2, 0, 12345);
        assert_eq!(numeric_to_decimal(&positive).unwrap().to_string(), "123.45");
        assert_eq!(
            numeric_to_decimal(&negative).unwrap().to_string(),
            "-123.45"
        );
    }

    #[test]
    fn numeric_scale_zero_is_an_integer() {
        assert_eq!(
            numeric_to_decimal(&numeric(0, 1, 7)).unwrap().to_string(),
            "7"
        );
    }

    #[test]
    fn numeric_rejects_magnitudes_that_do_not_fit_i128() {
        // 最高位置 1 的尾数超出 i128 能表示的正数范围。
        assert!(numeric_to_decimal(&numeric(0, 1, 1u128 << 127)).is_err());
    }

    #[test]
    fn decimal_text_path_parses_high_precision() {
        // 27 位整数 + 2 位小数：超出 f64 的精确范围，但仍在 rust_decimal 的
        // 96 位尾数之内——正是「必须走文本路径」的那一类值。
        let value = parse_decimal("123456789012345678901234567.12").unwrap();
        assert_eq!(value.to_string(), "123456789012345678901234567.12");
    }

    #[test]
    fn decimal_text_path_reports_the_offending_text() {
        let err = parse_decimal("不是数").unwrap_err();
        assert!(err.contains("不是数"), "{err}");
    }

    #[test]
    fn decimal_text_beyond_rust_decimal_is_text_not_an_error() {
        // 38 位整数部分 + 20 位小数：`DECIMAL(38,20)` 放得下，`rust_decimal` 的
        // 96 位尾数放不下。这种值必须原样按文本交回，而不是报 `Decode`。
        let text = "123456789012345678.12345678901234567890";
        assert!(
            parse_decimal(text).is_err(),
            "前提：该值装不进 rust_decimal"
        );
        assert_eq!(decimal_text_value(text), DmValue::Text(text.to_owned()));
    }

    #[test]
    fn decimal_text_that_fits_stays_numeric() {
        // 边界的两侧都在这一条里：能装下的仍旧是 Decimal（去掉首尾空白）。
        assert_eq!(
            decimal_text_value(" 2.25 "),
            DmValue::Decimal(rust_decimal::Decimal::new(225, 2))
        );
    }

    #[test]
    fn timestamp_fraction_is_nanoseconds_not_microseconds() {
        // ODBC 规范与真机实测（T3.5）：`fraction` 是纳秒。123_456_000 ns = 123456 µs。
        let raw = odbc_sys::Timestamp {
            year: 2024,
            month: 3,
            day: 4,
            hour: 5,
            minute: 6,
            second: 7,
            fraction: 123_456_000,
        };
        let value = timestamp_to_value(&raw).unwrap();
        assert_eq!(value.to_string(), "2024-03-04 05:06:07.123456");
    }

    #[test]
    fn sub_microsecond_fractions_are_truncated_not_rejected() {
        // 纳秒精度比 `chrono` 的微秒高；999 ns 截断成 0 µs，而不是报错。
        let raw = odbc_sys::Timestamp {
            year: 2024,
            month: 3,
            day: 4,
            hour: 5,
            minute: 6,
            second: 7,
            fraction: 999,
        };
        let value = timestamp_to_value(&raw).unwrap();
        assert_eq!(value.to_string(), "2024-03-04 05:06:07");
    }

    #[test]
    fn zero_dates_are_rejected_instead_of_silently_becoming_an_epoch() {
        let raw = odbc_sys::Timestamp {
            year: 0,
            month: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            fraction: 0,
        };
        assert!(timestamp_to_value(&raw).is_err());
    }

    #[test]
    fn negative_column_attributes_are_treated_as_absent() {
        assert_eq!(positive(-1), None);
        assert_eq!(positive(0), None);
        assert_eq!(positive(255), Some(255));
        assert_eq!(non_negative_usize(0), Some(0));
        assert_eq!(non_negative_usize(-1), None);
    }

    #[test]
    fn decode_errors_carry_the_column_and_the_detail() {
        let err = decode_error("第 2 列", "日期越界：0-0-0");
        assert_eq!(err.context.stage, Stage::Fetch);
        match &err.kind {
            DmErrorKind::Decode { column } => assert_eq!(column.as_deref(), Some("第 2 列")),
            other => panic!("期望 Decode，实际 {other:?}"),
        }
        assert_eq!(err.diags().len(), 1);
        assert!(err.diags()[0].message.contains("日期越界"));
    }

    #[test]
    fn wide_and_binary_chunks_are_four_kibibytes() {
        assert_eq!(WIDE_CHUNK_UNITS * size_of::<u16>(), 4096);
        assert_eq!(BINARY_CHUNK_BYTES, 4096);
    }

    /// CT-12：分块重取的**唯一**判断点（`written_units`）对每种指标形态都给对了答案。
    ///
    /// 这条用例存在的意义是：§10.2 原写「断言 `TruncationInfo` 被正确识别并重取」，而
    /// `TruncationInfo` 是 `odbc-api` 的 `BinColumn` 才有的东西——本项目**没有**走
    /// `BinColumn`（自实现分块 `SQLGetData`，见 `read_binary`），所以判据改成断言这里。
    #[test]
    fn ct12_written_units_interprets_every_indicator_shape() {
        // 指标 = 本趟容量（不管它是「总长度」还是「剩余长度」）⇒ 本趟写满。
        assert_eq!(written_units(4096, 4096, "第 1 列").unwrap(), 4096);
        // 指标 > 容量 ⇒ 只写满本趟，剩下的交给下一趟续取（§6.4「截断必重取」）。
        assert_eq!(written_units(64 * 1024, 4096, "第 1 列").unwrap(), 4096);
        // 指标 < 容量 ⇒ 本趟就只写了这么多。
        assert_eq!(written_units(100, 4096, "第 1 列").unwrap(), 100);
        // 指标 = 0 是空值而不是 NULL：本趟 0 字节，循环再走一趟。
        assert_eq!(written_units(0, 4096, "第 1 列").unwrap(), 0);
        // SQL_NO_TOTAL：驱动没给长度，以「本趟缓冲被填满」为准。
        assert_eq!(written_units(NO_TOTAL, 4096, "第 1 列").unwrap(), 4096);
        // 其余负指标：报错而不是猜，且错误必须指到列、阶段必须是 Fetch。
        for bad in [-2 as Len, -3, -1024] {
            let err = written_units(bad, 4096, "第 2 列").unwrap_err();
            assert_eq!(err.context.stage, Stage::Fetch);
            match &err.kind {
                DmErrorKind::Decode { column } => assert_eq!(column.as_deref(), Some("第 2 列")),
                other => panic!("期望 Decode，实际 {other:?}"),
            }
            assert!(err.diags()[0].message.contains("无法解释"), "{err:?}");
        }
        // SQL_NULL_DATA 必须由调用方**先**消化：这里如实报错，不静默当 0 字节。
        assert!(written_units(NULL_DATA, 4096, "第 1 列").is_err());
    }

    #[test]
    fn scratch_can_be_reused_across_rows() {
        let mut scratch = FetchScratch::with_capacity(WIDE_CHUNK_UNITS, BINARY_CHUNK_BYTES);
        assert!(scratch.wide_capacity() >= WIDE_CHUNK_UNITS);
        assert!(scratch.binary_capacity() >= BINARY_CHUNK_BYTES);
        let wide_before = scratch.wide_capacity();
        scratch.wide.clear();
        scratch.binary.clear();
        assert_eq!(scratch.wide_capacity(), wide_before, "clear 不该丢容量");
    }
}
