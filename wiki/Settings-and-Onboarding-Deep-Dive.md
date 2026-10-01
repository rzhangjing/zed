# Settings & Onboarding 深挖（设置界面 · 设置内容模型 · 首次引导）

> 返回 [Home](Home) · [Module-Index](Module-Index)
>
> 概览见 [Settings-and-Themes.md](Settings-and-Themes.md)。本页是**函数/类型级参考手册**，覆盖设置三件套：`settings_content`（与 UI 无关的**设置数据模型**，按域拆分）· `settings_ui`（图形化 `SettingsWindow` 设置页）· `onboarding`（首启引导 `Onboarding`）；并关联 `settings`/`settings_json`/`settings_macros`/`settings_profile_selector` 基础设施。所有符号均来自 `grep`/`read` 确证（`文件:行号`）。

## 1. 三crate 分工

```mermaid
graph TB
    SC[settings_content<br/>设置数据模型 *Content + MergeFrom] --> SU[settings_ui::SettingsWindow<br/>图形设置页]
    SC --> RUNTIME[settings crate<br/>typed settings + watch]
    SM[settings_macros<br/>#[derive(RegisterSetting)]] --> RUNTIME
    SJ[settings_json<br/>jsonc 解析/编辑] --> SU
    SU --> RUNTIME
    OB[onboarding::Onboarding<br/>首启引导] --> RUNTIME
    OB --> SJ
```

## 2. `settings_content`（数据模型 · 与 UI 无关）

`settings_content.rs`(48KB) 是"原始设置内容"的 schema 层：每个可编辑项是 `Option<T>`，便于"仅覆盖被改动的键"。

| 类型 | 行 | 角色 |
| --- | --- | --- |
| `struct SettingsContent` | 174 | **根内容聚合**（editor/language/theme/terminal/workspace/project/title_bar… 各子域） |
| `trait RootUserSettings` | 422 | 用户根设置契约（`DeserializeOwned`） |
| `enum ProfileBase` / `struct SettingsProfile` | 470 / 481 | 设置档位（base）与自定义 profile |
| `struct UserSettingsContent` / `ExtensionsSettingsContent` / `AudioSettingsContent` | 496 / 510 / 560 | 分面内容 |
| `enum BaseKeymapContent` | 530 | 基础键位（VSCode/Vim/…）内容 |
| `enum ParseStatus` | 107 | jsonc 解析状态 |
| `struct PixelSetting` / `enum HideMouseMode` / `ReduceMotionMode` | 60 / 135 / 164 | 通用取值类型 |
| `struct FeatureFlagsMap` | 359 | 特性开关 map |
| `trait MergeFrom` | merge_from.rs:17 | **增量合并**：`merge_from(&mut self, other)`/`merge_from_option`(22)——把"改动片段"叠到默认值上（大量 `impl`：33/63/77/…） |

**按域文件**（每域一份 `*Content` 结构，字段全 `Option<..>`）：`editor.rs`(34KB)、`language.rs`(57KB)、`theme.rs`(53KB)、`terminal.rs`(18KB)、`workspace.rs`(35KB)、`project.rs`(34KB)、`agent.rs`(52KB)、`language_model.rs`(23KB)、`action.rs`(11KB)、`title_bar.rs`、`extension.rs`；`fallible_options.rs`/`serde_helper.rs` 提供容错反序列化。

## 3. `settings_ui`（图形设置页 · `settings_ui.rs` 259KB）

| 类型 | 行 | 角色 |
| --- | --- | --- |
| `struct SettingsWindow` | 950 | 主 `Item`（工作区标签页），左导航 + 右表单 |
| `struct SettingsPage` | 1077 | 当前页状态；`enum SettingsPageItem`(1083) 页内条目 |
| `struct SubPage` / `struct NavBarEntry` | 1032 / 1068 | 子页层级与左侧导航项 |
| `struct SettingField<T>` | 113 | **单字段模型**（绑定某 `*Content` 的 `Option<T>`） |
| `trait AnySettingField` | 176 | 类型擦除字段面（供统一渲染/写回） |
| `struct SettingFieldRenderer` | 283 | 字段渲染器（toggle/text/select/slider） |
| `struct UnimplementedSettingField` | 155 | 未接入字段的占位 |
| `struct SettingsFieldMetadata` | 441 | 字段元数据（文档/键路径） |
| `enum SettingsFileTarget` | 684 | **写回目标**：User / Project / Profile（决定改哪份 json） |
| `struct SearchIndex`/`SearchDocument`/`SearchKeyLUTEntry` | 1019/1014/1025 | 设置全文搜索索引 |

- `page_data.rs`（**513KB**！）：静态声明所有设置页/分组/字段的目录数据（由 `render_page`3768 / `render_settings_item`1475 / `render_toggle_button`4962 / `render_text_field`4897 消费）。
- `pages/`(11)：各具体页（Editor/Preferences/Language…）；`components/`(8)：复用控件；`settings_ui.rs` 顶部 `FocusFile`(111) 动作支持"设置里定位到某文件"。
- 写回：编辑字段 → 组 `SettingsContent` 片段 → `MergeFrom` 合并进对应 jsonc 文件 → `settings` crate 广播 → 运行时生效。

## 4. `onboarding`（首启引导 · `onboarding.rs` 25KB）

| 符号 | 位置 | 角色 |
| --- | --- | --- |
| `struct Onboarding` | 208（`impl Item` 396） | 引导页 `Item`（首个标签），逐屏收集偏好 |
| `struct OnboardingPagesDb` | 658 | 已看引导页的 SQLite 记录（`items`/`visited`） |
| `action ImportVsCodeSettings` / `ImportCursorSettings` | 42 / 51 | 一键导入 VS Code / Cursor 设置 |
| `struct SettingsImportState` | 574 | 导入进行中/完成状态 |
| `struct BaseKeymapSelector` / `BaseKeymapSelectorDelegate` | base_keymap_picker.rs:45 / 75 | 基础键位选择（复用 `picker`） |
| `enum ThemePreviewStyle` / `struct ThemePreviewTile` | theme_preview.rs:13 / 22 | 主题预览瓦片（实时套用） |
| `struct MultibufferHint` | multibuffer_hint.rs:11 | 多缓冲提示卡 |
| `basics_page.rs`(27KB) | — | 基础引导页装配 |

引导流程：欢迎 → 选基础键位（`BaseKeymapSelector`）→ 选主题（`ThemePreviewTile`）→ AI/隐私选项 → 写 `settings.json`（经 `SettingsContent` + `MergeFrom`），可"导入 VS Code/Cursor"（`ImportVsCodeSettings`）。

## 5. 基础设施 crate（一览）

- `settings`：类型化 `SettingsStore`、`#[derive(RegisterSetting)]` 落地、`watch` 变更流（typed settings 门面）。
- `settings_json`：jsonc 的**无损编辑**（保留注释/格式，按 key 路径写回）。
- `settings_macros`：`FromSettings`/`RegisterSetting` 等 proc-macro。
- `settings_profile_selector`：状态栏档位切换器。

## 6. 集成 / 相关页

- 深页：[GPUI-Deep-Dive.md](GPUI-Deep-Dive.md)、[Picker-Family-Deep-Dive.md](Picker-Family-Deep-Dive.md)（键位选择器）、[Extension-Deep-Dive.md](Extension-Deep-Dive.md)（扩展设置）、[Agent-Deep-Dive.md](Agent-Deep-Dive.md)（`agent.rs` 设置内容）、[Workspace-Deep-Dive.md](Workspace-Deep-Dive.md)（主题/键位应用）、[Persistence.md](Persistence.md)（`OnboardingPagesDb`）
- 概览：[Settings-and-Themes.md](Settings-and-Themes.md)
- 导航：[Home](Home) · [Module-Index](Module-Index)
