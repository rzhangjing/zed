use super::{ColumnAlias, Comma, Delimited, Formatter, Ident, TableIdent, ToSql};

use crate::{
    serializer::Flavor,
    stmt::{self, AlterColumnChanges, ColumnDef},
};
use toasty_core::{schema::db, stmt::SourceTableId};

struct ColumnsWithConstraints<'a>(&'a stmt::CreateTable);

impl ToSql for ColumnsWithConstraints<'_> {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        // SQLite needs the PK specified with the auto increment
        let trailing_pk = if f.serializer.is_sqlite() {
            // Sqlite only supports auto incrementing columns if they are the only primary key.
            match self.0.columns.iter().filter(|c| c.auto_increment).count() {
                0 => true,
                1 => {
                    // In this case, the primary key **must** be the auto incrementing column
                    let Some(pk) = self.0.primary_key.as_deref().and_then(|pk| pk.as_record())
                    else {
                        todo!("Toasty should catch this earlier")
                    };

                    let [stmt::Expr::Reference(pk)] = &pk.fields[..] else {
                        todo!("Toasty should catch this earlier")
                    };

                    let pk = pk.as_expr_column_unwrap();

                    assert_eq!(0, pk.nesting);
                    assert!(
                        self.0.columns[pk.column].auto_increment,
                        "Toasty should catch this earlier"
                    );

                    false
                }
                _ => panic!("Toasty should catch this case earlier"),
            }
        } else {
            true
        };

        let has_trailing_pk = self.0.primary_key.is_some() && trailing_pk;

        for (index, column) in self.0.columns.iter().enumerate() {
            fmt!(f, "\n    " column);
            if index < self.0.columns.len() - 1 || has_trailing_pk {
                fmt!(f, ",");
            }
        }

        match &self.0.primary_key {
            Some(pk) if trailing_pk => fmt!(f, "\n    PRIMARY KEY " pk "\n"),
            _ => fmt!(f, "\n"),
        }
    }
}

impl ToSql for &stmt::CreateIndex {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let index = f.serializer.index(self.index);
        let table = f.serializer.table(self.on);
        let index_name = Ident(&index.name);
        let table_name = TableIdent(&table.name);
        let columns = Comma(&self.columns);
        let unique = if self.unique { "UNIQUE " } else { "" };

        // Create a new expression scope to serialize the statement
        let mut f = f.scope(table);

        fmt!(
            &mut f, "CREATE " unique "INDEX " index_name " ON " table_name " (" columns ")"
        );
    }
}

impl ToSql for &stmt::AddColumn {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let table = f.serializer.table(self.table);
        let table_name = TableIdent(&table.name);

        // Create new expression scope to serialize the statement
        let mut f = f.scope(table);

        fmt!(
            &mut f, "ALTER TABLE " table_name " ADD COLUMN " self.column
        );
    }
}

impl ToSql for &stmt::AlterColumn {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let table = f.serializer.table(self.id.table);
        let table_name = TableIdent(&table.name);

        // Create new expression scope to serialize the statement
        let mut f = f.scope(table);

        let column_name = Ident(&self.column_def.name);

        match f.serializer.dialect {
            Flavor::Postgresql => match &self.changes {
                AlterColumnChanges {
                    new_name: Some(name),
                    new_ty: None,
                    new_not_null: None,
                    new_auto_increment: None,
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " RENAME COLUMN " column_name " TO " Ident(name.as_str()))
                }
                AlterColumnChanges {
                    new_name: None,
                    new_ty: Some(ty),
                    new_not_null: None,
                    new_auto_increment: None,
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " ALTER COLUMN " column_name " TYPE " ty);

                    if let Some(intermediate_ty) =
                        enum_change_intermediate_ty(&self.column_def.ty, ty)
                    {
                        fmt!(
                            &mut f,
                            " USING " Ident(&self.column_def.name) "::" intermediate_ty "::" ty
                        );
                    }
                }
                AlterColumnChanges {
                    new_name: None,
                    new_ty: None,
                    new_not_null: Some(true),
                    new_auto_increment: None,
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " ALTER COLUMN " column_name " SET NOT NULL")
                }
                AlterColumnChanges {
                    new_name: None,
                    new_ty: None,
                    new_not_null: Some(false),
                    new_auto_increment: None,
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " ALTER COLUMN " column_name " DROP NOT NULL")
                }
                AlterColumnChanges {
                    new_name: None,
                    new_ty: None,
                    new_not_null: None,
                    new_auto_increment: Some(true),
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " ALTER COLUMN " column_name " ADD GENERATED BY DEFAULT AS IDENTITY")
                }
                AlterColumnChanges {
                    new_name: None,
                    new_ty: None,
                    new_not_null: None,
                    new_auto_increment: Some(false),
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " ALTER COLUMN " column_name " DROP IDENTITY")
                }
                _ => panic!(
                    "PostgreSQL does not support modifying multiple column properties in one ALTER TABLE statement"
                ),
            },
            Flavor::Mysql | Flavor::MariaDb => {
                let new_column_def = ColumnDef {
                    name: self
                        .changes
                        .new_name
                        .as_ref()
                        .unwrap_or(&self.column_def.name)
                        .clone(),
                    ty: self
                        .changes
                        .new_ty
                        .as_ref()
                        .unwrap_or(&self.column_def.ty)
                        .clone(),
                    not_null: self
                        .changes
                        .new_not_null
                        .unwrap_or(self.column_def.not_null),
                    auto_increment: self
                        .changes
                        .new_auto_increment
                        .unwrap_or(self.column_def.auto_increment),
                    check: self.column_def.check.clone(),
                };
                fmt!(&mut f, "ALTER TABLE " table_name " CHANGE COLUMN " column_name " " new_column_def)
            }
            Flavor::Sqlite => match &self.changes {
                AlterColumnChanges {
                    new_name: Some(name),
                    new_ty: None,
                    new_not_null: None,
                    new_auto_increment: None,
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " RENAME COLUMN " column_name " TO " Ident(name.as_str()))
                }
                _ => panic!("SQLite only supports renaming columns in ALTER TABLE statement"),
            },
            // 达梦（Oracle 兼容）：改名走 `RENAME COLUMN`，其余属性改动走
            // `MODIFY <完整列定义>`（可以一次改多项，这一点与 PostgreSQL 不同）。
            Flavor::Dameng => match &self.changes {
                AlterColumnChanges {
                    new_name: Some(name),
                    new_ty: None,
                    new_not_null: None,
                    new_auto_increment: None,
                } => {
                    fmt!(&mut f, "ALTER TABLE " table_name " RENAME COLUMN " column_name " TO " Ident(name.as_str()))
                }
                AlterColumnChanges {
                    new_name: Some(_), ..
                } => unimplemented!(
                    "dameng: renaming a column and changing its definition in one ALTER TABLE is not supported"
                ),
                _ => {
                    let new_column_def = ColumnDef {
                        name: self.column_def.name.clone(),
                        ty: self
                            .changes
                            .new_ty
                            .as_ref()
                            .unwrap_or(&self.column_def.ty)
                            .clone(),
                        not_null: self
                            .changes
                            .new_not_null
                            .unwrap_or(self.column_def.not_null),
                        auto_increment: self
                            .changes
                            .new_auto_increment
                            .unwrap_or(self.column_def.auto_increment),
                        check: self.column_def.check.clone(),
                    };
                    fmt!(&mut f, "ALTER TABLE " table_name " MODIFY " new_column_def)
                }
            },
        }
    }
}

/// Returns the text type used to convert between distinct PostgreSQL enum types.
/// PostgreSQL does not define casts between separately declared enums, even when
/// their variants match, so `ALTER COLUMN ... TYPE` must cast each value through
/// its textual label. See <https://wiki.postgresql.org/wiki/Mass_type_replacement>.
fn enum_change_intermediate_ty(previous: &db::Type, next: &db::Type) -> Option<&'static str> {
    match (previous, next) {
        (db::Type::Enum(previous), db::Type::Enum(next)) if previous.name != next.name => {
            Some("TEXT")
        }
        (db::Type::List(previous), db::Type::List(next)) => {
            match (previous.as_ref(), next.as_ref()) {
                (db::Type::Enum(previous), db::Type::Enum(next)) if previous.name != next.name => {
                    Some("TEXT[]")
                }
                _ => None,
            }
        }
        _ => None,
    }
}

impl ToSql for &stmt::AlterTable {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match &self.action {
            stmt::AlterTableAction::RenameTo(new_name) => {
                fmt!(f, "ALTER TABLE " self.name " RENAME TO " new_name);
            }
        }
    }
}

impl ToSql for &stmt::CopyTable {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let target_cols = Comma(self.columns.iter().map(|(target, _)| target));
        let source_cols = Comma(self.columns.iter().map(|(_, source)| source));
        fmt!(f, "INSERT INTO " self.target " (" target_cols ") SELECT " source_cols " FROM " self.source);
    }
}

impl ToSql for &stmt::CreateTable {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let table = f.serializer.table(self.table);
        let name = TableIdent(&table.name);
        let columns = ColumnsWithConstraints(self);

        // Create new expression scope to serialize the statement
        let mut f = f.scope(table);

        fmt!(
            &mut f, "CREATE TABLE " name " (" columns ")"
        );
    }
}

impl ToSql for &stmt::CreateType {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        use toasty_core::stmt::Value;

        let name = self
            .ty
            .name
            .as_deref()
            .expect("CREATE TYPE requires a type name");
        let name = Ident(name);

        fmt!(f, "CREATE TYPE " name " AS ENUM (");
        for (i, variant) in self.ty.variants.iter().enumerate() {
            if i > 0 {
                f.dst.push_str(", ");
            }
            Value::String(variant.name.clone()).to_sql(f);
        }
        f.dst.push(')');
    }
}

impl ToSql for &stmt::AlterType {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        use toasty_core::stmt::Value;

        let name = Ident(&self.type_name);

        fmt!(f, "ALTER TYPE " name " ADD VALUE ");
        Value::String(self.variant.name.clone()).to_sql(f);
    }
}

impl ToSql for &stmt::RenameType {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        fmt!(
            f,
            "ALTER TYPE " Ident(&self.type_name) " RENAME TO " Ident(&self.new_name)
        );
    }
}

impl ToSql for &stmt::Delete {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        assert!(self.returning.is_none());

        // Conditions never reach the serializer: the planner rewrites a
        // conditional DELETE into a CTE or read-modify-write plan (see
        // `plan_conditional_sql_query_as_*`), stripping the condition and
        // folding its check into a filter predicate.
        debug_assert!(
            self.condition.is_none(),
            "SQL DELETE condition should have been lowered by the planner; condition={:#?}",
            self.condition
        );

        // Create a new expression scope to serialize the statement
        let mut f = f.scope(self);
        f.alias = true;

        fmt!(&mut f, "DELETE FROM " self.from self.filter);
    }
}

impl ToSql for &stmt::Filter {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        if let Some(expr) = &self.expr {
            // 达梦不接受「值表达式」当过滤条件（证据与规则见 `Predicate`）。
            // 引擎对「无过滤」的全表查询正是发一个布尔参数，所以这条改写是必需的。
            fmt!(f, " WHERE " Predicate(expr));
        }
    }
}

impl ToSql for &stmt::Direction {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::Direction::Asc => fmt!(f, "ASC"),
            stmt::Direction::Desc => fmt!(f, "DESC"),
        }
    }
}

impl ToSql for &stmt::DropColumn {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let table = f.serializer.table(self.table);
        let table_name = TableIdent(&table.name);
        let if_exists = if self.if_exists { "IF EXISTS " } else { "" };

        // Create new expression scope to serialize the statement
        let mut f = f.scope(table);

        fmt!(&mut f, "ALTER TABLE " table_name " DROP COLUMN " if_exists self.name);
    }
}

impl ToSql for &stmt::DropIndex {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let if_exists = if self.if_exists { "IF EXISTS " } else { "" };
        fmt!(f, "DROP INDEX " if_exists self.name);
    }
}

impl ToSql for &stmt::Pragma {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        if !f.serializer.is_sqlite() {
            panic!("\"PRAGMA\" statements only supported in SQLite");
        }
        match &self.value {
            Some(value) => fmt!(f, "PRAGMA " self.name.as_str() " = " value.as_str()),
            None => fmt!(f, "PRAGMA " self.name.as_str()),
        }
    }
}

impl ToSql for &stmt::DropTable {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let if_exists = if self.if_exists { "IF EXISTS " } else { "" };
        fmt!(f, "DROP TABLE " if_exists self.name);
    }
}

impl ToSql for &stmt::Insert {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        // Create a new expression scope to serialize the statement
        let mut f = f.scope(self);

        let returning = self
            .returning
            .as_ref()
            .map(|returning| (" RETURNING ", returning));

        if returning.is_some() && matches!(f.serializer.dialect, Flavor::Mysql) {
            panic!(
                "MySQL does not support the RETURNING clause with INSERT statements; returning={returning:#?}"
            );
        }

        f.in_insert = true;

        // 达梦的 upsert 用 `MERGE INTO`：源必须写在 `USING (…)` 里，位置与
        // PostgreSQL 的 `INSERT … ON CONFLICT` **完全不同**，所以整条语句自己渲染。
        // 真机形态见 `crates/toasty-driver-dameng/tests/engine_probe.rs` 第 15 节。
        if matches!(f.serializer.dialect, Flavor::Dameng) && self.upsert.is_some() {
            dameng_merge_into(self, &mut f);
            return;
        }

        let upsert = self.upsert.as_ref().map(|_| UpsertClause(self));

        // 达梦：自增（`IDENTITY`）列不能以 `DEFAULT` 或 `NULL` 出现在 `INSERT`
        // 里（真机实测 `-2871` / `-2723`），而引擎对自增列一律发
        // `Expr::Default` ⇒ 这些列必须整列删掉。见
        // `dameng_drop_identity_columns`。
        if matches!(f.serializer.dialect, Flavor::Dameng)
            && dameng_drop_identity_columns(self, &mut f)
        {
            return;
        }

        fmt!(
            &mut f, "INSERT INTO " self.target " " self.source upsert returning
        );
    }
}

/// 达梦的 upsert 渲染：`MERGE INTO`。
///
/// 真机结论（`crates/toasty-driver-dameng/tests/engine_probe.rs` 第 15 节）：
/// `MERGE INTO` 可用；`USING (SELECT ? AS "c" FROM DUAL)` 可用；带不带 `AS`
/// 别名都可以；`SET` 左侧可以不带表别名；命中 `MATCHED` 分支可重复执行；
/// 而 MySQL 的 `ON DUPLICATE KEY UPDATE` 达梦没有（`-2007`）。
///
/// 渲染形态：
///
/// ```sql
/// MERGE INTO "T" AS t
/// USING (SELECT ? AS "s_0", ? AS "s_1" FROM DUAL) AS s
/// ON (t."ID" = s."s_0")
/// WHEN MATCHED THEN UPDATE SET "NAME" = ?
/// WHEN NOT MATCHED THEN INSERT ("ID", "NAME") VALUES (s."s_0", s."s_1")
/// ```
///
/// 只处理「表目标 + 单行 `VALUES` 源」这一种形状（引擎对 upsert 只发这种；
/// 多行会由引擎拆成多条语句）。自增列必须是**显式值**——达梦要
/// `SET IDENTITY_INSERT "<表>" ON` 才允许写自增列（真机 `-2723`），那一步在驱动
/// 层（`crates/toasty-driver-dameng/src/conn.rs`）补，序列化器只渲染这一条语句。
fn dameng_merge_into(insert: &stmt::Insert, f: &mut Formatter<'_>) {
    f.in_insert = true;

    let upsert = insert.upsert.as_ref().unwrap();
    // 冲突目标列（主键或唯一键）——**不是**「主键」的通用别名：`ON (...)` 用它们，
    // 而 `WHEN MATCHED` 的空更新（`Ignore`）必须避开它们（达梦不允许更新关联列）。
    let stmt::UpsertTarget::Columns(target_columns) = &upsert.target else {
        panic!("upsert target must be lowered before SQL serialization")
    };
    let stmt::InsertTarget::Table(insert_table) = &insert.target else {
        panic!(
            "dameng MERGE needs a table target; target={:#?}",
            insert.target
        )
    };
    let stmt::ExprSet::Values(values) = &insert.source.body else {
        panic!(
            "dameng MERGE needs a VALUES source; source={:#?}",
            insert.source
        )
    };
    let [row] = &values.rows[..] else {
        panic!(
            "dameng MERGE needs exactly one source row; rows={:#?}",
            values.rows
        )
    };
    let fields: Vec<&stmt::Expr> = match row {
        stmt::Expr::Record(record) => record.fields.iter().collect(),
        _ if insert_table.columns.len() == 1 => vec![row],
        _ => panic!("dameng MERGE needs a record source row; row={row:#?}"),
    };
    assert_eq!(
        fields.len(),
        insert_table.columns.len(),
        "dameng MERGE source row arity mismatch"
    );

    let table = f.serializer.schema.table(insert_table.table);
    if insert_table
        .columns
        .iter()
        .zip(&fields)
        .any(|(column_id, expr)| {
            table.columns[column_id.index].auto_increment && matches!(expr, stmt::Expr::Default)
        })
    {
        panic!(
            "dameng upsert needs explicit values for the auto-increment columns of `{}`; \
             `DEFAULT` cannot be written in a MERGE INTO",
            table.name
        );
    }

    let table_name = f.serializer.table_name(insert_table);
    let source = Comma(
        fields
            .iter()
            .enumerate()
            .map(|(ordinal, expr)| AliasedSourceValue(expr, ordinal)),
    );
    let on = Delimited(
        target_columns.iter().map(|column_id| MergeJoin {
            column: *column_id,
            ordinal: insert_table
                .columns
                .iter()
                .position(|candidate| candidate == column_id)
                .unwrap_or_else(|| {
                    panic!("upsert key column {column_id:?} is not in the insert column list")
                }),
        }),
        " AND ",
    );

    fmt!(
        f,
        "MERGE INTO " table_name " AS t USING (SELECT " source " FROM DUAL) AS s ON (" on ")"
    );

    match upsert.action {
        stmt::UpsertAction::Update => {
            // `WHEN MATCHED` 分支里 `upsert.update(...)` 引用的「新值」要落到 `USING`
            // 源列上（达梦没有 `excluded`）⇒ 把插入列列表挂到 formatter 上，供
            // `Expr::Incoming` 的达梦臂查 ordinal（见 `serializer/expr.rs`）。
            let previous_source = f.dameng_merge_source.replace(insert_table.columns.clone());
            let assignments = AssignmentList::upsert(table, &upsert.shared);
            fmt!(f, " WHEN MATCHED THEN UPDATE SET " assignments);
            f.dameng_merge_source = previous_source;
        }
        // 达梦没有 `DO NOTHING`，但 `WHEN MATCHED` 分支又要给点东西，所以用「把某列赋成
        // 它自己」的空更新占位，再**补一个恒假条件** `WHERE 1 = 0`——这样命中行时受影响
        // 行数是 **0**，驱动才能把「被忽略」与「真的插进去了」分开（`or_ignore()` 要
        // 在这种情形返回 `None`：真机探针 16m/16n）。
        //
        // **不能挑 `ON` 关联列**做占位列——那正是冲突目标列，达梦会报
        // `[HY000/-4064] 不能更新关联条件中的列`（Round 18 才发现：探针 15f 当年用的是
        // 非键列 `"name" = t."name"`，所以这个坑一直没暴露；起因是套件
        // `crud_upsert::upsert_explicit_assignments_override_model_defaults`）。
        //
        // 优先挑一个不在冲突目标里的列；只有键列的表没有这种列，那时**整个 MATCHED
        // 分支都不写**——`MERGE … ON (…) WHEN NOT MATCHED THEN INSERT …` 正是
        // 「命中就什么都不做」且命中行受影响行数为 0（探针 16b/16c/16h/16j 确认）。
        stmt::UpsertAction::Ignore => {
            if let Some(column) = insert_table
                .columns
                .iter()
                .find(|column| !target_columns.contains(column))
            {
                // 右值必须带目标别名 `t.`：真机探针 16f 里 `SET "name" = "name"` 被拒
                // （`[HY000/-2112]`，`USING` 子查询里也有同名列 ⇒ 解析不明确），而
                // `SET "name" = t."name"`（探针 16d）是达梦接受的写法。
                fmt!(
                    f,
                    " WHEN MATCHED THEN UPDATE SET "
                    f.serializer.column_name(*column)
                    " = t."
                    f.serializer.column_name(*column)
                    " WHERE 1 = 0"
                );
            }
        }
    }

    let columns = Comma(
        insert_table
            .columns
            .iter()
            .map(|column_id| f.serializer.column_name(*column_id)),
    );
    let inserted = Comma((0..fields.len()).map(SourceRef));

    fmt!(
        f,
        " WHEN NOT MATCHED THEN INSERT (" columns ") VALUES (" inserted ")"
    );
}

/// `USING` 子查询里的一个源值：`<expr> AS "s_<ordinal>"`。
struct AliasedSourceValue<'a>(&'a stmt::Expr, usize);

impl ToSql for AliasedSourceValue<'_> {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        self.0.to_sql(f);
        f.dst.push_str(" AS ");
        SourceAlias(self.1).to_sql(f);
    }
}

/// `USING` 子查询里的列别名（不带 `s.` 前缀）：`"s_<ordinal>"`。
#[derive(Clone, Copy)]
struct SourceAlias(usize);

impl ToSql for SourceAlias {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        f.dst.push_str("\"s_");
        f.dst.push_str(&self.0.to_string());
        f.dst.push('"');
    }
}

/// 引用源列：`s."s_<ordinal>"`。
#[derive(Clone, Copy)]
struct SourceRef(usize);

impl ToSql for SourceRef {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        f.dst.push_str("s.");
        SourceAlias(self.0).to_sql(f);
    }
}

/// `ON` 条件里的一个键列：`t."<列>" = s."s_<ordinal>"`。
#[derive(Clone, Copy)]
struct MergeJoin {
    column: db::ColumnId,
    ordinal: usize,
}

impl ToSql for MergeJoin {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        fmt!(
            f,
            "t." f.serializer.column_name(self.column) " = " SourceRef(self.ordinal)
        );
    }
}

/// 达梦专用的 `INSERT` 改写：把「自增列 + 每一行对应值都是 `Expr::Default`」
/// 的列从列名表与每一行里删掉。
///
/// 返回 `true` 表示语句已经渲染完毕（调用方直接返回）。
///
/// 真机实测（`crates/toasty-driver-dameng/tests/insert_probe.rs`）：
/// - `("id", "name") VALUES (DEFAULT, ?)` → `[HY000/-2871] DEFAULT不允许作为显式标识值`
/// - `("id", "name") VALUES (NULL, ?)` → `[HY000/-2723] 仅当指定列列表，且SET IDENTITY_INSERT为ON时，才能对自增列赋值`
/// - `("name") VALUES (?)`（整列省略）→ 成功
/// - 只有自增列时 `INSERT INTO t DEFAULT VALUES` → 成功
/// - 非自增列上的 `DEFAULT`（`("name", "flag") VALUES (?, DEFAULT)`）→ 成功，保持原样
///
/// 只处理「目标是表 + 源是 `VALUES` + 无 `upsert`/`returning`」这一种形状；
/// 其余形状返回 `false` 交给通用路径（真出现时按既有路径报错或渲染）。
fn dameng_drop_identity_columns(insert: &stmt::Insert, f: &mut Formatter<'_>) -> bool {
    if insert.upsert.is_some() || insert.returning.is_some() {
        return false;
    }

    let stmt::InsertTarget::Table(insert_table) = &insert.target else {
        return false;
    };
    let stmt::ExprSet::Values(values) = &insert.source.body else {
        return false;
    };

    // 每行的字段列表：`VALUES` 的行是 `Expr::Record`；单列时允许行本身就是值。
    let mut rows: Vec<Vec<&stmt::Expr>> = Vec::with_capacity(values.rows.len());
    for row in &values.rows {
        match row {
            stmt::Expr::Record(record) => rows.push(record.fields.iter().collect()),
            _ if insert_table.columns.len() == 1 => rows.push(vec![row]),
            _ => return false,
        }
    }
    if rows
        .iter()
        .any(|fields| fields.len() != insert_table.columns.len())
    {
        return false;
    }

    let table = f.serializer.schema.table(insert_table.table);
    let drop: Vec<bool> = insert_table
        .columns
        .iter()
        .enumerate()
        .map(|(i, column_id)| {
            table.columns[column_id.index].auto_increment
                && rows
                    .iter()
                    .all(|fields| matches!(fields[i], stmt::Expr::Default))
        })
        .collect();

    if !drop.iter().any(|dropped| *dropped) {
        return false;
    }

    let table_name = f.serializer.table_name(insert_table);

    if drop.iter().all(|dropped| *dropped) {
        // 全是自增列：`(id) VALUES (DEFAULT)` 达梦同样拒绝，`DEFAULT VALUES` 可用。
        fmt!(f, "INSERT INTO " table_name " DEFAULT VALUES");
        return true;
    }

    struct DamengRow<'a>(Vec<&'a stmt::Expr>);

    impl ToSql for DamengRow<'_> {
        fn to_sql(self, f: &mut Formatter<'_>) {
            let fields = Comma(self.0);
            fmt!(f, "(" fields ")");
        }
    }

    let columns = Comma(
        insert_table
            .columns
            .iter()
            .enumerate()
            .filter(|(i, _)| !drop[*i])
            .map(|(_, column_id)| f.serializer.column_name(*column_id)),
    );
    let kept_rows = rows.iter().map(|fields| {
        let kept: Vec<&stmt::Expr> = fields
            .iter()
            .enumerate()
            .filter(|(i, _)| !drop[*i])
            .map(|(_, expr)| *expr)
            .collect();
        DamengRow(kept)
    });

    fmt!(f, "INSERT INTO " table_name " (" columns ") VALUES " Delimited(kept_rows, ", "));
    true
}

struct UpsertClause<'a>(&'a stmt::Insert);

impl ToSql for UpsertClause<'_> {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let insert = self.0;
        let upsert = insert.upsert.as_ref().unwrap();
        let target = insert.target.as_table_unwrap();
        let stmt::UpsertTarget::Columns(columns) = &upsert.target else {
            panic!("upsert target must be lowered before SQL serialization")
        };
        let columns = Comma(
            columns
                .iter()
                .map(|column| f.serializer.column_name(*column)),
        );

        fmt!(f, " ON CONFLICT (" columns ")");
        match upsert.action {
            stmt::UpsertAction::Ignore => fmt!(f, " DO NOTHING"),
            stmt::UpsertAction::Update => {
                let table = f.serializer.table(target.table);
                let assignments = AssignmentList::upsert(table, &upsert.shared);
                fmt!(f, " DO UPDATE SET " assignments);
            }
        }
    }
}

impl ToSql for &stmt::InsertTarget {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::InsertTarget::Table(insert_table) => {
                let table_name = f.serializer.table_name(insert_table);
                let columns = Comma(
                    insert_table
                        .columns
                        .iter()
                        .map(|column_id| f.serializer.column_name(*column_id)),
                );

                fmt!(f, table_name " (" columns ")");
            }
            _ => todo!("self={self:?}"),
        }
    }
}

impl ToSql for &stmt::Limit {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::Limit::Cursor(cursor) => {
                assert!(
                    cursor.after.is_none(),
                    "Limit::Cursor with after cannot be serialized to SQL, should already be lowered"
                );
                fmt!(f, "LIMIT " cursor.page_size);
            }
            stmt::Limit::Offset(limit_offset) => {
                fmt!(f, "LIMIT " limit_offset.limit);
                if let Some(offset) = limit_offset.offset.as_ref() {
                    fmt!(f, " OFFSET " offset);
                }
            }
        }
    }
}

impl ToSql for &stmt::Query {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        // Create a new expression scope to serialize the statement
        let mut f = f.scope(self);
        f.alias = true;

        let locks = if self.locks.is_empty() {
            None
        } else {
            Some((" ", Delimited(&self.locks, " ")))
        };

        let body = &self.body;
        let order_by = self.order_by.as_ref().map(|order_by| (" ", order_by));
        let limit = self.limit.as_ref().map(|limit| (" ", limit));

        fmt!(&mut f, self.with body order_by limit locks);
    }
}

impl ToSql for &stmt::ExprSet {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::ExprSet::Select(expr) => expr.to_sql(f),
            stmt::ExprSet::Values(expr) => expr.to_sql(f),
            stmt::ExprSet::Update(expr) => expr.to_sql(f),
            stmt::ExprSet::Delete(expr) => expr.to_sql(f),
            _ => todo!("self={self:?}"),
        }
    }
}

impl ToSql for &stmt::OrderBy {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let order_by = Comma(&self.exprs);

        fmt!(f, "ORDER BY " order_by);
    }
}

impl ToSql for &stmt::OrderByExpr {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        // 达梦的 NULL 默认落点与**引擎的 keyset 分页假设**不一致：
        // 引擎按 PostgreSQL 语义生成游标谓词（`ASC` 时认为 NULL 排在**末尾**——
        // 游标值是 NULL 时它只发 `<col> IS NULL AND <主键> > ?`，不再有 `<col> > ?`
        // 那一支），而达梦 `ORDER BY x ASC` 真机实测把 NULL 排在**最前**
        // （见 `engine_probe` 第 14 节），于是分页在 NULL 组里排完就停了：
        // 官方套件 `paginate_multi_column::paginate_nullable_unique_column_uses_primary_key`
        // 实得 `[1, 2]`、期望 `[1, 2, 3, 4]`（行是 `(1,NULL)(2,NULL)(3,10)(4,20)`）。
        //
        // ⇒ 达梦分支把 NULL 落点**显式钉成 PostgreSQL 那一套**：`ASC → NULLS LAST`、
        // `DESC → NULLS FIRST`。不写就等于把语义交给驱动的默认值，翻页会错。
        if let Some(order) = &self.order {
            fmt!(f, self.expr " " order);
            if matches!(f.serializer.dialect, Flavor::Dameng) {
                if matches!(order, stmt::Direction::Asc) {
                    fmt!(f, " NULLS LAST");
                } else {
                    fmt!(f, " NULLS FIRST");
                }
            }
        } else {
            fmt!(f, self.expr);
        }
    }
}

/// 投影列里的一个表达式：达梦分支会把**布尔表达式**包成 `CASE WHEN … THEN 1 ELSE 0 END`。
///
/// 为什么必须有这一步：达梦不接受布尔值当列（真机实测 `SELECT 1 = 1 AS c FROM DUAL`
/// 报 `[42000/-2007] 第 1 行, 第 44 列[AS]附近出现错误`），而 `CASE WHEN … THEN 1 ELSE 0
/// END` 返回 `Int(1)`/`Int(0)`。
struct Projected<'a>(&'a stmt::Expr);

impl ToSql for Projected<'_> {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        if matches!(f.serializer.dialect, Flavor::Dameng) && boolean_valued(self.0) {
            fmt!(f, "CASE WHEN " self.0 " THEN 1 ELSE 0 END");
        } else {
            self.0.to_sql(f);
        }
    }
}

/// 布尔条件里的一个表达式：达梦分支会把**值表达式**补成 `<expr> = 1`。
///
/// 达梦在语法层区分「谓词」与「值表达式」：比较、`AND/OR/NOT`、`IS NULL`、
/// `EXISTS`、`LIKE`、`BETWEEN`、`IN` 是谓词，可以直接当条件；布尔**列**、
/// 绑定**参数**、引用是值表达式，直接当条件会被拒：
///
/// ```text
/// WHERE ?          → [HY000/-4104] 查询使用值表达式作为过滤条件
/// WHERE ? = TRUE   → [07006/-6105] 数据类型不匹配
/// WHERE ? = 1      → OK（布尔参数与整型参数两种形态都过）
/// WHERE <布尔列>    → 同上，直接当条件也是 -4104
/// ```
///
/// （以上都是本仓真机实测，见 `docs/设计方案.md` §9.4 规则 16。）
///
/// 所以达梦分支统一补 ` = 1`（**不是** ` = TRUE`，那会类型不匹配）。
/// 非达梦方言原样输出，上游 golden 快照不受影响。
pub(super) struct Predicate<'a>(pub(super) &'a stmt::Expr);

impl ToSql for Predicate<'_> {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        if matches!(f.serializer.dialect, Flavor::Dameng) && !boolean_valued(self.0) {
            fmt!(f, "(" self.0 ") = 1");
        } else {
            self.0.to_sql(f);
        }
    }
}

/// 这个表达式在 SQL 里是不是产生布尔值（达梦需要 `CASE` 包装，见 `Projected`）。
pub(super) fn boolean_valued(expr: &stmt::Expr) -> bool {
    match expr {
        stmt::Expr::And(_)
        | stmt::Expr::Or(_)
        | stmt::Expr::Not(_)
        | stmt::Expr::Between(_)
        | stmt::Expr::InList(_)
        | stmt::Expr::InSubquery(_)
        | stmt::Expr::Like(_)
        | stmt::Expr::StartsWith(_)
        | stmt::Expr::Exists(_)
        | stmt::Expr::IsNull(_)
        | stmt::Expr::Match(_)
        | stmt::Expr::IsVariant(_)
        | stmt::Expr::AllOp(_)
        | stmt::Expr::AnyOp(_)
        | stmt::Expr::Intersects(_)
        | stmt::Expr::IsSuperset(_) => true,
        stmt::Expr::BinaryOp(op) => matches!(
            op.op,
            stmt::BinaryOp::Eq
                | stmt::BinaryOp::Ne
                | stmt::BinaryOp::Gt
                | stmt::BinaryOp::Ge
                | stmt::BinaryOp::Lt
                | stmt::BinaryOp::Le
        ),
        _ => false,
    }
}

impl ToSql for &stmt::Returning {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::Returning::Project(stmt::Expr::Record(expr_record)) => {
                // Alias every projected field positionally (`AS column1`, ...).
                // A nested SELECT/RETURNING referenced from an outer query (e.g.
                // a data-modifying CTE joined for its returned rows) is read by
                // that alias — `ColumnAlias` — so a bare column reference must
                // carry it too, not just computed expressions. Drivers read
                // top-level results positionally, so the alias is harmless there.
                //
                // ⚠️ 达梦不接受「布尔表达式直接当投影列」（真机实测 `SELECT 1 = 1 AS c
                // FROM DUAL` 报 `[42000/-2007] 第 1 行, 第 44 列[AS]附近出现错误`，
                // 官方的 `field_version` 等簇就是 `SELECT … "version" = ? AS column1`）
                // ⇒ 达梦分支把这类表达式包成 `CASE WHEN … THEN 1 ELSE 0 END`
                // （`Projected`），驱动侧再把 0/1 还原成 `Bool`（见 `value.rs`）。
                let fields = expr_record
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, expr)| (Projected(expr), Some(" AS "), Some(ColumnAlias(i))));

                fmt!(f, Comma(fields));
            }
            stmt::Returning::Project(stmt::Expr::Value(stmt::Value::Record(value_record))) => {
                fmt!(f, Comma(&value_record.fields));
            }
            _ => todo!("returning={self:#?}"),
        }
    }
}

impl ToSql for &stmt::Select {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let source_table = self.source.as_table_unwrap();
        let select = if self.distinct {
            "SELECT DISTINCT "
        } else {
            "SELECT "
        };

        if source_table.from.is_empty() {
            fmt!(f, select self.returning)
        } else {
            fmt!(f, select self.returning " FROM " self.source self.filter);
        }
    }
}

impl ToSql for &stmt::Lock {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::Lock::Update => fmt!(f, "FOR UPDATE"),
            stmt::Lock::Share => fmt!(f, "FOR SHARE"),
        }
    }
}

impl ToSql for &stmt::Source {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::Source::Table(source_table) => {
                source_table.to_sql(f);
            }
            _ => todo!("self={self:?}"),
        }
    }
}

impl ToSql for &toasty_core::stmt::Statement {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        use toasty_core::stmt::Statement::*;

        f.depth += 1;

        match self {
            Delete(stmt) => stmt.to_sql(f),
            Insert(stmt) => stmt.to_sql(f),
            Query(stmt) => stmt.to_sql(f),
            Update(stmt) => stmt.to_sql(f),
        }

        f.depth -= 1;
    }
}

impl ToSql for &stmt::Statement {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::Statement::AddColumn(stmt) => stmt.to_sql(f),
            stmt::Statement::AlterColumn(stmt) => stmt.to_sql(f),
            stmt::Statement::AlterTable(stmt) => stmt.to_sql(f),
            stmt::Statement::AlterType(stmt) => stmt.to_sql(f),
            stmt::Statement::CopyTable(stmt) => stmt.to_sql(f),
            stmt::Statement::CreateIndex(stmt) => stmt.to_sql(f),
            stmt::Statement::CreateTable(stmt) => stmt.to_sql(f),
            stmt::Statement::CreateType(stmt) => stmt.to_sql(f),
            stmt::Statement::DropColumn(stmt) => stmt.to_sql(f),
            stmt::Statement::DropIndex(stmt) => stmt.to_sql(f),
            stmt::Statement::DropTable(stmt) => stmt.to_sql(f),
            stmt::Statement::Pragma(stmt) => stmt.to_sql(f),
            stmt::Statement::RenameType(stmt) => stmt.to_sql(f),
            stmt::Statement::Delete(stmt) => stmt.to_sql(f),
            stmt::Statement::Insert(stmt) => stmt.to_sql(f),
            stmt::Statement::Query(stmt) => stmt.to_sql(f),
            stmt::Statement::Update(stmt) => stmt.to_sql(f),
        }
    }
}

impl ToSql for &stmt::SourceTable {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        // Iterate over each TableWithJoins in the from clause
        for (i, table_with_joins) in self.from.iter().enumerate() {
            if i > 0 {
                fmt!(f, ", ");
            }

            // Serialize the main table relation
            match &table_with_joins.relation {
                stmt::TableFactor::Table(table_id) => {
                    let table_ref = &self.tables[table_id.0];
                    let alias = TableAlias {
                        depth: f.depth,
                        table: *table_id,
                    };

                    fmt!(f, table_ref " AS " alias);
                }
            }

            // Serialize the joins
            for join in &table_with_joins.joins {
                let (kw, expr) = match &join.constraint {
                    stmt::JoinOp::Inner(expr) => (" INNER JOIN ", expr),
                    stmt::JoinOp::Left(expr) => (" LEFT JOIN ", expr),
                };
                let join_table_ref = &self.tables[join.table.0];
                let alias = TableAlias {
                    depth: f.depth,
                    table: join.table,
                };
                fmt!(f, kw join_table_ref " AS " alias " ON " expr);
            }
        }
    }
}

impl ToSql for &stmt::TableRef {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::TableRef::Table(table_id) => {
                let table_name = f.serializer.table_name(*table_id);
                fmt!(f, table_name);
            }
            stmt::TableRef::Derived(table_derived) => fmt!(f, table_derived),
            stmt::TableRef::Cte { nesting, index } => {
                assert!(f.depth >= *nesting, "nesting={nesting} depth={}", f.depth);

                let depth = f.depth - nesting;
                fmt!(f, "cte_" depth "_" index);
            }
            stmt::TableRef::Arg(..) => panic!("unexpected TableRef argument; table_ref={self:#?}"),
        }
    }
}

impl ToSql for &stmt::TableDerived {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        debug_assert!(f.alias);

        f.depth += 1;
        fmt!(f, "(" self.subquery ")");
        f.depth -= 1;
    }
}

struct TableAlias {
    depth: usize,
    table: SourceTableId,
}

impl ToSql for &TableAlias {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        fmt!(f, "tbl_" self.depth "_" self.table.0);
    }
}

impl ToSql for &stmt::Update {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let table = f.serializer.schema.table(self.target.as_table_unwrap());
        let assignments = AssignmentList::update(table, &self.assignments);

        // Create a new expression scope to serialize the statement
        let mut f = f.scope(self);
        f.alias = false;

        // 达梦：列表 `remove(v)`（语义是「删掉**每一个**匹配元素」）在单条 `UPDATE … SET`
        // 里没有可用写法 —— `SET 列 = (SELECT … JSON_TABLE(列) …)` 因为引用了 UPDATE 目标
        // 列被判 `[HY000/-4083] 引用列未找到`，而让路径自己过滤（`'$[*]?(@ != …)'`）达梦判
        // `[HY000/-3102] JSON路径表达式语法错误`。可用的形状是 `MERGE` + 派生表重建，见
        // [`serialize_dameng_list_remove`]；形状由真机探针第 22 节验证（22g/22h 普通与绑定
        // 参数、22i/22j 全匹配边界，日志 `target/m22_probe22k.log` / `target/m22_probe22l.log`）。
        if matches!(f.serializer.dialect, Flavor::Dameng)
            && self.returning.is_none()
            && let Some((column, value)) = dameng_list_remove(table, &self.assignments)
        {
            serialize_dameng_list_remove(&mut f, &self.filter, table, &self.target, column, value);
            return;
        }

        let returning = self
            .returning
            .as_ref()
            .map(|returning| (" RETURNING ", returning));

        if returning.is_some() && f.serializer.is_mysql() {
            panic!(
                "MySQL does not support the RETURNING clause with UPDATE statements; returning={returning:#?}"
            );
        }

        // Conditions never reach the serializer: the planner rewrites a
        // conditional UPDATE into a CTE or read-modify-write plan (see
        // `plan_conditional_sql_query_as_*`), stripping the condition and
        // folding its check into a filter predicate.
        debug_assert!(
            self.condition.is_none(),
            "SQL UPDATE condition should have been lowered by the planner; condition={:#?}",
            self.condition
        );

        fmt!(&mut f, "UPDATE " self.target " SET " assignments self.filter returning);
    }
}

/// 认出「这条 `UPDATE` 恰好就是一次列表 `remove(v)`」。
///
/// 只认**恰好一个**赋值、且它落在 `List(..)` 列上的 `Assignment::Remove`：多个赋值
/// （`remove` 与 `set` 混写）或同一投影上的 `Batch` 都没有对应的派生表形状（派生表只重建
/// 一列），交给旧的渲染路径（`serialize_remove`）如实报错。
fn dameng_list_remove<'a>(
    table: &'a db::Table,
    assignments: &'a stmt::Assignments,
) -> Option<(&'a db::Column, &'a stmt::Expr)> {
    if assignments.len() != 1 {
        return None;
    }

    let (projection, assignment) = assignments.iter().next()?;
    let stmt::Assignment::Remove(value) = assignment else {
        return None;
    };

    let column = table.resolve(projection);
    matches!(column.storage_ty, db::Type::List(_)).then_some((column, value))
}

/// 达梦：把「列表 `remove(v)`」渲染成 `MERGE` + 派生表重建。
///
/// 形状（真机探针第 22 节验证：22g/22h 普通与绑定参数、22i/22j 全匹配边界）：
///
/// ```sql
/// MERGE INTO "T" AS t USING (
///   SELECT src."PK" AS "PK_0",
///          '[' || NVL(LISTAGG(CASE WHEN jt."E" <> ? THEN JSON_QUOTE(jt."E") END, ',')
///                     WITHIN GROUP (ORDER BY jt."N"), '') || ']' AS "V_0"
///   FROM (SELECT "PK", "COL" FROM "T" AS tbl_0_0 WHERE …) src
///   LEFT JOIN JSON_TABLE(src."COL", '$[*]'
///                        COLUMNS ("E" VARCHAR(8188) PATH '$', "N" FOR ORDINALITY)) jt
///     ON 1 = 1
///   GROUP BY src."PK") s
/// ON (t."PK" = s."PK_0")
/// WHEN MATCHED THEN UPDATE SET t."COL" = s."V_0"
/// ```
///
/// 三处都不是「顺手简化」，各有真机依据：
///
/// ① `LEFT JOIN … ON 1 = 1` 而不是逗号连接 —— 逗号连接下「元素全被删掉」的行在派生表里
///    没有分组，`MERGE` 就**不会更新**它（语义上应写成 `[]`）：探针 22i 就是这么错的，
///    22j 换成 `LEFT JOIN` 并把过滤挪进 `CASE` 才正确；
/// ② 过滤落在 `CASE` 里、靠 `LISTAGG` 跳过 NULL 的性质丢掉匹配元素，而不是写
///    `WHERE jt."E" <> 值`（后者又会把整个空分组滤掉）；
/// ③ 元素 JSON 文本：`PATH '$'` 给出的是**裸文本**，数字/布尔/定点数在 JSON 里就是裸
///    字面量、直接拼回去；字符串/时间/UUID/网络地址要 `JSON_QUOTE` 补引号（它的转义是
///    对的，探针 20m）。
fn serialize_dameng_list_remove(
    f: &mut Formatter<'_>,
    filter: &stmt::Filter,
    table: &db::Table,
    target: &stmt::UpdateTarget,
    column: &db::Column,
    value: &stmt::Expr,
) {
    let pk_columns: Vec<&db::Column> = table.primary_key_columns().collect();
    assert!(
        !pk_columns.is_empty(),
        "达梦的列表 `remove` 靠主键把重建结果写回表，但表 {} 没有主键",
        table.name
    );

    let db::Type::List(element) = &column.storage_ty else {
        unreachable!("只有 `List(..)` 列的 `remove` 才会走到这条渲染路径");
    };
    // 数字/布尔元素在 JSON 里是裸字面量；其余类型要引号。
    let bare_element = matches!(
        **element,
        db::Type::Boolean
            | db::Type::Integer(_)
            | db::Type::UnsignedInteger(_)
            | db::Type::Float(_)
            | db::Type::Numeric(_)
    );

    f.dst.push_str("MERGE INTO ");
    f.serializer.table_name(table.id).to_sql(f);

    // USING 子查询：主键列 + 重建后的列表列。
    f.dst.push_str(" AS t USING (SELECT ");
    for (index, pk) in pk_columns.iter().enumerate() {
        if index > 0 {
            f.dst.push_str(", ");
        }
        f.dst.push_str("src.");
        f.serializer.column_name(pk.id).to_sql(f);
        f.dst.push_str(" AS ");
        Ident(&format!("pk_{index}")).to_sql(f);
    }
    f.dst
        .push_str(", '[' || NVL(LISTAGG(CASE WHEN jt.\"E\" <> ");
    value.to_sql(f);
    f.dst.push_str(" THEN ");
    if bare_element {
        f.dst.push_str("jt.\"E\"");
    } else {
        f.dst.push_str("JSON_QUOTE(jt.\"E\")");
    }
    f.dst.push_str(
        " END, ',') WITHIN GROUP (ORDER BY jt.\"N\"), '') || ']' AS \"V_0\" FROM (SELECT ",
    );
    for (index, pk) in pk_columns.iter().enumerate() {
        if index > 0 {
            f.dst.push_str(", ");
        }
        f.serializer.column_name(pk.id).to_sql(f);
    }
    f.dst.push_str(", ");
    Ident(&column.name).to_sql(f);
    f.dst.push_str(" FROM ");
    // 与原 `UPDATE` 用**同一个**渲染器（`"T" AS tbl_<depth>_0`）：过滤表达式里的列引用若被
    // 渲染成限定名，也仍然指得到。
    target.to_sql(f);
    filter.to_sql(f);
    f.dst.push_str(") src LEFT JOIN JSON_TABLE(src.");
    Ident(&column.name).to_sql(f);
    f.dst.push_str(
        ", '$[*]' COLUMNS (\"E\" VARCHAR(8188) PATH '$', \"N\" FOR ORDINALITY)) jt ON 1 = 1 GROUP BY ",
    );
    for (index, pk) in pk_columns.iter().enumerate() {
        if index > 0 {
            f.dst.push_str(", ");
        }
        f.dst.push_str("src.");
        f.serializer.column_name(pk.id).to_sql(f);
    }
    f.dst.push_str(") s ON (");
    for (index, pk) in pk_columns.iter().enumerate() {
        if index > 0 {
            f.dst.push_str(" AND ");
        }
        f.dst.push_str("t.");
        f.serializer.column_name(pk.id).to_sql(f);
        f.dst.push_str(" = s.");
        Ident(&format!("pk_{index}")).to_sql(f);
    }
    f.dst.push_str(") WHEN MATCHED THEN UPDATE SET t.");
    Ident(&column.name).to_sql(f);
    f.dst.push_str(" = s.\"V_0\"");
}

struct AssignmentList<'a> {
    table: &'a db::Table,
    assignments: &'a stmt::Assignments,
    qualify_existing: bool,
}

impl<'a> AssignmentList<'a> {
    fn update(table: &'a db::Table, assignments: &'a stmt::Assignments) -> Self {
        Self {
            table,
            assignments,
            qualify_existing: false,
        }
    }

    fn upsert(table: &'a db::Table, assignments: &'a stmt::Assignments) -> Self {
        Self {
            table,
            assignments,
            qualify_existing: true,
        }
    }
}

#[derive(Clone, Copy)]
struct AssignmentColumn(db::ColumnId);

impl ToSql for AssignmentColumn {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        if matches!(f.serializer.dialect, Flavor::Postgresql)
            && f.assignment_table == Some(self.0.table)
        {
            fmt!(f, f.serializer.table_name(self.0.table) "." f.serializer.column_name(self.0))
        } else {
            fmt!(f, f.serializer.column_name(self.0))
        }
    }
}

impl ToSql for AssignmentList<'_> {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        let previous_assignment_table = f.assignment_table;
        f.assignment_table = self.qualify_existing.then_some(self.table.id);
        let assignments: Vec<_> = self.assignments.iter().collect();

        for (i, (projection, assignment)) in assignments.iter().enumerate() {
            if i > 0 {
                f.dst.push_str(", ");
            }

            let column = self.table.resolve(projection);
            let column_name = Ident(&column.name);
            let existing_column = AssignmentColumn(column.id);

            // Serialize column name and equals sign
            column_name.to_sql(f);
            f.dst.push_str(" = ");

            match assignment {
                stmt::Assignment::Set(expr) => expr.to_sql(f),
                stmt::Assignment::Append(expr) => {
                    serialize_append(f, existing_column, expr);
                }
                stmt::Assignment::Remove(expr) => {
                    serialize_remove(f, existing_column, expr);
                }
                stmt::Assignment::Pop => {
                    serialize_pop(f, existing_column);
                }
                stmt::Assignment::RemoveAt(expr) => {
                    serialize_remove_at(f, existing_column, expr);
                }
                stmt::Assignment::Add(expr) => {
                    fmt!(f, existing_column " + " expr);
                }
                stmt::Assignment::Subtract(expr) => {
                    fmt!(f, existing_column " - " expr);
                }
                _ => todo!(
                    "only SET / APPEND / REMOVE / POP / REMOVE_AT / ADD / SUBTRACT supported in SQL serialization; got {assignment:#?}"
                ),
            }
        }

        f.assignment_table = previous_assignment_table;
    }
}

/// Emit a backend-specific append expression for an `Assignment::Append`.
///
/// The right-hand side is a list expression (PG `text[]` bind for native
/// arrays, JSON string bind for MySQL `JSON` and SQLite JSON1). The
/// serializer renders the dialect's native append operator:
///
/// - PostgreSQL: `col || $1` — `text[] || text[]` concatenates arrays.
///   During an upsert, `col` is table-qualified to distinguish the stored
///   value from `excluded.col`.
/// - MySQL: `JSON_MERGE_PRESERVE(col, $1)` — preserves duplicates and
///   appends every element of the right-hand array to the left-hand one.
/// - SQLite: a scalar expression removes the closing bracket from the stored
///   array and the opening bracket from the appended array, then joins them
///   with a comma when both contain elements. `json()` validates and
///   canonicalizes the result. This form also works in SQLite-compatible
///   engines that reject subqueries inside an upsert assignment.
fn serialize_append(f: &mut super::Formatter<'_>, column: AssignmentColumn, expr: &stmt::Expr) {
    match f.serializer.dialect {
        Flavor::Postgresql => fmt!(f, column " || " expr),
        Flavor::Mysql | Flavor::MariaDb => {
            fmt!(f, "JSON_MERGE_PRESERVE(" column ", " expr ")")
        }
        Flavor::Sqlite => fmt!(
            f,
            "json(substr(" column ", 1, length(" column ") - 1) || \
             CASE WHEN json_array_length(" column ") > 0 \
             AND json_array_length(" expr ") > 0 THEN ',' ELSE '' END || \
             substr(" expr ", 2))"
        ),
        // 达梦：列表列存成 `CLOB` 里的 JSON 文本，且没有 `json()`/
        // `json_array_length()`；用纯字符串判空（空数组即 `'[]'`）等价地拼接。
        // 前提：我们写进去的一定是规范化的 `[…]` 文本（见 `conn.rs` 的编码）。
        Flavor::Dameng => fmt!(
            f,
            "substr(" column ", 1, length(" column ") - 1) || \
             CASE WHEN " column " <> '[]' AND " expr " <> '[]' THEN ',' ELSE '' END || \
             substr(" expr ", 2)"
        ),
    }
}

/// Emit `stmt::remove(value)` against a `Vec<scalar>` column.
///
/// - PostgreSQL: `array_remove(col, $value)` — removes every element equal
///   to `$value` from a `T[]` column. Atomic.
/// - 达梦：列表列是 `CLOB` 里的 JSON 数组文本。**普通的 `UPDATE … SET` 不再走这里**
///   —— 它在 `impl ToSql for &stmt::Update` 里被 [`serialize_dameng_list_remove`]
///   （`MERGE` + 派生表重建）接管，形状由真机探针第 22 节钉死（22g/22h 普通与绑定
///   参数、22i/22j 全匹配边界）。这里只剩**那条路没覆盖的形状**：`AssignmentList::upsert`
///   的命中分支（upsert 的 `WHEN MATCHED THEN UPDATE` 也调这个函数）与「一次 `UPDATE`
///   里 `remove` 和其他赋值混写」。二者都如实 `unimplemented!`（**不静默降级**）——
///   达梦上「删掉每个匹配元素」没有单条表达式写法：没有 `JSON_ARRAYAGG`（探针 18k
///   `-2207`），`JSON_REMOVE` 既不吃「路径数组」（18z4 `-3102`）也不吃过滤路径（18z7），
///   多路径还是**顺序施加**（18j）⇒ 不能靠 `JSON_SEARCH('all')`；`JSON_QUERY` 的过滤路径
///   整个不存在（Round 22 探针 20a–20o 一律 `-3102`）；把重建塞进 `SET` 的相关子查询被判
///   `[HY000/-4083] 引用列未找到`（19q/19s/19y/19z2/19z4）。⇒ `Capability::vec_remove`
///   只在普通 `UPDATE` 上成立，取舍见 `docs/设计方案.md` §12.2 第 24 条。
fn serialize_remove(f: &mut super::Formatter<'_>, column: AssignmentColumn, expr: &stmt::Expr) {
    match f.serializer.dialect {
        Flavor::Postgresql => fmt!(f, "array_remove(" column ", " expr ")"),
        Flavor::Dameng => unimplemented!(
            "dameng: stmt::remove on a Vec<scalar> column is only rendered for a plain \
             `UPDATE … SET` (MERGE + derived-table rebuild); upsert matched-clauses and \
             multi-assignment updates are not covered — see docs/设计方案.md §12.2 item 24",
        ),
        Flavor::Mysql | Flavor::MariaDb | Flavor::Sqlite => panic!(
            "stmt::remove on a Vec<scalar> field is not yet implemented for this SQL dialect; \
             the lowering should have rejected this before reaching the serializer",
        ),
    }
}

/// Emit `stmt::pop()` against a `Vec<scalar>` column.
///
/// - PostgreSQL: `col[1:cardinality(col) - 1]` — slices off the last
///   element via 1-based PG array slicing. Atomic.
/// - 达梦：`JSON_REMOVE(col, '$[n-1]')`；空列表必须走 `CASE`——路径里出现
///   `-1` 会被达梦判成 `-3102 JSON 路径表达式语法错误`（真机探针 18g），
///   而 `CASE` 的另一支直接返回原列（空的 no-op 语义）。
/// - MySQL / SQLite: not yet supported — `vec_pop` is gated off and the
///   lowering rejects these backends before reaching here.
fn serialize_pop(f: &mut super::Formatter<'_>, column: AssignmentColumn) {
    match f.serializer.dialect {
        Flavor::Postgresql => fmt!(f, column "[1:cardinality(" column ") - 1]"),
        Flavor::Dameng => fmt!(
            f,
            "CASE WHEN JSON_LENGTH(" column ") = 0 THEN " column " \
             ELSE JSON_REMOVE(" column ", '$[' || (JSON_LENGTH(" column ") - 1) || ']') END"
        ),
        Flavor::Mysql | Flavor::MariaDb | Flavor::Sqlite => panic!(
            "stmt::pop on a Vec<scalar> field is not yet implemented for this SQL dialect; \
             the lowering should have rejected this before reaching the serializer",
        ),
    }
}

/// Emit `stmt::remove_at(idx)` against a `Vec<scalar>` column.
///
/// `idx` is a 0-based `usize`. PostgreSQL arrays are 1-based, so the
/// element at user-facing index `i` lives at PG position `i + 1`. The
/// expression `col[1:i] || col[i + 2:cardinality(col)]` keeps the prefix
/// up to (1-based) position `i` and the suffix from position `i + 2`,
/// dropping the element at position `i + 1` (user index `i`).
///
/// Out-of-bounds indices are a no-op: when `i >= cardinality(col)`, the
/// first slice yields the entire array and the second yields the empty
/// slice, so the concatenation reproduces the input.
///
/// - 达梦：`JSON_REMOVE(col, '$[i]')`。越界路径是 no-op（真机探针 18d：
///   `JSON_REMOVE('["a","b"]', '$[5]')` 原样返回）⇒ 越界语义自动成立。
/// - MySQL / SQLite: not yet supported — `vec_remove_at` is gated off
///   and the lowering rejects these backends before reaching here.
fn serialize_remove_at(f: &mut super::Formatter<'_>, column: AssignmentColumn, expr: &stmt::Expr) {
    match f.serializer.dialect {
        Flavor::Postgresql => fmt!(
            f,
            column "[1:" expr "] || " column "[" expr " + 2:cardinality(" column ")]"
        ),
        // `expr` 是 0 基下标（引擎给的是 `usize` 字面量或整型表达式）；`||`
        // 会把数值隐式转成文本（探针 18u：绑定参数形态也成立）。
        Flavor::Dameng => fmt!(f, "JSON_REMOVE(" column ", '$[' || " expr " || ']')"),
        Flavor::Mysql | Flavor::MariaDb | Flavor::Sqlite => panic!(
            "stmt::remove_at on a Vec<scalar> field is not yet implemented for this SQL dialect; \
             the lowering should have rejected this before reaching the serializer",
        ),
    }
}

impl ToSql for &stmt::UpdateTarget {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::UpdateTarget::Table(table_id) => {
                let table_name = f.serializer.table_name(*table_id);
                let alias = TableAlias {
                    depth: f.depth,
                    table: SourceTableId(0),
                };

                fmt!(f, table_name " AS " alias);
            }
            _ => todo!(),
        }
    }
}

impl ToSql for &stmt::Values {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        // MySQL requires the `ROW(...)` keyword for table value constructors
        // when used in subqueries, but NOT in INSERT statements. MariaDB has
        // no `VALUES ROW(...)` at all and takes the plain form below.
        //
        // Rows are `Expr::Record`s, which serialize with their own `(...)`.
        // Inside `ROW(...)` we need the fields comma-separated *without*
        // those parens — otherwise MySQL parses `ROW((1, 'a'))` as a single
        // row-expression operand and rejects it with "Operand should contain
        // 1 column(s)".
        if matches!(f.serializer.dialect, Flavor::Mysql) && !f.in_insert {
            // `Expr::Record` serializes with its own `(...)`; render its
            // fields directly inside `ROW(...)` so we don't end up with
            // `ROW((a, b))` (which MySQL parses as a single row-typed
            // operand and rejects). Other expression shapes are wrapped
            // as-is — a single-scalar row `ROW(x)` is well-formed.
            for (i, row) in self.rows.iter().enumerate() {
                if i == 0 {
                    fmt!(f, "VALUES ");
                } else {
                    fmt!(f, ", ");
                }
                match row {
                    stmt::Expr::Record(record) => {
                        fmt!(f, "ROW(" Comma(record.fields.iter()) ")")
                    }
                    _ => fmt!(f, "ROW(" row ")"),
                }
            }
        } else if matches!(f.serializer.dialect, Flavor::MariaDb) && !f.in_insert {
            // MariaDB's table value constructor cannot type a bare `?`: the
            // column binds to the empty string, so the join silently matches
            // nothing. A UNION ALL of SELECTs binds and names its own columns.
            let rows = self.rows.iter().enumerate().map(|(i, row)| {
                let fields = match row {
                    stmt::Expr::Record(record) => &record.fields[..],
                    other => std::slice::from_ref(other),
                };
                // Only the first SELECT names the columns.
                let fields = fields.iter().enumerate().map(move |(column, field)| {
                    (field, (i == 0).then_some((" AS ", ColumnAlias(column))))
                });
                ("SELECT ", Comma(fields))
            });
            fmt!(f, Delimited(rows, " UNION ALL "));
        } else if matches!(f.serializer.dialect, Flavor::Dameng) && !f.in_insert {
            // 达梦**没有** `VALUES` 表值构造器：`FROM (VALUES (?)) AS t` 报
            // `[42000/-2007] 第 1 行, 第 225 列[VALUES]附近出现错误: 语法分析出错`
            // （真机实测，引擎在 include/preload 降低时会发这种形状）。
            // 也没有 MySQL 的 `VALUES ROW(...)`。
            // ⇒ 与 MariaDB 臂同构：用 `UNION ALL` 的 `SELECT` 组合代替，列别名只写在第一条
            // `SELECT` 上；额外显式 `FROM DUAL`，因为达梦不保证支持无 `FROM` 的 `SELECT`。
            let rows = self.rows.iter().enumerate().map(|(i, row)| {
                let fields = match row {
                    stmt::Expr::Record(record) => &record.fields[..],
                    other => std::slice::from_ref(other),
                };
                let fields = fields.iter().enumerate().map(move |(column, field)| {
                    (field, (i == 0).then_some((" AS ", ColumnAlias(column))))
                });
                ("SELECT ", Comma(fields), " FROM DUAL")
            });
            fmt!(f, Delimited(rows, " UNION ALL "));
        } else {
            let rows = Comma(self.rows.iter());
            fmt!(f, "VALUES " rows)
        }
    }
}
