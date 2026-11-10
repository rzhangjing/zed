//! 几何生成 - 所有程序化几何类型（包围体、椭圆、走廊、围墙、折线等多种几何）。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::clone_on_copy)]
pub mod coplanar_polygon;
pub mod corridor;
pub mod ellipse;
pub mod frustum_geo;
pub mod ground_polyline;
pub mod polyline_geo;
pub mod polyline_volume;
pub mod wall;

use crate::bounding::BoundingSphere;
use crate::ellipsoid::Ellipsoid;
use crate::rectangle::Rectangle;
use crate::math_utils;
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

pub use coplanar_polygon::{coplanar_polygon_geometry, CoplanarPolygonOptions};
pub use corridor::{corridor_geometry, corridor_outline_geometry, CornerType, CorridorOptions};
pub use ground_polyline::{ground_polyline_geometry, GroundPolylineOptions};
pub use ellipse::{
    ellipse_geometry, ellipse_outline_geometry, EllipseOptions,
};
pub use polyline_geo::{polyline_geometry, PolylineOptions};
pub use polyline_volume::{polyline_volume_geometry, PolylineVolumeOptions};
pub use frustum_geo::{
    frustum_geometry, frustum_outline_geometry, FrustumDef,
};
pub use wall::{wall_geometry, wall_outline_geometry, WallOptions};

/// 顶点格式标志 - 生成哪些属性。
/// 映射到 CesiumJS `VertexFormat`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VertexFormat {
    pub position: bool,
    pub normal: bool,
    pub st: bool,
    pub tangent: bool,
    pub bitangent: bool,
}

impl VertexFormat {
    /// 启用所有属性。
    pub const ALL: Self = Self {
        position: true,
        normal: true,
        st: true,
        tangent: true,
        bitangent: true,
    };

    /// 仅位置。
    pub const POSITION_ONLY: Self = Self {
        position: true,
        normal: false,
        st: false,
        tangent: false,
        bitangent: false,
    };

    /// 位置和法线。
    pub const POSITION_AND_NORMAL: Self = Self {
        position: true,
        normal: true,
        st: false,
        tangent: false,
        bitangent: false,
    };

    /// 位置和纹理坐标。
    pub const POSITION_AND_ST: Self = Self {
        position: true,
        normal: false,
        st: true,
        tangent: false,
        bitangent: false,
    };

    /// 打包本结构所用的元素数量。
    pub const PACKED_LENGTH: usize = 5;

    /// 将本 VertexFormat 打包进一个扁平数组。
    ///
    /// 映射到 CesiumJS `VertexFormat.pack`
    pub fn pack(&self, array: &mut [f64], starting_index: usize) {
        array[starting_index] = if self.position { 1.0 } else { 0.0 };
        array[starting_index + 1] = if self.normal { 1.0 } else { 0.0 };
        array[starting_index + 2] = if self.st { 1.0 } else { 0.0 };
        array[starting_index + 3] = if self.tangent { 1.0 } else { 0.0 };
        array[starting_index + 4] = if self.bitangent { 1.0 } else { 0.0 };
    }

    /// 从一个扁平数组解包出 VertexFormat。
    ///
    /// 映射到 CesiumJS `VertexFormat.unpack`
    pub fn unpack(array: &[f64], starting_index: usize) -> Self {
        Self {
            position: array[starting_index] != 0.0,
            normal: array[starting_index + 1] != 0.0,
            st: array[starting_index + 2] != 0.0,
            tangent: array[starting_index + 3] != 0.0,
            bitangent: array[starting_index + 4] != 0.0,
        }
    }

    /// 将本 VertexFormat 打包进一个新的 Vec<f64>。
    pub fn pack_array(&self) -> Vec<f64> {
        let mut array = vec![0.0; Self::PACKED_LENGTH];
        self.pack(&mut array, 0);
        array
    }

    /// 从一个 Vec<f64> 解包出 VertexFormat。
    pub fn unpack_array(array: &[f64]) -> Self {
        Self::unpack(array, 0)
    }
}

impl Default for VertexFormat {
    /// 默认顶点格式等价于 `ALL`（启用全部属性）。
    fn default() -> Self {
        Self::ALL
    }
}

/// 所生成几何的图元拓扑。
/// 映射到 CesiumJS `PrimitiveType`（TRIANGLES / LINES）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimitiveType {
    /// 三角形列表（填充表面）。
    #[default]
    Triangles,
    /// 线列表（轮廓；索引为顶点对）。
    Lines,
}

/// 中间几何表示（f64 精度，与 GPU 解耦）。
/// 映射到 CesiumJS 几何 worker 的输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometryData {
    /// 顶点位置（始终存在）。
    pub positions: Vec<[f64; 3]>,
    /// 顶点法线（可选）。
    pub normals: Option<Vec<[f64; 3]>>,
    /// 纹理坐标（可选）。
    pub tex_coords: Option<Vec<[f64; 2]>>,
    /// 切线向量（可选）。
    pub tangents: Option<Vec<[f64; 3]>>,
    /// 副切线向量（可选）。
    pub bitangents: Option<Vec<[f64; 3]>>,
    /// 索引（根据 `primitive_type` 为三角形或线对）。
    pub indices: Vec<u32>,
    /// 几何的包围球。
    pub bounding_sphere: BoundingSphere,
    /// 图元拓扑（填充用三角形，轮廓用线）。
    #[serde(default)]
    pub primitive_type: PrimitiveType,
}

/// 多边形层级：外环 + 可选的洞。
/// 映射到 CesiumJS `PolygonHierarchy`
#[derive(Debug, Clone)]
pub struct PolygonHierarchy {
    /// 外环位置（测绘坐标或笛卡尔坐标）。
    pub positions: Vec<DVec3>,
    /// 洞（每个洞都是一圈位置）。
    pub holes: Vec<PolygonHierarchy>,
}

// ============================================================================
// 几何生成器
// ============================================================================

/// 生成一个椭球几何。
/// 映射到 `EllipsoidGeometry` / `Workers/createEllipsoidGeometry`
///
/// `radii` 为三轴半径（米）；`stacks` 为余纬度方向的分层数；
/// `slices` 为经度方向的切片数；`vf` 指定要生成哪些顶点属性。
///
/// # 参数
/// - `radii`：三轴半径（米），控制椭球形状。
/// - `stacks`：余纬度方向分层数，越大极点附近越平滑。
/// - `slices`：经度方向切片数，越大赤道附近越平滑。
/// - `vf`：顶点格式，控制是否额外生成法线与 UV。
///
/// # 返回
/// 一个以三角形填充、法线朝外的 `GeometryData`。
pub fn ellipsoid_geometry(
    radii: DVec3,
    stacks: u32,
    slices: u32,
    vf: VertexFormat,
) -> GeometryData {
    let mut positions = Vec::new();
    let mut normals = if vf.normal { Some(Vec::new()) } else { None };
    let mut tex_coords = if vf.st { Some(Vec::new()) } else { None };

    // 逐层（stack）扫描余纬度 phi：0（北极）到 PI（南极），共 stacks+1 条纬圈。
    for i in 0..=stacks {
        let phi = std::f64::consts::PI * i as f64 / stacks as f64;
        let sin_phi = phi.sin();
        let cos_phi = phi.cos();

        // 逐片（slice）扫描经度 theta：绕一圈 0..2PI，共 slices+1 条经线。
        for j in 0..=slices {
            let theta = 2.0 * std::f64::consts::PI * j as f64 / slices as f64;
            let sin_theta = theta.sin();
            let cos_theta = theta.cos();

            // 单位球面点 (x,y,z)，再沿各轴乘以半径得到椭球顶点。
            let x = cos_theta * sin_phi;
            let y = sin_theta * sin_phi;
            let z = cos_phi;

            positions.push([x * radii.x, y * radii.y, z * radii.z]);

            if let Some(ref mut n) = normals {
                // 法线是单位球上归一化后的位置
                let normal = DVec3::new(x, y, z);
                n.push([normal.x, normal.y, normal.z]);
            }

            if let Some(ref mut st) = tex_coords {
                st.push([j as f64 / slices as f64, i as f64 / stacks as f64]);
            }
        }
    }

    // 逐四边形单元发射两个三角形；顶点行宽为 slices+1，故下一行偏移 slices+1。
    let mut indices = Vec::new();
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * (slices + 1) + j;
            let b = a + slices + 1;
            // 从外侧观察时为逆时针绕序（法线朝外）
            indices.push(a);
            indices.push(a + 1);
            indices.push(b);
            indices.push(a + 1);
            indices.push(b + 1);
            indices.push(b);
        }
    }

    let bs = BoundingSphere::new(DVec3::ZERO, radii.x.max(radii.y).max(radii.z));

    GeometryData {
        positions,
        normals,
        tex_coords,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 生成一个球体几何。
/// 映射到 `SphereGeometry`
///
/// 球体是三轴半径相等的椭球特例，直接复用 `ellipsoid_geometry`。
/// 包围球半径与传入 `radius` 一致，无需额外计算。
pub fn sphere_geometry(radius: f64, stacks: u32, slices: u32, vf: VertexFormat) -> GeometryData {
    ellipsoid_geometry(DVec3::splat(radius), stacks, slices, vf)
}

/// 生成一个盒子几何。
/// 映射到 `BoxGeometry` / `Workers/createBoxGeometry`
///
/// `minimum`/`maximum` 为轴对齐盒子的两个对角顶点；每个面使用独立
/// 顶点以便拥有逐面法线，因此共 24 个顶点、6 个面。
///
/// # 参数
/// - `minimum`：盒子各轴最小坐标角点。
/// - `maximum`：盒子各轴最大坐标角点。
/// - `vf`：顶点格式，控制逐面法线与 UV 的生成。
pub fn box_geometry(minimum: DVec3, maximum: DVec3, vf: VertexFormat) -> GeometryData {
    let size = maximum - minimum;
    let center = (minimum + maximum) * 0.5;

    // 6 个面，每个面 4 个顶点 = 24 个顶点；角点用 ±1 符号表示，再映射到盒面。
    let corners = [
        // +X 面
        [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [1.0, -1.0, 1.0],
        // -X 面
        [-1.0, -1.0, -1.0], [-1.0, -1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0],
        // +Y 面
        [-1.0, 1.0, -1.0], [-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, -1.0],
        // -Y 面
        [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0], [-1.0, -1.0, 1.0],
        // +Z 面
        [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0],
        // -Z 面
        [-1.0, -1.0, -1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, -1.0, -1.0],
    ];

    // 每个面共享一个朝外的常法线（与该面 4 个顶点相同）。
    let face_normals = [
        [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0], [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0], [0.0, 0.0, -1.0],
    ];

    let mut positions = Vec::with_capacity(24);
    let mut normals_vec = if vf.normal { Some(Vec::with_capacity(24)) } else { None };
    let mut tex_coords = if vf.st { Some(Vec::with_capacity(24)) } else { None };

    for (face_idx, corner_group) in corners.chunks(4).enumerate() {
        for (ci, corner) in corner_group.iter().enumerate() {
            positions.push([
                center.x + corner[0] * size.x * 0.5,
                center.y + corner[1] * size.y * 0.5,
                center.z + corner[2] * size.z * 0.5,
            ]);
            if let Some(ref mut n) = normals_vec {
                n.push(face_normals[face_idx]);
            }
            if let Some(ref mut st) = tex_coords {
                let u = if ci == 0 || ci == 3 { 0.0 } else { 1.0 };
                let v = if ci < 2 { 0.0 } else { 1.0 };
                st.push([u, v]);
            }
        }
    }

    let mut indices = Vec::with_capacity(36);
    for face in 0..6u32 {
        let base = face * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let bs = BoundingSphere::new(center, size.length() * 0.5);

    GeometryData {
        positions,
        normals: normals_vec,
        tex_coords,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 生成一个圆柱几何。
/// 映射到 `CylinderGeometry`
///
/// `length` 为沿 Z 轴的总长（米）；`top_radius`/`bottom_radius` 为两端
/// 半径；`slices` 为周向离散段数。本实现仅生成侧壁，未封顶底。
///
/// # 参数
/// - `length`：沿轴总长（米），上下各占一半。
/// - `top_radius`：顶端半径（米）。
/// - `bottom_radius`：底端半径（米）；与顶端不等时为圆台。
/// - `slices`：周向段数，控制侧面平滑度。
/// - `vf`：顶点格式。
pub fn cylinder_geometry(
    length: f64,
    top_radius: f64,
    bottom_radius: f64,
    slices: u32,
    vf: VertexFormat,
) -> GeometryData {
    let half_length = length * 0.5;
    let mut positions = Vec::new();
    let mut normals_vec = if vf.normal { Some(Vec::new()) } else { None };
    let mut tex_coords = if vf.st { Some(Vec::new()) } else { None };

    // 侧面顶点
    // 侧面顶点：沿每片经线成对发射底/顶顶点，法线无 z 分量（垂直侧面）。
    for i in 0..=slices {
        let theta = 2.0 * std::f64::consts::PI * i as f64 / slices as f64;
        let cos_t = theta.cos();
        let sin_t = theta.sin();

        // 底部顶点（z = -half_length，半径按 bottom_radius 缩）
        positions.push([cos_t * bottom_radius, sin_t * bottom_radius, -half_length]);
        if let Some(ref mut n) = normals_vec {
            n.push([cos_t, sin_t, 0.0]);
        }
        if let Some(ref mut st) = tex_coords {
            st.push([i as f64 / slices as f64, 0.0]);
        }

        // 顶部顶点（z = +half_length，半径按 top_radius 缩）
        positions.push([cos_t * top_radius, sin_t * top_radius, half_length]);
        if let Some(ref mut n) = normals_vec {
            n.push([cos_t, sin_t, 0.0]);
        }
        if let Some(ref mut st) = tex_coords {
            st.push([i as f64 / slices as f64, 1.0]);
        }
    }

    // 侧面索引：相邻两列的四个顶点构成两个三角形，形成围成一圈的侧壁。
    let mut indices = Vec::new();
    for i in 0..slices {
        let base = i * 2;
        indices.push(base);
        indices.push(base + 1);
        indices.push(base + 2);
        indices.push(base + 1);
        indices.push(base + 3);
        indices.push(base + 2);
    }

    let max_radius = top_radius.max(bottom_radius);
    let bs = BoundingSphere::new(DVec3::ZERO, (max_radius * max_radius + half_length * half_length).sqrt());

    GeometryData {
        positions,
        normals: normals_vec,
        tex_coords,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 在椭球表面上生成一个矩形几何。
/// 映射到 `RectangleGeometry` / `Workers/createRectangleGeometry`
///
/// `rect` 为经纬矩形；`granularity` 为采样间隔（弧度）；`height` 为椭球
/// 上方高度（米）。返回以 (cols*rows) 网格采样、逐面三角化的几何。
///
/// # 参数
/// - `rect`：经纬范围矩形。
/// - `ellipsoid`：投影所依据的椭球。
/// - `granularity`：网格采样间隔（弧度）。
/// - `height`：距椭球面的高度（米）。
/// - `vf`：顶点格式。
pub fn rectangle_geometry(
    rect: &Rectangle,
    ellipsoid: &Ellipsoid,
    granularity: f64,
    height: f64,
    vf: VertexFormat,
) -> GeometryData {
    let width = rect.width();
    let h = rect.height();
    // 按 granularity（弧度间距）将矩形离散化为网格，至少 1 行/列，+1 为端点。
    let cols = ((width / granularity).ceil() as u32).max(1) + 1;
    let rows = ((h / granularity).ceil() as u32).max(1) + 1;

    let mut positions = Vec::with_capacity((cols * rows) as usize);
    let mut normals_vec = if vf.normal { Some(Vec::new()) } else { None };
    let mut tex_coords = if vf.st { Some(Vec::new()) } else { None };

    // 逐行逐列在经纬网格上采样，将每个 (lon,lat,height) 大地坐标投影为椭球面笛卡尔点。
    for row in 0..rows {
        let lat = rect.south + h * row as f64 / (rows - 1) as f64;
        for col in 0..cols {
            let lon = rect.west + width * col as f64 / (cols - 1) as f64;
            let carto = crate::cartographic::Cartographic::from_radians(lon, lat, height);
            let pos = ellipsoid.cartographic_to_cartesian(&carto);
            positions.push([pos.x, pos.y, pos.z]);

            if let Some(ref mut n) = normals_vec {
                let normal = ellipsoid.geodetic_surface_normal(pos).unwrap_or(DVec3::Z);
                n.push([normal.x, normal.y, normal.z]);
            }
            if let Some(ref mut st) = tex_coords {
                st.push([col as f64 / (cols - 1) as f64, row as f64 / (rows - 1) as f64]);
            }
        }
    }

    // 逐网格单元发射两个三角形；行宽为 cols，下一行索引偏移 cols。
    let mut indices = Vec::new();
    for row in 0..(rows - 1) {
        for col in 0..(cols - 1) {
            let a = row * cols + col;
            let b = a + cols;
            indices.push(a);
            indices.push(b);
            indices.push(a + 1);
            indices.push(a + 1);
            indices.push(b);
            indices.push(b + 1);
        }
    }

    let bs = BoundingSphere::from_points(
        &positions.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    GeometryData {
        positions,
        normals: normals_vec,
        tex_coords,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 在椭球上生成一个圆形几何。
/// 映射到 `CircleGeometry`
///
/// 以 `center` 为圆心、`radius`（米）为地面半径，在椭球面上用
/// 扇形网格（中心点 + `segments` 个周向点）近似一个圆盘。
///
/// # 参数
/// - `center`：圆心的笛卡尔坐标。
/// - `radius`：地面半径（米）。
/// - `ellipsoid`：承载圆的椭球。
/// - `segments`：周向段数，越大越接近真圆。
/// - `vf`：顶点格式。
pub fn circle_geometry(
    center: DVec3,
    radius: f64,
    ellipsoid: &Ellipsoid,
    segments: u32,
    vf: VertexFormat,
) -> GeometryData {
    let center_carto = ellipsoid.cartesian_to_cartographic(center);
    let height = center_carto.map(|c| c.height).unwrap_or(0.0);
    let center_carto = center_carto.unwrap_or_default();

    // 预留中心顶点 + segments 个环绕顶点的空间。
    let mut positions = Vec::with_capacity(segments as usize + 1);
    let mut normals_vec = if vf.normal { Some(Vec::new()) } else { None };

    // 中心顶点
    positions.push([center.x, center.y, center.z]);
    if let Some(ref mut n) = normals_vec {
        let normal = ellipsoid.geodetic_surface_normal(center).unwrap_or(DVec3::Z);
        n.push([normal.x, normal.y, normal.z]);
    }

    // 环绕顶点：沿周向均分角度，将半径投影为经纬偏移后回到椭球面。
    for i in 0..=segments {
        let angle = 2.0 * std::f64::consts::PI * i as f64 / segments as f64;
        // 近似：沿表面以米为单位的偏移
        let d_lat = radius * angle.cos() / ellipsoid.maximum_radius();
        let d_lon = radius * angle.sin() / (ellipsoid.maximum_radius() * center_carto.latitude.cos().max(1e-10));

        let carto = crate::cartographic::Cartographic::from_radians(
            center_carto.longitude + d_lon,
            center_carto.latitude + d_lat,
            height,
        );
        let pos = ellipsoid.cartographic_to_cartesian(&carto);
        positions.push([pos.x, pos.y, pos.z]);

        if let Some(ref mut n) = normals_vec {
            let normal = ellipsoid.geodetic_surface_normal(pos).unwrap_or(DVec3::Z);
            n.push([normal.x, normal.y, normal.z]);
        }
    }

    // 扇形三角化：每个环绕段与中心点（索引 0）构成一个三角形。
    let mut indices = Vec::new();
    for i in 0..segments {
        indices.push(0);
        indices.push(i + 1);
        indices.push(i + 2);
    }

    let bs = BoundingSphere::new(center, radius);
    let num_vertices = positions.len();

    GeometryData {
        positions,
        normals: normals_vec,
        tex_coords: if vf.st { Some(vec![[0.5, 0.5]; num_vertices]) } else { None },
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 生成一个平面几何（XY 平面中的单位四边形）。
/// 映射到 `PlaneGeometry`
///
/// 顶点固定在 z=0 平面、边长为 1 并以原点为中心；包围球半径为
/// 对角线一半（√2/2）。
///
/// # 参数
/// - `vf`：顶点格式，控制是否生成法线与 UV。
pub fn plane_geometry(vf: VertexFormat) -> GeometryData {
    let positions = vec![
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
    ];
    // 平面面向 +Z；法线均为 (0,0,1)，UV 与四角一一对应。
    let normals_vec = if vf.normal {
        Some(vec![[0.0, 0.0, 1.0]; 4])
    } else {
        None
    };
    let tex_coords = if vf.st {
        Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
    } else {
        None
    };
    let indices = vec![0, 1, 2, 0, 2, 3];

    GeometryData {
        positions,
        normals: normals_vec,
        tex_coords,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, std::f64::consts::FRAC_1_SQRT_2),
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 生成一个盒子轮廓几何（12 条边作为线段）。
/// 映射到 `BoxOutlineGeometry`
///
/// 仅使用 8 个角点与 24 个索引（每边两个端点）描述线框，无面法线。
///
/// # 参数
/// - `minimum`/`maximum`：轴对齐盒子的两个对角顶点。
pub fn box_outline_geometry(minimum: DVec3, maximum: DVec3) -> GeometryData {
    let size = maximum - minimum;
    let center = (minimum + maximum) * 0.5;
    let hx = size.x * 0.5;
    let hy = size.y * 0.5;
    let hz = size.z * 0.5;

    // 盒子的 8 个角点。
    let corners = [
        [center.x - hx, center.y - hy, center.z - hz], // 0
        [center.x + hx, center.y - hy, center.z - hz], // 1
        [center.x + hx, center.y + hy, center.z - hz], // 2
        [center.x - hx, center.y + hy, center.z - hz], // 3
        [center.x - hx, center.y - hy, center.z + hz], // 4
        [center.x + hx, center.y - hy, center.z + hz], // 5
        [center.x + hx, center.y + hy, center.z + hz], // 6
        [center.x - hx, center.y + hy, center.z + hz], // 7
    ];

    // 12 条边：4 条底、4 条顶、4 条垂直。
    let indices: Vec<u32> = vec![
        0, 1, 1, 2, 2, 3, 3, 0, // 底部
        4, 5, 5, 6, 6, 7, 7, 4, // 顶部
        0, 4, 1, 5, 2, 6, 3, 7, // 垂直
    ];

    let bs = BoundingSphere::new(center, size.length() * 0.5);

    GeometryData {
        positions: corners.to_vec(),
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 在椭球上生成一个椭球轮廓几何（3 个大圆）。
/// 映射到 `EllipsoidOutlineGeometry`
///
/// 沿赤道（XY）、子午线（XZ）与侧向（YZ）三个大圆发射线段，
/// 用作线框式轮廓，不生成面。
pub fn ellipsoid_outline_geometry(radii: DVec3, stacks: u32, slices: u32) -> GeometryData {
    let mut positions: Vec<[f64; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // XY 圆（赤道）。
    let base = positions.len() as u32;
    for i in 0..=slices {
        let theta = 2.0 * std::f64::consts::PI * i as f64 / slices as f64;
        positions.push([radii.x * theta.cos(), radii.y * theta.sin(), 0.0]);
        if i > 0 {
            indices.push(base + i - 1);
            indices.push(base + i);
        }
    }

    // XZ 圆（子午线）：固定 y=0，沿 phi 扫一圈。
    let base = positions.len() as u32;
    for i in 0..=stacks {
        let phi = 2.0 * std::f64::consts::PI * i as f64 / stacks as f64;
        positions.push([radii.x * phi.cos(), 0.0, radii.z * phi.sin()]);
        if i > 0 {
            indices.push(base + i - 1);
            indices.push(base + i);
        }
    }

    // YZ 圆（侧向）：固定 x=0，沿 phi 扫一圈。
    let base = positions.len() as u32;
    for i in 0..=stacks {
        let phi = 2.0 * std::f64::consts::PI * i as f64 / stacks as f64;
        positions.push([0.0, radii.y * phi.cos(), radii.z * phi.sin()]);
        if i > 0 {
            indices.push(base + i - 1);
            indices.push(base + i);
        }
    }

    // 每对相邻顶点构成一条线段（索引成对），包围球半径取最大轴半径。
    let max_r = radii.x.max(radii.y).max(radii.z);
    let bs = BoundingSphere::new(DVec3::ZERO, max_r);

    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 在椭球表面上生成一个圆形轮廓几何。
/// 映射到 `CircleOutlineGeometry`
///
/// 沿周向按 `granularity`（弧度）采样闭合成环；若中心无法
/// 转为大地坐标则回退为空线集。
///
/// # 参数
/// - `center`：圆心笛卡尔坐标。
/// - `radius`：地面半径（米）。
/// - `ellipsoid`：参考椭球。
/// - `granularity`：采样角间距（弧度）。
pub fn circle_outline_geometry(
    center: DVec3,
    radius: f64,
    ellipsoid: &Ellipsoid,
    granularity: f64,
) -> GeometryData {
    let center_carto = ellipsoid.cartesian_to_cartographic(center);
    let Some(center_carto) = center_carto else {
        return empty_lines();
    };

    let num_segments = ((2.0 * std::f64::consts::PI / granularity).ceil() as u32).max(3);
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(num_segments as usize);
    let mut indices: Vec<u32> = Vec::with_capacity(num_segments as usize * 2);

    for i in 0..num_segments {
        let angle = 2.0 * std::f64::consts::PI * i as f64 / num_segments as f64;
        let d_lat = (radius / ellipsoid.maximum_radius()) * angle.sin();
        let d_lon = (radius / (ellipsoid.maximum_radius() * center_carto.latitude.cos().max(0.01))) * angle.cos();
        let carto = crate::cartographic::Cartographic::from_radians(
            center_carto.longitude + d_lon,
            center_carto.latitude + d_lat,
            center_carto.height,
        );
        let p = ellipsoid.cartographic_to_cartesian(&carto);
        positions.push([p.x, p.y, p.z]);

        if i > 0 {
            indices.push(i - 1);
            indices.push(i);
        }
    }
    // 闭合环路。
    indices.push(num_segments - 1);
    indices.push(0);

    let bs = BoundingSphere::from_points(
        &positions.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 在椭球表面上生成一个矩形轮廓几何。
/// 映射到 `RectangleOutlineGeometry`
///
/// 沿南/东/北/西四条边各自按 `granularity` 采样为线段，不填充内部。
///
/// # 参数
/// - `rect`：经纬范围矩形。
/// - `ellipsoid`：参考椭球。
/// - `granularity`：采样角间距（弧度）。
pub fn rectangle_outline_geometry(
    rect: &Rectangle,
    ellipsoid: &Ellipsoid,
    granularity: f64,
) -> GeometryData {
    let mut positions: Vec<[f64; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // 将一条边 (lon0,lat0)->(lon1,lat1) 按 granularity 采样为顶点序列与线段索引。
    let add_edge = |positions: &mut Vec<[f64; 3]>, indices: &mut Vec<u32>,
                    lon0: f64, lat0: f64, lon1: f64, lat1: f64| {
        let angular_dist = ((lat1 - lat0).powi(2) + (lon1 - lon0).powi(2)).sqrt();
        let num_seg = ((angular_dist / granularity).ceil() as usize).max(1);
        let base = positions.len() as u32;
        for i in 0..=num_seg {
            let t = i as f64 / num_seg as f64;
            let lon = math_utils::lerp(lon0, lon1, t);
            let lat = math_utils::lerp(lat0, lat1, t);
            let carto = crate::cartographic::Cartographic::from_radians(lon, lat, 0.0);
            let p = ellipsoid.cartographic_to_cartesian(&carto);
            positions.push([p.x, p.y, p.z]);
            if i > 0 {
                indices.push(base + i as u32 - 1);
                indices.push(base + i as u32);
            }
        }
    };

    // 底边（沿南边从西到东）。
    add_edge(&mut positions, &mut indices, rect.west, rect.south, rect.east, rect.south);
    // 右边（沿东边从南到北）。
    add_edge(&mut positions, &mut indices, rect.east, rect.south, rect.east, rect.north);
    // 顶边（沿北边从东到西）。
    add_edge(&mut positions, &mut indices, rect.east, rect.north, rect.west, rect.north);
    // 左边（沿西边从北到南）。
    add_edge(&mut positions, &mut indices, rect.west, rect.north, rect.west, rect.south);

    let bs = BoundingSphere::from_points(
        &positions.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 生成一个圆柱轮廓几何。
/// 映射到 `CylinderOutlineGeometry`
///
/// 包含底部圆、顶部圆，以及最多 16 条等间隔的垂直侧棱。
///
/// # 参数
/// - `length`：沿轴总长（米）。
/// - `top_radius`/`bottom_radius`：两端半径（米）。
/// - `slices`：周向段数。
pub fn cylinder_outline_geometry(
    length: f64,
    top_radius: f64,
    bottom_radius: f64,
    slices: u32,
) -> GeometryData {
    let half_length = length * 0.5;
    let mut positions: Vec<[f64; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // 底部圆。
    let base = positions.len() as u32;
    for i in 0..=slices {
        let theta = 2.0 * std::f64::consts::PI * i as f64 / slices as f64;
        positions.push([bottom_radius * theta.cos(), bottom_radius * theta.sin(), -half_length]);
        if i > 0 {
            indices.push(base + i - 1);
            indices.push(base + i);
        }
    }

    // 顶部圆。
    let base = positions.len() as u32;
    for i in 0..=slices {
        let theta = 2.0 * std::f64::consts::PI * i as f64 / slices as f64;
        positions.push([top_radius * theta.cos(), top_radius * theta.sin(), half_length]);
        if i > 0 {
            indices.push(base + i - 1);
            indices.push(base + i);
        }
    }

    // 垂直边（按间隔连接底部与顶部）。
    let num_verticals = slices.min(16);
    for i in 0..num_verticals {
        let theta = 2.0 * std::f64::consts::PI * i as f64 / slices as f64;
        let bottom_idx = positions.len() as u32;
        positions.push([bottom_radius * theta.cos(), bottom_radius * theta.sin(), -half_length]);
        let top_idx = positions.len() as u32;
        positions.push([top_radius * theta.cos(), top_radius * theta.sin(), half_length]);
        indices.push(bottom_idx);
        indices.push(top_idx);
    }

    let max_radius = top_radius.max(bottom_radius);
    let bs = BoundingSphere::new(DVec3::ZERO, (max_radius * max_radius + half_length * half_length).sqrt());

    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: bs,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 生成一个平面轮廓几何（单位四边形各边）。
/// 映射到 `PlaneOutlineGeometry`
///
/// 四个角点依次首尾相连，共 8 个索引（4 条边）。无面与法线。
pub fn plane_outline_geometry() -> GeometryData {
    let positions = vec![
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
    ];
    let indices = vec![0, 1, 1, 2, 2, 3, 3, 0];

    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: BoundingSphere::new(DVec3::ZERO, std::f64::consts::FRAC_1_SQRT_2),
        primitive_type: PrimitiveType::Lines,
    }
}

/// 返回一个不含任何顶点/索引的空线集几何，用作退化输入的回退。
fn empty_lines() -> GeometryData {
    GeometryData {
        positions: Vec::new(),
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: Vec::new(),
        bounding_sphere: BoundingSphere::default(),
        primitive_type: PrimitiveType::Lines,
    }
}

// ============================================================================
// 管道算法
// ============================================================================

/// 以给定的细分粒度在两个位置之间生成一段弧（大圆）。
/// 映射到 `PolylinePipeline.generateArc`
///
/// 将每段相邻顶点在大地坐标上按 `granularity`（弧度）细分，再回到
/// 笛卡尔，以保证沿线贴合椭球曲面。
///
/// # 参数
/// - `positions`：折线控制点（笛卡尔）。
/// - `granularity`：最大插值角间距（弧度）。
/// - `ellipsoid`：用于大地↔笛卡尔往返的椭球。
pub fn generate_arc(positions: &[DVec3], granularity: f64, ellipsoid: &Ellipsoid) -> Vec<DVec3> {
    if positions.len() < 2 {
        return positions.to_vec();
    }

    let mut result = Vec::new();
    // 逐段处理相邻顶点对，按大地距离与 granularity 决定中间插值点数。
    for i in 0..positions.len() - 1 {
        let start = positions[i];
        let end = positions[i + 1];

        let start_carto = ellipsoid.cartesian_to_cartographic(start);
        let end_carto = ellipsoid.cartesian_to_cartographic(end);

        if let (Some(sc), Some(ec)) = (start_carto, end_carto) {
            // 用经纬差近似角距离，据此估算细分段数（至少 1）。
            let angular_distance = ((ec.latitude - sc.latitude).powi(2)
                + (ec.longitude - sc.longitude).powi(2))
            .sqrt();
            let num_segments = ((angular_distance / granularity).ceil() as usize).max(1);

            for j in 0..num_segments {
                // 在大地坐标上对 (lon,lat,height) 线性插值，再回到笛卡尔。
                let t = j as f64 / num_segments as f64;
                let lon = math_utils::lerp(sc.longitude, ec.longitude, t);
                let lat = math_utils::lerp(sc.latitude, ec.latitude, t);
                let h = math_utils::lerp(sc.height, ec.height, t);
                let carto = crate::cartographic::Cartographic::from_radians(lon, lat, h);
                result.push(ellipsoid.cartographic_to_cartesian(&carto));
            }
        } else {
            result.push(start);
        }
    }
    result.push(*positions.last().unwrap());
    result
}

/// 使用 earcut 算法对一个 2D 多边形进行三角剖分。
/// `holes` 是 `positions` 中各洞起始索引的数组。
/// 映射到 `PolygonPipeline.triangulate`
///
/// 少于 3 个顶点时返回空；否则将 [f64;2] 顶点交给 earcut，返回
/// 三角形索引列表。
pub fn triangulate_polygon(positions: &[DVec2], holes: &[u32]) -> Vec<u32> {
    let n = positions.len();
    if n < 3 {
        return Vec::new();
    }

    let data: Vec<[f64; 2]> = positions.iter().map(|p| [p.x, p.y]).collect();
    let mut earcut = earcut::Earcut::new();
    let mut triangles: Vec<u32> = Vec::new();
    earcut.earcut(data.iter().copied(), holes, &mut triangles);
    triangles
}

/// 计算一个 2D 多边形的带符号面积。
/// 映射到 `PolygonPipeline.computeArea2D`
///
/// 采用鞋带公式；逆时针为正、顺时针为负，少于 3 顶点时为 0。
pub fn compute_area2d(positions: &[DVec2]) -> f64 {
    let n = positions.len();
    if n < 3 {
        return 0.0;
    }
    // 面积累加（鞋带公式）：逐项叠加 x_i*y_j - x_j*y_i，最后乘 0.5。
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += positions[i].x * positions[j].y;
        area -= positions[j].x * positions[i].y;
    }
    area * 0.5
}

/// 计算一个 2D 多边形的绕序。
/// 映射到 `PolygonPipeline.computeWindingOrder2D`
///
/// 以带符号面积的正负判定：面积 > 0 为逆时针，否则为顺时针。
pub fn compute_winding_order(positions: &[DVec2]) -> WindingOrder {
    if compute_area2d(positions) > 0.0 {
        WindingOrder::CounterClockwise
    } else {
        WindingOrder::Clockwise
    }
}

/// 多边形的绕序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindingOrder {
    /// 顺时针（带符号面积为负）。
    Clockwise,
    /// 逆时针（带符号面积为正）。
    CounterClockwise,
}

// ============================================================================
// GeometryPipeline 函数
// ============================================================================

/// 为三角形几何计算逐顶点法线。
/// 映射到 `GeometryPipeline.computeNormal`
///
/// 先算每个三角形的叉积面法线，再逐顶点累加所邻接的面法线并
/// 归一化；非三角形或空索引时直接返回。
pub fn compute_normal(geo: &mut GeometryData) {
    if geo.primitive_type != PrimitiveType::Triangles || geo.indices.is_empty() {
        return;
    }

    let num_vertices = geo.positions.len();
    let num_triangles = geo.indices.len() / 3;

    // 计算面法线。
    let mut face_normals: Vec<DVec3> = Vec::with_capacity(num_triangles);
    for tri in 0..num_triangles {
        let i0 = geo.indices[tri * 3] as usize;
        let i1 = geo.indices[tri * 3 + 1] as usize;
        let i2 = geo.indices[tri * 3 + 2] as usize;

        let v0 = DVec3::from(geo.positions[i0]);
        let v1 = DVec3::from(geo.positions[i1]);
        let v2 = DVec3::from(geo.positions[i2]);

        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        face_normals.push(edge1.cross(edge2));
    }

    // 逐顶点累加面法线。
    let mut vertex_normals: Vec<DVec3> = vec![DVec3::ZERO; num_vertices];
    for (tri, &fnormal) in face_normals.iter().enumerate() {
        let i0 = geo.indices[tri * 3] as usize;
        let i1 = geo.indices[tri * 3 + 1] as usize;
        let i2 = geo.indices[tri * 3 + 2] as usize;

        vertex_normals[i0] += fnormal;
        vertex_normals[i1] += fnormal;
        vertex_normals[i2] += fnormal;
    }

    // 归一化。每个顶点法线除以长度，退化时退回 +Z。
    let normals: Vec<[f64; 3]> = vertex_normals
        .iter()
        .map(|n| {
            let normalized = n.normalize_or(DVec3::Z);
            [normalized.x, normalized.y, normalized.z]
        })
        .collect();

    geo.normals = Some(normals);
}

/// 为三角形几何计算逐顶点切线和副切线。
/// 映射到 `GeometryPipeline.computeTangentAndBitangent`
///
/// 基于 Eric Lengyel 的《Computing Tangent Space Basis Vectors for an Arbitrary Mesh》。
///
/// # 参数
/// - `geo`：就地写入切线/副切线的几何；需已具备 UV。
///
/// 若缺法线则先调用 `compute_normal`；缺 UV 时直接返回。
pub fn compute_tangent_and_bitangent(geo: &mut GeometryData) {
    if geo.primitive_type != PrimitiveType::Triangles || geo.indices.is_empty() {
        return;
    }

    let normals = match &geo.normals {
        Some(n) => n,
        None => {
            compute_normal(geo);
            geo.normals.as_ref().unwrap()
        }
    };
    let tex_coords = match &geo.tex_coords {
        Some(st) => st,
        None => return, // 计算切线需要 UV。
    };

    let num_vertices = geo.positions.len();
    let num_triangles = geo.indices.len() / 3;

    // 逐三角形累加：tan1 收集切线方向 sdir，tan2 收集副切线方向 tdir。
    let mut tan1: Vec<DVec3> = vec![DVec3::ZERO; num_vertices];
    let mut tan2: Vec<DVec3> = vec![DVec3::ZERO; num_vertices];

    for tri in 0..num_triangles {
        let i0 = geo.indices[tri * 3] as usize;
        let i1 = geo.indices[tri * 3 + 1] as usize;
        let i2 = geo.indices[tri * 3 + 2] as usize;

        let v0 = DVec3::from(geo.positions[i0]);
        let v1 = DVec3::from(geo.positions[i1]);
        let v2 = DVec3::from(geo.positions[i2]);

        let w0 = DVec2::from(tex_coords[i0]);
        let w1 = DVec2::from(tex_coords[i1]);
        let w2 = DVec2::from(tex_coords[i2]);

        // 三角形两条边的笛卡尔分量差（x1,x2,y1,y2,z1,z2）。
        let x1 = v1.x - v0.x;
        let x2 = v2.x - v0.x;
        let y1 = v1.y - v0.y;
        let y2 = v2.y - v0.y;
        let z1 = v1.z - v0.z;
        let z2 = v2.z - v0.z;

        // 对应的 UV 增量（s1,s2,t1,t2），用于求解纹理空间到世界空间的变换。
        let s1 = w1.x - w0.x;
        let s2 = w2.x - w0.x;
        let t1 = w1.y - w0.y;
        let t2 = w2.y - w0.y;

        // UV 三角形的面积分母；接近 0 时视为退化，取 r=0 避免除零。
        let denom = s1 * t2 - s2 * t1;
        let r = if denom.abs() > 1e-10 { 1.0 / denom } else { 0.0 };

        // sdir 沿 +U 方向、tdir 沿 +V 方向的梯度（除以 UV 面积）。
        let sdir = DVec3::new(
            (t2 * x1 - t1 * x2) * r,
            (t2 * y1 - t1 * y2) * r,
            (t2 * z1 - t1 * z2) * r,
        );
        let tdir = DVec3::new(
            (s1 * x2 - s2 * x1) * r,
            (s1 * y2 - s2 * y1) * r,
            (s1 * z2 - s2 * z1) * r,
        );

        tan1[i0] += sdir;
        tan1[i1] += sdir;
        tan1[i2] += sdir;

        tan2[i0] += tdir;
        tan2[i1] += tdir;
        tan2[i2] += tdir;
    }

    let mut tangents: Vec<[f64; 3]> = Vec::with_capacity(num_vertices);
    let mut bitangents: Vec<[f64; 3]> = Vec::with_capacity(num_vertices);

    for i in 0..num_vertices {
        let n = DVec3::from(normals[i]);
        let t = tan1[i];

        // Gram-Schmidt 正交化。
        let tangent = (t - n * n.dot(t)).normalize_or(DVec3::X);
        tangents.push([tangent.x, tangent.y, tangent.z]);

        // 计算手性。
        let bitangent = n.cross(tangent).normalize_or(DVec3::Y);
        bitangents.push([bitangent.x, bitangent.y, bitangent.z]);
    }

    geo.tangents = Some(tangents);
    geo.bitangents = Some(bitangents);
}

/// 将三角形索引转换为线索引（线框）。
/// 映射到 `GeometryPipeline.toWireframe`
///
/// 每个三角形展开为三条边（共 6 个索引），并将拓扑改为 `Lines`。
pub fn to_wireframe(geo: &mut GeometryData) {
    if geo.primitive_type != PrimitiveType::Triangles || geo.indices.is_empty() {
        return;
    }

    let num_triangles = geo.indices.len() / 3;
    // 每个三角形展开为 3 条边，预留 num_triangles*6 个索引空间。
    let mut lines: Vec<u32> = Vec::with_capacity(num_triangles * 6);

    for tri in 0..num_triangles {
        let i0 = geo.indices[tri * 3];
        let i1 = geo.indices[tri * 3 + 1];
        let i2 = geo.indices[tri * 3 + 2];

        lines.extend_from_slice(&[i0, i1, i1, i2, i2, i0]);
    }

    geo.indices = lines;
    geo.primitive_type = PrimitiveType::Lines;
}

/// 使用 GeographicProjection 将 3D 位置投影到 2D。
/// 返回 (position3d, position2d) 数组。
/// 映射到 `GeometryPipeline.projectTo2D`
///
/// 保留原始 3D 坐标的同时，为每个点计算对应的 2D 地图投影坐标（以
/// 经线方向为 x、纬线方向为 y），供贴地几何使用。
///
/// # 参数
/// - `positions`：待投影的 [f64;3] 顶点数组。
/// - `ellipsoid`：参考椭球，同时用于构造地理投影。
///
/// 返回 (原始3d, 投影2d) 两个同长数组。
pub fn project_to_2d(
    positions: &[[f64; 3]],
    ellipsoid: &Ellipsoid,
) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    use crate::projection::MapProjection;
    let projection = crate::projection::GeographicProjection::new(ellipsoid.clone());
    let pos3d = positions.to_vec();
    // pos2d 与 pos3d 同序；无法转大地坐标的点（如中心）置为 (0,0,0)。
    let mut pos2d: Vec<[f64; 3]> = Vec::with_capacity(positions.len());

    for p in positions {
        let cart = ellipsoid.cartesian_to_cartographic(DVec3::from(*p));
        match cart {
            Some(c) => {
                let projected = projection.project(&c);
                pos2d.push([projected.x, projected.y, projected.z]);
            }
            None => {
                pos2d.push([0.0, 0.0, 0.0]);
            }
        }
    }

    (pos3d, pos2d)
}

/// 将一个 f64 值编码为高/低两个 f32 部分，用于 GPU 精度。
/// 映射到 `EncodedCartesian3.encode`
///
/// 将 high（首位）与 low（残差）两个 f32 相加可近似还原原值。
pub fn encode_f64_to_f32_pair(value: f64) -> (f32, f32) {
    // 将高/低两个 f32 相加近似还原原值；high 取首位，low 取残差。
    let high = value as f32;
    let low = (value - high as f64) as f32;
    (high, low)
}

/// 将一个位置属性（[f64;3] 数组）编码为高/低 f32 对。
/// 映射到 `GeometryPipeline.encodeAttribute`
///
/// 逐顶点调用 `encode_f64_to_f32_pair`，得到 high/low 两个平行数组。
///
/// # 参数
/// - `positions`：待编码的 [f64;3] 顶点数组。
///
/// 返回 (high, low)，两者与输入同长。
pub fn encode_attribute(
    positions: &[[f64; 3]],
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    // 逐顶点将 x/y/z 三分量各自拆为 high/low，打包为两个 [f32;3] 数组。
    let mut high: Vec<[f32; 3]> = Vec::with_capacity(positions.len());
    let mut low: Vec<[f32; 3]> = Vec::with_capacity(positions.len());

    for p in positions {
        let (hx, lx) = encode_f64_to_f32_pair(p[0]);
        let (hy, ly) = encode_f64_to_f32_pair(p[1]);
        let (hz, lz) = encode_f64_to_f32_pair(p[2]);
        high.push([hx, hy, hz]);
        low.push([lx, ly, lz]);
    }

    (high, low)
}

/// 用一个模型矩阵变换几何的位置和法线。
/// 映射到 `GeometryPipeline.transformToWorldCoordinates`
///
/// 位置用完整 4x4 变换，法线用 3x3 逆转置以保持垂直性；变换后重算包围球。
///
/// # 参数
/// - `geo`：就地修改的几何（位置、法线、包围球）。
/// - `model_matrix`：将局部坐标变换到世界坐标的 4x4 矩阵。
pub fn transform_to_world_coordinates(
    geo: &mut GeometryData,
    model_matrix: &glam::DMat4,
) {
    // 法线矩阵为模型矩阵 3x3 部分的逆转置；不可逆时退回原 3x3。
    let normal_matrix = {
        let m3 = glam::DMat3::from_cols(
            model_matrix.x_axis.truncate(),
            model_matrix.y_axis.truncate(),
            model_matrix.z_axis.truncate(),
        );
        let det = m3.determinant();
        if det.abs() > 1e-10 {
            m3.inverse().transpose()
        } else {
            m3
        }
    };

    for p in geo.positions.iter_mut() {
        let v = model_matrix.transform_point3(DVec3::from(*p));
        *p = [v.x, v.y, v.z];
    }

    if let Some(ref mut normals) = geo.normals {
        for n in normals.iter_mut() {
            let v = normal_matrix * DVec3::from(*n);
            let normalized = v.normalize_or(DVec3::ZERO);
            *n = [normalized.x, normalized.y, normalized.z];
        }
    }

    // 更新包围球
    if !geo.positions.is_empty() {
        let mut center = DVec3::ZERO;
        for p in &geo.positions {
            center += DVec3::from(*p);
        }
        center /= geo.positions.len() as f64;
        let mut max_dist_sq = 0.0_f64;
        for p in &geo.positions {
            let d = (DVec3::from(*p) - center).length_squared();
            if d > max_dist_sq {
                max_dist_sq = d;
            }
        }
        geo.bounding_sphere = BoundingSphere::new(center, max_dist_sq.sqrt());
    }
}

/// 用八面体编码压缩顶点法线，并与纹理坐标一起打包。
/// 映射到 `GeometryPipeline.compressVertices`
///
/// 需存在法线；无 ST 时每顶点输出一个 u32（八面体 xy），有 ST 时额外
/// 输出一个打包的 ST u32。缺法线时返回 None。
///
/// # 参数
/// - `geo`：包含法线（必需）与可选 ST 的几何。
pub fn compress_vertices(geo: &GeometryData) -> Option<Vec<u32>> {
    let normals = geo.normals.as_ref()?;
    let num_vertices = normals.len();

    // 打包压缩后的法线（每顶点 2 个 u16）+ 可选的 ST（每顶点 2 个 u16）
    // 无 ST 时每顶点 2 个 u16（一个 u32），有 ST 时 4 个 u16（两个 u32）。
    let has_st = geo.tex_coords.is_some();
    let components_per_vertex = if has_st { 4 } else { 2 };
    let mut compressed: Vec<u32> = Vec::with_capacity(num_vertices * components_per_vertex / 2 + 1);

    let st = geo.tex_coords.as_ref();

    for i in 0..num_vertices {
        let n = DVec3::from(normals[i]);
        // 将法线八面体编码为 2 字节
        let oct = crate::attribute_compression::oct_encode(n);
        let oct_x = (oct.x.round() as u32) & 0xFFFF;
        let oct_y = (oct.y.round() as u32) & 0xFFFF;

        if let Some(sts) = st {
            let s = (sts[i][0].clamp(0.0, 1.0) * 65535.0).round() as u32;
            let t = (sts[i][1].clamp(0.0, 1.0) * 65535.0).round() as u32;
            // 打包：[normal_xy(u32), st(u32)]
            let normal_packed = oct_x | (oct_y << 16);
            let st_packed = s | (t << 16);
            compressed.push(normal_packed);
            compressed.push(st_packed);
        } else {
            let normal_packed = oct_x | (oct_y << 16);
            compressed.push(normal_packed);
        }
    }

    Some(compressed)
}

/// 为向量属性（例如法线可视化）创建线段。
/// 映射到 `GeometryPipeline.createLineSegmentsForVectors`
///
/// 从每个位置出发，沿对应向量延伸 `length` 长度，成对发射顶点构成线段。
///
/// # 参数
/// - `positions`：线段起点数组。
/// - `vectors`：与起点一一对应的方向向量。
/// - `length`：统一的线段延伸长度。
pub fn create_line_segments_for_vectors(
    positions: &[[f64; 3]],
    vectors: &[[f64; 3]],
    length: f64,
) -> GeometryData {
    let mut line_positions: Vec<[f64; 3]> = Vec::with_capacity(positions.len() * 2);

    for i in 0..positions.len() {
        let p = DVec3::from(positions[i]);
        let v = DVec3::from(vectors[i]) * length;
        line_positions.push(positions[i]);
        let end = p + v;
        line_positions.push([end.x, end.y, end.z]);
    }

    let mut indices: Vec<u32> = Vec::with_capacity(positions.len() * 2);
    for i in 0..positions.len() as u32 {
        indices.push(i * 2);
        indices.push(i * 2 + 1);
    }

    // 包围球：中心与输入相同，半径 + length
    let mut center = DVec3::ZERO;
    for p in positions {
        center += DVec3::from(*p);
    }
    // 先取顶点均值作为中心，再取最大顶点距离平方作为半径基准。
    if !positions.is_empty() {
        center /= positions.len() as f64;
    }
    let mut max_dist_sq = 0.0_f64;
    for p in positions {
        let d = (DVec3::from(*p) - center).length_squared();
        if d > max_dist_sq {
            max_dist_sq = d;
        }
    }
    let radius = max_dist_sq.sqrt() + length;

    GeometryData {
        positions: line_positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere: BoundingSphere::new(center, radius),
        primitive_type: PrimitiveType::Lines,
    }
}

/// 重新排序几何的索引和属性，以优化顶点前置缓存。
/// 映射到 `GeometryPipeline.reorderForPreVertexCache`
///
/// 按索引首次出现顺序重新编号顶点，丢弃未引用顶点，并同步压缩各属性数组。
///
/// # 参数
/// - `geo`：就地修改的几何；空索引时直接返回。
pub fn reorder_for_pre_vertex_cache(geo: &mut GeometryData) {
    if geo.indices.is_empty() {
        return;
    }

    let num_vertices = geo.positions.len();
    let mut used: Vec<bool> = vec![false; num_vertices];
    let mut remap: Vec<Option<u32>> = vec![None; num_vertices];
    let mut new_index: u32 = 0;

    // 第一遍：确定哪些顶点被使用，并分配新索引
    for &idx in &geo.indices {
        let i = idx as usize;
        if i < num_vertices && !used[i] {
            used[i] = true;
            remap[i] = Some(new_index);
            new_index += 1;
        }
    }

    // 未被任何索引引用的顶点会被丢弃，因此属性数组与索引同步压缩。
    // 重映射索引
    let new_indices: Vec<u32> = geo.indices.iter().map(|&idx| {
        remap[idx as usize].unwrap_or(0)
    }).collect();
    geo.indices = new_indices;

    // 压缩属性
    let used_indices: Vec<usize> = (0..num_vertices)
        .filter(|&i| used[i])
        .collect();

    geo.positions = used_indices.iter().map(|&i| geo.positions[i]).collect();
    if let Some(ref mut normals) = geo.normals {
        *normals = used_indices.iter().map(|&i| normals[i]).collect();
    }
    if let Some(ref mut st) = geo.tex_coords {
        *st = used_indices.iter().map(|&i| st[i]).collect();
    }
    if let Some(ref mut tangents) = geo.tangents {
        *tangents = used_indices.iter().map(|&i| tangents[i]).collect();
    }
    if let Some(ref mut bitangents) = geo.bitangents {
        *bitangents = used_indices.iter().map(|&i| bitangents[i]).collect();
    }
}

/// 将几何拆分为多个能容纳在 u16 索引（最多 65536 个顶点）内的几何。
/// 映射到 `GeometryPipeline.fitToUnsignedShortIndices`
///
/// # 参数
/// - `geo`：顶点数可能超过 65536 的几何。
///
/// 返回一组拆分后的几何，每个都不超过 u16 索引上限。
pub fn fit_to_unsigned_short_indices(geo: &GeometryData) -> Vec<GeometryData> {
    // 以每个图元的顶点数（三角形 3、线 2）为步长扫描原索引，累积到当前批次。
    const MAX_VERTICES: usize = 65536;
    let num_vertices = geo.positions.len();

    if num_vertices <= MAX_VERTICES {
        return vec![geo.clone()];
    }

    let vertices_per_primitive = match geo.primitive_type {
        PrimitiveType::Triangles => 3,
        PrimitiveType::Lines => 2,
    };

    let mut result: Vec<GeometryData> = Vec::new();
    let mut current_vertices: Vec<[f64; 3]> = Vec::new();
    let mut current_normals: Vec<[f64; 3]> = Vec::new();
    let mut current_st: Vec<[f64; 2]> = Vec::new();
    let mut current_indices: Vec<u32> = Vec::new();
    let mut vertex_map: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();

    let num_primitives = geo.indices.len() / vertices_per_primitive;

    for prim in 0..num_primitives {
        let base = prim * vertices_per_primitive;

        // 检查添加此图元是否会超出限制
        let mut new_vertices_needed = 0;
        for k in 0..vertices_per_primitive {
            let old_idx = geo.indices[base + k] as usize;
            if !vertex_map.contains_key(&old_idx) {
                new_vertices_needed += 1;
            }
        }

        if current_vertices.len() + new_vertices_needed > MAX_VERTICES && !current_vertices.is_empty() {
            // 刷写当前批次
            result.push(GeometryData {
                positions: std::mem::take(&mut current_vertices),
                normals: if geo.normals.is_some() { Some(std::mem::take(&mut current_normals)) } else { None },
                tex_coords: if geo.tex_coords.is_some() { Some(std::mem::take(&mut current_st)) } else { None },
                tangents: None,
                bitangents: None,
                indices: std::mem::take(&mut current_indices),
                bounding_sphere: geo.bounding_sphere.clone(),
                primitive_type: geo.primitive_type,
            });
            vertex_map.clear();
        }

        // 添加顶点和索引
        for k in 0..vertices_per_primitive {
            let old_idx = geo.indices[base + k] as usize;
            let new_idx = *vertex_map.entry(old_idx).or_insert_with(|| {
                let idx = current_vertices.len() as u32;
                current_vertices.push(geo.positions[old_idx]);
                if let Some(ref normals) = geo.normals {
                    current_normals.push(normals[old_idx]);
                }
                if let Some(ref st) = geo.tex_coords {
                    current_st.push(st[old_idx]);
                }
                idx
            });
            current_indices.push(new_idx);
        }
    }

    // 刷写剩余部分
    if !current_vertices.is_empty() {
        result.push(GeometryData {
            positions: current_vertices,
            normals: if geo.normals.is_some() { Some(current_normals) } else { None },
            tex_coords: if geo.tex_coords.is_some() { Some(current_st) } else { None },
            tangents: None,
            bitangents: None,
            indices: current_indices,
            bounding_sphere: geo.bounding_sphere.clone(),
            primitive_type: geo.primitive_type,
        });
    }

    result
}

/// 拆分穿越国际日期变更线（经度 ±π）的几何。
/// 返回拆分后的几何（西/东两半）。
/// 映射到 `GeometryPipeline.splitLongitude`
///
/// 这是简化版：按三角形多数投票分到东/西两侧，未在 IDL 处插值。
///
/// # 参数
/// - `geo`：待拆分的三角形几何。
/// - `ellipsoid`：用于将顶点回转为大地坐标的椭球。
pub fn split_longitude(geo: &GeometryData, ellipsoid: &Ellipsoid) -> Vec<GeometryData> {
    if geo.positions.is_empty() || geo.primitive_type != PrimitiveType::Triangles {
        return vec![geo.clone()];
    }

    // 将位置转换为测绘坐标，并检查是否有穿越 IDL 的
    // 同时逐顶点标记是否到达远东西两侧（|lon| > 90°）。
    let cartos: Vec<Option<crate::cartographic::Cartographic>> = geo.positions.iter()
        .map(|p| ellipsoid.cartesian_to_cartographic(DVec3::from(*p)))
        .collect();

    // 检查几何是否穿越 IDL：
    // 1. 某个三角形的顶点经度差 > PI，或
    // 2. 几何在 IDL 两侧都有顶点（lon > PI/2 且 lon < -PI/2）
    let mut crosses_idl = false;
    let mut has_far_east = false;  // 经度 > PI/2（90°E）
    let mut has_far_west = false;  // 经度 < -PI/2（90°W）

    for c in cartos.iter().flatten() {
        if c.longitude > std::f64::consts::FRAC_PI_2 {
            has_far_east = true;
        }
        if c.longitude < -std::f64::consts::FRAC_PI_2 {
            has_far_west = true;
        }
    }
    if has_far_east && has_far_west {
        crosses_idl = true;
    }

    // 还检查三角形内部的大幅度经度跳变
    if !crosses_idl {
        for tri in 0..geo.indices.len() / 3 {
            let i0 = geo.indices[tri * 3] as usize;
            let i1 = geo.indices[tri * 3 + 1] as usize;
            let i2 = geo.indices[tri * 3 + 2] as usize;

            if let (Some(c0), Some(c1), Some(c2)) = (&cartos[i0], &cartos[i1], &cartos[i2]) {
                let lons = [c0.longitude, c1.longitude, c2.longitude];
                for a in 0..3 {
                    for b in (a+1)..3 {
                        if (lons[a] - lons[b]).abs() > std::f64::consts::PI {
                            crosses_idl = true;
                            break;
                        }
                    }
                    if crosses_idl { break; }
                }
            }
            if crosses_idl { break; }
        }
    }

    if !crosses_idl {
        return vec![geo.clone()];
    }

    // 对于穿越 IDL 的几何，拆分为东（止）和西（负）两部分
    // 这是一个简化版本 - 完整的 CesiumJS 实现会在 IDL 处进行插值
    let mut east_positions: Vec<[f64; 3]> = Vec::new();
    let mut west_positions: Vec<[f64; 3]> = Vec::new();
    let mut east_indices: Vec<u32> = Vec::new();
    let mut west_indices: Vec<u32> = Vec::new();
    let mut east_map: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    let mut west_map: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();

    for tri in 0..geo.indices.len() / 3 {
        let i0 = geo.indices[tri * 3] as usize;
        let i1 = geo.indices[tri * 3 + 1] as usize;
        let i2 = geo.indices[tri * 3 + 2] as usize;

        // 确定此三角形属于哪一侧（多数投票）
        let mut east_count = 0;
        let mut west_count = 0;
        for &idx in &[i0, i1, i2] {
            if let Some(Some(c)) = cartos.get(idx) {
                if c.longitude >= 0.0 {
                    east_count += 1;
                } else {
                    west_count += 1;
                }
            }
        }

        if east_count >= west_count {
            // 归入东侧
            for &idx in &[i0, i1, i2] {
                let new_idx = *east_map.entry(idx).or_insert_with(|| {
                    let i = east_positions.len() as u32;
                    east_positions.push(geo.positions[idx]);
                    i
                });
                east_indices.push(new_idx);
            }
        } else {
            // 归入西侧
            for &idx in &[i0, i1, i2] {
                let new_idx = *west_map.entry(idx).or_insert_with(|| {
                    let i = west_positions.len() as u32;
                    west_positions.push(geo.positions[idx]);
                    i
                });
                west_indices.push(new_idx);
            }
        }
    }

    let mut result = Vec::new();
    if !east_positions.is_empty() {
        result.push(GeometryData {
            positions: east_positions,
            normals: None,
            tex_coords: None,
            tangents: None,
            bitangents: None,
            indices: east_indices,
            bounding_sphere: geo.bounding_sphere.clone(),
            primitive_type: PrimitiveType::Triangles,
        });
    }
    if !west_positions.is_empty() {
        result.push(GeometryData {
            positions: west_positions,
            normals: None,
            tex_coords: None,
            tangents: None,
            bitangents: None,
            indices: west_indices,
            bounding_sphere: geo.bounding_sphere.clone(),
            primitive_type: PrimitiveType::Triangles,
        });
    }

    if result.is_empty() {
        vec![geo.clone()]
    } else {
        result
    }
}

/// 通过拼接属性、拼接并调整索引、以及创建一个
/// 包含所有输入的统一包围球，将多个几何合并为一个。
///
/// 若这些几何并非都共享某个可选属性（法线、
/// 纹理坐标、切线、副切线），则该属性会从结果中丢弃。
/// 仅当每个输入几何都有非空索引列表时才合并索引；否则结果没有索引。
///
/// 映射到 CesiumJS `GeometryPipeline.combineInstances` / `combineGeometries`。
/// 仅当这些几何都共享某个可选属性时才合并；否则丢弃该属性。
///
/// # 参数
/// - `geometries`：待合并的几何列表（至少一个，且图元类型一致）。
pub fn combine_geometries(geometries: &[GeometryData]) -> GeometryData {
    assert!(
        !geometries.is_empty(),
        "geometries must have length greater than zero"
    );

    let primitive_type = geometries[0].primitive_type;
    let have_indices = !geometries[0].indices.is_empty();

    for geo in &geometries[1..] {
        assert_eq!(
            geo.primitive_type, primitive_type,
            "All geometries must have the same primitiveType."
        );
        assert_eq!(
            !geo.indices.is_empty(),
            have_indices,
            "All geometries must have an indices or not have one."
        );
    }

    // 合并位置。
    // 位置始终存在，直接将各几何的顶点拼接为一个长数组。
    let mut positions: Vec<[f64; 3]> = Vec::new();
    for geo in geometries {
        positions.extend_from_slice(&geo.positions);
    }

    // 仅当某个可选属性存在于所有几何中时才合并它。
    // 否则若任一几何缺该属性，结果中就不保留它（避免长度不一致）。
    let all_have = |f: fn(&GeometryData) -> &Option<Vec<[f64; 3]>>| -> bool {
        geometries.iter().all(|g| f(g).is_some())
    };

    let normals = if all_have(|g| &g.normals) {
        let mut v = Vec::new();
        for geo in geometries {
            v.extend_from_slice(geo.normals.as_ref().unwrap());
        }
        Some(v)
    } else {
        None
    };

    // 切线/副切线合并与法线同构：均依赖 `all_have` 判定后拼接。
    let tangents = if all_have(|g| &g.tangents) {
        let mut v = Vec::new();
        for geo in geometries {
            v.extend_from_slice(geo.tangents.as_ref().unwrap());
        }
        Some(v)
    } else {
        None
    };

    let bitangents = if all_have(|g| &g.bitangents) {
        let mut v = Vec::new();
        for geo in geometries {
            v.extend_from_slice(geo.bitangents.as_ref().unwrap());
        }
        Some(v)
    } else {
        None
    };

    // 纹理坐标为 [f64;2]，类型不同于法线，故单独用 all 判定。
    let tex_coords = if geometries.iter().all(|g| g.tex_coords.is_some()) {
        let mut v = Vec::new();
        for geo in geometries {
            v.extend_from_slice(geo.tex_coords.as_ref().unwrap());
        }
        Some(v)
    } else {
        None
    };

    // 逐几何扫描：将当前几何的索引加上已累计的顶点偏移，再接到目标。
    let indices = if have_indices {
        let mut dest: Vec<u32> = Vec::new();
        let mut offset: u32 = 0;
        for geo in geometries {
            for &idx in &geo.indices {
                dest.push(offset + idx);
            }
            offset += geo.positions.len() as u32;
        }
        dest
    } else {
        Vec::new()
    };

    // 创建一个包含所有几何的包围球。
    // 逐个合并相邻包围球，得到能容纳全部输入的球体。
    let mut bounding_sphere = geometries[0].bounding_sphere.clone();
    for geo in &geometries[1..] {
        bounding_sphere = bounding_sphere.union(&geo.bounding_sphere);
    }

    GeometryData {
        positions,
        normals,
        tex_coords,
        tangents,
        bitangents,
        indices,
        bounding_sphere,
        primitive_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证椭球几何的顶点数为 (stacks+1)*(slices+1)，索引数为 stacks*slices*6。
    #[test]
    fn test_ellipsoid_geometry_vertex_count() {
        let geo = ellipsoid_geometry(DVec3::splat(1.0), 16, 32, VertexFormat::ALL);
        // (stacks+1) * (slices+1) 个顶点
        assert_eq!(geo.positions.len(), 17 * 33);
        assert!(geo.normals.is_some());
        assert!(geo.tex_coords.is_some());
        // stacks * slices * 6 个索引
        assert_eq!(geo.indices.len(), 16 * 32 * 6);
    }

    /// 验证球体几何复用椭球生成器且包围球半径等于传入半径。
    #[test]
    fn test_sphere_geometry() {
        let geo = sphere_geometry(5.0, 8, 16, VertexFormat::POSITION_ONLY);
        assert_eq!(geo.positions.len(), 9 * 17);
        assert!(geo.normals.is_none());
        assert!((geo.bounding_sphere.radius - 5.0).abs() < 1e-10);
    }

    /// 验证盒子几何恰有 24 个顶点与 36 个索引（6 面各自独立）。
    #[test]
    fn test_box_geometry() {
        let geo = box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::ALL);
        assert_eq!(geo.positions.len(), 24); // 6 个面 * 4 个顶点
        assert_eq!(geo.indices.len(), 36); // 6 个面 * 2 个三角形 * 3
    }

    /// 验证平面几何为 4 顶点、6 索引的单位四边形。
    #[test]
    fn test_plane_geometry() {
        let geo = plane_geometry(VertexFormat::ALL);
        assert_eq!(geo.positions.len(), 4);
        assert_eq!(geo.indices.len(), 6);
    }

    /// 验证矩形几何按 granularity 细分产生多于 4 个顶点且带法线。
    #[test]
    fn test_rectangle_geometry() {
        let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
        let geo = rectangle_geometry(&rect, &Ellipsoid::WGS84, math_utils::to_radians(1.0), 0.0, VertexFormat::ALL);
        assert!(geo.positions.len() > 4);
        assert!(geo.normals.is_some());
    }

    /// 验证大圆弧细分在两点之间插入了中间点。
    #[test]
    fn test_generate_arc() {
        let ellipsoid = Ellipsoid::WGS84;
        let start = ellipsoid.cartographic_to_cartesian(&crate::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
        let end = ellipsoid.cartographic_to_cartesian(&crate::cartographic::Cartographic::from_degrees(10.0, 0.0, 0.0));
        let arc = generate_arc(&[start, end], math_utils::to_radians(1.0), &ellipsoid);
        assert!(arc.len() > 2); // 应包含中间点
    }

    /// 验证四边形经 earcut 三角剖分后得到 2 个三角形（6 索引）。
    #[test]
    fn test_triangulate_polygon() {
        let positions = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
        ];
        let indices = triangulate_polygon(&positions, &[]);
        assert_eq!(indices.len(), 6); // 一个四边形对应 2 个三角形
    }

    /// 验证单位正方形的带符号面积为 1.0。
    #[test]
    fn test_compute_area2d() {
        let positions = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
        ];
        let area = compute_area2d(&positions);
        assert!((area - 1.0).abs() < 1e-10);
    }

    /// 验证给定顶点序的多边形被判定为逆时针绕序。
    #[test]
    fn test_winding_order() {
        let ccw = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(0.0, 1.0),
        ];
        assert_eq!(compute_winding_order(&ccw), WindingOrder::CounterClockwise);
    }

    /// 验证盒子轮廓几何有 8 角点与 12 条边（24 索引），拓扑为线。
    #[test]
    fn test_box_outline() {
        let geo = box_outline_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0));
        assert_eq!(geo.positions.len(), 8);
        assert_eq!(geo.indices.len(), 24); // 12 条边 * 2
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    }

    /// 验证椭球轮廓几何由三个大圆组成且索引成对（线集）。
    #[test]
    fn test_ellipsoid_outline() {
        let geo = ellipsoid_outline_geometry(DVec3::new(1.0, 2.0, 3.0), 16, 32);
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.indices.len() % 2, 0);
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    }

    /// 验证圆形轮廓几何生成非空顶点且索引成对并闭合成环。
    #[test]
    fn test_circle_outline() {
        let ell = Ellipsoid::WGS84;
        let center = ell.cartographic_to_cartesian(&crate::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0));
        let geo = circle_outline_geometry(center, 100_000.0, &ell, math_utils::to_radians(1.0));
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.indices.len() % 2, 0);
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    }

    /// 验证矩形轮廓几何沿四条边采样为线集。
    #[test]
    fn test_rectangle_outline() {
        let ell = Ellipsoid::WGS84;
        let rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
        let geo = rectangle_outline_geometry(&rect, &ell, math_utils::to_radians(1.0));
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.indices.len() % 2, 0);
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    }

    /// 验证圆柱轮廓几何含顶/底圆与垂直边，拓扑为线。
    #[test]
    fn test_cylinder_outline() {
        let geo = cylinder_outline_geometry(2.0, 1.0, 1.0, 16);
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.indices.len() % 2, 0);
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    }

    /// 验证平面轮廓几何为 4 顶点、4 条边（8 索引）。
    #[test]
    fn test_plane_outline() {
        let geo = plane_outline_geometry();
        assert_eq!(geo.positions.len(), 4);
        assert_eq!(geo.indices.len(), 8); // 4 条边 * 2
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    }

    /// 验证 compute_normal 为逐顶点生成单位长度法线。
    #[test]
    fn test_compute_normal() {
        let mut geo = box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::POSITION_ONLY);
        assert!(geo.normals.is_none());
        compute_normal(&mut geo);
        assert!(geo.normals.is_some());
        let normals = geo.normals.unwrap();
        assert_eq!(normals.len(), geo.positions.len());
        // 所有法线都应为单位长度。
        for n in &normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-6);
        }
    }

    /// 验证切线/副切线计算产出与顶点数相同的数组。
    #[test]
    fn test_compute_tangent_and_bitangent() {
        let mut geo = box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::ALL);
        // box_geometry 不计算切线，因此由我们来计算。
        assert!(geo.tangents.is_none());
        compute_tangent_and_bitangent(&mut geo);
        assert!(geo.tangents.is_some());
        assert!(geo.bitangents.is_some());
        let tangents = geo.tangents.unwrap();
        assert_eq!(tangents.len(), geo.positions.len());
    }

    /// 验证三角索引转线框后每个三角形生成 3 条边（6 索引）。
    #[test]
    fn test_to_wireframe() {
        let mut geo = box_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0), VertexFormat::POSITION_ONLY);
        assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
        let tri_count = geo.indices.len() / 3;
        to_wireframe(&mut geo);
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
        assert_eq!(geo.indices.len(), tri_count * 6); // 每个三角形 -> 3 条边 -> 6 个索引
    }
}
