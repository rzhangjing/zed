//! Quadtree LOD selection for the ellipsoid globe.
//!
//! The traversal starts at a 4x4 grid of `ROOT_Z` tiles and repeatedly splits
//! the visible leaf whose projected size on screen is largest, until every leaf
//! is at or below `target_tile_px` (or the depth/size budget stops it). Working
//! "largest first" is what makes the tile budget behave: near, high-detail tiles
//! win the atlas layers, and anything left unsplit is displayed by its parent,
//! which still covers the same ground.
//!
//! Two conservative culls run on every candidate: the six frustum planes and the
//! Earth horizon (the tangent plane through the eye, using the polar radius so a
//! visible tile is never culled). Both use `f64` ECEF bounding spheres; the
//! culling result is the same space the mesh is built in.

use std::collections::HashSet;

use glam::DVec3;

use crate::gpu::texture::LAYER_COUNT;
use crate::math::camera::{Frustum, GlobeCamera, HorizonOccluder};
use crate::math::geo::{Ellipsoid, EllipsoidKind, TileId, tile_bounds};

/// Quadtree roots: 4x4 tiles at z=2. z=0/z=1 tiles are only ever used as
/// texture fallbacks, never as mesh nodes.
pub const ROOT_Z: u32 = 2;
/// Deepest mesh level. At the closest orbit altitude (2 km) a z=16 tile is
/// ~430 px across, so this is a real cap, not an unreachable one.
pub const MAX_Z: u32 = 16;
/// Split while a tile's projected diameter exceeds this many pixels.
pub const TARGET_TILE_PX: f64 = 320.0;
/// A node that was already split last frame needs to shrink below
/// `target * SPLIT_HYSTERESIS` before it is merged again; prevents LOD flicker
/// when the camera sits exactly on a threshold.
pub const SPLIT_HYSTERESIS: f64 = 0.85;
/// Visible-tile budget. One atlas layer is left free on purpose: uploads happen
/// before the frame's layer lookup, so a spare layer guarantees that a newly
/// uploaded tile evicts a tile that is no longer on screen instead of one that
/// is still visible (which would request it again and ping-pong the LRU).
pub const MAX_VISIBLE_TILES: usize = (LAYER_COUNT - 2) as usize;

/// Tunables for the traversal (env-overridable so the acceptance runs can
/// compare settings without a rebuild).
#[derive(Clone, Copy, Debug)]
pub struct LodConfig {
    pub root_z: u32,
    pub max_z: u32,
    pub target_tile_px: f64,
    pub max_tiles: usize,
    pub ellipsoid: Ellipsoid,
    pub ellipsoid_kind: EllipsoidKind,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            root_z: ROOT_Z,
            max_z: MAX_Z,
            target_tile_px: TARGET_TILE_PX,
            max_tiles: MAX_VISIBLE_TILES,
            ellipsoid: Ellipsoid::WGS84,
            ellipsoid_kind: EllipsoidKind::Wgs84,
        }
    }
}

impl LodConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(value) = std::env::var("EW_GIS_GLOBE_TARGET_PX") {
            match value.trim().parse::<f64>() {
                Ok(px) if px.is_finite() => config.target_tile_px = px.clamp(64.0, 1024.0),
                _ => log::warn!("ignoring EW_GIS_GLOBE_TARGET_PX={value:?}"),
            }
        }
        if let Ok(value) = std::env::var("EW_GIS_GLOBE_MAX_Z") {
            match value.trim().parse::<u32>() {
                Ok(z) => config.max_z = z.clamp(config.root_z + 1, 18),
                Err(_) => log::warn!("ignoring EW_GIS_GLOBE_MAX_Z={value:?}"),
            }
        }
        if let Ok(value) = std::env::var("EW_GIS_GLOBE_ELLIPSOID") {
            match EllipsoidKind::parse(&value) {
                Some(kind) => {
                    config.ellipsoid_kind = kind;
                    config.ellipsoid = kind.ellipsoid();
                }
                None => log::warn!("ignoring EW_GIS_GLOBE_ELLIPSOID={value:?}"),
            }
        }
        config.max_tiles = config.max_tiles.clamp(4, MAX_VISIBLE_TILES);
        config
    }

    pub fn label(&self) -> String {
        format!(
            "{} root z{}..{} target {} px, {} tiles",
            self.ellipsoid_kind.label(),
            self.root_z,
            self.max_z,
            self.target_tile_px,
            self.max_tiles
        )
    }
}

/// One rendered quadtree leaf.
#[derive(Clone, Copy, Debug)]
pub struct VisibleTile {
    pub id: TileId,
    /// Projected diameter on screen, pixels.
    pub screen_px: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LodStats {
    pub visible: usize,
    pub max_depth: u32,
    pub splits: usize,
    pub frustum_culled: usize,
    pub horizon_culled: usize,
    /// True when the tile budget (not the screen-space target) stopped the
    /// refinement, i.e. the view wants more layers than the atlas has.
    pub budget_limited: bool,
    /// North/south polar cap meshes, in that order.
    pub caps: [bool; 2],
}

pub struct LodSelection {
    pub tiles: Vec<VisibleTile>,
    /// Nodes split this frame, fed back in as the hysteresis input.
    pub split: HashSet<TileId>,
    pub stats: LodStats,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    id: TileId,
    screen_px: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cull {
    Frustum,
    Horizon,
}

/// Run the quadtree refinement for one camera. Pure: no GPU, no tile I/O.
pub fn select(
    camera: &GlobeCamera,
    aspect: f64,
    viewport_height_px: f64,
    config: &LodConfig,
    prev_split: &HashSet<TileId>,
) -> LodSelection {
    let view = camera.view(aspect, config.ellipsoid);
    let frustum = view.frustum();
    let eye = view.eye_vec3();
    let occluder = HorizonOccluder::new(eye, config.ellipsoid);
    let half_fov = (camera.fov_y_deg as f64).to_radians() * 0.5;
    let focal_px = (viewport_height_px.max(1.0) * 0.5) / half_fov.tan();

    let mut stats = LodStats::default();
    let mut nodes: Vec<Node> = Vec::with_capacity(config.max_tiles + 4);
    let mut split: HashSet<TileId> = HashSet::with_capacity(config.max_tiles);

    let roots = 1u32 << config.root_z;
    for y in 0..roots {
        for x in 0..roots {
            let id = TileId::new(config.root_z, x, y);
            match node_for(id, eye, &frustum, &occluder, config.ellipsoid, focal_px) {
                Ok(node) => nodes.push(node),
                Err(Cull::Frustum) => stats.frustum_culled += 1,
                Err(Cull::Horizon) => stats.horizon_culled += 1,
            }
        }
    }

    loop {
        let Some(index) = best_split_candidate(&nodes, config, prev_split) else {
            break;
        };
        // A split turns one node into up to four; requiring room for three keeps
        // the visible count under the atlas budget at all times. The candidate
        // test comes first so `budget_limited` only reports a real truncation.
        if nodes.len() + 3 > config.max_tiles {
            stats.budget_limited = true;
            break;
        }
        let node = nodes.swap_remove(index);
        split.insert(node.id);
        stats.splits += 1;
        for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let child = TileId::new(node.id.z + 1, node.id.x * 2 + dx, node.id.y * 2 + dy);
            match node_for(child, eye, &frustum, &occluder, config.ellipsoid, focal_px) {
                Ok(node) => nodes.push(node),
                Err(Cull::Frustum) => stats.frustum_culled += 1,
                Err(Cull::Horizon) => stats.horizon_culled += 1,
            }
        }
    }

    nodes.sort_by_key(|node| (node.id.z, node.id.y, node.id.x));
    stats.visible = nodes.len();
    stats.max_depth = nodes.iter().map(|node| node.id.z).max().unwrap_or(0);
    stats.caps = [
        sphere_visible(
            cap_center(config.ellipsoid, true),
            CAP_RADIUS,
            &frustum,
            &occluder,
        ),
        sphere_visible(
            cap_center(config.ellipsoid, false),
            CAP_RADIUS,
            &frustum,
            &occluder,
        ),
    ];

    LodSelection {
        tiles: nodes
            .into_iter()
            .map(|node| VisibleTile {
                id: node.id,
                screen_px: node.screen_px,
            })
            .collect(),
        split,
        stats,
    }
}

fn best_split_candidate(
    nodes: &[Node],
    config: &LodConfig,
    prev_split: &HashSet<TileId>,
) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, node) in nodes.iter().enumerate() {
        if node.id.z >= config.max_z {
            continue;
        }
        let threshold = if prev_split.contains(&node.id) {
            config.target_tile_px * SPLIT_HYSTERESIS
        } else {
            config.target_tile_px
        };
        if node.screen_px < threshold {
            continue;
        }
        let better = match best {
            None => true,
            Some(current) => {
                let other = &nodes[current];
                if (node.screen_px - other.screen_px).abs() > 1e-9 {
                    node.screen_px > other.screen_px
                } else {
                    (node.id.z, node.id.y, node.id.x) < (other.id.z, other.id.y, other.id.x)
                }
            }
        };
        if better {
            best = Some(index);
        }
    }
    best
}

fn node_for(
    id: TileId,
    eye: DVec3,
    frustum: &Frustum,
    occluder: &HorizonOccluder,
    ellipsoid: Ellipsoid,
    focal_px: f64,
) -> Result<Node, Cull> {
    let bounds = tile_bounds(id);
    let (lon_min, lon_max) = (bounds.lon_min, bounds.lon_max);
    let (lat_min, lat_max) = (bounds.lat_min, bounds.lat_max);
    let center = DVec3::from_array(ellipsoid.surface(
        (lon_min + lon_max) * 0.5,
        (lat_min + lat_max) * 0.5,
    ));
    // The patch is contained in the sphere around its surface midpoint; the
    // corners are its farthest points, plus a small margin for the flattened
    // ellipsoid's asymmetry.
    let mut radius: f64 = 0.0;
    for (lon, lat) in [
        (lon_min, lat_min),
        (lon_min, lat_max),
        (lon_max, lat_min),
        (lon_max, lat_max),
    ] {
        let corner = DVec3::from_array(ellipsoid.surface(lon, lat));
        radius = radius.max((corner - center).length());
    }
    radius *= 1.02;
    if !frustum.intersects_sphere(center, radius) {
        return Err(Cull::Frustum);
    }
    if !occluder.sphere_visible(center, radius) {
        return Err(Cull::Horizon);
    }
    let distance = (center - eye).length();
    let screen_px = 2.0 * radius * focal_px / (distance - radius).max(1.0);
    Ok(Node { id, screen_px })
}

/// Approximate bounding radius of a polar cap mesh: the chord between the cap's
/// centre point (lat ~87.5) and a ring point on the far side of the pole
/// (~832 km), with margin.
const CAP_RADIUS: f64 = 900_000.0;

fn cap_center(ellipsoid: Ellipsoid, north: bool) -> DVec3 {
    let mid_lat = (90.0 + crate::math::geo::MAX_MERCATOR_LAT) * 0.5;
    let lat = if north { mid_lat } else { -mid_lat };
    DVec3::from_array(ellipsoid.surface(0.0, lat))
}

fn sphere_visible(center: DVec3, radius: f64, frustum: &Frustum, occluder: &HorizonOccluder) -> bool {
    frustum.intersects_sphere(center, radius) && occluder.sphere_visible(center, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection_for(camera: &GlobeCamera) -> LodSelection {
        select(
            camera,
            16.0 / 9.0,
            1080.0,
            &LodConfig::default(),
            &HashSet::new(),
        )
    }

    #[test]
    fn default_view_is_bounded_and_well_formed() {
        let camera = GlobeCamera::default();
        let selection = selection_for(&camera);
        assert!(!selection.tiles.is_empty());
        assert!(selection.tiles.len() <= MAX_VISIBLE_TILES);
        assert!(selection.stats.max_depth >= ROOT_Z);
        assert!(selection.stats.max_depth <= MAX_Z);
        let mut seen = HashSet::new();
        for tile in &selection.tiles {
            assert!(seen.insert(tile.id), "duplicate tile {:?}", tile.id);
            assert!(tile.id.z >= ROOT_Z && tile.id.z <= MAX_Z);
            // No selected tile may be the ancestor of another selected tile:
            // that would draw the same ground twice.
            let mut parent = tile.id.parent();
            while let Some(candidate) = parent {
                assert!(!seen.contains(&candidate), "ancestor of {:?} drawn", tile.id);
                parent = candidate.parent();
            }
        }
        // Some part of the globe is always under the camera.
        assert!(selection.tiles.iter().any(|tile| tile.screen_px > 0.0));
    }

    #[test]
    fn zooming_in_adds_depth_and_a_finer_leaf() {
        let mut camera = GlobeCamera::default();
        let far = selection_for(&camera);
        camera.distance = crate::math::geo::EARTH_RADIUS + 40_000.0;
        let close = selection_for(&camera);
        assert!(
            close.stats.max_depth > far.stats.max_depth,
            "close z{} far z{}",
            close.stats.max_depth,
            far.stats.max_depth
        );
        // A close look at a small patch is covered by fewer, much finer tiles.
        assert!(close.tiles.len() <= MAX_VISIBLE_TILES);
        assert!(close.tiles.iter().all(|tile| tile.id.z > ROOT_Z));
        assert!(
            close.tiles.iter().any(|tile| tile.id.z >= MAX_Z.saturating_sub(4)),
            "expected at least one tile near the depth cap"
        );
    }

    #[test]
    fn selection_is_deterministic() {
        let camera = GlobeCamera::default();
        let a = selection_for(&camera);
        let b = selection_for(&camera);
        let ids_a: Vec<_> = a.tiles.iter().map(|tile| tile.id).collect();
        let ids_b: Vec<_> = b.tiles.iter().map(|tile| tile.id).collect();
        assert_eq!(ids_a, ids_b);
        assert_eq!(a.stats.visible, b.stats.visible);
    }

    #[test]
    fn hysteresis_keeps_previously_split_children() {
        let camera = GlobeCamera::default();
        let config = LodConfig::default();
        let first = select(&camera, 16.0 / 9.0, 1080.0, &config, &HashSet::new());
        let second = select(&camera, 16.0 / 9.0, 1080.0, &config, &first.split);
        // The split set is a fixed point under the hysteresis band.
        assert_eq!(first.split, second.split);
        assert_eq!(first.stats.visible, second.stats.visible);
    }

    #[test]
    fn default_view_shows_the_north_cap_only() {
        let camera = GlobeCamera::default();
        let selection = selection_for(&camera);
        assert!(selection.stats.caps[0], "north cap should be visible");
        assert!(!selection.stats.caps[1], "south cap is past the horizon");
    }
}
