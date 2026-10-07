# Tooling, Evals & Utilities（评测 / 基准 / 基础工具库）

收拢"不直接面向用户、但支撑全局"的模块：**离线评测** [`eval_cli`](../crates/eval_cli)/[`eval_utils`](../crates/eval_utils)、**性能基准** `*_benchmarks`、**Schema/文档** [`schema_generator`](../crates/schema_generator)/[`json_schema_store`](../crates/json_schema_store)/[`docs_preprocessor`](../crates/docs_preprocessor)、以及**基础工具库** [`util`](../crates/util)/`util_macros`、[`collections`](../crates/collections)、[`path`](../crates/path)、[`node_runtime`](../crates/node_runtime)、[`watch`](../crates/watch)、[`env_var`](../crates/env_var)。

## 1. eval_utils / eval_cli：Agent 离线评测框架
用于在 CI/本地跑"给定任务 → Agent 执行 → 校验结果"的可复现评测（配合 gpui 无头）。

[`eval_utils`](../crates/eval_utils/src/eval_utils.rs)
- `enum OutcomeKind`（[L24](../crates/eval_utils/src/eval_utils.rs)）：通过/失败分类。
- `trait EvalOutputProcessor`（[L30](../crates/eval_utils/src/eval_utils.rs)）：如何判定一次输出。
- `struct EvalOutput<M>`（[L37](../crates/eval_utils/src/eval_utils.rs)）+ `passed`(L44)/`failed`(L52)：带指标的结果。
- `struct NoProcessor`（[L61](../crates/eval_utils/src/eval_utils.rs)）：空处理器。
- `eval::<P>(...)`（[L70](../crates/eval_utils/src/eval_utils.rs)）：评测主驱动。

[`eval_cli`](../crates/eval_cli/src)
- `struct Args`（[`main.rs:66`](../crates/eval_cli/src/main.rs)）：CLI 参数（评测文件、模型、并发）。
- `struct EvalResult`（[L117](../crates/eval_cli/src/main.rs)）/ `struct RunStats`（[L148](../crates/eval_cli/src/main.rs)）：单次结果与汇总统计。
- `struct AgentCliAppState`（[`headless.rs:21`](../crates/eval_cli/src/headless.rs)）：无头 `App` 状态，复用真实 Agent 工具链（[Agent-and-AI.md](Agent-and-AI.md)）。

## 2. 性能基准：`*_benchmarks`
[`crates/benchmarks`](../crates/benchmarks)（+ `editor_benchmarks`/`fs_benchmarks`/`project_benchmarks`/`worktree_benchmarks`）用 criterion 组织 `benches/`。它们依赖 gpui 的 `bench-support` feature（见 [GPUI-Platform-Backends.md](GPUI-Platform-Backends.md) 的 `current_headless_renderer`），可在无窗口环境测渲染/调度/大 buffer 性能。运行方式与解释参见技能 `gpui-bench`。

## 3. schema_generator：产出 settings JSON Schema
[`main.rs`](../crates/schema_generator/src/main.rs)：`Args`(L13) + `enum SchemaType`(L24)，把 [`settings_content`](Settings-and-Themes.md) 的类型导出为 `assets/settings/settings.schema.json`，供编辑器内 settings.json 补全/校验。

## 4. json_schema_store：按文件关联外部 Schema
[`json_schema_store.rs`](../crates/json_schema_store/src/json_schema_store.rs)
- `struct SchemaStore`（[L92](../crates/json_schema_store/src/json_schema_store.rs)）+ `init`(L56)：注册 LSP `textDocument/didOpen` 等的 schema 关联。
- `handle_schema_request`（[L141](../crates/json_schema_store/src/json_schema_store.rs)）：响应 `workspace/schemas` 类请求，返回对应 JSON Schema（[LSP-Features.md](LSP-Features.md)）。
- `all_schema_file_associations`（[L410](../crates/json_schema_store/src/json_schema_store.rs)）：glob→schema 映射表。
- `normalize_action_name`/`denormalize_action_name`（[L557](../crates/json_schema_store/src/json_schema_store.rs) / [L561](../crates/json_schema_store/src/json_schema_store.rs)）：Action 名 ↔ 文件名规范化。

## 5. docs_preprocessor
[`docs_preprocessor`](../crates/docs_preprocessor)：mdbook 预处理器，展开 `# {{#action ...}}` 之类的占位（生成文档时注入 Action/设置清单）。纯构建期工具，无 `pub struct` 运行类型。

## 6. node_runtime / watch / env_var
- [`node_runtime`](../crates/node_runtime/src/node_runtime.rs)：`struct NodeRuntime`（[L52](../crates/node_runtime/src/node_runtime.rs)）——下载/管理内嵌 Node.js，供 LSP（ts/js/pyright）、扩展、prettier 使用；`new`（[L63](../crates/node_runtime/src/node_runtime.rs)）。
- [`watch`](../crates/watch)：对 `notify`/fswatch/FSEvents 的封装，被 [`fs::Watcher`](Project-Panel-and-FS.md) 使用。
- [`env_var`](../crates/env_var)：进程环境读写；`zed_env_vars` 集中常量。

## 7. 基础工具库（几乎被所有 crate 依赖）
| Crate | 关键内容 | 用途 |
|---|---|---|
| [`util`](../crates/util) | `ResultExt`/`log`、网络/进程/路径 helper、`post_incubation` 等 | 全站杂项工具 |
| `util_macros` | `paths!`、`define_lang!` 辅助宏 | 编译期拼接 |
| [`collections`](../crates/collections) | `HashMap`/`HashSet`/`IndexMap`/`VecMap` 别名 | 统一容器选型 |
| [`path`](../crates/path) | `PathMatcher`/`PathStyle`/相对路径与位置类型 | 跨平台路径语义 |
| [`paths`](../crates/paths) | 配置/数据/临时目录 | 见 [Settings-and-Themes.md](Settings-and-Themes.md) |
| [`refineable`](../crates/refineable) | `Refineable`/`StyleRefinement` | GPUI 样式构建 |
| [`gpui_shared_string`](../crates/gpui_shared_string) | `SharedString` | 低成本文本 |

## 8. 与其他页面的关系
- Agent 评测依赖真实工具链：[Agent-and-AI.md](Agent-and-AI.md)。
- 基准依赖无头渲染：[GPUI-Platform-Backends.md](GPUI-Platform-Backends.md)。
- Schema 关联 LSP：[LSP-Features.md](LSP-Features.md)。
- 全部基础库被引用于：[Architecture.md](Architecture.md)。
