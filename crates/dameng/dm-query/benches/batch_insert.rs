//! # V-07：批量写走数组绑定的增速（真机基准）
//!
//! 设计方案 §12.3 的 T6.6 要求把 §4.7 / §5.10 的 33× 目标**固化**成可复跑的基准，
//! 而不是只留一次手测记录。本文件就是那条记录的执行体。
//!
//! ## 为什么不用 criterion
//!
//! 本项目的惯例是能不加依赖就不加（同 M4 未引入 `dhat` 的理由）。这里要回答的问题
//! 很窄——「同样插 N 行，列式参数数组比逐行循环快几倍」——没有统计显著性的诉求，
//! `std::time::Instant` 取中位数就够了。所以用 `harness = false` 自己当 main。
//!
//! ## 两条被测路径必须只差在「绑定方式」上
//!
//! * **逐行**：`execute_many` **之前**的写法——每组参数 `bind_params` + `execute`
//!   各一次，1000 行就是 1000 次往返；
//! * **数组**：`execute_many`，一次 `SQLSetStmtAttr(PARAMSET_SIZE)` +
//!   每列一次 `SQLBindParameter` + 一次 `SQLExecute`。
//!
//! 两边用**同一条 SQL、同一张表、同一条连接**，唯一变量是绑定方式；否则比出来的
//! 数字说明不了 §4.7 的结论。计时**不含**建连与清表：`Instant` 只掐在真正写数据的
//! 那一段上，否则一次建连（几十毫秒）会把数组绑定那一列淹掉。
//!
//! ## 跑法
//!
//! ```powershell
//! $env:DM_TEST_DSN='DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc bench -p dm-query --bench batch_insert
//! ```
//!
//! 没配 `DM_TEST_DSN` 时**优雅跳过**（打印一行原因就返回）：CI 里 `cargo bench`
//! 不带 DSN，不能让基准把整个 CI 弄红。
//!
//! ## `diag` 模式：10000 行时数组路径的单行成本为什么翻倍
//!
//! V-07 主表留下一个**未定位**的现象：数组路径的单行成本在 1000 行是 5.6 µs/行，
//! 到 10000 行变成 11.7 µs/行（2.1×），而逐行路径稳定在 ~85 µs/行。这不影响「数组绑定
//! 更快」的结论，但会影响 §5.10/§14 V-07 引用数字时的口径，所以单独做三个对照：
//!
//! 1. **克隆对照**：`execute_many` 的冻结签名吃 `impl IntoIterator`，当前 bench 只能在
//!    计时窗口内 `params.to_vec()`。把克隆挪到窗口外用 `params.clone()`，看倍数回不回到
//!    ~15×。两个变体的**计时区段刻意不同**（原生那次窗口内含克隆），所以两边并排看的是
//!    「克隆值多少毫秒」，而不是「谁更快」。
//! 2. **规模曲线**：1000 / 5000 / 10000 上数组路径的 µs/行，判断劣化是**跳变**还是**连续上升**。
//! 3. **一次大调用 vs 多次小调用**：同为 10000 行，① 一次 10000 组；② 10 次各 1000 组
//!    （同一事务、同一 `PooledConn`）。相等 ⇒ 劣化只与「总行数」有关；②更快 ⇒ 劣化发生在
//!    「单次调用的规模」里（驱动/服务端的 set 处理）。
//!
//! 每个规格先跑一次**热身**（不进中位数）：编译版首次 `SQLExecute` 会带上驱动/服务端
//! 的一次性开销，热身把它摊掉，否则几轮里的第一轮会被它污染。
//!
//! ```powershell
//! cargo +stable-x86_64-pc-windows-msvc bench -p dm-query --bench batch_insert -- diag
//! ```

use std::time::{Duration, Instant};

use dm_core::{DmConfig, DmValue, PoolConfig};
use dm_error::DmError;
use dm_odbc::OdbcBackend;
use dm_pool::{DmPool, ExecutorConfig};
use dm_query::PooledConn;

/// bench 专用表。**刻意不复用集成夹具的 `DM_*` 表**：基准要反复 `delete`，
/// 与用例抢同一张表会互相踩（还会让计时把别的用例的等待算进来）。
const T_BENCH: &str = "DM_BENCH_BATCH";

/// 一次插一行、两列（`ID`、`V`）。两种规格共用同一条 SQL。
const INSERT_SQL: &str = "insert into DM_BENCH_BATCH (ID, V) values (?, ?)";

/// 每组的轮数。取 5 是「中位数有意义」的下限，再多就把 10000 行那组的耗时拖成分钟级。
const ROUNDS: usize = 5;

/// 两种规格：1000 行是 V-07 的口径，10000 行看倍数是否随规模变化。
const SIZES: [usize; 2] = [1000, 10_000];

/// `diag` 模式每格轮数。取 5 是「中位数能压住抖动」的下限；1000 行那格一轮只需几毫秒，
/// 10000 行那格一轮约 0.1 s，整体仍在几分钟内。
const DIAG_ROUNDS: usize = 5;

/// `diag 1a` 纯内存克隆的轮数：不碰数据库，多跑几轮几乎不花时间，中位数才稳。
const CLONE_ROUNDS: usize = 9;

/// `diag` 模式的规模曲线采样点。1000 与 10000 是 V-07 有数的两点，5000 夹在中间，
/// 用来分辨「跳变」与「连续上升」——跳跃阈值若落在 (1000, 5000] 或 (5000, 10000] 内，
/// 三点就能看出斜率在哪一段突变。
const DIAG_SIZES: [usize; 3] = [1000, 5000, 10_000];

/// 实验 3 的「多次小调用」切分：10000 行 = 10 次 × 1000 组。
const SPLIT: usize = 1000;

// ---------------------------------------------------------------------------
// 配置：没配环境就跳过
// ---------------------------------------------------------------------------

/// 读真机配置：`DM_TEST_DSN` 优先，否则 `DM_SERVER` 系；两者都没有返回 `None`。
fn base_config() -> Option<DmConfig> {
    if let Ok(dsn) = std::env::var("DM_TEST_DSN")
        && !dsn.trim().is_empty()
    {
        let mut cfg = DmConfig::from_connection_string(&dsn).ok()?;
        cfg.pool = single_conn_pool();
        return Some(cfg);
    }
    if std::env::var("DM_SERVER").is_ok() {
        let mut cfg = DmConfig::from_env().ok()?;
        cfg.pool = single_conn_pool();
        return Some(cfg);
    }
    None
}

/// 基准要的是**一条**连接：多连接会把 1000 次往返分散到不同会话上，计时就不再是
/// 「绑定方式的差别」而是「连接的差别」。
fn single_conn_pool() -> PoolConfig {
    PoolConfig {
        max_size: 1,
        min_idle: 0,
        ..PoolConfig::default()
    }
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Some(cfg) = base_config() else {
        println!("跳过：未设置 DM_TEST_DSN（基准只在真库上跑）");
        return;
    };

    let pool = match DmPool::connect_with(cfg, ExecutorConfig::for_pool(1)).await {
        Ok(pool) => pool,
        Err(err) => {
            // 基准不该因为库没起来就 panic：CI 上「跳过」比「红」有用。
            println!("跳过：真机建池失败——{err}");
            return;
        }
    };

    let diag = std::env::args().nth(1).as_deref() == Some("diag");
    let outcome = if diag {
        run_diag(&pool).await
    } else {
        run(&pool).await
    };
    if let Err(err) = outcome {
        println!("跳过：基准执行失败——{err}");
    }
}

async fn run(pool: &DmPool<OdbcBackend>) -> Result<(), DmError> {
    {
        let mut conn = acquire(pool).await?;
        setup(&mut conn).await?;
    }

    println!("== V-07 批量写基准（达梦 DM8，每规格 {ROUNDS} 轮取中位数） ==");
    println!("表 {T_BENCH}(ID INT, V VARBINARY(200))；相同 SQL、相同连接，只换绑定方式");
    println!("计时不含建连与清表：只掐真正写数据的那一段");
    println!();
    println!(
        "{:>8}  {:>12}  {:>12}  {:>8}",
        "行数", "逐行 ms", "数组 ms", "倍数"
    );

    let mut summary: Vec<(usize, f64, f64)> = Vec::new();
    for rows in SIZES {
        let per_group = build_params(rows);
        let mut row_by_row = Vec::with_capacity(ROUNDS);
        let mut array = Vec::with_capacity(ROUNDS);

        for _ in 0..ROUNDS {
            {
                let mut conn = acquire(pool).await?;
                clear(&mut conn).await?;
            }
            row_by_row.push(time_row_by_row(pool, &per_group).await?);

            {
                let mut conn = acquire(pool).await?;
                clear(&mut conn).await?;
            }
            array.push(time_array(pool, &per_group).await?);
        }

        let row_ms = median_ms(&mut row_by_row);
        let array_ms = median_ms(&mut array);
        let ratio = row_ms / array_ms;
        println!("{rows:>8}  {row_ms:>12.1}  {array_ms:>12.1}  {ratio:>7.2}x");
        summary.push((rows, row_ms, ratio));
    }

    {
        let mut conn = acquire(pool).await?;
        if let Err(err) = clear(&mut conn).await {
            println!("警告：收尾清表失败——{err}");
        }
    }

    println!();
    for (rows, row_ms, ratio) in &summary {
        println!("V-07：{rows} 行 → {ratio:.2}×（逐行 {row_ms:.1} ms）");
    }
    Ok(())
}

async fn acquire(pool: &DmPool<OdbcBackend>) -> Result<PooledConn<OdbcBackend>, DmError> {
    PooledConn::acquire(pool).await
}

// ---------------------------------------------------------------------------
// diag：为什么 10000 行时单行成本翻倍
// ---------------------------------------------------------------------------

/// 诊断主流程。三个实验各自清表、各自计时，互不共享预热状态。
async fn run_diag(pool: &DmPool<OdbcBackend>) -> Result<(), DmError> {
    {
        let mut conn = acquire(pool).await?;
        setup(&mut conn).await?;
    }

    diag_clone(pool).await?;
    diag_curve(pool).await?;
    diag_split(pool).await?;

    let mut conn = acquire(pool).await?;
    if let Err(err) = clear(&mut conn).await {
        println!("警告：收尾清表失败——{err}");
    }
    Ok(())
}

/// 实验 1：克隆对照。
///
/// 与 `diag 2` 的区别：这里每次量**同一个规格**的两种变体，并且先用纯内存微基准
/// 直接量出「克隆一次 10000 组参数」本身要多少毫秒——那是这次劣化的候选原因之一，
/// 只有把它的绝对量级钉死，才能判断它够不够解释 2.1×。
async fn diag_clone(pool: &DmPool<OdbcBackend>) -> Result<(), DmError> {
    println!("== diag 1a：克隆本身（纯内存，{CLONE_ROUNDS} 轮中位数） ==");
    println!();
    println!("{:>10}  {:>14}  {:>14}", "行数", "克隆 ms", "µs/行");
    for rows in SIZES {
        let params = build_params(rows);
        let mut samples = Vec::with_capacity(CLONE_ROUNDS);
        for _ in 0..CLONE_ROUNDS {
            let started = Instant::now();
            let copy = params.to_vec();
            samples.push(started.elapsed());
            // 防止编译器把「没人用的克隆」优化掉。
            std::hint::black_box(&copy);
        }
        let ms = median_ms(&mut samples);
        println!(
            "{rows:>10}  {ms:>14.3}  {:>14.2}",
            ms * 1000.0 / rows as f64
        );
    }
    println!();

    println!("== diag 1b：克隆在窗口内 vs 窗口外（DB 计时，{DIAG_ROUNDS} 轮中位数） ==");
    println!();
    println!(
        "{:>8}  {:>14}  {:>14}  {:>14}  {:>10}",
        "行数", "原生 ms", "预克隆 ms", "差 ms", "差 µs/行"
    );
    for rows in SIZES {
        let params = build_params(rows);
        let mut native = PairResult::new();
        let mut pre = PairResult::new();
        for round in 0..DIAG_ROUNDS {
            clear_quiet(pool).await?;
            let sample = time_array(pool, &params).await?;
            native.push(round, sample);

            clear_quiet(pool).await?;
            let sample = time_array_precloned(pool, &params).await?;
            pre.push(round, sample);
        }
        let (native_ms, native_min, native_max) = median_min_max_ms(&mut native.timed);
        let (pre_ms, _, _) = median_min_max_ms(&mut pre.timed);
        println!(
            "{rows:>8}  {native_ms:>14.1}  {pre_ms:>14.1}  {:>14.1}  {:>10.2}   [{native_min:.1}..{native_max:.1}]",
            native_ms - pre_ms,
            (native_ms - pre_ms) * 1000.0 / rows as f64
        );
    }
    println!();
    Ok(())
}

/// 实验 2：规模曲线。逐行那列只跑一次（它已是线性基线，跑三次只会把诊断拖长）。
///
/// 除了中位数，这里还打**最小/最大**：10000 行那一格在多次重跑之间出现过 ~2× 的摆动，
/// 只看中位数会掩盖「同一操作本身就不稳」这件事，而它直接决定后面能不能下结论。
async fn diag_curve(pool: &DmPool<OdbcBackend>) -> Result<(), DmError> {
    println!("== diag 2：规模曲线（数组路径 {DIAG_ROUNDS} 轮，首轮热身不计） ==");
    println!();
    println!(
        "{:>8}  {:>10}  {:>10}  {:>10}  {:>12}  {:>12}  {:>12}",
        "行数", "数组中位", "数组最小", "数组最大", "数组 µs/行", "逐行 ms", "逐行 µs/行"
    );

    for rows in DIAG_SIZES {
        let params = build_params(rows);
        let mut array = Vec::with_capacity(DIAG_ROUNDS);
        for round in 0..DIAG_ROUNDS {
            clear_quiet(pool).await?;
            let sample = time_array(pool, &params).await?;
            if round > 0 {
                array.push(sample);
            }
        }
        let (array_ms, array_min, array_max) = median_min_max_ms(&mut array);

        clear_quiet(pool).await?;
        let mut row_by_row = Vec::with_capacity(DIAG_ROUNDS);
        for round in 0..DIAG_ROUNDS {
            clear_quiet(pool).await?;
            let sample = time_row_by_row(pool, &params).await?;
            if round > 0 {
                row_by_row.push(sample);
            }
        }
        let (row_ms, _, _) = median_min_max_ms(&mut row_by_row);

        println!(
            "{rows:>8}  {array_ms:>10.1}  {array_min:>10.1}  {array_max:>10.1}  {:>12.2}  {row_ms:>12.1}  {:>12.2}",
            array_ms * 1000.0 / rows as f64,
            row_ms * 1000.0 / rows as f64
        );
    }
    println!();
    Ok(())
}

/// A/B 对照的记账：`round` 是原始逐轮样本（含热身那一轮），`timed` 是去掉热身后的样本。
struct PairResult {
    /// 逐轮原始样本，含热身那一轮。目前只作对照留档，诊断输出用的是 `timed`。
    #[allow(dead_code)]
    round: Vec<Duration>,
    timed: Vec<Duration>,
}

impl PairResult {
    fn new() -> Self {
        Self {
            round: Vec::new(),
            timed: Vec::new(),
        }
    }

    fn push(&mut self, round: usize, sample: Duration) {
        self.round.push(sample);
        if round > 0 {
            self.timed.push(sample);
        }
    }
}

/// `diag 3` 的一次对照：总行数 `rows`、小切分 `split`，返回
/// `(一次大调用, 多次小调用)` 两组样本。调用者把「同总行数、不同切分」并排比。
async fn split_pair(
    pool: &DmPool<OdbcBackend>,
    rows: usize,
    split: usize,
) -> Result<(PairResult, PairResult), DmError> {
    let params = build_params(rows);
    let mut single = PairResult::new();
    let mut split_ms = PairResult::new();
    for round in 0..DIAG_ROUNDS {
        clear_quiet(pool).await?;
        let sample = time_array(pool, &params).await?;
        single.push(round, sample);

        clear_quiet(pool).await?;
        let sample = time_array_split(pool, &params, split).await?;
        split_ms.push(round, sample);
    }
    Ok((single, split_ms))
}

/// 实验 3：固定总行数，只换「每次调用多大」。
///
/// 两个总行数各测两个切分：若劣化由**单次调用规模**引起，则同一总行数下小切分应明显
/// 更快（①/② 显著大于 1）；若由**总行数**引起，则同一总行数下两种切分应相当（≈1）。
///
/// 第一行刻意是 **10000 行 vs 10000 行**（A/A 对照）：同一条代码路径、同一张表、同一个
/// 事务形状，唯一差别是它被跑了两遍。这一行给出「在没有任何自变量变化时，同一次运行里
/// 两个位置的能量差有多大」——没有它，下面两行的 ①/② 就分不清是效应还是位置噪声。
async fn diag_split(pool: &DmPool<OdbcBackend>) -> Result<(), DmError> {
    println!("== diag 3：固定总行数，只换单次调用的规模（{DIAG_ROUNDS} 轮，首轮热身不计） ==");
    println!();
    println!(
        "{:>8}  {:>22}  {:>13}  {:>13}  {:>13}  {:>13}  {:>9}",
        "总行数", "切分", "① µs/行", "② µs/行", "① 最小", "① 最大", "①/②"
    );

    for (label, rows, split) in [
        ("A/A 10000 vs 10000", 10_000usize, 0usize),
        ("1×10000 vs 10×1000", 10_000, SPLIT),
        ("1×5000 vs 5×1000", 5000, SPLIT),
    ] {
        // `split == 0` 表示 A/A：两边都走一次大调用，不存在小切分。
        let (mut single, mut other) = if split == 0 {
            split_pair(pool, rows, rows).await?
        } else {
            split_pair(pool, rows, split).await?
        };
        let one = median_min_max_ms(&mut single.timed);
        let two = median_min_max_ms(&mut other.timed);
        let (one_med, _, one_max) = one;
        let (two_med, _, _) = two;
        println!(
            "{rows:>8}  {label:>22}  {:>13.2}  {:>13.2}  {:>13.1}  {:>13.1}  {:>8.2}x",
            one_med * 1000.0 / rows as f64,
            two_med * 1000.0 / rows as f64,
            one.1,
            one_max,
            one_med / two_med
        );
    }
    println!();
    println!(
        "（① = 一次大调用；② = 多次小调用；A/A 行两边都是「一次大调用」，因此 ①/② 只反映噪声）"
    );
    println!();
    Ok(())
}

/// 诊断里每次测量前清表：独立 try，失败只提醒不中断整轮诊断。
async fn clear_quiet(pool: &DmPool<OdbcBackend>) -> Result<(), DmError> {
    let mut conn = acquire(pool).await?;
    if let Err(err) = clear(&mut conn).await {
        println!("警告：清表失败——{err}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 计时
// ---------------------------------------------------------------------------

/// 逐行路径：每组参数 `bind_params` + `execute` 各一次。
///
/// 这条路走的是 `PooledConn::execute`（单组），1000 行 = 1000 次调用、1000 次往返，
/// 正是 §4.7 要淘汰的那条。传 `args.as_slice()`（`&[DmValue]` 实现了 `IntoArgs`）而不是
/// `vec![args.clone()]`：后者会把「克隆 1000 次参数」记进逐行那一列。
async fn time_row_by_row(
    pool: &DmPool<OdbcBackend>,
    per_group: &[Vec<DmValue>],
) -> Result<Duration, DmError> {
    let mut txn = acquire(pool).await?.begin().await?;
    let started = Instant::now();
    for args in per_group {
        // 单组参数：`execute` 内部 `args.is_empty()` 为假 ⇒ `bind_params` + `execute("")`。
        // 传 `args.as_slice()`（`&[DmValue]`）而不是 `slice::from_ref(args)`——后者给出的是
        // `&[Vec<DmValue>]`，一整片「参数组」的切片，不实现 `IntoArgs`。
        txn.execute(INSERT_SQL, args.as_slice()).await?;
    }
    let elapsed = started.elapsed();
    txn.commit().await?;
    Ok(elapsed)
}

/// 数组绑定路径：`execute_many` 一次 `SQLExecute` 打完整组。
async fn time_array(
    pool: &DmPool<OdbcBackend>,
    params: &[Vec<DmValue>],
) -> Result<Duration, DmError> {
    let mut txn = acquire(pool).await?.begin().await?;
    let started = Instant::now();
    txn.execute_many(INSERT_SQL, params.to_vec()).await?;
    let elapsed = started.elapsed();
    txn.commit().await?;
    Ok(elapsed)
}

/// 中位数（毫秒）。`ROUNDS` 是奇数，取排序后的正中那个。
fn median_ms(samples: &mut [Duration]) -> f64 {
    samples.sort_unstable();
    let mid = samples[samples.len() / 2];
    mid.as_secs_f64() * 1000.0
}

/// `(中位数, 最小, 最大)` 毫秒。诊断里抖动本身就是结论的一部分，所以三个都给。
fn median_min_max_ms(samples: &mut [Duration]) -> (f64, f64, f64) {
    samples.sort_unstable();
    let mid = samples[samples.len() / 2];
    (
        mid.as_secs_f64() * 1000.0,
        samples[0].as_secs_f64() * 1000.0,
        samples[samples.len() - 1].as_secs_f64() * 1000.0,
    )
}

/// `diag 1` 的「预克隆」变体：`clone()` 发生在 `Instant::now()` **之前**，窗口内只有
/// `execute_many` 本身。与 `time_array` 的差就是那一次克隆的毫秒数。
async fn time_array_precloned(
    pool: &DmPool<OdbcBackend>,
    params: &[Vec<DmValue>],
) -> Result<Duration, DmError> {
    let mut txn = acquire(pool).await?.begin().await?;
    let owned = params.to_vec();
    let started = Instant::now();
    txn.execute_many(INSERT_SQL, owned).await?;
    let elapsed = started.elapsed();
    txn.commit().await?;
    Ok(elapsed)
}

/// `diag 3` 的「多次小调用」变体：把 `params` 按 `split` 切成多段，每段一次
/// `execute_many`，**全部在同一个事务、同一条 `PooledConn` 上**——这样两边唯一的
/// 差别就是「单次调用的规模」，而不是事务或连接的差别。
async fn time_array_split(
    pool: &DmPool<OdbcBackend>,
    params: &[Vec<DmValue>],
    split: usize,
) -> Result<Duration, DmError> {
    let mut txn = acquire(pool).await?.begin().await?;
    let started = Instant::now();
    for chunk in params.chunks(split) {
        // 每次调用都自带 `to_vec()`：与 `time_array` 口径一致，两端都付同样的克隆成本。
        txn.execute_many(INSERT_SQL, chunk.to_vec()).await?;
    }
    let elapsed = started.elapsed();
    txn.commit().await?;
    Ok(elapsed)
}

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 造 `rows` 组参数：`ID` 是行号，`V` 是 8 字节二进制（走 `EXT_VAR_BINARY`，
/// 顺带覆盖二进制列的数组绑定——纯 INT 的基准会漏掉变长列的步长逻辑）。
fn build_params(rows: usize) -> Vec<Vec<DmValue>> {
    (0..rows)
        .map(|i| {
            vec![
                DmValue::Int(i as i32),
                DmValue::Bytes(i.to_le_bytes().to_vec()),
            ]
        })
        .collect()
}

/// 建表 + 建唯一索引（幂等：先删后建）。
///
/// 用 `drop table if exists` + `create table` 而不是 `create ... if not exists` +
/// `create unique index if not exists`：后者依赖达梦对两条 `if not exists` 语法的支持，
/// 而前者与集成夹具用的是同一条已验证路径。代价是每次跑基准多一次建表，但它发生在
/// 计时窗口**之外**（`Instant` 只掐写数据那段）。
///
/// `V` 上带唯一索引是刻意的：`V` 由行号小端编码而来，组内天然互不相同，唯一索引让
/// 服务端**真的**去比较每一行——不建索引的话，量到的可能只是「服务端根本没看值」的
/// 空转。两条路径付同样的索引维护成本，所以比较仍然公平。
async fn setup(conn: &mut PooledConn<OdbcBackend>) -> Result<(), DmError> {
    for sql in [
        format!("drop table if exists {T_BENCH}"),
        format!("create table {T_BENCH} (ID INT PRIMARY KEY, V VARBINARY(200))"),
        format!("create unique index IDX_{T_BENCH}_V on {T_BENCH} (V)"),
    ] {
        conn.execute(&sql, ()).await?;
    }
    Ok(())
}

/// 清空行，但**保留表与索引**：重建表会把索引构建的开销计进来，且下一轮 `clear`
/// 就不再是一条等价路径了。
async fn clear(conn: &mut PooledConn<OdbcBackend>) -> Result<(), DmError> {
    conn.execute(&format!("delete from {T_BENCH}"), ()).await?;
    Ok(())
}
