//! `OdbcBackend`：把 `dm-core` 的 [`Backend`] trait 端到端跑一遍。
//!
//! 前面几个文件测的是 `dm-odbc` 自己的类型（`OdbcConnection` / `OdbcStatement` / `fetch`…）。
//! 这个文件测的是**上层真正会用到的那张脸**：`dm-pool` / `dm-query` 只认 `Backend`，
//! 它一旦在某个方法上语义走偏（比如把「驱动没说影响行数」当成「零行」），
//! 上层拿到的是一个**静默的错值**，不会有任何报错。

use dm_core::backend::{Backend, ExecOutcome, FetchState};
use dm_core::{DmValue, RowBuf};

use dm_odbc::OdbcBackend;

use crate::common::{T_TXN, dm_test, test_config};

/// 连一个后端（用与夹具同一套环境变量；没配环境时夹具已经让用例返回了）。
fn backend(h: &mut crate::common::Harness) -> Option<OdbcBackend> {
    let Some(cfg) = test_config() else {
        h.fail("环境没配好");
        return None;
    };
    match OdbcBackend::connect(&cfg) {
        Ok(backend) => Some(backend),
        Err(err) => {
            h.record("OdbcBackend::connect", &err);
            None
        }
    }
}

dm_test!(fn backend_connect_ping_and_name(h) {
    let Some(mut backend) = backend(h) else {
        return;
    };
    h.eq("名字用于诊断输出", backend.name(), "dm-odbc");

    match backend.ping() {
        // `Ok(false)` 表示「连接已死」而不是错误；刚连上就该是活的。
        Ok(alive) => h.check("刚连上 ping 是活的", alive),
        Err(err) => h.record("ping", &err),
    }

    // 活语句计数是 CT-09 的可观测面：这里刚连上，还没有语句被分配出去。
    h.eq("活语句计数从 0 开始", backend.connection().live_statement_count(), 0);
});

dm_test!(fn backend_prepare_bind_execute_reports_affected_rows(h) {
    h.apply_schema();
    let Some(mut backend) = backend(h) else {
        return;
    };

    let mut stmt = match backend.prepare(&format!("insert into {T_TXN} (ID, V) values (?, ?)")) {
        Ok(stmt) => stmt,
        Err(err) => return h.record("prepare", &err),
    };
    let args = [DmValue::Int(1), DmValue::Int(11)];
    if let Err(err) = backend.bind_params(&mut stmt, &args) {
        return h.record("bind_params", &err);
    }
    match backend.execute(&mut stmt, "") {
        Ok(ExecOutcome::Affected { rows }) => h.eq("插一行报 1 行", rows, Some(1)),
        Ok(other) => h.fail(format!("insert 不该产出结果集：{other:?}")),
        Err(err) => return h.record("execute", &err),
    }
    if let Err(err) = backend.close_cursor(&mut stmt) {
        h.record("close_cursor", &err);
    }

    // 再插一行，确认同一个后端可以连续 prepare 多条语句（语句是交出去的，后端不留表）。
    let mut stmt2 = match backend.prepare(&format!("insert into {T_TXN} (ID, V) values (?, ?)")) {
        Ok(stmt) => stmt,
        Err(err) => return h.record("第二次 prepare", &err),
    };
    let args = [DmValue::Int(2), DmValue::Int(22)];
    if let Err(err) = backend.bind_params(&mut stmt2, &args) {
        return h.record("第二次 bind_params", &err);
    }
    if let Err(err) = backend.execute(&mut stmt2, "") {
        return h.record("第二次 execute", &err);
    }
    if let Err(err) = backend.close_cursor(&mut stmt2) {
        h.record("第二次 close_cursor", &err);
    }
    drop(stmt);
    drop(stmt2);

    h.expect_scalar(
        "两行都落库",
        &format!("select count(*) from {T_TXN}"),
        "2",
    );
    h.eq(
        "语句析构之后活语句计数回到 0",
        backend.connection().live_statement_count(),
        0,
    );
});

dm_test!(fn backend_select_columns_and_fetch_through_the_trait(h) {
    h.apply_schema();
    let Some(mut backend) = backend(h) else {
        return;
    };

    let Some(cfg) = test_config() else { return };
    let _ = cfg;
    let mut stmt = match backend.prepare("select 1 as A, 'x' as B from dual") {
        Ok(stmt) => stmt,
        Err(err) => return h.record("prepare", &err),
    };

    let columns = match backend.execute(&mut stmt, "") {
        Ok(ExecOutcome::ResultSet { columns }) => columns,
        Ok(other) => return h.fail(format!("select 应该是结果集：{other:?}")),
        Err(err) => return h.record("execute", &err),
    };
    h.eq("两列", columns.len(), 2);

    // `columns(&Self::Stmt)` 拿的是缓存那份——`execute` 已经把 `Arc<[ColumnMeta]>` 交出来了。
    match backend.columns(&stmt) {
        Ok(cached) => h.eq("缓存里的列数是同一份", cached.len(), columns.len()),
        Err(err) => h.record("columns", &err),
    }

    let mut buf = RowBuf::new(columns.len());
    match backend.fetch(&mut stmt, &mut buf) {
        Ok(FetchState::Rows { rows }) => {
            h.eq("一行", rows, 1);
            h.eq("行宽等于列数", buf.columns(), columns.len());
        }
        Ok(FetchState::Exhausted) => h.fail("一行都没取到"),
        Ok(other) => h.fail(format!("未知的 FetchState：{other:?}")),
        Err(err) => return h.record("fetch", &err),
    }
    match backend.fetch(&mut stmt, &mut buf) {
        Ok(FetchState::Exhausted) => h.check("第二行就是 Exhausted", true),
        Ok(FetchState::Rows { rows }) => h.fail(format!("又冒出来 {rows} 行")),
        Ok(other) => h.fail(format!("未知的 FetchState：{other:?}")),
        Err(err) => h.record("第二次 fetch", &err),
    }
    if let Err(err) = backend.close_cursor(&mut stmt) {
        h.record("close_cursor", &err);
    }
});

dm_test!(fn backend_transaction_and_reset(h) {
    h.apply_schema();
    let Some(mut backend) = backend(h) else {
        return;
    };

    if let Err(err) = backend.begin() {
        return h.record("begin", &err);
    }
    let mut stmt = match backend.prepare(&format!("insert into {T_TXN} (ID, V) values (?, ?)")) {
        Ok(stmt) => stmt,
        Err(err) => return h.record("prepare", &err),
    };
    let args = [DmValue::Int(1), DmValue::Int(11)];
    if let Err(err) = backend.bind_params(&mut stmt, &args) {
        return h.record("bind_params", &err);
    }
    if let Err(err) = backend.execute(&mut stmt, "") {
        return h.record("execute", &err);
    }
    if let Err(err) = backend.close_cursor(&mut stmt) {
        h.record("close_cursor", &err);
    }
    drop(stmt);

    if let Err(err) = backend.rollback() {
        return h.record("rollback", &err);
    }
    h.expect_scalar(
        "回滚丢掉那一行",
        &format!("select count(*) from {T_TXN}"),
        "0",
    );

    // `reset`：关游标 → 回滚未提交 → 还原 `autocommit=ON`（§5.7）。
    // 这里在**事务中途**调它，验证它确实把连接带回可复用的状态。
    if let Err(err) = backend.begin() {
        return h.record("reset 之前的 begin", &err);
    }
    let mut stmt = match backend.prepare(&format!("insert into {T_TXN} (ID, V) values (?, ?)")) {
        Ok(stmt) => stmt,
        Err(err) => return h.record("reset 之前的 prepare", &err),
    };
    let args = [DmValue::Int(2), DmValue::Int(22)];
    if let Err(err) = backend.bind_params(&mut stmt, &args) {
        return h.record("reset 之前的 bind_params", &err);
    }
    if let Err(err) = backend.execute(&mut stmt, "") {
        return h.record("reset 之前的 execute", &err);
    }
    drop(stmt);

    if let Err(err) = backend.reset() {
        return h.record("reset", &err);
    }
    h.expect_scalar(
        "reset 回滚了未提交的事务",
        &format!("select count(*) from {T_TXN}"),
        "0",
    );

    // `reset` 之后 `autocommit` 应该是 ON：不 begin 直接插入就该立即生效。
    let mut stmt = match backend.prepare(&format!("insert into {T_TXN} (ID, V) values (?, ?)")) {
        Ok(stmt) => stmt,
        Err(err) => return h.record("reset 之后的 prepare", &err),
    };
    let args = [DmValue::Int(3), DmValue::Int(33)];
    if let Err(err) = backend.bind_params(&mut stmt, &args) {
        return h.record("reset 之后的 bind_params", &err);
    }
    if let Err(err) = backend.execute(&mut stmt, "") {
        return h.record("reset 之后的 execute", &err);
    }
    if let Err(err) = backend.close_cursor(&mut stmt) {
        h.record("close_cursor", &err);
    }
    drop(stmt);
    h.expect_scalar(
        "reset 之后 autocommit 已恢复",
        &format!("select count(*) from {T_TXN}"),
        "1",
    );
});

dm_test!(fn backend_commit_persists_and_close_is_confirmatory(h) {
    h.apply_schema();
    let Some(mut backend) = backend(h) else {
        return;
    };

    if let Err(err) = backend.begin() {
        return h.record("begin", &err);
    }
    let mut stmt = match backend.prepare(&format!("insert into {T_TXN} (ID, V) values (?, ?)")) {
        Ok(stmt) => stmt,
        Err(err) => return h.record("prepare", &err),
    };
    let args = [DmValue::Int(1), DmValue::Int(11)];
    if let Err(err) = backend.bind_params(&mut stmt, &args) {
        return h.record("bind_params", &err);
    }
    if let Err(err) = backend.execute(&mut stmt, "") {
        return h.record("execute", &err);
    }
    if let Err(err) = backend.close_cursor(&mut stmt) {
        h.record("close_cursor", &err);
    }
    drop(stmt);

    if let Err(err) = backend.commit() {
        return h.record("commit", &err);
    }

    // 已提交的行**换一条连接**也看得见（夹具的连接是独立的一条）。
    h.expect_scalar(
        "提交之后夹具连接也看得到",
        &format!("select count(*) from {T_TXN}"),
        "1",
    );

    // `close` 之前确认没有活语句——CT-09 把顺序交给调用方，后端能做的就是在close 前喊一声。
    h.eq(
        "关闭前没有活语句",
        backend.connection().live_statement_count(),
        0,
    );
    backend.close();
    h.check("close 是确认式的（没有 panic）", true);
});
