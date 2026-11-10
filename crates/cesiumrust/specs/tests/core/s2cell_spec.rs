//! 参考自 `Specs/Core/S2CellSpec`（27 个 it()，15 个 A 类）
//!
//! 省略 B 类（throws）测试，因为 Rust 的类型系统会强制输入合法。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::s2cell::S2Cell;
use glam::DVec3;

fn cartesian_from_degrees(lon_deg: f64, lat_deg: f64) -> DVec3 {
    let cartographic = Cartographic::from_degrees(lon_deg, lat_deg, 0.0);
    Cartographic::to_cartesian(&cartographic, &Ellipsoid::WGS84)
}

#[test]
fn constructor_works() {
    let cell = S2Cell::new(3458764513820540928u128);
    assert_eq!(cell.cell_id(), 3458764513820540928u128);
}

#[test]
fn creates_cell_from_valid_token() {
    let cell = S2Cell::from_token("3");
    assert_eq!(cell.cell_id(), 3458764513820540928u128);
}

#[test]
fn creates_cell_from_valid_face_position_level() {
    let cell = S2Cell::from_face_position_level(0, 0, 1);
    assert_eq!(S2Cell::get_token_from_id(cell.cell_id()), "04");

    let cell = S2Cell::from_face_position_level(0, 1, 1);
    assert_eq!(S2Cell::get_token_from_id(cell.cell_id()), "0c");

    let cell = S2Cell::from_face_position_level(0, 2, 1);
    assert_eq!(S2Cell::get_token_from_id(cell.cell_id()), "14");

    let cell = S2Cell::from_face_position_level(0, 3, 1);
    assert_eq!(S2Cell::get_token_from_id(cell.cell_id()), "1c");

    let cell = S2Cell::from_face_position_level(2, 0, 1);
    assert_eq!(S2Cell::get_token_from_id(cell.cell_id()), "44");

    let cell = S2Cell::from_face_position_level(4, 0, 1);
    assert_eq!(S2Cell::get_token_from_id(cell.cell_id()), "84");

    let cell = S2Cell::from_face_position_level(1, 538969508876688737u128, 30);
    assert_eq!(S2Cell::get_token_from_id(cell.cell_id()), "2ef59bd352b93ac3");
}

#[test]
fn accepts_valid_token() {
    assert!(S2Cell::is_valid_token("1"));
    assert!(S2Cell::is_valid_token("2ef59bd34"));
    assert!(S2Cell::is_valid_token("2ef59bd352b93ac3"));
}

#[test]
fn rejects_token_of_invalid_value() {
    assert!(!S2Cell::is_valid_token("LOL"));
    assert!(!S2Cell::is_valid_token("----"));
    assert!(!S2Cell::is_valid_token(&"9".repeat(17)));
    assert!(!S2Cell::is_valid_token("0"));
}

#[test]
fn accepts_valid_cell_id() {
    assert!(S2Cell::is_valid_id(3383782026967071428u128));
    assert!(S2Cell::is_valid_id(3458764513820540928u128));
}

#[test]
fn rejects_cell_id_of_invalid_value() {
    assert!(!S2Cell::is_valid_id(0));
    // Face 大于 5
    assert!(!S2Cell::is_valid_id(0b0010101000000000000000000000000000000000000000000000000000000000u128));
}

#[test]
fn correctly_converts_token_to_cell_id() {
    assert_eq!(S2Cell::get_id_from_token("04"), 288230376151711744u128);
    assert_eq!(S2Cell::get_id_from_token("3"), 3458764513820540928u128);
    assert_eq!(
        S2Cell::get_id_from_token("2ef59bd352b93ac3"),
        3383782026967071427u128
    );
}

#[test]
fn correctly_converts_cell_id_to_token() {
    assert_eq!(S2Cell::get_token_from_id(288230376151711744u128), "04");
    assert_eq!(S2Cell::get_token_from_id(3458764513820540928u128), "3");
    assert_eq!(
        S2Cell::get_token_from_id(3383782026967071427u128),
        "2ef59bd352b93ac3"
    );
}

#[test]
fn gets_correct_level_of_cell() {
    assert_eq!(S2Cell::get_level(3170534137668829184u128), 1);
    assert_eq!(S2Cell::get_level(3383782026921377792u128), 16);
    assert_eq!(S2Cell::get_level(3383782026967071427u128), 30);
}

#[test]
fn gets_correct_parent_of_cell() {
    let cell = S2Cell::new(3383782026967515136u128);
    let parent = cell.get_parent();
    assert_eq!(parent.cell_id(), 3383782026971709440u128);
}

#[test]
fn gets_correct_parent_of_cell_at_given_level() {
    let cell = S2Cell::new(3383782026967056384u128);
    let parent = cell.get_parent_at_level(21);
    assert_eq!(parent.cell_id(), 3383782026967252992u128);

    let parent = cell.get_parent_at_level(7);
    assert_eq!(parent.cell_id(), 3383821801271328768u128);

    let parent = cell.get_parent_at_level(0);
    assert_eq!(parent.cell_id(), 3458764513820540928u128);
}

#[test]
fn gets_correct_children_of_cell() {
    let cell = S2Cell::new(3383782026971709440u128);
    let expected_child_cell_ids: [u128; 4] = [
        3383782026959126528,
        3383782026967515136,
        3383782026975903744,
        3383782026984292352,
    ];
    for i in 0..4 {
        assert_eq!(cell.get_child(i).cell_id(), expected_child_cell_ids[i as usize]);
    }
}

#[test]
fn gets_correct_center_of_cell() {
    // 相对于地球半径（约 6e6）使用 EPSILON10，故绝对容差约 1e-4
    let eps = 1e-4;

    let center = S2Cell::from_token("1").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(0.0, 0.0);
    assert!((center - expected).length() < eps, "face 0 center: {:?} vs {:?}", center, expected);

    let center = S2Cell::from_token("3").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(90.0, 0.0);
    assert!((center - expected).length() < eps, "face 1 center: {:?} vs {:?}", center, expected);

    let center = S2Cell::from_token("5").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(-180.0, 90.0);
    assert!((center - expected).length() < eps, "face 2 center: {:?} vs {:?}", center, expected);

    let center = S2Cell::from_token("7").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(-180.0, 0.0);
    assert!((center - expected).length() < eps, "face 3 center: {:?} vs {:?}", center, expected);

    let center = S2Cell::from_token("9").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(-90.0, 0.0);
    assert!((center - expected).length() < eps, "face 4 center: {:?} vs {:?}", center, expected);

    let center = S2Cell::from_token("b").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(0.0, -90.0);
    assert!((center - expected).length() < eps, "face 5 center: {:?} vs {:?}", center, expected);

    let center = S2Cell::from_token("2ef59bd352b93ac3").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(105.64131803774308, -10.490091033598308);
    assert!((center - expected).length() < eps, "deep cell center: {:?} vs {:?}", center, expected);

    let center = S2Cell::from_token("1234567").get_center(&Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(9.868307318504081, 27.468392925827605);
    assert!((center - expected).length() < eps, "cell 1234567 center: {:?} vs {:?}", center, expected);
}

#[test]
fn gets_correct_vertices_of_cell() {
    // 相对于地球半径使用 EPSILON15，绝对容差约 1e-8
    let eps = 1e-8;
    let cell = S2Cell::from_token("2ef59bd352b93ac3");

    let v0 = cell.get_vertex(0, &Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(105.64131799299665, -10.490091077431977);
    assert!((v0 - expected).length() < eps, "vertex 0: {:?} vs {:?}", v0, expected);

    let v1 = cell.get_vertex(1, &Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(105.64131808248949, -10.490091072946313);
    assert!((v1 - expected).length() < eps, "vertex 1: {:?} vs {:?}", v1, expected);

    let v2 = cell.get_vertex(2, &Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(105.64131808248948, -10.490090989764633);
    assert!((v2 - expected).length() < eps, "vertex 2: {:?} vs {:?}", v2, expected);

    let v3 = cell.get_vertex(3, &Ellipsoid::WGS84);
    let expected = cartesian_from_degrees(105.64131799299665, -10.4900909942503);
    assert!((v3 - expected).length() < eps, "vertex 3: {:?} vs {:?}", v3, expected);
}
