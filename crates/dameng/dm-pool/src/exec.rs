//! T4.1 —— 异步 ↔ 阻塞桥接：**专用阻塞线程池**（设计方案 §5.4）。
//!
//! # 为什么不用 `tokio::task::spawn_blocking`
//!
//! 两个致命问题：
//!
//! 1. **阻塞线程数不可控**：`spawn_blocking` 的线程上限（默认 512）与池的 `max_size`
//!    不是一个旋钮。§5.4 要求「池 `max_size` ≤ 可用阻塞线程数」，否则会出现
//!    「借到了连接却排不到阻塞线程」的假死——用自建线程池，这条约束**由构造保证**
//!    （[`BlockingExecutor::new`] 直接按 `max_size` 开线程）。
//! 2. **任务无法取消，但 `abort` 对已开始的任务无效**，且我们无法在任务开始前后
//!    观察到「调用方已经不等了」。本类型用 [`JobReply`] 把这个观察点交给任务自己
//!    （§5.4 第 2 步的 `raw.finish_op(ok)` 需要在**返回值已经没人要**时把连接判死）。
//!
//! # 桥接次数必须是 O(1) 或 O(批)（§5.9 硬纪律 1）
//!
//! 本类型只提供「一次提交、一次回传」。**绝不能**把每一行取值都做成一次 [`BlockingExecutor::run`]：
//! 一次桥接 ~1–10 µs，1000 行就是 1–10 ms 纯开销。一次 `query` 的全部 ODBC 调用必须发生在
//! **同一个**提交内；流式取值每次一整批。
//!
//! # 背压
//!
//! 队列**有界**（[`ExecutorConfig::queue_capacity`]）。队列满时 [`BlockingExecutor::submit`]
//! 等待空位（异步等待，**不阻塞 tokio worker**）。容量为 [`usize::MAX`] 时视为无界
//! （仅测试用；生产必须给有限值，理由：无界队列会把「服务端变慢」翻译成「进程 OOM」）。

use std::collections::VecDeque;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
use tokio::sync::{Semaphore, oneshot};

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// 阻塞线程池的构造参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorConfig {
    /// 工作线程数。**必须 ≥ 池的 `max_size`**，否则连接借得出来却排不到线程。
    pub threads: usize,
    /// 队列容量（有界，背压用）。[`usize::MAX`] = 无界（仅测试）。
    pub queue_capacity: usize,
    /// 线程名前缀，便于在 profiler / 调试器里认出是谁在阻塞。
    pub thread_name: String,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            queue_capacity: 64,
            thread_name: "dm-pool-blocking".to_owned(),
        }
    }
}

impl ExecutorConfig {
    /// 按池的 `max_size` 推导：线程数 = `max_size`，队列 = `4 × max_size`（下限 8）。
    ///
    /// 队列取一个小的倍数：队列只是**抖动缓冲**，真正的并发上限是线程数。
    /// 队列配得过大，等于把「服务端变慢」藏起来，只在内存里堆任务。
    #[must_use]
    pub fn for_pool(max_size: usize) -> Self {
        Self {
            threads: max_size.max(1),
            queue_capacity: max_size.saturating_mul(4).max(8),
            thread_name: "dm-pool-blocking".to_owned(),
        }
    }

    /// 线程名带序号（`name-0`、`name-1`…）。
    fn thread_name_at(&self, index: usize) -> String {
        format!("{}-{index}", self.thread_name)
    }
}

// ---------------------------------------------------------------------------
// 任务回传口
// ---------------------------------------------------------------------------

/// 任务向调用方回传结果的唯一通道（§5.4 第 2 步的 `tx`）。
///
/// 两个用途，缺一不可：
///
/// 1. [`send`](JobReply::send) —— 把结果交给等待中的 future；
/// 2. [`is_cancelled`](JobReply::is_cancelled) —— **调用方已经不等了**（超时、
///    `select!`、上层取消把它 drop 了）。这是本层唯一能观察到「取消」的位置：
///    `tokio` 不会告诉我们 future 被 drop 了，但 receiver 被 drop 会让 sender 变成
///    closed。任务据此把连接的 `ok` 标志判为 `false`（§5.4：`ok = r.is_ok() && !raw.cursor_open()`，
///    取消时还要再 `&& !reply.is_cancelled()`），连接于是进 `Dirty`/`Poisoned` 而被销毁
///    —— 这正是 D11「取消后不复用」的落点。
///
/// 任务**不**调用 `send` 也没关系：`Drop` 会丢掉通道，wait 侧拿到 `Err(RecvError)`，
/// 由 [`BlockingExecutor::run`] 翻译成「任务没有回传结果」。
#[must_use = "不回传结果等于让调用方永远等不到值"]
pub struct JobReply<R> {
    tx: Option<oneshot::Sender<R>>,
}

impl<R> JobReply<R> {
    /// 调用方是否已经放弃等待。
    ///
    /// 注意这是一个**瞬时**判断：`true` 之后不会再变回 `false`，但 `false` 不保证
    /// 后续 `send` 一定成功（竞态窗口内调用方仍可能 drop）。所以它只用于**悲观决策**
    /// （把连接判死），不能用于乐观决策（「还没取消，那我就复用这条连接」）。
    pub fn is_cancelled(&self) -> bool {
        self.tx.as_ref().is_none_or(oneshot::Sender::is_closed)
    }

    /// 回传结果。返回 `false` 表示调用方已经不在了（结果被丢弃）。
    pub fn send(mut self, value: R) -> bool {
        self.tx.take().is_some_and(|tx| tx.send(value).is_ok())
    }
}

impl<R> fmt::Debug for JobReply<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobReply")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// 队列条目
// ---------------------------------------------------------------------------

/// 队列里的一个任务。
///
/// 持有 permit 直到**出队**才释放：这样 [`Semaphore`] 约束的是**队列深度**，
/// 而不是「排队 + 在跑」的总数。在跑的任务数天然被工作线程数限制，不需要再管。
struct JobEntry {
    job: Box<dyn FnOnce() + Send + 'static>,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl fmt::Debug for JobEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobEntry").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// 内部状态
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ExecutorInner {
    /// 有界队列。**只在极短临界区内持有**，绝不在其中做任何 IO。
    queue: Mutex<VecDeque<JobEntry>>,
    /// 队列非空 / 关闭时唤醒工作线程。
    cond: Condvar,
    /// 队列空位。容量 `usize::MAX` 时为 `None`（无界）。
    slots: Option<Arc<Semaphore>>,
    capacity: usize,
    shutting_down: AtomicBool,
    // ---- 指标 ----
    threads: usize,
    submitted: AtomicU64,
    completed: AtomicU64,
    panicked: AtomicU64,
    queue_depth: AtomicUsize,
    max_queue_depth: AtomicUsize,
    active: AtomicUsize,
    busy_peak: AtomicUsize,
}

impl ExecutorInner {
    /// 取队列锁，**容忍中毒**：一个 panic 的任务不该让整个池再也借不出连接。
    fn lock_queue(&self) -> MutexGuard<'_, VecDeque<JobEntry>> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pop(&self) -> Option<JobEntry> {
        let mut queue = self.lock_queue();
        loop {
            if let Some(entry) = queue.pop_front() {
                self.queue_depth.fetch_sub(1, Ordering::Relaxed);
                return Some(entry);
            }
            if self.shutting_down.load(Ordering::Acquire) {
                return None;
            }
            queue = self.cond.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// 工作线程主体。**必须在 `catch_unwind` 里跑任务**：ODBC 驱动的 FFI 边界一旦
    /// 抛出 panic，不隔离就会带走整个线程，池的并发上限静默下降。
    fn worker_loop(self: &Arc<Self>) {
        while let Some(entry) = self.pop() {
            // ★ permit 在**出队时**就释放——这正是 [`JobEntry`] 文档承诺的语义：
            // `slots` 约束的是**队列深度**，不是「排队 + 在跑」的总数（在跑的数天然
            // 被线程数限制）。曾经写成 `let job = entry.job;`，让 `entry` 在整轮结束
            // 才 drop：容量 1 + 线程 1 时占位任务会把唯一的空位攥到跑完，
            // 后续提交全部卡死（T4.1 单测 `full_queue_awaits_…` 实测踩到）。
            let JobEntry { job, _permit } = entry;
            drop(_permit);
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.busy_peak.fetch_max(active, Ordering::AcqRel);
            let panicked = catch_unwind(AssertUnwindSafe(job)).is_err();
            self.active.fetch_sub(1, Ordering::AcqRel);
            self.completed.fetch_add(1, Ordering::Relaxed);
            if panicked {
                self.panicked.fetch_add(1, Ordering::Relaxed);
                // 不回传结果：调用方拿到 `RecvError`，由 `run` 翻译成
                // 「连接状态未知」的错误（见 `run` 的文档），池据此丢弃连接。
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 线程池
// ---------------------------------------------------------------------------

/// 专用阻塞线程池。由 `dm-pool` 的池持有一个，池的每条连接恰好占一个线程工作时。
pub struct BlockingExecutor {
    inner: Arc<ExecutorInner>,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl fmt::Debug for BlockingExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockingExecutor")
            .field("threads", &self.inner.threads)
            .field("queue_capacity", &self.inner.capacity)
            .field("queue_depth", &self.queue_depth())
            .field("active", &self.active_tasks())
            .field("shutting_down", &self.is_shutting_down())
            .finish()
    }
}

impl BlockingExecutor {
    /// 按配置开线程。
    ///
    /// `threads = 0` 会被拒绝：那意味着池永远借不到连接（而不是「无限并发」）。
    pub fn new(cfg: ExecutorConfig) -> Result<Self, DmError> {
        if cfg.threads == 0 {
            return Err(DmError::at(
                DmErrorKind::Config("阻塞线程池的 threads 不能为 0：池将永远借不出连接".to_owned()),
                Stage::Connect,
            ));
        }
        let capacity = cfg.queue_capacity;
        let inner = Arc::new(ExecutorInner {
            queue: Mutex::new(VecDeque::new()),
            cond: Condvar::new(),
            slots: (capacity != usize::MAX).then(|| Arc::new(Semaphore::new(capacity))),
            capacity,
            shutting_down: AtomicBool::new(false),
            threads: cfg.threads,
            submitted: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            panicked: AtomicU64::new(0),
            queue_depth: AtomicUsize::new(0),
            max_queue_depth: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            busy_peak: AtomicUsize::new(0),
        });

        let mut handles = Vec::with_capacity(cfg.threads);
        for index in 0..cfg.threads {
            let worker = Arc::clone(&inner);
            let handle = thread::Builder::new()
                .name(cfg.thread_name_at(index))
                .spawn(move || worker.worker_loop())
                .map_err(|e| {
                    // 开线程失败：把已经开起来的线程收掉，避免半成品池泄漏线程。
                    inner.shutting_down.store(true, Ordering::Release);
                    inner.cond.notify_all();
                    DmError::at(
                        DmErrorKind::Config(format!(
                            "创建阻塞工作线程 {index} 失败（已开 {} 个）：{e}",
                            index
                        )),
                        Stage::Connect,
                    )
                })?;
            handles.push(handle);
        }

        Ok(Self {
            inner,
            handles: Mutex::new(handles),
        })
    }

    /// 按池的 `max_size` 推导（线程数 = `max_size`）。
    pub fn for_pool(max_size: usize) -> Result<Self, DmError> {
        Self::new(ExecutorConfig::for_pool(max_size))
    }

    // ---- 提交 ----

    /// 提交一个任务，返回接收端；任务通过 [`JobReply`] 回传结果。
    ///
    /// 队列满时**异步等待**空位——这是背压，不是错误。
    ///
    /// 闭包**只**通过 [`JobReply::send`] 回传（返回值固定为 `()`）：
    /// 这样「有没有回传」本身就是一个信号——没回传（panic、或任务提前 `return`）
    /// 会让等待方拿到 `Err(RecvError)`，见 [`BlockingExecutor::run`]。
    pub async fn submit<R, F>(&self, f: F) -> Result<oneshot::Receiver<R>, DmError>
    where
        R: Send + 'static,
        F: FnOnce(JobReply<R>) + Send + 'static,
    {
        // 先取空位（await），再碰锁：绝不在持有 std 锁时 await（§5.8）。
        let permit = match &self.inner.slots {
            Some(slots) => Some(
                Arc::clone(slots)
                    .acquire_owned()
                    .await
                    .map_err(|_| self.closed_error())?,
            ),
            None => None,
        };

        let (tx, rx) = oneshot::channel();
        let job: Box<dyn FnOnce() + Send + 'static> =
            Box::new(move || f(JobReply { tx: Some(tx) }));

        // 入队与「是否已关闭」的判断必须在**同一个**临界区里，否则会丢任务：
        // 任务可能落在「已置关闭标志、但工作线程已经退出」的队列里永远没人跑。
        {
            let mut queue = self.inner.lock_queue();
            if self.inner.shutting_down.load(Ordering::Acquire) {
                return Err(self.closed_error());
            }
            queue.push_back(JobEntry {
                job,
                _permit: permit,
            });
            let depth = self.inner.queue_depth.fetch_add(1, Ordering::AcqRel) + 1;
            self.inner
                .max_queue_depth
                .fetch_max(depth, Ordering::AcqRel);
        }
        self.inner.submitted.fetch_add(1, Ordering::Relaxed);
        self.inner.cond.notify_one();
        Ok(rx)
    }

    /// 提交并等待——**桥接的原子单位**。
    ///
    /// 错误翻译：
    ///
    /// | 情形 | 返回 |
    /// |---|---|
    /// | 线程池已关闭 | `Config("阻塞线程池已关闭…")` |
    /// | 任务 panic / 没回传结果 | `Io` + 一条诊断（见下） |
    ///
    /// 任务 panic 映射成 [`DmErrorKind::Io`] 是**有意**的：`Io.is_connection_lost() == true`，
    /// 于是「§6.8 与连接池的硬契约」自动给出正确行为——**丢弃连接**。panic 之后连接内部
    /// 状态未知，复用它是本项目最不能接受的错误类型（静默失步）。诊断里写清了真实原因，
    /// 不靠 `Io` 这个名字误导排查。
    pub async fn run<R, F>(&self, f: F) -> Result<R, DmError>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let rx = self
            .submit(move |reply| {
                let _ = reply.send(f());
            })
            .await?;
        match rx.await {
            Ok(value) => Ok(value),
            // 没有回传结果有两种可能，必须分清：
            // - 池正在关闭（任务被丢弃、根本没跑）→ 关闭错误；
            // - 任务 panic（或自己忘了回传）→ 连接状态未知。
            Err(_) if self.is_shutting_down() => Err(self.closed_error()),
            Err(_) => Err(no_reply_error()),
        }
    }

    fn closed_error(&self) -> DmError {
        DmError::at(
            DmErrorKind::Config("阻塞线程池已关闭，不再接受新任务".to_owned()),
            Stage::Connect,
        )
    }

    // ---- 指标 ----

    /// 工作线程数。
    pub fn threads(&self) -> usize {
        self.inner.threads
    }

    /// 队列容量（`usize::MAX` = 无界）。
    pub fn queue_capacity(&self) -> usize {
        self.inner.capacity
    }

    /// 当前排队深度（不含正在执行的任务）。
    pub fn queue_depth(&self) -> usize {
        self.inner.queue_depth.load(Ordering::Acquire)
    }

    /// 队列深度历史峰值。**持续贴着容量**说明线程数不够（池的并发上限被桥接卡住了）。
    pub fn max_queue_depth(&self) -> usize {
        self.inner.max_queue_depth.load(Ordering::Acquire)
    }

    /// 正在执行的任务数（≤ [`threads`](Self::threads)）。
    pub fn active_tasks(&self) -> usize {
        self.inner.active.load(Ordering::Acquire)
    }

    /// 同时忙碌过的线程数峰值。
    pub fn busy_peak(&self) -> usize {
        self.inner.busy_peak.load(Ordering::Acquire)
    }

    /// 累计提交数。
    pub fn submitted(&self) -> u64 {
        self.inner.submitted.load(Ordering::Relaxed)
    }

    /// 累计完成数（含 panic 的）。
    pub fn completed(&self) -> u64 {
        self.inner.completed.load(Ordering::Relaxed)
    }

    /// 累计 panic 数。
    pub fn panicked(&self) -> u64 {
        self.inner.panicked.load(Ordering::Relaxed)
    }

    /// 是否已经进入关闭流程。
    pub fn is_shutting_down(&self) -> bool {
        self.inner.shutting_down.load(Ordering::Acquire)
    }

    // ---- 关闭 ----

    /// 丢弃**尚未出队**的任务（它们的调用方立刻拿到「池已关闭」错误）。
    ///
    /// 正在执行的任务不受影响——执行中的阻塞调用**没有办法安全中止**
    /// （ODBC 的 `SQLCancel` 只保证「发出了取消请求」，见 §5.4 / D11）。
    pub fn drain_pending(&self) -> usize {
        let mut queue = self.inner.lock_queue();
        let dropped = queue.len();
        queue.clear();
        self.inner.queue_depth.store(0, Ordering::Release);
        dropped
    }

    /// 置关闭标志并唤醒全部工作线程。
    ///
    /// **已经在队列里的任务仍会被执行完**（优雅关闭）；此后的 [`submit`](Self::submit)
    /// 一律失败。等待空位中的调用方也会被唤醒并拿到错误（[`Semaphore::close`]）。
    pub fn shutdown(&self) {
        {
            let _queue = self.inner.lock_queue();
            if self.inner.shutting_down.swap(true, Ordering::AcqRel) {
                return; // 幂等
            }
            if let Some(slots) = &self.inner.slots {
                slots.close();
            }
        }
        self.inner.cond.notify_all();
    }

    /// 等所有工作线程退出。**只在明确的关闭路径上调用**；`Drop` 刻意不这么做（见下）。
    pub fn join(&self) {
        let handles = std::mem::take(&mut *self.handles.lock().unwrap_or_else(|e| e.into_inner()));
        for handle in handles {
            // 工作线程不会 panic（任务在 catch_unwind 里），所以这里忽略结果即可；
            // 真要有人在 worker_loop 里 panic，那也是 `join` 该吞掉的收尾噪音。
            let _ = handle.join();
        }
    }

    /// 关闭并等待（= [`shutdown`](Self::shutdown) + [`join`](Self::join)）。
    ///
    /// ⚠️ 若有阻塞任务卡在 ODBC 调用里（驱动不返回），这里会**等到它返回为止**。
    /// 把它放在明确的关闭路径上（`dm_pool::DmPool::wait_closed`），不要放在 `Drop` 里。
    pub fn shutdown_and_join(&self) {
        self.shutdown();
        self.join();
    }
}

impl Drop for BlockingExecutor {
    /// 关闭信号 + 丢弃未出队的任务，**但不 join 工作线程**。
    ///
    /// # 为什么 `Drop` 不能 join（真机调试教训）
    ///
    /// 一个卡在 ODBC 调用里的阻塞任务**可能永远不返回**——达梦驱动在服务端无响应时就是这样。
    /// 早期版本在这里 `join()`，于是「池被丢弃 / future 被取消 / 用例 panic 后析构」
    /// 会**永久挂住析构线程**（实测：整个测试进程 CPU 归零、再也不动）。
    /// 正确做法是把「等所有任务做完」变成一个**显式的、调用方可控的**动作
    /// （[`shutdown_and_join`](Self::shutdown_and_join)，`dm-pool` 由 `wait_closed` 触发）。
    ///
    /// 线程被 detach 掉不会泄漏任何东西：进程退出时内核回收；而「卡住的任务」
    /// 本来就无法安全回收（它的连接已经在池的账上被标记销毁）。
    fn drop(&mut self) {
        self.shutdown();
        self.drain_pending();
        // detach：JoinHandle 直接丢掉，不 join。
        self.handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

/// 「任务没有回传结果」——panic 或任务自己忘了 `send`。
fn no_reply_error() -> DmError {
    let mut err = DmError::at(DmErrorKind::Io, Stage::Execute);
    err.push_diag(DiagRecord::native(
        0,
        "阻塞任务没有回传结果（多半是 panic）：连接内部状态未知，按连接丢失处理并丢弃该连接",
    ));
    err
}

// ---------------------------------------------------------------------------
// 单测
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn exec(threads: usize, cap: usize) -> BlockingExecutor {
        BlockingExecutor::new(ExecutorConfig {
            threads,
            queue_capacity: cap,
            thread_name: "dm-test".to_owned(),
        })
        .expect("建池")
    }

    /// 等到条件成立。**不要**用「spawn 完就断言」来建立时序：`tokio::spawn` 的调度顺序
    /// 不保证，那会让用例在真机上偶发失败（第一版就踩了：holder 还没开始跑，
    /// 队列深度就已经被另一个任务用掉）。
    async fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
        for _ in 0..2000 {
            if pred() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("等待「{what}」超时");
    }

    #[tokio::test]
    async fn run_returns_value_and_updates_metrics() {
        let ex = exec(2, 8);
        for i in 0..10 {
            assert_eq!(ex.run(move || i * 2).await.unwrap(), i * 2);
        }
        assert_eq!(ex.submitted(), 10);
        // 调用方拿到的回复是在任务体里 `reply.send(..)` 发出的，而 `completed` 由工作线程
        // 在任务**返回之后**才累加（这样「完成」才不包含 panic 的那一半）。所以这里必须等，
        // 不能直接断言——直接断言是在赌调度顺序（曾经偶发红过）。
        wait_until("10 个任务都记进 completed", || ex.completed() == 10).await;
        assert_eq!(ex.completed(), 10);
        assert_eq!(ex.panicked(), 0);
        assert_eq!(ex.queue_depth(), 0);
        assert_eq!(ex.active_tasks(), 0);
        ex.shutdown_and_join();
    }

    #[tokio::test]
    async fn zero_threads_is_rejected() {
        let err = BlockingExecutor::new(ExecutorConfig {
            threads: 0,
            ..ExecutorConfig::default()
        })
        .unwrap_err();
        assert!(
            matches!(err.kind, DmErrorKind::Config(ref m) if m.contains("threads")),
            "got {:?}",
            err.kind
        );
    }

    /// 一个线程 + 队列容量 2：并发提交 12 个任务，队列深度不得越过容量。
    #[tokio::test]
    async fn queue_depth_never_exceeds_capacity() {
        let ex = Arc::new(exec(1, 2));
        let tasks: Vec<_> = (0..12)
            .map(|i| {
                let ex = Arc::clone(&ex);
                tokio::spawn(async move { ex.run(move || i).await })
            })
            .collect();

        // 边跑边采样：只要有一次超过容量，背压就是坏的。
        for _ in 0..40 {
            assert!(
                ex.queue_depth() <= ex.queue_capacity(),
                "队列深度 {} 越过容量 {}",
                ex.queue_depth(),
                ex.queue_capacity()
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        let mut got = Vec::new();
        for task in tasks {
            got.push(task.await.unwrap().unwrap());
        }
        got.sort_unstable();
        assert_eq!(got, (0..12).collect::<Vec<_>>());
        assert!(ex.max_queue_depth() <= 2, "峰值 {}", ex.max_queue_depth());
        ex.shutdown_and_join();
    }

    /// 任务 panic：调用方拿到 `Io`，**工作线程活着**，后续任务照常。
    #[tokio::test]
    async fn panicking_task_is_isolated() {
        let ex = exec(1, 8);
        let err = ex
            .run(|| -> u32 { panic!("模拟 ODBC 层 panic") })
            .await
            .unwrap_err();
        assert!(matches!(err.kind, DmErrorKind::Io));
        assert!(
            err.is_connection_lost(),
            "panic 必须被判定为「连接不可复用」"
        );
        assert!(err.diags().iter().any(|d| d.message.contains("panic")));

        // 线程还活着：同一个池继续能干活。
        assert_eq!(ex.run(|| 7).await.unwrap(), 7);
        // 同上：计数由工作线程在任务返回之后累加，而回复是任务体里发的 ⇒ 要等，不能赌。
        wait_until("panic 计数与完成计数都到位", || {
            ex.panicked() == 1 && ex.completed() == 2
        })
        .await;
        assert_eq!(ex.panicked(), 1);
        assert_eq!(ex.completed(), 2);
        ex.shutdown_and_join();
    }

    /// 调用方放弃等待：任务能观察到 `is_cancelled()`（D11 的唯一观察点）。
    #[tokio::test]
    async fn cancelled_receiver_is_visible_to_job() {
        let ex = Arc::new(exec(1, 8));
        let started = Arc::new(tokio::sync::Notify::new());
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();

        let rx = {
            let started = Arc::clone(&started);
            ex.submit(move |reply| {
                started.notify_one();
                // 等调用方把 receiver 丢掉。
                for _ in 0..200 {
                    if reply.is_cancelled() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                let observed = reply.is_cancelled();
                let _ = cancelled_tx.send(observed);
                let _ = reply.send(u32::from(observed));
            })
            .await
            .unwrap()
        };

        started.notified().await;
        drop(rx); // ★ 取消：这正是 `timeout` / `select!` / 上层 drop 做的事
        let observed = cancelled_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(observed, "任务没有观察到调用方已放弃");
        ex.shutdown_and_join();
    }

    /// 关闭后：已入队的任务照常跑完，新提交被拒绝。
    #[tokio::test]
    async fn shutdown_drains_queue_then_rejects() {
        let ex = Arc::new(exec(1, 8));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let done = Arc::new(AtomicUsize::new(0));

        // 一个占住线程的长任务。**直接 submit 入队、不经过 tokio::spawn**：
        // 这样它必定是第一个任务，后面的排队数才可预期——`tokio::spawn` 的调度顺序
        // 没有任何保证（第一版就是因为这个才偶发失败）。
        let blocking = {
            let gate = Arc::clone(&gate);
            let done = Arc::clone(&done);
            ex.submit(move |reply| {
                let (lock, cond) = &*gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = cond.wait(open).unwrap();
                }
                done.fetch_add(1, Ordering::SeqCst);
                let _ = reply.send(());
            })
            .await
            .unwrap()
        };
        // 再排两个：它们必须在关闭后仍然被执行。
        let queued: Vec<_> = (0..2)
            .map(|_| {
                let ex = Arc::clone(&ex);
                let done = Arc::clone(&done);
                tokio::spawn(
                    async move { ex.run(move || done.fetch_add(1, Ordering::SeqCst)).await },
                )
            })
            .collect();

        // 先确认占位任务真的在跑，再看排队数——顺序不能靠 spawn 的调度顺序假设。
        wait_until("占位任务开始执行", || ex.active_tasks() == 1).await;
        wait_until("排队数到 2", || ex.queue_depth() == 2).await;

        ex.shutdown();
        let err = ex.run(|| 1u32).await.unwrap_err();
        assert!(
            matches!(err.kind, DmErrorKind::Config(ref m) if m.contains("已关闭")),
            "got {:?}",
            err.kind
        );

        {
            let (lock, cond) = &*gate;
            *lock.lock().unwrap() = true;
            cond.notify_all();
        }
        blocking.await.unwrap();
        for task in queued {
            task.await.unwrap().unwrap();
        }
        assert_eq!(done.load(Ordering::SeqCst), 3, "关闭时丢掉了已入队的任务");
        ex.join();
    }

    /// 队列满时 [`BlockingExecutor::submit`] 是**异步等待**，不是阻塞 tokio worker。
    #[tokio::test(flavor = "current_thread")]
    async fn full_queue_awaits_without_blocking_the_runtime() {
        let ex = Arc::new(exec(1, 1));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));

        let holder = {
            let gate = Arc::clone(&gate);
            ex.submit(move |reply| {
                let (lock, cond) = &*gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = cond.wait(open).unwrap();
                }
                let _ = reply.send(());
            })
            .await
            .unwrap()
        };
        wait_until("占位任务开始执行", || ex.active_tasks() == 1).await;
        // 占位任务 + 1 个排队 = 队列满。
        let queued = {
            let ex = Arc::clone(&ex);
            tokio::spawn(async move { ex.run(|| ()).await })
        };
        wait_until("排队数到 1", || ex.queue_depth() == 1).await;

        // 第三个提交必须挂住；同时 runtime 还能跑别的任务（不阻塞 worker）。
        let pending = {
            let ex = Arc::clone(&ex);
            tokio::spawn(async move { ex.run(|| ()).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!pending.is_finished(), "队列满时应当背压等待");
        assert!(
            ex.queue_depth() <= 1,
            "背压期间队列深度涨到了 {}",
            ex.queue_depth()
        );

        {
            let (lock, cond) = &*gate;
            *lock.lock().unwrap() = true;
            cond.notify_all();
        }
        holder.await.unwrap();
        queued.await.unwrap().unwrap();
        pending.await.unwrap().unwrap();
        ex.shutdown_and_join();
    }

    #[test]
    fn for_pool_scales_queue_with_max_size() {
        let cfg = ExecutorConfig::for_pool(10);
        assert_eq!(
            cfg.threads, 10,
            "线程数必须 ≥ max_size，否则借到连接却排不到线程"
        );
        assert_eq!(cfg.queue_capacity, 40);
        // max_size = 0 是非法配置，但推导函数不该 panic（validate 会先报错）。
        assert_eq!(ExecutorConfig::for_pool(0).threads, 1);
    }
}
