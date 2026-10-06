# Zed 项目 Wiki 首页

> 本 Wiki 面向源码阅读/二次开发者，聚焦 **模块职责 → 关键函数功能 → 调用流程**。所有符号名均来自仓库真实代码（`crates/`）。

## 1. Zed 是什么

Zed 是一款用 **Rust** 编写的高性能代码编辑器 / IDE，核心卖点：

- **自研 GPU 加速 UI 框架 [GPUI](GPUI.md)**：整棵界面树在 GPU 上合成，输入延迟极低。
- **实时协作**：基于 CRDT 的共享项目（[`client`](#)）。原 `collab` 服务端与 `livekit_client`/`call` 语音栈已从本 fork 移除。
- **内置 AI 助手**：对话式 Agent、代码编辑预测（[`agent`](#) / [`language_model`](Agent-and-AI.md) / [`edit_prediction`](Agent-and-AI.md)）。
- **语言支持**：Tree-sitter 语法 + LSP 双轨（[`language`](Language-and-Project.md) / [`languages`](Language-and-Project.md)）。
- **编辑器内核**：Rope 缓冲区、多缓冲 diff（[`editor`](Editor.md) / [`rope`](Editor.md) / [`multi_buffer`](Editor.md)）。

## 2. 仓库结构概览

| 目录 | 说明 |
|---|---|
| `crates/` | 244 个 Cargo workspace 成员（各功能模块） |
| `assets/` | 图标、主题、keymaps、声音等资源 |
| `vendor/` | 离线依赖源（`replace-with = "vendored-sources"`） |
| `docs/` | 面向用户的手册（mdbook，非本 Wiki） |
| `script/` | 构建/发布/CI 脚本 |
| `.cargo/config.toml` | 构建配置、source 替换、rustflags |
| `wiki/` | **本开发者 Wiki**（repo-wiki 格式） |

Cargo workspace 根为 [`Cargo.toml`](../Cargo.toml)：`members` 显式列出各 crate，`default-members = ["crates/zed"]`（最终产物 `zed` 二进制）。

## 3. Wiki 目录

### 总览
- [模块综述（各模块用途速查 + 一键跳转，新人首选）](模块综述.md)
- [Architecture（整体架构与模块分层）](Architecture.md)
- [Module-Index（全量模块索引 / 覆盖矩阵，240+ crate）](Module-Index.md)
- [Startup-Flow（启动函数级调用流程）](Startup-Flow.md)

### 参考手册（Deep Dive · 逐 crate 全 API）
> 参考手册级：每个核心 crate 一页，穷举其真实公开类型/方法与内部流程。
- [GPUI-Deep-Dive（App/Window/Entity/Element 生命周期/样式系统）](GPUI-Deep-Dive.md)
- [Editor-Deep-Dive（editor/display_map/element/LSP 叠加层全子系统）](Editor-Deep-Dive.md)
- [Multi-Buffer-Deep-Dive（Excerpt/坐标 newtype/事务/Anchor）](Multi-Buffer-Deep-Dive.md)
- [Text-Buffer-Deep-Dive（CRDT Buffer/Operation/Patch/Selection/UndoMap）](Text-Buffer-Deep-Dive.md)
- [Rope-Deep-Dive（Chunk/TextSummary/坐标互转全 API）](Rope-Deep-Dive.md)
- [Sum-Tree-Deep-Dive（Item/Summary/Dimension/Cursor/TreeMap）](Sum-Tree-Deep-Dive.md)
- [Language-Deep-Dive（Buffer/Language/Registry/SyntaxMap）](Language-Deep-Dive.md)
- [Project-Deep-Dive（Project/LspStore/GitStore/Worktree 全子 store）](Project-Deep-Dive.md)
- [Workspace-Deep-Dive（Workspace/Pane/Dock/Item 契约/持久化）](Workspace-Deep-Dive.md)
- [Terminal-Deep-Dive（PTY/Alacritty 适配/TerminalElement 渲染）](Terminal-Deep-Dive.md)
- [Debugger-Deep-Dive（dap 传输/Session 状态机/debugger_ui 面板）](Debugger-Deep-Dive.md)
- [Agent-Deep-Dive（NativeAgent/Thread/31 tool/AcpThread/LanguageModel）](Agent-Deep-Dive.md)
- [Markdown-Deep-Dive（parser/markdown.rs 渲染/preview 面板）](Markdown-Deep-Dive.md)
- [Collab-Deep-Dive（历史：Client/call Room/collab 服务器/channel/collab_ui）](Collab-Deep-Dive.md)
- [Panels-Deep-Dive（project_panel/outline_panel/call_hierarchy）](Panels-Deep-Dive.md)
- [Search-Deep-Dive（BufferSearchBar/ProjectSearch/TextFinder/registrar）](Search-Deep-Dive.md)
- [Git-Deep-Dive（git 状态/GitStore·Repository/GitPanel·图/冲突）](Git-Deep-Dive.md)
- [Extension-Deep-Dive（extension_api/宿主类型/ExtensionStore·WasmHost/市场页）](Extension-Deep-Dive.md)
- [Edit-Prediction-Deep-Dive（EditPredictionStore/Delegate 双抽象/zeta・mercury・fim・ollama/context）](Edit-Prediction-Deep-Dive.md)
- [GPUI-Platform-Backends-Deep-Dive（Platform/PlatformWindow 抽象 + macOS・Windows・Linux・wgpu・Web 后端）](GPUI-Platform-Backends-Deep-Dive.md)
- [Picker-Family-Deep-Dive（Picker/PickerDelegate + fuzzy・fuzzy_nucleo 双引擎 + file_finder/project_symbols）](Picker-Family-Deep-Dive.md)
- [Diagnostics-Deep-Dive（Diagnostic/DiagnosticSet → editor 波浪线/块 → 项目/缓冲侧栏）](Diagnostics-Deep-Dive.md)
- [Settings-and-Onboarding-Deep-Dive（settings_content 模型/SettingsWindow/Onboarding 引导）](Settings-and-Onboarding-Deep-Dive.md)
- [Vim-Deep-Dive（Mode/Operator/Motion/文本对象/Helix 增强/Neovim 差分测试）](Vim-Deep-Dive.md)
- [Call-and-Voice-Deep-Dive（历史：音视频 RTC/livekit_client·mock；audio 管线仍在）](Call-and-Voice-Deep-Dive.md)
- [Task-System-Deep-Dive（TaskTemplate/变量替换/Inventory/TasksModal/终端·DAP 分发）](Task-System-Deep-Dive.md)
- [Extension-Host-Deep-Dive（Wasmtime 组件宿主/版本化 WIT/CapabilityGranter/Headless）](Extension-Host-Deep-Dive.md)
- [GPUI-Macros-and-Utilities-Deep-Dive（proc-macro/Refineable 样式级联/SharedString/Tokio 桥/gpui_util）](GPUI-Macros-and-Utilities-Deep-Dive.md)
- [Model-Providers-Deep-Dive（LanguageModel/Provider 契约 + 逐厂商 client→适配层两层架构/SSE/Bedrock SigV4/云）](Model-Providers-Deep-Dive.md)
- [Persistence-Deep-Dive（sqlez/db 两套迁移栈 + AppMigrator inventory 拓扑排序 + migrator 配置 JSON 迁移）](Persistence-Deep-Dive.md)
- [Network-HTTP-Deep-Dive（HttpClient 契约/ReqwestClient 实现/http_proxy 沙箱代理+DNS 重绑定防护/proxy_handshake 隧道/net Windows UDS/session）](Network-HTTP-Deep-Dive.md)
- [Telemetry-and-Updates-Deep-Dive（telemetry 事件门/telemetry_events schema/crashes Sentry/release_channel 通道/AutoUpdater 跨平台安装/auto_update_ui/feedback）](Telemetry-and-Updates-Deep-Dive.md)
- [Agent-Skills-and-Context-Deep-Dive（agent_skills 作域优先级/prompt_store 模板引擎+用户提示/context_server MCP 客户端与内建服务端）](Agent-Skills-and-Context-Deep-Dive.md)
- [UI-Primitives-Deep-Dive（ui crate：styles 主题映射/traits 交互链/51 个 RenderOnce 组件/utils）](UI-Primitives-Deep-Dive.md)
- [Diff-Engines-Deep-Dive（buffer_diff 增量快照+hunk 坐标映射+DiffOperations stage/streaming_diff 字符行级流式差分）](Diff-Engines-Deep-Dive.md)
- [Language-Tooling-Deep-Dive（languages 内置语言注册/language_tools 语法树·高亮树·LSP日志·key context 调试视图）](Language-Tooling-Deep-Dive.md)
- [Edit-Prediction-UI-CLI-Deep-Dive（types 契约/context BM25·gitlog 检索/ui 按钮·评分弹窗/cli 离线评测）](Edit-Prediction-UI-CLI-Deep-Dive.md)
- [Misc-Preview-Items-Deep-Dive（markdown_preview 双向同步/image_viewer 缩放持久/component_preview 预览簿）](Misc-Preview-Items-Deep-Dive.md)
- [Keymap-and-Navigation-Deep-Dive（keymap_editor 可视改键+冲突检测/which_key 键位提示/file_finder 模糊导航）](Keymap-and-Navigation-Deep-Dive.md)
- [Copilot-Stack-Deep-Dive（copilot LSP server+编辑预测 provider/copilot_chat OAuth+模型清单+流式/copilot_ui 登录）](Copilot-Stack-Deep-Dive.md)
- [CLI-and-Packaging-Deep-Dive（cli CliRequest/IPC 握手/Bundle 装配/install_cli 脚本安装）](CLI-and-Packaging-Deep-Dive.md)
- [Selectors-and-Themes-Misc-Deep-Dive（theme 注册/外观/图标主题 + language·encoding·line_ending·toolchain·settings_profile·theme 选择器 + file_icons）](Selectors-and-Themes-Misc-Deep-Dive.md)
- [Tooling-Evals-Benchmarks-Deep-Dive（eval_cli Rust+zed_eval Python 编排/eval_utils 契约/criterion 基准/edit_prediction_metrics 打分）](Tooling-Evals-Benchmarks-Deep-Dive.md)
- [Web-Search-Deep-Dive（web_search provider 抽象+注册中心/web_search_providers 云实现/cloud_llm schema/agent WebSearchTool/授权页）](Web-Search-Deep-Dive.md)

### UI 框架与交互
- [GPUI（UI 框架：渲染三阶段 / 事件 / Action 分发）](GPUI.md)
- [GPUI-Internals（事件循环 / 布局 / 绘制 / 实体系统）](GPUI-Internals.md)
- [GPUI-Platform-Backends（macOS/Windows/Linux/Web 后端与渲染器）](GPUI-Platform-Backends.md)
- [Workspace-Pane-Dock（应用外壳 / Item 系统）](Workspace-Pane-Dock.md)
- [Picker-and-Commands（命令面板 / 模糊匹配）](Picker-and-Commands.md)
- [Misc-Items-and-Selectors（图片/SVG/表格预览 / 各选择器 / 导航）](Misc-Items-and-Selectors.md)

### 编辑内核
- [Editor（编辑器：输入 → 缓冲区 → 渲染）](Editor.md)
- [Editing-Deep-Dive（多光标 / 撤销 / 片段 / 多缓冲）](Editing-Deep-Dive.md)
- [Data-Structures（sum_tree / rope / diff 底座）](Data-Structures.md)
- [Vim-and-Key-Input（Vim/Helix 模式 / which-key / 键位编辑器）](Vim-and-Key-Input.md)
- [Search（缓冲查找 / 全局搜索 / 替换）](Search.md)
- [Markdown-and-Preview（解析 / 渲染 / 实时预览）](Markdown-and-Preview.md)

### 语言与项目
- [Language & Project（LSP / worktree / 语言检测）](Language-and-Project.md)
- [LSP-Features（语言服务器 / 诊断 / 大纲 / 调用层次）](LSP-Features.md)
- [Project-Panel-and-FS（文件系统 / Worktree / 项目树）](Project-Panel-and-FS.md)
- [Git-Integration（GitRepository / GitStore / git_ui）](Git-Integration.md)
- [Terminal（PTY / Alacritty / GPUI 渲染）](Terminal.md)

### 智能与协作
- [Agent & AI（模型接入 / 对话请求流程）](Agent-and-AI.md)
- [Model-Providers（全厂商 Provider / 注册表 / 鉴权）](Model-Providers.md)
- [Edit-Prediction（Zeta 编辑预测全链）](Edit-Prediction.md)
- [Collaboration & Call（历史：RPC / livekit / audio 与 mock 机制）](Collaboration-and-Call.md)
- [Channels-and-Collab-UI（频道仍在；协作侧栏 / 通话通知已移除）](Channels-and-Collab-UI.md)

### 基础设施与工具
- [Persistence（db / sqlez / migrator）](Persistence.md)
- [Network-And-HTTP（http_client / reqwest / 受控代理）](Network-And-HTTP.md)
- [Telemetry-Logging-Updates（遥测 / zlog / ztracing / 自动更新 / 反馈）](Telemetry-Logging-Updates.md)
- [Tooling-Evals-Utilities（评测 / 基准 / 基础工具库）](Tooling-Evals-Utilities.md)

### 扩展与配置
- [Extension-System（Wasm 扩展 / wasmtime / 能力注册）](Extension-System.md)
- [Settings-and-Themes（分层设置 / 主题注册）](Settings-and-Themes.md)

### 高级特性
- [Debugger（DAP 协议 / 会话状态 / 调试 UI）](Debugger.md)
- [Tasks-and-Tooling（任务运行 / REPL / 日志 / 剖析）](Tasks-and-Tooling.md)

### 构建与平台
- [Building on Windows（MSVC 构建 / spectre 裁剪）](Building-on-Windows.md)

## 4. 快速开始

```bash
# macOS / Linux
cargo run                                    # 编译并启动 zed（default-members = crates/zed）

# 只构建某个 crate
cargo build -p gpui
cargo test -p editor
```

Windows 桌面版需 **MSVC** 工具链（非 GNU），详见 [Building on Windows](Building-on-Windows.md)。

## 5. 阅读建议

- 想理解"程序如何跑起来" → 先看 [Startup-Flow](Startup-Flow.md)。
- 想理解"界面如何绘制/响应点击" → 先看 [GPUI](GPUI.md)，再看 [Editor](Editor.md)。
- 关注协作 / 音频 / 依赖裁剪 → 看 [Collaboration & Call](Collaboration-and-Call.md) 与 [Building on Windows](Building-on-Windows.md)。
