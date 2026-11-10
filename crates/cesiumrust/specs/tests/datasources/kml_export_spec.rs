//! KML 导出规范 - 参考自 DataSources/exportKmlSpec
//! 测试 KmlExporter、KmlExportStyle、KmlExportPlacemark、KmlExportGeometry、rgba_to_kml_color

use cesium_kml::{
    rgba_to_kml_color, KmlExportGeometry, KmlExportIconStyle, KmlExportLabelStyle,
    KmlExportLineStyle, KmlExportOptions, KmlExportPlacemark, KmlExportPolyStyle, KmlExportStyle,
    KmlExporter,
};

// ═══════════════════════════════════════════════════════════════════════════════
// KmlExporter 基础
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn kml_exporter_default_options() {
    let opts = KmlExportOptions::default();
    assert_eq!(opts.name, "Cesium Export");
    assert_eq!(opts.kml_version, "2.2");
    assert!(opts.export_model);
    assert!(opts.export_images);
    assert!(!opts.time_dynamic);
    assert!(opts.description.is_none());
}

#[test]
fn kml_exporter_new_produces_valid_xml() {
    let exporter = KmlExporter::new();
    let kml = exporter.to_kml();

    assert!(kml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(kml.contains("<kml"));
    assert!(kml.contains("xmlns=\"http://www.opengis.net/kml/2.2\""));
    assert!(kml.contains("xmlns:gx=\"http://www.google.com/kml/ext/2.2\""));
    assert!(kml.contains("xmlns:atom=\"http://www.w3.org/2005/Atom\""));
    assert!(kml.contains("<Document>"));
    assert!(kml.contains("</Document>"));
    assert!(kml.contains("</kml>"));
}

#[test]
fn kml_exporter_custom_name() {
    let opts = KmlExportOptions {
        name: "My Custom Export".to_string(),
        ..Default::default()
    };
    let exporter = KmlExporter::with_options(opts);
    let kml = exporter.to_kml();
    assert!(kml.contains("<name>My Custom Export</name>"));
}

#[test]
fn kml_exporter_with_description() {
    let opts = KmlExportOptions {
        description: Some("Test description".to_string()),
        ..Default::default()
    };
    let exporter = KmlExporter::with_options(opts);
    let kml = exporter.to_kml();
    assert!(kml.contains("<description>Test description</description>"));
}

#[test]
fn kml_exporter_export_result() {
    let exporter = KmlExporter::new();
    let result = exporter.export();
    assert!(!result.kml.is_empty());
    assert!(result.external_files.is_empty());
    assert!(result.kmz.is_none());
}

// ═══════════════════════════════════════════════════════════════════════════════
// KmlExportStyle
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn kml_style_icon_style() {
    let style = KmlExportStyle {
        id: "iconStyle1".to_string(),
        icon_style: Some(KmlExportIconStyle {
            color: "ff0000ff".to_string(),
            scale: 2.0,
            icon_href: Some("pushpin.png".to_string()),
        }),
        line_style: None,
        poly_style: None,
        label_style: None,
    };

    let kml = style.to_kml(4);
    assert!(kml.contains("<Style id=\"iconStyle1\">"));
    assert!(kml.contains("<IconStyle>"));
    assert!(kml.contains("<color>ff0000ff</color>"));
    assert!(kml.contains("<scale>2</scale>"));
    assert!(kml.contains("<Icon><href>pushpin.png</href></Icon>"));
    assert!(kml.contains("</IconStyle>"));
    assert!(kml.contains("</Style>"));
}

#[test]
fn kml_style_line_style() {
    let style = KmlExportStyle {
        id: "lineStyle1".to_string(),
        icon_style: None,
        line_style: Some(KmlExportLineStyle {
            color: "ff00ff00".to_string(),
            width: 3.5,
        }),
        poly_style: None,
        label_style: None,
    };

    let kml = style.to_kml(2);
    assert!(kml.contains("<LineStyle>"));
    assert!(kml.contains("<color>ff00ff00</color>"));
    assert!(kml.contains("<width>3.5</width>"));
    assert!(kml.contains("</LineStyle>"));
}

#[test]
fn kml_style_poly_style() {
    let style = KmlExportStyle {
        id: "polyStyle1".to_string(),
        icon_style: None,
        line_style: None,
        poly_style: Some(KmlExportPolyStyle {
            color: "7f00ff00".to_string(),
            fill: true,
            outline: false,
        }),
        label_style: None,
    };

    let kml = style.to_kml(2);
    assert!(kml.contains("<PolyStyle>"));
    assert!(kml.contains("<color>7f00ff00</color>"));
    assert!(kml.contains("<fill>1</fill>"));
    assert!(kml.contains("<outline>0</outline>"));
}

#[test]
fn kml_style_label_style() {
    let style = KmlExportStyle {
        id: "labelStyle1".to_string(),
        icon_style: None,
        line_style: None,
        poly_style: None,
        label_style: Some(KmlExportLabelStyle {
            color: "ffffffff".to_string(),
            scale: 0.8,
        }),
    };

    let kml = style.to_kml(2);
    assert!(kml.contains("<LabelStyle>"));
    assert!(kml.contains("<color>ffffffff</color>"));
    assert!(kml.contains("<scale>0.8</scale>"));
}

#[test]
fn kml_style_new_constructor() {
    let style = KmlExportStyle::new("empty_style");
    assert_eq!(style.id, "empty_style");
    assert!(style.icon_style.is_none());
    assert!(style.line_style.is_none());
    assert!(style.poly_style.is_none());
    assert!(style.label_style.is_none());
}

// ═══════════════════════════════════════════════════════════════════════════════
// KmlExportPlacemark + Geometry
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn kml_placemark_point() {
    let mut exporter = KmlExporter::new();
    exporter.add_placemark(KmlExportPlacemark::new(
        "Test Point",
        KmlExportGeometry::Point {
            coordinates: vec![[-122.0822, 37.4222, 0.0]],
        },
    ));

    let kml = exporter.to_kml();
    assert!(kml.contains("<Placemark>"));
    assert!(kml.contains("<name>Test Point</name>"));
    assert!(kml.contains("<Point>"));
    assert!(kml.contains("<coordinates>-122.0822,37.4222,0</coordinates>"));
    assert!(kml.contains("</Point>"));
    assert!(kml.contains("</Placemark>"));
}

#[test]
fn kml_placemark_linestring_tessellate() {
    let geometry = KmlExportGeometry::LineString {
        coordinates: vec![
            [-122.0, 37.0, 0.0],
            [-121.0, 38.0, 0.0],
            [-120.0, 39.0, 0.0],
        ],
        tessellate: true,
    };

    let kml = geometry.to_kml(4);
    assert!(kml.contains("<LineString>"));
    assert!(kml.contains("<tessellate>1</tessellate>"));
    assert!(kml.contains("<coordinates>"));
    assert!(kml.contains("-122,37,0 -121,38,0 -120,39,0"));
}

#[test]
fn kml_placemark_polygon_with_hole() {
    let geometry = KmlExportGeometry::Polygon {
        outer_boundary: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
        ],
        inner_boundaries: vec![vec![
            [0.2, 0.2, 0.0],
            [0.8, 0.2, 0.0],
            [0.8, 0.8, 0.0],
            [0.2, 0.8, 0.0],
            [0.2, 0.2, 0.0],
        ]],
    };

    let kml = geometry.to_kml(2);
    assert!(kml.contains("<Polygon>"));
    assert!(kml.contains("<outerBoundaryIs>"));
    assert!(kml.contains("<innerBoundaryIs>"));
    assert!(kml.contains("<LinearRing>"));
}

#[test]
fn kml_placemark_model() {
    let geometry = KmlExportGeometry::Model {
        href: "models/CesiumMilkTruck.glb".to_string(),
        location: [-122.0, 37.0, 100.0],
        heading: 45.0,
        tilt: 10.0,
        roll: 0.0,
        scale: 2.0,
    };

    let kml = geometry.to_kml(4);
    assert!(kml.contains("<Model>"));
    assert!(kml.contains("<longitude>-122</longitude>"));
    assert!(kml.contains("<latitude>37</latitude>"));
    assert!(kml.contains("<altitude>100</altitude>"));
    assert!(kml.contains("<heading>45</heading>"));
    assert!(kml.contains("<tilt>10</tilt>"));
    assert!(kml.contains("<href>models/CesiumMilkTruck.glb</href>"));
}

#[test]
fn kml_placemark_with_style_url() {
    let mut placemark = KmlExportPlacemark::new(
        "Styled Point",
        KmlExportGeometry::Point {
            coordinates: vec![[0.0, 0.0, 0.0]],
        },
    );
    placemark.style_url = Some("#myStyle".to_string());
    placemark.description = Some("A described point".to_string());

    let kml = placemark.to_kml(4);
    assert!(kml.contains("<styleUrl>#myStyle</styleUrl>"));
    assert!(kml.contains("<description>A described point</description>"));
}

#[test]
fn kml_exporter_multiple_styles_and_placemarks() {
    let mut exporter = KmlExporter::new();

    exporter.add_style(KmlExportStyle {
        id: "s1".to_string(),
        icon_style: Some(KmlExportIconStyle {
            color: "ff0000ff".to_string(),
            scale: 1.0,
            icon_href: None,
        }),
        line_style: None,
        poly_style: None,
        label_style: None,
    });

    exporter.add_placemark(KmlExportPlacemark::new(
        "P1",
        KmlExportGeometry::Point {
            coordinates: vec![[10.0, 20.0, 0.0]],
        },
    ));
    exporter.add_placemark(KmlExportPlacemark::new(
        "P2",
        KmlExportGeometry::Point {
            coordinates: vec![[30.0, 40.0, 0.0]],
        },
    ));

    let kml = exporter.to_kml();
    assert!(kml.contains("<Style id=\"s1\">"));
    assert!(kml.contains("<name>P1</name>"));
    assert!(kml.contains("<name>P2</name>"));
    // Styles 应出现在 Placemarks 之前
    let style_pos = kml.find("<Style").unwrap();
    let placemark_pos = kml.find("<Placemark>").unwrap();
    assert!(style_pos < placemark_pos);
}

// ═══════════════════════════════════════════════════════════════════════════════
// rgba_to_kml_color
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn kml_color_red_opaque() {
    assert_eq!(rgba_to_kml_color(1.0, 0.0, 0.0, 1.0), "ff0000ff");
}

#[test]
fn kml_color_green_opaque() {
    assert_eq!(rgba_to_kml_color(0.0, 1.0, 0.0, 1.0), "ff00ff00");
}

#[test]
fn kml_color_blue_opaque() {
    assert_eq!(rgba_to_kml_color(0.0, 0.0, 1.0, 1.0), "ffff0000");
}

#[test]
fn kml_color_white_semitransparent() {
    // a=0.5 → 7f, b=ff, g=ff, r=ff
    assert_eq!(rgba_to_kml_color(1.0, 1.0, 1.0, 0.5), "7fffffff");
}

#[test]
fn kml_color_black_transparent() {
    assert_eq!(rgba_to_kml_color(0.0, 0.0, 0.0, 0.0), "00000000");
}

// ═══════════════════════════════════════════════════════════════════════════════
// XML 转义
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn kml_xml_escaping_in_name() {
    let opts = KmlExportOptions {
        name: "A & B <tag>".to_string(),
        ..Default::default()
    };
    let exporter = KmlExporter::with_options(opts);
    let kml = exporter.to_kml();
    assert!(kml.contains("<name>A &amp; B &lt;tag&gt;</name>"));
}
