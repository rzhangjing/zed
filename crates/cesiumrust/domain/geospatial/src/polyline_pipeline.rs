//! 折线流水线 —— 带高度插值的弧细分。
//!
//! 核心例程
//! [`generate_arc`] 将一条折线细分为椭球上的大地线弧，
//! 并将每个生成的点抬升到（逐顶点插值的）高度。这是
//! 围墙、走廊和折线几何的基础。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::needless_range_loop)]
use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use crate::geodesic::EllipsoidGeodesic;
use crate::math_utils::chord_length;
use crate::ray::{line_segment_plane, Plane};
use glam::{DMat4, DVec3};

/// 默认粒度：一度（以弧度计）（CesiumJS `RADIANS_PER_DEGREE`）。
pub const DEFAULT_GRANULARITY: f64 = std::f64::consts::PI / 180.0;

/// 一个线段需细分多少段，才能使每条弦都不超过 `min_distance`。
///
/// 映射到 `PolylinePipeline.numberOfPoints`。
///
/// 取两端点直线距离除以最小弦长后向上取整，至少为 1。
pub fn number_of_points(p0: DVec3, p1: DVec3, min_distance: f64) -> usize {
    let distance = p0.distance(p1);
    (distance / min_distance).ceil() as usize
}

/// 在 `h0` 和 `h1` 之间将高度线性细分为 `num_points` 个采样。
///
/// 映射到私有的 `subdivideHeights`。
///
/// 若两端高度几乎相等则直接全部填 `h0`；否则按等步长从 h0 线性插值到 h1。
fn subdivide_heights(num_points: usize, h0: f64, h1: f64) -> Vec<f64> {
    let mut heights = vec![0.0; num_points];
    if (h0 - h1).abs() < f64::EPSILON {
        heights.fill(h0);
        return heights;
    }
    let d_height = h1 - h0;
    let height_per_vertex = d_height / num_points as f64;
    for (i, h) in heights.iter_mut().enumerate() {
        *h = h0 + i as f64 * height_per_vertex;
    }
    heights
}

/// 从 `p0` 到 `p1` 生成单条笛卡尔弧（包含 `p0`，不包含
/// `p1`），将结果追加到 `out`。返回追加的点数。
///
/// 映射到私有的 `generateCartesianArc`。
///
/// # 参数
/// - `p0`、`p1`：线段两端点（地心固定系）。
/// - `min_distance`：每条弦的最大长度（米）。
/// - `ellipsoid`：参考椭球。
/// - `h0`、`h1`：两端高度（米），沿弧线性插值。
/// - `out`：输出顶点缓冲（追写）。
///
/// # 返回
/// 本段追加的点数 `num_points`（至少 1）。
fn generate_cartesian_arc(
    p0: DVec3,
    p1: DVec3,
    min_distance: f64,
    ellipsoid: &Ellipsoid,
    h0: f64,
    h1: f64,
    out: &mut Vec<DVec3>,
) -> usize {
    let first = ellipsoid.scale_to_geodetic_surface(p0).unwrap_or(p0);
    let last = ellipsoid.scale_to_geodetic_surface(p1).unwrap_or(p1);
    let num_points = number_of_points(p0, p1, min_distance);

    let start = ellipsoid.cartesian_to_cartographic(first).unwrap_or_default();
    let end = ellipsoid.cartesian_to_cartographic(last).unwrap_or_default();
    let heights = subdivide_heights(num_points, h0, h1);

    let geodesic = EllipsoidGeodesic::new(start, end, ellipsoid);
    let surface_distance_between_points = geodesic.surface_distance() / num_points as f64;

    // 位于 h0 的首点。
    let mut start_carto = start;
    start_carto.height = h0;
    out.push(ellipsoid.cartographic_to_cartesian(&start_carto));

    for (i, height) in heights.iter().enumerate().skip(1) {
        let mut carto =
            geodesic.interpolate_using_surface_distance(i as f64 * surface_distance_between_points);
        carto.height = *height;
        out.push(ellipsoid.cartographic_to_cartesian(&carto));
    }

    num_points
}

/// [`generate_arc`] 的选项。
pub struct ArcOptions<'a> {
    /// 折线的位置。
    pub positions: &'a [DVec3],
    /// 逐顶点高度。`None` 表示每个顶点高度为 0。
    pub heights: Option<&'a [f64]>,
    /// 角度粒度（弧度）。
    pub granularity: f64,
    /// 参考椭球。
    pub ellipsoid: &'a Ellipsoid,
}

/// 将一条折线细分为大地线弧，并将每个点抬升到其
/// （插值后的）高度。
///
/// 映射到 `PolylinePipeline.generateArc`。
///
/// # 参数
/// - `options`：[`ArcOptions`]，含位置/逐顶点高度/粒度/椭球。
///
/// # 返回
/// 沿大地线细分并抬升高度后的点列（地心固定系）；空输入返回空。单一位置时仅抬升到对应高度。
pub fn generate_arc(options: &ArcOptions) -> Vec<DVec3> {
    let positions = options.positions;
    let ellipsoid = options.ellipsoid;
    let length = positions.len();

    if length < 1 {
        return Vec::new();
    }

    let height_at = |i: usize| -> f64 {
        match options.heights {
            Some(h) => h[i],
            None => 0.0,
        }
    };

    if length == 1 {
        let mut p = ellipsoid.scale_to_geodetic_surface(positions[0]).unwrap_or(positions[0]);
        let height = height_at(0);
        if height != 0.0 {
            let n = ellipsoid.geodetic_surface_normal(p).unwrap_or(DVec3::Z);
            p += n * height;
        }
        return vec![p];
    }

    let min_distance = chord_length(options.granularity, ellipsoid.maximum_radius());

    let mut result = Vec::new();
    for i in 0..length - 1 {
        let p0 = positions[i];
        let p1 = positions[i + 1];
        let h0 = height_at(i);
        let h1 = height_at(i + 1);
        generate_cartesian_arc(p0, p1, min_distance, ellipsoid, h0, h1, &mut result);
    }

    // 精确地追加最后一个点。
    let last_point = positions[length - 1];
    let mut carto = ellipsoid
        .cartesian_to_cartographic(last_point)
        .unwrap_or(Cartographic::ZERO);
    carto.height = height_at(length - 1);
    result.push(ellipsoid.cartographic_to_cartesian(&carto));

    result
}

/// [`wrap_longitude`] 的结果：拆分后的折线位置与各段长度。
pub struct WrapLongitudeResult {
    /// 位置，在国际日期变更线交叉处可能带有额外的点。
    pub positions: Vec<DVec3>,
    /// 每段中的位置数量。
    pub lengths: Vec<usize>,
}

/// 将一条折线拆分为若干段，使其不跨越 ±180 度经线。
///
/// 映射到 `PolylinePipeline.wrapLongitude`。
///
/// # 参数
/// - `positions`：折线顶点（地心固定系）。
/// - `model_matrix`：可选模型矩阵，用于将顶点变回局部坐标系后再判断与国际日期变更线的相交。
///
/// # 返回
/// [`WrapLongitudeResult`]：在交叉处插入两个错位点后重新拼接的位置列，及各段的顶点数。
pub fn wrap_longitude(positions: &[DVec3], model_matrix: Option<&DMat4>) -> WrapLongitudeResult {
    let mut cartesians: Vec<DVec3> = Vec::new();
    let mut segments: Vec<usize> = Vec::new();

    if positions.is_empty() {
        return WrapLongitudeResult {
            positions: cartesians,
            lengths: segments,
        };
    }

    let model = model_matrix.copied().unwrap_or(DMat4::IDENTITY);
    let inverse_model = model.inverse();

    let origin = (inverse_model * DVec3::ZERO.extend(1.0)).truncate();
    // 构造两个参考平面：xz 平面（法线沿 Y）与 yz 平面（即国际日期变更线所在平面）。
    let xz_normal = (inverse_model * DVec3::Y.extend(0.0)).truncate().normalize();
    let xz_plane = Plane::from_point_normal(origin, xz_normal);
    let yz_normal = (inverse_model * DVec3::X.extend(0.0)).truncate().normalize();
    let yz_plane = Plane::from_point_normal(origin, yz_normal);

    let mut count = 1usize;
    cartesians.push(positions[0]);
    let mut prev = positions[0];

    for i in 1..positions.len() {
        let cur = positions[i];

        // 若任一端点位于 yz 平面的负侧，则与国际日期变更线相交
        if yz_plane.point_distance(prev) < 0.0 || yz_plane.point_distance(cur) < 0.0 {
            // 并且与 xz 平面相交
            if let Some(intersection) = line_segment_plane(prev, cur, &xz_plane) {
                // 将 xz 平面上的点略微偏移、远离该平面
                // 两个错开点分属变更线两侧，使拆分后的两段均不跨线。
                let mut offset = xz_normal * 5.0e-9;
                if xz_plane.point_distance(prev) < 0.0 {
                    offset = -offset;
                }

                cartesians.push(intersection + offset);
                segments.push(count + 1);

                cartesians.push(intersection - offset);
                count = 1;
            }
        }

        cartesians.push(cur);
        count += 1;
        prev = cur;
    }

    segments.push(count);

    WrapLongitudeResult {
        positions: cartesians,
        lengths: segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math_utils::to_radians;

    #[test]
    /// 验证段数按弦长向上取整（ceil(10/3)=4、ceil(10/5)=2）。
    fn test_number_of_points() {
        let p0 = DVec3::new(0.0, 0.0, 0.0);
        let p1 = DVec3::new(10.0, 0.0, 0.0);
        assert_eq!(number_of_points(p0, p1, 3.0), 4); // ceil(10/3)
        assert_eq!(number_of_points(p0, p1, 5.0), 2); // ceil(10/5)
    }

    #[test]
    /// 验证单一位置时返回恰好一个点。
    fn test_generate_arc_single_point() {
        let ell = Ellipsoid::WGS84;
        let pos = ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
        let opts = ArcOptions {
            positions: &[pos],
            heights: None,
            granularity: DEFAULT_GRANULARITY,
            ellipsoid: &ell,
        };
        let arc = generate_arc(&opts);
        assert_eq!(arc.len(), 1);
    }

    #[test]
    /// 验证生成的弧保留首尾端点且均落在表面（高度~0）。
    fn test_generate_arc_endpoints_preserved() {
        let ell = Ellipsoid::WGS84;
        let p0 = ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
        let p1 = ell.cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 0.0, 0.0));
        let opts = ArcOptions {
            positions: &[p0, p1],
            heights: None,
            granularity: DEFAULT_GRANULARITY,
            ellipsoid: &ell,
        };
        let arc = generate_arc(&opts);
        assert!(arc.len() >= 3, "arc len {}", arc.len());
        // 首点靠近 p0，尾点靠近 p1。
        assert!((arc[0] - p0).length() < 1.0);
        assert!((arc[arc.len() - 1] - p1).length() < 1.0);
        // 所有点都在表面上（高度 ~ 0）。
        for p in &arc {
            let c = ell.cartesian_to_cartographic(*p).unwrap();
            assert!(c.height.abs() < 1e-3, "height {}", c.height);
        }
    }

    #[test]
    /// 验证带高度输入时每个点都抬升到指定高度（~1000m）。
    fn test_generate_arc_with_heights() {
        let ell = Ellipsoid::WGS84;
        let p0 = ell.cartographic_to_cartesian(&Cartographic::from_degrees(0.0, 0.0, 0.0));
        let p1 = ell.cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 0.0, 0.0));
        let heights = [1000.0, 1000.0];
        let opts = ArcOptions {
            positions: &[p0, p1],
            heights: Some(&heights),
            granularity: DEFAULT_GRANULARITY,
            ellipsoid: &ell,
        };
        let arc = generate_arc(&opts);
        // 每个点都应位于 ~1000 m 高度。
        for p in &arc {
            let c = ell.cartesian_to_cartographic(*p).unwrap();
            assert!((c.height - 1000.0).abs() < 1.0, "height {}", c.height);
        }
    }

    #[test]
    /// 验证同纬度非赤道两点间的大地线中点向极点弯曲（纬度 > 45°）。
    fn test_generate_arc_geodesic_not_linear() {
        // 同一纬度（偏离赤道）两点之间的大地线，相对于等纬度线会向
        // 极点弯曲。
        let ell = Ellipsoid::WGS84;
        let p0 = ell.cartographic_to_cartesian(&Cartographic::from_degrees(-10.0, 45.0, 0.0));
        let p1 = ell.cartographic_to_cartesian(&Cartographic::from_degrees(10.0, 45.0, 0.0));
        let opts = ArcOptions {
            positions: &[p0, p1],
            heights: None,
            granularity: to_radians(0.5),
            ellipsoid: &ell,
        };
        let arc = generate_arc(&opts);
        // 大圆的中点纬度应 > 45 度。
        let mid = arc[arc.len() / 2];
        let mid_carto = ell.cartesian_to_cartographic(mid).unwrap();
        assert!(
            mid_carto.latitude > to_radians(45.0),
            "mid lat {}",
            mid_carto.latitude
        );
    }
}
