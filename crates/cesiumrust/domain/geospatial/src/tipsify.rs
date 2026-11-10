//! Tipsify —— 用于顶点着色器后缓存优化的三角形重排序。
//!
//! 基于 2007 年 SIGGRAPH 论文 "Fast Triangle Reordering for Vertex Locality
//! and Reduced Overdraw"（作者：Sander、Nehab 和 Barczak）。
//!
//! 核心思想：从一个起始顶点出发，反复选择“下一个最优顶点”并输出其邻接且尚未
//! 发射的三角形，使共享顶点的三角形尽量相邻，从而提升后变换顶点缓存命中率。
//! [`calculate_acmr`] 度量任意索引序列的平均未命中率，[`tipsify`] 则生成优化后的顺序。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_is_multiple_of)]
/// 计算给定索引集合的平均缓存未命中率（ACMR）。
///
/// 映射到 `Tipsify.calculateACMR`。
///
/// # Panic
/// - `indices.len()` 必须 >= 3 且为 3 的倍数。
/// - `cache_size` 必须 >= 3。
pub fn calculate_acmr(indices: &[u32], maximum_index: Option<u32>, cache_size: u32) -> f64 {
    let num_indices = indices.len();
    assert!(
        num_indices >= 3 && num_indices % 3 == 0,
        "indices length must be a multiple of three"
    );
    assert!(cache_size >= 3, "cacheSize must be greater than two");

    // 若未给定则计算最大索引
    let max_idx = match maximum_index {
        Some(m) => m,
        None => {
            let mut m = 0u32;
            for &idx in indices {
                if idx > m {
                    m = idx;
                }
            }
            m
        }
    };

    assert!(max_idx > 0, "maximumIndex must be greater than zero");

    // 顶点时间戳
    let mut vertex_time_stamps = vec![0u32; (max_idx + 1) as usize];

    // 缓存处理
    // 逐索引模拟 LRU 缓存：若某顶点已跌出缓存窗口则计为一次未命中并推入新时间戳。
    let mut s = cache_size + 1;
    for &idx in indices {
        if s - vertex_time_stamps[idx as usize] > cache_size {
            vertex_time_stamps[idx as usize] = s;
            s += 1;
        }
    }

    (s - cache_size + 1) as f64 / (num_indices as f64 / 3.0)
}

/// 为顶点着色器后缓存优化三角形。
///
/// 映射到 `Tipsify.tipsify`。
/// 以优化后的顺序返回输入索引的列表。
///
/// # Panic
/// - `indices.len()` 必须 >= 3 且为 3 的倍数。
/// - `cache_size` 必须 >= 3。
pub fn tipsify(indices: &[u32], maximum_index: Option<u32>, cache_size: u32) -> Vec<u32> {
    let num_indices = indices.len();
    assert!(
        num_indices >= 3 && num_indices % 3 == 0,
        "indices length must be a multiple of three"
    );
    assert!(cache_size >= 3, "cacheSize must be greater than two");

    // 确定最大索引 + 1
    let maximum_index_plus_one: usize = match maximum_index {
        Some(m) => {
            assert!(m > 0, "maximumIndex must be greater than zero");
            (m + 1) as usize
        }
        None => {
            let mut m = 0u32;
            for &idx in indices {
                if idx > m {
                    m = idx;
                }
            }
            (m + 1) as usize
        }
    };

    // 顶点数据
    let mut vertices: Vec<VertexData> = (0..maximum_index_plus_one)
        .map(|_| VertexData {
            num_live_triangles: 0,
            time_stamp: 0,
            vertex_triangles: Vec::new(),
        })
        .collect();

    // 构建顶点-三角形邻接关系
    // 逐三角形把其三个顶点各自登记到邻接表，并累加 live 三角形计数。
    let num_triangles = num_indices / 3;
    let mut triangle = 0usize;
    let mut current_index = 0usize;
    while current_index < num_indices {
        let i0 = indices[current_index] as usize;
        let i1 = indices[current_index + 1] as usize;
        let i2 = indices[current_index + 2] as usize;
        vertices[i0].vertex_triangles.push(triangle);
        vertices[i0].num_live_triangles += 1;
        vertices[i1].vertex_triangles.push(triangle);
        vertices[i1].num_live_triangles += 1;
        vertices[i2].vertex_triangles.push(triangle);
        vertices[i2].num_live_triangles += 1;
        triangle += 1;
        current_index += 3;
    }

    // 起始索引
    let mut f: i64 = 0;
    // 时间戳
    // s 为全局单调递增的时间戳；f 为当前起始顶点，-1 表示遍历结束。
    let mut s = cache_size + 1;
    let mut cursor: usize = 1;

    let mut dead_end: Vec<usize> = Vec::new();
    let mut triangle_emitted = vec![false; num_triangles];
    let mut output_indices: Vec<u32> = Vec::with_capacity(num_indices);

    while f != -1 {
        let mut one_ring: Vec<usize> = Vec::new();
        let f_idx = f as usize;
        let limit = vertices[f_idx].vertex_triangles.len();
        for k in 0..limit {
            triangle = vertices[f_idx].vertex_triangles[k];
            if !triangle_emitted[triangle] {
                triangle_emitted[triangle] = true;
                current_index = triangle * 3;
                for _j in 0..3 {
                    let index = indices[current_index] as usize;
                    one_ring.push(index);
                    dead_end.push(index);

                    output_indices.push(indices[current_index]);

                    let vertex = &mut vertices[index];
                    vertex.num_live_triangles -= 1;
                    if s - vertex.time_stamp > cache_size {
                        vertex.time_stamp = s;
                        s += 1;
                    }
                    current_index += 1;
                }
            }
        }

        // getNextVertex
        f = get_next_vertex(
            cache_size,
            &one_ring,
            &mut vertices,
            s,
            &mut dead_end,
            maximum_index_plus_one,
            &mut cursor,
        );
    }

    output_indices
}

/// 单个顶点在重排序过程中的临时状态。
struct VertexData {
    /// 尚包含该顶点且未发射的三角形数量（发射过程中递减）。
    num_live_triangles: i32,
    /// 该顶点最后一次命中缓存时的时间戳（用于判断是否在缓存窗口内）。
    time_stamp: u32,
    /// 包含该顶点的三角形索引列表（顶点-三角形邻接表）。
    vertex_triangles: Vec<usize>,
}

/// 从当前顶点的邻接环中选出一个“下一个顶点”以继续遍历。
///
/// # 参数
/// - `cache_size`：后变换缓存大小（条目数）。
/// - `one_ring`：当前顶点邻接环内的候选顶点。
/// - `vertices`：全顶点状态（含 live 三角形数与时间戳）。
/// - `s`：当前全局时间戳。
/// - `dead_end`：死胡同顶点栈，无候选时回退使用。
/// - `maximum_index_plus_one`：顶点总数上界。
/// - `cursor`：扫描未访问顶点的游标。
///
/// # 返回
/// 下一顶点索引；若 one_ring 无可用则弹 dead_end，再不行则从 cursor 向后扫描；均无时返回 `-1`。
fn get_next_vertex(
    cache_size: u32,
    one_ring: &[usize],
    vertices: &mut [VertexData],
    s: u32,
    dead_end: &mut Vec<usize>,
    maximum_index_plus_one: usize,
    cursor: &mut usize,
) -> i64 {
    let mut n: i64 = -1;
    let mut m: i32 = -1;

    for &index in one_ring {
        if vertices[index].num_live_triangles > 0 {
            let mut p: i32 = 0;
            if s - vertices[index].time_stamp + 2 * vertices[index].num_live_triangles as u32
                <= cache_size
            {
                p = (s - vertices[index].time_stamp) as i32;
            }
            if p > m || m == -1 {
                m = p;
                n = index as i64;
            }
        }
    }

    if n == -1 {
        // skipDeadEnd
        while let Some(d) = dead_end.pop() {
            if vertices[d].num_live_triangles > 0 {
                return d as i64;
            }
        }
        while *cursor < maximum_index_plus_one {
            if vertices[*cursor].num_live_triangles > 0 {
                *cursor += 1;
                return (*cursor - 1) as i64;
            }
            *cursor += 1;
        }
        return -1;
    }
    n
}
