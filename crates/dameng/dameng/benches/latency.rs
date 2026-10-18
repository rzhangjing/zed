//! 真机延迟基准（设计方案 §10.6 AC-12 的后半）——**需要一个可达的达梦 DM8**。
//!
//! 判据原文：**真机 `select 1` P99 < 2 ms（环回连接）**，以及**自研开销 < 50 µs**。
//!
//! # 口径（同一台机、同一个库）
//!
//! `select 1` 组（AC-12 后半的判据对象）：
//!
//! | 口径 | 走了什么 | 用途 |
//! |---|---|---|
//! | A 裸驱动 | `dm_odbc::OdbcConnection` + `exec_direct` + `fetch`，**不经过**池与 `dm-query` | 「ODBC 驱动 + 服务端」的地板价 |
//! | A2 裸驱动+每轮新语句 | 每轮新建语句 + `SQLPrepare` + `SQLExecDirectW`，与 B 的语句生命周期对齐 | 自研开销的减数 |
//! | B 全栈 | 借出一次池化连接，循环 `PooledConn::query_one` | P99 < 2 ms 的判据对象 |
//! | C 每次借出 | 每轮重新借出 + 查询 + 归还 | 池握手的代价（B 与 C 的差） |
//!
//! **DML 组（语句缓存的收益，CAP-C-11 / 设计方案 §5.4 末）**：同一条
//! `insert into … values (?, ?)` 在同一个池化连接上重复执行，缓存开（D）与缓存关（E）
//! 各跑一遍。查询语句**故意不在这一组**：DM8 上「结果集取尽的语句」不可复用
//! （`24000/-70027`，见 `crates/dm-odbc/examples/cursor_reuse_probe.rs`），
//! 所以 `select 1` 组看不出语句缓存的收益——这是规则决定的事实，不是基准的缺陷。
//!
//! | 口径 | 走了什么 | 用途 |
//! |---|---|---|
//! | D 全栈 DML（缓存开） | 默认 `stmt_cache_size = 16`，同一句 `insert` 命中缓存 | 缓存命中后的每条语句成本 |
//! | E 全栈 DML（缓存关） | `pool_stmt_cache_size(0)`，每条语句都 `SQLAllocHandle` + `SQLPrepare` | 收益的减数 |
//!
//! D/E 组会在库里建一张 `DM_BENCH_STMT_CACHE` 表并在结束时删掉（需要建表权限）。
//!
//! **自研开销 := B 的 P50 − A 的 P50**：两条都含同样的驱动与服务端成本，差出来的
//! 就是池桥接 + 参数封送 + `RowSet` 物化 + 错误上下文与指纹。这条差值只在这台机上
//! 成立（服务端负载会同时抬高两边，但不会同比例抬高差值），所以数字要连着环境一起引用。
//!
//! 离线的「纯自研开销」对照（假后端，不含驱动与网络）见
//! `crates/dm-query/benches/overhead.rs`——那个是地板价，本文件的 B−A 不应低于它。
//!
//! # 跑法
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc bench -p dameng --bench latency
//! cargo +stable-x86_64-pc-windows-msvc bench -p dameng --bench latency -- --strict
//! ```
//!
//! 没配 `DM_TEST_DSN` / `DM_SERVER` 时打印一行原因并正常退出（基准只在真库上跑）。
//! 默认**只打印判定**并返回 0；加 `--strict` 才把判定不过变成退出码 1。

#![allow(clippy::print_stdout)]

use std::time::{Duration, Instant};

use dameng::prelude::*;
use dameng::raw::core::RowBuf;
use dameng::raw::odbc::{FetchScratch, OdbcConnection, StmtOutcome, describe_columns, fetch};

/// 被测语句：不落表、不算术，把服务端工作量压到最小。与离线基准用同一句。
const SQL: &str = "select 1 as N from dual";

const WARMUP: usize = 500;
const ITERS: usize = 2_000;
/// 「每次借出」那条更贵，轮数少一点。
const ITERS_ACQUIRE: usize = 500;

/// AC-12 后半的判据：真机 `select 1` 的 P99 上限（微秒）。
const BUDGET_P99_US: f64 = 2_000.0;
/// 自研开销上限（微秒）：`B − A`。
const BUDGET_OVERHEAD_US: f64 = 50.0;

/// 语句缓存收益（CAP-C-11）用的表与语句：DML 才可能命中缓存。
const CACHE_TBL: &str = "DM_BENCH_STMT_CACHE";
const CACHE_SQL: &str = "insert into DM_BENCH_STMT_CACHE (ID, V) values (?, ?)";
/// DML 组每次都要真的写一行，轮数比查询组少一点。
const ITERS_DML: usize = 1_000;
/// DML 组的热身轮数（缓存组第一轮必然 miss，之后全 hit）。
const WARMUP_DML: usize = 100;

/// 从环境读真机配置：`DM_TEST_DSN`（ODBC 连接串）优先，否则 `DM_SERVER` 系。
fn config() -> Option<DmConfig> {
    if let Ok(dsn) = std::env::var("DM_TEST_DSN")
        && !dsn.trim().is_empty()
    {
        return Some(DmConfig::from_connection_string(&dsn).expect("DM_TEST_DSN 不是合法的连接串"));
    }
    if std::env::var("DM_SERVER").is_ok() {
        return Some(DmConfig::from_env().expect("DM_SERVER 环境下无法构造 DmConfig"));
    }
    None
}

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

fn row(label: &str, iters: usize, stats: &Stats) {
    println!(
        "{label:<22} {iters:>7}  {:>9.3} {:>9.3} {:>9.3} {:>9.3}",
        stats.p50, stats.p95, stats.p99, stats.max
    );
}

/// 口径 A：裸驱动，不经过池与 `dm-query`。
fn raw_path(cfg: &DmConfig) -> Result<Stats, DmError> {
    let mut conn = OdbcConnection::connect(cfg)?;
    let mut stmt = conn.allocate_statement()?;

    // 热身：首轮要把驱动的内部缓冲、服务端的语句缓存都点起来。
    for _ in 0..WARMUP {
        let _ = raw_once(&mut stmt)?;
    }

    let mut samples = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let start = Instant::now();
        let _ = raw_once(&mut stmt)?;
        samples.push(start.elapsed());
    }
    Ok(Stats::of(samples))
}

/// 口径 A2：裸驱动，但**每轮重新分配并预处理语句**——与口径 B 的语句生命周期一致
/// （`dm-odbc` 的 `Backend::prepare` 就是「分配句柄 + `SQLPrepare`」，之后 `execute` 走
/// `SQLExecDirectW`）。对齐之后 `B − A2` 才是自研开销；`B − A` 里另外混着「B 每条语句都
/// 重新分配句柄 + 预处理一次」这个语句生命周期差异，两者都要如实打印出来。
fn raw_path_alloc_each(cfg: &DmConfig) -> Result<Stats, DmError> {
    let mut conn = OdbcConnection::connect(cfg)?;

    for _ in 0..WARMUP {
        let _ = raw_alloc_once(&mut conn)?;
    }

    let mut samples = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let start = Instant::now();
        let _ = raw_alloc_once(&mut conn)?;
        samples.push(start.elapsed());
    }
    Ok(Stats::of(samples))
}

/// 每轮一条新语句：分配句柄 → `SQLPrepare` → `SQLExecDirectW` → 取数 → 释放句柄。
fn raw_alloc_once(conn: &mut OdbcConnection) -> Result<usize, DmError> {
    let mut stmt = conn.allocate_statement()?;
    stmt.prepare(SQL)?;
    raw_once(&mut stmt)
}

/// 执行一次并取回结果集的行（返回取到的行数）。
fn raw_once(stmt: &mut dameng::OdbcStatement<'_>) -> Result<usize, DmError> {
    let outcome = stmt.exec_direct(SQL)?;
    let StmtOutcome::ResultSet { .. } = outcome else {
        return Ok(0);
    };
    let columns = describe_columns(stmt, None)?;
    let mut buf = RowBuf::with_capacity(1, 64);
    let mut scratch = FetchScratch::new();
    Ok(fetch(stmt, &columns, &mut buf, &mut scratch)?.rows())
}

/// DML 组：同一个池化连接上重复执行同一条 `insert`。
///
/// `cache_size = 0` 就是「没有语句缓存」的旧行为（每条语句都 `SQLAllocHandle` +
/// `SQLPrepare`），`cache_size = 16` 是默认值（同一条 SQL 第二次起命中缓存）。
/// 返回（样本、缓存 hits、缓存 misses、缓存里剩下的条目数）。
async fn pool_dml(cfg: DmConfig, cache_size: usize) -> Result<(Stats, u64, u64, usize), DmError> {
    let mut cfg = cfg;
    cfg.pool.stmt_cache_size = cache_size;
    let pool = Pool::connect_with(cfg, ExecutorConfig::for_pool(2)).await?;
    let mut conn = PooledConn::acquire(&pool).await?;

    for i in 0..WARMUP_DML {
        let _ = conn.execute(CACHE_SQL, (i as i32, i as i32)).await?;
    }
    let mut samples = Vec::with_capacity(ITERS_DML);
    for i in 0..ITERS_DML {
        let start = Instant::now();
        let _ = conn.execute(CACHE_SQL, (i as i32, i as i32)).await?;
        samples.push(start.elapsed());
    }
    let cache = conn.guard().stmt_cache_stats();
    drop(conn);

    pool.close().await?;
    pool.wait_closed().await;
    Ok((Stats::of(samples), cache.hits, cache.misses, cache.size))
}

/// D/E 组共用的建表与清场（缓存开不开对 DDL 没有意义，用一条短命连接做完）。
async fn prepare_cache_table(cfg: &DmConfig) -> Result<(), DmError> {
    let pool = Pool::connect_with(cfg.clone(), ExecutorConfig::for_pool(1)).await?;
    let mut conn = PooledConn::acquire(&pool).await?;
    // 上一次跑崩了会留下表；先删（不存在时报错忽略）。
    let _ = conn.execute(&format!("drop table {CACHE_TBL}"), ()).await;
    conn.execute(&format!("create table {CACHE_TBL} (ID INT, V INT)"), ())
        .await?;
    drop(conn);
    pool.close().await?;
    pool.wait_closed().await;
    Ok(())
}

/// 收尾：删掉 D/E 组建的表，不给下次跑留垃圾。
async fn drop_cache_table(cfg: &DmConfig) {
    if let Ok(pool) = Pool::connect_with(cfg.clone(), ExecutorConfig::for_pool(1)).await {
        if let Ok(mut conn) = PooledConn::acquire(&pool).await {
            let _ = conn.execute(&format!("drop table {CACHE_TBL}"), ()).await;
        }
        let _ = pool.close().await;
        pool.wait_closed().await;
    }
}

async fn run() -> Result<(), DmError> {
    let Some(cfg) = config() else {
        println!("跳过：未配置 DM_TEST_DSN / DM_SERVER（真机基准需要一台可达的达梦 DM8）");
        return Ok(());
    };

    println!("== AC-12 / 真机 `{SQL}`（单位 µs） ==");
    println!(
        "热身 {WARMUP} 轮不计入；判据：全栈 P99 < {BUDGET_P99_US} µs、自研开销（B−A2）< {BUDGET_OVERHEAD_US} µs"
    );

    // ---- A/A2 裸驱动 ----
    let raw = raw_path(&cfg)?;
    let raw_alloc = raw_path_alloc_each(&cfg)?;

    // ---- B/C 池与查询层 ----
    let pool = Pool::connect_with(cfg.clone(), ExecutorConfig::for_pool(4)).await?;

    let mut conn = PooledConn::acquire(&pool).await?;
    for _ in 0..WARMUP {
        conn.query_one(SQL, ()).await?;
    }
    let mut samples = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let start = Instant::now();
        conn.query_one(SQL, ()).await?;
        samples.push(start.elapsed());
    }
    let stack = Stats::of(samples);
    drop(conn);

    for _ in 0..(WARMUP / 5) {
        let mut conn = PooledConn::acquire(&pool).await?;
        conn.query_one(SQL, ()).await?;
    }
    let mut samples = Vec::with_capacity(ITERS_ACQUIRE);
    for _ in 0..ITERS_ACQUIRE {
        let start = Instant::now();
        let mut conn = PooledConn::acquire(&pool).await?;
        conn.query_one(SQL, ()).await?;
        drop(conn);
        samples.push(start.elapsed());
    }
    let acquire = Stats::of(samples);

    println!(
        "\n{:<22} {:>7}  {:>9} {:>9} {:>9} {:>9}",
        "口径", "轮数", "P50", "P95", "P99", "max"
    );
    println!("{}", "-".repeat(74));
    row("A 裸驱动", ITERS, &raw);
    row("A2 裸驱动+每轮新语句", ITERS, &raw_alloc);
    row("B 全栈（池+查询层）", ITERS, &stack);
    row("C 每次借出+查询", ITERS_ACQUIRE, &acquire);

    let overhead_reuse = (stack.p50 - raw.p50).max(0.0);
    let overhead = (stack.p50 - raw_alloc.p50).max(0.0);
    let pass_p99 = stack.p99 < BUDGET_P99_US;
    let pass_overhead = overhead < BUDGET_OVERHEAD_US;

    println!("\n-- 判定（AC-12 后半）--");
    println!(
        "真机 `select 1` 全栈 P99 = {:.3} µs，判据 < {BUDGET_P99_US} µs ⇒ {}",
        stack.p99,
        if pass_p99 { "达标" } else { "**未达标**" }
    );
    println!(
        "自研开销（B 的 P50 {:.3} − A2 的 P50 {:.3}，口径对齐）= {:.3} µs，判据 < {BUDGET_OVERHEAD_US} µs ⇒ {}",
        stack.p50,
        raw_alloc.p50,
        overhead,
        if pass_overhead {
            "达标"
        } else {
            "**未达标**"
        }
    );
    println!(
        "参考：B − A（A 复用一条句柄）= {:.3} µs；差额里含「B 每条语句都重新分配句柄 + `SQLPrepare` + `SQLExecute`」这项语句生命周期差异",
        overhead_reuse
    );
    println!(
        "池握手（C 的 P50 {:.3} − B 的 P50 {:.3}）≈ {:.3} µs",
        acquire.p50,
        stack.p50,
        (acquire.p50 - stack.p50).max(0.0)
    );

    let stats = pool.stats();
    println!(
        "池指标：size={} in_use={} idle={} 建连={} 借出={} 归还={}",
        stats.size, stats.in_use, stats.idle, stats.created, stats.acquired, stats.released
    );

    pool.close().await?;
    pool.wait_closed().await;

    // ---- D/E DML：语句缓存的收益（CAP-C-11）----
    prepare_cache_table(&cfg).await?;
    let (dml_cached, hits, misses, size) = pool_dml(cfg.clone(), 16).await?;
    let (dml_uncached, _, _, _) = pool_dml(cfg.clone(), 0).await?;
    drop_cache_table(&cfg).await;

    println!("\n== 语句缓存收益（CAP-C-11，真机 `insert … values (?, ?)`，单位 µs） ==");
    row("D 全栈 DML（缓存开）", ITERS_DML, &dml_cached);
    row("E 全栈 DML（缓存关）", ITERS_DML, &dml_uncached);
    let cache_gain = (dml_uncached.p50 - dml_cached.p50).max(0.0);
    let cache_gain_pct = if dml_uncached.p50 > 0.0 {
        cache_gain / dml_uncached.p50 * 100.0
    } else {
        0.0
    };
    println!(
        "语句缓存收益（E 的 P50 {:.3} − D 的 P50 {:.3}）= {:.3} µs ≈ {:.1}%；D 组缓存 hits={hits} misses={misses} size={size}",
        dml_uncached.p50, dml_cached.p50, cache_gain, cache_gain_pct
    );

    if std::env::args().any(|arg| arg == "--strict") && !(pass_p99 && pass_overhead) {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "AC-12 后半未达标：P99 {:.3} µs（判据 {BUDGET_P99_US}）、自研开销 {:.3} µs（判据 {BUDGET_OVERHEAD_US}）",
                stack.p99, overhead
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
