//! 走廊几何 —— 沿一条折线路径、宽度恒定的带状区域。
//!
//! 走廊由一系列中心线位置和一个宽度定义；
//! 几何体是左右边缘之间的扁平条状体（可选地带圆角/斜接/切角）。

use crate::bounding::BoundingSphere;
use crate::ellipsoid::Ellipsoid;
use crate::geometry::{GeometryData, PrimitiveType, VertexFormat};
use crate::math_utils::{self, EPSILON7};
use crate::polyline_pipeline::{generate_arc, ArcOptions};
use glam::DVec3;

/// 走廊转弯处的角风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CornerType {
    /// 圆角（默认）。
    #[default]
    Rounded,
    /// 斜接（尖锐）角。
    Mitered,
    /// 切角（裁切）。
    Beveled,
}

/// 描述一个走廊的选项。
#[derive(Debug, Clone)]
pub struct CorridorOptions {
    /// 中心线位置（至少 2 个）。
    pub positions: Vec<DVec3>,
    /// 宽度（米）。
    pub width: f64,
    /// 椭球表面上方的高度。
    pub height: f64,
    /// 角度粒度（弧度）。
    pub granularity: f64,
    /// 角风格。
    pub corner_type: CornerType,
    /// 参考椭球。
    pub ellipsoid: Ellipsoid,
}

impl Default for CorridorOptions {
    /// 默认走廊选项：空中心线、零宽度/高度、1° 角度粒度、圆角、WGS84 椭球。
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            width: 0.0,
            height: 0.0,
            granularity: std::f64::consts::PI / 180.0,
            corner_type: CornerType::Rounded,
            ellipsoid: Ellipsoid::WGS84,
        }
    }
}

/// 判断从 `backward` 到 `forward` 的角度（从椭球外部观察）是否大于 pi。
///
/// 映射到 `PolylineVolumeGeometryLibrary.angleIsGreaterThanPi`。
///
/// # 参数
/// - `forward`/`backward`：在当前顶点处指向前进/后退方向的切向向量。
/// - `position`：当前顶点（用于求地表法线）。
/// - `ellipsoid`：参考椭球。
///
/// # 返回
/// 以本地东-北标架叉积符号判断转向；角大于 π（即左转为外）时返回 `true`。
fn angle_is_greater_than_pi(
    forward: DVec3,
    backward: DVec3,
    position: DVec3,
    ellipsoid: &Ellipsoid,
) -> bool {
    // 在地表法线处构造局部东-北标架，用于把三维转向投影为二维叉积符号。
    let normal = ellipsoid.geodetic_surface_normal(position).unwrap_or(DVec3::Z);
    let mut east = DVec3::Z.cross(normal);
    if east.length_squared() < 1e-30 {
        east = DVec3::X.cross(normal);
    }
    east = east.normalize_or(DVec3::X);
    let north = normal.cross(east).normalize_or(DVec3::Y);

    let next_pt = position + forward;
    let prev_pt = position + backward;
    let next_x = (next_pt - position).dot(east);
    let next_y = (next_pt - position).dot(north);
    let prev_x = (prev_pt - position).dot(east);
    let prev_y = (prev_pt - position).dot(north);

    prev_x * next_y - prev_y * next_x >= 0.0
}

/// 将向量绕一个单位轴旋转一个角度（Rodrigues 公式）。
///
/// # 参数
/// - `v`：待旋转向量；`axis`：单位旋转轴；`angle`：弧度。
///
/// # 返回
/// 绕 `axis` 转过 `angle` 后的向量。
fn rotate_around_axis(v: DVec3, axis: DVec3, angle: f64) -> DVec3 {
    // Rodrigues 公式：v·cosθ + (k×v)·sinθ + k(k·v)(1-cosθ)。
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    v * cos_a + axis.cross(v) * sin_a + axis * axis.dot(v) * (1.0 - cos_a)
}

/// 计算一段圆角弧。
///
/// # 参数
/// - `corner_point`：角点（旋转中心所在球面点）。
/// - `start_point`/`end_point`：弧的起止点。
/// - `corner_type`：角风格，`Beveled` 时仅切一段。
/// - `left_is_outside`：外侧在左还是右，决定旋转轴朝向。
///
/// # 返回
/// 沿球面细分的角点位置数组，末点被强制对齐 `end_point`。
fn compute_round_corner(
    corner_point: DVec3,
    start_point: DVec3,
    end_point: DVec3,
    corner_type: CornerType,
    left_is_outside: bool,
) -> Vec<DVec3> {
    // 以角点为心，起止点相对角点的夹角即弧所张角。
    let v1 = start_point - corner_point;
    let v2 = end_point - corner_point;
    let angle = v1.angle_between(v2);
    let granularity = if corner_type == CornerType::Beveled {
        1
    } else {
        (angle / math_utils::to_radians(5.0)).ceil() as usize + 1
    };

    // 旋转轴取角点的（反）径向，使细分点始终贴在球面上。
    let axis = if left_is_outside {
        (-corner_point).normalize_or(DVec3::Z)
    } else {
        corner_point.normalize_or(DVec3::Z)
    };
    let step_angle = angle / granularity as f64;

    let mut array: Vec<DVec3> = Vec::with_capacity(granularity + 1);
    let mut current = start_point;
    for _ in 0..granularity {
        current = rotate_around_axis(current, axis, step_angle);
        array.push(current);
    }
    if let Some(last) = array.last_mut() {
        *last = end_point;
    }
    array
}

/// 计算一个斜接角（2 个点）。
///
/// # 参数
/// - `position`：角顶点。
/// - `left_corner_direction`：朝外侧的角方向向量（已按半宽缩放）。
/// - `last_point`：与角相连的边缘末点。
/// - `left_is_outside`：外侧方位。
///
/// # 返回
/// 两点数组：斜接尖点与末点。
fn compute_mitered_corner(
    position: DVec3,
    left_corner_direction: DVec3,
    last_point: DVec3,
    left_is_outside: bool,
) -> Vec<DVec3> {
    // 斜接尖点位于角顶点沿外侧方向偏移角方向向量之处。
    let corner_point = if left_is_outside {
        position + left_corner_direction
    } else {
        position - left_corner_direction
    };
    vec![corner_point, last_point]
}

/// 一个已计算的角（左侧或右侧位置）。
struct CornerData {
    /// 外侧（左边缘）角位置；当角位于左侧外部时为 `Some`，否则 `None`。
    left_positions: Option<Vec<DVec3>>,
    /// 外侧（右边缘）角位置；当角位于右侧外部时为 `Some`，否则 `None`。
    right_positions: Option<Vec<DVec3>>,
}

/// 将中心线弧偏移为右侧和左侧边缘位置。
///
/// # 参数
/// - `positions`：中心线弧上的点。
/// - `left`：左侧单位方向。
/// - `scalar`：偏移距离（半宽）。
/// - `out`：输出容器，依次压入右缘与左缘（左缘逆序）。
fn add_shifted_positions(
    positions: &[DVec3],
    left: DVec3,
    scalar: f64,
    out: &mut Vec<Vec<DVec3>>,
) {
    // 左右偏移向量互为相反数；沿弧逐点平移得到两条边缘。
    let scaled_left = left * scalar;
    let scaled_right = -scaled_left;

    let right_positions: Vec<DVec3> = positions.iter().map(|&p| p + scaled_right).collect();
    // 左侧位置以逆序存储（与 CesiumJS 一致）。
    let left_positions: Vec<DVec3> = positions.iter().rev().map(|&p| p + scaled_left).collect();

    out.push(right_positions);
    out.push(left_positions);
}

/// 映射到 `CorridorGeometryLibrary.computePositions`。
///
/// # 参数
/// - `positions`：中心线位置（≥2）。
/// - `width`：走廊宽度（米）。
/// - `granularity`：弧段细分角度（弧度）。
/// - `corner_type`：转弯角风格。
/// - `ellipsoid`：参考椭球。
///
/// # 返回
/// 逐段偏移后的边缘位置组，以及各转弯处的角数据。
fn compute_corridor_positions(
    positions: &[DVec3],
    width: f64,
    granularity: f64,
    corner_type: CornerType,
    ellipsoid: &Ellipsoid,
) -> (Vec<Vec<DVec3>>, Vec<CornerData>) {
    // 半宽：中心线左右两侧各偏移 half_width 形成边缘。
    let half_width = width / 2.0;
    let mut calculated_positions: Vec<Vec<DVec3>> = Vec::new();
    let mut corners: Vec<CornerData> = Vec::new();

    let mut position = positions[0];
    let mut next_position = positions[1];

    // 前进方向：当前段单位切向。
    let mut forward = (next_position - position).normalize_or(DVec3::X);
    // 地表法线 + 左手方向：left 垂直于切向并指向走廊左侧。
    let normal = ellipsoid.geodetic_surface_normal(position).unwrap_or(DVec3::Z);
    let mut left = normal.cross(forward).normalize_or(DVec3::Y);

    // 记录上一点，供逐段细分弧线使用。
    let mut previous_pos = position;
    position = next_position;
    let mut backward = -forward;

    let length = positions.len();
    for i in 1..length - 1 {
        let normal = ellipsoid.geodetic_surface_normal(position).unwrap_or(DVec3::Z);
        next_position = positions[i + 1];
        forward = (next_position - position).normalize_or(DVec3::X);

        let forward_proj = (forward - normal * forward.dot(normal)).normalize_or(DVec3::X);
        let backward_proj = (backward - normal * backward.dot(normal)).normalize_or(DVec3::X);

        // 前进/后退在切平面上的投影若不同向，则该顶点需要生成角。
        let do_corner =
            !math_utils::equals_epsilon(forward_proj.dot(backward_proj).abs(), 1.0, 0.0, EPSILON7);

        if do_corner {
            let mut corner_direction = (forward + backward).normalize_or(DVec3::X);
            corner_direction = corner_direction.cross(normal);
            corner_direction = normal.cross(corner_direction);
            corner_direction = corner_direction.normalize_or(DVec3::X);

            // 由角方向与后退方向夹角求斜接伸缩比例，限制最小值避免退化。
            let cross_mag = corner_direction.cross(backward).length();
            let scalar = half_width / cross_mag.max(0.25);

            let left_is_outside =
                angle_is_greater_than_pi(forward, backward, position, ellipsoid);

            corner_direction *= scalar;

            if left_is_outside {
                let right_pos = position + corner_direction;
                let center = right_pos + left * half_width;
                let left_pos = right_pos + left * (half_width * 2.0);

                let seg = [previous_pos, center];
                let opts = ArcOptions { positions: &seg, heights: None, granularity, ellipsoid };
                let subdivided = generate_arc(&opts);
                add_shifted_positions(&subdivided, left, half_width, &mut calculated_positions);

                let start_point = left_pos;
                left = normal.cross(forward).normalize_or(DVec3::Y);
                let new_left_pos = right_pos + left * (half_width * 2.0);
                previous_pos = right_pos + left * half_width;

                let corner_positions = match corner_type {
                    CornerType::Rounded | CornerType::Beveled => compute_round_corner(
                        right_pos, start_point, new_left_pos, corner_type, left_is_outside,
                    ),
                    CornerType::Mitered => compute_mitered_corner(
                        position, -corner_direction, new_left_pos, left_is_outside,
                    ),
                };
                corners.push(CornerData { left_positions: Some(corner_positions), right_positions: None });
            } else {
                let left_pos = position + corner_direction;
                let center = left_pos - left * half_width;
                let right_pos = left_pos - left * (half_width * 2.0);

                let seg = [previous_pos, center];
                let opts = ArcOptions { positions: &seg, heights: None, granularity, ellipsoid };
                let subdivided = generate_arc(&opts);
                add_shifted_positions(&subdivided, left, half_width, &mut calculated_positions);

                let start_point = right_pos;
                left = normal.cross(forward).normalize_or(DVec3::Y);
                let new_right_pos = left_pos - left * (half_width * 2.0);
                previous_pos = left_pos - left * half_width;

                let corner_positions = match corner_type {
                    CornerType::Rounded | CornerType::Beveled => compute_round_corner(
                        left_pos, start_point, new_right_pos, corner_type, left_is_outside,
                    ),
                    CornerType::Mitered => compute_mitered_corner(
                        position, corner_direction, new_right_pos, left_is_outside,
                    ),
                };
                corners.push(CornerData { left_positions: None, right_positions: Some(corner_positions) });
            }
            backward = -forward;
        }
        position = next_position;
    }

    // 最后一段。
    let seg = [previous_pos, position];
    let opts = ArcOptions { positions: &seg, heights: None, granularity, ellipsoid };
    let subdivided = generate_arc(&opts);
    add_shifted_positions(&subdivided, left, half_width, &mut calculated_positions);

    (calculated_positions, corners)
}

/// 由已计算的位置 + 角组装出右侧和左侧边缘。
///
/// # 参数
/// - `positions`：`compute_corridor_positions` 输出的成对右/左段。
/// - `corners`：各转弯角数据，按出现顺序穿插。
///
/// # 返回
/// 连续的 (右边缘, 左边缘) 顶点序列。
fn assemble_edges(
    positions: &[Vec<DVec3>],
    corners: &[CornerData],
) -> (Vec<DVec3>, Vec<DVec3>) {
    let mut right_edge: Vec<DVec3> = Vec::new();
    let mut left_edge: Vec<DVec3> = Vec::new();

    let mut corner_idx = 0;
    let mut i = 0;
    while i + 1 < positions.len() {
        let right_seg = &positions[i];
        let left_seg = &positions[i + 1];

        // 首段直接整段并入；后续段跳过首点以免与上一段/角重复。
        if i == 0 {
            right_edge.extend_from_slice(right_seg);
            left_edge.extend_from_slice(left_seg);
        } else {
            // 跳过角连接处重复的首/尾点。
            if right_seg.len() > 1 {
                right_edge.extend_from_slice(&right_seg[1..]);
            }
            if left_seg.len() > 1 {
                left_edge.extend_from_slice(&left_seg[1..]);
            }
        }

        // 插入角位置。
        if corner_idx < corners.len() {
            let corner = &corners[corner_idx];
            if let Some(ref lp) = corner.left_positions {
                left_edge.extend_from_slice(lp);
            }
            if let Some(ref rp) = corner.right_positions {
                right_edge.extend_from_slice(rp);
            }
            corner_idx += 1;
        }

        i += 2;
    }

    (right_edge, left_edge)
}

/// 生成一个走廊几何（扁平、非拉伸）。
///
/// 映射到 CesiumJS `CorridorGeometry.createGeometry`。
///
/// # 参数
/// - `options`：走廊选项（中心线、宽度、高度、角风格等）。
/// - `vf`：顶点格式，控制是否生成法线/切线/UV。
///
/// # 返回
/// 右缘与左缘之间拉成三角条带的 `GeometryData`；退化输入返回空几何。
pub fn corridor_geometry(options: &CorridorOptions, vf: VertexFormat) -> GeometryData {
    let ellipsoid = &options.ellipsoid;
    let width = options.width;

    // 将位置缩放到表面并去除重复项。
    let mut positions: Vec<DVec3> = options
        .positions
        .iter()
        .map(|&p| ellipsoid.scale_to_geodetic_surface(p).unwrap_or(p))
        .collect();
    positions.dedup_by(|a, b| {
        (a.x - b.x).abs() <= crate::math_utils::EPSILON10
            && (a.y - b.y).abs() <= crate::math_utils::EPSILON10
            && (a.z - b.z).abs() <= crate::math_utils::EPSILON10
    });

    // 少于 2 点或非正宽度无法成带，直接返回空几何。
    if positions.len() < 2 || width <= 0.0 {
        return empty_geometry();
    }

    let (computed_positions, corners) = compute_corridor_positions(
        &positions,
        width,
        options.granularity,
        options.corner_type,
        ellipsoid,
    );

    let (right_edge, left_edge) = assemble_edges(&computed_positions, &corners);

    // 两侧边缘长度；任一不足 2 则无法成带。
    let right_count = right_edge.len();
    let left_count = left_edge.len();
    if right_count < 2 || left_count < 2 {
        return empty_geometry();
    }

    // 输出顶点总数 = 右缘 + 左缘；据此预分配各属性缓冲的容量。
    let total_verts = right_count + left_count;
    // 逐属性输出缓冲：位置必有，其余按 vf 标志决定是否分配。
    let mut pos_out: Vec<[f64; 3]> = Vec::with_capacity(total_verts);
    let mut normals_out: Option<Vec<[f64; 3]>> = if vf.normal { Some(Vec::with_capacity(total_verts)) } else { None };
    let mut tangents_out: Option<Vec<[f64; 3]>> = if vf.tangent { Some(Vec::with_capacity(total_verts)) } else { None };
    let mut bitangents_out: Option<Vec<[f64; 3]>> = if vf.bitangent { Some(Vec::with_capacity(total_verts)) } else { None };
    let mut st_out: Option<Vec<[f64; 2]>> = if vf.st { Some(Vec::with_capacity(total_verts)) } else { None };

    // 右侧边缘顶点：先抬升到 height，再按 vf 逐属性写入输出缓冲。
    // U 沿弧长从 0 递增到 1，V 固定为 0（右缘）。
    let right_st = if right_count > 1 { 1.0 / (right_count - 1) as f64 } else { 1.0 };
    for (idx, p) in right_edge.iter().enumerate() {
        let raised = raise_to_height(*p, options.height, ellipsoid);
        pos_out.push([raised.x, raised.y, raised.z]);
        if let Some(ref mut n) = normals_out {
            let normal = ellipsoid.geodetic_surface_normal(raised).unwrap_or(DVec3::Z);
            n.push([normal.x, normal.y, normal.z]);
        }
        if let Some(ref mut t) = tangents_out {
            let tangent = compute_tangent(&right_edge, idx, ellipsoid);
            t.push([tangent.x, tangent.y, tangent.z]);
        }
        if let Some(ref mut b) = bitangents_out {
            let normal = ellipsoid.geodetic_surface_normal(raised).unwrap_or(DVec3::Z);
            let tangent = compute_tangent(&right_edge, idx, ellipsoid);
            let bitangent = normal.cross(tangent).normalize_or(DVec3::Y);
            b.push([bitangent.x, bitangent.y, bitangent.z]);
        }
        if let Some(ref mut st) = st_out {
            st.push([idx as f64 * right_st, 0.0]);
        }
    }

    // 左侧边缘顶点（为保持一致的绕序而反转）。
    // U 沿弧长从 1 递减到 0，V 固定为 1（左缘），与右缘配对成条带。
    let left_st = if left_count > 1 { 1.0 / (left_count - 1) as f64 } else { 1.0 };
    for (idx, p) in left_edge.iter().enumerate() {
        let raised = raise_to_height(*p, options.height, ellipsoid);
        pos_out.push([raised.x, raised.y, raised.z]);
        if let Some(ref mut n) = normals_out {
            let normal = ellipsoid.geodetic_surface_normal(raised).unwrap_or(DVec3::Z);
            n.push([normal.x, normal.y, normal.z]);
        }
        if let Some(ref mut t) = tangents_out {
            let tangent = compute_tangent(&left_edge, idx, ellipsoid);
            t.push([tangent.x, tangent.y, tangent.z]);
        }
        if let Some(ref mut b) = bitangents_out {
            let normal = ellipsoid.geodetic_surface_normal(raised).unwrap_or(DVec3::Z);
            let tangent = compute_tangent(&left_edge, idx, ellipsoid);
            let bitangent = normal.cross(tangent).normalize_or(DVec3::Y);
            b.push([bitangent.x, bitangent.y, bitangent.z]);
        }
        if let Some(ref mut st) = st_out {
            st.push([(left_count - 1 - idx) as f64 * left_st, 1.0]);
        }
    }

    // 三角剖分：在右侧与左侧边缘之间拉成条带。
    // 条带数取两侧边缘较短者，逐对顶点连成四边形（两个三角形）。
    let strip_count = right_count.min(left_count);
    let mut indices: Vec<u32> = Vec::with_capacity((strip_count - 1) * 6);
    for i in 0..strip_count - 1 {
        let r0 = i as u32;
        let r1 = (i + 1) as u32;
        let l0 = (right_count + i) as u32;
        let l1 = (right_count + i + 1) as u32;
        indices.extend_from_slice(&[l0, r0, l1]);
        indices.extend_from_slice(&[l1, r0, r1]);
    }

    // 由全部输出顶点拟合包围球，供剔除与相交测试使用。
    let bounding_sphere = BoundingSphere::from_points(
        &pos_out.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    GeometryData {
        positions: pos_out,
        normals: normals_out,
        tex_coords: st_out,
        tangents: tangents_out,
        bitangents: bitangents_out,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 生成一个走廊线框几何（绕走廊一周的线循环）。
///
/// # 参数
/// - `options`：走廊选项；边缘位置计算同 `corridor_geometry`。
///
/// # 返回
/// 以线段闭合环路勾勒走廊轮廓的 `GeometryData`（`Lines` 拓扑）。
pub fn corridor_outline_geometry(options: &CorridorOptions) -> GeometryData {
    let ellipsoid = &options.ellipsoid;
    let width = options.width;

    let mut positions: Vec<DVec3> = options
        .positions
        .iter()
        .map(|&p| ellipsoid.scale_to_geodetic_surface(p).unwrap_or(p))
        .collect();
    positions.dedup_by(|a, b| {
        (a.x - b.x).abs() <= crate::math_utils::EPSILON10
            && (a.y - b.y).abs() <= crate::math_utils::EPSILON10
            && (a.z - b.z).abs() <= crate::math_utils::EPSILON10
    });

    if positions.len() < 2 || width <= 0.0 {
        return empty_geometry_lines();
    }

    let (computed_positions, corners) = compute_corridor_positions(
        &positions,
        width,
        options.granularity,
        options.corner_type,
        ellipsoid,
    );

    let (right_edge, left_edge) = assemble_edges(&computed_positions, &corners);

    // 线框：右侧边缘前向 + 左侧边缘前向（反向回溯）。
    let mut pos_out: Vec<[f64; 3]> = Vec::new();
    for p in &right_edge {
        let raised = raise_to_height(*p, options.height, ellipsoid);
        pos_out.push([raised.x, raised.y, raised.z]);
    }
    // 左侧边缘逆序以构成环路。
    for p in left_edge.iter().rev() {
        let raised = raise_to_height(*p, options.height, ellipsoid);
        pos_out.push([raised.x, raised.y, raised.z]);
    }

    // 线框索引：相邻顶点成对连线，共 n 段（含末尾闭合）。
    let n = pos_out.len();
    let mut indices: Vec<u32> = Vec::with_capacity(n * 2);
    for i in 0..n - 1 {
        indices.push(i as u32);
        indices.push((i + 1) as u32);
    }
    // 闭合环路。
    indices.push((n - 1) as u32);
    indices.push(0);

    let bounding_sphere = BoundingSphere::from_points(
        &pos_out.iter().map(|p| DVec3::new(p[0], p[1], p[2])).collect::<Vec<_>>(),
    );

    GeometryData {
        positions: pos_out,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 沿椭球地表法线将点抬升给定高度；高度近 0 时原样返回。
///
/// # 参数
/// - `p`：表面上（或附近）的点。
/// - `height`：相对椭球的抬升高度（米）。
/// - `ellipsoid`：参考椭球。
fn raise_to_height(p: DVec3, height: f64, ellipsoid: &Ellipsoid) -> DVec3 {
    if height.abs() < f64::EPSILON {
        return p;
    }
    let normal = ellipsoid.geodetic_surface_normal(p).unwrap_or(DVec3::Z);
    p + normal * height
}

/// 以中心差分估计边缘上某顶点的切线方向。
///
/// # 参数
/// - `edge`：边缘顶点序列。
/// - `idx`：目标顶点下标。
/// - `_ellipsoid`：保留参数（当前未用）。
fn compute_tangent(edge: &[DVec3], idx: usize, _ellipsoid: &Ellipsoid) -> DVec3 {
    let next = if idx + 1 < edge.len() { edge[idx + 1] } else { edge[idx] };
    let prev = if idx > 0 { edge[idx - 1] } else { edge[idx] };
    (next - prev).normalize_or(DVec3::X)
}

/// 构造一个位置/索引皆空的三角形几何（退化输入的兜底返回值）。
fn empty_geometry() -> GeometryData {
    GeometryData {
        positions: Vec::new(),
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: Vec::new(),
        bounding_sphere: BoundingSphere::default(),
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 构造一个位置/索引皆空的线段几何（线框退化输入的兜底返回值）。
fn empty_geometry_lines() -> GeometryData {
    GeometryData {
        positions: Vec::new(),
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices: Vec::new(),
        bounding_sphere: BoundingSphere::default(),
        primitive_type: PrimitiveType::Lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartographic::Cartographic;

    fn corridor_opts() -> CorridorOptions {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-72.0, 40.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(-70.0, 35.0, 0.0)),
        ];
        CorridorOptions {
            positions,
            width: 100_000.0,
            height: 0.0,
            granularity: std::f64::consts::PI / 180.0,
            corner_type: CornerType::Rounded,
            ellipsoid: ell,
        }
    }

    /// 基础用例：两点直线走廊应生成三角条带并含法线与 UV。
    #[test]
    fn test_corridor_basic() {
        let geo = corridor_geometry(&corridor_opts(), VertexFormat::ALL);
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
        assert_eq!(geo.indices.len() % 3, 0);
        assert!(geo.normals.is_some());
        assert!(geo.tex_coords.is_some());
        // 应拥有右侧 + 左侧边缘顶点。
        assert!(geo.positions.len() >= 4);
    }

    /// 宽度校验：100km 宽走廊的包围球半径应大于 50km。
    #[test]
    fn test_corridor_width_correct() {
        let opts = corridor_opts();
        let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
        // 走廊应跨越大约 100km 宽度。
        // 检查包围球半径是否合理（> 50km）。
        assert!(geo.bounding_sphere.radius > 50_000.0);
    }

    /// 线框用例：轮廓应为闭合环路且索引成对、均落在顶点范围内。
    #[test]
    fn test_corridor_outline() {
        let geo = corridor_outline_geometry(&corridor_opts());
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
        assert_eq!(geo.indices.len() % 2, 0);
        let n = geo.positions.len() as u32;
        for &idx in &geo.indices {
            assert!(idx < n);
        }
    }

    /// 退化输入：中心线不足 2 点时返回空几何。
    #[test]
    fn test_corridor_too_few_positions() {
        let ell = Ellipsoid::WGS84;
        let opts = CorridorOptions {
            positions: vec![ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0))],
            width: 1000.0,
            ..Default::default()
        };
        let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.is_empty());
    }

    /// 退化输入：宽度为 0 时返回空几何。
    #[test]
    fn test_corridor_zero_width() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
        ];
        let opts = CorridorOptions {
            positions,
            width: 0.0,
            ..Default::default()
        };
        let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(geo.positions.is_empty());
    }

    /// 带转弯用例：斜接角风格的三点折线走廊应生成合法三角索引。
    #[test]
    fn test_corridor_with_corner() {
        let ell = Ellipsoid::WGS84;
        let positions = vec![
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 0.0, 0.0)),
            ell.cartographic_to_cartesian(&Cartographic::from_degrees(1.0, 1.0, 0.0)),
        ];
        let opts = CorridorOptions {
            positions,
            width: 50_000.0,
            corner_type: CornerType::Mitered,
            ..Default::default()
        };
        let geo = corridor_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(!geo.positions.is_empty());
        assert_eq!(geo.indices.len() % 3, 0);
    }
}
