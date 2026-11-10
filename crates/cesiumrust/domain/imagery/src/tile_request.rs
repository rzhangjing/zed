//! 影像瓦片请求计算。
//!
//! 基于瓦片分割方案和图层配置，计算给定地形瓦片需要请求哪些影像瓦片。

use cesium_geospatial::cartographic::Cartographic;
use cesium_geospatial::rectangle::Rectangle;
use cesium_geospatial::tiling_scheme::TilingScheme;

use crate::imagery_layer::ImageryLayer;

/// 一个影像瓦片请求。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageryTileRequest {
    /// 影像图层 ID。
    pub layer_id: u64,
    /// 瓦片的 X 坐标。
    pub x: u32,
    /// 瓦片的 Y 坐标。
    pub y: u32,
    /// 瓦片的层级。
    pub level: u32,
}

impl ImageryTileRequest {
    /// 创建一个新的影像瓦片请求。
    pub fn new(layer_id: u64, x: u32, y: u32, level: u32) -> Self {
        Self {
            layer_id,
            x,
            y,
            level,
        }
    }
}

/// 计算覆盖一个地形瓦片矩形所需的影像瓦片请求。
///
/// # 参数
/// * `layer` - 影像图层配置
/// * `terrain_rectangle` - 地形瓦片的矩形区域
/// * `terrain_level` - 地形瓦片的层级
/// * `tiling_scheme` - 影像提供者使用的瓦片分割方案
///
/// # 返回
/// 覆盖该地形瓦片的影像瓦片请求列表
pub fn compute_tile_requests(
    layer: &ImageryLayer,
    terrain_rectangle: &Rectangle,
    terrain_level: u32,
    tiling_scheme: &TilingScheme,
) -> Vec<ImageryTileRequest> {
    let mut requests = Vec::new();

    // 检查图层是否可见且级别有效
    if !layer.show {
        return requests;
    }

    // 计算要使用的影像级别
    // 通常影像级别与地形级别一致，但可被夹取
    let imagery_level = terrain_level.clamp(layer.minimum_level, layer.maximum_level);

    // 检查地形矩形是否与图层矩形相交
    let intersection = match terrain_rectangle.intersection(&layer.rectangle) {
        Some(rect) => rect,
        None => return requests,
    };

    // 获取覆盖相交矩形的瓦片范围
    // 将位置略微向内夹取，以处理精确的边界情况
    let (num_x_tiles, num_y_tiles) = tiling_scheme.tiles_at_level(imagery_level);
    let scheme_rect = tiling_scheme.rectangle();
    let epsilon = 1e-12;

    let nw_lon = intersection.west.max(scheme_rect.west);
    let nw_lat = intersection.north.min(scheme_rect.north);
    let se_lon = intersection.east.min(scheme_rect.east - epsilon);
    let se_lat = intersection.south.max(scheme_rect.south + epsilon);

    let nw_carto = Cartographic::from_radians(nw_lon, nw_lat, 0.0);
    let se_carto = Cartographic::from_radians(se_lon, se_lat, 0.0);

    let (x_min, y_min) = tiling_scheme
        .position_to_tile(&nw_carto, imagery_level)
        .unwrap_or_default();
    let (x_max, y_max) = match tiling_scheme.position_to_tile(&se_carto, imagery_level) {
        Some(tile) => tile,
        None => (num_x_tiles.saturating_sub(1), num_y_tiles.saturating_sub(1)),
    };

    // 处理可能的环绕或无效坐标
    let (x_min, x_max) = if x_min <= x_max {
        (x_min, x_max)
    } else {
        (x_max, x_min)
    };
    let (y_min, y_max) = if y_min <= y_max {
        (y_min, y_max)
    } else {
        (y_max, y_min)
    };

    // 夹取到有效的瓦片范围
    let x_min = x_min.min(num_x_tiles.saturating_sub(1));
    let x_max = x_max.min(num_x_tiles.saturating_sub(1));
    let y_min = y_min.min(num_y_tiles.saturating_sub(1));
    let y_max = y_max.min(num_y_tiles.saturating_sub(1));

    // 为范围内的所有瓦片生成请求
    for y in y_min..=y_max {
        for x in x_min..=x_max {
            requests.push(ImageryTileRequest::new(layer.id, x, y, imagery_level));
        }
    }

    requests
}

/// 计算从地形瓦片到影像瓦片的纹理坐标映射。
///
/// # 参数
/// * `terrain_rectangle` - 地形瓦片的矩形区域
/// * `imagery_rectangle` - 影像瓦片的矩形区域
///
/// # 返回
/// 用于纹理坐标映射的 (平移, 缩放) 元组
pub fn compute_texture_mapping(
    terrain_rectangle: &Rectangle,
    imagery_rectangle: &Rectangle,
) -> ([f64; 2], [f64; 2]) {
    let terrain_width = terrain_rectangle.width();
    let terrain_height = terrain_rectangle.height();
    let imagery_width = imagery_rectangle.width();
    let imagery_height = imagery_rectangle.height();

    // 计算缩放：地形瓦片覆盖影像瓦片的比例
    let scale_x = terrain_width / imagery_width;
    let scale_y = terrain_height / imagery_height;

    // 计算平移：地形瓦片在影像瓦片内的偏移
    let translation_x = (terrain_rectangle.west - imagery_rectangle.west) / imagery_width;
    let translation_y = (terrain_rectangle.south - imagery_rectangle.south) / imagery_height;

    ([translation_x, translation_y], [scale_x, scale_y])
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_geospatial::ellipsoid::Ellipsoid;
    use cesium_geospatial::tiling_scheme::TilingScheme;

    fn create_geographic_tiling_scheme() -> TilingScheme {
        TilingScheme::geographic(Ellipsoid::WGS84)
    }

    #[test]
    fn test_compute_tile_requests() {
        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE);
        let terrain_rect = Rectangle::from_degrees(-180.0, -90.0, 0.0, 90.0);
        let tiling_scheme = create_geographic_tiling_scheme();

        let requests = compute_tile_requests(&layer, &terrain_rect, 0, &tiling_scheme);

        // 在 0 级，地理瓦片分割方案有 2x1 个瓦片
        // 地形矩形覆盖西半球，因此应请求瓦片 (0, 0)
        assert!(!requests.is_empty());
        assert!(requests.iter().all(|r| r.layer_id == 1));
    }

    #[test]
    fn test_compute_tile_requests_invisible_layer() {
        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE).with_show(false);
        let terrain_rect = Rectangle::from_degrees(-180.0, -90.0, 0.0, 90.0);
        let tiling_scheme = create_geographic_tiling_scheme();

        let requests = compute_tile_requests(&layer, &terrain_rect, 0, &tiling_scheme);

        assert!(requests.is_empty());
    }

    #[test]
    fn test_compute_tile_requests_level_clamping() {
        let layer = ImageryLayer::new(1, Rectangle::MAX_VALUE)
            .with_level_range(2, 5);
        let terrain_rect = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
        let tiling_scheme = create_geographic_tiling_scheme();

        // 在 0 级发起的请求应被夹取到 2 级
        let requests = compute_tile_requests(&layer, &terrain_rect, 0, &tiling_scheme);
        assert!(requests.iter().all(|r| r.level == 2));

        // 在 10 级发起的请求应被夹取到 5 级
        let requests = compute_tile_requests(&layer, &terrain_rect, 10, &tiling_scheme);
        assert!(requests.iter().all(|r| r.level == 5));
    }

    #[test]
    fn test_compute_texture_mapping() {
        let terrain_rect = Rectangle::from_degrees(-90.0, -45.0, 0.0, 45.0);
        let imagery_rect = Rectangle::from_degrees(-180.0, -90.0, 0.0, 90.0);

        let (translation, scale) = compute_texture_mapping(&terrain_rect, &imagery_rect);

        // 在 X 方向上，地形覆盖影像的东半球
        assert!((translation[0] - 0.5).abs() < 1e-10);
        // 在 Y 方向上，地形覆盖影像的中间一半
        assert!((translation[1] - 0.25).abs() < 1e-10);
        // 缩放应在两个维度上均为 0.5
        assert!((scale[0] - 0.5).abs() < 1e-10);
        assert!((scale[1] - 0.5).abs() < 1e-10);
    }
}
