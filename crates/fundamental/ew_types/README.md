# ew-types —— 让大模型读懂你的 Rust 类型

扫源码，就地生成**类型卡**：字段的单位、范围、默认值、不变量、枚举变体与状态流转，全部变成大模型可直接消费的 JSON。

不生成中间文件，不维护第二份数据源，改完代码下次调用就是最新的。

## 为什么不用别的做法

| 做法 | 问题 |
| --- | --- |
| 把 `.rs` 全文塞给模型 | impl 与逻辑混在一起，100 个类型约 1.2 万 token，模型仍要自己从散文里猜单位 |
| 生成卡片文件提交到仓库 | 那是第二份事实源，改代码没重生成就漂移，漂一次就没人信 |
| 导出 JSON Schema | 只有序列化视图，单位、范围、默认值、不变量、流转全丢 |
| **ew-types（本工具）** | 现场解析源码，按需取卡，没有可漂移的中间物 |

## 快速开始

```bash
# 在仓库根执行（本工作区里包名是 `ew_types`，目录是 crates/fundamental/ew_types）
cargo build -p ew_types --release
# 或者先跑一遍测试，确认环境正常
cargo test -p ew_types
```

> **路径里有中文时**（如 `E:\Rust\标准规范`），Windows 上 msys2 的 ld 处理不了非 ASCII 路径，把产物目录指到纯英文路径即可：
> `$env:CARGO_TARGET_DIR='E:\Rust\target-types'; cargo build -p ew_types --offline`

拿仓库自带的 demo 试：

```bash
cargo run -p ew_types -- --root crates/fundamental/ew_types/demo ls
```

```
Currency | enum | domain::order | 币种。 | src/domain/order.rs:10
LineItem | struct | domain::order | 订单明细行。下单时冻结单价，后续改价不影响已生成的订单。 | src/domain/order.rs:63
Money | struct | domain::order | 金额。以最小货币单位记账，不做浮点运算。 | src/domain/order.rs:19
Order | struct | domain::order | 订单。下单时冻结商品价格，创建后不再变更。 | src/domain/order.rs:121
…
共 11 个类型
```

```bash
cargo run -p ew_types -- --root crates/fundamental/ew_types/demo get OrderStatus
```

```json
{
  "name": "OrderStatus",
  "kind": "enum",
  "path": "domain::order",
  "doc": "订单状态。",
  "variants": [
    { "name": "Pending", "doc": "已创建，等待支付。" },
    { "name": "Paid", "doc": "已支付，等待发货。" }
  ],
  "transitions": [
    { "from": "Pending", "to": "Paid", "via": "支付成功" },
    { "from": "Paid", "to": "Shipped", "via": "仓库发货" }
  ],
  "src": "src/domain/order.rs:79"
}
```

（实际输出更完整，这里截断了两处。）

## 命令

| 命令 | 作用 | 典型用法 |
| --- | --- | --- |
| `ls [--module M]` | 列类型索引：名字、类别、模块、一句话 | 模型不知道有什么类型时先问它 |
| `get <NAME>` | 取单个类型卡 | 生成表单、生成测试、写调用代码 |
| `search <KEYWORD>` | 按名字 / 模块 / 中文摘要模糊搜 | 只记得"订单"不记得类型名 |
| `refs <TYPE>` | 反查谁在字段或构造器里用了它 | 改字段前算影响面 |
| `check [--json]` | 注释合规检查，有错退出码 1 | 进 CI，挡不合规的注释 |
| `schema` | 打印卡片 JSON Schema | 校验输出、生成客户端 |
| `mcp` | 以 MCP 服务运行（stdio） | 接 Claude / Cursor / 自建 agent |

全局选项：

- `--root DIR` 扫描目录，默认从当前目录向上找最近的 `Cargo.toml`
- `--format json|compact` 默认 `json`；`compact` 是省 token 的纯文本视图
- `--module M` 只列某个模块

## 接进大模型

### 方式一：MCP（推荐）

在客户端的 MCP 配置里加一条：

```json
{
  "mcpServers": {
    "ew-types": {
      "command": "ew-types",
      "args": ["mcp", "--root", "/path/to/your/crate"]
    }
  }
}
```

> 本仓内没装 `ew-types` 时，把 `command` 换成 `cargo`、`args` 换成 `["run", "-p", "ew_types", "--", "mcp", "--root", "crates/fundamental/ew_types/demo"]` 即可。

模型会看到五个工具：`ls`、`get`、`search`、`refs`、`check`。它自己决定查什么，只把需要的类型卡拉进上下文。

### 方式二：CLI + shell

任何能跑命令的 agent 都能用：

```bash
cargo run -p ew_types -- --root crates/fundamental/ew_types/demo get Order | jq '.fields[] | {name, unit, range, default}'
```

### 方式三：全量注入（≤50 个类型）

```bash
cargo run -p ew_types -- --root crates/fundamental/ew_types/demo ls --format compact > /tmp/type-index.txt
```

把索引放进 system 消息，取卡仍然走工具。

## 在 CI 里用

```yaml
- name: 类型注释合规
  run: |
    cargo install --path crates/fundamental/ew_types --locked
    ew-types check --root . --json > ew-types-report.json
```

> 在本仓内请用 `-p ew_types`，不必安装：`cargo run -p ew_types -- check --root crates/fundamental/ew_types/demo --json`。

`check` 有错误时退出码为 1，直接把不合规的注释挡在合入前。

规则清单：

| 规则 | 检查内容 |
| --- | --- |
| TY-01 | 类型缺文档注释 |
| TY-02 | 注释里含 TODO / 待定 / 暂时 |
| FD-01 | 字段缺文档注释 |
| FD-02 | 字段注释缺「单位」段 |
| FD-03 | 字段注释缺「范围」段 |
| FD-04 | 字段注释缺「默认值」段 |
| EN-01 | 枚举变体缺注释（应写何时进入） |
| EN-02 | 存在 `Other` / `Unknown` 兜底变体（警告） |

`newtype` 的内层字段不单独要求注释，类型级注释已覆盖。

## 工作原理

1. 用 `syn` 解析 `src/**/*.rs`，拿 struct / enum / type alias 与 `#[doc]` 注释
2. 按《Rust 结构与枚举定义规范》的五段句式拆字段注释：`含义。单位：…。范围：…。默认值：…`
3. `# 不变量`、`# 流转`、`# 错误`、`# 示例` 各自解析成结构化数组
4. 输出 JSON（或 compact），直接交给模型

**没有中间文件。** 每次调用都是一次现场解析，源码改了结果立刻变，不存在"忘了重新生成"。

性能：1 万行以内一次全量解析是毫秒级，CLI 每次启动重扫完全可以接受；MCP 每次 `tools/call` 也会重新扫描，保证模型看到的永远是当前源码。

## 目录

```
crates/fundamental/ew_types/
├── src/
│   ├── main.rs       CLI 入口
│   ├── extract.rs    syn 解析：类型、字段、derive、构造器
│   ├── docparse.rs   注释五段与流转解析
│   ├── model.rs      卡片数据模型
│   ├── render.rs     JSON / compact 渲染
│   ├── check.rs      合规检查
│   ├── api.rs        CLI 与 MCP 共用的业务逻辑
│   └── mcp.rs        最小 MCP 服务端
├── schema/card.schema.json   输出契约
├── docs/
│   ├── card-format.md        卡片格式详解
│   └── integration.md        接入指南与提示词
├── demo/                     示例 crate（11 个类型，全部合规）
│   ├── src/domain/order.rs
│   ├── src/domain/user.rs
│   ├── bad/                  故意不合规，用于演示 check
│   └── expected/             命令输出快照
└── tests/demo_test.rs        端到端断言
```

## 已知限制

| 限制 | 说明 |
| --- | --- |
| 宏生成的类型看不到 | `syn` 只看源码 AST，`macro_rules!` 或过程宏展开出来的结构不出现 |
| `cfg` 分支 | 同一类型在多个 `#[cfg]` 下重复定义时，会各自成卡（未按条件编译过滤） |
| 泛型实例化 | 只给 `Load<T>`，不给 `Load<Order>`；模型需要自行代入 |
| 跨 crate 引用 | 只扫 `--root` 指定目录；外部 crate 的类型显示为名字，不给卡片 |
| 注释不合规 | 五段拆不出来时字段的 `unit` / `range` / `default` 为 `null`，并出现在 `missing` 里，不猜 |

## 常见问题

**为什么不直接用 `rustdoc --output-format json`？**
它需要 nightly 的不稳定选项，且输出是给机器看的完整 HIR，噪声大、还要再写一层转换。`syn` 在 stable 上就够，字段注释本来就在 `#[doc]` 属性里。

**为什么不缓存？**
小仓解析是毫秒级；缓存反而引入"缓存与源码不同步"的风险，正是本工具要消灭的东西。真遇到大仓再说，届时按文件内容哈希在进程内缓存。

**中文注释会乱码吗？**
不会。工具全程 UTF-8；只有 Windows 上 `cargo` 的链接器受非 ASCII 路径影响，用 `CARGO_TARGET_DIR` 绕开即可。
