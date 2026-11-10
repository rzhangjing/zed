//! PolygonPipeline —— 椭球上多边形网格的细分算法。
//!
//! 本模块把由索引描述的三角形网格在椭球（借助辅助球面近似）上递归细分，
//! 使相邻顶点间的地面距离不超过指定的角度粒度 `granularity`，从而在球面上
//! 得到近似均匀、适合渲染的多边形网格。核心流程：
//!
//! - 用栈存放待处理三角形，弹出后计算三边长度并取最长者；
//! - 若最长边超过目标弦长，则在中点把它一分为二，两个子三角形重新入栈；
//! - 否则该三角形已足够小，直接写入最终索引；
//! - 共享边通过 `edges` 哈希缓存，保证同一条边只被拆分一次、避免裂缝。
//!
//! 提供两种度量方式：`compute_subdivision` 以辅助球面弦长近似大圆弧；
//! `compute_rhumb_line_subdivision` 则以恒向线（等方位角航线）的真实表面
//! 距离度量，并在恒向线上插值中点。两者的网格拓扑处理完全一致，仅边长
//! 度量与中点求法不同。
//!
//! 两个函数都返回 `SubdivisionResult`，其中位置以 `[x0, y0, z0, ...]` 的
//! 扁平浮点数组存放，索引以 `u32` 三元组描述三角形；当输入带有 UV 时，
//! 纹理坐标同样按中点线性插值，保持与位置一一对应。粒度 `granularity`
//! 越小，输出的三角形越细密，相应地顶点与三角形数量也越多。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_is_multiple_of, clippy::len_zero)]
use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use crate::ellipsoid_rhumb_line::EllipsoidRhumbLine;
use crate::math_utils::chord_length;
use glam::{DVec2, DVec3};
use std::collections::HashMap;
use std::f64::consts::PI;

/// 一次细分操作的结果。
/// 映射到 PolygonPipeline.computeSubdivision 返回的 Geometry 对象
#[derive(Debug, Clone)]
pub struct SubdivisionResult {
    /// 扁平的位置值 [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f64>,
    /// 三角形索引
    pub indices: Vec<u32>,
    /// 可选的扁平纹理坐标 [u0, v0, u1, v1, ...]；仅当输入携带 UV 时为
    /// `Some`，其长度恒为位置数目的 2/3，与三角形逐顶点对齐。
    pub texcoords: Option<Vec<f64>>,
}

const RADIANS_PER_DEGREE: f64 = PI / 180.0;

/// 细分位置并将点抬升到椭球表面。
/// 映射到 `PolygonPipeline.computeSubdivision`
///
/// # 参数
/// * `ellipsoid` - 多边形所在的椭球
/// * `positions` - Cartesian3 位置数组
/// * `indices` - 三角形索引
/// * `texcoords` - 可选的纹理坐标
/// * `granularity` - 相邻细分之间的距离（弧度）（默认：RADIANS_PER_DEGREE）
///
/// # 算法
/// 采用“最长边中点分裂”的自适应细分：每次从栈中弹出一个三角形，计算
/// 其三条边在辅助球面上的长度平方，若最长者超过 `granularity` 对应的
/// 目标弦长，就在该边中点插入新顶点并把三角形拆成两个，重新入栈继续
/// 处理；否则三角形已足够精细，直接把索引写入结果。共享边借助 `edges`
/// 缓存，使同一条边只被拆分一次，相邻三角形因此能共用中点、不产生裂缝。
///
/// 由于以弦长平方作判据，比较时无需对边长开方，减少了三角函数与开方的
/// 开销；辅助球近似则在保持细分均匀性的同时，回避了直接在椭球面上求椭圆
/// 弧长的复杂度。
///
/// # 返回
/// 含扁平位置数组、三角形索引数组，以及（当输入带 UV 时）扁平纹理坐标的
/// `SubdivisionResult`。
pub fn compute_subdivision(
    ellipsoid: &Ellipsoid,
    positions: &[DVec3],
    indices: &[u32],
    texcoords: Option<&[DVec2]>,
    granularity: Option<f64>,
) -> SubdivisionResult {
    let granularity = granularity.unwrap_or(RADIANS_PER_DEGREE);
    let has_texcoords = texcoords.is_some();

    debug_assert!(indices.len() >= 3, "At least three indices are required");
    debug_assert!(indices.len() % 3 == 0, "Number of indices must be divisible by three");
    debug_assert!(granularity > 0.0, "Granularity must be greater than zero");

    // 需要（或可能需要）细分的三角形：以栈的形式存放，弹出即处理，
    // 拆分出的子三角形重新压回，直到栈空即全部细化完毕。
    let mut triangles: Vec<u32> = indices.to_vec();

    // 由边拆分产生的新位置会被追加到位置列表末尾。
    let mut subdivided_positions: Vec<f64> = Vec::with_capacity(positions.len() * 3);
    let mut subdivided_texcoords: Vec<f64> = if has_texcoords {
        Vec::with_capacity(positions.len() * 2)
    } else {
        Vec::new()
    };

    for item in positions {
        subdivided_positions.push(item.x);
        subdivided_positions.push(item.y);
        subdivided_positions.push(item.z);
    }
    if let Some(tcs) = texcoords {
        for tc in tcs {
            subdivided_texcoords.push(tc.x);
            subdivided_texcoords.push(tc.y);
        }
    }

    // 收敛后的最终索引：足够小的三角形直接写入此处。
    let mut subdivided_indices: Vec<u32> = Vec::new();

    // 用于确保共享的边不会被拆分多次：键为规范化（小,大）的顶点对，
    // 值为该边首次拆分产生的中点索引，后续遇到同一边直接复用。
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();

    // 以椭球最大半径构造辅助球；granularity（弧度）经 chord_length 换算成
    // 球面弦长，再取其平方作为拆分判据（比较时无需开方）。下面循环逐个弹出
    // 三角形，若其最长边超过该阈值就在中点拆分，否则视为已足够精细。
    let radius = ellipsoid.maximum_radius();
    let min_distance = chord_length(granularity, radius);
    let min_distance_sqrd = min_distance * min_distance;

    while triangles.len() > 0 {
        let i2 = triangles.pop().unwrap();
        let i1 = triangles.pop().unwrap();
        let i0 = triangles.pop().unwrap();

        let v0 = DVec3::new(
            subdivided_positions[(i0 * 3) as usize],
            subdivided_positions[(i0 * 3 + 1) as usize],
            subdivided_positions[(i0 * 3 + 2) as usize],
        );
        let v1 = DVec3::new(
            subdivided_positions[(i1 * 3) as usize],
            subdivided_positions[(i1 * 3 + 1) as usize],
            subdivided_positions[(i1 * 3 + 2) as usize],
        );
        let v2 = DVec3::new(
            subdivided_positions[(i2 * 3) as usize],
            subdivided_positions[(i2 * 3 + 1) as usize],
            subdivided_positions[(i2 * 3 + 2) as usize],
        );

        // 把三顶点归一化后缩放到辅助球面，得到用于测边的球面近似坐标，
        // 避免直接在椭球面上计算椭圆弧长带来的复杂性。
        let s0 = v0.normalize() * radius;
        let s1 = v1.normalize() * radius;
        let s2 = v2.normalize() * radius;

        // 三条边在辅助球面上的长度平方；后续取最大值以确定拆分哪条边。
        let g0 = (s0 - s1).length_squared();
        let g1 = (s1 - s2).length_squared();
        let g2 = (s2 - s0).length_squared();

        let max = g0.max(g1).max(g2);

        // 若最长边超阈值，则在其对应边的中点拆分，产生两个子三角形重新入栈；
        // i0-i1 边（g0）为最长时走此分支。
        if max > min_distance_sqrd {
            if g0 == max {
                let edge = (i0.min(i1), i0.max(i1));
                let mid_idx = if let Some(&idx) = edges.get(&edge) {
                    idx
                } else {
                    // 首次遇到该共享边：取 v0、v1 的欧氏中点追加到扁平位置缓冲，
                    // 若带 UV 则同样插值中点 UV；登记索引后复用，避免同一边重复拆分。
                    let mid = (v0 + v1) * 0.5;
                    subdivided_positions.push(mid.x);
                    subdivided_positions.push(mid.y);
                    subdivided_positions.push(mid.z);
                    let idx = (subdivided_positions.len() / 3 - 1) as u32;
                    edges.insert(edge, idx);

                    if has_texcoords {
                        let t0 = DVec2::new(
                            subdivided_texcoords[(i0 * 2) as usize],
                            subdivided_texcoords[(i0 * 2 + 1) as usize],
                        );
                        let t1 = DVec2::new(
                            subdivided_texcoords[(i1 * 2) as usize],
                            subdivided_texcoords[(i1 * 2 + 1) as usize],
                        );
                        let mid_tc = (t0 + t1) * 0.5;
                        subdivided_texcoords.push(mid_tc.x);
                        subdivided_texcoords.push(mid_tc.y);
                    }
                    idx
                };

                triangles.push(i0);
                triangles.push(mid_idx);
                triangles.push(i2);
                triangles.push(mid_idx);
                triangles.push(i1);
                triangles.push(i2);
            } else if g1 == max {
                let edge = (i1.min(i2), i1.max(i2));
                let mid_idx = if let Some(&idx) = edges.get(&edge) {
                    idx
                } else {
                    // g1 为最长边（i1-i2）：取 v1、v2 的中点，逻辑与 g0 分支一致。
                    let mid = (v1 + v2) * 0.5;
                    subdivided_positions.push(mid.x);
                    subdivided_positions.push(mid.y);
                    subdivided_positions.push(mid.z);
                    let idx = (subdivided_positions.len() / 3 - 1) as u32;
                    edges.insert(edge, idx);

                    if has_texcoords {
                        let t1 = DVec2::new(
                            subdivided_texcoords[(i1 * 2) as usize],
                            subdivided_texcoords[(i1 * 2 + 1) as usize],
                        );
                        let t2 = DVec2::new(
                            subdivided_texcoords[(i2 * 2) as usize],
                            subdivided_texcoords[(i2 * 2 + 1) as usize],
                        );
                        let mid_tc = (t1 + t2) * 0.5;
                        subdivided_texcoords.push(mid_tc.x);
                        subdivided_texcoords.push(mid_tc.y);
                    }
                    idx
                };

                triangles.push(i1);
                triangles.push(mid_idx);
                triangles.push(i0);
                triangles.push(mid_idx);
                triangles.push(i2);
                triangles.push(i0);
            } else {
                // g2 == max
                let edge = (i2.min(i0), i2.max(i0));
                let mid_idx = if let Some(&idx) = edges.get(&edge) {
                    idx
                } else {
                    // g2 为最长边（i2-i0）：取 v2、v0 的中点，逻辑与 g0 分支一致。
                    let mid = (v2 + v0) * 0.5;
                    subdivided_positions.push(mid.x);
                    subdivided_positions.push(mid.y);
                    subdivided_positions.push(mid.z);
                    let idx = (subdivided_positions.len() / 3 - 1) as u32;
                    edges.insert(edge, idx);

                    if has_texcoords {
                        let t2 = DVec2::new(
                            subdivided_texcoords[(i2 * 2) as usize],
                            subdivided_texcoords[(i2 * 2 + 1) as usize],
                        );
                        let t0 = DVec2::new(
                            subdivided_texcoords[(i0 * 2) as usize],
                            subdivided_texcoords[(i0 * 2 + 1) as usize],
                        );
                        let mid_tc = (t2 + t0) * 0.5;
                        subdivided_texcoords.push(mid_tc.x);
                        subdivided_texcoords.push(mid_tc.y);
                    }
                    idx
                };

                triangles.push(i2);
                triangles.push(mid_idx);
                triangles.push(i1);
                triangles.push(mid_idx);
                triangles.push(i0);
                triangles.push(i1);
            }
        } else {
            subdivided_indices.push(i0);
            subdivided_indices.push(i1);
            subdivided_indices.push(i2);
        }
    }

    SubdivisionResult {
        positions: subdivided_positions,
        indices: subdivided_indices,
        texcoords: if has_texcoords {
            Some(subdivided_texcoords)
        } else {
            None
        },
    }
}

/// 在恒向线上细分位置并将点抬升到椭球表面。
/// 映射到 `PolygonPipeline.computeRhumbLineSubdivision`
///
/// 与 `compute_subdivision` 的网格拓扑处理完全一致，区别仅在于边长度量与
/// 中点求法：这里以恒向线（等方位角航线，loxodrome）的真实表面距离衡量边
/// 长，并沿恒向线插值出中点，因此细分点沿恒向线而非大圆弧分布。
///
/// # 参数
/// - `ellipsoid`：多边形所在的椭球。
/// - `positions`：笛卡尔位置数组（Cartesian3）。
/// - `indices`：三角形索引，长度须为 3 的倍数。
/// - `texcoords`：可选的逐顶点纹理坐标。
/// - `granularity`：相邻细分点的目标间隔（弧度）。
///
/// # 返回
/// 沿恒向线细化后的 `SubdivisionResult`。
///
/// # 注意
/// 恒向线在接近两极或跨越经线时会显著偏离大圆，因而该细分得到的网格与
/// `compute_subdivision` 并不相同；它更适合需要保持恒定方位角语义的场景
/// （如按航向等分的剖面）。
pub fn compute_rhumb_line_subdivision(
    ellipsoid: &Ellipsoid,
    positions: &[DVec3],
    indices: &[u32],
    texcoords: Option<&[DVec2]>,
    granularity: Option<f64>,
) -> SubdivisionResult {
    let granularity = granularity.unwrap_or(RADIANS_PER_DEGREE);
    let has_texcoords = texcoords.is_some();

    debug_assert!(indices.len() >= 3, "At least three indices are required");
    debug_assert!(indices.len() % 3 == 0, "Number of indices must be divisible by three");
    debug_assert!(granularity > 0.0, "Granularity must be greater than zero");

    // 待细分的三角形栈：与 compute_subdivision 一样以 `Vec` 充当栈，每次弹出
    // 三个索引构成一个三角形，拆分出的子三角形重新压回，直到栈空。
    let mut triangles: Vec<u32> = indices.to_vec();

    // 细分过程中新增的顶点持续追加到该扁平位置缓冲末尾，最终一并输出。
    let mut subdivided_positions: Vec<f64> = Vec::with_capacity(positions.len() * 3);
    let mut subdivided_texcoords: Vec<f64> = if has_texcoords {
        Vec::with_capacity(positions.len() * 2)
    } else {
        Vec::new()
    };

    for item in positions {
        subdivided_positions.push(item.x);
        subdivided_positions.push(item.y);
        subdivided_positions.push(item.z);
    }
    if let Some(tcs) = texcoords {
        for tc in tcs {
            subdivided_texcoords.push(tc.x);
            subdivided_texcoords.push(tc.y);
        }
    }

    // 最终索引与共享边缓存（含义同 compute_subdivision）。
    let mut subdivided_indices: Vec<u32> = Vec::new();
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();

    // 同样以最大半径换算目标弦长，但后续比较的是恒向线的真实表面距离（米），
    // 而非辅助球上的弦长平方；因此这里保留了未平方的 `min_distance`。
    let radius = ellipsoid.maximum_radius();
    let min_distance = chord_length(granularity, radius);

    // 恒向线以表面距离（米）而非弦长平方为判据，故直接拿 `min_distance` 比较。
    // 三条边复用同一组占位端点构造恒向线对象，随后在循环内对每条边调用
    // set_end_points 重置，避免在细分过程中反复分配新的恒向线。
    let dummy_start = Cartographic::from_radians(0.0, 0.0, 0.0);
    let dummy_end = Cartographic::from_radians(0.0, 0.1, 0.0);
    let mut rhumb0 = EllipsoidRhumbLine::new(&dummy_start, &dummy_end, ellipsoid);
    let mut rhumb1 = EllipsoidRhumbLine::new(&dummy_start, &dummy_end, ellipsoid);
    let mut rhumb2 = EllipsoidRhumbLine::new(&dummy_start, &dummy_end, ellipsoid);

    // 弹出三角形，先把顶点转回大地坐标，再用三条占位恒向线逐一度量各边长度。
    while triangles.len() > 0 {
        let i2 = triangles.pop().unwrap();
        let i1 = triangles.pop().unwrap();
        let i0 = triangles.pop().unwrap();

        let v0 = DVec3::new(
            subdivided_positions[(i0 * 3) as usize],
            subdivided_positions[(i0 * 3 + 1) as usize],
            subdivided_positions[(i0 * 3 + 2) as usize],
        );
        let v1 = DVec3::new(
            subdivided_positions[(i1 * 3) as usize],
            subdivided_positions[(i1 * 3 + 1) as usize],
            subdivided_positions[(i1 * 3 + 2) as usize],
        );
        let v2 = DVec3::new(
            subdivided_positions[(i2 * 3) as usize],
            subdivided_positions[(i2 * 3 + 1) as usize],
            subdivided_positions[(i2 * 3 + 2) as usize],
        );

        // 恒向线版需在大地坐标下沿等方位角航线度量，故先把顶点转回经纬度。
        let c0 = ellipsoid.cartesian_to_cartographic(v0).unwrap();
        let c1 = ellipsoid.cartesian_to_cartographic(v1).unwrap();
        let c2 = ellipsoid.cartesian_to_cartographic(v2).unwrap();

        // 逐边重置占位恒向线的端点，取其表面距离作为该边的长度。
        rhumb0.set_end_points(&c0, &c1);
        let g0 = rhumb0.surface_distance();
        rhumb1.set_end_points(&c1, &c2);
        let g1 = rhumb1.surface_distance();
        rhumb2.set_end_points(&c2, &c0);
        let g2 = rhumb2.surface_distance();

        // 取三边恒向线长度的最大值，作为是否需要拆分该三角形的判据。
        let max = g0.max(g1).max(g2);

        // 最长恒向线边超过目标距离即需拆分；此处 g0 边（c0-c1）为最长。
        if max > min_distance {
            if g0 == max {
                let edge = (i0.min(i1), i0.max(i1));
                let mid_idx = if let Some(&idx) = edges.get(&edge) {
                    idx
                } else {
                    // 沿恒向线取该边中点，高度取两端均值，再转回笛卡尔坐标并登记共享边。
                    let mid = rhumb0.interpolate_using_fraction(0.5);
                    let mid_height = (c0.height + c1.height) * 0.5;
                    let mid_cartesian = ellipsoid.cartographic_to_cartesian(
                        &Cartographic::from_radians(mid.longitude, mid.latitude, mid_height),
                    );
                    subdivided_positions.push(mid_cartesian.x);
                    subdivided_positions.push(mid_cartesian.y);
                    subdivided_positions.push(mid_cartesian.z);
                    let idx = (subdivided_positions.len() / 3 - 1) as u32;
                    edges.insert(edge, idx);

                    if has_texcoords {
                        let t0 = DVec2::new(
                            subdivided_texcoords[(i0 * 2) as usize],
                            subdivided_texcoords[(i0 * 2 + 1) as usize],
                        );
                        let t1 = DVec2::new(
                            subdivided_texcoords[(i1 * 2) as usize],
                            subdivided_texcoords[(i1 * 2 + 1) as usize],
                        );
                        let mid_tc = (t0 + t1) * 0.5;
                        subdivided_texcoords.push(mid_tc.x);
                        subdivided_texcoords.push(mid_tc.y);
                    }
                    idx
                };

                triangles.push(i0);
                triangles.push(mid_idx);
                triangles.push(i2);
                triangles.push(mid_idx);
                triangles.push(i1);
                triangles.push(i2);
            } else if g1 == max {
                let edge = (i1.min(i2), i1.max(i2));
                let mid_idx = if let Some(&idx) = edges.get(&edge) {
                    idx
                } else {
                    // g1 为最长边：沿 rhumb1 插值 v1-v2 中点。
                    let mid = rhumb1.interpolate_using_fraction(0.5);
                    let mid_height = (c1.height + c2.height) * 0.5;
                    let mid_cartesian = ellipsoid.cartographic_to_cartesian(
                        &Cartographic::from_radians(mid.longitude, mid.latitude, mid_height),
                    );
                    subdivided_positions.push(mid_cartesian.x);
                    subdivided_positions.push(mid_cartesian.y);
                    subdivided_positions.push(mid_cartesian.z);
                    let idx = (subdivided_positions.len() / 3 - 1) as u32;
                    edges.insert(edge, idx);

                    if has_texcoords {
                        let t1 = DVec2::new(
                            subdivided_texcoords[(i1 * 2) as usize],
                            subdivided_texcoords[(i1 * 2 + 1) as usize],
                        );
                        let t2 = DVec2::new(
                            subdivided_texcoords[(i2 * 2) as usize],
                            subdivided_texcoords[(i2 * 2 + 1) as usize],
                        );
                        let mid_tc = (t1 + t2) * 0.5;
                        subdivided_texcoords.push(mid_tc.x);
                        subdivided_texcoords.push(mid_tc.y);
                    }
                    idx
                };

                triangles.push(i1);
                triangles.push(mid_idx);
                triangles.push(i0);
                triangles.push(mid_idx);
                triangles.push(i2);
                triangles.push(i0);
            } else {
                // g2 == max
                let edge = (i2.min(i0), i2.max(i0));
                let mid_idx = if let Some(&idx) = edges.get(&edge) {
                    idx
                } else {
                    // g2 为最长边：沿 rhumb2 插值 v2-v0 中点。
                    let mid = rhumb2.interpolate_using_fraction(0.5);
                    let mid_height = (c2.height + c0.height) * 0.5;
                    let mid_cartesian = ellipsoid.cartographic_to_cartesian(
                        &Cartographic::from_radians(mid.longitude, mid.latitude, mid_height),
                    );
                    subdivided_positions.push(mid_cartesian.x);
                    subdivided_positions.push(mid_cartesian.y);
                    subdivided_positions.push(mid_cartesian.z);
                    let idx = (subdivided_positions.len() / 3 - 1) as u32;
                    edges.insert(edge, idx);

                    if has_texcoords {
                        let t2 = DVec2::new(
                            subdivided_texcoords[(i2 * 2) as usize],
                            subdivided_texcoords[(i2 * 2 + 1) as usize],
                        );
                        let t0 = DVec2::new(
                            subdivided_texcoords[(i0 * 2) as usize],
                            subdivided_texcoords[(i0 * 2 + 1) as usize],
                        );
                        let mid_tc = (t2 + t0) * 0.5;
                        subdivided_texcoords.push(mid_tc.x);
                        subdivided_texcoords.push(mid_tc.y);
                    }
                    idx
                };

                triangles.push(i2);
                triangles.push(mid_idx);
                triangles.push(i1);
                triangles.push(mid_idx);
                triangles.push(i0);
                triangles.push(i1);
            }
        } else {
            subdivided_indices.push(i0);
            subdivided_indices.push(i1);
            subdivided_indices.push(i2);
        }
    }

    SubdivisionResult {
        positions: subdivided_positions,
        indices: subdivided_indices,
        texcoords: if has_texcoords {
            Some(subdivided_texcoords)
        } else {
            None
        },
    }
}
