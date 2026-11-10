//! 线框索引生成器。

/// 用于几何渲染的图元类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    /// 点集。
    Points,
    /// 独立线段。
    Lines,
    /// 闭合线段环。
    LineLoop,
    /// 连续线段带。
    LineStrip,
    /// 独立三角形。
    Triangles,
    /// 三角形带（共享相邻边）。
    TriangleStrip,
    /// 三角形扇（共享中心顶点）。
    TriangleFan,
}

impl PrimitiveType {
    /// 返回该值是否为一个有效的 PrimitiveType。
    ///
    /// 映射到 CesiumJS `PrimitiveType.validate`。
    pub fn validate(&self) -> bool {
        true // 所有枚举变体均有效
    }

    /// 返回该图元类型是否为线类型。
    ///
    /// 映射到 CesiumJS `PrimitiveType.isLines`。
    pub fn is_lines(&self) -> bool {
        matches!(self, Self::Lines | Self::LineLoop | Self::LineStrip)
    }

    /// 返回该图元类型是否为三角形类型。
    ///
    /// 映射到 CesiumJS `PrimitiveType.isTriangles`。
    pub fn is_triangles(&self) -> bool {
        matches!(self, Self::Triangles | Self::TriangleStrip | Self::TriangleFan)
    }
}

/// 返回对于给定的图元类型和索引数量，将生成多少个线框索引。
///
/// 三角形每面 3 索引变 6；带/扇形为首边 2 + 每三角形 4；其余类型不变。
pub fn get_wireframe_indices_count(primitive_type: PrimitiveType, index_count: usize) -> usize {
    match primitive_type {
        PrimitiveType::Triangles => {
            // 每个三角形（3 个索引）变为 3 条线段（6 个索引）
            (index_count / 3) * 6
        }
        PrimitiveType::TriangleStrip | PrimitiveType::TriangleFan => {
            // 首边 + 每个三角形 2 条边
            if index_count < 3 {
                0
            } else {
                2 + (index_count - 2) * 4
            }
        }
        _ => index_count,
    }
}

/// 为给定的图元类型创建线框索引。
///
/// # 返回
/// 三角形类图元返回线段对索引列；非三角形类型返回 None。
///
/// 对于非三角形图元类型返回 None。
/// 若提供了 `indices`，则将其作为源索引；
/// 否则生成递增索引 [0, 1, 2, ...]。
pub fn create_wireframe_indices(
    primitive_type: PrimitiveType,
    index_count: usize,
    indices: Option<&[u32]>,
) -> Option<Vec<u32>> {
    match primitive_type {
        PrimitiveType::Triangles => {
            let triangle_count = index_count / 3;
            let mut result = Vec::with_capacity(triangle_count * 6);
            for i in 0..triangle_count {
                let (i0, i1, i2) = if let Some(idx) = indices {
                    (idx[i * 3], idx[i * 3 + 1], idx[i * 3 + 2])
                } else {
                    let base = (i * 3) as u32;
                    (base, base + 1, base + 2)
                };
                result.push(i0);
                result.push(i1);
                result.push(i1);
                result.push(i2);
                result.push(i2);
                result.push(i0);
            }
            Some(result)
        }
        PrimitiveType::TriangleStrip => {
            if index_count < 3 {
                return Some(Vec::new());
            }
            let get = |i: usize| -> u32 {
                if let Some(idx) = indices {
                    idx[i]
                } else {
                    i as u32
                }
            };
            let mut result = Vec::new();
            // 首边
            result.push(get(0));
            result.push(get(1));
            // 对于带形中的每个三角形
            for i in 0..(index_count - 2) {
                let i0 = get(i);
                let i1 = get(i + 1);
                let i2 = get(i + 2);
                result.push(i1);
                result.push(i2);
                result.push(i2);
                result.push(i0);
            }
            Some(result)
        }
        PrimitiveType::TriangleFan => {
            if index_count < 3 {
                return Some(Vec::new());
            }
            let get = |i: usize| -> u32 {
                if let Some(idx) = indices {
                    idx[i]
                } else {
                    i as u32
                }
            };
            let mut result = Vec::new();
            // 首边
            result.push(get(0));
            result.push(get(1));
            // 对于扇形中的每个三角形（均共享顶点 0）
            for i in 0..(index_count - 2) {
                let i0 = get(0); // 中心顶点
                let i1 = get(i + 1);
                let i2 = get(i + 2);
                result.push(i1);
                result.push(i2);
                result.push(i2);
                result.push(i0);
            }
            Some(result)
        }
        _ => None,
    }
}
