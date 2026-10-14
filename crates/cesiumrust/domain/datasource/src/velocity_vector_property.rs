//! VelocityVectorProperty - 从位置属性导出速度方向。
//!
//! 它用有限差分对位置属性微分，得到当前时刻的速度向量，可选归一化。

use crate::property_system::property::DynProperty;
use crate::property_system::value::PropertyValue;
use cesium_time::JulianDate;
use glam::DVec3;
use std::sync::Arc;

/// 一个通过有限差分从位置属性计算速度向量（可选归一化）的
/// 属性。
///
/// 若未绑定位置属性则无值；归一化时只保留方向单位向量。
#[derive(Clone)]
pub struct VelocityVectorProperty {
    /// 用于导出速度的位置属性。
    position: Option<Arc<dyn DynProperty>>,
    /// 是否归一化速度向量。
    normalize: bool,
}

impl VelocityVectorProperty {
    /// 创建一个无位置的新 VelocityVectorProperty。
    pub fn new() -> Self {
        Self {
            position: None,
            normalize: true,
        }
    }

    /// 使用位置属性创建 VelocityVectorProperty。
    pub fn with_position(position: Arc<dyn DynProperty>, normalize: bool) -> Self {
        Self {
            position: Some(position),
            normalize,
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

    /// 获取速度是否归一化。
    pub fn normalize(&self) -> bool {
        self.normalize
    }

    /// 设置是否归一化速度。
    pub fn set_normalize(&mut self, normalize: bool) {
        self.normalize = normalize;
    }

    /// 获取给定时间处的速度向量。
    ///
    /// 通过在 time 和 time+dt 处求值位置，
    /// 然后计算差值得到速度。若 normalize 为 true，则结果会被
    /// 归一化为单位长度。
    ///
    /// 映射到 `VelocityVectorProperty.prototype.getValue`
    pub fn get_value(&self, time: &JulianDate) -> Option<DVec3> {
        let position = self.position.as_ref()?;

        // 为有限差分使用一个较小的时间增量
        let dt = 1.0 / 60.0; // 六十分之一秒
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

        let velocity = (after - before) / dt;

        if self.normalize {
            let length = velocity.length();
            if length < 1e-15 {
                return None;
            }
            Some(velocity / length)
        } else {
            Some(velocity)
        }
    }

    /// 将此属性与另一个属性进行比较。
    pub fn equals(&self, other: &VelocityVectorProperty) -> bool {
        self.normalize == other.normalize
            && match (&self.position, &other.position) {
                (None, None) => true,
                (Some(_), None) | (None, Some(_)) => false,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            }
    }
}

impl Default for VelocityVectorProperty {
    /// 构造一个未绑定位置属性的默认速度向量属性。
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for VelocityVectorProperty {
    /// 输出是否归一化与是否已绑定位置，避免打印不可展示的闭包。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VelocityVectorProperty")
            .field("normalize", &self.normalize)
            .field("has_position", &self.position.is_some())
            .finish()
    }
}
