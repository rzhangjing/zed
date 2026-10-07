# Zed 裁剪后 crate 全量分析（198 个 workspace 成员）

## 0. 数据来源与口径

- 图数据：`cargo metadata --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc`（host = `x86_64-pc-windows-msvc`），快照落盘于 `%TEMP%\crate-graph-win.csv`。
- 规模：**198 个 workspace 成员**（`crates/*` + `tooling/{compliance,perf,xtask}`），**1,844 条内部直接依赖边**。
- 含义：`In` = 该 crate 直接依赖的内部 crate；`Rev` = 反向依赖（谁依赖它）。计数均为 **Windows 目标视图**；只写在非 Windows `target` 段里的依赖不计入，这类"仅其它平台使用"的情况会在备注中单独标出。
- 用途来源：`Cargo.toml` 的 `description` 仅 17/198 个 crate 存在、`src/lib.rs` 顶部 `//!` 文档仅 2/198 → 绝大多数用途由逐 crate 阅读 `src/` 模块与公开 API 推断。
- 分组：把 198 个 crate 按领域切成 9 组，各组明细表分别见 `01-platform.md` … `09-ai-agent.md`。
- ⚠️ 口径提醒：`In`/`Rev` 来自 `cargo metadata` 的 resolve 图，**包含 dev-dependencies 边**（例如 `sum_tree → zlog`、`watch → gpui/zlog`、`util_macros → perf`）。要判断"运行期是否真依赖"，需回看各 crate 的 `[dependencies]`；各组明细表已尽量在备注里区分。

## 1. 依赖图总体形状

- 平均出度 = 平均入度 = **9.31**；入度分布：0 个消费者 **8** 个 crate、1 个 **59**、2–4 个 **72**、5–9 个 **32**、10–29 个 **57**、≥30 个 **56**。
- 最长依赖链 **34 层**（L0 叶子 → L34 `zed`），是严格的单向分层 DAG：只有上层依赖下层，无环。
- 枢纽（RevN ≥ 20，改动影响面最大）：`gpui` 134、`util` 104、`collections` 78、`settings` 78、`ui` 63、`workspace` 63、`language` 60、`project` 60、`theme` 49、`editor` 47、`theme_settings` 46、`fs` 42、`zed_actions` 41、`menu` 35、`zlog` 30、`db` 29、`picker` 29、`http_client` 29、`client` 27、`fuzzy` 27、`paths` 27、`release_channel` 27、`telemetry` 27、`text` 25、`task` 20。
- 无内部依赖的叶子（26）：`clock`、`compliance`、`derive_refineable`、`feature_flags_macros`、`gpui_linux`、`gpui_shared_string`、`gpui_util`、`gpui_web`、`html_to_markdown`、`http_client_tls`、`http_proxy`、`icons`、`media`、`net`、`oauth_callback_server`、`path`、`proto`、`proxy_handshake`、`scheduler`、`shell_command_parser`、`snippet`、`telemetry_events`、`time_format`、`windows_resources`、`zeta_prompt`、`ztracing_macro`。
- 无反向依赖的（8）：`zed`（应用入口）、`xtask`（构建任务可执行）、`auto_update_helper`（独立更新助手二进制）、`theme_importer`（独立主题导入工具）、`media`（**完全孤立**：唯一消费者 `gpui_macos` 已删除）、`gpui_linux`/`gpui_web`/`gpui_wgpu`（只在非 Windows target 段被 `gpui_platform`/`gpui_linux` 引用）。
- 按依赖类别看：**没有任何 crate 只被 dev-dependencies 依赖**（每个 crate 至少有一个运行期消费者，或属于上面 8 个 rev=0 的特例）；**只有 1 个 crate 仅作为 build-dependency 存在**——`windows_resources`（Windows 资源清单嵌入，被 `zed`/`cli`/`assets` 在构建期使用）。

## 2. 35 层依赖分层（每层内按字母序）

| 层 | crate 数 | 成员 |
| --- | --- | --- |
| L0 | 26 | clock, compliance, derive_refineable, feature_flags_macros, gpui_linux, gpui_shared_string, gpui_util, gpui_web, html_to_markdown, http_client_tls, http_proxy, icons, media, net, oauth_callback_server, path, proto, proxy_handshake, scheduler, shell_command_parser, snippet, telemetry_events, time_format, windows_resources, zeta_prompt, ztracing_macro |
| L1 | 10 | auto_update_helper, collections, edit_prediction_metrics, env_var, gpui_macros, refineable, sandbox, settings_macros, telemetry, xtask |
| L2 | 5 | gpui_windows, language_core, perf, zed_env_vars, zlog |
| L3 | 3 | gpui_platform, util_macros, ztracing |
| L4 | 3 | eval_utils, sum_tree, util |
| L5 | 5 | grammars, http_client, paths, settings_json, sqlez |
| L6 | 4 | deepseek, language_model_core, reqwest_client, sqlez_macros |
| L7 | 4 | gpui, llama_cpp, lmstudio, open_ai |
| L8 | 16 | askpass, assets, credentials_provider, fuzzy, gpui_tokio, gpui_wgpu, input_latency_ui, menu, mermaid_render, release_channel, rope, rpc, settings_content, syntax_theme, watch, zed_actions |
| L9 | 13 | cli, db, fuzzy_nucleo, language_model, lsp, migrator, node_runtime, streaming_diff, system_specs, task, text, theme, zed_credentials_provider |
| L10 | 5 | component, crashes, file_icons, git, session |
| L11 | 2 | fs, ui_macros |
| L12 | 3 | agent_skills, settings, snippet_provider |
| L13 | 8 | audio, context_server, feature_flags, git_hosting_providers, ollama, theme_settings, vim_mode_setting, zlog_settings |
| L14 | 4 | language, terminal, theme_importer, ui |
| L15 | 6 | buffer_diff, language_detection, prettier, prompt_store, ui_input, worktree |
| L16 | 2 | client, multi_buffer |
| L17 | 3 | dap, edit_prediction_types, language_models |
| L18 | 2 | dap_adapters, json_schema_store |
| L19 | 1 | languages |
| L20 | 1 | markdown |
| L21 | 1 | project |
| L22 | 3 | action_log, agent_settings, edit_prediction_context |
| L23 | 2 | acp_thread, workspace |
| L24 | 12 | agent_servers, auto_update, breadcrumbs, command_palette_hooks, edit_prediction, etw_tracing, feedback, install_cli, notifications, platform_title_bar, svg_preview, ui_prompt |
| L25 | 2 | editor, miniprofiler_ui |
| L26 | 15 | activity_indicator, agent, component_preview, debugger_tools, diagnostics, edit_prediction_ui, go_to_line, image_viewer, inspector_ui, journal, language_onboarding, language_tools, markdown_preview, picker, terminal_view |
| L27 | 15 | auto_update_ui, call_hierarchy, command_palette, encoding_selector, git_ui_core, line_ending_selector, onboarding, open_path_prompt, picker_preview, repl, settings_profile_selector, tab_switcher, tabular_data_preview, tasks_ui, theme_selector |
| L28 | 11 | debugger_ui, file_finder, language_selector, lsp_locations, outline, project_symbols, recent_projects, search, snippets_ui, toolchain_selector, which_key |
| L29 | 5 | git_ui, keymap_editor, outline_panel, settings_ui, title_bar |
| L30 | 1 | project_panel |
| L31 | 1 | vim |
| L32 | 1 | agent_ui |
| L33 | 2 | acp_tools, sidebar |
| L34 | 1 | zed |

> 读法：层号越大越靠近最终应用；同层之间没有依赖关系。要判断"删掉 X 会不会连带影响"，看第 3 节的入度与各明细表的 Rev 列即可——入度越高（`gpui`/`util`/`settings`/`ui`/`workspace`）改动代价越大。

## 3. 裁剪后的结构异常与残留

1. **`media` 完全孤立**：`crates/media/Cargo.toml:7` 自述 `description = "Bindings to macos media handling APIs for Zed"`，`src/media.rs:6,213,217,222` 与 `src/bindings.rs:6,9` 全部是 `#[cfg(target_os = "macos")]`（连 `build.rs` 亦然）；唯一消费者曾是 `gpui_macos`（已删），现在 Windows 视图与全 target 视图里都没有任何 crate 依赖它，但根 `Cargo.toml:117`（members）与 `Cargo.toml:354`（workspace 依赖）仍声明。→ 纯 mac 残留，可直接删除（含根 manifest 两行）。
2. **`gpui_linux` / `gpui_web` / `gpui_wgpu` 在 Windows 上无消费者**：三者只被 `crates/gpui_platform/Cargo.toml:32`（linux target 段）、`:35`（wasm target 段）与 `crates/gpui_linux/Cargo.toml:62`（可选依赖 `gpui_wgpu`）引用。若目标是**纯 Windows**，这三个 crate 及 `gpui_platform` 中对应 target 段属可删范围（注意 `gpui_platform/Cargo.toml:21-22` 的 `wayland`/`x11` 特性）。
3. **悬空脚本引用**：CI 六个 job 仍执行已删除的 `./script/bundle-mac`（`.github/workflows/release.yml:561,607`、`release_nightly.yml:292,342`、`run_bundling.yml:196,241`）与 `tooling/xtask/src/tasks/workflows/run_bundling.rs:66`。
4. **按用户要求保留的内联 mac 分支**（"嵌在文件里的不许动"）：`crates/gpui_platform/src/gpui_platform.rs:58-99`（仍 `use gpui_macos::…`）、`crates/sandbox/src/sandbox.rs:23-24`（仍 `mod macos_seatbelt;`）、`crates/zed/src/zed.rs:3-11`（仍 `mod mac_only_instance;` 且该目录已删，`crates/zed/src/main.rs:369` 也 `use zed::mac_only_instance::*`）、`crates/zed/build.rs:5-19`（仍打印 macOS 链接参数）、`.cargo/config.toml:21`（`MACOSX_DEPLOYMENT_TARGET`）。同类的 manifest 级残留仍未清：`crates/cli/Cargo.toml:50-53`（core-foundation / core-services / plist）、`crates/zed/Cargo.toml:117-122`（`gpui_platform` 的 wayland/x11 特性）、`:233-239`（ashpd）、`:261-291`（osx bundle 元数据）。
5. **开发/工具类 crate 的可选性**：`theme_importer`（独立 `src/main.rs`，无 Cargo 消费者，仅被 `script/import-themes:3` 用 `cargo run -p theme_importer -- "$@"` 调用）、`component_preview`（`crates/zed/Cargo.toml:96` 依赖 + `examples/component_preview.rs` 预览入口）、`xtask`（`tooling/xtask` 构建/CI 任务）、`auto_update_helper`（`[[bin]]`：由 `script/bundle-windows.ps1:118-119` 打包、运行时被 `crates/auto_update/src/auto_update.rs:1152` 按路径 `auto_update_helper.exe` 调用，非 Cargo 依赖）——都不是主链路，但除 `theme_importer` 外都有明确调用方。
6. **`wiki/Module-Index.md` 已明显过时**：该页自述枚举"`crates/` 下全部约 240 个 crate"，仍列出大量已删除的 crate 目录；当前实际是 **198 个成员**（`crates/` 下 194 个直接成员目录 + 嵌套的 `crates/refineable/derive_refineable` + `tooling/{compliance,perf,xtask}` 3 个；`tooling/lints` 非成员）。用脚本把 wiki 里的反引号标识符与当前成员集比对后，**wiki 仍提到但磁盘上已不存在**的 crate 家族包括：
   - AI provider：`anthropic`、`google_ai`、`mistral`、`open_router`、`opencode`、`codestral`、`copilot`/`copilot_chat`/`copilot_ui`、`openai_subscribed`、`x_ai`、`bedrock`、`aws_http_client`、`ai_onboarding`、`language_models_cloud`、`cloud_api_client`、`cloud_api_types`、`cloud_llm_client`；
   - 搜索：`web_search`、`web_search_providers`；
   - 协作/语音：`collab`、`collab_ui`、`channel`、`livekit_api`、`livekit_client`、`share_screen`；
   - 扩展系统：`extension_api`、`extension_host`、`extension_cli`、`extensions_ui`、`language_extension`、`debug_adapter_extension`、`theme_extension`；
   - 工具/基准：`eval_cli`、`edit_prediction_cli`、`project_benchmarks`、`fs_benchmarks`、`editor_benchmarks`、`worktree_benchmarks`、`docs_preprocessor`；
   - 平台：`gpui_apple`、`gpui_macos`。
   本报告即当前口径的权威快照，wiki 索引需要重写或加"已裁剪"标注。

## 4. 明细表索引

| 文件 | 分组 | crate 数 |
| --- | --- | --- |
| `01-platform.md` | 平台与渲染（GPUI、窗口后端、GPU、音频、资源） | 13 |
| `02-foundation-core.md` | 基础库（数据结构、路径、错误/日志/追踪、调度、宏） | 31 |
| `03-app-tooling.md` | 应用入口与构建工具（zed、cli、workspace、xtask…） | 7 |
| `04-network-ops.md` | 网络、更新、遥测/崩溃、RPC | 21 |
| `05-project-model.md` | 项目/工作区模型、设置、Git、语言生态 | 27 |
| `06-editor-kernel.md` | 编辑器内核、LSP/DAP、终端 | 30 |
| `07-ui-framework.md` | UI 框架与通用组件、主题、菜单、图标 | 21 |
| `08-ui-panels.md` | 面板与视图插件、Vim、调试/剖析 UI | 20 |
| `09-ai-agent.md` | AI provider、Agent、编辑预测、Markdown 渲染 | 28 |
| 合计 | | **198** |
