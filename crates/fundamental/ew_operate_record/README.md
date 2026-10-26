# ew_operate_record

嵌入宿主程序内部的**用户操作记录中间件**。只做一件事：

> 用户点了什么、对哪个对象、结果如何 → 记到数据库里。

系统自己执行的动作（定时任务、初始化、内部补偿、重试）**不经过这里**，也不该调用它——
这是本中间件刻意保留的边界，所以没有「操作者类型」这种字段，也就不需要判断「是不是系统干的」。

## 特点

* **一行调用**：`recorder.log(OperationLog::new(user_id, action))`，不引入异步运行时；
* **不拖慢业务**：只入队，落库在后台线程批量完成；
* **不会拖死业务**：队列满丢弃并计数，不阻塞界面/请求；
* **时间戳即主键**：记录时间由 `log()` 打点，写入线程保证严格递增（微秒），所以没有自增 id；
* **零外部依赖**：SQLite（bundled）随程序走，部署不需要装数据库。

## 快速开始

```toml
[dependencies]
ew_operate_record = "0.2"
```

```rust
use ew_operate_record::{OperationLog, Query, Recorder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let recorder = Recorder::open("data/operations.db")?;   // 目录不存在会自动创建

    // 只记录用户发起的操作
    recorder.log(
        OperationLog::new("u-1024", "删除订单")
            .with_user_name("李雷")
            .target("order:O-88231")
            .result("成功")
            .detail("客户重复下单，已退款 12.5 元"),
    )?;

    recorder.flush()?;                                      // 需要确已落库时调用

    let entries = recorder.query(&Query::new().user("u-1024").limit(20))?;
    for entry in entries {
        println!("{} {} {}", entry.occurred_at, entry.action, entry.target.unwrap_or_default());
    }

    recorder.close()?;                                      // 进程退出前调用
    Ok(())
}
```

`close()` 会排空队列并刷盘；即使忘记调用，最后一个 `Recorder` 析构时也会刷完。

## 接口

| 方法 | 说明 |
| --- | --- |
| `Recorder::open(path)` / `open_with(path, options)` | 打开（或创建）数据库并启动后台写入线程 |
| `Recorder::open_in_memory()` | 内存库，测试用 |
| `log(OperationLog)` | 入队；队列满返回 `QueueFull` |
| `log_user(user_id, action)` | 简写 |
| `flush()` | 等待此前入队记录全部落库（阻塞） |
| `close()` | 排空、刷盘、停线程；可重复调用 |
| `query(&Query)` | 条件查询，最新在前 |
| `count(&Query)` | 条件计数 |
| `prune_before(time)` | 删除早于某时间的记录（建议启动时清理一次） |
| `checkpoint()` | 把 WAL 并回主库并截断（备份前用） |
| `stats()` | `accepted / written / failed / dropped` 计数 |
| `last_error()` | 最近一次写库失败原因 |

`OperationLog` 字段：`user_id`、`user_name`、`action`、`target`、`result`、`detail`。
`user_id`/`action` 必填（非空、≤64 字符），`target` ≤128，`detail` 超过 4096 字符自动截断。
**发生时间不用填**：`log()` 打点，写入时保证严格递增。

`Query` 条件：`user`、`action`、`keyword`（动作/对象/详情/操作人模糊匹配）、`time_range`、
`limit`（1–1000，默认 100）、`before(上一页最后一条的时间)` 翻页。

## 写入语义

* `log()` 成功 = 已入队，**不保证已落库**；要确认请 `flush()`；
* 队列容量默认 4096，满了 `log()` 返回 `QueueFull`，本条丢弃（`stats().dropped` 累加）；
* 后台线程有积压才合批（默认单批 256），队列空立即提交，正常情况延迟在毫秒级；
* 记录时间取 `max(log() 时刻, 上一条 + 1µs)`：正常用户操作间隔远大于 1µs，所以等于是真实时间；
  即使系统时钟粒度粗或回拨，也不会产生重复主键；
* 写库失败会重试一次，仍失败则计入 `failed` 并记录 `last_error()`，不阻塞后续记录；
* SQLite 使用 WAL + `synchronous=NORMAL`，单进程内读写共存。

## 表结构

```sql
CREATE TABLE operation_log (
    occurred_at INTEGER PRIMARY KEY,  -- 发生时间（Unix 微秒），即记录的主键
    user_id     TEXT NOT NULL,        -- 操作人
    user_name   TEXT,                 -- 操作人显示名
    action      TEXT NOT NULL,        -- 做了什么，如「删除订单」
    target      TEXT,                 -- 操作对象，如 order:O-88231
    result      TEXT,                 -- 结果，如「成功」「失败：无权限」
    detail      TEXT                  -- 附加信息（参数、原因等）
);
CREATE INDEX idx_operation_log_user   ON operation_log (user_id, occurred_at DESC);
CREATE INDEX idx_operation_log_action ON operation_log (action, occurred_at DESC);
```

按时间排序、时间范围查询、翻页都直接走主键，不需要额外的时间索引。
数据库可直接用任何 SQLite 工具打开查看（`occurred_at` 除以 1e6 即 Unix 秒）。

## 性能（实测）

`cargo run --release --example bench`，Windows 开发机（NVMe），单线程写入 2 万条：

| 指标 | 结果 |
| --- | --- |
| 写入吞吐 | 20 000 条 / 315 ms ≈ **6.4 万条/秒** |
| 单条存储占用 | ≈ 193 字节 |
| 按操作人查 100 条 | 0.3 ms |

写入 2 万条约占 3.7 MiB；按 10 万次操作/天估算，一年约 7 GiB（配合 `prune_before` 控制留存）。

## 边界

* 只记「用户做的事」，不记系统内部动作——这是设计约定，由调用点保证；
* 时间戳由本进程保证递增：**同一个库文件只应由一个进程写**（WAL 允许多读者）；
* 队列满会丢记录（有计数）；对「一条都不能丢」的场景，提高容量并在关键操作后 `flush()`；
* 不做防篡改签名、不做字段级加密。数据库文件的访问控制和磁盘加密属于部署侧职责。

## 目录

```
src/lib.rs        对外导出与文档
src/log.rs        OperationLog / LogEntry
src/query.rs      查询条件
src/store.rs      SQLite 建表、批量写入、查询、清理
src/recorder.rs   有界队列、后台写入线程、统计
examples/         basic（用法）、bench（吞吐）
tests/            行为测试（查询、翻页、丢弃、关闭、清理、并发、迁移）
docs/设计.md      简短设计说明
```

## License

MIT OR Apache-2.0
