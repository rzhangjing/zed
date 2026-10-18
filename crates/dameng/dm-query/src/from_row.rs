//! 行映射：`FromRow`（设计方案 §6.12 冻结项）。
//!
//! # 为什么按「元组 = 按位置取列」而不是「按名字取字段」
//!
//! 名字映射需要 `dm-macros` 的 `#[derive(FromRow)]`（T5.3）与运行期列名查找；
//! 元组按**位置**取值是最小的、没有歧义的起点（也是 `sqlx` 的 `FromRow` 口径），
//! 而 `Row::get("列名")` 随时能补上按名取值的场景。
//!
//! 单列实现（`i32` / `String` / …）等价于「第一列」，是为了 `query_one::<i64>` 这种
//! 便捷用法不必写 `(i64,)`。两者不冲突：`(i64,)` 与 `i64` 是两个不同的类型。

use dm_core::DmValue;
use dm_error::{DmError, DmErrorKind, Stage};

use crate::row::{Decode, Row};

/// 从一行解出 `Self`。
///
/// # Errors
///
/// 列缺失/类型不允许/NULL 落在非 `Option` 上时，错误语义与 [`Row::get`] 完全一致。
pub trait FromRow: Sized {
    /// 映射一行。
    fn from_row(row: &Row) -> Result<Self, DmError>;
}

/// 单列：等价于 `row.get::<Self>(0)`。
macro_rules! impl_from_row_scalar {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromRow for $ty {
                fn from_row(row: &Row) -> Result<Self, DmError> {
                    row.get::<Self>(0_usize)
                }
            }
        )*
    };
}

impl_from_row_scalar!(
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    u64,
    bool,
    f32,
    f64,
    String,
    Vec<u8>
);

#[cfg(feature = "chrono")]
impl_from_row_scalar!(chrono::NaiveDate, chrono::NaiveTime, chrono::NaiveDateTime);

#[cfg(feature = "decimal")]
impl_from_row_scalar!(rust_decimal::Decimal);

impl FromRow for dm_core::DmInterval {
    fn from_row(row: &Row) -> Result<Self, DmError> {
        row.get::<Self>(0_usize)
    }
}

/// 零列：`query_one::<()>` 用来「只关心有没有行」。
impl FromRow for () {
    fn from_row(_row: &Row) -> Result<Self, DmError> {
        Ok(())
    }
}

/// 整行搬运（结果集本身已经是 `Row` 了）。
impl FromRow for Row {
    fn from_row(row: &Row) -> Result<Self, DmError> {
        Ok(row.clone())
    }
}

/// 第一列的原始值（不做任何转换，NULL 也照搬）。
impl FromRow for DmValue {
    fn from_row(row: &Row) -> Result<Self, DmError> {
        match row.values().first() {
            Some(value) => Ok(value.clone()),
            None => Err(no_columns()),
        }
    }
}

impl<T: Decode> FromRow for Option<T> {
    fn from_row(row: &Row) -> Result<Self, DmError> {
        row.get::<Self>(0_usize)
    }
}

/// 没有列可解（`Affected` 的退化行）。
fn no_columns() -> DmError {
    let mut err = DmError::at(DmErrorKind::Decode { column: None }, Stage::Fetch);
    err.push_diag(dm_error::DiagRecord::native(
        0,
        "这一行没有列：多半是把 DML 的 Affected 结果当成结果集来解了",
    ));
    err
}

/// 元组：按位置取列，第 n 个元素取第 n 列。
macro_rules! impl_from_row_tuple {
    ($($idx:tt => $name:ident),+) => {
        impl<$($name: Decode),+> FromRow for ($($name,)+) {
            fn from_row(row: &Row) -> Result<Self, DmError> {
                Ok(( $( row.get::<$name>($idx)?, )+ ))
            }
        }
    };
}

impl_from_row_tuple!(0 => A);
impl_from_row_tuple!(0 => A, 1 => B);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I, 9 => J);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I, 9 => J,
                     10 => K);
impl_from_row_tuple!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I, 9 => J,
                     10 => K, 11 => L);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use dm_core::{ColumnMeta, DataType};

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

    fn row(values: Vec<DmValue>) -> Row {
        let n = values.len();
        let names: Vec<String> = (0..n).map(|i| format!("C{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        Row::new(values, columns(&refs))
    }

    #[test]
    fn tuple_maps_by_position() {
        let r = row(vec![DmValue::Int(1), DmValue::Text("李".to_owned())]);
        let (a, b): (i32, String) = FromRow::from_row(&r).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, "李");
    }

    #[test]
    fn scalar_maps_first_column() {
        let r = row(vec![DmValue::BigInt(42)]);
        assert_eq!(i64::from_row(&r).unwrap(), 42);
        assert_eq!(i32::from_row(&r).unwrap(), 42);
    }

    #[test]
    fn option_maps_null() {
        let r = row(vec![DmValue::Null]);
        assert_eq!(Option::<i32>::from_row(&r).unwrap(), None);
    }

    #[test]
    fn dm_value_and_row_pass_through() {
        let r = row(vec![DmValue::Int(5)]);
        assert_eq!(DmValue::from_row(&r).unwrap(), DmValue::Int(5));
        assert_eq!(Row::from_row(&r).unwrap(), r);
        assert_eq!(<()>::from_row(&r).unwrap(), ());
    }

    #[test]
    fn arity_too_short_reports_structured_error() {
        let r = row(vec![DmValue::Int(1)]);
        let err = <(i32, i32)>::from_row(&r).unwrap_err();
        assert_eq!(err.kind, DmErrorKind::Decode { column: None });
        assert!(err.diags()[0].message.contains("越界"));
    }

    #[test]
    fn twelve_columns_are_supported() {
        let r = row((1..=12).map(DmValue::Int).collect());
        let t: (i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32) =
            FromRow::from_row(&r).unwrap();
        assert_eq!(t.11, 12);
    }
}
