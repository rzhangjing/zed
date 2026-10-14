//! 实体聚类与实体视图（相机跟随）。
//!
//! 本模块提供两块能力：按屏幕邻近度把相邻实体聚合为簇的聚类引擎，
//! 以及让相机跟随、追踪或注视某个实体的实体视图。

// 遗留 CesiumJS 移植的风格技术债（deferred.md #18）；在 M13 lint 清理
// 或本文件在其里程碑被重写时重新审视
#![allow(unused_imports)]
use crate::entity::Entity;
use crate::entity_collection::EntityCollection;
use std::collections::HashMap;

/// 网格单元键：(cell_x, cell_y)。
type GridKey = (i64, i64);
/// 实体位置条目：(entity_id, position)。
type EntityPos = (String, [f64; 3]);
/// 用于聚类的空间哈希网格。
type ClusterGrid = HashMap<GridKey, Vec<EntityPos>>;

/// 一簇相邻实体。
#[derive(Debug, Clone)]
pub struct Cluster {
    /// 质心位置 [lon_rad, lat_rad, height_m]。
    pub position: [f64; 3],
    /// 此簇中的实体 ID。
    pub entity_ids: Vec<String>,
    /// 簇中实体数量。
    pub count: usize,
}

impl Cluster {
    /// 若此簇仅包含一个实体则返回 true。
    pub fn is_single(&self) -> bool {
        self.count <= 1
    }
}

/// 实体聚类的配置。
///
/// 描述聚类是否启用、聚合的像素半径与成簇所需的最少实体数。
#[derive(Debug, Clone)]
pub struct EntityClusterOptions {
    /// 是否启用聚类。
    pub enabled: bool,
    /// 聚类的像素范围（在此范围内的实体会被聚为一簇）。
    pub pixel_range: f64,
    /// 形成一个簇所需的最少实体数量。
    pub minimum_cluster_size: usize,
}

impl Default for EntityClusterOptions {
    /// 默认启用聚类，像素半径 80，成簇最少 2 个实体。
    fn default() -> Self {
        Self {
            enabled: true,
            pixel_range: 80.0,
            minimum_cluster_size: 2,
        }
    }
}

/// 实体聚类引擎。
///
/// 基于屏幕空间邻近度将相邻实体分为若干簇。
/// 在本领域实现中，我们在地图空间中使用一个简单的基于网格的空间哈希，
/// 作为屏幕空间聚类的近似。
///
/// 簇心取成员均值，仅当网格桶内成员数达到阈值才成簇。
#[derive(Debug)]
pub struct EntityCluster {
    /// 聚类选项。
    pub options: EntityClusterOptions,
    /// 当前的簇。
    clusters: Vec<Cluster>,
    /// 网格单元大小（以弧度计，为像素范围的近似）。
    cell_size: f64,
}

impl EntityCluster {
    /// 使用默认选项创建新的实体聚类。
    pub fn new() -> Self {
        Self {
            options: EntityClusterOptions::default(),
            clusters: Vec::new(),
            cell_size: 0.01, // 约 0.57 度
        }
    }

    /// 使用自定义选项创建新的实体聚类。
    pub fn with_options(options: EntityClusterOptions) -> Self {
        let cell_size = options.pixel_range * 0.000125; // 近似换算
        Self {
            options,
            clusters: Vec::new(),
            cell_size,
        }
    }

    /// 在给定时间处为给定实体更新各簇。
    ///
    /// 使用基于网格的空间哈希来将相邻实体分组。
    pub fn update(&mut self, entities: &EntityCollection, time: f64) {
        self.clusters.clear();

        if !self.options.enabled {
            return;
        }

        // 基于网格的聚类
        let mut grid: ClusterGrid = HashMap::new();

        for entity in entities.values() {
            if !entity.show {
                continue;
            }

            if let Some(pos) = entity.position.get_value(time) {
                let cell_x = (pos[0] / self.cell_size).floor() as i64;
                let cell_y = (pos[1] / self.cell_size).floor() as i64;
                grid.entry((cell_x, cell_y))
                    .or_default()
                    .push((entity.id.clone(), *pos));
            }
        }

        // 将网格单元转换为簇
        for ((_cx, _cy), members) in grid {
            if members.len() >= self.options.minimum_cluster_size {
                // 计算质心
                let count = members.len();
                let mut lon_sum = 0.0;
                let mut lat_sum = 0.0;
                let mut h_sum = 0.0;
                let ids: Vec<String> = members.iter().map(|(id, _)| id.clone()).collect();
                for (_, pos) in &members {
                    lon_sum += pos[0];
                    lat_sum += pos[1];
                    h_sum += pos[2];
                }
                self.clusters.push(Cluster {
                    position: [
                        lon_sum / count as f64,
                        lat_sum / count as f64,
                        h_sum / count as f64,
                    ],
                    entity_ids: ids,
                    count,
                });
            } else {
                // 单个实体（未被聚类）
                for (id, pos) in members {
                    self.clusters.push(Cluster {
                        position: pos,
                        entity_ids: vec![id],
                        count: 1,
                    });
                }
            }
        }
    }

    /// 获取当前的各簇。
    pub fn clusters(&self) -> &[Cluster] {
        &self.clusters
    }

    /// 簇的数量。
    pub fn cluster_count(&self) -> usize {
        self.clusters.len()
    }

    /// 实际簇的数量（count > 1）。
    pub fn actual_cluster_count(&self) -> usize {
        self.clusters.iter().filter(|c| !c.is_single()).count()
    }

    /// 被聚类的实体总数。
    pub fn clustered_entity_count(&self) -> usize {
        self.clusters.iter().filter(|c| !c.is_single()).map(|c| c.count).sum()
    }
}

impl Default for EntityCluster {
    /// 以默认选项构造一个空的聚类引擎。
    fn default() -> Self {
        Self::new()
    }
}

/// EntityView 的相机跟随模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityViewMode {
    /// 相机跟随实体位置。
    #[default]
    Follow,
    /// 相机以固定偏移追踪实体。
    Track,
    /// 相机从远处注视实体。
    LookAt,
}

/// 实体视图：使相机跟随/追踪一个实体。
///
/// 依据所选模式在每帧计算相机目标位姿，使视角稳定锁定到实体。
#[derive(Debug, Clone)]
pub struct EntityView {
    /// 被跟随的实体 ID。
    pub entity_id: String,
    /// 视图模式。
    pub mode: EntityViewMode,
    /// 相对于实体的偏移 [x, y, z]，以米计（用于 Track/LookAt 模式）。
    pub offset: [f64; 3],
    /// 最后已知的实体位置 [x, y, z]，以 Cartesian3 表示。
    pub last_position: [f64; 3],
    /// 视图是否处于活动状态。
    pub active: bool,
}

impl EntityView {
    /// 创建一个新的实体视图，跟随给定实体。
    pub fn new(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            mode: EntityViewMode::Follow,
            offset: [0.0, 0.0, 0.0],
            last_position: [0.0; 3],
            active: true,
        }
    }

    /// 创建一个带偏移的追踪型实体视图。
    pub fn tracking(entity_id: impl Into<String>, offset: [f64; 3]) -> Self {
        Self {
            entity_id: entity_id.into(),
            mode: EntityViewMode::Track,
            offset,
            last_position: [0.0; 3],
            active: true,
        }
    }

    /// 在给定时间处为给定实体更新视图。
    ///
    /// 返回目标相机位置（实体位置 + 偏移）。
    pub fn update(
        &mut self,
        entity: &Entity,
        time: f64,
        ellipsoid: &cesium_geospatial::Ellipsoid,
    ) -> Option<[f64; 3]> {
        if !self.active {
            return None;
        }

        let pos = entity.position.get_value(time)?;
        let carto = cesium_geospatial::Cartographic::from_radians(pos[0], pos[1], pos[2]);
        let cart = ellipsoid.cartographic_to_cartesian(&carto);

        self.last_position = [cart.x, cart.y, cart.z];

        let target = match self.mode {
            EntityViewMode::Follow => [cart.x, cart.y, cart.z],
            EntityViewMode::Track | EntityViewMode::LookAt => [
                cart.x + self.offset[0],
                cart.y + self.offset[1],
                cart.z + self.offset[2],
            ],
        };

        Some(target)
    }

    /// 停用视图。
    pub fn deactivate(&mut self) {
        self.active = false;
    }

    /// 激活视图。
    pub fn activate(&mut self) {
        self.active = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::PointGraphics;
    use crate::property::Property;

    fn make_cluster_entities() -> EntityCollection {
        let mut collection = EntityCollection::new();
        // 一组 3 个相邻实体
        collection.add(Entity::new("p1").with_position(0.0, 0.0, 0.0).with_point(PointGraphics::default()));
        collection.add(Entity::new("p2").with_position(0.001, 0.001, 0.0).with_point(PointGraphics::default()));
        collection.add(Entity::new("p3").with_position(0.002, 0.002, 0.0).with_point(PointGraphics::default()));
        // 孤立的实体，距离较远
        collection.add(Entity::new("p4").with_position(1.0, 1.0, 0.0).with_point(PointGraphics::default()));
        collection
    }

    #[test]
    fn test_entity_cluster_basic() {
        let mut cluster = EntityCluster::new();
        let entities = make_cluster_entities();

        cluster.update(&entities, 0.0);

        // 应含有若干簇
        assert!(cluster.cluster_count() > 0);
        // 至少一个实际的簇（count > 1）
        assert!(cluster.actual_cluster_count() >= 1);
    }

    #[test]
    fn test_entity_cluster_disabled() {
        let mut cluster = EntityCluster::with_options(EntityClusterOptions {
            enabled: false,
            ..Default::default()
        });
        let entities = make_cluster_entities();

        cluster.update(&entities, 0.0);
        assert_eq!(cluster.cluster_count(), 0);
    }

    #[test]
    fn test_entity_cluster_minimum_size() {
        let mut cluster = EntityCluster::with_options(EntityClusterOptions {
            enabled: true,
            pixel_range: 80.0,
            minimum_cluster_size: 5, // 需要 5 个才聚类
        });
        let entities = make_cluster_entities();

        cluster.update(&entities, 0.0);
        // 不应形成任何簇（最多 3 个相邻）
        assert_eq!(cluster.actual_cluster_count(), 0);
    }

    #[test]
    fn test_entity_cluster_single_entities() {
        let mut cluster = EntityCluster::new();
        let mut entities = EntityCollection::new();
        entities.add(Entity::new("solo").with_position(0.5, 0.5, 0.0));

        cluster.update(&entities, 0.0);
        assert_eq!(cluster.cluster_count(), 1);
        assert!(cluster.clusters()[0].is_single());
    }

    #[test]
    fn test_entity_view_follow() {
        let entity = Entity::new("vehicle").with_position(0.0, 0.0, 1000.0);
        let ellipsoid = cesium_geospatial::Ellipsoid::WGS84;

        let mut view = EntityView::new("vehicle");
        let target = view.update(&entity, 0.0, &ellipsoid).unwrap();

        // 位置应位于椭球表面 + 1000m
        let dist = (target[0] * target[0] + target[1] * target[1] + target[2] * target[2]).sqrt();
        assert!(dist > 6371000.0); // 地球半径 + 高度
    }

    #[test]
    fn test_entity_view_tracking() {
        let entity = Entity::new("sat").with_position(0.0, 0.0, 0.0);
        let ellipsoid = cesium_geospatial::Ellipsoid::WGS84;

        let mut view = EntityView::tracking("sat", [1000.0, 0.0, 0.0]);
        let target = view.update(&entity, 0.0, &ellipsoid).unwrap();

        // 目标应相对实体位置有偏移
        let entity_pos = ellipsoid.cartographic_to_cartesian(
            &cesium_geospatial::Cartographic::from_radians(0.0, 0.0, 0.0),
        );
        let dx = target[0] - entity_pos.x;
        assert!((dx - 1000.0).abs() < 1.0);
    }

    #[test]
    fn test_entity_view_inactive() {
        let entity = Entity::new("v").with_position(0.0, 0.0, 0.0);
        let ellipsoid = cesium_geospatial::Ellipsoid::WGS84;

        let mut view = EntityView::new("v");
        view.deactivate();
        assert!(view.update(&entity, 0.0, &ellipsoid).is_none());

        view.activate();
        assert!(view.update(&entity, 0.0, &ellipsoid).is_some());
    }

    #[test]
    fn test_cluster_centroid() {
        let mut cluster = EntityCluster::new();
        let mut entities = EntityCollection::new();
        entities.add(Entity::new("a").with_position(0.0, 0.0, 0.0));
        entities.add(Entity::new("b").with_position(0.002, 0.002, 0.0));

        cluster.update(&entities, 0.0);

        // 找到同时包含两个实体的簇
        let multi = cluster.clusters().iter().find(|c| c.count == 2);
        if let Some(c) = multi {
            // 质心应大致位于 (0.001, 0.001)
            assert!((c.position[0] - 0.001).abs() < 0.002);
            assert!((c.position[1] - 0.001).abs() < 0.002);
        }
    }
}
