# Module Index（全量模块索引与覆盖矩阵）

> 本表枚举 `crates/` 下**全部约 240 个 crate**，按模块族归类，标注职责与对应详解页。目标：任何模块都能从这里定位到文档；`✅` 已有专页，`🔧` 已并入相关页，`⏳` 待深挖（见文末计划）。

> 📘 **参考手册（Deep Dive）层**：针对最核心的 crate，已提供"逐类型/逐函数"的参考手册页（公开 API 全量 + 内部流程），与下方按族的概览页互补：
> [Sum-Tree](Sum-Tree-Deep-Dive.md) · [Rope](Rope-Deep-Dive.md) · [Text](Text-Buffer-Deep-Dive.md) · [Multi-Buffer](Multi-Buffer-Deep-Dive.md) · [Editor](Editor-Deep-Dive.md) · [GPUI](GPUI-Deep-Dive.md) · [Language](Language-Deep-Dive.md) · [Project/Worktree](Project-Deep-Dive.md) · [Workspace](Workspace-Deep-Dive.md)
> 📗 **第二批**：[Terminal](Terminal-Deep-Dive.md) · [Debugger/DAP](Debugger-Deep-Dive.md) · [Agent](Agent-Deep-Dive.md) · [Markdown](Markdown-Deep-Dive.md) · [Collab](Collab-Deep-Dive.md) · [Panels](Panels-Deep-Dive.md)
> 📙 **第三批**：[Search](Search-Deep-Dive.md) · [Git](Git-Deep-Dive.md) · [Extension](Extension-Deep-Dive.md)
> 📘 **第四批**：[Edit-Prediction](Edit-Prediction-Deep-Dive.md)
> 🌐 **第五批**：[GPUI-Platform-Backends](GPUI-Platform-Backends-Deep-Dive.md)
> 📚 **第六批**：[Picker-Family](Picker-Family-Deep-Dive.md) · [Diagnostics](Diagnostics-Deep-Dive.md) · [Settings&Onboarding](Settings-and-Onboarding-Deep-Dive.md)
> ⌨️ **第七批**：[Call&Voice](Call-and-Voice-Deep-Dive.md) · [Task-System](Task-System-Deep-Dive.md)
> 🧩 **第八批**：[Extension-Host](Extension-Host-Deep-Dive.md)（extension_host 运行时内核）
> ⚙️ **第九批**：[GPUI-Macros&Util](GPUI-Macros-and-Utilities-Deep-Dive.md) · [Model-Providers](Model-Providers-Deep-Dive.md)
> 🗄️ **第十批**：[Persistence](Persistence-Deep-Dive.md) · [Network-HTTP](Network-HTTP-Deep-Dive.md)
> 📡 **第十一批**：[Telemetry&Updates](Telemetry-and-Updates-Deep-Dive.md) · [Agent-Skills&Context](Agent-Skills-and-Context-Deep-Dive.md)
> 🎨 **第十二批**：[UI-Primitives](UI-Primitives-Deep-Dive.md) · [Diff-Engines](Diff-Engines-Deep-Dive.md)
> 🔤 **第十三批**：[Language-Tooling](Language-Tooling-Deep-Dive.md) · [Edit-Prediction-UI-CLI](Edit-Prediction-UI-CLI-Deep-Dive.md)
> 🔒 **第十四批**：[Misc-Preview-Items](Misc-Preview-Items-Deep-Dive.md)
> ⌨️ **第十五批**：[Keymap&Navigation](Keymap-and-Navigation-Deep-Dive.md) · [Copilot-Stack](Copilot-Stack-Deep-Dive.md)
> 🎨 **第十六批**：[CLI&Packaging](CLI-and-Packaging-Deep-Dive.md) · [Selectors&Themes](Selectors-and-Themes-Misc-Deep-Dive.md)
> 🧪 **第十七批**：[Tooling&Evals&Benchmarks](Tooling-Evals-Benchmarks-Deep-Dive.md)

## 1. 应用入口与打包
| Crate | 职责 | 覆盖 |
|---|---|---|
| `zed` | 主二进制入口、`main.rs`/`bin`、启动初始化 | ✅ [Startup-Flow](Startup-Flow.md) |
| `cli` | `zed` 命令行（打开、安装） | 🔧 [Building-on-Windows](Building-on-Windows.md) / **📘[Deep](CLI-and-Packaging-Deep-Dive.md)** |
| `install_cli` | `zed install` 扩展安装 CLI | **📘[Deep](CLI-and-Packaging-Deep-Dive.md)** |
| `assets` | 内嵌图标/字体/主题资源生成 | 🔧 [Home](Home.md) |
| `paths` | 配置/数据/临时目录路径解析 | 🔧 [Settings-and-Themes](Settings-and-Themes.md) |
| `zed_actions` | 全局 Action 定义聚合 | 🔧 [Picker-and-Commands](Picker-and-Commands.md) |
| `zed_env_vars` | 环境变量常量集中定义 | ⏳ |
| `sandbox` | OS 级命令沙箱（macOS seatbelt / Linux bwrap / Windows WSL），Agent 命令执行隔离：`Sandbox`(sandbox.rs:485)、`SandboxPolicy`(85)、`SandboxFsPolicy/NetPolicy`(92/110)、`WrappedCommand`(391) | ⏳ |
| `crashes` | 崩溃收集与上报（Sentry） | **📘[Deep](Telemetry-and-Updates-Deep-Dive.md)** |
| `etw_tracing` | Windows ETW 追踪接入 | 🔧 [Building-on-Windows](Building-on-Windows.md) |

## 2. GPUI 核心与平台后端
| Crate | 职责 | 覆盖 |
|---|---|---|
| `gpui` | UI 框架核心：实体/事件/布局/绘制/文本 | ✅ [GPUI](GPUI.md) / [GPUI-Internals](GPUI-Internals.md) / **📘[Deep](GPUI-Deep-Dive.md)** |
| `gpui_platform` | 平台抽象层与派发 | **📘[Deep](GPUI-Platform-Backends-Deep-Dive.md)** |
| `gpui_macos` / `gpui_apple` | macOS 后端（Cocoa/Metal） | **📘[Deep](GPUI-Platform-Backends-Deep-Dive.md)** |
| `gpui_linux` | Linux 后端（X11/Wayland） | **📘[Deep](GPUI-Platform-Backends-Deep-Dive.md)** |
| `gpui_windows` | Windows 后端（DirectX/Win32） | **📘[Deep](GPUI-Platform-Backends-Deep-Dive.md)** |
| `gpui_wgpu` | 跨平台 wgpu 渲染器 | **📘[Deep](GPUI-Platform-Backends-Deep-Dive.md)** |
| `gpui_web` | Web(Wasm) 后端 | **📘[Deep](GPUI-Platform-Backends-Deep-Dive.md)** |
| `gpui_macros` | `derive`/`impl` 宏（Action、Render 等） | **📘[Deep](GPUI-Macros-and-Utilities-Deep-Dive.md)** |
| `gpui_shared_string` | `SharedString` intern 字符串 | **📘[Deep](GPUI-Macros-and-Utilities-Deep-Dive.md)** |
| `gpui_tokio` | GPUI ↔ tokio 桥接 | **📘[Deep](GPUI-Macros-and-Utilities-Deep-Dive.md)** |
| `gpui_util` | GPUI 通用工具 | **📘[Deep](GPUI-Macros-and-Utilities-Deep-Dive.md)** |
| `scheduler` | 确定性任务调度/时钟/执行器（`Clock`、`TestScheduler`、`LocalExecutor`） | ✅ [GPUI-Internals](GPUI-Internals.md) |
| `refineable` | `Refineable` 样式构建器 trait | 🔧 [GPUI](GPUI.md) / **📘[Deep](GPUI-Macros-and-Utilities-Deep-Dive.md)** |

## 3. UI 组件库
| Crate | 职责 | 覆盖 |
|---|---|---|
| `ui` | 共享组件（Button/ListItem/预 styled 元素） | 🔧 [GPUI](GPUI.md) / **📘[Deep](UI-Primitives-Deep-Dive.md)** |
| `ui_macros` | `kw`/`text` 等构造宏 | ⏳ |
| `ui_input` | 通用输入控件 | ⏳ |
| `ui_prompt` | 确认/输入弹窗 | ⏳ |
| `icons` | 图标库与旧 `Icon` 迁移 | 🔧 [Home](Home.md) |
| `menu` | 应用/右键菜单构建 | 🔧 [Startup-Flow](Startup-Flow.md) |
| `title_bar` | 自定义窗口标题栏 | ⏳ |
| `platform_title_bar` | 平台标题栏适配 | ⏳ |
| `component` / `component_preview` | 组件系统与预览浏览器 | **📘[Deep](Misc-Preview-Items-Deep-Dive.md)** |
| `sidebar` | 侧栏工具/导航条 | 🔧 [Workspace-Pane-Dock](Workspace-Pane-Dock.md) |
| `inspector_ui` | UI 检查器 | ⏳ |

## 4. 应用外壳与导航
| Crate | 职责 | 覆盖 |
|---|---|---|
| `workspace` | 应用外壳：Pane/PaneGroup/Dock/Item/持久化 | ✅ [Workspace-Pane-Dock](Workspace-Pane-Dock.md) / **📘[Deep](Workspace-Deep-Dive.md)** |
| `picker` | 通用模糊搜索选择器基座 | ✅ [Picker-and-Commands](Picker-and-Commands.md) / **📘[Deep](Picker-Family-Deep-Dive.md)** |
| `picker_preview` | Picker 项预览 | 🔧 [Picker-and-Commands](Picker-and-Commands.md) |
| `command_palette` | 命令面板 | ✅ [Picker-and-Commands](Picker-and-Commands.md) |
| `command_palette_hooks` | 命令可用性/过滤钩子 | 🔧 [Picker-and-Commands](Picker-and-Commands.md) |
| `tab_switcher` | `cmd-tab` 打开项切换器 | ⏳ |
| `breadcrumbs` | 面包屑路径条 | ⏳ |
| `go_to_line` | 跳转行 | ⏳ |
| `recent_projects` | 最近项目列表 | ⏳ |
| `onboarding` | 首次启动引导页 | **📘[Deep](Settings-and-Onboarding-Deep-Dive.md)** |
| `journal` | 每日/每周笔记入口（`new_journal_entry`） | ✅ [Tasks-and-Tooling](Tasks-and-Tooling.md) |

## 5. 编辑内核
| Crate | 职责 | 覆盖 |
|---|---|---|
| `editor` | 编辑器视图/选择/输入 | ✅ [Editor](Editor.md) / [Editing-Deep-Dive](Editing-Deep-Dive.md) / **📘[Deep](Editor-Deep-Dive.md)** |
| `multi_buffer` | 多 buffer/diff 缝合展示面 | ✅ [Editing-Deep-Dive](Editing-Deep-Dive.md) / **📘[Deep](Multi-Buffer-Deep-Dive.md)** |
| `buffer_diff` | 双 buffer 差异对比（增量 diff 快照） | **📘[Deep](Diff-Engines-Deep-Dive.md)** |
| `streaming_diff` | 流式 diff（AI 逐步应用） | **📘[Deep](Diff-Engines-Deep-Dive.md)** |
| `rope` | Rope 文本序列 | ✅ **📘[Deep](Rope-Deep-Dive.md)** / [Data-Structures](Data-Structures.md) |
| `text` | CRDT 文本内核（`text::Buffer`/`Operation`/`Anchor`） | ✅ [Editing-Deep-Dive](Editing-Deep-Dive.md) / **📘[Deep](Text-Buffer-Deep-Dive.md)** |
| `sum_tree` | 函数式 B+ 树（buffer/worktree 底层） | ✅ **📘[Deep](Sum-Tree-Deep-Dive.md)** |
| `snippet` / `snippet_provider` / `snippets_ui` | 片段语法/来源/管理 UI | 🔧 [Editing-Deep-Dive](Editing-Deep-Dive.md) |
| `input_latency_ui` | 输入延迟可视化 | ⏳ |
| `action_log` | Agent 编辑动作日志（接受/回退） | ✅ [Agent-and-AI](Agent-and-AI.md) |

## 6. 语言 / 语法 / LSP
| Crate | 职责 | 覆盖 |
|---|---|---|
| `language` | `Buffer`、语言、LSP、Tree-sitter 粘合 | ✅ [Language-and-Project](Language-and-Project.md) / **📘[Deep](Language-Deep-Dive.md)** |
| `language_core` | 语言基础类型（无 UI 依赖） | 🔧 [Language-and-Project](Language-and-Project.md) |
| `language_detection` | 语言/编码自动检测 | 🔧 [Language-and-Project](Language-and-Project.md) |
| `languages` | 内置各语言配置（config + queries + LspAdapter 注册） | 🔧 [Language-and-Project](Language-and-Project.md) / **📘[Deep](Language-Tooling-Deep-Dive.md)** |
| `grammars` | Tree-sitter 语法 crate 聚合 | 🔧 [Language-and-Project](Language-and-Project.md) / **📘[Deep](Language-Tooling-Deep-Dive.md)** |
| `syntax_theme` | 语法高亮主题（TextMate→Zed） | ⏳ |
| `lsp` | LSP 协议封装/进程管理（`LanguageServer`） | ✅ [LSP-Features](LSP-Features.md) |
| `lsp_locations` | 定义/引用跳转位置面板 | 🔧 [LSP-Features](LSP-Features.md) |
| `language_extension` | 语言/语法贡献到扩展机制 | 🔧 [Extension-System](Extension-System.md) |
| `language_selector` | 切换语言 picker | **📘[Deep](Selectors-and-Themes-Misc-Deep-Dive.md)** |
| `language_tools` | 语法树/高亮树/LSP 日志/key context 调试视图 | **📘[Deep](Language-Tooling-Deep-Dive.md)** |
| `outline` / `outline_panel` | 符号大纲 | ✅ [LSP-Features](LSP-Features.md) |
| `diagnostics` | 诊断侧栏/内联块渲染 | **📘[Deep](Diagnostics-Deep-Dive.md)** |
| `call_hierarchy` | 调用层次视图 | ✅ [LSP-Features](LSP-Features.md) / **📘[Deep](Panels-Deep-Dive.md)** |
| `project_symbols` | 跨工作区符号搜索 | ✅ [LSP-Features](LSP-Features.md) / **📘[Deep](Picker-Family-Deep-Dive.md)** |
| `prettier` | Prettier 格式化桥 | ⏳ |
| `language_onboarding` | 语言服务器安装引导 | ⏳ |

## 7. 项目 / 文件 / Git
| Crate | 职责 | 覆盖 |
|---|---|---|
| `project` | 项目核心：worktree/buffer/LSP/git/search 汇聚 | ✅ 多页 / **📘[Deep](Project-Deep-Dive.md)** |
| `worktree` | 目录树模型（`Worktree`/`Entry`） | ✅ [Project-Panel-and-FS](Project-Panel-and-FS.md) |
| `fs` | 文件系统抽象与监听 | ✅ [Project-Panel-and-FS](Project-Panel-and-FS.md) |
| `file_icons` | 文件类型→图标映射 | 🔧 [Project-Panel-and-FS](Project-Panel-and-FS.md) / **📘[Deep](Selectors-and-Themes-Misc-Deep-Dive.md)** |
| `project_panel` | 项目树面板 | ✅ [Project-Panel-and-FS](Project-Panel-and-FS.md) / **📘[Deep](Panels-Deep-Dive.md)** |
| `file_finder` | 模糊文件查找（`cmd-p`） | ✅ [Picker-and-Commands](Picker-and-Commands.md) / **📘[Deep](Picker-Family-Deep-Dive.md)** / **📘[Deep](Keymap-and-Navigation-Deep-Dive.md)** |
| `git` | `GitRepository`/`Repository`（shell out git） | ✅ [Git-Integration](Git-Integration.md) / **📘[Deep](Git-Deep-Dive.md)** |
| `git_ui` / `git_ui_core` | Git 面板/blame/冲突解决 UI | ✅ [Git-Integration](Git-Integration.md) / **📘[Deep](Git-Deep-Dive.md)** |
| `git_hosting_providers` | GitHub/GitLab 远程与 PR 链接 | ✅ [Git-Integration](Git-Integration.md) |

## 8. 搜索与模糊匹配
| Crate | 职责 | 覆盖 |
|---|---|---|
| `search` | 缓冲查找 / 全局搜索 UI | ✅ [Search](Search.md) / **📘[Deep](Search-Deep-Dive.md)** |
| `fuzzy` | `Matcher`/`CharBag`/`PathMatch` 模糊匹配 | ✅ [Picker-and-Commands](Picker-and-Commands.md) / **📘[Deep](Picker-Family-Deep-Dive.md)** |
| `fuzzy_nucleo` | nucleo 匹配算法移植 | **📘[Deep](Picker-Family-Deep-Dive.md)** |

## 9. 终端
| Crate | 职责 | 覆盖 |
|---|---|---|
| `terminal` | PTY + Alacritty 引擎（`Terminal`） | ✅ [Terminal](Terminal.md) / **📘[Deep](Terminal-Deep-Dive.md)** |
| `terminal_view` | 终端视图/元素/面板 | ✅ [Terminal](Terminal.md) / **📘[Deep](Terminal-Deep-Dive.md)** |
| `shell_command_parser` | Shell 命令行解析（用于 Agent/任务） | 🔧 [Tasks-and-Tooling](Tasks-and-Tooling.md) |
| `explorer_command_injector` | Windows 资源管理器右键集成 | ⏳ |

## 10. AI / Agent
| Crate | 职责 | 覆盖 |
|---|---|---|
| `agent` | Agent 主逻辑（`AcpThread`/工具/编辑） | ✅ [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Agent-Deep-Dive.md)** |
| `agent_ui` | Agent 界面（会话/消息/工具卡片） | ✅ [Agent-and-AI](Agent-and-AI.md) |
| `agent_settings` | Agent 设置（默认 profile/工具） | 🔧 [Agent-and-AI](Agent-and-AI.md) |
| `agent_servers` | 外部 ACP Agent 服务器接入（Claude Code 等） | ✅ [Agent-and-AI](Agent-and-AI.md) |
| `agent_skills` | 技能（skills）加载与作域优先级 | **📘[Deep](Agent-Skills-and-Context-Deep-Dive.md)** |
| `acp_thread` / `acp_tools` | ACP 协议线程与工具 | ✅ [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Agent-Deep-Dive.md)** |
| `context_server` | MCP / context server | 🔧 [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Agent-Skills-and-Context-Deep-Dive.md)** |
| `prompt_store` | 提示词模板引擎与存储 | 🔧 [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Agent-Skills-and-Context-Deep-Dive.md)** |
| `zeta_prompt` | 编辑预测提示构造 | 🔧 [Agent-and-AI](Agent-and-AI.md) |
| `activity_indicator` | 活动/加载指示（token/请求计数） | ⏳ |
| `ai_onboarding`（已移除 · 历史） | AI 首次配置引导 | ⏳ |

## 11. 模型 Provider 与云 API
| Crate | 职责 | 覆盖 |
|---|---|---|
| `language_model` | `LanguageModel`/`Provider` trait + 注册表 | ✅ [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Agent-Deep-Dive.md)** |
| `language_model_core` | 模型基础类型（事件/用量/工具调用） | ✅ [Agent-and-AI](Agent-and-AI.md) |
| `language_models` | 各 provider 装配 | ✅ [Agent-and-AI](Agent-and-AI.md) |
| `language_models_cloud` | Zed 云托管模型目录 | 🔧 [Agent-and-AI](Agent-and-AI.md) |
| `anthropic`（已移除 · 历史）/`open_ai`/`openai_subscribed`（已移除）/`codestral`（已移除）/`mistral`（已移除）/`deepseek`/`google_ai`（已移除 · 历史）/`bedrock`（已移除）/`x_ai`（已移除）/`ollama`/`open_router`（已移除）/`llama_cpp`/`lmstudio`/`opencode`（已移除） | 各厂商 HTTP 客户端 | ✅ [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Model-Providers-Deep-Dive.md)** |
| `aws_http_client`（已移除 · 历史） | 原 AWS SigV4 签名 HTTP（Bedrock）；该 crate 仅适配传输，SigV4 由 aws-config/aws-sigv4 完成 | **📘[Deep](Model-Providers-Deep-Dive.md)** |
| `cloud_llm_client` / `cloud_api_client` / `cloud_api_types`（已移除 · 历史） | Zed 云 LLM/API 协议 | **📘[Deep](Model-Providers-Deep-Dive.md)** |
| `web_search` / `web_search_providers`（已移除 · 历史） | 联网搜索工具（Agent 用）；随 batch-1 整体删除 | |

## 12. Copilot
| Crate | 职责 | 覆盖 |
|---|---|---|
| `copilot` | GitHub Copilot 补全/预测核心（LSP server） | ✅ [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Copilot-Stack-Deep-Dive.md)** |
| `copilot_chat` | Copilot Chat 模型接入/OAuth/流式 | 🔧 [Agent-and-AI](Agent-and-AI.md) / **📘[Deep](Copilot-Stack-Deep-Dive.md)** |
| `copilot_ui` | Copilot 状态/登录 UI | **📘[Deep](Copilot-Stack-Deep-Dive.md)** |

## 13. 编辑预测（Zeta）
| Crate | 职责 | 覆盖 |
|---|---|---|
| `edit_prediction` | 预测引擎与 `EditPredictionStore` | ✅ [Agent-and-AI](Agent-and-AI.md) **📘[Deep](Edit-Prediction-Deep-Dive.md)** |
| `edit_prediction_types` / `_context` / `_metrics` | 类型/上下文采集/指标 | **📘[Deep](Edit-Prediction-Deep-Dive.md)** / **📘[Deep](Edit-Prediction-UI-CLI-Deep-Dive.md)** / 📘[metrics](Tooling-Evals-Benchmarks-Deep-Dive.md) |
| `edit_prediction_ui` | 状态栏按钮/上下文检视/评分弹窗 | **📘[Deep](Edit-Prediction-UI-CLI-Deep-Dive.md)** |
| `edit_prediction_cli` | 离线评测/prompt 格式化/headless 跑批 CLI | **📘[Deep](Edit-Prediction-UI-CLI-Deep-Dive.md)** |

## 14. 协作 / RPC / 音视频
| Crate | 职责 | 覆盖 |
|---|---|---|
| `rpc` | `Peer`/传输/keepalive | ✅ [Collaboration-and-Call](Collaboration-and-Call.md) |
| `client` | `Client`/用户/项目 RPC | ✅ [Collaboration-and-Call](Collaboration-and-Call.md) / **📘[Deep](Collab-Deep-Dive.md)** |
| `collab`（已移除） | 协作服务器（独立二进制） | ✅ [Collaboration-and-Call](Collaboration-and-Call.md) / **📘[Deep](Collab-Deep-Dive.md)** |
| `collab_ui`（已移除） | 协作面板/频道视图/通知 | ✅ [Channels-and-Collab-UI](Channels-and-Collab-UI.md) / **📘[Deep](Collab-Deep-Dive.md)** |
| `channel`（已移除 · 历史） | 频道存储/共享笔记 | ✅ [Channels-and-Collab-UI](Channels-and-Collab-UI.md) / **📘[Deep](Collab-Deep-Dive.md)** |
| `proto` | protobuf 消息定义 | ✅ [Collaboration-and-Call](Collaboration-and-Call.md) |
| `call`（已移除） | 通话/房间（`ActiveCall`/`Room`/`toggle_mute`/`share_screen`） | ✅ [Collaboration-and-Call](Collaboration-and-Call.md) / **📘[Deep](Call-and-Voice-Deep-Dive.md)** |
| `livekit_api` / `livekit_client`（已移除） | 音视频 SFU（含 mock 路径） | ✅ [Collaboration-and-Call](Collaboration-and-Call.md) / **📘[Deep](Call-and-Voice-Deep-Dive.md)** |
| `audio` | 音效/音频管线（cpal/rodio） | ✅ [Collaboration-and-Call](Collaboration-and-Call.md) / **📘[Deep](Call-and-Voice-Deep-Dive.md)** |
| `notifications` | 系统通知（仅 `status_toast`；`notification_store` 已移除 · 历史） | 🔧 [Channels-and-Collab-UI](Channels-and-Collab-UI.md) |
| `askpass` | 凭据口令弹窗（AskPassSession/PasswordProxy/EncryptedPassword） | ⏳ |
| `credentials_provider` / `zed_credentials_provider` | 凭据存储抽象与实现 | ⏳ |
| `oauth_callback_server` | OAuth 回调本地服务 | ⏳ |

## 15. 调试（DAP）
| Crate | 职责 | 覆盖 |
|---|---|---|
| `dap` | DAP 协议/client/transport | ✅ [Debugger](Debugger.md) / **📘[Deep](Debugger-Deep-Dive.md)** |
| `dap_adapters` | 内置 adapter 定义 | 🔧 [Debugger](Debugger.md) |
| `debug_adapter_extension` | 扩展贡献 adapter | 🔧 [Debugger](Debugger.md) |
| `debugger_ui` | 调试面板/会话 UI | ✅ [Debugger](Debugger.md) / **📘[Deep](Debugger-Deep-Dive.md)** |
| `debugger_tools` | 调试辅助工具 | ⏳ |
| `task` | 任务模板/变量/ResolvedTask/VS Code 格式（`TaskTemplate`/`TaskVariables`/`SpawnInTerminal`） | ✅ [Tasks-and-Tooling](Tasks-and-Tooling.md) / **📘[Deep](Task-System-Deep-Dive.md)** |
| `tasks_ui` | 任务模态/生成 UI（`TasksModal`/`spawn_task_or_modal`；`project` 侧 `Inventory`/`TaskStore`） | ✅ [Tasks-and-Tooling](Tasks-and-Tooling.md) / **📘[Deep](Task-System-Deep-Dive.md)** |

## 16. 设置 / 主题 / 键位
| Crate | 职责 | 覆盖 |
|---|---|---|
| `settings` | 分层设置/`SettingsStore` | ✅ [Settings-and-Themes](Settings-and-Themes.md) **📘[Deep](Settings-and-Onboarding-Deep-Dive.md)** |
| `settings_content` / `settings_json` / `settings_macros` | 设置数据结构/JSON/derive | 🔧 [Settings-and-Themes](Settings-and-Themes.md) / **📘[Deep](Settings-and-Onboarding-Deep-Dive.md)** |
| `settings_ui` | 设置编辑界面 | ✅ [Settings-and-Themes](Settings-and-Themes.md) / **📘[Deep](Settings-and-Onboarding-Deep-Dive.md)** |
| `settings_profile_selector` | 设置 Profile 切换 | 🔧 [Settings-and-Themes](Settings-and-Themes.md) / **📘[Deep](Selectors-and-Themes-Misc-Deep-Dive.md)** |
| `theme` / `theme_settings` | 主题模型与设置 | ✅ [Settings-and-Themes](Settings-and-Themes.md) / **📘[Deep](Selectors-and-Themes-Misc-Deep-Dive.md)** |
| `theme_selector` | 主题切换 picker | 🔧 [Settings-and-Themes](Settings-and-Themes.md) / **📘[Deep](Selectors-and-Themes-Misc-Deep-Dive.md)** |
| `theme_extension` / `theme_importer` | 主题扩展/导入 | ⏳ |
| `time_format` | 时间格式（12/24） | ⏳ |
| `feature_flags` / `feature_flags_macros` | 特性开关 | 🔧 [Agent-and-AI](Agent-and-AI.md) |
| `encoding_selector` / `line_ending_selector` | 编码/换行选择 | **📘[Deep](Selectors-and-Themes-Misc-Deep-Dive.md)** |
| `toolchain_selector` | 语言工具链选择 | **📘[Deep](Selectors-and-Themes-Misc-Deep-Dive.md)** |
| `keymap_editor` | 键位映射编辑器（冲突检测） | **📘[Deep](Keymap-and-Navigation-Deep-Dive.md)** |
| `which_key` | which-key 键位提示浮层（`WhichKeyModal`、`PendingKeystrokesIndicator`） | **📘[Deep](Keymap-and-Navigation-Deep-Dive.md)** |

## 17. 扩展系统
| Crate | 职责 | 覆盖 |
|---|---|---|
| `extension` | `Extension`/`ExtensionHostProxy` trait | ✅ [Extension-System](Extension-System.md) / **📘[Deep](Extension-Deep-Dive.md)** / **🧩[HostDeep](Extension-Host-Deep-Dive.md)** |
| `extension_api` | 扩展 Wasm API（wit/宏） | ✅ [Extension-System](Extension-System.md) / **📘[Deep](Extension-Deep-Dive.md)** |
| `extension_host` | wasmtime 宿主（`WasmHost`/`WasmState`/版本化 WIT/`CapabilityGranter`/`HeadlessExtensionStore`） | ✅ [Extension-System](Extension-System.md) / **🧩[HostDeep](Extension-Host-Deep-Dive.md)** |
| `extension_cli` | 扩展打包/发布 CLI | 🔧 [Extension-System](Extension-System.md) |
| `extensions_ui` | 扩展浏览/管理 UI | ✅ [Extension-System](Extension-System.md) / **📘[Deep](Extension-Deep-Dive.md)** |

## 18. 预览 / 可视化 Item
| Crate | 职责 | 覆盖 |
|---|---|---|
| `repl` | Jupyter 式内核/会话（`ReplStore`/`KernelSpecification`） | ✅ [Tasks-and-Tooling](Tasks-and-Tooling.md) |
| `markdown` / `markdown_preview` | Markdown 渲染与实时预览 | ✅ [Markdown-and-Preview](Markdown-and-Preview.md) / **📘[Deep](Markdown-Deep-Dive.md)** / **📘[Deep](Misc-Preview-Items-Deep-Dive.md)** |
| `mermaid_render` | Mermaid 图渲染 | 🔧 [Markdown-and-Preview](Markdown-and-Preview.md) |
| `image_viewer` | 图片查看 Item（缩放/持久） | **📘[Deep](Misc-Preview-Items-Deep-Dive.md)** |
| `svg_preview` | SVG 预览 | ⏳ |
| `tabular_data_preview` | 表格数据预览（CSV/JSON） | ⏳ |
| `html_to_markdown` | HTML→Markdown（剪贴板/扩展） | ⏳ |

## 19. 基础设施 / 数据 / 网络
| Crate | 职责 | 覆盖 |
|---|---|---|
| `db` / `sqlez` / `sqlez_macros` / `migrator` | SQLite 封装与迁移 | **📘[Deep](Persistence-Deep-Dive.md)** |
| `collections` | `HashMap`/`IndexMap` 等别名与扩展 | 🔧 [Editing-Deep-Dive](Editing-Deep-Dive.md) |
| `path` | 路径与位置类型 | 🔧 [Language-and-Project](Language-and-Project.md) |
| `util` / `util_macros` | 通用工具（结果/网络/进程/宏） | 🔧 [Architecture](Architecture.md) |
| `media` | SIMD/底层原语封装 | ⏳ |
| `clock` | 时间抽象 | 🔧 [Editing-Deep-Dive](Editing-Deep-Dive.md) |
| `node_runtime` | 内嵌 Node.js 下载/运行 | ⏳ |
| `watch` | 文件监视（notify 封装） | 🔧 [Project-Panel-and-FS](Project-Panel-and-FS.md) |
| `net` | Unix socket/TCP 辅助（`UnixListener`/`UnixStream`） | **📘[Deep](Network-HTTP-Deep-Dive.md)** |
| `http_client` / `http_client_tls` / `reqwest_client` | HTTP 客户端抽象与实现 | **📘[Deep](Network-HTTP-Deep-Dive.md)** |
| `http_proxy` / `proxy_handshake` | 代理配置与握手 | **📘[Deep](Network-HTTP-Deep-Dive.md)** |
| `session` | `Session`/`AppSession`：应用会话标识与窗口栈 | **📘[Deep](Network-HTTP-Deep-Dive.md)** |

## 20. 遥测 / 日志 / 更新
| Crate | 职责 | 覆盖 |
|---|---|---|
| `telemetry` / `telemetry_events` | 事件遥测上报 | **📘[Deep](Telemetry-and-Updates-Deep-Dive.md)** |
| `zlog` / `zlog_settings` | 统一日志（tracing subscriber） | ⏳ |
| `ztracing` / `ztracing_macro` | 分布式追踪 | ⏳ |
| `auto_update` / `auto_update_helper` / `auto_update_ui` | 自动更新 | **📘[Deep](Telemetry-and-Updates-Deep-Dive.md)** |
| `feedback` | 反馈/issue 提交（含诊断打包） | **📘[Deep](Telemetry-and-Updates-Deep-Dive.md)** |
| `release_channel` | stable/preview/dev/nightly 渠道 | **📘[Deep](Telemetry-and-Updates-Deep-Dive.md)** |
| `system_specs` | 机器规格采集 | ⏳ |
| `env_var` | 运行时环境变量读写 | ⏳ |
| `json_schema_store` / `schema_generator` | JSON Schema 存储/生成 | ⏳ |
| `docs_preprocessor` | 文档预处理（mdbook） | ⏳ |

## 21. 基准与评测工具
| Crate | 职责 | 覆盖 |
|---|---|---|
| `benchmarks` / `editor_benchmarks` / `fs_benchmarks` / `project_benchmarks` / `worktree_benchmarks` | Criterion 性能基准 | **📘[Deep](Tooling-Evals-Benchmarks-Deep-Dive.md)** |
| `eval_cli` / `eval_utils` | Agent/模型离线评测框架 | **📘[Deep](Tooling-Evals-Benchmarks-Deep-Dive.md)** |

## 22. 深挖批次（已全部完成 ✅）
本索引已**覆盖全部模块的定位**。此前标 `⏳` 的模块族已逐族深挖成页（均先 `grep`/`read` 确证真实符号）：

- ✅ G9 **数据结构底座** → [Data-Structures.md](Data-Structures.md)
- ✅ G3 **模型 Provider 逐个** → [Model-Providers.md](Model-Providers.md)
- ✅ G2 **编辑预测全链** → [Edit-Prediction.md](Edit-Prediction.md)
- ✅ G4 **DB/持久化** → [Persistence.md](Persistence.md)
- ✅ G5 **网络/HTTP/代理** → [Network-And-HTTP.md](Network-And-HTTP.md)
- ✅ G6 **GPUI 平台后端** → [GPUI-Platform-Backends.md](GPUI-Platform-Backends.md)
- ✅ G7 **遥测/日志/更新** → [Telemetry-Logging-Updates.md](Telemetry-Logging-Updates.md)
- ✅ G8 **其余 UI Item 与选择器** → [Misc-Items-and-Selectors.md](Misc-Items-and-Selectors.md)
- ✅ G10 **工具/评测/基准/基础库** → [Tooling-Evals-Utilities.md](Tooling-Evals-Utilities.md)

### 仍可按需继续细分的方向（如后续需要）
- 每个模型 provider crate（`anthropic`（已移除 · 历史）/`ollama`/`bedrock`（已移除）…）的**逐方法** HTTP 细节；
- `languages`/`grammars` 各内置语言的 config/queries 明细；
- `extension_api` 的 wit ABI 与宿主 proxy 全方法表。
