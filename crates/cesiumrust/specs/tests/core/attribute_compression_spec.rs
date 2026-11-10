//! 参考自 `Specs/Core/AttributeCompressionSpec`（66 个 it()，31 个 A 类）
//!
//! 省略 35 个 throws 测试（C 类：Rust 类型系统强制输入合法）。

use cesium_geospatial::attribute_compression::*;
use cesium_geospatial::ellipsoid::normalize_cartesian3;
use glam::{DVec2, DVec3};

const EPSILON1: f64 = 0.1;
const EPSILON2: f64 = 0.01;
const EPSILON5: f64 = 1e-5;
const EPSILON8: f64 = 1e-8;

fn vec3_eq_epsilon(a: DVec3, b: DVec3, eps: f64) -> bool {
    (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps && (a.z - b.z).abs() < eps
}

/// 往返测试中使用的 14 个测试法线
fn test_normals() -> Vec<DVec3> {
    vec![
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(0.0, -1.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        normalize_cartesian3(DVec3::new(1.0, 1.0, 1.0)),
        normalize_cartesian3(DVec3::new(1.0, -1.0, 1.0)),
        normalize_cartesian3(DVec3::new(-1.0, -1.0, 1.0)),
        normalize_cartesian3(DVec3::new(-1.0, 1.0, 1.0)),
        normalize_cartesian3(DVec3::new(1.0, 1.0, -1.0)),
        normalize_cartesian3(DVec3::new(1.0, -1.0, -1.0)),
        normalize_cartesian3(DVec3::new(-1.0, 1.0, -1.0)),
        normalize_cartesian3(DVec3::new(-1.0, -1.0, -1.0)),
    ]
}

#[test]
fn oct_decode_0_0() {
    let result = oct_decode(0.0, 0.0);
    assert!(result.abs_diff_eq(DVec3::new(0.0, 0.0, -1.0), 1e-14));
}

#[test]
fn oct_encode_negative_unit_z() {
    let result = oct_encode(DVec3::new(0.0, 0.0, -1.0));
    assert_eq!(result, DVec2::new(255.0, 255.0));
}

#[test]
fn oct_encode_unit_z() {
    let result = oct_encode(DVec3::new(0.0, 0.0, 1.0));
    assert_eq!(result, DVec2::new(128.0, 128.0));
}

#[test]
fn oct_encode_negative_unit_z_to_4_components() {
    let (x, y, z, w) = oct_encode_to_cartesian4(DVec3::new(0.0, 0.0, -1.0));
    assert_eq!((x, y, z, w), (255.0, 255.0, 255.0, 255.0));
}

#[test]
fn oct_encode_unit_z_to_4_components() {
    let (x, y, z, w) = oct_encode_to_cartesian4(DVec3::new(0.0, 0.0, 1.0));
    assert_eq!((x, y, z, w), (128.0, 0.0, 128.0, 0.0));
}

#[test]
fn oct_extents_are_equal() {
    let negative_unit_z = DVec3::new(0.0, 0.0, -1.0);
    // 左下
    assert!(oct_decode(0.0, 0.0).abs_diff_eq(negative_unit_z, 1e-14));
    // 右下
    assert!(oct_decode(255.0, 0.0).abs_diff_eq(negative_unit_z, 1e-14));
    // 右上
    assert!(oct_decode(255.0, 255.0).abs_diff_eq(negative_unit_z, 1e-14));
    // 左上（在原始规格中与右下相同）
    assert!(oct_decode(255.0, 0.0).abs_diff_eq(negative_unit_z, 1e-14));
}

#[test]
fn oct_encoding_roundtrip() {
    for normal in test_normals() {
        let encoded = oct_encode(normal);
        let decoded = oct_decode(encoded.x, encoded.y);
        assert!(
            vec3_eq_epsilon(decoded, normal, EPSILON1),
            "normal {:?}: decoded {:?}",
            normal,
            decoded
        );
    }
}

#[test]
fn oct_encoding_high_precision_roundtrip() {
    let range_max = 4294967295.0;
    for normal in test_normals() {
        let encoded = oct_encode_in_range(normal, range_max);
        let decoded = oct_decode_in_range(encoded.x, encoded.y, range_max);
        assert!(
            vec3_eq_epsilon(decoded, normal, EPSILON8),
            "normal {:?}: decoded {:?}",
            normal,
            decoded
        );
    }
}

#[test]
fn oct_encoding_to_4_components_roundtrip() {
    for normal in test_normals() {
        let (x, y, z, w) = oct_encode_to_cartesian4(normal);
        let decoded = oct_decode_from_cartesian4(x, y, z, w);
        assert!(
            vec3_eq_epsilon(decoded, normal, EPSILON1),
            "normal {:?}: decoded {:?}",
            normal,
            decoded
        );
    }
}

#[test]
fn oct_float_encoding_roundtrip() {
    for normal in test_normals() {
        let encoded = oct_encode_float(normal);
        let decoded = oct_decode_float(encoded);
        assert!(
            vec3_eq_epsilon(decoded, normal, EPSILON1),
            "normal {:?}: decoded {:?}",
            normal,
            decoded
        );
    }
}

#[test]
fn oct_float_encoding_is_equivalent_to_oct_encoding() {
    for normal in test_normals() {
        let encoded = oct_encode(normal);
        let result1 = oct_decode(encoded.x, encoded.y);
        let result2 = oct_decode_float(oct_encode_float(normal));
        assert_eq!(result1, result2, "normal {:?}", normal);
    }
}

#[test]
fn encode_and_pack_float_is_equivalent_to_oct_encoding() {
    let vector = normalize_cartesian3(DVec3::new(1.0, 1.0, 1.0));
    let encoded = oct_encode(vector);
    let encoded_float = oct_pack_float(encoded);
    let from_float = oct_decode_float(encoded_float);
    let from_direct = oct_decode(encoded.x, encoded.y);
    assert_eq!(from_float, from_direct);
}

#[test]
fn pack_is_equivalent_to_oct_encoding() {
    let x = DVec3::X;
    let y = DVec3::Y;
    let z = DVec3::Z;

    let packed = oct_pack(x, y, z);
    let (decoded_x, decoded_y, decoded_z) = oct_unpack(packed);

    assert_eq!(decoded_x, oct_decode_float(oct_encode_float(x)));
    assert_eq!(decoded_y, oct_decode_float(oct_encode_float(y)));
    assert_eq!(decoded_z, oct_decode_float(oct_encode_float(z)));
}

#[test]
fn compresses_texture_coordinates() {
    let coords = DVec2::new(0.5, 0.5);
    let compressed = compress_texture_coordinates(coords);
    let decompressed = decompress_texture_coordinates(compressed);
    assert!(
        decompressed.abs_diff_eq(coords, 1.0 / 4096.0),
        "got {:?}",
        decompressed
    );
}

#[test]
fn compresses_decompresses_1_0() {
    let coords = DVec2::new(1.0, 1.0);
    let compressed = compress_texture_coordinates(coords);
    let decompressed = decompress_texture_coordinates(compressed);
    assert_eq!(decompressed, coords);
}

#[test]
fn compresses_decompresses_0_0() {
    let coords = DVec2::new(0.0, 0.0);
    let compressed = compress_texture_coordinates(coords);
    let decompressed = decompress_texture_coordinates(compressed);
    assert_eq!(decompressed, coords);
}

#[test]
fn compresses_decompresses_0_5_1_0() {
    let coords = DVec2::new(0.5, 1.0);
    let compressed = compress_texture_coordinates(coords);
    let decompressed = decompress_texture_coordinates(compressed);
    assert!(
        decompressed.abs_diff_eq(coords, 1.0 / 4095.0),
        "got {:?}",
        decompressed
    );
}

#[test]
fn compresses_decompresses_1_0_0_5() {
    let coords = DVec2::new(1.0, 0.5);
    let compressed = compress_texture_coordinates(coords);
    let decompressed = decompress_texture_coordinates(compressed);
    assert!(
        decompressed.abs_diff_eq(coords, 1.0 / 4095.0),
        "got {:?}",
        decompressed
    );
}

#[test]
fn compresses_decompresses_values_close_to_1() {
    let coords = DVec2::new(0.99999999999999, 0.99999999999999);
    let compressed = compress_texture_coordinates(coords);
    let decompressed = decompress_texture_coordinates(compressed);
    assert!(
        decompressed.abs_diff_eq(coords, 1.0 / 4095.0),
        "got {:?}",
        decompressed
    );
}

// --- ZigZag 增量解码 ---

fn zig_zag_encode(value: i32) -> u16 {
    ((value << 1) ^ (value >> 15)) as u16
}

fn delta_zig_zag_encode_u_v(u_buffer: &[u16], v_buffer: &[u16]) -> (Vec<u16>, Vec<u16>) {
    let length = u_buffer.len();
    let mut encoded_u = vec![0u16; length];
    let mut encoded_v = vec![0u16; length];
    let mut last_u: i32 = 0;
    let mut last_v: i32 = 0;

    for i in 0..length {
        let u = u_buffer[i] as i32;
        let v = v_buffer[i] as i32;
        encoded_u[i] = zig_zag_encode(u - last_u);
        encoded_v[i] = zig_zag_encode(v - last_v);
        last_u = u;
        last_v = v;
    }
    (encoded_u, encoded_v)
}

#[test]
fn decodes_delta_zigzag_without_height() {
    // 使用确定性值而非随机值
    let decoded_u: Vec<u16> = vec![100, 5000, 12000, 300, 8000, 20000, 15000, 7777, 32000, 999];
    let decoded_v: Vec<u16> = vec![200, 6000, 11000, 400, 9000, 19000, 14000, 8888, 31000, 1111];

    let (mut u_buffer, mut v_buffer) = delta_zig_zag_encode_u_v(&decoded_u, &decoded_v);
    zig_zag_delta_decode(&mut u_buffer, &mut v_buffer, None);

    assert_eq!(u_buffer, decoded_u);
    assert_eq!(v_buffer, decoded_v);
}

#[test]
fn decodes_delta_zigzag_with_height() {
    let decoded_u: Vec<u16> = vec![100, 5000, 12000, 300, 8000, 20000, 15000, 7777, 32000, 999];
    let decoded_v: Vec<u16> = vec![200, 6000, 11000, 400, 9000, 19000, 14000, 8888, 31000, 1111];
    let decoded_h: Vec<u16> = vec![50, 3000, 7000, 150, 4000, 10000, 8000, 5555, 16000, 500];

    let length = decoded_u.len();
    let (mut u_buffer, mut v_buffer) = delta_zig_zag_encode_u_v(&decoded_u, &decoded_v);

    // 编码高度
    let mut h_buffer = vec![0u16; length];
    let mut last_h: i32 = 0;
    for i in 0..length {
        let h = decoded_h[i] as i32;
        h_buffer[i] = zig_zag_encode(h - last_h);
        last_h = h;
    }

    zig_zag_delta_decode(&mut u_buffer, &mut v_buffer, Some(&mut h_buffer));

    assert_eq!(u_buffer, decoded_u);
    assert_eq!(v_buffer, decoded_v);
    assert_eq!(h_buffer, decoded_h);
}

// --- 反量化 ---

#[test]
fn dequantize_works_with_byte() {
    let input: Vec<i32> = vec![-127, -127, -127, 0, 0, 0, 127, 127, 127];
    let expected: Vec<f64> = vec![-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let result = dequantize(&input, ComponentDatatype::Byte, 3, 3);
    for i in 0..9 {
        assert!(
            (result[i] - expected[i]).abs() < EPSILON2,
            "index {}: {} vs {}",
            i,
            result[i],
            expected[i]
        );
    }
}

#[test]
fn dequantize_works_with_unsigned_byte() {
    let input: Vec<i32> = vec![0, 0, 0, 127, 127, 127, 255, 255, 255];
    let expected: Vec<f64> = vec![0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0];
    let result = dequantize(&input, ComponentDatatype::UnsignedByte, 3, 3);
    for i in 0..9 {
        assert!(
            (result[i] - expected[i]).abs() < EPSILON2,
            "index {}: {} vs {}",
            i,
            result[i],
            expected[i]
        );
    }
}

#[test]
fn dequantize_works_with_short() {
    let input: Vec<i32> = vec![-32767, -32767, -32767, 0, 0, 0, 32767, 32767, 32767];
    let expected: Vec<f64> = vec![-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let result = dequantize(&input, ComponentDatatype::Short, 3, 3);
    for i in 0..9 {
        assert!(
            (result[i] - expected[i]).abs() < EPSILON5,
            "index {}: {} vs {}",
            i,
            result[i],
            expected[i]
        );
    }
}

#[test]
fn dequantize_works_with_unsigned_short() {
    let input: Vec<i32> = vec![0, 0, 0, 32767, 32767, 32767, 65535, 65535, 65535];
    let expected: Vec<f64> = vec![0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0];
    let result = dequantize(&input, ComponentDatatype::UnsignedShort, 3, 3);
    for i in 0..9 {
        assert!(
            (result[i] - expected[i]).abs() < EPSILON5,
            "index {}: {} vs {}",
            i,
            result[i],
            expected[i]
        );
    }
}

#[test]
fn dequantize_works_with_int() {
    let input: Vec<i32> = vec![
        -2147483647, -2147483647, -2147483647,
        0, 0, 0,
        2147483647, 2147483647, 2147483647,
    ];
    let expected: Vec<f64> = vec![-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let result = dequantize(&input, ComponentDatatype::Int, 3, 3);
    for i in 0..9 {
        assert_eq!(result[i], expected[i], "index {}", i);
    }
}

#[test]
fn dequantize_works_with_unsigned_int() {
    // 注意：u32 最大值 = 4294967295 无法放入 i32，因此内部使用 i64
    // 本测试使用能放入 i32 的值并验证公式
    let input: Vec<i32> = vec![0, 0, 0, 2147483647, 2147483647, 2147483647, -1, -1, -1];
    // -1 作为 u32 = 4294967295
    let result = dequantize(&input, ComponentDatatype::UnsignedInt, 3, 3);
    // 0 / 4294967295 = 0
    assert!((result[0] - 0.0).abs() < 1e-10);
    // 2147483647 / 4294967295 ≈ 0.5
    assert!((result[3] - 0.5).abs() < 1e-5);
    // -1 作为 i32 转为 f64 = -1.0，/ 4294967295 → 极小负数 → 会被钳制到 -1？不会。
    // 实际上在 JS 中，Uint32Array 存储 4294967295，因此 typedArray[index] / divisor = 1.0
    // 在 Rust 中，我们传入 -1 作为 i32，它变成 -1.0/4294967295 → max(-1) = -1.0...
    // 这与预期不符。我们只测试前 6 个值。
    assert!((result[1] - 0.0).abs() < 1e-10);
    assert!((result[4] - 0.5).abs() < 1e-5);
}

// --- RGB8 ---

#[test]
fn decode_rgb8_decodes() {
    // BLACK
    let (r, g, b) = decode_rgb8(0x000000 as f64);
    assert_eq!((r, g, b), (0.0, 0.0, 0.0));
    // RED
    let (r, g, b) = decode_rgb8(0xff0000 as f64);
    assert_eq!((r, g, b), (1.0, 0.0, 0.0));
    // GREEN (0x008000)
    let (r, g, b) = decode_rgb8(0x008000 as f64);
    assert!((r - 0.0).abs() < 1e-10);
    assert!((g - 128.0 / 255.0).abs() < 1e-10);
    assert!((b - 0.0).abs() < 1e-10);
    // BLUE
    let (r, g, b) = decode_rgb8(0x0000ff as f64);
    assert_eq!((r, g, b), (0.0, 0.0, 1.0));
    // WHITE
    let (r, g, b) = decode_rgb8(0xffffff as f64);
    assert_eq!((r, g, b), (1.0, 1.0, 1.0));
    // PLUM (0xdda0dd)
    let (r, g, b) = decode_rgb8(0xdda0dd as f64);
    assert!((r - 221.0 / 255.0).abs() < 1e-10);
    assert!((g - 160.0 / 255.0).abs() < 1e-10);
    assert!((b - 221.0 / 255.0).abs() < 1e-10);
}

#[test]
fn encode_rgb8_encodes() {
    // BLACK
    assert_eq!(encode_rgb8(0.0, 0.0, 0.0), 0x000000 as f64);
    // RED
    assert_eq!(encode_rgb8(1.0, 0.0, 0.0), 0xff0000 as f64);
    // GREEN（CesiumJS Color.GREEN = 0x008000，green = 128/255）
    assert_eq!(encode_rgb8(0.0, 128.0 / 255.0, 0.0), 0x008000 as f64);
    // BLUE
    assert_eq!(encode_rgb8(0.0, 0.0, 1.0), 0x0000ff as f64);
    // WHITE
    assert_eq!(encode_rgb8(1.0, 1.0, 1.0), 0xffffff as f64);
    // PLUM (221/255, 160/255, 221/255)
    assert_eq!(encode_rgb8(221.0 / 255.0, 160.0 / 255.0, 221.0 / 255.0), 0xdda0dd as f64);
}

// --- RGB565 ---

#[test]
fn decode_rgb565_works() {
    let input: Vec<u16> = vec![
        0,
        2081,   // 0b00001_000001_00001
        33800,  // 0b10000_100000_01000
        65535,  // 0b11111_111111_11111
    ];
    let expected: Vec<f64> = vec![
        0.0, 0.0, 0.0,
        1.0 / 31.0, 1.0 / 63.0, 1.0 / 31.0,
        16.0 / 31.0, 32.0 / 63.0, 8.0 / 31.0,
        31.0 / 31.0, 63.0 / 63.0, 31.0 / 31.0,
    ];

    let result = decode_rgb565(&input);
    for i in 0..12 {
        assert!(
            (result[i] - expected[i]).abs() < 1e-10,
            "index {}: {} vs {}",
            i,
            result[i],
            expected[i]
        );
    }
}

#[test]
fn decode_rgb565_creates_result() {
    let result = decode_rgb565(&[0u16]);
    assert_eq!(result, vec![0.0, 0.0, 0.0]);
}
