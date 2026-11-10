//! 核心工具函数（二分查找、重心坐标、三角形内点判定、数组细分）。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_div_ceil)]
use crate::math_utils::EPSILON14;
use glam::DVec3;

/// 使用二分查找在有序数组中查找一个项。
///
/// 若 `item_to_find` 存在则返回其索引。
/// 若未找到，返回一个负数，它是该项应插入位置
/// 之前的索引的按位取反（!）。
///
/// 映射到 `binarySearch(array, itemToFind, comparator)`
pub fn binary_search<T, F>(array: &[T], item_to_find: &T, comparator: F) -> i64
where
    F: Fn(&T, &T) -> i64,
{
    let mut low: i64 = 0;
    let mut high: i64 = array.len() as i64 - 1;

    while low <= high {
        let i = ((low + high) / 2) as usize;
        let comparison = comparator(&array[i], item_to_find);
        if comparison < 0 {
            low = i as i64 + 1;
        } else if comparison > 0 {
            high = i as i64 - 1;
        } else {
            return i as i64;
        }
    }
    !(high + 1)
}

/// 计算一个点关于一个三角形（3D）的重心坐标。
///
/// 返回 Some(DVec3)，其中 x、y、z 分别对应于
/// p0、p1、p2 的重心坐标。若三角形退化则返回 None。
///
/// 映射到 `barycentricCoordinates(point, p0, p1, p2)`
pub fn barycentric_coordinates(
    point: DVec3,
    p0: DVec3,
    p1: DVec3,
    p2: DVec3,
) -> Option<DVec3> {
    // 检查点是否等于任一顶点
    if point.abs_diff_eq(p0, EPSILON14) {
        return Some(DVec3::X);
    }
    if point.abs_diff_eq(p1, EPSILON14) {
        return Some(DVec3::Y);
    }
    if point.abs_diff_eq(p2, EPSILON14) {
        return Some(DVec3::Z);
    }

    let v0 = p1 - p0;
    let v1 = p2 - p0;
    let v2 = point - p0;

    let dot00 = v0.dot(v0);
    let dot01 = v0.dot(v1);
    let dot02 = v0.dot(v2);
    let dot11 = v1.dot(v1);
    let dot12 = v1.dot(v2);

    let mut y = dot11 * dot02 - dot01 * dot12;
    let mut z = dot00 * dot12 - dot01 * dot02;
    let q = dot00 * dot11 - dot01 * dot01;

    // 三角形退化
    if q == 0.0 {
        return None;
    }

    y /= q;
    z /= q;
    let x = 1.0 - y - z;
    Some(DVec3::new(x, y, z))
}

/// 判断一个 2D 点是否在一个由三个 2D 点定义的三角形内部。
///
/// 仅当点严格位于内部（不在边或顶点上）时才返回 true。
///
/// 映射到 `pointInsideTriangle(point, p0, p1, p2)`
pub fn point_inside_triangle(
    point: (f64, f64),
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
) -> bool {
    // 使用重心坐标方法
    let (px, py) = point;
    let (x1, y1) = p0;
    let (x2, y2) = p1;
    let (x3, y3) = p2;

    let x1mx3 = x1 - x3;
    let x3mx2 = x3 - x2;
    let y2my3 = y2 - y3;
    let y1my3 = y1 - y3;
    let inverse_det = 1.0 / (y2my3 * x1mx3 + x3mx2 * y1my3);
    let dpx = px - x3;
    let dpy = py - y3;

    let u = (y2my3 * dpx + x3mx2 * dpy) * inverse_det;
    let v = (-y1my3 * dpx + x1mx3 * dpy) * inverse_det;
    let w = 1.0 - u - v;

    // 严格内部：所有坐标必须 > 0（不在边上）
    u > 0.0 && v > 0.0 && w > 0.0
}

/// 将数组拆分为指定数量的子数组。
///
/// 映射到 `subdivideArray(array, numberOfArrays)`
pub fn subdivide_array<T: Clone>(array: &[T], number_of_arrays: usize) -> Vec<Vec<T>> {
    debug_assert!(number_of_arrays > 0, "number_of_arrays must be > 0");

    let length = array.len();
    if length == 0 {
        return Vec::new();
    }

    let mut result: Vec<Vec<T>> = Vec::with_capacity(number_of_arrays);
    let mut i = 0;
    // 每一轮按“剩余元素向上取整均分到剩余子数组”计算切片长度，
    // 使各子数组长度尽量均衡。
    for _ in 0..number_of_arrays {
        let remaining = length - i;
        let remaining_arrays = number_of_arrays - result.len();
        let count = (remaining + remaining_arrays - 1) / remaining_arrays;
        let end = (i + count).min(length);
        if i < end {
            result.push(array[i..end].to_vec());
        }
        i = end;
    }
    result
}

/// 使用稳定排序（归并排序语义）就地排序数组。
///
/// 比较器返回一个 Ordering：若 a 应排在 b 之前则为 Less。
pub fn merge_sort<T, F>(array: &mut [T], comparator: F)
where
    F: Fn(&T, &T) -> std::cmp::Ordering,
{
    // Rust 的 sort_by 是稳定排序（自适应归并排序 + 插入排序），
    // 与 CesiumJS mergeSort 的语义完全匹配。
    array.sort_by(comparator);
}
