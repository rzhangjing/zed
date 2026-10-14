//! DataSources/GeoJsonDataSourceSpec → Rust integration tests

use cesium_datasource::geojson::{parse_geojson, GeoJsonOptions};

fn default_options() -> GeoJsonOptions {
    GeoJsonOptions::default()
}

// === Point ===

#[test]
fn test_geojson_point() {
    let json = r#"{
        "type": "Point",
        "coordinates": [100.0, 0.0]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 1);
    let entity = ds.entities.values().next().unwrap();
    assert!(entity.point.is_some());
    assert!(entity.position.is_defined());
}

#[test]
fn test_geojson_point_with_altitude() {
    let json = r#"{
        "type": "Point",
        "coordinates": [100.0, 0.0, 500.0]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 1);
    let entity = ds.entities.values().next().unwrap();
    assert!(entity.point.is_some());
}

// === MultiPoint ===

#[test]
fn test_geojson_multipoint() {
    let json = r#"{
        "type": "MultiPoint",
        "coordinates": [[100.0, 0.0], [101.0, 1.0]]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 2);
    for entity in ds.entities.values() {
        assert!(entity.point.is_some());
    }
}

// === LineString ===

#[test]
fn test_geojson_linestring() {
    let json = r#"{
        "type": "LineString",
        "coordinates": [[100.0, 0.0], [101.0, 1.0], [102.0, 0.0]]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 1);
    let entity = ds.entities.values().next().unwrap();
    assert!(entity.polyline.is_some());
}

// === MultiLineString ===

#[test]
fn test_geojson_multilinestring() {
    let json = r#"{
        "type": "MultiLineString",
        "coordinates": [
            [[100.0, 0.0], [101.0, 1.0]],
            [[102.0, 2.0], [103.0, 3.0]]
        ]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 2);
    for entity in ds.entities.values() {
        assert!(entity.polyline.is_some());
    }
}

// === Polygon ===

#[test]
fn test_geojson_polygon() {
    let json = r#"{
        "type": "Polygon",
        "coordinates": [[[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 1.0], [100.0, 0.0]]]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 1);
    let entity = ds.entities.values().next().unwrap();
    assert!(entity.polygon.is_some());
}

#[test]
fn test_geojson_polygon_with_hole() {
    let json = r#"{
        "type": "Polygon",
        "coordinates": [
            [[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 1.0], [100.0, 0.0]],
            [[100.2, 0.2], [100.8, 0.2], [100.8, 0.8], [100.2, 0.8], [100.2, 0.2]]
        ]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 1);
    let entity = ds.entities.values().next().unwrap();
    assert!(entity.polygon.is_some());
}

// === MultiPolygon ===

#[test]
fn test_geojson_multipolygon() {
    let json = r#"{
        "type": "MultiPolygon",
        "coordinates": [
            [[[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 0.0]]],
            [[[102.0, 2.0], [103.0, 2.0], [103.0, 3.0], [102.0, 2.0]]]
        ]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 2);
    for entity in ds.entities.values() {
        assert!(entity.polygon.is_some());
    }
}

// === Feature ===

#[test]
fn test_geojson_feature() {
    let json = r#"{
        "type": "Feature",
        "geometry": {
            "type": "Point",
            "coordinates": [102.0, 0.5]
        },
        "properties": {
            "name": "Test Point"
        }
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 1);
    let entity = ds.entities.values().next().unwrap();
    assert!(entity.point.is_some());
    assert_eq!(entity.name.as_deref(), Some("Test Point"));
}

#[test]
fn test_geojson_feature_null_geometry() {
    let json = r#"{
        "type": "Feature",
        "geometry": null,
        "properties": {"name": "no geom"}
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 0);
}

// === FeatureCollection ===

#[test]
fn test_geojson_feature_collection() {
    let json = r#"{
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "geometry": {"type": "Point", "coordinates": [100.0, 0.0]},
                "properties": {"name": "A"}
            },
            {
                "type": "Feature",
                "geometry": {"type": "LineString", "coordinates": [[101.0, 0.0], [102.0, 1.0]]},
                "properties": {"name": "B"}
            },
            {
                "type": "Feature",
                "geometry": {"type": "Polygon", "coordinates": [[[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 0.0]]]},
                "properties": {"name": "C"}
            }
        ]
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 3);
}

// === Options ===

#[test]
fn test_geojson_options_default() {
    let opts = GeoJsonOptions::default();
    assert!((opts.marker_size - 8.0).abs() < 1e-10);
    assert!((opts.stroke_width - 2.0).abs() < 1e-10);
    assert!(!opts.clamp_to_ground);
}

#[test]
fn test_geojson_custom_options() {
    let opts = GeoJsonOptions {
        clamp_to_ground: true,
        ..Default::default()
    };
    let json = r#"{"type": "Point", "coordinates": [0.0, 0.0]}"#;
    let ds = parse_geojson(json, &opts).unwrap();
    assert_eq!(ds.entities.len(), 1);
}

// === Error handling ===

#[test]
fn test_geojson_invalid_json() {
    let result = parse_geojson("not json at all", &default_options());
    assert!(result.is_err());
}

#[test]
fn test_geojson_empty_feature_collection() {
    let json = r#"{"type": "FeatureCollection", "features": []}"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.entities.len(), 0);
}

// === DataSource metadata ===

#[test]
fn test_geojson_datasource_loaded() {
    let json = r#"{"type": "Point", "coordinates": [0.0, 0.0]}"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert!(ds.loaded);
}

#[test]
fn test_geojson_datasource_name() {
    let json = r#"{"type": "Point", "coordinates": [0.0, 0.0]}"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    assert_eq!(ds.name, "GeoJSON");
}

// === Feature properties ===

#[test]
fn test_geojson_feature_properties_stored() {
    let json = r#"{
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [0.0, 0.0]},
        "properties": {"population": 1000, "country": "Test"}
    }"#;
    let ds = parse_geojson(json, &default_options()).unwrap();
    let entity = ds.entities.values().next().unwrap();
    assert!(entity.properties.contains_key("population"));
    assert!(entity.properties.contains_key("country"));
}
