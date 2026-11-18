//! Icon pass: one instanced draw for all animated textured quads.

use crate::gpu::GpuContext;
use crate::gpu::pipelines::{
    CameraUniform, create_icon_pipeline, icon_bind_group_layout, make_pipeline_layout,
};

pub struct IconPass {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    draw_count: u32,
}

impl IconPass {
    pub fn new(
        gpu: &GpuContext,
        format: wgpu::TextureFormat,
        icon_view: &wgpu::TextureView,
        icon_sampler: &wgpu::Sampler,
    ) -> Self {
        let layout = icon_bind_group_layout(&gpu.device);
        let pipeline_layout = make_pipeline_layout(&gpu.device, "ew_gis icon", &layout);
        let pipeline = create_icon_pipeline(&gpu.device, format, &pipeline_layout);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis icon camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ew_gis icon bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(icon_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(icon_sampler),
                },
            ],
        });
        Self {
            pipeline,
            bind_group,
            uniform,
            draw_count: 0,
        }
    }

    pub fn set_camera(&self, gpu: &GpuContext, view_proj: glam::Mat4, eye: [f32; 3]) {
        let uniform = CameraUniform::new(view_proj, eye);
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn set_count(&mut self, count: usize) {
        self.draw_count = count as u32;
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, instances: &wgpu::Buffer) {
        if self.draw_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.draw(0..6, 0..self.draw_count);
    }
}
