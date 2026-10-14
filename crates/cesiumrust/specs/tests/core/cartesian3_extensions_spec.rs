//! Core/Cartesian3Spec（CesiumJS 特有扩展）→ Rust 集成测试
//! 覆盖：fromSpherical、mostOrthogonalAxis、projectVector、midpoint、
//! equalsEpsilon、pack/unpack、fromDegrees、fromRadians、fromDegreesArray、
//! fromRadiansArray、fromDegreesArrayHeights、fromRadiansArrayHeights

use cesium_geospatial::cartesian3_ext;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::spherical::Spherical;
use cesium_geospatial::cartographic::Cartographic;
use glam::DVec3;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_3, FRAC_PI_4};

const EPS: f64 = 1e-10;

fn assert_vec3_eq(actual: DVec3, expected: DVec3, msg: &str) {
    assert!(
        (actual.x - expected.x).abs() < EPS
            && (actual.y - expected.y).abs() < EPS
            && (actual.z - expected.z).abs() < EPS,
        "{msg}: expected ({}, {}, {}), got ({}, {}, {})",
        expected.x, expected.y, expected.z,
        actual.x, actual.y, actual.z
    );
}

// ─── fromSpherical ──────────────────────────────────────────────────────────

#[test]
fn from_spherical_basic() {
    // 时钟角=60°、锥角=135°、幅值=√8
    let spherical = Spherical::new(FRAC_PI_3, FRAC_PI_4 + FRAC_PI_2, 8.0_f64.sqrt());
    let result = cartesian3_ext::from_spherical(&spherical);
    // 径向 = √8 * sin(135°) = √8 * √2/2 = 2
    // x = 2 * cos(60°) = 1, y = 2 * sin(60°) = √3, z = √8 * cos(135°) = -2
    let expected = DVec3::new(1.0, 3.0_f64.sqrt(), -2.0);
    assert_vec3_eq(result, expected, "fromSpherical");
}

#[test]
fn from_spherical_roundtrip() {
    let original = DVec3::new(1.0, 2.0, 3.0);
    let spherical = Spherical::from_cartesian3(original);
    let back = cartesian3_ext::from_spherical(&spherical);
    assert_vec3_eq(back, original, "fromSpherical roundtrip");
}

// ─── mostOrthogonalAxis ─────────────────────────────────────────────────────

#[test]
fn most_orthogonal_axis_x_dominant() {
    // (1, 0, 0) → 绝对值归一化 = (1, 0, 0) → y <= z → UNIT_Y
    let result = cartesian3_ext::most_orthogonal_axis(DVec3::new(1.0, 0.0, 0.0));
    assert_eq!(result, DVec3::Y);
}

#[test]
fn most_orthogonal_axis_y_dominant() {
    // (0, 1, 0) → 绝对值归一化 = (0, 1, 0) → x <= y, x <= z → UNIT_X
    let result = cartesian3_ext::most_orthogonal_axis(DVec3::new(0.0, 1.0, 0.0));
    assert_eq!(result, DVec3::X);
}

#[test]
fn most_orthogonal_axis_z_dominant() {
    // (0, 0, 1) → 绝对值归一化 = (0, 0, 1) → x <= y, x <= z → UNIT_X
    let result = cartesian3_ext::most_orthogonal_axis(DVec3::new(0.0, 0.0, 1.0));
    assert_eq!(result, DVec3::X);
}

#[test]
fn most_orthogonal_axis_mixed() {
    // (1, 2, 3) → 归一化绝对值 ≈ (0.267, 0.535, 0.802) → x <= y, x <= z → UNIT_X
    let result = cartesian3_ext::most_orthogonal_axis(DVec3::new(1.0, 2.0, 3.0));
    assert_eq!(result, DVec3::X);
}

#[test]
fn most_orthogonal_axis_y_smallest() {
    // (0.1, 0.9, 0.5) → 归一化绝对值：y 最大，x < z → x <= y 且 x <= z → UNIT_X
    // 实际：f.x=0.097, f.y=0.873, f.z=0.485 → x <= y, x <= z → UNIT_X
    let result = cartesian3_ext::most_orthogonal_axis(DVec3::new(0.1, 0.9, 0.5));
    assert_eq!(result, DVec3::X);
}

#[test]
fn most_orthogonal_axis_z_smallest() {
    // (0.5, 0.9, 0.1) → 归一化绝对值：f.x=0.485, f.y=0.873, f.z=0.097
    // x <= y？是。x <= z？否 → UNIT_Z
    let result = cartesian3_ext::most_orthogonal_axis(DVec3::new(0.5, 0.9, 0.1));
    assert_eq!(result, DVec3::Z);
}

// ─── projectVector ──────────────────────────────────────────────────────────

#[test]
fn project_vector_basic() {
    let a = DVec3::new(3.0, 4.0, 0.0);
    let b = DVec3::new(1.0, 0.0, 0.0);
    let result = cartesian3_ext::project_vector(a, b);
    assert_vec3_eq(result, DVec3::new(3.0, 0.0, 0.0), "projectVector");
}

#[test]
fn project_vector_diagonal() {
    let a = DVec3::new(1.0, 1.0, 0.0);
    let b = DVec3::new(1.0, 0.0, 0.0);
    let result = cartesian3_ext::project_vector(a, b);
    assert_vec3_eq(result, DVec3::new(1.0, 0.0, 0.0), "projectVector diagonal");
}

// ─── midpoint ───────────────────────────────────────────────────────────────

#[test]
fn midpoint_basic() {
    let left = DVec3::new(1.0, 2.0, 3.0);
    let right = DVec3::new(5.0, 6.0, 7.0);
    let result = cartesian3_ext::midpoint(left, right);
    assert_vec3_eq(result, DVec3::new(3.0, 4.0, 5.0), "midpoint");
}

#[test]
fn midpoint_negative() {
    let left = DVec3::new(-2.0, -4.0, -6.0);
    let right = DVec3::new(2.0, 4.0, 6.0);
    let result = cartesian3_ext::midpoint(left, right);
    assert_vec3_eq(result, DVec3::ZERO, "midpoint negative");
}

// ─── equalsEpsilon ──────────────────────────────────────────────────────────

#[test]
fn equals_epsilon_exact() {
    let a = DVec3::new(1.0, 2.0, 3.0);
    let b = DVec3::new(1.0, 2.0, 3.0);
    assert!(cartesian3_ext::equals_epsilon(a, b, 0.0, 0.0));
}

#[test]
fn equals_epsilon_within() {
    let a = DVec3::new(1.0, 2.0, 3.0);
    let b = DVec3::new(1.0 + 1e-12, 2.0 - 1e-12, 3.0 + 1e-12);
    assert!(cartesian3_ext::equals_epsilon(a, b, 0.0, 1e-10));
}

#[test]
fn equals_epsilon_outside() {
    let a = DVec3::new(1.0, 2.0, 3.0);
    let b = DVec3::new(2.0, 2.0, 3.0);
    assert!(!cartesian3_ext::equals_epsilon(a, b, 0.0, 0.5));
}

// ─── pack / unpack ──────────────────────────────────────────────────────────

#[test]
fn pack_basic() {
    let v = DVec3::new(1.0, 2.0, 3.0);
    let mut array = vec![0.0; 6];
    cartesian3_ext::pack(v, &mut array, 0);
    assert_eq!(array[0], 1.0);
    assert_eq!(array[1], 2.0);
    assert_eq!(array[2], 3.0);
}

#[test]
fn pack_with_offset() {
    let v = DVec3::new(4.0, 5.0, 6.0);
    let mut array = vec![0.0; 6];
    cartesian3_ext::pack(v, &mut array, 3);
    assert_eq!(array[3], 4.0);
    assert_eq!(array[4], 5.0);
    assert_eq!(array[5], 6.0);
}

#[test]
fn unpack_basic() {
    let array = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let result = cartesian3_ext::unpack(&array, 0);
    assert_eq!(result, DVec3::new(10.0, 20.0, 30.0));
}

#[test]
fn unpack_with_offset() {
    let array = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let result = cartesian3_ext::unpack(&array, 3);
    assert_eq!(result, DVec3::new(40.0, 50.0, 60.0));
}

#[test]
fn pack_unpack_roundtrip() {
    let v = DVec3::new(-1.5, 2.7, 3.14);
    let mut array = vec![0.0; 3];
    cartesian3_ext::pack(v, &mut array, 0);
    let result = cartesian3_ext::unpack(&array, 0);
    assert_eq!(result, v);
}

// ─── fromDegrees / fromRadians ──────────────────────────────────────────────

#[test]
fn from_degrees_matches_ellipsoid() {
    let ellipsoid = Ellipsoid::WGS84;
    let lon = -115.0;
    let lat = 37.0;
    let height = 100000.0;

    let actual = cartesian3_ext::from_degrees(lon, lat, height, &ellipsoid);
    let expected = ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(lon, lat, height));
    assert_vec3_eq(actual, expected, "fromDegrees matches ellipsoid");
}

#[test]
fn from_degrees_zero_height() {
    let ellipsoid = Ellipsoid::WGS84;
    let actual = cartesian3_ext::from_degrees(0.0, 0.0, 0.0, &ellipsoid);
    // 在 WGS84 上的 (0, 0)，位置应为（长半轴, 0, 0）
    let expected = DVec3::new(6378137.0, 0.0, 0.0);
    assert!(
        (actual.x - expected.x).abs() < 1e-3 && actual.y.abs() < 1e-3 && actual.z.abs() < 1e-3,
        "fromDegrees(0,0): expected ~({:.1}, 0, 0), got ({:.1}, {:.1}, {:.1})",
        expected.x, actual.x, actual.y, actual.z
    );
}

#[test]
fn from_radians_matches_ellipsoid() {
    let ellipsoid = Ellipsoid::WGS84;
    let lon = -2.007;
    let lat = 0.645;
    let height = 50000.0;

    let actual = cartesian3_ext::from_radians(lon, lat, height, &ellipsoid);
    let expected = ellipsoid.cartographic_to_cartesian(&Cartographic::from_radians(lon, lat, height));
    assert_vec3_eq(actual, expected, "fromRadians matches ellipsoid");
}

#[test]
fn from_radians_north_pole() {
    let ellipsoid = Ellipsoid::WGS84;
    let actual = cartesian3_ext::from_radians(0.0, FRAC_PI_2, 0.0, &ellipsoid);
    // 在北极点，位置应为 (0, 0, 短半轴)
    let expected_z = 6356752.314245179; // WGS84 b
    assert!(
        actual.x.abs() < 1e-3 && actual.y.abs() < 1e-3 && (actual.z - expected_z).abs() < 1e-3,
        "fromRadians north pole: expected ~(0, 0, {:.3}), got ({:.3}, {:.3}, {:.3})",
        expected_z, actual.x, actual.y, actual.z
    );
}

// ─── fromDegreesArray / fromRadiansArray ────────────────────────────────────

#[test]
fn from_degrees_array_basic() {
    let ellipsoid = Ellipsoid::WGS84;
    let coords = [-115.0, 37.0, -107.0, 33.0];
    let result = cartesian3_ext::from_degrees_array(&coords, &ellipsoid);

    assert_eq!(result.len(), 2);
    let expected0 = cartesian3_ext::from_degrees(-115.0, 37.0, 0.0, &ellipsoid);
    let expected1 = cartesian3_ext::from_degrees(-107.0, 33.0, 0.0, &ellipsoid);
    assert_vec3_eq(result[0], expected0, "fromDegreesArray[0]");
    assert_vec3_eq(result[1], expected1, "fromDegreesArray[1]");
}

#[test]
#[should_panic(expected = "multiple of 2")]
fn from_degrees_array_throws_odd_length() {
    let ellipsoid = Ellipsoid::WGS84;
    cartesian3_ext::from_degrees_array(&[1.0, 3.0, 5.0], &ellipsoid);
}

#[test]
fn from_radians_array_basic() {
    let ellipsoid = Ellipsoid::WGS84;
    let coords = [-2.007, 0.645, -1.867, 0.575];
    let result = cartesian3_ext::from_radians_array(&coords, &ellipsoid);

    assert_eq!(result.len(), 2);
    let expected0 = cartesian3_ext::from_radians(-2.007, 0.645, 0.0, &ellipsoid);
    let expected1 = cartesian3_ext::from_radians(-1.867, 0.575, 0.0, &ellipsoid);
    assert_vec3_eq(result[0], expected0, "fromRadiansArray[0]");
    assert_vec3_eq(result[1], expected1, "fromRadiansArray[1]");
}

// ─── fromDegreesArrayHeights / fromRadiansArrayHeights ──────────────────────

#[test]
fn from_degrees_array_heights_basic() {
    let ellipsoid = Ellipsoid::WGS84;
    let coords = [-115.0, 37.0, 100000.0, -107.0, 33.0, 150000.0];
    let result = cartesian3_ext::from_degrees_array_heights(&coords, &ellipsoid);

    assert_eq!(result.len(), 2);
    let expected0 = cartesian3_ext::from_degrees(-115.0, 37.0, 100000.0, &ellipsoid);
    let expected1 = cartesian3_ext::from_degrees(-107.0, 33.0, 150000.0, &ellipsoid);
    assert_vec3_eq(result[0], expected0, "fromDegreesArrayHeights[0]");
    assert_vec3_eq(result[1], expected1, "fromDegreesArrayHeights[1]");
}

#[test]
#[should_panic(expected = "multiple of 3")]
fn from_degrees_array_heights_throws_bad_length() {
    let ellipsoid = Ellipsoid::WGS84;
    cartesian3_ext::from_degrees_array_heights(&[1.0, 2.0], &ellipsoid);
}

#[test]
fn from_radians_array_heights_basic() {
    let ellipsoid = Ellipsoid::WGS84;
    let coords = [-2.007, 0.645, 100000.0, -1.867, 0.575, 150000.0];
    let result = cartesian3_ext::from_radians_array_heights(&coords, &ellipsoid);

    assert_eq!(result.len(), 2);
    let expected0 = cartesian3_ext::from_radians(-2.007, 0.645, 100000.0, &ellipsoid);
    let expected1 = cartesian3_ext::from_radians(-1.867, 0.575, 150000.0, &ellipsoid);
    assert_vec3_eq(result[0], expected0, "fromRadiansArrayHeights[0]");
    assert_vec3_eq(result[1], expected1, "fromRadiansArrayHeights[1]");
}

#[test]
#[should_panic(expected = "multiple of 3")]
fn from_radians_array_heights_throws_bad_length() {
    let ellipsoid = Ellipsoid::WGS84;
    cartesian3_ext::from_radians_array_heights(&[1.0, 2.0, 3.0, 4.0], &ellipsoid);
}

// ─── to_spherical roundtrip ─────────────────────────────────────────────────

#[test]
fn to_spherical_roundtrip() {
    let v = DVec3::new(3.0, 4.0, 5.0);
    let spherical = cartesian3_ext::to_spherical(v);
    let back = cartesian3_ext::from_spherical(&spherical);
    assert_vec3_eq(back, v, "toSpherical roundtrip");
}
