//! 用户服务实现（module-user）
//!
//! 提供 [`UserService`] 接口的实现：依赖 [`Database`] 存取数据。
use async_trait::async_trait;
use interfaces::{Database, UserService};
use std::sync::Arc;

/// 用户服务实现体。
pub struct UserServiceImpl {
    /// 数据库接口（构造时注入，与同类型其它服务共享同一实例）。
    db: Arc<dyn Database>,
}

impl UserServiceImpl {
    /// 创建实例。`db`：注入的数据库接口，一般由容器工厂解析后传入。
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }
}

/// 实现 [`UserService`]：同时提供同步与异步两类查询方法。
#[async_trait]
impl UserService for UserServiceImpl {
    /// 同步查找用户：仅使用 `db.name()`（无需 IO）。
    fn find(&self, id: u64) -> Option<String> {
        Some(format!("user-{} from {}", id, self.db.name()))
    }

    /// 同步统计用户数：调用 `db.query` 并返回结果行数。
    fn count(&self) -> usize {
        // 固定 SQL 拿回行数作为总数（实际应 COUNT(*) 或缓存）。
        self.db.query("select count(*) from users").len()
    }

    /// 异步查找：`await` 下游 [`Database::query_async`]，体现模块间异步互调（user → db）。
    async fn find_async(&self, id: u64) -> Option<String> {
        let rows = self
            .db
            .query_async("select name from users where id = ?")
            .await;
        Some(format!(
            "user-{} from {} (rows={})",
            id,
            self.db.name(),
            rows.len()
        ))
    }
}
