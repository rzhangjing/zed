//! 嵌入宿主程序的「用户操作记录」中间件。
//!
//! 只做一件事：**用户点了什么、对什么对象、结果如何，记到数据库里**。
//! 系统自己执行的动作（定时任务、内部补偿、启动初始化）不经过这里，也不该调用它。
//!
//! ```no_run
//! use ew_operate_record::{OperationLog, Query, Recorder};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let recorder = Recorder::open("data/operations.db")?;
//!
//! recorder.log(
//!     OperationLog::new("u-1024", "删除订单")
//!         .target("order:O-88231")
//!         .result("成功")
//!         .detail("订单已退款 12.5 元"),
//! )?;
//!
//! recorder.flush()?;                       // 等落库（可选）
//! let entries = recorder.query(&Query::new().user("u-1024").limit(20))?;
//! println!("{} 条记录", entries.len());
//! recorder.close()?;                       // 进程退出前调用
//! # Ok(())
//! # }
//! ```
//!
//! 设计取舍：
//!
//! * 入队即返回，落库在独立线程批量完成，不阻塞界面/请求；
//! * 队列满时丢弃并计数（[`Recorder::stats`]），不会把业务拖死；
//! * 一张表、一个索引集，字段刻意保持少：操作人、动作、对象、结果、详情、时间；
//! * 没有操作者类型：**能调用就说明是用户操作**，系统动作不要调用。

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

mod error;
mod log;
mod query;
mod recorder;
mod store;

pub use error::{Error, Result};
pub use log::{LogEntry, OperationLog};
pub use query::Query;
pub use recorder::{Options, Recorder, Stats};
