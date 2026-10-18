//! 大对象：跨分块边界完整取回，`NULL` 与空值的区分，以及增补平面字符的实测边界
//! （§6.4 / §6.5 / §14 V-08）。
//!
//! 达梦的 `CLOB` / `BLOB` 走的是 `SQLGetData` 的分块路径（`result.rs` 的首趟试探 +
//! 按指标重取；宽字符首趟按列的声明长度定，`TEXT`/`CLOB` 这类没有上界的列保持 4 KiB）。
//! 这里故意把数据做得比块大好几倍，好让「指标解释成总长度还是剩余长度」这类驱动差异
//! 暴露出来——`result.rs` 的实现不依赖任一种解释，靠的是「成功收尾必补 NUL」与
//! 「截断时缓冲必被填满」这两条 ODBC 保证。
//!
//! 增补平面字符（如 🀄）是**另一类**问题，而且读写两侧都出过事：见
//! `clob_astral_character_probe` 的观测表、两条 `*_is_not_exact_today` 缺陷钉板与
//! §6.5 的实测记录。

use dm_core::backend::FetchState;
use dm_core::{DmValue, RowBuf};

use dm_odbc::lob::{ChunkState, LOB_CHUNK_BYTES, LobRead};
use dm_odbc::{FetchScratch, describe_columns, fetch};

use crate::common::{T_LOB, cell_text, dm_test};

/// 8 KiB 的确定性字节串（覆盖 0x00 与 0xFF，避免「恰好全一样」蒙混过关）。
fn blob_8192() -> Vec<u8> {
    (0..8192).map(|i| (i % 256) as u8).collect()
}

dm_test!(fn lob_clob_spans_several_chunks(h) {
    h.apply_schema();

    // 2500 个「达梦」= 5000 个字符 = 10000 个 UTF-16 码元 = 20 KiB，
    // 是 4 KiB 试探块（2048 码元）的近 5 倍。
    let text = "达梦".repeat(2500);
    let args = [DmValue::Int(1), DmValue::Text(text.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );

    let Some(rows) = h.try_rows(&format!("select C from {T_LOB} where ID = 1")) else {
        return;
    };
    let got = rows.first().and_then(|r| cell_text(r, 0));
    h.eq(
        "CLOB 长度一致",
        got.as_ref().map(String::len),
        Some(text.len()),
    );
    h.eq("CLOB 内容一致", got, Some(text));

    // 服务端侧交叉验证：`length(CLOB)` 在达梦上数**字符**（§7 真机实测补注⑥）。
    h.expect_scalar(
        "服务端 length(CLOB) 数字符",
        &format!("select length(C) from {T_LOB} where ID = 1"),
        "5000",
    );
});

dm_test!(fn lob_blob_spans_several_chunks(h) {
    h.apply_schema();

    let bytes = blob_8192();
    let args = [DmValue::Int(1), DmValue::Bytes(bytes.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, B) values (?, ?)"),
        &args,
    );

    let Some(rows) = h.try_rows(&format!("select B from {T_LOB} where ID = 1")) else {
        return;
    };
    match rows.first().and_then(|r| r.first()) {
        Some(DmValue::Bytes(got)) => {
            h.eq("BLOB 字节数一致", got.len(), bytes.len());
            h.eq("BLOB 逐字节一致", got.clone(), bytes);
        }
        other => h.fail(format!("BLOB 取回来不是 Bytes：{other:?}")),
    }

    // 服务端侧交叉验证：`length(BLOB)` 数**字节**（与 CLOB 相反）。
    h.expect_scalar(
        "服务端 length(BLOB) 数字节",
        &format!("select length(B) from {T_LOB} where ID = 1"),
        "8192",
    );
});

dm_test!(fn lob_null_and_empty_stay_distinguishable(h) {
    h.apply_schema();

    // 1 = NULL，2 = 空串/空字节，3 = 有内容。三行连着放，读回来的形态必须两两不同。
    let nulls: [DmValue; 3] = [DmValue::Int(1), DmValue::Null, DmValue::Null];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C, B) values (?, ?, ?)"),
        &nulls,
    );
    let empties: [DmValue; 3] = [
        DmValue::Int(2),
        DmValue::Text(String::new()),
        DmValue::Bytes(Vec::new()),
    ];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C, B) values (?, ?, ?)"),
        &empties,
    );
    let filled: [DmValue; 3] = [
        DmValue::Int(3),
        DmValue::Text("x".to_owned()),
        DmValue::Bytes(vec![0x00, 0xff]),
    ];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C, B) values (?, ?, ?)"),
        &filled,
    );

    let Some(rows) = h.try_rows(&format!("select ID, C, B from {T_LOB} order by ID")) else {
        return;
    };
    h.eq("三行都在", rows.len(), 3);
    let Some(null_row) = rows.first() else { return };
    let Some(empty_row) = rows.get(1) else { return };
    let Some(filled_row) = rows.get(2) else { return };

    h.check(
        "NULL 行是 NULL 而不是空串",
        matches!(null_row.get(1), Some(DmValue::Null)) && matches!(null_row.get(2), Some(DmValue::Null)),
    );
    h.check(
        "空 CLOB 是空文本而不是 NULL",
        matches!(empty_row.get(1), Some(DmValue::Text(s)) if s.is_empty()),
    );
    h.check(
        "空 BLOB 是零长度 Bytes 而不是 NULL",
        matches!(empty_row.get(2), Some(DmValue::Bytes(b)) if b.is_empty()),
    );
    h.check(
        "有内容的行原样回来",
        matches!(filled_row.get(1), Some(DmValue::Text(s)) if s == "x")
            && matches!(filled_row.get(2), Some(DmValue::Bytes(b)) if b == &[0x00, 0xff]),
    );

    // 客户端判别得开，服务端侧也要对应：`C IS NULL` 不能写成 `select C is null`（§6.8 实测）。
    h.expect_scalar(
        "服务端确认第 1 行是 NULL",
        &format!(
            "select CASE WHEN C IS NULL THEN 'null' ELSE 'notnull' END from {T_LOB} where ID = 1"
        ),
        "null",
    );
    h.expect_scalar(
        "服务端确认第 2 行不是 NULL",
        &format!(
            "select CASE WHEN C IS NULL THEN 'null' ELSE 'notnull' END from {T_LOB} where ID = 2"
        ),
        "notnull",
    );
});

dm_test!(fn lob_one_mib_blob_round_trip(h) {
    h.apply_schema();

    // T6.1 的门禁探针：1 MiB 远超 4 KiB 试探块（256 个分块），也远超任何
    // 「一个 ODBC 参数缓冲该有多大」的直觉。这条用例同时在问两个问题：
    //
    // 1. 读——分块 `SQLGetData` 能不能把 1 MiB 一字节不差地拿回来；
    // 2. 写——达梦**接不接受预分配满长度缓冲**的参数（§6.5 的 T6.2 待验证项）。
    //    若服务端/驱动在这里回错（典型是参数太长或需要延迟输入），那 `SQLPutData`
    //    就不再是「可选优化」而是必需品，本用例的错误就是那个结论的证据。
    let bytes: Vec<u8> = (0..1024 * 1024).map(|i| (i * 31 % 251) as u8).collect();
    let args = [DmValue::Int(1), DmValue::Bytes(bytes.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, B) values (?, ?)"),
        &args,
    );

    // 先问服务端：写进去的确实是 1 MiB（`length(BLOB)` 数字节，§7 真机实测补注⑧）。
    // 少了这一句，「客户端把一个很短的值当成长值」也会看起来通过。
    h.expect_scalar(
        "服务端 length(BLOB) 就是 1 MiB",
        &format!("select length(B) from {T_LOB} where ID = 1"),
        "1048576",
    );

    let Some(rows) = h.try_rows(&format!("select B from {T_LOB} where ID = 1")) else {
        return;
    };
    match rows.first().and_then(|r| r.first()) {
        Some(DmValue::Bytes(got)) => {
            h.eq("BLOB 字节数一致", got.len(), bytes.len());
            // 逐字节比对，但**不要**把 1 MiB 打进失败信息：只说第一个不同的位置。
            let first_diff = got.iter().zip(&bytes).position(|(a, b)| a != b);
            h.check_that(first_diff.is_none(), || {
                format!(
                    "BLOB 在偏移 {first_diff:?} 处不同（客户端 {} 字节 / 期望 {} 字节）",
                    got.len(),
                    bytes.len()
                )
            });
        }
        other => h.fail(format!(
            "1 MiB BLOB 取回来不是 Bytes：{}",
            other.map_or("（没有值）".to_owned(), |v| match v {
                DmValue::Null => "NULL".to_owned(),
                other => format!("{other:?}").chars().take(64).collect(),
            })
        )),
    }
});

dm_test!(fn lob_ten_mib_blob_round_trip(h) {
    h.apply_schema();

    // T6.2 / §14 V-06 的**写侧上限探针**：1 MiB 已经在 `lob_one_mib_blob_round_trip` 里过了，
    // 这条把值放大 10 倍，问的是「预分配满长度缓冲」这条路线的边界在哪。
    //
    // 这里是**严格**断言而不是「观测型」用例：T6.2 的验收判据就是「10 MiB BLOB 写成功」。
    // 如果达梦的驱动在这个尺寸上回错（典型是参数列大小超限、或要求走延迟输入），
    // 那么 §6.5 表里的 `SQLPutData` 就不是「可选优化」而是必需品——本用例的失败信息
    // 就是得出那个结论的证据，失败时请把它原样记进文档，不要改成「尽力」了事。
    let len = 10 * 1024 * 1024;
    let bytes: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
    let args = [DmValue::Int(1), DmValue::Bytes(bytes.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, B) values (?, ?)"),
        &args,
    );

    h.expect_scalar(
        "服务端 length(BLOB) 就是 10 MiB",
        &format!("select length(B) from {T_LOB} where ID = 1"),
        "10485760",
    );

    let Some(rows) = h.try_rows(&format!("select B from {T_LOB} where ID = 1")) else {
        return;
    };
    match rows.first().and_then(|r| r.first()) {
        Some(DmValue::Bytes(got)) => {
            h.eq("BLOB 字节数一致", got.len(), bytes.len());
            // 10 MiB 打进失败信息会淹掉日志：只说第一个不同的偏移。
            let first_diff = got.iter().zip(&bytes).position(|(a, b)| a != b);
            h.check_that(first_diff.is_none(), || {
                format!(
                    "10 MiB BLOB 在偏移 {first_diff:?} 处不同（客户端 {} 字节 / 期望 {} 字节）",
                    got.len(),
                    bytes.len()
                )
            });
        }
        other => h.fail(format!("10 MiB BLOB 取回来不是 Bytes：{other:?}")),
    }
});

/// CLOB 参数写边界探针的用例表：`(类别, 标称字符数, 文本)`。
///
/// ASCII 与 CJK 两个维度同时扫——「按字符数」与「按服务端字符集字节数」两种解释
/// 会给出不同的边界位置，只扫一种分不出来。
fn clob_probe_cases() -> Vec<(&'static str, usize, String)> {
    let mut cases: Vec<(&'static str, usize, String)> = Vec::new();
    for chars in [8_000usize, 8_190, 9_000, 16_000, 262_144, 600_000] {
        cases.push(("ASCII", chars, "a".repeat(chars)));
    }
    for chars in [2_500usize, 4_000, 5_000, 8_000, 16_000] {
        cases.push(("CJK", chars, "达梦".repeat(chars / 2)));
    }
    // 代理对：1 个增补平面字符 = 1 个 `char` = 2 个 UTF-16 码元，
    // 用来分辨服务端 `length()` 数的是字符还是码元。
    cases.push(("代理对", 3_000, "达梦🀄".repeat(1_000)));
    cases
}

dm_test!(fn clob_parameter_write_boundary(h) {
    h.apply_schema();

    // T6.2 / T6.3 的**量**：参数化写 `CLOB`，达梦在哪个尺寸上拒掉？
    //
    // `DmValue::Text` 现在绑成 `SQL_C_WCHAR` + `SQL_VARCHAR`（`param.rs` 的 `type_of`）。
    // 这条探针第一次跑出来的结论就否掉了一个假设：**纯 ASCII / CJK 文本没有尺寸上限**
    // （600000 个 ASCII、16000 个 CJK 都写进去了），`-70013 转换失败` 只在
    // **增补平面字符**上出现，而且是写侧比读侧更晚才出问题（见 `clob_astral_character_probe`）。
    // 保留这条表的用处：它是「达梦换了版本边界会不会挪」的回归观测，也是
    // 「服务端 `length()` 数的是字符还是码元」的对照（代理对那行给出答案：字符）。
    //
    // 只做观测，不做判据：达梦换版本边界会挪，钉死数字是把测试变成绊脚石。
    // 观测表默认被测试框架吞掉，要看到它请用 `--nocapture` 跑；
    // 结论回写 §6.5 与 §12.3 的 M6 出口记录。
    let mut observations: Vec<String> = Vec::new();
    for (i, (kind, chars, text)) in clob_probe_cases().into_iter().enumerate() {
        let actual = text.chars().count();
        let args = [DmValue::Int(100 + i as i32), DmValue::Text(text)];
        let outcome = h.exec_params(
            &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
            &args,
        );
        let label = format!("{kind} {chars} 字符（实际 {actual}）");
        match outcome {
            Ok(_) => observations.push(format!("{label}: OK")),
            Err(err) => {
                let code = err
                    .server_code_and_message()
                    .map_or_else(|| "无服务端码".to_owned(), |(code, _)| code.to_string());
                observations.push(format!("{label}: 拒绝 {code} / {:?}", err.kind));
            }
        }
    }

    let known_ok = observations
        .iter()
        .any(|line| line.starts_with("CJK 5000") && line.ends_with("OK"));
    h.check_that(known_ok, || {
        format!("5000 字符的 CLOB 参数写应当成功（既有用例钉过的水位）：{observations:#?}")
    });

    println!("CLOB 参数写边界观测（ASCII / CJK / 代理对）：");
    for line in &observations {
        println!("  {line}");
    }
});

/// 把一次「读 CLOB 文本」压成一行观测：成功报字符数与（可选）内容是否一致，
/// 失败报服务端码 + 错误种类 + 首行消息。
///
/// 用宏而不是函数，是因为 `Harness` 的类型参数（生命周期/泛型）在这里没有名字可用，
/// 而观测用例不需要为它起名字。给了第三个参数时，输出会多一段 `内容=EQ/NE`——
/// **长度对不代表内容是那个值**，量代理对时这一栏才是判据。
macro_rules! describe_read {
    ($h:expr, $sql:expr $(, $expected:expr)?) => {{
        match $h.rows($sql) {
            Ok(rows) => match rows.first().and_then(|row| row.first()) {
                Some(DmValue::Text(text)) => {
                    let summary = format!("OK {} 字符", text.chars().count());
                    $(
                        let summary = format!(
                            "{summary} / 内容={}",
                            match first_char_diff(&text, $expected) {
                                None => "EQ".to_owned(),
                                Some((index, got, want)) => format!(
                                    "NE（第 {index} 个字符：期望 U+{:04X}，读回 U+{:04X}）",
                                    want as u32, got as u32
                                ),
                            }
                        );
                    )?
                    summary
                }
                Some(other) => format!("非文本 {other:?}"),
                None => "没有行".to_owned(),
            },
            Err(err) => {
                let code = err
                    .server_code_and_message()
                    .map_or_else(|| "无服务端码".to_owned(), |(code, _)| code.to_string());
                // 完整诊断链都要看：`lone surrogate`（驱动给的码元流本身非法）与
                // 「驱动写入了 0 个码元」（续取推不动）是两种不同的失败，得分开记。
                let detail = format!("{err}")
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!("Err {code} / {:?}：{detail}", err.kind)
            }
        }
    }};
}

/// 把字符串的前 `n` 个 UTF-16 码元打成 `U+XXXX` 列表——代理对的问题要看**码元**，不看字符。
fn units_hex(text: &str, n: usize) -> String {
    text.encode_utf16()
        .take(n)
        .map(|unit| format!("U+{unit:04X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 读回值与期望值的第一个不同字符，附上两个码点（`None` = 逐字符一致）。
///
/// 返回 `(下标, 读回的字符, 期望的字符)`；比整串 `==` 多出来的信息是「换成了哪个字符」——
/// 增补平面字符被替换时，只看 `NE` 是查不出换成了什么的。
fn first_char_diff(got: &str, want: &str) -> Option<(usize, char, char)> {
    got.chars()
        .zip(want.chars())
        .enumerate()
        .find_map(|(index, (got, want))| (got != want).then_some((index, got, want)))
}

dm_test!(fn clob_astral_character_probe(h) {
    h.apply_schema();

    // 增补平面字符（U+1F404 🀄，1 个 `char` = 2 个 UTF-16 码元）的观测表。
    //
    // 已经量到的事实：增补平面字符在这个驱动上**保不住码点**——`A🀄B` 写进去读回来是
    // `A𖾟B`（另一个合法的增补平面字符），1 次重复也这样，说明与分不分趟无关；
    // 值更长（≥ 40000 码元的非 BMP 文本）时服务端连写都拒（`-70013 转换失败`）；
    // 同一形状的**纯 BMP** 文本（5000 字符到 400000 字符）往返逐字一致。
    // 还有一条：参数写进去的 `A🀄B` 服务端 `length(C)=3`，而 SQL 字面量写进去的
    // `length(C)=4`，且服务端判定两者**不相等**——两条写路径对代理对的解释就不一致。
    //
    // 责任在写侧还是读侧目前**还没定位**（读回来的那个码点也是合法的，无从判断是存错了
    // 还是解码错了）。关键手法：判据尽量落在**服务端自己算的窄字符结果**上
    // （`length()`、`case when C = '…'`），这样「服务端存的是什么」不必经过我们的解码器。
    //
    // 只观测、不做判据（唯一断言是纯 BMP 对照组，那是回归网），结论已回写 §6.5 / §14 V-08；
    // 内容维度的缺陷由两条 `*_is_not_exact_today` 用例钉住。
    // 观测表默认被测试框架吞掉，要看请加 `--nocapture`。
    let mut observations: Vec<String> = Vec::new();

    // ① 小值（3 字符 / 4 码元，单趟 4 KiB 试探就能取全）、走参数绑定。
    //
    // 注意 `describe_read!` 的期望值是**语法替换**进展开里的，所以绝不能叫 `text`
    // （那正是宏内部读回值的绑定名，会被遮蔽成「自己等于自己」）。
    let small = "A🀄B";
    let args = [DmValue::Int(1), DmValue::Text(small.to_owned())];
    observations.push(format!(
        "① 小值参数写（3 字符 / 4 码元）: {}",
        match h.exec_params(
            &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
            &args
        ) {
            Ok(_) => "OK".to_owned(),
            Err(err) => format!("拒绝 / {:?}", err.kind),
        }
    ));
    observations.push(format!(
        "① 服务端 length(C): {:?}",
        h.scalar(&format!("select length(C) from {T_LOB} where ID = 1"))
            .ok()
            .flatten()
    ));
    observations.push(format!(
        "① 服务端认为等于字面量 A🀄B: {:?}",
        h.scalar(&format!(
            "select case when C = 'A🀄B' then 'EQ' else 'NE' end from {T_LOB} where ID = 1"
        ))
        .ok()
        .flatten()
    ));
    observations.push(format!(
        "① 读回小值: {}",
        describe_read!(h, &format!("select C from {T_LOB} where ID = 1"), small)
    ));

    // ② 同一个值改走**字面量**写（不经过参数绑定）：若它读得回来，责任就在参数路径上。
    observations.push(format!(
        "② 字面量写: {}",
        match h.exec(&format!("insert into {T_LOB} (ID, C) values (2, 'A🀄B')")) {
            Ok(_) => "OK".to_owned(),
            Err(err) => format!("拒绝 / {:?}", err.kind),
        }
    ));
    observations.push(format!(
        "② 服务端 length(C): {:?}",
        h.scalar(&format!("select length(C) from {T_LOB} where ID = 2"))
            .ok()
            .flatten()
    ));
    observations.push(format!(
        "② 读回字面量值: {}",
        describe_read!(h, &format!("select C from {T_LOB} where ID = 2"), small)
    ));

    // ③ 放大：找「写被拒」与「读变坏」各自从哪一档开始（511/512/513 用来试窗口边界）。
    for (i, reps) in [
        1usize, 100, 511, 512, 513, 1_000, 10_000, 15_000, 20_000, 100_000,
    ]
    .into_iter()
    .enumerate()
    {
        // 期望值叫 `payload` 而不是 `text`：后者是 `describe_read!` 内部读回值的绑定名，
        // 同名会被语法替换遮蔽成「自己等于自己」（见 ① 的注释）。
        let payload = "达梦🀄".repeat(reps);
        let id = 100 + i as i32;
        let args = [DmValue::Int(id), DmValue::Text(payload.clone())];
        let write = match h.exec_params(
            &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
            &args,
        ) {
            Ok(_) => "OK".to_owned(),
            Err(err) => {
                let code = err
                    .server_code_and_message()
                    .map_or_else(|| "无服务端码".to_owned(), |(code, _)| code.to_string());
                format!("拒绝 {code} / {:?}", err.kind)
            }
        };
        let length = h
            .scalar(&format!("select length(C) from {T_LOB} where ID = {id}"))
            .ok()
            .flatten();
        let read = describe_read!(
            h,
            &format!("select C from {T_LOB} where ID = {id}"),
            &payload
        );
        observations.push(format!(
            "③ {reps} 次重复（{} 字符 / {} 码元）: 写={write} 服务端 length={length:?} 读={read}",
            payload.chars().count(),
            payload.encode_utf16().count()
        ));
    }

    // 对照组：纯 BMP 文本走同一条参数路径必须完好。这是回归网，不是观测项。
    let control = "达梦".repeat(20_000); // 40000 字符 / 80000 码元，远超单趟试探块
    let args = [DmValue::Int(9), DmValue::Text(control)];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );
    h.expect_scalar(
        "对照组：纯 BMP 文本 length(C) 是 40000",
        &format!("select length(C) from {T_LOB} where ID = 9"),
        "40000",
    );

    println!("CLOB 增补平面字符观测：");
    for line in &observations {
        println!("  {line}");
    }
});

dm_test!(fn clob_astral_round_trip_is_not_exact_today(h) {
    h.apply_schema();

    // **这是一条「钉住已知驱动缺陷」的用例，不是能力回归网。**
    //
    // 真机实测（§6.5）：写 `A🀄B`（3 字符 / 4 码元，单趟试探就取得完，读写两侧都没有
    // 分趟的借口）读回来是 `A𖾟B`——中间那个增补平面字符被换成了**另一个合法的**增补
    // 平面字符。长度对、字符数对、内容错，而且与长度无关（1 次重复也换字）。
    // 所以 §6.5 对非 BMP 字符**不承诺内容保真**：要存非 BMP 文本，请走
    // `VARBINARY`/`BLOB` 的 UTF-8 字节（字节路径有真机逐字节用例）。
    //
    // 断言写成「必须仍然不保真」是故意的：驱动哪天修好，这条会红，红的时候请回来把断言
    // 反过来、更新 §6.5 的承诺档，并删掉这段注释。
    let payload = "A🀄B";
    let args = [DmValue::Int(1), DmValue::Text(payload.to_owned())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );

    let got = h
        .rows(&format!("select C from {T_LOB} where ID = 1"))
        .map(|rows| rows.first().and_then(|row| cell_text(row, 0)));

    println!(
        "短增补平面值：绑定码元=[{}] 读回={}",
        units_hex(payload, 8),
        match &got {
            Ok(Some(text)) => format!("[{}]", units_hex(text, 8)),
            Ok(None) => "NULL".to_owned(),
            Err(err) => format!("Err {err:?}"),
        }
    );

    match got {
        Ok(Some(text)) => {
            h.eq("字符数一致", text.chars().count(), payload.chars().count());
            h.check_that(text != payload, || {
                format!(
                    "驱动已修好增补平面字符（读回 [{}]）：请更新 §6.5 并删除本断言",
                    units_hex(&text, 8)
                )
            });
        }
        // 读不回来（`Decode` / 连接层报错）同样是「不保真」，算通过——但要留痕。
        Err(err) => println!("短增补平面值：读取报错——{err:?}"),
        Ok(None) => h.fail("短增补平面值读回来是 NULL"),
    }
});

dm_test!(fn clob_astral_round_trip_over_one_probe_is_not_exact_today(h) {
    h.apply_schema();

    // 同一条缺陷的**长值**一侧：3000 字符 / 4000 码元的非 BMP 文本，超过 4 KiB 首趟
    // 试探（2048 码元）后必然续取。实测两种表现都出现过——内容被换字，或续取交出畸形
    // UTF-16 而在 `String::from_utf16` 处报 `Decode`。这条只钉住「不保真」这个事实，
    // 不钉住具体是哪种表现：驱动换一次表现就改一次测试是没意义的。
    let payload = "达梦🀄".repeat(1_000);
    let args = [DmValue::Int(1), DmValue::Text(payload.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );
    h.expect_scalar(
        "服务端 length(CLOB) 数的是字符",
        &format!("select length(C) from {T_LOB} where ID = 1"),
        "3000",
    );

    let got = h
        .rows(&format!("select C from {T_LOB} where ID = 1"))
        .map(|rows| rows.first().and_then(|row| cell_text(row, 0)));

    match got {
        Ok(Some(text)) => {
            h.eq("字符数一致", text.chars().count(), payload.chars().count());
            let detail = match first_char_diff(&text, &payload) {
                None => "逐字符一致".to_owned(),
                Some((index, got, want)) => format!(
                    "第 {index} 个字符：期望 U+{:04X}，读回 U+{:04X}",
                    want as u32, got as u32
                ),
            };
            println!("长增补平面值（4000 码元）：{detail}");
            h.check_that(text != payload, || {
                format!("驱动已修好增补平面字符（{detail}）：请更新 §6.5 并删除本断言")
            });
        }
        // 续取交出畸形 UTF-16 时 `read_wide` 报 `Decode`——同样是「不保真」，算通过。
        Err(err) => println!("长增补平面值：读取报错（畸形 UTF-16 走 `Decode`）——{err:?}"),
        Ok(None) => h.fail("长增补平面值读回来是 NULL"),
    }
});

dm_test!(fn clob_large_bmp_text_round_trips(h) {
    h.apply_schema();

    // §14 V-08 的写读双侧证据，也是 T6.2 降级档（「预分配满长度缓冲」）在**文本**上的对应物：
    // 400000 个 BMP 字符 = 800000 个 UTF-16 码元 = 1.6 MiB，远超单趟 4 KiB 试探块，
    // 读侧必然走多趟 `SQLGetData` 累积（`result.rs` 的 `read_wide`）。
    //
    // 更正记录：这条用例早先写的是「1 MiB 级 CLOB 参数写必被达梦拒掉（`-70013 转换失败`）」。
    // 那是**误判**——当时的边界探针（`.m6d.log`，已随工作区清理删除）证明 600000 个 ASCII / 16000 个 CJK 字符都能写进去；
    // 真正触发 `-70013` 的是**增补平面字符（代理对）**，见 `clob_astral_character_probe`。
    // 教训：机制要从探针里量出来，不能从一次失败的现场反推。
    let text = "达梦".repeat(200_000); // 400000 字符 / 800000 码元
    let args = [DmValue::Int(1), DmValue::Text(text.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );

    h.expect_scalar(
        "服务端 length(CLOB) 是 400000 个字符",
        &format!("select length(C) from {T_LOB} where ID = 1"),
        "400000",
    );

    let Some(rows) = h.try_rows(&format!("select C from {T_LOB} where ID = 1")) else {
        return;
    };
    match rows.first().and_then(|r| r.first()) {
        Some(DmValue::Text(got)) => {
            h.eq("读回字符数一致", got.chars().count(), 400_000);
            let first_diff = got.chars().zip(text.chars()).position(|(a, b)| a != b);
            h.check_that(first_diff.is_none(), || {
                format!("大 CLOB 在第 {first_diff:?} 个字符处不同")
            });
        }
        other => h.fail(format!("大 CLOB 取回来不是 Text：{other:?}")),
    }
});

dm_test!(fn lob_fetch_reuses_one_rowbuf_and_scratch(h) {
    h.apply_schema();

    // 三行内容各不相同，且一行比一行大——同一个 `RowBuf` 与 `FetchScratch` 跨行复用，
    // 行与行之间不能串味（`fetch` 每次进来先 `clear()`，容量保留）。
    let payloads = ["达梦".repeat(1500), "达梦".repeat(3000), "达梦".repeat(4500)];
    for (index, text) in payloads.iter().enumerate() {
        let args = [DmValue::Int(index as i32), DmValue::Text(text.clone())];
        h.try_exec_params(
            &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
            &args,
        );
    }

    let length_in_char = h.length_in_char;
    let mut scratch = FetchScratch::new();
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.exec_direct(&format!("select ID, C from {T_LOB} order by ID")) {
        return h.record("exec_direct", &err);
    }
    let columns = match describe_columns(&mut stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns", &err),
    };
    h.eq("两列", columns.len(), 2);

    let mut buf = RowBuf::new(columns.len());
    let mut collected: Vec<Option<String>> = Vec::new();
    let mut reused_capacity = 0usize;
    loop {
        match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
            Ok(FetchState::Rows { rows }) => {
                h.eq("每次取回恰好一行", rows, 1);
                h.eq("缓冲区列数没变", buf.columns(), 2);
                reused_capacity = reused_capacity.max(scratch.wide_capacity());
                collected.push(buf.row(0).and_then(|r| cell_text(r, 1)));
            }
            Ok(FetchState::Exhausted) => break,
            Ok(other) => return h.fail(format!("未知的 FetchState：{other:?}")),
            Err(err) => return h.record("fetch", &err),
        }
    }
    h.eq(
        "三行的内容都对",
        collected,
        payloads.iter().map(|s| Some(s.clone())).collect::<Vec<_>>(),
    );
    h.check_that(reused_capacity >= 2048, || {
        format!("取值暂存被停用/过小（最大 {reused_capacity} 码元），分块路径可能没走到")
    });
});

dm_test!(fn lob_binary_scratch_keeps_capacity_across_rows(h) {
    h.apply_schema();

    // 二进制取值的暂存必须**跨行留住容量**（§5.4 纪律 2「缓冲复用」）。
    // 这里钉的是一个真实踩过的坑：`read_binary` 曾用 `std::mem::take` 把缓冲连容量
    // 一起交出去，于是下一行又从 4 KiB 起步、按 `resize` 一格格涨——1 MiB 的 BLOB
    // 每行几百次重分配与拷贝。修法是复制出值、`clear()` 保留容量；这条用例在每行
    // 取回后立刻查 `binary_capacity()`，`take` 的写法在这里必然回 0。
    let payloads: [Vec<u8>; 3] = [
        (0..100u32).map(|i| (i % 256) as u8).collect(),
        (0..5000u32).map(|i| (i % 256) as u8).collect(),
        (0..9000u32).map(|i| (i % 256) as u8).collect(),
    ];
    for (index, bytes) in payloads.iter().enumerate() {
        let args = [DmValue::Int(index as i32), DmValue::Bytes(bytes.clone())];
        h.try_exec_params(
            &format!("insert into {T_LOB} (ID, B) values (?, ?)"),
            &args,
        );
    }

    let length_in_char = h.length_in_char;
    let mut scratch = FetchScratch::new();
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => return h.record("SQLAllocHandle", &err),
    };
    if let Err(err) = stmt.exec_direct(&format!("select ID, B from {T_LOB} order by ID")) {
        return h.record("exec_direct", &err);
    }
    let columns = match describe_columns(&mut stmt, length_in_char) {
        Ok(columns) => columns,
        Err(err) => return h.record("describe_columns", &err),
    };

    let mut buf = RowBuf::new(columns.len());
    let mut collected: Vec<Vec<u8>> = Vec::new();
    let mut capacity_after_row: Vec<usize> = Vec::new();
    loop {
        match fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
            Ok(FetchState::Rows { rows }) => {
                h.eq("每次取回恰好一行", rows, 1);
                let value = match buf.row(0).and_then(|r| r.get(1)) {
                    Some(DmValue::Bytes(bytes)) => bytes.clone(),
                    other => return h.fail(format!("BLOB 取回来不是 Bytes：{other:?}")),
                };
                // 取完之后暂存里应该还留着至少放得下这一行的容量。
                capacity_after_row.push(scratch.binary_capacity());
                h.check_that(scratch.binary_capacity() >= value.len(), || {
                    format!(
                        "第 {} 行取完后二进制暂存只剩 {} 字节容量（该行 {} 字节）：\
                         缓冲被交出去了，跨行复用失效",
                        collected.len() + 1,
                        scratch.binary_capacity(),
                        value.len()
                    )
                });
                collected.push(value);
            }
            Ok(FetchState::Exhausted) => break,
            Ok(other) => return h.fail(format!("未知的 FetchState：{other:?}")),
            Err(err) => return h.record("fetch", &err),
        }
    }

    h.eq("取回三行", collected.len(), 3);
    h.eq(
        "三行的字节数都对",
        collected.iter().map(Vec::len).collect::<Vec<_>>(),
        payloads.iter().map(Vec::len).collect::<Vec<_>>(),
    );
    h.check("三行逐字节一致", collected == payloads);
    h.check_that(capacity_after_row.len() == 3, || {
        format!("容量采样数不对：{capacity_after_row:?}")
    });
});

// ---------------------------------------------------------------------------
// T6.3：分块读（`LobRead`）——「不为了看一眼就先全量载入」
// ---------------------------------------------------------------------------

/// 把 `select <一列>` 的第一行按 [`LobRead`] 分块读回。
///
/// 与 `fetch` 的区别是本用例自己发 `SQLFetch`：`fetch` 的语义就是「把整行取成
/// `DmValue`」，那正是分块读要绕开的事。
fn chunked_read_first_row(
    h: &mut crate::common::Harness,
    sql: &str,
    column_index: u16,
) -> Option<(Vec<u8>, usize, usize)> {
    let mut stmt = match h.conn.allocate_statement_owned() {
        Ok(stmt) => stmt,
        Err(err) => {
            h.record("SQLAllocHandle", &err);
            return None;
        }
    };
    if let Err(err) = stmt.exec_direct(sql) {
        h.record("exec_direct", &err);
        return None;
    }
    let columns = match describe_columns(&mut stmt, h.length_in_char) {
        Ok(columns) => columns,
        Err(err) => {
            h.record("describe_columns", &err);
            return None;
        }
    };
    let Some(meta) = columns.get(usize::from(column_index) - 1).cloned() else {
        h.fail(format!("结果集里没有第 {column_index} 列"));
        return None;
    };

    // `LobRead::open` 自己把游标推进到第一行（真机路径上的安全封装；裸 `SQLFetch`
    // 是 unsafe，集成测试里不写 unsafe）。取不到行 ⇒ `Ok(None)`。
    let mut reader = match LobRead::open(&mut stmt, column_index, &meta) {
        Ok(Some(reader)) => reader,
        Ok(None) => {
            h.fail("结果集里一行都没有");
            return None;
        }
        Err(err) => {
            h.record("LobRead::open", &err);
            return None;
        }
    };

    let mut value: Vec<u8> = Vec::new();
    let mut chunks = 0usize;
    let mut max_added = 0usize;
    loop {
        let before = value.len();
        match reader.next_chunk(&mut value) {
            Ok(ChunkState::Done) => break,
            Ok(ChunkState::More) => {}
            Err(err) => {
                h.record("LobRead::next_chunk", &err);
                return None;
            }
        }
        let added = value.len() - before;
        max_added = max_added.max(added);
        if added > 0 {
            chunks += 1;
        }
    }
    h.check("这一列不是 NULL", !reader.is_null());
    h.eq(
        "读到的字节数与读取器自报一致",
        reader.read_bytes(),
        value.len(),
    );
    Some((value, chunks, max_added))
}

dm_test!(fn lob_chunked_reader_takes_a_one_mib_clob_apart(h) {
    h.apply_schema();

    // T6.3 的判据：「1 MiB CLOB 分块往返」。用 400000 个 **BMP** 字符
    // （400000 个 UTF-16 码元、1.2 MB UTF-8）：BMP 文本的整值往返是 §6.5 保证的，
    // 所以这条用例失败一定是分块逻辑的错，而不是驱动那条增补平面缺陷。
    let text = "达梦".repeat(200_000);
    let args = [DmValue::Int(1), DmValue::Text(text.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, C) values (?, ?)"),
        &args,
    );

    let Some((value, chunks, max_added)) =
        chunked_read_first_row(h, &format!("select C from {T_LOB} where ID = 1"), 1)
    else {
        return;
    };

    // 一趟的窗口是 2048 个码元；BMP 字符最多 3 字节 UTF-8，所以一趟最多 6144 字节
    // （留一个码元的余量给跨趟的高代理）。**这一条才是「不放大」的功能性证据**：
    // 1.2 MB 的值从来不会在一趟里出现。
    let ceiling = 3 * (LOB_CHUNK_BYTES / 2 + 1);
    h.check_that(max_added <= ceiling, || {
        format!("单趟交付了 {max_added} 字节，超过窗口上界 {ceiling}：分块读退化成了整值读")
    });
    // 400000 码元 ÷ 2048 = 196 趟（趟数明显偏少就说明驱动把整值一趟给了我们）。
    h.check_that(chunks >= 190, || {
        format!("只分了 {chunks} 趟：1.2 MB 的 CLOB 应该有 196 趟（窗口 2048 码元）")
    });
    h.eq("分块读回的字节数一致", value.len(), text.len());
    h.check("分块读回的内容逐字节一致", value == text.as_bytes());
});

dm_test!(fn lob_chunked_reader_takes_a_one_mib_blob_apart(h) {
    h.apply_schema();

    // 二进制侧：1 MiB 确定性字节串（含 0x00 与 0xFF）。`BLOB` 逐字节精确是 §6.5 保证的。
    let bytes: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
    let args = [DmValue::Int(1), DmValue::Bytes(bytes.clone())];
    h.try_exec_params(
        &format!("insert into {T_LOB} (ID, B) values (?, ?)"),
        &args,
    );

    let Some((value, chunks, max_added)) =
        chunked_read_first_row(h, &format!("select B from {T_LOB} where ID = 1"), 1)
    else {
        return;
    };

    h.check_that(max_added <= LOB_CHUNK_BYTES, || {
        format!("单趟交付了 {max_added} 字节，超过窗口 {LOB_CHUNK_BYTES}：分块读退化成了整值读")
    });
    h.check_that(chunks >= 200, || {
        format!("只分了 {chunks} 趟：1 MiB 的 BLOB 应该有 256 趟（窗口 4 KiB）")
    });
    h.eq("分块读回的字节数一致", value.len(), bytes.len());
    h.check("分块读回的内容逐字节一致", value == bytes);
});
