//! EncodedCartesian3 - 用于 GPU 渲染的 Cartesian3 定点编码。

use glam::DVec3;

/// 一个 Cartesian3 的定点编码，拆分为两个 Cartesian3 值（high 和 low），
/// 当它们转换为 32 位浮点并相加时，近似原始输入。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncodedCartesian3 {
    /// 每个分量的高位。
    pub high: DVec3,
    /// 每个分量的低位。
    pub low: DVec3,
}

impl Default for EncodedCartesian3 {
    /// 默认构造：高位与低位均为零向量。
    fn default() -> Self {
        Self {
            high: DVec3::ZERO,
            low: DVec3::ZERO,
        }
    }
}

/// 将一个 64 位浮点值编码为两个 f64 值（high, low），当它们
/// 转换为 32 位浮点并相加时，近似原始输入。
///
/// 映射到 `EncodedCartesian3.encode`
pub fn encode(value: f64) -> (f64, f64) {
    if value >= 0.0 {
        let double_high = (value / 65536.0).floor() * 65536.0;
        (double_high, value - double_high)
    } else {
        let double_high = (-value / 65536.0).floor() * 65536.0;
        (-double_high, value + double_high)
    }
}

/// 将 Cartesian3 编码为 EncodedCartesian3。
///
/// 映射到 `EncodedCartesian3.fromCartesian`
pub fn from_cartesian(cartesian: DVec3) -> EncodedCartesian3 {
    let (hx, lx) = encode(cartesian.x);
    let (hy, ly) = encode(cartesian.y);
    let (hz, lz) = encode(cartesian.z);
    EncodedCartesian3 {
        high: DVec3::new(hx, hy, hz),
        low: DVec3::new(lx, ly, lz),
    }
}

/// 编码一个 Cartesian3，并以
/// [high.x, high.y, high.z, low.x, low.y, low.z] 的形式从 `index` 处写入数组。
///
/// 映射到 `EncodedCartesian3.writeElements`
pub fn write_elements(cartesian: DVec3, array: &mut [f64], index: usize) {
    let encoded = from_cartesian(cartesian);
    array[index] = encoded.high.x;
    array[index + 1] = encoded.high.y;
    array[index + 2] = encoded.high.z;
    array[index + 3] = encoded.low.x;
    array[index + 4] = encoded.low.y;
    array[index + 5] = encoded.low.z;
}
