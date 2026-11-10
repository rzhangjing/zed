//! 不依赖 tokio 的 tile fetch 层 + `CESIUM_ENABLE_PIPELINE` 门控辅助函数（M1.4）。
//!
//! # 为何这住在 `adapters/bevy-render`（架构红线）
//! 四个 P0 loader 住在这个适配层里，但权威的门控访问器
//! `pipeline_enabled()` 住在 `application/cesium-app/src/feature_flags.rs`。
//! 一个适配层**绝不可**依赖应用层（六边形规则），所以它在此
//! 无法被导入。取而代之，本模块直接读取*进程全局*的
//! 环境变量 `CESIUM_ENABLE_PIPELINE`，使用一个与 `feature_flags::truthy`
//! 逐字节相同的 `truthy` 谓词。读一个环境变量不是一种应用依赖（env 是进程
//! 状态，而非 app artefact），且它镜像了 cesium-app 自身在启动时读 env 的方式
//! —— 所以两条读取路径在解析与默认值上完全一致。
//!
//! 考虑过并否决的替代方案：
//! - (b) 一个被注入的 `Resource`/component 门控 —— 会迫使 cesium-app 去接它，
//!   而 pipeline 插件是选择性开启 / 不在默认运行时里，所以
//!   loader 无法依赖它的存在（它们必须安全地默认 OFF）。
//! - (c) 从纯 `std` 的 `cesium-pipeline` core 暴露该门控 —— 会把一个 Bevy/app
//!   的推广关注点推进领域无关的 core。
//!
//! 选项 (a) —— 一次本地 env 读取 —— 是耦合最少、不违规的选择。
//!
//! # Fetch 路由（都不依赖 tokio，都离开帧线程）
//! - [`pipeline_fetch`] —— 门控 ON：来自 cesium-pipeline core 的**共享 keep-alive 池化**
//!   `UreqBackend`（那个"ureq 阻塞池"）。每进程一个 agent ⇒ tile-server 连接
//!   在各次 fetch 之间保持温热（相对每次调用新建客户端的 M1 改进）。
//! - [`legacy_fetch`] —— 门控 OFF：每次调用一个**全新**的 `UreqBackend`，保留
//!   迁移前 `HttpTileFetcher::new(url)`-per-tile 的语义（无池化），
//!   只是去掉了 tokio current-thread 运行时。
//!
//! 两者都被设计为运行在一个后台 worker（Bevy `IoTaskPool`）上，从不在
//! 帧线程上。
//!
//! # 关于 `GenericPipeline` 的说明（完整编排推迟到 M1.5）
//! core 的 `GenericPipeline<K, Payload>`（那个 `TilePipeline` impl）在 M1.4
//! 并**未**按 loader 接入，原因是在阅读 core 时发现的具体几点：
//! 1. 它的 `Decoder: Fn(&[u8]) -> Option<Payload>` 无法访问 tile key，
//!    但地形解码需要 `skirt_height(level)`、tileset content 需要
//!    `rtc_center` —— 两者都由 key 派生，所以仅凭字节的 decode 无法构建
//!    payload。
//! 2. 它的 `K: Copy` 约束排除了结构以 key 索引的 tileset loader
//!    （`Vec<usize>` 路径、`String` url）。
//! 3. 让它的 worker 池跨帧持久需要一个新 `Resource`，
//!    违反 M1.4 的"保持四个系统签名不变"约束。
//!
//! 因此 M1.4 消费 core 的 `NetworkBackend`（ureq 阻塞 keep-alive 池）—— 那层
//! 不依赖 tokio 的 fetch —— 而这恰恰就是本里程碑所要求的
//! "ureq 阻塞池，帧线程不阻塞"。完整的 `TilePipeline` 编排（去重/wanted/
//! budget drain）在 dynamic_globe 被瘦身、且存在一个 key-感知的解码器之后，
//! 于 M1.5 落地。

use std::sync::{Arc, OnceLock};

use cesium_pipeline::net::ureq_backend::UreqBackend;
use cesium_pipeline::net::{FetchResult, NetworkBackend};

/// `CESIUM_ENABLE_PIPELINE` —— M1.x pipeline 门控。名称与 cesium-app
/// feature-flag 注册表（`feature_flags::ENV_ENABLE_PIPELINE`）一致，所以两条读取
/// 路径观察到完全相同的环境变量。
pub const ENV_ENABLE_PIPELINE: &str = "CESIUM_ENABLE_PIPELINE";

/// 本帧一个 loader 走哪条 fetch 路由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchRoute {
    /// 门控 ON —— 共享 keep-alive 池化的后端（cesium-pipeline core）。
    Pipeline,
    /// 门控 OFF —— 每次调用新建的后端（旧语义，不依赖 tokio）。
    Legacy,
}

/// 真值-token 谓词 —— 与 cesium-app 里的 `feature_flags::truthy` **逐字节相同**。
/// 接受（不区分大小写、去周边空白）：`1`、`true`、`yes`、`on`。
/// 其余一切（包括未设置）都是 `false`。
fn truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 从一个原始 env 值（`None` = 未设置）纯求值门控。从 [`pipeline_gate_enabled`]
/// 拆出，以便它可以在**不**改动进程全局 env
/// （那会与并行测试竞态）的前提下被单元测试。
pub fn gate_from_env_value(raw: Option<String>) -> bool {
    match raw {
        Some(v) => truthy(&v),
        None => false,
    }
}

/// 在不依赖应用层的前提下读取 `CESIUM_ENABLE_PIPELINE` 门控
/// （见模块级的架构理由）。默认 OFF。
pub fn pipeline_gate_enabled() -> bool {
    gate_from_env_value(std::env::var(ENV_ENABLE_PIPELINE).ok())
}

/// 从一个显式的门控标志解析 fetch 路由（一个可测试的接缝，不
/// 触碰 env，因此两条分支在测试中都被确定性地演练）。
pub fn route_for(use_pipeline: bool) -> FetchRoute {
    if use_pipeline {
        FetchRoute::Pipeline
    } else {
        FetchRoute::Legacy
    }
}

/// 共享的 keep-alive 池化后端（cesium-pipeline core）。每进程一个 agent ⇒
/// 跨每一次 tile fetch 的温热连接（pipeline 路由）。
fn shared_backend() -> Arc<dyn NetworkBackend> {
    static BACKEND: OnceLock<Arc<dyn NetworkBackend>> = OnceLock::new();
    Arc::clone(BACKEND.get_or_init(|| Arc::new(UreqBackend::new())))
}

/// 把一个 core [`FetchResult`] 映到 `Result<Vec<u8>, String>`。
fn to_result(url: &str, r: FetchResult) -> Result<Vec<u8>, String> {
    match r {
        FetchResult::Ok(bytes) => Ok(bytes),
        FetchResult::Transient(e) => Err(format!("fetch {url}: transient: {e}")),
        FetchResult::Permanent(e) => Err(format!("fetch {url}: permanent: {e}")),
    }
}

/// 门控 ON：经由共享 keep-alive 池化的后端 fetch（那个 ureq 阻塞池）。
/// 阻塞式；必须运行在一个后台 worker 上，从不在帧线程。
pub fn pipeline_fetch(url: &str) -> Result<Vec<u8>, String> {
    to_result(url, shared_backend().fetch(url))
}

/// 门控 OFF：经由一个每次调用新建的后端 fetch（旧的无池化语义，
/// 不依赖 tokio）。对应迁移前逐 tile 的 `HttpTileFetcher::new(url)`。
/// 阻塞式；必须运行在一个后台 worker 上，从不在帧线程。
pub fn legacy_fetch(url: &str) -> Result<Vec<u8>, String> {
    to_result(url, UreqBackend::new().fetch(url))
}

/// 分派到由 `use_pipeline` 选定的路由。这是所有四个 P0 loader 使用的
/// 单一不依赖 tokio 的 fetch 入口点。
pub fn fetch_gated(url: &str, use_pipeline: bool) -> Result<Vec<u8>, String> {
    match route_for(use_pipeline) {
        FetchRoute::Pipeline => pipeline_fetch(url),
        FetchRoute::Legacy => legacy_fetch(url),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_accepts_canonical_tokens() {
        for t in ["1", "true", "TRUE", "True", "yes", "YES", "on", "ON", " 1 ", "\ttrue\n"] {
            assert!(
                gate_from_env_value(Some(t.to_string())),
                "expected ON: {t:?}"
            );
        }
    }

    #[test]
    fn gate_rejects_everything_else_and_defaults_off() {
        assert!(!gate_from_env_value(None), "unset must be OFF (default)");
        for t in ["", "0", "false", "no", "off", "2", "maybe", "enabled"] {
            assert!(
                !gate_from_env_value(Some(t.to_string())),
                "expected OFF: {t:?}"
            );
        }
    }

    #[test]
    fn route_for_selects_branch_deterministically() {
        // 证明门控 ON/OFF 分支仅由该标志选定（无 env
        // 改动，所以它在并行测试执行下无竞态）。
        assert_eq!(route_for(true), FetchRoute::Pipeline);
        assert_eq!(route_for(false), FetchRoute::Legacy);
    }

    #[test]
    fn shared_backend_is_pooled_singleton() {
        // 门控 ON 复用单一 agent（keep-alive 池）；指针相等证明
        // 相对旧每次调用路由的连接池化改进。
        let a = shared_backend();
        let b = shared_backend();
        assert!(
            Arc::ptr_eq(&a, &b),
            "pipeline route must share one pooled backend"
        );
        assert_eq!(a.name(), "ureq");
    }

    #[test]
    fn legacy_fetch_maps_error_without_panic() {
        // 一个不可路由的 URL 产出一个 Err（从不 panic，也从不阻塞超出 ureq
        // 超时）：不依赖 tokio 的路径像旧那个一样优雅降级。
        let r = legacy_fetch("http://127.0.0.1:1/__no_such_tile__");
        assert!(r.is_err());
    }

    #[test]
    fn fetch_gated_routes_by_flag() {
        // 两条路由都命中同一个不可路由的 URL，且都必须返回 Err，
        // 证明该分派接缝对任一门控值都工作。
        let url = "http://127.0.0.1:1/__no_such_tile__";
        assert!(fetch_gated(url, true).is_err());
        assert!(fetch_gated(url, false).is_err());
    }
}
