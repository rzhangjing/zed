//! 带 keep-alive 语义的阻塞式工作线程池。
//!
//! 照搬 `dynamic_globe.rs:2143-2285`（`download_worker`）：
//! - 16 个线程（L48：`DOWNLOAD_THREADS = 16`）
//! - 通过 `Arc<Mutex<mpsc::Receiver>>` 共享的任务队列
//! - 带 keep-alive 连接池的 ureq agent（L2148-2151）
//! - 3 次重试，采用 `250ms << attempt` 指数退避（L2186-2191）
//! - 抓取前的 wanted 集门控（L2161）
//! - 三态结果：aborted / failed / placeholder / success

use std::collections::HashSet;
use std::hash::Hash;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::net::{FetchResult, NetworkBackend};

/// 提交给工作线程池的一个任务。
#[derive(Debug, Clone)]
pub struct Job<K> {
    /// 标识此任务的瓦片键。
    pub key: K,
    /// 要抓取的 URL。
    pub url: String,
    /// 优先级（队列很深时，值越高 = 越早被抓取）。
    pub priority: f64,
}

/// 已完成的工作线程任务的结果。
#[derive(Debug)]
pub struct JobResult<K, Payload> {
    /// 瓦片键。
    pub key: K,
    /// 抓取的结果。
    pub outcome: JobOutcome<Payload>,
}

/// 与 `dynamic_globe.rs:1052-1098` 对应的三态结果，外加成功态。
#[derive(Debug)]
pub enum JobOutcome<Payload> {
    /// 抓取成功并带有解码后的载荷。
    Success(Payload),
    /// 瓦片在传输途中离开了 wanted 集（L1052-1060）。
    Aborted,
    /// 所有重试耗尽——瞬时失败（L1062-1072）。
    Failed,
    /// 无可用的影像——检测到占位符（L1074-1098）。
    Placeholder,
}

/// 工作线程池的配置。
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 工作线程数量（L48：16）。
    pub threads: usize,
    /// 每个任务的最大重试次数（L2186：3）。
    pub max_attempts: u32,
    /// 退避基准时长（L2189：250 ms）。实际值 = base << attempt。
    pub backoff_base: Duration,
}

impl Default for PoolConfig {
    /// 默认池配置：16 工作线程、3 次重试、250 ms 退避基准（与上游经验值对齐）。
    fn default() -> Self {
        Self {
            threads: 16,
            max_attempts: 3,
            backoff_base: Duration::from_millis(250),
        }
    }
}

/// 类型擦除的解码函数：原始字节 → 可选的 Payload。
/// 对占位符瓦片返回 `None`（L2211：`is_placeholder_tile`）。
pub type Decoder<Payload> =
    Arc<dyn Fn(&[u8]) -> Option<Payload> + Send + Sync + 'static>;

/// 用于任务结果的通道对（规避 clippy::type_complexity）。
type ResultChannel<K, Payload> = (
    mpsc::Sender<JobResult<K, Payload>>,
    mpsc::Receiver<JobResult<K, Payload>>,
);

/// 在 N 个线程上处理抓取任务的阻塞式工作线程池。
///
/// 对应 `dynamic_globe.rs` 中创建 `DOWNLOAD_THREADS`（16）个
/// worker、每个都运行 `download_worker` 的线程 spawn 循环。
pub struct WorkerPool<K, Payload>
where
    K: Hash + Eq + Copy + Send + 'static,
    Payload: Send + 'static,
{
    /// 向 worker 派发任务的发送端（多 worker 共享，故用 `mpsc`）。
    job_tx: mpsc::Sender<Job<K>>,
    /// 回收已完成结果的接收端；用 `Arc<Mutex<..>>` 包裹以供多 worker 发送。
    result_rx: Arc<Mutex<mpsc::Receiver<JobResult<K, Payload>>>>,
    /// 当前“仍然想要”的瓦片键集，worker 在抓取前据此门控取消。
    wanted: Arc<Mutex<HashSet<K>>>,
    /// 各 worker 线程句柄（仅需保活；`shutdown` 依赖通道关闭使其自然退出）。
    _handles: Vec<thread::JoinHandle<()>>,
}

impl<K, Payload> WorkerPool<K, Payload>
where
    K: Hash + Eq + Copy + Send + 'static,
    Payload: Send + 'static,
{
    /// 以给定配置 spawn 一个工作线程池。
    ///
    /// `backend`：网络实现（在所有 worker 间共享）。
    /// `decode`：将原始字节 → Payload 的闭包（None = 占位符）。
    /// `config`：线程数、重试策略。
    pub fn spawn(
        backend: Arc<dyn NetworkBackend>,
        decode: Decoder<Payload>,
        config: PoolConfig,
    ) -> Self {
        let (job_tx, job_rx): (mpsc::Sender<Job<K>>, mpsc::Receiver<Job<K>>) = mpsc::channel();
        let (result_tx, result_rx): ResultChannel<K, Payload> = mpsc::channel();

        let job_rx = Arc::new(Mutex::new(job_rx));
        let result_rx = Arc::new(Mutex::new(result_rx));
        let wanted: Arc<Mutex<HashSet<K>>> = Arc::new(Mutex::new(HashSet::new()));

        let mut handles = Vec::with_capacity(config.threads);

        for _ in 0..config.threads {
            let rx = Arc::clone(&job_rx);
            let tx = result_tx.clone();
            let w = Arc::clone(&wanted);
            let be = Arc::clone(&backend);
            let dec = Arc::clone(&decode);
            let cfg = config.clone();

            let handle = thread::spawn(move || {
                worker_loop::<K, Payload>(&rx, &tx, &w, &be, &dec, &cfg);
            });
            handles.push(handle);
        }

        Self {
            job_tx,
            result_rx,
            wanted,
            _handles: handles,
        }
    }

    /// 向线程池提交一个任务。非阻塞（入队等待 worker）。
    pub fn submit(&self, job: Job<K>) {
        let _ = self.job_tx.send(job);
    }

    /// 排空已完成的结果（非阻塞）。最多返回 `max` 个结果。
    ///
    /// 对应 L1047-1048 处的 `tex_rx.rx.lock().unwrap().try_recv()` 排空循环，
    /// 受 `MAX_TEXTURE_UPLOADS_PER_FRAME` 限制。
    pub fn drain_results(&self, max: usize) -> Vec<JobResult<K, Payload>> {
        let rx = self.result_rx.lock().unwrap();
        let mut results = Vec::new();
        while results.len() < max {
            match rx.try_recv() {
                Ok(r) => results.push(r),
                Err(_) => break,
            }
        }
        results
    }

    /// 替换 wanted 集。worker 在抓取前会检查此门控（L2161）。
    pub fn refresh_wanted(&self, keys: &[K]) {
        let mut w = self.wanted.lock().unwrap();
        w.clear();
        w.extend(keys.iter().copied());
    }

    /// 向 wanted 集添加键而不清空（用于立即注入，L448-451）。
    pub fn extend_wanted(&self, keys: &[K]) {
        let mut w = self.wanted.lock().unwrap();
        w.extend(keys.iter().copied());
    }

    /// 从 wanted 集中移除单个键。
    ///
    /// 由 `ResourceBackend::cancel`（M8）使用，以便 worker 门控（L2161）发现
    /// 该键缺失，并为那个特定键产生一个 `Aborted` 结果，
    /// 而不扰动其余在途的 wanted 集。这是附加式的——
    /// 黄金路径的 `GenericPipeline` 仍继续依赖 `refresh_wanted`。
    pub fn remove_wanted(&self, key: &K) {
        self.wanted.lock().unwrap().remove(key);
    }

    /// 关闭任务通道，通知 worker 排空后退出。
    pub fn shutdown(self) {
        drop(self.job_tx);
        // 当 job_rx 返回 Err（通道关闭）时 worker 将退出。
    }
}

/// 单次重试退避休眠的上限（L1 review fix）。修复前的
/// `backoff_base << attempt` 会无界增长，因此一个较大的 `max_attempts`
/// 可能休眠数小时，而 `1u32 << attempt` 一旦 `attempt >= 32`
/// 就会溢出（debug 下 panic / release 下环绕）。
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// 计算第 `attempt` 次重试的退避：`base << min(attempt, 20)`，
/// 饱和并限制到 [`MAX_BACKOFF`]。L1 review fix——移位被限制
/// （不会出现 `1u32 << attempt` 溢出）且结果有一个合理的上限。
fn backoff_for(base: Duration, attempt: u32) -> Duration {
    let shift = attempt.min(20);
    base.saturating_mul(1u32 << shift).min(MAX_BACKOFF)
}

/// worker 循环：拉取任务、在 wanted 上门控、带重试地抓取、解码、发送结果。
///
/// 忠实复刻 `dynamic_globe.rs:2153-2284`。
fn worker_loop<K, Payload>(
    job_rx: &Arc<Mutex<mpsc::Receiver<Job<K>>>>,
    result_tx: &mpsc::Sender<JobResult<K, Payload>>,
    wanted: &Arc<Mutex<HashSet<K>>>,
    backend: &Arc<dyn NetworkBackend>,
    decode: &Decoder<Payload>,
    config: &PoolConfig,
) where
    K: Hash + Eq + Copy + Send + 'static,
    Payload: Send + 'static,
{
    loop {
        // 阻塞直到有任务到来（L2154：`job_rx.lock().unwrap().recv()`）
        let job = match job_rx.lock().unwrap().recv() {
            Ok(j) => j,
            Err(_) => return, // 通道关闭——shutdown
        };

        // wanted 集门控（L2161）：跳过没人会看的抓取
        if !wanted.lock().unwrap().contains(&job.key) {
            let _ = result_tx.send(JobResult {
                key: job.key,
                outcome: JobOutcome::Aborted,
            });
            continue;
        }

        // 带指数退避的重试循环（L2186-2191）
        let mut delivered = false;
        for attempt in 0..config.max_attempts {
            if attempt > 0 {
                // L2188-2190：sleep(250ms << attempt)。L1 review fix：限制
                // 移位（attempt.min(20)）以便 `1u32 << attempt` 不会溢出，
                // 并将乘积限制到 MAX_BACKOFF，这样一个较大的 max_attempts
                // 不会产生无界（数小时）的休眠。
                thread::sleep(backoff_for(config.backoff_base, attempt));
            }

            match backend.fetch(&job.url) {
                FetchResult::Ok(data) => {
                    match decode(&data) {
                        Some(payload) => {
                            let _ = result_tx.send(JobResult {
                                key: job.key,
                                outcome: JobOutcome::Success(payload),
                            });
                        }
                        None => {
                            // 解码返回 None = 占位符瓦片（L2211-2229）
                            let _ = result_tx.send(JobResult {
                                key: job.key,
                                outcome: JobOutcome::Placeholder,
                            });
                        }
                    }
                    delivered = true;
                    break;
                }
                FetchResult::Permanent(_) => {
                    // 404 等：视为占位符（瓦片不存在）
                    let _ = result_tx.send(JobResult {
                        key: job.key,
                        outcome: JobOutcome::Placeholder,
                    });
                    delivered = true;
                    break;
                }
                FetchResult::Transient(_) => {
                    // 重试（L2186：`for attempt in 0..3u32`）
                    continue;
                }
            }
        }

        if !delivered {
            // 所有重试耗尽（L2265-2283）：Failed，而非占位符
            let _ = result_tx.send(JobResult {
                key: job.key,
                outcome: JobOutcome::Failed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::ureq_backend::UreqBackend;
    use std::collections::HashMap;

    /// 为测试返回固定响应的 mock backend。
    struct MockBackend {
        responses: Mutex<HashMap<String, FetchResult>>,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                responses: Mutex::new(HashMap::new()),
            }
        }

        fn add(&self, url: &str, result: FetchResult) {
            self.responses.lock().unwrap().insert(url.to_string(), result);
        }
    }

    impl NetworkBackend for MockBackend {
        fn fetch(&self, url: &str) -> FetchResult {
            self.responses
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or(FetchResult::Transient("not mocked".into()))
        }
        fn name(&self) -> &str {
            "mock"
        }
        fn timeout(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    type TileKey = (u32, u32, u32);

    /// 测试用解码器：非空数据原样返回为 `Some`，空数据返回 `None`（占位符）。
    fn test_decoder(data: &[u8]) -> Option<Vec<u8>> {
        if data.is_empty() { None } else { Some(data.to_vec()) }
    }

    #[test]
    /// 验证给定 URL 命中 wanted 集且 fetch 成功时，任务被处理为 `Success`。
    fn pool_processes_job_successfully() {
        let mock = Arc::new(MockBackend::new());
        mock.add("http://tile/1", FetchResult::Ok(vec![1, 2, 3]));

        let decode: Decoder<Vec<u8>> = Arc::new(test_decoder);
        let pool: WorkerPool<TileKey, Vec<u8>> = WorkerPool::spawn(
            mock,
            decode,
            PoolConfig {
                threads: 2,
                max_attempts: 3,
                backoff_base: Duration::from_millis(10),
            },
        );

        pool.refresh_wanted(&[(1, 0, 4)]);
        pool.submit(Job {
            key: (1, 0, 4),
            url: "http://tile/1".into(),
            priority: 1.0,
        });

        std::thread::sleep(Duration::from_millis(100));
        let results = pool.drain_results(10);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, JobOutcome::Success(_)));
    }

    #[test]
    /// 验证未加入 wanted 集的瓦片在抓取前被门控取消，产生 `Aborted`。
    fn pool_aborts_unwanted_tile() {
        let mock = Arc::new(MockBackend::new());
        let decode: Decoder<Vec<u8>> = Arc::new(test_decoder);
        let pool: WorkerPool<TileKey, Vec<u8>> = WorkerPool::spawn(
            mock,
            decode,
            PoolConfig {
                threads: 1,
                max_attempts: 1,
                backoff_base: Duration::from_millis(10),
            },
        );

        // 不添加到 wanted 集
        pool.submit(Job {
            key: (9, 9, 9),
            url: "http://tile/9".into(),
            priority: 1.0,
        });

        std::thread::sleep(Duration::from_millis(100));
        let results = pool.drain_results(10);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, JobOutcome::Aborted));
    }

    #[test]
    /// 验证连续 `Transient` 失败会重试至耗尽，最终报为 `Failed`。
    fn pool_retries_transient_failures() {
        let mock = Arc::new(MockBackend::new());
        mock.add(
            "http://tile/fail",
            FetchResult::Transient("timeout".into()),
        );

        let decode: Decoder<Vec<u8>> = Arc::new(test_decoder);
        let pool: WorkerPool<TileKey, Vec<u8>> = WorkerPool::spawn(
            mock,
            decode,
            PoolConfig {
                threads: 1,
                max_attempts: 3,
                backoff_base: Duration::from_millis(10),
            },
        );

        pool.refresh_wanted(&[(5, 5, 5)]);
        pool.submit(Job {
            key: (5, 5, 5),
            url: "http://tile/fail".into(),
            priority: 1.0,
        });

        std::thread::sleep(Duration::from_millis(500));
        let results = pool.drain_results(10);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, JobOutcome::Failed));
    }

    #[test]
    /// 验证解码器返回 `None` 时判定为占位符瓦片，产生 `Placeholder`。
    fn pool_detects_placeholder() {
        let mock = Arc::new(MockBackend::new());
        mock.add("http://tile/ph", FetchResult::Ok(vec![0, 0, 0]));

        // 解码器对此数据返回 None = 占位符
        let decode: Decoder<Vec<u8>> = Arc::new(|_data: &[u8]| None);
        let pool: WorkerPool<TileKey, Vec<u8>> = WorkerPool::spawn(
            mock,
            decode,
            PoolConfig {
                threads: 1,
                max_attempts: 3,
                backoff_base: Duration::from_millis(10),
            },
        );

        pool.refresh_wanted(&[(2, 2, 4)]);
        pool.submit(Job {
            key: (2, 2, 4),
            url: "http://tile/ph".into(),
            priority: 1.0,
        });

        std::thread::sleep(Duration::from_millis(100));
        let results = pool.drain_results(10);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, JobOutcome::Placeholder));
    }

    #[test]
    /// 编译期断言 `UreqBackend` 满足 `Send + Sync`（跨线程共享前提）。
    fn ureq_backend_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<UreqBackend>();
    }

    #[test]
    /// 验证退避计算对移位与上限做饱和，不会溢出或无界休眠。
    fn backoff_for_caps_shift_and_ceiling() {
        // L1 review fix：受限的移位（不会 `1u32 << attempt` 溢出）+ 合理的
        // 上限，这样一个巨大的尝试次数不会休眠数小时。
        let base = Duration::from_millis(250);
        assert_eq!(backoff_for(base, 1), Duration::from_millis(500)); // 250 << 1
        assert_eq!(backoff_for(base, 2), Duration::from_millis(1000)); // 250 << 2
        // attempt >= 32 在修复前会溢出 `1u32 << attempt`；现在已被限制。
        assert_eq!(backoff_for(base, 40), MAX_BACKOFF);
        assert_eq!(backoff_for(base, u32::MAX), MAX_BACKOFF);
        // 对任何尝试次数都不超过上限。
        for attempt in 0..64u32 {
            assert!(backoff_for(base, attempt) <= MAX_BACKOFF);
        }
    }
}
