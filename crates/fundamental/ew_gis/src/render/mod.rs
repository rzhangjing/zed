//! Scene recorder shared by the window swapchain and the offscreen screenshot
//! target. All pipelines target the same sRGB format and Depth32Float, so the
//! exact same recording function can be used for both.
//!
//! Draw order: `Plane` is tile → plot fill → plot line → plot point → icons;
//! `Globe` is globe → plot fill → plot line → plot point → icons. The vector
//! plot sits above the base map and below the icon overlay, and all three plot
//! layers share one camera uniform.

pub mod globe_pass;
pub mod icon_pass;
pub mod plot_pass;
pub mod tile_pass;

use crate::gpu::GpuContext;
use crate::scene::SceneMode;
use crate::scene::globe::GlobeScene;
use crate::scene::icons::IconDemo;
use crate::scene::plot::{PlotCamera, PlotScene};
use crate::tiles::TileSystem;

pub use globe_pass::GlobePass;
pub use icon_pass::IconPass;
pub use plot_pass::PlotPass;
pub use tile_pass::TilePass;

pub struct Renderer {
    pub tile: TilePass,
    pub icon: IconPass,
    pub globe: GlobePass,
    pub plot: PlotPass,
}

impl Renderer {
    pub fn new(gpu: &GpuContext, tiles: &TileSystem, icons: &IconDemo) -> Self {
        let tile = TilePass::new(gpu, gpu.format, &tiles.array.view, &tiles.array.sampler);
        let icon = IconPass::new(gpu, gpu.format, &icons.view, &icons.sampler);
        let globe = GlobePass::new(gpu, gpu.format, &tiles.array.view, &tiles.array.sampler);
        let plot = PlotPass::new(gpu, gpu.format);
        Self {
            tile,
            icon,
            globe,
            plot,
        }
    }

    pub fn set_plane_camera(&self, gpu: &GpuContext, view_proj: glam::Mat4, eye: [f32; 3]) {
        self.tile.set_camera(gpu, view_proj, eye);
        self.icon.set_camera(gpu, view_proj, eye);
    }

    pub fn set_globe_camera(&self, gpu: &GpuContext, view_proj: glam::Mat4, eye: [f32; 3]) {
        self.globe.set_camera(gpu, view_proj, eye);
        // The same camera drives the globe-anchored icon overlay.
        self.icon.set_camera(gpu, view_proj, eye);
    }

    /// The plot uniform is written once per frame for all three plot layers.
    pub fn set_plot_camera(&self, gpu: &GpuContext, camera: &PlotCamera) {
        self.plot.set_camera(gpu, camera);
    }

    pub fn upload_tiles(&mut self, gpu: &GpuContext, plane: &crate::scene::plane::PlaneScene) {
        self.tile
            .upload(gpu, plane.instance_bytes(), plane.instances.len());
    }

    pub fn set_icon_count(&mut self, count: usize) {
        self.icon.set_count(count);
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        mode: SceneMode,
        plot: &PlotScene,
        icons: &IconDemo,
        globe: &GlobeScene,
    ) {
        let color_attachments = [Some(wgpu::RenderPassColorAttachment {
            view: color_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.006,
                    g: 0.010,
                    b: 0.020,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })];
        let depth_stencil_attachment = wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ew_gis scene pass"),
            color_attachments: &color_attachments,
            depth_stencil_attachment: Some(depth_stencil_attachment),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        match mode {
            SceneMode::Plane => {
                self.tile.draw(&mut pass);
                self.plot.draw(&mut pass, plot);
                self.icon.draw(&mut pass, &icons.buffer);
            }
            SceneMode::Globe => {
                self.globe.draw(&mut pass, globe);
                self.plot.draw(&mut pass, plot);
                self.icon.draw(&mut pass, &icons.buffer);
            }
        }
    }

    /// Draw calls per frame: the base map (tile or globe) + three plot layers +
    /// icons. A plot layer with no geometry skips its draw, so this is the
    /// fixed upper bound the report quotes.
    pub fn draw_calls(_mode: SceneMode) -> u32 {
        5
    }
}
