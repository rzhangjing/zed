# 汇总：依赖关系规律、裁剪候选与结构风险

## 11.1 依赖关系的六条结构性规律

1. **严格单向的 35 层 DAG**：无环（出现的自环全部由 dev-dependency 造成，如 `project→project`、`fs→fs`、`sum_tree→zlog`）。底层 L0 是 26 个零内部依赖叶子（`clock`、`collections`、`util`、`path`、`task`、`zlog`、`icons`、`proto`、`net` 等），顶层 L34 是 `zed`。
2. **技术基座**：`gpui`(RevN=134)、`util`(104)、`collections`(78) 被全仓复用；其上依次是 `settings`(78)、`ui`(63)、`workspace`(63)、`language`(60)、`project`(60)、`theme`(49)、`editor`(47)。
3. **用反转依赖打破环**（四处典型）：`theme` ↔ `theme_settings` 靠 `crates/theme/src/theme_settings_provider.rs:9-24` 的 trait；`ui_input` ↔ `editor` 靠 `ERASED_EDITOR_FACTORY`（`crates/ui_input/src/ui_input.rs:44` 定义、`crates/editor/src/editor.rs:403` 注册、`crates/picker/src/head.rs:23` 取用）；`git_ui_core` 用 `AnyView` + builder（`crates/git_ui_core/src/git_ui_core.rs:18-22`）；`editor` ← `breadcrumbs` 用渲染回调注入（`crates/editor/src/editor.rs:361`）。
4. **三个单点聚合器**：`project`(RevN=60) 聚合 worktree/buffer/git/lsp/task/terminal/prettier/toolchain 各 store；`workspace`(63) 是所有视图/面板的编排层（`crates/workspace/src/workspace.rs` 19,559 行）；`zed`(InN=119, RevN=0) 是唯一最终二进制（另有 `visual-tests` feature 下的 `zed_visual_test_runner`）。
5. **命名与实际必须核对**：`action_log` 是 AI 工具动作记录/撤销（不是 git 历史）；`debugger_tools` 是 DAP 日志工具栏 UI（不是 CLI）；`sidebar` 不是 Panel，实现的是 `workspace::multi_workspace::Sidebar`（`crates/workspace/src/multi_workspace.rs:119`，`:385` 注册）；`component_preview` 才是预览工具，`picker_preview` 是运行时依赖（`crates/file_finder/src/file_finder.rs:178`）；`language_model` 是 trait + GPUI 全局注册表，`language_models` 才是 provider 集合。
6. **总览文件里也验证过的一点**：只有 `windows_resources` 是纯 build-dependency；**没有任何 crate 只被 dev-dependencies 依赖**。

## 11.2 完全孤立 / 工具类（裁剪风险最低）

| crate | 证据 | 说明 |
| --- | --- | --- |
| `media` | 全仓仅根 `Cargo.toml:117`(members)、`:354`(workspace dep) 提及；`media::` 在 `*.rs` 零命中；非 macOS 下 `core_media`/`core_video` 被 `#[cfg(target_os="macos")]` 全部排除（`crates/media/src/media.rs:6,213`），`build.rs` 非 mac 是空 `main()` | 真孤儿且 100% 是 mac 绑定；删除可顺带清 core-foundation/core-video/metal/objc/foreign-types 与无条件的 build-dep `bindgen` |
| `theme_importer` | 无 `[lib]`、零 crate 依赖，仅 `script/import-themes:3` 调用 | CLI 工具，非运行时 |
| `xtask` | 只依赖 `compliance`；`tooling/xtask` | 纯开发/CI 工具 |
| `compliance` | 只有 `xtask` 消费 | 许可证/合规检查 |
| `gpui_linux` / `gpui_web` / `gpui_wgpu` | Windows 视图 Rev=0，消费者全在非 Windows target 段：`crates/gpui_platform/Cargo.toml:32`(cfg(linux/freebsd))、`:35`(cfg(wasm))、`crates/gpui_linux/Cargo.toml:62`(optional)、`crates/gpui_web/Cargo.toml:23` | Windows 全走 `gpui_windows` 的 DirectX/DirectWrite；若要 Windows-only 精简，三者 + 对应 target 段可联动删除 |
| `audio`（不可整删，但有死代码） | crate 仍活跃（`settings_ui` 音频设置页、`zed` 的 `audio::init`）；`Sound` 8 个变体里 7 个（Joined/GuestJoined/Leave/Mute/Unmute/StartScreenshare/StopScreenshare）零调用点 | 协作轴删除后的死变体，仅 `Sound::AgentDone` 被 `crates/agent_ui/src/agent_panel.rs:2890` 与 `conversation_view.rs:2905` 使用（`crates/audio/src/audio.rs:21-45`） |

## 11.3 空转 / 死代码残留（功能已删，脚手架还在）

- **遥测整条链已空转**：全仓**没有任何 `telemetry::init(` 调用点**（已复核），所以 `telemetry::send_event` 的队列恒空，**49 个文件里的 `telemetry::event!` 全部是 no-op**；`telemetry_events::EventRequestBody` 除定义处外无构造点。要删就删 `telemetry`(66 行)+`telemetry_events`，代价是处理 49 个文件的宏调用。
- **`ztracing` / `ztracing_macro` 默认构建是空壳**：instrument 直通、span/event 宏丢弃；`ztracing_macro` 只有 7 行恒等宏，却有 15 个消费者。
- **`feature_flags` 的 flag 全是已删轴残留**：notebooks/panic/acp-beta/diff-review/create-thread-tool/lsp-tool；`clock` 仍留 `ReplicaId::AGENT`/`FIRST_COLLAB_ID`（协作/Agent 轴遗留常量）。
- **`streaming_diff`**：唯一生产消费者是 `crates/agent/src/edit_session.rs:25,392,613`；**`action_log`**：AI 工具动作记录。二者只在保留 AI 轴时才有意义。
- **界面死代码**：`tabular_data_preview` 的 `dev-tools` feature 全仓无启用点（性能覆盖层）；`input_latency_ui` 没有 `init()`（只有 5 分钟遥测与 dev 动作）；`which_key` 默认关闭（`enabled=false`）；`crates/onboarding/src/basics_page.rs:599` 的 `render_zed_agent_button(_user_store,_cx)` 两个参数都没用（登录轴删除后的空壳）。
- **类型层残留**：`language_model_core::provider` 仍留 anthropic/google/x_ai/zed 云常量；`agent` 的 eval fixture 仍引用已删的 `inline_completion::EditPredictionProvider`/Copilot。
- **声明但零引用**：`crates/diagnostics/Cargo.toml:16` 的 `agent_settings`、`crates/recent_projects/Cargo.toml:34` 的 `open_path_prompt`。
- **`open_ai/src/batches.rs` 零消费者**：全 workspace 只有 `crates/open_ai/src/open_ai.rs:1` 的 `pub mod batches;` 引用它（OpenAI Batch API）→ 可整文件+module 声明删除，是本组最明确的候选。
- **`eval_utils` 实为测试专用**：CSV 记 2 个消费者（`agent`、`agent_ui`），但分别是 dev-dep（`crates/agent/Cargo.toml:95`）与 optional/`test-support`（`crates/agent_ui/Cargo.toml:50,123`）→ 运行时无人依赖。
- **未使用参数 / 空壳依赖**：`EditPredictionStore::new(_client: Arc<Client>, _user_store: Entity<UserStore>, …)`（`crates/edit_prediction/src/edit_prediction.rs:735`）两个参数都没用，且 crate 内已无 `db::` 引用 → `client`/`db` 依赖近乎空壳；`language_models::init(_user_store)`（`crates/language_models/src/language_models.rs:123`）同样未用。
- **已删 provider 的硬编码残留**：`crates/agent_ui/src/model_selector.rs:777-800` 仍列 `("Recommended", vec!["zed/claude", "anthropic/claude"])` 与 `("Antropic", …)`；`crates/language_model_core/src/provider.rs:3-25` 仍存 anthropic/baseten/google/x_ai/zed.dev 常量。
- **依赖面泄漏**：`crates/context_server` 把 `http_client` 的 `features=["test-support"]` 写在 `[dependencies]` 里（测试支持进入运行时依赖）。

## 11.4 结构异常与构建风险（建议修，与裁剪无关）

1. **四处 macOS 死分支**（将来为非 Windows 目标构建会立即编译失败，因为对应文件/crate 已删）：
   - `crates/gpui_platform/src/gpui_platform.rs:58-61`、`:86-90` 仍 `#[cfg(target_os = "macos")]` 引用 `gpui_macos::MacPlatform` / `gpui_macos::metal_renderer::MetalHeadlessRenderer`，而该 crate 已无 mac target 段；
   - `crates/sandbox/src/sandbox.rs:23-24` 仍 `mod macos_seatbelt;`（文件已删；Windows 走 `windows_wsl.rs`、Linux 走 `linux_bubblewrap.rs`，均不受影响）；
   - `crates/zed/src/zed.rs:3-4` 仍 `mod mac_only_instance;`（文件已删；`crates/zed/src/main.rs:369` 仍 use）；
   - `crates/zed/build.rs:5-19` 仍打 macOS 链接参数。
2. **清单/元数据悬空**：`tooling/xtask/src/tasks/workflows/run_bundling.rs:66` 与它生成的 `.github/workflows/release.yml:561`、`release_nightly.yml:292` 仍调已删的 `./script/bundle-mac`；`crates/cli/Cargo.toml:50-53`、`crates/zed/Cargo.toml:117-122,233-239,261-291` 仍留 mac 依赖与 osx bundle 元数据；`crates/time_format/Cargo.toml` 仍留 core-foundation；`crates/install_cli` 的安装路径仍是 macOS `osascript` 符号链接。
3. **隐式耦合**：`crates/gpui_macros/src/bench.rs:96,98,139,141` 的 `quote!` 展开代码硬编码 `gpui_platform::current_headless_renderer()` / `current_platform()`，但 `gpui_macros` 的 `Cargo.toml` 并未声明该依赖（靠 `gpui` 的 dev-dep 满足）。重构/裁剪 `gpui_platform` 时不要漏。
4. **文档过时**：`wiki/Module-Index.md` 自述约 240 个 crate，与当前 198 个成员比对出 55 个已不存在的标识符（AI provider 家族、web_search、协作 livekit/collab/channel、扩展系统 extension_*、基准工具、gpui_macos/gpui_apple 等）。
5. **未验证项**：本次未跑 `cargo test`；此前裁剪遗留的迁移 `rename_web_search_to_search_web` 会把设置改写成不存在的工具名。

## 11.5 各组主线（一句话）

- **Platform(13)**：`gpui` 框架 + Windows 唯一后端 `gpui_windows`（DirectX/DirectWrite）；`gpui_platform` 按 target 分发；`gpui_macros`/`gpui_shared_string`/`gpui_util`/`gpui_tokio` 是支撑层。
- **Foundation-core(31)**：零依赖原语 → 单点工具 → 组合基建 → 领域层；`util`/`collections` 是枢纽。
- **App/tooling(7)**：`zed` 二进制（InN=119）+ `workspace` 视图编排（19,559 行）+ `xtask`/`cli`/`assets`/`install_cli`/`zed_credentials_provider`。
- **Network/ops(21)**：`http_client` 是 29 个 crate 的网络基座；`client` 已无连接/登录（`Peer::new(0)` + 恒 `ConnectionError`，`crates/client/src/client.rs:206-213`、`:525-531`），只剩 `http_client()`/`credentials_provider()`/`telemetry()`/handler 表；`crashes` 是保留的真实上报面；`auto_update` 三件套链路完整（helper 用 Restart Manager + 可回滚 Job）。
- **Project-model(27)**：settings 单向四层 `settings_json→settings_content→settings`（`settings_store.rs:795` 调 `migrator::migrate_settings`）；`project` 聚合器；`db` 是 sqlez SQLite 层（与 project/worktree 无依赖）。
- **Editor-kernel(30)**：数据层 `rope→text→multi_buffer`；`editor` 是 UI+编排层（不自存文本）；LSP = 协议在 `lsp`、编排在 `project`、UI 分散；DAP = `dap` + `dap_adapters` + `debugger_tools`；`terminal→terminal_view→repl`。
- **UI-framework(21)**：theme 体系（trait 反转）+ picker/菜单/通知/图标/标题栏 + `ui`/`ui_input`/`ui_prompt` 组件基座。
- **UI-panels(20)**：只有 2 个真 Panel（`outline_panel`、`debugger_ui::DebugPanel`，在 `crates/zed/src/zed.rs:767-771` 构造、`:789-793` 挂载），其余是 Item/ModalView；`vim` + `vim_mode_setting`。
- **AI-agent(28)**：provider 四层 = `language_model_core`（协议）→ `language_model`（trait+注册表）→ `language_models`（注册/设置，`api_compatible.rs` 抽共用 UI）→ 5 个线协议 crate（deepseek 云端 HTTP、ollama:11434、lmstudio:1234、llama_cpp:8080、open_ai 兼容）；agent 双轨（内置 `agent` + 外部 ACP CLI `agent_servers`，都实现 `acp_thread::AgentConnection`）；编辑预测六件套（`edit_prediction`/`_context`/`_metrics`/`_types`/`_ui`/`zeta_prompt`）；`markdown`/`mermaid_render`/`markdown_preview` 不是 agent 专属（`markdown` RevN=14）。

## 11.6 结论

198 个 crate 全部处于依赖图内；**真正"零消费者、可整删"的只有 `media`**（且它已经是空壳）。`theme_importer`、`xtask`、`compliance` 是工具类；`gpui_linux`/`gpui_web`/`gpui_wgpu` 是"仅其它平台使用"（Windows-only 精简时可联动删除）。其余体积真正来自**已删功能的脚手架残留**（遥测全链空转、`ztracing` 空壳、`feature_flags`/`ReplicaId` 常量、`Sound` 死变体、mac 死分支与悬空脚本引用），这些是"进一步瘦身"最安全的目标；而按功能轴继续删 crate（编辑器/项目模型/GUI 面板）收益大但会直接砍功能。
