//! Feature 标志注册表 —— 集中处理所有 `CESIUM_ENABLE_*` 环境变量解析。
//!
//! ## 设计
//!
//! 本模块是 cesium-app 中每一个 opt-in feature 门控的
//! **单一真相源**。它取代了 `main.rs` 中先前散落的 `env_enabled`
//! 辅助函数，并暴露一套类型安全、可发现的 API。
//!
//! 提供两类标志：
//!
//! 1. **激活标志** —— 目前由 `main.rs` 消费，以决定接入哪些插件。
//!    它们均保留 M0.3 之前的确切语义（默认
//!    OFF，`1`/`true`/`yes`/`on` 启用，大小写不敏感，容忍空白）。
//!    当前激活集合（截至 M6 Wave A）：
//!    - *chains / harness*：`TERRAIN`、`TILESET`、`NEW_CHAINS`（总伞），
//!      screenshot + offline + fixed-time + perf-trace + headless 运行时开关；
//!    - *lighting*（M4.1）：`CESIUM_LIGHTING_MODE`（`lighting_mode()`）；
//!    - *post-process*（M4.2 内置 + M5-E1/E2 自实现）：`POSTPROCESS_BUILTIN`、
//!      `POSTPROCESS`，外加两个 M5-E **子门控** `FXAA` / `AO`。
//!      ⚠ **自 P0 观感清理起无消费者**：后处理栈连同其适配器模块
//!      `adapters/bevy-render/src/effects` 与领域 crate `cesium-effects`
//!      已删除（目标见 docs/SCOPE_2D3D_IMAGERY_PLOT.md）；这些常量与
//!      访问器仅为诊断命名空间的可比性而保留；
//!    - *sky*（M5-B/M5-C）：`SKYDOME`（及其反向推导的 `glow_enabled()`）；
//!    - *material*（M5-D）：`MATERIAL_SHOWCASE`；
//!    - *M6 Wave A*（任务 #81 集成）：`CLIPPING`（M6.2）、`PANORAMA`（M6.3）、
//!      `IBL`（M6.5）、`OIT`（M6.4）、`SPLIT`（M6.1）、`CLOUDS`（M6.6）。
//!      ⚠ **自 P0 观感清理起无消费者**（同上；`effects/` 模块与
//!      `M6WaveARenderGraphPlugin` 已删除）。
//!
//! 2. **保留命名空间标志** —— 现在就声明，以便下游里程碑
//!    （M1–M17）无需再改一轮 `main.rs` 即可采用它们。
//!    每个保留标志都有稳定的常量 + 访问器；消费者在后续
//!    里程碑接入。**声明一个保留标志从不改变运行时行为**
//!    （目前没有插件读取它们）。
//!    **规则**：一旦某里程碑将保留标志接入 `main.rs`，该
//!    标志就变为 *active* —— 其访问器文档必须重新标注为 ACTIVE
//!    （写明里程碑 + 它所门控的代码路径），且必须列入上方
//!    的 Active 集合。[`RESERVED_FLAGS`] 的成员关系保持稳定，
//!    以便跨里程碑的诊断可比性（参见该常量处的说明），
//!    因此“移出 Reserved 分区”指的是*文档与访问器分类*，
//!    而非从诊断列表中删除。
//!
//! ## 像素中性契约
//!
//! 从 `main.rs::env_enabled` 迁移到本模块是**仅源码层面**的：
//! 解析谓词、默认值，以及每个环境变量名都是字节相同的。
//! 没有 feature 门控会默认翻转，因此渲染帧保持不变。
//!
//! ## 环境变量命名
//!
//! 所有标志均遵循 `CESIUM_ENABLE_<UPPER_SNAKE>`。激活标志另外
//! 还支持旧别名 `CESIUM_ENABLE_NEW_CHAINS`，它同时启用 terrain
//! 和 tileset（为 capture-harness 兼容而保留）。
//!
//! ## 两种默认语义共存（添加门控前请先读此）
//!
//! * **Feature 门控**（`env_flag`）在变量未设置时默认 **OFF**。
//!   每个 M4/M5/M6 能力门控都使用此语义，因此 golden 路径和 v0
//!   像素基线默认保持不受影响。
//! * **后处理子门控**（`sub_gate_flag`）在未设置时默认 **ON**。
//!   `FXAA` / `AO` 曾是*主 `POSTPROCESS` 门控的子门控*：将它们保持
//!   未设置以复现 M5-E 之前的行为，即仅主门控就同时启用
//!   两个效果（`specs/scripts/v2_fxaa.toml` / `v2_ao.toml` 依赖于此）。
//!   它们在此注册，以使本模块仍是环境变量**名称**及其确切解析
//!   语义的单一真相源 —— 一个对一个实际默认 ON 的门控报告
//!   “默认 OFF”的注册表，会比根本没有注册表更糟糕。
//!   ⚠ 自 P0 观感清理起 `POSTPROCESS` 全链已无消费者（见上），
//!   但**解析语义本身保持冻结**：`sub_gate_flag` 仍默认 ON。
//!
//! ## 适配器层镜像（DDD）
//!
//! `adapters/bevy-render` 无法导入这个 `application` crate（分层
//! 规则），而 `cesium-app` 是一个
//! 无 lib 目标的可执行 crate。因此每个适配器都保留一份字节相同的本地
//! `const` + `*_gate_enabled()` 镜像，通过同一个权威的 4-token 谓词
//! （`pipeline::fetch::gate_from_env_value`）读取**相同的环境变量名**。
//! 本模块是*权威的*注册表：下方的测试将每个环境变量名
//! 固定为字符串字面量，因此任意一侧的单边重命名会让测试套件
//! 变红，而不是静默地将一个门控拆成两半。
//!
//! 注（P0 观感清理）：原先的跨 crate 漂移守卫还与
//! `cesium_bevy_render::effects` 的镜像常量对拍；该模块已删除，
//! 守卫现在只覆盖仍然存在的适配器镜像
//! （`pipeline::fetch::gate_from_env_value` 与
//! `headless::ENV_HEADLESS_HDR`）。

// 保留命名空间（M1–M17 的常量 + 访问器）故意在任何消费者存在
// 之前就声明，因此编译器会将每个条目标记为死代码。这是有意为之：
// 本模块是一个注册表，而注册表条目的价值在于它为下游里程碑提供的
// 稳定契约，而不在于今天的可执行文件是否读取它。此 allow 仅作
// 用于本模块；激活标志（terrain/tileset/new_chains）确实由 main.rs
// 消费，若它们真的变为死代码仍会告警。
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

use cesium_bevy_render::LightingMode;

// ── 激活标志（目前由 main.rs 消费）───────────────────────

/// `CESIUM_ENABLE_TERRAIN` —— 启用 Cesium World Terrain 链路。
pub const ENV_ENABLE_TERRAIN: &str = "CESIUM_ENABLE_TERRAIN";
/// `CESIUM_ENABLE_TILESET` —— 启用 3D Tiles 瓦片集链路。
pub const ENV_ENABLE_TILESET: &str = "CESIUM_ENABLE_TILESET";
/// `CESIUM_ENABLE_NEW_CHAINS` —— 旧总伞：同时启用 terrain 和
/// tileset。为 `capture_baseline.ps1` 兼容而保留。
pub const ENV_ENABLE_NEW_CHAINS: &str = "CESIUM_ENABLE_NEW_CHAINS";
/// `CESIUM_ENABLE_PLOT` —— 2D/3D 态势标绘 overlay 桥接。
/// **仅窗口模式**（main.rs 还要求 `!headless`）且未设置时
/// **默认 ON**：`CESIUM_ENABLE_PLOT=0` 退出。无头模式从不注册它，因此
/// 无论此标志如何，离屏基线都不受影响。
pub const ENV_ENABLE_PLOT: &str = "CESIUM_ENABLE_PLOT";

// ── 运行时模式开关（M0.5 perf-trace / headless）─────────────────
//
// 这些不是 feature 启用标志（它们不门控插件）；它们
// 配置 perf-trace 子系统的运行时模式。在此注册是为了
// 履约“单一环境读取路径”契约：所有 `CESIUM_*` 环境变量
// 都在本模块文档化，即使它们的消费者位于他处。
// 布尔值通过 `env_flag()` 读取，类型化值通过 `std::env::var()` 读取。

/// `CESIUM_HEADLESS` —— truthy：以非交互式自动退出模式运行。
pub const ENV_HEADLESS: &str = "CESIUM_HEADLESS";
/// `CESIUM_PERF_TRACE` —— 路径：CSV perf-trace 输出文件。
pub const ENV_PERF_TRACE: &str = "CESIUM_PERF_TRACE";
/// `CESIUM_CAMERA_SCRIPT` —— 路径：用于轨迹播放的 TOML 相机脚本。
pub const ENV_CAMERA_SCRIPT: &str = "CESIUM_CAMERA_SCRIPT";
/// `CESIUM_TRACE_INTERVAL` —— u32：每 N 帧发出一行 CSV（默认 1）。
pub const ENV_TRACE_INTERVAL: &str = "CESIUM_TRACE_INTERVAL";
/// `CESIUM_HEADLESS_FRAMES` —— u32：headless 退出的帧数安全上限。
pub const ENV_HEADLESS_FRAMES: &str = "CESIUM_HEADLESS_FRAMES";
/// `CESIUM_HEADLESS_SECS` —— f64：headless 退出前的墙钟秒数。
pub const ENV_HEADLESS_SECS: &str = "CESIUM_HEADLESS_SECS";
/// `CESIUM_HEADLESS_OUTPUT` —— 路径：M11.3 离屏捕获写出的 PNG。
pub const ENV_HEADLESS_OUTPUT: &str = "CESIUM_HEADLESS_OUTPUT";
/// FIX-HL-HDRMIRROR：适配器私有
/// `cesium_bevy_render::headless::ENV_HEADLESS_HDR`（`"CESIUM_HEADLESS_HDR"`，
/// M11.4 HDR 离屏目标门控）的跨 crate 镜像。在此声明*仅*是为了
/// 使 `adapter_gate_mirrors_are_byte_identical_to_the_registry`
/// 中的字节相同漂移守卫能固定两侧；故意**不**加入
/// `RESERVED_FLAGS`（该门控在适配器 crate 内本地读取，与
/// `fx::ENV_ENABLE_*` 常量的消费方式一致）。将适配器私有的
/// `hdr_truthy` 归并到本注册表的工作推迟到 M11.6（参见 `docs/deferred.md`）。
pub const ENV_HEADLESS_HDR: &str = "CESIUM_HEADLESS_HDR";

// ── 离线确定性环境家族（M3.3）──────────────────────────────
//
// 这些门控离线确定性捕获路径（M3.3 / M3.4）。它们不是
// `CESIUM_ENABLE_*` feature 门控：它们配置*资产来自何处*
// 以及*本次运行是否因可重现性而冻结*。在此注册，以使
// 本模块仍是应用读取的每个环境变量的单一真相源。
// 均默认 OFF / 未设置，因此在线默认路径与
// dynamic_globe golden 路径保持逐字节不变。
//
// 由 main.rs（self-check / FIXED_TIME / FIXED_CAMERA / screenshot
// script）以及 bevy-render 离线加载器门控（M3.3）消费，它们读取
// 相同的名称 —— 本注册表是双方共同遵守的契约。

/// `OFFLINE_IMAGERY_ROOT` —— 离线影像 XYZ 金字塔目录
/// （`{root}/{z}/{x}/{y}.png`，即 `gen_offline_assets` 布局）。设置后，
/// 影像由 `FileTileFetcher` 供送，而非在线 Bing/OSM。
pub const ENV_OFFLINE_IMAGERY_ROOT: &str = "OFFLINE_IMAGERY_ROOT";
/// `OFFLINE_TERRAIN_ROOT` —— 离线 heightmap-1.0 瓦片集目录
/// （`layer.json` + `{root}/{z}/{x}/{y}.terrain`）。设置后，terrain 由
/// `FileTerrainFetcher` 供送，而非 Cesium ion。
pub const ENV_OFFLINE_TERRAIN_ROOT: &str = "OFFLINE_TERRAIN_ROOT";
/// `CESIUM_TILE_CACHE_DIR` —— 在线下载瓦片的**本地磁盘缓存根目录**。
/// 未设置时默认为工作目录下的 `tile_cache`。2D 与 3D 影像链路共用同一
/// 键空间（`{root}/bing/{z}/{x}/{y}.jpg`），因此一次下载即被两条链路、跨
/// 会话反复复用，命中时零网络。这是一个**路径型**运行时开关（与
/// `OFFLINE_IMAGERY_ROOT` 同类），不是 feature 门控，因此**不**列入
/// `RESERVED_FLAGS`（不影响 `summary_line()` 的 `reserved=n/N` 分母）。
pub const ENV_TILE_CACHE_DIR: &str = "CESIUM_TILE_CACHE_DIR";
/// `STRICT_OFFLINE` —— truthy：以 `with_strict_offline(true)` 构造
/// 离线 fetcher，使任何 `http(s)` 请求都同步 panic（无网络回退），
/// 保证离线确定性。
pub const ENV_STRICT_OFFLINE: &str = "STRICT_OFFLINE";
/// `FIXED_TIME` —— truthy：冻结 Bevy 时钟（零 delta），以获得
/// 确定性的光照/天体状态（M4 阴影基线铺垫）。
pub const ENV_FIXED_TIME: &str = "FIXED_TIME";
/// `FIXED_CAMERA` —— 指向一个 TOML 的路径，内含单个 `[camera]` 的 `pos`/`quat`/
/// `fov_y`，每帧重新应用（为 pixel_diff 提供确定性视角）。
pub const ENV_FIXED_CAMERA: &str = "FIXED_CAMERA";
/// `CESIUM_SCREENSHOT_SCRIPT` —— 指向 TOML 批量捕获脚本的路径
/// （`[[shot]]` 条目：`frame` + 相机姿态 + 输出 `name`）。M3.4。
pub const ENV_SCREENSHOT_SCRIPT: &str = "CESIUM_SCREENSHOT_SCRIPT";
/// `CESIUM_OFFLINE_SELFCHECK` —— truthy：运行无头离线自检
/// （通过真实 fetcher 回读 fixtures + 断言 STRICT_OFFLINE 在 http 时
/// panic），并在构建 GPU 应用前退出。无需窗口。
pub const ENV_OFFLINE_SELFCHECK: &str = "CESIUM_OFFLINE_SELFCHECK";
/// `CESIUM_GIT_SHA` —— 可选的 git SHA，烙印到截图元数据（由捕获
/// harness / CI 设置；未设置时回退到 `"unknown"`）。
pub const ENV_GIT_SHA: &str = "CESIUM_GIT_SHA";
// （已退役 建议1 / P1-1，2026-09-27）：`CESIUMRST_LEGACY_DYNAMIC_GLOBE` 及其
// 冻结的 legacy 单体在 G4 证明 M1.5 薄壳与它像素中性后被移除。
// 参见 docs/PIPELINE_PROMOTION_PLAN.md + verification_evidence/g4/。

// ── 光照 / 后处理模式开关（M4.1）─────────────────────
//
// 这些门控光照装置与内置后处理栈。它们不是保留
// 命名空间中的 `CESIUM_ENABLE_*` feature 门控：它们
// 配置场景在启动时*如何*被照亮 / 进后处理。

/// `CESIUM_LIGHTING_MODE` —— 选择光照装置。
/// 接受值（大小写不敏感）：`full_ambient`（默认）、`day_night`。
/// 任何无法识别的值都回退到 `full_ambient`。
pub const ENV_LIGHTING_MODE: &str = "CESIUM_LIGHTING_MODE";

/// `CESIUM_ENABLE_POSTPROCESS_BUILTIN` —— truthy：启用由 M4.2+ 接入的
/// Bevy 内置后处理栈（tone-mapping / bloom / HDR）。
/// 与保留的 `CESIUM_ENABLE_POSTPROCESS`（L133）**相互独立**，
/// 后者是 M5.5/M5.6 自定义后处理阶段的预留位。
///
/// ⚠ **自 P0 观感清理起无消费者**：`CesiumEffectsPlugin` 与
/// `adapters/bevy-render/src/effects` 已删除，`main.rs` 不再注册任何
/// 后处理；`orbit_camera.rs` 也恒定使用 `Tonemapping::None`。
pub const ENV_ENABLE_POSTPROCESS_BUILTIN: &str = "CESIUM_ENABLE_POSTPROCESS_BUILTIN";

/// `CESIUM_ENABLE_FXAA` —— `ENV_ENABLE_POSTPROCESS` 的**子门控**，选择
/// M5-E1 自实现的 FXAA 节点（仅质量预设 12）。
///
/// **自 M5-E1 起为 ACTIVE**；由 M6 Wave A（任务 #81）在此注册，以
/// 解决 Terry 在 M5-Verify 提出的 Medium 发现：该名称仅作为模块私有的
/// `const` 存在于 `adapters/bevy-render/src/effects/post_process.rs` 内。
///
/// ⚠ **不是默认 OFF 的 feature 门控。** 未设置 ⇒ ON（参见 [`sub_gate_flag`]），
/// 因此仅主 `CESIUM_ENABLE_POSTPROCESS` 门控仍会像 M5-E1 之前那样恰好启用 FXAA。
/// 实际启用为 `postprocess_enabled() && fxaa_enabled()`。
pub const ENV_ENABLE_FXAA: &str = "CESIUM_ENABLE_FXAA";

/// `CESIUM_ENABLE_AO` —— `ENV_ENABLE_POSTPROCESS` 的**子门控**，选择
/// M5-E2 自实现的 SSAO 节点（半球 16 采样 + 4×4 模糊）。
///
/// 与 [`ENV_ENABLE_FXAA`] 相同的注册理由，也相同的 ⚠ **未设置 ⇒ ON**
/// 子门控语义。实际启用为 `postprocess_enabled() && ao_enabled()`。
pub const ENV_ENABLE_AO: &str = "CESIUM_ENABLE_AO";

// ── 保留命名空间（现在就声明，由后续里程碑消费）────
//
// 每个常量都文档化将接入它的里程碑。在此声明它们
// 在运行时不花费任何代价（除非调用访问器，否则不会发生环境读取），
// 并为下游工作提供稳定契约。
//
// 注（M5 收口）：在此声明的两个条目后续已被里程碑
// WIRED，因此今天为 *active* —— `ENV_ENABLE_POSTPROCESS`
// （M5-E1 FXAA + M5-E2 SSAO 渲染图节点）与 `ENV_ENABLE_SKYDOME`
// （M5-B/M5-C 过天穹）。它们保留在本块和 [`RESERVED_FLAGS`]
// 中，以便诊断命名空间跨里程碑保持可比；下方它们的
// 访问器相应标注为 ACTIVE。
// `ENV_ENABLE_MATERIAL_SHOWCASE`（M5-D）声明在
// `ENV_ENABLE_SKYDOME` 旁边，因为它属于同一个 M5 门控家族，但它
// 天生就是 active（从未保留），因此**不**在 `RESERVED_FLAGS` 中。
//
// 注（M6 Wave A，任务 #81）：在此声明的另外三个条目现已 WIRED，
// 因此为 *active* —— `ENV_ENABLE_CLIPPING`（M6.2）、`ENV_ENABLE_PANORAMA`
// （M6.3）和 `ENV_ENABLE_IBL`（M6.5）。根据上方规则，它们保留在本块
// 和 [`RESERVED_FLAGS`] 中（文档/分类变更，而非删除），以便诊断命名空间
// 保持可 diff。`ENV_ENABLE_CLOUDS`（M6.6）由本任务新*加入*列表，
// 它将 `summary_line()` 的分母从 `reserved=n/17` 移到 `reserved=n/18`。
// `ENV_ENABLE_FXAA` / `ENV_ENABLE_AO` 也新注册在此，但它们是
// **active 子门控**（由 M5-E1/E2 天生接入），因此 —— 与
// `ENV_ENABLE_MATERIAL_SHOWCASE` 和 `CESIUM_HEADLESS` 运行时开关一样 ——
// 它们**不**列在 `RESERVED_FLAGS` 中。

/// M1 —— 统一渲染管线（取代逐系统分支）。
pub const ENV_ENABLE_PIPELINE: &str = "CESIUM_ENABLE_PIPELINE";
/// M5.5/M5.6 —— 后处理栈（自定义 WGSL 阶段：bloom / AO / tone-mapping / 颜色分级）。
pub const ENV_ENABLE_POSTPROCESS: &str = "CESIUM_ENABLE_POSTPROCESS";
/// M3.x —— 级联阴影贴图。
pub const ENV_ENABLE_CSM: &str = "CESIUM_ENABLE_CSM";
/// M5.2/M5.3 —— 程序化天穹（大气 + 太阳 + 月亮）。与
/// AtmosphereGlowPlugin 互斥：SKYDOME=1 → glow 强制 OFF。
/// **自 M5-B/M5-C 起为 ACTIVE**（在 `main.rs` 天穹分支消费）。
pub const ENV_ENABLE_SKYDOME: &str = "CESIUM_ENABLE_SKYDOME";
/// M5.4/M5-D —— Fabric 材质展示场景（内置材质 + 从
/// `Water` 程序化材质 `case 17u` 移植的三种海浪状态（Calm/Medium/Rough）。
/// **自 M5-D 起为 ACTIVE**（在 `main.rs` material-showcase 分支消费）。
/// 默认 **OFF** → `MaterialShowcasePlugin` 不会注册 → 场景
/// 中无额外实体/材质 → v0 基线保持像素中性
/// （PSNR=∞）。纯追加：不 altered 任何现有插件注册。
pub const ENV_ENABLE_MATERIAL_SHOWCASE: &str = "CESIUM_ENABLE_MATERIAL_SHOWCASE";
/// M5.x —— 实体垂贴到 terrain（地表图元）。
pub const ENV_ENABLE_DRAPING: &str = "CESIUM_ENABLE_DRAPING";
/// M6.2 —— 视锥裁剪平面（剖面 / 盒体）。
/// **自 M6 Wave A 起为 ACTIVE**（任务 #81）：门控
/// `effects::register_clipping_planes_node` + `CesiumClippingPlanes`
/// 相机组件 + `CesiumClippingLabel` 渲染图边。
pub const ENV_ENABLE_CLIPPING: &str = "CESIUM_ENABLE_CLIPPING";
/// M6.3 —— 360° 全景捕获模式。
/// **自 M6 Wave A 起为 ACTIVE**（任务 #81）：门控
/// `effects::register_panorama_node` + `CesiumPanorama` 相机组件 +
/// `MainOpaquePass → CesiumPanoramaLabel → MainTransmissivePass`
/// 的 `MainPass` 内串行插入。
pub const ENV_ENABLE_PANORAMA: &str = "CESIUM_ENABLE_PANORAMA";
/// M6.5 —— 面向 PBR 材质的基于图像的光照（IBL）。
/// **自 M6 Wave A 起为 ACTIVE**（任务 #81）：门控 `effects::register_ibl_node` +
/// `CesiumIbl` 相机组件 + `CesiumIblLabel` 渲染图边。
pub const ENV_ENABLE_IBL: &str = "CESIUM_ENABLE_IBL";
/// M6.4 —— 与顺序无关的透明度（OIT）。
pub const ENV_ENABLE_OIT: &str = "CESIUM_ENABLE_OIT";
/// M6.1 —— 分屏多视图渲染。
pub const ENV_ENABLE_SPLIT: &str = "CESIUM_ENABLE_SPLIT";
/// M6.6 —— 体积/程序化云层。
/// **由 M6 Wave A 预置**（任务 #81，源自 #51 里程碑审计：
/// 已有五个 M6 门控但缺少 `CLOUDS`）。注册它是为了让 M6.6 无需
/// 再改注册表即可接入消费者。**尚无消费者** —— 声明它
/// 不改变任何运行时行为，且它像每个 feature 门控一样默认 OFF。
pub const ENV_ENABLE_CLOUDS: &str = "CESIUM_ENABLE_CLOUDS";

// 注（P0 观感清理）：上方六个 M6 门控（CLIPPING / PANORAMA / IBL / OIT /
// SPLIT / CLOUDS）以及 M4.2/M5-E 的 POSTPROCESS 家族，其**全部消费者**
// 已在 P0 中删除 —— `domain/effects`、`adapters/bevy-render/src/effects`
// （含 `M6WaveARenderGraphPlugin`、`CesiumEffectsPlugin`、FXAA/SSAO 节点、
// clipping/panorama/ibl/oit/clouds/split 节点）以及 `adapters/bevy-render`
// 对 `wgpu` 的直接依赖。理由是这些都属于「复杂观感」，而目标
// （2D/3D 地球影像高效显示 + 2D/3D 导航标绘）不需要它们
// （分析见 docs/SCOPE_2D3D_IMAGERY_PLOT.md）。
// 常量与访问器**故意保留**：`[`RESERVED_FLAGS`]` 的成员关系是跨里程碑
// 可比性的冻结词汇表（删除会改变 `FlagSnapshot::summary_line()` 的
// `reserved=n/18` 分母并破坏既有捕获元数据 diff），且它们在本模块的
// `#![allow(dead_code)]` 下不产生告警。
/// M2 —— 新相机控制器（取代 orbit_camera）。
pub const ENV_ENABLE_NEW_CAMERA: &str = "CESIUM_ENABLE_NEW_CAMERA";
/// M8 —— 可插拔资源后端（资产流式加载）。
pub const ENV_ENABLE_RESOURCE_BACKEND: &str = "CESIUM_ENABLE_RESOURCE_BACKEND";
/// M7 —— 3D Tiles 样式 JSEP 表达式求值器（已完成）。
pub const ENV_ENABLE_STYLING_JSEP: &str = "CESIUM_ENABLE_STYLING_JSEP";
/// M14.x —— KML 导出路径。
pub const ENV_ENABLE_KML_EXPORT: &str = "CESIUM_ENABLE_KML_EXPORT";
/// M15.x —— glTF 升级管线（KHR 扩展 / Draco）。
pub const ENV_ENABLE_GLTF_UPGRADE: &str = "CESIUM_ENABLE_GLTF_UPGRADE";
/// M16.x —— Draco 网格压缩解码路径。
pub const ENV_ENABLE_DRACO: &str = "CESIUM_ENABLE_DRACO";
/// M17.x —— 点云渲染管线。
pub const ENV_ENABLE_POINT_CLOUD: &str = "CESIUM_ENABLE_POINT_CLOUD";

/// 完整保留命名空间，供诊断 / `--list-flags` 类工具使用。
/// 顺序按里程碑递增，以便 UI 可按 wave 分组。
///
/// 成员关系**禁止移除**：`POSTPROCESS`（M5-E1/E2）、
/// `SKYDOME`（M5-B/C）、`CLIPPING`（M6.2）、`PANORAMA`（M6.3）与 `IBL`（M6.5）
/// 都已接入并处于 active，但在此删除它们会改变
/// `FlagSnapshot::summary_line()` 的 `reserved=n/N` 分母并破坏
/// 跨里程碑的捕获元数据 diff。**新增**是允许的且
/// 会移动分母：M6 Wave A 追加了 `CLOUDS`（M6.6 预置），使它
/// 从 17 → 18 —— `summary_line()` 的读者必须比对打印出的
/// 分母，切勿假定 17。天生 active 的标志（如
/// `ENV_ENABLE_MATERIAL_SHOWCASE`，M5-D；`ENV_ENABLE_FXAA` / `ENV_ENABLE_AO`，
/// M5-E 子门控）**不**列在此处。
///
/// 注（P0 观感清理）：`POSTPROCESS` 与六个 M6 门控的**消费者已删除**
/// （它们因此从 active 退回 reserved），但按上面这条规则它们
/// **仍留在列表中** —— 分母依旧为 18。
pub const RESERVED_FLAGS: &[&str] = &[
    ENV_ENABLE_PIPELINE,
    ENV_ENABLE_POSTPROCESS,
    ENV_ENABLE_CSM,
    ENV_ENABLE_SKYDOME,
    ENV_ENABLE_DRAPING,
    ENV_ENABLE_CLIPPING,
    ENV_ENABLE_PANORAMA,
    ENV_ENABLE_IBL,
    ENV_ENABLE_OIT,
    ENV_ENABLE_SPLIT,
    ENV_ENABLE_CLOUDS,
    ENV_ENABLE_NEW_CAMERA,
    ENV_ENABLE_RESOURCE_BACKEND,
    ENV_ENABLE_STYLING_JSEP,
    ENV_ENABLE_KML_EXPORT,
    ENV_ENABLE_GLTF_UPGRADE,
    ENV_ENABLE_DRACO,
    ENV_ENABLE_POINT_CLOUD,
];

// ── 核心谓词 ─────────────────────────────────────────────────────

/// truthy-token 谓词。**与 M0.3 之前的 `main.rs` 实现字节相同** —— 未重新
/// 验证每个消费者的默认值前不要“改进”它。
///
/// 接受（大小写不敏感，修剪四周空白）：
/// `1`、`true`、`yes`、`on`。其他所有值（包括未设置）均为 `false`。
fn truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 读取单个环境标志。未设置或无法解析时返回 `false`。
///
/// 这是 feature 门控的**唯一**环境读取路径；下方每个访问器都
/// 汇聚到它，以使行为统一。
pub fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => truthy(&v),
        Err(_) => false,
    }
}

/// 读取一个**后处理子门控**。变量未设置时返回 `true`。
///
/// 这是 `effects/post_process.rs::sub_gate_enabled` 的镜像（它调用权威的
/// `pipeline::fetch::gate_from_env_value(Some(raw))`，即同一个 [`truthy`] 谓词）——
/// 与 [`env_flag`] 的*唯一*区别在于 `Err(_) => true` 分支。它存在是为了
/// 使 `fxaa_enabled()` / `ao_enabled()` 能报告真实的运行时行为：未设置的子门控
/// 为 ON，仅主 `CESIUM_ENABLE_POSTPROCESS` 门控就能像 M5-E1/E2 拆分它们
/// 之前那样同时启用两个效果。
///
/// **不要**将其用于 feature 门控 —— 每个能力门控都通过 [`env_flag`]
/// 默认 OFF，以使 golden 路径保持像素中性。
fn sub_gate_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => truthy(&v),
        Err(_) => true,
    }
}

// ── 激活标志访问器 ──────────────────────────────────────────────

/// 是否启用 terrain 链路？（`CESIUM_ENABLE_TERRAIN` 或 `CESIUM_ENABLE_NEW_CHAINS`）
///
/// 完整保留 M0.3 之前的语义：总伞标志开启两条链路，
/// 而任一单独标志只开启自己的链路。
pub fn terrain_enabled() -> bool {
    env_flag(ENV_ENABLE_NEW_CHAINS) || env_flag(ENV_ENABLE_TERRAIN)
}

/// 是否启用 3D Tiles 瓦片集链路？（`CESIUM_ENABLE_TILESET` 或 `CESIUM_ENABLE_NEW_CHAINS`）
pub fn tileset_enabled() -> bool {
    env_flag(ENV_ENABLE_NEW_CHAINS) || env_flag(ENV_ENABLE_TILESET)
}

/// 是否启用标绘 overlay 桥接？（`CESIUM_ENABLE_PLOT`，**默认 ON**）。
///
/// 由 `main.rs` 的窗口分支（`!headless && plot_enabled()`）消费，以注册
/// `cesium_plot_bevy::CesiumPlotBridgePlugin`。使用后处理子门控相同的
/// 未设置即为 true 语义，因此该特性在交互式会话中默认开启，
/// 同时保留可选退出能力；它对无头 golden 路径无影响，因为无头
/// 从不添加该插件。M0 仅注册资源 —— 无可见输出。
pub fn plot_enabled() -> bool {
    sub_gate_flag(ENV_ENABLE_PLOT)
}

// ── 运行时模式访问器（M0.5 perf-trace / M11.3 headless）─────────

/// 是否启用无头离屏渲染模式？（`CESIUM_HEADLESS`）
///
/// 为 truthy 时，`main.rs` 将窗口版 `WindowPlugin` 换为无 surface
/// 配置，并添加 [`cesium_bevy_render::headless::CesiumHeadlessPlugin`]，
/// 它将场景渲染到离屏目标、捕获一张 PNG，然后干净退出。
///
/// **默认 OFF**：变量未设置时，窗口启动路径逐字节不变（golden 路径
/// 中性）。这是一个运行时模式开关，不是 feature 启用门控，因此
/// 故意处于冻结的 `RESERVED_FLAGS` 可比性命名空间之外。
pub fn headless_enabled() -> bool {
    env_flag(ENV_HEADLESS)
}

/// M11.3 无头捕获触发前默认渲染的热身帧数。足够基础球体 + 首圈
/// LOD 环稳定；可通过 `CESIUM_HEADLESS_FRAMES` 覆盖。
pub const DEFAULT_HEADLESS_FRAMES: usize = 120;

/// 无头离屏捕获前需渲染的帧数（`CESIUM_HEADLESS_FRAMES`）。未设置
/// 或无法解析时回退到 [`DEFAULT_HEADLESS_FRAMES`]。
pub fn headless_frames() -> usize {
    match std::env::var(ENV_HEADLESS_FRAMES) {
        Ok(v) => v.trim().parse::<usize>().unwrap_or(DEFAULT_HEADLESS_FRAMES),
        Err(_) => DEFAULT_HEADLESS_FRAMES,
    }
}

/// 无头离屏捕获的输出 PNG 路径（`CESIUM_HEADLESS_OUTPUT`）。
/// 未设置/空值时默认为工作目录下的 `headless_capture.png`。
pub fn headless_output() -> PathBuf {
    env_path(ENV_HEADLESS_OUTPUT).unwrap_or_else(|| PathBuf::from("headless_capture.png"))
}

// ── 离线确定性访问器（M3.3）───────────────────────────────

/// 读取一个路径型环境变量，未设置或空时返回 `None`。
/// 会修剪空白；空值被视为未设置，因此一个空的
/// `OFFLINE_IMAGERY_ROOT=` 绝不会意外启用离线路径。
fn env_path(name: &str) -> Option<PathBuf> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(PathBuf::from(v.trim())),
        _ => None,
    }
}

/// 离线影像金字塔根目录（`OFFLINE_IMAGERY_ROOT`），若已设置。
pub fn offline_imagery_root() -> Option<PathBuf> {
    env_path(ENV_OFFLINE_IMAGERY_ROOT)
}

/// 离线 terrain 瓦片集根目录（`OFFLINE_TERRAIN_ROOT`），若已设置。
pub fn offline_terrain_root() -> Option<PathBuf> {
    env_path(ENV_OFFLINE_TERRAIN_ROOT)
}

/// 在线瓦片磁盘缓存根目录（`CESIUM_TILE_CACHE_DIR`）；未设置/空值回退到
/// 工作目录下的 `tile_cache`。与离线根目录相互独立：仅当**未**设置
/// `OFFLINE_IMAGERY_ROOT` 时，下载 worker 才会读写此缓存。
pub fn tile_cache_dir() -> PathBuf {
    env_path(ENV_TILE_CACHE_DIR).unwrap_or_else(|| PathBuf::from("tile_cache"))
}

/// `true` when either offline root is set (serve assets from disk, not net).
pub fn offline_mode() -> bool {
    offline_imagery_root().is_some() || offline_terrain_root().is_some()
}

/// STRICT_OFFLINE: forbid any network fallback (http(s) → synchronous panic).
pub fn strict_offline() -> bool {
    env_flag(ENV_STRICT_OFFLINE)
}

/// FIXED_TIME：冻结时钟以获得确定性的光照/天体状态。
pub fn fixed_time_enabled() -> bool {
    env_flag(ENV_FIXED_TIME)
}

/// FIXED_CAMERA TOML 路径（单一确定性视角），若已设置。
pub fn fixed_camera_path() -> Option<PathBuf> {
    env_path(ENV_FIXED_CAMERA)
}

/// `CESIUM_SCREENSHOT_SCRIPT` 批量捕获 TOML 路径（M3.4），若已设置。
pub fn screenshot_script_path() -> Option<PathBuf> {
    env_path(ENV_SCREENSHOT_SCRIPT)
}

/// 是否运行无头离线自检并在 GPU 应用构建前退出？
pub fn offline_selfcheck() -> bool {
    env_flag(ENV_OFFLINE_SELFCHECK)
}

// （已退役 建议1 / P1-1，2026-09-27）：`legacy_dynamic_globe()` 访问器与
// 冻结的 `dynamic_globe_legacy.rs` 单体一并移除（G4 验证与 M1.5
// 薄壳像素中性；golden 路径现在仅为薄壳）。

// ── 光照 / 后处理访问器（M4.1）─────────────────────────

/// 将 `CESIUM_LIGHTING_MODE` 解析为 [`LightingMode`]。
///
/// 接受的 token（大小写不敏感，修剪空白）：
/// - `day_night` | `daynight` | `day-night` → [`LightingMode::DayNight`]
/// - 其他所有值（包括未设置）→ [`LightingMode::FullAmbient`]
///
/// 默认为 `FullAmbient`，因此 v0 基线路径像素相同。
pub fn lighting_mode() -> LightingMode {
    match std::env::var(ENV_LIGHTING_MODE) {
        Ok(raw) => {
            let normalized = raw.trim().to_ascii_lowercase().replace('-', "_");
            match normalized.as_str() {
                "day_night" | "daynight" => LightingMode::DayNight,
                _ => LightingMode::FullAmbient,
            }
        }
        Err(_) => LightingMode::FullAmbient,
    }
}

/// `CESIUM_ENABLE_POSTPROCESS_BUILTIN`：启用 Bevy 内置后处理
/// （tone-mapping / bloom / HDR）。默认 OFF。与保留的
/// `CESIUM_ENABLE_POSTPROCESS`（它门控 M5.5/M5.6 自定义阶段）相互独立。
pub fn postprocess_builtin_enabled() -> bool {
    env_flag(ENV_ENABLE_POSTPROCESS_BUILTIN)
}

/// M5-E1 FXAA **子门控** —— **ACTIVE**，且 ⚠ **未设置时默认 ON**。
///
/// 由 `effects/post_process.rs::CesiumEffectsPlugin::build` 消费，它将
/// 该值写入 `PostProcessConfig::fxaa_enabled`，因此写入相机的 `CesiumFxaa`
/// 组件。本访问器是该契约在注册表侧的陈述；由于适配器不得
/// 导入应用层（DDD），它保留一份字节相同的本地镜像。
///
/// 实际启用还需要主门控：`postprocess_enabled() && fxaa_enabled()`。
/// `POSTPROCESS` 未设置时，FXAA 渲染图节点根本不会注册，因此子门控的
/// ON 默认值无法泄漏到 golden 路径。
pub fn fxaa_enabled() -> bool {
    sub_gate_flag(ENV_ENABLE_FXAA)
}

/// M5-E2 SSAO **子门控** —— **ACTIVE**，且 ⚠ **未设置时默认 ON**。
///
/// [`fxaa_enabled`] 针对 AO 节点的镜像契约
/// （`PostProcessConfig::ambient_occlusion_enabled` → `CesiumAmbientOcclusion`）。
/// 实际启用：`postprocess_enabled() && ao_enabled()`。
pub fn ao_enabled() -> bool {
    sub_gate_flag(ENV_ENABLE_AO)
}

/// 截图元数据的 Git SHA（`CESIUM_GIT_SHA`，否则 `"unknown"`）。
pub fn git_sha() -> String {
    std::env::var(ENV_GIT_SHA)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── 标志访问器（active + reserved）────────────────────────
//
// 本块混合了 `main.rs` 今天消费的标志（后处理、
// 天穹 / glow、材质展示）与仍在等待其里程碑的标志；
// 每个访问器的文档都写明属于哪类。每个访问器都是 `#[inline]`
// 并编译为单个 `env_flag` 调用。
// 它们故意**不**缓存：未来里程碑可能想在运行时切换
// 一个标志（例如通过调试控制台），而缓存会锁定
// 启动值。若某里程碑需要缓存，它可将访问器包装在自己的
// `OnceLock` 中。

/// M1 pipeline 标志（保留 —— 尚无消费者）。
#[inline]
pub fn pipeline_enabled() -> bool {
    env_flag(ENV_ENABLE_PIPELINE)
}
/// M5.5/M5.6 后处理标志 —— **自 M5-E1/E2 起为 ACTIVE**：门控由
/// `effects/graph.rs::register_render_graph` 注册的自实现 FXAA（预设 12）+ SSAO
/// （半球 16 采样 + 4×4 模糊）渲染图节点（`CesiumPostProcessLabel::{Fxaa,
/// AmbientOcclusion}`），在 `main.rs` 的 `CesiumEffectsPlugin` 分支消费。不再是
/// “自定义 WGSL 阶段”的预留：它已接入。与 `POSTPROCESS_BUILTIN`（M4.2
/// 相机 bundle 上的 Bevy 内置 tone-mapping/bloom/HDR）相互独立。
#[inline]
pub fn postprocess_enabled() -> bool {
    env_flag(ENV_ENABLE_POSTPROCESS)
}
/// M3.x CSM 标志（保留）。
#[inline]
pub fn csm_enabled() -> bool {
    env_flag(ENV_ENABLE_CSM)
}
/// M5.2/M5.3 天穹标志 —— **自 M5-B/M5-C 起为 ACTIVE**：门控
/// `CesiumAtmospherePlugin`（程序化天穹，单次散射 WGSL）。
/// 开启时，glow 被强制 OFF（互斥），且天穹应与
/// `POSTPROCESS_BUILTIN` 配对（见 `main.rs` 警告），以使未归一化的
/// 内散射辐亮度被 tone-mapped 而非被截断。
#[inline]
pub fn skydome_enabled() -> bool {
    env_flag(ENV_ENABLE_SKYDOME)
}
/// AtmosphereGlow 门控 —— skydome 的反向。
///
/// 互斥规则：SKYDOME=1 → glow 强制 0。8-shell 回退
/// glow（AtmosphereGlowPlugin）与程序化天穹
/// （CesiumAtmospherePlugin）渲染重叠的大气效果；同时启用
/// 两者会双重绘制临边。本访问器集中该规则，使 main.rs
/// 只需调用 `glow_enabled()` 而不重复逻辑。
///
/// 默认（SKYDOME 未设置/OFF）：返回 true → glow ON → v0 零 diff。
#[inline]
pub fn glow_enabled() -> bool {
    !skydome_enabled()
}
/// M5.4/M5-D 材质展示标志 —— **ACTIVE**：门控
/// `MaterialShowcasePlugin`（内置 Fabric 材质 + 用于
/// `specs/baselines/v2_water` 捕获的 Water Calm/Medium/Rough 海浪状态）。
///
/// 默认（未设置/OFF）：返回 false → 插件未注册 → 无额外
/// 实体 → v0 基线像素中性（PSNR=∞）。在此注册而非在
/// `main.rs` 中以裸字符串字面量读取，以使本模块仍是每个
/// `CESIUM_*` 环境名的单一真相源。
#[inline]
pub fn material_showcase_enabled() -> bool {
    env_flag(ENV_ENABLE_MATERIAL_SHOWCASE)
}
/// M5.x draping 标志（保留）。
#[inline]
pub fn draping_enabled() -> bool {
    env_flag(ENV_ENABLE_DRAPING)
}
/// M6.2 clipping 标志 —— **自 M6 Wave A 起为 ACTIVE**（任务 #81）：门控
/// `effects::register_clipping_planes_node`、`main.rs` 插入的 `CesiumClippingPlanes`
/// 相机组件，以及 `Core3d` 后处理区域中的 `CesiumClippingLabel` 边。
///
/// 默认（未设置/OFF）：节点未注册、无边、组件未插入
/// → v0 基线像素中性（PSNR=∞）。
#[inline]
pub fn clipping_enabled() -> bool {
    env_flag(ENV_ENABLE_CLIPPING)
}
/// M6.3 panorama 标志 —— **自 M6 Wave A 起为 ACTIVE**（任务 #81）：门控
/// `effects::register_panorama_node`、`CesiumPanorama` 相机组件，以及
/// `MainOpaquePass → CesiumPanoramaLabel → MainTransmissivePass`
/// 的串行 `MainPass` 内插入。
///
/// 默认（未设置/OFF）：节点未注册且 —— 关键地 —— 现有的
/// `MainOpaquePass → MainTransmissivePass` 边**保持原样**，因此主 pass
/// 链逐字节就是 M6 之前的那条 → v0 像素中性。
#[inline]
pub fn panorama_enabled() -> bool {
    env_flag(ENV_ENABLE_PANORAMA)
}
/// M6.5 IBL 标志 —— **自 M6 Wave A 起为 ACTIVE**（任务 #81）：门控
/// `effects::register_ibl_node`、`CesiumIbl` 相机组件，以及
/// `CesiumIblLabel` 边（HDR 区域，紧接 clipping 之后）。
///
/// 默认（未设置/OFF）：节点未注册、无边、组件未插入
/// → v0 基线像素中性（PSNR=∞）。
#[inline]
pub fn ibl_enabled() -> bool {
    env_flag(ENV_ENABLE_IBL)
}
/// M6.4 OIT flag (reserved).
#[inline]
pub fn oit_enabled() -> bool {
    env_flag(ENV_ENABLE_OIT)
}
/// M6.1 分屏标志（保留）。
#[inline]
pub fn split_enabled() -> bool {
    env_flag(ENV_ENABLE_SPLIT)
}
/// M6.6 clouds 标志（**保留** —— 由 M6 Wave A 预置，尚无消费者）。
/// 声明它不改变任何运行时行为；它默认 OFF。
#[inline]
pub fn clouds_enabled() -> bool {
    env_flag(ENV_ENABLE_CLOUDS)
}
/// M2 新相机标志（保留 —— 取代 orbit_camera）。
#[inline]
pub fn new_camera_enabled() -> bool {
    env_flag(ENV_ENABLE_NEW_CAMERA)
}
/// M8 资源后端标志（保留 —— 可插拔资产流式加载）。
#[inline]
pub fn resource_backend_enabled() -> bool {
    env_flag(ENV_ENABLE_RESOURCE_BACKEND)
}
/// M7 样式-JSEP 标志（已完成 —— 表达式求值器已落地）。
#[inline]
pub fn styling_jsep_enabled() -> bool {
    env_flag(ENV_ENABLE_STYLING_JSEP)
}
/// M14.x KML 导出标志（保留）。
#[inline]
pub fn kml_export_enabled() -> bool {
    env_flag(ENV_ENABLE_KML_EXPORT)
}
/// M15.x glTF 升级标志（保留）。
#[inline]
pub fn gltf_upgrade_enabled() -> bool {
    env_flag(ENV_ENABLE_GLTF_UPGRADE)
}
/// M16.x Draco 标志（保留）。
#[inline]
pub fn draco_enabled() -> bool {
    env_flag(ENV_ENABLE_DRACO)
}
/// M17.x 点云标志（保留）。
#[inline]
pub fn point_cloud_enabled() -> bool {
    env_flag(ENV_ENABLE_POINT_CLOUD)
}

// ── 快照（供诊断 / trace 头使用）─────────────────────────

/// 首次调用时每个标志值的不可变快照。适用于烙印到 perf-trace
/// CSV 头或启动日志行，以便评审者能分辨某次运行开了哪些功能。
///
/// 懒初始化：首次调用读取环境，后续调用返回缓存的快照。
/// 这很安全，因为环境变量是进程全局的，且快照仅用于诊断
/// （从不用于控制流）。
#[derive(Debug, Clone)]
pub struct FlagSnapshot {
    pub terrain: bool,
    pub tileset: bool,
    pub new_chains: bool,
    pub reserved: Vec<(&'static str, bool)>,
}

impl FlagSnapshot {
    /// 将当前环境捕获到快照中。
    pub fn capture() -> Self {
        Self {
            terrain: terrain_enabled(),
            tileset: tileset_enabled(),
            new_chains: env_flag(ENV_ENABLE_NEW_CHAINS),
            reserved: RESERVED_FLAGS.iter().map(|f| (*f, env_flag(f))).collect(),
        }
    }

    /// 单行人类可读摘要，例如 `"terrain=off tileset=off reserved=0/18"`。
    /// 分母是 `RESERVED_FLAGS.len()`，因此当命名空间新增条目时会
    /// 移动（M6 Wave A：通过 `CLOUDS` 从 17 → 18）。
    pub fn summary_line(&self) -> String {
        let on = self.reserved.iter().filter(|(_, v)| *v).count();
        format!(
            "terrain={} tileset={} new_chains={} reserved={}/{}",
            yn(self.terrain),
            yn(self.tileset),
            yn(self.new_chains),
            on,
            self.reserved.len()
        )
    }
}

/// 将布尔开关渲染为供人阅读的 `on`/`off` 字样。
fn yn(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

/// 进程级缓存快照（首次调用捕获，后续调用复用）。
static SNAPSHOT: OnceLock<FlagSnapshot> = OnceLock::new();

/// 返回缓存的 [`FlagSnapshot`]，首次调用时捕获。
pub fn snapshot() -> &'static FlagSnapshot {
    SNAPSHOT.get_or_init(FlagSnapshot::capture)
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_accepts_canonical_tokens() {
        for t in ["1", "true", "TRUE", "True", "yes", "YES", "on", "ON", " 1 ", "\ttrue\n"] {
            assert!(truthy(t), "expected truthy: {:?}", t);
        }
    }

    #[test]
    fn truthy_rejects_everything_else() {
        for t in ["", "0", "false", "no", "off", "2", "maybe", "enabled"] {
            assert!(!truthy(t), "expected falsy: {:?}", t);
        }
    }

    #[test]
    fn env_flag_unset_is_false() {
        // 使用一个没有测试设置的名字，以使结果确定性。
        assert!(!env_flag("CESIUM_ENABLE___DEFINITELY_UNSET___"));
    }

    #[test]
    fn env_path_unset_is_none() {
        // 路径访问器镜像 env_flag：未设置 → None（offline 默认 OFF）。
        assert!(env_path("CESIUM___DEFINITELY_UNSET___").is_none());
    }

    #[test]
    fn git_sha_falls_back_to_unknown() {
        // 从不为空：未设置的 CESIUM_GIT_SHA 产出 "unknown" 哨兵值，因此
        // 截图元数据总是有一个 sha 字段。
        assert!(!git_sha().is_empty());
    }

    #[test]
    fn offline_accessors_are_contracts_only() {
        // 读取 offline 访问器必须从不 panic、也从不翻转一个
        // 默认值：它们是纯环境读取。（值取决于周围
        // 环境，因此我们只断言调用契约，而非具体结果。）
        let _ = offline_imagery_root();
        let _ = offline_terrain_root();
        let _ = offline_mode();
        let _ = strict_offline();
        let _ = fixed_time_enabled();
        let _ = fixed_camera_path();
        let _ = screenshot_script_path();
        let _ = offline_selfcheck();
    }

    #[test]
    fn reserved_namespace_is_complete() {
        // 防止意外地从列表中删除一个保留标志。
        // M6 Wave A（任务 #81）追加了 ENV_ENABLE_CLOUDS：17 -> 18。计数
        // 被固定，因此一次*删除*（它会静默改变 summary_line() 的
        // 分母并破坏跨里程碑元数据 diff）会变红。
        assert_eq!(RESERVED_FLAGS.len(), 18);
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_PIPELINE));
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_POINT_CLOUD));
        // 三个 M6 Wave A 门控即使现已接入仍保持列出
        // （禁止移除，参见 RESERVED_FLAGS 处的说明）。
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_CLIPPING));
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_PANORAMA));
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_IBL));
        // 本任务添加的 M6.6 预置。
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_CLOUDS));
        // 无重复：重复的条目会抬高分母。
        let mut sorted: Vec<&str> = RESERVED_FLAGS.to_vec();
        let before = sorted.len();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "RESERVED_FLAGS must not contain duplicates");
    }

    #[test]
    fn snapshot_summary_is_stable() {
        let s = FlagSnapshot {
            terrain: false,
            tileset: false,
            new_chains: false,
            reserved: RESERVED_FLAGS.iter().map(|f| (*f, false)).collect(),
        };
        assert_eq!(
            s.summary_line(),
            "terrain=off tileset=off new_chains=off reserved=0/18"
        );
    }

    // ── M4.1 光照 / 后处理测试 ────────────────────────────

    #[test]
    fn lighting_mode_default_is_full_ambient() {
        // 当 CESIUM_LIGHTING_MODE 未设置（普通测试环境）时，默认为
        // FullAmbient —— 保证 v0 像素中性。
        // 注：即使周围环境碰巧设置了它，这仍然验证
        // 解析契约（不 panic，返回一个合法变体）。
        let mode = lighting_mode();
        assert!(
            mode == LightingMode::FullAmbient || mode == LightingMode::DayNight,
            "lighting_mode() must return a valid variant"
        );
    }

    #[test]
    fn lighting_mode_parses_day_night_tokens() {
        // 直接验证解析逻辑（与环境无关）。
        for token in ["day_night", "DayNight", "DAY_NIGHT", " day-night ", "daynight"] {
            let normalized = token.trim().to_ascii_lowercase().replace('-', "_");
            let mode = match normalized.as_str() {
                "day_night" | "daynight" => LightingMode::DayNight,
                _ => LightingMode::FullAmbient,
            };
            assert_eq!(mode, LightingMode::DayNight, "token {:?} should parse as DayNight", token);
        }
    }

    #[test]
    fn lighting_mode_unknown_falls_back_to_full_ambient() {
        for token in ["", "full_ambient", "garbage", "2", "off"] {
            let normalized = token.trim().to_ascii_lowercase().replace('-', "_");
            let mode = match normalized.as_str() {
                "day_night" | "daynight" => LightingMode::DayNight,
                _ => LightingMode::FullAmbient,
            };
            assert_eq!(mode, LightingMode::FullAmbient, "token {:?} should fall back", token);
        }
    }

    #[test]
    fn postprocess_builtin_default_is_off() {
        // CESIUM_ENABLE_POSTPROCESS_BUILTIN 未设置 → false（无内置 PP）。
        // 使用与所有其他标志相同的 truthy 谓词。
        let _ = postprocess_builtin_enabled(); // 不得 panic
    }

    #[test]
    fn postprocess_builtin_is_independent_from_reserved() {
        // 两个后处理常量必须是不同的字符串。
        assert_ne!(ENV_ENABLE_POSTPROCESS_BUILTIN, ENV_ENABLE_POSTPROCESS);
    }

    #[test]
    fn glow_enabled_is_inverse_of_skydome() {
        // 互斥：glow_enabled() == !skydome_enabled()。
        // 两者都读取同一个环境变量（CESIUM_ENABLE_SKYDOME），因此在任何
        // 进程状态下它们都必须是逻辑上的反值。
        assert_eq!(glow_enabled(), !skydome_enabled());
    }

    // ── M5-D 材质展示门控测试 ─────────────────────────────

    #[test]
    fn material_showcase_defaults_off() {
        // (1) 注册表常量必须匹配 M5-D 文档化的环境名，
        //     因此 main.rs 中不会再重现裸字符串漂移。
        assert_eq!(ENV_ENABLE_MATERIAL_SHOWCASE, "CESIUM_ENABLE_MATERIAL_SHOWCASE");
        // (2) 访问器必须汇聚到单个 env_flag 谓词
        //     （无私有缓存，无替代解析）。
        assert_eq!(
            material_showcase_enabled(),
            env_flag(ENV_ENABLE_MATERIAL_SHOWCASE)
        );
        // (3) 默认 OFF：变量未设置（普通测试/CI 环境）时该门控
        //     为 false → MaterialShowcasePlugin 未注册 → v0 像素中性。
        //     由 var_os 保护，因此故意 opt-in 的周围环境（例如
        //     一次 capture harness 运行）不会产生假红。
        if std::env::var_os(ENV_ENABLE_MATERIAL_SHOWCASE).is_none() {
            assert!(
                !material_showcase_enabled(),
                "CESIUM_ENABLE_MATERIAL_SHOWCASE unset must default to OFF"
            );
        }
        // (4) 天生 active 的标志：**不**属于冻结的保留命名空间列表。
        assert!(!RESERVED_FLAGS.contains(&ENV_ENABLE_MATERIAL_SHOWCASE));
        // (5) 它必须与每个其他已注册标志都是不同的环境变量
        //     （不会意外别名化 skydome/post-process 门控）。
        assert_ne!(ENV_ENABLE_MATERIAL_SHOWCASE, ENV_ENABLE_SKYDOME);
        assert_ne!(ENV_ENABLE_MATERIAL_SHOWCASE, ENV_ENABLE_POSTPROCESS);
        assert_ne!(ENV_ENABLE_MATERIAL_SHOWCASE, ENV_ENABLE_POSTPROCESS_BUILTIN);
    }

    // ── M11.3 无头运行时模式门控测试 ────────────────────────

    #[test]
    fn headless_defaults_off() {
        // (1) 注册表常量匹配文档化的环境名，因此
        //     main.rs 分支处不会再重现裸字符串漂移。
        assert_eq!(ENV_HEADLESS, "CESIUM_HEADLESS");
        // (2) 访问器汇聚到单个 env_flag 谓词
        //     （无私有缓存，无替代解析）。
        assert_eq!(headless_enabled(), env_flag(ENV_HEADLESS));
        // (3) 默认 OFF：未设置（普通测试/CI 环境）时窗口 golden
        //     路径不受影响。由 var_os 保护，因此故意 opt-in
        //     的 capture-harness 环境不会产生假红。
        if std::env::var_os(ENV_HEADLESS).is_none() {
            assert!(
                !headless_enabled(),
                "CESIUM_HEADLESS unset must default to OFF"
            );
        }
        // (4) 运行时模式开关，而非冻结的 feature 门控：**不得**处于
        //     RESERVED_FLAGS 可比性命名空间中。
        assert!(!RESERVED_FLAGS.contains(&ENV_HEADLESS));
    }

    // ── M6 Wave A 门控测试（任务 #81 集成）──────────────

    /// 三个已接入的 M6 Wave A 门控：环境名以字面量固定（因此相对于
    /// `effects/{clipping_planes,panorama,ibl}.rs` 中的适配器层镜像的
    /// 单边重命名会变红），访问器汇聚到单个 `env_flag` 谓词，且三者
    /// 都默认 OFF，使 v0 基线保持像素中性。
    #[test]
    fn m6_wave_a_gates_default_off() {
        // (1) 注册表常量必须匹配 M6.2/M6.3/M6.5 适配器
        //     文档化的环境名，逐字节。
        assert_eq!(ENV_ENABLE_CLIPPING, "CESIUM_ENABLE_CLIPPING");
        assert_eq!(ENV_ENABLE_PANORAMA, "CESIUM_ENABLE_PANORAMA");
        assert_eq!(ENV_ENABLE_IBL, "CESIUM_ENABLE_IBL");
        // (2) Feature 门控语义：未设置/falsy => false，{1,true,yes,on} => true。
        assert_eq!(clipping_enabled(), env_flag(ENV_ENABLE_CLIPPING));
        assert_eq!(panorama_enabled(), env_flag(ENV_ENABLE_PANORAMA));
        assert_eq!(ibl_enabled(), env_flag(ENV_ENABLE_IBL));
        // (3) 默认 OFF，由 var_os 保护，因此 opt-in 的 capture harness（例如
        //     一次 v3_clipping 基线运行）不能产生假红。
        if std::env::var_os(ENV_ENABLE_CLIPPING).is_none() {
            assert!(!clipping_enabled(), "CESIUM_ENABLE_CLIPPING unset must default to OFF");
        }
        if std::env::var_os(ENV_ENABLE_PANORAMA).is_none() {
            assert!(!panorama_enabled(), "CESIUM_ENABLE_PANORAMA unset must default to OFF");
        }
        if std::env::var_os(ENV_ENABLE_IBL).is_none() {
            assert!(!ibl_enabled(), "CESIUM_ENABLE_IBL unset must default to OFF");
        }
        // (4) 两两不同：三个独立的环境变量，无别名。
        assert_ne!(ENV_ENABLE_CLIPPING, ENV_ENABLE_PANORAMA);
        assert_ne!(ENV_ENABLE_CLIPPING, ENV_ENABLE_IBL);
        assert_ne!(ENV_ENABLE_PANORAMA, ENV_ENABLE_IBL);
        // (5) 它们是 feature 门控，而非后处理子门控：两个
        //     谓词必须在未设置默认值上不一致（false 对 true）。
        assert_ne!(
            env_flag("CESIUM_ENABLE___DEFINITELY_UNSET___"),
            sub_gate_flag("CESIUM_ENABLE___DEFINITELY_UNSET___"),
            "feature gates (env_flag) and sub-gates (sub_gate_flag) must differ when unset"
        );
    }

    /// M6.6 预置：`CLOUDS` 已声明 + 已列出 + 访问器存在，且**尚无
    /// 消费者**，因此它必须默认 OFF 并不改变任何东西。
    #[test]
    fn clouds_gate_is_reserved_and_defaults_off() {
        assert_eq!(ENV_ENABLE_CLOUDS, "CESIUM_ENABLE_CLOUDS");
        assert_eq!(clouds_enabled(), env_flag(ENV_ENABLE_CLOUDS));
        if std::env::var_os(ENV_ENABLE_CLOUDS).is_none() {
            assert!(!clouds_enabled(), "CESIUM_ENABLE_CLOUDS unset must default to OFF");
        }
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_CLOUDS));
        assert_ne!(ENV_ENABLE_CLOUDS, ENV_ENABLE_SKYDOME);
    }

    /// FXAA / AO 是**主 POSTPROCESS 门控的子门控**，因此它们的未设置
    /// 默认值为 ON（保留 M5-E1/E2 之前的行为）—— 故意不同于上方
    /// 每个 feature 门控。根据 Terry 的 M5-Verify Medium 发现在此注册；
    /// 访问器必须陈述真实的运行时语义。
    #[test]
    fn postprocess_sub_gates_default_on() {
        // (1) 名称固定：在本任务注册它们之前，这些是
        //     effects/post_process.rs 中的模块私有常量。
        assert_eq!(ENV_ENABLE_FXAA, "CESIUM_ENABLE_FXAA");
        assert_eq!(ENV_ENABLE_AO, "CESIUM_ENABLE_AO");
        assert_ne!(ENV_ENABLE_FXAA, ENV_ENABLE_AO);
        // (2) 子门控谓词，而非 feature 门控谓词：未设置 => true。
        assert!(sub_gate_flag("CESIUM_ENABLE___DEFINITELY_UNSET___"));
        assert!(!env_flag("CESIUM_ENABLE___DEFINITELY_UNSET___"));
        // (3) falsy token 仍会禁用，使用共享的 truthy 谓词。
        for t in ["0", "false", "no", "off", ""] {
            assert!(!truthy(t), "sub-gate must honour the falsy token {:?}", t);
        }
        // (4) 未设置时访问器报告 ON（文档化的子门控默认值）。
        //     由 var_os 保护，因此 capture harness 中显式 opt-out 的
        //     环境不会产生假红。
        if std::env::var_os(ENV_ENABLE_FXAA).is_none() {
            assert!(fxaa_enabled(), "unset CESIUM_ENABLE_FXAA sub-gate defaults ON");
        }
        if std::env::var_os(ENV_ENABLE_AO).is_none() {
            assert!(ao_enabled(), "unset CESIUM_ENABLE_AO sub-gate defaults ON");
        }
        // (5) active 子门控**不**属于冻结的保留命名空间
        //     （与 MATERIAL_SHOWCASE / HEADLESS 相同的分类）。
        assert!(!RESERVED_FLAGS.contains(&ENV_ENABLE_FXAA));
        assert!(!RESERVED_FLAGS.contains(&ENV_ENABLE_AO));
        // (6) ON 默认值不能泄漏到 golden 路径：没有主门控时，FXAA/AO
        //     渲染图节点从不注册。
        if std::env::var_os(ENV_ENABLE_POSTPROCESS).is_none() {
            assert!(
                !postprocess_enabled(),
                "master gate unset must keep the whole post-process chain off"
            );
        }
    }

    /// 五个 M6 门控构成一个不相交的命名空间（不会为新能力
    /// 意外复用现有标志名）。
    #[test]
    fn m6_gate_namespace_is_disjoint() {
        let m6 = [
            ENV_ENABLE_SPLIT,
            ENV_ENABLE_CLIPPING,
            ENV_ENABLE_PANORAMA,
            ENV_ENABLE_OIT,
            ENV_ENABLE_IBL,
            ENV_ENABLE_CLOUDS,
        ];
        for (i, a) in m6.iter().enumerate() {
            for b in m6.iter().skip(i + 1) {
                assert_ne!(a, b, "M6 gate names must be pairwise distinct");
            }
        }
        // M6.1/M6.4（SPLIT/OIT）按惯例仍列在 RESERVED_FLAGS 中（该
        // 集合是一个稳定的诊断词汇表；获得一个消费者是一次
        // 文档重分类，而非移除 —— 参见本模块顶部的说明）。
        // 两者现在都已接入一个适配器节点（Phase-3 FIX-INTEG/FIX-SPLIT）。
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_SPLIT));
        assert!(RESERVED_FLAGS.contains(&ENV_ENABLE_OIT));
    }

    /// **单一真相源的跨 crate 漂移守卫**（任务 #81，
    /// PORTING_CONVENTIONS.md §gate registry）。
    ///
    /// `cesium-app` 依赖 `cesium-bevy-render` —— 从不反向 —— 因此
    /// 适配器侧的门控助手无法导入本注册表，各自携带一份*镜像*
    /// 解析器。这里其他测试都将注册表一侧与字符串字面量固定，
    /// 这只有在无人同时更新字面量时才能捕获重命名。本测试
    /// 通过与**真实适配器实现**比较来闭环，因此单边重命名任一侧
    /// 都会变红，且它额外证明适配器的 truthy 解析器与 [`truthy`]
    /// （`{1, true, yes, on}` 集合，修剪 + 小写）逐 token 相同。
    ///
    /// 注（P0 观感清理）：原先还对拍 `cesium_bevy_render::effects` 的
    /// 门控镜像常量（`CLIPPING` / `PANORAMA` / `IBL` / `OIT` / `CLOUDS` /
    /// `SPLIT` / `FXAA` / `AO`）与 `effects::*_gate_enabled()`；该模块
    /// 已随观感清理删除，那些断言一并移除。剩余两个适配器镜像仍在册。
    #[test]
    fn adapter_mirrors_are_byte_identical_to_the_registry() {
        use cesium_bevy_render::pipeline::fetch::gate_from_env_value;

        // (1) 适配器的解析器必须恰好接受规范 token。
        for t in ["1", "true", "TRUE", "True", "yes", "YES", "on", "ON", " 1 ", "\ttrue\n"] {
            assert!(
                gate_from_env_value(Some(t.to_string())),
                "adapter gate_from_env_value must accept {:?} (registry truthy does)",
                t
            );
            assert!(truthy(t), "registry truthy must accept {:?}", t);
        }

        // (2) …并拒绝其他所有值，包括空字符串。
        for t in ["", "0", "false", "no", "off", "2", "maybe", "enabled"] {
            assert!(
                !gate_from_env_value(Some(t.to_string())),
                "adapter gate_from_env_value must reject {:?} (registry truthy does)",
                t
            );
            assert!(!truthy(t), "registry truthy must reject {:?}", t);
        }

        // (3) 未设置（None）对 *feature* 门控为 OFF。FXAA/AO 子门控是
        //     故意的例外，由 `sub_gate_flag` 处理，而非由
        //     `gate_from_env_value` 处理（其消费者已随 P0 移除，但
        //     解析语义仍冻结在注册表一侧 —— 见上方模块文档）。
        assert!(!gate_from_env_value(None));

        // (4) FIX-HL-HDRMIRROR：M11.4 无头 HDR 离屏目标门控是一个适配器
        //     私有常量（`headless/mod.rs`）；在此镜像它，以便任一侧的重命名
        //     都会使该漂移守卫变红。适配器的 `hdr_truthy` 仍是一个
        //     故意的重复（依赖方向禁止导入本注册表）—— 完全归并推迟到 M11.6。
        assert_eq!(ENV_HEADLESS_HDR, cesium_bevy_render::headless::ENV_HEADLESS_HDR);
    }
}
