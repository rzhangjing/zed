//! 用于地形瓦片选择的四叉树遍历。
//!
//! 提供四叉树瓦片遍历、基于屏幕空间误差（SSE）的 LOD 选择
//! 与瓦片细化决策。

use cesium_geospatial::bounding::BoundingSphere;
use glam::DVec3;

/// 四叉树中的一个瓦片。
#[derive(Debug, Clone, PartialEq)]
pub struct QuadtreeTile {
    /// 瓦片 X 坐标。
    pub x: u32,
    /// 瓦片 Y 坐标。
    pub y: u32,
    /// 瓦片层级（缩放）。
    pub level: u32,
    /// 瓦片的包围球。
    pub bounding_sphere: BoundingSphere,
    /// 瓦片的几何误差（米）。
    pub geometric_error: f64,
    /// 瓦片是否有可渲染内容。
    pub has_content: bool,
    /// 瓦片是否可细化（是否有子节点）。
    pub refineable: bool,
    /// 瓦片状态。
    pub state: TileState,
}

impl QuadtreeTile {
    /// 创建一个新四叉树瓦片。
    pub fn new(
        x: u32,
        y: u32,
        level: u32,
        bounding_sphere: BoundingSphere,
        geometric_error: f64,
    ) -> Self {
        Self {
            x,
            y,
            level,
            bounding_sphere,
            geometric_error,
            has_content: true,
            refineable: true,
            state: TileState::Unloaded,
        }
    }

    /// 计算该瓦片的屏幕空间误差。
    ///
    /// # 参数
    /// * `camera_position` - 相机的世界空间位置
    /// * `viewport_height` - 视口高度（像素）
    /// * `fov_y` - 垂直视场角（弧度）
    ///
    /// # 返回
    /// 屏幕空间误差（像素）
    pub fn compute_screen_space_error(
        &self,
        camera_position: DVec3,
        viewport_height: f64,
        fov_y: f64,
    ) -> f64 {
        let distance = (camera_position - self.bounding_sphere.center).length()
            - self.bounding_sphere.radius;
        let distance = distance.max(1.0); // 避免除以零

        // SSE = (geometric_error * viewport_height) / (distance * 2 * tan(fov_y / 2))
        let sse_denominator = 2.0 * (fov_y / 2.0).tan();
        (self.geometric_error * viewport_height) / (distance * sse_denominator)
    }

    /// 返回子瓦片坐标。
    pub fn children_coords(&self) -> [(u32, u32); 4] {
        let child_x = self.x * 2;
        let child_y = self.y * 2;
        [
            (child_x, child_y),
            (child_x + 1, child_y),
            (child_x, child_y + 1),
            (child_x + 1, child_y + 1),
        ]
    }
}

/// 瓦片加载/渲染状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileState {
    /// 瓦片未加载。
    #[default]
    Unloaded,
    /// 瓦片止在加载。
    Loading,
    /// 瓦片已加载并可渲染。
    Loaded,
    /// 瓦片当前正在渲染。
    Rendered,
    /// 瓦片已细化（改为渲染其子节点）。
    Refined,
}

/// 四叉树遍历配置。
#[derive(Debug, Clone)]
pub struct QuadtreeConfig {
    /// 最大屏幕空间误差阈值（像素）。
    pub maximum_screen_space_error: f64,
    /// 最大瓦片层级。
    pub maximum_level: u32,
    /// 最小瓦片层级。
    pub minimum_level: u32,
    /// 是否启用雾敲除。
    pub fog_culling: bool,
    /// 用于敲除的雾密度。
    pub fog_density: f64,
}

impl Default for QuadtreeConfig {
    /// 返回缺省遍历配置：最大 SSE 2.0、最大层级 22、禁用雾剔除。
    fn default() -> Self {
        Self {
            maximum_screen_space_error: 2.0,
            maximum_level: 22,
            minimum_level: 0,
            fog_culling: false,
            fog_density: 0.0002,
        }
    }
}

/// 四叉树遍历的结果。
#[derive(Debug, Clone, Default)]
pub struct TraversalResult {
    /// 待渲染的瓦片。
    pub tiles_to_render: Vec<QuadtreeTile>,
    /// 待加载的瓦片。
    pub tiles_to_load: Vec<QuadtreeTile>,
    /// 访问的瓦片总数。
    pub tiles_visited: u32,
    /// 到达的最大深度。
    pub max_depth: u32,
}

/// 用于地形瓦片管理的四叉树基本体。
///
/// 从一组根瓦片出发递归遍历，依据屏幕空间误差决定细化或收录。
#[derive(Debug)]
pub struct QuadtreePrimitive {
    /// 根瓦片（WGS84 通常为 2 个：西半球与东半球）。
    pub root_tiles: Vec<QuadtreeTile>,
    /// 遍历配置。
    pub config: QuadtreeConfig,
}

impl QuadtreePrimitive {
    /// 创建一个新四叉树基本体。
    pub fn new(root_tiles: Vec<QuadtreeTile>, config: QuadtreeConfig) -> Self {
        Self { root_tiles, config }
    }

    /// 遍历四叉树并为渲染选择瓦片。
    ///
    /// # 参数
    /// * `camera_position` - 相机的世界空间位置
    /// * `viewport_height` - 视口高度（像素）
    /// * `fov_y` - 垂直视场角（弧度）
    /// * `tile_provider` - 获取子瓦片的函数
    pub fn traverse<F>(
        &self,
        camera_position: DVec3,
        viewport_height: f64,
        fov_y: f64,
        tile_provider: &F,
    ) -> TraversalResult
    where
        F: Fn(u32, u32, u32) -> Option<QuadtreeTile>,
    {
        let mut result = TraversalResult::default();

        for root in &self.root_tiles {
            self.visit_tile(
                root,
                camera_position,
                viewport_height,
                fov_y,
                tile_provider,
                &mut result,
            );
        }

        result
    }

    /// 递归访问单个瓦片：据 SSE 判定是否细化到子瓦片，否则将本瓦片收录用于渲染。
    fn visit_tile<F>(
        &self,
        tile: &QuadtreeTile,
        camera_position: DVec3,
        viewport_height: f64,
        fov_y: f64,
        tile_provider: &F,
        result: &mut TraversalResult,
    ) where
        F: Fn(u32, u32, u32) -> Option<QuadtreeTile>,
    {
        result.tiles_visited += 1;
        result.max_depth = result.max_depth.max(tile.level);

        // 检查瓦片是否可见（视锥敲除可在此实现）
        // 目前提设所有瓦片都可见

        // 计算屏幕空间误差
        let sse = tile.compute_screen_space_error(camera_position, viewport_height, fov_y);

        // 检查是否应细化该瓦片
        let should_refine = tile.refineable
            && tile.level < self.config.maximum_level
            && sse > self.config.maximum_screen_space_error;

        if should_refine {
            // 尝试加载并访问子节点
            let children_coords = tile.children_coords();
            let mut all_children_loaded = true;

            for (cx, cy) in children_coords {
                if let Some(child) = tile_provider(cx, cy, tile.level + 1) {
                    if child.state == TileState::Loaded || child.state == TileState::Rendered {
                        self.visit_tile(
                            &child,
                            camera_position,
                            viewport_height,
                            fov_y,
                            tile_provider,
                            result,
                        );
                    } else {
                        all_children_loaded = false;
                        result.tiles_to_load.push(child);
                    }
                } else {
                    all_children_loaded = false;
                }
            }

            // 若并非所有子节点都已加载，则回退渲染该瓦片
            if !all_children_loaded && tile.has_content {
                result.tiles_to_render.push(tile.clone());
            }
        } else if tile.has_content {
            // 渲染该瓦片
            result.tiles_to_render.push(tile.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tile(x: u32, y: u32, level: u32, geometric_error: f64) -> QuadtreeTile {
        QuadtreeTile::new(
            x,
            y,
            level,
            BoundingSphere::new(DVec3::ZERO, 1000000.0),
            geometric_error,
        )
    }

    #[test]
    fn test_quadtree_tile_creation() {
        let tile = create_test_tile(0, 0, 0, 100000.0);
        assert_eq!(tile.x, 0);
        assert_eq!(tile.y, 0);
        assert_eq!(tile.level, 0);
        assert_eq!(tile.geometric_error, 100000.0);
        assert_eq!(tile.state, TileState::Unloaded);
    }

    #[test]
    fn test_screen_space_error() {
        let tile = create_test_tile(0, 0, 0, 10000.0);
        let camera_position = DVec3::new(0.0, 0.0, 2000000.0);
        let viewport_height = 1080.0;
        let fov_y = std::f64::consts::FRAC_PI_4; // 45 度

        let sse = tile.compute_screen_space_error(camera_position, viewport_height, fov_y);

        // SSE 应为正且合理
        assert!(sse > 0.0);
        assert!(sse < 10000.0); // 不应大得离谱
    }

    #[test]
    fn test_sse_decreases_with_distance() {
        let tile = create_test_tile(0, 0, 0, 10000.0);
        let viewport_height = 1080.0;
        let fov_y = std::f64::consts::FRAC_PI_4;

        let sse_near = tile.compute_screen_space_error(
            DVec3::new(0.0, 0.0, 1500000.0),
            viewport_height,
            fov_y,
        );
        let sse_far = tile.compute_screen_space_error(
            DVec3::new(0.0, 0.0, 5000000.0),
            viewport_height,
            fov_y,
        );

        assert!(sse_near > sse_far);
    }

    #[test]
    fn test_children_coords() {
        let tile = create_test_tile(1, 2, 3, 10000.0);
        let children = tile.children_coords();

        assert_eq!(children[0], (2, 4));
        assert_eq!(children[1], (3, 4));
        assert_eq!(children[2], (2, 5));
        assert_eq!(children[3], (3, 5));
    }

    #[test]
    fn test_quadtree_config_default() {
        let config = QuadtreeConfig::default();
        assert_eq!(config.maximum_screen_space_error, 2.0);
        assert_eq!(config.maximum_level, 22);
        assert_eq!(config.minimum_level, 0);
        assert!(!config.fog_culling);
    }

    #[test]
    fn test_traversal_single_tile() {
        let root = create_test_tile(0, 0, 0, 100.0); // 低几何误差 = 低 SSE
        let primitive = QuadtreePrimitive::new(
            vec![root],
            QuadtreeConfig {
                maximum_screen_space_error: 2.0,
                maximum_level: 10,
                ..Default::default()
            },
        );

        let camera_position = DVec3::new(0.0, 0.0, 10000000.0); // 远处
        let result = primitive.traverse(camera_position, 1080.0, std::f64::consts::FRAC_PI_4, &|_, _, _| None);

        // 应渲染根瓦片（SSE 低于阈值）
        assert_eq!(result.tiles_to_render.len(), 1);
        assert_eq!(result.tiles_visited, 1);
    }

    #[test]
    fn test_traversal_with_refinement() {
        let root = create_test_tile(0, 0, 0, 1000000.0); // 高几何误差 = 高 SSE
        let primitive = QuadtreePrimitive::new(
            vec![root],
            QuadtreeConfig {
                maximum_screen_space_error: 2.0,
                maximum_level: 10,
                ..Default::default()
            },
        );

        let camera_position = DVec3::new(0.0, 0.0, 2000000.0); // 近处

        // 提供子节点（它们将为 Unloaded，因此加入加载队列）
        let result = primitive.traverse(camera_position, 1080.0, std::f64::consts::FRAC_PI_4, &|x, y, level| {
            if level <= 2 {
                Some(create_test_tile(x, y, level, 1000000.0 / (level as f64 + 1.0)))
            } else {
                None
            }
        });

        // 根瓦片应作为回退被渲染（子瓦片尚未加载）
        assert_eq!(result.tiles_to_render.len(), 1);
        // 子瓦片应被排入加载队列
        assert_eq!(result.tiles_to_load.len(), 4);
        assert_eq!(result.tiles_visited, 1);
    }

    #[test]
    fn test_tile_state_default() {
        assert_eq!(TileState::default(), TileState::Unloaded);
    }
}
