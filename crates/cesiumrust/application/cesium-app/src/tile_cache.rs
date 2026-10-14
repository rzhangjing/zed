//! 瓦片加载的效率与可靠性核心 —— 供 2D（`map2d`）与 3D（`globe_pipeline`）
//! 两条影像链路**共享**：
//!
//! 1. **进程级共享连接池**：一个常驻的 [`ureq::Agent`]（内部自带按主机的
//!    keep-alive 连接池，且 `Send + Sync`）被所有下载 worker 线程复用。
//!    这消除了旧 3D 路径最致命的开销 —— 每个瓦片都 `UreqBackend::new()` 新建
//!    一条连接（DNS + TCP + TLS 握手，单片可达 100–300ms）。连接温复用后，
//!    同一瓦片服务器（`ecn.t{0..7}.tiles.virtualearth.net`）的后续请求直接
//!    走已建立的连接。
//!
//! 2. **本地磁盘缓存**：成功下载（HTTP 200）的瓦片按 quadkey 键落盘到
//!    `{cache_root}/bing/{z}/{x}/{y}.jpg`。此后任何一次请求（无论来自 2D 还是
//!    3D、本次还是下次启动）都**先查磁盘、命中即返回，零网络**。因为两条链路
//!    取的是同一套 Bing Aerial 瓦片，缓存天然共享：2D 下载过的区域，切到 3D
//!    立即可用。
//!
//! ## 契约与不变量
//! - 只在**在线窗口分支**生效：`OFFLINE_IMAGERY_ROOT` 设定后由调用方自行从磁盘
//!   读取，本模块不参与，因此 headless / golden 离屏基线逐字节不受影响。
//! - **原子写**：先写同目录临时文件再 `rename`，避免并发 worker 读到半截文件；
//!   文件系统只读等写入失败时静默降级为“仅返回字节、不落盘”，绝不影响渲染。
//! - 只有 HTTP 200 才写盘；4xx/5xx/传输错误返回 `None`，交由调用方退避重试，
//!   绝不缓存失败响应。

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

/// 进程级共享 `ureq::Agent`。ureq 的 `Agent` 内部按主机维护 keep-alive
/// 连接池，`Agent: Send + Sync`，可被多个 worker 线程并发共享 —— 这正是
/// 相对“每瓦片新建 agent”的关键改进：连接在多次获取之间保持温热。
pub fn shared_agent() -> Arc<ureq::Agent> {
    static AGENT: OnceLock<Arc<ureq::Agent>> = OnceLock::new();
    Arc::clone(AGENT.get_or_init(|| {
        Arc::new(
            ureq::AgentBuilder::new()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) CesiumRust/0.1")
                .timeout(std::time::Duration::from_secs(15))
                .build(),
        )
    }))
}

/// 磁盘缓存根目录（进程内解析一次并缓存）。取
/// [`crate::feature_flags::tile_cache_dir`]：`CESIUM_TILE_CACHE_DIR` 覆盖，
/// 否则默认工作目录下的 `tile_cache`。
fn cache_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(crate::feature_flags::tile_cache_dir)
}

/// 某个 Bing Aerial 瓦片在磁盘缓存中的规范路径：`{root}/bing/{z}/{x}/{y}.jpg`。
/// 2D 与 3D 共用同一键空间，因此一条链路下载的瓦片会被另一条直接命中。
pub fn tile_path(z: u32, x: u32, y: u32) -> PathBuf {
    cache_root()
        .join("bing")
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.jpg"))
}

/// 由瓦片坐标拼出 Bing Aerial JPEG URL（按 `(x+y)%8` 选子域，与旧两处链路
/// 完全一致，确保缓存键与 URL 一一对应）。
fn bing_url(x: u32, y: u32, quadkey: &str) -> String {
    let sub = (x + y) % 8;
    format!("https://ecn.t{sub}.tiles.virtualearth.net/tiles/a{quadkey}.jpeg?g=14393")
}

/// 临时文件名后缀计数器，保证并发写各用独立 `.tmp`，`rename` 前互不覆盖。
fn next_tmp_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// 获取一个 Bing 瓦片的原始字节：
/// 1. **磁盘缓存命中** → 直接读回（零网络，最快路径）；
/// 2. 未命中 → 经**共享连接池**下载；
/// 3. 下载成功（200 且非空）→ **原子落盘**供后续复用，并返回字节。
///
/// 任何网络/读取失败都返回 `None`（不写盘、不 panic），由调用方决定退避重试。
///
/// # 参数
/// - `x`/`y`/`z`：规范 Web-Mercator 瓦片坐标。
/// - `quadkey`：由 `(x, y, z)` 编码出的 Bing quadkey（调用方已算好）。
pub fn fetch_tile(x: u32, y: u32, z: u32, quadkey: &str) -> Option<Vec<u8>> {
    let path = tile_path(z, x, y);

    // 1) 磁盘命中。
    if let Ok(bytes) = std::fs::read(&path) {
        if !bytes.is_empty() {
            return Some(bytes);
        }
    }

    // 2) 未命中：经共享连接池下载。ureq 的 `.call()` 仅对 2xx 返回 Ok，
    //    403/429（限速）与 404（无此瓦片）都会走 Err 分支 → 不缓存、返回 None。
    let url = bing_url(x, y, quadkey);
    let resp = match shared_agent().get(&url).call() {
        Ok(r) => r,
        Err(_) => return None,
    };
    let mut buf = Vec::new();
    if resp.into_reader().read_to_end(&mut buf).is_err() || buf.is_empty() {
        return None;
    }

    // 3) 原子落盘（临时文件 + rename）。写失败静默降级，不影响本次返回。
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_ok() {
            let tmp = dir.join(format!(".{y}.{}.tmp", next_tmp_seq()));
            if std::fs::write(&tmp, &buf).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            } else {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    Some(buf)
}

#[cfg(test)]
mod tile_cache_tests {
    use super::*;

    #[test]
    fn tile_path_is_namespaced_under_bing() {
        let p = tile_path(3, 5, 2);
        let s = p.to_string_lossy().replace('\\', "/");
        assert!(s.contains("/bing/3/5/2.jpg"), "unexpected path: {s}");
    }

    #[test]
    fn bing_url_cycles_subdomain() {
        // 子域 = (x + y) % 8，与旧 2D/3D 两处链路一致。
        let u0 = bing_url(0, 0, "0");
        let u8 = bing_url(8, 0, "0");
        assert!(u0.contains("ecn.t0."), "got {u0}");
        assert!(u8.contains("ecn.t0."), "got {u8}");
        let u3 = bing_url(1, 2, "3");
        assert!(u3.contains("ecn.t3."), "got {u3}");
        assert!(u3.ends_with("/tiles/a3.jpeg?g=14393"));
    }

    #[test]
    fn shared_agent_is_a_pooled_singleton() {
        // 两次取用必须是同一个 Arc（进程级唯一连接池），这是“连接温复用”
        // 相对“每瓦片新建 agent”的核心不变量。
        let a = shared_agent();
        let b = shared_agent();
        assert!(Arc::ptr_eq(&a, &b), "shared_agent must be a process singleton");
    }
}
