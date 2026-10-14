//! billboard、label 与 PointPrimitive 集合。
//!
//! 本模块提供三类图元及其集合：始终面向屏幕的 billboard、
//! 随实体定位的 label，以及屏幕空间的点图元；每类集合统一控制可见性。

use crate::property::Color;

/// 用于 billboard/label 定位的垂直对齐原点。
///
/// 指定 billboard/label 在垂直方向的对齐基准。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalOrigin {
    /// 原点位于条目顶部。
    Top,
    /// 原点位于条目中心。
    #[default]
    Center,
    /// 原点位于条目底部。
    Bottom,
    /// 原点位于文本基线（仅 label）。
    Baseline,
}

/// 用于 billboard/label 定位的水平对齐原点。
///
/// 指定 billboard/label 在水平方向的对齐基准。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalOrigin {
    /// 原点位于条目左侧。
    Left,
    /// 原点位于条目中心。
    #[default]
    Center,
    /// 原点位于条目右侧。
    Right,
}

/// label 样式（填充、轮廓，或两者）。
///
/// 控制 label 是仅填充、仅轮廓还是两者兼具。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelStyle {
    /// 仅填充。
    #[default]
    Fill,
    /// 仅轮廓。
    Outline,
    /// 填充与轮廓。
    FillAndOutline,
}

/// 基于距离的缩放条件。
///
/// 以近/远距离及其对应值构成一条分段线性插值曲线。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearFarScalar {
    /// 近距。
    pub near: f64,
    /// 近距处的值。
    pub near_value: f64,
    /// 远距。
    pub far: f64,
    /// 远距处的值。
    pub far_value: f64,
}

impl NearFarScalar {
    /// 创建新的 near-far scalar。
    pub fn new(near: f64, near_value: f64, far: f64, far_value: f64) -> Self {
        Self { near, near_value, far, far_value }
    }

    /// 在给定距离处插值。
    pub fn value_at_distance(&self, distance: f64) -> f64 {
        if distance <= self.near {
            self.near_value
        } else if distance >= self.far {
            self.far_value
        } else {
            let t = (distance - self.near) / (self.far - self.near);
            self.near_value + t * (self.far_value - self.near_value)
        }
    }
}

impl Default for NearFarScalar {
    /// 缺省为 near=0、far=1，两端值均为 0。
    fn default() -> Self {
        Self { near: 0.0, near_value: 0.0, far: 1.0, far_value: 0.0 }
    }
}

/// 距离显示条件（近/远裁剪）。
///
/// 给出按相机距离显示图元的近远裁剪区间。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceDisplayCondition {
    /// 近距（米）。
    pub near: f64,
    /// 远距（米）。
    pub far: f64,
}

impl DistanceDisplayCondition {
    /// 将该对象打包进数组所用的元素数量。
    pub const PACKED_LENGTH: usize = 2;

    /// 创建新的距离显示条件。
    pub fn new(near: f64, far: f64) -> Self {
        Self { near, far }
    }

    /// 若给定距离落在条件范围内则返回 true。
    pub fn is_visible(&self, distance: f64) -> bool {
        distance >= self.near && distance <= self.far
    }

    /// 将所提供实例存入所提供数组。
    ///
    /// 映射到 CesiumJS `DistanceDisplayCondition.pack`
    pub fn pack(&self, array: &mut [f64], starting_index: usize) {
        array[starting_index] = self.near;
        array[starting_index + 1] = self.far;
    }

    /// 从打包数组中取出一个实例。
    ///
    /// 映射到 CesiumJS `DistanceDisplayCondition.unpack`
    pub fn unpack(array: &[f64], starting_index: usize) -> Self {
        Self {
            near: array[starting_index],
            far: array[starting_index + 1],
        }
    }

    /// 判断两个距离显示条件是否相等。
    ///
    /// 映射到 CesiumJS `DistanceDisplayCondition.equals`
    pub fn equals(&self, other: &Self) -> bool {
        self.near == other.near && self.far == other.far
    }
}

impl Default for DistanceDisplayCondition {
    /// 缺省裁剪区间为 [0, f64::MAX]，即始终显示。
    fn default() -> Self {
        Self { near: 0.0, far: f64::MAX }
    }
}

/// billboard 集合中的一个 billboard。
///
/// 承载单个 billboard 图像的位置、对齐与外观设置。
#[derive(Debug, Clone, PartialEq)]
pub struct Billboard {
    /// billboard 是否显示。
    pub show: bool,
    /// Cartesian3 位置 [x, y, z]。
    pub position: [f64; 3],
    /// 像素偏移 [x, y]。
    pub pixel_offset: [f64; 2],
    /// 眼睛偏移 [x, y, z]。
    pub eye_offset: [f64; 3],
    /// 垂直对齐原点。
    pub vertical_origin: VerticalOrigin,
    /// 水平对齐原点。
    pub horizontal_origin: HorizontalOrigin,
    /// 缩放因子。
    pub scale: f64,
    /// 颜色染色。
    pub color: Color,
    /// 旋转（弧度）。
    pub rotation: f64,
    /// 对齐轴 [x, y, z]。
    pub aligned_axis: [f64; 3],
    /// 宽度（像素）（None = 使用图像宽度）。
    pub width: Option<f64>,
    /// 高度（像素）（None = 使用图像高度）。
    pub height: Option<f64>,
    /// 尺寸是否以米而非像素计。
    pub size_in_meters: bool,
    /// 图像 URI 或 ID。
    pub image: Option<String>,
    /// 按距离缩放。
    pub scale_by_distance: Option<NearFarScalar>,
    /// 按距离半透明。
    pub translucency_by_distance: Option<NearFarScalar>,
    /// 距离显示条件。
    pub distance_display_condition: Option<DistanceDisplayCondition>,
    /// 用户自定义 ID。
    pub id: Option<String>,
}

impl Default for Billboard {
    /// 构造一个居中、白色、缩放 1 的默认可见 billboard。
    fn default() -> Self {
        Self {
            show: true,
            position: [0.0; 3],
            pixel_offset: [0.0; 2],
            eye_offset: [0.0; 3],
            vertical_origin: VerticalOrigin::Center,
            horizontal_origin: HorizontalOrigin::Center,
            scale: 1.0,
            color: Color::WHITE,
            rotation: 0.0,
            aligned_axis: [0.0; 3],
            width: None,
            height: None,
            size_in_meters: false,
            image: None,
            scale_by_distance: None,
            translucency_by_distance: None,
            distance_display_condition: None,
            id: None,
        }
    }
}

/// billboard 的集合。
///
/// 顺序持有一组 billboard，并统一控制整体可见性。
#[derive(Debug, Default)]
pub struct BillboardCollection {
    /// 集合内的 billboard 列表。
    billboards: Vec<Billboard>,
    /// 整个集合是否可见。
    show: bool,
}

impl BillboardCollection {
    /// 创建新的空集合。
    pub fn new() -> Self {
        Self { billboards: Vec::new(), show: true }
    }

    /// 向集合添加一个 billboard。
    pub fn add(&mut self, billboard: Billboard) -> usize {
        let index = self.billboards.len();
        self.billboards.push(billboard);
        index
    }

    /// 按索引移除一个 billboard。
    pub fn remove(&mut self, index: usize) -> Option<Billboard> {
        if index < self.billboards.len() {
            Some(self.billboards.remove(index))
        } else {
            None
        }
    }

    /// 按索引获取一个 billboard。
    pub fn get(&self, index: usize) -> Option<&Billboard> {
        self.billboards.get(index)
    }

    /// 按索引获取一个可变 billboard。
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Billboard> {
        self.billboards.get_mut(index)
    }

    /// billboard 数量。
    pub fn len(&self) -> usize {
        self.billboards.len()
    }

    /// 若为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.billboards.is_empty()
    }

    /// 遍历 billboards。
    pub fn iter(&self) -> impl Iterator<Item = &Billboard> {
        self.billboards.iter()
    }

    /// 集合是否显示。
    pub fn show(&self) -> bool {
        self.show
    }

    /// 设置集合是否显示。
    pub fn set_show(&mut self, show: bool) {
        self.show = show;
    }

    /// 清除所有 billboards。
    pub fn clear(&mut self) {
        self.billboards.clear();
    }
}

/// label 集合中的一个 label。
///
/// 承载单个 label 文本的位置、字体与描边设置。
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// label 是否显示。
    pub show: bool,
    /// Cartesian3 位置 [x, y, z]。
    pub position: [f64; 3],
    /// 文本内容。
    pub text: String,
    /// 字体（CSS 格式）。
    pub font: String,
    /// 填充颜色。
    pub fill_color: Color,
    /// 轮廓颜色。
    pub outline_color: Color,
    /// 轮廓宽度（像素）。
    pub outline_width: f64,
    /// label 样式。
    pub style: LabelStyle,
    /// 是否显示背景。
    pub show_background: bool,
    /// 背景颜色。
    pub background_color: Color,
    /// 背景内边距 [x, y]。
    pub background_padding: [f64; 2],
    /// 垂直对齐原点。
    pub vertical_origin: VerticalOrigin,
    /// 水平对齐原点。
    pub horizontal_origin: HorizontalOrigin,
    /// 像素偏移 [x, y]。
    pub pixel_offset: [f64; 2],
    /// 眼睛偏移 [x, y, z]。
    pub eye_offset: [f64; 3],
    /// 缩放因子。
    pub scale: f64,
    /// 按距离缩放。
    pub scale_by_distance: Option<NearFarScalar>,
    /// 按距离半透明。
    pub translucency_by_distance: Option<NearFarScalar>,
    /// 距离显示条件。
    pub distance_display_condition: Option<DistanceDisplayCondition>,
    /// 用户自定义 ID。
    pub id: Option<String>,
}

impl Default for Label {
    /// 构造一个基线左对齐、字体 30px sans-serif、白字黑边的默认 label。
    fn default() -> Self {
        Self {
            show: true,
            position: [0.0; 3],
            text: String::new(),
            font: "30px sans-serif".to_string(),
            fill_color: Color::WHITE,
            outline_color: Color::BLACK,
            outline_width: 1.0,
            style: LabelStyle::Fill,
            show_background: false,
            background_color: Color::new(0.165, 0.165, 0.165, 0.8),
            background_padding: [7.0, 5.0],
            vertical_origin: VerticalOrigin::Baseline,
            horizontal_origin: HorizontalOrigin::Left,
            pixel_offset: [0.0; 2],
            eye_offset: [0.0; 3],
            scale: 1.0,
            scale_by_distance: None,
            translucency_by_distance: None,
            distance_display_condition: None,
            id: None,
        }
    }
}

/// label 的集合。
///
/// 顺序持有一组 label，并统一控制整体可见性。
#[derive(Debug, Default)]
pub struct LabelCollection {
    /// 集合内的 label 列表。
    labels: Vec<Label>,
    /// 整个集合是否可见。
    show: bool,
}

impl LabelCollection {
    /// 创建新的空集合。
    pub fn new() -> Self {
        Self { labels: Vec::new(), show: true }
    }

    /// 向集合添加一个 label。
    pub fn add(&mut self, label: Label) -> usize {
        let index = self.labels.len();
        self.labels.push(label);
        index
    }

    /// 按索引移除一个 label。
    pub fn remove(&mut self, index: usize) -> Option<Label> {
        if index < self.labels.len() {
            Some(self.labels.remove(index))
        } else {
            None
        }
    }

    /// 按索引获取一个 label。
    pub fn get(&self, index: usize) -> Option<&Label> {
        self.labels.get(index)
    }

    /// 按索引获取一个可变 label。
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Label> {
        self.labels.get_mut(index)
    }

    /// label 数量。
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// 若为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// 遍历 labels。
    pub fn iter(&self) -> impl Iterator<Item = &Label> {
        self.labels.iter()
    }

    /// 集合是否显示。
    pub fn show(&self) -> bool {
        self.show
    }

    /// 设置集合是否显示。
    pub fn set_show(&mut self, show: bool) {
        self.show = show;
    }

    /// 清除所有 labels。
    pub fn clear(&mut self) {
        self.labels.clear();
    }
}

/// 点图元集合中的一个点图元。
///
/// 承载单个屏幕空间点的位置、颜色与轮廓设置。
#[derive(Debug, Clone, PartialEq)]
pub struct PointPrimitive {
    /// 点是否显示。
    pub show: bool,
    /// Cartesian3 位置 [x, y, z]。
    pub position: [f64; 3],
    /// 点颜色。
    pub color: Color,
    /// 轮廓颜色。
    pub outline_color: Color,
    /// 轮廓宽度（像素）。
    pub outline_width: f64,
    /// 像素尺寸。
    pub pixel_size: f64,
    /// 按距离缩放。
    pub scale_by_distance: Option<NearFarScalar>,
    /// 按距离半透明。
    pub translucency_by_distance: Option<NearFarScalar>,
    /// 距离显示条件。
    pub distance_display_condition: Option<DistanceDisplayCondition>,
    /// 用户自定义 ID。
    pub id: Option<String>,
}

impl Default for PointPrimitive {
    /// 构造一个像素尺寸 10、白色、无轮廓的默认可见点图元。
    fn default() -> Self {
        Self {
            show: true,
            position: [0.0; 3],
            color: Color::WHITE,
            outline_color: Color::TRANSPARENT,
            outline_width: 0.0,
            pixel_size: 10.0,
            scale_by_distance: None,
            translucency_by_distance: None,
            distance_display_condition: None,
            id: None,
        }
    }
}

/// 点图元的集合。
///
/// 顺序持有一组点图元，并统一控制整体可见性。
#[derive(Debug, Default)]
pub struct PointPrimitiveCollection {
    /// 集合内的点图元列表。
    points: Vec<PointPrimitive>,
    /// 整个集合是否可见。
    show: bool,
}

impl PointPrimitiveCollection {
    /// 创建新的空集合。
    pub fn new() -> Self {
        Self { points: Vec::new(), show: true }
    }

    /// 向集合添加一个点。
    pub fn add(&mut self, point: PointPrimitive) -> usize {
        let index = self.points.len();
        self.points.push(point);
        index
    }

    /// 按索引移除一个点。
    pub fn remove(&mut self, index: usize) -> Option<PointPrimitive> {
        if index < self.points.len() {
            Some(self.points.remove(index))
        } else {
            None
        }
    }

    /// 按索引获取一个点。
    pub fn get(&self, index: usize) -> Option<&PointPrimitive> {
        self.points.get(index)
    }

    /// 按索引获取一个可变点。
    pub fn get_mut(&mut self, index: usize) -> Option<&mut PointPrimitive> {
        self.points.get_mut(index)
    }

    /// 点数量。
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// 若为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// 遍历 points。
    pub fn iter(&self) -> impl Iterator<Item = &PointPrimitive> {
        self.points.iter()
    }

    /// 集合是否显示。
    pub fn show(&self) -> bool {
        self.show
    }

    /// 设置集合是否显示。
    pub fn set_show(&mut self, show: bool) {
        self.show = show;
    }

    /// 清除所有 points。
    pub fn clear(&mut self) {
        self.points.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_billboard_collection() {
        let mut collection = BillboardCollection::new();
        assert!(collection.is_empty());

        let bb = Billboard {
            position: [1.0, 2.0, 3.0],
            scale: 2.0,
            color: Color::RED,
            image: Some("marker.png".to_string()),
            ..Default::default()
        };
        let idx = collection.add(bb);
        assert_eq!(idx, 0);
        assert_eq!(collection.len(), 1);

        let retrieved = collection.get(0).unwrap();
        assert_eq!(retrieved.position, [1.0, 2.0, 3.0]);
        assert_eq!(retrieved.scale, 2.0);
        assert_eq!(retrieved.image, Some("marker.png".to_string()));
    }

    #[test]
    fn test_billboard_remove() {
        let mut collection = BillboardCollection::new();
        collection.add(Billboard::default());
        collection.add(Billboard { scale: 3.0, ..Default::default() });
        assert_eq!(collection.len(), 2);

        let removed = collection.remove(0).unwrap();
        assert_eq!(removed.scale, 1.0);
        assert_eq!(collection.len(), 1);
        assert_eq!(collection.get(0).unwrap().scale, 3.0);
    }

    #[test]
    fn test_label_collection() {
        let mut collection = LabelCollection::new();
        let label = Label {
            text: "Hello World".to_string(),
            position: [100.0, 200.0, 300.0],
            fill_color: Color::YELLOW,
            font: "16px monospace".to_string(),
            ..Default::default()
        };
        collection.add(label);
        assert_eq!(collection.len(), 1);

        let l = collection.get(0).unwrap();
        assert_eq!(l.text, "Hello World");
        assert_eq!(l.font, "16px monospace");
    }

    #[test]
    fn test_point_primitive_collection() {
        let mut collection = PointPrimitiveCollection::new();
        let point = PointPrimitive {
            position: [10.0, 20.0, 30.0],
            color: Color::GREEN,
            pixel_size: 15.0,
            outline_width: 2.0,
            outline_color: Color::BLACK,
            ..Default::default()
        };
        collection.add(point);
        assert_eq!(collection.len(), 1);

        let p = collection.get(0).unwrap();
        assert_eq!(p.pixel_size, 15.0);
        assert_eq!(p.color, Color::GREEN);
    }

    #[test]
    fn test_near_far_scalar() {
        let nfs = NearFarScalar::new(100.0, 1.0, 1000.0, 0.5);
        assert!((nfs.value_at_distance(50.0) - 1.0).abs() < 1e-10);
        assert!((nfs.value_at_distance(100.0) - 1.0).abs() < 1e-10);
        assert!((nfs.value_at_distance(550.0) - 0.75).abs() < 1e-10);
        assert!((nfs.value_at_distance(1000.0) - 0.5).abs() < 1e-10);
        assert!((nfs.value_at_distance(2000.0) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_distance_display_condition() {
        let ddc = DistanceDisplayCondition::new(100.0, 10000.0);
        assert!(!ddc.is_visible(50.0));
        assert!(ddc.is_visible(100.0));
        assert!(ddc.is_visible(5000.0));
        assert!(ddc.is_visible(10000.0));
        assert!(!ddc.is_visible(20000.0));
    }

    #[test]
    fn test_collection_show() {
        let mut collection = BillboardCollection::new();
        assert!(collection.show());
        collection.set_show(false);
        assert!(!collection.show());
    }

    #[test]
    fn test_label_style_default() {
        assert_eq!(LabelStyle::default(), LabelStyle::Fill);
        assert_eq!(VerticalOrigin::default(), VerticalOrigin::Center);
        assert_eq!(HorizontalOrigin::default(), HorizontalOrigin::Center);
    }

    #[test]
    fn test_point_collection_clear() {
        let mut collection = PointPrimitiveCollection::new();
        collection.add(PointPrimitive::default());
        collection.add(PointPrimitive::default());
        assert_eq!(collection.len(), 2);
        collection.clear();
        assert!(collection.is_empty());
    }
}
