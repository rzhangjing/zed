//! 冻结面 `LobRef`/`LobStream` 的离线判据（T6.3 的「分块读」那一半）。
//!
//! 真机那一半在 `crates/dm-odbc/tests/integration/lob.rs`（1 MiB 的 CLOB/BLOB 分块读、
//! 逐字节一致）与 `tests/query_real.rs`（`#[ignore]` + `DM_TEST_DSN`）。这里钉的是
//! **中间件自己的账**：窗口怎么切、NULL 与空值怎么分开、状态挂在谁身上、出错与取消之后
//! 连接还能不能要回来。
//!
//! 为什么必须离线能测：真机上「取一块」的边界由驱动决定（`SQLGetData` 一次给多少），
//! 想在真机上构造「第 1 块失败」「第一行是 NULL、第二行是空值」这类形状，只能靠巧合并
//! 不可重复。窗口大小与逐行取值都是假后端的参数。

mod common;

use std::sync::atomic::Ordering;

use common::{FakeBackend, Fixture, columns};
use dm_core::{DataType, DmValue};
use dm_query::{DmErrorKind, LobChunk, LobRef};

/// 等一个条件成立（池的复位/回收是异步的，断言前必须等它落地）。
async fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if pred() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("等待「{what}」超时");
}

/// 造一份分块读计划：一列 CLOB，`values` 一行一个取值（`None` = SQL NULL）。
fn lob_plan(values: Vec<Option<Vec<u8>>>, window: usize) -> impl FnOnce(&mut common::Plan) {
    move |plan: &mut common::Plan| {
        plan.columns = columns(&[("BODY", DataType::LongVarChar)]);
        plan.rows = values.iter().map(|_| vec![DmValue::Null]).collect();
        plan.lob_rows = values;
        plan.lob_chunk_bytes = window;
    }
}

/// 把当前行的分块读读到底，返回拼起来的字节。
async fn read_row(stream: &mut dm_query::LobStream<FakeBackend>) -> Vec<u8> {
    let mut value = Vec::new();
    loop {
        let state = stream.read_chunk(&mut value).await.expect("分块读失败");
        if state.is_done() {
            break;
        }
    }
    value
}

fn text(bytes: &[u8]) -> Option<Vec<u8>> {
    Some(bytes.to_vec())
}

#[tokio::test]
async fn chunks_come_in_windows_and_the_value_is_reassembled() {
    let fx = Fixture::open(2).await;
    fx.shared
        .configure(lob_plan(vec![text(b"ABCDEFGHIJ"), text(b"xyz")], 4));

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::name("BODY"))
        .await
        .unwrap();

    assert_eq!(stream.column().name, "BODY");
    assert_eq!(stream.column_index(), 0);

    // 第一行：4 + 4 + 2 + Done ⇒ 窗口真的是 4，不是「一次全给」。
    assert!(stream.next_row().await.unwrap());
    assert_eq!(read_row(&mut stream).await, b"ABCDEFGHIJ");
    assert_eq!(stream.bytes_read(), 10);
    assert!(stream.is_drained(), "读到 Done 之后这一行就算读完了");
    assert!(!stream.is_null());

    // 第二行：上一行的账要清零。
    assert!(stream.next_row().await.unwrap());
    assert_eq!(stream.bytes_read(), 0, "换行必须把上一行的字节数清零");
    assert!(!stream.is_drained());
    assert_eq!(read_row(&mut stream).await, b"xyz");
    assert_eq!(stream.rows_read(), 2);

    assert!(!stream.next_row().await.unwrap());
    assert!(stream.is_eof());

    // 分了几趟是可数的：10 字节 / 窗口 4 ⇒ 4 次（含那次 `Done`），3 字节 ⇒ 2 次。
    assert_eq!(
        fx.shared.lob_chunks(),
        6,
        "分块次数 = ceil(10/4)+1 + ceil(3/4)+1"
    );

    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn chunks_append_to_the_callers_buffer() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(lob_plan(vec![text(b"ABCDEFGHIJ")], 4));

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::index(0))
        .await
        .unwrap();
    assert!(stream.next_row().await.unwrap());

    // 追加语义：调用方自己往里塞的内容不会被清掉，也不要求 `out` 是空的。
    let mut value = b"prefix:".to_vec();
    assert_eq!(stream.read_chunk(&mut value).await.unwrap(), LobChunk::More);
    assert_eq!(value, b"prefix:ABCD");
    assert_eq!(stream.bytes_read(), 4, "只算这一列读出来的字节");

    assert_eq!(read_row(&mut stream).await, b"EFGHIJ");

    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn null_and_empty_are_told_apart() {
    let fx = Fixture::open(2).await;
    fx.shared
        .configure(lob_plan(vec![None, Some(Vec::new())], 4));

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::index(0))
        .await
        .unwrap();

    assert!(stream.next_row().await.unwrap());
    let mut value = Vec::new();
    assert_eq!(
        stream.read_chunk(&mut value).await.unwrap(),
        LobChunk::Null,
        "NULL 是 `LobChunk::Null`，不是「取了几块空数据」"
    );
    assert!(value.is_empty(), "NULL 不会往 out 里追加任何东西");
    assert!(stream.is_null());
    assert!(!stream.is_empty(), "NULL 不是空值");

    assert!(stream.next_row().await.unwrap());
    let mut value = Vec::new();
    assert_eq!(stream.read_chunk(&mut value).await.unwrap(), LobChunk::Done);
    assert!(!stream.is_null());
    assert!(stream.is_empty(), "空值是「读完了且一个字节都没有」");
    assert!(stream.is_drained());

    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn reading_before_next_row_is_rejected() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(lob_plan(vec![text(b"abc")], 4));

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::index(0))
        .await
        .unwrap();

    let mut value = Vec::new();
    let err = stream.read_chunk(&mut value).await.unwrap_err();
    assert!(matches!(err.kind, DmErrorKind::Config(_)), "{err}");
    assert!(
        err.to_string().contains("next_row"),
        "报错要说清怎么修：{err}"
    );
    assert_eq!(fx.shared.lob_starts(), 0, "定位之前不该碰驱动");

    // 定位之后同一条流立刻就能用（不是「一错到底」）。
    assert!(stream.next_row().await.unwrap());
    assert_eq!(read_row(&mut stream).await, b"abc");

    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_failed_chunk_surfaces_and_the_next_attempt_continues() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("BODY", DataType::LongVarChar)]);
        plan.rows = vec![vec![DmValue::Null]];
        plan.lob_rows = vec![text(b"ABCDEFGHIJ")];
        plan.lob_chunk_bytes = 4;
        // 第一趟取块失败（按连接丢失处置）：`out` 里的内容不受影响，状态也不作废。
        plan.lob_fail_times = 1;
    });

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::index(0))
        .await
        .unwrap();
    assert!(stream.next_row().await.unwrap());

    let mut value = b"kept:".to_vec();
    let err = stream.read_chunk(&mut value).await.unwrap_err();
    assert!(err.is_connection_lost(), "取块失败按连接丢失处置：{err}");
    assert_eq!(value, b"kept:", "失败的这一趟不许动调用方的累积区");
    assert_eq!(stream.bytes_read(), 0);

    // 状态挂在语句上 ⇒ 重试接着读，而且 `lob_start` 只该被调一次。
    let mut rest = Vec::new();
    loop {
        let state = stream.read_chunk(&mut rest).await.unwrap();
        if state.is_done() {
            break;
        }
    }
    assert_eq!(rest, b"ABCDEFGHIJ");
    assert_eq!(fx.shared.lob_starts(), 1, "状态挂上去了就不该再开一次");

    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_failed_start_is_retried_instead_of_being_remembered_as_attached() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("BODY", DataType::LongVarChar)]);
        plan.rows = vec![vec![DmValue::Null]];
        plan.lob_rows = vec![text(b"abc")];
        plan.lob_chunk_bytes = 4;
        plan.lob_start_fails = true;
    });

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::index(0))
        .await
        .unwrap();
    assert!(stream.next_row().await.unwrap());

    let mut value = Vec::new();
    let err = stream.read_chunk(&mut value).await.unwrap_err();
    // 驱动的报错原样冒出来（`Display` 只有大类名，细节在诊断链里）。
    assert!(matches!(err.kind, DmErrorKind::Syntax), "{err}");
    assert_eq!(
        err.server_code_and_message().map(|(_, text)| text),
        Some("假后端：按配置让 lob_start 失败（连接没坏）")
    );

    // `lob_start` 失败时状态**没有**挂上：修好之后重试必须从 `lob_start` 重来一次，
    // 而不是跳过它直接取块（那在真机上只会得到一句状态错）。
    fx.shared.configure(|plan| plan.lob_start_fails = false);
    assert_eq!(read_row(&mut stream).await, b"abc");
    assert_eq!(fx.shared.lob_starts(), 2, "失败的那次之后要重开一次");

    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn into_conn_returns_a_usable_connection() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("BODY", DataType::LongVarChar)]);
        plan.rows = vec![vec![DmValue::Null]];
        plan.lob_rows = vec![text(b"abcdefgh")];
        plan.lob_chunk_bytes = 4;
    });

    let conn = fx.conn().await;
    // 列名大小写不敏感（与 `Row::get` 同一口径）：按 "body" 找 "BODY"。
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::name("body"))
        .await
        .unwrap();
    assert!(stream.next_row().await.unwrap());
    assert_eq!(read_row(&mut stream).await, b"abcdefgh");

    let mut conn = stream.into_conn().await;
    assert_eq!(
        fx.shared.closes.load(Ordering::Relaxed),
        0,
        "取尽之后连接是干净的，不该被丢掉"
    );
    // 还回来的连接确实能用。
    let set = conn.query("select BODY from T", ()).await.unwrap();
    assert_eq!(set.rows().len(), 1);
    drop(conn);

    fx.shutdown().await;
}

#[tokio::test]
async fn into_conn_works_before_the_value_is_drained() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("BODY", DataType::LongVarChar)]);
        plan.rows = vec![vec![DmValue::Null]];
        plan.lob_rows = vec![text(b"abcdefghij")];
        plan.lob_chunk_bytes = 4;
    });

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::index(0))
        .await
        .unwrap();
    assert!(stream.next_row().await.unwrap());

    // 只取一块就把连接要回去：已经取到的字节留在调用方手里，`into_conn` 照样能用
    // （这也是它比 `drop` 干净的地方——不必等池复位）。
    let mut first = Vec::new();
    assert_eq!(stream.read_chunk(&mut first).await.unwrap(), LobChunk::More);
    assert_eq!(first, b"abcd");
    let mut conn = stream.into_conn().await;
    let set = conn.query("select BODY from T", ()).await.unwrap();
    assert_eq!(set.rows().len(), 1);
    drop(conn);

    fx.shutdown().await;
}

#[tokio::test]
async fn a_stream_dropped_mid_value_marks_the_connection_dirty() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("BODY", DataType::LongVarChar)]);
        plan.rows = vec![vec![DmValue::Null]];
        plan.lob_rows = vec![text(b"abcdefghij")];
        plan.lob_chunk_bytes = 4;
    });

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::index(0))
        .await
        .unwrap();
    assert!(stream.next_row().await.unwrap());
    let mut value = Vec::new();
    assert_eq!(stream.read_chunk(&mut value).await.unwrap(), LobChunk::More);
    assert_eq!(value, b"abcd");
    // 游标没读尽就把流丢掉：这条连接不能原样复用，池会先复位它。
    drop(stream);

    wait_until("半途丢弃的分块读流让连接被复位", || {
        fx.shared.resets.load(Ordering::Relaxed) >= 1
    })
    .await;

    fx.shutdown().await;
}

#[test]
fn lob_ref_reports_its_shape() {
    let by_index = LobRef::index(2);
    assert_eq!(by_index.as_index(), Some(2));
    assert_eq!(by_index.as_name(), None, "按下标指的时候没有名字");

    let by_name = LobRef::name("body");
    assert_eq!(by_name.as_name(), Some("body"));
    assert_eq!(
        by_name.as_index(),
        None,
        "按名字指的时候还没有下标——下标要等结果集列出来才算得出来"
    );
}

#[test]
fn lob_ref_resolves_like_row_get_does() {
    let cols = columns(&[("ID", DataType::Integer), ("BODY", DataType::LongVarChar)]);

    assert_eq!(LobRef::index(1).resolve(&cols).unwrap(), 1);
    // 大小写不敏感：与 `Row::get("body")` 是同一套口径（同一个 `ColumnIndex`）。
    assert_eq!(LobRef::name("body").resolve(&cols).unwrap(), 1);
    assert_eq!(LobRef::name("BODY").resolve(&cols).unwrap(), 1);

    let missing = LobRef::name("nope").resolve(&cols).unwrap_err();
    assert!(
        matches!(missing.kind, DmErrorKind::Decode { .. }),
        "找不到列与 Row::get 同口径：{missing:?}"
    );

    let out_of_range = LobRef::index(9).resolve(&cols).unwrap_err();
    assert!(
        matches!(out_of_range.kind, DmErrorKind::Decode { .. }),
        "越界也是 Decode：{out_of_range:?}"
    );
}

#[tokio::test]
async fn a_stream_reports_its_shape_and_debug() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(lob_plan(vec![text(b"abcd")], 2));

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::name("BODY"))
        .await
        .unwrap();

    assert_eq!(stream.columns().len(), 1);
    assert_eq!(stream.column_index(), 0);
    assert_eq!(stream.column().name, "BODY");
    assert_eq!(stream.bytes_read(), 0);
    assert_eq!(stream.rows_read(), 0);
    assert!(!stream.is_null());
    assert!(!stream.is_drained());
    assert!(!stream.is_eof());

    // `Debug` 只报「读者要知道的」这几件事实，不把语句句柄与连接打出来——`conn` 会把
    // 池的信息一起拉进来，对排查没帮助。
    let repr = format!("{stream:?}");
    assert!(repr.starts_with("LobStream"), "{repr}");
    assert!(repr.contains("BODY"), "{repr}");
    assert!(repr.contains(".."), "句柄与连接不该出现在 Debug 里：{repr}");

    assert!(stream.next_row().await.unwrap());
    assert_eq!(read_row(&mut stream).await, b"abcd");
    assert_eq!(stream.bytes_read(), 4);
    assert_eq!(stream.rows_read(), 1);
    assert!(!stream.next_row().await.unwrap());
    assert!(stream.is_eof());

    // 流读尽之后才丢：这条连接可以原样回池（`Drop` 不会白标脏）。
    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_statement_without_a_result_set_is_rejected() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));

    let conn = fx.conn().await;
    // 带参数的那条路（`bind_params` + `execute("")`）也一起走一遍：分块读的入口不该
    // 只在零参数时才对。
    let err = conn
        .lob_stream("insert into T (BODY) values (?)", (7i32,), LobRef::index(1))
        .await
        .unwrap_err();

    let DmErrorKind::Config(message) = &err.kind else {
        panic!("非查询语句上没有列可按块读，应当是用法错误：{err:?}");
    };
    assert!(message.contains("结果集"), "{message}");

    fx.shutdown().await;
}
