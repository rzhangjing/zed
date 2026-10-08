# Deep Reference: multi_buffer

> 参考手册级：`crates/multi_buffer`。`MultiBuffer` 把**多个 `text::Buffer` 片段（excerpt）拼成一个可编辑、可 diff、可搜索的统一视图**——编辑器的 buffer、diff 视图、outline、project search 结果、agent buffer 全部建立在它之上。

## 1. 类型总览（真实清单）
| 类型 | 位置 | 角色 |
|---|---|---|
| `struct MultiBuffer` | [multi_buffer.rs:74](../crates/multi_buffer/src/multi_buffer.rs) | 可变多 buffer（`EntityType`；持一组 `Entity<Buffer>` + excerpt 树） |
| `struct MultiBufferSnapshot` | [L694](../crates/multi_buffer/src/multi_buffer.rs) | 只读快照（渲染/查询主入口，`Send+Sync`） |
| `enum Event` | [L99](../crates/multi_buffer/src/multi_buffer.rs) | 变更事件（见 §5） |
| `pub(crate) struct Excerpt` | [L857](../crates/multi_buffer/src/multi_buffer.rs) | 一个片段（见 §2）——**核心单元** |
| `struct ExcerptRange<T>` | [L875](../crates/multi_buffer/src/multi_buffer.rs) | `context`(展示区) + `primary`(高亮/匹配区) |
| `struct PathKey` | [path_key.rs](../crates/multi_buffer/src/path_key.rs)（re-export L63） | excerpt 在全局的排序键（路径 + 序号），支持同文件多片段 |
| `struct PathKeyIndex` | multi_buffer.rs | 同 path_key 下的次级序号 |
| `struct ExcerptSummary` / `struct ExcerptBoundary` / `ExcerptBoundaryInfo` / `ExpandInfo` / `RowInfo` | [L908](../crates/multi_buffer/src/multi_buffer.rs)/[823](../crates/multi_buffer/src/multi_buffer.rs)/[754](../crates/multi_buffer/src/multi_buffer.rs)/[840](../crates/multi_buffer/src/multi_buffer.rs)/[846](../crates/multi_buffer/src/multi_buffer.rs) | excerpt 树的摘要与边界计算 |
| `struct MultiBufferDiffHunk` | [L133](../crates/multi_buffer/src/multi_buffer.rs) | 跨 excerpt 的 diff 块（`status()`(L151)/`is_created_file()`(L155)） |
| `enum Anchor` / `struct ExcerptAnchor` | [anchor.rs:28](../crates/multi_buffer/src/anchor.rs)/[17](../crates/multi_buffer/src/anchor.rs) | 多 buffer 位置稳定引用（见 §3） |
| `trait AnchorRangeExt` | anchor.rs | `Anchor` 区间比较/排序 |
| `struct BufferTransaction`(impl) | [transaction.rs](../crates/multi_buffer/src/transaction.rs) | 跨多 buffer 的原子事务 + undo/redo（见 §4） |

### 坐标 newtype（避免混用不同 buffer 的偏移）
| 类型 | 位置 | 空间 |
|---|---|---|
| `MultiBufferPoint = Point`(L162) | [L162](../crates/multi_buffer/src/multi_buffer.rs) | 拼接后视图的行/列 |
| `MultiBufferRow(pub u32)` | [L169](../crates/multi_buffer/src/multi_buffer.rs) | 拼接后行号 |
| `MultiBufferOffset(pub usize)` | [L226](../crates/multi_buffer/src/multi_buffer.rs) | 拼接后 UTF-8 偏移 |
| `MultiBufferOffsetUtf16` | [L365](../crates/multi_buffer/src/multi_buffer.rs) | 拼接后 UTF-16 偏移 |
| `BufferOffset(pub usize)` | [L303](../crates/multi_buffer/src/multi_buffer.rs) | **单个** excerpt 源 buffer 内偏移 |
| `BufferOffsetUtf16` | [L412](../crates/multi_buffer/src/multi_buffer.rs) | 源 buffer UTF-16 偏移 |
| `MultiBufferOffsetUniformSampler` | [L238](../crates/multi_buffer/src/multi_buffer.rs) | 供 fuzzy 采样的均匀映射 |

## 2. `Excerpt`：拼装的基本单元
[multi_buffer.rs:857](../crates/multi_buffer/src/multi_buffer.rs)
```
Excerpt {
  path_key: PathKey,               // 排序/去重键（路径+序号）
  path_key_index: PathKeyIndex,
  buffer_id: BufferId,             // 指向哪个 text::Buffer
  range: ExcerptRange<text::Anchor>,// 展示 context + 高亮 primary（都是 Anchor，随编辑迁移）
  max_buffer_row: BufferRow,       // 该片段最后一行（源 buffer 行）
  text_summary: TextSummary,       // 该片段聚合摘要（行数/字符/utf16）
  has_trailing_newline: bool,      // 片段间换行拼接标记
}
```
- Excerpt 存于 `SumTree<Excerpt>`，`ExcerptSummary` 作为 `Dimension`，于是**多 buffer 全局行/偏移 ↔ 某 excerpt 内源 buffer 行/偏移**能 O(log n) 双向换算。
- `MultiBufferRow ↔ BufferRow`：`row_info`(L846 `RowInfo`)、`ExcerptBoundary`(L823) 提供换算；`excerpts()`(L5561) 遍历全部片段。
- **同一 buffer 可被多次 excerpt**（如 diff 里左右两侧、或一个文件多处上下文），靠 `PathKey` 序号区分。

## 3. `Anchor`：跨 buffer 的稳定引用
[anchor.rs:28](../crates/multi_buffer/src/anchor.rs)
```
enum Anchor { Min, Excerpt(ExcerptAnchor), Max }
ExcerptAnchor { text_anchor: text::Anchor, path: PathKeyIndex, diff_base_anchor: Option<text::Anchor> }
```
- `text_anchor` 复用 [text::Anchor](Text-Buffer-Deep-Dive.md)（依附插入 operation），`path` 指明属于哪个 excerpt，`diff_base_anchor` 让 anchor 同时能映射到 **diff 基线版本**（用于 hunk 归属稳定）。
- `Anchor::Min/Max` 表示"永远钉住多 buffer 首/尾"（如把整份文档当一片段）。
- `AnchorSeekTarget`(L37)：解析时区分 `Missing`（源 buffer 已从该 path key 移除）与 `Excerpt`（正常），保证 buffer 被移除时 anchor 优雅降级。

## 4. 跨 buffer 事务 / undo（transaction.rs）
`MultiBuffer` 的编辑经一个协调器把改动分发到各 `text::Buffer` 并形成**单一可撤销单元**：
| 方法 | 位置 | 语义 |
|---|---|---|
| `set_group_interval(dur)` | [L250](../crates/multi_buffer/src/transaction.rs) | 自动合并窗口 |
| `start_transaction(cx)` | [L256](../crates/multi_buffer/src/transaction.rs) | 开事务 |
| `start_transaction_at(ts)` | [L260](../crates/multi_buffer/src/transaction.rs) | 带时间戳 |
| `end_transaction(cx)` / `end_transaction_with_source` / `end_transaction_at` | [L287](../crates/multi_buffer/src/transaction.rs).. | 提交（附带 `BufferEditSource`） |
| `push_transaction(...)` | [L435](../crates/multi_buffer/src/transaction.rs) | 压入各 buffer 的子事务 |
| `merge_transactions(...)` | [L392](../crates/multi_buffer/src/transaction.rs) | 合并相邻事务 |
| `finalize_last_transaction(cx)` | [L426](../crates/multi_buffer/src/transaction.rs) | 收尾（下次编辑另起） |
| `group_until_transaction(...)` | [L444](../crates/multi_buffer/src/transaction.rs) | 分组到某事务 |
| `undo(cx)` / `redo(cx)` / `undo_transaction(id,cx)` | [L457](../crates/multi_buffer/src/transaction.rs)/[490](../crates/multi_buffer/src/transaction.rs)/[517](../crates/multi_buffer/src/multi_buffer.rs) | 跨 buffer 原子撤销/重做 |
| `edited_ranges_for_transaction(...)` | [L346](../crates/multi_buffer/src/transaction.rs) | 该事务改了哪些区间（局部重解析/diff 用） |

## 5. `Event`：订阅契约（[L99](../crates/multi_buffer/src/multi_buffer.rs)）
`BufferRangesUpdated{buffer,path_key,ranges}`（excerpt 区间变）、`BuffersRemoved{ids}`、`BuffersEdited{ids}`、`DiffHunksToggled`、`Edited{edited_buffer,source}`、`TransactionUndone{id}`、`Reloaded`、`CapabilityChanged`（只读↔可编辑）、`LanguageChanged(id,bool)`、`SettingsChanged`、`Reparsed(id)`、`Saved`、`FileHandleChanged`、`DirtyChanged`、`DiagnosticsUpdated`、`BufferDiffChanged`。
> editor 订阅 `MultiBuffer`，据这些事件驱动重绘、语法重解析、diagnostic、diff 装饰。

## 6. 关键查询 / 快照方法
`text_for_range::<T:ToOffset>(range)`(L3437) 跨 excerpt 拼接取文本、`clip_point(point,bias)`(L4263)、`is_empty`(L1399/L4165)、`excerpts()`(L5561)、`summary()`/`text_summary()`、`point_for_offset`/`offset_for_point`（MultiBufferPoint↔MultiBufferOffset，内部走 excerpt 树维度）。

## 7. 典型用法（真实映射）
- 普通文件编辑：`Editor` → `MultiBuffer`（单 excerpt，`Anchor::Max/Min` 或整文件 range）。
- Diff 对比：一 buffer + `base` buffer 合成多 excerpt，`MultiBufferDiffHunk` 标注增删改。
- Project search / outline / diagnostics：把命中行作为多个 `ExcerptRange{primary=匹配,context=上下文}` 拼进一个 `MultiBuffer` 展示（见 [Search.md](Search.md)）。
- Agent / ACP：AI 生成内容以 excerpt 形式并入可编辑多 buffer（见 [Agent-and-AI.md](Agent-and-AI.md)）。

## 8. 依赖链
[Sum-Tree-Deep-Dive.md](Sum-Tree-Deep-Dive.md) → [Rope-Deep-Dive.md](Rope-Deep-Dive.md) → [Text-Buffer-Deep-Dive.md](Text-Buffer-Deep-Dive.md) → **本页** → [Editor-Deep-Dive.md](Editor-Deep-Dive.md)。
