//! Tile subsystem: provider + workers + LRU texture-array heap.

pub mod mvt_worker;
pub mod procedural;
pub mod provider;
pub mod worker;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use crate::gpu::GpuContext;
use crate::gpu::texture::TileArray;
use crate::math::geo::TileId;
use provider::{Provider, ProviderConfig, TileOrigin};
use worker::TileWorker;

pub const MAX_UPLOADS_PER_FRAME: usize = 4;

pub struct TileSystem {
    pub array: TileArray,
    worker: TileWorker,
    requested: HashSet<TileId>,
    pub total_uploaded: u64,
    pub total_requested: u64,
    pub last_origin: Option<TileOrigin>,
}

impl TileSystem {
    pub fn new(gpu: &GpuContext, config: ProviderConfig) -> Self {
        let start = Instant::now();
        // The provider is created first: `Single` imagery decodes its startup
        // image here, and supplies the layer-0 world base instead of the
        // procedural map. Every other kind returns `None`.
        let provider = Arc::new(Provider::new(config));
        let single_world = provider.world_image();
        let world_source = if single_world.is_some() {
            "single image"
        } else {
            "procedural"
        };
        let world = single_world
            .unwrap_or_else(|| procedural::generate_world_map(crate::gpu::texture::TILE_SIZE));
        if let Ok(path) = std::env::var("EW_GIS_DUMP_WORLD") {
            if !path.trim().is_empty() {
                if let Err(error) = image::save_buffer(
                    &path,
                    &world,
                    crate::gpu::texture::TILE_SIZE,
                    crate::gpu::texture::TILE_SIZE,
                    image::ColorType::Rgba8,
                ) {
                    log::warn!("could not dump world base map to {path}: {error}");
                } else {
                    log::info!("dumped world base map to {path}");
                }
            }
        }
        let array = TileArray::new(gpu, &world);
        let world_generation_ms = start.elapsed().as_secs_f64() * 1000.0;
        let source_label = provider.config().label();
        let thread_count = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .clamp(2, 6);
        log::info!(
            "tile source {}, {} workers, world layer from {} in {:.1} ms",
            source_label,
            thread_count,
            world_source,
            world_generation_ms
        );
        Self {
            array,
            worker: TileWorker::spawn(provider, thread_count),
            requested: HashSet::with_capacity(256),
            total_uploaded: 0,
            total_requested: 0,
            last_origin: None,
        }
    }

    /// Queue a tile for background production if it is neither resident nor
    /// already in flight.
    pub fn request(&mut self, id: TileId) {
        if self.array.contains(id) || !self.requested.insert(id) {
            return;
        }
        self.worker.request(id);
        self.total_requested += 1;
    }

    /// Drain up to four ready tiles and upload them to the atlas. Called once
    /// per rendered frame.
    pub fn upload_ready(&mut self, gpu: &GpuContext) -> usize {
        let mut uploaded = 0;
        while uploaded < MAX_UPLOADS_PER_FRAME {
            let Some(ready) = self.worker.try_recv() else {
                break;
            };
            self.requested.remove(&ready.id);
            if self.array.upload(gpu, ready.id, &ready.rgba).is_some() {
                self.total_uploaded += 1;
                uploaded += 1;
                let origin = ready.origin;
                let first = self.last_origin != Some(origin);
                self.last_origin = Some(origin);
                if first {
                    log::info!(
                        "tile {:?} uploaded from {} ({}x{} atlas layer)",
                        ready.id,
                        origin.label(),
                        crate::gpu::texture::TILE_SIZE,
                        crate::gpu::texture::TILE_SIZE
                    );
                } else {
                    log::debug!("tile {:?} uploaded from {}", ready.id, origin.label());
                }
            }
        }
        uploaded
    }

    pub fn pending(&self) -> usize {
        self.requested.len()
    }

    pub fn ready_for_screenshot(&self) -> bool {
        self.total_uploaded > 0 && self.requested.is_empty()
    }

    pub fn used_layers(&self) -> usize {
        // + layer 0 (world base).
        self.array.used_layers() + 1
    }

    pub fn resident_tiles(&self) -> usize {
        self.array.resident_count()
    }
}
