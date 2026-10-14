//! 视锥体几何（相机视锥可视化）。
//!
//! 视锥体由其 8 个角点（4 个近 + 4 个远）构建，这些角点通过将 NDC 角点
//! 经逆视图-投影矩阵反投影而得，然后组装为 6 个四边形平面（近、远、-x、-y、+x、+y）。

use crate::bounding::BoundingSphere;
use crate::frustum::{OrthographicFrustum, PerspectiveFrustum};
use crate::geometry::{GeometryData, PrimitiveType, VertexFormat};
use glam::{DMat3, DMat4, DQuat, DVec3, DVec4};

/// 一个视锥体定义（透视或正交）。
pub enum FrustumDef {
    /// 透视视锥（由 fov/aspect/near/far 定义）。
    Perspective(PerspectiveFrustum),
    /// 正交视锥（由 width/height/near/far 定义）。
    Orthographic(OrthographicFrustum),
}

/// 远平面的 NDC 角点 (x, y, z=1, w=1)。
const FRUSTUM_CORNERS_NDC: [[f64; 4]; 4] = [
    [-1.0, -1.0, 1.0, 1.0],
    [1.0, -1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0, 1.0],
    [-1.0, 1.0, 1.0, 1.0],
];

/// 视图矩阵构造（`Matrix4.computeView(position, direction, up, right)`）：以行主序
/// 将矩阵排列为 `[right; up; -direction]`，平移位于最后一列。glam 为列主序，
/// 因此我们直接提供各列。
fn compute_view(position: DVec3, direction: DVec3, up: DVec3, right: DVec3) -> DMat4 {
    DMat4::from_cols(
        DVec4::new(right.x, up.x, -direction.x, 0.0),
        DVec4::new(right.y, up.y, -direction.y, 0.0),
        DVec4::new(right.z, up.z, -direction.z, 0.0),
        DVec4::new(
            -right.dot(position),
            -up.dot(position),
            direction.dot(position),
            1.0,
        ),
    )
}

/// 计算视锥体的 8 个角点位置（先近平面，后远平面）。
///
/// 实现 `_computeNearFarPlanes`。返回按
/// `[near0, near1, near2, near3, far0, far1, far2, far3]` 排列的 8 个位置，
/// 其中角点顺序与 [`FRUSTUM_CORNERS_NDC`] 一致。
fn compute_near_far_planes(
    origin: DVec3,
    orientation: DQuat,
    frustum: &FrustumDef,
) -> Vec<[f64; 3]> {
    let rotation = DMat3::from_quat(orientation);
    let mut x = rotation.col(0).normalize();
    let y = rotation.col(1).normalize();
    // 取第三列作为看向方（-Z），故下方立即对 x 取负以对齐右手视坐标。
    let z = rotation.col(2).normalize();
    x = -x;

    let view = compute_view(origin, z, y, x);

    let mut positions = vec![[0.0f64; 3]; 8];

    match frustum {
        FrustumDef::Perspective(p) => {
            let projection = p.projection_matrix();
            let view_projection = projection * view;
            let inv_vp = view_projection.inverse();
            let splits = [p.near, p.far];

            for i in 0..2 {
                for j in 0..4 {
                    // 将 NDC 角点经逆 view-projection 反投影，再沿射线方向定位到具体 split 平面上。
                    let c = FRUSTUM_CORNERS_NDC[j];
                    let corner = inv_vp * DVec4::new(c[0], c[1], c[2], c[3]);
                    // 逆转透视除法。
                    let w = 1.0 / corner.w;
                    let mut corner3 = DVec3::new(corner.x, corner.y, corner.z) * w;

                    corner3 = (corner3 - origin).normalize();
                    let fac = z.dot(corner3);
                    corner3 = corner3 * (splits[i] / fac) + origin;

                    positions[4 * i + j] = [corner3.x, corner3.y, corner3.z];
                }
            }
        }
        FrustumDef::Orthographic(o) => {
            // 正交：无需反透视除法，直接将 NDC 角点映回半宽/半高矩形后由 inv_view 变回世界。
            let inv_view = view.inverse();
            let right = o.width * 0.5;
            let left = -right;
            let top = o.height() * 0.5;
            let bottom = -top;
            // 对于正交投影，splits 为 [0, near, far]；迭代 i 使用
            // 位于距离 splits[i + 1] 处的平面。
            let splits = [0.0, o.near, o.far];

            for i in 0..2 {
                for j in 0..4 {
                    let c = FRUSTUM_CORNERS_NDC[j];
                    let cx = (c[0] * (right - left) + left + right) * 0.5;
                    let cy = (c[1] * (top - bottom) + bottom + top) * 0.5;
                    let cz = -splits[i + 1];
                    let corner = inv_view * DVec4::new(cx, cy, cz, 1.0);
                    positions[4 * i + j] = [corner.x, corner.y, corner.z];
                }
            }
        }
    }

    positions
}

/// 生成一个实心视锥体几何（6 个四边形平面）。
///
/// 映射到 CesiumJS `FrustumGeometry`。
pub fn frustum_geometry(
    frustum: &FrustumDef,
    origin: DVec3,
    orientation: DQuat,
    vf: VertexFormat,
) -> GeometryData {
    let corners = compute_near_far_planes(origin, orientation, frustum);

    // 构建 6 个平面 x 4 个顶点。近/远平面直接来自角点；四个侧面
    // 由角点组合而成（镜像 FrustumGeometry.createGeometry 中的索引算术）。
    let c = |k: usize| corners[k];
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(24);

    // 近平面（角点 0..4）。
    positions.extend_from_slice(&[c(0), c(1), c(2), c(3)]);
    // 远平面（角点 4..8）。
    positions.extend_from_slice(&[c(4), c(5), c(6), c(7)]);

    // 四个侧面由角点组合而成，镜像 FrustumGeometry.createGeometry 中的
    // 扁平索引算术。扁平角点数组为 [near0..near3, far0..far3]，因此扁平
    // 索引 k 在 k < 4 时映射到 near[k]，在 k >= 4 时映射到 far[k - 4]。
    let near = [c(0), c(1), c(2), c(3)];
    let far = [c(4), c(5), c(6), c(7)];
    // -x 平面：扁平 [4], [0], [3], [7] => near[0], near[3], far[3], far[0]
    positions.extend_from_slice(&[near[0], near[3], far[3], far[0]]);
    // -y 平面：扁平 [5], [1], [0], [4] => near[1], near[0], far[0], far[1]
    positions.extend_from_slice(&[near[1], near[0], far[0], far[1]]);
    // +x 平面：扁平 [1], [5], [6], [2] => near[1], far[1], far[2], near[2]
    positions.extend_from_slice(&[near[1], far[1], far[2], near[2]]);
    // +y 平面：扁平 [2], [6], [7], [3] => near[2], far[2], far[3], near[3]
    positions.extend_from_slice(&[near[2], far[2], far[3], near[3]]);

    let number_of_planes = 6usize;

    // 逐平面的常量属性。
    let rotation = DMat3::from_quat(orientation);
    let mut x = rotation.col(0).normalize();
    let y = rotation.col(1).normalize();
    let z = rotation.col(2).normalize();
    x = -x;
    let neg_x = -x;
    let neg_y = -y;
    let neg_z = -z;

    // 每个平面的 (法线、切线、副切线)，按 CesiumJS 顺序。
    let plane_attrs: [(DVec3, DVec3, DVec3); 6] = [
        (neg_z, x, y),    // 近
        (z, neg_x, y),    // 远
        (neg_x, neg_z, y),   // -x
        (neg_y, neg_z, neg_x), // -y
        (x, z, y),        // +x
        (y, z, neg_x),    // +y
    ];

    let mut normals: Option<Vec<[f64; 3]>> = if vf.normal { Some(Vec::new()) } else { None };
    let mut tangents: Option<Vec<[f64; 3]>> = if vf.tangent { Some(Vec::new()) } else { None };
    let mut bitangents: Option<Vec<[f64; 3]>> = if vf.bitangent { Some(Vec::new()) } else { None };
    let mut tex_coords: Option<Vec<[f64; 2]>> = if vf.st { Some(Vec::new()) } else { None };

    let st_quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    // 逐平面写入常量法线/切线/副切线（每面 4 顶点重复）与一整块 quad 贴图坐标。
    for (normal, tangent, bitangent) in &plane_attrs {
        for _ in 0..4 {
            if let Some(ref mut n) = normals {
                n.push([normal.x, normal.y, normal.z]);
            }
            if let Some(ref mut t) = tangents {
                t.push([tangent.x, tangent.y, tangent.z]);
            }
            if let Some(ref mut b) = bitangents {
                b.push([bitangent.x, bitangent.y, bitangent.z]);
            }
        }
        if let Some(ref mut st) = tex_coords {
            st.extend_from_slice(&st_quad);
        }
    }

    // 逐平面拆为两个三角形：索引 [0,1,2, 0,2,3]，共 6 个平面。
    let mut indices: Vec<u32> = Vec::with_capacity(6 * number_of_planes);
    for i in 0..number_of_planes {
        let index = (i * 4) as u32;
        indices.extend_from_slice(&[index, index + 1, index + 2, index, index + 2, index + 3]);
    }

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

/// 生成一个视锥体线框几何（作为线段序列的 12 条边）。
///
/// 映射到 CesiumJS `FrustumOutlineGeometry`。线框由 4 条近边、4 条远边和
/// 4 条连接边组成。
pub fn frustum_outline_geometry(
    frustum: &FrustumDef,
    origin: DVec3,
    orientation: DQuat,
) -> GeometryData {
    let corners = compute_near_far_planes(origin, orientation, frustum);
    let positions = corners;

    // 边：近环路、远环路，以及 4 条连接边。
    let mut indices: Vec<u32> = Vec::with_capacity(24);
    // 近环路 (0-1-2-3)。
    for i in 0..4u32 {
        indices.push(i);
        indices.push((i + 1) % 4);
    }
    // 远环路 (4-5-6-7)。
    for i in 0..4u32 {
        indices.push(4 + i);
        indices.push(4 + (i + 1) % 4);
    }
    // 连接边。
    for i in 0..4u32 {
        indices.push(i);
        indices.push(4 + i);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个透视视锥：60° 垂直视角、16:9 宽高比、near=1、far=100。
    fn perspective() -> FrustumDef {
        FrustumDef::Perspective(PerspectiveFrustum::new(
            std::f64::consts::FRAC_PI_3,
            16.0 / 9.0,
            1.0,
            100.0,
        ))
    }

    #[test]
    /// 验证实心视锥几何的顶点/索引/法线/贴图坐标数量与图元类型。
    fn test_frustum_geometry_counts() {
        let geo = frustum_geometry(&perspective(), DVec3::ZERO, DQuat::IDENTITY, VertexFormat::ALL);
        assert_eq!(geo.positions.len(), 24); // 6 个平面 x 4 个顶点
        assert_eq!(geo.indices.len(), 36); // 6 个平面 x 2 个三角形 x 3
        assert_eq!(geo.normals.as_ref().unwrap().len(), 24);
        assert_eq!(geo.tex_coords.as_ref().unwrap().len(), 24);
        assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
    }

    #[test]
    /// 验证透视视锥的近平面角点位于 near 深度、远平面角点位于 far 深度。
    fn test_frustum_corners_at_correct_depth() {
        // 在单位方位下视锥体沿 -Z 方向观看……近平面的
        // 角点都应位于沿视锥轴距离 `near` 处。
        let corners = compute_near_far_planes(DVec3::ZERO, DQuat::IDENTITY, &perspective());
        // 视锥轴是方位的 Z 列（单位 => +Z）。
        let axis = DVec3::Z;
        for corner in &corners[0..4] {
            let d = DVec3::new(corner[0], corner[1], corner[2]).dot(axis);
            assert!((d - 1.0).abs() < 1e-6, "near corner depth {}", d);
        }
        for corner in &corners[4..8] {
            let d = DVec3::new(corner[0], corner[1], corner[2]).dot(axis);
            assert!((d - 100.0).abs() < 1e-6, "far corner depth {}", d);
        }
    }

    #[test]
    /// 验证线框视锥由 8 个角点与 24 个索引（12 条边）组成。
    fn test_frustum_outline_counts() {
        let geo = frustum_outline_geometry(&perspective(), DVec3::ZERO, DQuat::IDENTITY);
        assert_eq!(geo.positions.len(), 8);
        assert_eq!(geo.indices.len(), 24); // 12 条边 x 2
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    }

    #[test]
    /// 验证正交视锥的顶点/索引数量，且近平面角点构成 width×height 矩形。
    fn test_orthographic_frustum() {
        let frustum = FrustumDef::Orthographic(OrthographicFrustum::new(10.0, 1.0, 1.0, 50.0));
        let geo = frustum_geometry(&frustum, DVec3::ZERO, DQuat::IDENTITY, VertexFormat::POSITION_ONLY);
        assert_eq!(geo.positions.len(), 24);
        assert_eq!(geo.indices.len(), 36);
        // 正交投影的近平面角点应构成一个 width x height 的矩形。
        let corners = compute_near_far_planes(DVec3::ZERO, DQuat::IDENTITY, &frustum);
        let xs: Vec<f64> = corners[0..4].iter().map(|c| c[0]).collect();
        let max_x = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!((max_x - 5.0).abs() < 1e-6, "half-width {}", max_x);
    }
}
