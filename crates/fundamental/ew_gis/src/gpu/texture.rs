//! Fixed 512x512 RGBA8 texture array with 64 layers.
//!
//! Layer 0 is reserved for the equirectangular world base map (shared by the
//! globe and by the 2D plane's parent/root fallback). Layers 1..64 are an LRU
//! tile heap. Uploading is rate-limited by `TileSystem`, not by this type.

use lru::LruCache;
use std::num::NonZeroUsize;

use crate::gpu::GpuContext;
use crate::math::geo::TileId;

pub const TILE_SIZE: u32 = 512;
pub const LAYER_COUNT: u32 = 64;
pub const WORLD_LAYER: u32 = 0;
const TILE_LAYERS: usize = (LAYER_COUNT - 1) as usize;

pub struct TileArray {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// tile -> array layer, LRU ordered.
    resident: LruCache<TileId, u8>,
    next_free_layer: u8,
    pub evictions: u64,
    pub uploads: u64,
}

impl TileArray {
    pub fn new(gpu: &GpuContext, world_rgba: &[u8]) -> Self {
        assert_eq!(
            world_rgba.len(),
            (TILE_SIZE * TILE_SIZE * 4) as usize,
            "world base map must be a 512x512 RGBA image"
        );

        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ew_gis tile array"),
            size: wgpu::Extent3d {
                width: TILE_SIZE,
                height: TILE_SIZE,
                depth_or_array_layers: LAYER_COUNT,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ew_gis tile array view"),
            format: None,
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            usage: None,
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(LAYER_COUNT),
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ew_gis tile sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: WORLD_LAYER,
                },
                aspect: wgpu::TextureAspect::All,
            },
            world_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TILE_SIZE * 4),
                rows_per_image: Some(TILE_SIZE),
            },
            wgpu::Extent3d {
                width: TILE_SIZE,
                height: TILE_SIZE,
                depth_or_array_layers: 1,
            },
        );

        let mut resident = LruCache::new(NonZeroUsize::new(TILE_LAYERS).unwrap());
        // Keep the cache full-ordered from the start; no tile occupies a layer
        // until the first upload.
        let _ = &mut resident;

        Self {
            texture,
            view,
            sampler,
            resident,
            next_free_layer: 1,
            evictions: 0,
            uploads: 0,
        }
    }

    /// Look up a tile and mark it MRU. Returns its array layer.
    pub fn lookup(&mut self, id: TileId) -> Option<u8> {
        self.resident.get(&id).copied()
    }

    pub fn contains(&self, id: TileId) -> bool {
        self.resident.peek(&id).is_some()
    }

    /// Upload one already-decoded 512x512 RGBA tile. Returns the layer used.
    pub fn upload(&mut self, gpu: &GpuContext, id: TileId, rgba: &[u8]) -> Option<u8> {
        if rgba.len() != (TILE_SIZE * TILE_SIZE * 4) as usize {
            log::warn!(
                "tile {:?} has {} bytes, expected {}",
                id,
                rgba.len(),
                TILE_SIZE * TILE_SIZE * 4
            );
            return None;
        }

        let layer = if let Some(layer) = self.resident.get(&id).copied() {
            layer
        } else {
            let layer = if self.resident.len() >= TILE_LAYERS {
                let (_evicted, layer) = self.resident.pop_lru().expect("LRU is non-empty");
                self.evictions += 1;
                layer
            } else {
                let layer = self.next_free_layer;
                self.next_free_layer = self.next_free_layer.saturating_add(1);
                layer
            };
            self.resident.put(id, layer);
            layer
        };

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TILE_SIZE * 4),
                rows_per_image: Some(TILE_SIZE),
            },
            wgpu::Extent3d {
                width: TILE_SIZE,
                height: TILE_SIZE,
                depth_or_array_layers: 1,
            },
        );
        self.uploads += 1;
        Some(layer)
    }

    /// Number of non-reserved layers currently occupied (0..=63).
    pub fn used_layers(&self) -> usize {
        self.resident.len()
    }

    pub fn resident_count(&self) -> usize {
        self.resident.len()
    }
}
