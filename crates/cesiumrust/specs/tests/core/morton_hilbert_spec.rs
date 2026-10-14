//! 参考自 `Specs/Core/MortonOrderSpec`（16 个 it()，6 个 A 类）
//! 以及 `Specs/Core/HilbertOrderSpec`（8 个 it()，2 个 A 类）
//!
//! 省略 B 类（针对 undefined/越界的 throws）测试，因为 Rust 的类型系统
//! 会在编译期强制输入合法。

use cesium_geospatial::morton_hilbert::*;

// =============================================================================
// MortonOrder
// =============================================================================

#[test]
fn morton_encode_2d_works() {
    assert_eq!(morton_encode_2d(0, 0), 0);
    assert_eq!(morton_encode_2d(1, 0), 1);
    assert_eq!(morton_encode_2d(0, 1), 2);
    assert_eq!(morton_encode_2d(1, 1), 3);
    assert_eq!(morton_encode_2d(2, 0), 4);
    assert_eq!(morton_encode_2d(3, 0), 5);
    assert_eq!(morton_encode_2d(2, 1), 6);
    assert_eq!(morton_encode_2d(3, 1), 7);

    assert_eq!(morton_encode_2d(7, 5), 55);

    // 最大的 16 位无符号整数输入 → 最大的 32 位无符号整数输出
    assert_eq!(morton_encode_2d(65535, 65535), 4294967295);
    assert_eq!(morton_encode_2d(65535, 0), 1431655765);
    assert_eq!(morton_encode_2d(0, 65535), 2863311530);
}

#[test]
fn morton_decode_2d_works() {
    assert_eq!(morton_decode_2d(0), (0, 0));
    assert_eq!(morton_decode_2d(1), (1, 0));
    assert_eq!(morton_decode_2d(2), (0, 1));
    assert_eq!(morton_decode_2d(3), (1, 1));
    assert_eq!(morton_decode_2d(4), (2, 0));
    assert_eq!(morton_decode_2d(5), (3, 0));
    assert_eq!(morton_decode_2d(6), (2, 1));
    assert_eq!(morton_decode_2d(7), (3, 1));

    assert_eq!(morton_decode_2d(55), (7, 5));

    // 最大的 32 位无符号整数输入 → 最大的 16 位无符号整数输出
    assert_eq!(morton_decode_2d(4294967295), (65535, 65535));
    assert_eq!(morton_decode_2d(1431655765), (65535, 0));
    assert_eq!(morton_decode_2d(2863311530), (0, 65535));
}

#[test]
fn morton_encode_3d_works() {
    assert_eq!(morton_encode_3d(0, 0, 0), 0);
    assert_eq!(morton_encode_3d(1, 0, 0), 1);
    assert_eq!(morton_encode_3d(0, 1, 0), 2);
    assert_eq!(morton_encode_3d(1, 1, 0), 3);
    assert_eq!(morton_encode_3d(0, 0, 1), 4);
    assert_eq!(morton_encode_3d(1, 0, 1), 5);
    assert_eq!(morton_encode_3d(0, 1, 1), 6);
    assert_eq!(morton_encode_3d(1, 1, 1), 7);

    assert_eq!(morton_encode_3d(1, 3, 3), 55);

    // 最大的 10 位无符号整数输入 → 最大的 30 位无符号整数输出
    assert_eq!(morton_encode_3d(1023, 1023, 1023), 1073741823);
    assert_eq!(morton_encode_3d(1023, 0, 0), 153391689);
    assert_eq!(morton_encode_3d(0, 1023, 0), 306783378);
    assert_eq!(morton_encode_3d(0, 0, 1023), 613566756);
}

#[test]
fn morton_decode_3d_works() {
    assert_eq!(morton_decode_3d(0), (0, 0, 0));
    assert_eq!(morton_decode_3d(1), (1, 0, 0));
    assert_eq!(morton_decode_3d(2), (0, 1, 0));
    assert_eq!(morton_decode_3d(3), (1, 1, 0));
    assert_eq!(morton_decode_3d(4), (0, 0, 1));
    assert_eq!(morton_decode_3d(5), (1, 0, 1));
    assert_eq!(morton_decode_3d(6), (0, 1, 1));
    assert_eq!(morton_decode_3d(7), (1, 1, 1));

    assert_eq!(morton_decode_3d(55), (1, 3, 3));

    // 最大的 30 位无符号整数输入 → 最大的 10 位无符号整数输出
    assert_eq!(morton_decode_3d(1073741823), (1023, 1023, 1023));
    assert_eq!(morton_decode_3d(153391689), (1023, 0, 0));
    assert_eq!(morton_decode_3d(306783378), (0, 1023, 0));
    assert_eq!(morton_decode_3d(613566756), (0, 0, 1023));
}

#[test]
fn morton_decode_2d_roundtrip() {
    // 先编码再解码应返回原始值
    let test_cases: Vec<(u32, u32)> = vec![
        (0, 0),
        (1, 0),
        (0, 1),
        (255, 128),
        (65535, 65535),
        (12345, 54321),
    ];
    for (x, y) in test_cases {
        let encoded = morton_encode_2d(x, y);
        let (dx, dy) = morton_decode_2d(encoded);
        assert_eq!((dx, dy), (x, y), "roundtrip failed for ({}, {})", x, y);
    }
}

#[test]
fn morton_decode_3d_roundtrip() {
    let test_cases: Vec<(u32, u32, u32)> = vec![
        (0, 0, 0),
        (1, 0, 0),
        (0, 1, 0),
        (0, 0, 1),
        (1023, 1023, 1023),
        (123, 456, 789),
    ];
    for (x, y, z) in test_cases {
        let encoded = morton_encode_3d(x, y, z);
        let (dx, dy, dz) = morton_decode_3d(encoded);
        assert_eq!((dx, dy, dz), (x, y, z), "roundtrip failed for ({}, {}, {})", x, y, z);
    }
}

// =============================================================================
// HilbertOrder
// =============================================================================

#[test]
fn hilbert_encode_2d_works() {
    assert_eq!(hilbert_encode_2d(1, 0, 0), 0u128);
    assert_eq!(hilbert_encode_2d(1, 0, 1), 1u128);
    assert_eq!(hilbert_encode_2d(1, 1, 1), 2u128);
    assert_eq!(hilbert_encode_2d(1, 1, 0), 3u128);

    assert_eq!(hilbert_encode_2d(2, 0, 0), 0u128);
    assert_eq!(hilbert_encode_2d(2, 1, 0), 1u128);
    assert_eq!(hilbert_encode_2d(2, 1, 1), 2u128);
    assert_eq!(hilbert_encode_2d(2, 0, 1), 3u128);
    assert_eq!(hilbert_encode_2d(2, 0, 2), 4u128);
    assert_eq!(hilbert_encode_2d(2, 0, 3), 5u128);
    assert_eq!(hilbert_encode_2d(2, 1, 3), 6u128);
    assert_eq!(hilbert_encode_2d(2, 1, 2), 7u128);
    assert_eq!(hilbert_encode_2d(2, 2, 2), 8u128);
    assert_eq!(hilbert_encode_2d(2, 2, 3), 9u128);
    assert_eq!(hilbert_encode_2d(2, 3, 3), 10u128);
    assert_eq!(hilbert_encode_2d(2, 3, 2), 11u128);
    assert_eq!(hilbert_encode_2d(2, 3, 1), 12u128);
    assert_eq!(hilbert_encode_2d(2, 2, 1), 13u128);
    assert_eq!(hilbert_encode_2d(2, 2, 0), 14u128);
    assert_eq!(hilbert_encode_2d(2, 3, 0), 15u128);
}

#[test]
fn hilbert_decode_2d_works() {
    assert_eq!(hilbert_decode_2d(1, 0u128), (0, 0));
    assert_eq!(hilbert_decode_2d(1, 1u128), (0, 1));
    assert_eq!(hilbert_decode_2d(1, 2u128), (1, 1));
    assert_eq!(hilbert_decode_2d(1, 3u128), (1, 0));

    assert_eq!(hilbert_decode_2d(2, 0u128), (0, 0));
    assert_eq!(hilbert_decode_2d(2, 1u128), (1, 0));
    assert_eq!(hilbert_decode_2d(2, 2u128), (1, 1));
    assert_eq!(hilbert_decode_2d(2, 3u128), (0, 1));
    assert_eq!(hilbert_decode_2d(2, 4u128), (0, 2));
    assert_eq!(hilbert_decode_2d(2, 5u128), (0, 3));
    assert_eq!(hilbert_decode_2d(2, 6u128), (1, 3));
    assert_eq!(hilbert_decode_2d(2, 7u128), (1, 2));
    assert_eq!(hilbert_decode_2d(2, 8u128), (2, 2));
    assert_eq!(hilbert_decode_2d(2, 9u128), (2, 3));
    assert_eq!(hilbert_decode_2d(2, 10u128), (3, 3));
    assert_eq!(hilbert_decode_2d(2, 11u128), (3, 2));
    assert_eq!(hilbert_decode_2d(2, 12u128), (3, 1));
    assert_eq!(hilbert_decode_2d(2, 13u128), (2, 1));
    assert_eq!(hilbert_decode_2d(2, 14u128), (2, 0));
    assert_eq!(hilbert_decode_2d(2, 15u128), (3, 0));
}
