//! 样条插值系统。
//!
//! 提供多种参数化样条：线性、Catmull-Rom、Hermite（入/出切线）、
//! 四元数（SLERP）、阶梯、常量、形变权重与标量样条。
//! 它们共享 [`Spline`] trait 提供的时间轴定位、周期回绕与夹取能力，
//! 并在给定时间处各自实现相应的插值求值。

use glam::{DQuat, DVec3};

// ============================================================================
// Spline 特质
// ============================================================================

/// 通用的样条操作。
///
/// 统一提供时间节点访问与基于时间轴的区间定位、周期回绕、夹取。
pub trait Spline {
    /// 获取时间值。
    fn times(&self) -> &[f64];

    /// 查找给定时间对应的时间区间索引。
    /// 返回满足 times[i] <= time <= times[i+1] 的索引 i。
    fn find_time_interval(&self, time: f64) -> usize {
        let times = self.times();
        // 无时间节点时退化返回 0
        if times.is_empty() {
            return 0;
        }
        // time 落在首点之前 → 归入第一段
        if time <= times[0] {
            return 0;
        }
        let last = times.len() - 1;
        // time 落在末点之后 → 归入最后一段
        if time >= times[last] {
            return last.saturating_sub(1);
        }
        // 二分查找：在 [lo, hi] 上定位满足 times[mid] <= time < times[mid+1] 的段
        let mut lo = 0;
        let mut hi = last;
        while lo < hi {
            let mid = (lo + hi) / 2;
            // time 落在 [times[mid], times[mid+1]) → 命中本段
            if times[mid] <= time && time < times[mid + 1] {
                return mid;
            } else if times[mid] > time {
                // 中点已在 time 右侧 → 收缩到左半区
                hi = mid;
            } else {
                // 中点仍在 time 左侧 → 收缩到右半区
                lo = mid + 1;
            }
        }
        // 收敛后钳到最后一个合法段索引
        lo.min(last.saturating_sub(1))
    }

    /// 将时间环绕到样条的周期。
    fn wrap_time(&self, time: f64) -> f64 {
        let times = self.times();
        // 少于两个时间节点无法周期化，原样返回
        if times.len() < 2 {
            return time;
        }
        let start = times[0];
        let end = times[times.len() - 1];
        let duration = end - start;
        // 周期非正时钳回起点
        if duration <= 0.0 {
            return start;
        }
        // 取模回绕到 [start, end)；对负偏移补一个周期
        let mut t = (time - start) % duration;
        if t < 0.0 {
            t += duration;
        }
        // 回绕后平移回以 start 为起点的绝对时间
        start + t
    }

    /// 将时间夹取到样条的范围内。
    fn clamp_time(&self, time: f64) -> f64 {
        let times = self.times();
        // 空节点直接透传；否则把 time 夹取到 [首节点, 末节点]
        if times.is_empty() {
            return time;
        }
        time.clamp(times[0], times[times.len() - 1])
    }
}

// ============================================================================
// LinearSpline
// ============================================================================

/// 分段线性插值样条。
///
/// 在相邻控制点之间按归一化时间参数做直线插值，节点处仅 C0 连续。
#[derive(Debug, Clone, PartialEq)]
pub struct LinearSpline {
    /// 时间值（严格递增）。
    pub times: Vec<f64>,
    /// 控制点。
    pub points: Vec<DVec3>,
}

impl LinearSpline {
    /// 创建一个新的线性样条。
    ///
    /// 调用方需保证时间严格递增且与控制点一一对应。
    pub fn new(times: Vec<f64>, points: Vec<DVec3>) -> Self {
        // 控制点数至少 2，且时间数与点数一一对应
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");
        Self { times, points }
    }

    /// 在给定时间处求样条的值。
    pub fn evaluate(&self, time: f64) -> DVec3 {
        // 定位包围段，再按归一化参数 u 在两端点间线性插值
        let i = self.find_time_interval(time);
        let t0 = self.times[i];
        let t1 = self.times[i + 1];
        // u = (time - t0) / (t1 - t0)，区间退化时取 0
        let u = if (t1 - t0).abs() > 1e-15 {
            (time - t0) / (t1 - t0)
        } else {
            0.0
        };
        self.points[i].lerp(self.points[i + 1], u)
    }
}

impl Spline for LinearSpline {
    /// 返回时间节点序列的切片视图。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

// ============================================================================
// CatmullRomSpline
// ============================================================================

/// 用于平滑 C1 连续曲线的 Catmull-Rom 样条。
///
/// 内部段自动由相邻点估计切线，首/末段使用显式或推导出的端点切线。
#[derive(Debug, Clone, PartialEq)]
pub struct CatmullRomSpline {
    /// 时间值。
    pub times: Vec<f64>,
    /// 控制点。
    pub points: Vec<DVec3>,
    /// 首点处的切线。
    pub first_tangent: DVec3,
    /// 末点处的切线。
    pub last_tangent: DVec3,
}

impl CatmullRomSpline {
    /// 创建一个带有自动计算切线的新 Catmull-Rom 样条。
    ///
    /// 端点切线由相邻控制点差分自动估计，无需调用方显式给入。
    pub fn new(times: Vec<f64>, points: Vec<DVec3>) -> Self {
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");

        let n = points.len();
        let (first_tangent, last_tangent) = if n > 2 {
            // 首点切线 = (2*p1 - p2 - p0) * 0.5（由相邻三点差分）
            let ft = (points[1] * 2.0 - points[2] - points[0]) * 0.5;
            // 末点切线 = (p_{n-1} - 2*p_{n-2} + p_{n-3}) * 0.5
            let lt = (points[n - 1] - points[n - 2] * 2.0 + points[n - 3]) * 0.5;
            (ft, lt)
        } else {
            // 仅两个点时退化为端点连线方向
            (points[1] - points[0], points[1] - points[0])
        };

        Self {
            times,
            points,
            first_tangent,
            last_tangent,
        }
    }

    /// 使用显式切线创建。
    ///
    /// 允许调用方指定首/末点的出入切线，以精确控制端点走向。
    pub fn with_tangents(
        times: Vec<f64>,
        points: Vec<DVec3>,
        first_tangent: DVec3,
        last_tangent: DVec3,
    ) -> Self {
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");
        Self {
            times,
            points,
            first_tangent,
            last_tangent,
        }
    }

    /// 在给定时间处求样条的值。
    ///
    /// 首/末段使用 Hermite 基，内部段使用 Catmull-Rom 矩阵。
    pub fn evaluate(&self, time: f64) -> DVec3 {
        let n = self.points.len();
        if n < 3 {
            // 控制点少于 3：无法构造 Catmull-Rom，直接线性回退
            let t0 = self.times[0];
            let inv_span = 1.0 / (self.times[1] - t0);
            let u = (time - t0) * inv_span;
            return self.points[0].lerp(self.points[1], u);
        }

        let i = self.find_time_interval(time);
        let t0 = self.times[i];
        let t1 = self.times[i + 1];
        let u = if (t1 - t0).abs() > 1e-15 {
            (time - t0) / (t1 - t0)
        } else {
            0.0
        };

        let u2 = u * u;
        let u3 = u2 * u;

        if i == 0 {
            // 首段：带 firstTangent 的 Hermite，末点切线由中心差分估计
            let p0 = self.points[0];
            let p1 = self.points[1];
            let m0 = self.first_tangent;
            let m1 = (self.points[2] - self.points[0]) * 0.5;

            // Hermite 四次基函数 h00/h10/h01/h11（以 u 为参）
            let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
            let h10 = u3 - 2.0 * u2 + u;
            let h01 = -2.0 * u3 + 3.0 * u2;
            let h11 = u3 - u2;

            p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11
        } else if i == n - 2 {
            // 末段：带 lastTangent 的 Hermite，首点切线由中心差分估计
            let p0 = self.points[i];
            let p1 = self.points[i + 1];
            let m0 = (self.points[i + 1] - self.points[i - 1]) * 0.5;
            let m1 = self.last_tangent;

            let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
            let h10 = u3 - 2.0 * u2 + u;
            let h01 = -2.0 * u3 + 3.0 * u2;
            let h11 = u3 - u2;

            p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11
        } else {
            // 内部段：Catmull-Rom 系数矩阵，取四个连续点 p_{i-1}..p_{i+2}
            // 多项式展开系数（0.5 因子已并入 c0..c3）
            let p0 = self.points[i - 1];
            let p1 = self.points[i];
            let p2 = self.points[i + 1];
            let p3 = self.points[i + 2];

            // 四个基多项式系数 c0..c3（关于 u 的三次式）
            let c0 = -0.5 * u3 + u2 - 0.5 * u;
            let c1 = 1.5 * u3 - 2.5 * u2 + 1.0;
            let c2 = -1.5 * u3 + 2.0 * u2 + 0.5 * u;
            let c3 = 0.5 * u3 - 0.5 * u2;

            p0 * c0 + p1 * c1 + p2 * c2 + p3 * c3
        }
    }
}

impl Spline for CatmullRomSpline {
    /// 返回时间节点序列的切片视图。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

// ============================================================================
// HermiteSpline
// ============================================================================

/// 带显式切线的 Hermite 样条。
///
/// 每段由两端点位置与出/入切线唯一确定，支持 C1 与自然/夹取 C2 构造。
/// inTangents 与 outTangents 的长度为 `points.len() - 1`。
/// 对于段 [i, i+1]：out_tangents[i] 是 points[i] 处的出向切线，
/// in_tangents[i] 是 points[i+1] 处的入向切线。
#[derive(Debug, Clone, PartialEq)]
pub struct HermiteSpline {
    /// 时间值。
    pub times: Vec<f64>,
    /// 控制点。
    pub points: Vec<DVec3>,
    /// 入向切线（长度 = points.len() - 1）。
    pub in_tangents: Vec<DVec3>,
    /// 出向切线（长度 = points.len() - 1）。
    pub out_tangents: Vec<DVec3>,
}

impl HermiteSpline {
    /// 创建一个新的 Hermite 样条。
    /// in_tangents 与 out_tangents 的长度必须等于 points.len() - 1。
    pub fn new(
        times: Vec<f64>,
        points: Vec<DVec3>,
        in_tangents: Vec<DVec3>,
        out_tangents: Vec<DVec3>,
    ) -> Self {
        // 校验：入/出切线数组长度必须等于点数减一
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");
        assert_eq!(
            in_tangents.len(),
            points.len() - 1,
            "inTangents.length must be points.length - 1"
        );
        assert_eq!(
            out_tangents.len(),
            points.len() - 1,
            "outTangents.length must be points.length - 1"
        );
        Self {
            times,
            points,
            in_tangents,
            out_tangents,
        }
    }

    /// 由每个点处共享的切线创建 C1 连续样条。
    ///
    /// 共享切线：out_tangents 取前 n-1 个、in_tangents 取后 n-1 个，保证段间 C1 连续。
    pub fn create_c1(times: Vec<f64>, points: Vec<DVec3>, tangents: Vec<DVec3>) -> Self {
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");
        assert_eq!(tangents.len(), points.len(), "tangents and points must match");
        // 共享切线：out 取前 n-1 个、in 取后 n-1 个，同一切线在相邻段两侧重用
        let out_tangents = tangents[..tangents.len() - 1].to_vec();
        let in_tangents = tangents[1..].to_vec();
        Self { times, points, in_tangents, out_tangents }
    }

    /// 创建自然三次样条（C2 连续）。
    ///
    /// 以“端点二阶导为零”的自然边界条件解出各点共享切线。
    pub fn create_natural_cubic(times: Vec<f64>, points: Vec<DVec3>) -> Self {
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");

        // 点数 < 3 时退化为单段直线，两端切线取点差
        if points.len() < 3 {
            let tangent = points[1] - points[0];
            return Self {
                times,
                points,
                in_tangents: vec![tangent],
                out_tangents: vec![tangent],
            };
        }

        // 由自然三次方程组解出各点切线，再拆为出/入切线数组
        let tangents = generate_natural(&points);
        let out_tangents = tangents[..tangents.len() - 1].to_vec();
        let in_tangents = tangents[1..].to_vec();
        Self { times, points, in_tangents, out_tangents }
    }

    /// 创建夹取三次样条（C2，带指定的端点切线）。
    ///
    /// 与三次自然样条同，但首/末切线由调用方显式固定。
    pub fn create_clamped_cubic(
        times: Vec<f64>,
        points: Vec<DVec3>,
        first_tangent: DVec3,
        last_tangent: DVec3,
    ) -> Self {
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");

        // 点数 < 3 时直接采用传入的端点切线
        if points.len() < 3 {
            let tangent = points[1] - points[0];
            return Self {
                times,
                points,
                in_tangents: vec![tangent],
                out_tangents: vec![tangent],
            };
        }

        // 由夹取三次方程组解出各点切线
        let tangents = generate_clamped(&points, first_tangent, last_tangent);
        let out_tangents = tangents[..tangents.len() - 1].to_vec();
        let in_tangents = tangents[1..].to_vec();
        Self { times, points, in_tangents, out_tangents }
    }

    /// 在给定时间处求样条的值。
    /// 使用带 timesDelta 缩放的 Hermite 基系数矩阵。
    pub fn evaluate(&self, time: f64) -> DVec3 {
        let i = self.find_time_interval(time);
        let t0 = self.times[i];
        let t1 = self.times[i + 1];
        // timesDelta = 实际时间间隔，用于把切线系数缩放到真实时长
        let times_delta = t1 - t0;
        let u = if times_delta.abs() > 1e-15 {
            (time - t0) / times_delta
        } else {
            0.0
        };

        let u2 = u * u;
        let u3 = u2 * u;
        // Hermite 基系数，其中切线项乘以 timesDelta 以匹配实际时间间隔
        let coef_start = 2.0 * u3 - 3.0 * u2 + 1.0;
        let coef_end = -2.0 * u3 + 3.0 * u2;
        let coef_out = (u3 - 2.0 * u2 + u) * times_delta;
        let coef_in = (u3 - u2) * times_delta;

        self.points[i] * coef_start
            + self.points[i + 1] * coef_end
            + self.out_tangents[i] * coef_out
            + self.in_tangents[i] * coef_in
    }
}

impl Spline for HermiteSpline {
    /// 返回时间节点序列的切片视图。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

/// 使用 Thomas 算法求解三对角方程组。
///
/// 对给定下/主/上对角与右端向量，以 O(n) 前向消元加回代求解。
pub fn tridiagonal_solve(
    lower: &[f64],
    diagonal: &[f64],
    upper: &[f64],
    right: &[DVec3],
) -> Vec<DVec3> {
    let n = right.len();
    let mut c = vec![0.0f64; upper.len()];
    let mut d = vec![DVec3::ZERO; n];
    let mut x = vec![DVec3::ZERO; n];

    // 前向消元：归一化首行的上对角与右端项
    c[0] = upper[0] / diagonal[0];
    d[0] = right[0] * (1.0 / diagonal[0]);

    // 逐行消去下对角，得到约化的 c、d
    for i in 1..c.len() {
        let scalar = 1.0 / (diagonal[i] - c[i - 1] * lower[i - 1]);
        c[i] = upper[i] * scalar;
        d[i] = (right[i] - d[i - 1] * lower[i - 1]) * scalar;
    }

    // 最后一行单独消元（不参与上面的循环，因其无上对角 c[n]）
    let i = c.len();
    let scalar = 1.0 / (diagonal[i] - c[i - 1] * lower[i - 1]);
    d[i] = (right[i] - d[i - 1] * lower[i - 1]) * scalar;

    // 回代：自末行向上依次解出各未知量 x[i] = d[i] - x[i+1]*c[i]
    x[n - 1] = d[n - 1];
    for i in (0..n - 1).rev() {
        x[i] = d[i] - x[i + 1] * c[i];
    }

    x
}

/// 为自然三次样条生成切线。
///
/// 组装自然边界（端点二阶导为零）的三对角方程组后交由 [`tridiagonal_solve`] 求解。
fn generate_natural(points: &[DVec3]) -> Vec<DVec3> {
    let n = points.len();
    let mut l = vec![0.0f64; n - 1];
    let mut u = vec![0.0f64; n - 1];
    let mut d = vec![0.0f64; n];
    let mut r = vec![DVec3::ZERO; n];

    // 首行：自然边界 d[0]=2, r[0]=3*(p1-p0)
    l[0] = 1.0;
    u[0] = 1.0;
    d[0] = 2.0;
    r[0] = (points[1] - points[0]) * 3.0;

    // 内部行：d[i]=4, r[i]=3*(p_{i+1}-p_{i-1})
    for i in 1..n - 1 {
        l[i] = 1.0;
        u[i] = 1.0;
        d[i] = 4.0;
        r[i] = (points[i + 1] - points[i - 1]) * 3.0;
    }

    // 末行：自然边界 d[n-1]=2, r[n-1]=3*(p_{n-1}-p_{n-2})
    d[n - 1] = 2.0;
    r[n - 1] = (points[n - 1] - points[n - 2]) * 3.0;

    // 组装完毕，交由三对角求解器返回各点切线
    tridiagonal_solve(&l, &d, &u, &r)
}

/// 为夹取三次样条生成切线。
///
/// 端点行强制一阶导等于给定切线，内部行与自然三次同。
fn generate_clamped(points: &[DVec3], first_tangent: DVec3, last_tangent: DVec3) -> Vec<DVec3> {
    let n = points.len();
    let mut l = vec![0.0f64; n - 1];
    let mut u = vec![0.0f64; n - 1];
    let mut d = vec![0.0f64; n];
    let mut r = vec![DVec3::ZERO; n];

    // 首行：强制一阶导 = first_tangent
    l[0] = 1.0;
    d[0] = 1.0;
    u[0] = 0.0;
    r[0] = first_tangent;

    // 内部行：d[i]=4, r[i]=3*(p_{i+1}-p_{i-1})
    for i in 1..n - 2 {
        l[i] = 1.0;
        u[i] = 1.0;
        d[i] = 4.0;
        r[i] = (points[i + 1] - points[i - 1]) * 3.0;
    }

    let i = n - 2;
    l[i] = 0.0;
    u[i] = 1.0;
    d[i] = 4.0;
    r[i] = (points[i + 1] - points[i - 1]) * 3.0;

    // 末行：强制一阶导 = last_tangent
    d[n - 1] = 1.0;
    r[n - 1] = last_tangent;

    tridiagonal_solve(&l, &d, &u, &r)
}

// ============================================================================
// QuaternionSpline
// ============================================================================

/// 使用 SLERP 插值的四元数样条。
///
/// 逐段对相邻旋转做球面线性插值，适用于姿态/朝向的时间插值。
#[derive(Debug, Clone, PartialEq)]
pub struct QuaternionSpline {
    /// 时间值。
    pub times: Vec<f64>,
    /// 四元数控制点。
    pub points: Vec<DQuat>,
}

impl QuaternionSpline {
    /// 创建一个新的四元数样条。
    pub fn new(times: Vec<f64>, points: Vec<DQuat>) -> Self {
        // 控制点数至少 2，时间数与四元数点数一一对应
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");
        Self { times, points }
    }

    /// 使用 SLERP 在给定时间处求样条的值。
    pub fn evaluate(&self, time: f64) -> DQuat {
        // 定位包围段后按归一化参数对两端四元数做 SLERP
        let i = self.find_time_interval(time);
        let t0 = self.times[i];
        let t1 = self.times[i + 1];
        let u = if (t1 - t0).abs() > 1e-15 {
            (time - t0) / (t1 - t0)
        } else {
            0.0
        };
        self.points[i].slerp(self.points[i + 1], u)
    }
}

impl Spline for QuaternionSpline {
    /// 返回时间节点序列的切片视图。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

// ============================================================================
// SteppedSpline
// ============================================================================

/// 阶梯（分段常量）样条 —— 保持取值直到下一个关键帧。
///
/// 在关键帧之间不做插值，适用于开关、模式等离散取值的时间轴。
#[derive(Debug, Clone, PartialEq)]
pub struct SteppedSpline {
    /// 时间值。
    pub times: Vec<f64>,
    /// 控制点。
    pub points: Vec<DVec3>,
}

impl SteppedSpline {
    /// 创建一个新的阶梯样条。
    pub fn new(times: Vec<f64>, points: Vec<DVec3>) -> Self {
        // 控制点数至少 2，时间数与点数一一对应
        assert!(points.len() >= 2, "points.length must be >= 2");
        assert_eq!(times.len(), points.len(), "times and points must match");
        Self { times, points }
    }

    /// 在给定时间处求样条的值（返回上一个关键帧的值）。
    pub fn evaluate(&self, time: f64) -> DVec3 {
        // 阶梯样条：直接返回包围段左端点的常量值
        let i = self.find_time_interval(time);
        self.points[i]
    }
}

impl Spline for SteppedSpline {
    /// 返回时间节点序列的切片视图。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

// ============================================================================
// ConstantSpline
// ============================================================================

/// 常量样条 —— 始终返回同一个值。
///
/// 用于形变权重等需要“无时间变化”占位样条的场景。
#[derive(Debug, Clone, PartialEq)]
pub struct ConstantSpline {
    /// 常量值。
    pub value: DVec3,
    /// 时间范围（用于接口兼容）。
    pub times: Vec<f64>,
}

impl ConstantSpline {
    /// 创建一个新的常量样条。
    pub fn new(value: DVec3) -> Self {
        // 默认时间范围 [0,1] 仅为满足 Spline 接口，无实际时间意义
        Self {
            value,
            times: vec![0.0, 1.0],
        }
    }

    /// 使用特定的时间范围创建。
    pub fn with_time_range(value: DVec3, start: f64, end: f64) -> Self {
        // 允许自定义起止时间（仍不影响求值结果）
        Self {
            value,
            times: vec![start, end],
        }
    }

    /// 求值（始终返回常量值）。
    pub fn evaluate(&self, _time: f64) -> DVec3 {
        // 忽略时间入参，恒返回常量值
        self.value
    }

    /// 对于常量样条，wrapTime 始终返回 0.0。
    pub fn wrap_time(&self, _time: f64) -> f64 {
        // 常量样条无周期概念，恒返回 0
        0.0
    }

    /// 对于常量样条，clampTime 始终返回 0.0。
    pub fn clamp_time(&self, _time: f64) -> f64 {
        // 常量样条无时间轴概念，恒返回 0
        0.0
    }
}

impl Spline for ConstantSpline {
    /// 返回时间范围切片（仅为接口兼容）。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

// ============================================================================
// MorphWeightSpline
// ============================================================================

/// 用于形变目标权重（标量值）的样条。
///
/// 对每个形变目标的权重随时间做线性插值，权重通常落在 0.0~1.0。
#[derive(Debug, Clone, PartialEq)]
pub struct MorphWeightSpline {
    /// 时间值。
    pub times: Vec<f64>,
    /// 权重值（通常在 0.0 到 1.0 之间）。
    pub weights: Vec<f64>,
}

impl MorphWeightSpline {
    /// 创建一个新的形变权重样条。
    pub fn new(times: Vec<f64>, weights: Vec<f64>) -> Self {
        // 权重个数至少 2，时间数与权重数一一对应
        assert!(weights.len() >= 2, "weights.length must be >= 2");
        assert_eq!(times.len(), weights.len(), "times and weights must match");
        Self { times, weights }
    }

    /// 在给定时间处求权重（线性插值）。
    pub fn evaluate(&self, time: f64) -> f64 {
        // 权重按包围段两端线性插值
        let i = self.find_time_interval(time);
        let t0 = self.times[i];
        let t1 = self.times[i + 1];
        let u = if (t1 - t0).abs() > 1e-15 {
            (time - t0) / (t1 - t0)
        } else {
            0.0
        };
        self.weights[i] + (self.weights[i + 1] - self.weights[i]) * u
    }
}

impl Spline for MorphWeightSpline {
    /// 返回时间节点序列的切片视图。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

// ============================================================================
// ScalarSpline
// ============================================================================

/// 用于标量值的样条（线性插值）。
///
/// 与 [`MorphWeightSpline`] 结构相同，专用于任意标量轨道的逐段线性求值。
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarSpline {
    /// 时间值。
    pub times: Vec<f64>,
    /// 标量值。
    pub values: Vec<f64>,
}

impl ScalarSpline {
    /// 创建一个新的标量样条。
    pub fn new(times: Vec<f64>, values: Vec<f64>) -> Self {
        // 标量个数至少 2，时间数与标量数一一对应
        assert!(values.len() >= 2, "values.length must be >= 2");
        assert_eq!(times.len(), values.len(), "times and values must match");
        Self { times, values }
    }

    /// 在给定时间处求标量值。
    pub fn evaluate(&self, time: f64) -> f64 {
        // 标量按包围段两端线性插值
        let i = self.find_time_interval(time);
        let t0 = self.times[i];
        let t1 = self.times[i + 1];
        let u = if (t1 - t0).abs() > 1e-15 {
            (time - t0) / (t1 - t0)
        } else {
            0.0
        };
        self.values[i] + (self.values[i + 1] - self.values[i]) * u
    }
}

impl Spline for ScalarSpline {
    /// 返回时间节点序列的切片视图。
    fn times(&self) -> &[f64] {
        &self.times
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn test_linear_spline_endpoints() {
        let spline = LinearSpline::new(
            vec![0.0, 1.0, 2.0],
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 1.0),
                DVec3::new(2.0, 0.0, 0.0),
            ],
        );
        let p0 = spline.evaluate(0.0);
        assert!((p0 - DVec3::new(0.0, 0.0, 0.0)).length() < 1e-10);
        let p1 = spline.evaluate(1.0);
        assert!((p1 - DVec3::new(1.0, 1.0, 1.0)).length() < 1e-10);
        let p2 = spline.evaluate(2.0);
        assert!((p2 - DVec3::new(2.0, 0.0, 0.0)).length() < 1e-10);
    }

    #[test]
    fn test_linear_spline_midpoint() {
        let spline = LinearSpline::new(
            vec![0.0, 2.0],
            vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(4.0, 2.0, 0.0)],
        );
        let mid = spline.evaluate(1.0);
        assert!((mid - DVec3::new(2.0, 1.0, 0.0)).length() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_spline_endpoints() {
        let spline = CatmullRomSpline::new(
            vec![0.0, 1.0, 2.0, 3.0],
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(3.0, 1.0, 0.0),
            ],
        );
        let p0 = spline.evaluate(0.0);
        assert!((p0 - DVec3::new(0.0, 0.0, 0.0)).length() < 1e-10);
        let p3 = spline.evaluate(3.0);
        assert!((p3 - DVec3::new(3.0, 1.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn test_hermite_spline() {
        let spline = HermiteSpline::new(
            vec![0.0, 1.0],
            vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 1.0, 0.0)],
            vec![DVec3::new(1.0, 0.0, 0.0)],
            vec![DVec3::new(1.0, 0.0, 0.0)],
        );
        let p0 = spline.evaluate(0.0);
        assert!((p0 - DVec3::new(0.0, 0.0, 0.0)).length() < 1e-10);
        let p1 = spline.evaluate(1.0);
        assert!((p1 - DVec3::new(1.0, 1.0, 0.0)).length() < 1e-10);
    }

    #[test]
    fn test_quaternion_spline() {
        let q0 = DQuat::IDENTITY;
        let q1 = DQuat::from_rotation_z(FRAC_PI_2);
        let spline = QuaternionSpline::new(vec![0.0, 1.0], vec![q0, q1]);
        let r0 = spline.evaluate(0.0);
        assert!((r0.x - q0.x).abs() < 1e-10);
        assert!((r0.w - q0.w).abs() < 1e-10);
        let r1 = spline.evaluate(1.0);
        assert!((r1.z - q1.z).abs() < 1e-6);
    }

    #[test]
    fn test_stepped_spline() {
        let spline = SteppedSpline::new(
            vec![0.0, 1.0, 2.0],
            vec![
                DVec3::new(0.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 1.0),
                DVec3::new(2.0, 2.0, 2.0),
            ],
        );
        let p = spline.evaluate(0.5);
        assert!((p - DVec3::new(0.0, 0.0, 0.0)).length() < 1e-10);
        let p = spline.evaluate(1.5);
        assert!((p - DVec3::new(1.0, 1.0, 1.0)).length() < 1e-10);
    }

    #[test]
    fn test_natural_cubic() {
        let spline = HermiteSpline::create_natural_cubic(
            vec![0.0, 1.0, 2.0, 3.0],
            vec![
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(0.0, 1.0, FRAC_PI_2),
                DVec3::new(-1.0, 0.0, std::f64::consts::PI),
                DVec3::new(0.0, -1.0, 3.0 * FRAC_PI_2),
            ],
        );
        let p0 = spline.evaluate(0.0);
        assert!((p0 - DVec3::new(1.0, 0.0, 0.0)).length() < 1e-10);
    }

    #[test]
    fn test_wrap_time() {
        let spline = LinearSpline::new(
            vec![0.0, 1.0, 2.0],
            vec![DVec3::ZERO, DVec3::ONE, DVec3::ZERO],
        );
        assert!((spline.wrap_time(2.5) - 0.5).abs() < 1e-10);
        assert!((spline.wrap_time(-0.5) - 1.5).abs() < 1e-10);
    }

    #[test]
    fn test_clamp_time() {
        let spline = LinearSpline::new(
            vec![0.0, 1.0, 2.0],
            vec![DVec3::ZERO, DVec3::ONE, DVec3::ZERO],
        );
        assert!((spline.clamp_time(-1.0) - 0.0).abs() < 1e-10);
        assert!((spline.clamp_time(5.0) - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_find_time_interval() {
        let spline = LinearSpline::new(
            vec![0.0, 1.0, 2.0, 3.0],
            vec![DVec3::ZERO, DVec3::ONE, DVec3::ZERO, DVec3::ONE],
        );
        assert_eq!(spline.find_time_interval(0.5), 0);
        assert_eq!(spline.find_time_interval(1.5), 1);
        assert_eq!(spline.find_time_interval(2.5), 2);
        assert_eq!(spline.find_time_interval(0.0), 0);
        assert_eq!(spline.find_time_interval(3.0), 2);
    }
}
