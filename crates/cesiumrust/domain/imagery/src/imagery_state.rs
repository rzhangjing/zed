//! 影像状态机。
//!
//! 描述单张影像从加载到可用的生命周期状态。

use serde::{Deserialize, Serialize};

/// 影像瓦片的状态。
/// 映射到 CesiumJS `ImageryState`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ImageryState {
    /// 尚未请求影像。
    #[default]
    Unloaded,
    /// 影像请求正在进行中。
    Transitioning,
    /// 已收到影像数据但尚未处理。
    Received,
    /// 纹理已加载但尚未就绪。
    TextureLoaded,
    /// 影像已可供渲染。
    Ready,
    /// 影像请求失败。
    Failed,
    /// 影像无效（例如格式错误）。
    Invalid,
    /// 占位影像（加载期间使用）。
    Placeholder,
}

impl ImageryState {
    /// 若影像处于终态（Ready、Failed、Invalid）则返回 true。
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Ready | Self::Failed | Self::Invalid)
    }

    /// 若影像可被渲染则返回 true。
    pub fn is_renderable(&self) -> bool {
        matches!(self, Self::Ready | Self::Placeholder)
    }

    /// 若此状态应发起请求则返回 true。
    pub fn should_request(&self) -> bool {
        matches!(self, Self::Unloaded | Self::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_terminal() {
        assert!(ImageryState::Ready.is_terminal());
        assert!(ImageryState::Failed.is_terminal());
        assert!(ImageryState::Invalid.is_terminal());
        assert!(!ImageryState::Unloaded.is_terminal());
        assert!(!ImageryState::Transitioning.is_terminal());
    }

    #[test]
    fn test_is_renderable() {
        assert!(ImageryState::Ready.is_renderable());
        assert!(ImageryState::Placeholder.is_renderable());
        assert!(!ImageryState::Unloaded.is_renderable());
        assert!(!ImageryState::Failed.is_renderable());
    }

    #[test]
    fn test_should_request() {
        assert!(ImageryState::Unloaded.should_request());
        assert!(ImageryState::Failed.should_request());
        assert!(!ImageryState::Ready.should_request());
        assert!(!ImageryState::Transitioning.should_request());
    }
}
