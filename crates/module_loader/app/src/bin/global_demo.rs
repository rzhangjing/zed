//! 全局容器用法演示（谨慎使用）。
//!
//! 仅在无法显式传递容器的场景（框架回调、宏展开处）考虑全局容器，
//! 并始终提供 set_global / init_with 以便测试替换。

use std::sync::Arc;

use container::{get_or_init_global, Container};
use interfaces::{Database, OrderService, UserService};
use module_db::SqliteDb;
use module_order::OrderServiceImpl;
use module_user::UserServiceImpl;

/// 任意深层位置：无需层层传递容器即可拿到服务（正是全局容器要解决的场景）。
fn somewhere_deep_in_the_stack() {
    // 直接从全局单例读取顶层订单服务。
    let orders = container::global().get::<Arc<dyn OrderService>>().unwrap();
    println!("[全局] {}", orders.place(7));
}

/// 组装容器：注册 Database → UserService → OrderService 三级同步单例。
fn build() -> Arc<Container> {
    let c = Container::new();
    // 底层数据库。
    c.register::<Arc<dyn Database>, _>(|_| Ok(Arc::new(SqliteDb::new()) as Arc<dyn Database>));
    // 用户服务，依赖数据库。
    c.register::<Arc<dyn UserService>, _>(|c| {
        let db = c.get::<Arc<dyn Database>>()?;
        Ok(Arc::new(UserServiceImpl::new(db)) as Arc<dyn UserService>)
    });
    // 订单服务，依赖用户服务。
    c.register::<Arc<dyn OrderService>, _>(|c| {
        let users = c.get::<Arc<dyn UserService>>()?;
        Ok(Arc::new(OrderServiceImpl::new(users)) as Arc<dyn OrderService>)
    });
    c
}

/// 入口：惰性初始化全局容器，再从深层调用点访问服务。
fn main() {
    // 初始化全局容器（幂等：若已存在则不覆盖）。
    let installed = get_or_init_global(build);
    println!(
        "[全局] 容器{}就绪",
        if installed { "已初始化" } else { "早已" }
    );

    somewhere_deep_in_the_stack();
}
