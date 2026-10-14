//! VelocityOrientationProperty - 从位置速度导出方向四元数。
//!
//! 它取位置属性的速度，在给定椭球上构造一个对齐参考系，输出旋转四元数。

use crate::property_system::property::DynProperty;
use crate::property_system::value::PropertyValue;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::transforms::rotation_matrix_from_position_velocity;
use cesium_time::JulianDate;
use glam::DQuat;
use std::sync::Arc;

/// 一个从位置属性的速度计算方向四元数的属性。所得的
/// 四元数表示从椭球固定参考系到速度对齐参考系的旋转。
///
/// 若未绑定位置或当前时刻无速度则无值；椭球决定局部程度参考系。
#[derive(Clone)]
pub struct VelocityOrientationProperty {
    /// 用于导出速度的位置属性。
    position: Option<Arc<dyn DynProperty>>,
    /// 用于计算旋转的椭球。
    ellipsoid: Ellipsoid,
}

impl VelocityOrientationProperty {
    /// 创建一个无位置的新 VelocityOrientationProperty。
    pub fn new() -> Self {
        Self {
            position: None,
            ellipsoid: Ellipsoid::WGS84,
        }
    }

    /// 使用位置属性和椭球创建 VelocityOrientationProperty。
    pub fn with_position(position: Arc<dyn DynProperty>, ellipsoid: Ellipsoid) -> Self {
        Self {
            position: Some(position),
            ellipsoid,
        }
    }

    /// 获取此属性是否为常量。
    pub fn is_constant(&self) -> bool {
        match &self.position {
            None => true,
            Some(p) => p.is_constant(),
        }
    }

    /// 获取位置属性。
    pub fn position(&self) -> Option<&Arc<dyn DynProperty>> {
        self.position.as_ref()
    }

    /// 设置位置属性。
    pub fn set_position(&mut self, position: Option<Arc<dyn DynProperty>>) {
        self.position = position;
    }

    /// 获取椭球。
    pub fn ellipsoid(&self) -> &Ellipsoid {
        &self.ellipsoid
    }

    /// 设置椭球。
    pub fn set_ellipsoid(&mut self, ellipsoid: Ellipsoid) {
        self.ellipsoid = ellipsoid;
    }

    /// 获取给定时间处的方向四元数。
    ///
    /// 通过对位置属性进行有限差分计算速度，
    /// 然后使用 `rotationMatrixFromPositionVelocity` 获取旋转
    /// 矩阵，再将其转换为四元数。
    ///
    /// 映射到 `VelocityOrientationProperty.prototype.getValue`
    pub fn get_value(&self, time: &JulianDate) -> Option<DQuat> {
        let position = self.position.as_ref()?;

        // 为有限差分使用一个较小的时间增量
        let dt = 1.0 / 60.0;
        let time_after = time.add_seconds(dt);

        let pos_before = position.get_value(time);
        let pos_after = position.get_value(&time_after);

        let before = match pos_before {
            PropertyValue::Cartesian3(v) => v,
            _ => return None,
        };
        let after = match pos_after {
            PropertyValue::Cartesian3(v) => v,
            _ => return None,
        };

        let velocity = after - before;
        if velocity.length() < 1e-15 {
            return None;
        }
        let velocity_normalized = velocity.normalize();

        let matrix =
            rotation_matrix_from_position_velocity(before, velocity_normalized, &self.ellipsoid);

        Some(DQuat::from_mat3(&matrix))
    }

    /// 将此属性与另一个属性进行比较。
    pub fn equals(&self, other: &VelocityOrientationProperty) -> bool {
        self.ellipsoid == other.ellipsoid
            && match (&self.position, &other.position) {
                (None, None) => true,
                (Some(_), None) | (None, Some(_)) => false,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            }
    }
}

impl Default for VelocityOrientationProperty {
    /// 构造一个使用默认椭球且未绑定位置属性的方向属性。
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for VelocityOrientationProperty {
    /// 输出是否已绑定位置与所用椭球，避免打印不可展示的闭包。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VelocityOrientationProperty")
            .field("has_position", &self.position.is_some())
            .field("ellipsoid", &self.ellipsoid)
            .finish()
    }
}
