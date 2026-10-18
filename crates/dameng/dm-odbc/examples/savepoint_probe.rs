//! 保存点提交探针：`ROLLBACK TO SAVEPOINT` 之后那次 `commit()` 到底有没有落地。
//!
//! 起因：T7.2 的 `dameng/examples/txn.rs` 在真机上跑出「保存点之前那行提交后应当可见、
//! 实得不可见」，`dm-query` 的变体矩阵（`tests/query_real.rs` 的
//! `real_probe_where_the_savepoint_commit_goes`）确认：**只有「`rollback_to` 之后紧接着
//! `commit`」会把整个事务丢掉**；中间多跑**任何**一条语句（哪怕是一条注定失败的
//! `RELEASE SAVEPOINT`，或者一条 `select`）都会恢复正常。
//!
//! 这个探针把池与 `dm-query` 两层都摘掉，只用 `dm-odbc` 的裸连接问三个问题：
//!
//! 1. 异常在裸 ODBC 层就能复现吗（还是我们上面两层的锅）？
//! 2. 「提交」之后连接自己还看得见那一行吗——即事务是不是压根没结束？
//! 3. 中间多一条语句、或者改用 `SQLSetConnectAttr(AUTOCOMMIT, ON)`，能救回来吗？
//!
//! 只打印、不断言：它是定位工具，不是门禁。
//!
//! 跑法：
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example savepoint_probe
//! ```

use dm_core::{DmConfig, DmValue, RowBuf};
use dm_error::DmError;
use dm_odbc::{
    FetchScratch, OdbcConnection, OdbcStatement, ParamArena, StmtOutcome, bind_params,
    describe_columns, fetch,
};

/// 探针专用表（带前缀，不碰集成夹具的 `DM_*` 表）。
const T: &str = "DM_ODBC_SP_PROBE";

fn connect() -> OdbcConnection {
    let cfg = match std::env::var("DM_TEST_DSN") {
        Ok(dsn) if !dsn.trim().is_empty() => {
            DmConfig::from_connection_string(&dsn).expect("DM_TEST_DSN 不是合法的连接串")
        }
        _ => DmConfig::from_env().expect("需要 DM_TEST_DSN 或 DM_SERVER / DM_USER / DM_PASSWORD"),
    };
    OdbcConnection::connect(&cfg).expect("连接失败")
}

/// 临时语句执行一条 SQL；结果集取尽后语句立即析构（＝关游标）。
fn exec(conn: &mut OdbcConnection, sql: &str) -> Result<(), DmError> {
    let mut stmt = conn.allocate_statement()?;
    match stmt.exec_direct(sql)? {
        StmtOutcome::ResultSet { .. } => drain(&mut stmt),
        StmtOutcome::Affected { .. } => Ok(()),
    }
}

fn drain(stmt: &mut OdbcStatement<'_>) -> Result<(), DmError> {
    let columns = describe_columns(stmt, None)?;
    // 列数必须与结果集一致：`RowBuf` 宽度不对会在 `push_row` 报 `Decode`。
    let mut buf = RowBuf::with_capacity(columns.len(), 8);
    let mut scratch = FetchScratch::new();
    loop {
        if fetch(stmt, &columns, &mut buf, &mut scratch)?.is_exhausted() {
            return Ok(());
        }
    }
}

/// 这条连接自己看不看得见表里的行（只问「有没有」，不解析值）。
fn sees(conn: &mut OdbcConnection, who: &str) -> bool {
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("      {who}：申请语句失败：{err}");
            return false;
        }
    };
    match stmt.exec_direct(&format!("select ID from {T}")) {
        Ok(StmtOutcome::ResultSet { .. }) => {}
        Ok(StmtOutcome::Affected { .. }) => {
            println!("      {who}：不是结果集");
            return false;
        }
        Err(err) => {
            println!("      {who}：执行失败：{err}");
            return false;
        }
    }
    let columns = match describe_columns(&mut stmt, None) {
        Ok(columns) => columns,
        Err(err) => {
            println!("      {who}：描述列失败：{err}");
            return false;
        }
    };
    let mut buf = RowBuf::with_capacity(columns.len(), 8);
    let mut scratch = FetchScratch::new();
    match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
        Ok(state) => state.rows() > 0,
        Err(err) => {
            println!("      {who}：取数失败：{err}");
            false
        }
    }
}

fn clear(writer: &mut OdbcConnection) {
    if let Err(err) = exec(writer, &format!("delete from {T}")) {
        println!("清表失败：{err}");
    }
}

/// 与 [`exec`] 同一形状，但执行完**显式**关一次游标（`SQLFreeStmt(SQL_CLOSE)`）。
///
/// `dm-query` 每次执行都以 `raw.close_cursor(&mut stmt)` 收尾
/// （`crates/dm-query/src/conn.rs:361`），而 [`exec`] 只是让语句析构（＝`SQLFreeHandle`）。
/// R13 就是为这一处差别准备的。
fn exec_with_close(conn: &mut OdbcConnection, sql: &str) -> Result<(), DmError> {
    let mut stmt = conn.allocate_statement()?;
    match stmt.exec_direct(sql)? {
        StmtOutcome::ResultSet { .. } => drain(&mut stmt)?,
        StmtOutcome::Affected { .. } => {}
    }
    stmt.close_cursor()
}

/// 按 `dm-query` 的真实路径跑一条**无参**语句：`prepare` → 同一句柄上 `exec_direct` → 关游标。
///
/// 这是 `dm-query::conn::prepare_and_execute` 对空参数语句的做法
/// （`crates/dm-query/src/conn.rs:280-293`：`raw.prepare(sql)` 之后
/// `raw.execute(&mut stmt, sql)`，而 `dm-odbc` 的 `execute` 在 `sql` 非空时走的正是
/// `exec_direct`，见 `crates/dm-odbc/src/lib.rs:188-193`）。与上面 `exec()` 的差别只有一处：
/// **句柄先 `SQLPrepare` 过一次**。R9/R10 就是为这一处差别准备的。
fn exec_like_query(conn: &mut OdbcConnection, sql: &str) -> Result<(), DmError> {
    let mut stmt = conn.allocate_statement()?;
    stmt.prepare(sql)?;
    match stmt.exec_direct(sql)? {
        StmtOutcome::ResultSet { .. } => drain(&mut stmt)?,
        StmtOutcome::Affected { .. } => {}
    }
    stmt.close_cursor()
}

/// 按 `dm-query` 的真实路径跑一条**带参**插入：`prepare` → `bind_params` → `exec` → 关游标
/// （`crates/dm-query/src/conn.rs:286-291` 的 `else` 分支）。
fn insert_like_query(conn: &mut OdbcConnection, id: i32, v: i32) -> Result<(), DmError> {
    let mut arena = ParamArena::new();
    let mut stmt = conn.allocate_statement()?;
    stmt.prepare(&format!("insert into {T} (ID, V) values (?, ?)"))?;
    bind_params(&mut stmt, &[DmValue::Int(id), DmValue::Int(v)], &mut arena)?;
    stmt.exec()?;
    stmt.close_cursor()
}

/// 按 `dm-query` 的真实路径开事务：`begin()`（＝autocommit OFF）之后**先**跑一句
/// `SET TRANSACTION ISOLATION LEVEL READ COMMITTED`
/// （`crates/dm-query/src/conn.rs:263-271`，SQL 文本见 `crates/dm-query/src/txn.rs`
/// 的 `isolation_sql`，其单元测试逐字钉住了这句话）。
fn begin_like_query(conn: &mut OdbcConnection) {
    conn.begin().expect("begin 失败");
    if let Err(err) = exec_like_query(conn, "SET TRANSACTION ISOLATION LEVEL READ COMMITTED") {
        println!("      ⚠️ SET TRANSACTION 失败：{err}");
    }
}

/// 报一次「提交方 / 外部（独立连接）」两个视角，并给一行结论。
fn report(label: &str, writer: &mut OdbcConnection, reader: &mut OdbcConnection) -> (bool, bool) {
    let own = sees(writer, "提交方");
    let outside = sees(reader, "外部");
    let verdict = if outside {
        "✅ 已提交（外部看得见）"
    } else if own {
        "❌ 事务没结束：只有提交方自己看得见未提交的行"
    } else {
        "⚠️ 两处都看不见：事务丢了"
    };
    println!("  {label}\n      提交方 = {own}，外部 = {outside} ⇒ {verdict}");
    (own, outside)
}

fn main() {
    let mut writer = connect();
    let mut reader = connect();

    for sql in [
        format!("drop table if exists {T}"),
        format!("create table {T} (ID INT PRIMARY KEY, V INT)"),
    ] {
        if let Err(err) = exec(&mut writer, &sql) {
            println!("建表失败：{err}");
            return;
        }
    }

    println!("\n== 保存点提交探针（dm-odbc 裸连接：不经池、不经 dm-query） ==");
    println!("表 {T}(ID INT PRIMARY KEY, V INT)；外部视角来自**另一条物理连接**\n");

    const INS1: &str = "insert into DM_ODBC_SP_PROBE (ID, V) values (1, 1)";
    const INS2: &str = "insert into DM_ODBC_SP_PROBE (ID, V) values (2, 2)";
    const SP: &str = "SAVEPOINT sp1";
    const BACK: &str = "ROLLBACK TO SAVEPOINT sp1";

    let step = |writer: &mut OdbcConnection, sql: &str| {
        if let Err(err) = exec(writer, sql) {
            println!("      ⚠️ `{sql}` 失败：{err}");
        }
    };

    // R0 基线：插入后直接提交。
    clear(&mut writer);
    println!("R0 基线：插入 → commit");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    writer.commit().expect("commit 失败");
    report("R0", &mut writer, &mut reader);

    // R1 ⭐ 最小复现：回滚到保存点之后**紧接着**提交。
    clear(&mut writer);
    println!("\nR1 最小复现：插入 → SAVEPOINT → 插入 → ROLLBACK TO SAVEPOINT → commit");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    step(&mut writer, BACK);
    writer.commit().expect("commit 失败");
    let (_, after_r1) = report("R1", &mut writer, &mut reader);

    // R1b：在 R1 那个连接上再跑一条无关语句 + 再提交一次，看能不能救回来。
    println!("\nR1b 续 R1：再跑一条无关语句 → 再 commit 一次");
    step(&mut writer, "select 1 from dual");
    match writer.commit() {
        Ok(()) => println!("      第二次 commit 成功"),
        Err(err) => println!("      第二次 commit 失败：{err}"),
    }
    let (_, after_r1b) = report("R1b", &mut writer, &mut reader);
    println!(
        "      ⇒ 第一次提交（外部 = {after_r1}）与补一条语句后再提交（外部 = {after_r1b}）的差别，\
         就是「事务当时还在不在」"
    );

    // R1c：真提交过的话，回滚不可能把行拿掉。
    println!("\nR1c 续 R1b：rollback");
    match writer.rollback() {
        Ok(()) => println!("      rollback 成功"),
        Err(err) => println!("      rollback 失败：{err}"),
    }
    report("R1c", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R2 不做：`OdbcStatement` 可变借用它的连接，所以「语句句柄活到 commit 之后」在这个
    // API 层**根本写不出来**（写了就是 `E0499`），`dm-query` 那一层同样如此。
    // ⇒「驱动需要先关掉语句才认 `SQLEndTran`」这个假设只能在「语句已经析构」这一侧验证，
    //   也就是下面所有 R 变体的形状（每条语句都是临时对象、执行完立即析构）。
    println!(
        "\nR2 略过：`OdbcStatement` 可变借用连接，「语句活到 commit 之后」在本层写不出来（E0499）"
    );

    // R3：不用 `commit()`，改用 `SQLSetConnectAttr(AUTOCOMMIT, ON)`。
    clear(&mut writer);
    println!("\nR3：ROLLBACK TO SAVEPOINT → set_autocommit(true)");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    step(&mut writer, BACK);
    match writer.set_autocommit(true) {
        Ok(()) => println!("      set_autocommit(true) 成功"),
        Err(err) => println!("      set_autocommit(true) 失败：{err}"),
    }
    report("R3", &mut writer, &mut reader);

    // R4：回滚到保存点之后插一条无关语句，再提交（镜像 dm-query 的 D/F/H 变体）。
    clear(&mut writer);
    println!("\nR4：ROLLBACK TO SAVEPOINT → select 1 → commit");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    step(&mut writer, BACK);
    step(&mut writer, "select 1 from dual");
    writer.commit().expect("commit 失败");
    report("R4", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R5：建了保存点但不回滚（对照组）。
    clear(&mut writer);
    println!("\nR5 对照：插入 → SAVEPOINT → 插入 → commit（不回滚）");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    writer.commit().expect("commit 失败");
    report("R5", &mut writer, &mut reader);

    // R6：保存点之后没有插入直接回滚（镜像 dm-query 的 G 变体）。
    clear(&mut writer);
    println!("\nR6：插入 → SAVEPOINT → ROLLBACK TO SAVEPOINT → commit（中间无语句）");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, BACK);
    writer.commit().expect("commit 失败");
    report("R6", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R7 ⭐ 修法候选：回滚到保存点之后把 autocommit 重新置 OFF——如果驱动在
    // `ROLLBACK TO SAVEPOINT` 之后把自己的「事务中」感知清掉了，这一下会把状态扳回来。
    clear(&mut writer);
    println!("\nR7 修法候选：ROLLBACK TO SAVEPOINT → set_autocommit(false) → commit");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    step(&mut writer, BACK);
    match writer.set_autocommit(false) {
        Ok(()) => println!("      set_autocommit(false) 成功"),
        Err(err) => println!("      set_autocommit(false) 失败：{err}"),
    }
    writer.commit().expect("commit 失败");
    report("R7（重新武装 autocommit）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R8：先 ON 再 OFF（强制驱动与服务端就 autocommit 对话两次）再提交——R7 的加固版。
    clear(&mut writer);
    println!("\nR8：ROLLBACK TO SAVEPOINT → set_autocommit(true) → set_autocommit(false) → commit");
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    step(&mut writer, BACK);
    for flag in [true, false] {
        match writer.set_autocommit(flag) {
            Ok(()) => println!("      set_autocommit({flag}) 成功"),
            Err(err) => println!("      set_autocommit({flag}) 失败：{err}"),
        }
    }
    writer.commit().expect("commit 失败");
    report("R8（ON→OFF 来回）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R9：把 `ROLLBACK TO SAVEPOINT` 换成 `dm-query` 的真实路径（先 `SQLPrepare`、再在同一
    // 句柄上 `SQLExecDirect`），其余仍是裸 `SQLExecDirect`。看「prepare 过一次」是不是关键差别。
    // 每个 R9 之后的变体都换一条全新连接：上一条很可能正带着「驱动与服务端对事务的认知不一致」的状态。
    writer = connect();
    clear(&mut writer);
    println!(
        "\nR9：插入 → SAVEPOINT → 插入 → ROLLBACK TO SAVEPOINT（prepare→exec_direct）→ commit"
    );
    writer.begin().expect("begin 失败");
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    match exec_like_query(&mut writer, BACK) {
        Ok(()) => println!("      exec_like_query(ROLLBACK TO SAVEPOINT) 成功"),
        Err(err) => println!("      exec_like_query(ROLLBACK TO SAVEPOINT) 失败：{err}"),
    }
    writer.commit().expect("commit 失败");
    report(
        "R9（回滚语句走 prepare+exec_direct）",
        &mut writer,
        &mut reader,
    );
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R10 ⭐ 完全镜像 dm-query 的形状：`begin()` 之后先下发隔离级别语句，插入走
    // `prepare`+`bind_params`+`exec`，无参语句走 `prepare`+`exec_direct`+`close_cursor`。
    writer = connect();
    clear(&mut writer);
    println!(
        "\nR10：完全镜像 dm-query（SET TRANSACTION → 绑定插入 → SAVEPOINT → 绑定插入 → ROLLBACK TO SAVEPOINT → commit）"
    );
    begin_like_query(&mut writer);
    match insert_like_query(&mut writer, 1, 1) {
        Ok(()) => println!("      绑定插入 ID=1 成功"),
        Err(err) => println!("      绑定插入 ID=1 失败：{err}"),
    }
    if let Err(err) = exec_like_query(&mut writer, SP) {
        println!("      SAVEPOINT 失败：{err}");
    }
    match insert_like_query(&mut writer, 2, 2) {
        Ok(()) => println!("      绑定插入 ID=2 成功"),
        Err(err) => println!("      绑定插入 ID=2 失败：{err}"),
    }
    if let Err(err) = exec_like_query(&mut writer, BACK) {
        println!("      ROLLBACK TO SAVEPOINT 失败：{err}");
    }
    writer.commit().expect("commit 失败");
    let (_, after_r10) = report("R10", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R11：R10 + 回滚之后多一条语句（对照组）。镜像 dm-query 的 D/F/H：多任何一条语句都能救回来。
    writer = connect();
    clear(&mut writer);
    println!("\nR11 对照：R10 + ROLLBACK TO SAVEPOINT → select 1 → commit");
    begin_like_query(&mut writer);
    let _ = insert_like_query(&mut writer, 1, 1);
    let _ = exec_like_query(&mut writer, SP);
    let _ = insert_like_query(&mut writer, 2, 2);
    let _ = exec_like_query(&mut writer, BACK);
    if let Err(err) = exec_like_query(&mut writer, "select 1 from dual") {
        println!("      select 1 失败：{err}");
    }
    writer.commit().expect("commit 失败");
    let (_, after_r11) = report("R11", &mut writer, &mut reader);
    println!(
        "      ⇒ R10 外部 = {after_r10}，R11 外部 = {after_r11}；两者若不同，\"RollbackTo 之后紧接 commit\" \
         就是这一层的重现形状"
    );
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R12：与 R10 同一形状，只把「绑定插入」换回「字面量插入」——隔离绑定这条路径。
    writer = connect();
    clear(&mut writer);
    println!(
        "\nR12：R10 形状但插入走字面量（SET TRANSACTION → 字面量插入 → SAVEPOINT → 字面量插入 → ROLLBACK TO SAVEPOINT → commit）"
    );
    begin_like_query(&mut writer);
    step(&mut writer, INS1);
    if let Err(err) = exec_like_query(&mut writer, SP) {
        println!("      SAVEPOINT 失败：{err}");
    }
    step(&mut writer, INS2);
    if let Err(err) = exec_like_query(&mut writer, BACK) {
        println!("      ROLLBACK TO SAVEPOINT 失败：{err}");
    }
    writer.commit().expect("commit 失败");
    let (_, after_r12) = report("R12", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R13：与 R1 同一形状（不 prepare），只把「执行完就析构」换成「显式 close_cursor」——
    // 隔离 `SQLFreeStmt(SQL_CLOSE)` 这一处。
    writer = connect();
    clear(&mut writer);
    println!(
        "\nR13：插入 → SAVEPOINT → 插入 → ROLLBACK TO SAVEPOINT（exec_direct + close_cursor）→ commit"
    );
    writer.begin().expect("begin 失败");
    if let Err(err) = exec_with_close(&mut writer, INS1) {
        println!("      INS1 失败：{err}");
    }
    if let Err(err) = exec_with_close(&mut writer, SP) {
        println!("      SAVEPOINT 失败：{err}");
    }
    if let Err(err) = exec_with_close(&mut writer, INS2) {
        println!("      INS2 失败：{err}");
    }
    if let Err(err) = exec_with_close(&mut writer, BACK) {
        println!("      ROLLBACK TO SAVEPOINT 失败：{err}");
    }
    writer.commit().expect("commit 失败");
    let (_, after_r13) = report("R13", &mut writer, &mut reader);
    println!(
        "      ⇒ R12 外部 = {after_r12}，R13 外部 = {after_r13}；把 R1/R9/R10/R12/R13 五个外部值并排看，\
         就能指出是 prepare、close_cursor、SET TRANSACTION 还是绑定插入把 commit 弄丢的"
    );
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R14：R12 的对照组——隔离级别语句仍走 `prepare`+`exec_direct`，但保存点两句换回「一条
    // `SQLExecDirect` 了事」。如果 R12 坏、R14 好，那么「预处理过的 `ROLLBACK TO SAVEPOINT`」
    // 就是根因，R15 就是修法在真实形状下的验证。
    writer = connect();
    clear(&mut writer);
    println!(
        "\nR14 修法候选：SET TRANSACTION（prepare）→ 字面量插入 → SAVEPOINT / ROLLBACK TO SAVEPOINT（只 exec_direct）→ commit"
    );
    begin_like_query(&mut writer);
    step(&mut writer, INS1);
    step(&mut writer, SP);
    step(&mut writer, INS2);
    step(&mut writer, BACK);
    writer.commit().expect("commit 失败");
    let (_, after_r14) = report("R14（保存点语句不 prepare）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R15 ⭐ 修法候选（真实形状）：完全镜像 dm-query——隔离级别语句与**绑定插入**都走 prepare，
    // 只有保存点那两句换成「不预处理、直接 `SQLExecDirect`」。R10 坏而 R15 好 ⇒ 修法成立。
    writer = connect();
    clear(&mut writer);
    println!(
        "\nR15 修法候选（真实形状）：SET TRANSACTION + 绑定插入（prepare）→ SAVEPOINT / ROLLBACK TO SAVEPOINT（只 exec_direct）→ commit"
    );
    begin_like_query(&mut writer);
    match insert_like_query(&mut writer, 1, 1) {
        Ok(()) => println!("      绑定插入 ID=1 成功"),
        Err(err) => println!("      绑定插入 ID=1 失败：{err}"),
    }
    step(&mut writer, SP);
    match insert_like_query(&mut writer, 2, 2) {
        Ok(()) => println!("      绑定插入 ID=2 成功"),
        Err(err) => println!("      绑定插入 ID=2 失败：{err}"),
    }
    step(&mut writer, BACK);
    writer.commit().expect("commit 失败");
    let (_, after_r15) = report("R15", &mut writer, &mut reader);
    println!(
        "\n== 结论汇总 ==\n  R10（完全镜像 dm-query，外部 = {after_r10}）\n  \
         R14（只有保存点语句不 prepare，外部 = {after_r14}）\n  \
         R15（真实形状 + 保存点语句不 prepare，外部 = {after_r15}）\n  \
         R11（R10 + 回滚后多一条语句，外部 = {after_r11}）"
    );
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // -----------------------------------------------------------------------
    // R16–R21：把「哪一处让 commit 失效」压到单一变量。
    //
    // R9–R15 全是**全新连接**，而 R1/R6（形状相同、却通过）跑在那条「已经提交过一次
    // 事务」的连接上 ⇒ 连接新旧与 prepare/close_cursor 三件事被混在一起了。
    // 这一段每个变体都换新连接，逐个排除。
    // -----------------------------------------------------------------------
    let fresh = |label: &str| {
        let mut conn = connect();
        clear(&mut conn);
        println!("\n{label}");
        conn
    };
    let insert = |id: i32| format!("insert into {T} (ID, V) values ({id}, {id})");

    // R16 控制组：全新连接 + 与 R6 逐字相同的调用序列（纯 exec_direct、不 prepare、不 close_cursor）。
    writer = fresh("R16 控制：全新连接 + R6 形状（纯 exec_direct，回滚后紧接 commit）");
    writer.begin().expect("begin 失败");
    let r16_a = insert(101);
    let r16_b = insert(102);
    step(&mut writer, &r16_a);
    step(&mut writer, SP);
    step(&mut writer, &r16_b);
    step(&mut writer, BACK);
    writer.commit().expect("commit 失败");
    let (_, after_r16) = report("R16", &mut writer, &mut reader);
    // R16b：再 commit 一次——如果这次好了，说明驱动只是把「事务开着」的标志清了一次。
    match writer.commit() {
        Ok(()) => println!("      第二次 commit 成功"),
        Err(err) => println!("      第二次 commit 失败：{err}"),
    }
    let (_, after_r16b) = report("R16b（再 commit 一次）", &mut writer, &mut reader);
    // R16c：补一条语句之后再 commit（dm-query 的 D/F/H 形状）。
    step(&mut writer, "select 1 from dual");
    match writer.commit() {
        Ok(()) => println!("      补一条语句后的 commit 成功"),
        Err(err) => println!("      补一条语句后的 commit 失败：{err}"),
    }
    let (_, after_r16c) = report("R16c（补一条语句再 commit）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R17：那一句 commit 改成**语句形式**（`exec("commit")`），不走 SQLEndTran。
    writer = fresh("R17：全新连接 + ROLLBACK TO SAVEPOINT → exec(\"commit\")（语句式提交）");
    writer.begin().expect("begin 失败");
    let r17_a = insert(201);
    let r17_b = insert(202);
    step(&mut writer, &r17_a);
    step(&mut writer, SP);
    step(&mut writer, &r17_b);
    step(&mut writer, BACK);
    step(&mut writer, "commit");
    let (_, after_r17) = report("R17（语句式 commit）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R18：失败的 commit 之后调 set_autocommit(true)——按 ODBC 规范它会**提交**未完成事务。
    // 它到底把行提交了还是回滚了，决定池的复位动作会不会把用户的写入丢掉。
    writer = fresh("R18：全新连接 + 回滚 → commit（预期失效）→ set_autocommit(true)");
    writer.begin().expect("begin 失败");
    let r18_a = insert(301);
    let r18_b = insert(302);
    step(&mut writer, &r18_a);
    step(&mut writer, SP);
    step(&mut writer, &r18_b);
    step(&mut writer, BACK);
    writer.commit().expect("commit 失败");
    let (_, after_r18) = report("R18（commit 之后）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      set_autocommit(true) 失败：{err}");
    }
    let (own_r18b, outside_r18b) = report(
        "R18b（set_autocommit(true) 之后）",
        &mut writer,
        &mut reader,
    );
    println!(
        "      ⇒ 提交方 = {own_r18b}、外部 = {outside_r18b}：两者都为真＝属性设置把事务**提交**了；\
         都变假＝被**回滚**了"
    );

    // R19：回滚之后补一条语句再 commit（对照 R16c，但语句更早）。
    writer = fresh("R19：全新连接 + ROLLBACK TO SAVEPOINT → select 1 → commit");
    writer.begin().expect("begin 失败");
    let r19_a = insert(401);
    let r19_b = insert(402);
    step(&mut writer, &r19_a);
    step(&mut writer, SP);
    step(&mut writer, &r19_b);
    step(&mut writer, BACK);
    step(&mut writer, "select 1 from dual");
    writer.commit().expect("commit 失败");
    let (_, after_r19) = report("R19（回滚后补一条语句）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R20 ⭐ 关键对照：先在**同一条新连接**上完整跑一次事务（这会让连接停在
    // autocommit = OFF，于是后面这次事务里不再发生 ON→OFF 的属性变更），再跑 R16 的形状。
    writer = fresh("R20：新连接先完整跑一次事务（把 autocommit 留在 OFF）→ 再跑 R16 形状");
    writer.begin().expect("begin 失败");
    let r20_w = insert(501);
    step(&mut writer, &r20_w);
    writer.commit().expect("commit 失败");
    println!("      预热事务已提交（此时 autocommit 仍为 OFF，与 R0 之后的状态一致）");
    let r20_a = insert(502);
    let r20_b = insert(503);
    step(&mut writer, &r20_a);
    step(&mut writer, SP);
    step(&mut writer, &r20_b);
    step(&mut writer, BACK);
    writer.commit().expect("commit 失败");
    let (_, after_r20) = report(
        "R20（本事务内没有 autocommit 属性变更）",
        &mut writer,
        &mut reader,
    );
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    // R21：把「回滚之后最后一条语句」换成 SAVEPOINT（不是 ROLLBACK 开头），看驱动是不是
    // 按语句前缀把 ROLLBACK TO SAVEPOINT 误判成「事务结束」。
    writer = fresh("R21：全新连接 + ROLLBACK TO SAVEPOINT → SAVEPOINT sp2 → commit");
    writer.begin().expect("begin 失败");
    let r21_a = insert(601);
    let r21_b = insert(602);
    step(&mut writer, &r21_a);
    step(&mut writer, SP);
    step(&mut writer, &r21_b);
    step(&mut writer, BACK);
    step(&mut writer, "SAVEPOINT sp2");
    writer.commit().expect("commit 失败");
    let (_, after_r21) = report("R21（最后一条语句是 SAVEPOINT）", &mut writer, &mut reader);
    if let Err(err) = writer.set_autocommit(true) {
        println!("      复位 autocommit 失败：{err}");
    }

    println!(
        "\n== R16–R21 汇总 ==\n  R16 全新连接 + R6 形状           外部 = {after_r16}\n  \
         R16b 再 commit 一次              外部 = {after_r16b}\n  \
         R16c 补一条语句再 commit         外部 = {after_r16c}\n  \
         R17 语句式 commit                外部 = {after_r17}\n  \
         R18 commit 之后                  外部 = {after_r18}\n  \
         R19 回滚后补一条语句             外部 = {after_r19}\n  \
         R20 本事务无 autocommit 变更     外部 = {after_r20}\n  \
         R21 最后一条语句是 SAVEPOINT     外部 = {after_r21}"
    );

    let _ = exec(&mut writer, &format!("drop table if exists {T}"));
    println!("\n（探针不删除表以外的任何东西；跑完已 drop 探针表）");
}
