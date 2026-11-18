//! # dm-error —— 达梦中间件的统一错误模型
//!
//! 这个 crate 是整条依赖链的最底层（`dm-error ← dm-core ← dm-odbc ← dm-pool ← dm-query`），
//! 因此**不依赖任何东西**（除 `thiserror`），特别是不依赖 `tokio` 与 `odbc-api`。
//!
//! ## 设计要点
//!
//! 1. **错误分类与连接池的硬契约**：每个错误都必须能回答两个**互不蕴含**的问题——
//!    [`DmErrorKind::is_connection_lost`]（连接是否已不可复用，必须丢弃而不是回池）与
//!    [`DmError::is_retryable`]（能否在新的连接/事务上重试）。这两个判定用
//!    **穷尽 `match`（无 `_` 臂）**实现，新增变体会直接编译失败，从而强制作者表态。
//! 2. **默认不自动重试**：达梦没有 PostgreSQL 那样清晰的 `serialization_failure` 约定，
//!    自动重试的误判代价（重复写入）高于收益。只提供显式的重试入口。
//! 3. **诊断链完整保留**：ODBC 可以一次返回多条诊断记录，全部收进
//!    [`DmError::diags`]；`Display` 只渲染主错误，细节按需展开。
//!
//! 对应的设计章节：`docs/设计方案.md` §6.8。

#![forbid(unsafe_code)]

pub mod catalog;

use std::fmt;

/// ODBC `SQLSTATE` 的固定长度（5 个 ASCII 字节，无结尾 NUL）。
pub const SQLSTATE_LEN: usize = 5;

/// 把 `SQLSTATE` 的原始字节渲染成字符串，**无损**。
///
/// 规则：可打印 ASCII（`0x20..=0x7E`，**含空格**）原样输出，其余一律写成 `\xNN`。
/// ODBC 的 `SQLSTATE` 不足 5 位时右补空格（见 [`sqlstate_from_str`]），
/// 所以空格必须保持可读；而全零（驱动未提供）由 [`DiagRecord::sqlstate_str`] 判定为空。
pub fn sqlstate_to_string(sqlstate: &[u8; SQLSTATE_LEN]) -> String {
    let mut out = String::with_capacity(SQLSTATE_LEN);
    for &b in sqlstate {
        if (0x20..=0x7E).contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02X}"));
        }
    }
    out
}

/// `SQLSTATE` 是否「驱动未提供」。
///
/// 达梦/ODBC 在「没有 `SQLSTATE`」时给的是 5 个 NUL 字节（也有实现给全空格），
/// 两者都视为未提供。判定必须看**原始字节**，不能在 `sqlstate_to_string` 之后再判断——
/// 那时全零已经变成 `\x00\x00\x00\x00\x00` 这样的可打印文本，`trim()` 去不掉了。
pub fn sqlstate_is_absent(sqlstate: &[u8; SQLSTATE_LEN]) -> bool {
    sqlstate.iter().all(|&b| b == 0 || b == b' ')
}

/// 供显示用的 `SQLSTATE`：未提供时返回**空串**，否则返回转义后的字符串（未 trim）。
pub fn sqlstate_display(sqlstate: &[u8; SQLSTATE_LEN]) -> String {
    if sqlstate_is_absent(sqlstate) {
        String::new()
    } else {
        sqlstate_to_string(sqlstate)
    }
}

/// 从字符串解析 `SQLSTATE`。不足 5 位右补空格，超长截断。
pub fn sqlstate_from_str(s: &str) -> [u8; SQLSTATE_LEN] {
    let mut out = [b' '; SQLSTATE_LEN];
    for (slot, b) in out.iter_mut().zip(s.bytes()) {
        *slot = b;
    }
    out
}

// ---------------------------------------------------------------------------
// 诊断记录
// ---------------------------------------------------------------------------

/// 一条诊断记录，对应 ODBC 的 `SQLGetDiagRec` + `SQLGetDiagField`。
///
/// 达梦的一次失败可能产生**多条**这样的记录，全部按顺序保留在 [`DmError::diags`] 中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagRecord {
    /// 5 字节 `SQLSTATE`，例如 `b"42000"`。全零表示驱动未提供。
    pub sqlstate: [u8; SQLSTATE_LEN],
    /// 驱动/服务端原生错误码，例如 `-2007`。`0` 表示未提供。
    pub native_code: i32,
    /// 面向人的错误消息（宽字符路径取回，已是合法 UTF-8）。
    pub message: String,
    /// 出错列号（1-based），仅部分诊断字段提供。
    pub column: Option<i32>,
    /// 出错行号（1-based），仅部分诊断字段提供。
    pub row: Option<i32>,
}

impl DiagRecord {
    /// 用最小信息构造一条诊断记录。
    pub fn new(sqlstate: [u8; SQLSTATE_LEN], native_code: i32, message: impl Into<String>) -> Self {
        Self {
            sqlstate,
            native_code,
            message: message.into(),
            column: None,
            row: None,
        }
    }

    /// 只有原生码与消息（驱动未给 `SQLSTATE` 时的常见形态）。
    pub fn native(native_code: i32, message: impl Into<String>) -> Self {
        Self::new([0u8; SQLSTATE_LEN], native_code, message)
    }

    /// 补上列号/行号，链式构造。
    pub fn at(mut self, column: Option<i32>, row: Option<i32>) -> Self {
        self.column = column;
        self.row = row;
        self
    }

    /// `SQLSTATE` 的字符串形式；**驱动未提供时返回空串**。
    ///
    /// 达梦/ODBC 在「没有 `SQLSTATE`」时给的是 5 个 NUL 字节（也有实现给全空格），
    /// 两者都视为「未提供」，以免渲染出 `\x00\x00\x00\x00\x00` 这种噪声。
    pub fn sqlstate_str(&self) -> String {
        sqlstate_display(&self.sqlstate)
    }
}

impl fmt::Display for DiagRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ss = self.sqlstate_str();
        let ss = ss.trim();
        if ss.is_empty() {
            write!(f, "[{}] {}", self.native_code, self.message)?;
        } else {
            write!(f, "[{ss}/{}] {}", self.native_code, self.message)?;
        }
        match (self.column, self.row) {
            (Some(c), Some(r)) => write!(f, "（第 {r} 行第 {c} 列）"),
            (Some(c), None) => write!(f, "（第 {c} 列）"),
            (None, Some(r)) => write!(f, "（第 {r} 行）"),
            (None, None) => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// 错误发生的阶段
// ---------------------------------------------------------------------------

/// 错误发生的阶段。同一个 `SQLSTATE` 在不同阶段要走不同处理路径
/// （例如 `-70005 字符串截断` 在 `Bind` 阶段是「参数超长」，在 `Fetch` 阶段是「缓冲太小」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    /// 配置解析/校验。
    Config,
    /// 驱动注册探测与装载。
    DriverLoad,
    /// 建立连接。
    Connect,
    /// 预处理语句。
    Prepare,
    /// 参数绑定。
    Bind,
    /// 执行。
    Execute,
    /// 取结果。
    Fetch,
    /// 归还前的复位。
    Reset,
    /// 关闭句柄。
    Close,
    /// 未归类。
    #[default]
    Other,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Stage::Config => "配置",
            Stage::DriverLoad => "驱动装载",
            Stage::Connect => "建立连接",
            Stage::Prepare => "预处理",
            Stage::Bind => "参数绑定",
            Stage::Execute => "执行",
            Stage::Fetch => "取结果",
            Stage::Reset => "连接复位",
            Stage::Close => "关闭句柄",
            Stage::Other => "其他",
        };
        f.write_str(s)
    }
}

/// 错误的附加上下文。全部字段都有合理的默认值，便于在边界处补齐。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ErrorContext {
    /// 发生阶段。
    pub stage: Stage,
    /// SQL 指纹（去掉字面量后的规范化 SQL），**不含原始参数值**，可安全进日志。
    pub sql_fingerprint: Option<String>,
    /// 该错误是否发生在**语句级**（而非连接级）。决定超时能否安全重试。
    pub stmt_level: bool,
    /// 调用方是否声明该语句**幂等**。只有幂等语句才允许由中间件自动重试。
    pub idempotent: bool,
}

impl ErrorContext {
    pub fn new(stage: Stage) -> Self {
        Self {
            stage,
            ..Default::default()
        }
    }

    pub fn with_sql(mut self, fingerprint: impl Into<String>) -> Self {
        self.sql_fingerprint = Some(fingerprint.into());
        self
    }

    pub fn stmt_level(mut self) -> Self {
        self.stmt_level = true;
        self
    }

    pub fn idempotent(mut self) -> Self {
        self.idempotent = true;
        self
    }
}

// ---------------------------------------------------------------------------
// 错误种类
// ---------------------------------------------------------------------------

/// 达梦中间件的错误种类。**新增变体时必须同时更新 [`DmErrorKind::is_connection_lost`]
/// 与 [`DmErrorKind::is_retryable_kind`]**——它们没有 `_` 兜底臂，漏了会编译失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmErrorKind {
    // ---- 装载 / 配置 ----
    /// 在预期位置都找不到 ODBC 驱动。`tried` 是探测过的全部候选，`hint` 是修复建议。
    DriverNotFound { tried: Vec<String>, hint: String },
    /// 配置非法（端口越界、`max_size = 0`、`idle_timeout >= max_lifetime`、驱动名为空……）。
    Config(String),

    // ---- 连接 ----
    /// 建连失败（服务端拒绝、认证失败、网络不通）。
    Connect { source: String },
    /// 底层 I/O 错误：连接已经不可信。
    Io,
    /// 在 `acquire_timeout` 内没能从池里拿到连接。
    PoolTimeout,
    /// 连接**曾可用**但已断开（服务端踢会话、网络中断）。
    ConnectionLost,

    // ---- 服务端 ----
    /// SQL 语法错误（`42000/-2007`）。
    Syntax,
    /// 对象不存在：表/列/视图等（`-2106 无效的表或视图名`、`-2111 无效的列名`）。
    ObjectNotFound,
    /// 违反约束（外键、CHECK、NOT NULL 等）。
    ConstraintViolation,
    /// 主键或唯一键冲突。
    DuplicateKey,
    /// 死锁，已被服务端回滚。
    Deadlock,
    /// 序列化失败（隔离级别冲突）。
    SerializationFailure,
    /// 语句超时；**也包括行锁等待失败**（`HY000 / -6409 锁等待`，M6 真机实测）。
    ///
    /// 锁等待失败为什么算超时：调用方唯一能做的事就是「等一会儿再试」或者「放弃」，
    /// 与语句超时完全同构。它不是 `ConnectionLost`——连接本身是好的，`reset()` 后回池。
    Timeout,
    /// 在只读事务中执行了写操作。
    ReadOnlyTransaction,

    // ---- 编解码 ----
    /// 参数编码失败（`22001/-70005 字符串截断` 归此类）。
    Encode { column: Option<String> },
    /// 结果解码失败。
    Decode { column: Option<String> },
    /// 值为 `NULL`，但目标 Rust 类型不是 `Option`。
    NullViolation { column: String },
    /// 类型不匹配。**绝不静默转换**：该拒绝的必须拒绝。
    DataTypeMismatch { expect: String, got: String },
    /// 连接期字符集自检失败（`配置值 != select unicode()` 的实际值）。
    ///
    /// 码页用 `i32` 而不是 `i16`：Windows 码页最大是 **65001（UTF-8）**，
    /// 超出 `i16` 范围。达梦 `DSQL_ATTR_LOCAL_CODE` 也是 4 字节。
    CharsetMismatch { configured: i32, actual: i32 },

    // ---- 其他 ----
    /// 明确不支持的特性（如 ODBC 异步 `HYC00/-70054`）。绝不静默降级。
    Unsupported { feature: String },
    /// 未被上面任何一类覆盖的服务端错误，保留原始三元组。
    Server {
        sqlstate: [u8; SQLSTATE_LEN],
        code: i32,
        message: String,
    },
}

impl DmErrorKind {
    /// **连接是否已经不可复用**（`true` ⇒ 池必须丢弃连接，不做 `reset()`、不回池）。
    ///
    /// 穷尽匹配，无 `_` 臂。
    pub fn is_connection_lost(&self) -> bool {
        match self {
            // 连接已经不可信 / 已断开：必须丢弃。
            DmErrorKind::Io | DmErrorKind::ConnectionLost => true,

            // 以下是「连接本身仍然健康」的错误，走 reset() 后回池。
            //
            // 注意 `Connect`：它表示**新建连接失败**，此时并不存在一条需要丢弃的池内连接，
            // 因此是 false（可重试），而不是 true。
            DmErrorKind::DriverNotFound { .. }
            | DmErrorKind::Config(_)
            | DmErrorKind::Connect { .. }
            | DmErrorKind::PoolTimeout
            | DmErrorKind::Syntax
            | DmErrorKind::ObjectNotFound
            | DmErrorKind::ConstraintViolation
            | DmErrorKind::DuplicateKey
            | DmErrorKind::Deadlock
            | DmErrorKind::SerializationFailure
            | DmErrorKind::Timeout
            | DmErrorKind::ReadOnlyTransaction
            | DmErrorKind::Encode { .. }
            | DmErrorKind::Decode { .. }
            | DmErrorKind::NullViolation { .. }
            | DmErrorKind::DataTypeMismatch { .. }
            | DmErrorKind::CharsetMismatch { .. }
            | DmErrorKind::Unsupported { .. }
            | DmErrorKind::Server { .. } => false,
        }
    }

    /// **无条件**可重试的种类。条件可重试的（`Timeout`）不在这里，
    /// 由 [`DmError::is_retryable`] 结合上下文判定。
    ///
    /// 穷尽匹配，无 `_` 臂。
    pub fn is_retryable_kind(&self) -> bool {
        match self {
            DmErrorKind::Io
            | DmErrorKind::Connect { .. }
            | DmErrorKind::PoolTimeout
            | DmErrorKind::Deadlock
            | DmErrorKind::SerializationFailure => true,

            DmErrorKind::DriverNotFound { .. }
            | DmErrorKind::Config(_)
            | DmErrorKind::ConnectionLost
            | DmErrorKind::Syntax
            | DmErrorKind::ObjectNotFound
            | DmErrorKind::ConstraintViolation
            | DmErrorKind::DuplicateKey
            | DmErrorKind::Timeout
            | DmErrorKind::ReadOnlyTransaction
            | DmErrorKind::Encode { .. }
            | DmErrorKind::Decode { .. }
            | DmErrorKind::NullViolation { .. }
            | DmErrorKind::DataTypeMismatch { .. }
            | DmErrorKind::CharsetMismatch { .. }
            | DmErrorKind::Unsupported { .. }
            | DmErrorKind::Server { .. } => false,
        }
    }

    /// 服务端错误携带的 `(SQLSTATE, 原生码)`，非服务端错误返回 `None`。
    pub fn server_code(&self) -> Option<([u8; SQLSTATE_LEN], i32)> {
        match self {
            DmErrorKind::Server { sqlstate, code, .. } => Some((*sqlstate, *code)),
            _ => None,
        }
    }

    /// 把服务端返回的 `(SQLSTATE, 原生码, 消息)` 分类成具体的错误种类。
    ///
    /// **只认已实测登记的错误码**（见 [`codes`]）。未登记的一律落回
    /// [`DmErrorKind::Server`]，原始三元组一字不改 —— 宁可让调用方看到生码，
    /// 也不猜一个可能错的语义。新增映射必须先在真机上测出来、再登记进 [`codes`]，
    /// 并同步补一条本函数的用例。
    ///
    /// 判定以**原生码**为准（原生码比 SQLSTATE 精确）；`IM002` 是唯一没有原生码、
    /// 只能靠 SQLSTATE 判定的情形。
    ///
    /// `-70027`（无效的游标状态）**故意不分类**：它是「调用方违反了游标纪律」，
    /// 由 `dm-odbc` 在语句对象层自理，正常路径必须零出现（集成测试断言）。
    pub fn classify_server(sqlstate: [u8; SQLSTATE_LEN], code: i32, message: &str) -> DmErrorKind {
        match code {
            codes::SYNTAX => DmErrorKind::Syntax,
            codes::INVALID_COLUMN | codes::INVALID_TABLE => DmErrorKind::ObjectNotFound,
            codes::STRING_TRUNCATED => DmErrorKind::Encode { column: None },
            codes::FEATURE_NOT_IMPLEMENTED => DmErrorKind::Unsupported {
                feature: describe_unsupported(sqlstate, code, message),
            },
            codes::NEED_MORE_INFO => DmErrorKind::Connect {
                source: describe_server(sqlstate, code, message),
            },
            // 行锁等待失败：与语句超时同构（见 [`DmErrorKind::Timeout`] 的文档）。
            // 触发方式是 `… for update nowait` 撞上别人持有的事务，此时它**立刻**返回，
            // 所以「超时」这个词指的是「没等到」，不是「等够了时长」。
            codes::LOCK_WAIT => DmErrorKind::Timeout,
            _ if sqlstate == *b"IM002" => DmErrorKind::Connect {
                source: describe_server(sqlstate, code, message),
            },
            _ => DmErrorKind::Server {
                sqlstate,
                code,
                message: message.to_string(),
            },
        }
    }
}

/// **已实测登记的达梦服务端错误码**（方案 §6.8 的表）。
///
/// 每个常量的文档都注明「触发条件」，因为它同时也是我们对驱动的理解记录：
/// 只要有一个码的语义判断错了，排查就会从这里开始。
pub mod codes {
    /// `-70027` `无效的游标状态`。
    ///
    /// 触发：游标未关时再次执行同一语句 / 对非查询语句调用 `fetch`。
    /// **故意不参与分类**——这是调用方违反游标纪律，由 `dm-odbc` 语句对象层自理。
    pub const INVALID_CURSOR: i32 = -70027;

    /// `42000 / -2007` `语法分析出错`。
    ///
    /// 触发：反引号包标识符、`[a]` 方括号、`GENERATED … AS IDENTITY`、
    /// `ON DUPLICATE KEY UPDATE`。
    pub const SYNTAX: i32 = -2007;

    /// `-2111` `无效的列名`。
    ///
    /// 触发：双引号包小写标识符（双引号是大小写精确匹配，不加引号才折叠为大写）。
    pub const INVALID_COLUMN: i32 = -2111;

    /// `42S02 / -2106 第1 行附近出现错误: 无效的表或视图名[…]`。
    ///
    /// 触发：查询不存在的表/视图（T3.3 真机实测）。
    /// **与 [`INVALID_COLUMN`] 是两条不同的码**：达梦对「表不存在」给 `-2106`（标准 SQLSTATE
    /// `42S02`），对「列不存在」才给 `-2111`。早先只登记了 `-2111`，于是「表不存在」落回
    /// `Server`——分类表的缺口只有靠真机跑一遍才看得见。
    pub const INVALID_TABLE: i32 = -2106;

    /// `22001 / -70005` `字符串截断`。
    ///
    /// 触发：绑定参数超过列的声明长度。**注意**：`cast(… as varchar(n))` 是静默截断，
    /// 与绑参超长报错不同。
    pub const STRING_TRUNCATED: i32 = -70005;

    /// `HYC00 / -70054` `特性未实现`。
    ///
    /// 触发：设置 `SQL_ATTR_ASYNC_ENABLE`（达梦 ODBC 完全不支持异步）。
    pub const FEATURE_NOT_IMPLEMENTED: i32 = -70054;

    /// `HY000 / -71008` `需要更多信息`。
    ///
    /// 触发：连接串缺 `SERVER`/`UID`（实测：二者同时为空时）。
    /// 这条正是 [`crate::ErrorContext`] 之外必须再给 `DmConfig` 加
    /// `validate_connectable()` 的原因——放任到驱动只会得到读不懂的 `-71008`。
    pub const NEED_MORE_INFO: i32 = -71008;

    /// `HY000 / -6804` `需要更多的参数`。
    ///
    /// 触发：`SQLExecDirect` 下发带 `?` 的语句（同 `-70039`，故意不登记）。
    /// **故意不参与分类**——同 `-70039`，它是调用方参数绑定出了错。
    pub const NEED_MORE_PARAMS: i32 = -6804;

    /// `HY000 / -6409` `锁等待`。
    ///
    /// 触发（M6/T6.5 真机实测）：一条连接开着事务锁住某行不放，另一条连接对同一行
    /// 下 `select … for update nowait`，后者立刻拿到这个码。等超时的路径报的是同一个码，
    /// 只是慢。
    pub const LOCK_WAIT: i32 = -6409;

    /// `22018 / -6111` `字符串转换出错`。
    ///
    /// 触发：`insert into … (N) values ('abc')`——把不能转的字符串往数值列塞。
    /// **故意不参与分类**：`DmErrorKind::DataTypeMismatch { expect, got }` 装的是
    /// 客户端侧的期望/实际类型，服务端这句话里没有这些信息，硬填就是编造。
    pub const STRING_CONVERSION_FAILED: i32 = -6111;

    /// `22007 / -6118` `非法的时间日期类型数据`。
    ///
    /// 触发：把 `'not-a-date'` 写进 `DATE` 列。**故意不参与分类**（同 -6111）。
    pub const INVALID_DATETIME: i32 = -6118;

    /// `22012 / -6103` `除0错误`。
    ///
    /// 触发：`select 1/0 from dual`。**故意不参与分类**——`DmErrorKind` 里没有、
    /// 也不该为它新加一个「算术错误」变体（§6.12 冻结：不新增变体）。
    pub const DIVISION_BY_ZERO: i32 = -6103;

    /// 所有已登记的原生码，供测试遍历与文档生成。
    pub const REGISTERED: [i32; 11] = [
        INVALID_CURSOR,
        SYNTAX,
        INVALID_COLUMN,
        INVALID_TABLE,
        STRING_TRUNCATED,
        FEATURE_NOT_IMPLEMENTED,
        NEED_MORE_INFO,
        LOCK_WAIT,
        STRING_CONVERSION_FAILED,
        INVALID_DATETIME,
        DIVISION_BY_ZERO,
    ];
}

/// 把服务端三元组压成一行人类可读的来源描述，例如 `HY000/-71008 需要更多信息`。
fn describe_server(sqlstate: [u8; SQLSTATE_LEN], code: i32, message: &str) -> String {
    let ss = sqlstate_display(&sqlstate);
    let ss = ss.trim();
    let mut out = String::new();
    if !ss.is_empty() {
        out.push_str(ss);
        if code != 0 {
            out.push('/');
        }
    }
    if code != 0 {
        out.push_str(&code.to_string());
    }
    if !message.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(message);
    }
    if out.is_empty() {
        out.push_str("服务端未提供诊断信息");
    }
    out
}

/// 「特性未实现」的描述。
///
/// `dm-odbc` 在知道自己正在设置哪个属性时应当直接构造
/// `Unsupported { feature: "SQL_ATTR_ASYNC_ENABLE" }`；这里的组合串是分类器的兜底，
/// 保证信息不丢（消息 + 码），只是不如属性名精确。
fn describe_unsupported(sqlstate: [u8; SQLSTATE_LEN], code: i32, message: &str) -> String {
    describe_server(sqlstate, code, message)
}

impl fmt::Display for DmErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DmErrorKind::DriverNotFound { tried, hint } => {
                write!(
                    f,
                    "未找到达梦 ODBC 驱动；已尝试：{}。{hint}",
                    tried.join(", ")
                )
            }
            DmErrorKind::Config(m) => write!(f, "配置错误：{m}"),
            DmErrorKind::Connect { source } => write!(f, "建立连接失败：{source}"),
            DmErrorKind::Io => f.write_str("底层 I/O 错误"),
            DmErrorKind::PoolTimeout => f.write_str("在超时时间内未能从连接池取得连接"),
            DmErrorKind::ConnectionLost => f.write_str("连接已断开"),
            DmErrorKind::Syntax => f.write_str("SQL 语法错误"),
            DmErrorKind::ObjectNotFound => f.write_str("对象不存在（表、列或视图）"),
            DmErrorKind::ConstraintViolation => f.write_str("违反完整性约束"),
            DmErrorKind::DuplicateKey => f.write_str("主键或唯一键冲突"),
            DmErrorKind::Deadlock => f.write_str("检测到死锁，事务已被回滚"),
            DmErrorKind::SerializationFailure => f.write_str("序列化失败（隔离级别冲突）"),
            DmErrorKind::Timeout => f.write_str("语句执行超时"),
            DmErrorKind::ReadOnlyTransaction => f.write_str("只读事务中不允许写操作"),
            DmErrorKind::Encode { column } => match column {
                Some(c) => write!(f, "参数编码失败（列 {c}）"),
                None => f.write_str("参数编码失败"),
            },
            DmErrorKind::Decode { column } => match column {
                Some(c) => write!(f, "结果解码失败（列 {c}）"),
                None => f.write_str("结果解码失败"),
            },
            DmErrorKind::NullViolation { column } => {
                write!(f, "列 {column} 的值为 NULL，但目标类型不接受 NULL")
            }
            DmErrorKind::DataTypeMismatch { expect, got } => {
                write!(f, "类型不匹配：期望 {expect}，实际 {got}")
            }
            DmErrorKind::CharsetMismatch { configured, actual } => write!(
                f,
                "字符集不匹配：配置为 {configured}，服务端 select unicode() 返回 {actual}"
            ),
            DmErrorKind::Unsupported { feature } => write!(f, "不支持的特性：{feature}"),
            DmErrorKind::Server {
                sqlstate,
                code,
                message,
            } => {
                let ss = sqlstate_to_string(sqlstate);
                let ss = ss.trim();
                if ss.is_empty() {
                    write!(f, "服务端错误 [{code}] {message}")
                } else {
                    write!(f, "服务端错误 [{ss}/{code}] {message}")
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 顶层错误
// ---------------------------------------------------------------------------

/// 中间件对外暴露的唯一错误类型。
#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct DmError {
    /// 错误种类。
    pub kind: DmErrorKind,
    /// 诊断链，按 ODBC 返回顺序排列。可能为空（例如纯本地校验失败）。
    pub diags: Vec<DiagRecord>,
    /// 附加上下文。
    pub context: ErrorContext,
}

impl DmError {
    /// 只有种类，无诊断、无上下文。
    pub fn new(kind: DmErrorKind) -> Self {
        Self {
            kind,
            diags: Vec::new(),
            context: ErrorContext::default(),
        }
    }

    /// 只带阶段。
    pub fn at(kind: DmErrorKind, stage: Stage) -> Self {
        Self {
            kind,
            diags: Vec::new(),
            context: ErrorContext::new(stage),
        }
    }

    /// 带诊断链与上下文。
    pub fn with_diags(kind: DmErrorKind, diags: Vec<DiagRecord>, context: ErrorContext) -> Self {
        Self {
            kind,
            diags,
            context,
        }
    }

    /// 用服务端三元组构造**已分类**的错误，并保留一条原始诊断记录。
    ///
    /// 分类规则见 [`DmErrorKind::classify_server`]，登记表见 [`codes`]。
    /// **诊断记录一定会被保留**——分类只改变 `kind`，不吞掉原始三元组，
    /// 所以调用方即使看到 `Syntax` 也能通过 [`DmError::server_code_and_message`] 拿到 `-2007`。
    ///
    /// `dm-odbc` 在把 `SQLGetDiagRec` 采到的记录转成 `DmError` 时用这个构造函数。
    pub fn from_server(
        sqlstate: [u8; SQLSTATE_LEN],
        code: i32,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        let kind = DmErrorKind::classify_server(sqlstate, code, &message);
        let mut err = Self::new(kind);
        err.push_diag(DiagRecord::new(sqlstate, code, message));
        err
    }

    /// 用服务端三元组构造**未分类**的 `Server` 错误。
    ///
    /// 只在你确实想让 `kind` 保持原始三元组时使用（例如诊断收集器的中间态）。
    /// 面向用户的错误请用 [`DmError::from_server`]。
    pub fn server_raw(sqlstate: [u8; SQLSTATE_LEN], code: i32, message: impl Into<String>) -> Self {
        Self::new(DmErrorKind::Server {
            sqlstate,
            code,
            message: message.into(),
        })
    }

    /// 追加一条诊断记录。
    pub fn push_diag(&mut self, rec: DiagRecord) {
        self.diags.push(rec);
    }

    /// 诊断链。
    pub fn diags(&self) -> &[DiagRecord] {
        &self.diags
    }

    /// 第一条诊断的 `(原生码, 消息)`，供上层提取服务端细节。
    pub fn server_code_and_message(&self) -> Option<(i32, &str)> {
        self.diags
            .first()
            .map(|d| (d.native_code, d.message.as_str()))
    }

    /// **连接是否已不可复用**。池据此决定「丢弃」还是「reset 后回池」。
    pub fn is_connection_lost(&self) -> bool {
        self.kind.is_connection_lost()
    }

    /// 能否在**新的连接/事务**上重试。
    ///
    /// 无条件可重试的种类见 [`DmErrorKind::is_retryable_kind`]；`Timeout` 是条件可重试：
    /// 只有「语句级 + 调用方声明幂等」才允许重试，否则重复写入的代价高于收益。
    pub fn is_retryable(&self) -> bool {
        if self.kind.is_retryable_kind() {
            return true;
        }
        matches!(self.kind, DmErrorKind::Timeout)
            && self.context.stmt_level
            && self.context.idempotent
    }

    /// 是否只读事务违规。
    pub fn is_read_only_transaction(&self) -> bool {
        matches!(self.kind, DmErrorKind::ReadOnlyTransaction)
    }
}

impl From<DmErrorKind> for DmError {
    fn from(kind: DmErrorKind) -> Self {
        DmError::new(kind)
    }
}

// ---------------------------------------------------------------------------
// 测试：分类判定全分支
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部变体的一份样本，用于穷尽测试。
    fn all_kinds() -> Vec<DmErrorKind> {
        vec![
            DmErrorKind::DriverNotFound {
                tried: vec!["a".into()],
                hint: "装驱动".into(),
            },
            DmErrorKind::Config("port".into()),
            DmErrorKind::Connect {
                source: "refused".into(),
            },
            DmErrorKind::Io,
            DmErrorKind::PoolTimeout,
            DmErrorKind::ConnectionLost,
            DmErrorKind::Syntax,
            DmErrorKind::ObjectNotFound,
            DmErrorKind::ConstraintViolation,
            DmErrorKind::DuplicateKey,
            DmErrorKind::Deadlock,
            DmErrorKind::SerializationFailure,
            DmErrorKind::Timeout,
            DmErrorKind::ReadOnlyTransaction,
            DmErrorKind::Encode {
                column: Some("c".into()),
            },
            DmErrorKind::Encode { column: None },
            DmErrorKind::Decode {
                column: Some("c".into()),
            },
            DmErrorKind::Decode { column: None },
            DmErrorKind::NullViolation { column: "c".into() },
            DmErrorKind::DataTypeMismatch {
                expect: "i64".into(),
                got: "String".into(),
            },
            DmErrorKind::CharsetMismatch {
                configured: 65001,
                actual: 0,
            },
            DmErrorKind::Unsupported {
                feature: "async".into(),
            },
            DmErrorKind::Server {
                sqlstate: *b"42000",
                code: -2007,
                message: "语法分析出错".into(),
            },
        ]
    }

    #[test]
    fn every_kind_has_non_empty_display() {
        for k in all_kinds() {
            let s = k.to_string();
            assert!(!s.is_empty(), "变体 {k:?} 的 Display 为空");
        }
    }

    #[test]
    fn connection_lost_matches_spec_table() {
        // 方案 §6.8 的表：只有 Io 与 ConnectionLost 为 true。
        assert!(DmErrorKind::Io.is_connection_lost());
        assert!(DmErrorKind::ConnectionLost.is_connection_lost());

        for k in all_kinds() {
            if matches!(k, DmErrorKind::Io | DmErrorKind::ConnectionLost) {
                continue;
            }
            assert!(
                !k.is_connection_lost(),
                "变体 {k:?} 不应被判为 connection_lost"
            );
        }
    }

    #[test]
    fn connection_lost_and_retryable_are_independent() {
        // ConnectionLost: lost=true, retryable=false
        assert!(DmErrorKind::ConnectionLost.is_connection_lost());
        assert!(!DmErrorKind::ConnectionLost.is_retryable_kind());

        // Io: lost=true, retryable=true —— 两者互不蕴含
        assert!(DmErrorKind::Io.is_connection_lost());
        assert!(DmErrorKind::Io.is_retryable_kind());

        // Deadlock: lost=false, retryable=true
        assert!(!DmErrorKind::Deadlock.is_connection_lost());
        assert!(DmErrorKind::Deadlock.is_retryable_kind());

        // Syntax: 两者皆 false
        assert!(!DmErrorKind::Syntax.is_connection_lost());
        assert!(!DmErrorKind::Syntax.is_retryable_kind());
    }

    #[test]
    fn unconditional_retryable_set_matches_spec() {
        let expect_retryable = |k: &DmErrorKind| {
            matches!(
                k,
                DmErrorKind::Io
                    | DmErrorKind::Connect { .. }
                    | DmErrorKind::PoolTimeout
                    | DmErrorKind::Deadlock
                    | DmErrorKind::SerializationFailure
            )
        };
        for k in all_kinds() {
            assert_eq!(
                k.is_retryable_kind(),
                expect_retryable(&k),
                "变体 {k:?} 的无条件可重试判定与方案 §6.8 不符"
            );
        }
    }

    #[test]
    fn timeout_is_conditionally_retryable() {
        // 默认（非语句级、非幂等）：不可重试。
        let plain = DmError::at(DmErrorKind::Timeout, Stage::Execute);
        assert!(!plain.is_retryable());

        // 仅语句级：仍不可重试。
        let stmt = DmError {
            kind: DmErrorKind::Timeout,
            diags: vec![],
            context: ErrorContext::new(Stage::Execute).stmt_level(),
        };
        assert!(!stmt.is_retryable());

        // 仅幂等：仍不可重试。
        let idem = DmError {
            kind: DmErrorKind::Timeout,
            diags: vec![],
            context: ErrorContext::new(Stage::Execute).idempotent(),
        };
        assert!(!idem.is_retryable());

        // 语句级 + 幂等：可重试。
        let both = DmError {
            kind: DmErrorKind::Timeout,
            diags: vec![],
            context: ErrorContext::new(Stage::Execute).stmt_level().idempotent(),
        };
        assert!(both.is_retryable());
    }

    #[test]
    fn timeout_conditional_does_not_leak_into_other_kinds() {
        // 即便是「语句级 + 幂等」的上下文，非 Timeout 的种类也不受影响。
        let ctx = ErrorContext::new(Stage::Execute).stmt_level().idempotent();
        let e = DmError::with_diags(DmErrorKind::DuplicateKey, vec![], ctx);
        assert!(!e.is_retryable());
    }

    #[test]
    fn read_only_transaction_predicate() {
        assert!(DmError::new(DmErrorKind::ReadOnlyTransaction).is_read_only_transaction());
        assert!(!DmError::new(DmErrorKind::Syntax).is_read_only_transaction());
    }

    #[test]
    fn server_code_roundtrip() {
        let e = DmError::server_raw(*b"42000", -2007, "语法分析出错");
        assert_eq!(e.kind.server_code(), Some((*b"42000", -2007)));
        assert!(e.to_string().contains("42000"));
        assert!(e.to_string().contains("-2007"));
        assert!(
            !DmError::new(DmErrorKind::Syntax)
                .kind
                .server_code()
                .is_some()
        );
    }

    #[test]
    fn diagnostics_chain_is_preserved_in_order() {
        let mut e = DmError::at(
            DmErrorKind::Encode {
                column: Some("b".into()),
            },
            Stage::Bind,
        );
        e.push_diag(DiagRecord::new(*b"22001", -70005, "字符串截断"));
        e.push_diag(DiagRecord::native(-1, "附加信息").at(Some(2), Some(3)));
        assert_eq!(e.diags().len(), 2);
        assert_eq!(e.server_code_and_message(), Some((-70005, "字符串截断")));
        assert_eq!(e.diags()[1].to_string(), "[-1] 附加信息（第 3 行第 2 列）");
    }

    #[test]
    fn diag_display_handles_missing_sqlstate() {
        let d = DiagRecord::new([0u8; SQLSTATE_LEN], -70027, "无效的游标状态");
        assert_eq!(d.to_string(), "[-70027] 无效的游标状态");
        assert!(d.sqlstate_str().trim().is_empty());
    }

    #[test]
    fn sqlstate_helpers_roundtrip_and_escape() {
        assert_eq!(sqlstate_from_str("42000"), *b"42000");
        // 不足 5 位右补空格
        assert_eq!(sqlstate_from_str("HY"), [b'H', b'Y', b' ', b' ', b' ']);
        // 超长截断
        assert_eq!(sqlstate_from_str("42000EXTRA"), *b"42000");
        // 不可打印字节被转义，保证无损
        assert_eq!(
            sqlstate_to_string(&[0x00, b'A', 0x1F, b' ', b' ']),
            "\\x00A\\x1F  "
        );
        assert_eq!(sqlstate_to_string(b"HYC00"), "HYC00");
    }

    #[test]
    fn error_context_builders() {
        let c = ErrorContext::new(Stage::Fetch)
            .with_sql("select * from t where id = ?")
            .stmt_level()
            .idempotent();
        assert_eq!(c.stage, Stage::Fetch);
        assert_eq!(
            c.sql_fingerprint.as_deref(),
            Some("select * from t where id = ?")
        );
        assert!(c.stmt_level);
        assert!(c.idempotent);
        assert_eq!(ErrorContext::default().stage, Stage::Other);
    }

    #[test]
    fn driver_not_found_display_lists_candidates() {
        let k = DmErrorKind::DriverNotFound {
            tried: vec!["DM8 ODBC DRIVER".into(), "DM ODBC DRIVER".into()],
            hint: "请检查 odbcinst.ini".into(),
        };
        let s = k.to_string();
        assert!(s.contains("DM8 ODBC DRIVER"));
        assert!(s.contains("DM ODBC DRIVER"));
        assert!(s.contains("odbcinst.ini"));
    }

    #[test]
    fn error_is_std_error_and_source_chain_works() {
        fn assert_error<E: std::error::Error + Send + Sync + 'static>(_: &E) {}
        let e = DmError::new(DmErrorKind::Syntax);
        assert_error(&e);
        // 装箱后仍可下转型取回
        let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(e);
        let back = boxed
            .downcast_ref::<DmError>()
            .expect("应能下转型回 DmError");
        assert_eq!(back.kind, DmErrorKind::Syntax);
    }

    #[test]
    fn from_kind_conversion() {
        let e: DmError = DmErrorKind::PoolTimeout.into();
        assert!(e.is_retryable());
        assert!(!e.is_connection_lost());
        assert!(e.diags().is_empty());
    }

    #[test]
    fn stage_display_covers_all_variants() {
        let stages = [
            Stage::Config,
            Stage::DriverLoad,
            Stage::Connect,
            Stage::Prepare,
            Stage::Bind,
            Stage::Execute,
            Stage::Fetch,
            Stage::Reset,
            Stage::Close,
            Stage::Other,
        ];
        for s in stages {
            assert!(!s.to_string().is_empty(), "阶段 {s:?} 的 Display 为空");
        }
    }

    // ---- 实测错误码必须落在预期分类（方案 §6.8 的错误码表） ----

    /// 给种类一个稳定的标签，用于表驱动断言与快照。
    ///
    /// 穷尽匹配、无 `_` 臂：新增 `DmErrorKind` 变体时这里会编译失败，
    /// 强制作者在本测试里表态（这正是 §6.8「CI 用穷尽 match 强制」的落点之一）。
    fn kind_tag(kind: &DmErrorKind) -> &'static str {
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

    /// §6.8 表的每一行：每个已实测的三元组必须落在它那一行的分类上。
    #[test]
    fn evidenced_codes_land_in_expected_classes() {
        // (sqlstate, code, message, 期望标签)
        let table: [(&str, i32, &str, &str); 9] = [
            ("42000", -2007, "语法分析出错", "Syntax"),
            ("", -2111, "无效的列名", "ObjectNotFound"),
            // T3.3 真机实测：表不存在给的是 -2106/42S02，不是 -2111。
            (
                "42S02",
                -2106,
                "第1 行附近出现错误:\n无效的表或视图名[T]",
                "ObjectNotFound",
            ),
            ("22001", -70005, "字符串截断", "Encode"),
            ("HYC00", -70054, "特性未实现", "Unsupported"),
            (
                "IM002",
                0,
                "未发现数据源名称并且未指定默认驱动程序",
                "Connect",
            ),
            ("HY000", -71008, "需要更多信息", "Connect"),
            // M6/T6.5 真机实测：行锁等待失败 → Timeout（连接本身是好的）。
            ("HY000", -6409, "锁等待", "Timeout"),
            // -70027 故意不分类：它是调用方违反游标纪律，由 dm-odbc 语句对象层自理
            ("", -70027, "无效的游标状态", "Server"),
        ];
        for (ss, code, msg, want) in table {
            let e = DmError::from_server(sqlstate_from_str(ss), code, msg);
            assert_eq!(
                kind_tag(&e.kind),
                want,
                "({ss:?}, {code}) 分类错误，得到 {:?}",
                e.kind
            );
        }
    }

    /// 未登记的错误码**一律**落回 `Server`，且原始三元组一字不改。
    #[test]
    fn unregistered_codes_fall_through_to_raw_server() {
        let cases: [(&str, i32, &str); 6] = [
            ("23000", -2601, "违反唯一约束"),
            ("40001", -1, "序列化失败"),
            ("40P01", -2, "死锁"),
            ("08006", -3, "连接异常"),
            ("HY000", -70028, "未登记的新码"),
            ("", 0, "什么都没有"),
        ];
        for (ss, code, msg) in cases {
            let e = DmError::from_server(sqlstate_from_str(ss), code, msg);
            match &e.kind {
                DmErrorKind::Server {
                    sqlstate,
                    code: got_code,
                    message,
                } => {
                    assert_eq!(*sqlstate, sqlstate_from_str(ss), "SQLSTATE 被改写");
                    assert_eq!(*got_code, code, "原生码被改写");
                    assert_eq!(message, msg, "消息被改写");
                }
                other => panic!("未登记的 ({ss:?}, {code}) 应当落回 Server，得到 {other:?}"),
            }
            assert!(!e.is_retryable());
            assert!(!e.is_connection_lost());
        }
    }

    /// 判定以**原生码**为准；`IM002` 是唯一靠 SQLSTATE 判定的情形。
    #[test]
    fn classification_keys_on_native_code_not_sqlstate() {
        // -2007 配上一个不对的 SQLSTATE 仍然是语法错误
        let weird = DmErrorKind::classify_server(*b"99999", codes::SYNTAX, "语法分析出错");
        assert_eq!(kind_tag(&weird), "Syntax");

        // IM002 没有原生码，只能靠 SQLSTATE
        let im = DmErrorKind::classify_server(*b"IM002", 0, "未发现数据源名称");
        assert_eq!(kind_tag(&im), "Connect");

        // HYC00 但没有登记的原生码 → 不猜，落回 Server
        let hyc = DmErrorKind::classify_server(*b"HYC00", 0, "特性未实现");
        assert_eq!(kind_tag(&hyc), "Server");
    }

    /// 分类不能把原始诊断吞掉——这是 §6.8「多记录诊断链」的前提。
    #[test]
    fn from_server_preserves_the_raw_diagnostic() {
        let e = DmError::from_server(*b"42000", codes::SYNTAX, "语法分析出错");
        assert_eq!(kind_tag(&e.kind), "Syntax");
        assert_eq!(e.diags().len(), 1);
        assert_eq!(
            e.server_code_and_message(),
            Some((codes::SYNTAX, "语法分析出错"))
        );
        assert_eq!(e.diags()[0].sqlstate_str(), "42000");
    }

    /// 每个登记在 [`codes`] 里的码都必须有明确归属：
    /// 要么落到某个具体分类，要么在文档里写明「故意不分类」。
    /// 这条测试让「新增登记码却忘了补映射」无法蒙混过关。
    #[test]
    fn every_registered_code_has_a_deliberate_home() {
        // 这几个码是真机测出来的，但**故意不分类**——理由写在各自常量的文档里
        // （`DataTypeMismatch` 装的是客户端侧的 expect/got，服务端的话术填不进去；
        // 除零没有对应变体）。它们照样进目录，只是 `classified: false`。
        let deliberately_unclassified = [
            codes::INVALID_CURSOR,
            codes::NEED_MORE_PARAMS,
            codes::STRING_CONVERSION_FAILED,
            codes::INVALID_DATETIME,
            codes::DIVISION_BY_ZERO,
        ];
        for code in codes::REGISTERED {
            let kind = DmErrorKind::classify_server(*b"HY000", code, "占位消息");
            let tag = kind_tag(&kind);
            if deliberately_unclassified.contains(&code) {
                assert_eq!(tag, "Server", "{code} 被标为「故意不分类」却分类成了 {tag}");
            } else {
                assert_ne!(tag, "Server", "{code} 已登记但没有任何映射");
            }
        }
    }

    /// 分类后的重试/连接有效性契约（池据此决定丢弃还是 reset 回池）。
    #[test]
    fn classified_kinds_carry_the_pool_contract() {
        let rows: [(&str, i32, bool, bool); 7] = [
            // (sqlstate, code, expect_retryable, expect_connection_lost)
            ("42000", -2007, false, false),  // Syntax
            ("", -2111, false, false),       // ObjectNotFound
            ("22001", -70005, false, false), // Encode
            ("HYC00", -70054, false, false), // Unsupported
            ("IM002", 0, true, false),       // Connect：可重试但不是「连接已断」
            ("HY000", -71008, true, false),  // Connect
            ("", -70027, false, false),      // Server(-70027)
        ];
        for (ss, code, want_retry, want_lost) in rows {
            let e = DmError::from_server(sqlstate_from_str(ss), code, "消息");
            assert_eq!(e.is_retryable(), want_retry, "({ss:?}, {code}) 可重试判定");
            assert_eq!(
                e.is_connection_lost(),
                want_lost,
                "({ss:?}, {code}) 连接有效性判定"
            );
        }
    }

    /// `describe_server` / `describe_unsupported` 的组合规则。
    #[test]
    fn describe_helpers_compose_readable_sources() {
        assert_eq!(
            describe_server(*b"HY000", -71008, "需要更多信息"),
            "HY000/-71008 需要更多信息"
        );
        assert_eq!(
            describe_server(*b"IM002", 0, "未发现数据源"),
            "IM002 未发现数据源"
        );
        assert_eq!(
            describe_server([0u8; 5], -70027, "无效的游标状态"),
            "-70027 无效的游标状态"
        );
        assert_eq!(describe_server([0u8; 5], 0, ""), "服务端未提供诊断信息");
        assert_eq!(
            describe_unsupported(*b"HYC00", -70054, "特性未实现"),
            "HYC00/-70054 特性未实现"
        );
    }

    /// 对外错误文本的快照（`insta` 内联快照：不依赖外部 `.snap` 文件，保持仓库自洽）。
    #[test]
    fn snapshot_classified_error_messages() {
        let cases: [(&str, i32, &str); 8] = [
            ("42000", -2007, "语法分析出错"),
            ("", -2111, "无效的列名"),
            ("22001", -70005, "字符串截断"),
            ("HYC00", -70054, "特性未实现"),
            ("IM002", 0, "未发现数据源名称并且未指定默认驱动程序"),
            ("HY000", -71008, "需要更多信息"),
            ("", -70027, "无效的游标状态"),
            ("23000", -2601, "违反唯一约束"),
        ];
        let mut out = String::new();
        for (ss, code, msg) in cases {
            let e = DmError::from_server(sqlstate_from_str(ss), code, msg);
            out.push_str(&format!("({ss:?}, {code}) => {e}\n"));
        }
        insta::assert_snapshot!(out, @r#"
        ("42000", -2007) => SQL 语法错误
        ("", -2111) => 对象不存在（表、列或视图）
        ("22001", -70005) => 参数编码失败
        ("HYC00", -70054) => 不支持的特性：HYC00/-70054 特性未实现
        ("IM002", 0) => 建立连接失败：IM002 未发现数据源名称并且未指定默认驱动程序
        ("HY000", -71008) => 建立连接失败：HY000/-71008 需要更多信息
        ("", -70027) => 服务端错误 [-70027] 无效的游标状态
        ("23000", -2601) => 服务端错误 [23000/-2601] 违反唯一约束
        "#);
    }
}
