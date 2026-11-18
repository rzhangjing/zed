#[macro_use]
mod fmt;
use fmt::ToSql;

mod column;
use column::ColumnAlias;

mod cte;

mod delim;
use delim::{Comma, Delimited, Period};

mod dialect;
pub(crate) use dialect::Flavor;

mod ident;
use ident::{Ident, TableIdent};
// `dameng_table_name` 供驱动渲染 `SET IDENTITY_INSERT` 用：那边是手写 SQL 片段，
// 走不了 `ToSql`，必须共享同一套表名渲染规则（模式名原样 / 表名折大写）。
pub use ident::dameng_table_name;

mod params;
pub use params::Placeholder;

// Fragment serializers
mod column_def;
mod expr;
mod name;
mod statement;
mod ty;
mod value;

use crate::stmt::Statement;

use toasty_core::{
    driver::operation::{IsolationLevel, Transaction, TransactionMode},
    schema::db::{self, Index, Table},
    stmt::IntoExprTarget,
};

/// Serialize a statement to a SQL string
#[derive(Debug)]
pub struct Serializer<'a> {
    /// Schema against which the statement is to be serialized
    schema: &'a db::Schema,

    /// The SQL dialect handles the differences between databases and
    /// supported features.
    dialect: Flavor,

    /// SQL emitted for [`TransactionMode::Default`] under the SQLite dialect.
    /// Constructors that don't override this leave it at `"BEGIN"`, which is
    /// SQLite's natural default (DEFERRED). A driver that wants `Default` to
    /// mean something engine-specific — Turso under `concurrent_writes()`
    /// uses `"BEGIN CONCURRENT"` — sets this through
    /// [`Self::sqlite_with_default_begin`]. The non-`Default` variants
    /// (`Deferred`, `Immediate`, `Exclusive`) always emit fixed SQL.
    sqlite_default_begin: &'static str,
}

struct Formatter<'a> {
    /// Handle to the serializer
    serializer: &'a Serializer<'a>,

    /// Expression-resolution context for the current scope. Re-scoped (via
    /// [`Formatter::scope`]) each time serialization descends into a new
    /// query level, so it travels with the formatter rather than as a
    /// separate argument.
    cx: ExprContext<'a>,

    /// Where to write the serialized SQL
    dst: &'a mut String,

    /// Current query depth. This is used to determine the nesting level when
    /// generating names
    depth: usize,

    /// True when table names should be aliased.
    alias: bool,

    /// True when inside an INSERT statement. Used by MySQL to decide whether
    /// VALUES rows need the ROW() wrapper (required in subqueries but not in
    /// INSERT).
    in_insert: bool,

    /// Target table whose stored columns must be qualified inside a PostgreSQL
    /// upsert assignment to distinguish them from `excluded` columns.
    assignment_table: Option<db::TableId>,

    /// 正在渲染的达梦 `MERGE INTO` 的插入列列表（`InsertTable::columns` 的副本），
    /// **顺序即 `USING` 子查询里的源列别名顺序**（`s."s_<ordinal>"`）。
    ///
    /// 达梦 upsert 的 `WHEN MATCHED` 分支没有 `excluded`：`upsert.update(...)` 里引用的
    /// 「新值」（`stmt::Expr::Incoming`）只能落到这里的源列上。只在渲染 `WHEN MATCHED`
    /// 分支期间为 `Some`（见 `serializer::statement::dameng_merge_into`）。
    dameng_merge_source: Option<Vec<db::ColumnId>>,

    /// Collects `Expr::Arg(n)` positions in the order they appear in the SQL.
    /// Used by MySQL (which uses positional `?` without indices) to reorder
    /// the params vec to match placeholder occurrence order. Borrowed so a
    /// scoped child formatter writes through to the root's vec.
    arg_positions: &'a mut Vec<usize>,
}

impl<'a> Formatter<'a> {
    /// Descend into a new expression scope, returning a child formatter that
    /// shares this one's output sink and arg collector (so writes flow back
    /// to the root) but resolves references against `target`.
    ///
    /// The child borrows `self`, so the parent scope stays live on the stack
    /// for the child's lifetime — that is what keeps the `ExprContext` parent
    /// chain valid for nested-reference resolution.
    fn scope<'c>(&'c mut self, target: impl IntoExprTarget<'c, db::Schema>) -> Formatter<'c> {
        Formatter {
            serializer: self.serializer,
            cx: self.cx.scope(target),
            dst: &mut *self.dst,
            depth: self.depth,
            alias: self.alias,
            in_insert: self.in_insert,
            assignment_table: self.assignment_table,
            dameng_merge_source: self.dameng_merge_source.clone(),
            arg_positions: &mut *self.arg_positions,
        }
    }
}

/// Expression context bound to a database-level schema.
pub type ExprContext<'a> = toasty_core::stmt::ExprContext<'a, db::Schema>;

impl<'a> Serializer<'a> {
    /// Serializes a [`Statement`] to a SQL string with all values inlined as
    /// literals (no bind parameters). Appends a trailing semicolon.
    ///
    /// Use this for DDL statements (`CREATE TABLE`, `CREATE TYPE`, etc.) where
    /// bind parameters are not supported. DML statements should already have
    /// their parameters extracted (as `Expr::Arg` placeholders) before reaching
    /// the serializer.
    pub fn serialize(&self, stmt: &Statement) -> String {
        self.serialize_with_arg_order(stmt).0
    }

    /// Serializes a [`Statement`] and returns both the SQL string and the order
    /// in which `Expr::Arg(n)` placeholders appear in the SQL.
    ///
    /// The arg order is needed by MySQL which uses positional `?` without
    /// indices — the caller must reorder its params vec to match the occurrence
    /// order. PostgreSQL and SQLite use indexed placeholders (`$1`, `?1`) so
    /// they can ignore the arg order.
    pub fn serialize_with_arg_order(&self, stmt: &Statement) -> (String, Vec<usize>) {
        let mut ret = String::new();
        let mut arg_positions = Vec::new();

        {
            let mut fmt = Formatter {
                serializer: self,
                cx: ExprContext::new(self.schema),
                dst: &mut ret,
                depth: 0,
                alias: false,
                in_insert: false,
                assignment_table: None,
                dameng_merge_source: None,
                arg_positions: &mut arg_positions,
            };

            stmt.to_sql(&mut fmt);
        }

        ret.push(';');
        (ret, arg_positions)
    }

    /// Serialize a transaction control operation to a SQL string.
    ///
    /// The generated SQL is dialect-specific (e.g., MySQL uses `START TRANSACTION`
    /// while other databases use `BEGIN`). Savepoints are named `sp_{id}`.
    pub fn serialize_transaction(&self, op: &Transaction) -> String {
        let mut ret = String::new();
        let mut arg_positions = Vec::new();

        {
            let mut f = Formatter {
                serializer: self,
                cx: ExprContext::new(self.schema),
                dst: &mut ret,
                depth: 0,
                alias: false,
                in_insert: false,
                assignment_table: None,
                dameng_merge_source: None,
                arg_positions: &mut arg_positions,
            };

            match op {
                Transaction::Start {
                    isolation,
                    read_only,
                    mode,
                } => fmt!(
                    &mut f,
                    self.serialize_transaction_start(*isolation, *read_only, *mode)
                ),
                Transaction::Commit => fmt!(&mut f, "COMMIT"),
                Transaction::Rollback => fmt!(&mut f, "ROLLBACK"),
                Transaction::Savepoint(name) => {
                    fmt!(&mut f, "SAVEPOINT " Ident(name))
                }
                Transaction::ReleaseSavepoint(name) => {
                    fmt!(&mut f, "RELEASE SAVEPOINT " Ident(name))
                }
                Transaction::RollbackToSavepoint(name) => {
                    fmt!(&mut f, "ROLLBACK TO SAVEPOINT " Ident(name))
                }
            };
        }

        ret.push(';');
        ret
    }

    fn serialize_transaction_start(
        &self,
        isolation: Option<IsolationLevel>,
        read_only: bool,
        mode: TransactionMode,
    ) -> String {
        fn isolation_level_str(level: IsolationLevel) -> &'static str {
            match level {
                IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
                IsolationLevel::ReadCommitted => "READ COMMITTED",
                IsolationLevel::RepeatableRead => "REPEATABLE READ",
                IsolationLevel::Serializable => "SERIALIZABLE",
            }
        }

        match self.dialect {
            // MySQL has no SQLite-style lock-mode keyword; drivers
            // reject non-Default `mode` before reaching the serializer.
            Flavor::Mysql | Flavor::MariaDb => {
                let mut sql = String::new();
                if let Some(level) = isolation {
                    sql.push_str("SET TRANSACTION ISOLATION LEVEL ");
                    sql.push_str(isolation_level_str(level));
                    sql.push_str("; ");
                }
                sql.push_str("START TRANSACTION");
                if read_only {
                    sql.push_str(" READ ONLY");
                }
                sql
            }
            // PostgreSQL has no SQLite-style lock-mode keyword; drivers
            // reject non-Default `mode` before reaching the serializer.
            Flavor::Postgresql => {
                let mut sql = String::from("BEGIN");
                if let Some(level) = isolation {
                    sql.push_str(" ISOLATION LEVEL ");
                    sql.push_str(isolation_level_str(level));
                }
                if read_only {
                    sql.push_str(" READ ONLY");
                }
                sql
            }
            // SQLite has no per-transaction isolation level or read-only
            // keyword; the lock-acquisition mode is the only knob. `Default`
            // emits whatever the serializer was configured with at
            // construction (`BEGIN` by default, or e.g. `BEGIN CONCURRENT`
            // for Turso under MVCC). `Deferred`/`Immediate`/`Exclusive` are
            // explicit caller requests with fixed SQL.
            Flavor::Sqlite => match mode {
                TransactionMode::Default => self.sqlite_default_begin.to_string(),
                TransactionMode::Deferred => "BEGIN".to_string(),
                TransactionMode::Immediate => "BEGIN IMMEDIATE".to_string(),
                TransactionMode::Exclusive => "BEGIN EXCLUSIVE".to_string(),
            },
            // 达梦：隔离级别用 `SET TRANSACTION ISOLATION LEVEL`（DM8 支持，
            // V-29 真机实测点）；开事务本身由驱动的 `set_autocommit(false)`
            // 负责（见 `crates/toasty-driver-dameng/src/conn.rs`），这里不重复
            // 发 `START TRANSACTION`，避免 DM 报「已在事务中」。
            Flavor::Dameng => {
                let mut sql = String::new();
                if let Some(level) = isolation {
                    sql.push_str("SET TRANSACTION ISOLATION LEVEL ");
                    sql.push_str(isolation_level_str(level));
                }
                if read_only {
                    if !sql.is_empty() {
                        sql.push_str("; ");
                    }
                    sql.push_str("SET TRANSACTION READ ONLY");
                }
                sql
            }
        }
    }

    fn table(&self, id: impl Into<db::TableId>) -> &'a Table {
        self.schema.table(id.into())
    }

    fn index(&self, id: impl Into<db::IndexId>) -> &'a Index {
        self.schema.index(id.into())
    }

    fn table_name(&self, id: impl Into<db::TableId>) -> TableIdent<&str> {
        let table = self.schema.table(id.into());
        TableIdent(&table.name)
    }

    fn column_name(&self, id: impl Into<db::ColumnId>) -> Ident<&str> {
        let column = self.schema.column(id.into());
        Ident(&column.name)
    }
}
