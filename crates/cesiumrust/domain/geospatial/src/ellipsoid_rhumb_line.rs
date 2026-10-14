//! EllipsoidRhumbLine - 椭球上的恒向线（斜航线）。
//!
//! 恒向线（loxodrome）是一条与所有经线相交成恒定方位角的曲线。本模块提供
//! [`EllipsoidRhumbLine`] 及其构造、表面距离/方位角计算、与给定经纬度求交、
//! 以及沿线的距离/比例插值等方法。
//!
//! 内部度量依赖于两组级数展开：`calculate_m` 沿子午线求等距纬度量，
//! `calculate_inverse_m` 作其反变换；`calculate_sigma` 给出等角纬度量，用于
//! 在经纬度间换算恒向线的方位角与经度差。当椭球退化为球时各公式自动简化。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::too_many_arguments, clippy::needless_late_init)]
use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use crate::math_utils::{equals_epsilon, negative_pi_to_pi, sign, EPSILON10, EPSILON12, EPSILON14, EPSILON8, PI_OVER_TWO};

/// 由纬度计算子午线弧长 `m`（等距纬度量，米）。
///
/// 采用勒让德级数展开到 e¹²，将大地纬度转换为沿子午线的展开长度；椭球退
/// 化为球（`ellipticity == 0`）时直接返回 `major · latitude`。
///
/// # 参数
/// - `ellipticity`：第一偏心率 e。
/// - `major`：长半轴（米）。
/// - `latitude`：大地纬度（弧度）。
///
/// # 返回
/// 对应纬度的子午线弧长（米）。
fn calculate_m(ellipticity: f64, major: f64, latitude: f64) -> f64 {
    if ellipticity == 0.0 {
        return major * latitude;
    }

    let e2 = ellipticity * ellipticity;
    let e4 = e2 * e2;
    let e6 = e4 * e2;
    let e8 = e6 * e2;
    let e10 = e8 * e2;
    let e12 = e10 * e2;
    let phi = latitude;
    let sin2_phi = (2.0 * phi).sin();
    let sin4_phi = (4.0 * phi).sin();
    let sin6_phi = (6.0 * phi).sin();
    let sin8_phi = (8.0 * phi).sin();
    let sin10_phi = (10.0 * phi).sin();
    let sin12_phi = (12.0 * phi).sin();

    // 子午线弧长的勒让德级数：主项正比于 φ，其余各项按 sin(2kφ) 展开，
    // 系数为偏心率 e 的偶次幂组合，展开到 e¹² 以保证椭球上的高精度。
    major
        * ((1.0 - e2 / 4.0 - (3.0 * e4) / 64.0 - (5.0 * e6) / 256.0
            - (175.0 * e8) / 16384.0
            - (441.0 * e10) / 65536.0
            - (4851.0 * e12) / 1048576.0)
            * phi
            - ((3.0 * e2) / 8.0
                + (3.0 * e4) / 32.0
                + (45.0 * e6) / 1024.0
                + (105.0 * e8) / 4096.0
                + (2205.0 * e10) / 131072.0
                + (6237.0 * e12) / 524288.0)
                * sin2_phi
            + ((15.0 * e4) / 256.0
                + (45.0 * e6) / 1024.0
                + (525.0 * e8) / 16384.0
                + (1575.0 * e10) / 65536.0
                + (155925.0 * e12) / 8388608.0)
                * sin4_phi
            - ((35.0 * e6) / 3072.0
                + (175.0 * e8) / 12288.0
                + (3675.0 * e10) / 262144.0
                + (13475.0 * e12) / 1048576.0)
                * sin6_phi
            + ((315.0 * e8) / 131072.0
                + (2205.0 * e10) / 524288.0
                + (43659.0 * e12) / 8388608.0)
                * sin8_phi
            - ((693.0 * e10) / 1310720.0 + (6237.0 * e12) / 5242880.0) * sin10_phi
            + ((1001.0 * e12) / 8388608.0) * sin12_phi)
}

/// 由子午线弧长 `m` 反解对应纬度（`calculate_m` 的逆变换）。
///
/// # 参数
/// - `m`：子午线弧长（米）。
/// - `ellipticity`：第一偏心率 e。
/// - `major`：长半轴（米）。
///
/// # 返回
/// 归一化纬度（弧度）。
fn calculate_inverse_m(m: f64, ellipticity: f64, major: f64) -> f64 {
    let d = m / major;

    if ellipticity == 0.0 {
        return d;
    }

    let d2 = d * d;
    let d3 = d2 * d;
    let d4 = d3 * d;
    let e = ellipticity;
    let e2 = e * e;
    let e4 = e2 * e2;
    let e6 = e4 * e2;
    let e8 = e6 * e2;
    let e10 = e8 * e2;
    let e12 = e10 * e2;
    let sin2_d = (2.0 * d).sin();
    let cos2_d = (2.0 * d).cos();
    let sin4_d = (4.0 * d).sin();
    let cos4_d = (4.0 * d).cos();
    let sin6_d = (6.0 * d).sin();
    let cos6_d = (6.0 * d).cos();
    let sin8_d = (8.0 * d).sin();
    let cos8_d = (8.0 * d).cos();
    let sin10_d = (10.0 * d).sin();
    let cos10_d = (10.0 * d).cos();
    let sin12_d = (12.0 * d).sin();

    // 反演级数：以 d = m/major 为变量，把纬度展开为 d 与 e 幂次的多项式，
    // 含 cos(2k·d) 与 sin(2k·d) 两类修正项，精度同样到 e¹²。
    d + (d * e2) / 4.0
        + (7.0 * d * e4) / 64.0
        + (15.0 * d * e6) / 256.0
        + (579.0 * d * e8) / 16384.0
        + (1515.0 * d * e10) / 65536.0
        + (16837.0 * d * e12) / 1048576.0
        + ((3.0 * d * e4) / 16.0 + (45.0 * d * e6) / 256.0
            - (d * (32.0 * d2 - 561.0) * e8) / 4096.0
            - (d * (232.0 * d2 - 1677.0) * e10) / 16384.0
            + (d * (399985.0 - 90560.0 * d2 + 512.0 * d4) * e12) / 5242880.0)
            * cos2_d
        + ((21.0 * d * e6) / 256.0 + (483.0 * d * e8) / 4096.0
            - (d * (224.0 * d2 - 1969.0) * e10) / 16384.0
            - (d * (33152.0 * d2 - 112599.0) * e12) / 1048576.0)
            * cos4_d
        + ((151.0 * d * e8) / 4096.0
            + (4681.0 * d * e10) / 65536.0
            + (1479.0 * d * e12) / 16384.0
            - (453.0 * d3 * e12) / 32768.0)
            * cos6_d
        + ((1097.0 * d * e10) / 65536.0 + (42783.0 * d * e12) / 1048576.0) * cos8_d
        + ((8011.0 * d * e12) / 1048576.0) * cos10_d
        + ((3.0 * e2) / 8.0
            + (3.0 * e4) / 16.0
            + (213.0 * e6) / 2048.0
            - (3.0 * d2 * e6) / 64.0
            + (255.0 * e8) / 4096.0
            - (33.0 * d2 * e8) / 512.0
            + (20861.0 * e10) / 524288.0
            - (33.0 * d2 * e10) / 512.0
            + (d4 * e10) / 1024.0
            + (28273.0 * e12) / 1048576.0
            - (471.0 * d2 * e12) / 8192.0
            + (9.0 * d4 * e12) / 4096.0)
            * sin2_d
        + ((21.0 * e4) / 256.0
            + (21.0 * e6) / 256.0
            + (533.0 * e8) / 8192.0
            - (21.0 * d2 * e8) / 512.0
            + (197.0 * e10) / 4096.0
            - (315.0 * d2 * e10) / 4096.0
            + (584039.0 * e12) / 16777216.0
            - (12517.0 * d2 * e12) / 131072.0
            + (7.0 * d4 * e12) / 2048.0)
            * sin4_d
        + ((151.0 * e6) / 6144.0
            + (151.0 * e8) / 4096.0
            + (5019.0 * e10) / 131072.0
            - (453.0 * d2 * e10) / 16384.0
            + (26965.0 * e12) / 786432.0
            - (8607.0 * d2 * e12) / 131072.0)
            * sin6_d
        + ((1097.0 * e8) / 131072.0
            + (1097.0 * e10) / 65536.0
            + (225797.0 * e12) / 10485760.0
            - (1097.0 * d2 * e12) / 65536.0)
            * sin8_d
        + ((8011.0 * e10) / 2621440.0 + (8011.0 * e12) / 1048576.0) * sin10_d
        + ((293393.0 * e12) / 251658240.0) * sin12_d
}

/// 计算纬度对应的等角纬度量 `σ`（isometric latitude）。
///
/// # 参数
/// - `ellipticity`：第一偏心率 e。
/// - `latitude`：大地纬度（弧度）。
///
/// # 返回
/// `ln(tan(π/4 + φ/2))` 减去含偏心率的修正项。
fn calculate_sigma(ellipticity: f64, latitude: f64) -> f64 {
    if ellipticity == 0.0 {
        return (0.5 * (PI_OVER_TWO + latitude)).tan().ln();
    }

    // 球面项减去椭球修正项（含 e·sinφ 的对数比）。
    let e_sin_l = ellipticity * latitude.sin();
    (0.5 * (PI_OVER_TWO + latitude)).tan().ln()
        - (ellipticity / 2.0) * ((1.0 + e_sin_l) / (1.0 - e_sin_l)).ln()
}

/// 计算由起点指向终点的恒向线方位角。
///
/// # 参数
/// - `ellipticity`：第一偏心率 e。
/// - `first_longitude`/`first_latitude`：起点经纬度（弧度）。
/// - `second_longitude`/`second_latitude`：终点经纬度（弧度）。
///
/// # 返回
/// 方位角（弧度），由两点等角纬度差与经度差共同确定。
fn calculate_heading(
    ellipticity: f64,
    first_longitude: f64,
    first_latitude: f64,
    second_longitude: f64,
    second_latitude: f64,
) -> f64 {
    // 方位角即经度差与等角纬度差在切平面上的反正切。
    let sigma1 = calculate_sigma(ellipticity, first_latitude);
    let sigma2 = calculate_sigma(ellipticity, second_latitude);
    (negative_pi_to_pi(second_longitude - first_longitude)).atan2(sigma2 - sigma1)
}

/// 计算恒向线上两点间的表面弧长（米）。
///
/// # 参数
/// - `ellipticity`/`ellipticity_squared`：第一偏心率 e 与 e²。
/// - `major`/`minor`：长/短半轴（米）。
/// - `heading`：恒向线方位角（弧度）。
/// - `first_latitude`/`second_latitude`：起/止纬度（弧度）。
/// - `delta_longitude`：经度差（弧度）。
///
/// # 返回
/// 表面弧长的绝对值（米）；东西向（heading≈±π/2）沿平行圈量取。
fn calculate_arc_length(
    ellipticity: f64,
    ellipticity_squared: f64,
    major: f64,
    minor: f64,
    heading: f64,
    first_latitude: f64,
    second_latitude: f64,
    delta_longitude: f64,
) -> f64 {
    let distance;

    // 检查该恒向线是否具有恒定纬度
    if equals_epsilon(heading.abs(), PI_OVER_TWO, EPSILON8, EPSILON8) {
        // 若 heading 接近 90 度
        if major == minor {
            distance = major * first_latitude.cos() * negative_pi_to_pi(delta_longitude);
        } else {
            let sin_phi = first_latitude.sin();
            distance = (major * first_latitude.cos() * negative_pi_to_pi(delta_longitude))
                / (1.0 - ellipticity_squared * sin_phi * sin_phi).sqrt();
        }
    } else {
        let m1 = calculate_m(ellipticity, major, first_latitude);
        let m2 = calculate_m(ellipticity, major, second_latitude);
        distance = (m2 - m1) / heading.cos();
    }
    distance.abs()
}

/// 沿恒向线自起点前进给定表面距离，返回终点大地坐标。
///
/// # 参数
/// - `start`：起点大地坐标。
/// - `heading`：恒向线方位角（弧度）。
/// - `distance`：沿恒向线行进的表面距离（米）。
/// - `major`：长半轴（米）。
/// - `ellipticity`：第一偏心率 e。
///
/// # 返回
/// 距离对应的终点大地坐标（高度置 0）。
fn interpolate_using_surface_distance(
    start: &Cartographic,
    heading: f64,
    distance: f64,
    major: f64,
    ellipticity: f64,
) -> Cartographic {
    if distance == 0.0 {
        return *start;
    }

    let ellipticity_squared = ellipticity * ellipticity;

    let longitude;
    let latitude;

    // 检查该恒向线是否具有恒定纬度
    if (PI_OVER_TWO - heading.abs()).abs() > EPSILON8 {
        // 计算第二个点的纬度
        let m1 = calculate_m(ellipticity, major, start.latitude);
        let delta_m = distance * heading.cos();
        let m2 = m1 + delta_m;
        latitude = calculate_inverse_m(m2, ellipticity, major);

        // 现在查找第二个点的经度
        if heading.abs() < EPSILON10 {
            longitude = negative_pi_to_pi(start.longitude);
        } else {
            let sigma1 = calculate_sigma(ellipticity, start.latitude);
            let sigma2 = calculate_sigma(ellipticity, latitude);
            let delta_longitude = heading.tan() * (sigma2 - sigma1);
            longitude = negative_pi_to_pi(start.longitude + delta_longitude);
        }
    } else {
        // 若 heading 接近 90 度
        latitude = start.latitude;
        let local_rad;

        if ellipticity == 0.0 {
            local_rad = major * start.latitude.cos();
        } else {
            let sin_phi = start.latitude.sin();
            local_rad =
                (major * start.latitude.cos()) / (1.0 - ellipticity_squared * sin_phi * sin_phi).sqrt();
        }

        let delta_longitude = distance / local_rad;
        if heading > 0.0 {
            longitude = negative_pi_to_pi(start.longitude + delta_longitude);
        } else {
            longitude = negative_pi_to_pi(start.longitude - delta_longitude);
        }
    }

    Cartographic::from_radians(longitude, latitude, 0.0)
}

/// 椭球上的恒向线（斜航线）。
/// 映射到 CesiumJS `EllipsoidRhumbLine`
#[derive(Clone, Debug)]
pub struct EllipsoidRhumbLine {
    /// 起点大地坐标（高度已置 0）。
    start: Cartographic,
    /// 终点大地坐标（高度已置 0）。
    end: Cartographic,
    /// 恒向线方位角（弧度，正北为 0，顺时针）。
    heading: f64,
    /// 起终点间的表面距离（米）。
    distance: f64,
    /// 椭球第一偏心率 e。
    ellipticity: f64,
    /// 第一偏心率的平方 e²。
    ellipticity_squared: f64,
    /// 长半轴（米）。
    major: f64,
    /// 短半轴（米）；当前仅参与偏心率推导，暂未直接读取。
    #[allow(dead_code)]
    minor: f64,
}

impl EllipsoidRhumbLine {
    /// 由给定椭球上的起点和终点测绘坐标创建一条新的恒向线。
    pub fn new(start: &Cartographic, end: &Cartographic, ellipsoid: &Ellipsoid) -> Self {
        let major = ellipsoid.maximum_radius();
        let minor = ellipsoid.minimum_radius();
        let major_squared = major * major;
        let minor_squared = minor * minor;
        let ellipticity_squared = (major_squared - minor_squared) / major_squared;
        let ellipticity = ellipticity_squared.sqrt();

        // 先求恒向线方位角，再据此计算起终点间的表面距离。
        let heading = calculate_heading(
            ellipticity,
            start.longitude,
            start.latitude,
            end.longitude,
            end.latitude,
        );

        let delta_longitude = end.longitude - start.longitude;
        let distance = calculate_arc_length(
            ellipticity,
            ellipticity_squared,
            major,
            minor,
            heading,
            start.latitude,
            end.latitude,
            delta_longitude,
        );

        let mut s = *start;
        s.height = 0.0;
        let mut e = *end;
        e.height = 0.0;

        Self {
            start: s,
            end: e,
            heading,
            distance,
            ellipticity,
            ellipticity_squared,
            major,
            minor,
        }
    }

    /// 设置新的端点并重新计算各属性。
    pub fn set_end_points(&mut self, start: &Cartographic, end: &Cartographic) {
        let heading = calculate_heading(
            self.ellipticity,
            start.longitude,
            start.latitude,
            end.longitude,
            end.latitude,
        );

        let delta_longitude = end.longitude - start.longitude;
        let distance = calculate_arc_length(
            self.ellipticity,
            self.ellipticity_squared,
            self.major,
            self.minor,
            heading,
            start.latitude,
            end.latitude,
            delta_longitude,
        );

        let mut s = *start;
        s.height = 0.0;
        let mut e = *end;
        e.height = 0.0;

        self.start = s;
        self.end = e;
        self.heading = heading;
        self.distance = distance;
    }

    /// 获取起点与终点之间的表面距离。
    pub fn surface_distance(&self) -> f64 {
        self.distance
    }

    /// 获取恒向线的方位角。
    pub fn heading(&self) -> f64 {
        self.heading
    }

    /// 获取起点。
    pub fn start(&self) -> Cartographic {
        self.start
    }

    /// 获取终点。
    pub fn end(&self) -> Cartographic {
        self.end
    }

    /// 由起点、方位角和距离创建一条恒向线。
    ///
    /// 映射到 `EllipsoidRhumbLine.fromStartHeadingDistance`。
    pub fn from_start_heading_distance(
        start: &Cartographic,
        heading: f64,
        distance: f64,
        ellipsoid: &Ellipsoid,
    ) -> Self {
        let major = ellipsoid.maximum_radius();
        let minor = ellipsoid.minimum_radius();
        let major_squared = major * major;
        let minor_squared = minor * minor;
        let ellipticity_squared = (major_squared - minor_squared) / major_squared;
        let ellipticity = ellipticity_squared.sqrt();

        let heading = negative_pi_to_pi(heading);
        let end = interpolate_using_surface_distance(start, heading, distance, major, ellipticity);

        Self::new(start, &end, ellipsoid)
    }

    /// 查找恒向线与给定经度的交点。
    ///
    /// 对于经度不匹配的南北向线返回 `None`。
    /// 映射到 `EllipsoidRhumbLine.prototype.findIntersectionWithLongitude`。
    pub fn find_intersection_with_longitude(&self, intersection_longitude: f64) -> Option<Cartographic> {
        let ellipticity = self.ellipticity;
        let heading = self.heading;
        let abs_heading = heading.abs();
        let start = &self.start;

        let mut intersection_longitude = negative_pi_to_pi(intersection_longitude);

        if equals_epsilon(intersection_longitude.abs(), std::f64::consts::PI, EPSILON14, EPSILON14) {
            intersection_longitude = sign(start.longitude) * std::f64::consts::PI;
        }

        // 东西向恒向线（heading ~ ±PI/2）
        if (PI_OVER_TWO - abs_heading).abs() <= EPSILON8 {
            return Some(Cartographic::from_radians(intersection_longitude, start.latitude, 0.0));
        }

        // 南北向恒向线（heading ~ 0 或 PI）
        if equals_epsilon((PI_OVER_TWO - abs_heading).abs(), PI_OVER_TWO, EPSILON8, EPSILON8) {
            if equals_epsilon(intersection_longitude, start.longitude, EPSILON12, EPSILON12) {
                return None;
            }
            let latitude = PI_OVER_TWO * sign(PI_OVER_TWO - heading);
            return Some(Cartographic::from_radians(intersection_longitude, latitude, 0.0));
        }

        // 来自 http://edwilliams.org/ellipsoid/ellipsoid.pdf 第 9 个公式的迭代求解器
        // 反复用等角纬度的闭式解更新 φ，直到相邻两次差值进入 EPSILON12 容差。
        let phi1 = start.latitude;
        let e_sin_phi1 = ellipticity * phi1.sin();
        let left_component = (0.5 * (PI_OVER_TWO + phi1)).tan()
            * ((intersection_longitude - start.longitude) / heading.tan()).exp();
        let denominator = (1.0 + e_sin_phi1) / (1.0 - e_sin_phi1);

        let mut new_phi = start.latitude;
        let new_phi_result;
        loop {
            let phi = new_phi;
            let e_sin_phi = ellipticity * phi.sin();
            let numerator = (1.0 + e_sin_phi) / (1.0 - e_sin_phi);
            new_phi = 2.0
                * (left_component * (numerator / denominator).powf(ellipticity / 2.0)).atan()
                - PI_OVER_TWO;
            if equals_epsilon(new_phi, phi, EPSILON12, EPSILON12) {
                new_phi_result = new_phi;
                break;
            }
        }

        Some(Cartographic::from_radians(intersection_longitude, new_phi_result, 0.0))
    }

    /// 查找恒向线与给定纬度的交点。
    ///
    /// 对于东西向线（恒定纬度）返回 `None`。
    /// 映射到 `EllipsoidRhumbLine.prototype.findIntersectionWithLatitude`。
    pub fn find_intersection_with_latitude(&self, intersection_latitude: f64) -> Option<Cartographic> {
        let ellipticity = self.ellipticity;
        let heading = self.heading;
        let start = &self.start;

        // 东西向恒向线：无交点或有无穷多个交点
        if equals_epsilon(heading.abs(), PI_OVER_TWO, EPSILON8, EPSILON8) {
            return None;
        }

        let sigma1 = calculate_sigma(ellipticity, start.latitude);
        let sigma2 = calculate_sigma(ellipticity, intersection_latitude);
        let delta_longitude = heading.tan() * (sigma2 - sigma1);
        let longitude = negative_pi_to_pi(start.longitude + delta_longitude);

        Some(Cartographic::from_radians(longitude, intersection_latitude, 0.0))
    }

    /// 在恒向线上按给定比例（0..1）插值一个点。
    pub fn interpolate_using_fraction(&self, fraction: f64) -> Cartographic {
        // 比例先换算成沿恒向线的表面距离，再委托距离版插值。
        self.interpolate_using_surface_distance(fraction * self.distance)
    }

    /// 在恒向线上按给定表面距离插值一个点。
    pub fn interpolate_using_surface_distance(&self, distance: f64) -> Cartographic {
        interpolate_using_surface_distance(
            &self.start,
            self.heading,
            distance,
            self.major,
            self.ellipticity,
        )
    }
}
