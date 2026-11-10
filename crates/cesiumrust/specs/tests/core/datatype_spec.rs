//! Core/ComponentDatatypeSpec + Core/IndexDatatypeSpec → Rust 集成测试
//!
//! ComponentDatatypeSpec：13 个原始 it() 块 → 移植 5 个 A 类测试
//! IndexDatatypeSpec：14 个原始 it() 块 → 移植 5 个 A 类测试
//!
//! 省略的 C 类测试（JS 类型化数组 / DeveloperError throws）：
//! - ComponentDatatype：fromTypedArray throws(1)、createTypedArray(2)、createArrayBufferView(4)、
//!   createTypedArray throws(2)、fromName throws(1) = 10 个 C 类
//! - IndexDatatype：createTypedArray throws(1)、createTypedArrayFromArrayBuffer(4+throws3)、
//!   getSizeInBytes throws(1)、fromTypedArray throws(1) = 10 个 C 类
//!
//! 注意：JS 的 `fromTypedArray` 对应 Rust 的 `from_gl_value`（依据 WebGL 常量推断类型）。
//! JS 的 `createTypedArray`/`createArrayBufferView` 属于 C 类（JS 特有的类型化数组创建）。

use cesium_geospatial::attribute_compression::{ComponentDatatype, IndexDatatype};

// ============================================================================
// ComponentDatatype
// ============================================================================

#[test]
fn component_datatype_from_gl_value_works() {
    // 对应 "fromTypedArray works" —— 每个 JS 类型化数组都对应一个 GL 常量
    assert_eq!(
        ComponentDatatype::from_gl_value(0x1400),
        Some(ComponentDatatype::Byte)
    );
    assert_eq!(
        ComponentDatatype::from_gl_value(0x1401),
        Some(ComponentDatatype::UnsignedByte)
    );
    assert_eq!(
        ComponentDatatype::from_gl_value(0x1402),
        Some(ComponentDatatype::Short)
    );
    assert_eq!(
        ComponentDatatype::from_gl_value(0x1403),
        Some(ComponentDatatype::UnsignedShort)
    );
    assert_eq!(
        ComponentDatatype::from_gl_value(0x1404),
        Some(ComponentDatatype::Int)
    );
    assert_eq!(
        ComponentDatatype::from_gl_value(0x1405),
        Some(ComponentDatatype::UnsignedInt)
    );
    assert_eq!(
        ComponentDatatype::from_gl_value(0x1406),
        Some(ComponentDatatype::Float)
    );
    assert_eq!(
        ComponentDatatype::from_gl_value(0x140A),
        Some(ComponentDatatype::Double)
    );
    // 非法值
    assert_eq!(ComponentDatatype::from_gl_value(0x9999), None);
}

#[test]
fn component_datatype_validate_works() {
    // 所有枚举变体都合法
    assert!(ComponentDatatype::Byte.validate());
    assert!(ComponentDatatype::UnsignedByte.validate());
    assert!(ComponentDatatype::Short.validate());
    assert!(ComponentDatatype::UnsignedShort.validate());
    assert!(ComponentDatatype::Int.validate());
    assert!(ComponentDatatype::UnsignedInt.validate());
    assert!(ComponentDatatype::Float.validate());
    assert!(ComponentDatatype::Double.validate());
}

#[test]
fn component_datatype_get_size_in_bytes() {
    assert_eq!(ComponentDatatype::Byte.size_in_bytes(), 1);
    assert_eq!(ComponentDatatype::UnsignedByte.size_in_bytes(), 1);
    assert_eq!(ComponentDatatype::Short.size_in_bytes(), 2);
    assert_eq!(ComponentDatatype::UnsignedShort.size_in_bytes(), 2);
    assert_eq!(ComponentDatatype::Int.size_in_bytes(), 4);
    assert_eq!(ComponentDatatype::UnsignedInt.size_in_bytes(), 4);
    assert_eq!(ComponentDatatype::Float.size_in_bytes(), 4);
    assert_eq!(ComponentDatatype::Double.size_in_bytes(), 8);
}

#[test]
fn component_datatype_from_name_works() {
    assert_eq!(
        ComponentDatatype::from_name("BYTE"),
        Some(ComponentDatatype::Byte)
    );
    assert_eq!(
        ComponentDatatype::from_name("UNSIGNED_BYTE"),
        Some(ComponentDatatype::UnsignedByte)
    );
    assert_eq!(
        ComponentDatatype::from_name("SHORT"),
        Some(ComponentDatatype::Short)
    );
    assert_eq!(
        ComponentDatatype::from_name("UNSIGNED_SHORT"),
        Some(ComponentDatatype::UnsignedShort)
    );
    assert_eq!(
        ComponentDatatype::from_name("INT"),
        Some(ComponentDatatype::Int)
    );
    assert_eq!(
        ComponentDatatype::from_name("UNSIGNED_INT"),
        Some(ComponentDatatype::UnsignedInt)
    );
    assert_eq!(
        ComponentDatatype::from_name("FLOAT"),
        Some(ComponentDatatype::Float)
    );
    assert_eq!(
        ComponentDatatype::from_name("DOUBLE"),
        Some(ComponentDatatype::Double)
    );
    // 非法名称
    assert_eq!(ComponentDatatype::from_name("INVALID"), None);
}

#[test]
fn component_datatype_gl_value_roundtrip() {
    // 验证所有变体的 gl_value → from_gl_value 往返
    let all = [
        ComponentDatatype::Byte,
        ComponentDatatype::UnsignedByte,
        ComponentDatatype::Short,
        ComponentDatatype::UnsignedShort,
        ComponentDatatype::Int,
        ComponentDatatype::UnsignedInt,
        ComponentDatatype::Float,
        ComponentDatatype::Double,
    ];
    for dt in &all {
        assert_eq!(ComponentDatatype::from_gl_value(dt.gl_value()), Some(*dt));
    }
}

// ============================================================================
// IndexDatatype
// ============================================================================

#[test]
fn index_datatype_validate_validates_input() {
    assert!(IndexDatatype::UnsignedByte.validate());
    assert!(IndexDatatype::UnsignedShort.validate());
    assert!(IndexDatatype::UnsignedInt.validate());
}

#[test]
fn index_datatype_create_typed_array_logic() {
    // 对应 "createTypedArray creates array"：
    // numberOfVertices < 65536 → UNSIGNED_SHORT（2 字节）
    // numberOfVertices >= 65536 → UNSIGNED_INT（4 字节）
    let dt = IndexDatatype::for_vertex_count(3);
    assert_eq!(dt.size_in_bytes(), 2); // Uint16Array.BYTES_PER_ELEMENT
    assert_eq!(dt, IndexDatatype::UnsignedShort);

    let dt = IndexDatatype::for_vertex_count(IndexDatatype::SIXTY_FOUR_KILOBYTES + 1);
    assert_eq!(dt.size_in_bytes(), 4); // Uint32Array.BYTES_PER_ELEMENT
    assert_eq!(dt, IndexDatatype::UnsignedInt);
}

#[test]
fn index_datatype_get_size_in_bytes_returns_size() {
    assert_eq!(IndexDatatype::UnsignedByte.size_in_bytes(), 1);
    assert_eq!(IndexDatatype::UnsignedShort.size_in_bytes(), 2);
    assert_eq!(IndexDatatype::UnsignedInt.size_in_bytes(), 4);
}

#[test]
fn index_datatype_from_name_works() {
    assert_eq!(
        IndexDatatype::from_name("UNSIGNED_BYTE"),
        Some(IndexDatatype::UnsignedByte)
    );
    assert_eq!(
        IndexDatatype::from_name("UNSIGNED_SHORT"),
        Some(IndexDatatype::UnsignedShort)
    );
    assert_eq!(
        IndexDatatype::from_name("UNSIGNED_INT"),
        Some(IndexDatatype::UnsignedInt)
    );
    assert_eq!(IndexDatatype::from_name("INVALID"), None);
}

#[test]
fn index_datatype_from_gl_value_works() {
    assert_eq!(
        IndexDatatype::from_gl_value(0x1401),
        Some(IndexDatatype::UnsignedByte)
    );
    assert_eq!(
        IndexDatatype::from_gl_value(0x1403),
        Some(IndexDatatype::UnsignedShort)
    );
    assert_eq!(
        IndexDatatype::from_gl_value(0x1405),
        Some(IndexDatatype::UnsignedInt)
    );
    // 非法
    assert_eq!(IndexDatatype::from_gl_value(0x1400), None); // BYTE 不是索引类型
    assert_eq!(IndexDatatype::from_gl_value(0x9999), None);
}
