//! 从 dynamic_globe 黄金路径中提取的屏幕空间误差四叉树 LOD
//!（M1.5）。逻辑逐字节一致 —— 仅模块边界改变。
//!
//! 原始位置位于 `dynamic_globe.rs`：
//! - 常量：L39-44、L75
//! - `compute_segments`: L2066-2069
//! - `focal_pixels`: L1507-1513
//! - `compute_sub_camera_point`: L1719-1730
//! - `Visit` enum: L1563-1567
//! - `compute_visible_tiles`: L1528-1552
//! - `visit_tile`: L1569-1714
//! - 显示集稳定性（refine_cover / blocked / coarsening）：L505-577

// 冻结的遗留黄金路径风格债务；局部 allow 以满足严格 CI clippy 门槛
#![allow(clippy::type_complexity, clippy::unnecessary_map_or)]

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::orbit_camera::{OrbitState, CAMERA_FOV_Y};

// ── 常量（黄金路径逐字保留） ─────────────────────────────────────

pub const MIN_ZOOM: u32 = 3;
/// 请求影像的最深层级。Bing 在 z=21 已不提供影像，只返回无影像占位图
/// （实测 256×256 PNG、1033 字节、均值 RGB(244,241,237) 暖白），而
/// [`crate::globe_textures::is_placeholder_tile`] 的旧判据要求冷色调，
/// 认不出这个暖白变体 ⇒ 会被当作真实影像铺到球面上（相机贴地表时表现为
/// 大片纯白块与空洞）。z=20 仍是真实 JPEG（实测 7.8 KB），故封顶到 20，
/// 从源头上不请求占位层级。
pub const MAX_ZOOM: u32 = 20;
pub const BASE_SEGMENTS: u32 = 48;
/// WGS84 半长轴（米），用于屏幕空间误差计算。
/// METERS_PER_RENDER_UNIT = 6378137 (硬约束).
pub const EARTH_RADIUS_M: f64 = 6378137.0;
pub const MAX_TILE_SCREEN_PX: f64 = 288.0;
/// 保留最粗层级作为永久性的全球回退层。
pub const BASE_LAYER_ZOOM: u32 = 3;

/// 额外常驻的粗级回退层（③「常驻底图提到 z≤4」）。
///
/// 与 [`BASE_LAYER_ZOOM`] 分开是有意的：`BASE_LAYER_ZOOM = 3` 被两个契约锁死
/// ——基础球 composite 的 8×8 图集（`globe_pipeline::run_base_sphere_composite`
/// 按该层级取 8×8 块）与 `BudgetPolicy::base_layer_zoom`（
/// `adapters/pipeline/tests/pipeline_invariants.rs` 断言其等于 3），因此向上
/// 扩展常驻层必须另设常量。
///
/// `RESIDENT_ZOOM = 4` 使启动时预生成 z ≤ 4 的永久实体（z=1..=4 共
/// 4+16+64+256 = 340 个，原来 z=1..=3 的 84 个），它们的**纹理与实体都不参与
/// 淘汰**，从而在中低空提供一个比 z=3 细 2 倍的兜底（代价：约 256 张额外纹理
/// ≈ 90 MB 显存与启动时的额外下载）。
pub const RESIDENT_ZOOM: u32 = 4;

// ── LodContext trait ─────────────────────────────────────────────────────

/// 抽象 LOD 遍历所查询的 TileManager 状态。
/// 薄壳与遗留 TileManager 都实现此 trait。
pub trait LodContext {
    /// 查询给定瓦片键是否已有对应实体（已加载并渲染）。
    fn has_entity(&self, key: &(u32, u32, u32)) -> bool;
    /// 返回给定瓦片键的纹理边长（像素）；尚未就绪时返回 `None`。
    fn tex_size(&self, key: &(u32, u32, u32)) -> Option<u32>;
}

// ── 瓦片键类型 ────────────────────────────────────────────────────────

pub type TileKey = (u32, u32, u32);

// ── 函数 ────────────────────────────────────────────────────────────

/// 原始：`dynamic_globe.rs:2066-2069`。
pub fn compute_segments(zoom: u32) -> u32 {
    // 每升高一级段数减半（右移），但不低于下限 8，避免高层级网格过于粗糙。
    (BASE_SEGMENTS >> zoom.saturating_sub(MIN_ZOOM)).max(8)
}

/// 以像素为单位的垂直焦距：(H/2) / tan(fov/2)。
/// 原始：`dynamic_globe.rs:1507-1513`。
pub fn focal_pixels(windows: &Query<&Window>) -> f64 {
    let h = windows
        .get_single()
        .map(|w| w.height() as f64)
        .unwrap_or(720.0);
    (h * 0.5) / ((CAMERA_FOV_Y as f64) * 0.5).tan()
}

/// 原始：`dynamic_globe.rs:1719-1730`。
pub fn compute_sub_camera_point(orbit: &OrbitState) -> (f64, f64) {
    // 由 heading/pitch 构造视线方向单位向量（以球坐标展开）。
    let cos_pitch = orbit.pitch.cos();
    let sin_pitch = orbit.pitch.sin();
    let dir_x = cos_pitch * orbit.heading.cos();
    let dir_y = cos_pitch * orbit.heading.sin();
    let dir_z = sin_pitch;

    // 将方向向量归一化后反正弦/反正切得到注视点的纬度与经度（弧度）。
    let len = (dir_x * dir_x + dir_y * dir_y + dir_z * dir_z).sqrt();
    let lat = (dir_z / len).asin() as f64;
    let lon = (dir_y as f64).atan2(dir_x as f64);
    (lat, lon)
}

/// 遍历结果：描述整棵子树是否均已就绪可渲染。
/// 原始：`dynamic_globe.rs:1563-1567`。
pub enum Visit {
    Ready,
    NotReady,
    Culled,
}

/// CesiumJS 风格、感知 KICK 的四叉树划分。
/// 原始：`dynamic_globe.rs:1528-1552`。
pub fn compute_visible_tiles<C: LodContext>(
    lat_rad: f64,
    lon_rad: f64,
    distance: f64,
    focal_px: f64,
    ctx: &C,
) -> (Vec<(TileKey, f32)>, Vec<(TileKey, f32)>) {
    // 将 LOD 距离下限设为略低于相机最近的 min_distance
    //（1.0000157 ≈ 100 米），使其永不生效；此处更粗的下限会封顶
    // 可达的最深瓦片层级，无论相机降得多低。
    let d = distance.max(1.00001);
    // 将相机注视点经纬度投影为单位向量 (cx,cy,cz)，并算出可见地平帽的半角 cap。
    let cx = lat_rad.cos() * lon_rad.cos();
    let cy = lat_rad.cos() * lon_rad.sin();
    let cz = lat_rad.sin();
    let cap = (1.0 / d).acos();

    let mut render = Vec::new();
    let mut load = Vec::new();
    // 从最粗层级 MIN_ZOOM 的四分瓦片逐个启动递归划分。
    let n0 = 1u32 << MIN_ZOOM;
    for y in 0..n0 {
        for x in 0..n0 {
            visit_tile(x, y, MIN_ZOOM, cx, cy, cz, d, cap, focal_px, ctx, &mut render, &mut load);
        }
    }
    (render, load)
}

/// 原始：`dynamic_globe.rs:1569-1714`（逐字节保留）。
#[allow(clippy::too_many_arguments)]
pub fn visit_tile<C: LodContext>(
    x: u32,
    y: u32,
    z: u32,
    cx: f64,
    cy: f64,
    cz: f64,
    d: f64,
    cap: f64,
    focal_px: f64,
    ctx: &C,
    render: &mut Vec<(TileKey, f32)>,
    load: &mut Vec<(TileKey, f32)>,
) -> Visit {
    // 当前层级每边瓦片数 n = 2^z；取瓦片中心的经纬度并投影为单位向量。
    let n = 1u64 << z;
    // 经度由列索引均匀划分 2π；纬度用等积投影（sinh/atan 反双曲）并限幅到 ±1.4844 弧度。
    let lon = (x as f64 + 0.5) / n as f64 * 2.0 * std::f64::consts::PI
        - std::f64::consts::PI;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * (y as f64 + 0.5) / n as f64))
        .sinh()
        .atan()
        .clamp(-1.4844, 1.4844);

    let tx = lat.cos() * lon.cos();
    let ty = lat.cos() * lon.sin();
    let tz = lat.sin();

    // 计算瓦片中心与相机方向向量的夹角 theta；若超出可见帽加边界则剔除。
    let dot = (tx * cx + ty * cy + tz * cz).clamp(-1.0, 1.0);
    let theta = dot.acos();
    let margin = 2.0 * std::f64::consts::PI / n as f64;
    if theta > cap + margin {
        return Visit::Culled;
    }

    // 估算瓦片到相机的实际距离（米）与该瓦片对应的屏幕像素宽度。
    let ex = tx - cx * d;
    let ey = ty - cy * d;
    let ez = tz - cz * d;
    let dist_m = (ex * ex + ey * ey + ez * ez).sqrt() * EARTH_RADIUS_M;

    let w_m = 2.0 * std::f64::consts::PI * EARTH_RADIUS_M / n as f64;
    // 瓦片实际宽度除以距离再乘以焦距，得到该瓦片在屏幕上的像素宽度。
    let screen_px = w_m / dist_m * focal_px;

    let has_ent = ctx.has_entity(&(x, y, z));

    // 屏幕像素足够小或已达最深：直接渲染本页（不再细分）。
    if screen_px <= MAX_TILE_SCREEN_PX || z >= MAX_ZOOM {
        render.push(((x, y, z), screen_px as f32));
        if has_ent {
            Visit::Ready
        } else {
            Visit::NotReady
        }
    } else {
        // 否则递归访问四个子瓦片，根据子代结果决定用父页回退还是下钻。
        let start = render.len();
        let (x2, y2, z1) = (x * 2, y * 2, z + 1);
        let children = [
            (x2, y2, z1),
            (x2 + 1, y2, z1),
            (x2, y2 + 1, z1),
            (x2 + 1, y2 + 1, z1),
        ];
        let outcomes = [
            visit_tile(x2, y2, z1, cx, cy, cz, d, cap, focal_px, ctx, render, load),
            visit_tile(x2 + 1, y2, z1, cx, cy, cz, d, cap, focal_px, ctx, render, load),
            visit_tile(x2, y2 + 1, z1, cx, cy, cz, d, cap, focal_px, ctx, render, load),
            visit_tile(x2 + 1, y2 + 1, z1, cx, cy, cz, d, cap, focal_px, ctx, render, load),
        ];
        // 若四个子代均被剔除，则回退到渲染本页。
        let any_selected = outcomes
            .iter()
            .any(|o| matches!(o, Visit::Ready | Visit::NotReady));
        if !any_selected {
            render.push(((x, y, z), screen_px as f32));
            return if ctx.has_entity(&(x, y, z)) {
                Visit::Ready
            } else {
                Visit::NotReady
            };
        }
        // 若子代未全部就绪，则回退本页并调度未就绪的子瓦片加载（粗化保护）。
        let all_ready = outcomes
            .iter()
            .all(|o| matches!(o, Visit::Ready | Visit::Culled));
        if !all_ready {
            render.truncate(start);
            // 将未就绪且纹理分辨率不足（<256）的子瓦片加入加载队列，屏幕宽度减半估算。
            for (c, o) in children.iter().zip(outcomes.iter()) {
                if !matches!(o, Visit::Culled)
                    && (!ctx.has_entity(c) || ctx.tex_size(c).map_or(false, |s| s < 256))
                {
                    load.push((*c, (screen_px * 0.5) as f32));
                }
            }
            render.push(((x, y, z), screen_px as f32));
            if has_ent {
                Visit::Ready
            } else {
                Visit::NotReady
            }
        } else {
            Visit::Ready
        }
    }
}

// ── 显示集稳定性（CesiumJS allAreRenderable） ────────────────────

/// 从旧/新划分计算稳定的显示集。
/// 原始：`dynamic_globe.rs:505-577`（逐字节保留）。
///
/// 返回 `(display_set, partition_changed)`。
pub fn compute_display_set(
    old_set: &HashSet<TileKey>,
    new_set: &HashSet<TileKey>,
    new_load_set: &HashSet<TileKey>,
    prev_partition: &HashSet<TileKey>,
    prev_load: &HashSet<TileKey>,
    replacement_ready: &dyn Fn(&TileKey) -> bool,
) -> (HashSet<TileKey>, bool) {
    // 建立「粗化覆盖表」：对每个新显示瓦片向上回溯，若某祖先在旧集却不在新集，
    // 说明该祖先正被这批后代细化替换，记录祖先到后代列表的映射。
    let mut refine_cover: HashMap<TileKey, Vec<TileKey>> = HashMap::new();
    for n in new_set {
        let (mut ax, mut ay, mut az) = *n;
        while az > 0 {
            ax >>= 1;
            ay >>= 1;
            az -= 1;
            let a = (ax, ay, az);
            if old_set.contains(&a) && !new_set.contains(&a) {
                refine_cover.entry(a).or_default().push(*n);
                break;
            }
        }
    }
    // display 为最终稳定显示集；blocked 记录因替换未就绪而暂时被占位阻塞的瓦片。
    let mut display: HashSet<TileKey> = HashSet::new();
    let mut blocked: HashSet<TileKey> = HashSet::new();
    // 逐个处理旧集瓦片，决定保留、下钻到子代还是上钻到祖先替换。
    for old in old_set.iter() {
        // 新旧集交集：无需过渡，直接保留。
        if new_set.contains(old) {
            display.insert(*old);
            continue;
        }
        // 旧瓦片被粗化：向上寻找新集中的祖先作为替换。
        let (mut ax, mut ay, mut az) = *old;
        let mut ancestor: Option<TileKey> = None;
        while az > 0 {
            ax >>= 1;
            ay >>= 1;
            az -= 1;
            if new_set.contains(&(ax, ay, az)) {
                ancestor = Some((ax, ay, az));
                break;
            }
        }
        // 若存在祖先替换：就绪则显示祖先，否则保留旧瓦片并阻塞该祖先（避免闪烁）。
        if let Some(a) = ancestor {
            if replacement_ready(&a) {
                display.insert(a);
            } else {
                display.insert(*old);
                blocked.insert(a);
            }
            continue;
        }
        // 否则查看粗化覆盖表：后代全部就绪则用后代替换，否则保留旧瓦片并阻塞后代。
        if let Some(desc) = refine_cover.get(old) {
            if desc.iter().all(replacement_ready) {
                display.extend(desc.iter().copied());
            } else {
                display.insert(*old);
                blocked.extend(desc.iter().copied());
            }
            continue;
        }
    }
    // 收尾：处理新集中尚未被 display/blocked 覆盖的瓦片，避免与仍保留的粗祖先重叠。
    for n in new_set {
        if blocked.contains(n) || display.contains(n) {
            continue;
        }
        // 向上回溯判断该瓦片是否被某个仍显示的旧祖先粗覆盖。
        let (mut ax, mut ay, mut az) = *n;
        let mut covered = old_set.contains(n);
        while !covered && az > 0 {
            ax >>= 1;
            ay >>= 1;
            az -= 1;
            covered = old_set.contains(&(ax, ay, az));
        }
        // 未被覆盖则加入显示集。
        if !covered {
            display.insert(*n);
        }
    }
    // 划分相对上一帧是否改变，供上层决定是否重建显示实体。
    let partition_changed = new_set != prev_partition || new_load_set != prev_load;
    (display, partition_changed)
}
