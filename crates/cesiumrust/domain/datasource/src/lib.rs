//! cesium-datasource：Entity 与 DataSource 领域模型
//!
//! 本 crate 实现以 Entity 为中心的数据源领域层：把可随时间变化的属性
//! （常量、采样、时间区间集合）绑定到各类图形元素，并对外提供实体集合
//! 管理、图形更新器、可视化器以及 GeoJSON / CZML 解析入口。
//!
//! # 主要构件
//! - 属性系统：常量属性、采样属性、时间区间集合属性
//! - Entity：聚合点、线、面、标牌、标签、模型、椭圆等图形元素
//! - EntityCollection：实体的增删改查与变更事件
//! - 解析器：GeoJSON（RFC 7946）与 CZML（基础子集）
//!
//! # 分层约束
//! 本 crate 仅依赖领域层的数学与时间类型，不直接触及渲染后端；图形
//! 更新器与可视化器仅负责把属性求值后的结果组装为几何实例，具体的
//! 绘制提交由上层的适配器完成。

pub mod property;
pub mod property_system;
pub mod entity;
pub mod entity_collection;
pub mod geojson;
pub mod czml;
pub mod geometry_updater;
pub mod visualizer;
pub mod primitives;
pub mod datasource_display;
pub mod cluster;
pub mod animation;
pub mod property_bag;
pub mod datasource_collection;
pub mod composite_entity_collection;
pub mod velocity_vector_property;
pub mod velocity_orientation_property;
pub mod node_transformation_property;
pub mod datasource_clock;
pub mod custom_data_source;
pub mod property_array;

pub use property::{Color, Property, PositionProperty, ColorProperty, NumberProperty, BoolProperty, StringProperty};
pub use entity::{
    Entity, PointGraphics, PolylineGraphics, PolygonGraphics,
    BillboardGraphics, LabelGraphics, ModelGraphics, EllipseGraphics,
    BoxGraphics, CylinderGraphics, CorridorGraphics, RectangleGraphics,
    WallGraphics, EllipsoidGraphics, PlaneGraphics, PathGraphics,
    PolylineVolumeGraphics, HeightReference, CornerType, ClassificationType,
    ShadowMode, PlaneDef,
};
pub use entity_collection::{EntityCollection, DataSource};
pub use geojson::{parse_geojson, GeoJsonOptions, GeoJsonError};
pub use czml::{parse_czml, CzmlError};
pub use property_bag::PropertyBag;
pub use datasource_collection::DataSourceCollection;
pub use composite_entity_collection::CompositeEntityCollection;
pub use velocity_vector_property::VelocityVectorProperty;
pub use velocity_orientation_property::VelocityOrientationProperty;
pub use node_transformation_property::{NodeTransformationProperty, NodeTransformationValue};
pub use datasource_clock::DataSourceClock;
pub use custom_data_source::CustomDataSource;
pub use property_array::{PropertyArray, PositionPropertyArray};
