//! 业务方法层的异步互调演示
//!
//! 展示 order → user → db 的 `async fn` 调用链：
//! 上层服务的异步方法内部 `await` 下层服务的异步方法，
//! 服务实例仍由 DI 容器以单例方式注入与共享。

use std::sync::Arc;

use container::Container;
use interfaces::{Database, OrderService, UserService};
use module_db::SqliteDb;
use module_order::OrderServiceImpl;
use module_user::UserServiceImpl;

/// 组装容器：注册三个同步单例服务（Database → UserService → OrderService）。
///
/// 依赖在首次 `get` 时才逐层解析构造（懒初始化）。
fn build() -> Arc<Container> {
    let c = Container::new();
    // 底层数据库：无依赖。
    c.register::<Arc<dyn Database>, _>(|_| Ok(Arc::new(SqliteDb::new()) as Arc<dyn Database>));
    // 用户服务：依赖数据库。
    c.register::<Arc<dyn UserService>, _>(|c| {
        let db = c.get::<Arc<dyn Database>>()?;
        Ok(Arc::new(UserServiceImpl::new(db)) as Arc<dyn UserService>)
    });
    // 订单服务：依赖用户服务。
    c.register::<Arc<dyn OrderService>, _>(|c| {
        let users = c.get::<Arc<dyn UserService>>()?;
        Ok(Arc::new(OrderServiceImpl::new(users)) as Arc<dyn OrderService>)
    });
    c
}

/// 入口：在 tokio 运行时下演示同步与异步两种业务调用。
#[tokio::main]
async fn main() {
    let c = build();

    // 取出顶层服务（单例）。
    let orders = c.get::<Arc<dyn OrderService>>().unwrap();

    // 同步调用：order → user → db（普通方法）。
    println!("[sync ] {}", orders.place(42));

    // 异步调用：order.place_async().await → user.find_async().await → db.query_async().await
    println!("[async] {}", orders.place_async(7).await);

    // 也可单独对某一层发起异步调用。
    let users = c.get::<Arc<dyn UserService>>().unwrap();
    println!("[async] user.find_async -> {:?}", users.find_async(100).await);

    let db = c.get::<Arc<dyn Database>>().unwrap();
    println!(
        "[async] db.query_async  -> rows = {}",
        db.query_async("select 1").await.len()
    );

    // 单例校验：无论同步/异步获取，底层实例唯一。
    let orders2 = c.get::<Arc<dyn OrderService>>().unwrap();
    assert!(Arc::ptr_eq(&orders, &orders2));
    println!("异步业务调用链演示完成，单例校验通过。");
}
