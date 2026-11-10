//! Ellipsoid —— 在笛卡尔坐标中定义的一个二次曲面。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::needless_return)]
use crate::cartographic::Cartographic;
use crate::math_utils::{self, EPSILON1, EPSILON12, EPSILON14, EPSILON15, LUNAR_RADIUS, TWO_PI};
use crate::rectangle::Rectangle;
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

/// 由方程 `(x / a)^2 + (y / b)^2 + (z / c)^2 = 1`
/// 在笛卡尔坐标中定义的二次曲面。
/// 主要用于表示天体的形状。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ellipsoid {
    /// 椭球的半径 (x, y, z)。
    radii: DVec3,
    /// 半径的平方。
    radii_squared: DVec3,
    /// 半径的四次方。
    radii_to_the_fourth: DVec3,
    /// 半径的倒数。
    one_over_radii: DVec3,
    /// 半径平方的倒数。
    one_over_radii_squared: DVec3,
    /// 最小半径。
    minimum_radius: f64,
    /// 最大半径。
    maximum_radius: f64,
    /// 靠近中心的容差。
    center_tolerance_squared: f64,
    /// squaredXOverSquaredZ
    squared_x_over_squared_z: f64,
}

/// 通过将每个分量除以其模长来归一化一个 Cartesian3。
///
/// 这是对 CesiumJS `Cartesian3.normalize` 的逐位精确移植，后者计算
/// `component / magnitude`（每个分量一次正确舍入的 IEEE-754 除法）。
/// glam 的 `DVec3::normalize` 则计算 `component * (1.0 / length)`
/// （乘以倒数，两次舍入），结果可能与 CesiumJS 相差 1 ulp。
/// 为了对照原版 CesiumJS Specs（即真值）进行验证，必须使用直接除法的形式。
/// 凡移植 CesiumJS `Cartesian3.normalize` 之处，都应使用本辅助函数。
#[inline]
pub fn normalize_cartesian3(v: DVec3) -> DVec3 {
    let magnitude = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    DVec3::new(v.x / magnitude, v.y / magnitude, v.z / magnitude)
}

impl Ellipsoid {
    /// WGS84 椭球：a = 6378137.0, b = 6378137.0, c = 6356752.3142451793
    #[allow(clippy::excessive_precision)]
    pub const WGS84: Self = Self::from_radii_unchecked(
        6378137.0,
        6378137.0,
        6356752.3142451793,
    );

    /// 单位球（各方向半径均为 1）。
    pub const UNIT_SPHERE: Self = Self::from_radii_unchecked(1.0, 1.0, 1.0);

    /// 月球椭球（`Ellipsoid.MOON`）：一个半径为
    /// `CesiumMath.LUNAR_RADIUS`（1737400.0 米）的球。（注意：这不是 IAU 2000
    /// 的三轴月球椭球；CesiumJS 将月球建模为一个球体。）
    pub const MOON: Self = Self::from_radii_unchecked(LUNAR_RADIUS, LUNAR_RADIUS, LUNAR_RADIUS);

    /// 所有半径为零的退化椭球。
    /// 映射到 CesiumJS `Ellipsoid.ZERO`。
    pub const ZERO: Self = Self::from_radii_unchecked(0.0, 0.0, 0.0);

    /// 由半径创建 Ellipsoid。用于静态初始化的 const 版本。
    pub(crate) const fn from_radii_unchecked(x: f64, y: f64, z: f64) -> Self {
        let radii_squared = DVec3::new(x * x, y * y, z * z);
        let radii_to_the_fourth = DVec3::new(x * x * x * x, y * y * y * y, z * z * z * z);
        let one_over_radii = DVec3::new(
            if x == 0.0 { 0.0 } else { 1.0 / x },
            if y == 0.0 { 0.0 } else { 1.0 / y },
            if z == 0.0 { 0.0 } else { 1.0 / z },
        );
        let one_over_radii_squared = DVec3::new(
            if x == 0.0 { 0.0 } else { 1.0 / (x * x) },
            if y == 0.0 { 0.0 } else { 1.0 / (y * y) },
            if z == 0.0 { 0.0 } else { 1.0 / (z * z) },
        );
        let minimum_radius = if x < y {
            if x < z { x } else { z }
        } else if y < z {
            y
        } else {
            z
        };
        let maximum_radius = if x > y {
            if x > z { x } else { z }
        } else if y > z {
            y
        } else {
            z
        };
        let squared_x_over_squared_z = if radii_squared.z != 0.0 {
            radii_squared.x / radii_squared.z
        } else {
            0.0
        };

        Self {
            radii: DVec3::new(x, y, z),
            radii_squared,
            radii_to_the_fourth,
            one_over_radii,
            one_over_radii_squared,
            minimum_radius,
            maximum_radius,
            center_tolerance_squared: EPSILON1,
            squared_x_over_squared_z,
        }
    }

    /// 由半径值创建一个新 Ellipsoid。
    /// 映射到 `new Ellipsoid(x, y, z)`
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        assert!(x >= 0.0, "x radius must be >= 0");
        assert!(y >= 0.0, "y radius must be >= 0");
        assert!(z >= 0.0, "z radius must be >= 0");
        Self::from_radii_unchecked(x, y, z)
    }

    /// 由半径的 DVec3 创建 Ellipsoid。
    /// 映射到 `Ellipsoid.fromCartesian3`
    pub fn from_cartesian3(radii: DVec3) -> Self {
        Self::new(radii.x, radii.y, radii.z)
    }

    // --- 访问器 ---

    #[inline]
    pub fn radii(&self) -> DVec3 {
        self.radii
    }

    /// 返回三轴半径的平方。
    #[inline]
    pub fn radii_squared(&self) -> DVec3 {
        self.radii_squared
    }

    /// 返回三轴半径的四次方。
    #[inline]
    pub fn radii_to_the_fourth(&self) -> DVec3 {
        self.radii_to_the_fourth
    }

    /// 返回三轴半径的倒数。
    #[inline]
    pub fn one_over_radii(&self) -> DVec3 {
        self.one_over_radii
    }

    /// 返回三轴半径平方的倒数。
    #[inline]
    pub fn one_over_radii_squared(&self) -> DVec3 {
        self.one_over_radii_squared
    }

    /// 返回最短半径（极半径）。
    #[inline]
    pub fn minimum_radius(&self) -> f64 {
        self.minimum_radius
    }

    /// 返回最长半径（赤道半径）。
    #[inline]
    pub fn maximum_radius(&self) -> f64 {
        self.maximum_radius
    }

    /// 返回 X 半径平方与 Z 半径平方的比值（用于大地线法线计算）。
    #[inline]
    pub fn squared_x_over_squared_z(&self) -> f64 {
        self.squared_x_over_squared_z
    }

    // --- 核心算法 ---

    /// 计算椭球表面在给定测绘位置处的切平面法线。
    /// 映射到 `Ellipsoid.geodeticSurfaceNormalCartographic`
    pub fn geodetic_surface_normal_cartographic(&self, cartographic: &Cartographic) -> DVec3 {
        let longitude = cartographic.longitude;
        let latitude = cartographic.latitude;
        let cos_latitude = latitude.cos();

        let x = cos_latitude * longitude.cos();
        let y = cos_latitude * longitude.sin();
        let z = latitude.sin();

        normalize_cartesian3(DVec3::new(x, y, z))
    }

    /// 计算椭球表面在给定笛卡尔位置处的切平面法线。
    /// 映射到 `Ellipsoid.geodeticSurfaceNormal`
    /// 若位置位于椭球中心则返回 None。
    pub fn geodetic_surface_normal(&self, cartesian: DVec3) -> Option<DVec3> {
        if cartesian.abs_diff_eq(DVec3::ZERO, EPSILON14) {
            return None;
        }
        let result = cartesian * self.one_over_radii_squared;
        Some(normalize_cartesian3(result))
    }

    /// 将给定的测绘坐标转换为笛卡尔表示。
    /// 映射到 `Ellipsoid.cartographicToCartesian`
    pub fn cartographic_to_cartesian(&self, cartographic: &Cartographic) -> DVec3 {
        let n = self.geodetic_surface_normal_cartographic(cartographic);
        let k = self.radii_squared * n;
        let gamma = (n.dot(k)).sqrt();
        let k_scaled = k / gamma;
        let n_scaled = n * cartographic.height;
        k_scaled + n_scaled
    }

    /// 将测绘坐标数组转换为笛卡尔位置。
    /// 映射到 `Ellipsoid.cartographicArrayToCartesianArray`
    pub fn cartographic_array_to_cartesian_array(
        &self,
        cartographics: &[Cartographic],
    ) -> Vec<DVec3> {
        cartographics
            .iter()
            .map(|c| self.cartographic_to_cartesian(c))
            .collect()
    }

    /// 将给定的笛卡尔坐标转换为测绘表示。
    /// 映射到 `Ellipsoid.cartesianToCartographic`
    /// 若位置位于椭球中心则返回 None。
    pub fn cartesian_to_cartographic(&self, cartesian: DVec3) -> Option<Cartographic> {
        let p = self.scale_to_geodetic_surface(cartesian)?;
        let n = self.geodetic_surface_normal(p)?;
        let h = cartesian - p;

        let longitude = n.y.atan2(n.x);
        let latitude = n.z.asin();
        let height = math_utils::sign(h.dot(cartesian)) * h.length();

        Some(Cartographic {
            longitude,
            latitude,
            height,
        })
    }

    /// 将笛卡尔坐标数组转换为测绘位置。
    /// 映射到 `Ellipsoid.cartesianArrayToCartographicArray`
    pub fn cartesian_array_to_cartographic_array(
        &self,
        cartesians: &[DVec3],
    ) -> Vec<Option<Cartographic>> {
        cartesians
            .iter()
            .map(|c| self.cartesian_to_cartographic(*c))
            .collect()
    }

    /// 沿大地表面法线缩放给定的笛卡尔位置，使其落在本椭球表面上。
    /// 映射到 `Ellipsoid.scaleToGeodeticSurface`。
    /// 若位置位于椭球中心则返回 None。
    pub fn scale_to_geodetic_surface(&self, cartesian: DVec3) -> Option<DVec3> {
        scale_to_geodetic_surface(
            cartesian,
            self.one_over_radii,
            self.one_over_radii_squared,
            self.center_tolerance_squared,
        )
    }

    /// 沿大地表面法线缩放给定的笛卡尔位置，使其落在本椭球表面上。
    /// 若位置位于中心，则返回中心。
    /// 映射到 `Ellipsoid.scaleToGeocentricSurface`
    pub fn scale_to_geocentric_surface(&self, cartesian: DVec3) -> Option<DVec3> {
        let position_x = cartesian.x;
        let position_y = cartesian.y;
        let position_z = cartesian.z;

        let beta = 1.0
            / ((position_x * position_x) * self.one_over_radii_squared.x
                + (position_y * position_y) * self.one_over_radii_squared.y
                + (position_z * position_z) * self.one_over_radii_squared.z)
                .sqrt();

        // beta 为将点缩放到椭球面上所需的比例因子；非有限（如原点位于中心）则无法缩放。
        if !beta.is_finite() {
            return None;
        }

        Some(cartesian * beta)
    }

    /// 计算射线与椭球的相交。
    /// 返回沿射线的参数距离区间 (start, stop)，或 None。
    /// 与 `IntersectionTests.rayEllipsoid` 语义一致。
    pub fn intersection(&self, ray_origin: DVec3, ray_direction: DVec3) -> Option<(f64, f64)> {
        // 将射线变换到单位球空间：q 为缩放后的起点，w 为缩放后的方向。
        let q = ray_origin * self.one_over_radii;
        let w = ray_direction * self.one_over_radii;

        let q2 = q.length_squared();
        let qw = q.dot(w);

        if q2 > 1.0 {
            // 在椭球外部。
            if qw >= 0.0 {
                // 朝外看或相切（0 个交点）。
                return None;
            }

            // qw < 0.0
            let qw2 = qw * qw;
            let difference = q2 - 1.0; // 取正值。
            let w2 = w.length_squared();
            let product = w2 * difference;

            if qw2 < product {
                // 虚根（0 个交点）。
                return None;
            } else if qw2 > product {
                // 相异根（2 个交点）。
                let discriminant = qw * qw - product;
                let temp = -qw + discriminant.sqrt(); // 避免相消。
                let root0 = temp / w2;
                let root1 = difference / temp;
                if root0 < root1 {
                    Some((root0, root1))
                } else {
                    Some((root1, root0))
                }
            } else {
                // qw2 == product。重根（2 个交点）。
                let root = (difference / w2).sqrt();
                Some((root, root))
            }
        } else if q2 < 1.0 {
            // 在椭球内部（2 个交点）。
            let difference = q2 - 1.0; // 取负值。
            let w2 = w.length_squared();
            let product = w2 * difference; // 取负值。

            let discriminant = qw * qw - product;
            let temp = -qw + discriminant.sqrt(); // 取正值。
            Some((0.0, temp / w2))
        } else {
            // q2 == 1.0。在椭球上。
            if qw < 0.0 {
                // 朝内看。
                let w2 = w.length_squared();
                Some((0.0, -qw / w2))
            } else {
                // qw >= 0.0。朝外看或相切。
                None
            }
        }
    }

    /// 通过将各分量乘以 `oneOverRadii`，把笛卡尔 X、Y、Z 位置变换到椭球缩放空间。
    /// 映射到 `Ellipsoid.transformPositionToScaledSpace`
    pub fn transform_position_to_scaled_space(&self, position: DVec3) -> DVec3 {
        position * self.one_over_radii
    }

    /// 通过将各分量乘以 `radii`，把笛卡尔 X、Y、Z 位置从椭球缩放空间变换回来。
    /// 映射到 `Ellipsoid.transformPositionFromScaledSpace`
    pub fn transform_position_from_scaled_space(&self, position: DVec3) -> DVec3 {
        position * self.radii
    }

    /// 计算从本椭球中心指向给定笛卡尔位置的单位向量（即地心表面法线）。
    /// 映射到 `Ellipsoid.geocentricSurfaceNormal`（= `Cartesian3.normalize`）
    pub fn geocentric_surface_normal(&self, cartesian: DVec3) -> DVec3 {
        normalize_cartesian3(cartesian)
    }

    /// 计算表面法线与 z 轴的交点。
    /// 映射到 `Ellipsoid.getSurfaceNormalIntersectionWithZAxis`
    ///
    /// 若交点位于椭球（按 `buffer` 收缩后）之外，则返回 `None`。
    ///
    /// # Panic
    /// 若该椭球不是旋转椭球（radii.x != radii.y）或 radii.z 不大于 0，则 Panic。
    pub fn get_surface_normal_intersection_with_z_axis(
        &self,
        position: DVec3,
        buffer: Option<f64>,
    ) -> Option<DVec3> {
        assert!(
            math_utils::equals_epsilon(self.radii.x, self.radii.y, EPSILON15, 0.0),
            "Ellipsoid must be an ellipsoid of revolution (radii.x == radii.y)"
        );
        assert!(self.radii.z > 0.0, "Ellipsoid.radii.z must be greater than 0");

        let buffer = buffer.unwrap_or(0.0);
        let squared_x_over_squared_z = self.squared_x_over_squared_z;

        let z = position.z * (1.0 - squared_x_over_squared_z);

        if z.abs() >= self.radii.z - buffer {
            return None;
        }

        Some(DVec3::new(0.0, 0.0, z))
    }

    /// 计算表面给定位置处的椭球曲率。
    /// 映射到 `Ellipsoid.getLocalCurvature`
    /// 以 `DVec2` 返回局部曲率 (east, north)，若表面法线/z 轴交点在椭球之外
    /// 则返回 `None`。
    pub fn get_local_curvature(&self, surface_position: DVec3) -> Option<DVec2> {
        let prime_vertical_endpoint = self
            .get_surface_normal_intersection_with_z_axis(surface_position, Some(0.0))?;
        let prime_vertical_radius = surface_position.distance(prime_vertical_endpoint);
        // 子午圈半径 = (1 - e^2) * primeVerticalRadius^3 / a^2
        // 其中 1 - e^2 = b^2 / a^2，因此子午圈 = b^2 * primeVerticalRadius^3 / a^4
        //   = (b * primeVerticalRadius / a^2)^2 * primeVertical
        let radius_ratio =
            (self.minimum_radius * prime_vertical_radius) / self.maximum_radius.powi(2);
        let meridional_radius = prime_vertical_radius * radius_ratio.powi(2);

        Some(DVec2::new(
            1.0 / prime_vertical_radius,
            1.0 / meridional_radius,
        ))
    }

    /// 使用 Gauss-Legendre 10 阶求积，近似计算本椭球表面上某矩形的面积。
    /// 映射到 `Ellipsoid.surfaceArea`
    pub fn surface_area(&self, rectangle: &Rectangle) -> f64 {
        let min_longitude = rectangle.west;
        let mut max_longitude = rectangle.east;
        let min_latitude = rectangle.south;
        let max_latitude = rectangle.north;

        while max_longitude < min_longitude {
            max_longitude += TWO_PI;
        }

        let a2 = self.radii_squared.x;
        let b2 = self.radii_squared.y;
        let c2 = self.radii_squared.z;
        let a2b2 = a2 * b2;

        gauss_legendre_quadrature(min_latitude, max_latitude, |lat| {
            // phi 表示从北极量起的角度
            // sin(phi) = sin(pi / 2 - lat) = cos(lat)，cos(phi) 类似
            let sin_phi = lat.cos();
            let cos_phi = lat.sin();
            lat.cos()
                * gauss_legendre_quadrature(min_longitude, max_longitude, |lon| {
                    let cos_theta = lon.cos();
                    let sin_theta = lon.sin();
                    (a2b2 * cos_phi * cos_phi
                        + c2
                            * (b2 * cos_theta * cos_theta + a2 * sin_theta * sin_theta)
                            * sin_phi
                            * sin_phi)
                        .sqrt()
                })
        })
    }

    /// 将该对象打包进数组时所使用的元素个数。
    /// 映射到 `Ellipsoid.packedLength`
    pub const PACKED_LENGTH: usize = 3;

    /// 将给定的实例存入给定的数组。
    /// 映射到 `Ellipsoid.pack`
    pub fn pack(&self, array: &mut [f64], starting_index: usize) {
        array[starting_index] = self.radii.x;
        array[starting_index + 1] = self.radii.y;
        array[starting_index + 2] = self.radii.z;
    }

    /// 从打包数组中取回一个实例。
    /// 映射到 `Ellipsoid.unpack`
    pub fn unpack(array: &[f64], starting_index: usize) -> Self {
        Self::new(
            array[starting_index],
            array[starting_index + 1],
            array[starting_index + 2],
        )
    }
}

impl std::fmt::Display for Ellipsoid {
    /// 格式化为 `(radii.x, radii.y, radii.z)`。
    /// 映射到 `Ellipsoid.toString`
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.radii.x, self.radii.y, self.radii.z)
    }
}

/// 沿大地表面法线缩放给定的笛卡尔位置，使其落在椭球表面上。
/// 使用牛顿迭代法求解。
fn scale_to_geodetic_surface(
    cartesian: DVec3,
    one_over_radii: DVec3,
    one_over_radii_squared: DVec3,
    center_tolerance_squared: f64,
) -> Option<DVec3> {
    let position_x = cartesian.x;
    let position_y = cartesian.y;
    let position_z = cartesian.z;

    let one_over_radii_x = one_over_radii.x;
    let one_over_radii_y = one_over_radii.y;
    let one_over_radii_z = one_over_radii.z;

    let x2 = position_x * position_x * one_over_radii_x * one_over_radii_x;
    let y2 = position_y * position_y * one_over_radii_y * one_over_radii_y;
    let z2 = position_z * position_z * one_over_radii_z * one_over_radii_z;

    // 计算椭球范数的平方。
    let squared_norm = x2 + y2 + z2;
    let ratio = (1.0 / squared_norm).sqrt();

    // 作为初始近似，假定径向交点即为投影点。
    let intersection = cartesian * ratio;

    // 若位置靠近中心，迭代将不会收敛。
    if squared_norm < center_tolerance_squared {
        if !ratio.is_finite() {
            return None;
        }
        return Some(intersection);
    }

    let one_over_radii_squared_x = one_over_radii_squared.x;
    let one_over_radii_squared_y = one_over_radii_squared.y;
    let one_over_radii_squared_z = one_over_radii_squared.z;

    // 用交点处的梯度代替真正的单位法线。
    let gradient = DVec3::new(
        intersection.x * one_over_radii_squared_x * 2.0,
        intersection.y * one_over_radii_squared_y * 2.0,
        intersection.z * one_over_radii_squared_z * 2.0,
    );

    // 计算法线向量乘子 lambda 的初始猜测值。
    let mut lambda =
        ((1.0 - ratio) * cartesian.length()) / (0.5 * gradient.length());
    let mut correction: f64 = 0.0;

    let mut x_multiplier: f64;
    let mut y_multiplier: f64;
    let mut z_multiplier: f64;

    loop {
        lambda -= correction;

        x_multiplier = 1.0 / (1.0 + lambda * one_over_radii_squared_x);
        y_multiplier = 1.0 / (1.0 + lambda * one_over_radii_squared_y);
        z_multiplier = 1.0 / (1.0 + lambda * one_over_radii_squared_z);

        let x_multiplier2 = x_multiplier * x_multiplier;
        let y_multiplier2 = y_multiplier * y_multiplier;
        let z_multiplier2 = z_multiplier * z_multiplier;

        let x_multiplier3 = x_multiplier2 * x_multiplier;
        let y_multiplier3 = y_multiplier2 * y_multiplier;
        let z_multiplier3 = z_multiplier2 * z_multiplier;

        let func =
            x2 * x_multiplier2 + y2 * y_multiplier2 + z2 * z_multiplier2 - 1.0;

        // 用于速度和加速度计算的"分母"
        let denominator = x2 * x_multiplier3 * one_over_radii_squared_x
            + y2 * y_multiplier3 * one_over_radii_squared_y
            + z2 * z_multiplier3 * one_over_radii_squared_z;

        let derivative = -2.0 * denominator;
        correction = func / derivative;

        if func.abs() <= EPSILON12 {
            break;
        }
    }

    Some(DVec3::new(
        position_x * x_multiplier,
        position_y * y_multiplier,
        position_z * z_multiplier,
    ))
}

/// Gauss-Legendre 10 阶求积的横坐标（最后一个元素未使用，保留以对应
/// CesiumJS 的表格布局）。
const GAUSS_LEGENDRE_ABSCISSAS: [f64; 6] = [
    0.14887433898163,
    0.43339539412925,
    0.67940956829902,
    0.86506336668898,
    0.97390652851717,
    0.0,
];

/// Gauss-Legendre 10 阶求积的权重。
const GAUSS_LEGENDRE_WEIGHTS: [f64; 6] = [
    0.29552422471475,
    0.26926671930999,
    0.21908636251598,
    0.14945134915058,
    0.066671344308684,
    0.0,
];

/// 计算给定定积分的 10 阶 Gauss-Legendre 求积。
/// 为 `gaussLegendreQuadrature` 的私有辅助实现。
fn gauss_legendre_quadrature<F: Fn(f64) -> f64>(a: f64, b: f64, func: F) -> f64 {
    // 由于五个权重相加为一（十个权重相加为二），此处的范围是常规范围的一半。
    // 横坐标的值会乘以二以补偿这一点。
    let x_mean = 0.5 * (b + a);
    let x_range = 0.5 * (b - a);

    let mut sum = 0.0;
    // 5 点高斯-勒让德求积：在每个采样点将节点向两侧各对称取一个，累加权重乘函数值。
    for i in 0..5 {
        let dx = x_range * GAUSS_LEGENDRE_ABSCISSAS[i];
        sum += GAUSS_LEGENDRE_WEIGHTS[i] * (func(x_mean + dx) + func(x_mean - dx));
    }

    // 将和按 x 的范围缩放。
    sum * x_range
}

impl Default for Ellipsoid {
    /// 默认椭球为 WGS84。
    fn default() -> Self {
        Self::WGS84
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::excessive_precision)]
    fn test_wgs84_radii() {
        let ell = Ellipsoid::WGS84;
        assert_eq!(ell.radii().x, 6378137.0);
        assert_eq!(ell.radii().y, 6378137.0);
        assert!((ell.radii().z - 6356752.3142451793).abs() < 1e-10);
    }

    #[test]
    fn test_geodetic_surface_normal_cartographic() {
        let ell = Ellipsoid::WGS84;
        // 在赤道、本初子午线处：法线应为 (1, 0, 0)
        let c = Cartographic::from_radians(0.0, 0.0, 0.0);
        let n = ell.geodetic_surface_normal_cartographic(&c);
        assert!((n.x - 1.0).abs() < 1e-15);
        assert!(n.y.abs() < 1e-15);
        assert!(n.z.abs() < 1e-15);

        // 在北极处：法线应为 (0, 0, 1)
        let c = Cartographic::from_radians(0.0, std::f64::consts::PI / 2.0, 0.0);
        let n = ell.geodetic_surface_normal_cartographic(&c);
        assert!(n.x.abs() < 1e-15);
        assert!(n.y.abs() < 1e-15);
        assert!((n.z - 1.0).abs() < 1e-15);
    }

    #[test]
    fn test_cartographic_to_cartesian_roundtrip() {
        let ell = Ellipsoid::WGS84;
        let original = Cartographic::from_degrees(21.0, 78.0, 5000.0);
        let cartesian = ell.cartographic_to_cartesian(&original);
        let result = ell.cartesian_to_cartographic(cartesian).unwrap();

        assert!(
            (result.longitude - original.longitude).abs() < 1e-10,
            "longitude diff: {}",
            (result.longitude - original.longitude).abs()
        );
        assert!(
            (result.latitude - original.latitude).abs() < 1e-10,
            "latitude diff: {}",
            (result.latitude - original.latitude).abs()
        );
        assert!(
            (result.height - original.height).abs() < 1e-6,
            "height diff: {}",
            (result.height - original.height).abs()
        );
    }

    #[test]
    fn test_cartographic_to_cartesian_equator() {
        let ell = Ellipsoid::WGS84;
        let c = Cartographic::from_radians(0.0, 0.0, 0.0);
        let cartesian = ell.cartographic_to_cartesian(&c);
        // 在赤道、本初子午线、高度 0 处：应为 (6378137, 0, 0)
        assert!((cartesian.x - 6378137.0).abs() < 1e-6);
        assert!(cartesian.y.abs() < 1e-6);
        assert!(cartesian.z.abs() < 1e-6);
    }

    #[test]
    fn test_scale_to_geodetic_surface() {
        let ell = Ellipsoid::WGS84;
        // 表面上方的一点应被缩小到表面上
        let point = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
        let surface = ell.scale_to_geodetic_surface(point).unwrap();
        assert!((surface.x - 6378137.0).abs() < 1e-6);
        assert!(surface.y.abs() < 1e-6);
        assert!(surface.z.abs() < 1e-6);
    }

    #[test]
    fn test_scale_to_geodetic_surface_center() {
        let ell = Ellipsoid::WGS84;
        // 在中心处，应返回 None 或中心本身
        let result = ell.scale_to_geodetic_surface(DVec3::ZERO);
        // 中心在容差内，比值为无穷 → None
        assert!(result.is_none());
    }

    #[test]
    fn test_geodetic_surface_normal_at_center() {
        let ell = Ellipsoid::WGS84;
        assert!(ell.geodetic_surface_normal(DVec3::ZERO).is_none());
    }

    #[test]
    fn test_intersection() {
        let ell = Ellipsoid::WGS84;
        // 沿 x 轴从外部指向中心的射线
        let origin = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
        let direction = DVec3::new(-1.0, 0.0, 0.0);
        let (t0, t1) = ell.intersection(origin, direction).unwrap();
        // t0 应命中近侧表面，t1 命中远侧表面
        let hit0 = origin + direction * t0;
        let hit1 = origin + direction * t1;
        assert!((hit0.x - 6378137.0).abs() < 1e-3);
        assert!((hit1.x + 6378137.0).abs() < 1e-3);
    }

    #[test]
    fn test_multiple_roundtrip_positions() {
        let ell = Ellipsoid::WGS84;
        let test_cases = vec![
            Cartographic::from_degrees(0.0, 0.0, 0.0),
            Cartographic::from_degrees(180.0, 0.0, 0.0),
            Cartographic::from_degrees(-122.4194, 37.7749, 100.0),
            Cartographic::from_degrees(139.6917, 35.6895, 40.0),
            Cartographic::from_degrees(0.0, 89.999, 10000.0),
            Cartographic::from_degrees(-179.999, -89.999, 0.0),
        ];

        for original in &test_cases {
            let cartesian = ell.cartographic_to_cartesian(original);
            let result = ell.cartesian_to_cartographic(cartesian).unwrap();
            assert!(
                (result.longitude - original.longitude).abs() < 1e-10,
                "Failed for {:?}: lon diff = {}",
                original,
                (result.longitude - original.longitude).abs()
            );
            assert!(
                (result.latitude - original.latitude).abs() < 1e-10,
                "Failed for {:?}: lat diff = {}",
                original,
                (result.latitude - original.latitude).abs()
            );
            assert!(
                (result.height - original.height).abs() < 1e-6,
                "Failed for {:?}: height diff = {}",
                original,
                (result.height - original.height).abs()
            );
        }
    }
}
