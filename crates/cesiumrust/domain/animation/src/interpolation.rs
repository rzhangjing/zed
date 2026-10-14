//! 用于动画的时间插值算法。
//!
//! 提供线性、Hermite 三次、Lagrange 多项式与 Catmull-Rom 等
//! 常用插值内核，均含标量与 DVec3 两个版本，供采样轨迹求值。

use glam::DVec3;

/// 插值算法类型。
///
/// 选择对采样点求值时采用的多项式阶数与是否使用导数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpolationType {
    /// 线性插值（1 阶）。
    #[default]
    Linear,
    /// Hermite 多项式插值（使用导数）。
    Hermite,
    /// Lagrange 多项式插值。
    Lagrange,
}

/// 一个时间-数值采样点。
///
/// 记录某时刻的自变量时间与函数值，并可选携带导数供 Hermite 使用。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplePoint {
    /// 自纪元起的时间（秒）。
    pub time: f64,
    /// 此时刻的值。
    pub value: f64,
    /// 可选导数（用于 Hermite）。
    pub derivative: Option<f64>,
}

impl SamplePoint {
    /// 创建一个新的采样点。
    pub fn new(time: f64, value: f64) -> Self {
        // 不带导数的默认构造：derivative 置为 None
        Self {
            time,
            value,
            derivative: None,
        }
    }

    /// 创建一个带导数的采样点。
    pub fn with_derivative(time: f64, value: f64, derivative: f64) -> Self {
        // 供 Hermite 插值使用：额外携带该时刻的导数值
        Self {
            time,
            value,
            derivative: Some(derivative),
        }
    }
}

/// 两个值之间的线性插值。
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    // 标准线性插值：从 a 向 b 按参数 t 推进
    a + (b - a) * t
}

/// DVec3 的线性插值。
pub fn lerp_vec3(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    // 向量的线性插值等价于逐分量套用 lerp
    a + (b - a) * t
}

/// 两个带切线的点之间的 Hermite（三次）插值。
///
/// # 参数
/// * `p0` - 起始值
/// * `m0` - 起始切线
/// * `p1` - 结束值
/// * `m1` - 结束切线
/// * `t` - 参数 [0, 1]
pub fn hermite(p0: f64, m0: f64, p1: f64, m1: f64, t: f64) -> f64 {
    // 预计算 t 的二次与三次幂，供基多项式复用
    let t2 = t * t;
    let t3 = t2 * t;

    // 四次 Hermite 基函数 H00/H10/H01/H11（关于 t 的多项式）
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;

    // 结果为位置与切线的加权和：两端函数值乘 H00/H01，两端导数乘 H10/H11
    h00 * p0 + h10 * m0 + h01 * p1 + h11 * m1
}

/// DVec3 的 Hermite 插值。
pub fn hermite_vec3(p0: DVec3, m0: DVec3, p1: DVec3, m1: DVec3, t: f64) -> DVec3 {
    // 对 x/y/z 三个分量分别执行标量 Hermite 插值
    DVec3::new(
        hermite(p0.x, m0.x, p1.x, m1.x, t),
        hermite(p0.y, m0.y, p1.y, m1.y, t),
        hermite(p0.z, m0.z, p1.z, m1.z, t),
    )
}

/// Lagrange 多项式插值。
///
/// 以全部采样点构造一个至多 n-1 次多项式，再在 t 处求值。
///
/// # 参数
/// * `points` - 采样点（时间、数值）
/// * `t` - 要插值的时间
pub fn lagrange_interpolate(points: &[SamplePoint], t: f64) -> f64 {
    let n = points.len();
    // 空集返回 0；单点退化为常值
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return points[0].value;
    }

    // 累加每个基多项式 L_i(t) * y_i
    let mut result = 0.0;
    for i in 0..n {
        let mut basis = points[i].value;
        for j in 0..n {
            if i != j {
                // 基函数：对所有 j≠i 连乘 (t - x_j) / (x_i - x_j)
                let denom = points[i].time - points[j].time;
                if denom.abs() > 1e-15 {
                    basis *= (t - points[j].time) / denom;
                }
            }
        }
        result += basis;
    }
    result
}

/// DVec3 的 Lagrange 插值。
pub fn lagrange_interpolate_vec3(
    times: &[f64],
    values: &[DVec3],
    t: f64,
) -> DVec3 {
    // 取 times 与 values 的公共长度作为有效采样数
    let n = times.len().min(values.len());
    if n == 0 {
        return DVec3::ZERO;
    }

    // 拆出 x/y/z 三条标量采样序列，分别复用标量 Lagrange 插值
    let points_x: Vec<SamplePoint> = (0..n)
        .map(|i| SamplePoint::new(times[i], values[i].x))
        .collect();
    let points_y: Vec<SamplePoint> = (0..n)
        .map(|i| SamplePoint::new(times[i], values[i].y))
        .collect();
    let points_z: Vec<SamplePoint> = (0..n)
        .map(|i| SamplePoint::new(times[i], values[i].z))
        .collect();

    DVec3::new(
        lagrange_interpolate(&points_x, t),
        lagrange_interpolate(&points_y, t),
        lagrange_interpolate(&points_z, t),
    )
}

/// Catmull-Rom 样条插值（一种自动切线的 Hermite）。
///
/// # 参数
/// * `p0`, `p1`, `p2`, `p3` - 四个控制点
/// * `t` - 参数 [0, 1]（在 p1 与 p2 之间插值）
pub fn catmull_rom(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    // 预计算 t 的二次与三次幂
    let t2 = t * t;
    let t3 = t2 * t;

    // Catmull-Rom 矩阵形式展开：以 p1、p2 为段端点，p0、p3 提供切线信息
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// DVec3 的 Catmull-Rom 样条。
pub fn catmull_rom_vec3(p0: DVec3, p1: DVec3, p2: DVec3, p3: DVec3, t: f64) -> DVec3 {
    // 逐分量套用标量 Catmull-Rom 公式
    DVec3::new(
        catmull_rom(p0.x, p1.x, p2.x, p3.x, t),
        catmull_rom(p0.y, p1.y, p2.y, p3.y, t),
        catmull_rom(p0.z, p1.z, p2.z, p3.z, t),
    )
}

/// 单位向量（方向）的球面线性插值。
///
/// 沿两单位向量间的大圆弧以恒定角速度插值，保持结果为单位长。
pub fn slerp_vec3(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    // 两单位向量夹角的余弦（夹取防浮点越界）
    let dot = a.dot(b).clamp(-1.0, 1.0);

    if dot.abs() > 0.9995 {
        // 近乎平行，使用线性插值
        return lerp_vec3(a, b, t).normalize();
    }

    // 常规情形：按夹角正弦分配两端的权重，再归一化回单位球面
    let theta = dot.acos();
    let sin_theta = theta.sin();
    // 端点权重：按 (1-t) 与 t 相对夹角的正弦分配
    let wa = ((1.0 - t) * theta).sin() / sin_theta;
    let wb = (t * theta).sin() / sin_theta;

    (a * wa + b * wb).normalize()
}

/// 使用指定算法插值一个数值。
///
/// 根据 algo 分发到线性/Hermite/Lagrange 三条求值路径。
pub fn interpolate(
    algo: InterpolationType,
    points: &[SamplePoint],
    t: f64,
) -> f64 {
    // 空/单点边界：无点返回 0，单点直接取其值
    if points.is_empty() {
        return 0.0;
    }
    if points.len() == 1 {
        return points[0].value;
    }

    match algo {
        InterpolationType::Linear => {
            // 找到包围区间
            let (i0, i1) = find_bracket(points, t);
            let dt = points[i1].time - points[i0].time;
            // 归一化参数 frac = (t - x0) / (x1 - x0)，区间退化时取 0
            let frac = if dt.abs() > 1e-15 {
                (t - points[i0].time) / dt
            } else {
                0.0
            };
            lerp(points[i0].value, points[i1].value, frac)
        }
        InterpolationType::Hermite => {
            // Hermite：把端点导数乘以区间长度换算为切线向量再插值
            let (i0, i1) = find_bracket(points, t);
            let dt = points[i1].time - points[i0].time;
            let frac = if dt.abs() > 1e-15 {
                (t - points[i0].time) / dt
            } else {
                0.0
            };
            let m0 = points[i0].derivative.unwrap_or(0.0) * dt;
            let m1 = points[i1].derivative.unwrap_or(0.0) * dt;
            hermite(points[i0].value, m0, points[i1].value, m1, frac)
        }
        InterpolationType::Lagrange => lagrange_interpolate(points, t),
    }
}

/// 查找时间 t 的包围索引。
fn find_bracket(points: &[SamplePoint], t: f64) -> (usize, usize) {
    // t 不越过首点 → 取最前一对索引
    if t <= points[0].time {
        return (0, 1.min(points.len() - 1));
    }
    let last = points.len() - 1;
    // t 越过末点 → 取最后一段
    if t >= points[last].time {
        return (last.saturating_sub(1), last);
    }

    // 线性扫描首个满足 x_i <= t <= x_{i+1} 的区间
    for i in 0..last {
        if t >= points[i].time && t <= points[i + 1].time {
            return (i, i + 1);
        }
    }
    (last.saturating_sub(1), last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lerp() {
        assert!((lerp(0.0, 10.0, 0.0) - 0.0).abs() < 1e-10);
        assert!((lerp(0.0, 10.0, 0.5) - 5.0).abs() < 1e-10);
        assert!((lerp(0.0, 10.0, 1.0) - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_lerp_vec3() {
        let a = DVec3::new(0.0, 0.0, 0.0);
        let b = DVec3::new(10.0, 20.0, 30.0);
        let result = lerp_vec3(a, b, 0.5);
        assert!((result.x - 5.0).abs() < 1e-10);
        assert!((result.y - 10.0).abs() < 1e-10);
        assert!((result.z - 15.0).abs() < 1e-10);
    }

    #[test]
    fn test_hermite_endpoints() {
        // Hermite 应穿过端点
        let result_start = hermite(1.0, 0.0, 5.0, 0.0, 0.0);
        let result_end = hermite(1.0, 0.0, 5.0, 0.0, 1.0);
        assert!((result_start - 1.0).abs() < 1e-10);
        assert!((result_end - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_hermite_midpoint() {
        // 切线为零时，中点应为平均值
        let result = hermite(0.0, 0.0, 10.0, 0.0, 0.5);
        assert!((result - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_lagrange_linear() {
        // 两个点 = 线性插值
        let points = vec![SamplePoint::new(0.0, 0.0), SamplePoint::new(1.0, 10.0)];
        let result = lagrange_interpolate(&points, 0.5);
        assert!((result - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_lagrange_quadratic() {
        // y = x^2 上的三个点
        let points = vec![
            SamplePoint::new(0.0, 0.0),
            SamplePoint::new(1.0, 1.0),
            SamplePoint::new(2.0, 4.0),
        ];
        let result = lagrange_interpolate(&points, 1.5);
        assert!((result - 2.25).abs() < 1e-10);
    }

    #[test]
    fn test_catmull_rom_endpoints() {
        let result_start = catmull_rom(0.0, 1.0, 4.0, 9.0, 0.0);
        let result_end = catmull_rom(0.0, 1.0, 4.0, 9.0, 1.0);
        assert!((result_start - 1.0).abs() < 1e-10);
        assert!((result_end - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_slerp_same_direction() {
        let a = DVec3::X;
        let b = DVec3::X;
        let result = slerp_vec3(a, b, 0.5);
        assert!((result.x - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_slerp_perpendicular() {
        let a = DVec3::X;
        let b = DVec3::Y;
        let result = slerp_vec3(a, b, 0.5);
        // 应处于 45 度
        let expected = (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2);
        assert!((result.x - expected.0).abs() < 1e-10);
        assert!((result.y - expected.1).abs() < 1e-10);
    }

    #[test]
    fn test_interpolate_linear() {
        let points = vec![
            SamplePoint::new(0.0, 0.0),
            SamplePoint::new(10.0, 100.0),
        ];
        let result = interpolate(InterpolationType::Linear, &points, 5.0);
        assert!((result - 50.0).abs() < 1e-10);
    }

    #[test]
    fn test_interpolate_hermite() {
        let points = vec![
            SamplePoint::with_derivative(0.0, 0.0, 0.0),
            SamplePoint::with_derivative(1.0, 1.0, 0.0),
        ];
        let result = interpolate(InterpolationType::Hermite, &points, 0.5);
        assert!((result - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_interpolate_lagrange() {
        let points = vec![
            SamplePoint::new(0.0, 0.0),
            SamplePoint::new(1.0, 1.0),
            SamplePoint::new(2.0, 4.0),
        ];
        let result = interpolate(InterpolationType::Lagrange, &points, 0.5);
        assert!((result - 0.25).abs() < 1e-10);
    }

    #[test]
    fn test_interpolate_empty() {
        let result = interpolate(InterpolationType::Linear, &[], 0.5);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn test_interpolate_single() {
        let points = vec![SamplePoint::new(0.0, 42.0)];
        let result = interpolate(InterpolationType::Linear, &points, 0.5);
        assert_eq!(result, 42.0);
    }

    #[test]
    fn test_interpolation_type_default() {
        assert_eq!(InterpolationType::default(), InterpolationType::Linear);
    }
}
