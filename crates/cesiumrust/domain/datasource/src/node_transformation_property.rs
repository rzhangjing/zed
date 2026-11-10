//! NodeTransformationProperty - 用于模型节点 TRS 变换的组合属性。
//!
//! 它把一个模型节点的平移、旋转与缩放三个子属性组合为一个整体，求值时
//! 分别取出三个分量并汇成一个 `NodeTransformationValue`；任一子属性缺失则
//! 采用对应的缺省值（零平移、单位旋转、一缩放）。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::unnecessary_map_or)]
use crate::property_system::property::DynProperty;
use crate::property_system::value::PropertyValue;
use cesium_time::JulianDate;
use glam::{DQuat, DVec3};
use std::sync::Arc;

/// NodeTransformationProperty 在给定时间处解析后的值。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeTransformationValue {
    /// 平移偏移。
    pub translation: DVec3,
    /// 旋转四元数。
    pub rotation: DQuat,
    /// 缩放因子。
    pub scale: DVec3,
}

impl Default for NodeTransformationValue {
    /// 构造恒等变换：零平移、单位旋转、一缩放。
    fn default() -> Self {
        Self {
            translation: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
            scale: DVec3::ONE,
        }
    }
}

/// 一个表示模型节点变换的属性，由平移、
/// 旋转和缩放子属性组合而成。
///
/// 三个子属性均为可选的 `DynProperty`；只要任一为非常量则整体非常量，
/// 求值时逐分量取回并组装为 TRS 值，相等判定也比较三个子属性。
#[derive(Clone)]
pub struct NodeTransformationProperty {
    /// 平移属性。
    translation: Option<Arc<dyn DynProperty>>,
    /// 旋转属性。
    rotation: Option<Arc<dyn DynProperty>>,
    /// 缩放属性。
    scale: Option<Arc<dyn DynProperty>>,
}

impl NodeTransformationProperty {
    /// 创建一个无子属性的新 NodeTransformationProperty。
    pub fn new() -> Self {
        Self {
            translation: None,
            rotation: None,
            scale: None,
        }
    }

    /// 创建一个具有常量值的 NodeTransformationProperty。
    pub fn with_values(translation: DVec3, rotation: DQuat, scale: DVec3) -> Self {
        use crate::property_system::property::ConstantProperty;
        Self {
            translation: Some(Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(
                translation,
            )))),
            rotation: Some(Arc::new(ConstantProperty::new(PropertyValue::Quaternion(
                rotation,
            )))),
            scale: Some(Arc::new(ConstantProperty::new(PropertyValue::Cartesian3(
                scale,
            )))),
        }
    }

    /// 获取此属性是否为常量（所有子属性均为常量）。
    pub fn is_constant(&self) -> bool {
        let t_const = self.translation.as_ref().map_or(true, |p| p.is_constant());
        let r_const = self.rotation.as_ref().map_or(true, |p| p.is_constant());
        let s_const = self.scale.as_ref().map_or(true, |p| p.is_constant());
        t_const && r_const && s_const
    }

    /// 获取平移属性。
    pub fn translation(&self) -> Option<&Arc<dyn DynProperty>> {
        self.translation.as_ref()
    }

    /// 设置平移属性。
    pub fn set_translation(&mut self, prop: Option<Arc<dyn DynProperty>>) {
        self.translation = prop;
    }

    /// 获取旋转属性。
    pub fn rotation(&self) -> Option<&Arc<dyn DynProperty>> {
        self.rotation.as_ref()
    }

    /// 设置旋转属性。
    pub fn set_rotation(&mut self, prop: Option<Arc<dyn DynProperty>>) {
        self.rotation = prop;
    }

    /// 获取缩放属性。
    pub fn scale(&self) -> Option<&Arc<dyn DynProperty>> {
        self.scale.as_ref()
    }

    /// 设置缩放属性。
    pub fn set_scale(&mut self, prop: Option<Arc<dyn DynProperty>>) {
        self.scale = prop;
    }

    /// 获取变换在给定时间处解析后的值。
    ///
    /// 默认值：translation=ZERO，rotation=IDENTITY，scale=ONE。
    ///
    /// 映射到 `NodeTransformationProperty.prototype.getValue`
    pub fn get_value(&self, time: &JulianDate) -> NodeTransformationValue {
        let translation = self
            .translation
            .as_ref()
            .and_then(|p| match p.get_value(time) {
                PropertyValue::Cartesian3(v) => Some(v),
                _ => None,
            })
            .unwrap_or(DVec3::ZERO);

        let rotation = self
            .rotation
            .as_ref()
            .and_then(|p| match p.get_value(time) {
                PropertyValue::Quaternion(q) => Some(q),
                _ => None,
            })
            .unwrap_or(DQuat::IDENTITY);

        let scale = self
            .scale
            .as_ref()
            .and_then(|p| match p.get_value(time) {
                PropertyValue::Cartesian3(v) => Some(v),
                _ => None,
            })
            .unwrap_or(DVec3::ONE);

        NodeTransformationValue {
            translation,
            rotation,
            scale,
        }
    }

    /// 将此属性与另一个属性进行比较。
    pub fn equals(&self, other: &NodeTransformationProperty) -> bool {
        prop_equals(&self.translation, &other.translation)
            && prop_equals(&self.rotation, &other.rotation)
            && prop_equals(&self.scale, &other.scale)
    }
}

/// 比较两个可选子属性是否相等：同为 None 相等，一方有值则逐—比较。
fn prop_equals(
    a: &Option<Arc<dyn DynProperty>>,
    b: &Option<Arc<dyn DynProperty>>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
        (Some(pa), Some(pb)) => pa.equals(pb.as_ref()),
    }
}

impl Default for NodeTransformationProperty {
    /// 缺省构造一个三子属性均为空的节点变换属性。
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for NodeTransformationProperty {
    /// 以非字段枚举的方式输出结构概要，避免 trait 对象无法直接 Debug。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeTransformationProperty")
            .field("has_translation", &self.translation.is_some())
            .field("has_rotation", &self.rotation.is_some())
            .field("has_scale", &self.scale.is_some())
            .finish()
    }
}
