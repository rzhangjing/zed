//! 椭球大地线（椭球上的大圆路径）。
//!
//! 使用 Vincenty 反算公式
//! 计算两个测绘坐标点之间的表面距离和方位角，并使用一个级数展开
//! 在给定表面距离处插值中间点。

use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use crate::math_utils::EPSILON12;

/// 大地线级数展开的预计算常量。
#[derive(Debug, Clone, Default)]
struct GeodesicConstants {
    /// 长半轴（最大曲率半径，赤道半径，米）。
    a: f64,
    /// 短半轴（最小曲率半径，极半径，米）。
    b: f64,
    /// 扁率 `f = (a - b) / a`。
    f: f64,
    /// 起始方位角的余弦。
    cosine_heading: f64,
    /// 起始方位角的正弦。
    sine_heading: f64,
    /// 约化纬度的余弦。
    cosine_u: f64,
    /// 约化纬度的正弦。
    sine_u: f64,
    /// 球面辅助长度 σ（起始值）。
    sigma: f64,
    /// 赤道方位角的正弦。
    sine_alpha: f64,
    /// 赤道方位角余弦的平方。
    cosine_squared_alpha: f64,
    /// 赤道方位角的余弦。
    cosine_alpha: f64,
    /// 级数系数 `u²/4`。
    u2_over4: f64,
    /// 级数系数 `u⁴/16`。
    u4_over16: f64,
    /// 级数系数 `u⁶/64`。
    u6_over64: f64,
    /// 级数系数 `u⁸/256`。
    u8_over256: f64,
    /// 归一化距离比值（用于插值反推 σ）。
    distance_ratio: f64,
}

/// 椭球上连接两个地球素点（planetodetic）的大地线。
///
/// 映射到 CesiumJS `EllipsoidGeodesic`。
#[derive(Debug, Clone)]
pub struct EllipsoidGeodesic {
    /// 起点（高度已置 0）。
    start: Cartographic,
    /// 终点（高度已置 0）。
    end: Cartographic,
    /// 起点处的方位角（弧度）。
    start_heading: f64,
    /// 终点处的方位角（弧度）。
    end_heading: f64,
    /// 两点间的表面距离（米）。
    distance: f64,
    /// 预计算的级数展开常量。
    constants: GeodesicConstants,
    /// 椭球最大半径（米）。
    maximum_radius: f64,
    /// 椭球最小半径（米）。
    minimum_radius: f64,
}

/// 计算 Vincenty 公式中的辅助量 `C`。
///
/// # 参数
/// - `f`：扁率；`cosine_squared_alpha`：赤道方位角余弦的平方。
///
/// # 返回
/// `f·cos²α·(4 + f·(4 - 3cos²α)) / 16`。
fn compute_c(f: f64, cosine_squared_alpha: f64) -> f64 {
    f * cosine_squared_alpha * (4.0 + f * (4.0 - 3.0 * cosine_squared_alpha)) / 16.0
}

#[allow(clippy::too_many_arguments)]
/// 计算经度修正量 `Δλ`（Vincenty 反算级数项）。
///
/// # 参数
/// - `f`：扁率；`sine_alpha`/`cosine_squared_alpha`：赤道方位角量。
/// - `sigma`/`sine_sigma`/`cosine_sigma`：球面辅助角及其三角值。
/// - `cosine_twice_sigma_midpoint`：中点处 `cos(2σₘ)`。
///
/// # 返回
/// `(1 - C)·f·sinα` 乘以一个含 σ 与中点余弦的级数，即 Δλ。
fn compute_delta_lambda(
    f: f64,
    sine_alpha: f64,
    cosine_squared_alpha: f64,
    sigma: f64,
    sine_sigma: f64,
    cosine_sigma: f64,
    cosine_twice_sigma_midpoint: f64,
) -> f64 {
    let c = compute_c(f, cosine_squared_alpha);
    (1.0 - c)
        * f
        * sine_alpha
        * (sigma
            + c * sine_sigma
                * (cosine_twice_sigma_midpoint
                    + c * cosine_sigma
                        * (2.0 * cosine_twice_sigma_midpoint * cosine_twice_sigma_midpoint
                            - 1.0)))
}

/// Vincenty 反算公式的结果。
struct VincentyResult {
    /// 两点间的椭球表面距离（米）。
    distance: f64,
    /// 起点方位角（弧度）。
    start_heading: f64,
    /// 终点方位角（弧度）。
    end_heading: f64,
    /// `u² = cos²α·(a² - b²)/b²`，供级数展开使用。
    u_squared: f64,
}

/// 收敛迭代所产生的中间值。
struct VincentyIteration {
    /// 收敛时的球面辅助角 σ。
    sigma: f64,
    /// σ 的正弦。
    sine_sigma: f64,
    /// σ 的余弦。
    cosine_sigma: f64,
    /// 赤道方位角余弦的平方。
    cosine_squared_alpha: f64,
    /// 中点处 `cos(2σₘ)`。
    cosine_twice_sigma_midpoint: f64,
    /// 收敛后的经度差 λ。
    lambda: f64,
}

/// 用 Vincenty 反算公式求椭球上两点间的距离与方位角。
///
/// 迭代求解经度差 λ 直到收敛，再由级数展开得到精确弧长。
///
/// # 参数
/// - `major`/`minor`：椭球长/短半轴（米）。
/// - `first_longitude`/`first_latitude`：起点经纬度（弧度）。
/// - `second_longitude`/`second_latitude`：终点经纬度（弧度）。
///
/// # 返回
/// 含表面距离、起/止方位角与 `u²` 的 `VincentyResult`。
fn vincenty_inverse_formula(
    major: f64,
    minor: f64,
    first_longitude: f64,
    first_latitude: f64,
    second_longitude: f64,
    second_latitude: f64,
) -> VincentyResult {
    // 扁率与经度差 L。
    let eff = (major - minor) / major;
    let l = second_longitude - first_longitude;

    // 约化纬度 U1/U2：将大地纬度压缩到辅助球上。
    let u1 = ((1.0 - eff) * first_latitude.tan()).atan();
    let u2 = ((1.0 - eff) * second_latitude.tan()).atan();

    let cosine_u1 = u1.cos();
    let sine_u1 = u1.sin();
    let cosine_u2 = u2.cos();
    let sine_u2 = u2.sin();

    // 预存四个约化纬度的正弦/余弦乘积组合，供迭代复用。
    let cc = cosine_u1 * cosine_u2;
    let cs = cosine_u1 * sine_u2;
    let ss = sine_u1 * sine_u2;
    let sc = sine_u1 * cosine_u2;

    let mut lambda = l;

    // 反复更新 λ 直至相邻两次差值小于容差（不动点迭代）。
    let iter = loop {
        let cosine_lambda = lambda.cos();
        let sine_lambda = lambda.sin();

        let temp = cs - sc * cosine_lambda;
        let sine_sigma =
            (cosine_u2 * cosine_u2 * sine_lambda * sine_lambda + temp * temp).sqrt();
        let cosine_sigma = ss + cc * cosine_lambda;

        // σ：球面辅助角，由正弦/余弦联合定象限。
        let sigma = sine_sigma.atan2(cosine_sigma);

        let (sine_alpha, cosine_squared_alpha) = if sine_sigma == 0.0 {
            (0.0, 1.0)
        } else {
            let sa = cc * sine_lambda / sine_sigma;
            (sa, 1.0 - sa * sa)
        };

        let lambda_dot = lambda;

        let mut cosine_twice_sigma_midpoint =
            cosine_sigma - 2.0 * ss / cosine_squared_alpha;
        if !cosine_twice_sigma_midpoint.is_finite() {
            cosine_twice_sigma_midpoint = 0.0;
        }

        lambda = l
            + compute_delta_lambda(
                eff,
                sine_alpha,
                cosine_squared_alpha,
                sigma,
                sine_sigma,
                cosine_sigma,
                cosine_twice_sigma_midpoint,
            );

        if (lambda - lambda_dot).abs() <= EPSILON12 {
            break VincentyIteration {
                sigma,
                sine_sigma,
                cosine_sigma,
                cosine_squared_alpha,
                cosine_twice_sigma_midpoint,
                lambda,
            };
        }
    };

    let cosine_squared_alpha = iter.cosine_squared_alpha;
    let u_squared =
        cosine_squared_alpha * (major * major - minor * minor) / (minor * minor);
    let big_a = 1.0
        + u_squared
            * (4096.0 + u_squared * (u_squared * (320.0 - 175.0 * u_squared) - 768.0))
            / 16384.0;
    let big_b = u_squared
        * (256.0 + u_squared * (u_squared * (74.0 - 47.0 * u_squared) - 128.0))
        / 1024.0;

    let cosine_twice_sigma_midpoint = iter.cosine_twice_sigma_midpoint;
    let cosine_squared_twice_sigma_midpoint =
        cosine_twice_sigma_midpoint * cosine_twice_sigma_midpoint;
    let sine_sigma = iter.sine_sigma;
    let cosine_sigma = iter.cosine_sigma;
    let delta_sigma = big_b
        * sine_sigma
        * (cosine_twice_sigma_midpoint
            + big_b
                * (cosine_sigma * (2.0 * cosine_squared_twice_sigma_midpoint - 1.0)
                    - big_b
                        * cosine_twice_sigma_midpoint
                        * (4.0 * sine_sigma * sine_sigma - 3.0)
                        * (4.0 * cosine_squared_twice_sigma_midpoint - 3.0)
                        / 6.0)
                / 4.0);

    let distance = minor * big_a * (iter.sigma - delta_sigma);

    let cosine_lambda = iter.lambda.cos();
    let sine_lambda = iter.lambda.sin();
    let start_heading = (cosine_u2 * sine_lambda).atan2(cs - sc * cosine_lambda);
    let end_heading = (cosine_u1 * sine_lambda).atan2(cs * cosine_lambda - sc);

    VincentyResult {
        distance,
        start_heading,
        end_heading,
        u_squared,
    }
}

/// 为插值预计算大地线级数展开常量。
///
/// # 参数
/// - `start`：起点大地坐标（高度应为 0）。
/// - `start_heading`：起点方位角（弧度）。
/// - `u_squared`：由 Vincenty 反算得到的 `u²`。
/// - `maximum_radius`/`minimum_radius`：椭球长/短半轴（米）。
///
/// # 返回
/// 汇总余弦/正弦、σ、级数系数与距离比值的 `GeodesicConstants`。
fn set_constants(
    start: &Cartographic,
    start_heading: f64,
    u_squared: f64,
    maximum_radius: f64,
    minimum_radius: f64,
) -> GeodesicConstants {
    let a = maximum_radius;
    let b = minimum_radius;
    let f = (a - b) / a;

    let cosine_heading = start_heading.cos();
    let sine_heading = start_heading.sin();

    // 由起点纬度求约化纬度正切，进而得 σ₁ 与赤道方位角。
    let tan_u = (1.0 - f) * start.latitude.tan();

    let cosine_u = 1.0 / (1.0 + tan_u * tan_u).sqrt();
    let sine_u = cosine_u * tan_u;

    let sigma = tan_u.atan2(cosine_heading);

    let sine_alpha = cosine_u * sine_heading;
    let sine_squared_alpha = sine_alpha * sine_alpha;

    let cosine_squared_alpha = 1.0 - sine_squared_alpha;
    let cosine_alpha = cosine_squared_alpha.sqrt();

    let u2_over4 = u_squared / 4.0;
    let u4_over16 = u2_over4 * u2_over4;
    let u6_over64 = u4_over16 * u2_over4;
    let u8_over256 = u4_over16 * u4_over16;

    // 级数系数 a0..a3 用于把弧长展开成 σ 的三角级数。
    let a0 = 1.0 + u2_over4 - 3.0 * u4_over16 / 4.0 + 5.0 * u6_over64 / 4.0
        - 175.0 * u8_over256 / 64.0;
    let a1 = 1.0 - u2_over4 + 15.0 * u4_over16 / 8.0 - 35.0 * u6_over64 / 8.0;
    let a2 = 1.0 - 3.0 * u2_over4 + 35.0 * u4_over16 / 4.0;
    let a3 = 1.0 - 5.0 * u2_over4;

    let distance_ratio = a0 * sigma
        - a1 * (2.0 * sigma).sin() * u2_over4 / 2.0
        - a2 * (4.0 * sigma).sin() * u4_over16 / 16.0
        - a3 * (6.0 * sigma).sin() * u6_over64 / 48.0
        - (8.0 * sigma).sin() * 5.0 * u8_over256 / 512.0;

    GeodesicConstants {
        a,
        b,
        f,
        cosine_heading,
        sine_heading,
        cosine_u,
        sine_u,
        sigma,
        sine_alpha,
        cosine_squared_alpha,
        cosine_alpha,
        u2_over4,
        u4_over16,
        u6_over64,
        u8_over256,
        distance_ratio,
    }
}

impl EllipsoidGeodesic {
    /// 在给定椭球上创建一条从 `start` 连接到 `end` 的大地线。
    ///
    /// 映射到 `EllipsoidGeodesic` 构造函数 / `setEndPoints`。
    pub fn new(start: Cartographic, end: Cartographic, ellipsoid: &Ellipsoid) -> Self {
        Self::from_radii(start, end, ellipsoid.maximum_radius(), ellipsoid.minimum_radius())
    }

    /// 由显式的椭球半径创建一条大地线。
    fn from_radii(
        start: Cartographic,
        end: Cartographic,
        maximum_radius: f64,
        minimum_radius: f64,
    ) -> Self {
        let vincenty = vincenty_inverse_formula(
            maximum_radius,
            minimum_radius,
            start.longitude,
            start.latitude,
            end.longitude,
            end.latitude,
        );

        let mut start0 = start;
        let mut end0 = end;
        start0.height = 0.0;
        end0.height = 0.0;

        let constants = set_constants(
            &start0,
            vincenty.start_heading,
            vincenty.u_squared,
            maximum_radius,
            minimum_radius,
        );

        Self {
            start: start0,
            end: end0,
            start_heading: vincenty.start_heading,
            end_heading: vincenty.end_heading,
            distance: vincenty.distance,
            constants,
            maximum_radius,
            minimum_radius,
        }
    }

    /// 重置大地线的端点。
    pub fn set_end_points(&mut self, start: Cartographic, end: Cartographic) {
        *self = Self::from_radii(start, end, self.maximum_radius, self.minimum_radius);
    }

    /// 起点与终点之间的表面距离。
    pub fn surface_distance(&self) -> f64 {
        self.distance
    }

    /// 起点处的方位角。
    pub fn start_heading(&self) -> f64 {
        self.start_heading
    }

    /// 终点处的方位角。
    pub fn end_heading(&self) -> f64 {
        self.end_heading
    }

    /// 大地线的起点。
    pub fn start(&self) -> Cartographic {
        self.start
    }

    /// 大地线的终点。
    pub fn end(&self) -> Cartographic {
        self.end
    }

    /// 在大地上按给定比例（0..1）插值一个点。
    ///
    /// # 参数
    /// - `fraction`：沿总表面距离的比例。
    ///
    /// # 返回
    /// 对应比例处的 `Cartographic` 点。
    pub fn interpolate_using_fraction(&self, fraction: f64) -> Cartographic {
        self.interpolate_using_surface_distance(self.distance * fraction)
    }

    /// 从起点开始按给定表面距离插值一个点。
    ///
    /// 映射到 `EllipsoidGeodesic.interpolateUsingSurfaceDistance`。
    pub fn interpolate_using_surface_distance(&self, distance: f64) -> Cartographic {
        let c = &self.constants;

        let s = c.distance_ratio + distance / c.b;

        let cosine2s = (2.0 * s).cos();
        let cosine4s = (4.0 * s).cos();
        let cosine6s = (6.0 * s).cos();
        let sine2s = (2.0 * s).sin();
        let sine4s = (4.0 * s).sin();
        let sine6s = (6.0 * s).sin();
        let sine8s = (8.0 * s).sin();

        let s2 = s * s;
        let s3 = s * s2;

        let u8_over256 = c.u8_over256;
        let u2_over4 = c.u2_over4;
        let u6_over64 = c.u6_over64;
        let u4_over16 = c.u4_over16;

        let mut sigma = (2.0 * s3 * u8_over256 * cosine2s) / 3.0
            + s * (1.0 - u2_over4 + 7.0 * u4_over16 / 4.0 - 15.0 * u6_over64 / 4.0
                + 579.0 * u8_over256 / 64.0
                - (u4_over16 - 15.0 * u6_over64 / 4.0 + 187.0 * u8_over256 / 16.0)
                    * cosine2s
                - (5.0 * u6_over64 / 4.0 - 115.0 * u8_over256 / 16.0) * cosine4s
                - 29.0 * u8_over256 * cosine6s / 16.0)
            + (u2_over4 / 2.0 - u4_over16 + 71.0 * u6_over64 / 32.0
                - 85.0 * u8_over256 / 16.0)
                * sine2s
            + (5.0 * u4_over16 / 16.0 - 5.0 * u6_over64 / 4.0
                + 383.0 * u8_over256 / 96.0)
                * sine4s
            - s2 * ((u6_over64 - 11.0 * u8_over256 / 2.0) * sine2s
                + 5.0 * u8_over256 * sine4s / 2.0)
            + (29.0 * u6_over64 / 96.0 - 29.0 * u8_over256 / 16.0) * sine6s
            + 539.0 * u8_over256 * sine8s / 1536.0;

        let theta = (sigma.sin() * c.cosine_alpha).asin();
        let latitude = ((c.a / c.b) * theta.tan()).atan();

        // 用以纬度幅角的相对参数重新定义。
        sigma -= c.sigma;

        let cosine_twice_sigma_midpoint = (2.0 * c.sigma + sigma).cos();

        let sine_sigma = sigma.sin();
        let cosine_sigma = sigma.cos();

        let cc = c.cosine_u * cosine_sigma;
        let ss = c.sine_u * sine_sigma;

        let lambda =
            (sine_sigma * c.sine_heading).atan2(cc - ss * c.cosine_heading);

        let l = lambda
            - compute_delta_lambda(
                c.f,
                c.sine_alpha,
                c.cosine_squared_alpha,
                sigma,
                sine_sigma,
                cosine_sigma,
                cosine_twice_sigma_midpoint,
            );

        Cartographic::from_radians(self.start.longitude + l, latitude, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math_utils::to_radians;

    /// 赤道上 1° 弧长应约等于 半径×角（弧度）。
    #[test]
    fn test_surface_distance_equator() {
        // WGS84 赤道上 1 度约 111319.49 米
        let ell = Ellipsoid::WGS84;
        let start = Cartographic::from_degrees(0.0, 0.0, 0.0);
        let end = Cartographic::from_degrees(1.0, 0.0, 0.0);
        let g = EllipsoidGeodesic::new(start, end, &ell);
        let expected = to_radians(1.0) * ell.maximum_radius();
        assert!(
            (g.surface_distance() - expected).abs() < 1.0,
            "distance {}",
            g.surface_distance()
        );
    }

    /// 赤道两点间中点插值：经度应为半程、纬度为 0。
    #[test]
    fn test_interpolate_midpoint() {
        let ell = Ellipsoid::WGS84;
        let start = Cartographic::from_degrees(0.0, 0.0, 0.0);
        let end = Cartographic::from_degrees(10.0, 0.0, 0.0);
        let g = EllipsoidGeodesic::new(start, end, &ell);
        let mid = g.interpolate_using_fraction(0.5);
        assert!(
            (to_radians(5.0) - mid.longitude).abs() < 1e-6,
            "mid lon {}",
            mid.longitude
        );
        assert!(mid.latitude.abs() < 1e-6, "mid lat {}", mid.latitude);
    }

    /// 端点插值：距离 0 与全长应分别还原起点与终点。
    #[test]
    fn test_interpolate_endpoints() {
        let ell = Ellipsoid::WGS84;
        let start = Cartographic::from_degrees(-105.0, 40.0, 0.0);
        let end = Cartographic::from_degrees(-100.0, 38.0, 0.0);
        let g = EllipsoidGeodesic::new(start, end, &ell);
        let p0 = g.interpolate_using_surface_distance(0.0);
        assert!((p0.longitude - start.longitude).abs() < 1e-9);
        assert!((p0.latitude - start.latitude).abs() < 1e-9);
        let p1 = g.interpolate_using_surface_distance(g.surface_distance());
        assert!((p1.longitude - end.longitude).abs() < 1e-6);
        assert!((p1.latitude - end.latitude).abs() < 1e-6);
    }
}
