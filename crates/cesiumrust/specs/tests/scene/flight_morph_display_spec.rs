//! 相机飞行/变形 + DataSourceDisplay → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Scene/Camera (flyTo/flyHome/lookAt)
//! - Scene/SceneMode (morphing)
//! - DataSources/DataSourceDisplay
//!
//! A 类测试：飞行插值、变形状态机、显示同步。
//! C 类省略：requestAnimationFrame、DOM 事件、WebGL 渲染。

use cesium_camera::{Camera, SceneMode};
use cesium_interaction::flight::{CameraFlight, FlightOptions, compute_look_at};
use cesium_interaction::morphing::SceneMorph;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::bounding::BoundingSphere;
use glam::DVec3;

// === CameraFlight ===

#[test]
fn flight_fly_to_creates_valid_path() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let destination = DVec3::new(1000000.0, 0.0, 0.0);
    let flight = CameraFlight::fly_to(&camera, destination, None, None, 2.0);

    assert_eq!(flight.start_position, camera.position);
    assert_eq!(flight.end_position, destination);
    assert_eq!(flight.duration, 2.0);
    assert!(!flight.complete);
    assert!((flight.progress() - 0.0).abs() < 1e-10);
}

#[test]
fn flight_update_interpolates_position() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let destination = DVec3::new(10000000.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 2.0);

    // 中途
    let result = flight.update(1.0);
    assert!(result.is_some());
    let progress = flight.progress();
    assert!(progress > 0.4 && progress < 0.6);
    assert!(!flight.complete);

    // 完成
    let _result = flight.update(1.5);
    assert!(flight.complete);
    assert!((flight.progress() - 1.0).abs() < 1e-10);
}

#[test]
fn flight_zero_duration_completes_immediately() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let destination = DVec3::new(1000000.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 0.0);

    let _result = flight.update(0.016);
    assert!(flight.complete);
}

#[test]
fn flight_apply_to_camera_updates_position() {
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let destination = DVec3::new(1000000.0, 0.0, 0.0);
    let mut flight = CameraFlight::fly_to(&camera, destination, None, None, 1.0);

    // 部分应用 - 仍在进行中
    let in_progress = flight.apply_to_camera(&mut camera, 0.5);
    assert!(in_progress);

    // 超过时长应用 - 完成
    flight.apply_to_camera(&mut camera, 1.0);
    assert!(flight.complete);
    // 相机应位于或非常接近目标点
    assert!((camera.position - destination).length() < 1.0);
}

#[test]
fn flight_fly_home() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let ellipsoid = Ellipsoid::WGS84;
    let flight = CameraFlight::fly_home(&camera, &ellipsoid, 3.0);

    assert_eq!(flight.duration, 3.0);
    assert!(!flight.complete);
    // 结束位置应高于椭球表面
    assert!(flight.end_position.length() > ellipsoid.radii().x);
}

#[test]
fn flight_compute_look_at() {
    let target = DVec3::new(1000000.0, 0.0, 0.0);
    let offset = DVec3::new(0.0, 0.0, 500000.0);
    let (position, direction, up) = compute_look_at(target, offset);

    // Position = target + offset
    let expected_pos = target + offset;
    assert!((position - expected_pos).length() < 1e-6);

    // Direction 应从 position 指向 target
    let expected_dir = (target - position).normalize();
    assert!((direction - expected_dir).length() < 1e-6);

    // Up 应垂直于 direction
    assert!(direction.dot(up).abs() < 1e-6);
}

#[test]
fn flight_with_options() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let options = FlightOptions {
        destination: DVec3::new(5000000.0, 0.0, 0.0),
        duration: 5.0,
        ..Default::default()
    };
    let flight = CameraFlight::fly_to_with_options(&camera, &options);
    assert_eq!(flight.duration, 5.0);
    assert_eq!(flight.end_position, options.destination);
}

#[test]
fn flight_fly_to_bounding_sphere() {
    let camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );
    let bs = BoundingSphere::new(DVec3::new(1000000.0, 0.0, 0.0), 50000.0);
    let flight = CameraFlight::fly_to_bounding_sphere(&camera, &bs, None, 2.0);

    assert_eq!(flight.duration, 2.0);
    assert!(!flight.complete);
    // 结束位置应偏离球心
    let dist_to_center = (flight.end_position - bs.center).length();
    assert!(dist_to_center > bs.radius);
}

// === SceneMorph ===

#[test]
fn morph_initial_state() {
    let morph = SceneMorph::new();
    assert!(!morph.is_morphing());
    assert_eq!(morph.progress(), 0.0);
}

#[test]
fn morph_start_and_update() {
    let mut morph = SceneMorph::new();
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );

    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    assert!(morph.is_morphing());

    // 更新到一半
    morph.update(1.0, &mut camera);
    assert!(morph.is_morphing());
    assert!(morph.progress() > 0.0);

    // 超过时长更新
    morph.update(1.5, &mut camera);
    assert!(!morph.is_morphing());
}

#[test]
fn morph_complete() {
    let mut morph = SceneMorph::new();
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );

    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    assert!(morph.is_morphing());

    morph.complete_morph(&mut camera);
    assert!(!morph.is_morphing());
    // 完成后，状态为 Idle，因此 progress() == 0
    assert_eq!(morph.progress(), 0.0);
    // 相机应位于结束位置
    assert_eq!(camera.position, morph.end_position);
}

#[test]
fn morph_cancel() {
    let mut morph = SceneMorph::new();
    let mut camera = Camera::new(
        DVec3::new(0.0, 0.0, 10000000.0),
        -DVec3::Z,
        DVec3::Y,
    );

    morph.start_morph(&camera, SceneMode::Scene3D, SceneMode::Scene2D, &Ellipsoid::WGS84, 2.0);
    morph.update(0.5, &mut camera);
    assert!(morph.is_morphing());

    morph.cancel_morph(&mut camera);
    assert!(!morph.is_morphing());
    // 取消后，状态为 Idle，因此 progress() == 0
    assert_eq!(morph.progress(), 0.0);
    // 相机应恢复到起始位置
    assert_eq!(camera.position, morph.start_position);
}

// === DataSourceDisplay ===

#[test]
fn datasource_display_new() {
    use cesium_datasource::datasource_display::DataSourceDisplay;
    let display = DataSourceDisplay::wgs84();
    assert_eq!(display.billboard_count(), 0);
    assert_eq!(display.label_count(), 0);
    assert_eq!(display.point_count(), 0);
    assert_eq!(display.geometry_instance_count(), 0);
}

#[test]
fn datasource_display_update_with_entities() {
    use cesium_datasource::datasource_display::DataSourceDisplay;
    use cesium_datasource::entity_collection::EntityCollection;
    use cesium_datasource::entity::{Entity, PointGraphics};
    use cesium_datasource::property::Property;

    let mut display = DataSourceDisplay::wgs84();
    let mut entities = EntityCollection::new();

    // 添加带点图形的实体
    let mut entity = Entity::new("test-entity");
    entity.position = Property::Constant([0.0, 0.0, 6378137.0]);
    entity.point = Some(PointGraphics::default());
    entities.add(entity);

    display.update(&entities, 0.0);
    assert_eq!(display.point_count(), 1);
}

#[test]
fn datasource_display_hidden_entity_skipped() {
    use cesium_datasource::datasource_display::DataSourceDisplay;
    use cesium_datasource::entity_collection::EntityCollection;
    use cesium_datasource::entity::{Entity, PointGraphics};
    use cesium_datasource::property::Property;

    let mut display = DataSourceDisplay::wgs84();
    let mut entities = EntityCollection::new();

    let mut entity = Entity::new("hidden");
    entity.show = false;
    entity.position = Property::Constant([0.0, 0.0, 6378137.0]);
    entity.point = Some(PointGraphics::default());
    entities.add(entity);

    display.update(&entities, 0.0);
    assert_eq!(display.point_count(), 0);
}
