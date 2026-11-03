//! 演示用领域模型。新类型按《Rust 结构与枚举定义规范》书写；旧字段注释保留 `。` 以覆盖兼容性。

pub mod domain;

pub use domain::order::{
    Currency, LineItem, Money, Order, OrderError, OrderId, OrderResult, OrderStatus, PaymentState,
};
pub use domain::user::{Load, Role, User, UserId};
