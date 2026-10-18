//! 大对象探针：`CLOB` / `BLOB` 怎么**写**、怎么**读**。
//!
//! 背景（`dmPython/vlong.c:165,219-225`）：达梦的长列既可以当普通变长字节读
//! （`DSQL_C_CHAR`/`DSQL_C_BINARY` + 大缓冲），也可以走「缓冲区里放指针」模式
//! （8 字节槽 + 指示符给长度）。写侧可用 `DSQL_DATA_AT_EXEC` +
//! `dpi_param_data`/`dpi_put_data` 分块交付。
//!
//! 本探针回答四件事：
//!
//! 1. 小 `CLOB`/`BLOB` 用普通变长读能不能完整拿到（长度 + 校验和）；
//! 2. 1 MiB `BLOB` / 200 KB `CLOB` 用 `DATA_AT_EXEC` 写是否可行，`buf_len` 该给总长还是 8；
//! 3. 写进去的字节读回来是否一致（逐块读全 + 校验和比对）；
//! 4. 指针模式（8 字节缓冲 + 指示符给长度）能不能用——将来做流式读要靠它。
//!
//! 只打印、不断言。运行：
//! `cargo +stable-x86_64-pc-windows-msvc run -p dm-dpi --example dpi_lob_probe`
//!
//! # 实测结论（本机 DM8 1-3-140，`LOCAL_CODE=UTF8`）
//!
//! - **写**：`DSQL_DATA_AT_EXEC` + `dpi_param_data`/`dpi_put_data` 可行；1 MiB `BLOB`
//!   与 200 KB `CLOB` 一次 `put_data` 交付即可，`buf_len` 给**总长**（给 8 会在
//!   `dpi_put_data` 后报 `-70026`）。多个大对象参数可以混在一条语句里，驱动会连续
//!   两次要数据。
//! - **读**：`dpi_get_data` 单次调用只要**缓冲够大**就能一次返回完整值
//!   （J/K 段：200016 字节 `CLOB`、1048576 字节 `BLOB` 一次读完、逐字节一致）。
//!   之前 C/G 段看到「64 KB 分块 + 块尾 NUL」纯粹是缓冲只给了 64 KB 造成的：
//!   截断时驱动会在缓冲末尾塞一个 NUL，所以按 64 KB 循环读会读出多余字节。
//! - **先用 NULL 缓冲问长度**（`dpi_get_data(..., NULL, 0, &len)`）有效，返回的是
//!   客户端字符集下的字节数（`CLOB` 200016、`BLOB` 1048576），据此一次分配即可。
//! - **指针模式不成立**：8 字节缓冲 + 指示符给真实长度时，驱动只把**头 8 个数据
//!   字节**写进缓冲（E 段：`ind=200016` 而 `slot` 是数据头），解引用会崩。
//! - **`DSQL_C_LOB_HANDLE` 绑定结果列在本机不可用**：`bind_col2` + `fetch` 直接
//!   `STATUS_ACCESS_VIOLATION`（F 段，默认跳过），因此定位器路径放弃。
//! - `CLOB` 用 `BINARY` 读拿到的是**库内字符集原始字节**（GB18030，148160 字节，
//!   长度精确、无 NUL 伪影），用 `CHAR` 读才是转换后的 UTF-8。`dm-dpi` 后端因此
//!   统一用 `CHAR`（客户端字符集已是 UTF-8），无需自己解码 `GB18030`。

use std::ffi::c_void;

use dm_dpi::ffi::{SLength, attr, codeset, ctype, indicator, param, sqltype};
use dm_dpi::handle::{Conn, Dpi, ReadChunk};

/// 探针表名。
const TABLE: &str = "P_LOB";

/// 分块读时每次要多少字节。
const READ_CHUNK: usize = 64 * 1024;

fn main() {
    if let Err(err) = run() {
        eprintln!("探针失败：{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let dpi = Dpi::open()?;
    let env = dpi.env()?;
    let con = env.connect()?;
    con.set_attr_i32(attr::LOCAL_CODE, codeset::UTF8)?;
    con.login("127.0.0.1:5236", "SYSDBA", "SYSDBA")?;
    println!("[OK] 登录成功");

    con.exec_direct(&format!("drop table if exists {TABLE} cascade constraints"))
        .ok();
    con.exec_direct(&format!("create table {TABLE} (id int, c clob, b blob)"))?;
    println!("[OK] 建表 {TABLE}");

    // ---- A. 小对象：字面量写入，普通变长读 -------------------------------
    println!("\n=== A. 小 CLOB/BLOB（字面量写入 + 普通变长读）===");
    con.exec_direct(&format!(
        "insert into {TABLE} values (1, '小 CLOB 中文', 0x0102030405)"
    ))?;
    read_one(&con, 1, "小")?;

    // ---- B. 大对象：DATA_AT_EXEC 写 --------------------------------------
    println!("\n=== B. 大对象写（DATA_AT_EXEC + put_data）===");
    let blob = pattern_bytes(1 << 20); // 1 MiB
    let clob = big_text(200_000); // ~200 KB 的中英混合
    let clob_bytes = clob.as_bytes();
    println!(
        "[..] BLOB {} 字节（校验和 {}）/ CLOB {} 字节（校验和 {}）",
        blob.len(),
        checksum(&blob),
        clob_bytes.len(),
        checksum(clob_bytes)
    );

    // buf_len 先用「总长度」，不行再试「8」。
    for (label, buf_len_as_total) in [("buf_len=总长", true), ("buf_len=8", false)] {
        println!("\n-- {label}");
        // 达梦 DPI 实测：一条语句里有两个 `DATA_AT_EXEC` 参数时，交完第一个参数的数据后
        // 再调 `dpi_param_data` 会报 `-70026 非法的函数调用序列`。所以先分别验证
        // 「只有一个大对象参数」的两种写法，最后再确认双参数确实不行。
        let blob_ok = write_single_lob(
            &con,
            3,
            "b",
            ctype::BINARY,
            sqltype::BLOB,
            &blob,
            buf_len_as_total,
        )?;
        let blob_fit = read_all(&con, 3, "b", ctype::BINARY)?;
        println!(
            "   B1 单 BLOB(1 MiB)：{}",
            match (&blob_ok, &blob_fit) {
                (true, Some(got)) => format!(
                    "写入成功，读回 {} 字节，{}",
                    got.len(),
                    verdict(got.as_slice() == blob.as_slice())
                ),
                (true, None) => "写入成功但读不到".to_owned(),
                (false, _) => "写入失败".to_owned(),
            }
        );

        let clob_ok = write_single_lob(
            &con,
            4,
            "c",
            ctype::CHAR,
            sqltype::CLOB,
            clob_bytes,
            buf_len_as_total,
        )?;
        let clob_fit = read_all(&con, 4, "c", ctype::CHAR)?;
        println!(
            "   B2 单 CLOB(200 KB)：{}",
            match (&clob_ok, &clob_fit) {
                (true, Some(got)) => {
                    if got.as_slice() != clob_bytes {
                        diff_report(clob_bytes, got);
                        println!("     库内 lengthb = {:?}", db_length(&con, 4, "c")?);
                    }
                    format!(
                        "写入成功，读回 {} 字节，{}",
                        got.len(),
                        verdict(got.as_slice() == clob_bytes)
                    )
                }
                (true, None) => "写入成功但读不到".to_owned(),
                (false, _) => "写入失败".to_owned(),
            }
        );

        println!("   B3 双大对象参数（CLOB + BLOB 同一条 insert）：下面这条会报错，属于已知限制");
        write_two_lobs(&con, &clob, clob_bytes, &blob, buf_len_as_total);

        if blob_ok && clob_ok {
            println!("[OK] {label} 的单参数大对象写入端到端正确");
            break;
        }
        println!("[!!] {label} 没走通，换下一种");
    }

    // ---- C. 分块读：确认单次 get_data 的上限 ------------------------------
    println!("\n=== C. 分块读（同一列反复 get_data）===");
    for (id, column, value_ctype, name) in [
        (4, "c", ctype::CHAR, "CLOB(200 KB)"),
        (3, "b", ctype::BINARY, "BLOB(1 MiB)"),
    ] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("  {name}: 没有行");
            continue;
        }
        let mut total = 0usize;
        let mut chunks = 0usize;
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            match stmt.read_chunk(1, value_ctype, &mut buf)? {
                ReadChunk::Null => {
                    println!("  {name}: NULL");
                    break;
                }
                ReadChunk::End => break,
                ReadChunk::Data(len) => {
                    chunks += 1;
                    total += len;
                    if chunks <= 3 {
                        println!("  第 {chunks} 块 {len} 字节");
                    }
                }
            }
            if chunks > 4096 {
                println!("  {name}: 块数异常，停止");
                break;
            }
        }
        println!("  {name}: 共 {chunks} 块 / {total} 字节");
        stmt.close_cursor();
        drop(stmt);
    }

    // ---- D. 指针模式：8 字节缓冲 + 指示符给长度 ---------------------------
    println!("\n=== D. 指针模式（8 字节缓冲，用列自己的 C 类型）===");
    for (id, column, value_ctype, name) in [
        (4, "c", ctype::CHAR, "CLOB(200 KB)"),
        (3, "b", ctype::BINARY, "BLOB(1 MiB)"),
    ] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("  {name}: 没有行");
            continue;
        }
        let mut slot = [0u8; 8];
        match stmt.read_chunk(1, value_ctype, &mut slot)? {
            ReadChunk::Null => println!("  {name}: NULL"),
            ReadChunk::End => println!("  {name}: 没有数据（指示符 0）"),
            ReadChunk::Data(len) => {
                let ptr = i64::from_le_bytes(slot);
                println!(
                    "  {name}: 指示符={len}（缓冲 8 字节）slot={:02X?} → {}",
                    slot,
                    if len > 8 {
                        format!("指针 0x{ptr:x}（指针模式）")
                    } else {
                        "短值，直接就是数据".to_owned()
                    }
                );
                if len > 8 && ptr != 0 {
                    // SAFETY: 探针里只在「指示符声称长度 > 8」时按指针读前 32 字节，
                    // 且该行还没被下一次 fetch 覆盖。
                    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, 32) };
                    println!("     指针前 32 字节：{:02X?}", bytes);
                }
            }
        }
        stmt.close_cursor();
        drop(stmt);
    }

    // ---- E. 指针模式：dpi_bind_col + 8 字节缓冲 --------------------------
    println!("\n=== E. 指针模式（dpi_bind_col2 + 8 字节缓冲）===");
    for (id, column, value_ctype, name) in [
        (4, "c", ctype::NCHAR, "CLOB(200 KB)"),
        (3, "b", ctype::BINARY, "BLOB(1 MiB)"),
    ] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        let mut slot = [0u8; 8];
        let mut ind: SLength = 0;
        let mut act: SLength = 0;
        // SAFETY: 8 字节槽、指示符、实长都活到 fetch 之后。
        unsafe {
            stmt.bind_col2(
                1,
                value_ctype,
                slot.as_mut_ptr().cast(),
                8,
                &raw mut ind,
                &raw mut act,
            )?;
        }
        if !stmt.fetch()? {
            println!("  {name}: 没有行");
            stmt.close_cursor();
            continue;
        }
        let ptr = i64::from_le_bytes(slot);
        println!("  {name}: ind={ind} act={act} slot={:02X?}", slot);
        if ind > 8 {
            // 这里**不**解引用：实测驱动把长度报成真实长度，却只往 8 字节缓冲里塞了
            // 头 8 个数据字节（不是指针），解引用就会崩。指针模式要用 LOB 定位器（F 段）。
            println!(
                "     指示符说 {ind} 字节，但缓冲里是头 8 个数据字节（0x{ptr:x}）⇒ 不是指针模式，被截断"
            );
        } else {
            println!("     短值，缓冲里直接是数据");
        }
        stmt.close_cursor();
        drop(stmt);
    }

    // ---- F. 定位器（结果列按 LOB_HANDLE 绑定）----------------------------
    println!("\n=== F. 定位器（结果列按 LOB_HANDLE 绑定）===");
    let f_items: Vec<(i32, &str, &str)> = if std::env::var("DM_PROBE_F").is_ok() {
        vec![(4, "c", "CLOB(200 KB)"), (3, "b", "BLOB(1 MiB)")]
    } else {
        println!("  （跳过：本机 LOB_HANDLE 绑定会崩，设 DM_PROBE_F=1 才跑）");
        Vec::new()
    };
    for (id, column, name) in f_items {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        let mut slot = [0u8; 8];
        let mut ind: SLength = 0;
        let mut act: SLength = 0;
        // SAFETY: 8 字节槽、指示符、实长都活到 fetch 之后。
        unsafe {
            stmt.bind_col2(
                1,
                ctype::LOB_HANDLE,
                slot.as_mut_ptr().cast(),
                8,
                &raw mut ind,
                &raw mut act,
            )?;
        }
        if !stmt.fetch()? {
            println!("  {name}: 没有行");
            stmt.close_cursor();
            continue;
        }
        let locator = usize::from_le_bytes(slot) as *mut c_void;
        println!(
            "  {name}: ind={ind} act={act} slot={:02X?} → 定位器 0x{:x}",
            slot, locator as usize
        );
        if locator.is_null() {
            println!("     空定位器");
            stmt.close_cursor();
            continue;
        }
        // SAFETY: 定位器来自刚 fetch 的这一行，语句还活着。
        let (len, got, checksum_got, eq) = unsafe {
            let len = stmt.lob_length(locator)?;
            let cap = usize::try_from(len.max(0)).unwrap_or(0).max(1);
            let mut buf = vec![0u8; cap];
            let value_ctype = if name.starts_with("CLOB") {
                ctype::CHAR
            } else {
                ctype::BINARY
            };
            let got = stmt.lob_read(locator, 0, value_ctype, &mut buf)?;
            let bytes = &buf[..got];
            let eq = if name.starts_with("CLOB") {
                bytes == clob_bytes
            } else {
                bytes == blob.as_slice()
            };
            (len, got, checksum(bytes), eq)
        };
        println!(
            "     定位器长度 = {len}，一次读出 {got} 字节，校验和 {checksum_got}，与原值{}",
            verdict(eq)
        );
        stmt.close_cursor();
        drop(stmt);
    }

    // ---- G. 分块读走 dpi_get_data2（看是否还丢字节）--------------------
    println!("\n=== G. 分块读走 dpi_get_data2 ===");
    for (id, column, name, want) in [
        (4, "c", "CLOB(200 KB)", Some(clob_bytes)),
        (3, "b", "BLOB(1 MiB)", Some(blob.as_slice())),
    ] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("  {name}: 没有行");
            stmt.close_cursor();
            continue;
        }
        let value_ctype = if name.starts_with("CLOB") {
            ctype::CHAR
        } else {
            ctype::BINARY
        };
        let mut collected: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; 65536];
        let mut chunks = 0usize;
        loop {
            match stmt.read_chunk2(1, value_ctype, &mut buf)? {
                ReadChunk::Data(n) => {
                    println!(
                        "     块 {chunks}: {n} 字节，末字节 {:?}",
                        buf.get(n.saturating_sub(1))
                    );
                    collected.extend_from_slice(&buf[..n]);
                    chunks += 1;
                    if n < buf.len() {
                        break;
                    }
                }
                ReadChunk::End => break,
                ReadChunk::Null => {
                    println!("  {name}: NULL");
                    break;
                }
            }
        }
        let eq = want.is_some_and(|w| collected == w);
        println!(
            "  {name}: {chunks} 块 / {} 字节，校验和 {}，与原值{}",
            collected.len(),
            checksum(&collected),
            verdict(eq)
        );
        stmt.close_cursor();
    }

    // ---- H. CLOB 分块读：WCHAR 与 BINARY 两条路径 ------------------------
    println!("\n=== H. CLOB 分块读：WCHAR / BINARY ===");
    for (label, value_ctype) in [("WCHAR", ctype::WCHAR), ("BINARY", ctype::BINARY)] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select c from {TABLE} where id = 4"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("  {label}: 没有行");
            stmt.close_cursor();
            continue;
        }
        let mut buf = vec![0u8; 65536];
        let mut total = 0usize;
        let mut chunks = 0usize;
        let mut head: Vec<u8> = Vec::new();
        println!("  {label}:");
        loop {
            match stmt.read_chunk(1, value_ctype, &mut buf)? {
                ReadChunk::Data(n) => {
                    if head.len() < 16 {
                        head.extend_from_slice(&buf[..n.min(16 - head.len())]);
                    }
                    println!(
                        "     块 {chunks}: {n} 字节，末字节 {:02X?}",
                        buf.get(n.saturating_sub(1))
                    );
                    total += n;
                    chunks += 1;
                    if n < buf.len() {
                        break;
                    }
                }
                ReadChunk::End => break,
                ReadChunk::Null => {
                    println!("     NULL");
                    break;
                }
            }
        }
        println!(
            "  {label}: {chunks} 块 / {total} 字节，头 16 字节 {:02X?}",
            head
        );
        stmt.close_cursor();
    }
    println!(
        "  原始 CLOB：{} 字节，头 16 字节 {:02X?}",
        clob_bytes.len(),
        &clob_bytes[..16.min(clob_bytes.len())]
    );

    // ---- I. 不设 LOCAL_CODE 的连接上，用 WCHAR 分块读 CLOB ---------------
    println!("\n=== I. 新连接（不设 LOCAL_CODE）用 WCHAR 分块读 CLOB ===");
    if std::env::var("DM_PROBE_I").is_ok() {
        let con2 = env.connect()?;
        con2.login(
            &std::env::var("DM_TEST_SERVER").unwrap_or_else(|_| "127.0.0.1:5236".to_owned()),
            &std::env::var("DM_TEST_USER").unwrap_or_else(|_| "SYSDBA".to_owned()),
            &std::env::var("DM_TEST_PASSWORD").unwrap_or_else(|_| "SYSDBA".to_owned()),
        )?;
        let version = con2.get_attr_str(attr::SERVER_VERSION).unwrap_or_default();
        con2.exec_direct("set schema \"DBTESTA\"")?;
        let stmt = con2.stmt()?;
        stmt.prepare(&format!("select c from {TABLE} where id = 4"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("  没有行");
        } else {
            let mut buf = vec![0u8; 65536];
            let mut units: Vec<u16> = Vec::new();
            let mut chunks = 0usize;
            loop {
                match stmt.read_chunk(1, ctype::WCHAR, &mut buf)? {
                    ReadChunk::Data(n) => {
                        let mut take = n / 2;
                        let slice: Vec<u16> = buf[..n]
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        // 每个满块末尾可能有一个 UTF-16 的 NUL 终结符，先记着不裁。
                        if take > 0 && slice[take - 1] == 0 {
                            take -= 1;
                        }
                        units.extend_from_slice(&slice[..take]);
                        println!(
                            "     块 {chunks}: {n} 字节（收 {take} 个单元），末单元 {:04X?}",
                            slice.last()
                        );
                        chunks += 1;
                        if n < buf.len() {
                            break;
                        }
                    }
                    ReadChunk::End => break,
                    ReadChunk::Null => {
                        println!("     NULL");
                        break;
                    }
                }
            }
            let text = String::from_utf16_lossy(&units);
            println!(
                "  连接字符集 {}：{chunks} 块 / {} 单元 / {} 字节（UTF-8），与原值{}",
                version,
                units.len(),
                text.len(),
                verdict(text.as_bytes() == clob_bytes)
            );
            println!("     开头 {:?}", text.chars().take(12).collect::<String>());
            stmt.close_cursor();
        }
        drop(stmt);
        con2.logout()?;
    }

    // ---- J. 先用 NULL 缓冲问长度，再一次读完 -----------------------------
    println!("\n=== J. 先问长度再一次读完 ===");
    for (id, column, name, want) in [
        (4, "c", "CLOB(200 KB)", Some(clob_bytes)),
        (3, "b", "BLOB(1 MiB)", Some(&blob[..])),
        (1, "c", "CLOB(15 B)", None),
        (2, "c", "CLOB(200 KB 中文)", None),
    ] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("  {name}: 没有行");
            stmt.close_cursor();
            continue;
        }
        let value_ctype = if column == "c" {
            ctype::CHAR
        } else {
            ctype::BINARY
        };
        let hinted = stmt.column_length(1, value_ctype)?;
        println!("  {name}: NULL 缓冲问长度 → {hinted:?}");
        if let Some(len) = hinted.filter(|l| *l > 0) {
            let cap = usize::try_from(len).unwrap_or(0);
            let mut buf = vec![0u8; cap + 2];
            let got = stmt.read_raw(1, value_ctype, &mut buf)?;
            let n = got.unwrap_or(0);
            let bytes = &buf[..n];
            let eq = want.is_some_and(|w| bytes == w);
            println!(
                "     分配 {} 字节，一次读回 {n} 字节，校验和 {}，与原值{}",
                cap + 2,
                checksum(bytes),
                if want.is_some() {
                    verdict(eq)
                } else {
                    "（未比对）"
                }
            );
        }
        stmt.close_cursor();
    }

    // ---- K. 不问长度，直接给 4 MiB 缓冲一次读 ---------------------------
    println!("\n=== K. 不问长度，一次给 4 MiB 缓冲 ===");
    for (id, column, name, want) in [
        (4, "c", "CLOB(200 KB)", Some(clob_bytes)),
        (3, "b", "BLOB(1 MiB)", Some(&blob[..])),
        (1, "c", "CLOB(15 B)", None),
    ] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("  {name}: 没有行");
            stmt.close_cursor();
            continue;
        }
        let value_ctype = if column == "c" {
            ctype::CHAR
        } else {
            ctype::BINARY
        };
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let got = stmt.read_raw(1, value_ctype, &mut buf)?;
        let n = got.unwrap_or(0);
        let eq = want.is_some_and(|w| &buf[..n] == w);
        println!(
            "  {name}: 一次返回 {n} 字节，校验和 {}，与原值{}",
            checksum(&buf[..n]),
            verdict(eq)
        );
        stmt.close_cursor();
    }

    con.exec_direct(&format!("drop table {TABLE}"))?;
    println!("\n[OK] 已删表");
    con.logout().ok();
    Ok(())
}

/// 读一行里的 `c`/`b`，打印长度与校验和（两列各用一条语句，避免「一列只能读一次」）。
fn read_one(con: &Conn, id: i64, label: &str) -> Result<(), Box<dyn std::error::Error>> {
    for (column, value_ctype, name) in [("c", ctype::CHAR, "CLOB"), ("b", ctype::BINARY, "BLOB")] {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
        stmt.exec()?;
        if !stmt.fetch()? {
            println!("[!!] {label} {name}：没有行");
            continue;
        }
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        match stmt.read_raw(1, value_ctype, &mut buf) {
            Ok(Some(len)) => {
                let bytes = &buf[..len];
                println!(
                    "  {label} {name}: 读到 {len} 字节，校验和 {}，前 16 字节 {:02X?}",
                    checksum(bytes),
                    &bytes[..len.min(16)]
                );
                if name == "CLOB" {
                    let text = String::from_utf8_lossy(&bytes[..len.min(120)]).into_owned();
                    println!("     文本开头：{text:?}");
                }
            }
            Ok(None) => println!("  {label} {name}: NULL"),
            Err(err) => println!("  {label} {name}: 读失败 {err}"),
        }
        stmt.close_cursor();
        drop(stmt);
    }
    Ok(())
}

/// 一条语句只带一个 `DATA_AT_EXEC` 大对象参数：写入 `data`。
fn write_single_lob(
    con: &Conn,
    id: i32,
    column: &str,
    value_ctype: i16,
    dtype: i16,
    data: &[u8],
    buf_len_as_total: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    con.exec_direct(&format!("delete from {TABLE} where id = {id}"))
        .ok();
    let stmt = con.stmt()?;
    stmt.prepare(&format!("insert into {TABLE} (id, {column}) values (?, ?)"))?;
    let mut ind_id: SLength = 4;
    let mut act_id: SLength = 4;
    let mut ind_lob: SLength = indicator::DATA_AT_EXEC;
    let mut act_lob: SLength = SLength::try_from(data.len()).unwrap_or(0);
    let size = if buf_len_as_total {
        SLength::try_from(data.len()).unwrap_or(0)
    } else {
        8
    };
    // SAFETY: id 与 data 都活到 exec / put_data 结束之后；指示符与实长变量同理。
    unsafe {
        stmt.bind_param2(
            1,
            param::INPUT,
            ctype::SLONG,
            sqltype::INT,
            10,
            0,
            std::ptr::from_mut(&mut { id }).cast::<c_void>(),
            4,
            &raw mut ind_id,
            &raw mut act_id,
        )?;
        stmt.bind_param2(
            2,
            param::INPUT,
            value_ctype,
            dtype,
            0,
            0,
            data.as_ptr().cast_mut().cast::<c_void>(),
            size,
            &raw mut ind_lob,
            &raw mut act_lob,
        )?;
    }
    let started = std::time::Instant::now();
    let needs = stmt.exec_deferred()?;
    let mut sent = 0usize;
    if needs {
        match stmt.param_data()? {
            Some(dst) => {
                println!(
                    "     exec 要数据（缓冲 0x{:x}），整块交付 {}",
                    dst as usize,
                    data.len()
                );
                stmt.put_data(data)?;
                sent = 1;
            }
            None => println!("     exec 没有要数据（意外）"),
        }
        match stmt.param_data() {
            Ok(Some(_)) => println!("     交完后仍在要数据（意外）"),
            Ok(None) => println!("     交完后 → 执行完毕"),
            Err(err) => println!("     交完后再问 → {err}（按已完成处理）"),
        }
    } else {
        println!("     exec_deferred → needs_data=false");
    }
    println!(
        "     put_data 次数={sent}，耗时 {:.1} ms，affected={:?}",
        started.elapsed().as_secs_f64() * 1000.0,
        stmt.affected_rows()
    );
    stmt.unbind_params().ok();
    Ok(true)
}

/// 一条语句带两个 `DATA_AT_EXEC` 参数：验证达梦是否支持（预期不支持）。
fn write_two_lobs(con: &Conn, clob: &str, clob_bytes: &[u8], blob: &[u8], buf_len_as_total: bool) {
    let id: i32 = 2;
    con.exec_direct(&format!("delete from {TABLE} where id = {id}"))
        .ok();
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let stmt = con.stmt()?;
        stmt.prepare(&format!("insert into {TABLE} (id, c, b) values (?, ?, ?)"))?;
        let mut ind_id: SLength = 4;
        let mut act_id: SLength = 4;
        let mut ind_c: SLength = indicator::DATA_AT_EXEC;
        let mut act_c: SLength = SLength::try_from(clob_bytes.len()).unwrap_or(0);
        let mut ind_b: SLength = indicator::DATA_AT_EXEC;
        let mut act_b: SLength = SLength::try_from(blob.len()).unwrap_or(0);
        let size = |len: usize| {
            if buf_len_as_total {
                SLength::try_from(len).unwrap_or(0)
            } else {
                8
            }
        };
        // SAFETY: 三个缓冲与指示符/实长变量都活到 exec 结束之后。
        unsafe {
            stmt.bind_param2(
                1,
                param::INPUT,
                ctype::SLONG,
                sqltype::INT,
                10,
                0,
                std::ptr::from_mut(&mut { id }).cast::<c_void>(),
                4,
                &raw mut ind_id,
                &raw mut act_id,
            )?;
            stmt.bind_param2(
                2,
                param::INPUT,
                ctype::CHAR,
                sqltype::CLOB,
                0,
                0,
                clob_bytes.as_ptr().cast_mut().cast::<c_void>(),
                size(clob_bytes.len()),
                &raw mut ind_c,
                &raw mut act_c,
            )?;
            stmt.bind_param2(
                3,
                param::INPUT,
                ctype::BINARY,
                sqltype::BLOB,
                0,
                0,
                blob.as_ptr().cast_mut().cast::<c_void>(),
                size(blob.len()),
                &raw mut ind_b,
                &raw mut act_b,
            )?;
        }
        let needs = stmt.exec_deferred()?;
        println!("     exec_deferred → needs_data={needs}");
        let mut round = 0usize;
        loop {
            match stmt.param_data() {
                Ok(Some(dst)) => {
                    round += 1;
                    println!("     第 {round} 次要数据（缓冲 0x{:x}）", dst as usize);
                    // 按参数顺序整块交付：第一个要的是 c，第二个要的是 b。
                    if round == 1 {
                        stmt.put_data(clob_bytes)?;
                    } else {
                        stmt.put_data(blob)?;
                    }
                }
                Ok(None) => {
                    println!("     驱动说执行完毕");
                    break;
                }
                Err(err) => {
                    println!("     {err}");
                    break;
                }
            }
            if round > 8 {
                println!("     轮次异常，停止");
                break;
            }
        }
        println!(
            "     双大对象结果：affected={:?}（1 表示两个都写进去了）",
            stmt.affected_rows()
        );
        let _ = clob;
        Ok(())
    })();
    if let Err(err) = result {
        println!("     [!!] {err}");
    }
}

/// 分块把一整列读完。
fn read_all(
    con: &Conn,
    id: i64,
    column: &str,
    value_ctype: i16,
) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    let stmt = con.stmt()?;
    stmt.prepare(&format!("select {column} from {TABLE} where id = {id}"))?;
    stmt.exec()?;
    if !stmt.fetch()? {
        return Ok(None);
    }
    let mut out = Vec::new();
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        match stmt.read_chunk(1, value_ctype, &mut buf)? {
            ReadChunk::Null => {
                stmt.close_cursor();
                return Ok(None);
            }
            ReadChunk::End => break,
            ReadChunk::Data(len) => out.extend_from_slice(&buf[..len]),
        }
    }
    stmt.close_cursor();
    Ok(Some(out))
}

/// 生成 `len` 字节的确定性图案（每字节等于下标低 8 位），便于校验和比对。
fn pattern_bytes(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i & 0xFF).unwrap_or(0))
        .collect()
}

/// 生成约 `len` 字节的文本：中文与 ASCII 交替。
fn big_text(len: usize) -> String {
    let unit = "达梦数据库 DPI 探针 ";
    let mut text = String::with_capacity(len + unit.len());
    while text.len() < len {
        text.push_str(unit);
    }
    text
}

/// 字节校验和（逐字节累加 `u64`）。
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, b| acc + u64::from(*b))
}

/// 把布尔变成中文结论。
fn verdict(ok: bool) -> &'static str {
    if ok { "一致" } else { "不一致" }
}

/// 打印两段字节的第一处差异（探针用）。
fn diff_report(expected: &[u8], got: &[u8]) {
    let n = expected.len().min(got.len());
    let pos = (0..n).find(|&i| expected[i] != got[i]);
    match pos {
        Some(i) => {
            let from = i.saturating_sub(12);
            let to = (i + 20).min(n);
            println!(
                "     首处差异在第 {i} 字节（期望 {} / 实际 {} 字节）",
                expected.len(),
                got.len()
            );
            println!("       期望[{from}..{to}] = {:02X?}", &expected[from..to]);
            println!("       实际[{from}..{to}] = {:02X?}", &got[from..to]);
            println!(
                "       期望文本 = {:?}",
                String::from_utf8_lossy(&expected[i.saturating_sub(8)..(i + 16).min(n)])
                    .into_owned()
            );
            println!(
                "       实际文本 = {:?}",
                String::from_utf8_lossy(&got[i.saturating_sub(8)..(i + 16).min(n)]).into_owned()
            );
        }
        None => println!(
            "     前缀 {} 字节一致，只是长度不同（期望 {} / 实际 {}）",
            n,
            expected.len(),
            got.len()
        ),
    }
}

/// 查一列的库内字节长度（`lengthb`）。
fn db_length(con: &Conn, id: i64, column: &str) -> Result<Option<i64>, Box<dyn std::error::Error>> {
    let stmt = con.stmt()?;
    stmt.prepare(&format!(
        "select lengthb({column}) from {TABLE} where id = {id}"
    ))?;
    stmt.exec()?;
    let out = if stmt.fetch()? {
        stmt.column_i64(1)?
    } else {
        None
    };
    stmt.close_cursor();
    Ok(out)
}
