//! 无窗口离线自检（`CESIUM_OFFLINE_SELFCHECK`）。
//!
//! 在**无窗口、无 GPU** 的情况下证明 —— 离线确定性接线
//! 是真实的：
//!
//! 1. `OFFLINE_IMAGERY_ROOT` / `OFFLINE_TERRAIN_ROOT` fixture（由
//!    `tools/gen_offline_assets` 生成，M3.2）能通过*实际的* M3.1 fetcher
//!    （`FileTileFetcher` / `FileTerrainFetcher`）成功读回，且
//! 2. `STRICT_OFFLINE=1` 使任何 `http(s)` 请求同步 panic（无
//!    网络回退），这是离线确定性的核心保证。
//!
//! 当设置了自检 flag 时，`main()` 会调用 [`run`] 并在构建 Bevy 应用*之前*
//! 退出，因此该路径对 CI/无头环境友好。
//!
//! 端口方法返回装箱的 future，离线 fetcher 会在首次 poll 时 resolve
//! 它们（同步的 `std::fs::read`）；我们用一个基于 [`Waker::noop`] 的
//! [`block_on`] 驱动它们，而非启动 tokio 运行时。

use cesium_network::{FileTerrainFetcher, FileTileFetcher, FileTileScheme, TerrainScheme};
use cesium_ports_driven::{TerrainProvider, TileFetcher};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::task::{Context, Poll, Waker};

use crate::feature_flags;

/// Heightmap-1.0 网格单元数（`65×65`）；一块解码后的地形瓦片必须
/// 恰好有这么多个 position。
const TERRAIN_POSITIONS: usize = 65 * 65;
/// PNG 魔数前缀（`\x89PNG`）；证明一张影像瓦片是真实编码的 PNG。
const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];

/// 无需运行时即可将一个立即就绪的 future 驱动至完成。
fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = Box::pin(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!(
            "offline self-check: fetcher future returned Pending \
             (expected immediate Ready — offline IO must not block)"
        ),
    }
}

/// 规范化 `p`，在其尚不存在时回退到原路径（从而后续的读取会
/// 报出干净的 NotFound 而非 IO 错误）。
fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// 运行离线自检，打印一份报告。返回进程退出码：每项检查均绿时为
/// `0`，任一失败时为 `2`。
pub fn run() -> i32 {
    println!("[selfcheck] offline determinism self-check (headless, no GPU)");
    let strict = feature_flags::strict_offline();
    let imagery = feature_flags::offline_imagery_root();
    let terrain = feature_flags::offline_terrain_root();
    println!(
        "[selfcheck] STRICT_OFFLINE={} imagery_root={:?} terrain_root={:?}",
        strict, imagery, terrain
    );

    // 累计通过项与失败明细，供末尾报告与退出码使用。
    let mut checks = 0usize;
    let mut failures: Vec<String> = Vec::new();

    // ── 影像通过 FileTileFetcher (Xyz) 读回 ────────────────────
    // 逐个拉取少量固定坐标，验证读回字节流具有真正的 PNG 魔数。
    match &imagery {
        Some(raw) => {
            let root = canon(raw);
            let fetcher = FileTileFetcher::new(&root, FileTileScheme::Xyz).with_strict_offline(strict);
            for url in ["0/0/0", "1/0/0", "3/5/2"] {
                match block_on(fetcher.fetch(url, 1.0)) {
                    Ok(bytes) if bytes.len() >= 4 && bytes[..4] == PNG_MAGIC => checks += 1,
                    Ok(_) => failures.push(format!("imagery '{url}': not a PNG (bad magic)")),
                    Err(e) => failures.push(format!("imagery '{url}': {e}")),
                }
            }
        }
        None => println!("[selfcheck] OFFLINE_IMAGERY_ROOT unset — imagery read-back skipped"),
    }

    // ── 地形通过 FileTerrainFetcher (Tms，从 layer.json) 读回 ─
    match &terrain {
        Some(raw) => {
            let root = canon(raw);
            let layer = root.join("layer.json");
            // from_layer_url 接受一个裸的（已规范化的）绝对路径。
            match FileTerrainFetcher::from_layer_url(
                &layer.display().to_string(),
                TerrainScheme::Tms,
                strict,
            ) {
                Ok(fetcher) => {
                    println!(
                        "[selfcheck] terrain layer.json maxzoom={}",
                        fetcher.maximum_level()
                    );
                    for (x, y, level) in [(0u32, 0u32, 0u32), (1, 0, 1), (2, 1, 2)] {
                        match block_on(fetcher.request_tile_geometry(x, y, level)) {
                            Ok(g) if g.positions.len() == TERRAIN_POSITIONS => checks += 1,
                            Ok(g) => failures.push(format!(
                                "terrain ({x},{y},{level}): {} positions (expected {TERRAIN_POSITIONS})",
                                g.positions.len()
                            )),
                            Err(e) => {
                                failures.push(format!("terrain ({x},{y},{level}): {e}"))
                            }
                        }
                    }
                }
                Err(e) => failures.push(format!("terrain from_layer_url: {e}")),
            }
        }
        None => println!("[selfcheck] OFFLINE_TERRAIN_ROOT unset — terrain read-back skipped"),
    }

    // ── STRICT_OFFLINE：http(s) 必须同步 panic ────────────────────
    if strict {
        if strict_offline_panics_on_http(imagery.as_deref()) {
            println!("[selfcheck] STRICT_OFFLINE: http(s) fetch panicked as required");
            checks += 1;
        } else {
            failures.push("STRICT_OFFLINE: http(s) fetch did NOT panic".to_string());
        }
    } else {
        println!("[selfcheck] STRICT_OFFLINE unset — http-panic assertion skipped");
    }

    if failures.is_empty() {
        println!("[selfcheck] PASS — {checks} checks green");
        0
    } else {
        for f in &failures {
            eprintln!("[selfcheck] FAIL: {f}");
        }
        println!("[selfcheck] FAIL — {} checks green, {} failures", checks, failures.len());
        2
    }
}

/// 当一个 STRICT_OFFLINE 的 `FileTileFetcher` 对 `https` URL panic 时
/// 返回 `true`（所需的无网络回退行为）。panic hook 被静默，
/// 使预期的 panic 不污染报告。
fn strict_offline_panics_on_http(imagery_root: Option<&Path>) -> bool {
    let root = imagery_root.map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let fetcher = FileTileFetcher::new(root, FileTileScheme::Xyz).with_strict_offline(true);
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        // 在 STRICT_OFFLINE 下，一个 http(s) URL 会在 `fetch` *内部*同步
        // panic（在 future 构建 / 任何 IO 发生之前）。`drop` 那个从未运行的
        // future，使 clippy 的 `let_underscore_future` 保持静默。
        drop(fetcher.fetch("https://example.com/0/0/0.png", 1.0));
    }))
    .is_err();
    std::panic::set_hook(prev_hook);
    panicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_drives_immediately_ready_future() {
        assert_eq!(block_on(async { 42u32 }), 42);
    }

    #[test]
    fn strict_offline_fetcher_panics_on_https() {
        // 无需 fixture：panic 在任何磁盘访问之前就在 `fetch` 内部
        // 同步触发，因此任意 root 都可以。
        assert!(strict_offline_panics_on_http(None));
    }

    #[test]
    fn canon_of_missing_path_is_passthrough() {
        let p = Path::new("definitely/does/not/exist");
        assert_eq!(canon(p), p.to_path_buf());
    }
}
