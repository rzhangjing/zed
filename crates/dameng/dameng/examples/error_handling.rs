//! 错误处理示例：把「语义分类」和「驱动原话」放在一起看。
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dameng --example error_handling
//! ```
//!
//! 本示例要传达的三件事（设计见 `docs/设计方案.md` §6.8、§8）：
//!
//! 1. **错误是分类好的**：`err.kind` 是一组能 `match` 的语义，而不是让你去 `contains("…")`
//!    的字符串。分类只在**真机实测登记过**的错误码上生效（`dm-error` 的 `codes` 模块），
//!    未登记的一律落回 [`DmErrorKind::Server`] 并把原始三元组一字不改地留着；
//! 2. **原话永远不丢**：`err.diags()` 保留 `SQLGetDiagRec` 采到的每一条记录
//!    （SQLSTATE / 原生码 / 列号 / 行号 / 宽字符消息），分类只改 `kind`，不吞诊断；
//! 3. **重试判定问库、不问人**：`err.is_connection_lost()`（这条连接还能用吗）与
//!    `err.is_retryable()`（换个连接重放安全吗）是两条独立的判据。
//!
//! 每个场景都**不 panic**：本示例要展示「各种错误长什么样」，若某个错误在别的达梦
//! 版本上没出现（或没被分类），它只打印一行说明。真正致命的问题（连不上、建表失败）
//! 仍用 `?` 直接抛出——示例不该把连接错误也吞掉。

use dameng::prelude::*;
// `Fingerprint` 不在 prelude 里（prelude 只放绝大多数程序都要用的项），从 crate 根取。
use dameng::Fingerprint;

/// 示例表：名字带前缀，避免与别的示例/测试撞名。
///
/// `NAME VARCHAR(5)` 是故意的：**按字节计量**，5 个字节足够触发 `-70005 字符串截断`。
const T_ERR: &str = "DM_EXAMPLE_ERR";

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

/// 把一条 `DmError` 拆开打印：语义分类、阶段、诊断链、重试判定。
fn report(label: &str, err: &DmError) {
    println!("── {label}");
    println!("   kind         : {:?}", err.kind);
    println!("   面向人的描述 : {err}");
    println!("   阶段         : {}", err.context.stage);
    println!(
        "   SQL 指纹     : {}",
        err.context.sql_fingerprint.as_deref().unwrap_or("（无）")
    );
    println!("   连接不可用？ : {}", err.is_connection_lost());
    println!("   可重试？     : {}", err.is_retryable());
    if let Some((code, msg)) = err.server_code_and_message() {
        println!("   服务端三元组 : 原生码 {code} / {msg}");
    }
    for (i, diag) in err.diags().iter().enumerate() {
        println!(
            "   诊断[{i}]      : SQLSTATE {} / 原生码 {} / 列 {:?} / 行 {:?} / {}",
            diag.sqlstate_str(),
            diag.native_code,
            diag.column,
            diag.row,
            diag.message
        );
    }
}

// 当前线程运行时足够（阻塞工作都在 `dm-pool` 的阻塞线程池上）。
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), DmError> {
    // ---- 0) 不碰数据库：本地校验失败也是分类好的错误 ------------------------
    //
    // `pool.max_size` 就是 ODBC 的并发上限（达梦不支持异步），0 是非法值；
    // `DmConfigBuilder::build` 在进驱动之前就把它拦下，类型是 `Config`。
    if let Err(err) = DmConfig::builder()
        .server("localhost")
        .pool_max_size(0)
        .build()
    {
        report("本地配置校验：pool.max_size = 0（无需数据库）", &err);
    } else {
        println!("（意外）pool.max_size = 0 竟然通过了校验");
    }

    let Some(cfg) = config() else {
        println!("跳过：未配置 DM_TEST_DSN / DM_SERVER（本示例需要一台可达的达梦 DM8）");
        return Ok(());
    };

    let pool = Pool::connect_with(cfg, ExecutorConfig::for_pool(1)).await?;
    let mut conn = PooledConn::acquire(&pool).await?;

    // ---- 1) SQL 指纹：进日志的 SQL 不含字面量，也不含参数值 ---------------
    let raw = "select * from DM_EXAMPLE_ERR where ID = 42 and NOTE = '机密'";
    println!("指纹：{raw}\n   → {}", Fingerprint::of(raw));

    // 建表（达梦没有 `create table if not exists`，所以先 `drop … if exists`）。
    conn.execute(&format!("drop table if exists {T_ERR}"), ())
        .await?;
    conn.execute(
        &format!("create table {T_ERR} (ID INT PRIMARY KEY, NAME VARCHAR(5))"),
        (),
    )
    .await?;

    // ---- 2) 语法错误：达梦不认反引号（`42000 / -2007`）→ Syntax ------------
    if let Err(err) = conn.query("select `ID` from dual", ()).await {
        report("语法：反引号包标识符", &err);
    } else {
        println!("（意外）反引号竟然被接受了——请更新能力表与 §6.8");
    }

    // ---- 3) 对象不存在：表名打错（`42S02 / -2106`）→ ObjectNotFound -------
    if let Err(err) = conn
        .query(&format!("select 1 as N from {T_ERR}_NOPE"), ())
        .await
    {
        report("对象不存在：表名打错", &err);
    } else {
        println!("（意外）查询不存在的表竟然成功了");
    }

    // ---- 4) 形状错误：批量参数每组个数不一致 → 客户端侧的 Config ----------
    //
    // 这条**不发往服务端**：`dm-query` 在把行优先参数转成列式之前先做形状校验
    // （每组参数个数相同、列数 == `?` 个数），否则错位绑定只会得到更难懂的驱动报错。
    let rows: Vec<Vec<DmValue>> = vec![vec![1_i32.into(), "接口".into()], vec![2_i32.into()]];
    match conn
        .execute_many(
            &format!("insert into {T_ERR} (ID, NAME) values (?, ?)"),
            rows,
        )
        .await
    {
        Ok(outcome) => println!("（意外）形状不一致竟然执行成功了：{outcome:?}"),
        Err(err) => report("批量形状：第 2 组只有 1 个参数", &err),
    }

    // ---- 5) 参数超长：`22001 / -70005` → Encode ---------------------------
    //
    // 注意 `varchar(5)` 是**按字节**算的（`LENGTH_IN_CHAR=0`）：汉字占 3 字节，
    // 所以下面的字符串光按字节就远超列宽——这条错误与「塞了几个字符」无关。
    let too_long = "超长超长超长超长超长超长超长";
    println!(
        "待写入字符串：{} 个字符 / {} 字节，列 `NAME VARCHAR(5)` = 5 字节",
        too_long.chars().count(),
        too_long.len()
    );
    match conn
        .execute(
            &format!("insert into {T_ERR} (ID, NAME) values (?, ?)"),
            (10_i32, too_long),
        )
        .await
    {
        Ok(n) => println!("（意外）{n} 行写进去了——请确认列宽与计量方式（按字节？）"),
        Err(err) => report("参数超长：往 varchar(5) 塞远超 5 字节的串", &err),
    }

    // ---- 6) 未登记的错误码：如实落回 Server，原话保留 ---------------------
    //
    // 主键冲突在达梦是 `23000 / -6602`。这条**故意没登记**成 `DuplicateKey`：
    // 实测（§12.2 G6 记录）达梦把批量违约报成**语句级**错误，拿不到行下标，
    // 客户端也拿不到约束名以外的信息，所以宁可不猜、只给原始三元组。
    conn.execute(
        &format!("insert into {T_ERR} (ID, NAME) values (?, ?)"),
        (20_i32, "先来"),
    )
    .await?;
    if let Err(err) = conn
        .execute(
            &format!("insert into {T_ERR} (ID, NAME) values (?, ?)"),
            (20_i32, "后来"),
        )
        .await
    {
        report("主键冲突（未登记 → Server）", &err);
    } else {
        println!("（意外）重复主键竟然写进去了");
    }

    // ---- 7) `NULL` 读成非 `Option` → NullViolation（带列名）-------------
    //
    // 探针查询本身如果不成立（例如 `cast(null as varchar(10))` 在达梦上另有写法），
    // 这里也只报告、不中断：示例的职责是把错误语义讲清楚，真机断言在集成测试里。
    match conn
        .query_one("select cast(null as varchar(10)) as S from dual", ())
        .await
    {
        Err(err) => report("探针查询失败：`cast(null as varchar(10))`", &err),
        Ok(None) => println!("（意外）`select cast(null as …)` 没有返回行"),
        Ok(Some(row)) => match row.get::<String>("S") {
            Ok(v) => println!("（意外）NULL 读成了 {v:?}"),
            Err(err) => report("取值为 NULL 但目标类型是 String", &err),
        },
    }

    // ---- 8) 类型不匹配：文本解析失败 → Decode（带列名）-------------------
    match conn.query_one("select 'abc' as S from dual", ()).await {
        Err(err) => report("探针查询失败：`select 'abc' as S from dual`", &err),
        Ok(None) => println!("（意外）查询没有返回行"),
        Ok(Some(row)) => match row.get::<i32>("S") {
            Ok(v) => println!("（意外）'abc' 解析成了 {v}"),
            Err(err) => report("文本 → i32：解析失败", &err),
        },
    }

    // ---- 9) 二进制 → 文本：直接拒绝（DataTypeMismatch），不硬解出乱码 ----
    match conn
        .query_one("select cast('ab' as varbinary(4)) as B from dual", ())
        .await
    {
        Err(err) => report("探针查询失败：`cast('ab' as varbinary(4))`", &err),
        Ok(None) => println!("（意外）查询没有返回行"),
        Ok(Some(row)) => match row.get::<String>("B") {
            Ok(v) => println!("（意外）二进制读成了文本：{v:?}"),
            Err(err) => report("二进制 → String：直接拒绝", &err),
        },
    }

    // 收尾：清表（示例不该在库里留东西）→ 关池 → 等存活连接归零。
    conn.execute(&format!("drop table if exists {T_ERR}"), ())
        .await?;
    drop(conn);
    pool.close().await?;
    pool.wait_closed().await;
    println!("完成。");
    Ok(())
}
