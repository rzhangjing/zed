//! T3.5 真机探针：结果集描述、两段式取值、截断重取、NULL 与空串。
//!
//! 跑法（需要一台可达的达梦与已注册的驱动）：
//!
//! ```text
//! set DM_DRIVER=DM8 ODBC DRIVER
//! set DM_SERVER=localhost
//! set DM_PORT=5236
//! set DM_USER=SYSDBA
//! set DM_PASSWORD=SYSDBA
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example result_probe
//! ```
//!
//! 它**不**用 `assert!`：探针的价值是把观测到的东西原样打出来。退出码 0 = 全过，
//! 1 = 有检查没过，2 = 环境/连接就失败了（那时没东西可验）。

use std::process::ExitCode;

use dm_core::backend::{FetchState, RowBuf};
use dm_core::{DmConfig, DmValue};
use dm_error::DmError;
use dm_odbc::{
    FetchScratch, OdbcConnection, ParamArena, StmtOutcome, bind_params, describe_columns, fetch,
};

/// 探针用的表名（大写，DM 默认折叠为大写）。
const TABLE: &str = "DMD_ODBC_PROBE_RESULT";

/// 大文本：4096 个汉字 = 8192 个 UTF-16 码元 = 16 KiB 宽字符，是 4 KiB 试探块的 4 倍。
fn big_text() -> String {
    "达梦".repeat(2048)
}

/// 中等文本：2000 个汉字 = GB18030 下 4000 字节，塞得进 `varchar(8000)`。
fn medium_text() -> String {
    "达梦".repeat(1000)
}

/// 大二进制：0..=255 循环 32 遍 = 8192 字节。
fn big_bytes() -> Vec<u8> {
    (0..8192u32).map(|i| (i % 256) as u8).collect()
}

struct Tally {
    checks: usize,
    failures: usize,
    invalid_cursor: usize,
}

impl Tally {
    fn new() -> Self {
        Self {
            checks: 0,
            failures: 0,
            invalid_cursor: 0,
        }
    }

    fn check(&mut self, what: &str, ok: bool) {
        self.checks += 1;
        if ok {
            println!("  [ok]   {what}");
        } else {
            self.failures += 1;
            println!("  [FAIL] {what}");
        }
    }

    fn note(&mut self, what: &str) {
        println!("  [note] {what}");
    }

    /// 记账一个错误：顺手统计 `-70027`（本层的硬指标必须是 0）。
    fn note_error(&mut self, err: &DmError) {
        if let Some((_, code)) = err.kind.server_code()
            && code == dm_error::codes::INVALID_CURSOR
        {
            self.invalid_cursor += 1;
        }
    }

    fn ok<T>(&mut self, what: &str, result: Result<T, DmError>) -> Option<T> {
        match result {
            Ok(value) => {
                self.check(what, true);
                Some(value)
            }
            Err(err) => {
                self.note_error(&err);
                self.check(&format!("{what} —— 失败：{}", describe(&err)), false);
                None
            }
        }
    }
}

fn describe(err: &DmError) -> String {
    let mut out = format!("{} @ {:?}", err.kind, err.context.stage);
    for diag in err.diags() {
        out.push_str(&format!(
            "\n           [{} / {}] {}",
            diag.sqlstate_str(),
            diag.native_code,
            diag.message.trim()
        ));
    }
    out
}

/// 逐个试探类型名：达梦对同一个概念可能只认其中一种写法。
///
/// 这一步的价值不是「探针能不能跑」，而是把 §7 类型表的**服务端可用性**钉在实测上：
/// 哪些类型名被接受、哪些被 `22019/-6119 无效的数据类型` 拒绝，都必须看见原文。
fn discover_types(conn: &mut OdbcConnection, tally: &mut Tally) {
    const CANDIDATES: &[&str] = &[
        "char(10)",
        "varchar(100)",
        "varchar(8000)",
        "clob",
        "tinyint",
        "smallint",
        "int",
        "bigint",
        "decimal(20,4)",
        "numeric(10,2)",
        "real",
        "float",
        "double",
        "double precision",
        "boolean",
        "bit",
        "date",
        "time",
        "timestamp",
        "varbinary(300)",
        "binary(8)",
        "blob",
    ];

    let mut ok_types = Vec::new();
    let mut bad_types = Vec::new();
    for sql_type in CANDIDATES {
        let ddl = format!("create table DMD_ODBC_PROBE_TYPE (c {sql_type})");
        let mut stmt = match conn.allocate_statement() {
            Ok(stmt) => stmt,
            Err(err) => {
                tally.note(&format!("类型探测时分配语句失败：{}", describe(&err)));
                return;
            }
        };
        match stmt.exec_direct(&ddl) {
            Ok(_) => {
                ok_types.push(*sql_type);
                let _ = stmt.exec_direct("drop table DMD_ODBC_PROBE_TYPE");
            }
            Err(err) => {
                let first = err
                    .diags()
                    .first()
                    .map(|d| format!("{} / {}", d.sqlstate_str(), d.native_code))
                    .unwrap_or_else(|| "无诊断".to_string());
                bad_types.push(format!("{sql_type} => {first}"));
                let _ = stmt.close_cursor();
            }
        }
    }

    println!("  接受（{}）：{}", ok_types.len(), ok_types.join(", "));
    if bad_types.is_empty() {
        println!("  拒绝（0）：无");
    } else {
        println!("  拒绝（{}）：", bad_types.len());
        for row in &bad_types {
            println!("    {row}");
        }
    }
    tally.check(
        "类型名探测：至少 char/varchar/int/bigint/decimal/timestamp 被接受",
        [
            "char(10)",
            "varchar(100)",
            "int",
            "bigint",
            "decimal(20,4)",
            "timestamp",
        ]
        .iter()
        .all(|t| ok_types.contains(t)),
    );
}

/// 取出唯一一行的第 `index` 列（0 起）。
fn cell(row: &[DmValue], index: usize) -> DmValue {
    row[index].clone()
}

fn main() -> ExitCode {
    let cfg = match DmConfig::from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            println!("环境变量不完整，无法连接：{err}");
            println!("需要 DM_DRIVER / DM_SERVER / DM_PORT / DM_USER / DM_PASSWORD");
            return ExitCode::from(2);
        }
    };

    let mut tally = Tally::new();
    let big_text = big_text();
    let medium_text = medium_text();
    let big_bytes = big_bytes();

    let mut conn = match OdbcConnection::connect(&cfg) {
        Ok(conn) => conn,
        Err(err) => {
            println!("连接失败：{}", describe(&err));
            return ExitCode::from(2);
        }
    };
    println!("已连接（服务端 unicode = {:?}）", conn.server_unicode());

    // ------------------------------------------------------------------
    // 0. 逐类型试探：先把「达梦到底认哪些类型名」问清楚
    // ------------------------------------------------------------------
    println!("\n== 0. 类型名探测 ==");
    discover_types(&mut conn, &mut tally);

    // 先取出连接级配置：语句对象借用连接，之后就不能再读 `conn` 了。
    let length_in_char = conn.config().length_in_char;

    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("分配语句失败：{}", describe(&err));
            return ExitCode::from(2);
        }
    };

    // ------------------------------------------------------------------
    // 1. 建表
    // ------------------------------------------------------------------
    println!("\n== 1. 建表 ==");
    // 达梦没有 `drop table if exists`，先删再建，删不掉是正常的。
    match stmt.exec_direct(&format!("drop table {TABLE}")) {
        Ok(_) => tally.note("旧表已删除"),
        Err(err) => tally.note(&format!("旧表不存在或删不掉（预期之内）：{}", err.kind)),
    }

    let ddl = format!(
        "create table {TABLE} (
             ID int primary key,
             C_CHAR char(10),
             C_VARCHAR varchar(100),
             C_BIGVARCHAR varchar(8000),
             C_CLOB clob,
             C_TINYINT tinyint,
             C_SMALLINT smallint,
             C_INT int,
             C_BIGINT bigint,
             C_DECIMAL decimal(20,4),
             C_NUMERIC numeric(10,2),
             C_REAL real,
             C_FLOAT float,
             C_DOUBLE double,
             C_BOOL bit,
             C_DATE date,
             C_TIME time,
             C_TIMESTAMP timestamp,
             C_VARBINARY varbinary(300),
             C_BLOB blob
         )"
    );
    // 真机实测：`boolean` 是**无效的数据类型**（`22019/-6119`），达梦只认 `bit`。
    // 类型名探测那段会把这条差异原样打出来。
    if tally
        .ok(
            "create table（20 列覆盖 §7 的主要类型）",
            stmt.exec_direct(&ddl),
        )
        .is_none()
    {
        return finish(tally);
    }

    // ------------------------------------------------------------------
    // 2. 用字面量插入第 1 行（全部有值）
    // ------------------------------------------------------------------
    println!("\n== 2. 字面量插入第 1 行 ==");
    let insert1 = format!(
        "insert into {TABLE} values (
             1, 'abc', '中文和 ASCII 混排', 'x', 'clob 字面量',
             7, -1234, 2000000000, 9007199254740993,
             1234567890123456.7890, 12345.67,
             1.5, 2.5, 3.25, 1,
             date '2024-03-04', time '05:06:07', timestamp '2024-03-04 05:06:07.123456',
             0x0A0B0C, 0x0102030405
         )"
    );
    if tally
        .ok("insert 第 1 行", stmt.exec_direct(&insert1))
        .is_none()
    {
        return finish(tally);
    }

    // ------------------------------------------------------------------
    // 3. 用参数插入第 2/3/4 行：NULL、大值、空串
    // ------------------------------------------------------------------
    println!("\n== 3. 参数插入第 2/3/4 行 ==");
    let insert_sql = format!(
        "insert into {TABLE} (ID, C_CHAR, C_VARCHAR, C_BIGVARCHAR, C_CLOB, C_TINYINT,
                              C_SMALLINT, C_INT, C_BIGINT, C_DECIMAL, C_NUMERIC,
                              C_REAL, C_FLOAT, C_DOUBLE, C_BOOL, C_DATE, C_TIME,
                              C_TIMESTAMP, C_VARBINARY, C_BLOB)
         values (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
    );
    let mut arena = ParamArena::new();

    // 第 2 行：全部 NULL。
    let row2: Vec<DmValue> = (0..20)
        .map(|i| {
            if i == 0 {
                DmValue::Int(2)
            } else {
                DmValue::Null
            }
        })
        .collect();
    tally.ok("prepare 参数插入", stmt.prepare(&insert_sql));
    tally.ok(
        "bind 第 2 行（19 个 NULL）",
        bind_params(&mut stmt, &row2, &mut arena),
    );
    tally.ok("exec 第 2 行", stmt.exec());

    // 第 3 行：大文本 + 大二进制 + 各类型边界值。
    // 注意各文本列的**声明长度**：`varchar(100)` 只放短串、`varchar(8000)` 放
    // 4000 字节（GB18030 下 2000 个汉字）的中等串，16 KiB 的宽字符只塞 CLOB——
    // 塞错列会被服务端以 `22001/-70005 字符串截断` 拒绝，那是我们自己的错。
    let row3: Vec<DmValue> = vec![
        DmValue::Int(3),
        DmValue::Text("0123456789".to_owned()),
        DmValue::Text("小文本".to_owned()),
        DmValue::Text(medium_text.clone()),
        DmValue::Text(big_text.clone()),
        DmValue::TinyInt(-128),
        DmValue::SmallInt(-32768),
        DmValue::Int(i32::MIN),
        DmValue::BigInt(i64::MIN),
        DmValue::Decimal("9999999999999999.9999".parse().unwrap()),
        DmValue::Decimal("-12345678.90".parse().unwrap()),
        DmValue::Real(-1.25),
        DmValue::Double(-2.5),
        DmValue::Double(1.7976931348623157e308),
        DmValue::Bool(true),
        DmValue::Date(chrono::NaiveDate::from_ymd_opt(1999, 12, 31).unwrap()),
        DmValue::Time(chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap()),
        DmValue::Timestamp(
            chrono::NaiveDate::from_ymd_opt(1999, 12, 31)
                .unwrap()
                .and_hms_micro_opt(23, 59, 59, 999_999)
                .unwrap(),
        ),
        // VARBINARY(300) 只放 3 字节；8 KiB 是 C_BLOB 的事。
        DmValue::Bytes(vec![0x0A, 0x0B, 0x0C]),
        DmValue::Bytes(big_bytes.clone()),
    ];
    tally.ok(
        "bind 第 3 行（大值 + 类型边界）",
        bind_params(&mut stmt, &row3, &mut arena),
    );
    tally.ok("exec 第 3 行", stmt.exec());

    // 第 4 行：空串（与 NULL 必须分得开）。
    let mut row4: Vec<DmValue> = vec![DmValue::Int(4)];
    row4.extend((0..19).map(|_| DmValue::Text(String::new())));
    // 非文本列不能绑空串，改回 NULL，只让文本列是空串（ID 列必须留着）。
    for (i, value) in row4.iter_mut().enumerate().skip(1) {
        let text_column = matches!(i, 1..=4);
        if !text_column {
            *value = DmValue::Null;
        }
    }
    tally.ok(
        "bind 第 4 行（空串）",
        bind_params(&mut stmt, &row4, &mut arena),
    );
    tally.ok("exec 第 4 行", stmt.exec());

    // ------------------------------------------------------------------
    // 4. 描述列
    // ------------------------------------------------------------------
    println!("\n== 4. 描述列 ==");
    let select = format!("select * from {TABLE} order by ID");
    let outcome = tally.ok("select * 执行", stmt.exec_direct(&select));
    match outcome {
        Some(StmtOutcome::ResultSet { columns }) => tally.check("结果集报 20 列", columns == 20),
        other => {
            tally.check(&format!("select 应产出结果集，实际 {other:?}"), false);
            return finish(tally);
        }
    }

    let meta = match tally.ok(
        "describe_columns",
        describe_columns(&mut stmt, length_in_char),
    ) {
        Some(meta) => meta,
        None => return finish(tally),
    };

    println!("  列清单：");
    for column in meta.iter() {
        println!(
            "    {:<14} {:<18} nullable={:<5} size={:?} precision={:?} scale={:?} length_in_char={:?}",
            column.name,
            column.data_type.to_string(),
            column.nullable,
            column.column_size,
            column.precision,
            column.scale,
            column.length_in_char
        );
    }
    tally.check("20 列的元数据都拿到了", meta.len() == 20);
    // §7：本机服务端字符集是 GB18030，所以文本列的 column_size 按字节解释。
    let text_columns: Vec<_> = meta.iter().filter(|c| c.data_type.is_text()).collect();
    tally.check(
        "文本列都标了 length_in_char",
        text_columns.iter().all(|c| c.length_in_char.is_some()),
    );

    // ------------------------------------------------------------------
    // 5. 逐行取值
    // ------------------------------------------------------------------
    println!("\n== 5. 逐行取值 ==");
    let mut buf = RowBuf::new(meta.len());
    let mut scratch = FetchScratch::new();
    let mut rows: Vec<Vec<DmValue>> = Vec::new();

    loop {
        match fetch(&mut stmt, &meta, &mut buf, &mut scratch) {
            Ok(FetchState::Rows { .. }) => {
                let row = buf.row(0).expect("刚取到一行").to_vec();
                rows.push(row);
            }
            Ok(FetchState::Exhausted) => break,
            Ok(other) => {
                tally.check(&format!("fetch 返回了意外状态 {other:?}"), false);
                break;
            }
            Err(err) => {
                tally.note_error(&err);
                tally.check(&format!("fetch 失败：{}", describe(&err)), false);
                break;
            }
        }
    }
    tally.check("取满 4 行后走到 Exhausted", rows.len() == 4);
    tally.check("游标已记为关闭", !stmt.is_cursor_open());

    if rows.len() == 4 {
        check_row1(&mut tally, &rows[0]);
        check_row2(&mut tally, &rows[1]);
        check_row3(&mut tally, &rows[2], &big_text, &medium_text, &big_bytes);
        check_row4(&mut tally, &rows[3]);
    }

    // ------------------------------------------------------------------
    // 6. 服务端侧交叉验证
    // ------------------------------------------------------------------
    println!("\n== 6. 服务端侧交叉验证 ==");
    // `stmt` 借用了 `conn`，交叉验证要自己分配语句句柄，所以先还回去。
    drop(stmt);
    server_side_cross_check(&mut conn, &mut tally, &big_bytes, length_in_char);

    // ------------------------------------------------------------------
    // 7. 清场
    // ------------------------------------------------------------------
    println!("\n== 7. 清场 ==");
    match conn.allocate_statement() {
        Ok(mut cleaner) => {
            if let Err(err) = cleaner.exec_direct(&format!("drop table {TABLE}")) {
                tally.note(&format!("drop table 失败：{}", err.kind));
            }
        }
        Err(err) => tally.note(&format!("清场时分配语句失败：{}", describe(&err))),
    }

    finish(tally)
}

fn check_row1(tally: &mut Tally, row: &[DmValue]) {
    println!("  第 1 行（字面量）：");
    tally.check(
        "CHAR 取值 = \"abc\"（DM 的 CHAR 不补空格到定长）",
        matches!(cell(row, 1), DmValue::Text(ref t) if t == "abc" || t.trim_end() == "abc"),
    );
    tally.check(
        "VARCHAR 中文原样",
        matches!(cell(row, 2), DmValue::Text(ref t) if t == "中文和 ASCII 混排"),
    );
    tally.check("TINYINT = 7", matches!(cell(row, 5), DmValue::TinyInt(7)));
    tally.check(
        "SMALLINT = -1234",
        matches!(cell(row, 6), DmValue::SmallInt(-1234)),
    );
    tally.check(
        "INT = 2000000000",
        matches!(cell(row, 7), DmValue::Int(2_000_000_000)),
    );
    tally.check(
        "BIGINT 超出 f64 精度仍精确（9007199254740993）",
        matches!(cell(row, 8), DmValue::BigInt(9_007_199_254_740_993)),
    );
    // 十进制的比较用**数值相等**而不是字符串相等：`Decimal` 的 `scale` 是表示细节
    // （`12.30` 与 `12.3` 数值相同），拿 `to_string()` 比会把表示差异当成数据错误。
    check_decimal(tally, row, 9, "DECIMAL(20,4)", "1234567890123456.7890");
    check_decimal(tally, row, 10, "NUMERIC(10,2)", "12345.67");
    tally.check(
        "REAL = 1.5",
        matches!(cell(row, 11), DmValue::Real(v) if (v - 1.5).abs() < 1e-6),
    );
    tally.check(
        "DOUBLE = 3.25",
        matches!(cell(row, 13), DmValue::Double(v) if (v - 3.25).abs() < 1e-12),
    );
    tally.check(
        "BOOLEAN 真值取成 Bool(true)",
        matches!(cell(row, 14), DmValue::Bool(true)),
    );
    tally.check(
        "DATE = 2024-03-04",
        matches!(cell(row, 15), DmValue::Date(d) if d.to_string() == "2024-03-04"),
    );
    tally.check(
        "TIME = 05:06:07",
        matches!(cell(row, 16), DmValue::Time(t) if t.to_string() == "05:06:07"),
    );
    tally.check(
        "TIMESTAMP 保留微秒",
        matches!(cell(row, 17), DmValue::Timestamp(t) if t.to_string() == "2024-03-04 05:06:07.123456"),
    );
    tally.check(
        "VARBINARY 原样字节",
        matches!(cell(row, 18), DmValue::Bytes(ref b) if b == &[0x0A, 0x0B, 0x0C]),
    );
    tally.check(
        "BLOB 原样字节",
        matches!(cell(row, 19), DmValue::Bytes(ref b) if b == &[0x01, 0x02, 0x03, 0x04, 0x05]),
    );
}

fn check_row2(tally: &mut Tally, row: &[DmValue]) {
    println!("  第 2 行（全 NULL）：");
    let nulls = (1..20).all(|i| matches!(cell(row, i), DmValue::Null));
    tally.check("19 列全部是 DmValue::Null", nulls);
}

fn check_row3(
    tally: &mut Tally,
    row: &[DmValue],
    big_text: &str,
    medium_text: &str,
    big_bytes: &[u8],
) {
    println!("  第 3 行（大值 + 类型边界）：");
    let expected: [(usize, &str, &str); 3] = [
        (2, "小文本", "varchar(100)"),
        (3, medium_text, "varchar(8000) 中等文本"),
        (4, big_text, "CLOB 16 KiB 宽字符"),
    ];
    for (index, want, what) in expected {
        match cell(row, index) {
            DmValue::Text(text) => {
                let units: usize = text.encode_utf16().count();
                tally.check(
                    &format!("第 {index} 列 {what} 完整取回（{units} 码元，跨越 4 KiB 块边界）"),
                    text == want,
                );
            }
            other => tally.check(&format!("第 {index} 列应为文本，实际 {other:?}"), false),
        }
    }
    tally.check(
        "TINYINT = -128",
        matches!(cell(row, 5), DmValue::TinyInt(-128)),
    );
    tally.check(
        "SMALLINT = -32768",
        matches!(cell(row, 6), DmValue::SmallInt(-32768)),
    );
    tally.check(
        "INT = i32::MIN",
        matches!(cell(row, 7), DmValue::Int(i32::MIN)),
    );
    tally.check(
        "BIGINT = i64::MIN",
        matches!(cell(row, 8), DmValue::BigInt(i64::MIN)),
    );
    check_decimal(tally, row, 9, "DECIMAL 上界", "9999999999999999.9999");
    check_decimal(tally, row, 10, "NUMERIC 负值", "-12345678.90");
    tally.check(
        "DATE = 1999-12-31",
        matches!(cell(row, 15), DmValue::Date(d) if d.to_string() == "1999-12-31"),
    );
    tally.check(
        "TIME = 23:59:59",
        matches!(cell(row, 16), DmValue::Time(t) if t.to_string() == "23:59:59"),
    );
    tally.check(
        "TIMESTAMP 微秒边界 999999",
        matches!(cell(row, 17), DmValue::Timestamp(t) if t.to_string().ends_with(".999999")),
    );
    tally.check(
        "VARBINARY(300) 3 字节原样",
        matches!(cell(row, 18), DmValue::Bytes(ref b) if b == &[0x0A, 0x0B, 0x0C]),
    );
    match cell(row, 19) {
        DmValue::Bytes(bytes) => {
            tally.check(
                &format!("BLOB 8 KiB 完整取回（实际 {} 字节）", bytes.len()),
                bytes == big_bytes,
            );
        }
        other => tally.check(&format!("BLOB 应为字节，实际 {other:?}"), false),
    }
    let _ = big_bytes;
}

fn check_row4(tally: &mut Tally, row: &[DmValue]) {
    println!("  第 4 行（空串）：");
    for index in [1usize, 2, 3, 4] {
        match cell(row, index) {
            DmValue::Text(text) => {
                // `CHAR(n)` 是定长、末尾补空格：空串取回来是 n 个空格，这不算错。
                // `VARCHAR`/`CLOB` 必须原样是空串。
                let want_blank = index == 1;
                let ok = if want_blank {
                    text.trim().is_empty()
                } else {
                    text.is_empty()
                };
                tally.check(
                    &format!(
                        "第 {index} 列空串是 Text 而不是 Null（与第 2 行的 NULL 分得开；实际 {} 字符）",
                        text.chars().count()
                    ),
                    ok,
                );
            }
            // 达梦把空串折成 NULL 时如实报告，但必须**明确**看到这一点。
            DmValue::Null => tally.note(&format!(
                "第 {index} 列空串在服务端变成了 NULL —— 服务端把 '' 与 NULL 视为同一个值"
            )),
            other => tally.check(&format!("第 {index} 列应为空串，实际 {other:?}"), false),
        }
    }
}

/// 比对一列十进制值：用数值相等，并把实际值打出来。
fn check_decimal(tally: &mut Tally, row: &[DmValue], index: usize, what: &str, want: &str) {
    let want: rust_decimal::Decimal = want.parse().expect("期望值是合法的十进制字面量");
    match cell(row, index) {
        DmValue::Decimal(ref d) => tally.check(
            &format!("{what} = {want}（实际 {d}，scale {}）", d.scale()),
            *d == want,
        ),
        other => tally.check(&format!("{what} 应为 Decimal，实际 {other:?}"), false),
    }
}

fn server_side_cross_check(
    conn: &mut OdbcConnection,
    tally: &mut Tally,
    big_bytes: &[u8],
    length_in_char: Option<bool>,
) {
    // 真机实测：`RAWTOHEX(BLOB)` 会被达梦以 `HY000/-5403 参数不兼容` 拒绝
    // （`RAWTOHEX(VARBINARY)` 正常），所以 BLOB 用 `length()` 观测、VARBINARY 用
    // `rawtohex()` 做逐字节硬校验。
    let sql = format!(
        "select length(C_BLOB), to_char(C_DECIMAL), rawtohex(C_VARBINARY), length(C_CLOB) \
         from {TABLE} where ID = 3"
    );
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            tally.check(&format!("分配语句失败：{}", describe(&err)), false);
            return;
        }
    };
    if tally.ok("服务端交叉查询", stmt.exec_direct(&sql)).is_none() {
        return;
    }
    let meta = match tally.ok(
        "交叉查询列描述",
        describe_columns(&mut stmt, length_in_char),
    ) {
        Some(meta) => meta,
        None => return,
    };
    let mut buf = RowBuf::new(meta.len());
    let mut scratch = FetchScratch::new();
    if !matches!(
        fetch(&mut stmt, &meta, &mut buf, &mut scratch),
        Ok(FetchState::Rows { .. })
    ) {
        tally.check("交叉查询没有返回行", false);
        return;
    }
    let row = buf.row(0).expect("刚取到一行").to_vec();

    match cell(&row, 0) {
        DmValue::Int(n) => tally.check(
            &format!(
                "服务端 length(BLOB) = {n}（客户端取回 {} 字节）",
                big_bytes.len()
            ),
            n == big_bytes.len() as i32,
        ),
        DmValue::BigInt(n) => tally.check(
            &format!(
                "服务端 length(BLOB) = {n}（客户端取回 {} 字节）",
                big_bytes.len()
            ),
            n == big_bytes.len() as i64,
        ),
        other => tally.note(&format!("length(BLOB) 返回了 {other:?}")),
    }
    match cell(&row, 1) {
        DmValue::Text(text) => tally.check(
            &format!("服务端 to_char(DECIMAL) = {text}"),
            text.trim() == "9999999999999999.9999",
        ),
        other => tally.check(&format!("to_char 应返回文本，实际 {other:?}"), false),
    }
    match cell(&row, 2) {
        DmValue::Text(hex) => tally.check(
            &format!("服务端 rawtohex(VARBINARY) = {}（应为 0A0B0C）", hex.trim()),
            hex.trim().eq_ignore_ascii_case("0A0B0C"),
        ),
        other => tally.check(
            &format!("rawtohex(VARBINARY) 应返回文本，实际 {other:?}"),
            false,
        ),
    }
    match cell(&row, 3) {
        DmValue::Int(n) => tally.note(&format!("服务端 length(CLOB) = {n}")),
        DmValue::BigInt(n) => tally.note(&format!("服务端 length(CLOB) = {n}")),
        other => tally.note(&format!("length(CLOB) 返回了 {other:?}")),
    }
}

fn finish(tally: Tally) -> ExitCode {
    println!("\n================ 总结 ================");
    println!("检查 {} 项，失败 {} 项", tally.checks, tally.failures);
    println!("-70027（无效的游标状态）出现 {} 次", tally.invalid_cursor);
    if tally.failures == 0 {
        println!("全部通过。");
        ExitCode::SUCCESS
    } else {
        println!("有检查未通过，见上面的 [FAIL]。");
        ExitCode::FAILURE
    }
}
