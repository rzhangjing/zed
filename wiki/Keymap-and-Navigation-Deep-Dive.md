# 键位编辑与导航 深入解析（Deep Dive）

> 本页覆盖"键位提示 + 文件/符号导航"两块：`which_key`（which-key 风格的待输入键位提示浮层）、`file_finder`（模糊文件/历史导航 picker）。关联底座：`gpui` 的 `KeyContext`/`BindingObserver`、`settings` 的 `KeymapFile`、`fuzzy`/`picker`/`paths`。

> ⚠️ 已移除（Batch 1–2c）：可视化改键界面 `keymap_editor`（`keymap_editor` crate，含冲突检测与动作名补全）已从本 fork 删除。现在改键直接编辑 `keymap.json`（通过 `zed::OpenKeymapFile` 动作打开）。

## 1. 分层设计

- **键位内核**（在 `gpui`+`settings`）：`Keymap` 把 context→keystroke→Action 编成树；`keystroke` 匹配、`BindingObserver` 捕获一段前缀以判定"是否有后续绑定"——这正是 which_key 的基础。
- **提示浮层** `which_key`：订阅 `cx.on_keystroke`，当用户敲了一个"有后续子绑定"的前缀键，弹出后续可选键与动作名（Vim/Emacs which-key 体验）。
- **导航层** `file_finder`：`Item` + `Picker`，把工作树条目/历史/诊断喂给模糊匹配，Enter 打开文件；与 `project_symbols` 并列构成"跳去某处"家族。

## 2. 类型总览

### which_key

| 符号 | 文件 | 角色 |
| --- | --- | --- |
| `fn init(cx)` | which_key.rs:79 | 订阅全局 keystroke |
| `struct PendingBinding` | which_key.rs:16 (`pub(crate)`) | 待展开的键位前缀 |
| `which_key_modal.rs` | (401 行) | 全屏 which-key 浏览 |
| `pending_keystrokes_indicator.rs` | (896 行) | 底部待输入键位指示 |

### file_finder（file_finder.rs，86KB）

| 符号 | 文件:行 | 角色 |
| --- | --- | --- |
| `struct FileFinder` | :66 | 组装 `Picker` 的 Entity |
| `struct FileFinderDelegate` | :357 | `Picker` 回调（搜索/确认） |
| `enum Event` | :846 | `Opened`/`Disconnected` |

## 3. 核心方法与调用锚点

**which_key 提示（which_key.rs）**
- `init(cx)`(:79) 注册 `BindingObserver`/keystroke 订阅。当一次按键命中"父绑定但仍有子绑定未决"，`PendingBinding`(:16) 记录该前缀，`pending_keystrokes_indicator` 在状态栏列出候选后续键与 `HumanizedActionName`；`which_key_modal` 提供全量浏览。

**file_finder 导航（file_finder.rs）**
- `FileFinder::new` 建 `Picker` 并把 `FileFinderDelegate`(:357) 交给它；`match_strings` 用 `fuzzy` 对工作树 path 打分，`confirm` 发 `Event::Opened`(:846) 让 workspace 打开文件。它同时支持多目录/多文件选择（`multi_select_tests.rs`）与"最近文件"模式。

## 4. 键位输入与提示流程

```mermaid
graph TB
    A[用户按下键] --> B[gpui KeyContext 匹配 Binding]
    B --> C{命中且有子绑定?}
    C -->|是| D[which_key PendingBinding 记录前缀]
    D --> E[底部/浮层列出后续候选键]
    C -->|否, 唯一| F[dispatch Action]
```

## 5. 集成点

- `which_key` 依赖 `gpui` 的 `Keymap`/`KeyContext`/`BindingObserver` 与 `settings::KeymapFile`；键位本身仍由 `settings` 的 `KeymapFile` 解析，改键通过直接编辑 `keymap.json` 完成。
- `file_finder` 基于 `picker`+`fuzzy`+`project::Project`（工作树 entry），由 `workspace` Action `file_finder::Toggle` 调起（见 Picker-Family 页）。

## 6. 相关页

- [Picker-Family-Deep-Dive](Picker-Family-Deep-Dive.md)（`file_finder` 依赖的 `Picker`/`fuzzy`）
- [Settings-and-Onboarding-Deep-Dive](Settings-and-Onboarding-Deep-Dive.md)（`KeymapFile`/settings 栈）
- [GPUI-Deep-Dive](GPUI-Deep-Dive.md)（`Keymap`/`KeyContext`/`Action` 内核）
