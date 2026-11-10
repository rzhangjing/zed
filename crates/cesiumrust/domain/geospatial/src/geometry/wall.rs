//! 围墙几何 —— 在顶部与底部高度之间垂直拉伸出的幕帘。
//!
//! 围墙由一系列表面位置定义，
//! 它们在最小和最大高度之间垂直拉伸。相邻位置之间的弧沿大地线细分（参见
//! [`crate::polyline_pipeline`]）。

use crate::bounding::BoundingSphere;
use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use crate::geometry::{GeometryData, PrimitiveType, VertexFormat};
use crate::math_utils::EPSILON10;
use crate::polyline_pipeline::{generate_arc, ArcOptions};
use glam::DVec3;

/// 默认粒度：一度（以弧度计）。
const DEFAULT_GRANULARITY: f64 = std::f64::consts::PI / 180.0;

/// 描述一个围墙的选项。
#[derive(Debug, Clone)]
pub struct WallOptions {
    /// 定义围墙路径的表面位置（至少 2 个）。
    pub positions: Vec<DVec3>,
    /// 每个位置的最大（顶部）高度。`None` 使用每个位置自身的高度。
    pub maximum_heights: Option<Vec<f64>>,
    /// 每个位置的最小（底部）高度。`None` 使用 0。
    pub minimum_heights: Option<Vec<f64>>,
    /// 角度粒度（弧度）。
    pub granularity: f64,
    /// 参考椭球。
    pub ellipsoid: Ellipsoid,
}

impl WallOptions {
    /// 由恒定的顶部/底部高度创建一个围墙（镜像
    /// `WallGeometry.fromConstantHeights`）。
    ///
    /// # 参数
    /// - `positions`：围墙表面位置（≥2）。
    /// - `minimum_height`/`maximum_height`：恒定的底/顶高度；`None` 分别退化为 0 与逐点自身高度。
    /// - `ellipsoid`：参考椭球。
    ///
    /// # 返回
    /// 各位置共享同一顶/底高度的 `WallOptions`。
    pub fn from_constant_heights(
        positions: Vec<DVec3>,
        minimum_height: Option<f64>,
        maximum_height: Option<f64>,
        ellipsoid: Ellipsoid,
    ) -> Self {
        let length = positions.len();
        let minimum_heights = minimum_height.map(|h| vec![h; length]);
        let maximum_heights = maximum_height.map(|h| vec![h; length]);
        Self {
            positions,
            maximum_heights,
            minimum_heights,
            granularity: DEFAULT_GRANULARITY,
            ellipsoid,
        }
    }
}

impl Default for WallOptions {
    /// 默认围墙选项：空位置、无顶/底高度、默认粒度、WGS84 椭球。
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            maximum_heights: None,
            minimum_heights: None,
            granularity: DEFAULT_GRANULARITY,
            ellipsoid: Ellipsoid::WGS84,
        }
    }
}

/// 去重后的清理位置。
struct CleanedPositions {
    /// 去重后的位置序列（与下方两个高度数组一一对应）。
    positions: Vec<DVec3>,
    /// 每个位置的顶部高度（米）。
    top_heights: Vec<f64>,
    /// 每个位置的底部高度（米）。
    bottom_heights: Vec<f64>,
}

/// 比较两个大地坐标的经/纬度是否在容差内相等（忽略高度）。
///
/// # 参数
/// - `c0`/`c1`：待比较的大地坐标。
///
/// # 返回
/// 纬度与经度差均不超过 `EPSILON10` 时返回 `true`。
fn lat_lon_equals(c0: &Cartographic, c1: &Cartographic) -> bool {
    (c0.latitude - c1.latitude).abs() <= EPSILON10
        && (c0.longitude - c1.longitude).abs() <= EPSILON10
}

/// 判断两个笛卡尔坐标是否在各分量上于 `EPSILON10` 内相等。
///
/// # 参数
/// - `a`/`b`：待比较的三维点。
///
/// # 返回
/// 三分量绝对差均不超过容差时返回 `true`。
fn cartesian_equals_epsilon(a: DVec3, b: DVec3) -> bool {
    (a.x - b.x).abs() <= EPSILON10
        && (a.y - b.y).abs() <= EPSILON10
        && (a.z - b.z).abs() <= EPSILON10
}

/// 移除连续重复的位置（并合并共享相同经/纬度的位置的高度）。
///
/// 映射到 `WallGeometryLibrary` 的私有 `removeDuplicates`。
///
/// # 参数
/// - `ellipsoid`：参考椭球，用于笛卡尔↔大地坐标转换。
/// - `positions`/`top_heights`/`bottom_heights`：原始位置与逐点顶/底高度。
///
/// # 返回
/// 去重并合并同经纬度高度后的清理位置；不足 2 点或全退化时返回 `None`。
fn remove_duplicates(
    ellipsoid: &Ellipsoid,
    positions: &[DVec3],
    top_heights: Option<&[f64]>,
    bottom_heights: Option<&[f64]>,
) -> Option<CleanedPositions> {
    // arrayRemoveDuplicates：丢弃连续的、完全相等的位置。
    let mut deduped: Vec<DVec3> = Vec::with_capacity(positions.len());
    for &p in positions {
        if deduped.last().is_none_or(|&last| !cartesian_equals_epsilon(last, p)) {
            deduped.push(p);
        }
    }

    let length = deduped.len();
    if length < 2 {
        return None;
    }

    // 是否提供逐点底/顶高度；缺省则底部按 0 处理。
    let has_bottom = bottom_heights.is_some();
    let has_top = top_heights.is_some();

    let mut cleaned_positions: Vec<DVec3> = Vec::with_capacity(length);
    let mut cleaned_top: Vec<f64> = Vec::with_capacity(length);
    let mut cleaned_bottom: Vec<f64> = Vec::with_capacity(length);

    let v0 = deduped[0];
    cleaned_positions.push(v0);

    let mut c0 = ellipsoid.cartesian_to_cartographic(v0).unwrap_or_default();
    if has_top {
        c0.height = top_heights.unwrap()[0];
    }
    cleaned_top.push(c0.height);
    cleaned_bottom.push(if has_bottom { bottom_heights.unwrap()[0] } else { 0.0 });

    // 首点若顶==底，则初始认为“全等高”，后续任一点打破即置假。
    let start_top = cleaned_top[0];
    let start_bottom = cleaned_bottom[0];
    let mut has_all_same_heights = (start_top - start_bottom).abs() < f64::EPSILON;

    for (i, &v1) in deduped.iter().enumerate().skip(1) {
        let mut c1 = ellipsoid.cartesian_to_cartographic(v1).unwrap_or_default();
        if has_top {
            c1.height = top_heights.unwrap()[i];
        }
        has_all_same_heigths_check(&mut has_all_same_heights, c1.height);

        if !lat_lon_equals(&c0, &c1) {
            cleaned_positions.push(v1);
            cleaned_top.push(c1.height);
            cleaned_bottom.push(if has_bottom { bottom_heights.unwrap()[i] } else { 0.0 });
            let idx = cleaned_top.len() - 1;
            has_all_same_heights =
                has_all_same_heights && (cleaned_top[idx] - cleaned_bottom[idx]).abs() < f64::EPSILON;
            c0 = c1;
        } else if c0.height < c1.height {
            // 相邻位置共享经/纬度：保留较大的顶部高度。
            let idx = cleaned_top.len() - 1;
            cleaned_top[idx] = c1.height;
        }
    }

    if has_all_same_heights || cleaned_positions.len() < 2 {
        return None;
    }

    Some(CleanedPositions {
        positions: cleaned_positions,
        top_heights: cleaned_top,
        bottom_heights: cleaned_bottom,
    })
}

/// 将高度近 0 时持续收窄“全等高”标志。
///
/// # 参数
/// - `flag`：当前“所有顶/底高度相等”的累计标志（原地更新）。
/// - `height`：待测高度。
#[inline]
fn has_all_same_heigths_check(flag: &mut bool, height: f64) {
    *flag = *flag && height.abs() < f64::EPSILON;
}

/// [`compute_positions`] 的结果。
struct WallPositions {
    /// 细分后的顶部位置序列。
    top_positions: Vec<DVec3>,
    /// 与顶部逐点对应的底部位置序列。
    bottom_positions: Vec<DVec3>,
    /// 角点数量（去重后位置数 - 2），用于计算 UV 步长。
    num_corners: usize,
}

/// 计算围墙细分后的顶部和底部位置数组。
///
/// 映射到 `WallGeometryLibrary.computePositions`。当 `duplicate_corners` 为
/// true（实心几何）时，每段独立细分，因此角点会被重复以获得正确的逐面法线；
/// 为 false（线框）时，整条路径作为单条弧细分。
///
/// # 参数
/// - `ellipsoid`：参考椭球。
/// - `wall_positions`：围墙路径位置。
/// - `maximum_heights`/`minimum_heights`：逐点顶/底高度。
/// - `granularity`：大地线细分角度（弧度）。
/// - `duplicate_corners`：是否按段重复角点（实心为 true）。
///
/// # 返回
/// 细分后的顶/底位置数组与角点数；退化输入返回 `None`。
fn compute_positions(
    ellipsoid: &Ellipsoid,
    wall_positions: &[DVec3],
    maximum_heights: Option<&[f64]>,
    minimum_heights: Option<&[f64]>,
    granularity: f64,
    duplicate_corners: bool,
) -> Option<WallPositions> {
    let cleaned = remove_duplicates(ellipsoid, wall_positions, maximum_heights, minimum_heights)?;

    let wall_positions = cleaned.positions;
    let maximum_heights = cleaned.top_heights;
    let minimum_heights = cleaned.bottom_heights;

    // 去重后的点数；角点数 = 总点数减 2（首尾非角）。
    let length = wall_positions.len();
    let num_corners = length - 2;

    // 实心：逐段独立细分以重复角点；线框：整条路径一次细分。
    let (top_positions, bottom_positions) = if duplicate_corners {
        let mut top_positions: Vec<DVec3> = Vec::new();
        let mut bottom_positions: Vec<DVec3> = Vec::new();

        for i in 0..length - 1 {
            // 取出相邻两点作为一段，分别按顶/底高度生成大地线弧。
            let seg_positions = [wall_positions[i], wall_positions[i + 1]];

            let top_heights = [maximum_heights[i], maximum_heights[i + 1]];
            let top_opts = ArcOptions {
                positions: &seg_positions,
                heights: Some(&top_heights),
                granularity,
                ellipsoid,
            };
            let top = generate_arc(&top_opts);

            let bottom_heights = [minimum_heights[i], minimum_heights[i + 1]];
            let bottom_opts = ArcOptions {
                positions: &seg_positions,
                heights: Some(&bottom_heights),
                granularity,
                ellipsoid,
            };
            let bottom = generate_arc(&bottom_opts);

            top_positions.extend_from_slice(&top);
            bottom_positions.extend_from_slice(&bottom);
        }
        (top_positions, bottom_positions)
    } else {
        let top_opts = ArcOptions {
            positions: &wall_positions,
            heights: Some(&maximum_heights),
            granularity,
            ellipsoid,
        };
        let bottom_opts = ArcOptions {
            positions: &wall_positions,
            heights: Some(&minimum_heights),
            granularity,
            ellipsoid,
        };
        (generate_arc(&top_opts), generate_arc(&bottom_opts))
    };

    Some(WallPositions {
        top_positions,
        bottom_positions,
        num_corners,
    })
}

/// 生成一个实心围墙几何。
///
/// 映射到 CesiumJS `WallGeometry.createGeometry`。
///
/// # 参数
/// - `options`：围墙选项（位置、逐点顶/底高度、粒度、椭球）。
/// - `vf`：顶点格式，控制是否生成法线/切线/副切线/UV。
///
/// # 返回
/// 顶部与底部之间垂直拉伸、逐面法线的三角幕帘 `GeometryData`。
pub fn wall_geometry(options: &WallOptions, vf: VertexFormat) -> GeometryData {
    let ellipsoid = &options.ellipsoid;
    let pos = compute_positions(
        ellipsoid,
        &options.positions,
        options.maximum_heights.as_deref(),
        options.minimum_heights.as_deref(),
        options.granularity,
        true,
    );

    let Some(pos) = pos else {
        return empty_geometry(PrimitiveType::Triangles);
    };

    let top_positions = &pos.top_positions;
    let bottom_positions = &pos.bottom_positions;
    let num_corners = pos.num_corners;
    let length = top_positions.len();

    // 交错底部（偶数）和顶部（奇数）位置。
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(length * 2);
    let mut normals: Option<Vec<[f64; 3]>> = if vf.normal { Some(Vec::new()) } else { None };
    let mut tangents: Option<Vec<[f64; 3]>> = if vf.tangent { Some(Vec::new()) } else { None };
    let mut bitangents: Option<Vec<[f64; 3]>> = if vf.bitangent { Some(Vec::new()) } else { None };
    let mut tex_coords: Option<Vec<[f64; 2]>> = if vf.st { Some(Vec::new()) } else { None };

    // 逐面法线/切线状态；recompute_normal 在遇到角点重复时置真以重算。
    // s 为沿路径累积的 U 纹理坐标。
    let mut normal = DVec3::ZERO;
    let mut tangent = DVec3::ZERO;
    let mut bitangent = DVec3::ZERO;
    let mut recompute_normal = true;
    let mut s = 0.0f64;
    // U 方向单步增量：总弧长数 = 顶点数 - 角点数 - 1。
    let ds = if length > num_corners + 1 {
        1.0 / (length - num_corners - 1) as f64
    } else {
        0.0
    };

    for i in 0..length {
        let top_position = top_positions[i];
        let bottom_position = bottom_positions[i];

        // 底、顶两点成对入列，共享同一 U、V 分别为 0/1。
        positions.push([bottom_position.x, bottom_position.y, bottom_position.z]);
        positions.push([top_position.x, top_position.y, top_position.z]);

        // 底部/顶部分别写 V=0/1，U 取当前累积值 s。
        if let Some(ref mut st) = tex_coords {
            st.push([s, 0.0]);
            st.push([s, 1.0]);
        }

        // 仅当请求了朝向相关属性时才计算法线/切线/副切线。
        if normals.is_some() || tangents.is_some() || bitangents.is_some() {
            let mut next_top = DVec3::ZERO;
            let surface_normal = ellipsoid
                .geodetic_surface_normal(top_position)
                .unwrap_or(DVec3::Z);
            // 内侧地面点：沿法线向内一个单位，用于构成面平面。
            let ground_position = top_position - surface_normal;
            if i + 1 < length {
                next_top = top_positions[i + 1];
            }

            // 由 (地表内侧→下一顶部) 两向量叉积求该面朝向外的法线。
            if recompute_normal {
                let scaled_next = next_top - top_position;
                let scaled_ground = ground_position - top_position;
                normal = scaled_ground.cross(scaled_next).normalize_or(DVec3::Z);
                recompute_normal = false;
            }

            // 角点处顶/下一顶重合：标记需重算；否则推进 U 并更新切线/副切线。
            if cartesian_equals_epsilon(top_position, next_top) {
                recompute_normal = true;
            } else {
                s += ds;
                if tangents.is_some() {
                    tangent = (next_top - top_position).normalize_or(DVec3::X);
                }
                if bitangents.is_some() {
                    bitangent = normal.cross(tangent).normalize_or(DVec3::Y);
                }
            }

            if let Some(ref mut n) = normals {
                n.push([normal.x, normal.y, normal.z]);
                n.push([normal.x, normal.y, normal.z]);
            }
            if let Some(ref mut t) = tangents {
                t.push([tangent.x, tangent.y, tangent.z]);
                t.push([tangent.x, tangent.y, tangent.z]);
            }
            if let Some(ref mut b) = bitangents {
                b.push([bitangent.x, bitangent.y, bitangent.z]);
                b.push([bitangent.x, bitangent.y, bitangent.z]);
            }
        }
    }

    // 每个围墙四边形两个三角形。
    let num_vertices = positions.len();
    let mut indices: Vec<u32> = Vec::new();
    let mut i = 0usize;
    while i + 2 < num_vertices {
        let ll = i;
        let lr = i + 2;
        let pl = DVec3::from(positions[ll]);
        let pr = DVec3::from(positions[lr]);
        if cartesian_equals_epsilon(pl, pr) {
            i += 2;
            continue;
        }
        // 四边形 (ll,lr,ul,ur) → 三角形 (ul,ll,ur) 与 (ur,ll,lr)。
        let ul = i + 1;
        let ur = i + 3;
        indices.extend_from_slice(&[ul as u32, ll as u32, ur as u32]);
        indices.extend_from_slice(&[ur as u32, ll as u32, lr as u32]);
        i += 2;
    }

    // 由全部交错顶点拟合包围球，供剔除与相交测试。
    let bounding_sphere = BoundingSphere::from_points(
        &positions.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    GeometryData {
        positions,
        normals,
        tex_coords,
        tangents,
        bitangents,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 映射到 CesiumJS `WallOutlineGeometry.createGeometry`。
///
/// # 参数
/// - `options`：围墙选项；顶/底位置整条路径作为单条弧细分。
///
/// # 返回
/// 勾勒围墙左右竖边与顶底边的线段 `GeometryData`（`Lines` 拓扑）。
pub fn wall_outline_geometry(options: &WallOptions) -> GeometryData {
    let ellipsoid = &options.ellipsoid;
    let pos = compute_positions(
        ellipsoid,
        &options.positions,
        options.maximum_heights.as_deref(),
        options.minimum_heights.as_deref(),
        options.granularity,
        false,
    );

    let Some(pos) = pos else {
        return empty_geometry(PrimitiveType::Lines);
    };

    let top_positions = &pos.top_positions;
    let bottom_positions = &pos.bottom_positions;
    let length = top_positions.len();

    // 交错底部（偶数）和顶部（奇数）。
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(length * 2);
    // 线框同样底/顶交错存放，便于按列取竖边。
    for i in 0..length {
        let bp = bottom_positions[i];
        let tp = top_positions[i];
        positions.push([bp.x, bp.y, bp.z]);
        positions.push([tp.x, tp.y, tp.z]);
    }

    let num_vertices = positions.len();
    let mut indices: Vec<u32> = Vec::new();
    let mut i = 0usize;
    while i + 2 < num_vertices {
        let ll = i;
        let lr = i + 2;
        let pl = DVec3::from(positions[ll]);
        let pr = DVec3::from(positions[lr]);
        if cartesian_equals_epsilon(pl, pr) {
            i += 2;
            continue;
        }
        // 每列发射三条边：左竖边、顶边、底边。
        let ul = i + 1;
        let ur = i + 3;
        // 左侧竖边、顶边、底边。
        indices.extend_from_slice(&[ul as u32, ll as u32]);
        indices.extend_from_slice(&[ul as u32, ur as u32]);
        indices.extend_from_slice(&[ll as u32, lr as u32]);
        i += 2;
    }
    // 最后一条竖边。
    if num_vertices >= 2 {
        indices.push((num_vertices - 2) as u32);
        indices.push((num_vertices - 1) as u32);
    }

    let bounding_sphere = BoundingSphere::from_points(
        &positions.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 构造一个位置/索引皆空的几何（退化输入的兜底返回值）。
///
/// # 参数
/// - `primitive_type`：回退时使用的拓扑（三角形或线段）。
fn empty_geometry(primitive_type: PrimitiveType) -> GeometryData {
    GeometryData {
        positions: Vec::new(),
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: Vec::new(),
        bounding_sphere: BoundingSphere::default(),
        primitive_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个 4 点、0~10000m 恒定高度的测试围墙选项。
    fn wall_options() -> WallOptions {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(19.0, 47.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(19.0, 48.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(20.0, 48.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(20.0, 47.0, 0.0)),
        ];
        WallOptions::from_constant_heights(positions, Some(0.0), Some(10000.0), ell)
    }

    /// 基础用例：实心围墙生成三角、位置成偶数（底/顶交错）且法线/UV 数量与位置匹配。
    #[test]
    fn test_wall_geometry_basic() {
        let geo = wall_geometry(&wall_options(), VertexFormat::ALL);
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
        assert_eq!(geo.indices.len() % 3, 0);
        // 位置交错底部/顶部 => 偶数个。
        assert_eq!(geo.positions.len() % 2, 0);
        assert_eq!(geo.normals.as_ref().unwrap().len(), geo.positions.len());
        assert_eq!(geo.tex_coords.as_ref().unwrap().len(), geo.positions.len());
    }

    /// 高度校验：偶数位为底部(~0m)、奇数位为顶部(~10000m)。
    #[test]
    fn test_wall_heights_correct() {
        let ell = Ellipsoid::WGS84;
        let geo = wall_geometry(&wall_options(), VertexFormat::POSITION_ONLY);
        // 偶数索引为底部（~0 m），奇数为顶部（~10000 m）。
        for (i, p) in geo.positions.iter().enumerate() {
            let c = ell.cartesian_to_cartographic(DVec3::new(p[0], p[1], p[2])).unwrap();
            if i % 2 == 0 {
                assert!(c.height.abs() < 1.0, "bottom height {}", c.height);
            } else {
                assert!((c.height - 10000.0).abs() < 1.0, "top height {}", c.height);
            }
        }
    }

    /// 线框用例：索引成对且全部落在顶点范围内。
    #[test]
    fn test_wall_outline_basic() {
        let geo = wall_outline_geometry(&wall_options());
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
        assert_eq!(geo.indices.len() % 2, 0);
        // 所有索引都在范围内。
        let n = geo.positions.len() as u32;
        for &idx in &geo.indices {
            assert!(idx < n);
        }
    }

    /// 退化输入：所有顶高为 0 时围墙退化，返回空几何。
    #[test]
    fn test_wall_degenerate_all_zero_heights() {
        // 当所有顶部高度都为 0 时，CesiumJS 认为围墙是退化的。
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        ];
        let opts = WallOptions::from_constant_heights(positions, None, None, ell);
        let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.is_empty());
    }

    /// 退化输入：位置不足 2 个时返回空几何。
    #[test]
    fn test_wall_too_few_positions() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0))];
        let opts = WallOptions::from_constant_heights(positions, Some(0.0), Some(100.0), ell);
        let geo = wall_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.is_empty());
    }
}
