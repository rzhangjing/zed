//! Core/TransformsSpec → Rust 集成测试（对齐实现）。
//!
//! 同时保留 `HeadingPitchRollSpec`、
//! `HeadingPitchRangeSpec` 和 `TranslationRotationScaleSpec` 的现有移植版本。
//!
//! ## 平台适配（均有文档说明，而非静默放宽）
//!
//! - **result-parameter 变体**：CesiumJS 的 `f(.., result)` 重载会修改调用方
//!   传入的 `result` 并将其返回（`expect(result).toBe(returnedResult)`）。
//!   Rust 使用拥有所有权的返回值，因此每个 "works with a result parameter" 用例
//!   都被合并进对应的 "without a result parameter" 版本（数值断言完全相同）。
//! - **"throws without/with no <arg>"** 用例：这些断言 CesiumJS 运行时的
//!   `undefined`/`null` 参数校验（`toThrowDeveloperError`）。Rust 的类型系统
//!   使相应的误用成为编译错误，因此这些用例没有运行时对应版本，予以省略。
//! - **`localFrameToFixedFrameGenerator` 非法轴名**用例（`undefined`、
//!   `"northe"`）：CesiumJS 在运行时校验字符串轴名。Rust 的
//!   `LocalFrameAxis` 枚举使非法名称无法表示（编译期安全），因此仅移植
//!   相同/相反轴的 panic 用例。
//! - **对 `Cartesian4` 平移使用 `toEqualEpsilon`**：在
//!   `ellipsoidTo2DModelMatrix` 中，规范用 `new Cartesian4()`（w = 0）构造
//!   `expectedTranslation`，而 `getTranslation` 得到 w = 1。我们仅比较
//!   xyz 分量，这正是所要验证的语义平移。
//!
//! ## 此处未移植（已跟踪，并非遗漏）
//!
//! - `computeTemeToPseudoFixedMatrix`（3 个用例）：需要 `JulianDate` + 闰秒
//!   数据——推迟到 time 模块任务（t9）。
//! - `computeIcrfToMoonFixedMatrix` / `computeIcrfToFixedMatrix` / `Iau2006XysData`
//!   / 地球定向参数用例（约 15 个）：**C 类**——需要外部
//!   EOP/XYS 数据文件以及完整的 IAU 2006/2000A 处理管线。
//! - `pointToGLWindowCoordinates` / `pointToWindowCoordinates`（8 个用例）：需要
//!   `Matrix4.computePerspectiveFieldOfView` / `computeViewportTransformation` /
//!   `fromCamera` 辅助函数——推迟到这些 Matrix4 辅助函数被移植之后。

use cesium_geospatial::transforms::{
    basis_to_2d, east_north_up_to_fixed_frame, ellipsoid_to_2d_model_matrix,
    fixed_frame_to_heading_pitch_roll, heading_pitch_roll_quaternion,
    heading_pitch_roll_quaternion_with_local_frame, heading_pitch_roll_to_fixed_frame,
    heading_pitch_roll_to_fixed_frame_with_local_frame, local_frame_to_fixed_frame,
    north_east_down_to_fixed_frame, north_up_east_to_fixed_frame, north_west_up_to_fixed_frame,
    rotation_matrix_from_position_velocity, HeadingPitchRange, HeadingPitchRoll, LocalFrameAxis,
    TranslationRotationScale,
};
use cesium_geospatial::{Cartographic, Ellipsoid, GeographicProjection, MapProjection};
use cesium_specs::{
    assert_approx, assert_mat3_epsilon, assert_vec3_epsilon, assert_vec4_epsilon, epsilon,
    to_radians,
};
use glam::{DMat3, DMat4, DQuat, DVec3, DVec4};
use std::f64::consts::PI;

use LocalFrameAxis::*;

// === 参考 CesiumJS Matrix4 静态方法的本地辅助函数 ===

/// 对应 `Matrix4.inverseTransformation`（`[R^T | -R^T * t]`），供
/// basisTo2D / ellipsoidTo2DModelMatrix 规范使用。
fn inverse_transformation(matrix: &DMat4) -> DMat4 {
    let rotation = DMat3::from_cols(
        matrix.x_axis.truncate(),
        matrix.y_axis.truncate(),
        matrix.z_axis.truncate(),
    );
    let rotation_t = rotation.transpose();
    let new_translation = -(rotation_t * matrix.w_axis.truncate());
    DMat4::from_cols(
        rotation_t.x_axis.extend(0.0),
        rotation_t.y_axis.extend(0.0),
        rotation_t.z_axis.extend(0.0),
        new_translation.extend(1.0),
    )
}

/// 将本地坐标系轴名映射到经典 East-North-Up 矩阵中对应的（可能取反的）列，
/// 对应 "normal use of localFrameToFixedFrameGenerator" 规范中的轴名分发。
fn enu_column(enu: &DMat4, axis: LocalFrameAxis) -> DVec4 {
    match axis {
        East => enu.x_axis,
        West => -enu.x_axis,
        North => enu.y_axis,
        South => -enu.y_axis,
        Up => enu.z_axis,
        Down => -enu.z_axis,
    }
}

// === HeadingPitchRoll (HeadingPitchRollSpec) ===

#[test]
fn test_hpr_new() {
    let hpr = HeadingPitchRoll::new(0.1, 0.2, 0.3);
    assert_approx!(hpr.heading, 0.1, epsilon::EPSILON15);
    assert_approx!(hpr.pitch, 0.2, epsilon::EPSILON15);
    assert_approx!(hpr.roll, 0.3, epsilon::EPSILON15);
}

#[test]
fn test_hpr_from_degrees() {
    let hpr = HeadingPitchRoll::from_degrees(90.0, 45.0, 0.0);
    assert_approx!(hpr.heading, PI / 2.0, epsilon::EPSILON10);
    assert_approx!(hpr.pitch, PI / 4.0, epsilon::EPSILON10);
    assert_approx!(hpr.roll, 0.0, epsilon::EPSILON15);
}

#[test]
fn test_hpr_to_quaternion_identity() {
    let hpr = HeadingPitchRoll::new(0.0, 0.0, 0.0);
    let q = hpr.to_quaternion();
    assert_approx!(q.w, 1.0, epsilon::EPSILON10);
    assert_approx!(q.x, 0.0, epsilon::EPSILON10);
    assert_approx!(q.y, 0.0, epsilon::EPSILON10);
    assert_approx!(q.z, 0.0, epsilon::EPSILON10);
}

#[test]
fn test_hpr_to_quaternion_heading_90() {
    let hpr = HeadingPitchRoll::new(PI / 2.0, 0.0, 0.0);
    let q = hpr.to_quaternion();
    // CesiumJS 约定：heading 绕 -Z 旋转，故 z = -sin(PI/4)。
    assert_approx!(q.z, -(PI / 4.0).sin(), epsilon::EPSILON10);
    assert_approx!(q.w, (PI / 4.0).cos(), epsilon::EPSILON10);
}

#[test]
fn test_hpr_to_quaternion_pitch_90() {
    let hpr = HeadingPitchRoll::new(0.0, PI / 2.0, 0.0);
    let q = hpr.to_quaternion();
    // CesiumJS 约定：pitch 绕 -Y 旋转，故 y = -sin(PI/4)。
    assert_approx!(q.y, -(PI / 4.0).sin(), epsilon::EPSILON10);
    assert_approx!(q.w, (PI / 4.0).cos(), epsilon::EPSILON10);
}

// === HeadingPitchRange (HeadingPitchRangeSpec) ===

#[test]
fn test_hpr_range_new() {
    let hpr_range = HeadingPitchRange::new(0.5, -0.3, 1000.0);
    assert_approx!(hpr_range.heading, 0.5, epsilon::EPSILON15);
    assert_approx!(hpr_range.pitch, -0.3, epsilon::EPSILON15);
    assert_approx!(hpr_range.range, 1000.0, epsilon::EPSILON15);
}

// === TranslationRotationScale (TranslationRotationScaleSpec) ===

#[test]
fn test_trs_new() {
    let t = DVec3::new(1.0, 2.0, 3.0);
    let r = DQuat::IDENTITY;
    let s = DVec3::new(2.0, 2.0, 2.0);
    let trs = TranslationRotationScale::new(t, r, s);
    assert_vec3_epsilon!(trs.translation, t, epsilon::EPSILON15);
    assert_vec3_epsilon!(trs.scale, s, epsilon::EPSILON15);
}

#[test]
fn test_trs_to_matrix4_identity() {
    let trs = TranslationRotationScale::new(DVec3::ZERO, DQuat::IDENTITY, DVec3::ONE);
    let mat = trs.to_matrix4();
    assert!(mat.abs_diff_eq(DMat4::IDENTITY, 1e-10));
}

#[test]
fn test_trs_to_matrix4_translation() {
    let trs = TranslationRotationScale::new(
        DVec3::new(5.0, 10.0, 15.0),
        DQuat::IDENTITY,
        DVec3::ONE,
    );
    let mat = trs.to_matrix4();
    assert_vec3_epsilon!(
        mat.w_axis.truncate(),
        DVec3::new(5.0, 10.0, 15.0),
        epsilon::EPSILON10
    );
}

#[test]
fn test_trs_to_matrix4_scale() {
    let trs = TranslationRotationScale::new(
        DVec3::ZERO,
        DQuat::IDENTITY,
        DVec3::new(2.0, 3.0, 4.0),
    );
    let mat = trs.to_matrix4();
    assert_approx!(mat.x_axis.x, 2.0, epsilon::EPSILON10);
    assert_approx!(mat.y_axis.y, 3.0, epsilon::EPSILON10);
    assert_approx!(mat.z_axis.z, 4.0, epsilon::EPSILON10);
}

// === eastNorthUpToFixedFrame ===

#[test]
fn test_enu_works_without_a_result_parameter() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let expected_translation = DVec4::new(origin.x, origin.y, origin.z, 1.0);

    let m = east_north_up_to_fixed_frame(origin, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.y_axis, DVec4::Z, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.z_axis, DVec4::X, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_enu_works_at_the_north_pole() {
    let north_pole = DVec3::new(0.0, 0.0, 1.0);
    let expected_translation = DVec4::new(0.0, 0.0, 1.0, 1.0);

    let m = east_north_up_to_fixed_frame(north_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.y_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.z_axis, DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_enu_works_at_the_south_pole() {
    let south_pole = DVec3::new(0.0, 0.0, -1.0);
    let expected_translation = DVec4::new(0.0, 0.0, -1.0, 1.0);

    let m = east_north_up_to_fixed_frame(south_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.y_axis, DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.z_axis, -DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_enu_works_at_the_origin() {
    let expected_translation = DVec4::new(0.0, 0.0, 0.0, 1.0);

    let m = east_north_up_to_fixed_frame(DVec3::ZERO, &Ellipsoid::WGS84);
    assert_vec4_epsilon!(m.x_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.y_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.z_axis, DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

// === northEastDownToFixedFrame ===

#[test]
fn test_ned_works_without_a_result_parameter() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let expected_translation = DVec4::new(origin.x, origin.y, origin.z, 1.0);

    let m = north_east_down_to_fixed_frame(origin, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::Z, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.z_axis, -DVec4::X, epsilon::EPSILON15); // 下
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_ned_works_at_the_north_pole() {
    let north_pole = DVec3::new(0.0, 0.0, 1.0);
    let expected_translation = DVec4::new(0.0, 0.0, 1.0, 1.0);

    let m = north_east_down_to_fixed_frame(north_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.z_axis, -DVec4::Z, epsilon::EPSILON15); // 下
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_ned_works_at_the_south_pole() {
    let south_pole = DVec3::new(0.0, 0.0, -1.0);
    let expected_translation = DVec4::new(0.0, 0.0, -1.0, 1.0);

    let m = north_east_down_to_fixed_frame(south_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.z_axis, DVec4::Z, epsilon::EPSILON15); // 下
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_ned_works_at_the_origin() {
    let expected_translation = DVec4::new(0.0, 0.0, 0.0, 1.0);

    let m = north_east_down_to_fixed_frame(DVec3::ZERO, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.z_axis, -DVec4::Z, epsilon::EPSILON15); // 下
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

// === northUpEastToFixedFrame ===

#[test]
fn test_nue_works_without_a_result_parameter() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let expected_translation = DVec4::new(origin.x, origin.y, origin.z, 1.0);

    let m = north_up_east_to_fixed_frame(origin, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::Z, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, DVec4::X, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.z_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_nue_works_at_the_north_pole() {
    let north_pole = DVec3::new(0.0, 0.0, 1.0);
    let expected_translation = DVec4::new(0.0, 0.0, 1.0, 1.0);

    let m = north_up_east_to_fixed_frame(north_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.z_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_nue_works_at_the_south_pole() {
    let south_pole = DVec3::new(0.0, 0.0, -1.0);
    let expected_translation = DVec4::new(0.0, 0.0, -1.0, 1.0);

    let m = north_up_east_to_fixed_frame(south_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, -DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.z_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_nue_works_at_the_origin() {
    let expected_translation = DVec4::new(0.0, 0.0, 0.0, 1.0);

    let m = north_up_east_to_fixed_frame(DVec3::ZERO, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.z_axis, DVec4::Y, epsilon::EPSILON15); // 东
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

// === northWestUpToFixedFrame ===

#[test]
fn test_nwu_works_without_a_result_parameter() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let expected_translation = DVec4::new(origin.x, origin.y, origin.z, 1.0);

    let m = north_west_up_to_fixed_frame(origin, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::Z, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, -DVec4::Y, epsilon::EPSILON15); // 西
    assert_vec4_epsilon!(m.z_axis, DVec4::X, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_nwu_works_at_the_north_pole() {
    let north_pole = DVec3::new(0.0, 0.0, 1.0);
    let expected_translation = DVec4::new(0.0, 0.0, 1.0, 1.0);

    let m = north_west_up_to_fixed_frame(north_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, -DVec4::Y, epsilon::EPSILON15); // 西
    assert_vec4_epsilon!(m.z_axis, DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_nwu_works_at_the_south_pole() {
    let south_pole = DVec3::new(0.0, 0.0, -1.0);
    let expected_translation = DVec4::new(0.0, 0.0, -1.0, 1.0);

    let m = north_west_up_to_fixed_frame(south_pole, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, -DVec4::Y, epsilon::EPSILON15); // 西
    assert_vec4_epsilon!(m.z_axis, -DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

#[test]
fn test_nwu_works_at_the_origin() {
    let expected_translation = DVec4::new(0.0, 0.0, 0.0, 1.0);

    let m = north_west_up_to_fixed_frame(DVec3::ZERO, &Ellipsoid::UNIT_SPHERE);
    assert_vec4_epsilon!(m.x_axis, -DVec4::X, epsilon::EPSILON15); // 北
    assert_vec4_epsilon!(m.y_axis, -DVec4::Y, epsilon::EPSILON15); // 西
    assert_vec4_epsilon!(m.z_axis, DVec4::Z, epsilon::EPSILON15); // 上
    assert_vec4_epsilon!(m.w_axis, expected_translation, epsilon::EPSILON15); // 平移
}

// === localFrameToFixedFrameGenerator ===

#[test]
fn test_local_frame_to_fixed_frame_generator_normal_use() {
    let cartesian_tab = [
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(10.0, 20.0, 30.0),
        DVec3::new(-10.0, -20.0, -30.0),
        DVec3::new(-25.0, 60.0, -1.0),
        DVec3::new(9.0, 0.0, -7.0),
    ];

    //（firstAxis、secondAxis、预期的列顺序）
    let converter_tab: [(LocalFrameAxis, LocalFrameAxis, [LocalFrameAxis; 3]); 20] = [
        (North, East, [North, East, Down]),
        (North, West, [North, West, Up]),
        (North, Up, [North, Up, East]),
        (North, Down, [North, Down, West]),
        (South, East, [South, East, Up]),
        (South, West, [South, West, Down]),
        (South, Up, [South, Up, West]),
        (South, Down, [South, Down, East]),
        (East, North, [East, North, Up]),
        (East, South, [East, South, Down]),
        (East, Up, [East, Up, South]),
        (East, Down, [East, Down, North]),
        (West, North, [West, North, Down]),
        (West, South, [West, South, Up]),
        (West, Up, [West, Up, North]),
        (West, Down, [West, Down, South]),
        (Up, North, [Up, North, West]),
        (Up, South, [Up, South, East]),
        (Up, East, [Up, East, North]),
        (Up, West, [Up, West, South]),
    ];

    for &position in &cartesian_tab {
        let enu = east_north_up_to_fixed_frame(position, &Ellipsoid::UNIT_SPHERE);
        for &(first, second, order) in &converter_tab {
            let converter_matrix =
                local_frame_to_fixed_frame(first, second, position, &Ellipsoid::UNIT_SPHERE);

            // 检查平移
            assert_vec4_epsilon!(converter_matrix.w_axis, enu.w_axis, epsilon::EPSILON15);

            // 检查各轴
            let converter_cols = [
                converter_matrix.x_axis,
                converter_matrix.y_axis,
                converter_matrix.z_axis,
            ];
            for (j, &axis_name) in order.iter().enumerate() {
                let expected = enu_column(&enu, axis_name);
                assert_vec4_epsilon!(converter_cols[j], expected, epsilon::EPSILON15);
            }
        }
    }
}

#[test]
fn test_local_frame_to_fixed_frame_generator_abnormal_use() {
    // 相同或相反的轴对必须 panic（CesiumJS DeveloperError）。
    // `undefined` / 非法名称用例在 Rust 中是编译期安全的，
    // 因此予以省略（见模块文档）。
    let bad_pairs = [
        (North, North),
        (North, South),
        (South, North),
        (South, South),
        (Up, Up),
        (Up, Down),
        (Down, Up),
        (Down, Down),
        (East, East),
        (East, West),
        (West, East),
        (West, West),
    ];

    let origin = DVec3::new(1.0, 0.0, 0.0);
    let ellipsoid = Ellipsoid::UNIT_SPHERE;
    for &(first, second) in &bad_pairs {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            local_frame_to_fixed_frame(first, second, origin, &ellipsoid)
        }));
        assert!(
            result.is_err(),
            "expected panic for axis pair ({:?}, {:?})",
            first,
            second
        );
    }
}

// === headingPitchRollToFixedFrame ===

#[test]
fn test_hpr_to_fixed_frame_default() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let hpr = HeadingPitchRoll::new(to_radians(20.0), to_radians(30.0), to_radians(40.0));

    let expected_rotation = DMat3::from_quat(hpr.to_quaternion());
    let expected_x = expected_rotation.x_axis;
    let expected_y = expected_rotation.y_axis;
    let expected_z = expected_rotation.z_axis;
    // 在单位球上的 (1,0,0) 处，ENU 坐标系是一个循环置换，因此
    // 每个固定帧列都是将 HPR 列按 (x, y, z) -> (z, x, y) 映射所得。
    let expected_x = DVec3::new(expected_x.z, expected_x.x, expected_x.y);
    let expected_y = DVec3::new(expected_y.z, expected_y.x, expected_y.y);
    let expected_z = DVec3::new(expected_z.z, expected_z.x, expected_z.y);

    let m = heading_pitch_roll_to_fixed_frame(&hpr, origin, &Ellipsoid::UNIT_SPHERE);
    let actual_x = m.x_axis.truncate();
    let actual_y = m.y_axis.truncate();
    let actual_z = m.z_axis.truncate();
    let actual_translation = m.w_axis.truncate();

    assert_vec3_epsilon!(actual_x, expected_x, epsilon::EPSILON15);
    assert_vec3_epsilon!(actual_y, expected_y, epsilon::EPSILON15);
    assert_vec3_epsilon!(actual_z, expected_z, epsilon::EPSILON15);
    assert_vec3_epsilon!(actual_translation, origin, epsilon::EPSILON15);
}

#[test]
fn test_hpr_to_fixed_frame_custom_frame() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let hpr = HeadingPitchRoll::new(to_radians(20.0), to_radians(30.0), to_radians(40.0));

    let expected_rotation = DMat3::from_quat(hpr.to_quaternion());
    let expected_east = expected_rotation.x_axis;
    let expected_north = expected_rotation.y_axis;
    let expected_up = expected_rotation.z_axis;
    let expected_east = DVec3::new(expected_east.z, expected_east.x, expected_east.y);
    let expected_north = DVec3::new(expected_north.z, expected_north.x, expected_north.y);
    let expected_up = DVec3::new(expected_up.z, expected_up.x, expected_up.y);

    // 自定义本地坐标系（"west"、"south"）——即 up/north/east 顺序。
    let m = heading_pitch_roll_to_fixed_frame_with_local_frame(
        &hpr,
        origin,
        &Ellipsoid::UNIT_SPHERE,
        West,
        South,
    );
    let mut actual_east = m.x_axis.truncate();
    actual_east.y = -actual_east.y;
    actual_east.z = -actual_east.z;
    let mut actual_north = m.y_axis.truncate();
    actual_north.y = -actual_north.y;
    actual_north.z = -actual_north.z;
    let mut actual_up = m.z_axis.truncate();
    actual_up.y = -actual_up.y;
    actual_up.z = -actual_up.z;
    let actual_translation = m.w_axis.truncate();

    assert_vec3_epsilon!(actual_east, expected_east, epsilon::EPSILON15);
    assert_vec3_epsilon!(actual_north, expected_north, epsilon::EPSILON15);
    assert_vec3_epsilon!(actual_up, expected_up, epsilon::EPSILON15);
    assert_vec3_epsilon!(actual_translation, origin, epsilon::EPSILON15);
}

// === headingPitchRollQuaternion ===

#[test]
fn test_hpr_quaternion_default() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let hpr = HeadingPitchRoll::new(to_radians(20.0), to_radians(30.0), to_radians(40.0));

    let transform = heading_pitch_roll_to_fixed_frame(&hpr, origin, &Ellipsoid::UNIT_SPHERE);
    let expected = DMat3::from_cols(
        transform.x_axis.truncate(),
        transform.y_axis.truncate(),
        transform.z_axis.truncate(),
    );

    let quaternion = heading_pitch_roll_quaternion(&hpr, origin, &Ellipsoid::UNIT_SPHERE);
    let actual = DMat3::from_quat(quaternion);
    assert_mat3_epsilon!(actual, expected, epsilon::EPSILON11);
}

#[test]
fn test_hpr_quaternion_custom_frame() {
    let origin = DVec3::new(1.0, 0.0, 0.0);
    let hpr = HeadingPitchRoll::new(to_radians(20.0), to_radians(30.0), to_radians(40.0));

    let transform = heading_pitch_roll_to_fixed_frame_with_local_frame(
        &hpr,
        origin,
        &Ellipsoid::UNIT_SPHERE,
        West,
        South,
    );
    let expected = DMat3::from_cols(
        transform.x_axis.truncate(),
        transform.y_axis.truncate(),
        transform.z_axis.truncate(),
    );

    let quaternion = heading_pitch_roll_quaternion_with_local_frame(
        &hpr,
        origin,
        &Ellipsoid::UNIT_SPHERE,
        West,
        South,
    );
    let actual = DMat3::from_quat(quaternion);
    assert_mat3_epsilon!(actual, expected, epsilon::EPSILON11);
}

// === rotationMatrixFromPositionVelocity ===

#[test]
fn test_rotation_matrix_from_position_velocity() {
    // CesiumJS 的 `new Matrix3(...)` 字面量是行主序；此处已转换为
    // glam 的列主序列。
    let m = rotation_matrix_from_position_velocity(DVec3::X, DVec3::Y, &Ellipsoid::WGS84);
    let expected = DMat3::from_cols(
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
    );
    assert_mat3_epsilon!(m, expected, epsilon::EPSILON14);

    let m = rotation_matrix_from_position_velocity(DVec3::X, DVec3::Z, &Ellipsoid::WGS84);
    let expected = DMat3::from_cols(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, -1.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
    );
    assert_mat3_epsilon!(m, expected, epsilon::EPSILON14);

    let m = rotation_matrix_from_position_velocity(DVec3::Y, DVec3::Z, &Ellipsoid::WGS84);
    let expected = DMat3::from_cols(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
    );
    assert_mat3_epsilon!(m, expected, epsilon::EPSILON14);
}

// === basisTo2D ===

#[test]
fn test_basis_to_2d_projects_translation() {
    let ellipsoid = Ellipsoid::WGS84;
    let projection = GeographicProjection::new(ellipsoid);
    let origin =
        ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 100.0));
    let hpr = HeadingPitchRoll::new(to_radians(90.0), to_radians(45.0), 0.0);

    let model_matrix = heading_pitch_roll_to_fixed_frame(&hpr, origin, &ellipsoid);
    let model_matrix_2d = basis_to_2d(&projection, &model_matrix);

    let translation_2d = model_matrix_2d.w_axis.truncate();

    let carto = ellipsoid.cartesian_to_cartographic(origin).unwrap();
    let expected = projection.project(&carto);
    let expected = DVec3::new(expected.z, expected.x, expected.y);

    assert_vec3_epsilon!(translation_2d, expected, epsilon::EPSILON15);
}

#[test]
fn test_basis_to_2d_transforms_rotation() {
    let ellipsoid = Ellipsoid::WGS84;
    let projection = GeographicProjection::new(ellipsoid);
    let origin =
        ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 100.0));
    let hpr = HeadingPitchRoll::new(to_radians(90.0), to_radians(45.0), 0.0);

    let model_matrix = heading_pitch_roll_to_fixed_frame(&hpr, origin, &ellipsoid);
    let model_matrix_2d = basis_to_2d(&projection, &model_matrix);

    let rotation_2d = DMat3::from_cols(
        model_matrix_2d.x_axis.truncate(),
        model_matrix_2d.y_axis.truncate(),
        model_matrix_2d.z_axis.truncate(),
    );

    let enu = east_north_up_to_fixed_frame(origin, &ellipsoid);
    let enu_inverse = inverse_transformation(&enu);

    let hpr_plus_translate = enu_inverse * model_matrix;
    let hpr2 = DMat3::from_cols(
        hpr_plus_translate.x_axis.truncate(),
        hpr_plus_translate.y_axis.truncate(),
        hpr_plus_translate.z_axis.truncate(),
    );

    // 预期的各行 = (hpr2.row2, hpr2.row0, hpr2.row1)；等价地，每个
    // 预期列都是将对应的 hpr2 列按 (x,y,z)->(z,x,y) 映射所得。
    let expected = DMat3::from_cols(
        DVec3::new(hpr2.x_axis.z, hpr2.x_axis.x, hpr2.x_axis.y),
        DVec3::new(hpr2.y_axis.z, hpr2.y_axis.x, hpr2.y_axis.y),
        DVec3::new(hpr2.z_axis.z, hpr2.z_axis.x, hpr2.z_axis.y),
    );

    assert_mat3_epsilon!(rotation_2d, expected, epsilon::EPSILON3);
}

// === ellipsoidTo2DModelMatrix ===

#[test]
fn test_ellipsoid_to_2d_model_matrix() {
    let ellipsoid = Ellipsoid::WGS84;
    let projection = GeographicProjection::new(ellipsoid);
    let origin =
        ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 100.0));

    let actual = ellipsoid_to_2d_model_matrix(&projection, origin);
    let expected = DMat4::from_translation(origin);
    let expected = basis_to_2d(&projection, &expected);

    let actual_rotation = DMat3::from_cols(
        actual.x_axis.truncate(),
        actual.y_axis.truncate(),
        actual.z_axis.truncate(),
    );
    let expected_rotation = DMat3::from_cols(
        expected.x_axis.truncate(),
        expected.y_axis.truncate(),
        expected.z_axis.truncate(),
    );
    assert_mat3_epsilon!(actual_rotation, expected_rotation, epsilon::EPSILON14);

    let from_enu = east_north_up_to_fixed_frame(origin, &ellipsoid);
    let to_enu = inverse_transformation(&from_enu);
    let to_enu_translation = to_enu.w_axis;
    let projected_translation = expected.w_axis;

    let expected_translation = DVec3::new(
        projected_translation.x + to_enu_translation.z,
        projected_translation.y + to_enu_translation.x,
        projected_translation.z + to_enu_translation.y,
    );
    let actual_translation = actual.w_axis.truncate();

    // 仅比较 xyz：规范的 `expectedTranslation` 是默认的
    // `Cartesian4`（w = 0），而 `getTranslation` 得到 w = 1（见文档）。
    assert_vec3_epsilon!(actual_translation, expected_translation, epsilon::EPSILON14);
}

// === fixedFrameToHeadingPitchRoll ===

#[test]
fn test_fixed_frame_to_heading_pitch_roll() {
    let expected = HeadingPitchRoll::new(0.5, 0.6, 0.7);

    let origin = Ellipsoid::WGS84.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
    let transform = east_north_up_to_fixed_frame(origin, &Ellipsoid::WGS84);
    let transform2 = TranslationRotationScale::new(DVec3::ZERO, expected.to_quaternion(), DVec3::ONE)
        .to_matrix4();
    let transform = transform * transform2;

    let actual = fixed_frame_to_heading_pitch_roll(&transform, &Ellipsoid::WGS84);
    assert_approx!(actual.heading, expected.heading, epsilon::EPSILON10);
    assert_approx!(actual.pitch, expected.pitch, epsilon::EPSILON10);
    assert_approx!(actual.roll, expected.roll, epsilon::EPSILON10);
}
