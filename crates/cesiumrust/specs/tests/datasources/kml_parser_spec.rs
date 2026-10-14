//! KML Parser + Tour spec
//! Tests: parse_coordinates, parse_kml_color, parse_kml_simple, kml_to_datasource,
//!        KmlTour playlist/playback, KmlTourFlyTo builder

use cesium_kml::{
    FlyToMode, KmlCoordinate, KmlDocument, KmlGeometry, KmlPlacemark, KmlStyle, KmlTour,
    KmlTourEntry, KmlTourFlyTo, KmlTourWait, kml_to_datasource, parse_coordinates,
    parse_kml_color, parse_kml_simple,
};
use glam::DVec3;

// ═══════════════════════════════════════════════════════════════════════════════
// parse_coordinates
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_coordinates_with_altitude() {
    let coords = parse_coordinates("-122.08,37.42,100 -121.5,36.0,200");
    assert_eq!(coords.len(), 2);
    assert!((coords[0].longitude - (-122.08)).abs() < 1e-10);
    assert!((coords[0].latitude - 37.42).abs() < 1e-10);
    assert!((coords[0].altitude - 100.0).abs() < 1e-10);
    assert!((coords[1].altitude - 200.0).abs() < 1e-10);
}

#[test]
fn parse_coordinates_without_altitude() {
    let coords = parse_coordinates("-122.0,37.0");
    assert_eq!(coords.len(), 1);
    assert!((coords[0].altitude - 0.0).abs() < 1e-10);
}

#[test]
fn parse_coordinates_empty_string() {
    let coords = parse_coordinates("");
    assert!(coords.is_empty());
}

#[test]
fn parse_coordinates_whitespace_handling() {
    let coords = parse_coordinates("  -122.0,37.0,0   -121.0,36.0,0  ");
    assert_eq!(coords.len(), 2);
}

#[test]
fn kml_coordinate_to_cartographic() {
    let coord = KmlCoordinate::new(180.0, 90.0, 1000.0);
    let carto = coord.to_cartographic();
    assert!((carto.longitude - std::f64::consts::PI).abs() < 1e-10);
    assert!((carto.latitude - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
    assert!((carto.height - 1000.0).abs() < 1e-10);
}

// ═══════════════════════════════════════════════════════════════════════════════
// parse_kml_color
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_kml_color_red() {
    // aabbggrr format: ff0000ff = alpha=ff, blue=00, green=00, red=ff
    let c = parse_kml_color("ff0000ff").unwrap();
    assert!((c.red - 1.0).abs() < 1e-10);
    assert!((c.green - 0.0).abs() < 1e-10);
    assert!((c.blue - 0.0).abs() < 1e-10);
    assert!((c.alpha - 1.0).abs() < 1e-10);
}

#[test]
fn parse_kml_color_semi_transparent_green() {
    let c = parse_kml_color("8000ff00").unwrap();
    assert!((c.red - 0.0).abs() < 1e-10);
    assert!((c.green - 1.0).abs() < 1e-10);
    assert!((c.blue - 0.0).abs() < 1e-10);
    assert!((c.alpha - 128.0 / 255.0).abs() < 0.01);
}

#[test]
fn parse_kml_color_invalid_length() {
    assert!(parse_kml_color("fff").is_none());
    assert!(parse_kml_color("").is_none());
}

// ═══════════════════════════════════════════════════════════════════════════════
// parse_kml_simple
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_kml_simple_point() {
    let kml = r#"<kml><Document><name>Test</name>
    <Placemark><name>P1</name><Point><coordinates>-122.0,37.0,0</coordinates></Point></Placemark>
    </Document></kml>"#;
    let doc = parse_kml_simple(kml).unwrap();
    assert_eq!(doc.name, Some("Test".to_string()));
    assert_eq!(doc.placemarks.len(), 1);
    assert_eq!(doc.placemarks[0].name, Some("P1".to_string()));
    match &doc.placemarks[0].geometry {
        Some(KmlGeometry::Point { coordinate, .. }) => {
            assert!((coordinate.longitude - (-122.0)).abs() < 1e-10);
        }
        _ => panic!("Expected Point"),
    }
}

#[test]
fn parse_kml_simple_linestring() {
    let kml = r#"<kml><Placemark><name>Line</name><LineString>
    <coordinates>-122.0,37.0,0 -122.1,37.1,0 -122.2,37.2,0</coordinates>
    </LineString></Placemark></kml>"#;
    let doc = parse_kml_simple(kml).unwrap();
    match &doc.placemarks[0].geometry {
        Some(KmlGeometry::LineString { coordinates, .. }) => {
            assert_eq!(coordinates.len(), 3);
        }
        _ => panic!("Expected LineString"),
    }
}

#[test]
fn parse_kml_simple_polygon() {
    let kml = r#"<kml><Placemark><name>Poly</name><Polygon><outerBoundaryIs>
    <coordinates>-122.0,37.0,0 -122.1,37.0,0 -122.1,37.1,0 -122.0,37.0,0</coordinates>
    </outerBoundaryIs></Polygon></Placemark></kml>"#;
    let doc = parse_kml_simple(kml).unwrap();
    match &doc.placemarks[0].geometry {
        Some(KmlGeometry::Polygon { outer, .. }) => {
            assert_eq!(outer.len(), 4);
        }
        _ => panic!("Expected Polygon"),
    }
}

#[test]
fn parse_kml_simple_multiple_placemarks() {
    let kml = r#"<kml>
    <Placemark><name>A</name><Point><coordinates>0,0,0</coordinates></Point></Placemark>
    <Placemark><name>B</name><Point><coordinates>1,1,0</coordinates></Point></Placemark>
    </kml>"#;
    let doc = parse_kml_simple(kml).unwrap();
    assert_eq!(doc.placemarks.len(), 2);
    assert_eq!(doc.placemarks[0].name, Some("A".to_string()));
    assert_eq!(doc.placemarks[1].name, Some("B".to_string()));
}

#[test]
fn parse_kml_simple_with_style() {
    let kml = r#"<kml><Placemark><name>Styled</name>
    <Style><LineStyle><color>ff0000ff</color><width>3.0</width></LineStyle></Style>
    <LineString><coordinates>0,0,0 1,1,0</coordinates></LineString>
    </Placemark></kml>"#;
    let doc = parse_kml_simple(kml).unwrap();
    let style = doc.placemarks[0].style.as_ref().unwrap();
    let ls = style.line_style.as_ref().unwrap();
    assert_eq!(ls.color, Some("ff0000ff".to_string()));
    assert!((ls.width - 3.0).abs() < 1e-10);
}

#[test]
fn parse_kml_simple_extrude_flag() {
    let kml = r#"<kml><Placemark><Point><extrude>1</extrude>
    <coordinates>0,0,100</coordinates></Point></Placemark></kml>"#;
    let doc = parse_kml_simple(kml).unwrap();
    match &doc.placemarks[0].geometry {
        Some(KmlGeometry::Point { extrude, .. }) => assert!(*extrude),
        _ => panic!("Expected Point"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// kml_to_datasource
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn kml_to_datasource_converts_entities() {
    let doc = KmlDocument {
        name: Some("MyDoc".to_string()),
        placemarks: vec![
            KmlPlacemark {
                id: Some("p1".to_string()),
                name: Some("Point1".to_string()),
                geometry: Some(KmlGeometry::Point {
                    coordinate: KmlCoordinate::new(-122.0, 37.0, 0.0),
                    extrude: false,
                }),
                ..Default::default()
            },
            KmlPlacemark {
                id: Some("p2".to_string()),
                name: Some("Line1".to_string()),
                geometry: Some(KmlGeometry::LineString {
                    coordinates: vec![
                        KmlCoordinate::new(0.0, 0.0, 0.0),
                        KmlCoordinate::new(1.0, 1.0, 0.0),
                    ],
                    extrude: false,
                    tessellate: false,
                }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let ds = kml_to_datasource(&doc);
    assert_eq!(ds.name, "MyDoc");
    assert_eq!(ds.entities.len(), 2);
}

#[test]
fn kml_to_datasource_default_name() {
    let doc = KmlDocument::default();
    let ds = kml_to_datasource(&doc);
    assert_eq!(ds.name, "KML");
}

// ═══════════════════════════════════════════════════════════════════════════════
// KmlTour
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn tour_fly_to_builder() {
    let ft = KmlTourFlyTo::new(5.0, DVec3::new(-122.0, 37.0, 1000.0))
        .with_heading(45.0)
        .with_tilt(60.0)
        .with_range(5000.0)
        .with_mode(FlyToMode::Bounce);
    assert_eq!(ft.duration, 5.0);
    assert_eq!(ft.heading, Some(45.0));
    assert_eq!(ft.tilt, Some(60.0));
    assert_eq!(ft.range, Some(5000.0));
    assert_eq!(ft.fly_to_mode, FlyToMode::Bounce);
}

#[test]
fn tour_wait_duration() {
    let w = KmlTourWait::new(2.5);
    assert_eq!(w.duration, 2.5);
}

#[test]
fn tour_entry_duration() {
    let ft = KmlTourEntry::FlyTo(KmlTourFlyTo::new(3.0, DVec3::ZERO));
    let w = KmlTourEntry::Wait(KmlTourWait::new(1.5));
    assert_eq!(ft.duration(), 3.0);
    assert_eq!(w.duration(), 1.5);
}

#[test]
fn tour_playlist_total_duration() {
    let mut tour = KmlTour::new("t1", "City Tour");
    tour.add_fly_to(KmlTourFlyTo::new(5.0, DVec3::new(-122.0, 37.0, 0.0)));
    tour.add_wait(KmlTourWait::new(2.0));
    tour.add_fly_to(KmlTourFlyTo::new(4.0, DVec3::new(-121.0, 38.0, 0.0)));
    assert_eq!(tour.entry_count(), 3);
    assert!((tour.total_duration() - 11.0).abs() < 1e-10);
}

#[test]
fn tour_playback_lifecycle() {
    let mut tour = KmlTour::new("t1", "Test");
    tour.add_fly_to(KmlTourFlyTo::new(1.0, DVec3::ZERO));
    tour.add_wait(KmlTourWait::new(1.0));

    assert!(!tour.is_playing);
    tour.play();
    assert!(tour.is_playing);
    assert_eq!(tour.playlist_index, 0);
    assert!(!tour.is_complete());

    assert!(tour.advance());
    assert_eq!(tour.playlist_index, 1);

    assert!(!tour.advance());
    assert!(tour.is_complete());
    assert!(!tour.is_playing);
}

#[test]
fn tour_stop_resets() {
    let mut tour = KmlTour::new("t1", "Test");
    tour.add_fly_to(KmlTourFlyTo::new(1.0, DVec3::ZERO));
    tour.add_wait(KmlTourWait::new(1.0));
    tour.play();
    tour.advance();
    assert_eq!(tour.playlist_index, 1);
    tour.stop();
    assert!(!tour.is_playing);
    assert_eq!(tour.playlist_index, 0);
}

#[test]
fn tour_current_entry() {
    let mut tour = KmlTour::new("t1", "Test");
    tour.add_fly_to(KmlTourFlyTo::new(5.0, DVec3::new(1.0, 2.0, 3.0)));
    tour.add_wait(KmlTourWait::new(2.0));

    assert_eq!(tour.current_entry().unwrap().duration(), 5.0);
    tour.advance();
    assert_eq!(tour.current_entry().unwrap().duration(), 2.0);
}

#[test]
fn tour_empty_playlist() {
    let tour = KmlTour::new("empty", "Empty");
    assert_eq!(tour.entry_count(), 0);
    assert!((tour.total_duration() - 0.0).abs() < 1e-10);
    assert!(tour.current_entry().is_none());
    assert!(tour.is_complete());
}

// ═══════════════════════════════════════════════════════════════════════════════
// KmlStyle / KmlPlacemark defaults
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn kml_style_default_empty() {
    let s = KmlStyle::default();
    assert!(s.id.is_none());
    assert!(s.icon_style.is_none());
    assert!(s.line_style.is_none());
    assert!(s.poly_style.is_none());
    assert!(s.label_style.is_none());
}

#[test]
fn kml_placemark_default() {
    let pm = KmlPlacemark::default();
    assert!(pm.id.is_none());
    assert!(pm.name.is_none());
    assert!(pm.geometry.is_none());
    assert!(pm.visibility);
    assert!(pm.extended_data.is_empty());
}
