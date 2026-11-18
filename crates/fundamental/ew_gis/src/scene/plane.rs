//! Visible 2D tile selection and instancing.
//!
//! Every visible target tile becomes one `TileInstance`. If the exact atlas
//! layer is missing, we sample the nearest resident ancestor (with the correct
//! child UV sub-rect); if no ancestor is resident yet, we sample layer 0 (the
//! equirectangular world base). The 3D globe resolves layers the same way in
//! `scene::globe`; this path is unchanged since stage 1.

use bytemuck::{Pod, Zeroable};

use crate::gpu::texture::{TileArray, WORLD_LAYER};
use crate::math::camera::PlaneCamera;
use crate::math::geo::{
    EARTH_RADIUS, GeoRect, TileId, lonlat_to_plane, lonlat_to_tile, tile_bounds, tile_uv_on_world,
};
use crate::tiles::TileSystem;

const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;
/// Two quads per tile, comfortably above the maximum visible count at the
/// clamped zoom range.
const MAX_VISIBLE_TILES: usize = 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TileInstance {
    pub center: [f32; 2],
    pub size: [f32; 2],
    pub uv_rect: [f32; 4],
    pub layer: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaneStats {
    pub target_z: u32,
    pub visible_tiles: usize,
    pub exact_tiles: usize,
    pub fallback_tiles: usize,
}

pub struct PlaneScene {
    pub instances: Vec<TileInstance>,
    pub stats: PlaneStats,
    pub center_plane: [f64; 2],
    /// Ids of the tiles that produced an instance in the last [`Self::update`],
    /// in row-major order. The MVT layer consumes these as its visible set; the
    /// tile stats above stay count-only.
    pub visible_ids: Vec<TileId>,
}

impl Default for PlaneScene {
    fn default() -> Self {
        Self {
            instances: Vec::with_capacity(MAX_VISIBLE_TILES),
            stats: PlaneStats::default(),
            center_plane: [0.0; 2],
            visible_ids: Vec::with_capacity(MAX_VISIBLE_TILES),
        }
    }
}

impl PlaneScene {
    /// Rebuild instance data and queue missing exact tiles. Reuses the instance
    /// vector's capacity; no per-frame heap allocation after warmup.
    pub fn update(
        &mut self,
        camera: &PlaneCamera,
        tiles: &mut TileSystem,
        aspect: f64,
        viewport_height: f64,
    ) {
        self.instances.clear();
        self.visible_ids.clear();
        let center = lonlat_to_plane(camera.center_lon, camera.center_lat);
        self.center_plane = center;
        let (half_x, half_y) = camera.half_extent(aspect);
        let rect =
            GeoRect::from_center_extent(camera.center_lon, camera.center_lat, half_x, half_y);
        let mut z = camera.target_zoom(viewport_height);
        // Keep the visible set below the atlas capacity so queued exact tiles
        // are not evicted immediately at very large viewport/DPR combinations.
        let (mut x0, mut y0, mut x1, mut y1) = visible_range(z, &rect);
        while (x1 - x0 + 1) * (y1 - y0 + 1) > 56 && z > 1 {
            z -= 1;
            let range = visible_range(z, &rect);
            x0 = range.0;
            y0 = range.1;
            x1 = range.2;
            y1 = range.3;
        }

        let mut exact = 0usize;
        let mut fallback = 0usize;
        let mut visible = 0usize;

        'tiles: for y in y0..=y1 {
            for x in x0..=x1 {
                if visible >= MAX_VISIBLE_TILES {
                    break 'tiles;
                }
                let id = TileId::new(z, x, y);
                let bounds = tile_bounds(id);
                if bounds.lon_max < rect.lon_min
                    || bounds.lon_min > rect.lon_max
                    || bounds.lat_max < rect.lat_min
                    || bounds.lat_min > rect.lat_max
                {
                    continue;
                }

                let (layer, uv_rect, is_exact) = if let Some(layer) = tiles.array.lookup(id) {
                    (layer, [0.0, 0.0, 1.0, 1.0], true)
                } else if let Some((layer, uv)) = lookup_parent(&mut tiles.array, id) {
                    (layer, uv, false)
                } else {
                    (WORLD_LAYER as u8, tile_uv_on_world(id), false)
                };

                if is_exact {
                    exact += 1;
                } else {
                    fallback += 1;
                    // Queue the exact tile even when an ancestor already draws.
                    tiles.request(id);
                }

                let center_lon = bounds.center().0;
                let center_lat = bounds.center().1;
                let plane = lonlat_to_plane(center_lon, center_lat);
                let width = EARTH_RADIUS * (bounds.lon_max - bounds.lon_min) * DEG_TO_RAD;
                let height = EARTH_RADIUS * (bounds.lat_max - bounds.lat_min) * DEG_TO_RAD;

                self.instances.push(TileInstance {
                    center: [(plane[0] - center[0]) as f32, (plane[1] - center[1]) as f32],
                    size: [width as f32, height as f32],
                    uv_rect,
                    layer: layer as u32,
                });
                self.visible_ids.push(id);
                visible += 1;
            }
        }

        self.stats = PlaneStats {
            target_z: z,
            visible_tiles: visible,
            exact_tiles: exact,
            fallback_tiles: fallback,
        };
    }

    pub fn instance_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.instances)
    }

    /// Visible tile ids from the last update (row-major, no duplicates).
    pub fn visible_ids(&self) -> &[TileId] {
        &self.visible_ids
    }
}

fn visible_range(z: u32, rect: &GeoRect) -> (u32, u32, u32, u32) {
    let n = TileId::count(z).max(1);
    let (x0, y0) = lonlat_to_tile(rect.lon_min, rect.lat_max, z);
    let (x1, y1) = lonlat_to_tile(rect.lon_max, rect.lat_min, z);
    (x0.min(n - 1), y0.min(n - 1), x1.min(n - 1), y1.min(n - 1))
}

/// Nearest resident ancestor, returning the child's UV sub-rect inside that
/// ancestor's Mercator tile.
fn lookup_parent(array: &mut TileArray, id: TileId) -> Option<(u8, [f32; 4])> {
    let mut child = id;
    let mut scale = 1.0f32;
    while let Some(parent) = child.parent() {
        scale *= 2.0;
        if let Some(layer) = array.lookup(parent) {
            let u0 = (id.x as f32 - parent.x as f32 * scale) / scale;
            let v0 = (id.y as f32 - parent.y as f32 * scale) / scale;
            let size = 1.0 / scale;
            return Some((layer, [u0, v0, u0 + size, v0 + size]));
        }
        child = parent;
    }
    None
}
