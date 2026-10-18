//! `dm-query` 的真机门禁：事务、保存点、`Drop` 回滚、derive 往返、流式取数
//! （需要达梦 DM8 实例）。判据来自设计方案 §12.2 的 M5 表（T5.1 / T5.2 / T5.3）。
//!
//! 与 `query.rs` / `derive.rs` 的分工：那两组跑在离线假后端上，钉的是**这一层自己的
//! 逻辑**；本组用**真达梦**钉「事务在服务端真的回了滚、保存点真的只影响之后、
//! 流式真的把 10 万行都送到了」。离线假后端永远证明不了这些——它连事务都没有。
//!
//! 跑法（host 是 gnu，`dlltool.exe` 不在 PATH，必须显式 msvc）：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p dm-query --test query_real -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! ## 内存那一半：本组末尾的三条「内存预算」用例
//!
//! T5.2 的「`fetch_stream` 10 万行 **RSS 平坦**」与 T6.3 的「分块读/写不随值放大」原先
//! 只有功能性验证，如实写着「未采 `dhat`/RSS」。现在由末尾三条用例用**进程内计数分配器**
//! （`tests/meter/mod.rs`：`GlobalAlloc` 薄包装 `System`，不引依赖）量「窗口内峰值活字节」。
//!
//! 能证到的是**我们与 `odbc-api` 这一层**不随数据量放大；驱动 DLL 内部的 `malloc` 走它
//! 自己的 CRT，本进程看不见，所以**整进程 RSS** 仍不是本组的判据（缺口原文见
//! `docs/acceptance-report.md` §4）。三条用例都带**阳性对照**：同一条路径把数据真攒在
//! 内存里，仪器必须看得见——否则「峰值很小」只说明仪器没灵敏度。

use std::future::poll_fn;
use std::pin::Pin;
use std::time::{Duration, Instant};

use dm_core::{DmConfig, DmValue, PoolConfig};
use dm_error::{DmError, DmErrorKind};
use dm_macros::FromRow;
// 默认后端（dpi feature 打开时是 `dm_dpi::DpiBackend`），这样同一份真机用例能跑两种后端。
use dm_pool::{DmPool, ExecutorConfig};
use dm_query::DefaultBackend as OdbcBackend;
use dm_query::{LobBytes, LobKind, LobParam, LobReader, LobRef, PooledConn, RowStream};
use futures_core::Stream;

/// 进程内计数分配器（`GlobalAlloc` 薄包装 `System`）：末尾三条「内存预算」用例的仪器。
/// 与 `alloc_budget.rs` 共用同一份模块，但每个测试二进制各编一份。
mod meter;

/// 事务表：`ID` 主键、`V` 值。名字带前缀，避免与别的测试组撞名。
const T_TXN: &str = "DM_Q_REAL_TXN";
/// 流式用例的种子表：**恰好 10 行**，5 路自连接正好 10^5 = 100000 行。
const T_SEED: &str = "DM_Q_REAL_SEED";
/// T6.4 批量写用例的表：`ID` 主键 + `V` 上有**唯一索引**（第 500 行违约靠它）。
const T_BATCH: &str = "DM_Q_REAL_BATCH";
/// T6.3 分块读用例的表：`BODY` 是 CLOB。本组用例自己建，不走 `setup()`。
const T_LOB: &str = "DM_Q_REAL_LOB";
/// T6.3 分块**写**用例的表：`B` 是 BLOB、`C` 是 CLOB（`SQLPutData` 延迟输入）。
const T_LOB_W: &str = "DM_Q_REAL_LOBW";

// ---------------------------------------------------------------------------
// 配置：没配环境就跳过
// ---------------------------------------------------------------------------

/// 从环境读真机配置：`DM_TEST_DSN` 优先，否则 `DM_SERVER` 系。
fn base_config() -> Option<DmConfig> {
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

/// 配好池参数的真机配置；返回 `None` 表示本机没配达梦（用例跳过）。
///
/// `max_size` 原样生效（下限 1）：可见性用例要 `2+` 条连接（一条在事务里，另一条从
/// 外面看），而「丢弃的事务被池回滚」用例**必须**用 `1`——只有一条物理连接才能证明
/// 复位真的在**同一条**连接上发生了服务端回滚。
fn real_config(max_size: usize) -> Option<DmConfig> {
    let Some(mut cfg) = base_config() else {
        eprintln!("跳过：未配置 DM_TEST_DSN / DM_SERVER（真机用例只在真库上跑，见文件头注释）");
        return None;
    };
    cfg.pool = PoolConfig {
        max_size: max_size.max(1),
        min_idle: 0,
        ..PoolConfig::default()
    };
    Some(cfg)
}

/// 建池（`max_size` 见上）。
async fn real_pool(max_size: usize) -> Option<DmPool<OdbcBackend>> {
    let cfg = real_config(max_size)?;
    let pool = DmPool::connect_with(cfg, ExecutorConfig::for_pool(max_size.max(1)))
        .await
        .expect("真机上建池失败");
    Some(pool)
}

/// 借一条连接出来（`PooledConn` 由 `dm-query` 提供；用例自己 drop）。
async fn conn_of(pool: &DmPool<OdbcBackend>) -> PooledConn<OdbcBackend> {
    PooledConn::acquire(pool).await.expect("从池里借连接失败")
}

/// 建好本组要用的三张表（幂等：先删后建）。
async fn setup(pool: &DmPool<OdbcBackend>) {
    let mut conn = conn_of(pool).await;
    for sql in [
        format!("drop table if exists {T_TXN}"),
        format!("drop table if exists {T_SEED}"),
        format!("drop table if exists {T_BATCH}"),
        format!("create table {T_TXN} (ID INT PRIMARY KEY, V INT)"),
        format!("create table {T_SEED} (ID INT PRIMARY KEY)"),
        // 批量写表：`ID` 主键之外，`V` 上再加一个**唯一索引**——第 500 行违约靠它。
        // 用「唯一索引」而不是「唯一约束」：两者在达梦上的错误码同族，而索引版本
        // 不依赖命名约束，`drop table` 收拾起来更干净。
        format!("create table {T_BATCH} (ID INT PRIMARY KEY, V INT)"),
        format!("create unique index IDX_{T_BATCH}_V on {T_BATCH} (V)"),
    ] {
        conn.execute(&sql, ())
            .await
            .unwrap_or_else(|err| panic!("建表语句失败：{sql}\n{err:?}"));
    }
}

/// `select count(*)`（`count(*)` 在达梦上回 BIGINT，取 `i64`）。
async fn count_rows(conn: &mut PooledConn<OdbcBackend>, table: &str) -> i64 {
    let row = conn
        .query_one(&format!("select count(*) from {table}"), ())
        .await
        .expect("count(*) 查询失败")
        .expect("count(*) 必须有且只有一行");
    row.get::<i64>(0).expect("count(*) 应当是整数")
}

/// 把一个流的下一项 `await` 出来（不引 `futures-util`，只用 `poll_fn`）。
async fn next_row(stream: &mut RowStream<OdbcBackend>) -> Option<Result<dm_query::Row, DmError>> {
    poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}

/// 当前后端是否把 LOB **按窗口逐趟交付**。
///
/// `dm-odbc` 是：一趟最多 2048 个码元（`LOB_CHUNK_BYTES`），所以「趟数 ≥ 190」
/// 「峰值不随值增长」这类断言成立。`dm-dpi` **整值一趟**（`dpi_get_data` 的分块模式
/// 会在块尾塞 NUL，见 `crates/dm-dpi/src/backend.rs` 的 `read_long`），因此窗口类
/// 断言只对 ODBC 有意义——但**内容逐字节一致**、**读完之后不残留**这些断言两种后端
/// 都必须过。
fn lob_is_windowed() -> bool {
    cfg!(not(feature = "dpi"))
}

// ---------------------------------------------------------------------------
// T5.2：真机端到端
// ---------------------------------------------------------------------------

/// 最小端到端：参数化插入 → 参数化查询 → 列元数据与值的往返。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_query_round_trips_rows_and_columns() {
    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let mut conn = conn_of(&pool).await;
    let affected = conn
        .execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (1i32, 42i32),
        )
        .await
        .expect("插入失败");
    assert_eq!(affected, 1, "插入一行应当回 1");

    let row = conn
        .query_one(&format!("select ID, V from {T_TXN} where ID = ?"), (1i32,))
        .await
        .expect("查询失败")
        .expect("必须有一行");
    assert_eq!(row.get::<i32>("ID").unwrap(), 1);
    assert_eq!(row.get::<i32>("V").unwrap(), 42);
    // 列名查找大小写不敏感是冻结语义（§6.12），真机上也要成立。
    assert_eq!(row.get::<i32>("id").unwrap(), 1);
    assert_eq!(row.columns().len(), 2);

    // 「没有行」不是错误：`query_one` 在空结果上是 `Ok(None)`。
    let missing = conn
        .query_one(&format!("select ID from {T_TXN} where ID = ?"), (999i32,))
        .await
        .expect("空结果不该是错误");
    assert!(missing.is_none(), "空结果应当是 None");

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// T5.1：回滚后数据在**另一条连接**上不可见；提交后可见。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_rollback_hides_and_commit_persists() {
    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let mut observer = conn_of(&pool).await;

    // 第一条事务：插入后回滚。
    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (1i32, 10i32),
        )
        .await
        .expect("事务内插入失败");

        // 未提交：写方自己看得见，外面看不见（默认 READ COMMITTED）。
        assert_eq!(count_rows(txn.conn(), T_TXN).await, 1);
        assert_eq!(
            count_rows(&mut observer, T_TXN).await,
            0,
            "未提交的插入不该被另一条连接看见"
        );

        txn.rollback().await.expect("回滚失败");
    }
    assert_eq!(
        count_rows(&mut observer, T_TXN).await,
        0,
        "回滚之后另一条连接依然应当看不到那一行"
    );

    // 第二条事务：插入后提交，必须看得见。
    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (2i32, 20i32),
        )
        .await
        .expect("事务内插入失败");
        txn.commit().await.expect("提交失败");
    }
    assert_eq!(
        count_rows(&mut observer, T_TXN).await,
        1,
        "提交之后另一条连接应当看得见那一行"
    );

    drop(observer);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// T5.1：`Transaction` 被丢弃（没提交也没回滚）⇒ **服务端已回滚**。
///
/// `max_size = 1` 是这个用例的关键：整个池只有一条物理连接，于是「丢事务之后再借
/// 回来」借到的必然是**同一条**连接，而它必须先经过池的复位（= 服务端回滚 +
/// `autocommit = ON`）才可能被借出。断言因此有了意义——如果复位没有回滚，那条连接
/// 仍停在旧事务里，下一句 `count(*)` 就会看见未提交的行。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_dropped_transaction_rolls_back_on_the_server() {
    let Some(pool) = real_pool(1).await else {
        return;
    };
    setup(&pool).await;

    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (7i32, 70i32),
        )
        .await
        .expect("事务内插入失败");
        assert_eq!(
            count_rows(txn.conn(), T_TXN).await,
            1,
            "事务内应当看得见自己插的行"
        );
        // 故意不 commit、不 rollback：`Drop` 只能标脏，回滚交给池在阻塞线程上做。
    }

    // `acquire` 会等到那条待复位的连接被复位完（名额在它手上）。
    let mut conn = conn_of(&pool).await;
    assert_eq!(
        count_rows(&mut conn, T_TXN).await,
        0,
        "被丢弃的事务必须在服务端回滚：同一条物理连接上不该还看得见那一行"
    );
    drop(conn);

    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// T5.1：保存点回滚**只影响它之后**的语句，且 `release_savepoint` 语法可用。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_savepoint_rolls_back_only_what_came_after() {
    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let mut observer = conn_of(&pool).await;

    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");

        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (1i32, 1i32),
        )
        .await
        .expect("保存点之前插入失败");
        txn.savepoint("sp1").await.expect("建立保存点失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (2i32, 2i32),
        )
        .await
        .expect("保存点之后插入失败");

        txn.rollback_to("sp1").await.expect("回滚到保存点失败");

        // 保存点之前的那一行还在事务里，之后的没了。
        assert_eq!(
            count_rows(txn.conn(), T_TXN).await,
            1,
            "sp1 之前的那一行必须还在"
        );
        let survivors = txn
            .conn()
            .query(&format!("select ID from {T_TXN} order by ID"), ())
            .await
            .expect("查询失败");
        assert_eq!(
            survivors.rows().first().and_then(|r| r.get::<i32>(0).ok()),
            Some(1),
            "活下来的应当是 ID = 1"
        );

        // 达梦实测：DM8 没有 `RELEASE SAVEPOINT` 语法（-2007）。这一层把它翻成
        // `Unsupported`，并且必须保留原始诊断——调用方要能看见服务端原话。
        let err = txn.release_savepoint("sp1").await.expect_err(
            "DM8 应当拒绝 RELEASE SAVEPOINT；这里通过了说明服务端已经支持，\
             请更新能力表、CAP-E-05 与 docs/设计方案.md §6.6",
        );
        assert!(
            matches!(err.kind, DmErrorKind::Unsupported { .. }),
            "期望 Unsupported，实得 {:?}",
            err.kind
        );
        // 诊断原文必须保留。消息语言取决于驱动设置（ODBC 走宽字符拿中文、DPI 在
        // `LANG_ID=ENGLISH` 下拿英文），所以按**原生码**判，不绑死某一种文案。
        assert!(
            err.diags.iter().any(|d| d.native_code == -2007),
            "原始 -2007 诊断必须保留：{:?}",
            err.diags
        );

        // 失败的是**语句**不是事务：这个语法错没有把事务搞坏，提交必须成功。
        txn.commit().await.expect("提交失败");
    }

    assert_eq!(
        count_rows(&mut observer, T_TXN).await,
        1,
        "提交后另一条连接应当只看到 sp1 之前的那一行"
    );

    drop(observer);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// 保存点之后再提交：**独立会话**必须看得见「保存点之前」那一行。
///
/// 与 [`real_savepoint_rolls_back_only_what_came_after`] 的区别是**观察者是另一个池**，
/// 也就是另一批物理会话。同池观察可以（错误地）由「池复用了同一条连接」解释掉：
/// 一条 `autocommit = OFF` 的泄漏连接会看见自己那条未提交事务里的行，于是用例
/// 即便在「提交其实没发生」时也能通过。独立池没有这个漏洞——它只能看见**服务端真的
/// 提交过**的数据。
///
/// 这条用例是 2026 年 T7.2 的 `dameng/examples/txn.rs` 在真机上跑出
/// 「回滚到保存点后行数 = 1（应为 2）」之后补的定位用例（见 §12.3 的 M7 记录）。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_savepoint_commit_is_visible_to_an_independent_pool() {
    let Some(pool) = real_pool(2).await else {
        return;
    };
    let Some(other) = real_pool(1).await else {
        return;
    };
    setup(&pool).await;

    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (1i32, 10i32),
        )
        .await
        .expect("保存点之前插入失败");
        txn.savepoint("sp1").await.expect("建立保存点失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (2i32, 20i32),
        )
        .await
        .expect("保存点之后插入失败");
        txn.rollback_to("sp1").await.expect("回滚到保存点失败");
        txn.commit().await.expect("提交失败");
    }

    // 独立池 ⇒ 独立会话 ⇒ 只能看到服务端真的提交过的行。
    let mut probe = conn_of(&other).await;
    assert_eq!(
        count_rows(&mut probe, T_TXN).await,
        1,
        "另一条独立会话必须看得见 sp1 之前那一行、看不见之后那一行"
    );
    let survivors = probe
        .query(&format!("select ID from {T_TXN} order by ID"), ())
        .await
        .expect("查询失败");
    assert_eq!(
        survivors.rows().first().and_then(|r| r.get::<i32>(0).ok()),
        Some(1),
        "活下来并已提交的应当只有 ID = 1"
    );
    drop(probe);

    // 同池再借一条也必须一致——两个数不一致就说明有连接带着未提交事务回了池。
    let mut mine = conn_of(&pool).await;
    assert_eq!(
        count_rows(&mut mine, T_TXN).await,
        1,
        "提交方自己的池也必须只看得见 1 行"
    );
    drop(mine);

    other.close().await.expect("关池失败");
    other.wait_closed().await;
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// 例形状的回归：`max_size = 10` + `min_idle = 1` + **每步之间插一次池上 count**。
///
/// 这是 `dameng/examples/txn.rs` 在真机上跑出「应为 2、实得 1」时的**完整形状**
/// （默认 `max_size = 10`、默认 `min_idle = 1`，且每步都重新借一条连接数行）。
/// 三个断言分别钉住：提交落地、保存点只撤销之后、失败语句不改变已提交行数。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_savepoint_commit_survives_interleaved_pool_counts() {
    let Some(mut cfg) = real_config(10) else {
        return;
    };
    // 与出问题的示例配置**逐项一致**：上限 10（默认）+ 保活 1 条（默认）。
    cfg.pool.min_idle = 1;
    let pool = DmPool::connect_with(cfg, ExecutorConfig::for_pool(10))
        .await
        .expect("真机上建池失败");
    setup(&pool).await;

    // 场景 1：先提交一行基准（示例的 `提交后行数 = 1` 那一步）。
    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (1i32, 10i32),
        )
        .await
        .expect("插入失败");
        txn.commit().await.expect("提交失败");
    }
    let mut c1 = conn_of(&pool).await;
    assert_eq!(count_rows(&mut c1, T_TXN).await, 1, "基准行应当已提交");
    drop(c1);

    // 场景 2：sp1 之前插入 + sp1 之后插入 + 回滚到 sp1 + 提交。
    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (2i32, 20i32),
        )
        .await
        .expect("保存点之前插入失败");
        txn.savepoint("sp1").await.expect("建立保存点失败");
        txn.execute(
            &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
            (3i32, 30i32),
        )
        .await
        .expect("保存点之后插入失败");
        txn.rollback_to("sp1").await.expect("回滚到保存点失败");
        txn.commit().await.expect("提交失败");
    }
    let mut c2 = conn_of(&pool).await;
    assert_eq!(
        count_rows(&mut c2, T_TXN).await,
        2,
        "sp1 之前那一行必须由这次提交落地（示例里这一步实得 1）"
    );
    drop(c2);

    // 场景 3：一条被服务端以语法错拒掉的语句不该影响事务（示例场景 4）。
    {
        let mut txn = conn_of(&pool).await.begin().await.expect("开事务失败");
        txn.savepoint("sp2").await.expect("建立保存点失败");
        let _ = txn.release_savepoint("sp2").await;
        txn.commit().await.expect("提交失败");
    }
    let mut c3 = conn_of(&pool).await;
    assert_eq!(
        count_rows(&mut c3, T_TXN).await,
        2,
        "失败的语句不该改变已提交行数"
    );
    drop(c3);

    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// T5.3：`#[derive(FromRow)]` 在真机结果集上的往返（含 `Option` 与 `rename`）。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_derive_round_trips_from_a_real_result_set() {
    #[derive(Debug, FromRow, PartialEq)]
    struct Row2 {
        #[dm(rename = "ID")]
        id: i32,
        v: Option<i32>,
    }

    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let mut conn = conn_of(&pool).await;
    conn.execute(
        &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
        (1i32, 11i32),
    )
    .await
    .expect("插入失败");
    conn.execute(
        &format!("insert into {T_TXN} (ID, V) values (?, ?)"),
        (2i32, DmValue::Null),
    )
    .await
    .expect("插入 NULL 行失败");

    let rows = conn
        .query(&format!("select ID, V from {T_TXN} order by ID"), ())
        .await
        .expect("查询失败");
    let mapped: Vec<Row2> = rows
        .rows()
        .iter()
        .map(|row| row.to::<Row2>().expect("derive 映射失败"))
        .collect();
    assert_eq!(
        mapped,
        vec![Row2 { id: 1, v: Some(11) }, Row2 { id: 2, v: None }]
    );

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// T5.2：流式取数把 10 万行全部按序送到。
///
/// 结果集由 `DM_Q_REAL_SEED`（10 行）**五路自连接**产生：10^5 = 100000，不依赖
/// `CONNECT BY` 这类方言写法。这里验的是功能与增量正确性；RSS 平坦见文件头注释。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_stream_delivers_one_hundred_thousand_rows() {
    const EXPECTED: i64 = 100_000;
    // 1 + 2 + … + 100000
    const SUM: i64 = EXPECTED * (EXPECTED + 1) / 2;

    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let mut conn = conn_of(&pool).await;
    conn.execute_many(
        &format!("insert into {T_SEED} (ID) values (?)"),
        (0..10i32).map(|i| (i,)),
    )
    .await
    .expect("种子表插入失败");

    let sql = format!(
        "select rownum as n from {T_SEED} a, {T_SEED} b, {T_SEED} c, {T_SEED} d, {T_SEED} e"
    );
    let mut stream = conn.fetch_stream(&sql, ()).await.expect("开流失败");

    let mut count = 0i64;
    let mut sum = 0i64;
    while let Some(item) = next_row(&mut stream).await {
        let row = item.expect("流中途报错");
        sum += row.get::<i64>("N").expect("rownum 应当是整数");
        count += 1;
    }
    assert_eq!(count, EXPECTED, "流必须把 10 万行全部送到");
    assert_eq!(sum, SUM, "逐行累加必须与 1…100000 的和一致");

    // `into_conn` 之后连接还能用（流把连接还回来了，不是吃掉）。
    let mut conn = stream.into_conn().await;
    assert_eq!(count_rows(&mut conn, T_TXN).await, 0);

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

// ---------------------------------------------------------------------------
// T6.4：批量写走列式参数数组绑定（§4.7 / §6.7 / §6.12 第 7 条）
// ---------------------------------------------------------------------------

/// 1000 行批插：一次 `SQLExecute` 打完整批，服务端必须真的多出 1000 行。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_batch_insert_writes_a_thousand_rows_through_the_array_path() {
    const ROWS: i32 = 1000;

    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let mut conn = conn_of(&pool).await;
    let outcome = conn
        .execute_many(
            &format!("insert into {T_BATCH} (ID, V) values (?, ?)"),
            (0..ROWS).map(|i| (i, i * 2)),
        )
        .await
        .expect("数组绑定批插不该报错");

    assert_eq!(outcome.failed_at, None, "1000 行都不违约：{outcome:?}");
    assert!(outcome.is_complete(), "{outcome:?}");
    assert_eq!(
        outcome.executed, ROWS as usize,
        "executed 应当是「服务端报成功状态的行数」"
    );
    assert_eq!(
        outcome.affected,
        u64::from(ROWS as u32),
        "affected 应当等于插入行数：{outcome:?}"
    );
    assert_eq!(
        count_rows(&mut conn, T_BATCH).await,
        i64::from(ROWS),
        "服务端必须真的多出 1000 行"
    );

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// 批量中途违约的**实测行为**：达梦把整批在**语句级**打回，不留部分成果。
///
/// # 实测结论（2026-02，DM8 @ localhost:5236，本条用例就是证据）
///
/// 铺底 0..500 之后，再打一批 1000 行、其中第 500 行的 `V` 撞唯一索引 `IDX_..._V`，
/// 实测拿到的是：
///
/// ```text
/// Err(DmError { kind: Server { sqlstate: [50,51,48,48,48], code: -6602,
///               message: "违反表[DM_Q_REAL_BATCH]唯一性约束条件[CONS134226715]" },
///               context: ErrorContext { stage: Execute, .. } })
/// ```
///
/// 也就是说：**`SQLExecute` 直接返错，不是 `SQL_SUCCESS_WITH_INFO` + 参数状态数组里
/// 标某一行失败**。ODBC 的 `PARAM_STATUS_PTR` 只有在 `SQLExecute` 返回
/// `SQL_SUCCESS_WITH_INFO` 时才有行级意义；达梦这里走的是整条语句失败那条路，
/// 所以列式数组绑定呈现不出「第 500 行的下标」，行级信息拿不到。
///
/// 服务端的结果是**原子**的：批量里一行都没落地，表里仍然只有铺底那 500 行
/// （ODBC 的自动提交语义下，一条失败的 `SQLExecute` 不留部分成果）。
///
/// # 为什么这条用例的断言长这样
///
/// 设计文档 §6.7 原本期望「行级失败记进 `BatchOutcome.failed_at`，不抬成 `Err`」。
/// 这个期望**只在驱动愿意用 `SQL_SUCCESS_WITH_INFO` 报行级错误时**才成立；达梦不
/// 愿意。所以这里把实测口径钉下来（`Err` + `stage: Execute` + 服务端原子），
/// 并把 §6.7 需要回写的地方记在这条用例里，而不是猜一个 `failed_at = Some(500)` 出来
/// 骗过测试。
///
/// 顺带验掉两件容易坏的事：① 这批报错之后，连接和语句缓存还能不能继续用
/// （同一条池化连接上接着跑单行 `execute`）；② 服务端真的**一行都没留**。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_batch_insert_reports_a_mid_batch_violation_as_a_statement_error() {
    // 铺底 0..500：批量里 (ID=500, V=0) 会撞上 `IDX_..._V` 唯一索引。
    const SEEDED: i32 = 500;
    const VIOLATOR: i32 = 500;

    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let sql = format!("insert into {T_BATCH} (ID, V) values (?, ?)");

    let mut conn = conn_of(&pool).await;

    {
        let seeded = conn
            .execute_many(&sql, (0..SEEDED).map(|i| (i, i)))
            .await
            .expect("铺底不该失败");
        assert_eq!(seeded.failed_at, None, "铺底：{seeded:?}");
        assert_eq!(count_rows(&mut conn, T_BATCH).await, i64::from(SEEDED));
    }

    let failed = conn.execute_many(&sql, (0..1000i32).map(|i| (i, i))).await;

    // 实测：语句级 `Err`，不是 `Ok(BatchOutcome { failed_at })`。
    let err = failed.expect_err(
        "实测口径：达梦对批量中的唯一性违约返回 SQLExecute 错误，\
         列式数组绑定拿不到行级状态数组",
    );
    match &err.kind {
        DmErrorKind::Server { code, .. } => {
            assert_eq!(*code, -6602, "唯一性约束违约的达梦原生错误码：{err:?}")
        }
        other => panic!("实测应当是服务端错误，而不是 {other:?}：{err:?}"),
    }
    assert_eq!(
        err.context.stage,
        dm_error::Stage::Execute,
        "错误发生在一句 SQLExecute 上：{err:?}"
    );

    // 服务端原子性：这批一行都没落地，只留铺底 500 行。
    let landed = count_rows(&mut conn, T_BATCH).await;
    assert_eq!(
        landed,
        i64::from(SEEDED),
        "实测：这批一行都没落地，只留铺底的 {SEEDED} 行（实际 {landed}）"
    );

    // 语句缓存回归：这句 SQL 的预处理语句刚被数组绑定 + 失败用过，现在必须回到单行口径。
    let affected = conn
        .execute(&sql, (VIOLATOR, VIOLATOR))
        .await
        .expect("批量失败之后的单行执行必须照常工作（PARAMSET_SIZE 必须已复位）");
    assert_eq!(affected, 1, "单行插入只影响一行");
    assert_eq!(
        count_rows(&mut conn, T_BATCH).await,
        i64::from(SEEDED) + 1,
        "铺底 500 + 刚才这 1 行"
    );

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// 语句缓存回归：数组绑定用过的预处理语句，下一组参数必须回到单行口径。
///
/// # 为什么这条最要紧
///
/// `PARAMSET_SIZE` 是**语句属性**。数组绑定把它设成 N 之后如果不复位，语句一旦被
/// 语句缓存或池化连接复用，后面每一次普通单行 `execute` 都会带着「要 N 组参数」的
/// 期望去执行。池里只有 1 条连接，所以两次 `conn_of` 借到的是同一条。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_statement_reuse_after_array_binding_still_runs_single_rows() {
    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let sql = format!("insert into {T_BATCH} (ID, V) values (?, ?)");

    {
        let mut conn = conn_of(&pool).await;
        let outcome = conn
            .execute_many(&sql, (0..16i32).map(|i| (i, i)))
            .await
            .expect("数组绑定批插失败");
        assert_eq!(outcome.failed_at, None, "{outcome:?}");
    }

    {
        let mut conn = conn_of(&pool).await;
        // 借回的同一条连接上跑单行：如果 `PARAMSET_SIZE` 没复位，这一句就是第一个
        // 撞上去的。用同一个 SQL 文本，尽量命中同一条缓存语句。
        let affected = conn
            .execute(&sql, (16i32, 16i32))
            .await
            .expect("数组绑定之后的单行执行必须照常工作");
        assert_eq!(affected, 1, "单行插入只影响一行");
        assert_eq!(count_rows(&mut conn, T_BATCH).await, 17, "16 + 1");
    }

    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// V-07 的原始数据：同样的 1000 行，逐行循环 vs 列式数组绑定，各测一次耗时。
///
/// 用 `std::time::Instant`，不引 criterion。两条路跑在同一张表、同样的参数形状上；
/// 报告直接打在测试输出里（跑的时候加 `--nocapture`）。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_batch_insert_times_the_row_loop_against_array_binding() {
    const ROWS: i32 = 1000;

    let Some(pool) = real_pool(4).await else {
        return;
    };
    setup(&pool).await;

    let sql = format!("insert into {T_BATCH} (ID, V) values (?, ?)");
    let mut conn = conn_of(&pool).await;

    // 逐行循环：每行一次 bind + execute。等价于 `execute_param_array` 默认实现那条路。
    let mut row_affected = 0u64;
    let row_start = Instant::now();
    for i in 0..ROWS {
        row_affected += conn.execute(&sql, (i, i * 2)).await.expect("逐行插入失败");
    }
    let row_elapsed = row_start.elapsed();
    assert_eq!(row_affected, u64::from(ROWS as u32));

    // 数组绑定：一次 `SQLExecute` 打完整批。
    let array_start = Instant::now();
    let array_outcome = conn
        .execute_many(&sql, (ROWS..ROWS * 2).map(|i| (i, i * 2)))
        .await
        .expect("数组绑定批插失败");
    let array_elapsed = array_start.elapsed();
    assert_eq!(array_outcome.failed_at, None, "{array_outcome:?}");

    assert_eq!(
        count_rows(&mut conn, T_BATCH).await,
        i64::from(ROWS) * 2,
        "两条路各插 1000 行"
    );

    let row_ms = row_elapsed.as_secs_f64() * 1000.0;
    let array_ms = array_elapsed.as_secs_f64() * 1000.0;
    let ratio = row_ms / array_ms.max(f64::EPSILON);
    println!(
        "\n== V-07 批量写 1000 行的实测耗时（达梦 DM8 @ localhost:5236） ==\n\
         逐行循环  (bind+execute × {ROWS}): {row_ms:>8.1} ms\n\
         数组绑定  (一次 SQLExecute)    : {array_ms:>8.1} ms\n\
         倍数（逐行 / 数组）            : {ratio:>8.2}x\n\
         数组 outcome : {array_outcome:?}\n"
    );

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

// ---------------------------------------------------------------------------
// AC-09：慢查询告警（真机长查询按阈值打 warn，字段含指纹与行数）
// ---------------------------------------------------------------------------

/// 把日志收进内存的写入器（只在本文件里用，目标是**断言 warn 真的打了**）。
#[derive(Clone, Default)]
struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("日志锁中毒").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// 装一次全局订阅者（进程内只装一次）。返回的句柄可反复 `clone`，
/// 捕获到的文本按「装的时刻起」累积。
fn install_log_capture() -> LogCapture {
    static CAPTURE: std::sync::OnceLock<LogCapture> = std::sync::OnceLock::new();
    CAPTURE
        .get_or_init(|| {
            let capture = LogCapture::default();
            let subscriber = tracing_subscriber::fmt()
                .with_writer(capture.clone())
                .with_max_level(tracing::Level::WARN)
                .with_ansi(false)
                .with_target(false)
                .finish();
            // 本文件里只有本用例装订阅者；真装不上（别的用例先装了）也不 panic，
            // 后面的断言会以「没捕获到日志」失败，不会变成假绿。
            let _ = tracing::subscriber::set_global_default(subscriber);
            capture
        })
        .clone()
}

/// AC-09 的最后一条证据：真机长查询必须按 `slow_query_threshold` 打 `warn!`，
/// 且这条日志里带 SQL **指纹**（字面量已被换成 `?`）与本次**行数**；
/// 阈值设成 `ZERO` 时则**一条都不打**。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_slow_query_warns_with_fingerprint_and_rows() {
    let Some(mut cfg) = real_config(2) else {
        return;
    };
    // 阈值压到 1 ms：门禁里不可能真等一秒，所以拿一条**足够重**的聚合语句跨过门槛
    // （见下：语句里 10^6 行的那 7 路自连接）。
    cfg.slow_query_threshold = Duration::from_millis(1);
    let pool = DmPool::connect_with(cfg, ExecutorConfig::for_pool(2))
        .await
        .expect("真机上建池失败");
    setup(&pool).await;

    // 种子表恰好 10 行（与流式用例共用同一张表）：`setup` 只建表**不插数据**，
    // 少了这一步，下面的 5 路自连接会安静地回 0 行（首轮就是这么红的）。
    let mut conn = conn_of(&pool).await;
    conn.execute_many(
        &format!("insert into {T_SEED} (ID) values (?)"),
        (0..10i32).map(|i| (i,)),
    )
    .await
    .expect("种子表插入失败");

    let capture = install_log_capture();
    let mark = capture.0.lock().expect("日志锁中毒").len();

    // 10^5 行的**真取数**：`(10 行的子查询) × 4 张 10 行的表` = 10^5 行。为什么不是
    // 「一条很重的聚合」：实测 7 路自连接的 `count(*)`（10^6 行）只要 **667 µs**——
    // 达梦会把这种可折叠的聚合算成一步，慢不下来；真正超过 1 ms 的是把 10 万行搬回
    // 客户端（AC-12 量过 `select 1` 全栈 P99 只有 96 µs，所以「慢」只能来自真实取数）。
    //
    // 字面量放在**子查询**里（而不是末尾的 `where`）：指纹的 `Display` 只印前
    // `DISPLAY_CHARS = 96` 个字符（`crates/dm-query/src/fingerprint.rs:25`），放在末尾
    // 的 `where a.ID >= 0` 会被截掉，日志里就看不到「字面量已换成 `?`」这件事了。
    let sql = format!(
        "select rownum as n from (select ID from {T_SEED} where ID >= 0) a, {T_SEED} b, \
         {T_SEED} c, {T_SEED} d, {T_SEED} e"
    );
    let started = Instant::now();
    let rows = conn.query(&sql, ()).await.expect("长查询失败");
    let elapsed = started.elapsed();
    assert_eq!(rows.len(), 100_000, "5 路自连接应当回 10^5 行");
    assert!(
        elapsed >= Duration::from_millis(1),
        "这条语句只跑了 {elapsed:?}，没跨过 1 ms 门槛，慢查询自然打不出来"
    );

    let text = {
        let buf = capture.0.lock().expect("日志锁中毒");
        String::from_utf8_lossy(&buf[mark..]).into_owned()
    };
    assert!(
        !text.is_empty(),
        "阈值 1 ms 的长查询必须触发慢查询 warn，但一条日志都没捕获到"
    );
    assert!(text.contains("慢查询"), "warn 文案不对：{text}");
    // `tracing-subscriber` 的 fmt 层给字符串字段加引号（`op="query"`），
    // 而指纹的**大小写**属于 `Fingerprint` 的内部口径而不是本用例要断言的东西：
    // 先把引号去掉、统一小写，再断言字段与指纹内容，免得断言被这两件事绊倒。
    let flat = text.replace('"', "").to_lowercase();
    assert!(flat.contains("op=query"), "缺 op 字段：{text}");
    assert!(flat.contains("sql_fp="), "缺 sql_fp 字段：{text}");
    assert!(
        flat.contains("threshold_ms=1"),
        "缺 threshold_ms 字段：{text}"
    );
    assert!(
        flat.contains("where id >= ?"),
        "指纹必须把字面量换成 `?`：{text}"
    );
    assert!(
        flat.contains("rows=100000"),
        "这次取数回了 10^5 行，rows 字段应当是 100000：{text}"
    );
    println!("\n== AC-09 慢查询 warn（真机 DM8；语句实测 {elapsed:?}） ==\n{text}");

    // 另一半判据：阈值设成 ZERO（关闭）时**一条都不打**。
    drop(conn);
    let Some(mut off_cfg) = real_config(2) else {
        return;
    };
    off_cfg.slow_query_threshold = Duration::ZERO;
    let pool_off = DmPool::connect_with(off_cfg, ExecutorConfig::for_pool(2))
        .await
        .expect("真机上建池失败");
    let off_mark = capture.0.lock().expect("日志锁中毒").len();
    let mut conn_off = conn_of(&pool_off).await;
    conn_off
        .query(&sql, ())
        .await
        .expect("关闭慢查询后长查询仍须成功");
    let off_text = {
        let buf = capture.0.lock().expect("日志锁中毒");
        String::from_utf8_lossy(&buf[off_mark..]).into_owned()
    };
    assert!(
        !off_text.contains("慢查询"),
        "阈值 ZERO 必须关掉慢查询告警，但捕获到了：{off_text}"
    );

    drop(conn_off);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
    pool_off.close().await.expect("关池失败");
    pool_off.wait_closed().await;
}

// ---------------------------------------------------------------------------
// T6.3：真机分块读（`LobRef`/`LobStream`）
// ---------------------------------------------------------------------------

/// 冻结面 `LobRef`/`LobStream` 的真机判据：1.2 MB 的 CLOB 分块读，逐字节一致。
///
/// 离线那一半（`tests/lob.rs`）钉的是窗口切分、NULL/空值、出错重试、连接归属；
/// 这里钉的是**真驱动**那一半：一趟的窗口真的由 `SQLGetData` 决定（不放大成整值读），
/// 值真的一致。
///
/// 用 400000 个 **BMP** 字符（`"达梦"` × 200000，1.2 MB UTF-8）：BMP 文本的整值往返是
/// §6.5 保证的，于是「内容不一致」只可能是分块读的错，而不是驱动那条增补平面缺陷。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_lob_stream_reads_a_one_mib_clob_in_windows() {
    let Some(pool) = real_pool(2).await else {
        return;
    };
    setup(&pool).await;

    let text = "达梦".repeat(200_000);

    {
        let mut conn = conn_of(&pool).await;
        for sql in [
            format!("drop table if exists {T_LOB}"),
            format!("create table {T_LOB} (ID INT PRIMARY KEY, BODY CLOB)"),
        ] {
            conn.execute(&sql, ())
                .await
                .unwrap_or_else(|err| panic!("建表语句失败：{sql}\n{err:?}"));
        }
        conn.execute(
            &format!("insert into {T_LOB} (ID, BODY) values (?, ?)"),
            (1i32, text.clone()),
        )
        .await
        .expect("插入 1.2 MB 的 CLOB 失败");
    }

    let conn = conn_of(&pool).await;
    let mut stream = conn
        .lob_stream(
            &format!("select BODY from {T_LOB} where ID = ?"),
            (1i32,),
            LobRef::name("BODY"),
        )
        .await
        .expect("开分块读失败");
    assert_eq!(stream.column().name, "BODY");

    assert!(stream.next_row().await.expect("定位第一行失败"));
    let mut value = Vec::new();
    let mut chunks = 0usize;
    let mut max_added = 0usize;
    loop {
        let before = value.len();
        let state = stream.read_chunk(&mut value).await.expect("分块读失败");
        max_added = max_added.max(value.len() - before);
        chunks += 1;
        if state.is_done() {
            break;
        }
    }

    // 一趟的窗口是 2048 个码元；BMP 字符最多 3 字节 UTF-8，所以一趟最多 6144 字节
    // （留一个码元的余量给跨趟的高代理）。**这一条才是「不放大」的功能性证据**：
    // 1.2 MB 的值从来不会在一趟里出现。
    //
    // 只对「按窗口交付」的后端成立：`dm-dpi` 是整值一趟（`dpi_get_data` 的分块模式
    // 会在块尾塞 NUL），窗口由后端自己保证；内容逐字节一致那条断言两种后端都必须过。
    if lob_is_windowed() {
        let ceiling = 3 * (dm_odbc::lob::LOB_CHUNK_BYTES / 2 + 1);
        assert!(
            max_added <= ceiling,
            "单趟交付了 {max_added} 字节，超过窗口上界 {ceiling}：分块读退化成了整值读"
        );
        // 400000 码元 ÷ 2048 = 196 趟（趟数明显偏少就说明驱动把整值一趟给了我们）。
        assert!(
            chunks >= 190,
            "只分了 {chunks} 趟（窗口 2048 码元，应当约 196 趟）"
        );
    } else {
        println!("[LOB] 后端整值交付：{chunks} 趟、单趟最大 {max_added} 字节（窗口断言跳过）");
    }
    assert_eq!(value.len(), text.len(), "分块读回的字节数不一致");
    assert!(value == text.as_bytes(), "分块读回的内容必须逐字节一致");
    assert_eq!(stream.bytes_read(), text.len(), "流自报的字节数要对得上");

    assert!(!stream.next_row().await.expect("取尽失败"), "只有一行");
    assert!(stream.is_eof());
    drop(stream);
    println!(
        "\n== T6.3 真机分块读 ==\n{chunks} 趟、单趟最多 {max_added} 字节、共 {} 字节（与源值一致）",
        value.len()
    );

    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

// ---------------------------------------------------------------------------
// T6.3：真机分块写（`PooledConn::execute_lob`，SQLPutData 延迟输入）
// ---------------------------------------------------------------------------

/// 建 `T_LOB_W` 并清空。
async fn setup_lob_write(conn: &mut PooledConn<OdbcBackend>) {
    for sql in [
        format!("drop table if exists {T_LOB_W}"),
        format!("create table {T_LOB_W} (ID INT PRIMARY KEY, B BLOB, C CLOB)"),
    ] {
        conn.execute(&sql, ())
            .await
            .unwrap_or_else(|err| panic!("建表语句失败：{sql}\n{err:?}"));
    }
}

/// 3 MiB 的 BLOB 用**内存里的值**流式写进去，回读逐字节一致。
///
/// 离线那一半（`tests/lob_write.rs`）钉的是 `?` 位置、源分趟、错误分类与语句缓存；
/// 这里钉的是**真驱动**那一半：达梦真的回 `SQL_NEED_DATA`（探针
/// `crates/dm-odbc/examples/putdata_probe.rs` 的 8 个用例），我们把 3 MiB 分 ~48 趟
/// 交出去、服务端存下的字节与源逐字节一致。
///
/// 用 3 MiB 而不是 1 MiB：值得超过一次 `LOB_WRITE_CHUNK_BYTES`（64 KiB）几十倍，
/// 「只送了第一趟就报成功」这类错会立刻暴露成内容不一致。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_lob_write_sends_a_three_mib_blob_in_windows() {
    let Some(pool) = real_pool(2).await else {
        return;
    };
    setup(&pool).await;

    // 确定性的伪随机字节：跨趟边界的内容各不相同，一旦串块或丢块立刻不一致。
    let mut state = 0x1234_5678u32;
    let blob: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();

    {
        let mut conn = conn_of(&pool).await;
        setup_lob_write(&mut conn).await;
        let affected = conn
            .execute_lob(
                &format!("insert into {T_LOB_W} (ID, B) values (?, ?)"),
                (1i32,),
                LobParam::new(1, LobKind::Binary, LobBytes::new(blob.clone())),
            )
            .await
            .expect("3 MiB 的 BLOB 分块写失败");
        assert_eq!(affected, 1);
    }

    let mut conn = conn_of(&pool).await;
    let row = conn
        .query_one(&format!("select B from {T_LOB_W} where ID = ?"), (1i32,))
        .await
        .expect("回读查询失败")
        .expect("必须有一行");
    let back = row.get::<Vec<u8>>(0).expect("B 应当是二进制");
    assert_eq!(back.len(), blob.len(), "回读的长度不一致");
    assert!(back == blob, "回读的内容必须逐字节一致");
    println!(
        "\n== T6.3 真机分块写（BLOB）==\n{} 字节往返一致（每趟上限 {} 字节）",
        blob.len(),
        dm_core::LOB_WRITE_CHUNK_BYTES
    );

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

/// 约 1.1 MB 的 CLOB 用 `LobReader`（真流式源）写进去，回读的文本逐字符一致。
///
/// 文本前段刻意用 3 字节一个的汉字：`LOB_WRITE_CHUNK_BYTES`（64 KiB）不是 3 的倍数
/// （`65536 % 3 == 1`），所以第一个写窗口的边界**一定落在汉字内部**——这正好逼出驱动层
/// [`dm_odbc::lob::LobFeeder`] 的 `carry`（半个 UTF-8 字符留到下一趟）与 UTF-16LE 转换；
/// 后段再混排 1 字节 ASCII，让后续窗口的编码长度也不是定长。用例里对这一点有显式断言，
/// 免得以后有人把文本换成对齐的拼法、让这段代码白测。
#[tokio::test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
async fn real_lob_write_streams_a_clob_from_a_reader() {
    let Some(pool) = real_pool(2).await else {
        return;
    };
    setup(&pool).await;

    let text = format!(
        "{}{}",
        "达梦数据库".repeat(20_000),
        "abcDM达梦123".repeat(30_000)
    );
    let bytes = text.as_bytes().to_vec();
    let cut = dm_core::LOB_WRITE_CHUNK_BYTES;
    assert!(
        bytes.len() > cut,
        "文本（{} 字节）要长过一个写窗口（{cut} 字节）",
        bytes.len()
    );
    assert!(
        bytes[cut] & 0xC0 == 0x80,
        "第 {cut} 字节必须落在多字节字符内部（实际是 {:02X}）——落不到，`carry` 就没被测到",
        bytes[cut]
    );

    {
        let mut conn = conn_of(&pool).await;
        setup_lob_write(&mut conn).await;
        // 真流式源：驱动层一趟一趟从 `std::io::Read` 里取，不在内存里预先摊平。
        let reader = std::io::Cursor::new(bytes.clone());
        let affected = conn
            .execute_lob(
                &format!("insert into {T_LOB_W} (ID, C) values (?, ?)"),
                (2i32,),
                LobParam::new(1, LobKind::Text, LobReader::new(reader)),
            )
            .await
            .expect("CLOB 分块写失败");
        assert_eq!(affected, 1);
    }

    let mut conn = conn_of(&pool).await;
    let row = conn
        .query_one(&format!("select C from {T_LOB_W} where ID = ?"), (2i32,))
        .await
        .expect("回读查询失败")
        .expect("必须有一行");
    let back = row.get::<String>(0).expect("C 应当是文本");
    assert_eq!(back.len(), text.len(), "回读的字符数不一致");
    assert!(back == text, "回读的文本必须逐字符一致");
    println!(
        "\n== T6.3 真机分块写（CLOB）==\n{} 字节 / {} 字符往返一致",
        bytes.len(),
        back.chars().count()
    );

    drop(conn);
    pool.close().await.expect("关池失败");
    pool.wait_closed().await;
}

// ---------------------------------------------------------------------------
// 已移除：定位「保存点之后那次提交去哪了」的临时探针
// ---------------------------------------------------------------------------
// 这里原本有一个只打印不断言的变体矩阵（A–H）与两个帮手（`clear_txn`/`probe_ids`），
// 用来定位 DM8 ODBC 驱动「`ROLLBACK TO SAVEPOINT` 紧接 `SQLEndTran(SQL_COMMIT)`
// 会被静默丢掉」的缺陷。缺陷已修（见 `crates/dm-query/src/txn.rs` 的
// `Transaction::rollback_to`），钉住语义的正式用例是上面那两条
// `real_savepoint_commit_*`；驱动层的诊断矩阵保留在
// `crates/dm-odbc/examples/savepoint_probe.rs`（R0–R21），不在这里重复。

// ---------------------------------------------------------------------------
// 内存预算：G4 / T4.7、T5.2、T6.3 里「峰值内存不随数据量放大」那一半
// ---------------------------------------------------------------------------
//
// 量法见 `tests/meter/mod.rs`。三条共同纪律：**先拿闸门、再开窗**；窗口里只放被测负载
// （建连接、灌数据都在 `open()` 之前做完）；每条都跟一个**阳性对照**。
//
// 本组按文件头的跑法用 `--include-ignored --test-threads=1`：计数是**进程全局**的，
// 别的用例同时在分配就会污染窗口（闸门只能串行化同样拿闸门的用例）。
//
// 真机上的边界：看得见的是**我们与 `odbc-api`** 的分配；驱动 DLL 内部的 `malloc` 走它
// 自己的 CRT。所以钉的是「中间件这一层不随数据量放大」，不是「整进程 RSS」。

/// 内存预算用例的种子表：10 行、每行 [`BUDGET_PAYLOAD`] 字节文本；`ways` 路自连接 ⇒ `10^ways` 行。
const T_BUDGET: &str = "DM_Q_REAL_BUDGET";
/// 种子行数（自连接的基数：行数正好是 10 的幂）。
const BUDGET_SEED: usize = 10;
/// 每行文本长度：行数 × 它 = 「要是全留在内存里」的量级。
const BUDGET_PAYLOAD: usize = 120;

/// 一次窗口的读数。三个数的含义见 `tests/meter/mod.rs`。
struct Reading {
    /// 窗口内峰值活字节相对窗口开始的增量。
    peak: usize,
    /// 窗口内新分配的总字节数（含瞬时）。
    total: usize,
    /// 窗口结束时的净活字节变化。
    net: isize,
}

impl Reading {
    /// 取读数。**先 drop 掉被测负载再调**：这样 `net` 问的是「全都放开之后还剩多少」，
    /// 而 `peak` 是高水位，drop 不会改它。
    fn take(window: &meter::Window) -> Reading {
        Reading {
            peak: window.peak_growth(),
            total: window.total_growth(),
            net: window.net_growth(),
        }
    }
}

/// 真机测量用的运行时：**带 `enable_all()`**——池的超时走 `tokio::time`，ODBC 调用走
/// `spawn_blocking`。`meter::runtime()` 那个只带时间驱动，是给离线假后端的。
///
/// 用同步 `#[test]` + 手动 `block_on` 的理由：闸门要跨 `await` 持有，写成
/// `#[tokio::test] async fn` 会踩 clippy 的 `await_holding_lock`（假阳性）。
fn budget_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("建真机测量运行时失败")
}

/// `ways` 路自连接：种子表恰好 10 行，所以行数正好是 `10^ways`。
fn budget_sql(ways: usize) -> String {
    assert!((1..=5).contains(&ways), "自连接路数要在 1..=5：{ways}");
    let mut sql = format!("select a.ID, a.V from {T_BUDGET} a");
    for alias in ["b", "c", "d", "e"].iter().take(ways - 1) {
        sql.push_str(&format!(", {T_BUDGET} {alias}"));
    }
    sql
}

/// 建表 + 灌 10 行带载荷的种子数据。
async fn setup_budget(conn: &mut PooledConn<OdbcBackend>) {
    for sql in [
        format!("drop table if exists {T_BUDGET}"),
        format!("create table {T_BUDGET} (ID INT PRIMARY KEY, V VARCHAR(400))"),
    ] {
        conn.execute(&sql, ())
            .await
            .unwrap_or_else(|err| panic!("建表语句失败：{sql}\n{err:?}"));
    }
    let text = "x".repeat(BUDGET_PAYLOAD);
    // 逐行 `execute` 而不是 `execute_many`：这里要的是「行数与载荷确定」，不测批量写
    // （批量写有自己的用例），也就不必赌驱动对变长列的数组绑定。
    for id in 0..BUDGET_SEED as i32 {
        let affected = conn
            .execute(
                &format!("insert into {T_BUDGET} (ID, V) values (?, ?)"),
                (id, text.clone()),
            )
            .await
            .unwrap_or_else(|err| panic!("插入种子行 {id} 失败：{err:?}"));
        assert_eq!(affected, 1);
    }
}

/// 流式取一次，返回（行数、收回的载荷字节、读数）。流在本函数里 drop，所以 `net` 是
/// 「全都放开之后还剩多少」。
async fn stream_budget(pool: &DmPool<OdbcBackend>, ways: usize) -> (usize, usize, Reading) {
    let sql = budget_sql(ways);
    // 连接必须在窗口**之前**拿到：`fetch_stream` 是按值拿走 `self` 的（流要拥有这条连接，
    // 见 `PooledConn::fetch_stream` 的「为什么它按值拿走 `self`」），池手上就那一条空闲
    // 连接，所以每次拿到的都是热的同一条。
    let conn = conn_of(pool).await;
    let window = meter::Window::open();
    let mut rows = 0usize;
    let mut payload = 0usize;
    {
        let mut stream = conn.fetch_stream(&sql, ()).await.expect("开行流失败");
        while let Some(row) = next_row(&mut stream).await {
            let row = row.expect("行流报错");
            let id = row.get::<i32>(0).expect("ID 应当是整数");
            assert!(id >= 0, "ID 不该是负数：{id}");
            payload += row.get::<String>(1).expect("V 应当是文本").len();
            rows += 1;
        }
    }
    (rows, payload, Reading::take(&window))
}

/// 建 `{T_LOB}` 并用**我们自己的分块写**灌值：整值参数会撞驱动的参数长度上限，而写路径
/// 本来就有独立用例（上面那两条 `real_lob_write_*`）。
async fn seed_lob_table(conn: &mut PooledConn<OdbcBackend>, values: &[(i32, &str)]) {
    for sql in [
        format!("drop table if exists {T_LOB}"),
        format!("create table {T_LOB} (ID INT PRIMARY KEY, BODY CLOB)"),
    ] {
        conn.execute(&sql, ())
            .await
            .unwrap_or_else(|err| panic!("建表语句失败：{sql}\n{err:?}"));
    }
    for (id, text) in values {
        let affected = conn
            .execute_lob(
                &format!("insert into {T_LOB} (ID, BODY) values (?, ?)"),
                (*id,),
                LobParam::new(1, LobKind::Text, LobBytes::new(text.as_bytes().to_vec())),
            )
            .await
            .unwrap_or_else(|err| panic!("灌 CLOB({id}) 失败：{err:?}"));
        assert_eq!(affected, 1);
    }
}

/// 分块读一次，返回（读到的总字节、趟数、读数）。
///
/// `accumulate = false` 是**正确用法**：每趟读完就丢，我们这一侧只占一个窗口。
/// `true` 是**阳性对照**：把整值攒进 `value`，仪器必须看得见整个值。
async fn read_clob_budget(
    pool: &DmPool<OdbcBackend>,
    id: i32,
    accumulate: bool,
) -> (usize, usize, Reading) {
    // 同 `stream_budget`：`lob_stream` 也按值拿走连接，所以先拿连接再开窗口。
    let conn = conn_of(pool).await;
    let window = meter::Window::open();
    let mut total = 0usize;
    let mut trips = 0usize;
    {
        let mut stream = conn
            .lob_stream(
                &format!("select BODY from {T_LOB} where ID = ?"),
                (id,),
                LobRef::name("BODY"),
            )
            .await
            .expect("开分块读失败");
        assert!(stream.next_row().await.expect("定位第一行失败"), "没读到行");
        let mut value: Vec<u8> = Vec::new();
        loop {
            if !accumulate {
                value.clear();
            }
            let before = value.len();
            let state = stream.read_chunk(&mut value).await.expect("分块读失败");
            total += value.len() - before;
            trips += 1;
            if state.is_done() {
                break;
            }
        }
    }
    (total, trips, Reading::take(&window))
}

/// 分块写一次（BLOB），返回读数。源 `Vec` 由调用方在**窗口外**建好再 move 进来——
/// `LobBytes` 的契约就是「值已经在内存里」，被测的是「按窗口交出去」那一段。
async fn write_blob_budget(conn: &mut PooledConn<OdbcBackend>, id: i32, blob: Vec<u8>) -> Reading {
    let window = meter::Window::open();
    let affected = conn
        .execute_lob(
            &format!("insert into {T_LOB_W} (ID, B) values (?, ?)"),
            (id,),
            LobParam::new(1, LobKind::Binary, LobBytes::new(blob)),
        )
        .await
        .expect("分块写失败");
    assert_eq!(affected, 1);
    Reading::take(&window)
}

/// 确定性伪随机字节（同上文那条 3 MiB 用例用的 LCG，不引 `rand`）。
fn budget_blob(len: usize) -> Vec<u8> {
    let mut state: u32 = 0x9E37_79B9;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push((state >> 24) as u8);
    }
    out
}

/// T5.2 的内存那一半：**行流的峰值由一批决定，不由行数决定**。
///
/// 1 万行 → 10 万行（每行 120 字节 ⇒ 载荷 1.2 MB → 12 MB）：峰值不该跟着涨，更不该到
/// 载荷量级。阳性对照：同样的 SQL 走 `conn.query`（`RowSet` 是**全量物化**，见
/// `crates/dm-query/src/rowset.rs` 的文件头），1 万行必须被仪器看见。
#[test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
fn real_row_stream_memory_does_not_follow_the_row_count() {
    let _gate = meter::serialize();
    let rt = budget_runtime();
    rt.block_on(async {
        let Some(pool) = real_pool(2).await else {
            return;
        };
        setup(&pool).await;

        {
            let mut conn = conn_of(&pool).await;
            setup_budget(&mut conn).await;
        }

        // 热身（也是第一次真读数）：2 路 = 100 行，把池、语句、行缓冲的一次性分配挪出窗口。
        let (warm_rows, warm_payload, _) = stream_budget(&pool, 2).await;
        assert_eq!(warm_rows, BUDGET_SEED * BUDGET_SEED);
        assert_eq!(warm_payload, warm_rows * BUDGET_PAYLOAD);

        let (ten_k_rows, ten_k_payload, ten_k) = stream_budget(&pool, 4).await;
        let (hundred_k_rows, hundred_k_payload, hundred_k) = stream_budget(&pool, 5).await;
        assert_eq!(ten_k_rows, 10_000);
        assert_eq!(hundred_k_rows, 100_000);
        assert_eq!(ten_k_payload, ten_k_rows * BUDGET_PAYLOAD);
        assert_eq!(hundred_k_payload, hundred_k_rows * BUDGET_PAYLOAD);

        // 阳性对照：`RowSet` 全量物化 1 万行（载荷 1.2 MB）——仪器必须看得见。
        let mut conn = conn_of(&pool).await;
        let window = meter::Window::open();
        let rows_in_set = conn
            .query(&budget_sql(4), ())
            .await
            .expect("全量查询失败")
            .rows()
            .len();
        let control = Reading::take(&window);
        assert_eq!(rows_in_set, 10_000);

        assert!(
            hundred_k.peak < 4 * 1024 * 1024,
            "流式取 10 万行（载荷 {}）的峰值是 {}：那是把结果攒下来的量级",
            meter::kib(hundred_k_payload),
            meter::kib(hundred_k.peak)
        );
        assert!(
            hundred_k.peak <= ten_k.peak + 256 * 1024,
            "行数 ×10 把峰值从 {} 抬到了 {}",
            meter::kib(ten_k.peak),
            meter::kib(hundred_k.peak)
        );
        assert!(
            hundred_k.net < 2 * 1024 * 1024,
            "取完之后净留下 {}：取过的行不该被留住",
            meter::kib(hundred_k.net.unsigned_abs())
        );
        assert!(
            control.peak >= 1024 * 1024,
            "阳性对照失败：全量物化 1 万行（载荷 {}）的峰值只有 {}——仪器没有灵敏度",
            meter::kib(10_000 * BUDGET_PAYLOAD),
            meter::kib(control.peak)
        );

        println!(
            "\n== T5.2 真机内存预算（行流）==\n\
             1 万行：峰值 {}、总分配 {}、净 {}\n\
             10 万行：峰值 {}、总分配 {}、净 {}\n\
             阳性对照（RowSet 全量 1 万行）：峰值 {}",
            meter::kib(ten_k.peak),
            meter::kib(ten_k.total),
            ten_k.net,
            meter::kib(hundred_k.peak),
            meter::kib(hundred_k.total),
            hundred_k.net,
            meter::kib(control.peak),
        );

        drop(conn);
        pool.close().await.expect("关池失败");
        pool.wait_closed().await;
    });
}

/// T6.3 分块**读**的内存那一半：**峰值由一趟窗口决定，不由值决定**。
///
/// 1 MiB → 4 MiB CLOB：峰值不该跟着涨。阳性对照：把整值攒在一个 `Vec` 里，4 MiB 必须
/// 被仪器看见。
#[test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
fn real_chunked_read_memory_does_not_follow_the_value() {
    let _gate = meter::serialize();
    let rt = budget_runtime();
    rt.block_on(async {
        let Some(pool) = real_pool(2).await else {
            return;
        };
        setup(&pool).await;

        // BMP 字符（每个 3 字节 UTF-8）：「达梦」×N。
        let small = "达梦".repeat(175_000); // 350 000 字符 ≈ 1.05 MB
        let large = "达梦".repeat(700_000); // 1 400 000 字符 ≈ 4.20 MB

        {
            let mut conn = conn_of(&pool).await;
            seed_lob_table(&mut conn, &[(1, small.as_str()), (2, large.as_str())]).await;
        }

        // 1 MiB 那次同时是 4 MiB 那次的**热身**：`LobState` 的窗口缓冲到这里已经分配好了。
        let (small_bytes, small_trips, small_r) = read_clob_budget(&pool, 1, false).await;
        assert_eq!(small_bytes, small.len());

        let (large_bytes, large_trips, large_r) = read_clob_budget(&pool, 2, false).await;
        assert_eq!(large_bytes, large.len());
        let (control_bytes, _, control_r) = read_clob_budget(&pool, 2, true).await;
        assert_eq!(control_bytes, large.len());

        // 窗口类断言只对按窗口交付的后端（`dm-odbc`）成立；`dm-dpi` 整值一趟，峰值
        // 必然等于值的大小——那正是它已知的取舍，不是「值被留住」。两种后端都成立的
        // 是下面两条：**读完之后不残留**、以及阳性对照有灵敏度。
        if lob_is_windowed() {
            assert!(small_trips > 1, "1 MiB 只用了 {small_trips} 趟：窗口不对");
            assert!(
                large_r.peak < 1024 * 1024,
                "分块读 {} 的峰值是 {}：值不该进内存",
                meter::kib(large.len()),
                meter::kib(large_r.peak)
            );
            assert!(
                large_r.peak <= small_r.peak + 256 * 1024,
                "值 ×4 把峰值从 {} 抬到了 {}",
                meter::kib(small_r.peak),
                meter::kib(large_r.peak)
            );
        } else {
            println!(
                "[LOB] 后端整值交付：1 MiB 用 {small_trips} 趟、4 MiB 用 {large_trips} 趟、\
                 峰值 {}（窗口断言跳过）",
                meter::kib(large_r.peak)
            );
        }
        assert!(
            large_r.net < 256 * 1024,
            "读完之后净留下 {}：读出来的值不该被留住",
            meter::kib(large_r.net.unsigned_abs())
        );
        assert!(
            control_r.peak >= large.len(),
            "阳性对照失败：把 {} 攒在一个 `Vec` 里的峰值只有 {}——仪器没有灵敏度",
            meter::kib(large.len()),
            meter::kib(control_r.peak)
        );

        println!(
            "\n== T6.3 真机内存预算（分块读）==\n\
             1 MiB：{small_trips} 趟、峰值 {}、总分配 {}\n\
             4 MiB：{large_trips} 趟、峰值 {}、总分配 {}、净 {}\n\
             阳性对照（攒整值）：峰值 {}",
            meter::kib(small_r.peak),
            meter::kib(small_r.total),
            meter::kib(large_r.peak),
            meter::kib(large_r.total),
            large_r.net,
            meter::kib(control_r.peak),
        );

        pool.close().await.expect("关池失败");
        pool.wait_closed().await;
    });
}

/// T6.3 分块**写**的内存那一半：**我们这一侧只多一个窗口**。
///
/// 源 `Vec` 在窗口外建好再 move 进去（`LobBytes` 的文档自己写着「内存曲线与整值绑定
/// 一样」）；被测的是「把它按 `LOB_WRITE_CHUNK_BYTES` 一趟一趟交出去」这段。阳性对照：
/// 把 4 MiB 的源**建在窗口里**——仪器必须看得见这 4 MiB。
#[test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
fn real_chunked_write_memory_does_not_follow_the_value() {
    let _gate = meter::serialize();
    let rt = budget_runtime();
    rt.block_on(async {
        let Some(pool) = real_pool(2).await else {
            return;
        };
        setup(&pool).await;

        let small = budget_blob(1024 * 1024);
        let large = budget_blob(4 * 1024 * 1024);
        let large_len = large.len();

        let mut conn = conn_of(&pool).await;
        setup_lob_write(&mut conn).await;

        // 1 MiB 那次同时充当热身。
        let small_r = write_blob_budget(&mut conn, 1, small).await;
        let large_r = write_blob_budget(&mut conn, 2, large).await;

        let window = meter::Window::open();
        let materialized = budget_blob(4 * 1024 * 1024);
        let control = Reading::take(&window);
        let control_len = materialized.len();
        drop(materialized);

        // 窗口类断言只对按窗口交付的后端（`dm-odbc`：`LOB_WRITE_CHUNK_BYTES` 一趟）
        // 成立。`dm-dpi` 的 `DATA_AT_EXEC` 路径要先把源读成一整块再 `put_data`（驱动
        // 的 `put_data` 每个参数只收一次整块，自己切块会把数据串到下一个参数上），
        // 所以峰值就是值的大小。两种后端都成立的是：**写完之后不残留**、阳性对照有灵敏度。
        if lob_is_windowed() {
            assert!(
                large_r.peak < 1024 * 1024,
                "分块写 {} 时我们这一侧的峰值是 {}：写窗口（{}）之外不该有别的",
                meter::kib(large_len),
                meter::kib(large_r.peak),
                meter::kib(dm_core::LOB_WRITE_CHUNK_BYTES)
            );
            assert!(
                large_r.peak <= small_r.peak + 256 * 1024,
                "值 ×4 把峰值从 {} 抬到了 {}",
                meter::kib(small_r.peak),
                meter::kib(large_r.peak)
            );
        } else {
            println!(
                "[LOB] 后端整值交付：4 MiB 写峰值 {}（窗口断言跳过）",
                meter::kib(large_r.peak)
            );
        }
        assert!(
            large_r.net < 1024 * 1024,
            "写完之后净留下 {}",
            meter::kib(large_r.net.unsigned_abs())
        );
        assert!(
            control.peak >= control_len,
            "阳性对照失败：在窗口里建 {} 的峰值只有 {}——仪器没有灵敏度",
            meter::kib(control_len),
            meter::kib(control.peak)
        );

        assert_eq!(
            count_rows(&mut conn, T_LOB_W).await,
            2,
            "两行 BLOB 都要落库"
        );

        println!(
            "\n== T6.3 真机内存预算（分块写）==\n\
             1 MiB：峰值 {}、总分配 {}、净 {}\n\
             4 MiB：峰值 {}、总分配 {}、净 {}\n\
             阳性对照（在窗口里建 4 MiB 源）：峰值 {}",
            meter::kib(small_r.peak),
            meter::kib(small_r.total),
            small_r.net,
            meter::kib(large_r.peak),
            meter::kib(large_r.total),
            large_r.net,
            meter::kib(control.peak),
        );

        drop(conn);
        pool.close().await.expect("关池失败");
        pool.wait_closed().await;
    });
}

/// T5.2「流式 RSS 平坦」的**整进程口径**那一半：上面的计数分配器只看得见我们自己（与
/// `odbc-api`）的分配，看不见**驱动 DLL 内部的 `malloc`**；RSS 看得见。
///
/// 判据：把 1 万行与 10 万行各流式抽干一次，**RSS 的增量不随行数放大**；阳性对照是
/// `RowSet` 全量物化 10 万行（载荷 12 MB）——RSS 探针必须看得见它。
///
/// 三条纪律跟内存预算那组一样：先热身一跑把池/语句/驱动游标的一次性分配做掉再取基线；
/// 顺序固定（先流式、后物化）；只断量级，不断精确字节数（RSS 是操作系统口径，别的线程
/// 也会抬它）。注意**驱动自己的缓冲**如果随行数增长，这条用例会红——那正是它想要的结果。
#[test]
#[ignore = "真机 dm-query 门禁：需要 DM_TEST_DSN（或 DM_SERVER/DM_USER/DM_PASSWORD）"]
fn real_row_stream_rss_does_not_follow_the_row_count() {
    let _gate = meter::serialize();
    let rt = budget_runtime();
    rt.block_on(async {
        let Some(pool) = real_pool(2).await else {
            return;
        };
        setup(&pool).await;

        {
            let mut conn = conn_of(&pool).await;
            setup_budget(&mut conn).await;
        }

        // 热身：2 路 = 100 行，把一次性分配都做掉，RSS 也就稳了。
        let (warm_rows, _, _) = stream_budget(&pool, 2).await;
        assert_eq!(warm_rows, BUDGET_SEED * BUDGET_SEED);

        let baseline = meter::rss_kib().expect("RSS 探针在 Windows/Linux 上都可用");
        let (ten_k_rows, _, _) = stream_budget(&pool, 4).await;
        let after_ten_k = meter::rss_kib().expect("RSS 探针刚才还可用了");
        let (hundred_k_rows, _, _) = stream_budget(&pool, 5).await;
        let after_hundred_k = meter::rss_kib().expect("RSS 探针刚才还可用了");
        assert_eq!(ten_k_rows, 10_000);
        assert_eq!(hundred_k_rows, 100_000);

        // 阳性对照：全量物化 10 万行（载荷 12 MB），RSS 必须被抬起来。
        //
        // `RowSet` 必须**活过**读数：写成 `conn.query(..).await.unwrap().rows().len()` 的话，
        // 临时值在那条语句结束时就 drop 了，大块的页会被立刻还回操作系统（实测只涨了
        // 432 KiB），对照就成了摆设。
        let control_before = meter::rss_kib().expect("RSS 探针刚才还可用了");
        let mut conn = conn_of(&pool).await;
        let set = conn
            .query(&budget_sql(5), ())
            .await
            .expect("全量查询失败");
        let rows_in_set = set.rows().len();
        let control_after = meter::rss_kib().expect("RSS 探针刚才还可用了");
        assert_eq!(rows_in_set, 100_000);
        drop(set);
        drop(conn);

        let streamed = after_hundred_k.saturating_sub(after_ten_k);
        let materialized = control_after.saturating_sub(control_before);

        println!(
            "\n== T5.2 真机 RSS（整进程口径）==\n\
             基线（热身 100 行后）：{baseline} KiB\n\
             1 万行后：{after_ten_k} KiB（较基线 +{}）\n\
             10 万行后：{after_hundred_k} KiB（较基线 +{}）\n\
             阳性对照（RowSet 全量 10 万行）：{control_before} → {control_after} KiB（+{materialized}）",
            after_ten_k.saturating_sub(baseline),
            after_hundred_k.saturating_sub(baseline),
        );

        assert!(
            streamed < 16 * 1024,
            "行数 ×10 又让 RSS 涨了 {streamed} KiB（1 万行后 {after_ten_k} → 10 万行后 {after_hundred_k}）：\
             要么是我们留住了行，要么是驱动自己按行缓冲"
        );
        assert!(
            materialized >= 8 * 1024,
            "阳性对照失败：全量物化 10 万行（载荷 {}）只让 RSS 涨了 {materialized} KiB——探针没有灵敏度",
            meter::kib(100_000 * BUDGET_PAYLOAD)
        );

        pool.close().await.expect("关池失败");
        pool.wait_closed().await;
    });
}
