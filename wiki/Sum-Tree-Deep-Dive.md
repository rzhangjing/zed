# Deep Reference: sum_tree

> 参考手册级：`crates/sum_tree` 的**全部公开类型、trait、方法**与内部机制。该 crate 是 Zed 中 rope、worktree、text 操作日志等的通用底座——一棵**带增量摘要的持久化（结构共享）平衡树**。

## 0. 设计模型
- 元素类型 `T: Item`，每个元素带一个可 combine 的 `Summary`。
- 内部节点缓存其子树的 `Summary`，于是可凭任意实现了 `Dimension` 的量在 **O(log n)** 定位，无需线性扫描。
- 树根是 `Arc<Node<T>>`；写时复制（COW），克隆仅增引用计数——所以 `SumTree` 天然可作"历史快照"。

## 1. 核心 trait（扩展点）
定义于 [`sum_tree.rs`](../crates/sum_tree/src/sum_tree.rs)：

| Trait | 位置 | 关联项 / 方法 | 语义 |
|---|---|---|---|
| `trait Item: Clone` | [L34](../crates/sum_tree/src/sum_tree.rs) | `type Summary: Summary`；`fn summary(&self, cx) -> Self::Summary` | 元素如何被摘要 |
| `trait KeyedItem: Item` | [L41](../crates/sum_tree/src/sum_tree.rs) | `type Key: Dimension + Ord`；`fn key(&self)` | 有唯一可 seek 键的元素（供 TreeMap） |
| `trait Summary: Clone` | [L51](../crates/sum_tree/src/sum_tree.rs) | `type Context<'a>: Copy`；`fn zero(cx)`；`fn add_summary(&mut self, &Self, cx)` | 可 combine 的聚合量 |
| `trait ContextLessSummary: Clone` | [L57](../crates/sum_tree/src/sum_tree.rs) | `fn zero()`；`fn add_summary(&mut self,&Self)` | 无需上下文的 Summary；有 blanket `impl Summary`（L62） |
| `struct NoSummary` | [L75](../crates/sum_tree/src/sum_tree.rs) | `impl ContextLessSummary`（L80） | 不关心维度时的占位 |
| `trait Dimension<'a, S: Summary>: Clone` | [L95](../crates/sum_tree/src/sum_tree.rs) | `zero(cx)`、`add_summary`、`with_added_summary`(L100)、`from_summary`(L105) | "可按其定位"的量（如第 N 字符 / 某坐标） |
| blanket `impl Dimension for T: Summary` | [L112](../crates/sum_tree/src/sum_tree.rs) | —— | Summary 自身也是 Dimension |
| `trait MapSeekTarget<K>` | tree_map 使用 | `cmp_with_key`/`start`/`end` | 有序映射的 seek 目标 |

**关键点**：`Summary::Context` 允许摘要计算依赖外部上下文（如 tab size），`add_summary` 是 combine 的唯一入口。

## 2. `SumTree<T>`：主类型 API
[`sum_tree.rs`](../crates/sum_tree/src/sum_tree.rs)（结构 [L213](../crates/sum_tree/src/sum_tree.rs) `struct SumTree<T: Item>(Arc<Node<T>>)`）

### 构造
| 方法 | 位置 | 说明 |
|---|---|---|
| `new(cx)` | [L226](../crates/sum_tree/src/sum_tree.rs) | 空树（仅一个 `from_summary` 根） |
| `from_summary(summary)` | [L235](../crates/sum_tree/src/sum_tree.rs) | 由摘要造树（无叶子） |
| `from_item(item, cx)` | [L243](../crates/sum_tree/src/sum_tree.rs) | 单元素树 |
| `from_iter(iter, cx)` | [L249](../crates/sum_tree/src/sum_tree.rs) | 顺序构建（内部攒 `MAX_INLINE_ITEMS` 后 `build_tree`） |
| `from_par_iter(iter, cx)` | [L318](../crates/sum_tree/src/sum_tree.rs) | 并行构建（rayon，分段 reduce） |

### 读取 / 遍历
| 方法 | 位置 | 说明 |
|---|---|---|
| `summary()` | [L736](../crates/sum_tree/src/sum_tree.rs) | 根摘要（即全树聚合） |
| `items(cx)` | [L381](../crates/sum_tree/src/sum_tree.rs) | 收集为 `Vec<T>` |
| `iter()` | [L392](../crates/sum_tree/src/sum_tree.rs) | 惰性 `Iter<T>` 前向遍历 |
| `first()` / `last()` | [L622](../crates/sum_tree/src/sum_tree.rs) / [L626](../crates/sum_tree/src/sum_tree.rs) | 边界元素 |
| `cursor::<D>()` | [L597](../crates/sum_tree/src/sum_tree.rs) | 按维度 `D` 建游标 |
| `cursor_for_item(item)` | —— | 定位到某元素游标 |
| `find_exact::<D,Target>` | [L400](../crates/sum_tree/src/sum_tree.rs) | 精确命中某维度位置 |
| `find::<D,Target>(target,cx)` | [L426](../crates/sum_tree/src/sum_tree.rs) | 返回 (cursor, found) |
| `find_with_prev::<D,Target>` | [L511](../crates/sum_tree/src/sum_tree.rs) | 同时给前驱（区间定位常用） |
| `filter::<F,U>()` | [L609](../crates/sum_tree/src/sum_tree.rs) | 按谓词惰性产出映射值 |

### 写（返回新树或原地 builder）
| 方法 | 位置 | 说明 |
|---|---|---|
| `edit(range, cx, f)` | [L1190](../crates/sum_tree/src/sum_tree.rs) | 核心：对 `[range]` 区间回调可 `splice`，产出更新后的树（COW，仅重写受影响路径） |
| `splice<Operation>(range, operation, cx)` | impl 内 | 用某个 `Operation` 在区间上增删改（rope 的编辑入口） |
| `append(other, cx)` | [L780](../crates/sum_tree/src/sum_tree.rs) | 末尾拼接另一棵树（可能触发 rebalance） |
| `Cursor::edit` | cursor.rs | 在游标处编辑 |

`Node`/`Child` 是私有结构（`Arc<Node>`）；`rev_len`/`Dimension` 的 `sub()`（[L198](../crates/sum_tree/src/sum_tree.rs) `invert`）支撑反向定位。

## 3. `Cursor<T, D>`：定位游标
[`cursor.rs`](../crates/sum_tree/src/cursor.rs)（结构 [L30](../crates/sum_tree/src/cursor.rs) `Cursor<'a,'b,T:Item,D>`）

| 方法 | 位置 | 说明 |
|---|---|---|
| `new(tree, cx)` | [L64](../crates/sum_tree/src/cursor.rs) | 定位到起点 |
| `reset()` | [L75](../crates/sum_tree/src/cursor.rs) | 回到树根 |
| `start()` / `end()` | [L82](../crates/sum_tree/src/cursor.rs) / [L87](../crates/sum_tree/src/cursor.rs) | 当前位置的维度区间 |
| `item()` | [L99](../crates/sum_tree/src/cursor.rs) | 当前叶元素 |
| `item_summary()` | [L118](../crates/sum_tree/src/cursor.rs) | 当前元素摘要 |
| `next_item()` / `prev_item()` | [L139](../crates/sum_tree/src/cursor.rs) / [L177](../crates/sum_tree/src/cursor.rs) | 邻接元素 |
| `next()` / `prev()` | [L291](../crates/sum_tree/src/cursor.rs) / [L216](../crates/sum_tree/src/cursor.rs) | 前/后步进（跨节点） |
| `search_forward(filter)` | [L296](../crates/sum_tree/src/cursor.rs) | 结合谓词向下搜索（剪枝） |
| `search_backward(filter)` | [L221](../crates/sum_tree/src/cursor.rs) | 反向剪枝搜索 |
| `seek(pos, bias)` | [L408](../crates/sum_tree/src/cursor.rs) | 二分定位到 `Target` |
| `seek_forward(pos, bias)` | [L423](../crates/sum_tree/src/cursor.rs) | 只向前定位（单调游标更快） |
| `did_seek()` | [L395](../crates/sum_tree/src/cursor.rs) | 是否发生位置移动 |

`Bias`（Left/Right）决定落在多字节字符边界的一侧——rope 的字符/行定位靠它。

## 4. `TreeMap` / `TreeSet`：有序映射
[`tree_map.rs`](../crates/sum_tree/src/tree_map.rs)：基于 `SumTree<MapEntry<K,V>>` 的有序 map/set（非 std BTreeMap，但 API 相近且可 O(log) 区间操作）。

| 符号 | 位置 | 说明 |
|---|---|---|
| `struct TreeMap<K,V>` | [L7](../crates/sum_tree/src/tree_map.rs) | 有序映射 |
| `struct MapEntry<K,V>` | [L13](../crates/sum_tree/src/tree_map.rs) | 树元素（`KeyedItem`） |
| `struct MapKey<K>` / `MapKeyRef<'a,K>` | [L19](../crates/sum_tree/src/tree_map.rs) / [L28](../crates/sum_tree/src/tree_map.rs) | 按 key 的 `Dimension`（seek 用） |
| `struct TreeSet<K>` | [L37](../crates/sum_tree/src/tree_map.rs) | `TreeMap<K,()>` |
| `from_ordered_entries` | [L42](../crates/sum_tree/src/tree_map.rs) | 由已排序项构建 |
| `is_empty`/`contains_key`/`get` | [L52](../crates/sum_tree/src/tree_map.rs)..| 查询 |
| `insert`/`insert_or_replace`/`extend`/`clear` | [L75](../crates/sum_tree/src/tree_map.rs).. | 写入 |
| `remove` / `remove_range` | [L97](../crates/sum_tree/src/tree_map.rs) / [L112](../crates/sum_tree/src/tree_map.rs) | 删除（区间） |
| `iter` | [L176](../crates/sum_tree/src/tree_map.rs) | `(&K,&V)` 顺序迭代；另 range 迭代 |

## 5. 复杂度与不变量
- 读定位、`seek`、`edit`：O(log n)；`edit` 仅重写根→受影响叶路径并沿路重算 `Summary`。
- 树保持"每节点子节点数 ≤ `MAX_INLINE_ITEMS`"，超出即 split（`from_iter`/`append` 触发 `build_tree`/rebalance）。
- `Summary` 单调 combine，`Cursor` 的 `start/end` 区间由已跳过子树的摘要累加得到。

## 6. 被谁使用（真实）
- [`rope`](Rope-Deep-Dive.md)：`Rope = SumTree<Chunk>`，`TextSummary` 有 chars/lines/utf16 多维度。
- [`text`](Text-Buffer-Deep-Dive.md)：操作日志、`UndoMap`(`SumTree<UndoMapEntry>`)、`OperationQueue`(`SumTree<OperationItem>`)。
- [`worktree`](Project-Panel-and-FS.md)：目录项按路径 key 排序的 `SumTree`。
- [`edit_prediction`](Edit-Prediction.md) 等需要"历史快照 + 结构共享"处。

## 7. 参考
- 概览版见 [Data-Structures.md](Data-Structures.md)；上层文本内核见 [Text-Buffer-Deep-Dive.md](Text-Buffer-Deep-Dive.md)。
