//! T3.4（参数绑定）真机探针。
//!
//! 覆盖 §12.2 T3.4 的验证口径：空串 / 中文 / 含 NUL / 超 4 KiB / NULL 五类文本，
//! 加上整数、浮点、日期、DECIMAL、VARBINARY、BLOB，以及「稳态零分配」与
//! 「类型变化才复位」。
//!
//! 两个连接：`OdbcConnection`（被测：走 [`dm_odbc::param::bind_params`]）负责写，
//! `odbc_api::Connection`（读）负责用**服务端自己算出来的值**校验——比如
//! `rawtohex`、`length`、`case when ... then`，避免拿我们自己的取值路径去证明
//! 我们自己的写入路径。
//!
//! 用法：
//! ```text
//! $env:DM_DRIVER='DM8 ODBC DRIVER'; $env:DM_SERVER='localhost'; $env:DM_PORT='5236'
//! $env:DM_USER='SYSDBA'; $env:DM_PASSWORD='SYSDBA'
//! cargo run -p dm-odbc --example param_probe
//! ```

use std::fmt::Display;

use dm_core::{DmConfig, DmValue};
use dm_error::{DmError, DmErrorKind, Stage, codes};
use dm_odbc::conn::OdbcConnection;
use dm_odbc::param::{ParamArena, bind_params};
use odbc_api::{Connection, ConnectionOptions, Cursor, Environment};

/// 记账：过了多少条、挂了几条、`-70027` 出现了几次。
#[derive(Default)]
struct Tally {
    checks: u32,
    failures: u32,
    invalid_cursor: u32,
}

impl Tally {
    fn check(&mut self, name: &str, ok: bool, detail: impl Display) {
        self.checks += 1;
        if ok {
            println!("  [ok]   {name}: {detail}");
        } else {
            self.failures += 1;
            println!("  [FAIL] {name}: {detail}");
        }
    }

    fn observe(&mut self, name: &str, detail: impl Display) {
        self.checks += 1;
        println!("  [note] {name}: {detail}");
    }

    fn note_cursor(&mut self, err: &DmError) {
        if err.server_code_and_message().map(|(code, _)| code) == Some(codes::INVALID_CURSOR) {
            self.invalid_cursor += 1;
        }
    }
}

type R<T> = Result<T, String>;

/// 单列单行的宽字符取出（§4.3：中文一律走这条；窄路径拿到的是 GB18030 字节）。
fn scalar(reader: &Connection<'_>, sql: &str) -> R<Option<String>> {
    let Some(mut cur) = reader
        .execute(sql, (), None)
        .map_err(|e| format!("execute: {e}"))?
    else {
        return Ok(None);
    };
    let Some(mut row) = cur.next_row().map_err(|e| format!("next_row: {e}"))? else {
        return Ok(None);
    };
    let mut buf: Vec<u16> = Vec::new();
    match row.get_wide_text(1, &mut buf) {
        Ok(true) => Ok(Some(String::from_utf16_lossy(&buf))),
        Ok(false) => Ok(None),
        Err(e) => Err(format!("get_wide_text: {e}")),
    }
}

/// 直接下发一条语句（建表/清理用，不参数化）。
fn exec_direct(reader: &Connection<'_>, sql: &str) -> R<()> {
    reader
        .execute(sql, (), None)
        .map(|_| ())
        .map_err(|e| format!("execute: {e}"))
}

/// 带参数插入一行。
fn insert(
    conn: &mut OdbcConnection,
    arena: &mut ParamArena,
    sql: &str,
    args: &[DmValue],
) -> Result<(), DmError> {
    let mut stmt = conn.allocate_statement()?;
    stmt.prepare(sql)?;
    bind_params(&mut stmt, args, arena)?;
    stmt.exec()?;
    Ok(())
}

fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02X}"));
    }
    out
}

/// 列序：0=ID 1=N 2=F 3=D 4=TS 5=T 6=BIG 7=B(VARBINARY) 8=BL(BLOB)
const COLS: &str = "ID,N,F,D,TS,T,BIG,B,BL";
/// 一共几个 `?`。
const PARAMS: usize = 9;

fn null_row() -> Vec<DmValue> {
    (0..PARAMS).map(|_| DmValue::Null).collect()
}

fn main() {
    let mut tally = Tally::default();

    let cfg = match DmConfig::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("读不到连接配置（DM_DRIVER/DM_SERVER/DM_PORT/DM_USER/DM_PASSWORD）：{e}");
            std::process::exit(2);
        }
    };
    let cs = dm_core::render_odbc_string(&cfg);

    let env = Environment::new().expect("ODBC 环境");
    let reader = match env.connect_with_connection_string(&cs, ConnectionOptions::default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("读连接建不起来：{e}");
            std::process::exit(2);
        }
    };
    let mut conn = match OdbcConnection::connect(&cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("被测连接建不起来：{e}");
            std::process::exit(2);
        }
    };

    let tag = format!("DM_P{}", std::process::id());
    let table = format!("{tag}_T");
    let rep = format!("{tag}_R");

    let _ = exec_direct(&reader, &format!("DROP TABLE {table}"));
    let _ = exec_direct(&reader, &format!("DROP TABLE {rep}"));
    for ddl in [
        format!(
            "CREATE TABLE {table} (\
               ID INT NOT NULL, N BIGINT, F DOUBLE, D DECIMAL(20,4), TS TIMESTAMP, \
               T VARCHAR(100), BIG CLOB, B VARBINARY(300), BL BLOB)"
        ),
        format!("CREATE TABLE {rep} (ID INT NOT NULL, EXTRA INT)"),
    ] {
        if let Err(e) = exec_direct(&reader, &ddl) {
            eprintln!("建表失败：{e}\n{ddl}");
            std::process::exit(2);
        }
    }
    println!("== 表 {table} / {rep} 已建好 ==");

    let sql = format!("INSERT INTO {table} ({COLS}) VALUES (?,?,?,?,?,?,?,?,?)");

    // ---- 1. 全类型一行 ----------------------------------------------------
    let big_text: String = "汉字Abc".repeat(1000); // 5000 个字符，超过 4 KiB 探测缓冲
    let big_chars = big_text.chars().count();
    let blob: Vec<u8> = (0..300u32).map(|i| (i % 256) as u8).collect();
    let row1 = vec![
        DmValue::Int(1),
        DmValue::BigInt(i64::MAX),
        DmValue::Double(1.5),
        DmValue::Decimal("12345.6789".parse().expect("decimal")),
        DmValue::Timestamp(
            chrono::NaiveDate::from_ymd_opt(2024, 3, 5)
                .expect("date")
                .and_hms_opt(1, 2, 3)
                .expect("time"),
        ),
        DmValue::Text("中文测试".to_string()),
        DmValue::Text(big_text.clone()),
        DmValue::Bytes(blob.clone()),
        DmValue::Bytes(blob.clone()),
    ];
    let mut arena = ParamArena::new();
    let t0 = std::time::Instant::now();
    match insert(&mut conn, &mut arena, &sql, &row1) {
        Ok(()) => tally.check(
            "全类型插入",
            true,
            format!(
                "{PARAMS} 个参数（含 {big_chars} 字符 CLOB 与 300 字节二进制×2）耗时 {} µs",
                t0.elapsed().as_micros()
            ),
        ),
        Err(e) => tally.check("全类型插入", false, format!("{e}")),
    }

    // ---- 2. 其余参数全 NULL 的一行 ----------------------------------------
    // ID 是 NOT NULL 列（真机实测：[23000/-6609] 违反列[ID]非空约束），
    // 所以「全 NULL」只能指除 ID 之外的 8 个参数。
    let mut row2 = null_row();
    row2[0] = DmValue::Int(2);
    match insert(&mut conn, &mut arena, &sql, &row2) {
        Ok(()) => tally.check(
            "其余参数全 NULL 插入",
            true,
            format!("{} 个 NULL 全部用 indicator=-1 表达", PARAMS - 1),
        ),
        Err(e) => tally.check("其余参数全 NULL 插入", false, format!("{e}")),
    }

    // ---- 3. 空串 / 零值一行（与 NULL 划清界限）-----------------------------
    let mut row3 = null_row();
    row3[0] = DmValue::Int(3);
    row3[1] = DmValue::BigInt(0);
    row3[2] = DmValue::Double(0.0);
    row3[3] = DmValue::Decimal("0".parse().expect("decimal"));
    row3[5] = DmValue::Text(String::new());
    row3[6] = DmValue::Text(String::new());
    row3[7] = DmValue::Bytes(Vec::new());
    row3[8] = DmValue::Bytes(Vec::new());
    match insert(&mut conn, &mut arena, &sql, &row3) {
        Ok(()) => tally.check(
            "空串/零值插入",
            true,
            "空串用 indicator=0 表达，与 NULL 不同",
        ),
        Err(e) => tally.check("空串/零值插入", false, format!("{e}")),
    }

    // ---- 4. 负 DECIMAL（绝对值 + 独立符号位）------------------------------
    let mut row4 = null_row();
    row4[0] = DmValue::Int(4);
    row4[3] = DmValue::Decimal("-0.05".parse().expect("decimal"));
    match insert(&mut conn, &mut arena, &sql, &row4) {
        Ok(()) => tally.check("负 DECIMAL 插入", true, "-0.05（precision 必须 ≥ scale）"),
        Err(e) => tally.check("负 DECIMAL 插入", false, format!("{e}")),
    }

    // ---- 5. 服务端侧校验 --------------------------------------------------
    match scalar(&reader, &format!("SELECT COUNT(*) FROM {table}")) {
        Ok(Some(n)) => tally.check("行数", n == "4", format!("count(*)={n}（期望 4）")),
        other => tally.check("行数", false, format!("{other:?}")),
    }

    let cases: Vec<(&str, String, String)> = vec![
        (
            "BIGINT 上界",
            format!(
                "SELECT CASE WHEN N = 9223372036854775807 THEN 'Y' ELSE 'N' END FROM {table} WHERE ID=1"
            ),
            "Y".to_string(),
        ),
        (
            "DOUBLE 1.5",
            format!("SELECT CASE WHEN F = 1.5 THEN 'Y' ELSE 'N' END FROM {table} WHERE ID=1"),
            "Y".to_string(),
        ),
        (
            "DECIMAL 12345.6789",
            format!(
                "SELECT CASE WHEN D = 12345.6789 THEN 'Y' ELSE 'N' END FROM {table} WHERE ID=1"
            ),
            "Y".to_string(),
        ),
        (
            "DECIMAL -0.05（符号位）",
            format!("SELECT CASE WHEN D = -0.05 THEN 'Y' ELSE 'N' END FROM {table} WHERE ID=4"),
            "Y".to_string(),
        ),
        (
            "TIMESTAMP",
            format!("SELECT TO_CHAR(TS,'YYYY-MM-DD HH24:MI:SS') FROM {table} WHERE ID=1"),
            "2024-03-05 01:02:03".to_string(),
        ),
        (
            "中文 VARCHAR",
            format!("SELECT T FROM {table} WHERE ID=1"),
            "中文测试".to_string(),
        ),
    ];
    for (name, sql, want) in cases {
        match scalar(&reader, &sql) {
            Ok(Some(got)) => tally.check(name, got == want, format!("got={got:?} want={want:?}")),
            other => tally.check(name, false, format!("{other:?}")),
        }
    }

    // 5000 字符的 CLOB：用服务端自己的 length() 数（比我们本地数更可信）
    match scalar(
        &reader,
        &format!("SELECT LENGTH(BIG) FROM {table} WHERE ID=1"),
    ) {
        Ok(Some(got)) => tally.check(
            "超 4 KiB 文本",
            got == big_chars.to_string(),
            format!(
                "服务端 length()={got}（期望 {big_chars}，原串 {} 字节 UTF-8）",
                big_text.len()
            ),
        ),
        other => tally.check("超 4 KiB 文本", false, format!("{other:?}")),
    }

    // VARBINARY：拿服务端 rawtohex 与本地十六进制比（逐字节，最硬的证据）
    match scalar(
        &reader,
        &format!("SELECT RAWTOHEX(B) FROM {table} WHERE ID=1"),
    ) {
        Ok(Some(got)) => {
            let want = hex_of(&blob);
            tally.check(
                "VARBINARY 300 字节",
                got.eq_ignore_ascii_case(&want),
                format!("rawtohex 长度={} 期望={}", got.len(), want.len()),
            );
        }
        other => tally.check("VARBINARY 300 字节", false, format!("{other:?}")),
    }
    // BLOB：`rawtohex` 对 BLOB 报「参数不兼容」，改用 length()（观测）
    match scalar(
        &reader,
        &format!("SELECT LENGTH(BL) FROM {table} WHERE ID=1"),
    ) {
        Ok(Some(got)) => tally.observe("BLOB LENGTH", format!("length(BLOB)={got}（期望 300）")),
        other => tally.observe(
            "BLOB LENGTH",
            format!("{other:?}（DM 的 length() 可能不接受 BLOB）"),
        ),
    }

    // NULL 与空串的界线：这是 §6.4 里最容易糊掉的一条。
    // `SELECT T IS NULL` 在达梦里是语法错，必须写成 CASE WHEN。
    let nullness: [(&str, &str, &str, &str); 6] = [
        ("NULL 行 T IS NULL", "T", "ID=2", "null"),
        ("NULL 行 N IS NULL", "N", "ID=2", "null"),
        ("NULL 行 BL IS NULL", "BL", "ID=2", "null"),
        ("空串行 T IS NULL", "T", "ID=3", "notnull"),
        ("空串行 B(VARBINARY) IS NULL", "B", "ID=3", "notnull"),
        ("空串行 BL(BLOB) IS NULL", "BL", "ID=3", "notnull"),
    ];
    for (name, col, id, want) in nullness {
        let sql = format!(
            "SELECT CASE WHEN {col} IS NULL THEN 'null' ELSE 'notnull' END FROM {table} WHERE {id}"
        );
        match scalar(&reader, &sql) {
            Ok(Some(got)) => tally.check(
                name,
                got == want,
                format!("{id} 的 {col}：{got}（期望 {want}）"),
            ),
            other => tally.check(name, false, format!("{other:?}")),
        }
    }
    // 空串到底存成了什么：这是「客户端区分了、服务端还认不认」的判据。
    // 单条 SQL 把三种可能一次问清，避免把「值是 NULL」和「LENGTH() 返 NULL」混为一谈。
    for (name, col, id) in [
        ("空串的落库形态（VARCHAR）", "T", "ID=3"),
        ("空串的落库形态（CLOB）", "BIG", "ID=3"),
        ("空串的落库形态（VARBINARY）", "B", "ID=3"),
    ] {
        let sql = format!(
            "SELECT CASE WHEN {col} IS NULL THEN 'IS_NULL' \
             WHEN LENGTH({col}) IS NULL THEN 'NOT_NULL_LENGTH_IS_NULL' \
             ELSE 'LEN_'||LENGTH({col}) END FROM {table} WHERE {id}"
        );
        match scalar(&reader, &sql) {
            Ok(Some(got)) => {
                // 判据不是 `LEN_0`：达梦的 `LENGTH('')` **本身就返 NULL**（Oracle 兼容语义，
                // 基线用例 `SELECT LENGTH('')` 实测 `EMPTY_IS_NULL`），所以 VARCHAR/VARBINARY
                // 上的空串会得到 `NOT_NULL_LENGTH_IS_NULL`。真正要守的是「服务端没把空串
                // 改写成 NULL」——即答案不含 `IS_NULL`。
                let preserved = got != "IS_NULL";
                tally.check(
                    name,
                    preserved,
                    format!(
                        "{got} —— IS_NULL 表示服务端把空串当成了 NULL（那 §6.4「NULL 与空串严格区分」\
                         在服务端就只剩一半）；NOT_NULL_LENGTH_IS_NULL 表示值不是 NULL、只是 \
                         `LENGTH()` 对空串返 NULL（应用层不能拿 LENGTH 区分空串与 NULL）"
                    ),
                );
            }
            other => tally.check(name, false, format!("{other:?}")),
        }
    }
    // 客户端侧最硬的证据：空串回读出来必须是「零长度文本」而不是 NULL。
    for (name, col) in [("空串回读（VARCHAR）", "T"), ("空串回读（VARBINARY）", "B")] {
        match scalar(&reader, &format!("SELECT {col} FROM {table} WHERE ID=3")) {
            Ok(Some(s)) => tally.check(
                name,
                s.is_empty(),
                format!("读到 {} 个字符（NULL 会走另一个分支）", s.chars().count()),
            ),
            other => tally.check(name, false, format!("{other:?}")),
        }
    }
    // 基线：LENGTH('') 在达梦上本身就是 NULL 吗？（Oracle 兼容语义）
    match scalar(
        &reader,
        "SELECT CASE WHEN LENGTH('') IS NULL THEN 'EMPTY_IS_NULL' ELSE 'EMPTY_LEN_OK' END FROM dual",
    ) {
        Ok(Some(got)) => tally.observe("基线 LENGTH('')", got),
        other => tally.observe("基线 LENGTH('')", format!("{other:?}")),
    }

    // 含 NUL 的文本：先把观测结果记下来
    let with_nul = "a\u{0}b".to_string();
    let mut row5 = null_row();
    row5[0] = DmValue::Int(5);
    row5[5] = DmValue::Text(with_nul.clone());
    match insert(&mut conn, &mut arena, &sql, &row5) {
        Ok(()) => {
            let len = scalar(
                &reader,
                &format!("SELECT LENGTH(T) FROM {table} WHERE ID=5"),
            )
            .unwrap_or(None);
            let val = scalar(&reader, &format!("SELECT T FROM {table} WHERE ID=5")).unwrap_or(None);
            let hex = scalar(
                &reader,
                &format!("SELECT RAWTOHEX(T) FROM {table} WHERE ID=5"),
            )
            .unwrap_or(None);
            tally.observe(
                "含 NUL 的文本",
                format!(
                    "插入成功；服务端 length()={len:?}、读回={val:?}、rawtohex={hex:?}（原串 3 个字符）"
                ),
            );
            if len.as_deref() != Some("3") {
                tally.observe(
                    "含 NUL 的文本（结论）",
                    "服务端在第一个 NUL 处**静默截断**：客户端拿到的是成功返回，行里却是残缺数据。\
                     必须写进设计文档与 §14——文本参数不得含 NUL，且这一条没有任何返回码能兜住",
                );
            }
        }
        Err(e) => tally.observe("含 NUL 的文本", format!("插入被拒：{e}")),
    }

    // ---- 6. 稳态零分配 ----------------------------------------------------
    let rep_sql = format!("INSERT INTO {rep} (ID) VALUES (?)");
    {
        let mut arena = ParamArena::new();
        let mut stmt = match conn.allocate_statement() {
            Ok(s) => s,
            Err(e) => {
                tally.check("稳态绑定", false, format!("分配语句失败：{e}"));
                finish(&tally, &reader, &[&table, &rep]);
            }
        };
        if let Err(e) = stmt.prepare(&rep_sql) {
            tally.check("稳态绑定", false, format!("prepare 失败：{e}"));
        } else {
            tally.check(
                "param_count 问驱动",
                stmt.param_count() == 1,
                format!("SQLNumParams 报 {}（期望 1）", stmt.param_count()),
            );
            let mut grew = 0u32;
            let mut last_capacity = 0usize;
            let mut errors = Vec::new();
            for i in 0..200 {
                if let Err(e) = bind_params(&mut stmt, &[DmValue::Int(i)], &mut arena) {
                    tally.note_cursor(&e);
                    errors.push(e.to_string());
                    break;
                }
                let capacity = arena.capacity();
                if i >= 2 && capacity != last_capacity {
                    grew += 1;
                }
                last_capacity = capacity;
                if let Err(e) = stmt.exec() {
                    tally.note_cursor(&e);
                    errors.push(e.to_string());
                    break;
                }
            }
            tally.check(
                "200 次绑定 + 执行的稳态分配",
                grew == 0 && errors.is_empty(),
                format!(
                    "arena 容量 {} 字节（第 3 次之后增长 {grew} 次），错误 {} 条",
                    arena.capacity(),
                    errors.len()
                ),
            );
            let mut repeat_ok = true;
            for i in 200..205 {
                if bind_params(&mut stmt, &[DmValue::Int(i)], &mut arena).is_err()
                    || stmt.exec().is_err()
                {
                    repeat_ok = false;
                    break;
                }
            }
            tally.check("同一语句连续执行", repeat_ok, "5 次重复绑定 + 执行无状态错");
        }
    }

    // ---- 7. 类型变化触发复位（同一语句、同一位置、两种类型）----------------
    {
        let mut arena = ParamArena::new();
        match conn.allocate_statement() {
            Ok(mut stmt) => {
                if let Err(e) = stmt.prepare(&rep_sql) {
                    tally.check("类型变化复位", false, format!("prepare 失败：{e}"));
                } else {
                    let plan: [DmValue; 3] = [
                        DmValue::Int(300),
                        DmValue::Text("301".to_string()),
                        DmValue::Int(302),
                    ];
                    let mut ok = true;
                    let mut detail = String::new();
                    for value in &plan {
                        if let Err(e) =
                            bind_params(&mut stmt, std::slice::from_ref(value), &mut arena)
                        {
                            tally.note_cursor(&e);
                            ok = false;
                            detail = format!("绑定 {value:?} 失败：{e}");
                            break;
                        }
                        if let Err(e) = stmt.exec() {
                            tally.note_cursor(&e);
                            ok = false;
                            detail = format!("执行 {value:?} 失败：{e}");
                            break;
                        }
                    }
                    tally.check(
                        "类型变化（INT → 文本 → INT）触发复位后仍可执行",
                        ok,
                        if detail.is_empty() {
                            "3 次执行：INT、VARCHAR、INT，无 -70027".to_string()
                        } else {
                            detail
                        },
                    );
                }
            }
            Err(e) => tally.check("类型变化复位", false, format!("分配语句失败：{e}")),
        }
    }
    match scalar(&reader, &format!("SELECT COUNT(*) FROM {rep}")) {
        Ok(Some(n)) => tally.check(
            "重复执行落库行数",
            n == "208",
            format!("count(*)={n}（期望 200+5+3=208）"),
        ),
        other => tally.check("重复执行落库行数", false, format!("{other:?}")),
    }

    // ---- 8. 用法错误提前拒绝 ----------------------------------------------
    {
        let two_sql = format!("INSERT INTO {rep} (ID,EXTRA) VALUES (?,?)");
        match conn.allocate_statement() {
            Ok(mut stmt) => match stmt.prepare(&two_sql) {
                Err(e) => tally.check("参数个数不符提前拒绝", false, format!("prepare 失败：{e}")),
                Ok(()) => match bind_params(&mut stmt, &[DmValue::Int(1)], &mut arena) {
                    Err(e) => {
                        let is_config = matches!(&e.kind, DmErrorKind::Config(_));
                        tally.check(
                            "参数个数不符提前拒绝",
                            is_config && e.context.stage == Stage::Bind,
                            format!(
                                "kind={:?} stage={:?} —— 不把 07002/-70039 留给服务端",
                                e.kind, e.context.stage
                            ),
                        );
                    }
                    Ok(()) => tally.check("参数个数不符提前拒绝", false, "居然绑成功了"),
                },
            },
            Err(e) => tally.check("参数个数不符提前拒绝", false, format!("分配语句失败：{e}")),
        }
    }
    {
        match conn.allocate_statement() {
            Ok(mut stmt) => match bind_params(&mut stmt, &[DmValue::Int(1)], &mut arena) {
                Err(e) => {
                    let is_config = matches!(&e.kind, DmErrorKind::Config(_));
                    tally.check(
                        "未预处理就绑定被拒",
                        is_config && e.context.stage == Stage::Bind,
                        format!("kind={:?}", e.kind),
                    );
                }
                Ok(()) => tally.check("未预处理就绑定被拒", false, "Fresh 语句居然绑成功了"),
            },
            Err(e) => tally.check("未预处理就绑定被拒", false, format!("分配语句失败：{e}")),
        }
    }
    {
        let mut stmt = match conn.allocate_statement() {
            Ok(s) => s,
            Err(e) => {
                tally.check("不支持的类型被拒", false, format!("分配语句失败：{e}"));
                finish(&tally, &reader, &[&table, &rep]);
            }
        };
        let _ = stmt.prepare(&rep_sql);
        match bind_params(&mut stmt, &[DmValue::Unknown(vec![1, 2])], &mut arena) {
            Err(e) => {
                let unsupported = matches!(&e.kind, DmErrorKind::Unsupported { .. });
                tally.check(
                    "未知类型码参数被拒（不静默降级）",
                    unsupported && e.context.stage == Stage::Bind,
                    format!("kind={:?}", e.kind),
                );
            }
            Ok(()) => tally.check("未知类型码参数被拒（不静默降级）", false, "居然绑成功了"),
        }
    }

    finish(&tally, &reader, &[&table, &rep]);
}

/// 收尾：清理临时表、打总结、设退出码。
fn finish(tally: &Tally, reader: &Connection<'_>, tables: &[&String]) -> ! {
    for table in tables {
        let _ = exec_direct(reader, &format!("DROP TABLE {table}"));
    }
    println!("\n===== T3.4 真机验收 =====");
    println!(
        "检查 {} 条，失败 {} 条，`-70027` 出现 {} 次（要求 0）",
        tally.checks, tally.failures, tally.invalid_cursor
    );
    let verdict = tally.failures == 0 && tally.invalid_cursor == 0;
    println!("判定：{}", if verdict { "PASS" } else { "FAIL" });
    std::process::exit(if verdict { 0 } else { 1 });
}
