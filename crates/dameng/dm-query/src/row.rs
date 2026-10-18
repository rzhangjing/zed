//! 取值：`ColumnIndex`、`Decode`、`Row`（设计方案 §6.12 冻结项）。
//!
//! # 三条不会让步的语义
//!
//! 1. `get::<T>()` 类型不匹配 → [`DmErrorKind::Decode`]，**绝不静默返回默认值**；
//! 2. `NULL` → 非 `Option<T>` → [`DmErrorKind::NullViolation`]（**带列名**）；
//! 3. 列名查找**大小写不敏感**（DM 默认把标识符折成大写，而 ODBC 元数据里
//!    回来的形态取决于驱动——冻结为不敏感是为了与那个形态解耦）。
//!
//! # 列序号/列名不存在的错误形态（在冻结的错误模型里的落法）
//!
//! `DmErrorKind` 是冻结的**全变体**枚举，里面没有 `ColumnNotFound` /
//! `ColumnIndexOutOfRange`（§15.5 的 CAP-F-06 要的是「结构化错误」而不是
//! 「特定变体名」）。两层信息因此这样落：
//!
//! | 情形 | `kind` | 诊断记录 |
//! |---|---|---|
//! | 列名不存在 | `Decode { column: Some(名字) }` | 「结果集里没有名为 … 的列；现有列：…」 |
//! | 序号越界 | `Decode { column: None }` | 「列序号 N 越界（结果集只有 M 列）」 |
//!
//! 两边的 `Stage` 都是 [`Stage::Fetch`]——它们只可能发生在「读已取回的值」这一步。

use std::sync::Arc;

use dm_core::{ColumnMeta, ConvertError, DmValue, ValueRef};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};

use crate::from_row::FromRow;

// ---------------------------------------------------------------------------
// 错误构造（本模块内共享给 rowset / stream / conn）
// ---------------------------------------------------------------------------

/// 列名不存在。
pub(crate) fn column_not_found(name: &str, columns: &[ColumnMeta]) -> DmError {
    let mut available: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    if available.len() > 12 {
        available.truncate(12);
    }
    let mut err = DmError::at(
        DmErrorKind::Decode {
            column: Some(name.to_owned()),
        },
        Stage::Fetch,
    );
    err.push_diag(DiagRecord::native(
        0,
        format!(
            "结果集里没有名为「{name}」的列（列名查找大小写不敏感）；当前结果集的列：{}",
            available.join(", ")
        ),
    ));
    err
}

/// 列序号越界。
pub(crate) fn column_out_of_range(index: usize, len: usize) -> DmError {
    let mut err = DmError::at(DmErrorKind::Decode { column: None }, Stage::Fetch);
    err.push_diag(DiagRecord::native(
        0,
        format!("列序号 {index} 越界：当前结果集只有 {len} 列（序号从 0 开始）"),
    ));
    err
}

// ---------------------------------------------------------------------------
// ColumnIndex
// ---------------------------------------------------------------------------

/// 「怎么在大海里捞到那一列」——序号或列名。
///
/// 冻结约定：`&str` 实现**大小写不敏感**（ASCII 折叠，DM 的标识符就是 ASCII 折叠规则）。
pub trait ColumnIndex {
    /// 在 `columns` 里定位列，返回从 0 开始的序号。
    ///
    /// # Errors
    ///
    /// 列名不存在或序号越界时返回 [`DmErrorKind::Decode`]（见模块文档的对照表）。
    fn index(&self, columns: &[ColumnMeta]) -> Result<usize, DmError>;
}

impl ColumnIndex for usize {
    fn index(&self, columns: &[ColumnMeta]) -> Result<usize, DmError> {
        if *self < columns.len() {
            Ok(*self)
        } else {
            Err(column_out_of_range(*self, columns.len()))
        }
    }
}

impl ColumnIndex for &str {
    fn index(&self, columns: &[ColumnMeta]) -> Result<usize, DmError> {
        columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(self))
            .ok_or_else(|| column_not_found(self, columns))
    }
}

impl ColumnIndex for String {
    fn index(&self, columns: &[ColumnMeta]) -> Result<usize, DmError> {
        self.as_str().index(columns)
    }
}

impl ColumnIndex for &String {
    fn index(&self, columns: &[ColumnMeta]) -> Result<usize, DmError> {
        self.as_str().index(columns)
    }
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// 从一列的值解出 `Self`。
///
/// 实现者应当**只做白名单内的转换**：能安全互转的（整数家族、文本 ↔ 数字）允许，
/// 语义有损的一律走 [`ConvertError::Mismatch`] 拒绝。`Option<T>` 是唯一的
/// 「NULL 豁免」通道。
pub trait Decode: Sized {
    /// 解码一个值。`column` 只用于错误信息（列名），不影响转换规则。
    ///
    /// # Errors
    ///
    /// 类型不允许、解析失败、或 NULL 落在非 `Option` 上时返回 `DmError`。
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError>;
}

/// 整数家族：先走 `as_i64`，再核对目标宽度。
macro_rules! decode_signed {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Decode for $ty {
                fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
                    let n = value.as_i64().map_err(|e| e.into_dm_error(column))?;
                    <$ty>::try_from(n).map_err(|_| {
                        ConvertError::Parse {
                            from: value.data_type(),
                            to: stringify!($ty),
                            reason: format!("{n} 超出 {} 的取值范围", stringify!($ty)),
                        }
                        .into_dm_error(column)
                    })
                }
            }
        )*
    };
}

decode_signed!(i8, i16, i32, i64);

/// 无符号家族：`as_i64` 之后既要非负也要放得下。
macro_rules! decode_unsigned {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Decode for $ty {
                fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
                    let n = value.as_i64().map_err(|e| e.into_dm_error(column))?;
                    <$ty>::try_from(n).map_err(|_| {
                        ConvertError::Parse {
                            from: value.data_type(),
                            to: stringify!($ty),
                            reason: format!("{n} 不是合法的 {}", stringify!($ty)),
                        }
                        .into_dm_error(column)
                    })
                }
            }
        )*
    };
}

decode_unsigned!(u8, u16, u32, u64);

impl Decode for bool {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_bool().map_err(|e| e.into_dm_error(column))
    }
}

impl Decode for f32 {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        // `f32` 允许从 `REAL` 直取；`as_f64` 已覆盖 `REAL`/`DOUBLE`/文本。
        let x = value.as_f64().map_err(|e| e.into_dm_error(column))?;
        Ok(x as f32)
    }
}

impl Decode for f64 {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_f64().map_err(|e| e.into_dm_error(column))
    }
}

impl Decode for String {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_text().map_err(|e| e.into_dm_error(column))
    }
}

impl Decode for Vec<u8> {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value
            .as_bytes()
            .map(<[u8]>::to_vec)
            .map_err(|e| e.into_dm_error(column))
    }
}

impl Decode for DmValue {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        if value == ValueRef::Null {
            // 与其它实现保持一致：NULL 落到非 `Option` 上是 `NullViolation`。
            return Err(ConvertError::Null.into_dm_error(column));
        }
        Ok(value.to_owned_value())
    }
}

#[cfg(feature = "chrono")]
impl Decode for chrono::NaiveDate {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_date().map_err(|e| e.into_dm_error(column))
    }
}

#[cfg(feature = "chrono")]
impl Decode for chrono::NaiveTime {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_time().map_err(|e| e.into_dm_error(column))
    }
}

#[cfg(feature = "chrono")]
impl Decode for chrono::NaiveDateTime {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_timestamp().map_err(|e| e.into_dm_error(column))
    }
}

#[cfg(feature = "decimal")]
impl Decode for rust_decimal::Decimal {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_decimal().map_err(|e| e.into_dm_error(column))
    }
}

impl Decode for dm_core::DmInterval {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        value.as_interval().map_err(|e| e.into_dm_error(column))
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(value: ValueRef<'_>, column: Option<&str>) -> Result<Self, DmError> {
        if value == ValueRef::Null {
            return Ok(None);
        }
        T::decode(value, column).map(Some)
    }
}

// ---------------------------------------------------------------------------
// Row
// ---------------------------------------------------------------------------

/// 结果集里的一行。
///
/// **拥有**列元数据（`Arc`，与结果集共享同一份）与一行值。之所以拥有而不是借用：
/// `RowStream` 的 `Item` 必须是自有的（`Stream` 不能吐出自引用），两种取值入口
/// （`RowSet` 与 `RowStream`）因此统一成同一种类型，`FromRow` 也只需要面对一种形态。
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    columns: Arc<[ColumnMeta]>,
    values: Vec<DmValue>,
}

impl Row {
    /// 从裸值构造（走一次 `Vec` 的所有权转移，不复制值本体）。
    #[must_use]
    pub(crate) fn new(values: Vec<DmValue>, columns: Arc<[ColumnMeta]>) -> Self {
        Self { columns, values }
    }

    /// 从切片构造（复制一行值——`RowBuf` 的缓冲是复用的，行必须被搬出来）。
    #[must_use]
    pub(crate) fn from_slice(values: &[DmValue], columns: &Arc<[ColumnMeta]>) -> Self {
        Self::new(values.to_vec(), Arc::clone(columns))
    }

    /// 列元数据。
    #[must_use]
    pub fn columns(&self) -> &[ColumnMeta] {
        &self.columns
    }

    /// 列数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// 是否一行都没有（正常结果集不会出现，只在 `Affected` 的退化行上）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 裸值切片。
    #[must_use]
    pub fn values(&self) -> &[DmValue] {
        &self.values
    }

    /// 按序号或列名取**原始值**。
    ///
    /// # Errors
    ///
    /// 列定位失败时返回 [`DmErrorKind::Decode`]。
    pub fn value(&self, index: impl ColumnIndex) -> Result<&DmValue, DmError> {
        let i = index.index(&self.columns)?;
        self.values
            .get(i)
            .ok_or_else(|| column_out_of_range(i, self.values.len()))
    }

    /// 按序号或列名取值并解成 `T`。
    ///
    /// # Errors
    ///
    /// - 列定位失败 → [`DmErrorKind::Decode`]；
    /// - 类型不允许/解析失败 → `Decode` / `DataTypeMismatch`；
    /// - `NULL` 落到非 `Option<T>` → [`DmErrorKind::NullViolation`]（带列名）。
    pub fn get<T: Decode>(&self, index: impl ColumnIndex) -> Result<T, DmError> {
        let i = index.index(&self.columns)?;
        let value = self
            .values
            .get(i)
            .ok_or_else(|| column_out_of_range(i, self.values.len()))?;
        let name = self.columns.get(i).map(|c| c.name.as_str());
        T::decode(value.as_value_ref(), name)
    }

    /// 把整行映射成 `T`（`FromRow` 的便捷入口，等价于 `T::from_row(&row)`）。
    ///
    /// # Errors
    ///
    /// 由 `T` 的 `FromRow` 实现决定。
    pub fn to<T: FromRow>(&self) -> Result<T, DmError> {
        T::from_row(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dm_core::{DataType, DmValue};

    fn columns(names: &[&str]) -> Arc<[ColumnMeta]> {
        names
            .iter()
            .map(|n| ColumnMeta {
                name: (*n).to_owned(),
                data_type: DataType::VarChar,
                nullable: true,
                precision: None,
                scale: None,
                column_size: None,
                length_in_char: None,
            })
            .collect::<Vec<_>>()
            .into()
    }

    fn row() -> Row {
        Row::new(
            vec![
                DmValue::Int(7),
                DmValue::Text("文本".to_owned()),
                DmValue::Null,
            ],
            columns(&["ID", "NAME", "OPT"]),
        )
    }

    #[test]
    fn column_name_lookup_is_case_insensitive() {
        let r = row();
        assert_eq!(r.get::<i32>("id").unwrap(), 7);
        assert_eq!(r.get::<String>("Name").unwrap(), "文本");
        assert_eq!(r.get::<i32>("ID").unwrap(), 7);
    }

    #[test]
    fn unknown_column_reports_structured_error_with_name() {
        let err = row().get::<i32>("nope").unwrap_err();
        assert_eq!(
            err.kind,
            DmErrorKind::Decode {
                column: Some("nope".to_owned())
            }
        );
        assert_eq!(err.context.stage, Stage::Fetch);
        assert!(err.diags()[0].message.contains("NOPE") || err.diags()[0].message.contains("nope"));
    }

    #[test]
    fn out_of_range_index_reports_structured_error() {
        let err = row().get::<i32>(9_usize).unwrap_err();
        assert_eq!(err.kind, DmErrorKind::Decode { column: None });
        assert!(err.diags()[0].message.contains("越界"));
    }

    #[test]
    fn null_into_non_option_is_null_violation_with_column_name() {
        let err = row().get::<String>("OPT").unwrap_err();
        assert_eq!(
            err.kind,
            DmErrorKind::NullViolation {
                column: "OPT".to_owned()
            }
        );
    }

    #[test]
    fn null_into_option_is_none() {
        let r = row();
        assert_eq!(r.get::<Option<String>>("OPT").unwrap(), None);
        assert_eq!(r.get::<Option<i32>>("ID").unwrap(), Some(7));
    }

    #[test]
    fn integer_width_is_checked() {
        // 40_000 放得进 i32/i64/u16，放不进 i16——取 i16 必须报错而不是截断成负数。
        let r = Row::new(vec![DmValue::BigInt(40_000)], columns(&["N"]));
        assert!(r.get::<i16>("N").is_err());
        assert_eq!(r.get::<i32>("N").unwrap(), 40_000);
        assert_eq!(r.get::<i64>("N").unwrap(), 40_000);
        assert_eq!(r.get::<u16>("N").unwrap(), 40_000);
        assert!(r.get::<u8>("N").is_err());
    }

    #[test]
    fn negative_into_unsigned_is_rejected() {
        let r = Row::new(vec![DmValue::Int(-1)], columns(&["N"]));
        assert!(r.get::<u32>("N").is_err());
    }

    #[test]
    fn value_ref_exposes_raw_value() {
        let r = row();
        assert_eq!(r.value("ID").unwrap(), &DmValue::Int(7));
        assert_eq!(r.values().len(), 3);
        assert_eq!(r.len(), 3);
        assert!(!r.is_empty());
        assert_eq!(r.columns().len(), 3);
    }
}
