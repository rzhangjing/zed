//! 图层与组共享的树边类型：一个成员要么是
//! 元素（叶），要么是嵌套的组（枝）。

use serde::{Deserialize, Serialize};

use super::ids::{ElementId, GroupId};

/// [`Layer`](super::layer::Layer) 或 [`Group`](super::group::Group) 中的一个子槽位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Node {
    /// 一个叶元素（通过文档的元素表解析）。
    Element(ElementId),
    /// 一个嵌套的组（通过文档的组表解析）。
    Group(GroupId),
}
