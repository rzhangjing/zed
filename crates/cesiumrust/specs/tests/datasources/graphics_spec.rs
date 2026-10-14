//! Graphics 规范 - 参考自 DataSources/*GraphicsSpec
//! 覆盖：PointGraphics、PolylineGraphics、PolygonGraphics、BillboardGraphics、
//! LabelGraphics、ModelGraphics、EllipseGraphics、BoxGraphics、CylinderGraphics、
//! CorridorGraphics、RectangleGraphics、WallGraphics、EllipsoidGraphics、PlaneGraphics、PathGraphics

use cesium_datasource::{
    BillboardGraphics, BoxGraphics, CorridorGraphics, CylinderGraphics, EllipseGraphics,
    EllipsoidGraphics, Entity, LabelGraphics, ModelGraphics, PathGraphics, PlaneGraphics,
    PointGraphics, PolygonGraphics, PolylineGraphics, RectangleGraphics, WallGraphics,
    Property,
};

// ─── PointGraphics ──────────────────────────────────────────────────────────

#[test]
fn point_graphics_default() {
    let g = PointGraphics::default();
    assert_eq!(g.pixel_size.get_value(0.0), Some(&1.0));
    assert_eq!(g.show.get_value(0.0), Some(&true));
}

#[test]
fn point_graphics_custom() {
    let mut g = PointGraphics::default();
    g.pixel_size = Property::Constant(10.0);
    assert_eq!(g.pixel_size.get_value(0.0), Some(&10.0));
}

// ─── PolylineGraphics ───────────────────────────────────────────────────────

#[test]
fn polyline_graphics_default() {
    let g = PolylineGraphics::default();
    assert_eq!(g.width.get_value(0.0), Some(&1.0));
}

#[test]
fn polyline_graphics_positions() {
    let mut g = PolylineGraphics::default();
    g.positions = Property::Constant(vec![
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
    ]);
    let pos = g.positions.get_value(0.0).unwrap();
    assert_eq!(pos.len(), 2);
}

// ─── PolygonGraphics ────────────────────────────────────────────────────────

#[test]
fn polygon_graphics_default_fill() {
    let g = PolygonGraphics::default();
    assert_eq!(g.fill.get_value(0.0), Some(&true));
}

#[test]
fn polygon_graphics_with_holes() {
    let mut g = PolygonGraphics::default();
    g.holes = vec![vec![[0.1, 0.1, 0.0], [0.2, 0.1, 0.0], [0.2, 0.2, 0.0]]];
    assert_eq!(g.holes.len(), 1);
}

// ─── BillboardGraphics ──────────────────────────────────────────────────────

#[test]
fn billboard_graphics_default() {
    let g = BillboardGraphics::default();
    // image 默认为 Undefined → get_value 返回 None
    assert_eq!(g.image.get_value(0.0), None);
    assert_eq!(g.scale.get_value(0.0), Some(&1.0));
}

#[test]
fn billboard_graphics_custom_image() {
    let mut g = BillboardGraphics::default();
    g.image = Property::Constant("test.png".to_string());
    assert_eq!(g.image.get_value(0.0), Some(&"test.png".to_string()));
}

// ─── LabelGraphics ──────────────────────────────────────────────────────────

#[test]
fn label_graphics_default() {
    let g = LabelGraphics::default();
    // text 默认为 Undefined
    assert_eq!(g.text.get_value(0.0), None);
}

#[test]
fn label_graphics_custom_text() {
    let mut g = LabelGraphics::default();
    g.text = Property::Constant("Hello".to_string());
    assert_eq!(g.text.get_value(0.0), Some(&"Hello".to_string()));
}

// ─── ModelGraphics ──────────────────────────────────────────────────────────

#[test]
fn model_graphics_default() {
    let g = ModelGraphics::default();
    // uri 默认为 Undefined
    assert_eq!(g.uri.get_value(0.0), None);
    assert_eq!(g.scale.get_value(0.0), Some(&1.0));
}

// ─── EllipseGraphics ────────────────────────────────────────────────────────

#[test]
fn ellipse_graphics_default() {
    let g = EllipseGraphics::default();
    // semi_major/minor 默认为 Undefined
    assert_eq!(g.semi_major_axis.get_value(0.0), None);
    assert_eq!(g.semi_minor_axis.get_value(0.0), None);
}

#[test]
fn ellipse_graphics_custom() {
    let mut g = EllipseGraphics::default();
    g.semi_major_axis = Property::Constant(500.0);
    g.semi_minor_axis = Property::Constant(300.0);
    assert_eq!(g.semi_major_axis.get_value(0.0), Some(&500.0));
    assert_eq!(g.semi_minor_axis.get_value(0.0), Some(&300.0));
}

// ─── BoxGraphics ────────────────────────────────────────────────────────────

#[test]
fn box_graphics_default() {
    let g = BoxGraphics::default();
    // dimensions 默认为 Undefined
    assert_eq!(g.dimensions.get_value(0.0), None);
    assert_eq!(g.fill.get_value(0.0), Some(&true));
}

#[test]
fn box_graphics_custom_dimensions() {
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([10.0, 20.0, 30.0]);
    assert_eq!(*g.dimensions.get_value(0.0).unwrap(), [10.0, 20.0, 30.0]);
}

// ─── CylinderGraphics ───────────────────────────────────────────────────────

#[test]
fn cylinder_graphics_default() {
    let g = CylinderGraphics::default();
    // length/radii 默认为 Undefined
    assert_eq!(g.length.get_value(0.0), None);
    assert_eq!(g.top_radius.get_value(0.0), None);
    assert_eq!(g.bottom_radius.get_value(0.0), None);
}

// ─── CorridorGraphics ───────────────────────────────────────────────────────

#[test]
fn corridor_graphics_default() {
    let g = CorridorGraphics::default();
    // width 默认为 Undefined
    assert_eq!(g.width.get_value(0.0), None);
}

// ─── RectangleGraphics ──────────────────────────────────────────────────────

#[test]
fn rectangle_graphics_default() {
    let g = RectangleGraphics::default();
    // coordinates 默认为 Undefined
    assert_eq!(g.coordinates.get_value(0.0), None);
}

// ─── WallGraphics ───────────────────────────────────────────────────────────

#[test]
fn wall_graphics_default() {
    let g = WallGraphics::default();
    // positions 默认为 Undefined
    assert_eq!(g.positions.get_value(0.0), None);
}

// ─── EllipsoidGraphics ──────────────────────────────────────────────────────

#[test]
fn ellipsoid_graphics_default() {
    let g = EllipsoidGraphics::default();
    // radii 默认为 Undefined
    assert_eq!(g.radii.get_value(0.0), None);
}

// ─── PlaneGraphics ──────────────────────────────────────────────────────────

#[test]
fn plane_graphics_default() {
    let g = PlaneGraphics::default();
    // dimensions 默认为 Undefined
    assert_eq!(g.dimensions.get_value(0.0), None);
}

// ─── PathGraphics ───────────────────────────────────────────────────────────

#[test]
fn path_graphics_default() {
    let g = PathGraphics::default();
    assert_eq!(g.width.get_value(0.0), Some(&1.0));
}

// ─── Entity has_graphics ────────────────────────────────────────────────────

#[test]
fn entity_no_graphics() {
    let e = Entity::new("test");
    assert!(!e.has_graphics());
}

#[test]
fn entity_with_point_graphics() {
    let mut e = Entity::new("test");
    e.point = Some(PointGraphics::default());
    assert!(e.has_graphics());
}

#[test]
fn entity_with_polyline_graphics() {
    let mut e = Entity::new("test");
    e.polyline = Some(PolylineGraphics::default());
    assert!(e.has_graphics());
}

#[test]
fn entity_with_model_graphics() {
    let mut e = Entity::new("test");
    e.model = Some(ModelGraphics::default());
    assert!(e.has_graphics());
}
