//! 单条物理连接的共享核心与借出句柄（设计方案 §5.4、§5.6、§5.7）。
//!
//! ## 为什么 `ConnInner` 要被 `Arc` 共享
//!
//! 一次操作会**跨线程**：`async` 侧发起，阻塞线程执行。如果裸连接由 `async` 侧持有，
//! 那么 future 被 drop（超时 / `select!` / 上层取消）时它就会随 future 一起消失，
//! 或者更糟——一个状态未知的连接被当成干净的还回池里（§5.4 开头）。
//!
//! 所以裸连接放在 [`ConnInner::slot`] 里，两侧都只持有 `Arc<ConnInner>`：
//! - 借出时 `async` 侧把裸连接**取走**交给阻塞闭包（此刻 `state = InFlight`）；
//! - 阻塞闭包结束时**先定终态、再把裸连接放回 slot**（`Idle` / `Dirty` / `Poisoned`）。
//!
//! 于是「谁负责把连接还给池」由一对原子旗标决定，见
//! [`ConnInner::mark_guard_gone`] / [`ConnInner::guard_is_gone`] / [`ConnInner::claim_publish`]：
//! 句柄侧「先宣告已走，再看裸连接在不在槽里」，阻塞侧「先把裸连接放回槽里，
//! 再看句柄走没走」——两个方向的读写顺序互为反向，**不可能两边都以为自己不管**；
//! 归还责任本身用 `CAS(false→true)` 认领，**恰有一方成功**，所以既不丢也不重复归还。
//!
//! ⚠️ 这个握手**不能拿 `state` 当信号**：取消/毒化路径会把 `state` 改写成
//! `Poisoned`，于是「句柄已走但状态不是 `InFlight`」的连接会两头都没人归还
//! （T4.7 的超时用例真机踩到过，连接再也回不了池、`live` 永远不归零）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use dm_core::{Backend, DmConfig};
use dm_error::{DmError, DmErrorKind, Stage};
use tokio::sync::OwnedSemaphorePermit;

use crate::inner::{Liveness, PoolInner};
use crate::stmt_cache::{StmtCache, StmtCacheStats, StmtEntry};

/// 连接在池内的状态（§5.6 状态机）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ConnState {
    /// 空闲，可由 `acquire` 直接借出。
    Idle = 0,
    /// 已借出，阻塞闭包正在用它。
    InFlight = 1,
    /// 用完了但可能留了残留（游标/未提交事务/失败）：必须 `reset()` 之后才能复用。
    Dirty = 3,
    /// 状态未知或已不可信：**必须物理关闭**，绝不复用（**D11**）。
    Poisoned = 4,
}

impl ConnState {
    pub(crate) fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::InFlight,
            3 => Self::Dirty,
            4 => Self::Poisoned,
            _ => Self::Idle,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::InFlight => "InFlight",
            Self::Dirty => "Dirty",
            Self::Poisoned => "Poisoned",
        }
    }
}

/// 单条连接的使用统计（原子累加，不参与同步）。
#[derive(Debug, Default)]
pub(crate) struct ConnStats {
    pub(crate) ops: AtomicU64,
    pub(crate) errors: AtomicU64,
    pub(crate) rows: AtomicU64,
    pub(crate) total_nanos: AtomicU64,
    /// 超过 `DmConfig::slow_query_threshold` 的操作数（§6.9 慢查询告警）。
    ///
    /// 只在这里计数：真正的 `warn!` 由 `dm-query` 在拿到行数后打，
    /// 因为池这一层看不到「本次取了多少行」。
    pub(crate) slow_ops: AtomicU64,
}

/// `ConnStats` 的读快照。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnStatsSnapshot {
    /// 在这个连接上跑过的操作数。
    pub ops: u64,
    /// 其中失败的操作数。
    pub errors: u64,
    /// 累计取到的行数。
    pub rows: u64,
    /// 累计耗时（纳秒）。
    pub total_nanos: u64,
    /// 其中超过慢查询阈值的操作数。
    pub slow_ops: u64,
}

/// 连接「停放位」：裸连接与空位票**互斥存在**。
///
/// - 空闲（在池里）⇒ `conn = Some`、`ticket = None`（空闲连接不占空位票）；
/// - 借出 / 待复位 ⇒ `conn = None`（在阻塞闭包手里，或已放回）、`ticket = Some`。
///
/// 这条「空闲不占票、非空闲占票」的规则是 §5.7 不超发的关键：
/// 如果空闲连接也攥着票，`acquire` 就会在「全员空闲」时把自己锁死。
/// 裸连接 + 空位票 + 时间戳。**每次访问都只做取/放，绝不在锁里做 IO**（§5.8）。
pub(crate) struct ConnSlot<B: Backend> {
    conn: Option<B>,
    ticket: Option<OwnedSemaphorePermit>,
    /// 归还时刻（`idle_timeout` 从这一刻起算）。
    idle_since: Instant,
    /// 上次探活时刻（节流用）。
    last_checked: Instant,
}

/// 一条物理连接的共享核心。
pub(crate) struct ConnInner<B: Backend> {
    id: u64,
    created_at: Instant,
    state: AtomicU64,
    slot: Mutex<ConnSlot<B>>,
    stats: ConnStats,
    /// 存活计数器的共享锚点（见 [`Liveness`]）。用 `Arc` 而不是 `Weak<PoolInner>`：
    /// 连接被「遗弃」时（future 在桥接中途被 drop）也要能把数减对，
    /// 而那时池本身可能已经没了。
    liveness: Arc<Liveness>,
    /// `release_liveness` 的幂等闸门。
    released: std::sync::atomic::AtomicBool,
    /// 句柄是否已经 drop。**归还责任的握手靠它 + [`ConnInner::claim_publish`]**，
    /// 不靠 `state`：`state` 会被取消/毒化路径改写，用它当握手信号会让
    /// 「句柄已走但状态不是 InFlight」的连接两头都没人归还（真机超时用例踩到过）。
    guard_gone: std::sync::atomic::AtomicBool,
    /// 「谁负责把连接还给池」的认领闸门：两侧都 `CAS(false→true)`，**恰有一方成功**。
    returned: std::sync::atomic::AtomicBool,
    /// 这条连接的预处理语句缓存（CAP-C-11）。
    ///
    /// 只在阻塞闭包里短锁持有（锁里不做 IO 之外的等待），与 `slot` 一样
    /// 容忍中毒：一条语句的状态错乱不该让整条连接瘫掉。
    stmt_cache: Mutex<StmtCache<B::Stmt>>,
}

impl<B: Backend> Drop for ConnInner<B> {
    fn drop(&mut self) {
        // 兜底之一：**先清语句缓存**（`Drop::drop` 的执行早于字段析构，所以此刻
        // 槽里的裸连接还在）。语句句柄必须先于父连接释放，否则
        // `SQLFreeHandle(SQL_HANDLE_STMT)` 拿到 `SQL_INVALID_HANDLE`（-2），
        // `odbc-api` 的 `Drop` 会直接 panic（真机 `query_real` 踩到过）。显式关闭
        // 的两条路（`inner.rs::close_physically` / `discard`）已经各自先清一次；
        // 这里覆盖的是「池自己被 drop、没人调 `close()`」那条路。
        //
        // ⚠️ 这一行**不是冗余**：`ConnInner` 的字段声明顺序是 `slot` 在前、
        // `stmt_cache` 在后，而 Rust 按声明顺序析构字段——只靠字段析构会把顺序
        // 正好搞反（先丢连接、再丢语句）。`crates/dm-pool/tests/drop_order.rs`
        // 钉的就是这条（CT-09）。
        self.clear_stmt_cache();
        // 兜底之二：任何绕过显式销毁的路径（遗弃、池被 drop）都在这里把数减掉，
        // 否则 `stats().size` 会永远虚高、`wait_closed()` 会一直等下去。
        self.release_liveness();
    }
}

impl<B: Backend> std::fmt::Debug for ConnInner<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnInner")
            .field("id", &self.id)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl<B: Backend> ConnInner<B> {
    /// `ticket` 用 `Option` 是因为**空闲连接不持有空位票**（见 `inner.rs` 的核算规则）：
    /// 池刚建好时那条连接就停在空闲栈里，没有票。
    pub(crate) fn new(
        id: u64,
        conn: B,
        ticket: Option<OwnedSemaphorePermit>,
        now: Instant,
        liveness: Arc<Liveness>,
        stmt_cache_size: usize,
    ) -> Arc<Self> {
        liveness.add();
        Arc::new(Self {
            id,
            created_at: now,
            state: AtomicU64::new(ConnState::Idle as u64),
            slot: Mutex::new(ConnSlot {
                conn: Some(conn),
                ticket,
                idle_since: now,
                last_checked: now,
            }),
            stats: ConnStats::default(),
            liveness,
            released: std::sync::atomic::AtomicBool::new(false),
            guard_gone: std::sync::atomic::AtomicBool::new(false),
            returned: std::sync::atomic::AtomicBool::new(false),
            stmt_cache: Mutex::new(StmtCache::new(stmt_cache_size)),
        })
    }

    /// 把这条连接从「存活数」里划掉。幂等：显式销毁路径与 `Drop` 都会调。
    pub(crate) fn release_liveness(&self) {
        if self
            .released
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.liveness.gone();
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// 建连时刻——`max_lifetime` 从这一刻起算（§5.7）。
    pub(crate) fn created_at(&self) -> Instant {
        self.created_at
    }

    /// 取 slot 锁，**容忍中毒**：一条连接的状态错乱不该让整个池瘫掉。
    pub(crate) fn lock_slot(&self) -> MutexGuard<'_, ConnSlot<B>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 取语句缓存锁，**同样容忍中毒**。
    pub(crate) fn lock_stmt_cache(&self) -> MutexGuard<'_, StmtCache<B::Stmt>> {
        self.stmt_cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 丢掉全部缓存语句（复位路径调用：见 `inner.rs::recycle_one`）。
    pub(crate) fn clear_stmt_cache(&self) {
        self.lock_stmt_cache().clear();
    }

    /// 语句缓存统计（不含容量为 0 时的历史账——那种情况本来也不记账）。
    pub(crate) fn stmt_cache_stats(&self) -> StmtCacheStats {
        self.lock_stmt_cache().stats()
    }

    pub(crate) fn state(&self) -> ConnState {
        ConnState::from_u8(self.state.load(Ordering::Acquire) as u8)
    }

    pub(crate) fn set_state(&self, state: ConnState) {
        self.state.store(state as u64, Ordering::Release);
    }

    /// 句柄侧宣告「我不再负责归还了」。
    ///
    /// **只宣告，不认领**：能不能归还取决于「裸连接此刻在不在槽里」。
    /// 早先的版本在这里顺手把认领也做了，结果句柄赢了认领却因为
    /// 「裸连接还在阻塞闭包手里」而无法归还，闭包那边又因为「认领已被拿走」
    /// 而不归还——一次归还就这么凭空消失（T4.7 并发用例实测：`released` 只有
    /// `acquired` 的一半）。现在认领只表示「谁来 publish」，且**谁有裸连接谁才认领**。
    pub(crate) fn mark_guard_gone(&self) {
        self.guard_gone.store(true, Ordering::Release);
    }

    /// 句柄是否已经走了（阻塞侧收官时用它决定要不要顺手归还）。
    pub(crate) fn guard_is_gone(&self) -> bool {
        self.guard_gone.load(Ordering::Acquire)
    }

    /// 认领「归还并发布」的权利。两侧都调，**恰有一方**拿到 `true`，所以
    /// 既不会漏掉一次归还，也不会把同一条连接归还两次。
    pub(crate) fn claim_publish(&self) -> bool {
        self.returned
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// 把裸连接取走交给阻塞闭包。`None` = 此刻没有可借出的连接。
    ///
    /// **一次借出的开始**：归还握手的两面旗标在这里复位。它们是「本次借出」的
    /// 状态而不是连接的固有属性——忘了复位，第二次借出同一条连接时
    /// `guard_gone`/`returned` 还留着上一次的 `true`，两侧都会认为对方已经发布过，
    /// 于是这次归还没有任何人执行（T4.7 并发用例实测：一半的归还凭空消失、
    /// `released` 只有 `acquired` 的一半、物理连接只增不减）。
    pub(crate) fn take_conn(&self) -> Option<B> {
        let mut slot = self.lock_slot();
        let raw = slot.conn.take();
        if raw.is_some() {
            self.set_state(ConnState::InFlight);
            self.guard_gone.store(false, Ordering::Release);
            self.returned.store(false, Ordering::Release);
        }
        raw
    }

    /// 把裸连接放回停放位。**不改状态**——终态由调用方先写好（顺序：
    /// 先 `set_state` 再 `put_conn`），两者之间的窗口正是
    /// 「句柄已取消/已 drop」的检票口。
    pub(crate) fn put_conn(&self, conn: B, now: Instant) {
        let mut slot = self.lock_slot();
        slot.conn = Some(conn);
        slot.idle_since = now;
    }

    /// 取走空位票（回池/销毁时释放）。
    pub(crate) fn take_ticket(&self) -> Option<OwnedSemaphorePermit> {
        self.lock_slot().ticket.take()
    }

    /// 装上空位票（`acquire` 借出时空闲连接不占票，需要新票）。
    pub(crate) fn attach_ticket(&self, ticket: OwnedSemaphorePermit) {
        let mut slot = self.lock_slot();
        debug_assert!(slot.ticket.is_none(), "连接上不该同时存在两张空位票");
        slot.ticket = Some(ticket);
    }

    /// 裸连接是否还在槽里。归还路径用它判断「能不能复用」——
    /// 被取消的桥接任务会把裸连接带走，那种连接不能再进空闲栈。
    pub(crate) fn has_conn(&self) -> bool {
        self.lock_slot().conn.is_some()
    }

    /// 归还时刻。复位后重新入栈时要刷新（`idle_timeout` 从**归还**起算）。
    pub(crate) fn idle_since(&self) -> Instant {
        self.lock_slot().idle_since
    }

    /// 探活节流：距上次探活是否已超过 `interval`。
    pub(crate) fn needs_check(&self, interval: std::time::Duration, now: Instant) -> bool {
        now.duration_since(self.lock_slot().last_checked) >= interval
    }

    pub(crate) fn mark_checked(&self, now: Instant) {
        self.lock_slot().last_checked = now;
    }

    pub(crate) fn stats(&self) -> &ConnStats {
        &self.stats
    }
}

// ---------------------------------------------------------------------------
// 借出句柄
// ---------------------------------------------------------------------------

/// 借出句柄：**drop 即归还**（§5.7 的 `PoolGuard`）。
///
/// 归还判定在这里做，但它**不做任何 IO**：drop 发生在 tokio 工作线程上，
/// 在那里同步调 ODBC 会阻塞整个运行时（§5.8）。所以这里只把连接分派给池的
/// 空闲栈或待复位队列，真正的 `reset()`/`close()` 由专用阻塞线程或后台回收任务执行。
pub struct PoolGuard<B: Backend = crate::DefaultBackend> {
    conn: Option<Arc<ConnInner<B>>>,
    pool: Arc<PoolInner<B>>,
    /// 调用方（`dm-query`）显式标脏：开了事务、留了游标、或做了一半。
    dirty: bool,
}

impl<B: Backend> std::fmt::Debug for PoolGuard<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolGuard")
            .field("conn_id", &self.conn.as_ref().map(|c| c.id()))
            .field("state", &self.state())
            .field("dirty", &self.dirty)
            .finish()
    }
}

impl<B: Backend> PoolGuard<B> {
    pub(crate) fn new(conn: Arc<ConnInner<B>>, pool: Arc<PoolInner<B>>) -> Self {
        Self {
            conn: Some(conn),
            pool,
            dirty: false,
        }
    }

    /// 连接编号（日志/指标用，池内唯一且单调）。
    pub fn id(&self) -> u64 {
        self.conn.as_ref().map_or(0, |c| c.id())
    }

    /// 连接当前状态标签。
    pub fn state(&self) -> &'static str {
        self.conn
            .as_ref()
            .map_or("Returned", |c| c.state().as_str())
    }

    /// 这条连接所属池的配置（只读）。
    pub fn config(&self) -> &DmConfig {
        self.pool.config()
    }

    /// 这条连接的累计使用统计。
    pub fn conn_stats(&self) -> ConnStatsSnapshot {
        let Some(conn) = &self.conn else {
            return ConnStatsSnapshot::default();
        };
        let s = conn.stats();
        ConnStatsSnapshot {
            ops: s.ops.load(Ordering::Relaxed),
            errors: s.errors.load(Ordering::Relaxed),
            rows: s.rows.load(Ordering::Relaxed),
            total_nanos: s.total_nanos.load(Ordering::Relaxed),
            slow_ops: s.slow_ops.load(Ordering::Relaxed),
        }
    }

    /// 这条连接的语句缓存统计（CAP-C-11 的观测口）。
    ///
    /// 句柄已经归还（`state == "Returned"`）时返回全零：缓存已经随连接走了。
    pub fn stmt_cache_stats(&self) -> StmtCacheStats {
        match &self.conn {
            Some(conn) => conn.stmt_cache_stats(),
            None => StmtCacheStats::default(),
        }
    }

    /// 标记「这条连接不能干净复用」。归还时会走复位或直接销毁。
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// 标记「状态未知」：等价于 [`mark_dirty`](Self::mark_dirty)，但语义更强，
    /// 归还时**跳过复位直接销毁**（取消/超时/驱动报错后的路径，**D11**）。
    pub fn mark_poisoned(&mut self) {
        self.dirty = true;
        if let Some(conn) = &self.conn {
            conn.set_state(ConnState::Poisoned);
        }
    }

    /// 在阻塞线程上跑一段需要裸连接的操作——**异步↔阻塞桥接的唯一入口**（§5.4）。
    ///
    /// 闭包必须是 `'static`：它会被 move 到另一个线程上执行，所以不能借用调用方的栈。
    /// 需要传参就 move 拥有所有权的缓冲进去（`dm-query` 的参数 arena 就是这么做的）。
    ///
    /// # 取消语义（**D11**）
    ///
    /// 调用方放弃等待（`timeout` / `select!` / 上层 drop）时，闭包**仍会跑完**——
    /// 阻塞调用没法安全中止。闭包返回后会用 [`JobReply::is_cancelled`] 自检：
    /// 一旦发现没人要结果了，就把连接标成 [`ConnState::Poisoned`]（除非配置里
    /// 开了 `reuse_after_cancel` 逃生舱），从而保证「取消过的连接绝不回池」。
    ///
    /// [`JobReply::is_cancelled`]: crate::JobReply::is_cancelled
    pub async fn with_conn<T, F>(&mut self, f: F) -> Result<T, DmError>
    where
        T: Send + 'static,
        F: FnOnce(&mut B) -> Result<T, DmError> + Send + 'static,
    {
        self.with_conn_inner(move |raw, _conn| f(raw)).await
    }

    /// [`with_conn`](Self::with_conn) 的内部形态：闭包还能拿到这条连接的共享核心
    /// （语句缓存挂在它上面，见 [`with_cached_stmt`](Self::with_cached_stmt)）。
    pub(crate) async fn with_conn_inner<T, F>(&mut self, f: F) -> Result<T, DmError>
    where
        T: Send + 'static,
        F: FnOnce(&mut B, &Arc<ConnInner<B>>) -> Result<T, DmError> + Send + 'static,
    {
        let conn = self.conn.as_ref().ok_or_else(guard_returned_error)?;
        let raw = conn.take_conn().ok_or_else(conn_busy_error)?;

        let job_conn = Arc::clone(conn);
        let pool = Arc::clone(&self.pool);
        let job_pool = Arc::clone(&pool);
        let reuse_after_cancel = pool.config().pool.reuse_after_cancel;
        // 慢查询阈值也在提交前读一次：闭包跑在阻塞线程上，不碰池锁（§5.8）。
        let slow_threshold = pool.config().slow_query_threshold;
        let started = Instant::now();

        let rx = pool
            .executor()
            .submit(move |reply: crate::JobReply<Result<T, DmError>>| {
                let mut raw = raw;
                // 闭包 panic 必须在这里就地接住。若让它穿出去，`put_conn` 就永远
                // 不会执行：裸连接在 `raw` 里随栈展开被析构（对 odbc-api 那意味着
                // 一次没配对的 `SQLFreeHandle`），连接名额与存活计数也一起漏掉
                // （`wait_closed` 于是永远等不到零）。接住之后按「状态未知」处理，
                // 与取消/超时同一条路：毒化 + 物理关闭。
                let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    f(&mut raw, &job_conn)
                })) {
                    Ok(result) => result,
                    Err(payload) => Err(panic_in_operation(panic_payload_message(&*payload))),
                };
                let now = Instant::now();
                let cancelled = reply.is_cancelled();

                // 统计：无论成败都记账，便于「这条连接是不是有问题」的判断。
                let stats = job_conn.stats();
                stats.ops.fetch_add(1, Ordering::Relaxed);
                let elapsed = elapsed_nanos(started, now);
                stats.total_nanos.fetch_add(elapsed, Ordering::Relaxed);
                if !slow_threshold.is_zero() && elapsed >= slow_query_nanos(slow_threshold) {
                    stats.slow_ops.fetch_add(1, Ordering::Relaxed);
                }
                if result.is_err() {
                    stats.errors.fetch_add(1, Ordering::Relaxed);
                }

                // 终态：失败 ⇒ 脏（要复位）；成功但被取消 ⇒ 状态未知（D11）。
                // panic 走的是「失败」分支，但它的错误种类是 `Io`（连接不可信），
                // 所以下面按 `is_connection_lost` 直接毒化。
                let lost = result
                    .as_ref()
                    .err()
                    .is_some_and(|err| err.is_connection_lost());
                let state = match (&result, cancelled) {
                    (Err(_), _) if lost => ConnState::Poisoned,
                    (Err(_), _) => ConnState::Dirty,
                    (Ok(_), true) if !reuse_after_cancel => {
                        job_pool.note_cancelled();
                        ConnState::Poisoned
                    }
                    (Ok(_), _) => ConnState::Idle,
                };
                // 顺序很重要：**先定终态，再把裸连接放回槽里**。归还那侧是靠
                // 「裸连接在槽里」判断「桥接已经收官的」，所以它一旦看到裸连接，
                // 读到的终态必然已经写好。
                job_conn.set_state(state);
                job_conn.put_conn(raw, now);
                // 收官：句柄要是已经走了，就由阻塞侧把连接还给池。它能看到
                // 「句柄已走」而这个时刻裸连接已经在槽里，因此认领必然能落地。
                if job_conn.guard_is_gone() && job_conn.claim_publish() {
                    job_pool.publish(job_conn, state);
                }

                let _ = reply.send(result);
            })
            .await?;

        match rx.await {
            Ok(result) => result,
            Err(_) if pool.is_shutting_down() => Err(pool.closed_error()),
            // 闭包 panic（或忘了回传）：状态未知，按「连接不可复用」处理。
            Err(_) => Err(reply_lost_error()),
        }
    }

    /// 在阻塞线程上跑一段**需要预处理语句**的操作：语句按 SQL 文本缓存在这条连接上
    /// （CAP-C-11，设计方案 §5.4）。
    ///
    /// 命中时复用上次的语句句柄，省掉一次 `SQLPrepare` 往返（§3.1 实测 ②−① ≈ 31 µs）。
    /// 未命中就新建一条，跑成功之后放进缓存；**执行失败就把这条语句丢掉**——
    /// ODBC 语句在错误之后状态未知（游标位置、参数绑定、结果集都可能残留），
    /// 下一趟重建远比复用一条半坏的语句便宜。
    ///
    /// # 只有「还能再执行」的语句才进缓存
    ///
    /// 放回缓存之前会问一次 [`Backend::statement_reusable`]：达梦的结果集语句**取尽之后
    /// 句柄就死了**（`24000/-70027`，真机证据见 `crates/dm-odbc/examples/cursor_reuse_probe.rs`），
    /// 所以那种语句只跑这一次、不放回（下次同一条 SQL 重新 `prepare`），查询语句因此
    /// 不跨调用复用；DML/DDL 形态照常复用。判据由后端给，假后端默认 `true`。
    ///
    /// 缓存随连接走：连接被复位时清空（`inner.rs::recycle_one`），被销毁时随句柄释放。
    /// 容量来自 `DmConfig::pool.stmt_cache_size`，`0` 表示关闭；关闭时本方法等价于
    /// 「自己 `prepare` 一条临时语句」，行为与没有这一层时完全一致。
    ///
    /// # 闭包约束
    ///
    /// 与 [`with_conn`](Self::with_conn) 同一套（`'static`、在阻塞线程上跑、panic 被
    /// 接住后按「状态未知」处理），参数的第三个分量是 SQL 文本本身——缓存键用它，
    /// 调用方不必再自己 clone 一份。语句的所有权归缓存，闭包不能把它带走。
    pub async fn with_cached_stmt<T, F>(&mut self, sql: String, f: F) -> Result<T, DmError>
    where
        T: Send + 'static,
        F: FnOnce(&mut B, &mut B::Stmt, &str) -> Result<T, DmError> + Send + 'static,
    {
        self.with_conn_inner(move |raw, conn| {
            let mut cache = conn.lock_stmt_cache();
            if !cache.is_enabled() {
                // 缓存关闭：新建 → 用 → 随作用域释放，与服务端不残留任何语句。
                let mut stmt = raw.prepare(&sql)?;
                return f(raw, &mut stmt, &sql);
            }
            match cache.take(&sql) {
                Some(mut entry) => match f(raw, entry.stmt_mut(), &sql) {
                    Ok(value) => {
                        if raw.statement_reusable(entry.stmt()) {
                            cache.put(entry);
                        } else {
                            // 驱动说这条语句不能再来一次（达梦的结果集语句）：
                            // 条目在这里 drop，下次同一条 SQL 重新 prepare。
                            tracing::debug!("语句不可复用：不放进缓存");
                        }
                        Ok(value)
                    }
                    Err(err) => {
                        // 条目在这里被 drop：语句不回收，下次重建。
                        tracing::debug!(error = %err, "语句执行失败：丢弃缓存里的这条语句");
                        Err(err)
                    }
                },
                None => {
                    let mut stmt = raw.prepare(&sql)?;
                    cache.note_miss();
                    match f(raw, &mut stmt, &sql) {
                        Ok(value) => {
                            if raw.statement_reusable(&stmt) {
                                cache.put(StmtEntry::new(sql.clone(), stmt));
                            }
                            Ok(value)
                        }
                        Err(err) => Err(err),
                    }
                }
            }
        })
        .await
    }

    /// 带超时的操作（**G4 判据**：超时必须报 [`DmErrorKind::Timeout`]，且这条
    /// 连接不许再回池）。
    ///
    /// 与 [`with_conn`](Self::with_conn) 的关系：这只是在它外面套一层
    /// `tokio::time::timeout`，**销毁语义完全靠 `with_conn` 自带的取消检测**——
    /// 超时会把 `with_conn` 的 future drop 掉，阻塞闭包跑完后
    /// `JobReply::is_cancelled()` 为真，于是把连接标成
    /// [`ConnState::Poisoned`]（除非开了 `reuse_after_cancel`）。这里再显式
    /// [`mark_poisoned`](Self::mark_poisoned) 一次是**双保险**：万一闭包在超时
    /// 之前就已经收官、恰好又把结果送到一个刚被丢弃的 receiver 上，句柄这一侧
    /// 也必须按「不可复用」归还。
    pub async fn with_timeout<T, F>(
        &mut self,
        limit: std::time::Duration,
        f: F,
    ) -> Result<T, DmError>
    where
        T: Send + 'static,
        F: FnOnce(&mut B) -> Result<T, DmError> + Send + 'static,
    {
        match tokio::time::timeout(limit, self.with_conn(f)).await {
            Ok(result) => result,
            Err(_elapsed) => {
                self.mark_poisoned();
                let mut err = DmError::at(DmErrorKind::Timeout, Stage::Execute);
                err.push_diag(dm_error::DiagRecord::native(
                    0,
                    format!(
                        "操作超过 {} ms 未返回：连接状态未知，已按不可复用处理（D11）",
                        limit.as_millis()
                    ),
                ));
                Err(err)
            }
        }
    }

    /// 探活（`SQL_ATTR_CONNECTION_DEAD`，不跑 SQL）。见 `health.rs`。
    pub async fn ping(&mut self) -> Result<bool, DmError> {
        let conn = self.conn.as_ref().ok_or_else(guard_returned_error)?;
        if !conn.needs_check(
            self.pool.config().pool.health_check_interval,
            Instant::now(),
        ) {
            return Ok(true); // 节流：刚探过就别再问
        }
        let checked = crate::health::check(&self.pool, Arc::clone(conn)).await?;
        Ok(checked)
    }
}

impl<B: Backend> Drop for PoolGuard<B> {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return; // 已经显式归还过
        };
        // 归还责任的握手（见 `ConnInner::mark_guard_gone` 的文档）：
        //   句柄侧：先宣告「我走了」→ 再看裸连接在不在槽里 → 在才认领；
        //   阻塞侧：先把裸连接放回槽里 → 再看句柄走没走 → 走了才认领。
        // 两个方向的读写顺序互为反向（论证见模块头），于是
        // 「两边都没归还」和「两边都归还」都不可能发生。
        conn.mark_guard_gone();
        if !conn.has_conn() {
            // 认领了也没用：裸连接还在阻塞闭包手里（取消/超时的典型情形），
            // 回收线程拿不到它。留给阻塞侧收官时归还——它会看到句柄已经走了。
            return;
        }
        if !conn.claim_publish() {
            // 桥接任务已经归还并发布过了，句柄不再插手。
            return;
        }
        let state = conn.state();
        let final_state = if self.dirty && state == ConnState::Idle {
            ConnState::Dirty
        } else {
            state
        };
        conn.set_state(final_state);
        self.pool.publish(conn, final_state);
    }
}

fn elapsed_nanos(start: Instant, end: Instant) -> u64 {
    u64::try_from(end.duration_since(start).as_nanos()).unwrap_or(u64::MAX)
}

/// 把慢查询阈值换成纳秒（溢出按 `u64::MAX` 饱和，等价于「永不触发」）。
fn slow_query_nanos(threshold: std::time::Duration) -> u64 {
    u64::try_from(threshold.as_nanos()).unwrap_or(u64::MAX)
}

/// `catch_unwind` 的负载转成人能读的字符串。
pub(crate) fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "（非字符串 panic 负载）".to_owned()
    }
}

/// 操作闭包 panic 时给调用方的错误。
///
/// 种类选 `Io`（`is_connection_lost() == true`）：panic 之后这条连接的状态
/// 已经不可知，必须物理关闭而不是复位复用（**D11** 的同一条道理）。
fn panic_in_operation(message: String) -> DmError {
    let mut err = DmError::at(DmErrorKind::Io, Stage::Execute);
    err.push_diag(dm_error::DiagRecord::native(
        0,
        format!("操作闭包 panic：{message}（连接状态未知，已丢弃）"),
    ));
    err
}

// ---------------------------------------------------------------------------
// 错误构造
// ---------------------------------------------------------------------------

pub(crate) fn guard_returned_error() -> DmError {
    DmError::at(
        DmErrorKind::Config("连接已归还：PoolGuard 归还之后不能再使用".to_owned()),
        Stage::Config,
    )
}

pub(crate) fn conn_busy_error() -> DmError {
    DmError::at(
        DmErrorKind::Config("连接正被另一个操作占用：PoolGuard 上的操作必须串行".to_owned()),
        Stage::Config,
    )
}

fn reply_lost_error() -> DmError {
    let mut err = DmError::at(DmErrorKind::Io, Stage::Execute);
    err.push_diag(dm_error::DiagRecord::native(
        0,
        "阻塞任务没有回传结果（多半是 panic）：连接状态未知，已丢弃".to_owned(),
    ));
    err
}
