//! 订单服务实现（module-order）
//!
//! 提供 [`OrderService`] 接口的实现：依赖 [`UserService`]。
use async_trait::async_trait;
use interfaces::{OrderService, UserService};
use std::sync::Arc;

/// 订单服务实现体。
pub struct OrderServiceImpl {
    /// 用户服务接口（构造时注入，与同类型其它服务共享同一实例）。
    users: Arc<dyn UserService>,
}

impl OrderServiceImpl {
    /// 创建实例。`users`：注入的用户服务接口，一般由容器工厂解析后传入。
    pub fn new(users: Arc<dyn UserService>) -> Self {
        Self { users }
    }
}

/// 实现 [`OrderService`]：同时提供同步与异步两类下单方法。
#[async_trait]
impl OrderService for OrderServiceImpl {
    /// 同步下单：`users.find()` → 拼接描述字符串。
    fn place(&self, user_id: u64) -> String {
        let user = self.users.find(user_id).unwrap_or_default();
        format!("订单已创建: {}", user)
    }

    /// 异步下单：`await` 下游 [`UserService::find_async`]（order → user → db 异步链）。
    async fn place_async(&self, user_id: u64) -> String {
        let user = self.users.find_async(user_id).await.unwrap_or_default();
        format!("订单已创建(异步): {}", user)
    }
}
