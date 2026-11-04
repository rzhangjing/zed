//! SQLite 数据库实现（module-db）
//!
//! 提供 [`Database`] 接口的一个具体实现。
//! 本模块不依赖容器，只关心接口契约；由主程序负责向容器注册。
use async_trait::async_trait;
use interfaces::Database;

/// 模拟 SQLite 连接（实际项目中替换为 `rusqlite::Connection` 等）。
pub struct SqliteDb {
    /// 数据库连接字符串（标识符，仅用于演示）。
    conn: String,
}

impl SqliteDb {
    /// 创建一个新的模拟 SQLite 实例，默认连接 `sqlite://app.db`。
    pub fn new() -> Self {
        Self {
            conn: "sqlite://app.db".into(),
        }
    }
}

/// 实现 [`Default`]：等同于 [`SqliteDb::new`]，便于自动派生。
impl Default for SqliteDb {
    fn default() -> Self {
        Self::new()
    }
}

/// 实现 [`Database`] trait：同步与异步两套查询方法。
#[async_trait]
impl Database for SqliteDb {
    /// 返回连接标识字符串（同步）。
    fn name(&self) -> &str {
        &self.conn
    }

    /// 同步查询：直接返回两行固定数据（模拟结果集）。
    fn query(&self, _sql: &str) -> Vec<String> {
        vec!["row-1".into(), "row-2".into()]
    }

    /// 异步查询：让出执行器模拟非阻塞 IO（sleep 10ms），再返回同样的两行数据。
    /// 上层业务可通过 `await` 本方法实现异步数据访问。
    async fn query_async(&self, _sql: &str) -> Vec<String> {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        vec!["row-1".into(), "row-2".into()]
    }
}
