//! 带可信服务器集成的代理 URL 重写。
//!
//! 提供 [`DefaultProxy`] 将资源 URL 前置代理地址，以及
//! [`ProxyPolicy`] 依据可信服务器注册表决定是否代理的
//! 逻辑，等价于资源 URL 组件构造时的代理分支处理。
//!
//! [`DefaultProxy`] 会在资源 URL 前拼上一个代理 URL，以便
//! 将跨源请求路由到一个同源服务器。当配置了
//! [`TrustedServers`] 注册表时，代理 **仅应用于不可信**
//! 的服务器 —— 可信服务器会被直接访问（凭据
//! 无需 CORS 预检即可流动）。
//!
//! 本模块是 **纯领域逻辑** —— 无网络 IO，无框架
//! 依赖。

use crate::trusted_servers::TrustedServers;

/// 一个简单的代理，它将所需的资源 URL 作为唯一的查询
/// 参数拼接到代理基础 URL 之后。
///
/// 拼接规则：若代理基础 URL 不含 `?` 则插入 `?` 分隔符，
/// 否则直接拼接资源 URL。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultProxy {
    /// 代理基础 URL（例如 `/proxy/` 或 `https://proxy.example.com/?url=`）。
    proxy_url: String,
}

impl DefaultProxy {
    /// 从给定的代理基础 URL 创建一个新代理。
    ///
    /// # Panic
    /// 若 `proxy_url` 为空则 panic（拒绝空代理地址）。
    pub fn new(proxy_url: impl Into<String>) -> Self {
        let url = proxy_url.into();
        assert!(!url.is_empty(), "DefaultProxy: proxy URL must not be empty");
        Self { proxy_url: url }
    }

    /// 返回代理基础 URL。
    pub fn proxy_url(&self) -> &str {
        &self.proxy_url
    }

    /// 重写 `resource_url` 使其经由本代理。
    ///
    /// 若代理 URL 已包含 `?`，则直接拼接资源（假设代理
    /// 期望该 URL 作为最后一个查询参数
    /// 值）。否则插入一个 `?` 分隔符。
    pub fn get_url(&self, resource_url: &str) -> String {
        let prefix = if self.proxy_url.contains('?') { "" } else { "?" };
        format!("{}{}{}", self.proxy_url, prefix, resource_url)
    }
}

/// 一个代理策略，根据可信服务器注册表
/// 决定是否代理某个给定 URL。
///
/// 核心规则：仅当配置了代理 **且** 目标服务器不在
/// `TrustedServers` 中时才应用代理——可信服务器直连。
///
/// 该策略封装了这一决策，使调用方无需知道
/// 可信服务器的检查。
#[derive(Debug, Clone)]
pub struct ProxyPolicy {
    /// 用于不可信服务器的代理（None = 从不代理）。
    proxy: Option<DefaultProxy>,
    /// 可信服务器注册表（直接访问，允许凭据）。
    trusted_servers: TrustedServers,
}

impl ProxyPolicy {
    /// 创建一个不带代理的策略（所有 URL 原样通过）。
    pub fn no_proxy() -> Self {
        Self {
            proxy: None,
            trusted_servers: TrustedServers::new(),
        }
    }

    /// 创建一个将不可信服务器经由 `proxy` 代理的策略。
    pub fn with_proxy(proxy: DefaultProxy) -> Self {
        Self {
            proxy: Some(proxy),
            trusted_servers: TrustedServers::new(),
        }
    }

    /// 创建一个同时带有代理和已预填可信服务器
    /// 注册表的策略。
    pub fn new(proxy: Option<DefaultProxy>, trusted_servers: TrustedServers) -> Self {
        Self {
            proxy,
            trusted_servers,
        }
    }

    /// 返回可信服务器注册表的可变引用，以便宿主
    /// 在运行时添加/移除条目。
    pub fn trusted_servers_mut(&mut self) -> &mut TrustedServers {
        &mut self.trusted_servers
    }

    /// 返回可信服务器注册表的引用。
    pub fn trusted_servers(&self) -> &TrustedServers {
        &self.trusted_servers
    }

    /// 返回代理的引用（若已配置）。
    pub fn proxy(&self) -> Option<&DefaultProxy> {
        self.proxy.as_ref()
    }

    /// 设置或替换代理。
    pub fn set_proxy(&mut self, proxy: Option<DefaultProxy>) {
        self.proxy = proxy;
    }

    /// 判断 `url` 是否应被代理。
    ///
    /// 在以下情况下返回 `true`：
    /// 1. 配置了代理，且
    /// 2. URL 的服务器不在可信注册表中。
    ///
    /// Data URI 和 blob URI 从不代理（它们没有服务器）。
    pub fn should_proxy(&self, url: &str) -> bool {
        if self.proxy.is_none() {
            return false;
        }
        // Data URI 和 blob URI 绕过代理。
        if url.starts_with("data:") || url.starts_with("blob:") {
            return false;
        }
        !self.trusted_servers.is_trusted(url)
    }

    /// 若策略认为应代理，则将代理应用到 `url`。
    /// 否则原样返回该 URL。
    ///
    /// 这是 Resource 的 URL 构建管道的唯一入口点：
    /// ```ignore
    /// let final_url = policy.apply(&resource.build_url());
    /// ```
    pub fn apply(&self, url: &str) -> String {
        if self.should_proxy(url) {
            // 可安全 unwrap：should_proxy 仅在 proxy 为 Some 时返回 true。
            self.proxy.as_ref().unwrap().get_url(url)
        } else {
            url.to_string()
        }
    }

    /// 应用代理，并额外透传应拼接到 *原始*（代理前）
    /// URL 上的附加查询参数。
    ///
    /// 用于支持“先把查询参数拼到原始 URL，再整体交给
    /// 代理包裹”的模式：
    /// ```text
    /// proxy.getURL(baseUrl + "?" + queryString)
    /// ```
    pub fn apply_with_query(&self, base_url: &str, query_string: &str) -> String {
        let url = if query_string.is_empty() {
            base_url.to_string()
        } else if base_url.contains('?') {
            format!("{}&{}", base_url, query_string)
        } else {
            format!("{}?{}", base_url, query_string)
        };
        self.apply(&url)
    }
}

impl Default for ProxyPolicy {
    /// 默认策略：不启用任何代理（直连）。
    fn default() -> Self {
        Self::no_proxy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_proxy_appends_with_question_mark() {
        let proxy = DefaultProxy::new("https://proxy.example.com/");
        assert_eq!(
            proxy.get_url("https://tiles.example.com/0/0/0.png"),
            "https://proxy.example.com/?https://tiles.example.com/0/0/0.png"
        );
    }

    #[test]
    fn default_proxy_appends_without_extra_question_mark() {
        let proxy = DefaultProxy::new("https://proxy.example.com/?url=");
        assert_eq!(
            proxy.get_url("https://tiles.example.com/a.png"),
            "https://proxy.example.com/?url=https://tiles.example.com/a.png"
        );
    }

    #[test]
    #[should_panic(expected = "proxy URL must not be empty")]
    fn default_proxy_panics_on_empty() {
        DefaultProxy::new("");
    }

    #[test]
    fn policy_no_proxy_passes_through() {
        let policy = ProxyPolicy::no_proxy();
        assert_eq!(
            policy.apply("https://example.com/tile.png"),
            "https://example.com/tile.png"
        );
    }

    #[test]
    fn policy_proxies_untrusted_server() {
        let proxy = DefaultProxy::new("/proxy");
        let policy = ProxyPolicy::with_proxy(proxy);
        assert_eq!(
            policy.apply("https://untrusted.com/data"),
            "/proxy?https://untrusted.com/data"
        );
    }

    #[test]
    fn policy_skips_proxy_for_trusted_server() {
        let proxy = DefaultProxy::new("/proxy");
        let mut ts = TrustedServers::new();
        ts.add("trusted.com", 443);
        let policy = ProxyPolicy::new(Some(proxy), ts);

        // 可信：不代理
        assert_eq!(
            policy.apply("https://trusted.com/data"),
            "https://trusted.com/data"
        );
        // 不可信：代理
        assert_eq!(
            policy.apply("https://other.com/data"),
            "/proxy?https://other.com/data"
        );
    }

    #[test]
    fn policy_never_proxies_data_uri() {
        let proxy = DefaultProxy::new("/proxy");
        let policy = ProxyPolicy::with_proxy(proxy);
        assert_eq!(
            policy.apply("data:text/plain,hello"),
            "data:text/plain,hello"
        );
    }

    #[test]
    fn policy_never_proxies_blob_uri() {
        let proxy = DefaultProxy::new("/proxy");
        let policy = ProxyPolicy::with_proxy(proxy);
        assert_eq!(
            policy.apply("blob:https://example.com/uuid"),
            "blob:https://example.com/uuid"
        );
    }

    #[test]
    fn apply_with_query_builds_correct_url() {
        let policy = ProxyPolicy::no_proxy();
        assert_eq!(
            policy.apply_with_query("https://example.com/api", "key=value&foo=bar"),
            "https://example.com/api?key=value&foo=bar"
        );
        // 已有查询
        assert_eq!(
            policy.apply_with_query("https://example.com/api?existing=1", "key=value"),
            "https://example.com/api?existing=1&key=value"
        );
        // 空查询
        assert_eq!(
            policy.apply_with_query("https://example.com/api", ""),
            "https://example.com/api"
        );
    }

    #[test]
    fn apply_with_query_proxies_result() {
        let proxy = DefaultProxy::new("/proxy");
        let policy = ProxyPolicy::with_proxy(proxy);
        assert_eq!(
            policy.apply_with_query("https://untrusted.com/api", "key=value"),
            "/proxy?https://untrusted.com/api?key=value"
        );
    }
}
