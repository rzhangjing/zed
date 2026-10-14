//! 参考自 CesiumJS EntitySpec 的测试（13 个 A 类测试）
//! - constructor/isAvailable/merge/computeModelMatrix/addProperty/removeProperty

use cesium_datasource::entity::Entity;
use cesium_datasource::property::Property;
use cesium_geospatial::Ellipsoid;
use cesium_time::{JulianDate, TimeInterval, TimeIntervalCollection, TimeIntervalData};

// ===== 构造 =====

#[test]
fn test_constructor_sets_expected_properties() {
    let entity = Entity::new("test-id")
        .with_name("Test Name")
        .with_position(0.1, 0.2, 100.0);

    assert_eq!(entity.id, "test-id");
    assert_eq!(entity.name, Some("Test Name".to_string()));
    assert!(entity.show);
    assert!(matches!(entity.position, Property::Constant(_)));
}

#[test]
fn test_constructor_creates_unique_id() {
    // 在 Rust 中我们始终要求一个 ID，但可测试不同实体有不同 ID
    let e1 = Entity::new("id-1");
    let e2 = Entity::new("id-2");
    assert_ne!(e1.id, e2.id);
}

// ===== isAvailable =====

#[test]
fn test_is_available_always_true_if_no_availability() {
    let entity = Entity::new("test");
    let time = JulianDate::from_date_components(2020, 1, 1, 0, 0, 0, 0.0);
    assert!(entity.is_available(&time));
}

#[test]
fn test_is_available_works() {
    let mut entity = Entity::new("test");

    // 创建可用性区间: [2020-01-01, 2020-12-31]
    let start = JulianDate::from_date_components(2020, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2020, 12, 31, 23, 59, 59, 0.0);
    let interval = TimeInterval::new(start, stop, true, true);
    let mut tic: TimeIntervalCollection<()> = TimeIntervalCollection::new();
    tic.add_interval(TimeIntervalData::new(interval, None), &|_, _| true);
    entity.availability = Some(tic);

    // 时间在可用性内
    let inside = JulianDate::from_date_components(2020, 6, 15, 12, 0, 0, 0.0);
    assert!(entity.is_available(&inside));

    // 时间在可用性外
    let outside = JulianDate::from_date_components(2021, 6, 15, 12, 0, 0, 0.0);
    assert!(!entity.is_available(&outside));
}

// ===== addProperty / removeProperty =====

#[test]
fn test_can_add_and_remove_custom_properties() {
    let mut entity = Entity::new("test");
    entity.add_property("population", serde_json::json!(1000000));
    entity.add_property("country", serde_json::json!("USA"));

    assert_eq!(entity.properties.len(), 2);
    assert_eq!(entity.properties["population"], serde_json::json!(1000000));
    assert_eq!(entity.properties["country"], serde_json::json!("USA"));

    let removed = entity.remove_property("population");
    assert_eq!(removed, Some(serde_json::json!(1000000)));
    assert_eq!(entity.properties.len(), 1);
    assert!(!entity.properties.contains_key("population"));
}

#[test]
fn test_can_re_add_removed_properties() {
    let mut entity = Entity::new("test");
    entity.add_property("key", serde_json::json!("value1"));
    entity.remove_property("key");
    assert!(!entity.properties.contains_key("key"));

    entity.add_property("key", serde_json::json!("value2"));
    assert_eq!(entity.properties["key"], serde_json::json!("value2"));
}

// ===== merge =====

#[test]
fn test_merge_ignores_reserved_property_names() {
    let mut target = Entity::new("target").with_name("Target Name");
    let source = Entity::new("source").with_name("Source Name");

    target.merge(&source);
    // name 不应被覆盖
    assert_eq!(target.name, Some("Target Name".to_string()));
    // ID 应保持不变
    assert_eq!(target.id, "target");
}

#[test]
fn test_merge_does_not_overwrite_availability() {
    let mut target = Entity::new("target");
    let start = JulianDate::from_date_components(2020, 1, 1, 0, 0, 0, 0.0);
    let stop = JulianDate::from_date_components(2020, 6, 1, 0, 0, 0, 0.0);
    let interval = TimeInterval::new(start, stop, true, true);
    let mut tic: TimeIntervalCollection<()> = TimeIntervalCollection::new();
    tic.add_interval(TimeIntervalData::new(interval, None), &|_, _| true);
    target.availability = Some(tic);

    let source = Entity::new("source");
    // source 无可用性
    target.merge(&source);
    // target 的可用性应被保留
    assert!(target.availability.is_some());
}

#[test]
fn test_merge_works_with_custom_properties() {
    let mut target = Entity::new("target")
        .with_property("existing", serde_json::json!("target_value"));

    let source = Entity::new("source")
        .with_property("existing", serde_json::json!("source_value"))
        .with_property("new_prop", serde_json::json!(42));

    target.merge(&source);
    // 已有属性不应被覆盖
    assert_eq!(target.properties["existing"], serde_json::json!("target_value"));
    // 新属性应被添加
    assert_eq!(target.properties["new_prop"], serde_json::json!(42));
}

#[test]
fn test_merge_fills_undefined_position() {
    let mut target = Entity::new("target");
    let source = Entity::new("source").with_position(0.5, 0.6, 200.0);

    target.merge(&source);
    // position 应从 source 填充
    assert!(matches!(target.position, Property::Constant(_)));
}

// ===== computeModelMatrix =====

#[test]
fn test_compute_model_matrix_returns_none_when_position_undefined() {
    let entity = Entity::new("test");
    let ellipsoid = Ellipsoid::WGS84;
    let result = entity.compute_model_matrix(0.0, &ellipsoid);
    assert!(result.is_none());
}

#[test]
fn test_compute_model_matrix_returns_enu_when_no_orientation() {
    let entity = Entity::new("test").with_position(0.0, 0.0, 0.0);
    let ellipsoid = Ellipsoid::WGS84;
    let result = entity.compute_model_matrix(0.0, &ellipsoid);
    assert!(result.is_some());

    let mat = result.unwrap();
    // 平移应在赤道上 lon=0 处
    let translation = mat.w_axis;
    // At lon=0, lat=0, h=0: x ≈ 6378137, y ≈ 0, z ≈ 0
    assert!((translation.x - 6378137.0).abs() < 1.0);
    assert!(translation.y.abs() < 1.0);
    assert!(translation.z.abs() < 1.0);
}

#[test]
fn test_compute_model_matrix_with_orientation() {
    let mut entity = Entity::new("test").with_position(0.0, 0.0, 0.0);
    // Identity quaternion [x, y, z, w] = [0, 0, 0, 1]
    entity.orientation = Property::Constant([0.0, 0.0, 0.0, 1.0]);

    let ellipsoid = Ellipsoid::WGS84;
    let result = entity.compute_model_matrix(0.0, &ellipsoid);
    assert!(result.is_some());

    let mat = result.unwrap();
    // 使用单位四元数时，旋转应为单位阵
    let rot = glam::DMat3::from_cols(
        mat.col(0).truncate(),
        mat.col(1).truncate(),
        mat.col(2).truncate(),
    );
    let identity = glam::DMat3::IDENTITY;
    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (rot.col(i)[j] - identity.col(i)[j]).abs() < 1e-10,
                "rot[{}][{}] = {} != {}",
                i, j, rot.col(i)[j], identity.col(i)[j]
            );
        }
    }
}
