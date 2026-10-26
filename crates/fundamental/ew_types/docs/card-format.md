# 类型卡格式

`ew-types` 的输出契约。JSON Schema 见 [`../schema/card.schema.json`](../schema/card.schema.json)，可用 `ew-types schema` 直接打印。

## 一个类型一张卡

```json
{
  "name": "Order",
  "kind": "struct",
  "path": "domain::order",
  "doc": "订单。下单时冻结商品价格，创建后不再变更。",
  "invariants": ["`items` 非空", "`total` 的币种与每个 `items` 项一致"],
  "fields": [ … ],
  "ctor": { "name": "try_new", "sig": "(id, items, total) -> Result<Order, OrderError>", "checks": [ … ] },
  "derives": ["Debug", "Clone"],
  "src": "src/domain/order.rs:121"
}
```

## 顶层字段

| 字段 | 类型 | 必有 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | 类型名，不含路径 |
| `kind` | enum | 是 | `struct` / `enum` / `newtype` / `alias` |
| `path` | string | 是 | 模块路径，如 `domain::order`。文件路径推导，不含 crate 名 |
| `doc` | string | 是 | 首行摘要；缺失时为空串（此时 `check` 报 TY-01） |
| `invariants` | string[] | 否 | `# 不变量` 段，逐条 |
| `generics` | string[] | 否 | 泛型参数名，如 `["T"]` |
| `fields` | Field[] | 否 | struct / newtype / alias 有 |
| `variants` | Variant[] | 否 | enum 有 |
| `transitions` | Transition[] | 否 | enum 的 `# 流转` |
| `ctor` | Ctor | 否 | 构造入口，只认 `try_new` / `new` / `create` / `from_parts` |
| `derives` | string[] | 否 | 保留源码顺序 |
| `nonExhaustive` | bool | 否 | 只有为 `true` 才出现 |
| `src` | string | 是 | `文件:行号`，用于追溯与定位 |

`kind` 的判定：

- `newtype`：只有一个匿名字段的 tuple struct，如 `struct OrderId(Ulid)`
- `alias`：`type X = Y`，此时 `fields` 里有且只有一条 `name` 为 `target` 的记录

## Field

| 字段 | 类型 | 必有 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | 字段名；tuple 字段用下标 `"0"` |
| `type` | string | 是 | 类型源码，空白已规范化 |
| `doc` | string | 是 | 注释原文 |
| `meaning` | string | 是 | 含义段（第一个关键段之前的内容） |
| `unit` | string \| null | 是 | `单位：` 段；`—` 转成 `null` |
| `range` | string \| null | 是 | `范围：` 段 |
| `default` | string \| null | 是 | `默认值：` 段；`无` 保留为字符串 |
| `precision` | string | 否 | 精度段，如 `两位小数` |
| `nullable` | bool | 是 | 注释里出现「可为空」「为空表示」时为 `true` |
| `note` | string | 否 | 五段之外的自由说明，如 `保持下单顺序` |
| `missing` | string[] | 否 | 缺的必写段：`unit` / `range` / `default` |

### `null` 与 `missing` 的区别

这是最容易误读的地方，两者语义不同：

| 情况 | `unit` | `missing` | 模型该怎么理解 |
| --- | --- | --- | --- |
| 注释写了 `单位：—` | `null` | `[]` | 明确没有单位（字符串、ID 等） |
| 注释写了 `单位：分` | `"分"` | `[]` | 单位是分 |
| 注释根本没写单位段 | `null` | `["unit"]` | **定义未声明**，不要猜，可以要求补注释 |

`missing` 为空表示三段都写了；不为空表示是注释的缺口，不是业务的"不适用"。

## Variant

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `name` | string | 变体名 |
| `doc` | string | 变体注释（应写"何时进入"） |
| `payload` | Field[] | 带载荷变体的字段，结构与 Field 相同 |

## Transition

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `from` | string | 起始变体 |
| `to` | string | 目标变体 |
| `via` | string | 触发条件，可为空串 |

### 两种写法都支持

推荐一行一条，分支随意写：

```rust
/// # 流转
/// - Pending -> Paid：支付成功
/// - Paid -> Shipped：仓库发货
/// - Pending -> Cancelled：用户取消
```

文本图也认，但只解析直链，分支行会被跳过：

```rust
/// # 流转
/// ```text
/// Pending ──支付成功──▶ Paid ──发货──▶ Shipped
/// ```
```

## Ctor

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `name` | string | 方法名 |
| `sig` | string | `(参数名列表) -> 返回类型`，`Self` 已替换为类型名 |
| `checks` | string[] | 取自该方法的 `# 错误` 段 |

构造入口优先级：`try_new` > `new` > `create` > `from_parts`，取第一个命中的。

## 索引条目

`ew-types ls` 输出的是索引，不是完整卡片，只够判断"要不要取这张卡"：

```json
{
  "count": 11,
  "types": [
    { "name": "Order", "kind": "struct", "path": "domain::order", "doc": "订单。…", "fields": 6, "src": "src/domain/order.rs:121" }
  ]
}
```

`fields` / `variants` 计数为 0 时不输出。

## 紧凑视图

`--format compact` 输出的是给人和模型看的纯文本，不是 JSON：

```
Order · struct · domain::order · src/domain/order.rs:121
  订单。下单时冻结商品价格，创建后不再变更。
  inv: `items` 非空 | `total` 的币种与每个 `items` 项一致
  ctor: try_new(id, items, total) -> Result<Order, OrderError>
  fields:
    total | Money | 应付总额 | 单位:分 | 范围:≥ 1 | 默认值:无 | 精度:两位小数
```

用 ` | ` 分隔而不用空格对齐，因为中文是宽字符，等宽字体下也会错列。

## 演进规则

- **只加不改**：新增字段用可选字段；已有字段的含义与类型不得变更
- 破坏性改动要升 `schemaVersion`（当前无此字段，出现时从 `2` 开始）
- `kind` 的枚举值只增不减
- 下游若做严格校验，用 `additionalProperties: false` 会挡住新增可选字段，建议只在 CI 侧严格

## 校验

```bash
ew-types schema > card.schema.json
cargo run -- get Order | check-jsonschema --schemafile card.schema.json /dev/stdin
```

工具自身的输出必须能通过这份 schema —— 这条也写进了测试约定。
