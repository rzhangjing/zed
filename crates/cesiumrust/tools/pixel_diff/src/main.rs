//! pixel_diff CLI — 像素级图像回归门工具
//!
//! ## 用法
//! ```text
//! pixel_diff <baseline.png> <candidate.png> [--threshold <dB>] [--json]
//! ```
//!
//! ## 退出码
//! - 0：PASS（PSNR ≥ 阈值）
//! - 1：FAIL（PSNR < 阈值）
//! - 2：ERROR（文件不存在、尺寸不一致、解码失败等）
//!
//! ## 约定
//! - Alpha 通道被忽略，仅比对 RGB
//! - 两图尺寸必须一致，否则报错 exit 2
//! - PSNR = Infinity 时 JSON 输出 `"inf"`

use std::process;

/// 程序入口：解析参数、加载两张图、比对并按需输出 JSON/文本，最后以 PASS/FAIL 决定退出码。
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // 无参数或请求帮助时，向 stderr 打印用法（空参视为错误 exit 2）。
    if args.is_empty() || args.contains(&"--help".to_owned()) || args.contains(&"-h".to_owned()) {
        eprintln!(
            "Usage: pixel_diff <baseline.png> <candidate.png> [--threshold <dB>] [--json]\n\
             \n\
             Compare two images pixel-by-pixel and report PSNR/SSIM/MAE.\n\
             \n\
             Options:\n\
             \x20 --threshold <dB>   PSNR pass threshold (default: 40.0 dB)\n\
             \x20 --json             Output in JSON format\n\
             \x20 -h, --help         Show this help\n\
             \n\
             Exit codes:\n\
             \x20 0  PASS (PSNR >= threshold)\n\
             \x20 1  FAIL (PSNR < threshold)\n\
             \x20 2  ERROR (invalid input, size mismatch, decode failure)"
        );
        process::exit(if args.is_empty() { 2 } else { 0 });
    }

    // 解析位置参数与标志位
    let mut positional: Vec<String> = Vec::new();
    let mut threshold: f64 = 40.0;
    let mut json_output = false;

    let mut i = 0;
    // 逐 token 扫描：--threshold 消费下一个值作参数，--json 置位，其余归为位置参数。
    while i < args.len() {
        match args[i].as_str() {
            "--threshold" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("error: --threshold requires a value");
                    process::exit(2);
                }
                threshold = match args[i].parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("error: invalid threshold value: {}", args[i]);
                        process::exit(2);
                    }
                };
            }
            "--json" => {
                json_output = true;
            }
            other => {
                positional.push(other.to_owned());
            }
        }
        i += 1;
    }

    // 位置参数必须恰为两张图（baseline + candidate），否则报错退出。
    if positional.len() != 2 {
        eprintln!("error: expected exactly 2 image paths, got {}", positional.len());
        process::exit(2);
    }

    let baseline_path = &positional[0];
    let candidate_path = &positional[1];

    // 加载图像
    let baseline = match image::open(baseline_path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: failed to load baseline '{}': {}", baseline_path, e);
            process::exit(2);
        }
    };
    let candidate = match image::open(candidate_path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: failed to load candidate '{}': {}", candidate_path, e);
            process::exit(2);
        }
    };

    // 比对
    let metrics = match pixel_diff::compare_images(&baseline, &candidate, threshold) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(2);
        }
    };

    // 输出
    // 根据 --json 选择结构化 JSON 输出或人类可读摘要。
    if json_output {
        println!("{}", pixel_diff::metrics_to_json(&metrics));
    } else {
        println!("{}", pixel_diff::metrics_to_human(&metrics));
    }

    // 退出码
    // PASS 返回 0，FAIL 返回 1，供回归门判定。
    process::exit(if metrics.pass { 0 } else { 1 });
}
