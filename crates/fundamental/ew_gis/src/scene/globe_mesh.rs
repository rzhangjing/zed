//! CPU-side mesh construction for the quadtree globe.
//!
//! One visible quadtree leaf contributes a `TILE_SEGMENTS` x `TILE_SEGMENTS`
//! grid on the WGS84 ellipsoid plus four skirt strips that hang inward along the
//! geodetic normal. The grid is uniform in the tile's *Mercator* vertical
//! coordinate, so the tile-local `uv` stays linear in the same parameter (that
//! is what makes a 0..1 texture rectangle map exactly, and what makes the
//! ancestor fallback a plain affine transform).
//!
//! The polar caps (lat +-85.05..90) are not covered by any WebMercator tile.
//! They are appended to the same vertex/index buffers with the equirectangular
//! world layer and its lon/lat UV, so the globe has no holes and still needs a
//! single draw call. Their geometry is static and coarse; a dedicated polar
//! quadtree is future work.

use bytemuck::{Pod, Zeroable};

use crate::gpu::texture::WORLD_LAYER;
use crate::math::geo::{
    Ellipsoid, MAX_MERCATOR_LAT, TileBounds, TileId, lonlat_to_world_uv, tile_bounds, tile_lat_at,
};
use crate::math::lod::MAX_VISIBLE_TILES;

/// Grid segments per tile edge. 16 keeps the vertex budget at ~357 vertices per
/// tile while the screen-space target (320 px) stays well above the geometric
/// error at the closest orbit altitude.
pub const TILE_SEGMENTS: u32 = 16;
/// Longitude segments of each polar cap.
pub const CAP_LON_SEGMENTS: u32 = 96;
/// Latitude rows of each polar cap (85.05 -> 90).
pub const CAP_LAT_ROWS: u32 = 6;

const GRID_VERTICES: usize = ((TILE_SEGMENTS + 1) * (TILE_SEGMENTS + 1)) as usize;
const SKIRT_VERTICES: usize = (4 * (TILE_SEGMENTS + 1)) as usize;
/// Grid plus four skirt strips.
pub const VERTICES_PER_TILE: usize = GRID_VERTICES + SKIRT_VERTICES;
const GRID_INDICES: usize = (TILE_SEGMENTS * TILE_SEGMENTS * 6) as usize;
const SKIRT_INDICES: usize = (4 * TILE_SEGMENTS * 6) as usize;
pub const INDICES_PER_TILE: usize = GRID_INDICES + SKIRT_INDICES;

pub const VERTICES_PER_CAP: usize = ((CAP_LON_SEGMENTS + 1) * (CAP_LAT_ROWS + 1)) as usize;
pub const INDICES_PER_CAP: usize = (CAP_LON_SEGMENTS * CAP_LAT_ROWS * 6) as usize;

/// Capacity of the merged buffers: every visible tile plus both caps.
pub const MAX_VERTICES: usize = MAX_VISIBLE_TILES * VERTICES_PER_TILE + 2 * VERTICES_PER_CAP;
pub const MAX_INDICES: usize = MAX_VISIBLE_TILES * INDICES_PER_TILE + 2 * INDICES_PER_CAP;

/// Skirt length as a fraction of the tile's on-surface diagonal. The crack it
/// must hide is the chord sagitta of a one-level-coarser neighbour
/// (`R * dAngular^2 / 2`); 5% of the diagonal is comfortably larger across the
/// whole z=2..16 range.
const SKIRT_FRACTION: f64 = 0.05;
const SKIRT_MIN_M: f64 = 0.5;
const SKIRT_MAX_M: f64 = 200_000.0;

/// One globe vertex: ECEF position relative to the render origin, ellipsoid
/// normal, tile-local UV, and the texture-array layer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct GlobeVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
}

/// Where a tile's `uv` comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UvSource {
    /// `uv_texture = offset + uv_tile * scale`, i.e. the tile is drawn from an
    /// ancestor layer and this is its sub-rectangle inside that ancestor.
    Tile { offset: [f32; 2], scale: f32 },
    /// The tile has no resident ancestor at all: sample the equirectangular
    /// world layer (layer 0) with per-vertex lon/lat UV.
    World,
}

/// A resolved visible tile, ready for mesh construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderTile {
    pub id: TileId,
    pub layer: u32,
    pub uv: UvSource,
    /// Projected diameter on screen, pixels (LOD metric, for logs).
    pub screen_px: f32,
    /// The exact layer for `id` is resident; false means an ancestor/world
    /// fallback is being displayed while the exact tile loads.
    pub exact: bool,
}

/// Reusable CPU mesh buffers. `build` clears and refills them in place so a
/// rebuild does not allocate after warmup.
pub struct GlobeMesh {
    pub vertices: Vec<GlobeVertex>,
    pub indices: Vec<u32>,
    pub caps: [bool; 2],
}

impl Default for GlobeMesh {
    fn default() -> Self {
        Self {
            vertices: Vec::with_capacity(MAX_VERTICES),
            indices: Vec::with_capacity(MAX_INDICES),
            caps: [false; 2],
        }
    }
}

impl GlobeMesh {
    /// Rebuild the merged mesh for the selected tiles. The vertex positions are
    /// ECEF minus `origin` (the render origin; the Earth centre today), so the
    /// shader works in one fixed frame.
    pub fn build(
        &mut self,
        tiles: &[RenderTile],
        caps: [bool; 2],
        ellipsoid: Ellipsoid,
        origin: [f64; 3],
    ) {
        self.vertices.clear();
        self.indices.clear();
        self.caps = caps;
        for tile in tiles.iter().take(MAX_VISIBLE_TILES) {
            push_tile(
                &mut self.vertices,
                &mut self.indices,
                tile,
                ellipsoid,
                origin,
            );
        }
        for (index, north) in [true, false].into_iter().enumerate() {
            if caps[index] {
                push_polar_cap(
                    &mut self.vertices,
                    &mut self.indices,
                    ellipsoid,
                    origin,
                    north,
                );
            }
        }
        debug_assert!(self.vertices.len() <= MAX_VERTICES);
        debug_assert!(self.indices.len() <= MAX_INDICES);
    }

    pub fn vertex_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.vertices)
    }

    pub fn index_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.indices)
    }
}

fn relative(position: [f64; 3], origin: [f64; 3]) -> [f32; 3] {
    [
        (position[0] - origin[0]) as f32,
        (position[1] - origin[1]) as f32,
        (position[2] - origin[2]) as f32,
    ]
}

fn push_tile(
    vertices: &mut Vec<GlobeVertex>,
    indices: &mut Vec<u32>,
    tile: &RenderTile,
    ellipsoid: Ellipsoid,
    origin: [f64; 3],
) {
    let bounds = tile_bounds(tile.id);
    let base = vertices.len() as u32;
    let segments = TILE_SEGMENTS;
    let step = 1.0 / segments as f64;
    let lon_span = bounds.lon_max - bounds.lon_min;

    for row in 0..=segments {
        let v = row as f64 * step;
        let lat = tile_lat_at(tile.id, v);
        for col in 0..=segments {
            let u = col as f64 * step;
            let lon = bounds.lon_min + lon_span * u;
            let position = ellipsoid.surface(lon, lat);
            let normal = ellipsoid.normal(lon, lat);
            let uv = match tile.uv {
                UvSource::Tile { offset, scale } => {
                    [offset[0] + u as f32 * scale, offset[1] + v as f32 * scale]
                }
                UvSource::World => lonlat_to_world_uv(lon, lat),
            };
            vertices.push(GlobeVertex {
                position: relative(position, origin),
                normal: [normal[0] as f32, normal[1] as f32, normal[2] as f32],
                uv,
                layer: tile.layer,
            });
        }
    }

    let stride = segments + 1;
    for row in 0..segments {
        for col in 0..segments {
            let a = base + row * stride + col;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let depth = skirt_depth(&bounds, ellipsoid);
    let north: Vec<u32> = (0..=segments).map(|col| base + col).collect();
    let south: Vec<u32> = (0..=segments)
        .map(|col| base + segments * stride + col)
        .collect();
    let west: Vec<u32> = (0..=segments).map(|row| base + row * stride).collect();
    let east: Vec<u32> = (0..=segments)
        .map(|row| base + row * stride + segments)
        .collect();
    for edge in [north, south, west, east] {
        push_skirt_strip(vertices, indices, &edge, depth);
    }
}

/// Duplicate one boundary row shifted inward along its normals and stitch it to
/// the surface edge, hiding the crack against a coarser neighbour.
fn push_skirt_strip(
    vertices: &mut Vec<GlobeVertex>,
    indices: &mut Vec<u32>,
    edge: &[u32],
    depth: f64,
) {
    let first = vertices.len() as u32;
    vertices.reserve(edge.len());
    for &grid_index in edge {
        let source = vertices[grid_index as usize];
        vertices.push(GlobeVertex {
            position: [
                source.position[0] - source.normal[0] * depth as f32,
                source.position[1] - source.normal[1] * depth as f32,
                source.position[2] - source.normal[2] * depth as f32,
            ],
            normal: source.normal,
            uv: source.uv,
            layer: source.layer,
        });
    }
    for pair in 0..edge.len().saturating_sub(1) {
        let a = edge[pair];
        let b = edge[pair + 1];
        let c = first + pair as u32;
        let d = c + 1;
        indices.extend_from_slice(&[a, b, c, b, d, c]);
    }
}

fn skirt_depth(bounds: &TileBounds, ellipsoid: Ellipsoid) -> f64 {
    let corners = [
        ellipsoid.surface(bounds.lon_min, bounds.lat_min),
        ellipsoid.surface(bounds.lon_min, bounds.lat_max),
        ellipsoid.surface(bounds.lon_max, bounds.lat_min),
        ellipsoid.surface(bounds.lon_max, bounds.lat_max),
    ];
    let mut diagonal: f64 = 0.0;
    for (index, a) in corners.iter().enumerate() {
        for b in corners.iter().skip(index + 1) {
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            diagonal = diagonal.max(d);
        }
    }
    (diagonal * SKIRT_FRACTION).clamp(SKIRT_MIN_M, SKIRT_MAX_M)
}

/// One polar cap, sampled as a lon/lat grid and textured from the
/// equirectangular world layer (the only texture that covers lat > 85.05).
fn push_polar_cap(
    vertices: &mut Vec<GlobeVertex>,
    indices: &mut Vec<u32>,
    ellipsoid: Ellipsoid,
    origin: [f64; 3],
    north: bool,
) {
    let base = vertices.len() as u32;
    let lat_inner = MAX_MERCATOR_LAT;
    let lat_span = 90.0 - lat_inner;
    for row in 0..=CAP_LAT_ROWS {
        let t = row as f64 / CAP_LAT_ROWS as f64;
        let lat = if north {
            lat_inner + lat_span * t
        } else {
            -lat_inner - lat_span * t
        };
        for col in 0..=CAP_LON_SEGMENTS {
            let lon = -180.0 + 360.0 * col as f64 / CAP_LON_SEGMENTS as f64;
            let position = ellipsoid.surface(lon, lat);
            let normal = ellipsoid.normal(lon, lat);
            vertices.push(GlobeVertex {
                position: relative(position, origin),
                normal: [normal[0] as f32, normal[1] as f32, normal[2] as f32],
                uv: lonlat_to_world_uv(lon, lat),
                layer: WORLD_LAYER,
            });
        }
    }

    let stride = CAP_LON_SEGMENTS + 1;
    for row in 0..CAP_LAT_ROWS {
        for col in 0..CAP_LON_SEGMENTS {
            let a = base + row * stride + col;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tile(z: u32, x: u32, y: u32, layer: u32, uv: UvSource) -> RenderTile {
        RenderTile {
            id: TileId::new(z, x, y),
            layer,
            uv,
            screen_px: 300.0,
            exact: true,
        }
    }

    #[test]
    fn tile_mesh_counts_match_the_budget() {
        let mut mesh = GlobeMesh::default();
        let tiles = vec![sample_tile(
            4,
            8,
            8,
            3,
            UvSource::Tile {
                offset: [0.0, 0.0],
                scale: 1.0,
            },
        )];
        mesh.build(&tiles, [false, false], Ellipsoid::WGS84, [0.0; 3]);
        assert_eq!(mesh.vertices.len(), VERTICES_PER_TILE);
        assert_eq!(mesh.indices.len(), INDICES_PER_TILE);
        assert_eq!(mesh.vertices.len(), 357);
        assert_eq!(mesh.indices.len(), 1920);
        assert!(VERTICES_PER_TILE * MAX_VISIBLE_TILES <= MAX_VERTICES);
    }

    #[test]
    fn exact_tile_uv_is_the_unit_square() {
        let mut mesh = GlobeMesh::default();
        mesh.build(
            &[sample_tile(
                5,
                17,
                11,
                7,
                UvSource::Tile {
                    offset: [0.0, 0.0],
                    scale: 1.0,
                },
            )],
            [false, false],
            Ellipsoid::WGS84,
            [0.0; 3],
        );
        let stride = TILE_SEGMENTS as usize + 1;
        let nw = mesh.vertices[0];
        let se = mesh.vertices[stride * stride - 1];
        assert_eq!(nw.uv, [0.0, 0.0]);
        assert_eq!(se.uv, [1.0, 1.0]);
        assert_eq!(nw.layer, 7);
        // The north edge is the tile's Mercator north edge.
        let bounds = tile_bounds(TileId::new(5, 17, 11));
        let grid = &mesh.vertices[..GRID_VERTICES];
        let nw_pos = grid[0].position;
        let expect = Ellipsoid::WGS84.surface(bounds.lon_min, bounds.lat_max);
        assert!((nw_pos[0] as f64 - expect[0]).abs() < 1.0);
        assert!((nw_pos[2] as f64 - expect[2]).abs() < 1.0);
    }

    #[test]
    fn ancestor_fallback_uv_is_an_affine_subrect() {
        let mut mesh = GlobeMesh::default();
        mesh.build(
            &[sample_tile(
                6,
                22,
                25,
                4,
                UvSource::Tile {
                    offset: [0.5, 0.5],
                    scale: 0.5,
                },
            )],
            [false, false],
            Ellipsoid::WGS84,
            [0.0; 3],
        );
        let stride = TILE_SEGMENTS as usize + 1;
        let nw = mesh.vertices[0].uv;
        let ne = mesh.vertices[TILE_SEGMENTS as usize].uv;
        let se = mesh.vertices[stride * stride - 1].uv;
        assert_eq!(nw, [0.5, 0.5]);
        assert_eq!(ne, [1.0, 0.5]);
        assert_eq!(se, [1.0, 1.0]);
    }

    #[test]
    fn world_fallback_uv_is_equirectangular() {
        let mut mesh = GlobeMesh::default();
        let id = TileId::new(3, 1, 6);
        mesh.build(
            &[sample_tile(id.z, id.x, id.y, WORLD_LAYER, UvSource::World)],
            [false, false],
            Ellipsoid::WGS84,
            [0.0; 3],
        );
        let bounds = tile_bounds(id);
        let nw = mesh.vertices[0].uv;
        let expect = lonlat_to_world_uv(bounds.lon_min, bounds.lat_max);
        assert!((nw[0] - expect[0]).abs() < 1e-6);
        assert!((nw[1] - expect[1]).abs() < 1e-6);
        assert_eq!(mesh.vertices[0].layer, WORLD_LAYER);
    }

    #[test]
    fn vertices_sit_on_or_inside_the_ellipsoid() {
        let ellipsoid = Ellipsoid::WGS84;
        let mut mesh = GlobeMesh::default();
        mesh.build(
            &[sample_tile(
                7,
                64,
                40,
                1,
                UvSource::Tile {
                    offset: [0.0, 0.0],
                    scale: 1.0,
                },
            )],
            [true, false],
            ellipsoid,
            [0.0; 3],
        );
        let outer = ellipsoid.max_radius() * 1.0005;
        let inner = ellipsoid.min_radius() * 0.98;
        let radius = |v: &GlobeVertex| {
            ((v.position[0] as f64).powi(2)
                + (v.position[1] as f64).powi(2)
                + (v.position[2] as f64).powi(2))
            .sqrt()
        };
        for vertex in &mesh.vertices {
            let r = radius(vertex);
            assert!(r <= outer, "vertex {r} above the surface");
            assert!(r >= inner, "vertex {r} far inside");
        }
        // The north skirt strip starts right after the grid, and its first
        // vertex is the inward copy of grid vertex 0.
        let grid = radius(&mesh.vertices[0]);
        let skirt = radius(&mesh.vertices[GRID_VERTICES]);
        assert!(skirt < grid, "skirt {skirt} not inside grid {grid}");
        assert!(mesh.caps[0]);
        assert!(!mesh.caps[1]);
    }

    #[test]
    fn cap_covers_the_polar_gap_only() {
        let ellipsoid = Ellipsoid::WGS84;
        let mut mesh = GlobeMesh::default();
        mesh.build(&[], [true, false], ellipsoid, [0.0; 3]);
        assert_eq!(mesh.vertices.len(), VERTICES_PER_CAP);
        assert_eq!(mesh.indices.len(), INDICES_PER_CAP);
        // The cap starts at the WebMercator cutoff, i.e. every vertex is at
        // z > 6.31e6 (the surface z at lat 85.05) and inside the polar radius.
        for vertex in &mesh.vertices {
            let z = vertex.position[2] as f64;
            assert!(z > 6_300_000.0, "cap vertex at z {z}");
            assert!(z <= ellipsoid.min_radius() + 1.0);
        }
    }
}
