//! Imagery providers, modelled on CesiumJS `ImageryProvider`s, plus decoding.
//!
//! [`ImageryKind`] is the imagery scheme; [`TileSource`] is the network policy
//! that decides the fallback chain:
//!
//! - `Online`: imagery URL → local directory → procedural.
//! - `Offline` / `Auto`: local directory → procedural (never blocks on a
//!   network; `Auto` keeps the stage-1 meaning "local if present").
//! - `Procedural`: always the procedural generator.
//!
//! `Single` has no URL template: one image is read (or downloaded) once at
//! startup, resized to a 512² layer-0 world map, and cropped per tile. If it
//! cannot be loaded the world layer silently falls back to the procedural map.
//!
//! ## URL rules
//!
//! | Kind | Rule |
//! | --- | --- |
//! | `Xyz` | `{z}{x}{y}{s}` substituted in the template; `{s}` cycles `a`/`b`/`c` |
//! | `Tms` | same, but `y` becomes `2^z - 1 - y` |
//! | `Arcgis` | `{base}/tile/{z}/{y}/{x}` (y before x) |
//! | `Wms` | `GetMap` with a WebMercator (`EPSG:3857`) `BBOX` in metres |
//! | `Single` | no per-tile URL; the startup image is cropped to the tile bbox |
//!
//! The WMS bbox uses `x_min = x/2^z·2πR - πR` and `y_max = πR - y/2^z·2πR`,
//! with `CRS` for WMS 1.3.0 and `SRS` for 1.1.1. Online tiles are opt-in so an
//! acceptance run never blocks on a proxy or certificate failure.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use image::RgbaImage;
use image::imageops::{self, FilterType};

use crate::gpu::texture::TILE_SIZE;
use crate::math::geo::{EARTH_RADIUS, TileId, tile_bounds};
use crate::tiles::procedural;

const USER_AGENT: &str = "ew_gis/0.1 (EW GIS imagery provider; offline fallback)";

pub const DEFAULT_XYZ_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
pub const DEFAULT_TMS_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
pub const DEFAULT_ARCGIS_URL: &str =
    "https://services.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer";
pub const DEFAULT_WMS_FORMAT: &str = "image/png";
pub const DEFAULT_WMS_VERSION: &str = "1.3.0";
pub const DEFAULT_WMS_CRS: &str = "EPSG:3857";

/// `π·R`, the half-width of the WebMercator world in metres.
pub const WEB_MERCATOR_HALF: f64 = std::f64::consts::PI * EARTH_RADIUS;

/// The three `{s}` subdomains cycled by `Xyz` templates.
const SUBDOMAINS: [&str; 3] = ["a", "b", "c"];

/// Startup single-image cap: a physically large source is downscaled once so a
/// 4K+ world map cannot cost hundreds of megabytes for the whole session.
const MAX_SINGLE_WIDTH: u32 = 4096;
const MAX_SINGLE_HEIGHT: u32 = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSource {
    Auto,
    Online,
    Offline,
    Procedural,
}

impl TileSource {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "default" => Some(Self::Auto),
            "online" | "http" | "osm" => Some(Self::Online),
            "offline" | "local" => Some(Self::Offline),
            "procedural" | "proc" | "none" => Some(Self::Procedural),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto(local→procedural)",
            Self::Online => "online(imagery→local→procedural)",
            Self::Offline => "offline(local→procedural)",
            Self::Procedural => "procedural",
        }
    }
}

/// The imagery scheme, the CesiumJS `ImageryProvider` equivalent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageryKind {
    /// Standard XYZ (OSM-style) URL template: `{z}`, `{x}`, `{y}`, `{s}`.
    Xyz { url_template: String },
    /// XYZ with the tile-row axis flipped: `{z}/{x}/{2^z-1-y}`.
    Tms { url_template: String },
    /// OGC WMS `GetMap` request built for a WebMercator tile bbox.
    Wms {
        base_url: String,
        layers: String,
        styles: String,
        format: String,
        version: String,
        crs: String,
    },
    /// ArcGIS REST tile endpoint: `{base}/tile/{z}/{y}/{x}`.
    Arcgis { base_url: String },
    /// One world image used as the layer-0 base map and cropped per tile.
    Single { path_or_url: String },
}

impl ImageryKind {
    pub fn tag(&self) -> ImageryTag {
        match self {
            Self::Xyz { .. } => ImageryTag::Xyz,
            Self::Tms { .. } => ImageryTag::Tms,
            Self::Wms { .. } => ImageryTag::Wms,
            Self::Arcgis { .. } => ImageryTag::Arcgis,
            Self::Single { .. } => ImageryTag::Single,
        }
    }

    pub fn label(&self) -> &'static str {
        self.tag().label()
    }

    /// Per-tile URL, or `None` for `Single` (which crops a startup image).
    pub fn url_for(&self, id: TileId) -> Option<String> {
        match self {
            Self::Xyz { url_template } => Some(xyz_url(url_template, id)),
            Self::Tms { url_template } => Some(tms_url(url_template, id)),
            Self::Wms {
                base_url,
                layers,
                styles,
                format,
                version,
                crs,
            } => Some(wms_url(base_url, layers, styles, format, version, crs, id)),
            Self::Arcgis { base_url } => Some(arcgis_url(base_url, id)),
            Self::Single { .. } => None,
        }
    }

    fn default_for(tag: ImageryTag) -> Self {
        match tag {
            ImageryTag::Xyz | ImageryTag::Local => Self::Xyz {
                url_template: DEFAULT_XYZ_URL.to_string(),
            },
            ImageryTag::Tms => Self::Tms {
                url_template: DEFAULT_TMS_URL.to_string(),
            },
            ImageryTag::Wms => Self::Wms {
                base_url: String::new(),
                layers: String::new(),
                styles: String::new(),
                format: DEFAULT_WMS_FORMAT.to_string(),
                version: DEFAULT_WMS_VERSION.to_string(),
                crs: DEFAULT_WMS_CRS.to_string(),
            },
            ImageryTag::Arcgis => Self::Arcgis {
                base_url: DEFAULT_ARCGIS_URL.to_string(),
            },
            ImageryTag::Single => Self::Single {
                path_or_url: String::new(),
            },
        }
    }

    /// Reject configurations that could only ever fail every tile request.
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Xyz { url_template } | Self::Tms { url_template } => {
                if url_template.trim().is_empty() {
                    Err("empty tile URL template".to_string())
                } else {
                    Ok(())
                }
            }
            Self::Wms {
                base_url, layers, ..
            } => {
                if base_url.trim().is_empty() {
                    Err("empty WMS base URL (--wms-url)".to_string())
                } else if layers.trim().is_empty() {
                    Err("empty WMS layer list (--wms-layers)".to_string())
                } else {
                    Ok(())
                }
            }
            Self::Arcgis { base_url } => {
                if base_url.trim().is_empty() {
                    Err("empty ArcGIS base URL (--arcgis-url)".to_string())
                } else {
                    Ok(())
                }
            }
            Self::Single { path_or_url } => {
                if path_or_url.trim().is_empty() {
                    Err("empty single-image path (--single-image)".to_string())
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// CLI/env spelling of an imagery scheme (`--imagery`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageryTag {
    Xyz,
    Tms,
    Wms,
    Arcgis,
    Single,
    /// Not an imagery provider: local directory tiles only.
    Local,
}

impl ImageryTag {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "xyz" | "osm" | "http" => Some(Self::Xyz),
            "tms" => Some(Self::Tms),
            "wms" => Some(Self::Wms),
            "arcgis" | "esri" | "arcgis-rest" => Some(Self::Arcgis),
            "single" | "image" | "single-image" => Some(Self::Single),
            "local" | "offline" | "dir" => Some(Self::Local),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Xyz => "xyz",
            Self::Tms => "tms",
            Self::Wms => "wms",
            Self::Arcgis => "arcgis",
            Self::Single => "single",
            Self::Local => "local",
        }
    }
}

/// URL/kind overrides collected from the command line (env goes through the
/// same type, so both paths share one precedence rule: last writer wins).
#[derive(Clone, Debug, Default)]
pub struct ImageryOptions {
    pub kind: Option<ImageryTag>,
    pub tile_url: Option<String>,
    pub tms_url: Option<String>,
    pub wms_url: Option<String>,
    pub wms_layers: Option<String>,
    pub wms_styles: Option<String>,
    pub wms_format: Option<String>,
    pub wms_version: Option<String>,
    pub wms_crs: Option<String>,
    pub arcgis_url: Option<String>,
    pub single_image: Option<String>,
}

impl ImageryOptions {
    pub fn from_env() -> Self {
        let read = |name: &str| std::env::var(name).ok();
        let kind = read("EW_GIS_IMAGERY").and_then(|value| match ImageryTag::parse(&value) {
            Some(tag) => Some(tag),
            None => {
                log::warn!("unknown EW_GIS_IMAGERY={value:?}; keeping the current imagery");
                None
            }
        });
        Self {
            kind,
            tile_url: read("EW_GIS_TILE_URL"),
            tms_url: read("EW_GIS_TMS_URL"),
            wms_url: read("EW_GIS_WMS_URL"),
            wms_layers: read("EW_GIS_WMS_LAYERS"),
            wms_styles: read("EW_GIS_WMS_STYLES"),
            wms_format: read("EW_GIS_WMS_FORMAT"),
            wms_version: read("EW_GIS_WMS_VERSION"),
            wms_crs: read("EW_GIS_WMS_CRS"),
            arcgis_url: read("EW_GIS_ARCGIS_URL"),
            single_image: read("EW_GIS_SINGLE_IMAGE"),
        }
    }

    /// Apply these overrides onto a provider config in place.
    pub fn apply(&self, config: &mut ProviderConfig) {
        if let Some(tag) = self.kind {
            match tag {
                // `local` is a tile-source policy, not an imagery provider.
                ImageryTag::Local => {
                    config.source = TileSource::Offline;
                    return;
                }
                other => {
                    if other != config.imagery.tag() {
                        config.imagery = ImageryKind::default_for(other);
                    }
                }
            }
        }

        let imagery = match &config.imagery {
            ImageryKind::Xyz { url_template } => ImageryKind::Xyz {
                url_template: pick(&self.tile_url).unwrap_or_else(|| url_template.clone()),
            },
            ImageryKind::Tms { url_template } => ImageryKind::Tms {
                url_template: pick(&self.tms_url).unwrap_or_else(|| url_template.clone()),
            },
            ImageryKind::Wms {
                base_url,
                layers,
                styles,
                format,
                version,
                crs,
            } => ImageryKind::Wms {
                base_url: pick(&self.wms_url).unwrap_or_else(|| base_url.clone()),
                layers: pick(&self.wms_layers).unwrap_or_else(|| layers.clone()),
                styles: pick(&self.wms_styles).unwrap_or_else(|| styles.clone()),
                format: pick(&self.wms_format).unwrap_or_else(|| format.clone()),
                version: pick(&self.wms_version).unwrap_or_else(|| version.clone()),
                crs: pick(&self.wms_crs).unwrap_or_else(|| crs.clone()),
            },
            ImageryKind::Arcgis { base_url } => ImageryKind::Arcgis {
                base_url: pick(&self.arcgis_url).unwrap_or_else(|| base_url.clone()),
            },
            ImageryKind::Single { path_or_url } => ImageryKind::Single {
                path_or_url: pick(&self.single_image).unwrap_or_else(|| path_or_url.clone()),
            },
        };
        config.imagery = imagery;
    }
}

fn pick(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub source: TileSource,
    pub imagery: ImageryKind,
    pub tile_dir: PathBuf,
    pub template: String,
    pub timeout: Duration,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Self {
            source: TileSource::Auto,
            imagery: ImageryKind::default_for(ImageryTag::Xyz),
            tile_dir: manifest.join("assets/tiles"),
            template: "{z}/{x}/{y}.png".to_string(),
            timeout: Duration::from_secs(7),
        }
    }
}

impl ProviderConfig {
    /// Build from the environment. Overrides are merged but deliberately not
    /// validated yet: the caller may still apply CLI options (which can fill in
    /// a URL or layer list) and must call [`Self::resolve_imagery`] last.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(value) = std::env::var("EW_GIS_TILE_SOURCE") {
            if let Some(source) = TileSource::parse(&value) {
                config.source = source;
            } else {
                log::warn!("unknown EW_GIS_TILE_SOURCE={value:?}; keeping auto");
            }
        }
        if let Ok(dir) = std::env::var("EW_GIS_TILE_DIR") {
            config.tile_dir = PathBuf::from(dir);
        }
        if let Ok(template) = std::env::var("EW_GIS_TILE_TEMPLATE") {
            config.template = template;
        }
        if let Ok(timeout) = std::env::var("EW_GIS_TILE_TIMEOUT_MS") {
            if let Ok(ms) = timeout.parse::<u64>() {
                config.timeout = Duration::from_millis(ms.clamp(250, 60_000));
            }
        }
        ImageryOptions::from_env().apply(&mut config);
        config
    }

    pub fn with_source(mut self, source: TileSource) -> Self {
        self.source = source;
        self
    }

    /// Final validation of the imagery kind: a configuration that could never
    /// fetch a tile (empty WMS URL/layer list, empty single-image path, empty
    /// template) falls back to the default XYZ provider instead of failing
    /// every request. Call this after the last override has been applied.
    pub fn resolve_imagery(&mut self) {
        if let Err(reason) = self.imagery.validate() {
            log::warn!("{reason}; falling back to XYZ ({DEFAULT_XYZ_URL})");
            self.imagery = ImageryKind::Xyz {
                url_template: DEFAULT_XYZ_URL.to_string(),
            };
        }
    }

    /// One-line description for the startup log.
    pub fn label(&self) -> String {
        format!("{} ({})", self.imagery.label(), self.source.label())
    }

    fn local_path(&self, id: TileId) -> PathBuf {
        let relative = self
            .template
            .replace("{z}", &id.z.to_string())
            .replace("{x}", &id.x.to_string())
            .replace("{y}", &id.y.to_string())
            .replace("{s}", "a");
        self.tile_dir.join(relative)
    }
}

/// Substitute `{z}`, `{x}`, `{y}` and `{s}` (cycling `a`/`b`/`c`).
pub fn xyz_url(template: &str, id: TileId) -> String {
    expand_template(template, id.z, id.x, id.y)
}

/// TMS numbering: the row is counted from the south edge.
pub fn tms_y(z: u32, y: u32) -> u32 {
    let count = TileId::count(z);
    count.saturating_sub(1).saturating_sub(y)
}

pub fn tms_url(template: &str, id: TileId) -> String {
    expand_template(template, id.z, id.x, tms_y(id.z, id.y))
}

fn expand_template(template: &str, z: u32, x: u32, y: u32) -> String {
    let subdomain = SUBDOMAINS[(x as usize + y as usize) % SUBDOMAINS.len()];
    template
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
        .replace("{s}", subdomain)
}

/// ArcGIS REST tiles are addressed `{z}/{y}/{x}` (row before column).
pub fn arcgis_url(base_url: &str, id: TileId) -> String {
    format!(
        "{}/tile/{}/{}/{}",
        base_url.trim_end_matches('/'),
        id.z,
        id.y,
        id.x
    )
}

/// WebMercator bounding box of one tile, in metres: `[minx, miny, maxx, maxy]`.
pub fn wms_bbox(id: TileId) -> [f64; 4] {
    let count = TileId::count(id.z) as f64;
    let span = 2.0 * WEB_MERCATOR_HALF / count;
    let min_x = id.x as f64 * span - WEB_MERCATOR_HALF;
    let max_x = (id.x as f64 + 1.0) * span - WEB_MERCATOR_HALF;
    let max_y = WEB_MERCATOR_HALF - id.y as f64 * span;
    let min_y = WEB_MERCATOR_HALF - (id.y as f64 + 1.0) * span;
    [min_x, min_y, max_x, max_y]
}

/// Build a WMS 1.3.0/1.1.1 `GetMap` URL for one WebMercator tile.
///
/// The axis parameter follows the version: `CRS` for 1.3.0, `SRS` before it.
#[allow(clippy::too_many_arguments)]
pub fn wms_url(
    base_url: &str,
    layers: &str,
    styles: &str,
    format: &str,
    version: &str,
    crs: &str,
    id: TileId,
) -> String {
    let bbox = wms_bbox(id);
    let version = if version.trim().is_empty() {
        DEFAULT_WMS_VERSION
    } else {
        version.trim()
    };
    let crs = if crs.trim().is_empty() {
        DEFAULT_WMS_CRS
    } else {
        crs.trim()
    };
    let format = if format.trim().is_empty() {
        DEFAULT_WMS_FORMAT
    } else {
        format.trim()
    };
    let axis = if version.starts_with("1.3") {
        "CRS"
    } else {
        "SRS"
    };
    let separator = if base_url.contains('?') { '&' } else { '?' };
    format!(
        "{base}{separator}SERVICE=WMS&VERSION={version}&REQUEST=GetMap&LAYERS={layers}\
         &STYLES={styles}&FORMAT={format}&TRANSPARENT=TRUE&{axis}={crs}\
         &WIDTH={TILE_SIZE}&HEIGHT={TILE_SIZE}\
         &BBOX={min_x:.3},{min_y:.3},{max_x:.3},{max_y:.3}",
        base = base_url.trim(),
        layers = encode_query_value(layers),
        styles = encode_query_value(styles),
        format = encode_query_value(format),
        crs = encode_query_value(crs),
        min_x = bbox[0],
        min_y = bbox[1],
        max_x = bbox[2],
        max_y = bbox[3],
    )
}

/// Minimal query-value encoding: enough for layer names with spaces and for
/// separators that would otherwise split the query string.
fn encode_query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.trim().chars() {
        match character {
            ' ' => encoded.push_str("%20"),
            '%' => encoded.push_str("%25"),
            '&' => encoded.push_str("%26"),
            '#' => encoded.push_str("%23"),
            '+' => encoded.push_str("%2B"),
            '?' => encoded.push_str("%3F"),
            other => encoded.push(other),
        }
    }
    encoded
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileOrigin {
    Online,
    Local,
    Procedural,
}

impl TileOrigin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Local => "local",
            Self::Procedural => "procedural",
        }
    }
}

pub struct TileImage {
    pub rgba: Vec<u8>,
    pub origin: TileOrigin,
}

/// The decoded `Single` source, kept for per-tile cropping.
struct SingleBase {
    image: RgbaImage,
    world: Vec<u8>,
    origin: TileOrigin,
}

pub struct Provider {
    config: ProviderConfig,
    agent: ureq::Agent,
    single: Option<SingleBase>,
}

impl Provider {
    pub fn new(config: ProviderConfig) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(config.timeout)
            .timeout_read(config.timeout)
            .timeout_write(config.timeout)
            .user_agent(USER_AGENT)
            .build();
        let single = match &config.imagery {
            ImageryKind::Single { path_or_url } if !path_or_url.trim().is_empty() => {
                match load_single_source(path_or_url, &agent) {
                    Ok((image, origin)) => {
                        let world =
                            imageops::resize(&image, TILE_SIZE, TILE_SIZE, FilterType::Triangle)
                                .into_raw();
                        log::info!(
                            "single imagery {} loaded ({}x{}), world base {}x{}",
                            path_or_url,
                            image.width(),
                            image.height(),
                            TILE_SIZE,
                            TILE_SIZE
                        );
                        Some(SingleBase {
                            image,
                            world,
                            origin,
                        })
                    }
                    Err(error) => {
                        log::warn!(
                            "single imagery {path_or_url} failed: {error}; \
                             the world base falls back to the procedural map"
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        Self {
            config,
            agent,
            single,
        }
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }

    /// Startup world image (layer 0) when the provider supplies one. `None`
    /// means the caller should use the procedural world map.
    pub fn world_image(&self) -> Option<Vec<u8>> {
        self.single.as_ref().map(|base| base.world.clone())
    }

    pub fn load(&self, id: TileId) -> TileImage {
        match self.config.source {
            TileSource::Procedural => self.procedural(id),
            TileSource::Offline | TileSource::Auto => match self.single_tile(id) {
                Some(image) => image,
                None => self.local_then_procedural(id),
            },
            TileSource::Online => match self.config.imagery.url_for(id) {
                Some(url) => match self.fetch_url(&url, id) {
                    Some(image) => image,
                    None => self.local_then_procedural(id),
                },
                None => self
                    .single_tile(id)
                    .unwrap_or_else(|| self.local_then_procedural(id)),
            },
        }
    }

    fn fetch_url(&self, url: &str, id: TileId) -> Option<TileImage> {
        let result = (|| -> Result<Vec<u8>, String> {
            let response = self
                .agent
                .get(url)
                .set("Accept", "image/png,image/jpeg,image/webp,image/*;q=0.8")
                .call()
                .map_err(|error| error.to_string())?;
            let status = response.status();
            if !(200..300).contains(&status) {
                return Err(format!("HTTP {status}"));
            }
            let mut bytes = Vec::with_capacity(64 * 1024);
            response
                .into_reader()
                .take(8 * 1024 * 1024)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.is_empty() {
                return Err("empty response".to_string());
            }
            Ok(bytes)
        })();

        match result {
            Ok(bytes) => match decode_resize(&bytes) {
                Ok(rgba) => {
                    log::debug!("online tile {:?} loaded ({} bytes)", id, bytes.len());
                    Some(TileImage {
                        rgba,
                        origin: TileOrigin::Online,
                    })
                }
                Err(error) => {
                    log::warn!("online tile {:?} decode failed: {error}", id);
                    None
                }
            },
            Err(error) => {
                log::warn!("online tile {:?} failed ({url}): {error}", id);
                None
            }
        }
    }

    /// Crop the startup image to the tile's geographic bounds (equirectangular
    /// assumption, the same simplification Cesium's single-tile provider makes).
    fn single_tile(&self, id: TileId) -> Option<TileImage> {
        let base = self.single.as_ref()?;
        let bounds = tile_bounds(id);
        let (width, height) = base.image.dimensions();
        let width_f = width.max(1) as f64;
        let height_f = height.max(1) as f64;
        let u0 = ((bounds.lon_min + 180.0) / 360.0).clamp(0.0, 1.0);
        let u1 = ((bounds.lon_max + 180.0) / 360.0).clamp(0.0, 1.0);
        let v0 = ((90.0 - bounds.lat_max) / 180.0).clamp(0.0, 1.0);
        let v1 = ((90.0 - bounds.lat_min) / 180.0).clamp(0.0, 1.0);
        let x0 = (u0 * width_f).floor() as u32;
        let x1 = ((u1 * width_f).ceil() as u32).min(width);
        let y0 = (v0 * height_f).floor() as u32;
        let y1 = ((v1 * height_f).ceil() as u32).min(height);
        let crop_width = x1.saturating_sub(x0).max(1).min(width);
        let crop_height = y1.saturating_sub(y0).max(1).min(height);
        let x0 = x0.min(width - crop_width);
        let y0 = y0.min(height - crop_height);
        let crop = imageops::crop_imm(&base.image, x0, y0, crop_width, crop_height).to_image();
        let rgba = imageops::resize(&crop, TILE_SIZE, TILE_SIZE, FilterType::Triangle).into_raw();
        Some(TileImage {
            rgba,
            origin: base.origin,
        })
    }

    fn local_then_procedural(&self, id: TileId) -> TileImage {
        let path = self.config.local_path(id);
        if path.is_file() {
            match std::fs::read(&path) {
                Ok(bytes) => match decode_resize(&bytes) {
                    Ok(rgba) => {
                        return TileImage {
                            rgba,
                            origin: TileOrigin::Local,
                        };
                    }
                    Err(error) => {
                        log::warn!("local tile {} decode failed: {error}", path.display())
                    }
                },
                Err(error) => log::warn!("local tile {} read failed: {error}", path.display()),
            }
        }
        self.procedural(id)
    }

    fn procedural(&self, id: TileId) -> TileImage {
        // The worker produces 256px tiles (the stage-1 fallback spec) and the
        // provider resizes once to the atlas layer size.
        let raw = procedural::generate_tile(id, 256);
        let image = RgbaImage::from_raw(256, 256, raw).expect("procedural tile dimensions");
        let rgba = if TILE_SIZE != 256 {
            imageops::resize(&image, TILE_SIZE, TILE_SIZE, FilterType::Triangle).into_raw()
        } else {
            image.into_raw()
        };
        TileImage {
            rgba,
            origin: TileOrigin::Procedural,
        }
    }
}

/// Read (from disk) or download (over HTTP) the single world image.
fn load_single_source(
    source: &str,
    agent: &ureq::Agent,
) -> Result<(RgbaImage, TileOrigin), String> {
    let source = source.trim();
    if source.starts_with("http://") || source.starts_with("https://") {
        let bytes = fetch_bytes(agent, source)?;
        Ok((decode_capped(&bytes)?, TileOrigin::Online))
    } else {
        let path = Path::new(source);
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok((decode_capped(&bytes)?, TileOrigin::Local))
    }
}

fn fetch_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let response = agent
        .get(url)
        .set("Accept", "image/png,image/jpeg,image/webp,image/*;q=0.8")
        .call()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    let mut bytes = Vec::with_capacity(256 * 1024);
    response
        .into_reader()
        .take(32 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.is_empty() {
        return Err("empty response".to_string());
    }
    Ok(bytes)
}

fn decode_capped(bytes: &[u8]) -> Result<RgbaImage, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let (width, height) = image.dimensions();
    if width <= MAX_SINGLE_WIDTH && height <= MAX_SINGLE_HEIGHT {
        return Ok(image);
    }
    let scale = (MAX_SINGLE_WIDTH as f64 / width as f64)
        .min(MAX_SINGLE_HEIGHT as f64 / height as f64)
        .min(1.0);
    let scaled_width = ((width as f64 * scale).round() as u32).max(1);
    let scaled_height = ((height as f64 * scale).round() as u32).max(1);
    log::info!(
        "single imagery downscaled {width}x{height} -> {scaled_width}x{scaled_height} \
         (cap {MAX_SINGLE_WIDTH}x{MAX_SINGLE_HEIGHT})"
    );
    Ok(imageops::resize(
        &image,
        scaled_width,
        scaled_height,
        FilterType::Triangle,
    ))
}

fn decode_resize(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let rgba = image.to_rgba8();
    let resized = if rgba.width() == TILE_SIZE && rgba.height() == TILE_SIZE {
        rgba
    } else {
        imageops::resize(&rgba, TILE_SIZE, TILE_SIZE, FilterType::Triangle)
    };
    Ok(resized.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox_of(url: &str) -> [f64; 4] {
        let start = url.find("BBOX=").expect("BBOX in the WMS URL") + "BBOX=".len();
        let rest = &url[start..];
        let end = rest.find('&').unwrap_or(rest.len());
        let values: Vec<f64> = rest[..end]
            .split(',')
            .map(|value| value.parse().expect("numeric bbox bound"))
            .collect();
        assert_eq!(values.len(), 4, "WMS bbox has four bounds");
        [values[0], values[1], values[2], values[3]]
    }

    #[test]
    fn xyz_template_substitutes_axes_and_subdomain() {
        let id = TileId::new(2, 1, 3);
        let url = xyz_url("https://{s}.example.test/{z}/{x}/{y}.png", id);
        assert_eq!(url, "https://b.example.test/2/1/3.png");
        // No placeholder means a fixed URL (a single tile drawn everywhere).
        assert_eq!(
            xyz_url("https://example.test/world.png", id),
            "https://example.test/world.png"
        );
        // z=0 is legal and must not be mangled.
        assert_eq!(
            xyz_url("https://x/{z}/{x}/{y}.png", TileId::new(0, 0, 0)),
            "https://x/0/0/0.png"
        );
    }

    #[test]
    fn tms_flips_the_row_axis() {
        assert_eq!(tms_y(0, 0), 0);
        assert_eq!(tms_y(2, 0), 3);
        assert_eq!(tms_y(2, 3), 0);
        assert_eq!(tms_y(4, 1), 14);
        let id = TileId::new(2, 1, 0);
        assert_eq!(
            tms_url("https://tms.test/{z}/{x}/{y}.png", id),
            "https://tms.test/2/1/3.png"
        );
    }

    #[test]
    fn arcgis_addresses_y_before_x() {
        let id = TileId::new(3, 5, 2);
        assert_eq!(
            arcgis_url("https://example.test/MapServer/", id),
            "https://example.test/MapServer/tile/3/2/5"
        );
        assert_eq!(
            ImageryKind::Arcgis {
                base_url: "https://example.test/MapServer".to_string()
            }
            .url_for(id)
            .as_deref(),
            Some("https://example.test/MapServer/tile/3/2/5")
        );
    }

    #[test]
    fn wms_bbox_covers_the_web_mercator_world() {
        let world = wms_bbox(TileId::new(0, 0, 0));
        assert!((world[0] + WEB_MERCATOR_HALF).abs() < 1e-6);
        assert!((world[1] + WEB_MERCATOR_HALF).abs() < 1e-6);
        assert!((world[2] - WEB_MERCATOR_HALF).abs() < 1e-6);
        assert!((world[3] - WEB_MERCATOR_HALF).abs() < 1e-6);

        // z=1: the north-west tile is the top-left quadrant, y still upward.
        let nw = wms_bbox(TileId::new(1, 0, 0));
        assert!((nw[0] + WEB_MERCATOR_HALF).abs() < 1e-6);
        assert!((nw[1] - 0.0).abs() < 1e-6);
        assert!((nw[2] - 0.0).abs() < 1e-6);
        assert!((nw[3] - WEB_MERCATOR_HALF).abs() < 1e-6);

        // The south-east quadrant is y-flipped in tile space.
        let se = wms_bbox(TileId::new(1, 1, 1));
        assert!((se[0] - 0.0).abs() < 1e-6);
        assert!((se[1] + WEB_MERCATOR_HALF).abs() < 1e-6);
        assert!((se[2] - WEB_MERCATOR_HALF).abs() < 1e-6);
        assert!((se[3] - 0.0).abs() < 1e-6);

        // Every tile spans an eighth of the world at z=2 (2πR / 4).
        let tile = wms_bbox(TileId::new(2, 3, 2));
        assert!((tile[2] - tile[0] - WEB_MERCATOR_HALF * 0.5).abs() < 1e-6);
        assert!((tile[3] - tile[1] - WEB_MERCATOR_HALF * 0.5).abs() < 1e-6);
    }

    #[test]
    fn wms_url_uses_crs_for_130_and_srs_for_111() {
        let id = TileId::new(1, 0, 0);
        let url = wms_url(
            "https://example.test/wms",
            "workspace:layer",
            "default",
            "image/png",
            "1.3.0",
            "EPSG:3857",
            id,
        );
        assert!(
            url.starts_with("https://example.test/wms?SERVICE=WMS&"),
            "{url}"
        );
        assert!(url.contains("VERSION=1.3.0"));
        assert!(url.contains("REQUEST=GetMap"));
        assert!(url.contains("LAYERS=workspace:layer"));
        assert!(url.contains("STYLES=default"));
        assert!(url.contains("FORMAT=image/png"));
        assert!(url.contains("TRANSPARENT=TRUE"));
        assert!(url.contains("CRS=EPSG:3857"));
        assert!(!url.contains("SRS="));
        assert!(url.contains("WIDTH=512"));
        assert!(url.contains("HEIGHT=512"));
        let bbox = bbox_of(&url);
        assert!((bbox[0] + WEB_MERCATOR_HALF).abs() < 1e-3);
        assert!((bbox[1] - 0.0).abs() < 1e-3);
        assert!((bbox[2] - 0.0).abs() < 1e-3);
        assert!((bbox[3] - WEB_MERCATOR_HALF).abs() < 1e-3);

        let legacy = wms_url(
            "https://example.test/wms?map=global",
            "layer",
            "",
            "",
            "1.1.1",
            "",
            id,
        );
        assert!(legacy.contains("SRS=EPSG:3857"), "{legacy}");
        assert!(!legacy.contains("CRS="));
        // An existing query string is extended with `&`, not a second `?`.
        assert!(legacy.starts_with("https://example.test/wms?map=global&SERVICE=WMS"));
        // Empty format/crs fall back to the documented defaults.
        assert!(legacy.contains("FORMAT=image/png"));
    }

    #[test]
    fn wms_layer_names_are_query_encoded() {
        let url = wms_url(
            "https://example.test/wms",
            "a b",
            "",
            "image/png",
            "1.3.0",
            "EPSG:3857",
            TileId::new(0, 0, 0),
        );
        assert!(url.contains("LAYERS=a%20b"), "{url}");
    }

    fn options(kind: ImageryTag) -> ImageryOptions {
        ImageryOptions {
            kind: Some(kind),
            ..Default::default()
        }
    }

    #[test]
    fn cli_imagery_kind_switches_the_provider() {
        let mut config = ProviderConfig::default();
        assert_eq!(config.imagery.tag(), ImageryTag::Xyz);

        options(ImageryTag::Wms).apply(&mut config);
        // A WMS without URL/layers would fail every tile: resolution falls
        // back to XYZ, but not before the CLI had a chance to fill them in.
        assert_eq!(config.imagery.tag(), ImageryTag::Wms);
        config.resolve_imagery();
        assert_eq!(config.imagery.tag(), ImageryTag::Xyz);

        ImageryOptions {
            kind: Some(ImageryTag::Wms),
            wms_url: Some("https://example.test/wms".to_string()),
            wms_layers: Some("layer".to_string()),
            ..Default::default()
        }
        .apply(&mut config);
        match &config.imagery {
            ImageryKind::Wms {
                base_url,
                layers,
                version,
                crs,
                ..
            } => {
                assert_eq!(base_url, "https://example.test/wms");
                assert_eq!(layers, "layer");
                assert_eq!(version, DEFAULT_WMS_VERSION);
                assert_eq!(crs, DEFAULT_WMS_CRS);
            }
            other => panic!("expected WMS, got {other:?}"),
        }

        ImageryOptions {
            kind: Some(ImageryTag::Local),
            ..Default::default()
        }
        .apply(&mut config);
        assert_eq!(config.source, TileSource::Offline);
    }

    #[test]
    fn tile_url_override_keeps_the_kind() {
        let mut config = ProviderConfig::default();
        ImageryOptions {
            tile_url: Some("https://local.test/{z}/{x}/{y}.jpg".to_string()),
            ..Default::default()
        }
        .apply(&mut config);
        assert_eq!(
            config.imagery,
            ImageryKind::Xyz {
                url_template: "https://local.test/{z}/{x}/{y}.jpg".to_string()
            }
        );
        assert_eq!(config.label(), "xyz (auto(local→procedural))");
    }

    fn temp_image(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ew_gis_single_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        let pixels: Vec<u8> = (0..16 * 16)
            .flat_map(|index| [index as u8, 40, 200, 255])
            .collect();
        image::save_buffer(&path, &pixels, 16, 16, image::ColorType::Rgba8)
            .expect("write test png");
        path
    }

    #[test]
    fn single_imagery_supplies_the_world_layer_and_crops_tiles() {
        let path = temp_image("world.png");
        let config = ProviderConfig {
            source: TileSource::Offline,
            imagery: ImageryKind::Single {
                path_or_url: path.display().to_string(),
            },
            ..Default::default()
        };
        let provider = Provider::new(config);
        let world = provider.world_image().expect("single world image");
        assert_eq!(world.len(), (TILE_SIZE * TILE_SIZE * 4) as usize);
        assert!(world.chunks(4).all(|pixel| pixel[3] == 255));

        let tile = provider.load(TileId::new(0, 0, 0));
        assert_eq!(tile.rgba.len(), (TILE_SIZE * TILE_SIZE * 4) as usize);
        assert_eq!(tile.origin, TileOrigin::Local);

        // A coarse tile takes the crop path, not the procedural fallback.
        let coarse = provider.load(TileId::new(1, 0, 0));
        assert_eq!(coarse.origin, TileOrigin::Local);
        assert_eq!(coarse.rgba.len(), (TILE_SIZE * TILE_SIZE * 4) as usize);
    }

    #[test]
    fn missing_single_image_falls_back_to_procedural() {
        let config = ProviderConfig {
            source: TileSource::Offline,
            imagery: ImageryKind::Single {
                path_or_url: "target/definitely-does-not-exist.png".to_string(),
            },
            ..Default::default()
        };
        let provider = Provider::new(config);
        assert!(provider.world_image().is_none());
        let tile = provider.load(TileId::new(1, 0, 0));
        assert_eq!(tile.origin, TileOrigin::Procedural);
        assert_eq!(tile.rgba.len(), (TILE_SIZE * TILE_SIZE * 4) as usize);
    }

    #[test]
    fn invalid_imagery_configuration_falls_back_to_xyz() {
        let mut config = ProviderConfig::default();
        config.imagery = ImageryKind::Wms {
            base_url: String::new(),
            layers: String::new(),
            styles: String::new(),
            format: DEFAULT_WMS_FORMAT.to_string(),
            version: DEFAULT_WMS_VERSION.to_string(),
            crs: DEFAULT_WMS_CRS.to_string(),
        };
        ImageryOptions::default().apply(&mut config);
        config.resolve_imagery();
        assert_eq!(
            config.imagery,
            ImageryKind::Xyz {
                url_template: DEFAULT_XYZ_URL.to_string()
            }
        );
    }

    #[test]
    fn imagery_resolution_waits_for_every_override() {
        // Environment sets the kind but not the endpoint; the CLI supplies it
        // afterwards, so resolution must not have happened in between.
        let mut config = ProviderConfig::default();
        options(ImageryTag::Wms).apply(&mut config);
        ImageryOptions {
            wms_url: Some("https://example.test/wms".to_string()),
            wms_layers: Some("layer".to_string()),
            ..Default::default()
        }
        .apply(&mut config);
        config.resolve_imagery();
        assert_eq!(config.imagery.tag(), ImageryTag::Wms);

        // Without the CLI fill-in the same environment-only state falls back.
        let mut bare = ProviderConfig::default();
        options(ImageryTag::Single).apply(&mut bare);
        bare.resolve_imagery();
        assert_eq!(bare.imagery.tag(), ImageryTag::Xyz);
    }

    #[test]
    fn local_template_replaces_the_xyz_placeholders() {
        let config = ProviderConfig {
            template: "tiles/{z}/{x}/{y}.jpg".to_string(),
            tile_dir: PathBuf::from("root"),
            ..Default::default()
        };
        let path = config.local_path(TileId::new(3, 4, 5));
        assert!(path.ends_with("root/tiles/3/4/5.jpg"), "{path:?}");
    }
}
