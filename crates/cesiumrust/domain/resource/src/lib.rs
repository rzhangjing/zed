//! cesium-resource：资源管理与请求调度。
//!
//! 领域层 —— 纯 Rust，无框架依赖，无网络 IO。
//!
//! 覆盖资源（Resource）、请求（Request）、请求调度器（RequestScheduler）、
//! 代理（Proxy）、Ion 端点、data URI 解码、优先级与统计等能力。
//!
//! # 架构
//!
//! 本 crate 提供资源管理的 **纯领域逻辑**：
//! URL 构造、查询参数处理、代理策略、data URI 解码、
//! Ion 端点构建、带优先级/限流的请求调度、重试
//! 语义，以及统计聚合。
//!
//! 实际的 HTTP IO 不在此处执行 —— `Resource::build_fetch_descriptor`
//! 系列函数产生 [`FetchDescriptor`] 值，由适配层
//! （`adapters/network`）执行。这种分离确保了领域层完全
//! 可在无网络访问或无异步运行时的情况下测试。
//!
//! # 模块布局
//!
//! | 模块 | 职责 |
//! |--------|---------------|
//! | `lib.rs` | Resource、Request、RequestScheduler、FetchDescriptor |
//! | `proxy.rs` | DefaultProxy、ProxyPolicy + 可信服务器门控 |
//! | `data_uri.rs` | data: URI 解析/解码（base64 + 百分号） |
//! | `ion.rs` | Ion 资产端点 URL/头部构造 |
//! | `statistics.rs` | RequestStatistics 聚合 |
//! | `priority.rs` | PriorityFunction trait + SSED/距离实现 |
//! | `trusted_servers.rs` | TrustedServers 注册表 |

pub mod data_uri;
pub mod ion;
pub mod priority;
pub mod proxy;
pub mod statistics;
pub mod trusted_servers;

use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;

use crate::priority::{FrameContext, PriorityFunction, PriorityKey};
use crate::proxy::ProxyPolicy;
use crate::statistics::RequestStatistics;

/// 请求的类型。
/// 区分地形/影像/3D Tiles 与其他来源，用于分桶限流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum RequestType {
    /// 地形请求。
    Terrain,
    /// 影像请求。
    Imagery,
    /// 3D Tiles 请求。
    Tiles3D,
    /// 其他请求类型。
    #[default]
    Other,
}

/// 请求的状态。
/// 跟踪请求从发出到完成/失败/取消的生命周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RequestState {
    /// 初始状态，尚未发出。
    #[default]
    Unissued,
    /// 已发出但尚未激活。
    Issued,
    /// 正在活跃处理中。
    Active,
    /// 已收到响应，处理中。
    Received,
    /// 请求失败。
    Failed,
    /// 请求被取消。
    Cancelled,
}

/// 请求的唯一标识符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

/// 存储发起一个请求所需的信息。
/// 聚合 URL、类型、优先级、节流与重试等字段。
#[derive(Debug, Clone)]
pub struct Request {
    /// 唯一标识符。
    pub id: RequestId,
    /// 要请求的 URL。
    pub url: String,
    /// 优先级（越低 = 优先级越高）。
    pub priority: f64,
    /// 是否对请求进行限流与优先级排序。
    pub throttle: bool,
    /// 是否按服务器限流。
    pub throttle_by_server: bool,
    /// 请求类型。
    pub request_type: RequestType,
    /// 当前状态。
    pub state: RequestState,
    /// 用于限流的服务器键。
    pub server_key: String,
    /// 供调度器的 [`PriorityFunction`] 消费的空间键，每帧
    /// 重新计算 `priority`（见 `update_with_context`）。
    ///
    /// 承载供每帧重算优先级所需的瓦片/几何数据，
    /// 相当于优先级闭包所捕获的上下文。
    pub priority_key: Option<PriorityKey>,
}

impl Request {
    /// 创建一个新请求。
    pub fn new(url: String, request_type: RequestType) -> Self {
        let server_key = extract_server_key(&url);
        Self {
            id: RequestId(0),
            url,
            priority: 0.0,
            throttle: false,
            throttle_by_server: false,
            request_type,
            state: RequestState::Unissued,
            server_key,
            priority_key: None,
        }
    }

    /// 创建一个被限流的请求。
    pub fn throttled(url: String, request_type: RequestType, priority: f64) -> Self {
        let server_key = extract_server_key(&url);
        Self {
            id: RequestId(0),
            url,
            priority,
            throttle: true,
            throttle_by_server: true,
            request_type,
            state: RequestState::Unissued,
            server_key,
            priority_key: None,
        }
    }

    /// 附加一个空间 [`PriorityKey`]，以便调度器的优先级函数
    /// 能从帧状态重新计算本请求的优先级。
    pub fn with_priority_key(mut self, key: PriorityKey) -> Self {
        self.priority_key = Some(key);
        self
    }
}

/// 用于优先队列排序的包装器（按优先级的最小堆）。
#[derive(Debug, Clone)]
struct PrioritizedRequest {
    /// 待排序请求的 ID。
    id: RequestId,
    /// 优先级值（越低 = 越先出堆）。
    priority: f64,
}

impl PartialEq for PrioritizedRequest {
    /// 按优先级判等（用 total_cmp 保证与 Ord 一致，含 NaN）。
    fn eq(&self, other: &Self) -> bool {
        // M1 评审修复：使用 `total_cmp`，使得即使对于 NaN 优先级，`eq` 也与
        // `Ord::cmp` 一致。修复前的实现在这里用 `==`（对 NaN 为 false），
        // 而 `cmp` 用 `partial_cmp().unwrap_or(Equal)`（对 NaN 为 Equal），
        // 违反了 `Ord`/`Eq` 一致性法则 `a.cmp(b) == Equal <=> a == b`。
        self.priority.total_cmp(&other.priority) == Ordering::Equal
    }
}

impl Eq for PrioritizedRequest {}

impl PartialOrd for PrioritizedRequest {
    /// 委托给 Ord::cmp，因此全序与偏序结果一致。
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedRequest {
    /// 为最小堆反转比较优先级（值越低 = 优先级越高）。
    fn cmp(&self, other: &Self) -> Ordering {
        // 为最小堆反转排序（更低的优先级值 = 更高的优先级）。M1 评审
        // 修复：`total_cmp` 赋予 NaN 在总序中的一个确定位置（正 NaN 排在
        // `+inf` 之上），因此一个 NaN 优先级的请求自然地沉到最小堆的底部，
        // 永远不会破坏 `BinaryHeap` 不变式。修复前的
        // `partial_cmp().unwrap_or(Equal)` 使 NaN 与每个值都比较为 `Equal`，
        // 破坏了堆的总序契约。
        other.priority.total_cmp(&self.priority)
    }
}

/// 限制优先级值以保证安插入堆（M1 评审修复）。
///
/// NaN 优先级 —— 当一个复合优先级将 `f64::MAX` 量级的各分量求和到
/// `inf`，然后 `inf + (-inf)` 时，在实践中可能达到 —— 会违反
/// `BinaryHeap` 的总序不变式。将 NaN 映射为 `f64::MAX` 会使此类
/// 请求沉到最小堆的 *底部*（优先级最低），从而永不扰乱
/// 格式良好的优先级的排序。有限值（包括 `±inf`）原样透传；
/// 与 `total_cmp` 的 `Ord` 实现结合，这构成了对 NaN 路径的纵深防御。
fn sanitize_priority(priority: f64) -> f64 {
    if priority.is_nan() {
        f64::MAX
    } else {
        priority
    }
}

/// 管理请求的限流与优先级排序。
/// 以优先级堆与逐服务器槽位约束决定哪些请求得以激活。
#[derive(Debug)]
pub struct RequestScheduler {
    /// 同时活动请求的最大数量。
    pub maximum_requests: usize,
    /// 每服务器同时活动请求的最大数量。
    pub maximum_requests_per_server: usize,
    /// 逐服务器对最大请求数的覆盖。
    pub requests_by_server: HashMap<String, usize>,
    /// 是否对请求限流。
    pub throttle_requests: bool,
    /// 优先级堆的最大长度。
    pub priority_heap_length: usize,
    /// 当优先级堆饱和时保留的延迟请求最大数量。当空闲出
    /// 槽位时，延迟请求会被重新提升入堆，以便下一帧
    /// 重新尝试此前被限流的瓦片请求。
    pub maximum_deferred: usize,

    // 内部状态
    /// 当前已激活的请求，按 ID 索引。
    active_requests: HashMap<RequestId, Request>,
    /// 待定请求的优先级最小堆。
    pending_heap: BinaryHeap<PrioritizedRequest>,
    /// 逐服务器的当前活动请求数。
    active_count_by_server: HashMap<String, usize>,
    /// 下一个可分配的请求 ID 计数器。
    next_id: u64,

    /// 聚合的请求统计（attempted/active/succeeded/failed/cancelled
    /// + 逐服务器 + 逐类型），供诊断与限流决策读取。
    statistics: RequestStatistics,

    /// 可选的可插拔优先级函数。设置后，`update_with_context` 会在提升前
    /// 从帧状态重新计算每个待定请求的优先级。
    priority_function: Option<Box<dyn PriorityFunction>>,

    /// 从饱和的优先级堆中被拒绝的请求，为后续提升而保留。
    /// 隐式排序；当堆重新有空闲槽位时按优先级重新插入。
    deferred: Vec<Request>,
}

impl RequestScheduler {
    /// 使用默认设置创建一个新的 RequestScheduler。
    pub fn new() -> Self {
        Self {
            maximum_requests: 50,
            maximum_requests_per_server: 18,
            requests_by_server: HashMap::new(),
            throttle_requests: true,
            priority_heap_length: 20,
            maximum_deferred: 64,
            active_requests: HashMap::new(),
            pending_heap: BinaryHeap::new(),
            active_count_by_server: HashMap::new(),
            next_id: 0,
            statistics: RequestStatistics::new(),
            priority_function: None,
            deferred: Vec::new(),
        }
    }

    /// 返回聚合统计的共享引用（为诊断而暴露）。
    pub fn statistics(&self) -> &RequestStatistics {
        &self.statistics
    }

    /// 返回聚合统计的可变引用。
    pub fn statistics_mut(&mut self) -> &mut RequestStatistics {
        &mut self.statistics
    }

    /// 将聚合统计重置为零（用于测试或重新初始化）。
    pub fn reset_statistics(&mut self) {
        self.statistics.reset();
    }

    /// 安装一个可插拔的优先级函数。
    ///
    /// 一旦设置，[`RequestScheduler::update_with_context`] 会在提升前从帧上下文
    /// 重新计算每个待定请求的优先级，实现逐帧的
    /// 优先级重排序。
    pub fn set_priority_function(&mut self, f: Box<dyn PriorityFunction>) {
        self.priority_function = Some(f);
    }

    /// 返回已安装的优先级函数的名称（若有）。
    pub fn priority_function_name(&self) -> Option<&str> {
        self.priority_function.as_ref().map(|f| f.name())
    }

    /// 返回当前被延迟（堆拒绝）的请求数量。
    pub fn deferred_count(&self) -> usize {
        self.deferred.len()
    }

    /// 返回当前活动请求的数量（跨所有服务器求和）。
    pub fn active_request_count(&self) -> usize {
        self.active_count_by_server.values().sum()
    }

    /// 返回待定请求的数量。
    pub fn pending_request_count(&self) -> usize {
        self.pending_heap.len()
    }

    /// 检查某个服务器是否有空闲槽位接受更多请求。
    pub fn server_has_open_slots(&self, server_key: &str, desired_requests: usize) -> bool {
        let max_requests = self
            .requests_by_server
            .get(server_key)
            .copied()
            .unwrap_or(self.maximum_requests_per_server);
        let current = self.active_count_by_server.get(server_key).copied().unwrap_or(0);
        current + desired_requests <= max_requests
    }

    /// 检查优先级堆是否有空闲槽位容纳更多待定请求。
    pub fn heap_has_open_slots(&self, desired_requests: usize) -> bool {
        self.pending_heap.len() + desired_requests <= self.priority_heap_length
    }

    /// 调度一个请求。若被接受则返回请求 ID。
    ///
    /// 当启用了限流且请求
    /// 无法立即激活时，它会被放入优先级堆。若堆已饱和，请求会被
    /// *延迟*（为后续提升而保留）而非丢弃，最多到
    /// [`Self::maximum_deferred`]。
    pub fn schedule(&mut self, mut request: Request) -> Option<RequestId> {
        // 分配 ID
        let id = RequestId(self.next_id);
        self.next_id += 1;
        request.id = id;
        self.statistics.on_scheduled();

        // 若不限流，立即激活
        if !self.throttle_requests || !request.throttle {
            self.activate_request(request);
            return Some(id);
        }

        // 检查是否可以立即激活
        if self.can_activate(&request) {
            self.activate_request(request);
            return Some(id);
        }

        // 若还有空间则加入待定堆
        if self.pending_heap.len() < self.priority_heap_length {
            request.state = RequestState::Issued;
            self.pending_heap.push(PrioritizedRequest {
                id,
                // M1 评审修复：防止 NaN 优先级破坏堆的总序
                // （NaN 作为 `f64::MAX` 沉到底部）。
                priority: sanitize_priority(request.priority),
            });
            self.active_requests.insert(id, request);
            Some(id)
        } else {
            // 堆饱和 —— 应用优先级拒绝规则（与内部优先级队列的
            // 插入语义一致）：
            // 当新项的优先级 **不优于** 最差的常驻项时，优先级队列的
            // 插入操作会失败，此时请求会被直接拒绝。只有当
            // 新项严格优于最差的常驻项时，我们才驱逐最差项
            // （若还有空间则放入 M8.1 延迟队列）并接纳新来者。
            //
            // `pending_heap` 是按优先级值的最小堆（值越低 = 优先级越高，见
            // `PrioritizedRequest::cmp`），因此 *最差* 的常驻项是 `priority`
            // 字段 **最大** 的那个条目。
            let worst_priority = self
                .pending_heap
                .iter()
                .map(|p| p.priority)
                .fold(f64::NEG_INFINITY, f64::max);

            if request.priority.partial_cmp(&worst_priority) != Some(Ordering::Less) {
                // 新请求不严格优于最差的常驻项
                // （涵盖同优先级、更差优先级以及 NaN 不可比的
                // 情况）。拒绝它并记账 —— 这与“堆无空闲槽位”
                // 的拒绝分支一致。
                self.statistics.on_cancelled_pending();
                return None;
            }

            // 新请求严格优于最差的常驻项：驱逐最差项，接纳新来者。
            // `BinaryHeap` 没有按值移除，因此排空 + 重建（堆大小受
            // `priority_heap_length` 限制，默认 20 —— O(n) 重建可忽略不计，
            // 且能精确保持不变式）。
            let mut entries: Vec<PrioritizedRequest> = self.pending_heap.drain().collect();
            let mut worst_idx = 0;
            for (i, e) in entries.iter().enumerate() {
                if e.priority > entries[worst_idx].priority {
                    worst_idx = i;
                }
            }
            let evicted = entries.remove(worst_idx);
            entries.push(PrioritizedRequest {
                id,
                // M1 评审修复：NaN → f64::MAX 沉底守卫。
                priority: sanitize_priority(request.priority),
            });
            for e in entries {
                self.pending_heap.push(e);
            }

            // 将被驱逐的请求路由到 M8.1 延迟队列（若还有空间），以便后续
            // `update()` 能在堆排空后将其提升回来；否则丢弃它并为取消记账。
            if let Some(mut evicted_req) = self.active_requests.remove(&evicted.id) {
                evicted_req.state = RequestState::Unissued;
                if self.deferred.len() < self.maximum_deferred {
                    self.deferred.push(evicted_req);
                } else {
                    self.statistics.on_cancelled_pending();
                }
            }

            request.state = RequestState::Issued;
            self.active_requests.insert(id, request);
            Some(id)
        }
    }

    /// 取消一个请求。
    ///
    /// 根据请求是否已被激活（cancelled-active）还是仍为待定（cancelled-pending），
    /// 以不同方式更新统计。
    pub fn cancel(&mut self, id: RequestId) -> bool {
        let was_active = self
            .active_requests
            .get(&id)
            .map(|r| r.state == RequestState::Active)
            .unwrap_or(false);
        match self.deactivate_request(id) {
            Some(request) => {
                if was_active {
                    let server_key = request.server_key.clone();
                    let rtype = request.request_type;
                    self.statistics.on_cancelled_active(&server_key, rtype);
                } else {
                    self.statistics.on_cancelled_pending();
                }
                true
            }
            None => false,
        }
    }

    /// 将一个请求标记为成功完成。
    pub fn complete(&mut self, id: RequestId) -> bool {
        let was_active = self
            .active_requests
            .get(&id)
            .map(|r| r.state == RequestState::Active)
            .unwrap_or(false);
        match self.deactivate_request(id) {
            Some(request) => {
                let server_key = request.server_key.clone();
                let rtype = request.request_type;
                if was_active {
                    self.statistics.on_completed(&server_key, rtype);
                } else {
                    // 在仍为待定时完成：计为成功而不做
                    // active 递减（它从未被激活）。
                    self.statistics.succeeded += 1;
                    *self.statistics.completed_by_server.entry(server_key).or_insert(0) += 1;
                    *self.statistics.completed_by_type.entry(rtype).or_insert(0) += 1;
                }
                true
            }
            None => false,
        }
    }

    /// 将一个请求标记为失败（重试耗尽或不可恢复的错误）。
    ///
    /// 失败路径：递增失败计数并释放该请求，
    /// 使其占用的服务器槽位重新开启。
    pub fn fail(&mut self, id: RequestId) -> bool {
        let was_active = self
            .active_requests
            .get(&id)
            .map(|r| r.state == RequestState::Active)
            .unwrap_or(false);
        match self.deactivate_request(id) {
            Some(request) => {
                let server_key = request.server_key.clone();
                let rtype = request.request_type;
                if was_active {
                    self.statistics.on_failed(&server_key, rtype);
                } else {
                    self.statistics.failed += 1;
                    *self.statistics.failed_by_server.entry(server_key).or_insert(0) += 1;
                    *self.statistics.failed_by_type.entry(rtype).or_insert(0) += 1;
                }
                true
            }
            None => false,
        }
    }

    /// 更新优先级并激活待定请求。
    /// 应每帧调用一次。
    ///
    /// 操作顺序：
    /// 1. 快照之前的活动计数用于增量诊断。
    /// 2. 若槽位开启，将延迟请求提升入优先级堆。
    /// 3. 在全局/逐服务器槽位可用时激活待定请求。
    pub fn update(&mut self) {
        self.statistics.snapshot_last_active();
        self.promote_deferred();

        // 激活待定请求。核心规则：仅当
        // 全局槽位预算耗尽时循环才终止。当一个堆顶项自身的服务器
        // 已饱和时，它会被 SKIPPED——弹出并停放到延迟队列留待后续
        // 帧——然后扫描 CONTINUES，因此发往其他有空闲槽位服务器的
        // 待定请求仍会被激活。修复前的代码在任何 `can_activate == false`
        // 时 `break`，将“全局预算满”与“本请求的服务器满”混为一谈：
        // 一个处于堆顶的饱和服务器会使其他所有服务器的积压在整个
        // 帧内挨饿（H1 评审修复）。
        while let Some(prioritized) = self.pending_heap.peek() {
            // 全局预算耗尽 → 本帧不能再激活任何东西。
            if !self.global_has_slots() {
                break;
            }
            let id = prioritized.id;
            let Some(request) = self.active_requests.get(&id) else {
                // 陈旧的堆条目（请求已取消/完成）→ 丢弃。
                self.pending_heap.pop();
                continue;
            };
            if self.server_has_slots(request) {
                let request = self.active_requests.get_mut(&id).unwrap();
                request.state = RequestState::Active;
                let server_key = request.server_key.clone();
                let rtype = request.request_type;
                self.pending_heap.pop();
                *self.active_count_by_server.entry(server_key.clone()).or_insert(0) += 1;
                self.statistics.on_activated(&server_key, rtype);
            } else {
                // 本请求的服务器已饱和。跳过它（从堆中弹出）
                // 并将其停放到延迟队列，以便 `promote_deferred` 在其服务器排空后的
                // 后续帧重新接纳它，然后继续扫描其他服务器的请求。弹出——而非
                // 将其留在堆顶——正是让循环能越过饱和服务器继续推进的原因
                // （反复重新 peek 它就是修复前的 `break` 使其他服务器挨饿的原因）。
                self.pending_heap.pop();
                if let Some(mut req) = self.active_requests.remove(&id) {
                    req.state = RequestState::Unissued;
                    if self.deferred.len() < self.maximum_deferred {
                        self.deferred.push(req);
                    } else {
                        self.statistics.on_cancelled_pending();
                    }
                }
            }
        }
    }

    /// 从帧状态重新计算待定请求的优先级，然后更新。
    ///
    /// 当安装了 [`PriorityFunction`] 时，每个携带 [`PriorityKey`] 的待定
    /// （Issued）请求都会根据提供的 [`FrameContext`] 重新计算优先级；然后重建
    /// 优先级堆并运行 [`Self::update`]。这样每帧都会在提升前
    /// 重新求值每个待定请求的优先级。
    ///
    /// 若未安装优先级函数，这等价于 [`Self::update`]。
    pub fn update_with_context(&mut self, context: &FrameContext) {
        if let Some(pf) = self.priority_function.as_ref() {
            let mut new_entries: Vec<PrioritizedRequest> = Vec::new();
            // `pf` 借用 `self.priority_function`；循环借用
            // `self.active_requests` —— 不相交的字段。
            for (id, request) in self.active_requests.iter_mut() {
                if request.state == RequestState::Issued {
                    if let Some(key) = &request.priority_key {
                        // M1 评审修复：对重新计算的优先级做守卫，以免优先级
                        // 函数返回的 NaN 破坏重建后堆的总序（NaN → f64::MAX 沉底）。
                        request.priority = sanitize_priority(pf.compute_priority(key, context));
                    }
                    new_entries.push(PrioritizedRequest {
                        id: *id,
                        priority: sanitize_priority(request.priority),
                    });
                }
            }
            self.pending_heap = BinaryHeap::from(new_entries);
        }
        self.update();
    }

    /// 按 ID 获取一个请求（活动或待定；延迟请求尚未在此追踪
    /// —— 见 [`Self::deferred_requests`]）。
    pub fn get_request(&self, id: RequestId) -> Option<&Request> {
        self.active_requests.get(&id)
    }

    /// 返回当前被延迟（堆拒绝、等待提升）的
    /// 请求。
    pub fn deferred_requests(&self) -> &[Request] {
        &self.deferred
    }

    // 内部辅助函数

    /// 全局槽位谓词：只要调度器还有空间接纳 *任何* 进一步的
    /// 活动请求（无论哪个服务器）就为 `true`。从旧的合并
    /// `can_activate` 中拆分出来，以便 [`Self::update`] 能区分“全局
    /// 预算耗尽”（终止循环）与“本请求的服务器饱和”（仅跳过该请求）
    /// —— 即 H1 评审修复。
    fn global_has_slots(&self) -> bool {
        let active_count = self.active_count_by_server.values().sum::<usize>();
        active_count < self.maximum_requests
    }

    /// 逐服务器谓词：若本请求可以在其自身服务器上占用一个槽位
    /// （或根本不按服务器限流）则为 `true`。
    fn server_has_slots(&self, request: &Request) -> bool {
        !request.throttle_by_server || self.server_has_open_slots(&request.server_key, 1)
    }

    /// 组合的立即激活谓词（全局 AND 逐服务器）。供 [`Self::schedule`]
    /// 的快路径检查使用；[`Self::update`] 分别查询
    /// 两个谓词，以致一个饱和的服务器不会拖慢整个
    /// 激活循环。
    fn can_activate(&self, request: &Request) -> bool {
        self.global_has_slots() && self.server_has_slots(request)
    }

    /// 在槽位开启时将延迟请求提升入优先级堆。
    ///
    /// 延迟请求按优先级值升序（最优优先）提升，以便最重要的
    /// 延迟请求在容量空闲时重新进入堆。
    fn promote_deferred(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        // 最优（优先级值最低）优先。M1 评审修复：`total_cmp` 保持排序
        // 为总序（NaN 获得一个确定位置），而非之前的
        // `partial_cmp().unwrap_or(Equal)`，后者使 NaN 无序。
        self.deferred
            .sort_by(|a, b| a.priority.total_cmp(&b.priority));

        let mut remaining: Vec<Request> = Vec::new();
        for mut request in self.deferred.drain(..) {
            if self.pending_heap.len() < self.priority_heap_length {
                let id = request.id;
                // M1 评审修复：NaN → f64::MAX 沉底守卫。
                let priority = sanitize_priority(request.priority);
                request.state = RequestState::Issued;
                self.active_requests.insert(id, request);
                self.pending_heap.push(PrioritizedRequest { id, priority });
            } else {
                remaining.push(request);
            }
        }
        self.deferred = remaining;
    }

    /// 激活一个请求：置为 Active 状态，占用其服务器槽位并更新统计。
    fn activate_request(&mut self, mut request: Request) {
        request.state = RequestState::Active;
        let server_key = request.server_key.clone();
        let rtype = request.request_type;
        *self.active_count_by_server.entry(server_key.clone()).or_insert(0) += 1;
        self.statistics.on_activated(&server_key, rtype);
        self.active_requests.insert(request.id, request);
    }

    /// 从一个请求停止追踪，若它曾处于活动状态则释放其服务器槽位。
    /// 返回被移除的请求，以便调用方根据结果（complete/fail/cancel）
    /// 更新统计。
    fn deactivate_request(&mut self, id: RequestId) -> Option<Request> {
        if let Some(request) = self.active_requests.remove(&id) {
            if request.state == RequestState::Active {
                if let Some(count) = self.active_count_by_server.get_mut(&request.server_key) {
                    *count = count.saturating_sub(1);
                }
            }
            Some(request)
        } else {
            None
        }
    }
}

impl Default for RequestScheduler {
    /// 默认调度器，等价于 [`RequestScheduler::new`]。
    fn default() -> Self {
        Self::new()
    }
}

/// 从 URL 中提取服务器键（host:port）。
///
/// 添加默认端口：http→80，https→443。
pub fn get_server_key(url: &str) -> String {
    if let Some(start) = url.find("://") {
        let scheme = url[..start].to_lowercase();
        let after_scheme = &url[start + 3..];
        let end = after_scheme.find('/').unwrap_or(after_scheme.len());
        let authority = &after_scheme[..end];
        // 剥离凭据（user:pass@）
        let authority = if let Some(at) = authority.find('@') {
            &authority[at + 1..]
        } else {
            authority
        };
        // 若缺失则添加默认端口
        if authority.contains(':') {
            authority.to_lowercase()
        } else {
            match scheme.as_str() {
                "http" => format!("{}:80", authority.to_lowercase()),
                "https" => format!("{}:443", authority.to_lowercase()),
                _ => authority.to_lowercase(),
            }
        }
    } else {
        url.to_string()
    }
}

/// 用于向后兼容的内部别名。
fn extract_server_key(url: &str) -> String {
    get_server_key(url)
}

/// 一个带查询参数的资源 URL 模板。
/// 聚合基础 URL、查询参数、模板值与头部。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    /// 基础 URL（不含查询字符串）。
    pub url: String,
    /// 查询参数。
    pub query_parameters: HashMap<String, String>,
    /// 用于 URL 替换的模板值（{key} → value）。
    pub template_values: HashMap<String, String>,
    /// HTTP 头部。
    pub headers: HashMap<String, String>,
}

/// 创建派生资源的选项。
/// 描述相对于父资源派生时要合并的 URL/查询/模板/头部。
#[derive(Debug, Clone, Default)]
pub struct DeriveResourceOptions {
    /// 要相对于父资源解析的相对或绝对 URL。
    pub url: Option<String>,
    /// 额外的查询参数。
    pub query_parameters: Vec<(String, String)>,
    /// 额外的模板值。
    pub template_values: Vec<(String, String)>,
    /// 额外的头部。
    pub headers: Vec<(String, String)>,
}

impl Resource {
    /// 使用给定的 URL 创建一个新资源。
    /// 若 URL 中存在则解析查询参数。
    pub fn new(url: impl Into<String>) -> Self {
        let raw = url.into();
        let (base, query_params) = Self::parse_url(&raw);
        Self {
            url: base,
            query_parameters: query_params,
            template_values: HashMap::new(),
            headers: HashMap::new(),
        }
    }

    /// 创建一个不从 URL 解析查询参数的资源。
    /// 查询参数须后续通过 `with_query` 显式添加。
    pub fn new_unparsed(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            query_parameters: HashMap::new(),
            template_values: HashMap::new(),
            headers: HashMap::new(),
        }
    }

    /// 添加一个查询参数。
    pub fn with_query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query_parameters.insert(key.into(), value.into());
        self
    }

    /// 添加一个模板值。
    pub fn with_template_value(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.template_values.insert(key.into(), value.into());
        self
    }

    /// 添加一个头部。
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// 若 URL 末尾没有正斜杠则追加上一个。
    pub fn append_forward_slash(&mut self) {
        if !self.url.ends_with('/') {
            self.url.push('/');
        }
    }

    /// 获取 URL 组件，可选择是否包含查询参数。
    /// 供上层按需拼出带/不带查询串的 URL 片段。
    pub fn get_url_component(&self, include_query: bool) -> String {
        if !include_query || self.query_parameters.is_empty() {
            return self.url.clone();
        }
        format!("{}?{}", self.url, self.build_query_string())
    }

    /// 构建含查询参数与模板替换的完整 URL。
    /// 应用模板值后按需要追加查询串。
    pub fn build_url(&self) -> String {
        let base = self.apply_template_values(&self.url);
        if self.query_parameters.is_empty() {
            return base;
        }
        if base.contains('?') {
            format!("{}&{}", base, self.build_query_string())
        } else {
            format!("{}?{}", base, self.build_query_string())
        }
    }

    /// 获取此资源的服务器键。
    pub fn server_key(&self) -> String {
        extract_server_key(&self.url)
    }

    /// 设置查询参数，可选择将现有值保留为默认值。
    /// `use_as_default` 为真时只填充尚不存在的键。
    pub fn set_query_parameters(&mut self, params: Vec<(String, String)>, use_as_default: bool) {
        if use_as_default {
            // 仅添加尚不存在的键
            for (k, v) in params {
                self.query_parameters.entry(k).or_insert(v);
            }
        } else {
            // 全部覆盖
            self.query_parameters = params.into_iter().collect();
        }
    }

    /// 通过将相对 URL 相对于本资源解析来创建一个派生资源。
    /// 合并选项中的查询/模板/头部到派生结果。
    pub fn get_derived_resource(&self, options: &DeriveResourceOptions) -> Self {
        let mut derived_url = self.url.clone();

        if let Some(ref rel_url) = options.url {
            derived_url = Self::resolve_url(&derived_url, rel_url);
        }

        // 合并查询参数
        let mut query = self.query_parameters.clone();
        for (k, v) in &options.query_parameters {
            query.insert(k.clone(), v.clone());
        }

        // 从派生 URL 解析查询参数
        let (base, url_params) = Self::parse_url(&derived_url);
        for (k, v) in url_params {
            query.insert(k, v);
        }

        // 合并模板值
        let mut templates = self.template_values.clone();
        for (k, v) in &options.template_values {
            templates.insert(k.clone(), v.clone());
        }

        // 合并头部
        let mut headers = self.headers.clone();
        for (k, v) in &options.headers {
            headers.insert(k.clone(), v.clone());
        }

        // 将模板值应用于 URL
        let final_url = Self::apply_template_values_static(&base, &templates);

        Self {
            url: final_url,
            query_parameters: query,
            template_values: templates,
            headers,
        }
    }

    /// 创建一个附加了相对路径的派生资源（旧版 API）。
    pub fn derive(&self, relative_path: &str) -> Self {
        let base = if self.url.ends_with('/') {
            &self.url
        } else if let Some(pos) = self.url.rfind('/') {
            &self.url[..=pos]
        } else {
            &self.url
        };

        Self {
            url: format!("{}{}", base, relative_path),
            query_parameters: self.query_parameters.clone(),
            template_values: self.template_values.clone(),
            headers: self.headers.clone(),
        }
    }

    // ─── 内部辅助函数 ──────────────────────────────────────────────────────

    /// 将一个 URL 解析为基础部分（不含查询）与查询参数。
    fn parse_url(raw: &str) -> (String, HashMap<String, String>) {
        if let Some(qpos) = raw.find('?') {
            let base = raw[..qpos].to_string();
            let query_str = &raw[qpos + 1..];
            let params = Self::parse_query_string(query_str);
            (base, params)
        } else {
            (raw.to_string(), HashMap::new())
        }
    }

    /// 将查询字符串解析为键值对。
    fn parse_query_string(qs: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for pair in qs.split('&') {
            if pair.is_empty() {
                continue;
            }
            if let Some(eq) = pair.find('=') {
                let key = pair[..eq].to_string();
                let value = pair[eq + 1..].to_string();
                map.insert(key, value);
            } else {
                map.insert(pair.to_string(), String::new());
            }
        }
        map
    }

    /// 从参数构建查询字符串（为确定性而排序）。
    fn build_query_string(&self) -> String {
        let mut pairs: Vec<_> = self.query_parameters.iter().collect();
        // deferred.md #14: 采纳 clippy 建议解引用克隆内层 String（原 k.clone() 对 &&String 双重引用克隆）。
        pairs.sort_by_key(|(k, _)| (*k).clone());
        pairs
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&")
    }

    /// 将模板值应用于本资源的 URL。
    fn apply_template_values(&self, url: &str) -> String {
        Self::apply_template_values_static(url, &self.template_values)
    }

    /// 用模板值替换 URL 中的 {key} 占位符。
    fn apply_template_values_static(url: &str, templates: &HashMap<String, String>) -> String {
        if templates.is_empty() {
            return url.to_string();
        }
        let mut result = url.to_string();
        for (key, value) in templates {
            let placeholder = format!("{{{}}}", key);
            // 对值进行 URL 编码（编码特殊字符）
            let encoded = Self::encode_uri_component(value);
            result = result.replace(&placeholder, &encoded);
        }
        result
    }

    /// 编码一个 URI 组件（对特殊字符进行百分号编码）。
    fn encode_uri_component(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~'
                | b'*' | b'\'' | b'(' | b')' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }

    /// 将相对 URL 相对于基础 URL 解析。
    /// 支持绝对 URL 直传与基于目录前缀的相对解析。
    fn resolve_url(base: &str, relative: &str) -> String {
        // 若 relative 是绝对的（含方案），直接使用它
        if relative.contains("://") {
            return relative.to_string();
        }

        // 获取基础 URL 的目录部分
        let directory = if base.ends_with('/') {
            base.to_string()
        } else if let Some(pos) = base.rfind('/') {
            base[..=pos].to_string()
        } else {
            base.to_string()
        };

        format!("{}{}", directory, relative)
    }

    // ─── URI 分类 ──────────────────────────────────────────────

    /// 若本资源的 URL 是 `data:` URI 则返回 `true`。
    ///
    /// Data URI 内联携带其负载且从不访问网络，因此
    /// fetch-descriptor 管道会对它们短路（不走代理，不走调度器），
    /// 识别走标准的 `data:` 前缀判断。
    pub fn is_data_uri(&self) -> bool {
        crate::data_uri::is_data_uri(&self.url)
    }

    /// 若本资源的 URL 是 `blob:` URI 则返回 `true`。
    ///
    /// Blob URI 引用内存中的对象；与 data URI 一样它们
    /// 从不被代理，也从不发起网络请求。
    pub fn is_blob_uri(&self) -> bool {
        self.url.starts_with("blob:")
    }

    /// 返回本资源的基础 URI（`scheme://authority/`）。
    ///
    /// 无方案前缀时原样返回已有 URL。
    pub fn get_base_uri(&self) -> String {
        if let Some(start) = self.url.find("://") {
            let after = &self.url[start + 3..];
            let end = after.find('/').unwrap_or(after.len());
            format!("{}/", &self.url[..start + 3 + end])
        } else {
            self.url.clone()
        }
    }

    /// 追加查询值，覆盖任何已存在的键。
    ///
    /// 逐键插入，同名键的旧值被覆盖。
    pub fn append_query_values(&mut self, params: &[(String, String)]) {
        for (k, v) in params {
            self.query_parameters.insert(k.clone(), v.clone());
        }
    }

    /// 移除给定的查询参数键。
    ///
    /// 按给定键逐个从查询表中移除。
    pub fn remove_query_values(&mut self, keys: &[&str]) {
        for k in keys {
            self.query_parameters.remove(*k);
        }
    }

    /// 以不同的基础 URL 克隆本资源，保留查询
    /// 参数、模板值与头部。
    ///
    /// 用于仅替换 URL 而其他字段保持不变的派生场景。
    pub fn clone_with_url(&self, url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            query_parameters: self.query_parameters.clone(),
            template_values: self.template_values.clone(),
            headers: self.headers.clone(),
        }
    }

    // ─── Fetch descriptor 构造（纯函数；无 IO） ──────────────────────

    /// 构建一个 [`FetchDescriptor`]，描述要请求 *什么* 而不
    /// 执行任何 IO。
    ///
    /// 这是各 fetch 方法在领域侧的共同入口：
    /// 适配层（`adapters/network`）消费该描述符并执行
    /// 实际的 HTTP 请求。因为构造是纯函数，整个 URL /
    /// 头部 / 代理 / 重试管道都可在无网络的情况下单元测试。
    ///
    /// Data URI 短路：设置 `is_data_uri` 且不应用代理。
    pub fn build_fetch_descriptor(
        &self,
        response_type: ResponseType,
        proxy: Option<&ProxyPolicy>,
    ) -> FetchDescriptor {
        let raw_url = self.build_url();
        let is_data = self.is_data_uri() || self.is_blob_uri();
        let final_url = if is_data {
            raw_url
        } else {
            match proxy {
                Some(policy) => policy.apply(&raw_url),
                None => raw_url,
            }
        };

        FetchDescriptor {
            url: final_url,
            method: HttpMethod::Get,
            headers: self.headers.clone(),
            response_type,
            request_type: RequestType::Other,
            retry: RetryPolicy::default(),
            priority: 0.0,
            server_key: if is_data { String::new() } else { self.server_key() },
            body: None,
            is_data_uri: is_data,
        }
    }

    /// 获取二进制内容（`ArrayBuffer`）的描述符。
    /// 响应类型固定为 `ArrayBuffer`。
    pub fn fetch_array_buffer(&self, proxy: Option<&ProxyPolicy>) -> FetchDescriptor {
        self.build_fetch_descriptor(ResponseType::ArrayBuffer, proxy)
    }

    /// 获取 JSON 内容的描述符。
    /// 响应类型固定为 `Json`。
    pub fn fetch_json(&self, proxy: Option<&ProxyPolicy>) -> FetchDescriptor {
        self.build_fetch_descriptor(ResponseType::Json, proxy)
    }

    /// 获取文本内容的描述符。
    /// 响应类型固定为 `Text`。
    pub fn fetch_text(&self, proxy: Option<&ProxyPolicy>) -> FetchDescriptor {
        self.build_fetch_descriptor(ResponseType::Text, proxy)
    }

    /// 获取图像内容的描述符。
    /// 响应类型固定为 `Image`。
    pub fn fetch_image(&self, proxy: Option<&ProxyPolicy>) -> FetchDescriptor {
        self.build_fetch_descriptor(ResponseType::Image, proxy)
    }

    /// 获取 blob 内容的描述符。
    /// 响应类型固定为 `Blob`。
    pub fn fetch_blob(&self, proxy: Option<&ProxyPolicy>) -> FetchDescriptor {
        self.build_fetch_descriptor(ResponseType::Blob, proxy)
    }

    /// 携带请求体的 POST 请求描述符。
    /// 方法置为 `POST` 并携带给定请求体。
    pub fn post(&self, body: Vec<u8>, proxy: Option<&ProxyPolicy>) -> FetchDescriptor {
        let mut descriptor = self.build_fetch_descriptor(ResponseType::Json, proxy);
        descriptor.method = HttpMethod::Post;
        descriptor.body = Some(body);
        descriptor
    }
}

impl std::fmt::Display for Resource {
    /// 渲染为构建后的完整 URL。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.build_url())
    }
}

// ─── 响应 / 方法 / fetch 描述符类型 ─────────────────────────

/// 预期的响应体类型。
///
/// 枚举涵盖数组缓冲/文本/JSON/图像等形态。适配层
/// 用此决定如何解码 HTTP 响应体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseType {
    /// 原始字节。
    ArrayBuffer,
    /// 二进制大对象（图像等）。
    Blob,
    /// 已解析的文档（XML/HTML）。
    Document,
    /// JSON 值。
    Json,
    /// UTF-8 文本。
    Text,
    /// 已解码的图像。
    Image,
    /// 已解码的位图（可上传 GPU）。
    ImageBitmap,
}

/// 一次 fetch 的 HTTP 方法。
///
/// 涵盖只读的 GET 与会修改资源的 POST/PUT/PATCH/DELETE。
/// 具体方法由各 fetch 构造器选定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    /// 返回规范的大写方法 token。
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        }
    }
}

/// 一个完全解析的网络请求描述，由纯领域层产生、
/// 由适配层执行。
///
/// 这是将 IO 排除在领域层之外的边界类型：`Resource`
/// 构造它，`adapters/network` 消费它。
#[derive(Debug, Clone, PartialEq)]
pub struct FetchDescriptor {
    /// 最终 URL（已应用代理，已替换查询/模板）。
    pub url: String,
    /// HTTP 方法。
    pub method: HttpMethod,
    /// 请求头部。
    pub headers: HashMap<String, String>,
    /// 预期的响应体类型。
    pub response_type: ResponseType,
    /// 逻辑请求类型（用于调度器统计/限流）。
    pub request_type: RequestType,
    /// 失败时应用的重试策略。
    pub retry: RetryPolicy,
    /// 初始优先级提示（越低 = 优先级越高）。
    pub priority: f64,
    /// 用于逐服务器限流的服务器键（`host:port`）。对 data URI 为空。
    pub server_key: String,
    /// 可选的请求体（POST/PUT/PATCH）。
    pub body: Option<Vec<u8>>,
    /// URL 是否为内联的 `data:`/`blob:` URI（无需网络）。
    pub is_data_uri: bool,
}

impl FetchDescriptor {
    /// 覆盖逻辑请求类型。
    pub fn with_request_type(mut self, request_type: RequestType) -> Self {
        self.request_type = request_type;
        self
    }

    /// 覆盖重试策略。
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// 覆盖初始优先级提示。
    pub fn with_priority(mut self, priority: f64) -> Self {
        self.priority = priority;
        self
    }

    /// 添加（或替换）一个头部。
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// 将此描述符转换为一个可调度的 [`Request`]。
    pub fn to_request(&self) -> Request {
        let mut request = Request::throttled(
            self.url.clone(),
            self.request_type,
            self.priority,
        );
        request.server_key = if self.server_key.is_empty() {
            extract_server_key(&self.url)
        } else {
            self.server_key.clone()
        };
        request
    }
}

// ─── 重试语义（纯函数） ─────────────────────────────────────────────

/// 请求失败的分类，用于决定可重试性。
///
/// 依据 HTTP 状态码对失败进行分类，
/// 从而决定一次重试是否值得发起。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestErrorClass {
    /// 可恢复（5xx、网络超时）—— 退避重试。
    Transient,
    /// 被限流（HTTP 429）—— 仅当 `retry_on_throttled` 时重试。
    Throttled,
    /// 客户端错误（除 429 外的 4xx）—— 从不重试。
    Permanent,
}

/// 在重试尝试之间应用的退避形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffStrategy {
    /// 尝试之间固定延迟。
    Fixed,
    /// 延迟随尝试次数线性增长。
    Linear,
    /// 延迟每次尝试翻倍（上限为 `max_delay_millis`）。
    Exponential,
}

/// 一次失败尝试后调度器/调用方的决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// 立即重试（无延迟）。
    RetryNow,
    /// 在给定毫秒数延迟后重试。
    RetryAfterMillis(u64),
    /// 停止重试；上报失败。
    GiveUp,
}

/// 纯重试策略。
///
/// 描述最大尝试次数与退避形状。
/// 默认仅一次尝试且无延迟，但宿主可
/// 为瞬态/限流失败配置指数退避。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 初始请求之后的最大重试次数。
    pub max_attempts: u32,
    /// 基准延迟（毫秒，由退避策略使用）。
    pub base_delay_millis: u64,
    /// 计算出的延迟的上限。
    pub max_delay_millis: u64,
    /// 退避形状。
    pub backoff: BackoffStrategy,
    /// 是否重试被限流（429）的响应。
    pub retry_on_throttled: bool,
}

impl Default for RetryPolicy {
    /// 默认重试策略：仅一次尝试、固定退避、对限流响应重试。
    fn default() -> Self {
        Self {
            // 默认仅尝试 1 次（不额外重试）。
            max_attempts: 1,
            base_delay_millis: 0,
            max_delay_millis: 30_000,
            backoff: BackoffStrategy::Fixed,
            retry_on_throttled: true,
        }
    }
}

impl RetryPolicy {
    /// 一个从不重试的策略。
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 0,
            ..Default::default()
        }
    }

    /// 带合理默认基准延迟的指数退避。
    pub fn exponential(max_attempts: u32, base_delay_millis: u64) -> Self {
        Self {
            max_attempts,
            base_delay_millis,
            backoff: BackoffStrategy::Exponential,
            ..Default::default()
        }
    }

    /// 计算在给定（从 0 开始）重试尝试之前的延迟。
    pub fn delay_for(&self, attempt: u32) -> u64 {
        if self.base_delay_millis == 0 {
            return 0;
        }
        let raw = match self.backoff {
            BackoffStrategy::Fixed => self.base_delay_millis,
            BackoffStrategy::Linear => {
                self.base_delay_millis.saturating_mul((attempt.max(1)) as u64)
            }
            BackoffStrategy::Exponential => {
                self.base_delay_millis.saturating_mul(1u64 << attempt.min(20))
            }
        };
        raw.min(self.max_delay_millis)
    }

    /// 决定一次失败尝试后是否重试。
    ///
    /// `attempt` 是已执行的重试次数（0 = 首次失败）。
    /// 这是重试决策的纯函数核心。
    pub fn decide(&self, attempt: u32, error: RequestErrorClass) -> RetryDecision {
        match error {
            RequestErrorClass::Permanent => RetryDecision::GiveUp,
            RequestErrorClass::Throttled if !self.retry_on_throttled => RetryDecision::GiveUp,
            _ => {
                if attempt >= self.max_attempts {
                    RetryDecision::GiveUp
                } else {
                    let delay = self.delay_for(attempt);
                    if delay == 0 {
                        RetryDecision::RetryNow
                    } else {
                        RetryDecision::RetryAfterMillis(delay)
                    }
                }
            }
        }
    }
}

/// 将一个 HTTP 状态码分类为一个 [`RequestErrorClass`]。
///
/// 基于 HTTP 状态的退避启发式：
/// 429 = 限流，5xx = 瞬态，其他 4xx = 永久。
pub fn classify_status(status: u16) -> RequestErrorClass {
    match status {
        429 => RequestErrorClass::Throttled,
        400..=499 => RequestErrorClass::Permanent,
        _ => RequestErrorClass::Transient,
    }
}

/// 一次失败尝试后交给 [`RetryCallback`] 的上下文。
#[derive(Debug, Clone)]
pub struct RetryContext {
    /// 失败的 URL。
    pub url: String,
    /// 已执行的重试次数（0 = 首次失败）。
    pub attempt: u32,
    /// HTTP 状态码，若失败来自响应。
    pub status: Option<u16>,
    /// 预先分类的错误类别。
    pub error_class: RequestErrorClass,
    /// 生效的策略。
    pub policy: RetryPolicy,
}

impl RetryContext {
    /// 从状态码构建一个上下文，自动对其分类。
    pub fn from_status(url: impl Into<String>, attempt: u32, status: u16, policy: RetryPolicy) -> Self {
        Self {
            url: url.into(),
            attempt,
            status: Some(status),
            error_class: classify_status(status),
            policy,
        }
    }

    /// 默认决策：委托给策略的 `decide`。
    pub fn default_decision(&self) -> RetryDecision {
        self.policy.decide(self.attempt, self.error_class)
    }
}

/// 一个可插拔的重试回调。
///
/// 领域层提供纯决策；适配器在尝试之间调用该回调。保留为装箱的
/// 闭包，以便宿主注入自定义逻辑（例如遵守 `Retry-After` 头部）。
pub type RetryCallback = Box<dyn Fn(&RetryContext) -> RetryDecision + Send + Sync>;

/// 返回默认的重试回调，它简单地应用策略。
///
/// 当未提供自定义回调时，它给出内置重试策略的
/// 默认决策。
pub fn default_retry_callback() -> RetryCallback {
    Box::new(|ctx: &RetryContext| ctx.default_decision())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_scheduler_basic() {
        let mut scheduler = RequestScheduler::new();
        let request = Request::new(
            "https://example.com/tile.b3dm".to_string(),
            RequestType::Tiles3D,
        );

        let id = scheduler.schedule(request).unwrap();
        assert_eq!(scheduler.active_request_count(), 1);

        scheduler.complete(id);
        assert_eq!(scheduler.active_request_count(), 0);
    }

    #[test]
    fn test_server_throttling() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests_per_server = 2;

        // 向同一服务器调度 2 个请求
        for i in 0..2 {
            let request = Request::throttled(
                format!("https://example.com/tile{}.b3dm", i),
                RequestType::Tiles3D,
                i as f64,
            );
            scheduler.schedule(request).unwrap();
        }
        scheduler.update();

        // 第三个请求应为待定（非活动）
        let request = Request::throttled(
            "https://example.com/tile2.b3dm".to_string(),
            RequestType::Tiles3D,
            2.0,
        );
        scheduler.schedule(request).unwrap();

        // 应只有 2 个为活动（服务器键包含默认端口）
        assert!(scheduler.server_has_open_slots("example.com:443", 0));
        assert!(!scheduler.server_has_open_slots("example.com:443", 1));
    }

    #[test]
    fn test_priority_ordering() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 1;
        scheduler.throttle_requests = true;

        // 第一个请求立即激活
        let r1 = Request::throttled(
            "https://a.com/1".to_string(),
            RequestType::Other,
            10.0,
        );
        scheduler.schedule(r1).unwrap();

        // 这些应为待定
        let r2 = Request::throttled(
            "https://b.com/2".to_string(),
            RequestType::Other,
            5.0, // 更高优先级（值更低）
        );
        let r3 = Request::throttled(
            "https://c.com/3".to_string(),
            RequestType::Other,
            1.0, // 最高优先级
        );
        scheduler.schedule(r2).unwrap();
        scheduler.schedule(r3).unwrap();

        assert_eq!(scheduler.pending_request_count(), 2);
    }

    /// H1 评审修复：一个处于优先级堆顶的饱和服务器，绝不能拖慢
    /// 发往其他有空闲槽位服务器的待定请求的激活（修复前的
    /// “任何满则 break”会使它们挨饿）。
    #[test]
    fn update_activates_other_server_when_heap_top_server_is_saturated() {
        let mut sched = RequestScheduler::new();
        sched.throttle_requests = true;
        sched.maximum_requests = 50; // 宽裕的全局预算
        sched.maximum_requests_per_server = 1; // 每服务器一个活动
        sched.priority_heap_length = 20;

        // 用每个服务器各一个活动请求使两个服务器都饱和。
        let a1 = sched
            .schedule(Request::throttled(
                "https://servera.example/a1".into(),
                RequestType::Terrain,
                0.0,
            ))
            .unwrap();
        let b0 = sched
            .schedule(Request::throttled(
                "https://serverb.example/b0".into(),
                RequestType::Terrain,
                0.0,
            ))
            .unwrap();
        assert_eq!(sched.get_request(a1).unwrap().state, RequestState::Active);
        assert_eq!(sched.get_request(b0).unwrap().state, RequestState::Active);

        // 这两个进入待定堆（两个服务器当前都满）。
        // serverA 的请求拥有更好的优先级（0.0 < 1.0）因此位于
        // 堆顶——正是修复前使其他请求挨饿的那个位置。
        let a2 = sched
            .schedule(Request::throttled(
                "https://servera.example/a2".into(),
                RequestType::Terrain,
                0.0,
            ))
            .unwrap();
        let b1 = sched
            .schedule(Request::throttled(
                "https://serverb.example/b1".into(),
                RequestType::Terrain,
                1.0,
            ))
            .unwrap();
        assert_eq!(sched.pending_request_count(), 2);

        // 释放 serverB（完成 b0）；serverA 保持饱和（a1 仍活动）。
        assert!(sched.complete(b0));

        sched.update();

        // serverB 的待定请求（b1）必须激活，即使堆顶
        // （a2）属于仍处于饱和的 serverA。
        assert_eq!(
            sched.get_request(b1).map(|r| r.state),
            Some(RequestState::Active),
            "serverB pending request must not be starved by a saturated serverA at the heap top"
        );
        // serverA 的 a2 本帧被跳过并停放到延迟队列。
        assert!(
            sched.get_request(a2).is_none(),
            "a2 leaves the active map (deferred) rather than activating on a full server"
        );
        assert_eq!(sched.deferred_count(), 1, "a2 parked in deferred for a later frame");
        assert_eq!(sched.deferred_requests()[0].id, a2);
    }

    /// M1 评审修复：`Ord::cmp` 与 `PartialEq::eq` 必须即使对于 NaN
    /// 优先级也保持一致（`a.cmp(b) == Equal <=> a == b`）。
    #[test]
    fn prioritized_request_ord_eq_consistent_for_nan() {
        let nan = f64::NAN;
        let a = PrioritizedRequest { id: RequestId(1), priority: nan };
        let b = PrioritizedRequest { id: RequestId(2), priority: nan };
        assert_eq!(
            a.cmp(&b) == Ordering::Equal,
            a == b,
            "Ord/Eq consistency law must hold for NaN"
        );
        // NaN 与一个有限值相比：严格排序，从不 Equal（修复前的 cmp
        // 对 NaN 比较返回 Equal，破坏了堆排序）。
        let c = PrioritizedRequest { id: RequestId(3), priority: 0.0 };
        assert_ne!(a.cmp(&c), Ordering::Equal);
        assert!(a != c);
        // sanitize_priority 将 NaN 沉到底部值。
        assert_eq!(sanitize_priority(nan), f64::MAX);
        assert_eq!(sanitize_priority(2.5), 2.5);
    }

    /// M1 评审修复：一个 NaN 优先级的请求会沉到调度堆的底部（最后
    /// 激活）而不破坏格式良好的优先级的排序。NaN 可通过复合
    /// 优先级将 `f64::MAX` 量级的各分量求和得到的 `inf + (-inf)` 达到。
    #[test]
    fn nan_priority_request_activates_last_without_corrupting_heap() {
        let mut sched = RequestScheduler::new();
        sched.throttle_requests = true;
        sched.maximum_requests = 1; // 一次排空一个 → 激活顺序 == 堆顺序
        sched.maximum_requests_per_server = 50;
        sched.priority_heap_length = 20;

        let nan = f64::INFINITY + f64::NEG_INFINITY;
        assert!(nan.is_nan());

        let mk = |prio: f64| {
            let mut r =
                Request::throttled("https://s.example/t".into(), RequestType::Terrain, prio);
            r.throttle_by_server = false; // 隔离全局预算排序
            r
        };

        // 第一个（5.0）激活（预算 1）；其余堆入堆。
        let id_first = sched.schedule(mk(5.0)).unwrap();
        let id_nan = sched.schedule(mk(nan)).unwrap();
        let id_low = sched.schedule(mk(1.0)).unwrap();
        let id_mid = sched.schedule(mk(3.0)).unwrap();
        assert_eq!(sched.get_request(id_first).unwrap().state, RequestState::Active);
        assert_eq!(sched.pending_request_count(), 3, "nan/1.0/3.0 all pending");

        // 一次排空一个，记录激活顺序。
        let mut order = Vec::new();
        let mut current = id_first;
        loop {
            sched.complete(current);
            sched.update();
            let next = [id_low, id_mid, id_nan].into_iter().find(|id| {
                sched
                    .get_request(*id)
                    .map(|r| r.state == RequestState::Active)
                    .unwrap_or(false)
            });
            match next {
                Some(id) => {
                    order.push(id);
                    current = id;
                }
                None => break,
            }
        }
        // 最优优先的堆顺序：1.0、3.0，然后 NaN 沉到底部（最后）。
        assert_eq!(
            order,
            vec![id_low, id_mid, id_nan],
            "NaN priority must activate LAST (sunk to heap bottom), well-formed priorities keep their order"
        );
    }

    #[test]
    fn test_resource_build_url() {
        let resource = Resource::new("https://example.com/api")
            .with_query("key", "value")
            .with_query("format", "json");

        let url = resource.build_url();
        assert!(url.starts_with("https://example.com/api?"));
        assert!(url.contains("key=value"));
        assert!(url.contains("format=json"));
    }

    #[test]
    fn test_resource_derive() {
        let base = Resource::new("https://example.com/tileset.json");
        let derived = base.derive("tiles/tile.b3dm");
        assert_eq!(derived.url, "https://example.com/tiles/tile.b3dm");
    }

    #[test]
    fn test_extract_server_key() {
        assert_eq!(
            extract_server_key("https://example.com:443/path"),
            "example.com:443"
        );
        assert_eq!(
            extract_server_key("http://localhost:8080/api"),
            "localhost:8080"
        );
    }

    // ─── 门③: query multi-value ──────────────────────────────────────────

    #[test]
    fn test_append_and_remove_query_values() {
        let mut resource = Resource::new("https://example.com/api");
        resource.append_query_values(&[
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ]);
        assert_eq!(resource.query_parameters.len(), 2);
        // 覆盖已存在的键。
        resource.append_query_values(&[("a".to_string(), "9".to_string())]);
        assert_eq!(resource.query_parameters.get("a").unwrap(), "9");
        resource.remove_query_values(&["a"]);
        assert!(!resource.query_parameters.contains_key("a"));
        assert!(resource.query_parameters.contains_key("b"));
    }

    #[test]
    fn test_query_string_is_sorted_and_multi_key() {
        let resource = Resource::new("https://example.com/tile")
            .with_query("z", "3")
            .with_query("x", "1")
            .with_query("y", "2");
        let url = resource.build_url();
        // 为确定性排序：x、y、z。
        assert!(url.find("x=1").unwrap() < url.find("y=2").unwrap());
        assert!(url.find("y=2").unwrap() < url.find("z=3").unwrap());
    }

    #[test]
    fn test_set_query_parameters_default_preserves_existing() {
        let mut resource = Resource::new("https://example.com/api").with_query("k", "original");
        resource.set_query_parameters(
            vec![
                ("k".to_string(), "new".to_string()),
                ("j".to_string(), "added".to_string()),
            ],
            true,
        );
        // use_as_default=true：保留已存在的键，添加新键。
        assert_eq!(resource.query_parameters.get("k").unwrap(), "original");
        assert_eq!(resource.query_parameters.get("j").unwrap(), "added");
    }

    // ─── URI 分类 / 基础 uri ─────────────────────────────────────────────

    #[test]
    fn test_is_data_and_blob_uri() {
        assert!(Resource::new("data:text/plain;base64,SGk=").is_data_uri());
        assert!(!Resource::new("https://example.com/x").is_data_uri());
        assert!(Resource::new("blob:https://example.com/uuid").is_blob_uri());
    }

    #[test]
    fn test_get_base_uri() {
        let resource = Resource::new("https://example.com:8080/a/b/c.json");
        assert_eq!(resource.get_base_uri(), "https://example.com:8080/");
    }

    #[test]
    fn test_clone_with_url_preserves_state() {
        let resource = Resource::new("https://example.com/a")
            .with_query("k", "v")
            .with_header("X-Token", "abc");
        let cloned = resource.clone_with_url("https://other.com/b");
        assert_eq!(cloned.url, "https://other.com/b");
        assert_eq!(cloned.query_parameters.get("k").unwrap(), "v");
        assert_eq!(cloned.headers.get("X-Token").unwrap(), "abc");
    }

    #[test]
    fn test_resource_display() {
        let resource = Resource::new("https://example.com/api").with_query("k", "v");
        assert_eq!(format!("{}", resource), "https://example.com/api?k=v");
    }

    // ─── FetchDescriptor 构造 ────────────────────────────────────────────

    #[test]
    fn test_fetch_descriptor_get() {
        let resource = Resource::new("https://example.com/tile.b3dm");
        let descriptor = resource.fetch_array_buffer(None);
        assert_eq!(descriptor.method, HttpMethod::Get);
        assert_eq!(descriptor.response_type, ResponseType::ArrayBuffer);
        assert_eq!(descriptor.url, "https://example.com/tile.b3dm");
        assert_eq!(descriptor.server_key, "example.com:443");
        assert!(!descriptor.is_data_uri);
    }

    #[test]
    fn test_fetch_descriptor_response_type_variants() {
        let resource = Resource::new("https://example.com/x");
        assert_eq!(resource.fetch_json(None).response_type, ResponseType::Json);
        assert_eq!(resource.fetch_text(None).response_type, ResponseType::Text);
        assert_eq!(resource.fetch_image(None).response_type, ResponseType::Image);
        assert_eq!(resource.fetch_blob(None).response_type, ResponseType::Blob);
    }

    #[test]
    fn test_fetch_descriptor_data_uri_short_circuits_proxy() {
        let resource = Resource::new("data:application/octet-stream;base64,QUJD");
        let proxy = crate::proxy::ProxyPolicy::with_proxy(crate::proxy::DefaultProxy::new(
            "https://proxy.example.com/".to_string(),
        ));
        let descriptor = resource.fetch_array_buffer(Some(&proxy));
        // Data URI：不应用代理，服务器键为空，标记为内联。
        assert!(descriptor.is_data_uri);
        assert!(descriptor.url.starts_with("data:"));
        assert!(descriptor.server_key.is_empty());
    }

    #[test]
    fn test_fetch_descriptor_applies_proxy_for_untrusted() {
        let resource = Resource::new("https://untrusted.com/tile.png");
        let proxy = crate::proxy::ProxyPolicy::with_proxy(crate::proxy::DefaultProxy::new(
            "https://proxy.example.com/".to_string(),
        ));
        let descriptor = resource.fetch_image(Some(&proxy));
        assert!(descriptor.url.contains("proxy.example.com"));
        assert!(descriptor.url.contains("untrusted.com"));
    }

    #[test]
    fn test_post_descriptor_carries_body() {
        let resource = Resource::new("https://example.com/service");
        let descriptor = resource.post(vec![1, 2, 3], None);
        assert_eq!(descriptor.method, HttpMethod::Post);
        assert_eq!(descriptor.body, Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_descriptor_to_request_server_key() {
        let resource = Resource::new("https://example.com/tile.b3dm");
        let descriptor = resource
            .fetch_array_buffer(None)
            .with_request_type(RequestType::Tiles3D)
            .with_priority(5.0);
        let request = descriptor.to_request();
        assert_eq!(request.request_type, RequestType::Tiles3D);
        assert_eq!(request.priority, 5.0);
        assert_eq!(request.server_key, "example.com:443");
    }

    // ─── 重试语义 ─────────────────────────────────────────────────────────

    #[test]
    fn test_retry_policy_default_single_attempt() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_attempts, 1);
        // 首次瞬时失败立即重试（无基础延迟）。
        assert_eq!(
            policy.decide(0, RequestErrorClass::Transient),
            RetryDecision::RetryNow
        );
        // 第二次尝试超出 max_attempts=1。
        assert_eq!(
            policy.decide(1, RequestErrorClass::Transient),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn test_retry_policy_permanent_never_retries() {
        let policy = RetryPolicy::exponential(5, 100);
        assert_eq!(
            policy.decide(0, RequestErrorClass::Permanent),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn test_retry_policy_exponential_backoff_delays() {
        let policy = RetryPolicy::exponential(5, 100);
        assert_eq!(
            policy.decide(0, RequestErrorClass::Transient),
            RetryDecision::RetryAfterMillis(100)
        );
        assert_eq!(
            policy.decide(1, RequestErrorClass::Transient),
            RetryDecision::RetryAfterMillis(200)
        );
        assert_eq!(
            policy.decide(2, RequestErrorClass::Transient),
            RetryDecision::RetryAfterMillis(400)
        );
    }

    #[test]
    fn test_retry_policy_delay_capped_at_max() {
        let mut policy = RetryPolicy::exponential(30, 1000);
        policy.max_delay_millis = 5000;
        assert_eq!(policy.delay_for(20), 5000);
    }

    #[test]
    fn test_retry_policy_throttled_honours_flag() {
        let mut policy = RetryPolicy::exponential(3, 100);
        assert_eq!(
            policy.decide(0, RequestErrorClass::Throttled),
            RetryDecision::RetryAfterMillis(100)
        );
        policy.retry_on_throttled = false;
        assert_eq!(
            policy.decide(0, RequestErrorClass::Throttled),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn test_classify_status() {
        assert_eq!(classify_status(429), RequestErrorClass::Throttled);
        assert_eq!(classify_status(404), RequestErrorClass::Permanent);
        assert_eq!(classify_status(500), RequestErrorClass::Transient);
        assert_eq!(classify_status(503), RequestErrorClass::Transient);
    }

    #[test]
    fn test_default_retry_callback_applies_policy() {
        let callback = default_retry_callback();
        let ctx = RetryContext::from_status("https://x.com/a", 0, 500, RetryPolicy::default());
        assert_eq!(callback(&ctx), RetryDecision::RetryNow);

        let ctx404 = RetryContext::from_status("https://x.com/a", 0, 404, RetryPolicy::default());
        assert_eq!(callback(&ctx404), RetryDecision::GiveUp);
    }

    // ─── 门③: statistics integration ─────────────────────────────────────

    #[test]
    fn test_scheduler_statistics_success() {
        let mut scheduler = RequestScheduler::new();
        let id = scheduler
            .schedule(Request::new(
                "https://example.com/a.b3dm".to_string(),
                RequestType::Tiles3D,
            ))
            .unwrap();
        assert_eq!(scheduler.statistics().attempted, 1);
        assert_eq!(scheduler.statistics().active, 1);
        scheduler.complete(id);
        assert_eq!(scheduler.statistics().succeeded, 1);
        assert_eq!(scheduler.statistics().active, 0);
        assert_eq!(
            scheduler.statistics().active_for_type(RequestType::Tiles3D),
            0
        );
    }

    #[test]
    fn test_scheduler_statistics_failure() {
        let mut scheduler = RequestScheduler::new();
        let id = scheduler
            .schedule(Request::new(
                "https://example.com/a".to_string(),
                RequestType::Imagery,
            ))
            .unwrap();
        scheduler.fail(id);
        assert_eq!(scheduler.statistics().failed, 1);
        assert_eq!(scheduler.statistics().active, 0);
    }

    #[test]
    fn test_scheduler_statistics_cancel_active_vs_pending() {
        // 活动请求的取消。
        let mut scheduler = RequestScheduler::new();
        let id = scheduler
            .schedule(Request::new(
                "https://example.com/a".to_string(),
                RequestType::Other,
            ))
            .unwrap();
        scheduler.cancel(id);
        assert_eq!(scheduler.statistics().cancelled_active, 1);

        // 待定请求的取消（由 maximum_requests=0 强制入堆）。
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 0;
        let id = scheduler
            .schedule(Request::throttled(
                "https://example.com/b".to_string(),
                RequestType::Other,
                1.0,
            ))
            .unwrap();
        scheduler.cancel(id);
        assert_eq!(scheduler.statistics().cancelled_pending, 1);
    }

    #[test]
    fn test_scheduler_statistics_reset() {
        let mut scheduler = RequestScheduler::new();
        scheduler
            .schedule(Request::new(
                "https://example.com/a".to_string(),
                RequestType::Other,
            ))
            .unwrap();
        scheduler.reset_statistics();
        assert_eq!(scheduler.statistics().attempted, 0);
    }

    // ─── 延迟提升 ──────────────────────────────────────────────

    #[test]
    fn test_scheduler_deferred_when_heap_saturated() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 0; // 无任何请求可激活
        scheduler.priority_heap_length = 1;

        // 第一个被限流的请求填满堆（优先级 1.0）。
        scheduler
            .schedule(Request::throttled(
                "https://example.com/a".to_string(),
                RequestType::Other,
                1.0,
            ))
            .unwrap();
        assert_eq!(scheduler.pending_request_count(), 1);

        // 第二个请求拥有**更优**的优先级（0.5 < 1.0），因此根据
        // 内部优先级队列的插入规则，它会把最差的驻留者
        // （p=1.0 的请求）逐出到 M8.1 延迟队列中，并占据其堆
        // 槽位。正是这条逐出路径让延迟-提升保持活跃，
        // 同时仍遵守规范对更差优先级
        // 新来者的拒绝（见 `test_scheduler_rejects_worse_priority_when_heap_full`）。
        scheduler
            .schedule(Request::throttled(
                "https://example.com/b".to_string(),
                RequestType::Other,
                0.5,
            ))
            .unwrap();
        assert_eq!(scheduler.deferred_count(), 1);
        assert_eq!(scheduler.pending_request_count(), 1);

        // 扩大堆并更新会把延迟的请求重新提升回来。
        scheduler.priority_heap_length = 2;
        scheduler.update();
        assert_eq!(scheduler.deferred_count(), 0);
        assert_eq!(scheduler.pending_request_count(), 2);
    }

    #[test]
    fn test_scheduler_rejects_when_deferred_saturated() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 0;
        scheduler.priority_heap_length = 1;
        scheduler.maximum_deferred = 1;

        // r1（p=1.0）填满堆。
        scheduler
            .schedule(Request::throttled(
                "https://example.com/a".to_string(),
                RequestType::Other,
                1.0,
            ))
            .unwrap();
        // r2（p=0.5）更优 → 将 r1 逐出到延迟队列（现已满）。
        scheduler
            .schedule(Request::throttled(
                "https://example.com/b".to_string(),
                RequestType::Other,
                0.5,
            ))
            .unwrap();
        assert_eq!(scheduler.deferred_count(), 1);
        // r3（p=2.0）比堆中驻留者（p=0.5）更差 → 无论是否有延迟余量，
        // 都会被优先级规则直接拒绝。
        assert!(scheduler
            .schedule(Request::throttled(
                "https://example.com/c".to_string(),
                RequestType::Other,
                2.0,
            ))
            .is_none());
    }

    /// 优先级拒绝规则：当一个新来请求的优先级相对于已饱和堆中
    /// 最差的驻留者**并非严格更优**时，会被直接拒绝（绝不延迟保留）。
    /// 这正是以下测试所断言的规则：
    /// `specs/tests/core/request_scheduler_spec.rs::honors_priority_heap_length`
    /// 和 `::handles_low_priority_requests`。
    #[test]
    fn test_scheduler_rejects_worse_priority_when_heap_full() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 0;
        scheduler.priority_heap_length = 1;

        scheduler
            .schedule(Request::throttled(
                "https://example.com/a".to_string(),
                RequestType::Other,
                0.0,
            ))
            .unwrap();
        // 更差的优先级（1.0 > 0.0）→ 被拒绝，而非延迟。
        assert!(scheduler
            .schedule(Request::throttled(
                "https://example.com/b".to_string(),
                RequestType::Other,
                1.0,
            ))
            .is_none());
        assert_eq!(scheduler.deferred_count(), 0);
        assert_eq!(scheduler.pending_request_count(), 1);

        // 相等的优先级（0.0 == 0.0）同样被拒绝（并非*严格*更优）。
        assert!(scheduler
            .schedule(Request::throttled(
                "https://example.com/c".to_string(),
                RequestType::Other,
                0.0,
            ))
            .is_none());
    }

    // ─── 优先级函数集成 ───────────────────────────────────

    #[test]
    fn test_scheduler_priority_function_recomputes() {
        use crate::priority::{FrameContext, PriorityKey, SsedPriority};

        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 0; // 让全部请求保持待定
        scheduler.set_priority_function(Box::new(SsedPriority::new()));
        assert_eq!(scheduler.priority_function_name(), Some("SSED"));

        let key = PriorityKey::new(0, 0, 15)
            .with_center(0.0, 0.0, 6_378_137.0 - 5000.0)
            .with_geometric_error(50.0);
        let id = scheduler
            .schedule(
                Request::throttled(
                    "https://example.com/tile".to_string(),
                    RequestType::Tiles3D,
                    999.0,
                )
                .with_priority_key(key),
            )
            .unwrap();

        let context = FrameContext::new().with_camera(0.0, 0.0, 6_378_137.0);
        scheduler.update_with_context(&context);

        // 存储的优先级已被 SSED 函数重新计算（不再是 999）。
        let request = scheduler.get_request(id).unwrap();
        assert!((request.priority - 999.0).abs() > f64::EPSILON);
    }

    #[test]
    fn test_scheduler_update_without_priority_function_is_noop_recompute() {
        let mut scheduler = RequestScheduler::new();
        scheduler.maximum_requests = 0;
        let id = scheduler
            .schedule(Request::throttled(
                "https://example.com/a".to_string(),
                RequestType::Other,
                7.0,
            ))
            .unwrap();
        // 未安装优先级函数：优先级不变。
        scheduler.update_with_context(&crate::priority::FrameContext::new());
        assert_eq!(scheduler.get_request(id).unwrap().priority, 7.0);
    }
}
