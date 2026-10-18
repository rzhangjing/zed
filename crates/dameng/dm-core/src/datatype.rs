//! ODBC SQL 数据类型码 → [`DataType`] 的映射（设计方案 §7）。
//!
//! **本模块不依赖 `odbc-api`**（CI 用 `cargo tree` 强校验）：这里只认识整数码值，
//! 由 `dm-odbc` 负责把 `odbc_api::DataType` 翻译成 `i16` 后调用 [`DataType::from_odbc`]。
//!
//! ## 为什么要有 `Unknown(i16)`
//!
//! 码值来自驱动，**任何自研驱动都可能上报非标准码**。设计要求是「未知类型码
//! **不 panic**，按原始字节返回并 `tracing::warn!`」（§7 表末行），所以这里
//! 用一个显式变体保留原码，让上层能把它写进日志与错误消息。
//!
//! ⚠️ 达梦 ODBC 驱动**实际**上报的码值是否与标准一致，尚未实测——这正是
//! §14 `V-17` 要确认的事；在确认之前，本模块的映射是「按标准 ODBC 表实现，
//! 真机不符则改表不改接口」。

use core::fmt;

/// ODBC 的 13 个 INTERVAL 子类型（`SQL_INTERVAL_*`，码值 101…113）。
///
/// 与 `DPI.h` 的 `DPIINTERVAL` 一一对应（`DSQL_IS_YEAR = 1` … `MINUTE_TO_SECOND = 13`），
/// 因此只需要一套枚举。取值/绑定的结构体在 `dm-odbc`（§14 `V-09` 未做）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[non_exhaustive]
pub enum IntervalKind {
    /// `INTERVAL YEAR`
    Year,
    /// `INTERVAL MONTH`
    Month,
    /// `INTERVAL DAY`（`Default`）
    #[default]
    Day,
    /// `INTERVAL HOUR`
    Hour,
    /// `INTERVAL MINUTE`
    Minute,
    /// `INTERVAL SECOND`
    Second,
    /// `INTERVAL YEAR TO MONTH`
    YearToMonth,
    /// `INTERVAL DAY TO HOUR`
    DayToHour,
    /// `INTERVAL DAY TO MINUTE`
    DayToMinute,
    /// `INTERVAL DAY TO SECOND`
    DayToSecond,
    /// `INTERVAL HOUR TO MINUTE`
    HourToMinute,
    /// `INTERVAL HOUR TO SECOND`
    HourToSecond,
    /// `INTERVAL MINUTE TO SECOND`
    MinuteToSecond,
}

impl IntervalKind {
    /// ODBC 码值（`SQL_INTERVAL_YEAR = 101` … `SQL_INTERVAL_MINUTE_TO_SECOND = 113`）。
    pub const ALL: [IntervalKind; 13] = [
        IntervalKind::Year,
        IntervalKind::Month,
        IntervalKind::Day,
        IntervalKind::Hour,
        IntervalKind::Minute,
        IntervalKind::Second,
        IntervalKind::YearToMonth,
        IntervalKind::DayToHour,
        IntervalKind::DayToMinute,
        IntervalKind::DayToSecond,
        IntervalKind::HourToMinute,
        IntervalKind::HourToSecond,
        IntervalKind::MinuteToSecond,
    ];

    /// 由 ODBC 码值构造；非 101…113 返回 `None`。
    pub const fn from_odbc(code: i16) -> Option<Self> {
        Some(match code {
            101 => IntervalKind::Year,
            102 => IntervalKind::Month,
            103 => IntervalKind::Day,
            104 => IntervalKind::Hour,
            105 => IntervalKind::Minute,
            106 => IntervalKind::Second,
            107 => IntervalKind::YearToMonth,
            108 => IntervalKind::DayToHour,
            109 => IntervalKind::DayToMinute,
            110 => IntervalKind::DayToSecond,
            111 => IntervalKind::HourToMinute,
            112 => IntervalKind::HourToSecond,
            113 => IntervalKind::MinuteToSecond,
            _ => return None,
        })
    }

    /// 对应的 ODBC 码值。
    pub const fn odbc_code(self) -> i16 {
        101 + self as i16
    }

    /// 两个字段都有的子类型（`YEAR TO MONTH` 家族）月/年同时出现。
    pub const fn is_year_month(self) -> bool {
        matches!(
            self,
            IntervalKind::Year | IntervalKind::Month | IntervalKind::YearToMonth
        )
    }

    /// 涉及「天」的字段。
    pub const fn has_day(self) -> bool {
        matches!(
            self,
            IntervalKind::Day
                | IntervalKind::DayToHour
                | IntervalKind::DayToMinute
                | IntervalKind::DayToSecond
        )
    }

    /// 涉及「秒 + 小数秒」的字段。
    pub const fn has_fraction(self) -> bool {
        matches!(
            self,
            IntervalKind::Second
                | IntervalKind::DayToSecond
                | IntervalKind::HourToSecond
                | IntervalKind::MinuteToSecond
        )
    }
}

/// DM/ODBC 的列类型，已归一化为中间件自己的词汇表。
///
/// 与 §7 表一一对应；`Unknown` 兜底任何不在表内的码值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum DataType {
    /// `CHAR(n)`／`SQL_CHAR`
    Char,
    /// `VARCHAR(n)`／`VARCHAR2`／`SQL_VARCHAR`
    VarChar,
    /// `TEXT`／`CLOB`／`SQL_LONGVARCHAR`
    LongVarChar,
    /// `SQL_WCHAR`（宽字符定长，DM 少见但标准里有）
    WChar,
    /// `SQL_WVARCHAR`
    WVarChar,
    /// `SQL_WLONGVARCHAR`
    WLongVarChar,
    /// `DECIMAL`／`NUMERIC`／`NUMBER`／`SQL_NUMERIC`／`SQL_DECIMAL`
    Decimal,
    /// `FLOAT`／`SQL_FLOAT`（DM 的 `FLOAT` 是浮点，不是定点）
    Float,
    /// `REAL`／`SQL_REAL`
    Real,
    /// `DOUBLE`／`SQL_DOUBLE`
    Double,
    /// `TINYINT`／`SQL_TINYINT`
    TinyInt,
    /// `SMALLINT`／`SQL_SMALLINT`
    SmallInt,
    /// `INT`／`INTEGER`／`SQL_INTEGER`
    Integer,
    /// `BIGINT`／`SQL_BIGINT`
    BigInt,
    /// `BOOLEAN`／`SQL_BIT`（⚠️ 达梦 `BOOLEAN` 与 `BIT(n)` 都可能报 `SQL_BIT`，按声明区分）
    Bit,
    /// `BINARY(n)`／`SQL_BINARY`
    Binary,
    /// `VARBINARY(n)`／`SQL_VARBINARY`
    VarBinary,
    /// `BLOB`／`IMAGE`／`SQL_LONGVARBINARY`
    LongVarBinary,
    /// `DATE`／`SQL_TYPE_DATE`
    Date,
    /// `TIME`／`SQL_TYPE_TIME`
    Time,
    /// `TIMESTAMP`／`DATETIME`／`SQL_TYPE_TIMESTAMP`
    Timestamp,
    /// `SQL_GUID`（DM 无对应，保留以便识别）
    Guid,
    /// `INTERVAL …`（13 个子类型）
    Interval(IntervalKind),
    /// 无法识别的码值。**不 panic**，由上层 `tracing::warn!` 后按原始字节处理。
    Unknown(i16),
}

/// 类型大类。用于「类型不匹配」判定的粗粒度比较（§7 的四条行为约定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TypeCategory {
    /// 文本：`CHAR` / `VARCHAR` / `CLOB` / 宽字符三兄弟
    Text,
    /// 二进制：`BINARY` / `VARBINARY` / `BLOB`
    Binary,
    /// 整数
    Integer,
    /// 浮点
    Float,
    /// 定点（`DECIMAL`/`NUMERIC`）
    Decimal,
    /// 布尔
    Boolean,
    /// 日期时间
    Temporal,
    /// 区间
    Interval,
    /// 未识别
    Unknown,
}

impl TypeCategory {
    /// 用于错误消息的稳定名称（§7 `DataTypeMismatch { expect: "Text", got: "Blob" }`）。
    pub const fn as_str(self) -> &'static str {
        match self {
            TypeCategory::Text => "Text",
            TypeCategory::Binary => "Binary",
            TypeCategory::Integer => "Integer",
            TypeCategory::Float => "Float",
            TypeCategory::Decimal => "Decimal",
            TypeCategory::Boolean => "Boolean",
            TypeCategory::Temporal => "Temporal",
            TypeCategory::Interval => "Interval",
            TypeCategory::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for TypeCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---- 标准 ODBC SQL 类型码（本模块自持，避免依赖 odbc-sys） ----

/// `SQL_CHAR`
pub const SQL_CHAR: i16 = 1;
/// `SQL_NUMERIC`
pub const SQL_NUMERIC: i16 = 2;
/// `SQL_DECIMAL`
pub const SQL_DECIMAL: i16 = 3;
/// `SQL_INTEGER`
pub const SQL_INTEGER: i16 = 4;
/// `SQL_SMALLINT`
pub const SQL_SMALLINT: i16 = 5;
/// `SQL_FLOAT`
pub const SQL_FLOAT: i16 = 6;
/// `SQL_REAL`
pub const SQL_REAL: i16 = 7;
/// `SQL_DOUBLE`
pub const SQL_DOUBLE: i16 = 8;
/// `SQL_DATETIME`（ODBC 2.x 的统称，不该在描述列时出现）
pub const SQL_DATETIME: i16 = 9;
/// `SQL_VARCHAR`
pub const SQL_VARCHAR: i16 = 12;
/// `SQL_TYPE_DATE`
pub const SQL_TYPE_DATE: i16 = 91;
/// `SQL_TYPE_TIME`
pub const SQL_TYPE_TIME: i16 = 92;
/// `SQL_TYPE_TIMESTAMP`
pub const SQL_TYPE_TIMESTAMP: i16 = 93;
/// `SQL_LONGVARCHAR`
pub const SQL_LONGVARCHAR: i16 = -1;
/// `SQL_BINARY`
pub const SQL_BINARY: i16 = -2;
/// `SQL_VARBINARY`
pub const SQL_VARBINARY: i16 = -3;
/// `SQL_LONGVARBINARY`
pub const SQL_LONGVARBINARY: i16 = -4;
/// `SQL_BIGINT`
pub const SQL_BIGINT: i16 = -5;
/// `SQL_TINYINT`
pub const SQL_TINYINT: i16 = -6;
/// `SQL_BIT`
pub const SQL_BIT: i16 = -7;
/// `SQL_WCHAR`
pub const SQL_WCHAR: i16 = -8;
/// `SQL_WVARCHAR`
pub const SQL_WVARCHAR: i16 = -9;
/// `SQL_WLONGVARCHAR`
pub const SQL_WLONGVARCHAR: i16 = -10;
/// `SQL_GUID`
pub const SQL_GUID: i16 = -11;

impl DataType {
    /// 由 ODBC SQL 类型码构造。**永不失败**：不认识的码值变成 [`DataType::Unknown`]。
    pub const fn from_odbc(code: i16) -> Self {
        match code {
            SQL_CHAR => DataType::Char,
            SQL_VARCHAR => DataType::VarChar,
            SQL_LONGVARCHAR => DataType::LongVarChar,
            SQL_WCHAR => DataType::WChar,
            SQL_WVARCHAR => DataType::WVarChar,
            SQL_WLONGVARCHAR => DataType::WLongVarChar,
            SQL_NUMERIC | SQL_DECIMAL => DataType::Decimal,
            SQL_FLOAT => DataType::Float,
            SQL_REAL => DataType::Real,
            SQL_DOUBLE => DataType::Double,
            SQL_TINYINT => DataType::TinyInt,
            SQL_SMALLINT => DataType::SmallInt,
            SQL_INTEGER => DataType::Integer,
            SQL_BIGINT => DataType::BigInt,
            SQL_BIT => DataType::Bit,
            SQL_BINARY => DataType::Binary,
            SQL_VARBINARY => DataType::VarBinary,
            SQL_LONGVARBINARY => DataType::LongVarBinary,
            SQL_TYPE_DATE => DataType::Date,
            SQL_TYPE_TIME => DataType::Time,
            SQL_TYPE_TIMESTAMP => DataType::Timestamp,
            SQL_GUID => DataType::Guid,
            other => match IntervalKind::from_odbc(other) {
                Some(k) => DataType::Interval(k),
                None => DataType::Unknown(other),
            },
        }
    }

    /// 对应的**规范** ODBC 码值。
    ///
    /// `Unknown(code)` 原样返回 `code`，因此对未知码恒有
    /// `DataType::from_odbc(c).odbc_code() == c`。
    ///
    /// ⚠️ **唯一的例外是 `SQL_NUMERIC`(2)**：它与 `SQL_DECIMAL`(3) 归一化成
    /// [`DataType::Decimal`]，回码时取规范值 `SQL_DECIMAL`。也就是说
    /// `from_odbc(2).odbc_code() == 3`。若某处必须知道驱动**原报**的码值，
    /// 不要从这里取——那需要 `dm-odbc` 在构造 `ColumnMeta` 时另行保留。
    pub const fn odbc_code(self) -> i16 {
        match self {
            DataType::Char => SQL_CHAR,
            DataType::VarChar => SQL_VARCHAR,
            DataType::LongVarChar => SQL_LONGVARCHAR,
            DataType::WChar => SQL_WCHAR,
            DataType::WVarChar => SQL_WVARCHAR,
            DataType::WLongVarChar => SQL_WLONGVARCHAR,
            DataType::Decimal => SQL_DECIMAL,
            DataType::Float => SQL_FLOAT,
            DataType::Real => SQL_REAL,
            DataType::Double => SQL_DOUBLE,
            DataType::TinyInt => SQL_TINYINT,
            DataType::SmallInt => SQL_SMALLINT,
            DataType::Integer => SQL_INTEGER,
            DataType::BigInt => SQL_BIGINT,
            DataType::Bit => SQL_BIT,
            DataType::Binary => SQL_BINARY,
            DataType::VarBinary => SQL_VARBINARY,
            DataType::LongVarBinary => SQL_LONGVARBINARY,
            DataType::Date => SQL_TYPE_DATE,
            DataType::Time => SQL_TYPE_TIME,
            DataType::Timestamp => SQL_TYPE_TIMESTAMP,
            DataType::Guid => SQL_GUID,
            DataType::Interval(k) => k.odbc_code(),
            DataType::Unknown(code) => code,
        }
    }

    /// 粗粒度大类，供「类型不匹配」判定使用。
    pub const fn category(self) -> TypeCategory {
        match self {
            DataType::Char
            | DataType::VarChar
            | DataType::LongVarChar
            | DataType::WChar
            | DataType::WVarChar
            | DataType::WLongVarChar => TypeCategory::Text,
            DataType::Binary | DataType::VarBinary | DataType::LongVarBinary => {
                TypeCategory::Binary
            }
            DataType::TinyInt | DataType::SmallInt | DataType::Integer | DataType::BigInt => {
                TypeCategory::Integer
            }
            DataType::Real | DataType::Float | DataType::Double => TypeCategory::Float,
            DataType::Decimal => TypeCategory::Decimal,
            DataType::Bit => TypeCategory::Boolean,
            DataType::Date | DataType::Time | DataType::Timestamp => TypeCategory::Temporal,
            DataType::Interval(_) => TypeCategory::Interval,
            DataType::Guid => TypeCategory::Binary,
            DataType::Unknown(_) => TypeCategory::Unknown,
        }
    }

    /// 是否是「文本类」（可用文本路径取值）。
    pub const fn is_text(self) -> bool {
        matches!(self.category(), TypeCategory::Text)
    }

    /// 是否是二进制/LOB 类。
    pub const fn is_binary(self) -> bool {
        matches!(self.category(), TypeCategory::Binary)
    }

    /// 是否是大对象（`CLOB`/`TEXT`/`BLOB`/`IMAGE`）。
    ///
    /// §7 只把 `LongVarChar` 与 `LongVarBinary` 归为 LOB；定长 `BINARY(n)` 不是。
    pub const fn is_lob(self) -> bool {
        matches!(self, DataType::LongVarChar | DataType::LongVarBinary)
    }

    /// 是否是需要**分块**路径的类型（LOB 一律分块，见 §6.5）。
    pub const fn needs_chunked_read(self) -> bool {
        self.is_lob()
    }

    /// 是否为 `Unknown`。
    pub const fn is_unknown(self) -> bool {
        matches!(self, DataType::Unknown(_))
    }

    /// §7 表第四列「Rust 默认类型」，用于诊断消息与 Toasty 映射（§9）。
    pub const fn default_rust_type(self) -> &'static str {
        match self {
            DataType::Char
            | DataType::VarChar
            | DataType::LongVarChar
            | DataType::WChar
            | DataType::WVarChar
            | DataType::WLongVarChar => "String",
            DataType::Decimal => "Decimal",
            DataType::Float => "f32",
            DataType::Real => "f32",
            DataType::Double => "f64",
            DataType::TinyInt => "i8",
            DataType::SmallInt => "i16",
            DataType::Integer => "i32",
            DataType::BigInt => "i64",
            DataType::Bit => "bool",
            DataType::Binary | DataType::VarBinary | DataType::LongVarBinary => "Vec<u8>",
            DataType::Date => "NaiveDate",
            DataType::Time => "NaiveTime",
            DataType::Timestamp => "NaiveDateTime",
            DataType::Guid => "String",
            DataType::Interval(_) => "DmInterval",
            DataType::Unknown(_) => "Vec<u8>",
        }
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataType::Unknown(code) => write!(f, "Unknown({code})"),
            DataType::Interval(k) => write!(f, "Interval({k:?})"),
            other => f.write_str(match other {
                DataType::Char => "CHAR",
                DataType::VarChar => "VARCHAR",
                DataType::LongVarChar => "CLOB",
                DataType::WChar => "WCHAR",
                DataType::WVarChar => "WVARCHAR",
                DataType::WLongVarChar => "WLONGVARCHAR",
                DataType::Decimal => "DECIMAL",
                DataType::Float => "FLOAT",
                DataType::Real => "REAL",
                DataType::Double => "DOUBLE",
                DataType::TinyInt => "TINYINT",
                DataType::SmallInt => "SMALLINT",
                DataType::Integer => "INT",
                DataType::BigInt => "BIGINT",
                DataType::Bit => "BOOLEAN",
                DataType::Binary => "BINARY",
                DataType::VarBinary => "VARBINARY",
                DataType::LongVarBinary => "BLOB",
                DataType::Date => "DATE",
                DataType::Time => "TIME",
                DataType::Timestamp => "TIMESTAMP",
                DataType::Guid => "GUID",
                DataType::Interval(_) | DataType::Unknown(_) => unreachable!(),
            }),
        }
    }
}

impl fmt::Display for IntervalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IntervalKind::Year => "INTERVAL YEAR",
            IntervalKind::Month => "INTERVAL MONTH",
            IntervalKind::Day => "INTERVAL DAY",
            IntervalKind::Hour => "INTERVAL HOUR",
            IntervalKind::Minute => "INTERVAL MINUTE",
            IntervalKind::Second => "INTERVAL SECOND",
            IntervalKind::YearToMonth => "INTERVAL YEAR TO MONTH",
            IntervalKind::DayToHour => "INTERVAL DAY TO HOUR",
            IntervalKind::DayToMinute => "INTERVAL DAY TO MINUTE",
            IntervalKind::DayToSecond => "INTERVAL DAY TO SECOND",
            IntervalKind::HourToMinute => "INTERVAL HOUR TO MINUTE",
            IntervalKind::HourToSecond => "INTERVAL HOUR TO SECOND",
            IntervalKind::MinuteToSecond => "INTERVAL MINUTE TO SECOND",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §7 表里的每个类型：码值 → `DataType` → 码值，必须回到原点。
    #[test]
    fn every_table_row_round_trips() {
        let pairs: &[(i16, DataType)] = &[
            (SQL_CHAR, DataType::Char),
            (SQL_VARCHAR, DataType::VarChar),
            (SQL_LONGVARCHAR, DataType::LongVarChar),
            (SQL_WCHAR, DataType::WChar),
            (SQL_WVARCHAR, DataType::WVarChar),
            (SQL_WLONGVARCHAR, DataType::WLongVarChar),
            (SQL_NUMERIC, DataType::Decimal),
            (SQL_DECIMAL, DataType::Decimal),
            (SQL_FLOAT, DataType::Float),
            (SQL_REAL, DataType::Real),
            (SQL_DOUBLE, DataType::Double),
            (SQL_TINYINT, DataType::TinyInt),
            (SQL_SMALLINT, DataType::SmallInt),
            (SQL_INTEGER, DataType::Integer),
            (SQL_BIGINT, DataType::BigInt),
            (SQL_BIT, DataType::Bit),
            (SQL_BINARY, DataType::Binary),
            (SQL_VARBINARY, DataType::VarBinary),
            (SQL_LONGVARBINARY, DataType::LongVarBinary),
            (SQL_TYPE_DATE, DataType::Date),
            (SQL_TYPE_TIME, DataType::Time),
            (SQL_TYPE_TIMESTAMP, DataType::Timestamp),
            (SQL_GUID, DataType::Guid),
        ];
        for (code, want) in pairs {
            assert_eq!(DataType::from_odbc(*code), *want, "码值 {code}");
            // SQL_DECIMAL 与 SQL_NUMERIC 映射到同一个变体，按变体回码是规范值 DECIMAL。
            if *code == SQL_DECIMAL || *code == SQL_NUMERIC {
                assert_eq!(want.odbc_code(), SQL_DECIMAL, "{want} 的规范码");
            } else {
                assert_eq!(want.odbc_code(), *code, "{want} 回码不等");
            }
        }
    }

    #[test]
    fn numeric_and_decimal_share_one_variant() {
        assert_eq!(DataType::from_odbc(SQL_NUMERIC), DataType::Decimal);
        assert_eq!(DataType::from_odbc(SQL_DECIMAL), DataType::Decimal);
    }

    /// 13 个 INTERVAL 子类型全码值往返。
    #[test]
    fn all_thirteen_intervals_round_trip() {
        assert_eq!(IntervalKind::ALL.len(), 13);
        for (i, k) in IntervalKind::ALL.iter().enumerate() {
            let code = 101 + i as i16;
            assert_eq!(k.odbc_code(), code);
            assert_eq!(IntervalKind::from_odbc(code), Some(*k));
            assert_eq!(DataType::from_odbc(code), DataType::Interval(*k));
            assert_eq!(DataType::from_odbc(code).odbc_code(), code);
        }
        assert_eq!(IntervalKind::from_odbc(100), None);
        assert_eq!(IntervalKind::from_odbc(114), None);
    }

    /// **关键契约**：任意 `i16` 都被接受、都不 panic、未知码保留原值（§7 表末行）。
    ///
    /// 唯一不能「原码往返」的是 `SQL_NUMERIC`(2)——它与 `SQL_DECIMAL`(3) 归一化成
    /// 同一个变体，回码时取规范值 `SQL_DECIMAL`。这条例外写进断言，别让它悄悄扩大。
    #[test]
    fn every_code_is_accepted_and_canonicalises() {
        for code in i16::MIN..=i16::MAX {
            let dt = DataType::from_odbc(code);
            if code != SQL_NUMERIC {
                assert_eq!(dt.odbc_code(), code, "码值 {code} 往返断裂");
            } else {
                assert_eq!(dt.odbc_code(), SQL_DECIMAL, "SQL_NUMERIC 的规范码");
            }
            // 归一化必须幂等：再走一遍不漂移。
            assert_eq!(
                DataType::from_odbc(dt.odbc_code()),
                dt,
                "码值 {code} 归一化不稳定"
            );
            if let DataType::Unknown(c) = dt {
                assert_eq!(c, code, "未知码必须原样保留");
                assert_eq!(dt.category(), TypeCategory::Unknown);
            }
        }
    }

    #[test]
    fn unknown_fallback_is_byte_oriented() {
        let dt = DataType::from_odbc(4242);
        assert_eq!(dt, DataType::Unknown(4242));
        assert_eq!(dt.default_rust_type(), "Vec<u8>");
        assert_eq!(dt.to_string(), "Unknown(4242)");
    }

    /// `SQL_DATETIME`(9) 是 ODBC 2.x 的统称，不该出现在列描述里；
    /// 真出现时按未知处理，倒逼我们看见它（而不是猜成 DATE）。
    #[test]
    fn odbc2_datetime_umbrella_is_treated_as_unknown() {
        assert_eq!(DataType::from_odbc(SQL_DATETIME), DataType::Unknown(9));
    }

    #[test]
    fn categories_match_the_design() {
        assert_eq!(DataType::VarChar.category(), TypeCategory::Text);
        assert_eq!(DataType::LongVarChar.category(), TypeCategory::Text);
        assert_eq!(DataType::WVarChar.category(), TypeCategory::Text);
        assert_eq!(DataType::VarBinary.category(), TypeCategory::Binary);
        assert_eq!(DataType::LongVarBinary.category(), TypeCategory::Binary);
        assert_eq!(DataType::Guid.category(), TypeCategory::Binary);
        assert_eq!(DataType::Integer.category(), TypeCategory::Integer);
        assert_eq!(DataType::Double.category(), TypeCategory::Float);
        assert_eq!(DataType::Decimal.category(), TypeCategory::Decimal);
        assert_eq!(DataType::Bit.category(), TypeCategory::Boolean);
        assert_eq!(DataType::Timestamp.category(), TypeCategory::Temporal);
        assert_eq!(
            DataType::Interval(IntervalKind::DayToSecond).category(),
            TypeCategory::Interval
        );
        assert_eq!(DataType::Unknown(1_000).category(), TypeCategory::Unknown);
    }

    #[test]
    fn category_names_are_stable_for_error_messages() {
        // §7「列是 BLOB，用户 row.get::<String>()」→ DataTypeMismatch{expect:"Text",got:"Blob"}
        assert_eq!(TypeCategory::Text.as_str(), "Text");
        assert_eq!(DataType::LongVarBinary.category().as_str(), "Binary");
        assert_eq!(TypeCategory::Binary.to_string(), "Binary");
    }

    #[test]
    fn lob_classification_is_exact() {
        assert!(DataType::LongVarChar.is_lob());
        assert!(DataType::LongVarBinary.is_lob());
        assert!(!DataType::VarChar.is_lob());
        assert!(!DataType::Binary.is_lob());
        assert!(!DataType::VarBinary.is_lob());
        assert!(DataType::LongVarBinary.needs_chunked_read());
        assert!(!DataType::VarBinary.needs_chunked_read());
    }

    #[test]
    fn rust_type_names_follow_the_table() {
        for (dt, want) in [
            (DataType::VarChar, "String"),
            (DataType::TinyInt, "i8"),
            (DataType::SmallInt, "i16"),
            (DataType::Integer, "i32"),
            (DataType::BigInt, "i64"),
            (DataType::Real, "f32"),
            (DataType::Float, "f32"),
            (DataType::Double, "f64"),
            (DataType::Bit, "bool"),
            (DataType::Decimal, "Decimal"),
            (DataType::Binary, "Vec<u8>"),
            (DataType::LongVarBinary, "Vec<u8>"),
            (DataType::Date, "NaiveDate"),
            (DataType::Time, "NaiveTime"),
            (DataType::Timestamp, "NaiveDateTime"),
        ] {
            assert_eq!(dt.default_rust_type(), want, "{dt}");
        }
    }

    #[test]
    fn interval_helpers_match_the_field_layout() {
        use IntervalKind::*;
        assert!(Year.is_year_month() && Month.is_year_month() && YearToMonth.is_year_month());
        assert!(!Day.is_year_month());
        assert!(Day.has_day() && DayToSecond.has_day() && !Hour.has_day());
        assert!(Second.has_fraction() && MinuteToSecond.has_fraction());
        assert!(!YearToMonth.has_fraction() && !DayToHour.has_fraction());
        assert_eq!(DayToSecond.to_string(), "INTERVAL DAY TO SECOND");
    }

    #[test]
    fn display_names_are_human_readable() {
        assert_eq!(DataType::LongVarChar.to_string(), "CLOB");
        assert_eq!(DataType::LongVarBinary.to_string(), "BLOB");
        assert_eq!(DataType::Bit.to_string(), "BOOLEAN");
        assert_eq!(DataType::Integer.to_string(), "INT");
        assert_eq!(
            DataType::Interval(IntervalKind::YearToMonth).to_string(),
            "Interval(YearToMonth)"
        );
    }
}
