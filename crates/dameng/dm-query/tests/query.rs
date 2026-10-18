//! `dm-query` 的离线测试组（T5.1 / T5.2 / T5.6 的 L4 层）。
//!
//! 全部跑在 `tests/common/mod.rs` 的假后端上：不连数据库、不碰 ODBC，只钉
//! **这一层自己的逻辑**——按参数分流走哪条执行路径、结果集怎么物化、列定位与解码
//! 的报错形态、事务与保存点的 SQL、流式取数的分批与标脏。
//!
//! 真机行为（达梦的错误码、`-70027`、LOB、`v$sessions`）不在这里，见
//! `tests/query_real.rs`（`#[ignore]` + `DM_TEST_DSN`）。

mod common;

use std::future::poll_fn;
use std::pin::Pin;

use common::{FakeBackend, Fixture, columns};
use dm_core::{DataType, DmValue};
use dm_error::{DmError, DmErrorKind, Stage};
use dm_query::{IsolationLevel, Row, RowStream};
use futures_core::Stream;

/// 等一个条件成立（后台回收/复位是异步的，断言前必须等它落地）。
async fn wait_until(what: &str, mut pred: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if pred() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("等待「{what}」超时");
}

/// 把一个流的下一项 `await` 出来（不引额外依赖，只用 `poll_fn`）。
async fn next_row(stream: &mut RowStream<FakeBackend>) -> Option<Result<Row, DmError>> {
    poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}

/// 收完整个流。
async fn drain(stream: &mut RowStream<FakeBackend>) -> Result<Vec<Row>, DmError> {
    let mut rows = Vec::new();
    while let Some(item) = next_row(stream).await {
        rows.push(item?);
    }
    Ok(rows)
}

/// 计数流水里以 `prefix` 开头的条目。
fn count_ops(shared: &common::FakeShared, prefix: &str) -> usize {
    shared
        .op_log()
        .iter()
        .filter(|entry| entry.starts_with(prefix))
        .count()
}

// ---------------------------------------------------------------------------
// 结果集
// ---------------------------------------------------------------------------

#[tokio::test]
async fn query_returns_rows_and_columns() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer), ("NAME", DataType::VarChar)]);
        plan.rows = vec![
            vec![DmValue::Int(1), DmValue::Text("甲".to_owned())],
            vec![DmValue::Int(2), DmValue::Text("乙".to_owned())],
        ];
    });

    let mut conn = fx.conn().await;
    let set = conn.query("select ID, NAME from T", ()).await.unwrap();

    assert_eq!(set.len(), 2);
    assert!(!set.is_empty());
    assert_eq!(set.columns().len(), 2);
    assert_eq!(set.columns()[0].name, "ID");
    assert_eq!(set.affected(), None, "结果集分支没有影响行数");
    assert_eq!(set.rows()[1].get::<String>("NAME").unwrap(), "乙");
    assert_eq!(
        fx.shared
            .closes_cursor
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "取完之后必须关游标"
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn query_decodes_typed_values_including_null() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[
            ("I", DataType::BigInt),
            ("S", DataType::VarChar),
            ("OPT", DataType::Integer),
        ]);
        plan.rows = vec![vec![
            DmValue::BigInt(-9),
            DmValue::Text("文本".to_owned()),
            DmValue::Null,
        ]];
    });

    let mut conn = fx.conn().await;
    let set = conn.query("select I, S, OPT from T", ()).await.unwrap();
    let row = set.first().expect("有一行");

    assert_eq!(row.get::<i64>("I").unwrap(), -9);
    assert_eq!(row.get::<String>("S").unwrap(), "文本");
    assert_eq!(row.get::<Option<i64>>("OPT").unwrap(), None);
    assert_eq!(row.len(), 3);
    assert_eq!(row.values()[0], DmValue::BigInt(-9));

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn column_lookup_is_case_insensitive() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("id", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(7)]];
    });

    let mut conn = fx.conn().await;
    let set = conn.query("select id from T", ()).await.unwrap();
    let row = set.first().unwrap();

    // 冻结语义②：`&str` 列名大小写不敏感（三种写法都要命中）。
    assert_eq!(row.get::<i64>("ID").unwrap(), 7);
    assert_eq!(row.get::<i64>("id").unwrap(), 7);
    assert_eq!(row.get::<i64>(String::from("Id")).unwrap(), 7);

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn unknown_column_reports_decode_error_with_name() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)]];
    });

    let mut conn = fx.conn().await;
    let set = conn.query("select ID from T", ()).await.unwrap();
    let err = set.first().unwrap().get::<i64>("NOPE").unwrap_err();

    // `DmErrorKind` 是冻结的全变体（没有 `ColumnNotFound`），列相关错误一律落 `Decode`。
    match &err.kind {
        DmErrorKind::Decode { column } => assert_eq!(column.as_deref(), Some("NOPE")),
        other => panic!("期望 Decode，实得 {other:?}"),
    }
    assert_eq!(err.context.stage, Stage::Fetch);
    let text = format!("{err}");
    assert!(text.contains("NOPE"), "错误里必须出现列名：{text}");

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn column_index_out_of_range_is_decode_error() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)]];
    });

    let mut conn = fx.conn().await;
    let set = conn.query("select ID from T", ()).await.unwrap();
    let err = set.first().unwrap().get::<i64>(5usize).unwrap_err();

    assert!(matches!(err.kind, DmErrorKind::Decode { .. }));
    assert_eq!(err.context.stage, Stage::Fetch);

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn null_into_non_option_is_null_violation() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("NAME", DataType::VarChar)]);
        plan.rows = vec![vec![DmValue::Null]];
    });

    let mut conn = fx.conn().await;
    let set = conn.query("select NAME from T", ()).await.unwrap();
    let row = set.first().unwrap();

    let err = row.get::<String>("NAME").unwrap_err();
    match &err.kind {
        DmErrorKind::NullViolation { column } => assert_eq!(column, "NAME"),
        other => panic!("期望 NullViolation，实得 {other:?}"),
    }
    // 同一个 NULL 落到 `Option<T>` 上是合法的。
    assert_eq!(row.get::<Option<String>>("NAME").unwrap(), None);

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn type_mismatch_is_decode_error() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("B", DataType::VarBinary), ("T", DataType::VarChar)]);
        plan.rows = vec![vec![
            DmValue::Bytes(vec![1, 2, 3]),
            DmValue::Text("不是数字".to_owned()),
        ]];
    });

    let mut conn = fx.conn().await;
    let set = conn.query("select B, T from T", ()).await.unwrap();
    let row = set.first().unwrap();

    // 大类就不对 → `DataTypeMismatch`。字符串解析失败 → `Decode`。两者都不是 panic。
    let mismatch = row.get::<i64>("B").unwrap_err();
    assert!(
        matches!(mismatch.kind, DmErrorKind::DataTypeMismatch { .. }),
        "期望 DataTypeMismatch，实得 {:?}",
        mismatch.kind
    );
    let parse = row.get::<i64>("T").unwrap_err();
    assert!(
        matches!(parse.kind, DmErrorKind::Decode { .. }),
        "期望 Decode，实得 {:?}",
        parse.kind
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn query_one_returns_first_row_only() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)], vec![DmValue::Int(2)]];
    });

    let mut conn = fx.conn().await;
    let one = conn.query_one("select ID from T", ()).await.unwrap();
    assert_eq!(one.unwrap().get::<i64>("ID").unwrap(), 1);

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn query_one_returns_none_when_empty() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = Vec::new();
    });

    let mut conn = fx.conn().await;
    let one = conn
        .query_one("select ID from T where 1=0", ())
        .await
        .unwrap();
    assert!(one.is_none());

    drop(conn);
    fx.shutdown().await;
}

// ---------------------------------------------------------------------------
// DML / DDL：`query` 不报错，`execute` 给影响行数
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dml_query_reports_affected_not_error() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(3));

    let mut conn = fx.conn().await;
    let set = conn.query("delete from T where ID > 10", ()).await.unwrap();

    assert!(set.is_empty());
    assert!(set.columns().is_empty(), "没有列 ⇒ 走的是 Affected 分支");
    assert_eq!(set.affected(), Some(3));

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn ddl_query_reports_affected_none_when_driver_silent() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = None);

    let mut conn = fx.conn().await;
    let set = conn.query("create table X (ID int)", ()).await.unwrap();

    assert!(set.is_empty());
    assert_eq!(set.affected(), None, "`None` = 驱动没说，不能压成 0");

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn execute_returns_affected_rows() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(5));

    let mut conn = fx.conn().await;
    assert_eq!(conn.execute("update T set V = 1", ()).await.unwrap(), 5);

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn execute_on_select_discards_the_result_set() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)], vec![DmValue::Int(2)]];
    });

    let mut conn = fx.conn().await;
    // 误用 `execute` 跑查询：结果集被丢掉，不报错，影响行数按 0 记。
    assert_eq!(conn.execute("select ID from T", ()).await.unwrap(), 0);
    assert_eq!(
        fx.shared
            .closes_cursor
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "丢弃结果集也要把游标关掉，否则连接回池时还吊着游标"
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn execute_many_reports_full_success() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", vec![(1,), (2,), (3,)])
        .await
        .unwrap();

    assert_eq!(outcome.executed, 3);
    assert_eq!(outcome.affected, 3);
    assert!(outcome.is_complete());
    assert_eq!(outcome.failed_at, None);
    // 一次 query 只桥接一次：语句只建一次，参数绑三次、执行三次。
    assert_eq!(
        count_ops(&fx.shared, "prepare#"),
        1,
        "批量写必须只预处理一条语句"
    );
    assert_eq!(count_ops(&fx.shared, "bind#"), 3);
    assert_eq!(count_ops(&fx.shared, "execute#"), 3);

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn execute_many_reports_partial_success() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.affected = Some(1);
        plan.fail_execute_times = 1;
    });

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", vec![(1,), (2,), (3,)])
        .await
        .unwrap();

    // 第一组参数就失败：部分成功不当整体失败。
    assert_eq!(outcome.failed_at, Some(0));
    assert_eq!(outcome.executed, 0);
    assert!(!outcome.is_complete());

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn execute_many_with_no_params_is_ok() {
    let fx = Fixture::open(2).await;

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many(
            "insert into T (ID) values (1)",
            Vec::<dm_query::Args>::new(),
        )
        .await
        .unwrap();

    assert_eq!(outcome.executed, 0);
    assert!(outcome.is_complete());

    drop(conn);
    fx.shutdown().await;
}

// ---------------------------------------------------------------------------
// 批量写（T6.4）：数组绑定路径 vs 默认逐行路径
// ---------------------------------------------------------------------------

/// 覆盖了数组绑定的后端，批量写必须**一次**列式调用搞定，而不是逐行循环。
#[tokio::test]
async fn execute_many_uses_the_param_array_path_when_the_backend_supports_it() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.param_count = Some(1);
        plan.array_binding = true;
        plan.array_affected = 3;
        plan.array_executed = 3;
    });

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", vec![(1,), (2,), (3,)])
        .await
        .unwrap();

    assert_eq!(
        fx.shared.param_array_calls(),
        1,
        "批量写要走列式数组绑定，且只走一次"
    );
    assert_eq!(
        fx.shared.param_array_shape(),
        vec![(1, 3)],
        "列式形状必须是「1 列（1 个 ?）× 3 行」——转置在这里被钉住"
    );
    // 走数组绑定就不再逐行 bind/execute 了，这正是 §4.7 省下来的往返。
    assert_eq!(count_ops(&fx.shared, "bind#"), 0, "数组绑定不做逐行 bind");
    assert_eq!(
        count_ops(&fx.shared, "execute#"),
        0,
        "数组绑定只打一次 SQLExecute，不该出现逐行 execute 的流水"
    );

    assert_eq!(outcome.affected, 3);
    assert_eq!(outcome.executed, 3);
    assert_eq!(outcome.failed_at, None);
    assert!(outcome.is_complete());

    drop(conn);
    fx.shutdown().await;
}

/// 数组绑定返回「中途失败」时，三个字段按后端给的数记账——**不**用下标反推成功数。
#[tokio::test]
async fn execute_many_maps_a_partial_param_array_outcome() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.param_count = Some(1);
        plan.array_binding = true;
        // 第 1 行违约。`affected` 特意给成 99：它必须原样透传，不能被 `executed`
        // 覆盖掉（这两笔账是独立的）。
        plan.array_affected = 99;
        plan.array_executed = 1;
        plan.array_failed_at = Some(1);
    });

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", vec![(1,), (2,), (3,)])
        .await
        .unwrap();

    assert_eq!(outcome.failed_at, Some(1));
    assert_eq!(
        outcome.executed, 1,
        "成功组数取后端的 executed，不用下标反推"
    );
    assert_eq!(outcome.affected, 99, "affected 原样透传");
    assert!(!outcome.is_complete());

    drop(conn);
    fx.shutdown().await;
}

/// 后端**没有**覆盖数组绑定时（trait 默认实现），结果必须与数组路径一致。
#[tokio::test]
async fn execute_many_falls_back_to_the_default_row_loop_with_the_same_result() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        // 不设 `array_binding`：假后端走 trait 的默认实现（逐行 bind + execute）。
        plan.param_count = Some(1);
        plan.affected = Some(1);
    });

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", vec![(1,), (2,), (3,)])
        .await
        .unwrap();

    assert_eq!(fx.shared.param_array_calls(), 0, "默认实现不该记数组调用");
    assert_eq!(count_ops(&fx.shared, "bind#"), 3);
    assert_eq!(count_ops(&fx.shared, "execute#"), 3);
    assert_eq!(outcome.affected, 3);
    assert_eq!(outcome.executed, 3);
    assert_eq!(outcome.failed_at, None);

    drop(conn);
    fx.shutdown().await;
}

/// 默认实现（逐行 `bind_params` + `execute`）跑到某一行失败：循环在该行**停下**，
/// 失败下标记账，整批不是 `Err`。
///
/// # 为什么这里断言的是「第 0 行」而不是「前面的行已经落地」
///
/// 假后端的失败开关是 `Plan::fail_execute_plain_times`，它的语义是「**还允许成功
/// 几次**」——实测下来，只要它非零，**第一次** `execute` 就报错（`=2` 也是
/// `failed_at = Some(0)`，流水里只有一次 `bind`/`execute`）。也就是说这个开关
/// 表达不了「前两行成功、第三行失败」。想要「已经落地的前几组不许被抹掉」那种
/// 场景，用**数组绑定**那条路（[`execute_many_maps_a_partial_param_array_outcome`]，
/// 那里后端直接给 `failed_at`/`executed`，可以精确指定）。
///
/// 这里验的是默认实现自己的记账纪律：失败下标记的是**出错那一行**，
/// 循环真的停在那里（只有一次 `bind`/`execute`），而且整批返回 `Ok` 而不是 `Err`。
#[tokio::test]
async fn execute_many_default_loop_stops_at_the_first_failing_row() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.param_count = Some(1);
        plan.affected = Some(1);
        plan.fail_execute_plain_times = 1;
    });

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", vec![(1,), (2,), (3,)])
        .await
        .expect("行级失败必须记进 BatchOutcome，不是 Err");

    assert_eq!(outcome.failed_at, Some(0), "流水：{:?}", fx.shared.op_log());
    assert_eq!(outcome.executed, 0, "第 0 行就失败，没有成功的组");
    assert_eq!(outcome.affected, 0);
    assert!(!outcome.is_complete());
    assert_eq!(count_ops(&fx.shared, "bind#"), 1, "第 0 行绑完就停");
    assert_eq!(count_ops(&fx.shared, "execute#"), 1, "不许把后两行也打出去");

    drop(conn);
    fx.shutdown().await;
}

/// 第 2 行违约、前两行已经生效：`BatchOutcome` 必须把「已生效」留在账上，
/// **不**允许拿 `failed_at` 反推出「成功 2 行」再抹掉后端的 `executed`。
#[tokio::test]
async fn execute_many_keeps_the_groups_that_already_landed() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.param_count = Some(1);
        plan.affected = Some(1);
        plan.array_binding = true;
        // 前两行成功、第 2 行违约——这三个数由后端直接给，可以精确指定。
        plan.array_affected = 2;
        plan.array_executed = 2;
        plan.array_failed_at = Some(2);
    });

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", vec![(1,), (2,), (3,)])
        .await
        .unwrap();

    assert_eq!(outcome.failed_at, Some(2), "流水：{:?}", fx.shared.op_log());
    assert_eq!(outcome.executed, 2, "第 0/1 组已经生效，不能被抹掉");
    assert_eq!(outcome.affected, 2);
    assert!(!outcome.is_complete());
    assert_eq!(
        count_ops(&fx.shared, "execute_param_array#"),
        1,
        "一次数组执行"
    );

    drop(conn);
    fx.shutdown().await;
}

/// 每组参数个数不一致：转置不出来，必须报 `Config`，而且**一条语句都不执行**。
#[tokio::test]
async fn execute_many_rejects_groups_with_uneven_widths() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.param_count = Some(2));

    // 用 `Args` 而不是元组：元组各组的**类型**必须一致（`(1, 10)` 与 `(2,)` 是两个
    // 不同类型），而这里要的正是「同一类型、宽度不同」这种非法输入。
    let mut wide = dm_query::Args::with_capacity(2);
    wide.push(1).push(10);
    let mut narrow = dm_query::Args::with_capacity(2);
    narrow.push(2);

    let mut conn = fx.conn().await;
    let err = conn
        .execute_many("insert into T (A, B) values (?, ?)", vec![wide, narrow])
        .await
        .expect_err("第 1 组少一个参数");

    assert!(
        matches!(err.kind, DmErrorKind::Config(_)),
        "形状错误是调用方用法错误，不是执行失败：{err}"
    );
    assert!(err.to_string().contains("第 1 组"), "{err}");
    assert_eq!(fx.shared.param_array_calls(), 0);
    assert_eq!(
        count_ops(&fx.shared, "execute#"),
        0,
        "形状没校验过就不该执行任何语句"
    );

    drop(conn);
    fx.shutdown().await;
}

/// 列数与语句的 `?` 个数不符：同样在转置阶段就报 `Config`。
#[tokio::test]
async fn execute_many_rejects_a_column_count_mismatch() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.param_count = Some(2));

    let mut conn = fx.conn().await;
    let err = conn
        .execute_many("insert into T (A, B) values (?, ?)", vec![(1, 10, 100)])
        .await
        .expect_err("多给了一个参数");

    assert!(matches!(err.kind, DmErrorKind::Config(_)), "{err}");
    assert_eq!(fx.shared.param_array_calls(), 0);

    drop(conn);
    fx.shutdown().await;
}

/// 空批次：合法，不执行任何语句，账是零。
#[tokio::test]
async fn execute_many_with_an_empty_batch_executes_nothing() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.param_count = Some(1);
        plan.array_binding = true;
    });

    let mut conn = fx.conn().await;
    let outcome = conn
        .execute_many("insert into T (ID) values (?)", Vec::<(i32,)>::new())
        .await
        .unwrap();

    assert_eq!(outcome.affected, 0);
    assert_eq!(outcome.executed, 0);
    assert_eq!(outcome.failed_at, None);
    assert_eq!(fx.shared.param_array_calls(), 0, "空批次不碰数组绑定");
    assert_eq!(count_ops(&fx.shared, "execute#"), 0);

    drop(conn);
    fx.shutdown().await;
}

// ---------------------------------------------------------------------------
// 流式取数
// ---------------------------------------------------------------------------

/// 造一份流式取数的计划。
///
/// `batch_rows` 取 `STREAM_CHUNK_ROWS`（64）：这样「驱动一批」正好对应「流一步」，
/// 步边界是可数的——否则一次 `fetch_stream` 的首批就会把几十次 `fetch` 吸收掉，
/// 用例断言不到分批行为。
fn stream_plan(rows: usize) -> impl FnOnce(&mut common::Plan) {
    move |plan: &mut common::Plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = (0..rows)
            .map(|i| vec![DmValue::Int(i as i32)])
            .collect::<Vec<_>>();
        plan.batch_rows = 64;
    }
}

#[tokio::test]
async fn stream_yields_all_rows() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(stream_plan(200));

    let conn = fx.conn().await;
    let mut stream = conn.fetch_stream("select ID from T", ()).await.unwrap();

    assert_eq!(stream.columns().len(), 1, "首批就把列元数据定下来了");
    let rows = drain(&mut stream).await.unwrap();
    assert_eq!(rows.len(), 200);
    let first: i64 = rows[0].get("ID").unwrap();
    assert_eq!(first, 0);
    let last: i64 = rows[199].get("ID").unwrap();
    assert_eq!(last, 199);
    drop(stream);

    // 分批：200 行 / 每批 64 行 ⇒ 至少 3 次取数，绝不是「一次全取」。
    assert!(
        fx.shared
            .fetch_calls
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 3,
        "必须分批取数"
    );

    fx.shutdown().await;
}

#[tokio::test]
async fn stream_dropped_midway_marks_connection_dirty() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(stream_plan(1000));

    let conn = fx.conn().await;
    let mut stream = conn.fetch_stream("select ID from T", ()).await.unwrap();
    // 只取一行就把流丢掉：游标没取尽。
    let _ = next_row(&mut stream).await.unwrap().unwrap();
    drop(stream);

    // 连接回池时应当被复位过（池发现它是脏的）。
    let mut conn = fx.conn().await;
    let got = conn.query_one("select ID from T", ()).await.unwrap();
    assert!(got.is_some());
    drop(conn);

    wait_until("半途丢弃的流让连接被复位", || {
        fx.shared.resets.load(std::sync::atomic::Ordering::Relaxed) >= 1
    })
    .await;

    fx.shutdown().await;
}

#[tokio::test]
async fn stream_error_is_reported_then_finished() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("ID", DataType::Integer)]);
        plan.rows = (0..200).map(|i| vec![DmValue::Int(i)]).collect();
        plan.batch_rows = 64;
        // 首批（第 0 次 fetch）成功，第二批（第 1 次）失败：这样 `fetch_stream` 的
        // 「首批同步等」不会把它拦成 `Err`，错误是从流里吐出来的。
        plan.fail_fetch_on = Some(1);
    });

    let conn = fx.conn().await;
    let mut stream = conn.fetch_stream("select ID from T", ()).await.unwrap();

    // 首批成功，之后某一批失败：错误吐一次，然后流结束。
    let mut errors = 0;
    let mut rows = 0;
    while let Some(item) = next_row(&mut stream).await {
        match item {
            Ok(_) => rows += 1,
            Err(err) => {
                assert!(err.is_connection_lost(), "取数失败按连接丢失处置：{err}");
                errors += 1;
            }
        }
    }
    assert_eq!(errors, 1, "错误只报一次");
    assert!(rows >= 1, "失败之前已经吐出的行不能丢");

    // 流在出错后**仍然持有**连接（`Phase::Ready`），必须显式丢掉才会归还：
    // 这是刻意的——出错不等于连接可以悄悄还回池里（它按连接丢失处置）。
    // 忘了 drop 的话 `shutdown()` 会等 `wait_closed` 到超时并响亮地报错。
    drop(stream);

    fx.shutdown().await;
}

#[tokio::test]
async fn stream_into_conn_returns_a_usable_connection() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(stream_plan(3));

    let conn = fx.conn().await;
    let mut stream = conn.fetch_stream("select ID from T", ()).await.unwrap();
    let _first = next_row(&mut stream).await.unwrap().unwrap();

    let mut conn = stream.into_conn().await;
    assert_eq!(conn.execute("update T set V = 1", ()).await.unwrap(), 0);
    drop(conn);

    fx.shutdown().await;
}

// ---------------------------------------------------------------------------
// 事务与保存点
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transaction_commit_reaches_the_server() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    let mut tx = conn.begin().await.unwrap();
    tx.execute("insert into T (ID) values (1)", ())
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        fx.shared.begins.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        fx.shared.commits.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        fx.shared
            .rollbacks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );

    fx.shutdown().await;
}

#[tokio::test]
async fn transaction_rollback_reaches_the_server() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    let mut tx = conn.begin().await.unwrap();
    tx.execute("insert into T (ID) values (1)", ())
        .await
        .unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(
        fx.shared
            .rollbacks
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    fx.shutdown().await;
}

#[tokio::test]
async fn transaction_dropped_without_commit_is_rolled_back_by_pool() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| plan.affected = Some(1));

    {
        let conn = fx.conn().await;
        let mut tx = conn.begin().await.unwrap();
        tx.execute("insert into T (ID) values (1)", ())
            .await
            .unwrap();
        // 故意不 commit：drop 只能标脏（`Drop` 里不能同步调 ODBC），回滚交给池。
    }

    // 借回来时连接必须已经被复位过（复位的头一步就是回滚）。
    let mut conn = fx.conn().await;
    assert_eq!(conn.execute("update T set V = 1", ()).await.unwrap(), 1);
    drop(conn);

    wait_until("未提交的事务被回滚", || {
        // 池的复位路径**就是**回滚路径（`Backend::reset` 的第一步是服务端回滚），
        // 所以这里等的是 `reset_rollbacks` 而不是 `rollbacks`——后者只统计显式
        // 调用 `Backend::rollback()` 的次数，池根本不会走那条路。
        fx.shared
            .reset_rollbacks
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1
    })
    .await;
    wait_until("事务连接被复位", || {
        fx.shared.resets.load(std::sync::atomic::Ordering::Relaxed) >= 1
    })
    .await;
    assert_eq!(
        fx.pool.stats().in_use,
        0,
        "事务对象丢掉之后，连接要真的回池"
    );

    fx.shutdown().await;
}

#[tokio::test]
async fn transaction_savepoint_and_rollback_to() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    let mut tx = conn.begin().await.unwrap();
    tx.savepoint("sp1").await.unwrap();
    tx.execute("insert into T (ID) values (1)", ())
        .await
        .unwrap();
    tx.rollback_to("sp1").await.unwrap();
    tx.commit().await.unwrap();

    let log = fx.shared.op_log();
    assert!(
        log.contains(&"execute#SAVEPOINT sp1".to_owned()),
        "保存点 SQL 不对：{log:?}"
    );
    assert!(
        log.contains(&"execute#ROLLBACK TO SAVEPOINT sp1".to_owned()),
        "回滚到保存点的 SQL 不对：{log:?}"
    );

    fx.shutdown().await;
}

#[tokio::test]
async fn transaction_release_savepoint_reaches_the_server() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    let mut tx = conn.begin().await.unwrap();
    tx.savepoint("sp1").await.unwrap();
    tx.release_savepoint("sp1").await.unwrap();
    tx.commit().await.unwrap();

    let log = fx.shared.op_log();
    assert!(
        log.contains(&"execute#RELEASE SAVEPOINT sp1".to_owned()),
        "释放保存点的 SQL 不对：{log:?}"
    );
    // 顺序：先建立、后释放，中间不能有回滚。
    let idx_save = log
        .iter()
        .position(|op| op == "execute#SAVEPOINT sp1")
        .unwrap();
    let idx_release = log
        .iter()
        .position(|op| op == "execute#RELEASE SAVEPOINT sp1")
        .unwrap();
    assert!(idx_save < idx_release, "先建立再释放：{log:?}");

    fx.shutdown().await;
}

#[tokio::test]
async fn release_savepoint_rejects_an_invalid_name() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    let mut tx = conn.begin().await.unwrap();
    let err = tx.release_savepoint("sp1; drop table T").await.unwrap_err();

    // 与 `savepoint` 同一条白名单：名字要拼进 SQL，必须在拼之前挡住。
    assert!(
        matches!(err.kind, DmErrorKind::Config(_)),
        "期望 Config，实得 {:?}",
        err.kind
    );
    assert!(
        !fx.shared
            .op_log()
            .iter()
            .any(|op| op.contains("drop table")),
        "非法名字绝不能拼进 SQL"
    );
    tx.rollback().await.unwrap();

    fx.shutdown().await;
}

#[tokio::test]
async fn release_savepoint_translates_a_syntax_rejection_into_unsupported() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    // 达梦 DM8 实测会对 `RELEASE SAVEPOINT` 回语法错（HY000/52000/-2007）。夹具用
    // 「下一次执行报语法错」复现这个**服务端行为**，断言这一层把它翻成明确的
    // `Unsupported`，而不是把 -2007 原样抛给调用方。
    //
    // 注入必须在 `begin()` **之后**：`begin()` 自己会先下发一条
    // `SET TRANSACTION ISOLATION LEVEL …`，否则被喂给注入的是那条语句，
    // 用例测的就不是 `release_savepoint` 了。
    let mut tx = conn.begin().await.unwrap();
    fx.shared
        .configure(|plan| plan.fail_execute_plain_times = 1);

    let err = tx.release_savepoint("sp1").await.unwrap_err();

    assert!(
        matches!(err.kind, DmErrorKind::Unsupported { .. }),
        "期望 Unsupported，实得 {:?}",
        err.kind
    );
    // 证据（原始语法错诊断）必须留着——否则「为什么不支持」只能靠猜。
    assert!(
        err.diags.iter().any(|d| d.message.contains("语法")),
        "原始语法错诊断要保留：{:?}",
        err.diags
    );
    assert!(
        err.diags
            .iter()
            .any(|d| d.message.contains("RELEASE SAVEPOINT")),
        "诊断里要点出是这条语句：{:?}",
        err.diags
    );
    // 失败的是**语句**，不是事务：事务仍然能提交（DM 的语法错是语句级）。
    tx.commit().await.unwrap();

    fx.shutdown().await;
}

#[tokio::test]
async fn savepoint_rejects_an_invalid_name() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    let mut tx = conn.begin().await.unwrap();
    let err = tx.savepoint("sp1; drop table T").await.unwrap_err();

    // 名字要被白名单挡在拼 SQL 之前——这是注入面。
    assert!(
        matches!(err.kind, DmErrorKind::Config(_)),
        "期望 Config，实得 {:?}",
        err.kind
    );
    assert!(
        !fx.shared
            .op_log()
            .iter()
            .any(|op| op.contains("drop table")),
        "非法名字绝不能拼进 SQL"
    );
    tx.rollback().await.unwrap();

    fx.shutdown().await;
}

#[tokio::test]
async fn begin_with_sets_the_isolation_level() {
    let fx = Fixture::open(2).await;

    let conn = fx.conn().await;
    let mut tx = conn.begin_with(IsolationLevel::Serializable).await.unwrap();
    tx.execute("update T set V = 1", ()).await.unwrap();
    tx.commit().await.unwrap();

    let log = fx.shared.op_log();
    assert!(
        log.contains(&"execute#SET TRANSACTION ISOLATION LEVEL SERIALIZABLE".to_owned()),
        "隔离级别 SQL 不对：{log:?}"
    );
    // 隔离级别必须在 `BEGIN` 之后、业务语句之前下发。
    let idx_set = log
        .iter()
        .position(|op| op.starts_with("execute#SET TRANSACTION"))
        .unwrap();
    let idx_begin = log.iter().position(|op| op == "begin").unwrap();
    assert!(idx_begin < idx_set, "先 BEGIN 再设隔离级别：{log:?}");

    fx.shutdown().await;
}

#[tokio::test]
async fn begin_with_rolls_back_when_the_isolation_level_is_rejected() {
    let fx = Fixture::open(2).await;
    // 让 `SET TRANSACTION …` 这一句失败（它是 `begin_with` 里的第一次执行）。
    fx.shared.configure(|plan| plan.fail_execute_times = 1);

    let conn = fx.conn().await;
    let err = conn
        .begin_with(IsolationLevel::Serializable)
        .await
        .unwrap_err();
    assert!(err.is_connection_lost() || matches!(err.kind, DmErrorKind::Io));

    // 不留下半个事务。
    assert!(
        fx.shared
            .rollbacks
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1,
        "隔离级别没设上就必须回滚"
    );

    fx.shutdown().await;
}

// ---------------------------------------------------------------------------
// 与池的边界
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_dropped_connection_returns_to_the_pool() {
    let fx = Fixture::open(1).await;

    for _ in 0..3 {
        let mut conn = fx.conn().await;
        conn.query_one("select ID from T", ()).await.unwrap();
    }
    assert_eq!(
        fx.shared
            .connects
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(fx.pool.stats().in_use, 0);
    assert!(fx.pool.stats().waiting == 0);

    fx.shutdown().await;
}

#[tokio::test]
async fn query_failure_does_not_poison_a_healthy_connection() {
    let fx = Fixture::open(2).await;
    // 一次**普通**失败（语法错，不是断连）：连接应当复位后继续复用。
    fx.shared
        .configure(|plan| plan.fail_execute_plain_times = 1);

    let mut conn = fx.conn().await;
    let err = conn.execute("update T set V = 1", ()).await.unwrap_err();
    assert!(!err.is_connection_lost(), "语法错不该被当成断连：{err}");
    // 同一句话再来一次就成功（假后端的失败次数已经用完）。
    assert_eq!(conn.execute("update T set V = 1", ()).await.unwrap(), 0);
    drop(conn);

    assert_eq!(
        fx.shared.closes.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "普通失败不该把连接销毁"
    );

    fx.shutdown().await;
}
