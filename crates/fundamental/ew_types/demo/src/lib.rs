//! 演示用领域模型。所有类型都按《Rust 结构与枚举定义规范》书写。

pub mod domain;

pub use domain::order::{Currency, LineItem, Money, Order, OrderError, OrderId, OrderStatus};
pub use domain::user::{Load, Role, User, UserId};
