# 全单例运行时查询 DI 容器

一个纯标准库实现的 Rust 依赖注入容器：**类型擦除注册表 + 延迟初始化 + 循环依赖检测**，
所有服务遵循单例语义。

## 设计概览

```
┌──────────────────────────────────────┐
│              Container               │
│  slots: RwLock<TypeId → Arc<Slot>>   │  ← 类型擦除 + 并发安全
│  Slot.value: OnceLock<Arc<dyn Any>>  │  ← 延迟初始化 / 单例
│  Slot.factory: Fn(&Container) → T     │  ← 首次查询时构造
│  RESOLVE_STACK: thread_local<Vec>     │  ← 循环检测
└──────────────────────────────────────┘
```

单例三原则：

| 原则 | 说明 |
| --- | --- |
| 一次创建 | 首次查询时构造，之后复用同一 `Arc` |
| 线程安全 | `RwLock` 保护注册表，`OnceLock` 保护每个实例 |
| 顺序无关 | 注册顺序不影响查询，构造发生在首次查询时 |

## Workspace 结构

| crate | 职责 |
| --- | --- |
| `container` | 容器核心：`Container` / `Slot` / `CycleGuard` / 全局容器 |
| `interfaces` | 服务接口：`Database` / `UserService` / `OrderService` |
| `module-db` | `SqliteDb` 实现 |
| `module-user` | `UserServiceImpl`（依赖 `Database`） |
| `module-order` | `OrderServiceImpl`（依赖 `UserService`） |
| `loader` | 配置驱动模块加载：解析 `modules.xml` + 注册表装配 |
| `app` | 组装与查询示例，含配置加载、显式传递、全局、异步与作用域演示 |

## 快速开始

```bash
cargo build            # 编译
cargo test             # 运行测试（含并发、循环依赖、全局、异步、作用域、XML 加载）
cargo run -q -p app --bin app          # 依据 modules.xml 配置加载模块
cargo run -q --bin global_demo      # 全局容器示例
cargo run -q --bin async_scope_demo # 异步工厂 + 请求级作用域示例
cargo run -q --bin async_call_demo  # 业务方法层异步互调（order → user → db await 链）
```

## 基本用法

```rust
use std::sync::Arc;
use container::Container;
use interfaces::{Database, OrderService, UserService};

let c = Container::new();

// 注册工厂（延迟初始化，注册顺序无关）
c.register::<Arc<dyn Database>, _>(|_| Ok(Arc::new(SqliteDb::new()) as Arc<dyn Database>));
c.register::<Arc<dyn UserService>, _>(|c| {
    let db = c.get::<Arc<dyn Database>>()?;
    Ok(Arc::new(UserServiceImpl::new(db)) as Arc<dyn UserService>)
});
c.register::<Arc<dyn OrderService>, _>(|c| {
    let users = c.get::<Arc<dyn UserService>>()?;
    Ok(Arc::new(OrderServiceImpl::new(users)) as Arc<dyn OrderService>)
});

// 查询 OrderService 会级联触发 UserService → Database 构造
let orders = c.get::<Arc<dyn OrderService>>().unwrap();
println!("{}", orders.place(42));

// 单例：再次查询返回同一底层实例
assert!(Arc::ptr_eq(&orders, &c.get::<Arc<dyn OrderService>>().unwrap()));
```

> **类型令牌约定**：服务类型 `T` 取 `Arc<dyn Trait>`，`get::<T>()` 直接返回 `T`。
> 类型擦除的 key 是 `Arc<dyn Trait>` 这个具体类型。

## API 一览

| 方法 | 说明 |
| --- | --- |
| `Container::new() -> Arc<Self>` | 新建容器 |
| `register::<T, F>(factory)` | 注册同步工厂，首次查询时构造（`T: Clone`） |
| `register_async::<T, F, Fut>(factory)` | 注册异步工厂，`get_async` 时 await 构造 |
| `register_instance::<T>(value)` | 注入现成实例，立即就绪 |
| `get::<T>() -> Result<T, DiError>` | 同步查询，首次触发构造 |
| `get_async::<T>().await -> Result<T, DiError>` | 异步查询（需 tokio 运行时） |
| `try_get::<T>() -> Option<T>` | 可选查询，未注册返回 `None` |
| `contains::<T>() -> bool` | 是否已注册 |
| `scope() -> Scope` | 派生请求级作用域（共享父单例） |
| `set_global` / `init_with` / `get_or_init_global` | 全局容器安装 / 替换 / 惰性初始化 |
| `global` / `try_global` / `reset_global` | 全局容器读取 / 可选读取 / 清空 |

### Scope（请求级作用域）

| 方法 | 说明 |
| --- | --- |
| `insert::<T>(value)` | 注入请求级实例 |
| `register::<T, F>(factory)` | 请求级同步工厂 |
| `register_async::<T, F, Fut>(factory)` | 请求级异步工厂 |
| `get::<T>()` / `get_async::<T>().await` | 本地优先，未命中委托父容器 |
| `contains_local::<T>()` / `parent()` | 本地注册检查 / 取父容器 |

### 业务方法层的异步调用（async-trait）

容器层解决的是「服务的构造/获取」，而**服务方法之间的相互调用**是另一层问题。二者独立：

- **同步方法互调**：服务构造时注入 `Arc<dyn Trait>` 后，直接调用即可（零开销），如 `OrderService::place → UserService::find → Database::query`。
- **异步方法互调**：`trait` 用 `#[async_trait]` 标注，方法写成 `async fn`（内部 `await` 下游服务的异步方法），即可实现 `order.place_async().await → user.find_async().await → db.query_async().await` 的非阻塞调用链。`async-trait` 负责让含 `async fn` 的 trait 保持**对象安全**，从而仍以 `Arc<dyn Trait>` 作令牌注入容器。

```rust
#[async_trait]
pub trait OrderService: Send + Sync {
    fn place(&self, user_id: u64) -> String;          // 同步互调
    async fn place_async(&self, user_id: u64) -> String; // 异步互调
}

// 在 tokio 运行时中：
let orders = container.get::<Arc<dyn OrderService>>().unwrap();
println!("{}", orders.place_async(7).await); // 触发 order → user → db 异步链
```

> 说明：异步方法的 `await` 依赖下游服务的**方法**，与容器如何构造服务无关；因此异步互调可与同步/异步任意工厂组合使用。见 `app/src/bin/async_call_demo.rs`。

## 配置驱动模块加载（loader）

主程序不再把模块硬编码到 `build()`，而是读取根目录 `modules.xml`，由配置决定**加载哪些模块、以何顺序、是否启用**。

### 配置文件格式

```xml
<modules>
  <module name="db" path="module-db" order="0">
    <provides>Database</provides>
  </module>
  <module name="user" path="module-user" order="10">
    <provides>UserService</provides>
    <depends>db</depends>
  </module>
  <!-- enabled=false 的模块会被跳过，不进入容器 -->
  <module name="metrics" path="module-metrics" enabled="false"/>
</modules>
```

| 字段 | 位置 | 说明 |
| --- | --- | --- |
| `name` | 属性 | 模块名，与 `ModuleRegistry` 登记的工厂 key 对应 |
| `path` | 属性 | 模块相对路径（对应 crate 目录） |
| `enabled` | 属性 | 是否加载，默认 `true` |
| `order` | 属性 | 无依赖约束时的同级装配顺序，越小越先 |
| `provides` | 子元素 | 该模块提供的服务（可多个） |
| `depends` | 子元素 | 依赖的模块名（可多个，用于拓扑排序） |

### 工作方式（配置驱动静态注册）

1. `loader::load_file("modules.xml")` 解析出 `Vec<ModuleConfig>`。
2. `app` 提供 `ModuleRegistry`：把模块名映射到一个“装配工厂”（内部调用 `Container::register`）。
3. `loader::load_modules(&configs, &registry, &container)`：过滤启用项 → 按 `depends` 做 **Kahn 拓扑排序**（同层按 `order`/`name`）→ 依次调用工厂注册。

```rust
use std::path::Path;
use std::sync::Arc;
use container::Container;
use loader::{load_file, load_modules, ModuleRegistry};

let mut reg = ModuleRegistry::new();
reg.add("db", |c| {
    c.register::<Arc<dyn Database>, _>(|_| Ok(Arc::new(SqliteDb::new()) as Arc<dyn Database>));
    Ok(())
});
// ... 登记 user / order ...

let configs = load_file(Path::new("modules.xml"))?;
let c = Container::new();
let report = load_modules(&configs, &reg, &c)?;
assert_eq!(report.loaded, vec!["db", "user", "order"]);
```

> 路径解析：`app` 默认读取 `../modules.xml`（相对其 crate 目录），也可用环境变量 `MODULES_XML` 覆盖。

### 错误处理

| 变体 | 含义 |
| --- | --- |
| `UnknownModule(name)` | 配置启用但注册表无对应工厂 |
| `MissingDependency{module,dependency}` | 依赖了未启用/不存在的模块 |
| `CircularDependency(path)` | 模块依赖存在环 |
| `FactoryFailed(msg)` | 工厂注册自身返回错误 |
| `Io` / `Xml` | 文件读取 / XML 解析错误 |

> 说明：模块随主程序**静态编译**，`libloading` 动态库加载未采用，以换取跨平台稳定性与无 `unsafe`；新增模块只需在 `ModuleRegistry` 登记工厂即可被配置发现。

## 关键机制

### 异步服务工厂
用 `register_async` 注册返回 `Future` 的工厂，首次 `get_async::<T>().await` 时构造。
内部用 `tokio::sync::OnceCell<Result<Erased, DiError>>` 实现**可失败的单次异步初始化**：
并发下只有一个 init future 真正运行（单飞），其余等待并复用同一实例，`Arc::ptr_eq` 成立。
工厂接收 `Arc<Container>`，内部可 `c.get::<Dep>()`（同步）或 `c.get_async::<Dep>().await`（异步）解析依赖。

### 请求级作用域（Scope）
`Container::scope()` 派生一个作用域：**共享父容器单例**，同时持有仅属于本作用域的实例。
查找顺序为「本地 → 父容器」；作用域服务不写入父容器，作用域引用计数归零即释放。
典型用法：每请求 `insert` 一个 `RequestCtx`，再注册依赖该上下文与父单例的请求级服务（支持同步与异步）。

### 异步循环检测
异步解析的环检测基于 `tokio::task_local!` 的解析栈：顶层 `get_async` 建立作用域，
嵌套依赖解析复用同一栈。因为 task-local 随任务跨工作线程迁移仍有效，且各并发顶层解析相互独立，
既能正确检测 `A → B → A` 环（在 `get_or_init` 之前返回 `CircularDependency`，避免 OnceCell 死锁），
又不会在不同请求间产生误报。

### 并发下的单例保证
多线程同时首次查询时，工厂可能被并发执行多次，但 `OnceLock::set` 只保留第一个成功写入的实例，
后续 `get` 一律返回该权威实例。因此**实例唯一性（`Arc::ptr_eq`）始终成立**。
`test_concurrent_first_get_yields_single_instance` 用 16 线程验证此收敛性质。

### 循环依赖检测
解析栈基于 `thread_local`，通过 `CycleGuard` 的 RAII 压栈 / 弹栈实现：

```
A::get() → enter(A)          栈: [A]
  工厂调用 B::get() → enter(B)  栈: [A, B]
    工厂调用 A::get() → enter(A)  A 已在栈中 → Err(CircularDependency([A, B, A]))
```

### 显式传递 vs 全局容器
- **显式传递（推荐）**：依赖可见，测试易替换（换一个 `Container`）。
- **全局容器（谨慎）**：仅在无法传递容器的地方（框架回调、宏展开处）使用；
  代价是依赖不可见、测试需隔离、同进程难以运行多个独立实例。始终可用
  `init_with` / `reset_global` 在测试中替换或清空。

## 错误类型

| 变体 | 含义 |
| --- | --- |
| `NotRegistered(TypeId)` | 服务未注册 |
| `TypeMismatch(TypeId)` | 注册与查询类型不一致 |
| `CircularDependency(Vec<TypeId>)` | 检测到循环依赖，含完整路径 |
| `FactoryFailed(String)` | 工厂返回错误 / 预注入槽位误用工厂 |
