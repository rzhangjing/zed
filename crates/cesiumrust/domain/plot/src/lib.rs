//! cesium-plot —— 2D/3D 态势标绘覆盖层的无框架依赖核心。
//!
//! 本 crate 拥有*场景文档*（图层 → 组 → 元素）、
//! 地理几何模型（单一事实源 = 经度/纬度/高度）、纯几何
//! 算法（采样 / 细分 / 命中测试）、多维可见性评估以及 GeoJSON IO。其中没有
//! 任何部分依赖游戏引擎，因此每条规则都可确定性地进行单元测试，
//! 并在各前端间复用。
//!
//! Bevy 集成（ECS 视图同步、相机拾取、交互
//! 状态机）位于兄弟 crate `cesium-plot-bevy`，它消费本
//! crate 的类型。
//!
//! 设计文档：计划 `cesium-plot_标绘系统总体设计`。

pub mod agent;
pub mod geo;
pub mod geom;
pub mod io;
pub mod model;
pub mod ops;
pub mod visibility;
