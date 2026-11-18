//! Offline, deterministic world map generation.
//!
//! The land/border masks are committed PNGs generated from Natural Earth 1:110m
//! admin-0 country polygons by `tools/gen_masks.py`. Keeping masks as assets
//! means the module has no network or build-time data dependency, while the 2D
//! fallback and the 3D globe share exactly the same coastline.
//!
//! The procedural tile painter adds deterministic terrain color, coast lines
//! and political borders. It draws no graticule or tile border, so the fallback
//! map is free of grid lines. It is a legible fallback, not a basemap.

use std::sync::OnceLock;

use image::{ImageBuffer, Rgba, RgbaImage};

use crate::math::geo::{MAX_MERCATOR_LAT, TileId, tile_bounds};

const LAND_MASK_PNG: &[u8] = include_bytes!("../../assets/land_mask.png");
const BORDERS_MASK_PNG: &[u8] = include_bytes!("../../assets/borders_mask.png");

struct Mask {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Mask {
    fn decode(bytes: &[u8]) -> Self {
        let image = image::load_from_memory(bytes)
            .expect("embedded ew_gis land mask is a valid PNG")
            .to_luma8();
        let (width, height) = image.dimensions();
        Self {
            width,
            height,
            data: image.into_raw(),
        }
    }

    fn sample(&self, lon: f64, lat: f64) -> f32 {
        let fx = ((lon + 180.0) / 360.0 * (self.width.saturating_sub(1)) as f64)
            .clamp(0.0, (self.width - 1) as f64);
        let fy = ((90.0 - lat) / 180.0 * (self.height.saturating_sub(1)) as f64)
            .clamp(0.0, (self.height - 1) as f64);
        let x0 = fx.floor() as u32;
        let y0 = fy.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = (fx - x0 as f64) as f32;
        let ty = (fy - y0 as f64) as f32;
        let idx =
            |x: u32, y: u32| -> f32 { self.data[(y * self.width + x) as usize] as f32 / 255.0 };
        let top = idx(x0, y0) + (idx(x1, y0) - idx(x0, y0)) * tx;
        let bottom = idx(x0, y1) + (idx(x1, y1) - idx(x0, y1)) * tx;
        top + (bottom - top) * ty
    }
}

static LAND_MASK: OnceLock<Mask> = OnceLock::new();
static BORDERS_MASK: OnceLock<Mask> = OnceLock::new();

fn land_mask() -> &'static Mask {
    LAND_MASK.get_or_init(|| Mask::decode(LAND_MASK_PNG))
}

fn borders_mask() -> &'static Mask {
    BORDERS_MASK.get_or_init(|| Mask::decode(BORDERS_MASK_PNG))
}

fn noise(lon: f64, lat: f64) -> f64 {
    let x = lon as f32;
    let y = lat as f32;
    let n = value_noise(x * 0.18, y * 0.18) * 0.55
        + value_noise(x * 0.53 + 11.0, y * 0.53 - 4.0) * 0.30
        + value_noise(x * 1.31 - 7.0, y * 1.31 + 19.0) * 0.15;
    n as f64
}

fn hash2(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65_535.0
}

fn value_noise(x: f32, y: f32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let xf = x - xi;
    let yf = y - yi;
    let u = xf * xf * (3.0 - 2.0 * xf);
    let v = yf * yf * (3.0 - 2.0 * yf);
    let x0 = xi as i32;
    let y0 = yi as i32;
    let a = hash2(x0, y0);
    let b = hash2(x0 + 1, y0);
    let c = hash2(x0, y0 + 1);
    let d = hash2(x0 + 1, y0 + 1);
    let top = a + (b - a) * u;
    let bottom = c + (d - c) * u;
    top + (bottom - top) * v
}

fn blend(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Color at a geographic point. `pixel_deg` is the angular size of one output
/// pixel, used for resolution-independent coast lines and graticules.
fn color_at(lon: f64, lat: f64, pixel_deg: f64) -> [u8; 4] {
    let land = land_mask();
    let is_land = land.sample(lon, lat) > 0.5;
    let n = noise(lon, lat);
    let mut rgb = if is_land {
        let cold = lat.abs() > 64.0;
        let terrain = noise(lon * 1.35 + 17.0, lat * 1.35 - 9.0);
        let mut c = if cold {
            [0.80, 0.82, 0.83]
        } else if lat.abs() < 13.0 {
            [0.14, 0.37, 0.16]
        } else if terrain > 0.62 {
            [0.55, 0.47, 0.30]
        } else if terrain > 0.54 {
            [0.38, 0.44, 0.25]
        } else {
            [0.22, 0.40, 0.19]
        };
        c = blend(c, [0.35, 0.43, 0.24], (n * 0.30) as f32);

        // Coast line: sample a small ring around the pixel.
        let eps = pixel_deg.max(0.025);
        let mut coast = false;
        for (dx, dy) in [
            (-eps, 0.0),
            (eps, 0.0),
            (0.0, -eps),
            (0.0, eps),
            (eps * 0.7, eps * 0.7),
            (-eps * 0.7, -eps * 0.7),
        ] {
            if land.sample(lon + dx, lat + dy) <= 0.5 {
                coast = true;
                break;
            }
        }
        if coast {
            c = blend(c, [0.72, 0.65, 0.43], 0.82);
        }

        // Political borders are intentionally subtle: they read as context,
        // not as the primary geometry.
        let border = borders_mask().sample(lon, lat);
        if border > 0.28 {
            c = blend(c, [0.63, 0.58, 0.40], (border * 0.50).min(0.42));
        }
        c
    } else {
        let depth = ((lat.abs() * 0.065).sin() * 0.5 + 0.5) as f32;
        let mut c = blend([0.026, 0.105, 0.205], [0.016, 0.060, 0.145], depth);
        let band = ((lon * 0.07 + lat * 0.11).sin() * 0.5 + 0.5) as f32;
        c = blend(c, [0.040, 0.185, 0.300], band * 0.28);
        c
    };

    // No graticule: the procedural fallback is a clean terrain/coast/border
    // basemap. (The old 15° grid was removed to keep the map free of grid lines
    // in screenshots and in the default offline view.)

    // Polar ice tint.
    if lat.abs() > 76.0 {
        rgb = blend(rgb, [0.90, 0.94, 0.98], 0.50);
    }

    [
        (rgb[0].clamp(0.0, 1.0) * 255.0) as u8,
        (rgb[1].clamp(0.0, 1.0) * 255.0) as u8,
        (rgb[2].clamp(0.0, 1.0) * 255.0) as u8,
        255,
    ]
}

/// 512x512 equirectangular layer-0 world map.
pub fn generate_world_map(size: u32) -> Vec<u8> {
    let img: RgbaImage = ImageBuffer::from_fn(size, size, |x, y| {
        let lon = -180.0 + (x as f64 + 0.5) / size as f64 * 360.0;
        let lat = 90.0 - (y as f64 + 0.5) / size as f64 * 180.0;
        Rgba(color_at(lon, lat, 360.0 / size as f64))
    });
    img.into_raw()
}

/// Procedural tile at the requested size (the provider resizes to the atlas's
/// 512x512 layer format).
pub fn generate_tile(id: TileId, size: u32) -> Vec<u8> {
    let bounds = tile_bounds(id);
    let n = (1u32 << id.z.min(30)) as f64;
    let lon_span = 360.0 / n;
    let lat_top = bounds.lat_max.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    let lat_bottom = bounds.lat_min.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    let pixel_lon = lon_span / size as f64;
    let pixel_lat = (lat_top - lat_bottom).abs() / size as f64;
    let pixel_deg = pixel_lon.max(pixel_lat);

    let img: RgbaImage = ImageBuffer::from_fn(size, size, |x, y| {
        let lon = bounds.lon_min + (x as f64 + 0.5) / size as f64 * lon_span;
        let lat = lat_top + (lat_bottom - lat_top) * (y as f64 + 0.5) / size as f64;
        Rgba(color_at(lon, lat, pixel_deg))
    });
    img.into_raw()
}

/// Reserved for stage 2's day/night or blue-marble layer; the current stage only
/// needs the static fallback above.
#[allow(dead_code)]
pub fn generate_world_map_rgba(size: u32) -> RgbaImage {
    let raw = generate_world_map(size);
    RgbaImage::from_raw(size, size, raw).expect("generated RGBA dimensions")
}
