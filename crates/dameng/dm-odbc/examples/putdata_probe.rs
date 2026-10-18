//! `SQLPutData`（执行时喂参 / `SQL_NEED_DATA`）探针：达梦的 ODBC 驱动到底支不支持。
//!
//! # 为什么要有这个探针
//!
//! 设计方案 §14 的 V-06（分块 LOB **写**）目前是**缓冲式**的：调用方先给一个完整的
//! `DmValue::Bytes`/`Text`，我们一次 `SQLBindParameter` 把整块内存交给驱动。大值
//! （几十 MB 的 BLOB/CLOB）因此必须**整块进内存**——`LobWriter` 也一样，它只是把
//! 拼装做得好一点，最终仍然要一次性绑上去。
//!
//! ODBC 给的标准出路是**执行时喂参**：把参数绑成「延迟输入」——`ColumnSize` 给总长
//! 或 0、指标给 `SQL_LEN_DATA_AT_EXEC(n)`（长度已知）或 `SQL_DATA_AT_EXEC`（未知），
//! 然后 `SQLExecute` 返回 `SQL_NEED_DATA`，调用方反复 `SQLParamData` / `SQLPutData`
//! 按窗口把数据推进去。**内存占用一份窗口，而不是一份值。**
//!
//! 问题是：达梦的驱动到底会不会返回 `SQL_NEED_DATA`？
//!
//! `crates/dm-odbc/src/stmt.rs` 的模块文档说「达梦上不可达」，但那句话的证据只是
//! 两条**别的**路径：`exec_direct` 带 `?` 占位符报 `-6804 需要更多的参数`，
//! `SQLExecute` 绑定数与占位符数不符报 `07002/-70039`。这两条都跟「延迟输入」无关，
//! 所以真到要写这条能力时**必须先问驱动**，不能拿那两句话当结论。
//!
//! # 探六个问题
//!
//! | # | 绑法 | 指标 | 批大小 |
//! |---|------|------|--------|
//! | 1 | `odbc-api` 的 `bind_delayed_input_parameter` + `BlobSlice` | `len_data_at_exec` | 单批（整值） |
//! | 2 | 同上 | `len_data_at_exec` | 4096 字节（多批） |
//! | 3 | 同上，但 `size_hint() == None` | `DATA_AT_EXEC` | 4096 字节 |
//! | 4 | 裸 `SQLBindParameter`（**照本项目的 `type_of`**：`Binary`/`EXT_VAR_BINARY`） | `len_data_at_exec` | 4096 字节 |
//! | 5 | 裸 `SQLBindParameter`，指标 `DATA_AT_EXEC` | `DATA_AT_EXEC` | 4096 字节 |
//! | 6 | 裸 `SQLBindParameter`（**宽字符**：`WChar`/`VARCHAR`，同 `type_of` 的文本臂） | `len_data_at_exec` | 2048 码元 |
//! | 7 | 同 6，但长度未知 | `DATA_AT_EXEC` | 2048 码元 |
//! | 8 | 喂完之后**复用同一条语句**再执行一次普通绑定（池会缓存语句，必须问） | — | — |
//!
//! 1～3 走的是 `odbc-api` 现成的机制（`Blob`/`BlobParam`），4～6 走的是**我们将要
//! 实现的那条路**：自己 `SQLBindParameter` + 自己 `SQLParamData`/`SQLPutData` 循环，
//! token 用我们自己的结构体地址（不需要 `odbc-api` 那个 `*mut &mut dyn Blob` 的
//! 胖指针约定）。
//!
//! 每个用例执行完都**立刻回读**并与源值逐字节比对——「驱动说成功」不等于「数据对了」。
//!
//! 只打印、不断言：它是定位工具，不是门禁。
//!
//! 跑法：
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example putdata_probe
//! ```

use std::io;
use std::num::NonZeroUsize;

use dm_core::{DmConfig, DmValue, RowBuf};
use dm_error::DmError;
use dm_odbc::{FetchScratch, OdbcConnection, OdbcStatement, StmtOutcome, describe_columns, fetch};
use odbc_api::DataType;
use odbc_api::handles::{HasDataType, SqlResult, Statement};
use odbc_api::parameter::{Blob, BlobParam, BlobSlice};
use odbc_sys::{
    CDataType, DATA_AT_EXEC, HStmt, Len, ParamType, Pointer, SQLBindParameter, SQLPutData,
    SqlDataType, SqlReturn, ULen, len_data_at_exec,
};

/// 探针专用表（带前缀，不碰集成夹具与别的探针的 `DM_*` 表）。
const T_B: &str = "DM_ODBC_PD_PROBE_B";
const T_C: &str = "DM_ODBC_PD_PROBE_C";
const INSERT_B: &str = "insert into DM_ODBC_PD_PROBE_B (B) values (?)";
const INSERT_C: &str = "insert into DM_ODBC_PD_PROBE_C (C) values (?)";

/// 二进制载荷：100 KiB + 7 字节——故意不是批大小的整数倍，末批必须短。
const BLOB_LEN: usize = 100 * 1024 + 7;
/// 批大小（字节）。
const BATCH_BYTES: usize = 4096;
/// 宽字符每批码元数（与 `crates/dm-odbc/src/result.rs` 的 `WIDE_CHUNK_UNITS` 一致）。
const BATCH_UNITS: usize = 2048;
/// 文本载荷重复次数；只用 BMP 字符——非 BMP 是**另一个**已知缺陷（§4.8），
/// 混进来会把「喂参能不能用」和「非 BMP 保不保真」两件事搅在一起。
const TEXT_REPEAT: usize = 4000;

fn main() {
    let mut conn = connect();
    let data: Vec<u8> = (0..BLOB_LEN).map(|i| (i * 31 % 251) as u8).collect();
    let text = "达梦数据库DM8-中文-".repeat(TEXT_REPEAT);

    println!(
        "二进制载荷 {} 字节（批 {}）；文本载荷 {} 字节（批 {BATCH_UNITS} 码元）",
        data.len(),
        BATCH_BYTES,
        text.len()
    );

    setup(&mut conn);

    odbc_style(
        &mut conn,
        "1. odbc-api / Binary / 单批（BlobSlice::from_byte_slice）",
        &data,
        &mut BlobSlice::from_byte_slice(&data),
    );
    odbc_style(
        &mut conn,
        &format!("2. odbc-api / Binary / 多批（batch_size = {BATCH_BYTES}）"),
        &data,
        &mut BlobSlice {
            is_binary: true,
            batch_size: BATCH_BYTES,
            blob: &data,
        },
    );
    odbc_style(
        &mut conn,
        "3. odbc-api / Binary / size_hint() == None（指标 SQL_DATA_AT_EXEC）",
        &data,
        &mut UnknownLen {
            data: &data,
            batch: BATCH_BYTES,
            pos: 0,
        },
    );

    raw_binary(&mut conn, &data, true);
    raw_binary(&mut conn, &data, false);
    raw_text(&mut conn, &text, true);
    raw_text(&mut conn, &text, false);
    reuse_after_delayed(&mut conn, &data);

    teardown(&mut conn);
    println!("\n探针结束：请人工看每一节「驱动返回什么」与「回读是否一致」。");
}

// ---------------------------------------------------------------------------
// 连接与建表
// ---------------------------------------------------------------------------

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
    let mut buf = RowBuf::with_capacity(columns.len(), 8);
    let mut scratch = FetchScratch::new();
    loop {
        if fetch(stmt, &columns, &mut buf, &mut scratch)?.is_exhausted() {
            return Ok(());
        }
    }
}

fn setup(conn: &mut OdbcConnection) {
    for sql in [
        format!("drop table if exists {T_B}"),
        format!("create table {T_B} (B BLOB)"),
        format!("drop table if exists {T_C}"),
        format!("create table {T_C} (C CLOB)"),
    ] {
        if let Err(err) = exec(conn, &sql) {
            println!("建表语句失败：{sql}\n  {err}");
        }
    }
}

fn teardown(conn: &mut OdbcConnection) {
    for sql in [format!("drop table {T_B}"), format!("drop table {T_C}")] {
        if let Err(err) = exec(conn, &sql) {
            println!("清理失败：{sql}\n  {err}");
        }
    }
}

/// 清空表：每个用例回读时只应看见自己这一行。
fn clear(conn: &mut OdbcConnection, table: &str) {
    if let Err(err) = exec(conn, &format!("delete from {table}")) {
        println!("清空 {table} 失败：{err}");
    }
}

// ---------------------------------------------------------------------------
// 执行 + 喂数据循环
// ---------------------------------------------------------------------------

/// 一次 `SQLPutData` 的结果。
enum Fed {
    /// 喂进去了这么多字节。
    Bytes(usize),
    /// 源已空。
    Exhausted,
    /// 驱动拒绝了这一批（原始返回码已经打印过）。
    Failed,
}

/// 执行语句；若驱动返回 `SQL_NEED_DATA`，就用 `next` 反复喂批。
///
/// 这就是我们将要实现的那条循环的最小形状：**`SQLParamData` 一次给出一个需要数据的
/// 参数**，该参数的所有批用 `SQLPutData` 推完，然后再问一次 `SQLParamData`。
fn run_delayed(stmt: &mut OdbcStatement<'_>, mut next: impl FnMut(Pointer) -> Fed) {
    let executed = unsafe { stmt.statement().execute() };
    println!("  SQLExecute -> {executed:?}");
    match executed {
        SqlResult::Success(()) | SqlResult::SuccessWithInfo(()) => {
            println!("  ⚠️ 驱动**没有**要数据（没有 SQL_NEED_DATA）");
            return;
        }
        SqlResult::NeedData => println!("  ⭐ 驱动返回 SQL_NEED_DATA：这条路驱动认得"),
        SqlResult::Error { function } => {
            show_error(stmt, function);
            return;
        }
        other => {
            println!("  非预期返回：{other:?}");
            return;
        }
    }

    let mut total = 0usize;
    let mut rounds = 0usize;
    loop {
        rounds += 1;
        if rounds > 1_000_000 {
            println!("  ⚠️ 问了 100 万次 SQLParamData，停下");
            return;
        }
        match stmt.statement().param_data() {
            SqlResult::Success(Some(token)) => {
                let mut fed = 0usize;
                let mut batches = 0usize;
                loop {
                    match next(token) {
                        Fed::Bytes(n) => {
                            fed += n;
                            batches += 1;
                        }
                        Fed::Exhausted => break,
                        Fed::Failed => {
                            println!("  ⚠️ SQLPutData 失败，停下");
                            return;
                        }
                    }
                }
                total += fed;
                println!(
                    "  SQLParamData -> NEED_DATA(token={token:?})：喂了 {batches} 批 / {fed} 字节"
                );
                if fed == 0 {
                    println!("  ⚠️ 驱动还在要数据，但源已空 → 停下");
                    return;
                }
            }
            SqlResult::Success(None) => {
                println!("  SQLParamData -> SUCCESS：喂完，共 {total} 字节");
                return;
            }
            SqlResult::Error { function } => {
                show_error(stmt, function);
                return;
            }
            other => {
                println!("  SQLParamData -> {other:?}（停止）");
                return;
            }
        }
    }
}

/// 推一批数据（`SQLPutData`）。`hstmt` 是裸句柄，因此闭包里不必再借语句。
fn put_batch(hstmt: HStmt, batch: &[u8]) -> Fed {
    // SAFETY：`hstmt` 来自一个活着的语句；`batch` 在本调用期间有效；长度按字节给。
    let ret = unsafe { SQLPutData(hstmt, batch.as_ptr() as Pointer, batch.len() as Len) };
    match ret {
        SqlReturn::SUCCESS | SqlReturn::SUCCESS_WITH_INFO => Fed::Bytes(batch.len()),
        other => {
            println!("  SQLPutData -> {other:?}");
            Fed::Failed
        }
    }
}

/// 打印一条 ODBC 诊断（`odbc-api` 的 `Error` 会把 state / 原生码 / 消息都带出来）。
fn show_error(stmt: &mut OdbcStatement<'_>, function: &'static str) {
    let raw: SqlResult<()> = SqlResult::Error { function };
    match raw.into_result(stmt.statement()) {
        Ok(()) => println!("  {function} 报错，但诊断是空的"),
        Err(err) => println!("  {function} 报错：{err}"),
    }
}

// ---------------------------------------------------------------------------
// 1～3：odbc-api 风格（Blob / BlobParam）
// ---------------------------------------------------------------------------

fn odbc_style(conn: &mut OdbcConnection, label: &str, expect: &[u8], blob: &mut impl Blob) {
    println!("\n== {label} ==");
    clear(conn, T_B);

    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("  申请语句失败：{err}");
            return;
        }
    };
    if let Err(err) = stmt.prepare(INSERT_B) {
        println!("  prepare 失败：{err}");
        return;
    }

    // `param` 一旦绑定就**不能移动**：驱动存下的 token 是它内部那个
    // `&mut dyn Blob` 字段的地址（`BlobParam::stream_ptr` 的胖指针取址）。
    let mut param = BlobParam::new(blob);
    let bind = unsafe { stmt.statement().bind_delayed_input_parameter(1, &mut param) };
    println!("  SQLBindParameter(delayed) -> {bind:?}");
    if bind.is_err() {
        show_error(&mut stmt, "SQLBindParameter");
        return;
    }

    let hstmt: HStmt = stmt.statement().as_sys();
    run_delayed(&mut stmt, |token| {
        // 与 odbc-api 自己的 `next_blob_param` 同一套约定：token 就是
        // `*mut &mut dyn Blob`（指向 `BlobParam` 里那个胖指针）。
        let blob_ptr = token as *mut &mut dyn Blob;
        // SAFETY：token 是上一步 `bind_delayed_input_parameter` 交给驱动的
        // `stream_ptr()`，且 `param` 在整个循环期间活着、没有移动。
        let blob: &mut dyn Blob = unsafe { *blob_ptr };
        match blob.next_batch() {
            Ok(Some(batch)) => put_batch(hstmt, batch),
            Ok(None) => Fed::Exhausted,
            Err(err) => {
                println!("  取批失败：{err}");
                Fed::Failed
            }
        }
    });

    drop(stmt);
    read_back_binary(conn, expect);
}

/// 长度未知的二进制源：`size_hint() == None` ⇒ `BlobParam` 的指标是 `SQL_DATA_AT_EXEC`。
struct UnknownLen<'a> {
    data: &'a [u8],
    batch: usize,
    pos: usize,
}

impl HasDataType for UnknownLen<'_> {
    fn data_type(&self) -> DataType {
        DataType::LongVarbinary {
            length: NonZeroUsize::new(self.data.len().max(1)),
        }
    }
}

// SAFETY：`size_hint()` 返回 `None`，所以没有「提示必须准确」的义务；每批都非空。
unsafe impl Blob for UnknownLen<'_> {
    fn c_data_type(&self) -> CDataType {
        CDataType::Binary
    }

    fn size_hint(&self) -> Option<usize> {
        None
    }

    fn next_batch(&mut self) -> io::Result<Option<&[u8]>> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let end = (self.pos + self.batch).min(self.data.len());
        let batch = &self.data[self.pos..end];
        self.pos = end;
        Ok(Some(batch))
    }
}

// ---------------------------------------------------------------------------
// 4/5：裸绑二进制（照本项目的 type_of：CDataType::Binary / SqlDataType::EXT_VAR_BINARY）
// ---------------------------------------------------------------------------

fn raw_binary(conn: &mut OdbcConnection, data: &[u8], known_len: bool) {
    let indicator = if known_len {
        len_data_at_exec(Len::try_from(data.len()).expect("长度超过 isize 上限"))
    } else {
        DATA_AT_EXEC
    };
    println!(
        "\n== 4/5 裸绑 Binary（ctype=Binary, sqltype=EXT_VAR_BINARY, ColumnSize={}, 指标={indicator}）==",
        data.len()
    );
    clear(conn, T_B);

    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("  申请语句失败：{err}");
            return;
        }
    };
    if let Err(err) = stmt.prepare(INSERT_B) {
        println!("  prepare 失败：{err}");
        return;
    }

    let mut src = BinSource {
        data,
        batch: BATCH_BYTES,
        pos: 0,
    };
    let token: *mut BinSource<'_> = &mut src;
    let token_addr = token as Pointer;
    let mut ind: Len = indicator;
    let hstmt: HStmt = stmt.statement().as_sys();
    // SAFETY：`ind` 与 `src` 都比驱动使用它们活得更久（本函数栈上，直到循环结束）；
    // `BufferLength` 对延迟参数无意义，给 0；值缓冲给 NULL（`ParameterValuePtr` 这里
    // 是 token，不是数据）。
    let ret = unsafe {
        SQLBindParameter(
            hstmt,
            1,
            ParamType::Input,
            CDataType::Binary,
            SqlDataType::EXT_VAR_BINARY,
            data.len() as ULen,
            0,
            token_addr,
            0,
            &mut ind as *mut Len,
        )
    };
    println!("  SQLBindParameter(raw) -> {ret:?}");
    if ret != SqlReturn::SUCCESS && ret != SqlReturn::SUCCESS_WITH_INFO {
        show_error(&mut stmt, "SQLBindParameter");
        return;
    }

    run_delayed(&mut stmt, |token| {
        if token != token_addr {
            println!("  ⚠️ 驱动给的 token 与绑定的不一致：{token:?} != {token_addr:?}");
        }
        // SAFETY：token 就是我们绑定时给的 `&mut BinSource`。
        let src: &mut BinSource<'_> = unsafe { &mut *(token as *mut BinSource<'_>) };
        match src.next_batch() {
            Some(batch) => put_batch(hstmt, batch),
            None => Fed::Exhausted,
        }
    });

    drop(stmt);
    read_back_binary(conn, data);
}

/// 我们自己那条路的喂数据源：token 就是这个结构体的地址。
struct BinSource<'a> {
    data: &'a [u8],
    batch: usize,
    pos: usize,
}

impl BinSource<'_> {
    fn next_batch(&mut self) -> Option<&[u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let end = (self.pos + self.batch).min(self.data.len());
        let batch = &self.data[self.pos..end];
        self.pos = end;
        Some(batch)
    }
}

// ---------------------------------------------------------------------------
// 6：裸绑宽字符（照 type_of 的文本臂：CDataType::WChar / SqlDataType::VARCHAR）
// ---------------------------------------------------------------------------

fn raw_text(conn: &mut OdbcConnection, text: &str, known_len: bool) {
    let mut src = WideTextSource::new(text, BATCH_UNITS);
    let total = src.total_bytes();
    // 长度已知：指标给总字节数、`ColumnSize` 给**码元数**（`type_of` 文本臂的口径）。
    // 长度未知：指标给 `SQL_DATA_AT_EXEC`、`ColumnSize` 给 0——调用方流式产生文本时
    // 事先不知道总长，这是那条路必须成立的形状。
    let (indicator, column_size) = if known_len {
        (
            len_data_at_exec(Len::try_from(total).expect("长度超过 isize 上限")),
            (total / 2) as ULen,
        )
    } else {
        (DATA_AT_EXEC, 0)
    };
    println!(
        "\n== 6/7 裸绑宽字符（ctype=WChar, sqltype=VARCHAR, 指标={indicator}, ColumnSize={column_size}, 长度已知={known_len}）=="
    );
    clear(conn, T_C);

    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("  申请语句失败：{err}");
            return;
        }
    };
    if let Err(err) = stmt.prepare(INSERT_C) {
        println!("  prepare 失败：{err}");
        return;
    }

    let token: *mut WideTextSource = &mut src;
    let token_addr = token as Pointer;
    let mut ind: Len = indicator;
    let hstmt: HStmt = stmt.statement().as_sys();
    // SAFETY：同 4——`ind`/`src` 活得比驱动使用它们更久。
    let ret = unsafe {
        SQLBindParameter(
            hstmt,
            1,
            ParamType::Input,
            CDataType::WChar,
            SqlDataType::VARCHAR,
            column_size,
            0,
            token_addr,
            0,
            &mut ind as *mut Len,
        )
    };
    println!("  SQLBindParameter(raw) -> {ret:?}");
    if ret != SqlReturn::SUCCESS && ret != SqlReturn::SUCCESS_WITH_INFO {
        show_error(&mut stmt, "SQLBindParameter");
        return;
    }

    run_delayed(&mut stmt, |token| {
        if token != token_addr {
            println!("  ⚠️ 驱动给的 token 与绑定的不一致：{token:?} != {token_addr:?}");
        }
        // SAFETY：同 4。
        let src: &mut WideTextSource = unsafe { &mut *(token as *mut WideTextSource) };
        match src.next_batch() {
            Some(batch) => put_batch(hstmt, batch),
            None => Fed::Exhausted,
        }
    });

    drop(stmt);
    read_back_text(conn, text);
}

// ---------------------------------------------------------------------------
// 8：喂完之后复用同一条语句
// ---------------------------------------------------------------------------

/// 延迟输入喂完之后，同一条语句还能不能拿来跑普通绑定？
///
/// 这不是学术问题：`dm-pool` 的语句缓存会**留下**执行过的语句，下一次可能用它跑一条
/// 完全不同的 SQL。若「喂过数据的语句」此后只能作废，实现里就必须显式把它从缓存里
/// 摘掉（像 Round 11 的 `statement_reusable` 那条判据一样，是一条真规则）。
fn reuse_after_delayed(conn: &mut OdbcConnection, data: &[u8]) {
    println!("\n== 8 延迟输入之后复用同一条语句（池会缓存语句） ==");
    clear(conn, T_B);

    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("  申请语句失败：{err}");
            return;
        }
    };
    if let Err(err) = stmt.prepare(INSERT_B) {
        println!("  prepare 失败：{err}");
        return;
    }

    // 第一次：延迟输入喂整块。
    let mut src = BinSource {
        data,
        batch: BATCH_BYTES,
        pos: 0,
    };
    let token_addr = &mut src as *mut BinSource<'_> as Pointer;
    let mut ind: Len = len_data_at_exec(Len::try_from(data.len()).expect("长度超过 isize 上限"));
    let hstmt: HStmt = stmt.statement().as_sys();
    // SAFETY：同 `raw_binary`。
    let ret = unsafe {
        SQLBindParameter(
            hstmt,
            1,
            ParamType::Input,
            CDataType::Binary,
            SqlDataType::EXT_VAR_BINARY,
            data.len() as ULen,
            0,
            token_addr,
            0,
            &mut ind as *mut Len,
        )
    };
    println!("  第一次 SQLBindParameter(delayed) -> {ret:?}");
    // 第二条连接专门用来「从旁边看」：`stmt` 借着 `conn`，同一连接上读不了。
    let mut watcher = connect();
    println!(
        "  喂数据之前，表里 {} 行",
        count(conn_guard(&mut watcher), T_B)
    );
    run_delayed(&mut stmt, |token| {
        // SAFETY：token 就是上面绑的 `&mut BinSource`。
        let src: &mut BinSource<'_> = unsafe { &mut *(token as *mut BinSource<'_>) };
        match src.next_batch() {
            Some(batch) => put_batch(hstmt, batch),
            None => Fed::Exhausted,
        }
    });
    println!("  喂数据之后，表里 {} 行", count(&mut watcher, T_B));
    println!("  第一次 SQLRowCount -> {:?}", stmt.affected_rows());

    // 第二次：复位参数，改绑一个小值，再执行一次。
    match stmt.reset_parameters() {
        Ok(()) => println!("  参数复位 -> OK"),
        Err(err) => {
            println!("  参数复位失败：{err}");
            return;
        }
    }
    let small: &[u8] = b"reuse-after-delayed";
    let mut ind2: Len = small.len() as Len;
    // SAFETY：`small` 与 `ind2` 都活到本次执行结束；`hstmt` 有效。
    let ret = unsafe {
        SQLBindParameter(
            hstmt,
            1,
            ParamType::Input,
            CDataType::Binary,
            SqlDataType::EXT_VAR_BINARY,
            small.len() as ULen,
            0,
            small.as_ptr() as Pointer,
            small.len() as Len,
            &mut ind2 as *mut Len,
        )
    };
    println!("  第二次 SQLBindParameter(普通) -> {ret:?}");
    // SAFETY：参数缓冲 `small`/`ind2` 在本次调用期间有效。
    let res = unsafe { stmt.statement().execute() };
    println!("  第二次 SQLExecute -> {res:?}");
    if let SqlResult::Error { function } = res {
        show_error(&mut stmt, function);
    }
    println!("  第二次 SQLRowCount -> {:?}", stmt.affected_rows());
    println!("  第二次执行之后，表里 {} 行", count(&mut watcher, T_B));

    drop(stmt);
    println!("  回读两行的长度：");
    read_back_sizes(conn);
}

/// 数一张表有多少行（给「旁观」连接用）。
fn count(conn: &mut OdbcConnection, table: &str) -> usize {
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("    计数失败：申请语句失败：{err}");
            return 0;
        }
    };
    let outcome = match stmt.exec_direct(&format!("select count(*) as N from {table}")) {
        Ok(outcome) => outcome,
        Err(err) => {
            println!("    计数失败：{err}");
            return 0;
        }
    };
    if !matches!(outcome, StmtOutcome::ResultSet { .. }) {
        println!("    计数失败：不是结果集：{outcome:?}");
        return 0;
    }
    let columns = match describe_columns(&mut stmt, None) {
        Ok(columns) => columns,
        Err(err) => {
            println!("    计数失败：取列元数据失败：{err}");
            return 0;
        }
    };
    let mut buf = RowBuf::with_capacity(columns.len(), 8);
    let mut scratch = FetchScratch::new();
    if let Err(err) = fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
        println!("    计数失败：{err}");
        return 0;
    }
    match buf.rows().next().map(|row| &row[0]) {
        Some(DmValue::Int(n)) => usize::try_from(*n).unwrap_or(0),
        Some(DmValue::BigInt(n)) => usize::try_from(*n).unwrap_or(0),
        Some(other) => {
            println!("    计数：类型是 {}", type_name(other));
            0
        }
        None => 0,
    }
}

/// 给一个借用来看看的名字，避免 `conn_guard` 变成空壳。
fn conn_guard(conn: &mut OdbcConnection) -> &mut OdbcConnection {
    conn
}

/// 把表里的 B 列逐行长度打出来（不比对内容）。
fn read_back_sizes(conn: &mut OdbcConnection) {
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("    回读失败：申请语句失败：{err}");
            return;
        }
    };
    let outcome = match stmt.exec_direct(&format!("select B from {T_B}")) {
        Ok(outcome) => outcome,
        Err(err) => {
            println!("    回读失败：{err}");
            return;
        }
    };
    if !matches!(outcome, StmtOutcome::ResultSet { .. }) {
        println!("    回读失败：不是结果集：{outcome:?}");
        return;
    }
    let columns = match describe_columns(&mut stmt, None) {
        Ok(columns) => columns,
        Err(err) => {
            println!("    回读失败：取列元数据失败：{err}");
            return;
        }
    };
    let mut buf = RowBuf::with_capacity(columns.len(), 8);
    let mut scratch = FetchScratch::new();
    let mut index = 0usize;
    loop {
        match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
            Ok(state) if state.is_exhausted() => break,
            Ok(_) => {}
            Err(err) => {
                println!("    回读失败：{err}");
                return;
            }
        }
        for row in buf.rows() {
            match &row[0] {
                DmValue::Bytes(bytes) => println!("    第 {index} 行：{} 字节", bytes.len()),
                DmValue::Null => println!("    第 {index} 行：NULL"),
                other => println!("    第 {index} 行：{}", type_name(other)),
            }
            index += 1;
        }
    }
}

/// 宽字符喂数据源：内部就是 UTF-16LE 的字节流，按**码元**切，绝不切开代理对。
struct WideTextSource {
    le: Vec<u8>,
    batch_bytes: usize,
    pos: usize,
}

impl WideTextSource {
    fn new(text: &str, batch_units: usize) -> Self {
        let mut le = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            le.extend_from_slice(&unit.to_le_bytes());
        }
        Self {
            le,
            batch_bytes: batch_units * 2,
            pos: 0,
        }
    }

    fn total_bytes(&self) -> usize {
        self.le.len()
    }

    fn next_batch(&mut self) -> Option<&[u8]> {
        if self.pos >= self.le.len() {
            return None;
        }
        let mut end = (self.pos + self.batch_bytes).min(self.le.len());
        // 整码元。
        end -= (end - self.pos) % 2;
        // 不切开代理对：若最后一个码元是低代理，就退一个码元（留给下一批）。
        while end > self.pos + 2 {
            let unit = u16::from_le_bytes([self.le[end - 2], self.le[end - 1]]);
            if (0xDC00..=0xDFFF).contains(&unit) {
                end -= 2;
            } else {
                break;
            }
        }
        if end <= self.pos {
            return None;
        }
        let batch = &self.le[self.pos..end];
        self.pos = end;
        Some(batch)
    }
}

// ---------------------------------------------------------------------------
// 回读：逐字节比对
// ---------------------------------------------------------------------------

fn read_back_binary(conn: &mut OdbcConnection, expect: &[u8]) {
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("  回读失败：申请语句失败：{err}");
            return;
        }
    };
    let outcome = match stmt.exec_direct(&format!("select B from {T_B}")) {
        Ok(outcome) => outcome,
        Err(err) => {
            println!("  回读失败：{err}");
            return;
        }
    };
    if !matches!(outcome, StmtOutcome::ResultSet { .. }) {
        println!("  回读失败：不是结果集：{outcome:?}");
        return;
    }
    let columns = match describe_columns(&mut stmt, None) {
        Ok(columns) => columns,
        Err(err) => {
            println!("  回读失败：取列元数据失败：{err}");
            return;
        }
    };
    let mut buf = RowBuf::with_capacity(columns.len(), 8);
    let mut scratch = FetchScratch::new();
    let mut saw_row = false;
    let mut index = 0usize;
    loop {
        match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
            Ok(state) if state.is_exhausted() => break,
            Ok(_) => {}
            Err(err) => {
                println!("  回读失败：{err}");
                return;
            }
        }
        for row in buf.rows() {
            saw_row = true;
            match &row[0] {
                DmValue::Bytes(bytes) => println!(
                    "  回读行 {index}：{} 字节（期望 {}）→ {}",
                    bytes.len(),
                    expect.len(),
                    if bytes.as_slice() == expect {
                        "一致 ✅"
                    } else {
                        "不一致 ❌"
                    }
                ),
                other => println!(
                    "  回读行 {index}：类型不是 Bytes，而是 {} ❌",
                    type_name(other)
                ),
            }
            index += 1;
        }
    }
    if !saw_row {
        println!("  回读：表里没有行 ❌（写入没落地）");
    }
}

fn read_back_text(conn: &mut OdbcConnection, expect: &str) {
    let mut stmt = match conn.allocate_statement() {
        Ok(stmt) => stmt,
        Err(err) => {
            println!("  回读失败：申请语句失败：{err}");
            return;
        }
    };
    let outcome = match stmt.exec_direct(&format!("select C from {T_C}")) {
        Ok(outcome) => outcome,
        Err(err) => {
            println!("  回读失败：{err}");
            return;
        }
    };
    if !matches!(outcome, StmtOutcome::ResultSet { .. }) {
        println!("  回读失败：不是结果集：{outcome:?}");
        return;
    }
    let columns = match describe_columns(&mut stmt, None) {
        Ok(columns) => columns,
        Err(err) => {
            println!("  回读失败：取列元数据失败：{err}");
            return;
        }
    };
    let mut buf = RowBuf::with_capacity(columns.len(), 8);
    let mut scratch = FetchScratch::new();
    let mut saw_row = false;
    let mut index = 0usize;
    loop {
        match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
            Ok(state) if state.is_exhausted() => break,
            Ok(_) => {}
            Err(err) => {
                println!("  回读失败：{err}");
                return;
            }
        }
        for row in buf.rows() {
            saw_row = true;
            match &row[0] {
                DmValue::Text(text) => {
                    let verdict = if text.as_str() == expect {
                        "一致 ✅".to_string()
                    } else {
                        format!(
                            "不一致 ❌（回读长度 {}，前 24 字符：{:?}）",
                            text.len(),
                            text.chars().take(24).collect::<String>()
                        )
                    };
                    println!("  回读行 {index}：{verdict}");
                }
                other => println!(
                    "  回读行 {index}：类型不是 Text，而是 {} ❌",
                    type_name(other)
                ),
            }
            index += 1;
        }
    }
    if !saw_row {
        println!("  回读：表里没有行 ❌（写入没落地）");
    }
}

fn type_name(value: &DmValue) -> &'static str {
    match value {
        DmValue::Null => "Null",
        DmValue::Bool(_) => "Bool",
        DmValue::TinyInt(_) => "TinyInt",
        DmValue::SmallInt(_) => "SmallInt",
        DmValue::Int(_) => "Int",
        DmValue::BigInt(_) => "BigInt",
        DmValue::Real(_) => "Real",
        DmValue::Double(_) => "Double",
        DmValue::Decimal(_) => "Decimal",
        DmValue::Text(_) => "Text",
        DmValue::Bytes(_) => "Bytes",
        DmValue::Date(_) => "Date",
        DmValue::Time(_) => "Time",
        DmValue::Timestamp(_) => "Timestamp",
        DmValue::Interval(_) => "Interval",
        DmValue::Unknown(_) => "Unknown",
        _ => "未知",
    }
}
