//! # dm-core —— 配置与连接串
//!
//! 这里是中间件的第二层（`dm-error ← dm-core ← dm-odbc ← …`）。**硬约束**：本 crate
//! 不依赖 `tokio`，也不依赖 `odbc-api`（CI 用 `cargo tree` 强校验）。因此这里只放
//! 纯数据结构与纯逻辑，任何 I/O 都在上层。
//!
//! 本模块（M0/T0.3 的范围）提供：
//!
//! - [`DmConfig`] —— 全部连接参数的载体，字段全部公开；
//! - [`DmConfigBuilder`] —— 结构化构造，带默认值分层与环境变量回退；
//! - [`SecretString`] —— 口令的脱敏包装，**`Debug` 不泄露明文**；
//! - [`PoolConfig`] —— 连接池参数（定义放在这里，`dm-pool` 复用之，避免反向依赖）；
//! - [`DmConfig::validate`] —— 语义校验，非法值返回 `DmErrorKind::Config`。
//!
//! 连接串的渲染与解析在 [`crate::connstr`]。

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use dm_error::{DmError, DmErrorKind, Stage};

/// 达梦 ODBC 驱动的默认名称（与控制面板 → ODBC 数据源管理器里注册的一致）。
pub const DEFAULT_DRIVER: &str = "DM8 ODBC DRIVER";
/// 达梦的默认监听端口。
pub const DEFAULT_PORT: u16 = 5236;
/// 建连超时默认值。
pub const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
/// 登录超时默认值。
pub const DEFAULT_LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// 慢查询告警阈值（设计方案 §6.9：默认 1 s）。
///
/// 语义：一条语句（含取数）耗时 **≥ 本阈值**时，`dm-query` 打一条 `warn!`，
/// 里面带 SQL 指纹与本次的行数/影响行数。设成 [`Duration::ZERO`] 即关闭。
pub const DEFAULT_SLOW_QUERY_THRESHOLD: Duration = Duration::from_secs(1);

/// 每条物理连接缓存的预处理语句条数（设计方案 §5.4 / CAP-C-11：默认 16）。
///
/// 同一条 SQL 文本第二次执行时复用第一次的语句句柄，省掉一次 `SQLPrepare`
/// 往返（§3.1 实测「裸驱动每轮重新 `SQLAllocHandle` + `SQLPrepare`」与
/// 「复用句柄」之差 ≈ 31 µs）。设成 `0` 即关闭缓存。
pub const DEFAULT_STMT_CACHE_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// 脱敏字符串
// ---------------------------------------------------------------------------

/// 口令包装：`Debug`/`Display` 都不输出明文。
///
/// 刻意**不实现 `Deref<Target = str>`**，避免明文在不经意间流进日志或格式化字符串；
/// 需要明文时必须显式调用 [`SecretString::expose`]。
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SecretString(String);

impl SecretString {
    /// 包装一个明文口令。
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// 显式取出明文。调用点应当是可审计的（例如拼连接串）。
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 明文长度（字节）。
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// 是否为空口令。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 只暴露长度，足以排查「配了但配错」的问题，又不泄露内容。
        write!(f, "SecretString(<已脱敏; {} 字节>)", self.0.len())
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl From<&str> for SecretString {
    fn from(v: &str) -> Self {
        Self::new(v)
    }
}

impl From<String> for SecretString {
    fn from(v: String) -> Self {
        Self::new(v)
    }
}

// ---------------------------------------------------------------------------
// 枚举型选项
// ---------------------------------------------------------------------------

/// 连接期字符集自检的严格程度（见设计方案 §8.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CharsetCheck {
    /// 不做自检。仅在确知服务端字符集时使用。
    Off,
    /// 不一致时 `tracing::warn!`，继续。
    Warn,
    /// 不一致时报错（默认）。
    #[default]
    Error,
}

impl CharsetCheck {
    /// 解析配置值（大小写不敏感）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "false" | "0" => Some(Self::Off),
            "warn" => Some(Self::Warn),
            "error" | "deny" | "true" | "1" => Some(Self::Error),
            _ => None,
        }
    }

    /// 渲染成配置值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// 服务端的兼容模式（`dm.ini` 的 `COMPATIBLE_MODE`，以及 ODBC 侧的 `COMPATIBLE_MODE` 关键字）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompatMode {
    /// 达梦原生模式（`0` / `DM`）。
    #[default]
    Dm,
    /// Oracle 兼容模式（`1` / `ORACLE`）。
    Oracle,
}

impl CompatMode {
    /// 解析配置值。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "DM" | "0" => Some(Self::Dm),
            "ORACLE" | "ORA" | "1" => Some(Self::Oracle),
            _ => None,
        }
    }

    /// 渲染成达梦接受的关键字值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dm => "DM",
            Self::Oracle => "ORACLE",
        }
    }

    /// `dm.ini` 里的数值形式。
    pub fn as_ini_value(self) -> i32 {
        match self {
            Self::Dm => 0,
            Self::Oracle => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// 连接池参数
// ---------------------------------------------------------------------------

/// 连接池参数，默认值见设计方案 §5.7。
///
/// 定义在 `dm-core`（而不是 `dm-pool`）是因为 [`DmConfig`] 需要内嵌它，
/// 而依赖方向是 `dm-core ← dm-pool`。`dm-pool` 会原样 `pub use` 本类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolConfig {
    /// 池上限。**同时就是阻塞线程池的并发上限**，因为 ODBC 是同步 API。
    pub max_size: usize,
    /// 保活下限。
    pub min_idle: usize,
    /// 获取连接的超时。
    pub acquire_timeout: Duration,
    /// 空闲超期回收（**从归还时刻起算**）。
    pub idle_timeout: Duration,
    /// 连接最大生存期（**从创建时刻起算**，用于规避服务端会话状态漂移）。
    pub max_lifetime: Duration,
    /// 探活节流间隔。
    pub health_check_interval: Duration,
    /// 是否每次借用都探活。
    pub test_on_acquire: bool,
    /// 后台回收周期。
    pub recycle_interval: Duration,
    /// 超时/取消后是否仍复用连接。
    ///
    /// 默认 `false`（决策 D11）：ODBC 的 `SQLCancel` 不保证服务端已经停止处理、
    /// 响应流已经对齐，复用会带来难以复现的错位，因此默认销毁。
    pub reuse_after_cancel: bool,
    /// 每条连接缓存的**预处理语句**条数上限（设计方案 §5.4 / CAP-C-11）。
    ///
    /// 按 SQL 文本精确匹配，超出上限按 LRU 淘汰；`0` 表示关闭
    /// （等价于 [`DmConfigBuilder::without_stmt_cache`]）。
    pub stmt_cache_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size: 10,
            min_idle: 1,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(1800),
            health_check_interval: Duration::from_secs(30),
            test_on_acquire: false,
            recycle_interval: Duration::from_secs(5),
            reuse_after_cancel: false,
            stmt_cache_size: DEFAULT_STMT_CACHE_SIZE,
        }
    }
}

// ---------------------------------------------------------------------------
// 主配置
// ---------------------------------------------------------------------------

/// 达梦连接的完整配置。
///
/// 所有字段公开，允许测试与高级用法直接构造；常规用法走 [`DmConfig::builder`]。
/// `Debug` 输出是安全的：口令字段是 [`SecretString`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmConfig {
    /// ODBC 驱动名，默认 [`DEFAULT_DRIVER`]。
    pub driver: String,
    /// 主机名或 IP。
    pub server: String,
    /// 端口，默认 [`DEFAULT_PORT`]。
    pub port: u16,
    /// 用户名。
    pub user: String,
    /// 口令（脱敏）。
    pub password: SecretString,
    /// 默认模式（schema）。
    ///
    /// **达梦 ODBC 驱动忽略连接串里的 `SCHEMA=` 关键字**（`SCHEMA_NAME=` / `CURRENT_SCHEMA=`
    /// 同样无效，实测连上后仍是登录用户的默认模式）。所以这个字段不只是「拼进连接串」：
    /// 建连时会在会话自检里带上当前模式，**与目标不一致才发一条 `SET SCHEMA "<模式名>"`**，
    /// 一致则不多发任何 SQL。模式名一律加双引号（未加引号会被折成大写，小写模式直接
    /// `-2103 无效的模式名`）。
    ///
    /// 用的是会话级切换，只影响本连接；池里每条新连接各自设一次。
    pub schema: Option<String>,
    /// 客户端字符集关键字，例如 `PG_UTF8`。留空则用服务端默认（GB18030）。
    pub character_code: Option<String>,
    /// 兼容模式。
    pub compatible_mode: Option<CompatMode>,
    /// 驱动内建的读写分离开关。
    ///
    /// **必须与应用层路由二选一**：两者同时开启会叠加，产生不可预期的主备分布
    /// （设计方案 R-12）。`dm-pool::router` 在检测到本项为真时拒绝启动。
    pub rw_separate: Option<bool>,
    /// 建连超时。
    pub connection_timeout: Duration,
    /// 登录超时。
    pub login_timeout: Duration,
    /// 连接建立后按顺序执行的初始化 SQL。
    pub init_sql: Vec<String>,
    /// 字符集自检策略。
    pub charset_check: CharsetCheck,
    /// `varchar(n)` 是按字符还是按字节计量。
    ///
    /// `None` 表示**连接期探测**（见设计方案 §4.4 与 §8.4）；`Some(false)` 是按字节。
    pub length_in_char: Option<bool>,
    /// 慢查询告警阈值（设计方案 §6.9）。
    ///
    /// 一条语句（含取数）耗时 **≥ 本阈值**时 `dm-query` 打 `warn!`，带 SQL 指纹与行数。
    /// [`Duration::ZERO`] 表示关闭。默认 [`DEFAULT_SLOW_QUERY_THRESHOLD`]。
    pub slow_query_threshold: Duration,
    /// 原始连接串键值对的覆盖层（逃生舱）。
    ///
    /// 键统一大写存储。渲染连接串时**这些键优先**：结构化字段若与其中某个键同名会被跳过。
    pub extra: BTreeMap<String, String>,
    /// 连接池参数。
    pub pool: PoolConfig,
}

impl Default for DmConfig {
    fn default() -> Self {
        Self {
            driver: DEFAULT_DRIVER.to_string(),
            server: String::new(),
            port: DEFAULT_PORT,
            user: String::new(),
            password: SecretString::default(),
            schema: None,
            character_code: None,
            compatible_mode: None,
            rw_separate: None,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
            login_timeout: DEFAULT_LOGIN_TIMEOUT,
            init_sql: Vec::new(),
            charset_check: CharsetCheck::default(),
            length_in_char: None,
            slow_query_threshold: DEFAULT_SLOW_QUERY_THRESHOLD,
            extra: BTreeMap::new(),
            pool: PoolConfig::default(),
        }
    }
}

impl DmConfig {
    /// 以默认为基线构造。
    pub fn new() -> Self {
        Self::default()
    }

    /// 结构化 builder（推荐入口）。
    pub fn builder() -> DmConfigBuilder {
        DmConfigBuilder::new()
    }

    /// 语义校验。非法值统一返回 [`DmErrorKind::Config`]。
    ///
    /// 校验项（设计方案 §6.1）：端口越界、`max_size = 0`、`min_idle > max_size`、
    /// `idle_timeout >= max_lifetime`、驱动名为空。
    ///
    /// 管的是**配置自洽性**，与「这组配置能不能真的连上」是两回事——后者见
    /// [`DmConfig::validate_connectable`]。两个都在建连入口被调用。
    pub fn validate(&self) -> Result<(), DmError> {
        if self.driver.trim().is_empty() {
            return Err(config_error("ODBC 驱动名不能为空"));
        }
        if self.port == 0 {
            return Err(config_error("端口不能为 0"));
        }
        if self.pool.max_size == 0 {
            return Err(config_error(
                "pool.max_size 不能为 0（它就是 ODBC 的并发上限）",
            ));
        }
        if self.pool.min_idle > self.pool.max_size {
            return Err(config_error(format!(
                "pool.min_idle ({}) 不能大于 pool.max_size ({})",
                self.pool.min_idle, self.pool.max_size
            )));
        }
        if self.pool.idle_timeout >= self.pool.max_lifetime {
            return Err(config_error(format!(
                "pool.idle_timeout ({:?}) 必须小于 pool.max_lifetime ({:?})，否则空闲回收永不触发",
                self.pool.idle_timeout, self.pool.max_lifetime
            )));
        }
        if self.pool.acquire_timeout.is_zero() {
            return Err(config_error("pool.acquire_timeout 不能为 0"));
        }
        if let Some(schema) = &self.schema {
            if schema.trim().is_empty() {
                return Err(config_error(
                    "schema 为空串：要默认模式就填模式名，不要默认模式请整个字段留空",
                ));
            }
            if schema.contains('\0') {
                return Err(config_error("schema 不能含 NUL 字符"));
            }
        }
        Ok(())
    }

    /// 建连前的可用性检查（[`DmConfig::validate`] 的补充，M1/T1.2 实测新增）。
    ///
    /// `validate()` 管配置自洽性；这里管「这组配置能不能真的发起一次连接」。
    /// 依据是 T1.2 的真机实测：`server` 与 `user` 同时为空时，达梦 ODBC 驱动在
    /// 建连阶段返回 `HY000 / -71008「需要更多信息」`——错误码完全读不出缺了什么。
    /// 在进驱动之前拦下，才能给出可操作的提示。
    ///
    /// `server` 与 `DSN` 二者有其一即可（`DSN=DM8` 自带主机信息，是合法 ODBC 用法）。
    pub fn validate_connectable(&self) -> Result<(), DmError> {
        if self.server.trim().is_empty() && !self.extra.contains_key("DSN") {
            return Err(config_error(
                "server 为空且未提供 DSN：请设置 server（如 localhost），\
                 或在连接串/extra 里给出 DSN",
            ));
        }
        if self.user.trim().is_empty() {
            return Err(config_error(
                "user 为空：达梦 ODBC 驱动会以 HY000 / -71008「需要更多信息」失败",
            ));
        }
        Ok(())
    }

    /// 该配置是否开启了驱动内建的读写分离（决定应用层路由能否启用）。
    pub fn native_rw_separate(&self) -> bool {
        self.rw_separate.unwrap_or(false)
    }

    // ----- 连接串的便利入口（实现在 crate::connstr） -----

    /// 按前缀自动分派解析：`dm://…` 走 URL，其余按 ODBC 连接串。
    ///
    /// ```
    /// # use dm_core::DmConfig;
    /// let a = DmConfig::parse("DRIVER={DM8 ODBC DRIVER};SERVER=h;UID=u;PWD=p").unwrap();
    /// let b = DmConfig::parse("dm://u:p@h:5236").unwrap();
    /// assert_eq!(a.server, b.server);
    /// ```
    pub fn parse(s: &str) -> Result<Self, DmError> {
        crate::connstr::parse(s)
    }

    /// 解析 ODBC 连接串（`KEY=VALUE;…`）。达梦 ODBC 驱动真正吃的形式。
    pub fn from_connection_string(s: &str) -> Result<Self, DmError> {
        crate::connstr::parse_odbc_string(s)
    }

    /// 解析本项目自定义的 `dm://user:pass@host:port/schema?k=v` 形式。
    pub fn from_url(s: &str) -> Result<Self, DmError> {
        crate::connstr::parse_dm_url(s)
    }

    /// 读环境变量并构造配置（显式环境变量 > 默认值）。
    ///
    /// 环境变量键名见 [`ENV_KEYS`]；口令相关变量只读取、不打印。
    pub fn from_env() -> Result<Self, DmError> {
        DmConfigBuilder::from_env().build()
    }

    /// 渲染成 ODBC 连接串。
    pub fn to_connection_string(&self) -> String {
        crate::connstr::render_odbc_string(self)
    }

    /// 渲染成 `dm://` URL 形式。
    pub fn to_url(&self) -> String {
        crate::connstr::render_dm_url(self)
    }
}

fn config_error(msg: impl Into<String>) -> DmError {
    DmError::at(DmErrorKind::Config(msg.into()), Stage::Config)
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// [`DmConfig`] 的结构化构造器。
///
/// 字段以 `Option` 保存，从而能区分「显式设过」与「没设」——
/// 这是实现「显式参数 > 环境变量 > 默认值」三级优先级的机制。
/// [`DmConfigBuilder::build`] 本身**不读环境变量**（保证可测）；
/// 需要环境变量回退时用 [`DmConfigBuilder::from_env`] 先播种。
#[derive(Debug, Clone, Default)]
pub struct DmConfigBuilder {
    driver: Option<String>,
    server: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    password: Option<SecretString>,
    schema: Option<String>,
    character_code: Option<String>,
    compatible_mode: Option<CompatMode>,
    rw_separate: Option<bool>,
    connection_timeout: Option<Duration>,
    login_timeout: Option<Duration>,
    init_sql: Option<Vec<String>>,
    charset_check: Option<CharsetCheck>,
    length_in_char: Option<bool>,
    slow_query_threshold: Option<Duration>,
    extra: BTreeMap<String, String>,
    pool: Option<PoolConfig>,
}

/// `from_env` 认得的全部环境变量名。
pub const ENV_KEYS: &[&str] = &[
    "DM_ODBC_DSN",
    "DM_DRIVER",
    "DM_SERVER",
    "DM_PORT",
    "DM_USER",
    "DM_PASSWORD",
    "DM_SCHEMA",
    "DM_CHARACTER_CODE",
    "DM_COMPATIBLE_MODE",
    "DM_RW_SEPARATE",
    "DM_CONNECTION_TIMEOUT",
    "DM_LOGIN_TIMEOUT",
    "DM_CHARSET_CHECK",
    "DM_LENGTH_IN_CHAR",
    "DM_SLOW_QUERY_MS",
    "DM_INIT_SQL",
    "DM_POOL_MAX_SIZE",
    "DM_POOL_MIN_IDLE",
    "DM_STMT_CACHE_SIZE",
];

impl DmConfigBuilder {
    /// 空构造器：一切走默认值。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从进程环境变量播种。
    ///
    /// - `DM_ODBC_DSN` 是一整条 ODBC 连接串，作为**基线**先解析进来，其余具名变量再覆盖它；
    /// - 逐一读取 [`ENV_KEYS`] 里的具名变量；
    /// - 解析失败的取值**忽略并保留原值**，不 panic（配置错误留给 `validate`/连接期报）。
    ///
    /// 之后再调用任何 setter 都能覆盖这里播种的值，于是三级优先级自然成立。
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// `from_env` 的可注入版本，便于用假环境做单元测试。
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut b = Self::new();

        // 1) 整条 DSN 作为基线
        if let Some(dsn) = lookup("DM_ODBC_DSN").filter(|s| !s.trim().is_empty())
            && let Ok(cfg) = crate::connstr::parse_odbc_string(&dsn)
        {
            b = b.seed_from(cfg);
        }

        // 2) 具名变量逐项覆盖
        if let Some(v) = lookup("DM_DRIVER") {
            b.driver = Some(v);
        }
        if let Some(v) = lookup("DM_SERVER") {
            b.server = Some(v);
        }
        if let Some(v) = lookup("DM_PORT").and_then(|v| v.trim().parse::<u16>().ok()) {
            b.port = Some(v);
        }
        if let Some(v) = lookup("DM_USER") {
            b.user = Some(v);
        }
        if let Some(v) = lookup("DM_PASSWORD") {
            b.password = Some(SecretString::new(v));
        }
        if let Some(v) = lookup("DM_SCHEMA") {
            b.schema = Some(v);
        }
        if let Some(v) = lookup("DM_CHARACTER_CODE") {
            b.character_code = Some(v);
        }
        if let Some(v) = lookup("DM_COMPATIBLE_MODE").and_then(|v| CompatMode::parse(&v)) {
            b.compatible_mode = Some(v);
        }
        if let Some(v) = lookup("DM_RW_SEPARATE").and_then(|v| parse_bool(&v)) {
            b.rw_separate = Some(v);
        }
        if let Some(v) = lookup("DM_CONNECTION_TIMEOUT").and_then(|v| parse_secs(&v)) {
            b.connection_timeout = Some(v);
        }
        if let Some(v) = lookup("DM_LOGIN_TIMEOUT").and_then(|v| parse_secs(&v)) {
            b.login_timeout = Some(v);
        }
        if let Some(v) = lookup("DM_CHARSET_CHECK").and_then(|v| CharsetCheck::parse(&v)) {
            b.charset_check = Some(v);
        }
        if let Some(v) = lookup("DM_LENGTH_IN_CHAR").and_then(|v| parse_bool(&v)) {
            b.length_in_char = Some(v);
        }
        // 0 是**显式关闭**（不是「没配」）：因此这里用 parse_secs 而不是 filter 掉 0。
        if let Some(v) = lookup("DM_SLOW_QUERY_MS").and_then(|v| v.trim().parse::<u64>().ok()) {
            b.slow_query_threshold = Some(Duration::from_millis(v));
        }
        if let Some(v) = lookup("DM_INIT_SQL") {
            b.init_sql = Some(
                v.split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            );
        }
        if let Some(v) = lookup("DM_POOL_MAX_SIZE").and_then(|v| v.trim().parse::<usize>().ok()) {
            let mut p = b.pool.clone().unwrap_or_default();
            p.max_size = v;
            b.pool = Some(p);
        }
        if let Some(v) = lookup("DM_POOL_MIN_IDLE").and_then(|v| v.trim().parse::<usize>().ok()) {
            let mut p = b.pool.clone().unwrap_or_default();
            p.min_idle = v;
            b.pool = Some(p);
        }
        // 同样：0 是**显式关闭语句缓存**（不是「没配」），所以不过滤 0。
        if let Some(v) = lookup("DM_STMT_CACHE_SIZE").and_then(|v| v.trim().parse::<usize>().ok()) {
            let mut p = b.pool.clone().unwrap_or_default();
            p.stmt_cache_size = v;
            b.pool = Some(p);
        }

        b
    }

    fn seed_from(mut self, cfg: DmConfig) -> Self {
        self.driver = Some(cfg.driver);
        self.server = Some(cfg.server);
        self.port = Some(cfg.port);
        self.user = Some(cfg.user);
        self.password = Some(cfg.password);
        self.schema = cfg.schema;
        self.character_code = cfg.character_code;
        self.compatible_mode = cfg.compatible_mode;
        self.rw_separate = cfg.rw_separate;
        self.connection_timeout = Some(cfg.connection_timeout);
        self.login_timeout = Some(cfg.login_timeout);
        self.init_sql = Some(cfg.init_sql);
        self.charset_check = Some(cfg.charset_check);
        self.length_in_char = cfg.length_in_char;
        self.slow_query_threshold = Some(cfg.slow_query_threshold);
        self.extra = cfg.extra;
        self.pool = Some(cfg.pool);
        self
    }

    /// ODBC 驱动名。
    pub fn driver(mut self, v: impl Into<String>) -> Self {
        self.driver = Some(v.into());
        self
    }

    /// 主机名或 IP。
    pub fn server(mut self, v: impl Into<String>) -> Self {
        self.server = Some(v.into());
        self
    }

    /// 端口。
    pub fn port(mut self, v: u16) -> Self {
        self.port = Some(v);
        self
    }

    /// 用户名。
    pub fn user(mut self, v: impl Into<String>) -> Self {
        self.user = Some(v.into());
        self
    }

    /// 口令。
    pub fn password(mut self, v: impl Into<SecretString>) -> Self {
        self.password = Some(v.into());
        self
    }

    /// 默认模式。
    pub fn schema(mut self, v: impl Into<String>) -> Self {
        self.schema = Some(v.into());
        self
    }

    /// 客户端字符集关键字。
    pub fn character_code(mut self, v: impl Into<String>) -> Self {
        self.character_code = Some(v.into());
        self
    }

    /// 兼容模式。
    pub fn compatible_mode(mut self, v: CompatMode) -> Self {
        self.compatible_mode = Some(v);
        self
    }

    /// 驱动内建读写分离开关。与 `dm-pool::router` 二选一。
    pub fn rw_separate(mut self, v: bool) -> Self {
        self.rw_separate = Some(v);
        self
    }

    /// 建连超时。
    pub fn connection_timeout(mut self, v: Duration) -> Self {
        self.connection_timeout = Some(v);
        self
    }

    /// 登录超时。
    pub fn login_timeout(mut self, v: Duration) -> Self {
        self.login_timeout = Some(v);
        self
    }

    /// 初始化 SQL（按顺序执行）。
    pub fn init_sql<I, S>(mut self, v: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.init_sql = Some(v.into_iter().map(Into::into).collect());
        self
    }

    /// 字符集自检策略。
    pub fn charset_check(mut self, v: CharsetCheck) -> Self {
        self.charset_check = Some(v);
        self
    }

    /// `varchar(n)` 的计量单位；`None` 表示连接期探测。
    pub fn length_in_char(mut self, v: Option<bool>) -> Self {
        self.length_in_char = v;
        self
    }

    /// 慢查询告警阈值（`Duration::ZERO` = 关闭）。
    pub fn slow_query_threshold(mut self, v: Duration) -> Self {
        self.slow_query_threshold = Some(v);
        self
    }

    /// 关闭慢查询告警（等价于阈值 `ZERO`）。
    pub fn without_slow_query_log(self) -> Self {
        self.slow_query_threshold(Duration::ZERO)
    }

    /// 追加一条原始连接串覆盖项（键会被转成大写）。
    pub fn extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra
            .insert(key.into().to_ascii_uppercase(), value.into());
        self
    }

    /// 整份池参数。
    pub fn pool(mut self, v: PoolConfig) -> Self {
        self.pool = Some(v);
        self
    }

    /// 池上限。
    pub fn pool_max_size(mut self, v: usize) -> Self {
        let mut p = self.pool.clone().unwrap_or_default();
        p.max_size = v;
        self.pool = Some(p);
        self
    }

    /// 保活下限。
    pub fn pool_min_idle(mut self, v: usize) -> Self {
        let mut p = self.pool.clone().unwrap_or_default();
        p.min_idle = v;
        self.pool = Some(p);
        self
    }

    /// 获取连接超时。
    pub fn pool_acquire_timeout(mut self, v: Duration) -> Self {
        let mut p = self.pool.clone().unwrap_or_default();
        p.acquire_timeout = v;
        self.pool = Some(p);
        self
    }

    /// 空闲回收时长。
    pub fn pool_idle_timeout(mut self, v: Duration) -> Self {
        let mut p = self.pool.clone().unwrap_or_default();
        p.idle_timeout = v;
        self.pool = Some(p);
        self
    }

    /// 连接最大生存期。
    pub fn pool_max_lifetime(mut self, v: Duration) -> Self {
        let mut p = self.pool.clone().unwrap_or_default();
        p.max_lifetime = v;
        self.pool = Some(p);
        self
    }

    /// 每条连接的预处理语句缓存条数（`0` = 关闭）。
    pub fn pool_stmt_cache_size(mut self, v: usize) -> Self {
        let mut p = self.pool.clone().unwrap_or_default();
        p.stmt_cache_size = v;
        self.pool = Some(p);
        self
    }

    /// 关掉语句缓存（等价于 `pool_stmt_cache_size(0)`）。
    ///
    /// 关闭后每条语句都走「新建句柄 → 预处理 → 执行 → 释放」，用于需要
    /// 「服务端不残留任何预处理语句」的场景（例如排查会话级资源占用）。
    pub fn without_stmt_cache(self) -> Self {
        self.pool_stmt_cache_size(0)
    }

    /// 组装并校验。
    pub fn build(self) -> Result<DmConfig, DmError> {
        let d = DmConfig::default();
        let cfg = DmConfig {
            driver: self.driver.unwrap_or(d.driver),
            server: self.server.unwrap_or(d.server),
            port: self.port.unwrap_or(d.port),
            user: self.user.unwrap_or(d.user),
            password: self.password.unwrap_or(d.password),
            schema: self.schema,
            character_code: self.character_code,
            compatible_mode: self.compatible_mode,
            rw_separate: self.rw_separate,
            connection_timeout: self.connection_timeout.unwrap_or(d.connection_timeout),
            login_timeout: self.login_timeout.unwrap_or(d.login_timeout),
            init_sql: self.init_sql.unwrap_or(d.init_sql),
            charset_check: self.charset_check.unwrap_or(d.charset_check),
            length_in_char: self.length_in_char,
            slow_query_threshold: self.slow_query_threshold.unwrap_or(d.slow_query_threshold),
            extra: self.extra,
            pool: self.pool.unwrap_or(d.pool),
        };
        cfg.validate()?;
        Ok(cfg)
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" | "y" => Some(true),
        "0" | "false" | "off" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn parse_secs(s: &str) -> Option<Duration> {
    s.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> DmConfigBuilder {
        DmConfig::builder()
            .server("localhost")
            .port(5236)
            .user("SYSDBA")
            .password("SYSDBA")
    }

    // ---------------- 默认值 ----------------

    #[test]
    fn defaults_match_the_design_table() {
        let c = DmConfig::default();
        assert_eq!(c.driver, "DM8 ODBC DRIVER");
        assert_eq!(c.port, 5236);
        assert_eq!(c.connection_timeout, Duration::from_secs(30));
        assert_eq!(c.login_timeout, Duration::from_secs(30));
        assert_eq!(c.charset_check, CharsetCheck::Error);
        assert_eq!(c.length_in_char, None);
        assert_eq!(
            c.slow_query_threshold, DEFAULT_SLOW_QUERY_THRESHOLD,
            "默认阈值 = 1 s（§6.9）"
        );
        assert!(c.schema.is_none());
        assert!(c.init_sql.is_empty());
        assert!(c.extra.is_empty());
        assert_eq!(c.compatible_mode, None);
        assert_eq!(c.rw_separate, None);
    }

    #[test]
    fn pool_defaults_match_the_design_table() {
        let p = PoolConfig::default();
        assert_eq!(p.max_size, 10);
        assert_eq!(p.min_idle, 1);
        assert_eq!(p.acquire_timeout, Duration::from_secs(30));
        assert_eq!(p.idle_timeout, Duration::from_secs(600));
        assert_eq!(p.max_lifetime, Duration::from_secs(1800));
        assert_eq!(p.health_check_interval, Duration::from_secs(30));
        assert!(!p.test_on_acquire);
        assert_eq!(p.recycle_interval, Duration::from_secs(5));
        // D11：取消后默认销毁连接，不复用
        assert!(!p.reuse_after_cancel);
        // CAP-C-11：默认开 16 条语句缓存
        assert_eq!(p.stmt_cache_size, DEFAULT_STMT_CACHE_SIZE);
        assert_eq!(p.stmt_cache_size, 16);
    }

    #[test]
    fn default_pool_is_self_consistent() {
        // 默认值本身必须能通过校验
        assert!(DmConfig::default().validate().is_ok());
    }

    // ---------------- builder ----------------

    #[test]
    fn builder_sets_every_structured_field() {
        let c = base()
            .driver("DM8 ODBC DRIVER")
            .schema("SYSDBA")
            .character_code("PG_UTF8")
            .compatible_mode(CompatMode::Oracle)
            .rw_separate(true)
            .connection_timeout(Duration::from_secs(5))
            .login_timeout(Duration::from_secs(7))
            .init_sql(["set schema SYSDBA", "alter session set time_zone='+08:00'"])
            .charset_check(CharsetCheck::Warn)
            .length_in_char(Some(false))
            .slow_query_threshold(Duration::from_millis(250))
            .extra("mp", "login")
            .pool_max_size(4)
            .pool_min_idle(2)
            .pool_acquire_timeout(Duration::from_secs(9))
            .pool_stmt_cache_size(3)
            .build()
            .expect("应构造成功");

        assert_eq!(c.driver, "DM8 ODBC DRIVER");
        assert_eq!(c.server, "localhost");
        assert_eq!(c.port, 5236);
        assert_eq!(c.user, "SYSDBA");
        assert_eq!(c.password.expose(), "SYSDBA");
        assert_eq!(c.schema.as_deref(), Some("SYSDBA"));
        assert_eq!(c.character_code.as_deref(), Some("PG_UTF8"));
        assert_eq!(c.compatible_mode, Some(CompatMode::Oracle));
        assert_eq!(c.rw_separate, Some(true));
        assert!(c.native_rw_separate());
        assert_eq!(c.connection_timeout, Duration::from_secs(5));
        assert_eq!(c.login_timeout, Duration::from_secs(7));
        assert_eq!(c.init_sql.len(), 2);
        assert_eq!(c.charset_check, CharsetCheck::Warn);
        assert_eq!(c.length_in_char, Some(false));
        assert_eq!(c.slow_query_threshold, Duration::from_millis(250));
        assert_eq!(c.extra.get("MP").map(String::as_str), Some("login"));
        assert_eq!(c.pool.max_size, 4);
        assert_eq!(c.pool.min_idle, 2);
        assert_eq!(c.pool.acquire_timeout, Duration::from_secs(9));
        assert_eq!(c.pool.stmt_cache_size, 3);
        // 未被覆盖的池参数保持默认
        assert_eq!(c.pool.max_lifetime, Duration::from_secs(1800));
    }

    #[test]
    fn builder_leaves_unset_fields_at_defaults() {
        let c = base().build().unwrap();
        assert_eq!(c.driver, DEFAULT_DRIVER);
        assert_eq!(c.charset_check, CharsetCheck::Error);
        assert_eq!(c.pool, PoolConfig::default());
        assert!(c.extra.is_empty());
    }

    #[test]
    fn extra_keys_are_uppercased_and_later_wins() {
        let c = base().extra("mp", "a").extra("MP", "b").build().unwrap();
        assert_eq!(c.extra.len(), 1);
        assert_eq!(c.extra.get("MP").map(String::as_str), Some("b"));
    }

    #[test]
    fn pool_setters_are_orthogonal() {
        // 逐个 setter 都不应把其它池参数重置
        let c = base()
            .pool_max_size(3)
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_lifetime(Duration::from_secs(120))
            .pool_stmt_cache_size(5)
            .build()
            .unwrap();
        assert_eq!(c.pool.max_size, 3);
        assert_eq!(c.pool.idle_timeout, Duration::from_secs(60));
        assert_eq!(c.pool.max_lifetime, Duration::from_secs(120));
        assert_eq!(c.pool.stmt_cache_size, 5);
        assert_eq!(c.pool.min_idle, 1, "min_idle 应保持默认");
        assert_eq!(
            c.pool.health_check_interval,
            Duration::from_secs(30),
            "其余应保持默认"
        );
    }

    #[test]
    fn whole_pool_struct_can_be_replaced() {
        let p = PoolConfig {
            max_size: 20,
            test_on_acquire: true,
            ..PoolConfig::default()
        };
        let c = base().pool(p.clone()).build().unwrap();
        assert_eq!(c.pool, p);
    }

    // ---------------- 校验 ----------------

    #[test]
    fn validate_rejects_empty_driver() {
        let e = base().driver("   ").build().unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("驱动名")));
        assert_eq!(e.context.stage, Stage::Config);
    }

    /// 空串 schema 会拼出 `SET SCHEMA ""`，在达梦上是必然失败的 SQL：提前拦掉。
    #[test]
    fn validate_rejects_blank_schema() {
        let e = base().schema("   ").build().unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("schema 为空串")));
        assert_eq!(e.context.stage, Stage::Config);
    }

    #[test]
    fn validate_rejects_zero_port() {
        let e = base().port(0).build().unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("端口")));
    }

    /// T1.2 实测补上的校验：`server` 为空且无 `DSN` 时会走到驱动里报
    /// `HY000 / -71008 需要更多信息`，失败点太靠后。
    #[test]
    fn connectable_rejects_empty_server_without_dsn() {
        let c = DmConfig::builder().user("SYSDBA").build().unwrap();
        let e = c.validate_connectable().unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("server 为空")));
        assert_eq!(e.context.stage, Stage::Config);
    }

    /// 有 `DSN` 时 `server` 可以为空——DSN 自带主机信息，这是合法的 ODBC 用法。
    #[test]
    fn connectable_allows_empty_server_when_dsn_is_present() {
        let c = DmConfig::builder()
            .extra("DSN", "DM8")
            .user("SYSDBA")
            .build()
            .unwrap();
        assert!(c.server.is_empty());
        assert_eq!(c.extra.get("DSN").map(String::as_str), Some("DM8"));
        assert!(c.validate_connectable().is_ok());
    }

    #[test]
    fn connectable_rejects_empty_user() {
        let c = DmConfig::builder().server("h").build().unwrap();
        let e = c.validate_connectable().unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("user 为空")));
    }

    /// `validate()`（配置自洽性）不该因为缺 server/user 而失败——那是
    /// `validate_connectable()` 的职责，二者分离已被这条测试钉住。
    #[test]
    fn validate_stays_independent_of_connectable() {
        let c = DmConfig::default();
        assert!(c.validate().is_ok());
        assert!(c.validate_connectable().is_err());
    }

    #[test]
    fn validate_rejects_zero_max_size() {
        let e = base().pool_max_size(0).build().unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("max_size")));
    }

    #[test]
    fn validate_rejects_min_idle_above_max_size() {
        let e = base()
            .pool_max_size(2)
            .pool_min_idle(3)
            .build()
            .unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("min_idle")));
    }

    #[test]
    fn validate_rejects_idle_timeout_not_below_max_lifetime() {
        // 相等也不行：否则空闲回收永不触发
        let e = base()
            .pool_idle_timeout(Duration::from_secs(1800))
            .pool_max_lifetime(Duration::from_secs(1800))
            .build()
            .unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("idle_timeout")));

        // 大于更不行
        let e2 = base()
            .pool_idle_timeout(Duration::from_secs(2000))
            .pool_max_lifetime(Duration::from_secs(1000))
            .build()
            .unwrap_err();
        assert!(matches!(e2.kind, DmErrorKind::Config(_)));

        // 严格小于就可以
        assert!(
            base()
                .pool_idle_timeout(Duration::from_secs(100))
                .pool_max_lifetime(Duration::from_secs(200))
                .build()
                .is_ok()
        );
    }

    #[test]
    fn validate_rejects_zero_acquire_timeout() {
        let e = base()
            .pool_acquire_timeout(Duration::ZERO)
            .build()
            .unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("acquire_timeout")));
    }

    #[test]
    fn all_config_errors_are_stage_config_and_not_retryable() {
        let cases = [
            base().driver("").build().unwrap_err(),
            base().port(0).build().unwrap_err(),
            base().pool_max_size(0).build().unwrap_err(),
        ];
        for e in cases {
            assert_eq!(e.context.stage, Stage::Config);
            assert!(!e.is_retryable(), "配置错误不该被重试");
            assert!(!e.is_connection_lost(), "配置错误与连接存活无关");
        }
    }

    // ---------------- 脱敏 ----------------

    #[test]
    fn secret_string_never_leaks_in_debug_or_display() {
        let s = SecretString::new("p@ssw0rd-绝密");
        let dbg = format!("{s:?}");
        let disp = format!("{s}");
        assert!(!dbg.contains("p@ssw0rd"), "Debug 泄露了口令：{dbg}");
        assert!(!dbg.contains("绝密"), "Debug 泄露了口令：{dbg}");
        assert!(!disp.contains("p@ssw0rd"), "Display 泄露了口令：{disp}");
        assert!(dbg.contains("已脱敏"));
        assert_eq!(disp, "***");
        // 但长度可见，便于排查「配了但配错」
        assert!(dbg.contains(&s.len().to_string()));
    }

    #[test]
    fn dm_config_debug_does_not_leak_password() {
        let c = base().password("TOP-SECRET-VALUE").build().unwrap();
        let dbg = format!("{c:?}");
        assert!(
            !dbg.contains("TOP-SECRET-VALUE"),
            "DmConfig 的 Debug 泄露了口令：{dbg}"
        );
        assert!(dbg.contains("已脱敏"));
        // 非敏感字段照常可见，保证 Debug 仍可用于排查
        assert!(dbg.contains("localhost"));
        assert!(dbg.contains("SYSDBA"));
    }

    #[test]
    fn secret_string_conversions_and_accessors() {
        let a: SecretString = "x".into();
        let b: SecretString = String::from("x").into();
        assert_eq!(a, b);
        assert_eq!(a.expose(), "x");
        assert_eq!(a.len(), 1);
        assert!(!a.is_empty());
        assert!(SecretString::default().is_empty());
    }

    // ---------------- 枚举解析 ----------------

    #[test]
    fn charset_check_parsing_is_case_insensitive_and_total_on_known_forms() {
        for s in ["off", "OFF", " none ", "false", "0"] {
            assert_eq!(CharsetCheck::parse(s), Some(CharsetCheck::Off), "{s}");
        }
        assert_eq!(CharsetCheck::parse("Warn"), Some(CharsetCheck::Warn));
        for s in ["error", "DENY", "true", "1"] {
            assert_eq!(CharsetCheck::parse(s), Some(CharsetCheck::Error), "{s}");
        }
        assert_eq!(CharsetCheck::parse("nonsense"), None);
        // 往返
        for v in [CharsetCheck::Off, CharsetCheck::Warn, CharsetCheck::Error] {
            assert_eq!(CharsetCheck::parse(v.as_str()), Some(v));
        }
    }

    #[test]
    fn compat_mode_parsing_and_rendering() {
        assert_eq!(CompatMode::parse("dm"), Some(CompatMode::Dm));
        assert_eq!(CompatMode::parse("0"), Some(CompatMode::Dm));
        assert_eq!(CompatMode::parse("Oracle"), Some(CompatMode::Oracle));
        assert_eq!(CompatMode::parse("ora"), Some(CompatMode::Oracle));
        assert_eq!(CompatMode::parse("1"), Some(CompatMode::Oracle));
        assert_eq!(CompatMode::parse("mysql"), None);
        for v in [CompatMode::Dm, CompatMode::Oracle] {
            assert_eq!(CompatMode::parse(v.as_str()), Some(v));
        }
        assert_eq!(CompatMode::Dm.as_ini_value(), 0);
        assert_eq!(CompatMode::Oracle.as_ini_value(), 1);
    }

    // ---------------- 环境变量分层 ----------------

    #[test]
    fn env_lookup_seeds_fields_absent_from_builder() {
        let env = |k: &str| match k {
            "DM_SERVER" => Some("10.0.0.9".to_string()),
            "DM_USER" => Some("ENVUSER".to_string()),
            "DM_PASSWORD" => Some("ENVPASS".to_string()),
            "DM_PORT" => Some("6000".to_string()),
            "DM_SCHEMA" => Some("ENVSCHEMA".to_string()),
            "DM_CHARACTER_CODE" => Some("PG_UTF8".to_string()),
            "DM_COMPATIBLE_MODE" => Some("oracle".to_string()),
            "DM_RW_SEPARATE" => Some("on".to_string()),
            "DM_CONNECTION_TIMEOUT" => Some("11".to_string()),
            "DM_LOGIN_TIMEOUT" => Some("12".to_string()),
            "DM_CHARSET_CHECK" => Some("warn".to_string()),
            "DM_LENGTH_IN_CHAR" => Some("false".to_string()),
            "DM_SLOW_QUERY_MS" => Some("250".to_string()),
            "DM_INIT_SQL" => Some("set schema A; set schema B".to_string()),
            "DM_POOL_MAX_SIZE" => Some("7".to_string()),
            "DM_POOL_MIN_IDLE" => Some("2".to_string()),
            "DM_STMT_CACHE_SIZE" => Some("5".to_string()),
            _ => None,
        };
        let c = DmConfigBuilder::from_lookup(env).build().unwrap();

        assert_eq!(c.server, "10.0.0.9");
        assert_eq!(c.user, "ENVUSER");
        assert_eq!(c.password.expose(), "ENVPASS");
        assert_eq!(c.port, 6000);
        assert_eq!(c.schema.as_deref(), Some("ENVSCHEMA"));
        assert_eq!(c.character_code.as_deref(), Some("PG_UTF8"));
        assert_eq!(c.compatible_mode, Some(CompatMode::Oracle));
        assert_eq!(c.rw_separate, Some(true));
        assert_eq!(c.connection_timeout, Duration::from_secs(11));
        assert_eq!(c.login_timeout, Duration::from_secs(12));
        assert_eq!(c.charset_check, CharsetCheck::Warn);
        assert_eq!(c.length_in_char, Some(false));
        assert_eq!(
            c.slow_query_threshold,
            Duration::from_millis(250),
            "DM_SLOW_QUERY_MS 是毫秒"
        );
        assert_eq!(c.init_sql, vec!["set schema A", "set schema B"]);
        assert_eq!(c.pool.max_size, 7);
        assert_eq!(c.pool.min_idle, 2);
        assert_eq!(c.pool.stmt_cache_size, 5, "DM_STMT_CACHE_SIZE 条数");
        // 驱动名环境里没给，走默认
        assert_eq!(c.driver, DEFAULT_DRIVER);
    }

    #[test]
    fn explicit_builder_beats_env() {
        let env = |k: &str| match k {
            "DM_SERVER" => Some("env-host".to_string()),
            "DM_PORT" => Some("6000".to_string()),
            _ => None,
        };
        let c = DmConfigBuilder::from_lookup(env)
            .server("explicit-host")
            .port(7000)
            .build()
            .unwrap();
        assert_eq!(c.server, "explicit-host", "显式参数必须压过环境变量");
        assert_eq!(c.port, 7000);
    }

    #[test]
    fn dsn_env_is_a_baseline_that_named_vars_override() {
        let env = |k: &str| match k {
            "DM_ODBC_DSN" => Some(
                "DRIVER={DM8 ODBC DRIVER};SERVER=dsn-host;TCP_PORT=1234;UID=dsnuser;PWD=dsnpass"
                    .to_string(),
            ),
            "DM_SERVER" => Some("named-host".to_string()),
            _ => None,
        };
        let c = DmConfigBuilder::from_lookup(env).build().unwrap();
        // 具名变量覆盖 DSN
        assert_eq!(c.server, "named-host");
        // DSN 里其余的值保留
        assert_eq!(c.port, 1234);
        assert_eq!(c.user, "dsnuser");
        assert_eq!(c.password.expose(), "dsnpass");
    }

    #[test]
    fn malformed_env_values_are_ignored_not_fatal() {
        let env = |k: &str| match k {
            "DM_PORT" => Some("not-a-port".to_string()),
            "DM_POOL_MAX_SIZE" => Some("-3".to_string()),
            "DM_CHARSET_CHECK" => Some("nonsense".to_string()),
            "DM_COMPATIBLE_MODE" => Some("mysql".to_string()),
            "DM_ODBC_DSN" => Some("<<<garbage>>>".to_string()),
            "DM_SERVER" => Some("ok-host".to_string()),
            _ => None,
        };
        let c = DmConfigBuilder::from_lookup(env).build().unwrap();
        assert_eq!(c.port, DEFAULT_PORT, "非法端口应回落到默认而不是 panic");
        assert_eq!(c.pool.max_size, 10);
        assert_eq!(c.charset_check, CharsetCheck::Error);
        assert_eq!(c.compatible_mode, None);
        assert_eq!(c.server, "ok-host");
    }

    /// 慢查询阈值的三个口径（§6.9 / AC-09）：默认 1 s、`ZERO` = 关闭、
    /// 环境里的 `0` 是**显式关闭**而不是「没配」（所以它必须压过默认值）。
    #[test]
    fn slow_query_threshold_defaults_disables_and_reads_env() {
        assert_eq!(
            DmConfig::default().slow_query_threshold,
            DEFAULT_SLOW_QUERY_THRESHOLD
        );
        let off = base().without_slow_query_log().build().unwrap();
        assert_eq!(off.slow_query_threshold, Duration::ZERO);

        let env = |k: &str| match k {
            "DM_SLOW_QUERY_MS" => Some("0".to_string()),
            _ => None,
        };
        assert_eq!(
            DmConfigBuilder::from_lookup(env)
                .build()
                .unwrap()
                .slow_query_threshold,
            Duration::ZERO,
            "0 是显式关闭"
        );

        let env = |k: &str| match k {
            "DM_SLOW_QUERY_MS" => Some(" 1500 ".to_string()),
            _ => None,
        };
        assert_eq!(
            DmConfigBuilder::from_lookup(env)
                .build()
                .unwrap()
                .slow_query_threshold,
            Duration::from_millis(1500)
        );
    }

    #[test]
    fn empty_dsn_env_is_skipped() {
        let env = |k: &str| match k {
            "DM_ODBC_DSN" => Some("   ".to_string()),
            _ => None,
        };
        let c = DmConfigBuilder::from_lookup(env).build().unwrap();
        assert_eq!(c.server, "");
        assert_eq!(c.driver, DEFAULT_DRIVER);
    }

    #[test]
    fn env_keys_list_is_stable() {
        // 防回归：env 变量名单一旦变动，使用方的部署脚本要跟着改
        assert_eq!(ENV_KEYS.len(), 19);
        assert!(ENV_KEYS.contains(&"DM_ODBC_DSN"));
        assert!(ENV_KEYS.contains(&"DM_PASSWORD"));
        assert!(ENV_KEYS.contains(&"DM_SLOW_QUERY_MS"));
        assert!(ENV_KEYS.contains(&"DM_STMT_CACHE_SIZE"));
        for k in ENV_KEYS {
            assert!(k.starts_with("DM_"), "{k} 应以 DM_ 开头");
        }
    }

    /// 语句缓存条数的三个口径（CAP-C-11）：默认 16、`0` = 关闭、
    /// 环境里的 `0` 是**显式关闭**而不是「没配」。
    #[test]
    fn stmt_cache_size_defaults_disables_and_reads_env() {
        assert_eq!(
            DmConfig::default().pool.stmt_cache_size,
            DEFAULT_STMT_CACHE_SIZE
        );
        let off = base().without_stmt_cache().build().unwrap();
        assert_eq!(off.pool.stmt_cache_size, 0);

        let env = |k: &str| match k {
            "DM_STMT_CACHE_SIZE" => Some("0".to_string()),
            _ => None,
        };
        assert_eq!(
            DmConfigBuilder::from_lookup(env)
                .build()
                .unwrap()
                .pool
                .stmt_cache_size,
            0,
            "0 是显式关闭"
        );

        let env = |k: &str| match k {
            "DM_STMT_CACHE_SIZE" => Some(" 64 ".to_string()),
            _ => None,
        };
        assert_eq!(
            DmConfigBuilder::from_lookup(env)
                .build()
                .unwrap()
                .pool
                .stmt_cache_size,
            64
        );
    }

    // ---------------- 其它 ----------------

    #[test]
    fn native_rw_separate_reflects_the_flag() {
        assert!(!DmConfig::default().native_rw_separate());
        assert!(!base().build().unwrap().native_rw_separate());
        assert!(
            base()
                .rw_separate(true)
                .build()
                .unwrap()
                .native_rw_separate()
        );
    }

    #[test]
    fn helper_parsers() {
        for s in ["1", "true", "ON", "yes", "Y"] {
            assert_eq!(parse_bool(s), Some(true), "{s}");
        }
        for s in ["0", "false", "off", "no", "N"] {
            assert_eq!(parse_bool(s), Some(false), "{s}");
        }
        assert_eq!(parse_bool("maybe"), None);
        assert_eq!(parse_secs("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_secs(" 30 "), Some(Duration::from_secs(30)));
        assert_eq!(parse_secs("-1"), None);
        assert_eq!(parse_secs("1.5"), None);
    }

    #[test]
    fn init_sql_can_be_empty_and_optional_fields_can_be_none() {
        let c = base()
            .init_sql(Vec::<String>::new())
            .schema("S")
            .build()
            .unwrap();
        assert!(c.init_sql.is_empty());
        assert_eq!(c.schema.as_deref(), Some("S"));
    }
}
