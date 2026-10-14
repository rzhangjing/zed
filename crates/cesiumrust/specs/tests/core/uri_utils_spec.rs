//! 参考自 CesiumJS 的测试：
//! - objectToQuerySpec（5 个 A 类）
//! - queryToObjectSpec（4 个 A 类）
//! - parseResponseHeadersSpec（2 个 A 类）
//! - getFilenameFromUriSpec（1 个 A 类）
//! - getExtensionFromUriSpec（1 个 A 类）

use cesium_geospatial::uri_utils::{
    get_extension_from_uri, get_filename_from_uri, object_to_query, parse_response_headers,
    query_to_object, QueryValue,
};
use std::collections::HashMap;

// ===== objectToQuery =====

#[test]
fn test_object_to_query_can_encode_data() {
    // 注意：HashMap 不保证顺序，因此我们用单键 map 进行测试
    let mut obj = HashMap::new();
    obj.insert("key1".to_string(), QueryValue::Single("some value".to_string()));
    let str = object_to_query(&obj);
    assert_eq!(str, "key1=some%20value");

    let mut obj = HashMap::new();
    obj.insert("key2".to_string(), QueryValue::Single("a/b".to_string()));
    let str = object_to_query(&obj);
    assert_eq!(str, "key2=a%2Fb");
}

#[test]
fn test_object_to_query_can_encode_arrays() {
    let mut obj = HashMap::new();
    obj.insert("key".to_string(), QueryValue::Array(vec!["a".to_string(), "b".to_string()]));
    let str = object_to_query(&obj);
    assert_eq!(str, "key=a&key=b");
}

#[test]
fn test_object_to_query_round_trip() {
    let mut obj = HashMap::new();
    obj.insert("foo".to_string(), QueryValue::Array(vec!["bar".to_string(), "bar2".to_string()]));
    obj.insert("bit".to_string(), QueryValue::Single("byte".to_string()));

    let query = object_to_query(&obj);
    let obj2 = query_to_object(&query);
    assert_eq!(obj2, obj);
}

#[test]
fn test_object_to_query_can_encode_blank() {
    let obj: HashMap<String, QueryValue> = HashMap::new();
    assert_eq!(object_to_query(&obj), "");
}

#[test]
fn test_object_to_query_combined() {
    // 由于 HashMap 顺序不确定，逐个测试各个键
    let mut obj = HashMap::new();
    obj.insert("key1".to_string(), QueryValue::Single("some value".to_string()));
    obj.insert("key2".to_string(), QueryValue::Single("a/b".to_string()));
    obj.insert("key3".to_string(), QueryValue::Array(vec!["x".to_string(), "y".to_string()]));

    let query = object_to_query(&obj);
    // 验证所有部分都存在
    assert!(query.contains("key1=some%20value"));
    assert!(query.contains("key2=a%2Fb"));
    assert!(query.contains("key3=x"));
    assert!(query.contains("key3=y"));
}

// ===== queryToObject =====

#[test]
fn test_query_to_object_can_decode_data() {
    let obj = query_to_object("key1=some%20value&key2=a%2Fb");
    assert_eq!(obj.get("key1"), Some(&QueryValue::Single("some value".to_string())));
    assert_eq!(obj.get("key2"), Some(&QueryValue::Single("a/b".to_string())));

    let obj = query_to_object(
        "spec=Core%2FobjectToQuery%20can%20encode%20data.&debug=Core%2FobjectToQuery%20can%20encode%20data.",
    );
    assert_eq!(
        obj.get("spec"),
        Some(&QueryValue::Single("Core/objectToQuery can encode data.".to_string()))
    );
    assert_eq!(
        obj.get("debug"),
        Some(&QueryValue::Single("Core/objectToQuery can encode data.".to_string()))
    );

    // + 被解码为空格
    let obj = query_to_object("q=query+string");
    assert_eq!(obj.get("q"), Some(&QueryValue::Single("query string".to_string())));
}

#[test]
fn test_query_to_object_can_decode_arrays() {
    let obj = query_to_object("key=a&key=b");
    assert_eq!(
        obj.get("key"),
        Some(&QueryValue::Array(vec!["a".to_string(), "b".to_string()]))
    );
}

#[test]
fn test_query_to_object_can_use_semicolon() {
    let obj = query_to_object("key=a;key=b;key2=c");
    assert_eq!(
        obj.get("key"),
        Some(&QueryValue::Array(vec!["a".to_string(), "b".to_string()]))
    );
    assert_eq!(obj.get("key2"), Some(&QueryValue::Single("c".to_string())));
}

#[test]
fn test_query_to_object_can_decode_blank() {
    let obj = query_to_object("");
    assert!(obj.is_empty());
}

// ===== parseResponseHeaders =====

#[test]
fn test_parse_response_headers_empty() {
    let result = parse_response_headers("");
    assert!(result.is_empty());
}

#[test]
fn test_parse_response_headers_correctly_parses() {
    let header_string = "Date: Sun, 24 Oct 2004 04:58:38 GMT\r\n\
                         Server: Apache/1.3.31 (Unix)\r\n\
                         Keep-Alive: timeout=15, max=99\r\n\
                         Connection: Keep-Alive\r\n\
                         Transfer-Encoding: chunked\r\n\
                         Content-Type: text/plain; charset=utf-8";

    let result = parse_response_headers(header_string);
    assert_eq!(result.get("Date").unwrap(), "Sun, 24 Oct 2004 04:58:38 GMT");
    assert_eq!(result.get("Server").unwrap(), "Apache/1.3.31 (Unix)");
    assert_eq!(result.get("Keep-Alive").unwrap(), "timeout=15, max=99");
    assert_eq!(result.get("Connection").unwrap(), "Keep-Alive");
    assert_eq!(result.get("Transfer-Encoding").unwrap(), "chunked");
    assert_eq!(result.get("Content-Type").unwrap(), "text/plain; charset=utf-8");
    assert_eq!(result.len(), 6);
}

// ===== getFilenameFromUri =====

#[test]
fn test_get_filename_from_uri_works() {
    let result = get_filename_from_uri("http://www.mysite.com/awesome?makeitawesome=true");
    assert_eq!(result, "awesome");

    let result = get_filename_from_uri("http://www.mysite.com/somefolder/awesome.png#makeitawesome");
    assert_eq!(result, "awesome.png");
}

// ===== getExtensionFromUri =====

#[test]
fn test_get_extension_from_uri_works() {
    let result = get_extension_from_uri("http://www.mysite.com/awesome?makeitawesome=true");
    assert_eq!(result, "");

    let result = get_extension_from_uri("http://www.mysite.com/somefolder/awesome.png#makeitawesome");
    assert_eq!(result, "png");

    let result = get_extension_from_uri("awesome.png");
    assert_eq!(result, "png");
}
