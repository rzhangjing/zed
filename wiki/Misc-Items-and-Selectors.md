# Misc Items & Selectors（其他 UI Item 与状态栏选择器）

本页收拢"零散但成面"的 UI 模块：**可视化 Item**（[`image_viewer`](../crates/image_viewer)、[`svg_preview`](../crates/svg_preview)、[`tabular_data_preview`](../crates/tabular_data_preview)）、**状态栏/命令选择器**（各 `*_selector`）、**导航辅助**（[`breadcrumbs`](../crates/breadcrumbs)、[`go_to_line`](../crates/go_to_line)、[`recent_projects`](../crates/recent_projects)、`tab_switcher`、`onboarding`）。它们都实现 [Workspace 的 `Item` trait](Workspace-Pane-Dock.md) 或作为 status bar 组件。

## 1. 可视化 Item

### image_viewer（图片查看）
[`image_viewer.rs`](../crates/image_viewer/src/image_viewer.rs)
- `struct ImageView`（[L65](../crates/image_viewer/src/image_viewer.rs)）：`Item` 实现，负责加载/缩放/旋转/裁剪，写回二进制 buffer。
- `struct ImageViewToolbarControls`（[L834](../crates/image_viewer/src/image_viewer.rs)）：底部工具栏。
- `struct ImageViewerDb`（[L1362](../crates/image_viewer/src/image_viewer.rs)）：基于 [`ThreadSafeConnection`](Persistence.md) 持久化查看偏好。
- `struct ImageInfo`（[`image_info.rs:10`](../crates/image_viewer/src/image_info.rs)）：尺寸/格式元信息。
- `ImageViewerSettings`（[`image_viewer_settings.rs:6`](../crates/image_viewer/src/image_viewer_settings.rs)）。

### svg_preview（SVG 预览）
[`svg_preview_view.rs`](../crates/svg_preview/src/svg_preview_view.rs)
- `struct SvgPreviewView`（[L17](../crates/svg_preview/src/svg_preview_view.rs)）：把 SVG buffer 渲染成图像 Item。
- `is_svg_file`（[L193](../crates/svg_preview/src/svg_preview_view.rs)）、`resolve_active_item_as_svg_buffer`（[L155](../crates/svg_preview/src/svg_preview_view.rs)）：判定与取 buffer。
- `open_preview_in_pane` / `open_preview_to_the_side_of_pane`（[L205](../crates/svg_preview/src/svg_preview_view.rs) / [L215](../crates/svg_preview/src/svg_preview_view.rs)）：布局。
- `register`（[L249](../crates/svg_preview/src/svg_preview_view.rs)）：向 `Workspace` 注册命令。

### tabular_data_preview（表格数据预览：CSV/JSON/…）
[`tabular_data_preview.rs`](../crates/tabular_data_preview/src/tabular_data_preview.rs) 及其 `types`/`table_data_engine` 子模块——把"表格状文本"渲染成可过滤/排序的网格：
- `struct TabularDataPreviewPane`（[L27](../crates/tabular_data_preview/src/tabular_data_preview.rs)）：主视图；`PerformanceMetrics`（[L307](../crates/tabular_data_preview/src/tabular_data_preview.rs)）。
- `struct TableLikeContent`（[`types/table_like_content.rs:7`](../crates/tabular_data_preview/src/types/table_like_content.rs)）、`enum TableCell`/`CellContentSpan`（[`types/table_cell.rs`](../crates/tabular_data_preview/src/types/table_cell.rs)）。
- 坐标类型 `DisplayRow`/`DataRow`/`AnyColumn`/`DisplayCellId`（[`types/coordinates.rs`](../crates/tabular_data_preview/src/types/coordinates.rs)）：区分"显示行"与"数据行"（过滤后不同）。
- 引擎：`DisplayToDataMapping`（[`table_data_engine.rs:58`](../crates/tabular_data_preview/src/table_data_engine.rs)）、按列过滤 `FilterEntry`/`FilterEntryState`（[`filtering_by_column.rs`](../crates/tabular_data_preview/src/table_data_engine/filtering_by_column.rs)）、排序 `SortDirection`/`AppliedSorting`（[`sorting_by_column.rs`](../crates/tabular_data_preview/src/table_data_engine/sorting_by_column.rs)）。
- 设置 `RowRenderMechanism`/`RowIdentifiers`/`FilterSortOrder`（[`settings.rs`](../crates/tabular_data_preview/src/settings.rs)）。

## 2. 导航辅助

### breadcrumbs（面包屑）
[`breadcrumbs.rs:24`](../crates/breadcrumbs/src/breadcrumbs.rs) `struct Breadcrumbs`；`RenderBreadcrumbText`（[L20](../crates/breadcrumbs/src/breadcrumbs.rs)）是注入的渲染回调（editor 提供符号路径 + file 提供路径段）。见 [Editor.md](Editor.md)。

### go_to_line（跳行 / 列位置）
- `struct GoToLine`（[`go_to_line.rs:24`](../crates/go_to_line/src/go_to_line.rs)）：`cmd-g` 弹出输入并 `editor.go_to_transaction_anchor`/移动光标。
- `CursorPosition` / `UserCaretPosition`（[`cursor_position.rs`](../crates/go_to_line/src/cursor_position.rs)）：状态栏行:列指示（订阅 Editor 光标事件）。

### recent_projects（最近项目 / 远程项目入口）
[`recent_projects.rs`](../crates/recent_projects/src/recent_projects.rs)
- `RecentProjects`（[L610](../crates/recent_projects/src/recent_projects.rs)）+ `RecentProjectsDelegate`（[L853](../crates/recent_projects/src/recent_projects.rs)）：`open`（[L689](../crates/recent_projects/src/recent_projects.rs)）/`popover`（[L717](../crates/recent_projects/src/recent_projects.rs)）。
- `SidebarRecentProjects`（[`sidebar_recent_projects.rs:25`](../crates/recent_projects/src/sidebar_recent_projects.rs)）：侧栏版本。
- **远程**：`RemoteServerProjects`（[`remote_servers.rs:57`](../crates/recent_projects/src/remote_servers.rs)）、`WslPicker`/`WslDistroSelected`（[`wsl_picker.rs`](../crates/recent_projects/src/wsl_picker.rs)）、`parse_ssh_config_hosts`（[`ssh_config.rs:17`](../crates/recent_projects/src/ssh_config.rs)）、`RemoteSettings::ssh_connections`（[`remote_connections.rs:39`](../crates/recent_projects/src/remote_connections.rs)）——把 SSH/WSL 主机列成"可打开的项目"，接到 [Remote-Development.md](Remote-Development.md)。

## 3. 状态栏 / 命令选择器（`*_selector` 家族）
这些是"薄 UI + picker"模式：读当前值 → 弹 `Picker` → 选中后写回设置或 buffer。

| Crate | 主结构 | 作用 |
|---|---|---|
| [`encoding_selector`](../crates/encoding_selector) | `EncodingSelector`（[L30](../crates/encoding_selector/src/encoding_selector.rs)）+`toggle`(L45)、`ActiveBufferEncoding`(L28) | 切换 buffer 文本编码 |
| [`line_ending_selector`](../crates/line_ending_selector) | `LineEndingSelector` | 切换 LF/CRLF |
| [`language_selector`](../crates/language_selector) | `LanguageSelector` | 切换语言（影响 LSP/高亮） |
| [`theme_selector`](../crates/theme_selector) | `ThemeSelector` | 切换主题（[Settings-and-Themes](Settings-and-Themes.md)） |
| [`toolchain_selector`](../crates/toolchain_selector) | `ToolchainSelector` | 选择语言工具链版本 |
| [`settings_profile_selector`](../crates/settings_profile_selector) | `SettingsProfileSelector` | 切换设置 Profile |
| [`input_latency_ui`](../crates/input_latency_ui) | 延迟可视化 | 输入延迟调试覆盖层 |

## 4. 其他外壳 Item
- [`onboarding`](../crates/onboarding)：首次启动引导页（登录/设置）。
- [`tab_switcher`](../crates/tab_switcher)：`ctrl-tab` 式打开项切换器。
- [`open_path_prompt`](../crates/open_path_prompt)：需要路径输入时的提示弹窗（被 terminal/tasks 复用）。

## 5. 与其他页面的关系
- `Item`/Pane 机制：[Workspace-Pane-Dock.md](Workspace-Pane-Dock.md)。
- picker 基座：[Picker-and-Commands.md](Picker-and-Commands.md)。
- 持久化（ImageViewerDb）：[Persistence.md](Persistence.md)。
- 编码/语言/主题选择改变 buffer 与设置：[Language-and-Project.md](Language-and-Project.md) / [Settings-and-Themes.md](Settings-and-Themes.md)。
