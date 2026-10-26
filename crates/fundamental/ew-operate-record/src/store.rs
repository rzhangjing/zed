//! SQLite 存储：建表、批量写入、条件查询、清理。

use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard};

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params, params_from_iter};
use time::OffsetDateTime;

use crate::error::{Error, Result};
use crate::log::{LogEntry, PendingLog};
use crate::query::Query;

/// 当前表结构版本。
const SCHEMA_VERSION: i64 = 2;

/// 记录表：发生时间即主键。
///
/// 时间戳精确到微秒，由写入线程保证严格递增（见 [`Store::insert_batch`]），
/// 所以它天然唯一；省掉自增 id 后，排序、翻页、按时间清理都直接走主键。
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS operation_log (
    occurred_at INTEGER PRIMARY KEY,
    user_id     TEXT    NOT NULL,
    user_name   TEXT,
    action      TEXT    NOT NULL,
    target      TEXT,
    result      TEXT,
    detail      TEXT
);
CREATE INDEX IF NOT EXISTS idx_operation_log_user   ON operation_log (user_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_operation_log_action ON operation_log (action, occurred_at DESC);
"#;

const INSERT_SQL: &str = "INSERT INTO operation_log \
     (occurred_at, user_id, user_name, action, target, result, detail) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

/// 数据库连接（单写者，读写共用一把锁）。
pub(crate) struct Store {
    connection: Mutex<Connection>,
    /// 已写入的最大时间戳（微秒），保证新记录严格递增。
    last_timestamp: AtomicI64,
}

impl Store {
    /// 打开（不存在则创建）数据库。
    pub(crate) fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut connection = Connection::open(path)?;
        configure(&mut connection, true)?;
        migrate(&mut connection)?;
        let last_timestamp = last_timestamp(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            last_timestamp: AtomicI64::new(last_timestamp),
        })
    }

    /// 内存数据库（测试或临时演示；进程退出即丢失）。
    pub(crate) fn open_in_memory() -> Result<Self> {
        let mut connection = Connection::open_in_memory()?;
        configure(&mut connection, false)?;
        migrate(&mut connection)?;
        let last_timestamp = last_timestamp(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            last_timestamp: AtomicI64::new(last_timestamp),
        })
    }

    /// 一个事务写入一批记录。
    ///
    /// 时间戳按 `max(记录时间, 上一条 + 1µs)` 生成：正常用户操作间隔远大于 1µs，
    /// 所以它等于真实发生时间；即使系统时钟粒度粗或发生回拨，也不会产生重复主键。
    pub(crate) fn insert_batch(&self, entries: &[PendingLog]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        {
            let mut statement = transaction.prepare_cached(INSERT_SQL)?;
            for entry in entries {
                let mut occurred_at = to_micros(entry.occurred_at);
                let previous = self.last_timestamp.load(Ordering::Relaxed);
                if occurred_at <= previous {
                    occurred_at = previous.saturating_add(1);
                }
                statement.execute(params![
                    occurred_at,
                    entry.log.user_id,
                    entry.log.user_name,
                    entry.log.action,
                    entry.log.target,
                    entry.log.result,
                    entry.log.detail,
                ])?;
                self.last_timestamp.store(occurred_at, Ordering::Relaxed);
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// 条件查询，按发生时间倒序（最新在前）。
    pub(crate) fn query(&self, query: &Query) -> Result<Vec<LogEntry>> {
        let connection = self.lock()?;
        let limit = query.limit.clamp(1, 1000);
        let (filter, mut binds) = filter_clause(query);
        let mut sql = format!(
            "SELECT occurred_at, user_id, user_name, action, target, result, detail \
             FROM operation_log WHERE 1 = 1{filter}"
        );
        if let Some(before_us) = query.before_us {
            sql.push_str(" AND occurred_at < ?");
            binds.push(SqlValue::Integer(before_us));
        }
        sql.push_str(" ORDER BY occurred_at DESC LIMIT ?");
        binds.push(SqlValue::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));

        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(binds.iter()))?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let occurred_at: i64 = row.get(0)?;
            entries.push(LogEntry {
                occurred_at: from_micros(occurred_at),
                user_id: row.get(1)?,
                user_name: row.get(2)?,
                action: row.get(3)?,
                target: row.get(4)?,
                result: row.get(5)?,
                detail: row.get(6)?,
            });
        }
        Ok(entries)
    }

    /// 条件计数（忽略分页参数）。
    pub(crate) fn count(&self, query: &Query) -> Result<u64> {
        let connection = self.lock()?;
        let (filter, binds) = filter_clause(query);
        let sql = format!("SELECT COUNT(*) FROM operation_log WHERE 1 = 1{filter}");
        let total: i64 =
            connection.query_row(&sql, params_from_iter(binds.iter()), |row| row.get(0))?;
        Ok(u64::try_from(total).unwrap_or_default())
    }

    /// 删除发生时间早于 `cutoff` 的记录，返回删除条数。
    pub(crate) fn prune_before(&self, cutoff: OffsetDateTime) -> Result<u64> {
        let connection = self.lock()?;
        let deleted = connection.execute(
            "DELETE FROM operation_log WHERE occurred_at < ?1",
            [to_micros(cutoff)],
        )?;
        Ok(u64::try_from(deleted).unwrap_or_default())
    }

    /// 把 WAL 内容并回主库并截断 WAL。
    pub(crate) fn checkpoint(&self) -> Result<()> {
        let connection = self.lock()?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| Error::Invalid("数据库连接已损坏".to_owned()))
    }
}

fn configure(connection: &mut Connection, on_disk: bool) -> Result<()> {
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let journal_mode = if on_disk { "WAL" } else { "MEMORY" };
    connection.execute_batch(&format!(
        "PRAGMA journal_mode = {journal_mode};
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -8192;"
    ))?;
    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(Error::Invalid(format!(
            "数据库表结构版本 {version} 高于当前程序支持的 {SCHEMA_VERSION}，请升级程序"
        )));
    }
    if version == 0 {
        connection.execute_batch(SCHEMA)?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        return Ok(());
    }
    if version == 1 {
        // 旧版是「自增 id + occurred_at 普通列（毫秒）」，重建为时间戳主键（微秒）。
        migrate_from_v1(connection)?;
    }
    Ok(())
}

fn migrate_from_v1(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "DROP INDEX IF EXISTS idx_operation_log_user;
         DROP INDEX IF EXISTS idx_operation_log_action;
         DROP INDEX IF EXISTS idx_operation_log_time;
         ALTER TABLE operation_log RENAME TO operation_log_v1;",
    )?;
    transaction.execute_batch(SCHEMA)?;
    {
        let mut select = transaction.prepare(
            "SELECT occurred_at, user_id, user_name, action, target, result, detail \
             FROM operation_log_v1 ORDER BY occurred_at, id",
        )?;
        let mut insert = transaction.prepare_cached(INSERT_SQL)?;
        let mut rows = select.query([])?;
        let mut last: i64 = i64::MIN;
        while let Some(row) = rows.next()? {
            let original: i64 = row.get(0)?;
            // 旧表是毫秒且允许同一毫秒多条：转微秒，并顺延保证唯一。
            let occurred_at = original.saturating_mul(1_000).max(last.saturating_add(1));
            insert.execute(params![
                occurred_at,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ])?;
            last = occurred_at;
        }
    }
    transaction.execute_batch("DROP TABLE operation_log_v1; PRAGMA user_version = 2;")?;
    transaction.commit()?;
    Ok(())
}

fn last_timestamp(connection: &Connection) -> Result<i64> {
    let last: i64 = connection.query_row(
        "SELECT COALESCE(MAX(occurred_at), 0) FROM operation_log",
        [],
        |row| row.get(0),
    )?;
    Ok(last)
}

fn filter_clause(query: &Query) -> (String, Vec<SqlValue>) {
    let mut sql = String::new();
    let mut binds = Vec::new();
    if let Some(user_id) = &query.user_id {
        sql.push_str(" AND user_id = ?");
        binds.push(SqlValue::Text(user_id.clone()));
    }
    if let Some(action) = &query.action {
        sql.push_str(" AND action = ?");
        binds.push(SqlValue::Text(action.clone()));
    }
    if let Some(from) = query.from {
        sql.push_str(" AND occurred_at >= ?");
        binds.push(SqlValue::Integer(to_micros(from)));
    }
    if let Some(to) = query.to {
        sql.push_str(" AND occurred_at <= ?");
        binds.push(SqlValue::Integer(to_micros(to)));
    }
    if let Some(keyword) = &query.keyword {
        let pattern = format!("%{}%", escape_like(keyword));
        sql.push_str(
            " AND (action LIKE ? ESCAPE '\\' OR COALESCE(target, '') LIKE ? ESCAPE '\\' \
             OR COALESCE(detail, '') LIKE ? ESCAPE '\\' OR user_id LIKE ? ESCAPE '\\')",
        );
        for _ in 0..4 {
            binds.push(SqlValue::Text(pattern.clone()));
        }
    }
    (sql, binds)
}

fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn to_micros(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000).unwrap_or_default()
}

fn from_micros(micros: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}
