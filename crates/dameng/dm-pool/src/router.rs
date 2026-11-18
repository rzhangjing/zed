//! 读写分离路由（设计方案 §6.10，feature `rw-split`）。
//!
//! # 冻结的规则（§6.10）
//!
//! 1. 显式 hint 强制主库（写操作、读己之写场景）——本模块用 [`RwMode::ReadWrite`]
//!    表达这个 hint：**只有 `ReadOnly` 才可能被送到备库**；
//! 2. **事务内粘住主库**：事务由 [`Router::acquire_primary`] 起，之后同一事务里的每条
//!    语句都用同一个 `PoolGuard`（连接是借出来的，天然粘住）；
//! 3. **无备库时回退主库并计数**（[`RouterStats::fallbacks`]）；
//! 4. **冲突检测**：驱动侧 `RW_SEPARATE=ON` 与应用层路由是**二选一**（`R-12`）——
//!    检测到就拒绝构造 Router。两层路由叠加会让「这一条到底发到哪」变得不可预测。
//!
//! # 为什么决策逻辑是一个纯函数
//!
//! [`pick`] 只吃「策略 + 请求序号 + 各备库的在飞数 + 随机种子」四样东西，不碰池。
//! 于是 18 个路由决策用例（T5.5 的验收项）可以在**没有任何连接**的情况下全部覆盖，
//! 而 [`Router`] 自己只剩下「取配置、挑池、调 `pick`」这三行胶水。
//! 这也是本模块唯一的可测性设计：`Random` 的种子可以被测试钉住。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dm_core::{Backend, DmConfig};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};

use crate::inner::DmPool;
use crate::stats::PoolStats;

/// 这条请求能不能发到备库。
///
/// `ReadWrite` 是「强制主库」的显式 hint：写操作、以及「刚写完要读回来」的场景
/// 必须用它，否则会读到备库上的旧值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RwMode {
    /// 读写：**一定**走主库。
    ReadWrite,
    /// 只读：**可能**走备库（由 [`RwStrategy`] 决定）。
    ReadOnly,
}

impl RwMode {
    /// 是否允许发到备库。
    #[must_use]
    pub const fn allows_replica(self) -> bool {
        matches!(self, Self::ReadOnly)
    }

    /// 日志/指标用的短名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadWrite => "read-write",
            Self::ReadOnly => "read-only",
        }
    }
}

/// 备库怎么挑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RwStrategy {
    /// 轮转（`RoundRobin`）：最省事、分布最均匀，但完全不看备库的忙闲。
    RoundRobin,
    /// 随机。
    Random,
    /// 挑在飞数最少的那个（负载感知）；并列时取下标最小的，保证可预测。
    LeastInFlight,
    /// 只有 `n%` 的只读请求去备库，其余留在主库。
    ///
    /// 用途：备库刚接进来、或者读放大需要**渐进**放开时，先用一个小比例探路。
    /// `n` 必须 ≤ 100（构造 [`Router`] 时校验）。
    Percent(u8),
}

impl RwStrategy {
    /// 日志/指标用的短名。
    #[must_use]
    pub fn as_str(self) -> String {
        match self {
            Self::RoundRobin => "round-robin".to_owned(),
            Self::Random => "random".to_owned(),
            Self::LeastInFlight => "least-in-flight".to_owned(),
            Self::Percent(n) => format!("percent({n})"),
        }
    }
}

/// 选出来的目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// 主库。
    Primary,
    /// 第几个备库。
    Replica(usize),
}

/// 纯决策：这条只读请求去哪。
///
/// - `seq`：单调递增的请求序号（`RoundRobin` 与 `Percent` 用它，保证可复现）；
/// - `loads`：各备库当前的在飞数（`LeastInFlight` 用它）；
/// - `seed`：`Random` 的种子（不进位的伪随机，测试可以钉住）。
///
/// `loads` 为空 = 没有备库：返回 `Primary`，调用方据此计 `fallbacks`。
pub(crate) fn pick(strategy: RwStrategy, seq: u64, loads: &[usize], seed: u64) -> Target {
    if loads.is_empty() {
        return Target::Primary;
    }
    let len = loads.len();
    match strategy {
        RwStrategy::RoundRobin => Target::Replica((seq % len as u64) as usize),
        RwStrategy::Random => Target::Replica((xorshift(seed) % len as u64) as usize),
        RwStrategy::LeastInFlight => {
            // 并列取下标最小的：同一个负载状态下决策必须可复现（否则用例会飘）。
            let mut best = 0;
            for (index, load) in loads.iter().enumerate() {
                if *load < loads[best] {
                    best = index;
                }
            }
            Target::Replica(best)
        }
        RwStrategy::Percent(percent) => {
            // `seq % 100 < percent` 让前 `percent` 个请求去备库——比例精确，且不需要浮点。
            if (seq % 100) < u64::from(percent) {
                Target::Replica((seq % len as u64) as usize)
            } else {
                Target::Primary
            }
        }
    }
}

/// 一次伪随机（xorshift64*）：不为了一个随机路由去引 `rand`。
fn xorshift(seed: u64) -> u64 {
    let mut x = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// 路由侧的累计计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RouterStats {
    /// 只读请求实际发到备库的次数。
    pub routed_replica: u64,
    /// 发到主库的次数（含写请求、命中 `Percent` 之外的只读、以及回退）。
    pub routed_primary: u64,
    /// **想要备库但没有备库**（或备库列表为空）而回退主库的次数。
    pub fallbacks: u64,
}

/// 主池 + 备池的路由器。
///
/// `Debug` 是手写的：池本身不带 `Debug`（`DmPool` 是手写实现，`Arc` 展开太吵），
/// 这里只打「几个备库、什么策略」。
pub struct Router<B: Backend = crate::DefaultBackend> {
    primary: Arc<DmPool<B>>,
    replicas: Vec<Arc<DmPool<B>>>,
    strategy: RwStrategy,
    /// 请求序号（`RoundRobin` / `Percent`）。
    seq: AtomicU64,
    /// `Random` 的种子：每挑一次就前进一格，测试可以用固定种子。
    seed: AtomicU64,
    routed_replica: AtomicU64,
    routed_primary: AtomicU64,
    fallbacks: AtomicU64,
}

impl<B: Backend> std::fmt::Debug for Router<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("replicas", &self.replicas.len())
            .field("strategy", &self.strategy.as_str())
            .finish_non_exhaustive()
    }
}

impl<B: Backend> Router<B> {
    /// 建一个路由器。
    ///
    /// # Errors
    ///
    /// - `RwStrategy::Percent(n)` 的 `n > 100` → `Config`；
    /// - 任何主/备池的配置里开着驱动侧 `RW_SEPARATE` → `Config`（`R-12` 冲突检测）。
    pub fn new(
        primary: Arc<DmPool<B>>,
        replicas: Vec<Arc<DmPool<B>>>,
        strategy: RwStrategy,
    ) -> Result<Self, DmError> {
        if let RwStrategy::Percent(percent) = strategy
            && percent > 100
        {
            return Err(config_error(format!(
                "RwStrategy::Percent({percent}) 越界：百分比必须 ≤ 100"
            )));
        }
        // 冲突检测：驱动内建读写分离与应用层路由**二选一**。
        check_no_driver_splitting(primary.config())?;
        for (index, replica) in replicas.iter().enumerate() {
            check_no_driver_splitting(replica.config()).map_err(|err| {
                let mut err = err;
                err.push_diag(DiagRecord::native(
                    0,
                    format!("冲突来自第 {index} 个备库池的配置"),
                ));
                err
            })?;
        }
        Ok(Self {
            primary,
            replicas,
            strategy,
            seq: AtomicU64::new(0),
            seed: AtomicU64::new(0x9E37_79B9_7F4A_7C15),
            routed_replica: AtomicU64::new(0),
            routed_primary: AtomicU64::new(0),
            fallbacks: AtomicU64::new(0),
        })
    }

    /// 钉住随机种子（测试用；生产上不必调）。
    #[must_use]
    pub fn with_seed(self, seed: u64) -> Self {
        self.seed.store(seed, Ordering::Relaxed);
        self
    }

    /// 主池。
    #[must_use]
    pub fn primary(&self) -> &Arc<DmPool<B>> {
        &self.primary
    }

    /// 备池。
    #[must_use]
    pub fn replicas(&self) -> &[Arc<DmPool<B>>] {
        &self.replicas
    }

    /// 当前策略。
    #[must_use]
    pub const fn strategy(&self) -> RwStrategy {
        self.strategy
    }

    /// 换策略（运营期调比例用；不影响已经在飞的事务）。
    pub fn set_strategy(&mut self, strategy: RwStrategy) -> Result<(), DmError> {
        if let RwStrategy::Percent(percent) = strategy
            && percent > 100
        {
            return Err(config_error(format!(
                "RwStrategy::Percent({percent}) 越界：百分比必须 ≤ 100"
            )));
        }
        self.strategy = strategy;
        Ok(())
    }

    /// 各备库当前的在飞数。
    #[must_use]
    pub fn replica_loads(&self) -> Vec<usize> {
        self.replicas
            .iter()
            .map(|pool| pool.stats().in_use)
            .collect()
    }

    /// 备池的统计快照（运维看备库健不健康）。
    #[must_use]
    pub fn replica_stats(&self) -> Vec<PoolStats> {
        self.replicas.iter().map(|pool| pool.stats()).collect()
    }

    /// 累计计数。
    #[must_use]
    pub fn stats(&self) -> RouterStats {
        RouterStats {
            routed_replica: self.routed_replica.load(Ordering::Relaxed),
            routed_primary: self.routed_primary.load(Ordering::Relaxed),
            fallbacks: self.fallbacks.load(Ordering::Relaxed),
        }
    }

    /// 只选池，不借连接（想自己 `acquire` 时用）。
    ///
    /// 无备库、或策略把这条请求留给主库时，返回主池。
    pub fn route(&self, mode: RwMode) -> &Arc<DmPool<B>> {
        let target = self.decide(mode);
        match target {
            Target::Primary => {
                self.routed_primary.fetch_add(1, Ordering::Relaxed);
                &self.primary
            }
            Target::Replica(index) => {
                self.routed_replica.fetch_add(1, Ordering::Relaxed);
                &self.replicas[index]
            }
        }
    }

    /// 路由并借出连接。
    ///
    /// # Errors
    ///
    /// 与 [`DmPool::acquire`] 一致（超时、池已关闭、建连失败）。
    pub async fn acquire(&self, mode: RwMode) -> Result<crate::conn::PoolGuard<B>, DmError> {
        self.route(mode).acquire().await
    }

    /// **事务入口**：事务一律开在主库（§6.10 规则 2）。
    ///
    /// 事务里的后续语句都用这个 guard，于是「粘住主库」是结构性保证，
    /// 不依赖调用方每句都记得传 `RwMode::ReadWrite`。
    ///
    /// # Errors
    ///
    /// 与 [`DmPool::acquire`] 一致。
    pub async fn acquire_primary(&self) -> Result<crate::conn::PoolGuard<B>, DmError> {
        self.routed_primary.fetch_add(1, Ordering::Relaxed);
        self.primary.acquire().await
    }

    /// 决策（含回退计数）。
    fn decide(&self, mode: RwMode) -> Target {
        if !mode.allows_replica() {
            return Target::Primary;
        }
        let loads = self.replica_loads();
        if loads.is_empty() {
            // 规则 3：没有备库不是错误，回退主库并计数。
            self.fallbacks.fetch_add(1, Ordering::Relaxed);
            return Target::Primary;
        }
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let seed = self
            .seed
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
        pick(self.strategy, seq, &loads, seed)
    }
}

/// 驱动侧 `RW_SEPARATE` 冲突检测（`R-12`）。
///
/// 两处都要查：结构化字段 `rw_separate`（连接串解析的结果），以及 `extra` 里
/// 原样留下的关键字（`DmConfig::from_connection_string` 会把不认识的关键字塞进 `extra`，
/// 而 `RW_SEPARATE_PERCENT` 这类没进结构化字段的只能在这里查）。
///
/// # Errors
///
/// 检测到开启 → `Config`。
pub fn check_no_driver_splitting(cfg: &DmConfig) -> Result<(), DmError> {
    if cfg.rw_separate == Some(true) {
        return Err(conflict_error("rw_separate = Some(true)"));
    }
    for (key, value) in &cfg.extra {
        if key.eq_ignore_ascii_case("RW_SEPARATE") && is_truthy(value) {
            return Err(conflict_error(&format!("连接串关键字 {key}={value}")));
        }
    }
    Ok(())
}

/// 连接串里的开/关写法（达梦接受 ON/OFF，也接受 1/0、TRUE/FALSE）。
fn is_truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_uppercase().as_str(),
        "ON" | "TRUE" | "1" | "Y" | "YES"
    )
}

fn conflict_error(source: &str) -> DmError {
    let mut err = DmError::at(
        DmErrorKind::Config(
            "驱动侧 RW_SEPARATE 已开启：驱动内建读写分离与应用层路由必须二选一（设计方案 R-12）"
                .to_owned(),
        ),
        Stage::Config,
    );
    err.push_diag(DiagRecord::native(
        0,
        format!(
            "冲突来源：{source}（设计方案 R-12）。两层路由叠加会让「这一条语句到底发到哪个库」\
             变得不可预测，请二选一：要么在连接串里关掉 RW_SEPARATE（改用本 Router），\
             要么不要构造 Router（改用驱动内建分离）。"
        ),
    ));
    err
}

fn config_error(message: String) -> DmError {
    let mut err = DmError::at(DmErrorKind::Config(message.clone()), Stage::Config);
    err.push_diag(DiagRecord::native(0, message));
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq_picks(strategy: RwStrategy, count: usize) -> Vec<Target> {
        (0..count)
            .map(|seq| pick(strategy, seq as u64, &[0; 3], 42))
            .collect()
    }

    #[test]
    fn read_write_mode_never_goes_to_a_replica() {
        assert!(!RwMode::ReadWrite.allows_replica());
        assert!(RwMode::ReadOnly.allows_replica());
        // `route` 的判据就是这一个布尔值，所以这里等价于「写永远走主库」。
        assert_eq!(RwMode::ReadWrite.as_str(), "read-write");
    }

    #[test]
    fn round_robin_cycles_over_every_replica() {
        let picks = seq_picks(RwStrategy::RoundRobin, 7);
        assert_eq!(
            picks,
            vec![
                Target::Replica(0),
                Target::Replica(1),
                Target::Replica(2),
                Target::Replica(0),
                Target::Replica(1),
                Target::Replica(2),
                Target::Replica(0),
            ]
        );
    }

    #[test]
    fn round_robin_with_one_replica_is_always_that_replica() {
        for seq in 0..5 {
            assert_eq!(
                pick(RwStrategy::RoundRobin, seq, &[9], 0),
                Target::Replica(0)
            );
        }
    }

    #[test]
    fn least_in_flight_picks_the_idle_replica() {
        assert_eq!(
            pick(RwStrategy::LeastInFlight, 0, &[5, 1, 4], 0),
            Target::Replica(1)
        );
        assert_eq!(
            pick(RwStrategy::LeastInFlight, 7, &[0, 3, 2], 0),
            Target::Replica(0)
        );
    }

    #[test]
    fn least_in_flight_breaks_ties_by_lowest_index() {
        // 并列时必须可复现：三个一样忙就选 0，不能「随机挑一个」。
        assert_eq!(
            pick(RwStrategy::LeastInFlight, 3, &[2, 2, 2], 9),
            Target::Replica(0)
        );
        assert_eq!(
            pick(RwStrategy::LeastInFlight, 3, &[4, 0, 0], 9),
            Target::Replica(1)
        );
    }

    #[test]
    fn least_in_flight_ignores_the_sequence_number() {
        let first = pick(RwStrategy::LeastInFlight, 0, &[1, 1, 0], 7);
        let later = pick(RwStrategy::LeastInFlight, 99, &[1, 1, 0], 7);
        assert_eq!(first, later);
    }

    #[test]
    fn random_is_deterministic_for_a_fixed_seed() {
        let a: Vec<Target> = (0..8)
            .map(|i| pick(RwStrategy::Random, i, &[0; 4], 12345))
            .collect();
        let b: Vec<Target> = (0..8)
            .map(|i| pick(RwStrategy::Random, i, &[0; 4], 12345))
            .collect();
        assert_eq!(a, b, "同一种子必须给同一串决策，否则用例会飘");
    }

    #[test]
    fn random_stays_inside_the_replica_range() {
        for seq in 0..1000 {
            match pick(RwStrategy::Random, seq, &[0; 3], seq) {
                Target::Replica(index) => assert!(index < 3),
                Target::Primary => panic!("有备库时随机策略不该回主库"),
            }
        }
    }

    #[test]
    fn random_spreads_over_more_than_one_replica() {
        let mut seen = std::collections::BTreeSet::new();
        for seq in 0..200 {
            if let Target::Replica(index) = pick(RwStrategy::Random, seq, &[0; 4], seq) {
                seen.insert(index);
            }
        }
        assert!(seen.len() > 1, "随机策略必须真的分散：{seen:?}");
    }

    #[test]
    fn percent_zero_keeps_everything_on_the_primary() {
        for seq in 0..50 {
            assert_eq!(
                pick(RwStrategy::Percent(0), seq, &[0; 2], 0),
                Target::Primary
            );
        }
    }

    #[test]
    fn percent_hundred_sends_everything_to_replicas() {
        for seq in 0..50 {
            match pick(RwStrategy::Percent(100), seq, &[0; 2], 0) {
                Target::Replica(index) => assert!(index < 2),
                Target::Primary => panic!("100% 时不该留在主库"),
            }
        }
    }

    #[test]
    fn percent_twenty_five_is_exactly_a_quarter() {
        let mut replica = 0;
        for seq in 0..400 {
            if matches!(
                pick(RwStrategy::Percent(25), seq, &[0; 2], 0),
                Target::Replica(_)
            ) {
                replica += 1;
            }
        }
        assert_eq!(replica, 100, "400 个请求里应当恰好 100 个去备库");
    }

    #[test]
    fn percent_five_keeps_the_first_five_out_of_a_hundred() {
        let picks = seq_picks(RwStrategy::Percent(5), 7);
        assert_eq!(
            picks,
            vec![
                Target::Replica(0),
                Target::Replica(1),
                Target::Replica(2),
                Target::Replica(0),
                Target::Replica(1),
                Target::Primary,
                Target::Primary,
            ]
        );
    }

    #[test]
    fn no_replica_always_falls_back_to_the_primary() {
        for strategy in [
            RwStrategy::RoundRobin,
            RwStrategy::Random,
            RwStrategy::LeastInFlight,
            RwStrategy::Percent(100),
        ] {
            assert_eq!(pick(strategy, 3, &[], 1), Target::Primary);
        }
    }

    #[test]
    fn strategy_names_are_log_friendly() {
        assert_eq!(RwStrategy::RoundRobin.as_str(), "round-robin");
        assert_eq!(RwStrategy::Random.as_str(), "random");
        assert_eq!(RwStrategy::LeastInFlight.as_str(), "least-in-flight");
        assert_eq!(RwStrategy::Percent(30).as_str(), "percent(30)");
    }

    #[test]
    fn driver_side_splitting_is_detected_from_the_structured_field() {
        let mut cfg = fake_config();
        cfg.rw_separate = Some(true);
        let err = check_no_driver_splitting(&cfg).unwrap_err();
        assert!(matches!(err.kind, DmErrorKind::Config(_)));
        assert_eq!(err.context.stage, Stage::Config);
        assert!(
            err.diags.iter().any(|d| d.message.contains("R-12")),
            "冲突诊断里要点出规则编号：{:?}",
            err.diags
        );
    }

    #[test]
    fn driver_side_splitting_is_detected_from_the_raw_keyword() {
        for raw in ["ON", "on", "1", "true", "Y"] {
            let mut cfg = fake_config();
            cfg.extra.insert("RW_SEPARATE".to_owned(), raw.to_owned());
            assert!(
                check_no_driver_splitting(&cfg).is_err(),
                "`RW_SEPARATE={raw}` 必须被识别为开启"
            );
        }
    }

    #[test]
    fn driver_side_splitting_off_is_not_a_conflict() {
        for raw in ["OFF", "0", "false", "N"] {
            let mut cfg = fake_config();
            cfg.extra.insert("RW_SEPARATE".to_owned(), raw.to_owned());
            assert!(check_no_driver_splitting(&cfg).is_ok(), "{raw} 不该算冲突");
        }
        let mut cfg = fake_config();
        cfg.rw_separate = Some(false);
        assert!(check_no_driver_splitting(&cfg).is_ok());
    }

    fn fake_config() -> DmConfig {
        DmConfig::from_connection_string(
            "DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=1;UID=FAKE;PWD=FAKE;DBNAME=FAKE",
        )
        .expect("内置假 DSN 必须能解析")
    }
}
