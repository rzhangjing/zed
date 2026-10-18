//! Minimal smoke probe: `odbc-api` 29.2 against a local DM8 server through the
//! "DM8 ODBC DRIVER" ODBC driver.
//!
//! Answers the 10 questions of the spike one by one and prints everything in a
//! line-oriented, paste-friendly format. Nothing is inferred: every line below is
//! the direct result of a real ODBC call.
//!
//! Build modes (see Cargo.toml):
//!   cargo run                    -> on Windows this is the WIDE build (SQL*W / UTF-16)
//!   cargo run --features narrow  -> NARROW build (SQL*A / bytes assumed to be UTF-8)

use std::time::Instant;

use odbc_api::handles::{Environment as RawEnv, SqlResult, SqlText, Statement};
use odbc_api::sys::{self, AttrOdbcVersion, Handle, HandleType, InfoType, StatementAttribute};
use odbc_api::{
    BindParamDesc, ColumnDescription, ConnectionOptions, Cursor, Environment, IntoParameter,
    ResultSetMetadata, Utf16String,
};

/// Which character path odbc-api compiles to. Mirrors odbc-api's own cfg expression
/// `any(feature = "wide", all(not(feature = "narrow"), target_os = "windows"))`.
#[cfg(all(not(feature = "narrow"), target_os = "windows"))]
const WIDE: bool = true;
#[cfg(any(feature = "narrow", not(target_os = "windows")))]
const WIDE: bool = false;

const CONN_BASE: &str =
    "DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA";

/// Test literal used throughout the probe.
const ZH: &str = "中文测试";
/// Reference encodings of `ZH`:
///   UTF-8          : E4 B8 AD E6 96 87 E6 B5 8B E8 AF 95
///   GB18030 / CP936: D6 D0 CE C4 B2 E2 CA D4
const ZH_UTF8: [u8; 12] = [
    0xE4, 0xB8, 0xAD, 0xE6, 0x96, 0x87, 0xE6, 0xB5, 0x8B, 0xE8, 0xAF, 0x95,
];
const ZH_GBK: [u8; 8] = [0xD6, 0xD0, 0xCE, 0xC4, 0xB2, 0xE2, 0xCA, 0xD4];
const ZH_UTF16: [u16; 4] = [0x4E2D, 0x6587, 0x6D4B, 0x8BD5];

const TABLE: &str = "t_odbcprobe";
const TABLE2: &str = "t_odbcprobe2";

// ---------------------------------------------------------------------------
// small formatting helpers
// ---------------------------------------------------------------------------

fn hex8(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{:02X}", x))
        .collect::<Vec<_>>()
        .join(" ")
}

fn hex16(b: &[u16]) -> String {
    b.iter()
        .map(|x| format!("{:04X}", x))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sr_label(r: sys::SqlReturn) -> String {
    if r == sys::SqlReturn::SUCCESS {
        "SQL_SUCCESS(0)".to_string()
    } else if r == sys::SqlReturn::SUCCESS_WITH_INFO {
        "SQL_SUCCESS_WITH_INFO(1)".to_string()
    } else if r == sys::SqlReturn::ERROR {
        "SQL_ERROR(-1)".to_string()
    } else if r == sys::SqlReturn::INVALID_HANDLE {
        "SQL_INVALID_HANDLE(-2)".to_string()
    } else if r == sys::SqlReturn::STILL_EXECUTING {
        "SQL_STILL_EXECUTING(2)".to_string()
    } else if r == sys::SqlReturn::NO_DATA {
        "SQL_NO_DATA(100)".to_string()
    } else {
        format!("OTHER({})", r.0)
    }
}

/// Label for `odbc-api`'s `SqlResult`, which discards the raw return code.
fn label<T>(r: &SqlResult<T>) -> String {
    match r {
        SqlResult::Success(_) => "SQL_SUCCESS".to_string(),
        SqlResult::SuccessWithInfo(_) => "SQL_SUCCESS_WITH_INFO".to_string(),
        SqlResult::NoData => "SQL_NO_DATA".to_string(),
        SqlResult::NeedData => "SQL_NEED_DATA".to_string(),
        SqlResult::StillExecuting => "SQL_STILL_EXECUTING".to_string(),
        SqlResult::Error { function } => format!("SQL_ERROR (from {})", function),
    }
}

/// Run a probe section, turning any `odbc-api` internal panic (some drivers return
/// `SQL_NO_DATA` / `SQL_NEED_DATA` in places where the crate only panics) into a printed
/// finding instead of aborting the whole probe.
fn guard<F: FnOnce()>(tag: &str, f: F) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    if let Err(e) = r {
        let msg = e
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        println!("[{tag}] *** odbc-api PANICKED (caught, probe continues): {msg}");
    }
}

/// Pull every diagnostic record off a raw ODBC handle with `SQLGetDiagRecW`.
fn diag_text(ht: HandleType, h: Handle) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut rec: i16 = 1;
    loop {
        let mut state = [0u16; 6];
        let mut native: i32 = 0;
        let mut msg = vec![0u16; 1024];
        let rc = unsafe {
            sys::SQLGetDiagRecW(
                ht,
                h,
                rec,
                state.as_mut_ptr(),
                &mut native,
                msg.as_mut_ptr(),
                (msg.len() * 2) as i16,
                std::ptr::null_mut(),
            )
        };
        if rc != sys::SqlReturn::SUCCESS {
            break;
        }
        let st: String = String::from_utf16_lossy(&state[..5]);
        let end = msg.iter().position(|&c| c == 0).unwrap_or(msg.len());
        out.push(format!(
            "SQLSTATE={} native={} msg=\"{}\"",
            st.trim_matches('\0'),
            native,
            String::from_utf16_lossy(&msg[..end])
        ));
        rec += 1;
        if rec > 8 {
            break;
        }
    }
    if out.is_empty() {
        "<no diagnostic records>".to_string()
    } else {
        out.join(" | ")
    }
}

fn show_narrow(label: &str, b: &[u8]) {
    println!("{:>30}: len={} bytes=[{}]", label, b.len(), hex8(b));
    match std::str::from_utf8(b) {
        Ok(s) => println!("{:>30}: from_utf8 OK  -> {}", label, s),
        Err(e) => println!("{:>30}: from_utf8 ERR -> {}", label, e),
    }
    println!(
        "{:>30}: from_utf8_lossy -> {}",
        label,
        String::from_utf8_lossy(b)
    );
    if b == ZH_UTF8 {
        println!("{:>30}: == expected UTF-8 of \"{}\"", label, ZH);
    }
    if b == ZH_GBK {
        println!("{:>30}: == expected GB18030/CP936 of \"{}\"", label, ZH);
    }
}

fn show_wide(label: &str, w: &[u16]) {
    println!("{:>30}: len={} u16=[{}]", label, w.len(), hex16(w));
    match String::from_utf16(w) {
        Ok(s) => println!("{:>30}: from_utf16 OK  -> {}", label, s),
        Err(e) => println!("{:>30}: from_utf16 ERR -> {}", label, e),
    }
    if w == ZH_UTF16 {
        println!("{:>30}: == expected UTF-16 of \"{}\"", label, ZH);
    }
}

// ---------------------------------------------------------------------------
// query helpers
// ---------------------------------------------------------------------------

/// Execute a statement for its side effect only; prints the outcome.
fn exec(conn: &odbc_api::Connection<'_>, tag: &str, sql: &str) {
    match conn.execute(sql, (), None) {
        Ok(_) => println!("[{tag}] OK    :: {sql}"),
        Err(e) => println!("[{tag}] ERROR :: {sql}\n[{tag}]   display: {e}\n[{tag}]   debug  : {e:?}"),
    }
}

/// `(rc, value)` of the first column of the first row, fetched as narrow text.
fn scalar_narrow(conn: &odbc_api::Connection<'_>, sql: &str) -> Result<Option<String>, String> {
    match conn.execute(sql, (), None) {
        Ok(Some(mut cur)) => match cur.next_row() {
            Ok(Some(mut row)) => {
                let mut buf = Vec::new();
                match row.get_text(1, &mut buf) {
                    Ok(true) => Ok(Some(String::from_utf8_lossy(&buf).into_owned())),
                    Ok(false) => Ok(None),
                    Err(e) => Err(format!("get_text: {e}")),
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("next_row: {e}")),
        },
        Ok(None) => Ok(None),
        Err(e) => Err(format!("execute: {e}")),
    }
}

/// Fetch the whole result set as narrow text columns.
fn fetch_rows(
    conn: &odbc_api::Connection<'_>,
    sql: &str,
) -> Result<Vec<Vec<Option<String>>>, odbc_api::Error> {
    let mut out = Vec::new();
    if let Some(mut cur) = conn.execute(sql, (), None)? {
        let n = cur.num_result_cols()?;
        while let Some(mut row) = cur.next_row()? {
            let mut r = Vec::new();
            for i in 1..=n as u16 {
                let mut buf = Vec::new();
                if row.get_text(i, &mut buf)? {
                    r.push(Some(String::from_utf8_lossy(&buf).into_owned()));
                } else {
                    r.push(None);
                }
            }
            out.push(r);
        }
    }
    Ok(out)
}

/// Dump every piece of column metadata `odbc-api` can express for a cursor.
fn dump_cols(tag: &str, cur: &mut impl ResultSetMetadata) -> Result<(), odbc_api::Error> {
    let n = cur.num_result_cols()?;
    println!("[{tag}] num_result_cols = {n}");
    for i in 1..=n as u16 {
        let mut cd = ColumnDescription::default();
        cur.describe_col(i, &mut cd)?;
        println!(
            "[{tag}] col {i} name={:?} data_type={:?}",
            cd.name_to_string(),
            cd.data_type
        );
        println!(
            "[{tag}] col {i} ColumnDescription.data_type.column_size() = {:?}",
            cd.data_type.column_size().map(|v| v.get())
        );
        println!(
            "[{tag}] col {i} ColumnDescription.data_type.decimal_digits() = {}",
            cd.data_type.decimal_digits()
        );
        println!(
            "[{tag}] col {i} ColumnDescription.nullability = {:?}",
            cd.nullability
        );
        println!(
            "[{tag}] col {i} SQLColAttribute col_octet_length = {:?}",
            cur.col_octet_length(i)?.map(|v| v.get())
        );
        println!(
            "[{tag}] col {i} SQLColAttribute col_display_size = {:?}",
            cur.col_display_size(i)?.map(|v| v.get())
        );
        println!(
            "[{tag}] col {i} SQLColAttribute col_precision = {:?}",
            cur.col_precision(i)?
        );
        println!(
            "[{tag}] col {i} SQLColAttribute col_scale = {:?}",
            cur.col_scale(i)?
        );
        println!(
            "[{tag}] col {i} SQLColAttribute col_nullability = {:?}",
            cur.col_nullability(i)?
        );
        println!(
            "[{tag}] col {i} col_data_type = {:?}",
            cur.col_data_type(i)?
        );
    }
    Ok(())
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (((sorted.len() - 1) as f64) * p).round() as usize;
    sorted[idx]
}

fn stats(sorted: &[u64]) -> String {
    let n = sorted.len();
    if n == 0 {
        return "<no samples>".to_string();
    }
    let sum: u64 = sorted.iter().sum();
    format!(
        "n={} mean={:.1}us min={}us p50={}us p90={}us p99={}us max={}us",
        n,
        sum as f64 / n as f64,
        sorted[0],
        percentile(sorted, 0.50),
        percentile(sorted, 0.90),
        percentile(sorted, 0.99),
        sorted[n - 1]
    )
}

// ---------------------------------------------------------------------------
// raw (odbc_api::sys) probes for Q7 / Q8
// ---------------------------------------------------------------------------

fn raw_probes(conn_str: &str) {
    println!("\n===== Q7(raw) / Q8 : low level probes via odbc_api::sys + odbc_api::handles =====");
    println!("  (odbc-api has no safe wrapper for SQLGetInfo / SQLSetStmtAttr, so these use");
    println!("   the escape hatch `odbc_api::sys`, which the crate re-exports itself.)");

    let env = match RawEnv::new() {
        SqlResult::Success(e) | SqlResult::SuccessWithInfo(e) => e,
        other => {
            println!("[RAW] SQLAllocHandle(Env) FAILED: {}", label(&other));
            return;
        }
    };
    let ver = env.declare_version(AttrOdbcVersion::Odbc3_80);
    println!(
        "[RAW] SQLSetEnvAttr(SQL_ATTR_ODBC_VERSION=SQL_OV_ODBC3_80) -> {}",
        label(&ver)
    );

    let mut conn = match env.allocate_connection() {
        SqlResult::Success(c) | SqlResult::SuccessWithInfo(c) => c,
        other => {
            println!("[RAW] SQLAllocHandle(Dbc) FAILED: {}", label(&other));
            return;
        }
    };
    match conn.connect_with_connection_string(&SqlText::new(conn_str)) {
        SqlResult::Success(()) | SqlResult::SuccessWithInfo(()) => {
            println!("[RAW] raw second connection established OK")
        }
        other => {
            println!("[RAW] raw connection FAILED: {}", label(&other));
            println!(
                "[RAW]   diag: {}",
                diag_text(HandleType::Dbc, conn.as_sys().as_handle())
            );
            return;
        }
    }

    // --- Q7: SQL_IDENTIFIER_QUOTE_CHAR, raw ---
    let mut buf = [0u16; 8];
    let mut len: i16 = 0;
    let rc = unsafe {
        sys::SQLGetInfoW(
            conn.as_sys(),
            InfoType::IdentifierQuoteChar,
            buf.as_mut_ptr() as sys::Pointer,
            (buf.len() * 2) as i16,
            &mut len,
        )
    };
    println!(
        "[Q7-raw] SQLGetInfoW(SQL_IDENTIFIER_QUOTE_CHAR) -> {} len_bytes={} raw_u16=[{}]",
        sr_label(rc),
        len,
        hex16(&buf)
    );
    if rc == sys::SqlReturn::ERROR {
        println!(
            "[Q7-raw]   diag: {}",
            diag_text(HandleType::Dbc, conn.as_sys().as_handle())
        );
    }
    let mut buf_a = [0u8; 8];
    let mut len_a: i16 = 0;
    let rc_a = unsafe {
        sys::SQLGetInfo(
            conn.as_sys(),
            InfoType::IdentifierQuoteChar,
            buf_a.as_mut_ptr() as sys::Pointer,
            buf_a.len() as i16,
            &mut len_a,
        )
    };
    println!(
        "[Q7-raw] SQLGetInfoA(SQL_IDENTIFIER_QUOTE_CHAR) -> {} len_bytes={} raw_bytes=[{}]",
        sr_label(rc_a),
        len_a,
        hex8(&buf_a)
    );

    // --- Q8: SQL_ASYNC_MODE, raw ---
    let mut mode: u32 = 0xFFFF_FFFF;
    let rc = unsafe {
        sys::SQLGetInfoW(
            conn.as_sys(),
            InfoType::AsyncMode,
            &mut mode as *mut u32 as sys::Pointer,
            4,
            std::ptr::null_mut(),
        )
    };
    println!(
        "[Q8] SQLGetInfoW(SQL_ASYNC_MODE) -> {} value={} (0=SQL_AM_NONE, 1=SQL_AM_CONNECTION, 2=SQL_AM_STATEMENT)",
        sr_label(rc),
        mode
    );
    if rc == sys::SqlReturn::ERROR {
        println!(
            "[Q8]   diag: {}",
            diag_text(HandleType::Dbc, conn.as_sys().as_handle())
        );
    }

    // --- Q8: SQL_ATTR_ASYNC_ENABLE on a fresh statement handle ---
    let mut stmt = match conn.allocate_statement() {
        SqlResult::Success(s) | SqlResult::SuccessWithInfo(s) => s,
        other => {
            println!("[Q8] SQLAllocHandle(Stmt) FAILED: {}", label(&other));
            return;
        }
    };
    let hstmt = stmt.as_sys();

    let rc_on = unsafe {
        sys::SQLSetStmtAttr(hstmt, StatementAttribute::AsyncEnable, 1 as sys::Pointer, 0)
    };
    println!(
        "[Q8] SQLSetStmtAttr(stmt, SQL_ATTR_ASYNC_ENABLE, SQL_ASYNC_ENABLE_ON=1) -> {}",
        sr_label(rc_on)
    );
    if rc_on == sys::SqlReturn::ERROR {
        println!(
            "[Q8]   diag: {}",
            diag_text(HandleType::Stmt, hstmt.as_handle())
        );
    }

    let mut val: u32 = 0xFFFF_FFFF;
    let rc_get = unsafe {
        sys::SQLGetStmtAttr(
            hstmt,
            StatementAttribute::AsyncEnable,
            &mut val as *mut u32 as sys::Pointer,
            4,
            std::ptr::null_mut(),
        )
    };
    println!(
        "[Q8] SQLGetStmtAttr(stmt, SQL_ATTR_ASYNC_ENABLE) -> {} value={}",
        sr_label(rc_get),
        val
    );
    if rc_get == sys::SqlReturn::ERROR {
        println!(
            "[Q8]   diag: {}",
            diag_text(HandleType::Stmt, hstmt.as_handle())
        );
    }

    // Does the driver actually honour it? Execute a trivial select and look for
    // SQL_STILL_EXECUTING instead of SQL_SUCCESS.
    let sql: Vec<u16> = Utf16String::from_str("select 1 from dual").into_vec();
    let rc_exec = unsafe { sys::SQLExecDirectW(hstmt, sql.as_ptr(), sql.len() as i32) };
    println!(
        "[Q8] SQLExecDirectW(\"select 1 from dual\") with SQL_ATTR_ASYNC_ENABLE=ON -> {} (SQL_STILL_EXECUTING would mean really async)",
        sr_label(rc_exec)
    );
    if rc_exec == sys::SqlReturn::ERROR || rc_exec == sys::SqlReturn::SUCCESS_WITH_INFO {
        println!(
            "[Q8]   diag: {}",
            diag_text(HandleType::Stmt, hstmt.as_handle())
        );
    }

    // turn it back off, to see whether the attribute is writable at all
    let rc_off = unsafe {
        sys::SQLSetStmtAttr(hstmt, StatementAttribute::AsyncEnable, 0 as sys::Pointer, 0)
    };
    println!(
        "[Q8] SQLSetStmtAttr(stmt, SQL_ATTR_ASYNC_ENABLE, SQL_ASYNC_ENABLE_OFF=0) -> {}",
        sr_label(rc_off)
    );
    if rc_off == sys::SqlReturn::ERROR {
        println!(
            "[Q8]   diag: {}",
            diag_text(HandleType::Stmt, hstmt.as_handle())
        );
    }
}

// ---------------------------------------------------------------------------

fn main() {
    let t_program = Instant::now();
    println!("################################################################");
    println!("# odbc-api + DM8 ODBC driver :: smoke probe");
    println!(
        "# crate build mode : {}",
        if WIDE {
            "WIDE  (odbc-api calls the Unicode SQL*W entry points, UTF-16 on the wire)"
        } else {
            "NARROW (odbc-api calls the ANSI SQL*A entry points, bytes assumed UTF-8)"
        }
    );
    println!("# target_os        : {}", std::env::consts::OS);
    println!("# odbc-api         : 29.2 (default-features = false, features = [odbc_version_3_80])");
    println!(
        "# own features     : narrow={} wide={}",
        cfg!(feature = "narrow"),
        cfg!(feature = "wide")
    );
    println!(
        "# compiler profile : {}",
        if cfg!(debug_assertions) {
            "debug (cargo run default)"
        } else {
            "release"
        }
    );
    println!("################################################################");

    // -----------------------------------------------------------------
    // Q1 : environment, drivers, DSNs, connection strings
    // -----------------------------------------------------------------
    println!("\n===== Q1 : can we connect, and with which connection string =====");
    let env = match Environment::new() {
        Ok(e) => {
            println!("[Q1] Environment::new() OK");
            e
        }
        Err(e) => {
            println!("[Q1] Environment::new() FAILED: {e}");
            return;
        }
    };

    match env.drivers() {
        Ok(ds) => {
            println!("[Q1] Environment::drivers() -> {} driver(s)", ds.len());
            for d in &ds {
                println!(
                    "[Q1]   driver description={:?} attributes={:?}",
                    d.description, d.attributes
                );
            }
        }
        Err(e) => println!("[Q1] Environment::drivers() FAILED: {e}"),
    }
    match env.data_sources() {
        Ok(ds) if ds.is_empty() => {
            println!("[Q1] Environment::data_sources() -> <no user DSN registered at all>")
        }
        Ok(ds) => {
            println!("[Q1] Environment::data_sources() -> {} DSN(s)", ds.len());
            for d in &ds {
                println!(
                    "[Q1]   dsn server_name={:?} driver={:?}",
                    d.server_name, d.driver
                );
            }
        }
        Err(e) => println!("[Q1] Environment::data_sources() FAILED: {e}"),
    }
    match env.system_data_sources() {
        Ok(ds) if ds.is_empty() => {
            println!("[Q1] Environment::system_data_sources() -> <no system DSN registered>")
        }
        Ok(ds) => {
            println!(
                "[Q1] Environment::system_data_sources() -> {} system DSN(s)",
                ds.len()
            );
            for d in &ds {
                println!(
                    "[Q1]   system dsn server_name={:?} driver={:?}",
                    d.server_name, d.driver
                );
            }
        }
        Err(e) => println!("[Q1] Environment::system_data_sources() FAILED: {e}"),
    }

    let attempts: Vec<(&str, String)> = vec![
        ("A base", CONN_BASE.to_string()),
        (
            "B base+PROTOCOL_TYPE=TCP",
            format!("{CONN_BASE};PROTOCOL_TYPE=TCP"),
        ),
        (
            "C base+CHARACTER_CODE=PG_UTF8",
            format!("{CONN_BASE};CHARACTER_CODE=PG_UTF8"),
        ),
        (
            "D base+PROTOCOL_TYPE=TCP+CHARACTER_CODE=PG_UTF8",
            format!("{CONN_BASE};PROTOCOL_TYPE=TCP;CHARACTER_CODE=PG_UTF8"),
        ),
        (
            "E base+CHARACTER_CODE=PG_GB18030",
            format!("{CONN_BASE};CHARACTER_CODE=PG_GB18030"),
        ),
        ("F DSN variant", "DSN=DM8;UID=SYSDBA;PWD=SYSDBA".to_string()),
        ("G DSN variant 2", "DSN=DM;UID=SYSDBA;PWD=SYSDBA".to_string()),
    ];

    for (name, cs) in &attempts {
        let t = Instant::now();
        match env.connect_with_connection_string(cs, ConnectionOptions::default()) {
            Ok(c) => {
                println!(
                    "[Q1] attempt {name} -> SUCCESS in {:?}\n[Q1]   conn_str: {cs}",
                    t.elapsed()
                );
                drop(c);
            }
            Err(e) => {
                println!(
                    "[Q1] attempt {name} -> FAILED in {:?}\n[Q1]   conn_str: {cs}",
                    t.elapsed()
                );
                println!("[Q1]   error display: {e}");
                println!("[Q1]   error debug  : {e:?}");
            }
        }
    }

    // connect for real, timing it (Q6 wants the connect cost)
    let t = Instant::now();
    let conn = match env.connect_with_connection_string(CONN_BASE, ConnectionOptions::default()) {
        Ok(c) => c,
        Err(e) => {
            println!("[FATAL] could not open the working connection: {e}");
            return;
        }
    };
    let connect_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "[Q6] time to establish the working connection = {:.3} ms (connection string \"A base\")",
        connect_ms
    );
    guard("Q1", || match conn.database_management_system_name() {
        Ok(n) => println!("[Q1] SQL_DBMS_NAME = {n:?}"),
        Err(e) => println!("[Q1] SQL_DBMS_NAME FAILED: {e}"),
    });
    for probe in [
        "select banner as b from v$version",
        "select id as b from v$instance",
        "select unicode() as c from dual",
    ] {
        match scalar_narrow(&conn, probe) {
            Ok(v) => println!("[Q1] server probe `{probe}` -> {v:?}"),
            Err(e) => println!("[Q1] server probe `{probe}` FAILED: {e}"),
        }
    }

    // -----------------------------------------------------------------
    // Q2 : server charset id
    // -----------------------------------------------------------------
    println!("\n===== Q2 : select unicode() from dual (server charset id) =====");
    for sql in [
        "select unicode() as c from dual",
        "select cast(unicode() as varchar(20)) as c from dual",
    ] {
        match scalar_narrow(&conn, sql) {
            Ok(v) => println!("[Q2] `{sql}` -> narrow get_text value = {v:?}"),
            Err(e) => println!("[Q2] `{sql}` -> ERROR {e}"),
        }
    }
    // NB: in odbc-api 29 `CursorRow::get_text` / `get_wide_text` use SQLGetData, which
    // *drains* the column. Fetching the same column twice trips over SQL_NO_DATA, so the
    // literal is selected twice under two aliases and read once per alias.
    guard("Q2", || {
        let sql = "select unicode() as c_narrow, unicode() as c_wide from dual";
        match conn.execute(sql, (), None) {
            Ok(Some(mut cur)) => match cur.next_row() {
                Ok(Some(mut row)) => {
                    let mut n = Vec::new();
                    let mut w = Vec::new();
                    let hn = row.get_text(1, &mut n);
                    let hw = row.get_wide_text(2, &mut w);
                    println!("[Q2] get_text(col 1)      -> {hn:?}");
                    show_narrow("[Q2] get_text", &n);
                    println!("[Q2] get_wide_text(col 2) -> {hw:?}");
                    show_wide("[Q2] get_wide_text", &w);
                }
                Ok(None) => println!("[Q2] no row"),
                Err(e) => println!("[Q2] next_row ERROR: {e}"),
            },
            Ok(None) => println!("[Q2] no cursor"),
            Err(e) => println!("[Q2] execute ERROR: {e}"),
        }
    });

    // -----------------------------------------------------------------
    // Q3 : Chinese literal through the narrow and the wide fetch path
    // -----------------------------------------------------------------
    println!("\n===== Q3 : select '中文测试' as c from dual, narrow vs wide fetch =====");
    println!("  NOTE: the literal travels as SQL text, so the build mode decides how the");
    println!("        driver sees it. Both fetch paths are exercised on the SAME row.");
    let sql = format!("select '{ZH}' as c_narrow, '{ZH}' as c_wide from dual");
    guard("Q3", || match conn.execute(&sql, (), None) {
        Ok(Some(mut cur)) => match cur.next_row() {
            Ok(Some(mut row)) => {
                let mut n = Vec::new();
                let mut w = Vec::new();
                let hn = row.get_text(1, &mut n);
                let hw = row.get_wide_text(2, &mut w);
                println!("[Q3] get_text(col 1, SQL_C_CHAR)       -> {hn:?}");
                show_narrow("[Q3] narrow path", &n);
                println!("[Q3] get_wide_text(col 2, SQL_C_WCHAR) -> {hw:?}");
                show_wide("[Q3] wide path", &w);
                println!(
                    "[Q3] narrow-decoded == wide-decoded ? {}",
                    String::from_utf8_lossy(&n) == String::from_utf16_lossy(&w)
                );
            }
            Ok(None) => println!("[Q3] no row"),
            Err(e) => println!("[Q3] next_row ERROR: {e}"),
        },
        Ok(None) => println!("[Q3] no cursor"),
        Err(e) => println!("[Q3] execute ERROR: {e}"),
    });
    println!("[Q3] expected encodings of \"{ZH}\":");
    println!("[Q3]   UTF-8          = {}", hex8(&ZH_UTF8));
    println!("[Q3]   GB18030/CP936  = {}", hex8(&ZH_GBK));
    println!("[Q3]   UTF-16         = {}", hex16(&ZH_UTF16));

    // -----------------------------------------------------------------
    // Q3b : can CHARACTER_CODE make the client side encoding sane?
    // -----------------------------------------------------------------
    println!("\n===== Q3b : does CHARACTER_CODE=... change what the client receives? =====");
    for (name, cs) in [
        ("default (no CHARACTER_CODE)", CONN_BASE.to_string()),
        (
            "CHARACTER_CODE=PG_UTF8",
            format!("{CONN_BASE};CHARACTER_CODE=PG_UTF8"),
        ),
        (
            "CHARACTER_CODE=PG_GB18030",
            format!("{CONN_BASE};CHARACTER_CODE=PG_GB18030"),
        ),
    ] {
        guard("Q3b", || match env.connect_with_connection_string(&cs, ConnectionOptions::default()) {
            Ok(c2) => {
                let sql = format!("select '{ZH}' as c_narrow, '{ZH}' as c_wide from dual");
                let mut done = false;
                if let Ok(Some(mut cur)) = c2.execute(&sql, (), None) {
                    if let Ok(Some(mut row)) = cur.next_row() {
                        let mut n = Vec::new();
                        let mut w = Vec::new();
                        let _ = row.get_text(1, &mut n);
                        let _ = row.get_wide_text(2, &mut w);
                        println!("[Q3b] {name}: narrow bytes = [{}]", hex8(&n));
                        println!(
                            "[Q3b] {name}: narrow decodes as UTF-8? {}",
                            std::str::from_utf8(&n).map(|s| s.to_string()).unwrap_or_else(|e| format!("NO ({e})"))
                        );
                        println!(
                            "[Q3b] {name}: narrow == GB18030 bytes? {}",
                            n == ZH_GBK
                        );
                        println!("[Q3b] {name}: wide   = [{}] -> {:?}", hex16(&w), String::from_utf16(&w).ok());
                        done = true;
                    }
                }
                if !done {
                    println!("[Q3b] {name}: query produced no row");
                }
            }
            Err(e) => println!("[Q3b] {name}: connect FAILED: {e}"),
        });
    }

    // -----------------------------------------------------------------
    // Q4 : DDL, parameter bound insert, lengthb / length / rawtohex
    // -----------------------------------------------------------------
    println!(
        "\n===== Q4 : create table {TABLE}, parameter-bound insert, lengthb/length/rawtohex ====="
    );
    exec(&conn, "Q4", &format!("drop table {TABLE}"));
    exec(
        &conn,
        "Q4",
        &format!("create table {TABLE}(id int primary key, v varchar(100))"),
    );

    let insert_sql = format!("insert into {TABLE}(id, v) values(?, ?)");
    let mut prepared = match conn.prepare(&insert_sql) {
        Ok(p) => p,
        Err(e) => {
            println!("[Q4] prepare FAILED: {e}");
            return;
        }
    };

    let mut values: Vec<(i32, String)> = vec![(1, ZH.to_string())];
    for (idx, n) in [49usize, 50, 51, 60, 100].iter().enumerate() {
        values.push(((idx + 2) as i32, "中".repeat(*n)));
    }

    for (id, v) in &values {
        let ip = (*id).into_parameter();
        let vp = v.as_str().into_parameter();
        let res = prepared.execute((&ip, &vp));
        let n_chars = v.chars().count();
        let n_utf8 = v.len();
        match res {
            Ok(_) => println!("[Q4] insert id={id} chars={n_chars} utf8_bytes={n_utf8} -> OK"),
            Err(e) => println!(
                "[Q4] insert id={id} chars={n_chars} utf8_bytes={n_utf8} -> ERROR {e}\n[Q4]   debug: {e:?}"
            ),
        }
    }
    drop(prepared);
    let _ = conn.commit();

    println!("[Q4] read back (narrow text path for all columns):");
    println!("[Q4] id | lengthb(v) | length(v) | rawtohex(v) | v");
    match fetch_rows(
        &conn,
        &format!("select id, lengthb(v), length(v), rawtohex(v), v from {TABLE} order by id"),
    ) {
        Ok(rows) => {
            for r in &rows {
                let f = |i: usize| {
                    r.get(i)
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| "<NULL>".into())
                };
                println!("[Q4] {} | {} | {} | {} | {}", f(0), f(1), f(2), f(3), f(4));
            }
            if rows.is_empty() {
                println!("[Q4] <no rows came back>");
            }
        }
        Err(e) => println!("[Q4] read back FAILED: {e}"),
    }

    // How does the server itself see a varchar(100) literal?  All three
    // columns are ASCII (digits / hex), so they survive either codepage.
    for n in [10usize, 33, 34, 49, 50, 51, 60, 100] {
        let lit = "中".repeat(n);
        let sql = format!(
            "select lengthb(v) as lb, length(v) as l, rawtohex(v) as h \
             from (select cast('{lit}' as varchar(100)) as v from dual)"
        );
        match fetch_rows(&conn, &sql) {
            Ok(rows) => {
                for r in &rows {
                    println!(
                        "[Q4] server view of cast({n} x 中 as varchar(100)): lengthb={:?} length={:?} rawtohex={:?}",
                        r[0], r[1], r[2]
                    );
                }
                if rows.is_empty() {
                    println!("[Q4] server view of cast({n} x 中) -> no row");
                }
            }
            Err(e) => println!("[Q4] server view of cast({n} x 中 as varchar(100)) -> ERROR {e}"),
        }
        if n == 49 {
            // juxtapose the server truth with the raw bytes the client narrow path gets
            let probe = format!("select cast('{lit}' as varchar(100)) as c from dual");
            match conn.execute(&probe, (), None) {
                Ok(Some(mut cur)) => match cur.next_row() {
                    Ok(Some(mut row)) => {
                        let mut buf = Vec::new();
                        match row.get_text(1, &mut buf) {
                            Ok(_) => println!(
                                "[Q4] client narrow path reports {} raw byte(s) for cast(49 x 中); first 6 = [{}]",
                                buf.len(),
                                hex8(&buf[..buf.len().min(6)])
                            ),
                            Err(e) => println!("[Q4] client narrow read of cast(49 x 中) -> get_text error {e}"),
                        }
                    }
                    _ => println!("[Q4] client narrow read of cast(49 x 中) -> next_row returned no row"),
                },
                _ => println!("[Q4] client narrow read of cast(49 x 中) -> execute returned no cursor"),
            }
        }
    }

    println!("[Q4] server side length semantics parameters:");
    match fetch_rows(
        &conn,
        "select para_name, para_value from v$dm_ini where para_name in ('LENGTH_IN_CHAR','COMPATIBLE_MODE','GLOBAL_CHARSET','CHARSET')",
    ) {
        Ok(rows) if rows.is_empty() => println!("[Q4]   <v$dm_ini query returned no rows>"),
        Ok(rows) => {
            for r in &rows {
                println!("[Q4]   v$dm_ini {:?}", r);
            }
        }
        Err(e) => println!("[Q4]   v$dm_ini query FAILED: {e}"),
    }
    for sql in [
        "select para_value as c from v$dm_ini where para_name = 'LENGTH_IN_CHAR'",
    ] {
        match scalar_narrow(&conn, sql) {
            Ok(v) => println!("[Q4] `{sql}` -> {v:?}"),
            Err(e) => println!("[Q4] `{sql}` FAILED: {e}"),
        }
    }

    // -----------------------------------------------------------------
    // Q5 : SQLDescribeCol metadata semantics
    // -----------------------------------------------------------------
    println!("\n===== Q5 : SQLDescribeCol / ResultSetMetadata semantics =====");
    println!("-- (a) declared column varchar(100) holding the 4-char Chinese value (id=1)");
    guard("Q5a", || {
        match conn.execute(&format!("select v from {TABLE} where id = 1"), (), None) {
            Ok(Some(mut cur)) => {
                if let Err(e) = dump_cols("Q5a", &mut cur) {
                    println!("[Q5a] dump FAILED: {e}");
                }
            }
            Ok(None) => println!("[Q5a] no cursor"),
            Err(e) => println!("[Q5a] execute FAILED: {e}"),
        }
    });

    println!("-- (b) same column re-described on the widest row actually inserted (id=3: 50 chars = 100 bytes)");
    guard("Q5b", || {
        match conn.execute(&format!("select v from {TABLE} where id = 3"), (), None) {
            Ok(Some(mut cur)) => {
                if let Err(e) = dump_cols("Q5b", &mut cur) {
                    println!("[Q5b] dump FAILED: {e}");
                }
            }
            Ok(None) => println!("[Q5b] no cursor"),
            Err(e) => println!("[Q5b] execute FAILED: {e}"),
        }
    });

    println!("-- (c) the integer column, for contrast");
    guard("Q5c", || {
        match conn.execute(&format!("select id from {TABLE} where id = 1"), (), None) {
            Ok(Some(mut cur)) => {
                if let Err(e) = dump_cols("Q5c", &mut cur) {
                    println!("[Q5c] dump FAILED: {e}");
                }
            }
            Ok(None) => println!("[Q5c] no cursor"),
            Err(e) => println!("[Q5c] execute FAILED: {e}"),
        }
    });

    println!("-- (d) select cast(? as varchar(10)) from dual with a Chinese parameter");
    guard("Q5d", || {
        match conn.prepare("select cast(? as varchar(10)) as c_narrow, cast(? as varchar(10)) as c_wide from dual") {
        Ok(mut p) => {
            println!("[Q5d] describe_param(1) = {:?}", p.describe_param(1));
            let vp = ZH.into_parameter();
            match p.execute((&vp, &vp)) {
                Ok(Some(mut cur)) => {
                    if let Err(e) = dump_cols("Q5d", &mut cur) {
                        println!("[Q5d] dump FAILED: {e}");
                    }
                    match cur.next_row() {
                        Ok(Some(mut row)) => {
                            let mut n = Vec::new();
                            let mut w = Vec::new();
                            let _ = row.get_text(1, &mut n);
                            let _ = row.get_wide_text(2, &mut w);
                            show_narrow("[Q5d] value narrow", &n);
                            show_wide("[Q5d] value wide", &w);
                        }
                        Ok(None) => println!("[Q5d] no row"),
                        Err(e) => println!("[Q5d] next_row FAILED: {e}"),
                    }
                }
                Ok(None) => println!("[Q5d] no cursor"),
                Err(e) => println!("[Q5d] execute FAILED: {e}"),
            }
        }
        Err(e) => println!("[Q5d] prepare FAILED: {e}"),
        }
    });

    println!("-- (e) casts of different shapes");
    guard("Q5e", || {
        for sql in [
            "select cast('ab' as varchar(100)) as c from dual",
            "select cast('中文测试' as varchar(100)) as c from dual",
            "select cast('中文测试' as varchar(10)) as c from dual",
            "select cast('中文测试' as char(10)) as c from dual",
            "select cast('中文测试' as varchar2(4000)) as c from dual",
        ] {
            match conn.execute(sql, (), None) {
                Ok(Some(mut cur)) => {
                    println!("[Q5e] --- {sql}");
                    if let Err(e) = dump_cols("Q5e", &mut cur) {
                        println!("[Q5e] dump FAILED: {e}");
                    }
                }
                Ok(None) => println!("[Q5e] {sql} -> no cursor"),
                Err(e) => println!("[Q5e] {sql} -> ERROR {e}"),
            }
        }
    });

    // -----------------------------------------------------------------
    // Q6 : latency
    // -----------------------------------------------------------------
    println!("\n===== Q6 : latency =====");
    println!("[Q6] connect (one-shot, measured above) = {connect_ms:.3} ms");

    for _ in 0..20 {
        let _ = scalar_narrow(&conn, "select 1 from dual");
    }

    let const_sql = "select 1 from dual";
    let mut samples: Vec<u64> = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let t = Instant::now();
        if let Ok(Some(mut cur)) = conn.execute(const_sql, (), None) {
            let _ = cur.next_row();
        }
        samples.push(t.elapsed().as_micros() as u64);
    }
    samples.sort_unstable();
    println!("[Q6] Connection::execute(\"select 1 from dual\") x1000 (fresh statement each time)");
    println!("[Q6]   {}", stats(&samples));

    if let Ok(mut pre) = conn.preallocate() {
        let mut s2: Vec<u64> = Vec::with_capacity(1000);
        for _ in 0..1000 {
            let t = Instant::now();
            if let Ok(Some(mut cur)) = pre.execute(const_sql, ()) {
                let _ = cur.next_row();
            }
            s2.push(t.elapsed().as_micros() as u64);
        }
        s2.sort_unstable();
        println!("[Q6] Preallocated::execute x1000 (handle reused, SQL re-sent + re-prepared)");
        println!("[Q6]   {}", stats(&s2));
    } else {
        println!("[Q6] preallocate() FAILED");
    }

    match conn.prepare(const_sql) {
        Ok(mut p) => {
            let mut s3: Vec<u64> = Vec::with_capacity(1000);
            for _ in 0..1000 {
                let t = Instant::now();
                if let Ok(Some(mut cur)) = p.execute(()) {
                    let _ = cur.next_row();
                }
                s3.push(t.elapsed().as_micros() as u64);
            }
            s3.sort_unstable();
            println!("[Q6] Prepared::execute x1000 (SQL PREPAREd once, no params)");
            println!("[Q6]   {}", stats(&s3));
        }
        Err(e) => println!("[Q6] prepare FAILED: {e}"),
    }

    match conn.prepare("select ? from dual") {
        Ok(mut p) => {
            let mut s4: Vec<u64> = Vec::with_capacity(1000);
            for i in 0..1000i32 {
                let ip = i.into_parameter();
                let t = Instant::now();
                if let Ok(Some(mut cur)) = p.execute(&ip) {
                    let _ = cur.next_row();
                }
                s4.push(t.elapsed().as_micros() as u64);
            }
            s4.sort_unstable();
            println!("[Q6] Prepared::execute(&param) x1000 (PREPAREd once, 1 param rebound each round)");
            println!("[Q6]   {}", stats(&s4));
        }
        Err(e) => println!("[Q6] prepare \"select ? from dual\" FAILED: {e}"),
    }

    // -----------------------------------------------------------------
    // Q7 : identifier quote character
    // -----------------------------------------------------------------
    println!("\n===== Q7 : identifier quote character =====");
    match conn.identifier_quote_char() {
        Ok(Some(c)) => println!(
            "[Q7] Connection::identifier_quote_char() = Some({c:?}) (U+{:04X})",
            c as u32
        ),
        Ok(None) => println!(
            "[Q7] Connection::identifier_quote_char() = None (driver reports no delimited identifier support)"
        ),
        Err(e) => println!("[Q7] identifier_quote_char FAILED: {e}"),
    }
    exec(&conn, "Q7", &format!("select `id` from {TABLE}"));
    exec(&conn, "Q7", &format!("select \"id\" from {TABLE}"));
    exec(&conn, "Q7", &format!("select \"ID\" from {TABLE}"));
    exec(&conn, "Q7", &format!("select id from {TABLE}"));

    // -----------------------------------------------------------------
    // Q8 : async (raw) + odbc-api polling path
    // -----------------------------------------------------------------
    guard("Q8", || raw_probes(CONN_BASE));
    println!(
        "-- odbc-api level: Preallocated::into_polling() (internally SQLSetStmtAttr(SQL_ATTR_ASYNC_ENABLE, ON))"
    );
    guard("Q8", || match conn.preallocate() {
        Ok(pre) => match pre.into_polling() {
            Ok(_) => println!(
                "[Q8] Preallocated::into_polling() -> Ok (driver accepted SQL_ATTR_ASYNC_ENABLE=ON)"
            ),
            Err(e) => println!("[Q8] Preallocated::into_polling() -> Err: {e}\n[Q8]   debug: {e:?}"),
        },
        Err(e) => println!("[Q8] preallocate FAILED: {e}"),
    });

    // -----------------------------------------------------------------
    // Q9 : MySQL dialect through ODBC
    // -----------------------------------------------------------------
    println!("\n===== Q9 : MySQL dialect statements =====");
    exec(&conn, "Q9.1", &format!("select `id` from {TABLE}"));
    exec(&conn, "Q9.2", &format!("select id from {TABLE} limit 0, 1"));
    exec(
        &conn,
        "Q9.3",
        &format!("create table {TABLE2}(id int auto_increment primary key, v varchar(20))"),
    );
    // If the DDL above succeeded, check whether the column really behaves like a
    // MySQL AUTO_INCREMENT column (id omitted on insert => server assigns it).
    guard("Q9.3b", || {
        exec(
            &conn,
            "Q9.3b",
            &format!("insert into {TABLE2}(v) values('a')"),
        );
        exec(
            &conn,
            "Q9.3b",
            &format!("insert into {TABLE2}(v) values('b')"),
        );
        match fetch_rows(&conn, &format!("select id, v from {TABLE2} order by id")) {
            Ok(rows) => {
                for r in &rows {
                    println!("[Q9.3b] t_odbcprobe2 row = {:?}", r);
                }
                if rows.is_empty() {
                    println!("[Q9.3b] <no rows>");
                }
            }
            Err(e) => println!("[Q9.3b] select FAILED: {e}"),
        }
    });
    exec(
        &conn,
        "Q9.4",
        &format!("insert into {TABLE}(id,v) values(1,'x') on duplicate key update v='y'"),
    );
    exec(&conn, "Q9.5", &format!("drop table if exists {TABLE2}"));

    // -----------------------------------------------------------------
    // Q10 : ColumnarBulkInserter vs single-row prepared loop
    // -----------------------------------------------------------------
    println!("\n===== Q10 : bulk array insert vs single-row prepared loop =====");
    const N: usize = 1000;

    exec(
        &conn,
        "Q10",
        &format!("delete from {TABLE} where id between 1000 and 200000"),
    );
    let _ = conn.commit();

    let mut bulk_ms = f64::NAN;
    match conn.prepare(&insert_sql) {
        Ok(prepared) => {
            let descs = [
                BindParamDesc::i32(false),
                if WIDE {
                    BindParamDesc::wide_text(256)
                } else {
                    BindParamDesc::text(256)
                },
            ];
            match prepared.into_column_inserter(N, descs) {
                Ok(mut ins) => {
                    let cap = ins.capacity();
                    println!("[Q10] into_column_inserter OK, capacity={cap}");
                    let t = Instant::now();
                    ins.set_num_rows(N);
                    {
                        let mut col = match ins.column_mut(0).as_slice::<i32>() {
                            Some(c) => c,
                            None => {
                                println!("[Q10] column_mut(0).as_slice::<i32>() returned None");
                                return;
                            }
                        };
                        for (i, slot) in col.iter_mut().enumerate().take(N) {
                            *slot = 1000 + i as i32;
                        }
                    }
                    if WIDE {
                        let mut col = ins.column_mut(1).as_wide_text().expect("wide text column");
                        for i in 0..N {
                            let s = format!("bulk-{i}-中文");
                            let w: Vec<u16> = Utf16String::from_str(&s).into_vec();
                            col.set_cell(i, Some(&w));
                        }
                    } else {
                        let mut col = ins.column_mut(1).as_text().expect("text column");
                        for i in 0..N {
                            let s = format!("bulk-{i}-中文");
                            let b = s.into_bytes();
                            col.set_cell(i, Some(&b));
                        }
                    }
                    match ins.execute() {
                        Ok(_) => {
                            bulk_ms = t.elapsed().as_secs_f64() * 1000.0;
                            println!("[Q10] ColumnarBulkInserter N={N} -> OK in {bulk_ms:.3} ms");
                        }
                        Err(e) => {
                            println!("[Q10] ColumnarBulkInserter execute FAILED: {e}");
                            println!("[Q10]   debug: {e:?}");
                        }
                    }
                }
                Err(e) => println!("[Q10] into_column_inserter FAILED: {e}"),
            }
        }
        Err(e) => println!("[Q10] prepare FAILED: {e}"),
    }
    let _ = conn.commit();

    match scalar_narrow(
        &conn,
        &format!(
            "select count(*) from {TABLE} where id between 1000 and {}",
            999 + N
        ),
    ) {
        Ok(v) => println!("[Q10] rows present after bulk insert = {v:?}"),
        Err(e) => println!("[Q10] count after bulk FAILED: {e}"),
    }
    match scalar_narrow(
        &conn,
        &format!(
            "select count(distinct v) from {TABLE} where id between 1000 and {}",
            999 + N
        ),
    ) {
        Ok(v) => println!("[Q10] DISTINCT v values written by the bulk insert = {v:?}"),
        Err(e) => println!("[Q10] distinct count FAILED: {e}"),
    }
    match fetch_rows(
        &conn,
        &format!(
            "select id, lengthb(v), rawtohex(v) from {TABLE} where id between 1000 and 1004 order by id"
        ),
    ) {
        Ok(rows) => {
            for r in &rows {
                println!("[Q10] sample row after bulk insert: {:?}", r);
            }
        }
        Err(e) => println!("[Q10] sample rows FAILED: {e}"),
    }

    exec(
        &conn,
        "Q10",
        &format!("delete from {TABLE} where id between 1000 and {}", 999 + N),
    );
    let _ = conn.commit();

    let mut loop_ms = f64::NAN;
    match conn.prepare(&insert_sql) {
        Ok(mut p) => {
            let t = Instant::now();
            let mut ok = 0usize;
            let mut first_err: Option<String> = None;
            for i in 0..N {
                let id = 1000 + i as i32;
                let s = format!("bulk-{i}-中文");
                let ip = id.into_parameter();
                let vp = s.as_str().into_parameter();
                match p.execute((&ip, &vp)) {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        if first_err.is_none() {
                            first_err = Some(format!("{e}"));
                        }
                    }
                }
            }
            loop_ms = t.elapsed().as_secs_f64() * 1000.0;
            println!("[Q10] single-row Prepared::execute loop N={N} -> {ok} ok, {loop_ms:.3} ms");
            if let Some(e) = first_err {
                println!("[Q10]   first error: {e}");
            }
        }
        Err(e) => println!("[Q10] prepare for single-row loop FAILED: {e}"),
    }
    let _ = conn.commit();

    match scalar_narrow(
        &conn,
        &format!(
            "select count(*) from {TABLE} where id between 1000 and {}",
            999 + N
        ),
    ) {
        Ok(v) => println!("[Q10] rows present after single-row loop = {v:?}"),
        Err(e) => println!("[Q10] count after loop FAILED: {e}"),
    }

    if bulk_ms.is_finite() && loop_ms.is_finite() {
        println!(
            "[Q10] bulk={bulk_ms:.3} ms   single-row-loop={loop_ms:.3} ms   ratio(loop/bulk)={:.2}x",
            loop_ms / bulk_ms
        );
        println!(
            "[Q10] per-row: bulk={:.1} us   loop={:.1} us",
            bulk_ms * 1000.0 / N as f64,
            loop_ms * 1000.0 / N as f64
        );
    } else {
        println!("[Q10] ratio not computable (one of the two runs failed)");
    }

    // -----------------------------------------------------------------
    // cleanup
    // -----------------------------------------------------------------
    println!("\n===== cleanup =====");
    for tbl in [TABLE, TABLE2, "T_ODBCPROBE", "T_ODBCPROBE2"] {
        exec(&conn, "cleanup", &format!("drop table {tbl}"));
    }
    let _ = conn.commit();

    println!("[cleanup] objects named T_ODBCPROBE* still visible in the data dictionary:");
    match fetch_rows(
        &conn,
        "select owner, table_name from all_tables where table_name like 'T\\_ODBCPROBE%' escape '\\'",
    ) {
        Ok(rows) if rows.is_empty() => println!("[cleanup]   <none>"),
        Ok(rows) => {
            for r in &rows {
                println!("[cleanup]   {:?}", r);
            }
        }
        Err(e) => println!("[cleanup]   verification query FAILED: {e}"),
    }

    println!(
        "\n[SUMMARY] build mode = {} | total wall clock = {:.3} s",
        if WIDE { "WIDE" } else { "NARROW" },
        t_program.elapsed().as_secs_f64()
    );
}
