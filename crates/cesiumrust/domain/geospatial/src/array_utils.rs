//! 数组工具（相邻重复项去重等）。

use crate::math_utils::EPSILON10;

/// 移除值数组中相邻的重复值。
///
/// 映射到 CesiumJS `arrayRemoveDuplicates`。
///
/// # 参数
/// * `values` - 值的数组。
/// * `equals_epsilon` - 使用 epsilon 比较值的函数：`fn(&T, &T, f64) -> bool`。
/// * `wrap_around` - 将最后一个值与第一个值比对比。若相等，则移除最后一个。
///
/// # 返回
/// 一个元组 `(cleaned_values, removed_indices)`。
/// - 若未发现重复，则原样返回原始值且 removed_indices 为空。
/// - 若发现重复，则返回一个移除了重复项及其原始索引的新 Vec。
pub fn array_remove_duplicates<T: Clone>(
    values: &[T],
    equals_epsilon: fn(&T, &T, f64) -> bool,
    wrap_around: bool,
) -> (Vec<T>, Vec<usize>) {
    let mut removed_indices: Vec<usize> = Vec::new();

    let length = values.len();
    if length < 2 {
        return (values.to_vec(), removed_indices);
    }

    let mut cleaned_values: Option<Vec<T>> = None;
    let mut last_clean_index: usize = 0;
    let mut removed_index_lci: usize = 0;

    let mut v0_idx = 0usize;

    // 从前到后扫描：以 v0_idx 为基准与当前项比较，相等则记入 removed_indices，
    // 否则推进基准并把该值保留到 cleaned_values。
    for i in 1..length {
        if equals_epsilon(&values[v0_idx], &values[i], EPSILON10) {
            if cleaned_values.is_none() {
                cleaned_values = Some(values[0..i].to_vec());
                last_clean_index = i - 1;
                removed_index_lci = 0;
            }
            removed_indices.push(i);
        } else {
            if let Some(ref mut cv) = cleaned_values {
                cv.push(values[i].clone());
                last_clean_index = i;
                removed_index_lci = removed_indices.len();
            }
            v0_idx = i;
        }
    }

    if wrap_around && equals_epsilon(&values[0], &values[length - 1], EPSILON10) {
        if let Some(ref mut cv) = cleaned_values {
            // 将 lastCleanIndex 插入到 removedIndices 的正确排序位置
            removed_indices.insert(removed_index_lci, last_clean_index);
            cv.truncate(cv.len() - 1);
        } else {
            removed_indices.push(length - 1);
            cleaned_values = Some(values[0..length - 1].to_vec());
        }
    }

    match cleaned_values {
        Some(cv) => (cv, removed_indices),
        None => (values.to_vec(), removed_indices),
    }
}

/// 便捷函数：若未移除任何重复项（原始数组未变）则返回 true。
pub fn array_remove_duplicates_in_place<T: Clone>(
    values: &[T],
    equals_epsilon: fn(&T, &T, f64) -> bool,
    wrap_around: bool,
    removed_indices: &mut Vec<usize>,
) -> Vec<T> {
    let (result, removed) = array_remove_duplicates(values, equals_epsilon, wrap_around);
    *removed_indices = removed;
    result
}
