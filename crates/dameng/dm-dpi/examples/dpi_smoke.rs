//! DPI/FFI 探针：只打印、不断言（与 `crates/dm-odbc/examples/*_probe.rs` 同一约定）。
//!
//! 目的：在写上层 `Backend` 之前，把 DPI 的**行为细节**一次量清楚：
//!
//! 1. 客户端字符集到底怎么设（`dpi_set_env_attr` 在这台机器上对 `LOCAL_CODE`
//!    和 `LANG_ID` 都返回 `-70023 无效的参数值`，所以这里把「值型别 × 长度」
//!    和「建连前 vs 建连后」的组合都试一遍）；
//! 2. 默认模式怎么切（ODBC 的 `SCHEMA=` 被忽略；DPI 有
//!    `DSQL_ATTR_CURRENT_SCHEMA`，但这里也验证 `SET SCHEMA` SQL 的退路）；
//! 3. 中文在 UTF-8 字节路径上是否一次往返就能对得上（拿十六进制比对）；
//! 4. 预处理 + 参数绑定（含 NULL 与时间戳）是否可用；
//! 5. 失败时诊断消息长什么样（客户端字符集没设对时是 GBK 字节，这里打十六进制）。
//!
//! ```text
//! set PATH=D:\dmdbms\bin;%PATH%
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-dpi --example dpi_smoke
//! ```

use std::error::Error;
use std::ffi::c_void;

use dm_dpi::ffi::{self, SLength, attr, ctype, indicator, param, sqltype};
use dm_dpi::{Conn, Dpi, DpiError, Stmt, Timestamp};

const SCHEMA: &str = "DBTESTA";
const TABLE: &str = "DBTESTA.DPI_SMOKE_1";

fn main() -> Result<(), Box<dyn Error>> {
    let server = env_or("DM_DPI_SERVER", "127.0.0.1:5236");
    let user = env_or("DM_DPI_USER", "SYSDBA");
    let password = env_or("DM_DPI_PASSWORD", "SYSDBA");

    // ① 加载共享库 + 初始化运行时（这一步失败说明部署有问题，直接退出）。
    let dpi = Dpi::open()?;
    println!("[OK] dmdpi 已加载，dpi_module_init 成功");

    let env = dpi.env()?;
    println!("[OK] 环境句柄已分配");

    // ② 字符集：环境级属性的各种写法。
    println!("\n=== A. 环境属性（建连前）===");
    probe_env(&env);

    // ③ 登录。
    println!("\n=== B. 登录 ===");
    let conn = env.connect()?;
    conn.login(&server, &user, &password)?;
    println!("[OK] 登录成功：{server} as {user}");

    // ④ 只读属性。
    println!("\n=== C. 连接属性 ===");
    show_val("SERVER_VERSION", conn.get_attr_str(attr::SERVER_VERSION));
    show_val("LOGIN_USER", conn.get_attr_str(attr::LOGIN_USER));

    // ⑤ 服务端字符集标记 + 当前模式。
    println!("\n=== D. 字符集标记与默认模式 ===");
    show_val(
        "SF_GET_UNICODE_FLAG()",
        scalar_text(&conn, "select SF_GET_UNICODE_FLAG() from dual"),
    );
    show_val("当前模式", current_schema(&conn));

    // ⑥ 切模式：先试 SQL 退路，再试 DPI 属性。
    println!("\n=== E. 切模式 ===");
    show(
        &format!("SQL: SET SCHEMA \"{SCHEMA}\""),
        conn.exec_direct(&format!("set schema \"{SCHEMA}\""))
            .map(|_| ()),
    );
    show_val("SET SCHEMA 之后的当前模式", current_schema(&conn));
    show(
        "属性: CURRENT_SCHEMA = DBTESTB（UTF-8 窄字符串）",
        conn.set_current_schema("DBTESTB"),
    );
    show_val("属性设置之后的当前模式", current_schema(&conn));

    // ⑦ 连接级字符集属性（有些驱动只在登录后接受）。
    println!("\n=== F. 连接属性 LOCAL_CODE（登录后）===");
    show(
        "con LOCAL_CODE = 1，val_len=4",
        conn.set_attr_i32(attr::LOCAL_CODE, 1),
    );

    // ⑧ 中文往返：字面量写、按字节读回来比对。
    println!("\n=== G. 中文往返（字面量）===");
    let stmt = conn.stmt()?;
    show(
        "drop table",
        stmt.exec_direct(&format!("drop table if exists {TABLE}"))
            .map(|_| ()),
    );
    show(
        "create table",
        stmt.exec_direct(&format!(
            "create table {TABLE} (id bigint not null, name varchar(64), amt double, ts timestamp(6))"
        ))
        .map(|_| ()),
    );
    stmt.close_cursor();
    show(
        "insert 字面量（中文）",
        stmt.exec_direct(&format!(
            "insert into {TABLE} (id, name, amt, ts) values (1, '中文OK', 2.5, timestamp '2024-05-27 10:11:12.123456')"
        ))
        .map(|_| ()),
    );
    stmt.close_cursor();
    check_round_trip(&conn);

    // ⑨ 预处理 + 参数绑定。
    println!("\n=== H. 参数绑定（含 NULL 与时间戳）===");
    let insert_sql = format!("insert into {TABLE} (id, name, amt, ts) values (?, ?, ?, ?)");
    let stmt = conn.stmt()?;
    show("prepare", stmt.prepare(&insert_sql).map(|_| ()));
    show_val("参数个数", stmt.param_count());

    let mut second = Insert::new(2, Some("参数中文OK"));
    show_val("bind + exec（有值）影响行数", second.run(&stmt));
    show_val("影响行数", stmt.affected_rows());

    stmt.close_cursor();
    show("unbind", stmt.unbind_params().map(|_| ()));
    let mut third = Insert::new(3, None);
    show_val("bind + exec（name = NULL）影响行数", third.run(&stmt));
    show_val("影响行数", stmt.affected_rows());
    stmt.close_cursor();

    // ⑩ 读回：列描述 + 逐行取值 + 十六进制。
    println!("\n=== I. 结果集读取 ===");
    let stmt = conn.stmt()?;
    show(
        "select",
        stmt.exec_direct(&format!(
            "select id, name, amt, ts, '中文值OK' as tag from {TABLE} order by id"
        ))
        .map(|_| ()),
    );
    match stmt.columns() {
        Ok(columns) => {
            for col in columns {
                println!(
                    "[..] 列 {} {} sql_type={} size={} decimals={} nullable={}",
                    col.index, col.name, col.sql_type, col.size, col.decimals, col.nullable
                );
            }
        }
        Err(e) => println!("[!!] 读列描述失败：{e}"),
    }
    let mut rows = 0;
    loop {
        match stmt.fetch() {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                println!("[!!] fetch 失败：{e}");
                break;
            }
        }
        rows += 1;
        println!(
            "[OK] 行{rows}: id={:?} name={:?} name本地={:?} amt={:?} tag={:?} ts={:?}",
            stmt.column_i64(1).ok(),
            stmt.column_text(2).ok().flatten(),
            stmt.column_text_local(2).ok().flatten(),
            stmt.column_f64(3).ok(),
            stmt.column_text(5).ok().flatten(),
            stmt.column_timestamp(4).ok().flatten().map(fmt_ts)
        );
    }
    println!("[OK] 共取到 {rows} 行");
    stmt.close_cursor();

    // ⑪ 非 BMP（代理对）与宽字符口径：DM 的 ODBC 层在 `🦀` 上有已知缺陷
    //    （`type_primitives::ty_str`），这里看 DPI 是不是同样的毛病。
    println!("\n=== I′. 非 BMP 字符（🦀 U+1F980）===");
    // 必须重新 prepare：上一条语句刚跑过 select，直接 bind + exec 会被驱动
    // 判为「非法的函数调用序列」（实测 [-70026]）。
    let crab_stmt = conn.stmt()?;
    show("prepare", crab_stmt.prepare(&insert_sql).map(|_| ()));
    let mut crab = Insert::new(4, Some("🦀"));
    show_val("插入 🦀", crab.run(&crab_stmt));
    crab_stmt.close_cursor();
    crab_stmt.unbind_params().ok();
    let mut named = Insert::new(5, Some("中文OK"));
    show_val("插入 中文OK（对照）", named.run(&crab_stmt));
    crab_stmt.close_cursor();
    match scalar_text(&conn, &format!("select name from {TABLE} where id = 4")) {
        Ok(text) if text == "🦀" => println!("[OK] 宽字符读回 == 🦀"),
        Ok(text) => println!("[!!] 读回 {:?}（期望 🦀）", text),
        Err(e) => println!("[!!] 读 🦀 失败：{e}"),
    }
    match scalar_pair(
        &conn,
        &format!("select lengthb(name), length(name) from {TABLE} where id = 4"),
    ) {
        Ok((bytes, chars)) => {
            println!("[..] 🦀 服务端 lengthb={bytes} length={chars}（UTF-8 下 4 字节 1 字符）");
        }
        Err(e) => println!("[!!] 读 🦀 的 lengthb/length 失败：{e}"),
    }
    let _ = stmt.exec_direct(&format!("delete from {TABLE} where id in (4, 5)"));
    stmt.close_cursor();

    // ⑫ 失败路径。
    println!("\n=== J. 失败路径的诊断 ===");
    show(
        "查不存在的表",
        stmt.exec_direct(&format!("select * from {SCHEMA}.NO_SUCH_TABLE_X"))
            .map(|_| ()),
    );
    show(
        "设不存在的模式属性",
        conn.set_current_schema("NO_SUCH_SCHEMA_X"),
    );

    // ⑬ 收尾。
    stmt.close_cursor();
    show(
        "drop table",
        stmt.exec_direct(&format!("drop table if exists {TABLE}"))
            .map(|_| ()),
    );
    println!("\n[OK] 探针结束");
    Ok(())
}

/// 探测 `dpi_set_env_attr` 对字符集相关属性的接受方式。
fn probe_env(env: &dm_dpi::Env) {
    let code: i32 = ffi::codeset::UTF8;
    show(
        "LOCAL_CODE = i32(1)，val_len=4",
        // SAFETY: 缓冲区是 `i32`，长度 4 与类型一致。
        unsafe { env.set_attr_raw(attr::LOCAL_CODE, ptr(&code), 4) },
    );
    show(
        "LOCAL_CODE = i32(1)，val_len=0",
        // SAFETY: 同上，只换长度口径做对照。
        unsafe { env.set_attr_raw(attr::LOCAL_CODE, ptr(&code), 0) },
    );
    let mut text = *b"UTF-8\0";
    show(
        "LOCAL_CODE = \"UTF-8\\0\"，val_len=6",
        // SAFETY: 6 字节缓冲区，长度 6 与内容一致。
        unsafe { env.set_attr_raw(attr::LOCAL_CODE, text.as_mut_ptr().cast::<c_void>(), 6) },
    );
    show(
        "LOCAL_CODE = \"UTF-8\"，val_len=5",
        // SAFETY: 缓冲区 6 字节，这里只标 5 字节，不会越界。
        unsafe { env.set_attr_raw(attr::LOCAL_CODE, text.as_mut_ptr().cast::<c_void>(), 5) },
    );
    let lang: i32 = 1;
    show(
        "LANG_ID = 1（英文错误消息）",
        // SAFETY: 缓冲区是 `i32`，长度 4。
        unsafe { env.set_attr_raw(attr::LANG_ID, ptr(&lang), 4) },
    );
}

/// 一次插入的四个参数值。**必须活到 `exec` 之后**：驱动的参数绑定存的是指针。
///
/// 文本一律以 **UTF-16** 绑定（`DSQL_C_WCHAR`）：本机数据库字符集是 GB18030，
/// 窄字节路径不做转换，UTF-8 字面量会被当成 GB18030 存坏（见探针 G 段结论）。
struct Insert {
    id: i64,
    /// UTF-16 缓冲区。NULL 参数也要放一个占位单元——本机实测传空指针会被
    /// 驱动拒绝（`[-70047] 无效的绑定数据指针缓冲长度`）。
    wide: Vec<u16>,
    is_null: bool,
    amt: f64,
    ts: Timestamp,
    ind: [SLength; 4],
}

impl Insert {
    fn new(id: i64, name: Option<&str>) -> Self {
        let (wide, is_null) = match name {
            Some(text) => (text.encode_utf16().collect(), false),
            None => (vec![0u16; 1], true),
        };
        Self {
            id,
            wide,
            is_null,
            amt: 2.5,
            ts: Timestamp {
                year: 2024,
                month: 5,
                day: 27,
                hour: 10,
                minute: 11,
                second: 12,
                fraction: 123_456,
            },
            ind: [0; 4],
        }
    }

    /// 绑定并执行，返回影响行数。
    fn run(&mut self, stmt: &Stmt) -> Result<u64, DpiError> {
        self.bind(stmt)?;
        stmt.exec()?;
        stmt.affected_rows()
    }

    fn bind(&mut self, stmt: &Stmt) -> Result<(), DpiError> {
        self.ind[0] = 8;
        // SAFETY: 缓冲区是 `self` 的字段，`self` 活到 `exec` 之后；BIGINT ↔ i64。
        unsafe {
            stmt.bind_param(
                1,
                param::INPUT,
                ctype::SBIGINT,
                sqltype::BIGINT,
                19,
                0,
                ptr(&self.id),
                8,
                &mut self.ind[0],
            )?;
        }

        // 文本参数走 UTF-16：`DSQL_C_WCHAR` 由驱动做客户端 ↔ 服务端字符集转换。
        // 裸指针先取出来，避免同时借用 `self.name_wide` 与 `self.ind`。
        let buf_len = SLength::try_from(self.wide.len() * 2).unwrap_or(0);
        self.ind[1] = if self.is_null {
            indicator::NULL_DATA
        } else {
            buf_len
        };
        // SAFETY: 缓冲区在 `self.name_wide` 里，活到 `exec` 之后；长度是字节数，
        // 与 `DSQL_C_WCHAR` 的口径一致（不以 NUL 结尾）。
        unsafe {
            stmt.bind_param(
                2,
                param::INPUT,
                ctype::WCHAR,
                sqltype::VARCHAR,
                64,
                0,
                self.wide.as_ptr().cast_mut().cast::<c_void>(),
                buf_len,
                &mut self.ind[1],
            )?;
        }

        self.ind[2] = 8;
        // SAFETY: DOUBLE ↔ f64。
        unsafe {
            stmt.bind_param(
                3,
                param::INPUT,
                ctype::DOUBLE,
                sqltype::DOUBLE,
                15,
                0,
                ptr(&self.amt),
                8,
                &mut self.ind[2],
            )?;
        }

        let ts_len = SLength::try_from(std::mem::size_of::<Timestamp>()).unwrap_or(16);
        self.ind[3] = ts_len;
        // SAFETY: `Timestamp` 与 `dpi_timestamp_t` 内存布局一致（`#[repr(C)]`，16 字节）。
        unsafe {
            stmt.bind_param(
                4,
                param::INPUT,
                ctype::TIMESTAMP,
                sqltype::TIMESTAMP,
                26,
                6,
                ptr(&self.ts),
                ts_len,
                &mut self.ind[3],
            )?;
        }
        Ok(())
    }
}

/// 用字面量写一行中文，再分别按 UTF-16 与本地字节读回来。
///
/// 判据是**宽字符读回等于原文**（这才是服务端语义正确），而不是「字节原样回显」——
/// 后者在 GB18030 库上会把 UTF-8 字节当 GB18030 存坏，字节虽然一致、内容是错的。
fn check_round_trip(conn: &Conn) {
    let literal = "中文OK";
    let counter = format!("select lengthb(name), length(name) from {TABLE} where id = 1");
    // ① 宽字符读回。
    match scalar_text(conn, &format!("select name from {TABLE} where id = 1")) {
        Ok(text) if text == literal => {
            println!("[OK] 宽字符读回 == 原文（{literal}）：服务端语义正确");
        }
        Ok(text) => println!("[!!] 宽字符读回 != 原文：读到 {text:?}（期望 {literal:?}）"),
        Err(e) => println!("[!!] 宽字符读回失败：{e}"),
    }
    // ② 本地字节（诊断用）。
    match scalar_bytes(conn, &format!("select name from {TABLE} where id = 1")) {
        Ok(Some(bytes)) => println!(
            "[..] 本地字节 = {}（UTF-8 会是 {}）",
            hex(&bytes),
            hex(literal.as_bytes())
        ),
        Ok(None) => println!("[..] 本地字节：NULL"),
        Err(e) => println!("[!!] 本地字节读取失败：{e}"),
    }
    // ③ 服务端口径：GB18030 下「中文OK」应是 lengthb=6 / length=4。
    match scalar_pair(conn, &counter) {
        Ok((bytes, chars)) => {
            println!("[..] 服务端 lengthb={bytes} length={chars}（正确值应为 6 / 4）")
        }
        Err(e) => println!("[!!] 读 lengthb/length 失败：{e}"),
    }
}

/// 读一行单列文本的原始字节。
fn scalar_bytes(conn: &Conn, sql: &str) -> Result<Option<Vec<u8>>, DpiError> {
    let stmt = conn.stmt()?;
    stmt.exec_direct(sql)?;
    let value = if stmt.fetch()? {
        stmt.column_bytes(1)?
    } else {
        None
    };
    stmt.close_cursor();
    Ok(value)
}

/// 读一行单列文本（走 UTF-16，驱动做字符集转换）。
fn scalar_text(conn: &Conn, sql: &str) -> Result<String, DpiError> {
    let stmt = conn.stmt()?;
    stmt.exec_direct(sql)?;
    let value = if stmt.fetch()? {
        stmt.column_text(1)?.unwrap_or_else(|| "<NULL>".to_owned())
    } else {
        "<没有行>".to_owned()
    };
    stmt.close_cursor();
    Ok(value)
}

/// 读一行两列整数（`lengthb` / `length`）。
fn scalar_pair(conn: &Conn, sql: &str) -> Result<(i64, i64), DpiError> {
    let stmt = conn.stmt()?;
    stmt.exec_direct(sql)?;
    let value = if stmt.fetch()? {
        (
            stmt.column_i64(1)?.unwrap_or(-1),
            stmt.column_i64(2)?.unwrap_or(-1),
        )
    } else {
        (-1, -1)
    };
    stmt.close_cursor();
    Ok(value)
}

/// 读当前模式（`SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID)`，与 ODBC 侧口径一致）。
fn current_schema(conn: &Conn) -> Result<String, DpiError> {
    scalar_text(
        conn,
        "select SF_GET_SCHEMA_NAME_BY_ID(CURRENT_SCHID) from dual",
    )
}

fn show(what: &str, result: Result<(), DpiError>) {
    match result {
        Ok(()) => println!("[OK] {what}"),
        Err(e) => println!("[!!] {what} → {e}"),
    }
}

fn show_val<T: std::fmt::Debug>(what: &str, result: Result<T, DpiError>) {
    match result {
        Ok(v) => println!("[OK] {what} = {v:?}"),
        Err(e) => println!("[!!] {what} → {e}"),
    }
}

/// 取 `&T` 的裸指针（驱动只读，但 C 接口要 `void*`）。
fn ptr<T>(value: &T) -> *mut c_void {
    std::ptr::from_ref(value).cast_mut().cast()
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn fmt_ts(ts: Timestamp) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
        ts.year, ts.month, ts.day, ts.hour, ts.minute, ts.second, ts.fraction
    )
}
