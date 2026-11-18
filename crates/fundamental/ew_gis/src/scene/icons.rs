//! 1024 instanced, textured icon quads. Each instance carries a full
//! `T * R * S` matrix built from a quaternion; the demo animates all three Euler
//! axes so the perspective camera sees foreshortened, side-tilted images rather
//! than only in-plane rotations.

use bytemuck::{Pod, Zeroable};
use glam::{EulerRot, Mat3, Mat4, Quat, Vec3};

use crate::gpu::GpuContext;
use crate::math::camera::{GlobeCamera, PlaneCamera};
use crate::math::geo::{Ellipsoid, lonlat_to_plane};

pub const ICON_COUNT: usize = 1024;
pub const ICON_TEXTURE_SIZE: u32 = 256;
const ICON_ALTITUDE_MAX: f32 = 150_000.0;

/// Altitude of the 2D icon overlay above the tile plane for a given eye height.
///
/// Shared with the vector plot pass, which caps its clip-space depth bias so a
/// semi-transparent fill drawn earlier can never punch the icon pass out (icons
/// are drawn last and do not write depth).
pub fn demo_altitude(height_m: f32) -> f32 {
    (height_m * 0.015).clamp(500.0, ICON_ALTITUDE_MAX)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct IconInstance {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
}

pub struct IconDemo {
    /// Owns the GPU texture; the bind group uses `view`.
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub instances: Vec<IconInstance>,
    pub buffer: wgpu::Buffer,
    positions: Vec<[f64; 2]>,
    sizes: Vec<f32>,
}

impl IconDemo {
    pub fn new(gpu: &GpuContext) -> Self {
        let pixels = generate_icon(ICON_TEXTURE_SIZE);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ew_gis icon texture"),
            size: wgpu::Extent3d {
                width: ICON_TEXTURE_SIZE,
                height: ICON_TEXTURE_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ICON_TEXTURE_SIZE * 4),
                rows_per_image: Some(ICON_TEXTURE_SIZE),
            },
            wgpu::Extent3d {
                width: ICON_TEXTURE_SIZE,
                height: ICON_TEXTURE_SIZE,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ew_gis icon sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis icon instances"),
            size: (ICON_COUNT * std::mem::size_of::<IconInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // A 32x32 grid centered on the default East-Asia view. Positions are
        // world-anchored (not screen-anchored), so panning moves the demo with
        // the map. The icons are intentionally kept small enough that the tile
        // grid and coastlines remain readable underneath.
        let mut positions = Vec::with_capacity(ICON_COUNT);
        let mut sizes = Vec::with_capacity(ICON_COUNT);
        let grid = 32.0f64;
        for index in 0..ICON_COUNT {
            let gx = (index % 32) as f64;
            let gy = (index / 32) as f64;
            let lon = 120.0 + (gx - (grid - 1.0) * 0.5) * 2.15;
            let lat = 30.0 + (gy - (grid - 1.0) * 0.5) * 1.25;
            positions.push(lonlat_to_plane(lon, lat));
            let wobble = ((index as f32 * 12.9898).sin() * 0.5 + 0.5).abs();
            sizes.push(78_000.0 + wobble * 44_000.0);
        }

        Self {
            texture,
            view,
            sampler,
            instances: vec![IconInstance::zeroed(); ICON_COUNT],
            buffer,
            positions,
            sizes,
        }
    }

    pub fn update(&mut self, gpu: &GpuContext, camera: &PlaneCamera, time: f32) {
        let center = lonlat_to_plane(camera.center_lon, camera.center_lat);
        // Keep the demo at a roughly constant screen size and just above the
        // plane across the whole zoom range.
        let size_scale = (camera.height / 9_000_000.0) as f32;
        let altitude = demo_altitude(camera.height as f32);
        const PALETTE: [[f32; 3]; 6] = [
            [0.38, 0.92, 1.00],
            [1.00, 0.76, 0.30],
            [0.55, 1.00, 0.62],
            [1.00, 0.42, 0.58],
            [0.78, 0.67, 1.00],
            [1.00, 0.95, 0.82],
        ];

        for (index, instance) in self.instances.iter_mut().enumerate() {
            let position = self.positions[index];
            let relative = Vec3::new(
                (position[0] - center[0]) as f32,
                (position[1] - center[1]) as f32,
                altitude,
            );

            let phase = index as f32;
            let rx = time * 0.58 + phase * 0.137 + (phase * 0.71).sin() * 0.85;
            let ry = time * 0.81 + phase * 0.093 + (phase * 0.37).cos() * 0.70;
            let rz = time * 0.43 + phase * 0.618;
            let rotation = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
            let scale = self.sizes[index] * size_scale;
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(scale, scale, 1.0),
                rotation,
                relative,
            );

            let color = PALETTE[index % PALETTE.len()];
            let pulse = 0.78 + 0.22 * (time * 1.7 + phase * 0.31).sin().abs();
            instance.model = model.to_cols_array_2d();
            instance.color = [color[0], color[1], color[2], pulse];
        }

        gpu.queue
            .write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.instances));
    }

    /// Globe overlay: the same 1024 instances, anchored to the WGS84 surface
    /// with a tangent frame (`east`, `north`, `up`) and the same three-axis
    /// local rotation. Scale is derived from the camera's metres-per-pixel so
    /// the icons keep a roughly constant screen size while orbiting/zooming.
    pub fn update_globe(
        &mut self,
        gpu: &GpuContext,
        camera: &GlobeCamera,
        ellipsoid: Ellipsoid,
        viewport_height: f64,
        time: f32,
    ) {
        const PALETTE: [[f32; 3]; 6] = [
            [0.38, 0.92, 1.00],
            [1.00, 0.76, 0.30],
            [0.55, 1.00, 0.62],
            [1.00, 0.42, 0.58],
            [0.78, 0.67, 1.00],
            [1.00, 0.95, 0.82],
        ];

        let surface_gap = (camera.distance - ellipsoid.a).max(10_000.0);
        let mpp = 2.0 * surface_gap * (camera.fov_y_deg.to_radians() * 0.5).tan()
            / viewport_height.max(1.0);
        let base_scale = (16.0 * mpp) as f32;

        let grid = 32.0f64;
        for (index, instance) in self.instances.iter_mut().enumerate() {
            let gx = (index % 32) as f64;
            let gy = (index / 32) as f64;
            // Spread the demo over the visible hemisphere around the camera.
            let lon = camera.yaw_deg + (gx - (grid - 1.0) * 0.5) * 3.5;
            let lat = (camera.pitch_deg + (gy - (grid - 1.0) * 0.5) * 1.8).clamp(-80.0, 80.0);

            let scale =
                base_scale * (0.72 + 0.5 * ((index as f32 * 12.9898).sin() * 0.5 + 0.5).abs());
            let altitude = (scale as f64 * 0.5).clamp(2_000.0, 80_000.0);
            let position = ellipsoid.point(lon, lat, altitude);

            let up64 = ellipsoid.normal(lon, lat);
            let up = Vec3::new(up64[0] as f32, up64[1] as f32, up64[2] as f32);
            let east = Vec3::Z.cross(up).normalize_or_zero();
            let north = up.cross(east);
            let tangent = Quat::from_mat3(&Mat3::from_cols(east, north, up));

            let phase = index as f32;
            let rx = time * 0.58 + phase * 0.137 + (phase * 0.71).sin() * 0.85;
            let ry = time * 0.81 + phase * 0.093 + (phase * 0.37).cos() * 0.70;
            let rz = time * 0.43 + phase * 0.618;
            let local = Quat::from_euler(EulerRot::XYZ, rx, ry, rz);
            let rotation = tangent * local;

            let translation = Vec3::new(position[0] as f32, position[1] as f32, position[2] as f32);
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(scale, scale, 1.0),
                rotation,
                translation,
            );

            let color = PALETTE[index % PALETTE.len()];
            let pulse = 0.78 + 0.22 * (time * 1.7 + phase * 0.31).sin().abs();
            instance.model = model.to_cols_array_2d();
            instance.color = [color[0], color[1], color[2], pulse];
        }

        gpu.queue
            .write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.instances));
    }
}

/// Procedural compass/star icon with a transparent background, a ring, and a
/// soft glow. The alpha edge is antialiased in texture space.
fn generate_icon(size: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let half = size as f32 * 0.5;
    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5 - half) / half;
            let py = (y as f32 + 0.5 - half) / half;
            let r = (px * px + py * py).sqrt();
            let theta = py.atan2(px);

            // Five-point star silhouette with alternating radius.
            let star_edge = 0.30 + 0.29 * (5.0 * theta).cos().abs().powf(0.70);
            let aa = 2.0 / half;
            let star_alpha = smoothstep(star_edge + aa, star_edge - aa, r);
            let ring_alpha = 1.0 - smoothstep(0.020, 0.034, (r - 0.79).abs());
            let glow = (1.0 - smoothstep(0.30, 0.95, r)) * 0.22;
            let core_alpha = smoothstep(0.23, 0.15, r);
            let alpha = (star_alpha.max(ring_alpha) * 0.96 + glow).min(1.0);

            let warm = (1.0 - smoothstep(0.0, 0.55, r)) * 0.85 + core_alpha * 0.15;
            let base = [0.30 + warm * 0.70, 0.88 + warm * 0.08, 1.00 - warm * 0.25];
            let core_mix = core_alpha * 0.75;
            let color = [
                base[0] * (1.0 - core_mix) + 1.0 * core_mix,
                base[1] * (1.0 - core_mix) + 0.98 * core_mix,
                base[2] * (1.0 - core_mix) + 0.82 * core_mix,
            ];

            let index = ((y * size + x) * 4) as usize;
            pixels[index] = (color[0].clamp(0.0, 1.0) * 255.0) as u8;
            pixels[index + 1] = (color[1].clamp(0.0, 1.0) * 255.0) as u8;
            pixels[index + 2] = (color[2].clamp(0.0, 1.0) * 255.0) as u8;
            pixels[index + 3] = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    pixels
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < f32::EPSILON {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
