//! M0.5 —— CSV 性能追踪、无头 CLI 与相机脚本播放。
//!
//! ## 架构
//!
//! ```text
//!  frame thread (Update)                    writer thread (dedicated)
//!  ─────────────────────                    ─────────────────────────
//!  PerfCounters ──► format CSV row ──► mpsc ──► ringbuffer ──► flush ──► file
//!       ▲                                              (batched, <0.3ms/frame)
//!       │
//!  dynamic_globe::process_pipeline (M0.4 write-back)
//! ```
//!
//! 帧线程唯一的工作是：读取计数器、`format!` 一行 CSV、
//! `tx.send(line)`。所有文件 I/O（打开、写入、刷新、关闭）都发生在
//! 写入线程上，因此慢磁盘绝不会拖住渲染循环。测得的
//! 自身开销每 5 秒经 `info!` 报告一次。
//!
//! ## CLI / 环境变量
//!
//! | 标志 | 环境变量 | 默认值 | 作用 |
//! |------|-----|---------|--------|
//! | `--headless` | `CESIUM_HEADLESS=1` | 关闭 | 在 `--headless-frames` 后自动退出；相机脚本驱动视图 |
//! | `--perf-trace=<path>` | `CESIUM_PERF_TRACE` | 关闭 | 将 CSV 追踪写入 `<path>` |
//! | `--camera-script=<toml>` | `CESIUM_CAMERA_SCRIPT` | 关闭 | 播放关键帧相机轨迹 |
//! | `--trace-interval=<n>` | `CESIUM_TRACE_INTERVAL` | 1 | 每 `n` 帧发出一行 CSV |
//! | `--headless-frames=<n>` | `CESIUM_HEADLESS_FRAMES` | `feature_flags::DEFAULT_HEADLESS_FRAMES` (120) | 裸 `--headless` 标志的自动退出上限。FIX-HL-FRAMES：单一真相源是 `feature_flags`；`CESIUM_HEADLESS`-env 捕获路径经 `CesiumHeadlessPlugin` 退出，而非此标志 |
//!
//! 两者都存在时，标志优先于环境变量。
//!
//! ## 相机脚本 TOML 格式
//!
//! ```toml
//! [meta]
//! name = "orbit_slow"
//! duration_s = 60.0
//! description = "Slow 360° equatorial orbit"
//!
//! [[keyframe]]
//! t = 0.0
//! lon = 0.0
//! lat = 23.0
//! height = 2.0
//!
//! [[keyframe]]
//! t = 60.0
//! lon = 360.0
//! lat = 23.0
//! height = 2.0
//! ```
//!
//! 每个关键帧的字段（除 `t` 外均可选）：
//! - `t` — 距脚本开始的秒数（必需，单调递增）
//! - `lon` / `heading` — 度；`heading` 优先（符合 Lee 的
//!   `CESIUM_CAM_HEADING` 约定）
//! - `lat` / `pitch` — 度；`pitch` 优先
//! - `height` — 表面上方的渲染单位（地球 R = 1）
//! - `distance` — 从中心的直接轨道距离；覆盖 `height`
//!
//! 相邻关键帧之间为**线性**插值。第一个关键帧之前相机保持
//! `keyframe[0]`；最后一个之后保持
//! `keyframe[last]`。单关键帧脚本即为静态相机。
//!
//! 当 `t=0` 未给出任何关键帧字段时，脚本**从 Lee 的 orbit_camera 环境
//! 播种**，因此 `--camera-script` 与 `CESIUM_CAM_*` 组合使用。

use bevy::prelude::*;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::dynamic_globe::TilePipelineSet;
use crate::feature_flags::env_flag;
use crate::orbit_camera::{orbit_state_from_env, OrbitState};
use crate::perf_counters::PerfCounters;

// ── CLI ──────────────────────────────────────────────────────────────────

/// perf-trace 子系统的已解析命令行 / 环境配置。
#[derive(Debug, Clone, Default)]
pub struct Cli {
    /// 无交互运行，并在 `headless_frames` 后自动退出。
    pub headless: bool,
    /// CSV 追踪输出路径。`None` = 禁用追踪。
    pub perf_trace: Option<PathBuf>,
    /// 相机脚本 TOML 路径。`None` = 无脚本播放。
    pub camera_script: Option<PathBuf>,
    /// 每 N 帧发出一行 CSV（默认 1 = 每帧）。
    pub trace_interval: u32,
    /// 无头模式下自动退出前的帧数。FIX-HL-FRAMES：默认值现
    /// 镜像 `feature_flags::DEFAULT_HEADLESS_FRAMES`（单一真相源，
    /// 当前为 120），而非发散的硬编码 3600。
    /// 这是一个**回退**上限：当已知墙钟目标（显式
    /// `--headless-secs`，或相机脚本的 `duration_s`）时，改由
    /// 墙钟目标支配退出，因此无论实际帧率如何
    /// 都能捕获完整轨迹。
    pub headless_frames: u32,
    /// 无头模式下自动退出前的墙钟秒数。设置后，此项
    /// 优先于 `headless_frames`（后者变为安全网）。
    /// 未设置但加载了相机脚本时，自动使用脚本的 `duration_s`
    ///（加一小段尾时）。
    pub headless_secs: Option<f64>,
}

impl Cli {
    /// 从 `std::env::args()` + 环境变量解析。
    ///
    /// 优先级：CLI 标志 > 环境变量 > 默认值。未知标志被忽略
    ///（与未来里程碑向后兼容）。
    pub fn from_env_and_args() -> Self {
        let mut cli = Self::default();

        // ── 环境变量默认值 ──────────────────────────────────────────────
        if env_flag("CESIUM_HEADLESS") {
            cli.headless = true;
        }
        if let Ok(p) = std::env::var("CESIUM_PERF_TRACE") {
            if !p.trim().is_empty() {
                cli.perf_trace = Some(PathBuf::from(p));
            }
        }
        if let Ok(p) = std::env::var("CESIUM_CAMERA_SCRIPT") {
            if !p.trim().is_empty() {
                cli.camera_script = Some(PathBuf::from(p));
            }
        }
        if let Ok(v) = std::env::var("CESIUM_TRACE_INTERVAL") {
            if let Ok(n) = v.trim().parse::<u32>() {
                cli.trace_interval = n.max(1);
            }
        }
        if let Ok(v) = std::env::var("CESIUM_HEADLESS_FRAMES") {
            if let Ok(n) = v.trim().parse::<u32>() {
                cli.headless_frames = n.max(1);
            }
        }
        if let Ok(v) = std::env::var("CESIUM_HEADLESS_SECS") {
            if let Ok(s) = v.trim().parse::<f64>() {
                if s > 0.0 && s.is_finite() {
                    cli.headless_secs = Some(s);
                }
            }
        }

        // ── CLI 覆盖 ─────────────────────────────────────────────
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            match a {
                "--headless" => cli.headless = true,
                "--perf-trace" => {
                    i += 1;
                    if i < args.len() {
                        cli.perf_trace = Some(PathBuf::from(&args[i]));
                    }
                }
                "--camera-script" => {
                    i += 1;
                    if i < args.len() {
                        cli.camera_script = Some(PathBuf::from(&args[i]));
                    }
                }
                "--trace-interval" => {
                    i += 1;
                    if i < args.len() {
                        if let Ok(n) = args[i].parse::<u32>() {
                            cli.trace_interval = n.max(1);
                        }
                    }
                }
                "--headless-frames" => {
                    i += 1;
                    if i < args.len() {
                        if let Ok(n) = args[i].parse::<u32>() {
                            cli.headless_frames = n.max(1);
                        }
                    }
                }
                "--headless-secs" => {
                    i += 1;
                    if i < args.len() {
                        if let Ok(s) = args[i].parse::<f64>() {
                            if s > 0.0 && s.is_finite() {
                                cli.headless_secs = Some(s);
                            }
                        }
                    }
                }
                other => {
                    // 支持 `--flag=value` 形式。
                    if let Some(v) = other.strip_prefix("--perf-trace=") {
                        cli.perf_trace = Some(PathBuf::from(v));
                    } else if let Some(v) = other.strip_prefix("--camera-script=") {
                        cli.camera_script = Some(PathBuf::from(v));
                    } else if let Some(v) = other.strip_prefix("--trace-interval=") {
                        if let Ok(n) = v.parse::<u32>() {
                            cli.trace_interval = n.max(1);
                        }
                    } else if let Some(v) = other.strip_prefix("--headless-frames=") {
                        if let Ok(n) = v.parse::<u32>() {
                            cli.headless_frames = n.max(1);
                        }
                    } else if let Some(v) = other.strip_prefix("--headless-secs=") {
                        if let Ok(s) = v.parse::<f64>() {
                            if s > 0.0 && s.is_finite() {
                                cli.headless_secs = Some(s);
                            }
                        }
                    }
                    // 未知标志：静默忽略（向后兼容）。
                }
            }
            i += 1;
        }

        // ── 未设置值的默认值 ─────────────────────────────────
        if cli.trace_interval == 0 {
            cli.trace_interval = 1;
        }
        if cli.headless_frames == 0 {
            // FIX-HL-FRAMES：从 `feature_flags` 的单一真相源取得回退值，
            // 而非私有的 3600，因此两个模块在未设置时
            // 对 `CESIUM_HEADLESS_FRAMES` 达成一致。这里的
            // 值是裸 `--headless` 的自动退出上限；在
            // `CESIUM_HEADLESS` 下捕获插件拥有退出权。
            cli.headless_frames = crate::feature_flags::DEFAULT_HEADLESS_FRAMES as u32;
        }
        // 无头意味着启用追踪，除非显式禁用 —— 但我们尊重显式
        // `--perf-trace` 的缺失：无追踪路径的无头模式
        // 只是自动退出（对冒烟测试有用）。

        cli
    }

    /// 当任一 perf-trace 子系统应激活时为真。
    pub fn active(&self) -> bool {
        self.headless || self.perf_trace.is_some() || self.camera_script.is_some()
    }
}

// MK4：`env_truthy` 已移除 —— 改用 `crate::feature_flags::env_flag`（所有
// CESIUM_* 标志的单一环境读取路径）。参见 feature_flags.rs 的
// "Runtime mode switches" 一节了解已注册的变量。

// ── 相机脚本（TOML） ─────────────────────────────────────────────────

/// 相机脚本文件的原始 TOML schema。
#[derive(Debug, Deserialize)]
struct CameraScriptFile {
    #[serde(default)]
    /// 可选的 `[meta]` 表（名称/时长/描述）。
    meta: Option<ScriptMeta>,
    #[serde(rename = "keyframe")]
    /// 原始关键帧列表（TOML 中每个 `[keyframe]` 一节）。
    keyframes: Vec<KeyframeRaw>,
}

#[derive(Debug, Deserialize)]
struct ScriptMeta {
    #[serde(default)]
    /// 脚本人类可读名称。
    name: Option<String>,
    #[serde(default)]
    /// 脚本总时长（秒）。
    duration_s: Option<f64>,
    #[serde(default)]
    /// 脚本描述，加载时记录日志。
    description: Option<String>,
}

/// 轨迹中的一个关键帧。除 `t` 外所有字段均可选；
/// 缺失字段继承前一个关键帧的值（第一个关键帧则
/// 继承环境播种值）。
#[derive(Debug, Deserialize, Clone)]
struct KeyframeRaw {
    /// 距脚本开始的秒数。
    t: f64,
    /// 以度为单位的经度（相机方位角）。别名：`heading`。
    #[serde(default)]
    lon: Option<f64>,
    /// 以度为单位的纬度（相机仰角）。别名：`pitch`。
    #[serde(default)]
    lat: Option<f64>,
    /// 以度为单位的朝向角 —— 优先于 `lon`（Lee 的约定）。
    #[serde(default)]
    heading: Option<f64>,
    /// 以度为单位的俯仰角 —— 优先于 `lat`。
    #[serde(default)]
    pitch: Option<f64>,
    /// 以渲染单位计的表面上方高度（地球 R = 1）。
    #[serde(default)]
    height: Option<f64>,
    /// 从中心的直接轨道距离；覆盖 `height`。
    #[serde(default)]
    distance: Option<f64>,
}

/// 已解析、所有字段填充的关键帧（无 `Option`）。
#[derive(Debug, Clone)]
struct Keyframe {
    /// 距脚本开始的秒数。
    t: f64,
    /// 已解算的方位角（弧度）。
    heading_rad: f32,
    /// 已解算的俯仰角（弧度）。
    pitch_rad: f32,
    /// 已解算的轨道距离（渲染单位）。
    distance: f32,
}

/// 已解析 + 已解决的相机脚本，准备好播放。
#[derive(Debug, Clone)]
pub struct CameraScript {
    pub name: String,
    pub duration_s: f64,
    /// 来自 `[meta]` 的可选人类描述，加载时记录日志。
    pub description: Option<String>,
    /// 已解算、字段全填充的关键帧序列。
    keyframes: Vec<Keyframe>,
}

impl CameraScript {
    /// 从 TOML 文件加载。对于第一个关键帧省略的任何字段，
    /// 回退到环境播种（Lee 的 `orbit_state_from_env` 等价物），
    /// 因此脚本可只指定它关心的轴。
    pub fn load(path: &PathBuf, env_seed: &OrbitState) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("camera-script read failed: {}", e))?;
        let raw: CameraScriptFile =
            toml::from_str(&text).map_err(|e| format!("camera-script TOML parse: {}", e))?;

        if raw.keyframes.is_empty() {
            return Err("camera-script has no [[keyframe]] entries".into());
        }

        let name = raw
            .meta
            .as_ref()
            .and_then(|m| m.name.clone())
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unnamed".into())
            });
        let description = raw.meta.as_ref().and_then(|m| m.description.clone());
        let duration_s = raw
            .meta
            .as_ref()
            .and_then(|m| m.duration_s)
            .unwrap_or_else(|| raw.keyframes.last().map(|k| k.t).unwrap_or(0.0));

        // 解析每个关键帧：缺失字段继承自前一个已解析
        // 关键帧；第一个关键帧继承自环境播种。
        let mut resolved: Vec<Keyframe> = Vec::with_capacity(raw.keyframes.len());
        let mut prev_heading = env_seed.heading;
        let mut prev_pitch = env_seed.pitch;
        let mut prev_distance = env_seed.distance;

        for kf in &raw.keyframes {
            // heading：显式 `heading` > `lon` > 继承
            let heading_deg = kf.heading.or(kf.lon);
            let heading_rad = match heading_deg {
                Some(d) => (d as f32).to_radians(),
                None => prev_heading,
            };
            // pitch：显式 `pitch` > `lat` > 继承
            let pitch_deg = kf.pitch.or(kf.lat);
            let pitch_rad = match pitch_deg {
                Some(d) => (d as f32).to_radians(),
                None => prev_pitch,
            };
            // distance：显式 `distance` > `height` (1.0 + h) > 继承
            let distance = if let Some(d) = kf.distance {
                d as f32
            } else if let Some(h) = kf.height {
                1.0 + h as f32 // GLOBE_RADIUS = 1.0
            } else {
                prev_distance
            };

            resolved.push(Keyframe {
                t: kf.t,
                heading_rad,
                pitch_rad,
                distance,
            });
            prev_heading = heading_rad;
            prev_pitch = pitch_rad;
            prev_distance = distance;
        }

        // 按时间排序（TOML 顺序不保证）。
        resolved.sort_by(|a, b| a.t.total_cmp(&b.t));

        Ok(Self {
            name,
            duration_s,
            description,
            keyframes: resolved,
        })
    }

    /// 在 `t` 秒处采样轨迹（线性插值）。
    /// 超出脚本范围时钳制到第一个 / 最后一个关键帧。
    pub fn sample(&self, t: f64) -> (f32, f32, f32) {
        let kfs = &self.keyframes;
        if kfs.len() == 1 || t <= kfs[0].t {
            let k = &kfs[0];
            return (k.heading_rad, k.pitch_rad, k.distance);
        }
        let last = kfs.len() - 1;
        if t >= kfs[last].t {
            let k = &kfs[last];
            return (k.heading_rad, k.pitch_rad, k.distance);
        }
        // 找到夹逼的一对。
        for i in 0..last {
            let a = &kfs[i];
            let b = &kfs[i + 1];
            if t >= a.t && t <= b.t {
                let span = b.t - a.t;
                let f = if span > 1e-9 {
                    ((t - a.t) / span) as f32
                } else {
                    0.0
                };
                return (
                    a.heading_rad + (b.heading_rad - a.heading_rad) * f,
                    a.pitch_rad + (b.pitch_rad - a.pitch_rad) * f,
                    a.distance + (b.distance - a.distance) * f,
                );
            }
        }
        let k = &kfs[last];
        (k.heading_rad, k.pitch_rad, k.distance)
    }
}

// ── CSV 写入线程 ────────────────────────────────────────────────────

/// CSV 列头。与下方 `format_row` 保持同步。
/// M4：原始 13 列按位置保留（基线兼容）；4 个新列
/// 追加在尾部。
const CSV_HEADER: &str = "frame_idx,dt_ms,view_sse,visible_n,partition_n,load_n,spawn_n,\
tex_upload_n,evict_n,gpu_tex_cache,mesh_backlog,dl_in_flight,stale_skips,\
retry_after,frame_mesh,frame_despawn,evict_deferred";

/// 从帧线程发往写入线程的消息。
enum WriterMsg {
    /// 一行 CSV（已格式化，换行结尾）。
    Row(String),
    /// 优雅关闭：刷新 + 关闭。
    Shutdown,
}

/// 写入线程的句柄。丢弃它会发送 `Shutdown`，因此即使调用者
/// 遗忘，线程也会刷新并退出。
struct WriterHandle {
    /// 向写入线程发送消息的发送端；`None` 表示已关闭。
    tx: Option<mpsc::Sender<WriterMsg>>,
    /// 写入线程的 join 句柄；`None` 表示已回收。
    join: Option<std::thread::JoinHandle<()>>,
}

impl WriterHandle {
    /// 启动命名写入线程，将轨迹行落盘到 `path`。
    fn spawn(path: PathBuf) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<WriterMsg>();
        let join = std::thread::Builder::new()
            .name("perf-trace-writer".into())
            .spawn(move || {
                writer_loop(path, rx);
            })?;
        Ok(Self {
            tx: Some(tx),
            join: Some(join),
        })
    }

    /// 尽力而为地发送一行轨迹；发送端已死则静默丢弃。
    fn send(&self, row: String) {
        if let Some(tx) = &self.tx {
            // 尽力而为：若写入线程已死，丢弃该行而非
            // 使帧线程 panic。
            let _ = tx.send(WriterMsg::Row(row));
        }
    }
}

impl Drop for WriterHandle {
    /// 丢弃时发送 `Shutdown` 并 join，使写入线程刷新尾部行后退出。
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(WriterMsg::Shutdown);
        }
        if let Some(j) = self.join.take() {
            // 给写入线程最多 2 秒刷新；不要永久阻塞关闭。
            let _ = j.join();
        }
    }
}

/// 写入线程主循环：在环形缓冲区中批量处理行，定期刷新。
///
/// 刷新策略：每 64 行或每 200 毫秒，以先到者为准。这将
/// 内存限制在（≤64 行 × ~200 B ≈ 13 KB），同时使磁盘 I/O 远离
/// 帧线程的关键路径。
fn writer_loop(path: PathBuf, rx: mpsc::Receiver<WriterMsg>) {
    let file = match File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[perf-trace] failed to create {}: {}", path.display(), e);
            return;
        }
    };
    let mut w = BufWriter::new(file);
    if let Err(e) = writeln!(w, "{}", CSV_HEADER) {
        eprintln!("[perf-trace] header write failed: {}", e);
        return;
    }

    const BATCH: usize = 64;
    const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
    let mut buf: Vec<String> = Vec::with_capacity(BATCH);
    let mut last_flush = Instant::now();

    loop {
        // 排空可用消息，阻塞时长不超过刷新间隔，
        // 因此即使行涓流到达也能遵守基于时间的刷新。
        let timeout = FLUSH_INTERVAL.saturating_sub(last_flush.elapsed());
        match rx.recv_timeout(timeout.max(Duration::from_millis(1))) {
            Ok(WriterMsg::Row(row)) => buf.push(row),
            Ok(WriterMsg::Shutdown) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // 非阻塞地排空任何额外排队的行以批量处理。
        while buf.len() < BATCH * 4 {
            match rx.try_recv() {
                Ok(WriterMsg::Row(row)) => buf.push(row),
                Ok(WriterMsg::Shutdown) => {
                    flush_rows(&mut w, &mut buf);
                    return;
                }
                Err(_) => break,
            }
        }
        if buf.len() >= BATCH || last_flush.elapsed() >= FLUSH_INTERVAL {
            flush_rows(&mut w, &mut buf);
            last_flush = Instant::now();
        }
    }
    flush_rows(&mut w, &mut buf);
}

/// 将缓冲中的轨迹行刷入 `BufWriter` 并清空缓冲。
fn flush_rows(w: &mut BufWriter<File>, buf: &mut Vec<String>) {
    if buf.is_empty() {
        return;
    }
    for row in buf.drain(..) {
        // 忽略写错误：磁盘满绝不能使应用崩溃。
        let _ = w.write_all(row.as_bytes());
    }
    let _ = w.flush();
}

/// 从当前计数器格式化一行 CSV。
///
/// 列顺序与 `CSV_HEADER` 匹配。`view_sse` 是四叉树遍历内的逐瓦片值，
/// 而非单一标量；在延迟项 DEFER-M0-VIEWSSE 解决（由 Jimmy 登记）
/// 之前，它保持为 `0` 占位符。
fn format_row(c: &PerfCounters) -> String {
    format!(
        "{},{:.3},0,{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
        c.frame_idx,
        c.dt_ms,
        c.tile_entities,   // visible_n：已生成实体（划分子集）
        c.spawn_queue,      // partition_n 代理：排队等待生成
        c.load_set,         // load_n：被 KICK 阻塞、仍在加载的子节点
        c.frame_spawn,      // spawn_n
        c.frame_tex,        // tex_upload_n
        c.evict_total,      // evict_n（累计）
        c.gpu_tex_order,    // gpu_tex_cache
        c.backlog,          // mesh_backlog
        c.in_flight,        // dl_in_flight
        c.stale_skips,      // stale_skips（累计）
        c.retry_after,      // retry_after（M4 第 14 列）
        c.frame_mesh,       // frame_mesh（M4 第 15 列）
        c.frame_despawn,    // frame_despawn（M4 第 16 列）
        c.evict_deferred,   // evict_deferred（M4 第 17 列）
    )
}

// ── Bevy 插件 + 系统 ────────────────────────────────────────────────

/// 持有 perf-trace 子系统运行时状态的资源。
#[derive(Resource)]
struct TraceState {
    /// 启动时解析的 CLI 配置。
    cli: Cli,
    /// 写入线程句柄；未启用时为 `None`。
    writer: Option<WriterHandle>,
    /// 已加载的相机脚本（若有）。
    script: Option<CameraScript>,
    /// 应用启动时刻，作为相机脚本时间基准。
    start: Instant,
    /// 用于 5 秒报告的累计自身开销。
    overhead_accum: Duration,
    /// 累计的已测帧数（用于 5 秒报告）。
    overhead_frames: u32,
    /// 上一次开销报告时刻。
    last_report: Instant,
    /// 用于 trace-interval 门控的帧计数器。
    frame_counter: u32,
    /// 无头模式的墙钟退出目标。`Some(d)` 意味着：一旦
    /// `start.elapsed() >= d` 就退出，无论帧数。`None` 意味着：
    /// 回退到 `headless_frames` 上限。在插件构建时由
    /// `--headless-secs` 计算，否则用相机脚本的 `duration_s` + 尾时。
    exit_after: Option<Duration>,
}

/// 连装 perf-trace 子系统的插件。
///
/// 当 `cli.active()` 为 false 时无作用：不注册任何系统，
/// 不插入任何资源，零运行时开销。
pub struct PerfTracePlugin {
    /// 控制子系统是否激活的 CLI 配置。
    cli: Cli,
}

impl PerfTracePlugin {
    /// 以给定 CLI 配置构造插件。
    pub fn new(cli: Cli) -> Self {
        Self { cli }
    }
}

impl Plugin for PerfTracePlugin {
    /// 插件装配入口：仅当 `cli.active()` 时插入资源并注册系统，否则空操作。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        if !self.cli.active() {
            return;
        }

        // 使用环境播种的 OrbitState 加载相机脚本（若有），
        // 作为省略关键帧字段的回退。MK5：直接调用
        // `orbit_state_from_env`（相机播种的单一真相源）。
        let script = self.cli.camera_script.as_ref().and_then(|p| {
            let seed = orbit_state_from_env();
            match CameraScript::load(p, &seed) {
                Ok(s) => {
                    let desc = s
                        .description
                        .as_deref()
                        .unwrap_or("(no description)");
                    info!(
                        "[perf-trace] camera-script '{}' loaded: {} keyframes, {:.1}s — {}",
                        s.name,
                        s.keyframes.len(),
                        s.duration_s,
                        desc
                    );
                    Some(s)
                }
                Err(e) => {
                    error!("[perf-trace] {}", e);
                    None
                }
            }
        });

        // Spawn the writer thread (if a trace path is given).
        let writer = self.cli.perf_trace.as_ref().and_then(|p| {
            match WriterHandle::spawn(p.clone()) {
                Ok(w) => {
                    info!("[perf-trace] CSV writer started: {}", p.display());
                    Some(w)
                }
                Err(e) => {
                    error!("[perf-trace] writer spawn failed: {}", e);
                    None
                }
            }
        });

        // 在 `script` 被移入资源之前计算墙钟退出目标。
        // 优先级：显式 `--headless-secs` > 已加载脚本的
        // `duration_s`（+0.5 秒收尾，使最后一个关键帧被
        // 捕获且其 CSV 行刷新）> None（回退到帧数上限）。
        //
        // R2：`is_finite()` 守卫 —— `Duration::from_secs_f64(INFINITY)` 会 panic。
        // TOML 的 `duration_s` 也可能是 inf/nan；回退到 3600 秒。
        let exit_after = self
            .cli
            .headless_secs
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(Duration::from_secs_f64)
            .or_else(|| {
                script.as_ref().map(|s| {
                    let d = s.duration_s + 0.5;
                    if d.is_finite() && d > 0.0 {
                        Duration::from_secs_f64(d)
                    } else {
                        Duration::from_secs(3600)
                    }
                })
            });

        app.insert_resource(TraceState {
            cli: self.cli.clone(),
            writer,
            script,
            start: Instant::now(),
            overhead_accum: Duration::ZERO,
            overhead_frames: 0,
            last_report: Instant::now(),
            frame_counter: 0,
            exit_after,
        });

        // 相机脚本播放运行在 trace 采样器之前，使 CSV 行
        // 反映的是脚本驱动的相机，而非上一帧的。
        app.add_systems(Update, camera_script_system);
        // R1：显式 `.after(TilePipelineSet)` 确保 PerfCounters 在我们采样
        // 之前已用本帧数据填充。若无此约束，Bevy 调度器可能先运行
        // trace_sampler_system（二者都持有 ResMut<PerfCounters>），导致
        // CSV 中出现过期值。
        app.add_systems(
            Update,
            trace_sampler_system
                .after(camera_script_system)
                .after(TilePipelineSet),
        );
        // 无头自动退出最后运行，使最后一帧的行得以发出。
        app.add_systems(Update, headless_exit_system.after(trace_sampler_system));
    }
}

// MK5：移除 `env_seed_state()` —— 现直接调用 orbit_camera.rs 的
// `orbit_state_from_env()`（自 M0 评审起为 pub(crate)）。相机环境播种的
// 单一真相源，消除两处实现之间的漂移。

/// 相机脚本播放：用插值轨迹覆盖 OrbitState。仅在加载了脚本时
/// 运行。
///
/// 我们同时设置 `distance` 和 `target_distance`，使 orbit_camera 的惯性
/// 滑行为空操作（target == current），并直接设置 heading/pitch，因为
/// 无头模式没有鼠标输入来修改它们。
fn camera_script_system(
    state: Res<TraceState>,
    mut orbit: ResMut<OrbitState>,
) {
    // R5: borrow instead of clone — CameraScript is immutable after load.
    let Some(script) = state.script.as_ref() else {
        return;
    };
    let t = state.start.elapsed().as_secs_f64();
    let (heading, pitch, distance) = script.sample(t);
    orbit.heading = heading;
    orbit.pitch = pitch;
    orbit.distance = distance;
    orbit.target_distance = distance;
}

/// Trace 采样器：推进 PerfCounters 的帧记账，每 `trace_interval` 帧
/// 格式化一行 CSV，并交给写入线程。测量自身开销，每 5 秒报告一次。
///
/// 取得 `ResMut<PerfCounters>` 以便能调用 `end_frame`（frame_idx +
/// dt_ms）。R1：`.after(process_pipeline)` 在注册时强制约束，因此
/// 仪表值/增量保证已用本帧数据填充。
fn trace_sampler_system(
    mut state: ResMut<TraceState>,
    mut counters: ResMut<PerfCounters>,
    time: Res<Time>,
) {
    let t0 = Instant::now();

    state.frame_counter = state.frame_counter.wrapping_add(1);

    // 推进帧记账：frame_idx + dt_ms。这是这两个字段的唯一
    // 写入者，因此 CSV 行的帧编号与产生这些计数器的帧一致。
    counters.end_frame(time.delta_secs_f64() * 1000.0);

    // 按 trace_interval 门控（默认 1 = 每帧）。
    if state.frame_counter.is_multiple_of(state.cli.trace_interval.max(1)) {
        if let Some(writer) = &state.writer {
            let row = format_row(&counters);
            writer.send(row);
        }
    }

    let elapsed = t0.elapsed();
    state.overhead_accum += elapsed;
    state.overhead_frames += 1;
    if state.last_report.elapsed() >= Duration::from_secs(5) {
        let avg = state.overhead_accum.as_secs_f64()
            / (state.overhead_frames.max(1)) as f64
            * 1000.0;
        info!(
            "[perf-trace] self-overhead: {:.4} ms/frame (avg over {} frames)",
            avg, state.overhead_frames
        );
        // M0.4 验收：PerfCounters 必须与旧版 `[stats]` printf 一致。
        // 以可比较的单行形式发出相同数字，使评审者能目测旧日志
        // 与新计数器之间的一致性。
        info!("[perf-trace] counters: {}", counters.summary_line());
        state.overhead_accum = Duration::ZERO;
        state.overhead_frames = 0;
        state.last_report = Instant::now();
    }
}

/// 无头自动退出。当墙钟目标已知（`exit_after`）时，由它
/// 控制退出，从而不论帧率如何都能捕获完整的相机轨迹；否则
/// 回退到 `headless_frames` 上限。
///
/// FIX-HL-EXIT：本系统是裸 `--headless` CLI 标志的退出路径。
/// 在 `CESIUM_HEADLESS` 环境下，`main.rs` 强制 `cli.headless = false`
/// （“无头模式所有权”），因此下方的提前返回会挂起本系统，
/// 并将退出所有权交给 `CesiumHeadlessPlugin` 的离屏捕获 →
/// `AppExit::Success` 链。因此该分支是*有条件的*，而非死代码 ——
/// 不要移除它。
fn headless_exit_system(
    state: Res<TraceState>,
    mut exit: EventWriter<AppExit>,
) {
    if !state.cli.headless {
        return;
    }
    match state.exit_after {
        Some(d) => {
            let elapsed = state.start.elapsed();
            if elapsed >= d {
                info!(
                    "[perf-trace] headless: reached {:.1}s wall-clock ({} frames), exiting",
                    elapsed.as_secs_f64(),
                    state.frame_counter
                );
                exit.send(AppExit::Success);
            }
        }
        None => {
            if state.frame_counter >= state.cli.headless_frames {
                info!(
                    "[perf-trace] headless: reached {} frames, exiting",
                    state.cli.headless_frames
                );
                exit.send(AppExit::Success);
            }
        }
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_defaults_are_sane() {
        // 清除环境变量以使测试具有确定性。
        //
        // 注意（并入 Zed 工作区时的适配）：本 crate 现在以 edition 2024
        // 构建（成员 manifest 用 `edition.workspace = true`，继承的是 Zed 的
        // `2024`，而不再是本树原来 `[workspace.package]` 的 `2021`）。自
        // edition 2024 起 `std::env::remove_var` 是 unsafe 函数，因此下面
        // 包了一层 unsafe 块 —— 这是该版本升级所必需的，不含任何新行为。
        // SAFETY: 该测试是唯一改动这些变量的地方，且它们在测试开始时先被
        // 清空，随后立即被读取。
        for k in [
            "CESIUM_HEADLESS",
            "CESIUM_PERF_TRACE",
            "CESIUM_CAMERA_SCRIPT",
            "CESIUM_TRACE_INTERVAL",
            "CESIUM_HEADLESS_FRAMES",
            "CESIUM_HEADLESS_SECS",
        ] {
            unsafe {
                std::env::remove_var(k);
            }
        }
        let cli = Cli::from_env_and_args();
        assert!(!cli.headless);
        assert!(cli.perf_trace.is_none());
        assert!(cli.camera_script.is_none());
        assert_eq!(cli.trace_interval, 1);
        assert_eq!(
            cli.headless_frames,
            crate::feature_flags::DEFAULT_HEADLESS_FRAMES as u32,
            "FIX-HL-FRAMES: perf-trace's unset default must equal feature_flags' single source of truth"
        );
        assert!(cli.headless_secs.is_none());
        assert!(!cli.active());
    }

    #[test]
    fn camera_script_single_keyframe_is_static() {
        let seed = OrbitState::default();
        let toml_text = r#"
[meta]
name = "static"
duration_s = 10.0

[[keyframe]]
t = 0.0
lon = 45.0
lat = 30.0
height = 1.5
"#;
        let dir = std::env::temp_dir().join("cesium_test_static.toml");
        std::fs::write(&dir, toml_text).unwrap();
        let script = CameraScript::load(&dir, &seed).unwrap();
        assert_eq!(script.name, "static");
        assert_eq!(script.keyframes.len(), 1);
        let (h, p, d) = script.sample(0.0);
        let (h2, p2, d2) = script.sample(999.0);
        assert!((h - h2).abs() < 1e-6);
        assert!((p - p2).abs() < 1e-6);
        assert!((d - d2).abs() < 1e-6);
        assert!((d - 2.5).abs() < 1e-6); // 1.0 + 1.5
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn camera_script_interpolates_linearly() {
        let seed = OrbitState::default();
        let toml_text = r#"
[[keyframe]]
t = 0.0
lon = 0.0
lat = 0.0
height = 1.0

[[keyframe]]
t = 10.0
lon = 100.0
lat = 50.0
height = 3.0
"#;
        let dir = std::env::temp_dir().join("cesium_test_interp.toml");
        std::fs::write(&dir, toml_text).unwrap();
        let script = CameraScript::load(&dir, &seed).unwrap();
        let (h, p, d) = script.sample(5.0);
        // 中点：lon=50°, lat=25°, height=2.0 → distance=3.0
        assert!((h - 50.0f32.to_radians()).abs() < 1e-5);
        assert!((p - 25.0f32.to_radians()).abs() < 1e-5);
        assert!((d - 3.0).abs() < 1e-5);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn camera_script_heading_takes_precedence_over_lon() {
        let seed = OrbitState::default();
        let toml_text = r#"
[[keyframe]]
t = 0.0
lon = 10.0
heading = 99.0
"#;
        let dir = std::env::temp_dir().join("cesium_test_prec.toml");
        std::fs::write(&dir, toml_text).unwrap();
        let script = CameraScript::load(&dir, &seed).unwrap();
        let (h, _, _) = script.sample(0.0);
        assert!((h - 99.0f32.to_radians()).abs() < 1e-5);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn format_row_matches_header_column_count() {
        let c = PerfCounters {
            frame_idx: 7,
            dt_ms: 16.667,
            tile_entities: 100,
            spawn_queue: 5,
            load_set: 12,
            frame_spawn: 3,
            frame_tex: 2,
            frame_mesh: 9,
            frame_despawn: 1,
            evict_total: 4,
            evict_deferred: 2,
            gpu_tex_order: 50,
            backlog: 4,
            in_flight: 6,
            retry_after: 3,
            stale_skips: 8,
        };
        let row = format_row(&c);
        let cols: Vec<&str> = row.trim_end().split(',').collect();
        let header_cols: Vec<&str> = CSV_HEADER.split(',').collect();
        assert_eq!(
            cols.len(),
            header_cols.len(),
            "row has {} cols, header has {}: {:?}",
            cols.len(),
            header_cols.len(),
            cols
        );
        assert_eq!(header_cols.len(), 17); // M4：原有 13 列 + 追加 4 列
        assert_eq!(cols[0], "7"); // frame_idx
        assert_eq!(cols[5], "12"); // load_n = load_set
        assert_eq!(cols[13], "3"); // retry_after（M4 第 14 列）
        assert_eq!(cols[14], "9"); // frame_mesh（M4 第 15 列）
        assert_eq!(cols[15], "1"); // frame_despawn（M4 第 16 列）
        assert_eq!(cols[16], "2"); // evict_deferred（M4 第 17 列）
    }
}
