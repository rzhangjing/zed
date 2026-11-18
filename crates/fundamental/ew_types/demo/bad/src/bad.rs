//! 故意不合规的示例，仅用于演示 `ew-types check`。

/// 缺字段注释的结构体。
pub struct Sloppy {
    pub a: i64,
    /// 备注。
    pub b: i64,
}

pub struct NoDocs {
    pub x: u32,
}

/// 状态。
pub enum BadStatus {
    A,
    Other(String),
}
