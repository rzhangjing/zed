//! cesium-pipeline —— 通用瓦片流水线适配器（M1.2）。
//!
//! 无 Bevy、纯 `std` 的实现，对应 `cesium-ports-driven` 中 M1.1 的
//! `TilePipeline` 契约。以可复用、可测试的 crate 形式，忠实复刻
//! `dynamic_globe.rs`（黄金路径参考）的预算/驱逐/过期/重试语义。
//!
//! ## 架构
//!
//! ```text
//!  submit(key, prio)          poll_ready(budget)
//!       │                          ▲
//!       ▼                          │
//!  ┌─────────┐   job_tx    ┌──────────────┐   result_tx   ┌──────────┐
//!  │ Dedup   │────────────►│ Worker Pool  │──────────────►│ Drain    │
//!  │ + Wanted│             │ (N threads)  │               │ Queue    │
//!  └─────────┘             └──────────────┘               └──────────┘
//!       ▲                          │
//!       │                          ▼
//!  refresh_wanted()         NetworkBackend (ureq / reqwest)
//! ```
//!
//! ## 模块地图
//!
//! | 模块 | 职责 |
//! |--------|---------------|
//! | `runtime` | `GenericPipeline<K,P>` —— `TilePipeline` 的实现 |
//! | `pool` | 阻塞式工作池（16 线程，keep-alive） |
//! | `budget` | `DefaultBudget` —— 逐字采用的 dynamic_globe 常量 |
//! | `gpu_cache` | 带基础层锁定 + 活跃延迟的 FIFO 驱逐 |
//! | `hidden_lru` | 隐藏（温回退）实体的 LRU 追踪 |
//! | `dedup` | 在途去重集合 |
//! | `base_layer` | 基础层 zoom 豁免逻辑 |
//! | `retry` | `DefaultRetry` —— 双时间尺度重试策略 |
//! | `staleness` | `DefaultStaleness` —— 三态结果分类 |
//! | `net` | `NetworkBackend` trait + ureq/reqwest 实现 |
//! | `resource_backend` | M8 `ResourceBackend` —— 经由复用的缓存层级流式传输 bulk 资产 |

pub mod base_layer;
pub mod budget;
pub mod dedup;
pub mod gpu_cache;
pub mod hidden_lru;
pub mod net;
pub mod pool;
pub mod resource_backend;
pub mod retry;
pub mod runtime;
pub mod staleness;

// 为方便起见，重新导出核心流水线类型 + 默认策略实现。
pub use budget::DefaultBudget;
pub use resource_backend::PipelineResourceBackend;
pub use retry::DefaultRetry;
pub use runtime::GenericPipeline;
pub use staleness::DefaultStaleness;
