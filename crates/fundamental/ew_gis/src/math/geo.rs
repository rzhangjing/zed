//! Geodesy, WebMercator tile addressing, and the equirectangular plane mapping.
//!
//! The 2D plane is the simple "plate carrée" world used by stage 1:
//!
//! ```text
//! x = R * lon_rad
//! y = R * lat_rad
//! z = 0
//! ```
//!
//! Tiles still use the standard WebMercator XYZ scheme (OSM numbering). A tile
//! therefore has rectangular lon/lat bounds and a rectangular footprint on the
//! plane. Its texture is Mercator-projected, so at high latitude it is stretched
//! vertically on this plane; the 3D globe replaces that mapping with the WGS84
//! ellipsoid and its artifact disappears.
//!
//! Stage 2 adds [`Ellipsoid`] (WGS84 by default, sphere selectable), the ECEF
//! bridge used by the globe mesh/camera, and the tile-local helpers the
//! quadtree LOD traversal and mesh builder need. ECEF uses the astronomy
//! convention `x = lon 0, y = lon 90E, z = north`; the globe mesh keeps that
//! convention end to end.

/// WGS84 semi-major axis (metres). Also the stage-1 sphere/plane radius.
pub const EARTH_RADIUS: f64 = 6_378_137.0;
/// WGS84 flattening: 1 / 298.257223563.
pub const WGS84_FLATTENING: f64 = 1.0 / 298.257_223_563;
/// WGS84 semi-minor axis (metres).
pub const WGS84_POLAR_RADIUS: f64 = EARTH_RADIUS * (1.0 - WGS84_FLATTENING);
/// WebMercator latitude cutoff: atan(sinh(pi)) in degrees.
pub const MAX_MERCATOR_LAT: f64 = 85.051_128_779_806_6;
pub const EARTH_CIRCUMFERENCE: f64 = std::f64::consts::TAU * EARTH_RADIUS;

const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;
const RAD_TO_DEG: f64 = 180.0 / std::f64::consts::PI;

/// Which reference surface the globe mesh and camera use.
///
/// The default is WGS84. `Sphere` keeps the stage-1 geometry (semi-major axis
/// every direction) so the two can be compared with one env var.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EllipsoidKind {
    Wgs84,
    Sphere,
}

impl EllipsoidKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "wgs84" | "ellipsoid" | "earth" => Some(Self::Wgs84),
            "sphere" | "spherical" | "ball" => Some(Self::Sphere),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Wgs84 => "WGS84",
            Self::Sphere => "sphere",
        }
    }

    pub fn ellipsoid(self) -> Ellipsoid {
        match self {
            Self::Wgs84 => Ellipsoid::WGS84,
            Self::Sphere => Ellipsoid::SPHERE,
        }
    }
}

/// A biaxial ellipsoid of revolution (WGS84 or a sphere).
///
/// All methods take geodetic longitude/latitude in degrees and altitudes in
/// metres above the surface, and return ECEF metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipsoid {
    /// Semi-major (equatorial) axis.
    pub a: f64,
    /// Semi-minor (polar) axis.
    pub b: f64,
}

impl Ellipsoid {
    pub const WGS84: Self = Self {
        a: EARTH_RADIUS,
        b: WGS84_POLAR_RADIUS,
    };
    pub const SPHERE: Self = Self {
        a: EARTH_RADIUS,
        b: EARTH_RADIUS,
    };

    /// First eccentricity squared.
    pub fn e2(self) -> f64 {
        1.0 - (self.b * self.b) / (self.a * self.a)
    }

    /// Geodetic surface point (altitude 0) in ECEF metres.
    pub fn surface(self, lon_deg: f64, lat_deg: f64) -> [f64; 3] {
        self.point(lon_deg, lat_deg, 0.0)
    }

    /// Geodetic coordinate with an altitude above the ellipsoid, in ECEF metres.
    pub fn point(self, lon_deg: f64, lat_deg: f64, altitude: f64) -> [f64; 3] {
        let e2 = self.e2();
        let lon = lon_deg * DEG_TO_RAD;
        let lat = lat_deg * DEG_TO_RAD;
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        // Prime vertical radius of curvature.
        let n = self.a / (1.0 - e2 * sin_lat * sin_lat).sqrt();
        [
            (n + altitude) * cos_lat * lon.cos(),
            (n + altitude) * cos_lat * lon.sin(),
            (n * (1.0 - e2) + altitude) * sin_lat,
        ]
    }

    /// Unit geodetic surface normal (the "up" direction) in ECEF. It is the
    /// lon/lat unit direction, independent of the flattening.
    pub fn normal(self, lon_deg: f64, lat_deg: f64) -> [f64; 3] {
        lonlat_to_unit(lon_deg, lat_deg)
    }

    /// Conservative inner (polar) radius for horizon/occlusion tests.
    pub fn min_radius(self) -> f64 {
        self.b.min(self.a)
    }

    /// Conservative outer (equatorial) radius; also the far-plane allowance.
    pub fn max_radius(self) -> f64 {
        self.a.max(self.b)
    }

    /// Inverse of [`Self::point`]: ECEF metres to `(lon, lat, altitude)` in
    /// degrees and metres.
    ///
    /// Fixed-point iteration (the standard geodetic latitude solve); the height
    /// uses the division-free identity `h = p·cos(lat) + z·sin(lat) - a·√(1-e²sin²lat)`
    /// so it stays well defined at the poles.
    pub fn to_geodetic(self, point: [f64; 3]) -> (f64, f64, f64) {
        let (x, y, z) = (point[0], point[1], point[2]);
        let lon = y.atan2(x);
        let p = (x * x + y * y).sqrt();
        if p < 1.0e-6 {
            // On the polar axis: geodetic latitude is ±90 by definition.
            let lat = if z < 0.0 { -90.0 } else { 90.0 };
            return (lon.to_degrees(), lat, z.abs() - self.b);
        }
        let e2 = self.e2();
        let mut lat = z.atan2(p * (1.0 - e2));
        for _ in 0..12 {
            let sin = lat.sin();
            let n = self.a / (1.0 - e2 * sin * sin).sqrt();
            let cos = lat.cos();
            let h = p * cos + z * sin - self.a * (1.0 - e2 * sin * sin).sqrt();
            let denominator = n + h;
            if denominator.abs() < 1.0e-9 {
                break;
            }
            let next = z.atan2(p * (1.0 - e2 * n / denominator));
            if (next - lat).abs() < 1.0e-13 {
                lat = next;
                break;
            }
            lat = next;
        }
        let sin = lat.sin();
        let cos = lat.cos();
        let h = p * cos + z * sin - self.a * (1.0 - e2 * sin * sin).sqrt();
        (lon.to_degrees(), lat.to_degrees(), h)
    }

    /// First intersection of a ray with the ellipsoid, in ECEF metres.
    ///
    /// `origin` is an ECEF point and `direction` any non-zero vector (it does
    /// not need to be normalized; the returned point is `origin + t·direction`).
    /// Returns `None` when the ray misses the surface, and the exit point when
    /// the origin is already inside the ellipsoid.
    pub fn ray_intersection(self, origin: [f64; 3], direction: [f64; 3]) -> Option<[f64; 3]> {
        // Scale z by a/b to turn the biaxial ellipsoid into a sphere of radius a.
        let axis = self.a / self.b.max(1.0);
        let (ox, oy, oz) = (origin[0], origin[1], origin[2] * axis);
        let (dx, dy, dz) = (direction[0], direction[1], direction[2] * axis);
        let qa = dx * dx + dy * dy + dz * dz;
        if qa < 1.0e-30 {
            return None;
        }
        let qb = 2.0 * (ox * dx + oy * dy + oz * dz);
        let qc = ox * ox + oy * oy + oz * oz - self.a * self.a;
        let discriminant = qb * qb - 4.0 * qa * qc;
        if discriminant < 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        let near = (-qb - root) / (2.0 * qa);
        let far = (-qb + root) / (2.0 * qa);
        let t = if near > 0.0 {
            near
        } else if far > 0.0 {
            far
        } else {
            return None;
        };
        Some([
            origin[0] + direction[0] * t,
            origin[1] + direction[1] * t,
            origin[2] + direction[2] * t,
        ])
    }
}

/// A stable identifier for one WebMercator XYZ tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    pub const fn new(z: u32, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    pub fn count(z: u32) -> u32 {
        1u32 << z.min(30)
    }

    pub fn parent(self) -> Option<Self> {
        (self.z > 0).then(|| Self::new(self.z - 1, self.x / 2, self.y / 2))
    }
}

/// Geographic bounds of a tile, in degrees. `lat_min < lat_max`.
#[derive(Clone, Copy, Debug)]
pub struct TileBounds {
    pub lon_min: f64,
    pub lon_max: f64,
    pub lat_min: f64,
    pub lat_max: f64,
}

impl TileBounds {
    pub fn center(self) -> (f64, f64) {
        (
            (self.lon_min + self.lon_max) * 0.5,
            (self.lat_min + self.lat_max) * 0.5,
        )
    }
}

pub fn lonlat_to_plane(lon_deg: f64, lat_deg: f64) -> [f64; 2] {
    [
        EARTH_RADIUS * lon_deg * DEG_TO_RAD,
        EARTH_RADIUS * lat_deg * DEG_TO_RAD,
    ]
}

/// Inverse of the plane mapping, used by the 2D picking and drag code.
pub fn plane_to_lonlat(x: f64, y: f64) -> (f64, f64) {
    (x / EARTH_RADIUS * RAD_TO_DEG, y / EARTH_RADIUS * RAD_TO_DEG)
}

/// WGS84 geodetic coordinates to ECEF metres, in `f64`. Stage 2's globe mesh and
/// camera use this (via [`Ellipsoid`]) so the sphere is never sampled.
#[allow(dead_code)] // Public geodesy entry point; the mesh/camera use `Ellipsoid`.
pub fn geodetic_to_ecef(lon_deg: f64, lat_deg: f64, altitude: f64) -> [f64; 3] {
    Ellipsoid::WGS84.point(lon_deg, lat_deg, altitude)
}

/// Unit direction on the globe for a lon/lat pair. The globe mesh, the
/// ellipsoid normal, and the camera use this same convention (x = lon 0,
/// y = lon 90E, z = north).
pub fn lonlat_to_unit(lon_deg: f64, lat_deg: f64) -> [f64; 3] {
    let lon = lon_deg * DEG_TO_RAD;
    let lat = lat_deg * DEG_TO_RAD;
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}

pub fn lonlat_to_tile(lon_deg: f64, lat_deg: f64, z: u32) -> (u32, u32) {
    let n = TileId::count(z) as f64;
    let lon = lon_deg.clamp(-180.0, 180.0);
    let lat = lat_deg.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    let x = ((lon + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0) as u32;
    let y = ((1.0 - (lat * DEG_TO_RAD).tan().asinh() / std::f64::consts::PI) * 0.5 * n)
        .floor()
        .clamp(0.0, n - 1.0) as u32;
    (x, y)
}

/// Inverse WebMercator (tile Y coordinate -> latitude).
pub fn tile_y_to_lat(y: f64, z: u32) -> f64 {
    let n = TileId::count(z) as f64;
    let merc = std::f64::consts::PI * (1.0 - 2.0 * y / n);
    merc.sinh().atan().to_degrees()
}

pub fn tile_bounds(id: TileId) -> TileBounds {
    let n = TileId::count(id.z) as f64;
    let lon_min = id.x as f64 / n * 360.0 - 180.0;
    let lon_max = (id.x + 1) as f64 / n * 360.0 - 180.0;
    // Tile y grows southward, so y is the north edge.
    let lat_max = tile_y_to_lat(id.y as f64, id.z);
    let lat_min = tile_y_to_lat((id.y + 1) as f64, id.z);
    TileBounds {
        lon_min,
        lon_max,
        lat_min,
        lat_max,
    }
}

/// Latitude at a fractional tile row. `v` is the tile-local Mercator coordinate
/// (0 = north edge, 1 = south edge). The globe mesh grids are uniform in this
/// coordinate, which keeps the tile UV linear in the same parameter.
pub fn tile_lat_at(id: TileId, v: f64) -> f64 {
    tile_y_to_lat(id.y as f64 + v, id.z)
}

/// UV of a lon/lat point inside the equirectangular world layer (layer 0),
/// `v` measured from the north edge.
pub fn lonlat_to_world_uv(lon_deg: f64, lat_deg: f64) -> [f32; 2] {
    [
        ((lon_deg + 180.0) / 360.0) as f32,
        ((90.0 - lat_deg) / 180.0) as f32,
    ]
}

/// UV rectangle of a tile in the equirectangular layer-0 world map.
/// Returns `(u0, v0, u1, v1)` with `v0` north.
pub fn tile_uv_on_world(id: TileId) -> [f32; 4] {
    let b = tile_bounds(id);
    [
        ((b.lon_min + 180.0) / 360.0) as f32,
        ((90.0 - b.lat_max) / 180.0) as f32,
        ((b.lon_max + 180.0) / 360.0) as f32,
        ((90.0 - b.lat_min) / 180.0) as f32,
    ]
}

/// Visible longitude/latitude rectangle for a nadir camera.
#[derive(Clone, Copy, Debug)]
pub struct GeoRect {
    pub lon_min: f64,
    pub lon_max: f64,
    pub lat_min: f64,
    pub lat_max: f64,
}

impl GeoRect {
    pub fn from_center_extent(
        center_lon: f64,
        center_lat: f64,
        half_x_m: f64,
        half_y_m: f64,
    ) -> Self {
        let half_lon = half_x_m / EARTH_RADIUS * RAD_TO_DEG;
        let half_lat = half_y_m / EARTH_RADIUS * RAD_TO_DEG;
        Self {
            lon_min: (center_lon - half_lon).max(-180.0),
            lon_max: (center_lon + half_lon).min(180.0),
            lat_min: (center_lat - half_lat).max(-MAX_MERCATOR_LAT),
            lat_max: (center_lat + half_lat).min(MAX_MERCATOR_LAT),
        }
    }
}

/// Target WebMercator zoom for a nadir perspective camera. A 512px layer is
/// treated as the natural screen size of a tile; clamp keeps the selection
/// bounded for a 64-layer atlas.
pub fn zoom_for_height(height_m: f64, viewport_height_px: f64, fov_y_deg: f64) -> u32 {
    let half_fov = (fov_y_deg * DEG_TO_RAD * 0.5).tan();
    let meters_per_pixel = 2.0 * height_m * half_fov / viewport_height_px.max(1.0);
    let target_tile_m = 512.0 * meters_per_pixel;
    let z = (EARTH_CIRCUMFERENCE / target_tile_m.max(1.0)).log2();
    z.round().clamp(1.0, 19.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_roundtrip_at_equator() {
        let (x, y) = lonlat_to_tile(0.0, 0.0, 4);
        assert_eq!((x, y), (8, 8));
        let b = tile_bounds(TileId::new(4, 8, 8));
        assert!((b.lon_min - 0.0).abs() < 1e-9);
        assert!((b.lat_max - 0.0).abs() < 1e-9);
        assert!(b.lat_min < 0.0);
    }

    #[test]
    fn ecef_and_unit_agree_in_direction() {
        // On the reference sphere the geocentric and geodetic directions are the
        // same, so the ECEF direction equals the lon/lat unit vector exactly.
        let sphere = Ellipsoid::SPHERE;
        let e = sphere.surface(105.0, 35.0);
        let u = lonlat_to_unit(105.0, 35.0);
        let len = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
        for axis in 0..3 {
            assert!((e[axis] / len - u[axis]).abs() < 1e-12);
        }
        // On WGS84 the geodetic normal differs from the geocentric direction by
        // up to ~0.19 degrees (the flattening); the direction still agrees.
        let e = geodetic_to_ecef(105.0, 35.0, 0.0);
        let len = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
        let dot = (e[0] / len) * u[0] + (e[1] / len) * u[1] + (e[2] / len) * u[2];
        assert!(dot > 0.999_99, "dot {dot}");
    }

    #[test]
    fn ellipsoid_axes_match_wgs84() {
        let e = Ellipsoid::WGS84;
        let equatorial = e.surface(0.0, 0.0);
        assert!((equatorial[0] - 6_378_137.0).abs() < 1e-6);
        assert!(equatorial[2].abs() < 1e-6);
        let polar = e.surface(37.0, 90.0);
        assert!(polar[0].abs() < 1e-6);
        assert!((polar[2] - 6_356_752.314_245_179).abs() < 1e-3);
        assert!((e.e2() - 0.006_694_379_990_14).abs() < 1e-12);
        // The sphere variant collapses to the stage-1 radius everywhere.
        let s = Ellipsoid::SPHERE;
        for lat in [-90.0, -45.0, 0.0, 45.0, 90.0] {
            let p = s.surface(37.0, lat);
            let radius = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((radius - EARTH_RADIUS).abs() < 1e-6);
        }
    }

    #[test]
    fn geodetic_normal_is_unit_and_matches_surface() {
        let e = Ellipsoid::WGS84;
        for lat in [-80.0, -30.0, 0.0, 23.5, 70.0, 89.0] {
            let n = e.normal(42.0, lat);
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-12);
            // The normal is parallel-ish to the radial direction (WGS84 differs
            // by at most the flattening, ~0.19 degrees).
            let p = e.surface(42.0, lat);
            let rl = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            let dot = (p[0] * n[0] + p[1] * n[1] + p[2] * n[2]) / rl;
            assert!(dot > 0.999_99, "lat {lat} dot {dot}");
        }
    }

    #[test]
    fn tile_lat_at_spans_tile_bounds() {
        for z in 1..8 {
            for y in [0, 1, (1 << z) / 2, (1 << z) - 1] {
                let id = TileId::new(z, 3.min((1 << z) - 1), y);
                let b = tile_bounds(id);
                let north = tile_lat_at(id, 0.0);
                let south = tile_lat_at(id, 1.0);
                assert!((north - b.lat_max).abs() < 1e-9);
                assert!((south - b.lat_min).abs() < 1e-9);
                // Fractional rows are monotonically southerly.
                assert!(tile_lat_at(id, 0.25) > tile_lat_at(id, 0.75));
            }
        }
    }

    #[test]
    fn world_uv_maps_the_full_range() {
        let top_left = lonlat_to_world_uv(-180.0, 90.0);
        assert!((top_left[0] - 0.0).abs() < 1e-6);
        assert!((top_left[1] - 0.0).abs() < 1e-6);
        let bottom_right = lonlat_to_world_uv(180.0, -90.0);
        assert!((bottom_right[0] - 1.0).abs() < 1e-6);
        assert!((bottom_right[1] - 1.0).abs() < 1e-6);
        let uv = lonlat_to_world_uv(0.0, 0.0);
        assert!((uv[0] - 0.5).abs() < 1e-6);
        assert!((uv[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn world_uv_is_inside_unit_square() {
        for z in 1..6 {
            for x in 0..(1 << z) {
                let uv = tile_uv_on_world(TileId::new(z, x, z / 2));
                assert!(uv[0] >= 0.0 && uv[2] <= 1.0);
                assert!(uv[1] >= 0.0 && uv[3] <= 1.0);
            }
        }
    }

    #[test]
    fn geodetic_roundtrip_matches_ecef() {
        for ellipsoid in [Ellipsoid::WGS84, Ellipsoid::SPHERE] {
            for lon in [-179.5, -90.0, 0.0, 37.25, 178.0] {
                for lat in [-89.9, -75.0, -33.3, 0.0, 27.5, 66.6, 89.9] {
                    for altitude in [0.0, 420.0, 25_000.0, 900_000.0] {
                        let ecef = ellipsoid.point(lon, lat, altitude);
                        let (lon2, lat2, alt2) = ellipsoid.to_geodetic(ecef);
                        let lon_error = (lon2 - lon).abs().min(360.0 - (lon2 - lon).abs());
                        assert!(lon_error < 1.0e-9, "lon {lon} -> {lon2}");
                        assert!((lat2 - lat).abs() < 1.0e-9, "lat {lat} -> {lat2}");
                        assert!(
                            (alt2 - altitude).abs() < 1.0e-3,
                            "alt {altitude} -> {alt2} at {lat}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ray_ellipsoid_hits_the_near_surface() {
        let ellipsoid = Ellipsoid::WGS84;
        let camera = [0.0, 0.0, 20_000_000.0];
        // Straight down onto the north pole.
        let hit = ellipsoid
            .ray_intersection(camera, [0.0, 0.0, -1.0])
            .expect("nadir ray must hit");
        assert!((hit[2] - ellipsoid.b).abs() < 1.0e-3, "hit {hit:?}");
        // Oblique ray towards 110E 25N: the hit point must sit on the surface
        // and report back the lon/lat we aimed at.
        let target = ellipsoid.point(110.0, 25.0, 0.0);
        let direction = [
            target[0] - camera[0],
            target[1] - camera[1],
            target[2] - camera[2],
        ];
        let hit = ellipsoid
            .ray_intersection(camera, direction)
            .expect("oblique ray must hit");
        for axis in 0..3 {
            assert!((hit[axis] - target[axis]).abs() < 1.0e-3, "hit {hit:?}");
        }
        // A ray pointing away from the Earth misses.
        assert!(
            ellipsoid
                .ray_intersection(camera, [0.0, 0.0, 1.0])
                .is_none()
        );
    }
}
