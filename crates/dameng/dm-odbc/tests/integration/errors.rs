//! 错误分类（§6.8）：实测到的原生码必须落进「该落的那一类」，没登记的必须原样落回 `Server`。
//!
//! 这些用例的价值不在「错误发生了」（那是必然的），而在于**分类表与真机一致**：
//! `classify_server` 是以原生码为准的硬映射，一旦达梦换了码、或者我们记错了码，
//! 调用方的重试/降级逻辑就会在错误的前提下跑。

use dm_core::DmValue;
use dm_error::{DmError, DmErrorKind};
use dm_odbc::OdbcConnection;

use crate::common::{Harness, T_ERR, T_STR, T_TXN, T_TYPES, describe, dm_test, note_error};

/// 跑一条**注定失败**的语句，记账后把错误交回给调用方断言。
fn failed(h: &mut Harness, label: &str, sql: &str) -> Option<DmError> {
    match h.exec(sql) {
        Ok(outcome) => {
            h.fail(format!("{label}：本该失败却成功了（{outcome:?}）"));
            None
        }
        Err(err) => {
            note_error(&err);
            Some(err)
        }
    }
}

/// 断言错误的分类与原生码。
fn expect_class(
    h: &mut Harness,
    label: &str,
    err: &DmError,
    code: i32,
    check: fn(&DmErrorKind) -> bool,
) {
    h.check_that(check(&err.kind), || {
        format!("{label}：分类不对\n{}", describe(err))
    });
    // 原生码要从**诊断记录**里读：`DmErrorKind::server_code()` 只对「没登记、原样落回
    // `Server`」的错误有值，一旦分类成功，码就被收进语义里了（那是设计如此）。
    h.eq(
        &format!("{label}：原生码"),
        err.server_code_and_message().map(|(code, _)| code),
        Some(code),
    );
    h.check_that(!err.diags().is_empty(), || {
        format!("{label}：失败必须带诊断记录")
    });
    h.check_that(
        err.diags().iter().any(|rec| !rec.sqlstate_str().is_empty()),
        || format!("{label}：诊断里应该有 SQLSTATE"),
    );
}

dm_test!(fn errors_table_not_found_is_object_not_found(h) {
    h.apply_schema();
    let Some(err) = failed(h, "查不存在的表", "select 1 from DM_T_NOT_THERE") else {
        return;
    };
    expect_class(h, "查不存在的表", &err, -2106, |kind| {
        matches!(kind, DmErrorKind::ObjectNotFound)
    });
});

dm_test!(fn errors_column_not_found_is_object_not_found(h) {
    let Some(err) = failed(
        h,
        "查不存在的列",
        "select DM_COLUMN_NOT_THERE from dual",
    ) else {
        return;
    };
    expect_class(h, "查不存在的列", &err, -2111, |kind| {
        matches!(kind, DmErrorKind::ObjectNotFound)
    });
});

dm_test!(fn errors_syntax_error_is_syntax(h) {
    let Some(err) = failed(h, "语法错", "selectt 1 from dual") else {
        return;
    };
    expect_class(h, "语法错", &err, -2007, |kind| {
        matches!(kind, DmErrorKind::Syntax)
    });
});

dm_test!(fn errors_string_truncation_is_encode(h) {
    h.apply_schema();
    let long = "x".repeat(300);
    let argv = [DmValue::Int(1), DmValue::Int(1), DmValue::Text(long), DmValue::Int(1)];
    match h.exec_params(
        &format!("insert into {T_ERR} (ID, N, U, C) values (?, ?, ?, ?)"),
        &argv,
    ) {
        Ok(outcome) => h.fail(format!("超长字符串竟然插进去了（{outcome:?}）")),
        Err(err) => {
            note_error(&err);
            expect_class(h, "超长字符串", &err, -70005, |kind| {
                matches!(kind, DmErrorKind::Encode { .. })
            });
            // 阶段是 **`Execute` 而不是 `Bind`**：真机实测如此，而且这是唯一可能的答案——
            // `SQLBindParameter` 只认 C 类型与缓冲长度，服务端列的宽度对驱动是未知的
            // （一条 `INSERT ... VALUES (?)` 的参数没有列元数据可查），所以「太长」只能等
            // 服务端在 `Execute` 时回 `-70005`。§6.8 的「超长转 `Encode`」说的是**分类**，
            // 不是阶段：我们照样把它归到 `Encode`，但不会假装自己能在绑定时就发现。
            h.eq("阶段是 Execute", err.context.stage, dm_error::Stage::Execute);
        }
    }
});

dm_test!(fn errors_constraint_violations_fall_back_to_server(h) {
    h.apply_schema();
    h.try_exec(&format!("insert into {T_ERR} (ID, N, U, C) values (1, 1, 'u1', 1)"));

    // 主键冲突与唯一约束：真机实测达梦报 `23000 / -6602`（**不是** Oracle 风格的 -2601）。
    // 这一条**故意不登记**——`classify_server` 只认实测登记过的码，别的原样落回 `Server`
    // （宁可让调用方看到生码，也不猜一个可能错的语义）。所以这里断言的就是
    // 「落回 Server 且三元组不改写」。
    let Some(err) = failed(
        h,
        "主键冲突",
        &format!("insert into {T_ERR} (ID, N, U, C) values (1, 2, 'u2', 1)"),
    ) else {
        return;
    };
    expect_class(h, "主键冲突", &err, -6602, |kind| {
        matches!(kind, DmErrorKind::Server { .. })
    });

    let Some(err) = failed(
        h,
        "唯一约束冲突",
        &format!("insert into {T_ERR} (ID, N, U, C) values (2, 2, 'u1', 1)"),
    ) else {
        return;
    };
    expect_class(h, "唯一约束冲突", &err, -6602, |kind| {
        matches!(kind, DmErrorKind::Server { .. })
    });
});

dm_test!(fn errors_not_null_and_check_violations_fall_back_to_server(h) {
    h.apply_schema();

    // `N INT NOT NULL` 传 NULL：`23000 / -6609`（列非空约束），未登记 ⇒ Server。
    let args = [DmValue::Int(1), DmValue::Null, DmValue::Null, DmValue::Null];
    match h.exec_params(
        &format!("insert into {T_ERR} (ID, N, U, C) values (?, ?, ?, ?)"),
        &args,
    ) {
        Ok(outcome) => h.fail(format!("NOT NULL 违例竟然成功（{outcome:?}）")),
        Err(err) => {
            note_error(&err);
            let code = err.server_code_and_message().map(|(code, _)| code);
            h.check_that(
                matches!(code, Some(-6609) | Some(-2601)),
                || format!("期望非空约束的原生码，实际 {code:?}\n{}", describe(&err)),
            );
            h.check_that(matches!(err.kind, DmErrorKind::Server { .. }), || {
                format!("未登记的码必须原样落回 Server\n{}", describe(&err))
            });
        }
    }

    // `C INT CHECK (C > 0)` 传 -1：CHECK 违例，同样未登记。
    let args = [DmValue::Int(2), DmValue::Int(2), DmValue::Text("ok".to_owned()), DmValue::Int(-1)];
    match h.exec_params(
        &format!("insert into {T_ERR} (ID, N, U, C) values (?, ?, ?, ?)"),
        &args,
    ) {
        Ok(outcome) => h.fail(format!("CHECK 违例竟然成功（{outcome:?}）")),
        Err(err) => {
            note_error(&err);
            h.check_that(matches!(err.kind, DmErrorKind::Server { .. }), || {
                format!("未登记的码必须原样落回 Server\n{}", describe(&err))
            });
            h.eq("带诊断", !err.diags().is_empty(), true);
        }
    }
});

dm_test!(fn errors_handle_survives_a_failed_statement(h) {
    h.apply_schema();

    let Some(err) = failed(h, "先制造一个错误", "select 1 from DM_T_NOT_THERE") else {
        return;
    };
    note_error(&err);

    // 同一条连接（以及同一个语句对象）在错误之后必须还能用。
    h.expect_scalar(
        "错误之后连接仍可用",
        "select 1 from dual",
        "1",
    );

    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.exec_direct("select 1 from dual not a table") {
        note_error(&err);
        h.check("同一条语句上先失败一次", true);
    } else {
        h.fail("坏语句竟然成功了");
    }
    // 同一个句柄接着跑正确的语句：这正是 `close_cursor_if_open` + 游标纪律要保住的东西。
    match stmt.exec_direct("select 2 from dual") {
        Ok(outcome) => h.check("同一个句柄随后成功", outcome.has_result_set()),
        Err(err) => h.record("错误之后再执行", &err),
    }
    if let Err(err) = stmt.close_cursor() {
        h.record("close_cursor", &err);
    }

    // 语句里的错误不该污染同一个连接上的下一句。
    h.expect_scalar(
        "另一条语句照常工作",
        &format!("select count(*) from {T_STR}"),
        "0",
    );
});

// §10.3 的「类型不匹配」与「除零」两类：**采到了码，但故意不登记**。
//
// 不登记的理由不是偷懒：`DmErrorKind::DataTypeMismatch { expect, got }` 装的是
// **客户端侧**的「期望什么、拿到了什么」，而服务端只说了句「字符串转换出错」，
// 填不进去（硬填就是编造语义）；除零则根本没有对应变体。所以这几个码原样落回
// `Server`，但它们的 `(SQLSTATE, 原生码, 触发方式)` 进了 `docs/errors-catalog.json`，
// 排障时看得到，升级达梦后也能靠 `examples/errors_probe.rs` 重跑对比。
dm_test!(fn errors_unregistered_conversion_codes_stay_server(h) {
    h.apply_schema();
    let cases = [
        (
            "字符串转换出错",
            format!("insert into {T_STR} (ID, N) values (9001, 'abc')"),
            "22018",
            -6111,
        ),
        (
            "非法的时间日期类型数据",
            format!("insert into {T_TYPES} (ID, C_DATE) values (9002, 'not-a-date')"),
            "22007",
            -6118,
        ),
        (
            "除0错误",
            "select 1/0 from dual".to_owned(),
            "22012",
            -6103,
        ),
    ];
    for (label, sql, sqlstate, code) in cases {
        let Some(err) = failed(h, label, &sql) else {
            continue;
        };
        h.check_that(matches!(err.kind, DmErrorKind::Server { .. }), || {
            format!("{label}：未登记的码必须原样落回 Server\n{}", describe(&err))
        });
        h.eq(
            &format!("{label}：原生码"),
            err.server_code_and_message().map(|(code, _)| code),
            Some(code),
        );
        h.eq(
            &format!("{label}：SQLSTATE"),
            err.diags().first().map(|rec| rec.sqlstate_str().to_owned()),
            Some(sqlstate.to_owned()),
        );
    }
});

// 行锁等待失败（`HY000 / -6409 锁等待`）→ `DmErrorKind::Timeout`。
//
// §10.3 把「锁超时」列为一类必须采集的错误。这里用 `FOR UPDATE NOWAIT` 触发而不是
// 真等超时：`NOWAIT` 是**确定性**的，等超时会让用例挂上几分钟，而两条路径在达梦上
// 报的是同一个码。持锁与试锁必须在**两条连接**上做——ODBC 的语句会可变借走连接，
// 一条连接上开不出两条同时活着的语句。
fn lock_wait_error(h: &mut crate::common::Harness) -> Option<DmError> {
    let cfg = crate::common::test_config()?;
    let mut holder = match OdbcConnection::connect(&cfg) {
        Ok(conn) => conn,
        Err(err) => {
            h.record("持锁方连接", &err);
            return None;
        }
    };
    let mut waiter = match OdbcConnection::connect(&cfg) {
        Ok(conn) => conn,
        Err(err) => {
            h.record("等待方连接", &err);
            return None;
        }
    };

    // 准备数据用等待方连接（它稍后才试锁，顺序使用，借用不重叠）。
    for sql in [
        format!("delete from {T_TXN} where ID = 1"),
        format!("insert into {T_TXN} (ID, V) values (1, 0)"),
    ] {
        if let Err(err) = waiter
            .allocate_statement()
            .and_then(|mut stmt| stmt.exec_direct(&sql))
        {
            h.record("准备数据", &err);
            return None;
        }
    }

    if let Err(err) = holder.begin() {
        h.record("持锁方 begin", &err);
        return None;
    }

    // 持锁：语句对象要活到 NOWAIT 侧跑完。达梦的行锁跟着**事务**走（关游标不一定放锁，
    // 探针实测过），但让语句活到那一刻才是「持锁方还在等」该有的样子，也免得依赖
    // 驱动的释放时机。
    let mut held = match holder.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            h.record("持锁方申请语句", &err);
            return None;
        }
    };
    if let Err(err) = held.exec_direct(&format!("select * from {T_TXN} where ID = 1 for update")) {
        h.record("持锁语句", &err);
        // 未提交的事务随连接析构由服务端回滚（这里不显式 rollback，免得与 `held`
        // 的可变借用打架）。
        return None;
    }

    let outcome = waiter.allocate_statement().and_then(|mut stmt| {
        stmt.exec_direct(&format!(
            "select * from {T_TXN} where ID = 1 for update nowait"
        ))
    });

    // 放锁 → 回滚 → 清行，别把状态留给下一个用例。
    drop(held);
    if let Err(err) = holder.rollback() {
        h.record("持锁方 rollback", &err);
    }
    let _ = waiter
        .allocate_statement()
        .and_then(|mut stmt| stmt.exec_direct(&format!("delete from {T_TXN} where ID = 1")));

    match outcome {
        Ok(outcome) => {
            h.fail(format!(
                "`for update nowait` 竟然成功（{outcome:?}）——那这条就没测到锁冲突"
            ));
            None
        }
        Err(err) => Some(err),
    }
}

dm_test!(fn errors_lock_wait_is_timeout(h) {
    h.apply_schema();
    if let Some(err) = lock_wait_error(h) {
        note_error(&err);
        expect_class(h, "行锁等待失败", &err, -6409, |kind| {
            matches!(kind, DmErrorKind::Timeout)
        });
    }
});
