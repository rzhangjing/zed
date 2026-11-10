//! 端到端骨架：通过调度器（以及最终的
//! `HttpTileFetcher`）消费 `priority_function`。
//!
//! 纯函数 `PriorityFunction` / `RequestScheduler::update_with_context`
//! 机制已在 `cesium-resource` 中实现并在此演练。将计算出的
//! 优先级接入 `HttpTileFetcher::fetch(url, priority)`——它
//! 目前忽略 `_priority`（`adapters/network/src/lib.rs:186`）——
//! 是 M8.3 (#66) + M11.1 的联合交付物。

use cesium_resource::priority::{
    DistanceDecayPriority, FrameContext, PriorityFunction, PriorityKey, SsedPriority,
};
use cesium_resource::{Request, RequestScheduler, RequestType};

/// 骨架：SSED 优先级将较近瓦片排在较远瓦片之前。
#[test]
#[ignore = "M8-wiremock skeleton: needs HttpTileFetcher to consume `_priority` (M8.3/#66 + M11.1). See docs/deferred.md."]
fn ssed_priority_orders_near_before_far() {
    let ssed = SsedPriority::new();
    let ctx = FrameContext::new().with_camera(0.0, 0.0, 6_378_137.0);

    let near = PriorityKey::new(0, 0, 18)
        .with_center(0.0, 0.0, 6_378_137.0 - 1_000.0)
        .with_geometric_error(20.0);
    let far = PriorityKey::new(0, 0, 10)
        .with_center(0.0, 0.0, 6_378_137.0 - 500_000.0)
        .with_geometric_error(20.0);

    let p_near = ssed.compute_priority(&near, &ctx);
    let p_far = ssed.compute_priority(&far, &ctx);
    // 值越小 = 优先级越高；较近瓦片必须排在最前。
    assert!(p_near <= p_far, "near={p_near} far={p_far}");
}

/// 骨架：调度器从帧上下文重新计算待处理优先级。
#[test]
#[ignore = "M8-wiremock skeleton: needs HttpTileFetcher to consume `_priority` (M8.3/#66 + M11.1). See docs/deferred.md."]
fn scheduler_recomputes_priority_with_context() {
    let mut scheduler = RequestScheduler::new();
    scheduler.maximum_requests = 0; // 使请求保持待处理
    let pf: Box<dyn PriorityFunction> = Box::new(SsedPriority::new());
    scheduler.set_priority_function(pf);
    assert_eq!(scheduler.priority_function_name(), Some("SSED"));

    let key = PriorityKey::new(1, 1, 14)
        .with_center(0.0, 0.0, 6_378_137.0 - 10_000.0)
        .with_geometric_error(75.0);

    let id = scheduler
        .schedule(
            Request::throttled(
                "https://tiles.example.com/1/1/14.b3dm".to_string(),
                RequestType::Tiles3D,
                12345.0, // 该函数必须覆盖的哨兵优先级
            )
            .with_priority_key(key),
        )
        .unwrap();

    let ctx = FrameContext::new().with_camera(0.0, 0.0, 6_378_137.0);
    scheduler.update_with_context(&ctx);

    let request = scheduler.get_request(id).expect("request is pending");
    assert!(
        (request.priority - 12345.0).abs() > f64::EPSILON,
        "priority function should have recomputed the sentinel value"
    );
}

/// 骨架：距离衰减优先级是一种有效的替代信号。
#[test]
#[ignore = "M8-wiremock skeleton: needs HttpTileFetcher to consume `_priority` (M8.3/#66 + M11.1). See docs/deferred.md."]
fn distance_decay_priority_is_monotonic() {
    let decay = DistanceDecayPriority::new();
    let ctx = FrameContext::new().with_camera(0.0, 0.0, 6_378_137.0);

    let near = PriorityKey::new(0, 0, 16).with_center(0.0, 0.0, 6_378_137.0 - 500.0);
    let far = PriorityKey::new(0, 0, 8).with_center(0.0, 0.0, 6_378_137.0 - 900_000.0);

    let p_near = decay.compute_priority(&near, &ctx);
    let p_far = decay.compute_priority(&far, &ctx);
    assert!(p_near <= p_far, "near={p_near} far={p_far}");
}
