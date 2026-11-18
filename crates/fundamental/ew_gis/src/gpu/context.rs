//! wgpu 29 device/surface bootstrap.
//!
//! Two constructors share the adapter/device half: [`GpuContext::new`] for the
//! standalone `winit` window (a swapchain surface) and
//! [`GpuContext::new_headless`] for a host that renders offscreen and reads the
//! frame back (the Zed GPUI window). The surface and its configuration are
//! optional, so every method here is safe on a headless context.

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("could not create a wgpu surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("no compatible GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("could not create a wgpu device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("surface advertises no sRGB format")]
    NoSrgbSurfaceFormat,
}

pub struct GpuContext {
    /// Kept alive for the lifetime of the surface; `Surface` internally
    /// dispatches through this instance.
    #[allow(dead_code)]
    pub instance: wgpu::Instance,
    #[allow(dead_code)]
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// The swapchain surface. `None` for a headless context, which renders into
    /// an offscreen target instead (see `screenshot::OffscreenTarget`).
    pub surface: Option<wgpu::Surface<'static>>,
    /// The surface configuration. `None` exactly when `surface` is `None`.
    pub config: Option<wgpu::SurfaceConfiguration>,
    /// Preferred sRGB color format. Also used for the screenshot/offscreen
    /// target so the same pipelines can render both destinations.
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
}

impl GpuContext {
    /// Create a context that presents to `window`'s swapchain.
    pub async fn new(window: Arc<Window>, size: PhysicalSize<u32>) -> Result<Self, GpuError> {
        let instance = new_instance();
        let surface = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;
        let (device, queue) = request_device(&adapter).await?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .ok_or(GpuError::NoSrgbSurfaceFormat)?;

        let width = size.width.max(1);
        let height = size.height.max(1);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        log::info!(
            "surface: {}x{}, format {:?}, present AutoVsync, depth Depth32Float",
            width,
            height,
            format
        );

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface: Some(surface),
            config: Some(config),
            format,
            width,
            height,
        })
    }

    /// Create a context with no surface: every frame goes to an offscreen
    /// target that the caller reads back.
    ///
    /// Blocking, because that is the only synchronous shape a host thread can
    /// call; it must not run on a UI executor that forbids `block_on`. The
    /// offscreen path's format is always [`wgpu::TextureFormat::Rgba8UnormSrgb`],
    /// the same sRGB pipelines the window path uses.
    pub fn new_headless(width: u32, height: u32) -> Result<Self, GpuError> {
        pollster::block_on(Self::new_headless_async(width, height))
    }

    async fn new_headless_async(width: u32, height: u32) -> Result<Self, GpuError> {
        let instance = new_instance();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                // No surface to be compatible with: any adapter that can make a
                // device is fine.
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await?;
        let (device, queue) = request_device(&adapter).await?;

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let width = width.max(1);
        let height = height.max(1);
        log::info!("headless: {width}x{height}, format {format:?}");

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface: None,
            config: None,
            format,
            width,
            height,
        })
    }

    /// Resize both the tracked size and, when there is one, the surface.
    ///
    /// A headless context only records the new size: its offscreen target is
    /// reallocated by whoever owns it (`viewer::Viewer::resize`).
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        if let Some(config) = self.config.as_mut() {
            config.width = width;
            config.height = height;
        }
        if let (Some(surface), Some(config)) = (self.surface.as_ref(), self.config.as_ref()) {
            surface.configure(&self.device, config);
            log::info!("resized surface to {width}x{height}");
        } else {
            log::info!("resized headless target to {width}x{height}");
        }
    }

    pub fn aspect(&self) -> f64 {
        self.width as f64 / self.height.max(1) as f64
    }

    /// Reconfigure the swapchain after a suboptimal/outdated/lost surface.
    ///
    /// A no-op on a headless context, which has no surface to reconfigure.
    pub fn reconfigure(&self) {
        if let (Some(surface), Some(config)) = (self.surface.as_ref(), self.config.as_ref()) {
            surface.configure(&self.device, config);
        }
    }
}

fn new_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

async fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GpuError> {
    let info = adapter.get_info();
    log::info!(
        "adapter: {} ({:?}, {:?}), driver {} {}",
        info.name,
        info.backend,
        info.device_type,
        info.driver,
        info.driver_info
    );

    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("ew_gis device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        })
        .await
        .map_err(GpuError::Device)
}
