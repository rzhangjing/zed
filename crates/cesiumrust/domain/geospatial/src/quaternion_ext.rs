//! 四元数扩展函数 —— glam 未提供的额外四元数算法。
//!
//! 实现以下四元数扩展方法：
//! computeAxis、computeAngle、log、exp、computeInnerQuadrangle、squad、fastSlerp、fastSquad
//!
//! 这些方法共同支撑平滑的旋转插值：log/exp 建立单位四元数与李代数（纯虚向量）间的双射，
//! computeInnerQuadrangle 求 squad 的控制点，而 fastSlerp/fastSquad 用多项式逼近换取速度。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::excessive_precision)]
use glam::{DMat3, DQuat, DVec3};

use crate::math_utils;

/// 计算一个四元数的归一化旋转轴。
/// 映射到 `Quaternion.computeAxis`。
///
/// 当 w 接近 ±1（旋转角趋于 0 或 2π）时轴未定义，回退为 +X 轴；否则用 1/√(1-w²) 缩放虚部。
pub fn compute_axis(quaternion: DQuat) -> DVec3 {
    let w = quaternion.w;
    if (w - 1.0).abs() < math_utils::EPSILON6 || (w + 1.0).abs() < math_utils::EPSILON6 {
        return DVec3::new(1.0, 0.0, 0.0);
    }

    let scalar = 1.0 / (1.0 - w * w).sqrt();
    DVec3::new(
        quaternion.x * scalar,
        quaternion.y * scalar,
        quaternion.z * scalar,
    )
}

/// 计算给定的四元数的旋转角度。
/// 映射到 `Quaternion.computeAngle`。
///
/// w 接近 1 时角度为 0；否则返回 2·acos(w)（对应单位四元数的完整旋转角）。
pub fn compute_angle(quaternion: DQuat) -> f64 {
    if (quaternion.w - 1.0).abs() < math_utils::EPSILON6 {
        return 0.0;
    }
    2.0 * quaternion.w.acos()
}

/// 对数四元数函数。
/// 映射到 `Quaternion.log`。
/// 返回对数的 Cartesian3（向量部分）。
///
/// 以 θ=acos(w) 为旋转半角，将虚部按 θ/sinθ 缩放；θ=0 时缩放因子取 0 以避免除零。
pub fn quaternion_log(quaternion: DQuat) -> DVec3 {
    let theta = math_utils::acos_clamped(quaternion.w);
    let mut theta_over_sin_theta = 0.0;

    if theta != 0.0 {
        theta_over_sin_theta = theta / theta.sin();
    }

    DVec3::new(
        quaternion.x * theta_over_sin_theta,
        quaternion.y * theta_over_sin_theta,
        quaternion.z * theta_over_sin_theta,
    )
}

/// 指数四元数函数。
/// 映射到 `Quaternion.exp`。
/// 接受一个 Cartesian3（纯虚四元数）并返回一个单位四元数。
///
/// 与 [`quaternion_log`] 互逆：以 θ=|v| 为模长，虚部按 sinθ/θ 缩放，实部为 cosθ。
pub fn quaternion_exp(cartesian: DVec3) -> DQuat {
    let theta = cartesian.length();
    let mut sin_theta_over_theta = 0.0;

    if theta != 0.0 {
        sin_theta_over_theta = theta.sin() / theta;
    }

    DQuat::from_xyzw(
        cartesian.x * sin_theta_over_theta,
        cartesian.y * sin_theta_over_theta,
        cartesian.z * sin_theta_over_theta,
        theta.cos(),
    )
}

/// 计算一个内部四边形点。
/// 它将计算能保证 squad 曲线为 C¹ 的四元数。
/// 映射到 `Quaternion.computeInnerQuadrangle`。
///
/// 以 q1 为参考将 q0、q2 取对数、求和缩放后回指数，得到控制点 s1，保证拼接处一阶连续。
pub fn compute_inner_quadrangle(q0: DQuat, q1: DQuat, q2: DQuat) -> DQuat {
    let q_inv = q1.conjugate();

    let product1 = q_inv * q2;
    let cart0 = quaternion_log(product1);

    let product2 = q_inv * q0;
    let cart1 = quaternion_log(product2);

    let sum = cart0 + cart1;
    let negated = sum * (-0.25);
    let exp_result = quaternion_exp(negated);

    q1 * exp_result
}

/// 使用给定的四元数计算在 t 处的线性插值或外推。
/// 映射到 `Quaternion.lerp`。
///
/// 按 (1-t) 与 t 分别缩放 start、end 后逐分量相加；不做归一化，仅用于 slerp 退化分支的回退。
pub fn quaternion_lerp(start: DQuat, end: DQuat, t: f64) -> DQuat {
    let scaled_end = DQuat::from_xyzw(
        end.x * t,
        end.y * t,
        end.z * t,
        end.w * t,
    );
    let scaled_start = DQuat::from_xyzw(
        start.x * (1.0 - t),
        start.y * (1.0 - t),
        start.z * (1.0 - t),
        start.w * (1.0 - t),
    );
    DQuat::from_xyzw(
        scaled_start.x + scaled_end.x,
        scaled_start.y + scaled_end.y,
        scaled_start.z + scaled_end.z,
        scaled_start.w + scaled_end.w,
    )
}

/// 计算在 t 处的球面线性插值或外推。
/// 映射到 `Quaternion.slerp`（带 lerp 回退的 CesiumJS 版本）。
pub fn cesium_slerp(start: DQuat, end: DQuat, t: f64) -> DQuat {
    let mut dot = start.x * end.x + start.y * end.y + start.z * end.z + start.w * end.w;

    // start 之间的夹角必须是锐角。由于 q 和 -q 表示
    // 相同的旋转，取反 q 以获得锐角。
    let mut r = end;
    if dot < 0.0 {
        dot = -dot;
        r = DQuat::from_xyzw(-end.x, -end.y, -end.z, -end.w);
    }

    // dot > 0，当点积趋近于 1 时，四元数之间的
    // 夹角消失。使用线性插值。
    if 1.0 - dot < math_utils::EPSILON6 {
        return quaternion_lerp(start, r, t);
    }

    let theta = dot.acos();
    let sin_theta = theta.sin();
    let s0 = ((1.0 - t) * theta).sin() / sin_theta;
    let s1 = (t * theta).sin() / sin_theta;

    DQuat::from_xyzw(
        start.x * s0 + r.x * s1,
        start.y * s0 + r.y * s1,
        start.z * s0 + r.z * s1,
        start.w * s0 + r.w * s1,
    )
}

/// 计算四元数之间的球面四边形插值。
/// 映射到 `Quaternion.squad`。
///
/// 由两组 slerp 结果再做一次带 2t(1-t) 参数的 slerp，得到经过 q0、q1 且以 s0、s1 为切向控制点的平滑插值。
pub fn squad(q0: DQuat, q1: DQuat, s0: DQuat, s1: DQuat, t: f64) -> DQuat {
    let slerp0 = cesium_slerp(q0, q1, t);
    let slerp1 = cesium_slerp(s0, s1, t);
    cesium_slerp(slerp0, slerp1, 2.0 * t * (1.0 - t))
}

// fastSlerp 多项式逼近的常量
// 用于修正第 8 项系数，使 cos 的多项式逼近精度达 10⁻⁶。
const OPMU: f64 = 1.90110745351730037;

/// 为 fastSlerp 预计算的 u 和 v 数组。
/// 前 7 项由整数序列算出，第 8 项用逼近常数 OPMU 修正以保证精度。
fn fast_slerp_coefficients() -> ([f64; 8], [f64; 8]) {
    let mut u = [0.0f64; 8];
    let mut v = [0.0f64; 8];

    for i in 0..7 {
        let s = i as f64 + 1.0;
        let t = 2.0 * s + 1.0;
        u[i] = 1.0 / (s * t);
        v[i] = s / t;
    }

    u[7] = OPMU / (8.0 * 17.0);
    v[7] = (OPMU * 8.0) / 17.0;

    (u, v)
}

/// 计算在 t 处的球面线性插值或外推。
/// 该实现比 slerp 更快，但仅精确到 10⁻⁶。
/// 映射到 `Quaternion.fastSlerp`。
pub fn fast_slerp(start: DQuat, end: DQuat, t: f64) -> DQuat {
    let (u, v) = fast_slerp_coefficients();

    let mut x = start.x * end.x + start.y * end.y + start.z * end.z + start.w * end.w;

    let sign;
    // 点积为负时翻转 end 符号（q 与 -q 表示同一旋转），保证取锐角分支。
    if x >= 0.0 {
        sign = 1.0;
    } else {
        sign = -1.0;
        x = -x;
    }

    let xm1 = x - 1.0;
    let d = 1.0 - t;
    let sqr_t = t * t;
    let sqr_d = d * d;

    let mut b_t = [0.0f64; 8];
    let mut b_d = [0.0f64; 8];

    // 用 u/v 系数与 (x-1) 构造 Horner 多项式的逐项因子，分别对应 t 与 d=1-t。
    for i in 0..8 {
        b_t[i] = (u[i] * sqr_t - v[i]) * xm1;
        b_d[i] = (u[i] * sqr_d - v[i]) * xm1;
    }

    let c_t = sign
        * t
        * (1.0
            + b_t[0]
                * (1.0
                    + b_t[1]
                        * (1.0
                            + b_t[2]
                                * (1.0
                                    + b_t[3]
                                        * (1.0
                                            + b_t[4]
                                                * (1.0
                                                    + b_t[5]
                                                        * (1.0 + b_t[6] * (1.0 + b_t[7]))))))));
    let c_d = d
        * (1.0
            + b_d[0]
                * (1.0
                    + b_d[1]
                        * (1.0
                            + b_d[2]
                                * (1.0
                                    + b_d[3]
                                        * (1.0
                                            + b_d[4]
                                                * (1.0
                                                    + b_d[5]
                                                        * (1.0 + b_d[6] * (1.0 + b_d[7]))))))));

    DQuat::from_xyzw(
        start.x * c_d + end.x * c_t,
        start.y * c_d + end.y * c_t,
        start.z * c_d + end.z * c_t,
        start.w * c_d + end.w * c_t,
    )
}

/// 计算四元数之间的球面四边形插值。
/// 一个比 squad 更快但精度较低的实现。
/// 映射到 `Quaternion.fastSquad`。
pub fn fast_squad(q0: DQuat, q1: DQuat, s0: DQuat, s1: DQuat, t: f64) -> DQuat {
    let slerp0 = fast_slerp(q0, q1, t);
    let slerp1 = fast_slerp(s0, s1, t);
    fast_slerp(slerp0, slerp1, 2.0 * t * (1.0 - t))
}

/// 从给定的旋转矩阵（Matrix3）计算一个 Quaternion。
/// 映射到 `Quaternion.fromRotationMatrix`。
/// 使用标准的 Shepperd 方法，并采用正确的符号约定。
///
/// 根据矩阵的迹与各对角元选最大者作为主元分支，避免开方出现数值不稳定；四个分支分别以 w/x/y/z 为主。
pub fn from_rotation_matrix(matrix: &DMat3) -> DQuat {
    let m00 = matrix.x_axis.x;
    let m01 = matrix.y_axis.x;
    let m02 = matrix.z_axis.x;
    let m10 = matrix.x_axis.y;
    let m11 = matrix.y_axis.y;
    let m12 = matrix.z_axis.y;
    let m20 = matrix.x_axis.z;
    let m21 = matrix.y_axis.z;
    let m22 = matrix.z_axis.z;
    let trace = m00 + m11 + m22;

    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0; // s = 4w
        let w = 0.25 * s;
        let inv_s = 1.0 / s;
        let x = (m21 - m12) * inv_s;
        let y = (m02 - m20) * inv_s;
        let z = (m10 - m01) * inv_s;
        DQuat::from_xyzw(x, y, z, w)
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0; // s = 4x
        let w = (m21 - m12) / s;
        let x = 0.25 * s;
        let y = (m01 + m10) / s;
        let z = (m02 + m20) / s;
        DQuat::from_xyzw(x, y, z, w)
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0; // s = 4y
        let w = (m02 - m20) / s;
        let x = (m01 + m10) / s;
        let y = 0.25 * s;
        let z = (m12 + m21) / s;
        DQuat::from_xyzw(x, y, z, w)
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0; // s = 4z
        let w = (m10 - m01) / s;
        let x = (m02 + m20) / s;
        let y = (m12 + m21) / s;
        let z = 0.25 * s;
        DQuat::from_xyzw(x, y, z, w)
    }
}

/// 检查两个四元数是否在一个 epsilon 范围内相等。
/// 映射到 `Quaternion.equalsEpsilon`。
///
/// 四个分量均在 ±epsilon 内则视为相等（不处理 q 与 -q 同旋转的情况）。
pub fn equals_epsilon(left: DQuat, right: DQuat, epsilon: f64) -> bool {
    (left.x - right.x).abs() <= epsilon
        && (left.y - right.y).abs() <= epsilon
        && (left.z - right.z).abs() <= epsilon
        && (left.w - right.w).abs() <= epsilon
}
