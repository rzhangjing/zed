//! cesium-geospatial：椭球体、坐标、投影、瓦片划分、包围体、几何
//! 领域层 —— 纯 Rust，f64 精度，无框架依赖。
//!
//! 模块按职责聚类如下：
//! - 基础数学与坐标：`math_utils` 提供角度/精度常量，`cartographic` 为经纬度高程三元组，
//!   `cartesian2/3/4_ext` 扩展 glam 向量的几何语义。
//! - 椭球与参考系：`ellipsoid` 定义参考椭球及坐标变换，`ellipsoid_rhumb_line` 求恒向线，
//!   `ellipsoid_tangent_plane` 构造局部切平面，`spherical`/`stereographic` 为球面与投影工具。
//! - 区域与投影：`rectangle` 描述经纬矩形，`projection` 提供地理/WebMercator 投影，
//!   `tiling_scheme`/`morton_hilbert`/`s2cell` 负责瓦片划分与空间编码。
//! - 包围体与可见性：`bounding` 含 AABB/球/OBB，`occluder`/`ellipsoidal_occluder` 做地平线剔除，
//!   `frustum` 处理视锥裁剪，`ray` 提供射线求交。
//! - 几何生成管线：`geometry` 汇聚几何类型，`polyline_pipeline`/`polygon_pipeline`/`geodesic`
//!   细分大地线，`simple_polyline_geometry`/`tipsify` 生成顶点并重排序，`wireframe` 生成线框。
//! - 线性代数扩展：`matrix2/3/4_ext`、`quaternion_ext`、`transforms` 提供旋转/姿态/变换矩阵运算。
//! - 天文与时间：`simon1994_planetary_positions` 与 `iau_orientation` 计算天体位置和 IAU 姿态。
//! - 通用容器与工具：`heap`/`queue`/`double_ended_priority_queue`/`doubly_linked_list`/`associative_array`/`managed_array`
//!   为数据结构，`array_utils`/`utilities`/`uri_utils`/`color`/`polynomial` 为辅助函数，
//!   `vertical_exaggeration` 处理高程夸张，`attribute_compression`/`encoded_cartesian3` 负责属性压缩。

pub mod math_utils;
pub mod cartographic;
pub mod ellipsoid;
pub mod rectangle;
pub mod projection;
pub mod tiling_scheme;
pub mod bounding;
pub mod ray;
pub mod frustum;
pub mod transforms;
pub mod geometry;
pub mod geodesic;
pub mod polyline_pipeline;
pub mod polygon_pipeline;
pub mod ellipsoid_rhumb_line;
pub mod ellipsoid_tangent_plane;
pub mod ellipsoidal_occluder;
pub mod attribute_compression;
pub mod encoded_cartesian3;
pub mod color;
pub mod tipsify;
pub mod array_utils;
pub mod polynomial;
pub mod morton_hilbert;
pub mod occluder;
pub mod s2cell;
pub mod utilities;
pub mod spherical;
pub mod stereographic;
pub mod heap;
pub mod managed_array;
pub mod vertical_exaggeration;
pub mod queue;
pub mod wireframe;
pub mod double_ended_priority_queue;
pub mod associative_array;
pub mod doubly_linked_list;
pub mod polygon_geometry_library;
pub mod geometry_instance_attribute;
pub mod cartesian3_ext;
pub mod matrix4_ext;
pub mod quaternion_ext;
pub mod cartesian2_ext;
pub mod cartesian4_ext;
pub mod matrix3_ext;
pub mod matrix2_ext;
pub mod simon1994_planetary_positions;
pub mod iau_orientation;
pub mod uri_utils;
pub mod simple_polyline_geometry;

pub use cartographic::Cartographic;
pub use ellipsoid::Ellipsoid;
pub use rectangle::Rectangle;
pub use projection::{GeographicProjection, MapProjection, WebMercatorProjection};
pub use tiling_scheme::TilingScheme;
pub use bounding::{AxisAlignedBoundingBox, BoundingRectangle, BoundingSphere, OrientedBoundingBox};
pub use ray::{ray_ellipsoid, Intersect, Plane, Ray};
pub use frustum::{Cullable, CullingVolume, OrthographicFrustum, PerspectiveFrustum};
pub use transforms::{HeadingPitchRoll, HeadingPitchRange, TranslationRotationScale};
pub use geometry::{GeometryData, VertexFormat};
pub use attribute_compression::{ComponentDatatype, IndexDatatype};
pub use geodesic::EllipsoidGeodesic;
