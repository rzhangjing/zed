//! Resource URL 操作规格测试。
//! 参考自 CesiumJS Core/ResourceSpec（2744 行，约 60 个 it()）
//!
//! A 类测试：URL 解析、查询参数、模板值、
//! appendForwardSlash、getDerivedResource、setQueryParameters、getUrlComponent。
//! 省略 C 类：网络抓取、代理、retryCallback、ImageBitmap、canvas。

use cesium_resource::{DeriveResourceOptions, Resource};

// === 构造函数 / URL 解析 ===

#[test]
fn constructor_sets_url_and_parses_query() {
    let resource = Resource::new("http://test.com/tileset?foo=bar&baz=foo");
    assert_eq!(resource.get_url_component(false), "http://test.com/tileset");
    assert_eq!(
        resource.get_url_component(true),
        "http://test.com/tileset?baz=foo&foo=bar"
    );
    assert_eq!(resource.query_parameters.get("foo").unwrap(), "bar");
    assert_eq!(resource.query_parameters.get("baz").unwrap(), "foo");
}

#[test]
fn constructor_without_query_has_empty_params() {
    let resource = Resource::new("http://invalid.domain.com/tileset");
    assert_eq!(resource.url, "http://invalid.domain.com/tileset");
    assert!(resource.query_parameters.is_empty());
    assert!(resource.template_values.is_empty());
    assert!(resource.headers.is_empty());
}

#[test]
fn constructor_unparsed_preserves_query_in_url() {
    let resource = Resource::new_unparsed("http://test.com/tileset?foo=bar&baz=foo");
    assert_eq!(
        resource.get_url_component(false),
        "http://test.com/tileset?foo=bar&baz=foo"
    );
    assert!(resource.query_parameters.is_empty());
}

// === appendForwardSlash ===

#[test]
fn append_forward_slash_appends() {
    let mut resource = Resource::new("http://test.com/tileset");
    assert_eq!(resource.url, "http://test.com/tileset");
    resource.append_forward_slash();
    assert_eq!(resource.url, "http://test.com/tileset/");
}

#[test]
fn append_forward_slash_noop_if_already_ends_with_slash() {
    let mut resource = Resource::new("http://test.com/tileset/");
    resource.append_forward_slash();
    assert_eq!(resource.url, "http://test.com/tileset/");
}

// === 模板值 ===

#[test]
fn replaces_template_values_in_url() {
    let resource = Resource::new("http://test.com/tileset/{foo}/{bar}")
        .with_template_value("foo", "test1")
        .with_template_value("bar", "test2");

    assert_eq!(resource.build_url(), "http://test.com/tileset/test1/test2");
}

#[test]
fn replaces_numeric_template_values() {
    let resource = Resource::new("http://test.com/tileset/{0}/{1}")
        .with_template_value("0", "test1")
        .with_template_value("1", "test2");

    assert_eq!(resource.build_url(), "http://test.com/tileset/test1/test2");
}

#[test]
fn leaves_template_values_unchanged_if_not_provided() {
    let resource = Resource::new("http://test.com/tileset/{foo}/{bar}");
    assert_eq!(resource.build_url(), "http://test.com/tileset/{foo}/{bar}");
}

#[test]
fn url_encodes_replacement_template_values() {
    let resource = Resource::new("http://test.com/tileset/{foo}/{bar}")
        .with_template_value("foo", "a/b")
        .with_template_value("bar", "x$y#");

    assert_eq!(
        resource.build_url(),
        "http://test.com/tileset/a%2Fb/x%24y%23"
    );
}

// === getUrlComponent ===

#[test]
fn get_url_component_without_query() {
    let resource = Resource::new("http://test.com/tileset?key=value");
    assert_eq!(resource.get_url_component(false), "http://test.com/tileset");
}

#[test]
fn get_url_component_with_query() {
    let resource = Resource::new("http://test.com/tileset?key=value");
    assert_eq!(
        resource.get_url_component(true),
        "http://test.com/tileset?key=value"
    );
}

#[test]
fn get_url_component_empty_query_returns_base() {
    let resource = Resource::new("http://test.com/tileset");
    assert_eq!(resource.get_url_component(true), "http://test.com/tileset");
}

// === setQueryParameters ===

#[test]
fn set_query_parameters_use_as_default_true() {
    let mut resource = Resource::new("http://test.com/terrain")
        .with_query("x", "1")
        .with_query("y", "2");

    resource.set_query_parameters(
        vec![
            ("x".to_string(), "3".to_string()),
            ("y".to_string(), "4".to_string()),
            ("z".to_string(), "0".to_string()),
        ],
        true,
    );

    // 保留已有键，新增键
    assert_eq!(resource.query_parameters.get("x").unwrap(), "1");
    assert_eq!(resource.query_parameters.get("y").unwrap(), "2");
    assert_eq!(resource.query_parameters.get("z").unwrap(), "0");
}

#[test]
fn set_query_parameters_use_as_default_false() {
    let mut resource = Resource::new("http://test.com/terrain")
        .with_query("x", "1")
        .with_query("y", "2");

    resource.set_query_parameters(
        vec![
            ("x".to_string(), "3".to_string()),
            ("y".to_string(), "4".to_string()),
            ("z".to_string(), "0".to_string()),
        ],
        false,
    );

    // 全部覆盖
    assert_eq!(resource.query_parameters.get("x").unwrap(), "3");
    assert_eq!(resource.query_parameters.get("y").unwrap(), "4");
    assert_eq!(resource.query_parameters.get("z").unwrap(), "0");
    assert_eq!(resource.query_parameters.len(), 3);
}

// === getDerivedResource ===

#[test]
fn derived_resource_with_directory_parent() {
    let parent = Resource::new("http://test.com/tileset/");
    let derived = parent.get_derived_resource(&DeriveResourceOptions {
        url: Some("tileset.json".to_string()),
        ..Default::default()
    });

    assert_eq!(derived.url, "http://test.com/tileset/tileset.json");
}

#[test]
fn derived_resource_with_file_parent() {
    let parent = Resource::new("http://test.com/tileset/tileset.json");
    let derived = parent.get_derived_resource(&DeriveResourceOptions {
        url: Some("0/0/0.b3dm".to_string()),
        ..Default::default()
    });

    assert_eq!(derived.url, "http://test.com/tileset/0/0/0.b3dm");
}

#[test]
fn derived_resource_with_template_values() {
    let parent = Resource::new("http://test.com/terrain/{z}/{x}/{y}.terrain");
    let derived = parent.get_derived_resource(&DeriveResourceOptions {
        template_values: vec![
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "2".to_string()),
            ("z".to_string(), "0".to_string()),
        ],
        ..Default::default()
    });

    assert_eq!(derived.url, "http://test.com/terrain/0/1/2.terrain");
}

#[test]
fn derived_resource_with_query_parameters() {
    let parent = Resource::new("http://test.com/terrain");
    let derived = parent.get_derived_resource(&DeriveResourceOptions {
        query_parameters: vec![
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "2".to_string()),
            ("z".to_string(), "0".to_string()),
        ],
        ..Default::default()
    });

    // URL 保持不变，查询参数被合并
    assert_eq!(derived.url, "http://test.com/terrain");
    assert_eq!(derived.query_parameters.get("x").unwrap(), "1");
    assert_eq!(derived.query_parameters.get("y").unwrap(), "2");
    assert_eq!(derived.query_parameters.get("z").unwrap(), "0");
}

#[test]
fn derived_resource_merges_parent_query_params() {
    let mut parent = Resource::new("http://test.com/tileset?key=value");
    parent.append_forward_slash();
    parent.query_parameters.insert("foo".to_string(), "bar".to_string());

    let derived = parent.get_derived_resource(&DeriveResourceOptions {
        url: Some("tileset.json".to_string()),
        query_parameters: vec![
            ("key1".to_string(), "value1".to_string()),
            ("key2".to_string(), "value2".to_string()),
        ],
        ..Default::default()
    });

    assert_eq!(derived.url, "http://test.com/tileset/tileset.json");
    // 保留父级查询参数
    assert_eq!(derived.query_parameters.get("key").unwrap(), "value");
    assert_eq!(derived.query_parameters.get("foo").unwrap(), "bar");
    // 新增参数
    assert_eq!(derived.query_parameters.get("key1").unwrap(), "value1");
    assert_eq!(derived.query_parameters.get("key2").unwrap(), "value2");
}

#[test]
fn derived_resource_absolute_url_overrides() {
    let parent = Resource::new("http://test.com/tileset/");
    let derived = parent.get_derived_resource(&DeriveResourceOptions {
        url: Some("http://other.com/data.json".to_string()),
        ..Default::default()
    });

    assert_eq!(derived.url, "http://other.com/data.json");
}

#[test]
fn derived_resource_inherits_headers() {
    let parent = Resource::new("http://test.com/tileset/")
        .with_header("Accept", "application/json");

    let derived = parent.get_derived_resource(&DeriveResourceOptions {
        url: Some("tile.b3dm".to_string()),
        ..Default::default()
    });

    assert_eq!(derived.headers.get("Accept").unwrap(), "application/json");
}

// === build_url with query ===

#[test]
fn build_url_includes_sorted_query_params() {
    let resource = Resource::new("http://test.com/api")
        .with_query("key", "value")
        .with_query("format", "json");

    let url = resource.build_url();
    assert_eq!(url, "http://test.com/api?format=json&key=value");
}

#[test]
fn build_url_no_query_returns_base() {
    let resource = Resource::new("http://test.com/api");
    assert_eq!(resource.build_url(), "http://test.com/api");
}

// === server_key ===

#[test]
fn resource_server_key() {
    let resource = Resource::new("https://example.com/path/to/resource");
    assert_eq!(resource.server_key(), "example.com:443");
}

#[test]
fn resource_server_key_explicit_port() {
    let resource = Resource::new("http://localhost:8080/api");
    assert_eq!(resource.server_key(), "localhost:8080");
}
