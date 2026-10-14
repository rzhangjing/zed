//! 具备 STRICT_OFFLINE 语义的离线磁盘支撑瓦片获取器。
//!
//! 通过从本地目录读取瓦片字节来实现 [`TileFetcher`] 驱动端口。支持两种
//! 磁盘布局（由 [`FileTileScheme`] 选择）：
//!
//! * [`FileTileScheme::Xyz`] —— 规范的 `{root}/{level}/{x}/{y}.{ext}`
//!   金字塔布局，被 viewer-demo 离线影像 fixture 以及 CesiumJS 的
//!   `NaturalEarthII` 资源使用（参考实现：
//!   `cesium-rs/examples/viewer-demo/src/main.rs` L601-623）。
//! * [`FileTileScheme::Quadkey`] —— Bing 风格的单段 quadkey，以
//!   `{root}/{quadkey}.{ext}` 形式存储（quadkey 数字 `0`-`3`，层级 = 数字位数）。
//!
//! # STRICT_OFFLINE 契约
//!
//! 离线 viewer-demo 路径绝不回退到 HTTP。当启用
//! [`FileTileFetcher::with_strict_offline`] 且调用方将一个 `http://` 或
//! `https://` URL 传给 [`TileFetcher::fetch`] 时，获取器会*同步地*立即
//! panic —— 甚至在返回的 future 被构造之前 —— 因此该违规会响亮地暴露，
//! 无法被 async 运行时静默吞掉。
//!
//! # 异步形态
//!
//! [`TileFetcher::fetch`] 为端口兼容性返回一个 boxed future，但磁盘读取本身
//! 只是 `async move` 块内一次朴素的 [`std::fs::read`]。没有 tokio 运行时，
//! 没有 `spawn_blocking`：该 future 是 `Send` 的，并在首次 poll 时即完成，
//! 这与「离线 = 同步本地 IO」的契约相符。

use cesium_ports_driven::{PortError, PortResult, TileFetcher};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// 从传给 [`TileFetcher::fetch`] 的 URL 解释出的瓦片寻址方案。
///
/// 该方案决定 URL 尾部如何映射到 [`FileTileFetcher::root`] 下的磁盘路径：
///
/// | 方案      | URL 尾部              | 磁盘路径                          |
/// |----------|---------------------|---------------------------------|
/// | `Xyz`    | `0/0/0[.ext]`       | `{root}/0/0/0.{ext}`            |
/// | `Quadkey`| `120[.ext]`         | `{root}/120.{ext}`              |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTileScheme {
    /// `{level}/{x}/{y}` XYZ 金字塔（第 0 行位于北极）。
    Xyz,
    /// Bing 风格的 quadkey 字符串（数字 `0`-`3`，层级 = 数字位数）。
    Quadkey,
}

/// 从本地目录读取字节的离线瓦片获取器。
///
/// 可克隆、`Send + Sync`，且不含任何 HTTP 依赖 —— 无需运行时即可跨线程安全共享。
#[derive(Debug, Clone)]
pub struct FileTileFetcher {
    /// 瓦片金字塔在磁盘上的根目录，所有相对 URL 尾部都相对它 join。
    root: PathBuf,
    /// URL 尾部到磁盘路径的映射方案（XYZ 金字塔或 quadkey）。
    scheme: FileTileScheme,
    /// STRICT_OFFLINE 开关：为真时收到 `http(s)://` URL 立即 panic。
    strict_offline: bool,
    /// 调用方省略扩展名时追加的默认瓦片文件扩展名（缺省 `png`）。
    extension: String,
}

impl FileTileFetcher {
    /// 创建一个以 `root` 为根、使用给定 URL 方案的获取器。
    ///
    /// 默认扩展名为 `png`（viewer-demo 离线影像 fixture 写入
    /// `{level}/{x}/{y}.png`）。当瓦片集使用 `jpg`/`jpeg` 时，用
    /// [`Self::with_extension`] 覆盖。
    pub fn new(root: impl AsRef<Path>, scheme: FileTileScheme) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            scheme,
            strict_offline: false,
            extension: "png".to_string(),
        }
    }

    /// 启用 STRICT_OFFLINE 语义：任何传给 [`TileFetcher::fetch`] 的
    /// `http://` 或 `https://` URL 都会触发立即 panic。
    ///
    /// 这是离线 viewer-demo 契约 —— 不存在 HTTP 回退路径，因此网络 URL
    /// 到达此获取器是一个接线错误，必须响亮地失败。
    pub fn with_strict_offline(mut self, strict: bool) -> Self {
        self.strict_offline = strict;
        self
    }

    /// 覆盖默认的瓦片文件扩展名（默认：`png`）。
    pub fn with_extension(mut self, ext: impl Into<String>) -> Self {
        self.extension = ext.into();
        self
    }

    /// 瓦片金字塔的根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 当前启用的 URL 方案。
    pub fn scheme(&self) -> FileTileScheme {
        self.scheme
    }

    /// STRICT_OFFLINE 语义是否处于激活状态。
    pub fn strict_offline(&self) -> bool {
        self.strict_offline
    }

    /// 将一个 URL 解析为 [`Self::root`] 下的磁盘路径。
    ///
    /// # Panic
    ///
    /// 当 [`Self::strict_offline`] 为 `true` 且 `url` 以 `http://` 或
    /// `https://` 开头时 panic —— STRICT_OFFLINE 契约禁止任何网络回退。
    fn resolve(&self, url: &str) -> PortResult<PathBuf> {
        if self.strict_offline && is_http_url(url) {
            panic!(
                "STRICT_OFFLINE violation: FileTileFetcher received network URL '{}' \
                 (offline mode forbids HTTP fallback; wire the offline root instead)",
                url
            );
        }
        let tail = strip_scheme_and_host(url);
        if tail.is_empty() {
            return Err(PortError::NotFound(format!(
                "FileTileFetcher: empty tile path after stripping scheme from '{}'",
                url
            )));
        }
        // 绝对路径（POSIX `/...` 或 Windows `C:\...`）绕过与 root 的 join，
        // 以便 `file:///abs/path.png` 这类 URL 能正确解析。
        let candidate = Path::new(tail);
        let path = if candidate.is_absolute() || has_windows_drive(tail) {
            candidate.to_path_buf()
        } else {
            self.root.join(tail)
        };
        // 当调用方省略扩展名时追加配置的扩展名，以便
        // `fetch("0/0/0")` 和 `fetch("0/0/0.png")` 都能解析。
        if path.extension().is_none() {
            return Ok(path.with_extension(&self.extension));
        }
        Ok(path)
    }
}

impl TileFetcher for FileTileFetcher {
    /// 从本地磁盘读取一个瓦片的原始字节。
    ///
    /// - `url`：待获取的瓦片 URL（可为 `file://`、裸相对路径或绝对路径）；
    ///   `_priority`：仅为端口兼容而保留，本地读取不使用优先级。
    ///
    /// 返回一个解析为 [`Vec<u8>`] 的 boxed future。当 [`Self::strict_offline`]
    /// 开启且 `url` 为 `http(s)://` 时*同步地*立即 panic（早于 future 构造）。
    /// 磁盘读取为朴素 [`std::fs::read`]，future 在首次 poll 时即完成。
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        _priority: f64,
    ) -> Pin<Box<dyn Future<Output = PortResult<Vec<u8>>> + Send + 'a>> {
        // 同步解析，以便 STRICT_OFFLINE 违规在 future 被构造之前即 panic
        // （响亮失败，不被 async 吞掉）。
        let path = match self.resolve(url) {
            Ok(p) => p,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        Box::pin(async move {
            std::fs::read(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    PortError::NotFound(format!(
                        "FileTileFetcher: tile not found at '{}'",
                        path.display()
                    ))
                } else {
                    PortError::Network(format!(
                        "FileTileFetcher: IO error reading '{}': {}",
                        path.display(),
                        e
                    ))
                }
            })
        })
    }

    /// 取消一个进行中的获取。磁盘读取是同步且不可取消的（future 首次 poll
    /// 即完成），因此这里无实际动作，仅为满足 [`TileFetcher`] 契约。
    fn cancel(&self, _url: &str) {
        // 磁盘读取是同步且不可取消的；future 在首次 poll 时即完成，
        // 因此没有可取消的东西。
    }
}

/// 当 `url` 以 `http://` 或 `https://` 开头时返回 `true`
/// （大小写不敏感）。
fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// 剥离 `scheme://host` 前缀，返回路径尾部。
///
/// * `file:///abs/path.png` → `/abs/path.png`（保留前导斜杠，使路径仍为绝对）。
/// * `https://host/a/b.png` → `a/b.png`（剥离 host）。
/// * `a/b.png`（无 scheme）→ `a/b.png`（原样返回）。
fn strip_scheme_and_host(url: &str) -> &str {
    let Some(idx) = url.find("://") else {
        return url;
    };
    let scheme = &url[..idx];
    let after = &url[idx + 3..];
    // `file://` URL 保留前导斜杠以在 POSIX 上保持绝对；在 Windows 上，
    // `file:///C:/path` 产生 `/C:/path`，其前导斜杠必须去掉，好让盘符
    // 成为路径前缀。
    if scheme.eq_ignore_ascii_case("file") {
        if let Some(slash) = after.find('/') {
            return strip_windows_drive_slash(&after[slash..]);
        }
        return after;
    }
    // http(s)://host/path → 丢弃 host，保留首个 '/' 之后的路径。
    match after.find('/') {
        Some(slash) => &after[slash + 1..],
        None => "",
    }
}

/// 在 Windows 上，`file:///C:/path` URL 产生的尾部为 `/C:/path`；必须去掉
/// 前导斜杠，好让盘符成为路径前缀。POSIX 尾部（`/abs/path`）原样返回。
fn strip_windows_drive_slash(tail: &str) -> &str {
    let b = tail.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[2] == b':' && b[1].is_ascii_alphabetic() {
        &tail[1..]
    } else {
        tail
    }
}

/// 检测 `s` 开头是否为类似 `C:` 的 Windows 盘符前缀。
fn has_windows_drive(s: &str) -> bool {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), Some(':')) => c.is_ascii_alphabetic(),
        _ => false,
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cesium-file-tile-fetcher-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    // --- URL 辅助函数 ---------------------------------------------------------

    #[test]
    fn test_is_http_url() {
        assert!(is_http_url("http://example.com/a.png"));
        assert!(is_http_url("HTTPS://example.com/a.png"));
        assert!(!is_http_url("file:///abs/a.png"));
        assert!(!is_http_url("0/0/0.png"));
        assert!(!is_http_url("/abs/a.png"));
    }

    #[test]
    fn test_strip_scheme_and_host_file() {
        assert_eq!(strip_scheme_and_host("file:///abs/a.png"), "/abs/a.png");
        assert_eq!(strip_scheme_and_host("file://host/a.png"), "/a.png");
    }

    #[test]
    fn test_strip_scheme_and_host_https() {
        assert_eq!(
            strip_scheme_and_host("https://example.com/tiles/0/0/0.png"),
            "tiles/0/0/0.png"
        );
        assert_eq!(strip_scheme_and_host("https://example.com"), "");
    }

    #[test]
    fn test_strip_scheme_and_host_plain() {
        assert_eq!(strip_scheme_and_host("0/0/0.png"), "0/0/0.png");
    }

    #[test]
    fn test_has_windows_drive() {
        assert!(has_windows_drive("C:/tiles"));
        assert!(has_windows_drive("d:\\tiles"));
        assert!(!has_windows_drive("/tiles"));
        assert!(!has_windows_drive("0/0/0"));
    }

    // --- resolve（路径解析）-------------------------------------------------------------

    #[test]
    fn test_resolve_xyz_relative_appends_extension() {
        let dir = unique_temp_dir("resolve-xyz");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        let path = fetcher.resolve("0/0/0").unwrap();
        assert_eq!(path, dir.join("0/0/0.png"));
    }

    #[test]
    fn test_resolve_xyz_relative_keeps_extension() {
        let dir = unique_temp_dir("resolve-xyz-ext");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        let path = fetcher.resolve("1/2/3.jpg").unwrap();
        assert_eq!(path, dir.join("1/2/3.jpg"));
    }

    #[test]
    fn test_resolve_quadkey_relative() {
        let dir = unique_temp_dir("resolve-qk");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Quadkey);
        let path = fetcher.resolve("120").unwrap();
        assert_eq!(path, dir.join("120.png"));
    }

    #[test]
    fn test_resolve_custom_extension() {
        let dir = unique_temp_dir("resolve-ext");
        let fetcher =
            FileTileFetcher::new(&dir, FileTileScheme::Xyz).with_extension("jpeg");
        let path = fetcher.resolve("0/0/0").unwrap();
        assert_eq!(path, dir.join("0/0/0.jpeg"));
    }

    #[test]
    fn test_resolve_file_url_absolute() {
        let dir = unique_temp_dir("resolve-file-url");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        // 指向 root 之外的 file:// URL 必须保持绝对。
        let abs = dir.join("abs.png");
        let url = format!("file:///{}", abs.display().to_string().replace('\\', "/"));
        let path = fetcher.resolve(&url).unwrap();
        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap(), "abs.png");
    }

    #[test]
    fn test_resolve_https_tail_under_root_when_not_strict() {
        let dir = unique_temp_dir("resolve-https-tail");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        // 非严格模式：https 尾部会被重新基准化到离线 root 之下。
        let path = fetcher
            .resolve("https://example.com/tiles/0/0/0.png")
            .unwrap();
        assert_eq!(path, dir.join("tiles/0/0/0.png"));
    }

    #[test]
    fn test_resolve_empty_tail_errors() {
        let dir = unique_temp_dir("resolve-empty");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        let err = fetcher.resolve("https://example.com").unwrap_err();
        assert!(matches!(err, PortError::NotFound(_)));
    }

    // --- STRICT_OFFLINE panic ------------------------------------------------

    #[test]
    #[should_panic(expected = "STRICT_OFFLINE violation")]
    fn test_strict_offline_panics_on_https() {
        let dir = unique_temp_dir("strict-https");
        let fetcher =
            FileTileFetcher::new(&dir, FileTileScheme::Xyz).with_strict_offline(true);
        // 在 `resolve` 内部同步 panic，先于 future 被构造。
        let _ = fetcher.resolve("https://example.com/0/0/0.png");
    }

    #[test]
    #[should_panic(expected = "STRICT_OFFLINE violation")]
    fn test_strict_offline_panics_on_http() {
        let dir = unique_temp_dir("strict-http");
        let fetcher =
            FileTileFetcher::new(&dir, FileTileScheme::Xyz).with_strict_offline(true);
        let _ = fetcher.resolve("http://example.com/0/0/0.png");
    }

    #[test]
    fn test_strict_offline_allows_file_and_relative() {
        let dir = unique_temp_dir("strict-ok");
        let fetcher =
            FileTileFetcher::new(&dir, FileTileScheme::Xyz).with_strict_offline(true);
        // 在 STRICT_OFFLINE 下，相对路径和 file:// URL 都是允许的。
        assert!(fetcher.resolve("0/0/0").is_ok());
        assert!(fetcher
            .resolve(&format!(
                "file:///{}",
                dir.join("a.png").display().to_string().replace('\\', "/")
            ))
            .is_ok());
    }

    #[tokio::test]
    #[should_panic(expected = "STRICT_OFFLINE violation")]
    async fn test_strict_offline_panics_via_fetch() {
        let dir = unique_temp_dir("strict-fetch");
        let fetcher =
            FileTileFetcher::new(&dir, FileTileScheme::Xyz).with_strict_offline(true);
        // panic 在 `fetch` 内部同步触发，因此是否 await 无关紧要。
        let _ = fetcher.fetch("https://example.com/0/0/0.png", 1.0).await;
    }

    // --- 端到端获取 ----------------------------------------------------

    #[tokio::test]
    async fn test_fetch_xyz_returns_bytes() {
        let dir = unique_temp_dir("fetch-xyz");
        std::fs::create_dir_all(dir.join("0/0")).unwrap();
        std::fs::write(dir.join("0/0/0.png"), b"tile-bytes").unwrap();

        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        let data = fetcher.fetch("0/0/0", 1.0).await.unwrap();
        assert_eq!(data, b"tile-bytes");
    }

    #[tokio::test]
    async fn test_fetch_xyz_with_extension_in_url() {
        let dir = unique_temp_dir("fetch-xyz-ext");
        std::fs::create_dir_all(dir.join("2/3")).unwrap();
        std::fs::write(dir.join("2/3/1.jpg"), b"jpeg-bytes").unwrap();

        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        let data = fetcher.fetch("2/3/1.jpg", 1.0).await.unwrap();
        assert_eq!(data, b"jpeg-bytes");
    }

    #[tokio::test]
    async fn test_fetch_quadkey_returns_bytes() {
        let dir = unique_temp_dir("fetch-qk");
        std::fs::write(dir.join("120.png"), b"qk-bytes").unwrap();

        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Quadkey);
        let data = fetcher.fetch("120", 1.0).await.unwrap();
        assert_eq!(data, b"qk-bytes");
    }

    #[tokio::test]
    async fn test_fetch_missing_tile_is_not_found() {
        let dir = unique_temp_dir("fetch-missing");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        let err = fetcher.fetch("9/9/9", 1.0).await.unwrap_err();
        assert!(matches!(err, PortError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_fetch_cancel_is_noop() {
        let dir = unique_temp_dir("fetch-cancel");
        std::fs::create_dir_all(dir.join("0/0")).unwrap();
        std::fs::write(dir.join("0/0/0.png"), b"x").unwrap();

        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Xyz);
        fetcher.cancel("0/0/0");
        // cancel 是一个空操作；读取仍会成功。
        let data = fetcher.fetch("0/0/0", 1.0).await.unwrap();
        assert_eq!(data, b"x");
    }

    #[test]
    fn test_accessors() {
        let dir = unique_temp_dir("accessors");
        let fetcher = FileTileFetcher::new(&dir, FileTileScheme::Quadkey)
            .with_strict_offline(true)
            .with_extension("jpg");
        assert_eq!(fetcher.root(), dir.as_path());
        assert_eq!(fetcher.scheme(), FileTileScheme::Quadkey);
        assert!(fetcher.strict_offline());
    }
}
