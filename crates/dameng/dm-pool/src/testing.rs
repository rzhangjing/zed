//! 离线假后端：把池的状态机单独跑起来，完全不碰 ODBC（设计方案 §10.1 的 L4 层）。
//!
//! 这个文件同时被两处使用：
//!
//! - `tests/common/mod.rs` 用 `#[path = "../../src/testing.rs"]` 把它原样编进集成测试
//!   （集成测试是独立 crate，看不见 `#[cfg(test)]` 的模块），所以它**只依赖
//!   `dm-core`/`dm-error` 的公开 API**，不引用 dm-pool 自身的任何类型；
//! - 将来 `src/` 内的单测若要用，`lib.rs` 里加 `#[cfg(test)] mod testing;` 即可。
//!
//! 假后端能做的「坏行为」都是池必须正确应对的：建连失败/变慢、探活判死、
//! 探活报错、复位失败。它同时记录操作日志，用来验证复位的四步顺序。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dm_core::{Backend, ColumnMeta, DmConfig, DmValue, ExecOutcome, FetchState, RowBuf};
use dm_error::{DmError, DmErrorKind, Stage};

/// 假后端的语句类型。池不关心语句内部，只要求 `Send + 'static`。
///
/// 它持有 `Arc<FakeShared>` 是为了在 `Drop` 里留一行日志——**CT-09** 要断言
/// 「语句句柄一定先于它所属的连接被释放」。真机上顺序反了会看到
/// `SQLFreeHandle failed with error code: -2` 甚至 `0xc0000005`
/// （证据见 `crates/dm-odbc/examples/cursor_reuse_probe.rs`）。
#[derive(Debug, Default, Clone)]
pub struct FakeStmt {
    pub sql: String,
    /// 语句编号（Drop 日志里的标识）。
    pub id: u64,
    /// 它属于哪条物理连接（Drop 日志带上，便于断言顺序）。
    pub conn: u64,
    shared: Option<Arc<FakeShared>>,
}

impl Drop for FakeStmt {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.as_ref() {
            shared.stmt_drops.fetch_add(1, Ordering::Relaxed);
            shared.log(format!("drop_stmt#{}@conn{}", self.id, self.conn));
        }
    }
}

/// 所有假连接共享的观测点。
#[derive(Debug)]
pub struct FakeShared {
    /// 建连尝试次数（含失败）。
    pub connects: AtomicUsize,
    /// 还要失败几次建连（用来测失败路径，且失败必须归还名额）。
    pub connect_failures: AtomicUsize,
    /// 建连延迟（毫秒）：用来制造「正在建连」的窗口。
    pub connect_delay_ms: AtomicU64,
    /// 探活调用次数。
    pub pings: AtomicUsize,
    /// 还要报几次探活错误（不是「已死」，是驱动不支持之类的普通错误）。
    pub ping_errors: AtomicUsize,
    /// 探活结论：`false` 表示连接已死。
    pub ping_alive: AtomicBool,
    /// 复位次数（成功与失败都算）。
    pub resets: AtomicUsize,
    /// 还要失败几次复位。
    pub reset_failures: AtomicUsize,
    /// 还要在复位里 panic 几次（驱动在同步代码里 panic 是有先例的，
    /// 池必须接住它而不是让回收闸门卡死）。
    pub panic_next_resets: AtomicUsize,
    /// 被显式 `Backend::close` 的次数。
    pub closed: AtomicUsize,
    /// `Backend::prepare` 的调用次数（语句缓存的判据：同一条 SQL 第二次不该再
    /// 走到这里）。
    pub prepares: AtomicUsize,
    /// 语句句柄被释放的次数（`Drop for FakeStmt`；CT-09 的判据之一）。
    pub stmt_drops: AtomicUsize,
    /// 自增语句编号。
    pub next_stmt: AtomicU64,
    /// 还要在 `close` 里 panic 几次（同上：池必须接住，
    /// 否则名额与存活计数会被一起跳过）。
    pub panic_next_closes: AtomicUsize,
    /// 当前存活的物理连接数（connect 加、close 减）。
    pub live: AtomicUsize,
    /// 历史峰值（G4 判据「峰值 `in_use ≤ max_size`」用它）。
    pub peak_live: AtomicUsize,
    /// 自增连接编号。
    pub next_id: AtomicU64,
    /// 操作日志，形如 `connect#1` / `reset#1` / `close#1`。
    pub ops: Mutex<Vec<String>>,
}

impl Default for FakeShared {
    fn default() -> Self {
        Self {
            connects: AtomicUsize::new(0),
            connect_failures: AtomicUsize::new(0),
            connect_delay_ms: AtomicU64::new(0),
            pings: AtomicUsize::new(0),
            ping_errors: AtomicUsize::new(0),
            ping_alive: AtomicBool::new(true),
            resets: AtomicUsize::new(0),
            reset_failures: AtomicUsize::new(0),
            panic_next_resets: AtomicUsize::new(0),
            closed: AtomicUsize::new(0),
            prepares: AtomicUsize::new(0),
            stmt_drops: AtomicUsize::new(0),
            next_stmt: AtomicU64::new(1),
            panic_next_closes: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            peak_live: AtomicUsize::new(0),
            next_id: AtomicU64::new(1),
            ops: Mutex::new(Vec::new()),
        }
    }
}

impl FakeShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 接下来 `n` 次建连都失败。
    pub fn fail_next_connects(&self, n: usize) {
        self.connect_failures.store(n, Ordering::Relaxed);
    }

    /// 接下来 `n` 次复位都失败。
    pub fn fail_next_resets(&self, n: usize) {
        self.reset_failures.store(n, Ordering::Relaxed);
    }

    /// 接下来 `n` 次复位都 panic（模拟驱动在同步代码里炸掉）。
    pub fn panic_next_resets(&self, n: usize) {
        self.panic_next_resets.store(n, Ordering::Relaxed);
    }

    /// 接下来 `n` 次 `close` 都 panic。
    pub fn panic_next_closes(&self, n: usize) {
        self.panic_next_closes.store(n, Ordering::Relaxed);
    }

    /// 让建连慢下来（制造「有连接正在建」的窗口）。
    pub fn set_connect_delay_ms(&self, ms: u64) {
        self.connect_delay_ms.store(ms, Ordering::Relaxed);
    }

    /// 接下来 `n` 次探活返回普通错误（不是「已死」）。
    pub fn fail_next_pings(&self, n: usize) {
        self.ping_errors.store(n, Ordering::Relaxed);
    }

    pub fn set_ping_alive(&self, alive: bool) {
        self.ping_alive.store(alive, Ordering::Relaxed);
    }

    pub fn live(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }

    pub fn peak_live(&self) -> usize {
        self.peak_live.load(Ordering::Relaxed)
    }

    /// `prepare` 的累计调用次数（语句缓存的判据）。
    pub fn prepares(&self) -> usize {
        self.prepares.load(Ordering::Relaxed)
    }

    /// 语句句柄被 drop 的累计次数（CT-09 的判据）。
    pub fn stmt_drops(&self) -> usize {
        self.stmt_drops.load(Ordering::Relaxed)
    }

    pub fn op_log(&self) -> Vec<String> {
        self.ops.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn log(&self, entry: impl Into<String>) {
        self.ops
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry.into());
    }
}

/// 一条假连接。除了编号，所有状态都在共享的 [`FakeShared`] 里。
#[derive(Debug)]
pub struct FakeBackend {
    shared: Arc<FakeShared>,
    id: u64,
}

impl Backend for FakeBackend {
    type Stmt = FakeStmt;

    fn name(&self) -> &'static str {
        "fake"
    }

    fn connect(cfg: &DmConfig) -> Result<Self, DmError>
    where
        Self: Sized,
    {
        // 假后端的旋钮无法塞进 `DmConfig` 的类型化字段里，于是约定：
        // 配置的 `extra[FAKE_TOKEN_KEY]` 里放一个登记号，`registry_of` 用它
        // 换回本次用例的共享状态。
        //
        // **不能用线程局部**：`Backend::connect` 是在池的**阻塞线程**上跑的，
        // 调用方线程绑定的东西在那边看不见。
        let shared = shared_for_config(cfg)?;
        let delay = shared.connect_delay_ms.load(Ordering::Relaxed);
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        shared.connects.fetch_add(1, Ordering::Relaxed);
        if shared
            .connect_failures
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(DmError::at(
                DmErrorKind::Connect {
                    source: "假后端：按配置建连失败".to_owned(),
                },
                Stage::Connect,
            ));
        }
        let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
        let live = shared.live.fetch_add(1, Ordering::Relaxed) + 1;
        shared.peak_live.fetch_max(live, Ordering::Relaxed);
        shared.log(format!("connect#{id}"));
        Ok(Self { shared, id })
    }

    fn ping(&mut self) -> Result<bool, DmError> {
        self.shared.pings.fetch_add(1, Ordering::Relaxed);
        if self
            .shared
            .ping_errors
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(DmError::at(
                DmErrorKind::Unsupported {
                    feature: "假后端：按配置让探活报错".to_owned(),
                },
                Stage::Other,
            ));
        }
        Ok(self.shared.ping_alive.load(Ordering::Relaxed))
    }

    fn prepare(&mut self, sql: &str) -> Result<Self::Stmt, DmError> {
        self.shared.prepares.fetch_add(1, Ordering::Relaxed);
        let id = self.shared.next_stmt.fetch_add(1, Ordering::Relaxed);
        Ok(FakeStmt {
            sql: sql.to_owned(),
            id,
            conn: self.id,
            shared: Some(Arc::clone(&self.shared)),
        })
    }

    fn bind_params(&mut self, _stmt: &mut Self::Stmt, _args: &[DmValue]) -> Result<(), DmError> {
        Ok(())
    }

    fn execute(&mut self, _stmt: &mut Self::Stmt, _sql: &str) -> Result<ExecOutcome, DmError> {
        Ok(ExecOutcome::Affected { rows: Some(1) })
    }

    /// 假后端照抄 `dm-odbc` 的判据：会产出结果集的语句（`select`）取尽之后句柄就死了，
    /// 不能再执行（真机证据见 `crates/dm-odbc/examples/cursor_reuse_probe.rs`）。
    /// 这样 `crates/dm-pool/tests/stmt_cache.rs` 能同时钉住「DML 复用」与「查询不复用」。
    fn statement_reusable(&self, stmt: &Self::Stmt) -> bool {
        !stmt
            .sql
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("select")
    }

    fn columns(&mut self, _stmt: &Self::Stmt) -> Result<Arc<[ColumnMeta]>, DmError> {
        Ok(Arc::from(Vec::<ColumnMeta>::new()))
    }

    fn fetch(&mut self, _stmt: &mut Self::Stmt, _into: &mut RowBuf) -> Result<FetchState, DmError> {
        Ok(FetchState::Exhausted)
    }

    fn close_cursor(&mut self, _stmt: &mut Self::Stmt) -> Result<(), DmError> {
        Ok(())
    }

    fn begin(&mut self) -> Result<(), DmError> {
        self.shared.log(format!("begin#{}", self.id));
        Ok(())
    }

    fn commit(&mut self) -> Result<(), DmError> {
        self.shared.log(format!("commit#{}", self.id));
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), DmError> {
        self.shared.log(format!("rollback#{}", self.id));
        Ok(())
    }

    fn reset(&mut self) -> Result<(), DmError> {
        // 顺序：先关游标，再回滚，最后交给上层（`dm-odbc` 的实现里还有
        // 「还原 autocommit」与「清语句缓存」两步）。假后端把顺序记进日志，
        // 让「复位真的按 §5.7 的四步走」这件事可断言。
        self.shared.log(format!("close_cursor#{0}", self.id));
        self.shared.log(format!("rollback#{0}", self.id));
        self.shared.resets.fetch_add(1, Ordering::Relaxed);
        if self
            .shared
            .panic_next_resets
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            // 驱动在同步代码里 panic 是有先例的（`odbc-api` 的 `Drop`：
            // `SQLFreeHandle` 失败就 panic），池必须接得住。
            panic!("假后端：按配置在复位时 panic");
        }
        if self
            .shared
            .reset_failures
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(DmError::at(DmErrorKind::Io, Stage::Reset));
        }
        self.shared.log(format!("reset#{0}", self.id));
        Ok(())
    }

    fn close(self)
    where
        Self: Sized,
    {
        self.shared.live.fetch_sub(1, Ordering::Relaxed);
        self.shared.closed.fetch_add(1, Ordering::Relaxed);
        self.shared.log(format!("close#{}", self.id));
        if self
            .shared
            .panic_next_closes
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            // 同上：句柄释放之后 panic。池的账（名额 + 存活计数）必须照样结清。
            panic!("假后端：按配置在关闭时 panic");
        }
    }
}

/// 连接对象本身的析构（`Backend::close` 之外的路径，例如池被直接 drop）。
///
/// 留日志是为了让 CT-09 能断言「语句先于连接」：真机上把语句句柄留到
/// 连接/env 之后才释放，`SQLFreeHandle` 会失败（`-2`）甚至直接崩进程。
impl Drop for FakeBackend {
    fn drop(&mut self) {
        self.shared.log(format!("drop_conn#{}", self.id));
    }
}

// ---------------------------------------------------------------------------
// 共享状态怎么传给 `Backend::connect`
// ---------------------------------------------------------------------------
//
// `Backend::connect` 只拿得到 `&DmConfig`，而 `DmConfig` 里没有「建连延时」
// 这种假后端专用的旋钮；`connect` 又是在池的阻塞线程上跑的，线程局部变量
// 传不过去。于是用一个**进程级登记表**：每个用例先 `fake_config(shared)`
// 拿到一份 `DmConfig`，它的 `extra[FAKE_TOKEN_KEY]` 里是一个自增登记号；
// `connect` 用这个号换回共享状态。登记号让并行跑用例互不干扰。

use std::collections::HashMap;
use std::sync::OnceLock;

/// `DmConfig::extra` 里放假后端登记号的键名。
pub const FAKE_TOKEN_KEY: &str = "fake.token";

fn registry() -> &'static Mutex<HashMap<u64, Arc<FakeShared>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u64, Arc<FakeShared>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 登记一份共享状态，返回它的号。
pub fn register(shared: Arc<FakeShared>) -> u64 {
    static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(token, shared);
    token
}

/// 按号取回共享状态（测试想拿到「池内部那条连接」的观测点时用）。
pub fn shared_of(token: u64) -> Option<Arc<FakeShared>> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&token)
        .cloned()
}

/// 从配置里取出假后端共享状态。
pub fn shared_for_config(cfg: &DmConfig) -> Result<Arc<FakeShared>, DmError> {
    let token = cfg
        .extra
        .get(FAKE_TOKEN_KEY)
        .and_then(|raw| raw.parse::<u64>().ok())
        .ok_or_else(|| {
            DmError::at(
                DmErrorKind::Config(format!(
                    "假后端配置缺少登记号：请在 DmConfig::extra 里设置 {FAKE_TOKEN_KEY}"
                )),
                Stage::Config,
            )
        })?;
    shared_of(token).ok_or_else(|| {
        DmError::at(
            DmErrorKind::Config(format!("假后端登记号 {token} 不存在")),
            Stage::Config,
        )
    })
}

/// 造一份能交给 `DmPool::connect_with` 的假后端配置。
///
/// 连接串里的服务器/驱动全是占位值：`connect` 由假后端实现，不会真的去连。
/// 数据库名固定 `FAKE`——池把它当作池身份的一部分（不同库不能复用连接）。
pub fn fake_config(shared: &Arc<FakeShared>) -> DmConfig {
    let token = register(Arc::clone(shared));
    let mut cfg = DmConfig::from_connection_string(
        "DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=1;UID=FAKE;PWD=FAKE;DBNAME=FAKE",
    )
    .expect("假连接串必须能解析");
    cfg.extra
        .insert(FAKE_TOKEN_KEY.to_owned(), token.to_string());
    cfg
}

/// 复位的四步日志判定：`close_cursor` 必须出现在 `rollback` 之前。
pub fn reset_steps_are_ordered(log: &[String]) -> bool {
    let mut saw_close_cursor = false;
    for entry in log {
        if entry.starts_with("close_cursor#") {
            saw_close_cursor = true;
        }
        if entry.starts_with("rollback#") && !saw_close_cursor {
            return false;
        }
    }
    saw_close_cursor
}

/// 日志里出现过的物理连接编号（从 `connect#N` / `close#N` / `drop_conn#N` 里取）。
pub fn conn_ids(log: &[String]) -> Vec<u64> {
    let mut ids = Vec::new();
    for entry in log {
        for prefix in ["connect#", "close#", "drop_conn#"] {
            if let Some(rest) = entry.strip_prefix(prefix)
                && let Ok(id) = rest.parse::<u64>()
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    ids
}

/// **CT-09 的离线判据**：连接 `conn` 拥有的每条语句，其释放日志
/// （`drop_stmt#N@conn{conn}`）都必须排在该连接的 `conn_marker`（`reset#` /
/// `close#` / `drop_conn#`）**之前**，且至少真的释放过一条。
///
/// 真机上的对应现象：语句晚于连接释放会看到 `SQLFreeHandle failed with error
/// code: -2` / `0xc0000005`（`crates/dm-odbc/examples/cursor_reuse_probe.rs`）。
pub fn statements_are_dropped_before(log: &[String], conn: u64, conn_marker: &str) -> bool {
    let boundary_marker = format!("{conn_marker}{conn}");
    let Some(boundary) = log.iter().position(|entry| *entry == boundary_marker) else {
        return false;
    };
    let needle = format!("@conn{conn}");
    let mut saw_drop = false;
    for (index, entry) in log.iter().enumerate() {
        if entry.starts_with("drop_stmt#") && entry.contains(&needle) {
            saw_drop = true;
            if index > boundary {
                return false;
            }
        }
    }
    saw_drop
}
