//! 「自研开销」基准（设计方案 §10.6 AC-12 的前半）——**离线**，不连数据库、不碰 ODBC。
//!
//! 判据原文：**假 ODBC 下 `select 1` 自研开销 < 50 µs**。
//!
//! # 口径：这条测的到底是什么
//!
//! 后端换成 `tests/common` 那个**假**实现（`FakeBackend`），于是链路上只剩我们自己写的
//! 代码：`PooledConn::query_one` → 池的阻塞线程往返（channel hop）→ `dm-query` 的参数
//! 封送与 `RowSet` 物化 → `dm-core` 的值模型 → `dm-error` 的上下文/指纹。驱动与网络
//! **一分钱都不在里面**。
//!
//! 所以这个数字是「我们这层的**地板价**」：真机上整条 `select 1` 的延迟必然比它大
//! （要加 ODBC 驱动 + 服务端），而**开销**那一项如果连地板价都超过 50 µs，
//! AC-12 就不必再上真机去测了。真机两口径的对照见
//! `crates/dameng/benches/latency.rs`（`全栈 − 裸驱动`）。
//!
//! # 跑法
//!
//! ```text
//! cargo +stable-x86_64-pc-windows-msvc bench -p dm-query --bench overhead
//! cargo +stable-x86_64-pc-windows-msvc bench -p dm-query --bench overhead -- --strict
//! ```
//!
//! 默认**只打印判定**并返回 0：基准跑在共享开发机上，几微秒的抖动不该把别人的门禁弄红。
//! 要把它当门禁用就加 `--strict`（判定不过则退出码 1）。判据本身写死在
//! [`BUDGET_US`]，改判据要连着设计方案一起改。

#![allow(clippy::print_stdout)]

#[path = "../tests/common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::Fixture;
use dm_core::{DataType, DmValue};
use dm_error::{DmError, DmErrorKind, Stage};

/// 被测语句。与真机基准用同一句，便于两条数字对照。
const SQL: &str = "select 1 as N from dual";

/// 复用借出连接的那条路径迭代次数（单次几微秒，跑多点才稳）。
const ITERS_REUSE: usize = 20_000;
/// 「每次借出 + 查询 + 归还」的迭代次数（每次都要过一遍池的握手，慢两个量级）。
const ITERS_ACQUIRE: usize = 5_000;
/// 不计入统计的热身轮数：头几百次会把管道、线程池、tracing 的懒初始化都算进去。
const WARMUP: usize = 2_000;

/// AC-12 的判据：自研开销上限（微秒）= 50 µs。
const BUDGET_US: f64 = 50.0;

/// 一组样本的分位数（单位：微秒）。
struct Stats {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

impl Stats {
    fn of(mut samples: Vec<Duration>) -> Self {
        assert!(!samples.is_empty(), "样本不能为空");
        samples.sort_unstable();
        let last = samples.len() - 1;
        let at = |q: f64| -> f64 {
            // 最近秩法：`q=0.99` 且 100 个样本时取第 99 个（0 起算），不插值。
            let rank = (last as f64 * q).round() as usize;
            samples[rank.min(last)].as_secs_f64() * 1e6
        };
        Self {
            p50: at(0.50),
            p95: at(0.95),
            p99: at(0.99),
            max: samples[last].as_secs_f64() * 1e6,
        }
    }
}

/// 把一批样本打成一行表。
fn row(label: &str, iters: usize, stats: &Stats) {
    println!(
        "{label:<28} {iters:>7}  {:>9.3} {:>9.3} {:>9.3} {:>9.3}",
        stats.p50, stats.p95, stats.p99, stats.max
    );
}

fn header() {
    println!(
        "\n{:<28} {:>7}  {:>9} {:>9} {:>9} {:>9}",
        "口径", "轮数", "P50", "P95", "P99", "max"
    );
    println!("{}", "-".repeat(80));
}

async fn run() -> Result<(), DmError> {
    println!("== AC-12 / 自研开销：假 ODBC 下 `{SQL}`（单位 µs） ==");
    println!("后端 = 假实现（无驱动、无网络）；热身 {WARMUP} 轮不计入；判据 P50 < {BUDGET_US} µs");

    let fixture = Fixture::open(2).await;
    // 一句话、一行、一列：把「服务端工作量」压到最小，剩下的都是我们的开销。
    fixture.shared.configure(|plan| {
        plan.columns = common::columns(&[("N", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)]];
        plan.batch_rows = 4;
    });

    // ---- 口径 A：借出一次，之后连查（池握手只付一次） ----
    let mut conn = fixture.conn().await;
    for _ in 0..WARMUP {
        conn.query_one(SQL, ()).await?;
    }
    let mut samples = Vec::with_capacity(ITERS_REUSE);
    for _ in 0..ITERS_REUSE {
        let start = Instant::now();
        conn.query_one(SQL, ()).await?;
        samples.push(start.elapsed());
    }
    let reuse = Stats::of(samples);
    drop(conn);

    // ---- 口径 B：每次借出 + 查询 + 归还（含池的完整握手） ----
    for _ in 0..(WARMUP / 4) {
        let mut conn = fixture.conn().await;
        conn.query_one(SQL, ()).await?;
    }
    let mut samples = Vec::with_capacity(ITERS_ACQUIRE);
    for _ in 0..ITERS_ACQUIRE {
        let start = Instant::now();
        let mut conn = fixture.conn().await;
        conn.query_one(SQL, ()).await?;
        drop(conn);
        samples.push(start.elapsed());
    }
    let acquire = Stats::of(samples);

    header();
    row("A 复用借出的连接", ITERS_REUSE, &reuse);
    row("B 每次借出+查询+归还", ITERS_ACQUIRE, &acquire);

    println!("\n-- 判定（AC-12 前半）--");
    let pass = reuse.p50 < BUDGET_US;
    println!(
        "自研开销（口径 A 的 P50）= {:.3} µs，判据 < {BUDGET_US} µs ⇒ {}",
        reuse.p50,
        if pass { "达标" } else { "**未达标**" }
    );
    println!(
        "口径 B 的 P50 = {:.3} µs（其中隔离出的池握手 ≈ {:.3} µs）",
        acquire.p50,
        (acquire.p50 - reuse.p50).max(0.0)
    );

    // 池指标兜底：跑完两万轮之后不该有连接泄漏。
    let stats = fixture.pool.stats();
    println!(
        "池指标：size={} in_use={} idle={} 建连失败={} 累计借出={} 累计归还={}",
        stats.size, stats.in_use, stats.idle, stats.connect_failed, stats.acquired, stats.released
    );

    fixture.shutdown().await;

    if std::env::args().any(|arg| arg == "--strict") && !pass {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "AC-12 前半未达标：自研开销 P50 {:.3} µs ≥ {BUDGET_US} µs",
                reuse.p50
            )),
            Stage::Other,
        ));
    }
    Ok(())
}

fn main() -> Result<(), DmError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("建 current_thread 运行时不该失败");
    runtime.block_on(run())
}
