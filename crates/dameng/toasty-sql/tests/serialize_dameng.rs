//! 达梦方言（`Serializer::dameng`）的 golden 测试 —— 本 fork 新增，不覆盖上游。
//!
//! 覆盖 `toasty-sql/src/serializer/{ty,column_def,ident,dialect}.rs` 里
//! `Flavor::Dameng` 分支的渲染结果：达梦原生类型名、`IDENTITY(1, 1)` 自增、
//! ANSI 双引号标识符、以及「明确不支持」的类型仍在渲染期 panic（而不是静默
//! 生成语义错误的 SQL）。
//!
//! 关于 `Capability`：驱动声明的方言**身份**就是 `Dialect::Mysql`
//! （见 `docs/设计方案.md` §9.1.1），所以这里与生产一致地用 `Capability::MYSQL`
//! 走迁移生成；**渲染方言**由 `Serializer::dameng` 独立决定。
//!
//! 期望值用 `expect![[..]]` 内联；首次落地用
//! `$env:UPDATE_EXPECT=1; cargo +stable-x86_64-pc-windows-msvc test -p toasty-sql`
//! 自动填入，之后必须能在不开 `UPDATE_EXPECT` 的情况下通过。

use expect_test::expect;
use toasty_core::{
    driver::Capability,
    schema::{
        db::{
            Column, ColumnId, EnumVariant, IndexId, PrimaryKey, Schema, Table, TableId, Type,
            TypeEnum,
        },
        diff,
    },
    stmt as core_stmt,
};
use toasty_sql::{Serializer, migration::MigrationStatement, stmt::Statement as SqlStatement};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_column(table_id: usize, index: usize, name: &str, storage_ty: Type) -> Column {
    make_column_with(table_id, index, name, storage_ty, false)
}

fn make_column_with(
    table_id: usize,
    index: usize,
    name: &str,
    storage_ty: Type,
    auto_increment: bool,
) -> Column {
    Column {
        id: ColumnId {
            table: TableId(table_id),
            index,
        },
        name: name.to_string(),
        ty: core_stmt::Type::String,
        storage_ty,
        nullable: false,
        primary_key: index == 0,
        auto_increment,
        versionable: false,
    }
}

fn make_table(id: usize, name: &str, columns: Vec<Column>) -> Table {
    let pk_columns: Vec<ColumnId> = columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.id)
        .collect();

    Table {
        id: TableId(id),
        name: name.to_string(),
        columns,
        primary_key: PrimaryKey {
            columns: pk_columns,
            index: IndexId {
                table: TableId(id),
                index: 0,
            },
        },
        indices: vec![],
    }
}

fn make_enum_type(variants: &[&str]) -> TypeEnum {
    TypeEnum {
        name: None,
        variants: variants
            .iter()
            .map(|v| EnumVariant {
                name: v.to_string(),
            })
            .collect(),
    }
}

/// 用达梦方言渲染 `from → to` 的迁移语句。
fn render_migration(columns: Vec<Column>) -> Vec<String> {
    render_migration_named("t", columns)
}

/// 同上，但表名可以带模式前缀（`DBTESTA.t_order`）。
fn render_migration_named(table_name: &str, columns: Vec<Column>) -> Vec<String> {
    let from = Schema::default();
    let to = Schema {
        tables: vec![make_table(0, table_name, columns)],
    };

    let hints = diff::RenameHints::new();
    let schema_diff = diff::Schema::from(&from, &to, &hints);
    let stmts = MigrationStatement::from_diff(&schema_diff, &Capability::MYSQL);

    stmts
        .iter()
        .map(|ms| Serializer::dameng(ms.schema()).serialize(ms.statement()))
        .collect()
}

// ---------------------------------------------------------------------------
// 类型渲染：全部列在一个 CREATE TABLE 里，便于一次看全映射
// ---------------------------------------------------------------------------

#[test]
fn dameng_create_table_renders_native_types() {
    expect![[r#"
        CREATE TABLE "T" (
            "ID" BIGINT NOT NULL,
            "FLAG" BIT NOT NULL,
            "CODE" VARCHAR(64) NOT NULL,
            "BODY" VARCHAR(8188) NOT NULL,
            "AMOUNT" DECIMAL(10, 2) NOT NULL,
            "FREE_AMOUNT" NUMBER NOT NULL,
            "DIGEST" VARBINARY(16) NOT NULL,
            "PAYLOAD" BLOB NOT NULL,
            "EXT_ID" VARBINARY(16) NOT NULL,
            "CREATED_AT" TIMESTAMP(6) NOT NULL,
            "BIRTHDAY" DATE NOT NULL,
            "AT_TIME" TIME(6) NOT NULL,
            "AT_DATETIME" DATETIME(6) NOT NULL,
            "DOC" CLOB NOT NULL,
            "TAGS" CLOB NOT NULL,
            "STATE" VARCHAR(255) NOT NULL,
            PRIMARY KEY ("ID")
        );"#]]
    .assert_eq(
        &render_migration(vec![
            make_column(0, 0, "id", Type::Integer(8)),
            make_column(0, 1, "flag", Type::Boolean),
            make_column(0, 2, "code", Type::VarChar(64)),
            make_column(0, 3, "body", Type::Text),
            make_column(0, 4, "amount", Type::Numeric(Some((10, 2)))),
            make_column(0, 5, "free_amount", Type::Numeric(None)),
            make_column(0, 6, "digest", Type::Binary(16)),
            make_column(0, 7, "payload", Type::Blob),
            make_column(0, 8, "ext_id", Type::Uuid),
            make_column(0, 9, "created_at", Type::Timestamp(6)),
            make_column(0, 10, "birthday", Type::Date),
            make_column(0, 11, "at_time", Type::Time(6)),
            make_column(0, 12, "at_datetime", Type::DateTime(6)),
            make_column(0, 13, "doc", Type::Json),
            make_column(0, 14, "tags", Type::List(Box::new(Type::Text))),
            make_column(
                0,
                15,
                "state",
                Type::Enum(make_enum_type(&["draft", "published"])),
            ),
        ])
        .join("\n"),
    );
}

#[test]
fn dameng_auto_increment_renders_identity() {
    expect![[r#"
        CREATE TABLE "T" (
            "ID" BIGINT NOT NULL IDENTITY(1, 1),
            "NAME" VARCHAR(8188) NOT NULL,
            PRIMARY KEY ("ID")
        );"#]]
    .assert_eq(
        &render_migration(vec![
            make_column_with(0, 0, "id", Type::Integer(8), true),
            make_column(0, 1, "name", Type::Text),
        ])
        .join("\n"),
    );
}

// ---------------------------------------------------------------------------
// DDL 语句形态
// ---------------------------------------------------------------------------

#[test]
fn dameng_drop_table_uses_ansi_identifiers() {
    let table = make_table(0, "users", vec![make_column(0, 0, "id", Type::Integer(8))]);
    let schema = Schema {
        tables: vec![table.clone()],
    };

    let sql = Serializer::dameng(&schema).serialize(&SqlStatement::drop_table(&table));

    expect![[r#"DROP TABLE "USERS";"#]].assert_eq(&sql);
}

// ---------------------------------------------------------------------------
// 表名带模式前缀（Round 36）
// ---------------------------------------------------------------------------
//
// 模型侧写法 `#[table = "DBTESTA.T_ORDER"]` 会把一整串塞进 `db::Table::name`。
// 达梦里 `"DBTESTA.T_ORDER"` 是一个**名字里带点**的表（不存在，`-2106`），必须渲染成
// 两个标识符。拆分规则：
//
// - **模式名原样**：达梦的模式是大小写敏感对象——`SET SCHEMA dbtest_lower` 会被折成
//   `DBTEST_LOWER` 而报 `-2103 无效的模式名`，只有 `"dbtest_lower"` 才对。所以模式段
//   不能跟表名一起折大写。
// - **表名折大写**：与不带前缀时的既有行为一致（用户手写 `RawSql` 里的表名不带引号，
//   达梦折大写，见 `serializer/ident.rs` 的说明）。
// - **只拆一段**：`a.b.c` → `"a"."B.C"`（达梦没有三级对象名）；某一侧为空则退化成
//   单个标识符，不生成 `""."T"` 这种非法语句。

#[test]
fn dameng_qualified_table_name_renders_two_identifiers() {
    let sql = render_migration_named(
        "DBTESTA.t_order",
        vec![
            make_column(0, 0, "id", Type::Integer(8)),
            make_column(0, 1, "title", Type::Text),
        ],
    );

    expect![[r#"
        CREATE TABLE DBTESTA.T_ORDER (
            "ID" BIGINT NOT NULL,
            "TITLE" VARCHAR(8188) NOT NULL,
            PRIMARY KEY ("ID")
        );"#]]
    .assert_eq(&sql.join("\n"));
}

#[test]
fn dameng_qualified_table_name_folds_both_parts() {
    // 小写输入（用户手写 `dbtesta.t_order` 的习惯）也折成大写、不带引号。
    let sql = render_migration_named(
        "dbtest_lower.t",
        vec![make_column(0, 0, "id", Type::Integer(8))],
    );

    expect![[r#"
        CREATE TABLE DBTEST_LOWER.T (
            "ID" BIGINT NOT NULL,
            PRIMARY KEY ("ID")
        );"#]]
    .assert_eq(&sql.join("\n"));
}

#[test]
fn dameng_qualified_table_name_quotes_only_when_needed() {
    // 保留字（`ORDER`）与带空格的模式名必须回到加引号的写法。
    let sql = render_migration_named(
        "my schema.order",
        vec![make_column(0, 0, "id", Type::Integer(8))],
    );

    expect![[r#"
        CREATE TABLE "MY SCHEMA"."ORDER" (
            "ID" BIGINT NOT NULL,
            PRIMARY KEY ("ID")
        );"#]]
    .assert_eq(&sql.join("\n"));
}

#[test]
fn dameng_qualified_table_name_in_dml() {
    let table = make_table(
        0,
        "DBTESTA.users",
        vec![
            make_column_with(0, 0, "id", Type::Integer(8), true),
            make_column(0, 1, "name", Type::Text),
        ],
    );
    let schema = Schema {
        tables: vec![table.clone()],
    };

    let drop = Serializer::dameng(&schema).serialize(&SqlStatement::drop_table(&table));
    expect![[r#"DROP TABLE DBTESTA.USERS;"#]].assert_eq(&drop);

    let insert = insert_into(
        &schema,
        &[0, 1],
        vec![vec![core_stmt::Expr::Default, core_stmt::Expr::from("a")]],
    );
    expect![[r#"INSERT INTO DBTESTA.USERS ("NAME") VALUES ('a');"#]].assert_eq(&insert);
}

#[test]
fn dameng_table_name_edge_cases() {
    use toasty_sql::dameng_table_name;

    // 不带前缀：与改动前一致（整串折大写 + 双引号）。
    assert_eq!(dameng_table_name("users"), r#""USERS""#);
    // 带前缀：两段都折大写，能裸写就裸写（用户手写 `S.T` 的形态）。
    assert_eq!(dameng_table_name("DBTESTA.users"), "DBTESTA.USERS");
    assert_eq!(
        dameng_table_name("dbtest_lower.users"),
        "DBTEST_LOWER.USERS"
    );
    assert_eq!(dameng_table_name("  db . t1  "), "DB.T1");
    // 只拆一段：`.` 右边的点留在表名段里，于是整段必须加引号。
    assert_eq!(dameng_table_name("a.b.c"), r#"A."B.C""#);
    // 某一侧为空 ⇒ 退化成单个标识符（不生成 `""."T"`）。
    assert_eq!(dameng_table_name("DBTESTA."), r#""DBTESTA.""#);
    assert_eq!(dameng_table_name(".t"), r#"".T""#);
    // 保留字、数字开头、空格 ⇒ 该加引号时就加引号。
    assert_eq!(dameng_table_name("db.order"), r#"DB."ORDER""#);
    assert_eq!(dameng_table_name("order.items"), r#""ORDER".ITEMS"#);
    assert_eq!(dameng_table_name("db.1t"), r#"DB."1T""#);
    assert_eq!(dameng_table_name("my schema.t"), r#""MY SCHEMA".T"#);
    // `_` / `$` / `#` 开头的裸写是合法的，不额外加引号。
    assert_eq!(dameng_table_name("db.t_1"), "DB.T_1");
    assert_eq!(dameng_table_name("_db.t"), "_DB.T");
    // 前后空白去掉；名字里的双引号按 SQL 规矩双写。
    assert_eq!(dameng_table_name("  orders  "), r#""ORDERS""#);
    assert_eq!(dameng_table_name(r#"we"ird"#), r#""WE""IRD""#);
    assert_eq!(dameng_table_name(r#"s.a"b"#), r#"S."A""B""#);
}

// ---------------------------------------------------------------------------
// 明确不支持的渲染必须 panic（可追踪的边界，而不是静默错 SQL）
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Only PostgreSQL supports CIDR")]
fn dameng_cidr_is_rejected_at_render_time() {
    render_migration(vec![
        make_column(0, 0, "id", Type::Integer(8)),
        make_column(0, 1, "net", Type::Cidr),
    ]);
}

#[test]
#[should_panic(expected = "Only PostgreSQL supports INET")]
fn dameng_inet_is_rejected_at_render_time() {
    render_migration(vec![
        make_column(0, 0, "id", Type::Integer(8)),
        make_column(0, 1, "net", Type::Inet),
    ]);
}

// ---------------------------------------------------------------------------
// INSERT 的自增列改写
// ---------------------------------------------------------------------------
//
// 真机实测（`crates/toasty-driver-dameng/tests/insert_probe.rs`）：达梦拒绝
// `("id") VALUES (DEFAULT)`（`-2871`）与 `("id") VALUES (NULL)`（`-2723`），
// 但接受「整列省略」，也接受只有自增列时的 `DEFAULT VALUES`；非自增列上的
// `DEFAULT` 是合法的。这里锁住 fork 的达梦分支对应的三种渲染结果。

fn insert_into(schema: &Schema, columns: &[usize], rows: Vec<Vec<core_stmt::Expr>>) -> String {
    let target = core_stmt::InsertTarget::Table(core_stmt::InsertTable {
        table: TableId(0),
        columns: columns
            .iter()
            .map(|index| ColumnId {
                table: TableId(0),
                index: *index,
            })
            .collect(),
    });
    let rows = rows.into_iter().map(core_stmt::Expr::record).collect();
    let source = core_stmt::Query::values(core_stmt::Values::new(rows));

    let insert = core_stmt::Insert {
        target,
        source,
        upsert: None,
        returning: None,
    };
    let stmt = SqlStatement::from(core_stmt::Statement::from(insert));

    Serializer::dameng(schema).serialize(&stmt)
}

fn auto_increment_schema() -> Schema {
    Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![
                make_column_with(0, 0, "id", Type::Integer(8), true),
                make_column(0, 1, "name", Type::Text),
            ],
        )],
    }
}

#[test]
fn dameng_insert_drops_identity_column_that_gets_default() {
    let sql = insert_into(
        &auto_increment_schema(),
        &[0, 1],
        vec![vec![core_stmt::Expr::Default, core_stmt::Expr::from("a")]],
    );

    expect![[r#"INSERT INTO "USERS" ("NAME") VALUES ('a');"#]].assert_eq(&sql);
}

#[test]
fn dameng_insert_drops_identity_column_across_multiple_rows() {
    let sql = insert_into(
        &auto_increment_schema(),
        &[0, 1],
        vec![
            vec![core_stmt::Expr::Default, core_stmt::Expr::from("a")],
            vec![core_stmt::Expr::Default, core_stmt::Expr::from("b")],
        ],
    );

    expect![[r#"INSERT INTO "USERS" ("NAME") VALUES ('a'), ('b');"#]].assert_eq(&sql);
}

#[test]
fn dameng_insert_all_identity_columns_becomes_default_values() {
    let schema = Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![make_column_with(0, 0, "id", Type::Integer(8), true)],
        )],
    };

    let sql = insert_into(&schema, &[0], vec![vec![core_stmt::Expr::Default]]);

    expect![[r#"INSERT INTO "USERS" DEFAULT VALUES;"#]].assert_eq(&sql);
}

#[test]
fn dameng_insert_keeps_default_on_non_identity_column() {
    let schema = Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![
                make_column_with(0, 0, "id", Type::Integer(8), true),
                make_column(0, 1, "name", Type::Text),
            ],
        )],
    };

    // id 显式给值、name 走列默认值 ⇒ 没有任何列可删，走通用路径（`DEFAULT` 合法）。
    let sql = insert_into(
        &schema,
        &[0, 1],
        vec![vec![core_stmt::Expr::from(1i64), core_stmt::Expr::Default]],
    );

    expect![[r#"INSERT INTO "USERS" ("ID", "NAME") VALUES (1, DEFAULT);"#]].assert_eq(&sql);
}

// ---------------------------------------------------------------------------
// WHERE 子句里的裸绑定参数
// ---------------------------------------------------------------------------
//
// 真机实测（`crates/toasty-driver-dameng/tests/filter_probe.rs`）：达梦拒绝
// `WHERE ?`（`-4104 查询使用值表达式作为过滤条件`），`WHERE ? = TRUE` 也不行
// （`-6105` 类型不匹配），但 `WHERE ? = 1` 可用。引擎对「无过滤」的全表查询正是
// 发一个布尔参数，所以达梦分支必须补这个比较。

fn render_delete_with(filter: core_stmt::Filter) -> String {
    let delete = core_stmt::Delete {
        from: core_stmt::Source::from(TableId(0)),
        filter,
        returning: None,
        condition: core_stmt::Condition::default(),
    };
    let stmt = SqlStatement::from(core_stmt::Statement::from(delete));

    Serializer::dameng(&auto_increment_schema()).serialize(&stmt)
}

#[test]
fn dameng_bare_parameter_filter_compares_to_one() {
    let sql = render_delete_with(core_stmt::Filter::new(core_stmt::Expr::arg(0)));

    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE (?) = 1;"#]].assert_eq(&sql);
}

#[test]
fn dameng_regular_filter_is_left_alone() {
    let sql = render_delete_with(core_stmt::Filter::new(core_stmt::Expr::eq(
        core_stmt::Expr::arg(0),
        core_stmt::Expr::arg(1),
    )));

    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE ? = ?;"#]].assert_eq(&sql);
}

// ---------------------------------------------------------------------------
// upsert 的 `MERGE INTO` 渲染
// ---------------------------------------------------------------------------
//
// 真机事实（`crates/toasty-driver-dameng/tests/engine_probe.rs` 第 15/16 节）：
// ① 达梦没有 `DO NOTHING`，`WHEN MATCHED` 分支必须给点东西（空更新占位）；而空更新在
//    命中行时同样报「影响 1 行」，驱动就没法把「被忽略」与「真插进去」分开 ⇒ 占位更新
//    必须带一个**恒假条件** `WHERE 1 = 0`（探针 16n），命中行时受影响行数就变成 0（探针 16m）；
// ② 但**不能**拿 `ON` 关联列来空更新 —— `[HY000/-4064] 不能更新关联条件中的列`（探针 16e），
//    所以占位符必须挑一个**不在冲突目标里**的列；
// ③ 只有键列的表没有这种列，此时干脆不写 `WHEN MATCHED` 子句（探针 16b/16c/16h/16j）：
//    `MERGE … ON (…) WHEN NOT MATCHED THEN INSERT …` 在达梦上就是「命中就什么都不做」
//    （命中行时影响 **0 行**，行保持原样）；
// ④ 空更新的右值必须带目标别名：`"NAME" = t."NAME"` 可以（探针 16d），
//    `"NAME" = "NAME"` 会被判 `[HY000/-2112]`（`USING` 子查询里有同名列 ⇒ 解析不明确，探针 16f）。
// 这四条一旦被改坏，规划期看不出来（能力位为真），只有真机才会报错 ⇒ 用 golden 钉住。

fn plain_users_schema() -> Schema {
    Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![
                make_column(0, 0, "id", Type::Integer(4)),
                make_column(0, 1, "name", Type::Text),
            ],
        )],
    }
}

fn key_only_schema() -> Schema {
    Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![make_column(0, 0, "id", Type::Integer(4))],
        )],
    }
}

fn upsert_into(
    schema: &Schema,
    columns: &[usize],
    row: Vec<core_stmt::Expr>,
    target_columns: &[usize],
    action: core_stmt::UpsertAction,
) -> String {
    upsert_into_with_shared(
        schema,
        columns,
        row,
        target_columns,
        action,
        core_stmt::Assignments::default(),
    )
}

fn upsert_into_with_shared(
    schema: &Schema,
    columns: &[usize],
    row: Vec<core_stmt::Expr>,
    target_columns: &[usize],
    action: core_stmt::UpsertAction,
    shared: core_stmt::Assignments,
) -> String {
    let column_id = |index: usize| ColumnId {
        table: TableId(0),
        index,
    };
    let target = core_stmt::InsertTarget::Table(core_stmt::InsertTable {
        table: TableId(0),
        columns: columns.iter().copied().map(column_id).collect(),
    });
    let source =
        core_stmt::Query::values(core_stmt::Values::new(vec![core_stmt::Expr::record(row)]));

    let insert = core_stmt::Insert {
        target,
        source,
        upsert: Some(Box::new(core_stmt::Upsert {
            target: core_stmt::UpsertTarget::Columns(
                target_columns.iter().copied().map(column_id).collect(),
            ),
            shared,
            defaults: core_stmt::Assignments::default(),
            update_defaults: core_stmt::Assignments::default(),
            create: core_stmt::Assignments::default(),
            update: core_stmt::Assignments::default(),
            action,
        })),
        returning: None,
    };
    let stmt = SqlStatement::from(core_stmt::Statement::from(insert));

    Serializer::dameng(schema).serialize(&stmt)
}

/// 主键冲突 + `Ignore`：空更新落在**非键列** `"NAME"` 上（不是 `"ID"`）。
#[test]
fn dameng_upsert_ignore_takes_a_non_key_column_for_the_no_op_update() {
    let sql = upsert_into(
        &plain_users_schema(),
        &[0, 1],
        vec![core_stmt::Expr::from(1i64), core_stmt::Expr::from("a")],
        &[0],
        core_stmt::UpsertAction::Ignore,
    );

    expect![[
        r#"MERGE INTO "USERS" AS t USING (SELECT 1 AS "s_0", 'a' AS "s_1" FROM DUAL) AS s ON (t."ID" = s."s_0") WHEN MATCHED THEN UPDATE SET "NAME" = t."NAME" WHERE 1 = 0 WHEN NOT MATCHED THEN INSERT ("ID", "NAME") VALUES (s."s_0", s."s_1");"#
    ]]
    .assert_eq(&sql);
}

/// 冲突目标是唯一键（不是第一列）时，空更新要跳过**所有**目标列。
#[test]
fn dameng_upsert_ignore_skips_every_target_column() {
    let schema = Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![
                make_column(0, 0, "id", Type::Integer(4)),
                make_column(0, 1, "email", Type::VarChar(64)),
                make_column(0, 2, "name", Type::Text),
            ],
        )],
    };

    let sql = upsert_into(
        &schema,
        &[0, 1, 2],
        vec![
            core_stmt::Expr::from(1i64),
            core_stmt::Expr::from("a@b"),
            core_stmt::Expr::from("a"),
        ],
        &[1],
        core_stmt::UpsertAction::Ignore,
    );

    expect![[
        r#"MERGE INTO "USERS" AS t USING (SELECT 1 AS "s_0", 'a@b' AS "s_1", 'a' AS "s_2" FROM DUAL) AS s ON (t."EMAIL" = s."s_1") WHEN MATCHED THEN UPDATE SET "ID" = t."ID" WHERE 1 = 0 WHEN NOT MATCHED THEN INSERT ("ID", "EMAIL", "NAME") VALUES (s."s_0", s."s_1", s."s_2");"#
    ]]
    .assert_eq(&sql);
}

/// 只有键列：没有可用的非键占位符 ⇒ 整个 `WHEN MATCHED` 子句都不写。
#[test]
fn dameng_upsert_ignore_on_a_key_only_table_omits_the_matched_clause() {
    let sql = upsert_into(
        &key_only_schema(),
        &[0],
        vec![core_stmt::Expr::from(1i64)],
        &[0],
        core_stmt::UpsertAction::Ignore,
    );

    expect![[
        r#"MERGE INTO "USERS" AS t USING (SELECT 1 AS "s_0" FROM DUAL) AS s ON (t."ID" = s."s_0") WHEN NOT MATCHED THEN INSERT ("ID") VALUES (s."s_0");"#
    ]]
    .assert_eq(&sql);
}

/// Round 28：`upsert.update(...)` 里引用「本次要写的新值」（引擎的 `incoming()`）时，
/// 达梦没有 PostgreSQL 的 `excluded` —— 只能落到 `USING` 源列上，而 ordinal 是
/// **该列在插入列列表里的位置**（不是列在表里的下标）。
///
/// 真机依据（`engine_probe.rs` 第 31 节）：`WHEN MATCHED THEN UPDATE SET "name" = s."s_2"`
/// 可用；套件 `crud_upsert::upsert_update_can_reference_incoming_value` 走的正是这条。
#[test]
fn dameng_upsert_incoming_value_refers_to_the_merge_source_column() {
    let schema = Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![
                make_column(0, 0, "id", Type::Integer(4)),
                make_column(0, 1, "email", Type::VarChar(64)),
                make_column(0, 2, "name", Type::Text),
            ],
        )],
    };

    let mut shared = core_stmt::Assignments::default();
    shared.set(
        2usize,
        core_stmt::Expr::project(
            core_stmt::Expr::Incoming(core_stmt::ExprIncoming::Table(TableId(0))),
            2usize,
        ),
    );

    let sql = upsert_into_with_shared(
        &schema,
        // 只插 `(email, name)`：源列 ordinal 与表内列下标**不同**，正是要钉住的东西
        // （`email` 在表里是 1、在插入列列表里是 0）。
        &[1, 2],
        vec![core_stmt::Expr::from("a@b"), core_stmt::Expr::from("a")],
        &[1],
        core_stmt::UpsertAction::Update,
        shared,
    );

    expect![[
        r#"MERGE INTO "USERS" AS t USING (SELECT 'a@b' AS "s_0", 'a' AS "s_1" FROM DUAL) AS s ON (t."EMAIL" = s."s_0") WHEN MATCHED THEN UPDATE SET "NAME" = s."s_1" WHEN NOT MATCHED THEN INSERT ("EMAIL", "NAME") VALUES (s."s_0", s."s_1");"#
    ]]
    .assert_eq(&sql);
}

// ---------------------------------------------------------------------------
// 集合谓词（`Vec<scalar>`）的渲染
// ---------------------------------------------------------------------------
//
// 真机事实（`crates/toasty-driver-dameng/tests/engine_probe.rs` 第 17 节，T8.12 侦察）：
// ① 达梦**没有** `MEMBER OF` / `SUBMULTISET` / 原生数组列 / `COLLATE`（探针 17k–17n 全是语法错误），
//    也**没有**可用的 `JSON_EXISTS` 过滤路径（17e/17o/17p/17aa/17ac 都是 `-2207`）；
// ② 但达梦有 MySQL 那一族的 JSON 判定函数，而且**能吃绑定参数**：
//    `JSON_CONTAINS(列, ?)`（17ak）、`JSON_OVERLAPS(列, ?)`（17al）、
//    `JSON_CONTAINS(列, JSON_ARRAY(?))`（17am）、`JSON_LENGTH(列)`（17r）；
// ③ 语义与 MySQL 对齐：17ap 证明 `JSON_CONTAINS` 是「candidate 的每个元素都在 target 里」
//    （= 超集），17ao 证明数字元素也按 JSON 类型比较；
// ④ 大小写**不**受服务端默认排序规则影响：17h/17i 的 `LIKE` 与 17an 的
//    `JSON_CONTAINS(…, JSON_ARRAY('ADMIN'))` 都只命中精确大小写的那一行 ——
//    官方套件 `vec_string_contains_filter` 明确要求这一点；
// ⑤ `CLOB`（fork 给 `db::Type::List` 选定的存储类型，见 `ty.rs`）与 `VARCHAR(8188)` 上
//    JSON 函数都能用（列建表 17a、17z，函数 17ar）。
// 这五条一旦被改坏，规划期看不出来（能力位为真），只有真机才会报错 ⇒ 用 golden 钉住形状。

fn render_delete_where(expr: core_stmt::Expr) -> String {
    render_delete_with(core_stmt::Filter::new(expr))
}

#[test]
fn dameng_is_superset_uses_json_contains() {
    let expr = core_stmt::Expr::IsSuperset(core_stmt::ExprIsSuperset {
        lhs: Box::new(core_stmt::Expr::arg(0)),
        rhs: Box::new(core_stmt::Expr::arg(1)),
    });

    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE JSON_CONTAINS(?, ?);"#]]
        .assert_eq(&render_delete_where(expr));
}

#[test]
fn dameng_intersects_uses_json_overlaps() {
    let expr = core_stmt::Expr::Intersects(core_stmt::ExprIntersects {
        lhs: Box::new(core_stmt::Expr::arg(0)),
        rhs: Box::new(core_stmt::Expr::arg(1)),
    });

    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE JSON_OVERLAPS(?, ?);"#]]
        .assert_eq(&render_delete_where(expr));
}

#[test]
fn dameng_length_uses_json_length() {
    let expr = core_stmt::Expr::eq(
        core_stmt::Expr::Length(core_stmt::ExprLength {
            expr: Box::new(core_stmt::Expr::arg(0)),
        }),
        core_stmt::Expr::arg(1),
    );

    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE JSON_LENGTH(?) = ?;"#]]
        .assert_eq(&render_delete_where(expr));
}

#[test]
fn dameng_contains_uses_json_contains_with_json_array() {
    let expr = core_stmt::Expr::AnyOp(core_stmt::ExprAnyOp {
        lhs: Box::new(core_stmt::Expr::arg(0)),
        op: core_stmt::BinaryOp::Eq,
        rhs: Box::new(core_stmt::Expr::arg(1)),
    });

    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE JSON_CONTAINS(?, JSON_ARRAY(?));"#]]
        .assert_eq(&render_delete_where(expr));
}

// ---------------------------------------------------------------------------
// 集合变更（`Vec<scalar>` 的原地删除）的渲染
// ---------------------------------------------------------------------------
//
// 真机事实（`crates/toasty-driver-dameng/tests/engine_probe.rs` 第 18/19 节，T8.13）：
// ① `JSON_REMOVE` 可用（探针 18c/18d/18t/18u），而且**越界路径是 no-op**（18d：
//    `JSON_REMOVE('["a","b"]', '$[5]')` 原样返回）⇒ `remove_at` 的越界语义白拿；
// ② 但路径里出现负数会被判 `-3102 JSON 路径表达式语法错误`（18e/18g）⇒ `pop` 在空列表上
//    必须先夹住（`CASE`，另一支直接回原列）；两条都已在真机验证（19k–19o：表列上连做
//    三次 `pop`，`["a","b"]` → `["a"]` → `[]`）；
// ③ **`remove(v)` 只能靠 `MERGE` + 派生表重建**（Round 22/23）：达梦没有 `JSON_ARRAYAGG`
//    （18k/18l/18x/18z5 全是 `-2207`），`JSON_REMOVE` 既不吃「路径数组」（18z4 `-3102`）
//    也不吃过滤路径（18z7），且多个路径是**顺序施加**（18j）⇒ 不能靠 `JSON_SEARCH('all')`；
//    把 `JSON_TABLE` + `LISTAGG` 塞进 `UPDATE … SET` 的相关子查询又被判
//    `[HY000/-4083] 引用列未找到`（19q/19s/19y/19z2/19z4），`JSON_QUERY` 的**过滤路径**更
//    干脆不存在（Round 22 探针 20a–20o 一律 `-3102`）。能用的是 **`MERGE` + `USING` 派生表**：
//    重建在**普通 `SELECT` 上下文**里做（那里 `JSON_TABLE` 可用），`MERGE` 只把算好的列写回。
//    形状由真机钉死（探针 22g/22h：普通与**绑定参数**；22i/22j：**全匹配**边界），三处细节
//    都不是顺手简化：① `LEFT JOIN … ON 1 = 1` 而非逗号连接 —— 逗号连接下「元素全被删掉」
//    的行在派生表里没有分组，`MERGE` 就**不会更新**它（应写 `[]`）；② 过滤写进
//    `CASE WHEN jt."E" <> 值 THEN … END`、靠 `LISTAGG` 跳过 NULL 来丢元素，写成
//    `WHERE jt."E" <> 值` 会把整个空分组滤掉；③ 元素 JSON 文本要自己拼 —— `PATH '$'` 给的是
//    **裸文本**（数字/布尔/定点数在 JSON 里本来就是裸字面量，直接拼），字符串/时间/UUID/
//    网络地址用 `JSON_QUOTE` 补引号（转义正确，探针 20m）。注意 `JSON_TABLE` 的
//    `FORMAT JSON` 列在真机上**恒为 NULL**（21a）⇒ 用它聚合会「执行成功、影响 1 行」却把
//    整列静默清空（19A/20i0/20j/20k），golden 特意钉住 `JSON_QUOTE` 这条正确写法。
// ①②③ 一旦被改坏，规划期看不出来（能力位为真），只有真机才会报错 ⇒ 用 golden 钉住形状。
// ①② 一旦被改坏，规划期看不出来（能力位为真），只有真机才会报错 ⇒ 用 golden 钉住形状。

fn list_tags_schema() -> Schema {
    Schema {
        tables: vec![make_table(
            0,
            "users",
            vec![
                make_column(0, 0, "id", Type::Integer(4)),
                make_column(0, 1, "tags", Type::List(Box::new(Type::Text))),
            ],
        )],
    }
}

fn render_update(schema: &Schema, assignments: core_stmt::Assignments) -> String {
    let update = core_stmt::Update {
        target: core_stmt::UpdateTarget::Table(TableId(0)),
        assignments,
        filter: core_stmt::Filter::ALL,
        condition: core_stmt::Condition::default(),
        returning: None,
    };

    Serializer::dameng(schema).serialize(&SqlStatement::from(core_stmt::Statement::from(update)))
}

#[test]
fn dameng_pop_guards_the_empty_list_in_a_case() {
    let mut assignments = core_stmt::Assignments::default();
    assignments.pop(1usize);

    expect![[
        r#"UPDATE "USERS" AS tbl_0_0 SET "TAGS" = CASE WHEN JSON_LENGTH("TAGS") = 0 THEN "TAGS" ELSE JSON_REMOVE("TAGS", '$[' || (JSON_LENGTH("TAGS") - 1) || ']') END;"#
    ]]
    .assert_eq(&render_update(&list_tags_schema(), assignments));
}

#[test]
fn dameng_remove_at_uses_a_json_path() {
    let mut assignments = core_stmt::Assignments::default();
    assignments.remove_at(1usize, core_stmt::Expr::from(1i64));

    expect![[r#"UPDATE "USERS" AS tbl_0_0 SET "TAGS" = JSON_REMOVE("TAGS", '$[' || 1 || ']');"#]]
        .assert_eq(&render_update(&list_tags_schema(), assignments));
}

#[test]
fn dameng_remove_value_rebuilds_the_list_through_a_merge() {
    let mut assignments = core_stmt::Assignments::default();
    assignments.remove(1usize, core_stmt::Expr::from("dup"));

    expect![[
        r#"MERGE INTO "USERS" AS t USING (SELECT src."ID" AS "PK_0", '[' || NVL(LISTAGG(CASE WHEN jt."E" <> 'dup' THEN JSON_QUOTE(jt."E") END, ',') WITHIN GROUP (ORDER BY jt."N"), '') || ']' AS "V_0" FROM (SELECT "ID", "TAGS" FROM "USERS" AS tbl_0_0) src LEFT JOIN JSON_TABLE(src."TAGS", '$[*]' COLUMNS ("E" VARCHAR(8188) PATH '$', "N" FOR ORDINALITY)) jt ON 1 = 1 GROUP BY src."ID") s ON (t."ID" = s."PK_0") WHEN MATCHED THEN UPDATE SET t."TAGS" = s."V_0";"#
    ]]
    .assert_eq(&render_update(&list_tags_schema(), assignments));
}

// ---------------------------------------------------------------------------
// `#[document]` 的路径读取（`JSON_VALUE`）
// ---------------------------------------------------------------------------
//
// 真机事实（`engine_probe.rs` 第 25/26/27 节，T8.14 落地时定界）：
// ① 达梦的 `JSON_VALUE(列, '$.a.b')` 直接把标量给成 **SQL 文本**（字符串不带引号、
//    数字与布尔按裸字面量），`$.a.b` 与 `$."a"` 两种路径写法都接受
//    （25c/25c2/25c3…25s）；容器叶子要用 `JSON_QUERY`（25n/25o），标量上用它回 `NULL`（25d）；
// ② 与**绑定参数**比较时，各叶子类型要分开处理：整数必须 `CAST(… AS BIGINT)`
//    （25v：裸文本与绑定 `BIGINT` 不隐式转换）、浮点 `CAST(… AS DOUBLE)`（26g）、
//    布尔只能用 `CASE WHEN … = 'true' THEN 1 ELSE 0 END`（26c 命中；`CAST('true' AS
//    BIT/INT)` 报 `[22018/-6111]`，26e/26e2）、UUID 要 `HEXTORAW(REPLACE(…, '-', ''))`
//    （27h 命中；反方向的 `RAWTOHEX(?)` 不行，26i2）；
// ③ 字符串、网络族与 `Decimal`/`BigDecimal` 叶子裸文本就能比（25t/25u；26j/26k/27f
//    实测达梦按数值比较）；
// ④ 时间叶子：存储文本是 ISO 8601（`2024-01-02T03:04:05.123456Z`，`Timestamp` 带
//    `Z`）⇒ 达梦的 `CAST(… AS TIMESTAMP(6))` 报 `[22007/-6118] 非法的时间日期类型数据`
//    （第 29 节探针复现了套件 `struct_embed_filter_temporal` 的失败），要先
//    `REPLACE(REPLACE(文本, 'T', ' '), 'Z', '')` 转成达梦自己的
//    `YYYY-MM-DD HH24:MI:SS.FF6`（29e 命中，29e2 投影仍带微秒；`TO_TIMESTAMP` 那条
//    29f 也行，但 `CAST` 与既有形状更近）；`Date`/`Time` 叶子各是 `YYYY-MM-DD` /
//    `HH:MM:SS.FFFFFF`，直接 `CAST`（29h/29i）。**参数**那侧的 ODBC 精度也必须报 6
//    —— `crates/dm-odbc/src/param.rs` 原先给 `(0, 0)`，达梦把绑定时间当整秒处理
//    （27b2 的 `TO_CHAR(?, …FF6)` 回 `.000000`），第 28 节探针把这条分开量了。
// 这些形状一旦被改坏，规划期看不出来（能力位为真），只有真机才会报错 ⇒ 用 golden 钉住。

fn render_json_extract(path: &[&str], ty: core_stmt::Type) -> String {
    render_delete_where(core_stmt::Expr::eq(
        core_stmt::Expr::from(core_stmt::FuncJsonExtract {
            base: Box::new(core_stmt::Expr::arg(0)),
            path: path.iter().map(|key| (*key).to_string()).collect(),
            ty,
        }),
        core_stmt::Expr::arg(1),
    ))
}

#[test]
fn dameng_document_string_leaf_needs_no_cast() {
    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE JSON_VALUE(?, '$.a.b') = ?;"#]]
        .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::String));
}

#[test]
fn dameng_document_numeric_leaves_are_cast() {
    expect![[
        r#"DELETE FROM "USERS" AS tbl_0_0 WHERE CAST(JSON_VALUE(?, '$.a.b') AS BIGINT) = ?;"#
    ]]
    .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::I64));

    expect![[
        r#"DELETE FROM "USERS" AS tbl_0_0 WHERE CAST(JSON_VALUE(?, '$.a.b') AS DOUBLE) = ?;"#
    ]]
    .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::F64));
}

#[test]
fn dameng_document_bool_leaf_uses_a_case() {
    expect![[
        r#"DELETE FROM "USERS" AS tbl_0_0 WHERE CASE WHEN JSON_VALUE(?, '$.a.b') = 'true' THEN 1 ELSE 0 END = ?;"#
    ]]
    .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::Bool));
}

#[test]
fn dameng_document_uuid_leaf_goes_through_hextoraw() {
    expect![[
        r#"DELETE FROM "USERS" AS tbl_0_0 WHERE HEXTORAW(REPLACE(JSON_VALUE(?, '$.a.b'), '-', '')) = ?;"#
    ]]
    .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::Uuid));
}

#[cfg(feature = "jiff")]
#[test]
fn dameng_document_temporal_leaves_are_cast_with_precision() {
    // `Timestamp`/`DateTime` 的文档存储文本是 ISO 8601（`…T…Z`），达梦的
    // `CAST(… AS TIMESTAMP(6))` 不认（`[22007/-6118]`）⇒ 先换成达梦自己的
    // `YYYY-MM-DD HH24:MI:SS.FF6`（探针第 29 节）。
    expect![[
        r#"DELETE FROM "USERS" AS tbl_0_0 WHERE CAST(REPLACE(REPLACE(JSON_VALUE(?, '$.a.b'), 'T', ' '), 'Z', '') AS TIMESTAMP(6)) = ?;"#
    ]]
    .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::Timestamp));

    expect![[
        r#"DELETE FROM "USERS" AS tbl_0_0 WHERE CAST(REPLACE(REPLACE(JSON_VALUE(?, '$.a.b'), 'T', ' '), 'Z', '') AS TIMESTAMP(6)) = ?;"#
    ]]
    .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::DateTime));

    expect![[
        r#"DELETE FROM "USERS" AS tbl_0_0 WHERE CAST(JSON_VALUE(?, '$.a.b') AS TIME(6)) = ?;"#
    ]]
    .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::Time));

    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE CAST(JSON_VALUE(?, '$.a.b') AS DATE) = ?;"#]]
        .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::Date));
}

#[cfg(feature = "rust_decimal")]
#[test]
fn dameng_document_decimal_leaf_compares_numerically() {
    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE JSON_VALUE(?, '$.a.b') = ?;"#]]
        .assert_eq(&render_json_extract(&["a", "b"], core_stmt::Type::Decimal));
}

#[cfg(feature = "bigdecimal")]
#[test]
fn dameng_document_bigdecimal_leaf_compares_numerically() {
    expect![[r#"DELETE FROM "USERS" AS tbl_0_0 WHERE JSON_VALUE(?, '$.a.b') = ?;"#]].assert_eq(
        &render_json_extract(&["a", "b"], core_stmt::Type::BigDecimal),
    );
}
