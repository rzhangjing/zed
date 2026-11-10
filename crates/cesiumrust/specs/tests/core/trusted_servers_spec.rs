//! TrustedServers 规格测试 - 参考自 Specs/Core/TrustedServersSpec
//!
//! A 类测试：8 个（纯逻辑，无浏览器/DOM）

use cesium_resource::trusted_servers::TrustedServers;

#[cfg(test)]
mod tests {
    use super::*;

    /// "http without a port"
    #[test]
    fn http_without_a_port() {
        let mut ts = TrustedServers::new();
        ts.add("cesiumjs.org", 80);
        assert!(ts.is_trusted("http://cesiumjs.org/index.html"));
        assert!(!ts.is_trusted("https://cesiumjs.org/index.html"));
    }

    /// "https without a port"
    #[test]
    fn https_without_a_port() {
        let mut ts = TrustedServers::new();
        ts.add("cesiumjs.org", 443);
        assert!(ts.is_trusted("https://cesiumjs.org/index.html"));
        assert!(!ts.is_trusted("http://cesiumjs.org/index.html"));
    }

    /// "add"
    #[test]
    fn add_with_explicit_port() {
        let mut ts = TrustedServers::new();
        assert!(!ts.is_trusted("http://cesiumjs.org:81/index.html"));
        ts.add("cesiumjs.org", 81);
        // 默认端口 80 不应匹配显式端口 81
        assert!(!ts.is_trusted("http://cesiumjs.org/index.html"));
        assert!(ts.is_trusted("http://cesiumjs.org:81/index.html"));
    }

    /// "remove"
    #[test]
    fn remove_server() {
        let mut ts = TrustedServers::new();
        ts.add("cesiumjs.org", 81);
        assert!(ts.is_trusted("http://cesiumjs.org:81/index.html"));
        // 移除错误的端口不应产生影响
        ts.remove("cesiumjs.org", 8080);
        assert!(ts.is_trusted("http://cesiumjs.org:81/index.html"));
        // 移除正确的端口
        ts.remove("cesiumjs.org", 81);
        assert!(!ts.is_trusted("http://cesiumjs.org:81/index.html"));
    }

    /// "handles username/password credentials"
    #[test]
    fn handles_credentials() {
        let mut ts = TrustedServers::new();
        ts.add("cesiumjs.org", 81);
        assert!(ts.is_trusted("http://user:pass@cesiumjs.org:81/index.html"));
    }

    /// "always returns false for relative paths"
    #[test]
    fn relative_paths_return_false() {
        let ts = TrustedServers::new();
        assert!(!ts.is_trusted("./data/index.html"));
    }

    /// "handles protocol relative URLs"
    #[test]
    fn protocol_relative_urls() {
        let mut ts = TrustedServers::new();
        ts.add("cesiumjs.org", 80);
        // 协议相对 URL 且无端口 → 无法确定默认端口
        // CesiumJS 使用 window.location.protocol，我们返回 false
        // 但带有显式端口时应可正常工作
        assert!(!ts.is_trusted("//cesiumjs.org/index.html"));
        // 带有显式端口
        ts.add("cesiumjs.org", 8080);
        assert!(ts.is_trusted("//cesiumjs.org:8080/index.html"));
    }

    /// "clear"
    #[test]
    fn clear_all() {
        let mut ts = TrustedServers::new();
        ts.add("cesiumjs.org", 80);
        assert!(ts.is_trusted("http://cesiumjs.org/index.html"));
        ts.clear();
        assert!(!ts.is_trusted("http://cesiumjs.org/index.html"));
        // clear 之后可以再次添加
        ts.add("cesiumjs.org", 80);
        assert!(ts.is_trusted("http://cesiumjs.org/index.html"));
    }
}
