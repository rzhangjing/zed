//! 球极投影坐标。

use crate::ellipsoid::Ellipsoid;
use crate::ellipsoid_tangent_plane::EllipsoidTangentPlane;
use crate::math_utils;
use crate::ray::{ray_plane, Ray};
use glam::{DVec2, DVec3};

/// 半径为 (0.5, 0.5, 0.5) 的椭球。
pub const HALF_UNIT_SPHERE: Ellipsoid = Ellipsoid::from_radii_unchecked(0.5, 0.5, 0.5);

/// 半单位球上的北极。
pub const NORTH_POLE: DVec3 = DVec3::new(0.0, 0.0, 0.5);
/// 半单位球上的南极。
pub const SOUTH_POLE: DVec3 = DVec3::new(0.0, 0.0, -0.5);

/// 标识所使用的极点切平面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoleTangentPlane {
    /// 北极处的切平面。
    North,
    /// 南极处的切平面。
    South,
}

/// 表示球极坐标中的一个点，通过将笛卡尔坐标从一个极点投影到
/// 另一个极点处的切平面而得到。
///
/// 映射到 CesiumJS `Stereographic`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stereographic {
    /// 球极 2D 坐标。
    pub position: DVec2,
    /// 所使用的极点切平面。
    pub tangent_plane: PoleTangentPlane,
}

impl Default for Stereographic {
    /// 默认构造：位置为原点、使用北极切平面。
    fn default() -> Self {
        Self {
            position: DVec2::ZERO,
            tangent_plane: PoleTangentPlane::North,
        }
    }
}

impl Stereographic {
    /// 以给定的位置和切平面创建一个新的 Stereographic。
    pub fn new(position: DVec2, tangent_plane: PoleTangentPlane) -> Self {
        Self {
            position,
            tangent_plane,
        }
    }

    /// 获取 x 坐标。
    #[inline]
    pub fn x(&self) -> f64 {
        self.position.x
    }

    /// 获取 y 坐标。
    #[inline]
    pub fn y(&self) -> f64 {
        self.position.y
    }

    /// 获取椭球（始终为半单位球）。
    #[inline]
    pub fn ellipsoid(&self) -> &'static Ellipsoid {
        &HALF_UNIT_SPHERE
    }

    /// 计算共形纬度（将椭球纬度投影到任意球面上）。
    pub fn conformal_latitude(&self) -> f64 {
        // r 为投影点到原点的平面距离，d 为球直径；由反投影几何恢复共形纬度。
        let r = self.position.length();
        let d = 2.0 * HALF_UNIT_SPHERE.maximum_radius();
        let sign = match self.tangent_plane {
            PoleTangentPlane::North => 1.0,
            PoleTangentPlane::South => -1.0,
        };
        sign * (math_utils::PI_OVER_TWO - 2.0 * r.atan2(d))
    }

    /// 计算经度。
    pub fn longitude(&self) -> f64 {
        let mut longitude = math_utils::PI_OVER_TWO + self.position.y.atan2(self.position.x);
        if longitude > std::f64::consts::PI {
            longitude -= math_utils::TWO_PI;
        }
        longitude
    }

    /// 计算在给定椭球上的大地纬度。
    ///
    /// 映射到 `Stereographic.prototype.getLatitude`
    pub fn get_latitude(&self, ellipsoid: &Ellipsoid) -> f64 {
        let conformal_lat = self.conformal_latitude();
        let longitude = self.longitude();

        // 将半单位球上的共形纬度转换为笛卡尔坐标
        let cos_lat = conformal_lat.cos();
        let cartesian = DVec3::new(
            HALF_UNIT_SPHERE.maximum_radius() * cos_lat * longitude.cos(),
            HALF_UNIT_SPHERE.maximum_radius() * cos_lat * longitude.sin(),
            HALF_UNIT_SPHERE.maximum_radius() * conformal_lat.sin(),
        );

        // 再将该笛卡尔坐标转换为目标椭球上的测绘坐标
        ellipsoid
            .cartesian_to_cartographic(cartesian)
            .map(|c| c.latitude)
            .unwrap_or(conformal_lat)
    }

    /// 计算给定 3D 位置到 2D 极点平面的投影。
    ///
    /// 映射到 `Stereographic.fromCartesian`
    pub fn from_cartesian(cartesian: DVec3) -> Self {
        let sign = if cartesian.z >= 0.0 { 1.0 } else { -1.0 };

        let (tangent_plane_id, origin) = if sign < 0.0 {
            (PoleTangentPlane::South, NORTH_POLE)
        } else {
            (PoleTangentPlane::North, SOUTH_POLE)
        };

        let tangent_plane = Self::get_tangent_plane(tangent_plane_id);

        // 从地心表面点射向对面极点的射线
        let surface_point = HALF_UNIT_SPHERE
            .scale_to_geocentric_surface(cartesian)
            .unwrap_or(cartesian);
        let direction = (surface_point - origin).normalize();
        let ray = Ray {
            origin: surface_point,
            direction,
        };

        let intersection_point = ray_plane(&ray, tangent_plane.plane())
            .expect("ray must intersect tangent plane");

        let v = intersection_point - origin;
        let x = tangent_plane.x_axis().dot(v);
        let y = sign * tangent_plane.y_axis().dot(v);

        Self {
            position: DVec2::new(x, y),
            tangent_plane: tangent_plane_id,
        }
    }

    /// 计算一组 3D 位置的投影。
    ///
    /// 映射到 `Stereographic.fromCartesianArray`
    pub fn from_cartesian_array(cartesians: &[DVec3]) -> Vec<Self> {
        cartesians.iter().map(|&c| Self::from_cartesian(c)).collect()
    }

    /// 获取给定极点的切平面。
    fn get_tangent_plane(pole: PoleTangentPlane) -> EllipsoidTangentPlane {
        match pole {
            PoleTangentPlane::North => EllipsoidTangentPlane::new(NORTH_POLE, &HALF_UNIT_SPHERE),
            PoleTangentPlane::South => EllipsoidTangentPlane::new(SOUTH_POLE, &HALF_UNIT_SPHERE),
        }
    }
}
