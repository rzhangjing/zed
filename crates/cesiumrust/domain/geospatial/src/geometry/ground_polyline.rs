//! 贴地折线几何 - 贴附在椭球表面上的折线。
//!
//! 简化版本：完整实现会与地形/3D Tiles 相交；这里我们贴附到
//! 椭球表面并生成一个可渲染的条状体。
//!
//! 处理流程：先把输入位置缩放到椭球表面（忽略高度）并去重，必要时闭合环路；
//! 再沿大地线按粒度细分成弧段点列；最后在每个点沿当地法线与切线的叉积（左方向）
//! 两侧各展开半个宽度，形成左右两列顶点并三角剖分为一个三角形带。

use crate::bounding::BoundingSphere;
use crate::ellipsoid::Ellipsoid;
use crate::geometry::{GeometryData, PrimitiveType, VertexFormat};
use crate::math_utils::EPSILON10;
use crate::polyline_pipeline::{generate_arc, ArcOptions};
use glam::DVec3;

/// 描述一条贴地折线的选项。
///
/// 位置至少两个；高度会被忽略，宽度以米为单位沿断面两侧各展开一半。
#[derive(Debug, Clone)]
pub struct GroundPolylineOptions {
    /// 折线的位置（至少 2 个）。高度会被忽略。
    pub positions: Vec<DVec3>,
    /// 宽度（米）。
    pub width: f64,
    /// 用于弧细分的角度粒度（弧度）。
    pub granularity: f64,
    /// 是否闭合环路（连接末尾与首点）。
    pub closed: bool,
    /// 参考椭球。
    pub ellipsoid: Ellipsoid,
}

impl Default for GroundPolylineOptions {
    /// 默认选项：空位置、宽 1 米、粒度 1°、不闭合、WGS84 椭球。
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            width: 1.0,
            granularity: std::f64::consts::PI / 180.0,
            closed: false,
            ellipsoid: Ellipsoid::WGS84,
        }
    }
}

/// 生成一个贴附在椭球表面上的贴地折线几何。
///
/// # 参数
/// - `options`：折线选项（位置/宽度/粒度/闭合/椭球）。
/// - `vf`：顶点格式，决定是否附带法线、切线、副切线与 UV。
///
/// # 返回
/// [`GeometryData`]：三角形带表示的条状体；位置不足两个或宽度非正时返回空几何。
pub fn ground_polyline_geometry(options: &GroundPolylineOptions, vf: VertexFormat) -> GeometryData {
    let ellipsoid = &options.ellipsoid;
    // 宽度与半宽驱动断面展开；宽度非正已在后面退化处理。
    let width = options.width;

    // 将位置缩放到表面（忽略高度）。
    // 即使输入带高度，也会被拉回椭球面，以保证折线贴地。
    let mut positions: Vec<DVec3> = options
        .positions
        .iter()
        .map(|&p| ellipsoid.scale_to_geodetic_surface(p).unwrap_or(p))
        .collect();

    // 去除重复项。
    // 缩放后相邻点可能重合，逐坐标比较 epsilon 去重以免产生零长段。
    positions.dedup_by(|a, b| {
        (a.x - b.x).abs() <= EPSILON10
            && (a.y - b.y).abs() <= EPSILON10
            && (a.z - b.z).abs() <= EPSILON10
    });

    // 退化保护：去重后不足两点或宽度非正均无法生成条状体。
    if positions.len() < 2 || width <= 0.0 {
        return empty_geometry();
    }

    // 环路闭合后首尾同点，会产生一个额外断面。
    if options.closed && positions.len() > 2 {
        positions.push(positions[0]);
    }

    // 细分为大地线弧：在相邻控制点间按角度粒度插入大地线中间点。
    let opts = ArcOptions {
        positions: &positions,
        heights: None,
        granularity: options.granularity,
        ellipsoid,
    };
    let arc = generate_arc(&opts);

    // 弧段点列不足两个无法成带，直接回退为空几何。
    let n = arc.len();
    if n < 2 {
        return empty_geometry();
    }

    let half_width = width / 2.0;

    // 生成条状体顶点：每个弧点展开为左/右两点，并按 vf 选择性地收集各属性。
    let mut pos_out: Vec<[f64; 3]> = Vec::with_capacity(n * 2);
    let mut normals_out: Option<Vec<[f64; 3]>> = if vf.normal { Some(Vec::with_capacity(n * 2)) } else { None };
    let mut tangents_out: Option<Vec<[f64; 3]>> = if vf.tangent { Some(Vec::with_capacity(n * 2)) } else { None };
    let mut bitangents_out: Option<Vec<[f64; 3]>> = if vf.bitangent { Some(Vec::with_capacity(n * 2)) } else { None };
    let mut st_out: Option<Vec<[f64; 2]>> = if vf.st { Some(Vec::with_capacity(n * 2)) } else { None };

    // UV 沿折线长度均匀参数化，s 从 0 到 1，t 区分左/右两侧。
    let st_s = if n > 1 { 1.0 / (n - 1) as f64 } else { 1.0 };

    for i in 0..n {
        // 逐弧点取法线（回退为 Z 轴以防球心处无定义）。
        let p = arc[i];
        let normal = ellipsoid.geodetic_surface_normal(p).unwrap_or(DVec3::Z);

        // 切线沿折线前进方向：首/尾用单侧差分，中间用中心差分。
        let tangent = if i == 0 {
            (arc[1] - arc[0]).normalize_or(DVec3::X)
        } else if i == n - 1 {
            (arc[n - 1] - arc[n - 2]).normalize_or(DVec3::X)
        } else {
            (arc[i + 1] - arc[i - 1]).normalize_or(DVec3::X)
        };

        // 左方向为法线与切线的叉积，沿它向两侧各偏半宽得到左右顶点。
        let left = normal.cross(tangent).normalize_or(DVec3::Y);

        // 沿左方向向两侧各偏半宽，得到断面的右、左两个顶点。
        let right_pt = p - left * half_width;
        let left_pt = p + left * half_width;

        // 先推右侧顶点再推左侧顶点，保证左右交替的索引排列与剖分一致。
        pos_out.push([right_pt.x, right_pt.y, right_pt.z]);
        pos_out.push([left_pt.x, left_pt.y, left_pt.z]);

        if let Some(ref mut norms) = normals_out {
            // 左右两点共用同一当地法线。
            norms.push([normal.x, normal.y, normal.z]);
            norms.push([normal.x, normal.y, normal.z]);
        }
        if let Some(ref mut tans) = tangents_out {
            // 左右两点共用同一切线方向。
            tans.push([tangent.x, tangent.y, tangent.z]);
            tans.push([tangent.x, tangent.y, tangent.z]);
        }
        if let Some(ref mut bits) = bitangents_out {
            // 副切线为法线与切线的叉积，与左方向同向。
            let bitangent = normal.cross(tangent).normalize_or(DVec3::Y);
            bits.push([bitangent.x, bitangent.y, bitangent.z]);
            bits.push([bitangent.x, bitangent.y, bitangent.z]);
        }
        if let Some(ref mut st) = st_out {
            // s 沿弧点序号递增，t 固定区分左右（0 右、1 左）。
            let s = i as f64 * st_s;
            st.push([s, 0.0]);
            st.push([s, 1.0]);
        }
    }

    // 三角剖分：相邻两断面间的四边形拆为两个三角形。
    let mut indices: Vec<u32> = Vec::with_capacity((n - 1) * 6);
    for i in 0..n - 1 {
        // 断面四顶点的全局索引：当前断面左右 r0/l0，下一断面左右 r1/l1。
        let r0 = (i * 2) as u32;
        let l0 = (i * 2 + 1) as u32;
        let r1 = ((i + 1) * 2) as u32;
        let l1 = ((i + 1) * 2 + 1) as u32;

        // 每个四边形用 (l0,r0,l1) 与 (l1,r0,r1) 两个三角形覆盖。
        indices.extend_from_slice(&[l0, r0, l1]);
        indices.extend_from_slice(&[l1, r0, r1]);
    }

    // 由全部输出顶点计算包围球，供后续裁剪/拾取使用。
    let bounding_sphere = BoundingSphere::from_points(
        &pos_out.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    // 组装最终几何：位置必带，其余顶点属性按 vf 选择是否为 Some。
    GeometryData {
        positions: pos_out,
        normals: normals_out,
        tex_coords: st_out,
        tangents: tangents_out,
        bitangents: bitangents_out,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 构造一个空的几何数据（无顶点、无索引），用于退化输入的回退。
fn empty_geometry() -> GeometryData {
    GeometryData {
        positions: Vec::new(),
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: Vec::new(),
        bounding_sphere: BoundingSphere::default(),
        primitive_type: PrimitiveType::Triangles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartographic::Cartographic;

    fn ground_opts() -> GroundPolylineOptions {
        // 构造一段横跨三点的示例折线选项（带高度，将被忽略），宽 5000 米。
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-112.0, 36.0, 1000.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-111.0, 36.5, 2000.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-110.0, 36.0, 500.0)),
        ];
        GroundPolylineOptions {
            positions,
            width: 5000.0,
            granularity: std::f64::consts::PI / 180.0,
            closed: false,
            ellipsoid: ell,
        }
    }

    #[test]
    /// 验证基本折线生成非空三角形带且顶点成对、法线存在。
    fn test_ground_polyline_basic() {
        let geo = ground_polyline_geometry(&ground_opts(), VertexFormat::ALL);
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
        assert_eq!(geo.indices.len() % 3, 0);
        assert_eq!(geo.positions.len() % 2, 0);
        assert!(geo.normals.is_some());
    }

    #[test]
    /// 验证所有输出顶点都贴近椭球表面（允许宽度展开的小偏移）。
    fn test_ground_polyline_on_surface() {
        let ell = Ellipsoid::WGS84;
        let geo = ground_polyline_geometry(&ground_opts(), VertexFormat::POSITION_ONLY);
        // 所有位置都应位于椭球表面上（在容差范围内）。
        for p in &geo.positions {
            let pos = DVec3::new(p[0], p[1], p[2]);
            let surface = ell.scale_to_geodetic_surface(pos).unwrap_or(pos);
            let dist = (pos - surface).length();
            // 允许由宽度展开带来的小偏移。
            assert!(dist < 5000.0, "position too far from surface: {}", dist);
        }
    }

    #[test]
    /// 验证闭合环路产生的顶点多于非闭合情形。
    fn test_ground_polyline_loop() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.5, 1.0, 0.0)),
        ];
        let opts = GroundPolylineOptions {
            positions,
            width: 1000.0,
            closed: true,
            ..Default::default()
        };
        let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(!geo.positions.is_empty());
        // 环路应比非环路拥有更多顶点。
        let opts_no_loop = GroundPolylineOptions {
            closed: false,
            ..opts.clone()
        };
        let geo_no_loop = ground_polyline_geometry(&opts_no_loop, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.len() > geo_no_loop.positions.len());
    }

    #[test]
    /// 验证位置不足两个时返回空几何。
    fn test_ground_polyline_too_few_positions() {
        let ell = Ellipsoid::WGS84;
        let opts = GroundPolylineOptions {
            positions: vec![ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0))],
            width: 100.0,
            ..Default::default()
        };
        let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.is_empty());
    }

    #[test]
    /// 验证宽度为零时返回空几何。
    fn test_ground_polyline_zero_width() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        ];
        let opts = GroundPolylineOptions {
            positions,
            width: 0.0,
            ..Default::default()
        };
        let geo = ground_polyline_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.is_empty());
    }
}
