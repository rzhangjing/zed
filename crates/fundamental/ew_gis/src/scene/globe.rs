//! Quadtree LOD globe scene.
//!
//! `GlobeScene` owns the merged dynamic vertex/index buffers for the whole
//! visible quadtree. Geometry lives in ECEF minus a fixed render `origin` (the
//! Earth centre), so a pure camera orbit only rewrites the camera uniform: the
//! mesh is rebuilt solely when the LOD selection, the atlas layer assignment, or
//! the polar caps change.
//!
//! Layer assignment is resolved here, not in the traversal: a visible tile with
//! a resident exact layer draws `uv = 0..1`; a tile whose exact layer is still
//! loading draws from its nearest resident ancestor with an affine sub-rect UV;
//! a tile with no resident ancestor draws from the equirectangular world layer
//! (layer 0) with lon/lat UV. That is the parent-fallback chain required by the
//! stage-2 spec, and it is what keeps the first frames from showing holes.

use std::collections::HashSet;
use std::time::Instant;

use crate::gpu::GpuContext;
use crate::gpu::texture::WORLD_LAYER;
use crate::math::camera::GlobeCamera;
use crate::math::geo::{Ellipsoid, TileId};
use crate::math::lod::{self, LodConfig, LodStats, MAX_VISIBLE_TILES};
use crate::scene::globe_mesh::{GlobeMesh, MAX_INDICES, MAX_VERTICES};
use crate::tiles::TileSystem;

pub use crate::scene::globe_mesh::{GlobeVertex, RenderTile, UvSource};

/// Render origin in ECEF metres. The Earth centre keeps the mesh independent of
/// the camera (no per-frame rebase); `f32` vertex precision is then ~0.5 m at the
/// surface, which is 0.35 px at the closest allowed orbit altitude (2 km).
pub const ORIGIN_ECEF: [f64; 3] = [0.0, 0.0, 0.0];

#[derive(Clone, Copy, Debug, Default)]
pub struct GlobeStats {
    pub visible_tiles: usize,
    pub exact_tiles: usize,
    pub fallback_tiles: usize,
    pub max_depth: u32,
    pub vertices: usize,
    pub indices: usize,
    pub draw_calls: u32,
    /// Number of completed geometry rebuilds since startup.
    pub rebuilds: u64,
    /// CPU time of the most recent rebuild (mesh build + buffer upload).
    pub rebuild_ms: f64,
    /// True when this frame rebuilt the geometry (LOD/layer/cap change).
    pub geometry_changed: bool,
    pub lod: LodStats,
}

pub struct GlobeScene {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub vertex_count: u32,
    /// Visible tiles as last uploaded (layer + UV mapping resolved).
    pub tiles: Vec<RenderTile>,
    pub stats: GlobeStats,
    pub config: LodConfig,
    pub origin: [f64; 3],
    mesh: GlobeMesh,
    scratch: Vec<RenderTile>,
    split: HashSet<TileId>,
    caps: [bool; 2],
}

impl GlobeScene {
    pub fn new(gpu: &GpuContext) -> Self {
        let config = LodConfig::from_env();
        let device = &gpu.device;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis globe tile vertices"),
            size: (MAX_VERTICES * std::mem::size_of::<GlobeVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ew_gis globe tile indices"),
            size: (MAX_INDICES * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!(
            "globe LOD: {}; buffers {} vertices / {} indices ({:.1} MiB)",
            config.label(),
            MAX_VERTICES,
            MAX_INDICES,
            ((MAX_VERTICES * std::mem::size_of::<GlobeVertex>()
                + MAX_INDICES * std::mem::size_of::<u32>()) as f64)
                / (1024.0 * 1024.0)
        );
        Self {
            vertex_buffer,
            index_buffer,
            index_count: 0,
            vertex_count: 0,
            tiles: Vec::with_capacity(config.max_tiles),
            stats: GlobeStats::default(),
            config,
            origin: ORIGIN_ECEF,
            mesh: GlobeMesh::default(),
            scratch: Vec::with_capacity(config.max_tiles),
            split: HashSet::with_capacity(config.max_tiles),
            caps: [false; 2],
        }
    }

    pub fn ellipsoid(&self) -> Ellipsoid {
        self.config.ellipsoid
    }

    /// Select the LOD set for this camera, resolve texture layers, and rebuild
    /// the merged mesh only when something visible actually changed.
    pub fn update(
        &mut self,
        gpu: &GpuContext,
        camera: &GlobeCamera,
        tiles: &mut TileSystem,
        aspect: f64,
        viewport_height: f64,
    ) -> GlobeStats {
        let selection = lod::select(camera, aspect, viewport_height, &self.config, &self.split);
        self.split = selection.split;

        self.scratch.clear();
        let mut exact = 0usize;
        let mut fallback = 0usize;
        for visible in &selection.tiles {
            let (layer, uv, is_exact) = resolve_layer(tiles, visible.id);
            if is_exact {
                exact += 1;
            } else {
                fallback += 1;
                // The exact tile is what we want; the immediate parent is
                // requested as well so the fallback chain sharpens while the
                // exact tile renders (both are deduplicated by TileSystem).
                tiles.request(visible.id);
                if let Some(parent) = visible.id.parent() {
                    tiles.request(parent);
                }
            }
            self.scratch.push(RenderTile {
                id: visible.id,
                layer,
                uv,
                screen_px: visible.screen_px as f32,
                exact: is_exact,
            });
        }
        debug_assert!(
            self.scratch.len() <= MAX_VISIBLE_TILES,
            "LOD budget exceeded"
        );

        let caps = selection.stats.caps;
        let changed = self.scratch != self.tiles || caps != self.caps;
        if changed {
            let start = Instant::now();
            self.mesh
                .build(&self.scratch, caps, self.config.ellipsoid, self.origin);
            let vertex_bytes = self.mesh.vertex_bytes();
            if !vertex_bytes.is_empty() {
                gpu.queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
            }
            let index_bytes = self.mesh.index_bytes();
            if !index_bytes.is_empty() {
                gpu.queue.write_buffer(&self.index_buffer, 0, index_bytes);
            }
            self.vertex_count = self.mesh.vertices.len() as u32;
            self.index_count = self.mesh.indices.len() as u32;
            self.stats.rebuilds += 1;
            self.stats.rebuild_ms = start.elapsed().as_secs_f64() * 1000.0;
            self.caps = caps;
            std::mem::swap(&mut self.tiles, &mut self.scratch);
        }

        self.stats.visible_tiles = self.tiles.len();
        self.stats.exact_tiles = exact;
        self.stats.fallback_tiles = fallback;
        self.stats.max_depth = self.tiles.iter().map(|tile| tile.id.z).max().unwrap_or(0);
        self.stats.vertices = self.vertex_count as usize;
        self.stats.indices = self.index_count as usize;
        self.stats.draw_calls = u32::from(self.index_count > 0);
        self.stats.geometry_changed = changed;
        self.stats.lod = selection.stats;
        self.stats
    }

    /// Total vertex capacity of the merged buffer.
    pub fn max_vertices() -> usize {
        MAX_VERTICES
    }

    pub fn max_indices() -> usize {
        MAX_INDICES
    }
}

/// Resolve which texture-array layer a visible tile draws from, and the UV
/// transform for that layer.
fn resolve_layer(tiles: &mut TileSystem, id: TileId) -> (u32, UvSource, bool) {
    if let Some(layer) = tiles.array.lookup(id) {
        return (
            layer as u32,
            UvSource::Tile {
                offset: [0.0, 0.0],
                scale: 1.0,
            },
            true,
        );
    }
    let mut child = id;
    let mut scale = 1.0f32;
    while let Some(parent) = child.parent() {
        scale *= 2.0;
        if let Some(layer) = tiles.array.lookup(parent) {
            let u0 = (id.x as f32 - parent.x as f32 * scale) / scale;
            let v0 = (id.y as f32 - parent.y as f32 * scale) / scale;
            return (
                layer as u32,
                UvSource::Tile {
                    offset: [u0, v0],
                    scale: 1.0 / scale,
                },
                false,
            );
        }
        child = parent;
    }
    (WORLD_LAYER, UvSource::World, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::globe_mesh::{
        INDICES_PER_CAP, INDICES_PER_TILE, VERTICES_PER_CAP, VERTICES_PER_TILE,
    };

    #[test]
    fn capacity_covers_a_full_lod_set() {
        // The buffer layout is one tile grid + skirts per visible tile, plus
        // both polar caps. Pin the per-tile counts so a mesh change cannot
        // silently overflow the fixed buffers.
        assert_eq!(VERTICES_PER_TILE, 357);
        assert_eq!(INDICES_PER_TILE, 1920);
        assert_eq!(
            MAX_VERTICES,
            MAX_VISIBLE_TILES * VERTICES_PER_TILE + 2 * VERTICES_PER_CAP
        );
        assert_eq!(
            MAX_INDICES,
            MAX_VISIBLE_TILES * INDICES_PER_TILE + 2 * INDICES_PER_CAP
        );
        let scene_capacity = GlobeScene::max_vertices();
        assert_eq!(scene_capacity, MAX_VERTICES);
    }

    #[test]
    fn origin_is_the_ellipsoid_centre() {
        assert_eq!(ORIGIN_ECEF, [0.0, 0.0, 0.0]);
        // Vertex positions are therefore plain ECEF; the largest coordinate is
        // the semi-major axis, comfortably inside `f32`'s integer range.
        assert!(Ellipsoid::WGS84.max_radius() < f32::MAX as f64);
    }
}
