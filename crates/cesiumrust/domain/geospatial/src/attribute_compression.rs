//! AttributeCompression —— 八面体编码、纹理坐标压缩、zigzag 解码。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::needless_range_loop)]
use crate::ellipsoid::normalize_cartesian3;
use crate::math_utils;
use glam::{DVec2, DVec3};

const RIGHT_SHIFT8: f64 = 1.0 / 256.0;
const LEFT_SHIFT16: f64 = 65536.0;
const LEFT_SHIFT8: f64 = 256.0;

/// 按照 'oct' 编码，将一个归一化向量编码为 [0, range_max] 范围内的 2 个 SNORM 值。
///
/// 映射到 `AttributeCompression.octEncodeInRange`
pub fn oct_encode_in_range(vector: DVec3, range_max: f64) -> DVec2 {
    let denom = vector.x.abs() + vector.y.abs() + vector.z.abs();
    let mut x = vector.x / denom;
    let mut y = vector.y / denom;

    if vector.z < 0.0 {
        let old_x = x;
        let old_y = y;
        x = (1.0 - old_y.abs()) * math_utils::sign_not_zero(old_x);
        y = (1.0 - old_x.abs()) * math_utils::sign_not_zero(old_y);
    }

    DVec2::new(
        math_utils::to_snorm(x, range_max),
        math_utils::to_snorm(y, range_max),
    )
}

/// 将一个归一化向量编码为 [0, 255] 范围内的 2 个 SNORM 值。
///
/// 映射到 `AttributeCompression.octEncode`
pub fn oct_encode(vector: DVec3) -> DVec2 {
    oct_encode_in_range(vector, 255.0)
}

/// 将一个归一化向量编码为 4 字节（Cartesian4 表示）。
/// 以 [0, 255] 范围内的 f64 值返回 (x, y, z, w)。
///
/// 映射到 `AttributeCompression.octEncodeToCartesian4`
pub fn oct_encode_to_cartesian4(vector: DVec3) -> (f64, f64, f64, f64) {
    let encoded = oct_encode_in_range(vector, 65535.0);
    let x = force_uint8(encoded.x * RIGHT_SHIFT8);
    let y = force_uint8(encoded.x);
    let z = force_uint8(encoded.y * RIGHT_SHIFT8);
    let w = force_uint8(encoded.y);
    (x, y, z, w)
}

/// 从 [0, range_max] 范围内的 'oct' 编码解码出一个单位长向量。
///
/// 映射到 `AttributeCompression.octDecodeInRange`
pub fn oct_decode_in_range(x: f64, y: f64, range_max: f64) -> DVec3 {
    let mut rx = math_utils::from_snorm(x, range_max);
    let mut ry = math_utils::from_snorm(y, range_max);
    let rz = 1.0 - (rx.abs() + ry.abs());

    if rz < 0.0 {
        let old_vx = rx;
        rx = (1.0 - ry.abs()) * math_utils::sign_not_zero(old_vx);
        ry = (1.0 - old_vx.abs()) * math_utils::sign_not_zero(ry);
    }

    normalize_cartesian3(DVec3::new(rx, ry, rz))
}

/// 从 2 字节 'oct' 编码解码出一个单位长向量。
///
/// 映射到 `AttributeCompression.octDecode`
pub fn oct_decode(x: f64, y: f64) -> DVec3 {
    oct_decode_in_range(x, y, 255.0)
}

/// 从 4 字节 'oct' 编码解码出一个单位长向量。
///
/// 映射到 `AttributeCompression.octDecodeFromCartesian4`
pub fn oct_decode_from_cartesian4(x: f64, y: f64, z: f64, w: f64) -> DVec3 {
    let x_oct16 = x * LEFT_SHIFT8 + y;
    let y_oct16 = z * LEFT_SHIFT8 + w;
    oct_decode_in_range(x_oct16, y_oct16, 65535.0)
}

/// 将一个 oct 编码的向量（2 字节）打包进单个浮点数。
///
/// 映射到 `AttributeCompression.octPackFloat`
pub fn oct_pack_float(encoded: DVec2) -> f64 {
    256.0 * encoded.x + encoded.y
}

/// 将一个归一化向量编码为单个浮点数（打包的 2 字节 oct 编码）。
///
/// 映射到 `AttributeCompression.octEncodeFloat`
pub fn oct_encode_float(vector: DVec3) -> f64 {
    let encoded = oct_encode(vector);
    oct_pack_float(encoded)
}

/// 从浮点数打包的 oct 编码解码出一个单位长向量。
///
/// 映射到 `AttributeCompression.octDecodeFloat`
pub fn oct_decode_float(value: f64) -> DVec3 {
    let temp = value / 256.0;
    let x = temp.floor();
    let y = (temp - x) * 256.0;
    oct_decode(x, y)
}

/// 将三个归一化向量编码为两个浮点数（打包的 oct 编码）。
///
/// 映射到 `AttributeCompression.octPack`
pub fn oct_pack(v1: DVec3, v2: DVec3, v3: DVec3) -> DVec2 {
    let encoded1 = oct_encode_float(v1);
    let encoded2 = oct_encode_float(v2);
    let encoded3 = oct_encode(v3);
    DVec2::new(
        LEFT_SHIFT16 * encoded3.x + encoded1,
        LEFT_SHIFT16 * encoded3.y + encoded2,
    )
}

/// 从两个打包的浮点数解码出三个单位长向量。
///
/// 映射到 `AttributeCompression.octUnpack`
pub fn oct_unpack(packed: DVec2) -> (DVec3, DVec3, DVec3) {
    let temp = packed.x / LEFT_SHIFT16;
    let x = temp.floor();
    let encoded_float1 = (temp - x) * LEFT_SHIFT16;

    let temp = packed.y / LEFT_SHIFT16;
    let y = temp.floor();
    let encoded_float2 = (temp - y) * LEFT_SHIFT16;

    let v1 = oct_decode_float(encoded_float1);
    let v2 = oct_decode_float(encoded_float2);
    let v3 = oct_decode(x, y);
    (v1, v2, v3)
}

/// 将纹理坐标压缩为单个浮点数（每个分量 12 位精度）。
///
/// 映射到 `AttributeCompression.compressTextureCoordinates`
pub fn compress_texture_coordinates(texture_coordinates: DVec2) -> f64 {
    let x = (texture_coordinates.x * 4095.0) as i64;
    let y = (texture_coordinates.y * 4095.0) as i64;
    4096.0 * x as f64 + y as f64
}

/// 从单个浮点数解压纹理坐标。
///
/// 映射到 `AttributeCompression.decompressTextureCoordinates`
pub fn decompress_texture_coordinates(compressed: f64) -> DVec2 {
    let temp = compressed / 4096.0;
    let x_zero_to_4095 = temp.floor();
    DVec2::new(
        x_zero_to_4095 / 4095.0,
        (compressed - x_zero_to_4095 * 4096.0) / 4095.0,
    )
}

/// ZigZag 解码：将无符号整数还原为有符号整数（最低位为符号位）。
fn zig_zag_decode(value: u16) -> i32 {
    let v = value as i32;
    (v >> 1) ^ -(v & 1)
}

/// 就地解码经 delta 与 ZigZag 编码的顶点。
///
/// 映射到 `AttributeCompression.zigZagDeltaDecode`
pub fn zig_zag_delta_decode(u_buffer: &mut [u16], v_buffer: &mut [u16], mut height_buffer: Option<&mut [u16]>) {
    let count = u_buffer.len();
    let mut u: i32 = 0;
    let mut v: i32 = 0;
    let mut height: i32 = 0;

    for i in 0..count {
        u += zig_zag_decode(u_buffer[i]);
        v += zig_zag_decode(v_buffer[i]);
        u_buffer[i] = u as u16;
        v_buffer[i] = v as u16;

        if let Some(ref mut hb) = height_buffer {
            height += zig_zag_decode(hb[i]);
            hb[i] = height as u16;
        }
    }
}

/// WebGL 分量数据类型。分量是内建类型，
/// 它们构成属性，属性又构成顶点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentDatatype {
    /// 8 位有符号字节 (gl.BYTE = 0x1400)
    Byte,
    /// 8 位无符号字节 (gl.UNSIGNED_BYTE = 0x1401)
    UnsignedByte,
    /// 16 位有符号短整型 (gl.SHORT = 0x1402)
    Short,
    /// 16 位无符号短整型 (gl.UNSIGNED_SHORT = 0x1403)
    UnsignedShort,
    /// 32 位有符号整型 (gl.INT = 0x1404)
    Int,
    /// 32 位无符号整型 (gl.UNSIGNED_INT = 0x1405)
    UnsignedInt,
    /// 32 位浮点 (gl.FLOAT = 0x1406)
    Float,
    /// 64 位浮点 (gl.DOUBLE = 0x140A)
    Double,
}

impl ComponentDatatype {
    /// 返回此分量数据类型的字节大小。
    ///
    /// 映射到 CesiumJS `ComponentDatatype.getSizeInBytes`
    pub fn size_in_bytes(self) -> usize {
        match self {
            Self::Byte | Self::UnsignedByte => 1,
            Self::Short | Self::UnsignedShort => 2,
            Self::Int | Self::UnsignedInt | Self::Float => 4,
            Self::Double => 8,
        }
    }

    /// 返回此分量数据类型的 WebGL 常量值。
    pub fn gl_value(self) -> u32 {
        match self {
            Self::Byte => 0x1400,
            Self::UnsignedByte => 0x1401,
            Self::Short => 0x1402,
            Self::UnsignedShort => 0x1403,
            Self::Int => 0x1404,
            Self::UnsignedInt => 0x1405,
            Self::Float => 0x1406,
            Self::Double => 0x140A,
        }
    }

    /// 校验所提供的值是否为有效的 ComponentDatatype。
    /// 在 Rust 中，任何枚举变体本质上都是有效的。
    ///
    /// 映射到 CesiumJS `ComponentDatatype.validate`
    pub fn validate(self) -> bool {
        true
    }

    /// 返回所提供名称字符串对应的 ComponentDatatype。
    ///
    /// 映射到 CesiumJS `ComponentDatatype.fromName`
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "BYTE" => Some(Self::Byte),
            "UNSIGNED_BYTE" => Some(Self::UnsignedByte),
            "SHORT" => Some(Self::Short),
            "UNSIGNED_SHORT" => Some(Self::UnsignedShort),
            "INT" => Some(Self::Int),
            "UNSIGNED_INT" => Some(Self::UnsignedInt),
            "FLOAT" => Some(Self::Float),
            "DOUBLE" => Some(Self::Double),
            _ => None,
        }
    }

    /// 从 WebGL 常量值返回 ComponentDatatype。
    pub fn from_gl_value(value: u32) -> Option<Self> {
        match value {
            0x1400 => Some(Self::Byte),
            0x1401 => Some(Self::UnsignedByte),
            0x1402 => Some(Self::Short),
            0x1403 => Some(Self::UnsignedShort),
            0x1404 => Some(Self::Int),
            0x1405 => Some(Self::UnsignedInt),
            0x1406 => Some(Self::Float),
            0x140A => Some(Self::Double),
            _ => None,
        }
    }

    /// 用于反量化的除数（仅整型）。
    pub fn divisor(self) -> f64 {
        match self {
            Self::Byte => 127.0,
            Self::UnsignedByte => 255.0,
            Self::Short => 32767.0,
            Self::UnsignedShort => 65535.0,
            Self::Int => 2147483647.0,
            Self::UnsignedInt => 4294967295.0,
            Self::Float | Self::Double => 1.0,
        }
    }
}

/// 几何索引的索引数据类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexDatatype {
    /// 8 位无符号字节 (gl.UNSIGNED_BYTE = 0x1401)
    UnsignedByte,
    /// 16 位无符号短整型 (gl.UNSIGNED_SHORT = 0x1403)
    UnsignedShort,
    /// 32 位无符号整型 (gl.UNSIGNED_INT = 0x1405)
    UnsignedInt,
}

impl IndexDatatype {
    /// 64K = 65536，即 UNSIGNED_SHORT 索引的最大顶点数。
    pub const SIXTY_FOUR_KILOBYTES: u64 = 65536;

    /// 返回此索引数据类型的字节大小。
    ///
    /// 映射到 CesiumJS `IndexDatatype.getSizeInBytes`
    pub fn size_in_bytes(self) -> usize {
        match self {
            Self::UnsignedByte => 1,
            Self::UnsignedShort => 2,
            Self::UnsignedInt => 4,
        }
    }

    /// 返回此索引数据类型的 WebGL 常量值。
    pub fn gl_value(self) -> u32 {
        match self {
            Self::UnsignedByte => 0x1401,
            Self::UnsignedShort => 0x1403,
            Self::UnsignedInt => 0x1405,
        }
    }

    /// 校验所提供的值是否为有效的 IndexDatatype。
    ///
    /// 映射到 CesiumJS `IndexDatatype.validate`
    pub fn validate(self) -> bool {
        true
    }

    /// 返回所提供名称字符串对应的 IndexDatatype。
    ///
    /// 映射到 CesiumJS `IndexDatatype.fromName`
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "UNSIGNED_BYTE" => Some(Self::UnsignedByte),
            "UNSIGNED_SHORT" => Some(Self::UnsignedShort),
            "UNSIGNED_INT" => Some(Self::UnsignedInt),
            _ => None,
        }
    }

    /// 从 WebGL 常量值返回 IndexDatatype。
    pub fn from_gl_value(value: u32) -> Option<Self> {
        match value {
            0x1401 => Some(Self::UnsignedByte),
            0x1403 => Some(Self::UnsignedShort),
            0x1405 => Some(Self::UnsignedInt),
            _ => None,
        }
    }

    /// 为给定的顶点数确定合适的 IndexDatatype。
    /// 若 numberOfVertices >= 65536，返回 UnsignedInt；否则返回 UnsignedShort。
    ///
    /// 映射到 CesiumJS `IndexDatatype.createTypedArray` 的逻辑
    pub fn for_vertex_count(number_of_vertices: u64) -> Self {
        if number_of_vertices >= Self::SIXTY_FOUR_KILOBYTES {
            Self::UnsignedInt
        } else {
            Self::UnsignedShort
        }
    }
}

/// 将一个 i32 值的类型化数组反量化为 f32（此处为 f64）。
///
/// 映射到 `AttributeCompression.dequantize`
pub fn dequantize(
    typed_array: &[i32],
    component_datatype: ComponentDatatype,
    components_per_attribute: usize,
    count: usize,
) -> Vec<f64> {
    let divisor = component_datatype.divisor();
    let mut result = vec![0.0_f64; count * components_per_attribute];

    for i in 0..count {
        for j in 0..components_per_attribute {
            let index = i * components_per_attribute + j;
            result[index] = (typed_array[index] as f64 / divisor).max(-1.0);
        }
    }
    result
}

/// 以 8 位精度将 RGB 值编码为单个浮点数（0xFFFFFF 表示）。
///
/// 映射到 `AttributeCompression.encodeRGB8`
pub fn encode_rgb8(red: f64, green: f64, blue: f64) -> f64 {
    let r = (math_utils::clamp(red * 255.0, 0.0, 255.0)).round();
    let g = (math_utils::clamp(green * 255.0, 0.0, 255.0)).round();
    let b = (math_utils::clamp(blue * 255.0, 0.0, 255.0)).round();
    r * LEFT_SHIFT16 + g * LEFT_SHIFT8 + b
}

/// 从单个浮点数解码出 8 位精度的 RGB 值。
/// 返回 [0, 1] 范围内的 (red, green, blue)。
///
/// 映射到 `AttributeCompression.decodeRGB8`
pub fn decode_rgb8(encoded: f64) -> (f64, f64, f64) {
    let encoded = encoded.floor() as i64;
    let red = ((encoded >> 16) & 255) as f64 / 255.0;
    let green = ((encoded >> 8) & 255) as f64 / 255.0;
    let blue = (encoded & 255) as f64 / 255.0;
    (red, green, blue)
}

/// 将 RGB565 编码的颜色解码为归一化的 RGB 值。
///
/// 映射到 `AttributeCompression.decodeRGB565`
pub fn decode_rgb565(typed_array: &[u16]) -> Vec<f64> {
    let count = typed_array.len();
    let mut result = vec![0.0_f64; count * 3];

    let mask5: u16 = (1 << 5) - 1;
    let mask6: u16 = (1 << 6) - 1;
    let normalize5 = 1.0 / 31.0;
    let normalize6 = 1.0 / 63.0;

    for i in 0..count {
        let value = typed_array[i];
        let red = (value >> 11) as f64;
        let green = ((value >> 5) & mask6) as f64;
        let blue = (value & mask5) as f64;

        let offset = 3 * i;
        result[offset] = red * normalize5;
        result[offset + 1] = green * normalize6;
        result[offset + 2] = blue * normalize5;
    }
    result
}

/// 将一个值强制放入 uint8 范围（模仿 JS Uint8Array 的截断）。
#[inline]
fn force_uint8(value: f64) -> f64 {
    (value as u32 & 0xFF) as f64
}
