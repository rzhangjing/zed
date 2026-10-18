//! `dm-migrate` 的真机门禁：版本记录表、脚本执行、失败即停（需要达梦 DM8 实例）。
//! 判据来自设计方案 §12.2 的 M7 表（T7.1）与 §15.7 的 CAP-G-01…03。
//!
//! 与 `src/sql.rs` 里的纯单元测试的分工：那组钉的是**分号切分器**（不需要数据库，
//! 默认就跑）；本组用**真达梦**钉「记录表真的建出来了、`apply_pending` 真的按序执行
//! 并记账、第二次真的跳过、坏脚本真的当场停住」。离线假后端永远证明不了这些——
//! 它连 `SELECT 1 FROM … WHERE 1 = 0` 报不报 `ObjectNotFound` 都演不出来。
//!
//! 跑法（host 是 gnu，`dlltool.exe` 不在 PATH，必须显式 msvc）：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p dm-migrate --test migrate_real -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! ## 关于共享的 `DM_SCHEMA_HISTORY`
//!
//! 本组用**默认表名**（[`HISTORY_TABLE`] 也就是 `DM_SCHEMA_HISTORY`），因为要验的正是
//! 「默认配置能不能用」。代价：它与别的 `dm-migrate` 用例共享同一张表，所以
//!
//! - 本组必须 `--test-threads=1` 跑（各用例自己 `drop table` 清干净）；
//! - **不碰**别的 crate 的表（`DM_Q_REAL_*` 等），清理只针对 `DM_SCHEMA_HISTORY`
//!   与本文件自己建的 `DM_MIG_REAL_*`。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dm_core::{DmConfig, PoolConfig};
use dm_migrate::{HISTORY_TABLE, Migrator};
// 默认后端（dpi feature 打开时是 `dm_dpi::DpiBackend`）。
use dm_pool::DefaultBackend as OdbcBackend;
use dm_pool::{DmPool, ExecutorConfig};
use dm_query::PooledConn;

// ---------------------------------------------------------------------------
// 配置：没配环境就跳过（与 `crates/dm-query/tests/query_real.rs` 同一门控方式）
// ---------------------------------------------------------------------------

/// 从环境读真机配置：`DM_TEST_DSN` 优先，否则 `DM_SERVER` 系。
fn base_config() -> Option<DmConfig> {
    if let Ok(dsn) = std::env::var("DM_TEST_DSN")
        && !dsn.trim().is_empty()
    {
        return Some(DmConfig::parse(&dsn).expect("DM_TEST_DSN 不是合法的连接串"));
    }
    if std::env::var("DM_SERVER").is_ok() {
        return Some(DmConfig::from_env().expect("DM_SERVER 环境下无法构造 DmConfig"));
    }
    None
}

/// 配好池参数的真机配置；返回 `None` 表示本机没配达梦（用例**正常返回**，不失败）。
///
/// `max_size = 2`：`apply_pending` 同时只借一条连接，2 条足以覆盖「探测连接还没还
/// 就又借一条」的情况（给 1 也能跑，借还严格串行，但 2 更能暴露借还漏洞）。
///
/// **必须是 `async`**：它只在 `setup()`（已经是 `async`）里被调用，而用例最外层已经
/// 有一个 `block_on`。写成同步函数再自己 `block_on` 就是**运行时套运行时**
/// （`Cannot start a runtime from within a runtime`，真机首跑实测踩到）。
async fn real_pool() -> Option<DmPool<OdbcBackend>> {
    let Some(mut cfg) = base_config() else {
        eprintln!("跳过：未配置 DM_TEST_DSN / DM_SERVER（真机用例只在真库上跑，见文件头注释）");
        return None;
    };
    cfg.pool = PoolConfig {
        max_size: 2,
        min_idle: 0,
        ..PoolConfig::default()
    };
    let pool = DmPool::<OdbcBackend>::connect_with(cfg, ExecutorConfig::for_pool(2))
        .await
        .expect("真机上建池失败");
    Some(pool)
}

/// 极小的 `block_on`：建池要在 **tokio 运行时上下文**里做（`DmPool` 的硬约束），
/// 而 `Migrator` 的公开面是 `async`，用例最外层需要它。
///
/// 不引 `futures`/`futures-lite`（本项目「能不加依赖就不加」），也不在测试里手写
/// 执行器：`tokio` 已在 `[dev-dependencies]`，直接用它的当前线程运行时。
///
/// ⚠️ 只能在**最外层**调一次：里面再嵌一层 `block_on`（哪怕建的是新运行时）会被
/// tokio 直接拒绝。
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("建 tokio 当前线程运行时失败")
        .block_on(fut)
}

/// 借一条连接。
async fn conn_of(pool: &DmPool<OdbcBackend>) -> PooledConn<OdbcBackend> {
    PooledConn::acquire(pool).await.expect("从池里借连接失败")
}

/// 清掉版本记录表（真机套件跑之前/之后都要清）。表不存在时也当成功。
async fn drop_history(pool: &DmPool<OdbcBackend>) {
    let mut conn = conn_of(pool).await;
    let _ = conn
        .execute(&format!("drop table if exists {HISTORY_TABLE}"), ())
        .await;
}

/// 一个用完就删的临时目录（唯一名：进程号 + 用例名，避免并行/残留互撞）。
struct TempDir(PathBuf);

impl TempDir {
    /// 建 `<可写根目录>/dm-migrate-{PID}-{tag}`。
    ///
    /// **实测**：这台机器上 `std::env::temp_dir()`（`%TEMP%`）对测试进程**不可写**
    /// （`create_dir_all` 报 `Os { code: 5, kind: PermissionDenied }`），所以按顺序试
    /// 候选根目录，谁先成功用谁：cargo 指派的 `CARGO_TARGET_TMPDIR`、工作区
    /// `target\tmp`、最后才退回 `%TEMP%`。
    fn new(tag: &str) -> Self {
        let name = format!("dm-migrate-{}-{tag}", std::process::id());
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(dir) = option_env!("CARGO_TARGET_TMPDIR") {
            candidates.push(PathBuf::from(dir));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("target").join("tmp"));
        }
        candidates.push(std::env::temp_dir());

        let dir = candidates
            .into_iter()
            .map(|base| base.join(&name))
            .find(|dir| {
                // 上一次跑崩了可能留下目录：先删干净再建，保证「首次应用 N 个」的 N 可预期。
                let _ = std::fs::remove_dir_all(dir);
                std::fs::create_dir_all(dir).is_ok()
            })
            .expect("所有候选根目录都建不出临时目录");
        Self(dir)
    }

    /// 目录路径。
    fn path(&self) -> &Path {
        &self.0
    }

    /// 往里写一个脚本文件。
    fn write(&self, file_name: &str, content: &str) {
        std::fs::write(self.0.join(file_name), content).expect("写脚本文件失败");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 一次真机用例的全部上下文：池、`Migrator`、用于清理的裸池句柄。
struct Real {
    /// 与 `migrator` 共享同一个池（`Arc`）。
    pool: Arc<DmPool<OdbcBackend>>,
    /// 待测的迁移入口。
    migrator: Migrator,
}

/// 建好池 + 清表 + 建 `Migrator`；没配环境就返回 `None`（用例**正常返回**）。
async fn setup() -> Option<Real> {
    let pool = real_pool().await?;
    drop_history(&pool).await;
    let pool = Arc::new(pool);
    let migrator = Migrator::new(Arc::clone(&pool));
    Some(Real { pool, migrator })
}

impl Real {
    /// 清理本组自己建的表（`drop if exists`，不存在也不报错）。
    async fn cleanup_tables(&self, tables: &[&str]) {
        let mut conn = conn_of(&self.pool).await;
        for table in tables {
            let _ = conn
                .execute(&format!("drop table if exists {table}"), ())
                .await;
        }
    }

    /// 用例收尾：清脚本建的表 + 清版本记录表。
    async fn finish(&self, tables: &[&str]) {
        self.cleanup_tables(tables).await;
        drop_history(&self.pool).await;
    }
}

// ---------------------------------------------------------------------------
// 用例
// ---------------------------------------------------------------------------

/// T7.1 / CAP-G-02：`ensure_table` 幂等（连调两次都成功），且 `applied()` 起初为空。
#[test]
#[ignore = "真机用例：需要 DM_TEST_DSN，用 --include-ignored 跑"]
fn ensure_table_is_idempotent() {
    block_on(async {
        let Some(real) = setup().await else { return };
        real.migrator
            .ensure_table()
            .await
            .expect("第一次 ensure_table 失败");
        real.migrator
            .ensure_table()
            .await
            .expect("第二次 ensure_table（幂等）失败");
        let applied = real.migrator.applied().await.expect("读已应用列表失败");
        assert!(applied.is_empty(), "清过表之后不该有记录：{applied:?}");
        real.finish(&[]).await;
    });
}

/// T7.1 / CAP-G-03：`apply_pending` 首次应用 N 个脚本、第二次应用 0 个、记录齐全。
#[test]
#[ignore = "真机用例：需要 DM_TEST_DSN，用 --include-ignored 跑"]
fn apply_pending_then_second_call_is_noop() {
    block_on(async {
        let Some(real) = setup().await else { return };
        let dir = TempDir::new("apply");
        // 版本号刻意用 9 与 10：字典序会把 V10 排在 V9 前面，数值序必须反过来。
        dir.write(
            "V9__second_by_name_first_by_number.sql",
            "create table DM_MIG_REAL_A (ID INT PRIMARY KEY);\n",
        );
        dir.write(
            "V10__later_number.sql",
            "create table DM_MIG_REAL_B (ID INT PRIMARY KEY);\n",
        );
        // 干扰项：不是 V…__….sql 形态，必须被静默忽略。
        dir.write("README.md", "# 不是迁移脚本\n");
        dir.write("U9__rollback.sql", "drop table DM_MIG_REAL_A;\n");

        let first = real
            .migrator
            .apply_pending(dir.path())
            .await
            .expect("首次 apply_pending 失败");
        assert_eq!(first, 2, "首次应当应用 2 个脚本（README/U 开头的不算）");

        let applied = real.migrator.applied().await.expect("读已应用列表失败");
        let versions: Vec<&str> = applied.iter().map(|a| a.version.as_str()).collect();
        assert_eq!(
            versions,
            vec!["9", "10"],
            "应用顺序必须是数值序：{applied:?}"
        );
        assert!(
            applied.iter().all(|a| a.checksum.len() == 16),
            "校验和应为 16 位十六进制：{applied:?}"
        );
        assert!(
            applied.iter().all(|a| !a.applied_at.is_empty()),
            "应用时间不应为空：{applied:?}"
        );

        let second = real
            .migrator
            .apply_pending(dir.path())
            .await
            .expect("第二次 apply_pending 失败");
        assert_eq!(second, 0, "已记录的全部跳过");

        // 两张表真的建出来了（能查到 0 行就说明表存在）。
        let mut conn = conn_of(&real.pool).await;
        let row = conn
            .query_one("select count(*) from DM_MIG_REAL_A", ())
            .await
            .expect("查 DM_MIG_REAL_A 失败（表应当存在）")
            .expect("count(*) 必有一行");
        assert_eq!(row.get::<i64>(0).expect("count(*) 应当是整数"), 0);
        drop(conn);

        real.finish(&["DM_MIG_REAL_A", "DM_MIG_REAL_B"]).await;
    });
}

/// T7.1（失败即停）：第 2 个脚本语法错时，第 1 个已提交、第 2 个回滚、第 3 个不执行。
#[test]
#[ignore = "真机用例：需要 DM_TEST_DSN，用 --include-ignored 跑"]
fn apply_pending_stops_at_first_failure() {
    block_on(async {
        let Some(real) = setup().await else { return };
        let dir = TempDir::new("fail");
        dir.write(
            "V1__ok.sql",
            "create table DM_MIG_REAL_OK (ID INT PRIMARY KEY);\n",
        );
        // 语法错：达梦肯定报错。
        dir.write("V2__bad.sql", "this is not sql at all;\n");
        // 第 3 个脚本一旦被执行，下面这句 `create table` 就会让断言失败。
        dir.write(
            "V3__must_not_run.sql",
            "create table DM_MIG_REAL_Z (ID INT PRIMARY KEY);\n",
        );

        let err = real
            .migrator
            .apply_pending(dir.path())
            .await
            .expect_err("坏脚本必须报错（失败即停）");
        eprintln!("坏脚本报错（预期）：{err:?}");
        let detail = format!("{err:?}");
        assert!(
            detail.contains("V2__bad.sql") || err.to_string().contains("V2__bad.sql"),
            "错误信息里应当指出是哪个脚本：{detail}"
        );

        // 已成功的不回滚：V1 的表存在，V3 的表不存在。
        let mut conn = conn_of(&real.pool).await;
        for (table, should_exist) in [("DM_MIG_REAL_OK", true), ("DM_MIG_REAL_Z", false)] {
            let probe = conn
                .query(&format!("select 1 from {table} where 1 = 0"), ())
                .await;
            assert_eq!(
                probe.is_ok(),
                should_exist,
                "{table} 存在性不符合「失败即停」语义：{probe:?}"
            );
        }
        drop(conn);

        // 记录表里只有 V1（V2 的记录行与它的语句同事务，一起回滚了）。
        let applied = real.migrator.applied().await.expect("读已应用列表失败");
        let versions: Vec<&str> = applied.iter().map(|a| a.version.as_str()).collect();
        assert_eq!(versions, vec!["1"], "只应记下第一个脚本：{applied:?}");

        real.finish(&["DM_MIG_REAL_OK", "DM_MIG_REAL_Z"]).await;
    });
}

/// T7.1（CAP-G-01）：`run_sql_file` 在真库上跑一个含字符串分号 + 注释的脚本。
#[test]
#[ignore = "真机用例：需要 DM_TEST_DSN，用 --include-ignored 跑"]
fn run_sql_file_handles_comments_and_literal_semicolons() {
    block_on(async {
        let Some(real) = setup().await else { return };
        let dir = TempDir::new("runsql");
        dir.write(
            "V1__literal.sql",
            "-- 注释里的分号 ; 不能被当成语句结束\n\
             create table DM_MIG_REAL_LIT (ID INT PRIMARY KEY, V VARCHAR(50));\n\
             /* 块注释里的分号 ; 也一样 */\n\
             insert into DM_MIG_REAL_LIT (ID, V) values (1, 'a;b');\n\
             insert into DM_MIG_REAL_LIT (ID, V) values (2, 'it''s; fine');\n",
        );

        real.migrator
            .ensure_table()
            .await
            .expect("ensure_table 失败");
        real.migrator
            .run_sql_file(&dir.path().join("V1__literal.sql"))
            .await
            .expect("run_sql_file 失败");

        let mut conn = conn_of(&real.pool).await;
        let row = conn
            .query_one("select count(*) from DM_MIG_REAL_LIT", ())
            .await
            .expect("查 DM_MIG_REAL_LIT 失败")
            .expect("count(*) 必有一行");
        assert_eq!(
            row.get::<i64>(0).expect("count(*) 应当是整数"),
            2,
            "两条 INSERT 都该执行（分号切分不能被注释/字面量骗到）"
        );
        let row = conn
            .query_one("select V from DM_MIG_REAL_LIT where ID = 1", ())
            .await
            .expect("查字面量失败")
            .expect("必有行");
        assert_eq!(
            row.get::<String>(0).expect("V 应当是字符串"),
            "a;b",
            "字符串字面量内的分号必须原样保留"
        );
        drop(conn);

        real.finish(&["DM_MIG_REAL_LIT"]).await;
    });
}
