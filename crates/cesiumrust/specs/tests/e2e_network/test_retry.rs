//! 端到端骨架：由 mock 服务器失败驱动的 `retry_callback` 语义。
//!
//! 纯函数 `RetryPolicy` / `RetryContext` / `default_retry_callback` 的决策
//! 已在 `cesium-resource` 中实现并在此断言。端到端
//! 循环（mock 返回 5xx/429 → `HttpTileFetcher` 以退避重试）
//! 被推迟至 M11.1。

use cesium_resource::{
    classify_status, default_retry_callback, BackoffStrategy, RequestErrorClass, RetryContext,
    RetryDecision, RetryPolicy,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

/// 骨架：先 503 后 200 的序列驱动一次重试并成功。
#[test]
#[ignore = "M8-wiremock skeleton: needs MockServer async runtime + HttpTileFetcher retry wiring (M11.1). See docs/deferred.md."]
fn transient_failure_retries_then_succeeds() {
    // 首次调用 503，第二次 200——wiremock 的 `respond_with` 序列在 M11.1 落地。
    let _mock = Mock::given(method("GET"))
        .and(path("/flaky/tile.b3dm"))
        .respond_with(ResponseTemplate::new(503));
    // TODO(M11.1): 挂载并以 RetryPolicy::exponential 驱动 HttpTileFetcher。

    // ── 域侧重试决策（纯函数） ──
    let policy = RetryPolicy::exponential(3, 100);
    let ctx = RetryContext::from_status("https://x.com/flaky/tile.b3dm", 0, 503, policy.clone());
    assert_eq!(ctx.error_class, RequestErrorClass::Transient);
    // 首次失败、100ms 指数基数 → 100ms 后重试。
    assert_eq!(ctx.default_decision(), RetryDecision::RetryAfterMillis(100));
}

/// 骨架：404 从不重试（永久性客户端错误）。
#[test]
#[ignore = "M8-wiremock skeleton: needs MockServer async runtime + HttpTileFetcher retry wiring (M11.1). See docs/deferred.md."]
fn permanent_failure_does_not_retry() {
    let _mock = Mock::given(method("GET"))
        .and(path("/missing/tile.b3dm"))
        .respond_with(ResponseTemplate::new(404));

    assert_eq!(classify_status(404), RequestErrorClass::Permanent);
    let policy = RetryPolicy::exponential(5, 100);
    assert_eq!(
        policy.decide(0, RequestErrorClass::Permanent),
        RetryDecision::GiveUp
    );
}

/// 骨架：429 被归类为限流，并遵循 `retry_on_throttled`。
#[test]
#[ignore = "M8-wiremock skeleton: needs MockServer async runtime + HttpTileFetcher retry wiring (M11.1). See docs/deferred.md."]
fn throttled_429_honours_retry_flag() {
    let _mock = Mock::given(method("GET"))
        .and(path("/throttled/tile.b3dm"))
        .respond_with(ResponseTemplate::new(429));

    assert_eq!(classify_status(429), RequestErrorClass::Throttled);

    let mut policy = RetryPolicy::default();
    policy.backoff = BackoffStrategy::Fixed;
    assert_eq!(
        policy.decide(0, RequestErrorClass::Throttled),
        RetryDecision::RetryNow
    );

    policy.retry_on_throttled = false;
    assert_eq!(
        policy.decide(0, RequestErrorClass::Throttled),
        RetryDecision::GiveUp
    );
}

/// 骨架：可插拔的 `retry_callback` 闭包正是适配器所调用的。
#[test]
#[ignore = "M8-wiremock skeleton: needs MockServer async runtime + HttpTileFetcher retry wiring (M11.1). See docs/deferred.md."]
fn default_retry_callback_applies_policy() {
    let callback = default_retry_callback();
    let ctx = RetryContext::from_status("https://x.com/a", 0, 500, RetryPolicy::default());
    // 默认策略：max_attempts=1、无延迟 → 首个 5xx 立即重试。
    assert_eq!(callback(&ctx), RetryDecision::RetryNow);

    // 尝试次数耗尽后放弃。
    let ctx2 = RetryContext::from_status("https://x.com/a", 1, 500, RetryPolicy::default());
    assert_eq!(callback(&ctx2), RetryDecision::GiveUp);
}
