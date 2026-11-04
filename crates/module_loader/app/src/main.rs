//! 主程序（app）
//!
//! 职责：读取 `modules.xml` 配置，按依赖拓扑顺序把各业务模块装配进 DI 容器，
//! 然后演示「显式传递容器 + 运行时查询服务 + 单例复用」。
use std::path::Path;
use std::sync::Arc;

use container::Container;
use interfaces::{Database, OrderService, UserService};
use loader::{load_file, load_modules, ModuleRegistry};
use module_db::SqliteDb;
use module_order::OrderServiceImpl;
use module_user::UserServiceImpl;

/// 构造「模块名 -> 装配工厂」注册表。
///
/// 每个工厂只负责把该模块的服务注册进容器（懒构造，装配顺序与注册顺序无关）。
/// 工厂为同步注册：服务实例在首次 `get` 时才真正构造。
fn registry() -> ModuleRegistry {
    let mut r = ModuleRegistry::new();

    // db 模块：注册 Database，无外部依赖。
    r.add("db", |c| {
        // 工厂闭包：首次 get 时才 new 出 SqliteDb，并擦除为 Arc<dyn Database>。
        c.register::<Arc<dyn Database>, _>(|_| {
            Ok(Arc::new(SqliteDb::new()) as Arc<dyn Database>)
        });
        Ok(())
    });

    // user 模块：注册 UserService，依赖 db。
    r.add("user", |c| {
        // 先在工厂内解析出 Database 单例，再注入 UserServiceImpl。
        c.register::<Arc<dyn UserService>, _>(|c| {
            let db = c.get::<Arc<dyn Database>>()?;
            Ok(Arc::new(UserServiceImpl::new(db)) as Arc<dyn UserService>)
        });
        Ok(())
    });

    // order 模块：注册 OrderService，依赖 user。
    r.add("order", |c| {
        // 解析 UserService 单例并注入 OrderServiceImpl。
        c.register::<Arc<dyn OrderService>, _>(|c| {
            let users = c.get::<Arc<dyn UserService>>()?;
            Ok(Arc::new(OrderServiceImpl::new(users)) as Arc<dyn OrderService>)
        });
        Ok(())
    });

    r
}

/// 解析 `modules.xml` 路径：优先环境变量 `MODULES_XML`，
/// 否则回退到相对本 crate 目录的 `../modules.xml`。
fn config_path() -> std::path::PathBuf {
    // 优先采用环境变量显式指定的配置路径。
    if let Ok(p) = std::env::var("MODULES_XML") {
        return std::path::PathBuf::from(p);
    }
    // 否则相对本 crate 源码目录定位 ../modules.xml，失败则回退到当前目录。
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../modules.xml")
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from("modules.xml"))
}

/// 根据配置文件加载模块并组装容器，返回共享的 [`Container`]。
fn build() -> Arc<Container> {
    // 步骤 1：定位并解析配置文件。
    let path = config_path();
    // 读取并解析 XML，得到模块清单。
    let configs = load_file(&path).unwrap_or_else(|e| panic!("读取配置 {:?} 失败: {}", path, e));

    // 步骤 2：创建空容器并按依赖拓扑装配启用的模块。
    let c = Container::new();
    // 依据依赖拓扑把启用的模块装配进容器。
    let report = load_modules(&configs, &registry(), &c)
        .unwrap_or_else(|e| panic!("模块装配失败: {}", e));

    // 步骤 3：打印加载报告（已加载 / 已禁用跳过）。
    println!("[loader] 配置文件: {:?}", path);
    println!("[loader] 已加载模块: {:?}", report.loaded);
    if !report.skipped.is_empty() {
        println!("[loader] 已跳过(禁用): {:?}", report.skipped);
    }
    c
}

/// 显式传递容器（推荐做法）：依赖可见，测试易替换。
fn run_app(container: &Arc<Container>) {
    // 任意位置查询，无需层层持有依赖。
    let orders = container.get::<Arc<dyn OrderService>>().unwrap();
    println!("{}", orders.place(42));

    // 再次查询返回同一实例（单例保证）。
    let orders2 = container.get::<Arc<dyn OrderService>>().unwrap();
    // Arc::ptr_eq 比较底层实例地址，验证确实是同一个对象。
    assert!(Arc::ptr_eq(&orders, &orders2));
    println!("单例校验通过：两次查询指向同一实例");

    // 底层依赖同样是同一实例。
    let users = container.get::<Arc<dyn UserService>>().unwrap();
    println!("count = {}", users.count());
}

/// 程序入口：装配容器并运行示例。
fn main() {
    // 先根据 XML 配置组装容器，再把容器显式传递给业务入口。
    let container = build();
    run_app(&container);
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use interfaces::Database;

    /// 测试替身：一个什么都不查的 Mock 数据库。
    struct MockDb;

    #[async_trait]
    impl Database for MockDb {
        fn name(&self) -> &str {
            "mock"
        }
        fn query(&self, _: &str) -> Vec<String> {
            vec![]
        }
        async fn query_async(&self, _: &str) -> Vec<String> {
            vec![]
        }
    }

    /// 验证：注入 Mock 后，UserService 走的确实是 Mock 实现（依赖可替换）。
    #[test]
    fn test_with_mock() {
        let c = Container::new();
        // 直接注入现成实例，替换真实数据库。
        c.register_instance::<Arc<dyn Database>>(Arc::new(MockDb));
        c.register::<Arc<dyn UserService>, _>(|c| {
            let db = c.get::<Arc<dyn Database>>()?;
            Ok(Arc::new(UserServiceImpl::new(db)) as Arc<dyn UserService>)
        });

        let users = c.get::<Arc<dyn UserService>>().unwrap();
        // Mock 的 name() 返回 "mock"，故拼接结果应包含 "mock"。
        let found = users.find(1).unwrap();
        assert!(
            found.contains("mock"),
            "应使用注入的 mock 库: {}",
            found
        );
    }
}
