//! KML 规范 - 参考自 DataSources/KmlDataSourceSpec、KmlTourSpec、exportKmlSpec
//! 覆盖：parse_kml_simple、parse_coordinates、parse_kml_color、kml_to_datasource、
//! KmlTour、KmlTourFlyTo、KmlExporter、rgba_to_kml_color

use cesium_kml::{
    parse_coordinates, parse_kml_color, parse_kml_simple, rgba_to_kml_color,
    KmlExporter, KmlTour, KmlTourFlyTo, FlyToMode,
};
use glam::DVec3;

const SIMPLE_KML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2">
  <Document>
    <name>Test Document</name>
    <Placemark>
      <name>Point 1</name>
      <Point>
        <coordinates>-122.0822035,37.4220033612141,0</coordinates>
      </Point>
    </Placemark>
    <Placemark>
      <name>Line 1</name>
      <LineString>
        <coordinates>
          -122.084075,37.4220033612141,0
          -122.085071,37.4226,0
        </coordinates>
      </LineString>
    </Placemark>
  </Document>
</kml>"#;

// ─── parse_kml_simple ───────────────────────────────────────────────────────

#[test]
fn parse_kml_simple_basic() {
    let doc = parse_kml_simple(SIMPLE_KML);
    assert!(doc.is_ok(), "should parse valid KML");
    let doc = doc.unwrap();
    assert_eq!(doc.name.as_deref(), Some("Test Document"));
    assert_eq!(doc.placemarks.len(), 2);
}

#[test]
fn parse_kml_simple_placemark_names() {
    let doc = parse_kml_simple(SIMPLE_KML).unwrap();
    assert_eq!(doc.placemarks[0].name.as_deref(), Some("Point 1"));
    assert_eq!(doc.placemarks[1].name.as_deref(), Some("Line 1"));
}

#[test]
fn parse_kml_simple_invalid() {
    let result = parse_kml_simple("not valid xml at all");
    // 应报错或产生空文档
    if let Ok(doc) = result {
        assert!(doc.placemarks.is_empty());
    }
}

// ─── parse_coordinates ──────────────────────────────────────────────────────

#[test]
fn parse_coordinates_single() {
    let coords = parse_coordinates("-122.0822035,37.4220033612141,0");
    assert_eq!(coords.len(), 1);
    assert!((coords[0].longitude - (-122.0822035)).abs() < 1e-6);
    assert!((coords[0].latitude - 37.4220033612141).abs() < 1e-6);
}

#[test]
fn parse_coordinates_multiple() {
    let coords = parse_coordinates(
        "-122.084075,37.4220033612141,0 -122.085071,37.4226,0",
    );
    assert_eq!(coords.len(), 2);
}

#[test]
fn parse_coordinates_empty() {
    let coords = parse_coordinates("");
    assert!(coords.is_empty());
}

// ─── parse_kml_color ────────────────────────────────────────────────────────

#[test]
fn parse_kml_color_aabbggrr() {
    // KML 颜色格式：aabbggrr
    let color = parse_kml_color("ff0000ff"); // 红色，完全不透明
    assert!(color.is_some());
    let c = color.unwrap();
    assert!((c.red - 1.0).abs() < 0.01);
    assert!((c.green - 0.0).abs() < 0.01);
    assert!((c.blue - 0.0).abs() < 0.01);
    assert!((c.alpha - 1.0).abs() < 0.01);
}

#[test]
fn parse_kml_color_green() {
    let color = parse_kml_color("ff00ff00"); // 绿色
    let c = color.unwrap();
    assert!((c.red - 0.0).abs() < 0.01);
    assert!((c.green - 1.0).abs() < 0.01);
    assert!((c.blue - 0.0).abs() < 0.01);
}

#[test]
fn parse_kml_color_invalid() {
    let color = parse_kml_color("xyz");
    assert!(color.is_none());
}

// ─── rgba_to_kml_color ──────────────────────────────────────────────────────

#[test]
fn rgba_to_kml_color_red() {
    let kml_color = rgba_to_kml_color(1.0, 0.0, 0.0, 1.0);
    assert_eq!(kml_color, "ff0000ff");
}

#[test]
fn rgba_to_kml_color_green() {
    let kml_color = rgba_to_kml_color(0.0, 1.0, 0.0, 1.0);
    assert_eq!(kml_color, "ff00ff00");
}

#[test]
fn rgba_to_kml_color_semi_transparent() {
    let kml_color = rgba_to_kml_color(1.0, 1.0, 1.0, 0.5);
    // Alpha 0.5 → 0x80 = 128 → "80" 或 0x7f = 127 → "7f"
    assert!(kml_color.starts_with("80") || kml_color.starts_with("7f"));
}

// ─── KmlTour ────────────────────────────────────────────────────────────────

#[test]
fn kml_tour_flyto_mode() {
    assert_ne!(FlyToMode::Bounce, FlyToMode::Smooth);
}

#[test]
fn kml_tour_flyto_creation() {
    let flyto = KmlTourFlyTo {
        duration: 5.0,
        position: DVec3::new(-122.0, 37.0, 1000.0),
        heading: Some(0.0),
        tilt: Some(45.0),
        range: Some(5000.0),
        fly_to_mode: FlyToMode::Smooth,
    };
    assert_eq!(flyto.duration, 5.0);
    assert_eq!(flyto.fly_to_mode, FlyToMode::Smooth);
    assert_eq!(flyto.position.x, -122.0);
}

#[test]
fn kml_tour_empty() {
    let tour = KmlTour {
        id: "tour1".to_string(),
        name: "Test Tour".to_string(),
        playlist: vec![],
        playlist_index: 0,
        is_playing: false,
    };
    assert!(tour.playlist.is_empty());
    assert!(!tour.is_playing);
}

// ─── KmlExporter ────────────────────────────────────────────────────────────

#[test]
fn kml_exporter_creation() {
    let exporter = KmlExporter::new();
    let _ = exporter; // 仅验证其可被创建
}
