| crate | 用途（中文，≤30 字） | 关键内部依赖 | 被依赖(数量/主要消费者) | 备注 |
|---|---|---|---|---|
| askpass | SSH/Git 口令提示代理，经 Unix socket 回传 | gpui, net, util | 7 / zed, git, git_ui, cli, git_ui_core, recent_projects, project | Windows 需 `set_askpass_program()` 指向 CLI 二进制（SSH_ASKPASS 要求可直接执行）；含 `EncryptedPassword` 零化密码类型；非 Windows 才用 `which`/`make_file_executable` |
| clock | Lamport 时间戳、ReplicaId、版本向量 | （无） | 15 / editor, buffer_diff, auto_update, text, language, client, multi_buffer, workspace, worktree, language_models, agent, edit_prediction, project | `src/clock.rs`=ReplicaId/Lamport/Global；`src/system_clock.rs`=SystemClock trait + FakeSystemClock（test-support 下 parking_lot）；协作 ID 常量仍在（AGENT/LOCAL_BRANCH），但 channel 轴已删 |
| collections | 标准集合别名：FxHashMap/IndexMap/VecMap | gpui_util | 78 / 全仓几乎所有 crate（editor, vim, gpui, zed, project, settings, text…） | `collections.rs` 仅 15 行类型别名 + `vecmap`；有 description；几乎零成本但极广 |
| credentials_provider | 凭据读写 trait 抽象（系统钥匙串后端） | gpui | 5 / client, language_model, language_models, zed_credentials_provider, project | 整个 crate 只有 34 行的 `CredentialsProvider` trait（read/write/delete，均 async） |
| derive_refineable | 生成 `XxxRefinement` 伴生类型的 derive 宏 | （无） | 1 / refineable | 过程宏 crate，位于 `crates/refineable/derive_refineable`；支持 `#[refineable(Debug/Serialize)]` |
| env_var | 环境变量惰性读取与 `env_var!`/`bool_env_var!` | gpui_shared_string | 2 / language_model, zed_env_vars | 40 行；`EnvVar::new` 把空串视作 None；`or()` 做优先级合并 |
| feature_flags | 特性开关注册/存储，来自设置与服务器公告 | collections, feature_flags_macros, fs, gpui, settings | 15 / editor, settings_ui, zed, client, agent_ui, agent_servers, agent, edit_prediction, sidebar… | `flags.rs` 12 个 flag：notebooks, panic, acp-beta, diff-review, create-thread-tool, lsp-tool 等 —— 多为 AI/agent 功能门；`ZED_DISABLE_STAFF` 环境变量 |
| feature_flags_macros | `#[derive(EnumFeatureFlag)]` 过程宏 | （无） | 1 / feature_flags | 为 unit-only 枚举生成 all_variants/override_key/label/from_wire；要求恰好一个 `#[default]` |
| fuzzy | 内存字符串/路径模糊匹配（自有算法） | gpui, gpui_util, path | 27 / editor, vim, file_finder, picker 类, agent_ui, git_ui, project, worktree… | `fuzzy.rs` 只 re-export strings/paths 匹配 API；`char_bag` 用于快速剪枝 |
| fuzzy_nucleo | 基于 nucleo 的模糊匹配（含异步版本） | fuzzy, gpui, gpui_util, path | 8 / language, file_finder, outline, tab_switcher, git_ui, command_palette, recent_projects, project | 与 `fuzzy` API 同形；额外提供 `match_strings_async`；有 criterion benchmark |
| html_to_markdown | 独立 HTML→Markdown 转换库 | （无） | 3 / repl, agent_ui, agent | 发布到 crates.io（publish=true, Apache-2.0）；基于 html5ever + RcDom；无任何 workspace 内部依赖，唯一完全自洽的可搬走 crate |
| path | 保证相对/规范/UTF-8 的相对路径类型 | （无） | 7 / language_core, fuzzy, fs, util, language_tools, fuzzy_nucleo, project | RelPath/RelPathBuf 内部一律 POSIX `/` 分隔；`PathStyle::local()` 在 Windows 返回 Windows；doc 注释自称 "for deltadb" |
| paths | Zed 的数据/配置/缓存/日志目录常量 | util | 27 / zed, client, fs, worktree, agent, settings, db, prompt_store, cli… | 定义 `APP_NAME="Zed"`（fork 改此处避免与 Zed 用户数据冲突）；EDITORCONFIG_NAME |
| refineable | 结构体部分覆盖（主题/设置层级）trait | derive_refineable | 3 / theme, gpui, theme_settings | 仅 re-export derive + `Refineable`/`IsEmpty` trait；`refine(&mut self, &Refinement)` 原地应用非空字段 |
| release_channel | 版本号与发布渠道（stable/dev/nightly/preview） | gpui | 27 / editor, zed, client, agent, language_models, settings, cli, auto_update… | 编译期从 `crates/zed/RELEASE_CHANNEL` include_str；`app_identifier()` 仅 Windows；`#![deny(missing_docs)]` |
| sandbox | agent 命令沙箱策略；Windows 走 WSL+Bubblewrap | http_proxy | 4 / zed, agent_ui, acp_thread, agent | **异常**：`sandbox.rs:24` 仍 `#[cfg(target_os="macos")] mod macos_seatbelt;` 且 509/824-853 行大量引用，但 `src/macos_seatbelt.rs` 已被删除 → macOS 目标已无法编译（Windows 走 `windows_wsl.rs`，2279 行）；策略=`SandboxPolicy{fs,net}`，Windows 仅 bool 级放开网络/写，无 loopback 代理 |
| scheduler | 任务调度器/执行器、优先级与时钟抽象 | （无） | 1 / gpui | 内部零依赖（全用 async-task/flume/chrono/web-time）；`LocalExecutor` 为 `!Send` 单 session；`Priority` 权重 High60/Medium30/Low10，RealtimeAudio 权重 0 独占线程；wasm-threads feature |
| shell_command_parser | 解析 shell 命令，供权限校验/前缀提取 | （无） | 2 / settings_ui, agent | 仅依赖 brush-parser；`extract_commands`/`extract_terminal_command_prefix` 返回 `TerminalCommandValidation::{Safe,Unsafe,Unsupported}` —— 面向 agent 终端命令审批 |
| snippet | 解析 LSP snippet 文本为 tabstop/占位 | （无） | 4 / editor, snippet_provider, languages, project | 360 行，仅 anyhow+smallvec；`Snippet::parse` 处理 `$0` 与 choices |
| sqlez | SQLite 封装：线程安全连接、迁移、领域模型 | collections, util | 4 / workspace, agent, db, sqlez_macros | `lib.rs` 导出 bindable/connection/domain/migrations/savepoint/statement/typed_statements/thread_safe_connection；按 uri 全局串行写队列，每库单 worker 线程 |
| sqlez_macros | `sql!` 宏：编译期格式化并校验 SQL | sqlez | 1 / db | 非 Linux/FreeBSD 上通过内存库 `sql_has_syntax_error` 做编译期语法检查；Linux 上跳过（error=None） |
| sum_tree | 并发友好 B 树（Cursor/Map/Set） | ztracing | 12 / editor, buffer_diff, text, git, language, rope, multi_buffer, markdown, gpui, worktree, project, notifications | 有 description；`TREE_BASE` test=2/non-test=6；用 `ztracing::instrument` 标注；`zlog` 只在 dev-dependencies |
| task | 任务/调试配置、模板与 VS Code 格式解析 | collections, gpui, proto, util, zed_actions | 20 / editor, tasks_ui, terminal, languages, language, dap, agent_ui, acp_thread, git_ui, workspace, project… | `task.rs` 定义 TaskId/SpawnInTerminal；解析 `.vscode/tasks.json`、launch.json、adapter schema；`zed_actions::RevealTarget` 由 util 层语义提供 |
| time_format | 时间戳本地化格式化（绝对/相对/增强） | （无） | 2 / zed, git_ui | 1161 行、零内部依赖；`TimestampFormat::{Absolute,EnhancedAbsolute,MediumAbsolute,Relative}`；平台依赖 sys-locale/time + macOS core-foundation、Windows windows（Windows 分支为本 fork 实际路径） |
| util | 通用工具箱：fs/shell/命令/归档/markdown/路径 | collections, gpui_util, path, util_macros(可选) | 104 / 全仓绝大多数 crate | 有 description；模块：archive, command, fs, process, shell(+shell_builder,shell_env), serde, schemars, markdown, size, time, paths, redact, disambiguate…；`Shell`/`get_system_shell` 被 task/terminal 复用；被 `zed_actions` 与 `db` 依赖，属最底层枢纽 |
| util_macros | `path!`/`uri!`/`line_endings!` 等工具宏 | perf(tooling/perf) | 6 / vim, terminal, util, inspector_ui, search, gpui | Windows 下 `path!` 把 `/` 换成 `\` 并补 `C:`；`perf-enabled` feature |
| watch | 单值广播通道（watchable value） | （无） | 11 / zed, language, action_log, agent_ui, node_runtime, agent_servers, acp_thread, git_ui, agent, git_ui_core, project | 纯 parking_lot 实现，生产依赖仅 parking_lot（gpui/zlog 是 dev-deps）；`Sender::send` 版本号+唤醒 waker 列表 |
| zlog | Zed 日志门面：作用域过滤、文件轮转、ANSI 着色 | collections | 30 / watch, editor, buffer_diff, sum_tree, text, lsp, language, dap, rope, workspace, worktree, agent, project… | `init/try_init` 读 `ZED_LOG`→`RUST_LOG`→CI；`SCOPE_DEPTH_MAX=4`；文件超 1MB 轮转；ANSI 色码内联 |
| zlog_settings | 设置里的 log.scopes → 日志级别映射 | collections, gpui, settings, zlog | 1 / zed | 30 行；`init()` 订阅 SettingsStore，转调 `zlog::filter::refresh_from_settings` |
| ztracing | tracing 门面；ztracing 开启时接 Tracy | zlog, ztracing_macro | 15 / editor, buffer_diff, sum_tree, zed, git, language, multi_buffer, mermaid_render, rope, search, gpui, git_ui, worktree, git_ui_core, project | **未开 `ztracing` feature 时**（默认）：`instrument` 为空展开、span/event 宏全部丢弃，只留空 `struct Span`；`tracy` feature 另有全局 ProfiledAllocator |
| ztracing_macro | `#[instrument]` 属性宏占位实现 | （无） | 1 / ztracing | `lib.rs` 仅 7 行：原样返回 item，不做任何插桩（真正的插桩来自 ztracing 里的 tracing / tracy 分支） |

## 证据与口径

- 依赖与被依赖数据取自 `crate-graph-win.csv`（`cargo metadata --filter-platform x86_64-pc-windows-msvc`）。**注意口径差异**：CSV 的 `In`/`Rev` 列把 `[dev-dependencies]` 也算作依赖 —— 例如 `sum_tree` 的 `zlog`、`watch` 的 `gpui`/`zlog`、`util_macros` 的 `perf` 实际只在 dev/build 语境。表中"关键内部依赖"一列按 `Cargo.toml` 的 `[dependencies]` + `[target.'cfg(...)'.dependencies]` 记录，因此与 CSV 的 InN 不完全一一对应。
- 用途判定依据：各 crate `Cargo.toml`、`src/lib.rs`（或 `src/main.rs` / `[lib] path` 指向的入口）头部模块声明与 doc 注释、以及关键类型/函数签名。未发现 description 的 crate 均通过模块与公开 API 推断，未臆造功能。
- 全仓仅 17/198 个 crate 有 `description`；本组内有 description 的是 `collections`、`derive_refineable`、`html_to_markdown`、`refineable`、`release_channel`、`sum_tree`、`util`、`util_macros`。

## 本组功能主线

底层无依赖的"零依赖原语"（`collections` 除外仅挂 gpui_util、`path`、`snippet`、`scheduler`、`shell_command_parser`、`html_to_markdown`、`time_format`、`derive_refineable`、`feature_flags_macros`、`ztracing_macro`）→ 单点工具（`env_var`、`watch`、`credentials_provider`、`clock`）→ 组合基建（`util`、`paths`、`zlog`/`zlog_settings`/`ztracing`、`sum_tree`、`sqlez`/`sqlez_macros`、`refineable`、`release_channel`、`feature_flags`）→ 领域层（`task`、`sandbox`、`askpass`、`fuzzy`/`fuzzy_nucleo`、`time_format`）。

## 异常发现

1. `crates/sandbox/src/sandbox.rs:24` 仍声明 `#[cfg(target_os = "macos")] mod macos_seatbelt;`，并在 509、824-826、831、853 行使用其类型，但 `crates/sandbox/src/macos_seatbelt.rs` 已被裁剪删除 —— macOS 目标编译必失败。Windows 不受影响（走 `mod windows_wsl`）。
2. `time_format/Cargo.toml` 仍保留 macOS `core-foundation`/`core-foundation-sys` 依赖；同类的 mac 专属残留值得在后续审计中统一清理。
3. `ztracing` 在本 fork 的默认构建（不开 `ztracing` feature）下是**空壳**：`instrument` 直通、所有 span/event 宏被 `__consume_all_tokens` 吞掉，`Span` 是空结构体；`ztracing_macro` 更是 7 行恒等宏。两者被 15/1 个 crate 引用，属"零运行时成本但也零收益"的插桩占位。
4. `feature_flags` 的 flag 集合（notebooks、panic、acp-beta、diff-review、create-thread-tool、lsp-tool 等）明显是 AI/agent 功能门，是已删除的 AI 轴的遗留物；`clock` 里的 `ReplicaId::AGENT`/`FIRST_COLLAB_ID` 同样是 channel/协作轴的残留常量。

## 明确可裁剪候选

- **`ztracing` + `ztracing_macro`**：默认构建下完全 no-op，若放弃 Tracy 性能剖析能力，把 `ztracing::instrument` 换成空属性或直接删掉、把 `ztracing::Span` 相关引用替换为 `()`，即可消掉两个 crate（涉及 15 个消费者的改动量）。
- **`feature_flags` / `feature_flags_macros`**：若随 AI 轴一并去掉，可省掉一个 proc-macro crate 和 15 处消费点；但 `panic` 等非 AI flag 需先迁移。
- **`html_to_markdown`**：唯一 publish 到 crates.io、零内部依赖的独立库，只在 `repl`/`agent_ui`/`agent` 使用；若保留 repl 的 HTML 复制能力则必须保留，否则可直接删。
- **`askpass`**：Windows 上依赖外部 CLI 二进制与 `ZED_ASKPASS_SOCKET` 协议，7 个消费者集中在 git 相关 crate；若不需要远程 Git 凭据提示可整块移除。
- **`time_format`**：仅 `zed`、`git_ui` 两处使用，可下沉为 `util` 的一个模块以消掉一个 crate（携带平台依赖 sys-locale/time）。
