//! 用于请求调度的可插拔优先级函数。
//!
//! 覆盖调度器优先级堆的排序依据，以及瓦片集/四叉树/地形
//! 提供者中常见的逐请求优先级回调模式。
//!
//! 优先级函数为每个请求返回一个数值优先级（越低 = 优先级
//! 越高）。调度器使用一个按此值排序的最小堆来决定将哪些待定请求提升。
//!
//! 本模块提供：
//! - 一个用于可插拔优先级计算的 [`PriorityFunction`] trait。
//! - 一个基于屏幕空间误差距离（SSED）的默认实现 [`SsedPriority`]
//!   —— 与 3D Tiles 和地形请求优先级排序所用的度量相同。
//! - 一个使用简单距离衰减的替代方案 [`DistanceDecayPriority`]。
//! - 一个组合多个优先级信号的 [`CompositePriority`]。
//!
//! **纯领域逻辑** —— 无 IO，无框架依赖，全程使用 f64。

use std::fmt::Debug;

/// 用于计算请求优先级的 trait。
///
/// 调度器每帧为每个待定/被限流的请求调用一次本函数，
/// 以决定提升顺序。值越低 = 优先级越高（匹配最小堆语义）。
///
/// 典型用法：为每个请求挂上一个闭包，依据 SSE/距离实时
/// 计算并返回其当前优先级。
pub trait PriorityFunction: Send + Sync + Debug {
    /// 为以 `key` 标识的请求计算优先级。
    ///
    /// 值越低表示优先级越高（最先从堆中提升）。
    /// `context` 提供屏幕空间计算所需的帧状态信息（相机位置等）。
    ///
    /// 返回 `f64::MAX` 可将一个请求有效地降级到堆的
    /// 底部（例如对于不再相关的请求）。
    fn compute_priority(&self, key: &PriorityKey, context: &FrameContext) -> f64;

    /// 返回本优先级函数的名称（用于诊断）。
    fn name(&self) -> &str;
}

/// 为一个请求标识以供优先级计算。
///
/// 足够通用，能覆盖地形瓦片、影像瓦片和 3D Tiles 内容。
#[derive(Debug, Clone, PartialEq)]
pub struct PriorityKey {
    /// 给定缩放级别下的瓦片坐标 (x, y)。
    pub x: u32,
    pub y: u32,
    /// 缩放级别（0 = 根）。
    pub zoom: u32,

    /// 世界坐标中的包围体积中心（ECEF 米，f64）。
    /// 用于基于距离的优先级计算。
    pub center_x: f64,
    pub center_y: f64,
    pub center_z: f64,

    /// 此瓦片的几何误差（米，用于 SSE 计算）。
    /// 表示瓦片在自身层级下残留的最大投影误差。
    pub geometric_error: f64,

    /// 额外的用户自定义权重乘子（默认 1.0）。
    /// 允许宿主对特定瓦片加偏（例如中心视域提升）。
    pub weight: f64,
}

impl PriorityKey {
    /// 创建一个仅含瓦片坐标的最小优先级键。
    pub fn new(x: u32, y: u32, zoom: u32) -> Self {
        Self {
            x,
            y,
            zoom,
            center_x: 0.0,
            center_y: 0.0,
            center_z: 0.0,
            geometric_error: 0.0,
            weight: 1.0,
        }
    }

    /// 设置包围体积中心（ECEF 米）。
    pub fn with_center(mut self, x: f64, y: f64, z: f64) -> Self {
        self.center_x = x;
        self.center_y = y;
        self.center_z = z;
        self
    }

    /// 设置几何误差（米）。
    pub fn with_geometric_error(mut self, error: f64) -> Self {
        self.geometric_error = error;
        self
    }

    /// 设置权重乘子。
    pub fn with_weight(mut self, weight: f64) -> Self {
        self.weight = weight;
        self
    }
}

/// 用于优先级计算的逐帧上下文。
///
/// 包含屏幕空间误差计算所需的相机状态和视口尺寸。
/// 由宿主（Bevy 系统）每帧更新一次。
#[derive(Debug, Clone)]
pub struct FrameContext {
    /// 世界坐标中的相机位置（ECEF 米，f64）。
    pub camera_x: f64,
    pub camera_y: f64,
    pub camera_z: f64,

    /// 视口宽度（像素）。
    pub viewport_width: f64,

    /// 视口高度（像素）。
    pub viewport_height: f64,

    /// 垂直视场角（弧度）。
    pub fov_y: f64,

    /// 最大屏幕空间误差阈值（像素）。
    /// SSE 高于此值的瓦片会被细化；低于此值则足够。
    ///
    /// 典型默认值为 16 像素，在质量与请求量之间权衡。
    pub maximum_screen_space_error: f64,

    /// 当前帧索引（单调递增，用于过期启发式）。
    pub frame_index: u64,
}

impl FrameContext {
    /// 创建一个使用典型值的默认帧上下文。
    pub fn new() -> Self {
        Self {
            camera_x: 0.0,
            camera_y: 0.0,
            camera_z: 6_378_137.0, // 默认：赤道上方 1 个地球半径处
            viewport_width: 1920.0,
            viewport_height: 1080.0,
            fov_y: std::f64::consts::FRAC_PI_3, // 60 度
            maximum_screen_space_error: 16.0,
            frame_index: 0,
        }
    }

    /// 设置相机位置。
    pub fn with_camera(mut self, x: f64, y: f64, z: f64) -> Self {
        self.camera_x = x;
        self.camera_y = y;
        self.camera_z = z;
        self
    }

    /// 设置视口尺寸。
    pub fn with_viewport(mut self, width: f64, height: f64) -> Self {
        self.viewport_width = width;
        self.viewport_height = height;
        self
    }

    /// 设置视场角。
    pub fn with_fov(mut self, fov_y: f64) -> Self {
        self.fov_y = fov_y;
        self
    }

    /// 设置最大屏幕空间误差。
    pub fn with_max_sse(mut self, sse: f64) -> Self {
        self.maximum_screen_space_error = sse;
        self
    }

    /// 计算从相机到一个点的距离。
    pub fn distance_to(&self, x: f64, y: f64, z: f64) -> f64 {
        let dx = self.camera_x - x;
        let dy = self.camera_y - y;
        let dz = self.camera_z - z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// 计算在给定距离处一个瓦片的屏幕空间误差。
    ///
    /// 公式：
    /// ```text
    /// sse = (geometricError * screenHeight) / (distance * 2 * tan(fovY / 2))
    /// ```
    ///
    /// 这是在透视投影下、假设瓦片的几何误差为世界空间中
    /// 一个长度的简化形式。
    pub fn screen_space_error(&self, geometric_error: f64, distance: f64) -> f64 {
        if distance <= 0.0 || geometric_error <= 0.0 {
            return f64::MAX;
        }
        let sse_denom = 2.0 * (self.fov_y / 2.0).tan();
        if sse_denom <= 0.0 {
            return f64::MAX;
        }
        (geometric_error * self.viewport_height) / (distance * sse_denom)
    }
}

impl Default for FrameContext {
    /// 默认帧上下文，等价于 [`FrameContext::new`]。
    fn default() -> Self {
        Self::new()
    }
}

// ── SSED 优先级（屏幕空间误差距离） ────────────────────────────

/// 基于屏幕空间误差距离（SSED）的默认优先级函数。
///
/// 这是 3D Tiles 与地形瓦片优先级排序所用的
/// 同一度量：屏幕空间误差越高（即视觉影响更大的细化）的瓦片
/// 获得越高优先级（数值越低）。
///
/// 优先级公式：
/// ```text
/// priority = max(0, maximumSSE - computedSSE) * weight
/// ```
///
/// - 若 `computedSSE >= maximumSSE`：priority = 0（最高 —— 瓦片必须细化）
/// - 若 `computedSSE < maximumSSE`：priority > 0（瓦片已经足够，
///   但我们仍为未来的相机移动加载它；SSE 越低 = 优先级越低）
///
/// 该度量同时服务于四叉树瓦片排序与瓦片集的 SSE 处理。
#[derive(Debug, Clone)]
pub struct SsedPriority {
    /// 应用于计算出的优先级的乘子（默认 1.0）。
    pub scale: f64,
}

impl SsedPriority {
    /// 创建一个使用默认 scale 的新 SSED 优先级函数。
    pub fn new() -> Self {
        Self { scale: 1.0 }
    }

    /// 使用自定义 scale 因子创建。
    pub fn with_scale(scale: f64) -> Self {
        Self { scale }
    }
}

impl Default for SsedPriority {
    /// 默认 SSED 优先级，scale 为 1.0。
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityFunction for SsedPriority {
    /// 依据 SSE 超出阈值的程度计算优先级，再乘以 scale 与权重。
    fn compute_priority(&self, key: &PriorityKey, context: &FrameContext) -> f64 {
        // 计算从相机到瓦片中心的距离。
        let distance = context.distance_to(key.center_x, key.center_y, key.center_z);

        // 计算此距离处的屏幕空间误差。
        let sse = context.screen_space_error(key.geometric_error, distance);

        // 优先级：SSE 超出阈值多少。
        // 超出越多 → 优先级值越低 → 最先提升。
        let excess = context.maximum_screen_space_error - sse;
        let priority = if excess <= 0.0 {
            // SSE 超出阈值：必须细化，最高优先级。
            0.0
        } else {
            // SSE 低于阈值：优先级与低出多少成正比。
            excess
        };

        priority * self.scale * key.weight
    }

    /// 返回本优先级函数的诊断名称。
    fn name(&self) -> &str {
        "SSED"
    }
}

// ── 距离衰减优先级 ──────────────────────────────────────

/// 简单的基于距离的优先级：越近的瓦片优先级越高。
///
/// 公式：`priority = distance / reference_distance * weight`
///
/// 适用于几何误差不具意义的影像层（给定缩放下
/// 所有瓦片都有相同的误差），但靠近相机的程度
/// 决定了视觉重要性。
///
/// 适用于几何误差无区分度的影像层：以单纯的距离衰减
/// 作为更轻量的优先级启发式。
#[derive(Debug, Clone)]
pub struct DistanceDecayPriority {
    /// 用于归一化的参考距离（在此距离处 priority = 1.0）。
    /// 默认：10_000_000.0 米（约 1.5 个地球半径）。
    pub reference_distance: f64,
}

impl DistanceDecayPriority {
    /// 使用默认参考距离创建。
    pub fn new() -> Self {
        Self {
            reference_distance: 10_000_000.0,
        }
    }

    /// 使用自定义参考距离创建。
    pub fn with_reference(reference_distance: f64) -> Self {
        Self { reference_distance }
    }
}

impl Default for DistanceDecayPriority {
    /// 默认距离衰减优先级，参考距离 1e7 米。
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityFunction for DistanceDecayPriority {
    /// 以 distance/reference_distance 归一化后乘以权重。
    fn compute_priority(&self, key: &PriorityKey, context: &FrameContext) -> f64 {
        let distance = context.distance_to(key.center_x, key.center_y, key.center_z);
        let normalized = if self.reference_distance > 0.0 {
            distance / self.reference_distance
        } else {
            f64::MAX
        };
        normalized * key.weight
    }

    /// 返回本优先级函数的诊断名称。
    fn name(&self) -> &str {
        "DistanceDecay"
    }
}

// ── 复合优先级 ──────────────────────────────────────────

/// 通过加权混合组合多个优先级函数。
///
/// 最终优先级是所有组成优先级的加权和：
/// ```text
/// priority = Σ (weight_i * function_i.compute_priority(key, context))
/// ```
///
/// 这使得宿主可以将 SSED（用于视觉细化的紧迫性）与
/// 距离衰减（用于空间局部性）及自定义启发式相结合。
#[derive(Debug)]
pub struct CompositePriority {
    /// 组成部分列表，每项为 (权重, 优先级函数)。
    components: Vec<(f64, Box<dyn PriorityFunction>)>,
}

impl CompositePriority {
    /// 创建一个空的复合体。
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// 以给定的权重添加一个优先级函数。
    pub fn add(mut self, weight: f64, function: Box<dyn PriorityFunction>) -> Self {
        self.components.push((weight, function));
        self
    }

    /// 返回组成部分函数的数量。
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// 返回复合体是否没有任何组成部分。
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

impl Default for CompositePriority {
    /// 默认空的复合优先级。
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityFunction for CompositePriority {
    /// 返回各组成部分优先级的加权和；空复合体返回 0。
    fn compute_priority(&self, key: &PriorityKey, context: &FrameContext) -> f64 {
        if self.components.is_empty() {
            return 0.0;
        }
        self.components
            .iter()
            .map(|(weight, func)| weight * func.compute_priority(key, context))
            .sum()
    }

    /// 返回本优先级函数的诊断名称。
    fn name(&self) -> &str {
        "Composite"
    }
}

// ── 静态优先级（用于测试 / 简单情况） ──────────────────────

/// 一个总是返回固定值的优先级函数。
///
/// 适用于测试，以及应用于拥有相同优先级的请求
/// （堆内的 FIFO 顺序）。
#[derive(Debug, Clone)]
pub struct StaticPriority {
    /// 总是返回的固定优先级值。
    value: f64,
}

impl StaticPriority {
    /// 使用给定的固定值创建一个静态优先级。
    pub fn new(value: f64) -> Self {
        Self { value }
    }
}

impl PriorityFunction for StaticPriority {
    /// 忽略 key/context，恒定返回固定值。
    fn compute_priority(&self, _key: &PriorityKey, _context: &FrameContext) -> f64 {
        self.value
    }

    /// 返回本优先级函数的诊断名称。
    fn name(&self) -> &str {
        "Static"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> FrameContext {
        FrameContext::new()
            .with_camera(0.0, 0.0, 6_378_137.0)
            .with_viewport(1920.0, 1080.0)
            .with_fov(std::f64::consts::FRAC_PI_3)
    }

    #[test]
    fn ssed_priority_closer_tile_has_higher_priority() {
        let ctx = make_context();
        let ssed = SsedPriority::new();

        // 靠近相机的瓦片（小距离 → 大 SSE → 低优先级值）
        let close = PriorityKey::new(0, 0, 15)
            .with_center(0.0, 0.0, 6_378_137.0 - 1000.0) // 相机下方 1km
            .with_geometric_error(10.0);

        // 远离相机的瓦片
        let far = PriorityKey::new(0, 0, 5)
            .with_center(0.0, 0.0, 0.0) // 地球中心
            .with_geometric_error(10.0);

        let p_close = ssed.compute_priority(&close, &ctx);
        let p_far = ssed.compute_priority(&far, &ctx);

        // 更近的瓦片应拥有更低的优先级值（更高优先级）
        assert!(
            p_close < p_far,
            "close priority {} should be < far priority {}",
            p_close,
            p_far
        );
    }

    #[test]
    fn ssed_priority_zero_when_sse_exceeds_threshold() {
        let ctx = make_context().with_max_sse(16.0);
        let ssed = SsedPriority::new();

        // 带大几何误差的非常近的瓦片 → SSE >> 阈值
        let key = PriorityKey::new(0, 0, 20)
            .with_center(0.0, 0.0, 6_378_137.0 - 100.0) // 相机下方 100m
            .with_geometric_error(1000.0);

        let priority = ssed.compute_priority(&key, &ctx);
        assert_eq!(priority, 0.0, "SSE exceeds threshold → priority must be 0");
    }

    #[test]
    fn distance_decay_closer_is_lower() {
        let ctx = make_context();
        let dd = DistanceDecayPriority::new();

        let close = PriorityKey::new(0, 0, 10)
            .with_center(0.0, 0.0, 6_378_137.0 - 100_000.0);
        let far = PriorityKey::new(0, 0, 5)
            .with_center(0.0, 0.0, 0.0);

        let p_close = dd.compute_priority(&close, &ctx);
        let p_far = dd.compute_priority(&far, &ctx);
        assert!(p_close < p_far);
    }

    #[test]
    fn distance_decay_reference_normalization() {
        let ctx = make_context();
        let dd = DistanceDecayPriority::with_reference(1_000_000.0);

        // 恰好在距相机 1M 米处的瓦片
        let key = PriorityKey::new(0, 0, 10)
            .with_center(0.0, 0.0, 6_378_137.0 - 1_000_000.0);

        let priority = dd.compute_priority(&key, &ctx);
        // 应约为 1.0（distance / reference_distance）
        assert!(
            (priority - 1.0).abs() < 0.001,
            "expected ~1.0, got {}",
            priority
        );
    }

    #[test]
    fn composite_blends_weights() {
        let ctx = make_context();
        let composite = CompositePriority::new()
            .add(2.0, Box::new(StaticPriority::new(3.0)))
            .add(1.0, Box::new(StaticPriority::new(5.0)));

        let key = PriorityKey::new(0, 0, 0);
        let priority = composite.compute_priority(&key, &ctx);
        // 2.0 * 3.0 + 1.0 * 5.0 = 11.0
        assert!((priority - 11.0).abs() < f64::EPSILON);
    }

    #[test]
    fn static_priority_always_same() {
        let ctx = make_context();
        let sp = StaticPriority::new(42.0);
        let key = PriorityKey::new(1, 2, 3).with_center(100.0, 200.0, 300.0);
        assert_eq!(sp.compute_priority(&key, &ctx), 42.0);
    }

    #[test]
    fn weight_multiplier_affects_priority() {
        let ctx = make_context();
        let dd = DistanceDecayPriority::new();

        let normal = PriorityKey::new(0, 0, 10)
            .with_center(0.0, 0.0, 6_000_000.0)
            .with_weight(1.0);
        let boosted = PriorityKey::new(0, 0, 10)
            .with_center(0.0, 0.0, 6_000_000.0)
            .with_weight(0.5); // 更低的权重 → 更低的优先级值 → 更高优先级

        let p_normal = dd.compute_priority(&normal, &ctx);
        let p_boosted = dd.compute_priority(&boosted, &ctx);
        assert!(p_boosted < p_normal);
    }

    #[test]
    fn screen_space_error_computation() {
        let ctx = FrameContext::new()
            .with_viewport(1080.0, 1080.0)
            .with_fov(std::f64::consts::FRAC_PI_2); // 90 度

        // 在距离 1000、几何误差 10 时：
        // sse = (10 * 1080) / (1000 * 2 * tan(45°)) = 10800 / 2000 = 5.4
        let sse = ctx.screen_space_error(10.0, 1000.0);
        assert!((sse - 5.4).abs() < 0.01, "expected ~5.4, got {}", sse);
    }

    #[test]
    fn screen_space_error_zero_distance_is_max() {
        let ctx = make_context();
        assert_eq!(ctx.screen_space_error(10.0, 0.0), f64::MAX);
    }

    #[test]
    fn frame_context_distance_to() {
        let ctx = FrameContext::new().with_camera(0.0, 0.0, 100.0);
        let dist = ctx.distance_to(0.0, 0.0, 0.0);
        assert!((dist - 100.0).abs() < f64::EPSILON);

        let dist2 = ctx.distance_to(3.0, 4.0, 100.0);
        assert!((dist2 - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn priority_function_names() {
        assert_eq!(SsedPriority::new().name(), "SSED");
        assert_eq!(DistanceDecayPriority::new().name(), "DistanceDecay");
        assert_eq!(StaticPriority::new(0.0).name(), "Static");
        assert_eq!(CompositePriority::new().name(), "Composite");
    }
}
