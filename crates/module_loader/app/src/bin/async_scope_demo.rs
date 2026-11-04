//! 异步服务工厂 + 请求级作用域 演示。
//!
//! 场景：
//! - 全局单例：Database、UserService、以及一个"异步构造"的 RemoteClient（模拟启动时异步连接）。
//! - 每个请求派生一个 Scope，注入请求级上下文 RequestCtx，并在作用域内构造请求级异步服务，
//!   它同时依赖父容器的单例与本请求的上下文。

use std::sync::Arc;

use container::{Container, Scope};
use interfaces::{Database, UserService};
use module_db::SqliteDb;
use module_user::UserServiceImpl;

/// 异步构造的远程客户端（模拟：启动时需 await 建立连接）。
pub struct RemoteClient {
    /// 已建立的远程端点地址。
    endpoint: String,
}

impl RemoteClient {
    /// 模拟一次异步握手连接，返回共享客户端。`db_name` 用于拼出端点名。
    async fn connect(db_name: &str) -> Arc<Self> {
        // 模拟异步 IO 握手。
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        Arc::new(Self {
            endpoint: format!("wss://remote/{}", db_name),
        })
    }
}

/// 请求级上下文（每请求不同）。派生 `Clone` 以便作为服务令牌被容器克隆返回。
#[derive(Clone)]
struct RequestCtx {
    /// 本次请求的唯一标识。
    request_id: String,
}

/// 请求级服务：结合本请求上下文 + 父容器单例。
struct RequestReporter {
    /// 本请求的上下文（请求级，来自 Scope）。
    ctx: RequestCtx,
    /// 用户服务（父容器单例，跨请求共享）。
    user: Arc<dyn UserService>,
    /// 远程客户端（父容器异步单例，跨请求共享）。
    remote: Arc<RemoteClient>,
}

impl RequestReporter {
    /// 生成一条上报文本，聚合请求 id、用户信息与远程端点。
    fn report(&self, user_id: u64) -> String {
        format!(
            "[{}] user={} via {}",
            self.ctx.request_id,
            self.user.find(user_id).unwrap_or_default(),
            self.remote.endpoint
        )
    }
}

/// 组装全局容器：同步单例（Database/UserService）+ 异步单例（RemoteClient）。
async fn build_root() -> Arc<Container> {
    let c = Container::new();

    // 底层数据库单例。
    c.register::<Arc<dyn Database>, _>(|_| Ok(Arc::new(SqliteDb::new()) as Arc<dyn Database>));
    // 用户服务单例，依赖数据库。
    c.register::<Arc<dyn UserService>, _>(|c| {
        let db = c.get::<Arc<dyn Database>>()?;
        Ok(Arc::new(UserServiceImpl::new(db)) as Arc<dyn UserService>)
    });

    // 异步工厂：await 依赖，模拟异步连接建立。
    c.register_async::<Arc<RemoteClient>, _, _>(|c| {
        Box::pin(async move {
            let db = c.get::<Arc<dyn Database>>()?;
            Ok(RemoteClient::connect(db.name()).await)
        })
    });

    c
}

/// 为一个请求派生作用域，并注册请求级异步服务。
fn build_request_scope(root: &Arc<Container>, request_id: &str) -> Scope {
    let scope = root.scope();
    // 注入本请求的上下文（请求级实例）。
    scope.insert::<RequestCtx>(RequestCtx {
        request_id: request_id.to_string(),
    });

    // 请求级异步服务：依赖本作用域的 RequestCtx 与父容器的单例。
    scope.register_async::<Arc<RequestReporter>, _, _>(|s| {
        Box::pin(async move {
            let ctx = s.get::<RequestCtx>()?; // 请求级（本地同步）
            let user = s.get_async::<Arc<dyn UserService>>().await?; // 委托父单例
            let remote = s.get_async::<Arc<RemoteClient>>().await?; // 委托父异步单例
            Ok(Arc::new(RequestReporter { ctx, user, remote }))
        })
    });

    scope
}

/// 入口：验证异步单例收敛 + 多请求作用域隔离。
#[tokio::main]
async fn main() {
    let root = build_root().await;

    // 异步单例：多次查询收敛到同一实例。
    let r1 = root.get_async::<Arc<RemoteClient>>().await.unwrap();
    let r2 = root.get_async::<Arc<RemoteClient>>().await.unwrap();
    assert!(Arc::ptr_eq(&r1, &r2)); // ptr_eq 确认地址相同（而非内容相同）
    println!("[root] remote = {}", r1.endpoint);

    // 两个并发请求各自拥有独立作用域。
    // 每个 tokio::spawn 模拟一个并发 HTTP 请求到达。
    let handles: Vec<_> = ["req-1", "req-2"]
        .iter()
        .map(|id| {
            let root = root.clone();
            let id = id.to_string();
            // 在独立任务中：派生作用域 -> 解析请求级服务 -> 上报。
            tokio::spawn(async move {
                let scope = build_request_scope(&root, &id);
                let reporter = scope.get_async::<Arc<RequestReporter>>().await.unwrap();
                reporter.report(42)
            })
        })
        .collect();

    for h in handles {
        // 依次 await 各任务，打印本请求的上报文本。
        println!("[scope] {}", h.await.unwrap());
    }

    // 作用域服务不污染父容器：确认 RequestCtx/RequestReporter 仅存在于 Scope 内。
    assert!(!root.contains::<RequestCtx>());
    assert!(!root.contains::<Arc<RequestReporter>>());
    println!("[ok] 请求级实例未泄漏到父容器");
}
