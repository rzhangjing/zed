//! 列元数据与「声明长度单位」的判定（设计方案 §4.4、§6.4、D16）。
//!
//! ## 这个模块存在的理由
//!
//! §4.4 的实测结论很反直觉：达梦的 `SQLDescribeCol` 对 `varchar(100)` 的
//! `column_size`、`SQL_DESC_OCTET_LENGTH`、`SQL_DESC_DISPLAY_SIZE`、`SQL_DESC_PRECISION`
//! **全部报 100**，而且**没有任何一个字段标明单位是字节还是字符**。
//!
//! 也就是说「这列有多少个字符」在 ODBC 层是**不可知**的，只能靠
//! ①服务端参数 `LENGTH_IN_CHAR` + ②一次实测探测来确定。
//! [`ColumnMeta::length_in_char`] 就是承载这个判定结果的位置；
//! 判定过程由 [`interpret_length_in_char`] 完成。

use crate::datatype::{DataType, TypeCategory};
use dm_error::{DmError, DmErrorKind, Stage};

/// 一列的元数据。字段与设计方案 §6.4 完全一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    /// 列名（ODBC 返回的大小写原样；DM 不加引号时会折叠为大写）。
    pub name: String,
    /// 归一化后的类型。
    pub data_type: DataType,
    /// 是否允许 NULL（来自 `SQLDescribeCol` 的 `Nullable`）。
    pub nullable: bool,
    /// 数值类型的精度（`SQL_DESC_PRECISION`）。
    pub precision: Option<usize>,
    /// 数值类型的小数位（`SQL_DESC_SCALE`）。
    pub scale: Option<usize>,
    /// ⚠️ **声明长度，不是内容长度**；单位见 [`ColumnMeta::length_in_char`]。
    pub column_size: Option<usize>,
    /// 该列的 `column_size` 是**字符数**（`true`）还是**字节数**（`false`）。
    ///
    /// `None` = 尚未探测。服务端参数 `LENGTH_IN_CHAR` 是库级设置，
    /// 因此实际部署里所有列同值；逐列保留是为了将来驱动若按列上报时不必改结构。
    pub length_in_char: Option<bool>,
}

impl ColumnMeta {
    /// 最小构造：只有列名与类型，其余留空/`false`。
    pub fn new(name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable: true,
            precision: None,
            scale: None,
            column_size: None,
            length_in_char: None,
        }
    }

    /// 设置可空性。
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = nullable;
        self
    }

    /// 设置声明长度（原样，不做单位换算）。
    pub fn with_column_size(mut self, column_size: usize) -> Self {
        self.column_size = Some(column_size);
        self
    }

    /// 设置 `column_size` 的单位。
    pub fn with_length_in_char(mut self, length_in_char: bool) -> Self {
        self.length_in_char = Some(length_in_char);
        self
    }

    /// 设置精度与小数位。
    pub fn with_precision_scale(mut self, precision: usize, scale: usize) -> Self {
        self.precision = Some(precision);
        self.scale = Some(scale);
        self
    }

    /// 类型大类。
    pub fn category(&self) -> TypeCategory {
        self.data_type.category()
    }

    /// 声明长度**原样**返回（单位由 [`ColumnMeta::length_in_char`] 解释）。
    pub fn declared_length(&self) -> Option<usize> {
        self.column_size
    }

    /// 把声明长度解释为**字节容量**。
    ///
    /// - `length_in_char == Some(false)`：原样就是字节数（§4.4 实测本机即此）。
    /// - `length_in_char == Some(true)`：按 GB18030 最坏情况 1 字符 ≤ 4 字节放大，
    ///   保证窄路径缓冲不会因为「声明的是字符数」而不足。
    /// - `None`（未知）：按字节原样返回——**宁可少算也不放大**，因为未知时
    ///   放大 4 倍会让每个定长列的缓冲凭空变大；真正的安全做法是先探测。
    pub fn declared_bytes(&self) -> Option<usize> {
        let n = self.column_size?;
        Some(match self.length_in_char {
            Some(true) => n.saturating_mul(GB18030_MAX_BYTES_PER_CHAR),
            _ => n,
        })
    }

    /// 把声明长度解释为**字符数上界**。
    ///
    /// 单字符最少占 1 字节（ASCII），所以「声明的字节数」本身就是字符数上界；
    /// `length_in_char == Some(true)` 时声明值就是字符数。两种情况结果相同——
    /// 这个巧合是这个函数唯一的正确性来源，已在测试里钉住。
    pub fn declared_chars(&self) -> Option<usize> {
        self.column_size
    }

    /// 宽字符（`SQL_C_WCHAR`）取值缓冲需要多少个 `u16`：字符数 + 1 个 NUL。
    pub fn wide_buffer_units(&self) -> Option<usize> {
        Some(self.declared_chars()?.saturating_add(1))
    }

    /// 窄字符取值缓冲需要多少字节：字节容量 + 1 个 NUL。
    pub fn narrow_buffer_bytes(&self) -> Option<usize> {
        Some(self.declared_bytes()?.saturating_add(1))
    }

    /// 用于错误消息的类型名（`DataType::Display`）。
    pub fn type_name(&self) -> String {
        self.data_type.to_string()
    }
}

/// GB18030 单字符最大字节数（4 字节区）。
pub const GB18030_MAX_BYTES_PER_CHAR: usize = 4;

/// 一次「声明长度单位」探测的原始观测。
///
/// 探测方法（§4.4）：建一张 `varchar(n)` 的表，不断插入汉字直到报
/// `22001 / -70005 字符串截断`，记录**能插进去的最大字符数**，
/// 同时用 `lengthb()` / `length()` 取回字节数与字符数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthProbe {
    /// 列的声明长度（`SQLDescribeCol` 报的 `column_size`）。
    pub declared_column_size: usize,
    /// 实际能插入的最大**字符**数。
    pub max_chars_accepted: usize,
    /// 插满后 `lengthb()` 的值（字节数）。
    pub byte_length: usize,
    /// 插满后 `length()` 的值（字符数，应等于 `max_chars_accepted`）。
    pub char_length: usize,
}

/// 由一次探测的观测判定 `column_size` 的单位。
///
/// 返回 `Ok(true)` = 按字符计量，`Ok(false)` = 按字节计量。
///
/// 判定规则（按证据强度排序）：
/// 1. 能插入的字符数**等于**声明长度 ⇒ 声明的是字符数（`true`）。
/// 2. 字节长度**等于**声明长度且字符数**大于**声明长度是不可能的；此时若
///    字符数小于声明长度而字节数等于声明长度 ⇒ 声明的是字节数（`false`）。
/// 3. 两条都不成立 ⇒ 证据不足，返回 `Config` 错误并把三个数字带上，
///    **不猜**（猜错会让每个字符串列的缓冲要么溢出要么浪费 4 倍）。
pub fn interpret_length_in_char(p: &LengthProbe) -> Result<bool, DmError> {
    if p.char_length != p.max_chars_accepted {
        return Err(length_probe_error(format!(
            "观测自相矛盾：max_chars_accepted={} 而 char_length={}（应相等）",
            p.max_chars_accepted, p.char_length
        )));
    }
    if p.max_chars_accepted == p.declared_column_size {
        return Ok(true);
    }
    if p.byte_length == p.declared_column_size {
        return Ok(false);
    }
    Err(length_probe_error(format!(
        "证据不足：声明长度={}，能插入 {} 个字符（{} 字节）——两个数都不等于声明长度。\
         请确认插入内容确实是多字节字符，或改用更长的探测串",
        p.declared_column_size, p.max_chars_accepted, p.byte_length
    )))
}

fn length_probe_error(msg: impl Into<String>) -> DmError {
    DmError::at(DmErrorKind::Config(msg.into()), Stage::Config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::SQL_VARCHAR;

    fn varchar(name: &str, size: usize) -> ColumnMeta {
        ColumnMeta::new(name, DataType::from_odbc(SQL_VARCHAR)).with_column_size(size)
    }

    #[test]
    fn new_leaves_optional_fields_empty() {
        let c = ColumnMeta::new("A", DataType::Integer);
        assert_eq!(c.name, "A");
        assert!(c.nullable, "默认按可空处理，保守");
        assert_eq!(c.precision, None);
        assert_eq!(c.scale, None);
        assert_eq!(c.column_size, None);
        assert_eq!(c.length_in_char, None);
        assert_eq!(c.category(), TypeCategory::Integer);
    }

    #[test]
    fn builders_compose() {
        let c = ColumnMeta::new("N", DataType::Decimal)
            .with_nullable(false)
            .with_column_size(38)
            .with_precision_scale(38, 10)
            .with_length_in_char(false);
        assert!(!c.nullable);
        assert_eq!(c.declared_length(), Some(38));
        assert_eq!(c.precision, Some(38));
        assert_eq!(c.scale, Some(10));
        assert_eq!(c.length_in_char, Some(false));
    }

    /// §4.4 实测情形：本机 `LENGTH_IN_CHAR=0`，`varchar(100)` 的 100 是**字节**。
    #[test]
    fn byte_semantics_matches_the_measured_machine() {
        let c = varchar("B", 100).with_length_in_char(false);
        assert_eq!(c.declared_bytes(), Some(100));
        assert_eq!(
            c.declared_chars(),
            Some(100),
            "字符数上界仍是 100（ASCII 最省）"
        );
        assert_eq!(c.narrow_buffer_bytes(), Some(101));
        assert_eq!(c.wide_buffer_units(), Some(101));
        // 实测：100 字节 = 50 个汉字
        assert_eq!(100 / 2, 50);
    }

    /// 若探测结论是按字符：字节容量要按 GB18030 最坏情况放大。
    #[test]
    fn char_semantics_scales_up_the_byte_capacity() {
        let c = varchar("C", 100).with_length_in_char(true);
        assert_eq!(c.declared_chars(), Some(100));
        assert_eq!(c.declared_bytes(), Some(400));
        assert_eq!(c.narrow_buffer_bytes(), Some(401));
        assert_eq!(c.wide_buffer_units(), Some(101));
    }

    /// 未知单位时**不放大**——放大 4 倍会让每个定长列白白多占内存。
    #[test]
    fn unknown_unit_keeps_the_declared_number() {
        let c = varchar("U", 100);
        assert_eq!(c.length_in_char, None);
        assert_eq!(c.declared_bytes(), Some(100));
        assert_eq!(c.declared_chars(), Some(100));
    }

    #[test]
    fn no_column_size_means_no_buffers() {
        let c = ColumnMeta::new("X", DataType::Integer);
        assert_eq!(c.declared_bytes(), None);
        assert_eq!(c.declared_chars(), None);
        assert_eq!(c.narrow_buffer_bytes(), None);
        assert_eq!(c.wide_buffer_units(), None);
    }

    #[test]
    fn buffer_helpers_do_not_overflow_on_absurd_sizes() {
        let c = varchar("H", usize::MAX).with_length_in_char(true);
        assert_eq!(c.declared_bytes(), Some(usize::MAX));
        assert_eq!(
            c.wide_buffer_units(),
            Some(usize::MAX),
            "saturating，不 panic"
        );
    }

    #[test]
    fn type_name_is_the_display_form() {
        assert_eq!(varchar("V", 10).type_name(), "VARCHAR");
        assert_eq!(
            ColumnMeta::new("B", DataType::LongVarBinary).type_name(),
            "BLOB"
        );
    }

    // ---------------- 长度单位判定 ----------------

    /// 声明 100、能插 100 个汉字 ⇒ 按字符。
    #[test]
    fn probe_chars_means_length_in_char() {
        let p = LengthProbe {
            declared_column_size: 100,
            max_chars_accepted: 100,
            byte_length: 200,
            char_length: 100,
        };
        assert_eq!(interpret_length_in_char(&p).ok(), Some(true));
    }

    /// 声明 100、只能插 50 个汉字（=100 字节）⇒ 按字节（本机实测就是这一种）。
    #[test]
    fn probe_bytes_means_length_in_bytes() {
        let p = LengthProbe {
            declared_column_size: 100,
            max_chars_accepted: 50,
            byte_length: 100,
            char_length: 50,
        };
        assert_eq!(interpret_length_in_char(&p).ok(), Some(false));
    }

    /// 观测自相矛盾必须报错，不能猜。
    #[test]
    fn probe_rejects_inconsistent_char_length() {
        let p = LengthProbe {
            declared_column_size: 100,
            max_chars_accepted: 50,
            byte_length: 100,
            char_length: 49,
        };
        let e = interpret_length_in_char(&p).unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("自相矛盾")));
        assert_eq!(e.context.stage, Stage::Config);
    }

    /// 两个数都对不上 ⇒ 证据不足，报错并带上三个数字。
    #[test]
    fn probe_refuses_to_guess_on_weak_evidence() {
        let p = LengthProbe {
            declared_column_size: 100,
            max_chars_accepted: 30,
            byte_length: 90,
            char_length: 30,
        };
        let e = interpret_length_in_char(&p).unwrap_err();
        match &e.kind {
            DmErrorKind::Config(m) => {
                assert!(m.contains("证据不足"), "{m}");
                assert!(
                    m.contains("100") && m.contains("30") && m.contains("90"),
                    "{m}"
                );
            }
            other => panic!("期望 Config，实得 {other:?}"),
        }
    }
}
