//! CZML + GeoJSON 扩展解析测试。
//!
//! 参考 CesiumJS：
//! - DataSources/CzmlDataSourceSpec（带时间标记的位置、材质、颜色）
//! - DataSources/GeoJsonDataSourceSpec（Multi*、GeometryCollection、properties）
//!
//! A 类测试：解析逻辑、坐标转换、实体创建。

use cesium_datasource::czml::parse_czml;
use cesium_datasource::geojson::{parse_geojson, GeoJsonOptions};
use cesium_datasource::property::Property;

// === CZML 扩展 ===

#[test]
fn czml_time_tagged_position() {
    let json = r#"[
        {"id":"document","name":"TestDoc","version":"1.0"},
        {"id":"sat","position":{"cartographicDegrees":[0,100,0,200,100,0,400,100,0]}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    assert_eq!(ds.name, "TestDoc");
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 1);
    let e = &entities[0];
    assert_eq!(e.id, "sat");
    // Position 应有定义（采样或常量）
    assert!(e.position.is_defined());
}

#[test]
fn czml_position_flat_array() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"e1","position":[100.0, 0.0, 500000.0]}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 1);
    let e = &entities[0];
    // 扁平数组 [lon, lat, height] 以度为单位 → 存为弧度
    match &e.position {
        Property::Constant(pos) => {
            assert!((pos[0] - 100.0_f64.to_radians()).abs() < 1e-10);
            assert!((pos[1] - 0.0).abs() < 1e-10);
            assert!((pos[2] - 500000.0).abs() < 1e-6);
        }
        _ => panic!("expected constant position"),
    }
}

#[test]
fn czml_point_with_color() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"p1","point":{"pixelSize":10,"color":{"rgba":[255,0,0,128]}}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let point = e.point.as_ref().unwrap();
    // pixel_size 为 NumberProperty = Property<f64>
    assert_eq!(*point.pixel_size.get_value(0.0).unwrap(), 10.0);
}

#[test]
fn czml_polyline_with_material() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"line1","polyline":{
            "positions":{"cartographicDegrees":[0,0,0, 10,10,0]},
            "width":3.0,
            "material":{"solidColor":{"color":{"rgba":[0,255,0,255]}}}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let polyline = e.polyline.as_ref().unwrap();
    assert_eq!(*polyline.width.get_value(0.0).unwrap(), 3.0);
}

#[test]
fn czml_polygon_with_heights() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"poly1","polygon":{
            "positions":{"cartographicDegrees":[0,0,0, 10,0,0, 10,10,0, 0,10,0]},
            "height":1000.0,
            "extrudedHeight":5000.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let polygon = e.polygon.as_ref().unwrap();
    assert_eq!(*polygon.height.get_value(0.0).unwrap(), 1000.0);
    assert_eq!(*polygon.extruded_height.get_value(0.0).unwrap(), 5000.0);
}

#[test]
fn czml_billboard_with_image() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"bb1","billboard":{
            "image":"http://example.com/icon.png",
            "scale":2.0,
            "rotation":1.57
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let bb = e.billboard.as_ref().unwrap();
    assert_eq!(bb.image.get_value(0.0).unwrap(), "http://example.com/icon.png");
    assert_eq!(*bb.scale.get_value(0.0).unwrap(), 2.0);
    assert!((*bb.rotation.get_value(0.0).unwrap() - 1.57).abs() < 1e-10);
}

#[test]
fn czml_model_with_gltf() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"m1","model":{
            "gltf":"http://example.com/model.glb",
            "scale":1.5,
            "minimumPixelSize":64
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let model = e.model.as_ref().unwrap();
    assert_eq!(model.uri.get_value(0.0).unwrap(), "http://example.com/model.glb");
    assert_eq!(*model.scale.get_value(0.0).unwrap(), 1.5);
    assert_eq!(*model.minimum_pixel_size.get_value(0.0).unwrap(), 64.0);
}

#[test]
fn czml_ellipse_with_axes() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"ell1","position":[0,0,0],"ellipse":{
            "semiMajorAxis":100000.0,
            "semiMinorAxis":50000.0,
            "height":10000.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let ellipse = e.ellipse.as_ref().unwrap();
    assert_eq!(*ellipse.semi_major_axis.get_value(0.0).unwrap(), 100000.0);
    assert_eq!(*ellipse.semi_minor_axis.get_value(0.0).unwrap(), 50000.0);
    assert_eq!(*ellipse.height.get_value(0.0).unwrap(), 10000.0);
}

#[test]
fn czml_box_with_dimensions() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"box1","position":[0,0,0],"box":{
            "dimensions":[100.0, 200.0, 300.0]
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let box_g = e.box_graphics.as_ref().unwrap();
    assert_eq!(*box_g.dimensions.get_value(0.0).unwrap(), [100.0, 200.0, 300.0]);
}

#[test]
fn czml_cylinder_radii() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"cyl1","position":[0,0,0],"cylinder":{
            "length":500.0,
            "topRadius":10.0,
            "bottomRadius":50.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let cyl = e.cylinder.as_ref().unwrap();
    assert_eq!(*cyl.length.get_value(0.0).unwrap(), 500.0);
    assert_eq!(*cyl.top_radius.get_value(0.0).unwrap(), 10.0);
    assert_eq!(*cyl.bottom_radius.get_value(0.0).unwrap(), 50.0);
}

#[test]
fn czml_path_lead_trail() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"path1","path":{
            "leadTime":300.0,
            "trailTime":600.0,
            "width":2.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let path = e.path.as_ref().unwrap();
    assert_eq!(*path.lead_time.get_value(0.0).unwrap(), 300.0);
    assert_eq!(*path.trail_time.get_value(0.0).unwrap(), 600.0);
    assert_eq!(*path.width.get_value(0.0).unwrap(), 2.0);
}

#[test]
fn czml_rectangle_coordinates() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"rect1","rectangle":{
            "coordinates":[-10.0, -20.0, 30.0, 40.0],
            "height":5000.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let rect = e.rectangle.as_ref().unwrap();
    assert_eq!(*rect.height.get_value(0.0).unwrap(), 5000.0);
}

#[test]
fn czml_wall_with_heights() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"wall1","wall":{
            "positions":{"cartographicDegrees":[0,0,0, 10,0,0, 10,10,0]},
            "maximumHeights":[1000,2000,3000],
            "minimumHeights":[0,100,200]
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let wall = e.wall.as_ref().unwrap();
    assert_eq!(*wall.maximum_heights.get_value(0.0).unwrap(), vec![1000.0, 2000.0, 3000.0]);
    assert_eq!(*wall.minimum_heights.get_value(0.0).unwrap(), vec![0.0, 100.0, 200.0]);
}

#[test]
fn czml_ellipsoid_radii() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"ellp1","position":[0,0,0],"ellipsoid":{
            "radii":[100.0, 200.0, 300.0]
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let ellp = e.ellipsoid.as_ref().unwrap();
    assert_eq!(*ellp.radii.get_value(0.0).unwrap(), [100.0, 200.0, 300.0]);
}

#[test]
fn czml_corridor_width_height() {
    let json = r#"[
        {"id":"document","version":"1.0"},
        {"id":"corr1","corridor":{
            "positions":{"cartographicDegrees":[0,0,0, 10,10,0, 20,0,0]},
            "width":50000.0,
            "height":10000.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let e = ds.entities.values().next().unwrap();
    let corridor = e.corridor.as_ref().unwrap();
    assert_eq!(*corridor.width.get_value(0.0).unwrap(), 50000.0);
    assert_eq!(*corridor.height.get_value(0.0).unwrap(), 10000.0);
}

// === GeoJSON 扩展 ===

#[test]
fn geojson_multi_point() {
    let json = r#"{
        "type": "MultiPoint",
        "coordinates": [[100.0, 0.0], [101.0, 1.0], [102.0, 2.0]]
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 3);
}

#[test]
fn geojson_multi_line_string() {
    let json = r#"{
        "type": "MultiLineString",
        "coordinates": [
            [[100.0, 0.0], [101.0, 1.0]],
            [[102.0, 2.0], [103.0, 3.0]]
        ]
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 2);
    for e in &entities {
        assert!(e.polyline.is_some());
    }
}

#[test]
fn geojson_multi_polygon() {
    let json = r#"{
        "type": "MultiPolygon",
        "coordinates": [
            [[[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 1.0], [100.0, 0.0]]],
            [[[102.0, 2.0], [103.0, 2.0], [103.0, 3.0], [102.0, 3.0], [102.0, 2.0]]]
        ]
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 2);
    for e in &entities {
        assert!(e.polygon.is_some());
    }
}

#[test]
fn geojson_geometry_collection() {
    let json = r#"{
        "type": "Feature",
        "properties": {},
        "geometry": {
            "type": "GeometryCollection",
            "geometries": [
                {"type": "Point", "coordinates": [100.0, 0.0]},
                {"type": "LineString", "coordinates": [[101.0, 0.0], [102.0, 1.0]]}
            ]
        }
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 2);
}

#[test]
fn geojson_feature_with_properties() {
    let json = r#"{
        "type": "Feature",
        "properties": {"name": "TestFeature", "population": 1000},
        "geometry": {"type": "Point", "coordinates": [100.0, 0.0]}
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 1);
    let e = &entities[0];
    assert!(e.point.is_some());
}

#[test]
fn geojson_feature_collection_multiple() {
    let json = r#"{
        "type": "FeatureCollection",
        "features": [
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[0,0]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,1]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[2,2]}}
        ]
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    assert_eq!(ds.entities.values().count(), 3);
}

#[test]
fn geojson_polygon_with_hole() {
    let json = r#"{
        "type": "Polygon",
        "coordinates": [
            [[100.0, 0.0], [101.0, 0.0], [101.0, 1.0], [100.0, 1.0], [100.0, 0.0]],
            [[100.2, 0.2], [100.8, 0.2], [100.8, 0.8], [100.2, 0.8], [100.2, 0.2]]
        ]
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 1);
    assert!(entities[0].polygon.is_some());
}

#[test]
fn geojson_custom_options() {
    let json = r#"{"type":"Point","coordinates":[0,0]}"#;
    let options = GeoJsonOptions {
        marker_size: 16.0,
        ..Default::default()
    };
    let ds = parse_geojson(json, &options).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 1);
    let e = &entities[0];
    if let Some(point) = &e.point {
        assert_eq!(*point.pixel_size.get_value(0.0).unwrap(), 16.0);
    }
}

#[test]
fn geojson_line_string_creates_polyline() {
    let json = r#"{
        "type": "LineString",
        "coordinates": [[0.0, 0.0], [10.0, 10.0], [20.0, 0.0]]
    }"#;
    let ds = parse_geojson(json, &GeoJsonOptions::default()).unwrap();
    let entities: Vec<_> = ds.entities.values().collect();
    assert_eq!(entities.len(), 1);
    assert!(entities[0].polyline.is_some());
}

#[test]
fn geojson_invalid_json_error() {
    let result = parse_geojson("not valid json", &GeoJsonOptions::default());
    assert!(result.is_err());
}
