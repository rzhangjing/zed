//! 完整的、与 CesiumJS 兼容的 Property 系统。
//!
//! 它围绕一个统一的属性 trait 组织约 25 个具体实现（ConstantProperty、
//! SampledProperty、TimeIntervalCollectionProperty、CompositeProperty、
//! CallbackProperty、ReferenceProperty、PositionProperty 家族、
//! MaterialProperty 家族……）。
//!
//! 与遗留的 `property` 模块（为兼容 GeoJSON/CZML 解析器
//! 而保留的简单枚举）不同，本模块实现了完整的
//! 基于 trait-object、时间动态的属性系统，具备类型擦除
//! 值、打包数组插值与参考系感知的位置。

pub mod interpolation;
pub mod material;
pub mod position;
pub mod property;
pub mod reference;
pub mod value;

pub use interpolation::{
    ExtrapolationType, HermitePolynomialApproximation, InterpolationAlgorithm,
    InterpolationAlgorithmKind, LagrangePolynomialApproximation, LinearApproximation,
};
pub use material::{
    arc_material_property_equals, CheckerboardMaterialProperty, ColorMaterialProperty,
    CompositeMaterialProperty, GridMaterialProperty, ImageMaterialProperty, MaterialProperty,
    MaterialUniforms, PolylineArrowMaterialProperty, PolylineDashMaterialProperty,
    PolylineGlowMaterialProperty, PolylineOutlineMaterialProperty, StripeMaterialProperty,
    StripeOrientation, COLOR_BLACK, COLOR_TRANSPARENT, COLOR_WHITE,
};
pub use position::{
    convert_to_reference_frame, CallbackPositionProperty, CompositePositionProperty,
    ConstantPositionProperty, PositionCallbackFn, SampledPositionProperty,
    TimeIntervalCollectionPositionProperty,
};
pub use property::{
    arc_property_equals, property_get_value_or_undefined, property_is_constant, CallbackFn,
    CallbackProperty, CompositeProperty, ConstantProperty, DynProperty, SampledProperty,
    TimeIntervalCollectionProperty,
};
pub use reference::{MapPropertyResolver, PropertyResolver, ReferenceProperty};
pub use value::{PackableType, PropertyValue, ReferenceFrame};
