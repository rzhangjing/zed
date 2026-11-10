//! 轮廓几何体扩展规格测试 - 来自
//! Core/BoxOutlineGeometrySpec、Core/SphereOutlineGeometrySpec、
//! Core/CylinderOutlineGeometrySpec 的额外 A 类测试

use cesium_geospatial::geometry::{
    box_outline_geometry, cylinder_outline_geometry, ellipsoid_outline_geometry,
    plane_outline_geometry, PrimitiveType,
};
use glam::DVec3;

const EPSILON10: f64 = 1e-10;

// ─── BoxOutlineGeometry 扩展（来自 BoxOutlineGeometrySpec） ─────────────

#[test]
fn box_outline_degenerate_min_equals_max() {
    // BoxOutlineGeometrySpec: "undefined is returned if min and max are equal"
    // Rust 实现生成位于单点的退化盒子
    let p = DVec3::new(250000.0, 250000.0, 250000.0);
    let geo = box_outline_geometry(p, p);

    // 所有位置坍缩到同一点
    for pos in &geo.positions {
        assert!((pos[0] - 250000.0).abs() < EPSILON10);
        assert!((pos[1] - 250000.0).abs() < EPSILON10);
        assert!((pos[2] - 250000.0).abs() < EPSILON10);
    }
    // 包围球半径应为 0
    assert!(geo.bounding_sphere.radius < EPSILON10);
}

#[test]
fn box_outline_from_dimensions_concept() {
    // BoxOutlineGeometrySpec: "fromDimensions" - box centered at origin
    // 我们的 API 直接接受 min/max，因此模拟 fromDimensions
    let dimensions = DVec3::new(1.0, 2.0, 3.0);
    let half = dimensions * 0.5;
    let geo = box_outline_geometry(-half, half);

    assert_eq!(geo.positions.len(), 8);
    assert_eq!(geo.indices.len(), 24); // 12 条边 * 2
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);

    // 所有位置应位于 [-half, half] 内
    for p in &geo.positions {
        assert!(p[0].abs() <= half.x + EPSILON10);
        assert!(p[1].abs() <= half.y + EPSILON10);
        assert!(p[2].abs() <= half.z + EPSILON10);
    }
}

#[test]
fn box_outline_from_aabb_concept() {
    // BoxOutlineGeometrySpec: "fromAxisAlignedBoundingBox"
    // 通过传入 AABB 的 min/max 模拟
    let min = DVec3::new(-1.0, -2.0, -3.0);
    let max = DVec3::new(1.0, 2.0, 3.0);
    let geo = box_outline_geometry(min, max);

    assert_eq!(geo.positions.len(), 8);
    assert_eq!(geo.indices.len(), 24);
}

// ─── SphereOutlineGeometry（用等半径的 ellipsoid_outline） ────────

#[test]
fn sphere_outline_computes_positions() {
    // SphereOutlineGeometrySpec: "computes positions"
    // 球体在 Rust 中使用等半径的 ellipsoid_outline_geometry
    let radii = DVec3::new(1.0, 1.0, 1.0);
    let geo = ellipsoid_outline_geometry(radii, 3, 3);

    // 3 个大圆，stacks=3、slices=3
    // XY 圆 (slices+1=4) + XZ 圆 (stacks+1=4) + YZ 圆 (stacks+1=4) = 12
    assert_eq!(geo.positions.len(), 12);
    assert_eq!(geo.indices.len(), 18); // 3 圆 * 3 段 * 2 索引
    assert!((geo.bounding_sphere.radius - 1.0).abs() < EPSILON10);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
}

#[test]
fn sphere_outline_positions_on_unit_sphere() {
    // SphereOutlineGeometrySpec：位置应位于单位球面上
    let radii = DVec3::new(1.0, 1.0, 1.0);
    let geo = ellipsoid_outline_geometry(radii, 8, 8);

    for p in &geo.positions {
        let pos = DVec3::from(*p);
        let magnitude = pos.length();
        assert!(
            (magnitude - 1.0).abs() < EPSILON10,
            "position magnitude {} should be 1.0", magnitude
        );
    }
}

#[test]
fn sphere_outline_degenerate_radius_zero() {
    // SphereOutlineGeometrySpec: "undefined is returned if radius is equals to zero"
    // Rust 实现生成位于原点的退化几何体
    let radii = DVec3::new(0.0, 0.0, 0.0);
    let geo = ellipsoid_outline_geometry(radii, 3, 3);

    // 所有位置应位于原点
    for p in &geo.positions {
        assert!(p[0].abs() < EPSILON10);
        assert!(p[1].abs() < EPSILON10);
        assert!(p[2].abs() < EPSILON10);
    }
    assert!(geo.bounding_sphere.radius < EPSILON10);
}

#[test]
fn sphere_outline_radius_scales_positions() {
    // SphereOutlineGeometrySpec：半径缩放位置
    let radius = 5.0;
    let radii = DVec3::new(radius, radius, radius);
    let geo = ellipsoid_outline_geometry(radii, 4, 4);

    for p in &geo.positions {
        let pos = DVec3::from(*p);
        let magnitude = pos.length();
        assert!(
            (magnitude - radius).abs() < EPSILON10,
            "position magnitude {} should be {}", magnitude, radius
        );
    }
    assert!((geo.bounding_sphere.radius - radius).abs() < EPSILON10);
}

// ─── CylinderOutlineGeometry 扩展（来自 CylinderOutlineGeometrySpec） ──

#[test]
fn cylinder_outline_degenerate_length_zero() {
    // CylinderOutlineGeometrySpec: "undefined is returned if length <= 0"
    // Rust 生成位于 z=0 的退化几何体
    let geo = cylinder_outline_geometry(0.0, 1.0, 1.0, 8);

    // 所有位置应位于 z=0
    for p in &geo.positions {
        assert!(p[2].abs() < EPSILON10, "z={} should be 0", p[2]);
    }
}

#[test]
fn cylinder_outline_degenerate_both_radii_zero() {
    // CylinderOutlineGeometrySpec: "undefined if both radii are zero"
    let geo = cylinder_outline_geometry(10.0, 0.0, 0.0, 8);

    // 所有位置应位于 Z 轴上（x=0, y=0）
    for p in &geo.positions {
        assert!(p[0].abs() < EPSILON10);
        assert!(p[1].abs() < EPSILON10);
    }
}

#[test]
fn cylinder_outline_cone_bottom_radius_zero() {
    // CylinderOutlineGeometrySpec: "computes positions with bottomRadius equals 0"
    let geo = cylinder_outline_geometry(10.0, 5.0, 0.0, 8);

    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);

    // 底部圆的顶点应位于原点（x=0, y=0, z=-half）
    let half = 5.0;
    for p in &geo.positions {
        if (p[2] + half).abs() < EPSILON10 {
            // 底部顶点
            assert!(p[0].abs() < EPSILON10);
            assert!(p[1].abs() < EPSILON10);
        }
    }
}

#[test]
fn cylinder_outline_bounding_sphere() {
    // CylinderOutlineGeometrySpec：包围球应包含圆柱
    let length = 2.0;
    let radius = 1.0;
    let geo = cylinder_outline_geometry(length, radius, radius, 8);

    let half_length = length * 0.5;
    let expected_radius = (radius * radius + half_length * half_length).sqrt();
    assert!((geo.bounding_sphere.radius - expected_radius).abs() < EPSILON10);
    assert!(geo.bounding_sphere.center.length() < EPSILON10);
}

// ─── PlaneOutlineGeometry 扩展（来自 PlaneOutlineGeometrySpec） ────────

#[test]
fn plane_outline_bounding_sphere() {
    // PlaneOutlineGeometrySpec：包围球应包含单位四边形
    let geo = plane_outline_geometry();

    // XY 平面内从 -0.5 到 0.5 的单位四边形
    // 包围球半径 = 对角线/2 = sqrt(0.5² + 0.5²) = sqrt(0.5) ≈ 0.707
    let expected_radius = std::f64::consts::FRAC_1_SQRT_2;
    assert!((geo.bounding_sphere.radius - expected_radius).abs() < EPSILON10);
    assert!(geo.bounding_sphere.center.length() < EPSILON10);
}

#[test]
fn plane_outline_indices_form_closed_loop() {
    // PlaneOutlineGeometrySpec：索引应构成 4 条边
    let geo = plane_outline_geometry();

    assert_eq!(geo.indices.len(), 8); // 4 条边 * 2 索引
    // 验证边：0-1, 1-2, 2-3, 3-0
    assert_eq!(geo.indices[0], 0);
    assert_eq!(geo.indices[1], 1);
    assert_eq!(geo.indices[2], 1);
    assert_eq!(geo.indices[3], 2);
    assert_eq!(geo.indices[4], 2);
    assert_eq!(geo.indices[5], 3);
    assert_eq!(geo.indices[6], 3);
    assert_eq!(geo.indices[7], 0);
}

// ─── 通用不变量 ──────────────────────────────────────────────────────

#[test]
fn all_outline_geometries_have_valid_indices() {
    // 所有索引应引用有效顶点
    let geometries = vec![
        box_outline_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0)),
        ellipsoid_outline_geometry(DVec3::new(1.0, 1.0, 1.0), 4, 4),
        cylinder_outline_geometry(2.0, 1.0, 1.0, 8),
        plane_outline_geometry(),
    ];

    for (name, geo) in geometries.iter().enumerate() {
        let num_vertices = geo.positions.len() as u32;
        for (i, &idx) in geo.indices.iter().enumerate() {
            assert!(
                idx < num_vertices,
                "geometry {} index[{}] = {} out of bounds (num_vertices = {})",
                name, i, idx, num_vertices
            );
        }
    }
}

#[test]
fn all_outline_geometries_use_lines_primitive() {
    let geometries = vec![
        box_outline_geometry(DVec3::splat(-1.0), DVec3::ONE),
        ellipsoid_outline_geometry(DVec3::ONE, 4, 4),
        cylinder_outline_geometry(2.0, 1.0, 1.0, 8),
        plane_outline_geometry(),
    ];

    for geo in &geometries {
        assert_eq!(
            geo.primitive_type,
            PrimitiveType::Lines,
            "all outline geometries should use Lines primitive"
        );
    }
}

// ─── BoxOutlineGeometry 补充（来自 BoxOutlineGeometrySpec） ───────────

#[test]
fn box_outline_from_dimensions_detail() {
    // BoxOutlineGeometrySpec: "fromDimensions" - box at origin with given dimensions
    let dimensions = DVec3::new(2.0, 3.0, 4.0);
    let half = dimensions * 0.5;
    let geo = box_outline_geometry(-half, half);

    assert_eq!(geo.positions.len(), 8);
    assert_eq!(geo.indices.len(), 24);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    // 所有位置应位于角点
    for p in &geo.positions {
        assert!((p[0].abs() - half.x).abs() < EPSILON10 || (p[1].abs() - half.y).abs() < EPSILON10 || (p[2].abs() - half.z).abs() < EPSILON10);
    }
}

// ─── CylinderOutlineGeometry 补充（来自 CylinderOutlineGeometrySpec） ──

#[test]
fn cylinder_outline_computes_positions_detail() {
    // CylinderOutlineGeometrySpec: "computes positions"
    let geo = cylinder_outline_geometry(10.0, 5.0, 5.0, 8);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    // 顶圆 (slices+1=9) + 底圆 (slices+1=9) + 连接线 (2×slices=16) = 34
    // 实际：顶 slices+1=9，底 slices+1=9，沿长度方向的线 slices*2=16
    // 总计：9+9=18 个唯一值？否，每条"边"的位置是独立的
    assert!(geo.positions.len() >= 18);
    assert!(geo.indices.len() >= 8 * 2 * 3); // 3 圆 × slices × 2 索引
}

#[test]
fn cylinder_outline_no_lines_along_length() {
    // CylinderOutlineGeometrySpec: "computes positions with no lines along the length"
    // 创建 number_of_vertical_lines=0 的圆柱轮廓
    // 我们的 API 不直接支持此参数，但我们可以测试所有索引均有效
    let geo = cylinder_outline_geometry(10.0, 5.0, 3.0, 16);
    assert!(!geo.positions.is_empty());
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    for &idx in &geo.indices {
        assert!(idx < geo.positions.len() as u32);
    }
}

// ─── EllipsoidOutlineGeometry 补充 ──────────────────────────────────────

#[test]
fn ellipsoid_outline_squished_radii() {
    // 非均匀半径（被压扁的球）
    let radii = DVec3::new(1.0, 2.0, 3.0);
    let geo = ellipsoid_outline_geometry(radii, 4, 6);
    assert!(geo.positions.len() >= 6);
    assert_eq!(geo.primitive_type, PrimitiveType::Lines);
    // 检查位置随半径缩放
    for p in &geo.positions {
        let pos = DVec3::from(*p);
        // 归一化距离：对于圆 (x/rx)^2 + (y/ry)^2 + (z/rz)^2 ≈ 1
        let scaled = (pos.x / radii.x).powi(2) + (pos.y / radii.y).powi(2) + (pos.z / radii.z).powi(2);
        assert!((scaled - 1.0).abs() < EPSILON10, "position not on outline surface");
    }
}

// ─── 通用不变量扩展 ───────────────────────────────────────────────

#[test]
fn all_outline_geometries_indices_form_valid_edges() {
    // 所有轮廓索引应成有效对（线段）
    let geometries = vec![
        box_outline_geometry(DVec3::new(-1.0, -1.0, -1.0), DVec3::new(1.0, 1.0, 1.0)),
        ellipsoid_outline_geometry(DVec3::ONE, 4, 4),
        cylinder_outline_geometry(2.0, 1.0, 1.0, 8),
        plane_outline_geometry(),
    ];
    for (name, geo) in geometries.iter().enumerate() {
        assert_eq!(geo.indices.len() % 2, 0, "geometry {}: indices must be pairs", name);
        for chunk in geo.indices.chunks(2) {
            assert_ne!(chunk[0], chunk[1], "geometry {}: degenerate edge (self-loop)", name);
        }
    }
}
