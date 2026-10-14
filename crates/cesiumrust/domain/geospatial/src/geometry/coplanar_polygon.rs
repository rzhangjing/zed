//! 共面多边形几何 —— 由任意共面位置构成的多边形。
//!
//! 将共面的 3D 位置投影到
//! 其最佳拟合平面上，在 2D 中三角剖分，并生成网格。
//!
//! 处理流程：先用 Newell 法计算稳健的平面法线并将法线定向为远离椭球中心；
//! 再由法线构造平面内的一组正交坐标轴 (axis1, axis2)，将 3D 位置减去形心后
//! 投影到轴上得到 2D 坐标；随后在 2D 中三角剖分，并根据包围矩形把 2D 坐标
//! 映射为归一化的 UV（可选绕中心旋转 st_rotation），最终组装为三角形网格。

use crate::bounding::BoundingSphere;
use crate::ellipsoid::Ellipsoid;
use crate::geometry::{triangulate_polygon, GeometryData, PrimitiveType, VertexFormat};
use crate::math_utils::EPSILON10;
use glam::DVec3;

/// 描述一个共面多边形的选项。
#[derive(Debug, Clone)]
pub struct CoplanarPolygonOptions {
    /// 多边形的位置（至少 3 个，必须共面）。
    pub positions: Vec<DVec3>,
    /// 纹理坐标旋转（弧度）。
    pub st_rotation: f64,
    /// 参考椭球。
    pub ellipsoid: Ellipsoid,
}

impl Default for CoplanarPolygonOptions {
    /// 默认选项：空位置、无 ST 旋转、WGS84 椭球。
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            st_rotation: 0.0,
            ellipsoid: Ellipsoid::WGS84,
        }
    }
}

/// 使用 Newell 法计算平面法线（对任意多边形都稳健）。
///
/// 逐边累加叉积分量，即使多边形非严格凸或略带扭曲也能得到平均意义上的最佳拟合法线。
fn compute_normal(positions: &[DVec3]) -> DVec3 {
    let n = positions.len();
    let mut normal = DVec3::ZERO;
    for i in 0..n {
        let current = positions[i];
        let next = positions[(i + 1) % n];
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    normal.normalize_or(DVec3::Z)
}

/// 计算位置的形心。
/// 顶点坐标的算术平均，用作投影时平移到局部原点的参考点。
fn compute_center(positions: &[DVec3]) -> DVec3 {
    let sum: DVec3 = positions.iter().sum();
    sum / positions.len() as f64
}

/// 生成一个共面多边形几何。
///
/// 映射到 CesiumJS `CoplanarPolygonGeometry.createGeometry`。
///
/// 先逐坐标去重并剔除退化输入，再计算并定向平面法线，最后委托内部的 `build_geometry` 完成投影与剖分。
pub fn coplanar_polygon_geometry(options: &CoplanarPolygonOptions, vf: VertexFormat) -> GeometryData {
    let ellipsoid = &options.ellipsoid;

    // 去除重复项。
    // 逐坐标比较 epsilon，避免共面点重复导致退化多边形。
    let mut positions: Vec<DVec3> = options.positions.clone();
    positions.dedup_by(|a, b| {
        (a.x - b.x).abs() <= EPSILON10
            && (a.y - b.y).abs() <= EPSILON10
            && (a.z - b.z).abs() <= EPSILON10
    });

    if positions.len() < 3 {
        return empty_geometry();
    }

    // 计算平面法线和坐标轴。
    // Newell 法得到的未定向法线，后续可能根据朝外要求翻转。
    let normal = compute_normal(&positions);

    // 确保法线朝外（远离椭球中心）。
    // 若与地表法线点积为负，则翻转法线后重新组装以保持绕序一致。
    let center = compute_center(&positions);
    if center.length_squared() > 1e-12 {
        let surface_normal = ellipsoid.geodetic_surface_normal(center).unwrap_or(DVec3::Z);
        if normal.dot(surface_normal) < 0.0 {
            // 翻转法线和 axis1 以保持一致的绕序。
            let normal = -normal;
            return build_geometry(&positions, normal, options.st_rotation, &vf);
        }
    }

    build_geometry(&positions, normal, options.st_rotation, &vf)
}

/// 组装共面多边形几何：投影、三角剖分并逐顶点生成法线/切线/副切线/UV。
///
/// # 参数
/// - `positions`：已去重且至少三点的多边形顶点。
/// - `normal`：已定向为朝外的平面法线。
/// - `st_rotation`：纹理坐标绕中心的旋转角（弧度）。
/// - `vf`：顶点格式，决定是否附带各属性。
///
/// # 返回
/// [`GeometryData`]：三角形表示的共面多边形；投影后三角剖分为空时回退为空几何。
fn build_geometry(
    positions: &[DVec3],
    normal: DVec3,
    st_rotation: f64,
    vf: &VertexFormat,
) -> GeometryData {
    let n = positions.len();

    // 计算平面坐标轴。
    // axis1 由 compute_axis1 选一个与法线最不对齐的世界轴叉积得到，axis2 再由法线×axis1 导出。
    let axis1 = compute_axis1(normal);
    let axis2 = normal.cross(axis1).normalize_or(DVec3::Y);

    // 将位置投影到 2D。
    // 先减去形心平移到局部原点，再分别投影到 axis1/axis2 上得到平面坐标。
    let center = compute_center(positions);
    let positions_2d: Vec<glam::DVec2> = positions
        .iter()
        .map(|&p| {
            let v = p - center;
            glam::DVec2::new(v.dot(axis1), v.dot(axis2))
        })
        .collect();

    // 三角剖分。
    // 在投影后的 2D 坐标上以耳切法剖分（无孔洞），失败时回退为空几何。
    let indices = triangulate_polygon(&positions_2d, &[]);
    if indices.is_empty() {
        return empty_geometry();
    }

    // 计算用于 ST 的包围矩形。
    // 遍历投影后的 2D 坐标统计 x/y 极值，用作后续归一化分母。
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for p in &positions_2d {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    let width = (max_x - min_x).max(1e-10);
    let height = (max_y - min_y).max(1e-10);

    // 若需要则应用 ST 旋转。
    // 旋转接近 0 时直接用单位旋转避免无谓三角函数开销。
    let (cos_r, sin_r) = if st_rotation.abs() > 1e-15 {
        (st_rotation.cos(), st_rotation.sin())
    } else {
        (1.0, 0.0)
    };

    // 生成顶点属性。
    // 位置必带，其余数组按 vf 选择是否为 Some，预留容量避免重分配。
    let mut pos_out: Vec<[f64; 3]> = Vec::with_capacity(n);
    let mut normals_out: Option<Vec<[f64; 3]>> = if vf.normal { Some(Vec::with_capacity(n)) } else { None };
    let mut tangents_out: Option<Vec<[f64; 3]>> = if vf.tangent { Some(Vec::with_capacity(n)) } else { None };
    let mut bitangents_out: Option<Vec<[f64; 3]>> = if vf.bitangent { Some(Vec::with_capacity(n)) } else { None };
    let mut st_out: Option<Vec<[f64; 2]>> = if vf.st { Some(Vec::with_capacity(n)) } else { None };

    for (i, &p) in positions.iter().enumerate() {
        // 共面多边形所有顶点共用同一法线与平面坐标轴，仅 UV 逐点不同。
        pos_out.push([p.x, p.y, p.z]);

        if let Some(ref mut norms) = normals_out {
            norms.push([normal.x, normal.y, normal.z]);
        }
        if let Some(ref mut tans) = tangents_out {
            tans.push([axis1.x, axis1.y, axis1.z]);
        }
        if let Some(ref mut bits) = bitangents_out {
            bits.push([axis2.x, axis2.y, axis2.z]);
        }
        if let Some(ref mut st) = st_out {
            let p2d = positions_2d[i];
            // 绕中心应用旋转。
            // 先把平面坐标绕原点旋转 st_rotation，再除以包围矩形宽高归一化到 [0,1]。
            let rx = p2d.x * cos_r - p2d.y * sin_r;
            let ry = p2d.x * sin_r + p2d.y * cos_r;
            let stx = ((rx - min_x) / width).clamp(0.0, 1.0);
            let sty = ((ry - min_y) / height).clamp(0.0, 1.0);
            st.push([stx, sty]);
        }
    }

    let bounding_sphere = BoundingSphere::from_points(
        &pos_out.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

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

/// 计算一个垂直于法线的向量（平面的 axis1）。
/// 选与法线对齐程度最低的世界轴作叉积，避免两向量近平行时叉积退化。
fn compute_axis1(normal: DVec3) -> DVec3 {
    // 选择与法线对齐程度最低的世界轴进行叉乘。
    let candidate = if normal.x.abs() <= normal.y.abs() && normal.x.abs() <= normal.z.abs() {
        DVec3::X
    } else if normal.y.abs() <= normal.z.abs() {
        DVec3::Y
    } else {
        DVec3::Z
    };
    normal.cross(candidate).normalize_or(DVec3::X)
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

    fn coplanar_opts() -> CoplanarPolygonOptions {
        let ell = Ellipsoid::WGS84;
        // 表面上的一个四边形（在小区域内大致共面）。
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-70.0, 40.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-70.0, 38.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 38.0, 0.0)),
        ];
        CoplanarPolygonOptions {
            positions,
            st_rotation: 0.0,
            ellipsoid: ell,
        }
    }

    #[test]
    /// 验证四边形共面多边形生成 4 顶点/2 三角形且法线与 UV 均就位。
    fn test_coplanar_basic() {
        let geo = coplanar_polygon_geometry(&coplanar_opts(), VertexFormat::ALL);
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
        assert_eq!(geo.indices.len() % 3, 0);
        // 4 个顶点，扇形三角剖分 = 2 个三角形 = 6 个索引。
        assert_eq!(geo.positions.len(), 4);
        assert_eq!(geo.indices.len(), 6);
        assert!(geo.normals.is_some());
        assert!(geo.tex_coords.is_some());
    }

    #[test]
    /// 验证三角形输入生成 3 顶点/1 三角形。
    fn test_coplanar_triangle() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.5, 1.0, 0.0)),
        ];
        let opts = CoplanarPolygonOptions {
            positions,
            ..Default::default()
        };
        let geo = coplanar_polygon_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert_eq!(geo.positions.len(), 3);
        assert_eq!(geo.indices.len(), 3);
    }

    #[test]
    /// 验证位置不足三点时退化为空几何。
    fn test_coplanar_too_few_positions() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        ];
        let opts = CoplanarPolygonOptions {
            positions,
            ..Default::default()
        };
        let geo = coplanar_polygon_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.is_empty());
    }

    #[test]
    /// 验证共面多边形所有顶点法线一致且朝外。
    fn test_coplanar_normals_consistent() {
        let geo = coplanar_polygon_geometry(&coplanar_opts(), VertexFormat::ALL);
        let normals = geo.normals.unwrap();
        // 所有法线应相同（共面多边形）。
        let n0 = DVec3::new(normals[0][0], normals[0][1], normals[0][2]);
        for n in &normals {
            let ni = DVec3::new(n[0], n[1], n[2]);
            assert!((n0 - ni).length() < 1e-10);
        }
        // 法线应朝外。
        let center = compute_center(
            &geo.positions.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
        );
        assert!(center.dot(n0) > 0.0);
    }

    #[test]
    /// 验证五边形共面多边形生成 5 顶点/3 三角形。
    fn test_coplanar_pentagon() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.5, 0.5, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.5, 1.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-0.5, 0.5, 0.0)),
        ];
        let opts = CoplanarPolygonOptions {
            positions,
            ..Default::default()
        };
        let geo = coplanar_polygon_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert_eq!(geo.positions.len(), 5);
        // 扇形三角剖分：5-2 = 3 个三角形 = 9 个索引。
        assert_eq!(geo.indices.len(), 9);
    }
}
