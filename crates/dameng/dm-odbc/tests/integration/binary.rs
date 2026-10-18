//! 二进制路径：逐字节往返、与服务端 `rawtohex` 比对、定长/变长/大对象三者的差异。
//!
//! 二进制没有「编码」这一层可以背锅，所以它是**最能证明参数缓冲布局正确**的用例：
//! 值缓冲里少写一个字节、指标报错一个字节、或者尾部被当成字符串补了 NUL，都会在这里现形。

use dm_core::DmValue;

use crate::common::{T_LOB, T_STR, cell_canonical, dm_test, hex, native_code, note_error};

dm_test!(fn binary_all_byte_values_round_trip(h) {
    h.apply_schema();

    // 覆盖 0x00（最容易在字符串路径里被当作结尾）与 0xFF（最容易在有符号处理里出错）。
    let bytes: Vec<u8> = (0..200u16).map(|i| i as u8).collect();
    let args = [
        DmValue::Int(1),
        DmValue::Text("bin".to_owned()),
        DmValue::Bytes(bytes.clone()),
        DmValue::Int(0),
    ];
    h.try_exec_params(
        &format!("insert into {T_STR} (ID, S, B, N) values (?, ?, ?, ?)"),
        &args,
    );

    let Some(rows) = h.try_rows(&format!("select B from {T_STR} where ID = 1")) else {
        return;
    };
    h.eq(
        "逐字节一致",
        rows.first().map(|r| cell_canonical(r, 0)),
        Some(hex(&bytes)),
    );

    // 服务端侧独立计算：`rawtohex` 给的是服务端自己看到的字节。
    h.expect_scalar(
        "服务端 rawtohex 与客户端一致",
        &format!("select rawtohex(B) from {T_STR} where ID = 1"),
        &hex(&bytes),
    );
    // 达梦的 `LENGTH` 对**二进制**列不是字节数：真机实测 3 字节的 `VARBINARY` 回 `6`，
    // 200 字节回 `400`——它数的是十六进制表示的字符数。`lengthb` 才是字节数
    // （同一组实测：3 → `3`，`octet_length` 与 `length` 一样是 2×）。
    // 这两条一起断言，是为了让「我们拿到的确实是 200 字节」和「服务端的口径偏了一倍」
    // 同时被钉住，将来达梦改了行为会立刻现形。
    h.expect_scalar(
        "服务端 length(B) 是十六进制字符数（2× 字节）",
        &format!("select length(B) from {T_STR} where ID = 1"),
        "400",
    );
    h.expect_scalar(
        "服务端 lengthb(B) 才是字节数",
        &format!("select lengthb(B) from {T_STR} where ID = 1"),
        "200",
    );
});

dm_test!(fn binary_empty_is_not_null(h) {
    h.apply_schema();

    let rows_in: [Vec<DmValue>; 2] = [
        vec![DmValue::Int(1), DmValue::Null, DmValue::Null, DmValue::Null],
        vec![
            DmValue::Int(2),
            DmValue::Null,
            DmValue::Bytes(Vec::new()),
            DmValue::Null,
        ],
    ];
    for args in &rows_in {
        h.try_exec_params(
            &format!("insert into {T_STR} (ID, S, B, N) values (?, ?, ?, ?)"),
            args,
        );
    }

    let Some(rows) = h.try_rows(&format!("select B from {T_STR} order by ID")) else {
        return;
    };
    h.check(
        "NULL 行是 DmValue::Null",
        matches!(rows.first().and_then(|r| r.first()), Some(DmValue::Null)),
    );
    h.check(
        "空二进制行是零长度 Bytes",
        matches!(rows.get(1).and_then(|r| r.first()), Some(DmValue::Bytes(b)) if b.is_empty()),
    );
    h.expect_scalar(
        "服务端也认为第 2 行不是 NULL",
        &format!(
            "select CASE WHEN B IS NULL THEN 'null' ELSE 'notnull' END from {T_STR} where ID = 2"
        ),
        "notnull",
    );
});

dm_test!(fn binary_binary_column_pads_and_varbinary_does_not(h) {
    h.apply_schema();

    // `C_BINARY(8)` 定长：写 3 个字节，服务端补零到 8；`C_VARBINARY(300)` 变长：写多少是多少。
    let args = vec![
        DmValue::Int(1),
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Null,
        DmValue::Bytes(vec![0xAA, 0xBB, 0xCC]),
        DmValue::Bytes(vec![0xAA, 0xBB, 0xCC]),
        DmValue::Null,
    ];
    let placeholders = std::iter::repeat_n("?", args.len())
        .collect::<Vec<_>>()
        .join(", ");
    h.try_exec_params(
        &format!("insert into DM_T_TYPES (ID, C_CHAR, C_VARCHAR, C_CLOB, C_TINYINT, C_SMALLINT, C_INT, C_BIGINT, C_DECIMAL, C_NUMERIC, C_REAL, C_DOUBLE, C_BIT, C_DATE, C_TIME, C_TIMESTAMP, C_VARBINARY, C_BINARY, C_BLOB) values ({placeholders})"),
        &args,
    );

    let Some(rows) = h.try_rows("select C_VARBINARY, C_BINARY from DM_T_TYPES where ID = 1") else {
        return;
    };
    let Some(row) = rows.first() else {
        return h.fail("没有取回该行");
    };
    h.eq(
        "VARBINARY 原样 3 字节",
        cell_canonical(row, 0),
        "AABBCC".to_string(),
    );
    h.eq(
        "BINARY(8) 补零到 8 字节",
        cell_canonical(row, 1),
        "AABBCC0000000000".to_string(),
    );

    h.expect_scalar(
        "服务端 rawtohex(VARBINARY)",
        "select rawtohex(C_VARBINARY) from DM_T_TYPES where ID = 1",
        "AABBCC",
    );
    h.expect_scalar(
        "服务端 rawtohex(BINARY)",
        "select rawtohex(C_BINARY) from DM_T_TYPES where ID = 1",
        "AABBCC0000000000",
    );
});

dm_test!(fn binary_blob_refuses_server_side_text_conversion(h) {
    h.apply_schema();

    // §6.8 真机实测：`RAWTOHEX(BLOB)` 报 `HY000 / -5403 参数不兼容`。
    // 这条**故意失败**，只作观测——它记录的是服务端的能力边界。
    match h.scalar(&format!("select rawtohex(B) from {T_LOB} where ID = 1")) {
        Ok(value) => h.fail(format!("RAWTOHEX(BLOB) 竟然成功了：{value:?}（§6.8 记 -5403）")),
        Err(err) => {
            note_error(&err);
            h.eq(
                "RAWTOHEX(BLOB) 被拒的原生码",
                native_code(&err),
                Some(-5403),
            );
        }
    }

    // 写进去，用 `lengthb()` 核对字节数（`length()` 对二进制列是十六进制字符数，见上一条用例）。
    let bytes = vec![0x01, 0x02, 0xFE, 0xFF];
    let args = [DmValue::Int(1), DmValue::Bytes(bytes.clone())];
    h.try_exec_params(&format!("insert into {T_LOB} (ID, B) values (?, ?)"), &args);
    h.expect_scalar(
        "lengthb(BLOB) 数字节",
        &format!("select lengthb(B) from {T_LOB} where ID = 1"),
        "4",
    );

    // 想拿 BLOB 的十六进制，`CAST(B AS VARBINARY)` 也不行：真机实测 `HY000 / -6120
    // 无法转换的数据类型`。**结论：BLOB 没有服务端侧的文本形态**，唯一通路是客户端
    // 分块 `SQLGetData`——那正是下面这条客户端往返要证明的（`lob.rs` 里还有跨块的那条）。
    match h.scalar(&format!(
        "select rawtohex(cast(B as varbinary(64))) from {T_LOB} where ID = 1"
    )) {
        Ok(value) => h.fail(format!(
            "CAST(BLOB AS VARBINARY) 竟然成功了：{value:?}（真机实测是 -6120）"
        )),
        Err(err) => {
            note_error(&err);
            h.eq(
                "CAST(BLOB AS VARBINARY) 被拒的原生码",
                native_code(&err),
                Some(-6120),
            );
        }
    }

    // 客户端侧：走我们自己的分块取值，字节必须一模一样。
    let Some(rows) = h.try_rows(&format!("select B from {T_LOB} where ID = 1")) else {
        return;
    };
    h.eq(
        "客户端取回的 BLOB 逐字节一致",
        rows.first().map(|r| cell_canonical(r, 0)),
        Some(hex(&bytes)),
    );
});
