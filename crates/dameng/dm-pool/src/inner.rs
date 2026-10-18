//! 池核心：门控、空闲栈、借出/归还、建连、关池（设计方案 §5.7、§5.8、§6.12）。
//!
//! ## 名额核算规则（本文件最重要的一条）
//!
//! 池用一个 `tokio::sync::Semaphore`（容量 `max_size`）做门控，规则只有一句：
//!
//! > **在飞（借出中 / 待复位 / 正在建）的连接各自持有 1 张空位票；
//! > 停在空闲栈里的连接一张都不持有。**
//!
//! 于是「可用票数 = `max_size` − 在飞数」，而空闲连接之所以能被借出，
//! 靠的是它归还时**把票还回了信号量**：谁拿到票，谁就有权拿走一条连接
//! （优先从空闲栈拿，栈空则新建）。这条规则同时解决了两个看起来矛盾的需求：
//!
//! - `in_flight + idle ≤ max_size`——没有票就绝不能凭空多出一条连接；
//! - **归还必须能唤醒等待者**——如果空闲连接也攥着票，归还时票数不变，
//!   等票的人永远醒不过来（`max_size` 条连接全空闲、全员却都在等，典型自锁）。
//!
//! 每次借出都是「**先拿票，再挑连接**」：顺序反了（先挑连接再拿票）会在
//! 「票被别的等待者抢走」时出现一条无票在飞的连接，把 `max_size` 顶破。
//!
//! ## 为什么所有 IO 都走阻塞线程
//!
//! 建连、复位、探活、物理关闭全是同步 ODBC 调用（§4.5 达梦不支持异步），
//! 在 tokio 工作线程上直接做会卡住整个运行时（§5.8）。所以本文件里
//! **只有 `exec.run(...)` 里面的闭包碰裸连接**，其余全是锁 + 原子操作。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use dm_core::{Backend, DmConfig};
use dm_error::{DmError, DmErrorKind, Stage};
use tokio::runtime::Handle;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio::time::{Instant as TokioInstant, timeout};

use crate::conn::{ConnInner, ConnState, PoolGuard};
use crate::exec::{BlockingExecutor, ExecutorConfig};
use crate::stats::{PoolCounters, PoolState, PoolStats};

/// 存活连接计数：池与每条连接共享（连接被丢弃时也要能把数减对）。
#[derive(Debug, Default)]
pub(crate) struct Liveness {
    live: AtomicUsize,
    notify: Notify,
}

impl Liveness {
    pub(crate) fn add(&self) {
        self.live.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn gone(&self) {
        let prev = self.live.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(
            prev > 0,
            "存活计数被减到负数：release_liveness 的幂等性坏了"
        );
        if prev <= 1 {
            // `wait_closed` 靠这条通知醒来。
            self.notify.notify_waiters();
        }
    }

    pub(crate) fn live(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    /// 等存活数归零。先登记的 `Notified` 用 `enable()` 抢在检查之前入队，
    /// 否则「检查为 0」之后、`await` 之前发生的那次通知会被漏掉。
    pub(crate) async fn wait_zero(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.live() == 0 {
                return;
            }
            notified.await;
        }
    }
}

/// 池的内部核心。对外只通过 [`DmPool`] 暴露。
pub(crate) struct PoolInner<B: Backend> {
    cfg: Arc<DmConfig>,
    exec: BlockingExecutor,
    /// 建连/借用名额。语义见本文件头部。
    permits: Arc<Semaphore>,
    /// 空闲栈（LIFO）。用后进先出是为了让连接「轮换着闲下来」，
    /// 空闲超期回收才有机会淘汰掉旧连接，而不是把最老的那条一直留在栈底。
    idle: Mutex<VecDeque<Arc<ConnInner<B>>>>,
    /// 已归还但需要复位（或需要销毁）的连接。**持有名额**。
    to_recycle: Mutex<VecDeque<Arc<ConnInner<B>>>>,
    state: AtomicU8,
    counters: PoolCounters,
    liveness: Arc<Liveness>,
    next_id: std::sync::atomic::AtomicU64,
    creating: AtomicUsize,
    /// 待复位队列的「有人在处理」闸门：保证同一时刻只有一个排空任务。
    recycling: AtomicBool,
    /// 保活建连的单飞闸门（§5.7：避免雪崩式建连）。
    filling: AtomicBool,
    /// 建连时取运行时句柄：`publish()` 可能在阻塞线程上被调用，
    /// 那里没有隐式的运行时上下文，`tokio::spawn` 会 panic。
    handle: Handle,
    maintenance: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl<B: Backend> std::fmt::Debug for PoolInner<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.snapshot();
        f.debug_struct("PoolInner")
            .field("state", &self.state())
            .field("stats", &stats)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> PoolInner<B> {
    /// 建池。`first` 是已经建好的第一条连接（由 `DmPool::connect_with` 建），
    /// 它直接进空闲栈且**不持有名额**。
    fn new(cfg: Arc<DmConfig>, exec: BlockingExecutor, first: B, handle: Handle) -> Arc<Self> {
        let now = Instant::now();
        let liveness = Arc::new(Liveness::default());
        // 建池这条连接同样带语句缓存（CAP-C-11）：它在空闲栈里等着被借出，
        // 借出去的那一刻就该和后来补建的连接一样有缓存可用。
        let stmt_cache_size = cfg.pool.stmt_cache_size;
        let first = ConnInner::new(0, first, None, now, Arc::clone(&liveness), stmt_cache_size);
        let inner = Arc::new(Self {
            permits: Arc::new(Semaphore::new(cfg.pool.max_size)),
            exec,
            idle: Mutex::new(VecDeque::from([first])),
            to_recycle: Mutex::new(VecDeque::new()),
            state: AtomicU8::new(PoolState::Running.as_u8()),
            counters: PoolCounters::new(),
            liveness,
            next_id: std::sync::atomic::AtomicU64::new(1),
            creating: AtomicUsize::new(0),
            recycling: AtomicBool::new(false),
            filling: AtomicBool::new(false),
            handle,
            maintenance: Mutex::new(None),
            cfg,
        });
        PoolCounters::add(&inner.counters.created);
        inner
    }

    fn state(&self) -> PoolState {
        PoolState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, state: PoolState) {
        self.state.store(state.as_u8(), Ordering::Release);
    }

    pub(crate) fn is_shutting_down(&self) -> bool {
        self.state() != PoolState::Running
    }

    pub(crate) fn config(&self) -> &DmConfig {
        &self.cfg
    }

    /// 阻塞线程池。`health.rs` 要借它把探活挪到阻塞线程上。
    pub(crate) fn executor(&self) -> &BlockingExecutor {
        &self.exec
    }

    /// 记一次「因取消/超时被判不可复用」（**D11**）。`conn.rs` 的桥接闭包调它。
    pub(crate) fn note_cancelled(&self) {
        PoolCounters::add(&self.counters.recycled_cancelled);
    }

    pub(crate) fn closed_error(&self) -> DmError {
        DmError::at(
            DmErrorKind::Config(format!("连接池已{}，不再接受新的借出请求", self.state())),
            Stage::Connect,
        )
    }

    fn ensure_open(&self) -> Result<(), DmError> {
        match self.state() {
            PoolState::Running => Ok(()),
            PoolState::Closing | PoolState::Closed => Err(self.closed_error()),
        }
    }

    // ---------------------------------------------------------------
    // 借出
    // ---------------------------------------------------------------

    pub(crate) async fn acquire(self: &Arc<Self>) -> Result<PoolGuard<B>, DmError> {
        let acquire_timeout = self.cfg.pool.acquire_timeout;
        let deadline = TokioInstant::now() + acquire_timeout;
        loop {
            self.ensure_open()?;
            // 先拿票，再挑连接——顺序不能反，理由见文件头部。
            let permit = self.acquire_permit(deadline, acquire_timeout).await?;
            self.ensure_open()?; // 拿到票的瞬间池可能刚好在关

            let Some(conn) = self.pop_idle() else {
                return self.open_guard(permit).await;
            };
            conn.attach_ticket(permit);

            let now = Instant::now();
            if now.duration_since(conn.created_at()) >= self.cfg.pool.max_lifetime {
                PoolCounters::add(&self.counters.lifetime_rotated);
                tracing::debug!(conn = conn.id(), "借出前轮换：超过 max_lifetime");
                self.discard(conn).await;
                continue;
            }
            if self.cfg.pool.test_on_acquire
                || conn.needs_check(self.cfg.pool.health_check_interval, now)
            {
                match crate::health::check(self, Arc::clone(&conn)).await {
                    Ok(true) => conn.mark_checked(now),
                    Ok(false) => {
                        PoolCounters::add(&self.counters.health_check_failed);
                        tracing::warn!(conn = conn.id(), "借出前探活失败：连接已死，淘汰");
                        self.discard(conn).await;
                        continue;
                    }
                    Err(err) if err.is_connection_lost() => {
                        PoolCounters::add(&self.counters.health_check_failed);
                        tracing::warn!(conn = conn.id(), error = %err, "借出前探活报断连：淘汰");
                        self.discard(conn).await;
                        continue;
                    }
                    // 探活本身报错但不像断连：不淘汰，让真正的操作去暴露它。
                    Err(err) => {
                        tracing::warn!(conn = conn.id(), error = %err, "探活失败（非断连），仍借出");
                        conn.mark_checked(now);
                    }
                }
            }

            PoolCounters::add(&self.counters.acquired);
            return Ok(PoolGuard::new(conn, Arc::clone(self)));
        }
    }

    /// 取一张空位票。等待期间维护 `waiting` 指标（用 RAII 保证被取消也会减掉）。
    async fn acquire_permit(
        self: &Arc<Self>,
        deadline: TokioInstant,
        acquire_timeout: Duration,
    ) -> Result<OwnedSemaphorePermit, DmError> {
        match Arc::clone(&self.permits).try_acquire_owned() {
            Ok(permit) => Ok(permit),
            Err(TryAcquireError::Closed) => Err(self.closed_error()),
            Err(TryAcquireError::NoPermits) => {
                self.counters.waiting.fetch_add(1, Ordering::Relaxed);
                let _waiting = Waiting::new(&self.counters);
                let remaining = deadline.saturating_duration_since(TokioInstant::now());
                if remaining.is_zero() {
                    PoolCounters::add(&self.counters.timed_out);
                    return Err(pool_timeout(acquire_timeout));
                }
                match timeout(remaining, Arc::clone(&self.permits).acquire_owned()).await {
                    Ok(Ok(permit)) => Ok(permit),
                    Ok(Err(_closed)) => Err(self.closed_error()),
                    Err(_elapsed) => {
                        PoolCounters::add(&self.counters.timed_out);
                        Err(pool_timeout(acquire_timeout))
                    }
                }
            }
        }
    }

    fn pop_idle(&self) -> Option<Arc<ConnInner<B>>> {
        self.idle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_back()
    }

    /// 用一张票换一条新连接，并直接包成句柄。
    async fn open_guard(
        self: &Arc<Self>,
        permit: OwnedSemaphorePermit,
    ) -> Result<PoolGuard<B>, DmError> {
        let conn = self.open(permit).await?;
        PoolCounters::add(&self.counters.acquired);
        Ok(PoolGuard::new(conn, Arc::clone(self)))
    }

    /// 新建一条物理连接（在阻塞线程上建），返回已登记存活计数的 `ConnInner`。
    async fn open(
        self: &Arc<Self>,
        permit: OwnedSemaphorePermit,
    ) -> Result<Arc<ConnInner<B>>, DmError> {
        self.creating.fetch_add(1, Ordering::AcqRel);
        let cfg = Arc::clone(&self.cfg);
        let result = self.exec.run(move || B::connect(&cfg)).await;
        self.creating.fetch_sub(1, Ordering::AcqRel);

        match result {
            Ok(Ok(raw)) => {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                PoolCounters::add(&self.counters.created);
                Ok(ConnInner::new(
                    id,
                    raw,
                    Some(permit),
                    Instant::now(),
                    Arc::clone(&self.liveness),
                    self.cfg.pool.stmt_cache_size,
                ))
            }
            Ok(Err(err)) => {
                PoolCounters::add(&self.counters.connect_failed);
                tracing::warn!(error = %err, "建连失败");
                Err(err)
            }
            Err(err) => {
                // 阻塞线程没回传结果（panic / 关闭）：同样算建连失败。
                PoolCounters::add(&self.counters.connect_failed);
                Err(err)
            }
        }
    }

    // ---------------------------------------------------------------
    // 归还
    // ---------------------------------------------------------------

    /// 归还入口。**绝不做 IO**（可能在 tokio 工作线程上被 `PoolGuard::drop` 调用）：
    /// 干净的连接直接入空闲栈并还票，脏的/毒化的/池已关的一律丢进待复位队列。
    pub(crate) fn publish(self: &Arc<Self>, conn: Arc<ConnInner<B>>, state: ConnState) {
        if !conn.has_conn() {
            // 防御：裸连接还在桥接任务手里，这次调用没有东西可归还。
            // 真正的归还由阻塞侧收官时发起（那时它会看到句柄已经走了）。
            return;
        }
        // 归还记账。`publish` 是「一次真归还」的唯一入口：句柄 drop 时由
        // `PoolGuard::drop` 调，句柄先走时由阻塞侧的收官分支调（探活那条路
        // 同理），两边**恰有一方**会调 —— 所以在这里 +1 不会重复也不会漏。
        PoolCounters::add(&self.counters.released);
        if state == ConnState::Idle && self.state() == PoolState::Running {
            self.publish_idle(conn);
        } else {
            self.enqueue_recycle(conn);
        }
    }

    fn publish_idle(self: &Arc<Self>, conn: Arc<ConnInner<B>>) {
        conn.set_state(ConnState::Idle);
        // 三步顺序都是必须的（T4.7 真机 256×1000 才把它逼出来）：
        //   1. **先摘票**：入栈之后这条连接随时会被别人借走，而借出路径会挂新票。
        //      先入栈后摘票会出现「一条连接上挂两张票」——轻则 debug_assert 炸，
        //      重则借出者拿到的连接被这边的 `drop(ticket)` 摘空、信号量白多出一张
        //      票，于是 `max_size` 被顶破（正是 G4 要防的事）。
        //   2. **再入栈**：等票的人在票被放回时才可能醒来；票先放回的话，他会
        //      看到空栈而**新建**一条连接，而这条连接随后才入栈 ⇒ 物理连接数
        //      超出 `max_size`（真机上就是 `v$sessions` 超标）。
        //   3. **最后放票**：唤醒等待者，让他能借到刚入栈的这条。
        let ticket = conn.take_ticket();
        self.idle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(Arc::clone(&conn));
        // 空闲连接不持有名额 ⇒ 把它还回信号量，唤醒一个等待者。
        drop(ticket);
    }

    fn enqueue_recycle(self: &Arc<Self>, conn: Arc<ConnInner<B>>) {
        if !conn.has_conn() {
            // 桥接任务还在跑并攥着裸连接，收官由它负责（它会再调一次 publish）。
            return;
        }
        // 保住 Poisoned：它决定回收线程是「先复位试试」还是「直接销毁」。
        // 把毒化降级成 Dirty 会让一条状态未知的连接被复位后复用（**D11**）。
        if conn.state() != ConnState::Poisoned {
            conn.set_state(ConnState::Dirty);
        }
        self.to_recycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(Arc::clone(&conn));
        self.schedule_recycle();
    }

    /// 确保有一个排空任务在处理待复位队列。可能从任意线程调用 ⇒ 必须用
    /// 构造池时抓到的 `Handle` 而不是 `tokio::spawn`。
    fn schedule_recycle(self: &Arc<Self>) {
        if self
            .recycling
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let pool = Arc::clone(self);
        self.handle.clone().spawn(async move {
            let job = Arc::clone(&pool);
            if let Err(err) = pool.exec.run(move || job.recycle_all()).await {
                // 阻塞线程池已经关了：没人会再排空，下一轮维护会兜底。
                tracing::warn!(error = %err, "待复位队列的排空任务没能提交");
                pool.recycling.store(false, Ordering::Release);
            }
        });
    }

    /// 排空待复位队列（**在阻塞线程上跑**，所以这里可以直接做 IO）。
    fn recycle_all(self: &Arc<Self>) {
        loop {
            let next = self
                .to_recycle
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front();
            match next {
                Some(conn) => self.recycle_one(conn),
                None => {
                    // 顺序关键：先撤闸门再复查队列。反过来的话，
                    // 「清空之后、撤闸门之前」入队的连接会没人处理。
                    self.recycling.store(false, Ordering::Release);
                    if self
                        .to_recycle
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty()
                    {
                        return;
                    }
                    if self
                        .recycling
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    }

    /// 单条连接的复位或销毁（§5.7 的四步复位由 `Backend::reset` 一次做完）。
    fn recycle_one(self: &Arc<Self>, conn: Arc<ConnInner<B>>) {
        let now = Instant::now();
        if conn.state() == ConnState::Poisoned
            || self.state() != PoolState::Running
            || now.duration_since(conn.created_at()) >= self.cfg.pool.max_lifetime
        {
            self.close_physically(conn, "毒化/超期/池已关");
            return;
        }
        let Some(mut raw) = conn.take_conn() else {
            // 按归还责任握手（见 `ConnInner::mark_guard_gone`）：能进待复位队列的
            // 连接必定是「裸连接已经在槽里」且「认领者已经归还」，所以这条分支
            // 不可达。真出现了只能是某个桥接闭包被丢弃（执行器已关）——那时连接
            // 已经没有可用的裸句柄，守恒的兜底是按销毁处理，
            // **绝不能留在队列里空转**（否则 `recycling` 闸门背后的队列永远排不空）。
            tracing::warn!(
                conn = conn.id(),
                "待复位队列里的连接没有裸连接：按已销毁处理"
            );
            drop(conn.take_ticket());
            conn.release_liveness();
            return;
        };
        // 复位前先清空**池侧**的语句缓存（§5.7 四步复位的第 4 步是驱动侧清语句，
        // 这里是它的外层对应物）：一条脏连接上的缓存语句未必还带着干净的游标状态，
        // 全部丢弃换一个确定的起点。句柄在这里被 drop，比 `raw.reset()` 先发生，
        // 于是复位看到的待释放语句更少。
        conn.clear_stmt_cache();
        // 复位跑的是驱动的同步代码，panic 是有可能的（odbc-api 的 `Drop` 就会
        // 在句柄释放失败时 panic）。**必须就地接住**：让 panic 穿出
        // `recycle_all` 会把 `recycling` 闸门永久留在 `true`，待复位队列就再
        // 也排不空了（而且 `conn.put_conn` 也被跳过）。
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| raw.reset())) {
            Ok(outcome) => outcome,
            Err(payload) => Err(panic_error(
                Stage::Reset,
                "复位时 panic",
                crate::conn::panic_payload_message(&*payload),
            )),
        };
        conn.put_conn(raw, now);
        match outcome {
            Ok(()) => {
                PoolCounters::add(&self.counters.recycled);
                tracing::trace!(conn = conn.id(), "复位成功，回空闲栈");
                self.publish_idle(conn);
            }
            Err(err) => {
                PoolCounters::add(&self.counters.recycle_failed);
                tracing::warn!(conn = conn.id(), error = %err, "复位失败：物理关闭");
                self.close_physically(conn, "复位失败");
            }
        }
    }

    /// 物理关闭（**在阻塞线程上跑**）。名额随连接上那张票一起归还。
    fn close_physically(self: &Arc<Self>, conn: Arc<ConnInner<B>>, why: &str) {
        // **必须先清语句缓存再关连接**。缓存里是驱动侧的语句句柄，它们只在父连接
        // 还活着的时候才能释放：连接一关，`SQLFreeHandle(SQL_HANDLE_STMT)` 会拿到
        // `SQL_INVALID_HANDLE`（-2），而 `odbc-api` 的 `Drop` 在这种情况直接 panic。
        // 真机踩到的形态：`query_real` 里 7 条用例报
        // `SQLFreeHandle failed with error code: -2`，整套则在退出时
        // `0xc0000005 STATUS_ACCESS_VIOLATION`。
        conn.clear_stmt_cache();
        if let Some(raw) = conn.take_conn() {
            tracing::debug!(conn = conn.id(), why, "物理关闭连接");
            // 驱动在释放句柄时 panic 是有先例的（odbc-api 的 `Drop`：
            // `SQLFreeHandle` 失败就 panic）。接住它，否则后面两行清理会被跳过，
            // 名额与存活计数永久漏掉。
            if let Err(payload) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || raw.close()))
            {
                tracing::error!(
                    conn = conn.id(),
                    panic = %crate::conn::panic_payload_message(&*payload),
                    "关闭连接时 panic：仍按已关闭处理"
                );
            }
        }
        drop(conn.take_ticket());
        conn.release_liveness();
        // 记账：`closed` 是「物理关掉了多少条」，与走哪条路无关。早先漏了这一行，
        // 于是**经回收路径销毁**（毒化/复位失败/超期）的连接一条都没记进去——
        // 离线用例断言的是假后端的 `closed` 计数，所以一直没露馅；真机 G4 用例
        // 断言 `stats.closed`，立刻暴露。
        PoolCounters::add(&self.counters.closed);
    }

    /// 异步版的物理关闭：把关闭动作丢给阻塞线程。借出路径（在 tokio 线程上）用它。
    pub(crate) async fn discard(self: &Arc<Self>, conn: Arc<ConnInner<B>>) {
        // 同 `close_physically`：语句句柄必须先于父连接释放。
        conn.clear_stmt_cache();
        if let Some(raw) = conn.take_conn() {
            let _ = self.exec.run(move || raw.close()).await;
        }
        drop(conn.take_ticket());
        conn.release_liveness();
        PoolCounters::add(&self.counters.closed);
    }

    /// 从空闲栈和待复位队列里把连接全部摘下来（关池用）。
    fn drain_queues(&self) -> Vec<Arc<ConnInner<B>>> {
        let mut out: Vec<_> = self
            .idle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect();
        out.extend(
            self.to_recycle
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..),
        );
        out
    }

    // ---------------------------------------------------------------
    // 维护
    // ---------------------------------------------------------------

    fn spawn_maintenance(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let interval = self.cfg.pool.recycle_interval;
        let handle = self
            .handle
            .clone()
            .spawn(async move { maintenance_loop(weak, interval).await });
        *self.maintenance.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    fn abort_maintenance(&self) {
        if let Some(handle) = self
            .maintenance
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            handle.abort();
        }
    }

    async fn maintain_once(self: &Arc<Self>) {
        let now = Instant::now();
        let running = self.state() == PoolState::Running;
        let mut expired: Vec<Arc<ConnInner<B>>> = Vec::new();
        {
            // 锁区间里只做判断和搬运，释放后再 await（§5.8 的硬纪律）。
            let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
            let mut keep = VecDeque::with_capacity(idle.len());
            while let Some(conn) = idle.pop_front() {
                let idle_too_long =
                    now.duration_since(conn.idle_since()) >= self.cfg.pool.idle_timeout;
                let too_old = now.duration_since(conn.created_at()) >= self.cfg.pool.max_lifetime;
                if running && !idle_too_long && !too_old {
                    keep.push_back(conn);
                } else {
                    if too_old {
                        PoolCounters::add(&self.counters.lifetime_rotated);
                    } else if running {
                        PoolCounters::add(&self.counters.expired);
                    }
                    expired.push(conn);
                }
            }
            *idle = keep;
        }
        for conn in expired {
            self.discard(conn).await;
        }
        self.fill_to_min_idle().await;
    }

    /// 保活：把空闲连接补到 `min_idle`（单飞，避免雪崩式建连）。
    async fn fill_to_min_idle(self: &Arc<Self>) {
        if self.state() != PoolState::Running {
            return;
        }
        if self.filling.swap(true, Ordering::AcqRel) {
            return;
        }
        let result = self.fill_loop().await;
        self.filling.store(false, Ordering::Release);
        if let Err(err) = result {
            tracing::warn!(error = %err, "保活建连失败");
        }
    }

    async fn fill_loop(self: &Arc<Self>) -> Result<(), DmError> {
        loop {
            if self.state() != PoolState::Running {
                return Ok(());
            }
            let idle = self.idle.lock().unwrap_or_else(|e| e.into_inner()).len();
            let dirty = self
                .to_recycle
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            if idle + dirty >= self.cfg.pool.min_idle {
                return Ok(());
            }
            if self.liveness.live() >= self.cfg.pool.max_size {
                return Ok(());
            }
            let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
                // 没票说明在飞的连接已经占满：等它们归还，下一轮再说。
                return Ok(());
            };
            let conn = self.open(permit).await?;
            self.publish_idle(conn);
        }
    }

    // ---------------------------------------------------------------
    // 指标与关池
    // ---------------------------------------------------------------

    pub(crate) fn snapshot(&self) -> PoolStats {
        let idle = self.idle.lock().unwrap_or_else(|e| e.into_inner()).len();
        let to_recycle = self
            .to_recycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        let creating = self.creating.load(Ordering::Acquire);
        let live = self.liveness.live();
        // 在飞 = 存活 − 空闲 − 待复位 − 正在建。用 saturating 是因为
        // 「刚建好还没入队」的瞬间可能出现一帧的负数。
        let in_use = live.saturating_sub(idle + to_recycle + creating);
        self.counters.snapshot(
            idle,
            to_recycle,
            in_use,
            creating,
            self.cfg.pool.max_size,
            self.cfg.pool.min_idle,
        )
    }

    pub(crate) async fn close(self: &Arc<Self>) {
        if self.state() == PoolState::Closed {
            return;
        }
        self.set_state(PoolState::Closing);
        // 关掉信号量：所有等待中的 acquire 立刻以「池已关闭」返回，
        // 而不是傻等到 acquire_timeout。
        self.permits.close();
        for conn in self.drain_queues() {
            self.discard(conn).await;
        }
        self.set_state(PoolState::Closed);
        tracing::debug!(stats = %self.snapshot().summary(), "连接池已关闭");
    }

    pub(crate) async fn wait_closed(self: &Arc<Self>) {
        self.liveness.wait_zero().await;
    }
}

/// 等待指标：被取消（future 被 drop）时也要把 `waiting` 减回去。
struct Waiting<'a> {
    counters: &'a PoolCounters,
}

impl<'a> Waiting<'a> {
    fn new(counters: &'a PoolCounters) -> Self {
        Self { counters }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.counters.waiting.fetch_sub(1, Ordering::Relaxed);
    }
}

fn pool_timeout(after: Duration) -> DmError {
    let mut err = DmError::at(DmErrorKind::PoolTimeout, Stage::Connect);
    err.push_diag(dm_error::DiagRecord::native(
        0,
        format!("等待 {} ms 仍未能从连接池取得连接", after.as_millis()),
    ));
    err
}

/// panic 转成的错误：种类固定 `Io`（连接不可信 ⇒ 丢弃不复用）。
fn panic_error(stage: Stage, what: &str, message: String) -> DmError {
    let mut err = DmError::at(DmErrorKind::Io, stage);
    err.push_diag(dm_error::DiagRecord::native(
        0,
        format!("{what}：{message}（连接状态未知，已丢弃）"),
    ));
    err
}

async fn maintenance_loop<B: Backend>(pool: Weak<PoolInner<B>>, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        let Some(inner) = pool.upgrade() else {
            return; // 池已销毁
        };
        if inner.state() == PoolState::Closed && inner.liveness.live() == 0 {
            return; // 关干净了，任务退出
        }
        inner.maintain_once().await;
    }
}

// ---------------------------------------------------------------------------
// 对外门面
// ---------------------------------------------------------------------------

/// 连接池。默认后端由 `dpi` feature 决定（[`crate::DefaultBackend`]；当前默认是 DPI 直连）。
///
/// ```no_run
/// # async fn demo() -> Result<(), dm_error::DmError> {
/// use dm_core::DmConfig;
/// use dm_core::Backend;
/// use dm_pool::DmPool;
///
/// let cfg = DmConfig::from_connection_string("DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA")?;
/// let pool = DmPool::connect(cfg).await?;
/// let mut conn = pool.get().await?;
/// let alive: bool = conn
///     .with_conn(|raw: &mut dm_pool::DefaultBackend| {
///         // 在阻塞线程上执行，闭包必须是 'static
///         raw.ping()
///     })
///     .await?;
/// assert!(alive);
/// drop(conn);
/// pool.close().await?;
/// pool.wait_closed().await;
/// # Ok(())
/// # }
/// ```
pub struct DmPool<B: Backend = crate::DefaultBackend> {
    inner: Arc<PoolInner<B>>,
}

impl<B: Backend> std::fmt::Debug for DmPool<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DmPool")
            .field("state", &self.inner.state())
            .field("stats", &self.inner.snapshot())
            .finish()
    }
}

impl<B: Backend> DmPool<B> {
    /// 建池：先建一条物理连接（连不上立刻报错，而不是等到第一次 `get`）。
    pub async fn connect(cfg: DmConfig) -> Result<Self, DmError> {
        let exec_cfg = ExecutorConfig::for_pool(cfg.pool.max_size);
        Self::connect_with(cfg, exec_cfg).await
    }

    /// 建池，并显式指定阻塞线程池参数。
    pub async fn connect_with(cfg: DmConfig, exec_cfg: ExecutorConfig) -> Result<Self, DmError> {
        validate_config(&cfg)?;
        let handle = Handle::try_current().map_err(|_| {
            DmError::at(
                DmErrorKind::Config(
                    "DmPool 必须在 tokio 运行时上下文里创建（后台维护与桥接都要用它）".to_owned(),
                ),
                Stage::Config,
            )
        })?;
        let exec = BlockingExecutor::new(exec_cfg)?;
        let borrow = Arc::new(cfg);
        let first = {
            let cfg = Arc::clone(&borrow);
            exec.run(move || B::connect(&cfg)).await??
        };
        // 先取名字：`first` 马上要被 move 进池里。
        let backend_name = first.name();
        let inner = PoolInner::new(borrow, exec, first, handle);
        inner.spawn_maintenance();
        tracing::info!(
            backend = backend_name,
            max_size = inner.cfg.pool.max_size,
            min_idle = inner.cfg.pool.min_idle,
            "连接池已启动"
        );
        Ok(Self { inner })
    }

    /// 借出一条连接。`PoolGuard` drop 时自动归还。
    pub async fn acquire(&self) -> Result<PoolGuard<B>, DmError> {
        self.inner.acquire().await
    }

    /// [`acquire`](Self::acquire) 的别名（`get` 是生态里更常见的名字）。
    pub async fn get(&self) -> Result<PoolGuard<B>, DmError> {
        self.acquire().await
    }

    /// 当前状态。
    pub fn state(&self) -> PoolState {
        self.inner.state()
    }

    /// 指标快照。
    pub fn stats(&self) -> PoolStats {
        self.inner.snapshot()
    }

    /// 池参数（含全部 `PoolConfig`）。
    pub fn config(&self) -> &DmConfig {
        self.inner.config()
    }

    /// 借出句柄是否还能用（池关掉之后借出的句柄仍可把当前操作做完）。
    pub fn is_closed(&self) -> bool {
        self.inner.state() == PoolState::Closed
    }

    /// 关闭池：不再接受借出，把所有空闲连接物理关闭。在飞的连接由调用方用完归还。
    pub async fn close(&self) -> Result<(), DmError> {
        self.inner.close().await;
        self.inner.abort_maintenance();
        Ok(())
    }

    /// 等所有物理连接都消失（配合 [`close`](Self::close) 用）。
    ///
    /// ⚠️ 只要还有借出未还的 `PoolGuard`，这里就会一直等下去。
    pub async fn wait_closed(&self) {
        self.inner.wait_closed().await;
    }
}

impl<B: Backend> Drop for DmPool<B> {
    fn drop(&mut self) {
        // Drop 里不能做阻塞 IO（可能在 tokio 工作线程上），所以只做三件事：
        // 拒新借出、唤醒等待者、停掉后台任务。裸连接随 `ConnInner` 一起析构。
        self.inner.set_state(PoolState::Closing);
        self.inner.permits.close();
        self.inner.abort_maintenance();
    }
}

fn validate_config(cfg: &DmConfig) -> Result<(), DmError> {
    let pool = &cfg.pool;
    if pool.max_size == 0 {
        return Err(DmError::at(
            DmErrorKind::Config("pool.max_size 必须 ≥ 1".to_owned()),
            Stage::Config,
        ));
    }
    if pool.min_idle > pool.max_size {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "pool.min_idle({}) 不能大于 pool.max_size({})",
                pool.min_idle, pool.max_size
            )),
            Stage::Config,
        ));
    }
    Ok(())
}
