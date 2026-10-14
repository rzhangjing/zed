//! 多边形几何库函数。
//!
//! 提供将跨越赤道（反子午线附近）的多边形拆分为若干部份的工具：
//! 先按粒度对恒向线/大地线边细分，再找出所有与赤道平面相交的边并插入交点，
//! 最后以交点为分隔递归地重新连线，把一个跨赤道的环拆为北/南两个不跨接的子环。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(unused_mut)]
use crate::ellipsoid::Ellipsoid;
use crate::ellipsoid_rhumb_line::EllipsoidRhumbLine;
use crate::math_utils;
use crate::ray::{line_segment_plane, Plane};
use glam::DVec3;
use std::f64::consts::PI;

/// 多边形边的弧类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArcType {
    /// 直线（无弧）。
    None,
    /// 大地线弧（大圆）。
    Geodesic,
    /// 恒向线弧（等倾角）。
    Rhumb,
}

/// 将两点 Cartesian3 之间的恒向线细分为一个扁平的位置数组。
///
/// 映射到 CesiumJS `PolygonGeometryLibrary.subdivideRhumbLine`
///
/// # 参数
/// - `ellipsoid`：参考椭球。
/// - `p0`、`p1`：线段两端点（地心固定系）。
/// - `min_distance`：目标最小段长（米），用于确定细分份数。
///
/// # 返回
/// 扁平的 `[x,y,z,...]` 坐标数组；转换失败或已短于 `min_distance` 时仅含 `p0`。
pub fn subdivide_rhumb_line(
    ellipsoid: &Ellipsoid,
    p0: DVec3,
    p1: DVec3,
    min_distance: f64,
) -> Vec<f64> {
    let c0 = ellipsoid.cartesian_to_cartographic(p0);
    let c1 = ellipsoid.cartesian_to_cartographic(p1);

    let (c0, c1) = match (c0, c1) {
        (Some(c0), Some(c1)) => (c0, c1),
        _ => {
            // 若转换失败，则仅返回 p0
            return vec![p0.x, p0.y, p0.z];
        }
    };

    let rhumb = EllipsoidRhumbLine::new(&c0, &c1, ellipsoid);

    if rhumb.surface_distance() <= min_distance {
        // 无需细分已经短于最小距离的线
        return vec![p0.x, p0.y, p0.z];
    }

    let n = rhumb.surface_distance() / min_distance;
    let count_divide = (math_utils::log2(n)).ceil().max(0.0) as u32;
    let num_vertices = 1u32 << count_divide; // 2^count_divide
    let distance_between_vertices = rhumb.surface_distance() / num_vertices as f64;

    let mut positions = Vec::with_capacity(num_vertices as usize * 3);
    for i in 0..num_vertices {
        let c = rhumb.interpolate_using_surface_distance(i as f64 * distance_between_vertices);
        let p = ellipsoid.cartographic_to_cartesian(&c);
        positions.push(p.x);
        positions.push(p.y);
        positions.push(p.z);
    }

    positions
}

/// 赤道平面上的边，用于多边形拆分期间。
struct EdgeOnPlane {
    /// 指向 positions 数组的索引。
    position: usize,
    /// 起点 z 坐标的符号（-1、0 或 1）。
    edge_type: i32,
    /// 在连线过程中此边是否已被访问。
    visited: bool,
    /// 下一点 z 坐标的符号。
    next: i32,
    /// 交点的经度（用于排序）。
    theta: f64,
}

/// 计算一条大地线边与赤道的交点。
///
/// 把赤道视为过原点、法线为 +Z 的平面，求线段与该平面的交点并缩回椭球面。
///
/// # 返回
/// 交点（地心固定系）；线段不穿过赤道平面时返回 `None`。
fn compute_equator_intersection_geodesic(
    start: DVec3,
    end: DVec3,
    ellipsoid: &Ellipsoid,
) -> Option<DVec3> {
    // 赤道平面：法线 = (0, 0, 1)，距离 = 0
    let plane = Plane::from_point_normal(DVec3::ZERO, DVec3::Z);
    let intersection = line_segment_plane(start, end, &plane)?;
    ellipsoid.scale_to_geodetic_surface(intersection)
}

/// 计算一条恒向线边与赤道的交点。
///
/// 仅当两端点纬度异号时才可能穿赤道；沿恒向线插值得到纬度 0 的交点后，
/// 还需校验其经度落在两端点经度区间内（含跨国际日期变更线的交换处理）。
fn compute_equator_intersection_rhumb(
    start: DVec3,
    end: DVec3,
    ellipsoid: &Ellipsoid,
) -> Option<DVec3> {
    let c0 = ellipsoid.cartesian_to_cartographic(start)?;
    let c1 = ellipsoid.cartesian_to_cartographic(end)?;

    // 若两点位于赤道同侧，则无交点
    if c0.latitude.signum() == c1.latitude.signum() {
        return None;
    }

    let mut rhumb = EllipsoidRhumbLine::new(&c0, &c1, ellipsoid);
    let intersection = rhumb.find_intersection_with_latitude(0.0)?;

    let min_longitude = c0.longitude.min(c1.longitude);
    let max_longitude = c0.longitude.max(c1.longitude);

    let (min_lon, max_lon) = if (max_longitude - min_longitude).abs() > PI {
        // 跨越国际日期变更线，交换 min 和 max
        (max_longitude, min_longitude)
    } else {
        (min_longitude, max_longitude)
    };

    if intersection.longitude < min_lon || intersection.longitude > max_lon {
        return None;
    }

    Some(ellipsoid.cartographic_to_cartesian(&intersection))
}

/// 根据弧类型计算一条边与赤道的交点。
///
/// [`ArcType::Rhumb`] 走恒向线算法，其余（大地线/直线）走大地线平面交点算法。
fn compute_equator_intersection(
    start: DVec3,
    end: DVec3,
    ellipsoid: &Ellipsoid,
    arc_type: ArcType,
) -> Option<DVec3> {
    match arc_type {
        ArcType::Rhumb => compute_equator_intersection_rhumb(start, end, ellipsoid),
        _ => compute_equator_intersection_geodesic(start, end, ellipsoid),
    }
}

/// 找出所有与赤道平面相交的边，并将交点拼接进 positions 数组。
///
/// # 参数
/// - `positions`：多边形环顶点；本函数会向其中插入新的赤道交点。
/// - `ellipsoid`：参考椭球。
/// - `arc_type`：边所遵循的弧类型。
///
/// # 返回
/// 每条与赤道平面相交（或端点落在平面上）的边对应的 [`EdgeOnPlane`]，含交点索引、类型与排序用经度。
fn compute_edges_on_plane(
    positions: &mut Vec<DVec3>,
    ellipsoid: &Ellipsoid,
    arc_type: ArcType,
) -> Vec<EdgeOnPlane> {
    let mut edges_on_plane: Vec<EdgeOnPlane> = Vec::new();
    let mut i = 0;

    while i < positions.len() {
        let start_point = positions[i];
        let end_point = positions[(i + 1) % positions.len()];

        let edge_type = math_utils::sign(start_point.z) as i32;
        let next = math_utils::sign(end_point.z) as i32;

        let get_longitude = |position: DVec3| -> f64 {
            ellipsoid
                .cartesian_to_cartographic(position)
                .map(|c| c.longitude)
                .unwrap_or(0.0)
        };

        if edge_type == 0 {
            // 起始位置位于拆分平面上
            edges_on_plane.push(EdgeOnPlane {
                position: i,
                edge_type,
                visited: false,
                next,
                theta: get_longitude(start_point),
            });
        } else if next != 0 {
            let intersection =
                compute_equator_intersection(start_point, end_point, ellipsoid, arc_type);

            i += 1;
            if intersection.is_none() {
                // 该线段完全位于上方或下方
                // 注意：`continue` 跳过了底部的 i+=1，与 JS 行为一致
                continue;
            }

            // 该线段穿过了赤道
            let intersection = intersection.unwrap();
            positions.insert(i, intersection);
            edges_on_plane.push(EdgeOnPlane {
                position: i,
                edge_type,
                visited: false,
                next,
                theta: get_longitude(intersection),
            });
        }

        i += 1;
    }

    edges_on_plane
}

/// 由位置和边信息递归地连线多边形。
///
/// 从 `start_index` 出发沿环逐点采集顶点，遇到已插入的赤道交点就跨过平面到另一侧
/// 继续，从而把一个跨赤道环拆为属于当前半球的子多边形，并对未访问的交点递归地连线另一半。
///
/// # 参数
/// - `polygons`：输出用的多边形环集合，本函数会就地替换/插入结果。
/// - `polygon_index`：当前正在处理的 `polygons` 下标。
/// - `positions`：含交点的完整顶点序列。
/// - `edges_on_plane`：赤道交点边集合（就地标记 `visited`）。
/// - `to_delete`：替换时需删除的旧环数量。
/// - `start_index`：本轮连线在 `positions` 中的起始下标。
/// - `above_plane`：当前子环是否位于北半球（决定遍历方向与跟边条件）。
///
/// # 返回
/// 下一个待处理多边形在 `polygons` 中的下标。
#[allow(clippy::too_many_arguments)]
fn wire_polygon(
    polygons: &mut Vec<Vec<DVec3>>,
    polygon_index: usize,
    positions: &[DVec3],
    edges_on_plane: &mut Vec<EdgeOnPlane>,
    to_delete: usize,
    start_index: usize,
    above_plane: bool,
) -> usize {
    let mut polygon: Vec<DVec3> = Vec::new();
    let mut i = start_index;
    let mut polygons_to_wire: Vec<usize> = Vec::new();

    loop {
        if i >= positions.len() || i == start_index && !polygon.is_empty() {
            break;
        }
        if polygon.len() >= positions.len() {
            break;
        }

        let position = positions[i];
        polygon.push(position);

        let edge_index = edges_on_plane.iter().position(|e| e.position == i);
        // 若当前位置不是交点，则直接前进到下一个顶点。
        let edge = match edge_index {
            Some(idx) => idx,
            None => {
                i += 1;
                continue;
            }
        };

        let has_been_visited = edges_on_plane[edge].visited;
        let edge_type = edges_on_plane[edge].edge_type;
        let next = edges_on_plane[edge].next;
        edges_on_plane[edge].visited = true;

        if edge_type == 0 {
            if next == 0 {
                // 特殊情况：沿边回退
                let prev_edge_idx = if above_plane {
                    if edge > 0 { Some(edge - 1) } else { None }
                } else {
                    if edge + 1 < edges_on_plane.len() { Some(edge + 1) } else { None }
                };

                if let Some(prev_idx) = prev_edge_idx {
                    if edges_on_plane[prev_idx].position == i + 1 {
                        edges_on_plane[prev_idx].visited = true;
                    } else {
                        i += 1;
                        continue;
                    }
                } else {
                    i += 1;
                    continue;
                }
            }

            // 三个多边形相接的特殊情况
            if (!has_been_visited && above_plane && next > 0)
                || (start_index == i && !above_plane && next < 0)
            {
                i += 1;
                continue;
            }
        }

        let follow_edge = if above_plane { edge_type >= 0 } else { edge_type <= 0 };
        if !follow_edge {
            i += 1;
            continue;
        }

        if !has_been_visited {
            // 从另一侧的这个位置开始连线另一个多边形
            polygons_to_wire.push(i);
        }

        // 逆时针继续到下一条边
        let next_edge_index = if above_plane {
            if edge + 1 < edges_on_plane.len() { Some(edge + 1) } else { None }
        } else {
            if edge > 0 { Some(edge - 1) } else { None }
        };

        match next_edge_index {
            Some(next_idx) => {
                i = edges_on_plane[next_idx].position;
            }
            None => {
                i += 1;
                continue;
            }
        }
    }

    // 替换 polygon_index 处的多边形
    if to_delete > 0 && polygon_index < polygons.len() {
        polygons.splice(polygon_index..polygon_index + to_delete, vec![polygon]);
    } else {
        polygons.insert(polygon_index, polygon);
    }

    let mut current_index = polygon_index;
    for wire_index in polygons_to_wire {
        current_index = wire_polygon(
            polygons,
            current_index + 1,
            positions,
            edges_on_plane,
            0,
            wire_index,
            !above_plane,
        );
    }

    current_index
}

/// 沿赤道拆分一个多边形数组。
///
/// 映射到 CesiumJS `PolygonGeometryLibrary.splitPolygonsOnEquator`
///
/// # 参数
/// - `outer_rings`：各多边形外环（地心固定系顶点序列）。
/// - `ellipsoid`：参考椭球。
/// - `arc_type`：边所遵循的弧类型。
///
/// # 返回
/// 拆分后的多边形环数组；不跨赤道的环原样保留，跨赤道的环被拆为北/南多个子环。
pub fn split_polygons_on_equator(
    outer_rings: &[Vec<DVec3>],
    ellipsoid: &Ellipsoid,
    arc_type: ArcType,
) -> Vec<Vec<DVec3>> {
    let mut result: Vec<Vec<DVec3>> = outer_rings.to_vec();

    let mut current_polygon = 0;
    while current_polygon < result.len() {
        let outer_ring = result[current_polygon].clone();
        let mut positions = outer_ring.clone();

        if outer_ring.len() < 3 {
            result[current_polygon] = positions;
            current_polygon += 1;
            continue;
        }

        // 步骤 1：获取所有与拆分线相交的边
        let mut edges_on_plane = compute_edges_on_plane(&mut positions, ellipsoid, arc_type);

        // 若没有相交或平面上只有一个点，则使用原多边形
        if positions.len() == outer_ring.len() || edges_on_plane.len() <= 1 {
            result[current_polygon] = positions;
            current_polygon += 1;
            continue;
        }

        // 步骤 2：沿拆分线按经度对边排序
        edges_on_plane.sort_by(|a, b| a.theta.partial_cmp(&b.theta).unwrap_or(std::cmp::Ordering::Equal));

        // 步骤 3：重新连线多边形
        let north = positions[0].z >= 0.0;
        current_polygon = wire_polygon(
            &mut result,
            current_polygon,
            &positions,
            &mut edges_on_plane,
            1,
            0,
            north,
        );
    }

    result
}
