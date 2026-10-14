//! DataSourceDisplay：实体可视化的中央协调器。
//!
//! 为一组数据源协调所有 visualizer（geometry、billboard、label、point、model、path），
//! 并在每帧更新它们：几何交给 `GeometryVisualizer` 增量重建，billboard、label、
//! point 则先清空再依据实体图形逐帧同步为对应的图元集合；坐标统一经所选椭球
//! 从弧度经纬高转换到直角位置。`MultiDataSourceDisplay` 在其上再聚合多个数据源
//! 的实体后统一驱动一次更新。

use cesium_geospatial::Ellipsoid;

use crate::entity_collection::{DataSource, EntityCollection};
use crate::geometry_updater::EntityGeometry;
use crate::primitives::{
    Billboard, BillboardCollection, Label, LabelCollection, PointPrimitive,
    PointPrimitiveCollection,
};
use crate::visualizer::GeometryVisualizer;

/// 所有数据源的显示状态。
///
/// 聚合一个几何 visualizer 与 billboard/label/point 三张图元集合，持有用于坐标
/// 转换的椭球；`initialized` 与 `last_time` 共同决定本帧是否需要重新同步图元。
#[derive(Debug)]
pub struct DataSourceDisplay {
    /// 几何 visualizer。
    geometry_visualizer: GeometryVisualizer,
    /// 所有实体的 billboard 集合。
    pub billboards: BillboardCollection,
    /// 所有实体的 label 集合。
    pub labels: LabelCollection,
    /// 所有实体的 point 图元集合。
    pub points: PointPrimitiveCollection,
    /// 用于坐标转换的椭球。
    ellipsoid: Ellipsoid,
    /// 显示是否已初始化。
    initialized: bool,
    /// 上一次更新的时间。
    last_time: f64,
}

impl DataSourceDisplay {
    /// 创建新的数据源显示。
    ///
    /// 以传入椭球初始化几何 visualizer，并建立空的 billboard/label/point
    /// 集合；初始为未初始化、上次时间置 0，首次 update 必触发同步。
    pub fn new(ellipsoid: Ellipsoid) -> Self {
        Self {
            geometry_visualizer: GeometryVisualizer::new(ellipsoid),
            billboards: BillboardCollection::new(),
            labels: LabelCollection::new(),
            points: PointPrimitiveCollection::new(),
            ellipsoid,
            initialized: false,
            last_time: 0.0,
        }
    }

    /// 创建一个使用 WGS84 椭球的新数据源显示。
    pub fn wgs84() -> Self {
        Self::new(Ellipsoid::WGS84)
    }

    /// 在给定时间处为给定实体集合更新显示。
    ///
    /// 这是每帧的主更新方法。它会：
    /// 1. 更新几何 visualizer
    /// 2. 将 billboard/label/point 集合与实体同步
    pub fn update(&mut self, entities: &EntityCollection, time: f64) {
        // 更新几何
        self.geometry_visualizer.update(entities, time);

        // 同步 billboard/label/point 集合
        if !self.initialized || (time - self.last_time).abs() > f64::EPSILON {
            self.sync_primitives(entities, time);
            self.initialized = true;
        }

        // 记录本次时间，供下一帧判断是否需要重新同步。
        self.last_time = time;
    }

    /// 将 billboard、label 和 point 集合与实体图形同步。
    ///
    /// 先清空三张图元集合，再逐实体跳过隐藏项、把弧度位置转直角坐标，
    /// 并按实体携带的图形类型逐项求值其属性（缺省回退）后加入对应集合。
    fn sync_primitives(&mut self, entities: &EntityCollection, time: f64) {
        // 先清空三张集合，避免残留上一帧的图元。
        self.billboards.clear();
        self.labels.clear();
        self.points.clear();

        // 逐实体处理，整体隐藏的实体直接跳过。
        for entity in entities.values() {
            if !entity.show {
                continue;
            }

            // 把弧度位置经椭球转到直角坐标，无位置时回退为原点。
            let position = entity
                .position
                .get_value(time)
                .map(|p| {
                    let carto = cesium_geospatial::Cartographic::from_radians(p[0], p[1], p[2]);
                    let cart = self.ellipsoid.cartographic_to_cartesian(&carto);
                    [cart.x, cart.y, cart.z]
                })
                .unwrap_or([0.0; 3]);

            // Billboard（广告牌）
            if let Some(ref bb_graphics) = entity.billboard {
                let show = bb_graphics.show.get_value(time).copied().unwrap_or(true);
                if show {
                    let billboard = Billboard {
                        show: true,
                        position,
                        scale: bb_graphics.scale.get_value(time).copied().unwrap_or(1.0),
                        color: bb_graphics.color.get_value(time).copied().unwrap_or(crate::property::Color::WHITE),
                        rotation: bb_graphics.rotation.get_value(time).copied().unwrap_or(0.0),
                        width: bb_graphics.width.get_value(time).copied(),
                        height: bb_graphics.height.get_value(time).copied(),
                        image: bb_graphics.image.get_value(time).cloned(),
                        id: Some(entity.id.clone()),
                        ..Default::default()
                    };
                    self.billboards.add(billboard);
                }
            }

            // Label（标签）
            if let Some(ref label_graphics) = entity.label {
                let show = label_graphics.show.get_value(time).copied().unwrap_or(true);
                if show {
                    let label = Label {
                        show: true,
                        position,
                        text: label_graphics.text.get_value(time).cloned().unwrap_or_default(),
                        font: label_graphics.font.get_value(time).cloned().unwrap_or_else(|| "30px sans-serif".to_string()),
                        fill_color: label_graphics.fill_color.get_value(time).copied().unwrap_or(crate::property::Color::WHITE),
                        outline_color: label_graphics.outline_color.get_value(time).copied().unwrap_or(crate::property::Color::BLACK),
                        outline_width: label_graphics.outline_width.get_value(time).copied().unwrap_or(2.0),
                        id: Some(entity.id.clone()),
                        ..Default::default()
                    };
                    self.labels.add(label);
                }
            }

            // Point（点）
            if let Some(ref point_graphics) = entity.point {
                let show = point_graphics.show.get_value(time).copied().unwrap_or(true);
                if show {
                    let point = PointPrimitive {
                        show: true,
                        position,
                        color: point_graphics.color.get_value(time).copied().unwrap_or(crate::property::Color::WHITE),
                        outline_color: point_graphics.outline_color.get_value(time).copied().unwrap_or(crate::property::Color::BLACK),
                        outline_width: point_graphics.outline_width.get_value(time).copied().unwrap_or(0.0),
                        pixel_size: point_graphics.pixel_size.get_value(time).copied().unwrap_or(1.0),
                        id: Some(entity.id.clone()),
                        ..Default::default()
                    };
                    self.points.add(point);
                }
            }
        }
    }

    /// 获取几何 visualizer。
    pub fn geometry_visualizer(&self) -> &GeometryVisualizer {
        &self.geometry_visualizer
    }

    /// 获取特定实体的几何。
    pub fn get_entity_geometry(&self, entity_id: &str) -> Option<&EntityGeometry> {
        self.geometry_visualizer.get_geometry(entity_id)
    }

    /// 几何实例总数。
    pub fn geometry_instance_count(&self) -> usize {
        self.geometry_visualizer.instance_count()
    }

    /// billboard 数量。
    pub fn billboard_count(&self) -> usize {
        self.billboards.len()
    }

    /// label 数量。
    pub fn label_count(&self) -> usize {
        self.labels.len()
    }

    /// point 数量。
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// 将显示标记为需要完全重建。
    ///
    /// 同时标脏几何 visualizer 并置 initialized 为假，使下一帧无论时间是否
    /// 变化都会重新同步全部图元。
    pub fn mark_dirty(&mut self) {
        self.geometry_visualizer.mark_dirty();
        // 复位初始化标志，强制下一帧重新同步全部图元。
        self.initialized = false;
    }
}

/// 管理多个数据源的数据源显示。
///
/// 在单个 `DataSourceDisplay` 之上维护一组 `DataSource`，更新时先把各源的实体
/// 合并到一个临时集合再统一驱动底层显示；增删数据源都会标脏以触发重建。
#[derive(Debug)]
pub struct MultiDataSourceDisplay {
    /// 底层显示。
    display: DataSourceDisplay,
    /// 被跟踪的数据源。
    sources: Vec<DataSource>,
}

impl MultiDataSourceDisplay {
    /// 创建新的多数据源显示。
    pub fn new(ellipsoid: Ellipsoid) -> Self {
        Self {
            display: DataSourceDisplay::new(ellipsoid),
            sources: Vec::new(),
        }
    }

    /// 创建一个使用 WGS84 椭球的新多数据源显示。
    pub fn wgs84() -> Self {
        Self::new(Ellipsoid::WGS84)
    }

    /// 添加一个数据源。
    ///
    /// 追入到源列表末尾并标脏底层显示，以便下次更新重算。
    pub fn add_data_source(&mut self, source: DataSource) {
        self.sources.push(source);
        self.display.mark_dirty();
    }

    /// 按名称移除一个数据源。
    ///
    /// 找到首个同名源则标脏并移除、返回该源；无匹配时返回 `None`。
    pub fn remove_data_source(&mut self, name: &str) -> Option<DataSource> {
        if let Some(idx) = self.sources.iter().position(|s| s.name == name) {
            self.display.mark_dirty();
            Some(self.sources.remove(idx))
        } else {
            None
        }
    }

    /// 在给定时间处更新所有数据源。
    ///
    /// 把各源的实体克隆合并到一个临时集合，再统一驱动底层显示在该时刻
    /// 更新，从而将多个数据源聚合为一份视图。
    pub fn update(&mut self, time: f64) {
        // 合并来自所有数据源的实体
        let mut merged = EntityCollection::new();
        for source in &self.sources {
            for entity in source.entities.values() {
                merged.add(entity.clone());
            }
        }
        // 用合并后的临时集合驱动底层显示在该时刻更新。
        self.display.update(&merged, time);
    }

    /// 获取底层显示。
    pub fn display(&self) -> &DataSourceDisplay {
        &self.display
    }

    /// 数据源数量。
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::*;
    use crate::property::Property;

    fn make_entities() -> EntityCollection {
        let mut collection = EntityCollection::new();

        // 带方框几何的实体
        collection.add(
            Entity::new("box-1")
                .with_position(0.0, 0.0, 0.0)
                .with_box(BoxGraphics {
                    dimensions: Property::Constant([100.0, 100.0, 100.0]),
                    ..Default::default()
                }),
        );

        // 带 billboard 的实体
        collection.add(
            Entity::new("bb-1")
                .with_position(0.1, 0.1, 0.0)
                .with_billboard(BillboardGraphics {
                    image: Property::Constant("marker.png".to_string()),
                    scale: Property::Constant(2.0),
                    ..Default::default()
                }),
        );

        // 带 label 的实体
        collection.add(
            Entity::new("label-1")
                .with_position(0.2, 0.2, 0.0)
                .with_label(LabelGraphics {
                    text: Property::Constant("Hello".to_string()),
                    ..Default::default()
                }),
        );

        // 带 point 的实体
        collection.add(
            Entity::new("point-1")
                .with_position(0.3, 0.3, 0.0)
                .with_point(PointGraphics {
                    pixel_size: Property::Constant(10.0),
                    color: Property::Constant(crate::property::Color::RED),
                    ..Default::default()
                }),
        );

        collection
    }

    /// 验证主更新流程：四类实体各生成一项，几何/广告牌/标签/点计数均为 1。
    #[test]
    fn test_data_source_display_update() {
        let mut display = DataSourceDisplay::wgs84();
        let entities = make_entities();

        display.update(&entities, 0.0);

        assert_eq!(display.geometry_instance_count(), 1); // 方框
        assert_eq!(display.billboard_count(), 1);
        assert_eq!(display.label_count(), 1);
        assert_eq!(display.point_count(), 1);
    }

    /// 验证几何提取：按 id 取回 box-1 的几何，其填充实例数为 1。
    #[test]
    fn test_data_source_display_geometry() {
        let mut display = DataSourceDisplay::wgs84();
        let entities = make_entities();

        display.update(&entities, 0.0);

        let geo = display.get_entity_geometry("box-1").unwrap();
        assert_eq!(geo.fill_instances.len(), 1);
    }

    /// 验证隐藏实体：show=false 的广告牌实体不会同步出任何图元，计数为 0。
    #[test]
    fn test_data_source_display_hidden_entity() {
        let mut display = DataSourceDisplay::wgs84();
        let mut entities = EntityCollection::new();

        let mut entity = Entity::new("hidden-bb")
            .with_position(0.0, 0.0, 0.0)
            .with_billboard(BillboardGraphics {
                image: Property::Constant("test.png".to_string()),
                ..Default::default()
            });
        entity.show = false;
        entities.add(entity);

        display.update(&entities, 0.0);
        assert_eq!(display.billboard_count(), 0);
    }

    /// 验证多源聚合：两个源各含一个实体，合并更新后底层只呈现一个几何与
    /// 一个点。
    #[test]
    fn test_multi_data_source_display() {
        let mut multi = MultiDataSourceDisplay::wgs84();

        let mut source1 = DataSource::new("source-1");
        source1.entities.add(
            Entity::new("s1-box")
                .with_position(0.0, 0.0, 0.0)
                .with_box(BoxGraphics {
                    dimensions: Property::Constant([50.0, 50.0, 50.0]),
                    ..Default::default()
                }),
        );

        let mut source2 = DataSource::new("source-2");
        source2.entities.add(
            Entity::new("s2-point")
                .with_position(0.1, 0.1, 0.0)
                .with_point(PointGraphics {
                    pixel_size: Property::Constant(5.0),
                    ..Default::default()
                }),
        );

        multi.add_data_source(source1);
        multi.add_data_source(source2);
        assert_eq!(multi.source_count(), 2);

        multi.update(0.0);
        assert_eq!(multi.display().geometry_instance_count(), 1);
        assert_eq!(multi.display().point_count(), 1);
    }

    /// 验证按名移除：移除已存在的临时源后计数归零并返回 Some。
    #[test]
    fn test_multi_data_source_remove() {
        let mut multi = MultiDataSourceDisplay::wgs84();
        multi.add_data_source(DataSource::new("temp"));
        assert_eq!(multi.source_count(), 1);

        let removed = multi.remove_data_source("temp");
        assert!(removed.is_some());
        assert_eq!(multi.source_count(), 0);
    }

    /// 验证标脏重建：mark_dirty 后以相同时间再次更新，几何实例数保持不变。
    #[test]
    fn test_display_mark_dirty() {
        let mut display = DataSourceDisplay::wgs84();
        let entities = make_entities();

        display.update(&entities, 0.0);
        let count1 = display.geometry_instance_count();

        display.mark_dirty();
        display.update(&entities, 0.0);
        let count2 = display.geometry_instance_count();

        assert_eq!(count1, count2);
    }
}
