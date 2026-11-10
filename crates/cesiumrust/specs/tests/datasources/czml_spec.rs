//! DataSources/CzmlDataSourceSpec → Rust 集成测试

use cesium_datasource::czml::parse_czml;

// === 基本解析 ===

#[test]
fn test_czml_document_packet_only() {
    let json = r#"[
        {"id": "document", "name": "Test Doc", "version": "1.0"}
    ]"#;
    let ds = parse_czml(json).unwrap();
    assert_eq!(ds.name, "Test Doc");
    assert_eq!(ds.entities.len(), 0);
    assert!(ds.loaded);
}

#[test]
fn test_czml_missing_document() {
    // 无 document 数据包的 CZML 仍应能解析（我们的实现中 document 可选）
    let json = r#"[
        {"id": "entity1", "name": "Test"}
    ]"#;
    let ds = parse_czml(json).unwrap();
    assert_eq!(ds.entities.len(), 1);
}

#[test]
fn test_czml_invalid_json() {
    let result = parse_czml("not valid json");
    assert!(result.is_err());
}

// === Position ===

#[test]
fn test_czml_entity_with_position() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "sat1", "position": {"cartographicDegrees": [10.0, 20.0, 300.0]}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    assert_eq!(ds.entities.len(), 1);
    let entity = ds.entities.get("sat1").unwrap();
    assert!(entity.position.is_defined());
}

#[test]
fn test_czml_entity_with_position_array() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "sat1", "position": [10.0, 20.0, 300.0]}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("sat1").unwrap();
    assert!(entity.position.is_defined());
}

// === Point ===

#[test]
fn test_czml_entity_with_point() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "p1", "point": {"pixelSize": 10.0, "color": {"rgba": [255, 0, 0, 255]}}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("p1").unwrap();
    assert!(entity.point.is_some());
}

// === Polyline ===

#[test]
fn test_czml_entity_with_polyline() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "line1", "polyline": {"positions": {"cartographicDegrees": [0.0, 0.0, 0.0, 10.0, 10.0, 0.0]}, "width": 3.0}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("line1").unwrap();
    assert!(entity.polyline.is_some());
}

// === Polygon ===

#[test]
fn test_czml_entity_with_polygon() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "poly1", "polygon": {"positions": {"cartographicDegrees": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0]}}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("poly1").unwrap();
    assert!(entity.polygon.is_some());
}

// === Label ===

#[test]
fn test_czml_entity_with_label() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "label1", "label": {"text": "Hello", "font": "12px sans-serif"}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("label1").unwrap();
    assert!(entity.label.is_some());
}

// === Billboard ===

#[test]
fn test_czml_entity_with_billboard() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "bb1", "billboard": {"image": "test.png", "scale": 2.0}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("bb1").unwrap();
    assert!(entity.billboard.is_some());
}

// === Model ===

#[test]
fn test_czml_entity_with_model() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "model1", "model": {"gltf": "model.glb", "scale": 1.5}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("model1").unwrap();
    assert!(entity.model.is_some());
}

// === Ellipse ===

#[test]
fn test_czml_entity_with_ellipse() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "ell1", "ellipse": {"semiMajorAxis": 100.0, "semiMinorAxis": 50.0}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("ell1").unwrap();
    assert!(entity.ellipse.is_some());
}

// === Box ===

#[test]
fn test_czml_entity_with_box() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "box1", "box": {"dimensions": [100.0, 200.0, 300.0]}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("box1").unwrap();
    assert!(entity.box_graphics.is_some());
}

// === Cylinder ===

#[test]
fn test_czml_entity_with_cylinder() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "cyl1", "cylinder": {"length": 500.0, "topRadius": 50.0, "bottomRadius": 100.0}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("cyl1").unwrap();
    assert!(entity.cylinder.is_some());
}

// === Corridor ===

#[test]
fn test_czml_entity_with_corridor() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "cor1", "corridor": {"positions": {"cartographicDegrees": [0.0, 0.0, 0.0, 1.0, 1.0, 0.0]}, "width": 200.0}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("cor1").unwrap();
    assert!(entity.corridor.is_some());
}

// === Rectangle ===

#[test]
fn test_czml_entity_with_rectangle() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "rect1", "rectangle": {"coordinates": [-10.0, -10.0, 10.0, 10.0]}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("rect1").unwrap();
    assert!(entity.rectangle.is_some());
}

// === Wall ===

#[test]
fn test_czml_entity_with_wall() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "wall1", "wall": {"positions": {"cartographicDegrees": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0]}}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("wall1").unwrap();
    assert!(entity.wall.is_some());
}

// === Ellipsoid ===

#[test]
fn test_czml_entity_with_ellipsoid() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "ellipsoid1", "ellipsoid": {"radii": [100.0, 200.0, 300.0]}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("ellipsoid1").unwrap();
    assert!(entity.ellipsoid.is_some());
}

// === Path ===

#[test]
fn test_czml_entity_with_path() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "path1", "path": {"leadTime": 3600.0, "trailTime": 7200.0}}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("path1").unwrap();
    assert!(entity.path.is_some());
}

// === Name 与 description ===

#[test]
fn test_czml_entity_name() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "e1", "name": "My Entity"}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("e1").unwrap();
    assert_eq!(entity.name.as_deref(), Some("My Entity"));
}

#[test]
fn test_czml_entity_description() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "e1", "description": "A test entity"}
    ]"#;
    let ds = parse_czml(json).unwrap();
    let entity = ds.entities.get("e1").unwrap();
    assert_eq!(entity.description.as_deref(), Some("A test entity"));
}

// === 多个实体 ===

#[test]
fn test_czml_multiple_entities() {
    let json = r#"[
        {"id": "document", "version": "1.0"},
        {"id": "e1", "name": "First"},
        {"id": "e2", "name": "Second"},
        {"id": "e3", "name": "Third"}
    ]"#;
    let ds = parse_czml(json).unwrap();
    assert_eq!(ds.entities.len(), 3);
    assert!(ds.entities.contains("e1"));
    assert!(ds.entities.contains("e2"));
    assert!(ds.entities.contains("e3"));
}

// === 从 document 获取的 DataSource 名称 ===

#[test]
fn test_czml_datasource_name_from_document() {
    let json = r#"[
        {"id": "document", "name": "My Scene", "version": "1.0"}
    ]"#;
    let ds = parse_czml(json).unwrap();
    assert_eq!(ds.name, "My Scene");
}
