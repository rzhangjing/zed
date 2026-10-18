//! 连接建立（T3.1，设计方案 §5.5 八步 + §8.3 连接期字符集自检）。
//!
//! # 八步的落点
//!
//! | 步 | 动作 | 这里怎么落 |
//! |---|---|---|
//! | ① | 解析并校验配置 | [`DmConfig::validate`] + [`DmConfig::validate_connectable`] |
//! | ② | 校验驱动注册 | [`check_registered`] → `DriverNotFound{tried,hint}` |
//! | ③ | 进程级 `Environment` | `odbc_api::environment()`（它自己就是 `OnceLock`） |
//! | ④ | `SQLDriverConnect`（**无 DSN**） | [`render_odbc_string`] + `ConnectionOptions{login_timeout_sec}` |
//! | ⑤ | 设会话属性 | auto-commit 走 `odbc-api`；隔离级别与连接超时走裸 `SQLSetConnectAttrW` |
//! | ⑥ | 字符集自检 | `select unicode() from dual`，按 [`CharsetCheck`] 处置 |
//! | ⑦ | `init_sql` 逐条执行 | 逐条 `execute`，结果集靠 `CursorImpl::drop` 关 |
//! | ⑧ | 写 `ConnStats` | [`ConnStats`] |
//!
//! # 两条必须说清楚的取舍
//!
//! 1. **探活不跑 SQL**：`ping` 走 `SQL_ATTR_CONNECTION_DEAD`（§4.2：一次往返 p50 29–34 µs，
//!    探活不该花这个钱，也不该在服务端留下语句痕迹）。
//! 2. **属性设置的先后顺序**：`odbc-api` 的高层 `Connection` **不暴露 `HDbc`**
//!    （只有 [`odbc_api::handles::Connection`] 有 `as_sys()`，而拿到它要先把高层连接
//!    `into_handle()` 消费掉；**`odbc-api::Connection` 上没有 `as_sys()`**）。所以会话属性分两批：影响 `init_sql` 的 **auto-commit**
//!    在自检与 `init_sql` **之前**设；只影响后续语句的 **隔离级别 / 连接超时** 在
//!    `into_handle()` 之后设。若 `init_sql` 依赖隔离级别，请改用连接串关键字表达
//!    （登记为 §14 待验证项）。

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dm_core::driver_registry::{DRIVER_NOT_FOUND_HINT, probe_cached};
use dm_core::{CharsetCheck, DmConfig, IsolationLevel, check_registered, render_odbc_string};
use dm_error::{DmError, DmErrorKind, ErrorContext, Stage};
use odbc_api::handles::{Connection, SqlResult, slice_to_cow_utf8};
use odbc_api::{ConnectionOptions, Cursor};
use odbc_sys::{ConnectionAttribute, Pointer, SQLSetConnectAttrW, SqlReturn};

use crate::diag;

/// 自检 SQL。达梦的 `unicode()` 返回**服务端**字符集编号（§4.1 实测默认 `0` = GB18030）。
const CHARSET_PROBE_SQL: &str = "select unicode() from dual";

/// 会话自检 SQL：字符集编号 + **当前模式名**，一条查询取回两件事。
///
/// 之所以并成一条：`SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID)` 正是「我现在在哪个模式」，
/// 而 [`DmConfig::schema`] 必须在建连时生效又不想为它多付一次往返——模式已经就位时
/// 连 `SET SCHEMA` 都不必发（`examples/schema_probe.rs` 实测：连接串里的 `SCHEMA=` /
/// `SCHEMA_NAME=` / `CURRENT_SCHEMA=` 达梦 ODBC 一律忽略，只有 `SET SCHEMA` 有效）。
const SESSION_PROBE_SQL: &str =
    "select unicode(), SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual";

/// 只探当前模式的 SQL（`charset_check=off` 但仍配了 `schema` 时用）。
const SCHEMA_PROBE_SQL: &str = "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual";

/// `unicode()` = GB18030（达梦安装默认，§4.1 实测）。
pub const UNICODE_GB18030: i32 = 0;

/// 本项目**声明支持**的服务端字符集编号。
///
/// §8.1（设计方案 L973）：服务端字符集在 `dminit` 时确定、**不可变**，本机 = GB18030（`unicode() = 0`）。
/// 全项目只在 GB18030 上做过实测（§4、T1.2、§7 的类型三方比对也都在这个库上跑），
/// 所以「支持集」现在只有这一个值。遇到别的值不是要静默继续，而是要让 §8.3 的
/// 强制自检拦下来——那时我们的全部长度/编码假设都没被验证过。
///
/// ⚠️ 这里**刻意不登记**任何未实测的编号（早先版本曾猜 `PG_UTF8 → 1`，真机证伪）。
pub const SUPPORTED_SERVER_UNICODE: &[i32] = &[UNICODE_GB18030];

/// 自检探针的宽字符缓冲（单元数）。`unicode()` 是个小整数，16 个单元绰绰有余。
const CHARSET_PROBE_BUF: usize = 16;

// ---------------------------------------------------------------------------
// 连接统计
// ---------------------------------------------------------------------------

/// 一条连接的只读快照。
///
/// 只放**dm-odbc 自己知道**的事实：建连耗时、服务端字符集、最后使用时刻。
/// 执行次数/耗时/慢查询等原子计数属于 `dm-pool`（设计方案 §9.1），不在这里。
#[derive(Debug, Clone)]
pub struct ConnStats {
    /// 连接变为可用（自检与 `init_sql` 都跑完）的时刻。
    pub created_at: Instant,
    /// 最后一次经本连接发起动作的时刻（`ping`/`commit`/执行语句）。
    pub last_used: Instant,
    /// 从进入 [`OdbcConnection::connect`] 到连接可用的耗时（微秒）。
    pub connect_micros: u64,
    /// `select unicode()` 的实测值；没测到（未开自检 / 查询失败）时为 `None`。
    pub server_unicode: Option<i32>,
    /// 是否真的比对过一次字符集（`charset_check = off` 或证据不足时为 `false`）。
    pub charset_checked: bool,
}

// ---------------------------------------------------------------------------
// 连接
// ---------------------------------------------------------------------------

/// 一条达梦 ODBC 物理连接。
///
/// 生命周期：构造即建连（[`OdbcConnection::connect`]），`Drop` 即断开；
/// 想拿到断开时的错误（例如网络已经断了）请显式调用 [`OdbcConnection::close`]。
///
/// 所有会改变服务端状态的方法都要 `&mut self`，所以**同一个连接不可能被两个线程同时用**；
/// 这正是 `Send` 的依据（与 `odbc-api` 自己的 `unsafe impl Send for Connection<'_>` 同理）。
pub struct OdbcConnection {
    handle: Connection<'static>,
    cfg: DmConfig,
    stats: ConnStats,
    /// 已经断开过。`Drop` 不能报错，而 `close(self)` 要报错，用这个标记避免断开两次
    /// （第二次 `SQLDisconnect` 只会得到一个无意义的句柄错）。
    closed: bool,
    /// 当前的 `SQL_ATTR_AUTOCOMMIT`。
    ///
    /// 建连时置 `ON`（§5.5 步骤⑤a）。**跟踪它是为了能安全断开**：真机实测（T3.7，
    /// `examples/txn_probe.rs`）达梦在「`autocommit = OFF` 且事务还开着」时
    /// `SQLDisconnect` 会返回成功，但紧接着的 `SQLFreeHandle(SQL_HANDLE_DBC)` 回 `-1`，
    /// 而 `odbc-api` 的 `Drop` 会为此 **panic**。所以断开前必须先 `SQLEndTran(SQL_ROLLBACK)`
    /// 把事务收掉——见 [`OdbcConnection::disconnect`]。
    autocommit: bool,
    /// 尚未释放的语句句柄数。
    ///
    /// 语句句柄是连接句柄的子句柄，必须在连接**之前**释放（契约 CT-09：`stmt → conn → env`）。
    /// 语句借用连接时这条顺序由借用检查器保证；但 [`OdbcConnection::allocate_statement_owned`]
    /// 交出的 `OdbcStatement<'static>` 不借用任何东西（`Backend::Stmt` 要求 `'static`），
    /// 这条顺序就只剩约定。这里用一个计数把它从「注释里的约定」变成**可观测的事实**：
    /// 断开时若还有活语句，会留下一条 `warn!`，而不是等到某次 `SQLFreeStmt` 打在已释放的
    /// 父句柄上才发作。
    live_statements: Arc<AtomicUsize>,
}

// SAFETY: ODBC 句柄本身可以在任意线程使用，只要同一时刻只有一个线程碰它
// （见 odbc-api 的 `unsafe impl Send for Connection<'_>`，源码 `src/connection.rs:88`）。
// 本类型把这条约束编码进了方法签名：所有会动服务端状态的方法都取 `&mut self`，
// 借用检查器保证同一连接不会被并发访问。`odbc-api` 只对高层 `Connection` 而不是
// `handles::Connection` 实现了 `Send`，所以我们在这里补上，理由与它完全相同。
unsafe impl Send for OdbcConnection {}

impl OdbcConnection {
    /// 建一条连接（§5.5 八步）。
    ///
    /// 失败一律是 [`DmError`]。**凡发起了 ODBC 调用的失败必然带诊断**（驱动一条记录都不给时
    /// 也会造一条「未提供诊断」的记录，§6.2）；唯一的例外是第 ② 步的注册表预检——
    /// 那时还没碰 ODBC，所以不可能有诊断，取而代之的是 `DriverNotFound { tried, hint }`
    /// 里的可操作提示（编一条假 `SQLSTATE` 比没有诊断更糟）。
    pub fn connect(cfg: &DmConfig) -> Result<Self, DmError> {
        let started = Instant::now();

        // ① 配置：先管自洽性，再管「能不能真的发起一次连接」。
        cfg.validate()?;
        cfg.validate_connectable()?;

        // ② 驱动注册校验（Windows 注册表 / 麒麟 `odbcinst.ini`）。
        let info = check_registered(cfg)?;
        tracing::debug!(
            driver = %cfg.driver,
            path = ?info.driver_path,
            origin = ?info.origin,
            "ODBC 驱动已注册"
        );

        // ③ 进程级环境（`odbc_api::environment()` 内部就是一个 `OnceLock`）。
        let env = odbc_api::environment().map_err(|err| {
            diag::from_odbc_error(err, Stage::DriverLoad, "SQLAllocHandle(SQL_HANDLE_ENV)")
        })?;

        // ④ SQLDriverConnect：驱动名、主机、账号全部写在连接串里，不用 DSN。
        let conn_str = render_odbc_string(cfg);
        let options = ConnectionOptions {
            login_timeout_sec: Some(secs_of(cfg.login_timeout)),
            ..Default::default()
        };
        let guard = env
            .connect_with_connection_string(&conn_str, options)
            .map_err(connect_failed)?;

        // ⑤a auto-commit：必须在自检与 init_sql 之前设好——它们会执行 SQL。
        //     达梦默认就是 autocommit on，这里显式设一次，不依赖驱动的默认值。
        //     注意这条走的是高层 `Connection::set_autocommit`（返回 `Result<(), Error>`），
        //     而不是 `handles::Connection` 上那个 `SqlResult` 版本。
        guard.set_autocommit(true).map_err(|err| {
            diag::from_odbc_error(
                err,
                Stage::Connect,
                "SQLSetConnectAttr(SQL_ATTR_AUTOCOMMIT)",
            )
        })?;

        // ⑥ 会话自检（§8.3）：一次查询同时拿到服务端字符集与**当前模式**。
        let session = inspect_session(&guard, cfg)?;

        // ⑥′ 默认模式（`DmConfig::schema`）：达梦连接串里的 `SCHEMA=` 无效，只能 `SET SCHEMA`；
        //      当前模式已经对得上就一条 SQL 都不发。
        apply_schema(&guard, cfg, session.current_schema.as_deref())?;
        let (server_unicode, charset_checked) = (session.unicode, session.charset_checked);

        // ⑦ init_sql。
        run_init_sql(&guard, cfg)?;

        // ⑧ 转成自有句柄（`into_handle` 不触发 Drop，句柄所有权跟着搬过来），
        //     再补设只影响后续语句的属性，最后写统计。
        let handle = guard.into_handle();
        let now = Instant::now();
        let conn = Self {
            handle,
            cfg: cfg.clone(),
            stats: ConnStats {
                created_at: now,
                last_used: now,
                connect_micros: micros_of(started.elapsed()),
                server_unicode,
                charset_checked,
            },
            closed: false,
            // §5.5 步骤⑤a 建连时就把 `SQL_ATTR_AUTOCOMMIT` 设成 `ON`。
            autocommit: true,
            live_statements: Arc::new(AtomicUsize::new(0)),
        };
        conn.apply_session_attrs()?;
        Ok(conn)
    }

    /// ⑤b 只影响后续语句的会话属性。
    fn apply_session_attrs(&self) -> Result<(), DmError> {
        // 隔离级别：达梦默认 READ COMMITTED，这里显式设一次（§5.10 声明支持 4 档）。
        // 等 `DmConfig` 长出隔离级别字段后，这里改成从配置取。
        set_connect_attr_int(
            &self.handle,
            ConnectionAttribute::TXN_ISOLATION,
            IsolationLevel::ReadCommitted.odbc_value(),
            "SQLSetConnectAttrW(SQL_ATTR_TXN_ISOLATION)",
        )?;

        // 连接超时：ODBC 只认整秒。
        set_connect_attr_int(
            &self.handle,
            ConnectionAttribute::CONNECTION_TIMEOUT,
            i32::try_from(secs_of(self.cfg.connection_timeout)).unwrap_or(i32::MAX),
            "SQLSetConnectAttrW(SQL_ATTR_CONNECTION_TIMEOUT)",
        )?;

        Ok(())
    }

    /// 探活。`Ok(false)` = 连接已死（**不是**错误），`Err` = 探测本身没做成。
    ///
    /// 走 `SQL_ATTR_CONNECTION_DEAD`，不发任何 SQL。
    pub fn ping(&mut self) -> Result<bool, DmError> {
        self.touch();
        let dead = sql_ok(
            self.handle.is_dead(),
            &self.handle,
            Stage::Other,
            "SQLGetConnectAttr(SQL_ATTR_CONNECTION_DEAD)",
        )?;
        Ok(!dead)
    }

    /// 提交。
    pub fn commit(&mut self) -> Result<(), DmError> {
        self.touch();
        sql_ok(
            self.handle.commit(),
            &self.handle,
            Stage::Execute,
            "SQLEndTran(SQL_COMMIT)",
        )
    }

    /// 回滚。
    pub fn rollback(&mut self) -> Result<(), DmError> {
        self.touch();
        sql_ok(
            self.handle.rollback(),
            &self.handle,
            Stage::Execute,
            "SQLEndTran(SQL_ROLLBACK)",
        )
    }

    /// 显式断开，把 `SQLDisconnect` 的错误交给调用方。
    ///
    /// [`Drop`] 里同一件事做了但吞掉了错误——所以「必须知道服务端有没有干净断开」的
    /// 调用点（连接池缩容、优雅停机）应当调这个。
    pub fn close(mut self) -> Result<(), DmError> {
        self.disconnect()
    }

    /// 断开一次，幂等。
    fn disconnect(&mut self) -> Result<(), DmError> {
        if self.closed {
            return Ok(());
        }
        // 先立标记：即使 `SQLDisconnect` 报错也不能再试第二次，否则 `Drop` 会重复断开。
        self.closed = true;
        let live = self.live_statement_count();
        if live > 0 {
            // 不 panic：这条路径在 `Drop` 里也会走，而 `Drop` 里 panic 会把「关连接失败」
            // 升级成「进程终止」。留一条可观测的日志，够定位了。
            tracing::warn!(
                live_statements = live,
                "断开连接时仍有语句句柄未释放：语句必须在连接之前析构（CT-09）"
            );
        }

        // **断开前必须把事务收掉**（真机实测，T3.7）。达梦在「`autocommit = OFF` 且事务还开着」
        // 时的表现是：`SQLDisconnect` 回 `SQL_SUCCESS`，但随后 `SQLFreeHandle(SQL_HANDLE_DBC)`
        // 回 `-1`；`odbc-api` 的 `Drop` 见到 `-1` 就 **panic**——于是「正常关连接」变成进程里
        // 一次无法捕获于用户的崩溃。触发面很宽：`begin()` 之后哪怕只跑了一句 `SELECT` 也算
        // 事务开着，所以「关连接」这条路径不能假设调用方一定显式 `commit`/`rollback` 过。
        //
        // 这里选**回滚**而不是 `SQLSetConnectAttr(AUTOCOMMIT, ON)`：按 ODBC 规范后者会**提交**
        // 未完成的事务，等于替调用方做了「保存」这个破坏性决定。回滚是唯一安全的默认。
        if !self.autocommit
            && let Err(err) = sql_ok(
                self.handle.rollback(),
                &self.handle,
                Stage::Execute,
                "SQLEndTran(SQL_ROLLBACK)",
            )
        {
            // 收尾路径不把错误升级：真正的目标是「让句柄能被释放」，
            // 而这件事即使回滚失败也未必做不到。
            tracing::warn!(error = %err, "断开前回滚未完成事务失败");
        }

        sql_ok(
            self.handle.disconnect(),
            &self.handle,
            Stage::Close,
            "SQLDisconnect",
        )
    }

    /// 开始一个事务：把 auto-commit 关掉（ODBC 里「开始事务」就是这个动作，
    /// 下一条语句起生效）。
    ///
    /// 达梦与 ODBC 都不需要单独的 `BEGIN`：`SQL_ATTR_AUTOCOMMIT = OFF` 之后，
    /// 语句隐式开启事务，直到 [`OdbcConnection::commit`] 或 [`OdbcConnection::rollback`]。
    pub fn begin(&mut self) -> Result<(), DmError> {
        self.set_autocommit(false)
    }

    /// 设 auto-commit 开关。
    ///
    /// 建连时已设为 `ON`（§5.5 步骤⑤a），这里供事务层使用。
    /// `odbc-api` 只在**高层** `Connection` 上有 `set_autocommit`，而句柄在
    /// `into_handle()` 之后已经是 `handles::Connection`，所以这条走裸
    /// `SQLSetConnectAttrW`——整数属性直接塞进指针参数，不会被解引用。
    pub fn set_autocommit(&mut self, enabled: bool) -> Result<(), DmError> {
        self.touch();
        set_connect_attr_int(
            &self.handle,
            ConnectionAttribute::AUTOCOMMIT,
            i32::from(enabled),
            "SQLSetConnectAttrW(SQL_ATTR_AUTOCOMMIT)",
        )?;
        self.autocommit = enabled;
        Ok(())
    }

    /// 当前的 `SQL_ATTR_AUTOCOMMIT`（建连时是 `true`）。
    #[must_use]
    pub fn is_autocommit(&self) -> bool {
        self.autocommit
    }

    /// 尚未释放的语句句柄数（CT-09 的可观测面）。
    pub fn live_statement_count(&self) -> usize {
        self.live_statements.load(Ordering::Relaxed)
    }

    /// 给新分配的语句句柄领一张「在册」凭证。
    pub(crate) fn statement_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.live_statements)
    }

    fn touch(&mut self) {
        self.stats.last_used = Instant::now();
    }

    /// 建连时实际使用的配置（密码是 [`dm_core::SecretString`]，`Debug` 里不会出现）。
    pub fn config(&self) -> &DmConfig {
        &self.cfg
    }

    /// 只读统计快照。
    pub fn stats(&self) -> &ConnStats {
        &self.stats
    }

    /// `select unicode()` 的实测值（未自检时为 `None`）。
    pub fn server_unicode(&self) -> Option<i32> {
        self.stats.server_unicode
    }

    /// 底层连接句柄。语句/参数/结果集三层（T3.3–T3.5）靠它分配语句。
    pub fn handle(&self) -> &Connection<'static> {
        &self.handle
    }

    /// 可变句柄。在它上面设属性/分配语句。
    pub fn handle_mut(&mut self) -> &mut Connection<'static> {
        &mut self.handle
    }
}

impl fmt::Debug for OdbcConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 刻意不打印 `self.cfg`：`DmConfig` 里有密码（虽然 `SecretString` 已做脱敏，
        // 但少一处泄露面总是好的）。这里只列不会泄密的字段。
        f.debug_struct("OdbcConnection")
            .field("server", &self.cfg.server)
            .field("port", &self.cfg.port)
            .field("user", &self.cfg.user)
            .field("driver", &self.cfg.driver)
            .field("unicode", &self.stats.server_unicode)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl Drop for OdbcConnection {
    fn drop(&mut self) {
        if let Err(err) = self.disconnect() {
            // `Drop` 不能报错。要错误请用 `close(self)`。
            tracing::debug!(error = %err, "关闭 ODBC 连接时出错（已忽略）");
        }
    }
}

// ---------------------------------------------------------------------------
// ⑤ 会话属性
// ---------------------------------------------------------------------------

/// 设一个**整数值**的连接属性。
///
/// ODBC 规定整数属性把值直接塞在指针参数里、`StringLength` 被忽略（这里传 `0`）。
/// `odbc-api` 只包装了 auto-commit / login-timeout / packet-size 三个连接属性，
/// 隔离级别与连接超时只能下探 `odbc-sys`。
///
/// 本函数是安全的：整数属性不会被解引用，句柄有效性由 `&Connection` 的生命周期保证。
fn set_connect_attr_int(
    handle: &Connection<'_>,
    attr: ConnectionAttribute,
    value: i32,
    function: &'static str,
) -> Result<(), DmError> {
    // SAFETY: `handle.as_sys()` 在 `handle` 存活期间有效（借用检查器保证），
    // 且 `SQLSetConnectAttrW` 对整数属性不会解引用 `value`。
    let rc = unsafe { SQLSetConnectAttrW(handle.as_sys(), attr, value as usize as Pointer, 0) };
    if rc == SqlReturn::SUCCESS || rc == SqlReturn::SUCCESS_WITH_INFO {
        Ok(())
    } else {
        Err(diag::collect_and_classify(handle, Stage::Connect, function))
    }
}

// ---------------------------------------------------------------------------
// ⑥ 字符集自检
// ---------------------------------------------------------------------------

/// `CHARACTER_CODE` 是**客户端**字符集关键字，**不能**拿去和 `unicode()`（服务端字符集）比。
///
/// 这一条是真机证伪出来的：`CHARACTER_CODE=PG_UTF8` 时 `unicode()` 仍然返回 `0`，
/// 而那条连接完全正常（T1.2 的 spike 用 `get_wide_text` 读回了正确的中文）。
/// 原因是 §8.1（设计方案 L973）：服务端字符集在 `dminit` 时确定、不可变，
/// `CHARACTER_CODE` 只改变驱动侧的转换目标。
///
/// 所以本模块的判据是「服务端字符集是否落在 [`SUPPORTED_SERVER_UNICODE`] 里」，
/// `character_code` 只记日志、不参与判定。若把两者混为一谈，
/// 默认的 `charset_check=error` 会拒绝一个**工作正常**的 `PG_UTF8` 配置——
/// 这是比漏检更糟的「假阳性阻断」。
fn charset_baseline(actual: i32) -> CharsetVerdict {
    if SUPPORTED_SERVER_UNICODE.contains(&actual) {
        CharsetVerdict::Agree
    } else {
        CharsetVerdict::Mismatch {
            // `configured` 是**本项目声明支持的基线**，不是 `character_code`：
            // `DmConfig` 里没有「期望的服务端字符集」字段（服务端字符集不可配，它由 dminit 定死）。
            configured: UNICODE_GB18030,
            actual,
        }
    }
}

/// 自检的三态结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharsetVerdict {
    /// 服务端字符集在我们验证过、声明支持的集合里。
    Agree,
    /// 服务端字符集不是我们验证过的那个（§8.3 的「不一致」）。
    Mismatch { configured: i32, actual: i32 },
}

fn judge_charset(actual: i32) -> CharsetVerdict {
    charset_baseline(actual)
}

/// 建连自检的产物。
struct SessionFacts {
    /// 服务端字符集编号；`charset_check=off` 或取不到时是 `None`。
    unicode: Option<i32>,
    /// 是否真的比对过字符集。
    charset_checked: bool,
    /// 会话当前模式名；取不到（查询降级）时是 `None`。
    current_schema: Option<String>,
}

/// 跑会话自检并按策略处置。
///
/// 顺带带出「当前模式」，供 [`apply_schema`] 决定要不要发 `SET SCHEMA`——原来那条
/// `select unicode()` 现在多取一列，**默认配置下不增加任何往返**。
fn inspect_session(
    guard: &odbc_api::Connection<'_>,
    cfg: &DmConfig,
) -> Result<SessionFacts, DmError> {
    let nothing = SessionFacts {
        unicode: None,
        charset_checked: false,
        current_schema: None,
    };

    if cfg.charset_check == CharsetCheck::Off && cfg.schema.is_none() {
        tracing::debug!("charset_check=off 且未配 schema：跳过连接期会话自检");
        return Ok(nothing);
    }
    if cfg.charset_check == CharsetCheck::Off {
        // 自检关掉但仍配了模式：只探模式，不判字符集。
        let current_schema = query_current_schema(guard).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "charset_check=off 下探当前模式失败，继续建连");
            None
        });
        return Ok(SessionFacts {
            current_schema,
            ..nothing
        });
    }

    let (actual, current_schema) = match query_session(guard) {
        Ok(Some(pair)) => pair,
        Ok(None) => {
            tracing::warn!("`{CHARSET_PROBE_SQL}` 返回 NULL，跳过字符集自检");
            return Ok(nothing);
        }
        Err(err) => {
            // §8.3：查询本身失败（权限/版本）→ 降级为告警，不阻塞连接。
            tracing::warn!(
                error = %err,
                "连接期字符集自检未能执行（权限或版本限制），已降级为告警"
            );
            return Ok(nothing);
        }
    };
    let Some(actual) = actual else {
        tracing::warn!("`{CHARSET_PROBE_SQL}` 返回 NULL，跳过字符集自检");
        return Ok(SessionFacts {
            current_schema,
            ..nothing
        });
    };

    match judge_charset(actual) {
        CharsetVerdict::Agree => {
            tracing::debug!(
                unicode = actual,
                character_code = ?cfg.character_code,
                "字符集自检通过（服务端字符集在声明支持的集合里）"
            );
            Ok(SessionFacts {
                unicode: Some(actual),
                charset_checked: true,
                current_schema,
            })
        }
        CharsetVerdict::Mismatch { configured, actual } => match cfg.charset_check {
            // `Off` 在函数开头已经返回，这里只会是 `Warn`/`Error`。
            CharsetCheck::Error => Err(DmError::at(
                DmErrorKind::CharsetMismatch { configured, actual },
                Stage::Connect,
            )),
            _ => {
                tracing::warn!(
                    configured,
                    actual,
                    "服务端字符集不在本项目验证过的集合里（charset_check=warn，继续建连）"
                );
                Ok(SessionFacts {
                    unicode: Some(actual),
                    charset_checked: true,
                    current_schema,
                })
            }
        },
    }
}

/// 取 `select unicode()`。`Ok(None)` = NULL。
fn query_unicode(guard: &odbc_api::Connection<'_>) -> Result<Option<i32>, DmError> {
    let mut cursor = guard
        .execute(CHARSET_PROBE_SQL, (), None)
        .map_err(|err| {
            diag::from_odbc_error(err, Stage::Connect, "SQLExecDirectW(select unicode())")
        })?
        .ok_or_else(|| {
            DmError::at(
                DmErrorKind::Server {
                    sqlstate: *b"HY000",
                    code: 0,
                    message: format!("`{CHARSET_PROBE_SQL}` 没有产生结果集"),
                },
                Stage::Connect,
            )
        })?;

    let mut buf: Vec<u16> = Vec::with_capacity(CHARSET_PROBE_BUF);
    let Some(mut row) = cursor
        .next_row()
        .map_err(|err| diag::from_odbc_error(err, Stage::Fetch, "SQLFetch(select unicode())"))?
    else {
        return Ok(None);
    };
    // 宽字符路径（§4.3）：探针值虽是 ASCII，也绝不走可能被 `narrow` 影响的窄口。
    let not_null = row
        .get_wide_text(1, &mut buf)
        .map_err(|err| diag::from_odbc_error(err, Stage::Fetch, "SQLGetData(select unicode())"))?;
    if !not_null {
        return Ok(None);
    }

    let text = slice_to_cow_utf8(&buf);
    let trimmed = text.trim();
    trimmed.parse::<i32>().map(Some).map_err(|_| {
        DmError::at(
            DmErrorKind::Server {
                sqlstate: *b"HY000",
                code: 0,
                message: format!("`{CHARSET_PROBE_SQL}` 返回了非整数：{trimmed:?}"),
            },
            Stage::Connect,
        )
    })
}

/// 会话自检的两个字段：`(服务端字符集编号, 会话当前模式名)`。
type SessionPair = (Option<i32>, Option<String>);

/// 一次取回「字符集编号 + 当前模式」。
///
/// 合查询失败（老版本不认 `CURRENT_SCHID` 之类）就退回单列 `select unicode()`：
/// 字符集自检不能因为顺带探模式而整条丢掉。
fn query_session(guard: &odbc_api::Connection<'_>) -> Result<Option<SessionPair>, DmError> {
    match query_text_columns(guard, SESSION_PROBE_SQL, 2) {
        Ok(Some(mut cells)) => {
            let current_schema = cells.pop().flatten();
            let Some(text) = cells.pop().flatten() else {
                return Ok(Some((None, current_schema)));
            };
            let trimmed = text.trim();
            let unicode = trimmed.parse::<i32>().map_err(|_| {
                DmError::at(
                    DmErrorKind::Server {
                        sqlstate: *b"HY000",
                        code: 0,
                        message: format!("`{SESSION_PROBE_SQL}` 返回了非整数：{trimmed:?}"),
                    },
                    Stage::Connect,
                )
            })?;
            Ok(Some((Some(unicode), current_schema)))
        }
        Ok(None) => Ok(None),
        Err(err) => {
            tracing::debug!(
                error = %err,
                "会话自检合查询失败，退回 `select unicode()` 单列查询"
            );
            Ok(query_unicode(guard)?.map(|unicode| (Some(unicode), None)))
        }
    }
}

/// 只取当前模式名。取不到时 `Ok(None)`。
fn query_current_schema(guard: &odbc_api::Connection<'_>) -> Result<Option<String>, DmError> {
    Ok(query_text_columns(guard, SCHEMA_PROBE_SQL, 1)?.and_then(|mut cells| cells.pop().flatten()))
}

/// 取首行前 `cols` 列的宽文本（`None` = 该列为 NULL）。无结果集 / 无行 → `Ok(None)`。
///
/// 一律走宽字符路径（§4.3）：模式名可能含中文，绝不能走会被 `narrow` 影响的窄口。
fn query_text_columns(
    guard: &odbc_api::Connection<'_>,
    sql: &str,
    cols: usize,
) -> Result<Option<Vec<Option<String>>>, DmError> {
    let mut cursor = guard
        .execute(sql, (), None)
        .map_err(|err| diag::from_odbc_error(err, Stage::Connect, "SQLExecDirectW(会话自检)"))?
        .ok_or_else(|| {
            DmError::at(
                DmErrorKind::Server {
                    sqlstate: *b"HY000",
                    code: 0,
                    message: format!("`{sql}` 没有产生结果集"),
                },
                Stage::Connect,
            )
        })?;

    let Some(mut row) = cursor
        .next_row()
        .map_err(|err| diag::from_odbc_error(err, Stage::Fetch, "SQLFetch(会话自检)"))?
    else {
        return Ok(None);
    };

    let mut buf: Vec<u16> = Vec::with_capacity(CHARSET_PROBE_BUF);
    let mut cells = Vec::with_capacity(cols);
    for index in 1..=cols {
        buf.clear();
        let not_null = row
            .get_wide_text(index as u16, &mut buf)
            .map_err(|err| diag::from_odbc_error(err, Stage::Fetch, "SQLGetData(会话自检)"))?;
        cells.push(if not_null {
            Some(slice_to_cow_utf8(&buf).into_owned())
        } else {
            None
        });
    }
    Ok(Some(cells))
}

// ---------------------------------------------------------------------------
// ⑥′ 默认模式（`DmConfig::schema`）
// ---------------------------------------------------------------------------

/// 让会话默认模式变成 `cfg.schema`。
///
/// **为什么必须发 SQL**：达梦 ODBC 驱动忽略连接串里的 `SCHEMA=` / `SCHEMA_NAME=` /
/// `CURRENT_SCHEMA=`（`examples/schema_probe.rs` 实测五组写法全部无效，连上后仍是登录用户
/// 默认模式），唯一有效的写法是 `SET SCHEMA`。`ALTER SESSION SET CURRENT_SCHEMA = x`
/// 也行，但前者更短且被所有版本支持。
///
/// **成本**：`current` 来自同一次会话自检，已经就位时**一条 SQL 都不发**；只有首次
/// 建连（或池里换了模式）才多一条 `SET SCHEMA`。
fn apply_schema(
    guard: &odbc_api::Connection<'_>,
    cfg: &DmConfig,
    current: Option<&str>,
) -> Result<(), DmError> {
    apply_schema_with(cfg.schema.as_deref(), current, |sql| {
        guard
            .execute(sql, (), None)
            .map(|_| ())
            .map_err(|err| diag::from_odbc_error(err, Stage::Connect, "SQLExecDirectW(SET SCHEMA)"))
    })
}

/// [`apply_schema`] 的判定核心：发不发、发什么，`exec` 抽出来是为了能单测。
fn apply_schema_with(
    wanted: Option<&str>,
    current: Option<&str>,
    exec: impl FnOnce(&str) -> Result<(), DmError>,
) -> Result<(), DmError> {
    let Some(wanted) = wanted else {
        return Ok(());
    };
    if wanted.trim().is_empty() {
        // `DmConfig::validate` 已经拦了，这里只是不让空串漏到 SQL 里。
        return Err(DmError::at(
            DmErrorKind::Config("schema 为空串，无法 SET SCHEMA".into()),
            Stage::Config,
        ));
    }
    // 逐字节比较，**不做大小写归一**：达梦模式名大小写敏感，小写模式必须带引号，
    // 未加引号会被折成大写然后 -2103 无效模式名（实测）。
    if current == Some(wanted) {
        tracing::debug!(schema = wanted, "连接默认模式已是目标模式，跳过 SET SCHEMA");
        return Ok(());
    }

    let sql = set_schema_sql(wanted);
    exec(&sql)?;
    tracing::debug!(schema = wanted, "已用 SET SCHEMA 切换连接默认模式");
    Ok(())
}

/// 生成 `SET SCHEMA "<name>"`。
///
/// 一律加双引号：达梦把未加引号的标识符折成大写，小写模式名不加引号会直接 -2103；
/// 加引号对 `DBTESTA` 这种大写模式名同样有效。名字里的 `"` 按 SQL 标准双写。
fn set_schema_sql(schema: &str) -> String {
    format!("SET SCHEMA \"{}\"", schema.replace('"', "\"\""))
}

// ---------------------------------------------------------------------------
// ⑦ init_sql
// ---------------------------------------------------------------------------

/// 逐条执行 `init_sql`。
///
/// 这里**不支持参数**：`init_sql` 是建连初始化（`SET SCHEMA` 之类），带 `?` 会直接失败。
/// 语句若产生结果集，游标交给 `CursorImpl::drop` 关掉——「同一语句连续两次执行报
/// `-70027` 无效的游标状态」就是没关游标引起的。
fn run_init_sql(guard: &odbc_api::Connection<'_>, cfg: &DmConfig) -> Result<(), DmError> {
    for sql in &cfg.init_sql {
        if sql.trim().is_empty() {
            continue;
        }
        tracing::debug!(sql, "执行连接初始化 SQL");
        let cursor = guard.execute(sql.as_str(), (), None).map_err(|err| {
            diag::from_odbc_error(err, Stage::Execute, "SQLExecDirectW(init_sql)")
        })?;
        drop(cursor);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ④ 失败映射
// ---------------------------------------------------------------------------

/// 建连失败 → [`DmError`]。
///
/// 「驱动没装」这一种必须换成 [`DmErrorKind::DriverNotFound`]（§6.2）：它带 `tried` 与
/// `hint`，而驱动管理器给的 `IM002` 只有一句「未发现数据源名称」。判定只看 `SQLSTATE`
/// （那时没有原生码）。
fn connect_failed(err: odbc_api::Error) -> DmError {
    let dm = diag::from_odbc_error(err, Stage::Connect, "SQLDriverConnectW");
    if diag::is_driver_not_found(dm.diags()) {
        let registry = probe_cached();
        let tried: Vec<String> = if registry.tried().is_empty() {
            registry.names().into_iter().map(str::to_owned).collect()
        } else {
            registry.tried().to_vec()
        };
        return driver_not_found(tried, dm.diags().to_vec());
    }
    dm
}

/// 造一个带 `tried`/`hint` 的 `DriverNotFound`（§6.2 要求的可操作提示）。
fn driver_not_found(tried: Vec<String>, diags: Vec<dm_error::DiagRecord>) -> DmError {
    DmError::with_diags(
        DmErrorKind::DriverNotFound {
            tried,
            hint: DRIVER_NOT_FOUND_HINT.to_owned(),
        },
        diags,
        ErrorContext::new(Stage::Connect),
    )
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/// `SqlResult<T>` → `Result<T, DmError>`：本 crate 的统一口径在 [`diag::sql_ok`]。
///
/// 这里保留一层同名包装，只是为了让本文件里的调用点读起来短一点；
/// 「`SQL_SUCCESS_WITH_INFO` 算成功、其余四种返回码算异常」这条判断**只有一份实现**，
/// 免得连接路径与语句路径对同一个返回码给出两种解释。
fn sql_ok<T>(
    result: SqlResult<T>,
    handle: &impl odbc_api::handles::Diagnostics,
    stage: Stage,
    function: &'static str,
) -> Result<T, DmError> {
    diag::sql_ok(result, handle, stage, function)
}

/// `Duration` → ODBC 的整秒。
///
/// **向上取整且最小 1**：`500ms` 若取整成 `0`，在 ODBC 里表示「永不超时」——
/// 那是把「超时要短」变成「超时没有」的方向性错误。
fn secs_of(d: Duration) -> u32 {
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    u32::try_from(secs.max(1)).unwrap_or(u32::MAX)
}

fn micros_of(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_durations_up_to_whole_seconds() {
        assert_eq!(secs_of(Duration::from_secs(30)), 30);
        assert_eq!(
            secs_of(Duration::from_millis(500)),
            1,
            "0 秒在 ODBC 里是「永不超时」"
        );
        assert_eq!(secs_of(Duration::ZERO), 1);
        assert_eq!(
            secs_of(Duration::from_millis(1500)),
            2,
            "向上取整，宁可早点超时"
        );
        assert_eq!(secs_of(Duration::from_secs(u64::from(u32::MAX))), u32::MAX);
        assert_eq!(secs_of(Duration::from_secs(u64::MAX)), u32::MAX);
    }

    /// 达梦把未加引号的标识符折成大写，小写模式名不加引号会直接 -2103；
    /// 所以生成的 `SET SCHEMA` 一律带双引号，名字里的 `"` 按 SQL 标准双写。
    #[test]
    fn set_schema_statement_always_quotes() {
        assert_eq!(set_schema_sql("DBTESTA"), "SET SCHEMA \"DBTESTA\"");
        assert_eq!(
            set_schema_sql("dbtest_lower"),
            "SET SCHEMA \"dbtest_lower\""
        );
        assert_eq!(
            set_schema_sql("odd\"name"),
            "SET SCHEMA \"odd\"\"name\"",
            "内部双引号必须双写，否则可以注入闭合引号"
        );
        assert_eq!(
            set_schema_sql("模式 中文"),
            "SET SCHEMA \"模式 中文\"",
            "模式名可以是中文/含空格"
        );
    }

    /// 模式已经就位时一条 SQL 都不发；没配模式时同样什么都不做。
    #[test]
    fn apply_schema_sends_only_when_needed() {
        // ① 没配 schema：连 exec 都不该被调用。
        let executed = std::cell::Cell::new(0usize);
        apply_schema_with(None, None, |_| {
            executed.set(executed.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(executed.get(), 0, "没配 schema 不该发 SQL");

        // ② 已经是目标模式：同样不发（这就是「不多付一次往返」的那条路径）。
        apply_schema_with(Some("DBTESTA"), Some("DBTESTA"), |_| {
            executed.set(executed.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(executed.get(), 0, "模式已就位不该再发 SET SCHEMA");

        // ③ 不一致：发且只发一条，内容带引号。
        let mut sent = String::new();
        apply_schema_with(Some("DBTESTB"), Some("DBTESTA"), |sql| {
            sent = sql.to_owned();
            Ok(())
        })
        .unwrap();
        assert_eq!(sent, "SET SCHEMA \"DBTESTB\"");

        // ④ 大小写不同就是不同模式（达梦模式名大小写敏感）。
        let mut sent = String::new();
        apply_schema_with(Some("dbtest_lower"), Some("DBTEST_LOWER"), |sql| {
            sent = sql.to_owned();
            Ok(())
        })
        .unwrap();
        assert_eq!(sent, "SET SCHEMA \"dbtest_lower\"");
    }

    #[test]
    fn accepts_only_declared_server_charsets() {
        assert_eq!(judge_charset(UNICODE_GB18030), CharsetVerdict::Agree);
        // 未实测过的编号一律算「不一致」——不是「跳过」，因为跳过等于放行未验证的假设。
        assert_eq!(
            judge_charset(1),
            CharsetVerdict::Mismatch {
                configured: UNICODE_GB18030,
                actual: 1,
            }
        );
        assert_eq!(
            judge_charset(-1),
            CharsetVerdict::Mismatch {
                configured: UNICODE_GB18030,
                actual: -1,
            }
        );
    }

    #[test]
    fn client_character_code_never_triggers_a_mismatch() {
        // 真机证伪后的回归守卫：`CHARACTER_CODE=PG_UTF8` 时服务端 `unicode()` 仍是 0，
        // 而那条连接工作正常。判定只看服务端字符集，与 `character_code` 无关，
        // 所以这里不可能因为客户端关键字而变化。
        assert_eq!(judge_charset(UNICODE_GB18030), CharsetVerdict::Agree);
        assert_eq!(SUPPORTED_SERVER_UNICODE, &[UNICODE_GB18030]);
    }

    #[test]
    fn driver_not_found_carries_tried_and_hint() {
        let err = driver_not_found(vec!["DM8 ODBC DRIVER".to_owned()], Vec::new());
        match &err.kind {
            DmErrorKind::DriverNotFound { tried, hint } => {
                assert_eq!(tried, &["DM8 ODBC DRIVER".to_owned()]);
                assert!(hint.contains("ODBC"), "提示要能指路：{hint}");
            }
            other => panic!("应当是 DriverNotFound，却得到 {other:?}"),
        }
        assert_eq!(err.context.stage, Stage::Connect);
    }

    #[test]
    fn non_driver_failures_keep_their_own_kind() {
        // 驱动一条诊断都没有时不能伪装成「没装驱动」。
        let err = connect_failed(odbc_api::Error::NoDiagnostics {
            function: "SQLDriverConnectW",
        });
        assert!(
            !matches!(err.kind, DmErrorKind::DriverNotFound { .. }),
            "无诊断的建连失败被错判成 DriverNotFound：{:?}",
            err.kind
        );
        assert_eq!(err.context.stage, Stage::Connect);
        assert_eq!(err.diags().len(), 1, "建连失败必然带诊断");
    }

    #[test]
    fn charset_mismatch_is_connect_stage() {
        let err = DmError::at(
            DmErrorKind::CharsetMismatch {
                configured: UNICODE_GB18030,
                actual: 7,
            },
            Stage::Connect,
        );
        assert_eq!(err.context.stage, Stage::Connect);
        let text = err.to_string();
        assert!(
            text.contains('7') && text.contains('0'),
            "错误消息要把基线与实测值都报出来：{text}"
        );
    }
}
