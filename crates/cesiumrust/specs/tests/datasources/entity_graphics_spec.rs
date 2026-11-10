//! Entity 图元规范 - 构建方法、has_graphics、图元默认值
//! 参考自 DataSources/EntitySpec（图元构造路径）

use cesium_datasource::{
    BillboardGraphics, BoxGraphics, CorridorGraphics, CylinderGraphics,
    EllipsoidGraphics, Entity, LabelGraphics, ModelGraphics, PathGraphics, PlaneDef,
    PlaneGraphics, PointGraphics, PolygonGraphics, PolylineGraphics, PolylineVolumeGraphics,
    Property, RectangleGraphics, WallGraphics, CornerType, ShadowMode,
};

// ─── 构建方法 ────────────────────────────────────────────────────────

#[test]
fn builder_with_point() {
    let e = Entity::new("e1").with_point(PointGraphics::default());
    assert!(e.point.is_some());
    assert!(e.has_graphics());
}

#[test]
fn builder_with_polyline() {
    let e = Entity::new("e2").with_polyline(PolylineGraphics::default());
    assert!(e.polyline.is_some());
    assert!(e.has_graphics());
}

#[test]
fn builder_with_polygon() {
    let e = Entity::new("e3").with_polygon(PolygonGraphics::default());
    assert!(e.polygon.is_some());
}

#[test]
fn builder_with_billboard() {
    let e = Entity::new("e4").with_billboard(BillboardGraphics::default());
    assert!(e.billboard.is_some());
}

#[test]
fn builder_with_label() {
    let e = Entity::new("e5").with_label(LabelGraphics::default());
    assert!(e.label.is_some());
}

#[test]
fn builder_with_model() {
    let e = Entity::new("e6").with_model(ModelGraphics::default());
    assert!(e.model.is_some());
}

#[test]
fn builder_with_box() {
    let e = Entity::new("e7").with_box(BoxGraphics::default());
    assert!(e.box_graphics.is_some());
}

#[test]
fn builder_with_cylinder() {
    let e = Entity::new("e8").with_cylinder(CylinderGraphics::default());
    assert!(e.cylinder.is_some());
}

#[test]
fn builder_with_corridor() {
    let e = Entity::new("e9").with_corridor(CorridorGraphics::default());
    assert!(e.corridor.is_some());
}

#[test]
fn builder_with_rectangle() {
    let e = Entity::new("e10").with_rectangle(RectangleGraphics::default());
    assert!(e.rectangle.is_some());
}

#[test]
fn builder_with_wall() {
    let e = Entity::new("e11").with_wall(WallGraphics::default());
    assert!(e.wall.is_some());
}

#[test]
fn builder_with_ellipsoid() {
    let e = Entity::new("e12").with_ellipsoid(EllipsoidGraphics::default());
    assert!(e.ellipsoid.is_some());
}

#[test]
fn builder_with_plane() {
    let e = Entity::new("e13").with_plane(PlaneGraphics::default());
    assert!(e.plane.is_some());
}

#[test]
fn builder_with_path() {
    let e = Entity::new("e14").with_path(PathGraphics::default());
    assert!(e.path.is_some());
}

#[test]
fn builder_with_polyline_volume() {
    let e = Entity::new("e15").with_polyline_volume(PolylineVolumeGraphics::default());
    assert!(e.polyline_volume.is_some());
}

#[test]
fn builder_with_position() {
    let e = Entity::new("e16").with_position(1.0, 2.0, 3.0);
    match &e.position {
        Property::Constant(v) => {
            assert!((v[0] - 1.0).abs() < 1e-10);
            assert!((v[1] - 2.0).abs() < 1e-10);
            assert!((v[2] - 3.0).abs() < 1e-10);
        }
        _ => panic!("expected Constant position"),
    }
}

#[test]
fn builder_with_name() {
    let e = Entity::new("e17").with_name("My Entity");
    assert_eq!(e.name.as_deref(), Some("My Entity"));
}

#[test]
fn builder_with_property() {
    let e = Entity::new("e18").with_property("key", serde_json::json!(42));
    assert_eq!(e.properties.get("key"), Some(&serde_json::json!(42)));
}

#[test]
fn builder_chaining() {
    let e = Entity::new("chain")
        .with_name("chained")
        .with_position(0.0, 0.0, 0.0)
        .with_point(PointGraphics::default())
        .with_property("custom", serde_json::json!("value"));
    assert_eq!(e.name.as_deref(), Some("chained"));
    assert!(e.point.is_some());
    assert!(e.has_graphics());
    assert_eq!(e.properties.get("custom"), Some(&serde_json::json!("value")));
}

// ─── has_graphics ───────────────────────────────────────────────────────────

#[test]
fn has_graphics_false_for_bare_entity() {
    let e = Entity::new("bare");
    assert!(!e.has_graphics());
}

#[test]
fn has_graphics_true_for_ellipse() {
    let e = Entity::new("ell").with_ellipsoid(EllipsoidGraphics::default());
    assert!(e.has_graphics());
}

// ─── 图元默认值 ──────────────────────────────────────────────────────

#[test]
fn point_graphics_defaults() {
    let g = PointGraphics::default();
    assert_eq!(g.pixel_size, Property::Constant(1.0));
    assert_eq!(g.outline_width, Property::Constant(0.0));
    assert_eq!(g.show, Property::Constant(true));
}

#[test]
fn polyline_graphics_defaults() {
    let g = PolylineGraphics::default();
    assert_eq!(g.width, Property::Constant(1.0));
    assert_eq!(g.show, Property::Constant(true));
}

#[test]
fn polygon_graphics_defaults() {
    let g = PolygonGraphics::default();
    assert_eq!(g.show, Property::Constant(true));
    assert_eq!(g.fill, Property::Constant(true));
    assert_eq!(g.outline, Property::Constant(false));
}

#[test]
fn box_graphics_defaults() {
    let g = BoxGraphics::default();
    assert_eq!(g.fill, Property::Constant(true));
    assert_eq!(g.outline, Property::Constant(false));
    assert_eq!(g.show, Property::Constant(true));
    assert_eq!(g.shadows, ShadowMode::Disabled);
}

#[test]
fn plane_graphics_defaults() {
    let g = PlaneGraphics::default();
    assert_eq!(g.plane, Property::Undefined);
    assert_eq!(g.dimensions, Property::Undefined);
    assert_eq!(g.fill, Property::Constant(true));
    assert_eq!(g.outline, Property::Constant(false));
}

#[test]
fn path_graphics_defaults() {
    let g = PathGraphics::default();
    assert_eq!(g.lead_time, Property::Undefined);
    assert_eq!(g.trail_time, Property::Undefined);
    assert_eq!(g.width, Property::Constant(1.0));
    assert_eq!(g.resolution, Property::Constant(60.0));
    assert_eq!(g.show, Property::Constant(true));
}

#[test]
fn polyline_volume_graphics_defaults() {
    let g = PolylineVolumeGraphics::default();
    assert_eq!(g.positions, Property::Undefined);
    assert_eq!(g.shape, Property::Undefined);
    assert_eq!(g.corner_type, CornerType::Rounded);
    assert_eq!(g.fill, Property::Constant(true));
    assert_eq!(g.outline, Property::Constant(false));
}

#[test]
fn billboard_graphics_defaults() {
    let g = BillboardGraphics::default();
    assert_eq!(g.scale, Property::Constant(1.0));
    assert_eq!(g.show, Property::Constant(true));
    assert_eq!(g.rotation, Property::Constant(0.0));
    assert_eq!(g.color, Property::Constant(cesium_datasource::Color::WHITE));
}

#[test]
fn model_graphics_defaults() {
    let g = ModelGraphics::default();
    assert_eq!(g.scale, Property::Constant(1.0));
    assert_eq!(g.show, Property::Constant(true));
}

// ─── PlaneDef ───────────────────────────────────────────────────────────────

#[test]
fn plane_def_construction() {
    let pd = PlaneDef {
        normal: [0.0, 0.0, 1.0],
        distance: 5.0,
    };
    assert_eq!(pd.normal, [0.0, 0.0, 1.0]);
    assert_eq!(pd.distance, 5.0);
}

#[test]
fn plane_def_clone_eq() {
    let pd = PlaneDef {
        normal: [1.0, 0.0, 0.0],
        distance: -3.0,
    };
    let pd2 = pd.clone();
    assert_eq!(pd, pd2);
}
