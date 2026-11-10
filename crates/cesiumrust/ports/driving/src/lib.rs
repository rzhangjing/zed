//! cesium-ports-driving：驱动端口（Driving ports，外部 → 领域）
//! 用户/应用与领域交互的 trait 契约。
//!
//! 在六边形架构中，驱动端口定义外部代码
//! （UI、CLI、测试）如何与领域交互（adapter 调用这些 trait）。

use cesium_camera::Camera;
use cesium_geospatial::{Cartographic, Ellipsoid, Rectangle};
use cesium_time::JulianDate;

// ============================================================================
// Viewer/Scene 控制
// ============================================================================

/// 控制 3D 地球的主 viewer 接口。
/// 面向应用层的 viewer / widget 表面（仅 API 表层）。
pub trait ViewerApi {
    /// 获取 camera 的引用。
    fn camera(&self) -> &Camera;

    /// 获取 camera 的可变引用。
    fn camera_mut(&mut self) -> &mut Camera;

    /// 获取当前仿真时间。
    fn current_time(&self) -> JulianDate;

    /// 设置当前仿真时间。
    fn set_current_time(&mut self, time: JulianDate);

    /// 获取此 viewer 使用的椭球体（ellipsoid）。
    fn ellipsoid(&self) -> &Ellipsoid;

    /// 渲染一帧。
    fn render(&mut self);

    /// 调整视口（viewport）大小。
    fn resize(&mut self, width: u32, height: u32);
}

// ============================================================================
// Camera 控制
// ============================================================================

/// Camera 操作接口。
/// 向用户暴露的 camera 操作方法集合。
pub trait CameraControl {
    /// 由位置与朝向设置 camera 视图。
    fn set_view(
        &mut self,
        position: Cartographic,
        heading: f64,
        pitch: f64,
        roll: f64,
    );

    /// 将 camera 飞行（fly）到目标位置。
    fn fly_to(
        &mut self,
        destination: Cartographic,
        heading: Option<f64>,
        pitch: Option<f64>,
        roll: Option<f64>,
        duration_secs: f64,
    );

    /// 从给定距离注视（look at）目标位置。
    fn look_at(
        &mut self,
        target: Cartographic,
        heading: f64,
        pitch: f64,
        range: f64,
    );

    /// 按给定量放大。
    fn zoom_in(&mut self, amount: Option<f64>);

    /// 按给定量缩小。
    fn zoom_out(&mut self, amount: Option<f64>);

    /// 将 camera 重置到初始（home）视图。
    fn home(&mut self);
}

// ============================================================================
// 数据源管理
// ============================================================================

/// 数据源的唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataSourceId(pub u64);

/// 管理数据源（GeoJSON、CZML、3D Tiles 等）
pub trait DataSourceManager {
    /// 从 URL 添加数据源。
    fn add_from_url(&mut self, url: &str) -> DataSourceId;

    /// 移除数据源。
    fn remove(&mut self, id: DataSourceId) -> bool;

    /// 显示/隐藏数据源。
    fn set_visible(&mut self, id: DataSourceId, visible: bool);

    /// 获取数据源的包围矩形（bounding rectangle）。
    fn bounds(&self, id: DataSourceId) -> Option<Rectangle>;
}

// ============================================================================
// 影像图层管理
// ============================================================================

/// 影像图层的唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageryLayerId(pub u64);

/// 管理影像图层。
pub trait ImageryLayerManager {
    /// 从 provider URL 添加影像图层。
    fn add_layer(&mut self, url: &str) -> ImageryLayerId;

    /// 移除影像图层。
    fn remove_layer(&mut self, id: ImageryLayerId) -> bool;

    /// 设置图层的不透明度（0.0 - 1.0）。
    fn set_opacity(&mut self, id: ImageryLayerId, opacity: f64);

    /// 设置图层的可见性。
    fn set_visible(&mut self, id: ImageryLayerId, visible: bool);

    /// 提升图层（增大其 z-order）。
    fn raise(&mut self, id: ImageryLayerId);

    /// 降低图层（减小其 z-order）。
    fn lower(&mut self, id: ImageryLayerId);
}

// ============================================================================
// 地形管理
// ============================================================================

/// 管理地形 provider。
pub trait TerrainManager {
    /// 从 URL 设置地形 provider。
    fn set_terrain(&mut self, url: &str);

    /// 禁用地形（使用椭球面）。
    fn disable_terrain(&mut self);

    /// 获取地形是否启用。
    fn is_terrain_enabled(&self) -> bool;

    /// 获取某 cartographic 位置处的高度。
    fn sample_height(&self, position: &Cartographic) -> Option<f64>;
}

// ============================================================================
// 实体/图元（Primitive）管理
// ============================================================================

/// 实体的唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId(pub u64);

/// 管理实体（点、折线、多边形、模型等）
pub trait EntityManager {
    /// 添加点实体。
    fn add_point(
        &mut self,
        position: Cartographic,
        color: [f32; 4],
        pixel_size: f64,
    ) -> EntityId;

    /// 添加折线（polyline）实体。
    fn add_polyline(
        &mut self,
        positions: &[Cartographic],
        color: [f32; 4],
        width: f64,
    ) -> EntityId;

    /// 添加多边形（polygon）实体。
    fn add_polygon(
        &mut self,
        positions: &[Cartographic],
        color: [f32; 4],
    ) -> EntityId;

    /// 添加 3D 模型实体。
    fn add_model(
        &mut self,
        position: Cartographic,
        uri: &str,
        scale: f64,
    ) -> EntityId;

    /// 移除实体。
    fn remove(&mut self, id: EntityId) -> bool;

    /// 设置实体的位置。
    fn set_position(&mut self, id: EntityId, position: Cartographic);

    /// 显示/隐藏实体。
    fn set_visible(&mut self, id: EntityId, visible: bool);
}

// ============================================================================
// 拾取/选择
// ============================================================================

/// 拾取（pick）操作的结果。
#[derive(Debug, Clone)]
pub enum PickResult {
    /// 拾取到实体。
    Entity(EntityId),
    /// 拾取到瓦片要素（3D Tiles）。
    TileFeature { tileset_id: u64, feature_id: u64 },
    /// 拾取到地球表面。
    GlobeSurface(Cartographic),
    /// 未拾取到任何对象。
    None,
}

/// 提供拾取/选择功能。
pub trait Picking {
    /// 在屏幕坐标 (x, y) 处拾取。
    fn pick(&self, x: f64, y: f64) -> PickResult;

    /// 在屏幕坐标处穿透拾取（返回该位置的所有对象）。
    fn drill_pick(&self, x: f64, y: f64) -> Vec<PickResult>;

    /// 获取屏幕坐标处对应的 cartographic 位置。
    fn pick_position(&self, x: f64, y: f64) -> Option<Cartographic>;
}
