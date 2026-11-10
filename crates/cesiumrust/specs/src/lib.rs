//! cesium-specs：从 CesiumJS Specs 移植的集成测试套件。
//!
//! 本 crate 为 `tests/` 下的集成测试文件提供测试辅助工具
//! 与重导出。

/// 断言两个 f64 值在 epsilon 容差内近似相等。
///
/// 参考 CesiumJS 的 `toEqualEpsilon(expected, epsilon)`。
#[macro_export]
macro_rules! assert_approx {
    ($a:expr, $b:expr, $eps:expr) => {
        let (a_val, b_val, eps_val) = ($a as f64, $b as f64, $eps as f64);
        assert!(
            (a_val - b_val).abs() <= eps_val,
            "assertion failed: |{} - {}| = {} > epsilon {}",
            a_val,
            b_val,
            (a_val - b_val).abs(),
            eps_val
        );
    };
}

/// 断言两个 DVec3 值在 epsilon 容差内近似相等。
///
/// 参考 CesiumJS 中 Cartesian3 的 `toEqualEpsilon`。
#[macro_export]
macro_rules! assert_vec3_epsilon {
    ($a:expr, $b:expr, $eps:expr) => {
        let (a_vec, b_vec, eps_val) = ($a, $b, $eps as f64);
        let diff = a_vec - b_vec;
        assert!(
            diff.x.abs() <= eps_val && diff.y.abs() <= eps_val && diff.z.abs() <= eps_val,
            "assertion failed: DVec3 {:?} != {:?} within epsilon {}",
            a_vec,
            b_vec,
            eps_val
        );
    };
}

/// 断言两个 DVec2 值在 epsilon 容差内近似相等。
#[macro_export]
macro_rules! assert_vec2_epsilon {
    ($a:expr, $b:expr, $eps:expr) => {
        let (a_vec, b_vec, eps_val) = ($a, $b, $eps as f64);
        let diff = a_vec - b_vec;
        assert!(
            diff.x.abs() <= eps_val && diff.y.abs() <= eps_val,
            "assertion failed: DVec2 {:?} != {:?} within epsilon {}",
            a_vec,
            b_vec,
            eps_val
        );
    };
}

/// 断言两个 DVec4 值在 epsilon 容差内近似相等。
#[macro_export]
macro_rules! assert_vec4_epsilon {
    ($a:expr, $b:expr, $eps:expr) => {
        let (a_vec, b_vec, eps_val) = ($a, $b, $eps as f64);
        let diff = a_vec - b_vec;
        assert!(
            diff.x.abs() <= eps_val
                && diff.y.abs() <= eps_val
                && diff.z.abs() <= eps_val
                && diff.w.abs() <= eps_val,
            "assertion failed: DVec4 {:?} != {:?} within epsilon {}",
            a_vec,
            b_vec,
            eps_val
        );
    };
}

/// 断言两个 DQuat 值在 epsilon 容差内近似相等。
#[macro_export]
macro_rules! assert_quat_epsilon {
    ($a:expr, $b:expr, $eps:expr) => {
        let (a_q, b_q, eps_val) = ($a, $b, $eps as f64);
        let dot = a_q.x * b_q.x + a_q.y * b_q.y + a_q.z * b_q.z + a_q.w * b_q.w;
        assert!(
            (dot.abs() - 1.0).abs() <= eps_val,
            "assertion failed: DQuat {:?} != {:?} within epsilon {}",
            a_q,
            b_q,
            eps_val
        );
    };
}

/// 断言两个 DMat3 值在 epsilon 容差内近似相等。
#[macro_export]
macro_rules! assert_mat3_epsilon {
    ($a:expr, $b:expr, $eps:expr) => {
        let (a_m, b_m, eps_val) = ($a, $b, $eps as f64);
        let diff = a_m - b_m;
        let max_diff = diff
            .to_cols_array()
            .iter()
            .fold(0.0f64, |mx, &v| mx.max(v.abs()));
        assert!(
            max_diff <= eps_val,
            "assertion failed: DMat3 max diff {} > epsilon {}",
            max_diff,
            eps_val
        );
    };
}

/// 断言两个 DMat4 值在 epsilon 容差内近似相等。
#[macro_export]
macro_rules! assert_mat4_epsilon {
    ($a:expr, $b:expr, $eps:expr) => {
        let (a_m, b_m, eps_val) = ($a, $b, $eps as f64);
        let diff = a_m - b_m;
        let max_diff = diff
            .to_cols_array()
            .iter()
            .fold(0.0f64, |mx, &v| mx.max(v.abs()));
        assert!(
            max_diff <= eps_val,
            "assertion failed: DMat4 max diff {} > epsilon {}",
            max_diff,
            eps_val
        );
    };
}

/// 与 CesiumJS Math 常量对应的常用 epsilon 常量。
pub mod epsilon {
    pub const EPSILON1: f64 = 1e-1;
    pub const EPSILON2: f64 = 1e-2;
    pub const EPSILON3: f64 = 1e-3;
    pub const EPSILON4: f64 = 1e-4;
    pub const EPSILON5: f64 = 1e-5;
    pub const EPSILON6: f64 = 1e-6;
    pub const EPSILON7: f64 = 1e-7;
    pub const EPSILON8: f64 = 1e-8;
    pub const EPSILON9: f64 = 1e-9;
    pub const EPSILON10: f64 = 1e-10;
    pub const EPSILON11: f64 = 1e-11;
    pub const EPSILON12: f64 = 1e-12;
    pub const EPSILON13: f64 = 1e-13;
    pub const EPSILON14: f64 = 1e-14;
    pub const EPSILON15: f64 = 1e-15;
    pub const EPSILON16: f64 = 1e-16;
    pub const EPSILON17: f64 = 1e-17;
    pub const EPSILON18: f64 = 1e-18;
    pub const EPSILON19: f64 = 1e-19;
    pub const EPSILON20: f64 = 1e-20;
}

/// 与 CesiumJS Math 对应的常用数学常量。
pub mod math_consts {
    pub const PI: f64 = std::f64::consts::PI;
    pub const TWO_PI: f64 = std::f64::consts::TAU;
    pub const PI_OVER_TWO: f64 = std::f64::consts::FRAC_PI_2;
    pub const PI_OVER_THREE: f64 = std::f64::consts::PI / 3.0;
    pub const PI_OVER_FOUR: f64 = std::f64::consts::FRAC_PI_4;
    pub const PI_OVER_SIX: f64 = std::f64::consts::PI / 6.0;
    pub const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;
    pub const DEGREES_PER_RADIAN: f64 = 180.0 / std::f64::consts::PI;
}

/// 将角度转换为弧度的辅助函数。
pub fn to_radians(degrees: f64) -> f64 {
    degrees * math_consts::RADIANS_PER_DEGREE
}

/// 将弧度转换为角度的辅助函数。
pub fn to_degrees(radians: f64) -> f64 {
    radians * math_consts::DEGREES_PER_RADIAN
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn test_assert_approx_macro() {
        assert_approx!(1.0, 1.0 + 1e-16, epsilon::EPSILON15);
    }

    #[test]
    fn test_assert_vec3_epsilon_macro() {
        let a = DVec3::new(1.0, 2.0, 3.0);
        let b = DVec3::new(1.0 + 1e-16, 2.0 - 1e-16, 3.0);
        assert_vec3_epsilon!(a, b, epsilon::EPSILON15);
    }

    #[test]
    fn test_to_radians() {
        assert_approx!(to_radians(180.0), math_consts::PI, epsilon::EPSILON15);
        assert_approx!(to_radians(90.0), math_consts::PI_OVER_TWO, epsilon::EPSILON15);
    }

    #[test]
    fn test_to_degrees() {
        assert_approx!(to_degrees(math_consts::PI), 180.0, epsilon::EPSILON13);
    }
}
