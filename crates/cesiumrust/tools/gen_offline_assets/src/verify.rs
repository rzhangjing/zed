//! 集成式读回校验。
//!
//! 生成之后，我们构造*真实的* M3.1 离线 fetcher
//! （`FileTileFetcher` / `FileTerrainFetcher`），通过它们的端口方法
//! 拉回少量瓦片，断言其解码成功。这证明了生成的布局、字节结构和
//! y 序约定与 fetcher 的解码逻辑完全对齐 —— 这正是 fixture 的意义。
//!
//! 端口方法返回装箱的 future，但离线 fetcher 会在首次 poll 时同步
//! resolve它们（普通 `std::fs::read`）。我们用一个基于 [`Waker::noop`]
//! 的最小化 [`block_on`] 来驱动它们，而非引入 tokio，因此
//! 本工具从不启动异步运行时。

use cesium_network::{FileTerrainFetcher, FileTileFetcher, FileTileScheme, TerrainScheme};
use cesium_ports_driven::{TerrainProvider, TileFetcher};
use std::future::Future;
use std::path::Path;
use std::task::{Context, Poll, Waker};

/// 无需运行时即可将一个立即就绪的 future 驱动至完成。
///
/// 离线 fetcher 构建的 future 在首次 poll 时就 `Ready`（它们包裹
/// 一个同步的 `std::fs::read`）。`Pending` 结果意味着 fetcher 契约发生了
/// 变化，因此我们响亮地失败而非空转。
fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = Box::pin(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!(
            "gen_offline_assets: offline future returned Pending \
             (expected immediate Ready — the fetcher must not block)"
        ),
    }
}

/// 通过 M3.1 fetcher 读回一批具代表性的影像与地形瓦片。
/// 返回通过的断言数。
///
/// # Errors
///
/// 在第块块读取或解码失败的瓦片处返回一个人类可读的错误字符串，
/// 使布局错位的 fixture 立即暴露。
pub fn verify(
    imagery_root: &Path,
    terrain_root: &Path,
    terrain_max_level: u32,
) -> Result<usize, String> {
    let mut checks = 0usize;

    // --- 影像：FileTileFetcher (Xyz) -------------------------------------
    let imagery = FileTileFetcher::new(imagery_root, FileTileScheme::Xyz);
    for url in ["0/0/0", "0/1/0", "1/0/0", "1/2/1", "3/5/2", "3/15/7"] {
        let data = block_on(imagery.fetch(url, 1.0))
            .map_err(|e| format!("imagery fetch '{url}' failed: {e}"))?;
        if data.is_empty() {
            return Err(format!("imagery fetch '{url}' returned empty bytes"));
        }
        // PNG 魔数 —— 证明文件是真实编码的 PNG，而非乱码。
        if data.len() < 8 || data[..8] != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
            return Err(format!("imagery fetch '{url}' is not a PNG (bad magic)"));
        }
        checks += 1;
    }

    // --- 地形：FileTerrainFetcher (Tms)，通过显式 root -----------------
    let terrain = FileTerrainFetcher::new(terrain_root, TerrainScheme::Tms, terrain_max_level);
    for (x, y, level) in [(0u32, 0u32, 0u32), (1, 0, 1), (2, 1, 2), (10, 3, 4)] {
        if !terrain.get_availability(x, y, level) {
            return Err(format!(
                "terrain get_availability({x},{y},{level}) == false (missing on disk)"
            ));
        }
        let geom = block_on(terrain.request_tile_geometry(x, y, level))
            .map_err(|e| format!("terrain request_tile_geometry({x},{y},{level}) failed: {e}"))?;
        if geom.positions.len() != crate::generate::TERRAIN_GRID_SIZE.pow(2) {
            return Err(format!(
                "terrain ({x},{y},{level}) decoded {} positions (expected {})",
                geom.positions.len(),
                crate::generate::TERRAIN_GRID_SIZE.pow(2)
            ));
        }
        checks += 1;
    }

    // --- 地形：FileTerrainFetcher::from_layer_url（maxzoom 接线） --------
    let layer_url = format!(
        "file:///{}",
        terrain_root
            .join("layer.json")
            .display()
            .to_string()
            .replace('\\', "/")
    );
    let from_layer = FileTerrainFetcher::from_layer_url(&layer_url, TerrainScheme::Tms, true)
        .map_err(|e| format!("from_layer_url('{layer_url}') failed: {e}"))?;
    if from_layer.maximum_level() != terrain_max_level {
        return Err(format!(
            "layer.json maxzoom read as {} (expected {terrain_max_level})",
            from_layer.maximum_level()
        ));
    }
    checks += 1;

    Ok(checks)
}
