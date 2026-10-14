//! 参考自 CesiumJS 的测试：
//! - isLeapYearSpec（1 个 A 类测试）
//! - getStringFromTypedArraySpec（5 个 A 类测试）
//! 总计：6 个测试

// ===== isLeapYear =====

#[test]
fn is_leap_year_valid_years() {
    // 参考自："Check for valid leap years"
    use cesium_time::is_leap_year;

    // 标准闰年（能被 4 整除，但不能被 100 整除）
    assert!(is_leap_year(2000)); // 能被 400 整除
    assert!(is_leap_year(2004));
    assert!(is_leap_year(2008));
    assert!(is_leap_year(2012));
    assert!(is_leap_year(2016));
    assert!(is_leap_year(2020));
    assert!(is_leap_year(2024));
    assert!(is_leap_year(1600)); // 能被 400 整除
    assert!(is_leap_year(1200)); // 能被 400 整除

    // 非闰年
    assert!(!is_leap_year(2001));
    assert!(!is_leap_year(2002));
    assert!(!is_leap_year(2003));
    assert!(!is_leap_year(2005));
    assert!(!is_leap_year(1900)); // 能被 100 整除但不能被 400 整除
    assert!(!is_leap_year(2100)); // 能被 100 整除但不能被 400 整除
    assert!(!is_leap_year(1800)); // 能被 100 整除但不能被 400 整除
    assert!(!is_leap_year(1700)); // 能被 100 整除但不能被 400 整除
}

// ===== getStringFromTypedArray =====
// 在 Rust 中，这对应 String::from_utf8 / std::str::from_utf8
// 我们测试等价的行为

/// 将字节切片（UTF-8）转换为 String。
/// 参考 CesiumJS 的 `getStringFromTypedArray(array, byteOffset, byteLength)`
fn get_string_from_typed_array(data: &[u8], byte_offset: usize, byte_length: Option<usize>) -> String {
    let len = byte_length.unwrap_or(data.len() - byte_offset);
    let slice = &data[byte_offset..byte_offset + len];
    String::from_utf8(slice.to_vec()).expect("Invalid UTF-8")
}

#[test]
fn converts_typed_array_to_string() {
    // 参考自："converts a typed array to string"
    let arr: &[u8] = &[67, 101, 115, 105, 117, 109]; // "Cesium"
    let string = get_string_from_typed_array(arr, 0, None);
    assert_eq!(string, "Cesium");

    // 空数组
    let arr: &[u8] = &[];
    let string = get_string_from_typed_array(arr, 0, None);
    assert_eq!(string, "");
}

#[test]
fn converts_sub_region_of_typed_array_to_string() {
    // 参考自："converts a sub-region of a typed array to a string"
    let arr: &[u8] = &[67, 101, 115, 105, 117, 109]; // "Cesium"
    let string = get_string_from_typed_array(arr, 1, Some(3));
    assert_eq!(string, "esi");
}

#[test]
fn unicode_2_byte_characters_work() {
    // 参考自："Unicode 2-byte characters work"
    // "Zürich" 的 UTF-8 编码：Z=90, ü=195,188, r=114, i=105, c=99, h=104
    let arr: &[u8] = &[90, 195, 188, 114, 105, 99, 104];
    let string = get_string_from_typed_array(arr, 0, None);
    assert_eq!(string, "Zürich");
}

#[test]
fn unicode_3_byte_characters_work() {
    // 参考自："Unicode 3-byte characters work"
    // U+08A0 (ࢠ) 的 UTF-8 编码：224, 162, 160
    let arr: &[u8] = &[224, 162, 160];
    let string = get_string_from_typed_array(arr, 0, None);
    assert_eq!(string, "ࢠ");
}

#[test]
fn unicode_4_byte_characters_work() {
    // 参考自："Unicode 4-byte characters work"
    // U+10281 (𐊁) 的 UTF-8 编码：240, 144, 138, 129
    let arr: &[u8] = &[240, 144, 138, 129];
    let string = get_string_from_typed_array(arr, 0, None);
    assert_eq!(string, "𐊁");
}
