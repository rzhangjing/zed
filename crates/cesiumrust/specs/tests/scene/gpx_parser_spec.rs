//! GPX 解析器规范
//! 参考自 CesiumJS DataSources/GpxDataSourceSpec

use cesium_gpx::parser::{
    gpx_to_datasource, parse_gpx_simple, GpxDocument, GpxRoutePoint, GpxTrackPoint, GpxWaypoint,
};

const SIMPLE_GPX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test">
  <metadata>
    <name>Test Document</name>
    <desc>A test GPX file</desc>
  </metadata>
  <wpt lat="40.7128" lon="-74.0060">
    <ele>10.5</ele>
    <name>New York</name>
    <cmt>A city</cmt>
    <desc>Big Apple</desc>
    <sym>City</sym>
    <time>2024-01-01T00:00:00Z</time>
  </wpt>
  <wpt lat="34.0522" lon="-118.2437">
    <name>Los Angeles</name>
  </wpt>
  <trk>
    <name>Morning Run</name>
    <trkseg>
      <trkpt lat="40.0" lon="-74.0">
        <ele>5.0</ele>
        <time>2024-01-01T06:00:00Z</time>
      </trkpt>
      <trkpt lat="40.001" lon="-74.001">
        <ele>6.0</ele>
      </trkpt>
      <trkpt lat="40.002" lon="-74.002"/>
    </trkseg>
  </trk>
  <rte>
    <name>Route 66</name>
    <rtept lat="35.0" lon="-100.0">
      <ele>100.0</ele>
      <name>Start</name>
    </rtept>
    <rtept lat="36.0" lon="-101.0">
      <name>End</name>
    </rtept>
  </rte>
</gpx>"#;

// ==================== 解析：元数据 ====================

#[test]
fn parse_metadata_name_and_desc() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    assert_eq!(doc.metadata.name.as_deref(), Some("Test Document"));
    assert_eq!(doc.metadata.description.as_deref(), Some("A test GPX file"));
}

// ==================== 解析：航点 ====================

#[test]
fn parse_waypoints_count() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    assert_eq!(doc.waypoints.len(), 2);
}

#[test]
fn parse_waypoint_full_attributes() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    let wpt = &doc.waypoints[0];
    assert!((wpt.latitude - 40.7128).abs() < 1e-10);
    assert!((wpt.longitude - (-74.0060)).abs() < 1e-10);
    assert!((wpt.elevation.unwrap() - 10.5).abs() < 1e-10);
    assert_eq!(wpt.name.as_deref(), Some("New York"));
    assert_eq!(wpt.comment.as_deref(), Some("A city"));
    assert_eq!(wpt.description.as_deref(), Some("Big Apple"));
    assert_eq!(wpt.symbol.as_deref(), Some("City"));
    assert_eq!(wpt.time.as_deref(), Some("2024-01-01T00:00:00Z"));
}

#[test]
fn parse_waypoint_minimal() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    let wpt = &doc.waypoints[1];
    assert!((wpt.latitude - 34.0522).abs() < 1e-10);
    assert!((wpt.longitude - (-118.2437)).abs() < 1e-10);
    assert!(wpt.elevation.is_none());
    assert_eq!(wpt.name.as_deref(), Some("Los Angeles"));
}

#[test]
fn waypoint_to_cartographic() {
    let wpt = GpxWaypoint::new(45.0, 90.0);
    let carto = wpt.to_cartographic();
    assert!((carto.longitude - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
    assert!((carto.latitude - std::f64::consts::FRAC_PI_4).abs() < 1e-10);
    assert!((carto.height).abs() < 1e-10);
}

// ==================== 解析：轨迹 ====================

#[test]
fn parse_track_count_and_name() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    assert_eq!(doc.tracks.len(), 1);
    assert_eq!(doc.tracks[0].name.as_deref(), Some("Morning Run"));
}

#[test]
fn parse_track_segment_points() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    let track = &doc.tracks[0];
    assert_eq!(track.segments.len(), 1);
    let seg = &track.segments[0];
    assert_eq!(seg.points.len(), 3);

    let p0 = &seg.points[0];
    assert!((p0.latitude - 40.0).abs() < 1e-10);
    assert!((p0.longitude - (-74.0)).abs() < 1e-10);
    assert!((p0.elevation.unwrap() - 5.0).abs() < 1e-10);
    assert_eq!(p0.time.as_deref(), Some("2024-01-01T06:00:00Z"));

    // 第三个点无高程/时间
    let p2 = &seg.points[2];
    assert!(p2.elevation.is_none());
    assert!(p2.time.is_none());
}

#[test]
fn track_point_to_cartographic() {
    let pt = GpxTrackPoint::new(30.0, 60.0);
    let carto = pt.to_cartographic();
    assert!((carto.latitude - 30.0_f64.to_radians()).abs() < 1e-10);
    assert!((carto.longitude - 60.0_f64.to_radians()).abs() < 1e-10);
}

// ==================== 解析：航线 ====================

#[test]
fn parse_route_count_and_name() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    assert_eq!(doc.routes.len(), 1);
    assert_eq!(doc.routes[0].name.as_deref(), Some("Route 66"));
}

#[test]
fn parse_route_points() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    let route = &doc.routes[0];
    assert_eq!(route.points.len(), 2);

    let p0 = &route.points[0];
    assert!((p0.latitude - 35.0).abs() < 1e-10);
    assert!((p0.longitude - (-100.0)).abs() < 1e-10);
    assert!((p0.elevation.unwrap() - 100.0).abs() < 1e-10);
    assert_eq!(p0.name.as_deref(), Some("Start"));

    let p1 = &route.points[1];
    assert!(p1.elevation.is_none());
    assert_eq!(p1.name.as_deref(), Some("End"));
}

#[test]
fn route_point_to_cartographic() {
    let pt = GpxRoutePoint::new(60.0, 120.0);
    let carto = pt.to_cartographic();
    assert!((carto.latitude - 60.0_f64.to_radians()).abs() < 1e-10);
    assert!((carto.longitude - 120.0_f64.to_radians()).abs() < 1e-10);
}

// ==================== DataSource 转换 ====================

#[test]
fn gpx_to_datasource_name() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    let ds = gpx_to_datasource(&doc);
    assert_eq!(ds.name, "Test Document");
}

#[test]
fn gpx_to_datasource_entity_count() {
    let doc = parse_gpx_simple(SIMPLE_GPX).unwrap();
    let ds = gpx_to_datasource(&doc);
    // 2 航点 + 1 轨迹段 + 1 航线 = 4 个实体
    assert_eq!(ds.entities.len(), 4);
}

#[test]
fn gpx_to_datasource_default_name() {
    let doc = GpxDocument::default();
    let ds = gpx_to_datasource(&doc);
    assert_eq!(ds.name, "GPX");
}

// ==================== 边缘情形 ====================

#[test]
fn parse_empty_gpx() {
    let xml = r#"<gpx version="1.1"></gpx>"#;
    let doc = parse_gpx_simple(xml).unwrap();
    assert!(doc.waypoints.is_empty());
    assert!(doc.tracks.is_empty());
    assert!(doc.routes.is_empty());
}

#[test]
fn parse_waypoint_missing_lat_errors() {
    let xml = r#"<gpx><wpt lon="10.0"><name>Bad</name></wpt></gpx>"#;
    let result = parse_gpx_simple(xml);
    assert!(result.is_err());
}
