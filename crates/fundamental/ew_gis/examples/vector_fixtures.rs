//! Generate the small acceptance fixtures used by the README screenshots.
//!
//! ```powershell
//! cargo run -p ew_gis --example vector_fixtures
//! ```
//!
//! Writes `target/ew_gis_east_asia.geojson` (3 points, 1 line, 1 polygon) and
//! `target/ew_gis_east_asia_points.shp` (+ `.shx`/`.prj`) with the same three
//! points, so both importers can be exercised against the same data.

use std::error::Error;
use std::path::Path;

const POINTS: [(f64, f64); 3] = [
    (116.4074, 39.9042),
    (121.4737, 31.2304),
    (113.2644, 23.1291),
];

const LINE: [(f64, f64); 4] = [(110.0, 30.0), (114.0, 32.0), (118.0, 33.5), (121.0, 31.0)];

const POLYGON: [(f64, f64); 5] = [
    (117.0, 22.0),
    (120.5, 22.0),
    (121.5, 25.0),
    (119.0, 26.5),
    (117.0, 24.0),
];

const GEOJSON: &str = r#"{
  "type": "FeatureCollection",
  "features": [
    {"type":"Feature","properties":{"name":"Beijing"},"geometry":{"type":"Point","coordinates":[116.4074,39.9042]}},
    {"type":"Feature","properties":{"name":"Shanghai"},"geometry":{"type":"Point","coordinates":[121.4737,31.2304]}},
    {"type":"Feature","properties":{"name":"Guangzhou"},"geometry":{"type":"Point","coordinates":[113.2644,23.1291]}},
    {"type":"Feature","properties":{"name":"coast line"},"geometry":{"type":"LineString","coordinates":[[110.0,30.0],[114.0,32.0],[118.0,33.5],[121.0,31.0]]}},
    {"type":"Feature","properties":{"name":"strait area"},"geometry":{"type":"Polygon","coordinates":[[[117.0,22.0],[120.5,22.0],[121.5,25.0],[119.0,26.5],[117.0,24.0],[117.0,22.0]]]}}
  ]
}
"#;

/// A minimal WGS84 `.prj` so the fixture does not trigger the "no .prj" warning.
const PRJ: &str = "GEOGCS[\"GCS_WGS_1984\",DATUM[\"D_WGS_1984\",SPHEROID[\"WGS_1984\",6378137.0,298.257223563]],PRIMEM[\"Greenwich\",0.0],UNIT[\"Degree\",0.0174532925199433]]";

fn main() -> Result<(), Box<dyn Error>> {
    let target = Path::new("target");
    std::fs::create_dir_all(target)?;

    let geojson = target.join("ew_gis_east_asia.geojson");
    std::fs::write(&geojson, GEOJSON)?;
    println!("wrote {}", geojson.display());

    let shp = target.join("ew_gis_east_asia_points.shp");
    {
        // Dropping the writer flushes the .shp/.shx headers.
        let mut writer = shapefile::ShapeWriter::from_path(&shp)?;
        for (lon, lat) in POINTS {
            writer.write_shape(&shapefile::Point::new(lon, lat))?;
        }
    }
    std::fs::write(shp.with_extension("prj"), PRJ)?;
    println!("wrote {}", shp.display());

    let lines_path = target.join("ew_gis_east_asia_lines.shp");
    {
        let mut writer = shapefile::ShapeWriter::from_path(&lines_path)?;
        let part = LINE
            .iter()
            .map(|(lon, lat)| shapefile::Point::new(*lon, *lat))
            .collect();
        writer.write_shape(&shapefile::Polyline::with_parts(vec![part]))?;
    }
    std::fs::write(lines_path.with_extension("prj"), PRJ)?;
    println!("wrote {}", lines_path.display());

    let polygon_path = target.join("ew_gis_east_asia_polygons.shp");
    {
        let mut writer = shapefile::ShapeWriter::from_path(&polygon_path)?;
        let ring = POLYGON
            .iter()
            .map(|(lon, lat)| shapefile::Point::new(*lon, *lat))
            .collect();
        writer.write_shape(&shapefile::Polygon::new(shapefile::PolygonRing::Outer(
            ring,
        )))?;
    }
    std::fs::write(polygon_path.with_extension("prj"), PRJ)?;
    println!("wrote {}", polygon_path.display());
    Ok(())
}
