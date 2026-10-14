//! 确定性的离线资产生成（影像金字塔 + heightmap-1.0 地形）。
//!
//! 此处的每个例程都是瓦片坐标 `(level, x, y)` 的纯函数 ——
//! **无 RNG、无 wall-clock、无网络**。两次运行产生
//! 逐字节一致的输出，因此生成器天然可复现。
//!
//! 磁盘布局镜像 viewer-demo 蓝图
//! （`cesium-rs/examples/viewer-demo/src/main.rs`），且正是
//! M3.1 离线 fetcher（`FileTileFetcher` / `FileTerrainFetcher`）读回时的输入：
//!
//! * **影像** —— XYZ 金字塔 `{root}/{level}/{x}/{y}.png`，地理 y
//!   序（第 0 行在北极），256×256 RGBA 过程化瓦片。
//! * **地形** —— heightmap-1.0 tileset：一个 `layer.json` 描述符加上
//!   `{root}/{level}/{x}/{disk_y}.terrain` 瓦片，磁盘上使用 **TMS** y 序
//!   （`disk_y = (1 << level) - 1 - y_geo`）。每块瓦片是一个 `65×65` 的
//!   `u16`-LE 编码高度网格 + 1 字节 childTileMask + 1 字节 waterMask
//!   （`65*65*2 + 2 = 8452` 字节）。

use std::f64::consts::{FRAC_PI_2, PI};
use std::fs;
use std::io;
use std::path::Path;

/// 离线影像金字塔的默认最高级别（蓝图
/// `OFFLINE_IMAGERY_MAXIMUM_LEVEL`）。级别 `0..=3` 构成一个地理金字塔，
/// 共 `2 + 8 + 32 + 128 = 170` 块瓦片。
pub const IMAGERY_DEFAULT_MAX_LEVEL: u32 = 3;
/// 离线地形 tileset 的默认最高级别（蓝图
/// `OFFLINE_TERRAIN_MAXIMUM_LEVEL`）。级别 `0..=4` 构成 `682` 块瓦片。
pub const TERRAIN_DEFAULT_MAX_LEVEL: u32 = 4;
/// 高度图网格宽度（heightmap-1.0 默认值；蓝图 `TERRAIN_GRID_SIZE`）。
pub const TERRAIN_GRID_SIZE: usize = 65;
/// 一块 heightmap-1.0 瓦片的字节大小（`65*65` u16 + childTileMask + waterMask）。
pub const HEIGHTMAP_TILE_BYTES: usize = TERRAIN_GRID_SIZE * TERRAIN_GRID_SIZE * 2 + 2;
/// 生成的影像瓦片的边长（像素）。
const IMAGERY_TILE_SIZE: u32 = 256;

/// 一次生成过程的聚合结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenStats {
    /// 写入的瓦片数（或已存在而被跳过时的数量）。
    pub tiles: usize,
    /// 所有瓦片的 payload 总字节数（不含目录开销）。
    pub bytes: u64,
    /// 当因输出已存在而跳过生成时为 `true`。
    pub skipped: bool,
}

/// 返回 `0..=max_level` 范围内地理金字塔的瓦片数，
/// 其中每级有 `(2 << level)` 列和 `(1 << level)` 行。
fn pyramid_tile_count(max_level: u32) -> usize {
    let mut total = 0usize;
    for level in 0..=max_level {
        let columns = 2usize << level;
        let rows = 1usize << level;
        total += columns * rows;
    }
    total
}

/// 当缺失时在 `root` 下生成离线影像 XYZ 金字塔。
///
/// 幂等：当 `root/0` 已存在且未设 `force` 时提前返回（`skipped = true`）。
/// 瓦片是 256×256 RGBA PNG，布局为 `{root}/{level}/{x}/{y}.png`，使用地理
/// y 序 —— 正是 `FileTileFetcher::new(root, FileTileScheme::Xyz)` 解析的路径。
pub fn ensure_imagery(root: &Path, max_level: u32, force: bool) -> io::Result<GenStats> {
    let expected = pyramid_tile_count(max_level);
    if !force && root.join("0").is_dir() {
        return Ok(GenStats {
            tiles: expected,
            bytes: 0,
            skipped: true,
        });
    }

    let mut bytes: u64 = 0;
    for level in 0..=max_level {
        let columns = 2u32 << level;
        let rows = 1u32 << level;
        for x in 0..columns {
            for y in 0..rows {
                let image = generate_tile(x, y, columns, rows);
                let path = root.join(format!("{level}/{x}/{y}.png"));
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                image.save(&path).map_err(|e| {
                    io::Error::other(format!("PNG encode {level}/{x}/{y}: {e}"))
                })?;
                bytes += fs::metadata(&path)?.len();
            }
        }
    }
    Ok(GenStats {
        tiles: expected,
        bytes,
        skipped: false,
    })
}

/// 当缺失时在 `root` 下生成离线 heightmap-1.0 地形 tileset。
///
/// 幂等：当 `root/layer.json` 已存在且未设 `force` 时提前返回
/// （`skipped = true`）。写入一个 `layer.json` 描述符加上
/// `{root}/{level}/{x}/{disk_y}.terrain` 瓦片，磁盘上使用 TMS y 序 ——
/// 正是 `FileTerrainFetcher::new(root, TerrainScheme::Tms, max_level)`
/// （和 `from_layer_url`）解码的布局。
pub fn ensure_terrain(root: &Path, max_level: u32, force: bool) -> io::Result<GenStats> {
    let expected = pyramid_tile_count(max_level);
    if !force && root.join("layer.json").is_file() {
        return Ok(GenStats {
            tiles: expected,
            bytes: 0,
            skipped: true,
        });
    }

    fs::create_dir_all(root)?;
    fs::write(root.join("layer.json"), layer_json(max_level))?;

    let mut bytes: u64 = 0;
    for level in 0..=max_level {
        let columns = 2u32 << level;
        let rows = 1u32 << level;
        for x in 0..columns {
            for y_geo in 0..rows {
                let payload = terrain_tile_payload(level, max_level);
                // 磁盘上的 TMS y 序：地理第 0 行（北）存储在最后。
                let disk_y = rows - y_geo - 1;
                let path = root.join(format!("{level}/{x}/{disk_y}.terrain"));
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, &payload)?;
                bytes += payload.len() as u64;
            }
        }
    }
    Ok(GenStats {
        tiles: expected,
        bytes,
        skipped: false,
    })
}

/// 构造一块 heightmap-1.0 瓦片 payload：`65×65` u16-LE 高度（西→东
/// 渐变坡，使解码出的 mesh 明显非平坦），随后一个 childTileMask 字节和
/// 一个 water-mask 字节。
fn terrain_tile_payload(level: u32, max_level: u32) -> Vec<u8> {
    let mut buffer: Vec<u8> = Vec::with_capacity(HEIGHTMAP_TILE_BYTES);
    for _row in 0..TERRAIN_GRID_SIZE {
        for col in 0..TERRAIN_GRID_SIZE {
            let u = col as f64 / (TERRAIN_GRID_SIZE - 1) as f64;
            let height = 100.0 * f64::from(level) + 300.0 * u;
            buffer.extend_from_slice(&encode_terrain_height(height).to_le_bytes());
        }
    }
    // childTileMask：叶级以下四个子块都存在，叶级处都没有。
    let child_mask: u8 = if level < max_level { 0x0F } else { 0x00 };
    buffer.push(child_mask);
    // 单字节水域 mask（全为陆地）。
    buffer.push(0);
    buffer
}

/// 将米制高度编码进 heightmap-1.0 的 u16 域。是 fetcher 解码的逆运算
/// （`height_m = encoded / 5 - 1000`），与蓝图的
/// `encode_terrain_height` 一致。
fn encode_terrain_height(height_meters: f64) -> u16 {
    ((height_meters + 1000.0) * 5.0).round() as u16
}

/// 渲染 `FileTerrainFetcher` 消费的 `layer.json` 描述符
/// （`read_maxzoom` 扫描 `"maxzoom"` 整数；`"scheme": "tms"` 记录
/// 磁盘上的 y 序）。
fn layer_json(maxzoom: u32) -> String {
    format!(
        "{{\n  \"tilejson\": \"2.1.0\",\n  \"format\": \"heightmap-1.0\",\n  \
         \"version\": \"1.0.0\",\n  \"scheme\": \"tms\",\n  \
         \"projection\": \"EPSG:4326\",\n  \"maxzoom\": {maxzoom},\n  \
         \"tiles\": [\"{{z}}/{{x}}/{{y}}.terrain\"]\n}}\n",
        maxzoom = maxzoom
    )
}

/// 为地理方案渲染一块 256×256 影像瓦片（y = 0 在北极），
/// 是 `(x, y, columns, rows)` 的纯函数。
///
/// 该图案刻意不对称 —— 一个红色北极冠、一个蓝色南极冠，中纬度处
/// 是带经度渐变的绿/白棋盘格 —— 使 UV 翻转、接缝和拉伸在瓦片
/// 被影像管线采样回来时一目了然。
fn generate_tile(x: u32, y: u32, columns: u32, rows: u32) -> image::RgbaImage {
    let size = IMAGERY_TILE_SIZE;
    let west = -PI + f64::from(x) * (2.0 * PI) / f64::from(columns);
    let north = FRAC_PI_2 - f64::from(y) * PI / f64::from(rows);
    let lon_step = (2.0 * PI) / f64::from(columns) / f64::from(size);
    let lat_step = -PI / f64::from(rows) / f64::from(size);

    let mut image = image::RgbaImage::new(size, size);
    for py in 0..size {
        let latitude = north + (f64::from(py) + 0.5) * lat_step;
        for px in 0..size {
            let longitude = west + (f64::from(px) + 0.5) * lon_step;
            let color = if latitude > PI / 4.0 {
                // 北极冠：红色（UV 翻转标记 —— 必须出现在顶部）。
                [255, 24, 24, 255]
            } else if latitude < -PI / 4.0 {
                // 南极冠：蓝色（必须出现在底部）。
                [24, 64, 255, 255]
            } else {
                // 中纬度：绿/白棋盘格 + 经度渐变。
                let checker = ((px / 32) + (py / 32)) % 2 == 0;
                let gradient = ((longitude + PI) / (2.0 * PI) * 128.0) as u8;
                if checker {
                    [32 + gradient / 4, 200, 64, 255]
                } else {
                    [232, 232, 224, 255]
                }
            };
            image.put_pixel(px, py, image::Rgba(color));
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_terrain_height_matches_fetcher_decode() {
        // 先 encode，再用 fetcher 的逆公式 decode。
        for h in [0.0, 100.0, 300.0, 700.0, -1000.0] {
            let encoded = encode_terrain_height(h);
            let decoded = f64::from(encoded) / 5.0 - 1000.0;
            assert!((decoded - h).abs() < 0.21, "{h} -> {decoded}");
        }
    }

    #[test]
    fn terrain_tile_payload_has_exact_byte_size() {
        let payload = terrain_tile_payload(2, 4);
        assert_eq!(payload.len(), HEIGHTMAP_TILE_BYTES);
        // 叶级以下存在 childTileMask。
        assert_eq!(payload[HEIGHTMAP_TILE_BYTES - 2], 0x0F);
        assert_eq!(payload[HEIGHTMAP_TILE_BYTES - 1], 0);
        let leaf = terrain_tile_payload(4, 4);
        assert_eq!(leaf[HEIGHTMAP_TILE_BYTES - 2], 0x00);
    }

    #[test]
    fn pyramid_tile_count_matches_blueprint() {
        // 影像级别 0..=3 = 2 + 8 + 32 + 128 = 170。
        assert_eq!(pyramid_tile_count(3), 170);
        // 地形级别 0..=4 = 170 + 512 = 682。
        assert_eq!(pyramid_tile_count(4), 682);
    }

    #[test]
    fn layer_json_contains_maxzoom() {
        let json = layer_json(4);
        assert!(json.contains("\"maxzoom\": 4"));
        assert!(json.contains("\"scheme\": \"tms\""));
        assert!(json.contains("\"format\": \"heightmap-1.0\""));
    }

    #[test]
    fn generate_tile_is_deterministic() {
        let a = generate_tile(1, 0, 2, 1);
        let b = generate_tile(1, 0, 2, 1);
        assert_eq!(a.as_raw(), b.as_raw());
    }
}
