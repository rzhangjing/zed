//! Per-frame render targets: window depth and, when requested, an offscreen
//! sRGB color target with COPY_SRC for screenshot readback.

use wgpu::TextureFormat;

pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

pub struct DepthTarget {
    /// Owns the depth allocation; `view` is what the passes bind.
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl DepthTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ew_gis depth"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width: width.max(1),
            height: height.max(1),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width.max(1) && self.height == height.max(1) {
            return;
        }
        *self = Self::new(device, width, height);
    }
}

/// Only allocated for screenshot capture and the headless/embedded viewer.
pub struct OffscreenTarget {
    pub color: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    pub depth: DepthTarget,
    /// Allocation size. Kept here so the readback does not have to trust a
    /// caller's idea of the window size (the embedded viewer's target and the
    /// GPU context's size can differ for one frame after a resize).
    pub width: u32,
    pub height: u32,
}

impl OffscreenTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32, format: TextureFormat) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ew_gis screenshot color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth = DepthTarget::new(device, width, height);
        Self {
            color,
            color_view,
            depth,
            width,
            height,
        }
    }
}
