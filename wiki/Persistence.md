# Persistence（db / sqlez / migrator 持久化层）

覆盖 [`sqlez`](../crates/sqlez)（SQLite 薄封装 + 线程安全连接）、[`sqlez_macros`](../crates/sqlez_macros)（编译期 SQL）、[`db`](../crates/db)（应用数据库与 KV 存储）、[`migrator`](../crates/migrator)（**设置/键位文件**的跨版本迁移）。Zed 用它保存 KV 状态、项目历史、Agent 会话、键位编辑草稿等。

## 1. 两套"迁移"要分清

| 机制 | Crate | 迁移对象 | 触发 |
|---|---|---|---|
| 数据库 schema 迁移 | `sqlez` | SQLite 表结构（按 domain） | 连接打开时 |
| 配置文件迁移 | `migrator` | 用户 `settings.json` / `keymap.json` 的 JSON | 加载设置时 |

## 2. sqlez：SQLite 封装
[`crates/sqlez/src`](../crates/sqlez/src)

| 符号 | 位置 | 作用 |
|---|---|---|
| `struct Connection` | [`connection.rs:12`](../crates/sqlez/src/connection.rs) | 裸连接（`libsqlite3_sys` 封装） |
| `Connection::open_file` | [L56](../crates/sqlez/src/connection.rs) | 打开持久 DB 文件 |
| `Connection::open_memory` | [L60](../crates/sqlez/src/connection.rs) | 内存库（测试用） |
| `Connection::backup_main` | [L82](../crates/sqlez/src/connection.rs) | 在线备份 |
| `Connection::sql_has_syntax_error` | [L101](../crates/sqlez/src/connection.rs) | 预检语法 |
| `struct Statement<'a>` | [`statement.rs:11`](../crates/sqlez/src/statement.rs) | 预处理语句（绑定/列映射） |
| `Connection::migrate` | [`migrations.rs:37`](../crates/sqlez/src/migrations.rs) | 按 domain 执行 schema 迁移 |

**schema 迁移模型**（migrations.rs 顶部注释）：每条迁移按 `domain` + `step` 记录在 `migrations(domain, step, migration)` 表；启动时若某步已记录的 SQL 文本与新代码不一致会**报错/回退**，否则补跑缺失步骤。迁移用 `eager_exec`（不 prepare）以支持多语句 schema 变更。

## 3. 线程安全连接（真正的写入口）
[`thread_safe_connection.rs`](../crates/sqlez/src/thread_safe_connection.rs)

| 符号 | 位置 | 作用 |
|---|---|---|
| `struct ThreadSafeConnection` | [L34](../crates/sqlez/src/thread_safe_connection.rs) | 可跨线程克隆的连接句柄 |
| `struct ThreadSafeConnectionBuilder<M: Migrator>` | [L44](../crates/sqlez/src/thread_safe_connection.rs) | 构造器，绑定一个 `Migrator` |
| `ThreadSafeConnection::builder` | [L143](../crates/sqlez/src/thread_safe_connection.rs) | 入口 |
| `with_db_initialization_query` | [L62](../crates/sqlez/src/thread_safe_connection.rs) | 初始化 SQL（如 PRAGMA） |
| `write<T>` | [L169](../crates/sqlez/src/thread_safe_connection.rs) | 把写操作投递到专用线程串行执行 |
| `background_thread_queue` / `locking_queue` | [L282](../crates/sqlez/src/thread_safe_connection.rs) / [L306](../crates/sqlez/src/thread_safe_connection.rs) | 写队列策略 |

SQLite 单写者模型 → Zed 用 `write()` 把所有写序列化到后台线程，读可在任意线程并发。`Migrator` trait（`migrate(&Connection)`）由 `db`/各领域实现（见 `domain.rs` 的一串 `impl Migrator`）。

## 4. sqlez_macros：编译期 SQL
[`sqlez_macros.rs:16`](../crates/sqlez_macros/src/sqlez_macros.rs) 的 `sql!` 过程宏：在**编译期**解析 SQL 字符串字面量并生成 prepare + 行列绑定代码，把运行时 SQL 错误尽量提前。

## 5. db：应用数据库 + KV 存储
[`crates/db/src`](../crates/db/src)（re-export `sqlez`/`sqlez_macros`/`paths::database_dir`/`release_channel`）

| 符号 | 位置 | 作用 |
|---|---|---|
| `struct AppDatabase(pub ThreadSafeConnection)` | [`db.rs:41`](../crates/db/src/db.rs) | 全局应用 DB |
| `struct AppMigrator` | [L46](../crates/db/src/db.rs) | 顶层 `Migrator`（组合各领域 domain 迁移） |
| `struct DomainMigration` | [L30](../crates/db/src/db.rs) | 单领域迁移描述 |
| `AppDatabase::global(cx)` | [L80](../crates/db/src/db.rs) | 从 `App` 取全局连接 |
| `AppDatabase::test_new` | [L72](../crates/db/src/db.rs) | 测试用内存库 |

KV 存储 [`kvp.rs`](../crates/db/src/kvp.rs)（最常用的通用键值表）：
- `struct KeyValueStore` [L12](../crates/db/src/kvp.rs)、`from_app_db`(L15)。
- `read_kvp`(L67)、`scoped(namespace)`→`ScopedKeyValueStore`(L89/L97)、`read`(L103)。
- `struct GlobalKeyValueStore`(L224)、`global()`(L253)、`read_kvp`(L258)。
命名空间隔离让不同子系统在同一张表里安全共存（如最近项目、UI 状态、flag）。

## 6. migrator：settings.json / keymap.json 迁移
[`crates/migrator/src/migrator.rs`](../crates/migrator/src/migrator.rs)

| 符号 | 位置 | 作用 |
|---|---|---|
| `migrate_settings(text)` | [L159](../crates/migrator/src/migrator.rs) | 把旧版设置 JSON 升级到当前 schema |
| `migrate_keymap(text)` | [L120](../crates/migrator/src/migrator.rs) | 旧键位文件升级 |
| `migrate_edit_prediction_provider_settings` | [L263](../crates/migrator/src/migrator.rs) | 特定字段迁移 |

迁移逻辑按**日期分目录**组织：`migrations/m_YYYY_MM_DD/settings.rs`，每个函数对 `serde_json::Value` 做原地改写。例如：
- `move_edit_prediction_provider_to_edit_predictions`（m_2026_02_02）
- `make_auto_indent_an_enum`（m_2025_01_27）、`make_git_gutter_width_an_enum`（m_2026_08_17）
- `rename_web_search_to_search_web`（m_2026_04_10）、`nest_markdown_preview_settings`（m_2026_08_30）
- `migrate_tool_permission_defaults`（m_2026_02_04）、`migrate_builtin_agent_servers_to_registry`（m_2026_02_25）

当设置项被重命名/改类型/重组时，就在这里加一条 `m_<date>::settings::fn`，`migrate_settings` 会按序应用，保证用户旧配置平滑升级（见 [Settings-and-Themes.md](Settings-and-Themes.md)）。

## 7. 与其他页面的关系
- 路径来源 `database_dir`：[Settings-and-Themes.md](Settings-and-Themes.md)（`paths`）。
- 键位草稿持久化 `KeybindingEditorDb`：[Vim-and-Key-Input.md](Vim-and-Key-Input.md)。
- 设置结构 `settings_content`：[Settings-and-Themes.md](Settings-and-Themes.md)。
- 被谁使用：project history / agent 会话 / telemetry 等（见 [Module-Index.md](Module-Index.md) 第 20 类）。
