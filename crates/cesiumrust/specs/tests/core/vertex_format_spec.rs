//! VertexFormat 规格测试。
//!
//! 参考 CesiumJS：
//! - Core/VertexFormatSpec
//!
//! A 类测试：clone、pack/unpack、常量。

use cesium_geospatial::VertexFormat;

#[test]
fn vertex_format_clone() {
    let vf = VertexFormat {
        position: true,
        normal: true,
        st: false,
        tangent: false,
        bitangent: false,
    };
    let cloned = vf;
    assert_eq!(cloned, vf);
}

#[test]
fn vertex_format_pack() {
    let vf = VertexFormat::POSITION_AND_NORMAL;
    let mut array = vec![0.0; 5];
    vf.pack(&mut array, 0);
    assert_eq!(array, vec![1.0, 1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn vertex_format_unpack() {
    let array = vec![1.0, 1.0, 0.0, 0.0, 0.0];
    let vf = VertexFormat::unpack(&array, 0);
    assert_eq!(vf, VertexFormat::POSITION_AND_NORMAL);
}

#[test]
fn vertex_format_pack_array() {
    let vf = VertexFormat::ALL;
    let array = vf.pack_array();
    assert_eq!(array, vec![1.0, 1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn vertex_format_unpack_array() {
    let array = vec![1.0, 0.0, 1.0, 0.0, 0.0];
    let vf = VertexFormat::unpack_array(&array);
    assert_eq!(vf, VertexFormat::POSITION_AND_ST);
}

#[test]
fn vertex_format_roundtrip() {
    let original = VertexFormat {
        position: true,
        normal: false,
        st: true,
        tangent: true,
        bitangent: false,
    };
    let packed = original.pack_array();
    let unpacked = VertexFormat::unpack_array(&packed);
    assert_eq!(unpacked, original);
}

#[test]
fn vertex_format_packed_length() {
    assert_eq!(VertexFormat::PACKED_LENGTH, 5);
}

#[test]
fn vertex_format_constants() {
    // 将预置的查找表绑定到局部变量，使每个 `assert!` 都是真正的字段读取，
    // 而非被 lint（正确地）判定为空转的编译期常量。
    let all = VertexFormat::ALL;
    assert!(all.position);
    assert!(all.normal);
    assert!(all.st);
    assert!(all.tangent);
    assert!(all.bitangent);

    let pos_only = VertexFormat::POSITION_ONLY;
    assert!(pos_only.position);
    assert!(!pos_only.normal);
    assert!(!pos_only.st);
    assert!(!pos_only.tangent);
    assert!(!pos_only.bitangent);

    let pos_norm = VertexFormat::POSITION_AND_NORMAL;
    assert!(pos_norm.position);
    assert!(pos_norm.normal);
    assert!(!pos_norm.st);

    let pos_st = VertexFormat::POSITION_AND_ST;
    assert!(pos_st.position);
    assert!(!pos_st.normal);
    assert!(pos_st.st);
}

#[test]
fn vertex_format_default() {
    let vf = VertexFormat::default();
    assert_eq!(vf, VertexFormat::ALL);
}
