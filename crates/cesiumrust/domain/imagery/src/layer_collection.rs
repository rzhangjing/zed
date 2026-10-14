//! 影像图层集合。
//!
//! 按叠放次序管理多个 [`ImageryLayer`]，提供增删、调序、查找与可见性控制，
//! 是影像叠加的中枢容器。

use crate::imagery_layer::ImageryLayer;

/// 一个带排序的影像图层集合。
///
/// 图层按从底部（索引 0）到顶部的顺序渲染。
/// 映射到 CesiumJS `ImageryLayerCollection`
#[derive(Debug, Clone, Default)]
pub struct ImageryLayerCollection {
    /// 按从底到顶顺序排列的图层。
    layers: Vec<ImageryLayer>,

    /// 用于生成唯一图层 ID 的计数器。
    next_id: u64,
}

impl ImageryLayerCollection {
    /// 创建一个空的集合。
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            next_id: 1,
        }
    }

    /// 向集合顶部添加一个图层。
    ///
    /// # 返回
    /// 分配给该图层的 ID
    pub fn add(&mut self, mut layer: ImageryLayer) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        layer.id = id;
        self.layers.push(layer);
        id
    }

    /// 在指定索引处添加一个图层。
    ///
    /// # 返回
    /// 分配给该图层的 ID
    pub fn add_at(&mut self, mut layer: ImageryLayer, index: usize) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        layer.id = id;
        let index = index.min(self.layers.len());
        self.layers.insert(index, layer);
        id
    }

    /// 按 ID 移除一个图层。
    ///
    /// # 返回
    /// 被移除的图层，若找到
    pub fn remove(&mut self, id: u64) -> Option<ImageryLayer> {
        if let Some(index) = self.layers.iter().position(|l| l.id == id) {
            Some(self.layers.remove(index))
        } else {
            None
        }
    }

    /// 移除指定索引处的图层。
    ///
    /// # 返回
    /// 被移除的图层，若索引有效
    pub fn remove_at(&mut self, index: usize) -> Option<ImageryLayer> {
        if index < self.layers.len() {
            Some(self.layers.remove(index))
        } else {
            None
        }
    }

    /// 按 ID 获取一个图层。
    pub fn get(&self, id: u64) -> Option<&ImageryLayer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// 按 ID 获取一个可变图层。
    pub fn get_mut(&mut self, id: u64) -> Option<&mut ImageryLayer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// 按索引获取一个图层。
    pub fn get_at(&self, index: usize) -> Option<&ImageryLayer> {
        self.layers.get(index)
    }

    /// 按索引获取一个可变图层。
    pub fn get_at_mut(&mut self, index: usize) -> Option<&mut ImageryLayer> {
        self.layers.get_mut(index)
    }

    /// 返回图层数量。
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// 若集合为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// 返回一个遍历各图层的迭代器。
    pub fn iter(&self) -> impl Iterator<Item = &ImageryLayer> {
        self.layers.iter()
    }

    /// 返回一个可变遍历各图层的迭代器。
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ImageryLayer> {
        self.layers.iter_mut()
    }

    /// 将图层在集合中上移（朝顶部）。
    pub fn raise(&mut self, id: u64) {
        if let Some(index) = self.layers.iter().position(|l| l.id == id) {
            if index < self.layers.len() - 1 {
                self.layers.swap(index, index + 1);
            }
        }
    }

    /// 将图层在集合中下移（朝底部）。
    pub fn lower(&mut self, id: u64) {
        if let Some(index) = self.layers.iter().position(|l| l.id == id) {
            if index > 0 {
                self.layers.swap(index, index - 1);
            }
        }
    }

    /// 将图层移到集合顶部。
    pub fn raise_to_top(&mut self, id: u64) {
        if let Some(index) = self.layers.iter().position(|l| l.id == id) {
            let layer = self.layers.remove(index);
            self.layers.push(layer);
        }
    }

    /// 将图层移到集合底部。
    pub fn lower_to_bottom(&mut self, id: u64) {
        if let Some(index) = self.layers.iter().position(|l| l.id == id) {
            let layer = self.layers.remove(index);
            self.layers.insert(0, layer);
        }
    }

    /// 按 ID 返回图层索引。
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// 仅返回可见的图层。
    pub fn visible_layers(&self) -> impl Iterator<Item = &ImageryLayer> {
        self.layers.iter().filter(|l| l.show)
    }

    /// 给定所有可见图层，计算一个像素的混合 alpha。
    ///
    /// 它实现从底到顶的标准 alpha 合成。
    ///
    /// # 参数
    /// * `layer_alphas` - 每个图层的 alpha 值（按集合顺序）
    ///
    /// # 返回
    /// 最终的混合 alpha 值
    pub fn compute_blended_alpha(&self, layer_alphas: &[f64]) -> f64 {
        let mut result = 0.0;
        let mut remaining = 1.0;

        for (layer, &alpha) in self.layers.iter().zip(layer_alphas.iter()) {
            if !layer.show {
                continue;
            }
            let contribution = alpha * remaining;
            result += contribution;
            remaining *= 1.0 - alpha;
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_geospatial::rectangle::Rectangle;

    fn create_test_layer() -> ImageryLayer {
        ImageryLayer::new(0, Rectangle::MAX_VALUE)
    }

    #[test]
    fn test_add_and_get() {
        let mut collection = ImageryLayerCollection::new();
        let id = collection.add(create_test_layer());

        assert_eq!(collection.len(), 1);
        assert!(collection.get(id).is_some());
    }

    #[test]
    fn test_remove() {
        let mut collection = ImageryLayerCollection::new();
        let id = collection.add(create_test_layer());

        assert!(collection.remove(id).is_some());
        assert_eq!(collection.len(), 0);
    }

    #[test]
    fn test_ordering() {
        let mut collection = ImageryLayerCollection::new();
        let id1 = collection.add(create_test_layer());
        let id2 = collection.add(create_test_layer());
        let id3 = collection.add(create_test_layer());

        assert_eq!(collection.index_of(id1), Some(0));
        assert_eq!(collection.index_of(id2), Some(1));
        assert_eq!(collection.index_of(id3), Some(2));

        collection.raise(id1);
        assert_eq!(collection.index_of(id1), Some(1));
        assert_eq!(collection.index_of(id2), Some(0));

        collection.raise_to_top(id1);
        assert_eq!(collection.index_of(id1), Some(2));

        collection.lower_to_bottom(id1);
        assert_eq!(collection.index_of(id1), Some(0));
    }

    #[test]
    fn test_visible_layers() {
        let mut collection = ImageryLayerCollection::new();
        collection.add(create_test_layer().with_show(true));
        collection.add(create_test_layer().with_show(false));
        collection.add(create_test_layer().with_show(true));

        assert_eq!(collection.visible_layers().count(), 2);
    }

    #[test]
    fn test_blended_alpha() {
        let mut collection = ImageryLayerCollection::new();
        collection.add(create_test_layer());
        collection.add(create_test_layer());

        // 两个图层，每个 alpha 0.5
        // 结果 = 0.5 + 0.5 * 0.5 = 0.75
        let alphas = vec![0.5, 0.5];
        let blended = collection.compute_blended_alpha(&alphas);
        assert!((blended - 0.75).abs() < 1e-10);
    }
}
