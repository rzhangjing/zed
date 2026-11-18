//! Mapbox Vector Tile (MVT) decoding into the same authoring shapes the plot
//! scene already renders ([`DemoShapes`]), plus the tile source (HTTP template
//! or local directory).
//!
//! ## Decoding
//!
//! The tile protobuf is decoded by `geozero`'s `mvt` module (a `prost`-generated
//! `Tile`); the geometry command stream (ZigZag parameters, `MoveTo` / `LineTo` /
//! `ClosePath`) is read by `geozero`'s `GeomProcessor` reader, so this module
//! never re-implements the wire format. What it does own is the tile-local to
//! geographic mapping:
//!
//! ```text
//! lon = lon_min + x/extent * (lon_max - lon_min)
//! lat = lat_max - y/extent * (lat_max - lat_min)
//! ```
//!
//! with `extent` defaulting to 4096 and `y` growing south, per the MVT spec.
//! That is a linear interpolation of latitude across the tile; the exact
//! WebMercator inverse would be [`crate::math::geo::tile_lat_at`]. The linear
//! form is what the layer is specified with and is within a fraction of a degree
//! at the zooms this demo renders; it is a documented approximation, not a
//! rounding artefact.
//!
//! ## What is rendered
//!
//! * Point / MultiPoint -> one [`GeoPoint`] per position.
//! * LineString / MultiLineString -> one [`GeoPolyline`] per part.
//! * Polygon / MultiPolygon -> one [`GeoPolygon`] per *exterior* ring. Interior
//!   rings (holes) are counted and skipped, exactly like the GeoJSON and
//!   Shapefile importers: the plot fill has no hole support.
//! * Malformed command streams, unknown geometry types and empty geometries are
//!   skipped and counted; one bad feature never fails the tile.
//!
//! ## Style
//!
//! Layers are coloured by first-appearance order in the tile, cycling the eight
//! [`LAYER_COLORS`]; markers are [`POINT_SIZE_PX`], polylines [`LINE_WIDTH_PX`],
//! area fills translucent ([`POLYGON_FILL_ALPHA`]) with a crisper outline. The
//! palette is deliberately more saturated than the imagery, so vectors stay
//! legible on both the procedural map and OSM tiles.
//!
//! ## Source
//!
//! [`MvtSource`] resolves `{z}/{x}/{y}` in an HTTP URL template and in a local
//! directory (`{z}/{x}/{y}.mvt`); the local file wins when present, so an
//! offline run and an acceptance server share one configuration.

use std::path::{Path, PathBuf};
use std::time::Duration;

use geozero::error::Result as GeoResult;
use geozero::mvt::tile;
use geozero::mvt::{Message, Tile};
use geozero::{GeomProcessor, GeozeroGeometry};

use crate::math::geo::{TileBounds, TileId, tile_bounds};
use crate::scene::plot::{DemoShapes, GeoPoint, GeoPolygon, GeoPolyline};

/// MVT default tile extent (tile-local units per tile edge).
pub const DEFAULT_EXTENT: u32 = 4096;

/// Point marker diameter for MVT layers (pixels).
pub const POINT_SIZE_PX: f32 = 4.0;
/// MVT polyline width (pixels).
pub const LINE_WIDTH_PX: f32 = 1.5;
/// MVT polygon outline width (pixels).
pub const POLYGON_STROKE_PX: f32 = 1.0;
/// Fill alpha of an MVT polygon.
pub const POLYGON_FILL_ALPHA: f32 = 0.22;
/// Stroke alpha of an MVT polygon outline.
pub const POLYGON_STROKE_ALPHA: f32 = 0.85;

/// Eight-colour cycle for MVT layers, one colour per layer (by appearance).
pub const LAYER_COLORS: [[f32; 4]; 8] = [
    [1.00, 0.36, 0.26, 1.00],
    [0.20, 0.72, 1.00, 1.00],
    [0.45, 1.00, 0.55, 1.00],
    [1.00, 0.86, 0.26, 1.00],
    [0.82, 0.56, 1.00, 1.00],
    [0.20, 1.00, 0.95, 1.00],
    [1.00, 0.55, 0.82, 1.00],
    [0.75, 0.86, 1.00, 1.00],
];

/// Colour of the layer at `index` in a tile's layer list (cycling).
pub fn layer_color(index: usize) -> [f32; 4] {
    LAYER_COLORS[index % LAYER_COLORS.len()]
}

/// Upper bound on one fetched tile, so a hostile or broken endpoint cannot
/// allocate unbounded memory on a worker thread.
pub const MAX_TILE_BYTES: usize = 16 * 1024 * 1024;

/// Default per-tile fetch timeout for an MVT source.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(7);

const USER_AGENT: &str = "ew_gis/0.1 (EW GIS MVT vector layer)";

#[derive(Debug, thiserror::Error)]
pub enum MvtError {
    #[error("cannot read {path}: {message}")]
    Io { path: String, message: String },
    #[error("MVT request {url} failed: {message}")]
    Http { url: String, message: String },
    #[error("MVT request {url} returned HTTP {status}")]
    HttpStatus { url: String, status: u16 },
    #[error("no MVT tile {z}/{x}/{y} (local {path} missing, no URL template configured)")]
    NoSource {
        z: u32,
        x: u32,
        y: u32,
        path: String,
    },
    #[error("invalid MVT tile {z}/{x}/{y}: {message}")]
    Decode {
        z: u32,
        x: u32,
        y: u32,
        message: String,
    },
}

/// Where one tile came from (startup/summary reporting).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MvtOrigin {
    Local,
    Online,
}

impl MvtOrigin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Online => "online",
        }
    }
}

/// One decoded MVT tile: the plot shapes plus the counters the HUD and the
/// summary report. `layers` / `features` are what the tile contained; `skipped`
/// and `holes` are what the importer had to drop.
#[derive(Clone, Debug)]
pub struct MvtTile {
    pub id: TileId,
    pub shapes: DemoShapes,
    /// Number of layers in the tile.
    pub layers: usize,
    /// Number of features in the tile (parsed and skipped alike).
    pub features: usize,
    /// Features dropped because the geometry was empty, unknown or malformed.
    pub skipped: usize,
    /// Interior rings dropped because the plot scene renders exterior rings only.
    pub holes: usize,
}

impl MvtTile {
    /// An empty tile for `id`, ready to be filled by the decoder.
    pub fn new(id: TileId) -> Self {
        Self {
            id,
            shapes: DemoShapes::empty(),
            layers: 0,
            features: 0,
            skipped: 0,
            holes: 0,
        }
    }

    pub fn shape_count(&self) -> usize {
        self.shapes.counts().iter().sum()
    }
}

/// Convert a tile-local MVT position (`0..extent`, `y` growing south) into
/// WGS84 degrees, using the linear tile-box mapping documented at module level.
///
/// Public entry point for the coordinate rule: the decoder precomputes the tile
/// bounds once per tile and uses the same helper, this is the per-point form.
#[allow(dead_code)]
pub fn tile_point_to_lonlat(id: TileId, x: f64, y: f64, extent: u32) -> [f64; 2] {
    tile_bounds_point_to_lonlat(tile_bounds(id), x, y, extent.max(1) as f64)
}

fn tile_bounds_point_to_lonlat(bounds: TileBounds, x: f64, y: f64, extent: f64) -> [f64; 2] {
    let u = x / extent;
    let v = y / extent;
    [
        bounds.lon_min + u * (bounds.lon_max - bounds.lon_min),
        bounds.lat_max - v * (bounds.lat_max - bounds.lat_min),
    ]
}

/// Decode one MVT tile into plot shapes.
pub fn decode_mvt(id: TileId, bytes: &[u8]) -> Result<MvtTile, MvtError> {
    let tile = Tile::decode(bytes).map_err(|error| MvtError::Decode {
        z: id.z,
        x: id.x,
        y: id.y,
        message: error.to_string(),
    })?;
    Ok(collect_tile(id, &tile))
}

fn collect_tile(id: TileId, tile: &Tile) -> MvtTile {
    let bounds = tile_bounds(id);
    let mut decoded = MvtTile::new(id);
    for (index, layer) in tile.layers.iter().enumerate() {
        decoded.layers += 1;
        let color = layer_color(index);
        let extent = layer.extent.unwrap_or(DEFAULT_EXTENT).max(1);
        for feature in &layer.features {
            decoded.features += 1;
            if !commands_valid(feature) {
                decoded.skipped += 1;
                continue;
            }
            let mut collector = Collector::new(bounds, extent as f64, color);
            if let Err(error) = feature.process_geom(&mut collector) {
                log::debug!("MVT {}/{}/{}: {error}", id.z, id.x, id.y);
                decoded.skipped += 1;
                continue;
            }
            if collector.shapes.counts().iter().sum::<usize>() == 0 {
                decoded.skipped += 1;
                continue;
            }
            decoded.holes += collector.holes;
            decoded.skipped += collector.skipped;
            decoded.shapes.points.extend(collector.shapes.points);
            decoded.shapes.polylines.extend(collector.shapes.polylines);
            decoded.shapes.polygons.extend(collector.shapes.polygons);
        }
    }
    decoded
}

/// Reject command streams that `geozero`'s reader would index out of bounds on.
///
/// The reader trusts the stream (`geom[0]`, `geom[3]`, a `LineTo` count that must
/// fit the buffer), so a truncated tile would panic a worker thread instead of
/// failing the decode. This walk mirrors the reader's slicing rules without
/// decoding anything.
fn commands_valid(feature: &tile::Feature) -> bool {
    let stream = &feature.geometry;
    let Some(kind) = feature
        .r#type
        .and_then(|value| tile::GeomType::try_from(value).ok())
    else {
        return false;
    };
    if stream.is_empty() {
        return false;
    }
    let command_id = stream[0] & 0x7;
    let command_count = (stream[0] >> 3) as usize;
    match kind {
        tile::GeomType::Unknown => false,
        tile::GeomType::Point => {
            // MoveTo(count) followed by two parameters per position.
            command_id == 1 && command_count >= 1 && stream.len() > 2 * command_count
        }
        tile::GeomType::Linestring | tile::GeomType::Polygon => {
            let polygon = kind == tile::GeomType::Polygon;
            let mut position = 0usize;
            while position < stream.len() {
                if stream.len() - position < 4 {
                    return false;
                }
                // MoveTo with exactly one position starts every part / ring.
                if stream[position] & 0x7 != 1 || stream[position] >> 3 != 1 {
                    return false;
                }
                // The next command must be LineTo; its count sizes the part.
                if stream[position + 3] & 0x7 != 2 {
                    return false;
                }
                let line_count = (stream[position + 3] >> 3) as usize;
                let mut size = 4 + 2 * line_count;
                if polygon {
                    // A ring is closed by ClosePath(1).
                    if stream.len() - position <= size {
                        return false;
                    }
                    if stream[position + size] & 0x7 != 7 {
                        return false;
                    }
                    size += 1;
                }
                if stream.len() - position < size {
                    return false;
                }
                position += size;
            }
            position > 0
        }
    }
}

/// Collects one feature's geometry into layer-styled plot shapes.
struct Collector {
    bounds: TileBounds,
    extent: f64,
    color: [f32; 4],
    shapes: DemoShapes,
    path: Vec<[f64; 2]>,
    in_path: bool,
    ring_mode: bool,
    ring_index: usize,
    holes: usize,
    skipped: usize,
}

impl Collector {
    fn new(bounds: TileBounds, extent: f64, color: [f32; 4]) -> Self {
        Self {
            bounds,
            extent,
            color,
            shapes: DemoShapes::empty(),
            path: Vec::new(),
            in_path: false,
            ring_mode: false,
            ring_index: 0,
            holes: 0,
            skipped: 0,
        }
    }

    fn position(&self, x: f64, y: f64) -> [f64; 2] {
        tile_bounds_point_to_lonlat(self.bounds, x, y, self.extent)
    }

    fn finish_line(&mut self) {
        let points = std::mem::take(&mut self.path);
        if points.len() < 2 {
            self.skipped += 1;
            return;
        }
        self.shapes.polylines.push(GeoPolyline {
            points,
            width_px: LINE_WIDTH_PX,
            color: self.color,
        });
    }

    fn finish_ring(&mut self) {
        let mut ring = std::mem::take(&mut self.path);
        if ring.len() >= 2 && ring[0] == ring[ring.len() - 1] {
            // MVT rings repeat the first position; the plot outline closes the
            // ring itself, so a duplicated vertex must not be kept.
            ring.pop();
        }
        if self.ring_index > 0 {
            // Interior ring: counted, not filled (the plot has no holes).
            self.holes += 1;
            return;
        }
        if ring.len() < 3 {
            self.skipped += 1;
            return;
        }
        self.shapes.polygons.push(GeoPolygon {
            ring,
            fill: [
                self.color[0],
                self.color[1],
                self.color[2],
                POLYGON_FILL_ALPHA,
            ],
            stroke: [
                self.color[0],
                self.color[1],
                self.color[2],
                POLYGON_STROKE_ALPHA,
            ],
            width_px: POLYGON_STROKE_PX,
        });
    }
}

impl GeomProcessor for Collector {
    fn xy(&mut self, x: f64, y: f64, _idx: usize) -> GeoResult<()> {
        let position = self.position(x, y);
        if self.in_path {
            self.path.push(position);
        } else {
            self.shapes.points.push(GeoPoint {
                lon: position[0],
                lat: position[1],
                size_px: POINT_SIZE_PX,
                color: self.color,
            });
        }
        Ok(())
    }

    fn linestring_begin(&mut self, _tagged: bool, _size: usize, idx: usize) -> GeoResult<()> {
        self.in_path = true;
        self.path.clear();
        self.ring_index = idx;
        Ok(())
    }

    fn linestring_end(&mut self, _tagged: bool, _idx: usize) -> GeoResult<()> {
        self.in_path = false;
        if self.ring_mode {
            self.finish_ring();
        } else {
            self.finish_line();
        }
        Ok(())
    }

    fn polygon_begin(&mut self, _tagged: bool, _size: usize, _idx: usize) -> GeoResult<()> {
        self.ring_mode = true;
        self.ring_index = 0;
        Ok(())
    }

    fn polygon_end(&mut self, _tagged: bool, _idx: usize) -> GeoResult<()> {
        self.ring_mode = false;
        self.in_path = false;
        self.path.clear();
        Ok(())
    }
}

/// Where MVT tiles come from: an HTTP template, a local directory, or both.
///
/// The local file is tried first (`{z}/{x}/{y}.mvt` under `local_dir`); when it
/// is absent the URL template is expanded (`{z}`, `{x}`, `{y}`) and fetched over
/// HTTP. An empty template disables HTTP, an empty directory disables local
/// lookup; a source with neither returns [`MvtError::NoSource`].
pub struct MvtSource {
    url_template: String,
    local_dir: PathBuf,
    timeout: Duration,
    agent: ureq::Agent,
}

impl std::fmt::Debug for MvtSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MvtSource")
            .field("url_template", &self.url_template)
            .field("local_dir", &self.local_dir)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl MvtSource {
    pub fn new(url_template: impl Into<String>, local_dir: impl Into<PathBuf>) -> Self {
        Self::with_timeout(url_template, local_dir, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(
        url_template: impl Into<String>,
        local_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(timeout)
            .timeout_read(timeout)
            .timeout_write(timeout)
            .user_agent(USER_AGENT)
            .build();
        Self {
            url_template: url_template.into().trim().to_string(),
            local_dir: local_dir.into(),
            timeout,
            agent,
        }
    }

    /// True when at least one of the URL template and the local directory is set.
    pub fn is_configured(&self) -> bool {
        !self.url_template.is_empty() || !self.local_dir.as_os_str().is_empty()
    }

    /// One-line description for the startup log.
    pub fn label(&self) -> String {
        let local = if self.local_dir.as_os_str().is_empty() {
            "none".to_string()
        } else {
            self.local_dir.display().to_string()
        };
        let remote = if self.url_template.is_empty() {
            "none".to_string()
        } else {
            self.url_template.clone()
        };
        format!(
            "local {local} / url {remote} / timeout {} ms",
            self.timeout.as_millis()
        )
    }

    /// Per-tile URL, or `None` when no template is configured.
    pub fn url_for(&self, id: TileId) -> Option<String> {
        if self.url_template.is_empty() {
            return None;
        }
        Some(
            self.url_template
                .replace("{z}", &id.z.to_string())
                .replace("{x}", &id.x.to_string())
                .replace("{y}", &id.y.to_string()),
        )
    }

    /// Local `{z}/{x}/{y}.mvt` path under the configured directory.
    pub fn local_path(&self, id: TileId) -> PathBuf {
        self.local_dir
            .join(id.z.to_string())
            .join(id.x.to_string())
            .join(format!("{}.mvt", id.y))
    }

    /// Read a tile from the local directory or fetch it from the URL template.
    pub fn fetch(&self, id: TileId) -> Result<(Vec<u8>, MvtOrigin), MvtError> {
        if !self.local_dir.as_os_str().is_empty() {
            let path = self.local_path(id);
            if path.is_file() {
                let bytes = std::fs::read(&path).map_err(|error| MvtError::Io {
                    path: path.display().to_string(),
                    message: error.to_string(),
                })?;
                if bytes.is_empty() {
                    return Err(MvtError::Io {
                        path: path.display().to_string(),
                        message: "empty tile file".to_string(),
                    });
                }
                return Ok((bytes, MvtOrigin::Local));
            }
        }
        match self.url_for(id) {
            Some(url) => self
                .fetch_url(&url, id)
                .map(|bytes| (bytes, MvtOrigin::Online)),
            None => Err(MvtError::NoSource {
                z: id.z,
                x: id.x,
                y: id.y,
                path: self.local_path(id).display().to_string(),
            }),
        }
    }

    fn fetch_url(&self, url: &str, id: TileId) -> Result<Vec<u8>, MvtError> {
        use std::io::Read;

        let response = self
            .agent
            .get(url)
            .set(
                "Accept",
                "application/vnd.mapbox-vector-tile, application/x-protobuf, \
                 application/octet-stream, */*",
            )
            .call()
            .map_err(|error| MvtError::Http {
                url: url.to_string(),
                message: error.to_string(),
            })?;
        let status = response.status();
        if !(200..300).contains(&status) {
            return Err(MvtError::HttpStatus {
                url: url.to_string(),
                status,
            });
        }
        let mut bytes = Vec::with_capacity(64 * 1024);
        response
            .into_reader()
            .take(MAX_TILE_BYTES as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| MvtError::Http {
                url: url.to_string(),
                message: error.to_string(),
            })?;
        if bytes.is_empty() {
            return Err(MvtError::Http {
                url: url.to_string(),
                message: "empty tile body".to_string(),
            });
        }
        log::debug!(
            "MVT tile {}/{}/{} fetched ({} bytes)",
            id.z,
            id.x,
            id.y,
            bytes.len()
        );
        Ok(bytes)
    }
}

/// Path of a local MVT tile, for the `--mvt-dir` help text and logging.
#[allow(dead_code)]
pub fn local_tile_path(root: &Path, id: TileId) -> PathBuf {
    root.join(id.z.to_string())
        .join(id.x.to_string())
        .join(format!("{}.mvt", id.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ew_gis_mvt_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn feature(kind: tile::GeomType, geometry: Vec<u32>) -> tile::Feature {
        tile::Feature {
            r#type: Some(kind as i32),
            geometry,
            ..Default::default()
        }
    }

    fn layer(name: &str, extent: u32, features: Vec<tile::Feature>) -> tile::Layer {
        tile::Layer {
            version: 2,
            name: name.to_string(),
            extent: Some(extent),
            features,
            ..Default::default()
        }
    }

    fn encode(layers: Vec<tile::Layer>) -> Vec<u8> {
        let tile = Tile { layers };
        tile.encode_to_vec()
    }

    /// Encode a closed ring (MoveTo, LineTo with the following deltas, ClosePath).
    /// The vertex order decides the ring role: clockwise in tile space is an
    /// exterior ring, counter-clockwise a hole (the MVT convention).
    fn ring_stream(points: &[(i32, i32)]) -> Vec<u32> {
        assert!(points.len() >= 3, "a ring needs three vertices");
        let mut stream = Vec::with_capacity(4 + 2 * points.len());
        stream.push(9); // MoveTo(1)
        stream.push(zigzag(points[0].0));
        stream.push(zigzag(points[0].1));
        stream.push((((points.len() - 1) as u32) << 3) | 2); // LineTo(n - 1)
        for pair in points.windows(2) {
            stream.push(zigzag(pair[1].0 - pair[0].0));
            stream.push(zigzag(pair[1].1 - pair[0].1));
        }
        stream.push(15); // ClosePath(1)
        stream
    }

    /// A clockwise (exterior) rectangle.
    fn rect_stream(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<u32> {
        ring_stream(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
    }

    fn zigzag(value: i32) -> u32 {
        ((value << 1) ^ (value >> 31)) as u32
    }

    #[test]
    fn tile_local_coordinates_map_to_lonlat() {
        // z=2/x=1/y=1: lon -90..0, lat 0..66.5133.
        let id = TileId::new(2, 1, 1);
        let bounds = tile_bounds(id);
        assert!((bounds.lon_min + 90.0).abs() < 1e-9);
        assert!((bounds.lon_max - 0.0).abs() < 1e-9);
        assert!((bounds.lat_max - 66.513_260_443_111_86).abs() < 1e-6);
        assert!(bounds.lat_min.abs() < 1e-9);

        // (0,0) is the north-west corner; y grows south.
        let nw = tile_point_to_lonlat(id, 0.0, 0.0, 4096);
        assert!((nw[0] - bounds.lon_min).abs() < 1e-12);
        assert!((nw[1] - bounds.lat_max).abs() < 1e-12);

        let se = tile_point_to_lonlat(id, 4096.0, 4096.0, 4096);
        assert!((se[0] - bounds.lon_max).abs() < 1e-12);
        assert!((se[1] - bounds.lat_min).abs() < 1e-12);

        // Centre of the tile is the centre of the tile box.
        let middle = tile_point_to_lonlat(id, 2048.0, 2048.0, 4096);
        assert!((middle[0] + 45.0).abs() < 1e-9);
        assert!((middle[1] - 33.256_630_221_555_93).abs() < 1e-6);

        // A non-default extent scales the same way (512 here).
        let quarter = tile_point_to_lonlat(id, 128.0, 384.0, 512);
        assert!((quarter[0] + 67.5).abs() < 1e-9);
        assert!((quarter[1] - 16.628_315_110_777_965).abs() < 1e-6);
    }

    #[test]
    fn point_line_and_polygon_decode_to_plot_shapes() {
        let id = TileId::new(2, 1, 1);
        let point = feature(tile::GeomType::Point, vec![9, 20, 20]); // (10,10)
        // (2,2) -> (2,10) -> (10,10)
        let line = feature(tile::GeomType::Linestring, vec![9, 4, 4, 18, 0, 16, 16, 0]);
        let polygon = feature(tile::GeomType::Polygon, rect_stream(10, 10, 30, 30));
        let bytes = encode(vec![
            layer("points", 4096, vec![point]),
            layer("roads", 4096, vec![line]),
            layer("landuse", 4096, vec![polygon]),
        ]);

        let tile = decode_mvt(id, &bytes).expect("decode");
        assert_eq!(tile.id, id);
        assert_eq!(tile.layers, 3);
        assert_eq!(tile.features, 3);
        assert_eq!(tile.skipped, 0);
        assert_eq!(tile.holes, 0);
        assert_eq!(tile.shapes.counts(), [1, 1, 1, 0]);
        assert_eq!(tile.shape_count(), 3);

        let expected = tile_point_to_lonlat(id, 10.0, 10.0, 4096);
        let marker = tile.shapes.points[0];
        assert!((marker.lon - expected[0]).abs() < 1e-9);
        assert!((marker.lat - expected[1]).abs() < 1e-9);
        assert_eq!(marker.size_px, POINT_SIZE_PX);
        assert_eq!(marker.color, layer_color(0));

        let line = &tile.shapes.polylines[0];
        assert_eq!(line.points.len(), 3);
        assert_eq!(line.width_px, LINE_WIDTH_PX);
        assert_eq!(line.color, layer_color(1));
        let start = tile_point_to_lonlat(id, 2.0, 2.0, 4096);
        assert!((line.points[0][0] - start[0]).abs() < 1e-9);
        assert!((line.points[0][1] - start[1]).abs() < 1e-9);

        // The closing vertex is dropped: the outline closes the ring itself.
        let polygon = &tile.shapes.polygons[0];
        assert_eq!(polygon.ring.len(), 4);
        assert!((polygon.fill[3] - POLYGON_FILL_ALPHA).abs() < 1e-6);
        assert!((polygon.stroke[3] - POLYGON_STROKE_ALPHA).abs() < 1e-6);
        assert_eq!(polygon.width_px, POLYGON_STROKE_PX);
        assert_eq!(polygon.fill[0], layer_color(2)[0]);
    }

    #[test]
    fn multipoint_and_multilinestring_expand_per_part() {
        let id = TileId::new(1, 0, 0);
        // (5,7) then (3,2), from geozero's own multipoint fixture.
        let points = feature(tile::GeomType::Point, vec![17, 10, 14, 3, 9]);
        // Two parts: (2,2)->(2,10)->(10,10) and (1,1)->(3,5).
        let lines = feature(
            tile::GeomType::Linestring,
            vec![9, 4, 4, 18, 0, 16, 16, 0, 9, 17, 17, 10, 4, 8],
        );
        let bytes = encode(vec![layer("mixed", 4096, vec![points, lines])]);
        let tile = decode_mvt(id, &bytes).expect("decode");
        assert_eq!(tile.shapes.counts(), [2, 2, 0, 0]);
        assert_eq!(tile.skipped, 0);
        let first = tile_point_to_lonlat(id, 5.0, 7.0, 4096);
        assert!((tile.shapes.points[0].lon - first[0]).abs() < 1e-9);
        assert!((tile.shapes.points[0].lat - first[1]).abs() < 1e-9);
        assert_eq!(tile.shapes.polylines[1].points.len(), 2);
    }

    #[test]
    fn polygon_holes_are_counted_but_not_rendered() {
        let id = TileId::new(2, 1, 1);
        let mut geometry = rect_stream(10, 10, 30, 30);
        // Counter-clockwise inner ring: a hole, not a second polygon.
        geometry.extend(ring_stream(&[(15, 15), (15, 25), (25, 25), (25, 15)]));
        let bytes = encode(vec![layer(
            "landuse",
            4096,
            vec![feature(tile::GeomType::Polygon, geometry)],
        )]);
        let tile = decode_mvt(id, &bytes).expect("decode");
        assert_eq!(tile.shapes.polygons.len(), 1);
        assert_eq!(tile.holes, 1);
        assert_eq!(tile.skipped, 0);
    }

    #[test]
    fn malformed_and_unknown_geometries_are_skipped_not_panicking() {
        let id = TileId::new(0, 0, 0);
        let unknown = tile::Feature {
            r#type: None,
            ..Default::default()
        };
        let bytes = encode(vec![layer(
            "broken",
            4096,
            vec![
                // Truncated point: the MoveTo count needs two parameters.
                feature(tile::GeomType::Point, vec![9]),
                // A line whose LineTo count exceeds the stream.
                feature(tile::GeomType::Linestring, vec![9, 4, 4, 18, 0]),
                // A polygon whose ring is not closed by ClosePath.
                feature(tile::GeomType::Polygon, vec![9, 4, 4, 26, 2, 2, 0, 0]),
                unknown,
            ],
        )]);
        let tile = decode_mvt(id, &bytes).expect("decode");
        assert_eq!(tile.features, 4);
        assert_eq!(tile.skipped, 4);
        assert_eq!(tile.shapes.counts(), [0, 0, 0, 0]);
    }

    #[test]
    fn garbage_bytes_report_a_decode_error() {
        let error = decode_mvt(TileId::new(1, 0, 0), b"not a vector tile")
            .expect_err("garbage must not decode");
        assert!(matches!(error, MvtError::Decode { .. }), "{error}");
        // An empty buffer is a valid empty protobuf message: no layers, no error.
        let empty = decode_mvt(TileId::new(1, 0, 0), b"").expect("empty protobuf");
        assert_eq!(empty.layers, 0);
    }

    #[test]
    fn a_tile_without_layers_is_empty_but_valid() {
        let tile = decode_mvt(TileId::new(3, 1, 2), &encode(Vec::new())).expect("decode");
        assert_eq!(tile.layers, 0);
        assert_eq!(tile.features, 0);
        assert_eq!(tile.shapes.counts(), [0, 0, 0, 0]);
    }

    #[test]
    fn layer_colours_cycle_through_eight_entries() {
        assert_eq!(LAYER_COLORS.len(), 8);
        for (index, color) in LAYER_COLORS.iter().enumerate() {
            assert_eq!(layer_color(index), *color);
            for other in LAYER_COLORS.iter().skip(index + 1) {
                assert_ne!(color, other);
            }
        }
        assert_eq!(layer_color(8), layer_color(0));
        assert_eq!(layer_color(11), layer_color(3));
    }

    #[test]
    fn source_resolves_url_and_local_paths() {
        let source = MvtSource::new("http://127.0.0.1:8080/tiles/{z}/{x}/{y}.mvt", "assets/mvt");
        let id = TileId::new(3, 4, 5);
        assert!(source.is_configured());
        assert_eq!(
            source.url_for(id).as_deref(),
            Some("http://127.0.0.1:8080/tiles/3/4/5.mvt")
        );
        assert_eq!(
            source.local_path(id),
            Path::new("assets/mvt").join("3").join("4").join("5.mvt")
        );
        assert_eq!(
            local_tile_path(Path::new("root"), id),
            Path::new("root").join("3").join("4").join("5.mvt")
        );

        let local_only = MvtSource::new("", "assets/mvt");
        assert_eq!(local_only.url_for(id), None);
        assert!(local_only.is_configured());

        let empty = MvtSource::new("", "");
        assert!(!empty.is_configured());
        assert_eq!(empty.url_for(id), None);
        assert!(matches!(
            empty.fetch(id),
            Err(MvtError::NoSource {
                z: 3,
                x: 4,
                y: 5,
                ..
            })
        ));
    }

    #[test]
    fn fetch_reads_the_local_tile_before_the_url() {
        let dir = temp_dir("local_first");
        let id = TileId::new(2, 1, 1);
        let path = dir.join("2").join("1").join("1.mvt");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create tile dir");
        std::fs::write(
            &path,
            encode(vec![layer(
                "points",
                4096,
                vec![feature(tile::GeomType::Point, vec![9, 20, 20])],
            )]),
        )
        .expect("write tile");

        // The URL points at a closed port; it must never be contacted because
        // the local file exists.
        let source = MvtSource::with_timeout(
            "http://127.0.0.1:9/{z}/{x}/{y}.mvt",
            &dir,
            Duration::from_millis(250),
        );
        let (bytes, origin) = source.fetch(id).expect("local fetch");
        assert_eq!(origin, MvtOrigin::Local);
        let tile = decode_mvt(id, &bytes).expect("decode");
        assert_eq!(tile.shapes.points.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_local_tile_without_a_url_reports_the_path() {
        let dir = temp_dir("missing");
        let source = MvtSource::new("", &dir);
        let error = source
            .fetch(TileId::new(4, 2, 3))
            .expect_err("nothing to fetch");
        let text = error.to_string();
        assert!(text.contains("4/2/3"), "{text}");
        assert!(text.contains("3.mvt"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refused_http_endpoint_reports_an_error() {
        // Loopback with a closed port: no external network, no hang (short
        // timeout). The variant depends on the OS error, so only assert failure.
        let source = MvtSource::with_timeout(
            "http://127.0.0.1:9/{z}/{x}/{y}.mvt",
            "",
            Duration::from_millis(250),
        );
        assert!(source.fetch(TileId::new(1, 0, 0)).is_err());
    }
}
