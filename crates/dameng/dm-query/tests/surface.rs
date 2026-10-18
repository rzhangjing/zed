//! 公开面上「写了但从没跑过」的那几处的离线判据（Round 15 补 AC-01 覆盖率）。
//!
//! 这些用例不新加能力，只把已经承诺出去的公开行为钉住：`Args` 的存取与清空、
//! `RowStream` 的 `Debug`、非查询语句给空流、首批失败在 `open` 就报错、以及
//! `Transaction` 上那几个透传方法。它们此前一行都没被执行过——覆盖率的账要真，
//! 先把「写了但从没跑过」的路径跑一遍，而不是为了数字写空壳。

mod common;

use common::{Fixture, columns};
use dm_core::{DataType, DmValue};
use dm_query::{Args, DmErrorKind, IntoArgs};

/// 一份「一列 INT、一行」的计划。
fn one_int_row() -> impl FnOnce(&mut common::Plan) {
    |plan: &mut common::Plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)]];
    }
}

#[test]
fn args_expose_their_values_and_can_be_cleared() {
    let mut args = vec![DmValue::Int(1), DmValue::Text("a".to_owned())].into_args();
    assert_eq!(args.len(), 2);
    assert!(!args.is_empty());
    assert_eq!(args.get(1), Some(&DmValue::Text("a".to_owned())));
    assert_eq!(
        args.values(),
        &[DmValue::Int(1), DmValue::Text("a".to_owned())]
    );

    // 清空但保留容量：`execute_many` 逐行复用同一个 `Args` 时走的就是这条。
    args.clear();
    assert!(args.is_empty());
    assert!(args.values().is_empty());

    // 取出底层 `Vec`（搬进阻塞闭包用，不再复制一份）。
    let owned: Args = vec![DmValue::Int(7)].into_args();
    assert_eq!(owned.into_values().len(), 1);
}

#[tokio::test]
async fn a_row_stream_reports_its_shape_and_debug() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(one_int_row());

    let conn = fx.conn().await;
    let stream = conn.fetch_stream("select ID from T", ()).await.unwrap();

    // 首批是同步等出来的，所以流一建好就已经在 `Ready` 档、缓冲里有那一行。
    let repr = format!("{stream:?}");
    assert!(repr.starts_with("RowStream"), "{repr}");
    assert!(repr.contains("columns: 1"), "{repr}");
    assert!(repr.contains("buffered: 1"), "{repr}");
    assert!(repr.contains("Ready"), "首批取完之后停在 Ready 档：{repr}");

    // 流读尽之后才丢：连接可以原样回池（`Drop` 不会白标脏）。
    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_non_query_statement_gives_an_empty_stream() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(2));

    let conn = fx.conn().await;
    let stream = conn.fetch_stream("update T set ID = 1", ()).await.unwrap();

    // 不是查询就没有列可按行解，也不该凭空造出一行：给空流，要影响行数请用 `execute`。
    // （`finished: true` = 一建好就取尽，调用方拿不到任何一行。）
    assert!(stream.columns().is_empty());
    let repr = format!("{stream:?}");
    assert!(repr.contains("columns: 0"), "{repr}");
    assert!(repr.contains("finished: true"), "{repr}");

    drop(stream);
    fx.shutdown().await;
}

#[tokio::test]
async fn a_failed_first_batch_surfaces_when_the_stream_opens() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)]];
        plan.fail_execute_times = 1;
    });

    let conn = fx.conn().await;
    // 首批的错在 `open` 就以 `Err` 出来，而不是变成一个「第一次 poll 才报错」的流。
    let err = conn
        .fetch_stream("select ID from T where ID = ?", (1i32,))
        .await
        .unwrap_err();
    assert!(matches!(err.kind, DmErrorKind::Io), "{err:?}");

    fx.shutdown().await;
}

#[tokio::test]
async fn a_transaction_passes_its_connection_through() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)]];
        plan.affected = Some(1);
    });

    let conn = fx.conn().await;
    let mut txn = conn.begin().await.unwrap();

    // `conn()` 借出来的就是事务那条连接（`begin` 消耗了 `PooledConn`，连接没换）。
    let conn_id = txn.conn().id();
    assert_eq!(txn.conn().id(), conn_id);

    assert!(txn.query("select ID from T", ()).await.is_ok());
    assert_eq!(
        txn.execute("update T set ID = ?", (1i32,)).await.unwrap(),
        1
    );
    assert!(
        txn.execute_many("insert into T (ID) values (?)", [(1i32,), (2i32,)])
            .await
            .is_ok()
    );

    txn.commit().await.unwrap();
    fx.shutdown().await;
}
