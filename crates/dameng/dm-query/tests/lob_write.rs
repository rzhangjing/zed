//! 分块 LOB **写**（`PooledConn::execute_lob`）的离线判据（T6.3 的「大 BLOB 写」那一半）。
//!
//! 真机那一半在 `crates/dm-odbc/examples/putdata_probe.rs`（探针：达梦确实会回
//! `SQL_NEED_DATA`，8 个用例逐字节回读一致）与 `tests/query_real.rs`（`#[ignore]` +
//! `DM_TEST_DSN`，多 MiB 的 BLOB/CLOB 流式写 + 回读比对）。这里钉的是**中间件自己的账**：
//!
//! - 一处 LOB 可以落在任意 `?` 位置上，而它**不在** `args` 里（`?` 总数 = `args.len() + 1`）；
//! - 源是「一趟一趟给」而不是「一次给完」：值已经在内存里的 [`LobBytes`] 与真正流式的
//!   [`LobReader`] 走的是同一条路；
//! - 形状不对是 `Config`（用法错），源自己读不出来是 `Encode`（**不是**连接故障）；
//! - 「喂到一半失败」的语句**不留**在预处理语句缓存里。
//!
//! 为什么必须离线能测：「源在第 2 趟报错」「同一处 LOB 落在第 1 个还是第 2 个 `?` 上」
//! 这类形状在真机上靠巧合、不可重复，而它们正是 `args`/`column` 口径最容易写错的地方。

mod common;

use std::io::Read;
use std::sync::atomic::Ordering;

use common::Fixture;
use dm_core::{LobBytes, LobKind, LobParam, LobReader, LobSource};
use dm_error::{DmError, DmErrorKind, Stage};

/// 等一个条件成立（池的丢弃/回收是异步的，断言前必须等它落地）。
async fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if pred() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("等待「{what}」超时");
}

/// `ops` 流水里 `prepare#<sql>` 出现的次数——用来观察语句有没有被复用。
fn prepares(fx: &Fixture, sql: &str) -> usize {
    let wanted = format!("prepare#{sql}");
    fx.shared
        .op_log()
        .iter()
        .filter(|entry| **entry == wanted)
        .count()
}

/// 一个每次只给一小段、并且可以**在第 `fail_at` 趟失败**的源。
///
/// 手写而不是用 [`LobBytes`]：要测的正是「源中途读不出来」的那条路。
struct Piecewise {
    pieces: Vec<Vec<u8>>,
    /// 第几趟（0 起算）开始失败；`None` = 从不失败。
    fail_at: Option<usize>,
    calls: usize,
}

impl Piecewise {
    fn new(pieces: Vec<Vec<u8>>, fail_at: Option<usize>) -> Self {
        Self {
            pieces,
            fail_at,
            calls: 0,
        }
    }
}

impl LobSource for Piecewise {
    fn next_chunk(&mut self, out: &mut Vec<u8>) -> Result<bool, DmError> {
        let call = self.calls;
        self.calls += 1;
        if self.fail_at == Some(call) {
            let mut err = DmError::at(DmErrorKind::Encode { column: None }, Stage::Execute);
            err.push_diag(dm_error::DiagRecord::native(
                0,
                "测试源：按配置让这一趟读不出来",
            ));
            return Err(err);
        }
        if self.pieces.is_empty() {
            return Ok(false);
        }
        out.extend_from_slice(&self.pieces.remove(0));
        Ok(true)
    }
}

/// 一个把内容按固定步长吐出来的读取器（模拟「边算边写」的流式源）。
struct Dribble {
    data: Vec<u8>,
    at: usize,
    step: usize,
}

impl Read for Dribble {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.data.len().saturating_sub(self.at);
        let take = remaining.min(self.step).min(buf.len());
        buf[..take].copy_from_slice(&self.data[self.at..self.at + take]);
        self.at += take;
        Ok(take)
    }
}

#[tokio::test]
async fn a_value_already_in_memory_is_written_through_the_same_path() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));
    let mut conn = fx.conn().await;
    let body = vec![0xA5u8; 4096];

    let affected = conn
        .execute_lob(
            "insert into T (ID, BODY) values (?, ?)",
            (7i32,),
            LobParam::new(1, LobKind::Binary, LobBytes::new(body.clone())),
        )
        .await
        .expect("分块写必须成功");

    assert_eq!(affected, 1, "影响行数来自执行结果");
    assert_eq!(fx.shared.lob_writes(), 1);
    assert_eq!(fx.shared.lob_written(), vec![body.clone()]);
    assert_eq!(fx.shared.lob_write_kinds(), vec![LobKind::Binary]);
    assert_eq!(
        fx.shared.lob_write_columns(),
        vec![1],
        "LOB 落在第 2 个 `?` 上"
    );
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn the_lob_column_may_sit_anywhere() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));
    let mut conn = fx.conn().await;

    // 同一处 LOB 分别放在第 1 个与第 2 个 `?` 上：`args` 始终只有一个值。
    let first = conn
        .execute_lob(
            "insert into T (BODY, ID) values (?, ?)",
            (7i32,),
            LobParam::new(0, LobKind::Binary, LobBytes::new(b"abc".to_vec())),
        )
        .await
        .expect("LOB 在第 1 个 `?` 上");
    let second = conn
        .execute_lob(
            "insert into T (ID, BODY) values (?, ?)",
            (7i32,),
            LobParam::new(1, LobKind::Binary, LobBytes::new(b"def".to_vec())),
        )
        .await
        .expect("LOB 在第 2 个 `?` 上");

    assert_eq!(first, 1);
    assert_eq!(second, 1);
    assert_eq!(fx.shared.lob_write_columns(), vec![0, 1]);
    assert_eq!(
        fx.shared.lob_written(),
        vec![b"abc".to_vec(), b"def".to_vec()]
    );
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_streaming_reader_is_drained_piece_by_piece() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));
    let mut conn = fx.conn().await;
    let data: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();

    // 每次 `Read::read` 最多给 7 字节：真正流式的源会分很多趟。
    let reader = Dribble {
        data: data.clone(),
        at: 0,
        step: 7,
    };
    let affected = conn
        .execute_lob(
            "insert into T (ID, BODY) values (?, ?)",
            (1i32,),
            LobParam::new(1, LobKind::Binary, LobReader::new(reader)),
        )
        .await
        .expect("流式写必须成功");

    assert_eq!(affected, 1);
    let written = fx.shared.lob_written();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0], data, "逐字节一致");
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_text_source_is_delivered_as_utf8_bytes_in_pieces() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));
    let mut conn = fx.conn().await;
    let text = "达梦 A🀄B";
    // 故意在每个字符中间切：这一层只保证「字节照实交付」，UTF-16 转换与「不切开代理对」
    // 是驱动层的事（`crates/dm-odbc/src/lob.rs` 的 `LobFeeder` 有单测）。
    let expected = text.as_bytes().to_vec();
    let pieces: Vec<Vec<u8>> = expected.chunks(2).map(<[u8]>::to_vec).collect();

    let affected = conn
        .execute_lob(
            "insert into T (ID, BODY) values (?, ?)",
            (1i32,),
            LobParam::new(1, LobKind::Text, Piecewise::new(pieces, None)),
        )
        .await
        .expect("文本源必须能分块交付");

    assert_eq!(affected, 1);
    assert_eq!(fx.shared.lob_write_kinds(), vec![LobKind::Text]);
    assert_eq!(fx.shared.lob_written(), vec![expected]);
    assert_eq!(
        String::from_utf8(fx.shared.lob_written()[0].clone()).unwrap(),
        text
    );
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn an_arity_mismatch_is_a_config_error_before_anything_is_written() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.param_count = Some(2));
    let mut conn = fx.conn().await;

    // 语句只有 2 个 `?`，但 `args` 给了两个（再加 1 处 LOB = 3）⇒ 形状不对。
    let err = conn
        .execute_lob(
            "insert into T (A, B) values (?, ?)",
            (1i32, 2i32),
            LobParam::new(1, LobKind::Binary, LobBytes::new(vec![1u8])),
        )
        .await
        .expect_err("形状不对必须报错");

    assert!(matches!(err.kind, DmErrorKind::Config(_)), "{:?}", err.kind);
    assert_eq!(err.context.stage, Stage::Bind);
    assert!(
        fx.shared.lob_written().is_empty(),
        "形状不对时一个字节都不该写"
    );
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_column_index_out_of_range_is_a_config_error() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.param_count = Some(1));
    let mut conn = fx.conn().await;

    let err = conn
        .execute_lob(
            "insert into T (BODY) values (?)",
            (),
            LobParam::new(3, LobKind::Binary, LobBytes::new(vec![1u8])),
        )
        .await
        .expect_err("列下标越界必须报错");

    assert!(matches!(err.kind, DmErrorKind::Config(_)), "{:?}", err.kind);
    assert_eq!(err.context.stage, Stage::Bind);
    assert!(fx.shared.lob_written().is_empty());
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_source_that_fails_halfway_is_an_encode_error() {
    let fx = Fixture::open(2).await;
    let mut conn = fx.conn().await;
    let source = Piecewise::new(
        vec![b"first".to_vec(), b"second".to_vec()],
        Some(1), // 第 2 趟失败
    );

    let err = conn
        .execute_lob(
            "insert into T (ID, BODY) values (?, ?)",
            (1i32,),
            LobParam::new(1, LobKind::Binary, source),
        )
        .await
        .expect_err("源读不出来必须报错");

    assert!(
        matches!(err.kind, DmErrorKind::Encode { column: None }),
        "{:?}",
        err.kind
    );
    assert_eq!(err.context.stage, Stage::Execute);
    assert!(
        !err.is_connection_lost(),
        "源自己失败不是连接故障：连接照常回池复用"
    );
    assert_eq!(
        fx.shared.lob_written(),
        vec![b"first".to_vec()],
        "失败前交出去的那半份照实留下（诊断用）"
    );
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_failed_write_is_not_left_in_the_statement_cache() {
    let fx = Fixture::open(2).await;
    // 第一次执行在「喂到一半」失败（按连接丢失处理）。
    fx.shared.configure(|plan| plan.lob_write_fail_times = 1);
    let sql = "insert into T (ID, BODY) values (?, ?)";

    {
        let mut conn = fx.conn().await;
        let err = conn
            .execute_lob(
                sql,
                (1i32,),
                LobParam::new(1, LobKind::Binary, LobBytes::new(vec![9u8; 128])),
            )
            .await
            .expect_err("第一次必须失败");
        assert!(matches!(err.kind, DmErrorKind::Io), "{:?}", err.kind);
        assert!(err.is_connection_lost());
    }

    // `Io` 意味着这条连接不再可信：池会丢掉它（而不是复位后复用）。
    wait_until("坏连接被池丢弃或复位", || {
        fx.shared.closes.load(Ordering::Relaxed) >= 1
            || fx.shared.resets.load(Ordering::Relaxed) >= 1
    })
    .await;

    fx.shared.configure(|plan| plan.affected = Some(1));
    let mut conn = fx.conn().await;
    let affected = conn
        .execute_lob(
            sql,
            (1i32,),
            LobParam::new(1, LobKind::Binary, LobBytes::new(vec![7u8; 128])),
        )
        .await
        .expect("第二次必须成功");
    assert_eq!(affected, 1);
    assert_eq!(fx.shared.lob_writes(), 2, "两次都真的走到了分块写");
    assert_eq!(
        prepares(&fx, sql),
        2,
        "失败那次的语句没有留在缓存里：第二次必须重新 prepare；{:?}",
        fx.shared.op_log()
    );
    assert_eq!(
        fx.shared.lob_written(),
        vec![vec![9u8; 128], vec![7u8; 128]],
        "失败那次只交出了第一批（128 字节整批都在第一趟里），重试那次完整交付"
    );
    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn two_successful_writes_reuse_the_same_prepared_statement() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));
    let mut conn = fx.conn().await;
    let sql = "insert into T (ID, BODY) values (?, ?)";

    for round in 0..2u8 {
        let affected = conn
            .execute_lob(
                sql,
                (i32::from(round),),
                LobParam::new(1, LobKind::Binary, LobBytes::new(vec![round; 64])),
            )
            .await
            .expect("分块写必须成功");
        assert_eq!(affected, 1);
    }

    // 第一次未命中要 prepare，第二次命中缓存 ⇒ 一共只有一次。
    // （准入判据是 `Backend::statement_reusable`；假后端默认 `true`，与 `dm-odbc` 对 DML
    // 的态度一致——只有产出结果集的语句才不可复用。默认缓存大小 16，见
    // `crates/dm-core/src/config.rs` 的 `DEFAULT_STMT_CACHE_SIZE`。）
    assert_eq!(
        prepares(&fx, sql),
        1,
        "两次分块写只应预处理一次；{:?}",
        fx.shared.op_log()
    );
    assert_eq!(fx.shared.lob_writes(), 2);
    drop(conn);
    fx.shutdown().await;
}
