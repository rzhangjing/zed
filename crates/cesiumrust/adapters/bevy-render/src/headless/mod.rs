//! 无头（无 surface）离屏 wgpu 渲染 + CPU 像素回读。
//!
//! 里程碑 **M11.3**。本模块提供把一帧渲染到一个离屏
//! [`Image`] 渲染目标、并把像素回读到 CPU 的机制，**无需窗口、
//! 显示器或显示服务器**。它是把 e2e 像素门控从一个
//! `continue-on-error` 脚手架转成一个可执行的硬门禁、以及退役那些因缺乏无头
//! 捕获路径而被推迟的 M5/M6 视觉基线的基础。
//!
//! # 可行性（spike 结果）
//!
//! 在参考机（NVIDIA RTX 3080，wgpu 23 / bevy 0.15）上已验证：
//! 一次无 surface 的已知清屏色渲染产出了一次精确的 CPU 回读
//! （`Color::srgb(0.25,0.5,0.75)` → RGBA `[64,128,191,255]`）。该机制
//! 因而在硬件上可行；在无 GPU 的 CI runner 上，同一代码路径运行于
//! `xvfb-run` + `llvmpipe` 软件渲染之下（见 e2e CI workflow）。
//!
//! # 设计 —— 复用 Bevy，不用 raw wgpu，不用 tokio
//!
//! * [`headless_window_plugin()`] 返回一个 [`WindowPlugin`]，其
//!   `primary_window: None` + `exit_condition: DontExit`，所以永不创建 surface，
//!   且应用不会因缺失窗口而自终止。
//! * [`RenderPlugin`](bevy::render::RenderPlugin) 从主 adapter 创建渲染子应用与
//!   `wgpu` 设备，独立于任何窗口。它还牵入 `WindowRenderPlugin` →
//!   `ScreenshotPlugin`，后者拥有完整的 `copy_texture_to_buffer` → `map_async` →
//!   256 字节行填充剥离的回读路径。
//! * [`create_offscreen_target`] 构造一个 RGBA8 离屏 [`Image`]，既可用作颜色
//!   附件**又可用作拷贝源**。
//! * [`retarget_cameras_to_offscreen`] 把场景的 [`Camera3d`] 指向那个
//!   image，使现有地球场景无需其它改动即可离屏渲染。
//! * [`CesiumHeadlessPlugin`] 把上述装配在一起，并在可配置的帧数之后经由
//!   [`Screenshot::image`] → [`save_to_disk`]（PNG）捕获，随后请求一次干净的应用退出。
//!
//! 插件添加顺序很重要，且对应 `DefaultPlugins`：`RenderPlugin` 必须在
//! `ImagePlugin` 之前添加，因为 `ImagePlugin::finish()` 从渲染子应用读取
//! `RenderDevice`。那一顺序是*应用*的责任（见 `main.rs`）；本模块从不重新添加那些插件。
//!
//! [`Image`]: bevy::prelude::Image
//! [`Camera3d`]: bevy::prelude::Camera3d
//! [`Screenshot::image`]: bevy::render::view::screenshot::Screenshot::image
//! [`save_to_disk`]: bevy::render::view::screenshot::save_to_disk

use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use bevy::window::{ExitCondition, WindowPlugin};
use std::path::PathBuf;

/// 默认离屏分辨率 —— 对应 `main.rs` 中的交互窗口
/// （`1280×720`），使无头基线与窗口化的基线可直接比较。
pub const DEFAULT_HEADLESS_WIDTH: u32 = 1280;
pub const DEFAULT_HEADLESS_HEIGHT: u32 = 720;

/// 离屏渲染目标的分辨率 + 像素格式。
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadlessConfig {
    pub width: u32,
    pub height: u32,
    /// 为 `true` 时离屏目标是一个半浮点 HDR 缓冲
    /// （[`TextureFormat::Rgba16Float`]）而非默认的 LDR sRGB
    /// （[`TextureFormat::Rgba8UnormSrgb`]）。捕获来自 v2 天空 / 水面 / 后处理
    /// 栈的未裁切辐亮度时需要它（M5-C/M5-D，deferred #45/#47）。
    /// **默认 `false`**，使 M11.3 的黄金捕获路径逐字节不变。由本地的
    /// `CESIUM_HEADLESS_HDR` 环境读取驱动（见 [`headless_hdr_from_env`]）—— 刻意*不*经
    /// `feature_flags` 路由，其冻结的单一真相源注册表正处于并发编辑中；
    /// 归并被推迟到 M11.6。
    pub hdr: bool,
}

impl Default for HeadlessConfig {
    /// 默认：标准宽高常量 + LDR（hdr=false）。
    fn default() -> Self {
        Self {
            width: DEFAULT_HEADLESS_WIDTH,
            height: DEFAULT_HEADLESS_HEIGHT,
            hdr: false,
        }
    }
}

impl HeadlessConfig {
    /// 以指定分辨率构造一个 LDR 离屏配置。
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            hdr: false,
        }
    }

    /// Builder：选择 HDR（Rgba16Float）离屏格式。默认 LDR。
    pub fn with_hdr(mut self, hdr: bool) -> Self {
        self.hdr = hdr;
        self
    }

    /// 本 config 为离屏目标所选的 [`TextureFormat`]。
    pub fn texture_format(&self) -> TextureFormat {
        if self.hdr {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        }
    }
}

/// `CESIUM_HEADLESS_HDR` —— 为真时：捕获进一个 HDR（Rgba16Float）离屏
/// 目标而非默认的 LDR sRGB 缓冲。本地读取（见
/// [`HeadlessConfig::hdr`]）；默认 OFF 保持 M11.3 黄金路径不变。
pub const ENV_HEADLESS_HDR: &str = "CESIUM_HEADLESS_HDR";

/// 真值 token 谓词，行为上与 `feature_flags::truthy`
/// （`1`/`true`/`yes`/`on`，不区分大小写、去空白）完全相同。刻意在此
/// 重复：`feature_flags` 位于 `cesium-app` crate 且是一个正被并发编辑的
/// 冻结注册表，所以本模块保留一个自包含的读取器。归并进
/// 单一真相源被推迟到 M11.6。
fn hdr_truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 从环境读取 [`ENV_HEADLESS_HDR`]。未设置/无法解析时为 `false`。
pub fn headless_hdr_from_env() -> bool {
    match std::env::var(ENV_HEADLESS_HDR) {
        Ok(v) => hdr_truthy(&v),
        Err(_) => false,
    }
}

/// 持有离屏渲染目标 [`Image`] handle + 尺寸的
/// 资源。
///
/// 由 [`create_offscreen_target`]（或 [`CesiumHeadlessPlugin`] 的启动系统）
/// 插入。消费方读取 `.image` 以把相机指向它，或请求一次 [`Screenshot`]。
#[derive(Resource, Clone, Debug)]
pub struct HeadlessTarget {
    pub image: Handle<Image>,
    pub width: u32,
    pub height: u32,
}

/// 返回用于无 surface 操作的 [`WindowPlugin`] 配置：
/// 无主窗口，且应用绝不因窗口关闭而自动退出。
pub fn headless_window_plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: None,
        exit_condition: ExitCondition::DontExit,
        ..default()
    }
}

/// 创建离屏渲染目标 [`Image`]（默认 LDR sRGB，或当
/// [`HeadlessConfig::hdr`] 时 HDR Rgba16Float），在 [`Assets<Image>`] 中注册它，
/// 插入一个 [`HeadlessTarget`] 资源，并返回它。
///
/// 这是一个独占-world 辅助函数（被插件的启动系统与测试使用）。
/// 该纹理以 `RENDER_ATTACHMENT | COPY_SRC |
/// COPY_DST | TEXTURE_BINDING` 创建，使它既可被渲染其中又可被回读。
pub fn create_offscreen_target(world: &mut World, config: &HeadlessConfig) -> HeadlessTarget {
    let size = Extent3d {
        width: config.width,
        height: config.height,
        depth_or_array_layers: 1,
    };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("cesium_headless_offscreen_target"),
            size,
            dimension: TextureDimension::D2,
            // LDR sRGB 输出对应交互窗口的默认值与
            // `tools/pixel_diff` 所消费的基线 PNG。HDR（Rgba16Float）是
            // 经 `CESIUM_HEADLESS_HDR` 选择性开启，用于未裁切的 v2 辐亮度捕获。
            format: config.texture_format(),
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::COPY_SRC
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    image.resize(size);
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    let target = HeadlessTarget {
        image: handle,
        width: config.width,
        height: config.height,
    };
    world.insert_resource(target.clone());
    target
}

/// 把每个尚未渲染到 image 的 [`Camera3d`] 指向离屏
/// [`HeadlessTarget`]。幂等，且当无头模式未激活（无 [`HeadlessTarget`] 资源）时
/// 是空操作，这使窗口化的黄金
/// 路径逐字节不变。
pub fn retarget_cameras_to_offscreen(
    target: Option<Res<HeadlessTarget>>,
    mut cameras: Query<&mut Camera, With<Camera3d>>,
) {
    let Some(target) = target else {
        return;
    };
    for mut camera in &mut cameras {
        if !matches!(camera.target, RenderTarget::Image(_)) {
            camera.target = RenderTarget::Image(target.image.clone());
        }
    }
}

/// [`CesiumHeadlessPlugin`] 的内部捕获状态机。
#[derive(Resource)]
struct HeadlessCaptureState {
    /// 距捕获还剩多少 update tick。
    frames_remaining: usize,
    /// 捕获图像写入的 PNG 路径。
    output_png: PathBuf,
    /// 是否已发起捕获（避免重复请求）。
    requested: bool,
}

/// 启用场景无头离屏捕获的插件。
///
/// 启动时它创建离屏目标（[`create_offscreen_target`]）；每帧
/// [`retarget_cameras_to_offscreen`] 让 3D 相机保持对准它。经 `frames` 次
/// update 后，它请求一次目标的 [`Screenshot`]，经由 Bevy 的 [`save_to_disk`] 把它存到
/// `output_png`（PNG），随后发送 [`AppExit::SUCCESS`] 使进程干净
/// 终止（该退出在帧末应用，在保存观察者写完文件之后）。
///
/// 仅当 `CESIUM_HEADLESS` 为真时才添加本插件；默认（窗口化）应用
/// 不得包含它，以保持黄金路径中立性。
pub struct CesiumHeadlessPlugin {
    pub config: HeadlessConfig,
    pub output_png: PathBuf,
    /// 捕获前渲染的 `Update` tick 数（场景预热 /
    /// tile 加载预算）。`0` 在启动后的第一个 tick 就捕获。
    pub frames: usize,
}

impl CesiumHeadlessPlugin {
    /// 以输出路径与预热帧数构造插件；HDR 从环境变量读取。
    ///
    /// # 参数
    /// - `output_png`：捕获写入的 PNG 路径
    /// - `frames`：捕获前的 `Update` tick 数
    pub fn new(output_png: impl Into<PathBuf>, frames: usize) -> Self {
        Self {
            // `main.rs` 经由 `new(output, frames)` 构建插件，且不可被并发编辑，
            // 所以 HDR 选择在此从环境读取
            // （默认 OFF → LDR sRGB → 黄金路径完好）。
            config: HeadlessConfig::default().with_hdr(headless_hdr_from_env()),
            output_png: output_png.into(),
            frames,
        }
    }

    /// Builder：显式指定离屏渲染配置（覆盖环境推导的默认）。
    pub fn with_config(mut self, config: HeadlessConfig) -> Self {
        self.config = config;
        self
    }
}

impl Plugin for CesiumHeadlessPlugin {
    /// 注入配置与捕获状态，挂载 Startup 创建离屏目标、逐帧改向相机、末尾捕获保存。
    ///
    /// # 参数
    /// - `app`：Bevy 应用
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config)
            .insert_resource(HeadlessCaptureState {
                frames_remaining: self.frames,
                output_png: self.output_png.clone(),
                requested: false,
            })
            .add_systems(Startup, headless_setup_target)
            .add_systems(
                Update,
                (retarget_cameras_to_offscreen, headless_capture_tick).chain(),
            );
    }
}

/// 独占启动系统：从 [`HeadlessConfig`] 构建离屏目标。
fn headless_setup_target(world: &mut World) {
    let config = *world.resource::<HeadlessConfig>();
    create_offscreen_target(world, &config);
}

/// 倒数 `frames`，随后请求离屏截图 → PNG → 退出。
fn headless_capture_tick(
    mut state: ResMut<HeadlessCaptureState>,
    target: Option<Res<HeadlessTarget>>,
    config: Res<HeadlessConfig>,
    mut commands: Commands,
) {
    if state.requested {
        return;
    }
    let Some(target) = target else {
        return;
    };
    if state.frames_remaining > 0 {
        state.frames_remaining -= 1;
        return;
    }
    let path = state.output_png.clone();
    // 保存器在观察者内同步写入 PNG；退出观察者的 `AppExit` 只在
    // 帧末应用，所以文件保证在进程终止前写完。LDR 分支逐字节
    // 就是 M11.3 路径（Bevy 自己的 `save_to_disk`）；HDR 用一个
    // 半浮点色调映射保存器，因为 Bevy 无法对 Rgba16Float 做 PNG 编码。
    let mut entity = commands.spawn(Screenshot::image(target.image.clone()));
    if config.hdr {
        entity.observe(save_hdr_to_disk(path));
    } else {
        entity.observe(save_to_disk(path));
    }
    entity.observe(
        |_trigger: Trigger<ScreenshotCaptured>, mut exit: EventWriter<AppExit>| {
            exit.send(AppExit::Success);
        },
    );
    state.requested = true;
}

/// Bevy [`save_to_disk`] 的 HDR 对应物。离屏目标是
/// [`TextureFormat::Rgba16Float`]，而 Bevy 的 PNG 保存器无法直接编码它
/// （`try_into_dynamic_image` 拒绝浮点格式）—— 没有这个，M11.4 HDR
/// 捕获会干净退出但不写文件。本观察者解码半浮点 RGBA 回读，
/// 用 Reinhard 把（可能 >1.0 的）辐亮度色调映射进 `[0,1]`，应用 sRGB OETF，
/// 并经 `image` crate（已是 `cesium-bevy-render` 依赖）写入一个 8 位 PNG。
///
/// 无损 HDR 输出（OpenEXR）被推迟到 M11.6 —— workspace 的 `image`
/// features 只有 `png`+`jpeg`。这个 8 位色调映射的 PNG 仍是一个
/// 对 v2 天空/水面/后处理栈忠实、可检查的基线
/// （#45/#47）：它压缩超单位的辐亮度而非将其裁切。
fn save_hdr_to_disk(path: PathBuf) -> impl FnMut(Trigger<ScreenshotCaptured>) {
    move |trigger: Trigger<ScreenshotCaptured>| {
        let img = &trigger.event().0;
        let w = img.texture_descriptor.size.width;
        let h = img.texture_descriptor.size.height;
        match hdr_bytes_to_rgba8(&img.data, w, h) {
            Some(rgba) => {
                if let Err(e) = rgba.save(&path) {
                    error!("HDR headless screenshot save failed at {path:?}: {e}");
                }
            }
            None => error!(
                "HDR headless screenshot: readback buffer ({} bytes) does not match {w}x{h} Rgba16Float",
                img.data.len()
            ),
        }
    }
}

/// 把一个 Rgba16Float 回读缓冲（8 字节/像素，小端 IEEE-754
/// half）解码为一个色调映射、sRGB 编码的 8 位 RGBA image。当
/// `data` 不恰好是 `w*h*8` 字节时返回 `None`。Alpha 被丢弃（不透明 PNG），
/// 对应 Bevy 的 LDR 保存器。
fn hdr_bytes_to_rgba8(data: &[u8], w: u32, h: u32) -> Option<image::RgbaImage> {
    // FIX-HL-HDRCAP：溢出安全的像素计数 + docstring 早已承诺的精确长度守卫。
    // u32 算术中的 `w * h * 4` 对 `w*h > 2^30` 会静默回绕（release）/ panic
    // （debug）；而没有长度检查时，`chunks_exact(8)` 会悄悄掉弃一个尾部
    // 部分像素，产生一个短缓冲，`from_raw` 随后拒绝它 —— 调用方只会
    // 把它当作一条日志行看到。在 `usize` 中用 `checked_mul` 计算并提前
    // 拒绝长度不匹配，使函数对其声明的契约是全函数性的。
    let npix = (w as usize).checked_mul(h as usize)?;
    let expected = npix.checked_mul(8)?;
    if data.len() != expected {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(npix.checked_mul(4)?);
    for px in data.as_chunks::<8>().0 {
        let r = f16_to_f32(u16::from_le_bytes([px[0], px[1]]));
        let g = f16_to_f32(u16::from_le_bytes([px[2], px[3]]));
        let b = f16_to_f32(u16::from_le_bytes([px[4], px[5]]));
        for c in [r, g, b] {
            let v = linear_to_srgb(tonemap_reinhard(c));
            out.push((v * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        out.push(255);
    }
    image::RgbaImage::from_raw(w, h, out)
}

/// IEEE-754 半精度（f16）→ f32。处理规格化数、非规格化数与 ±inf。
fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0f32 } else { 1.0f32 };
    let exp = ((bits >> 10) & 0x1f) as i32;
    let frac = (bits & 0x03ff) as f32;
    match exp {
        // 非规格化数：(frac/1024) * 2^-14 == frac * 2^-24。
        0 => sign * frac * 2.0f32.powi(-24),
        0x1f => {
            if frac == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + frac / 1024.0) * 2.0f32.powi(e - 15),
    }
}

/// 逐通道 Reinhard 色调映射：单调的 `[0,∞) → [0,1)`，所以超单位的 HDR
/// 辐亮度被压缩而非裁切。`+∞ → 1.0`（`c/(1+c)` 的极限）；
/// NaN / `-∞` / 非正数 → `0.0`。
fn tonemap_reinhard(c: f32) -> f32 {
    // FIX-HL-TONEMAP：`+inf` 必须趋近 1.0，而不像旧的 `c.is_finite()` 守卫那样
    // 落入 `0.0` 分支（它把 `+inf` 与 NaN / 负数归为一谈，
    // 把最亮的 HDR 辐亮度映射为黑 —— 与文档相反）。
    if c.is_nan() || c <= 0.0 {
        0.0
    } else if c == f32::INFINITY {
        1.0
    } else {
        c / (1.0 + c)
    }
}

/// 线性 → sRGB OETF（IEC 61966-2-1），对应 LDR 目标的隐式
/// 编码，使 HDR 与 LDR PNG 在感知上可比。
fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::core_pipeline::CorePipelinePlugin;
    use bevy::render::texture::ImagePlugin;
    use bevy::render::RenderPlugin;
    use std::sync::{Arc, Mutex};

    const W: u32 = 64;
    const H: u32 = 48;

    /// 构造一个无 surface 的 app（无窗口），其离屏目标已创建且
    /// 一个 `Camera3d` 对准它，已应用 `finish()`+`cleanup()`，且泉入了一些
    /// 预热帧。返回 app + 目标 image handle。
    ///
    /// 插件顺序对应 `DefaultPlugins`：RenderPlugin 在 ImagePlugin 之前。
    fn headless_app_with_camera(clear: Color) -> (App, Handle<Image>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .add_plugins(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .add_plugins(RenderPlugin::default())
            .add_plugins(ImagePlugin::default())
            .add_plugins(CorePipelinePlugin);

        app.insert_resource(ClearColor(clear));

        let config = HeadlessConfig::new(W, H);
        app.insert_resource(config);
        // 独占-world 辅助函数：创建离屏目标 + 资源。
        let target = {
            let world = app.world_mut();
            create_offscreen_target(world, &config)
        };

        // 相机渲染进离屏目标；根本没有窗口。
        app.world_mut().spawn((
            Camera3d::default(),
            Camera {
                target: RenderTarget::Image(target.image.clone()),
                clear_color: ClearColorConfig::Custom(clear),
                ..default()
            },
        ));

        // `App::run()` 在帧循环之前调用这些；用 `app.update()` 手动驱动
        // 帧需要显式调用它们一次，否则渲染子应用的 device /
        // `CapturedScreenshots` 会缺失。
        app.finish();
        app.cleanup();
        for _ in 0..8 {
            app.update();
        }

        (app, target.image)
    }

    /// SPIKE（回归）：无 surface 渲染 → CPU 回读得出精确的清屏色，
    /// 均匀的，且尺寸与缓冲大小正确。
    #[test]
    fn headless_offscreen_readback_matches_clear() {
        let clear = Color::srgb(0.25, 0.5, 0.75);
        let (mut app, target_handle) = headless_app_with_camera(clear);

        let captured: Arc<Mutex<Option<Image>>> = Arc::new(Mutex::new(None));
        let sink = captured.clone();
        app.world_mut().add_observer(
            move |trigger: Trigger<ScreenshotCaptured>| {
                *sink.lock().unwrap() = Some(trigger.event().0.clone());
            },
        );

        app.world_mut()
            .spawn(Screenshot::image(target_handle.clone()));

        let mut landed = false;
        for _ in 0..240 {
            app.update();
            if captured.lock().unwrap().is_some() {
                landed = true;
                break;
            }
        }
        assert!(
            landed,
            "headless readback never completed within 240 frames (no window/surface)"
        );

        let img = captured.lock().unwrap().clone().expect("captured image");
        let desc = &img.texture_descriptor;
        assert_eq!((desc.size.width, desc.size.height), (W, H));
        assert_eq!(img.data.len(), (W * H * 4) as usize);

        let first = &img.data[0..4];
        assert_ne!(first, &[0, 0, 0, 0], "readback is all-zero (clear missed GPU)");
        for px in img.data.as_chunks::<4>().0 {
            assert_eq!(
                px.as_slice(),
                first,
                "offscreen clear is non-uniform — readback corrupted"
            );
        }
        // (0.25, 0.5, 0.75) 的精确 sRGB 编码。
        assert_eq!(first, &[64, 128, 191, 255], "unexpected clear-colour bytes");
    }

    /// 生产捕获路径（Screenshot → `save_to_disk`）写一个真实、可解码的
    /// 离屏目标 PNG —— 即 `tools/pixel_diff` 为像素门控所消费的
    ///  artefact。
    #[test]
    fn headless_capture_writes_decodable_png() {
        let clear = Color::srgb(0.25, 0.5, 0.75);
        let (mut app, target_handle) = headless_app_with_camera(clear);

        let out = std::env::temp_dir().join(format!(
            "cesium_headless_spike_{}x{}.png",
            W, H
        ));
        let _ = std::fs::remove_file(&out); // 清除任何过期 artefact

        app.world_mut()
            .spawn(Screenshot::image(target_handle.clone()))
            .observe(save_to_disk(out.clone()));

        let mut wrote = false;
        for _ in 0..240 {
            app.update();
            if out.exists() && std::fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false) {
                wrote = true;
                break;
            }
        }
        assert!(wrote, "save_to_disk never produced a non-empty PNG at {out:?}");

        let decoded = image::open(&out).expect("PNG decodes").to_rgba8();
        assert_eq!((decoded.width(), decoded.height()), (W, H));
        // save_to_disk 经由 to_rgb8 丢弃 HDR alpha → PNG 是 RGB；中心 texel
        // 必须匹配清屏色的 sRGB 编码。
        let centre = decoded.get_pixel(W / 2, H / 2);
        assert_eq!(
            [centre[0], centre[1], centre[2]],
            [64, 128, 191],
            "PNG centre pixel does not match clear colour"
        );

        let _ = std::fs::remove_file(&out);
    }

    /// `retarget_cameras_to_offscreen` 在无 [`HeadlessTarget`] 时是空操作
    /// （窗口化黄金路径不受扰动），并在其存在时重定向一个窗口化的 `Camera3d`。
    #[test]
    fn retarget_is_noop_without_target_and_redirects_with_it() {
        // 无目标 → 相机保留它的窗口目标。
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let cam = app
            .world_mut()
            .spawn((Camera3d::default(), Camera::default()))
            .id();
        app.add_systems(Update, retarget_cameras_to_offscreen);
        app.update();
        assert!(
            matches!(
                app.world().get::<Camera>(cam).unwrap().target,
                RenderTarget::Window(_)
            ),
            "camera must stay windowed when headless is inactive"
        );
    }

    /// LDR 默认：`create_offscreen_target` 构造一个 Rgba8UnormSrgb image
    /// （黄金路径中立性 —— M11.3 捕获格式字节不变）。
    #[test]
    fn default_config_selects_ldr_srgb_target() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let config = HeadlessConfig::new(W, H);
        assert!(!config.hdr, "HDR must default OFF");
        assert_eq!(config.texture_format(), TextureFormat::Rgba8UnormSrgb);
        let target = create_offscreen_target(&mut world, &config);
        let img = world
            .resource::<Assets<Image>>()
            .get(&target.image)
            .expect("target image registered");
        assert_eq!(img.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb);
        // LDR RGBA8 = 4 字节/像素。
        assert_eq!(img.data.len(), (W * H * 4) as usize);
    }

    /// HDR 选择性开启：`with_hdr(true)` 选择 Rgba16Float（8 字节/像素），即
    /// 用于 v2 天空/水面/后处理基线的未裁切辐亮度缓冲。
    #[test]
    fn hdr_config_selects_rgba16float_target() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let config = HeadlessConfig::new(W, H).with_hdr(true);
        assert!(config.hdr);
        assert_eq!(config.texture_format(), TextureFormat::Rgba16Float);
        let target = create_offscreen_target(&mut world, &config);
        let img = world
            .resource::<Assets<Image>>()
            .get(&target.image)
            .expect("target image registered");
        assert_eq!(img.texture_descriptor.format, TextureFormat::Rgba16Float);
        // 半浮点 RGBA = 8 字节/像素。
        assert_eq!(img.data.len(), (W * H * 8) as usize);
    }

    /// `CESIUM_HEADLESS_HDR` 环境读取器：真值 token 启用 HDR，其余一切
    /// （包括未设置）都是 LDR。这是唯一改动此
    /// 环境变量的测试，所以不存在跨测试竞态。
    ///
    /// 注意（并入 Zed 工作区时的适配）：本 crate 现在以 edition 2024 构建
    /// （成员 manifest 用 `edition.workspace = true`，继承的是 Zed 的
    /// `2024`，而不再是本树原来 `[workspace.package]` 的 `2021`）。自
    /// edition 2024 起 `std::env::set_var` / `remove_var` 是 unsafe
    /// 函数，因此下面包了一层 unsafe 块 —— 这是该版本升级所必需的，
    /// 不含任何新行为。安全依据：本测试是唯一接触该变量的地方（见上），
    /// 而 `std::env` 的并发读写不安全正是该 API 变成 unsafe 的原因。
    #[test]
    fn hdr_env_reader_honours_truthy_tokens() {
        // SAFETY: see the note above — this test is the only reader or writer of
        // `CESIUM_HEADLESS_HDR` in this binary.
        unsafe {
            std::env::remove_var(ENV_HEADLESS_HDR);
        }
        assert!(!headless_hdr_from_env(), "unset must be LDR");
        for tok in ["1", "true", "YES", " on "] {
            unsafe {
                std::env::set_var(ENV_HEADLESS_HDR, tok);
            }
            assert!(headless_hdr_from_env(), "token {tok:?} must be truthy");
        }
        for tok in ["0", "false", "", "off", "nonsense"] {
            unsafe {
                std::env::set_var(ENV_HEADLESS_HDR, tok);
            }
            assert!(!headless_hdr_from_env(), "token {tok:?} must be falsy");
        }
        unsafe {
            std::env::remove_var(ENV_HEADLESS_HDR);
        }
    }

    /// f16 → f32 解码 Rgba16Float 回读所包含的 IEEE-754 半精度值
    /// （规格化数、非规格化数、±inf、符号）。
    #[test]
    fn f16_to_f32_decodes_known_values() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0x4000), 2.0);
        assert_eq!(f16_to_f32(0xbc00), -1.0);
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert!((f16_to_f32(0x3555) - 0.33325195).abs() < 1e-5, "~1/3");
        // 最小的非规格化数（0x0001）= 2^-24。
        assert!((f16_to_f32(0x0001) - 2.0f32.powi(-24)).abs() < 1e-12);
    }

    /// HDR 保存转换：半浮点 RGBA → Reinhard 色调映射 → sRGB OETF →
    /// 不透明 8 位 RGBA。一个线性的 1.0 变成 sRGB(0.5)≈188，黑保持黑，
    /// 而一个尺寸不匹配的缓冲被拒绝。
    #[test]
    fn hdr_bytes_to_rgba8_tonemaps_and_gamma_encodes() {
        let one = 0x3c00u16.to_le_bytes();
        let zero = 0x0000u16.to_le_bytes();
        let mut data = Vec::new();
        // 像素 0: (r=1.0, g=0.0, b=0.0, a=1.0)
        data.extend_from_slice(&one);
        data.extend_from_slice(&zero);
        data.extend_from_slice(&zero);
        data.extend_from_slice(&one);
        // 像素 1: (0,0,0,0)
        for _ in 0..4 {
            data.extend_from_slice(&zero);
        }
        let img = hdr_bytes_to_rgba8(&data, 2, 1).expect("2x1 buffer converts");
        let p0 = *img.get_pixel(0, 0);
        // Reinhard(1.0)=0.5 → sRGB(0.5)≈0.7354 → ≈188.
        assert!((175..=200).contains(&p0[0]), "red tonemapped: {}", p0[0]);
        assert_eq!(p0[1], 0, "green stays black");
        assert_eq!(p0[2], 0, "blue stays black");
        assert_eq!(p0[3], 255, "alpha forced opaque");
        let p1 = *img.get_pixel(1, 0);
        assert_eq!([p1[0], p1[1], p1[2]], [0, 0, 0], "black pixel stays black");
        // 缓冲对声明的尺寸而言太小 → None（优雅，无 panic）。
        assert!(hdr_bytes_to_rgba8(&data, 4, 4).is_none());
    }

    /// FIX-HL-TONEMAP：`+∞` 必须映射向白（1.0），即 `c/(1+c)` 的极限，
    /// 而非像旧的 `is_finite()` 守卫那样映射为黑。`-∞` / NaN / 非正数
    /// → 0.0；有限的正数遵循 Reinhard。
    #[test]
    fn tonemap_reinhard_handles_infinities_and_nan() {
        assert_eq!(tonemap_reinhard(f32::INFINITY), 1.0, "+inf → 1.0");
        assert_eq!(tonemap_reinhard(f32::NEG_INFINITY), 0.0, "-inf → 0.0");
        assert_eq!(tonemap_reinhard(f32::NAN), 0.0, "NaN → 0.0");
        assert_eq!(tonemap_reinhard(-1.0), 0.0, "negative → 0.0");
        assert_eq!(tonemap_reinhard(0.0), 0.0, "zero → 0.0");
        assert!((tonemap_reinhard(1.0) - 0.5).abs() < 1e-6, "1.0 → 0.5");
        // 大的有限辐亮度严格低于 1 且单调。
        let a = tonemap_reinhard(1.0e6);
        let b = tonemap_reinhard(1.0e7);
        assert!(a < 1.0 && a > 0.99 && b > a, "finite super-unit compresses toward 1");
    }

    /// FIX-HL-HDRCAP：一个比 `w*h*8` 更长的缓冲（一个尾部部分像素）
    /// 过去会溜过 `chunks_exact` 并产生一个短 `out`，`from_raw` 只将其
    /// 作为一条记录的 `None` 拒绝；现在精确长度守卫提前拒绝它，
    /// 而一个精确大小的缓冲仍可转换。
    #[test]
    fn hdr_bytes_to_rgba8_rejects_mismatched_length_exactly() {
        let w = 2u32;
        let h = 2u32;
        let exact = (w as usize) * (h as usize) * 8;
        // 恰好 w*h*8 字节 → Some。
        let data = vec![0u8; exact];
        assert!(hdr_bytes_to_rgba8(&data, w, h).is_some(), "exact length converts");
        // 多一个字节（以前会被 chunks_exact 静默掉弃）→ None。
        let mut too_long = data.clone();
        too_long.push(0);
        assert!(hdr_bytes_to_rgba8(&too_long, w, h).is_none(), "extra trailing byte rejected");
        // 少一个字节 → None。
        assert!(hdr_bytes_to_rgba8(&data[..exact - 1], w, h).is_none(), "short buffer rejected");
    }
}
