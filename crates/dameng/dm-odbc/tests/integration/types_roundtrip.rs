//! §7 类型表的往返验收：**每种类型 × 三态**（边界值 / 全 NULL / 空与零），
//! 外加列元数据与服务端侧三方比对。这是 G3 门禁的主体。
//!
//! 比较用 [`canonical`]（规范化文本），规则写在它的文档里。表驱动的好处是：新增一列时
//! 只需在 `COLUMNS`、插入参数、期望值三处各加一行，失败信息会直接指出是哪一列。

use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime, Timelike as _};
use dm_core::{ColumnMeta, DmValue};

use dm_odbc::describe_columns;

use crate::common::{Harness, T_TYPES, canonical, cell_canonical, cell_text, dm_test};

/// `DM_T_TYPES` 的列序（与 `fixtures/schema.sql` 一致）。
const COLUMNS: [&str; 19] = [
    "ID",
    "C_CHAR",
    "C_VARCHAR",
    "C_CLOB",
    "C_TINYINT",
    "C_SMALLINT",
    "C_INT",
    "C_BIGINT",
    "C_DECIMAL",
    "C_NUMERIC",
    "C_REAL",
    "C_DOUBLE",
    "C_BIT",
    "C_DATE",
    "C_TIME",
    "C_TIMESTAMP",
    "C_VARBINARY",
    "C_BINARY",
    "C_BLOB",
];

/// 19 个占位符的插入语句，列序与 `COLUMNS` 一致。
fn insert_sql() -> String {
    let placeholders = std::iter::repeat_n("?", COLUMNS.len())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "insert into {T_TYPES} ({}) values ({placeholders})",
        COLUMNS.join(", ")
    )
}

/// 全列查询，列序与 `COLUMNS` 一致。
fn select_sql() -> String {
    format!("select {} from {T_TYPES} order by ID", COLUMNS.join(", "))
}

/// 第 1 行的边界值（每种类型的极值或代表值）。
fn boundary_args() -> Vec<DmValue> {
    let date = NaiveDate::from_ymd_opt(1999, 12, 31).expect("合法日期");
    let time = NaiveTime::from_hms_opt(23, 59, 59).expect("合法时间");
    let timestamp = date
        .and_time(time)
        .with_nanosecond(123_456_000)
        .expect("合法纳秒");
    vec![
        DmValue::Int(1),
        DmValue::Text("abc".to_owned()),
        DmValue::Text("varchar-值".to_owned()),
        DmValue::Text("达梦".repeat(100)),
        DmValue::TinyInt(i8::MIN),
        DmValue::SmallInt(i16::MIN),
        DmValue::Int(i32::MIN),
        DmValue::BigInt(i64::MIN),
        DmValue::Decimal("9999999999999999.9999".parse().expect("合法定点数")),
        DmValue::Decimal("-12345678.90".parse().expect("合法定点数")),
        DmValue::Real(1.5),
        DmValue::Double(3.25),
        DmValue::Bool(true),
        DmValue::Date(date),
        DmValue::Time(time),
        DmValue::Timestamp(timestamp),
        DmValue::Bytes(vec![0x00, 0x0A, 0xFF]),
        DmValue::Bytes(vec![0x01; 8]),
        DmValue::Bytes(vec![0xAA; 64]),
    ]
}

/// 逐列比对规范化文本。
fn compare(h: &mut Harness, label: &str, row: &[DmValue], want: &[&str]) {
    for (index, expected) in want.iter().enumerate() {
        let got = cell_canonical(row, index);
        let name = COLUMNS.get(index).copied().unwrap_or("<越界>");
        h.check_that(got == *expected, || {
            format!("{label} · {name}：got {got:?}, want {expected:?}")
        });
    }
}

dm_test!(fn types_roundtrip_boundary_row(h) {
    h.apply_schema();
    let args = boundary_args();
    h.try_exec_params(&insert_sql(), &args);

    let Some(rows) = h.try_rows(&select_sql()) else {
        return;
    };
    h.eq("一行", rows.len(), 1);
    let Some(row) = rows.first() else { return };

    compare(
        h,
        "边界值行",
        row,
        &[
            "1",
            "abc",              // CHAR(10)：服务端补空格，规范化时去掉
            "varchar-值",
            &"达梦".repeat(100), // CLOB
            "-128",
            "-32768",
            "-2147483648",
            "-9223372036854775808",
            "9999999999999999.9999",
            "-12345678.9", // NUMERIC(10,2)：最简写法
            "1.5",
            "3.25",
            "1", // BIT 的宽字符路径取回来是 BOOLEAN 语义
            "1999-12-31",
            "23:59:59",
            // `boundary_args` 里 `C_TIMESTAMP = C_DATE.and_time(C_TIME)`（千年末），
            // 期望值必须跟着**输入**走——这里先前留着上一版输入（2024-03-04 05:06:07）
            // 的旧串，是纯粹的陈旧期望，不是服务端行为差异。
            "1999-12-31 23:59:59.123456",
            "000AFF",
            "0101010101010101",
            &"AA".repeat(64),
        ],
    );
});

dm_test!(fn types_roundtrip_all_null_row(h) {
    h.apply_schema();

    let mut args = vec![DmValue::Null; COLUMNS.len()];
    args[0] = DmValue::Int(2);
    h.try_exec_params(&insert_sql(), &args);

    let Some(rows) = h.try_rows(&select_sql()) else {
        return;
    };
    let Some(row) = rows.first() else {
        return h.fail("没有取回全 NULL 行");
    };

    let want = std::iter::once("2")
        .chain(std::iter::repeat_n("<NULL>", COLUMNS.len() - 1))
        .collect::<Vec<_>>();
    compare(h, "全 NULL 行", row, &want);
});

dm_test!(fn types_roundtrip_empty_and_zero_row(h) {
    h.apply_schema();

    // 日期时间列在这里留 NULL：达梦没有「空日期」这种值，§4 也拒绝了零日期。
    // 其余列都取「空 / 零」形态，专门压 `indicator == 0`（空串）与 `indicator == -1`（NULL）
    // 的分界——这两种在缓冲区里都是空的，只有指标能区分。
    let args = vec![
        DmValue::Int(3),
        DmValue::Text(String::new()),  // C_CHAR → 10 个空格
        DmValue::Text(String::new()),  // C_VARCHAR
        DmValue::Text(String::new()),  // C_CLOB
        DmValue::TinyInt(0),
        DmValue::SmallInt(0),
        DmValue::Int(0),
        DmValue::BigInt(0),
        DmValue::Decimal("0".parse().expect("合法定点数")),
        DmValue::Decimal("0".parse().expect("合法定点数")),
        DmValue::Real(0.0),
        DmValue::Double(0.0),
        DmValue::Bool(false),
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Bytes(Vec::new()),
        DmValue::Bytes(vec![0x00; 8]),
        DmValue::Bytes(Vec::new()),
    ];
    h.try_exec_params(&insert_sql(), &args);

    let Some(rows) = h.try_rows(&select_sql()) else {
        return;
    };
    let Some(row) = rows.first() else {
        return h.fail("没有取回空/零行");
    };

    compare(
        h,
        "空与零行",
        row,
        &[
            "3",
            "", // CHAR(10) 补空格 ⇒ 规范化后为空
            "",
            "",
            "0",
            "0",
            "0",
            "0",
            "0",
            "0",
            "0",
            "0",
            "0",
            "<NULL>",
            "<NULL>",
            "<NULL>",
            "",
            "0000000000000000", // BINARY(8) 定长：写入 0 字节也补满 8 个零
            "",
        ],
    );

    // 再确认一次「空串不是 NULL」：这三列必须是 `DmValue::Text("")` 而**不是** `Null`。
    h.check(
        "CHAR 空串取回来是空文本而不是 NULL",
        matches!(row.get(1), Some(DmValue::Text(s)) if s.trim().is_empty()),
    );
    h.check(
        "VARCHAR 空串取回来是空文本而不是 NULL",
        matches!(row.get(2), Some(DmValue::Text(s)) if s.is_empty()),
    );
    h.check(
        "CLOB 空串取回来是空文本而不是 NULL",
        matches!(row.get(3), Some(DmValue::Text(s)) if s.is_empty()),
    );
});

dm_test!(fn types_roundtrip_column_metadata(h) {
    let length_in_char = h.length_in_char;
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.exec_direct(&select_sql()) {
        return h.record("exec_direct", &err);
    }
    let columns: Arc<[ColumnMeta]> = match describe_columns(&mut stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns", &err),
    };

    h.eq("列出了 19 列", columns.len(), COLUMNS.len());
    let got: Vec<String> = columns
        .iter()
        .map(|c| c.data_type.to_string())
        .collect::<Vec<_>>();
    let want = [
        "INT",
        "CHAR",
        "VARCHAR",
        "CLOB",
        "TINYINT",
        "SMALLINT",
        "INT",
        "BIGINT",
        "DECIMAL",
        "DECIMAL",
        "REAL",
        "DOUBLE",
        "BOOLEAN",
        "DATE",
        "TIME",
        "TIMESTAMP",
        "VARBINARY",
        "BINARY",
        "BLOB",
    ];
    let want: Vec<String> = want.iter().map(|s| (*s).to_string()).collect();
    h.eq("列类型与 §7 表一致", got, want);

    for (index, name) in COLUMNS.iter().enumerate() {
        let Some(column) = columns.get(index) else {
            continue;
        };
        h.eq(&format!("{name} 的列名"), column.name.as_str(), *name);
    }

    // 日期时间三列必须落在 DATE/TIME/TIMESTAMP 上——这条是 `SQL_DESC_CONCISE_TYPE`
    // 那个坑的回归守卫：用 `SQL_DESC_TYPE` 的话达梦一律报 `SQL_DATETIME(9)`，
    // 三列会一起变成 Unknown（§6.4 真机实测补注①）。
    for index in [13usize, 14, 15] {
        let name = COLUMNS[index];
        let kind = columns.get(index).map(|c| c.data_type.to_string());
        h.check_that(
            kind.as_deref() == Some(match index {
                13 => "DATE",
                14 => "TIME",
                _ => "TIMESTAMP",
            }),
            || format!("{name} 的类型码退化成了 {kind:?}：请检查 describe_columns 是否还在用 col_concise_type"),
        );
    }

    // 文本列的长度单位：本机实测 `LENGTH_IN_CHAR=0`，即 `SQL_DESC_OCTET_LENGTH` 是字节。
    // 这是 `interpret_length_in_char` 的输入，写死等于把「本机配置」当「规格」——
    // 所以这里只断言**三态取值合法**，具体值打印出来供人工核对。
    for (index, name) in COLUMNS.iter().enumerate() {
        let Some(column) = columns.get(index) else {
            continue;
        };
        eprintln!(
            "[note] {name}: type={} nullable={} precision={:?} scale={:?} column_size={:?} length_in_char={:?}",
            column.data_type,
            column.nullable,
            column.precision,
            column.scale,
            column.column_size,
            column.length_in_char
        );
    }
    let text_units: Vec<Option<bool>> = [1usize, 2, 3]
        .iter()
        .filter_map(|i| columns.get(*i))
        .map(|c| c.length_in_char)
        .collect();
    h.check_that(text_units.iter().all(|u| u.is_some()), || {
        format!("文本列的 length_in_char 不该是未知：{text_units:?}")
    });

    // CLOB/BLOB 的 `column_size` 是 `i32::MAX` 哨兵（§7 真机实测补注⑤）：
    // 谁把它当容量去预分配，谁就会立刻 OOM。
    for index in [3usize, 18] {
        let name = COLUMNS[index];
        let size = columns.get(index).and_then(|c| c.column_size);
        h.check_that(size.is_some_and(|s| s as i64 == i64::from(i32::MAX)), || {
            format!("{name} 的 column_size 少了 i32::MAX 哨兵语义：{size:?}")
        });
    }
});

dm_test!(fn types_roundtrip_server_side_agreement(h) {
    h.apply_schema();
    let args = boundary_args();
    h.try_exec_params(&insert_sql(), &args);

    // 服务端自己算一遍，和我们取回来的值比对——**不拿我们自己的取值路径证明我们自己的写入路径**。
    h.expect_scalar(
        "服务端 to_char(DECIMAL)",
        &format!("select to_char(C_DECIMAL) from {T_TYPES} where ID = 1"),
        "9999999999999999.9999",
    );
    h.expect_scalar(
        "服务端 rawtohex(VARBINARY)",
        &format!("select rawtohex(C_VARBINARY) from {T_TYPES} where ID = 1"),
        "000AFF",
    );
    h.expect_scalar(
        "服务端 rawtohex(BINARY)",
        &format!("select rawtohex(C_BINARY) from {T_TYPES} where ID = 1"),
        "0101010101010101",
    );
    h.expect_scalar(
        "服务端 to_char(DATE)",
        &format!("select to_char(C_DATE, 'YYYY-MM-DD') from {T_TYPES} where ID = 1"),
        "1999-12-31",
    );
    h.expect_scalar(
        "服务端 to_char(TIME)",
        &format!("select to_char(C_TIME, 'HH24:MI:SS') from {T_TYPES} where ID = 1"),
        "23:59:59",
    );
    h.expect_scalar(
        "服务端 to_char(TIMESTAMP) 到微秒",
        &format!(
            "select to_char(C_TIMESTAMP, 'YYYY-MM-DD HH24:MI:SS.FF6') from {T_TYPES} where ID = 1"
        ),
        "1999-12-31 23:59:59.123456",
    );
    // `length(CLOB)` 数字符，`length(BLOB)` 数字节（§7 真机实测补注⑥）。
    h.expect_scalar(
        "服务端 length(CLOB) 数字符",
        &format!("select length(C_CLOB) from {T_TYPES} where ID = 1"),
        "200",
    );
    h.expect_scalar(
        "服务端 length(BLOB) 数字节",
        &format!("select length(C_BLOB) from {T_TYPES} where ID = 1"),
        "64",
    );
    h.expect_scalar(
        "服务端确认 BIT 列是真",
        &format!(
            "select CASE WHEN C_BIT = 1 THEN 'Y' ELSE 'N' END from {T_TYPES} where ID = 1"
        ),
        "Y",
    );

    // 顺便钉住「客户端取的字符数」与「服务端算的字符数」一致。
    let Some(rows) = h.try_rows(&format!(
        "select C_CLOB, length(C_CLOB) from {T_TYPES} where ID = 1"
    )) else {
        return;
    };
    let text_len = rows.first().and_then(|r| cell_text(r, 0)).map(|s| s.chars().count());
    let server_len = rows
        .first()
        .and_then(|r| cell_text(r, 1))
        .and_then(|s| s.parse::<usize>().ok());
    h.eq("CLOB 客户端字符数 == 服务端 length()", text_len, server_len);

    // 让 `canonical` 至少在一条路径上被用到，顺带覆盖 `Display` 的输出形态。
    h.eq(
        "canonical 对空值有固定写法",
        canonical(&DmValue::Null),
        "<NULL>".to_owned(),
    );
});
