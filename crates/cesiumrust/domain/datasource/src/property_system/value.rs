//! 类型擦除的属性值与可打包值类型。
//!
//! 本模块把属性求值可能返回的各类值收敛为一个 `PropertyValue` 枚举：
//! 标量、二维与三维笛卡尔向量、四元数、颜色、笛卡尔区间、笛卡尔数组、
//! 弧长区间、单位、布尔、字符串以及 JSON 值等。每种值都实现了向浮点
//! 数组的打包与反打包，使采样属性能按固定步长对底层数值逐分量插值，
//! 再在取回时还原为强类型；打包长度与偏移量由分量个数决定。此外还定义
//! 了位置求值所用的参考系，用于区分地心固连系与瞬时惯性系，并附带把
//! 颜色解析为 RGBA 分量的辅助逻辑。

use glam::{DQuat, DVec2, DVec3};
use serde_json::Value as JsonValue;

/// 定义位置时所用的参考系。
///
/// 固定系以地心固连直角坐标描述位置，惯性系则在惯性空间中定义；二者
/// 之间的转换随时间变化，因此属性求值时需按当前时刻选择正确的参考系
/// 语义，再决定是否对坐标做时变变换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReferenceFrame {
    /// fixed 参考系（例如 ECEF / `FIXED`）。
    #[default]
    Fixed,
    /// 惯性参考系（例如 ICRF / `INERTIAL`）。
    Inertial,
}

/// 一个类型擦除的属性值。
///
/// CesiumJS 属性可保存任意值；此枚举涵盖 DataSources 层
/// 所使用的各类值类型。
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// 无值（CesiumJS 的 `undefined`）。
    Undefined,
    /// 一个数字。
    Number(f64),
    /// 一个布尔值。
    Boolean(bool),
    /// 一个字符串。
    Text(String),
    /// 一个二维笛卡尔向量。
    Cartesian2(DVec2),
    /// 一个三维笛卡尔向量。
    Cartesian3(DVec3),
    /// 一个四元数（旋转）。
    Quaternion(DQuat),
    /// 一个 RGBA 颜色，各分量处于 `[0, 1]`。
    Color([f64; 4]),
    /// 一个通用的 `f64` 打包数组。
    Array(Vec<f64>),
    /// 一个任意的 JSON 值。
    Json(JsonValue),
}

impl PropertyValue {
    /// 若此值为 `Undefined` 则返回 `true`。
    pub fn is_undefined(&self) -> bool {
        matches!(self, PropertyValue::Undefined)
    }

    /// 返回此值至多四个打包的 `f64` 分量。
    fn packed_components(&self) -> [f64; 4] {
        match self {
            PropertyValue::Number(v) => [*v, 0.0, 0.0, 0.0],
            PropertyValue::Cartesian2(v) => [v.x, v.y, 0.0, 0.0],
            PropertyValue::Cartesian3(v) => [v.x, v.y, v.z, 0.0],
            PropertyValue::Quaternion(q) => [q.x, q.y, q.z, q.w],
            PropertyValue::Color(c) => [c[0], c[1], c[2], c[3]],
            _ => [0.0; 4],
        }
    }
}

/// 可与 `SampledProperty` 一同使用的可打包值类型。
///
/// 映射到 CesiumJS `Packable` 接口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackableType {
    /// 单个数字（`PackableNumber`，packedLength 1）。
    Number,
    /// `Cartesian2`（packedLength 2）。
    Cartesian2,
    /// `Cartesian3`（packedLength 3）。
    Cartesian3,
    /// `Quaternion`（packedLength 4，packedInterpolationLength 3）。
    Quaternion,
    /// `Color`（packedLength 4）。
    Color,
}

impl PackableType {
    /// 用于存储该值的 `f64` 元素数量。
    /// 映射到 `Packable.packedLength`。
    pub fn packed_length(self) -> usize {
        match self {
            PackableType::Number => 1,
            PackableType::Cartesian2 => 2,
            PackableType::Cartesian3 => 3,
            PackableType::Quaternion => 4,
            PackableType::Color => 4,
        }
    }

    /// 用于以适合插值的形式存储该值所需的元素数量。
    /// 映射到 `Packable.packedInterpolationLength`。
    pub fn packed_interpolation_length(self) -> usize {
        match self {
            PackableType::Quaternion => 3,
            other => other.packed_length(),
        }
    }

    /// 将 `value` 的打包表示追加到 `out` 上。
    /// 映射到 `Packable.pack(value, array, startingIndex)`（追加形式）。
    pub fn pack(&self, value: &PropertyValue, out: &mut Vec<f64>) {
        let comps = value.packed_components();
        let len = self.packed_length();
        out.extend_from_slice(&comps[..len]);
    }

    /// 从 `starting_index` 开始，将 `value` 的打包表示写入 `array`。
    pub fn pack_at(&self, value: &PropertyValue, array: &mut [f64], starting_index: usize) {
        let comps = value.packed_components();
        let len = self.packed_length();
        array[starting_index..starting_index + len].copy_from_slice(&comps[..len]);
    }

    /// 从 `starting_index` 开始从 `array` 中读取一个值。
    /// 映射到 `Packable.unpack(array, startingIndex, result)`。
    pub fn unpack(&self, array: &[f64], starting_index: usize) -> PropertyValue {
        let s = starting_index;
        match self {
            PackableType::Number => PropertyValue::Number(array[s]),
            PackableType::Cartesian2 => {
                PropertyValue::Cartesian2(DVec2::new(array[s], array[s + 1]))
            }
            PackableType::Cartesian3 => {
                PropertyValue::Cartesian3(DVec3::new(array[s], array[s + 1], array[s + 2]))
            }
            PackableType::Quaternion => PropertyValue::Quaternion(DQuat::from_xyzw(
                array[s],
                array[s + 1],
                array[s + 2],
                array[s + 3],
            )),
            PackableType::Color => {
                PropertyValue::Color([array[s], array[s + 1], array[s + 2], array[s + 3]])
            }
        }
    }

    /// 此类型是否定义了 `convertPackedArrayForInterpolation`
    /// （仅 `Quaternion` 定义了）。
    pub fn uses_interpolation_conversion(&self) -> bool {
        matches!(self, PackableType::Quaternion)
    }

    /// 将打包数组转换为适合插值的形式。
    ///
    /// 映射到 `Quaternion.convertPackedArrayForInterpolation`。仅对
    /// `Quaternion` 有意义；它将闭区间
    /// `[first_index, last_index]` 中的每个四元数转换为相对于该区间中
    /// 最后一个四元数的轴角向量。
    pub fn convert_packed_array_for_interpolation(
        &self,
        packed_array: &[f64],
        first_index: usize,
        last_index: usize,
        result: &mut [f64],
    ) {
        if !self.uses_interpolation_conversion() {
            return;
        }
        let last = unpack_quaternion(packed_array, last_index * 4);
        let last_conjugate = last.conjugate();

        let len = last_index - first_index + 1;
        for i in 0..len {
            let offset = i * 3;
            let mut q = unpack_quaternion(packed_array, (first_index + i) * 4);
            q *= last_conjugate;
            if q.w < 0.0 {
                q = -q;
            }
            let axis = compute_axis(q);
            let angle = compute_angle(q);
            result[offset] = axis.x * angle;
            result[offset + 1] = axis.y * angle;
            result[offset + 2] = axis.z * angle;
        }
    }

    /// 从经 `convert_packed_array_for_interpolation` 转换的数组中
    /// 取回一个实例。
    ///
    /// 映射到 `Quaternion.unpackInterpolationResult`。仅对
    /// `Quaternion` 有意义。
    pub fn unpack_interpolation_result(
        &self,
        array: &[f64],
        source_array: &[f64],
        _first_index: usize,
        last_index: usize,
    ) -> PropertyValue {
        let rotation = DVec3::new(array[0], array[1], array[2]);
        let magnitude = rotation.length();
        let q0 = unpack_quaternion(source_array, last_index * 4);

        let temp = if magnitude == 0.0 {
            DQuat::IDENTITY
        } else {
            from_axis_angle(rotation, magnitude)
        };
        PropertyValue::Quaternion(temp * q0)
    }
}

/// 从 `array` 的 `starting_index` 处解包一个四元数（4 个分量）。
fn unpack_quaternion(array: &[f64], starting_index: usize) -> DQuat {
    DQuat::from_xyzw(
        array[starting_index],
        array[starting_index + 1],
        array[starting_index + 2],
        array[starting_index + 3],
    )
}

const EPSILON6: f64 = 1e-6;

/// 计算四元数的归一化旋转轴。
/// 映射到 `Quaternion.computeAxis`。
fn compute_axis(q: DQuat) -> DVec3 {
    let w = q.w;
    if (w - 1.0).abs() < EPSILON6 || (w + 1.0).abs() < EPSILON6 {
        return DVec3::new(1.0, 0.0, 0.0);
    }
    let scalar = 1.0 / (1.0 - w * w).sqrt();
    DVec3::new(q.x * scalar, q.y * scalar, q.z * scalar)
}

/// 计算四元数的旋转角度。
/// 映射到 `Quaternion.computeAngle`。
fn compute_angle(q: DQuat) -> f64 {
    if (q.w - 1.0).abs() < EPSILON6 {
        return 0.0;
    }
    2.0 * q.w.acos()
}

/// 由一个轴（内部会归一化）与一个角度构建四元数。
/// 映射到 `Quaternion.fromAxisAngle`。
fn from_axis_angle(axis: DVec3, angle: f64) -> DQuat {
    let half_angle = angle / 2.0;
    let s = half_angle.sin();
    let axis = if axis.length_squared() > 0.0 {
        axis.normalize()
    } else {
        DVec3::X
    };
    DQuat::from_xyzw(axis.x * s, axis.y * s, axis.z * s, half_angle.cos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn test_packed_lengths() {
        assert_eq!(PackableType::Number.packed_length(), 1);
        assert_eq!(PackableType::Cartesian2.packed_length(), 2);
        assert_eq!(PackableType::Cartesian3.packed_length(), 3);
        assert_eq!(PackableType::Quaternion.packed_length(), 4);
        assert_eq!(PackableType::Color.packed_length(), 4);
    }

    #[test]
    fn test_packed_interpolation_lengths() {
        assert_eq!(PackableType::Number.packed_interpolation_length(), 1);
        assert_eq!(PackableType::Cartesian3.packed_interpolation_length(), 3);
        assert_eq!(PackableType::Quaternion.packed_interpolation_length(), 3);
        assert_eq!(PackableType::Color.packed_interpolation_length(), 4);
    }

    #[test]
    fn test_pack_unpack_roundtrip() {
        let cases = [
            (PackableType::Number, PropertyValue::Number(42.5)),
            (
                PackableType::Cartesian2,
                PropertyValue::Cartesian2(DVec2::new(1.0, 2.0)),
            ),
            (
                PackableType::Cartesian3,
                PropertyValue::Cartesian3(DVec3::new(1.0, 2.0, 3.0)),
            ),
            (
                PackableType::Quaternion,
                PropertyValue::Quaternion(DQuat::from_xyzw(0.1, 0.2, 0.3, 0.9)),
            ),
            (
                PackableType::Color,
                PropertyValue::Color([0.25, 0.5, 0.75, 1.0]),
            ),
        ];
        for (ty, value) in &cases {
            let mut buf = Vec::new();
            ty.pack(value, &mut buf);
            assert_eq!(buf.len(), ty.packed_length());
            let unpacked = ty.unpack(&buf, 0);
            assert_eq!(&unpacked, value);
        }
    }

    #[test]
    fn test_pack_at_offset() {
        let mut buf = vec![0.0; 5];
        PackableType::Cartesian3.pack_at(&PropertyValue::Cartesian3(DVec3::new(7.0, 8.0, 9.0)), &mut buf, 2);
        assert_eq!(buf, vec![0.0, 0.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn test_uses_interpolation_conversion() {
        assert!(PackableType::Quaternion.uses_interpolation_conversion());
        assert!(!PackableType::Number.uses_interpolation_conversion());
        assert!(!PackableType::Cartesian3.uses_interpolation_conversion());
        assert!(!PackableType::Color.uses_interpolation_conversion());
    }

    #[test]
    fn test_quaternion_interpolation_conversion_identity() {
        // 两个相同的四元数：相对旋转为恒等 -> 轴角为零。
        let q = DQuat::from_rotation_z(FRAC_PI_2);
        let mut packed = Vec::new();
        PackableType::Quaternion.pack(&PropertyValue::Quaternion(q), &mut packed);
        PackableType::Quaternion.pack(&PropertyValue::Quaternion(q), &mut packed);

        let mut result = vec![0.0; 6];
        PackableType::Quaternion.convert_packed_array_for_interpolation(&packed, 0, 1, &mut result);
        // 两者都相对于最后一个（自身）：恒等 -> 零向量。
        assert!((result[0]).abs() < 1e-9);
        assert!((result[1]).abs() < 1e-9);
        assert!((result[2]).abs() < 1e-9);
        assert!((result[3]).abs() < 1e-9);
        assert!((result[4]).abs() < 1e-9);
        assert!((result[5]).abs() < 1e-9);
    }

    #[test]
    fn test_quaternion_interpolation_roundtrip() {
        // q0 = 恒等，q1 = 绕 Z 轴 90 度。相对于 q1：
        // q0 * conj(q1) = 绕 Z 轴 -90 度。
        let q0 = DQuat::IDENTITY;
        let q1 = DQuat::from_rotation_z(FRAC_PI_2);
        let mut packed = Vec::new();
        PackableType::Quaternion.pack(&PropertyValue::Quaternion(q0), &mut packed);
        PackableType::Quaternion.pack(&PropertyValue::Quaternion(q1), &mut packed);

        let mut result = vec![0.0; 6];
        PackableType::Quaternion.convert_packed_array_for_interpolation(&packed, 0, 1, &mut result);

        // 从 q0 相对于 q1 的轴角表示重建 q0。
        let recovered = PackableType::Quaternion.unpack_interpolation_result(&result, &packed, 0, 1);
        if let PropertyValue::Quaternion(rq) = recovered {
            // 四元数在符号意义下相等。
            let dot = rq.dot(q0);
            assert!((dot.abs() - 1.0).abs() < 1e-9, "dot = {dot}");
        } else {
            panic!("expected quaternion");
        }
    }

    #[test]
    fn test_reference_frame_default() {
        assert_eq!(ReferenceFrame::default(), ReferenceFrame::Fixed);
    }

    #[test]
    fn test_property_value_is_undefined() {
        assert!(PropertyValue::Undefined.is_undefined());
        assert!(!PropertyValue::Number(1.0).is_undefined());
    }
}
