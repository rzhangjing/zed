//! 可信服务器注册表。
//!
//! 维护一组可信的 host:port 条目。向这些服务器发出的
//! 任何请求都会附带凭据（cookies、授权头等）。

use std::collections::HashSet;

/// 一个可信服务器注册表。
///
/// 以 `host:port` 权限键为粒度管理可信集合。
#[derive(Debug, Default, Clone)]
pub struct TrustedServers {
    /// 已注册的可信服务器权限集合（小写 host:port）。
    servers: HashSet<String>,
}

impl TrustedServers {
    /// 创建一个空的新注册表。
    pub fn new() -> Self {
        Self {
            servers: HashSet::new(),
        }
    }

    /// 向注册表添加一个可信服务器。
    ///
    /// 权限键统一转小写并显式携带端口后插入集合。
    pub fn add(&mut self, host: &str, port: u16) {
        let authority = format!("{}:{}", host.to_lowercase(), port);
        self.servers.insert(authority);
    }

    /// 从注册表移除一个可信服务器。
    ///
    /// 按小写的 host:port 权限键从集合精确移除。
    pub fn remove(&mut self, host: &str, port: u16) {
        let authority = format!("{}:{}", host.to_lowercase(), port);
        self.servers.remove(&authority);
    }

    /// 返回某个 URL 是否可信。
    ///
    /// 先从 URL 提取权限键，再查表判断是否可信。
    pub fn is_trusted(&self, url: &str) -> bool {
        match Self::get_authority(url) {
            Some(authority) => self.servers.contains(&authority),
            None => false,
        }
    }

    /// 清除所有可信服务器。
    pub fn clear(&mut self) {
        self.servers.clear();
    }

    /// 返回可信服务器的数量。
    pub fn len(&self) -> usize {
        self.servers.len()
    }

    /// 返回注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// 从 URL 中提取权限（host:port）。
    ///
    /// 处理：
    /// - 带默认端口（80/443）的 http/https 协议
    /// - Username:password@ 前缀剔除
    /// - 协议相对 URL（//host/path）
    ///
    /// 对相对 URL 或未知协议返回 None。
    fn get_authority(url: &str) -> Option<String> {
        let url = url.trim();

        // 处理协议相对 URL
        // deferred.md #14: 手动 strip "//" 前缀，等价 url.strip_prefix("//")；风格问题。
        #[allow(clippy::manual_strip)]
        if url.starts_with("//") {
            let rest = &url[2..];
            let authority = rest.split('/').next().unwrap_or("");
            if authority.is_empty() {
                return None;
            }
            let authority = Self::strip_credentials(authority);
            // 无协议 → 无法确定默认端口
            if authority.contains(':') {
                return Some(authority.to_lowercase());
            }
            return None;
        }

        // 解析协议
        let scheme_end = url.find("://")?;
        let scheme = &url[..scheme_end].to_lowercase();
        let rest = &url[scheme_end + 3..];

        // 提取权限（首个 / 之前）
        let authority = rest.split('/').next().unwrap_or("");
        if authority.is_empty() {
            return None;
        }

        let authority = Self::strip_credentials(authority);

        // 若缺失则添加默认端口
        if authority.contains(':') {
            Some(authority.to_lowercase())
        } else {
            match scheme.as_str() {
                "http" => Some(format!("{}:80", authority.to_lowercase())),
                "https" => Some(format!("{}:443", authority.to_lowercase())),
                _ => None,
            }
        }
    }

    /// 从权限字符串中剔除 username:password@。
    fn strip_credentials(authority: &str) -> &str {
        if let Some(at_pos) = authority.find('@') {
            &authority[at_pos + 1..]
        } else {
            authority
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_is_trusted() {
        let mut ts = TrustedServers::new();
        ts.add("example.com", 80);
        assert!(ts.is_trusted("http://example.com/path"));
        assert!(!ts.is_trusted("http://other.com/path"));
    }

    #[test]
    fn test_remove() {
        let mut ts = TrustedServers::new();
        ts.add("example.com", 80);
        ts.remove("example.com", 80);
        assert!(!ts.is_trusted("http://example.com/path"));
    }

    #[test]
    fn test_default_port_https() {
        let mut ts = TrustedServers::new();
        ts.add("secure.com", 443);
        assert!(ts.is_trusted("https://secure.com/api"));
    }

    #[test]
    fn test_credentials_stripped() {
        let mut ts = TrustedServers::new();
        ts.add("example.com", 80);
        assert!(ts.is_trusted("http://user:pass@example.com/path"));
    }

    #[test]
    fn test_clear() {
        let mut ts = TrustedServers::new();
        ts.add("a.com", 80);
        ts.add("b.com", 443);
        ts.clear();
        assert!(ts.is_empty());
    }
}
