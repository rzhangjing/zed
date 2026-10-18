//! 语句复用：同一句柄反复执行、参数类型变化后复位、`exec_direct` 与 `exec` 交替。
//!
//! T3.3 的验收口径（§12.2）：「同一语句连续两次执行不出状态错」。这里把它扩到三次，
//! 并额外压一压**参数类型变化**与**结果集没取完就重执行**这两条最容易踩出 `-70027` 的路径。
//!
//! 这里统一用 [`OdbcConnection::allocate_statement_owned`] 而不是借用版：借用版语句会
//! 借走连接，而用例要在语句存活期间用夹具记账（`h.eq(..)` 需要 `&mut Harness`）。
//! 拥有版语句不借连接，代价是**必须在连接之前析构**（CT-09）——它们是测试体的局部变量，
//! 先于 `h.finish()` 里的 `close()` 释放。

use dm_core::DmValue;
use dm_odbc::{ParamArena, StmtKind};

use crate::common::{T_STR, T_TXN, cell_text, dm_test};

dm_test!(fn stmt_cache_same_prepared_statement_runs_three_times(h) {
    h.apply_schema();

    let sql = format!("insert into {T_TXN} (ID, V) values (?, ?)");
    let mut arena = ParamArena::new();
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };

    if let Err(err) = stmt.prepare(&sql) {
        return h.record("prepare", &err);
    }
    h.eq("状态是 Prepared", stmt.kind(), StmtKind::Prepared);
    h.eq("SQLNumParams 报 2 个占位符", stmt.param_count(), 2);

    for i in 1..=3 {
        let args = [DmValue::Int(i), DmValue::Int(i * 10)];
        if let Err(err) = dm_odbc::bind_params(&mut stmt, &args, &mut arena) {
            return h.record("bind_params", &err);
        }
        match stmt.exec() {
            Ok(outcome) => {
                h.eq(
                    &format!("第 {i} 次执行影响 1 行"),
                    outcome.affected_rows(),
                    Some(1),
                );
            }
            Err(err) => return h.record(&format!("第 {i} 次 exec"), &err),
        }
    }

    drop(stmt);
    h.expect_scalar(
        "三次执行都落库了",
        &format!("select count(*) from {T_TXN}"),
        "3",
    );
});

dm_test!(fn stmt_cache_parameter_type_change_resets_parameters(h) {
    h.apply_schema();

    // 第 2 个参数的 C 类型在 `Int` 与 `Text` 之间来回变：`param.rs` 检测到类型序列变了就
    // 发一次 `SQLFreeStmt(SQL_RESET_PARAMS)` 并重新绑定。不复位的话驱动会沿用上一次的
    // C 类型去解释新缓冲——那是静默的数据错乱，不是报错。
    let sql = format!("insert into {T_STR} (ID, N) values (?, ?)");
    let mut arena = ParamArena::new();
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.prepare(&sql) {
        return h.record("prepare", &err);
    }

    let cases: [(i32, DmValue); 3] = [
        (1, DmValue::Int(5)),
        (2, DmValue::Text("6".to_owned())),
        (3, DmValue::Int(7)),
    ];
    for (id, value) in cases {
        let args = [DmValue::Int(id), value.clone()];
        if let Err(err) = dm_odbc::bind_params(&mut stmt, &args, &mut arena) {
            return h.record(&format!("绑定 {value:?}"), &err);
        }
        if let Err(err) = stmt.exec() {
            return h.record(&format!("执行 {value:?}"), &err);
        }
    }

    drop(stmt);
    let Some(rows) = h.try_rows(&format!("select N from {T_STR} order by ID")) else {
        return;
    };
    let got: Vec<Option<String>> = rows.iter().map(|r| cell_text(r, 0)).collect();
    h.eq(
        "三种绑定形态都正确落库",
        got,
        vec![
            Some("5".to_owned()),
            Some("6".to_owned()),
            Some("7".to_owned()),
        ],
    );
});

dm_test!(fn stmt_cache_exec_direct_and_exec_alternate_on_one_handle(h) {
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };

    // 一次性下发：不带参数，`param_count` 归零。
    match stmt.exec_direct("select 1 from dual") {
        Ok(outcome) => {
            h.check("exec_direct 产出结果集", outcome.has_result_set());
            h.eq("exec_direct 后参数个数是 0", stmt.param_count(), 0);
            h.eq("状态是 Direct", stmt.kind(), StmtKind::Direct);
        }
        Err(err) => return h.record("exec_direct", &err),
    }

    // 同一个句柄改走预处理。
    if let Err(err) = stmt.prepare("select 2 from dual") {
        return h.record("prepare", &err);
    }
    h.eq("状态回到 Prepared", stmt.kind(), StmtKind::Prepared);
    match stmt.exec() {
        Ok(outcome) => h.check("exec 产出结果集", outcome.has_result_set()),
        Err(err) => return h.record("exec", &err),
    }

    if let Err(err) = stmt.close_cursor() {
        h.record("close_cursor", &err);
    }
    drop(stmt);
});

dm_test!(fn stmt_cache_unconsumed_result_then_reexecute(h) {
    // 真正的 `-70027` 触发条件：**结果集一行没取就再执行**（§6.8）。
    // 本层在每次执行前 `close_cursor_if_open()`，所以这条路径必须干净。
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };

    for _ in 0..3 {
        match stmt.exec_direct("select 1 from dual") {
            // 故意不取行、不显式关游标
            Ok(outcome) => h.check("产出结果集", outcome.has_result_set()),
            Err(err) => return h.record("未取完就重执行", &err),
        }
        h.check("本层认为游标还开着", stmt.is_cursor_open());
    }

    if let Err(err) = stmt.close_cursor() {
        h.record("close_cursor", &err);
    }
    h.check("关掉之后不再是开着", !stmt.is_cursor_open());
    drop(stmt);
});
