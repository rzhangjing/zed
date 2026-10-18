use super::Serializer;

use toasty_core::{driver::SqlPlaceholder, schema::db};

/// 本 fork 的方言枚举。
///
/// 为什么不用上游的 `toasty_core::driver::Dialect`：那是**闭 4 变体**枚举，
/// 加第 5 个变体必须级联 fork `toasty-core` + `toasty`（见 `docs/设计方案.md`
/// §9.1.1 的 T8.0 结案）。方言身份（`Capability.sql`）依旧复用上游的
/// `Dialect::Mysql`，**渲染方言**由本枚举独立表达 —— 两者解耦。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavor {
    Sqlite,
    Postgresql,
    Mysql,
    MariaDb,
    /// 达梦 DM8（Oracle 兼容模式）。
    Dameng,
}

impl<'a> Serializer<'a> {
    /// Creates a serializer that emits SQLite SQL.
    pub fn sqlite(schema: &'a db::Schema) -> Self {
        Self::sqlite_with_default_begin(schema, "BEGIN")
    }

    /// Creates a SQLite-dialect serializer with a custom SQL string for
    /// `TransactionMode::Default`.
    ///
    /// Used by SQLite-compatible engines whose preferred "no opinion" BEGIN
    /// is not the classic deferred form — e.g. Turso with
    /// `concurrent_writes()` enabled, where `Default` means `BEGIN
    /// CONCURRENT`. The non-`Default` modes (`Deferred`, `Immediate`,
    /// `Exclusive`) still map to their standard SQLite SQL.
    pub fn sqlite_with_default_begin(schema: &'a db::Schema, default_begin: &'static str) -> Self {
        Serializer {
            schema,
            dialect: Flavor::Sqlite,
            sqlite_default_begin: default_begin,
        }
    }

    /// Returns `true` if this serializer targets SQLite.
    pub fn is_sqlite(&self) -> bool {
        matches!(self.dialect, Flavor::Sqlite)
    }

    /// Creates a serializer that emits PostgreSQL SQL.
    pub fn postgresql(schema: &'a db::Schema) -> Self {
        Serializer {
            schema,
            dialect: Flavor::Postgresql,
            sqlite_default_begin: "BEGIN",
        }
    }

    /// Creates a serializer that emits MySQL SQL.
    pub fn mysql(schema: &'a db::Schema) -> Self {
        Serializer {
            schema,
            dialect: Flavor::Mysql,
            sqlite_default_begin: "BEGIN",
        }
    }

    /// Creates a serializer that emits MariaDB SQL.
    pub fn mariadb(schema: &'a db::Schema) -> Self {
        Serializer {
            schema,
            dialect: Flavor::MariaDb,
            sqlite_default_begin: "BEGIN",
        }
    }

    /// Creates a serializer that emits 达梦 DM8 SQL.
    ///
    /// 达梦的渲染差异（相对 MySQL/MariaDB）：ANSI 双引号标识符、
    /// `IDENTITY(1, 1)` 自增、`CLOB`/`BLOB`/`VARBINARY(n)`、无 `JSON`/数组类型、
    /// 无 `ON DUPLICATE KEY UPDATE`（upsert 走 `MERGE INTO`）。
    pub fn dameng(schema: &'a db::Schema) -> Self {
        Serializer {
            schema,
            dialect: Flavor::Dameng,
            sqlite_default_begin: "BEGIN",
        }
    }

    /// Returns `true` for both MySQL and MariaDB.
    ///
    /// Most rendering is shared; the places that are not match on
    /// [`Flavor`] directly.
    pub(super) fn is_mysql(&self) -> bool {
        matches!(self.dialect, Flavor::Mysql | Flavor::MariaDb)
    }
}

/// The placeholder syntax a dialect's bind layer accepts.
pub(super) fn sql_placeholder(flavor: Flavor) -> SqlPlaceholder {
    match flavor {
        Flavor::Postgresql => SqlPlaceholder::DollarNumber,
        Flavor::Sqlite => SqlPlaceholder::NumberedQuestionMark,
        // 达梦的 ODBC 驱动用位置式 `?`（与 MySQL 同形），因此
        // `Serializer::serialize_with_arg_order` 的参数重排对它同样必需。
        Flavor::Mysql | Flavor::MariaDb | Flavor::Dameng => SqlPlaceholder::QuestionMark,
    }
}
