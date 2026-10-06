# Deep Reference: debugger (DAP)

> 参考手册级：调试横跨三 crate——`crates/dap`（**DAP 协议客户端/传输**）、`crates/project/src/debugger`（**会话状态机**）、`crates/debugger_ui`（**面板/视图**）。协议 schema 在独立 crate `dap_types`。三者构成"启动配置 → spawn 适配器 → initialize/launch → stopped/output 事件 → UI"完整链路。

## 1. `crates/dap`：协议与传输
| 类型 | 位置 | 角色 |
|---|---|---|
| `struct DebugAdapterClient` | [client.rs:32](../crates/dap/src/client.rs) | 与**一个** debug adapter 进程的 JSON-RPC 连接（`next_sequence_id`(L169)、`on_request<R>`(L191)、`on_request_ext`(L209)、`kill`(L173)、`add_log_handler`(L183)、`binary()`(L164)） |
| `struct SessionId(pub u32)` | [client.rs:19](../crates/dap/src/client.rs) | 会话 id（`from_proto`/`to_proto`） |
| `trait Transport` | [transport.rs:60](../crates/dap/src/transport.rs) | 抽象收发帧：`TcpTransport`(L472)、`StdioTransport`(L649)、`FakeTransport`(L739，测试) |
| `enum LogKind` / `IoKind` | [transport.rs:40](../crates/dap/src/transport.rs)/[46](../crates/dap/src/transport.rs) | adapter 日志 vs 线协议流量 |
| `enum RequestHandling<T>` | [transport.rs:53](../crates/dap/src/transport.rs) | 拦截请求的三种处理（Proxy/Respond/...） |
| `struct DapRegistry` | [registry.rs:41](../crates/dap/src/registry.rs) | 已注册 adapter（按 extension/language）：`adapters()`/`add_adapter`/`get_adapter` |
| `trait DapLocator` | [registry.rs:16](../crates/dap/src/registry.rs) | 把 `DebugRequest`→可执行的 `DebugAdapterBinary`（resolve/locate 两阶段） |
| `struct DebugAdapterBinary` | [adapters.rs](../crates/dap/src/adapters.rs) | adapter 可执行 + args + env + connection（TCP/stdio）配置 |
| `struct DebuggerSettings` | [debugger_settings.rs:5](../crates/dap/src/debugger_settings.rs) | 步进粒度等（`SteppingGranularity`）。→ [Settings-and-Themes.md](Settings-and-Themes.md) |
| `inline_value.rs` | [文件](../crates/dap/src/inline_value.rs) | `VariableLookupKind`/`VariableScope`/`InlineValueLocation`（hover 内联变量） |
| [proto_conversions.rs](../crates/dap/src/proto_conversions.rs)(20KB) | DAP types ↔ RPC proto（远程调试） |
| `dap.rs` | [文件](../crates/dap/src/dap.rs) | re-export `dap_types::*`（协议 schema：requests/events/definitions） |
`on_request<R: dap_types::requests::Request>`(transport.rs:762) 泛型按请求 command 路由 handler；`Initialize::COMMAND` 特殊处理握手(L886)。

## 2. `crates/project/src/debugger`：会话状态机（`session.rs` 115KB）
| 类型 | 位置 | 角色 |
|---|---|---|
| `struct Session` | [session.rs:691](../crates/project/src/debugger/session.rs) | **一次调试会话的 `Entity`**（持 `Vec<ThreadState>`、breakpoints、source lookups、client）。`new(...)`(L777) |
| `enum SessionState` | [session.rs:154](../crates/project/src/debugger/session.rs) | `Booting(Option<Task>)`（build/初始化）｜`Running(RunningMode)` |
| `struct RunningMode` | [session.rs:163](../crates/project/src/debugger/session.rs) | 运行中：`Arc<DebugAdapterClient>`+`DebugAdapterBinary`+worktree+`is_started`+`messages_tx` |
| `struct Thread` / `ThreadId(i64)` / `ThreadStatus` | [session.rs:121](../crates/project/src/debugger/session.rs)/[75](../crates/project/src/debugger/session.rs)/[99](../crates/project/src/debugger/session.rs) | 线程 + 其 `StackFrame` 栈（`struct StackFrame` L84） |
| `enum SessionEvent` | [session.rs:793](../crates/project/src/debugger/session.rs) | 广播给 UI：`StateChanged`、`LoadedContextChanged`、`ThreadEvent`、`BreakpointAdded`、`VariableUpdated`、`ConsoleOutput`、`Invalidated`、`CapabilityEvent`、`RunToCursorPositionEvent`、`Exited`… |
| `struct SessionSnapshot` | [session.rs:676](../crates/project/src/debugger/session.rs) | 只读视图（渲染取态） |
| `struct Watcher` / `DataBreakpointState` / `OutputToken` | L140/L148/L689 | watch 表达式、数据断点、输出游标 |
| `struct CompletionsQuery` | [session.rs:769](../crates/project/src/debugger/session.rs) | 控制台补全 |
| `struct DapStore` | [dap_store.rs:94](../crates/project/src/debugger/dap_store.rs) | 项目内所有 `Session` 集合（`Entity`，`Project` 字段 `dap_store`，见 [Project-Deep-Dive.md](Project-Deep-Dive.md)） |
| `struct BreakpointStore` | [breakpoint_store.rs](../crates/project/src/debugger/breakpoint_store.rs)(39KB) | 断点持久（`SourceBreakpoint`/`LogPoint`/条件，跨 buffer 迁移） |
| `dap_command.rs` | [文件](../crates/project/src/debugger/dap_command.rs)(59KB) | 每个 DAP 请求的高层封装（`Continue`/`Next`/`StepIn`/`StepOut`/`SetVariable`/`Evaluate`/`StackTrace`…） |
| `locators/` | [目录](../crates/project/src/debugger/locators) | 各来源 locator：`dap_locator`(模板)、`code_debug_locator`(任务)、`location` |
| `memory.rs` | [文件](../crates/project/src/debugger/memory.rs) | 内存读写（`ReadMemory`/`WriteMemory`） |
会话方法（`impl Session`）：`continue_thread`/`step_over`/`step_back`/`pause`/`disconnect`/`restart`/`set_breakpoints`/`run_to_position`/`variables`/`stack_frames`/`scopes`/`evaluate`/`source`——内部经 `dap_command` 发请求、把 `Event` 落入 `ThreadState`。

## 3. `crates/debugger_ui`：面板与视图
| 类型 | 位置 | 角色 |
|---|---|---|
| `struct DebugSession` | [session.rs:17](../crates/debugger_ui/src/session.rs) | 一个会话的根 `Entity`，`impl Item`(L101)；聚合 `running_state`(L88)/`session()->Entity<Session>`(L59) |
| `struct DebuggerPanel` | [debugger_panel.rs](../crates/debugger_ui/src/debugger_panel.rs)(83KB) | 右侧 dock `Item`：会话 tab、工具栏、`impl Item`；订阅 `SessionEvent` 刷新各列表 |
| `RunningState` | [session/running.rs](../crates/debugger_ui/src/session/running.rs)(74KB) | 会话运行视图的**布局容器**（多 tab：变量/调用栈/断点/控制台/内存/模块/已加载源） |
| Console | [running/console.rs](../crates/debugger_ui/src/session/running/console.rs)(32KB) | REPL：`Evaluate(context:repl)`、输入历史、`impl SearchableItem` |
| Variables | [running/variable_list.rs](../crates/debugger_ui/src/session/running/variable_list.rs)(59KB) | 变量树（`VariableReference` 懒加载 scopes） |
| Call Stack | [running/stack_frame_list.rs](../crates/debugger_ui/src/session/running/stack_frame_list.rs)(35KB) | 帧列表 + 线程切换 |
| Breakpoints | [running/breakpoint_list.rs](../crates/debugger_ui/src/session/running/breakpoint_list.rs)(56KB) | 断点列表/开关/条件 |
| Modules / Loaded Sources | [module_list.rs](../crates/debugger_ui/src/session/running/module_list.rs)/[loaded_source_list.rs](../crates/debugger_ui/src/session/running/loaded_source_list.rs) | 模块与源文件 |
| Memory | [running/memory_view.rs](../crates/debugger_ui/src/session/running/memory_view.rs)(36KB) | 十六进制内存查看 |
| Modals | [new_process_modal.rs](../crates/debugger_ui/src/new_process_modal.rs)(63KB)/[attach_modal.rs](../crates/debugger_ui/src/attach_modal.rs) | 新建/附加调试配置选择 |
| [dropdown_menus.rs](../crates/debugger_ui/src/dropdown_menus.rs) | 会话下拉/状态菜单 |
| [persistence.rs](../crates/debugger_ui/src/persistence.rs) | `SerializedDebuggerPanel`（恢复面板） |
`debugger_ui.rs`(19KB) 集中 `actions!`（`Start`/`Continue`/`Pause`/`StepOver`/`StepInto`/`StepOut`/`Stop`/`ToggleBreakpoint`/`RunToCursor`/`OpenDebugger`…）与注册 `init`。

## 4. 启动一次调试（真实流程）
```mermaid
graph TB
    A[Action::Start / 选配置] --> B[DapRegistry 找 adapter + DapLocator.locate]
    B --> C[resolve -> DebugAdapterBinary]
    C --> D[DapStore 建 Session, state=Booting]
    D --> E[spawn adapter 进程 -> Transport Stdio/Tcp]
    E --> F[DebugAdapterClient.initialize 握手 -> capabilities]
    F --> G[launch/attach 请求]
    G --> H[state=Running(RunningMode)]
    H --> I[adapter 发 Event: stopped/output/breakpoint]
    I --> J[Session 更新 ThreadState -> SessionEvent]
    J --> K[DebugSession/DebuggerPanel 各列表重绘]
```

## 5. 与编辑器/协作联动
- gutter 断点标记、当前帧高亮、inline value 由 editor 侧订阅 session（[Editor-Deep-Dive.md](Editor-Deep-Dive.md)）。
- 远程调试（历史）：DAP 原走 remote server（`proto_conversions` + `RemoteClient`）；本 fork 已移除 `crates/remote_server`，见 [Remote-Deep-Dive.md](Remote-Deep-Dive.md)。
→ 概览页 [Debugger.md](Debugger.md)。
