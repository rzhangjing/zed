//! Core/Resource + RequestScheduler → Rust 集成测试。
//!
//! 参考 CesiumJS：
//! - Core/Resource（URL 构建、查询参数、derive）
//! - Core/RequestScheduler（节流、优先级、服务器限制）
//!
//! A 类测试：RequestScheduler schedule/complete/cancel/throttle/priority、
//! Resource build_url/derive/server_key/with_query/with_header.
//! C 类省略：实际 HTTP 请求、Promise 链、重试逻辑。

use cesium_resource::{Request, RequestScheduler, RequestState, RequestType, Resource};

// === Resource ===

#[test]
fn resource_new() {
    let r = Resource::new("https://example.com/tileset.json");
    assert_eq!(r.url, "https://example.com/tileset.json");
    assert!(r.query_parameters.is_empty());
    assert!(r.headers.is_empty());
}

#[test]
fn resource_build_url_no_params() {
    let r = Resource::new("https://example.com/api");
    assert_eq!(r.build_url(), "https://example.com/api");
}

#[test]
fn resource_build_url_with_params() {
    let r = Resource::new("https://example.com/api")
        .with_query("key", "abc123");
    let url = r.build_url();
    assert!(url.starts_with("https://example.com/api?"));
    assert!(url.contains("key=abc123"));
}

#[test]
fn resource_build_url_existing_query() {
    let r = Resource::new("https://example.com/api?existing=1")
        .with_query("extra", "2");
    let url = r.build_url();
    assert!(url.contains("existing=1"));
    assert!(url.contains("extra=2"));
    assert!(url.contains('&'));
}

#[test]
fn resource_with_header() {
    let r = Resource::new("https://example.com")
        .with_header("Authorization", "Bearer token");
    assert_eq!(r.headers.get("Authorization").unwrap(), "Bearer token");
}

#[test]
fn resource_server_key() {
    let r = Resource::new("https://tiles.example.com:443/path/to/tile.b3dm");
    assert_eq!(r.server_key(), "tiles.example.com:443");
}

#[test]
fn resource_server_key_no_port() {
    let r = Resource::new("https://cdn.example.com/tile.png");
    // 服务器 key 现包含默认端口
    assert_eq!(r.server_key(), "cdn.example.com:443");
}

#[test]
fn resource_derive_relative() {
    let base = Resource::new("https://example.com/data/tileset.json");
    let derived = base.derive("tiles/root.b3dm");
    assert_eq!(derived.url, "https://example.com/data/tiles/root.b3dm");
}

#[test]
fn resource_derive_trailing_slash() {
    let base = Resource::new("https://example.com/data/");
    let derived = base.derive("tile.b3dm");
    assert_eq!(derived.url, "https://example.com/data/tile.b3dm");
}

#[test]
fn resource_derive_inherits_query() {
    let base = Resource::new("https://example.com/tileset.json")
        .with_query("token", "xyz");
    let derived = base.derive("tile.b3dm");
    assert_eq!(derived.query_parameters.get("token").unwrap(), "xyz");
}

// === RequestScheduler ===

#[test]
fn scheduler_defaults() {
    let scheduler = RequestScheduler::new();
    assert_eq!(scheduler.maximum_requests, 50);
    assert_eq!(scheduler.maximum_requests_per_server, 18);
    assert!(scheduler.throttle_requests);
    assert_eq!(scheduler.active_request_count(), 0);
    assert_eq!(scheduler.pending_request_count(), 0);
}

#[test]
fn scheduler_schedule_unthrottled() {
    let mut scheduler = RequestScheduler::new();
    let request = Request::new(
        "https://example.com/tile.b3dm".to_string(),
        RequestType::Tiles3D,
    );
    let id = scheduler.schedule(request).unwrap();
    assert_eq!(scheduler.active_request_count(), 1);

    let req = scheduler.get_request(id).unwrap();
    assert_eq!(req.state, RequestState::Active);
}

#[test]
fn scheduler_complete() {
    let mut scheduler = RequestScheduler::new();
    let request = Request::new(
        "https://example.com/tile.b3dm".to_string(),
        RequestType::Tiles3D,
    );
    let id = scheduler.schedule(request).unwrap();
    assert!(scheduler.complete(id));
    assert_eq!(scheduler.active_request_count(), 0);
}

#[test]
fn scheduler_cancel() {
    let mut scheduler = RequestScheduler::new();
    let request = Request::new(
        "https://example.com/tile.b3dm".to_string(),
        RequestType::Terrain,
    );
    let id = scheduler.schedule(request).unwrap();
    assert!(scheduler.cancel(id));
    assert_eq!(scheduler.active_request_count(), 0);
}

#[test]
fn scheduler_complete_nonexistent() {
    let mut scheduler = RequestScheduler::new();
    use cesium_resource::RequestId;
    assert!(!scheduler.complete(RequestId(999)));
}

#[test]
fn scheduler_server_has_open_slots() {
    let scheduler = RequestScheduler::new();
    assert!(scheduler.server_has_open_slots("example.com", 1));
    assert!(scheduler.server_has_open_slots("example.com", 18));
    assert!(!scheduler.server_has_open_slots("example.com", 19));
}

#[test]
fn scheduler_heap_has_open_slots() {
    let scheduler = RequestScheduler::new();
    assert!(scheduler.heap_has_open_slots(1));
    assert!(scheduler.heap_has_open_slots(20));
    assert!(!scheduler.heap_has_open_slots(21));
}

#[test]
fn scheduler_throttled_pending() {
    let mut scheduler = RequestScheduler::new();
    scheduler.maximum_requests_per_server = 1;

    // 第一个请求激活
    let r1 = Request::throttled(
        "https://example.com/1.b3dm".to_string(),
        RequestType::Tiles3D,
        1.0,
    );
    scheduler.schedule(r1).unwrap();
    scheduler.update();

    // 对同一服务器的第二个请求应处于 pending
    let r2 = Request::throttled(
        "https://example.com/2.b3dm".to_string(),
        RequestType::Tiles3D,
        2.0,
    );
    scheduler.schedule(r2).unwrap();

    // 服务器 key 包含默认端口
    assert!(!scheduler.server_has_open_slots("example.com:443", 1));
}

// === Request ===

#[test]
fn request_new_extracts_server_key() {
    let request = Request::new(
        "https://tiles.example.com:8080/path/tile.b3dm".to_string(),
        RequestType::Tiles3D,
    );
    assert_eq!(request.server_key, "tiles.example.com:8080");
    assert_eq!(request.request_type, RequestType::Tiles3D);
    assert_eq!(request.state, RequestState::Unissued);
}

#[test]
fn request_throttled() {
    let request = Request::throttled(
        "https://cdn.example.com/terrain.tile".to_string(),
        RequestType::Terrain,
        5.0,
    );
    assert!(request.throttle);
    assert!(request.throttle_by_server);
    assert!((request.priority - 5.0).abs() < 1e-10);
}

#[test]
fn request_type_default() {
    assert_eq!(RequestType::default(), RequestType::Other);
}

#[test]
fn request_state_default() {
    assert_eq!(RequestState::default(), RequestState::Unissued);
}
