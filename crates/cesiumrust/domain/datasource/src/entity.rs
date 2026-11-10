//! Entity 定义与图形属性。
//!
//! 本模块定义实体 `Entity` 及其各类图形属性：点、折线、多边形、
//! billboard、标注、模型、椭圆、方框、圆柱、走廊、矩形、墙等。

use crate::property::{BoolProperty, Color, ColorProperty, NumberProperty, PositionProperty, Property, StringProperty};

/// 相对于地形定位的高度参考。
///
/// 决定图形高度是绝对值还是相对地形/3D Tiles 表面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeightReference {
    /// 位置为绝对值（不做地形调整）。
    #[default]
    None,
    /// 位置被钳制到地形表面。
    ClampToGround,
    /// 位置高度相对于地形表面。
    RelativeToGround,
    /// 位置被钳制到最详细的 3D Tiles 表面。
    ClampToTileset,
    /// 位置高度相对于最详细的 3D Tiles 表面。
    RelativeToTileset,
}

/// corridor 与 polyline volume 的角部样式。
///
/// 控制走廊与立体线在拐角处的连接样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CornerType {
    /// 圆角。
    #[default]
    Rounded,
    /// 斜接（尖锐）角。
    Mitered,
    /// 倒角（切角）。
    Beveled,
}

/// 地面图元的分类类型。
///
/// 指明地面图元绘制在地形还是 3D Tiles 之上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClassificationType {
    /// 同时分类地形与 3D Tiles。
    #[default]
    Both,
    /// 仅分类地形。
    Terrain,
    /// 仅分类 3D Tiles。
    Cesium3DTile,
}

/// entity 的阴影模式。
///
/// 决定实体是否投射与接收阴影。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadowMode {
    /// 禁用阴影。
    #[default]
    Disabled,
    /// 仅投射阴影。
    CastOnly,
    /// 仅接收阴影。
    ReceiveOnly,
    /// 投射并接收阴影。
    Enabled,
}

/// 由法线与到原点距离定义的平面。
///
/// 以单位法线与带符号距离描述一个无限平面。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneDef {
    /// 平面法线 [x, y, z]。
    pub normal: [f64; 3],
    /// 到原点的带符号距离。
    pub distance: f64,
}

/// 点图形属性。
///
/// 描述一个屏幕空间点标记的外观与可见性。
#[derive(Debug, Clone, PartialEq)]
pub struct PointGraphics {
    /// 点颜色。
    pub color: ColorProperty,
    /// 点的像素尺寸。
    pub pixel_size: NumberProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度（像素）。
    pub outline_width: NumberProperty,
    /// 点是否显示。
    pub show: BoolProperty,
}

impl Default for PointGraphics {
    /// 构造各字段取常用缺省值的点图形（白色不透明、尺寸 1、无轮廓）。
    fn default() -> Self {
        Self {
            color: Property::Constant(Color::WHITE),
            pixel_size: Property::Constant(1.0),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(0.0),
            show: Property::Constant(true),
        }
    }
}

/// 折线图形属性。
///
/// 描述一条分段折线的样式与是否贴地。
#[derive(Debug, Clone, PartialEq)]
pub struct PolylineGraphics {
    /// 折线位置（[lon, lat, height] 数组）。
    pub positions: Property<Vec<[f64; 3]>>,
    /// 线宽（像素）。
    pub width: NumberProperty,
    /// 线颜色。
    pub color: ColorProperty,
    /// 折线是否显示。
    pub show: BoolProperty,
    /// 是否鉗制到地面。
    pub clamp_to_ground: BoolProperty,
}

impl Default for PolylineGraphics {
    /// 构造缺省折线图形：位置未定义、线宽 1、白色显示且不贴地。
    fn default() -> Self {
        Self {
            positions: Property::Undefined,
            width: Property::Constant(1.0),
            color: Property::Constant(Color::WHITE),
            show: Property::Constant(true),
            clamp_to_ground: Property::Constant(false),
        }
    }
}

/// 多边形图形属性。
///
/// 描述一个可带孔、可挤出的多边形填充与轮廓。
#[derive(Debug, Clone, PartialEq)]
pub struct PolygonGraphics {
    /// 多边形层级位置（外环）。
    pub positions: Property<Vec<[f64; 3]>>,
    /// 孔洞（内环）。
    pub holes: Vec<Vec<[f64; 3]>>,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 多边形是否填充。
    pub fill: BoolProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 多边形高度。
    pub height: NumberProperty,
    /// 挤出高度。
    pub extruded_height: NumberProperty,
    /// 多边形是否显示。
    pub show: BoolProperty,
}

impl Default for PolygonGraphics {
    /// 构造缺省多边形图形：默认填充白色、不显示轮廓且不挤出。
    fn default() -> Self {
        Self {
            positions: Property::Undefined,
            holes: Vec::new(),
            material: Property::Constant(Color::WHITE),
            fill: Property::Constant(true),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            height: Property::Undefined,
            extruded_height: Property::Undefined,
            show: Property::Constant(true),
        }
    }
}

/// billboard 图形属性。
///
/// 描述一个始终朝向屏幕的图像标记。
#[derive(Debug, Clone, PartialEq)]
pub struct BillboardGraphics {
    /// 图像 URI。
    pub image: StringProperty,
    /// 宽度（像素）。
    pub width: NumberProperty,
    /// 高度（像素）。
    pub height: NumberProperty,
    /// 颜色染色。
    pub color: ColorProperty,
    /// 旋转（弧度）。
    pub rotation: NumberProperty,
    /// 缩放因子。
    pub scale: NumberProperty,
    /// billboard 是否显示。
    pub show: BoolProperty,
}

impl Default for BillboardGraphics {
    /// 构造缺省 billboard：图像与尺寸未定义、不染色、不旋转、显示。
    fn default() -> Self {
        Self {
            image: Property::Undefined,
            width: Property::Undefined,
            height: Property::Undefined,
            color: Property::Constant(Color::WHITE),
            rotation: Property::Constant(0.0),
            scale: Property::Constant(1.0),
            show: Property::Constant(true),
        }
    }
}

/// label 图形属性。
///
/// 描述一段随实体定位的文本标注。
#[derive(Debug, Clone, PartialEq)]
pub struct LabelGraphics {
    /// label 文本。
    pub text: StringProperty,
    /// 字体（CSS 格式）。
    pub font: StringProperty,
    /// 填充颜色。
    pub fill_color: ColorProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// label 是否显示。
    pub show: BoolProperty,
}

impl Default for LabelGraphics {
    /// 构造缺省标注：文本未定义、字体 30px sans-serif、白字黑边。
    fn default() -> Self {
        Self {
            text: Property::Undefined,
            font: Property::Constant("30px sans-serif".to_string()),
            fill_color: Property::Constant(Color::WHITE),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(2.0),
            show: Property::Constant(true),
        }
    }
}

/// model 图形属性。
///
/// 描述一个 glTF 模型实例的引用与缩放。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelGraphics {
    /// model URI（glTF/glb）。
    pub uri: StringProperty,
    /// 缩放因子。
    pub scale: NumberProperty,
    /// 最小像素尺寸。
    pub minimum_pixel_size: NumberProperty,
    /// model 是否显示。
    pub show: BoolProperty,
}

impl Default for ModelGraphics {
    /// 构造缺省模型：URI 未定义、缩放 1、最小像素尺寸 0、显示。
    fn default() -> Self {
        Self {
            uri: Property::Undefined,
            scale: Property::Constant(1.0),
            minimum_pixel_size: Property::Constant(0.0),
            show: Property::Constant(true),
        }
    }
}

/// 椭圆图形属性。
///
/// 描述一个椭圆的轴长、旋转与填充轮廓。
#[derive(Debug, Clone, PartialEq)]
pub struct EllipseGraphics {
    /// 半长轴（米）。
    pub semi_major_axis: NumberProperty,
    /// 半短轴（米）。
    pub semi_minor_axis: NumberProperty,
    /// 旋转（弧度）。
    pub rotation: NumberProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 高度（米）。
    pub height: NumberProperty,
    /// 挤出高度。
    pub extruded_height: NumberProperty,
    /// 椭圆是否填充。
    pub fill: BoolProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 椭圆是否显示。
    pub show: BoolProperty,
}

impl Default for EllipseGraphics {
    /// 构造缺省椭圆：轴长未定义、不旋转、填充白色且不显示轮廓。
    fn default() -> Self {
        Self {
            semi_major_axis: Property::Undefined,
            semi_minor_axis: Property::Undefined,
            rotation: Property::Constant(0.0),
            material: Property::Constant(Color::WHITE),
            height: Property::Constant(0.0),
            extruded_height: Property::Undefined,
            fill: Property::Constant(true),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            show: Property::Constant(true),
        }
    }
}

/// 方框图形属性。
///
/// 描述一个以中心对齐的长方体尺寸与外观。
#[derive(Debug, Clone, PartialEq)]
pub struct BoxGraphics {
    /// 方框尺寸 [width, depth, height]（米）。
    pub dimensions: Property<[f64; 3]>,
    /// 高度参考。
    pub height_reference: HeightReference,
    /// 方框是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 方框是否显示。
    pub show: BoolProperty,
}

impl Default for BoxGraphics {
    /// 构造缺省方框：尺寸未定义、填充白色、不显示轮廓且无阴影。
    fn default() -> Self {
        Self {
            dimensions: Property::Undefined,
            height_reference: HeightReference::None,
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            show: Property::Constant(true),
        }
    }
}

/// 圆柱图形属性。
///
/// 描述一个圆柱/圆锥的长度与两端半径。
#[derive(Debug, Clone, PartialEq)]
pub struct CylinderGraphics {
    /// 长度（高度）（米）。
    pub length: NumberProperty,
    /// 顶部半径（米）。
    pub top_radius: NumberProperty,
    /// 底部半径（米）。
    pub bottom_radius: NumberProperty,
    /// 高度参考。
    pub height_reference: HeightReference,
    /// 轮廓的垂直线数量。
    pub number_of_vertical_lines: NumberProperty,
    /// 切片数（径向分段）。
    pub slices: NumberProperty,
    /// 圆柱是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 圆柱是否显示。
    pub show: BoolProperty,
}

impl Default for CylinderGraphics {
    /// 构造缺省圆柱：尺寸未定义，垂线 16、切片 128，填充白色无轮廓。
    fn default() -> Self {
        Self {
            length: Property::Undefined,
            top_radius: Property::Undefined,
            bottom_radius: Property::Undefined,
            height_reference: HeightReference::None,
            number_of_vertical_lines: Property::Constant(16.0),
            slices: Property::Constant(128.0),
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            show: Property::Constant(true),
        }
    }
}

/// corridor（走廊）图形属性。
///
/// 描述一条沿测地线铺设、具宽度的道路带。
#[derive(Debug, Clone, PartialEq)]
pub struct CorridorGraphics {
    /// corridor 中心线位置（[lon, lat, height] 数组）。
    pub positions: Property<Vec<[f64; 3]>>,
    /// corridor 宽度（米）。
    pub width: NumberProperty,
    /// corridor 高度。
    pub height: NumberProperty,
    /// 高度参考。
    pub height_reference: HeightReference,
    /// 挤出高度。
    pub extruded_height: NumberProperty,
    /// 角部类型。
    pub corner_type: CornerType,
    /// 角度细分粒度（弧度）。
    pub granularity: NumberProperty,
    /// corridor 是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 分类类型。
    pub classification_type: ClassificationType,
    /// 地面 corridor 排序用的 z-index。
    pub z_index: NumberProperty,
    /// corridor 是否显示。
    pub show: BoolProperty,
}

impl Default for CorridorGraphics {
    /// 构造缺省走廊：中心线与宽度未定义、圆角、填充白色且分类到两者。
    fn default() -> Self {
        Self {
            positions: Property::Undefined,
            width: Property::Undefined,
            height: Property::Constant(0.0),
            height_reference: HeightReference::None,
            extruded_height: Property::Undefined,
            corner_type: CornerType::Rounded,
            granularity: Property::Undefined,
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            classification_type: ClassificationType::Both,
            z_index: Property::Constant(0.0),
            show: Property::Constant(true),
        }
    }
}

/// 矩形图形属性。
///
/// 描述一个经纬矩形面的填充与轮廓。
#[derive(Debug, Clone, PartialEq)]
pub struct RectangleGraphics {
    /// 矩形坐标 [west, south, east, north]（弧度）。
    pub coordinates: Property<[f64; 4]>,
    /// 高度（米）。
    pub height: NumberProperty,
    /// 高度参考。
    pub height_reference: HeightReference,
    /// 挤出高度。
    pub extruded_height: NumberProperty,
    /// 矩形的旋转（弧度）。
    pub rotation: NumberProperty,
    /// 纹理坐标旋转（弧度）。
    pub st_rotation: NumberProperty,
    /// 角度细分粒度（弧度）。
    pub granularity: NumberProperty,
    /// 矩形是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 分类类型。
    pub classification_type: ClassificationType,
    /// 地面矩形排序用的 z-index。
    pub z_index: NumberProperty,
    /// 矩形是否显示。
    pub show: BoolProperty,
}

impl Default for RectangleGraphics {
    /// 构造缺省矩形：坐标未定义、高度 0、不旋转、填充白色且分类到两者。
    fn default() -> Self {
        Self {
            coordinates: Property::Undefined,
            height: Property::Constant(0.0),
            height_reference: HeightReference::None,
            extruded_height: Property::Undefined,
            rotation: Property::Constant(0.0),
            st_rotation: Property::Constant(0.0),
            granularity: Property::Undefined,
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            classification_type: ClassificationType::Both,
            z_index: Property::Constant(0.0),
            show: Property::Constant(true),
        }
    }
}

/// 墙（wall）图形属性。
///
/// 描述一面沿折线垂直拉伸的墙。
#[derive(Debug, Clone, PartialEq)]
pub struct WallGraphics {
    /// 墙位置（[lon, lat, height] 数组）。
    pub positions: Property<Vec<[f64; 3]>>,
    /// 每个位置的最低高度。
    pub minimum_heights: Property<Vec<f64>>,
    /// 每个位置的最高高度。
    pub maximum_heights: Property<Vec<f64>>,
    /// 角度细分粒度（弧度）。
    pub granularity: NumberProperty,
    /// 墙是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 墙是否显示。
    pub show: BoolProperty,
}

impl Default for WallGraphics {
    /// 构造缺省墙：位置与高低度未定义，填充白色、不显示轮廓。
    fn default() -> Self {
        Self {
            positions: Property::Undefined,
            minimum_heights: Property::Undefined,
            maximum_heights: Property::Undefined,
            granularity: Property::Undefined,
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            show: Property::Constant(true),
        }
    }
}

/// 球体图形属性。
///
/// 描述一个椭球体的三轴半径与外观。
#[derive(Debug, Clone, PartialEq)]
pub struct EllipsoidGraphics {
    /// 外半径 [x, y, z]（米）。
    pub radii: Property<[f64; 3]>,
    /// 空心球体的内半径。
    pub inner_radii: Property<[f64; 3]>,
    /// 最小时钟角（弧度）。
    pub minimum_clock: NumberProperty,
    /// 最大时钟角（弧度）。
    pub maximum_clock: NumberProperty,
    /// 最小锥角（弧度）。
    pub minimum_cone: NumberProperty,
    /// 最大锥角（弧度）。
    pub maximum_cone: NumberProperty,
    /// 高度参考。
    pub height_reference: HeightReference,
    /// 径向切片数。
    pub slices: NumberProperty,
    /// 纵向分区数。
    pub stack_partitions: NumberProperty,
    /// 切片分区数。
    pub slice_partitions: NumberProperty,
    /// 细分数量。
    pub subdivisions: NumberProperty,
    /// 球体是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 球体是否显示。
    pub show: BoolProperty,
}

impl Default for EllipsoidGraphics {
    /// 构造缺省椭球：半径未定义，覆盖完整时钟与锥角范围，填充白色。
    fn default() -> Self {
        Self {
            radii: Property::Undefined,
            inner_radii: Property::Undefined,
            minimum_clock: Property::Constant(0.0),
            maximum_clock: Property::Constant(std::f64::consts::TAU),
            minimum_cone: Property::Constant(0.0),
            maximum_cone: Property::Constant(std::f64::consts::PI),
            height_reference: HeightReference::None,
            slices: Property::Constant(128.0),
            stack_partitions: Property::Constant(64.0),
            slice_partitions: Property::Constant(64.0),
            subdivisions: Property::Constant(128.0),
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            show: Property::Constant(true),
        }
    }
}

/// 平面图形属性。
///
/// 描述一个平面的法线、距离与裁剪范围。
#[derive(Debug, Clone, PartialEq)]
pub struct PlaneGraphics {
    /// 平面定义（法线 + 距离）。
    pub plane: Property<PlaneDef>,
    /// 尺寸 [width, height]（米）。
    pub dimensions: Property<[f64; 2]>,
    /// 平面是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 平面是否显示。
    pub show: BoolProperty,
}

impl Default for PlaneGraphics {
    /// 构造缺省平面：平面与尺寸未定义，填充白色、不显示轮廓。
    fn default() -> Self {
        Self {
            plane: Property::Undefined,
            dimensions: Property::Undefined,
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            show: Property::Constant(true),
        }
    }
}

/// path（轨迹）图形属性（轨迹可视化）。
///
/// 描述实体随时间轨迹所形成的尾线。
#[derive(Debug, Clone, PartialEq)]
pub struct PathGraphics {
    /// 引导时间（秒）（向前显示多远）。
    pub lead_time: NumberProperty,
    /// 拖尾时间（秒）（向后显示多远）。
    pub trail_time: NumberProperty,
    /// path 宽度（像素）。
    pub width: NumberProperty,
    /// 采样分辨率（秒）。
    pub resolution: NumberProperty,
    /// path 材质颜色。
    pub material: ColorProperty,
    /// path 是否显示。
    pub show: BoolProperty,
}

impl Default for PathGraphics {
    /// 构造缺省轨迹：引导/拖尾时间未定义，宽 1、采样 60 秒、白色显示。
    fn default() -> Self {
        Self {
            lead_time: Property::Undefined,
            trail_time: Property::Undefined,
            width: Property::Constant(1.0),
            resolution: Property::Constant(60.0),
            material: Property::Constant(Color::WHITE),
            show: Property::Constant(true),
        }
    }
}

/// polyline volume 图形属性。
///
/// 描述一个沿路径扫掠给定截面所形成的立体。
#[derive(Debug, Clone, PartialEq)]
pub struct PolylineVolumeGraphics {
    /// 体中心线位置（[lon, lat, height] 数组）。
    pub positions: Property<Vec<[f64; 3]>>,
    /// 2D 横截面形状（[x, y] 数组，米）。
    pub shape: Property<Vec<[f64; 2]>>,
    /// 角部类型。
    pub corner_type: CornerType,
    /// 角度细分粒度（弧度）。
    pub granularity: NumberProperty,
    /// 体是否填充。
    pub fill: BoolProperty,
    /// 填充材质颜色。
    pub material: ColorProperty,
    /// 是否显示轮廓。
    pub outline: BoolProperty,
    /// 轮廓颜色。
    pub outline_color: ColorProperty,
    /// 轮廓宽度。
    pub outline_width: NumberProperty,
    /// 阴影模式。
    pub shadows: ShadowMode,
    /// 体是否显示。
    pub show: BoolProperty,
}

impl Default for PolylineVolumeGraphics {
    /// 构造缺省立体线：中心线与截面未定义、圆角，填充白色且不显轮廓。
    fn default() -> Self {
        Self {
            positions: Property::Undefined,
            shape: Property::Undefined,
            corner_type: CornerType::Rounded,
            granularity: Property::Undefined,
            fill: Property::Constant(true),
            material: Property::Constant(Color::WHITE),
            outline: Property::Constant(false),
            outline_color: Property::Constant(Color::BLACK),
            outline_width: Property::Constant(1.0),
            shadows: ShadowMode::Disabled,
            show: Property::Constant(true),
        }
    }
}

/// 数据源中的一个 entity。
///
/// 聚合位置、朝向与各图形字段，共同描述一个可时变的数据源实体。
#[derive(Debug, Clone)]
pub struct Entity {
    /// 唯一标识符。
    pub id: String,

    /// 可读名称。
    pub name: Option<String>,

    /// entity 是否显示。
    pub show: bool,

    /// entity 描述（HTML）。
    pub description: Option<String>,

    /// 位置属性 [longitude_rad, latitude_rad, height_m]。
    pub position: PositionProperty,

    /// 朝向（四元数 [x, y, z, w]）。
    pub orientation: Property<[f64; 4]>,

    /// 点图形。
    pub point: Option<PointGraphics>,

    /// 折线图形。
    pub polyline: Option<PolylineGraphics>,

    /// 多边形图形。
    pub polygon: Option<PolygonGraphics>,

    /// billboard 图形。
    pub billboard: Option<BillboardGraphics>,

    /// label 图形。
    pub label: Option<LabelGraphics>,

    /// model 图形。
    pub model: Option<ModelGraphics>,

    /// 椭圆图形。
    pub ellipse: Option<EllipseGraphics>,

    /// 方框图形。
    pub box_graphics: Option<BoxGraphics>,

    /// 圆柱图形。
    pub cylinder: Option<CylinderGraphics>,

    /// corridor 图形。
    pub corridor: Option<CorridorGraphics>,

    /// 矩形图形。
    pub rectangle: Option<RectangleGraphics>,

    /// 墙图形。
    pub wall: Option<WallGraphics>,

    /// 球体图形。
    pub ellipsoid: Option<EllipsoidGraphics>,

    /// 平面图形。
    pub plane: Option<PlaneGraphics>,

    /// path 图形。
    pub path: Option<PathGraphics>,

    /// polyline volume 图形。
    pub polyline_volume: Option<PolylineVolumeGraphics>,

    /// 父 entity ID。
    pub parent: Option<String>,

    /// 可用性区间集合。
    pub availability: Option<cesium_time::TimeIntervalCollection<()>>,

    /// 自定义属性（键值元数据）。
    pub properties: std::collections::HashMap<String, serde_json::Value>,
}

impl Entity {
    /// 创建一个带有给定 ID 的新 entity。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            show: true,
            description: None,
            position: Property::Undefined,
            orientation: Property::Undefined,
            point: None,
            polyline: None,
            polygon: None,
            billboard: None,
            label: None,
            model: None,
            ellipse: None,
            box_graphics: None,
            cylinder: None,
            corridor: None,
            rectangle: None,
            wall: None,
            ellipsoid: None,
            plane: None,
            path: None,
            polyline_volume: None,
            parent: None,
            availability: None,
            properties: std::collections::HashMap::new(),
        }
    }

    /// 设置名称。
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// 将位置设为常量 [lon_rad, lat_rad, height_m]。
    pub fn with_position(mut self, lon: f64, lat: f64, height: f64) -> Self {
        self.position = Property::Constant([lon, lat, height]);
        self
    }

    /// 设置点图形。
    pub fn with_point(mut self, point: PointGraphics) -> Self {
        self.point = Some(point);
        self
    }

    /// 设置折线图形。
    pub fn with_polyline(mut self, polyline: PolylineGraphics) -> Self {
        self.polyline = Some(polyline);
        self
    }

    /// 设置多边形图形。
    pub fn with_polygon(mut self, polygon: PolygonGraphics) -> Self {
        self.polygon = Some(polygon);
        self
    }

    /// 设置 billboard 图形。
    pub fn with_billboard(mut self, billboard: BillboardGraphics) -> Self {
        self.billboard = Some(billboard);
        self
    }

    /// 设置 label 图形。
    pub fn with_label(mut self, label: LabelGraphics) -> Self {
        self.label = Some(label);
        self
    }

    /// 设置 model 图形。
    pub fn with_model(mut self, model: ModelGraphics) -> Self {
        self.model = Some(model);
        self
    }

    /// 设置方框图形。
    pub fn with_box(mut self, box_graphics: BoxGraphics) -> Self {
        self.box_graphics = Some(box_graphics);
        self
    }

    /// 设置圆柱图形。
    pub fn with_cylinder(mut self, cylinder: CylinderGraphics) -> Self {
        self.cylinder = Some(cylinder);
        self
    }

    /// 设置 corridor 图形。
    pub fn with_corridor(mut self, corridor: CorridorGraphics) -> Self {
        self.corridor = Some(corridor);
        self
    }

    /// 设置矩形图形。
    pub fn with_rectangle(mut self, rectangle: RectangleGraphics) -> Self {
        self.rectangle = Some(rectangle);
        self
    }

    /// 设置墙图形。
    pub fn with_wall(mut self, wall: WallGraphics) -> Self {
        self.wall = Some(wall);
        self
    }

    /// 设置球体图形。
    pub fn with_ellipsoid(mut self, ellipsoid: EllipsoidGraphics) -> Self {
        self.ellipsoid = Some(ellipsoid);
        self
    }

    /// 设置平面图形。
    pub fn with_plane(mut self, plane: PlaneGraphics) -> Self {
        self.plane = Some(plane);
        self
    }

    /// 设置 path 图形。
    pub fn with_path(mut self, path: PathGraphics) -> Self {
        self.path = Some(path);
        self
    }

    /// 设置 polyline volume 图形。
    pub fn with_polyline_volume(mut self, polyline_volume: PolylineVolumeGraphics) -> Self {
        self.polyline_volume = Some(polyline_volume);
        self
    }

    /// 添加一个自定义属性。
    pub fn with_property(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.properties.insert(key.into(), value);
        self
    }

    /// 若此 entity 含有任何可渲染图形则返回 true。
    pub fn has_graphics(&self) -> bool {
        self.point.is_some()
            || self.polyline.is_some()
            || self.polygon.is_some()
            || self.billboard.is_some()
            || self.label.is_some()
            || self.model.is_some()
            || self.ellipse.is_some()
            || self.box_graphics.is_some()
            || self.cylinder.is_some()
            || self.corridor.is_some()
            || self.rectangle.is_some()
            || self.wall.is_some()
            || self.ellipsoid.is_some()
            || self.plane.is_some()
            || self.path.is_some()
            || self.polyline_volume.is_some()
    }

    /// 若在给定时间该 entity 可用则返回 true。
    /// 若未定义可用性，则始终返回 true。
    ///
    /// CesiumJS: `entity.isAvailable(time)`
    pub fn is_available(&self, time: &cesium_time::JulianDate) -> bool {
        match &self.availability {
            None => true,
            Some(tic) => tic.contains(time),
        }
    }

    /// 为该 entity 添加一个自定义属性。
    ///
    /// CesiumJS: `entity.addProperty(name, value)`
    pub fn add_property(&mut self, name: impl Into<String>, value: serde_json::Value) {
        self.properties.insert(name.into(), value);
    }

    /// 从该 entity 移除一个自定义属性。
    ///
    /// CesiumJS: `entity.removeProperty(name)`
    pub fn remove_property(&mut self, name: &str) -> Option<serde_json::Value> {
        self.properties.remove(name)
    }

    /// 将来自源 entity 的属性合并到本 entity。
    /// 保留的属性名（id、name、show 等）不会被覆盖。
    /// 自定义属性会被合并。
    ///
    /// CesiumJS: `entity.merge(source)`
    pub fn merge(&mut self, source: &Entity) {
        // 仅当未设置时合并 name
        if self.name.is_none() {
            self.name = source.name.clone();
        }
        // 仅当未设置时合并 description
        if self.description.is_none() {
            self.description = source.description.clone();
        }
        // 仅当为 Undefined 时合并 position
        if matches!(self.position, Property::Undefined) {
            self.position = source.position.clone();
        }
        // 仅当为 Undefined 时合并 orientation
        if matches!(self.orientation, Property::Undefined) {
            self.orientation = source.orientation.clone();
        }
        // 仅当未设置时合并图形
        if self.point.is_none() { self.point = source.point.clone(); }
        if self.polyline.is_none() { self.polyline = source.polyline.clone(); }
        if self.polygon.is_none() { self.polygon = source.polygon.clone(); }
        if self.billboard.is_none() { self.billboard = source.billboard.clone(); }
        if self.label.is_none() { self.label = source.label.clone(); }
        if self.model.is_none() { self.model = source.model.clone(); }
        if self.box_graphics.is_none() { self.box_graphics = source.box_graphics.clone(); }
        if self.cylinder.is_none() { self.cylinder = source.cylinder.clone(); }
        if self.corridor.is_none() { self.corridor = source.corridor.clone(); }
        if self.rectangle.is_none() { self.rectangle = source.rectangle.clone(); }
        if self.wall.is_none() { self.wall = source.wall.clone(); }
        if self.ellipsoid.is_none() { self.ellipsoid = source.ellipsoid.clone(); }
        if self.plane.is_none() { self.plane = source.plane.clone(); }
        if self.path.is_none() { self.path = source.path.clone(); }
        if self.polyline_volume.is_none() { self.polyline_volume = source.polyline_volume.clone(); }
        // 仅当未设置时合并 parent
        if self.parent.is_none() {
            self.parent = source.parent.clone();
        }
        // 合并自定义属性（由源填充缺失的键）
        for (key, value) in &source.properties {
            self.properties.entry(key.clone()).or_insert_with(|| value.clone());
        }
        // 注意：availability 不会被 merge 覆盖（CesiumJS 行为）
    }

    /// 计算在给定时间该 entity 的模型矩阵。
    /// 若 position 未定义则返回 None。
    /// 若定义了 orientation 则使用它；否则计算 ENU 坐标系。
    ///
    /// CesiumJS: `entity.computeModelMatrix(time, result)`
    pub fn compute_model_matrix(
        &self,
        time: f64,
        ellipsoid: &cesium_geospatial::Ellipsoid,
    ) -> Option<glam::DMat4> {
        // 获取位置
        let pos_arr = self.position.get_value(time)?;
        let cartesian = ellipsoid.cartographic_to_cartesian(
            &cesium_geospatial::Cartographic::from_radians(pos_arr[0], pos_arr[1], pos_arr[2]),
        );

        // 获取旋转
        let rotation = if let Some(orient_arr) = self.orientation.get_value(time) {
            // orientation 为 [x, y, z, w] 四元数
            let quat = glam::DQuat::from_xyzw(orient_arr[0], orient_arr[1], orient_arr[2], orient_arr[3]);
            glam::DMat3::from_quat(quat)
        } else {
            // 计算 ENU 坐标系
            let enu = cesium_geospatial::transforms::east_north_up_to_fixed_frame(cartesian, ellipsoid);
            // 提取旋转（左上 3x3）
            glam::DMat3::from_cols(
                enu.col(0).truncate(),
                enu.col(1).truncate(),
                enu.col(2).truncate(),
            )
        };

        // 构建 4x4 模型矩阵
        let mut result = glam::DMat4::from_mat3(rotation);
        result.w_axis = glam::DVec4::new(cartesian.x, cartesian.y, cartesian.z, 1.0);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_creation() {
        let entity = Entity::new("test-1").with_name("Test Entity");
        assert_eq!(entity.id, "test-1");
        assert_eq!(entity.name, Some("Test Entity".to_string()));
        assert!(entity.show);
        assert!(!entity.has_graphics());
    }

    #[test]
    fn test_entity_with_point() {
        let entity = Entity::new("point-1")
            .with_position(0.1, 0.2, 100.0)
            .with_point(PointGraphics {
                color: Property::Constant(Color::RED),
                pixel_size: Property::Constant(10.0),
                ..Default::default()
            });

        assert!(entity.has_graphics());
        let point = entity.point.unwrap();
        let color = point.color.get_value(0.0).unwrap();
        assert!((color.red - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_entity_with_polyline() {
        let entity = Entity::new("line-1").with_polyline(PolylineGraphics {
            positions: Property::Constant(vec![
                [0.0, 0.0, 0.0],
                [0.1, 0.1, 0.0],
                [0.2, 0.0, 0.0],
            ]),
            width: Property::Constant(3.0),
            color: Property::Constant(Color::BLUE),
            ..Default::default()
        });

        assert!(entity.has_graphics());
        let polyline = entity.polyline.unwrap();
        let positions = polyline.positions.get_value(0.0).unwrap();
        assert_eq!(positions.len(), 3);
    }

    #[test]
    fn test_entity_with_polygon() {
        let entity = Entity::new("poly-1").with_polygon(PolygonGraphics {
            positions: Property::Constant(vec![
                [0.0, 0.0, 0.0],
                [0.1, 0.0, 0.0],
                [0.1, 0.1, 0.0],
                [0.0, 0.1, 0.0],
            ]),
            material: Property::Constant(Color::new(1.0, 0.0, 0.0, 0.5)),
            ..Default::default()
        });

        assert!(entity.has_graphics());
        let polygon = entity.polygon.unwrap();
        let material = polygon.material.get_value(0.0).unwrap();
        assert!((material.alpha - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_entity_custom_properties() {
        let entity = Entity::new("prop-1")
            .with_property("population", serde_json::json!(1000000))
            .with_property("name", serde_json::json!("City"));

        assert_eq!(entity.properties.len(), 2);
        assert_eq!(entity.properties["population"], serde_json::json!(1000000));
    }

    #[test]
    fn test_entity_with_box() {
        let entity = Entity::new("box-1").with_box(BoxGraphics {
            dimensions: Property::Constant([100.0, 200.0, 300.0]),
            material: Property::Constant(Color::RED),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let bx = entity.box_graphics.unwrap();
        let dims = bx.dimensions.get_value(0.0).unwrap();
        assert_eq!(*dims, [100.0, 200.0, 300.0]);
    }

    #[test]
    fn test_entity_with_cylinder() {
        let entity = Entity::new("cyl-1").with_cylinder(CylinderGraphics {
            length: Property::Constant(500.0),
            top_radius: Property::Constant(50.0),
            bottom_radius: Property::Constant(100.0),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let cyl = entity.cylinder.unwrap();
        assert_eq!(*cyl.length.get_value(0.0).unwrap(), 500.0);
        assert_eq!(*cyl.top_radius.get_value(0.0).unwrap(), 50.0);
    }

    #[test]
    fn test_entity_with_corridor() {
        let entity = Entity::new("cor-1").with_corridor(CorridorGraphics {
            positions: Property::Constant(vec![[0.0, 0.0, 0.0], [0.1, 0.1, 0.0]]),
            width: Property::Constant(200.0),
            corner_type: CornerType::Beveled,
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let cor = entity.corridor.unwrap();
        assert_eq!(cor.corner_type, CornerType::Beveled);
    }

    #[test]
    fn test_entity_with_rectangle() {
        let entity = Entity::new("rect-1").with_rectangle(RectangleGraphics {
            coordinates: Property::Constant([-0.1, -0.1, 0.1, 0.1]),
            height: Property::Constant(1000.0),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let rect = entity.rectangle.unwrap();
        assert_eq!(*rect.height.get_value(0.0).unwrap(), 1000.0);
    }

    #[test]
    fn test_entity_with_wall() {
        let entity = Entity::new("wall-1").with_wall(WallGraphics {
            positions: Property::Constant(vec![[0.0, 0.0, 0.0], [0.1, 0.0, 0.0]]),
            maximum_heights: Property::Constant(vec![500.0, 500.0]),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let wall = entity.wall.unwrap();
        assert_eq!(wall.maximum_heights.get_value(0.0).unwrap().len(), 2);
    }

    #[test]
    fn test_entity_with_ellipsoid() {
        let entity = Entity::new("ell-1").with_ellipsoid(EllipsoidGraphics {
            radii: Property::Constant([100.0, 200.0, 300.0]),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let ell = entity.ellipsoid.unwrap();
        assert_eq!(*ell.radii.get_value(0.0).unwrap(), [100.0, 200.0, 300.0]);
    }

    #[test]
    fn test_entity_with_plane() {
        let entity = Entity::new("plane-1").with_plane(PlaneGraphics {
            plane: Property::Constant(PlaneDef { normal: [0.0, 0.0, 1.0], distance: 0.0 }),
            dimensions: Property::Constant([500.0, 500.0]),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let pl = entity.plane.unwrap();
        assert_eq!(pl.plane.get_value(0.0).unwrap().normal, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_entity_with_path() {
        let entity = Entity::new("path-1").with_path(PathGraphics {
            lead_time: Property::Constant(3600.0),
            trail_time: Property::Constant(7200.0),
            width: Property::Constant(3.0),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let path = entity.path.unwrap();
        assert_eq!(*path.lead_time.get_value(0.0).unwrap(), 3600.0);
    }

    #[test]
    fn test_entity_with_polyline_volume() {
        let entity = Entity::new("pv-1").with_polyline_volume(PolylineVolumeGraphics {
            positions: Property::Constant(vec![[0.0, 0.0, 0.0], [0.1, 0.0, 0.0]]),
            shape: Property::Constant(vec![[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0], [-50.0, 50.0]]),
            ..Default::default()
        });
        assert!(entity.has_graphics());
        let pv = entity.polyline_volume.unwrap();
        assert_eq!(pv.shape.get_value(0.0).unwrap().len(), 4);
    }

    #[test]
    fn test_height_reference_default() {
        assert_eq!(HeightReference::default(), HeightReference::None);
        assert_eq!(CornerType::default(), CornerType::Rounded);
        assert_eq!(ClassificationType::default(), ClassificationType::Both);
        assert_eq!(ShadowMode::default(), ShadowMode::Disabled);
    }
}
