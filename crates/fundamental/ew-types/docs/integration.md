# 接入指南

把类型卡送进大模型的三种方式、提示词怎么写、以及四个典型任务的调用序列。

## 先选方式

| 库规模 | 方式 | 一次任务的开销 | 说明 |
| --- | --- | --- | --- |
| ≤ 50 个类型 | 全量注入 | ~2k token（索引）+ 需要的卡 | 索引常驻 system 消息，取卡仍走工具或直接拼 |
| 50–300 | **MCP（推荐）** | 每次取卡 80–200 token | 模型自己决定查什么，省 token 且不会看漏 |
| > 300 | MCP + `refs` 反查 | 同上 | 大库先 `search` 定位，再 `get`，改前先 `refs` |

CLI 和 MCP 是同一套逻辑，选哪个取决于你的 agent 有没有工具能力：

- 有 MCP 客户端 → 用 `ew-types mcp`
- 只有 shell → 用 `ew-types` 命令，agent 自己调
- 两者都没有（纯对话）→ 手动跑命令，把 JSON 贴进对话

## MCP 配置

### Claude Desktop / Cursor 等标准客户端

```json
{
  "mcpServers": {
    "ew-types": {
      "command": "/abs/path/to/ew-types",
      "args": ["mcp", "--root", "/abs/path/to/your/crate"]
    }
  }
}
```

### 自建 agent

MCP 是 stdio + JSON-RPC 2.0。一次会话的报文顺序：

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"ew-types","version":"0.1.0"}}}
→ {"jsonrpc":"2.0","method":"notifications/initialized"}
→ {"jsonrpc":"2.0","id":2,"method":"tools/list"}
→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get","arguments":{"name":"Order"}}}
```

`tools/call` 的返回值固定是：

```json
{ "content": [{ "type": "text", "text": "<卡片 JSON 字符串>" }], "isError": false }
```

失败时 `isError` 为 `true`，`text` 里是原因（如"未找到类型 `Nope`"），不会编造一个类型出来。

## 提示词模板

给 agent 的系统提示词里加这一段就够：

```text
查 Rust 类型时使用 ew-types 工具，不要凭记忆猜字段。

1. 不确定有哪些类型，先 ls；只记得中文名，用 search。
2. 取字段级定义用 get <类型名>。
3. 改字段、改枚举前先 refs <类型名>，看谁在引用。
4. 卡片里 unit / range / default 为 null 且 missing 非空，表示"定义未声明"，
   直接说未声明，不要推断。
5. 生成的代码里字段名、变体名、构造器名必须来自卡片，不得自创。
```

第 4 条最关键：`missing` 非空就是"这里缺信息"的信号，模型应当回报缺口而不是补全。

## 四个典型任务

### 一、生成表单校验规则

```bash
ew-types get Order
```

模型拿到 `fields[].unit / range / default` 后可以直接映射：

| 字段 | 卡片信息 | 生成的校验 |
| --- | --- | --- |
| `total` | `unit=分`、`range="≥ 1"`、`precision=两位小数` | 数字框，最小值 1，两位小数，单位显示"元" |
| `status` | `range="见 [OrderStatus]"`、`default=Pending` | 下拉，选项来自枚举变体，默认选中 Pending |
| `created_at` | `unit=UTC` | 只读展示，按本地时区格式化 |

### 二、生成测试用例

```bash
ew-types get Load
```

`variants` 的五个变体就是状态覆盖矩阵，`payload` 说明每个状态需要构造什么数据：

```
Idle / Loading{started_at} / Ready{items} / Empty{query} / Failed{error, retryable}
```

再配合不变量（`Ready.items` 非空）就能推出断言：`Ready` 分支不测空数组、`Empty` 不测非空列表。

### 三、改字段前算影响面

```bash
ew-types refs Money
```

```json
{
  "type": "Money",
  "count": 2,
  "referencedBy": [
    { "from": "LineItem", "field": "price", "type": "Money", "src": "src/domain/order.rs:63" },
    { "from": "Order", "field": "total", "type": "Money", "src": "src/domain/order.rs:121" }
  ]
}
```

模型据此知道改 `Money` 会波及两处，不会改完才发现编译不过。

### 四、写调用代码

```bash
ew-types get User
```

`ctor.sig` 给了构造入口，`fields[].nullable` 给了哪些可以为空，`invariants` 给了构造前必须满足的条件——足够写出不触发 panic 的代码。

## CI 集成

### GitHub Actions

```yaml
- name: 类型注释合规
  run: |
    cargo build --release --manifest-path ew-types/Cargo.toml
    ./ew-types/target/release/ew-types check --root . --json > ew-types-report.json
- name: 上传报告
  if: always()
  uses: actions/upload-artifact@v4
  with:
    name: ew-types-report
    path: ew-types-report.json
```

### GitLab CI

```yaml
ew-types-check:
  script:
    - cargo build --release --manifest-path ew-types/Cargo.toml
    - ./ew-types/target/release/ew-types check --root . --json > ew-types-report.json
  artifacts:
    when: always
    paths: [ew-types-report.json]
```

退出码约定：`0` 通过，`1` 有违规，`2` 用法或路径错误。

## Token 预算

| 内容 | 单条 | 说明 |
| --- | --- | --- |
| 索引条目（compact） | ~20 token | `Order \| struct \| domain::order \| 订单。…` |
| 类型卡（JSON） | ~120–200 token | 字段越多越贵 |
| 类型卡（compact） | ~60–90 token | 大约 JSON 的一半 |
| `refs` 结果 | ~30 token / 条 | 通常 1–5 条 |

一轮正常任务取 3–8 张卡，合计 500–1500 token。相比把整个 crate 源码塞进去（1 万行约 15 万 token），差两个数量级。

## 反模式

| 反模式 | 后果 |
| --- | --- |
| 把 `src/` 全部贴进上下文 | 噪声压过信号，模型会漏看注释，还贵 |
| 把卡片文件提交进仓库 | 第二份事实源，必然漂移 |
| 让模型自己读 `.rs` 猜单位 | 单位写在散文里，十个模型十个答案 |
| `check` 只警告不失败 | 三个月后注释质量回到原样 |
| 在提示词里重复卡片内容 | 维护两份，改了不同步 |

## 与定义规范的配合

卡片里的 `unit` / `range` / `default` / `invariants` 全部来自注释，工具推不出来。所以：

- 注释按《Rust 结构与枚举定义规范》五段写 → 卡片字段齐全
- 注释不合规 → `missing` 非空 + `ew-types check` 报错

这两件事互为兜底：规范靠工具执行，工具靠规范取数。
