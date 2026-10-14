//! GeometryUpdater 扩展规范 - outline/show/dispatch/plane/polyline_volume
//! 参考自 DataSources/*GeometryUpdaterSpec（outline、show、material 路径）

use cesium_datasource::geometry_updater::{
    update_box_graphics, update_corridor_graphics, update_cylinder_graphics,
    update_ellipse_graphics, update_ellipsoid_graphics, update_entity_geometry,
    update_plane_graphics, update_polyline_graphics, update_polyline_volume_graphics,
    update_rectangle_graphics, update_wall_graphics, EntityGeometry,
};
use cesium_datasource::{
    BoxGraphics, CorridorGraphics, CylinderGraphics, EllipseGraphics, EllipsoidGraphics, Entity,
    PlaneDef, PlaneGraphics, PolylineGraphics, PolylineVolumeGraphics, Property,
    RectangleGraphics, WallGraphics, NumberProperty,
};
use cesium_geospatial::Ellipsoid;

fn wgs84() -> Ellipsoid {
    Ellipsoid::WGS84
}

fn entity_at_origin() -> Entity {
    let mut e = Entity::new("ext-entity");
    e.position = Property::Constant([0.0, 0.0, 0.0]);
    e
}

// ─── Box 轮廓 ────────────────────────────────────────────────────────────

#[test]
fn box_outline_produces_outline_instances() {
    let entity = entity_at_origin();
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([100.0, 200.0, 300.0]);
    g.outline = Property::Constant(true);
    let result = update_box_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty(), "outline should be generated");
    assert!(result.outline_instances[0].is_outline);
}

#[test]
fn box_outline_color_is_used() {
    let entity = entity_at_origin();
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([10.0, 10.0, 10.0]);
    g.outline = Property::Constant(true);
    g.outline_color = Property::Constant(cesium_datasource::Color::RED);
    let result = update_box_graphics(&entity, &g, 0.0, &wgs84());
    let inst = &result.outline_instances[0];
    assert_eq!(inst.color, cesium_datasource::Color::RED);
}

#[test]
fn box_show_false_returns_empty() {
    let entity = entity_at_origin();
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([100.0, 100.0, 100.0]);
    g.show = Property::Constant(false);
    let result = update_box_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty(), "show=false should produce no geometry");
}

#[test]
fn box_fill_false_no_fill_instances() {
    let entity = entity_at_origin();
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([100.0, 100.0, 100.0]);
    g.fill = Property::Constant(false);
    g.outline = Property::Constant(true);
    let result = update_box_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.fill_instances.is_empty(), "fill=false should skip fill");
    assert!(!result.outline_instances.is_empty(), "outline should still be generated");
}

// ─── Cylinder 轮廓 ───────────────────────────────────────────────────────

#[test]
fn cylinder_outline_produces_outline() {
    let entity = entity_at_origin();
    let mut g = CylinderGraphics::default();
    g.length = NumberProperty::Constant(100.0);
    g.top_radius = NumberProperty::Constant(50.0);
    g.bottom_radius = NumberProperty::Constant(50.0);
    g.outline = Property::Constant(true);
    let result = update_cylinder_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty());
}

#[test]
fn cylinder_show_false_empty() {
    let entity = entity_at_origin();
    let mut g = CylinderGraphics::default();
    g.length = NumberProperty::Constant(100.0);
    g.top_radius = NumberProperty::Constant(50.0);
    g.bottom_radius = NumberProperty::Constant(50.0);
    g.show = Property::Constant(false);
    let result = update_cylinder_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty());
}

// ─── Ellipse 轮廓 ────────────────────────────────────────────────────────

#[test]
fn ellipse_outline_produces_outline() {
    let entity = entity_at_origin();
    let mut g = EllipseGraphics::default();
    g.semi_major_axis = NumberProperty::Constant(100.0);
    g.semi_minor_axis = NumberProperty::Constant(80.0);
    g.outline = Property::Constant(true);
    let result = update_ellipse_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty());
}

// ─── Plane 图元 ─────────────────────────────────────────────────────────

#[test]
fn plane_produces_fill() {
    let entity = entity_at_origin();
    let mut g = PlaneGraphics::default();
    g.plane = Property::Constant(PlaneDef {
        normal: [0.0, 0.0, 1.0],
        distance: 0.0,
    });
    g.dimensions = Property::Constant([100.0, 100.0]);
    let result = update_plane_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.is_empty());
    assert!(!result.fill_instances.is_empty());
}

#[test]
fn plane_no_plane_def_returns_empty() {
    let entity = entity_at_origin();
    let g = PlaneGraphics::default(); // plane = Undefined
    let result = update_plane_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty(), "no plane def should produce no geometry");
}

#[test]
fn plane_no_dimensions_returns_empty() {
    let entity = entity_at_origin();
    let mut g = PlaneGraphics::default();
    g.plane = Property::Constant(PlaneDef {
        normal: [1.0, 0.0, 0.0],
        distance: 0.0,
    });
    // dimensions = Undefined
    let result = update_plane_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty());
}

#[test]
fn plane_outline_produces_outline() {
    let entity = entity_at_origin();
    let mut g = PlaneGraphics::default();
    g.plane = Property::Constant(PlaneDef {
        normal: [0.0, 1.0, 0.0],
        distance: 0.0,
    });
    g.dimensions = Property::Constant([50.0, 50.0]);
    g.outline = Property::Constant(true);
    let result = update_plane_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty());
}

// ─── Polyline 体积 ────────────────────────────────────────────────────────

#[test]
fn polyline_volume_produces_fill() {
    let entity = entity_at_origin();
    let mut g = PolylineVolumeGraphics::default();
    g.positions = Property::Constant(vec![
        [0.0, 0.0, 0.0],
        [0.01, 0.0, 0.0],
        [0.02, 0.0, 0.0],
    ]);
    g.shape = Property::Constant(vec![
        [-5.0, -5.0],
        [5.0, -5.0],
        [5.0, 5.0],
        [-5.0, 5.0],
    ]);
    let result = update_polyline_volume_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.is_empty());
    assert!(!result.fill_instances.is_empty());
}

#[test]
fn polyline_volume_no_shape_returns_empty() {
    let entity = entity_at_origin();
    let mut g = PolylineVolumeGraphics::default();
    g.positions = Property::Constant(vec![[0.0, 0.0, 0.0], [0.01, 0.0, 0.0]]);
    // shape = Undefined
    let result = update_polyline_volume_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty());
}

#[test]
fn polyline_volume_show_false_empty() {
    let entity = entity_at_origin();
    let mut g = PolylineVolumeGraphics::default();
    g.positions = Property::Constant(vec![[0.0, 0.0, 0.0], [0.01, 0.0, 0.0]]);
    g.shape = Property::Constant(vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0]]);
    g.show = Property::Constant(false);
    let result = update_polyline_volume_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty());
}

// ─── Polyline 边缘情形 ────────────────────────────────────────────────────

#[test]
fn polyline_less_than_2_positions_empty() {
    let entity = entity_at_origin();
    let mut g = PolylineGraphics::default();
    g.positions = Property::Constant(vec![[0.0, 0.0, 0.0]]);
    let result = update_polyline_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty(), "single position should produce no geometry");
}

#[test]
fn polyline_show_false_empty() {
    let entity = entity_at_origin();
    let mut g = PolylineGraphics::default();
    g.positions = Property::Constant(vec![[0.0, 0.0, 0.0], [0.01, 0.0, 0.0]]);
    g.show = Property::Constant(false);
    let result = update_polyline_graphics(&entity, &g, 0.0, &wgs84());
    assert!(result.is_empty());
}

// ─── update_entity_geometry 分发器 ──────────────────────────────────────

#[test]
fn dispatcher_entity_show_false_empty() {
    let mut entity = entity_at_origin();
    entity.show = false;
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([100.0, 100.0, 100.0]);
    entity.box_graphics = Some(g);
    let result = update_entity_geometry(&entity, 0.0, &wgs84());
    assert!(result.is_empty(), "entity.show=false should produce no geometry");
}

#[test]
fn dispatcher_box_graphics() {
    let mut entity = entity_at_origin();
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([100.0, 100.0, 100.0]);
    entity.box_graphics = Some(g);
    let result = update_entity_geometry(&entity, 0.0, &wgs84());
    assert!(!result.is_empty());
    assert!(!result.fill_instances.is_empty());
}

#[test]
fn dispatcher_multiple_graphics_combined() {
    let mut entity = entity_at_origin();
    let mut box_g = BoxGraphics::default();
    box_g.dimensions = Property::Constant([100.0, 100.0, 100.0]);
    entity.box_graphics = Some(box_g);

    let mut cyl_g = CylinderGraphics::default();
    cyl_g.length = NumberProperty::Constant(50.0);
    cyl_g.top_radius = NumberProperty::Constant(25.0);
    cyl_g.bottom_radius = NumberProperty::Constant(25.0);
    entity.cylinder = Some(cyl_g);

    let result = update_entity_geometry(&entity, 0.0, &wgs84());
    // box 和 cylinder 均应产生填充实例
    assert!(result.fill_instances.len() >= 2, "should have instances from both graphics");
}

#[test]
fn dispatcher_no_graphics_empty() {
    let entity = entity_at_origin();
    let result = update_entity_geometry(&entity, 0.0, &wgs84());
    assert!(result.is_empty(), "entity with no graphics should produce nothing");
}

// ─── EntityGeometry 辅助 ─────────────────────────────────────────────────

#[test]
fn entity_geometry_default_is_empty() {
    let geo = EntityGeometry::default();
    assert!(geo.is_empty());
    assert_eq!(geo.instance_count(), 0);
}

#[test]
fn entity_geometry_instance_count_sums() {
    let entity = entity_at_origin();
    let mut g = BoxGraphics::default();
    g.dimensions = Property::Constant([10.0, 10.0, 10.0]);
    g.outline = Property::Constant(true);
    let result = update_box_graphics(&entity, &g, 0.0, &wgs84());
    let expected = result.fill_instances.len() + result.outline_instances.len();
    assert_eq!(result.instance_count(), expected);
}

// ─── Corridor/Wall/Rectangle/Ellipsoid 轮廓 ──────────────────────────────

#[test]
fn corridor_outline_produces_outline() {
    let entity = entity_at_origin();
    let mut g = CorridorGraphics::default();
    g.positions = Property::Constant(vec![
        [0.0, 0.0, 0.0],
        [0.01, 0.0, 0.0],
        [0.02, 0.0, 0.0],
    ]);
    g.width = NumberProperty::Constant(100.0);
    g.outline = Property::Constant(true);
    let result = update_corridor_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty());
}

#[test]
fn wall_outline_produces_outline() {
    let entity = entity_at_origin();
    let mut g = WallGraphics::default();
    g.positions = Property::Constant(vec![
        [0.0, 0.0, 0.0],
        [0.01, 0.0, 0.0],
        [0.02, 0.0, 0.0],
    ]);
    g.outline = Property::Constant(true);
    let result = update_wall_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty());
}

#[test]
fn rectangle_outline_produces_outline() {
    let entity = entity_at_origin();
    let mut g = RectangleGraphics::default();
    g.coordinates = Property::Constant([0.0, 0.0, 0.01, 0.01]);
    g.outline = Property::Constant(true);
    let result = update_rectangle_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty());
}

#[test]
fn ellipsoid_outline_produces_outline() {
    let entity = entity_at_origin();
    let mut g = EllipsoidGraphics::default();
    g.radii = Property::Constant([100.0, 100.0, 100.0]);
    g.outline = Property::Constant(true);
    let result = update_ellipsoid_graphics(&entity, &g, 0.0, &wgs84());
    assert!(!result.outline_instances.is_empty());
}
