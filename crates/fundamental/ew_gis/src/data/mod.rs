//! Vector map import: ESRI Shapefile (`.shp`) and GeoJSON.
//!
//! Both loaders produce the same authoring type the plot scene already renders
//! ([`DemoShapes`]), so imported data needs no new GPU pipeline: point markers,
//! polylines and polygon fills all go through the existing three batches.
//!
//! ## Scope
//!
//! * Shapefile: Point / PointM / PointZ, MultiPoint (+M/Z), Polyline (+M/Z),
//!   Polygon (+M/Z). Only the `.shp` geometry is read; the `.dbf` attributes are
//!   ignored (kept in mind for a future label/attribute pass). The `.prj` is
//!   inspected but **not** honoured: coordinates are assumed WGS84 and are never
//!   reprojected, so a shapefile in another CRS is drawn in the wrong place.
//!   The loader records a warning instead (see [`LoadedShapes::warnings`]).
//! * GeoJSON: FeatureCollection / Feature / bare Geometry, and inside them
//!   Point, MultiPoint, LineString, MultiLineString, Polygon, MultiPolygon and
//!   GeometryCollection (recursively).
//! * Interior rings (holes) are counted and skipped, not rendered.
//! * Invalid, empty and over-cap geometries are skipped and counted; one bad
//!   record never aborts the import.
//!
//! ## Style
//!
//! One shared palette (see [`PALETTE`]); markers are [`POINT_SIZE_PX`],
//! polylines [`LINE_WIDTH_PX`], polygon outlines [`POLYGON_STROKE_PX`] and
//! polygon fills are translucent. Colours cycle per imported feature.
//!
//! ## Limits
//!
//! At most [`MAX_FEATURES`] shapes are imported per file, which bounds startup
//! and first-frame cost for the 10k-point/1k-line/500-polygon acceptance cases.

use std::path::Path;
use std::time::Instant;

use geojson::{GeoJson, Value};
use shapefile::{PolygonRing, Shape};

use crate::scene::plot::{DemoShapes, GeoPoint, GeoPolygon, GeoPolyline};

pub mod mvt;

/// Point marker diameter for imported features (pixels).
pub const POINT_SIZE_PX: f32 = 6.0;
/// Imported polyline width (pixels).
pub const LINE_WIDTH_PX: f32 = 2.0;
/// Imported polygon outline width (pixels).
pub const POLYGON_STROKE_PX: f32 = 2.0;
/// Fill alpha of an imported polygon.
pub const POLYGON_FILL_ALPHA: f32 = 0.30;
/// Stroke alpha of an imported polygon outline.
pub const POLYGON_STROKE_ALPHA: f32 = 0.95;

/// Upper bound on shapes imported from one file. Keeps startup and the first
/// geometry build bounded even for a pathological dataset.
pub const MAX_FEATURES: usize = 50_000;

/// Shared colour palette for imported vectors.
pub const PALETTE: [[f32; 4]; 8] = [
    [0.25, 0.78, 1.00, 1.00],
    [1.00, 0.72, 0.28, 1.00],
    [0.45, 0.95, 0.52, 1.00],
    [1.00, 0.45, 0.58, 1.00],
    [0.72, 0.62, 1.00, 1.00],
    [1.00, 0.92, 0.55, 1.00],
    [0.35, 0.92, 0.90, 1.00],
    [0.95, 0.60, 0.30, 1.00],
];

#[derive(Clone, Debug, thiserror::Error)]
pub enum DataError {
    #[error("cannot read {path}: {message}")]
    Io { path: String, message: String },
    #[error("invalid GeoJSON in {path}: {message}")]
    GeoJson { path: String, message: String },
    #[error("invalid shapefile {path}: {message}")]
    Shapefile { path: String, message: String },
    #[error("unsupported vector format {path} (.{extension}); use .shp or .geojson")]
    Unsupported { path: String, extension: String },
}

/// Shapes imported from one file plus the counters the acceptance report and
/// the startup log need.
#[derive(Clone, Debug, Default)]
pub struct LoadedShapes {
    pub shapes: DemoShapes,
    pub skipped: usize,
    /// Interior rings dropped because the plot scene renders outer rings only.
    pub holes: usize,
    /// Geometries dropped because [`MAX_FEATURES`] was reached.
    pub capped: usize,
    pub warnings: Vec<String>,
    /// File or inline source name, for logs.
    pub source: String,
    pub elapsed_ms: f64,
}

impl LoadedShapes {
    pub fn feature_count(&self) -> usize {
        self.shapes.counts().iter().sum()
    }

    /// Append another file's result into this one.
    pub fn merge(&mut self, other: LoadedShapes) {
        self.shapes.points.extend(other.shapes.points);
        self.shapes.polylines.extend(other.shapes.polylines);
        self.shapes.polygons.extend(other.shapes.polygons);
        self.shapes.sectors.extend(other.shapes.sectors);
        self.skipped += other.skipped;
        self.holes += other.holes;
        self.capped += other.capped;
        self.warnings.extend(other.warnings);
        self.elapsed_ms += other.elapsed_ms;
        if !other.source.is_empty() {
            if !self.source.is_empty() {
                self.source.push_str(", ");
            }
            self.source.push_str(&other.source);
        }
    }

    /// One-line report used by the startup log and the README.
    pub fn summary(&self) -> String {
        let counts = self.shapes.counts();
        format!(
            "{} shapes ({} points / {} lines / {} polygons), {} skipped, {} holes dropped, {} capped, {:.1} ms",
            self.feature_count(),
            counts[0],
            counts[1],
            counts[2],
            self.skipped,
            self.holes,
            self.capped,
            self.elapsed_ms
        )
    }
}

/// Load a vector file, choosing the loader by extension.
pub fn load_file(path: &Path) -> Result<LoadedShapes, DataError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "shp" => load_shapefile(path),
        "geojson" | "json" => load_geojson(path),
        other => Err(DataError::Unsupported {
            path: path.display().to_string(),
            extension: other.to_string(),
        }),
    }
}

pub fn load_geojson(path: &Path) -> Result<LoadedShapes, DataError> {
    let text = std::fs::read_to_string(path).map_err(|error| DataError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    load_geojson_str(&text, &path.display().to_string())
}

/// Parse GeoJSON from memory (the unit tests use this directly).
pub fn load_geojson_str(text: &str, source: &str) -> Result<LoadedShapes, DataError> {
    let started = Instant::now();
    let geojson: GeoJson = text
        .parse()
        .map_err(|error: geojson::Error| DataError::GeoJson {
            path: source.to_string(),
            message: error.to_string(),
        })?;
    let mut builder = Builder::new(source);
    match geojson {
        GeoJson::FeatureCollection(collection) => {
            for feature in collection.features {
                match feature.geometry {
                    Some(geometry) => builder.import_geometry(&geometry.value),
                    None => builder.skip("feature without a geometry".to_string()),
                }
            }
        }
        GeoJson::Feature(feature) => match feature.geometry {
            Some(geometry) => builder.import_geometry(&geometry.value),
            None => builder.skip("feature without a geometry".to_string()),
        },
        GeoJson::Geometry(geometry) => builder.import_geometry(&geometry.value),
    }
    Ok(builder.finish(started))
}

pub fn load_shapefile(path: &Path) -> Result<LoadedShapes, DataError> {
    if !path.is_file() {
        return Err(DataError::Io {
            path: path.display().to_string(),
            message: "file does not exist".to_string(),
        });
    }
    let started = Instant::now();
    let mut reader =
        shapefile::ShapeReader::from_path(path).map_err(|error| DataError::Shapefile {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
    let mut builder = Builder::new(path.display().to_string());
    builder.check_projection(path);
    for shape in reader.iter_shapes() {
        if builder.at_capacity() {
            builder.capped += 1;
            continue;
        }
        match shape {
            Ok(shape) => builder.import_shape(shape),
            Err(error) => builder.skip(format!("unreadable record: {error}")),
        }
    }
    Ok(builder.finish(started))
}

/// Coordinates accepted by both loaders. Longitude is wrapped into
/// `[-180, 180)`; latitude must be a real geographic latitude.
fn valid_lonlat(lon: f64, lat: f64) -> Option<[f64; 2]> {
    if !lon.is_finite() || !lat.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) {
        return None;
    }
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    let lon = if wrapped >= 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    };
    Some([lon, lat])
}

/// Validate every coordinate of a path; `None` when any of them is invalid.
fn path_coords(coords: &[[f64; 2]]) -> Option<Vec<[f64; 2]>> {
    let mut path = Vec::with_capacity(coords.len());
    for coord in coords {
        path.push(valid_lonlat(coord[0], coord[1])?);
    }
    Some(path)
}

/// Drop the GeoJSON closing duplicate, reject rings that cannot tessellate.
fn ring_of(coords: &[[f64; 2]]) -> Option<Vec<[f64; 2]>> {
    let mut ring = path_coords(coords)?;
    if ring.len() >= 2 && ring[0] == ring[ring.len() - 1] {
        ring.pop();
    }
    if ring.len() < 3 {
        return None;
    }
    let first = ring[0];
    if ring.iter().all(|point| *point == first) {
        return None;
    }
    Some(ring)
}

/// Offset used when indexing the style palette.
fn palette_color(index: usize) -> [f32; 4] {
    PALETTE[index % PALETTE.len()]
}

struct Builder {
    shapes: DemoShapes,
    skipped: usize,
    holes: usize,
    capped: usize,
    warnings: Vec<String>,
    next_style: usize,
    source: String,
}

impl Builder {
    fn new(source: impl Into<String>) -> Self {
        Self {
            shapes: DemoShapes::empty(),
            skipped: 0,
            holes: 0,
            capped: 0,
            warnings: Vec::new(),
            next_style: 0,
            source: source.into(),
        }
    }

    fn feature_count(&self) -> usize {
        self.shapes.counts().iter().sum()
    }

    fn at_capacity(&self) -> bool {
        self.feature_count() >= MAX_FEATURES
    }

    fn finish(self, started: Instant) -> LoadedShapes {
        LoadedShapes {
            shapes: self.shapes,
            skipped: self.skipped,
            holes: self.holes,
            capped: self.capped,
            warnings: self.warnings,
            source: self.source,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        }
    }

    fn skip(&mut self, reason: impl Into<String>) {
        self.skipped += 1;
        let reason = reason.into();
        if self.warnings.len() < 32 {
            self.warnings.push(format!("{}: {reason}", self.source));
        }
    }

    fn color(&mut self) -> [f32; 4] {
        let color = palette_color(self.next_style);
        self.next_style += 1;
        color
    }

    fn push_point(&mut self, lon: f64, lat: f64, color: [f32; 4]) {
        self.shapes.points.push(GeoPoint {
            lon,
            lat,
            size_px: POINT_SIZE_PX,
            color,
        });
    }

    fn push_line(&mut self, points: Vec<[f64; 2]>, color: [f32; 4]) {
        self.shapes.polylines.push(GeoPolyline {
            points,
            width_px: LINE_WIDTH_PX,
            color,
        });
    }

    fn push_polygon(&mut self, ring: Vec<[f64; 2]>, color: [f32; 4]) {
        self.shapes.polygons.push(GeoPolygon {
            ring,
            fill: [color[0], color[1], color[2], POLYGON_FILL_ALPHA],
            stroke: [color[0], color[1], color[2], POLYGON_STROKE_ALPHA],
            width_px: POLYGON_STROKE_PX,
        });
    }

    /// `.prj` handling: warn, never reproject.
    fn check_projection(&mut self, path: &Path) {
        let prj = path.with_extension("prj");
        match std::fs::read_to_string(&prj) {
            Ok(text) => {
                if !text.to_ascii_uppercase().contains("WGS_1984")
                    && !text.to_ascii_uppercase().contains("WGS84")
                    && !text.contains("EPSG:4326")
                    && !text.contains("4326")
                {
                    self.warnings.push(format!(
                        "{}: projection does not look like WGS84 ({}); \
                         coordinates are used as-is and are NOT reprojected",
                        path.display(),
                        prj.display()
                    ));
                }
            }
            Err(_) => self.warnings.push(format!(
                "{}: no .prj alongside the .shp; assuming WGS84, no reprojection",
                path.display()
            )),
        }
    }

    fn import_geometry(&mut self, value: &Value) {
        if self.at_capacity() {
            self.capped += 1;
            return;
        }
        let color = self.color();
        match value {
            Value::Point(coord) => self.import_coord(coord, color),
            Value::MultiPoint(coords) => {
                if coords.is_empty() {
                    self.skip("empty MultiPoint".to_string());
                    return;
                }
                for coord in coords {
                    if self.at_capacity() {
                        self.capped += 1;
                        return;
                    }
                    self.import_coord(coord, color);
                }
            }
            Value::LineString(coords) => self.import_path(coords, color),
            Value::MultiLineString(lines) => {
                if lines.is_empty() {
                    self.skip("empty MultiLineString".to_string());
                    return;
                }
                for line in lines {
                    if self.at_capacity() {
                        self.capped += 1;
                        return;
                    }
                    self.import_path(line, color);
                }
            }
            Value::Polygon(rings) => self.import_rings(rings, color),
            Value::MultiPolygon(polygons) => {
                if polygons.is_empty() {
                    self.skip("empty MultiPolygon".to_string());
                    return;
                }
                for rings in polygons {
                    if self.at_capacity() {
                        self.capped += 1;
                        return;
                    }
                    self.import_rings(rings, color);
                }
            }
            Value::GeometryCollection(geometries) => {
                if geometries.is_empty() {
                    self.skip("empty GeometryCollection".to_string());
                    return;
                }
                for geometry in geometries {
                    self.import_geometry(&geometry.value);
                }
            }
        }
    }

    fn import_coord(&mut self, coord: &[f64], color: [f32; 4]) {
        if coord.len() < 2 {
            self.skip("coordinate with fewer than two ordinates".to_string());
            return;
        }
        match valid_lonlat(coord[0], coord[1]) {
            Some([lon, lat]) => self.push_point(lon, lat, color),
            None => self.skip(format!("invalid coordinate {coord:?}")),
        }
    }

    fn import_path(&mut self, coords: &[Vec<f64>], color: [f32; 4]) {
        if coords.len() < 2 {
            self.skip("line with fewer than two positions".to_string());
            return;
        }
        let mut path = Vec::with_capacity(coords.len());
        for coord in coords {
            if coord.len() < 2 {
                self.skip("position with fewer than two ordinates".to_string());
                return;
            }
            match valid_lonlat(coord[0], coord[1]) {
                Some(point) => path.push(point),
                None => {
                    self.skip(format!("invalid position {coord:?}"));
                    return;
                }
            }
        }
        self.push_line(path, color);
    }

    fn import_rings(&mut self, rings: &[Vec<Vec<f64>>], color: [f32; 4]) {
        if rings.is_empty() {
            self.skip("polygon without rings".to_string());
            return;
        }
        let Some(outer) = self.geojson_ring(&rings[0]) else {
            self.skip("polygon with an invalid outer ring".to_string());
            return;
        };
        self.holes += rings.len() - 1;
        self.push_polygon(outer, color);
    }

    fn geojson_ring(&mut self, coords: &[Vec<f64>]) -> Option<Vec<[f64; 2]>> {
        let mut ring = Vec::with_capacity(coords.len());
        for coord in coords {
            if coord.len() < 2 {
                return None;
            }
            ring.push(valid_lonlat(coord[0], coord[1])?);
        }
        ring_of(&ring)
    }

    fn import_shape(&mut self, shape: Shape) {
        match shape {
            Shape::NullShape => self.skip("null shape record".to_string()),
            Shape::Point(point) => self.import_xy(point.x, point.y),
            Shape::PointM(point) => self.import_xy(point.x, point.y),
            Shape::PointZ(point) => self.import_xy(point.x, point.y),
            Shape::Multipoint(multipoint) => self.import_xy_slice(multipoint.points()),
            Shape::MultipointM(multipoint) => self.import_xy_slice(multipoint.points()),
            Shape::MultipointZ(multipoint) => self.import_xy_slice(multipoint.points()),
            Shape::Polyline(polyline) => self.import_parts(polyline.parts()),
            Shape::PolylineM(polyline) => self.import_parts(polyline.parts()),
            Shape::PolylineZ(polyline) => self.import_parts(polyline.parts()),
            Shape::Polygon(polygon) => self.import_shape_rings(polygon.rings()),
            Shape::PolygonM(polygon) => self.import_shape_rings(polygon.rings()),
            Shape::PolygonZ(polygon) => self.import_shape_rings(polygon.rings()),
            Shape::Multipatch(_) => self.skip("Multipatch records are not supported".to_string()),
        }
    }

    fn import_xy(&mut self, x: f64, y: f64) {
        let color = self.color();
        match valid_lonlat(x, y) {
            Some([lon, lat]) => self.push_point(lon, lat, color),
            None => self.skip(format!("invalid coordinate ({x}, {y})")),
        }
    }

    fn import_xy_slice<P: Xy>(&mut self, points: &[P]) {
        if points.is_empty() {
            self.skip("empty multipoint record".to_string());
            return;
        }
        let color = self.color();
        for point in points {
            if self.at_capacity() {
                self.capped += 1;
                return;
            }
            let (x, y) = point.xy();
            match valid_lonlat(x, y) {
                Some([lon, lat]) => self.push_point(lon, lat, color),
                None => self.skip(format!("invalid coordinate ({x}, {y})")),
            }
        }
    }

    fn import_parts<P: Xy>(&mut self, parts: &[Vec<P>]) {
        if parts.is_empty() {
            self.skip("polyline without parts".to_string());
            return;
        }
        let color = self.color();
        for part in parts {
            if self.at_capacity() {
                self.capped += 1;
                return;
            }
            let mut path = Vec::with_capacity(part.len());
            let mut valid = true;
            for point in part {
                let (x, y) = point.xy();
                match valid_lonlat(x, y) {
                    Some(position) => path.push(position),
                    None => {
                        self.skip(format!("invalid coordinate ({x}, {y})"));
                        valid = false;
                        break;
                    }
                }
            }
            if valid && path.len() >= 2 {
                self.push_line(path, color);
            } else if valid {
                self.skip("polyline part with fewer than two points".to_string());
            }
        }
    }

    fn import_shape_rings<P: Xy>(&mut self, rings: &[PolygonRing<P>]) {
        if rings.is_empty() {
            self.skip("polygon without rings".to_string());
            return;
        }
        let color = self.color();
        let mut holes = 0;
        let mut imported = 0;
        for ring in rings {
            match ring {
                PolygonRing::Outer(points) => {
                    if self.at_capacity() {
                        self.capped += 1;
                        break;
                    }
                    let mut coords = Vec::with_capacity(points.len());
                    for point in points {
                        let (x, y) = point.xy();
                        match valid_lonlat(x, y) {
                            Some(position) => coords.push(position),
                            None => {
                                coords.clear();
                                break;
                            }
                        }
                    }
                    match ring_of(&coords) {
                        Some(ring) => {
                            self.push_polygon(ring, color);
                            imported += 1;
                        }
                        None => self.skip("polygon part with an invalid outer ring".to_string()),
                    }
                }
                PolygonRing::Inner(_) => holes += 1,
            }
        }
        if imported == 0 && holes == 0 {
            self.skip("polygon without a usable outer ring".to_string());
        }
        self.holes += holes;
    }
}

/// Shapefile point types share only `x`/`y` accessors, not a trait.
trait Xy {
    fn xy(&self) -> (f64, f64);
}

impl Xy for shapefile::Point {
    fn xy(&self) -> (f64, f64) {
        (self.x, self.y)
    }
}

impl Xy for shapefile::PointM {
    fn xy(&self) -> (f64, f64) {
        (self.x, self.y)
    }
}

impl Xy for shapefile::PointZ {
    fn xy(&self) -> (f64, f64) {
        (self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ew_gis_data_{}_{}_{}",
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

    #[test]
    fn geojson_feature_collection_imports_every_geometry_kind() {
        let text = r##"{
          "type": "FeatureCollection",
          "features": [
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[116.4,39.9]}},
            {"type":"Feature","properties":{},"geometry":{"type":"MultiPoint","coordinates":[[121.5,31.2],[113.3,23.1]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[116.0,39.0],[117.5,38.0],[119.0,37.5]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"MultiLineString","coordinates":[[[100.0,20.0],[101.0,21.0]],[[102.0,22.0],[103.0,23.0]]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[110.0,25.0],[111.0,25.0],[111.5,26.0],[110.0,25.0]]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"MultiPolygon","coordinates":[[[[120.0,30.0],[121.0,30.0],[121.0,31.0],[120.0,30.0]]],[[[122.0,32.0],[123.0,32.0],[123.0,33.0],[122.0,32.0]]]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[108.0,18.0]},{"type":"LineString","coordinates":[[108.0,18.0],[109.0,19.0]]}]}}
          ]
        }"##;
        let loaded = load_geojson_str(text, "inline").expect("valid geojson");
        let counts = loaded.shapes.counts();
        // Point + MultiPoint(2) + GeometryCollection point.
        assert_eq!(counts[0], 4, "points {}", counts[0]);
        // LineString + 2 MultiLineString parts + GeometryCollection line.
        assert_eq!(counts[1], 4, "lines {}", counts[1]);
        // Polygon + 2 MultiPolygon parts.
        assert_eq!(counts[2], 3, "polygons {}", counts[2]);
        assert_eq!(loaded.skipped, 0);
        assert_eq!(loaded.holes, 0);

        assert!((loaded.shapes.points[0].lon - 116.4).abs() < 1e-9);
        assert!((loaded.shapes.points[0].lat - 39.9).abs() < 1e-9);
        assert_eq!(loaded.shapes.points[0].size_px, POINT_SIZE_PX);
        assert_eq!(loaded.shapes.polylines[0].points.len(), 3);
        assert_eq!(loaded.shapes.polylines[0].width_px, LINE_WIDTH_PX);
        assert_eq!(loaded.shapes.polygons[0].ring.len(), 3);
        assert_eq!(loaded.shapes.polygons[0].width_px, POLYGON_STROKE_PX);
        assert!(loaded.shapes.polygons[0].fill[3] < 1.0);
        assert!((loaded.shapes.polygons[0].stroke[3] - 0.95).abs() < 1e-6);
        assert_eq!(loaded.shapes.polygons[1].ring.len(), 3);
    }

    #[test]
    fn geojson_bare_geometry_and_feature_are_accepted() {
        let point = load_geojson_str(r#"{"type":"Point","coordinates":[10.0,20.0]}"#, "inline")
            .expect("bare geometry");
        assert_eq!(point.shapes.points.len(), 1);

        let feature = load_geojson_str(
            r#"{"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]}}"#,
            "inline",
        )
        .expect("bare feature");
        assert_eq!(feature.shapes.polylines.len(), 1);
        assert_eq!(feature.shapes.polylines[0].points.len(), 2);
    }

    #[test]
    fn geojson_holes_are_counted_but_not_rendered() {
        let text = r#"{
          "type":"Feature",
          "geometry":{"type":"Polygon","coordinates":[
            [[0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]],
            [[1.0,1.0],[2.0,1.0],[2.0,2.0],[1.0,2.0],[1.0,1.0]],
            [[2.5,2.5],[3.0,2.5],[3.0,3.0],[2.5,2.5]]
          ]}
        }"#;
        let loaded = load_geojson_str(text, "inline").expect("polygon with holes");
        assert_eq!(loaded.shapes.polygons.len(), 1);
        // The closing duplicate is dropped so the outline closes exactly once.
        assert_eq!(loaded.shapes.polygons[0].ring.len(), 4);
        assert_eq!(loaded.holes, 2);
        assert_eq!(loaded.skipped, 0);
    }

    #[test]
    fn geojson_invalid_and_empty_geometries_are_skipped() {
        let text = r##"{
          "type": "FeatureCollection",
          "features": [
            {"type":"Feature","properties":{},"geometry":null},
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[10.0,999.0]}},
            {"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0.0,0.0]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[]}},
            {"type":"Feature","properties":{},"geometry":{"type":"MultiPoint","coordinates":[]}},
            {"type":"Feature","properties":{},"geometry":{"type":"GeometryCollection","geometries":[]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[100.0,0.0]}}
          ]
        }"##;
        let loaded = load_geojson_str(text, "inline").expect("valid json");
        assert_eq!(loaded.shapes.counts(), [1, 0, 0, 0]);
        assert_eq!(loaded.skipped, 6);
    }

    #[test]
    fn geojson_longitudes_are_wrapped() {
        let loaded = load_geojson_str(r#"{"type":"Point","coordinates":[190.0,10.0]}"#, "inline")
            .expect("valid geojson");
        assert!((loaded.shapes.points[0].lon + 170.0).abs() < 1e-9);
    }

    #[test]
    fn geojson_broken_json_reports_an_error() {
        let error = load_geojson_str("{not json", "inline").expect_err("must fail");
        assert!(matches!(error, DataError::GeoJson { .. }), "{error:?}");
    }

    #[test]
    fn missing_and_empty_paths_do_not_panic() {
        let dir = temp_dir("errors");
        let missing = dir.join("missing.geojson");
        assert!(load_geojson(&missing).is_err());
        assert!(load_shapefile(&missing).is_err());
        assert!(load_file(&missing).is_err());

        let empty = dir.join("empty.geojson");
        std::fs::write(&empty, "").expect("write empty file");
        assert!(load_geojson(&empty).is_err());

        let empty_shp = dir.join("empty.shp");
        std::fs::write(&empty_shp, "").expect("write empty shapefile");
        assert!(load_shapefile(&empty_shp).is_err());

        let unsupported = dir.join("data.kml");
        std::fs::write(&unsupported, "<kml/>").expect("write kml");
        assert!(matches!(
            load_file(&unsupported),
            Err(DataError::Unsupported { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shapefile_points_and_polylines_are_read_from_written_fixtures() {
        let dir = temp_dir("shapefile");
        let points_path = dir.join("points.shp");
        {
            let mut writer = shapefile::ShapeWriter::from_path(&points_path).expect("shape writer");
            writer
                .write_shape(&shapefile::Point::new(116.4, 39.9))
                .expect("write point");
            writer
                .write_shape(&shapefile::Point::new(121.5, 31.2))
                .expect("write point");
        }
        let points = load_shapefile(&points_path).expect("read points");
        assert_eq!(points.shapes.points.len(), 2);
        assert!((points.shapes.points[0].lon - 116.4).abs() < 1e-9);
        assert!((points.shapes.points[0].lat - 39.9).abs() < 1e-9);

        let line_path = dir.join("lines.shp");
        {
            let mut writer = shapefile::ShapeWriter::from_path(&line_path).expect("shape writer");
            let polyline = shapefile::Polyline::with_parts(vec![vec![
                shapefile::Point::new(116.0, 39.0),
                shapefile::Point::new(118.0, 37.0),
                shapefile::Point::new(120.0, 35.5),
            ]]);
            writer.write_shape(&polyline).expect("write polyline");
        }
        let lines = load_shapefile(&line_path).expect("read polylines");
        assert_eq!(lines.shapes.polylines.len(), 1);
        assert_eq!(lines.shapes.polylines[0].points.len(), 3);
        assert_eq!(lines.shapes.polylines[0].width_px, LINE_WIDTH_PX);
        assert!((lines.shapes.polylines[0].points[2][0] - 120.0).abs() < 1e-9);
        // No .prj next to the fixture: the loader records the WGS84 assumption.
        assert!(
            lines
                .warnings
                .iter()
                .any(|warning| warning.contains("assuming WGS84")),
            "{:?}",
            lines.warnings
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shapefile_polygon_outer_ring_is_imported_and_holes_counted() {
        let dir = temp_dir("shapefile_polygon");
        let path = dir.join("polygon.shp");
        {
            let mut writer = shapefile::ShapeWriter::from_path(&path).expect("shape writer");
            let outer = shapefile::PolygonRing::Outer(vec![
                shapefile::Point::new(110.0, 25.0),
                shapefile::Point::new(112.0, 25.0),
                shapefile::Point::new(112.0, 27.0),
                shapefile::Point::new(110.0, 27.0),
                shapefile::Point::new(110.0, 25.0),
            ]);
            let inner = shapefile::PolygonRing::Inner(vec![
                shapefile::Point::new(110.5, 25.5),
                shapefile::Point::new(111.0, 25.5),
                shapefile::Point::new(111.0, 26.0),
                shapefile::Point::new(110.5, 25.5),
            ]);
            writer
                .write_shape(&shapefile::Polygon::with_rings(vec![outer, inner]))
                .expect("write polygon");
        }
        let loaded = load_shapefile(&path).expect("read polygon");
        assert_eq!(loaded.shapes.polygons.len(), 1);
        assert_eq!(loaded.holes, 1);
        assert_eq!(loaded.shapes.polygons[0].ring.len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shapefile_non_wgs84_projection_is_reported_not_applied() {
        let dir = temp_dir("shapefile_prj");
        let path = dir.join("projected.shp");
        {
            let mut writer = shapefile::ShapeWriter::from_path(&path).expect("shape writer");
            writer
                .write_shape(&shapefile::Point::new(40.5, 30.5))
                .expect("write point");
        }
        std::fs::write(
            path.with_extension("prj"),
            "PROJCS[\"NAD83 / UTM zone 10N\",GEOGCS[\"NAD83\",\
             DATUM[\"North_American_Datum_1983\",SPHEROID[\"GRS 1980\",6378137,298.257222101]],\
             PRIMEM[\"Greenwich\",0],UNIT[\"metre\",1]]",
        )
        .expect("write prj");
        let loaded = load_shapefile(&path).expect("read projected shapefile");
        // The coordinates are used as-is (no reprojection).
        assert_eq!(loaded.shapes.points.len(), 1);
        assert!((loaded.shapes.points[0].lon - 40.5).abs() < 1e-9);
        assert!((loaded.shapes.points[0].lat - 30.5).abs() < 1e-9);
        assert!(
            loaded
                .warnings
                .iter()
                .any(|warning| warning.contains("NOT reprojected")),
            "{:?}",
            loaded.warnings
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loaded_shapes_merge_counts_and_coordinates() {
        let a = load_geojson_str(
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1.0,2.0]}}]}"#,
            "a",
        )
        .expect("first");
        let b = load_geojson_str(
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"LineString","coordinates":[[3.0,4.0],[5.0,6.0]]}}]}"#,
            "b",
        )
        .expect("second");
        let mut merged = a;
        merged.merge(b);
        assert_eq!(merged.feature_count(), 2);
        assert_eq!(merged.shapes.counts(), [1, 1, 0, 0]);
        assert!(merged.source.contains("a") && merged.source.contains("b"));
        assert!(merged.summary().contains("1 points"));
    }

    #[test]
    fn palette_cycles_through_the_shared_colours() {
        assert_eq!(palette_color(0), PALETTE[0]);
        assert_eq!(palette_color(PALETTE.len()), PALETTE[0]);
        assert_ne!(PALETTE[0], PALETTE[1]);
    }

    #[test]
    fn large_import_stays_under_the_cap_and_is_fast() {
        // 10k points, 1k lines, 500 small polygons: the acceptance workload.
        let mut text = String::from(r#"{"type":"FeatureCollection","features":["#);
        for index in 0..10_000 {
            if index > 0 {
                text.push(',');
            }
            let lon = 100.0 + (index % 100) as f64 * 0.1;
            let lat = 20.0 + (index / 100) as f64 * 0.1;
            text.push_str(&format!(
                r#"{{"type":"Feature","geometry":{{"type":"Point","coordinates":[{lon},{lat}]}}}}"#
            ));
        }
        for index in 0..1_000 {
            let lat = 20.0 + index as f64 * 0.01;
            text.push_str(&format!(
                r#",{{"type":"Feature","geometry":{{"type":"LineString","coordinates":[[100.0,{lat}],[101.0,{lat}]]}}}}"#
            ));
        }
        for index in 0..500 {
            let lon = 105.0 + index as f64 * 0.01;
            let next = lon + 0.005;
            text.push_str(&format!(
                r#",{{"type":"Feature","geometry":{{"type":"Polygon","coordinates":[[[{lon},22.0],[{next},22.0],[{next},23.0],[{lon},22.0]]]}}}}"#
            ));
        }
        text.push_str("]}");
        let started = std::time::Instant::now();
        let loaded = load_geojson_str(&text, "synthetic").expect("large geojson");
        let elapsed = started.elapsed();
        assert_eq!(loaded.shapes.counts(), [10_000, 1_000, 500, 0]);
        assert!(loaded.feature_count() <= MAX_FEATURES);
        assert_eq!(loaded.capped, 0);
        assert!(
            elapsed.as_secs_f64() < 5.0,
            "10k/1k/500 import took {elapsed:?}"
        );
    }
}
