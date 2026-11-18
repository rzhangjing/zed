//! Tile pass: one instanced draw for every visible plane tile.

use crate::gpu::GpuContext;
use crate::gpu::pipelines::{
    CameraUniform, create_tile_pipeline, make_pipeline_layout, tile_bind_group_layout,
};
use crate::scene::plane::TileInstance;

const MAX_TILE_INSTANCES: usize = 1024;

pub struct TilePass {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    instances: wgpu::Buffer,
    draw_count: u32,
}

impl TilePass {
    pub fn new(
        gpu: &GpuContext,
        format: wgpu::TextureFormat,
        array_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let layout = tile_bind_group_layout(&gpu.device);
        let pipeline_layout = make_pipeline_layout(&gpu.device, "ew_gis tile", &layout);
        let pipeline = create_tile_pipeline(&gpu.device, format, &pipeline_layout);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis tile camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ew_gis tile bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis tile instances"),
            size: (MAX_TILE_INSTANCES * std::mem::size_of::<TileInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group,
            uniform,
            instances,
            draw_count: 0,
        }
    }

    pub fn set_camera(&self, gpu: &GpuContext, view_proj: glam::Mat4, eye: [f32; 3]) {
        let uniform = CameraUniform::new(view_proj, eye);
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn upload(&mut self, gpu: &GpuContext, bytes: &[u8], count: usize) {
        let count = count.min(MAX_TILE_INSTANCES);
        self.draw_count = count as u32;
        if count == 0 {
            return;
        }
        let byte_len = count * std::mem::size_of::<TileInstance>();
        gpu.queue
            .write_buffer(&self.instances, 0, &bytes[..byte_len]);
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.draw_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.draw_count);
    }
}
