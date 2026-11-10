//! 几何更新器：将 Entity 图形属性转换为 GeometryData。
//!
//! 每种图形类型都有一个更新器函数，在给定时间处提取属性值，
//! 并生成填充/轮廓几何实例。所有更新器共享同一套求值约定：
//! 先看 show 开关，再取必需几何参数（缺失即提前返回空结果），
//! 最后按 fill/outline 开关分别组装填充与轮廓实例。
//!
//! 坐标系约定：实体位置与各类路径均以弧度制的（经度、纬度、高度）地图
//! 投影坐标存储，需先转为 WGS84 椭球上的笛卡尔坐标再参与几何构造；模型
//! 矩阵为 4x4 列主序 f64，平移量占据最后三个分量。

use glam::DVec3;

use cesium_geospatial::geometry::{
    self, box_geometry, box_outline_geometry, cylinder_geometry, cylinder_outline_geometry,
    ellipse_geometry, ellipse_outline_geometry, plane_geometry, plane_outline_geometry,
    rectangle_geometry, rectangle_outline_geometry, GeometryData, VertexFormat,
};
use cesium_geospatial::geometry::corridor::{corridor_geometry, corridor_outline_geometry, CorridorOptions};
use cesium_geospatial::geometry::ellipse::EllipseOptions;
use cesium_geospatial::geometry::polyline_geo::{polyline_geometry, PolylineOptions};
use cesium_geospatial::geometry::polyline_volume::{polyline_volume_geometry, PolylineVolumeOptions};
use cesium_geospatial::geometry::wall::{wall_geometry, wall_outline_geometry, WallOptions};
use cesium_geospatial::{Cartographic, Ellipsoid, Rectangle};

use crate::entity::{
    BoxGraphics, CorridorGraphics, CylinderGraphics, EllipseGraphics, EllipsoidGraphics,
    Entity, PlaneGraphics, PolylineGraphics,
    PolylineVolumeGraphics, RectangleGraphics, WallGraphics,
};
use crate::property::Color;

/// 一个准备好用于渲染的几何实例。
///
/// 封装一份几何数据与其世界放置、颜色及来源实体标识，供下游
/// 可视化器直接提交给渲染后端。
#[derive(Debug, Clone)]
pub struct GeometryInstance {
    /// 几何数据（位置、索引、法线等）。
    pub geometry: GeometryData,
    /// 模型矩阵（4x4 列主序，f64）。
    pub model_matrix: [f64; 16],
    /// 填充颜色（RGBA 0..1）。
    pub color: Color,
    /// 此实例是否为轮廓实例。
    pub is_outline: bool,
    /// 产生此实例的实体 ID。
    pub entity_id: String,
}

impl GeometryInstance {
    /// 创建一个使用单位模型矩阵的新几何实例。
    pub fn new(geometry: GeometryData, color: Color, is_outline: bool, entity_id: String) -> Self {
        // 初始化为行主序书写的单位矩阵（内存中为列主序），后续可经
        // with_translation 覆盖平移列。
        Self {
            geometry,
            model_matrix: [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ],
            color,
            is_outline,
            entity_id,
        }
    }

    /// 由一个平移（以 Cartesian3 表示的位置）设置模型矩阵。
    pub fn with_translation(mut self, translation: DVec3) -> Self {
        // 列主序 4x4，平移在最后一列
        self.model_matrix[12] = translation.x;
        self.model_matrix[13] = translation.y;
        self.model_matrix[14] = translation.z;
        self
    }
}

/// 在给定时间处更新实体几何的结果。
#[derive(Debug, Clone, Default)]
pub struct EntityGeometry {
    /// 填充几何实例。
    pub fill_instances: Vec<GeometryInstance>,
    /// 轮廓几何实例。
    pub outline_instances: Vec<GeometryInstance>,
}

impl EntityGeometry {
    /// 若没有任何几何实例则返回 true。
    pub fn is_empty(&self) -> bool {
        // 填充与轮廓两组均为空才算空。
        self.fill_instances.is_empty() && self.outline_instances.is_empty()
    }

    /// 实例总数。
    pub fn instance_count(&self) -> usize {
        // 填充实例数与轮廓实例数之和。
        self.fill_instances.len() + self.outline_instances.len()
    }
}

/// 将一个地图投影位置 [lon_rad, lat_rad, height_m] 转换为 Cartesian3。
pub fn cartographic_to_cartesian(pos: &[f64; 3], ellipsoid: &Ellipsoid) -> DVec3 {
    // 入参为弧度制的 [经度, 纬度, 高度]，先包装为 Cartographic 再投影到椭球面。
    let carto = Cartographic::from_radians(pos[0], pos[1], pos[2]);
    ellipsoid.cartographic_to_cartesian(&carto)
}

/// 将一组地图投影位置转换为 Cartesian3。
pub fn positions_to_cartesian(positions: &[[f64; 3]], ellipsoid: &Ellipsoid) -> Vec<DVec3> {
    // 逐个将地图投影位置转为笛卡尔坐标，保留原有顺序。
    positions
        .iter()
        .map(|p| cartographic_to_cartesian(p, ellipsoid))
        .collect()
}

/// 在给定时间处更新实体的方框图形。
///
/// 依次求值 show、dimensions、position、fill/material 与 outline/outline_color：
/// 仅当 show 为真且 dimensions 存在时才生成几何；方框以 `half = dimensions/2`
/// 在局部 `-half..half` 区间构造成盒体，再平移至实体所在的大地位置。
///
/// # 返回
/// 含填充与轮廓实例的 `EntityGeometry`。
pub fn update_box_graphics(
    entity: &Entity,
    graphics: &BoxGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        // 关闭显隐开关时无需构建任何几何，直接返回空结果。
        return result;
    }

    // dimensions 为必需参数：缺失则无法确定盒体尺寸，提前返回。
    let dimensions = match graphics.dimensions.get_value(time) {
        Some(d) => *d,
        None => return result,
    };

    // 实体位置以地图投影坐标存储，需先转为笛卡尔坐标作为盒体中心。
    let position = match entity.position.get_value(time) {
        Some(p) => cartographic_to_cartesian(p, ellipsoid),
        None => DVec3::ZERO,
    };

    // 盒体以原点为中心、按各轴向半尺寸构造；顶点格式取全部属性。
    // fill 与 outline 两个开关相互独立，可同时开启。
    let half = DVec3::new(dimensions[0] / 2.0, dimensions[1] / 2.0, dimensions[2] / 2.0);
    let vf = VertexFormat::ALL;

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：material 求值为颜色（缺省白色），生成封闭盒体表面实例。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 以 -half 到 half 构六个面组成的封闭盒体。
        let geo = box_geometry(-half, half, vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone())
                .with_translation(position),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：生成盒体 12 条棱线，颜色取 outline_color（缺省黑色）。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 轮廓几何与填充共享同一半尺寸，仅取棱边拓扑。
        let geo = box_outline_geometry(-half, half);
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, entity.id.clone())
                .with_translation(position),
        );
    }

    result
}

/// 在给定时间处更新实体的圆柱图形。
///
/// 提取 length、top_radius、bottom_radius 与 slices，以实体位置为中心
/// 生成沿局部轴堆叠的圆柱面；slices 控制周向细分数（默认 128）。fill
/// 与 outline 开关分别控制顶/底面填充与侧面轮廓线。
pub fn update_cylinder_graphics(
    entity: &Entity,
    graphics: &CylinderGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        // 显隐关闭时不生成几何。
        return result;
    }

    // length 为必需参数；上下半径缺省为 0（退化为尖锐顶端/底端）。
    let length = match graphics.length.get_value(time) {
        Some(&l) => l,
        None => return result,
    };
    let top_radius = graphics.top_radius.get_value(time).copied().unwrap_or(0.0);
    let bottom_radius = graphics.bottom_radius.get_value(time).copied().unwrap_or(0.0);

    // 以实体位置为圆柱中心（地图投影坐标→笛卡尔）。
    // 与盒体一样，圆柱也以原点为中心构造再平移。
    let position = match entity.position.get_value(time) {
        Some(p) => cartographic_to_cartesian(p, ellipsoid),
        None => DVec3::ZERO,
    };

    // slices 为周向细分段数（默认 128）。
    let slices = graphics.slices.get_value(time).copied().unwrap_or(128.0) as u32;
    let vf = VertexFormat::ALL;

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：生成含顶/底盖的封闭圆柱面。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 按长度、上下半径与周向段数扫出圆柱侧面与盖面。
        let geo = cylinder_geometry(length, top_radius, bottom_radius, slices, vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone())
                .with_translation(position),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：仅生成侧面上下两条环线。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 轮廓只取圆周与上下环，不生成盖面。
        let geo = cylinder_outline_geometry(length, top_radius, bottom_radius, slices);
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, entity.id.clone())
                .with_translation(position),
        );
    }

    result
}

/// 在给定时间处更新实体的椭圆图形。
///
/// 以 semi_major_axis 与 semi_minor_axis 为必需参数，配合 height、rotation
/// 与默认 1 度粒度构造 `EllipseOptions`，并以实体位置为中心生成椭圆盘。
/// 填充与轮廓分别经 `ellipse_geometry` 与 `ellipse_outline_geometry` 产出。
pub fn update_ellipse_graphics(
    entity: &Entity,
    graphics: &EllipseGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        return result;
    }

    let semi_major = match graphics.semi_major_axis.get_value(time) {
        Some(&v) => v,
        None => return result,
    };
    // 半长轴与半短轴为必需参数，缺一则无法确定椭圆形状。
    let semi_minor = match graphics.semi_minor_axis.get_value(time) {
        Some(&v) => v,
        None => return result,
    };

    // 椭圆以实体位置为中心（地图投影坐标→笛卡尔）。
    let position = match entity.position.get_value(time) {
        Some(p) => cartographic_to_cartesian(p, ellipsoid),
        None => DVec3::ZERO,
    };

    // height 为椭圆离地高度，rotation 为长轴相对正北的旋转角，均缺省 0。
    let height = graphics.height.get_value(time).copied().unwrap_or(0.0);
    let rotation = graphics.rotation.get_value(time).copied().unwrap_or(0.0);
    let vf = VertexFormat::ALL;

    // 组装选项：粒度取 1 度（每段弧对应的地心角），st_rotation 局部旋转缺省 0。
    // 椭球面几何在选项内已完成定位（center），无需额外平移。
    let options = EllipseOptions {
        center: position,
        semi_major_axis: semi_major,
        semi_minor_axis: semi_minor,
        height,
        rotation,
        st_rotation: 0.0,
        granularity: std::f64::consts::PI / 180.0,
        ellipsoid: *ellipsoid,
    };

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：按选项生成椭圆盘面（中心已在 options 内，无需再平移）。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 盘面从中心向周边按粒度扇形采样。
        let geo = ellipse_geometry(&options, vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone()),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：沿椭圆周边生成闭合曲线。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 轮廓与填充共享选项，仅取周边采样点连成围线。
        let geo = ellipse_outline_geometry(&options);
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, entity.id.clone()),
        );
    }

    result
}

/// 在给定时间处更新实体的走廊（corridor）图形。
///
/// 走廊沿一组大地位置以固定宽度延伸。需 positions（至少 2 点）与 width；
/// 高度、粒度与转角类型（圆角/斜接/倒角）可选。位置先由地图投影坐标转为
/// 笛卡尔坐标再参与几何构造。
pub fn update_corridor_graphics(
    entity: &Entity,
    graphics: &CorridorGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        return result;
    }

    let positions_raw = match graphics.positions.get_value(time) {
        Some(p) => p,
        None => return result,
    };
    // positions 与 width 均为必需参数：缺少任一则走廊无法成形。
    let width = match graphics.width.get_value(time) {
        Some(&w) => w,
        None => return result,
    };

    // 将地图投影坐标转为笛卡尔坐标；不足 2 点无法构成路径。
    let positions = positions_to_cartesian(positions_raw, ellipsoid);
    if positions.len() < 2 {
        return result;
    }

    // height 缺省 0，granularity 缺省 1 度；转角类型按实体枚举映射到几何层枚举。
    let height = graphics.height.get_value(time).copied().unwrap_or(0.0);
    let granularity = graphics.granularity.get_value(time).copied().unwrap_or(std::f64::consts::PI / 180.0);
    let corner_type = match graphics.corner_type {
        crate::entity::CornerType::Rounded => cesium_geospatial::geometry::corridor::CornerType::Rounded,
        crate::entity::CornerType::Mitered => cesium_geospatial::geometry::corridor::CornerType::Mitered,
        crate::entity::CornerType::Beveled => cesium_geospatial::geometry::corridor::CornerType::Beveled,
    };

    // 组装走廊选项：位置、宽度、高度、粒度、转角类型与椭球。
    let options = CorridorOptions {
        positions,
        width,
        height,
        granularity,
        corner_type,
        ellipsoid: *ellipsoid,
    };

    let vf = VertexFormat::ALL;

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：沿路径扫掠出带状走廊表面。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 走廊面由选项内位置沿宽度两侧偏移生成，转角按类型处理。
        let geo = corridor_geometry(&options, vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone()),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：生成走廊两侧边缘与端头围线。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 轮廓与填充共享选项，仅取围线拓扑。
        let geo = corridor_outline_geometry(&options);
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, entity.id.clone()),
        );
    }

    result
}

/// 在给定时间处更新实体的矩形图形。
///
/// 矩形由西南角与东北角四个弧度坐标确定，配合 height 与 granularity 在地
/// 表上扫出一块矩形面。此更新器不依赖实体位置（参数仅用于取回 entity id）。
pub fn update_rectangle_graphics(
    _entity: &Entity,
    graphics: &RectangleGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        // 显隐关闭时直接返回空结果。
        return result;
    }

    // coordinates 为必需参数：四个弧度坐标 [west, south, east, north]。
    let coords = match graphics.coordinates.get_value(time) {
        Some(c) => *c,
        None => return result,
    };

    // 由四坐标构造矩形区域；height 为离地高度，granularity 缺省 1 度。
    let rect = Rectangle::new(coords[0], coords[1], coords[2], coords[3]);
    let height = graphics.height.get_value(time).copied().unwrap_or(0.0);
    let granularity = graphics.granularity.get_value(time).copied().unwrap_or(std::f64::consts::PI / 180.0);
    let vf = VertexFormat::ALL;

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：在地表扫出矩形面，顶点沿粒度加密。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 矩形面沿 east-west 与 north-south 按粒度采样高度。
        let geo = rectangle_geometry(&rect, ellipsoid, granularity, height, vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, _entity.id.clone()),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：沿矩形四边生成闭合围线。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 轮廓仅取矩形四角连线，不需高度参数。
        let geo = rectangle_outline_geometry(&rect, ellipsoid, granularity);
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, _entity.id.clone()),
        );
    }

    result
}

/// 在给定时间处更新实体的墙体（wall）图形。
///
/// 墙体沿一组位置竖直悬挂，需至少 2 个位置；minimum_heights 与 maximum_heights
/// 分别给出每个位置处墙底与墙顶高度（可选，缺省时贴合地表与粒度默认 1 度）。
pub fn update_wall_graphics(
    entity: &Entity,
    graphics: &WallGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        // 显隐关闭时直接返回空结果。
        return result;
    }

    // positions 为必需参数：墙顶/墙底共享的同一组路径点。
    let positions_raw = match graphics.positions.get_value(time) {
        Some(p) => p,
        None => return result,
    };

    // 地图投影坐标→笛卡尔；不足 2 点无法构成墙体。
    let positions = positions_to_cartesian(positions_raw, ellipsoid);
    if positions.len() < 2 {
        return result;
    }

    // 每位置处的墙底/墙顶高度为可选数组；粒度缺省 1 度。
    let minimum_heights = graphics.minimum_heights.get_value(time).cloned();
    let maximum_heights = graphics.maximum_heights.get_value(time).cloned();
    let granularity = graphics.granularity.get_value(time).copied().unwrap_or(std::f64::consts::PI / 180.0);
    let vf = VertexFormat::ALL;

    // 组装墙体选项：位置、上下高度数组、粒度与椭球。
    let options = WallOptions {
        positions,
        minimum_heights,
        maximum_heights,
        granularity,
        ellipsoid: *ellipsoid,
    };

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：沿路径生成竖直墙面。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 墙面逐段由上一位置的上下高度连到当前位置，形成竖直四边形带。
        let geo = wall_geometry(&options, vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone()),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：生成墙顶/墙底边缘线。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 轮廓与填充共享选项，仅取顶/底围线。
        let geo = wall_outline_geometry(&options);
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, entity.id.clone()),
        );
    }

    result
}

/// 在给定时间处更新实体的椭球图形。
///
/// 以三轴半径 radii 为必需参数，slices（周向）与 stack_partitions（纵向，默认
/// 64）控制网格精度；以实体位置为中心生成球面，fill 与 outline 分别产出表面与轮廓。
pub fn update_ellipsoid_graphics(
    entity: &Entity,
    graphics: &EllipsoidGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        // 显隐关闭时不生成几何。
        return result;
    }

    let radii = match graphics.radii.get_value(time) {
        Some(r) => *r,
        None => return result,
    };

    // radii 为必需参数：三轴半径 [x, y, z]，确定椭球形状与大小。
    let position = match entity.position.get_value(time) {
        Some(p) => cartographic_to_cartesian(p, ellipsoid),
        None => DVec3::ZERO,
    };

    // slices 周向段数默认 128，stack_partitions 纵向分区默认 64。
    let slices = graphics.slices.get_value(time).copied().unwrap_or(128.0) as u32;
    let stack_partitions = graphics.stack_partitions.get_value(time).copied().unwrap_or(64.0) as u32;
    let vf = VertexFormat::ALL;

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：以原点构造球面后平移到实体位置。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 球面按纵向分区与周向段数采样。
        let geo = geometry::ellipsoid_geometry(
            DVec3::from(radii),
            stack_partitions,
            slices,
            vf,
        );
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone())
                .with_translation(position),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：生成椭球表面的经纬网格线。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 轮廓取经线与纬线网格。
        let geo = geometry::ellipsoid_outline_geometry(
            DVec3::from(radii),
            stack_partitions,
            slices,
        );
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, entity.id.clone())
                .with_translation(position),
        );
    }

    result
}

/// 在给定时间处更新实体的平面图形。
///
/// 平面由一个单位法线与带符号距离定义的无限大平面裁剪到 dimensions 尺寸；
/// 本更新器先检查 plane 与 dimensions 属性是否存在，再在实体位置生成局部平面几何。
pub fn update_plane_graphics(
    entity: &Entity,
    graphics: &PlaneGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        return result;
    }

    // plane 定义（单位法线 + 带符号距离）与 dimensions（局部宽高）为必需参数；
    // 当前实现先仅检查两者存在，几何本体使用单位平面模板。此处 plane 为必需。
    let _plane_def = match graphics.plane.get_value(time) {
        Some(p) => p,
        None => return result,
    };

    let _dimensions = match graphics.dimensions.get_value(time) {
        Some(d) => *d,
        None => return result,
    };

    let position = match entity.position.get_value(time) {
        Some(p) => cartographic_to_cartesian(p, ellipsoid),
        None => DVec3::ZERO,
    };

    // 平面几何使用单位平面模板，再平移到实体位置。
    let vf = VertexFormat::ALL;

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：生成裁剪后的平面盘。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        let geo = plane_geometry(vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone())
                .with_translation(position),
        );
    }

    let outline = graphics.outline.get_value(time).copied().unwrap_or(false);
    if outline {
        // 轮廓分支：生成平面边界矩形围线。
        let outline_color = graphics.outline_color.get_value(time).copied().unwrap_or(Color::BLACK);
        // 平面轮廓为局部坐标下的单位正方形围道。
        let geo = plane_outline_geometry();
        result.outline_instances.push(
            GeometryInstance::new(geo, outline_color, true, entity.id.clone())
                .with_translation(position),
        );
    }

    result
}

/// 在给定时间处更新实体的折线（polyline）图形。
///
/// 折线需至少 2 个位置，沿大地测地线在相邻顶点间连接；width 与 color 控制外观，
/// 粒度固定为 1 度。折线作为单一填充实例产出（无单独轮廓）。
pub fn update_polyline_graphics(
    entity: &Entity,
    graphics: &PolylineGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        // 显隐关闭时直接返回空结果。
        return result;
    }

    // positions 为必需参数：折线的路径控制点。
    let positions_raw = match graphics.positions.get_value(time) {
        Some(p) => p,
        None => return result,
    };

    // 地图投影坐标→笛卡尔；不足 2 点无法构成折线。
    let positions = positions_to_cartesian(positions_raw, ellipsoid);
    if positions.len() < 2 {
        return result;
    }

    let width = graphics.width.get_value(time).copied().unwrap_or(1.0);
    // 折线颜色直接取自 color（默认白色），粒度固定为 1 度。
    let color = graphics.color.get_value(time).copied().unwrap_or(Color::WHITE);
    let granularity = std::f64::consts::PI / 180.0;
    let vf = VertexFormat::ALL;

    // 组装折线选项：位置、宽度、粒度与椭球。
    let options = PolylineOptions {
        positions,
        width,
        granularity,
        ellipsoid: *ellipsoid,
    };

    // 折线几何沿测地线连接相邻顶点，一次性产出带状三角面。
    let geo = polyline_geometry(&options, vf);
    result.fill_instances.push(
        GeometryInstance::new(geo, color, false, entity.id.clone()),
    );

    result
}

/// 在给定时间处更新实体的管状体（polyline volume）图形。
///
/// 沿折线路径扫掠一个二维截面形状，需 positions（至少 2 点）与 shape（截面顶点）；
/// 以 fill 开关控制是否生成扫掠体，粒度默认 1 度。与折线不同，管状体具有真实体积。
pub fn update_polyline_volume_graphics(
    entity: &Entity,
    graphics: &PolylineVolumeGraphics,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    let show = graphics.show.get_value(time).copied().unwrap_or(true);
    if !show {
        // 显隐关闭时直接返回空结果。
        return result;
    }

    let positions_raw = match graphics.positions.get_value(time) {
        Some(p) => p,
        None => return result,
    };
    // positions 与 shape 均为必需参数：shape 为待扫掠的二维截面顶点。
    let shape = match graphics.shape.get_value(time) {
        Some(s) => s.clone(),
        None => return result,
    };

    // 地图投影坐标→笛卡尔；不足 2 点无法构成扫掠路径。
    let positions = positions_to_cartesian(positions_raw, ellipsoid);
    if positions.len() < 2 {
        return result;
    }

    // 粒度缺省 1 度。
    let granularity = graphics.granularity.get_value(time).copied().unwrap_or(std::f64::consts::PI / 180.0);
    let vf = VertexFormat::ALL;

    // 组装管状体选项：路径位置、截面形状、粒度与椭球。
    let options = PolylineVolumeOptions {
        positions,
        shape,
        granularity,
        ellipsoid: *ellipsoid,
    };

    let fill = graphics.fill.get_value(time).copied().unwrap_or(true);
    if fill {
        // 填充分支：沿路径扫掠截面得到管状体。
        let color = graphics.material.get_value(time).copied().unwrap_or(Color::WHITE);
        // 截面沿路径逐点定向并扫掠出封闭体。
        let geo = polyline_volume_geometry(&options, vf);
        result.fill_instances.push(
            GeometryInstance::new(geo, color, false, entity.id.clone()),
        );
    }

    result
}

/// 在给定时间处更新实体的所有几何图形。
///
/// 这是主入口点，根据实体上定义了哪些图形
/// 分发到相应的更新器。
///
/// 当实体整体 show 为假时直接返回空结果；否则逐类累加各更新器的实例。
pub fn update_entity_geometry(
    entity: &Entity,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> EntityGeometry {
    let mut result = EntityGeometry::default();

    // 实体整体 show 为假时不产出任何几何。
    if !entity.show {
        return result;
    }

    // 依次对实体上已定义的每种图形调用对应更新器，并将各自的
    // 填充/轮廓实例并入聚合结果（同一实体可同时携带多种图形）。
    // 方框
    if let Some(ref graphics) = entity.box_graphics {
        let geo = update_box_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 圆柱
    if let Some(ref graphics) = entity.cylinder {
        let geo = update_cylinder_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 椭圆
    if let Some(ref graphics) = entity.ellipse {
        let geo = update_ellipse_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 走廊
    if let Some(ref graphics) = entity.corridor {
        let geo = update_corridor_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 矩形
    if let Some(ref graphics) = entity.rectangle {
        let geo = update_rectangle_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 墙体
    if let Some(ref graphics) = entity.wall {
        let geo = update_wall_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 椭球
    if let Some(ref graphics) = entity.ellipsoid {
        let geo = update_ellipsoid_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 平面
    if let Some(ref graphics) = entity.plane {
        let geo = update_plane_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 折线
    if let Some(ref graphics) = entity.polyline {
        let geo = update_polyline_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    // 管状体
    // 注：同一实体可同时携带多种图形，结果按类型累加。
    if let Some(ref graphics) = entity.polyline_volume {
        let geo = update_polyline_volume_graphics(entity, graphics, time, ellipsoid);
        result.fill_instances.extend(geo.fill_instances);
        result.outline_instances.extend(geo.outline_instances);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::*;
    use crate::property::Property;

    fn wgs84() -> Ellipsoid {
        // 测试统一使用 WGS84 参考椭球。
        Ellipsoid::WGS84
    }

    /// 方框图形：有尺寸无轮廓时应产出 1 个非空填充实例。
    #[test]
    fn test_update_box_graphics() {
        let entity = Entity::new("box-1")
            .with_position(0.0, 0.0, 0.0)
            .with_box(BoxGraphics {
                dimensions: Property::Constant([100.0, 200.0, 300.0]),
                ..Default::default()
            });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        // 默认仅填充：得到 1 个填充实例、无轮廓，且顶点非空。
        assert_eq!(result.fill_instances.len(), 1);
        assert_eq!(result.outline_instances.len(), 0);
        assert!(!result.fill_instances[0].geometry.positions.is_empty());
    }

    /// 方框开启轮廓时同时产出填充与轮廓各 1 个实例。
    #[test]
    fn test_update_box_with_outline() {
        let entity = Entity::new("box-2")
            .with_position(0.0, 0.0, 0.0)
            .with_box(BoxGraphics {
                dimensions: Property::Constant([100.0, 100.0, 100.0]),
                outline: Property::Constant(true),
                ..Default::default()
            });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        // 开启轮廓：填充与轮廓各 1 个，且轮廓实例标记为 is_outline。
        assert_eq!(result.fill_instances.len(), 1);
        assert_eq!(result.outline_instances.len(), 1);
        assert!(result.outline_instances[0].is_outline);
    }

    /// 圆柱图形：给定长度与上下半径应产出非空填充实例。
    #[test]
    fn test_update_cylinder_graphics() {
        let entity = Entity::new("cyl-1")
            .with_position(0.0, 0.0, 0.0)
            .with_cylinder(CylinderGraphics {
                length: Property::Constant(400.0),
                top_radius: Property::Constant(100.0),
                bottom_radius: Property::Constant(100.0),
                ..Default::default()
            });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        // 圆柱默认填充：得到非空顶点集的单一填充实例。
        assert_eq!(result.fill_instances.len(), 1);
        assert!(!result.fill_instances[0].geometry.positions.is_empty());
    }

    /// 椭球图形：给定三轴半径应产出 1 个填充实例。
    #[test]
    fn test_update_ellipse_graphics() {
        let entity = Entity::new("ell-1")
            .with_position(0.0, 0.0, 0.0)
            .with_ellipsoid(EllipsoidGraphics {
                radii: Property::Constant([500000.0, 300000.0, 200000.0]),
                ..Default::default()
            });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        // 椭球默认填充：产出 1 个填充实例。
        assert_eq!(result.fill_instances.len(), 1);
    }

    /// 走廊图形：沿 3 个位置以固定宽度应产出填充实例。
    #[test]
    fn test_update_corridor_graphics() {
        let entity = Entity::new("cor-1").with_corridor(CorridorGraphics {
            positions: Property::Constant(vec![
                [0.0, 0.0, 0.0],
                [0.05, 0.05, 0.0],
                [0.1, 0.0, 0.0],
            ]),
            // 走廊沿 3 个等间距位置以 10 万米宽度延伸。
            width: Property::Constant(100000.0),
            ..Default::default()
        });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        assert_eq!(result.fill_instances.len(), 1);
    }

    /// 墙体图形：沿位置与墙顶高度应产出填充实例。
    #[test]
    fn test_update_wall_graphics() {
        let entity = Entity::new("wall-1").with_wall(WallGraphics {
            positions: Property::Constant(vec![
                [0.0, 0.0, 0.0],
                [0.05, 0.0, 0.0],
                [0.1, 0.0, 0.0],
            ]),
            // 墙顶高度逐点给出，墙底缺省贴合地表。
            maximum_heights: Property::Constant(vec![100000.0, 100000.0, 100000.0]),
            ..Default::default()
        });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        assert_eq!(result.fill_instances.len(), 1);
    }

    /// 折线图形：沿 3 个位置应产出单一填充实例。
    #[test]
    fn test_update_polyline_graphics() {
        let entity = Entity::new("line-1").with_polyline(PolylineGraphics {
            positions: Property::Constant(vec![
                [0.0, 0.0, 0.0],
                [0.05, 0.05, 0.0],
                [0.1, 0.0, 0.0],
            ]),
            // 折线宽度 5（像素单位语义）。
            width: Property::Constant(5.0),
            ..Default::default()
        });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        assert_eq!(result.fill_instances.len(), 1);
    }

    /// 实体整体 show 为假时应返回空结果。
    #[test]
    fn test_hidden_entity_returns_empty() {
        let mut entity = Entity::new("hidden-1")
            .with_box(BoxGraphics {
                dimensions: Property::Constant([100.0, 100.0, 100.0]),
                ..Default::default()
            });
        // 手动将实体整体 show 置假。
        entity.show = false;

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        assert!(result.is_empty());
    }

    /// 图形自身 show 为假时应不产出任何实例。
    #[test]
    fn test_show_false_returns_empty() {
        let entity = Entity::new("box-noshow")
            .with_position(0.0, 0.0, 0.0)
            .with_box(BoxGraphics {
                dimensions: Property::Constant([100.0, 100.0, 100.0]),
                show: Property::Constant(false),
                ..Default::default()
            });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        assert!(result.is_empty());
    }

    /// with_translation 应将平移量写入列主序矩阵的最后三个分量。
    #[test]
    fn test_geometry_instance_translation() {
        let geo = box_geometry(DVec3::splat(-1.0), DVec3::ONE, VertexFormat::ALL);
        // 对单位盒体实例施加 (100,200,300) 平移。
        let instance = GeometryInstance::new(geo, Color::RED, false, "test".to_string())
            .with_translation(DVec3::new(100.0, 200.0, 300.0));

        assert!((instance.model_matrix[12] - 100.0).abs() < 1e-10);
        assert!((instance.model_matrix[13] - 200.0).abs() < 1e-10);
        assert!((instance.model_matrix[14] - 300.0).abs() < 1e-10);
    }

    /// 管状体图形：沿路径扫掠矩形截面应产出填充实例。
    #[test]
    fn test_update_polyline_volume_graphics() {
        let entity = Entity::new("pv-1").with_polyline_volume(PolylineVolumeGraphics {
            positions: Property::Constant(vec![
                [0.0, 0.0, 0.0],
                [0.05, 0.0, 0.0],
                [0.1, 0.0, 0.0],
            ]),
            // 扫掠截面为 1 万米见方的正方形。
            shape: Property::Constant(vec![
                [-5000.0, -5000.0],
                [5000.0, -5000.0],
                [5000.0, 5000.0],
                [-5000.0, 5000.0],
            ]),
            ..Default::default()
        });

        let result = update_entity_geometry(&entity, 0.0, &wgs84());
        assert_eq!(result.fill_instances.len(), 1);
    }
}
