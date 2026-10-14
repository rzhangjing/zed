//! 文档操作（计划 §6 / §8）：交互状态机与编辑桥接层所驱动
//! 的纯命令 / 历史 / 变换层。M5 交付了
//! 绘制提交草稿折叠与添加 / 移除命令；M6 新增了可逆的
//! 几何 / 样式 / 可见性命令、几何变换（平移 /
//! 旋转 / 缩放 / 顶点编辑）以及 [`HistoryStack`] 撤销 / 重做记录器。
//! M9 以坐标 [`snap`] 吸附与测地线
//! [`measure`] 读数完善工具包 —— 两者都是纯函数，因此无需引擎即可单元测试。

pub mod command;
pub mod history;
pub mod measure;
pub mod snap;
pub mod transform;

pub use command::{commit_draft, DrawKind, PlotCommand};
pub use history::HistoryStack;
pub use measure::{
    measure_area_m2, measure_length_m, path_length_m, polygon_area_m2, ring_area_m2,
    ring_length_m,
};
pub use snap::{snap, snap_to_grid, SnapConfig, SnapResult};
pub use transform::{rotate, scale, set_vertex, translate};
