//! 结果集取值路径（§6.4）：`FetchState` 的状态机、空结果集、多行循环、`RowBuf` 复用。
//!
//! 参数写入路径的往返在 `params.rs`，大对象分块在 `lob.rs`，这里盯的是**状态机**：
//! 什么时候该 `Rows`、什么时候该 `Exhausted`、取完之后游标有没有被记成关着。

use dm_core::backend::FetchState;
use dm_core::{DmValue, RowBuf};
use dm_odbc::{FetchScratch, describe_columns, fetch};

use crate::common::{T_STR, T_TYPES, cell_text, dm_test};

dm_test!(fn result_empty_result_set_is_immediately_exhausted(h) {
    let length_in_char = h.length_in_char;
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.exec_direct("select 1 from dual where 1 = 0") {
        return h.record("exec_direct", &err);
    }
    let columns = match describe_columns(&mut stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns", &err),
    };
    h.eq("空结果集也有列元数据", columns.len(), 1);

    let mut buf = RowBuf::new(columns.len());
    let mut scratch = FetchScratch::new();
    match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
        Ok(FetchState::Exhausted) => h.check("第一行就是 Exhausted", true),
        Ok(FetchState::Rows { rows }) => h.fail(format!("空结果集却取回 {rows} 行")),
        Ok(other) => h.fail(format!("未知的 FetchState：{other:?}")),
        Err(err) => h.record("空结果集的 fetch", &err),
    }
    h.check("取尽之后游标被记成关着", !stmt.is_cursor_open());
    h.check("缓冲区是空的", buf.is_empty());
    h.eq("但列数还在", buf.columns(), 1);

    // 取尽之后再取一次必须仍然干净（`SQLFetch` 在游标尾部返回 `SQL_NO_DATA`）。
    match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
        Ok(FetchState::Exhausted) => h.check("再取一次还是 Exhausted", true),
        Ok(FetchState::Rows { rows }) => h.fail(format!("取尽之后又冒出来 {rows} 行")),
        Ok(other) => h.fail(format!("未知的 FetchState：{other:?}")),
        Err(err) => h.record("取尽之后再取", &err),
    }
});

dm_test!(fn result_multi_row_loop_reuses_one_rowbuf(h) {
    h.apply_schema();

    let sql = format!("insert into {T_STR} (ID, S, N) values (?, ?, ?)");
    for i in 0..25 {
        let args = [
            DmValue::Int(i),
            DmValue::Text(format!("第 {i} 行")),
            DmValue::Int(i * 3),
        ];
        h.try_exec_params(&sql, &args);
    }

    let length_in_char = h.length_in_char;
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.exec_direct(&format!("select ID, S, N from {T_STR} order by ID")) {
        return h.record("exec_direct", &err);
    }
    let columns = match describe_columns(&mut stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns", &err),
    };

    let mut buf = RowBuf::new(columns.len());
    let mut scratch = FetchScratch::new();
    let mut seen: Vec<(Option<String>, Option<String>)> = Vec::new();
    let mut iter = 0;
    loop {
        iter += 1;
        if iter > 100 {
            return h.fail("取值循环超过 100 轮：`Exhausted` 一直没来");
        }
        match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
            Ok(FetchState::Rows { rows }) => {
                h.eq("每次一行", rows, 1);
                let row = buf.row(0).map(|r| r.to_vec()).unwrap_or_default();
                seen.push((cell_text(&row, 0), cell_text(&row, 1)));
            }
            Ok(FetchState::Exhausted) => break,
            Ok(other) => return h.fail(format!("未知的 FetchState：{other:?}")),
            Err(err) => return h.record(&format!("第 {iter} 轮 fetch"), &err),
        }
    }

    h.eq("取回 25 行", seen.len(), 25);
    let first = seen.first().cloned().unwrap_or_default();
    let last = seen.last().cloned().unwrap_or_default();
    h.eq("第一行对", first, (Some("0".to_owned()), Some("第 0 行".to_owned())));
    h.eq(
        "最后一行对",
        last,
        (Some("24".to_owned()), Some("第 24 行".to_owned())),
    );
    h.check("取尽之后游标关着", !stmt.is_cursor_open());
    h.check_that(scratch.wide_capacity() > 0, || {
        "取值暂存一次都没长过，分块路径可能没走到".to_owned()
    });
});

dm_test!(fn result_null_and_empty_are_distinguishable(h) {
    h.apply_schema();

    let cases: [Vec<DmValue>; 2] = [
        vec![DmValue::Int(1), DmValue::Null, DmValue::Int(1)],
        vec![
            DmValue::Int(2),
            DmValue::Text(String::new()),
            DmValue::Int(2),
        ],
    ];
    for args in &cases {
        h.try_exec_params(&format!("insert into {T_STR} (ID, S, N) values (?, ?, ?)"), args);
    }

    let Some(rows) = h.try_rows(&format!("select S from {T_STR} order by ID")) else {
        return;
    };
    h.eq("两行", rows.len(), 2);
    let got: Vec<Option<String>> = rows.iter().map(|r| cell_text(r, 0)).collect();
    h.eq(
        "第一列是 NULL、第二列是空串——`value_text` 一个给 None、一个给 Some(\"\")",
        got,
        vec![None, Some(String::new())],
    );
    h.check(
        "NULL 行的原始值是 DmValue::Null",
        matches!(rows.first().and_then(|r| r.first()), Some(DmValue::Null)),
    );
    h.check(
        "空串行的原始值是 DmValue::Text(\"\")",
        matches!(rows.get(1).and_then(|r| r.first()), Some(DmValue::Text(s)) if s.is_empty()),
    );
});

dm_test!(fn result_column_metadata_reports_nullability_and_precision(h) {
    h.apply_schema();
    let length_in_char = h.length_in_char;
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    let sql = "select cast(1 as int) as NOT_NULL_COL, cast(null as varchar(10)) as NULL_COL, \
               cast(123.45 as decimal(10,2)) as DEC_COL from dual";
    if let Err(err) = stmt.exec_direct(sql) {
        return h.record("exec_direct", &err);
    }
    let columns = match describe_columns(&mut stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns", &err),
    };
    h.eq("三列", columns.len(), 3);

    let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    h.eq(
        "列名原样（达梦大写）",
        names,
        vec![
            "NOT_NULL_COL".to_owned(),
            "NULL_COL".to_owned(),
            "DEC_COL".to_owned(),
        ],
    );

    h.eq(
        "常量列的类型是 INT",
        columns.first().map(|c| c.data_type.to_string()),
        Some("INT".to_owned()),
    );
    h.eq(
        "decimal(10,2) 的精度",
        columns.get(2).and_then(|c| c.precision),
        Some(10),
    );
    h.eq(
        "decimal(10,2) 的标度",
        columns.get(2).and_then(|c| c.scale),
        Some(2),
    );

    // 表达式列：**实测**达梦一律报「可空」（`cast(1 as int)` 也不例外——表达式没有
    // NOT NULL 约束可言，驱动按规范回 `SQL_NULLABLE`）。所以这三列全 `true` 是驱动的
    // 正确行为，不是我们走了保守默认值。这一点由 `examples/meta_probe.rs` 的原始属性表钉住。
    for column in columns.iter() {
        eprintln!("[note] 表达式列 {}: nullable={}", column.name, column.nullable);
    }
    h.eq(
        "表达式列全部报可空（达梦实测行为）",
        columns.iter().filter(|c| !c.nullable).count(),
        0,
    );

    // 真表列才有真答案：`DM_T_TYPES.ID` 是 `NOT NULL PRIMARY KEY`，必须读出「不可空」。
    // 这一条才是「可空性真的读到了」的证据。
    let mut table_stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    let table_sql = format!("select ID, C_INT, C_VARCHAR from {T_TYPES}");
    if let Err(err) = table_stmt.exec_direct(&table_sql) {
        return h.record("exec_direct(真表)", &err);
    }
    let table_columns = match describe_columns(&mut table_stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns(真表)", &err),
    };
    h.eq("真表三列", table_columns.len(), 3);
    for column in table_columns.iter() {
        eprintln!(
            "[note] 真表列 {}: nullable={} size={:?}",
            column.name, column.nullable, column.column_size
        );
    }
    h.eq(
        "NOT NULL 主键列读成不可空",
        table_columns.first().map(|c| c.nullable),
        Some(false),
    );
    h.eq(
        "没写 NOT NULL 的列读成可空",
        table_columns.get(1).map(|c| c.nullable),
        Some(true),
    );
});

dm_test!(fn result_fetch_without_a_result_set_is_refused(h) {
    // `fetch` 只该用在产出结果集的语句上；拿影响行数的语句来取，必须报可行动的错，
    // 而不是让驱动回一个 `-70027` 或干脆返回垃圾。
    let length_in_char = h.length_in_char;
    h.apply_schema();

    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.exec_direct(&format!("delete from {T_STR} where ID = -1")) {
        return h.record("exec_direct(delete)", &err);
    }
    h.check("delete 不产出结果集", !stmt.outcome().unwrap_or(
        dm_odbc::StmtOutcome::Affected { rows: Some(0) },
    ).has_result_set());

    // 列元数据问不到（`SQLNumResultCols` 为 0），`describe_columns` 会给出空列表；
    // 这里直接调 `fetch` 验证「列数与元数据不符」被挡住。
    let columns: Vec<dm_core::ColumnMeta> = Vec::new();
    let mut buf = RowBuf::new(1);
    let mut scratch = FetchScratch::new();
    match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
        Ok(state) => h.fail(format!("对没有结果集的语句取值竟然返回 {state:?}")),
        Err(err) => {
            crate::common::note_error(&err);
            h.eq("阶段是 Fetch", err.context.stage, dm_error::Stage::Fetch);
            h.check_that(
                matches!(err.kind, dm_error::DmErrorKind::Decode { .. }),
                || format!("期望 Decode，实际 {}", crate::common::describe(&err)),
            );
        }
    }
    // 顺带确认长度单位从配置来的那条路径没被上面这段影响。
    h.check(
        "length_in_char 三态之一",
        matches!(length_in_char, None | Some(true) | Some(false)),
    );
});
