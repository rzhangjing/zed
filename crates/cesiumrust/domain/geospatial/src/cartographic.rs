//! Cartographic - 由经度、纬度和高度定义的位置。

use crate::ellipsoid::Ellipsoid;
use crate::math_utils;
use glam::DVec3;
use serde::{Deserialize, Serialize};

/// 由经度、纬度以及椭球上方高度定义的位置。
/// 经度和纬度以弧度为单位。高度以米为单位。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cartographic {
    /// 弧度为单位的经度。
    pub longitude: f64,
    /// 弧度为单位的纬度。
    pub latitude: f64,
    /// 椭球上方以米为单位的高度。
    pub height: f64,
}

impl Cartographic {
    /// 从弧度创建新的 Cartographic。
    /// 映射到 Cartographic.fromRadians
    #[inline]
    pub fn from_radians(longitude: f64, latitude: f64, height: f64) -> Self {
        Self {
            longitude,
            latitude,
            height,
        }
    }

    /// 从角度创建新的 Cartographic。
    /// 映射到 Cartographic.fromDegrees
    #[inline]
    pub fn from_degrees(longitude: f64, latitude: f64, height: f64) -> Self {
        Self {
            longitude: math_utils::to_radians(longitude),
            latitude: math_utils::to_radians(latitude),
            height,
        }
    }

    /// 在原点 (0, 0, 0) 创建一个 Cartographic。
    pub const ZERO: Self = Self {
        longitude: 0.0,
        latitude: 0.0,
        height: 0.0,
    };

    /// 判断此 Cartographic 是否在 epsilon 范围内等于另一个。
    pub fn equals_epsilon(&self, other: &Self, epsilon: f64) -> bool {
        (self.longitude - other.longitude).abs() <= epsilon
            && (self.latitude - other.latitude).abs() <= epsilon
            && (self.height - other.height).abs() <= epsilon
    }

    /// 从 Cartographic 输入创建新的 Cartesian3 实例。
    /// 映射到 `Cartographic.toCartesian`。输入值以弧度为单位。
    /// 椭球显式传入（Rust 没有 `Ellipsoid.default` 全局变量）。
    pub fn to_cartesian(cartographic: &Cartographic, ellipsoid: &Ellipsoid) -> DVec3 {
        ellipsoid.cartographic_to_cartesian(cartographic)
    }

    /// 从 Cartesian 位置创建新的 Cartographic 实例。
    /// 映射到 `Cartographic.fromCartesian`。所得值以弧度为单位。
    /// 椭球显式传入（Rust 没有 `Ellipsoid.default` 全局变量）。
    /// 若 cartesian 位于椭球中心则返回 None。
    pub fn from_cartesian(cartesian: DVec3, ellipsoid: &Ellipsoid) -> Option<Cartographic> {
        ellipsoid.cartesian_to_cartographic(cartesian)
    }
}

impl std::fmt::Display for Cartographic {
    /// 映射到 `Cartographic.toString` → `(longitude, latitude, height)`。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.longitude, self.latitude, self.height)
    }
}

impl Default for Cartographic {
    /// 默认构造等价于 `Cartographic::ZERO`（经度、纬度、高度均为 0）。
    fn default() -> Self {
        Self::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_from_degrees() {
        let c = Cartographic::from_degrees(180.0, 90.0, 1000.0);
        assert!((c.longitude - PI).abs() < 1e-15);
        assert!((c.latitude - PI / 2.0).abs() < 1e-15);
        assert_eq!(c.height, 1000.0);
    }

    #[test]
    fn test_from_radians() {
        let c = Cartographic::from_radians(1.0, 0.5, 200.0);
        assert_eq!(c.longitude, 1.0);
        assert_eq!(c.latitude, 0.5);
        assert_eq!(c.height, 200.0);
    }

    #[test]
    fn test_equals_epsilon() {
        let a = Cartographic::from_radians(1.0, 0.5, 100.0);
        let b = Cartographic::from_radians(1.0 + 1e-12, 0.5, 100.0);
        assert!(a.equals_epsilon(&b, 1e-10));
        assert!(!a.equals_epsilon(&b, 1e-14));
    }
}
