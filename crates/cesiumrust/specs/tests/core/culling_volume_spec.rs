//! 参考自 CullingVolumeSpec（43 个 it()，40 个 A 类）
//!
//! 3 个 throws = C 类（Rust 类型系统强制输入合法）。
//! 每个 A 类测试同时验证 computeVisibility 和 computeVisibilityWithPlaneMask。

use cesium_geospatial::bounding::{AxisAlignedBoundingBox, BoundingSphere};
use cesium_geospatial::frustum::{Cullable, CullingVolume, PerspectiveFrustum};
use cesium_geospatial::ray::Intersect;
use glam::DVec3;
use std::f64::consts::PI;

/// 构造标准测试用剔除体：透视视锥 fov=PI/3、aspect=1、near=1、far=2，
/// 位于原点、朝 -Z 方向观察、up=+Y。
fn create_culling_volume() -> CullingVolume {
    let frustum = PerspectiveFrustum::new(PI / 3.0, 1.0, 1.0, 2.0);
    frustum.compute_culling_volume(DVec3::ZERO, -DVec3::Z, DVec3::Y)
}

/// 复刻 CesiumJS 测试辅助函数 `testWithAndWithoutPlaneMask`。
fn assert_visibility(cv: &CullingVolume, volume: &impl Cullable, expected: Intersect) {
    // 测试 computeVisibility
    assert_eq!(cv.visibility(volume), expected);

    // 测试 computeVisibilityWithPlaneMask
    let mask = cv.visibility_with_plane_mask(volume, CullingVolume::MASK_INDETERMINATE);
    match expected {
        Intersect::Inside => assert_eq!(mask, CullingVolume::MASK_INSIDE),
        Intersect::Outside => assert_eq!(mask, CullingVolume::MASK_OUTSIDE),
        Intersect::Intersecting => {
            assert_ne!(mask, CullingVolume::MASK_INSIDE);
            assert_ne!(mask, CullingVolume::MASK_OUTSIDE);
        }
    }
    // 幂等性：再次应用该掩码会返回相同的掩码
    assert_eq!(cv.visibility_with_plane_mask(volume, mask), mask);
}

// ===== 包围盒相交 =====

#[test]
fn culling_box_inside() {
    let cv = create_culling_volume();
    let box1 = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, 0.0, -1.25),
        DVec3::new(0.5, 0.0, -1.25),
        DVec3::new(-0.5, 0.0, -1.75),
        DVec3::new(0.5, 0.0, -1.75),
    ]);
    assert_visibility(&cv, &box1, Intersect::Inside);
}

#[test]
fn culling_box_intersect_far() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, 0.0, -1.5),
        DVec3::new(0.5, 0.0, -1.5),
        DVec3::new(-0.5, 0.0, -2.5),
        DVec3::new(0.5, 0.0, -2.5),
    ]);
    assert_visibility(&cv, &b, Intersect::Intersecting);
}

#[test]
fn culling_box_intersect_near() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, 0.0, -0.5),
        DVec3::new(0.5, 0.0, -0.5),
        DVec3::new(-0.5, 0.0, -1.5),
        DVec3::new(0.5, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &b, Intersect::Intersecting);
}

#[test]
fn culling_box_intersect_left() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-1.5, 0.0, -1.25),
        DVec3::new(0.0, 0.0, -1.25),
        DVec3::new(-1.5, 0.0, -1.5),
        DVec3::new(0.0, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &b, Intersect::Intersecting);
}

#[test]
fn culling_box_intersect_right() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(0.0, 0.0, -1.25),
        DVec3::new(1.5, 0.0, -1.25),
        DVec3::new(0.0, 0.0, -1.5),
        DVec3::new(1.5, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &b, Intersect::Intersecting);
}

#[test]
fn culling_box_intersect_top() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, 0.0, -1.25),
        DVec3::new(0.5, 0.0, -1.25),
        DVec3::new(-0.5, 2.0, -1.75),
        DVec3::new(0.5, 2.0, -1.75),
    ]);
    assert_visibility(&cv, &b, Intersect::Intersecting);
}

#[test]
fn culling_box_intersect_bottom() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, -2.0, -1.25),
        DVec3::new(0.5, 0.0, -1.25),
        DVec3::new(-0.5, -2.0, -1.5),
        DVec3::new(0.5, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &b, Intersect::Intersecting);
}

#[test]
fn culling_box_outside_far() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, 0.0, -2.25),
        DVec3::new(0.5, 0.0, -2.25),
        DVec3::new(-0.5, 0.0, -2.75),
        DVec3::new(0.5, 0.0, -2.75),
    ]);
    assert_visibility(&cv, &b, Intersect::Outside);
}

#[test]
fn culling_box_outside_near() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, 0.0, -0.25),
        DVec3::new(0.5, 0.0, -0.25),
        DVec3::new(-0.5, 0.0, -0.75),
        DVec3::new(0.5, 0.0, -0.75),
    ]);
    assert_visibility(&cv, &b, Intersect::Outside);
}

#[test]
fn culling_box_outside_left() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-5.0, 0.0, -1.25),
        DVec3::new(-3.0, 0.0, -1.25),
        DVec3::new(-5.0, 0.0, -1.75),
        DVec3::new(-3.0, 0.0, -1.75),
    ]);
    assert_visibility(&cv, &b, Intersect::Outside);
}

#[test]
fn culling_box_outside_right() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(3.0, 0.0, -1.25),
        DVec3::new(5.0, 0.0, -1.25),
        DVec3::new(3.0, 0.0, -1.75),
        DVec3::new(5.0, 0.0, -1.75),
    ]);
    assert_visibility(&cv, &b, Intersect::Outside);
}

#[test]
fn culling_box_outside_top() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, 3.0, -1.25),
        DVec3::new(0.5, 3.0, -1.25),
        DVec3::new(-0.5, 5.0, -1.75),
        DVec3::new(0.5, 5.0, -1.75),
    ]);
    assert_visibility(&cv, &b, Intersect::Outside);
}

#[test]
fn culling_box_outside_bottom() {
    let cv = create_culling_volume();
    let b = AxisAlignedBoundingBox::from_points(&[
        DVec3::new(-0.5, -3.0, -1.25),
        DVec3::new(0.5, -3.0, -1.25),
        DVec3::new(-0.5, -5.0, -1.75),
        DVec3::new(0.5, -5.0, -1.75),
    ]);
    assert_visibility(&cv, &b, Intersect::Outside);
}

// ===== 球体相交 =====

#[test]
fn culling_sphere_inside() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, 0.0, -1.25),
        DVec3::new(0.0, 0.0, -1.75),
    ]);
    assert_visibility(&cv, &s, Intersect::Inside);
}

#[test]
fn culling_sphere_intersect_far() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, 0.0, -1.5),
        DVec3::new(0.0, 0.0, -2.5),
    ]);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_sphere_intersect_near() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, 0.0, -0.5),
        DVec3::new(0.0, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_sphere_intersect_left() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(-1.0, 0.0, -1.5),
        DVec3::new(0.0, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_sphere_intersect_right() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, 0.0, -1.5),
        DVec3::new(1.0, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_sphere_intersect_top() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, 0.0, -1.5),
        DVec3::new(0.0, 2.0, -1.5),
    ]);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_sphere_intersect_bottom() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, -2.0, -1.5),
        DVec3::new(0.0, 0.0, -1.5),
    ]);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_sphere_outside_far() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, 0.0, -2.25),
        DVec3::new(0.0, 0.0, -2.75),
    ]);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_sphere_outside_near() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(0.0, 0.0, -0.25),
        DVec3::new(0.0, 0.0, -0.5),
    ]);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_sphere_outside_left() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(-5.0, 0.0, -1.25),
        DVec3::new(-4.5, 0.0, -1.75),
    ]);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_sphere_outside_right() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(4.5, 0.0, -1.25),
        DVec3::new(5.0, 0.0, -1.75),
    ]);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_sphere_outside_top() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(-0.5, 4.5, -1.25),
        DVec3::new(-0.5, 5.0, -1.25),
    ]);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_sphere_outside_bottom() {
    let cv = create_culling_volume();
    let s = BoundingSphere::from_points(&[
        DVec3::new(-0.5, -4.5, -1.25),
        DVec3::new(-0.5, -5.0, -1.25),
    ]);
    assert_visibility(&cv, &s, Intersect::Outside);
}

// ===== 从包围球构造 =====

const BS_CENTER: DVec3 = DVec3::new(1000.0, 2000.0, 3000.0);
const BS_RADIUS: f64 = 100.0;

fn from_sphere_culling_volume() -> CullingVolume {
    let sphere = BoundingSphere::new(BS_CENTER, BS_RADIUS);
    CullingVolume::from_bounding_sphere(&sphere)
}

#[test]
fn culling_from_sphere_inside() {
    let cv = from_sphere_culling_volume();
    let s = BoundingSphere::new(BS_CENTER, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Inside);
}

#[test]
fn culling_from_sphere_intersect_far() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, 0.0, BS_RADIUS * 1.5);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_from_sphere_intersect_near() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, 0.0, -BS_RADIUS * 1.5);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_from_sphere_intersect_left() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(-BS_RADIUS * 1.5, 0.0, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_from_sphere_intersect_right() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(BS_RADIUS * 1.5, 0.0, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_from_sphere_intersect_top() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, BS_RADIUS * 1.5, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_from_sphere_intersect_bottom() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, -BS_RADIUS * 1.5, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Intersecting);
}

#[test]
fn culling_from_sphere_outside_far() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, 0.0, BS_RADIUS * 2.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_from_sphere_outside_near() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, 0.0, -BS_RADIUS * 2.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_from_sphere_outside_left() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(-BS_RADIUS * 2.0, 0.0, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_from_sphere_outside_right() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(BS_RADIUS * 2.0, 0.0, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_from_sphere_outside_top() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, BS_RADIUS * 2.0, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Outside);
}

#[test]
fn culling_from_sphere_outside_bottom() {
    let cv = from_sphere_culling_volume();
    let center = BS_CENTER + DVec3::new(0.0, -BS_RADIUS * 2.0, 0.0);
    let s = BoundingSphere::new(center, BS_RADIUS * 0.5);
    assert_visibility(&cv, &s, Intersect::Outside);
}
