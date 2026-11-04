//! 全单例运行时查询 DI 容器
//!
//! 核心机制：类型擦除注册表 + 延迟初始化 + 循环检测。
//!
//! 设计约定：服务令牌类型 `T` 通常取 `Arc<dyn Trait>`。
//! `get::<T>()` 直接返回 `T`（对 `Arc` 而言即共享同一底层实例）。
//!
//! 三大约束：
//! - 一次创建：同一类型至多构造一次，后续查询复用同一实例。
//! - 线程安全：内部用 `RwLock` + `OnceLock`/`tokio::sync::OnceCell` 保护。
//! - 顺序无关：依赖通延迟初始化解耦，注册顺序不影响正确性。
//!
//! 能力总览：
//! - 同步/异步工厂：`register` / `register_async`，异步基于单飞 `OnceCell`。
//! - 循环检测：同步走 `thread_local` 栈，异步走 `task_local` 栈（跨 .await 仍有）。
//! - 全局容器：`set_global` / `init_with` 等（谨慎使用）。
//! - 请求级作用域：`Container::scope()` 派生，本地优先、委托父、不污染。

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};

/// 容器内部擦除后的值类型（任何服务最终都以这种形式存进槽位）。
type Erased = Arc<dyn Any + Send + Sync>;

/// 装箱的可发送 'static future（异步工厂返回的统一形态）。
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// 将擦除值还原为服务令牌 `T`（对 Arc 而言共享同一底层实例）。
///
/// - `e`：从槽位取出的擦除值（`Arc<dyn Any + Send + Sync>`）。
/// - `type_id`：目标类型 ID，仅用于失败时构造 `TypeMismatch`。
fn downcast_clone<T: Send + Sync + Clone + 'static>(
    e: Erased,
    type_id: TypeId,
) -> Result<T, DiError> {
    // downcast 失败说明注册与查询的令牌类型不一致。
    let arc = e
        .downcast::<T>()
        .map_err(|_| DiError::TypeMismatch(type_id))?;
    // 内层存的是 `Arc<T>`，克隆一份返回（共享同一底层实例）。
    Ok((*arc).clone())
}

// ---------- 错误 ----------
/// 容器统一的错误类型。派生 `Clone` 以便异步失败结果被缓存。
#[derive(Debug, Clone)]
pub enum DiError {
    /// 查询的类型从未注册
    NotRegistered(TypeId),
    /// 注册与查询的令牌类型不一致（downcast 失败）
    TypeMismatch(TypeId),
    /// 解析过程中检测到循环依赖，携带入栈路径
    CircularDependency(Vec<TypeId>),
    /// 工厂自身返回的错误
    FactoryFailed(String),
}

impl std::fmt::Display for DiError {
    /// 以中文描述格式化各错误变体。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiError::NotRegistered(id) => write!(f, "服务未注册: {:?}", id),
            DiError::TypeMismatch(id) => write!(f, "类型不匹配: {:?}", id),
            DiError::CircularDependency(path) => {
                write!(f, "循环依赖: {:?}", path)
            }
            DiError::FactoryFailed(msg) => write!(f, "工厂失败: {}", msg),
        }
    }
}

// 接入标准库错误 trait，便于用 `?` 向上传播。
impl std::error::Error for DiError {}

// ---------- 工厂种类 ----------
/// 一个服务的构造方式（三选一）。
enum Factory {
    /// 同步工厂：给定 &Container 直接构造。
    Sync(Box<dyn Fn(&Container) -> Result<Erased, DiError> + Send + Sync>),
    /// 异步工厂：构造需 await，持有 Arc<Container> 以便解析依赖。
    Async(Box<dyn Fn(Arc<Container>) -> BoxFuture<Result<Erased, DiError>> + Send + Sync>),
    /// 预注入实例：无需工厂，直接就绪。
    Ready(Erased),
}

// ---------- 类型擦除槽位 ----------
/// 每个服务一个槽位，支持延迟初始化（同步/异步各一份缓存）。
struct Slot {
    /// 同步已构造实例（OnceLock 保证仅初始化一次）。
    value: OnceLock<Erased>,
    /// 异步已构造实例（get_or_init 支持可失败单次初始化）。
    async_value: tokio::sync::OnceCell<Result<Erased, DiError>>,
    /// 构造该服务所用的工厂（同步/异步/现成）。
    factory: Factory,
}

impl Slot {
    /// 由同步工厂构造一个空槽位（尚未初始化）。
    fn new<F>(factory: F) -> Self
    where
        F: Fn(&Container) -> Result<Erased, DiError> + Send + Sync + 'static,
    {
        // 工厂延迟到首次 get 时才执行，两个缓存初始均为空。
        Self {
            value: OnceLock::new(),
            async_value: tokio::sync::OnceCell::new(),
            factory: Factory::Sync(Box::new(factory)),
        }
    }

    /// 由异步工厂构造一个空槽位（尚未初始化）。
    fn new_async<F>(factory: F) -> Self
    where
        F: Fn(Arc<Container>) -> BoxFuture<Result<Erased, DiError>> + Send + Sync + 'static,
    {
        // async_value 延迟到首次 get_async 时由 OnceCell 单飞填充。
        Self {
            value: OnceLock::new(),
            async_value: tokio::sync::OnceCell::new(),
            factory: Factory::Async(Box::new(factory)),
        }
    }

    /// 已注入的实例（不经过工厂）；直接写入 value 缓存。
    fn from_value(value: Erased) -> Self {
        // 先把实例克隆一份作为 Ready 负载，再立即填充 value。
        let slot = Self {
            value: OnceLock::new(),
            async_value: tokio::sync::OnceCell::new(),
            factory: Factory::Ready(value.clone()),
        };
        // 预注入实例：立即将 value 填充，后续 get 直接命中缓存。
        let _ = slot.value.set(value);
        slot
    }

    /// 同步获取或初始化。若已缓存直接返回；否则根据工厂种类构造。
    fn get_or_init(&self, container: &Container) -> Result<Erased, DiError> {
        // 快路径：已初始化则直接克隆返回。
        if let Some(v) = self.value.get() {
            return Ok(v.clone());
        }
        match &self.factory {
            Factory::Sync(f) => {
                let created = f(container)?;
                // 并发下可能被其他线程抢先 set，以先者为权威。
                let _ = self.value.set(created);
                Ok(self.value.get().unwrap().clone())
            }
            Factory::Ready(v) => Ok(v.clone()),
            // 异步工厂不能同步构造；若已被 get_async 填充则可取。
            Factory::Async(_) => match self.async_value.get() {
                Some(Ok(v)) => Ok(v.clone()),
                _ => Err(DiError::FactoryFailed(
                    "异步服务需通过 get_async 查询".into(),
                )),
            },
        }
    }
}

// ---------- 容器 ----------
/// DI 容器：服务令牌类型 -> 共享槽位。可多线程共享（`Send + Sync`）。
pub struct Container {
    /// 读写锁保护的注册表：TypeId -> 槽位。读多写少，适合 RwLock。
    slots: RwLock<HashMap<TypeId, Arc<Slot>>>,
}

impl Container {
    /// 新建一个空容器，返回 `Arc<Self>`（便于工厂/异步解析时克隆传递）。
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            slots: RwLock::new(HashMap::new()),
        })
    }

    // ====== 注册 API ======

    /// 注册工厂（延迟初始化，推荐）
    ///
    /// `T` 为服务令牌类型，通常是 `Arc<dyn Trait>`。工厂返回 `T`。
    pub fn register<T, F>(&self, factory: F)
    where
        T: Send + Sync + Clone + 'static,
        F: Fn(&Container) -> Result<T, DiError> + Send + Sync + 'static,
    {
        // 包装用户工厂：返回值再装箱为 Erased（`Arc<T>`）。
        let slot = Arc::new(Slot::new(move |c| {
            factory(c).map(|v| Arc::new(v) as Erased)
        }));
        // 以令牌类型的 TypeId 为键写入注册表（同键覆盖）。
        self.slots
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), slot);
    }

    /// 注册已有实例（立即就绪）
    pub fn register_instance<T: Send + Sync + Clone + 'static>(&self, value: T) {
        // 直接构造 Ready 槽位，无需工厂。
        let slot = Arc::new(Slot::from_value(Arc::new(value) as Erased));
        // 同样以 TypeId 为键入表（同键覆盖）。
        self.slots
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), slot);
    }

    /// 注册异步工厂（延迟初始化，构造需 await）
    ///
    /// 工厂接收 `Arc<Container>` 以便内部 `c.get_async::<Dep>().await` 解析依赖。
    pub fn register_async<T, F, Fut>(&self, factory: F)
    where
        T: Send + Sync + Clone + 'static,
        F: Fn(Arc<Container>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, DiError>> + Send + 'static,
    {
        // 用 Arc 包工厂，使其可在多次初始化中被克隆（Fn 而非 FnOnce）。
        let factory = Arc::new(factory);
        let slot = Arc::new(Slot::new_async(move |c: Arc<Container>| {
            let factory = factory.clone();
            Box::pin(async move { (factory)(c).await.map(|v| Arc::new(v) as Erased) })
        }));
        // 以令牌 TypeId 为键写入注册表（同键覆盖）。
        self.slots
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), slot);
    }

    // ====== 查询 API ======

    /// 查询服务（首次触发构造，之后复用同一实例）
    pub fn get<T: Send + Sync + Clone + 'static>(&self) -> Result<T, DiError> {
        let type_id = TypeId::of::<T>();

        // 1. 循环检测：压栈
        let _guard = CycleGuard::enter(type_id)?;

        // 2. 查找槽位
        let slot = self
            .slots
            .read()
            .unwrap()
            .get(&type_id)
            .cloned()
            .ok_or(DiError::NotRegistered(type_id))?;

        // 3. 取或初始化：命中缓存则复用，否则由工厂首次构造。
        let any = slot.get_or_init(self)?;

        // 4. 类型还原，并克隆出令牌（对 Arc 而言共享同一底层实例）。
        let arc: Arc<T> = any
            .downcast::<T>()
            .map_err(|_| DiError::TypeMismatch(type_id))?;
        Ok((*arc).clone())
    }

    /// 可选查询：未注册返回 None。
    pub fn try_get<T: Send + Sync + Clone + 'static>(&self) -> Option<T> {
        // 丢弃错误信息，只关心“能否拿到”。
        self.get::<T>().ok()
    }

    /// 已注册检查：仅看注册表中是否存在该类型的槽位（不代表已构造）。
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.slots
            .read()
            .unwrap()
            .contains_key(&TypeId::of::<T>())
    }

    /// 异步查询服务（首次触发 await 构造，之后复用同一实例）
    ///
    /// 需在 tokio 运行时上下文中调用。循环检测基于 `task_local`，
    /// 随任务跨工作线程迁移仍正确；并发顶层解析各自独立，不会误报。
    pub async fn get_async<T>(self: &Arc<Self>) -> Result<T, DiError>
    where
        T: Send + Sync + Clone + 'static,
    {
        // 取令牌类型 ID 作为注册表键。
        let type_id = TypeId::of::<T>();
        // 先取槽位（写锁短暂），后续初始化在锁外进行，避免持锁 await。
        let slot = self
            .slots
            .read()
            .unwrap()
            .get(&type_id)
            .cloned()
            .ok_or(DiError::NotRegistered(type_id))?;

        // 缓存快路径（已由任一同步/异步初始化填充）
        if let Some(Ok(v)) = slot.async_value.get() {
            return downcast_clone(v.clone(), type_id);
        }
        if let Some(v) = slot.value.get() {
            return downcast_clone(v.clone(), type_id);
        }

        // 顶层解析时建立 task_local 作域，嵌套解析复用同一栈
        let fut = self.resolve_async_slot(type_id, &slot);
        let erased = if async_in_scope() {
            // 已在作用域内：直接 await，不重复建栈。
            fut.await?
        } else {
            // 顶层：新建一个空的解析栈作用域包裹本次 await。
            ASYNC_RESOLVE
                .scope(std::cell::RefCell::new(Vec::new()), fut)
                .await?
        };
        // 把擦除值还原为服务令牌 T。
        downcast_clone(erased, type_id)
    }

    /// 在已建立的异步解析作用域内解析某个槽位（带循环检测 + 单飞初始化）。
    async fn resolve_async_slot(
        self: &Arc<Self>,
        type_id: TypeId,
        slot: &Arc<Slot>,
    ) -> Result<Erased, DiError> {
        // 压栈做循环检测（命中环则在初始化前返回 Err，避免死锁）。
        let _guard = AsyncCycleGuard::enter(type_id)?;
        // 双检：进入作用域后可能已被并发初始化，先查缓存。
        if let Some(Ok(v)) = slot.async_value.get() {
            return Ok(v.clone());
        }
        match &slot.factory {
            // 异步工厂：get_or_init 保证并发下仅初始化一次（单飞）。
            Factory::Async(f) => {
                let r = slot
                    .async_value
                    .get_or_init(|| {
                        let c = self.clone();
                        async move { (f)(c).await }
                    })
                    .await
                    .clone();
                r
            }
            // 同步工厂：直接构造并把结果同步到 async_value 缓存。
            Factory::Sync(f) => {
                let v = f(self)?;
                // 回写到异步缓存，供后续 get_async 快路径命中。
                let _ = slot.async_value.set(Ok(v.clone()));
                Ok(v)
            }
            // 现成实例：无需工厂，直接克隆返回。
            Factory::Ready(v) => Ok(v.clone()),
        }
    }

    /// 派生一个请求级作用域（父容器单例被共享）。
    pub fn scope(self: &Arc<Self>) -> Scope {
        // 作用域持父引用 + 自己的本地表；克隆作用域共享同一 ScopeInner。
        Scope {
            inner: Arc::new(ScopeInner {
                // 克隆一份 Arc 作为父引用，保证作用域期间父不被释放。
                parent: self.clone(),
                local: RwLock::new(HashMap::new()),
            }),
        }
    }
}

/// `Default` 等价于建立一个空容器（但返回 Self 而非 Arc）。
impl Default for Container {
    fn default() -> Self {
        // 与 new() 相同的内部结构，仅返回类型不同（不包 Arc）。
        Self {
            slots: RwLock::new(HashMap::new()),
        }
    }
}

// ---------- 全局容器（谨慎使用） ----------
//
// 仅在无法显式传递容器的地方（框架回调、宏展开处）使用。
// 代价：依赖不可见、测试需隔离、同进程难以跑两个独立实例。
// 因此额外提供 set_global/init_with 以便测试替换。

// 进程级全局容器：用 OnceLock 保证只初始化一次，内层 RwLock<Option> 支持测试替换。
static GLOBAL: OnceLock<RwLock<Option<Arc<Container>>>> = OnceLock::new();

/// 取得全局容器的静态锁（首次调用时惰性建立）。
fn global_slot() -> &'static RwLock<Option<Arc<Container>>> {
    GLOBAL.get_or_init(|| RwLock::new(None))
}

/// 安装全局容器：仅当尚未初始化时设置。
/// 返回 `true` 表示本次设置生效；`false` 表示已存在（不覆盖）。
pub fn set_global(container: Arc<Container>) -> bool {
    // 取写锁；仅当尚未初始化时写入，避免覆盖已有容器。
    let mut g = global_slot().write().unwrap();
    if g.is_none() {
        *g = Some(container);
        true
    } else {
        // 已存在则丢弃传入容器，返回 false。
        false
    }
}

/// 强制替换全局容器（供测试注入/替换使用）。
pub fn init_with(container: Arc<Container>) {
    *global_slot().write().unwrap() = Some(container);
}

/// 惰性初始化全局容器：若尚未设置则调用 `builder` 构造并安装。
/// 返回 `true` 表示本次完成初始化，`false` 表示已存在。
/// 注意：`builder` 在写锁内执行，应避免在其中再访问全局容器。
pub fn get_or_init_global<F>(builder: F) -> bool
where
    F: FnOnce() -> Arc<Container>,
{
    // 写锁内判断，避免竞态下重复初始化。
    let mut g = global_slot().write().unwrap();
    if g.is_some() {
        return false;
    }
    // 首次初始化：调用 builder 构造并安装。
    *g = Some(builder());
    true
}

/// 获取全局容器；未初始化返回 `None`。
pub fn try_global() -> Option<Arc<Container>> {
    global_slot().read().unwrap().clone()
}

/// 获取全局容器；未初始化则 panic。
pub fn global() -> Arc<Container> {
    try_global().expect("全局容器未初始化，请先调用 container::set_global / init_with")
}

/// 清空全局容器（测试隔离用）。
pub fn reset_global() {
    *global_slot().write().unwrap() = None;
}

// ---------- 循环检测 ----------
// 同步解析栈：每线程独立，记录当前正在解析的类型链。
thread_local! {
    static RESOLVE_STACK: std::cell::RefCell<Vec<TypeId>> =
        std::cell::RefCell::new(Vec::new());
}

/// RAII 守卫：进入时压栈，离开（Drop）时弹栈。
struct CycleGuard {
    /// 本守卫压入的类型（drop 时据此弹栈）。
    #[allow(dead_code)]
    type_id: TypeId,
}

impl CycleGuard {
    /// 尝试将 `type_id` 压栈；若已在栈中则说明成环，返回错误。
    fn enter(type_id: TypeId) -> Result<Self, DiError> {
        // 在线程本地解析栈上检查并压入当前类型。
        RESOLVE_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            // 已存在 => 检测到环，回传当前路径 + 本节点。
            if stack.contains(&type_id) {
                let mut path = stack.clone();
                path.push(type_id);
                return Err(DiError::CircularDependency(path));
            }
            stack.push(type_id);
            Ok(CycleGuard { type_id })
        })
    }
}

impl Drop for CycleGuard {
    /// 作用域结束时弹栈，保证同一线程后续解析不受影响。
    fn drop(&mut self) {
        RESOLVE_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

// ---------- 异步循环检测（task_local，随任务跨线程迁移仍有效） ----------
// 不能用 thread_local（跨 .await 可能切换工作线程），故用 task_local。
tokio::task_local! {
    static ASYNC_RESOLVE: std::cell::RefCell<Vec<TypeId>>;
}

/// 当前任务是否已处于异步解析作用域中（据此区分顶层/嵌套，避免重复建作域）。
fn async_in_scope() -> bool {
    ASYNC_RESOLVE.try_with(|_| ()).is_ok()
}

/// 异步版 RAII 守卫：在当前任务的解析栈上压/弹类型。
struct AsyncCycleGuard {
    /// 本守卫压入的类型（drop 时据此弹栈）。
    #[allow(dead_code)]
    type_id: TypeId,
}

impl AsyncCycleGuard {
    /// 尝试将 `type_id` 压入当前任务的异步解析栈；已在栈中则成环。
    fn enter(type_id: TypeId) -> Result<Self, DiError> {
        ASYNC_RESOLVE.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&type_id) {
                let mut path = stack.clone();
                path.push(type_id);
                return Err(DiError::CircularDependency(path));
            }
            stack.push(type_id);
            Ok(AsyncCycleGuard { type_id })
        })
    }
}

impl Drop for AsyncCycleGuard {
    fn drop(&mut self) {
        // 仅在作用域内创建，必在作用域内销毁；try_with 防御性忽略异常情形
        let _ = ASYNC_RESOLVE.try_with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

// ---------- 请求级作用域容器 ----------
//
// 复用父容器的单例，同时持有“请求级”实例：作用域引用计数归零即释放。
// 查找顺序：本地 -> 父容器。支持同步与异步两类服务。

/// 作用域内的服务构造方式（与容器的 Factory 对应，但闭包接收 &Scope/Scope）。
enum ScopeFactory {
    /// 同步工厂：给定 &Scope 直接构造。
    Sync(Box<dyn Fn(&Scope) -> Result<Erased, DiError> + Send + Sync>),
    /// 异步工厂：构造需 await，持有 Scope 以便递归解析。
    Async(Box<dyn Fn(Scope) -> BoxFuture<Result<Erased, DiError>> + Send + Sync>),
    /// 预注入实例：直接就绪。
    Ready(Erased),
}

/// 作用域本地槽位：与容器 Slot 结构相同，但工厂面对的是 Scope。
struct ScopeSlot {
    /// 同步已构造实例。
    value: OnceLock<Erased>,
    /// 异步已构造实例（可失败的单次初始化）。
    async_value: tokio::sync::OnceCell<Result<Erased, DiError>>,
    /// 构造本实例的工厂。
    factory: ScopeFactory,
}

impl ScopeSlot {
    /// 由同步工厂创建空槽位。
    fn new<F>(factory: F) -> Self
    where
        F: Fn(&Scope) -> Result<Erased, DiError> + Send + Sync + 'static,
    {
        Self {
            value: OnceLock::new(),
            async_value: tokio::sync::OnceCell::new(),
            factory: ScopeFactory::Sync(Box::new(factory)),
        }
    }

    /// 由异步工厂创建空槽位。
    fn new_async<F>(factory: F) -> Self
    where
        F: Fn(Scope) -> BoxFuture<Result<Erased, DiError>> + Send + Sync + 'static,
    {
        Self {
            value: OnceLock::new(),
            async_value: tokio::sync::OnceCell::new(),
            factory: ScopeFactory::Async(Box::new(factory)),
        }
    }

    /// 由现成实例创建已就绪槽位。
    fn from_value(value: Erased) -> Self {
        let slot = Self {
            value: OnceLock::new(),
            async_value: tokio::sync::OnceCell::new(),
            factory: ScopeFactory::Ready(value.clone()),
        };
        // 预注入：立即填充 value。
        let _ = slot.value.set(value);
        slot
    }

    /// 同步获取或初始化（并发下以先 set 成功者为权威）。
    fn get_or_init(&self, scope: &Scope) -> Result<Erased, DiError> {
        // 快路径：已初始化则直接克隆返回，无锁竞争以外的开销。
        if let Some(v) = self.value.get() {
            return Ok(v.clone());
        }
        match &self.factory {
            ScopeFactory::Sync(f) => {
                let created = f(scope)?;
                // 并发下可能被其他线程抢先 set，仅保留首个成功者。
                let _ = self.value.set(created);
                Ok(self.value.get().unwrap().clone())
            }
            ScopeFactory::Ready(v) => Ok(v.clone()),
            // 异步工厂无法同步构造；除非已被 get_async 填充否则报错。
            ScopeFactory::Async(_) => match self.async_value.get() {
                Some(Ok(v)) => Ok(v.clone()),
                _ => Err(DiError::FactoryFailed(
                    "作用域异步服务需 get_async 查询".into(),
                )),
            },
        }
    }
}

/// 作用域的内部共享数据：父容器引用 + 本地注册表。
struct ScopeInner {
    /// 父容器：本地未命中时逐级委托到这里。
    parent: Arc<Container>,
    /// 本作用域独有的槽位表。
    local: RwLock<HashMap<TypeId, Arc<ScopeSlot>>>,
}

/// 请求级作用域：克隆廉价（共享 Arc），作用域内服务不污染父容器。
#[derive(Clone)]
pub struct Scope {
    /// 共享的内部数据（父引用 + 本地表）；克隆仅增引用计数。
    inner: Arc<ScopeInner>,
}

// Scope 的实现：本地优先、未命中逐级委托父容器。
impl Scope {
    /// 父容器引用
    pub fn parent(&self) -> &Arc<Container> {
        // 直接返回内部共享数据持有的父引用。
        &self.inner.parent
    }

    /// 注入请求级实例（立即就绪）
    pub fn insert<T: Send + Sync + Clone + 'static>(&self, value: T) {
        // 包装为 Ready 槽位，键为令牌 TypeId，仅写入本地表（不影响父）。
        let slot = Arc::new(ScopeSlot::from_value(Arc::new(value) as Erased));
        self.inner
            .local
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), slot);
    }

    /// 注册请求级同步工厂
    pub fn register<T, F>(&self, factory: F)
    where
        T: Send + Sync + Clone + 'static,
        F: Fn(&Scope) -> Result<T, DiError> + Send + Sync + 'static,
    {
        // 包装用户工厂：返回值再装箱为 Erased；工厂面针对 Scope 而非 Container。
        let slot = Arc::new(ScopeSlot::new(move |s| {
            factory(s).map(|v| Arc::new(v) as Erased)
        }));
        self.inner
            .local
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), slot);
    }

    /// 注册请求级异步工厂
    pub fn register_async<T, F, Fut>(&self, factory: F)
    where
        T: Send + Sync + Clone + 'static,
        F: Fn(Scope) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, DiError>> + Send + 'static,
    {
        // 用 Arc 包工厂以便多次初始化时克隆；闭包内部 Box::pin 成统一 BoxFuture。
        let factory = Arc::new(factory);
        let slot = Arc::new(ScopeSlot::new_async(move |s| {
            let factory = factory.clone();
            Box::pin(async move { (factory)(s).await.map(|v| Arc::new(v) as Erased) })
        }));
        self.inner
            .local
            .write()
            .unwrap()
            .insert(TypeId::of::<T>(), slot);
    }

    /// 本作用域是否直接注册了该类型（不包含父容器）
    pub fn contains_local<T: 'static>(&self) -> bool {
        // 只查本地表，不逐级委托到父容器。
        self.inner
            .local
            .read()
            .unwrap()
            .contains_key(&TypeId::of::<T>())
    }

    /// 同步查询：本地优先，未命中委托父容器。
    pub fn get<T>(&self) -> Result<T, DiError>
    where
        T: Send + Sync + Clone + 'static,
    {
        let type_id = TypeId::of::<T>();
        // 先在读锁下拿本地槽位克隆，避免后续持锁调用工厂。
        let local_slot = self.inner.local.read().unwrap().get(&type_id).cloned();
        if let Some(slot) = local_slot {
            // 命中本地槽位：仅解析本地时才压栈做循环检测。
            let _guard = CycleGuard::enter(type_id)?;
            let erased = slot.get_or_init(self)?;
            return downcast_clone(erased, type_id);
        }
        // 本地无 → 委托父容器（父的单例被所有作用域共享）。
        self.inner.parent.get::<T>()
    }

    /// 异步查询：本地优先，未命中委托父容器。
    pub async fn get_async<T>(&self) -> Result<T, DiError>
    where
        T: Send + Sync + Clone + 'static,
    {
        // 取令牌类型 ID，先尝试从本地表拿槽位克隆。
        let type_id = TypeId::of::<T>();
        let local_slot = self.inner.local.read().unwrap().get(&type_id).cloned();
        if let Some(slot) = local_slot {
            // 快路径：本地已初始化（异步或同步）则直接返回。
            if let Some(Ok(v)) = slot.async_value.get() {
                return downcast_clone(v.clone(), type_id);
            }
            if let Some(v) = slot.value.get() {
                return downcast_clone(v.clone(), type_id);
            }
            // 顶层解析时建 task_local 作用域，嵌套复用同一栈。
            let fut = self.resolve_async_slot(type_id, &slot);
            let erased = if async_in_scope() {
                fut.await?
            } else {
                ASYNC_RESOLVE
                    .scope(std::cell::RefCell::new(Vec::new()), fut)
                    .await?
            };
            return downcast_clone(erased, type_id);
        }
        // 本地无 → 委托父容器的异步查询。
        self.inner.parent.get_async::<T>().await
    }

    /// 在已建立的异步解析作用域内解析本地槽位（带循环检测 + 单飞）。
    async fn resolve_async_slot(
        &self,
        type_id: TypeId,
        slot: &Arc<ScopeSlot>,
    ) -> Result<Erased, DiError> {
        // 压栈做异步循环检测（task_local 栈，跨 .await 仍有效）。
        let _guard = AsyncCycleGuard::enter(type_id)?;
        // 双检：已被并发初始化则直接复用。
        if let Some(Ok(v)) = slot.async_value.get() {
            return Ok(v.clone());
        }
        match &slot.factory {
            // 异步工厂：get_or_init 保证同一作域内仅初始化一次。
            ScopeFactory::Async(f) => {
                let r = slot
                    .async_value
                    .get_or_init(|| {
                        // 克隆一份 Scope 传入工厂，供其递归解析依赖。
                        let s = self.clone();
                        async move { (f)(s).await }
                    })
                    .await
                    .clone();
                r
            }
            // 同步工厂：直接构造并同步到 async_value。
            ScopeFactory::Sync(f) => {
                let v = f(self)?;
                let _ = slot.async_value.set(Ok(v.clone()));
                Ok(v)
            }
            ScopeFactory::Ready(v) => Ok(v.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 测试用服务接口：只有一个 name 方法，便于断言实例来源。
    trait Db: Send + Sync {
        fn name(&self) -> &str;
    }

    // 真实实现：name 返回 "real"。
    struct RealDb;
    impl Db for RealDb {
        fn name(&self) -> &str {
            "real"
        }
    }

    // Mock 实现：name 返回 "mock"，用于验证依赖可替换。
    struct MockDb;
    impl Db for MockDb {
        fn name(&self) -> &str {
            "mock"
        }
    }

    // 两个空标记接口 + 实现，专用于构造循环依赖场景。
    trait A: Send + Sync {}
    trait B: Send + Sync {}
    struct ImplA;
    impl A for ImplA {}
    struct ImplB;
    impl B for ImplB {}

    /// 预注入实例应为单例：两次 get 拿到同一地址。
    #[test]
    fn test_instance_singleton() {
        let c = Container::new();
        // 直接注入一个 MockDb 实例（走 Ready 槽位）。
        c.register_instance::<Arc<dyn Db>>(Arc::new(MockDb));

        let db = c.get::<Arc<dyn Db>>().unwrap();
        assert_eq!(db.name(), "mock");

        // 再次查询：ptr_eq 验证是同一实例。
        let db2 = c.get::<Arc<dyn Db>>().unwrap();
        assert!(Arc::ptr_eq(&db, &db2));
    }

    /// 懒工厂：首次 get 时才构造，得到工厂返回的实例。
    #[test]
    fn test_lazy_factory() {
        let c = Container::new();
        // 注册一个返回 RealDb 的同步工厂；此时尚未执行。
        c.register::<Arc<dyn Db>, _>(|_| Ok(Arc::new(RealDb) as Arc<dyn Db>));
        // 首次 get 触发工厂构造，得到 "real"。
        let db = c.get::<Arc<dyn Db>>().unwrap();
        assert_eq!(db.name(), "real");
    }

    /// 循环依赖检测：A→B→A 应报 CircularDependency。
    #[test]
    fn test_circular_dependency_detected() {
        let c = Container::new();
        // 构造 A→B→A 的相互依赖环。
        c.register::<Arc<dyn A>, _>(|c| {
            let _ = c.get::<Arc<dyn B>>()?; // A 依赖 B
            Ok(Arc::new(ImplA) as Arc<dyn A>)
        });
        c.register::<Arc<dyn B>, _>(|c| {
            let _ = c.get::<Arc<dyn A>>()?; // B 依赖 A → 环
            Ok(Arc::new(ImplB) as Arc<dyn B>)
        });

        // 解析 A 时应因成环而返回错误。
        let err = match c.get::<Arc<dyn A>>() {
            Err(e) => e,
            Ok(_) => panic!("应检测到循环依赖"),
        };
        assert!(matches!(err, DiError::CircularDependency(_)));
    }

    /// try_get / contains：未注册时均为空，注册后均命中。
    #[test]
    fn test_try_get_and_contains() {
        let c = Container::new();
        // 未注册：contains 为假、try_get 为 None。
        assert!(!c.contains::<Arc<dyn Db>>());
        assert!(c.try_get::<Arc<dyn Db>>().is_none());
        // 注册后：两者均命中。
        c.register_instance::<Arc<dyn Db>>(Arc::new(MockDb));
        assert!(c.contains::<Arc<dyn Db>>());
        assert!(c.try_get::<Arc<dyn Db>>().is_some());
    }

    /// 多线程并发首次查询：无论工厂是否被并发执行多次，
    /// 最终所有线程必须拿到同一个实例（单例保证）。
    #[test]
    fn test_concurrent_first_get_yields_single_instance() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;

        // 共享容器，供所有线程访问（验证 Container: Send + Sync）
        fn assert_send_sync<T: Send + Sync>(_: &T) {}

        let c = Container::new();
        assert_send_sync(&c);

        // 工厂内计数器 + sleep 放大并发窗口。
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let calls_for_factory = calls.clone();
        c.register::<Arc<dyn Db>, _>( move |_| {
            calls_for_factory.fetch_add(1, Ordering::SeqCst);
            // 放大并发窗口，制造更多线程同时命中未初始化分支
            thread::sleep(std::time::Duration::from_millis(20));
            Ok(Arc::new(RealDb) as Arc<dyn Db>)
        });

        // 生成 16 个线程并发首次 get。
        let mut handles = Vec::new();
        for _ in 0..16 {
            let c = c.clone();
            handles.push(thread::spawn(move || c.get::<Arc<dyn Db>>().unwrap()));
        }

        let results: Vec<Arc<dyn Db>> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();

        let first = &results[0];
        assert!(
            results.iter().all(|r| Arc::ptr_eq(first, r)),
            "并发首次查询必须收敛到同一实例"
        );
        assert!(first.name() == "real");
        // 工厂可能因竞争被调用 >= 1 次，但保留的实例唯一。
        assert!(calls.load(Ordering::SeqCst) >= 1);
    }

    /// 全局容器：设置 -> 查询 -> 替换 -> 清空。
    /// 因全局状态跨测试共享，这里串行完成一次完整生命周期。
    #[test]
    fn test_global_container() {
        // 初始清空：try_global 应为 None。
        reset_global();
        assert!(try_global().is_none());

        // 新建并注册后设为全局，首次设置应成功。
        let c = Container::new();
        c.register_instance::<Arc<dyn Db>>(Arc::new(RealDb));
        assert!(set_global(c.clone()));
        // 已存在时不覆盖
        assert!(!set_global(Container::new()));
        assert_eq!(global().get::<Arc<dyn Db>>().unwrap().name(), "real");

        // 强制替换（测试隔离）
        let mock = Container::new();
        mock.register_instance::<Arc<dyn Db>>(Arc::new(MockDb));
        init_with(mock);
        assert_eq!(global().get::<Arc<dyn Db>>().unwrap().name(), "mock");

        // 末尾再清空，恢复干净状态供其他测试。
        reset_global();
        assert!(try_global().is_none());
    }

    // ===== 异步工厂 =====

    // 异步服务接口：模拟一个需 await 建立的远程客户端。
    trait Remote: Send + Sync {
        fn url(&self) -> &str;
    }
    // Remote 的实现：持有并回显一个 url。
    struct RemoteImpl {
        url: String,
    }
    impl Remote for RemoteImpl {
        fn url(&self) -> &str {
            &self.url
        }
    }

    /// 异步工厂：await 构造并可依赖同步服务；且同样单例复用。
    #[tokio::test]
    async fn test_async_factory_and_singleton() {
        let c = Container::new();
        // 先注册一个同步服务供异步工厂依赖。
        c.register::<Arc<dyn Db>, _>(|_| Ok(Arc::new(RealDb) as Arc<dyn Db>));
        // 异步工厂：await 构造，并依赖同步服务
        c.register_async::<Arc<dyn Remote>, _, _>(|c| {
            Box::pin(async move {
                let db = c.get::<Arc<dyn Db>>()?; // 同步依赖
                tokio::task::yield_now().await; // 模拟异步 IO
                let _ = db.name();
                Ok(Arc::new(RemoteImpl {
                    url: "https://remote".into(),
                }) as Arc<dyn Remote>)
            })
        });

        // 首次 get_async 触发 await 构造。
        let r = c.get_async::<Arc<dyn Remote>>().await.unwrap();
        assert_eq!(r.url(), "https://remote");
        // 第二次命中缓存，ptr_eq 验证单例。
        let r2 = c.get_async::<Arc<dyn Remote>>().await.unwrap();
        assert!(Arc::ptr_eq(&r, &r2), "异步服务也应单例复用");
    }

    /// 并发单飞：16 个任务并发首次 get_async，工厂恰好只执行一次。
    #[tokio::test]
    async fn test_async_concurrent_single_instance() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let c = Container::new();
        // 用原子计数记录工厂真实执行次数。
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        c.register_async::<Arc<dyn Remote>, _, _>(move |_| {
            let calls2 = calls2.clone();
            Box::pin(async move {
                calls2.fetch_add(1, Ordering::SeqCst);
                // sleep 拉宽并发窗口，验证单飞不会重复进入。
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                Ok(Arc::new(RemoteImpl {
                    url: "x".into(),
                }) as Arc<dyn Remote>)
            })
        });

        // 并发 spawn 16 个任务同时首次 get_async。
        let mut handles = Vec::new();
        for _ in 0..16 {
            let c = c.clone();
            handles.push(tokio::spawn(async move {
                c.get_async::<Arc<dyn Remote>>().await.unwrap()
            }));
        }
        // 收集全部结果，验证均为同一实例。
        let mut results = Vec::new();
        for h in handles {
            results.push(h.await.unwrap());
        }
        let first = &results[0];
        assert!(results.iter().all(|r| Arc::ptr_eq(first, r)));
        // tokio OnceCell 单飞：init 应只真正运行一次
        assert_eq!(calls.load(Ordering::SeqCst), 1, "异步初始化应单飞");
    }

    /// 异步循环检测：X⇄Y 互为异步依赖应报 CircularDependency。
    #[tokio::test]
    async fn test_async_circular_detected() {
        // 两个空标记接口 + 实现，专用于异步成环。
        trait X: Send + Sync {}
        trait Y: Send + Sync {}
        struct IX;
        impl X for IX {}
        struct IY;
        impl Y for IY {}

        let c = Container::new();
        // X 的异步工厂先 await Y。
        c.register_async::<Arc<dyn X>, _, _>(|c| {
            Box::pin(async move {
                let _ = c.get_async::<Arc<dyn Y>>().await?;
                Ok(Arc::new(IX) as Arc<dyn X>)
            })
        });
        // Y 又 await X → 构成环。
        c.register_async::<Arc<dyn Y>, _, _>(|c| {
            Box::pin(async move {
                let _ = c.get_async::<Arc<dyn X>>().await?;
                Ok(Arc::new(IY) as Arc<dyn Y>)
            })
        });

        // 顶层解析 X 应因 task_local 循环检测而返回错误。
        match c.get_async::<Arc<dyn X>>().await {
            Err(DiError::CircularDependency(_)) => {}
            other => panic!("应检测到异步循环依赖, got {:?}", other.err()),
        }
    }

    // ===== 请求级作用域 =====

    /// 作用域委托：本地未注册的类型回退到父单例（同一实例）。
    #[test]
    fn test_scope_delegates_to_parent() {
        let c = Container::new();
        // 父容器注册同步 Db 工厂。
        c.register::<Arc<dyn Db>, _>(|_| Ok(Arc::new(RealDb) as Arc<dyn Db>));
        // 从父容器派生一个作用域。
        let scope = c.scope();
        // 本地未注册 -> 委托父单例
        let db = scope.get::<Arc<dyn Db>>().unwrap();
        assert_eq!(db.name(), "real");
        // 与父 get 结果 ptr_eq，证明共享同一单例。
        assert!(Arc::ptr_eq(&db, &c.get::<Arc<dyn Db>>().unwrap()));
    }

    /// 作用域隔离：不同 Scope 插入的同类型实例互不可见，且不污染父。
    #[test]
    fn test_scope_request_instance_isolation() {
        // 请求级令牌：克隆廉价的包装 id。
        #[derive(Clone)]
        struct ReqId(u64);

        let c = Container::new();
        // 从同一父容器派生两个独立作用域。
        let s1 = c.scope();
        let s2 = c.scope();
        // 各自插入同类型不同值，互不覆盖。
        s1.insert::<ReqId>(ReqId(1));
        s2.insert::<ReqId>(ReqId(2));

        assert_eq!(s1.get::<ReqId>().unwrap().0, 1);
        assert_eq!(s2.get::<ReqId>().unwrap().0, 2);
        // 作用域实例不污染父容器
        assert!(!c.contains::<ReqId>());
    }

    /// 作用域异步服务：能同时拿到请求级 Ctx 与父单例 Db，且不污染父。
    #[tokio::test]
    async fn test_scope_async_and_parent_singleton() {
        // 请求级上下文：携带一个 token。
        #[derive(Clone)]
        struct Ctx {
            token: String,
        }

        let c = Container::new();
        // 父容器注册同步 Db 单例。
        c.register::<Arc<dyn Db>, _>(|_| Ok(Arc::new(RealDb) as Arc<dyn Db>));

        let scope = c.scope();
        // 向作用域注入请求级 Ctx。
        scope.insert::<Ctx>(Ctx {
            token: "req-abc".into(),
        });
        // 作用域异步服务：依赖请求级 Ctx 与父单例 Db
        scope.register_async::<Arc<dyn Remote>, _, _>(|s| {
            Box::pin(async move {
                let ctx = s.get::<Ctx>()?;
                let db = s.get::<Arc<dyn Db>>()?;
                tokio::task::yield_now().await;
                let _ = db.name();
                Ok(Arc::new(RemoteImpl {
                    url: format!("remote://{}", ctx.token),
                }) as Arc<dyn Remote>)
            })
        });

        let r = scope.get_async::<Arc<dyn Remote>>().await.unwrap();
        assert_eq!(r.url(), "remote://req-abc");
        // 同一作用域内单例
        let r2 = scope.get_async::<Arc<dyn Remote>>().await.unwrap();
        assert!(Arc::ptr_eq(&r, &r2));
        // 作用域服务不污染父容器
        assert!(!c.contains::<Arc<dyn Remote>>());
    }
}
