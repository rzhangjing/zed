//! `#[derive(FromRow)]`（T5.3）的离线测试组。
//!
//! 覆盖：默认按字段名取列（大小写不敏感）、`#[dm(rename = "…")]`、`Option<T>` 收 `NULL`、
//! 非 `Option` 遇到 `NULL` 报 `NullViolation`、类型不匹配的诊断里带上**字段名**、
//! 以及泛型结构体（`where` 子句是按字段类型生成的）。
//!
//! 编译失败的用例（元组结构体、枚举、`#[dm(skip)]`）刻意**不用 `trybuild`**：
//! 那要再引一个依赖、还要维护一份 `.stderr` 快照，而「报错文本对不对」在
//! `dm-macros/src/lib.rs` 里已经是字面量，改坏了在代码评审里就能看见。

mod common;

use common::{Fixture, columns};
use dm_core::{DataType, DmValue};
use dm_error::DmErrorKind;
use dm_macros::FromRow;

#[derive(Debug, PartialEq, FromRow)]
struct User {
    #[dm(rename = "ID")]
    id: i64,
    name: String,
    opt: Option<i64>,
}

/// 行里所有列都参与映射，且字段名与列名同形（走大小写不敏感匹配）。
#[derive(Debug, PartialEq, FromRow)]
struct SameName {
    id: i64,
    name: String,
}

/// 泛型结构体：`where T: Decode` 是宏生成的。
#[derive(Debug, PartialEq, FromRow)]
struct Wrapper<T> {
    value: T,
}

#[tokio::test]
async fn derive_maps_fields_by_column_name_case_insensitively() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[
            ("ID", DataType::Integer),
            ("NAME", DataType::VarChar),
            ("OPT", DataType::Integer),
        ]);
        plan.rows = vec![vec![
            DmValue::BigInt(7),
            DmValue::Text("张三".to_owned()),
            DmValue::Null,
        ]];
    });

    let mut conn = fx.conn().await;
    let row = conn
        .query_one("select ID, NAME, OPT from T", ())
        .await
        .unwrap()
        .expect("有一行");
    let user: User = row.to().unwrap();

    assert_eq!(
        user,
        User {
            id: 7,
            name: "张三".to_owned(),
            opt: None,
        }
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn derive_uses_the_field_name_when_there_is_no_rename() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("id", DataType::Integer), ("NAME", DataType::VarChar)]);
        plan.rows = vec![vec![DmValue::Int(1), DmValue::Text("甲".to_owned())]];
    });

    let mut conn = fx.conn().await;
    let row = conn
        .query_one("select id, NAME from T", ())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.to::<SameName>().unwrap(),
        SameName {
            id: 1,
            name: "甲".to_owned(),
        }
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn derive_reports_null_violation_with_the_field_name() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("id", DataType::Integer), ("NAME", DataType::VarChar)]);
        plan.rows = vec![vec![DmValue::Int(1), DmValue::Null]];
    });

    let mut conn = fx.conn().await;
    let row = conn
        .query_one("select id, NAME from T", ())
        .await
        .unwrap()
        .unwrap();
    let err = row.to::<SameName>().unwrap_err();

    match &err.kind {
        DmErrorKind::NullViolation { column } => assert_eq!(column, "NAME"),
        other => panic!("期望 NullViolation，实得 {other:?}"),
    }
    // 派生宏补的那条诊断必须点出**结构体字段名**，否则 `rename` 之后没人知道是哪个字段。
    assert!(
        err.diags
            .iter()
            .any(|diag| diag.message.contains("字段 `name`")),
        "诊断里必须出现字段名：{:?}",
        err.diags
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn derive_reports_type_mismatch_with_the_field_name() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("id", DataType::Integer), ("NAME", DataType::VarBinary)]);
        plan.rows = vec![vec![DmValue::Int(1), DmValue::Bytes(vec![1, 2, 3])]];
    });

    let mut conn = fx.conn().await;
    let row = conn
        .query_one("select id, NAME from T", ())
        .await
        .unwrap()
        .unwrap();
    let err = row.to::<SameName>().unwrap_err();

    assert!(
        matches!(err.kind, DmErrorKind::DataTypeMismatch { .. }),
        "期望 DataTypeMismatch，实得 {:?}",
        err.kind
    );
    assert!(
        err.diags
            .iter()
            .any(|diag| diag.message.contains("字段 `name`")),
        "诊断里必须出现字段名：{:?}",
        err.diags
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn derive_missing_column_names_the_column() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        // 结果集里只有一列，而 `SameName` 要两列。
        plan.columns = columns(&[("id", DataType::Integer)]);
        plan.rows = vec![vec![DmValue::Int(1)]];
    });

    let mut conn = fx.conn().await;
    let row = conn
        .query_one("select id from T", ())
        .await
        .unwrap()
        .unwrap();
    let err = row.to::<SameName>().unwrap_err();

    // 报出来的是**我们去找的那个名字**（结构体字段名，`rename` 之后就是 `rename` 的值），
    // 不是「结果集里最像它的那一列」——没有候选列时后者根本不存在。
    match &err.kind {
        DmErrorKind::Decode { column } => assert_eq!(column.as_deref(), Some("name")),
        other => panic!("期望 Decode，实得 {other:?}"),
    }
    // 光说「找不到 name」不够：得把**现有列**摆出来，否则用户还得自己去 describe。
    assert!(
        err.diags
            .iter()
            .any(|diag| diag.message.contains("现有".to_owned().as_str())
                || diag.message.contains("当前结果集的列")),
        "诊断里必须列出当前结果集有哪些列：{:?}",
        err.diags
    );
    // 派生宏补的那条诊断要点出结构体字段名（`Row::get` 只知道列名）。
    assert!(
        err.diags
            .iter()
            .any(|diag| diag.message.contains("字段 `name`")),
        "诊断里必须出现字段名：{:?}",
        err.diags
    );

    drop(conn);
    fx.shutdown().await;
}

#[tokio::test]
async fn derive_works_on_a_generic_struct() {
    let fx = Fixture::open(2).await;
    fx.shared.configure(|plan| {
        plan.columns = columns(&[("value", DataType::VarChar)]);
        plan.rows = vec![vec![DmValue::Text("泛型".to_owned())]];
    });

    let mut conn = fx.conn().await;
    let row = conn
        .query_one("select value from T", ())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.to::<Wrapper<String>>().unwrap(),
        Wrapper {
            value: "泛型".to_owned(),
        }
    );

    drop(conn);
    fx.shutdown().await;
}
