//! 参数绑定的一致性：D8 纪律的五类取值、用法错误的提前拒绝、以及稳态零分配。
//!
//! 这里刻意用 [`OdbcConnection::allocate_statement_owned`]（拥有版语句），因为好几条用例要在
//! 「语句还活着」的时候用夹具记账。拥有版语句不借连接，代价是必须在连接之前析构（CT-09）。

use std::sync::Arc;

use dm_core::{DmValue, RowBuf};
use dm_error::DmErrorKind;
use dm_odbc::{ParamArena, bind_params};

use crate::common::{T_LOB, T_STR, cell_canonical, cell_text, describe, dm_test, hex, note_error};

/// 取一个拥有版语句（生命周期与连接解耦，便于在语句存活期间记账）。
fn owned(h: &mut crate::common::Harness) -> Option<dm_odbc::OdbcStatement<'static>> {
    match h.conn.allocate_statement_owned() {
        Ok(stmt) => Some(stmt),
        Err(err) => {
            h.record("SQLAllocHandle", &err);
            None
        }
    }
}

dm_test!(fn params_null_empty_chinese_binary_and_zero(h) {
    h.apply_schema();
    let mut arena = ParamArena::new();

    // 五类取值：NULL / 空串 / 中文 / 含 NUL 的二进制 / 零值。
    let cases: [(&str, Vec<DmValue>); 5] = [
        (
            "全 NULL",
            vec![DmValue::Int(1), DmValue::Null, DmValue::Null, DmValue::Null],
        ),
        (
            "空串与空二进制",
            vec![
                DmValue::Int(2),
                DmValue::Text(String::new()),
                DmValue::Bytes(Vec::new()),
                DmValue::Int(0),
            ],
        ),
        (
            "中文",
            vec![
                DmValue::Int(3),
                DmValue::Text("中文测试：达梦".to_owned()),
                DmValue::Bytes(vec![0x0A, 0x0B]),
                DmValue::Int(-1),
            ],
        ),
        (
            "含 NUL 的二进制",
            vec![
                DmValue::Int(4),
                DmValue::Text("x".to_owned()),
                DmValue::Bytes(vec![0x00, 0x01, 0x00, 0xFF]),
                DmValue::Int(7),
            ],
        ),
        (
            "整数边界",
            vec![
                DmValue::Int(5),
                DmValue::Null,
                DmValue::Null,
                DmValue::Int(i32::MIN),
            ],
        ),
    ];

    for (label, args) in &cases {
        let Some(mut stmt) = owned(h) else { return };
        if let Err(err) = stmt.prepare(&format!("insert into {T_STR} (ID, S, B, N) values (?, ?, ?, ?)"))
        {
            return h.record(&format!("{label} · prepare"), &err);
        }
        if let Err(err) = bind_params(&mut stmt, args, &mut arena) {
            return h.record(&format!("{label} · bind_params"), &err);
        }
        match stmt.exec() {
            Ok(outcome) => h.eq(
                &format!("{label} · 影响 1 行"),
                outcome.affected_rows(),
                Some(1),
            ),
            Err(err) => return h.record(&format!("{label} · exec"), &err),
        }
    }

    let Some(rows) = h.try_rows(&format!("select ID, S, B, N from {T_STR} order by ID")) else {
        return;
    };
    h.eq("五行都在", rows.len(), 5);

    let row = |rows: &Vec<Vec<DmValue>>, index: usize| rows.get(index).cloned().unwrap_or_default();
    let r1 = row(&rows, 0);
    h.check(
        "第 1 行是 NULL 而不是空串",
        matches!(r1.get(1), Some(DmValue::Null)) && matches!(r1.get(2), Some(DmValue::Null)),
    );
    let r2 = row(&rows, 1);
    h.check(
        "第 2 行是空串/空二进制而不是 NULL",
        matches!(r2.get(1), Some(DmValue::Text(s)) if s.is_empty())
            && matches!(r2.get(2), Some(DmValue::Bytes(b)) if b.is_empty()),
    );
    let r3 = row(&rows, 2);
    h.eq(
        "第 3 行中文原样",
        r3.get(1).and_then(|v| cell_text(std::slice::from_ref(v), 0)),
        Some("中文测试：达梦".to_owned()),
    );
    let r4 = row(&rows, 3);
    h.eq(
        "第 4 行含 NUL 的二进制原样",
        r4.get(2).and_then(|v| match v {
            DmValue::Bytes(b) => Some(hex(b)),
            _ => None,
        }),
        Some("000100FF".to_owned()),
    );
    let r5 = row(&rows, 4);
    h.eq(
        "第 5 行整数下界原样",
        r5.get(3).and_then(|v| match v {
            DmValue::Int(n) => Some(*n),
            _ => None,
        }),
        Some(i32::MIN),
    );
    h.eq(
        "第 5 行文本列是 NULL",
        cell_canonical(&r5, 1),
        "<NULL>".to_owned(),
    );
});

dm_test!(fn params_text_beyond_the_probe_chunk(h) {
    h.apply_schema();

    // 5000 个字符 = 10 KiB，远超 4 KiB 的分块试探边界；绑定侧的 `buf_len` 必须按
    // 实际写进去的 UTF-16 字节数报，短报会被驱动截断、长报会读到缓冲后面。
    let text = "达梦".repeat(2500);
    let args = [DmValue::Int(1), DmValue::Text(text.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );

    let Some(rows) = h.try_rows(&format!("select C from {T_LOB} where ID = 1")) else {
        return;
    };
    h.eq(
        "超长文本完整落库",
        rows.first().and_then(|r| cell_text(r, 0)),
        Some(text),
    );
    h.expect_scalar(
        "服务端 length() 数字符",
        &format!("select length(C) from {T_LOB} where ID = 1"),
        "5000",
    );
});

dm_test!(fn params_too_few_bindings_are_refused_locally(h) {
    // 本层提前拒绝：`args.len() != param_count` 是**调用方的用法错误**，
    // 不该把服务端的 `07002/-70039` 留给用户去猜（§6.3）。
    let mut stmt = match owned(h) {
        Some(stmt) => stmt,
        None => return,
    };
    if let Err(err) = stmt.prepare(&format!("insert into {T_STR} (ID, S) values (?, ?)")) {
        return h.record("prepare", &err);
    }
    h.eq("驱动报 2 个占位符", stmt.param_count(), 2);

    let mut arena = ParamArena::new();
    let args = [DmValue::Int(1)];
    match bind_params(&mut stmt, &args, &mut arena) {
        Ok(()) => h.fail("只绑一个参数竟然通过了"),
        Err(err) => {
            note_error(&err);
            h.eq("是配置/用法错误", err.context.stage, dm_error::Stage::Bind);
            h.check_that(matches!(err.kind, DmErrorKind::Config(_)), || {
                format!("期望 Config，实际 {}", describe(&err))
            });
        }
    }
});

dm_test!(fn params_unbound_execution_reports_the_server_code(h) {
    // 绕过本层守卫直接执行（等价于「调用方自己调了 SQLExecute 却没绑满」）：
    // 达梦回 `07002 / -70039 绑定的参数个数不正确`，走 `SQL_ERROR` 路径。
    // 这条码**故意不登记**进分类表（它是调用方用法错误，不是服务端语义），
    // 所以断言的是「原样落回 Server 且三元组不改写」。
    let mut stmt = match owned(h) {
        Some(stmt) => stmt,
        None => return,
    };
    if let Err(err) = stmt.prepare(&format!("insert into {T_STR} (ID, S) values (?, ?)")) {
        return h.record("prepare", &err);
    }
    match stmt.exec() {
        Ok(outcome) => h.fail(format!("没绑参数竟然执行成功：{outcome:?}")),
        Err(err) => {
            note_error(&err);
            let code = err.kind.server_code().map(|(_, code)| code);
            h.eq("原生码是 -70039", code, Some(-70039));
            h.check_that(matches!(err.kind, DmErrorKind::Server { .. }), || {
                format!("期望原样落回 Server，实际 {}", describe(&err))
            });
            h.check("带着诊断记录", !err.diags().is_empty());
        }
    }
});

dm_test!(fn params_exec_direct_with_placeholder_reports_the_server_code(h) {
    // 一次性下发 + `?` 占位符：达梦回 `HY000 / -6804 需要更多的参数`（不是 `SQL_NEED_DATA`）。
    // 同样故意不登记；本层另有守卫（`exec_direct` 不接受参数），这里走的是服务端路径。
    let mut stmt = match owned(h) {
        Some(stmt) => stmt,
        None => return,
    };
    match stmt.exec_direct(&format!("insert into {T_STR} (ID, S) values (?, ?)")) {
        Ok(outcome) => h.fail(format!("带占位符的 exec_direct 竟然成功：{outcome:?}")),
        Err(err) => {
            note_error(&err);
            let code = err.kind.server_code().map(|(_, code)| code);
            h.eq("原生码是 -6804", code, Some(-6804));
            h.check_that(matches!(err.kind, DmErrorKind::Server { .. }), || {
                format!("期望原样落回 Server，实际 {}", describe(&err))
            });
        }
    }
});

dm_test!(fn params_binding_on_a_fresh_statement_is_refused(h) {
    let mut stmt = match owned(h) {
        Some(stmt) => stmt,
        None => return,
    };
    let mut arena = ParamArena::new();
    let args = [DmValue::Int(1)];
    match bind_params(&mut stmt, &args, &mut arena) {
        Ok(()) => h.fail("在没预处理过的语句上绑定竟然通过了"),
        Err(err) => {
            note_error(&err);
            h.check_that(matches!(err.kind, DmErrorKind::Config(_)), || {
                format!("期望 Config，实际 {}", describe(&err))
            });
            h.eq("阶段是 Bind", err.context.stage, dm_error::Stage::Bind);
            let text = err.to_string();
            h.check_that(
                text.contains("prepare") || text.contains("exec_direct"),
                || format!("错误文本要给出可行动的下一步，实际：{text}"),
            );
        }
    }
});

dm_test!(fn params_unsupported_value_type_is_refused(h) {
    let mut stmt = match owned(h) {
        Some(stmt) => stmt,
        None => return,
    };
    if let Err(err) = stmt.prepare(&format!("insert into {T_STR} (ID, S) values (?, ?)")) {
        return h.record("prepare", &err);
    }
    let mut arena = ParamArena::new();
    // 未知类型码的原始字节：本层不猜它该用哪个 C 类型，直接拒绝（§7 兜底纪律：不静默降级）。
    let args = [DmValue::Int(1), DmValue::Unknown(vec![0x01, 0x02])];
    match bind_params(&mut stmt, &args, &mut arena) {
        Ok(()) => h.fail("未知类型码竟然绑定成功了"),
        Err(err) => {
            note_error(&err);
            h.check_that(matches!(err.kind, DmErrorKind::Unsupported { .. }), || {
                format!("期望 Unsupported，实际 {}", describe(&err))
            });
            h.eq("阶段是 Bind", err.context.stage, dm_error::Stage::Bind);
        }
    }
});

dm_test!(fn params_arena_reaches_steady_state_without_growth(h) {
    h.apply_schema();
    let sql = format!("insert into {T_STR} (ID, S, B, N) values (?, ?, ?, ?)");
    let mut stmt = match owned(h) {
        Some(stmt) => stmt,
        None => return,
    };
    if let Err(err) = stmt.prepare(&sql) {
        return h.record("prepare", &err);
    }

    let mut arena = ParamArena::new();
    let mut capacities: Vec<usize> = Vec::new();
    for i in 0..40 {
        // **故意用定宽值**：`第{i:04}行` 恒为 6 个字符、二进制恒为 8 字节，于是「只要缓冲
        // 够大就一次都不用再长」这件事才是个真命题。早先用 `第 {i} 行` 会随位数变长，
        // arena 跟着长到 388 字节——那是**正确行为**（缓冲必须放得下当前最宽的值），
        // 拿它当「没有稳态」来判失败是判错了。
        let args = [
            DmValue::Int(1000 + i),
            DmValue::Text(format!("第{i:04}行")),
            DmValue::Bytes(vec![(i % 256) as u8; 8]),
            DmValue::Int(i),
        ];
        if let Err(err) = bind_params(&mut stmt, &args, &mut arena) {
            return h.record(&format!("第 {i} 次绑定"), &err);
        }
        if let Err(err) = stmt.exec() {
            return h.record(&format!("第 {i} 次执行"), &err);
        }
        capacities.push(arena.capacity());
    }

    // 第一次绑定长到放下这一批参数，之后就**一次都不许再长**：稳态里再分配会让已经交给
    // 驱动的指针与缓冲分家（`ParamArena` 的整条设计就建立在这上面）。
    let baseline = capacities.first().copied().unwrap_or_default();
    h.eq(
        "第一次之后的每次绑定容量都不变",
        capacities.iter().skip(1).copied().collect::<Vec<_>>(),
        vec![baseline; capacities.len().saturating_sub(1)],
    );
    h.check_that(baseline > 0, || "参数缓冲容量居然为 0".to_owned());
    eprintln!(
        "[note] arena 容量序列：{:?} → 稳态 {baseline} 字节",
        &capacities[..capacities.len().min(6)]
    );

    if let Err(err) = stmt.close_cursor() {
        h.record("close_cursor", &err);
    }
});

dm_test!(fn params_rowbuf_reads_what_we_wrote(h) {
    // 写进去、读回来，中间只用本层的两条路径（`bind_params` / `fetch`），
    // 用来确认参数缓冲的布局（值 + 8 字节指标）没有互相踩。
    h.apply_schema();
    let text = "边界：中文与空串混合".repeat(3);
    let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let args = [
        DmValue::Int(1),
        DmValue::Text(text.clone()),
        DmValue::Bytes(bytes.clone()),
        DmValue::Int(-42),
    ];
    h.try_exec_params(
        &format!("insert into {T_STR} (ID, S, B, N) values (?, ?, ?, ?)"),
        &args,
    );

    let Some(mut stmt) = owned(h) else { return };
    if let Err(err) = stmt.exec_direct(&format!("select S, B, N from {T_STR} where ID = 1")) {
        return h.record("exec_direct", &err);
    }
    let length_in_char = h.length_in_char;
    let columns = match dm_odbc::describe_columns(&mut stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns", &err),
    };
    let columns: Arc<[dm_core::ColumnMeta]> = columns;
    let mut buf = RowBuf::new(columns.len());
    let mut scratch = dm_odbc::FetchScratch::new();
    match dm_odbc::fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
        Ok(dm_core::backend::FetchState::Rows { .. }) => {
            let Some(row) = buf.row(0) else {
                return h.fail("取回了行但缓冲区里是空的");
            };
            h.eq(
                "文本往返",
                row.first().and_then(|v| cell_text(std::slice::from_ref(v), 0)),
                Some(text),
            );
            h.eq(
                "二进制往返",
                row.get(1).and_then(|v| match v {
                    DmValue::Bytes(b) => Some(hex(b)),
                    _ => None,
                }),
                Some(hex(&bytes)),
            );
            h.eq(
                "整数往返",
                row.get(2).and_then(|v| match v {
                    DmValue::Int(n) => Some(*n),
                    _ => None,
                }),
                Some(-42),
            );
        }
        Ok(dm_core::backend::FetchState::Exhausted) => h.fail("一行都没取到"),
        Ok(other) => h.fail(format!("未知的 FetchState：{other:?}")),
        Err(err) => h.record("fetch", &err),
    }
});
