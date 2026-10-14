//! 文档树的数字标识 newtype。
//!
//! Id 由 [`crate` 的 `Document`](super::document::Document)
//! 从单个计数器单调分配，因此它们在单个文档内唯一，且序列化 / diff / 哈希
//! 都很廉价。它们是故意做成不透明的 `u64` 包装而非
//! UUID：文档是内存中的创作态，id 只需在会话内稳定
//! （且在 GeoJSON 往返中会被重新分配）。

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl $name {
            /// 包装一个原始计数器值。
            #[inline]
            pub const fn new(v: u64) -> Self {
                Self(v)
            }
            /// 底层值。
            #[inline]
            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            /// 渲染为 `类型名#计数`（例如 `ElementId#7`）。
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.0)
            }
        }
    };
}

id_newtype!(
    /// 文档中一个 [`Element`](super::element::Element) 的标识。
    ElementId
);
id_newtype!(
    /// 文档中一个 [`Group`](super::group::Group) 的标识。
    GroupId
);
id_newtype!(
    /// 文档中一个 [`Layer`](super::layer::Layer) 的标识。
    LayerId
);
