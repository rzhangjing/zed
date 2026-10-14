//! 标绘元素可采用的几何变体。
//!
//! 每个变体只存储 [`GeoPoint`] / 米 —— 即地理事实源
//! （计划 §3/§5）。栅格化为顶点并投影到世界空间是在后面发生的
//! （`geom` 采样 + 渲染桥接层），绝不在此处。
//! [`Geometry::Composite`] 是为组合/军标几何预留的扩展槽位：
//! M1 仅保证基本类型被建模，树/IO/可见性会穿过 Composite
//! 往返，但目前没有任何东西采样它。

use serde::{Deserialize, Serialize};

use crate::geo::{GeoBounds, GeoPoint};

/// 一个闭合环：至少 3 个点，首 == 尾为隐含（不存两遍）。
pub type Ring = Vec<GeoPoint>;

/// 标签相对于其锚点坐标的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LabelAnchor {
    /// 中心对齐（缺省）。
    #[default]
    Center,
    /// 锚点在文本左侧。
    Left,
    /// 锚点在文本右侧。
    Right,
    /// 锚点在文本上方。
    Top,
    /// 锚点在文本下方。
    Bottom,
}

/// 一个图标引用：哪个符号 + 它被放置的锚点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconGeometry {
    /// 地理锚点。
    pub at: GeoPoint,
    /// 选择图标图像的注册表键（由桥接层解析）。
    pub key: String,
}

/// 锚定在坐标上的文本标签。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelGeometry {
    /// 地理锚点。
    pub at: GeoPoint,
    /// 要渲染的文本（在当前内置字体下为 ASCII）。
    pub text: String,
    /// 锚点对齐方式。
    pub anchor: LabelAnchor,
    /// 从投影锚点起的像素偏移（x 向右，y 向上）。
    pub offset_px: [f32; 2],
}

/// 一条由 >= 2 个顶点连接而成的开放线。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polyline {
    /// 有序顶点序列（至少 2 个）。
    pub positions: Vec<GeoPoint>,
}

/// 一个简单多边形：一个外环 + 可选的孔。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polygon {
    /// 外边界环。
    pub outer: Ring,
    /// 内部孔环集合（可为空）。
    pub holes: Vec<Ring>,
}

/// 一个经/纬度轴对齐的矩形。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rectangle {
    /// 西边界经度（度）。
    pub west: f64,
    /// 南边界纬度（度）。
    pub south: f64,
    /// 东边界经度（度）。
    pub east: f64,
    /// 北边界纬度（度）。
    pub north: f64,
}

/// 一个地面圆：中心 + 以米为单位的半径（稍后采样）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Circle {
    /// 圆心（地理点）。
    pub center: GeoPoint,
    /// 半径（米）。
    pub radius_m: f64,
}

/// 一个地面椭圆：中心、半轴（米）与方位角（度，自北向
/// 顺时针）。稍后采样。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ellipse {
    /// 椭圆中心（地理点）。
    pub center: GeoPoint,
    /// 长半轴（米）。
    pub semi_major_m: f64,
    /// 短半轴（米）。
    pub semi_minor_m: f64,
    /// 长轴方位角（度，自北顺时针）。
    pub rotation_deg: f64,
}

/// 经过三点的圆弧：起点 → （途经）中心 → 终点。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Arc3 {
    /// 弧起点。
    pub start: GeoPoint,
    /// 弧途经的中点（控制点）。
    pub center: GeoPoint,
    /// 弧终点。
    pub end: GeoPoint,
}

/// [`Path`] 的一个段（混合的基本体片段拼接在一起）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PathSegment {
    /// 一段直线（顶点环）。
    Line(Ring),
    /// 一段三点圆弧。
    Arc(Arc3),
}

/// 由混合直线 / 圆弧段构成的复合路径。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Path {
    /// 依次拼接的段序列。
    pub segments: Vec<PathSegment>,
}

/// 为组合 / 军标类型预留的分类器（M9+）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SymbolKind {
    /// 无类型的复合体 —— 仅绘制其各部分。
    #[default]
    Generic,
}

/// 由其他几何体构建的几何体（符号库的扩展槽位）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composite {
    pub kind: SymbolKind,
    pub parts: Vec<Geometry>,
}

impl Composite {
    /// 由一个分类器及其组成几何体装配出一个复合符号。这是预留的
    /// M9+ 军标集成点：符号库将一个 `SymbolKind` 解析为一个 `Vec<Geometry>`
    /// 并在此折叠它们；随后一切下游（树 / IO / 可见性 / 采样）
    /// 都把该符号视为单个几何体，其 [`vertices`](Geometry::vertices)
    /// 与包围盒是各部分的并集。
    pub fn new(kind: SymbolKind, parts: Vec<Geometry>) -> Self {
        Self { kind, parts }
    }

    /// 组成部分的数量。
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// 复合体是否不含有任何部分。
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

/// 完整的几何并集。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    Point(GeoPoint),
    Icon(IconGeometry),
    Label(LabelGeometry),
    Polyline(Polyline),
    Polygon(Polygon),
    Rectangle(Rectangle),
    Circle(Circle),
    Ellipse(Ellipse),
    Arc(Arc3),
    Path(Path),
    Composite(Composite),
}

/// 可见性过滤器的类型维度所使用的粗粒度分类（§10.5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeometryKind {
    Point,
    Icon,
    Label,
    Line,
    Polygon,
    Rectangle,
    Circle,
    Ellipse,
    Arc,
    Path,
    Composite,
}

impl GeometryKind {
    /// 每一种类型，按稳定的展示顺序 —— 类型维度过滤器面板
    /// （计划 §10.5 / §9）为每个条目提供一个开关。
    pub const ALL: [GeometryKind; 11] = [
        GeometryKind::Point,
        GeometryKind::Icon,
        GeometryKind::Label,
        GeometryKind::Line,
        GeometryKind::Polygon,
        GeometryKind::Rectangle,
        GeometryKind::Circle,
        GeometryKind::Ellipse,
        GeometryKind::Arc,
        GeometryKind::Path,
        GeometryKind::Composite,
    ];

    /// 完整列表（见 [`GeometryKind::ALL`]）。
    pub fn all() -> &'static [GeometryKind] {
        &Self::ALL
    }
}

impl Geometry {
    /// 此几何体的可见性过滤器类。
    pub fn kind(&self) -> GeometryKind {
        match self {
            Geometry::Point(_) => GeometryKind::Point,
            Geometry::Icon(_) => GeometryKind::Icon,
            Geometry::Label(_) => GeometryKind::Label,
            Geometry::Polyline(_) => GeometryKind::Line,
            Geometry::Polygon(_) => GeometryKind::Polygon,
            Geometry::Rectangle(_) => GeometryKind::Rectangle,
            Geometry::Circle(_) => GeometryKind::Circle,
            Geometry::Ellipse(_) => GeometryKind::Ellipse,
            Geometry::Arc(_) => GeometryKind::Arc,
            Geometry::Path(_) => GeometryKind::Path,
            Geometry::Composite(_) => GeometryKind::Composite,
        }
    }

    /// 存储的每个顶点，用于保守的包围盒。圆 / 椭圆
    /// 在此只贡献其中心；采样遍（`geom`，M4）
    /// 负责真实范围，因此在那之前那些类型的缓存元素包围盒
    /// 只是一个中心代理。
    pub fn vertices(&self) -> Vec<GeoPoint> {
        match self {
            Geometry::Point(p) => vec![*p],
            Geometry::Icon(i) => vec![i.at],
            Geometry::Label(l) => vec![l.at],
            Geometry::Polyline(pl) => pl.positions.clone(),
            Geometry::Polygon(pg) => {
                let mut v = pg.outer.clone();
                for h in &pg.holes {
                    v.extend_from_slice(h);
                }
                v
            }
            Geometry::Rectangle(r) => vec![
                GeoPoint::surface(r.west, r.south),
                GeoPoint::surface(r.east, r.north),
            ],
            Geometry::Circle(c) => vec![c.center],
            Geometry::Ellipse(e) => vec![e.center],
            Geometry::Arc(a) => vec![a.start, a.center, a.end],
            Geometry::Path(p) => p
                .segments
                .iter()
                .flat_map(|s| match s {
                    PathSegment::Line(r) => r.clone(),
                    PathSegment::Arc(a) => vec![a.start, a.center, a.end],
                })
                .collect(),
            Geometry::Composite(c) => c.parts.iter().flat_map(|g| g.vertices()).collect(),
        }
    }

    /// 保守的地理包围盒，可证明包含采样器（[`crate::geom::sample`]）—— 从而
    /// 也包括渲染器与拾取器经 `shapes::{stroke_positions, face_rings}` ——
    /// 为此几何体能产生的每一个顶点。它故意取*地球*（大圆加密）的
    /// 范围，这是平面范围的超集，因此同一个盒子在两种视图模式下
    /// 都是安全的宽相位剔除：
    ///  * 折线 / 多边形 + 孔边 / 矩形边 / 路径线会沿其大圆
    ///    细分，捕获一个稀疏控制点盒子会漏掉的向极凸起；
    ///  * 圆 / 椭圆 / 圆弧使用其采样环，该环已位于真正的外层
    ///    范围（进一步细分只会向内收缩）；
    ///  * 点状类型退化为其锚点（拾取器会加上像素余量）。
    ///
    /// 每次编辑计算一次（[`Element::set_geometry`](crate::model::element::Element::set_geometry)），
    /// 绝不在渲染 / 拾取热路径上计算。
    pub fn bounds(&self) -> GeoBounds {
        use crate::geom::sample::{
            arc_ring, circle_ring, ellipse_ring, rectangle_ring, subdivide_great_circle,
            DEFAULT_SEGMENTS, GREAT_CIRCLE_STEP_RAD,
        };
        match self {
            Geometry::Point(p) => GeoBounds::from_point(*p),
            Geometry::Icon(i) => GeoBounds::from_point(i.at),
            Geometry::Label(l) => GeoBounds::from_point(l.at),
            Geometry::Polyline(pl) => {
                let sub = subdivide_great_circle(&pl.positions, GREAT_CIRCLE_STEP_RAD);
                GeoBounds::from_points(&sub)
            }
            Geometry::Polygon(pg) => {
                let mut acc = GeoBounds::from_points(&subdivide_closed(&pg.outer, GREAT_CIRCLE_STEP_RAD));
                for h in &pg.holes {
                    acc = acc.union(GeoBounds::from_points(&subdivide_closed(h, GREAT_CIRCLE_STEP_RAD)));
                }
                acc
            }
            Geometry::Rectangle(r) => {
                let ring = rectangle_ring(r);
                GeoBounds::from_points(&subdivide_closed(&ring, GREAT_CIRCLE_STEP_RAD))
            }
            Geometry::Circle(c) => GeoBounds::from_points(&circle_ring(c, DEFAULT_SEGMENTS)),
            Geometry::Ellipse(e) => GeoBounds::from_points(&ellipse_ring(e, DEFAULT_SEGMENTS)),
            Geometry::Arc(a) => GeoBounds::from_points(&arc_ring(a, DEFAULT_SEGMENTS)),
            Geometry::Path(p) => {
                let mut acc = GeoBounds::empty();
                for seg in &p.segments {
                    let pts = match seg {
                        PathSegment::Line(r) => subdivide_great_circle(r, GREAT_CIRCLE_STEP_RAD),
                        PathSegment::Arc(a) => arc_ring(a, DEFAULT_SEGMENTS),
                    };
                    acc = acc.union(GeoBounds::from_points(&pts));
                }
                acc
            }
            Geometry::Composite(c) => {
                let mut acc = GeoBounds::empty();
                for part in &c.parts {
                    acc = acc.union(part.bounds());
                }
                acc
            }
        }
    }

    /// 点状几何体的单个代表性锚点坐标
    /// （供标签 / 图标放置与单顶点移动使用）。
    pub fn anchor(&self) -> Option<GeoPoint> {
        match self {
            Geometry::Point(p) => Some(*p),
            Geometry::Icon(i) => Some(i.at),
            Geometry::Label(l) => Some(l.at),
            Geometry::Circle(c) => Some(c.center),
            Geometry::Ellipse(e) => Some(e.center),
            _ => None,
        }
    }
}

/// 闭合一个环（在末尾重复第一个顶点）然后沿大圆加密它，
/// 以便闭边的向极凸起也被包含。对于短到没有边的环，
/// 回退到原始切片。
fn subdivide_closed(ring: &[GeoPoint], step_rad: f64) -> Vec<GeoPoint> {
    use crate::geom::sample::subdivide_great_circle;
    if ring.len() < 2 {
        return ring.to_vec();
    }
    let mut closed = ring.to_vec();
    closed.push(ring[0]);
    subdivide_great_circle(&closed, step_rad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_matches_variant() {
        assert_eq!(Geometry::Point(GeoPoint::surface(0.0, 0.0)).kind(), GeometryKind::Point);
        assert_eq!(
            Geometry::Polyline(Polyline { positions: vec![GeoPoint::surface(1.0, 1.0), GeoPoint::surface(2.0, 2.0)] }).kind(),
            GeometryKind::Line
        );
        let comp = Geometry::Composite(Composite {
            kind: SymbolKind::Generic,
            parts: vec![Geometry::Point(GeoPoint::surface(0.0, 0.0))],
        });
        assert_eq!(comp.kind(), GeometryKind::Composite);
    }

    #[test]
    fn vertices_and_bounds() {
        let rect = Geometry::Rectangle(Rectangle { west: -10.0, south: -20.0, east: 30.0, north: 40.0 });
        let b = rect.bounds();
        // 东/西侧是经线（经度被精确保留）；南/北
        // 侧是等纬度弦，其大圆向极凸起，
        // 因此保守盒包含四角并向北超过 40° 成长。
        assert!(
            b.west_deg <= -10.0 && b.east_deg >= 30.0 && b.south_deg <= -20.0 && b.north_deg >= 40.0,
            "corners must be contained: {b:?}",
        );
        assert!(b.north_deg > 40.0 && b.north_deg < 50.0, "bulge must be captured, not absurd: {b:?}");

        let poly = Geometry::Polygon(Polygon {
            outer: vec![GeoPoint::surface(0.0, 0.0), GeoPoint::surface(1.0, 0.0), GeoPoint::surface(0.0, 1.0)],
            holes: vec![vec![GeoPoint::surface(5.0, 5.0)]],
        });
        // 孔也会拓宽保守包围盒。
        assert_eq!(poly.vertices().len(), 4);
        assert_eq!(poly.bounds().east_deg, 5.0);
    }

    #[test]
    fn bounds_covers_every_sampled_vertex() {
        use crate::geom::sample::{
            circle_ring, ellipse_ring, subdivide_great_circle, DEFAULT_SEGMENTS,
            GREAT_CIRCLE_STEP_RAD,
        };
        // 一条长的高纬度东西线：其加密大圆向极凸起，
        // 越过 60°N 的控制点，且包围盒必须覆盖渲染器 / 拾取器
        // 实际会测试的每个顶点。
        let line = Geometry::Polyline(Polyline {
            positions: vec![GeoPoint::surface(0.0, 60.0), GeoPoint::surface(60.0, 60.0)],
        });
        let b = line.bounds();
        let dens = subdivide_great_circle(&line.vertices(), GREAT_CIRCLE_STEP_RAD);
        assert!(dens.iter().all(|p| b.contains(*p)), "bulge vertex outside bounds");
        assert!(b.north_deg > 61.0, "must capture the poleward bulge, got {b:?}");

        // 一个圆：包围盒必须跨遍整个采样圆盘，而不是像
        // 旧的控制顶点盒那样收缩到中心。
        let c = Circle { center: GeoPoint::surface(10.0, 50.0), radius_m: 500_000.0 };
        let cb = Geometry::Circle(c).bounds();
        assert!(cb.width_deg() > 1.0 && cb.height_deg() > 1.0, "circle bounds degenerate: {cb:?}");
        assert!(circle_ring(&c, DEFAULT_SEGMENTS).iter().all(|p| cb.contains(*p)));

        // 同理，一个带朝向的椭圆。
        let e = Ellipse {
            center: GeoPoint::surface(0.0, 0.0),
            semi_major_m: 800_000.0,
            semi_minor_m: 300_000.0,
            rotation_deg: 30.0,
        };
        let eb = Geometry::Ellipse(e).bounds();
        assert!(ellipse_ring(&e, DEFAULT_SEGMENTS).iter().all(|p| eb.contains(*p)), "ellipse ring outside bounds");
    }

    #[test]
    fn serde_roundtrip_preserves_variant() {
        let g = Geometry::Circle(Circle { center: GeoPoint::new(10.0, 20.0, 300.0), radius_m: 1234.5 });
        let s = serde_json::to_string(&g).unwrap();
        let back: Geometry = serde_json::from_str(&s).unwrap();
        assert_eq!(g, back);
    }
}
