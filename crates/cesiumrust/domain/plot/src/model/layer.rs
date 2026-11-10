//! 一个图层：树中的顶层容器 —— 一个有序的根节点
//! 桶（元素 + 组），带可见性 / 可编辑性 / 不透明度标志以及
//! 新内容落入的“活动图层”标记（计划 §5、§9）。

use serde::{Deserialize, Serialize};

use super::ids::LayerId;
use super::node::Node;

/// 一个标绘图层。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    /// 绘制 / 拾取优先级；越高越后绘制（在上层）。与成员
    /// 自身的 `style.z_order`（在层内排序）不同。
    pub order: i32,
    /// 图层开/关开关（§10.2）。
    pub visible: bool,
    /// 施加在成员颜色之上的组不透明度乘子（§9）。
    pub opacity: f32,
    /// 本层允许编辑吗？锁定的图层为只读（§9）。
    pub editable: bool,
    /// 本层的元素是否可拾取/可选中？（§9）。
    pub selectable: bool,
    /// 新建 / 粘贴的元素落入活动图层（§9）。
    pub active: bool,
    /// 按绘制顺序排列的根成员。
    pub roots: Vec<Node>,
}

impl Layer {
    /// 一个可见、可编辑、可选中、空的图层。
    pub fn new(id: LayerId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            order: 0,
            visible: true,
            opacity: 1.0,
            editable: true,
            selectable: true,
            active: false,
            roots: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_layer_defaults() {
        let l = Layer::new(LayerId(1), "default");
        assert!(l.visible && l.editable && l.selectable);
        assert!(!l.active);
        assert_eq!(l.opacity, 1.0);
        assert!(l.roots.is_empty());
    }
}
