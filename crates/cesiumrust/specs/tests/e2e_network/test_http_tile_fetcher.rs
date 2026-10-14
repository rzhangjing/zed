//! 端到端骨架：针对 `wiremock::MockServer` 的 `HttpTileFetcher`。
//!
//! 今天验证域侧的 `FetchDescriptor` 构造；经由 `HttpTileFetcher`
//! 的真实 HTTP 往返被推迟至 M11.1（参见
//! docs/deferred.md "M8-wiremock skeleton"）。
//!
//! **M8.3 (#66) 接线证明**：下方的 [`m83_backend_types_are_wired`] 是一条
//! 未被忽略的编译期断言，确认真正的 `HttpTileFetcher` +
//! `NetworkResourceBackend` + `resource_fetch_backend_enabled` 类型可从
//! specs crate 访问。这使得 `cargo test --no-run` 成为对 M8.3 适配器
//! 接口存在的硬门槛，且无需 M11.1 将提供的
//! 异步框架。

use cesium_network::{
    resource_fetch_backend_enabled, HttpTileFetcher, NetworkResourceBackend,
    ENV_ENABLE_RESOURCE_FETCH_BACKEND,
};
use cesium_ports_driven::{ResourceBackend, TileFetcher};
use cesium_resource::{FetchDescriptor, HttpMethod, RequestType, Resource, ResponseType};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

/// M8.3 (#66) 编译期接线证明 —— **未被忽略**。
///
/// 断言真正的网络适配器接口可从 `cesium-specs` 访问：
/// * [`HttpTileFetcher`] 可被构造并持有一个
///   [`NetworkResourceBackend`]（M8.3 的委托目标）。
/// * [`NetworkResourceBackend`] 实现
///   [`cesium_ports_driven::ResourceBackend<u64>`]（dyn 兼容）。
/// * 环境变量未设置时 [`resource_fetch_backend_enabled`] 默认为 OFF
///   （使 v0 保持逐字节一致的黄金路径不变式）。
/// * [`ENV_ENABLE_RESOURCE_FETCH_BACKEND`] 是确切的门槛名（防止
///   意外重命名／与 M12 的
///   `ENV_ENABLE_RESOURCE_BACKEND` 混淆）。
///
/// 无需网络调用、无需 wiremock 挂载、无需异步运行时。
#[test]
fn m83_backend_types_are_wired() {
    // (a) M8.3 门槛名恰为独立的 M8.3 标识符，而非
    //     M12 的 `ENV_ENABLE_RESOURCE_BACKEND`。
    assert_eq!(
        ENV_ENABLE_RESOURCE_FETCH_BACKEND,
        "CESIUM_ENABLE_RESOURCE_FETCH_BACKEND"
    );
    assert_ne!(
        ENV_ENABLE_RESOURCE_FETCH_BACKEND,
        "CESIUM_ENABLE_RESOURCE_BACKEND",
        "M8.3 gate must NOT be conflated with the M12 Resource-backend flag"
    );

    // (b) 环境变量未设置时门槛默认 OFF（黄金路径
    //     不变式）。仅在周围环境变量干净时才断言此点，以免
    //     干扰那些合理切换门槛的并行测试。
    if std::env::var(ENV_ENABLE_RESOURCE_FETCH_BACKEND).is_err() {
        assert!(!resource_fetch_backend_enabled());
    }

    // (c) `HttpTileFetcher` 仍实现 `TileFetcher`（黄金路径
    //     所消费的端口），并可用默认 ureq agent
    //     构造。
    let fetcher = HttpTileFetcher::new("https://tiles.example.com");
    let _: &dyn TileFetcher = &fetcher;

    // (d) `NetworkResourceBackend` 作为
    //     `ResourceBackend<u64>` 是 dyn 兼容的 —— 这是门槛 ON 时
    //     `HttpTileFetcher::fetch` 会查询的 M8.3 委托目标。
    let backend = NetworkResourceBackend::new();
    let boxed: Box<dyn ResourceBackend<u64>> = Box::new(backend);
    assert_eq!(boxed.name(), "cesium-network-resource");
    assert!(boxed.is_available());
}

/// M8.4 (#67) 接线证明 —— **未被忽略**，运行真正的适配器执行。
///
/// 证明 M8.4 收敛门槛（`Resource::fetch` 全量切换收敛）：由
/// `Resource::fetch_*`/`post` 产生的无 IO 域描述符
/// 由适配器的 [`HttpTileFetcher::fetch_descriptor_blocking`] 执行 ——
/// 即那唯一的受门槛保护的入口 —— 而非散落于
/// 加载器中的临时 HTTP 调用。此处以零网络／无异步
/// 运行时演练两个分支：
/// * `data:` URI 描述符经纯域解码器短路，
/// * 非 GET（`post`）描述符浮现一个 `PortError::Network`，因为
///   `NetworkBackend::fetch(url)` trait 仅执行 GET（docs/deferred.md）。
///
/// 门槛 ON（后端）与 OFF（直连）的真实 HTTP 往返分支
/// 由 `adapters/network/src/lib.rs` 针对本地
/// 临时服务器的单元测试覆盖；下方 wiremock 驱动的端到端断言
/// 在 M11.1 提供异步框架之前保持 `#[ignore]`。
#[test]
fn m84_fetch_descriptor_executes_through_adapter() {
    use cesium_ports_driven::PortError;

    let fetcher = HttpTileFetcher::new("https://tiles.example.com");

    // (a) data: URI 描述符 -> 短路解码，零网络。
    //     "QUJDRA==" 是 "ABCD" 的 base64。
    let data_resource = Resource::new("data:application/octet-stream;base64,QUJDRA==");
    let data_descriptor = data_resource.fetch_array_buffer(None);
    assert!(data_descriptor.is_data_uri);
    let decoded = fetcher
        .fetch_descriptor_blocking(&data_descriptor)
        .expect("data URI decodes without network");
    assert_eq!(decoded, b"ABCD");

    // (b) 非 GET 描述符 -> 后端 trait 边界浮现一个错误，
    //     而非静默降级为 GET。
    let post_resource = Resource::new("https://tiles.example.com/api");
    let post_descriptor = post_resource.post(vec![1, 2, 3], None);
    assert!(matches!(post_descriptor.method, HttpMethod::Post));
    let err = fetcher
        .fetch_descriptor_blocking(&post_descriptor)
        .unwrap_err();
    assert!(matches!(err, PortError::Network(_)));

    // (c) 非 data URL 的 GET 描述符由门槛路由：当
    //     周围环境变量未设置时（黄金路径）它走逐字节一致的直连
    //     路径。断言门槛默认值；真实往返由
    //     adapters/network 中的本地服务器单元测试覆盖。
    let get_resource = Resource::new("https://tiles.example.com/tiles/0/0/0.b3dm");
    let get_descriptor = get_resource.fetch_array_buffer(None);
    assert!(matches!(get_descriptor.method, HttpMethod::Get));
    assert!(!get_descriptor.is_data_uri);
    if std::env::var(ENV_ENABLE_RESOURCE_FETCH_BACKEND).is_err() {
        assert!(!resource_fetch_backend_enabled());
    }
}

/// 骨架：经由真正 `HttpTileFetcher` 的一次 200 瓦片获取。
///
/// 下方的 `Mock`/`ResponseTemplate` 同步构造以证明
/// wiremock dev-dependency 可离线解析。挂载到 `MockServer` 并
/// 驱动 `HttpTileFetcher::fetch(url, priority)` 需要异步
/// 运行时（M11.1）。
#[test]
#[ignore = "M8-wiremock skeleton: needs MockServer async runtime + HttpTileFetcher wiring (M11.1). See docs/deferred.md."]
fn http_tile_fetcher_returns_200_bytes() {
    // ── wiremock 骨架（同步构造；异步挂载被推迟） ──
    let _mock = Mock::given(method("GET"))
        .and(path("/tiles/0/0/0.b3dm"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(b"glb-payload", "application/octet-stream"),
        );
    // TODO(M11.1): 先 `_mock.mount(&mock_server).await;`，再驱动
    // `HttpTileFetcher::fetch(&url, 0.0)` 并断言返回的字节。

    // ── 域侧断言（纯函数；已实现） ──
    let resource = Resource::new("https://tiles.example.com/tiles/0/0/0.b3dm");
    let descriptor: FetchDescriptor = resource
        .fetch_array_buffer(None)
        .with_request_type(RequestType::Tiles3D);

    assert_eq!(descriptor.method, HttpMethod::Get);
    assert_eq!(descriptor.response_type, ResponseType::ArrayBuffer);
    assert_eq!(descriptor.request_type, RequestType::Tiles3D);
    assert_eq!(descriptor.server_key, "tiles.example.com:443");
    assert!(!descriptor.is_data_uri);
}

/// 骨架：获取前对不受信任的服务器应用代理重写。
#[test]
#[ignore = "M8-wiremock skeleton: needs MockServer async runtime + HttpTileFetcher wiring (M11.1). See docs/deferred.md."]
fn http_tile_fetcher_applies_proxy_for_untrusted_server() {
    use cesium_resource::proxy::{DefaultProxy, ProxyPolicy};

    let policy =
        ProxyPolicy::with_proxy(DefaultProxy::new("https://proxy.example.com/".to_string()));

    let _mock = Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(b"proxied", "application/octet-stream"));

    let resource = Resource::new("https://untrusted.example.com/tile.b3dm");
    let descriptor = resource.fetch_array_buffer(Some(&policy));

    // 应用代理前缀，并将原始 URL 保留为查询参数。
    assert!(descriptor.url.starts_with("https://proxy.example.com/"));
    assert!(descriptor.url.contains("untrusted.example.com"));
}

/// 骨架：data: URI 完全短路网络。
#[test]
#[ignore = "M8-wiremock skeleton: needs MockServer async runtime + HttpTileFetcher wiring (M11.1). See docs/deferred.md."]
fn data_uri_never_hits_mock_server() {
    let resource = Resource::new("data:application/octet-stream;base64,QUJDRA==");
    let descriptor = resource.fetch_array_buffer(None);

    // 不会有 mock 被匹配：该描述符被标记为内联。
    assert!(descriptor.is_data_uri);
    assert!(descriptor.server_key.is_empty());
    assert!(descriptor.url.starts_with("data:"));
}
