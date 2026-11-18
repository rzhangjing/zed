//! 值模型：[`DmValue`]（拥有所有权）与 [`ValueRef`]（借用视图），以及 §7 的**转换矩阵**。
//!
//! ## 为什么把转换放在 `dm-core`
//!
//! 转换规则（哪个方向允许、哪个方向必须拒绝）是**语义**，不是 I/O。
//! 放在这里可以让 ①`dm-odbc` 只负责把字节搬进来，②`dm-query` 只负责补列名，
//! ③规则本身能被纯单元测试覆盖（不需要 DLL、不需要数据库）。
//!
//! ## 列名从哪来
//!
//! §7 要求失败时报 `Decode { column }` / `NullViolation { column }`，
//! 但 `dm-core` 不知道列名。所以这里返回**不含列名的** [`ConvertError`]，
//! 由 `dm-query` 的 `Row::get` 调 [`ConvertError::into_dm_error`] 补上。
//! 这样「拒绝」的判定与「报告」的上下文各归其位。
//!
//! ## 三条不能违反的约定（§7）
//!
//! 1. `Null` 与非 `Option` 目标 → [`ConvertError::Null`]，**绝不返回默认值**。
//! 2. 数值 ↔ 文本可以互转（文本中介），解析失败报 [`ConvertError::Parse`]，
//!    **不静默返回 0**。
//! 3. 二进制 → 文本**直接拒绝**（[`ConvertError::Mismatch`]）——按 GB18030 硬解
//!    二进制会产出乱码并让错误在上层才暴露。

use core::fmt;

use crate::datatype::{DataType, IntervalKind, TypeCategory};
use dm_error::{DmError, DmErrorKind, Stage};

/// 达梦的 INTERVAL 值（§7 表）。
///
/// 字段布局对齐 `DPI.h` 的 `dpi_interval_t`：一个 [`IntervalKind`] 说明哪几个字段有效，
/// 一个符号位，其余字段按需使用。**不做语义校验**（例如 `YearToMonth` 时 `days` 应为 0）——
/// 校验放在构造器里，不放进数据载体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DmInterval {
    /// 子类型，决定哪些字段有效。
    pub kind: IntervalKind,
    /// `+1` 或 `-1`（对应 `dpi_interval_t::interval_sign`）。
    pub sign: i8,
    /// `YEAR` 字段。
    pub years: u32,
    /// `MONTH` 字段。
    pub months: u32,
    /// `DAY` 字段。
    pub days: u32,
    /// `HOUR` 字段。
    pub hours: u32,
    /// `MINUTE` 字段。
    pub minutes: u32,
    /// `SECOND` 字段。
    pub seconds: u32,
    /// 小数秒（纳秒，对应 `dpi_interval_t` 的 `fraction`）。
    pub nanoseconds: u32,
}

// `IntervalKind` 的 `Default` 由 derive 提供（默认变体上标 `#[default]`），
// 手写 impl 会被 clippy 的 `derivable_impls` 拦下。

impl DmInterval {
    /// 纯 `YEAR TO MONTH` 形态的便捷构造。
    pub const fn year_to_month(years: u32, months: u32, sign: i8) -> Self {
        Self {
            kind: IntervalKind::YearToMonth,
            sign,
            years,
            months,
            days: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
            nanoseconds: 0,
        }
    }

    /// 纯 `DAY TO SECOND` 形态的便捷构造。
    pub const fn day_to_second(
        days: u32,
        hours: u32,
        minutes: u32,
        seconds: u32,
        nanoseconds: u32,
        sign: i8,
    ) -> Self {
        Self {
            kind: IntervalKind::DayToSecond,
            sign,
            years: 0,
            months: 0,
            days,
            hours,
            minutes,
            seconds,
            nanoseconds,
        }
    }
}

impl fmt::Display for DmInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = if self.sign < 0 { "-" } else { "" };
        write!(f, "INTERVAL {s}")?;
        let mut wrote = false;
        let sep = |f: &mut fmt::Formatter<'_>, wrote: &mut bool, n: u32, unit: &str| {
            if n != 0 {
                if *wrote {
                    write!(f, " ")?;
                }
                write!(f, "{n} {unit}")?;
                *wrote = true;
            }
            Ok::<(), fmt::Error>(())
        };
        sep(f, &mut wrote, self.years, "YEAR")?;
        sep(f, &mut wrote, self.months, "MONTH")?;
        sep(f, &mut wrote, self.days, "DAY")?;
        sep(f, &mut wrote, self.hours, "HOUR")?;
        sep(f, &mut wrote, self.minutes, "MINUTE")?;
        sep(f, &mut wrote, self.seconds, "SECOND")?;
        if self.nanoseconds != 0 {
            if wrote {
                write!(f, " ")?;
            }
            write!(f, "{}.{:09} SECOND", self.seconds, self.nanoseconds)?;
        }
        if !wrote {
            write!(f, "0")?;
        }
        Ok(())
    }
}

/// 一个绑定参数或一个列值（拥有所有权）。
///
/// ⚠️ `PartialEq` 但对 `f32`/`f64` 不做 NaN 特殊处理（按 IEEE 语义比较），
/// 因此本类型**不是** `Eq`。
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DmValue {
    /// SQL NULL。与「空串」严格区分（§6.4）。
    Null,
    /// `BOOLEAN` / `BIT`
    Bool(bool),
    /// `TINYINT`
    TinyInt(i8),
    /// `SMALLINT`
    SmallInt(i16),
    /// `INT`
    Int(i32),
    /// `BIGINT`
    BigInt(i64),
    /// `REAL` / `FLOAT`
    Real(f32),
    /// `DOUBLE`
    Double(f64),
    /// `DECIMAL` / `NUMERIC` / `NUMBER`（需要 feature `decimal`）
    #[cfg(feature = "decimal")]
    Decimal(rust_decimal::Decimal),
    /// 文本（`CHAR` / `VARCHAR` / `CLOB` / `ROWID` / 时区类型的文本路径）
    Text(String),
    /// 二进制（`BINARY` / `VARBINARY` / `BLOB`）
    Bytes(Vec<u8>),
    /// `DATE`（需要 feature `chrono`）
    #[cfg(feature = "chrono")]
    Date(chrono::NaiveDate),
    /// `TIME`（需要 feature `chrono`）
    #[cfg(feature = "chrono")]
    Time(chrono::NaiveTime),
    /// `TIMESTAMP` / `DATETIME`（需要 feature `chrono`）
    #[cfg(feature = "chrono")]
    Timestamp(chrono::NaiveDateTime),
    /// `INTERVAL` 的 13 个子类型
    Interval(DmInterval),
    /// 未知类型码兜底：**按原始字节返回**（§7 表末行），不做任何解释。
    Unknown(Vec<u8>),
}

/// [`DmValue`] 的借用视图（零拷贝）。
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ValueRef<'a> {
    /// SQL NULL
    Null,
    /// `BOOLEAN` / `BIT`
    Bool(bool),
    /// `TINYINT`
    TinyInt(i8),
    /// `SMALLINT`
    SmallInt(i16),
    /// `INT`
    Int(i32),
    /// `BIGINT`
    BigInt(i64),
    /// `REAL` / `FLOAT`
    Real(f32),
    /// `DOUBLE`
    Double(f64),
    /// `DECIMAL` / `NUMERIC`（需要 feature `decimal`）
    #[cfg(feature = "decimal")]
    Decimal(rust_decimal::Decimal),
    /// 文本
    Text(&'a str),
    /// 二进制
    Bytes(&'a [u8]),
    /// `DATE`（需要 feature `chrono`）
    #[cfg(feature = "chrono")]
    Date(chrono::NaiveDate),
    /// `TIME`（需要 feature `chrono`）
    #[cfg(feature = "chrono")]
    Time(chrono::NaiveTime),
    /// `TIMESTAMP`（需要 feature `chrono`）
    #[cfg(feature = "chrono")]
    Timestamp(chrono::NaiveDateTime),
    /// `INTERVAL`
    Interval(DmInterval),
    /// 未知类型码的原始字节
    Unknown(&'a [u8]),
}

/// 转换失败的原因（**不含列名**，列名由 `dm-query` 补）。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConvertError {
    /// 需要非 NULL，实得 NULL。
    Null,
    /// 该方向根本不允许（如 二进制 → 文本）。
    Mismatch {
        /// 目标大类。
        expect: TypeCategory,
        /// 实际值类型。
        got: DataType,
    },
    /// 允许经文本中介，但解析失败。
    Parse {
        /// 源类型。
        from: DataType,
        /// 目标 Rust 类型名。
        to: &'static str,
        /// 失败原因（来自 `ParseIntError` 等）。
        reason: String,
    },
}

impl ConvertError {
    /// 补上列名，变成上层看到的 [`DmError`]。
    ///
    /// 转换失败一律发生在「把已取回的值解释成 Rust 类型」这一步，所以
    /// `Stage` 取 [`Stage::Fetch`]；绑定方向的编码错误由 `dm-odbc` 用
    /// [`Stage::Bind`] 自行构造。
    pub fn into_dm_error(self, column: Option<&str>) -> DmError {
        // §7 的示例写作 `DataTypeMismatch { expect: "Text", got: "Blob" }`：
        // expect 取大类名，got 取具体类型名（这里得到 "BLOB"，比 "Blob" 更贴合
        // `DataType::Display`；如需逐字对齐文档请改这一个表达式，别改结构）。
        let kind = match self {
            ConvertError::Null => DmErrorKind::NullViolation {
                column: column.unwrap_or("<unnamed>").to_string(),
            },
            ConvertError::Mismatch { expect, got } => DmErrorKind::DataTypeMismatch {
                expect: expect.as_str().to_string(),
                got: got.to_string(),
            },
            ConvertError::Parse { .. } => DmErrorKind::Decode {
                column: column.map(str::to_string),
            },
        };
        DmError::at(kind, Stage::Fetch)
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::Null => write!(f, "值为 NULL，但目标类型不是 Option"),
            ConvertError::Mismatch { expect, got } => {
                write!(f, "类型不允许：需要 {expect}，实得 {got}")
            }
            ConvertError::Parse { from, to, reason } => {
                write!(f, "无法把 {from} 解析成 {to}: {reason}")
            }
        }
    }
}

impl core::error::Error for ConvertError {}

// ---------------------------------------------------------------- 构造

macro_rules! from_int {
    ($($t:ty => $v:ident),* $(,)?) => {
        $(
            impl From<$t> for DmValue {
                fn from(x: $t) -> Self { DmValue::$v(x) }
            }
        )*
    };
}

from_int!(i8 => TinyInt, i16 => SmallInt, i32 => Int, i64 => BigInt, bool => Bool);

impl From<f32> for DmValue {
    fn from(x: f32) -> Self {
        DmValue::Real(x)
    }
}

impl From<f64> for DmValue {
    fn from(x: f64) -> Self {
        DmValue::Double(x)
    }
}

impl From<String> for DmValue {
    fn from(x: String) -> Self {
        DmValue::Text(x)
    }
}

impl From<&str> for DmValue {
    fn from(x: &str) -> Self {
        DmValue::Text(x.to_string())
    }
}

impl From<Vec<u8>> for DmValue {
    fn from(x: Vec<u8>) -> Self {
        DmValue::Bytes(x)
    }
}

impl From<&[u8]> for DmValue {
    fn from(x: &[u8]) -> Self {
        DmValue::Bytes(x.to_vec())
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDate> for DmValue {
    fn from(x: chrono::NaiveDate) -> Self {
        DmValue::Date(x)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveTime> for DmValue {
    fn from(x: chrono::NaiveTime) -> Self {
        DmValue::Time(x)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDateTime> for DmValue {
    fn from(x: chrono::NaiveDateTime) -> Self {
        DmValue::Timestamp(x)
    }
}

#[cfg(feature = "decimal")]
impl From<rust_decimal::Decimal> for DmValue {
    fn from(x: rust_decimal::Decimal) -> Self {
        DmValue::Decimal(x)
    }
}

impl<T: Into<DmValue>> From<Option<T>> for DmValue {
    fn from(x: Option<T>) -> Self {
        match x {
            None => DmValue::Null,
            Some(v) => v.into(),
        }
    }
}

// ---------------------------------------------------------------- DmValue

impl DmValue {
    /// 是否为 SQL NULL。
    pub const fn is_null(&self) -> bool {
        matches!(self, DmValue::Null)
    }

    /// 零拷贝借用视图。
    pub fn as_value_ref(&self) -> ValueRef<'_> {
        match self {
            DmValue::Null => ValueRef::Null,
            DmValue::Bool(v) => ValueRef::Bool(*v),
            DmValue::TinyInt(v) => ValueRef::TinyInt(*v),
            DmValue::SmallInt(v) => ValueRef::SmallInt(*v),
            DmValue::Int(v) => ValueRef::Int(*v),
            DmValue::BigInt(v) => ValueRef::BigInt(*v),
            DmValue::Real(v) => ValueRef::Real(*v),
            DmValue::Double(v) => ValueRef::Double(*v),
            #[cfg(feature = "decimal")]
            DmValue::Decimal(v) => ValueRef::Decimal(*v),
            DmValue::Text(v) => ValueRef::Text(v),
            DmValue::Bytes(v) => ValueRef::Bytes(v),
            #[cfg(feature = "chrono")]
            DmValue::Date(v) => ValueRef::Date(*v),
            #[cfg(feature = "chrono")]
            DmValue::Time(v) => ValueRef::Time(*v),
            #[cfg(feature = "chrono")]
            DmValue::Timestamp(v) => ValueRef::Timestamp(*v),
            DmValue::Interval(v) => ValueRef::Interval(*v),
            DmValue::Unknown(v) => ValueRef::Unknown(v),
        }
    }

    /// 对应的 [`DataType`]。
    pub fn data_type(&self) -> DataType {
        self.as_value_ref().data_type()
    }

    /// 粗粒度大类（用于不匹配判定与错误消息）。
    pub fn category(&self) -> TypeCategory {
        self.data_type().category()
    }
}

macro_rules! delegate_conv {
    ($($name:ident -> $ret:ty),* $(,)?) => {
        impl DmValue {
            $(
                #[doc = concat!("见 [`ValueRef::", stringify!($name), "`]。")]
                pub fn $name(&self) -> Result<$ret, ConvertError> {
                    self.as_value_ref().$name()
                }
            )*
        }
    };
}

delegate_conv!(
    as_bool -> bool,
    as_i64 -> i64,
    as_i32 -> i32,
    as_f64 -> f64,
    as_bytes -> &[u8],
    as_str -> &str,
    as_text -> String,
);

#[cfg(feature = "decimal")]
impl DmValue {
    /// 见 [`ValueRef::as_decimal`]。
    pub fn as_decimal(&self) -> Result<rust_decimal::Decimal, ConvertError> {
        self.as_value_ref().as_decimal()
    }
}

#[cfg(feature = "chrono")]
impl DmValue {
    /// 见 [`ValueRef::as_date`]。
    pub fn as_date(&self) -> Result<chrono::NaiveDate, ConvertError> {
        self.as_value_ref().as_date()
    }
    /// 见 [`ValueRef::as_time`]。
    pub fn as_time(&self) -> Result<chrono::NaiveTime, ConvertError> {
        self.as_value_ref().as_time()
    }
    /// 见 [`ValueRef::as_timestamp`]。
    pub fn as_timestamp(&self) -> Result<chrono::NaiveDateTime, ConvertError> {
        self.as_value_ref().as_timestamp()
    }
}

impl DmValue {
    /// 见 [`ValueRef::as_interval`]。
    pub fn as_interval(&self) -> Result<DmInterval, ConvertError> {
        self.as_value_ref().as_interval()
    }

    /// 见 [`ValueRef::as_bigint_exact`]。
    pub fn as_bigint_exact(&self) -> Result<i64, ConvertError> {
        self.as_value_ref().as_bigint_exact()
    }
}

/// **不泄露二进制内容**：二进制一律渲染成 `<N bytes>`（§9.4 日志纪律）。
impl fmt::Display for DmValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_value_ref().fmt(f)
    }
}

// ---------------------------------------------------------------- ValueRef

impl<'a> ValueRef<'a> {
    /// 是否为 SQL NULL。
    pub const fn is_null(&self) -> bool {
        matches!(self, ValueRef::Null)
    }

    /// 对应的 [`DataType`]。
    pub fn data_type(&self) -> DataType {
        match self {
            ValueRef::Null => DataType::Unknown(-1),
            ValueRef::Bool(_) => DataType::Bit,
            ValueRef::TinyInt(_) => DataType::TinyInt,
            ValueRef::SmallInt(_) => DataType::SmallInt,
            ValueRef::Int(_) => DataType::Integer,
            ValueRef::BigInt(_) => DataType::BigInt,
            ValueRef::Real(_) => DataType::Real,
            ValueRef::Double(_) => DataType::Double,
            #[cfg(feature = "decimal")]
            ValueRef::Decimal(_) => DataType::Decimal,
            ValueRef::Text(_) => DataType::VarChar,
            ValueRef::Bytes(_) => DataType::VarBinary,
            #[cfg(feature = "chrono")]
            ValueRef::Date(_) => DataType::Date,
            #[cfg(feature = "chrono")]
            ValueRef::Time(_) => DataType::Time,
            #[cfg(feature = "chrono")]
            ValueRef::Timestamp(_) => DataType::Timestamp,
            ValueRef::Interval(v) => DataType::Interval(v.kind),
            ValueRef::Unknown(_) => DataType::Unknown(-1),
        }
    }

    /// 粗粒度大类。
    pub fn category(&self) -> TypeCategory {
        self.data_type().category()
    }

    /// 深拷贝成 [`DmValue`]。
    pub fn to_owned_value(self) -> DmValue {
        match self {
            ValueRef::Null => DmValue::Null,
            ValueRef::Bool(v) => DmValue::Bool(v),
            ValueRef::TinyInt(v) => DmValue::TinyInt(v),
            ValueRef::SmallInt(v) => DmValue::SmallInt(v),
            ValueRef::Int(v) => DmValue::Int(v),
            ValueRef::BigInt(v) => DmValue::BigInt(v),
            ValueRef::Real(v) => DmValue::Real(v),
            ValueRef::Double(v) => DmValue::Double(v),
            #[cfg(feature = "decimal")]
            ValueRef::Decimal(v) => DmValue::Decimal(v),
            ValueRef::Text(v) => DmValue::Text(v.to_string()),
            ValueRef::Bytes(v) => DmValue::Bytes(v.to_vec()),
            #[cfg(feature = "chrono")]
            ValueRef::Date(v) => DmValue::Date(v),
            #[cfg(feature = "chrono")]
            ValueRef::Time(v) => DmValue::Time(v),
            #[cfg(feature = "chrono")]
            ValueRef::Timestamp(v) => DmValue::Timestamp(v),
            ValueRef::Interval(v) => DmValue::Interval(v),
            ValueRef::Unknown(v) => DmValue::Unknown(v.to_vec()),
        }
    }

    fn mismatch(&self, expect: TypeCategory) -> ConvertError {
        ConvertError::Mismatch {
            expect,
            got: self.data_type(),
        }
    }

    fn parse_err(&self, to: &'static str, reason: impl fmt::Display) -> ConvertError {
        ConvertError::Parse {
            from: self.data_type(),
            to,
            reason: reason.to_string(),
        }
    }

    /// 取 `bool`。
    ///
    /// 允许：`Bool`；`Text` 按 `true/false/1/0/yes/no/on/off`（大小写不敏感）解析；
    /// 整数类按 `!= 0` 判定（DM 的 `BIT` 有时以整数回传）。
    /// 拒绝：二进制、小数、日期时间、区间。
    pub fn as_bool(&self) -> Result<bool, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Bool(v) => Ok(*v),
            ValueRef::TinyInt(v) => Ok(*v != 0),
            ValueRef::SmallInt(v) => Ok(*v != 0),
            ValueRef::Int(v) => Ok(*v != 0),
            ValueRef::BigInt(v) => Ok(*v != 0),
            ValueRef::Text(s) => {
                parse_bool(s).ok_or_else(|| self.parse_err("bool", "不是布尔字面量"))
            }
            other => Err(other.mismatch(TypeCategory::Boolean)),
        }
    }

    /// 取 `i64`（整数类可无损放大；`Text` 解析；`Decimal` 需为整数值）。
    pub fn as_i64(&self) -> Result<i64, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Bool(v) => Ok(i64::from(*v)),
            ValueRef::TinyInt(v) => Ok(i64::from(*v)),
            ValueRef::SmallInt(v) => Ok(i64::from(*v)),
            ValueRef::Int(v) => Ok(i64::from(*v)),
            ValueRef::BigInt(v) => Ok(*v),
            ValueRef::Text(s) => s
                .trim()
                .parse::<i64>()
                .map_err(|e| self.parse_err("i64", e)),
            #[cfg(feature = "decimal")]
            ValueRef::Decimal(d) => decimal_to_i64(*d)
                .ok_or_else(|| self.parse_err("i64", "小数部分非零或超出 i64 范围")),
            other => Err(other.mismatch(TypeCategory::Integer)),
        }
    }

    /// 取 `i32`（越界报错，**不截断**）。
    pub fn as_i32(&self) -> Result<i32, ConvertError> {
        let v = self.as_i64()?;
        i32::try_from(v).map_err(|e| self.parse_err("i32", e))
    }

    /// 取 `i64`，但**只接受整数类**（`Bool` 与 `Decimal` 走 [`ValueRef::as_i64`]）。
    ///
    /// 存在的理由：大多数集成代码只想拿到「DM 的整数列」，
    /// 不希望 `bool`/`Decimal` 静默混进来。`as_i64` 宽松，这个严格。
    pub fn as_bigint_exact(&self) -> Result<i64, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::TinyInt(v) => Ok(i64::from(*v)),
            ValueRef::SmallInt(v) => Ok(i64::from(*v)),
            ValueRef::Int(v) => Ok(i64::from(*v)),
            ValueRef::BigInt(v) => Ok(*v),
            other => Err(other.mismatch(TypeCategory::Integer)),
        }
    }

    /// 取 `f64`（整数类可精确放大到 2^53 以内；`Text` 解析）。
    pub fn as_f64(&self) -> Result<f64, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Real(v) => Ok(f64::from(*v)),
            ValueRef::Double(v) => Ok(*v),
            ValueRef::TinyInt(v) => Ok(f64::from(*v)),
            ValueRef::SmallInt(v) => Ok(f64::from(*v)),
            ValueRef::Int(v) => Ok(f64::from(*v)),
            ValueRef::BigInt(v) => Ok(*v as f64),
            ValueRef::Text(s) => s
                .trim()
                .parse::<f64>()
                .map_err(|e| self.parse_err("f64", e)),
            #[cfg(feature = "decimal")]
            ValueRef::Decimal(d) => Ok(d.to_string().parse::<f64>().unwrap_or(f64::NAN)),
            other => Err(other.mismatch(TypeCategory::Float)),
        }
    }

    /// 零拷贝取文本。**只接受本就是文本的值**；数值要文本请用 [`ValueRef::as_text`]。
    ///
    /// 二进制**拒绝**（§7：`BLOB` → `String` 必须报错，不能按 GB18030 硬解）。
    pub fn as_str(&self) -> Result<&'a str, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Text(s) => Ok(s),
            other => Err(other.mismatch(TypeCategory::Text)),
        }
    }

    /// 取文本，**允许数值/布尔/日期时间经格式化转换**（§7 第一行）。
    ///
    /// 二进制与未知码**拒绝**。
    pub fn as_text(&self) -> Result<String, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Text(s) => Ok((*s).to_string()),
            ValueRef::Bool(v) => Ok(if *v { "true" } else { "false" }.to_string()),
            ValueRef::TinyInt(v) => Ok(v.to_string()),
            ValueRef::SmallInt(v) => Ok(v.to_string()),
            ValueRef::Int(v) => Ok(v.to_string()),
            ValueRef::BigInt(v) => Ok(v.to_string()),
            ValueRef::Real(v) => Ok(v.to_string()),
            ValueRef::Double(v) => Ok(v.to_string()),
            #[cfg(feature = "decimal")]
            ValueRef::Decimal(v) => Ok(v.to_string()),
            #[cfg(feature = "chrono")]
            ValueRef::Date(v) => Ok(v.to_string()),
            #[cfg(feature = "chrono")]
            ValueRef::Time(v) => Ok(v.to_string()),
            #[cfg(feature = "chrono")]
            ValueRef::Timestamp(v) => Ok(v.to_string()),
            ValueRef::Interval(v) => Ok(v.to_string()),
            other => Err(other.mismatch(TypeCategory::Text)),
        }
    }

    /// 零拷贝取二进制。只接受二进制与未知码兜底（未知码就是「字节」）。
    pub fn as_bytes(&self) -> Result<&'a [u8], ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Bytes(b) | ValueRef::Unknown(b) => Ok(b),
            other => Err(other.mismatch(TypeCategory::Binary)),
        }
    }

    /// 取 `Decimal`。
    #[cfg(feature = "decimal")]
    pub fn as_decimal(&self) -> Result<rust_decimal::Decimal, ConvertError> {
        use rust_decimal::Decimal;
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Decimal(v) => Ok(*v),
            ValueRef::TinyInt(v) => Ok(Decimal::from(*v)),
            ValueRef::SmallInt(v) => Ok(Decimal::from(*v)),
            ValueRef::Int(v) => Ok(Decimal::from(*v)),
            ValueRef::BigInt(v) => Ok(Decimal::from(*v)),
            ValueRef::Text(s) => s
                .trim()
                .parse::<Decimal>()
                .map_err(|e| self.parse_err("Decimal", e)),
            other => Err(other.mismatch(TypeCategory::Decimal)),
        }
    }

    /// 取 `NaiveDate`（`Text` 可按 `%Y-%m-%d` 解析）。
    #[cfg(feature = "chrono")]
    pub fn as_date(&self) -> Result<chrono::NaiveDate, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Date(v) => Ok(*v),
            ValueRef::Text(s) => chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
                .map_err(|e| self.parse_err("NaiveDate", e)),
            other => Err(other.mismatch(TypeCategory::Temporal)),
        }
    }

    /// 取 `NaiveTime`（`Text` 按 `%H:%M:%S%.f` 解析）。
    #[cfg(feature = "chrono")]
    pub fn as_time(&self) -> Result<chrono::NaiveTime, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Time(v) => Ok(*v),
            ValueRef::Text(s) => {
                let t = s.trim();
                chrono::NaiveTime::parse_from_str(t, "%H:%M:%S%.f")
                    .or_else(|_| chrono::NaiveTime::parse_from_str(t, "%H:%M:%S"))
                    .map_err(|e| self.parse_err("NaiveTime", e))
            }
            other => Err(other.mismatch(TypeCategory::Temporal)),
        }
    }

    /// 取 `NaiveDateTime`（`Text` 按 `%Y-%m-%d %H:%M:%S%.f` 与 `T` 分隔两种写法解析）。
    #[cfg(feature = "chrono")]
    pub fn as_timestamp(&self) -> Result<chrono::NaiveDateTime, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Timestamp(v) => Ok(*v),
            ValueRef::Text(s) => {
                let t = s.trim();
                chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S%.f")
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f"))
                    .map_err(|e| self.parse_err("NaiveDateTime", e))
            }
            other => Err(other.mismatch(TypeCategory::Temporal)),
        }
    }

    /// 取 [`DmInterval`]。
    pub fn as_interval(&self) -> Result<DmInterval, ConvertError> {
        match self {
            ValueRef::Null => Err(ConvertError::Null),
            ValueRef::Interval(v) => Ok(*v),
            other => Err(other.mismatch(TypeCategory::Interval)),
        }
    }
}

/// **不泄露二进制内容**：二进制渲染成 `<N bytes>`。
impl fmt::Display for ValueRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueRef::Null => f.write_str("NULL"),
            ValueRef::Bool(v) => write!(f, "{v}"),
            ValueRef::TinyInt(v) => write!(f, "{v}"),
            ValueRef::SmallInt(v) => write!(f, "{v}"),
            ValueRef::Int(v) => write!(f, "{v}"),
            ValueRef::BigInt(v) => write!(f, "{v}"),
            ValueRef::Real(v) => write!(f, "{v}"),
            ValueRef::Double(v) => write!(f, "{v}"),
            #[cfg(feature = "decimal")]
            ValueRef::Decimal(v) => write!(f, "{v}"),
            ValueRef::Text(v) => write!(f, "{v}"),
            ValueRef::Bytes(b) => write!(f, "<{} bytes>", b.len()),
            #[cfg(feature = "chrono")]
            ValueRef::Date(v) => write!(f, "{v}"),
            #[cfg(feature = "chrono")]
            ValueRef::Time(v) => write!(f, "{v}"),
            #[cfg(feature = "chrono")]
            ValueRef::Timestamp(v) => write!(f, "{v}"),
            ValueRef::Interval(v) => write!(f, "{v}"),
            ValueRef::Unknown(b) => write!(f, "<unknown type, {} bytes>", b.len()),
        }
    }
}

// ---------------------------------------------------------------- 助手

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" | "y" | "on" => Some(true),
        "false" | "f" | "0" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(feature = "decimal")]
fn decimal_to_i64(d: rust_decimal::Decimal) -> Option<i64> {
    use rust_decimal::prelude::ToPrimitive;
    if !d.fract().is_zero() {
        return None;
    }
    d.to_i64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(e: ConvertError) -> String {
        e.into_dm_error(Some("COL")).to_string()
    }

    // ---------------- 基本不变量 ----------------

    /// NULL 与非 Option 目标：必须报 `NullViolation` 且带列名（§7 第四行）。
    #[test]
    fn null_never_silently_defaults() {
        let v = DmValue::Null;
        for label in [
            "as_bool", "as_i64", "as_f64", "as_str", "as_text", "as_bytes",
        ] {
            let r = match label {
                "as_bool" => v.as_bool().map(|_| ()),
                "as_i64" => v.as_i64().map(|_| ()),
                "as_f64" => v.as_f64().map(|_| ()),
                "as_str" => v.as_str().map(|_| ()),
                "as_text" => v.as_text().map(|_| ()),
                "as_bytes" => v.as_bytes().map(|_| ()),
                _ => unreachable!(),
            };
            let e = r.expect_err(label);
            assert_eq!(e, ConvertError::Null, "{label}");
            let s = err(e);
            assert!(s.contains("不接受 NULL"), "{label}: {s}");
            assert!(s.contains("COL"), "{label}: {s}");
        }
    }

    /// 无列名时也要能构造出可读错误。
    #[test]
    fn null_without_column_name_is_still_readable() {
        let e = ConvertError::Null.into_dm_error(None);
        assert!(e.to_string().contains("<unnamed>"));
    }

    /// `is_null` 只看 NULL，空串不算。
    #[test]
    fn empty_string_is_not_null() {
        let v = DmValue::Text(String::new());
        assert!(!v.is_null());
        assert_eq!(v.as_str().unwrap(), "");
    }

    // ---------------- 整数 ----------------

    #[test]
    fn integers_widen_losslessly() {
        assert_eq!(DmValue::TinyInt(-128).as_i64().unwrap(), -128);
        assert_eq!(DmValue::SmallInt(32767).as_i64().unwrap(), 32767);
        assert_eq!(DmValue::Int(-2147483648).as_i64().unwrap(), -2147483648);
        assert_eq!(DmValue::BigInt(i64::MIN).as_i64().unwrap(), i64::MIN);
    }

    /// `as_i32` 越界必须报错，**不截断**。
    #[test]
    fn as_i32_refuses_to_truncate() {
        let v = DmValue::BigInt(i64::from(i32::MAX) + 1);
        let e = v.as_i32().unwrap_err();
        assert!(matches!(e, ConvertError::Parse { to: "i32", .. }), "{e:?}");
        assert!(err(e).contains("结果解码失败"));
    }

    #[test]
    fn as_i32_accepts_boundary() {
        assert_eq!(
            DmValue::BigInt(i64::from(i32::MAX)).as_i32().unwrap(),
            i32::MAX
        );
        assert_eq!(
            DmValue::BigInt(i64::from(i32::MIN)).as_i32().unwrap(),
            i32::MIN
        );
    }

    /// 文本 → 整数：允许解析（§7 第二行），失败不静默返回 0。
    #[test]
    fn text_parses_to_integer_and_fails_loudly() {
        assert_eq!(DmValue::Text(" 42 ".into()).as_i64().unwrap(), 42);
        assert_eq!(DmValue::Text("-7".into()).as_i64().unwrap(), -7);
        let e = DmValue::Text("abc".into()).as_i64().unwrap_err();
        assert!(matches!(e, ConvertError::Parse { .. }), "{e:?}");
        assert!(err(e).contains("结果解码失败"));
        assert!(err(DmValue::Text("3.5".into()).as_i64().unwrap_err()).contains("结果解码失败"));
    }

    /// 整数 → 文本：允许（§7 第一行）。
    #[test]
    fn integer_to_text_is_allowed() {
        assert_eq!(DmValue::Int(7).as_text().unwrap(), "7");
        assert_eq!(DmValue::BigInt(-1).as_text().unwrap(), "-1");
        assert_eq!(DmValue::Bool(true).as_text().unwrap(), "true");
    }

    /// 严格版只吃整数类。
    #[test]
    fn bigint_exact_rejects_bool_and_text() {
        assert_eq!(DmValue::Int(1).as_bigint_exact().unwrap(), 1);
        let e = DmValue::Bool(true).as_bigint_exact().unwrap_err();
        assert!(matches!(e, ConvertError::Mismatch { .. }), "{e:?}");
        let e = DmValue::Text("1".into()).as_bigint_exact().unwrap_err();
        assert!(matches!(e, ConvertError::Mismatch { .. }), "{e:?}");
    }

    // ---------------- 布尔 ----------------

    #[test]
    fn bool_parses_the_usual_literals() {
        for t in ["true", "T", "1", "yes", "Y", "on", " ON "] {
            assert!(DmValue::Text(t.into()).as_bool().unwrap(), "{t}");
        }
        for f in ["false", "F", "0", "no", "N", "off"] {
            assert!(!DmValue::Text(f.into()).as_bool().unwrap(), "{f}");
        }
    }

    #[test]
    fn bool_rejects_nonsense_text() {
        let e = DmValue::Text("maybe".into()).as_bool().unwrap_err();
        assert!(matches!(e, ConvertError::Parse { to: "bool", .. }), "{e:?}");
    }

    /// 整数 → 布尔：DM 的 BIT 可能以整数回传。
    #[test]
    fn integers_convert_to_bool_by_zeroness() {
        assert!(DmValue::TinyInt(1).as_bool().unwrap());
        assert!(!DmValue::Int(0).as_bool().unwrap());
        assert!(DmValue::BigInt(-5).as_bool().unwrap());
    }

    #[test]
    fn bool_to_i64_is_zero_or_one() {
        assert_eq!(DmValue::Bool(true).as_i64().unwrap(), 1);
        assert_eq!(DmValue::Bool(false).as_i64().unwrap(), 0);
    }

    // ---------------- 浮点 ----------------

    #[test]
    fn floats_convert_both_ways() {
        assert_eq!(DmValue::Real(0.5).as_f64().unwrap(), 0.5);
        assert_eq!(DmValue::Double(-1.25).as_f64().unwrap(), -1.25);
        assert_eq!(DmValue::Int(3).as_f64().unwrap(), 3.0);
        assert_eq!(DmValue::Text(" 2.5 ".into()).as_f64().unwrap(), 2.5);
    }

    #[test]
    fn float_parse_failure_is_a_decode_error() {
        let e = DmValue::Text("x".into()).as_f64().unwrap_err();
        assert!(matches!(e, ConvertError::Parse { to: "f64", .. }), "{e:?}");
    }

    /// 浮点 → 文本：`Real` 不能当 `f64` 的精度看，但转文本必须可行。
    #[test]
    fn float_to_text_round_trips_via_parse() {
        let s = DmValue::Double(1.5).as_text().unwrap();
        assert_eq!(s.parse::<f64>().unwrap(), 1.5);
    }

    // ---------------- 文本 / 二进制：核心的「必须拒绝」 ----------------

    /// §7 第三行：二进制 → 文本**拒绝**，且错误里写明 expect/got。
    #[test]
    fn binary_to_text_is_rejected() {
        let v = DmValue::Bytes(vec![0xD6, 0xD0]);
        let e = v.as_str().unwrap_err();
        match e {
            ConvertError::Mismatch { expect, got } => {
                assert_eq!(expect, TypeCategory::Text);
                assert_eq!(got, DataType::VarBinary);
            }
            other => panic!("期望 Mismatch，实得 {other:?}"),
        }
        let s = err(v.as_str().unwrap_err());
        assert!(s.contains("类型不匹配"), "{s}");
        assert!(s.contains("Text"), "{s}");
        assert!(s.contains("VARBINARY"), "{s}");
    }

    /// 二进制也不能走 `as_text`（否则会按 GB18030 硬解出乱码）。
    #[test]
    fn binary_to_owned_text_is_rejected_too() {
        let v = DmValue::Bytes(vec![0xFF, 0xFE]);
        assert!(matches!(
            v.as_text().unwrap_err(),
            ConvertError::Mismatch { .. }
        ));
    }

    /// 未知类型码兜底：可以当字节取，不能当文本取。
    #[test]
    fn unknown_fallback_is_byte_oriented() {
        let v = DmValue::Unknown(vec![1, 2, 3]);
        assert_eq!(v.as_bytes().unwrap(), &[1, 2, 3]);
        assert!(matches!(
            v.as_str().unwrap_err(),
            ConvertError::Mismatch { .. }
        ));
        assert_eq!(v.category(), TypeCategory::Unknown);
    }

    /// 文本 → 字节被拒绝：这不是编码转换器（§8 的编码在 `dm-odbc`）。
    #[test]
    fn text_to_bytes_is_rejected() {
        let v = DmValue::Text("abc".into());
        let e = v.as_bytes().unwrap_err();
        assert!(matches!(
            e,
            ConvertError::Mismatch {
                expect: TypeCategory::Binary,
                ..
            }
        ));
    }

    #[test]
    fn bytes_round_trip_through_owned() {
        let v = DmValue::Bytes(vec![9, 8, 7]);
        assert_eq!(v.as_bytes().unwrap(), &[9, 8, 7]);
        assert_eq!(v.as_value_ref().to_owned_value(), v);
    }

    // ---------------- Decimal ----------------

    #[cfg(feature = "decimal")]
    mod decimal {
        use super::*;
        use rust_decimal::Decimal;
        use std::str::FromStr;

        #[test]
        fn decimal_accepts_integers_and_text() {
            let d = Decimal::from_str("123.456").unwrap();
            assert_eq!(DmValue::Decimal(d).as_decimal().unwrap(), d);
            assert_eq!(DmValue::Int(5).as_decimal().unwrap(), Decimal::from(5));
            assert_eq!(
                DmValue::Text("1.5".into()).as_decimal().unwrap(),
                Decimal::from_str("1.5").unwrap()
            );
        }

        #[test]
        fn decimal_from_bad_text_is_a_decode_error() {
            let e = DmValue::Text("1.2.3".into()).as_decimal().unwrap_err();
            assert!(
                matches!(e, ConvertError::Parse { to: "Decimal", .. }),
                "{e:?}"
            );
        }

        /// 小数 → i64：只有整数值才允许，且必须不截断。
        #[test]
        fn decimal_to_i64_requires_an_integral_value() {
            assert_eq!(DmValue::Decimal(Decimal::from(7)).as_i64().unwrap(), 7);
            let e = DmValue::Decimal(Decimal::from_str("7.5").unwrap())
                .as_i64()
                .unwrap_err();
            assert!(matches!(e, ConvertError::Parse { to: "i64", .. }), "{e:?}");
        }

        #[test]
        fn decimal_to_i64_refuses_overflow() {
            let big = Decimal::from_str("100000000000000000000").unwrap();
            assert!(DmValue::Decimal(big).as_i64().is_err());
        }

        #[test]
        fn decimal_to_text_is_exact() {
            let d = Decimal::from_str("0.10").unwrap();
            assert_eq!(DmValue::Decimal(d).as_text().unwrap(), "0.10");
        }

        /// 二进制 → 小数也必须拒绝。
        #[test]
        fn binary_to_decimal_is_rejected() {
            let v = DmValue::Bytes(vec![1]);
            assert!(matches!(
                v.as_decimal().unwrap_err(),
                ConvertError::Mismatch {
                    expect: TypeCategory::Decimal,
                    ..
                }
            ));
        }
    }

    // ---------------- 日期时间 ----------------

    #[cfg(feature = "chrono")]
    mod temporal {
        use super::*;
        use chrono::{NaiveDate, NaiveTime};

        #[test]
        fn date_parses_text() {
            let d = NaiveDate::from_ymd_opt(2024, 5, 27).unwrap();
            assert_eq!(DmValue::Date(d).as_date().unwrap(), d);
            assert_eq!(DmValue::Text("2024-05-27".into()).as_date().unwrap(), d);
        }

        #[test]
        fn time_parses_text_with_and_without_fraction() {
            let t = NaiveTime::from_hms_opt(1, 2, 3).unwrap();
            assert_eq!(DmValue::Time(t).as_time().unwrap(), t);
            assert_eq!(DmValue::Text("01:02:03".into()).as_time().unwrap(), t);
            assert_eq!(
                DmValue::Text("01:02:03.500".into()).as_time().unwrap(),
                NaiveTime::from_hms_milli_opt(1, 2, 3, 500).unwrap()
            );
        }

        #[test]
        fn timestamp_parses_both_separators() {
            let dt = NaiveDate::from_ymd_opt(2024, 5, 27)
                .unwrap()
                .and_hms_opt(1, 2, 3)
                .unwrap();
            assert_eq!(DmValue::Timestamp(dt).as_timestamp().unwrap(), dt);
            assert_eq!(
                DmValue::Text("2024-05-27 01:02:03".into())
                    .as_timestamp()
                    .unwrap(),
                dt
            );
            assert_eq!(
                DmValue::Text("2024-05-27T01:02:03".into())
                    .as_timestamp()
                    .unwrap(),
                dt
            );
        }

        #[test]
        fn temporal_parse_failures_are_decode_errors() {
            for s in ["not-a-date", "2024/05/27", "2024-13-01"] {
                assert!(DmValue::Text(s.into()).as_date().is_err(), "{s}");
            }
            assert!(matches!(
                DmValue::Text("x".into()).as_timestamp().unwrap_err(),
                ConvertError::Parse { .. }
            ));
        }

        /// 时区类型在 v1.0 走文本路径（§7）⇒ 文本必须能原样取出。
        #[test]
        fn timezone_types_stay_text_in_v1() {
            let v = DmValue::Text("2024-05-27 01:02:03 +08:00".into());
            assert_eq!(v.as_text().unwrap(), "2024-05-27 01:02:03 +08:00");
            // 但按无时区解析应当失败——不能悄悄丢掉偏移量。
            assert!(v.as_timestamp().is_err());
        }

        #[test]
        fn temporal_to_text_is_iso_like() {
            let d = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
            assert_eq!(DmValue::Date(d).as_text().unwrap(), "2024-01-02");
        }

        #[test]
        fn date_to_integer_is_rejected() {
            let d = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
            let e = DmValue::Date(d).as_i64().unwrap_err();
            assert!(matches!(
                e,
                ConvertError::Mismatch {
                    expect: TypeCategory::Integer,
                    ..
                }
            ));
        }
    }

    // ---------------- INTERVAL ----------------

    #[test]
    fn interval_round_trips_and_rejects_other_conversions() {
        let v = DmValue::Interval(DmInterval::year_to_month(1, 2, 1));
        assert_eq!(v.as_interval().unwrap().years, 1);
        assert_eq!(v.as_interval().unwrap().months, 2);
        assert!(matches!(
            v.as_i64().unwrap_err(),
            ConvertError::Mismatch {
                expect: TypeCategory::Integer,
                ..
            }
        ));
        assert!(matches!(
            v.as_str().unwrap_err(),
            ConvertError::Mismatch {
                expect: TypeCategory::Text,
                ..
            }
        ));
    }

    #[test]
    fn interval_display_is_readable() {
        assert_eq!(
            DmInterval::year_to_month(1, 2, 1).to_string(),
            "INTERVAL 1 YEAR 2 MONTH"
        );
        assert_eq!(
            DmInterval::day_to_second(0, 3, 4, 5, 0, -1).to_string(),
            "INTERVAL -3 HOUR 4 MINUTE 5 SECOND"
        );
        assert_eq!(DmInterval::default().to_string(), "INTERVAL 0");
    }

    // ---------------- 视图 / 所有权 / 显示 ----------------

    /// `ValueRef` 与 `DmValue` 的转换结果必须一致（同一套实现）。
    #[test]
    fn value_ref_and_owned_agree() {
        let values = vec![
            DmValue::Null,
            DmValue::Bool(false),
            DmValue::Int(-3),
            DmValue::BigInt(9),
            DmValue::Real(1.0),
            DmValue::Text("42".into()),
            DmValue::Bytes(vec![1, 2]),
            DmValue::Unknown(vec![3]),
            DmValue::Interval(DmInterval::default()),
        ];
        for v in values {
            let r = v.as_value_ref();
            assert_eq!(r.to_owned_value(), v, "往返：{v:?}");
            assert_eq!(v.data_type(), r.data_type(), "类型：{v:?}");
            assert_eq!(v.category(), r.category(), "大类：{v:?}");
            assert_eq!(v.is_null(), r.is_null());
        }
    }

    /// 日志安全：二进制内容绝不进 `Display`。
    #[test]
    fn display_never_leaks_binary_content() {
        let v = DmValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(v.to_string(), "<4 bytes>");
        assert_eq!(
            DmValue::Unknown(vec![0; 10]).to_string(),
            "<unknown type, 10 bytes>"
        );
        assert_eq!(DmValue::Null.to_string(), "NULL");
        assert_eq!(DmValue::Text("ok".into()).to_string(), "ok");
    }

    #[test]
    fn option_converts_to_null_or_value() {
        assert_eq!(DmValue::from(None::<i32>), DmValue::Null);
        assert_eq!(DmValue::from(Some(5i32)), DmValue::Int(5));
        assert_eq!(DmValue::from(Some("s")), DmValue::Text("s".into()));
    }

    #[test]
    fn from_impls_cover_the_common_rust_types() {
        assert_eq!(DmValue::from(1i8), DmValue::TinyInt(1));
        assert_eq!(DmValue::from(1i16), DmValue::SmallInt(1));
        assert_eq!(DmValue::from(1i32), DmValue::Int(1));
        assert_eq!(DmValue::from(1i64), DmValue::BigInt(1));
        assert_eq!(DmValue::from(true), DmValue::Bool(true));
        assert_eq!(DmValue::from(1.0f32), DmValue::Real(1.0));
        assert_eq!(DmValue::from(1.0f64), DmValue::Double(1.0));
        assert_eq!(DmValue::from("a"), DmValue::Text("a".into()));
        assert_eq!(DmValue::from("a".to_string()), DmValue::Text("a".into()));
        assert_eq!(DmValue::from(vec![1u8]), DmValue::Bytes(vec![1]));
        assert_eq!(DmValue::from(&[1u8][..]), DmValue::Bytes(vec![1]));
    }

    /// `ConvertError` 单独 `Display` 也要可读（日志里可能只有它）。
    #[test]
    fn convert_error_display_is_informative() {
        let e = ConvertError::Mismatch {
            expect: TypeCategory::Text,
            got: DataType::LongVarBinary,
        };
        assert_eq!(e.to_string(), "类型不允许：需要 Text，实得 BLOB");
    }
}
