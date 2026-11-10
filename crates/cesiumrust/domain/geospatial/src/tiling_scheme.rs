//! Tiling scheme - 将地球划分成瓦片网格（经纬度与 Web Mercator 两种）。

use crate::cartographic::Cartographic;
use crate::ellipsoid::Ellipsoid;
use crate::projection::{GeographicProjection, MapProjection, WebMercatorProjection};
use crate::rectangle::Rectangle;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// 定义地球如何被细分为瓦片。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilingScheme {
    /// 本瓦片方案所使用的投影。
    projection: TilingProjection,
    /// 瓦片方案覆盖的矩形。
    rectangle: Rectangle,
    /// level 0 时 X 方向的瓦片数。
    root_tiles_x: u32,
    /// level 0 时 Y 方向的瓦片数。
    root_tiles_y: u32,
}

/// 瓦片方案的投影变体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TilingProjection {
    /// 经纬度（地理）投影。
    Geographic(GeographicProjection),
    /// Web Mercator 投影。
    WebMercator(WebMercatorProjection),
}

impl TilingScheme {
    /// 创建地理瓦片方案（level 0 时宽 2 瓦、高 1 瓦）。
    /// 映射到 `GeographicTilingScheme`
    pub fn geographic(ellipsoid: Ellipsoid) -> Self {
        Self {
            projection: TilingProjection::Geographic(GeographicProjection::new(ellipsoid)),
            rectangle: Rectangle::MAX_VALUE,
            root_tiles_x: 2,
            root_tiles_y: 1,
        }
    }

    /// 创建 Web Mercator 瓦片方案（level 0 时宽 1 瓦、高 1 瓦）。
    /// 映射到 `WebMercatorTilingScheme`
    pub fn web_mercator(ellipsoid: Ellipsoid) -> Self {
        let max_lat = WebMercatorProjection::MAXIMUM_LATITUDE;
        Self {
            projection: TilingProjection::WebMercator(WebMercatorProjection::new(ellipsoid)),
            rectangle: Rectangle::new(-PI, -max_lat, PI, max_lat),
            root_tiles_x: 1,
            root_tiles_y: 1,
        }
    }

    /// 创建自定义瓦片方案。
    pub fn new(
        projection: TilingProjection,
        rectangle: Rectangle,
        root_tiles_x: u32,
        root_tiles_y: u32,
    ) -> Self {
        Self {
            projection,
            rectangle,
            root_tiles_x,
            root_tiles_y,
        }
    }

    /// 获取本瓦片方案所使用的投影。
    pub fn projection(&self) -> &TilingProjection {
        &self.projection
    }

    /// 获取本瓦片方案覆盖的矩形。
    pub fn rectangle(&self) -> &Rectangle {
        &self.rectangle
    }

    /// 获取 level 0 时 X 方向的瓦片数。
    pub fn root_tiles_x(&self) -> u32 {
        self.root_tiles_x
    }

    /// 获取 level 0 时 Y 方向的瓦片数。
    pub fn root_tiles_y(&self) -> u32 {
        self.root_tiles_y
    }

    /// 计算给定层级下 X 和 Y 方向的瓦片数。
    /// 映射到 `TilingScheme.getNumberOfXTilesAtLevel` / `getNumberOfYTilesAtLevel`
    pub fn tiles_at_level(&self, level: u32) -> (u32, u32) {
        // 每升一级，两个方向的瓦片数都翻倍（缩放因子 2^level）。
        let scale = 1u32 << level;
        (self.root_tiles_x * scale, self.root_tiles_y * scale)
    }

    /// 计算给定 x、y、level 处瓦片所覆盖的矩形。
    /// 映射到 `TilingScheme.tileXYToRectangle`
    pub fn tile_to_rectangle(&self, x: u32, y: u32, level: u32) -> Rectangle {
        let (tiles_x, tiles_y) = self.tiles_at_level(level);
        let tile_width = self.rectangle.width() / tiles_x as f64;
        let tile_height = self.rectangle.height() / tiles_y as f64;

        let west = self.rectangle.west + x as f64 * tile_width;
        let north = self.rectangle.north - y as f64 * tile_height;

        // Y 自北向南递增，故用 north 减去一个瓦片高得到 south。
        Rectangle::new(west, north - tile_height, west + tile_width, north)
    }

    /// 计算瓦片的原生矩形（以投影坐标表示）。
    pub fn tile_to_native_rectangle(&self, x: u32, y: u32, level: u32) -> Rectangle {
        let geo_rect = self.tile_to_rectangle(x, y, level);
        let sw = self.project(&Cartographic::from_radians(geo_rect.west, geo_rect.south, 0.0));
        let ne = self.project(&Cartographic::from_radians(geo_rect.east, geo_rect.north, 0.0));
        Rectangle::new(sw.x, sw.y, ne.x, ne.y)
    }

    /// 判断在某个层级下哪个瓦片包含给定的测绘坐标位置。
    /// 映射到 `TilingScheme.positionToTileXY`
    pub fn position_to_tile(&self, position: &Cartographic, level: u32) -> Option<(u32, u32)> {
        let (tiles_x, tiles_y) = self.tiles_at_level(level);
        let tile_width = self.rectangle.width() / tiles_x as f64;
        let tile_height = self.rectangle.height() / tiles_y as f64;

        // 由西/北边界起算的偏移除以瓦片尺寸并向下取整，得到所在瓦片坐标。
        let x = ((position.longitude - self.rectangle.west) / tile_width).floor() as i64;
        let y = ((self.rectangle.north - position.latitude) / tile_height).floor() as i64;

        if x < 0 || x >= tiles_x as i64 || y < 0 || y >= tiles_y as i64 {
            return None;
        }

        Some((x as u32, y as u32))
    }

    /// 使用本瓦片方案的投影将一个测绘坐标位置投影。
    pub fn project(&self, cartographic: &Cartographic) -> glam::DVec3 {
        match &self.projection {
            TilingProjection::Geographic(p) => p.project(cartographic),
            TilingProjection::WebMercator(p) => p.project(cartographic),
        }
    }

    /// 使用本瓦片方案的投影反投影坐标。
    pub fn unproject(&self, projected: glam::DVec3) -> Cartographic {
        match &self.projection {
            TilingProjection::Geographic(p) => p.unproject(projected),
            TilingProjection::WebMercator(p) => p.unproject(projected),
        }
    }

    /// 获取本瓦片方案所使用的椭球。
    pub fn ellipsoid(&self) -> &Ellipsoid {
        match &self.projection {
            TilingProjection::Geographic(p) => p.ellipsoid(),
            TilingProjection::WebMercator(p) => p.ellipsoid(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 验证地理瓦片方案 level 0 为 2×1 瓦。
    fn test_geographic_tiling_scheme_level0() {
        let ts = TilingScheme::geographic(Ellipsoid::WGS84);
        let (nx, ny) = ts.tiles_at_level(0);
        assert_eq!(nx, 2);
        assert_eq!(ny, 1);
    }

    #[test]
    /// 验证地理瓦片方案 level 1 为 4×2 瓦（每级翻倍）。
    fn test_geographic_tiling_scheme_level1() {
        let ts = TilingScheme::geographic(Ellipsoid::WGS84);
        let (nx, ny) = ts.tiles_at_level(1);
        assert_eq!(nx, 4);
        assert_eq!(ny, 2);
    }

    #[test]
    /// 验证 Web Mercator 瓦片方案 level 0 为 1×1 瓦。
    fn test_web_mercator_tiling_scheme_level0() {
        let ts = TilingScheme::web_mercator(Ellipsoid::WGS84);
        let (nx, ny) = ts.tiles_at_level(0);
        assert_eq!(nx, 1);
        assert_eq!(ny, 1);
    }

    #[test]
    /// 验证 level 0 瓦片 (0,0) 覆盖西半球矩形（经度 -π~0）。
    fn test_tile_to_rectangle() {
        let ts = TilingScheme::geographic(Ellipsoid::WGS84);
        // level 0，瓦片 (0,0) 应为西半球
        let rect = ts.tile_to_rectangle(0, 0, 0);
        assert!((rect.west - (-PI)).abs() < 1e-10);
        assert!((rect.east - 0.0).abs() < 1e-10);
        assert!((rect.south - (-PI / 2.0)).abs() < 1e-10);
        assert!((rect.north - (PI / 2.0)).abs() < 1e-10);
    }

    #[test]
    /// 验证靠近本初子午线东侧的点在 level 0 属于瓦片 (1,0)。
    fn test_position_to_tile() {
        let ts = TilingScheme::geographic(Ellipsoid::WGS84);
        // (0, 0) 处在 level 0 应属于瓦片 (1, 0)
        let pos = Cartographic::from_radians(0.001, 0.0, 0.0);
        let (x, y) = ts.position_to_tile(&pos, 0).unwrap();
        assert_eq!(x, 1);
        assert_eq!(y, 0);
    }

    #[test]
    /// 验证经度超出矩形范围的位置返回 None。
    fn test_position_to_tile_out_of_bounds() {
        let ts = TilingScheme::geographic(Ellipsoid::WGS84);
        // 远远超出范围的位置应返回 None
        let pos = Cartographic::from_radians(PI + 1.0, 0.0, 0.0);
        assert!(ts.position_to_tile(&pos, 0).is_none());
    }
}
