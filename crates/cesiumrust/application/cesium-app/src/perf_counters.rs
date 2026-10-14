//! PerfCounters —— 暴露 dynamic_globe TileManager 指标的 Bevy Resource。
//!
//! ## 设计
//!
//! `PerfCounters` 是一个**纯观测面**：dynamic_globe 每帧写入
//! 它，M0.5 perf-trace 插件从中读取，以
//! 生成 CSV 行。它**不含控制逻辑** —— 预算、
//! 淘汰、下载决策都不在此处。正是这种分离
//! 使得 M0.4 的注入保持像素中性：
//! `dynamic_globe.rs` 中的黄金路径保留其精确语义，唯一新增的工作
//! 是每帧若干次整数存储。
//!
//! ## 计数器分类
//!
//! 计划所要求的 11 个类别映射为三种形态：
//!
//! 1. **仪表值**（瞬时大小，直接从 TileManager 集合
//!    读取）：`in_flight`、`gpu_tex_order`、`tile_entities`、
//!    `spawn_queue`、`backlog`、`retry_after`。
//! 2. **每帧增量**（每帧开始重置为 0，随后由管线在干活时
//!    递增）：`frame_mesh`、
//!    `frame_tex`、`frame_spawn`、`frame_despawn`。
//! 3. **累计总量**（自进程启动单调递增）：`stale_skips`
//!    （因瓦片离开 wanted 集合而中止的下载）、
//!    `evict_total`（实际移除的 GPU 缓存条目）、`evict_deferred`
//!    （因瓦片仍在渲染而跳过的淘汰候选）。
//!
//! ## 线程安全
//!
//! 所有字段都是 Bevy `ResMut` 访问下的普通整数，因此帧线程
//! 是唯一的写入者。perf-trace 插件在同一线程（在一个 `Update` 系统
//! 中）读取它们，并通过通道把快照交给一个专用写入线程 —— 无锁、
//! 无原子操作、与渲染循环
//! 无竞争。

use bevy::prelude::*;

/// dynamic_globe 瓦片管线的每帧 + 累计计数器。
///
/// Default 全为零，即帧 0 的正确状态（此时尚未
/// 上传 / 生成 / 淘汰任何内容）。
#[derive(Resource, Debug, Clone, Default)]
pub struct PerfCounters {
    // ── 仪表值（瞬时集合大小） ────────────────────────

    /// 当前在途的下载（`TileManager::in_flight.len()`）。
    pub in_flight: u32,
    /// GPU 纹理缓存条目（`TileManager::gpu_tex_order.len()`）。
    /// 这就是 FIFO 淘汰顺序，因此它等于活跃纹理数。
    pub gpu_tex_order: u32,
    /// 已生成的瓦片实体（`TileManager::tile_entities.len()`）。
    pub tile_entities: u32,
    /// 等待网格构建 + 生成的瓦片（`TileManager::spawn_queue.len()`）。
    pub spawn_queue: u32,
    /// 等待 GPU 上传的已完成网格（`MeshPipeline::backlog.len()`）。
    pub backlog: u32,
    /// 瞬时失败后处于下载冷却的瓦片
    /// （`TileManager::retry_after.len()`）。
    pub retry_after: u32,
    /// 处于 load 集的瓦片 —— 仍在加载但未进入渲染划分
    ///（被 KICK 阻塞的子节点）。`TileManager::load_set.len()`。
    pub load_set: u32,

    // ── 每帧增量（每帧重置，随后递增） ──────────

    /// 本帧执行的网格上传（受 `MAX_MESH_UPLOADS_PER_FRAME` 上限约束）。
    pub frame_mesh: u32,
    /// 本帧应用的纹理上传（受 `MAX_TEXTURE_UPLOADS_PER_FRAME` 上限约束）。
    pub frame_tex: u32,
    /// 本帧生成的实体（受 `MAX_SPAWNS_PER_FRAME` 上限约束）。
    pub frame_spawn: u32,
    /// 本帧反生成的实体（受 `MAX_DESPAWNS_PER_FRAME` 上限约束）。
    pub frame_despawn: u32,

    // ── 累计总量（自进程启动单调递增） ──────────────

    /// 因瓦片在途中离开 wanted 集合而中止的下载
    ///（`download_worker` 中的 "stale skip" 路径）。
    pub stale_skips: u32,
    /// 实际被淘汰的 GPU 缓存条目（从 `gpu_textures` 等中移除）。
    pub evict_total: u32,
    /// 因瓦片仍在渲染而被推迟的淘汰候选
    ///（推回 `gpu_tex_order` 而非移除）。
    pub evict_deferred: u32,

    // ── 帧记账（供 trace 写入器使用） ───────────────────

    /// 单调帧索引，由 trace 系统每帧递增一次。
    /// 从 0 开始；第一行 CSV 是帧 1。
    pub frame_idx: u32,
    /// 上一帧的墙钟增量（毫秒，取自 Bevy 的
    /// `Time::delta_secs_f64`）。由 trace 系统写入，而非
    /// dynamic_globe。
    pub dt_ms: f64,
}

impl PerfCounters {
    /// 将四个每帧增量计数器重置为零。在每帧的
    /// **开始**（管线运行前）调用，使这些增量
    /// 仅反映本帧的工作。
    ///
    /// 仪表值和累计总量刻意不在此处重置：
    /// 仪表值每帧从 TileManager 重新读取，而累计
    /// 总量必须保持单调。
    #[inline]
    pub fn begin_frame(&mut self) {
        self.frame_mesh = 0;
        self.frame_tex = 0;
        self.frame_spawn = 0;
        self.frame_despawn = 0;
    }

    /// 推进帧索引并记录墙钟增量。管线运行后由 trace 系统
    /// **每帧一次**调用，
    /// 使 CSV 行中的 `frame_idx` 与产生这些
    /// 计数器的帧相匹配。
    #[inline]
    pub fn end_frame(&mut self, dt_ms: f64) {
        self.frame_idx = self.frame_idx.wrapping_add(1);
        self.dt_ms = dt_ms;
    }

    /// 一行人类可读摘要，仿照遗留的 `[stats]` printf，以便
    /// 审阅者能目测旧日志与新计数器的一致性。
    /// 供 M0.4 验收检查使用（"PerfCounters 与手工
    /// printf 校对一致"）。
    pub fn summary_line(&self) -> String {
        format!(
            "frame={} dt={:.2}ms ents={} vis_q={} backlog={} inflight={} retry={} load={} \
             f:mesh={} tex={} spawn={} despawn={} \
             cum:stale={} evict={} defer={} gpu_tex={}",
            self.frame_idx,
            self.dt_ms,
            self.tile_entities,
            self.spawn_queue,
            self.backlog,
            self.in_flight,
            self.retry_after,
            self.load_set,
            self.frame_mesh,
            self.frame_tex,
            self.frame_spawn,
            self.frame_despawn,
            self.stale_skips,
            self.evict_total,
            self.evict_deferred,
            self.gpu_tex_order,
        )
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_zero() {
        let c = PerfCounters::default();
        assert_eq!(c.in_flight, 0);
        assert_eq!(c.frame_idx, 0);
        assert_eq!(c.dt_ms, 0.0);
        assert_eq!(c.stale_skips, 0);
    }

    #[test]
    fn begin_frame_resets_only_deltas() {
        let mut c = PerfCounters {
            frame_mesh: 5,
            frame_tex: 6,
            frame_spawn: 7,
            frame_despawn: 8,
            in_flight: 10,
            tile_entities: 20,
            stale_skips: 30,
            evict_total: 40,
            ..Default::default()
        };
        c.begin_frame();
        assert_eq!(c.frame_mesh, 0);
        assert_eq!(c.frame_tex, 0);
        assert_eq!(c.frame_spawn, 0);
        assert_eq!(c.frame_despawn, 0);
        // 仪表值 + 累计值未受影响。
        assert_eq!(c.in_flight, 10);
        assert_eq!(c.tile_entities, 20);
        assert_eq!(c.stale_skips, 30);
        assert_eq!(c.evict_total, 40);
    }

    #[test]
    fn end_frame_advances_index_and_records_dt() {
        let mut c = PerfCounters::default();
        c.end_frame(16.67);
        assert_eq!(c.frame_idx, 1);
        assert!((c.dt_ms - 16.67).abs() < 1e-9);
        c.end_frame(16.70);
        assert_eq!(c.frame_idx, 2);
        assert!((c.dt_ms - 16.70).abs() < 1e-9);
    }

    #[test]
    fn summary_line_contains_all_fields() {
        let mut c = PerfCounters {
            tile_entities: 123,
            in_flight: 4,
            ..Default::default()
        };
        c.end_frame(16.0);
        let s = c.summary_line();
        assert!(s.contains("frame=1"));
        assert!(s.contains("ents=123"));
        assert!(s.contains("inflight=4"));
    }
}
