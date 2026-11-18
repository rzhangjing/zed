//! Plot pass: three draws for the whole vector overlay.
//!
//! `plot_fill` (polygon/circle/sector triangles), `plot_line` (instanced
//! screen-space segments) and `plot_point` (instanced SDF markers) share one
//! `PlotUniform` buffer and one bind group, so switching layers costs a
//! pipeline switch and a vertex-buffer bind, nothing else. An empty layer is
//! skipped entirely.

use crate::gpu::GpuContext;
use crate::gpu::pipelines::{
    PlotUniform, create_plot_fill_pipeline, create_plot_line_pipeline,
    create_plot_point_pipeline, make_pipeline_layout, plot_bind_group_layout,
};
use crate::scene::plot::{PlotCamera, PlotScene};

pub struct PlotPass {
    fill_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl PlotPass {
    pub fn new(gpu: &GpuContext, format: wgpu::TextureFormat) -> Self {
        let layout = plot_bind_group_layout(&gpu.device);
        let pipeline_layout = make_pipeline_layout(&gpu.device, "ew_gis plot", &layout);
        let fill_pipeline = create_plot_fill_pipeline(&gpu.device, format, &pipeline_layout);
        let line_pipeline = create_plot_line_pipeline(&gpu.device, format, &pipeline_layout);
        let point_pipeline = create_plot_point_pipeline(&gpu.device, format, &pipeline_layout);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis plot camera"),
            size: std::mem::size_of::<PlotUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ew_gis plot bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            fill_pipeline,
            line_pipeline,
            point_pipeline,
            bind_group,
            uniform,
        }
    }

    /// One uniform write for all three layers.
    pub fn set_camera(&self, gpu: &GpuContext, camera: &PlotCamera) {
        let uniform = PlotUniform::new(
            camera.view_proj,
            camera.eye,
            camera.screen,
            camera.depth_bias,
        );
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &PlotScene) {
        if scene.fill_vertex_count > 0 {
            pass.set_pipeline(&self.fill_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, scene.fill_buffer.slice(..));
            pass.draw(0..scene.fill_vertex_count, 0..1);
        }
        if scene.line_instance_count > 0 {
            pass.set_pipeline(&self.line_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, scene.line_buffer.slice(..));
            pass.draw(0..6, 0..scene.line_instance_count);
        }
        if scene.point_instance_count > 0 {
            pass.set_pipeline(&self.point_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, scene.point_buffer.slice(..));
            pass.draw(0..6, 0..scene.point_instance_count);
        }
    }
}
