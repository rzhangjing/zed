//! 参考自 CesiumJS TransformsSpec 的扩展变换测试。
//!
//! 覆盖：rotationMatrixFromPositionVelocity, fixedFrameToHeadingPitchRoll,
//! basisTo2D, ellipsoidTo2DModelMatrix，以及额外的参考系测试。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::projection::{GeographicProjection, MapProjection};
use cesium_geospatial::transforms::{
    basis_to_2d, east_north_up_to_fixed_frame, ellipsoid_to_2d_model_matrix,
    fixed_frame_to_heading_pitch_roll, heading_pitch_roll_to_fixed_frame,
    north_east_down_to_fixed_frame, north_up_east_to_fixed_frame,
    north_west_up_to_fixed_frame, rotation_matrix_from_position_velocity, HeadingPitchRoll,
};
use glam::{DMat3, DMat4, DVec3};

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

// ===========================================================================
// rotationMatrixFromPositionVelocity
// ===========================================================================

#[test]
fn rotation_matrix_from_position_velocity_unit_x_y() {
    // CesiumJS: position=UNIT_X, velocity=UNIT_Y
    // 期望 = Matrix3(0, 0, 1, 1, 0, 0, 0, 1, 0)（行主序）
    let matrix = rotation_matrix_from_position_velocity(DVec3::X, DVec3::Y, &wgs84());

    // CesiumJS Matrix3 为行主序：列为 (col0, col1, col2)
    // 行主序 (0,0,1, 1,0,0, 0,1,0) 表示：
    //   col0 = (0, 1, 0), col1 = (0, 0, 1), col2 = (1, 0, 0)
    let expected = DMat3::from_cols(
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
    );

    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (matrix.col(i)[j] - expected.col(i)[j]).abs() < 1e-14,
                "mismatch at ({},{}): {} vs {}",
                i,
                j,
                matrix.col(i)[j],
                expected.col(i)[j]
            );
        }
    }
}

#[test]
fn rotation_matrix_from_position_velocity_unit_x_z() {
    // CesiumJS: position=UNIT_X, velocity=UNIT_Z
    // 期望 = Matrix3(0, 0, 1, 0, -1, 0, 1, 0, 0)（行主序）
    let matrix = rotation_matrix_from_position_velocity(DVec3::X, DVec3::Z, &wgs84());

    // 行主序 (0,0,1, 0,-1,0, 1,0,0)：
    //   col0 = (0, 0, 1), col1 = (0, -1, 0), col2 = (1, 0, 0)
    let expected = DMat3::from_cols(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, -1.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
    );

    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (matrix.col(i)[j] - expected.col(i)[j]).abs() < 1e-14,
                "mismatch at ({},{}): {} vs {}",
                i,
                j,
                matrix.col(i)[j],
                expected.col(i)[j]
            );
        }
    }
}

#[test]
fn rotation_matrix_from_position_velocity_unit_y_z() {
    // CesiumJS: position=UNIT_Y, velocity=UNIT_Z
    // 期望 = Matrix3(0, 1, 0, 0, 0, 1, 1, 0, 0)（行主序）
    let matrix = rotation_matrix_from_position_velocity(DVec3::Y, DVec3::Z, &wgs84());

    // 行主序 (0,1,0, 0,0,1, 1,0,0)：
    //   col0 = (0, 0, 1), col1 = (1, 0, 0), col2 = (0, 1, 0)
    let expected = DMat3::from_cols(
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
    );

    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (matrix.col(i)[j] - expected.col(i)[j]).abs() < 1e-14,
                "mismatch at ({},{}): {} vs {}",
                i,
                j,
                matrix.col(i)[j],
                expected.col(i)[j]
            );
        }
    }
}

#[test]
fn rotation_matrix_columns_are_orthonormal() {
    let pos = wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(45.0, 30.0, 0.0));
    let vel = DVec3::new(0.0, 1.0, 0.5).normalize();
    let matrix = rotation_matrix_from_position_velocity(pos, vel, &wgs84());

    // 每一列都应为单位长度
    for i in 0..3 {
        let col = matrix.col(i);
        assert!(
            (col.length() - 1.0).abs() < 1e-10,
            "column {} should be unit, got {}",
            i,
            col.length()
        );
    }
    // 各列应相互正交
    assert!(matrix.col(0).dot(matrix.col(1)).abs() < 1e-10);
    assert!(matrix.col(0).dot(matrix.col(2)).abs() < 1e-10);
    assert!(matrix.col(1).dot(matrix.col(2)).abs() < 1e-10);
}

// ===========================================================================
// fixedFrameToHeadingPitchRoll
// ===========================================================================

#[test]
fn fixed_frame_to_heading_pitch_roll_roundtrip() {
    // CesiumJS：由 HPR 创建变换，再反向提取出 HPR
    let expected = HeadingPitchRoll::new(0.5, 0.6, 0.7);
    let origin = wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));

    let enu = east_north_up_to_fixed_frame(origin, &wgs84());
    let hpr_rotation = expected.to_quaternion();
    let rotation_mat = DMat4::from_quat(hpr_rotation);
    let transform = enu * rotation_mat;

    let actual = fixed_frame_to_heading_pitch_roll(&transform, &wgs84());

    assert!(
        (actual.heading - expected.heading).abs() < 1e-10,
        "heading: {} vs {}",
        actual.heading,
        expected.heading
    );
    assert!(
        (actual.pitch - expected.pitch).abs() < 1e-10,
        "pitch: {} vs {}",
        actual.pitch,
        expected.pitch
    );
    assert!(
        (actual.roll - expected.roll).abs() < 1e-10,
        "roll: {} vs {}",
        actual.roll,
        expected.roll
    );
}

#[test]
fn fixed_frame_to_heading_pitch_roll_zero_at_identity() {
    // 当以原点为中心的单位变换下，HPR 应为零
    let origin = wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 20.0, 0.0));
    let enu = east_north_up_to_fixed_frame(origin, &wgs84());

    let hpr = fixed_frame_to_heading_pitch_roll(&enu, &wgs84());
    assert!(hpr.heading.abs() < 1e-10, "heading should be 0, got {}", hpr.heading);
    assert!(hpr.pitch.abs() < 1e-10, "pitch should be 0, got {}", hpr.pitch);
    assert!(hpr.roll.abs() < 1e-10, "roll should be 0, got {}", hpr.roll);
}

// ===========================================================================
// basisTo2D
// ===========================================================================

#[test]
fn basis_to_2d_projects_translation() {
    // CesiumJS: "basisTo2D projects translation"
    let ellipsoid = wgs84();
    let projection = GeographicProjection::new(ellipsoid);
    let origin = ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 100.0));

    let hpr = HeadingPitchRoll::new(
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_4,
        0.0,
    );
    let model_matrix = heading_pitch_roll_to_fixed_frame(&hpr, origin, &ellipsoid);
    let model_matrix_2d = basis_to_2d(&projection, &model_matrix);

    // 平移列应为投影位置按 (z, x, y) 重排
    let translation_2d = model_matrix_2d.w_axis.truncate();
    let carto = ellipsoid.cartesian_to_cartographic(origin).unwrap();
    let projected = projection.project(&carto);
    let expected = DVec3::new(projected.z, projected.x, projected.y);

    let diff = (translation_2d - expected).length();
    assert!(
        diff < 1e-6,
        "translation2D should match projected position, diff={}",
        diff
    );
}

#[test]
fn basis_to_2d_rotation_is_orthonormal() {
    let ellipsoid = wgs84();
    let projection = GeographicProjection::new(ellipsoid);
    let origin = ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 100.0));

    let hpr = HeadingPitchRoll::new(
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_4,
        0.0,
    );
    let model_matrix = heading_pitch_roll_to_fixed_frame(&hpr, origin, &ellipsoid);
    let model_matrix_2d = basis_to_2d(&projection, &model_matrix);

    // 3x3 旋转部分应正交归一
    let rot = DMat3::from_cols(
        model_matrix_2d.x_axis.truncate(),
        model_matrix_2d.y_axis.truncate(),
        model_matrix_2d.z_axis.truncate(),
    );

    for i in 0..3 {
        let col = rot.col(i);
        assert!(
            (col.length() - 1.0).abs() < 1e-10,
            "rotation column {} should be unit, got {}",
            i,
            col.length()
        );
    }
    assert!(rot.col(0).dot(rot.col(1)).abs() < 1e-10);
    assert!(rot.col(0).dot(rot.col(2)).abs() < 1e-10);
    assert!(rot.col(1).dot(rot.col(2)).abs() < 1e-10);
}

// ===========================================================================
// ellipsoidTo2DModelMatrix
// ===========================================================================

#[test]
fn ellipsoid_to_2d_model_matrix_rotation_matches_basis_to_2d() {
    // CesiumJS: "ellipsoidTo2DModelMatrix creates a model matrix to transform
    // vertices centered origin to 2D"
    let ellipsoid = wgs84();
    let projection = GeographicProjection::new(ellipsoid);
    let origin = ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 100.0));

    let actual = ellipsoid_to_2d_model_matrix(&projection, origin);

    // 期望：basisTo2D(projection, Matrix4.fromTranslation(origin))
    let translation_mat = DMat4::from_translation(origin);
    let expected = basis_to_2d(&projection, &translation_mat);

    // 旋转部分应一致
    let actual_rot = DMat3::from_cols(
        actual.x_axis.truncate(),
        actual.y_axis.truncate(),
        actual.z_axis.truncate(),
    );
    let expected_rot = DMat3::from_cols(
        expected.x_axis.truncate(),
        expected.y_axis.truncate(),
        expected.z_axis.truncate(),
    );

    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (actual_rot.col(i)[j] - expected_rot.col(i)[j]).abs() < 1e-14,
                "rotation mismatch at ({},{})",
                i,
                j
            );
        }
    }
}

#[test]
fn ellipsoid_to_2d_model_matrix_is_valid_rigid_transform() {
    let ellipsoid = wgs84();
    let projection = GeographicProjection::new(ellipsoid);
    let origin = ellipsoid.cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 50.0, 0.0));

    let result = ellipsoid_to_2d_model_matrix(&projection, origin);

    // 旋转部分应正交归一
    let rot = DMat3::from_cols(
        result.x_axis.truncate(),
        result.y_axis.truncate(),
        result.z_axis.truncate(),
    );
    for i in 0..3 {
        assert!(
            (rot.col(i).length() - 1.0).abs() < 1e-10,
            "column {} not unit: {}",
            i,
            rot.col(i).length()
        );
    }
    assert!(rot.col(0).dot(rot.col(1)).abs() < 1e-10);
    assert!(rot.col(0).dot(rot.col(2)).abs() < 1e-10);
    assert!(rot.col(1).dot(rot.col(2)).abs() < 1e-10);

    // 平移应为有限值
    let t = result.w_axis.truncate();
    assert!(t.x.is_finite() && t.y.is_finite() && t.z.is_finite());
}

// ===========================================================================
// 参考系函数 - 补充覆盖
// ===========================================================================

#[test]
fn north_east_down_frame_at_equator() {
    // 在 (lat=0, lon=0)：NED 参考系
    // North = (0,0,1), East = (0,1,0), Down = (-1,0,0)
    let origin = DVec3::new(6378137.0, 0.0, 0.0);
    let frame = north_east_down_to_fixed_frame(origin, &wgs84());

    let north = frame.x_axis.truncate();
    let east = frame.y_axis.truncate();
    let down = frame.z_axis.truncate();

    assert!((north - DVec3::Z).length() < 1e-10, "North: {:?}", north);
    assert!((east - DVec3::Y).length() < 1e-10, "East: {:?}", east);
    assert!((down - (-DVec3::X)).length() < 1e-10, "Down: {:?}", down);
}

#[test]
fn north_up_east_frame_at_equator() {
    // 在 (lat=0, lon=0)：NUE 参考系
    // North = (0,0,1), Up = (1,0,0), East = (0,1,0)
    let origin = DVec3::new(6378137.0, 0.0, 0.0);
    let frame = north_up_east_to_fixed_frame(origin, &wgs84());

    let north = frame.x_axis.truncate();
    let up = frame.y_axis.truncate();
    let east = frame.z_axis.truncate();

    assert!((north - DVec3::Z).length() < 1e-10, "North: {:?}", north);
    assert!((up - DVec3::X).length() < 1e-10, "Up: {:?}", up);
    assert!((east - DVec3::Y).length() < 1e-10, "East: {:?}", east);
}

#[test]
fn north_west_up_frame_at_equator() {
    // 在 (lat=0, lon=0)：NWU 参考系
    // North = (0,0,1), West = (0,-1,0), Up = (1,0,0)
    let origin = DVec3::new(6378137.0, 0.0, 0.0);
    let frame = north_west_up_to_fixed_frame(origin, &wgs84());

    let north = frame.x_axis.truncate();
    let west = frame.y_axis.truncate();
    let up = frame.z_axis.truncate();

    assert!((north - DVec3::Z).length() < 1e-10, "North: {:?}", north);
    assert!((west - (-DVec3::Y)).length() < 1e-10, "West: {:?}", west);
    assert!((up - DVec3::X).length() < 1e-10, "Up: {:?}", up);
}

#[test]
fn enu_frame_columns_are_orthonormal() {
    let origin = wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(45.0, 60.0, 1000.0));
    let frame = east_north_up_to_fixed_frame(origin, &wgs84());

    let east = frame.x_axis.truncate();
    let north = frame.y_axis.truncate();
    let up = frame.z_axis.truncate();

    // 单位长度
    assert!((east.length() - 1.0).abs() < 1e-10);
    assert!((north.length() - 1.0).abs() < 1e-10);
    assert!((up.length() - 1.0).abs() < 1e-10);

    // 正交
    assert!(east.dot(north).abs() < 1e-10);
    assert!(east.dot(up).abs() < 1e-10);
    assert!(north.dot(up).abs() < 1e-10);

    // 右手系：east × north = up
    let cross = east.cross(north);
    assert!((cross - up).length() < 1e-10, "should be right-handed");
}

#[test]
fn heading_pitch_roll_to_fixed_frame_preserves_origin() {
    let origin = wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 0.0));
    let hpr = HeadingPitchRoll::new(0.3, 0.2, 0.1);
    let frame = heading_pitch_roll_to_fixed_frame(&hpr, origin, &wgs84());

    // 平移列应为原点
    let translation = frame.w_axis.truncate();
    let diff = (translation - origin).length();
    assert!(diff < 1e-6, "translation should be origin, diff={}", diff);
}

#[test]
fn heading_pitch_roll_to_fixed_frame_rotation_orthonormal() {
    let origin = wgs84().cartographic_to_cartesian(&Cartographic::from_degrees(120.0, -30.0, 0.0));
    let hpr = HeadingPitchRoll::new(1.0, 0.5, 0.3);
    let frame = heading_pitch_roll_to_fixed_frame(&hpr, origin, &wgs84());

    let rot = DMat3::from_cols(
        frame.x_axis.truncate(),
        frame.y_axis.truncate(),
        frame.z_axis.truncate(),
    );

    for i in 0..3 {
        assert!(
            (rot.col(i).length() - 1.0).abs() < 1e-10,
            "column {} not unit",
            i
        );
    }
    assert!(rot.col(0).dot(rot.col(1)).abs() < 1e-10);
    assert!(rot.col(0).dot(rot.col(2)).abs() < 1e-10);
    assert!(rot.col(1).dot(rot.col(2)).abs() < 1e-10);
}
