//! S2Cell - S2 几何库的单元（cell）表示。
//!
//! 基于 S2 C++ 参考实现：https://github.com/google/s2geometry

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(dead_code, clippy::too_many_arguments, clippy::unnecessary_cast)]
use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use glam::DVec3;

const S2_MAX_LEVEL: u32 = 30;
const S2_LIMIT_IJ: u32 = 1 << S2_MAX_LEVEL; // 2^30
const S2_MAX_SITI: u32 = 1 << (S2_MAX_LEVEL + 1); // 2^31
const S2_POSITION_BITS: u32 = 2 * S2_MAX_LEVEL + 1; // 61
const S2_LOOKUP_BITS: u32 = 4;
const S2_SWAP_MASK: u32 = 1;
const S2_INVERT_MASK: u32 = 2;

const S2_POSITION_TO_IJ: [[u32; 4]; 4] = [
    [0, 1, 3, 2], // 0：正常顺序
    [0, 2, 3, 1], // 1：置 Swap 位
    [3, 2, 0, 1], // 2：置 Invert 位
    [3, 1, 0, 2], // 3：Swap 且 Invert
];

const S2_POSITION_TO_ORIENTATION_MASK: [u32; 4] = [S2_SWAP_MASK, 0, 0, S2_SWAP_MASK | S2_INVERT_MASK];

/// 查找表（懒生成）
struct LookupTables {
    /// 由 (ij, 方向) 索引到 (position, 子方向) 的预计算表，共 1024 个条目。
    positions: Vec<u32>, // 1024 个条目
    /// 由 (position, 方向) 索引到 (ij, 子方向) 的预计算表，共 1024 个条目。
    ij: Vec<u32>,        // 1024 个条目
}

static LOOKUP_TABLES: std::sync::LazyLock<LookupTables> = std::sync::LazyLock::new(|| {
    let mut positions = vec![0u32; 1024];
    let mut ij = vec![0u32; 1024];
    generate_lookup_cell(0, 0, 0, 0, 0, 0, &mut positions, &mut ij);
    generate_lookup_cell(0, 0, 0, S2_SWAP_MASK, 0, S2_SWAP_MASK, &mut positions, &mut ij);
    generate_lookup_cell(0, 0, 0, S2_INVERT_MASK, 0, S2_INVERT_MASK, &mut positions, &mut ij);
    generate_lookup_cell(
        0, 0, 0, S2_SWAP_MASK | S2_INVERT_MASK, 0, S2_SWAP_MASK | S2_INVERT_MASK,
        &mut positions, &mut ij,
    );
    LookupTables { positions, ij }
});

/// 递归生成一个 4 叉层的 S2 希尔伯特索引查找表。
///
/// 逐层将 (i,j) 按方向表 `S2_POSITION_TO_IJ` 拆成 4 个子单元，直到 `S2_LOOKUP_BITS` 层写入双向映射。
fn generate_lookup_cell(
    level: u32,
    i: u32,
    j: u32,
    original_orientation: u32,
    position: u32,
    orientation: u32,
    lookup_positions: &mut [u32],
    lookup_ij: &mut [u32],
) {
    if level == S2_LOOKUP_BITS {
        let ij_val = (i << S2_LOOKUP_BITS) + j;
        lookup_positions[((ij_val << 2) + original_orientation) as usize] =
            (position << 2) + orientation;
        lookup_ij[((position << 2) + original_orientation) as usize] =
            (ij_val << 2) + orientation;
    } else {
        let level = level + 1;
        let i = i << 1;
        let j = j << 1;
        let position = position << 2;
        let r = &S2_POSITION_TO_IJ[orientation as usize];
        generate_lookup_cell(level, i + (r[0] >> 1), j + (r[0] & 1), original_orientation, position, orientation ^ S2_POSITION_TO_ORIENTATION_MASK[0], lookup_positions, lookup_ij);
        generate_lookup_cell(level, i + (r[1] >> 1), j + (r[1] & 1), original_orientation, position + 1, orientation ^ S2_POSITION_TO_ORIENTATION_MASK[1], lookup_positions, lookup_ij);
        generate_lookup_cell(level, i + (r[2] >> 1), j + (r[2] & 1), original_orientation, position + 2, orientation ^ S2_POSITION_TO_ORIENTATION_MASK[2], lookup_positions, lookup_ij);
        generate_lookup_cell(level, i + (r[3] >> 1), j + (r[3] & 1), original_orientation, position + 3, orientation ^ S2_POSITION_TO_ORIENTATION_MASK[3], lookup_positions, lookup_ij);
    }
}

/// 单位球上的一个 S2 单元。
/// 映射到 CesiumJS `Core/S2Cell`
#[derive(Debug, Clone, PartialEq)]
pub struct S2Cell {
    /// 单元的 64 位 S2 cell ID（含面、层级与希尔伯特位置）。
    cell_id: u128,
    /// 单元所在的层级（0–30），由 cell_id 推导。
    level: u32,
}

impl S2Cell {
    /// 由一个 64 位单元 ID 创建新的 S2Cell。
    pub fn new(cell_id: u128) -> Self {
        debug_assert!(Self::is_valid_id(cell_id), "cell ID is invalid");
        let level = Self::get_level(cell_id);
        Self { cell_id, level }
    }

    /// 由一个 token（十六进制表示）创建新的 S2Cell。
    /// 映射到 `S2Cell.fromToken`
    pub fn from_token(token: &str) -> Self {
        debug_assert!(Self::is_valid_token(token), "token is invalid");
        Self::new(Self::get_id_from_token(token))
    }

    /// 由面、沿 Hilbert 曲线的位置以及层级创建 S2Cell。
    /// 映射到 `S2Cell.fromFacePositionLevel`
    pub fn from_face_position_level(face: u32, position: u128, level: u32) -> Self {
        debug_assert!(face <= 5, "Invalid S2 Face (must be within 0-5)");
        debug_assert!(level <= S2_MAX_LEVEL, "Invalid level (must be within 0-30)");
        let max_position: u128 = 1u128 << (2 * level);
        debug_assert!(position < max_position, "Invalid Hilbert position for level");

        // 构造单元 ID：face（3 位）+ position（2*level 位）+ 哨兵位（1 位）+ 填充
        let face_bits = (face as u128) << S2_POSITION_BITS;
        let position_shift = S2_POSITION_BITS - 2 * level - 1;
        let position_bits = position << (position_shift + 1);
        let sentinel_bit: u128 = 1 << position_shift;
        let cell_id = face_bits | position_bits | sentinel_bit;
        Self::new(cell_id)
    }

    /// 单元 ID。
    pub fn cell_id(&self) -> u128 {
        self.cell_id
    }

    /// 本单元的层级。
    pub fn level(&self) -> u32 {
        self.level
    }

    /// 校验一个 S2 单元 ID。
    /// 映射到 `S2Cell.isValidId`
    pub fn is_valid_id(cell_id: u128) -> bool {
        if cell_id == 0 {
            return false;
        }
        // 检查 face 位 [0-5]
        if cell_id >> S2_POSITION_BITS > 5 {
            return false;
        }
        // 检查末尾的 1 位处于有效的偶数位
        let lowest_set_bit = cell_id & (!cell_id + 1);
        if lowest_set_bit & 0x1555555555555555u128 == 0 {
            return false;
        }
        true
    }

    /// 校验一个 S2 单元 token。
    /// 映射到 `S2Cell.isValidToken`
    pub fn is_valid_token(token: &str) -> bool {
        if token.is_empty() || token.len() > 16 {
            return false;
        }
        if !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        Self::is_valid_id(Self::get_id_from_token(token))
    }

    /// 将 token 转换为单元 ID。
    /// 映射到 `S2Cell.getIdFromToken`
    pub fn get_id_from_token(token: &str) -> u128 {
        let padded = format!("{}{}", token, "0".repeat(16 - token.len()));
        u128::from_str_radix(&padded, 16).unwrap_or(0)
    }

    /// 将单元 ID 转换为 token。
    /// 映射到 `S2Cell.getTokenFromId`
    pub fn get_token_from_id(cell_id: u128) -> String {
        let trailing_zero_bits = cell_id.trailing_zeros();
        let trailing_zero_hex = trailing_zero_bits / 4;
        let hex_string = format!("{:x}", cell_id);
        let hex_string = hex_string.trim_end_matches('0');
        let hex_string = if hex_string.is_empty() { "0" } else { hex_string };
        let zero_count = 16usize.saturating_sub(trailing_zero_hex as usize).saturating_sub(hex_string.len());
        format!("{}{}", "0".repeat(zero_count), hex_string)
    }

    /// 由单元 ID 获取层级。
    /// 映射到 `S2Cell.getLevel`
    pub fn get_level(cell_id: u128) -> u32 {
        let lsb_position = cell_id.trailing_zeros();
        S2_MAX_LEVEL - (lsb_position >> 1)
    }

    /// 获取给定索引 [0-3] 处的子单元。
    /// 映射到 `S2Cell.prototype.getChild`
    pub fn get_child(&self, index: u32) -> Self {
        debug_assert!(index <= 3, "child index must be in the range [0-3]");
        debug_assert!(self.level != 30, "cannot get child of leaf cell");
        let new_lsb = lsb(self.cell_id) >> 2;
        // 公式：cellId + (2*index + 1 - 4) * newLsb
        // 使用 i128 以处理 index 0、1 的负乘数
        let multiplier = (2 * index as i128 + 1 - 4) as i128;
        let child_cell_id = (self.cell_id as i128 + multiplier * new_lsb as i128) as u128;
        Self::new(child_cell_id)
    }

    /// 获取父单元。
    /// 映射到 `S2Cell.prototype.getParent`
    pub fn get_parent(&self) -> Self {
        debug_assert!(self.level != 0, "cannot get parent of root cell");
        let new_lsb = lsb(self.cell_id) << 2;
        Self::new((self.cell_id & (!new_lsb + 1)) | new_lsb)
    }

    /// 获取给定层级上的父单元。
    /// 映射到 `S2Cell.prototype.getParentAtLevel`
    pub fn get_parent_at_level(&self, level: u32) -> Self {
        debug_assert!(self.level != 0 && level <= self.level, "cannot get parent at invalid level");
        let new_lsb = lsb_for_level(level);
        // 在 u128 运算中：cellId & -newLsb 等价于 cellId & (!newLsb + 1)
        Self::new((self.cell_id & (!new_lsb + 1)) | new_lsb)
    }

    /// 获取单元中心，以给定椭球上的 Cartesian3 表示。
    /// 映射到 `S2Cell.prototype.getCenter`
    pub fn get_center(&self, ellipsoid: &Ellipsoid) -> DVec3 {
        let center = get_s2_center(self.cell_id, self.level);
        let center = center.normalize();
        let cartographic = Cartographic::from_cartesian(center, &Ellipsoid::UNIT_SPHERE).unwrap();
        Cartographic::to_cartesian(&cartographic, ellipsoid)
    }

    /// 获取单元的一个顶点（CCW 顺序，索引 [0-3]）。
    /// 映射到 `S2Cell.prototype.getVertex`
    pub fn get_vertex(&self, index: u32, ellipsoid: &Ellipsoid) -> DVec3 {
        debug_assert!(index <= 3, "vertex index must be in the range [0-3]");
        let vertex = get_s2_vertex(self.cell_id, self.level, index);
        let vertex = vertex.normalize();
        let cartographic = Cartographic::from_cartesian(vertex, &Ellipsoid::UNIT_SPHERE).unwrap();
        Cartographic::to_cartesian(&cartographic, ellipsoid)
    }
}

// =============================================================================
// 内部坐标转换函数
// =============================================================================

fn lsb(cell_id: u128) -> u128 {
    cell_id & (!cell_id + 1)
}

/// 返回给定层级的最低有效位（用于判定 cell 边界）。
fn lsb_for_level(level: u32) -> u128 {
    1u128 << (2 * (S2_MAX_LEVEL - level))
}

/// 计算 cell 在单位球上的中心点坐标。
fn get_s2_center(cell_id: u128, level: u32) -> DVec3 {
    let (face, si, ti) = convert_cell_id_to_face_siti(cell_id, level);
    convert_face_siti_to_xyz(face, si, ti)
}

/// 按逆时针顺序计算 cell 的第 index 个顶点坐标。
fn get_s2_vertex(cell_id: u128, level: u32, index: u32) -> DVec3 {
    let (face, i, j) = convert_cell_id_to_face_ij(cell_id);
    let uv = convert_ij_level_to_bound_uv(i, j, level);
    // CCW 顺序
    let y = (index >> 1) & 1;
    let u_idx = y ^ (index & 1);
    convert_face_uv_to_xyz(face, uv[0][u_idx as usize], uv[1][y as usize])
}

/// 将 cell ID 转为 (面, si, ti)，对非叶子单元修正中心位置奇偶。
fn convert_cell_id_to_face_siti(cell_id: u128, level: u32) -> (u32, u32, u32) {
    let (face, i, j) = convert_cell_id_to_face_ij(cell_id);
    let is_leaf = level == 30;
    let should_correct = !is_leaf && (((i as u128) ^ (cell_id >> 2)) & 1) != 0;
    let correction = if is_leaf { 1 } else if should_correct { 2 } else { 0 };
    let si = (i << 1) + correction;
    let ti = (j << 1) + correction;
    (face, si, ti)
}

/// 将 cell ID 沿希尔伯特位链逐段查表，解出 (面, i, j)。
fn convert_cell_id_to_face_ij(cell_id: u128) -> (u32, u32, u32) {
    let tables = &LOOKUP_TABLES;
    let face = (cell_id >> S2_POSITION_BITS) as u32;
    let mut bits = face & S2_SWAP_MASK;
    let lookup_mask = (1u32 << S2_LOOKUP_BITS) - 1;

    let mut i: u32 = 0;
    let mut j: u32 = 0;

    for k in (0..8).rev() {
        let number_of_bits = if k == 7 {
            S2_MAX_LEVEL - 7 * S2_LOOKUP_BITS
        } else {
            S2_LOOKUP_BITS
        };
        let extract_mask = (1u128 << (2 * number_of_bits)) - 1;
        bits += (((cell_id >> (k * 2 * S2_LOOKUP_BITS + 1)) & extract_mask) as u32) << 2;

        bits = tables.ij[bits as usize];

        let offset = k * S2_LOOKUP_BITS;
        i += (bits >> (S2_LOOKUP_BITS + 2)) << offset;
        j += ((bits >> 2) & lookup_mask) << offset;

        bits &= S2_SWAP_MASK | S2_INVERT_MASK;
    }

    (face, i, j)
}

/// 由面与 (si,ti) 坐标经 st→uv→xyz 变换得到球面点。
fn convert_face_siti_to_xyz(face: u32, si: u32, ti: u32) -> DVec3 {
    let s = convert_siti_to_st(si);
    let t = convert_siti_to_st(ti);
    let u = convert_st_to_uv(s);
    let v = convert_st_to_uv(t);
    convert_face_uv_to_xyz(face, u, v)
}

/// 将六个面各自的 (u,v) 映射到对应的球面坐标轴排列。
fn convert_face_uv_to_xyz(face: u32, u: f64, v: f64) -> DVec3 {
    match face {
        0 => DVec3::new(1.0, u, v),
        1 => DVec3::new(-u, 1.0, v),
        2 => DVec3::new(-u, -v, 1.0),
        3 => DVec3::new(-1.0, -v, -u),
        4 => DVec3::new(v, -1.0, -u),
        _ => DVec3::new(v, u, -1.0),
    }
}

/// 二次的 ST 到 UV 变换。
fn convert_st_to_uv(s: f64) -> f64 {
    if s >= 0.5 {
        (1.0 / 3.0) * (4.0 * s * s - 1.0)
    } else {
        (1.0 / 3.0) * (1.0 - 4.0 * (1.0 - s) * (1.0 - s))
    }
}

/// 将 siti 整数坐标归一化为 0–1 的 st 值。
fn convert_siti_to_st(si: u32) -> f64 {
    (1.0 / S2_MAX_SITI as f64) * si as f64
}

/// 由 (i,j,level) 求 cell 在 uv 空间的边界区间（u 与 v 各自的 low/high）。
fn convert_ij_level_to_bound_uv(i: u32, j: u32, level: u32) -> [[f64; 2]; 2] {
    let cell_size = get_size_ij(level);
    let mut result = [[0.0f64; 2]; 2];

    let ij_low = i & (!cell_size + 1); // i & -cellSize (unsigned wrap)
    let ij_high = ij_low + cell_size;
    result[0][0] = convert_st_to_uv(convert_ij_to_st_minimum(ij_low));
    result[0][1] = convert_st_to_uv(convert_ij_to_st_minimum(ij_high));

    let ij_low = j & (!cell_size + 1);
    let ij_high = ij_low + cell_size;
    result[1][0] = convert_st_to_uv(convert_ij_to_st_minimum(ij_low));
    result[1][1] = convert_st_to_uv(convert_ij_to_st_minimum(ij_high));

    result
}

/// 返回给定层级下单个 cell 在 ij 空间的整数边长。
fn get_size_ij(level: u32) -> u32 {
    1u32 << (S2_MAX_LEVEL - level)
}

/// 将 ij 整数坐标归一化为 0–1 的 st 下界值。
fn convert_ij_to_st_minimum(i: u32) -> f64 {
    (1.0 / S2_LIMIT_IJ as f64) * i as f64
}
