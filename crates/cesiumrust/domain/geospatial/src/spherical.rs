//! 球坐标。

use glam::DVec3;

/// 一组曲线 3D 坐标：clock、cone 和 magnitude。
/// 映射到 CesiumJS `Spherical`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spherical {
    /// 位于赤道平面内、从 x 轴起量的角度坐标。
    pub clock: f64,
    /// 从 z 轴起量的角度坐标（极角 / 锥角）。
    pub cone: f64,
    /// 从原点起量的线性坐标。
    pub magnitude: f64,
}

impl Default for Spherical {
    /// 默认值：clock=0、cone=0、magnitude=1。
    fn default() -> Self {
        Self { clock: 0.0, cone: 0.0, magnitude: 1.0 }
    }
}

impl Spherical {
    /// 由 clock、cone、magnitude 三个分量直接构造球坐标。
    pub fn new(clock: f64, cone: f64, magnitude: f64) -> Self {
        Self { clock, cone, magnitude }
    }

    /// 将 Cartesian3 转换为球坐标。
    /// 映射到 `Spherical.fromCartesian3`
    pub fn from_cartesian3(cartesian: DVec3) -> Self {
        let magnitude = cartesian.length();
        let mut cone = 0.0;
        let mut clock = 0.0;

        if magnitude > 0.0 {
            let rad = cartesian.z / magnitude;
            // 为 acos 安全地限制到 [-1, 1]
            cone = rad.clamp(-1.0, 1.0).acos();
            clock = cartesian.y.atan2(cartesian.x);
            if clock < 0.0 {
                clock += std::f64::consts::TAU;
            }
        }

        Self { clock, cone, magnitude }
    }

    /// 返回一个归一化的副本（magnitude = 1.0）。
    /// 映射到 `Spherical.normalize`
    pub fn normalize(&self) -> Self {
        Self {
            clock: self.clock,
            cone: self.cone,
            magnitude: 1.0,
        }
    }

    /// 若在 epsilon 范围内此球坐标等于 other 则返回 true。
    /// 映射到 `Spherical.equalsEpsilon`
    pub fn equals_epsilon(&self, other: &Self, epsilon: f64) -> bool {
        (self.clock - other.clock).abs() <= epsilon
            && (self.cone - other.cone).abs() <= epsilon
            && (self.magnitude - other.magnitude).abs() <= epsilon
    }
}

impl std::fmt::Display for Spherical {
    /// 以 `(clock, cone, magnitude)` 形式格式化输出。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.clock, self.cone, self.magnitude)
    }
}
