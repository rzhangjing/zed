//! # pixel_diff — 像素级图像比对核心算法
//!
//! ## 设计约定
//! - **颜色通道**：统一按 **RGB** 三通道比对，忽略 Alpha。加载时先转换为 RGB8。
//! - **位深归一化**：所有像素值归一化到 `[0.0, 1.0]` 的 f64 范围后计算指标。
//! - **尺寸不一致**：视为错误，不做重叠区域裁剪（调用方负责提前检查并报错）。
//! - **PSNR Infinity**：两图完全相同时 PSNR = `f64::INFINITY`；JSON 输出中序列化为字符串 `"inf"`。
//!
//! ## SSIM 公式（8×8 滑窗，步长 1）
//! 使用标准 SSIM 公式：
//! ```text
//! SSIM(x,y) = (2*μx*μy + C1)(2*σxy + C2) / ((μx² + μy² + C1)(σx² + σy² + C2))
//! C1 = (K1*L)², C2 = (K2*L)², K1=0.01, K2=0.03, L=1.0 (归一化后动态范围)
//! ```
//! 最终 SSIM 为所有窗口、所有通道的均值。

use image::{DynamicImage, GenericImageView};

/// 比对结果
#[derive(Debug, Clone)]
pub struct DiffMetrics {
    /// 峰值信噪比（dB）。两图完全相同时为 `f64::INFINITY`。
    pub psnr_db: f64,
    /// 结构相似性指数（0.0 – 1.0）。
    pub ssim: f64,
    /// 每通道平均绝对误差，归一化到 [0, 1]。
    pub mean_abs_err: f64,
    /// 是否 psnr_db >= 阈值。
    pub pass: bool,
    /// 所使用的阈值。
    pub threshold: f64,
}

/// 图像尺寸不一致时的错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeMismatch {
    pub baseline: (u32, u32),
    pub candidate: (u32, u32),
}

impl std::fmt::Display for SizeMismatch {
    /// 将尺寸不匹配信息格式化为人类可读的错误文本。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "image size mismatch: baseline={}x{}, candidate={}x{}",
            self.baseline.0, self.baseline.1, self.candidate.0, self.candidate.1
        )
    }
}

impl std::error::Error for SizeMismatch {}

/// 将 DynamicImage 转为 RGB f64 平面，值域 [0,1]
fn to_rgb_f64(img: &DynamicImage) -> (Vec<f64>, u32, u32) {
    let (w, h) = img.dimensions();
    // 先统一转为 RGB8（丢弃 alpha），再逐通道归一化到 [0,1]。
    let rgb = img.to_rgb8();
    let mut buf = Vec::with_capacity((w * h * 3) as usize);
    for pixel in rgb.pixels() {
        for &c in pixel.0.iter() {
            buf.push(c as f64 / 255.0);
        }
    }
    (buf, w, h)
}

/// 计算两幅 RGB f64 平面的 PSNR (dB)
///
/// PSNR = 10 * log10(MAX² / MSE)，MAX=1.0（归一化后）。
/// MSE=0 时返回 INFINITY。
pub fn compute_psnr(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len());
    if a.is_empty() {
        return f64::INFINITY;
    }
    // 均方误差 MSE = 平均((x-y)²)；随后 PSNR = 10*log10(1/MSE)。
    let mse: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        / a.len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (1.0 / mse).log10()
    }
}

/// 计算两幅 RGB f64 平面的 Mean Absolute Error（归一化 [0,1]）
pub fn compute_mae(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len());
    if a.is_empty() {
        return 0.0;
    }
    // 平均绝对误差 MAE = 平均(|x-y|)。
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .sum::<f64>()
        / a.len() as f64
}

/// 计算 SSIM（8×8 滑窗，步长 1，按通道分别计算后取均值）
///
/// 输入：RGB 平面（交错排列 R,G,B,R,G,B,...），宽 w，高 h
pub fn compute_ssim(a: &[f64], b: &[f64], w: u32, h: u32) -> f64 {
    const WINDOW: u32 = 8;
    const K1: f64 = 0.01;
    const K2: f64 = 0.03;
    // L = 1.0（归一化）
    let c1 = (K1 * 1.0).powi(2);
    let c2 = (K2 * 1.0).powi(2);

    if w < WINDOW || h < WINDOW {
        // 图像太小，退化为全局单窗口 SSIM
        return global_ssim(a, b, c1, c2);
    }

    let channels = 3usize;
    let mut total_ssim = 0.0f64;
    let mut count = 0u64;

    // 逐通道、逐 8×8 窗口滑移，累加各窗 SSIM 后取均值。
    for ch in 0..channels {
        for wy in 0..=(h - WINDOW) {
            for wx in 0..=(w - WINDOW) {
                let n = (WINDOW * WINDOW) as f64;
                let mut sum_a = 0.0f64;
                let mut sum_b = 0.0f64;
                let mut sum_a2 = 0.0f64;
                let mut sum_b2 = 0.0f64;
                let mut sum_ab = 0.0f64;

                // 累加窗口内的一阶/二阶矩与交叉项，用于估计均值、方差与协方差。
                for dy in 0..WINDOW {
                    for dx in 0..WINDOW {
                        let idx = (((wy + dy) * w + (wx + dx)) as usize) * channels + ch;
                        let pa = a[idx];
                        let pb = b[idx];
                        sum_a += pa;
                        sum_b += pb;
                        sum_a2 += pa * pa;
                        sum_b2 += pb * pb;
                        sum_ab += pa * pb;
                    }
                }

                // 由累加矩得到 μ、σ² 与 σxy。
                let mu_a = sum_a / n;
                let mu_b = sum_b / n;
                let sigma_a2 = sum_a2 / n - mu_a * mu_a;
                let sigma_b2 = sum_b2 / n - mu_b * mu_b;
                let sigma_ab = sum_ab / n - mu_a * mu_b;

                // 代入标准 SSIM 公式：(2μxμy+C1)(2σxy+C2) / ((μx²+μy²+C1)(σx²+σy²+C2))。
                let numerator = (2.0 * mu_a * mu_b + c1) * (2.0 * sigma_ab + c2);
                let denominator = (mu_a * mu_a + mu_b * mu_b + c1) * (sigma_a2 + sigma_b2 + c2);
                total_ssim += numerator / denominator;
                count += 1;
            }
        }
    }

    if count == 0 {
        1.0
    } else {
        total_ssim / count as f64
    }
}

/// 全局单窗口 SSIM（用于图像尺寸 < 8×8 的退化情况）
fn global_ssim(a: &[f64], b: &[f64], c1: f64, c2: f64) -> f64 {
    let n = a.len() as f64;
    if n == 0.0 {
        return 1.0;
    }
    // 在全平面（而非滑窗）上计算一阶/二阶统计量，当作单个大窗口处理。
    let mu_a = a.iter().sum::<f64>() / n;
    let mu_b = b.iter().sum::<f64>() / n;
    let sigma_a2 = a.iter().map(|x| (x - mu_a).powi(2)).sum::<f64>() / n;
    let sigma_b2 = b.iter().map(|x| (x - mu_b).powi(2)).sum::<f64>() / n;
    let sigma_ab = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x - mu_a) * (y - mu_b))
        .sum::<f64>()
        / n;

    let numerator = (2.0 * mu_a * mu_b + c1) * (2.0 * sigma_ab + c2);
    let denominator = (mu_a * mu_a + mu_b * mu_b + c1) * (sigma_a2 + sigma_b2 + c2);
    numerator / denominator
}

/// 主入口：比对两张已加载的 DynamicImage
///
/// - 尺寸不一致 → 返回 Err(SizeMismatch)
/// - 忽略 Alpha，按 RGB 比对
pub fn compare_images(
    baseline: &DynamicImage,
    candidate: &DynamicImage,
    threshold_db: f64,
) -> Result<DiffMetrics, SizeMismatch> {
    let (bw, bh) = baseline.dimensions();
    let (cw, ch) = candidate.dimensions();
    // 尺寸不一致直接报错，不做重叠区裁剪。
    if (bw, bh) != (cw, ch) {
        return Err(SizeMismatch {
            baseline: (bw, bh),
            candidate: (cw, ch),
        });
    }

    let (a, w, h) = to_rgb_f64(baseline);
    let (b, _, _) = to_rgb_f64(candidate);

    // 尺寸一致后统一转为 RGB 平面再计算各项指标。
    let psnr_db = compute_psnr(&a, &b);
    let ssim = compute_ssim(&a, &b, w, h);
    let mean_abs_err = compute_mae(&a, &b);
    // 只要 PSNR 不低于阈值即判定 PASS。
    let pass = psnr_db >= threshold_db;

    Ok(DiffMetrics {
        psnr_db,
        ssim,
        mean_abs_err,
        pass,
        threshold: threshold_db,
    })
}

/// 将 f64 转为 JSON 值，安全处理非有限数：
///
/// `serde_json::Number::from_f64` 只接受有限值，对 NaN / ±∞ 返回 `None`，
/// 若直接 `unwrap()` 会 panic。此处统一约定：
/// - NaN → 字符串 `"nan"`
/// - +∞ → 字符串 `"inf"`，-∞ → 字符串 `"-inf"`
/// - 有限值 → 正常 JSON 数值
fn f64_to_json_value(v: f64) -> serde_json::Value {
    // 按优先级依次处理 NaN / 无穷 / 有限值三类，避免 serde_json 拒绝非有限数而 panic。
    if v.is_nan() {
        serde_json::Value::String("nan".to_owned())
    } else if v.is_infinite() {
        serde_json::Value::String(if v.is_sign_positive() { "inf" } else { "-inf" }.to_owned())
    } else {
        // 已排除 NaN / ±∞，有限 f64 一定能被 serde_json 表示。
        serde_json::Value::Number(
            serde_json::Number::from_f64(v).expect("finite f64 is always representable"),
        )
    }
}

/// 将 DiffMetrics 序列化为 JSON 字符串
///
/// 非有限数值安全序列化：PSNR=+∞ 输出 `"inf"`，任意字段为 NaN 输出 `"nan"`，
/// 不会因 `serde_json::Number::from_f64` 拒绝 NaN/∞ 而 panic。
pub fn metrics_to_json(m: &DiffMetrics) -> String {
    let obj = serde_json::json!({
        "psnr_db": f64_to_json_value(m.psnr_db),
        "ssim": f64_to_json_value(m.ssim),
        "mean_abs_err": f64_to_json_value(m.mean_abs_err),
        "pass": m.pass,
        "threshold": f64_to_json_value(m.threshold),
    });
    serde_json::to_string_pretty(&obj).unwrap()
}

/// 生成人类可读摘要
pub fn metrics_to_human(m: &DiffMetrics) -> String {
    // 无穷 PSNR 特化为 "inf"，否则保留 4 位小数。
    let psnr_str = if m.psnr_db.is_infinite() {
        "inf".to_owned()
    } else {
        format!("{:.4}", m.psnr_db)
    };
    let status = if m.pass { "PASS" } else { "FAIL" };
    format!(
        "[{status}] PSNR: {psnr} dB | SSIM: {ssim:.6} | MAE: {mae:.6} | threshold: {th:.1} dB",
        status = status,
        psnr = psnr_str,
        ssim = m.ssim,
        mae = m.mean_abs_err,
        th = m.threshold,
    )
}

// ─── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, RgbaImage, RgbImage};

    /// 构造一张指定尺寸的纯颜色 RGB 图（测试辅助）。
    fn make_solid_rgb(w: u32, h: u32, r: u8, g: u8, b: u8) -> DynamicImage {
        let img: RgbImage = ImageBuffer::from_pixel(w, h, Rgb([r, g, b]));
        DynamicImage::ImageRgb8(img)
    }

    /// 两图完全相同时 PSNR 应为无穷大且判定通过。
    #[test]
    fn psnr_identical_images_is_infinity() {
        let img = make_solid_rgb(16, 16, 128, 64, 32);
        let result = compare_images(&img, &img, 40.0).unwrap();
        assert!(result.psnr_db.is_infinite());
        assert!(result.pass);
        assert_eq!(result.mean_abs_err, 0.0);
    }

    /// 纯黑 vs 纯白：MSE=1.0，PSNR=0 dB，应判定失败。
    #[test]
    fn psnr_black_vs_white_is_low() {
        let black = make_solid_rgb(16, 16, 0, 0, 0);
        let white = make_solid_rgb(16, 16, 255, 255, 255);
        let result = compare_images(&black, &white, 40.0).unwrap();
        // MSE = 1.0，PSNR = 10*log10(1/1) = 0 dB
        assert!((result.psnr_db - 0.0).abs() < 1e-10);
        assert!(!result.pass);
        assert!((result.mean_abs_err - 1.0).abs() < 1e-10);
    }

    /// 已知用例：diff=0.1 → MSE=0.01 → PSNR=20 dB。
    #[test]
    fn psnr_known_value() {
        // MSE = 0.01 → PSNR = 10*log10(1/0.01) = 20 dB
        let a: Vec<f64> = vec![0.5; 100];
        let b: Vec<f64> = vec![0.4; 100]; // diff = 0.1，MSE = 0.01
        let psnr = compute_psnr(&a, &b);
        assert!((psnr - 20.0).abs() < 1e-10);
    }

    /// 已知用例：逐像素绝对差恒为 0.2 → MAE=0.2。
    #[test]
    fn mae_known_value() {
        let a: Vec<f64> = vec![0.5; 100];
        let b: Vec<f64> = vec![0.3; 100];
        let mae = compute_mae(&a, &b);
        assert!((mae - 0.2).abs() < 1e-10);
    }

    /// 相同图像的 SSIM 应为 1.0。
    #[test]
    fn ssim_identical_is_one() {
        let img = make_solid_rgb(16, 16, 100, 150, 200);
        let (a, w, h) = to_rgb_f64(&img);
        let ssim = compute_ssim(&a, &a, w, h);
        assert!((ssim - 1.0).abs() < 1e-10);
    }

    /// 纯黑 vs 纯白的均匀图像，SSIM 应非常低。
    #[test]
    fn ssim_black_vs_white_is_low() {
        let black = make_solid_rgb(16, 16, 0, 0, 0);
        let white = make_solid_rgb(16, 16, 255, 255, 255);
        let (a, w, h) = to_rgb_f64(&black);
        let (b, _, _) = to_rgb_f64(&white);
        let ssim = compute_ssim(&a, &b, w, h);
        // 纯黑 vs 纯白的均匀图，SSIM 应非常低
        assert!(ssim < 0.05);
    }

    /// 尺寸不一致时应返回携带双方尺寸的 [`SizeMismatch`] 错误。
    #[test]
    fn size_mismatch_returns_error() {
        let a = make_solid_rgb(16, 16, 0, 0, 0);
        let b = make_solid_rgb(32, 32, 0, 0, 0);
        let err = compare_images(&a, &b, 40.0).unwrap_err();
        assert_eq!(err.baseline, (16, 16));
        assert_eq!(err.candidate, (32, 32));
    }

    /// 完全相同时 PSNR=∞，JSON 应将 psnr_db 序列化为 `"inf"`。
    #[test]
    fn json_output_inf_psnr() {
        let img = make_solid_rgb(8, 8, 42, 42, 42);
        let m = compare_images(&img, &img, 40.0).unwrap();
        let json = metrics_to_json(&m);
        assert!(json.contains("\"inf\""));
        assert!(json.contains("\"pass\": true"));
    }

    /// 有限 PSNR 时 JSON 不应包含 `"inf"`，且 pass=false。
    #[test]
    fn json_output_finite_psnr() {
        let black = make_solid_rgb(8, 8, 0, 0, 0);
        let white = make_solid_rgb(8, 8, 255, 255, 255);
        let m = compare_images(&black, &white, 40.0).unwrap();
        let json = metrics_to_json(&m);
        assert!(json.contains("\"pass\": false"));
        // 不应包含 "inf"
        assert!(!json.contains("\"inf\""));
    }

    /// PSNR 恰好等于阈值时应判定为通过（边界情形）。
    #[test]
    fn threshold_boundary() {
        // PSNR 恰好等于阈值时应通过
        let a: Vec<f64> = vec![0.5; 3 * 16 * 16];
        // 构造 b，使 MSE 恰好得出 20 dB 的 PSNR
        // PSNR = 10*log10(1/MSE) = 20 → MSE = 0.01 → diff = 0.1
        let b: Vec<f64> = vec![0.4; 3 * 16 * 16];
        let psnr = compute_psnr(&a, &b);
        assert!((psnr - 20.0).abs() < 1e-10);
        // 当阈值 = 20.0 时通过
        assert!(psnr >= 20.0);
    }

    /// 小于 8×8 窗口的图像应退化为全局单窗口 SSIM。
    #[test]
    fn small_image_ssim_fallback() {
        // 4x4 图像小于 8x8 窗口 → 全局 SSIM
        let img = make_solid_rgb(4, 4, 100, 100, 100);
        let (a, w, h) = to_rgb_f64(&img);
        let ssim = compute_ssim(&a, &a, w, h);
        // 相同的图 → SSIM 应为 1.0（或极其接近）
        assert!((ssim - 1.0).abs() < 1e-6);
    }

    /// NaN 字段应被安全序列化为 `"nan"`，不得 panic。
    #[test]
    fn json_output_nan_fields_does_not_panic() {
        // DiffMetrics 字段全 pub，下游可构造 NaN；metrics_to_json 必须不 panic
        // 并将 NaN 序列化为字符串 "nan"（而非 serde_json::Number::from_f64 → None → unwrap panic）。
        let m = DiffMetrics {
            psnr_db: f64::NAN,
            ssim: f64::NAN,
            mean_abs_err: f64::NAN,
            pass: false,
            threshold: 40.0,
        };
        let json = metrics_to_json(&m);
        assert!(json.contains("\"psnr_db\": \"nan\""));
        assert!(json.contains("\"ssim\": \"nan\""));
        assert!(json.contains("\"mean_abs_err\": \"nan\""));
        // 有限的 threshold 仍应为正常数值，不受 NaN 分支影响。
        assert!(json.contains("\"threshold\": 40.0"));
    }

    /// 负无穷 PSNR 应序列化为 `"-inf"`，同样不得 panic。
    #[test]
    fn json_output_negative_infinity_psnr() {
        // 负无穷应序列化为 "-inf"，同样不得 panic。
        let m = DiffMetrics {
            psnr_db: f64::NEG_INFINITY,
            ssim: 0.0,
            mean_abs_err: 1.0,
            pass: false,
            threshold: 40.0,
        };
        let json = metrics_to_json(&m);
        assert!(json.contains("\"psnr_db\": \"-inf\""));
    }

    /// RGBA 输入应忽略 alpha，仅按 RGB 三通道比对。
    #[test]
    fn rgba_image_ignores_alpha() {
        // 构造 RGBA 图像，alpha 不同但 RGB 相同
        let mut img1 = RgbaImage::new(16, 16);
        let mut img2 = RgbaImage::new(16, 16);
        for pixel in img1.pixels_mut() {
            *pixel = image::Rgba([100, 150, 200, 255]);
        }
        for pixel in img2.pixels_mut() {
            *pixel = image::Rgba([100, 150, 200, 0]);
        }
        let d1 = DynamicImage::ImageRgba8(img1);
        let d2 = DynamicImage::ImageRgba8(img2);
        let result = compare_images(&d1, &d2, 40.0).unwrap();
        assert!(result.psnr_db.is_infinite());
        assert!(result.pass);
    }
}
