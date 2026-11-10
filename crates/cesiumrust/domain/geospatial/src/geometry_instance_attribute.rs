//! GeometryInstanceAttribute 家族（颜色/显示/距离显示条件等逐实例属性）。

use crate::attribute_compression::ComponentDatatype;
use crate::color::Color;

/// 逐实例几何属性的值与类型信息。
/// 映射到 CesiumJS `GeometryInstanceAttribute`
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryInstanceAttribute {
    /// 属性中每个分量的数据类型。
    pub component_datatype: ComponentDatatype,
    /// 一个介于 1 和 4 之间的数，定义属性中的分量数量。
    pub components_per_attribute: u32,
    /// 当为 true 且 componentDatatype 为整数格式时，表示各分量
    /// 应映射到区间 [0, 1]（无符号）或 [-1, 1]（有符号）。
    pub normalize: bool,
    /// 属性的值。
    pub value: Vec<f64>,
}

impl GeometryInstanceAttribute {
    /// 创建一个新的 GeometryInstanceAttribute。
    ///
    /// # Panic
    /// 若 `components_per_attribute` 不在 1 到 4 之间则 Panic。
    pub fn new(
        component_datatype: ComponentDatatype,
        components_per_attribute: u32,
        normalize: bool,
        value: Vec<f64>,
    ) -> Self {
        assert!(
            (1..=4).contains(&components_per_attribute),
            "components_per_attribute must be between 1 and 4."
        );
        Self {
            component_datatype,
            components_per_attribute,
            normalize,
            value,
        }
    }
}

/// 逐实例几何颜色的值与类型信息。
/// 映射到 CesiumJS `ColorGeometryInstanceAttribute`
#[derive(Debug, Clone, PartialEq)]
pub struct ColorGeometryInstanceAttribute {
    /// 以 [R, G, B, A] 字节存储的属性值。
    pub value: [u8; 4],
}

impl ColorGeometryInstanceAttribute {
    /// 由浮点 RGBA 分量创建一个新的 ColorGeometryInstanceAttribute。
    pub fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self {
            value: [
                Color::float_to_byte(red),
                Color::float_to_byte(green),
                Color::float_to_byte(blue),
                Color::float_to_byte(alpha),
            ],
        }
    }

    /// 每个分量的数据类型：UNSIGNED_BYTE。
    pub fn component_datatype(&self) -> ComponentDatatype {
        ComponentDatatype::UnsignedByte
    }

    /// 分量数量：4。
    pub fn components_per_attribute(&self) -> u32 {
        4
    }

    /// Normalize：true。
    pub fn normalize(&self) -> bool {
        true
    }

    /// 由一个 Color 创建一个新的 ColorGeometryInstanceAttribute。
    /// 映射到 CesiumJS `ColorGeometryInstanceAttribute.fromColor`
    pub fn from_color(color: &Color) -> Self {
        Self {
            value: color.to_bytes(),
        }
    }

    /// 将一个颜色转换为可用于赋颜色属性的字节数组。
    /// 映射到 CesiumJS `ColorGeometryInstanceAttribute.toValue`
    pub fn to_value(color: &Color) -> [u8; 4] {
        color.to_bytes()
    }

    /// 比较两个 ColorGeometryInstanceAttribute 是否相等。
    /// 映射到 CesiumJS `ColorGeometryInstanceAttribute.equals`
    pub fn equals(
        left: Option<&ColorGeometryInstanceAttribute>,
        right: Option<&ColorGeometryInstanceAttribute>,
    ) -> bool {
        match (left, right) {
            (Some(l), Some(r)) => l.value == r.value,
            _ => false,
        }
    }
}

/// 逐实例几何属性的值与类型信息，该属性决定
/// 几何实例是否显示。
/// 映射到 CesiumJS `ShowGeometryInstanceAttribute`
#[derive(Debug, Clone, PartialEq)]
pub struct ShowGeometryInstanceAttribute {
    /// 以 [show] 字节存储的属性值。
    pub value: [u8; 1],
}

impl ShowGeometryInstanceAttribute {
    /// 创建一个新的 ShowGeometryInstanceAttribute。
    pub fn new(show: bool) -> Self {
        Self {
            value: Self::to_value(show),
        }
    }

    /// 每个分量的数据类型：UNSIGNED_BYTE。
    pub fn component_datatype(&self) -> ComponentDatatype {
        ComponentDatatype::UnsignedByte
    }

    /// 分量数量：1。
    pub fn components_per_attribute(&self) -> u32 {
        1
    }

    /// Normalize：false。
    pub fn normalize(&self) -> bool {
        false
    }

    /// 将一个布尔值 show 转换为类型化数组。
    /// 映射到 CesiumJS `ShowGeometryInstanceAttribute.toValue`
    pub fn to_value(show: bool) -> [u8; 1] {
        [show as u8]
    }
}

/// 逐实例几何属性的值与类型信息，该属性决定
/// 几何实例是否具有距离显示条件。
/// 映射到 CesiumJS `DistanceDisplayConditionGeometryInstanceAttribute`
#[derive(Debug, Clone, PartialEq)]
pub struct DistanceDisplayConditionGeometryInstanceAttribute {
    /// 以 [near, far] 浮点数存储的属性值。
    pub value: [f32; 2],
}

impl DistanceDisplayConditionGeometryInstanceAttribute {
    /// 创建一个新的 DistanceDisplayConditionGeometryInstanceAttribute。
    ///
    /// # Panic
    /// 若 far <= near 则 Panic。
    pub fn new(near: f32, far: f32) -> Self {
        assert!(
            far > near,
            "far distance must be greater than near distance."
        );
        Self {
            value: [near, far],
        }
    }

    /// 以默认值创建：near=0.0，far=f32::MAX。
    pub fn default_value() -> Self {
        Self {
            value: [0.0, f32::MAX],
        }
    }

    /// 每个分量的数据类型：FLOAT。
    pub fn component_datatype(&self) -> ComponentDatatype {
        ComponentDatatype::Float
    }

    /// 分量数量：2。
    pub fn components_per_attribute(&self) -> u32 {
        2
    }

    /// Normalize：false。
    pub fn normalize(&self) -> bool {
        false
    }

    /// 由一个 DistanceDisplayCondition（near、far 对）创建。
    /// 映射到 CesiumJS `DistanceDisplayConditionGeometryInstanceAttribute.fromDistanceDisplayCondition`
    pub fn from_distance_display_condition(near: f32, far: f32) -> Self {
        assert!(
            far > near,
            "distanceDisplayCondition.far distance must be greater than distanceDisplayCondition.near distance."
        );
        Self {
            value: [near, far],
        }
    }

    /// 将一个距离显示条件转换为浮点数组。
    /// 映射到 CesiumJS `DistanceDisplayConditionGeometryInstanceAttribute.toValue`
    pub fn to_value(near: f32, far: f32) -> [f32; 2] {
        [near, far]
    }
}
