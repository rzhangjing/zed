//! M1.5 薄壳动态地球 —— ImageryKey/ImageryPayload 特化，
//! 将瓦片获取/解码/GPU 缓存/淘汰委托给 `cesium-pipeline`
//! 核心（经 `fetch_gated` → 共享 ureq keep-alive 池）以及抽取的辅助
//! 模块（`globe_lod`、`globe_pipeline`、`globe_textures`、`base_sphere`）。
//!
//! ## 保留的隐式契约（逐字节/带注释）：
//! - `BaseSphereMarker`（从 `base_sphere` 重新导出）
//! - `DynamicGlobePlugin` —— 相同的系统链，相同的 Startup/Update 注册
//! - `BASE_LAYER_ZOOM = 3` 是 composite/契约锁定的核心常驻层，
//!   `RESIDENT_ZOOM = 4` 是 ③ 追加的常驻回退层（经 `DefaultBudget` 豁免）
//! - wanted 陈旧跳过（worker 在获取前检查 wanted 集合）
//! - `own_full_res`（L837-841 / L1163 语义在 `globe_pipeline` 中保留）
//! - `evict_gpu_cache` 三条不变式：常驻层（z ≤ RESIDENT_ZOOM）豁免、活跃
//!   push_back 延迟（花屏防护），终止 2250 << 3000（经 `DefaultBudget`）
//! - `TilePipelineSet` pub(crate)（perf_trace 跨模块契约）
//! - `PerfCounters` 回写（与遗留双向）
//!
//! ## GenericPipeline 遗留问题的解决（M1.4 → M1.5）：
//! (a) Decoder(bytes)→Payload 缺少 key 上下文：**通过特化解决**
//!     —— 影像解码（load_from_memory + 占位图 + mip 链）是
//!     与 key 无关的；降采样是 worker 通道中的逐任务标志。
//! (b) K:Copy 排除 tileset 的 Vec/String key：**不适用** —— 影像
//!     TileKey=(u32,u32,u32) 确为 Copy；tileset 是独立的消费者。
//! (c) 跨帧持久化需要新的 Resource：**已解决** —— TileManager +
//!     TextureReceiver 本身就是 Bevy Resource（天然跨帧）；它们封装了
//!     管线的 NetworkBackend（共享 ureq 池）+ DefaultBudget。
//!
//! ## 离线接线（M3 门槛 L127）：
//! 设置 `OFFLINE_IMAGERY_ROOT` 后，下载 worker 从磁盘读取瓦片
//! （`{root}/{z}/{x}/{y}.png`）而非 Bing。`STRICT_OFFLINE=1` 在任何
//! https URL 上 panic（无网络回退）。

// 冻结的遗留黄金路径风格债务；局部 allow 以满足严格 CI clippy 门槛
#![allow(clippy::unnecessary_map_or, clippy::type_complexity, clippy::too_many_arguments)]

use bevy::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::base_sphere::BaseSphereComposite;
use crate::globe_lod::{self, LodContext, TileKey, RESIDENT_ZOOM};
use crate::globe_pipeline;
use crate::orbit_camera::{CameraGesture, OrbitState};
use crate::perf_counters::PerfCounters;
use crate::tile_mesh::GlobeTile;
use cesium_pipeline::DefaultBudget;

// ── 类型 ────────────────────────────────────────────────────────────────

/// 缓存的瓦片图像数据（mip 链式 RGBA 字节 + 尺寸）。
pub(crate) struct CachedTexture {
    pub rgba_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
}

// ── 资源 ────────────────────────────────────────────────────────────

#[derive(Resource)]
pub(crate) struct TileManager {
    pub tile_entities: HashMap<TileKey, Entity>,
    pub textured_tiles: HashSet<TileKey>,
    pub current_zoom: u32,
    pub last_distance: f32,
    pub initialized: bool,
    pub visible_set: HashSet<TileKey>,
    pub partition_set: HashSet<TileKey>,
    pub load_set: HashSet<TileKey>,
    pub spawn_queue: VecDeque<TileKey>,
    pub queued: HashSet<TileKey>,
    pub gpu_meshes: HashMap<TileKey, Handle<Mesh>>,
    pub gpu_materials: HashMap<TileKey, Handle<StandardMaterial>>,
    pub gpu_textures: HashMap<TileKey, Handle<Image>>,
    pub no_data: HashSet<TileKey>,
    pub gpu_fb_meshes: HashMap<TileKey, Handle<Mesh>>,
    pub effective_tex: HashMap<TileKey, Handle<Image>>,
    pub effective_uv: HashMap<TileKey, [f32; 4]>,
    pub solid_tiles: HashSet<TileKey>,
    pub upsampled: HashSet<TileKey>,
    pub pending_fb_builds: HashSet<TileKey>,
    pub gpu_tex_order: VecDeque<TileKey>,
    pub gpu_tex_size: HashMap<TileKey, u32>,
    pub reupload: HashSet<TileKey>,
    pub in_flight: HashSet<TileKey>,
    pub retry_after: HashMap<TileKey, std::time::Instant>,
    pub spawn_order: VecDeque<TileKey>,
    pub hide_order: VecDeque<TileKey>,
    pub view_changed_this_frame: bool,
    /// 基础球 composite（z=3 的 8×8 全球图集）纹理句柄；烘焙完成后由
    /// `globe_pipeline::run_base_sphere_composite` 写入。无纹理祖先可继的
    /// 瓦片以它为"根祖先"取图集子矩形，避免铺深色纯色块。
    pub composite_texture: Option<Handle<Image>>,
}

impl Default for TileManager {
    /// 构造空管理的瓦片状态：各集合/映射初始化为空，层级取最小值，
    /// 距离/初始化/视图变更标志归零，等待启动系统首次填充。
    fn default() -> Self {
        Self {
            tile_entities: HashMap::new(), textured_tiles: HashSet::new(),
            current_zoom: globe_lod::MIN_ZOOM, last_distance: 0.0, initialized: false,
            visible_set: HashSet::new(), partition_set: HashSet::new(), load_set: HashSet::new(),
            spawn_queue: VecDeque::new(), queued: HashSet::new(),
            gpu_meshes: HashMap::new(), gpu_materials: HashMap::new(), gpu_textures: HashMap::new(),
            no_data: HashSet::new(), gpu_fb_meshes: HashMap::new(),
            effective_tex: HashMap::new(), effective_uv: HashMap::new(),
            solid_tiles: HashSet::new(), upsampled: HashSet::new(),
            pending_fb_builds: HashSet::new(), gpu_tex_order: VecDeque::new(),
            gpu_tex_size: HashMap::new(), reupload: HashSet::new(), in_flight: HashSet::new(),
            retry_after: HashMap::new(), spawn_order: VecDeque::new(), hide_order: VecDeque::new(),
            view_changed_this_frame: false, composite_texture: None,
        }
    }
}

impl LodContext for TileManager {
    /// 该瓦片是否已生成对应实体（供 LOD 判断是否可跳过重复生成）。
    fn has_entity(&self, key: &TileKey) -> bool { self.tile_entities.contains_key(key) }
    /// 查询该瓦片当前 GPU 纹理边长（像素）；未缓存纹理时返回 `None`。
    fn tex_size(&self, key: &TileKey) -> Option<u32> { self.gpu_tex_size.get(key).copied() }
}

impl TileManager {
    /// 反生成一个瓦片实体；GPU 句柄保持缓存以便廉价再生成。
    pub fn despawn_tile(&mut self, key: &TileKey, commands: &mut Commands) {
        if let Some(entity) = self.tile_entities.remove(key) {
            commands.entity(entity).despawn();
            self.textured_tiles.remove(key);
            self.upsampled.remove(key);
            self.pending_fb_builds.remove(key);
            self.retry_after.remove(key);
            self.spawn_order.retain(|k| k != key);
            self.hide_order.retain(|k| k != key);
        }
    }
}

#[derive(Resource)]
pub(crate) struct TextureReceiver {
    pub rx: Mutex<mpsc::Receiver<globe_pipeline::TileDownloadResult>>,
    pub job_tx: mpsc::Sender<(TileKey, bool)>,
    pub cache: Arc<Mutex<HashMap<TileKey, CachedTexture>>>,
    pub wanted: Arc<Mutex<HashSet<TileKey>>>,
}

impl Default for TextureReceiver {
    /// 建立纹理接收通道并拉起常驻下载 worker 池：主线程从 `rx` 轮询结果，
    /// worker 从共享 `job_rx` 取任务、用 `wanted` 集合跳过陈旧请求。
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let (job_tx, job_rx) = mpsc::channel::<(TileKey, bool)>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let wanted = Arc::new(Mutex::new(HashSet::new()));
        // 常驻 worker 池：16 个线程，带共享 ureq keep-alive 池
        // （经 fetch_gated 委托给 cesium-pipeline 的 UreqBackend）。
        for _ in 0..DefaultBudget::DOWNLOAD_THREADS {
            let job_rx = job_rx.clone();
            let tx = tx.clone();
            let wanted = wanted.clone();
            std::thread::spawn(move || globe_pipeline::download_worker(job_rx, tx, wanted));
        }
        Self { rx: Mutex::new(rx), job_tx, cache, wanted }
    }
}

#[derive(Resource)]
pub(crate) struct MeshPipeline {
    pub tx: mpsc::Sender<(TileKey, Mesh, bool)>,
    pub rx: Mutex<mpsc::Receiver<(TileKey, Mesh, bool)>>,
    pub backlog: VecDeque<(TileKey, Mesh, bool)>,
}

impl Default for MeshPipeline {
    /// 建立网格构建结果的通道：后台线程发来成品网格，主线程经 `backlog`
    /// 按预算逐帧消化，避免单帧插入过多网格导致卡顿。
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx: Mutex::new(rx), backlog: VecDeque::new() }
    }
}

// ── 插件 ───────────────────────────────────────────────────────────────

/// 瓦片管线链的 SystemSet 标记（pub(crate) —— perf_trace
/// 跨模块契约，自黄金路径 L300-301 保留）。
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TilePipelineSet;

pub struct DynamicGlobePlugin;

impl Plugin for DynamicGlobePlugin {
    /// 插件装配入口：初始化瓦片管理/纹理接收/网格管线三大资源与基础球合成，
    /// 挂载启动初次生成，并把三段更新系统串成 chain 归入 `TilePipelineSet`。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        app.init_resource::<TileManager>()
            .init_resource::<TextureReceiver>()
            .init_resource::<MeshPipeline>()
            .init_resource::<BaseSphereComposite>()
            .add_systems(Startup, initial_spawn)
            .add_systems(
                Update,
                (view_dependent_update, process_pipeline, sync_visibility)
                    .chain()
                    .in_set(TilePipelineSet),
            )
            .add_systems(Update, base_sphere_composite_system);
    }
}

// 重新导出，使 main.rs 的导入路径保持不变。
pub use crate::base_sphere::BaseSphereMarker;

// ── 启动 ──────────────────────────────────────────────────────────────

fn initial_spawn(
    mut mgr: ResMut<TileManager>,
    mut mesh_pipe: ResMut<MeshPipeline>,
    tex_rx: Res<TextureReceiver>,
    orbit: Res<OrbitState>,
    windows: Query<&Window>,
) {
    let (lat_rad, lon_rad) = globe_lod::compute_sub_camera_point(&orbit);
    let (visible, load) = globe_lod::compute_visible_tiles(
        lat_rad, lon_rad, orbit.distance as f64, globe_lod::focal_pixels(&windows), &*mgr,
    );
    // 取可见集合中最细层级，作为本帧 current_zoom 基准。
    let finest = visible.iter().map(|t| t.0 .2).max().unwrap_or(globe_lod::MIN_ZOOM);
    mgr.visible_set = visible.iter().map(|&(k, _)| k).collect();
    mgr.partition_set = mgr.visible_set.clone();
    mgr.load_set = load.iter().map(|&(k, _)| k).collect();

    globe_pipeline::enqueue_tiles(&mut mgr, &mut mesh_pipe, &tex_rx, &visible);
    globe_pipeline::enqueue_tiles(&mut mgr, &mut mesh_pipe, &tex_rx, &load);

    // 永久粗级回退层（z ≤ RESIDENT_ZOOM=4；其中 z ≤ BASE_LAYER_ZOOM=3 是
    // composite 8×8 图集与 BudgetPolicy 契约锁定的核心层，z=4 是 ③ 追加的
    // 额外常驻层）。re-queue 时给最高优先级，保证启动即补齐。
    let mut base: Vec<(TileKey, f32)> = Vec::new();
    for z in 1..=RESIDENT_ZOOM {
        for y in 0..(1u32 << z) {
            for x in 0..(1u32 << z) {
                base.push(((x, y, z), 100.0));
            }
        }
    }
    globe_pipeline::enqueue_tiles(&mut mgr, &mut mesh_pipe, &tex_rx, &base);
    mgr.current_zoom = finest;
    mgr.last_distance = orbit.distance;
    mgr.initialized = true;
}

// ── 视图相关更新 ────────────────────────────────────────────────

fn view_dependent_update(
    orbit: Res<OrbitState>,
    windows: Query<&Window>,
    mut mgr: ResMut<TileManager>,
    mut mesh_pipe: ResMut<MeshPipeline>,
    tex_rx: Res<TextureReceiver>,
) {
    // 启动系统尚未跑过一次时不做增量更新。
    if !mgr.initialized { return; }
    let (lat_rad, lon_rad) = globe_lod::compute_sub_camera_point(&orbit);
    let (new_visible, new_load) = globe_lod::compute_visible_tiles(
        lat_rad, lon_rad, orbit.distance as f64, globe_lod::focal_pixels(&windows), &*mgr,
    );
    let finest = new_visible.iter().map(|t| t.0 .2).max().unwrap_or(mgr.current_zoom);
    let new_set: HashSet<TileKey> = new_visible.iter().map(|&(k, _)| k).collect();
    let new_load_set: HashSet<TileKey> = new_load.iter().map(|&(k, _)| k).collect();

    let old_set = std::mem::take(&mut mgr.visible_set);
    let (display, partition_changed) = globe_lod::compute_display_set(
        &old_set, &new_set, &new_load_set, &mgr.partition_set, &mgr.load_set,
        &|k| globe_pipeline::replacement_ready(&mgr, k),
    );
    mgr.partition_set = new_set.clone();
    mgr.visible_set = display;
    mgr.load_set = new_load_set;

    globe_pipeline::enqueue_tiles(&mut mgr, &mut mesh_pipe, &tex_rx, &new_visible);
    globe_pipeline::enqueue_tiles(&mut mgr, &mut mesh_pipe, &tex_rx, &new_load);

    // 隐藏瓦片的 LRU 维护。
    let (vis, load, ents, hide) = {
        let m = &mut *mgr;
        (&m.visible_set, &m.load_set, &m.tile_entities, &mut m.hide_order)
    };
    hide.retain(|k| !vis.contains(k) && !load.contains(k) && ents.contains_key(k));
    let queued_hide: HashSet<TileKey> = hide.iter().copied().collect();
    for k in ents.keys() {
        if !vis.contains(k) && !load.contains(k) && !queued_hide.contains(k) {
            hide.push_back(*k);
        }
    }
    mgr.current_zoom = finest;
    mgr.last_distance = orbit.distance;
    mgr.view_changed_this_frame = partition_changed;
}

// ── 预算化资源管线 ──────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn process_pipeline(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut mgr: ResMut<TileManager>,
    mut mesh_pipe: ResMut<MeshPipeline>,
    tex_rx: Res<TextureReceiver>,
    mut perf: ResMut<PerfCounters>,
    // `Option` 与 `OrbitCameraPlugin` 缺席的最小 App（无头测试）兼容：
    // 没有手势资源时按稳定帧（默认预算）处理。
    gesture: Option<Res<CameraGesture>>,
) {
    // 相机手势进行中（拖动/倾斜/滑行）时抬高逐帧预算，让新级别瓦片尽快补齐；
    // 稳定帧回落为 `DefaultBudget`（黄金路径预算，见 `GestureBudget` 文档）。
    let budget = globe_pipeline::GestureBudget::for_gesture(
        gesture.is_some_and(|g| g.active),
    );
    // 每帧的预算化管线主循环：消化网格/纹理结果、生成与淘汰瓦片。
    globe_pipeline::run(
        &mut commands, &mut meshes, &mut materials, &mut images,
        &mut mgr, &mut mesh_pipe, &tex_rx, &mut perf, &budget,
    );
}

// ── 可见性同步 ──────────────────────────────────────────────────────

fn sync_visibility(mgr: Res<TileManager>, mut tiles: Query<(&GlobeTile, &mut Visibility)>) {
    for (tile, mut vis) in &mut tiles {
        // 仅当瓦片处于当前显示分区时可见，否则隐藏（保留实体以便廉价复用）。
        let in_partition = mgr.visible_set.contains(&(tile.x, tile.y, tile.z));
        let target = if in_partition { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != target { *vis = target; }
    }
}

// ── 基础球合成 ────────────────────────────────────────────────

fn base_sphere_composite_system(
    mut state: ResMut<BaseSphereComposite>,
    mut mgr: ResMut<TileManager>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    sphere: Query<&MeshMaterial3d<StandardMaterial>, With<BaseSphereMarker>>,
) {
    globe_pipeline::run_base_sphere_composite(
        &mut state, &mut mgr, &mut images, &mut materials, &sphere,
    );
}
