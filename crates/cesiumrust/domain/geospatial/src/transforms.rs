//! 坐标变换 —— ENU 参考系、HeadingPitchRoll、ICRF。

use crate::ellipsoid::Ellipsoid;
use crate::math_utils;
use crate::projection::MapProjection;
use glam::{DMat3, DMat4, DQuat, DVec3};
use serde::{Deserialize, Serialize};

/// 航向、俯仰与翻滚角（弧度制）。
/// 映射到 CesiumJS `HeadingPitchRoll`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeadingPitchRoll {
    /// 航向角（绕局部 Z/up 轴旋转），弧度制。
    pub heading: f64,
    /// 俯仰角（绕局部 Y/right 轴旋转），弧度制。
    pub pitch: f64,
    /// 翻滚角（绕局部 X/forward 轴旋转），弧度制。
    pub roll: f64,
}

impl Default for HeadingPitchRoll {
    /// 默认构造：航向/俯仰/翻滚均为 0。
    fn default() -> Self {
        Self { heading: 0.0, pitch: 0.0, roll: 0.0 }
    }
}

impl HeadingPitchRoll {
    /// 由航向/俯仰/翻滚（弧度）直接构造。
    pub fn new(heading: f64, pitch: f64, roll: f64) -> Self {
        Self { heading, pitch, roll }
    }

    /// 由角度（度）创建。
    pub fn from_degrees(heading: f64, pitch: f64, roll: f64) -> Self {
        Self {
            heading: math_utils::to_radians(heading),
            pitch: math_utils::to_radians(pitch),
            roll: math_utils::to_radians(roll),
        }
    }

    /// 转换为四元数。
    /// 映射到 `Quaternion.fromHeadingPitchRoll`：
    ///   heading(Z, -heading) * (pitch(Y, -pitch) * roll(X, +roll))
    ///
    /// 注意 CesiumJS 的符号约定：转换为轴角旋转时航向与俯仰会被取负
    /// （航向是绕负 Z 轴的旋转，俯仰绕负 Y 轴，翻滚绕
    /// 正 X 轴）。
    pub fn to_quaternion(&self) -> DQuat {
        let roll = DQuat::from_axis_angle(DVec3::X, self.roll);
        let pitch = DQuat::from_axis_angle(DVec3::Y, -self.pitch);
        let heading = DQuat::from_axis_angle(DVec3::Z, -self.heading);
        heading * (pitch * roll)
    }

    /// 由四元数计算航向/俯仰/翻滚。
    /// 映射到 `HeadingPitchRoll.fromQuaternion`
    pub fn from_quaternion(quaternion: DQuat) -> Self {
        let test = 2.0 * (quaternion.w * quaternion.y - quaternion.z * quaternion.x);
        let denominator_roll =
            1.0 - 2.0 * (quaternion.x * quaternion.x + quaternion.y * quaternion.y);
        let numerator_roll = 2.0 * (quaternion.w * quaternion.x + quaternion.y * quaternion.z);
        let denominator_heading =
            1.0 - 2.0 * (quaternion.y * quaternion.y + quaternion.z * quaternion.z);
        let numerator_heading = 2.0 * (quaternion.w * quaternion.z + quaternion.x * quaternion.y);
        Self {
            heading: -numerator_heading.atan2(denominator_heading),
            pitch: -math_utils::clamp(test, -1.0, 1.0).asin(),
            roll: numerator_roll.atan2(denominator_roll),
        }
    }

    /// 以相对/绝对 epsilon 容差进行比较。
    /// 映射到 `HeadingPitchRoll.equalsEpsilon`
    pub fn equals_epsilon(&self, other: &Self, relative_epsilon: f64) -> bool {
        /// 单分量比较：绝对差或相对差任一小于阈值即视为相等。
        fn eq_eps(left: f64, right: f64, rel_eps: f64) -> bool {
            // 先取绝对差，再判断是否落在绝对或相对阈值内。
            let abs_diff = (left - right).abs();
            abs_diff <= rel_eps || abs_diff <= rel_eps * left.abs().max(right.abs())
        }
        eq_eps(self.heading, other.heading, relative_epsilon)
            && eq_eps(self.pitch, other.pitch, relative_epsilon)
            && eq_eps(self.roll, other.roll, relative_epsilon)
    }
}

impl std::fmt::Display for HeadingPitchRoll {
    /// 以 `(heading, pitch, roll)` 弧度三元组格式化输出。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.heading, self.pitch, self.roll)
    }
}

/// 航向、俯仰与距离（range）。
/// 映射到 CesiumJS `HeadingPitchRange`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeadingPitchRange {
    /// 航向角，弧度制。
    pub heading: f64,
    /// 俯仰角，弧度制。
    pub pitch: f64,
    /// 距离（range），单位米。
    pub range: f64,
}

impl Default for HeadingPitchRange {
    /// 默认构造：航向/俯仰/距离均为 0。
    fn default() -> Self {
        Self { heading: 0.0, pitch: 0.0, range: 0.0 }
    }
}

impl HeadingPitchRange {
    /// 由航向/俯仰（弧度）与距离（米）构造。
    pub fn new(heading: f64, pitch: f64, range: f64) -> Self {
        Self { heading, pitch, range }
    }
}

/// 平移、旋转与缩放。
/// 映射到 CesiumJS `TranslationRotationScale`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TranslationRotationScale {
    /// 平移向量。
    pub translation: DVec3,
    /// 旋转四元数。
    pub rotation: DQuat,
    /// 缩放向量。
    pub scale: DVec3,
}

impl Default for TranslationRotationScale {
    /// 默认构造：平移为零、旋转为单位四元数、缩放为 1。
    fn default() -> Self {
        Self {
            translation: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
            scale: DVec3::ONE,
        }
    }
}

impl TranslationRotationScale {
    /// 由平移/旋转/缩放分量直接构造。
    pub fn new(translation: DVec3, rotation: DQuat, scale: DVec3) -> Self {
        Self { translation, rotation, scale }
    }

    /// 转换为 4x4 矩阵。
    /// 映射到 `Matrix4.fromTranslationRotationScale`
    pub fn to_matrix4(&self) -> DMat4 {
        let rotation_matrix = DMat3::from_quat(self.rotation);
        let scaled = DMat3::from_cols(
            rotation_matrix.x_axis * self.scale.x,
            rotation_matrix.y_axis * self.scale.y,
            rotation_matrix.z_axis * self.scale.z,
        );
        DMat4::from_cols(
            scaled.x_axis.extend(0.0),
            scaled.y_axis.extend(0.0),
            scaled.z_axis.extend(0.0),
            self.translation.extend(1.0),
        )
    }
}

/// 用于构建局部参考系的轴标识符。
/// 映射到 CesiumJS `localFrameToFixedFrameGenerator` 所接受的
/// 字符串轴名（"east"、"north"、"up"、"west"、"south"、"down"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalFrameAxis {
    East,
    North,
    Up,
    West,
    South,
    Down,
}

use LocalFrameAxis::*;

/// 补全右手局部参考系的第三轴（`first × second`）。
/// 映射到 CesiumJS `vectorProductLocalFrame`。当两轴相同或相反时返回
/// `None`（CesiumJS 会以 DeveloperError 拒绝这种情况）。
fn third_axis(first: LocalFrameAxis, second: LocalFrameAxis) -> Option<LocalFrameAxis> {
    match (first, second) {
        (Up, South) => Some(East),
        (Up, North) => Some(West),
        (Up, West) => Some(South),
        (Up, East) => Some(North),
        (Down, South) => Some(West),
        (Down, North) => Some(East),
        (Down, West) => Some(North),
        (Down, East) => Some(South),
        (South, Up) => Some(West),
        (South, Down) => Some(East),
        (South, West) => Some(Down),
        (South, East) => Some(Up),
        (North, Up) => Some(East),
        (North, Down) => Some(West),
        (North, West) => Some(Up),
        (North, East) => Some(Down),
        (West, Up) => Some(North),
        (West, Down) => Some(South),
        (West, North) => Some(Down),
        (West, South) => Some(Up),
        (East, Up) => Some(South),
        (East, Down) => Some(North),
        (East, North) => Some(Up),
        (East, South) => Some(Down),
        _ => None,
    }
}

/// 退化情形（椭球中心）下的局部参考系轴向量。
/// 映射到 CesiumJS `degeneratePositionLocalFrame`。
fn degenerate_axis(axis: LocalFrameAxis) -> DVec3 {
    match axis {
        North => DVec3::new(-1.0, 0.0, 0.0),
        East => DVec3::new(0.0, 1.0, 0.0),
        Up => DVec3::new(0.0, 0.0, 1.0),
        South => DVec3::new(1.0, 0.0, 0.0),
        West => DVec3::new(0.0, -1.0, 0.0),
        Down => DVec3::new(0.0, 0.0, -1.0),
    }
}

/// 该轴是否为 east 或 west（在极点处从不翻转符号）。
fn is_east_west(axis: LocalFrameAxis) -> bool {
    matches!(axis, East | West)
}

/// 从六个已计算的方向中，为具名轴选取具体的向量。
fn pick_axis(
    axis: LocalFrameAxis,
    east: DVec3,
    north: DVec3,
    up: DVec3,
    west: DVec3,
    south: DVec3,
    down: DVec3,
) -> DVec3 {
    match axis {
        East => east,
        North => north,
        Up => up,
        West => west,
        South => south,
        Down => down,
    }
}

/// 逐分量的 `Cartesian3.equalsEpsilon`（原版用 `CesiumMath.equalsEpsilon`
/// 比较每个分量，绝对 epsilon 默认取相对 epsilon 的值）。
fn vec3_equals_epsilon(left: DVec3, right: DVec3, epsilon: f64) -> bool {
    math_utils::equals_epsilon(left.x, right.x, epsilon, epsilon)
        && math_utils::equals_epsilon(left.y, right.y, epsilon, epsilon)
        && math_utils::equals_epsilon(left.z, right.z, epsilon, epsilon)
}

/// 在给定原点计算一个局部参考系。
/// 映射到 `Transforms.localFrameToFixedFrameGenerator(firstAxis, secondAxis)`
///
/// `first_axis` 与 `second_axis` 决定哪些大地测量方向映射到矩阵的
/// X 和 Y 列；Z 列是右手的第三轴（`first × second`）。
///
/// 参考系生成器的分支处理：
/// - 在椭球中心：使用退化的局部参考系。
/// - 在极点（x 和 y 都约为 0）：使用退化参考系，每个
///   非 east/west 轴都乘以 `sign(z)`。
/// - 否则：up = 大地表面法线，east = normalize(-origin.y,
///   origin.x, 0)，north = up × east，以及相反的 down/west/south。
pub fn local_frame_to_fixed_frame(
    first_axis: LocalFrameAxis,
    second_axis: LocalFrameAxis,
    origin: DVec3,
    ellipsoid: &Ellipsoid,
) -> DMat4 {
    let third = third_axis(first_axis, second_axis)
        .expect("firstAxis and secondAxis must be east, north, up, west, south or down.");

    let eps = math_utils::EPSILON14;
    let (first, second, third_vec) = if vec3_equals_epsilon(origin, DVec3::ZERO, eps) {
        // 若 x、y、z 均为零，使用退化的局部参考系。
        (
            degenerate_axis(first_axis),
            degenerate_axis(second_axis),
            degenerate_axis(third),
        )
    } else if math_utils::equals_epsilon(origin.x, 0.0, eps, eps)
        && math_utils::equals_epsilon(origin.y, 0.0, eps, eps)
    {
        // 若 x 和 y 为零，假定原点位于极点。
        let sign = math_utils::sign(origin.z);
        let mut first = degenerate_axis(first_axis);
        if !is_east_west(first_axis) {
            first *= sign;
        }
        let mut second = degenerate_axis(second_axis);
        if !is_east_west(second_axis) {
            second *= sign;
        }
        let mut third_vec = degenerate_axis(third);
        if !is_east_west(third) {
            third_vec *= sign;
        }
        (first, second, third_vec)
    } else {
        // 一般位置。
        let up = ellipsoid
            .geodetic_surface_normal(origin)
            .expect("origin must not be at the center of the ellipsoid");
        let east = crate::ellipsoid::normalize_cartesian3(DVec3::new(-origin.y, origin.x, 0.0));
        let north = up.cross(east);
        // down/west/south 分别为 up/east/north 的反向。
        let down = -up;
        let west = -east;
        let south = -north;
        (
            pick_axis(first_axis, east, north, up, west, south, down),
            pick_axis(second_axis, east, north, up, west, south, down),
            pick_axis(third, east, north, up, west, south, down),
        )
    };

    // 将三个局部轴作为前三列（齐次第 4 分量为 0），origin 作为第四列，拼成局部到 fixed 的变换矩阵。
    DMat4::from_cols(
        first.extend(0.0),
        second.extend(0.0),
        third_vec.extend(0.0),
        origin.extend(1.0),
    )
}

/// 在给定原点计算 East-North-Up (ENU) 参考系。
/// 映射到 `Transforms.eastNorthUpToFixedFrame`
pub fn east_north_up_to_fixed_frame(origin: DVec3, ellipsoid: &Ellipsoid) -> DMat4 {
    local_frame_to_fixed_frame(East, North, origin, ellipsoid)
}

/// 在给定原点计算 North-East-Down (NED) 参考系。
/// 映射到 `Transforms.northEastDownToFixedFrame`
pub fn north_east_down_to_fixed_frame(origin: DVec3, ellipsoid: &Ellipsoid) -> DMat4 {
    local_frame_to_fixed_frame(North, East, origin, ellipsoid)
}

/// 在给定原点计算 North-Up-East (NUE) 参考系。
/// 映射到 `Transforms.northUpEastToFixedFrame`
pub fn north_up_east_to_fixed_frame(origin: DVec3, ellipsoid: &Ellipsoid) -> DMat4 {
    local_frame_to_fixed_frame(North, Up, origin, ellipsoid)
}

/// 在给定原点计算 North-West-Up (NWU) 参考系。
/// 映射到 `Transforms.northWestUpToFixedFrame`
pub fn north_west_up_to_fixed_frame(origin: DVec3, ellipsoid: &Ellipsoid) -> DMat4 {
    local_frame_to_fixed_frame(North, West, origin, ellipsoid)
}

/// 在给定原点由航向/俯仰/翻滚计算 4x4 矩阵，使用默认的
/// East-North-Up 局部参考系。
/// 映射到 `Transforms.headingPitchRollToFixedFrame`
pub fn heading_pitch_roll_to_fixed_frame(
    hpr: &HeadingPitchRoll,
    origin: DVec3,
    ellipsoid: &Ellipsoid,
) -> DMat4 {
    heading_pitch_roll_to_fixed_frame_with_local_frame(hpr, origin, ellipsoid, East, North)
}

/// 在给定原点由航向/俯仰/翻滚计算 4x4 矩阵，使用由
/// `first_axis`/`second_axis` 定义的自定义局部参考系。
/// 映射到带自定义 `fixedFrameTransform` 的
/// `Transforms.headingPitchRollToFixedFrame`。
///
/// 算法：先构建局部参考系到 fixed 的矩阵，再乘以
/// 航向/俯仰/翻滚的旋转矩阵（作为刚体变换），即
/// `Matrix4.multiply(fixedFrame, hprMatrix)`。
pub fn heading_pitch_roll_to_fixed_frame_with_local_frame(
    hpr: &HeadingPitchRoll,
    origin: DVec3,
    ellipsoid: &Ellipsoid,
    first_axis: LocalFrameAxis,
    second_axis: LocalFrameAxis,
) -> DMat4 {
    // 航向/俯仰/翻滚仅影响旋转部分，平移分量为零；最后左乘局部参考系矩阵。
    let fixed_frame = local_frame_to_fixed_frame(first_axis, second_axis, origin, ellipsoid);
    let hpr_rotation = DMat3::from_quat(hpr.to_quaternion());
    let hpr_matrix = DMat4::from_cols(
        hpr_rotation.x_axis.extend(0.0),
        hpr_rotation.y_axis.extend(0.0),
        hpr_rotation.z_axis.extend(0.0),
        DVec3::ZERO.extend(1.0),
    );
    fixed_frame * hpr_matrix
}

/// 在给定原点由航向/俯仰/翻滚计算四元数，使用默认的
/// East-North-Up 局部参考系。
/// 映射到 `Transforms.headingPitchRollQuaternion`
pub fn heading_pitch_roll_quaternion(
    hpr: &HeadingPitchRoll,
    origin: DVec3,
    ellipsoid: &Ellipsoid,
) -> DQuat {
    heading_pitch_roll_quaternion_with_local_frame(hpr, origin, ellipsoid, East, North)
}

/// 在给定原点由航向/俯仰/翻滚计算四元数，使用自定义局部参考系。
/// 映射到带自定义 `fixedFrameTransform` 的
/// `Transforms.headingPitchRollQuaternion`。
pub fn heading_pitch_roll_quaternion_with_local_frame(
    hpr: &HeadingPitchRoll,
    origin: DVec3,
    ellipsoid: &Ellipsoid,
    first_axis: LocalFrameAxis,
    second_axis: LocalFrameAxis,
) -> DQuat {
    let transform = heading_pitch_roll_to_fixed_frame_with_local_frame(
        hpr,
        origin,
        ellipsoid,
        first_axis,
        second_axis,
    );
    let rotation = DMat3::from_cols(
        transform.x_axis.truncate(),
        transform.y_axis.truncate(),
        transform.z_axis.truncate(),
    );
    DQuat::from_mat3(&rotation)
}

/// 计算从 ICRF（惯性系）到 fixed 参考系的旋转矩阵。
/// 简化版：使用地球自转角度近似。
/// 映射到 `Transforms.computeIcrfToFixedMatrix`
pub fn compute_icrf_to_fixed_matrix(julian_date_seconds: f64) -> Option<DMat3> {
    // 简化的地球自转：GMST 近似
    // 完整实现应使用 IAU 2006/2000A 岁差-章动
    let days_since_j2000 = julian_date_seconds / 86400.0 - 2451545.0;
    // GMST 近似公式：以 J2000 起算的恒星时角，再归一到 [0, 2π)。
    let gmst = math_utils::zero_to_two_pi(
        math_utils::to_radians(280.46061837 + 360.98564736629 * days_since_j2000),
    );

    let cos_gmst = gmst.cos();
    let sin_gmst = gmst.sin();

    // 绕 Z 轴按 GMST 旋转
    Some(DMat3::from_cols_array(&[
        cos_gmst, -sin_gmst, 0.0,
        sin_gmst, cos_gmst, 0.0,
        0.0, 0.0, 1.0,
    ]))
}

/// 计算从 fixed 参考系到 ICRF（惯性系）的旋转矩阵。
/// 映射到 `Transforms.computeFixedToIcrfMatrix`
pub fn compute_fixed_to_icrf_matrix(julian_date_seconds: f64) -> Option<DMat3> {
    // 旋转矩阵为正交阵，逆变换即为其转置。
    compute_icrf_to_fixed_matrix(julian_date_seconds).map(|m| m.transpose())
}

/// 计算从某一位置看向目标的视图矩阵。
/// 映射到 `Transforms.lookAt`（简化版）
pub fn look_at(eye: DVec3, target: DVec3, up: DVec3) -> DMat4 {
    let z_axis = (eye - target).normalize();
    let x_axis = up.cross(z_axis).normalize();
    let y_axis = z_axis.cross(x_axis);

    DMat4::from_cols(
        x_axis.extend(0.0),
        y_axis.extend(0.0),
        z_axis.extend(0.0),
        eye.extend(1.0),
    )
}

/// 由位置和速度（飞行方向）计算旋转矩阵。
/// 映射到 `Transforms.rotationMatrixFromPositionVelocity`
///
/// 所得矩阵的各列为 `[velocity, right, up]`，
/// 即 `result[0..2]=velocity, result[3..5]=right,
/// result[6..8]=up`（列主序存储）。
pub fn rotation_matrix_from_position_velocity(
    position: DVec3,
    velocity: DVec3,
    ellipsoid: &Ellipsoid,
) -> DMat3 {
    let normal = ellipsoid
        .geodetic_surface_normal(position)
        .expect("position must not be at the center of the ellipsoid");

    let mut right = velocity.cross(normal);
    if vec3_equals_epsilon(right, DVec3::ZERO, math_utils::EPSILON6) {
        right = DVec3::X;
    }

    let up = crate::ellipsoid::normalize_cartesian3(right.cross(velocity));
    right = -velocity.cross(up);
    right = crate::ellipsoid::normalize_cartesian3(right);

    DMat3::from_cols(velocity, right, up)
}

/// 求一个刚体（正交旋转 + 平移）变换的逆。
/// 映射到 `Matrix4.inverseTransformation`：`[R^T | -R^T * t]`。
pub fn inverse_transformation(matrix: &DMat4) -> DMat4 {
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

/// 将 (x, y, z) 映射为 (z, x, y) 的 swizzle 矩阵，用于把 3D ENU 参考系
/// 转换为 2D 投影参考系。
/// 映射到 `Transforms.SWIZZLE_3D_TO_2D_MATRIX`（列主序的列为 [Y, Z, X]）。
fn swizzle_3d_to_2d_matrix() -> DMat4 {
    DMat4::from_cols(
        DVec3::Y.extend(0.0),
        DVec3::Z.extend(0.0),
        DVec3::X.extend(0.0),
        DVec3::ZERO.extend(1.0),
    )
}

/// 由 fixed 参考系中的变换计算航向/俯仰/翻滚角。
/// 映射到 `Transforms.fixedFrameToHeadingPitchRoll`
pub fn fixed_frame_to_heading_pitch_roll(
    transform: &DMat4,
    ellipsoid: &Ellipsoid,
) -> HeadingPitchRoll {
    let center = transform.w_axis.truncate();
    if center == DVec3::ZERO {
        return HeadingPitchRoll::new(0.0, 0.0, 0.0);
    }
    let to_fixed_frame = inverse_transformation(&east_north_up_to_fixed_frame(center, ellipsoid));

    // Matrix4.setScale(transform, (1,1,1))：归一化每个旋转列
    // （将 xyz 除以其长度），保留 w 分量；然后
    // Matrix4.setTranslation(.., ZERO)。
    let mut transform_copy = *transform;
    let x_scale = transform.x_axis.truncate().length();
    let y_scale = transform.y_axis.truncate().length();
    let z_scale = transform.z_axis.truncate().length();
    transform_copy.x_axis = (transform.x_axis.truncate() / x_scale).extend(transform.x_axis.w);
    transform_copy.y_axis = (transform.y_axis.truncate() / y_scale).extend(transform.y_axis.w);
    transform_copy.z_axis = (transform.z_axis.truncate() / z_scale).extend(transform.z_axis.w);
    transform_copy.w_axis = DVec3::ZERO.extend(1.0);

    let to_fixed_frame = to_fixed_frame * transform_copy;
    let rotation = DMat3::from_cols(
        to_fixed_frame.x_axis.truncate(),
        to_fixed_frame.y_axis.truncate(),
        to_fixed_frame.z_axis.truncate(),
    );
    HeadingPitchRoll::from_quaternion(DQuat::from_mat3(&rotation).normalize())
}

/// 使用给定投影，由 3D 基计算 2D 变换。
/// 映射到 `Transforms.basisTo2D`
pub fn basis_to_2d<P: MapProjection>(projection: &P, matrix: &DMat4) -> DMat4 {
    let rtc_center = matrix.w_axis.truncate();
    let ellipsoid = *projection.ellipsoid();

    let projected_position = if rtc_center == DVec3::ZERO {
        DVec3::ZERO
    } else {
        let cartographic = ellipsoid
            .cartesian_to_cartographic(rtc_center)
            .expect("rtcCenter must be on or above the ellipsoid");
        let p = projection.project(&cartographic);
        DVec3::new(p.z, p.x, p.y)
    };

    let from_enu = east_north_up_to_fixed_frame(rtc_center, &ellipsoid);
    let to_enu = inverse_transformation(&from_enu);
    let rotation = DMat3::from_cols(
        matrix.x_axis.truncate(),
        matrix.y_axis.truncate(),
        matrix.z_axis.truncate(),
    );
    let local = to_enu
        * DMat4::from_cols(
            rotation.x_axis.extend(0.0),
            rotation.y_axis.extend(0.0),
            rotation.z_axis.extend(0.0),
            DVec3::ZERO.extend(1.0),
        );
    let mut result = swizzle_3d_to_2d_matrix() * local;
    result.w_axis = projected_position.extend(1.0);
    result
}

/// 由以椭球为中心的 3D 参考系计算 2D 模型矩阵。
/// 映射到 `Transforms.ellipsoidTo2DModelMatrix`
pub fn ellipsoid_to_2d_model_matrix<P: MapProjection>(projection: &P, center: DVec3) -> DMat4 {
    let ellipsoid = *projection.ellipsoid();
    let from_enu = east_north_up_to_fixed_frame(center, &ellipsoid);
    let to_enu = inverse_transformation(&from_enu);
    let cartographic = ellipsoid
        .cartesian_to_cartographic(center)
        .expect("center must be on or above the ellipsoid");
    let p = projection.project(&cartographic);
    let projected_position = DVec3::new(p.z, p.x, p.y);
    let translation = DMat4::from_translation(projected_position);
    translation * (swizzle_3d_to_2d_matrix() * to_enu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_enu_at_equator_prime_meridian() {
        let ellipsoid = Ellipsoid::WGS84;
        let origin = DVec3::new(6378137.0, 0.0, 0.0);
        let frame = east_north_up_to_fixed_frame(origin, &ellipsoid);

        // 在 (lat=0, lon=0) 处：East = (0,1,0), North = (0,0,1), Up = (1,0,0)
        let east = frame.x_axis.truncate();
        let north = frame.y_axis.truncate();
        let up = frame.z_axis.truncate();

        assert!(east.abs_diff_eq(DVec3::Y, 1e-10), "East: {:?}", east);
        assert!(north.abs_diff_eq(DVec3::Z, 1e-10), "North: {:?}", north);
        assert!(up.abs_diff_eq(DVec3::X, 1e-10), "Up: {:?}", up);
    }

    #[test]
    #[allow(clippy::excessive_precision)]
    fn test_enu_at_north_pole() {
        let ellipsoid = Ellipsoid::WGS84;
        let origin = DVec3::new(0.0, 0.0, 6356752.3142451793);
        let frame = east_north_up_to_fixed_frame(origin, &ellipsoid);

        // 在北极：Up = (0,0,1)
        let up = frame.z_axis.truncate();
        assert!(up.abs_diff_eq(DVec3::Z, 1e-10), "Up at pole: {:?}", up);
    }

    #[test]
    fn test_heading_pitch_roll_quaternion_identity() {
        let hpr = HeadingPitchRoll::new(0.0, 0.0, 0.0);
        let quat = hpr.to_quaternion();
        assert!((quat.w - 1.0).abs() < 1e-10);
        assert!(quat.x.abs() < 1e-10);
        assert!(quat.y.abs() < 1e-10);
        assert!(quat.z.abs() < 1e-10);
    }

    #[test]
    fn test_heading_pitch_roll_heading_90() {
        let hpr = HeadingPitchRoll::new(PI / 2.0, 0.0, 0.0);
        let quat = hpr.to_quaternion();
        // 90° 航向 → 绕 -Z 旋转 90°（CesiumJS 约定），因此
        // z = -sin(PI/4), w = cos(PI/4)。
        assert!((quat.z + (PI / 4.0).sin()).abs() < 1e-10);
        assert!((quat.w - (PI / 4.0).cos()).abs() < 1e-10);
    }

    #[test]
    fn test_translation_rotation_scale_to_matrix() {
        let trs = TranslationRotationScale::new(
            DVec3::new(1.0, 2.0, 3.0),
            DQuat::IDENTITY,
            DVec3::ONE,
        );
        let mat = trs.to_matrix4();
        assert_eq!(mat.w_axis.truncate(), DVec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn test_icrf_to_fixed() {
        // 在 J2000 历元，GMST ≈ 280.46° → 旋转应非单位阵
        let j2000_seconds = 2451545.0 * 86400.0;
        let mat = compute_icrf_to_fixed_matrix(j2000_seconds).unwrap();
        // 应为一个有效的旋转矩阵（det ≈ 1）
        let det = mat.determinant();
        assert!((det - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_fixed_to_icrf_is_inverse() {
        let seconds = 2451545.0 * 86400.0 + 3600.0;
        let icrf_to_fixed = compute_icrf_to_fixed_matrix(seconds).unwrap();
        let fixed_to_icrf = compute_fixed_to_icrf_matrix(seconds).unwrap();
        let product = icrf_to_fixed * fixed_to_icrf;
        // 应为单位阵
        assert!(product.abs_diff_eq(DMat3::IDENTITY, 1e-10));
    }
}
