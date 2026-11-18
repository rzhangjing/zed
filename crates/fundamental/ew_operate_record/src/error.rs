//! 错误类型。

use thiserror::Error;

/// 中间件错误。
#[derive(Debug, Error)]
pub enum Error {
    /// 数据库读写失败。
    #[error("数据库错误: {0}")]
    Db(#[from] rusqlite::Error),

    /// 创建目录/打开文件失败。
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    /// 记录内容不合法（空的操作人/动作、字段超长等）。
    #[error("非法的操作记录: {0}")]
    Invalid(String),

    /// 记录器已关闭。
    #[error("记录器已关闭")]
    Closed,

    /// 队列已满，本条记录被丢弃。
    #[error("队列已满（容量 {capacity}），本条记录被丢弃")]
    QueueFull {
        /// 队列容量。
        capacity: usize,
    },
}

/// 中间件结果。
pub type Result<T, E = Error> = std::result::Result<T, E>;
