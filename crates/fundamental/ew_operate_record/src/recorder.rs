//! 记录器：有界队列 + 后台批量写库。

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use time::OffsetDateTime;

use crate::error::{Error, Result};
use crate::log::{LogEntry, OperationLog, PendingLog};
use crate::query::Query;
use crate::store::Store;

/// 队列默认容量（最大待写条数）。
pub const DEFAULT_QUEUE_CAPACITY: usize = 4096;
/// 单批默认最大条数。
pub const DEFAULT_BATCH_SIZE: usize = 256;

/// 记录器参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// 队列容量：积压超过该值后，新记录被丢弃并计入 [`Stats::dropped`]。
    pub queue_capacity: usize,
    /// 单个事务最多写入多少条（有积压时才合批）。
    pub batch_size: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

impl Options {
    /// 默认参数。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置队列容量。
    #[must_use]
    pub fn queue_capacity(mut self, queue_capacity: usize) -> Self {
        self.queue_capacity = queue_capacity;
        self
    }

    /// 设置单批条数。
    #[must_use]
    pub fn batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }
}

/// 运行统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// 成功入队条数。
    pub accepted: u64,
    /// 成功写库条数。
    pub written: u64,
    /// 写库失败条数（重试一次后仍失败）。
    pub failed: u64,
    /// 因队列满被丢弃条数。
    pub dropped: u64,
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    written: AtomicU64,
    failed: AtomicU64,
    dropped: AtomicU64,
}

enum Command {
    Record(PendingLog),
    Flush(Sender<()>),
}

struct Inner {
    sender: Mutex<Option<SyncSender<Command>>>,
    store: Arc<Store>,
    writer: Mutex<Option<JoinHandle<()>>>,
    join_lock: Mutex<()>,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<Option<String>>>,
    capacity: usize,
    closed: AtomicBool,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // 即使调用方忘了 close，也要把队列里的记录刷完。
        if let Ok(mut guard) = self.sender.lock() {
            guard.take();
        }
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

/// 用户操作记录器。克隆是廉价的，多个线程/模块共享同一个队列。
#[derive(Clone)]
pub struct Recorder {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Recorder")
            .field("stats", &self.stats())
            .finish()
    }
}

impl Recorder {
    /// 用默认参数打开（不存在则创建）数据库。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, Options::default())
    }

    /// 指定参数打开数据库。
    pub fn open_with(path: impl AsRef<Path>, options: Options) -> Result<Self> {
        let store = Arc::new(Store::open(path.as_ref())?);
        Self::start(store, options)
    }

    /// 使用内存数据库（测试、演示；进程退出即丢失）。
    pub fn open_in_memory() -> Result<Self> {
        let store = Arc::new(Store::open_in_memory()?);
        Self::start(store, Options::default())
    }

    fn start(store: Arc<Store>, options: Options) -> Result<Self> {
        if options.queue_capacity == 0 {
            return Err(Error::Invalid("queue_capacity 必须大于 0".to_owned()));
        }
        if options.batch_size == 0 {
            return Err(Error::Invalid("batch_size 必须大于 0".to_owned()));
        }

        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let counters = Arc::new(Counters::default());
        let last_error = Arc::new(Mutex::new(None));

        let writer_store = Arc::clone(&store);
        let writer_counters = Arc::clone(&counters);
        let writer_error = Arc::clone(&last_error);
        let batch_size = options.batch_size;
        let writer = thread::Builder::new()
            .name("ewor-writer".to_owned())
            .spawn(move || {
                writer_loop(
                    receiver,
                    writer_store,
                    batch_size,
                    writer_counters,
                    writer_error,
                )
            })
            .map_err(|error| Error::Invalid(format!("无法启动写入线程: {error}")))?;

        Ok(Self {
            inner: Arc::new(Inner {
                sender: Mutex::new(Some(sender)),
                store,
                writer: Mutex::new(Some(writer)),
                join_lock: Mutex::new(()),
                counters,
                last_error,
                capacity: options.queue_capacity,
                closed: AtomicBool::new(false),
            }),
        })
    }

    /// 记录一条用户操作：只入队，立即返回。
    ///
    /// 记录时间由本方法打点，写入时保证严格递增（微秒精度），因此时间戳可作主键。
    /// 队列满时返回 [`Error::QueueFull`]（本条被丢弃，见 [`Recorder::stats`]）。
    /// **系统自身执行的动作不要调用它。**
    pub fn log(&self, log: OperationLog) -> Result<()> {
        let pending = PendingLog {
            occurred_at: OffsetDateTime::now_utc(),
            log: log.normalize()?,
        };
        let sender = self.sender()?;
        match sender.try_send(Command::Record(pending)) {
            Ok(()) => {
                self.inner.counters.accepted.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                self.inner.counters.dropped.fetch_add(1, Ordering::Relaxed);
                Err(Error::QueueFull {
                    capacity: self.inner.capacity,
                })
            }
            Err(TrySendError::Disconnected(_)) => Err(Error::Closed),
        }
    }

    /// 常见场景的简写：只给操作人和动作。
    pub fn log_user(&self, user_id: impl Into<String>, action: impl Into<String>) -> Result<()> {
        self.log(OperationLog::new(user_id, action))
    }

    /// 等待此前所有入队记录写库。
    ///
    /// 会阻塞调用线程；只在关键节点或退出前使用。
    pub fn flush(&self) -> Result<()> {
        let sender = self.sender()?;
        let (ack, receiver) = mpsc::channel();
        sender
            .send(Command::Flush(ack))
            .map_err(|_| Error::Closed)?;
        receiver.recv().map_err(|_| Error::Closed)
    }

    /// 关闭记录器：排空队列、刷盘、停止写入线程。可重复调用。
    pub fn close(&self) -> Result<()> {
        let _guard = self
            .inner
            .join_lock
            .lock()
            .map_err(|_| Error::Invalid("记录器内部锁已损坏".to_owned()))?;
        if self.inner.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.inner.closed.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.inner.sender.lock() {
            guard.take();
        }
        if let Ok(mut guard) = self.inner.writer.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
        Ok(())
    }

    /// 条件查询，最新在前。
    pub fn query(&self, query: &Query) -> Result<Vec<LogEntry>> {
        self.inner.store.query(query)
    }

    /// 条件计数。
    pub fn count(&self, query: &Query) -> Result<u64> {
        self.inner.store.count(query)
    }

    /// 删除发生时间早于 `cutoff` 的记录，返回删除条数。
    ///
    /// 建议启动时调用一次，例如 `prune_before(now - 90 天)`。
    pub fn prune_before(&self, cutoff: OffsetDateTime) -> Result<u64> {
        self.inner.store.prune_before(cutoff)
    }

    /// 把 WAL 内容并回主库并截断 WAL。
    pub fn checkpoint(&self) -> Result<()> {
        self.inner.store.checkpoint()
    }

    /// 运行统计。
    pub fn stats(&self) -> Stats {
        let counters = &self.inner.counters;
        Stats {
            accepted: counters.accepted.load(Ordering::Relaxed),
            written: counters.written.load(Ordering::Relaxed),
            failed: counters.failed.load(Ordering::Relaxed),
            dropped: counters.dropped.load(Ordering::Relaxed),
        }
    }

    /// 最近一次写库失败的原因（成功写入不会清除它）。
    pub fn last_error(&self) -> Option<String> {
        self.inner
            .last_error
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    fn sender(&self) -> Result<SyncSender<Command>> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::Closed);
        }
        self.inner
            .sender
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .ok_or(Error::Closed)
    }
}

fn writer_loop(
    receiver: Receiver<Command>,
    store: Arc<Store>,
    batch_size: usize,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<Option<String>>>,
) {
    let mut records: Vec<PendingLog> = Vec::with_capacity(batch_size);
    let mut flushes: Vec<Sender<()>> = Vec::new();

    loop {
        match receiver.recv() {
            Ok(command) => push(command, &mut records, &mut flushes),
            // 发送端已全部丢弃，且队列已排空。
            Err(_) => break,
        }
        // 有积压才合批；排空立即提交。
        while records.len() < batch_size {
            match receiver.try_recv() {
                Ok(command) => push(command, &mut records, &mut flushes),
                Err(_) => break,
            }
        }

        if !records.is_empty() {
            let count = u64::try_from(records.len()).unwrap_or(u64::MAX);
            match persist(&store, &records) {
                Ok(()) => {
                    counters.written.fetch_add(count, Ordering::Relaxed);
                }
                Err(error) => {
                    counters.failed.fetch_add(count, Ordering::Relaxed);
                    if let Ok(mut guard) = last_error.lock() {
                        *guard = Some(error.to_string());
                    }
                }
            }
            records.clear();
        }
        for ack in flushes.drain(..) {
            let _ = ack.send(());
        }
    }
}

fn push(command: Command, records: &mut Vec<PendingLog>, flushes: &mut Vec<Sender<()>>) {
    match command {
        Command::Record(pending) => records.push(pending),
        Command::Flush(ack) => flushes.push(ack),
    }
}

fn persist(store: &Store, records: &[PendingLog]) -> Result<()> {
    match store.insert_batch(records) {
        Ok(()) => Ok(()),
        // 瞬时 BUSY（例如清理任务占着写锁）重试一次。
        Err(_) => store.insert_batch(records),
    }
}
