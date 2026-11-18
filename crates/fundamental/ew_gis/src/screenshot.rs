//! Offscreen frame readback and PNG encoding.
//!
//! The swapchain texture cannot be copied directly, so the offscreen path
//! renders the same scene into an Rgba8/Bgra8-sRGB texture created with
//! COPY_SRC. Two callers share it:
//!
//! * [`capture`] renders one frame and writes it to a PNG (the standalone
//!   `--screenshot` path);
//! * [`render_rgba`] returns the pixels instead, which is what the embedded Zed
//!   window presents as a GPUI `RenderImage` on every frame.

use std::path::{Path, PathBuf};

use crate::gpu::GpuContext;
use crate::gpu::frame::OffscreenTarget;
use crate::render::Renderer;
use crate::scene::SceneMode;
use crate::scene::globe::GlobeScene;
use crate::scene::icons::IconDemo;
use crate::scene::plot::PlotScene;

#[derive(Debug, thiserror::Error)]
pub enum ScreenshotError {
    #[error("screenshot I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("screenshot PNG encoding: {0}")]
    Image(#[from] image::ImageError),
    #[error("screenshot buffer mapping failed: {0}")]
    Map(String),
    #[error("screenshot device poll failed: {0}")]
    Poll(String),
    #[error("screenshot path has no parent directory")]
    NoParent,
}

/// One tightly packed RGBA8 frame, top row first.
pub struct FrameRgba {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, RGBA (never BGRA, whatever the target's
    /// format was).
    pub bytes: Vec<u8>,
}

/// Render the scene into a fresh offscreen target and read it back.
///
/// The target is allocated per call, so this is the convenience entry point for
/// one-shot captures. A host that renders continuously should keep an
/// [`OffscreenTarget`] and call [`render_rgba_into`] instead.
pub fn render_rgba(
    gpu: &GpuContext,
    renderer: &Renderer,
    plot: &PlotScene,
    icons: &IconDemo,
    globe: &GlobeScene,
    mode: SceneMode,
) -> Result<FrameRgba, ScreenshotError> {
    let target = OffscreenTarget::new(&gpu.device, gpu.width, gpu.height, gpu.format);
    render_rgba_into(gpu, &target, renderer, plot, icons, globe, mode)
}

/// Render the scene into an existing offscreen target and read it back.
///
/// This submits one command buffer and then waits for the device, so it blocks
/// for the duration of the readback. It must run on a thread that may block —
/// never on a UI executor.
pub fn render_rgba_into(
    gpu: &GpuContext,
    target: &OffscreenTarget,
    renderer: &Renderer,
    plot: &PlotScene,
    icons: &IconDemo,
    globe: &GlobeScene,
    mode: SceneMode,
) -> Result<FrameRgba, ScreenshotError> {
    let width = target.width.max(1);
    let height = target.height.max(1);

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ew_gis offscreen encoder"),
        });
    renderer.render(
        &mut encoder,
        &target.color_view,
        &target.depth.view,
        mode,
        plot,
        icons,
        globe,
    );

    let unpadded_bytes_per_row = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
    let buffer_size = padded_bytes_per_row as u64 * height as u64;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ew_gis offscreen readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    gpu.queue.submit(Some(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| ScreenshotError::Poll(error.to_string()))?;
    rx.recv()
        .map_err(|error| ScreenshotError::Map(error.to_string()))?
        .map_err(|error| ScreenshotError::Map(error.to_string()))?;

    let mapped = buffer.slice(..).get_mapped_range();
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let swap_red_blue = gpu.format == wgpu::TextureFormat::Bgra8UnormSrgb
        || gpu.format == wgpu::TextureFormat::Bgra8Unorm;
    for row in 0..height as usize {
        let src = &mapped[row * padded_bytes_per_row as usize
            ..row * padded_bytes_per_row as usize + unpadded_bytes_per_row as usize];
        let dst = &mut rgba
            [row * unpadded_bytes_per_row as usize..(row + 1) * unpadded_bytes_per_row as usize];
        dst.copy_from_slice(src);
        if swap_red_blue {
            for pixel in dst.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
    }
    drop(mapped);
    buffer.unmap();

    Ok(FrameRgba {
        width,
        height,
        bytes: rgba,
    })
}

pub fn capture(
    gpu: &GpuContext,
    renderer: &Renderer,
    plot: &PlotScene,
    icons: &IconDemo,
    globe: &GlobeScene,
    mode: SceneMode,
    path: &Path,
) -> Result<PathBuf, ScreenshotError> {
    let frame = render_rgba(gpu, renderer, plot, icons, globe, mode)?;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    } else {
        return Err(ScreenshotError::NoParent);
    }
    image::save_buffer(
        path,
        &frame.bytes,
        frame.width,
        frame.height,
        image::ColorType::Rgba8,
    )?;
    log::info!(
        "screenshot written: {} ({}x{}, mode {}, {:?})",
        path.display(),
        frame.width,
        frame.height,
        mode.label(),
        gpu.format
    );
    Ok(path.to_path_buf())
}
