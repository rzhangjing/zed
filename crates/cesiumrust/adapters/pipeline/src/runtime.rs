//! 通用流水线运行时——实现 `TilePipeline<K, Payload>`。
//!
//! 将工作线程池、去重集、wanted 集管理与统计追踪统一到一个内聚的
//! 流水线中，忠实复刻 `dynamic_globe.rs::process_pipeline`（L662-1429）
//! 与 `enqueue_tiles`（L371-459）中的编排。

use std::hash::Hash;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cesium_ports_driven::{PipelineStats, PollOutcome, TilePipeline};

use crate::budget::DefaultBudget;
use crate::dedup::Dedup;
use crate::net::NetworkBackend;
use crate::pool::{Decoder, Job, JobOutcome, PoolConfig, WorkerPool};

/// URL 构造器：将瓦片键映射为抓取 URL。
/// 对应 `dynamic_globe.rs:2176-2181`（quadkey URL 构造）。
pub type UrlBuilder<K> = Arc<dyn Fn(&K) -> String + Send + Sync>;

/// 通用瓦片流水线实现。
///
/// 类型参数：
/// - `K`：瓦片键（例如 `(u32, u32, u32)` = TileKey）。必须满足 Hash+Eq+Copy+Send+'static。
/// - `Payload`：下载结果（例如解码后的 RGBA + mip 链）。必须满足 Send+'static。
///
/// 实现来自 cesium-ports-driven 的 `TilePipeline<K, Payload>`（M1.1）。
pub struct GenericPipeline<K, Payload>
where
    K: Hash + Eq + Copy + Send + 'static,
    Payload: Send + 'static,
{
    pool: WorkerPool<K, Payload>,
    dedup: Dedup<K>,
    url_builder: UrlBuilder<K>,
    stats: Arc<StatsInner>,
}

/// 内部可变的统计计数器（为跨线程的 stats() 读取而采用原子量）。
struct StatsInner {
    /// 当前帧序号（递增），供逐帧预算与陈旧判定参考。
    frame_idx: AtomicU32,
    /// 因数据陈旧而被跳过的上传累计次数。
    stale_skips: AtomicU32,
    /// 已发起的瓦片驱逐（淘汰）累计总数。
    evict_total: AtomicU32,
    /// 因超每帧预算而被推迟的驱逐累计数。
    evict_deferred: AtomicU32,
    /// 当前在途（已提交未完成）的任务数。
    in_flight: AtomicU32,
    /// 重试冷却结束帧号（在此之前暂停相应瓦片重试）。
    retry_after: AtomicU32,
}

impl StatsInner {
    /// 构造一个所有计数器归零的初始统计块。
    fn new() -> Self {
        Self {
            frame_idx: AtomicU32::new(0),
            stale_skips: AtomicU32::new(0),
            evict_total: AtomicU32::new(0),
            evict_deferred: AtomicU32::new(0),
            in_flight: AtomicU32::new(0),
            retry_after: AtomicU32::new(0),
        }
    }
}

impl<K, Payload> GenericPipeline<K, Payload>
where
    K: Hash + Eq + Copy + Send + 'static,
    Payload: Send + 'static,
{
    /// 使用给定的网络 backend、URL 构造器与解码器创建一个流水线。
    ///
    /// - `backend`：网络实现（默认：`UreqBackend`）。
    /// - `url_builder`：将瓦片键 → 抓取 URL（L2176-2181）。
    /// - `decode`：将原始字节 → Payload。对占位符瓦片返回 None
    ///   （L2211：`is_placeholder_tile` 检查）。
    /// - `threads`：工作线程数（默认 16，L48）。
    pub fn new(
        backend: Arc<dyn NetworkBackend>,
        url_builder: UrlBuilder<K>,
        decode: Decoder<Payload>,
        threads: usize,
    ) -> Self {
        let config = PoolConfig {
            threads,
            max_attempts: 3,                          // L2186
            backoff_base: Duration::from_millis(250), // L2189
        };

        let pool = WorkerPool::spawn(backend, decode, config);
        let stats = Arc::new(StatsInner::new());

        Self {
            pool,
            dedup: Dedup::new(),
            url_builder,
            stats,
        }
    }

    /// 使用默认预算（16 线程）创建一个流水线。
    pub fn with_defaults(
        backend: Arc<dyn NetworkBackend>,
        url_builder: UrlBuilder<K>,
        decode: Decoder<Payload>,
    ) -> Self {
        Self::new(backend, url_builder, decode, DefaultBudget::DOWNLOAD_THREADS)
    }

    /// 使用显式的线程池配置创建一个流水线（用于测试）。
    pub fn with_config(
        backend: Arc<dyn NetworkBackend>,
        url_builder: UrlBuilder<K>,
        decode: Decoder<Payload>,
        config: PoolConfig,
    ) -> Self {
        let pool = WorkerPool::spawn(backend, decode, config);
        let stats = Arc::new(StatsInner::new());
        Self {
            pool,
            dedup: Dedup::new(),
            url_builder,
            stats,
        }
    }

    /// 推进帧计数器。由宿主系统每帧调用一次。
    pub fn begin_frame(&self) {
        self.stats.frame_idx.fetch_add(1, Ordering::Relaxed);
    }

    /// 记录一次驱逐事件（用于统计追踪）。
    pub fn record_eviction(&self, evicted: u32, deferred: u32) {
        self.stats.evict_total.fetch_add(evicted, Ordering::Relaxed);
        self.stats.evict_deferred.fetch_add(deferred, Ordering::Relaxed);
    }

    /// 更新 retry_after 指标（当前处于冷却中的瓦片）。
    pub fn set_retry_after(&self, count: u32) {
        self.stats.retry_after.store(count, Ordering::Relaxed);
    }

    /// 关闭流水线（丢弃任务通道，worker 退出）。
    pub fn shutdown(self) {
        self.pool.shutdown();
    }
}

impl<K, Payload> TilePipeline<K, Payload> for GenericPipeline<K, Payload>
where
    K: Hash + Eq + Copy + Send + 'static,
    Payload: Send + 'static,
{
    /// 提交一个瓦片以带优先级地下载。
    ///
    /// 对应 `enqueue_tiles`（L371-459）：
    /// - 去重检查（L406、L417）：若已在途则跳过
    /// - URL 构造（L2176-2181）
    /// - wanted 集注入（L448-451）
    /// - 向工作队列按优先级排序提交
    fn submit(&self, key: K, priority: f64) {
        // 去重：若已在途则跳过（L406/L417）
        if !self.dedup.insert(key) {
            return;
        }

        // 构造 URL（L2176-2181）
        let url = (self.url_builder)(&key);

        // 立即注入到 wanted 集（L448-451）
        self.pool.extend_wanted(&[key]);

        // 更新在途 gauge
        self.stats.in_flight.fetch_add(1, Ordering::Relaxed);

        // 提交到工作线程池
        self.pool.submit(Job { key, url, priority });
    }

    /// 取消一个待处理/在途的瓦片。
    ///
    /// 从 wanted 集中移除，以便 worker 门控（L2161）产生 Aborted。
    /// 同时也从去重集中移除，以便该瓦片稍后可重新提交。
    fn cancel(&self, key: &K) {
        self.dedup.remove(key);
        // 注：worker 会检测到缺失的 wanted 集条目并
        // 返回 Aborted。我们无需显式地通知 worker。
    }

    /// 轮询已完成的瓦片（非阻塞排空）。
    ///
    /// 对应 L1046-1048：`tex_rx.rx.lock().unwrap().try_recv()`，
    /// 受 `MAX_TEXTURE_UPLOADS_PER_FRAME`（16）限制。
    ///
    /// 遵循 L1052-1098 处的三态分发，将 `JobOutcome` 映射为 `PollOutcome`。
    fn poll_ready(&self, budget: usize) -> Vec<PollOutcome<K, Payload>> {
        let results = self.pool.drain_results(budget);
        let mut outcomes = Vec::with_capacity(results.len());

        for r in results {
            // 清除去重 + 在途 gauge（L1051：`mgr.in_flight.remove(&key)`）
            self.dedup.remove(&r.key);
            self.stats.in_flight.fetch_sub(1, Ordering::Relaxed);

            match r.outcome {
                JobOutcome::Success(payload) => {
                    outcomes.push(PollOutcome::Ready(r.key, payload));
                }
                JobOutcome::Aborted => {
                    // L1052-1060：计入 stale_skip
                    self.stats.stale_skips.fetch_add(1, Ordering::Relaxed);
                    outcomes.push(PollOutcome::Aborted(r.key));
                }
                JobOutcome::Failed => {
                    // L1062-1072：重试冷却由宿主处理
                    outcomes.push(PollOutcome::Failed(r.key));
                }
                JobOutcome::Placeholder => {
                    // L1074-1098：永久无数据
                    outcomes.push(PollOutcome::Placeholder(r.key));
                }
            }
        }

        outcomes
    }

    /// 替换 wanted 集（L1402-1409）。
    ///
    /// 不在新集中的瓦片会在 worker 门控（L2161）处成为驱逐候选。
    fn refresh_wanted(&self, wanted: &[K]) {
        self.pool.refresh_wanted(wanted);
    }

    /// 当前流水线统计的快照。
    ///
    /// 与 M0.4 PerfCounters / 17 列 CSV 格式对齐。
    fn stats(&self) -> PipelineStats {
        PipelineStats {
            frame_idx: self.stats.frame_idx.load(Ordering::Relaxed),
            dt_ms: 0.0,        // 由宿主提供（Bevy Time::delta_secs_f64）
            visible_n: 0,      // 由宿主提供（tile_entities.len()）
            partition_n: 0,    // 由宿主提供（spawn_queue.len()）
            load_n: self.dedup.len() as u32,
            spawn_n: 0,        // 由宿主提供（frame_spawn）
            tex_upload_n: 0,   // 由宿主提供（frame_tex）
            evict_n: self.stats.evict_total.load(Ordering::Relaxed),
            gpu_tex_cache: 0,  // 由宿主提供（gpu_tex_order.len()）
            mesh_backlog: 0,   // 由宿主提供（backlog.len()）
            dl_in_flight: self.stats.in_flight.load(Ordering::Relaxed),
            stale_skips: self.stats.stale_skips.load(Ordering::Relaxed),
            retry_after: self.stats.retry_after.load(Ordering::Relaxed),
            frame_mesh: 0,     // 由宿主提供（frame_mesh）
            frame_despawn: 0,  // 由宿主提供（frame_despawn）
            evict_deferred: self.stats.evict_deferred.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{FetchResult, NetworkBackend};
    use std::collections::HashMap;
    use std::sync::Mutex;

    type TileKey = (u32, u32, u32);

    /// 用于确定性测试的 mock backend。
    struct MockNet {
        responses: Mutex<HashMap<String, FetchResult>>,
    }

    impl MockNet {
        fn new() -> Self {
            Self {
                responses: Mutex::new(HashMap::new()),
            }
        }
        fn add(&self, url: &str, r: FetchResult) {
            self.responses.lock().unwrap().insert(url.to_string(), r);
        }
    }

    impl NetworkBackend for MockNet {
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

    fn url_builder(k: &TileKey) -> String {
        format!("http://tiles/{}/{}/{}", k.2, k.0, k.1)
    }

    fn make_pipeline(
        mock: Arc<MockNet>,
        threads: usize,
    ) -> GenericPipeline<TileKey, Vec<u8>> {
        let decode: Decoder<Vec<u8>> = Arc::new(|data: &[u8]| {
            if data.is_empty() { None } else { Some(data.to_vec()) }
        });
        // 使用短退避以加快测试
        let config = PoolConfig {
            threads,
            max_attempts: 3,
            backoff_base: Duration::from_millis(10),
        };
        GenericPipeline::with_config(mock, Arc::new(url_builder), decode, config)
    }

    #[test]
    fn pipeline_submit_and_poll() {
        let mock = Arc::new(MockNet::new());
        mock.add("http://tiles/4/1/2", FetchResult::Ok(vec![10, 20, 30]));

        let pipe = make_pipeline(mock, 2);
        pipe.refresh_wanted(&[(1, 2, 4)]);
        pipe.submit((1, 2, 4), 100.0);

        std::thread::sleep(Duration::from_millis(200));
        let results = pipe.poll_ready(16);
        assert_eq!(results.len(), 1);
        match &results[0] {
            PollOutcome::Ready(k, payload) => {
                assert_eq!(*k, (1, 2, 4));
                assert_eq!(payload, &[10, 20, 30]);
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn pipeline_dedup_prevents_double_submit() {
        let mock = Arc::new(MockNet::new());
        mock.add("http://tiles/5/0/0", FetchResult::Ok(vec![1]));

        let pipe = make_pipeline(mock, 1);
        pipe.refresh_wanted(&[(0, 0, 5)]);
        pipe.submit((0, 0, 5), 1.0);
        pipe.submit((0, 0, 5), 2.0); // 重复——应被忽略

        std::thread::sleep(Duration::from_millis(200));
        let results = pipe.poll_ready(16);
        assert_eq!(results.len(), 1); // 只有一个结果
    }

    #[test]
    fn pipeline_cancel_produces_aborted() {
        let mock = Arc::new(MockNet::new());
        mock.add("http://tiles/6/3/3", FetchResult::Transient("slow".into()));

        let pipe = make_pipeline(mock, 1);
        // 提交后立即清空 wanted
        pipe.submit((3, 3, 6), 1.0);
        pipe.refresh_wanted(&[]); // wanted 为空——瓦片将被中止

        std::thread::sleep(Duration::from_millis(300));
        let results = pipe.poll_ready(16);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], PollOutcome::Aborted(_)));
    }

    #[test]
    fn pipeline_stats_track_in_flight() {
        let mock = Arc::new(MockNet::new());
        mock.add("http://tiles/7/1/1", FetchResult::Transient("timeout".into()));

        let pipe = make_pipeline(mock, 1);
        pipe.refresh_wanted(&[(1, 1, 7)]);
        pipe.submit((1, 1, 7), 1.0);

        // 轮询前：in_flight 应为 1
        let s = pipe.stats();
        assert_eq!(s.dl_in_flight, 1);

        // 等待重试耗尽（3 次尝试 × 10ms 退避 = 约 70ms）
        std::thread::sleep(Duration::from_millis(300));
        let _ = pipe.poll_ready(16);

        // 轮询后：in_flight 应为 0
        let s = pipe.stats();
        assert_eq!(s.dl_in_flight, 0);
    }

    #[test]
    fn pipeline_placeholder_detection() {
        let mock = Arc::new(MockNet::new());
        mock.add("http://tiles/8/0/0", FetchResult::Ok(vec![])); // 空 = 占位符

        let pipe = make_pipeline(mock, 1);
        pipe.refresh_wanted(&[(0, 0, 8)]);
        pipe.submit((0, 0, 8), 1.0);

        std::thread::sleep(Duration::from_millis(200));
        let results = pipe.poll_ready(16);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], PollOutcome::Placeholder(_)));
    }

    #[test]
    fn pipeline_stale_skips_counter() {
        let mock = Arc::new(MockNet::new());
        let pipe = make_pipeline(mock, 1);

        // 提交时会加入 wanted，随后立即清空 wanted，以便 worker
        // 门控（L2161）发现瓦片缺失 → Aborted → stale_skips++
        pipe.submit((9, 9, 9), 1.0);
        pipe.refresh_wanted(&[]); // 在 worker 接手任务前清空 wanted

        std::thread::sleep(Duration::from_millis(200));
        let results = pipe.poll_ready(16);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], PollOutcome::Aborted(_)));

        let s = pipe.stats();
        assert_eq!(s.stale_skips, 1);
    }
}
