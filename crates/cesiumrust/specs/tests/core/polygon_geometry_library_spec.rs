//! Core/PolygonGeometryLibrarySpec → Rust 集成测试
//! 16 个原始 it() 块 → 已移植 16 个 A 类测试
//!
//! 所有测试均为 A 类（纯计算几何）。

use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::polygon_geometry_library::{
    subdivide_rhumb_line, split_polygons_on_equator, ArcType,
};
use glam::DVec3;

const EPSILON7: f64 = 1e-7;

/// 将扁平数组 [x0,y0,z0, x1,y1,z1, ...] 解包为 Vec<DVec3>。
fn unpack_array(flat: &[f64]) -> Vec<DVec3> {
    flat.chunks(3)
        .map(|c| DVec3::new(c[0], c[1], c[2]))
        .collect()
}

/// 将角度数组 [lon0,lat0, lon1,lat1, ...] 转换为 WGS84 上的 Vec<DVec3>。
fn from_degrees_array(degrees: &[f64]) -> Vec<DVec3> {
    let ellipsoid = Ellipsoid::WGS84;
    degrees
        .chunks(2)
        .map(|c| {
            let lon = c[0].to_radians();
            let lat = c[1].to_radians();
            let carto = cesium_geospatial::Cartographic::from_radians(lon, lat, 0.0);
            ellipsoid.cartographic_to_cartesian(&carto)
        })
        .collect()
}

fn assert_vec3_epsilon(actual: DVec3, expected: DVec3, epsilon: f64) {
    assert!(
        (actual.x - expected.x).abs() < epsilon
            && (actual.y - expected.y).abs() < epsilon
            && (actual.z - expected.z).abs() < epsilon,
        "Expected {:?} ≈ {:?} (epsilon={})",
        actual,
        expected,
        epsilon
    );
}

// ============================================================================
// subdivideRhumbLine
// ============================================================================

#[test]
fn subdivide_rhumb_line_returns_first_point_if_same() {
    let ellipsoid = Ellipsoid::WGS84;
    let p0 = DVec3::new(3813220.0, -5085291.0, 527179.0);
    let p1 = DVec3::new(3813220.0, -5085291.0, 527179.0);
    let positions = subdivide_rhumb_line(&ellipsoid, p0, p1, 2.0);
    assert_eq!(positions.len(), 3);
    assert_eq!(positions, vec![3813220.0, -5085291.0, 527179.0]);
}

#[test]
fn subdivide_rhumb_line_returns_first_point_if_close() {
    let ellipsoid = Ellipsoid::WGS84;
    let p0 = DVec3::new(3813220.0, -5085291.0, 527179.0);
    let p1 = DVec3::new(3813220.0, -5085291.0, 527179.0 + 1.0);
    // 实际表面距离约为 ~0.997
    let positions = subdivide_rhumb_line(&ellipsoid, p0, p1, 2.0);
    assert_eq!(positions.len(), 3);
    assert_eq!(positions, vec![3813220.0, -5085291.0, 527179.0]);
}

#[test]
fn subdivide_rhumb_line_subdivides() {
    let ellipsoid = Ellipsoid::WGS84;
    let p0 = DVec3::new(3813220.0, -5085291.0, 527179.0);
    let p1 = DVec3::new(3813220.0, -5085291.0, 527179.0 + 5.0);
    // 实际表面距离约为 ~4.983
    let positions = subdivide_rhumb_line(&ellipsoid, p0, p1, 2.0);
    assert_eq!(positions.len(), 12); // 4 vertices * 3

    let expected: Vec<f64> = vec![
        3813220.447295841, -5085291.596511482, 527179.0622555692,
        3813220.3851130935, -5085291.513584885, 527180.3036009098,
        3813220.3229302, -5085291.430658091, 527181.5449462304,
        3813220.2607471617, -5085291.347731101, 527182.7862915307,
    ];
    for i in 0..positions.len() {
        assert!(
            (positions[i] - expected[i]).abs() < EPSILON7,
            "positions[{}] = {} expected {} (diff={})",
            i, positions[i], expected[i], (positions[i] - expected[i]).abs()
        );
    }
}

// ============================================================================
// splitPolygonsOnEquator
// ============================================================================

#[test]
fn splits_simple_polygon_at_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        3813220.0, -5085291.0, 527179.0,
        3701301.0, -5097773.0, -993503.0,
        5037375.0, -3776794.0, -1017021.0,
        5049166.0, -3865306.0, 494270.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    let expected1 = DVec3::new(3799258.6687873346, -5123110.886796548, 0.0);
    let expected2 = DVec3::new(5077099.353935631, -3860530.240917096, 0.0);

    assert_eq!(polygons.len(), 2);
    assert_eq!(polygons[0].len(), 4);
    assert_vec3_epsilon(polygons[0][0], positions[0], EPSILON7);
    assert_vec3_epsilon(polygons[0][1], expected1, EPSILON7);
    assert_vec3_epsilon(polygons[0][2], expected2, EPSILON7);
    assert_vec3_epsilon(polygons[0][3], positions[3], EPSILON7);
    assert_eq!(polygons[1].len(), 4);
    assert_vec3_epsilon(polygons[1][0], expected1, EPSILON7);
    assert_vec3_epsilon(polygons[1][1], positions[1], EPSILON7);
    assert_vec3_epsilon(polygons[1][2], positions[2], EPSILON7);
    assert_vec3_epsilon(polygons[1][3], expected2, EPSILON7);
}

#[test]
fn no_split_one_position_touching_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        3813220.0, -5085291.0, 527179.0,
        3701301.0, -5097773.0, 0.0,
        5049166.0, -3865306.0, 494270.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    assert_eq!(polygons.len(), 1);
    assert_eq!(polygons[0].len(), 3);
    assert_vec3_epsilon(polygons[0][0], positions[0], EPSILON7);
    assert_vec3_epsilon(polygons[0][1], positions[1], EPSILON7);
    assert_vec3_epsilon(polygons[0][2], positions[2], EPSILON7);
}

#[test]
fn no_split_edge_on_equator_above() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -3219367.0, -5491259.0, 401098.0,
        -3217795.0, -5506913.0, 0.0,
        -2713036.0, -5772334.0, 0.0,
        -2713766.0, -5757498.0, 406910.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);
    assert_eq!(polygons.len(), 1);
    assert_eq!(polygons[0].len(), 4);
}

#[test]
fn no_split_edge_on_equator_below() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -3180138.0, -5441382.0, -974441.0,
        -3186540.0, -5525048.0, 0.0,
        -2198716.0, -5986569.0, 0.0,
        -2135113.0, -5925878.0, -996868.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);
    assert_eq!(polygons.len(), 1);
    assert_eq!(polygons[0].len(), 4);
}

#[test]
fn splits_positively_concave_polygon() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -3723536.687096985, -5140643.423654287, 622159.6094790212,
        -3706443.9124709764, -5089398.802336418, -1016836.564118223,
        -1818346.3577937474, -5988204.417556031, -1226992.0906221648,
        -1949728.2308330906, -6022778.780648997, 775419.1678640501,
        -2891108.934831509, -5659936.656854747, -534148.7427656263,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    assert_eq!(polygons.len(), 3);
    assert_eq!(polygons[0].len(), 3);
    assert_eq!(polygons[1].len(), 7);
    assert_eq!(polygons[2].len(), 3);
}

#[test]
fn splits_negatively_concave_polygon() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -4164072.7435535816, -4791571.5503237555, 605958.8290040599,
        -4167507.7232260685, -4800497.02674794, -508272.2109012767,
        -3712172.6000501625, -5184159.589216706, 116723.13202563708,
        -3259646.0020361557, -5455158.378873343, -532227.4715966922,
        -3283717.3855494126, -5434359.545068984, 592819.1229613343,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    assert_eq!(polygons.len(), 3);
    assert_eq!(polygons[0].len(), 7);
    assert_eq!(polygons[1].len(), 3);
    assert_eq!(polygons[2].len(), 3);
}

#[test]
fn splits_positively_concave_with_point_on_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -3592289.0, -5251493.0, 433532.0,
        -3568746.0, -5245699.0, -646544.0,
        -2273628.0, -5915229.0, -715098.0,
        -2410175.0, -5885323.0, 475855.0,
        -3012338.0, -5621469.0, 0.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    assert_eq!(polygons.len(), 3);
    assert_eq!(polygons[0].len(), 3);
    assert_eq!(polygons[1].len(), 5);
    assert_eq!(polygons[2].len(), 3);
}

#[test]
fn splits_negatively_concave_with_point_on_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -3774632.0, -5136123.0, 222459.0,
        -3714187.0, -5173580.0, -341046.0,
        -3516544.0, -5320967.0, 0.0,
        -3304860.0, -5444086.0, -342567.0,
        -3277484.0, -5466977.0, 218213.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    assert_eq!(polygons.len(), 3);
    assert_eq!(polygons[0].len(), 5);
    assert_eq!(polygons[1].len(), 3);
    assert_eq!(polygons[2].len(), 3);
}

#[test]
fn splits_polygon_with_edge_on_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -3227931.0, -5469496.0, 584508.0,
        -3150093.0, -5488360.0, -792747.0,
        -1700622.0, -6089685.0, -835364.0,
        -1786389.0, -6122714.0, 0.0,
        -2593600.0, -5826977.0, 0.0,
        -2609132.0, -5790155.0, 584508.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    assert_eq!(polygons.len(), 2);
    assert_eq!(polygons[0].len(), 4);
    assert_eq!(polygons[1].len(), 5);
}

#[test]
fn splits_polygon_with_backtracking_edge() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        -3491307.0, -5296123.0, 650596.0,
        -3495031.0, -5334507.0, 0.0,
        -4333607.0, -4677312.0, 0.0,
        -4275491.0, -4629182.0, -968553.0,
        -2403691.0, -5827997.0, -943662.0,
        -2484409.0, -5837281.0, 631344.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Geodesic);

    assert_eq!(polygons.len(), 2);
    assert_eq!(polygons[0].len(), 4);
    assert_eq!(polygons[1].len(), 5);
}

#[test]
fn splits_simple_rhumb_polygon_at_equator() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        3813220.0, -5085291.0, 527179.0,
        3701301.0, -5097773.0, -993503.0,
        5037375.0, -3776794.0, -1017021.0,
        5049166.0, -3865306.0, 494270.0,
    ]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Rhumb);

    let expected1 = DVec3::new(3799205.595277112, -5123150.245267465, 0.0);
    let expected2 = DVec3::new(5077127.456540122, -3860493.2820580625, 0.0);

    assert_eq!(polygons.len(), 2);
    assert_eq!(polygons[0].len(), 4);
    assert_vec3_epsilon(polygons[0][0], positions[0], EPSILON7);
    assert_vec3_epsilon(polygons[0][1], expected1, EPSILON7);
    assert_vec3_epsilon(polygons[0][2], expected2, EPSILON7);
    assert_vec3_epsilon(polygons[0][3], positions[3], EPSILON7);
    assert_eq!(polygons[1].len(), 4);
    assert_vec3_epsilon(polygons[1][0], expected1, EPSILON7);
    assert_vec3_epsilon(polygons[1][1], positions[1], EPSILON7);
    assert_vec3_epsilon(polygons[1][2], positions[2], EPSILON7);
    assert_vec3_epsilon(polygons[1][3], expected2, EPSILON7);
}

#[test]
fn splits_rhumb_polygon_across_idl() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = from_degrees_array(&[30.0, -30.0, 20.0, 30.0, -20.0, 30.0, -30.0, -30.0]);

    let polygons = split_polygons_on_equator(&[positions.clone()], &ellipsoid, ArcType::Rhumb);

    let expected1 = DVec3::new(5780555.229886577, 2695517.1720840395, 0.0);
    let expected2 = DVec3::new(5780555.229886577, -2695517.1720840395, 0.0);

    assert_eq!(polygons.len(), 2);
    assert_eq!(polygons[0].len(), 4);
    assert_vec3_epsilon(polygons[0][0], positions[0], EPSILON7);
    assert_vec3_epsilon(polygons[0][1], expected1, EPSILON7);
    assert_vec3_epsilon(polygons[0][2], expected2, EPSILON7);
    assert_vec3_epsilon(polygons[0][3], positions[3], EPSILON7);
    assert_eq!(polygons[1].len(), 4);
    assert_vec3_epsilon(polygons[1][0], expected1, EPSILON7);
    assert_vec3_epsilon(polygons[1][1], positions[1], EPSILON7);
    assert_vec3_epsilon(polygons[1][2], positions[2], EPSILON7);
    assert_vec3_epsilon(polygons[1][3], expected2, EPSILON7);
}

#[test]
fn splits_array_of_polygons() {
    let ellipsoid = Ellipsoid::WGS84;
    let positions = unpack_array(&[
        3813220.0, -5085291.0, 527179.0,
        3701301.0, -5097773.0, -993503.0,
        5037375.0, -3776794.0, -1017021.0,
        5049166.0, -3865306.0, 494270.0,
    ]);

    let polygons = split_polygons_on_equator(
        &[positions.clone(), positions.clone()],
        &ellipsoid,
        ArcType::Geodesic,
    );

    assert_eq!(polygons.len(), 4);
}
