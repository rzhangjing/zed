//! 错误码目录（M6 / T6.5）：§6.8 那张**实测表**的机器可读版本。
//!
//! §6.8 的表是写给人看的；这个模块的存在是为了让机器替我们守三件事（全部由
//! `crates/dm-odbc/tests/integration/catalog.rs` 的**离线**用例执行，不需要达梦在跑）：
//!
//! 1. `docs/errors-catalog.json` 与这里的 [`ENTRIES`] **逐字节一致**——改了代码没改文档，
//!    CI 红；
//! 2. [`crate::codes::REGISTERED`] 里的每个常量在目录里都有条目——新增了码却忘了记，
//!    CI 红；
//! 3. 条目声称的分类**真的就是**分类器的行为：`classified == true` 的条目喂给
//!    [`crate::DmErrorKind::classify_server`] 必须得到同名的 [`crate::DmErrorKind`]，
//!    `classified == false` 的必须原样落回 [`crate::DmErrorKind::Server`]。
//!
//! 第 3 条是最有价值的一条：它把「文档说它归到 `Syntax`」变成「分类器**就是**给它
//! `Syntax`」，不再依赖谁去读表。真机侧由 `crates/dm-odbc/tests/integration/errors.rs`
//! 的用例钉住（每个条目的 `covered_by` 写的就是用例名）；本模块只保证
//! 「代码 / 文档 / 分类器」三者不漂移。
//!
//! **未登记的码为什么也进目录**：调用方排障时最需要的信息恰好是「这个生码是什么意思、
//! 为什么没被分类」。`-6602`/`-70013` 这类**故意不登记**是一个决定，决定也要有地方记。
//!
//! JSON 里的 `retryable` 列（§10.3 那张采集表要求的「可重试」）**不另存一份**：它是
//! `classification`（本模块私有）现算出来的，所以它与分类器不可能分叉。

use crate::{DmErrorKind, SQLSTATE_LEN};

/// 目录里的一个错误码条目。
///
/// 字段顺序就是 `docs/errors-catalog.json` 里的键顺序（同一个来源，不两处维护）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    /// 观测到的 `SQLSTATE`（5 个 ASCII 字节）。**没观测到就写空串**，不编。
    pub sqlstate: &'static str,
    /// 达梦原生错误码。只有 `SQLSTATE` 没有原生码的（`IM002`）写 `0`。
    pub native_code: i32,
    /// 分类器给出的 [`DmErrorKind`] 变体名；`classified == false` 时固定是 `"Server"`。
    pub kind: &'static str,
    /// 是否登记（登记 = 分类器给语义；未登记 = 原样落回 `Server`）。
    pub classified: bool,
    /// 是否在真机上实测过。**没实测过的码不许进分类器**，只允许留在目录里当线索。
    pub verified: bool,
    /// 服务端原文（过长时截断，保留能 grep 的头部）。未逐字收录的写说明而不是编造。
    pub detail: &'static str,
    /// 怎么触发它。
    pub trigger: &'static str,
    /// 钉住它的用例名（空串 = 目录里还没有对应用例，只靠文档）。
    pub covered_by: &'static str,
}

/// 目录正文。**改动这里就必须重新生成 `docs/errors-catalog.json`**（见模块文档）。
pub const ENTRIES: &[CatalogEntry] = &[
    CatalogEntry {
        sqlstate: "",
        native_code: -70027,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "无效的游标状态",
        trigger: "游标未关时再次执行同一语句 / 对非查询语句调用 fetch。\
                  由 dm-odbc 的语句对象层自理（执行前先关游标），集成套件断言它零出现（AC-05）",
        covered_by: "",
    },
    CatalogEntry {
        sqlstate: "42000",
        native_code: -2007,
        kind: "Syntax",
        classified: true,
        verified: true,
        detail: "语法分析出错",
        trigger: "反引号 / [a] 方括号 / GENERATED … AS IDENTITY / ON DUPLICATE KEY UPDATE；\
                  DM8 亦不支持 RELEASE SAVEPOINT（M5 实测，dm-query 把它翻成 Unsupported）",
        covered_by: "errors_syntax_error_is_syntax",
    },
    CatalogEntry {
        sqlstate: "42S22",
        native_code: -2111,
        kind: "ObjectNotFound",
        classified: true,
        verified: true,
        detail: "无效的列名",
        trigger: "双引号包小写标识符（引号是大小写精确匹配）/ 查询不存在的列。\
                  SQLSTATE 42S22 由 M6 的 `examples/errors_probe.rs` 采到",
        covered_by: "errors_column_not_found_is_object_not_found",
    },
    CatalogEntry {
        sqlstate: "42S02",
        native_code: -2106,
        kind: "ObjectNotFound",
        classified: true,
        verified: true,
        detail: "第1 行附近出现错误: 无效的表或视图名[…]",
        trigger: "查询不存在的表或视图（表用 -2106，列才用 -2111）",
        covered_by: "errors_table_not_found_is_object_not_found",
    },
    CatalogEntry {
        sqlstate: "22001",
        native_code: -70005,
        kind: "Encode",
        classified: true,
        verified: true,
        detail: "字符串截断",
        trigger: "绑定的参数超过列的声明长度。阶段是 Execute 而不是 Bind：\
                  一条 INSERT … VALUES (?) 的参数没有列元数据可查",
        covered_by: "errors_string_truncation_is_encode",
    },
    CatalogEntry {
        sqlstate: "HYC00",
        native_code: -70054,
        kind: "Unsupported",
        classified: true,
        verified: true,
        detail: "特性未实现",
        trigger: "设置 SQL_ATTR_ASYNC_ENABLE（达梦 ODBC 完全不支持异步）",
        covered_by: "",
    },
    CatalogEntry {
        sqlstate: "HY000",
        native_code: -71008,
        kind: "Connect",
        classified: true,
        verified: true,
        detail: "需要更多信息",
        trigger: "连接串缺 SERVER / UID（二者同时为空时）。这正是 DmConfig 必须有 \
                  validate_connectable() 的原因：放任到驱动只会得到读不懂的 -71008",
        covered_by: "",
    },
    CatalogEntry {
        sqlstate: "IM002",
        native_code: 0,
        kind: "Connect",
        classified: true,
        verified: true,
        detail: "未发现数据源名称并且未指定默认驱动程序",
        trigger: "DSN 未注册（没有原生码，只有 SQLSTATE；分类器按 SQLSTATE 分支认它）",
        covered_by: "",
    },
    CatalogEntry {
        sqlstate: "23000",
        native_code: -6602,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "原文未逐字收录（达梦的主键冲突与唯一约束冲突都用这个码，不是 Oracle 风格的 -2601）",
        trigger: "插入重复主键 / 重复唯一键。故意不登记：宁可原样落回 Server，也不猜语义",
        covered_by: "errors_constraint_violations_fall_back_to_server",
    },
    CatalogEntry {
        sqlstate: "23000",
        native_code: -6609,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "违反列[…]非空约束",
        trigger: "向 NOT NULL 列插 NULL。故意不登记（同上）",
        covered_by: "errors_not_null_and_check_violations_fall_back_to_server",
    },
    CatalogEntry {
        sqlstate: "07002",
        native_code: -70039,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "绑定的参数个数不正确",
        trigger: "SQLExecute 时绑定个数少于 ? 个数。达梦**不**回 SQL_NEED_DATA（T3.3 实测）；\
                  语句对象的本地守卫挡在正常路径前面，所以故意不登记",
        covered_by: "params_unbound_execution_reports_the_server_code",
    },
    CatalogEntry {
        sqlstate: "HY000",
        native_code: -6804,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "需要更多的参数",
        trigger: "SQLExecDirect 下发带 ? 的语句（同 -70039，故意不登记）",
        covered_by: "params_exec_direct_with_placeholder_reports_the_server_code",
    },
    CatalogEntry {
        sqlstate: "HY000",
        native_code: -70013,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "转换失败",
        trigger: "宽字符参数写增补平面字符（代理对）且值大到 40000 个 UTF-16 码元附近时，\
                  服务端直接拒绝（M6/V-08 实测）；BMP 文本没有这个上限。\
                  该生码目前只由 `lob.rs` 的观测探针记录，没有硬断言它的用例",
        covered_by: "",
    },
    CatalogEntry {
        sqlstate: "HY000",
        native_code: -6409,
        kind: "Timeout",
        classified: true,
        verified: true,
        detail: "锁等待",
        trigger: "一条连接开着事务锁住某行，另一条连接对同一行下 `select … for update nowait`\
                  （M6/T6.5 实测，由 `examples/errors_probe.rs` 采到）。\
                  等超时的路径报同一个码。登记成 Timeout：连接本身是好的，调用方只能「稍后重试」\
                  或「放弃」，与语句超时同构",
        covered_by: "errors_lock_wait_is_timeout",
    },
    CatalogEntry {
        sqlstate: "22018",
        native_code: -6111,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "字符串转换出错",
        trigger: "把不能转的字符串往数值列塞（`insert into … (N) values ('abc')`）。\
                  故意不登记：`DmErrorKind::DataTypeMismatch` 装的是客户端侧的 expect/got，\
                  服务端这句话里没有这些信息，硬填就是编造",
        covered_by: "errors_unregistered_conversion_codes_stay_server",
    },
    CatalogEntry {
        sqlstate: "22007",
        native_code: -6118,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "非法的时间日期类型数据",
        trigger: "把 `'not-a-date'` 写进 DATE 列。故意不登记（同 -6111）",
        covered_by: "errors_unregistered_conversion_codes_stay_server",
    },
    CatalogEntry {
        sqlstate: "22012",
        native_code: -6103,
        kind: "Server",
        classified: false,
        verified: true,
        detail: "除0错误",
        trigger: "`select 1/0 from dual`（常量折叠）。故意不登记：`DmErrorKind` 里没有、\
                  也不该为它新加「算术错误」变体（§6.12 冻结：不新增变体）",
        covered_by: "errors_unregistered_conversion_codes_stay_server",
    },
];

/// [`DmErrorKind`] 的变体名。
///
/// 用**穷尽匹配**（无 `_` 臂）写：新增变体会直接编译失败，从而强制作者回来看一眼目录
/// 要不要跟进。这与 [`DmErrorKind::is_connection_lost`] 的纪律是同一条。
pub fn kind_name(kind: &DmErrorKind) -> &'static str {
    match kind {
        DmErrorKind::DriverNotFound { .. } => "DriverNotFound",
        DmErrorKind::Config(_) => "Config",
        DmErrorKind::Connect { .. } => "Connect",
        DmErrorKind::Io => "Io",
        DmErrorKind::PoolTimeout => "PoolTimeout",
        DmErrorKind::ConnectionLost => "ConnectionLost",
        DmErrorKind::Syntax => "Syntax",
        DmErrorKind::ObjectNotFound => "ObjectNotFound",
        DmErrorKind::ConstraintViolation => "ConstraintViolation",
        DmErrorKind::DuplicateKey => "DuplicateKey",
        DmErrorKind::Deadlock => "Deadlock",
        DmErrorKind::SerializationFailure => "SerializationFailure",
        DmErrorKind::Timeout => "Timeout",
        DmErrorKind::ReadOnlyTransaction => "ReadOnlyTransaction",
        DmErrorKind::Encode { .. } => "Encode",
        DmErrorKind::Decode { .. } => "Decode",
        DmErrorKind::NullViolation { .. } => "NullViolation",
        DmErrorKind::DataTypeMismatch { .. } => "DataTypeMismatch",
        DmErrorKind::CharsetMismatch { .. } => "CharsetMismatch",
        DmErrorKind::Unsupported { .. } => "Unsupported",
        DmErrorKind::Server { .. } => "Server",
    }
}

/// 生成 `docs/errors-catalog.json` 的正文（含结尾换行）。
///
/// 手写而不是引 `serde_json`：这个 crate 的纪律是「除 `thiserror` 不依赖任何东西」，
/// 而这里要序列化的结构是编译期常量，形状固定到不值得为它拉一棵依赖树。
pub fn render_json() -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("{\n");
    out.push_str("  \"generated_by\": \"crates/dm-error/src/catalog.rs\",\n");
    out.push_str("  \"design_doc\": \"docs/设计方案.md §6.8\",\n");
    out.push_str(
        "  \"note\": \"本文件由 dm_error::catalog::render_json() 生成，禁止手改；\
         改目录或分类器后跑 `DM_CATALOG_UPDATE=1 cargo test -p dm-odbc --test integration catalog` 重新生成。\",\n",
    );
    out.push_str(&format!("  \"entry_count\": {},\n", ENTRIES.len()));
    out.push_str("  \"entries\": [\n");
    for (index, entry) in ENTRIES.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"sqlstate\": {},\n",
            json_string(entry.sqlstate)
        ));
        out.push_str(&format!("      \"native_code\": {},\n", entry.native_code));
        out.push_str(&format!("      \"kind\": {},\n", json_string(entry.kind)));
        out.push_str(&format!("      \"classified\": {},\n", entry.classified));
        out.push_str(&format!(
            "      \"retryable\": {},\n",
            classification(entry).is_retryable_kind()
        ));
        out.push_str(&format!("      \"verified\": {},\n", entry.verified));
        out.push_str(&format!(
            "      \"detail\": {},\n",
            json_string(entry.detail)
        ));
        out.push_str(&format!(
            "      \"trigger\": {},\n",
            json_string(entry.trigger)
        ));
        out.push_str(&format!(
            "      \"covered_by\": {}\n",
            json_string(entry.covered_by)
        ));
        out.push_str("    }");
        if index + 1 < ENTRIES.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// 条目对应的分类器结果。
///
/// `render_json` 里的 `retryable` 列与离线用例里的分类断言都从这里算——**不另存一份**，
/// 于是「目录说的」与「分类器做的」在结构上就不可能分叉。
fn classification(entry: &CatalogEntry) -> DmErrorKind {
    DmErrorKind::classify_server(
        sqlstate_bytes(entry.sqlstate),
        entry.native_code,
        entry.detail,
    )
}

/// 把条目的 `SQLSTATE` 文本变成分类器要的 5 字节（空串 = 未观测到 = 全零）。
fn sqlstate_bytes(text: &str) -> [u8; SQLSTATE_LEN] {
    if text.is_empty() {
        [0u8; SQLSTATE_LEN]
    } else {
        crate::sqlstate_from_str(text)
    }
}

/// 把一段 UTF-8 文本渲染成 JSON 字符串字面量（含两侧引号）。
/// 中文按 UTF-8 原样写出，**不**转义成 `\uXXXX`：这份文件要能被人直接读、直接 grep。
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目录里同一个 `(SQLSTATE, 原生码)` 只能出现一次——重复条目意味着两处维护，
    /// 而不一致时「谁是真的」就没有答案了。
    #[test]
    fn entries_are_unique_by_code() {
        for (index, entry) in ENTRIES.iter().enumerate() {
            for other in &ENTRIES[index + 1..] {
                assert!(
                    entry.native_code != other.native_code || entry.sqlstate != other.sqlstate,
                    "重复的目录条目：{} / {}",
                    entry.native_code,
                    entry.sqlstate
                );
            }
        }
    }

    /// 登记的条目必须真机验过；目录里允许留未验的线索，但不许把它们说成已登记。
    #[test]
    fn classified_entries_are_verified() {
        for entry in ENTRIES {
            assert!(
                !entry.classified || entry.verified,
                "{} 声称已登记却未在真机验过",
                entry.native_code
            );
            assert!(
                entry.classified || entry.kind == "Server",
                "{} 未登记，kind 必须原样是 Server",
                entry.native_code
            );
        }
    }

    /// `render_json` 是纯函数：同样的目录必须每次渲染出同样的字节。
    #[test]
    fn rendering_is_stable_and_covers_every_code() {
        let rendered = render_json();
        assert_eq!(rendered, render_json(), "两次渲染不一致");
        assert!(rendered.ends_with("}\n"), "必须有结尾换行");
        for entry in ENTRIES {
            assert!(
                rendered.contains(&format!("\"native_code\": {}", entry.native_code)),
                "渲染结果里没有 {}",
                entry.native_code
            );
        }
    }

    /// 转义：引号、反斜杠、控制字符。
    #[test]
    fn json_escaping_keeps_text_greppable() {
        assert_eq!(json_string("普通文本"), "\"普通文本\"");
        assert_eq!(json_string("引号\"与\\反斜杠"), "\"引号\\\"与\\\\反斜杠\"");
        assert_eq!(json_string("换行\n制表\t"), "\"换行\\n制表\\t\"");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }

    /// 变体名是**穷尽**映射：这里逐个数一遍，漏了会编译失败。
    #[test]
    fn kind_names_are_exhaustive() {
        let names = [
            kind_name(&DmErrorKind::Io),
            kind_name(&DmErrorKind::Syntax),
            kind_name(&DmErrorKind::Encode { column: None }),
            kind_name(&DmErrorKind::Server {
                sqlstate: *b"HY000",
                code: -70013,
                message: "转换失败".to_owned(),
            }),
        ];
        assert_eq!(names, ["Io", "Syntax", "Encode", "Server"]);
    }
}
