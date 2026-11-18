//! 字符集：宽字符 API 的回归（CT-07 / CT-10）与中文往返。
//!
//! 达梦默认服务端字符集是 GB18030，而 ODBC 的宽字符接口走 UTF-16。两边的转换由驱动做，
//! 我们能钉住的只有两件事：**宽字符接口没被换成窄的**，以及**中文原样回来**。

use dm_core::DmValue;

use crate::common::{T_LOB, T_STR, cell_text, dm_test};

dm_test!(fn charset_wchar_api_is_two_bytes_wide(h) {
    // CT-10：`odbc-api` 的 `narrow` feature 一旦被启用，`SqlChar` 就是 `u8`，
    // 于是中文写入即乱码、`SQL_DBMS_NAME` 直接 panic（§4 实测记录）。
    // 这条断言是那个 feature 的回归守卫。
    h.eq(
        "SqlChar 必须是宽字符（未启用 narrow）",
        std::mem::size_of::<odbc_api::handles::SqlChar>(),
        2,
    );
});

dm_test!(fn charset_server_reports_gb18030(h) {
    // §4.1 实测：达梦默认 `unicode() = 0`（GB18030）。这条只做观测，不做强制——
    // 换一台服务端字符集不同的实例时，`charset_check` 的行为见 §14 V-18。
    let Some(rows) = h.try_rows("select unicode() from dual") else {
        return;
    };
    h.eq(
        "unicode() 可读",
        rows.first().and_then(|r| cell_text(r, 0)),
        Some("0".to_owned()),
    );
});

dm_test!(fn charset_chinese_text_round_trips_through_varchar(h) {
    h.apply_schema();

    let text = "中文测试：达梦的 GB18030 与 UTF-16 往返，含标点「，。？」";
    let args = [
        DmValue::Int(1),
        DmValue::Text(text.to_owned()),
        DmValue::Bytes(vec![0x0a, 0x0b, 0x0c]),
        DmValue::Int(42),
    ];
    h.try_exec_params(
        &format!("insert into {T_STR} (ID, S, B, N) values (?, ?, ?, ?)"),
        &args,
    );

    let Some(rows) = h.try_rows(&format!("select S from {T_STR} where ID = 1")) else {
        return;
    };
    h.eq(
        "中文原样回来",
        rows.first().and_then(|r| cell_text(r, 0)),
        Some(text.to_owned()),
    );

    // 服务端侧佐证：`varchar` 按字节计量（§4.4 `LENGTH_IN_CHAR=0`），所以中文字节的
    // 长度必然 ≥ 字符数。这里不写死字节数——GB18030 的常用汉字是 3 字节、生僻字 4 字节，
    // 写死就等于把「哪些字是常用字」也钉进了测试。
    let char_count = text.chars().count();
    match h.scalar(&format!("select length(S) from {T_STR} where ID = 1")) {
        Ok(Some(len)) => match len.parse::<usize>() {
            Ok(bytes) => h.check_that(bytes >= char_count, || {
                format!("length(S) = {bytes} 小于字符数 {char_count}，说明中文被截断了")
            }),
            Err(_) => h.fail(format!("length(S) 不是数字：{len}")),
        },
        Ok(None) => h.fail("length(S) 返回了 NULL"),
        Err(err) => h.record("length(S)", &err),
    }

    h.expect_scalar(
        "二进制列原样（十六进制比对）",
        &format!("select rawtohex(B) from {T_STR} where ID = 1"),
        "0A0B0C",
    );
});

dm_test!(fn charset_chinese_text_round_trips_through_clob(h) {
    h.apply_schema();

    // 2000 个汉字：远超 4 KiB 的分块试探边界，走的是 `SQL_LONGVARCHAR` 的分块路径。
    let text = "达梦".repeat(1000);
    let args = [DmValue::Int(2), DmValue::Text(text.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );

    let Some(rows) = h.try_rows(&format!("select C from {T_LOB} where ID = 2")) else {
        return;
    };
    let got = rows.first().and_then(|r| cell_text(r, 0));
    h.eq(
        "CLOB 中文长度一致",
        got.as_ref().map(String::len),
        Some(text.len()),
    );
    h.eq("CLOB 中文内容一致", got, Some(text));

    // `length(CLOB)` 数字符（§7 真机实测补注⑥）：2000 个汉字 ⇒ 2000。
    h.expect_scalar(
        "CLOB 的 length() 数字符",
        &format!("select length(C) from {T_LOB} where ID = 2"),
        "2000",
    );
});
