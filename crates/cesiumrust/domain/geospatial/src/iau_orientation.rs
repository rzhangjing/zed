//! IAU 2000 姿态模型 + IauOrientationAxes。
//!
//! 数据来自报告《Report of the IAU/IAG Working Group on Cartographic
//! Coordinates and Rotational Elements: 2000》。
//!
//! 本模块基于该报告的摄动级数展开，给定儒略日期计算天体的自转轴赤经、赤纬与
//! 本初子午线旋转角，再组装为从 ICRF 惯性系到天体固连系的 3×3 旋转矩阵。
//! 其中的系数常量与正弦/余弦幅角都直接取自报告附表的数值。

use cesium_time::JulianDate;
use glam::{DVec3, DMat3, DQuat};

/// 地球动力学时（TDT/TT）与国际原子时（TAI）的固定偏移（秒）。
const TDT_MINUS_TAI: f64 = 32.184;
/// J2000.0 历元对应的儒略日数（自参考日算起）。
const J2000D: f64 = 2451545.0;
/// 一度对应的弧弧度。
const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;
/// 一个儒略世纪的天数，用于把天数换算为世纪数 t。
const DAYS_PER_JULIAN_CENTURY: f64 = 36525.0;
/// 2π（弧度），用于将角度归一到 [0, 2π)。
const TWO_PI: f64 = 2.0 * std::f64::consts::PI;
/// π/2（弧度），旋转矩阵计算中的相位偏移。
const PI_OVER_TWO: f64 = std::f64::consts::PI / 2.0;

/// 月球摄动级数中各幅角的日变化率系数（度/日），取自 IAU 2000 报告附表。
const C1: f64 = -0.0529921;
const C2: f64 = -0.1059842;
const C3: f64 = 13.0120009;
const C4: f64 = 13.3407154;
const C5: f64 = 0.9856003;
const C6: f64 = 26.4057084;
const C7: f64 = 13.064993;
const C8: f64 = 0.3287146;
const C9: f64 = 1.7484877;
const C10: f64 = -0.1589763;
const C11: f64 = 0.0036096;
const C12: f64 = 0.1643573;
const C13: f64 = 12.9590088;

/// 某天体的姿态参数。
#[derive(Clone, Debug, Default)]
pub struct IauOrientationParameters {
    /// 自转北端指向的赤经（弧度）。
    pub right_ascension: f64,
    /// 自转北端指向的赤纬（弧度）。
    pub declination: f64,
    /// 本初子午线相对参考方向的旋转角（弧度）。
    pub rotation: f64,
    /// 旋转角随时间的变化率（弧度/秒）。
    pub rotation_rate: f64,
}

/// 计算月球的姿态参数。
///
/// 以儒略日期为输入，计算多个摄动幅角 e1..e13 的正弦/余弦，再按报告给定的
/// 截断级数组装出赤经、赤纬、旋转角与旋转率。
///
/// # 返回
/// [`IauOrientationParameters`]：该时刻月球自转轴与子午线位置（均为弧度）。
pub fn compute_moon(date: &JulianDate) -> IauOrientationParameters {
    let date_tt = date.add_seconds(TDT_MINUS_TAI);
    // 换算为自 J2000.0 起的天数 d 与儒略世纪数 t。
    let d = date_tt.total_days() - J2000D;
    let t = d / DAYS_PER_JULIAN_CENTURY;

    // 逐幅角：线性于 d 的角度（度→弧度），作为摄动级数的自变量。
    let e1 = (125.045 + C1 * d) * RADIANS_PER_DEGREE;
    let e2 = (250.089 + C2 * d) * RADIANS_PER_DEGREE;
    let e3 = (260.008 + C3 * d) * RADIANS_PER_DEGREE;
    let e4 = (176.625 + C4 * d) * RADIANS_PER_DEGREE;
    let e5 = (357.529 + C5 * d) * RADIANS_PER_DEGREE;
    let e6 = (311.589 + C6 * d) * RADIANS_PER_DEGREE;
    let e7 = (134.963 + C7 * d) * RADIANS_PER_DEGREE;
    let e8 = (276.617 + C8 * d) * RADIANS_PER_DEGREE;
    let e9 = (34.226 + C9 * d) * RADIANS_PER_DEGREE;
    let e10 = (15.134 + C10 * d) * RADIANS_PER_DEGREE;
    let e11 = (119.743 + C11 * d) * RADIANS_PER_DEGREE;
    let e12 = (239.961 + C12 * d) * RADIANS_PER_DEGREE;
    let e13 = (25.053 + C13 * d) * RADIANS_PER_DEGREE;

    // 预先计算各幅角的正弦与余弦，供下面多个级数表达式复用。
    let sin_e1 = e1.sin();
    let sin_e2 = e2.sin();
    let sin_e3 = e3.sin();
    let sin_e4 = e4.sin();
    let sin_e5 = e5.sin();
    let sin_e6 = e6.sin();
    let sin_e7 = e7.sin();
    let sin_e8 = e8.sin();
    let sin_e9 = e9.sin();
    let sin_e10 = e10.sin();
    let sin_e11 = e11.sin();
    let sin_e12 = e12.sin();
    let sin_e13 = e13.sin();

    let cos_e1 = e1.cos();
    let cos_e2 = e2.cos();
    let cos_e3 = e3.cos();
    let cos_e4 = e4.cos();
    let cos_e5 = e5.cos();
    let cos_e6 = e6.cos();
    let cos_e7 = e7.cos();
    let cos_e8 = e8.cos();
    let cos_e9 = e9.cos();
    let cos_e10 = e10.cos();
    let cos_e11 = e11.cos();
    let cos_e12 = e12.cos();
    let cos_e13 = e13.cos();

    // 赤经：常数项 + t 的一次项 + 若干部正弦摄动项（系数取自报告），整体度→弧度。
    let right_ascension = (269.9949 + 0.0031 * t - 3.8787 * sin_e1 - 0.1204 * sin_e2
        + 0.07 * sin_e3
        - 0.0172 * sin_e4
        + 0.0072 * sin_e6
        - 0.0052 * sin_e10
        + 0.0043 * sin_e13)
        * RADIANS_PER_DEGREE;

    let declination = (66.5392 + 0.013 * t + 1.5419 * cos_e1 + 0.0239 * cos_e2
        - 0.0278 * cos_e3
        + 0.0068 * cos_e4
        - 0.0029 * cos_e6
        + 0.0009 * cos_e7
        + 0.0008 * cos_e10
        - 0.0009 * cos_e13)
        * RADIANS_PER_DEGREE;

    let rotation = (38.3213 + 13.17635815 * d - 1.4e-12 * d * d + 3.561 * sin_e1
        + 0.1208 * sin_e2
        - 0.0642 * sin_e3
        + 0.0158 * sin_e4
        + 0.0252 * sin_e5
        - 0.0066 * sin_e6
        - 0.0047 * sin_e7
        - 0.0046 * sin_e8
        + 0.0028 * sin_e9
        + 0.0052 * sin_e10
        + 0.004 * sin_e11
        + 0.0019 * sin_e12
        - 0.0044 * sin_e13)
        * RADIANS_PER_DEGREE;

    // 旋转率：对旋转角关于时间求导（d 的导数引入 C_i 因子），除以 86400 秒后度→弧度。
    let rotation_rate = ((13.17635815 - 1.4e-12 * (2.0 * d) + 3.561 * cos_e1 * C1
        + 0.1208 * cos_e2 * C2
        - 0.0642 * cos_e3 * C3
        + 0.0158 * cos_e4 * C4
        + 0.0252 * cos_e5 * C5
        - 0.0066 * cos_e6 * C6
        - 0.0047 * cos_e7 * C7
        - 0.0046 * cos_e8 * C8
        + 0.0028 * cos_e9 * C9
        + 0.0052 * cos_e10 * C10
        + 0.004 * cos_e11 * C11
        + 0.0019 * cos_e12 * C12
        - 0.0044 * cos_e13 * C13)
        / 86400.0)
        * RADIANS_PER_DEGREE;

    IauOrientationParameters {
        right_ascension,
        declination,
        rotation,
        rotation_rate,
    }
}

/// 将任意弧度归一到 [0, 2π) 区间。
fn zero_to_two_pi(angle: f64) -> f64 {
    let mut result = angle % TWO_PI;
    if result < 0.0 {
        result += TWO_PI;
    }
    result
}

/// 由赤经和赤纬计算旋转矩阵。
///
/// 以赤经/赤纬确定自转轴（z 轴）与一个参考 x 轴（赤经加 90°），y 轴由叉积导出，
/// 三轴构成从惯性系到中间系的岁差/定向旋转。
fn compute_rotation_matrix(alpha: f64, delta: f64) -> DMat3 {
    let x_axis = DVec3::new(
        (alpha + PI_OVER_TWO).cos(),
        (alpha + PI_OVER_TWO).sin(),
        0.0,
    );

    let cos_dec = delta.cos();
    let z_axis = DVec3::new(cos_dec * alpha.cos(), cos_dec * alpha.sin(), delta.sin());

    let y_axis = z_axis.cross(x_axis);

    // CesiumJS 设置 result[0]=xAxis.x, result[1]=yAxis.x, result[2]=zAxis.x，
    // 即每一行是一个轴。glam 的 from_cols_array 是列主序：
    // col0=(row0[0],row1[0],row2[0]) = (x.x, y.x, z.x)
    DMat3::from_cols_array(&[
        x_axis.x, y_axis.x, z_axis.x,
        x_axis.y, y_axis.y, z_axis.y,
        x_axis.z, y_axis.z, z_axis.z,
    ])
}

/// 计算从 ICRF 到某 Globe 的 Fixed 轴的旋转（月球）。
///
/// 这就是以 `compute_moon` 为计算函数的 `IauOrientationAxes.evaluate()` 方法。
///
/// 先用赤经/赤纬得到定向矩阵 prec_mtx，再由旋转角绕 z 轴构造共轭四元数对应的 rot_mtx，
/// 返回二者的乘积作为最终 ICRF→Fixed 旋转。
pub fn evaluate_icrf_to_fixed(date: &JulianDate) -> DMat3 {
    let alpha_delta_w = compute_moon(date);
    let prec_mtx = compute_rotation_matrix(alpha_delta_w.right_ascension, alpha_delta_w.declination);

    let rot = zero_to_two_pi(alpha_delta_w.rotation);
    let quat = DQuat::from_axis_angle(DVec3::Z, rot);
    let rot_mtx = DMat3::from_quat(quat.conjugate());

    rot_mtx * prec_mtx
}
