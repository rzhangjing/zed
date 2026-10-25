# 预览类 Item 深入解析（Deep Dive）

> 本页覆盖两个"在 Pane 里渲染非文本内容"的 workspace item：`image_viewer`（图片查看器 + 缩放/持久）、`component_preview`（UI 组件预览簿 preview 页）。它们是 `Item` trait 的典型实现，与 `gpui`、`workspace::Pane` 协作。

> ⚠️ 已移除（Batch 1–2c）：原第三个 item `markdown_preview`（Markdown 实时预览）已从本 fork 删除，本页不再覆盖。

## 1. 分层设计

- **`image_viewer`** 把二进制图片解码成 GPUI `Image`，提供缩放/平移/适应窗口，并用 SQLite（`ImageViewerDb`）记忆每张图的缩放状态。
- **`component_preview`** 是一个调试/开发工具：把 `ui` crate 各组件的 story 渲染成可浏览页面（配合 `story_book`/`ui_macros::register_component`）。

## 2. 类型总览

### image_viewer（image_viewer.rs，47KB）

| 符号 | 文件:行 | 角色 |
| --- | --- | --- |
| `struct ImageView` | :65 | 图片 `Item`（缩放/平移） |
| `enum ImageViewEvent` | :534 | 内容变更事件 |
| `struct ImageViewToolbarControls` | :834 | 缩放工具栏 |
| `struct ImageViewerDb(ThreadSafeConnection)` | :1362 | 每图缩放状态持久化 |
| `image_info.rs` | — | 尺寸/格式解码 |

### component_preview（component_preview.rs，35KB）

| 符号 | 文件:行 | 角色 |
| --- | --- | --- |
| `fn init(app_state, cx)` | :25 | 注册 preview 页 |
| `enum PreviewPage` | :87 | 预览页类型 |
| `struct ComponentPreview` | :93 | GPUI Entity 预览容器 |
| `struct ActivePageId(pub String)` | :695 | 当前页 id（Global） |
| `struct ComponentPreviewPage` | :884 | 单组件预览页 |
| `persistence.rs` | — | 预览状态持久 |

## 3. 核心方法与调用锚点

**图片查看（image_viewer.rs）**
- `ImageView`(:65) 实现 `Item`，`Open` 时解码字节为 GPUI `Image`；缩放/平移状态经 `ImageViewerDb`(:1362，`ThreadSafeConnection`) 以 path 为键存 SQLite。
- `ImageViewToolbarControls`(:834) 提供 +/-/1:1/适应窗口；`ImageViewEvent`(:534) 驱动重载。

**组件预览（component_preview.rs）**
- `init(app_state, cx)`(:25) 注册；`ComponentPreview`(:93) 是容器 Entity，`PreviewPage`(:87) 枚举要渲染的 story，`ActivePageId`(:695，Global) 记录当前页；`ComponentPreviewPage`(:884) 渲染单组件。

## 4. 预览 item 生命周期

```mermaid
graph TB
    A[打开 .png] --> B[Item::deserialize / open]
    B --> C[建 ImageView]
    C --> D[Pane.add_item 呈现]
    D --> G[解码 + 读 ImageViewerDb 恢复缩放]
    G --> I[缩放写回 SQLite]
```

## 5. 集成点

- 两者都实现 `workspace::Item`（`tab_content`/`is_dirty`/`save`/`serialize`），注册进 item 类型表以便"重开工作区恢复"。
- `image_viewer` 用 `db`/`sqlez`（`ThreadSafeConnection`）持久缩放（见 Persistence 页）。
- `component_preview` 与 `ui`/`ui_macros`/`story_book` 配合，是 `ui` 组件的开发预览面（见 UI-Primitives 页）。

## 6. 相关页

- [Markdown-Deep-Dive](Markdown-Deep-Dive.md)（渲染内核）
- [UI-Primitives-Deep-Dive](UI-Primitives-Deep-Dive.md)（被 component_preview 预览的组件）
- [Workspace-Deep-Dive](Workspace-Deep-Dive.md)（`Item` trait / `Pane`）
- [Misc-Items-and-Selectors](Misc-Items-and-Selectors.md)（本族概览）
