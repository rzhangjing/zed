//! 可序列化的场景文档模型（计划 §5）。
//!
//! `Document`（一棵图层 → 组 → 元素的树）加上每个
//! 节点由其构建的各值类型。这里的一切都是普通数据（`Serialize`/`Deserialize`）
//! ，无引擎依赖，因此可在无头环境下进行 diff 与单元测试。

// 子模块：文档树各层与各值类型分文件建模，均无引擎依赖。
pub mod document;
pub mod element;
pub mod filters;
pub mod geometry;
pub mod group;
pub mod ids;
pub mod layer;
pub mod node;
pub mod pick;
pub mod style;
pub mod view;

// 平铺再导出：让下游只需 `use crate::model::X` 即可拿到常用类型。
pub use document::{Document, NewElement};
pub use element::{Element, ElementFlags, ScaleVisibility};
pub use filters::Filters;
pub use geometry::{
    Arc3, Circle, Composite, Ellipse, Geometry, GeometryKind, IconGeometry, LabelAnchor,
    LabelGeometry, Path, PathSegment, Polyline, Polygon, Rectangle, Ring, SymbolKind,
};
// 拾取常量与各向排名：用于从多个候选命中中挑出最优者。
pub use group::{Group, GroupTransform};
pub use ids::{ElementId, GroupId, LayerId};
pub use layer::Layer;
pub use node::Node;
pub use pick::{pick_best, Part, PickHit, RANK_LINE, RANK_MARKER, RANK_POLY_BODY, RANK_POLY_EDGE};
pub use style::{HeightReference, IconStyle, Outline, Rgba, Style, TextStyle, TRANSPARENT, WHITE};
pub use view::{ViewContext, ViewMode};
