//! Core/EventSpec、ResourceSpec、RequestSchedulerSpec、ColorSpec
//! 及其他工具类规格 → Rust 集成测试

use cesium_event::{Event, SimpleEvent};
use cesium_resource::{Request, RequestScheduler, RequestState, RequestType, Resource};
use cesium_specs::{assert_approx, epsilon};
use std::cell::Cell;
use std::rc::Rc;

// === Event ===

#[test]
fn test_event_new() {
    let event: Event<i32> = Event::new();
    assert_eq!(event.number_of_listeners(), 0);
    assert!(event.is_empty());
}

#[test]
fn test_event_add_listener() {
    let event: Event<i32> = Event::new();
    let _id = event.add_listener(|_| {});
    assert_eq!(event.number_of_listeners(), 1);
    assert!(!event.is_empty());
}

#[test]
fn test_event_raise() {
    let event: Event<i32> = Event::new();
    let received = Rc::new(Cell::new(0));
    let received_clone = received.clone();
    event.add_listener(move |val| {
        received_clone.set(*val);
    });
    event.raise(&42);
    assert_eq!(received.get(), 42);
}

#[test]
fn test_event_multiple_listeners() {
    let event: Event<i32> = Event::new();
    let sum = Rc::new(Cell::new(0));

    let sum1 = sum.clone();
    event.add_listener(move |val| {
        sum1.set(sum1.get() + val);
    });

    let sum2 = sum.clone();
    event.add_listener(move |val| {
        sum2.set(sum2.get() + val * 2);
    });

    event.raise(&10);
    assert_eq!(sum.get(), 30); // 10 + 20
}

#[test]
fn test_event_remove_listener() {
    let event: Event<i32> = Event::new();
    let count = Rc::new(Cell::new(0));
    let count_clone = count.clone();

    let id = event.add_listener(move |_| {
        count_clone.set(count_clone.get() + 1);
    });

    event.raise(&0);
    assert_eq!(count.get(), 1);

    assert!(event.remove_listener(id));
    event.raise(&0);
    assert_eq!(count.get(), 1); // 不应自增
}

#[test]
fn test_event_clear() {
    let event: Event<()> = Event::new();
    event.add_listener(|_| {});
    event.add_listener(|_| {});
    assert_eq!(event.number_of_listeners(), 2);

    event.clear();
    assert_eq!(event.number_of_listeners(), 0);
}

#[test]
fn test_simple_event() {
    let event = SimpleEvent::new();
    let fired = Rc::new(Cell::new(false));
    let fired_clone = fired.clone();

    event.add_listener(move |_| {
        fired_clone.set(true);
    });

    assert!(!fired.get());
    event.raise_simple();
    assert!(fired.get());
}

// === Resource ===

#[test]
fn test_resource_new() {
    let resource = Resource::new("https://example.com/api");
    assert_eq!(resource.url, "https://example.com/api");
    assert!(resource.query_parameters.is_empty());
    assert!(resource.headers.is_empty());
}

#[test]
fn test_resource_with_query() {
    let resource = Resource::new("https://example.com/api")
        .with_query("key", "value")
        .with_query("format", "json");
    assert_eq!(resource.query_parameters.len(), 2);
}

#[test]
fn test_resource_build_url() {
    let resource = Resource::new("https://example.com/api")
        .with_query("key", "value");
    let url = resource.build_url();
    assert!(url.starts_with("https://example.com/api?"));
    assert!(url.contains("key=value"));
}

#[test]
fn test_resource_build_url_no_params() {
    let resource = Resource::new("https://example.com/api");
    let url = resource.build_url();
    assert_eq!(url, "https://example.com/api");
}

#[test]
fn test_resource_with_header() {
    let resource = Resource::new("https://example.com/api")
        .with_header("Authorization", "Bearer token123");
    assert_eq!(resource.headers.len(), 1);
    assert_eq!(
        resource.headers.get("Authorization").unwrap(),
        "Bearer token123"
    );
}

#[test]
fn test_resource_server_key() {
    let resource = Resource::new("https://example.com:443/path/to/resource");
    assert_eq!(resource.server_key(), "example.com:443");
}

#[test]
fn test_resource_derive() {
    let base = Resource::new("https://example.com/tileset.json");
    let derived = base.derive("tiles/tile.b3dm");
    assert_eq!(derived.url, "https://example.com/tiles/tile.b3dm");
}

#[test]
fn test_resource_derive_preserves_params() {
    let base = Resource::new("https://example.com/tileset.json")
        .with_query("token", "abc");
    let derived = base.derive("tiles/tile.b3dm");
    assert_eq!(derived.query_parameters.get("token").unwrap(), "abc");
}

// === RequestScheduler ===

#[test]
fn test_request_scheduler_new() {
    let scheduler = RequestScheduler::new();
    assert_eq!(scheduler.maximum_requests, 50);
    assert_eq!(scheduler.maximum_requests_per_server, 18);
    assert!(scheduler.throttle_requests);
    assert_eq!(scheduler.active_request_count(), 0);
}

#[test]
fn test_request_scheduler_schedule() {
    let mut scheduler = RequestScheduler::new();
    let request = Request::new(
        "https://example.com/tile.b3dm".to_string(),
        RequestType::Tiles3D,
    );
    let id = scheduler.schedule(request).unwrap();
    assert_eq!(scheduler.active_request_count(), 1);

    scheduler.complete(id);
    assert_eq!(scheduler.active_request_count(), 0);
}

#[test]
fn test_request_scheduler_cancel() {
    let mut scheduler = RequestScheduler::new();
    let request = Request::new(
        "https://example.com/tile.b3dm".to_string(),
        RequestType::Tiles3D,
    );
    let id = scheduler.schedule(request).unwrap();
    assert_eq!(scheduler.active_request_count(), 1);

    assert!(scheduler.cancel(id));
    assert_eq!(scheduler.active_request_count(), 0);
}

#[test]
fn test_request_scheduler_server_throttling() {
    let mut scheduler = RequestScheduler::new();
    scheduler.maximum_requests_per_server = 2;

    // 向同一服务器调度 2 个请求
    for i in 0..2 {
        let request = Request::throttled(
            format!("https://example.com/tile{}.b3dm", i),
            RequestType::Tiles3D,
            i as f64,
        );
        scheduler.schedule(request).unwrap();
    }
    scheduler.update();

    // 检查服务器已无空闲槽位（服务器键包含默认端口）
    assert!(!scheduler.server_has_open_slots("example.com:443", 1));
}

#[test]
fn test_request_scheduler_heap_slots() {
    let scheduler = RequestScheduler::new();
    assert!(scheduler.heap_has_open_slots(1));
    assert!(scheduler.heap_has_open_slots(20));
    assert!(!scheduler.heap_has_open_slots(21));
}

// === Request ===

#[test]
fn test_request_new() {
    let request = Request::new(
        "https://example.com/tile.b3dm".to_string(),
        RequestType::Tiles3D,
    );
    assert_eq!(request.url, "https://example.com/tile.b3dm");
    assert_eq!(request.request_type, RequestType::Tiles3D);
    assert_eq!(request.state, RequestState::Unissued);
    assert!(!request.throttle);
}

#[test]
fn test_request_throttled() {
    let request = Request::throttled(
        "https://example.com/tile.b3dm".to_string(),
        RequestType::Imagery,
        5.0,
    );
    assert!(request.throttle);
    assert!(request.throttle_by_server);
    assert_approx!(request.priority, 5.0, epsilon::EPSILON15);
}

#[test]
fn test_request_server_key_extraction() {
    let request = Request::new(
        "https://example.com:8080/path/to/resource".to_string(),
        RequestType::Other,
    );
    assert_eq!(request.server_key, "example.com:8080");
}

// === RequestType ===

#[test]
fn test_request_type_default() {
    let rt = RequestType::default();
    assert_eq!(rt, RequestType::Other);
}

// === RequestState ===

#[test]
fn test_request_state_default() {
    let state = RequestState::default();
    assert_eq!(state, RequestState::Unissued);
}
