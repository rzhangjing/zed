//! 离线磁盘支撑的地形获取器（heightmap-1.0 格式）。
//!
//! 通过从本地目录读取 `.terrain` 瓦片来实现 [`TerrainProvider`] 驱动端口，
//! 目录布局为 `{root}/{level}/{x}/{y}.terrain`，与 viewer-demo 的离线
//! heightmap fixture 相对应（参考实现：
//! `cesium-rs/examples/viewer-demo/src/main.rs` L638-686，`ensure_offline_terrain`）。
//!
//! # 磁盘格式：heightmap-1.0
//!
//! 每个瓦片都是一个 `65×65` 的 `u16`-LE 编码高度网格，后跟一个
//! childTileMask 字节和一个 water-mask 字节（共 `65*65*2 + 2 = 8452`
//! 字节）。度量高度按如下方式还原：
//!
//! ```text
//! height_m = encoded / 5 - 1000
//! ```
//!
//! 这正是参考实现中 `encode_terrain_height`（`encoded = (height_m + 1000) * 5`）
//! 的逆变换。
//!
//! # Y 序约定
//!
//! [`TerrainScheme::Tms`] 将行 `y = 0` 存于**南**极（即 heightmap-1.0 /
//! `layer.json` 的 `"scheme": "tms"` 默认），因此磁盘行为
//! `(1 << level) - 1 - y_geo`。[`TerrainScheme::Geographic`] 将行 0 存于
//! 北极（不做翻转）。viewer-demo fixture 向磁盘写入 TMS。
//!
//! # 几何契约（IO 层，保留 f64）
//!
//! 解码后的 [`GeometryData`] 将顶点置于一个单位瓦片坐标系上：
//! `u ∈ [-1, 1]` 西→东，`v ∈ [-1, 1]` 北→南（行 0 = 北），且
//! `z = height_m`。索引构成一个 `64×64` 的三角形网格。所有值均保持
//! `f64` —— IO 层从不向下转换到 `f32`（那只在 GPU 边界才发生）。
//!
//! # STRICT_OFFLINE 契约
//!
//! 当启用 STRICT_OFFLINE 且 `layer.json` URL 是一个 `http://`/`https://`
//! URL 时，[`FileTerrainFetcher::from_layer_url`] 会 panic —— 离线路径
//! 没有网络回退。

use cesium_geospatial::geometry::PrimitiveType;
use cesium_geospatial::{BoundingSphere, GeometryData, Rectangle};
use cesium_ports_driven::{PortError, PortResult, TerrainProvider};
use glam::DVec3;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// Heightmap 网格宽度（heightmap-1.0 默认；参考实现的 `TERRAIN_GRID_SIZE`）。
const HEIGHTMAP_GRID_SIZE: usize = 65;
/// 单个 heightmap-1.0 瓦片的总字节数（`65*65` u16 + childTileMask + waterMask）。
const HEIGHTMAP_TILE_BYTES: usize = HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE * 2 + 2;
/// heightmap-1.0 解码缩放：`height_m = encoded / 5 - 1000`。
const HEIGHTMAP_SCALE: f64 = 1.0 / 5.0;
/// heightmap-1.0 解码偏移（米）。
const HEIGHTMAP_OFFSET: f64 = -1000.0;

/// 地形瓦片在磁盘上的 y 序约定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainScheme {
    /// TMS：行 `y = 0` 存于**南**极；磁盘行为
    /// `(1 << level) - 1 - y_geo`。这是 viewer-demo fixture 使用的
    /// heightmap-1.0 / `layer.json` `"scheme": "tms"` 默认。
    Tms,
    /// Geographic：行 `y = 0` 存于**北**极（不做翻转）。
    Geographic,
}

/// 从本地目录读取 heightmap-1.0 瓦片的离线地形获取器。
#[derive(Debug, Clone)]
pub struct FileTerrainFetcher {
    /// 磁盘上存放瓦片的根目录。
    root: PathBuf,
    /// 行序约定（TMS 还是 Geographic）。
    scheme: TerrainScheme,
    /// 严格离线：为真时不回退到网络。
    strict_offline: bool,
    /// 所提供服务的最深瓦片层级。
    maximum_level: u32,
    /// 覆盖的地理矩形（缺省为全球）。
    rectangle: Rectangle,
}

impl FileTerrainFetcher {
    /// 创建一个以 `root` 为根的获取器。
    ///
    /// `maximum_level` 是所提供服务的最深层级（viewer-demo fixture
    /// 使用 `4`）。rectangle 默认为整地球
    /// （[`Rectangle::MAX_VALUE`]）。
    pub fn new(root: impl AsRef<Path>, scheme: TerrainScheme, maximum_level: u32) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            scheme,
            strict_offline: false,
            maximum_level,
            rectangle: Rectangle::MAX_VALUE,
        }
    }

    /// 启用 STRICT_OFFLINE 语义：传给 [`Self::from_layer_url`] 的网络
    /// `layer.json` URL 会触发立即 panic。
    pub fn with_strict_offline(mut self, strict: bool) -> Self {
        self.strict_offline = strict;
        self
    }

    /// 覆盖所覆盖的 rectangle（默认：整地球）。
    pub fn with_rectangle(mut self, rectangle: Rectangle) -> Self {
        self.rectangle = rectangle;
        self
    }

    /// 从 `layer.json` URL 加载一个获取器。
    ///
    /// 仅支持 `file://` URL；根目录是 `layer.json` 的父目录。当 JSON 中存在
    /// `maxzoom` 字段时，`maximum_level` 从其读取，默认为 `0`。
    ///
    /// # Panic
    ///
    /// 当 `strict_offline` 为 `true` 且 `layer_json_url` 是一个
    /// `http://`/`https://` URL 时 panic —— STRICT_OFFLINE 契约禁止网络
    /// 回退。
    pub fn from_layer_url(
        layer_json_url: &str,
        scheme: TerrainScheme,
        strict_offline: bool,
    ) -> PortResult<Self> {
        if strict_offline && is_http_url(layer_json_url) {
            panic!(
                "STRICT_OFFLINE violation: FileTerrainFetcher received network layer URL '{}' \
                 (offline mode forbids HTTP fallback; wire the offline root instead)",
                layer_json_url
            );
        }
        let path = layer_url_to_path(layer_json_url)?;
        let root = path
            .parent()
            .ok_or_else(|| {
                PortError::NotFound(format!(
                    "FileTerrainFetcher: layer.json '{}' has no parent directory",
                    path.display()
                ))
            })?
            .to_path_buf();
        let maximum_level = read_maxzoom(&path).unwrap_or(0);
        Ok(Self {
            root,
            scheme,
            strict_offline,
            maximum_level,
            rectangle: Rectangle::MAX_VALUE,
        })
    }

    /// 地形瓦片集的根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 当前启用的 y 序方案。
    pub fn scheme(&self) -> TerrainScheme {
        self.scheme
    }

    /// STRICT_OFFLINE 语义是否处于激活状态。
    pub fn strict_offline(&self) -> bool {
        self.strict_offline
    }

    /// 解析瓦片 `(x, y, level)` 的磁盘路径，当 [`TerrainScheme::Tms`]
    /// 处于激活时应用 TMS 的 y 翻转。
    fn tile_path(&self, x: u32, y: u32, level: u32) -> PathBuf {
        // TMS 将地理 y 翻转为磁盘 y（(1<<level)-1-y）；Geographic 那么直接用。
        let disk_y = match self.scheme {
            TerrainScheme::Tms => (1u32 << level).saturating_sub(1).saturating_sub(y),
            TerrainScheme::Geographic => y,
        };
        self.root
            .join(level.to_string())
            .join(x.to_string())
            .join(format!("{}.terrain", disk_y))
    }

    /// 读取并解码一个 heightmap-1.0 瓦片为 [`GeometryData`]。
    fn read_tile(&self, x: u32, y: u32, level: u32) -> PortResult<GeometryData> {
        let path = self.tile_path(x, y, level);
        // 区分“文件不存在”与真正的 IO 错误，分别映射为
        // NotFound 与 Network 错误，以便上层区别对待。
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PortError::NotFound(format!(
                    "FileTerrainFetcher: terrain tile not found at '{}'",
                    path.display()
                ))
            } else {
                PortError::Network(format!(
                    "FileTerrainFetcher: IO error reading '{}': {}",
                    path.display(),
                    e
                ))
            }
        })?;
        decode_heightmap(&bytes)
    }
}

impl TerrainProvider for FileTerrainFetcher {
    /// 本提供器覆盖的地理矩形边界。
    fn rectangle(&self) -> Rectangle {
        self.rectangle
    }

    /// 可请求的最深瓦片层级；超过此层级的请求会被拒绝。
    fn maximum_level(&self) -> u32 {
        self.maximum_level
    }

    /// 异步请求一个地形瓦片的几何。
    ///
    /// - `x`/`y`/`level`：瓦片的列、行（地理空标）与层级。
    ///   返回一个解析为 [`GeometryData`] 的 future；若 `level` 超过
    ///   [`maximum_level`](Self::maximum_level) 则返回 [`PortError::NotFound`]。
    ///   由于是本地磁盘读取，future 在首次 poll 时就已完成。
    fn request_tile_geometry<'a>(
        &'a self,
        x: u32,
        y: u32,
        level: u32,
    ) -> Pin<Box<dyn Future<Output = PortResult<GeometryData>> + Send + 'a>> {
        if level > self.maximum_level {
            return Box::pin(async move {
                Err(PortError::NotFound(format!(
                    "FileTerrainFetcher: level {} exceeds maximum_level {}",
                    level, self.maximum_level
                )))
            });
        }
        // 同步读取；future 在首次 poll 时即完成（离线 IO）。
        let result = self.read_tile(x, y, level);
        Box::pin(async move { result })
    }

    /// 报告一个瓦片是否可用：层级不超限且对应磁盘文件存在。
    fn get_availability(&self, x: u32, y: u32, level: u32) -> bool {
        // 超出最深层级直接判为不可用。
        if level > self.maximum_level {
            return false;
        }
        self.tile_path(x, y, level).is_file()
    }
}

/// 将一个 heightmap-1.0 载荷解码为 [`GeometryData`] 网格。
///
/// 载荷至少要有 [`HEIGHTMAP_TILE_BYTES`] 字节：一个 `65×65` 的 `u16`-LE
/// 编码高度网格，然后一个 childTileMask 字节和一个 water-mask 字节（两个
/// mask 字节对几何而言被忽略）。
fn decode_heightmap(bytes: &[u8]) -> PortResult<GeometryData> {
    if bytes.len() < HEIGHTMAP_TILE_BYTES {
        return Err(PortError::Decode(format!(
            "heightmap-1.0 tile too small: {} bytes (expected >= {})",
            bytes.len(),
            HEIGHTMAP_TILE_BYTES
        )));
    }
    let grid = HEIGHTMAP_GRID_SIZE;
    let last = (grid - 1) as f64;
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(grid * grid);
    // 行优先遍历 65×65 网格，逐单元读一个 u16-LE 高度并反量化到米。
    for row in 0..grid {
        for col in 0..grid {
            let i = (row * grid + col) * 2;
            let encoded = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
            let height = encoded as f64 * HEIGHTMAP_SCALE + HEIGHTMAP_OFFSET;
            // 单位瓦片坐标系：u ∈ [-1, 1] 西→东，v ∈ [-1, 1] 北→南
            // （行 0 = 北），z = 高度（米）。全程 f64。
            let u = (col as f64 / last) * 2.0 - 1.0;
            let v = 1.0 - (row as f64 / last) * 2.0;
            positions.push([u, v, height]);
        }
    }
    // 64×64 三角形网格（每单元格两个三角形）。
    let mut indices: Vec<u32> = Vec::with_capacity((grid - 1) * (grid - 1) * 6);
    for row in 0..(grid - 1) {
        for col in 0..(grid - 1) {
            // 一个单元的四角 a(左上)/b(右上)/c(左下)/d(右下)，
            // 沿对角线 a-d…拆为两个三角形 (a,c,b) 与 (b,c,d)。
            let a = (row * grid + col) as u32;
            let b = a + 1;
            let c = a + grid as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let bounding_sphere = sphere_from_positions(&positions);
    Ok(GeometryData {
        positions,
        normals: None,
        tex_coords: None,
        tangents: None,
        bitangents: None,
        indices,
        bounding_sphere,
        primitive_type: PrimitiveType::Triangles,
    })
}

/// 计算一个包裹所有位置的包围球（AABB 中心 + 最大
/// 距离）。通过直接在 `[f64; 3]` 上运算来避免热路径上的
/// glam 依赖。
fn sphere_from_positions(positions: &[[f64; 3]]) -> BoundingSphere {
    if positions.is_empty() {
        return BoundingSphere {
            center: DVec3::ZERO,
            radius: 0.0,
        };
    }
    // 累加逐轴 AABB 的 min/max；包围球中心取对角中点。
    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    for p in positions {
        for axis in 0..3 {
            if p[axis] < min[axis] {
                min[axis] = p[axis];
            }
            if p[axis] > max[axis] {
                max[axis] = p[axis];
            }
        }
    }
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let mut radius_sq = 0.0f64;
    // 取到中心距离最大的点：半径平方取各顶点距离平方的最大值。
    for p in positions {
        let dx = p[0] - center[0];
        let dy = p[1] - center[1];
        let dz = p[2] - center[2];
        let d = dx * dx + dy * dy + dz * dz;
        if d > radius_sq {
            radius_sq = d;
        }
    }
    BoundingSphere {
        center: DVec3::from_array(center),
        radius: radius_sq.sqrt(),
    }
}

/// 当 `url` 以 `http://` 或 `https://` 开头时返回 `true`
/// （大小写不敏感）。
fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// 将一个 `layer.json` URL 转换为文件系统路径。
///
/// 接受 `file:///abs/path/layer.json`、裸的绝对路径以及相对路径。网络
/// URL 会被拒绝（离线获取器没有 HTTP 后端）。
fn layer_url_to_path(url: &str) -> PortResult<PathBuf> {
    let tail = if let Some(idx) = url.find("://") {
        let scheme = &url[..idx];
        if !scheme.eq_ignore_ascii_case("file") {
            return Err(PortError::NotFound(format!(
                "FileTerrainFetcher: unsupported layer URL scheme '{}' (only file:// is allowed offline)",
                scheme
            )));
        }
        let after = &url[idx + 3..];
        // file:///abs/path → /abs/path（保留前导斜杠）；在 Windows 上
        // file:///C:/path → C:/path（去掉前导斜杠，好让盘符
        // 成为路径前缀）。
        match after.find('/') {
            Some(slash) => strip_windows_drive_slash(&after[slash..]),
            None => after,
        }
    } else {
        url
    };
    if tail.is_empty() {
        return Err(PortError::NotFound(
            "FileTerrainFetcher: empty layer.json path".to_string(),
        ));
    }
    Ok(PathBuf::from(tail))
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

/// 从 `layer.json` 文件读取 `maxzoom` 字段。当文件缺失或该字段
/// 不存在/无法解析时返回 `None`。
///
/// 一次极简扫描（无 JSON 依赖）：找到 `"maxzoom"` 并解析其后的
/// 整数。对于确定性的离线 fixture 已足够。
fn read_maxzoom(layer_json: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(layer_json).ok()?;
    let key = "\"maxzoom\"";
    let start = text.find(key)? + key.len();
    let rest = &text[start..];
    // 跳过空白和 ':' 分隔符，然后收集数字。
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u32>().ok()
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
            "cesium-file-terrain-fetcher-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// 将一个度量高度编码到 heightmap-1.0 的 u16 域
    /// （`decode_heightmap` 的逆变换：`encoded = (height_m + 1000) * 5`）。
    fn encode_height(height_m: f64) -> u16 {
        ((height_m + 1000.0) * 5.0).round() as u16
    }

    /// 从一个 `65×65` 的度量高度网格（行优先，行 0 = 北）加两个尾部的
    /// mask 字节，构造一个 heightmap-1.0 瓦片载荷。
    fn build_heightmap_payload(heights: &[f64; HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE]) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::with_capacity(HEIGHTMAP_TILE_BYTES);
        for h in heights {
            buffer.extend_from_slice(&encode_height(*h).to_le_bytes());
        }
        buffer.push(0x0F); // childTileMask：四个子瓦片均存在
        buffer.push(0); // water mask：全为陆地
        buffer
    }

    /// 处处都为 `height_m` 的平坦瓦片。
    fn flat_heights(height_m: f64) -> [f64; HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE] {
        [height_m; HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE]
    }

    // --- 辅助函数 -------------------------------------------------------------

    #[test]
    fn test_is_http_url() {
        assert!(is_http_url("http://example.com/layer.json"));
        assert!(is_http_url("HTTPS://example.com/layer.json"));
        assert!(!is_http_url("file:///abs/layer.json"));
        assert!(!is_http_url("/abs/layer.json"));
    }

    #[test]
    fn test_layer_url_to_path_file() {
        let p = layer_url_to_path("file:///tiles/layer.json").unwrap();
        assert_eq!(p, PathBuf::from("/tiles/layer.json"));
    }

    #[test]
    fn test_layer_url_to_path_bare() {
        let p = layer_url_to_path("/tiles/layer.json").unwrap();
        assert_eq!(p, PathBuf::from("/tiles/layer.json"));
    }

    #[test]
    fn test_layer_url_to_path_rejects_https() {
        let err = layer_url_to_path("https://example.com/layer.json").unwrap_err();
        assert!(matches!(err, PortError::NotFound(_)));
    }

    #[test]
    fn test_read_maxzoom() {
        let dir = unique_temp_dir("maxzoom");
        let path = dir.join("layer.json");
        std::fs::write(
            &path,
            "{\n  \"format\": \"heightmap-1.0\",\n  \"maxzoom\": 4,\n  \"scheme\": \"tms\"\n}\n",
        )
        .unwrap();
        assert_eq!(read_maxzoom(&path), Some(4));
    }

    #[test]
    fn test_read_maxzoom_missing_field() {
        let dir = unique_temp_dir("maxzoom-none");
        let path = dir.join("layer.json");
        std::fs::write(&path, "{\"format\": \"heightmap-1.0\"}").unwrap();
        assert_eq!(read_maxzoom(&path), None);
    }

    // --- heightmap 解码 ----------------------------------------------------

    #[test]
    fn test_decode_heightmap_flat() {
        let payload = build_heightmap_payload(&flat_heights(100.0));
        let geom = decode_heightmap(&payload).unwrap();
        assert_eq!(geom.positions.len(), HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE);
        assert_eq!(
            geom.indices.len(),
            (HEIGHTMAP_GRID_SIZE - 1) * (HEIGHTMAP_GRID_SIZE - 1) * 6
        );
        assert_eq!(geom.primitive_type, PrimitiveType::Triangles);
        // 每个顶点都位于高度 100 m（± u16 舍入）。
        for p in &geom.positions {
            assert!((p[2] - 100.0).abs() < 0.21, "height {} off", p[2]);
        }
        // 角点的 UV 覆盖整个单位瓦片坐标系。
        let first = geom.positions[0];
        assert!((first[0] - (-1.0)).abs() < 1e-9);
        assert!((first[1] - 1.0).abs() < 1e-9);
        let last = geom.positions[HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE - 1];
        assert!((last[0] - 1.0).abs() < 1e-9);
        assert!((last[1] - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn test_decode_heightmap_ramp() {
        // 与参考实现 fixture 一致的西→东斜坡（height = 300 * u）。
        let mut heights = [0.0f64; HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE];
        for row in 0..HEIGHTMAP_GRID_SIZE {
            for col in 0..HEIGHTMAP_GRID_SIZE {
                let u = col as f64 / (HEIGHTMAP_GRID_SIZE - 1) as f64;
                heights[row * HEIGHTMAP_GRID_SIZE + col] = 300.0 * u;
            }
        }
        let payload = build_heightmap_payload(&heights);
        let geom = decode_heightmap(&payload).unwrap();
        // 西边缘（col 0）≈ 0 m，东边缘（col 64）≈ 300 m。
        let west = geom.positions[0][2];
        let east = geom.positions[HEIGHTMAP_GRID_SIZE - 1][2];
        assert!((west - 0.0).abs() < 0.21, "west {}", west);
        assert!((east - 300.0).abs() < 0.21, "east {}", east);
    }

    #[test]
    fn test_decode_heightmap_too_small() {
        let err = decode_heightmap(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, PortError::Decode(_)));
    }

    /// 空输入：包围球回退到原点、半径 0。
    #[test]
    fn test_sphere_from_positions_empty() {
        let s = sphere_from_positions(&[]);
        assert_eq!(s.center, DVec3::ZERO);
        assert_eq!(s.radius, 0.0);
    }

    // --- tile_path / TMS 翻转 ------------------------------------------------

    /// TMS 方案下磁盘 y 应等于 (1<<level)-1-y_geo。
    #[test]
    fn test_tile_path_tms_flips_y() {
        let dir = unique_temp_dir("tile-path-tms");
        let fetcher = FileTerrainFetcher::new(&dir, TerrainScheme::Tms, 4);
        // level 2 → 4 行；geographic y=0（北）→ 磁盘 y=3。
        let path = fetcher.tile_path(1, 0, 2);
        assert_eq!(path, dir.join("2").join("1").join("3.terrain"));
        // geographic y=3（南）→ 磁盘 y=0。
        let path = fetcher.tile_path(1, 3, 2);
        assert_eq!(path, dir.join("2").join("1").join("0.terrain"));
    }

    /// Geographic 方案不做 y 翻转：磁盘 y == 地理 y。
    #[test]
    fn test_tile_path_geographic_no_flip() {
        let dir = unique_temp_dir("tile-path-geo");
        let fetcher = FileTerrainFetcher::new(&dir, TerrainScheme::Geographic, 4);
        let path = fetcher.tile_path(1, 0, 2);
        assert_eq!(path, dir.join("2").join("1").join("0.terrain"));
        let path = fetcher.tile_path(1, 3, 2);
        assert_eq!(path, dir.join("2").join("1").join("3.terrain"));
    }

    // --- STRICT_OFFLINE panic ------------------------------------------------

    /// 严格离线 + https URL 应 panic（契约禁网络回退）。
    #[test]
    #[should_panic(expected = "STRICT_OFFLINE violation")]
    fn test_from_layer_url_strict_panics_on_https() {
        let _ = FileTerrainFetcher::from_layer_url(
            "https://example.com/terrain/layer.json",
            TerrainScheme::Tms,
            true,
        );
    }

    /// 严格离线 + http URL 也应 panic。
    #[test]
    #[should_panic(expected = "STRICT_OFFLINE violation")]
    fn test_from_layer_url_strict_panics_on_http() {
        let _ = FileTerrainFetcher::from_layer_url(
            "http://example.com/terrain/layer.json",
            TerrainScheme::Tms,
            true,
        );
    }

    /// 严格离线下 file:// URL 应正常加载。
    #[test]
    fn test_from_layer_url_strict_allows_file() {
        let dir = unique_temp_dir("from-url-ok");
        std::fs::write(
            dir.join("layer.json"),
            "{\"format\":\"heightmap-1.0\",\"maxzoom\":3,\"scheme\":\"tms\"}",
        )
        .unwrap();
        let url = format!(
            "file:///{}",
            dir.join("layer.json").display().to_string().replace('\\', "/")
        );
        let fetcher =
            FileTerrainFetcher::from_layer_url(&url, TerrainScheme::Tms, true).unwrap();
        assert_eq!(fetcher.maximum_level(), 3);
        assert!(fetcher.strict_offline());
        // 解析出的 root 必须回指磁盘上的同一个目录
        // （与分隔符无关：在 Windows 和 POSIX 上都能工作）。
        assert!(fetcher.root().join("layer.json").is_file());
    }

    /// 非严格模式下，https layer URL 应被拒绝为一个错误（而非 panic）。
    #[test]
    fn test_from_layer_url_not_strict_rejects_https_with_error() {
        // 不开 STRICT_OFFLINE 时，https URL 仍会被拒绝（无 HTTP
        // 后端），但以 PortError 而非 panic 的形式。
        let err = FileTerrainFetcher::from_layer_url(
            "https://example.com/terrain/layer.json",
            TerrainScheme::Tms,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, PortError::NotFound(_)));
    }

    // --- 端到端 request_tile_geometry ------------------------------------

    #[tokio::test]
    async fn test_request_tile_geometry_returns_grid() {
        let dir = unique_temp_dir("request-geom");
        // 写入一个 level-0 的 TMS 瓦片：geographic (0,0) → 磁盘 y = 0。
        std::fs::create_dir_all(dir.join("0").join("0")).unwrap();
        let payload = build_heightmap_payload(&flat_heights(50.0));
        std::fs::write(dir.join("0").join("0").join("0.terrain"), &payload).unwrap();

        let fetcher = FileTerrainFetcher::new(&dir, TerrainScheme::Tms, 4);
        let geom = fetcher.request_tile_geometry(0, 0, 0).await.unwrap();
        assert_eq!(geom.positions.len(), HEIGHTMAP_GRID_SIZE * HEIGHTMAP_GRID_SIZE);
        assert!((geom.positions[0][2] - 50.0).abs() < 0.21);
    }

    #[tokio::test]
    async fn test_request_tile_geometry_missing_is_not_found() {
        let dir = unique_temp_dir("request-missing");
        let fetcher = FileTerrainFetcher::new(&dir, TerrainScheme::Tms, 4);
        let err = fetcher.request_tile_geometry(0, 0, 0).await.unwrap_err();
        assert!(matches!(err, PortError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_request_tile_geometry_level_exceeds_max() {
        let dir = unique_temp_dir("request-exceed");
        let fetcher = FileTerrainFetcher::new(&dir, TerrainScheme::Tms, 2);
        let err = fetcher.request_tile_geometry(0, 0, 5).await.unwrap_err();
        assert!(matches!(err, PortError::NotFound(_)));
    }

    /// 本地 fixture 中存在的瓦片应报告为可用，不存在的为不可用。
    #[test]
    fn test_get_availability() {
        let dir = unique_temp_dir("availability");
        std::fs::create_dir_all(dir.join("1").join("0")).unwrap();
        let payload = build_heightmap_payload(&flat_heights(0.0));
        // level 1 TMS：geographic (0,0) → 磁盘 y = 1。
        std::fs::write(dir.join("1").join("0").join("1.terrain"), &payload).unwrap();

        let fetcher = FileTerrainFetcher::new(&dir, TerrainScheme::Tms, 4);
        assert!(fetcher.get_availability(0, 0, 1));
        assert!(!fetcher.get_availability(1, 0, 1)); // 磁盘 y=0 不存在
        assert!(!fetcher.get_availability(0, 0, 9)); // level > max
    }

    #[test]
    fn test_rectangle_and_maximum_level() {
        let dir = unique_temp_dir("accessors");
        let fetcher = FileTerrainFetcher::new(&dir, TerrainScheme::Geographic, 7);
        assert_eq!(fetcher.rectangle(), Rectangle::MAX_VALUE);
        assert_eq!(fetcher.maximum_level(), 7);
        assert_eq!(fetcher.scheme(), TerrainScheme::Geographic);
    }

    #[test]
    fn test_with_rectangle_override() {
        let dir = unique_temp_dir("rect-override");
        let rect = Rectangle::from_degrees(-10.0, -20.0, 30.0, 40.0);
        let fetcher =
            FileTerrainFetcher::new(&dir, TerrainScheme::Tms, 4).with_rectangle(rect);
        assert_eq!(fetcher.rectangle(), rect);
    }
}
