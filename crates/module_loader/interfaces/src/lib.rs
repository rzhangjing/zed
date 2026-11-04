//! 服务接口定义（跨模块共享的契约层）
//!
//! 本 crate 只声明 trait（接口），不含任何实现，供各业务模块与主程序共同依赖。
//!
//! 每个 trait 同时提供两类方法：
//! - **同步方法**：服务构造期注入后直接调用，零开销。
//! - **异步方法（`async fn`）**：业务方法调用层的异步互调。
//!
//! 借助 `#[async_trait]` 宏，含 `async fn` 的 trait 仍保持「对象安全」，
//! 因此可以用 `Arc<dyn Trait>` 作为令牌注册进 DI 容器并被 `get` 取出。
use async_trait::async_trait;

/// 数据库抽象：屏蔽具体存储实现（如 SQLite / Mock）。
#[async_trait]
pub trait Database: Send + Sync {
    /// 返回连接的标识名称（同步）。
    fn name(&self) -> &str;

    /// 执行一次同步查询，返回字符串行集合。`sql` 为查询语句。
    fn query(&self, sql: &str) -> Vec<String>;

    /// 异步查询：模拟一次非阻塞 IO（业务方法层的异步调用）。
    async fn query_async(&self, sql: &str) -> Vec<String>;
}

/// 用户服务抽象：内部依赖 [`Database`] 完成数据存取。
#[async_trait]
pub trait UserService: Send + Sync {
    /// 按 id 同步查找用户，返回展示字符串；未找到返回 `None`。
    fn find(&self, id: u64) -> Option<String>;

    /// 同步返回用户总数。
    fn count(&self) -> usize;

    /// 异步查找：内部 `await` 依赖的 [`Database::query_async`]。
    async fn find_async(&self, id: u64) -> Option<String>;
}

/// 订单服务抽象：内部依赖 [`UserService`]。
#[async_trait]
pub trait OrderService: Send + Sync {
    /// 同步下单：返回订单结果描述。`user_id` 为下单用户。
    fn place(&self, user_id: u64) -> String;

    /// 异步下单：内部 `await` 依赖的 [`UserService::find_async`]。
    async fn place_async(&self, user_id: u64) -> String;
}
