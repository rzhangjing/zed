//! The map's wgpu surface: an instance, device and swap chain on the native child window.
//!
//! This is the "no read-back" half of the GPU path. [`GpuPresenter::present`] draws a prepared
//! [`rustgis_render::gpu_scene::GpuScene`] into the child window's swap chain and presents it, so
//! the pixels never travel back to the CPU: the window shows the vectors the GPU drew, at the
//! window's device resolution.
//!
//! It runs on the engine thread (which owns the `Session`); only the HWND value crosses threads.
//! The child window itself is created, moved and destroyed on the UI thread (see `crate::win`).

#![cfg(target_os = "windows")]

use rustgis_map::Map;
use rustgis_render::View;
use rustgis_render::gpu_scene::GpuScene;

use crate::gpu::GpuMap;
use crate::win::MapWindowHandle;

use wgpu30 as wgpu;

/// A wgpu device and swap chain presenting into the map's child window.
pub struct GpuPresenter {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    map: GpuMap,
    /// The device's name, for the log and the status line.
    pub adapter: String,
    /// The surface's texture format: the map pipeline draws into it.
    pub format: wgpu::TextureFormat,
}

impl GpuPresenter {
    /// Create the surface for `hwnd`, pick an adapter that can present to it, and build the map
    /// pipelines for the surface's format. `size` is the child's physical (device-pixel) size.
    pub fn new(hwnd: isize, size: (u32, u32)) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let window = MapWindowHandle(hwnd);
        // SAFETY: the HWND is the live child window, created on the UI thread and only destroyed
        // after this presenter is dropped (`RustGisView::drop` detaches the engine first).
        let surface: wgpu::Surface<'static> = unsafe {
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_display_and_window(&window, &window)
                        .map_err(|error| format!("the map's HWND is not a valid surface: {error}"))?,
                )
                .map_err(|error| format!("could not create the map's surface: {error}"))?
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|error| format!("no GPU adapter can draw the map's window: {error}"))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rustgis-map"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults()
                .using_resolution(adapter.limits())
                .using_alignment(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .map_err(|error| format!("could not create the map's GPU device: {error}"))?;

        let caps = surface.get_capabilities(&adapter);
        // A non-sRGB target, as the CPU renderer's pixmap is: the shader's premultiplied values
        // are the sRGB bytes tiny-skia writes, and an sRGB view would encode them a second time.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| "the map's surface has no texture format".to_string())?;
        let width = size.0.max(1);
        let height = size.1.max(1);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let map = GpuMap::new(&device, &queue, format, (width, height));
        log::info!(
            "[rustgis] GPU map surface: {} ({:?}), {width}x{height}, {format:?}, present {:?}",
            info.name,
            info.backend,
            config.present_mode
        );
        Ok(GpuPresenter { surface, device, queue, config, map, adapter: info.name, format })
    }

    /// The swap chain's physical size.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Reconfigure the swap chain for a new physical size.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), String> {
        let width = size.0.max(1);
        let height = size.1.max(1);
        if (width, height) == (self.config.width, self.config.height) {
            return Ok(());
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.map.resize(&self.device, size);
        log::info!("[rustgis] GPU map surface resized to {width}x{height}");
        Ok(())
    }

    /// Upload `scene`, draw it, and present it. Returns the draw and present time.
    pub fn present(&mut self, map: &mut Map, view: &View, scene: &GpuScene, dpi: f32) -> Result<(), String> {
        self.resize(physical(view, dpi))?;
        if !self.map.prepare(&self.device, &self.queue, map, view, scene, dpi) {
            return Err("the map scene exceeds this GPU's limits".to_string());
        }
        self.draw_and_present()
    }

    /// Draw and present the scene already uploaded, re-projecting it for a moved camera (a pan
    /// or zoom without re-tessellation). `Ok(false)` when the scene cannot cover `view` (a CPU
    /// image run, a turn or tilt, a zoom past what f32 holds): the caller builds a new scene.
    pub fn present_reusing(&mut self, view: &View, dpi: f32) -> Result<bool, String> {
        self.resize(physical(view, dpi))?;
        if !self.map.prepare_reusing(&self.queue, view, dpi) {
            return Ok(false);
        }
        self.draw_and_present().map(|()| true)
    }

    /// Acquire a swap chain image, draw the prepared scene into it, and present it.
    fn draw_and_present(&mut self) -> Result<(), String> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                // Nothing is on screen to show: skip the frame, as the surface API asks.
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err("the map's surface was lost".to_string());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("the map's surface rejected a frame (validation error)".to_string());
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rustgis-map-frame"),
        });
        self.map.present(&mut encoder, &view);
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }

    /// The device's name.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Upload the label text layer (premultiplied RGBA at the surface's device resolution): the
    /// GPU pass draws it over the map.
    pub fn set_labels(&mut self, rgba: &[u8], width: u32, height: u32) {
        self.map.set_overlay(&self.device, &self.queue, width, height, rgba);
    }

    /// Drop the label layer (the map is moving: labels placed for the old camera would be stale).
    pub fn clear_labels(&mut self) {
        self.map.clear_overlay();
    }
}

/// The swap chain's physical size for `view` at `dpi`.
fn physical(view: &View, dpi: f32) -> (u32, u32) {
    let ppp = if dpi.is_finite() && dpi > 0.0 { dpi.clamp(0.1, 8.0) } else { 1.0 };
    (
        (view.width * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32,
        (view.height * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32,
    )
}
