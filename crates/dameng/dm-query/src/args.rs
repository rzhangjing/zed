//! 参数：`Args` / `IntoArgs`（设计方案 §6.12 冻结项）。
//!
//! # 为什么是「一个 `Vec<DmValue>`」而不是泛型参数包
//!
//! §5.4 纪律 1 要求**一次 `query` 的全部 ODBC 调用发生在同一个阻塞闭包内**。
//! 参数只要在进入闭包前收集成一个自有 `Vec<DmValue>`，就能整包搬进阻塞线程，
//! 于是跨线程桥接次数与参数个数**无关**——1 个参数和 16 个参数都是 1 次桥接。
//!
//! `IntoArgs` 为 0~12 元组、`Args`、`Vec<DmValue>`、`&[DmValue]`、`[DmValue; N]`
//! 提供实现，元素只需 `Into<DmValue>`（`dm-core` 已为整数/浮点/`bool`/文本/二进制/
//! `chrono`/`rust_decimal` 以及 **`Option<T>`**（`None` → `DmValue::Null`）提供转换）。
//!
//! ```
//! use dm_query::Args;
//! let a = Args::from_iter([1_i64.into(), "x".into()]);
//! assert_eq!(a.len(), 2);
//! ```

use dm_core::DmValue;

/// 一次语句执行的位置参数。
///
/// 与 `sqlx` 的 `Arguments` 同形：**只存值**，不存类型标签——类型信息由
/// `DmValue` 自己携带（`TINYINT` 与 `INT` 是不同的变体，绑定时不必再猜）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args {
    values: Vec<DmValue>,
}

impl Args {
    /// 空参数。
    #[must_use]
    pub const fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// 预分配 `n` 个位置（稳态零分配的前提：调用方知道参数个数）。
    #[must_use]
    pub fn with_capacity(n: usize) -> Self {
        Self {
            values: Vec::with_capacity(n),
        }
    }

    /// 追加一个参数。
    pub fn push(&mut self, value: impl Into<DmValue>) -> &mut Self {
        self.values.push(value.into());
        self
    }

    /// 参数个数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// 是否没有参数。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 按位置读参数。
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&DmValue> {
        self.values.get(index)
    }

    /// 参数切片——绑定层（`dm-odbc::bind_params`）直接吃这个。
    #[must_use]
    pub fn values(&self) -> &[DmValue] {
        &self.values
    }

    /// 取出底层 `Vec`（搬进阻塞闭包时用，不再复制）。
    #[must_use]
    pub fn into_values(self) -> Vec<DmValue> {
        self.values
    }

    /// 清空但保留容量。
    pub fn clear(&mut self) {
        self.values.clear();
    }
}

impl FromIterator<DmValue> for Args {
    fn from_iter<T: IntoIterator<Item = DmValue>>(iter: T) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

/// 「可以当作参数集合」的类型。
///
/// 这是 `PooledConn::{query, execute, fetch_stream, execute_many}` 的参数入口，
/// 所以**任何**实现都要先转成 `Args`；零参数的调用写 `()`。
pub trait IntoArgs {
    /// 转成 [`Args`]。
    fn into_args(self) -> Args;
}

impl IntoArgs for Args {
    fn into_args(self) -> Args {
        self
    }
}

impl IntoArgs for () {
    fn into_args(self) -> Args {
        Args::new()
    }
}

impl IntoArgs for Vec<DmValue> {
    fn into_args(self) -> Args {
        Args { values: self }
    }
}

impl IntoArgs for &[DmValue] {
    fn into_args(self) -> Args {
        Args {
            values: self.to_vec(),
        }
    }
}

impl<const N: usize> IntoArgs for [DmValue; N] {
    fn into_args(self) -> Args {
        Args {
            values: self.to_vec(),
        }
    }
}

/// 为 N 元组生成 `IntoArgs`：每个元素只需 `Into<DmValue>`。
macro_rules! impl_into_args_tuple {
    ($count:expr; $($idx:tt => $name:ident),+) => {
        impl<$($name: Into<DmValue>),+> IntoArgs for ($($name,)+) {
            fn into_args(self) -> Args {
                let mut args = Args::with_capacity($count);
                $( args.push(self.$idx); )+
                args
            }
        }
    };
}

impl_into_args_tuple!(1; 0 => A);
impl_into_args_tuple!(2; 0 => A, 1 => B);
impl_into_args_tuple!(3; 0 => A, 1 => B, 2 => C);
impl_into_args_tuple!(4; 0 => A, 1 => B, 2 => C, 3 => D);
impl_into_args_tuple!(5; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E);
impl_into_args_tuple!(6; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F);
impl_into_args_tuple!(7; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G);
impl_into_args_tuple!(8; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H);
impl_into_args_tuple!(9; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I);
impl_into_args_tuple!(10; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I,
                      9 => J);
impl_into_args_tuple!(11; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I,
                      9 => J, 10 => K);
impl_into_args_tuple!(12; 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I,
                      9 => J, 10 => K, 11 => L);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_are_constructible_from_unit() {
        let args = ().into_args();
        assert!(args.is_empty());
        assert_eq!(args.len(), 0);
    }

    #[test]
    fn tuple_args_keep_order_and_types() {
        let args = (1_i32, "文本", true, 2.5_f64).into_args();
        assert_eq!(args.len(), 4);
        assert_eq!(args.get(0), Some(&DmValue::Int(1)));
        assert_eq!(args.get(1), Some(&DmValue::Text("文本".to_owned())));
        assert_eq!(args.get(2), Some(&DmValue::Bool(true)));
        assert_eq!(args.get(3), Some(&DmValue::Double(2.5)));
    }

    #[test]
    fn none_becomes_null_and_some_unwraps() {
        let args = (None::<i64>, Some(7_i64)).into_args();
        assert_eq!(args.get(0), Some(&DmValue::Null));
        assert_eq!(args.get(1), Some(&DmValue::BigInt(7)));
    }

    #[test]
    fn slices_and_arrays_are_accepted() {
        let owned = vec![DmValue::Int(1), DmValue::Null];
        let from_slice = owned.as_slice().into_args();
        assert_eq!(from_slice, Args::from_iter(owned.clone()));
        let from_array = [DmValue::Int(1), DmValue::Null].into_args();
        assert_eq!(from_array, Args::from_iter(owned));
    }

    #[test]
    fn twelve_tuple_is_supported() {
        let args = (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12_i32).into_args();
        assert_eq!(args.len(), 12);
        assert_eq!(args.get(11), Some(&DmValue::Int(12)));
    }
}
