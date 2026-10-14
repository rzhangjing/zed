//! 太阳与月亮位置计算。
//!
//! 基于简化行星理论（VSOP87/月球坐标），
//! 以儒略日期为输入计算太阳与月亮在 ECEF 坐标中的
//! 近似位置。

use glam::DVec3;

/// 天文单位（米）。
pub const AU_IN_METERS: f64 = 1.495978707e11;

/// J2000 历元的儒略日期（2000-01-01 12:00 TT）。
pub const J2000_EPOCH: f64 = 2451545.0;

/// 自 J2000 起的儒略世纪数。
fn julian_centuries(julian_date: f64) -> f64 {
    (julian_date - J2000_EPOCH) / 36525.0
}

/// 计算太阳在地心惯性（ECI）坐标系中的位置。
///
/// 使用简化的 VSOP87 理论（低精度，约 0.01 度准确度）。
///
/// # 参数
/// * `julian_date` - 儒略日期（TT）
///
/// # 返回
/// ECI 中的太阳位置（米），ICRF 框架
pub fn compute_sun_position_eci(julian_date: f64) -> DVec3 {
    let t = julian_centuries(julian_date);

    // 平黄经（度）
    let l0 = normalize_degrees(280.46646 + 36000.76983 * t + 0.0003032 * t * t);

    // 平近点角（度）
    let m = normalize_degrees(357.52911 + 35999.05029 * t - 0.0001537 * t * t);
    let m_rad = m.to_radians();

    // 中心差
    let c = (1.914602 - 0.004817 * t - 0.000014 * t * t) * m_rad.sin()
        + (0.019993 - 0.000101 * t) * (2.0 * m_rad).sin()
        + 0.000289 * (3.0 * m_rad).sin();

    // 太阳真黄经
    let sun_lon = (l0 + c).to_radians();

    // 太阳距离（AU）
    let e = 0.016708634 - 0.000042037 * t - 0.0000001267 * t * t;
    let v = m_rad + c.to_radians();
    let r = 1.000001018 * (1.0 - e * e) / (1.0 + e * v.cos());

    // 转换为米
    let r_meters = r * AU_IN_METERS;

    // 黄赤交角
    let epsilon = (23.439291 - 0.0130042 * t).to_radians();

    // ECI 坐标（ICRF 近似）
    let x = r_meters * sun_lon.cos();
    let y = r_meters * sun_lon.sin() * epsilon.cos();
    let z = r_meters * sun_lon.sin() * epsilon.sin();

    DVec3::new(x, y, z)
}

/// 计算从地球指向太阳的方向（归一化），位于 ECI 中。
pub fn compute_sun_direction_eci(julian_date: f64) -> DVec3 {
    compute_sun_position_eci(julian_date).normalize()
}

/// 计算月亮在地心惯性（ECI）坐标系中的位置。
///
/// 使用简化的月球理论（低精度，约 0.1 度准确度）。
///
/// # 参数
/// * `julian_date` - 儒略日期（TT）
///
/// # 返回
/// ECI 中的月亮位置（米），ICRF 框架
pub fn compute_moon_position_eci(julian_date: f64) -> DVec3 {
    let t = julian_centuries(julian_date);

    // 月亮平黄经
    let l = normalize_degrees(218.3165 + 481267.8813 * t);
    let l_rad = l.to_radians();

    // 月亮平近点角
    let m = normalize_degrees(134.9634 + 477198.8676 * t);
    let m_rad = m.to_radians();

    // 月亮平角距
    let d = normalize_degrees(297.8502 + 445267.1115 * t);
    let d_rad = d.to_radians();

    // 月亮的纬度幅角
    let f = normalize_degrees(93.2720 + 483202.0175 * t);
    let f_rad = f.to_radians();

    // 太阳平近点角
    let ms = normalize_degrees(357.5291 + 35999.0503 * t);
    let ms_rad = ms.to_radians();

    // 黄经（简化）
    let lambda = l_rad
        + 0.1098_f64.to_radians() * m_rad.sin()
        + 0.0223_f64.to_radians() * (2.0 * d_rad - m_rad).sin()
        + 0.0115_f64.to_radians() * (2.0 * d_rad).sin()
        + 0.0037_f64.to_radians() * ms_rad.sin();

    // 黄纬（简化）
    let beta = 0.0895_f64.to_radians() * f_rad.sin()
        + 0.0049_f64.to_radians() * (m_rad + f_rad).sin()
        + 0.0048_f64.to_radians() * (m_rad - f_rad).sin();

    // 距离（km → 米）
    let dist_km = 385001.0 - 20905.0 * m_rad.cos() - 3699.0 * (2.0 * d_rad - m_rad).cos()
        - 2956.0 * (2.0 * d_rad).cos();
    let dist_meters = dist_km * 1000.0;

    // 黄赤交角
    let epsilon = (23.439291 - 0.0130042 * t).to_radians();

    // 黄道转赤道
    let x_ecl = dist_meters * beta.cos() * lambda.cos();
    let y_ecl = dist_meters * beta.cos() * lambda.sin();
    let z_ecl = dist_meters * beta.sin();

    // 按黄赤交角旋转
    let x = x_ecl;
    let y = y_ecl * epsilon.cos() - z_ecl * epsilon.sin();
    let z = y_ecl * epsilon.sin() + z_ecl * epsilon.cos();

    DVec3::new(x, y, z)
}

/// 计算从地球指向月亮的方向（归一化），位于 ECI 中。
pub fn compute_moon_direction_eci(julian_date: f64) -> DVec3 {
    compute_moon_position_eci(julian_date).normalize()
}

/// 近似 GMST（格林尼治平均恒星时），单位为弧度。
pub fn compute_gmst(julian_date: f64) -> f64 {
    let t = julian_centuries(julian_date);
    // GMST（度）
    let gmst_deg = 280.46061837 + 360.98564736629 * (julian_date - J2000_EPOCH)
        + 0.000387933 * t * t
        - t * t * t / 38710000.0;
    normalize_degrees(gmst_deg).to_radians()
}

/// 使用 GMST 将 ECI 向量旋转到 ECEF。
pub fn eci_to_ecef(eci: DVec3, julian_date: f64) -> DVec3 {
    let gmst = compute_gmst(julian_date);
    let cos_g = gmst.cos();
    let sin_g = gmst.sin();

    DVec3::new(
        cos_g * eci.x + sin_g * eci.y,
        -sin_g * eci.x + cos_g * eci.y,
        eci.z,
    )
}

/// 计算太阳在 ECEF 坐标中的位置。
pub fn compute_sun_position_ecef(julian_date: f64) -> DVec3 {
    let eci = compute_sun_position_eci(julian_date);
    eci_to_ecef(eci, julian_date)
}

/// 计算月亮在 ECEF 坐标中的位置。
pub fn compute_moon_position_ecef(julian_date: f64) -> DVec3 {
    let eci = compute_moon_position_eci(julian_date);
    eci_to_ecef(eci, julian_date)
}

/// 将角度归一化到 [0, 360)。
fn normalize_degrees(degrees: f64) -> f64 {
    let result = degrees % 360.0;
    if result < 0.0 {
        result + 360.0
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_sun_position_j2000() {
        // 在 J2000 历元，太阳应大致位于
        // 黄经约 280 度的方向
        let pos = compute_sun_position_eci(J2000_EPOCH);

        // 距离应约为 1 AU
        let dist = pos.length();
        assert!((dist - AU_IN_METERS).abs() / AU_IN_METERS < 0.02); // 在 2% 以内
    }

    #[test]
    fn test_sun_direction_normalized() {
        let dir = compute_sun_direction_eci(J2000_EPOCH);
        assert!((dir.length() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_moon_position_distance() {
        // 月亮应距地球约 385,000 km
        let pos = compute_moon_position_eci(J2000_EPOCH);
        let dist_km = pos.length() / 1000.0;

        // 月亮距离在约 356,000 到约 407,000 km 之间变化
        assert!(dist_km > 350_000.0);
        assert!(dist_km < 410_000.0);
    }

    #[test]
    fn test_moon_direction_normalized() {
        let dir = compute_moon_direction_eci(J2000_EPOCH);
        assert!((dir.length() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_gmst_range() {
        let gmst = compute_gmst(J2000_EPOCH);
        assert!(gmst >= 0.0);
        assert!(gmst < 2.0 * PI);
    }

    #[test]
    fn test_eci_to_ecef_preserves_magnitude() {
        let eci = DVec3::new(1.0e11, 2.0e10, 3.0e10);
        let ecef = eci_to_ecef(eci, J2000_EPOCH);

        // 旋转应保持模长
        assert!((eci.length() - ecef.length()).abs() / eci.length() < 1e-10);
    }

    #[test]
    fn test_sun_position_ecef() {
        let pos = compute_sun_position_ecef(J2000_EPOCH);
        let dist = pos.length();
        assert!((dist - AU_IN_METERS).abs() / AU_IN_METERS < 0.02);
    }

    #[test]
    fn test_normalize_degrees() {
        assert!((normalize_degrees(370.0) - 10.0).abs() < 1e-10);
        assert!((normalize_degrees(-10.0) - 350.0).abs() < 1e-10);
        assert!((normalize_degrees(720.0) - 0.0).abs() < 1e-10);
    }
}
