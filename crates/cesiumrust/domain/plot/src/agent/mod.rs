//! Agent 赋能层（计划 `Agent 赋能态势标绘`）。
//!
//! 一个小巧、无框架依赖的接口面，外部 agent（LLM 工具调用者、脚本、
//! 服务）通过它驱动态势标绘核心 —— **仅限进程内**，此处没有
//! MCP / HTTP 服务器，也没有内嵌的 LLM。设计红线：agent
//! 只发出高层*意图 / 参数 / 文档数据*；所有几何
//! 采样、投影与栅格化都留在确定性的纯核心中。
//!
//! 四个内聚的切片：
//!  * [`action`] —— 写路径：一个 JSON [`AgentAction::`](action::AgentAction)
//!    意图协议，经编译 + 校验后转换为可逆的
//!    [`PlotCommand`](crate::ops::PlotCommand)。
//!  * [`query`] —— 读路径：一个纯 [`query`](query::query) 过滤器加上
//!    几何 [`measure_length`](query::measure_length) /
//!    [`measure_area`](query::measure_area) 读数。
//!  * [`schema`] —— 自描述契约：一个手写的 JSON
//!    [`action_json_schema`](schema::action_json_schema) 以及无损文档
//!    [`export_document_json`](schema::export_document_json) /
//!    [`import_document_json`](schema::import_document_json)。
//!  * [`session`] —— 聚合点：[`PlotSession`]，宿主共享的
//!    单一入口（文档 + 历史藏在一个 API 之后）。

pub mod action;
pub mod query;
pub mod schema;
pub mod session;

pub use action::{apply_action, compile, ActionError, AgentAction, Applied, StylePatch};
pub use query::{measure_area, measure_length, query, ElementSummary, QueryFilter};
pub use schema::{action_json_schema, export_document_json, import_document_json, EXAMPLE_ACTION};
pub use session::PlotSession;
