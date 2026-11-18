//! Globe pass: one indexed draw for the merged quadtree tile mesh.
//!
//! The mesh is rebuilt by `GlobeScene::update` only when the LOD set, atlas
//! layer assignment, or polar caps change; otherwise this pass just reuses the
//! buffers and the pass rewrites the camera uniform.

use crate::gpu::GpuContext;
use crate::gpu::pipelines::{
    CameraUniform, create_globe_pipeline, make_pipeline_layout, tile_bind_group_layout,
};
use crate::scene::globe::GlobeScene;

pub struct GlobePass {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl GlobePass {
    pub fn new(
        gpu: &GpuContext,
        format: wgpu::TextureFormat,
        array_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let layout = tile_bind_group_layout(&gpu.device);
        let pipeline_layout = make_pipeline_layout(&gpu.device, "ew_gis globe", &layout);
        let pipeline = create_globe_pipeline(&gpu.device, format, &pipeline_layout);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis globe camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ew_gis globe bind group"),
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
        Self {
            pipeline,
            bind_group,
            uniform,
        }
    }

    pub fn set_camera(&self, gpu: &GpuContext, view_proj: glam::Mat4, eye: [f32; 3]) {
        let uniform = CameraUniform::new(view_proj, eye);
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &GlobeScene) {
        if scene.index_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
        pass.set_index_buffer(scene.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..scene.index_count, 0, 0..1);
    }
}
