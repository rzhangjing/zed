//! 地球瓦片管线内部实现 —— 从 dynamic_globe 黄金
//! 路径中抽取（M1.5）。包含预算化的 process_pipeline 主体、下载 worker
//! （带离线接线）、淘汰、网格构建与覆盖修复。
//!
//! 所有逻辑均与原始单体**逐字节一致**；仅模块
//! 边界改变。`evict_gpu_cache` 的三条不变式被保留：
//! - 常驻层（z ≤ `RESIDENT_ZOOM`，含 BASE_LAYER z ≤ 3）永久豁免
//! - 活跃实体 push_back 延迟（花屏防護核心）
//! - 终止性：`MAX_TILE_ENTITIES(1800) << MAX_GPU_CACHE_ENTRIES(3000)`
//!
//! ## 手势期的自适应预算（粗糙度优化 ①②）
//! `run` 接收一个 `&dyn BudgetPolicy`：默认是 `DefaultBudget`（黄金路径常量
//! 不动），相机手势进行中则是 [`GestureBudget`]，它按 [`GESTURE_BUDGET_SCALE`]
//! 放大上传/生成预算并放宽实体上限 [`GESTURE_ENTITY_SLACK`]（拖动中不淘汰
//! 旧级别瓦片，等新级别就绪再换），松手后自动回落。
//!
//! ## 离线接线（M3 门槛 L127）
//! `download_worker` 检查 `feature_flags::offline_imagery_root()`：
//! - 已设置 → 从磁盘读取 `{root}/{z}/{x}/{y}.png`（无网络）
//! - `STRICT_OFFLINE=1` + https URL → 同步 panic（无回退）

// 冻结的遗留黄金路径风格债务；局部 allow 以满足严格 CI clippy 门槛
#![allow(clippy::type_complexity, clippy::unnecessary_map_or, clippy::too_many_arguments)]

use bevy::prelude::*;
use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::base_sphere::{
    self, BaseSphereComposite, BaseSphereMarker, COMPOSITE_SIZE, COMPOSITE_TILE,
};
use crate::dynamic_globe::{MeshPipeline, TextureReceiver, TileManager};
use crate::feature_flags;
use crate::globe_lod::{self, TileKey, BASE_LAYER_ZOOM, RESIDENT_ZOOM};
use crate::globe_textures::{build_mip_chain, is_placeholder_tile, make_image};
use crate::perf_counters::PerfCounters;
use crate::tile_mesh::{create_tile_mesh, create_tile_mesh_uv, render_scale, GlobeTile};
use cesium_bevy_render::CesiumGlobe;
use cesium_pipeline::DefaultBudget;
use cesium_ports_driven::BudgetPolicy;

// ── Payload 类型 ─────────────────────────────────────────────────────────

/// 来自 worker 池的下载结果（取代遗留的 TileDownloadResult）。
/// 特化的 ImageryPayload：RGBA + mip 链 + 陈旧标志。
pub struct TileDownloadResult {
    /// 瓦片列索引 x。
    pub x: u32,
    /// 瓦片行索引 y。
    pub y: u32,
    /// 瓦片层级 z。
    pub z: u32,
    /// 拼接后的 RGBA 数据（含完整 mip 链）。
    pub rgba_data: Vec<u8>,
    /// 顶层纹理宽度（像素）。
    pub width: u32,
    /// 顶层纹理高度（像素）。
    pub height: u32,
    /// mip 链层级数。
    pub mip_levels: u32,
    /// 占位图（无数据）标志。
    pub placeholder: bool,
    /// 因陈旧而中断（未真正获取）标志。
    pub aborted: bool,
    /// 获取失败标志。
    pub failed: bool,
}

// ── 瓦片入队 ────────────────────────────────────────────────────────

/// 为需要的瓦片排队网格构建 + 下载，屏幕占比
/// 最高者优先。原始：`dynamic_globe.rs:371-459`。
pub(crate) fn enqueue_tiles(
    mgr: &mut TileManager,
    mesh_pipe: &mut MeshPipeline,
    tex_rx: &TextureReceiver,
    tiles: &[(TileKey, f32)],
) {
    // 三类待办队列：网格构建任务、下载任务、需生成的实体，均来自主循环本地缓冲。
    let mut mesh_jobs: Vec<(TileKey, u32, Option<[f32; 4]>)> = Vec::new();
    let mut downloads: Vec<(TileKey, bool, f32)> = Vec::new();
    let mut to_spawn: Vec<(TileKey, f32)> = Vec::new();

    {
        // 持锁遍历本帧需要的瓦片，按优先级与缓存状态分类排队。
        let cache = tex_rx.cache.lock().unwrap();
        for &(key, prio) in tiles {
            // 分辨率升级：own_full_res 契约（原 L837-841）。
            // 若当前显存分辨率不足 256 且高优先级/已上采样，则重拉全分辨率。
            if !mgr.no_data.contains(&key)
                && !mgr.reupload.contains(&key)
                && (prio > 192.0 || mgr.upsampled.contains(&key))
                && mgr.gpu_tex_size.get(&key).map_or(true, |&sz| sz < 256)
                && mgr.retry_after.get(&key).map_or(true, |t| std::time::Instant::now() >= *t)
            {
                downloads.push((key, false, prio));
                mgr.reupload.insert(key);
                mgr.in_flight.insert(key);
            }
            // 已有实体或已在队中：无需重复排队生成。
            if mgr.tile_entities.contains_key(&key) || mgr.queued.contains(&key) {
                continue;
            }
            // 新瓦片：加入生成队列；若无网格则同时排队网格构建。
            to_spawn.push((key, prio));
            if !mgr.gpu_meshes.contains_key(&key) {
                mesh_jobs.push((key, globe_lod::compute_segments(key.2), None));
            }
            // 若本地/显存/在途均无此瓦片且已过重试时间，则排队下载（低优先级标记降采样）。
            if !cache.contains_key(&key)
                && !mgr.gpu_textures.contains_key(&key)
                && !mgr.no_data.contains(&key)
                && !mgr.in_flight.contains(&key)
                && mgr.retry_after.get(&key).map_or(true, |t| std::time::Instant::now() >= *t)
            {
                downloads.push((key, prio < 64.0, prio));
                mgr.in_flight.insert(key);
            }
        }
    }

    // 按屏幕占比降序排列，先入队者优先生成/下载。
    to_spawn.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (key, _) in to_spawn {
        mgr.spawn_queue.push_back(key);
        mgr.queued.insert(key);
    }
    // 下载同样按优先级降序，确保高占比瓦片先取回。
    downloads.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let downloads: Vec<(TileKey, bool)> = downloads.into_iter().map(|(k, d, _)| (k, d)).collect();

    // 将当前想要的瓦片集合告知 worker（供陈旧跳过判断），再触发后台网格构建与下载。
    { let mut w = tex_rx.wanted.lock().unwrap(); w.extend(mgr.queued.iter().copied()); }
    if !mesh_jobs.is_empty() { start_mesh_builds(mesh_pipe, mesh_jobs); }
    if !downloads.is_empty() { start_downloads(tex_rx, &downloads); }
}

// ── 手势期预算策略（粗糙度优化 ①） ────────────────────────────────

/// 相机手势期间逐帧预算的放大倍数（①「拖动中抬高逐帧预算」）。
///
/// `DefaultBudget` 的常量是黄金路径参考值（见 `cesium_pipeline::DefaultBudget`
/// 文档：调优应通过自定义 `BudgetPolicy`，不要改常量），所以这里不改默认值，
/// 而是在拖动期间用 [`GestureBudget`] 包装：上传/生成预算 ×3，让新级别的瓦片
/// 更快补齐；松手后立即回落，避免稳定期持续高开销。
pub const GESTURE_BUDGET_SCALE: usize = 3;

/// 相机手势期间实体上限的额外余量（②「保留旧级别瓦片到新级别就绪」）。
///
/// 取硬上限的 1/4（1800 → 2250）。`globe_lod::compute_display_set` 已保证
/// 替换未就绪时继续显示旧瓦片，但逐帧清理（`run` 第 5 步）仍可能因为上限
/// 被顶到而把刚被替换下来的旧级别实体淘汰掉、下一帧又重建 —— 拖动中的
/// 粗糙/闪烁来源之一。拖动期间放宽上限即让这批“在岗的旧瓦片”活到新级别
/// 就绪；2250 仍远小于 `MAX_GPU_CACHE_ENTRIES(3000)`，淘汰终止性不变。
pub const GESTURE_ENTITY_SLACK: usize = DefaultBudget::MAX_TILE_ENTITIES / 4;

/// 手势期预算策略：包装 [`DefaultBudget`]，按倍数放大逐帧上传/生成预算。
///
/// 只放大“逐帧速率”类预算（mesh 上传、实体生成、纹理上传、despawn），
/// 并在手势期放宽实体上限；`base_layer_zoom` 与 GPU 缓存上界保持默认，
/// 以免破坏合同不变量（`adapters/pipeline/tests/pipeline_invariants.rs`）。
#[derive(Debug, Clone, Copy)]
pub struct GestureBudget {
    /// 逐帧预算倍数（1 = 等价于 `DefaultBudget`）。
    scale: usize,
}

impl GestureBudget {
    /// 以 `scale` 倍放大逐帧预算（自动下限为 1）。
    pub fn new(scale: usize) -> Self {
        Self { scale: scale.max(1) }
    }

    /// 根据“本帧是否正在进行相机手势”选择预算。
    pub fn for_gesture(gesture_active: bool) -> Self {
        Self::new(if gesture_active { GESTURE_BUDGET_SCALE } else { 1 })
    }
}

impl BudgetPolicy for GestureBudget {
    fn download_threads(&self) -> usize {
        DefaultBudget.download_threads()
    }

    fn max_mesh_uploads_per_frame(&self) -> usize {
        DefaultBudget.max_mesh_uploads_per_frame() * self.scale
    }

    fn max_spawns_per_frame(&self) -> usize {
        DefaultBudget.max_spawns_per_frame() * self.scale
    }

    fn max_texture_uploads_per_frame(&self) -> usize {
        DefaultBudget.max_texture_uploads_per_frame() * self.scale
    }

    fn max_despawns_per_frame(&self) -> usize {
        DefaultBudget.max_despawns_per_frame() * self.scale
    }

    fn max_tile_entities(&self) -> usize {
        if self.scale == 1 {
            DefaultBudget.max_tile_entities()
        } else {
            DefaultBudget.max_tile_entities() + GESTURE_ENTITY_SLACK
        }
    }

    fn base_layer_zoom(&self) -> u32 {
        DefaultBudget.base_layer_zoom()
    }

    fn max_gpu_cache_entries(&self) -> usize {
        DefaultBudget.max_gpu_cache_entries()
    }
}

// ── 主管线运行（process_pipeline 主体） ────────────────────────────

/// 预算化资源管线。原始：`dynamic_globe.rs:662-1429`。
///
/// `budget` 为逐帧预算策略：稳定期传 `&DefaultBudget`，相机手势进行中传
/// `&GestureBudget::for_gesture(true)`（见该类型文档）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    mgr: &mut TileManager,
    mesh_pipe: &mut MeshPipeline,
    tex_rx: &TextureReceiver,
    perf: &mut PerfCounters,
    budget: &dyn BudgetPolicy,
) {
    // 未初始化（尚无任何可见集）则直接早退。
    if !mgr.initialized { return; }
    perf.begin_frame();
    // 本帧淘汰/延迟/陈旧跳过计数，累加后回写至 PerfCounters。
    let mut evict_total_delta: u32 = 0;
    let mut evict_deferred_delta: u32 = 0;
    let mut stale_skips_delta: u32 = 0;
    // 取本帧视图是否变化（影响后续覆盖修复仅在稳定帧执行）。
    let view_changed = mgr.view_changed_this_frame;
    mgr.view_changed_this_frame = false;

    // 1) 将已完成的后台网格收集到待上传队列。
    let drained: Vec<(TileKey, Mesh, bool)> = {
        let rx = mesh_pipe.rx.lock().unwrap();
        let mut v = Vec::new();
        while let Ok(item) = rx.try_recv() { v.push(item); }
        v
    };
    for item in drained { mesh_pipe.backlog.push_back(item); }

    // 2) 在预算内将待上传网格上传到 GPU。
    let mut mesh_uploads = 0;
    while mesh_uploads < budget.max_mesh_uploads_per_frame() {
        let Some((key, mesh, is_fb)) = mesh_pipe.backlog.pop_front() else { break };
        // 已有同类网格则丢弃重复上传。
        if (is_fb && mgr.gpu_fb_meshes.contains_key(&key))
            || (!is_fb && mgr.gpu_meshes.contains_key(&key)) { continue; }
        // 不再可见/加载/排队的瓦片：无需上传。
        if !mgr.visible_set.contains(&key) && !mgr.load_set.contains(&key)
            && !mgr.queued.contains(&key) { continue; }
        // is_fb 表示带 UV 重映射的回退网格，否则为常规网格。
        if is_fb {
            mgr.pending_fb_builds.remove(&key);
            mgr.gpu_fb_meshes.insert(key, meshes.add(mesh));
        } else {
            mgr.gpu_meshes.insert(key, meshes.add(mesh));
        }
        // 每次新增 GPU 资源后触发淘汰，防止显存无限增长。
        let (e, d) = evict_gpu_cache(mgr);
        evict_total_delta += e; evict_deferred_delta += d;
        mesh_uploads += 1;
    }

    // 3) 在预算内生成网格句柄已就绪的实体。
    let scale = render_scale();
    let pending: Vec<TileKey> = mgr.spawn_queue.drain(..).collect();
    let mut still_queued: Vec<TileKey> = Vec::new();
    let mut spawns = 0;
    let max_spawns = budget.max_spawns_per_frame();

    let cache = tex_rx.cache.lock().unwrap();
    for key in pending {
        // 已生成或已不在排队集合：跳过。
        let already = mgr.tile_entities.contains_key(&key);
        if already || !mgr.queued.remove(&key) { continue; }

        // 判断该瓦片所需网格是否就绪：无数据瓦片看其实体类型（纯色或回退）对应的网格。
        let nodata = mgr.no_data.contains(&key);
        let mesh_ready = if nodata {
            if mgr.solid_tiles.contains(&key) { mgr.gpu_meshes.contains_key(&key) }
            else { mgr.gpu_fb_meshes.contains_key(&key) }
        } else {
            mgr.gpu_meshes.contains_key(&key) || mgr.gpu_fb_meshes.contains_key(&key)
        };
        if !mesh_ready || spawns >= max_spawns {
            // 网格未就绪或本帧生成预算用尽：重新入队等下一帧。
            still_queued.push(key); mgr.queued.insert(key); continue;
        }
        // 已离开可见/加载/划分集：放弃生成。
        if !mgr.visible_set.contains(&key) && !mgr.load_set.contains(&key)
            && !mgr.partition_set.contains(&key) { continue; }

        // 无数据瓦片：用纯色或继承祖先材质直接生成（不回退到拉伸祖先）。
        if nodata {
            if mgr.solid_tiles.contains(&key) && has_live_ancestor(mgr, &key) { continue; }
            // 纯色瓦片用常规网格，否则用回退网格。
            let mesh_handle = if mgr.solid_tiles.contains(&key) {
                mgr.gpu_meshes[&key].clone()
            } else { mgr.gpu_fb_meshes[&key].clone() };
            let material = if let Some(mat) = mgr.gpu_materials.get(&key) { mat.clone() }
            else if let Some(tex) = mgr.effective_tex.get(&key).cloned() {
                let mat = materials.add(StandardMaterial {
                    base_color: Color::WHITE, base_color_texture: Some(tex),
                    perceptual_roughness: 0.9, cull_mode: None, ..default()
                });
                mgr.gpu_materials.insert(key, mat.clone()); mat
            } else {
                let mat = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.01, 0.05, 0.10),
                    perceptual_roughness: 1.0, cull_mode: None, ..default()
                });
                mgr.gpu_materials.insert(key, mat.clone()); mat
            };
            let entity = commands.spawn((CesiumGlobe, GlobeTile { x: key.0, y: key.1, z: key.2 },
                Mesh3d(mesh_handle), MeshMaterial3d(material),
                Transform::from_scale(Vec3::splat(scale)))).id();
            mgr.tile_entities.insert(key, entity);
            mgr.spawn_order.push_back(key);
            mgr.textured_tiles.insert(key);
            spawns += 1; continue;
        }

        // 真实影像路径 —— own_full_res 契约（原 L837-841, L1163）：
        // 瓦片仅在达到全分辨率（≥256 px）后才显示其自身的纹理。
        let cached_tex = mgr.gpu_textures.get(&key).cloned().or_else(|| {
            cache.get(&key).map(|c| make_image(images, c.rgba_data.clone(), c.width, c.height, c.mip_levels))
        });
        let own_width = mgr.gpu_tex_size.get(&key).copied()
            .or_else(|| cache.get(&key).map(|c| c.width));
        let own_full_res = cached_tex.is_some()
            && (key.2 <= BASE_LAYER_ZOOM || own_width.map_or(false, |w| w >= 256));

        if !own_full_res {
            if key.2 <= BASE_LAYER_ZOOM {
                // 无影像的基根：直接落入纯色生成。
            } else if mgr.upsampled.contains(&key) {
                // 已被上采样占位：重新入队等待自身全分辨率纹理。
                still_queued.push(key); mgr.queued.insert(key); continue;
            } else {
                // 继承最近的有纹理祖先（UV 重映射）。
                let (mut ax, mut ay, mut az) = key;
                let mut ancestor: Option<(TileKey, Handle<Image>)> = None;
                // 逐级上移父瓦片，首个拥有有效纹理的祖先即为回退源。
                while az > 0 {
                    ax >>= 1; ay >>= 1; az -= 1;
                    if let Some(t) = mgr.effective_tex.get(&(ax, ay, az)) {
                        ancestor = Some(((ax, ay, az), t.clone())); break;
                    }
                }
                // 无可继祖先：放弃本轮（重新入队等下帧）。
                let Some((anc, tex)) = ancestor else {
                    still_queued.push(key); mgr.queued.insert(key); continue;
                };
                // 计算本瓦片在祖先纹理内对应的子矩形 UV（祖先 UV 按 2^dz 划分取第 rx/ry 格）。
                let dz = key.2 - anc.2;
                let side = (1u32 << dz) as f32;
                let rx = (key.0 % (1u32 << dz)) as f32;
                let ry = (key.1 % (1u32 << dz)) as f32;
                let [au0, av0, au1, av1] = mgr.effective_uv.get(&anc).copied().unwrap_or([0.,0.,1.,1.]);
                let fu = (au1 - au0) / side; let fv = (av1 - av0) / side;
                let uv = [au0 + rx*fu, av0 + ry*fv, au0 + (rx+1.)*fu, av0 + (ry+1.)*fv];
                // 回退网格未就绪则排队构建，本帧先跳过（下帧重试）。
                let mesh_handle = if let Some(h) = mgr.gpu_fb_meshes.get(&key) { h.clone() }
                else {
                    if !mgr.pending_fb_builds.contains(&key) {
                        mgr.pending_fb_builds.insert(key);
                        start_mesh_builds(mesh_pipe, vec![(key, globe_lod::compute_segments(key.2), Some(uv))]);
                    }
                    still_queued.push(key); mgr.queued.insert(key); continue;
                };
                let material = if let Some(mat) = mgr.gpu_materials.get(&key) { mat.clone() }
                else {
                    let mat = materials.add(StandardMaterial {
                        base_color: Color::WHITE, base_color_texture: Some(tex.clone()),
                        perceptual_roughness: 0.9, cull_mode: None, ..default()
                    });
                    mgr.gpu_materials.insert(key, mat.clone()); mat
                };
                mgr.effective_tex.insert(key, tex);
                mgr.effective_uv.insert(key, uv);
                let entity = commands.spawn((CesiumGlobe, GlobeTile { x: key.0, y: key.1, z: key.2 },
                    Mesh3d(mesh_handle), MeshMaterial3d(material),
                    Transform::from_scale(Vec3::splat(scale)))).id();
                mgr.tile_entities.insert(key, entity);
                mgr.spawn_order.push_back(key);
                mgr.textured_tiles.insert(key);
                mgr.upsampled.insert(key);
                spawns += 1; continue;
            }
        }

        // 自身全分辨率路径（或基根）。
        // 常规网格未就绪则重新入队。
        let Some(mesh_handle) = mgr.gpu_meshes.get(&key).cloned() else {
            still_queued.push(key); mgr.queued.insert(key); continue;
        };
        // 若有自身纹理则登记有效纹理/UV（全帧覆盖 0..1）。
        if let Some(tex) = &cached_tex {
            mgr.effective_tex.insert(key, tex.clone());
            mgr.effective_uv.insert(key, [0.0, 0.0, 1.0, 1.0]);
        }
        // 材质选取：已缓存材质 > 新建带纹理材质 > 新建纯色占位材质。
        let material = if let Some(mat) = mgr.gpu_materials.get(&key) { mat.clone() }
        else if let Some(tex) = cached_tex.clone() {
            let mat = materials.add(StandardMaterial {
                base_color: Color::WHITE, base_color_texture: Some(tex),
                perceptual_roughness: 0.9, cull_mode: None, ..default()
            });
            mgr.gpu_materials.insert(key, mat.clone()); mat
        } else {
            let mat = materials.add(StandardMaterial {
                base_color: Color::srgb(0.01, 0.05, 0.10),
                perceptual_roughness: 1.0, cull_mode: None, ..default()
            });
            mgr.gpu_materials.insert(key, mat.clone()); mat
        };
        // 新纹理首次入显存时记录 LRU 顺序并触发淘汰。
        if let Some(tex) = cached_tex {
            if !mgr.gpu_textures.contains_key(&key) { mgr.gpu_tex_order.push_back(key); }
            mgr.gpu_textures.insert(key, tex);
            let (e, d) = evict_gpu_cache(mgr);
            evict_total_delta += e; evict_deferred_delta += d;
        }
        let entity = commands.spawn((CesiumGlobe, GlobeTile { x: key.0, y: key.1, z: key.2 },
            Mesh3d(mesh_handle), MeshMaterial3d(material),
            Transform::from_scale(Vec3::splat(scale)))).id();
        mgr.tile_entities.insert(key, entity);
        mgr.spawn_order.push_back(key);
        mgr.textured_tiles.insert(key);
        spawns += 1;
    }
    drop(cache);
    mgr.spawn_queue = still_queued.into();

    // 4) 在预算内应用已下载的纹理。
    let mut tex_uploads = 0;
    while tex_uploads < budget.max_texture_uploads_per_frame() {
        // 从接收通道非阻塞取出一帧预算内的下载结果。
        let Ok(result) = tex_rx.rx.lock().unwrap().try_recv() else { break };
        let key = (result.x, result.y, result.z);
        mgr.in_flight.remove(&key);
        // aborted：worker 已陈旧跳过，计入陈旧跳过并清除重传标记。
        if result.aborted {
            mgr.reupload.remove(&key); stale_skips_delta += 1; continue;
        }
        // failed：记 10 秒退避重试时间，本帧不处理。
        if result.failed {
            mgr.reupload.remove(&key);
            mgr.retry_after.insert(key, std::time::Instant::now() + std::time::Duration::from_secs(10));
            continue;
        }
        // placeholder：无数据判定，继承祖先纹理或落入纯色，并排队回退网格。
        if result.placeholder {
            mgr.reupload.remove(&key);
            if mgr.gpu_textures.contains_key(&key) { tex_uploads += 1; continue; }
            if mgr.no_data.insert(key) {
                // 首次判定无数据：向上寻找最近有纹理祖先，取其纹理按 UV 重映射回退。
                let (mut ax, mut ay, mut az) = key;
                let mut ancestor: Option<(TileKey, Handle<Image>)> = None;
                while az > 0 {
                    ax >>= 1; ay >>= 1; az -= 1;
                    if let Some(t) = mgr.effective_tex.get(&(ax, ay, az)) {
                        ancestor = Some(((ax, ay, az), t.clone())); break;
                    }
                }
                if let Some((anc, tex)) = ancestor {
                    let dz = key.2 - anc.2; let side = (1u32 << dz) as f32;
                    let rx = (key.0 % (1u32 << dz)) as f32; let ry = (key.1 % (1u32 << dz)) as f32;
                    let [au0, av0, au1, av1] = mgr.effective_uv.get(&anc).copied().unwrap_or([0.,0.,1.,1.]);
                    let fu = (au1-au0)/side; let fv = (av1-av0)/side;
                    let uv = [au0+rx*fu, av0+ry*fv, au0+(rx+1.)*fu, av0+(ry+1.)*fv];
                    mgr.effective_tex.insert(key, tex);
                    mgr.effective_uv.insert(key, uv);
                    if !mgr.gpu_fb_meshes.contains_key(&key) {
                        start_mesh_builds(mesh_pipe, vec![(key, globe_lod::compute_segments(key.2), Some(uv))]);
                    }
                } else if let Some(tex) = mgr.composite_texture.clone() {
                    // 无纹理祖先可继：退一级把基础球 composite（z=3 的 8×8
                    // 全球图集）当作"根祖先"，按本瓦片在图集中的位置取子
                    // 矩形 ⇒ 缺口读作模糊的地球，而不是深色纯色块（旧行为
                    // 正是贴地表时看到的"空洞"）。
                    let base_z = globe_lod::BASE_LAYER_ZOOM;
                    let n3 = 1u32 << base_z;
                    let dz = key.2.saturating_sub(base_z);
                    let side = 1u32 << dz;
                    let rx = (key.0 % side) as f32;
                    let ry = (key.1 % side) as f32;
                    let inv = 1.0 / (n3 as f32 * side as f32);
                    let bx = (key.0 / side) as f32 * side as f32;
                    let by = (key.1 / side) as f32 * side as f32;
                    let uv = [
                        (bx + rx) * inv,
                        (by + ry) * inv,
                        (bx + rx + 1.0) * inv,
                        (by + ry + 1.0) * inv,
                    ];
                    mgr.effective_tex.insert(key, tex);
                    mgr.effective_uv.insert(key, uv);
                    if !mgr.gpu_fb_meshes.contains_key(&key) {
                        start_mesh_builds(
                            mesh_pipe,
                            vec![(key, globe_lod::compute_segments(key.2), Some(uv))],
                        );
                    }
                } else {
                    // 无纹理祖先可继：向上寻找已有实体的祖先复用其材质；否则标记为纯色瓦片。
                    let mut ix = key.0; let mut iy = key.1; let mut iz = key.2;
                    let mut inherited = false;
                    while iz > 0 {
                        ix >>= 1; iy >>= 1; iz -= 1;
                        if mgr.tile_entities.contains_key(&(ix, iy, iz)) {
                            if let Some(mat) = mgr.gpu_materials.get(&(ix,iy,iz)).cloned() {
                                mgr.gpu_materials.insert(key, mat); inherited = true;
                            }
                            break;
                        }
                    }
                    if !inherited { mgr.solid_tiles.insert(key); }
                }
            }
            if !mgr.tile_entities.contains_key(&key) && !mgr.queued.contains(&key) {
                mgr.spawn_queue.push_back(key); mgr.queued.insert(key);
            }
            tex_uploads += 1; continue;
        }

        // 纹理下载成功。
        // 若已有更高分辨率显存纹理则复用，否则新建并记录尺寸/顺序。
        let tex_handle = if matches!(
            (mgr.gpu_textures.get(&key), mgr.gpu_tex_size.get(&key)),
            (Some(_), Some(&sz)) if sz >= result.width
        ) { mgr.gpu_textures[&key].clone() }
        else {
            let h = make_image(images, result.rgba_data, result.width, result.height, result.mip_levels);
            if !mgr.gpu_textures.contains_key(&key) { mgr.gpu_tex_order.push_back(key); }
            mgr.gpu_textures.insert(key, h.clone());
            mgr.gpu_tex_size.insert(key, result.width);
            let (e, d) = evict_gpu_cache(mgr);
            evict_total_delta += e; evict_deferred_delta += d;
            h
        };
        mgr.reupload.remove(&key);
        // own_full_res 契约（原 L1163）：sub-256 填充不得重绘
        // 当前正在上采样祖先的瓦片。
        let filler = result.width < 256 && key.2 > BASE_LAYER_ZOOM;
        if !filler {
            mgr.effective_tex.insert(key, tex_handle.clone());
            mgr.effective_uv.insert(key, [0.0, 0.0, 1.0, 1.0]);
            if let Some(mat_handle) = mgr.gpu_materials.get(&key) {
                if let Some(mat) = materials.get_mut(mat_handle) {
                    mat.base_color_texture = Some(tex_handle);
                    mat.base_color = Color::WHITE;
                }
            }
        }
        // 仅当实体已存在时才标记为已贴纹理（避免未生成时误记）。
        if mgr.tile_entities.contains_key(&key) { mgr.textured_tiles.insert(key); }
        tex_uploads += 1;
    }

    // 锐化通道：具有自身全分辨率 + 真实网格的上采样瓦片原子交换。
    // 找出已同时具备实体、真实网格与≥２５６纹理的上采样瓦片，换回自身高清网格与材质。
    let sharpen: Vec<TileKey> = mgr.upsampled.iter().filter(|k| {
        mgr.tile_entities.contains_key(*k) && mgr.gpu_meshes.contains_key(*k)
            && matches!(mgr.gpu_tex_size.get(*k), Some(&s) if s >= 256)
    }).copied().collect();
    for key in sharpen {
        // 原子换回自身高清网格与材质，并从 upsampled 集合移除。
        let ent = mgr.tile_entities[&key];
        if let (Some(mesh), Some(mat)) = (mgr.gpu_meshes.get(&key).cloned(), mgr.gpu_materials.get(&key).cloned()) {
            commands.entity(ent).insert((Mesh3d(mesh), MeshMaterial3d(mat)));
            mgr.upsampled.remove(&key);
        }
    }

    // 5) 预算化清理（MAX_TILE_ENTITIES 上限，细层优先 LRU）。
    // 实体数超上限时，优先淘汰非保护、不可见、非基根且最细层/最旧的瓦片。
    // 上限在相机手势期间被 `GestureBudget` 放宽（②：让被替换下来的旧级别
    // 瓦片活到新级别就绪）；常驻层（z ≤ RESIDENT_ZOOM）始终豁免。
    let mut removed = 0;
    let max_entities = budget.max_tile_entities();
    let max_despawns = budget.max_despawns_per_frame();
    if mgr.tile_entities.len() > max_entities {
        // 保护链：覆盖所有待定叶节点的祖先不得被淘汰。
        let prot = protected_ancestors(mgr);
        while mgr.tile_entities.len() > max_entities && removed < max_despawns {
            let len = mgr.hide_order.len();
            let Some(key) = mgr.hide_order.iter().enumerate()
                .filter(|(_, k)| mgr.tile_entities.contains_key(*k) && !prot.contains(*k)
                    && !mgr.visible_set.contains(*k) && !mgr.load_set.contains(*k)
                    && k.2 > RESIDENT_ZOOM)
                .max_by_key(|(idx, k)| (k.2, len - idx))
                .map(|(_, k)| *k)
            else { break };
            mgr.despawn_tile(&key, commands); removed += 1;
        }
        if mgr.tile_entities.len() > max_entities {
            // 若仍未降到上限：按生成序逐个淘汰有存活祖先的多余瓦片（淘汰不会留空洞）。
            let mut extras: Vec<(usize, TileKey)> = {
                let pos = |k: &TileKey| mgr.spawn_order.iter().position(|o| o == k).unwrap_or(usize::MAX);
                let mut v: Vec<(usize, TileKey)> = mgr.tile_entities.keys().copied()
                    .filter(|k| k.2 > RESIDENT_ZOOM && !mgr.visible_set.contains(k)
                        && !mgr.load_set.contains(k) && has_live_ancestor(mgr, k))
                    .map(|k| (pos(&k), k)).collect();
                v.sort(); v
            };
            for (_, key) in extras.drain(..) {
                if mgr.tile_entities.len() <= max_entities || removed >= max_despawns * 2 { break; }
                mgr.despawn_tile(&key, commands); removed += 1;
            }
        }
    }

    // 覆盖修复（仅稳定帧）。
    // 视图稳定时，补齐可见/加载集中丢失或因等待而饥饿的瓦片。
    if !view_changed {
        // missing：可见/加载集中既无实体又未排队的瓦片（最多补 32 个）。
        let missing: Vec<TileKey> = mgr.visible_set.iter().chain(mgr.load_set.iter())
            .filter(|k| !mgr.tile_entities.contains_key(k) && !mgr.queued.contains(k))
            .copied().take(32).collect();
        // starved：已排队却因网格/纹理/下载均未就位而饥饿的瓦片。
        let starved: Vec<TileKey> = {
            let cache = tex_rx.cache.lock().unwrap();
            mgr.visible_set.iter().filter(|k| {
                mgr.queued.contains(k) && !mgr.tile_entities.contains_key(k)
                    && !mgr.in_flight.contains(k) && !mgr.no_data.contains(k)
                    && !mgr.gpu_textures.contains_key(k) && !cache.contains_key(k)
            }).copied().collect()
        };
        if !missing.is_empty() || !starved.is_empty() {
            // 为丢失/饥饿瓦片重新排队网格构建与下载，并将 missing 重新加入生成队列。
            let mut repair_mesh: Vec<(TileKey, u32, Option<[f32; 4]>)> = Vec::new();
            let mut repair_dl: Vec<(TileKey, bool)> = Vec::new();
            { let cache = tex_rx.cache.lock().unwrap();
              for k in missing.iter().chain(starved.iter()) {
                if !mgr.gpu_meshes.contains_key(k) && !mgr.gpu_fb_meshes.contains_key(k) {
                    repair_mesh.push((*k, globe_lod::compute_segments(k.2), None));
                }
                if !cache.contains_key(k) && !mgr.gpu_textures.contains_key(k)
                    && !mgr.no_data.contains(k) && !mgr.in_flight.contains(k)
                    && mgr.retry_after.get(k).map_or(true, |t| std::time::Instant::now() >= *t) {
                    repair_dl.push((*k, false)); mgr.in_flight.insert(*k);
                }
              }
            }
            if !repair_mesh.is_empty() { start_mesh_builds(mesh_pipe, repair_mesh); }
            if !repair_dl.is_empty() { start_downloads(tex_rx, &repair_dl); }
            for k in missing { mgr.spawn_queue.push_back(k); mgr.queued.insert(k); }
        }
    }

    // wanted 刷新。
    // 汇总当前排队/可见/加载/已生成实体集合，告知 worker 不再跳过这些瓦片。
    { let mut w = tex_rx.wanted.lock().unwrap(); w.clear();
      w.extend(mgr.queued.iter().copied()); w.extend(mgr.visible_set.iter().copied());
      w.extend(mgr.load_set.iter().copied()); w.extend(mgr.tile_entities.keys().copied());
    }

    // PerfCounters 回写（与遗留双向）。
    // 将本帧各集合长度与上传/生成/淘汰计数回写到性能面板供外部读取。
    perf.in_flight = mgr.in_flight.len() as u32;
    perf.gpu_tex_order = mgr.gpu_tex_order.len() as u32;
    perf.tile_entities = mgr.tile_entities.len() as u32;
    perf.spawn_queue = mgr.spawn_queue.len() as u32;
    perf.backlog = mesh_pipe.backlog.len() as u32;
    perf.retry_after = mgr.retry_after.len() as u32;
    perf.load_set = mgr.load_set.len() as u32;
    perf.frame_mesh = mesh_uploads as u32;
    perf.frame_tex = tex_uploads as u32;
    perf.frame_spawn = spawns as u32;
    perf.frame_despawn = removed as u32;
    perf.stale_skips = perf.stale_skips.wrapping_add(stale_skips_delta);
    perf.evict_total = perf.evict_total.wrapping_add(evict_total_delta);
    perf.evict_deferred = perf.evict_deferred.wrapping_add(evict_deferred_delta);
}

// ── GPU 缓存淘汰（三条不变式） ────────────────────────────────

/// 超过上限时按 FIFO 淘汰最旧的缓存 GPU 句柄。
///
/// 三条不变式（源自 `cesium-pipeline` GpuCache 设计，#25）：
/// 1. **常驻层豁免** — z ≤ `RESIDENT_ZOOM`（含 BASE_LAYER 的 z ≤ 3）永不淘汰
///    （永久回退层；③把常驻层从 z ≤ 3 提到 z ≤ 4）
/// 2. **活跃实体 push_back 延迟** — 花屏防護：当已生成实体仍引用
///    Assets 数据时释放它会破坏网格分配器（水平条纹）
/// 3. **终止性** — MAX_TILE_ENTITIES(1800) << MAX_GPU_CACHE_ENTRIES(3000)
///    保证始终存在可淘汰（已死）的条目；手势期实体上限放宽到 2250
///    （`GESTURE_ENTITY_SLACK`）后仍远低于 3000，不变量成立。
///
/// 原始：`dynamic_globe.rs:1476-1502`（逐字節保留）。
pub(crate) fn evict_gpu_cache(mgr: &mut TileManager) -> (u32, u32) {
    // evicted = 实际淘汰数；deferred = 因仍活跃而延迟的个数。
    let mut evicted: u32 = 0;
    let mut deferred: u32 = 0;
    // 仅当显存纹理 LRU 长度超上限时才淘汰。
    while mgr.gpu_tex_order.len() > DefaultBudget::MAX_GPU_CACHE_ENTRIES {
        let Some(old) = mgr.gpu_tex_order.pop_front() else {
            break;
        };
        if old.2 <= RESIDENT_ZOOM {
            // 不变式 1：永久回退层 —— 永不淘汰。
            continue;
        }
        if mgr.tile_entities.contains_key(&old) {
            // 不变式 2：仍在渲染 —— 延迟淘汰（花屏防護）。
            mgr.gpu_tex_order.push_back(old);
            deferred += 1;
            continue;
        }
        // 无实体引用：真正释放其纹理/材质/网格/有效纹理/尺寸等 GPU 资源。
        mgr.gpu_textures.remove(&old);
        mgr.gpu_materials.remove(&old);
        mgr.gpu_meshes.remove(&old);
        mgr.gpu_fb_meshes.remove(&old);
        mgr.effective_tex.remove(&old);
        mgr.gpu_tex_size.remove(&old);
        evicted += 1;
    }
    (evicted, deferred)
}

// ── 显示集辅助函数 ──────────────────────────────────────────────────

/// 显示可以安全移交给的划分叶节点：一个展示真实像素的活跃实体
///（自身全分辨率纹理，或刻意的纯色 /
/// 无数据继承）—— 绝非拉伸祖先的上采样。
///
/// 原始：`dynamic_globe.rs:1435-1446`（逐字節保留）。
pub(crate) fn replacement_ready(mgr: &TileManager, key: &TileKey) -> bool {
    // 无实体则尚未就绪。
    if !mgr.tile_entities.contains_key(key) {
        return false;
    }
    // 纯色/无数据均为刻意的真实展示，视为就绪。
    if mgr.solid_tiles.contains(key) || mgr.no_data.contains(key) {
        return true;
    }
    // 拉伸祖先的上采样不算就绪。
    if mgr.upsampled.contains(key) {
        return false;
    }
    // 否则需自身纹理≥２５６才算真正展示真实像素。
    matches!(mgr.gpu_tex_size.get(key), Some(&s) if s >= 256)
}

/// 当 `key` 的任一祖先拥有仍覆盖其区域的已生成实体时为真，
/// 因此 `key` 可被安全跳过/淘汰而不会留出空洞。
///
/// 原始：`dynamic_globe.rs:1451-1462`（逐字節保留）。
pub(crate) fn has_live_ancestor(mgr: &TileManager, key: &TileKey) -> bool {
    // 逐级上移父瓦片，任一祖先有实体即返回 true。
    let (mut x, mut y, mut z) = *key;
    while z > 0 {
        x >>= 1;
        y >>= 1;
        z -= 1;
        if mgr.tile_entities.contains_key(&(x, y, z)) {
            return true;
        }
    }
    false
}

/// 每个尚无已生成实体的 render/load 集瓦片的祖先键。
/// 保护覆盖每个待定叶节点的整条祖先链。
///
/// 原始：`dynamic_globe.rs:631-650`（逐字節保留）。
pub(crate) fn protected_ancestors(mgr: &TileManager) -> HashSet<TileKey> {
    let mut prot: HashSet<TileKey> = HashSet::new();
    // 对每个尚无实体的可见/加载瓦片，向上标记其整条祖先链。
    for v in mgr
        .visible_set
        .iter()
        .chain(mgr.load_set.iter())
        .filter(|v| !mgr.tile_entities.contains_key(v))
    {
        let (mut x, mut y, mut z) = *v;
        while z > 0 {
            x >>= 1;
            y >>= 1;
            z -= 1;
            if !prot.insert((x, y, z)) {
                break; // 更高层祖先已注册
            }
        }
    }
    prot
}

// ── 后台网格构建 ───────────────────────────────────────────────

/// 在 worker 线程上构建瓦片网格；结果经管线通道回流
/// 并在帧预算内上传到 GPU。
///
/// 原始：`dynamic_globe.rs:2076-2108`（逐字節保留）。
pub(crate) fn start_mesh_builds(pipe: &mut MeshPipeline, jobs: Vec<(TileKey, u32, Option<[f32; 4]>)>) {
    let tx = pipe.tx.clone();

    // 总调度线程：将任务均匀分片到 DOWNLOAD_THREADS 个并行构建 worker。
    std::thread::spawn(move || {
        let chunks: Vec<Vec<(TileKey, u32, Option<[f32; 4]>)>> = {
            // 按索引取模分桶，使各 worker 负载大致均衡。
            let mut c: Vec<Vec<(TileKey, u32, Option<[f32; 4]>)>> =
                (0..DefaultBudget::DOWNLOAD_THREADS).map(|_| Vec::new()).collect();
            for (i, job) in jobs.into_iter().enumerate() {
                c[i % DefaultBudget::DOWNLOAD_THREADS].push(job);
            }
            c
        };

        let mut handles = Vec::new();
        for chunk in chunks {
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                // 逐任务构建网格：有 UV 回退矩形则用 UV 变体，否则常规；结果回传通道。
                for (key, segments, uv) in chunk {
                    let mesh = match uv {
                        Some(rect) => create_tile_mesh_uv(key.0, key.1, key.2, segments, rect),
                        None => create_tile_mesh(key.0, key.1, key.2, segments),
                    };
                    if tx.send((key, mesh, uv.is_some())).is_err() {
                        return;
                    }
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
    });
}

// ── Bing Maps 下载 ──────────────────────────────────────────────────

/// 从瓦片坐标进行 Bing Maps quadkey 编码。
/// 原始：`dynamic_globe.rs:2112-2126`（逐字節保留）。
pub fn tile_to_quadkey(x: u32, y: u32, level: u32) -> String {
    // 从最高层到最低层逐位拼接：每位由 x/y 对应位组合成 0..3 的象限数码。
    let mut qk = String::with_capacity(level as usize);
    for i in (0..level).rev() {
        let mut d = 0u8;
        let mask = 1 << i;
        if (x & mask) != 0 {
            d |= 1;
        }
        if (y & mask) != 0 {
            d |= 2;
        }
        qk.push_str(&d.to_string());
    }
    qk
}

/// 向常驻 worker 池投递下载任务。非阻塞：任务排队，
/// worker 在出队时跳过陈旧任务（不在 `wanted` 中的）。
///
/// 原始：`dynamic_globe.rs:2133-2137`（逐字節保留）。
pub(crate) fn start_downloads(tex_rx: &TextureReceiver, tiles: &[(TileKey, bool)]) {
    // 逐个将（瓦片键, 是否降采样）任务投递到常驻 worker 池，非阻塞。
    for &(key, downscale) in tiles {
        let _ = tex_rx.job_tx.send((key, downscale));
    }
}

// ── 下载 worker（离线接线：M3 门槛 L127） ───────────────────────

/// 常驻下载池 worker：整个生命周期持有一个 ureq agent
///（连接池在多次获取间保持瓦片服务器连接温热）并持续
/// 拉取任务，直到投递通道关闭。
///
/// ## 离线接线（M3 门槛 L127）
/// - `OFFLINE_IMAGERY_ROOT` 已设置 → 从磁盘读取 `{root}/{z}/{x}/{y}.png`
/// - `STRICT_OFFLINE=1` + 任何 https URL → 同步 panic（无回退）
/// - 两者均未设置 → 经 `fetch_gated` 访问在线 Bing（共享 ureq keep-alive 池）
///
/// 原始：`dynamic_globe.rs:2143-2285`（逐字節保留，外加离线接线）。
pub fn download_worker(
    job_rx: Arc<Mutex<mpsc::Receiver<(TileKey, bool)>>>,
    tx: mpsc::Sender<TileDownloadResult>,
    wanted: Arc<Mutex<HashSet<TileKey>>>,
) {
    // 在 worker 启动时一次性解析离线配置（env 是进程全局的，
    // 进入 main() 后不可变 —— 无 TOCTOU 风险）。
    // 一次性解析离线/严格两个环境变量，后续循环直接复用。
    let offline_root = feature_flags::offline_imagery_root();
    let strict = feature_flags::strict_offline();

    loop {
        // 阻塞式接收一个下载任务；投递通道关闭则退出 worker。
        let job = job_rx.lock().unwrap().recv();
        let Ok(((px, py, pz), downscale)) = job else {
            return;
        };
        // wanted 陈旧跳过：快速平移会在几帧内使整批任务陈旧；
        // 跳过无人会查看的获取。
        if !wanted.lock().unwrap().contains(&(px, py, pz)) {
            let _ = tx.send(TileDownloadResult {
                x: px, y: py, z: pz,
                rgba_data: Vec::new(), width: 0, height: 0, mip_levels: 0,
                placeholder: false, aborted: true, failed: false,
            });
            continue;
        }

        // ── 离线路径：从磁盘读取 ─────────────────────────────────
        // 若磁盘存在该瓦片 PNG：解码并检测占位图，必要时降采样后构建 mip 链回传。
        if let Some(root) = &offline_root {
            let path = root.join(pz.to_string()).join(px.to_string())
                .join(format!("{}.png", py));
            match std::fs::read(&path) {
                // 磁盘命中：解码并走与在线相同的占位/降采样/mip 链流程。
                Ok(data) => {
                    if let Ok(img) = image::load_from_memory(&data) {
                        let rgba = img.to_rgba8();
                        let (is_ph, ph_avg, ph_maxd) = is_placeholder_tile(&rgba);
                        if is_ph {
                            eprintln!(
                                "[nodata] z{pz} x{px} y{py} verdict=placeholder avg={ph_avg:.2} maxd={ph_maxd}"
                            );
                            let _ = tx.send(TileDownloadResult {
                                x: px, y: py, z: pz,
                                rgba_data: Vec::new(), width: 0, height: 0, mip_levels: 0,
                                placeholder: true, aborted: false, failed: false,
                            });
                            continue;
                        }
                        // downscale 且宽>128：三角滤波降采样到 128×128 以节省带宽/显存。
                        let (rgba, w, h) = if downscale && rgba.width() > 128 {
                            let small = image::DynamicImage::ImageRgba8(rgba).resize(
                                128, 128, image::imageops::FilterType::Triangle,
                            );
                            let r = small.to_rgba8();
                            let (w, h) = r.dimensions();
                            (r, w, h)
                        } else {
                            let (w, h) = rgba.dimensions();
                            (rgba, w, h)
                        };
                        let (chain, levels) = build_mip_chain(rgba.into_raw(), w, h);
                        let _ = tx.send(TileDownloadResult {
                            x: px, y: py, z: pz,
                            rgba_data: chain, width: w, height: h, mip_levels: levels,
                            placeholder: false, aborted: false, failed: false,
                        });
                        continue;
                    }
                    // 解码失败 —— 落入失败结果。
                }
                Err(_) => {
                    // 文件未找到 —— 视为无数据（占位图）。
                    let _ = tx.send(TileDownloadResult {
                        x: px, y: py, z: pz,
                        rgba_data: Vec::new(), width: 0, height: 0, mip_levels: 0,
                        placeholder: true, aborted: false, failed: false,
                    });
                    continue;
                }
            }
            // 若执行到此，则解码失败。
            let _ = tx.send(TileDownloadResult {
                x: px, y: py, z: pz,
                rgba_data: Vec::new(), width: 0, height: 0, mip_levels: 0,
                placeholder: false, aborted: false, failed: true,
            });
            continue;
        }

        // ── 在线路径：磁盘缓存优先，未命中经进程级共享连接池下载并落盘 ──
        // 由瓦片坐标编码 quadkey；fetch_tile 内部先查磁盘缓存（命中即零网络），
        // 未命中才走共享 ureq Agent（连接温复用，不再逐瓦片新建 TCP+TLS 握手），
        // 成功下载后原子落盘供 2D/3D、跨会话反复复用。
        let qk = tile_to_quadkey(px, py, pz);

        // STRICT_OFFLINE：禁止任何网络回退（M3 门槛 L127）—— 命中磁盘缓存
        // 之外的获取都是硬错误，直接 panic 以暴露配置缺失。
        if strict {
            panic!(
                "STRICT_OFFLINE=1: network fetch attempted for tile z{pz} x{px} y{py} — \
                 set OFFLINE_IMAGERY_ROOT to provide tiles from disk",
            );
        }

        // 退避重试：多数请求走磁盘命中或温连接即时返回；仅网络失败才退避。
        let mut delivered = false;
        for attempt in 0..3u32 {
            // 重试前按 150ms×2^attempt 指数退避。
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(150u64 << attempt));
            }
            let bytes = crate::tile_cache::fetch_tile(px, py, pz, &qk);
            let fetched = bytes.as_deref().and_then(|b| image::load_from_memory(b).ok());
            if let Some(img) = fetched {
                // 解码成功：先做占位图检测（全均匀色视为无数据）。
                let rgba = img.to_rgba8();
                let (is_ph, ph_avg, ph_maxd) = is_placeholder_tile(&rgba);
                if is_ph {
                    eprintln!(
                        "[nodata] z{pz} x{px} y{py} verdict=placeholder avg={ph_avg:.2} maxd={ph_maxd}"
                    );
                    let _ = tx.send(TileDownloadResult {
                        x: px, y: py, z: pz,
                        rgba_data: Vec::new(), width: 0, height: 0, mip_levels: 0,
                        placeholder: true, aborted: false, failed: false,
                    });
                    delivered = true;
                    break;
                }
                // 非占位图：按需降采样，构建 mip 链后回传成功结果。
                let (rgba, w, h) = if downscale && rgba.width() > 128 {
                    let small = image::DynamicImage::ImageRgba8(rgba).resize(
                        128, 128, image::imageops::FilterType::Triangle,
                    );
                    let r = small.to_rgba8();
                    let (w, h) = r.dimensions();
                    (r, w, h)
                } else {
                    let (w, h) = rgba.dimensions();
                    (rgba, w, h)
                };
                let (chain, levels) = build_mip_chain(rgba.into_raw(), w, h);
                let _ = tx.send(TileDownloadResult {
                    x: px, y: py, z: pz,
                    rgba_data: chain, width: w, height: h, mip_levels: levels,
                    placeholder: false, aborted: false, failed: false,
                });
                delivered = true;
                break;
            }
            // 取到字节但解码失败（磁盘缓存损坏）：删除坏文件，下轮重新获取。
            if bytes.is_some() {
                let _ = std::fs::remove_file(crate::tile_cache::tile_path(pz, px, py));
            }
        }
        // 三次重试均未送达：报 retries-exhausted 并回传失败结果。
        if !delivered {
            eprintln!("[nodata] z{pz} x{px} y{py} verdict=retries-exhausted");
            let _ = tx.send(TileDownloadResult {
                x: px, y: py, z: pz,
                rgba_data: Vec::new(), width: 0, height: 0, mip_levels: 0,
                placeholder: false, aborted: false, failed: true,
            });
        }
    }
}

// ── 基础球合成 ────────────────────────────────────────────────

/// 当每个基础层（z=3）瓦片都拥有纹理或无数据判定后，
/// 将它们烘焙成一张 1024×1024 的墨卡托合成图并披覆
/// 到底球上。
///
/// 原始：`dynamic_globe.rs:1920-1991`（逐字節保留）。
pub(crate) fn run_base_sphere_composite(
    state: &mut BaseSphereComposite,
    mgr: &mut TileManager,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    sphere: &Query<&MeshMaterial3d<StandardMaterial>, With<BaseSphereMarker>>,
) {
    // 已完成合成则直接返回（一次性任务）。
    if state.done {
        return;
    }
    // 若烘焙已投递：尝试取回结果，取到则给底球贴合成图并标记完成。
    if let Some(rx) = &state.rx {
        let Ok((chain, levels)) = rx.lock().unwrap().try_recv() else {
            return;
        };
        // 将烘焙好的 mip 链包装为受限纹理句柄。
        let handle =
            base_sphere::make_clamped_image(images, chain, COMPOSITE_SIZE, COMPOSITE_SIZE, levels);
        // 找到底球实体材质，换上合成影像作为基础色纹理。
        mgr.composite_texture = Some(handle.clone());
        if let Ok(mat) = sphere.get_single() {
            if let Some(m) = materials.get_mut(&mat.0) {
                m.base_color = Color::WHITE;
                m.base_color_texture = Some(handle);
            }
        }
        state.rx = None;
        state.done = true;
        return;
    }

    // 尚未投递烘焙：列出 8×8 个基根（z=3）瓦片，全部就绪（有纹理或无数据）才启动烘焙。
    let keys: Vec<TileKey> = (0..8u32)
        .flat_map(|x| (0..8u32).map(move |y| (x, y, BASE_LAYER_ZOOM)))
        .collect();
    if !keys
        .iter()
        .all(|k| mgr.gpu_textures.contains_key(k) || mgr.no_data.contains(k))
    {
        return;
    }

    // 收集 128 px 块（盒式降采样的全分辨率瓦片）。
    let mut blocks: Vec<(u32, u32, Vec<u8>)> = Vec::with_capacity(keys.len());
    for k in keys {
        let block = mgr
            .gpu_textures
            .get(&k)
            .and_then(|h| images.get(h))
            .map(|img| {
                let w = img.texture_descriptor.size.width;
                let h = img.texture_descriptor.size.height;
                // 无纹理的基根瓦片：用海洋纯色块占位。
                base_sphere::box_downsample(&img.data[..(w * h * 4) as usize], w, COMPOSITE_TILE)
            })
            .unwrap_or_else(base_sphere::ocean_block);
        blocks.push((k.0, k.1, block));
    }

    state.rx = Some(base_sphere::spawn_composite_bake(blocks));
}

// ── 测试 ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_gesture_budget_equals_default_budget() {
        let idle = GestureBudget::for_gesture(false);
        let d = DefaultBudget;
        assert_eq!(idle.max_mesh_uploads_per_frame(), d.max_mesh_uploads_per_frame());
        assert_eq!(idle.max_spawns_per_frame(), d.max_spawns_per_frame());
        assert_eq!(idle.max_texture_uploads_per_frame(), d.max_texture_uploads_per_frame());
        assert_eq!(idle.max_despawns_per_frame(), d.max_despawns_per_frame());
        assert_eq!(idle.max_tile_entities(), d.max_tile_entities());
        // 契约字段在任何倍数下都不变。
        assert_eq!(idle.base_layer_zoom(), d.base_layer_zoom());
        assert_eq!(idle.max_gpu_cache_entries(), d.max_gpu_cache_entries());
        assert_eq!(idle.download_threads(), d.download_threads());
    }

    #[test]
    fn gesture_budget_scales_rate_budgets_and_relaxes_entity_cap() {
        let busy = GestureBudget::for_gesture(true);
        let d = DefaultBudget;
        assert_eq!(
            busy.max_mesh_uploads_per_frame(),
            d.max_mesh_uploads_per_frame() * GESTURE_BUDGET_SCALE
        );
        assert_eq!(busy.max_spawns_per_frame(), d.max_spawns_per_frame() * GESTURE_BUDGET_SCALE);
        assert_eq!(
            busy.max_texture_uploads_per_frame(),
            d.max_texture_uploads_per_frame() * GESTURE_BUDGET_SCALE
        );
        assert_eq!(busy.max_despawns_per_frame(), d.max_despawns_per_frame() * GESTURE_BUDGET_SCALE);
        // ②：拖动期间实体上限放宽 GESTURE_ENTITY_SLACK。
        assert_eq!(busy.max_tile_entities(), d.max_tile_entities() + GESTURE_ENTITY_SLACK);
        assert!(busy.max_tile_entities() > d.max_tile_entities());
        // 契约字段不随手势变化。
        assert_eq!(busy.base_layer_zoom(), d.base_layer_zoom());
        assert_eq!(busy.max_gpu_cache_entries(), d.max_gpu_cache_entries());
    }

    #[test]
    fn entity_caps_stay_below_gpu_cache_so_eviction_terminates() {
        let busy = GestureBudget::for_gesture(true);
        // 终止性不变量：即使手势期放宽，实体上限仍远小于 GPU 缓存上界，
        // 因此 gpu_tex_order 中总有可驱逐（已死）条目。
        assert!(busy.max_tile_entities() < busy.max_gpu_cache_entries());
        assert!(
            DefaultBudget.max_tile_entities() + GESTURE_ENTITY_SLACK
                < DefaultBudget.max_gpu_cache_entries()
        );
    }

    #[test]
    fn gesture_budget_scale_is_never_zero() {
        // 预算为 0 会让管线永不推进；`new` 必须钳到 1。
        let clamped = GestureBudget::new(0);
        let d = DefaultBudget;
        assert_eq!(clamped.max_spawns_per_frame(), d.max_spawns_per_frame());
        assert_eq!(clamped.max_mesh_uploads_per_frame(), d.max_mesh_uploads_per_frame());
    }

    #[test]
    fn resident_zoom_tops_base_layer_and_is_eviction_exempt() {
        // ③：常驻层必须在 composite/契约锁定的核心层之上，否则没有任何新效果。
        const { assert!(RESIDENT_ZOOM > BASE_LAYER_ZOOM) };
        // 常驻实体数（z ≤ RESIDENT_ZOOM）必须远小于 GPU 缓存上界，
        // 否则常驻层自身会把缓存挤满。
        let count: u32 = (1..=RESIDENT_ZOOM).map(|z| 1u32 << (2 * z)).sum();
        assert_eq!(count, 340, "z=1..=4 共 4+16+64+256 个常驻瓦片");
        assert!((count as usize) < DefaultBudget.max_gpu_cache_entries());
    }
}
