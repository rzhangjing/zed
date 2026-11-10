//! 椭球表面上的椭圆 / 圆形几何。
//!
//! 椭圆按自东→西的“列”进行剖分：
//! 首列和末列各含一个位置（最东/最西点），而每个内部列含偶数个
//! 位置，形成一种类菱形扇形布局。
//!
//! 本模块先由 [`compute_ellipse_positions`] 依参数角采样出椭圆边界与内部填充
//! 点，再由 [`top_indices`] 依据列布局生成三角形索引；[`ellipse_geometry`] 组装
//! 出实心的三角形几何（含可选法线与 UV），[`ellipse_outline_geometry`] 则沿外
//! 边界环路输出线框几何。圆形是半长轴等于半短轴的特例。所有点最终经
//! [`raise_positions_to_height`] 抬升到请求高度。

use crate::bounding::BoundingSphere;
use crate::ellipsoid::Ellipsoid;
use crate::geometry::{GeometryData, PrimitiveType, VertexFormat};
use crate::projection::{GeographicProjection, MapProjection};
use glam::{DMat3, DQuat, DVec3};

/// 计算椭圆边界上的单个点（`pointOnEllipsoid`）。
///
/// 给定参数角
/// `theta`，它通过将中心的单位位置矢量绕局部东/北平面内的一根轴旋转
/// 椭圆在 `theta` 处的角半径，找到位于椭圆边界上、椭球表面的点。
///
/// # 参数
/// - `theta`：参数角（弧度），描述椭圆上的采样位置。
/// - `rotation`：椭圆绕中心的旋转角（弧度）。
/// - `north_vec`/`east_vec`：中心处的局部北/东单位方向。
/// - `a_sqr`/`b_sqr`/`ab`：半轴平方 a²、b² 与乘积 a·b。
/// - `mag`：中心位置矢量的长度（米）。
/// - `unit_pos`：中心处的单位位置矢量。
///
/// # 返回
/// 椭圆边界上、位于椭球表面的点（笛卡尔坐标）。
#[allow(clippy::too_many_arguments)]
fn point_on_ellipsoid(
    theta: f64,
    rotation: f64,
    north_vec: DVec3,
    east_vec: DVec3,
    a_sqr: f64,
    ab: f64,
    b_sqr: f64,
    mag: f64,
    unit_pos: DVec3,
) -> DVec3 {
    let azimuth = theta + rotation;

    // 旋转轴落在当地东/北平面内，沿方位角方向。
    let rot_axis = east_vec * azimuth.cos() + north_vec * azimuth.sin();

    let cos_theta_squared = theta.cos() * theta.cos();
    let sin_theta_squared = theta.sin() * theta.sin();

    let radius = ab / (b_sqr * cos_theta_squared + a_sqr * sin_theta_squared).sqrt();
    let angle = radius / mag;

    // 将位置矢量旋转到椭圆的边界。
    let unit_quat = DQuat::from_axis_angle(rot_axis.normalize(), angle);
    let rot_mtx = DMat3::from_quat(unit_quat);

    let mut result = rot_mtx * unit_pos;
    result = result.normalize() * mag;
    result
}

/// [`compute_ellipse_positions`] 的结果。
pub struct EllipsePositions {
    /// 填充位置（列优先剖分），若已请求。
    pub positions: Vec<[f64; 3]>,
    /// 第一象限中的点数（驱动剖分）。
    pub num_pts: usize,
    /// 外边界位置（有序环路），若已请求。
    pub outer_positions: Vec<[f64; 3]>,
}

/// 计算构成椭圆的那些位置（`computeEllipsePositions`）。
///
/// * `semi_minor_axis` / `semi_major_axis` – 椭圆半径（米）。
/// * `rotation` – 椭圆绕其中心的旋转（弧度）。
/// * `center` – 中心位置（笛卡尔，位于/靠近椭球）。
/// * `granularity` – 角度粒度（弧度）；内部会乘以 8。
/// * `add_fill_positions` – 生成填充剖分位置。
/// * `add_edge_positions` – 生成外边界环路位置。
///
/// # 算法
/// 沿参数角自北向南逐列采样：先取东侧半部的点并对每列在边界两点间线性
/// 插值填充内部点，再以镜像参数角遍历西侧半部，首末各补上最南/最北的极点。
/// `add_fill_positions` 控制是否写入扇形填充位置，`add_edge_positions` 控制
/// 是否从两端向中间填充外边界环路。
///
/// # 返回
/// [`EllipsePositions`]：填充位置、第一象限点数 `num_pts`，以及外边界环路。
pub fn compute_ellipse_positions(
    semi_minor_axis: f64,
    semi_major_axis: f64,
    rotation: f64,
    center: DVec3,
    granularity: f64,
    add_fill_positions: bool,
    add_edge_positions: bool,
) -> EllipsePositions {
    // 缩放角度增量会使沿椭圆边界的距离与粒度更紧密地匹配
    // （参见 CesiumJS 注释）。
    let granularity = granularity * 8.0;

    // 预计算半轴平方与乘积，供边界点公式反复使用。
    let a_sqr = semi_minor_axis * semi_minor_axis;
    let b_sqr = semi_major_axis * semi_major_axis;
    let ab = semi_major_axis * semi_minor_axis;

    let mag = center.length();

    // 以地轴方向叉乘中心矢量得到当地东向，再叉乘得北向；unit_pos 为当地天顶。
    let unit_pos = center.normalize();
    let east_vec = DVec3::Z.cross(center).normalize();
    let north_vec = unit_pos.cross(east_vec);

    // 第一象限中的点数。
    // 第一象限点数由四分之一圆的角度粒度向上取整决定。
    let mut num_pts = 1 + (std::f64::consts::FRAC_PI_2 / granularity).ceil() as usize;

    // 每列间的参数角步长。
    let delta_theta = std::f64::consts::FRAC_PI_2 / (num_pts - 1) as f64;
    let theta = std::f64::consts::FRAC_PI_2 - num_pts as f64 * delta_theta;
    if theta < 0.0 {
        num_pts -= (theta.abs() / delta_theta).ceil() as usize;
    }

    // 填充位置总数 = 2·n·(n+2)，据此预分配容量。
    let size = 2 * (num_pts * (num_pts + 2));
    let mut positions: Vec<[f64; 3]> = if add_fill_positions {
        Vec::with_capacity(size)
    } else {
        Vec::new()
    };

    // 外环路点数 = 4·n（四个象限各 n 点）。
    let outer_positions_length = num_pts * 4;
    // 外环路从两端向中间填充。
    let mut outer_positions: Vec<[f64; 3]> = if add_edge_positions {
        vec![[0.0; 3]; outer_positions_length]
    } else {
        Vec::new()
    };
    // 外环路双向填充：右端从末尾递减、左端从头递增。
    let mut outer_right_index = outer_positions_length; // 排他，递减
    let mut outer_left_index = 0usize;

    // 计算椭圆“东侧”半部的点。
    // 从最北点开始，沿参数角向南推进。
    let mut theta = std::f64::consts::FRAC_PI_2;
    let position = point_on_ellipsoid(
        theta, rotation, north_vec, east_vec, a_sqr, ab, b_sqr, mag, unit_pos,
    );
    if add_fill_positions {
        positions.push([position.x, position.y, position.z]);
    }
    if add_edge_positions {
        outer_right_index -= 1;
        outer_positions[outer_right_index] = [position.x, position.y, position.z];
    }

    theta = std::f64::consts::FRAC_PI_2 - delta_theta;
    // 自北向南遍历东侧各列。
    for i in 1..num_pts + 1 {
        let position = point_on_ellipsoid(
            theta, rotation, north_vec, east_vec, a_sqr, ab, b_sqr, mag, unit_pos,
        );
        let reflected_position = point_on_ellipsoid(
            std::f64::consts::PI - theta,
            rotation,
            north_vec,
            east_vec,
            a_sqr,
            ab,
            b_sqr,
            mag,
            unit_pos,
        );

        if add_fill_positions {
            positions.push([position.x, position.y, position.z]);

            // 该列内部点数（含两端）为偶数，逐段线性插值填充。
            let num_interior = 2 * i + 2;
            for j in 1..num_interior - 1 {
                let t = j as f64 / (num_interior - 1) as f64;
                let interior = position.lerp(reflected_position, t);
                positions.push([interior.x, interior.y, interior.z]);
            }

            positions.push([reflected_position.x, reflected_position.y, reflected_position.z]);
        }

        if add_edge_positions {
            outer_right_index -= 1;
            outer_positions[outer_right_index] = [position.x, position.y, position.z];
            outer_positions[outer_left_index] =
                [reflected_position.x, reflected_position.y, reflected_position.z];
            outer_left_index += 1;
        }

        theta = std::f64::consts::FRAC_PI_2 - (i + 1) as f64 * delta_theta;
    }

    // 计算椭圆“西侧”半部的点。
    for i in (2..=num_pts).rev() {
        let theta = std::f64::consts::FRAC_PI_2 - (i - 1) as f64 * delta_theta;

        let position = point_on_ellipsoid(
            -theta, rotation, north_vec, east_vec, a_sqr, ab, b_sqr, mag, unit_pos,
        );
        let reflected_position = point_on_ellipsoid(
            theta + std::f64::consts::PI,
            rotation,
            north_vec,
            east_vec,
            a_sqr,
            ab,
            b_sqr,
            mag,
            unit_pos,
        );

        if add_fill_positions {
            positions.push([position.x, position.y, position.z]);

            let num_interior = 2 * (i - 1) + 2;
            for j in 1..num_interior - 1 {
                let t = j as f64 / (num_interior - 1) as f64;
                let interior = position.lerp(reflected_position, t);
                positions.push([interior.x, interior.y, interior.z]);
            }

            positions.push([reflected_position.x, reflected_position.y, reflected_position.z]);
        }

        if add_edge_positions {
            outer_right_index -= 1;
            outer_positions[outer_right_index] = [position.x, position.y, position.z];
            outer_positions[outer_left_index] =
                [reflected_position.x, reflected_position.y, reflected_position.z];
            outer_left_index += 1;
        }
    }

    // 末尾补上最南点（参数角 -π/2）。
    let theta = -std::f64::consts::FRAC_PI_2;
    let position = point_on_ellipsoid(
        theta, rotation, north_vec, east_vec, a_sqr, ab, b_sqr, mag, unit_pos,
    );
    if add_fill_positions {
        positions.push([position.x, position.y, position.z]);
    }
    if add_edge_positions {
        outer_right_index -= 1;
        outer_positions[outer_right_index] = [position.x, position.y, position.z];
    }

    EllipsePositions {
        positions,
        num_pts,
        outer_positions,
    }
}

/// 为填充的椭圆剖分生成三角形索引（`topIndices`）。
///
/// 索引算术与
/// [`compute_ellipse_positions`] 产生的列布局相对应。
///
/// # 参数
/// - `num_pts`：第一象限的采样点数（决定列数与三角形总数）。
///
/// # 返回
/// 长度为 `12·(n·(n+1)) - 6` 的三角形索引数组（每三个构成一个三角形）。
pub fn top_indices(num_pts: usize) -> Vec<u32> {
    // 总三角形数 = 2 * (-1 + 4 * (n*(n+1)/2))；索引数 = 三角形数 * 3。
    let total = 12 * (num_pts * (num_pts + 1)) - 6;
    let mut indices: Vec<u32> = Vec::with_capacity(total);

    let mut prev_index: u32 = 0;
    let mut position_index: u32 = 1;

    // 北向量“右侧”的三角形（第一个扇形）。
    for _ in 0..3 {
        indices.push(position_index);
        position_index += 1;
        indices.push(prev_index);
        indices.push(position_index);
    }

    for i in 2..num_pts + 1 {
        position_index = (i * (i + 1) - 1) as u32;
        prev_index = ((i - 1) * i - 1) as u32;

        indices.push(position_index);
        position_index += 1;
        indices.push(prev_index);
        indices.push(position_index);

        let num_interior = 2 * i;
        for _ in 0..num_interior - 1 {
            indices.push(position_index);
            indices.push(prev_index);
            prev_index += 1;
            indices.push(prev_index);

            indices.push(position_index);
            position_index += 1;
            indices.push(prev_index);
            indices.push(position_index);
        }

        indices.push(position_index);
        position_index += 1;
        indices.push(prev_index);
        indices.push(position_index);
    }

    // 中间一列三角形的索引。
    let num_interior = num_pts * 2;
    position_index += 1;
    prev_index += 1;
    for _ in 0..num_interior - 1 {
        indices.push(position_index);
        indices.push(prev_index);
        prev_index += 1;
        indices.push(prev_index);

        indices.push(position_index);
        position_index += 1;
        indices.push(prev_index);
        indices.push(position_index);
    }

    indices.push(position_index);
    indices.push(prev_index);
    prev_index += 1;
    indices.push(prev_index);

    indices.push(position_index);
    position_index += 1;
    indices.push(prev_index);
    prev_index += 1;
    indices.push(prev_index);

    // 反转过程，生成北向量“左侧”的索引。
    prev_index += 1;
    // 镜像列序生成西侧（北向左侧）三角形的索引。
    for i in (2..=num_pts - 1).rev() {
        indices.push(prev_index);
        prev_index += 1;
        indices.push(prev_index);
        indices.push(position_index);

        let num_interior = 2 * i;
        for _ in 0..num_interior - 1 {
            indices.push(position_index);
            indices.push(prev_index);
            prev_index += 1;
            indices.push(prev_index);

            indices.push(position_index);
            position_index += 1;
            indices.push(prev_index);
            indices.push(position_index);
        }

        indices.push(prev_index);
        prev_index += 1;
        indices.push(prev_index);
        prev_index += 1;
        indices.push(position_index);
        position_index += 1;
    }

    for _ in 0..3 {
        indices.push(prev_index);
        prev_index += 1;
        indices.push(prev_index);
        indices.push(position_index);
    }

    indices
}

/// 椭圆几何生成的选项。
pub struct EllipseOptions {
    /// 中心位置（笛卡尔）。
    pub center: DVec3,
    /// 半长轴（米）。
    pub semi_major_axis: f64,
    /// 半短轴（米）。
    pub semi_minor_axis: f64,
    /// 椭球。
    pub ellipsoid: Ellipsoid,
    /// 角度粒度（弧度）。
    pub granularity: f64,
    /// 椭球上方的高度（米）。
    pub height: f64,
    /// 椭圆绕其中心的旋转（弧度）。
    pub rotation: f64,
    /// 纹理坐标旋转（弧度）。
    pub st_rotation: f64,
}

impl Default for EllipseOptions {
    /// 默认椭圆选项：原点中心、单位半轴、WGS84、1 度粒度、无高度与旋转。
    fn default() -> Self {
        Self {
            center: DVec3::ZERO,
            semi_major_axis: 1.0,
            semi_minor_axis: 1.0,
            ellipsoid: Ellipsoid::WGS84,
            granularity: crate::math_utils::to_radians(1.0),
            height: 0.0,
            rotation: 0.0,
            st_rotation: 0.0,
        }
    }
}

/// 在椭球上生成一个实心椭圆几何。
///
/// 映射到 CesiumJS `EllipseGeometry`。`CircleGeometry` 是
/// `semi_major_axis == semi_minor_axis` 的特殊情形。
///
/// # 参数
/// - `options`：椭圆生成选项（中心、半轴、旋转、高度、粒度等）。
/// - `vf`：顶点格式，决定是否附带法线与纹理坐标。
///
/// # 返回
/// 三角形拓扑的 [`GeometryData`]；UV 由相对投影中心的偏移归一化得到。
pub fn ellipse_geometry(options: &EllipseOptions, vf: VertexFormat) -> GeometryData {
    let ellipsoid = options.ellipsoid;

    let cep = compute_ellipse_positions(
        options.semi_minor_axis,
        options.semi_major_axis,
        options.rotation,
        options.center,
        options.granularity,
        true,
        false,
    );
    let num_pts = cep.num_pts;

    // 将位置抬升到高度并计算属性。
    let positions = raise_positions_to_height(&cep.positions, &ellipsoid, options.height);

    let indices = top_indices(num_pts);

    // 用地理投影把大地坐标转为平面坐标，便于计算 UV 偏移。
    let projection = GeographicProjection::new(ellipsoid);
    let center_carto = ellipsoid
        .cartesian_to_cartographic(options.center)
        .unwrap_or_default();
    let projected_center = projection.project(&center_carto);

    // 按顶点格式决定是否分配 UV/法线缓冲。
    let mut tex_coords: Option<Vec<[f64; 2]>> = if vf.st { Some(Vec::new()) } else { None };
    let mut normals: Option<Vec<[f64; 3]>> = if vf.normal { Some(Vec::new()) } else { None };

    // 逐顶点按需求生成 UV 与法线。
    for p in &positions {
        let pos = DVec3::new(p[0], p[1], p[2]);

        if let Some(ref mut st) = tex_coords {
            let carto = ellipsoid.cartesian_to_cartographic(pos).unwrap_or_default();
            let projected = projection.project(&carto);
            // 相对投影中心的偏移按半轴归一化到 [0,1]，得到 UV。
            let rel = projected - projected_center;
            let u = (rel.x + options.semi_major_axis) / (2.0 * options.semi_major_axis);
            let v = (rel.y + options.semi_minor_axis) / (2.0 * options.semi_minor_axis);
            st.push([u, v]);
        }

        if let Some(ref mut n) = normals {
            let normal = ellipsoid.geodetic_surface_normal(pos).unwrap_or(DVec3::Z);
            n.push([normal.x, normal.y, normal.z]);
        }
    }

    // 包围球：中心抬升到高度，半径 = 半长轴。
    let bs_center = options
        .center
        + ellipsoid
            .geodetic_surface_normal(options.center)
            .unwrap_or(DVec3::Z)
            * options.height;
    let bounding_sphere = BoundingSphere::new(bs_center, options.semi_major_axis);

    GeometryData {
        positions,
        normals,
        tex_coords,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Triangles,
    }
}

/// 生成一个椭圆线框几何（线段序列）。
///
/// 映射到 CesiumJS `EllipseOutlineGeometry`。
///
/// # 参数
/// - `options`：椭圆生成选项，仅需外边界位置。
///
/// # 返回
/// 线段拓扑的 [`GeometryData`]：沿外环路相邻顶点两两连成闭合环。
pub fn ellipse_outline_geometry(options: &EllipseOptions) -> GeometryData {
    let ellipsoid = options.ellipsoid;

    let cep = compute_ellipse_positions(
        options.semi_minor_axis,
        options.semi_major_axis,
        options.rotation,
        options.center,
        options.granularity,
        false,
        true,
    );

    let positions = raise_positions_to_height(&cep.outer_positions, &ellipsoid, options.height);

    // 沿外环路的线循环。
    // 线循环：每个顶点连向下一个，末点回连首点。
    let n = positions.len();
    let mut indices: Vec<u32> = Vec::with_capacity(n * 2);
    for i in 0..n {
        indices.push(i as u32);
        indices.push(((i + 1) % n) as u32);
    }

    let bs_center = options
        .center
        + ellipsoid
            .geodetic_surface_normal(options.center)
            .unwrap_or(DVec3::Z)
            * options.height;
    let bounding_sphere = BoundingSphere::new(bs_center, options.semi_major_axis);

    GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Lines,
    }
}

/// 将位置抬升到椭球表面上方的给定高度（`raisePositionsToHeight`，非拉伸情形）。
///
/// # 参数
/// - `positions`：位于/靠近椭球表面的位置数组。
/// - `ellipsoid`：参考椭球，用于求法线。
/// - `height`：沿大地法线抬升的高度（米）。
///
/// # 返回
/// 每个位置先归算到大地表面再沿法线抬升后的新位置数组。
fn raise_positions_to_height(positions: &[[f64; 3]], ellipsoid: &Ellipsoid, height: f64) -> Vec<[f64; 3]> {
    positions
        .iter()
        .map(|p| {
            let pos = DVec3::new(p[0], p[1], p[2]);
            let surface = ellipsoid.scale_to_geodetic_surface(pos).unwrap_or(pos);
            let normal = ellipsoid.geodetic_surface_normal(surface).unwrap_or(DVec3::Z);
            let raised = surface + normal * height;
            [raised.x, raised.y, raised.z]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math_utils;

    fn equator_center() -> DVec3 {
        Ellipsoid::WGS84.cartographic_to_cartesian(&crate::cartographic::Cartographic::from_degrees(0.0, 0.0, 0.0))
    }

    /// 校验填充位置数与外环路数分别符合列公式 2·n·(n+2) 与 4·n。
    #[test]
    fn test_ellipse_positions_count() {
        let opts = EllipseOptions {
            center: equator_center(),
            semi_major_axis: 500_000.0,
            semi_minor_axis: 300_000.0,
            granularity: math_utils::to_radians(8.0),
            ..Default::default()
        };
        let cep = compute_ellipse_positions(
            opts.semi_minor_axis,
            opts.semi_major_axis,
            opts.rotation,
            opts.center,
            opts.granularity,
            true,
            true,
        );
        // 填充数必须符合列公式 2*n*(n+2)。
        assert_eq!(cep.positions.len(), 2 * cep.num_pts * (cep.num_pts + 2));
        // 外环路数必须为 4*n。
        assert_eq!(cep.outer_positions.len(), 4 * cep.num_pts);
    }

    /// 校验索引总数及索引上界随 num_pts 变化均正确。
    #[test]
    fn test_top_indices_count() {
        for num_pts in 2..8 {
            let indices = top_indices(num_pts);
            let expected = 12 * (num_pts * (num_pts + 1)) - 6;
            assert_eq!(indices.len(), expected, "num_pts={}", num_pts);
            // 所有索引都必须在填充位置范围内。
            let max_index = 2 * num_pts * (num_pts + 2);
            assert!(
                indices.iter().all(|&i| (i as usize) < max_index),
                "index out of range for num_pts={}",
                num_pts
            );
        }
    }

    /// 实心椭圆几何的法线/UV 应与位置逐顶点对应且类型为三角形。
    #[test]
    fn test_ellipse_geometry() {
        let opts = EllipseOptions {
            center: equator_center(),
            semi_major_axis: 500_000.0,
            semi_minor_axis: 300_000.0,
            granularity: math_utils::to_radians(8.0),
            ..Default::default()
        };
        let geo = ellipse_geometry(&opts, VertexFormat::ALL);
        assert_eq!(geo.positions.len(), geo.normals.as_ref().unwrap().len());
        assert_eq!(geo.positions.len(), geo.tex_coords.as_ref().unwrap().len());
        assert_eq!(geo.indices.len() % 3, 0);
        assert_eq!(geo.primitive_type, PrimitiveType::Triangles);
        // 包围球半径等于半长轴。
        assert!((geo.bounding_sphere.radius - 500_000.0).abs() < 1e-6);
    }

    /// 半长轴等于半短轴时退化为圆，仍应生成非空的位置与索引。
    #[test]
    fn test_circle_is_ellipse_special_case() {
        let opts = EllipseOptions {
            center: equator_center(),
            semi_major_axis: 400_000.0,
            semi_minor_axis: 400_000.0,
            granularity: math_utils::to_radians(8.0),
            ..Default::default()
        };
        let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(!geo.positions.is_empty());
        assert!(!geo.indices.is_empty());
    }

    /// 线框几何应为线段类型，且索引成对构成闭合环路。
    #[test]
    fn test_ellipse_outline_geometry() {
        let opts = EllipseOptions {
            center: equator_center(),
            semi_major_axis: 500_000.0,
            semi_minor_axis: 300_000.0,
            granularity: math_utils::to_radians(8.0),
            ..Default::default()
        };
        let geo = ellipse_outline_geometry(&opts);
        assert_eq!(geo.primitive_type, PrimitiveType::Lines);
        // 线索引成对出现并构成一个闭合环路。
        assert_eq!(geo.indices.len(), geo.positions.len() * 2);
        assert_eq!(geo.indices.len() % 2, 0);
    }

    /// 抬升到高度后，每个顶点的大地坐标高度应接近请求值。
    #[test]
    fn test_ellipse_positions_on_surface() {
        // 每个生成的位置都应（大致）位于抬升到请求
        // 高度的椭球表面上。
        let opts = EllipseOptions {
            center: equator_center(),
            semi_major_axis: 500_000.0,
            semi_minor_axis: 300_000.0,
            granularity: math_utils::to_radians(8.0),
            height: 1000.0,
            ..Default::default()
        };
        let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);
        for p in &geo.positions {
            let carto = Ellipsoid::WGS84
                .cartesian_to_cartographic(DVec3::new(p[0], p[1], p[2]))
                .unwrap();
            assert!((carto.height - 1000.0).abs() < 1.0, "height={}", carto.height);
        }
    }

    /// 每个三角形应引用两两不同的顶点且具有非零面积。
    #[test]
    fn test_ellipse_triangles_non_degenerate() {
        // 每个三角形都必须引用三个两两不同的顶点并具有
        // 非零面积，以确认列剖分有效。
        let opts = EllipseOptions {
            center: equator_center(),
            semi_major_axis: 500_000.0,
            semi_minor_axis: 300_000.0,
            granularity: math_utils::to_radians(8.0),
            ..Default::default()
        };
        let geo = ellipse_geometry(&opts, VertexFormat::POSITION_ONLY);
        assert!(!geo.indices.is_empty());
        for tri in geo.indices.as_chunks::<3>().0 {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            assert!(a != b && b != c && a != c, "degenerate triangle {:?}", tri);
            let pa = DVec3::from(geo.positions[a]);
            let pb = DVec3::from(geo.positions[b]);
            let pc = DVec3::from(geo.positions[c]);
            let area = (pb - pa).cross(pc - pa).length() * 0.5;
            assert!(area > 1e-6, "zero-area triangle {:?}", tri);
        }
    }
}
