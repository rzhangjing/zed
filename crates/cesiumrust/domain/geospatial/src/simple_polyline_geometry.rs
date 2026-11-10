//! SimplePolylineGeometry —— 一个简单的折线几何生成器。
//!
//! 与复杂折线不同，它不生成带固定屏幕宽度的四边形，而是直接输出一组
//! 逐对相连的线段（Lines）。根据 [`ArcType`] 选择是否沿大地线/恒向线按粒度细分；
//! 颜色支持逐顶点（沿弧线性插值）或逐段（整段同色）两种模式，最终输出扁平的位置、
//! RGBA 字节、成对索引与包围球。

use crate::bounding::BoundingSphere;
use crate::ellipsoid::Ellipsoid;
use crate::math_utils::chord_length;
use crate::polygon_geometry_library::ArcType;
use crate::polyline_pipeline::{generate_arc, number_of_points, ArcOptions};
use glam::DVec3;

/// 使用椭球从笛卡尔位置中提取高度。
///
/// 映射到 `PolylinePipeline.extractHeights`。
///
/// 逐个位置转为经纬度坐标并取高度；无定义点（如球心）回退为 0。
pub fn extract_heights(positions: &[DVec3], ellipsoid: &Ellipsoid) -> Vec<f64> {
    positions
        .iter()
        .map(|p| {
            ellipsoid
                .cartesian_to_cartographic(*p)
                .map(|c| c.height)
                .unwrap_or(0.0)
        })
        .collect()
}

/// 以 RGBA 字节表示的颜色。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorRgba {
    /// 红分量（[0,1] 浮点）。
    pub red: f64,
    /// 绿分量（[0,1] 浮点）。
    pub green: f64,
    /// 蓝分量（[0,1] 浮点）。
    pub blue: f64,
    /// 透明度分量（[0,1] 浮点）。
    pub alpha: f64,
}

impl ColorRgba {
    /// 由 [0,1] 浮点分量构造一个颜色。
    pub fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self { red, green, blue, alpha }
    }

    /// 将浮点颜色分量 [0,1] 转换为字节 [0,255]。
    /// 先限制到 [0,1] 再乘 255 四舍五入，避免越界。
    /// 映射到 `Color.floatToByte`。
    pub fn float_to_byte(value: f64) -> u8 {
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// 以字节数组返回 RGBA。
    /// 逐分量调用 float_to_byte，将 [0,1] 浮点量化为 [0,255] 字节。
    pub fn to_bytes(&self) -> [u8; 4] {
        [
            Self::float_to_byte(self.red),
            Self::float_to_byte(self.green),
            Self::float_to_byte(self.blue),
            Self::float_to_byte(self.alpha),
        ]
    }
}

/// SimplePolylineGeometry::create_geometry 的结果。
#[derive(Debug, Clone)]
pub struct SimplePolylineResult {
    /// 顶点位置（扁平存储：x,y,z,x,y,z,...）。
    pub position_values: Vec<f64>,
    /// 逐顶点的 RGBA 颜色字节（可选）。
    pub color_values: Option<Vec<u8>>,
    /// 线条索引（成对）。
    pub indices: Vec<u32>,
    /// 恒为 PrimitiveType::Lines。
    pub is_lines: bool,
    /// 由原始位置计算出的包围球。
    pub bounding_sphere: BoundingSphere,
}

/// SimplePolylineGeometry 的描述。
///
/// 映射到 CesiumJS `Core/SimplePolylineGeometry`。
#[derive(Debug, Clone)]
pub struct SimplePolylineGeometry {
    /// 折线位置（地心固定系，至少两个）。
    pub positions: Vec<DVec3>,
    /// 逐顶点或逐段颜色（可为空）。
    pub colors: Option<Vec<ColorRgba>>,
    /// 为真时颜色按顶点给定，否则按段给定。
    pub colors_per_vertex: bool,
    /// 边的弧类型（直线/大地线/恒向线）。
    pub arc_type: ArcType,
    /// 弧细分的角度粒度（弧度）。
    pub granularity: f64,
    /// 参考椭球。
    pub ellipsoid: Ellipsoid,
}

impl SimplePolylineGeometry {
    /// 创建一个新的 SimplePolylineGeometry。
    ///
    /// 直接包装各字段；位置至少两个，颜色可为逐顶点或逐段两种布局之一。
    pub fn new(
        positions: Vec<DVec3>,
        colors: Option<Vec<ColorRgba>>,
        colors_per_vertex: bool,
        arc_type: ArcType,
        granularity: f64,
        ellipsoid: Ellipsoid,
    ) -> Self {
        Self {
            positions,
            colors,
            colors_per_vertex,
            arc_type,
            granularity,
            ellipsoid,
        }
    }

    /// 计算一条简单折线的几何表示。
    ///
    /// 映射到 `SimplePolylineGeometry.createGeometry`。
    ///
    /// # 返回
    /// [`SimplePolylineResult`]：扁平顶点位置、逐顶点 RGBA 字节（若有颜色）、
    /// 成对的线条索引、包围球；大地线/恒向线模式下会先按粒度细分，`ArcType::None` 不细分。
    pub fn create_geometry(&self) -> SimplePolylineResult {
        let positions = &self.positions;
        let colors = &self.colors;
        let colors_per_vertex = self.colors_per_vertex;
        let arc_type = self.arc_type;
        let granularity = self.granularity;
        let ellipsoid = &self.ellipsoid;

        let per_segment_colors = colors.is_some() && !colors_per_vertex;
        let length = positions.len();

        // 逐段颜色需为每段复制弧顶点，逐顶点颜色则沿弧插值。
        let position_values: Vec<f64>;
        let mut color_values: Option<Vec<u8>> = None;

        if arc_type == ArcType::Geodesic || arc_type == ArcType::Rhumb {
            // 大地线/恒向线：先在椭球面上按粒度插入中间点。
            let heights = extract_heights(positions, ellipsoid);

            if per_segment_colors {
                // 逐段颜色：为每一段生成弧
                let colors_arr = colors.as_ref().unwrap();
                let min_distance = chord_length(granularity, ellipsoid.maximum_radius());

                let mut position_count = 0usize;
                // 先累加预估总顶点数以预留精确容量，避免逐段 push 时反复扩容。
                for i in 0..length - 1 {
                    position_count += number_of_points(positions[i], positions[i + 1], min_distance) + 1;
                }

                let mut pos_vals: Vec<f64> = Vec::with_capacity(position_count * 3);
                let mut col_vals: Vec<u8> = Vec::with_capacity(position_count * 4);

                for i in 0..length - 1 {
                    let arc_positions = generate_arc(&ArcOptions {
                        positions: &[positions[i], positions[i + 1]],
                        heights: Some(&[heights[i], heights[i + 1]]),
                        granularity,
                        ellipsoid,
                    });

                    let seg_len = arc_positions.len();
                    // 整段同色：将该段颜色字节重复填充到每个弧顶点。
                    let color = colors_arr[i];
                    let bytes = color.to_bytes();
                    for _ in 0..seg_len {
                        col_vals.extend_from_slice(&bytes);
                    }

                    for p in &arc_positions {
                        pos_vals.push(p.x);
                        pos_vals.push(p.y);
                        pos_vals.push(p.z);
                    }
                }

                position_values = pos_vals;
                color_values = Some(col_vals);
            } else {
                // 逐顶点颜色或无颜色：生成完整的弧
                let arc_positions = generate_arc(&ArcOptions {
                    positions,
                    heights: Some(&heights),
                    granularity,
                    ellipsoid,
                });

                let mut pos_vals: Vec<f64> = Vec::with_capacity(arc_positions.len() * 3);
                for p in &arc_positions {
                    pos_vals.push(p.x);
                    pos_vals.push(p.y);
                    pos_vals.push(p.z);
                }
                position_values = pos_vals;

                if let Some(colors_arr) = colors {
                    // 沿弧插值逐顶点颜色
                    let num_positions = arc_positions.len();
                    let mut col_vals: Vec<u8> = Vec::with_capacity(num_positions * 4);

                    let min_distance = chord_length(granularity, ellipsoid.maximum_radius());

                    for i in 0..length - 1 {
                        let p0 = positions[i];
                        let p1 = positions[i + 1];
                        let c0 = colors_arr[i];
                        let c1 = colors_arr[i + 1];

                        let num_pts = number_of_points(p0, p1, min_distance);
                        // 沿段内参数 t 在两端颜色间线性插值，逐顶点写入 RGBA。
                        for j in 0..num_pts {
                            let t = j as f64 / num_pts as f64;
                            let r = c0.red + (c1.red - c0.red) * t;
                            let g = c0.green + (c1.green - c0.green) * t;
                            let b = c0.blue + (c1.blue - c0.blue) * t;
                            let a = c0.alpha + (c1.alpha - c0.alpha) * t;
                            col_vals.push(ColorRgba::float_to_byte(r));
                            col_vals.push(ColorRgba::float_to_byte(g));
                            col_vals.push(ColorRgba::float_to_byte(b));
                            col_vals.push(ColorRgba::float_to_byte(a));
                        }
                    }

                    // 最后一个颜色
                    let last_color = colors_arr[length - 1];
                    col_vals.extend_from_slice(&last_color.to_bytes());

                    color_values = Some(col_vals);
                }
            }
        } else {
            // ArcType::None —— 不进行细分
            // 逐段颜色时每段需重复端点，故顶点数为 2*(段数)；否则直接用原顶点。
            let number_of_positions = if per_segment_colors {
                length * 2 - 2
            } else {
                length
            };

            let mut pos_vals: Vec<f64> = Vec::with_capacity(number_of_positions * 3);
            let mut col_vals: Vec<u8> = if colors.is_some() {
                Vec::with_capacity(number_of_positions * 4)
            } else {
                Vec::new()
            };

            let colors_arr = colors.as_deref();

            for i in 0..length {
                let p = positions[i];

                // 逐段颜色时除首点外每个内部点需重复一次，以便两侧段分别取色。
                if per_segment_colors && i > 0 {
                    pos_vals.push(p.x);
                    pos_vals.push(p.y);
                    pos_vals.push(p.z);

                    if let Some(cols) = colors_arr {
                        let color = cols[i - 1];
                        col_vals.extend_from_slice(&color.to_bytes());
                    }
                }

                if per_segment_colors && i == length - 1 {
                    break;
                }

                pos_vals.push(p.x);
                pos_vals.push(p.y);
                pos_vals.push(p.z);

                if let Some(cols) = colors_arr {
                    let color = cols[i];
                    col_vals.extend_from_slice(&color.to_bytes());
                }
            }

            position_values = pos_vals;
            if colors.is_some() {
                color_values = Some(col_vals);
            }
        }

        // 生成线条索引：每相邻两顶点构成一条段（i, i+1）。
        let number_of_positions = position_values.len() / 3;
        let number_of_indices = (number_of_positions - 1) * 2;
        let mut indices: Vec<u32> = Vec::with_capacity(number_of_indices);
        for i in 0..(number_of_positions - 1) as u32 {
            indices.push(i);
            indices.push(i + 1);
        }

        // 由原始位置计算包围球（用未细分的输入位置，与 JS 一致）。
        let bounding_sphere = BoundingSphere::from_points(positions);

        SimplePolylineResult {
            position_values,
            color_values,
            indices,
            is_lines: true,
            bounding_sphere,
        }
    }
}
