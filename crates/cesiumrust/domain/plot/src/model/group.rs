//! 组：命名的、可嵌套的成员集合，带可选的组级
//! 变换以及可见性/锁定继承（计划 §5、§9）。

use serde::{Deserialize, Serialize};

use super::ids::GroupId;
use super::node::Node;

/// 应用于整个组的变换，*不会*将其烧录进成员
/// 几何中（计划 §17.4：组保留变换；单元素编辑则烘焙）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GroupTransform {
    /// 绕组枢轴旋转，顺时针度数。
    pub rotate_deg: f64,
    /// 均匀缩放因子。
    pub scale: f64,
    /// 添加到成员的地理偏移（经、纬度度）。
    pub offset_lonlat: [f64; 2],
}

impl Default for GroupTransform {
    /// 恒等变换：不旋转、缩放 1.0、无地理偏移。
    fn default() -> Self {
        Self {
            rotate_deg: 0.0,
            scale: 1.0,
            offset_lonlat: [0.0, 0.0],
        }
    }
}

impl GroupTransform {
    /// 恒等变换。
    pub fn identity() -> Self {
        Self::default()
    }
    /// 当变换不改变任何内容时为 true。
    pub fn is_identity(&self) -> bool {
        self.rotate_deg == 0.0
            && self.scale == 1.0
            && self.offset_lonlat == [0.0, 0.0]
    }
}

/// 一组元素和/或子组。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// 外层组（若嵌套）（文档保证树一致性）。
    pub parent: Option<GroupId>,
    /// 按绘制顺序排列的成员。
    pub members: Vec<Node>,
    /// 可选的组变换；`None` == 恒等。
    pub transform: Option<GroupTransform>,
    /// 手动可见性（沿链向下继承）。
    pub visible: bool,
    /// 锁定的组无法被编辑（其成员也无法被单独
    /// 移动），但作为一个整体仍可能被选中。
    pub locked: bool,
    /// 为 true 时，成员样式由组接管并锁定以免
    /// 逐成员编辑（样式传播钩子）。
    pub style_lock: bool,
}

impl Group {
    /// 一个空、可见、未锁定的组。
    pub fn new(id: GroupId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            parent: None,
            members: Vec::new(),
            transform: None,
            visible: true,
            locked: false,
            style_lock: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_group_is_empty_and_editable() {
        let g = Group::new(GroupId(7), "taskforce");
        assert!(g.members.is_empty());
        assert!(g.visible && !g.locked);
        assert_eq!(g.transform, None);
    }

    #[test]
    fn identity_transform_predicate() {
        assert!(GroupTransform::identity().is_identity());
        assert!(!GroupTransform {
            scale: 2.0,
            ..GroupTransform::identity()
        }
        .is_identity());
    }
}
