//! 逐瓦片椭球网格生成。
//!
//! 实现 CesiumJS `HeightmapTessellator.computeVertices` 方法：
//! Web Mercator 划分方案中的每个瓦片 (x, y, z) 在 WGS84 椭球面上
//! 获得各自的网格片块，UV 坐标在该瓦片的地理范围内
//! 归一化到 [0,1]。

use bevy::prelude::*;
use cesium_bevy_render::METERS_PER_RENDER_UNIT;

/// WGS84 半长轴（米）。
const EARTH_RADIUS: f64 = 6378137.0;
/// WGS84 半短轴（米）。
const EARTH_RADIUS_MINOR: f64 = 6356752.314245;
/// WGS84 椭球的第一偏心率平方。
const E2: f64 =
    1.0 - (EARTH_RADIUS_MINOR * EARTH_RADIUS_MINOR) / (EARTH_RADIUS * EARTH_RADIUS);

/// 通过划分方案坐标标识地球瓦片实体的组件。
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobeTile {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// 以弧度为单位的地理矩形（west、south、east、north）。
#[derive(Debug, Clone, Copy)]
pub struct GeoRectangle {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

/// 计算 Web Mercator 瓦片的地理范围。
///
/// 计算方式（墨卡托投影反算）：
/// - 全局墨卡托范围：两轴均为 [-PI*R, PI*R]
/// - 瓦片宽/高（米）= 2*PI*R / 2^z
/// - 反投影：lon = x_m / R, lat = PI/2 - 2*atan(exp(-y_m / R))
pub fn tile_xy_to_rectangle(x: u32, y: u32, z: u32) -> GeoRectangle {
    let num_tiles = 1u64 << z;
    let tile_size_meters = 2.0 * std::f64::consts::PI * EARTH_RADIUS / num_tiles as f64;

    // 以米为单位的原生矩形（Web Mercator 坐标）
    let west_m = -std::f64::consts::PI * EARTH_RADIUS + x as f64 * tile_size_meters;
    let east_m = west_m + tile_size_meters;
    let north_m = std::f64::consts::PI * EARTH_RADIUS - y as f64 * tile_size_meters;
    let south_m = north_m - tile_size_meters;

    // 从 Web Mercator 米反投影为地理弧度
    let one_over_r = 1.0 / EARTH_RADIUS;
    let west = west_m * one_over_r;
    let east = east_m * one_over_r;
    let north = std::f64::consts::FRAC_PI_2 - 2.0 * (-north_m * one_over_r).exp().atan();
    let south = std::f64::consts::FRAC_PI_2 - 2.0 * (-south_m * one_over_r).exp().atan();

    GeoRectangle {
        west,
        south,
        east,
        north,
    }
}

/// 层级 `z` 瓦片悬挂裙边环的径向下降因子。
/// 裙边顶点恰好位于瓦片边界（与边缘行有相同的 lat/lon，
/// 因此相邻瓦片共享逐位相同的边缘顶点），但位于表面半径的此比例处：
/// 一面悬挂于表面之下的垂直墙，
/// CesiumJS `EllipsoidTessellator` 风格。LOD 层级在近于相同半径处
/// 渲染（REPLACE 细化 + 经实体缩放的相机自适应径向收拢），
/// 因此墙仅填充亚像素光栅化裂缝以及
/// LOD 边界处的弦-矢高折痕；深度以一定余量覆盖最粗
/// 活跃层级的矢高，同时保持为瓦片尺寸的
/// 一小部分。
fn skirt_drop(z: u32) -> f64 {
    1.0 - 3.0 * tuck_step(z)
}

/// 按层级缩放的径向步长：该层级瓦片弧长的 15%，钳制以保证粗层级
/// 保留可用裙边、深层级永不退化。下限
/// 必须超过最大运行时 LOD 落差：`adaptive_tuck_step` 上限为
/// 每层级 1.5e-4，而快速缩放/拖拽会使活跃相邻瓦片最多相差 ~4 层级
///（落差 ~6e-4）；更浅的裙边墙会在 LOD 边界留下
/// 可看穿的裂缝，在快速运动时表现为细条纹。
/// 既然 LOD 收拢已经实体缩放应用，此值仅驱动 [`skirt_drop`]。
fn tuck_step(z: u32) -> f64 {
    let arc = 2.0 * std::f64::consts::PI / (1u64 << z.min(24)) as f64;
    (arc * 0.15).clamp(5.0e-4, 6.0e-4)
}

/// 为 WGS84 椭球面上的单个瓦片生成 Bevy Mesh。
///
/// 遵循 CesiumJS `HeightmapTessellator.computeVertices`：
/// - 顶点分布于瓦片的地理范围内
/// - UV：u = (lon - west) / (east - west)，v = (lat - south) / (north - south)
/// - 位置：WGS84 椭球上的 cartographic_to_cartesian
/// - 法线：大地表面法线（椭球上归一化的位置）
/// - 三角形绕序：从外侧看为逆时针（外向法线）
/// - 一个悬挂裙边环（边界环在 `SKIRT_DROP` 半径处复制）
///   填充相邻瓦片间的光栅化裂缝，
///   绝不覆盖相邻瓦片的表面
///
/// 网格建于精确的椭球半径（无径向偏移）：
/// 每个瓦片——无论粗细——都位于其真实位置，
/// 完全类似 CesiumJS 地形网格。重叠/z-fighting 不可能，因为
/// 渲染划分（`sync_visibility`）从不将父瓦片与其子瓦片
/// 一起绘制，因此无需径向收拢，也不会形成
/// LOD 边界鳍片。
///
/// # 参数
/// * `x`、`y`、`z` - Web Mercator 划分方案中的瓦片坐标
/// * `segments` - 真实瓦片范围内每轴的细分数量
pub fn create_tile_mesh(x: u32, y: u32, z: u32, segments: u32) -> Mesh {
    create_tile_mesh_uv(x, y, z, segments, [0.0, 0.0, 1.0, 1.0])
}

/// 与 [`create_tile_mesh`] 相同，但将 UV 重映射到 `uv_rect` =
/// [u0, v0, u1, v1]，即纹理的一个子矩形。供继承祖先影像的
/// 无数据瓦片使用：子瓦片恰好采样祖先纹理中
/// 属于自己的象限区域（CesiumJS 风格的上采样
/// 回退），因此无影像区域可与真实覆盖无缝融合。
pub fn create_tile_mesh_uv(
    x: u32,
    y: u32,
    z: u32,
    segments: u32,
    uv_rect: [f32; 4],
) -> Mesh {
    let rect = tile_xy_to_rectangle(x, y, z);

    let width = rect.east - rect.west;
    let height = rect.north - rect.south;

    // 精确 [0,1] 网格：相邻瓦片共享逐位相同的边缘顶点
    //（相同 lat/lon 公式 -> 相同 f32 位置），因此相邻瓦片
    // 无重叠地相接；下方的悬挂裙边环填充
    // 剩余的亚像素裂缝。
    let verts_per_side = segments + 1;
    let grid_count = (verts_per_side * verts_per_side) as usize;
    let perimeter_count = (4 * segments) as usize;

    let ring_verts = 2 * perimeter_count;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(grid_count + ring_verts);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(grid_count + ring_verts);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(grid_count + ring_verts);

    for row in 0..verts_per_side {
        // v_norm 精确跨越 [0,1]：真实瓦片范围，无重叠。
        let v_norm = row as f64 / (verts_per_side - 1) as f64;
        let lat = rect.south + v_norm * height;

        let cos_lat = lat.cos();
        let sin_lat = lat.sin();

        for col in 0..verts_per_side {
            let u_norm = col as f64 / (verts_per_side - 1) as f64;
            let lon = rect.west + u_norm * width;

            let cos_lon = lon.cos();
            let sin_lon = lon.sin();

            // 大地表面法线方向
            let nx = cos_lat * cos_lon;
            let ny = cos_lat * sin_lon;
            let nz = sin_lat;

            // 椭球表面上的位置（大地坐标转直角坐标）：
            // N = a / sqrt(1 - e2*sin^2(lat)); x = N*cos(lat)*cos(lon) 等。
            let n_val = EARTH_RADIUS / (1.0 - E2 * sin_lat * sin_lat).sqrt();

            positions.push([
                (n_val * nx) as f32,
                (n_val * ny) as f32,
                (n_val * (1.0 - E2) * sin_lat) as f32,
            ]);
            normals.push([nx as f32, ny as f32, nz as f32]);
            // Bevy 在图像顶部采样 UV v=0（对地图瓦片而言行 0 = 北
            // 侧）。
            let u = u_norm as f32;
            let v = (1.0 - v_norm) as f32;
            uvs.push([
                uv_rect[0] + u * (uv_rect[2] - uv_rect[0]),
                uv_rect[1] + v * (uv_rect[3] - uv_rect[1]),
            ]);
        }
    }

    // 悬挂裙边：在按层级缩放的下降半径处复制边界环。墙
    // 从瓦片边缘垂直向下悬挂，因此只可能从裂缝中露出，绝不会
    // 盖过相邻瓦片。下降量随瓦片奇偶
    // 轻微交替，使同级相邻瓦片重合的墙
    // 不会在裂缝中 z-fight。
    let base_drop = skirt_drop(z);
    let drop = if (x + y + z) & 1 == 0 {
        base_drop
    } else {
        base_drop - 0.1 * tuck_step(z)
    };
    let mut perim: Vec<u32> = Vec::with_capacity(perimeter_count);
    // 裙边墙的每边恒定 uv：墙只会被看到于
    // 亚像素裂缝中，因此沿它的任何 uv 变化都会把边缘列横向
    // 挤压成条纹鳍片。因此两堵墙环都
    // 携带单个边缘中点纹素，使暴露的墙读作普通的
    // 边缘延续。
    let mid_u = |u: f32, v: f32| {
        [
            uv_rect[0] + u * (uv_rect[2] - uv_rect[0]),
            uv_rect[1] + v * (uv_rect[3] - uv_rect[1]),
        ]
    };
    let south_uv = mid_u(0.5, 1.0);
    let east_uv = mid_u(1.0, 0.5);
    let north_uv = mid_u(0.5, 0.0);
    let west_uv = mid_u(0.0, 0.5);
    let mut perim_uv: Vec<[f32; 2]> = Vec::with_capacity(perimeter_count);
    let last = verts_per_side - 1;
    for col in 0..verts_per_side {
        perim.push(col); // 南边，west -> east
        perim_uv.push(south_uv);
    }
    for row in 1..verts_per_side {
        perim.push(row * verts_per_side + last); // 东边，south -> north
        perim_uv.push(east_uv);
    }
    for col in (0..last).rev() {
        perim.push(last * verts_per_side + col); // 北边，east -> west
        perim_uv.push(north_uv);
    }
    for row in (1..last).rev() {
        perim.push(row * verts_per_side); // 西边，north -> south
        perim_uv.push(west_uv);
    }
    // 两堵专用墙环：一个与表面边缘重合的 TOP 环
    //（拥有自己的顶点，因为网格边缘顶点必须保持其
    // 变化的表面 uv）和一个位于下降半径的 BOTTOM 环。恒定->恒定
    // 插值使整堵墙保持单一平面颜色；从网格边缘变化的 uv
    // 插值（如共享顶点墙那样）正是
    // 产生条纹鳍片的原因。
    for (i, &g) in perim.iter().enumerate() {
        let p = positions[g as usize];
        positions.push(p); // 顶环：与边缘重合，恒定 uv
        normals.push(normals[g as usize]);
        uvs.push(perim_uv[i]);
        positions.push([p[0] * drop as f32, p[1] * drop as f32, p[2] * drop as f32]);
        normals.push(normals[g as usize]);
        uvs.push(perim_uv[i]);
    }

    // 生成三角形索引（从外侧看的逆时针绕序）
    let quads = verts_per_side - 1;
    let mut indices: Vec<u32> =
        Vec::with_capacity((quads * quads * 6) as usize + perimeter_count * 6);
    for row in 0..quads {
        for col in 0..quads {
            let a = row * verts_per_side + col;
            let b = a + verts_per_side;

            // 每个四边形两个三角形，从外侧看 CCW
            indices.push(a);
            indices.push(a + 1);
            indices.push(b);
            indices.push(a + 1);
            indices.push(b + 1);
            indices.push(b);
        }
    }
    // 两堵专用环之间的裙边墙带（双面材质，
    // 绕序无关）。每个周边点贡献一个顶/底顶点
    // 对，因此环索引交错：top = 2*i，
    // bottom = 2*i + 1（按网格偏移）。绝不直接引用网格边缘
    // 顶点：它们的 uv 沿边缘变化，会重新
    // 产生条纹鳍片。
    for i in 0..perimeter_count {
        let j = (i + 1) % perimeter_count;
        let t0 = grid_count as u32 + 2 * i as u32;
        let b0 = t0 + 1;
        let t1 = grid_count as u32 + 2 * j as u32;
        let b1 = t1 + 1;
        indices.push(t0);
        indices.push(b0);
        indices.push(b1);
        indices.push(t0);
        indices.push(b1);
        indices.push(t1);
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));

    mesh
}

/// 生成一个平滑的单位半径 UV 球（用于基础球的安全网，
/// 其轮廓在地平线处可见）。绕序与 `create_tile_mesh` 一致
///（从外侧看逆时针，行 south -> north）。
// 遗留 CesiumJS 移植风格债务（deferred.md #18）；在 M13 lint-cleanup 或本文件在其里程碑被重写时
#[allow(dead_code)]
pub fn create_uv_sphere(longitude_segments: u32, latitude_rings: u32) -> Mesh {
    let verts_x = longitude_segments + 1; // 最后一列重复接缝
    let verts_y = latitude_rings + 1;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((verts_x * verts_y) as usize);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity((verts_x * verts_y) as usize);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity((verts_x * verts_y) as usize);

    for row in 0..verts_y {
        // 行从南（-90 deg）迭代到北（+90 deg），与瓦片网格一致。
        let lat = -std::f64::consts::FRAC_PI_2
            + std::f64::consts::PI * row as f64 / latitude_rings as f64;
        let cos_lat = lat.cos();
        let sin_lat = lat.sin();
        for col in 0..verts_x {
            let lon = 2.0 * std::f64::consts::PI * col as f64 / longitude_segments as f64;
            let nx = cos_lat * lon.cos();
            let ny = cos_lat * lon.sin();
            let nz = sin_lat;
            positions.push([nx as f32, ny as f32, nz as f32]);
            normals.push([nx as f32, ny as f32, nz as f32]);
            uvs.push([
                col as f32 / longitude_segments as f32,
                row as f32 / latitude_rings as f32,
            ]);
        }
    }

    let mut indices: Vec<u32> =
        Vec::with_capacity((longitude_segments * latitude_rings * 6) as usize);
    for row in 0..latitude_rings {
        for col in 0..longitude_segments {
            let a = row * verts_x + col;
            let b = a + verts_x;
            indices.push(a);
            indices.push(a + 1);
            indices.push(b);
            indices.push(a + 1);
            indices.push(b + 1);
            indices.push(b);
        }
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));

    mesh
}

/// UV 球，其 V 坐标跟随 Web Mercator 纬度而非线性纬度，
/// 因此整球墨卡托合成纹理（运行时由基础瓦片层烘焙）
/// 会像瓦片层一样精确披覆。用于基础球：任何瞬时的覆盖空洞
/// 或地平线边缘届时都显示模糊的地球
/// 而非单一平面颜色。
pub fn create_mercator_uv_sphere(longitude_segments: u32, latitude_rings: u32) -> Mesh {
    let verts_x = longitude_segments + 1; // 最后一列重复接缝
    let verts_y = latitude_rings + 1;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((verts_x * verts_y) as usize);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity((verts_x * verts_y) as usize);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity((verts_x * verts_y) as usize);

    for row in 0..verts_y {
        // 行从南（-90 deg）迭代到北（+90 deg），与瓦片网格一致。
        let raw_lat = -std::f64::consts::FRAC_PI_2
            + std::f64::consts::PI * row as f64 / latitude_rings as f64;
        let lat = raw_lat.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
        let cos_lat = raw_lat.cos();
        let sin_lat = raw_lat.sin();
        // 墨卡托北距 [-PI, PI] -> v 在 [0, 1]，0 在北侧，
        // 与瓦片行 0 = 北一致。
        let t = (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln();
        let v = (1.0 - t / std::f64::consts::PI) / 2.0;
        for col in 0..verts_x {
            let lon = 2.0 * std::f64::consts::PI * col as f64 / longitude_segments as f64;
            let nx = cos_lat * lon.cos();
            let ny = cos_lat * lon.sin();
            let nz = sin_lat;
            positions.push([nx as f32, ny as f32, nz as f32]);
            normals.push([nx as f32, ny as f32, nz as f32]);
            uvs.push([col as f32 / longitude_segments as f32, v as f32]);
        }
    }

    let mut indices: Vec<u32> =
        Vec::with_capacity((longitude_segments * latitude_rings * 6) as usize);
    for row in 0..latitude_rings {
        for col in 0..longitude_segments {
            let a = row * verts_x + col;
            let b = a + verts_x;
            indices.push(a);
            indices.push(a + 1);
            indices.push(b);
            indices.push(a + 1);
            indices.push(b + 1);
            indices.push(b);
        }
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));

    mesh
}

/// 返回将米转换为渲染单位的渲染比例因子。
pub fn render_scale() -> f32 {
    (1.0 / METERS_PER_RENDER_UNIT) as f32
}

/// Web Mercator 最大纬度（弧度）：atan(sinh(PI)) = 85.05112877980659 deg。
/// Web Mercator 瓦片仅覆盖 [-MAX_LAT, MAX_LAT]；超过此纬度的
/// 极地区域不被任何瓦片覆盖，因此我们单独为它们加冠。
///（硬编码因 exp/atan 不是 const-fn；等于 PI/2 - 2*atan(exp(-PI))。）
const MAX_MERCATOR_LAT: f64 = 1.4844222297453322;

/// 生成一个极地冠网格，覆盖从 Web Mercator
/// 最大纬度（85.051129 deg）到极点（90 deg）的区域。
///
/// 这填补了 Web Mercator 划分留下的空洞（它无法
/// 表示极点）。冠以单一颜色（冰白）渲染，与 CesiumJS 一致——
/// 那里地形覆盖极点，影像仅披覆在
/// 瓦片化区域上。
///
/// # 参数
/// * `north` - 北极冠为 true，南极冠为 false
/// * `segments` - 绕极点的经向细分数量
pub fn create_polar_cap(north: bool, segments: u32) -> Mesh {
    let a = EARTH_RADIUS;
    let b = 6356752.314245_f64;
    let e2 = 1.0 - (b * b) / (a * a);

    // 环纬度：精确匹配最外圈瓦片行边界，使冠与
    // 相邻瓦片网格无缝融合。
    let ring_lat = if north { MAX_MERCATOR_LAT } else { -MAX_MERCATOR_LAT };
    let sin_ring = ring_lat.sin();
    let cos_ring = ring_lat.cos();
    let n_ring = a / (1.0 - e2 * sin_ring * sin_ring).sqrt();

    // 椭球表面上的极点顶点。
    let pole_z = if north { b } else { -b };

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((segments + 2) as usize);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity((segments + 2) as usize);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity((segments + 2) as usize);

    // 将冠收拢到**刚好**低于瓦片表面处：既要低于瓦片层（否则与瓦片
    // 共面，贴近地面时会盖住影像、产生闪烁的块），又不能低太多 ——
    // 旧值 0.9995（≈3.2 km）在贴地表时会把瓦片缺口读成一道深渊。
    // 1 − 1e-5 半径 ≈ 64 m；reversed-Z f32 深度的相对分辨率约 2⁻²⁴，
    // 最远视距（20 render unit）上约 7.7 m，64 m 有充分余量。
    // 层序（由外到内）：瓦片层(1.0) > 极地盖片(本值) > 基础球
    // （`main.rs` 的 `BASE_SPHERE_SCALE` ≈ 256 m）。
    const CAP_TUCK: f64 = 1.0 - 1.0e-5;

    // 极点处的中心顶点（法线沿极轴指向外）。
    positions.push([0.0, 0.0, (pole_z * CAP_TUCK) as f32]);
    normals.push([0.0, 0.0, if north { 1.0 } else { -1.0 }]);
    uvs.push([0.5, 0.5]);

    // 位于最大墨卡托纬度的顶点环。
    for i in 0..=segments {
        let lon = 2.0 * std::f64::consts::PI * i as f64 / segments as f64;
        let cos_lon = lon.cos();
        let sin_lon = lon.sin();

        let px = n_ring * cos_ring * cos_lon;
        let py = n_ring * cos_ring * sin_lon;
        let pz = n_ring * (1.0 - e2) * sin_ring;

        positions.push([
            (px * CAP_TUCK) as f32,
            (py * CAP_TUCK) as f32,
            (pz * CAP_TUCK) as f32,
        ]);
        normals.push([(cos_ring * cos_lon) as f32, (cos_ring * sin_lon) as f32, sin_ring as f32]);
        uvs.push([0.5, 0.5]);
    }

    // 从极点到环的三角形扇。
    // 选定的绕序使法线指向外（远离地球中心）。
    let mut indices: Vec<u32> = Vec::with_capacity(segments as usize * 3);
    for i in 0..segments {
        let ring_a = 1 + i;
        let ring_b = 1 + i + 1;
        if north {
            // 从上方（+z）看逆时针 -> 法线 +z（向外）。
            indices.push(0);
            indices.push(ring_a);
            indices.push(ring_b);
        } else {
            // 从上方看顺时针 -> 法线 -z（在南极处向外）。
            indices.push(0);
            indices.push(ring_b);
            indices.push(ring_a);
        }
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));

    mesh
}
