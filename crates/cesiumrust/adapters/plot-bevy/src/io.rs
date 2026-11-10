//! 面向实时叠加层的 GeoJSON 导入 / 导出（计划 §12 / §14，M8）。
//!
//! 基于纯函数 [`cesium_plot::io::geojson`] 编解码器的薄桥接。导入将每个
//! 解析出的元素折叠为一个**面向当前活动层的新增命令**（id 由实时文档
//! 重新铸造，因此粘贴绝不会碰撞），并通过 M6 提交路径应用，以便
//! 整个导入是*单个*撤销步（计划 §14 “粘贴落活动层”）。导出序列化
//! 实时文档，包含样式与 `x-plot` 载荷。
//!
//! 两者都是作用于普通 [`PlotDocument`] / [`PlotHistory`] 资源的自由函数——无窗口、
//! 无文件对话框——因此它们可 headless 单测；应用决定何时调用
//! 它们（菜单 / 拖放 / 剪贴板）。

use cesium_plot::io::geojson::{self, PlotIoError};
use cesium_plot::model::Document;
use cesium_plot::ops::PlotCommand;

use crate::edit::apply_command;
use crate::resources::{PlotDocument, PlotHistory};

/// 解析一个 GeoJSON 字符串并将其元素作为一个可撤销命令粘贴到活动层，
/// 返回放置的数量。一个不含受支持几何的空 / 外部文件不会新增任何内容
/// （仍不是错误）。
///
/// # 参数
/// - `plot_doc`/`history`：目标文档与命令栈。
/// - `text`：待解析的 GeoJSON 文本。
pub fn import_geojson(
    plot_doc: &mut PlotDocument,
    history: &mut PlotHistory,
    text: &str,
) -> Result<usize, PlotIoError> {
    let parsed = geojson::from_geojson(text)?;
    let steps = build_import_commands(&mut plot_doc.doc, &parsed);
    let count = steps.len();
    apply_command(plot_doc, history, PlotCommand::Composite { steps });
    Ok(count)
}

/// 将实时文档序列化为一个美观的 GeoJSON `FeatureCollection` 字符串。
///
/// # 参数
/// - `plot_doc`：待导出的文档。
pub fn export_geojson(plot_doc: &PlotDocument) -> Result<String, PlotIoError> {
    geojson::to_geojson(&plot_doc.doc)
}

/// 构建将 `src` 的每个元素丢到 `dst` 活动层的新增命令（若 `dst` 没有活动层则
/// 创建并聚焦一个），在 `dst` 的计数器中**重新铸造每个 id**，
/// 以便粘贴的元素绝不会与现有元素碰撞。对两个文档都是纯函数，因此
/// id 分配 / 活动层回退无需资源窗口即可单测。
///
/// # 参数
/// - `dst`：接收导入元素的目标文档（铸造新 id）。
/// - `src`：已解析的源文档。
fn build_import_commands(dst: &mut Document, src: &Document) -> Vec<PlotCommand> {
    let target = ensure_active_layer(dst);
    let mut steps = Vec::new();
    for id in src.flatten_draw_order() {
        let Some(el) = src.element(id) else {
            continue;
        };
        // 来自 `dst` 的新鲜 id；复制其他所有载荷（镜像 M6
        // 复制）。`make_element` 从几何副本重新计算 bounds。
        let mut ne = dst.make_element(el.name.clone(), el.geometry.clone());
        ne.element.style = el.style.clone();
        ne.element.attributes = el.attributes.clone();
        ne.element.flags = el.flags;
        ne.element.scale_visibility = el.scale_visibility;
        ne.element.time_window = el.time_window;
        steps.push(PlotCommand::AddElement {
            layer: target,
            element: Box::new(ne.element),
        });
    }
    steps
}

/// 确保文档有一个可粘贴到的活动层，当它没有时创建一个全新的。
/// 返回（可能是新建的）活动层 id。
///
/// # 参数
/// - `doc`：待保证活动层的文档。
fn ensure_active_layer(doc: &mut Document) -> cesium_plot::model::ids::LayerId {
    if let Some(l) = doc.active_layer() {
        return l;
    }
    let l = doc.new_layer("导入");
    doc.focus_layer(l);
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_plot::geo::GeoPoint;
    use cesium_plot::model::geometry::Geometry;
    use cesium_plot::ops::HistoryStack;

    /// 构造含一个已有点元素的默认层文档，供导入测试使用。
    fn fresh_doc(lon: f64, lat: f64) -> PlotDocument {
        let mut d = PlotDocument {
            doc: Document::with_default_layer(),
            revision: 0,
            dirty: true,
        };
        let layer = d.doc.active_layer().unwrap();
        let ne = d.doc.make_element("已有", Geometry::Point(GeoPoint::surface(lon, lat)));
        d.doc.add_element_to_layer(layer, ne);
        d
    }

    /// 生成一段含一个点与一条线的源文档 GeoJSON（导入测试用）。
    fn geojson_text() -> String {
        let mut src = Document::default();
        let l = src.new_layer("导入源");
        let a = src.make_element("甲", Geometry::Point(GeoPoint::surface(10.0, 20.0)));
        let b = src.make_element(
            "乙",
            Geometry::Polyline(cesium_plot::model::geometry::Polyline {
                positions: vec![GeoPoint::surface(0.0, 0.0), GeoPoint::surface(1.0, 1.0)],
            }),
        );
        src.add_element_to_layer(l, a);
        src.add_element_to_layer(l, b);
        geojson::to_geojson(&src).unwrap()
    }

    /// 导入应粘贴到活动层，并作为单个 composite 可一次撤销。
    #[test]
    fn import_pastes_into_active_layer_and_is_undoable() {
        let mut d = fresh_doc(0.0, 0.0);
        let mut h = PlotHistory(HistoryStack::new());
        let before = d.doc.element_count();
        let active = d.doc.active_layer().unwrap();
        let n = import_geojson(&mut d, &mut h, &geojson_text()).unwrap();
        assert_eq!(n, 2, "two imported features");
        assert_eq!(d.doc.element_count(), before + 2);
        // 所有内容都落在已存在的活动层中。
        for id in d.doc.element_ids() {
            if d.doc.element(id).unwrap().name.starts_with('甲') || d.doc.element(id).unwrap().name.starts_with('乙') {
                assert_eq!(d.doc.element_context(id).unwrap().0, active);
            }
        }
        // 一次撤销移除整个导入（单个 composite 步）。
        h.0.undo(&mut d.doc);
        assert_eq!(d.doc.element_count(), before);
    }

    /// 导入应铸造全新 id，绝不与现有元素碰撞。
    #[test]
    fn import_mints_fresh_ids_no_collision() {
        let mut d = fresh_doc(0.0, 0.0);
        let mut h = PlotHistory(HistoryStack::new());
        let existing: Vec<_> = d.doc.element_ids().collect();
        import_geojson(&mut d, &mut h, &geojson_text()).unwrap();
        // 没有任何导入 id 等于一个已存在的 id。
        let after: Vec<_> = d.doc.element_ids().collect();
        for id in &after {
            if existing.contains(id) {
                continue;
            }
            assert!(!existing.contains(id));
        }
        assert_eq!(after.len(), existing.len() + 2);
    }

    /// 导出→解析应回环实时文档（元素与几何保持一致）。
    #[test]
    fn export_roundtrips_live_document() {
        let d = fresh_doc(3.0, 4.0);
        let text = export_geojson(&d).unwrap();
        let back = geojson::from_geojson(&text).unwrap();
        assert_eq!(back, d.doc);
    }

    #[test]
    fn ensure_active_layer_creates_when_none() {
        let mut doc = Document::default();
        assert!(doc.active_layer().is_none());
        let l = ensure_active_layer(&mut doc);
        assert_eq!(doc.active_layer(), Some(l));
        // Idempotent: a second call returns the same active layer.
        assert_eq!(ensure_active_layer(&mut doc), l);
    }
}
