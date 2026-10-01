# Project Panel & File System（文件系统 / Worktree / 项目树）

自底向上三层：**文件系统抽象**（[`fs`](../crates/fs)，trait + 监听）→ **工作区模型**（[`worktree`](../crates/worktree) 里的 `Worktree`，把目录扫描成内存树并同步 Git/LSP）→ **项目树 UI**（[`project_panel`](../crates/project_panel)）。`Worktree` 集合由 `project` 里的 [`WorktreeStore`](../crates/project/src/worktree_store.rs) 统一管理。

## 1. 分层职责

| 层 | 类型 | 位置 | 职责 |
|---|---|---|---|
| FS 抽象 | `FileSystem` trait | [`fs.rs`](../crates/fs/src/fs.rs) | read/write/create_dir/rename/remove/read_dir/watch |
| FS 实现 | `RealFs` | [`fs.rs:445`](../crates/fs/src/fs.rs) | 真实磁盘实现 |
| 文件监听 | `Watcher` trait | [`fs.rs:72`](../crates/fs/src/fs.rs) | 基于 notify 的目录变更监听 |
| 监听注册 | `WatcherRegistrationId` | [`fs_watcher.rs:788`](../crates/fs/src/fs_watcher.rs) | 监听句柄 |
| 工作区模型 | `enum Worktree` | [`worktree.rs:102`](../crates/worktree/src/worktree.rs) | `Local`/`Remote` 两态 |
| 本地工作区 | `LocalWorktree` | [`worktree.rs:140`](../crates/worktree/src/worktree.rs) | 后台扫描线程 + 快照 |
| 远端工作区 | `RemoteWorktree` | [`worktree.rs:168`](../crates/worktree/src/worktree.rs) | 经 RPC 从远端同步 |
| 树节点 | `Entry` | [`worktree.rs:3959`](../crates/worktree/src/worktree.rs) | 文件/目录条目（含 git 状态、is_ignored） |
| 工作区事件 | `Event` | [`worktree.rs:470`](../crates/worktree/src/worktree.rs) | 增删改/扫描进度通知 |
| 工作区集合 | `WorktreeStore` | [`worktree_store.rs:207`](../crates/project/src/worktree_store.rs) | Project 持有多根 worktree |
| 项目树 UI | `ProjectPanel` | [`project_panel.rs:137`](../crates/project_panel/src/project_panel.rs) | 左侧 Dock 的文件树视图 |
| 面板设置 | `ProjectPanelSettings` | [`project_panel_settings.rs:13`](../crates/project_panel/src/project_panel_settings.rs) | 折叠/图标/git 状态显示等 |

## 2. fs：可替换的文件系统抽象

`FileSystem`（[fs.rs](../crates/fs/src/fs.rs)）把全部磁盘操作做成 `async` trait 方法：`create_dir`(L99)、`rename`(L113)、`write`(L138)、`read_dir`(L144)、`watch`(L164)。`RealFs`（L445）是生产实现（同文件里另有多套 impl：L730/L809/L1030… 对应不同 OS/测试）。之所以抽象成 trait，是为了：① 远端开发时把文件操作走 RPC 转发（见 [Remote-Development.md](Remote-Development.md)）；② 测试用内存/伪 FS。`Watcher`（L72）+ `WatcherRegistrationId`（fs_watcher.rs:788）封装 `notify`，目录一有变化就回调，供 `Worktree` 增量刷新。

## 3. Worktree：把目录变成可订阅的内存树

`Worktree` 现在是枚举（[worktree.rs:102](../crates/worktree/src/worktree.rs)）：`Local(LocalWorktree)`（L140）或 `Remote(RemoteWorktree)`（L168），`impl EventEmitter<Event>`（L481）。它是 `Project`、`project_panel`、搜索、Git 共用的"目录真相"。

- **扫描**：`LocalWorktree` 起后台线程递归读目录，产出 `Entry`（L3959，携带 `is_ignored`/git 状态/symlink 等）。扫描进度经 `Event`（L470）流式上报，UI 据此显示 loading。忽略规则、`.gitignore` 在此解析。
- **变更**：`create_entry`（[L963](../crates/worktree/src/worktree.rs)）、`delete_entry`（L1031）、`rename`/`copy` 等改动树；本地直接落 `fs`，远端（`handle_create_entry` L1142 等）走 RPC 并等对端回执再更新。
- **快照读**：UI/搜索不锁活树，而是取 `WorktreeSnapshot` 只读遍历，保证 GPUI 主线程不阻塞。

## 4. WorktreeStore：多根工作区

一个项目可有多个根（multi-root workspace）。`WorktreeStore`（[worktree_store.rs:207](../crates/project/src/worktree_store.rs)）持有 `Vec<Entity<Worktree>>`：

- `create_worktree`（[L773](../crates/project/src/worktree_store.rs)）异步挂载一个目录为新根。
- `add`（L960）/ `remove_worktree`（L1020）增删。
- 查询：`worktrees()`（L425，迭代所有 `Entity<Worktree>`）、`worktree_for_id`（L449）、`worktree_for_entry`（L454）、`worktree_and_entry_for_id`（L520）。
- `worktree_metadata_protos`（L1225）把根信息序列化进协作 proto，同步给其他成员。

`WorktreePaths`（L46）维护"主路径 ↔ 子文件夹路径"映射，支撑把一个文件夹的多个子目录当独立根展示。

## 5. ProjectPanel：项目树 UI

`ProjectPanel`（[project_panel.rs:137](../crates/project_panel/src/project_panel.rs)）是左侧 `Dock` 里的面板，`impl Render`（L7126）+ `impl Focusable`（L7930），`ProjectPanel::new`（L680）从 `workspace` 拿 `Project`/`Worktree`。它渲染虚拟化的树（只画可视区节点），把用户操作转成 `Worktree` 方法调用：新建/重命名/删除/拖拽移动/复制路径；`new_search_in_directory`（L3978）在某目录发起全局搜索（见 [Search.md](Search.md)）。git 状态色、文件图标（`file_icons` crate）、折叠箭头都由 `ProjectPanelSettings` 驱动。

## 6. 与 Git / LSP 的联动

`Worktree` 与 `GitStore` 共享目录状态：`Entry` 上的 git 徽标来自 git 状态查询；被 `.gitignore` 忽略的文件在树中淡显且默认不参与搜索。语言服务器由 `Worktree` 事件驱动启动（文件出现/消失→LSP 注册/注销），细节见 [Language-and-Project.md](Language-and-Project.md)、[LSP-Features.md](LSP-Features.md)。点击树中文件 → 打开 `Buffer` → 建 `Editor`（见 [Editor.md](Editor.md)），作为 `Item` 进 `Pane`（见 [Workspace-Pane-Dock.md](Workspace-Pane-Dock.md)）。

## 7. 关键符号速查

| 符号 | 位置 | 作用 |
|---|---|---|
| `trait FileSystem` | `fs/src/fs.rs` | 磁盘操作抽象 |
| `RealFs` | `fs/src/fs.rs:445` | 真实文件系统实现 |
| `trait Watcher` | `fs/src/fs.rs:72` | 目录变更监听 |
| `enum Worktree` | `worktree/src/worktree.rs:102` | Local/Remote 工作区 |
| `LocalWorktree` | `worktree.rs:140` | 本地扫描实现 |
| `struct Entry` | `worktree.rs:3959` | 文件/目录树节点 |
| `Worktree::create_entry` | `worktree.rs:963` | 新建文件/目录 |
| `WorktreeStore` | `project/src/worktree_store.rs:207` | 多根管理 |
| `WorktreeStore::create_worktree` | `worktree_store.rs:773` | 挂载新根 |
| `ProjectPanel` | `project_panel/src/project_panel.rs:137` | 左侧文件树面板 |

## 8. 与其他页面的关系
- 全局搜索遍历 worktree：[Search.md](Search.md)。
- Git 状态徽标来源：[Git-Integration.md](Git-Integration.md)。
- LSP 随文件增删启停：[LSP-Features.md](LSP-Features.md)、[Language-and-Project.md](Language-and-Project.md)。
- 远端 `RemoteWorktree` 走 RPC：[Remote-Development.md](Remote-Development.md)。
- 面板停靠与 Item 体系：[Workspace-Pane-Dock.md](Workspace-Pane-Dock.md)。
