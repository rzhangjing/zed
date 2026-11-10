//! Core/TipsifySpec → Rust 集成测试
//! 13 个原始 it() 块 → 移植 4 个 A 类测试（9 个 throws = C 类，编译期安全）
//!
//! 跳过的 C 类测试：
//! - "throws when calculating ACMR (1-4 of 4)" - 编译期类型安全
//! - "throws when executing Tipsify (1-5 of 5)" - 编译期类型安全

use cesium_geospatial::tipsify::{calculate_acmr, tipsify};

#[test]
fn can_calculate_the_acmr() {
    // 由 6 个三角形、7 个顶点构成的六边形
    let indices: Vec<u32> = vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0, 5, 6, 0, 1, 6];
    let acmr = calculate_acmr(&indices, Some(6), 3);
    assert_eq!(acmr, 2.0);
}

#[test]
fn can_calculate_the_acmr_without_a_specified_maximum_index() {
    let indices: Vec<u32> = vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0, 5, 6, 0, 1, 6];
    let acmr = calculate_acmr(&indices, None, 3);
    assert_eq!(acmr, 2.0);
}

#[test]
fn can_lower_acmr_using_the_tipsify_algorithm() {
    let indices: Vec<u32> = vec![
        0, 1, 7, 1, 7, 8, 1, 2, 8, 2, 8, 9, 2, 3, 9, 3, 9, 10, 3, 4, 10, 4, 10, 11, 4, 5, 11, 5,
        11, 12, 6, 13, 14, 6, 7, 14, 7, 14, 15, 7, 8, 15, 8, 15, 16, 8, 9, 16, 9, 16, 17, 9, 10,
        17, 10, 17, 18, 10, 11, 18, 11, 18, 19, 11, 12, 19, 12, 19, 20, 13, 21, 22, 13, 14, 22, 14,
        22, 23, 14, 15, 23, 15, 23, 24, 15, 16, 24, 16, 24, 25, 16, 17, 25, 17, 25, 26, 17, 18, 26,
        18, 26, 27, 18, 19, 27, 19, 27, 28, 19, 20, 28,
    ];
    let acmr_before = calculate_acmr(&indices, Some(28), 6);
    let result = tipsify(&indices, Some(28), 6);
    let acmr_after = calculate_acmr(&result, Some(28), 6);
    assert!(
        acmr_after < acmr_before,
        "ACMR should decrease: before={}, after={}",
        acmr_before,
        acmr_after
    );
}

#[test]
fn can_tipsify_without_knowing_the_maximum_index() {
    let indices: Vec<u32> = vec![
        0, 1, 7, 1, 7, 8, 1, 2, 8, 2, 8, 9, 2, 3, 9, 3, 9, 10, 3, 4, 10, 4, 10, 11, 4, 5, 11, 5,
        11, 12, 6, 13, 14, 6, 7, 14, 7, 14, 15, 7, 8, 15, 8, 15, 16, 8, 9, 16, 9, 16, 17, 9, 10,
        17, 10, 17, 18, 10, 11, 18, 11, 18, 19, 11, 12, 19, 12, 19, 20, 13, 21, 22, 13, 14, 22, 14,
        22, 23, 14, 15, 23, 15, 23, 24, 15, 16, 24, 16, 24, 25, 16, 17, 25, 17, 25, 26, 17, 18, 26,
        18, 26, 27, 18, 19, 27, 19, 27, 28, 19, 20, 28,
    ];
    let with_max = tipsify(&indices, Some(28), 6);
    let without_max = tipsify(&indices, None, 6);
    assert_eq!(with_max, without_max);
}
