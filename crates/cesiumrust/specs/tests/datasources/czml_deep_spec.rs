//! CZML 深度规范 - 详细属性解析、边缘情形、对象格式
//! 参考自 DataSources/CzmlDataSourceSpec（更深层的 A 类路径）

use cesium_datasource::{parse_czml, Property};

// ─── Rectangle 对象坐标 ───────────────────────────────────────────

#[test]
fn czml_rectangle_object_degrees() {
    let json = r#"[
        {"id": "document", "name": "Rect"},
        {"id": "rect-1", "rectangle": {
            "coordinates": {"degrees": [-10.0, -20.0, 30.0, 40.0]},
            "height": 500.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("rect-1").unwrap();
    assert!(entity.rectangle.is_some());
    let rect = entity.rectangle.as_ref().unwrap();
    let coords = rect.coordinates.get_value(0.0).unwrap();
    // 应被转换为弧度
    assert!((coords[0] - (-10.0f64).to_radians()).abs() < 1e-10);
    assert!((coords[1] - (-20.0f64).to_radians()).abs() < 1e-10);
    assert!((coords[2] - (30.0f64).to_radians()).abs() < 1e-10);
    assert!((coords[3] - (40.0f64).to_radians()).abs() < 1e-10);
    let h = rect.height.get_value(0.0).unwrap();
    assert!((*h - 500.0).abs() < 1e-10);
}

#[test]
fn czml_rectangle_array_format() {
    let json = r#"[
        {"id": "document", "name": "Rect"},
        {"id": "rect-2", "rectangle": {
            "coordinates": [0.0, 0.0, 90.0, 45.0]
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("rect-2").unwrap();
    let rect = entity.rectangle.as_ref().unwrap();
    let coords = rect.coordinates.get_value(0.0).unwrap();
    assert!((coords[2] - (90.0f64).to_radians()).abs() < 1e-10);
}

// ─── 带高度的 Wall ──────────────────────────────────────────────────────

#[test]
fn czml_wall_with_max_min_heights() {
    let json = r#"[
        {"id": "document", "name": "Walls"},
        {"id": "wall-1", "wall": {
            "positions": {"cartographicDegrees": [-75.0, 40.0, 0.0, -74.0, 41.0, 0.0]},
            "maximumHeights": [1000.0, 2000.0],
            "minimumHeights": [0.0, 100.0]
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("wall-1").unwrap();
    assert!(entity.wall.is_some());
    let wall = entity.wall.as_ref().unwrap();
    let max_h = wall.maximum_heights.get_value(0.0).unwrap();
    assert_eq!(max_h.len(), 2);
    assert!((max_h[0] - 1000.0).abs() < 1e-10);
    let min_h = wall.minimum_heights.get_value(0.0).unwrap();
    assert!((min_h[1] - 100.0).abs() < 1e-10);
}

// ─── 带半径的 Ellipsoid（cartesian3 对象） ───────────────────────────────

#[test]
fn czml_ellipsoid_radii_object() {
    let json = r#"[
        {"id": "document", "name": "Ellipsoids"},
        {"id": "ell-1", "position": {"cartographicDegrees": [-75.0, 40.0, 0.0]},
         "ellipsoid": {"radii": {"cartesian3": [100.0, 200.0, 300.0]}}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("ell-1").unwrap();
    assert!(entity.ellipsoid.is_some());
    let ell = entity.ellipsoid.as_ref().unwrap();
    let radii = ell.radii.get_value(0.0).unwrap();
    assert_eq!(*radii, [100.0, 200.0, 300.0]);
}

#[test]
fn czml_ellipsoid_radii_array() {
    let json = r#"[
        {"id": "document", "name": "Ellipsoids"},
        {"id": "ell-2", "ellipsoid": {"radii": [50.0, 50.0, 50.0]}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("ell-2").unwrap();
    let ell = entity.ellipsoid.as_ref().unwrap();
    let radii = ell.radii.get_value(0.0).unwrap();
    assert_eq!(*radii, [50.0, 50.0, 50.0]);
}

// ─── 带高度的 Corridor ───────────────────────────────────────────────────

#[test]
fn czml_corridor_with_height_and_width() {
    let json = r#"[
        {"id": "document", "name": "Corridors"},
        {"id": "cor-1", "corridor": {
            "positions": {"cartographicDegrees": [-75.0, 40.0, 0.0, -74.0, 41.0, 0.0, -73.0, 40.5, 0.0]},
            "width": 200.0,
            "height": 1000.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("cor-1").unwrap();
    assert!(entity.corridor.is_some());
    let cor = entity.corridor.as_ref().unwrap();
    let w = cor.width.get_value(0.0).unwrap();
    assert!((*w - 200.0).abs() < 1e-10);
    let h = cor.height.get_value(0.0).unwrap();
    assert!((*h - 1000.0).abs() < 1e-10);
    let positions = cor.positions.get_value(0.0).unwrap();
    assert_eq!(positions.len(), 3);
}

// ─── 带高度和材质的 Ellipse ───────────────────────────────────────

#[test]
fn czml_ellipse_with_height_and_material() {
    let json = r#"[
        {"id": "document", "name": "Ellipses"},
        {"id": "ell-1", "position": {"cartographicDegrees": [-75.0, 40.0, 0.0]},
         "ellipse": {
            "semiMajorAxis": 500.0,
            "semiMinorAxis": 300.0,
            "height": 200.0,
            "material": {"solidColor": {"color": {"rgba": [0, 0, 255, 200]}}}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("ell-1").unwrap();
    let ell = entity.ellipse.as_ref().unwrap();
    assert!((*ell.semi_major_axis.get_value(0.0).unwrap() - 500.0).abs() < 1e-10);
    assert!((*ell.semi_minor_axis.get_value(0.0).unwrap() - 300.0).abs() < 1e-10);
    assert!((*ell.height.get_value(0.0).unwrap() - 200.0).abs() < 1e-10);
    // 应提取材质颜色
    let mat = ell.material.get_value(0.0).unwrap();
    assert!((mat.blue - 1.0).abs() < 1e-10);
    assert!((mat.alpha - 200.0 / 255.0).abs() < 1e-10);
}

// ─── Position 对象格式 ─────────────────────────────────────────────────

#[test]
fn czml_position_object_format() {
    let json = r#"[
        {"id": "document", "name": "Pos"},
        {"id": "e1", "position": {"cartographicDegrees": [-122.0, 37.0, 100.0]}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("e1").unwrap();
    match &entity.position {
        Property::Constant(v) => {
            assert!((v[0] - (-122.0f64).to_radians()).abs() < 1e-10);
            assert!((v[1] - (37.0f64).to_radians()).abs() < 1e-10);
            assert!((v[2] - 100.0).abs() < 1e-10);
        }
        _ => panic!("expected Constant position"),
    }
}

// ─── Color 边缘情形 ───────────────────────────────────────────────────────

#[test]
fn czml_point_color_object_format() {
    let json = r#"[
        {"id": "document", "name": "Colors"},
        {"id": "p1", "point": {
            "color": {"rgba": [128, 64, 32, 255]},
            "pixelSize": 5
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("p1").unwrap();
    let pt = entity.point.as_ref().unwrap();
    let color = pt.color.get_value(0.0).unwrap();
    assert!((color.red - 128.0 / 255.0).abs() < 1e-10);
    assert!((color.green - 64.0 / 255.0).abs() < 1e-10);
    assert!((color.blue - 32.0 / 255.0).abs() < 1e-10);
}

#[test]
fn czml_point_outline_color() {
    let json = r#"[
        {"id": "document", "name": "Outline"},
        {"id": "p1", "point": {
            "outlineColor": {"rgba": [255, 0, 0, 255]},
            "outlineWidth": 3.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("p1").unwrap();
    let pt = entity.point.as_ref().unwrap();
    let oc = pt.outline_color.get_value(0.0).unwrap();
    assert!((oc.red - 1.0).abs() < 1e-10);
    let ow = pt.outline_width.get_value(0.0).unwrap();
    assert!((*ow - 3.0).abs() < 1e-10);
}

// ─── 单个实体上的多个图元 ────────────────────────────────────────

#[test]
fn czml_entity_with_multiple_graphics() {
    let json = r#"[
        {"id": "document", "name": "Multi"},
        {"id": "multi-1",
         "position": {"cartographicDegrees": [-75.0, 40.0, 0.0]},
         "point": {"pixelSize": 5},
         "label": {"text": "Hello"},
         "billboard": {"image": "icon.png"}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("multi-1").unwrap();
    assert!(entity.point.is_some());
    assert!(entity.label.is_some());
    assert!(entity.billboard.is_some());
    assert!(entity.has_graphics());
}

// ─── 带材质的 Box ──────────────────────────────────────────────────────

#[test]
fn czml_box_with_material_color() {
    let json = r#"[
        {"id": "document", "name": "BoxMat"},
        {"id": "box-1", "box": {
            "dimensions": {"cartesian3": [10.0, 20.0, 30.0]},
            "material": {"solidColor": {"color": {"rgba": [0, 255, 128, 255]}}}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("box-1").unwrap();
    let bx = entity.box_graphics.as_ref().unwrap();
    let mat = bx.material.get_value(0.0).unwrap();
    assert!((mat.green - 1.0).abs() < 1e-10);
    assert!((mat.blue - 128.0 / 255.0).abs() < 1e-10);
}

// ─── 带材质的 Cylinder ─────────────────────────────────────────────────

#[test]
fn czml_cylinder_with_material() {
    let json = r#"[
        {"id": "document", "name": "CylMat"},
        {"id": "cyl-1", "cylinder": {
            "length": 100.0,
            "topRadius": 10.0,
            "bottomRadius": 20.0,
            "material": {"solidColor": {"color": {"rgba": [255, 128, 0, 255]}}}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("cyl-1").unwrap();
    let cyl = entity.cylinder.as_ref().unwrap();
    assert!((*cyl.top_radius.get_value(0.0).unwrap() - 10.0).abs() < 1e-10);
    assert!((*cyl.bottom_radius.get_value(0.0).unwrap() - 20.0).abs() < 1e-10);
    let mat = cyl.material.get_value(0.0).unwrap();
    assert!((mat.red - 1.0).abs() < 1e-10);
}

// ─── 带材质的 Path ─────────────────────────────────────────────────────

#[test]
fn czml_path_with_material() {
    let json = r#"[
        {"id": "document", "name": "PathMat"},
        {"id": "path-1", "path": {
            "leadTime": 1800,
            "trailTime": 3600,
            "width": 2.0,
            "material": {"solidColor": {"color": {"rgba": [255, 255, 0, 255]}}}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("path-1").unwrap();
    let path = entity.path.as_ref().unwrap();
    assert!((*path.lead_time.get_value(0.0).unwrap() - 1800.0).abs() < 1e-10);
    assert!((*path.trail_time.get_value(0.0).unwrap() - 3600.0).abs() < 1e-10);
    let mat = path.material.get_value(0.0).unwrap();
    assert!((mat.red - 1.0).abs() < 1e-10);
    assert!((mat.green - 1.0).abs() < 1e-10);
}

// ─── 空/最简数据包 ──────────────────────────────────────────────────

#[test]
fn czml_entity_no_graphics() {
    let json = r#"[
        {"id": "document", "name": "Empty"},
        {"id": "empty-1", "name": "No Graphics"}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("empty-1").unwrap();
    assert!(!entity.has_graphics());
    assert_eq!(entity.name.as_deref(), Some("No Graphics"));
}

#[test]
fn czml_document_only() {
    let json = r#"[{"id": "document", "name": "Just Doc", "version": "1.0"}]"#;
    let ds = parse_czml(json).unwrap();
    assert_eq!(ds.name, "Just Doc");
    assert_eq!(ds.entities.len(), 0);
}

// ─── 带 rotation/width/height 的 Billboard ───────────────────────────────────

#[test]
fn czml_billboard_full_properties() {
    let json = r#"[
        {"id": "document", "name": "BB"},
        {"id": "bb-1", "billboard": {
            "image": "marker.png",
            "scale": 1.5,
            "rotation": 0.785,
            "width": 32.0,
            "height": 48.0,
            "color": {"rgba": [200, 100, 50, 255]}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("bb-1").unwrap();
    let bb = entity.billboard.as_ref().unwrap();
    assert!((*bb.scale.get_value(0.0).unwrap() - 1.5).abs() < 1e-10);
    assert!((*bb.rotation.get_value(0.0).unwrap() - 0.785).abs() < 1e-10);
    assert!((*bb.width.get_value(0.0).unwrap() - 32.0).abs() < 1e-10);
    assert!((*bb.height.get_value(0.0).unwrap() - 48.0).abs() < 1e-10);
}

// ─── 带 minimumPixelSize 的 Model ────────────────────────────────────────────

#[test]
fn czml_model_with_minimum_pixel_size() {
    let json = r#"[
        {"id": "document", "name": "Model"},
        {"id": "m-1", "model": {
            "gltf": "building.glb",
            "scale": 5.0,
            "minimumPixelSize": 64.0
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("m-1").unwrap();
    let model = entity.model.as_ref().unwrap();
    assert_eq!(model.uri.get_value(0.0).unwrap(), "building.glb");
    assert!((*model.scale.get_value(0.0).unwrap() - 5.0).abs() < 1e-10);
    assert!((*model.minimum_pixel_size.get_value(0.0).unwrap() - 64.0).abs() < 1e-10);
}

// ─── Polyline 材质颜色提取 ─────────────────────────────────────

#[test]
fn czml_polyline_material_color() {
    let json = r#"[
        {"id": "document", "name": "Line"},
        {"id": "l-1", "polyline": {
            "positions": {"cartographicDegrees": [0.0, 0.0, 0.0, 1.0, 1.0, 0.0]},
            "material": {"solidColor": {"color": {"rgba": [128, 0, 255, 255]}}}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("l-1").unwrap();
    let pl = entity.polyline.as_ref().unwrap();
    let color = pl.color.get_value(0.0).unwrap();
    assert!((color.red - 128.0 / 255.0).abs() < 1e-10);
    assert!((color.blue - 1.0).abs() < 1e-10);
}

// ─── Polygon 材质颜色 ─────────────────────────────────────────────────

#[test]
fn czml_polygon_material_color() {
    let json = r#"[
        {"id": "document", "name": "Poly"},
        {"id": "pg-1", "polygon": {
            "positions": {"cartographicDegrees": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0]},
            "material": {"solidColor": {"color": {"rgba": [0, 128, 64, 200]}}}
        }}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("pg-1").unwrap();
    let pg = entity.polygon.as_ref().unwrap();
    let mat = pg.material.get_value(0.0).unwrap();
    assert!((mat.green - 128.0 / 255.0).abs() < 1e-10);
    assert!((mat.alpha - 200.0 / 255.0).abs() < 1e-10);
}
