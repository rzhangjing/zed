//! 现场性能基准：宽表（100 字段）与大字段表（20 字段，含 1 KiB–200 KiB 二进制）。
//!
//! 这个例子不是门禁，是**测量工具**：它只打印结果，不做断言。所有数字都走中间件
//! 的公开面（`DmPool` + `PooledConn`/`RowStream`/`execute_lob`），不绕过任何一层。
//!
//! # 用法
//!
//! ```text
//! set DM_TEST_DSN=DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA
//! cargo run --release -p dm-query --example perf_bench -- <stage> [flags]
//! ```
//!
//! 阶段：`ddl` / `calib1` / `insert1` / `read1` / `rand1` / `ddl2` / `insert2` /
//! `read2` / `rand2` / `size` / `drop` / `all`
//!
//! 参数：`--rows N`（宽表行数，默认 1000 万）、`--blob-rows N`（大字段表行数，默认 10 万）、
//! `--batch N`（数组绑定每批行数，默认 500）、`--writers N`（并发写连接数，默认 4）、
//! `--ops N`（随机阶段操作数，默认 1 万）、`--txn N`（BLOB 批量插入时每 N 行提交一次，0 = 逐行自动提交）、
//! `--payload-min/--payload-max`（字节，默认 1024/204800）、
//! `--seed N`、`--keep`（不删表）、`--schema S`（默认 DBTESTA）。
//!
//! 输出以 `METRIC|` 开头，便于从日志里抓。

use std::future::poll_fn;
use std::hint::black_box;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dm_core::{DmConfig, DmValue, PoolConfig};
use dm_pool::{DmPool, ExecutorConfig};
// 走「默认后端」别名：Round 41 起默认是 DPI（`--no-default-features` 则回退 ODBC），
// 这样同一份基准程序可以在新旧两个后端上跑出可直接对比的数字（Round 42 复测就是这么用的）。
use dm_query::DefaultBackend as OdbcBackend;
use dm_query::{DmError, DmErrorKind, LobBytes, LobKind, LobParam, PooledConn, RowStream};
use futures_core::Stream;

// ---------------------------------------------------------------------------
// 参数
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Opts {
    stage: String,
    schema: String,
    rows: u64,
    blob_rows: u64,
    batch: usize,
    writers: usize,
    ops: u64,
    txn: usize,
    payload_min: usize,
    payload_max: usize,
    seed: u64,
    keep: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            stage: "all".into(),
            schema: "DBTESTA".into(),
            rows: 10_000_000,
            blob_rows: 100_000,
            batch: 500,
            writers: 4,
            ops: 10_000,
            txn: 0,
            payload_min: 1024,
            payload_max: 204_800,
            seed: 0x5EED_1234_ABCD_0001,
            keep: false,
        }
    }
}

fn next_num<T: std::str::FromStr>(it: &mut impl Iterator<Item = String>, key: &str) -> T
where
    T::Err: std::fmt::Display,
{
    let v = it.next().unwrap_or_else(|| panic!("{key} 需要一个值"));
    v.parse()
        .unwrap_or_else(|e| panic!("{key} 的值 {v:?} 不是合法数字：{e}"))
}

fn parse_args() -> Opts {
    let mut o = Opts::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--rows" => o.rows = next_num(&mut it, "--rows"),
            "--blob-rows" => o.blob_rows = next_num(&mut it, "--blob-rows"),
            "--batch" => o.batch = next_num(&mut it, "--batch"),
            "--writers" => o.writers = next_num(&mut it, "--writers"),
            "--ops" => o.ops = next_num(&mut it, "--ops"),
            "--txn" => o.txn = next_num(&mut it, "--txn"),
            "--payload-min" => o.payload_min = next_num(&mut it, "--payload-min"),
            "--payload-max" => o.payload_max = next_num(&mut it, "--payload-max"),
            "--seed" => o.seed = next_num(&mut it, "--seed"),
            "--schema" => {
                o.schema = it.next().expect("--schema 需要一个值");
            }
            "--keep" => o.keep = true,
            other => o.stage = other.trim_start_matches("--").to_string(),
        }
    }
    assert!(o.batch > 0, "--batch 必须大于 0");
    assert!(o.writers > 0, "--writers 必须大于 0");
    assert!(o.payload_min > 0 && o.payload_min <= o.payload_max);
    o
}

// ---------------------------------------------------------------------------
// 计时与统计
// ---------------------------------------------------------------------------

fn report_throughput(name: &str, wall: Duration, rows: u64, bytes: u64) {
    let s = wall.as_secs_f64();
    let rps = if s > 0.0 { rows as f64 / s } else { 0.0 };
    let mbps = if s > 0.0 { bytes as f64 / s / 1e6 } else { 0.0 };
    println!(
        "METRIC|{name}|wall_s={:.3}|rows={rows}|rows_per_s={:.0}|bytes={bytes}|mb_per_s={:.1}",
        s, rps, mbps
    );
}

fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn report_latency(name: &str, mut lat: Vec<u64>) {
    if lat.is_empty() {
        println!("METRIC|{name}|n=0");
        return;
    }
    let total: u64 = lat.iter().sum();
    lat.sort_unstable();
    let n = lat.len() as f64;
    println!(
        "METRIC|{name}|n={}|total_s={:.3}|mean_us={:.1}|p50_us={:.1}|p95_us={:.1}|p99_us={:.1}|min_us={:.1}|max_us={:.1}",
        lat.len(),
        total as f64 / 1e9,
        total as f64 / 1e3 / n,
        pct(&lat, 0.50) as f64 / 1e3,
        pct(&lat, 0.95) as f64 / 1e3,
        pct(&lat, 0.99) as f64 / 1e3,
        lat[0] as f64 / 1e3,
        lat[lat.len() - 1] as f64 / 1e3,
    );
}

// ---------------------------------------------------------------------------
// 伪随机（SplitMix64；不引第三方依赖）
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
    fn fill(&mut self, buf: &mut [u8]) {
        let mut i = 0;
        while i < buf.len() {
            let v = self.next_u64().to_le_bytes();
            let n = (buf.len() - i).min(8);
            buf[i..i + n].copy_from_slice(&v[..n]);
            i += n;
        }
    }
}

// ---------------------------------------------------------------------------
// 表结构
// ---------------------------------------------------------------------------

const T1: &str = "PERF_WIDE";
const T2: &str = "PERF_BLOB";

/// 宽表 100 列：`ID` + 90 个 `BIGINT` + 5 个 `INT` + 2 个 `DOUBLE` + 1 个 `VARCHAR(32)` + 1 个 `TIMESTAMP`。
fn wide_ddl() -> String {
    let mut cols = vec!["ID BIGINT NOT NULL PRIMARY KEY".to_string()];
    for i in 1..=90 {
        cols.push(format!("C{i:03} BIGINT"));
    }
    for i in 91..=95 {
        cols.push(format!("C{i:03} INT"));
    }
    cols.push("C096 DOUBLE".into());
    cols.push("C097 DOUBLE".into());
    cols.push("C098 VARCHAR(32)".into());
    cols.push("C099 TIMESTAMP".into());
    format!("create table {T1} (\n  {}\n)", cols.join(",\n  "))
}

fn wide_insert_sql() -> String {
    let mut names = vec!["ID".to_string()];
    for i in 1..=99 {
        names.push(format!("C{i:03}"));
    }
    let marks: Vec<&str> = (0..100).map(|_| "?").collect();
    format!(
        "insert into {T1} ({}) values ({})",
        names.join(", "),
        marks.join(", ")
    )
}

fn wide_row_values(id: i64, rng: &mut Rng, ts: chrono::NaiveDateTime) -> Vec<DmValue> {
    let mut v = Vec::with_capacity(100);
    v.push(DmValue::BigInt(id));
    for _ in 0..90 {
        v.push(DmValue::BigInt(rng.below(1_000_000) as i64));
    }
    for _ in 0..5 {
        v.push(DmValue::Int(rng.below(100_000) as i32));
    }
    v.push(DmValue::Double(rng.below(1_000_000) as f64 / 7.0));
    v.push(DmValue::Double(rng.below(1_000_000) as f64 / 11.0));
    v.push(DmValue::Text(format!("v{:016x}", rng.next_u64())));
    v.push(DmValue::Timestamp(ts));
    v
}

/// 大字段表 20 列：`ID` + 18 个标量 + `PAYLOAD BLOB`。
fn blob_ddl() -> String {
    format!(
        "create table {T2} (\n  \
         ID BIGINT NOT NULL PRIMARY KEY,\n  \
         K BIGINT,\n  T TIMESTAMP,\n  \
         S1 VARCHAR(64),\n  S2 VARCHAR(64),\n  S3 VARCHAR(64),\n  \
         I1 INT,\n  I2 INT,\n  I3 INT,\n  I4 INT,\n  \
         D1 DOUBLE,\n  D2 DOUBLE,\n  \
         N1 DECIMAL(18,4),\n  N2 DECIMAL(18,4),\n  \
         F1 FLOAT,\n  F2 FLOAT,\n  \
         B1 INT,\n  B2 INT,\n  B3 BIGINT,\n  \
         PAYLOAD BLOB\n)"
    )
}

/// `PAYLOAD` 是第 20 个 `?`（0 起算下标 19），前面 19 个都是标量列。
const BLOB_INSERT_SQL: &str = "insert into PERF_BLOB \
 (ID, K, T, S1, S2, S3, I1, I2, I3, I4, D1, D2, N1, N2, F1, F2, B1, B2, B3, PAYLOAD) \
 values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
const BLOB_LOB_INDEX: usize = 19;

fn blob_scalar_values(id: i64, rng: &mut Rng, ts: chrono::NaiveDateTime) -> Vec<DmValue> {
    vec![
        DmValue::BigInt(id),
        DmValue::BigInt(rng.below(1_000_000) as i64),
        DmValue::Timestamp(ts),
        DmValue::Text(format!("s1-{:012x}", rng.next_u64())),
        DmValue::Text(format!("s2-{:012x}", rng.next_u64())),
        DmValue::Text(format!("s3-{:012x}", rng.next_u64())),
        DmValue::Int(rng.below(100_000) as i32),
        DmValue::Int(rng.below(100_000) as i32),
        DmValue::Int(rng.below(100_000) as i32),
        DmValue::Int(rng.below(100_000) as i32),
        DmValue::Double(rng.below(1_000_000) as f64 / 3.0),
        DmValue::Double(rng.below(1_000_000) as f64 / 13.0),
        DmValue::Text("1234567890.1234".into()),
        DmValue::Text("9876543210.4321".into()),
        DmValue::Real(rng.below(10_000) as f32 / 9.0),
        DmValue::Real(rng.below(10_000) as f32 / 17.0),
        DmValue::Int(rng.below(1000) as i32),
        DmValue::Int(rng.below(1000) as i32),
        DmValue::BigInt(rng.below(1_000_000) as i64),
    ]
}

fn payload_len(o: &Opts, rng: &mut Rng) -> usize {
    let span = (o.payload_max - o.payload_min) as u64;
    o.payload_min + rng.below(span + 1) as usize
}

fn payload_bytes(o: &Opts, rng: &mut Rng) -> Vec<u8> {
    let n = payload_len(o, rng);
    let mut buf = vec![0u8; n];
    rng.fill(&mut buf);
    buf
}

fn ts_at(secs: i64) -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("合法日期")
        .and_hms_opt(0, 0, 0)
        .expect("合法时间")
        + chrono::Duration::seconds(secs)
}

// ---------------------------------------------------------------------------
// 连接
// ---------------------------------------------------------------------------

fn base_config(o: &Opts) -> DmConfig {
    let dsn = std::env::var("DM_TEST_DSN").unwrap_or_else(|_| {
        "DRIVER={DM8 ODBC DRIVER};SERVER=127.0.0.1;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA".to_string()
    });
    let mut cfg = DmConfig::from_connection_string(&dsn).expect("解析 DM_TEST_DSN 失败");
    cfg.schema = Some(o.schema.clone());
    cfg.slow_query_threshold = Duration::from_secs(3600);
    cfg.pool = PoolConfig {
        max_size: o.writers.max(4) + 4,
        min_idle: 1,
        ..PoolConfig::default()
    };
    cfg
}

async fn open_pool(o: &Opts) -> Result<Arc<DmPool<OdbcBackend>>, DmError> {
    let cfg = base_config(o);
    let max = cfg.pool.max_size;
    let exec = ExecutorConfig::for_pool(max);
    let pool = DmPool::connect_with(cfg, exec).await?;
    Ok(Arc::new(pool))
}

async fn close_pool(pool: Arc<DmPool<OdbcBackend>>) {
    if let Ok(pool) = Arc::try_unwrap(pool) {
        let _ = pool.close().await;
        pool.wait_closed().await;
    }
}

async fn scalar_i64(conn: &mut PooledConn<OdbcBackend>, sql: &str) -> i64 {
    conn.query_one(sql, ())
        .await
        .expect("标量查询失败")
        .expect("标量查询必须有一行")
        .get::<i64>(0)
        .expect("标量必须是整数")
}

async fn next_row(stream: &mut RowStream<OdbcBackend>) -> Option<Result<dm_query::Row, DmError>> {
    poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}

// ---------------------------------------------------------------------------
// 阶段
// ---------------------------------------------------------------------------

async fn stage_ddl(pool: &Arc<DmPool<OdbcBackend>>, _o: &Opts) -> Result<(), DmError> {
    let mut c = PooledConn::acquire(pool).await?;
    for t in [T1, T2] {
        c.execute(&format!("drop table if exists {t}"), ()).await?;
    }
    c.execute(&wide_ddl(), ()).await?;
    c.execute(&blob_ddl(), ()).await?;
    println!("OK|ddl|建表完成：{T1}（100 列）、{T2}（20 列，含 BLOB）");
    Ok(())
}

async fn stage_insert1(pool: &Arc<DmPool<OdbcBackend>>, o: &Opts) -> Result<(), DmError> {
    let sql = wide_insert_sql();
    let rows = o.rows;
    let batch = o.batch as u64;
    let writers = o.writers as u64;
    let started = Instant::now();
    let mut handles = Vec::new();
    for w in 0..writers {
        let pool = Arc::clone(pool);
        let sql = sql.clone();
        let o = o.clone();
        handles.push(tokio::spawn(async move {
            let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");
            let from = rows * w / writers + 1;
            let to = rows * (w + 1) / writers;
            let mut rng = Rng::new(o.seed ^ (w << 32));
            let mut next = from;
            let mut written: u64 = 0;
            let mut pending: Vec<Vec<DmValue>> = Vec::with_capacity(o.batch);
            let mut since_log = 0u64;
            while next <= to {
                let take = batch.min(to - next + 1);
                pending.clear();
                for i in 0..take {
                    pending.push(wide_row_values(
                        (next + i) as i64,
                        &mut rng,
                        ts_at((next + i) as i64 % 100_000),
                    ));
                }
                let out = conn.execute_many(&sql, pending.drain(..)).await?;
                if let Some(at) = out.failed_at {
                    return Err(DmErrorKind::Config(format!(
                        "worker {w} 第 {at} 组失败：affected={} executed={}",
                        out.affected, out.executed
                    ))
                    .into());
                }
                if out.affected != take {
                    return Err(DmErrorKind::Config(format!(
                        "worker {w} affected={} 期望 {take}",
                        out.affected
                    ))
                    .into());
                }
                written += take;
                since_log += take;
                next += take;
                if since_log >= 200_000 {
                    since_log = 0;
                    let s = started.elapsed().as_secs_f64();
                    let span = to - from + 1;
                    println!(
                        "PROGRESS|insert1|worker={w}|{written}/{span}|{:.0} rows/s|{:.1} s",
                        written as f64 / s.max(1e-9),
                        s
                    );
                }
            }
            Ok::<u64, DmError>(written)
        }));
    }
    let mut total = 0u64;
    for h in handles {
        total += h.await.expect("worker panic")?;
    }
    let wall = started.elapsed();
    // 行长粗估：100 列里 90×8 + 5×4 + 2×8 + 32 + 8 = 804 字节（服务端另加行头/对齐）。
    report_throughput("insert1", wall, total, total * 820);
    assert_eq!(total, o.rows, "写入行数与 --rows 不一致");
    Ok(())
}

async fn stage_read1(pool: &Arc<DmPool<OdbcBackend>>, o: &Opts) -> Result<(), DmError> {
    let conn = PooledConn::acquire(pool).await?;
    let mut stream = conn
        .fetch_stream(&format!("select * from {T1}"), ())
        .await?;
    let cols = stream.columns().len();
    let started = Instant::now();
    let mut rows: u64 = 0;
    let mut sum: i64 = 0;
    let mut text_len: u64 = 0;
    while let Some(item) = next_row(&mut stream).await {
        let row = item.expect("读取失败");
        let id = row.get::<i64>(0).expect("ID");
        let c091 = row.get::<i32>(91).expect("C091");
        let s = row.get::<String>(98).expect("C098");
        sum = sum.wrapping_add(id).wrapping_add(c091 as i64);
        text_len += s.len() as u64;
        rows += 1;
    }
    let wall = started.elapsed();
    black_box(sum);
    black_box(text_len);
    println!("INFO|read1|列数={cols}|checksum={sum}|text_bytes={text_len}");
    report_throughput("read1", wall, rows, rows * 820);
    assert_eq!(rows, o.rows, "读到的行数与 --rows 不一致");
    Ok(())
}

/// 从 `[base, base+n)` 里取 `n` 个互不重复的随机键（Fisher–Yates）。
fn shuffled_keys(rng: &mut Rng, base: i64, n: u64) -> Vec<i64> {
    let mut v: Vec<i64> = (0..n).map(|i| base + i as i64).collect();
    for i in (1..v.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        v.swap(i, j);
    }
    v
}

async fn stage_rand1(pool: &Arc<DmPool<OdbcBackend>>, o: &Opts) -> Result<(), DmError> {
    let mut conn = PooledConn::acquire(pool).await?;
    let mut rng = Rng::new(o.seed ^ 0xA1);
    let ops = o.ops;

    // ① 随机读：主键等值，取整行。
    let mut lat = Vec::with_capacity(ops as usize);
    for _ in 0..ops {
        let id = 1 + rng.below(o.rows) as i64;
        let t0 = Instant::now();
        let row = conn
            .query_one(&format!("select * from {T1} where ID = ?"), (id,))
            .await?;
        black_box(row.map(|r| r.len()));
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand1_read", lat);

    // ② 随机插入：主键散落在 [rows+1, rows+ops]，逐条插入（单行语句，非批量）。
    let keys = shuffled_keys(&mut rng, o.rows as i64 + 1, ops);
    let sql = wide_insert_sql();
    let mut lat = Vec::with_capacity(keys.len());
    for k in keys.iter().copied() {
        let vals = wide_row_values(k, &mut rng, ts_at(k % 100_000));
        let t0 = Instant::now();
        let n = conn.execute(&sql, vals).await?;
        black_box(n);
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand1_insert", lat);

    // ③ 随机修改：按主键改 2 列。
    let mut lat = Vec::with_capacity(ops as usize);
    for _ in 0..ops {
        let id = 1 + rng.below(o.rows) as i64;
        let t0 = Instant::now();
        let n = conn
            .execute(
                &format!("update {T1} set C091 = ?, C098 = ? where ID = ?"),
                (
                    rng.below(100_000) as i32,
                    format!("u{:016x}", rng.next_u64()),
                    id,
                ),
            )
            .await?;
        black_box(n);
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand1_update", lat);

    // ④ 随机删除：删的是 ② 插进去的那批键（保证命中）。
    let mut lat = Vec::with_capacity(keys.len());
    for k in keys.iter().copied() {
        let t0 = Instant::now();
        let n = conn
            .execute(&format!("delete from {T1} where ID = ?"), (k,))
            .await?;
        black_box(n);
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand1_delete", lat);
    Ok(())
}

async fn stage_insert2(pool: &Arc<DmPool<OdbcBackend>>, o: &Opts) -> Result<(), DmError> {
    let rows = o.blob_rows;
    let writers = o.writers as u64;
    let txn_rows = o.txn as u64;
    if txn_rows > 0 {
        println!("INFO|insert2|事务模式：每 {txn_rows} 行提交一次");
    } else {
        println!("INFO|insert2|自动提交模式：逐行提交（默认）");
    }
    let started = Instant::now();
    let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut handles = Vec::new();
    for w in 0..writers {
        let pool = Arc::clone(pool);
        let bytes = Arc::clone(&bytes);
        let o = o.clone();
        handles.push(tokio::spawn(async move {
            let from = rows * w / writers + 1;
            let to = rows * (w + 1) / writers;
            let mut rng = Rng::new(o.seed ^ 0xB2 ^ (w << 32));
            let mut written: u64 = 0;
            let mut local_bytes: u64 = 0;
            let mut since_log = 0u64;
            let mut next = from;
            if txn_rows == 0 {
                let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");
                while next <= to {
                    let (vals, lob, n) = blob_row(&o, &mut rng, next);
                    let affected = conn.execute_lob(BLOB_INSERT_SQL, vals, lob).await?;
                    if affected != 1 {
                        return Err(
                            DmErrorKind::Config(format!("BLOB 插入 affected={affected}")).into(),
                        );
                    }
                    written += 1;
                    local_bytes += n;
                    since_log += 1;
                    next += 1;
                    log_insert2_progress(w, &started, written, local_bytes, &mut since_log);
                }
            } else {
                while next <= to {
                    let take = txn_rows.min(to - next + 1);
                    let conn = PooledConn::acquire(&pool).await.expect("借连接失败");
                    let mut txn = conn.begin().await?;
                    for _ in 0..take {
                        let (vals, lob, n) = blob_row(&o, &mut rng, next);
                        let affected = txn.conn().execute_lob(BLOB_INSERT_SQL, vals, lob).await?;
                        if affected != 1 {
                            return Err(DmErrorKind::Config(format!(
                                "BLOB 插入 affected={affected}"
                            ))
                            .into());
                        }
                        written += 1;
                        local_bytes += n;
                        since_log += 1;
                        next += 1;
                        log_insert2_progress(w, &started, written, local_bytes, &mut since_log);
                    }
                    txn.commit().await?;
                }
            }
            bytes.fetch_add(local_bytes, std::sync::atomic::Ordering::Relaxed);
            Ok::<u64, DmError>(written)
        }));
    }
    let mut total = 0u64;
    for h in handles {
        total += h.await.expect("worker panic")?;
    }
    let wall = started.elapsed();
    report_throughput(
        "insert2",
        wall,
        total,
        bytes.load(std::sync::atomic::Ordering::Relaxed),
    );
    assert_eq!(total, o.blob_rows, "写入行数与 --blob-rows 不一致");
    Ok(())
}

fn log_insert2_progress(
    w: u64,
    started: &Instant,
    written: u64,
    local_bytes: u64,
    since_log: &mut u64,
) {
    if *since_log >= 5_000 {
        *since_log = 0;
        let s = started.elapsed().as_secs_f64();
        println!(
            "PROGRESS|insert2|worker={w}|{written} rows|{:.0} rows/s|{:.1} MB/s|{:.1} s",
            written as f64 / s.max(1e-9),
            local_bytes as f64 / 1e6 / s.max(1e-9),
            s
        );
    }
}

/// 生成第 `id` 行 BLOB 表的参数、LOB 参数和 payload 长度。
fn blob_row(o: &Opts, rng: &mut Rng, id: u64) -> (Vec<DmValue>, LobParam<LobBytes>, u64) {
    let vals = blob_scalar_values(id as i64, rng, ts_at(id as i64 % 100_000));
    let payload = payload_bytes(o, rng);
    let n = payload.len() as u64;
    let lob = LobParam::new(BLOB_LOB_INDEX, LobKind::Binary, LobBytes::new(payload));
    (vals, lob, n)
}

async fn stage_read2(pool: &Arc<DmPool<OdbcBackend>>, o: &Opts) -> Result<(), DmError> {
    let conn = PooledConn::acquire(pool).await?;
    let mut stream = conn
        .fetch_stream(&format!("select * from {T2}"), ())
        .await?;
    let cols = stream.columns().len();
    let started = Instant::now();
    let mut rows: u64 = 0;
    let mut bytes: u64 = 0;
    let mut sum: i64 = 0;
    while let Some(item) = next_row(&mut stream).await {
        let row = item.expect("读取失败");
        let id = row.get::<i64>(0).expect("ID");
        let payload = row.get::<Vec<u8>>(19).expect("PAYLOAD");
        bytes += payload.len() as u64;
        sum = sum.wrapping_add(id).wrapping_add(payload.len() as i64);
        rows += 1;
    }
    let wall = started.elapsed();
    black_box(sum);
    println!("INFO|read2|列数={cols}|checksum={sum}|payload_bytes={bytes}");
    report_throughput("read2", wall, rows, bytes);
    assert_eq!(rows, o.blob_rows, "读到的行数与 --blob-rows 不一致");
    Ok(())
}

async fn stage_rand2(pool: &Arc<DmPool<OdbcBackend>>, o: &Opts) -> Result<(), DmError> {
    let mut conn = PooledConn::acquire(pool).await?;
    let mut rng = Rng::new(o.seed ^ 0xC3);
    let ops = o.ops;

    // ① 随机读（含 BLOB）。
    let mut lat = Vec::with_capacity(ops as usize);
    let mut bytes: u64 = 0;
    for _ in 0..ops {
        let id = 1 + rng.below(o.blob_rows) as i64;
        let t0 = Instant::now();
        let row = conn
            .query_one(&format!("select * from {T2} where ID = ?"), (id,))
            .await?;
        if let Some(r) = &row {
            bytes += r.get::<Vec<u8>>(19).expect("PAYLOAD").len() as u64;
        }
        black_box(row.map(|r| r.len()));
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand2_read", lat);
    println!("INFO|rand2_read|读到的 payload 字节={bytes}");

    // ② 随机插入（含随机大小 BLOB）。
    let keys = shuffled_keys(&mut rng, o.blob_rows as i64 + 1, ops);
    let mut lat = Vec::with_capacity(keys.len());
    let mut ins_bytes: u64 = 0;
    for k in keys.iter().copied() {
        let vals = blob_scalar_values(k, &mut rng, ts_at(k % 100_000));
        let payload = payload_bytes(o, &mut rng);
        ins_bytes += payload.len() as u64;
        let lob = LobParam::new(BLOB_LOB_INDEX, LobKind::Binary, LobBytes::new(payload));
        let t0 = Instant::now();
        let n = conn.execute_lob(BLOB_INSERT_SQL, vals, lob).await?;
        black_box(n);
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand2_insert", lat);
    println!("INFO|rand2_insert|写入 payload 字节={ins_bytes}");

    // ③ 随机修改：改 S1 与 B3（不换 BLOB，避免与插入混在一起）。
    let mut lat = Vec::with_capacity(ops as usize);
    for _ in 0..ops {
        let id = 1 + rng.below(o.blob_rows) as i64;
        let t0 = Instant::now();
        let n = conn
            .execute(
                &format!("update {T2} set S1 = ?, B3 = ? where ID = ?"),
                (
                    format!("u{:016x}", rng.next_u64()),
                    rng.below(1_000_000) as i64,
                    id,
                ),
            )
            .await?;
        black_box(n);
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand2_update", lat);

    // ④ 随机修改（**带二进制列**）：把 PAYLOAD 整个换成新的随机长度（1k–200k）的数据，
    //    同时改两个标量列。用 `execute_lob` 走大字段写路径（`DATA_AT_EXEC` / `put_data`），
    //    这才是「大字段表上的修改」的真实成本。
    let mut lat = Vec::with_capacity(ops as usize);
    let mut upd_bytes: u64 = 0;
    for _ in 0..ops {
        let id = 1 + rng.below(o.blob_rows) as i64;
        let payload = payload_bytes(o, &mut rng);
        upd_bytes += payload.len() as u64;
        // 语句是 `S1, B3, PAYLOAD, ID` ⇒ PAYLOAD 是 0 起算的第 2 个参数。
        let lob = LobParam::new(2, LobKind::Binary, LobBytes::new(payload));
        let t0 = Instant::now();
        let n = conn
            .execute_lob(
                &format!("update {T2} set S1 = ?, B3 = ?, PAYLOAD = ? where ID = ?"),
                (
                    format!("v{:016x}", rng.next_u64()),
                    rng.below(1_000_000) as i64,
                    id,
                ),
                lob,
            )
            .await?;
        black_box(n);
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand2_update_lob", lat);
    println!("INFO|rand2_update_lob|写入 payload 字节={upd_bytes}");

    // ④ 随机删除：删 ② 插进去的那批键。
    let mut lat = Vec::with_capacity(keys.len());
    for k in keys.iter().copied() {
        let t0 = Instant::now();
        let n = conn
            .execute(&format!("delete from {T2} where ID = ?"), (k,))
            .await?;
        black_box(n);
        lat.push(t0.elapsed().as_nanos() as u64);
    }
    report_latency("rand2_delete", lat);
    Ok(())
}

async fn stage_size(pool: &Arc<DmPool<OdbcBackend>>, _o: &Opts) -> Result<(), DmError> {
    let mut conn = PooledConn::acquire(pool).await?;
    for t in [T1, T2] {
        let rows = scalar_i64(&mut conn, &format!("select count(*) from {t}")).await;
        let seg = conn
            .query_one(
                "select cast(nvl(sum(bytes), 0) as bigint) from user_segments where segment_name = ?",
                (t,),
            )
            .await?
            .and_then(|r| r.get::<i64>(0).ok())
            .unwrap_or(0);
        println!(
            "METRIC|size|table={t}|rows={rows}|segment_bytes={seg}|segment_mb={:.1}",
            seg as f64 / 1e6
        );
    }
    Ok(())
}

async fn stage_drop(pool: &Arc<DmPool<OdbcBackend>>, _o: &Opts) -> Result<(), DmError> {
    let mut c = PooledConn::acquire(pool).await?;
    for t in [T1, T2] {
        c.execute(&format!("drop table if exists {t}"), ()).await?;
    }
    println!("OK|drop|已删除 {T1} 与 {T2}");
    Ok(())
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let o = parse_args();
    println!(
        "CONFIG|stage={}|schema={}|rows={}|blob_rows={}|batch={}|writers={}|ops={}|txn={}|payload={}..{}|seed={}",
        o.stage,
        o.schema,
        o.rows,
        o.blob_rows,
        o.batch,
        o.writers,
        o.ops,
        o.txn,
        o.payload_min,
        o.payload_max,
        o.seed
    );
    let pool = match open_pool(&o).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("建池失败：{e}");
            std::process::exit(2);
        }
    };
    let total_started = Instant::now();
    let result = run(&o, &pool).await;
    match &result {
        Ok(()) => println!(
            "OK|{}|总耗时 {:.1} s",
            o.stage,
            total_started.elapsed().as_secs_f64()
        ),
        Err(e) => eprintln!("FAIL|{}|{e}", o.stage),
    }
    close_pool(Arc::clone(&pool)).await;
    if result.is_err() {
        std::process::exit(1);
    }
}

async fn run(o: &Opts, pool: &Arc<DmPool<OdbcBackend>>) -> Result<(), DmError> {
    let stages: Vec<&str> = match o.stage.as_str() {
        "all" => vec![
            "ddl", "size0", "insert1", "read1", "rand1", "size1", "ddl2", "insert2", "read2",
            "rand2", "size",
        ],
        other => vec![other],
    };
    for s in stages {
        let t0 = Instant::now();
        match s {
            "ddl" => stage_ddl(pool, o).await?,
            "ddl2" => println!("OK|ddl2|（ddl 已一并建好，跳过）"),
            "size0" => stage_size(pool, o).await?,
            "size1" => stage_size(pool, o).await?,
            "insert1" => stage_insert1(pool, o).await?,
            "read1" => stage_read1(pool, o).await?,
            "rand1" => stage_rand1(pool, o).await?,
            "insert2" => stage_insert2(pool, o).await?,
            "read2" => stage_read2(pool, o).await?,
            "rand2" => stage_rand2(pool, o).await?,
            "size" => stage_size(pool, o).await?,
            "drop" => stage_drop(pool, o).await?,
            other => {
                eprintln!("未知阶段：{other}");
                std::process::exit(2);
            }
        }
        println!("STAGE|{s}|{:.1} s", t0.elapsed().as_secs_f64());
    }
    if !o.keep && o.stage == "all" {
        println!("提示：加了 --keep 才保表；本轮的 all 不自动删表");
    }
    Ok(())
}
