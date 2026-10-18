//! 临时探针（T8.9）：把官方一致性套件里剩下的**每类失败各取一个最小形状**，
//! 逐个打到真机上，**不猜**。
//!
//! 覆盖的问题（按套件失败数排序）：
//! 1. `SELECT <布尔表达式> AS columnN` —— `field_version` 等簇（`-2007`，列 44 `AS`）；
//! 2. 标识符大小写 —— `raw_sql` 簇（用户 SQL 不写引号 ⇒ 折叠成大写，找不到我们的
//!    `"lowercase"` 表，`-2106`）；
//! 3. `VALUES` 派生表的替代写法 —— `embed_relation*` 簇；
//! 4. `NUMBER` / `DECIMAL(38,10)` 经 ODBC 回来的 `DmValue` 变体（`Double` ⇒ Decimal 还原失败）；
//! 5. `RELEASE SAVEPOINT` 是否真的不被达梦支持（套件 `tx_interactive::nested_*`）；
//! 6. `SET TRANSACTION READ ONLY`（套件 `read_only = true` 的两条）；
//! 7. `CLOB` 能不能比较/建索引（`db::Type::Text` ⇒ `VARCHAR(8188)` 的依据）；
//! 8. `VARCHAR(8188)` 与枚举的 `CHECK` 能否共存。
//! 9. TIME 的小数秒到底丢在**绑定侧**还是**取值侧**（套件 `type_jiff::ty_time`、
//!    `ty_time_precision_2`、`raw_sql::query_column_types_decode_temporal` 三条）。
//! 10. `ORDER BY` 的 NULL 默认落点、以及 `NULLS FIRST`/`NULLS LAST` 是否被接受
//!     （套件 `paginate_multi_column::paginate_nullable_unique_column_uses_primary_key`）。
//! 11. upsert ignore 的「空更新」能不能落在 `ON` 关联列上（Round 18，第 16 节）：
//!     达梦报 `[HY000/-4064] 不能更新关联条件中的列`，占位符必须挑非冲突目标列；
//!     只有键列的表则连非键列都没有，要看「不写 MATCHED 子句」的 MERGE 收不收。
//! 12. `Vec<scalar>` 的存储与判定手段（Round 20，第 17 节）：JSON 函数族能不能用、
//!     CLOB 上的 `LIKE` 折不折大小写、有没有原生数组 / `MEMBER OF` / `SUBMULTISET`。
//! 13. `Vec<scalar>` 的**原地删除**（Round 21，第 18 节）：`JSON_REMOVE` 一族在不在、
//!     `JSON_SEARCH('all')` 多命中怎么回、`JSON_TABLE + JSON_ARRAYAGG` 重建能不能
//!     放进 `UPDATE` 的赋值里（套件 `vec_string_pop_cases`/`vec_string_remove_value_cases`/
//!     `vec_string_remove_at_cases` 三条）。
//! 14. `Vec<scalar>` 的 `remove(v)`（「删掉**每个**匹配元素」）在 `UPDATE` 里到底能不能做
//!     （Round 21/22，第 19/20 节）：第 19 节三个形状全灭 —— A（`JSON_TABLE` 吃字面量 +
//!     `LISTAGG` 重建）在 SET 子查询里**静默写出 `[]`**（`影响 1 行` 却把整列清空）、
//!     B（非相关标量子查询）`-2205` 语法错误、C（MERGE + 派生表里的 `JSON_TABLE`）
//!     `-4083 引用列未找到`；第 20 节改用 `JSON_QUERY` 的**过滤路径**
//!     （`'$[*]?(@ != "dup")'`）与「把 `JSON_TABLE` 挪回普通 SELECT 上下文」的
//!     UPDATE（带表别名）/ MERGE（USING 派生表 + GROUP BY）两种形状。
//! 15. `capability.rs` 里最后 7 处 `(待实测)` 的定界（Round 24，第 23 节）：升序 NULL 的默认落点与
//!     `NULLS FIRST/LAST` 的接受度（`sql_nulls_first_on_asc`）、128 字节标识符上限、
//!     `VARCHAR(255)` 能不能作主键、`VARCHAR(8188)` 的索引与 `8189` 的拒绝、
//!     `IDENTITY` 在 TINYINT/SMALLINT 上收不收、`ALTER TABLE … MODIFY` 的形态与多属性原子性、
//!     `WITH … UPDATE` 在不在。
//! 16. `bigdecimal_implemented` 能不能翻真（Round 25，第 24 节）：套件的 `numeric(38,20)` 用例要 38 位
//!     有效数字，而 `DmValue::Decimal` 的 `rust_decimal` 只装得下约 28 位 ⇒ 探「文本参数能不能存进
//!     `DECIMAL(38,20)`」「回读是什么变体、精不精确」「服务端 `CAST(v AS VARCHAR)` 的尾零/标度」。
//! 17. `#[document]` 的路径提取（Round 26，第 25 节）：`JSON_VALUE` / `JSON_QUERY` /
//!     `JSON_EXTRACT` 在 **CLOB 文档**上在不在、叶子形态（带不带引号）、缺失路径/对象叶子的
//!     错误语义、以及「提取出来的叶子能不能直接与绑定参数比较」（决定 fork
//!     `serializer/expr.rs` 的 `serialize_json_extract` 达梦臂——fork 里最后一个
//!     `unimplemented!`，也是 `document_collections` 的前置）。
//! 18. upsert 最后两项能力位（Round 28，第 31 节）：冲突目标是**唯一键 / 复合唯一键**时
//!     `MERGE … ON (…)` 命中已有行是否保留**已有行的主键**（决定驱动 `insert_and_reload`
//!     的回查键是主键还是冲突目标列），以及 `WHEN MATCHED` 分支里「本次新值」（引擎
//!     `upsert.update(...)` 的 `incoming()`）渲染成 `USING` 源列 `s."s_N"` 达梦收不收。
//!
//! 跑法：
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --test engine_probe \
//!     -- --include-ignored --test-threads=1 --nocapture
//! ```

use dm_core::{DmConfig, DmValue};
use dm_query::PooledConn;

async fn conn() -> (dm_pool::DmPool, PooledConn) {
    let dsn = std::env::var("DM_TEST_DSN").expect("需要 DM_TEST_DSN");
    let cfg = DmConfig::from_connection_string(&dsn).expect("DSN 解析失败");
    let pool: dm_pool::DmPool = dm_pool::DmPool::connect(cfg).await.expect("连不上达梦");
    let conn = PooledConn::acquire(&pool).await.expect("借连接失败");
    (pool, conn)
}

async fn probe(conn: &mut PooledConn, label: &str, sql: &str, args: impl dm_query::IntoArgs) {
    match conn.query(sql, args).await {
        Ok(rs) => {
            let first = rs
                .first()
                .map(|row| format!("{:?}", row.values()))
                .unwrap_or_else(|| "<无行>".to_string());
            println!("OK   {label:<44} {first}");
        }
        Err(e) => println!("FAIL {label:<44} {e}"),
    }
}

async fn ddl(conn: &mut PooledConn, label: &str, sql: &str) {
    match conn.execute(sql, ()).await {
        Ok(n) => println!("OK   {label:<44} 影响 {n} 行"),
        Err(e) => println!("FAIL {label:<44} {e}"),
    }
}

#[tokio::test]
#[ignore = "真机探针：需要 DM_TEST_DSN"]
async fn probe_engine_shapes() {
    let (pool, mut conn) = conn().await;

    println!("\n=== 0. 准备表 ===");
    let _ = conn
        .execute("DROP TABLE \"dm_engine_probe\" CASCADE CONSTRAINTS", ())
        .await;
    let _ = conn
        .execute("DROP TABLE dm_engine_probe2 CASCADE CONSTRAINTS", ())
        .await;
    ddl(
        &mut conn,
        "0a. CREATE TABLE \"dm_engine_probe\"（带引号小写）",
        "CREATE TABLE \"dm_engine_probe\" (\"id\" INT, \"val\" VARCHAR(20), \"flag\" TINYINT, \"n\" NUMBER)",
    )
    .await;
    ddl(
        &mut conn,
        "0b. CREATE TABLE dm_engine_probe2（不带引号）",
        "CREATE TABLE dm_engine_probe2 (id INT, val VARCHAR(20))",
    )
    .await;
    ddl(
        &mut conn,
        "0c. INSERT 1.5 / TRUE",
        "INSERT INTO \"dm_engine_probe\" (\"id\", \"val\", \"flag\", \"n\") VALUES (1, 'x', 1, 1.5)",
    )
    .await;

    println!("\n=== 1. 标识符大小写（raw_sql 簇）===");
    probe(
        &mut conn,
        "1a. 不写引号的小写表名（用户 SQL 的形状）",
        "SELECT val FROM dm_engine_probe",
        (),
    )
    .await;
    probe(
        &mut conn,
        "1b. 写成大写",
        "SELECT val FROM DM_ENGINE_PROBE",
        (),
    )
    .await;
    probe(
        &mut conn,
        "1c. 带引号小写（我们生成的形状）",
        "SELECT \"val\" FROM \"dm_engine_probe\"",
        (),
    )
    .await;
    probe(
        &mut conn,
        "1d. 不带引号建的表 + 不写引号查",
        "SELECT val FROM dm_engine_probe2",
        (),
    )
    .await;
    probe(
        &mut conn,
        "1e. 不带引号建的表 + 带引号小写查",
        "SELECT \"val\" FROM \"dm_engine_probe2\"",
        (),
    )
    .await;
    probe(
        &mut conn,
        "1f. 目录里的真实名字",
        "SELECT TABLE_NAME FROM USER_TABLES WHERE TABLE_NAME LIKE 'DM_ENGINE%'",
        (),
    )
    .await;

    println!("\n=== 2. 布尔表达式当投影列（field_version 簇，-2007 列 44 [AS]）===");
    probe(
        &mut conn,
        "2a. SELECT 1 = 1 AS c",
        "SELECT 1 = 1 AS c FROM DUAL",
        (),
    )
    .await;
    probe(
        &mut conn,
        "2b. SELECT ? = 1 AS c（布尔参数）",
        "SELECT ? = 1 AS c FROM DUAL",
        vec![DmValue::Bool(true)],
    )
    .await;
    probe(
        &mut conn,
        "2c. SELECT \"flag\" = ? AS c（真列，TINYINT 参数）",
        "SELECT \"flag\" = ? AS c FROM \"dm_engine_probe\"",
        vec![DmValue::TinyInt(1)],
    )
    .await;
    probe(
        &mut conn,
        "2d. CASE WHEN 1 = 1 THEN 1 ELSE 0 END",
        "SELECT CASE WHEN 1 = 1 THEN 1 ELSE 0 END AS c FROM DUAL",
        (),
    )
    .await;
    probe(
        &mut conn,
        "2e. CASE WHEN ? = 1 THEN 1 ELSE 0 END",
        "SELECT CASE WHEN ? = 1 THEN 1 ELSE 0 END AS c FROM DUAL",
        vec![DmValue::Bool(true)],
    )
    .await;
    probe(
        &mut conn,
        "2f. CASE WHEN \"flag\" = ? THEN 1 ELSE 0 END",
        "SELECT CASE WHEN \"flag\" = ? THEN 1 ELSE 0 END AS c FROM \"dm_engine_probe\"",
        vec![DmValue::TinyInt(1)],
    )
    .await;
    probe(
        &mut conn,
        "2g. 直接选布尔列",
        "SELECT \"flag\" AS c FROM \"dm_engine_probe\"",
        (),
    )
    .await;

    println!("\n=== 3. VALUES 派生表的替代（embed_relation 簇）===");
    probe(
        &mut conn,
        "3a. PG/MySQL 形状（应当失败）",
        "SELECT t.column1 FROM (VALUES (?)) AS t(column1)",
        vec![DmValue::Int(1)],
    )
    .await;
    probe(
        &mut conn,
        "3b. 单行 SELECT ... FROM DUAL",
        "SELECT t.column1 FROM (SELECT ? AS column1 FROM DUAL) AS t",
        vec![DmValue::Int(1)],
    )
    .await;
    probe(
        &mut conn,
        "3c. 两行 UNION ALL",
        "SELECT t.column1 FROM (SELECT ? AS column1 FROM DUAL UNION ALL SELECT ? FROM DUAL) AS t",
        vec![DmValue::Int(1), DmValue::Int(2)],
    )
    .await;
    probe(
        &mut conn,
        "3d. 与真表关联（引擎的实际形状）",
        "SELECT p.\"id\" FROM \"dm_engine_probe\" AS p WHERE EXISTS (SELECT ? AS column1 FROM DUAL WHERE ? = p.\"id\")",
        vec![DmValue::Int(1), DmValue::Int(1)],
    )
    .await;

    println!("\n=== 4. NUMBER / DECIMAL 的 ODBC 返回类型（type_decimal 簇）===");
    probe(
        &mut conn,
        "4a. SELECT \"n\"（NUMBER 列）",
        "SELECT \"n\" FROM \"dm_engine_probe\"",
        (),
    )
    .await;
    let _ = conn
        .execute("DROP TABLE \"dm_dec_probe\" CASCADE CONSTRAINTS", ())
        .await;
    ddl(
        &mut conn,
        "4b. CREATE TABLE 带 DECIMAL(38,10)",
        "CREATE TABLE \"dm_dec_probe\" (\"d\" DECIMAL(38,10), \"n\" NUMBER)",
    )
    .await;
    let _ = conn
        .execute(
            "INSERT INTO \"dm_dec_probe\" (\"d\", \"n\") VALUES (2.25, 2.25)",
            (),
        )
        .await;
    probe(
        &mut conn,
        "4c. SELECT DECIMAL(38,10) 列",
        "SELECT \"d\" FROM \"dm_dec_probe\"",
        (),
    )
    .await;
    probe(
        &mut conn,
        "4d. SELECT NUMBER 列",
        "SELECT \"n\" FROM \"dm_dec_probe\"",
        (),
    )
    .await;
    probe(
        &mut conn,
        "4e. SELECT CAST(1.5 AS NUMBER)",
        "SELECT CAST(1.5 AS NUMBER) AS c FROM DUAL",
        (),
    )
    .await;
    probe(
        &mut conn,
        "4f. SELECT @@IDENTITY 形状",
        "SELECT 1.5 AS c FROM DUAL",
        (),
    )
    .await;

    println!("\n=== 5. RELEASE SAVEPOINT（tx_interactive::nested_*）===");
    {
        // `PooledConn::begin` 按值拿走连接，所以每条事务用例都要从池里另借一条。
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        match txn.savepoint("sp_probe").await {
            Ok(()) => println!("OK   {:<44} SAVEPOINT 通过", "5a. SAVEPOINT"),
            Err(e) => println!("FAIL {:<44} {e}", "5a. SAVEPOINT"),
        }
        match txn.release_savepoint("sp_probe").await {
            Ok(()) => println!(
                "OK   {:<44} RELEASE SAVEPOINT 通过",
                "5b. RELEASE SAVEPOINT"
            ),
            Err(e) => println!("FAIL {:<44} {e}", "5b. RELEASE SAVEPOINT"),
        }
        match txn.rollback_to("sp_probe").await {
            Ok(()) => println!(
                "OK   {:<44} 已释放后仍可 ROLLBACK TO",
                "5c. ROLLBACK TO（已释放）"
            ),
            Err(e) => println!("FAIL {:<44} {e}", "5c. ROLLBACK TO（已释放）"),
        }
        match txn.commit().await {
            Ok(()) => println!("OK   {:<44} COMMIT 通过", "5d. COMMIT"),
            Err(e) => println!("FAIL {:<44} {e}", "5d. COMMIT"),
        }
    }

    println!("\n=== 6. SET TRANSACTION READ ONLY ===");
    {
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        match txn.execute("SET TRANSACTION READ ONLY", ()).await {
            Ok(n) => println!("OK   {:<44} 影响 {n} 行", "6a. SET TRANSACTION READ ONLY"),
            Err(e) => println!("FAIL {:<44} {e}", "6a. SET TRANSACTION READ ONLY"),
        }
        match txn
            .execute(
                "UPDATE \"dm_engine_probe\" SET \"val\" = 'y' WHERE \"id\" = 1",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<44} 影响 {n} 行（只读事务竟然允许写？）",
                "6b. 只读事务里 UPDATE"
            ),
            Err(e) => println!("FAIL {:<44} {e}", "6b. 只读事务里 UPDATE"),
        }
        let _ = txn.rollback().await;
    }

    println!("\n=== 7. CLOB 能否比较/建索引 ===");
    let _ = conn
        .execute("DROP TABLE \"dm_clob_probe\" CASCADE CONSTRAINTS", ())
        .await;
    ddl(
        &mut conn,
        "7a. CREATE TABLE 带 CLOB",
        "CREATE TABLE \"dm_clob_probe\" (\"id\" INT, \"t\" CLOB)",
    )
    .await;
    let _ = conn
        .execute(
            "INSERT INTO \"dm_clob_probe\" (\"id\", \"t\") VALUES (1, 'x')",
            (),
        )
        .await;
    probe(
        &mut conn,
        "7b. WHERE CLOB = 'x'",
        "SELECT \"id\" FROM \"dm_clob_probe\" WHERE \"t\" = 'x'",
        (),
    )
    .await;
    ddl(
        &mut conn,
        "7c. 给 CLOB 建索引",
        "CREATE INDEX \"dm_clob_idx\" ON \"dm_clob_probe\" (\"t\")",
    )
    .await;

    println!("\n=== 8. VARCHAR(8188) + 枚举 CHECK ===");
    let _ = conn
        .execute("DROP TABLE \"dm_enum_probe\" CASCADE CONSTRAINTS", ())
        .await;
    ddl(
        &mut conn,
        "8a. VARCHAR(8188) + CHECK",
        "CREATE TABLE \"dm_enum_probe\" (\"id\" INT, \"e\" VARCHAR(8188) NOT NULL CHECK (\"e\" IN ('a', 'b')))",
    )
    .await;
    let _ = conn
        .execute(
            "INSERT INTO \"dm_enum_probe\" (\"id\", \"e\") VALUES (1, 'a')",
            (),
        )
        .await;
    ddl(
        &mut conn,
        "8b. 给枚举列建索引",
        "CREATE INDEX \"dm_enum_idx\" ON \"dm_enum_probe\" (\"e\")",
    )
    .await;
    ddl(
        &mut conn,
        "8c. 插非法枚值（应被 CHECK 拒）",
        "INSERT INTO \"dm_enum_probe\" (\"id\", \"e\") VALUES (2, 'z')",
    )
    .await;

    println!("\n=== 9. 布尔列该用什么类型（-6119 无效的数据类型）===");
    for (idx, (label, ty, literal)) in [
        ("9a. BOOLEAN", "BOOLEAN", "TRUE"),
        ("9b. BIT", "BIT", "1"),
        ("9c. TINYINT", "TINYINT", "1"),
        ("9d. CHAR(1)", "CHAR(1)", "'1'"),
        ("9e. SMALLINT", "SMALLINT", "1"),
        ("9f. INT", "INT", "1"),
    ]
    .into_iter()
    .enumerate()
    {
        let table = format!("dm_bool_probe_{idx}");
        let _ = conn
            .execute(&format!("DROP TABLE \"{table}\" CASCADE CONSTRAINTS"), ())
            .await;
        ddl(
            &mut conn,
            label,
            &format!("CREATE TABLE \"{table}\" (\"id\" INT, \"b\" {ty})"),
        )
        .await;
        let _ = conn
            .execute(
                &format!("INSERT INTO \"{table}\" (\"id\", \"b\") VALUES (1, {literal})"),
                (),
            )
            .await;
        probe(
            &mut conn,
            &format!("{label} 读回"),
            &format!("SELECT \"b\" FROM \"{table}\""),
            (),
        )
        .await;
        // 布尔参数能不能直接塞进这种列？
        let bind = conn
            .execute(
                &format!("INSERT INTO \"{table}\" (\"id\", \"b\") VALUES (?, ?)"),
                vec![DmValue::Int(2), DmValue::Bool(false)],
            )
            .await;
        match bind {
            Ok(n) => println!("OK   {label} 绑定 Bool(false) 影响 {n} 行"),
            Err(e) => println!("FAIL {label} 绑定 Bool(false) {e}"),
        }
        let _ = conn
            .execute(&format!("DROP TABLE \"{table}\" CASCADE CONSTRAINTS"), ())
            .await;
    }

    println!("\n=== 10. 保存点语法的原始形态 ===");
    {
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        for (label, sql) in [
            ("10a. SAVEPOINT sp_raw", "SAVEPOINT sp_raw"),
            ("10b. ROLLBACK TO sp_raw", "ROLLBACK TO sp_raw"),
            (
                "10c. ROLLBACK TO SAVEPOINT sp_raw",
                "ROLLBACK TO SAVEPOINT sp_raw",
            ),
            ("10d. RELEASE SAVEPOINT sp_raw", "RELEASE SAVEPOINT sp_raw"),
        ] {
            match txn.execute(sql, ()).await {
                Ok(n) => println!("OK   {label:<44} 影响 {n} 行"),
                Err(e) => println!("FAIL {label:<44} {e}"),
            }
        }
        let _ = txn.rollback().await;
    }

    println!("\n=== 11. 只读事务是否真的拦住写 ===");
    {
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        let _ = txn.execute("SET TRANSACTION READ ONLY", ()).await;
        match txn
            .execute("SELECT \"id\" FROM \"dm_engine_probe2\"", ())
            .await
        {
            Ok(n) => println!("OK   {:<44} 影响 {n} 行", "11a. 只读事务里 SELECT"),
            Err(e) => println!("FAIL {:<44} {e}", "11a. 只读事务里 SELECT"),
        }
        match txn
            .execute(
                "UPDATE \"dm_engine_probe\" SET \"val\" = 'y' WHERE \"id\" = 1",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<44} 影响 {n} 行（只读竟然允许写！）",
                "11b. 只读事务里 UPDATE"
            ),
            Err(e) => println!("FAIL {:<44} {e}", "11b. 只读事务里 UPDATE"),
        }
        let _ = txn.rollback().await;
    }

    println!("\n=== 12. 隔离级别 + 只读：能不能合成一条 / 能不能发两条 ===");
    {
        // 12a：一条语句同时给隔离级别与只读
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        match txn
            .execute(
                "SET TRANSACTION ISOLATION LEVEL READ COMMITTED READ ONLY",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "12a. 一条：ISOLATION LEVEL … READ ONLY"
            ),
            Err(e) => println!("FAIL {:<52} {e}", "12a. 一条：ISOLATION LEVEL … READ ONLY"),
        }
        match txn.execute("SELECT 1 FROM DUAL", ()).await {
            Ok(_) => println!("OK   {:<52} 只读事务里 SELECT", "12a2. 后续 SELECT"),
            Err(e) => println!("FAIL {:<52} {e}", "12a2. 后续 SELECT"),
        }
        let _ = txn.rollback().await;

        // 12b：先隔离级别、再只读（两条 SET TRANSACTION）
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        match txn
            .execute("SET TRANSACTION ISOLATION LEVEL READ COMMITTED", ())
            .await
        {
            Ok(_) => println!("OK   {:<52} 12b1. 先 ISOLATION LEVEL", "12b1"),
            Err(e) => println!("FAIL {:<52} {e}", "12b1. 先 ISOLATION LEVEL"),
        }
        match txn.execute("SET TRANSACTION READ ONLY", ()).await {
            Ok(_) => println!("OK   {:<52} 12b2. 再 READ ONLY", "12b2"),
            Err(e) => println!("FAIL {:<52} {e}", "12b2. 再 READ ONLY"),
        }
        let _ = txn.rollback().await;

        // 12c：先只读、再隔离级别
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        match txn.execute("SET TRANSACTION READ ONLY", ()).await {
            Ok(_) => println!("OK   {:<52} 12c1. 先 READ ONLY", "12c1"),
            Err(e) => println!("FAIL {:<52} {e}", "12c1. 先 READ ONLY"),
        }
        match txn
            .execute("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE", ())
            .await
        {
            Ok(_) => println!("OK   {:<52} 12c2. 再 ISOLATION LEVEL", "12c2"),
            Err(e) => println!("FAIL {:<52} {e}", "12c2. 再 ISOLATION LEVEL"),
        }
        let _ = txn.rollback().await;

        // 12d：只读 + 写，确认被拒（错误码用于映射到 read_only_transaction）
        let mut txn = PooledConn::acquire(&pool)
            .await
            .expect("借连接失败")
            .begin()
            .await
            .expect("begin 失败");
        let _ = txn.execute("SET TRANSACTION READ ONLY", ()).await;
        match txn
            .execute("CREATE TABLE \"dm_ro_probe\" (\"id\" INT)", ())
            .await
        {
            Ok(_) => println!(
                "OK   {:<52} 12d. 只读事务里 CREATE TABLE（竟然允许）",
                "12d"
            ),
            Err(e) => println!("FAIL {:<52} {e}", "12d. 只读事务里 CREATE TABLE"),
        }
        let _ = txn.rollback().await;
    }

    println!("\n=== 13. TIME 小数秒丢在哪一侧：绑定 vs 取值 ===");
    {
        let _ = conn
            .execute("DROP TABLE \"dm_time_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "13a. 建 TIME(6) 表",
            "CREATE TABLE \"dm_time_probe\" (\"id\" INT, \"v\" TIME(6))",
        )
        .await;

        // 13b：服务端字面量，完全绕过我们的绑定
        ddl(
            &mut conn,
            "13b. 字面量插入 TIME '14:30:45.123456'",
            "INSERT INTO \"dm_time_probe\" VALUES (1, TIME '14:30:45.123456')",
        )
        .await;
        // 13c：按文本取回来 ⇒ 服务端到底存住微秒没有
        probe(
            &mut conn,
            "13c. CAST(v AS VARCHAR) 第 1 行",
            "SELECT CAST(\"v\" AS VARCHAR(30)) FROM \"dm_time_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        // 13d：同一个值走我们的取值路径（当前是 SQL_C_TYPE_TIME）
        probe(
            &mut conn,
            "13d. 走驱动取第 1 行",
            "SELECT \"v\" FROM \"dm_time_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;

        // 13e：同一个值走我们的绑定路径写进去（当前按 SQL_TIMESTAMP_STRUCT 编码）
        match conn
            .execute(
                "INSERT INTO \"dm_time_probe\" VALUES (?, ?)",
                vec![
                    DmValue::Int(2),
                    DmValue::Time(
                        chrono::NaiveTime::from_hms_micro_opt(14, 30, 45, 123_456).unwrap(),
                    ),
                ],
            )
            .await
        {
            Ok(n) => println!("OK   {:<44} 绑定插入 {n} 行", "13e. 绑定插入"),
            Err(e) => println!("FAIL {:<44} {e}", "13e. 绑定插入"),
        }
        // 13f：把绑定写进去的值按文本读回来 ⇒ 绑定有没有丢小数秒
        probe(
            &mut conn,
            "13f. CAST(v AS VARCHAR) 第 2 行",
            "SELECT CAST(\"v\" AS VARCHAR(30)) FROM \"dm_time_probe\" WHERE \"id\" = 2",
            (),
        )
        .await;
        // 13g：再走一次我们的取值路径
        probe(
            &mut conn,
            "13g. 走驱动取第 2 行",
            "SELECT \"v\" FROM \"dm_time_probe\" WHERE \"id\" = 2",
            (),
        )
        .await;
    }

    println!("\n=== 14. ORDER BY 的 NULL 位置：默认 / NULLS FIRST / NULLS LAST ===");
    {
        let _ = conn
            .execute("DROP TABLE \"dm_null_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "14a. 建 NULL 排序探针表",
            "CREATE TABLE \"dm_null_probe\" (\"id\" INT, \"rank\" INT)",
        )
        .await;
        ddl(
            &mut conn,
            "14b. 插 (1,NULL)(2,NULL)(3,10)(4,20)",
            "INSERT INTO \"dm_null_probe\" VALUES (1,NULL),(2,NULL),(3,10),(4,20)",
        )
        .await;

        for (label, sql) in [
            (
                "14c. rank ASC（默认）",
                "SELECT \"id\" FROM \"dm_null_probe\" ORDER BY \"rank\" ASC",
            ),
            (
                "14d. rank ASC NULLS LAST",
                "SELECT \"id\" FROM \"dm_null_probe\" ORDER BY \"rank\" ASC NULLS LAST",
            ),
            (
                "14e. rank ASC NULLS FIRST",
                "SELECT \"id\" FROM \"dm_null_probe\" ORDER BY \"rank\" ASC NULLS FIRST",
            ),
            (
                "14f. rank DESC（默认）",
                "SELECT \"id\" FROM \"dm_null_probe\" ORDER BY \"rank\" DESC",
            ),
            (
                "14g. rank DESC NULLS FIRST",
                "SELECT \"id\" FROM \"dm_null_probe\" ORDER BY \"rank\" DESC NULLS FIRST",
            ),
            (
                "14h. rank DESC NULLS LAST",
                "SELECT \"id\" FROM \"dm_null_probe\" ORDER BY \"rank\" DESC NULLS LAST",
            ),
        ] {
            match conn.query(sql, ()).await {
                Ok(rs) => {
                    let ids: Vec<String> = rs
                        .rows()
                        .iter()
                        .map(|row| format!("{:?}", row.values()))
                        .collect();
                    println!("OK   {label:<40} {}", ids.join(" "));
                }
                Err(e) => println!("FAIL {label:<40} {e}"),
            }
        }
    }

    println!("\n=== 15. upsert：MERGE INTO 形态 / 自增列显式赋值（T8.3 前置事实）===");
    {
        let _ = conn
            .execute("DROP TABLE \"dm_merge_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "15a. 建非自增主键表",
            "CREATE TABLE \"dm_merge_probe\" (\"id\" INT PRIMARY KEY, \"name\" VARCHAR(50))",
        )
        .await;

        // 用字面量先把「语法形态」问清楚，再用参数重放（参数位置就是 fork 要渲染的位置）。
        for (label, sql) in [
            (
                "15b. MERGE + AS 别名 + SET t.col = s.col",
                "MERGE INTO \"dm_merge_probe\" AS t USING (SELECT 1 AS \"id\", 'a' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET t.\"name\" = s.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
            ),
            (
                "15c. MERGE 别名不带 AS",
                "MERGE INTO \"dm_merge_probe\" t USING (SELECT 2 AS \"id\", 'b' AS \"name\" FROM DUAL) s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET t.\"name\" = s.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
            ),
            (
                "15d. SET 左侧不带表别名",
                "MERGE INTO \"dm_merge_probe\" AS t USING (SELECT 3 AS \"id\", 'c' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = s.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
            ),
            (
                "15e. 幂等重放（命中 MATCHED 分支）",
                "MERGE INTO \"dm_merge_probe\" AS t USING (SELECT 1 AS \"id\", 'a2' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = s.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
            ),
            (
                "15f. ignore 形态（空更新占位）",
                "MERGE INTO \"dm_merge_probe\" AS t USING (SELECT 1 AS \"id\", 'x' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = t.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
            ),
            (
                "15g. MySQL 的 ON DUPLICATE KEY UPDATE",
                "INSERT INTO \"dm_merge_probe\" (\"id\", \"name\") VALUES (9, 'z') ON DUPLICATE KEY UPDATE \"name\" = 'z2'",
            ),
        ] {
            match conn.execute(sql, ()).await {
                Ok(n) => println!("OK   {label:<40} 影响 {n} 行"),
                Err(e) => println!("FAIL {label:<40} {e}"),
            }
        }

        // 参数形态：占位符落在 USING 子查询里（`@` 顺序 = 原始 VALUES 顺序）。
        match conn
            .execute(
                "MERGE INTO \"dm_merge_probe\" AS t USING (SELECT ? AS \"id\", ? AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = s.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
                vec![DmValue::Int(20), DmValue::Text("p".into())],
            )
            .await
        {
            Ok(n) => println!("OK   {:<40} 影响 {n} 行", "15h. MERGE + 参数占位符"),
            Err(e) => println!("FAIL {:<40} {e}", "15h. MERGE + 参数占位符"),
        }
        match conn
            .query(
                "SELECT \"id\", \"name\" FROM \"dm_merge_probe\" ORDER BY \"id\"",
                (),
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!("OK   {:<40} {}", "15i. 现状（id/name）", rows.join(" "));
            }
            Err(e) => println!("FAIL {:<40} {e}", "15i. 现状（id/name）"),
        }

        // 自增列：显式值要不要 `SET IDENTITY_INSERT`
        let _ = conn
            .execute("DROP TABLE \"dm_ident_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "15j. 建自增主键表",
            "CREATE TABLE \"dm_ident_probe\" (\"id\" INT IDENTITY(1,1) PRIMARY KEY, \"name\" VARCHAR(50))",
        )
        .await;
        let ident_args = || vec![DmValue::Int(100), DmValue::Text("x".into())];
        match conn
            .execute(
                "INSERT INTO \"dm_ident_probe\" (\"id\", \"name\") VALUES (?, ?)",
                ident_args(),
            )
            .await
        {
            Ok(n) => println!("OK   {:<40} 影响 {n} 行", "15k. 自增列直接给显式值"),
            Err(e) => println!("FAIL {:<40} {e}", "15k. 自增列直接给显式值"),
        }
        match conn
            .execute("SET IDENTITY_INSERT \"dm_ident_probe\" ON", ())
            .await
        {
            Ok(_) => println!("OK   {:<40}", "15l. SET IDENTITY_INSERT ON"),
            Err(e) => println!("FAIL {:<40} {e}", "15l. SET IDENTITY_INSERT ON"),
        }
        match conn
            .execute(
                "INSERT INTO \"dm_ident_probe\" (\"id\", \"name\") VALUES (?, ?)",
                ident_args(),
            )
            .await
        {
            Ok(n) => println!("OK   {:<40} 影响 {n} 行", "15m. IDENTITY_INSERT 下显式插入"),
            Err(e) => println!("FAIL {:<40} {e}", "15m. IDENTITY_INSERT 下显式插入"),
        }
        // MERGE 里显式写自增列（upsert_by_id 的真实形状）
        match conn
            .execute(
                "MERGE INTO \"dm_ident_probe\" AS t USING (SELECT ? AS \"id\", ? AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = s.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
                vec![DmValue::Int(200), DmValue::Text("m".into())],
            )
            .await
        {
            Ok(n) => println!("OK   {:<40} 影响 {n} 行", "15n. IDENTITY_INSERT 下 MERGE 新行"),
            Err(e) => println!("FAIL {:<40} {e}", "15n. IDENTITY_INSERT 下 MERGE 新行"),
        }
        match conn
            .execute(
                "MERGE INTO \"dm_ident_probe\" AS t USING (SELECT ? AS \"id\", ? AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = s.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
                vec![DmValue::Int(200), DmValue::Text("m2".into())],
            )
            .await
        {
            Ok(n) => println!("OK   {:<40} 影响 {n} 行", "15o. IDENTITY_INSERT 下 MERGE 命中"),
            Err(e) => println!("FAIL {:<40} {e}", "15o. IDENTITY_INSERT 下 MERGE 命中"),
        }
        match conn
            .execute("SET IDENTITY_INSERT \"dm_ident_probe\" OFF", ())
            .await
        {
            Ok(_) => println!("OK   {:<40}", "15p. SET IDENTITY_INSERT OFF"),
            Err(e) => println!("FAIL {:<40} {e}", "15p. SET IDENTITY_INSERT OFF"),
        }
        match conn
            .query("SELECT \"id\", \"name\" FROM \"dm_ident_probe\"", ())
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!("OK   {:<40} {}", "15q. 自增表现状", rows.join(" "));
            }
            Err(e) => println!("FAIL {:<40} {e}", "15q. 自增表现状"),
        }
    }

    println!("\n=== 16. upsert ignore：空更新能不能落在「ON 关联列」上（Round 18）===");
    {
        // 起因：放开 `upsert_targeted_ignore` 之后，官方套件
        // `crud_upsert::upsert_explicit_assignments_override_model_defaults` 报
        // `[HY000/-4064] 不能更新关联条件中的列` —— fork 的 ignore 渲染把
        // `insert_table.columns[0]`（往往就是冲突目标列、也就是 `ON` 里的列）当占位符，
        // 而探针 15f 当年用的是**非键列** `"name" = t."name"`，所以没暴露。
        // 这一节把「哪种占位符可用」一次问清。
        let _ = conn
            .execute("DROP TABLE \"dm_ignore_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "16a. 建带非键列的表",
            "CREATE TABLE \"dm_ignore_probe\" (\"id\" INT PRIMARY KEY, \"name\" VARCHAR(50))",
        )
        .await;

        // ① 只有 `WHEN NOT MATCHED` 的 MERGE（没有 MATCHED 子句）：达梦收不收？
        let only_not_matched = "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 1 AS \"id\", 'seed' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")";
        match conn.execute(only_not_matched, ()).await {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16b. 无 MATCHED 子句的 MERGE（建种子行）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16b. 无 MATCHED 子句的 MERGE（建种子行）"
            ),
        }
        // ② 同一句再放一次（这次是命中行）：若只影响 0 行且值不变 ⇒ 它就是 DO NOTHING。
        match conn
            .execute(
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 1 AS \"id\", 'changed' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
                (),
            )
            .await
        {
            Ok(n) => println!("OK   {:<52} 影响 {n} 行", "16c. 无 MATCHED 子句的 MERGE（命中行）"),
            Err(e) => println!("FAIL {:<52} {e}", "16c. 无 MATCHED 子句的 MERGE（命中行）"),
        }
        // ③ 空更新落在**非键列**上（15f 的老形态，带表限定符）。
        match conn
            .execute(
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 1 AS \"id\", 'x' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = t.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16d. 空更新在非键列（`\"name\" = t.\"name\"`）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16d. 空更新在非键列（`\"name\" = t.\"name\"`）"
            ),
        }
        // ④ 空更新落在**ON 关联列**上（fork 现在的渲染）⇒ 期望失败，把错误码钉住。
        match conn
            .execute(
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 1 AS \"id\", 'x' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"id\" = \"id\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16e. 空更新在 ON 关联列（`\"id\" = \"id\"`）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16e. 空更新在 ON 关联列（`\"id\" = \"id\"`）"
            ),
        }
        // ⑤ 非键列不带表限定符（fork 的空更新样式）。
        match conn
            .execute(
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 1 AS \"id\", 'x' AS \"name\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"name\" = \"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"id\", s.\"name\")",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16f. 空更新在非键列（不带限定符）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16f. 空更新在非键列（不带限定符）"
            ),
        }
        // ⑥ 只有键列的表：非键占位符**不存在**，看无 MATCHED 子句的形态还行不行。
        let _ = conn
            .execute("DROP TABLE \"dm_ignore_probe2\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "16g. 建只有键列的表",
            "CREATE TABLE \"dm_ignore_probe2\" (\"id\" INT PRIMARY KEY)",
        )
        .await;
        match conn
            .execute(
                "MERGE INTO \"dm_ignore_probe2\" AS t USING (SELECT 1 AS \"id\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN NOT MATCHED THEN INSERT (\"id\") VALUES (s.\"id\")",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16h. 键列表：无 MATCHED 子句（建种子行）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16h. 键列表：无 MATCHED 子句（建种子行）"
            ),
        }
        match conn
            .execute(
                "MERGE INTO \"dm_ignore_probe2\" AS t USING (SELECT 1 AS \"id\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET \"id\" = \"id\" WHEN NOT MATCHED THEN INSERT (\"id\") VALUES (s.\"id\")",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16i. 键列表：空更新在 ON 关联列"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16i. 键列表：空更新在 ON 关联列"
            ),
        }
        match conn
            .execute(
                "MERGE INTO \"dm_ignore_probe2\" AS t USING (SELECT 1 AS \"id\" FROM DUAL) AS s ON (t.\"id\" = s.\"id\") WHEN NOT MATCHED THEN INSERT (\"id\") VALUES (s.\"id\")",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16j. 键列表：无 MATCHED 子句（命中行）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16j. 键列表：无 MATCHED 子句（命中行）"
            ),
        }
        // ⑦ 不靠 MERGE 的替代形态（`INSERT … WHERE NOT EXISTS`），留作降级路径的证据。
        match conn
            .execute(
                "INSERT INTO \"dm_ignore_probe\" (\"id\", \"name\") SELECT 1, 'nope' FROM DUAL WHERE NOT EXISTS (SELECT 1 FROM \"dm_ignore_probe\" WHERE \"id\" = 1)",
                (),
            )
            .await
        {
            Ok(n) => println!(
                "OK   {:<52} 影响 {n} 行",
                "16k. `INSERT … WHERE NOT EXISTS`（命中行）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "16k. `INSERT … WHERE NOT EXISTS`（命中行）"
            ),
        }
        match conn
            .query(
                "SELECT \"id\", \"name\" FROM \"dm_ignore_probe\" ORDER BY \"id\"",
                (),
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!(
                    "OK   {:<52} {}",
                    "16l. 现状（应当仍是 seed）",
                    rows.join(" ")
                );
            }
            Err(e) => println!("FAIL {:<52} {e}", "16l. 现状（应当仍是 seed）"),
        }

        // ⑧ Round 18 追加：「命中就什么都不做」的写法能不能**让受影响行数自己区分开**
        //    「插入」与「忽略」。这一步是 `or_ignore()` 能不能返回 `None` 的前提——
        //    MERGE 的空更新（16d）在命中行时也报 1 行受影响，驱动无从判断。
        //    两种候选写法各测「命中行」与「未命中行」：理想结果是命中 0 行、未命中 1 行。
        for (label, sql) in [
            (
                "16m. MATCHED 带条件（`AND 1 = 0`），命中行",
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 1 AS \"s_0\", 'x' AS \"s_1\" FROM DUAL) AS s ON (t.\"id\" = s.\"s_0\") WHEN MATCHED AND 1 = 0 THEN UPDATE SET \"name\" = t.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"s_0\", s.\"s_1\")",
            ),
            (
                "16n. `UPDATE … WHERE 1 = 0`，命中行",
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 1 AS \"s_0\", 'x' AS \"s_1\" FROM DUAL) AS s ON (t.\"id\" = s.\"s_0\") WHEN MATCHED THEN UPDATE SET \"name\" = t.\"name\" WHERE 1 = 0 WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"s_0\", s.\"s_1\")",
            ),
            (
                "16o. MATCHED 带条件（`AND 1 = 0`），未命中行",
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 3 AS \"s_0\", 'three' AS \"s_1\" FROM DUAL) AS s ON (t.\"id\" = s.\"s_0\") WHEN MATCHED AND 1 = 0 THEN UPDATE SET \"name\" = t.\"name\" WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"s_0\", s.\"s_1\")",
            ),
            (
                "16p. `UPDATE … WHERE 1 = 0`，未命中行",
                "MERGE INTO \"dm_ignore_probe\" AS t USING (SELECT 4 AS \"s_0\", 'four' AS \"s_1\" FROM DUAL) AS s ON (t.\"id\" = s.\"s_0\") WHEN MATCHED THEN UPDATE SET \"name\" = t.\"name\" WHERE 1 = 0 WHEN NOT MATCHED THEN INSERT (\"id\", \"name\") VALUES (s.\"s_0\", s.\"s_1\")",
            ),
        ] {
            match conn.execute(sql, ()).await {
                Ok(n) => println!("OK   {label:<52} 影响 {n} 行"),
                Err(e) => println!("FAIL {label:<52} {e}"),
            }
        }

        // === 17. `Vec<scalar>`（最后一个能力位）：达梦拿什么做「存 + 成员/超集/交集」===
        // 套件 `type_collection.rs` 要的语义（全部 `requires(vec_scalar)`）：`Vec<String>` /
        // `Vec<Zoned>` / `Vec<TimeZone>` 往返、整值替换、`contains(x)`（成员，**大小写敏感**）、
        // `is_superset([..])`、`intersects([..])`。上游只给了 PG（`text[]` + `@>`/`&&`）与
        // DynamoDB 两条路，**MySQL/SQLite 的 JSON 路被标注成 future work** ⇒ 没有参考实现，
        // 先问达梦自己：JSON 函数族 / CLOB 上的 LIKE 折不折大小写 / 原生数组语法。
        ddl(
            &mut conn,
            "17a. 建表（CLOB 存 JSON 数组）",
            "CREATE TABLE \"dm_json_probe\" (\"id\" INT, \"tags\" CLOB)",
        )
        .await;
        ddl(
            &mut conn,
            "17b. 写入 JSON 数组",
            "INSERT INTO \"dm_json_probe\" (\"id\", \"tags\") VALUES (1, '[\"admin\",\"verified\"]')",
        )
        .await;
        ddl(
            &mut conn,
            "17c. 写入对照组（大小写）",
            "INSERT INTO \"dm_json_probe\" (\"id\", \"tags\") VALUES (2, '[\"Admin\"]')",
        )
        .await;
        probe(
            &mut conn,
            "17d. JSON_VALUE 取元素",
            "SELECT JSON_VALUE(\"tags\", '$[0]') FROM \"dm_json_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "17e. JSON_EXISTS 判成员",
            "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_EXISTS(\"tags\", '$[*]?(@ == \"admin\")')",
            (),
        )
        .await;
        probe(
            &mut conn,
            "17f. JSON_QUERY 取整数组",
            "SELECT JSON_QUERY(\"tags\", '$') FROM \"dm_json_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "17g. JSON_TABLE 展开",
            "SELECT * FROM JSON_TABLE((SELECT \"tags\" FROM \"dm_json_probe\" WHERE \"id\" = 1), '$[*]' COLUMNS (\"v\" VARCHAR(50) PATH '$'))",
            (),
        )
        .await;
        // 没有 JSON 函数时只剩字符串手段 ⇒ 先量清 CLOB 上的 LIKE 到底折不折大小写。
        probe(
            &mut conn,
            "17h. LIKE 小写（应只命中 id=1）",
            "SELECT \"id\" FROM \"dm_json_probe\" WHERE \"tags\" LIKE '%\"admin\"%'",
            (),
        )
        .await;
        probe(
            &mut conn,
            "17i. LIKE 大写（应只命中 id=2）",
            "SELECT \"id\" FROM \"dm_json_probe\" WHERE \"tags\" LIKE '%\"Admin\"%'",
            (),
        )
        .await;
        probe(
            &mut conn,
            "17j. INSTR 定位（大小写）",
            "SELECT INSTR(\"tags\", '\"admin\"') FROM \"dm_json_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "17k. 大小写敏感比对（COLLATE）",
            "SELECT \"id\" FROM \"dm_json_probe\" WHERE \"tags\" COLLATE BINARY LIKE '%\"admin\"%'",
            (),
        )
        .await;
        // 原生数组 / 集合判定语法（若有，可能比 JSON 文本更贴近上游语义）。
        ddl(
            &mut conn,
            "17l. 原生数组列？",
            "CREATE TABLE \"dm_arr_probe\" (\"id\" INT, \"tags\" VARCHAR(50) ARRAY)",
        )
        .await;
        probe(
            &mut conn,
            "17m. MEMBER OF",
            "SELECT 1 FROM DUAL WHERE 'a' MEMBER OF ('a', 'b')",
            (),
        )
        .await;
        probe(
            &mut conn,
            "17n. SUBMULTISET",
            "SELECT 1 FROM DUAL WHERE ('a', 'b') SUBMULTISET OF ('a', 'b', 'c')",
            (),
        )
        .await;
        // 17d/17f 说明 JSON_VALUE / JSON_QUERY 在。上一轮把「带过滤路径的 JSON_EXISTS」
        // 与 JSON_TABLE 都否掉了，这一轮把候选函数**逐个点名**，看有没有能一次吃下
        // 「整数组参数」的判定函数（有 ⇒ `is_superset`/`intersects` 可能一步到位）。
        for (label, sql) in [
            (
                "17o. JSON_EXISTS(简单路径)",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_EXISTS(\"tags\", '$[0]')",
            ),
            (
                "17p. JSON_EXISTS(通配)",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_EXISTS(\"tags\", '$[*]')",
            ),
            (
                "17q. JSON_VALUE(通配)",
                "SELECT JSON_VALUE(\"tags\", '$[*]') FROM \"dm_json_probe\" WHERE \"id\" = 1",
            ),
            (
                "17r. JSON_LENGTH",
                "SELECT JSON_LENGTH(\"tags\") FROM \"dm_json_probe\" WHERE \"id\" = 1",
            ),
            (
                "17s. JSON_CONTAINS",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_CONTAINS(\"tags\", '[\"admin\"]')",
            ),
            (
                "17t. JSON_OVERLAPS",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_OVERLAPS(\"tags\", '[\"admin\"]')",
            ),
            (
                "17u. JSON_SEARCH",
                "SELECT JSON_SEARCH(\"tags\", 'one', 'admin') FROM \"dm_json_probe\" WHERE \"id\" = 1",
            ),
            (
                "17v. JSON_ARRAY 构造",
                "SELECT JSON_ARRAY('a', 'b') FROM DUAL",
            ),
            (
                "17w. JSON_EQUAL",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_EQUAL(\"tags\", '[\"Admin\"]')",
            ),
            (
                "17x. INSTR 成员判定（带引号）",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE INSTR(\"tags\", '\"' || 'admin' || '\"') > 0",
            ),
            (
                "17y. LIKE 成员判定（拼接 + 转义）",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE \"tags\" LIKE '%' || '\"admin\"' || '%' ESCAPE '!'",
            ),
        ] {
            match conn.query(sql, ()).await {
                Ok(rs) => {
                    let rows: Vec<String> = rs
                        .rows()
                        .iter()
                        .map(|row| format!("{:?}", row.values()))
                        .collect();
                    println!("OK   {label:<52} {}", rows.join(" "));
                }
                Err(e) => println!("FAIL {label:<52} {e}"),
            }
        }
        // 列表列的可索引性（`CLOB` 不能建索引/`CHECK`，第 7 条）⇒ 换 `VARCHAR(8188)` 试试。
        ddl(
            &mut conn,
            "17z. 列表列换 VARCHAR(8188) + 索引",
            "CREATE TABLE \"dm_list_probe\" (\"id\" INT, \"tags\" VARCHAR(8188))",
        )
        .await;
        ddl(
            &mut conn,
            "17z2. 在列表列上建索引",
            "CREATE INDEX \"dm_list_probe_idx\" ON \"dm_list_probe\" (\"tags\")",
        )
        .await;

        // 17e/17g 的失败可能只是**语法**（Oracle 的路径过滤器与 JSON_TABLE 列子句各有写法）。
        // 这一批把 JSON_EXISTS / JSON_TABLE 的几种写法逐个试；只要有一种「按 JSON 数组逐元素
        // 出行」的写法能过，`is_superset` / `intersects` 就能照 SQLite 那套 `json_each` 思路落。
        for (label, sql) in [
            (
                "17aa. JSON_EXISTS(等号无空格)",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_EXISTS(\"tags\", '$[*]?(@==\"admin\")')",
            ),
            (
                "17ab. JSON_EXISTS(过滤 + PASSING 变量)",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_EXISTS(\"tags\", '$[*]?(@ == $v)' PASSING 'admin' AS \"v\")",
            ),
            (
                "17ac. JSON_EXISTS(路径带不存在元素)",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_EXISTS(\"tags\", '$[0]' || '')",
            ),
            (
                "17ad. JSON_QUERY(通配)",
                "SELECT JSON_QUERY(\"tags\", '$[*]') FROM \"dm_json_probe\" WHERE \"id\" = 1",
            ),
            (
                "17ae. JSON_VALUE(下标 1)",
                "SELECT JSON_VALUE(\"tags\", '$[1]') FROM \"dm_json_probe\" WHERE \"id\" = 1",
            ),
            (
                "17af. JSON_TABLE(字面量，简单列子句)",
                "SELECT \"v\" FROM JSON_TABLE('[\"a\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$'))",
            ),
            (
                "17ag. JSON_TABLE(Oracle 列子句带 FOR ORDINALITY)",
                "SELECT \"v\" FROM JSON_TABLE('[\"a\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"n\" FOR ORDINALITY))",
            ),
            (
                "17ah. JSON_TABLE(关联表列，逐元素出行)",
                "SELECT p.\"id\", jt.\"v\" FROM \"dm_json_probe\" p, JSON_TABLE(p.\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt",
            ),
            (
                "17ai. 子集判定（SQLite 思路：NOT EXISTS 反例）",
                "SELECT p.\"id\" FROM \"dm_json_probe\" p WHERE NOT EXISTS (SELECT 1 FROM JSON_TABLE('[\"admin\",\"verified\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt WHERE NOT EXISTS (SELECT 1 FROM JSON_TABLE(p.\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) j2 WHERE j2.\"v\" = jt.\"v\"))",
            ),
        ] {
            match conn.query(sql, ()).await {
                Ok(rs) => {
                    let rows: Vec<String> = rs
                        .rows()
                        .iter()
                        .map(|row| format!("{:?}", row.values()))
                        .collect();
                    println!("OK   {label:<52} {}", rows.join(" "));
                }
                Err(e) => println!("FAIL {label:<52} {e}"),
            }
        }
        // 关键一问：JSON_TABLE 的输入能不能是**参数**（`is_superset` 的右值就是一个整数组参数）。
        match conn
            .query(
                "SELECT \"v\" FROM JSON_TABLE(?, '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$'))",
                vec![DmValue::Text("[\"a\",\"b\"]".into())],
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!("OK   {:<52} {}", "17aj. JSON_TABLE(参数)", rows.join(" "));
            }
            Err(e) => println!("FAIL {:<52} {e}", "17aj. JSON_TABLE(参数)"),
        }

        // 17s/17t 用的是字面量；而 fork 渲染出来的 `is_superset` / `intersects` 右值是
        // **一个绑定参数**（JSON 数组文本）。判定函数必须能吃参数，方案才成立。
        for (label, sql, args) in [
            (
                "17ak. JSON_CONTAINS(列, 参数)",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_CONTAINS(\"tags\", ?)",
                vec![DmValue::Text("[\"admin\"]".into())],
            ),
            (
                "17al. JSON_OVERLAPS(列, 参数)",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_OVERLAPS(\"tags\", ?)",
                vec![DmValue::Text("[\"verified\",\"nope\"]".into())],
            ),
            (
                "17am. JSON_CONTAINS(列, JSON_ARRAY(标量参数))",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_CONTAINS(\"tags\", JSON_ARRAY(?))",
                vec![DmValue::Text("admin".into())],
            ),
            (
                "17an. .contains 大小写敏感（应 0 行）",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_CONTAINS(\"tags\", JSON_ARRAY(?))",
                vec![DmValue::Text("ADMIN".into())],
            ),
            (
                "17ao. JSON_CONTAINS 数字元素",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_CONTAINS('[1,2,3]', ?)",
                vec![DmValue::Text("[2]".into())],
            ),
            (
                "17ap. JSON_CONTAINS 子集语义（应 1 行）",
                "SELECT \"id\" FROM \"dm_json_probe\" WHERE JSON_CONTAINS(\"tags\", ?)",
                vec![DmValue::Text("[\"admin\",\"verified\"]".into())],
            ),
        ] {
            match conn.query(sql, args).await {
                Ok(rs) => {
                    let rows: Vec<String> = rs
                        .rows()
                        .iter()
                        .map(|row| format!("{:?}", row.values()))
                        .collect();
                    println!("OK   {label:<52} {}", rows.join(" "));
                }
                Err(e) => println!("FAIL {label:<52} {e}"),
            }
        }
        // JSON 函数在 `VARCHAR(8188)` 上是否同样可用（`CLOB` 不好建索引）。
        ddl(
            &mut conn,
            "17aq. VARCHAR 列写 JSON 数组",
            "INSERT INTO \"dm_list_probe\" (\"id\", \"tags\") VALUES (1, '[\"admin\"]')",
        )
        .await;
        match conn
            .query(
                "SELECT \"id\" FROM \"dm_list_probe\" WHERE JSON_CONTAINS(\"tags\", ?)",
                vec![DmValue::Text("[\"admin\"]".into())],
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!(
                    "OK   {:<52} {}",
                    "17ar. JSON_CONTAINS 在 VARCHAR 列上",
                    rows.join(" ")
                );
            }
            Err(e) => println!("FAIL {:<52} {e}", "17ar. JSON_CONTAINS 在 VARCHAR 列上"),
        }

        // ---- 第 18 节：列表原地删除（`vec_pop` / `vec_remove` / `vec_remove_at`）----
        // Round 20 只落了集合谓词与 push/extend/clear（走 `serialize_append` 的字符串拼接）。
        // 套件里还有三条用例被 `vec_pop`/`vec_remove`/`vec_remove_at` 挡着：`pop()`（去掉末元素、
        // 空列表 no-op）、`remove(v)`（**删掉所有等于 v 的元素**）、`remove_at(i)`（去掉下标 i、
        // 越界 no-op）。我们的列表列是 CLOB 里的 JSON 数组文本 ⇒ 原地删除只有两条路：
        // ① JSON 改写函数（`JSON_REMOVE` 一族）；② `JSON_TABLE` 拆元素 + `JSON_ARRAYAGG` 重建。
        ddl(
            &mut conn,
            "18a. 建列表删除探针表",
            "CREATE TABLE \"dm_vecrm_probe\" (\"id\" INT, \"tags\" CLOB)",
        )
        .await;
        ddl(
            &mut conn,
            "18b. 写种子行（含重复元素）",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (1, '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]')",
        )
        .await;
        for (label, sql) in [
            (
                "18c. JSON_REMOVE(字面量, 下标)",
                "SELECT JSON_REMOVE('[\"a\",\"b\",\"c\"]', '$[1]') FROM DUAL",
            ),
            (
                "18d. JSON_REMOVE 越界（应 no-op）",
                "SELECT JSON_REMOVE('[\"a\",\"b\"]', '$[5]') FROM DUAL",
            ),
            (
                "18e. JSON_REMOVE 空数组 + 负下标",
                "SELECT JSON_REMOVE('[]', '$[-1]') FROM DUAL",
            ),
            (
                "18f. pop 写法：路径由 JSON_LENGTH 算",
                "SELECT JSON_REMOVE('[\"a\",\"b\",\"c\"]', '$[' || (JSON_LENGTH('[\"a\",\"b\",\"c\"]') - 1) || ']') FROM DUAL",
            ),
            (
                "18g. pop 空列表（路径 $[-1]）",
                "SELECT JSON_REMOVE('[]', '$[' || (JSON_LENGTH('[]') - 1) || ']') FROM DUAL",
            ),
            (
                "18h. JSON_SEARCH('all') 多命中",
                "SELECT JSON_SEARCH('[\"a\",\"dup\",\"b\",\"dup\"]', 'all', 'dup') FROM DUAL",
            ),
            (
                "18i. JSON_SEARCH('all') 无命中",
                "SELECT JSON_SEARCH('[\"a\",\"b\"]', 'all', 'missing') FROM DUAL",
            ),
            (
                "18j. JSON_REMOVE 多个路径参数",
                "SELECT JSON_REMOVE('[\"a\",\"dup\",\"b\",\"dup\"]', '$[1]', '$[3]') FROM DUAL",
            ),
            (
                "18k. JSON_ARRAYAGG + JSON_TABLE 重建（去 dup）",
                "SELECT JSON_ARRAYAGG(jt.\"v\") FROM JSON_TABLE('[\"a\",\"dup\",\"b\",\"dup\",\"c\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt WHERE jt.\"v\" <> 'dup'",
            ),
            (
                "18l. JSON_ARRAYAGG 零行结果",
                "SELECT JSON_ARRAYAGG(jt.\"v\") FROM JSON_TABLE('[]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt",
            ),
            (
                "18m. JSON_SET",
                "SELECT JSON_SET('[\"a\",\"b\"]', '$[0]', 'z') FROM DUAL",
            ),
            (
                "18n. JSON_ARRAY_APPEND",
                "SELECT JSON_ARRAY_APPEND('[\"a\"]', '$', 'b') FROM DUAL",
            ),
            (
                "18o. JSON_INSERT",
                "SELECT JSON_INSERT('[\"a\"]', '$[1]', 'b') FROM DUAL",
            ),
            (
                "18p. JSON_MERGE_PRESERVE",
                "SELECT JSON_MERGE_PRESERVE('[\"a\"]', '[\"b\"]') FROM DUAL",
            ),
            (
                "18q. JSON_REPLACE",
                "SELECT JSON_REPLACE('[\"a\",\"b\"]', '$[0]', 'z') FROM DUAL",
            ),
            (
                "18r. JSON_KEYS",
                "SELECT JSON_KEYS('[\"a\",\"b\"]') FROM DUAL",
            ),
            ("18s. JSON_VALID", "SELECT JSON_VALID('[\"a\"]') FROM DUAL"),
            (
                "18x. 重建时元素类型（数字会不会变字符串）",
                "SELECT JSON_ARRAYAGG(jt.\"v\") FROM JSON_TABLE('[1,2,3]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt WHERE jt.\"v\" <> '2'",
            ),
            (
                "18y. FORMAT JSON 列子句",
                "SELECT JSON_ARRAYAGG(jt.\"v\") FROM JSON_TABLE('[1,2,3]', '$[*]' COLUMNS (\"v\" VARCHAR(100) FORMAT JSON PATH '$')) jt",
            ),
            (
                "18z. JSON_QUERY 取单个元素（带引号）",
                "SELECT JSON_QUERY('[\"a\",\"b\"]', '$[0]') FROM DUAL",
            ),
            (
                "18z2. JSON_ARRAYAGG 带 ORDER BY（FOR ORDINALITY）",
                "SELECT JSON_ARRAYAGG(jt.\"v\" ORDER BY jt.\"n\") FROM JSON_TABLE('[\"a\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"n\" FOR ORDINALITY)) jt",
            ),
            (
                "18z3. JSON_SEARCH('all') 两个命中",
                "SELECT JSON_SEARCH('[\"dup\",\"x\",\"dup\"]', 'all', 'dup') FROM DUAL",
            ),
            (
                "18z4. JSON_REMOVE 的路径参数直接给 JSON_SEARCH 结果",
                "SELECT JSON_REMOVE('[\"dup\",\"x\",\"dup\"]', JSON_SEARCH('[\"dup\",\"x\",\"dup\"]', 'all', 'dup')) FROM DUAL",
            ),
            (
                "18z5. JSON_ARRAYAGG 全列（含引号元素）",
                "SELECT JSON_ARRAYAGG(jt.\"v\") FROM JSON_TABLE('[\"a\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt",
            ),
            (
                "18z6. JSON_TABLE 的 v 与原文本比较（数字）",
                "SELECT jt.\"v\" FROM JSON_TABLE('[1,2,3]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt WHERE jt.\"v\" = '2'",
            ),
            (
                "18z7. JSON_REMOVE 带过滤路径（Oracle 风格）",
                "SELECT JSON_REMOVE('[\"a\",\"dup\",\"b\",\"dup\"]', '$[*]?(@ == \"dup\")') FROM DUAL",
            ),
            (
                "18t. JSON_REMOVE 作用在表列上",
                "SELECT JSON_REMOVE(\"tags\", '$[0]') FROM \"dm_vecrm_probe\" WHERE \"id\" = 1",
            ),
        ] {
            match conn.query(sql, ()).await {
                Ok(rs) => {
                    let rows: Vec<String> = rs
                        .rows()
                        .iter()
                        .map(|row| format!("{:?}", row.values()))
                        .collect();
                    println!("OK   {label:<52} {}", rows.join(" "));
                }
                Err(e) => println!("FAIL {label:<52} {e}"),
            }
        }
        // `remove_at(i)` 的 i 在 fork 里是**一个表达式**（套件传的是 `usize` 字面量，会做成参数）。
        // ⇒ 路径必须能用参数拼出来。
        match conn
            .query(
                "SELECT JSON_REMOVE(\"tags\", '$[' || ? || ']') FROM \"dm_vecrm_probe\" WHERE \"id\" = 1",
                vec![DmValue::Int(2)],
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!("OK   {:<52} {}", "18u. JSON_REMOVE 路径带参数", rows.join(" "));
            }
            Err(e) => println!("FAIL {:<52} {e}", "18u. JSON_REMOVE 路径带参数"),
        }
        // `remove(v)` 的 v 在 fork 里也是参数 ⇒ `JSON_SEARCH` 的值必须能吃参数。
        match conn
            .query(
                "SELECT JSON_SEARCH('[\"dup\",\"x\",\"dup\"]', 'all', ?) FROM DUAL",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!(
                    "OK   {:<52} {}",
                    "18z8. JSON_SEARCH 的值用参数",
                    rows.join(" ")
                );
            }
            Err(e) => println!("FAIL {:<52} {e}", "18z8. JSON_SEARCH 的值用参数"),
        }
        // 如果 18j 的多路径不成立，`remove(v)` 只能靠重建。重建要能在 **UPDATE 的赋值里**
        // 关联到本行的列（Oracle 系允许 `SET c = (SELECT … FROM JSON_TABLE(c …))`？要问）。
        ddl(
            &mut conn,
            "18v. UPDATE 赋值里用 JSON_TABLE + JSON_ARRAYAGG 重建",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = (SELECT JSON_ARRAYAGG(jt.\"v\") FROM JSON_TABLE(\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$')) jt WHERE jt.\"v\" <> 'dup') WHERE \"id\" = 1",
        )
        .await;
        probe(
            &mut conn,
            "18w. 18v 之后的行内容",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;

        // ---- 第 19 节：删除三件事的**落地方案**再确认 ----
        // 18f/18g 说明「路径里出现 `-1`」会被判 `-3102 JSON路径表达式语法错误`（空列表时
        // `JSON_LENGTH - 1 = -1`）⇒ `pop` 的路径必须先夹到 0；18d 又说明**越界路径是 no-op**，
        // 所以「空列表时用 `$[0]`」就等价于什么都不删。
        // `remove(v)` 那条更难：18k/18l/18x/18z5 证明达梦**没有 `JSON_ARRAYAGG`**（`-2207`），
        // 18z4/18z7 证明 `JSON_REMOVE` 既不吃「路径数组」也不吃过滤路径；18j 还证明多个路径
        // 是**顺序施加**（`$[1]`+`$[3]` 只删掉第一个 ⇒ 不能靠 `JSON_SEARCH('all')` 的结果）。
        // ⇒ 只剩「`JSON_TABLE` 逐元素 + `LISTAGG` 拼回 JSON 文本」这条路，这一节把它问完。
        for (label, sql) in [
            (
                "19a. JSON_REMOVE 空数组 + $[0]（应 no-op）",
                "SELECT JSON_REMOVE('[]', '$[0]') FROM DUAL",
            ),
            (
                "19b. pop 路径夹 0（非空）",
                "SELECT JSON_REMOVE('[\"a\",\"b\",\"c\"]', '$[' || CASE WHEN JSON_LENGTH('[\"a\",\"b\",\"c\"]') = 0 THEN 0 ELSE JSON_LENGTH('[\"a\",\"b\",\"c\"]') - 1 END || ']') FROM DUAL",
            ),
            (
                "19c. pop 路径夹 0（空）",
                "SELECT JSON_REMOVE('[]', '$[' || CASE WHEN JSON_LENGTH('[]') = 0 THEN 0 ELSE JSON_LENGTH('[]') - 1 END || ']') FROM DUAL",
            ),
            (
                "19d. pop 用 GREATEST（非空）",
                "SELECT JSON_REMOVE('[\"a\",\"b\",\"c\"]', '$[' || GREATEST(JSON_LENGTH('[\"a\",\"b\",\"c\"]') - 1, 0) || ']') FROM DUAL",
            ),
            (
                "19e. pop 用 GREATEST（空）",
                "SELECT JSON_REMOVE('[]', '$[' || GREATEST(JSON_LENGTH('[]') - 1, 0) || ']') FROM DUAL",
            ),
            (
                "19f. LISTAGG + JSON_TABLE 重建（去 dup）",
                "SELECT LISTAGG(jt.\"v\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'",
            ),
            (
                "19g. LISTAGG 零行（全被删光）",
                "SELECT LISTAGG(jt.\"v\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE('[\"dup\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'",
            ),
            ("19h. JSON_QUOTE", "SELECT JSON_QUOTE('a') FROM DUAL"),
            (
                "19i. NVL 包住零行 LISTAGG",
                "SELECT NVL(LISTAGG(jt.\"v\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') FROM JSON_TABLE('[\"dup\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'",
            ),
        ] {
            match conn.query(sql, ()).await {
                Ok(rs) => {
                    let rows: Vec<String> = rs
                        .rows()
                        .iter()
                        .map(|row| format!("{:?}", row.values()))
                        .collect();
                    println!("OK   {label:<52} {}", rows.join(" "));
                }
                Err(e) => println!("FAIL {label:<52} {e}"),
            }
        }
        // `pop` 落在真正的 UPDATE 赋值上（表列、CASE 夹 0；CLOB 的 CASE 类型解析要问）。
        ddl(
            &mut conn,
            "19j. 写 id=2 种子行",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (2, '[\"a\",\"b\",\"c\"]')",
        )
        .await;
        ddl(
            &mut conn,
            "19k. UPDATE: pop（CASE 夹 0）",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = CASE WHEN JSON_LENGTH(\"tags\") = 0 THEN \"tags\" ELSE JSON_REMOVE(\"tags\", '$[' || (JSON_LENGTH(\"tags\") - 1) || ']') END WHERE \"id\" = 2",
        )
        .await;
        probe(
            &mut conn,
            "19l. 19k 之后的行内容",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 2",
            (),
        )
        .await;
        ddl(
            &mut conn,
            "19m. 再 pop 一次（空列表路径）",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = CASE WHEN JSON_LENGTH(\"tags\") = 0 THEN \"tags\" ELSE JSON_REMOVE(\"tags\", '$[' || (JSON_LENGTH(\"tags\") - 1) || ']') END WHERE \"id\" = 2",
        )
        .await;
        ddl(
            &mut conn,
            "19n. 第三次 pop（这次列表已空）",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = CASE WHEN JSON_LENGTH(\"tags\") = 0 THEN \"tags\" ELSE JSON_REMOVE(\"tags\", '$[' || (JSON_LENGTH(\"tags\") - 1) || ']') END WHERE \"id\" = 2",
        )
        .await;
        probe(
            &mut conn,
            "19o. 三次 pop 之后的行内容（应 []）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 2",
            (),
        )
        .await;
        // `remove(v)` 的 UPDATE 形状：相关子查询把本行列逐元素拆开、`JSON_QUOTE` 还原元素、
        // `LISTAGG` 拼回、`NVL` 兜住「全删光」。元素类型先只按**字符串**试通。
        ddl(
            &mut conn,
            "19p. 写 id=3 种子行",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (3, '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]')",
        )
        .await;
        ddl(
            &mut conn,
            "19q. UPDATE: remove('dup')（JSON_TABLE + LISTAGG）",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[' || NVL((SELECT LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE(\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'), '') || ']' WHERE \"id\" = 3",
        )
        .await;
        probe(
            &mut conn,
            "19r. 19q 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 3",
            (),
        )
        .await;
        ddl(
            &mut conn,
            "19s. UPDATE: remove 时把全部元素删光",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[' || NVL((SELECT LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE(\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'a' AND jt.\"v\" <> 'b' AND jt.\"v\" <> 'c'), '') || ']' WHERE \"id\" = 3",
        )
        .await;
        probe(
            &mut conn,
            "19t. 19s 之后的行内容（应 []）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 3",
            (),
        )
        .await;

        // `FORMAT JSON` 若能用在 `JSON_TABLE` 的列规格上，`LISTAGG` 就能直接拼出
        // **与元素类型无关**的 JSON 文本（字符串自动带引号、数字不带）⇒ `remove(v)`
        // 的臂不需要看列的元素类型。同时试「原文本列 + JSON 列」两列并存（比较用前者，
        // 重建用后者 ⇒ 绑定的 `?` 是裸文本）。
        for (label, sql) in [
            (
                "19u. JSON_TABLE FORMAT JSON 列",
                "SELECT jt.\"j\" FROM JSON_TABLE('[\"a\",2,\"b\"]', '$[*]' COLUMNS (\"j\" VARCHAR(100) FORMAT JSON PATH '$')) jt",
            ),
            (
                "19v. 原文本列 + JSON 列 + 序号",
                "SELECT jt.\"v\", jt.\"j\", jt.\"n\" FROM JSON_TABLE('[\"a\",2,\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"j\" VARCHAR(100) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt",
            ),
            (
                "19w. LISTAGG 用 JSON 列重建",
                "SELECT '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(100) PATH '$', \"j\" VARCHAR(100) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'",
            ),
        ] {
            match conn.query(sql, ()).await {
                Ok(rs) => {
                    let rows: Vec<String> = rs
                        .rows()
                        .iter()
                        .map(|row| format!("{:?}", row.values()))
                        .collect();
                    println!("OK   {label:<52} {}", rows.join(" "));
                }
                Err(e) => println!("FAIL {label:<52} {e}"),
            }
        }
        // 相关子查询版的 `remove(v)`（表列 + 绑定参数），两种重建方式都试。
        ddl(
            &mut conn,
            "19x. 写 id=4 种子行",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (4, '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]')",
        )
        .await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[' || NVL((SELECT LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE(\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> ?), '') || ']' WHERE \"id\" = 4",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "19y. UPDATE: remove 用 JSON 列（子查询 + 参数）"),
            Err(e) => println!("FAIL {:<52} {e}", "19y. UPDATE: remove 用 JSON 列（子查询 + 参数）"),
        }
        probe(
            &mut conn,
            "19z. 19y 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 4",
            (),
        )
        .await;
        // 数字元素上的 `remove(v)`：`jt."v"` 是 TEXT 列，而引擎给的是**数字参数**
        // （`Vec<i64>` 的 `remove(2)` 会绑 `Int`）⇒ 隐式转换会不会把 TEXT 列转成数字？
        // 这一问决定 `serialize_remove` 要不要把比较写成 `jt."v" <> CAST(? AS VARCHAR(8188))`。
        ddl(
            &mut conn,
            "19z2. 写 id=5 数字数组种子行",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (5, '[1,2,3]')",
        )
        .await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[' || NVL((SELECT LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE(\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> ?), '') || ']' WHERE \"id\" = 5",
                vec![DmValue::Int(2)],
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "19z2. remove 数字元素（裸参数 Int）"),
            Err(e) => println!("FAIL {:<52} {e}", "19z2. remove 数字元素（裸参数 Int）"),
        }
        probe(
            &mut conn,
            "19z3. 19z2 之后的行内容（应 [1,3] 或 [\"1\",\"3\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 5",
            (),
        )
        .await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[' || NVL((SELECT LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE(\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> CAST(? AS VARCHAR(8188))), '') || ']' WHERE \"id\" = 5",
                vec![DmValue::Int(3)],
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "19z4. remove 数字元素（CAST 到 VARCHAR）"),
            Err(e) => println!("FAIL {:<52} {e}", "19z4. remove 数字元素（CAST 到 VARCHAR）"),
        }
        probe(
            &mut conn,
            "19z5. 19z4 之后的行内容（应 []）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 5",
            (),
        )
        .await;
        // `remove(v)` 的**相关子查询**写法在真机上被拒（19q/19s/19y/19z2/19z4 一律
        // `[HY000/-4083] 引用列未找到`：`JSON_TABLE` 的第一个参数不接受与外层 UPDATE
        // 目标相关的列引用）。下面试三种**非相关**改写，找一个能在 UPDATE 里用的：
        //   A：`JSON_TABLE` 吃字面量（只验「子查询 + LISTAGG 放进 SET」这个形态）
        //   B：`JSON_TABLE` 吃 `(SELECT 列 FROM 表 WHERE 主键 = N)`（非相关标量子查询）
        //   C：`MERGE INTO … USING (SELECT 主键, 列 FROM 表) AS s … JSON_TABLE(s.列, …)`
        let seed6 = "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]' WHERE \"id\" = 6";
        ddl(
            &mut conn,
            "19A0. 写 id=6 种子行",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (6, '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]')",
        )
        .await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[' || NVL((SELECT LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE('[\"a\",\"dup\",\"b\",\"dup\",\"c\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'), '') || ']' WHERE \"id\" = 6",
                (),
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "19A. remove(A: 字面量输入)"),
            Err(e) => println!("FAIL {:<52} {e}", "19A. remove(A: 字面量输入)"),
        }
        probe(
            &mut conn,
            "19A2. A 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;
        ddl(&mut conn, "19B0. 复位 id=6", seed6).await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[' || NVL((SELECT LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE((SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6), '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> ?), '') || ']' WHERE \"id\" = 6",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "19B. remove(B: 非相关标量子查询)"),
            Err(e) => println!("FAIL {:<52} {e}", "19B. remove(B: 非相关标量子查询)"),
        }
        probe(
            &mut conn,
            "19B2. B 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;
        ddl(&mut conn, "19C0. 复位 id=6", seed6).await;
        match conn
            .execute(
                "MERGE INTO \"dm_vecrm_probe\" AS t USING (SELECT \"id\" AS \"k\", \"tags\" AS \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6) AS s ON (t.\"id\" = s.\"k\") WHEN MATCHED THEN UPDATE SET t.\"tags\" = '[' || NVL((SELECT LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\") FROM JSON_TABLE(s.\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> ?), '') || ']'",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "19C. remove(C: MERGE + 派生表)"),
            Err(e) => println!("FAIL {:<52} {e}", "19C. remove(C: MERGE + 派生表)"),
        }
        probe(
            &mut conn,
            "19C2. C 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;

        // ---- 第 20 节：`remove(v)` 的最后一条 SQL 希望 —— JSON 路径过滤表达式 ----
        // 第 19 节的三个形状全灭：A（字面量输入 + `LISTAGG` 重建）在 UPDATE 的 SET 子查询里
        // **静默写出 `[]`**（`影响 1 行` 却把整列清空，最危险的一种失败）、B（非相关标量子查询）
        // 报 `-2205` 语法错误、C（MERGE + 派生表）报 `-4083 引用列未找到`。
        // 还有一种没问过的写法：让**路径自己过滤** —— `JSON_QUERY(列, '$[*]?(@ != "dup")')`。
        // 它不需要子查询、不需要 `JSON_TABLE`，与已证明可用的 `JSON_REMOVE` 是同一层。
        for (label, sql) in [
            (
                "20a. JSON_QUERY 过滤路径（字面量）",
                "SELECT JSON_QUERY('[\"a\",\"dup\",\"b\",\"dup\",\"c\"]', '$[*]?(@ != \"dup\")') FROM DUAL",
            ),
            (
                "20b. JSON_QUERY 过滤路径（数字）",
                "SELECT JSON_QUERY('[1,5,9]', '$[*]?(@ != 5)') FROM DUAL",
            ),
            (
                "20c. JSON_QUERY 过滤路径（表列）",
                "SELECT JSON_QUERY(\"tags\", '$[*]?(@ != \"dup\")') FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            ),
            (
                "20d. 全被滤掉时返回什么",
                "SELECT JSON_QUERY('[\"dup\"]', '$[*]?(@ != \"dup\")') FROM DUAL",
            ),
            (
                "20e. JSON_QUERY 通配无过滤（基线）",
                "SELECT JSON_QUERY('[\"a\",\"dup\"]', '$[*]') FROM DUAL",
            ),
        ] {
            probe(&mut conn, label, sql, ()).await;
        }
        probe(
            &mut conn,
            "20f. 过滤路径 + 拼接参数",
            "SELECT JSON_QUERY('[\"a\",\"dup\",\"b\"]', '$[*]?(@ != \"' || ? || '\")') FROM DUAL",
            vec![DmValue::Text("dup".into())],
        )
        .await;
        ddl(&mut conn, "20g0. 复位 id=6", seed6).await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = NVL(JSON_QUERY(\"tags\", '$[*]?(@ != \"dup\")'), '[]') WHERE \"id\" = 6",
                (),
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "20g. remove 的 UPDATE（过滤路径）"
            ),
            Err(e) => println!("FAIL {:<52} {e}", "20g. remove 的 UPDATE（过滤路径）"),
        }
        probe(
            &mut conn,
            "20g2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;
        ddl(
            &mut conn,
            "20h0. 写 id=7 种子行",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (7, '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]')",
        )
        .await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = NVL(JSON_QUERY(\"tags\", '$[*]?(@ != \"' || ? || '\")'), '[]') WHERE \"id\" = 7",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "20h. remove 的 UPDATE（过滤路径 + 参数）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "20h. remove 的 UPDATE（过滤路径 + 参数）"
            ),
        }
        probe(
            &mut conn,
            "20h2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 7",
            (),
        )
        .await;

        // 20i/20j/20k：把 `JSON_TABLE` 挪到**普通 SELECT 的上下文**里（它在那里已经证明可用，
        // 见 17ah/19f），而不是塞进 SET 表达式的子查询里 —— 形状分别是「UPDATE + 表别名」
        // 与「MERGE … USING 派生表 + GROUP BY」，SET 只引用派生表算好的列。
        probe(
            &mut conn,
            "20i0. 派生表 + JSON_TABLE + LISTAGG（纯查询）",
            "SELECT src.\"k\", '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM (SELECT \"id\" AS \"k\", \"tags\" AS \"c\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6) src, JSON_TABLE(src.\"c\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup' GROUP BY src.\"k\"",
            (),
        )
        .await;
        ddl(&mut conn, "20i1. 复位 id=6", seed6).await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" x SET \"tags\" = (SELECT '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE(x.\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup') WHERE x.\"id\" = 6",
                (),
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "20i. UPDATE 表别名 + 相关 JSON_TABLE"
            ),
            Err(e) => println!("FAIL {:<52} {e}", "20i. UPDATE 表别名 + 相关 JSON_TABLE"),
        }
        probe(
            &mut conn,
            "20i2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;
        ddl(&mut conn, "20j0. 复位 id=6", seed6).await;
        match conn
            .execute(
                "MERGE INTO \"dm_vecrm_probe\" AS t USING (SELECT src.\"k\" AS \"k\", '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' AS \"new_tags\" FROM (SELECT \"id\" AS \"k\", \"tags\" AS \"c\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6) src, JSON_TABLE(src.\"c\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup' GROUP BY src.\"k\") AS s ON (t.\"id\" = s.\"k\") WHEN MATCHED THEN UPDATE SET t.\"tags\" = s.\"new_tags\"",
                (),
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "20j. MERGE + 派生表重建"),
            Err(e) => println!("FAIL {:<52} {e}", "20j. MERGE + 派生表重建"),
        }
        probe(
            &mut conn,
            "20j2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;
        ddl(
            &mut conn,
            "20k0. 复位 id=7",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]' WHERE \"id\" = 7",
        )
        .await;
        match conn
            .execute(
                "MERGE INTO \"dm_vecrm_probe\" AS t USING (SELECT src.\"k\" AS \"k\", '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' AS \"new_tags\" FROM (SELECT \"id\" AS \"k\", \"tags\" AS \"c\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 7) src, JSON_TABLE(src.\"c\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> ? GROUP BY src.\"k\") AS s ON (t.\"id\" = s.\"k\") WHEN MATCHED THEN UPDATE SET t.\"tags\" = s.\"new_tags\"",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "20k. MERGE + 派生表 + 绑定参数"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "20k. MERGE + 派生表 + 绑定参数"
            ),
        }
        probe(
            &mut conn,
            "20k2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 7",
            (),
        )
        .await;

        // 20m/20n/20o：过滤路径要用**绑定参数**拼时，值的 JSON 引号由谁加。`JSON_QUOTE` 若可用，
        // 就能把 `?` 直接嵌进路径（`'$[*]?(@ != ' || JSON_QUOTE(?) || ')'`），
        // 免得驱动层去猜元素类型的引号规则。
        probe(
            &mut conn,
            "20m. JSON_QUOTE 的转义行为",
            "SELECT JSON_QUOTE('a\"b') FROM DUAL",
            (),
        )
        .await;
        probe(
            &mut conn,
            "20n. 过滤路径 + JSON_QUOTE(参数)",
            "SELECT JSON_QUERY('[\"a\",\"dup\",\"b\"]', '$[*]?(@ != ' || JSON_QUOTE(?) || ')') FROM DUAL",
            vec![DmValue::Text("dup".into())],
        )
        .await;
        ddl(
            &mut conn,
            "20o0. 写 id=8 种子行",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (8, '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]')",
        )
        .await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = NVL(JSON_QUERY(\"tags\", '$[*]?(@ != ' || JSON_QUOTE(?) || ')'), '[]') WHERE \"id\" = 8",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "20o. remove 的 UPDATE（过滤路径 + JSON_QUOTE 参数）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "20o. remove 的 UPDATE（过滤路径 + JSON_QUOTE 参数）"
            ),
        }
        probe(
            &mut conn,
            "20o2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 8",
            (),
        )
        .await;

        // ---- 第 21 节：`JSON_TABLE + LISTAGG` 重建为什么总是空 ----
        // 19A / 20i0 / 20j / 20k 四种形态都「执行成功但写出 `[]`」。最可能的解释是
        // `WHERE jt."v" <> 'dup'` 里的比较恒为 unknown（NULL）⇒ 一行都不剩 ⇒ `LISTAGG` 得 NULL。
        // 先把 `JSON_TABLE` 的三列原样 dump 出来（看 `PATH '$'` 到底给了什么），
        // 再逐个换比较写法，最后才谈能不能回到 UPDATE 里。
        match conn
            .query(
                "SELECT jt.\"v\", jt.\"j\", jt.\"n\" FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt",
                (),
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!("OK   {:<52} {}", "21a. JSON_TABLE 三列 dump", rows.join(" "));
            }
            Err(e) => println!("FAIL {:<52} {e}", "21a. JSON_TABLE 三列 dump"),
        }
        for (label, sql) in [
            (
                "21b. 重建（无过滤）",
                "SELECT '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt",
            ),
            (
                "21c. 重建 + v <> 'dup'",
                "SELECT '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'",
            ),
            (
                "21d. 重建 + j <> '\"dup\"'",
                "SELECT '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"j\" <> '\"dup\"'",
            ),
            (
                "21e. 重建 + NOT (v = 'dup')",
                "SELECT '[' || NVL(LISTAGG(jt.\"j\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"j\" VARCHAR(8188) FORMAT JSON PATH '$', \"n\" FOR ORDINALITY)) jt WHERE NOT (jt.\"v\" = 'dup')",
            ),
            (
                "21f. LISTAGG over jt.v（无引号形式）",
                "SELECT NVL(LISTAGG(jt.\"v\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '<NULL>') FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt",
            ),
        ] {
            probe(&mut conn, label, sql, ()).await;
        }

        // ---- 第 22 节：修好「元素 JSON 文本」之后，重建到底能不能用 ----
        // 第 21 节定位到根因：`FORMAT JSON` 那列（`"j"`）**每行都是 NULL**（21a），
        // 所以 `LISTAGG(jt."j", …)` 得 NULL ⇒ `NVL(…, '')` ⇒ `[]`。改用 `v` + `JSON_QUOTE`
        // （20m 证明它转义正确）重算元素 JSON 文本，再看重建在纯查询 / UPDATE 子查询 /
        // MERGE 派生表三种上下文里的表现 —— 这决定 `vec_remove` 能不能做。
        for (label, sql) in [
            (
                "22a. 重建 = JSON_QUOTE(v)（无过滤）",
                "SELECT '[' || NVL(LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt",
            ),
            (
                "22b. 重建 = JSON_QUOTE(v) + 过滤",
                "SELECT '[' || NVL(LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",\"dup\",\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup'",
            ),
            (
                "22c. 数字列表：v 裸文本重建",
                "SELECT '[' || NVL(LISTAGG(jt.\"v\", ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[1,2,3]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt",
            ),
            (
                "22d. 含 null 元素：NVL(JSON_QUOTE(v), 'null')",
                "SELECT '[' || NVL(LISTAGG(NVL(JSON_QUOTE(jt.\"v\"), 'null'), ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE('[\"a\",null,\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt",
            ),
        ] {
            probe(&mut conn, label, sql, ()).await;
        }
        match conn
            .query(
                "SELECT jt.\"v\", jt.\"n\" FROM JSON_TABLE('[\"a\",null,\"b\"]', '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt",
                (),
            )
            .await
        {
            Ok(rs) => {
                let rows: Vec<String> = rs
                    .rows()
                    .iter()
                    .map(|row| format!("{:?}", row.values()))
                    .collect();
                println!("OK   {:<52} {}", "22e. null 元素 dump", rows.join(" "));
            }
            Err(e) => println!("FAIL {:<52} {e}", "22e. null 元素 dump"),
        }
        ddl(&mut conn, "22f0. 复位 id=6", seed6).await;
        match conn
            .execute(
                "UPDATE \"dm_vecrm_probe\" SET \"tags\" = (SELECT '[' || NVL(LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' FROM JSON_TABLE(\"tags\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup') WHERE \"id\" = 6",
                (),
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "22f. UPDATE SET 子查询（相关 JSON_TABLE，已修元素文本）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "22f. UPDATE SET 子查询（相关 JSON_TABLE，已修元素文本）"
            ),
        }
        probe(
            &mut conn,
            "22f2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;
        ddl(&mut conn, "22g0. 复位 id=6", seed6).await;
        match conn
            .execute(
                "MERGE INTO \"dm_vecrm_probe\" AS t USING (SELECT src.\"k\" AS \"k\", '[' || NVL(LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' AS \"new_tags\" FROM (SELECT \"id\" AS \"k\", \"tags\" AS \"c\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6) src, JSON_TABLE(src.\"c\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> 'dup' GROUP BY src.\"k\") AS s ON (t.\"id\" = s.\"k\") WHEN MATCHED THEN UPDATE SET t.\"tags\" = s.\"new_tags\"",
                (),
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "22g. MERGE + 派生表（已修元素文本）"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "22g. MERGE + 派生表（已修元素文本）"
            ),
        }
        probe(
            &mut conn,
            "22g2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 6",
            (),
        )
        .await;
        ddl(
            &mut conn,
            "22h0. 复位 id=7",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[\"a\",\"dup\",\"b\",\"dup\",\"c\"]' WHERE \"id\" = 7",
        )
        .await;
        match conn
            .execute(
                "MERGE INTO \"dm_vecrm_probe\" AS t USING (SELECT src.\"k\" AS \"k\", '[' || NVL(LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' AS \"new_tags\" FROM (SELECT \"id\" AS \"k\", \"tags\" AS \"c\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 7) src, JSON_TABLE(src.\"c\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> ? GROUP BY src.\"k\") AS s ON (t.\"id\" = s.\"k\") WHEN MATCHED THEN UPDATE SET t.\"tags\" = s.\"new_tags\"",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "22h. MERGE + 派生表 + 绑定参数"
            ),
            Err(e) => println!(
                "FAIL {:<52} {e}",
                "22h. MERGE + 派生表 + 绑定参数"
            ),
        }
        probe(
            &mut conn,
            "22h2. 之后的行内容（应 [\"a\",\"b\",\"c\"]）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 7",
            (),
        )
        .await;

        // ---- 第 22 节补：`remove` 的两个边界 ----
        // 22g/22h 证明了 `MERGE` + 派生表可行，但 `WHERE jt."v" <> 值` 会让「元素全被删掉」的行
        // 在派生表里**没有分组** ⇒ MERGE 不碰它（该行语义上应变成 `[]`）。两种兜底：把过滤挪进
        // 聚合（`CASE WHEN … THEN … END`，`LISTAGG` 自动跳过 NULL）并用 `LEFT JOIN JSON_TABLE`
        // 保住行 —— 同时问达梦是否接受 `LEFT JOIN` 上的相关 `JSON_TABLE`。
        ddl(
            &mut conn,
            "22i0. 插入 id=8（[\"dup\",\"dup\"]）",
            "INSERT INTO \"dm_vecrm_probe\" (\"id\", \"tags\") VALUES (8, '[\"dup\",\"dup\"]')",
        )
        .await;
        match conn
            .execute(
                "MERGE INTO \"dm_vecrm_probe\" AS t USING (SELECT src.\"k\" AS \"k\", '[' || NVL(LISTAGG(JSON_QUOTE(jt.\"v\"), ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' AS \"new_tags\" FROM (SELECT \"id\" AS \"k\", \"tags\" AS \"c\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 8) src, JSON_TABLE(src.\"c\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt WHERE jt.\"v\" <> ? GROUP BY src.\"k\") AS s ON (t.\"id\" = s.\"k\") WHEN MATCHED THEN UPDATE SET t.\"tags\" = s.\"new_tags\"",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!("OK   {:<52} 影响 {count} 行", "22i. 全匹配：GROUP BY 版"),
            Err(e) => println!("FAIL {:<52} {e}", "22i. 全匹配：GROUP BY 版"),
        }
        probe(
            &mut conn,
            "22i2. 之后的行内容（语义应 []）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 8",
            (),
        )
        .await;
        ddl(
            &mut conn,
            "22j0. 复位 id=8",
            "UPDATE \"dm_vecrm_probe\" SET \"tags\" = '[\"dup\",\"dup\"]' WHERE \"id\" = 8",
        )
        .await;
        match conn
            .execute(
                "MERGE INTO \"dm_vecrm_probe\" AS t USING (SELECT src.\"k\" AS \"k\", '[' || NVL(LISTAGG(CASE WHEN jt.\"v\" <> ? THEN JSON_QUOTE(jt.\"v\") END, ',') WITHIN GROUP (ORDER BY jt.\"n\"), '') || ']' AS \"new_tags\" FROM (SELECT \"id\" AS \"k\", \"tags\" AS \"c\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 8) src LEFT JOIN JSON_TABLE(src.\"c\", '$[*]' COLUMNS (\"v\" VARCHAR(8188) PATH '$', \"n\" FOR ORDINALITY)) jt ON 1 = 1 GROUP BY src.\"k\") AS s ON (t.\"id\" = s.\"k\") WHEN MATCHED THEN UPDATE SET t.\"tags\" = s.\"new_tags\"",
                vec![DmValue::Text("dup".into())],
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<52} 影响 {count} 行",
                "22j. 全匹配：LEFT JOIN + CASE 版"
            ),
            Err(e) => println!("FAIL {:<52} {e}", "22j. 全匹配：LEFT JOIN + CASE 版"),
        }
        probe(
            &mut conn,
            "22j2. 之后的行内容（语义应 []）",
            "SELECT \"tags\" FROM \"dm_vecrm_probe\" WHERE \"id\" = 8",
            (),
        )
        .await;
    }

    // ---- 第 23 节：`capability.rs` 里全部 `(待实测)` 逐条定界（Round 24）----
    // 目的：把声明里**靠猜的那几项**变成实测事实。逐项对应 `src/capability.rs` 的注释：
    //   ① `sql_nulls_first_on_asc`（:269）—— 达梦升序把 NULL 排哪边、显式 `NULLS FIRST/LAST` 收不收；
    //   ② `max_identifier_length = Some(128)`（:192）—— 128 字节能用？129 会被拒吗？
    //   ③ `default_string_type = VarChar(255)`（:45）—— 255 字节能不能**建主键/索引**；
    //   ④ `varchar = Some(8188)`（:50）—— `VARCHAR(8188)` 能建索引、`VARCHAR(8189)` 建不出来？
    //   ⑤ `max_auto_increment_integer_width = Some(8)`（:188）—— `IDENTITY` 收 TINYINT/SMALLINT 吗；
    //   ⑥ `schema_mutations`（:135）—— `ALTER TABLE … MODIFY` 的确切形态与多属性是否一条改完；
    //   ⑦ `cte_with_update = false`（:143）—— `WITH … UPDATE` 在不在（对照组 `WITH … SELECT`）。
    if std::env::var("PROBE23").is_ok() {
        println!("\n=== 23. capability.rs 的 (待实测) 逐条定界 ===");

        // ① NULL 默认落点：看**首行**是 `Null` 还是 `Int(1)`。
        probe(
            &mut conn,
            "23a. ASC 首行（Null ⇒ NULL 在前；Int(1) ⇒ 在后）",
            "SELECT 1 AS \"v\" FROM DUAL UNION ALL SELECT NULL FROM DUAL ORDER BY 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "23b. DESC 首行（对照：应 Int(1)）",
            "SELECT 1 AS \"v\" FROM DUAL UNION ALL SELECT NULL FROM DUAL ORDER BY 1 DESC",
            (),
        )
        .await;
        probe(
            &mut conn,
            "23c. 显式 NULLS LAST 是否被接受",
            "SELECT 1 AS \"v\" FROM DUAL UNION ALL SELECT NULL FROM DUAL ORDER BY 1 NULLS LAST",
            (),
        )
        .await;
        probe(
            &mut conn,
            "23d. 显式 NULLS FIRST（DESC）是否被接受",
            "SELECT 1 AS \"v\" FROM DUAL UNION ALL SELECT NULL FROM DUAL ORDER BY 1 DESC NULLS FIRST",
            (),
        )
        .await;

        // ② 标识符长度：128 字节列名可用、129 字节应被拒。
        let c128 = "C".repeat(128);
        let c129 = "C".repeat(129);
        let _ = conn
            .execute("DROP TABLE \"dm_ident23_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "23e. 128 字节列名建表（应 OK）",
            &format!("CREATE TABLE \"dm_ident23_probe\" (\"id\" INT, \"{c128}\" INT)"),
        )
        .await;
        ddl(
            &mut conn,
            "23f. 129 字节列名 ADD COLUMN（应 FAIL）",
            &format!("ALTER TABLE \"dm_ident23_probe\" ADD \"{c129}\" INT"),
        )
        .await;

        // ③④ 字符串列的索引能力与 VARCHAR 上限。
        let _ = conn
            .execute("DROP TABLE \"dm_str23_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "23g. VARCHAR(255) 作主键（应 OK ⇒ default_string_type 可用作键）",
            "CREATE TABLE \"dm_str23_probe\" (\"id\" INT, \"s\" VARCHAR(255), PRIMARY KEY (\"s\"))",
        )
        .await;
        // 中文值进主键（V-21 明确要求「含中文值」）：LENGTH_IN_CHAR=0 ⇒ 按字节算，
        // 84 个汉字 = 252 字节应放得下，128 个汉字 = 384 字节应超限。
        let cjk84 = "中".repeat(84);
        let cjk128 = "中".repeat(128);
        ddl(
            &mut conn,
            "23g2. 中文主键 84 字（252 字节，应 OK）",
            &format!("INSERT INTO \"dm_str23_probe\" (\"id\", \"s\") VALUES (1, '{cjk84}')"),
        )
        .await;
        probe(
            &mut conn,
            "23g3. 按中文主键等值查回（应命中 1 行）",
            &format!("SELECT \"id\" FROM \"dm_str23_probe\" WHERE \"s\" = '{cjk84}'"),
            (),
        )
        .await;
        ddl(
            &mut conn,
            "23g4. 128 个汉字（384 字节 > 255，应 FAIL ⇒ 按字节）",
            &format!("INSERT INTO \"dm_str23_probe\" (\"id\", \"s\") VALUES (2, '{cjk128}')"),
        )
        .await;
        let _ = conn
            .execute("DROP TABLE \"dm_strmax23_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "23h. VARCHAR(8188) 列 + 独立索引（应 OK）",
            "CREATE TABLE \"dm_strmax23_probe\" (\"id\" INT PRIMARY KEY, \"s\" VARCHAR(8188))",
        )
        .await;
        ddl(
            &mut conn,
            "23i. 给 VARCHAR(8188) 列建索引（应 OK）",
            "CREATE INDEX \"dm_strmax23_idx\" ON \"dm_strmax23_probe\" (\"s\")",
        )
        .await;
        let _ = conn
            .execute("DROP TABLE \"dm_strmax23_bad\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "23j. VARCHAR(8189) 建表（实测 OK ⇒ 8188 不是语法上限）",
            "CREATE TABLE \"dm_strmax23_bad\" (\"id\" INT, \"s\" VARCHAR(8189))",
        )
        .await;
        // 23j 的结果推翻了「8188 是语法上限」的假设 ⇒ 继续找真上限与「声明的长度能不能真存下」。
        let _ = conn
            .execute("DROP TABLE \"dm_strbig23_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "23j2. VARCHAR(32767) 建表（看是否 OK）",
            "CREATE TABLE \"dm_strbig23_probe\" (\"id\" INT PRIMARY KEY, \"s\" VARCHAR(32767))",
        )
        .await;
        let _ = conn
            .execute("DROP TABLE \"dm_strbig23_bad\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "23j3. VARCHAR(32768) 建表（看是否 FAIL）",
            "CREATE TABLE \"dm_strbig23_bad\" (\"id\" INT, \"s\" VARCHAR(32768))",
        )
        .await;
        match conn
            .execute(
                "INSERT INTO \"dm_strbig23_probe\" (\"id\", \"s\") VALUES (1, ?)",
                vec![DmValue::Text("A".repeat(9000))],
            )
            .await
        {
            Ok(count) => println!(
                "OK   {:<44} 影响 {count} 行",
                "23j4. 往 VARCHAR(32767) 插 9000 字节"
            ),
            Err(e) => println!("FAIL {:<44} {e}", "23j4. 往 VARCHAR(32767) 插 9000 字节"),
        }
        probe(
            &mut conn,
            "23j5. 回读长度（9000 ⇒ 真存得下，不是声明大而已）",
            "SELECT LENGTH(\"s\") AS \"len\" FROM \"dm_strbig23_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;

        // ⑤ `IDENTITY` 在窄整数列上收不收。
        for (label, ty, table) in [
            ("23k. TINYINT + IDENTITY(1,1)", "TINYINT", "dm_identw23_a"),
            ("23l. SMALLINT + IDENTITY(1,1)", "SMALLINT", "dm_identw23_b"),
            ("23m. INT + IDENTITY(1,1)（基线）", "INT", "dm_identw23_c"),
            (
                "23n. BIGINT + IDENTITY(1,1)（基线）",
                "BIGINT",
                "dm_identw23_d",
            ),
        ] {
            let _ = conn
                .execute(&format!("DROP TABLE \"{table}\" CASCADE CONSTRAINTS"), ())
                .await;
            ddl(
                &mut conn,
                label,
                &format!("CREATE TABLE \"{table}\" (\"a\" {ty} IDENTITY(1, 1) PRIMARY KEY)"),
            )
            .await;
            ddl(
                &mut conn,
                &format!("{label} · 插一行（DEFAULT VALUES）"),
                &format!("INSERT INTO \"{table}\" DEFAULT VALUES"),
            )
            .await;
            probe(
                &mut conn,
                &format!("{label} · 回读"),
                &format!("SELECT \"a\" FROM \"{table}\""),
                (),
            )
            .await;
        }

        // ⑥ `ALTER TABLE … MODIFY` 的形态与多属性原子性。
        let _ = conn
            .execute("DROP TABLE \"dm_alt23_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "23o. 建表：VARCHAR(20) DEFAULT 'x' NOT NULL",
            "CREATE TABLE \"dm_alt23_probe\" (\"id\" INT PRIMARY KEY, \"s\" VARCHAR(20) DEFAULT 'x' NOT NULL)",
        )
        .await;
        ddl(
            &mut conn,
            "23p. MODIFY 只改类型（VARCHAR(20) → VARCHAR(50)）",
            "ALTER TABLE \"dm_alt23_probe\" MODIFY \"s\" VARCHAR(50)",
        )
        .await;
        ddl(
            &mut conn,
            "23q. MODIFY 类型 + 默认值一条写（多属性）",
            "ALTER TABLE \"dm_alt23_probe\" MODIFY \"s\" VARCHAR(60) DEFAULT 'y'",
        )
        .await;
        ddl(
            &mut conn,
            "23r. MODIFY 类型 + 默认值 + NULL（三属性一条）",
            "ALTER TABLE \"dm_alt23_probe\" MODIFY \"s\" VARCHAR(70) DEFAULT 'y' NULL",
        )
        .await;
        probe(
            &mut conn,
            "23s. USER_TAB_COLUMNS 里 s 的最终定义",
            "SELECT DATA_TYPE, DATA_LENGTH, NULLABLE, DATA_DEFAULT FROM USER_TAB_COLUMNS WHERE TABLE_NAME = 'dm_alt23_probe' AND COLUMN_NAME = 's'",
            (),
        )
        .await;

        // ⑦ `WITH … UPDATE`（`cte_with_update`）。
        ddl(
            &mut conn,
            "23t. WITH … UPDATE（看是否接受）",
            "WITH c AS (SELECT \"id\" FROM \"dm_alt23_probe\") UPDATE \"dm_alt23_probe\" SET \"s\" = 'z' WHERE \"id\" IN (SELECT \"id\" FROM c)",
        )
        .await;
        probe(
            &mut conn,
            "23u. 对照：WITH … SELECT（应 OK）",
            "WITH c AS (SELECT 1 AS \"v\" FROM DUAL) SELECT \"v\" FROM c",
            (),
        )
        .await;
        ddl(
            &mut conn,
            "23v. 对照：WITH … MERGE（另一形态）",
            "WITH c AS (SELECT \"id\" FROM \"dm_alt23_probe\") MERGE INTO \"dm_alt23_probe\" AS t USING (SELECT \"id\" FROM c) AS s ON (t.\"id\" = s.\"id\") WHEN MATCHED THEN UPDATE SET t.\"s\" = 'w'",
        )
        .await;
        // 上游字段文档：`cte_with_update` = "SQL: supports update statements in CTE queries"。
        // 两种读法都要问：① `WITH … UPDATE`（顶层 UPDATE，23t 已 OK）；
        // ② PG 式的**数据修改型 CTE**（`WITH x AS (UPDATE … RETURNING …) SELECT …`）——
        // 达梦本来就没有 `RETURNING`（§9.4 第 6 条），所以这一读法预期不成立。
        ddl(
            &mut conn,
            "23w. 数据修改型 CTE（WITH x AS (UPDATE … RETURNING …) SELECT …）",
            "WITH x AS (UPDATE \"dm_alt23_probe\" SET \"s\" = 'q' WHERE \"id\" = 1 RETURNING \"id\") SELECT \"id\" FROM x",
        )
        .await;
        ddl(
            &mut conn,
            "23x. CTE 里放 UPDATE 但不带 RETURNING",
            "WITH x AS (UPDATE \"dm_alt23_probe\" SET \"s\" = 'q' WHERE \"id\" = 1) SELECT 1 FROM x",
        )
        .await;

        for table in [
            "dm_ident23_probe",
            "dm_str23_probe",
            "dm_strmax23_probe",
            "dm_strbig23_probe",
            "dm_identw23_a",
            "dm_identw23_b",
            "dm_identw23_c",
            "dm_identw23_d",
            "dm_alt23_probe",
        ] {
            let _ = conn
                .execute(&format!("DROP TABLE \"{table}\" CASCADE CONSTRAINTS"), ())
                .await;
        }
    }

    // ---- 第 24 节：BigDecimal × `DECIMAL(38,20)` 的可行路径（Round 25，为 `bigdecimal_implemented` 定界）----
    // 目的：套件 `type_bigdecimal.rs::ty_bigdecimal_as_numeric_fixed_precision`
    // （`requires(and(native_decimal, bigdecimal_implemented))`）用 `numeric(38, 20)` 列 + **38 位有效数字**
    // （`123456789012345678.12345678901234567890`），而 `DmValue::Decimal` 里的 `rust_decimal`
    // 最多约 28~29 位有效数字 ⇒ 必须确认：① 38 位值能不能经**文本参数**存进 `DECIMAL(38,20)`；
    // ② 回读时我们的栈给的是哪个 `DmValue` 变体、是否逐字符精确（有无截断/舍入/补零）；
    // ③ 21 位以内的值回读是不是 `Decimal`（对照）；④ 服务端存下来后 `CAST(v AS VARCHAR)` 的形态。
    if std::env::var("PROBE24").is_ok() {
        println!("\n=== 24. BigDecimal × DECIMAL(38,20) 的可行路径 ===");

        const BIG: &str = "123456789012345678.12345678901234567890"; // 38 位有效数字
        const SMALL: &str = "3.14159265358979323846"; // 21 位（rust_decimal 装得下）

        let _ = conn
            .execute("DROP TABLE \"dm_bd24_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "24a. 建表 DECIMAL(38,20)",
            "CREATE TABLE \"dm_bd24_probe\" (\"id\" INT PRIMARY KEY, \"v\" DECIMAL(38,20))",
        )
        .await;

        // ① 文本参数 → DECIMAL(38,20)。
        probe(
            &mut conn,
            "24b. 文本参数插 38 位值",
            "INSERT INTO \"dm_bd24_probe\" (\"id\", \"v\") VALUES (1, ?)",
            vec![DmValue::Text(BIG.to_string())],
        )
        .await;
        probe(
            &mut conn,
            "24c. 文本参数插 21 位值（对照）",
            "INSERT INTO \"dm_bd24_probe\" (\"id\", \"v\") VALUES (2, ?)",
            vec![DmValue::Text(SMALL.to_string())],
        )
        .await;

        // ② 回读：看 `DmValue` 变体与精确性。
        probe(
            &mut conn,
            "24d. 回读 38 位值（看变体/是否截断）",
            "SELECT \"v\" FROM \"dm_bd24_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "24e. 回读 21 位值（对照，预期 Decimal）",
            "SELECT \"v\" FROM \"dm_bd24_probe\" WHERE \"id\" = 2",
            (),
        )
        .await;
        // ③ 服务端文本形态（尾零/标度）——首行即 38 位那一行。
        probe(
            &mut conn,
            "24f. 服务端 CAST(v AS VARCHAR(64)) 的形态",
            "SELECT \"id\", CAST(\"v\" AS VARCHAR(64)) AS \"s\", LENGTH(CAST(\"v\" AS VARCHAR(64))) AS \"n\" FROM \"dm_bd24_probe\" ORDER BY \"id\"",
            (),
        )
        .await;
        // ④ 引擎可能的 cast 形态：文本参数在 SELECT 里被转成 DECIMAL。
        probe(
            &mut conn,
            "24g. SELECT CAST(? AS DECIMAL(38,20)) 回读",
            "SELECT CAST(? AS DECIMAL(38,20)) AS \"v\" FROM DUAL",
            vec![DmValue::Text(BIG.to_string())],
        )
        .await;
        probe(
            &mut conn,
            "24h. 字面量 38 位（对照）",
            &format!("SELECT {BIG} AS \"v\" FROM DUAL"),
            (),
        )
        .await;
        // ⑤ 数值比较语义（若将来走文本存储，这条是隐患检查；DECIMAL 列应天然按数值比较）。
        probe(
            &mut conn,
            "24i. DECIMAL 上按值比较（9 < 10 应为 1）",
            "SELECT CASE WHEN CAST('9' AS DECIMAL(38,20)) < CAST('10' AS DECIMAL(38,20)) THEN 1 ELSE 0 END AS \"v\" FROM DUAL",
            (),
        )
        .await;

        // ⑥ 无参 `DECIMAL` 的语义（V-25）：达梦把它当「任意精度」还是 `DECIMAL(18,0)`（后者静默截断小数位）？
        let _ = conn
            .execute("DROP TABLE \"dm_dec25_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "24j. 建表无参 DECIMAL",
            "CREATE TABLE \"dm_dec25_probe\" (\"id\" INT PRIMARY KEY, \"d\" DECIMAL)",
        )
        .await;
        probe(
            &mut conn,
            "24j2. 插 1.2345（文本参数）",
            "INSERT INTO \"dm_dec25_probe\" (\"id\", \"d\") VALUES (1, ?)",
            vec![DmValue::Text("1.2345".to_string())],
        )
        .await;
        probe(
            &mut conn,
            "24k. 回读无参 DECIMAL 列",
            "SELECT \"d\" FROM \"dm_dec25_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "24l. USER_TAB_COLUMNS 看列型/精度/标度（首行）",
            "SELECT TABLE_NAME, DATA_TYPE, DATA_PRECISION, DATA_SCALE FROM USER_TAB_COLUMNS WHERE TABLE_NAME = 'dm_dec25_probe'",
            (),
        )
        .await;
        // 只看 `"d"` 那一列的元数据（24l 的首行是 `"id"` 列）。
        probe(
            &mut conn,
            "24l2. 只看 d 列的类型/精度/标度",
            "SELECT COLUMN_NAME, DATA_TYPE, DATA_PRECISION, DATA_SCALE FROM USER_TAB_COLUMNS WHERE TABLE_NAME = 'dm_dec25_probe' AND COLUMN_NAME = 'd'",
            (),
        )
        .await;
        // 无参 `DECIMAL` 到底装得下多少位：插 38 位值看报错/截断。
        probe(
            &mut conn,
            "24m. 无参 DECIMAL 插 38 位值",
            "INSERT INTO \"dm_dec25_probe\" (\"id\", \"d\") VALUES (2, ?)",
            vec![DmValue::Text(BIG.to_string())],
        )
        .await;
        probe(
            &mut conn,
            "24m2. 回读 38 位值（看是否截断/报错）",
            "SELECT \"d\" FROM \"dm_dec25_probe\" WHERE \"id\" = 2",
            (),
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_bd24_probe\" CASCADE CONSTRAINTS", ())
            .await;
        let _ = conn
            .execute("DROP TABLE \"dm_dec25_probe\" CASCADE CONSTRAINTS", ())
            .await;
    }

    // ------------------------------------------------------------------
    // 第 25 节：`#[document]` 的路径提取（`document_collections` 的前置）
    // ------------------------------------------------------------------
    //
    // 目的：达梦的 `JSON_VALUE` / `JSON_QUERY` / `JSON_EXTRACT` 在 **CLOB 文档**上的
    // 可用性、叶子形态（带不带引号）、缺失/类型不匹配的错误语义，以及「提取出来的
    // 叶子能否直接与绑定参数比较」——这决定 fork `serializer/expr.rs` 的
    // `serialize_json_extract` 达梦臂怎么写（当前是
    // `unimplemented!("dameng: document path extraction")`，也是 fork 里最后
    // 一个 `unimplemented!`）。
    if std::env::var("PROBE25").is_ok() {
        const DOC: &str = r#"{"s":"hello","n":42,"f":1.5,"b":true,"nil":null,"o":{"k":"deep"},"a":[1,2,3],"t":"2024-01-02 03:04:05.123456"}"#;
        let _ = conn
            .execute("DROP TABLE \"dm_doc25_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "25a. 建文档表（CLOB）",
            "CREATE TABLE \"dm_doc25_probe\" (\"id\" INT PRIMARY KEY, \"d\" CLOB)",
        )
        .await;
        probe(
            &mut conn,
            "25b. 插文档文本（绑定参数）",
            "INSERT INTO \"dm_doc25_probe\" (\"id\", \"d\") VALUES (1, ?)",
            vec![DmValue::Text(DOC.to_string())],
        )
        .await;

        let read = |_label: &'static str, expr: &'static str| {
            format!("SELECT {expr} AS \"v\" FROM \"dm_doc25_probe\" WHERE \"id\" = 1")
        };
        let count = |expr: &'static str| {
            format!("SELECT COUNT(*) AS \"n\" FROM \"dm_doc25_probe\" WHERE {expr}")
        };

        // ① 函数存在性与叶子形态。
        probe(
            &mut conn,
            "25c. JSON_VALUE(d, '$.s') 裸调用",
            &read("25c", "JSON_VALUE(\"d\", '$.s')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25c2. JSON_VALUE(d, '$.s' RETURNING VARCHAR(64))",
            &read("25c2", "JSON_VALUE(\"d\", '$.s' RETURNING VARCHAR(64))"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25d. JSON_QUERY(d, '$.s')",
            &read("25d", "JSON_QUERY(\"d\", '$.s')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25e. JSON_VALUE(d, '$.n') 数字叶子",
            &read("25e", "JSON_VALUE(\"d\", '$.n')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25f. JSON_VALUE(d, '$.f') 小数叶子",
            &read("25f", "JSON_VALUE(\"d\", '$.f')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25g. JSON_VALUE(d, '$.b') 布尔叶子",
            &read("25g", "JSON_VALUE(\"d\", '$.b')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25h. JSON_VALUE(d, '$.o.k') 嵌套叶子",
            &read("25h", "JSON_VALUE(\"d\", '$.o.k')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25i. JSON_VALUE(d, '$.missing') 路径不存在",
            &read("25i", "JSON_VALUE(\"d\", '$.missing')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25j. JSON_VALUE(d, '$.nil') 显式 null",
            &read("25j", "JSON_VALUE(\"d\", '$.nil')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25k. JSON_VALUE(d, '$.o') 对象叶子（标量函数上的类型不匹配）",
            &read("25k", "JSON_VALUE(\"d\", '$.o')"),
            (),
        )
        .await;

        // ② MySQL 那一族函数在不在（决定能否照抄 MySQL 臂）。
        probe(
            &mut conn,
            "25l. JSON_EXTRACT(d, '$.s')",
            &read("25l", "JSON_EXTRACT(\"d\", '$.s')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25m. JSON_UNQUOTE(JSON_EXTRACT(d, '$.s'))",
            &read("25m", "JSON_UNQUOTE(JSON_EXTRACT(\"d\", '$.s'))"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25n. JSON_QUERY(d, '$.o') 对象",
            &read("25n", "JSON_QUERY(\"d\", '$.o')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25o. JSON_QUERY(d, '$.a') 数组",
            &read("25o", "JSON_QUERY(\"d\", '$.a')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25p. JSON_VALUE(d, '$.a[0]') 数组下标",
            &read("25p", "JSON_VALUE(\"d\", '$.a[0]')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25q. JSON_LENGTH(d, '$.a') 带路径",
            &read("25q", "JSON_LENGTH(\"d\", '$.a')"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25r. JSON_VALUE(?, '$.s')（文档是绑定参数）",
            "SELECT JSON_VALUE(?, '$.s') AS \"v\" FROM DUAL",
            vec![DmValue::Text(DOC.to_string())],
        )
        .await;
        probe(
            &mut conn,
            "25s. 路径写法 '$.\"s\"'",
            &read("25s", "JSON_VALUE(\"d\", '$.\"s\"')"),
            (),
        )
        .await;

        // ③ 叶子与绑定参数/字面量的比较（引擎生成的过滤谓词形状）。
        probe(
            &mut conn,
            "25t. 字符串叶子 = 字面量",
            &count("JSON_VALUE(\"d\", '$.s') = 'hello'"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25u. 字符串叶子 = 绑定文本参数",
            &count("JSON_VALUE(\"d\", '$.s') = ?"),
            vec![DmValue::Text("hello".to_string())],
        )
        .await;
        probe(
            &mut conn,
            "25v. 数字叶子 = 绑定 BIGINT（不加 CAST）",
            &count("JSON_VALUE(\"d\", '$.n') = ?"),
            vec![DmValue::BigInt(42)],
        )
        .await;
        probe(
            &mut conn,
            "25w. CAST(数字叶子 AS BIGINT) = 绑定 BIGINT",
            &count("CAST(JSON_VALUE(\"d\", '$.n') AS BIGINT) = ?"),
            vec![DmValue::BigInt(42)],
        )
        .await;
        probe(
            &mut conn,
            "25x. 布尔叶子 = 绑定 BOOL",
            &count("JSON_VALUE(\"d\", '$.b') = ?"),
            vec![DmValue::Bool(true)],
        )
        .await;
        probe(
            &mut conn,
            "25y. 布尔叶子 = 绑定文本 'true'",
            &count("JSON_VALUE(\"d\", '$.b') = ?"),
            vec![DmValue::Text("true".to_string())],
        )
        .await;
        probe(
            &mut conn,
            "25y2. CAST(布尔叶子 AS VARCHAR(8)) = 文本 'true'",
            &count("CAST(JSON_VALUE(\"d\", '$.b') AS VARCHAR(8)) = ?"),
            vec![DmValue::Text("true".to_string())],
        )
        .await;
        probe(
            &mut conn,
            "25z. CAST(时间叶子 AS TIMESTAMP(6)) = 绑定文本",
            &count("CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(6)) = ?"),
            vec![DmValue::Text("2024-01-02 03:04:05.123456".to_string())],
        )
        .await;
        probe(
            &mut conn,
            "25z2. NULL ON ERROR 修饰符",
            &read("25z2", "JSON_VALUE(\"d\", '$.n' NULL ON ERROR)"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "25z3. RETURNING 数字串",
            &read("25z3", "JSON_VALUE(\"d\", '$.n' RETURNING VARCHAR(32))"),
            (),
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_doc25_probe\" CASCADE CONSTRAINTS", ())
            .await;
    }

    // ------------------------------------------------------------------
    // 第 26 节：文档叶子与**绑定参数**的比较（补全第 25 节的第二个缺口）
    // ------------------------------------------------------------------
    //
    // 第 25 节已经证明：`JSON_VALUE(d, '$.a.b')` 在 `CLOB` 文档上可用、嵌套路径与数组
    // 下标都吃、标量叶子返回**不带引号的文本**（`'$.n'` → `"42"`、`'$.b'` → `"true"`）、
    // 容器叶子返回 NULL（要用 `JSON_QUERY`，它给 `'{"k":"deep"}'`/`'[1,2,3]'`）。
    // 它同时暴露了一个**必须补测**的缺口：布尔叶子 `= 绑定 BOOL` **不匹配**（25x 得 0），
    // 只有 `= 绑定文本 'true'` 才匹配（25y）——而引擎给布尔过滤绑的是 `Value::Bool`。
    // 本节把这几种叶子的「与绑定参数比较」形状一次性钉死，并顺带看清
    // `DmValue::Bool`/`Bytes` 到底以什么形态上车（26a/26b）。
    if std::env::var("PROBE26").is_ok() {
        const DOC26: &str = r#"{"s":"hello","n":42,"f":1.5,"b":true,"u":"550e8400-e29b-41d4-a716-446655440000","dec":1.23,"t":"2024-01-02 03:04:05.123456"}"#;
        let count =
            |pred: &str| format!("SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc26_probe\" WHERE {pred}");
        let read = |alias: &str, expr: &str| {
            format!("SELECT {expr} AS \"{alias}\" FROM \"dm_doc26_probe\"")
        };

        let _ = conn
            .execute("DROP TABLE \"dm_doc26_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "26. 建文档表（CLOB）",
            "CREATE TABLE \"dm_doc26_probe\" (\"id\" INT PRIMARY KEY, \"d\" CLOB)",
        )
        .await;
        probe(
            &mut conn,
            "26. 插文档（绑定参数）",
            "INSERT INTO \"dm_doc26_probe\" (\"id\", \"d\") VALUES (1, ?)",
            vec![DmValue::Text(DOC26.to_string())],
        )
        .await;

        // ① 绑定值本身的形态（决定布尔/二进制叶子该拿什么比）。
        probe(
            &mut conn,
            "26a. SELECT 绑定 BOOL",
            "SELECT ? AS \"v\" FROM DUAL",
            vec![DmValue::Bool(true)],
        )
        .await;
        probe(
            &mut conn,
            "26b. SELECT 绑定 BYTES(16)",
            "SELECT ? AS \"v\" FROM DUAL",
            vec![DmValue::Bytes(vec![
                0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
                0x00, 0x00,
            ])],
        )
        .await;

        // ② 布尔叶子：三种可比形状（25x 已证明裸比较不成立）。
        probe(
            &mut conn,
            "26c. CASE 把布尔叶子折成 1/0 = 绑定 BOOL",
            &count("CASE WHEN JSON_VALUE(\"d\", '$.b') = 'true' THEN 1 ELSE 0 END = ?"),
            vec![DmValue::Bool(true)],
        )
        .await;
        probe(
            &mut conn,
            "26d. 布尔叶子 = CAST(绑定 BOOL AS VARCHAR(8))",
            &count("JSON_VALUE(\"d\", '$.b') = CAST(? AS VARCHAR(8))"),
            vec![DmValue::Bool(true)],
        )
        .await;
        probe(
            &mut conn,
            "26e. CAST(布尔叶子 AS BIT) = 绑定 BOOL",
            &count("CAST(JSON_VALUE(\"d\", '$.b') AS BIT) = ?"),
            vec![DmValue::Bool(true)],
        )
        .await;
        probe(
            &mut conn,
            "26e2. CAST(布尔叶子 AS INT) = 绑定 BOOL",
            &count("CAST(JSON_VALUE(\"d\", '$.b') AS INT) = ?"),
            vec![DmValue::Bool(true)],
        )
        .await;

        // ③ 浮点叶子。
        probe(
            &mut conn,
            "26f. 小数叶子 = 绑定 DOUBLE（裸）",
            &count("JSON_VALUE(\"d\", '$.f') = ?"),
            vec![DmValue::Double(1.5)],
        )
        .await;
        probe(
            &mut conn,
            "26g. CAST(小数叶子 AS DOUBLE) = 绑定 DOUBLE",
            &count("CAST(JSON_VALUE(\"d\", '$.f') AS DOUBLE) = ?"),
            vec![DmValue::Double(1.5)],
        )
        .await;

        // ④ UUID 叶子：驱动把 `Uuid` 编成 16 字节 BLOB，而 JSON 里是 36 字符文本。
        probe(
            &mut conn,
            "26h. UUID 叶子 = 绑定 BYTES(16)",
            &count("JSON_VALUE(\"d\", '$.u') = ?"),
            vec![DmValue::Bytes(vec![
                0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
                0x00, 0x00,
            ])],
        )
        .await;
        probe(
            &mut conn,
            "26i. UUID 叶子 = 绑定文本（36 字符）",
            &count("JSON_VALUE(\"d\", '$.u') = ?"),
            vec![DmValue::Text(
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
            )],
        )
        .await;
        probe(
            &mut conn,
            "26i2. UUID 叶子：把 BLOB 绑参里外都转文本后比较",
            &count("JSON_VALUE(\"d\", '$.u') = RAWTOHEX(?)"),
            vec![DmValue::Bytes(vec![
                0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
                0x00, 0x00,
            ])],
        )
        .await;

        // ⑤ 定点/十进制叶子。
        probe(
            &mut conn,
            "26j. CAST(十进制叶子 AS DECIMAL(38,20)) = 绑定文本",
            &count("CAST(JSON_VALUE(\"d\", '$.dec') AS DECIMAL(38,20)) = ?"),
            vec![DmValue::Text("1.23".to_string())],
        )
        .await;
        probe(
            &mut conn,
            "26k. 十进制叶子 = 绑定 DECIMAL",
            &count("CAST(JSON_VALUE(\"d\", '$.dec') AS DECIMAL(38,20)) = ?"),
            vec![DmValue::Decimal(rust_decimal::Decimal::new(123, 2))],
        )
        .await;

        // ⑥ 时间叶子：绑定真 TIMESTAMP（25z 只试过文本参数）。
        probe(
            &mut conn,
            "26l. CAST(时间叶子 AS TIMESTAMP(6)) = 绑定 TIMESTAMP",
            &count("CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(6)) = ?"),
            vec![DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_micro_opt(3, 4, 5, 123_456)
                    .unwrap(),
            )],
        )
        .await;

        // ⑦ 整数叶子的投影形态（引擎若投影叶子，回来的是文本还是整数）。
        probe(
            &mut conn,
            "26m. 投影 CAST(数字叶子 AS BIGINT)",
            &read("26m", "CAST(JSON_VALUE(\"d\", '$.n') AS BIGINT)"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "26n. 投影裸 JSON_VALUE（数字叶子）",
            &read("26n", "JSON_VALUE(\"d\", '$.n')"),
            (),
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_doc26_probe\" CASCADE CONSTRAINTS", ())
            .await;
    }

    // ------------------------------------------------------------------
    // 第 27 节：第 26 节剩下的三个尾巴（时间精度、定点裸比较、UUID 的十六进制路）
    // ------------------------------------------------------------------
    //
    // 26l 暴露的矛盾：`CAST(叶子 AS TIMESTAMP(6)) = 绑定 TIMESTAMP` **不匹配**，而 25z
    // 的「= 绑定文本」匹配 ⇒ 要么 cast 把微秒丢了、要么参数上车的形态与 cast 结果不同。
    // 26h 暴露的错误：UUID 叶子（36 字符文本）与驱动绑的 16 字节 BLOB 比较，达梦直接报
    // `-6147 无效的十六进制数字`（不是「不匹配」）⇒ 得给左边换个形态（`HEXTORAW`）。
    // 26k 只证明了「cast 到 DECIMAL(38,20) 后 = 绑定 DECIMAL」匹配，裸比较还没试。
    if std::env::var("PROBE27").is_ok() {
        let count =
            |pred: &str| format!("SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc26_probe\" WHERE {pred}");
        let read = |alias: &str, expr: &str| {
            format!("SELECT {expr} AS \"{alias}\" FROM \"dm_doc26_probe\"")
        };
        const TS: &str = "2024-01-02 03:04:05.123456";
        // 第 26 节末尾会 drop 这张表 ⇒ 本节自带建表+插入（两节可单独跑，也可一起跑）。
        const DOC27: &str = r#"{"s":"hello","n":42,"f":1.5,"b":true,"u":"550e8400-e29b-41d4-a716-446655440000","dec":1.23,"t":"2024-01-02 03:04:05.123456"}"#;
        let _ = conn
            .execute("DROP TABLE \"dm_doc26_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "27. 建文档表（CLOB）",
            "CREATE TABLE \"dm_doc26_probe\" (\"id\" INT PRIMARY KEY, \"d\" CLOB)",
        )
        .await;
        probe(
            &mut conn,
            "27. 插文档（绑定参数）",
            "INSERT INTO \"dm_doc26_probe\" (\"id\", \"d\") VALUES (1, ?)",
            vec![DmValue::Text(DOC27.to_string())],
        )
        .await;
        let ts_param = || {
            vec![DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_micro_opt(3, 4, 5, 123_456)
                    .unwrap(),
            )]
        };

        // ① 时间：cast 到底保不保住微秒；参数上车后长什么样。
        probe(
            &mut conn,
            "27a. CAST(文本 AS TIMESTAMP(6)) 的投影（看微秒）",
            &format!("SELECT CAST('{TS}' AS TIMESTAMP(6)) AS \"v\" FROM DUAL"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "27a2. CAST(文本 AS TIMESTAMP(9)) 的投影",
            &format!("SELECT CAST('{TS}' AS TIMESTAMP(9)) AS \"v\" FROM DUAL"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "27b. 投影 CAST(叶子 AS TIMESTAMP(6))",
            &read("27b", "CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(6))"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "27b2. 参数上车后的文本形态 TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6')",
            "SELECT TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6') AS \"v\" FROM DUAL",
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "27c. cast 叶子 = CAST(? AS TIMESTAMP(6))",
            &count("CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(6)) = CAST(? AS TIMESTAMP(6))"),
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "27d. cast 叶子 = TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6')",
            &count("CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(6)) = TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6')"),
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "27e. JSON_VALUE(叶子) = TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6')",
            &count("JSON_VALUE(\"d\", '$.t') = TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6')"),
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "27e2. cast 叶子（TIMESTAMP(9)）= 绑定 TIMESTAMP",
            &count("CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(9)) = ?"),
            ts_param(),
        )
        .await;

        // ② 定点：裸比较（不走 cast）。
        probe(
            &mut conn,
            "27f. 十进制叶子 = 绑定 DECIMAL（裸）",
            &count("JSON_VALUE(\"d\", '$.dec') = ?"),
            vec![DmValue::Decimal(rust_decimal::Decimal::new(123, 2))],
        )
        .await;

        // ③ UUID：把 36 字符文本换成 16 字节 RAW 再比（驱动绑的就是 16 字节 BLOB）。
        probe(
            &mut conn,
            "27g. HEXTORAW 投影（工具可用性）",
            "SELECT HEXTORAW('550E8400E29B41D4A716446655440000') AS \"v\" FROM DUAL",
            (),
        )
        .await;
        probe(
            &mut conn,
            "27h. RAW 化的 UUID 叶子 = 绑定 BYTES(16)",
            &count("HEXTORAW(REPLACE(JSON_VALUE(\"d\", '$.u'), '-', '')) = ?"),
            vec![DmValue::Bytes(vec![
                0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
                0x00, 0x00,
            ])],
        )
        .await;
        probe(
            &mut conn,
            "27h2. RAW 化前后对照：投影 HEXTORAW(REPLACE(...))",
            &read(
                "27h2",
                "HEXTORAW(REPLACE(JSON_VALUE(\"d\", '$.u'), '-', ''))",
            ),
            (),
        )
        .await;

        println!("（第 27 节复用第 26 节的 dm_doc26_probe，不额外清理）");
    }

    // ------------------------------------------------------------------
    // 第 28 节：时间参数的**微秒**（`column_size = 0` 的嫌疑）
    // ------------------------------------------------------------------
    //
    // 第 27 节的两个结果对不上：`CAST(文本 AS TIMESTAMP(6))` 投影回 `.123456`（微秒没丢），
    // 但 `TO_CHAR(?, '…FF6')` 里**绑定参数**只有 `.000000`，于是
    // `CAST(叶子 AS TIMESTAMP(6)) = ?` 恒不匹配（27c/27e2）。嫌疑落在
    // `crates/dm-odbc/src/param.rs`：`DmValue::Timestamp` 回报 `column_size = 0`、
    // `decimal_digits = 0`，而 ODBC 规范里日期时间型的 `ColumnSize` 是**字符串形态的
    // 长度**、`DecimalDigits` 是小数秒位数（整数/BIGINT 那边都老老实实报了 19）。
    // 本节先用**带 TIMESTAMP(6) 列**的往返把「写进去有没有微秒」「比较认不认微秒」分开量。
    if std::env::var("PROBE28").is_ok() {
        const TS28: &str = "2024-01-02 03:04:05.123456";
        let ts_param = || {
            vec![DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_micro_opt(3, 4, 5, 123_456)
                    .unwrap(),
            )]
        };
        let time_param = || {
            vec![DmValue::Time(
                chrono::NaiveTime::from_hms_micro_opt(1, 2, 3, 4).unwrap(),
            )]
        };

        let _ = conn
            .execute("DROP TABLE \"dm_ts28_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "28a. 建时间表（TIMESTAMP(6)/TIME(6)）",
            "CREATE TABLE \"dm_ts28_probe\" (\"id\" INT PRIMARY KEY, \"ts\" TIMESTAMP(6), \"tm\" TIME(6))",
        )
        .await;
        probe(
            &mut conn,
            "28b. 插绑定 TIMESTAMP/TIME 参数",
            "INSERT INTO \"dm_ts28_probe\" (\"id\", \"ts\", \"tm\") VALUES (1, ?, ?)",
            vec![
                DmValue::Timestamp(
                    chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                        .unwrap()
                        .and_hms_micro_opt(3, 4, 5, 123_456)
                        .unwrap(),
                ),
                DmValue::Time(chrono::NaiveTime::from_hms_micro_opt(1, 2, 3, 4).unwrap()),
            ],
        )
        .await;
        probe(
            &mut conn,
            "28c. 回读列（写进去有没有微秒）",
            "SELECT \"ts\", \"tm\" FROM \"dm_ts28_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "28d. TO_CHAR(列, '…FF6') 看服务端存的文本",
            "SELECT TO_CHAR(\"ts\", 'YYYY-MM-DD HH24:MI:SS.FF6') AS \"a\", TO_CHAR(\"tm\", 'HH24:MI:SS.FF6') AS \"b\" FROM \"dm_ts28_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "28e. 列 = 同一绑定参数（比较认不认微秒）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts28_probe\" WHERE \"ts\" = ?",
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "28f. 列 = 字面量 TIMESTAMP(6)",
            &format!("SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts28_probe\" WHERE \"ts\" = CAST('{TS28}' AS TIMESTAMP(6))"),
            (),
        )
        .await;
        probe(
            &mut conn,
            "28g. TIME 列 = 同一绑定参数",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts28_probe\" WHERE \"tm\" = ?",
            time_param(),
        )
        .await;
        probe(
            &mut conn,
            "28g2. TIME 列 = 字面量 TIME(6)",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts28_probe\" WHERE \"tm\" = CAST('01:02:03.000004' AS TIME(6))",
            (),
        )
        .await;
        probe(
            &mut conn,
            "28h. TO_CHAR(绑定参数, '…FF6')（27b2 的复现）",
            "SELECT TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6') AS \"v\" FROM DUAL",
            ts_param(),
        )
        .await;

        // 文档叶子：同一张 TIMESTAMP(6) 语义的 cast，配绑定参数。
        const DOC28: &str = r#"{"t":"2024-01-02 03:04:05.123456"}"#;
        let _ = conn
            .execute("DROP TABLE \"dm_doc28_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "28i. 建文档表（CLOB）",
            "CREATE TABLE \"dm_doc28_probe\" (\"id\" INT PRIMARY KEY, \"d\" CLOB)",
        )
        .await;
        probe(
            &mut conn,
            "28i2. 插文档",
            "INSERT INTO \"dm_doc28_probe\" (\"id\", \"d\") VALUES (1, ?)",
            vec![DmValue::Text(DOC28.to_string())],
        )
        .await;
        probe(
            &mut conn,
            "28j. CAST(文档时间叶子 AS TIMESTAMP(6)) = 绑定参数",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc28_probe\" WHERE CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(6)) = ?",
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "28k. Cast 两端都显式（左 cast、右 cast 参数）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc28_probe\" WHERE CAST(JSON_VALUE(\"d\", '$.t') AS TIMESTAMP(6)) = CAST(? AS TIMESTAMP(6))",
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "28l. 文本叶子 = 绑定参数（不经 cast）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc28_probe\" WHERE JSON_VALUE(\"d\", '$.t') = ?",
            ts_param(),
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_ts28_probe\" CASCADE CONSTRAINTS", ())
            .await;
        let _ = conn
            .execute("DROP TABLE \"dm_doc28_probe\" CASCADE CONSTRAINTS", ())
            .await;
    }

    if std::env::var("PROBE29").is_ok() {
        // 第 28 节的文档时间叶子用的是**服务端原生形态**（`2024-01-02 03:04:05.123456`），
        // 而驱动真正写进 `#[document]` 列的是 `toasty_sql::json` 的存储文本：
        // jiff `Timestamp` 走 RFC 3339（`…T…Z`）、`DateTime` 不带 `Z`、`Date`/`Time`
        // 各自是 `YYYY-MM-DD` / `HH:MM:SS.FFFFFF`。本节按**真实文本**复现套件里
        // `type_document::struct_embed_filter_temporal` 的 `[22007/-6118] 非法的时间
        // 日期类型数据`，并逐条验证候选渲染。
        let ts_param = || {
            vec![DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_micro_opt(3, 4, 5, 123_456)
                    .unwrap(),
            )]
        };
        let time_param = || {
            vec![DmValue::Time(
                chrono::NaiveTime::from_hms_micro_opt(3, 4, 5, 4).unwrap(),
            )]
        };
        let date_param = || {
            vec![DmValue::Date(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
            )]
        };

        const DOC29: &str = r#"{"ts":"2024-01-02T03:04:05.123456Z","dt":"2024-01-02T03:04:05.123456","d":"2024-01-02","tm":"03:04:05.000004"}"#;
        let _ = conn
            .execute("DROP TABLE \"dm_doc29_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "29a. 建文档表（CLOB）",
            "CREATE TABLE \"dm_doc29_probe\" (\"id\" INT PRIMARY KEY, \"d\" CLOB)",
        )
        .await;
        probe(
            &mut conn,
            "29b. 插文档（codec 的存储文本形态）",
            "INSERT INTO \"dm_doc29_probe\" (\"id\", \"d\") VALUES (1, ?)",
            vec![DmValue::Text(DOC29.to_string())],
        )
        .await;
        probe(
            &mut conn,
            "29c. 四个时间叶子的原文",
            "SELECT JSON_VALUE(\"d\", '$.ts') AS \"a\", JSON_VALUE(\"d\", '$.dt') AS \"b\", JSON_VALUE(\"d\", '$.d') AS \"c\", JSON_VALUE(\"d\", '$.tm') AS \"e\" FROM \"dm_doc29_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "29d. CAST(RFC 3339 带 Z AS TIMESTAMP(6))（复现套件失败）",
            "SELECT CAST(JSON_VALUE(\"d\", '$.ts') AS TIMESTAMP(6)) AS \"v\" FROM \"dm_doc29_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "29e. 候选：REPLACE(T→空格,Z→空) 后 CAST = 绑定参数",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc29_probe\" WHERE CAST(REPLACE(REPLACE(JSON_VALUE(\"d\", '$.ts'), 'T', ' '), 'Z', '') AS TIMESTAMP(6)) = ?",
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "29e2. 候选渲染的投影值（微秒在不在）",
            "SELECT CAST(REPLACE(REPLACE(JSON_VALUE(\"d\", '$.ts'), 'T', ' '), 'Z', '') AS TIMESTAMP(6)) AS \"v\" FROM \"dm_doc29_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "29f. TO_TIMESTAMP(候选, 'YYYY-MM-DD HH24:MI:SS.FF6') = 绑定参数",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc29_probe\" WHERE TO_TIMESTAMP(REPLACE(REPLACE(JSON_VALUE(\"d\", '$.ts'), 'T', ' '), 'Z', ''), 'YYYY-MM-DD HH24:MI:SS.FF6') = ?",
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "29g. DateTime 叶子（无 Z）：只换 T 后 CAST",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc29_probe\" WHERE CAST(REPLACE(JSON_VALUE(\"d\", '$.dt'), 'T', ' ') AS TIMESTAMP(6)) = ?",
            ts_param(),
        )
        .await;
        probe(
            &mut conn,
            "29h. Date 叶子 = 绑定 DATE",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc29_probe\" WHERE CAST(JSON_VALUE(\"d\", '$.d') AS DATE) = ?",
            date_param(),
        )
        .await;
        probe(
            &mut conn,
            "29h2. Date 叶子原文",
            "SELECT JSON_VALUE(\"d\", '$.d') AS \"v\" FROM \"dm_doc29_probe\" WHERE \"id\" = 1",
            (),
        )
        .await;
        probe(
            &mut conn,
            "29i. Time 叶子 = 绑定 TIME",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc29_probe\" WHERE CAST(JSON_VALUE(\"d\", '$.tm') AS TIME(6)) = ?",
            time_param(),
        )
        .await;
        probe(
            &mut conn,
            "29j. 不经 cast 的 RFC 3339 文本叶子 = 绑定参数",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc29_probe\" WHERE JSON_VALUE(\"d\", '$.ts') = ?",
            ts_param(),
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_doc29_probe\" CASCADE CONSTRAINTS", ())
            .await;
    }

    if std::env::var("PROBE30").is_ok() {
        // 套件 `type_document::struct_embed_timestamp_truncates_to_micros` 的文档注释写明了
        // 驱动的时间**参数**契约：「SQL 后端的时间类型只存到微秒，JSON 编解码器写入时
        // **截断**，以匹配各驱动**截断式**的参数绑定，于是按原始纳秒值等值过滤仍能命中
        // 被截断存储的行」。第 27/28/29 节只量到微秒，这一节量达梦拿到**纳秒**绑定参数
        // 时是截断、四舍五入还是原样比较。
        let nanos_param = || {
            vec![DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_nano_opt(3, 4, 5, 123_456_789)
                    .unwrap(),
            )]
        };
        let micros_param = || {
            vec![DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_micro_opt(3, 4, 5, 123_456)
                    .unwrap(),
            )]
        };

        let _ = conn
            .execute("DROP TABLE \"dm_ts30_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "30a. 建 TIMESTAMP(6) 表",
            "CREATE TABLE \"dm_ts30_probe\" (\"id\" INT PRIMARY KEY, \"ts\" TIMESTAMP(6))",
        )
        .await;
        probe(
            &mut conn,
            "30b. 插微秒字面量",
            "INSERT INTO \"dm_ts30_probe\" (\"id\", \"ts\") VALUES (1, TIMESTAMP '2024-01-02 03:04:05.123456')",
            (),
        )
        .await;
        probe(
            &mut conn,
            "30c. 插纳秒字面量（服务端怎么存）",
            "INSERT INTO \"dm_ts30_probe\" (\"id\", \"ts\") VALUES (2, TIMESTAMP '2024-01-02 03:04:05.123456789')",
            (),
        )
        .await;
        probe(
            &mut conn,
            "30d. 插**纳秒**绑定参数（服务端怎么存）",
            "INSERT INTO \"dm_ts30_probe\" (\"id\", \"ts\") VALUES (3, ?)",
            nanos_param(),
        )
        .await;
        probe(
            &mut conn,
            "30e. 三行的存储文本",
            "SELECT \"id\" AS \"i\", TO_CHAR(\"ts\", 'YYYY-MM-DD HH24:MI:SS.FF6') AS \"v\" FROM \"dm_ts30_probe\" ORDER BY \"id\"",
            (),
        )
        .await;
        probe(
            &mut conn,
            "30f. TO_CHAR(纳秒绑定参数)",
            "SELECT TO_CHAR(?, 'YYYY-MM-DD HH24:MI:SS.FF6') AS \"v\" FROM DUAL",
            nanos_param(),
        )
        .await;
        probe(
            &mut conn,
            "30g. 纳秒参数 = 微秒字面量行（1）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts30_probe\" WHERE \"id\" = 1 AND \"ts\" = ?",
            nanos_param(),
        )
        .await;
        probe(
            &mut conn,
            "30h. 纳秒参数 = 纳秒字面量行（2）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts30_probe\" WHERE \"id\" = 2 AND \"ts\" = ?",
            nanos_param(),
        )
        .await;
        probe(
            &mut conn,
            "30i. 纳秒参数 = 纳秒参数行（3）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts30_probe\" WHERE \"id\" = 3 AND \"ts\" = ?",
            nanos_param(),
        )
        .await;
        probe(
            &mut conn,
            "30j. TO_CHAR(CAST(纳秒参数 AS TIMESTAMP(6)))",
            "SELECT TO_CHAR(CAST(? AS TIMESTAMP(6)), 'YYYY-MM-DD HH24:MI:SS.FF6') AS \"v\" FROM DUAL",
            nanos_param(),
        )
        .await;
        probe(
            &mut conn,
            "30k. 微秒参数 = 微秒字面量行（1）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_ts30_probe\" WHERE \"id\" = 1 AND \"ts\" = ?",
            micros_param(),
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_ts30_probe\" CASCADE CONSTRAINTS", ())
            .await;

        // 文档路径：真正让套件失败的那条形状。
        const DOC30: &str = r#"{"at":"2024-01-02T03:04:05.123456Z"}"#;
        let _ = conn
            .execute("DROP TABLE \"dm_doc30_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "30l. 建文档表",
            "CREATE TABLE \"dm_doc30_probe\" (\"id\" INT PRIMARY KEY, \"d\" CLOB)",
        )
        .await;
        probe(
            &mut conn,
            "30m. 插文档（codec 截断后的存储文本）",
            "INSERT INTO \"dm_doc30_probe\" (\"id\", \"d\") VALUES (1, ?)",
            vec![DmValue::Text(DOC30.to_string())],
        )
        .await;
        probe(
            &mut conn,
            "30n. 文档叶子 CAST 后 = **纳秒**参数（复现套件失败）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc30_probe\" WHERE CAST(REPLACE(REPLACE(JSON_VALUE(\"d\", '$.at'), 'T', ' '), 'Z', '') AS TIMESTAMP(6)) = ?",
            nanos_param(),
        )
        .await;
        probe(
            &mut conn,
            "30o. 文档叶子 CAST 后 = **微秒**参数（修法方向）",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc30_probe\" WHERE CAST(REPLACE(REPLACE(JSON_VALUE(\"d\", '$.at'), 'T', ' '), 'Z', '') AS TIMESTAMP(6)) = ?",
            micros_param(),
        )
        .await;
        probe(
            &mut conn,
            "30p. 文档叶子 CAST 后 = CAST(纳秒参数 AS TIMESTAMP(6))",
            "SELECT COUNT(*) AS \"cnt\" FROM \"dm_doc30_probe\" WHERE CAST(REPLACE(REPLACE(JSON_VALUE(\"d\", '$.at'), 'T', ' '), 'Z', '') AS TIMESTAMP(6)) = CAST(? AS TIMESTAMP(6))",
            nanos_param(),
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_doc30_probe\" CASCADE CONSTRAINTS", ())
            .await;
    }

    if std::env::var("PROBE31").is_ok() {
        // Round 28：`upsert_unique` + `upsert_branch_assignments` 两条形状的达梦真机确认。
        //
        // ① 冲突目标是**唯一键**（不是主键）时，`MERGE … ON (t."EMAIL" = s."s_1")` 命中已有行，
        //    保留的必须是**已有行的主键**（源行带的是另一个值）⇒ 驱动要按冲突目标列回查才能
        //    拿到那一行（`insert_and_reload` 的改动）。
        // ② `WHEN MATCHED THEN UPDATE SET "name" = s."s_2"`：「本次要写的新值」（引擎的
        //    `upsert.update(...)` 里引用 `incoming()`）渲染成 `USING` 源列，达梦必须接受。
        // ③ 自引用 `SET "name" = "name"`（`update.name(stmt::set(<已有列>))`）也要接受。
        let _ = conn
            .execute("DROP TABLE \"dm_upsert31_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "31a. 建表（主键 ID + 唯一键 EMAIL）",
            "CREATE TABLE \"dm_upsert31_probe\" (\
             \"ID\" INT PRIMARY KEY, \
             \"EMAIL\" VARCHAR(64) NOT NULL UNIQUE, \
             \"NAME\" VARCHAR(64))",
        )
        .await;
        probe(
            &mut conn,
            "31b. 首次 upsert：源行(1, alice, Alice)，ON 唯一键 ⇒ 应走 INSERT 分支",
            "MERGE INTO \"dm_upsert31_probe\" AS t \
             USING (SELECT ? AS \"s_0\", ? AS \"s_1\", ? AS \"s_2\" FROM DUAL) AS s \
             ON (t.\"EMAIL\" = s.\"s_1\") \
             WHEN MATCHED THEN UPDATE SET \"NAME\" = s.\"s_2\" \
             WHEN NOT MATCHED THEN INSERT (\"ID\", \"EMAIL\", \"NAME\") VALUES (s.\"s_0\", s.\"s_1\", s.\"s_2\")",
            vec![
                DmValue::Int(1),
                DmValue::Text("alice@example.com".to_string()),
                DmValue::Text("Alice".to_string()),
            ],
        )
        .await;
        probe(
            &mut conn,
            "31c. 二次 upsert：源行(2, alice, Alicia) ⇒ 应走 MATCHED 分支且**不改主键**",
            "MERGE INTO \"dm_upsert31_probe\" AS t \
             USING (SELECT ? AS \"s_0\", ? AS \"s_1\", ? AS \"s_2\" FROM DUAL) AS s \
             ON (t.\"EMAIL\" = s.\"s_1\") \
             WHEN MATCHED THEN UPDATE SET \"NAME\" = s.\"s_2\" \
             WHEN NOT MATCHED THEN INSERT (\"ID\", \"EMAIL\", \"NAME\") VALUES (s.\"s_0\", s.\"s_1\", s.\"s_2\")",
            vec![
                DmValue::Int(2),
                DmValue::Text("alice@example.com".to_string()),
                DmValue::Text("Alicia".to_string()),
            ],
        )
        .await;
        probe(
            &mut conn,
            "31d. 表内容（期望只有 1 行：id=1, name=Alicia）",
            "SELECT \"ID\" AS \"i\", \"EMAIL\" AS \"e\", \"NAME\" AS \"n\" FROM \"dm_upsert31_probe\"",
            (),
        )
        .await;
        probe(
            &mut conn,
            "31e. 驱动按**冲突目标列**回查（模拟 RETURNING 的第二步）",
            "SELECT \"ID\" AS \"i\", \"NAME\" AS \"n\" FROM \"dm_upsert31_probe\" WHERE \"EMAIL\" = ?",
            vec![DmValue::Text("alice@example.com".to_string())],
        )
        .await;
        probe(
            &mut conn,
            "31f. MATCHED 分支里自引用 `SET \"NAME\" = \"name\"`",
            "MERGE INTO \"dm_upsert31_probe\" AS t \
             USING (SELECT ? AS \"s_0\", ? AS \"s_1\", ? AS \"s_2\" FROM DUAL) AS s \
             ON (t.\"EMAIL\" = s.\"s_1\") \
             WHEN MATCHED THEN UPDATE SET \"NAME\" = \"NAME\" \
             WHEN NOT MATCHED THEN INSERT (\"ID\", \"EMAIL\", \"NAME\") VALUES (s.\"s_0\", s.\"s_1\", s.\"s_2\")",
            vec![
                DmValue::Int(3),
                DmValue::Text("alice@example.com".to_string()),
                DmValue::Text("ignored".to_string()),
            ],
        )
        .await;

        // 复合唯一键（套件 `upsert_by_composite_unique_constraint` 的形状）。
        let _ = conn
            .execute("DROP TABLE \"dm_upsert31b_probe\" CASCADE CONSTRAINTS", ())
            .await;
        ddl(
            &mut conn,
            "31g. 建表（主键 ID + 复合唯一键 TENANT/SLUG）",
            "CREATE TABLE \"dm_upsert31b_probe\" (\
             \"ID\" INT PRIMARY KEY, \
             \"TENANT\" VARCHAR(32) NOT NULL, \
             \"SLUG\" VARCHAR(32) NOT NULL, \
             \"VALUE\" VARCHAR(64), \
             CONSTRAINT \"uq_31b\" UNIQUE (\"TENANT\", \"SLUG\"))",
        )
        .await;
        probe(
            &mut conn,
            "31h. 复合唯一键首次 upsert（源行 1/acme/home/one）",
            "MERGE INTO \"dm_upsert31b_probe\" AS t \
             USING (SELECT ? AS \"s_0\", ? AS \"s_1\", ? AS \"s_2\", ? AS \"s_3\" FROM DUAL) AS s \
             ON (t.\"TENANT\" = s.\"s_1\" AND t.\"SLUG\" = s.\"s_2\") \
             WHEN MATCHED THEN UPDATE SET \"VALUE\" = s.\"s_3\" \
             WHEN NOT MATCHED THEN INSERT (\"ID\", \"TENANT\", \"SLUG\", \"VALUE\") VALUES (s.\"s_0\", s.\"s_1\", s.\"s_2\", s.\"s_3\")",
            vec![
                DmValue::Int(1),
                DmValue::Text("acme".to_string()),
                DmValue::Text("home".to_string()),
                DmValue::Text("one".to_string()),
            ],
        )
        .await;
        probe(
            &mut conn,
            "31i. 复合唯一键二次 upsert（源行 2/acme/home/two）",
            "MERGE INTO \"dm_upsert31b_probe\" AS t \
             USING (SELECT ? AS \"s_0\", ? AS \"s_1\", ? AS \"s_2\", ? AS \"s_3\" FROM DUAL) AS s \
             ON (t.\"TENANT\" = s.\"s_1\" AND t.\"SLUG\" = s.\"s_2\") \
             WHEN MATCHED THEN UPDATE SET \"VALUE\" = s.\"s_3\" \
             WHEN NOT MATCHED THEN INSERT (\"ID\", \"TENANT\", \"SLUG\", \"VALUE\") VALUES (s.\"s_0\", s.\"s_1\", s.\"s_2\", s.\"s_3\")",
            vec![
                DmValue::Int(2),
                DmValue::Text("acme".to_string()),
                DmValue::Text("home".to_string()),
                DmValue::Text("two".to_string()),
            ],
        )
        .await;
        probe(
            &mut conn,
            "31j. 表内容（期望 id=1, value=two）",
            "SELECT \"ID\" AS \"i\", \"TENANT\" AS \"t\", \"SLUG\" AS \"s\", \"VALUE\" AS \"v\" FROM \"dm_upsert31b_probe\"",
            (),
        )
        .await;
        probe(
            &mut conn,
            "31k. 驱动按复合冲突目标列回查",
            "SELECT \"ID\" AS \"i\", \"VALUE\" AS \"v\" FROM \"dm_upsert31b_probe\" WHERE \"TENANT\" = ? AND \"SLUG\" = ?",
            vec![
                DmValue::Text("acme".to_string()),
                DmValue::Text("home".to_string()),
            ],
        )
        .await;

        let _ = conn
            .execute("DROP TABLE \"dm_upsert31_probe\" CASCADE CONSTRAINTS", ())
            .await;
        let _ = conn
            .execute("DROP TABLE \"dm_upsert31b_probe\" CASCADE CONSTRAINTS", ())
            .await;
    }

    println!("\n=== 清理 ===");
    for table in [
        "dm_engine_probe",
        "dm_engine_probe2",
        "dm_dec_probe",
        "dm_clob_probe",
        "dm_enum_probe",
        "dm_time_probe",
        "dm_null_probe",
        "dm_merge_probe",
        "dm_ident_probe",
        "dm_ignore_probe",
        "dm_ignore_probe2",
        "dm_json_probe",
        "dm_arr_probe",
        "dm_list_probe",
        "dm_vecrm_probe",
    ] {
        let _ = conn
            .execute(&format!("DROP TABLE \"{table}\" CASCADE CONSTRAINTS"), ())
            .await;
    }
    println!("清理完成");
}
