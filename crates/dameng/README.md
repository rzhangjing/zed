# dameng —— 达梦（DM8）Rust 数据库中间件

让 Rust 程序以熟悉的方式访问**达梦数据库（DM8）**：连接池、查询层、事务与保存点、数组绑定批量写、
LOB、统一错误模型、`tracing` 可观测、读写分离路由、迁移脚本执行入口。底层走**达梦官方 ODBC 驱动**
（`dodbc.dll` / `libdodbc.so`，驱动名 `DM8 ODBC DRIVER`），上层可选挂 [Toasty](https://crates.io/crates/toasty) ORM。

> 与「借 MySQL 方言」的路子不同：达梦在**解析层**不接受反引号（`-2007`）与 `ON DUPLICATE KEY UPDATE`，
> 所以本项目不假装达梦是 MySQL，而是按实测的达梦行为逐条实现（每条结论都有真机用例钉住）。

## 当前状态

| 里程碑 | 内容 | 状态 |
|---|---|---|
| M0–M6 | 骨架 · 驱动探针 · 错误与核心 · ODBC 后端 · 连接池 · 查询层 · 二进制/LOB/批处理/错误码目录 | ✅ 已过门禁（G2–G6） |
| M7 | `dm-migrate` + 本门面 + 示例 + 本文档 + 验收报告 | ✅ 已过 G7（附四条如实降级，见 [`docs/acceptance-report.md`](docs/acceptance-report.md) §4） |
| M8 | Toasty ORM 集成（`toasty-driver-dameng`） | ✅ 已过 G8：官方一致性套件真机 **851 / 851**（100%，Round 41 在 DPI 后端上复跑）；T8.0–T8.16 全部落盘——驱动 + vendored fork `crates/toasty-sql/` + 可运行示例 [`examples/toasty_crud.rs`](crates/toasty-driver-dameng/examples/toasty_crud.rs)。原 ODBC 后端上的非 BMP 文本缺陷（`type_primitives::ty_str`）在 DPI 后端上已消失，见 [`docs/toasty-fork.md`](docs/toasty-fork.md) 与下方「[Toasty 集成](#toasty-集成m8toasty-driver-dameng)」 |
| M9 | **默认后端换成 DPI 直连**（新增 `crates/dm-dpi`） | ✅ Round 41：运行时加载 `dmdpi.dll`/`libdmdpi.so`（不用配 ODBC 数据源）、`Backend` 对等实现、`dpi` feature 默认开、`dm-odbc` 保留可回退。官方一致性套件 **851 / 0**、离线 **866 / 0 / 126**，详见 [`docs/测试报告-dbtesta-dbtestb.md`](docs/测试报告-dbtesta-dbtestb.md) §15；Round 42 同一份大数据量基准复跑：**总耗时 815.5 s → 573.9 s（-29.6%）**、大字段插入 **2.71 倍**，详见 [`docs/性能测试报告.md`](docs/性能测试报告.md) §10 |

如实登记的缺口见下面「[已知边界](#已知边界与如实登记的缺口)」——本项目的一条硬规矩是
**不挑好看的数字写**：实测推翻了设计就改文档。

## 环境要求

1. **达梦 DM8 服务端**可达（本机开发用 `localhost:5236`）。
2. **达梦客户端库**（随达梦客户端/服务端安装包提供）：
   - **默认后端（DPI）**：只要 `dmdpi.dll`（Windows，通常在 `C:\dmdbms\bin`）或 `libdmdpi.so`（Linux）。程序运行时自动按
     `DM_DPI_LIB` → `DM_HOME/bin` → 默认安装路径 → 裸文件名的顺序找，**不需要配 ODBC 数据源、也不需要改 `PATH`**
     （Windows 用 `LoadLibraryExW(..., LOAD_WITH_ALTERED_SEARCH_PATH)`，同目录依赖由系统自动定位）。
   - **回退后端（ODBC）**：`DM8 ODBC DRIVER` 已注册（Windows `odbcad32.exe` 能看到；Linux 上 `libdodbc.so` 在 ODBC 驱动目录且 `odbcinst.ini` 有登记）。
3. **Rust**：MSRV `1.95`（edition 2024），因为 Toasty 0.11 要求 1.95。
   - Windows 上**必须用 msvc 宿主**：宿主默认的 `stable-x86_64-pc-windows-gnu` 链接期会报
     `error calling dlltool 'dlltool.exe': program not found`。建议
     `rustup override set stable-x86_64-pc-windows-msvc`，或每条命令都写 `cargo +stable-x86_64-pc-windows-msvc …`。

## 快速开始

```toml
# Cargo.toml（本 crate 尚未发布到 crates.io，先用 path/git 依赖）
[dependencies]
dameng = { path = "../dameng/crates/dameng" }
# 只在使用 `#[derive(FromRow)]` 时需要（原因见下面那条注）
dm-query = { path = "../dameng/crates/dm-query" }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

> **用 `#[derive(FromRow)]` 时为什么要自己再依赖 `dm-query`**：派生宏生成的代码引用
> **全限定路径** `::dm_query::…`（与 `serde` 同款做法：不往你的命名空间里塞任何名字），
> 而 `::dm_query` 只在**你自己的依赖表**里有 `dm-query` 时才会进入 extern prelude——
> 门面 `dameng` 依赖它并不等于你的 crate 也依赖它。不加会报
> `use of unresolved module or unlinked crate 'dm_query'`。不用派生宏就不需要这一行。

```rust,no_run
use dameng::prelude::*;

#[tokio::main]
async fn main() -> Result<(), DmError> {
    // 达梦 ODBC 驱动真正吃的是连接串；口令里的 @ : / 会被自动转义
    let cfg = DmConfig::parse(
        "DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA",
    )?;

    // 池大小就是并发上限：达梦不支持 ODBC 异步，每条连接占住一个线程
    let pool = DmPool::connect_with(cfg, ExecutorConfig::for_pool(4)).await?;

    let mut conn = PooledConn::acquire(&pool).await?;
    let row = conn
        .query_one("select 1 as N from dual", ())
        .await?
        .expect("select 1 必定返回一行");
    println!("N = {}", row.get::<i32>("N")?);

    // 参数化查询：`?` 是位置参数，参数用元组（单参数写 `(x,)`）
    let row = conn
        .query_one("select ? as S from dual", ("你好，达梦",))
        .await?
        .expect("select 必定返回一行");
    println!("S = {}", row.get::<String>("S")?);

    drop(conn);
    pool.close().await?;
    pool.wait_closed().await;
    Ok(())
}
```

四个可运行示例（没配 `DM_TEST_DSN` / `DM_SERVER` 时会打印一行原因后正常退出）：

```powershell
$env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
cargo +stable-x86_64-pc-windows-msvc run -p dameng --example basic_query   # 查询、建表、批插、FromRow 派生
cargo +stable-x86_64-pc-windows-msvc run -p dameng --example pool          # 12 个任务抢 4 条连接，看池指标与关池
cargo +stable-x86_64-pc-windows-msvc run -p dameng --example txn           # 提交/回滚/保存点/未提交即丢弃
cargo +stable-x86_64-pc-windows-msvc run -p dameng --example error_handling # 归一化分类 + 驱动原话 + 重试判定
```

`examples/` 里的每个示例都是**可复制的最小用法**，而不是玩具：写完之后 `drop(conn)`、`pool.close().await`、
`pool.wait_closed().await` 三步收尾是推荐的常驻进程关闭姿势。

## Toasty 集成（M8，`toasty-driver-dameng`）

**挂载方式**：Toasty 的 URL→驱动分派是**闭 match**（`toasty/src/db/connect.rs`），第三方驱动
无法按 scheme 注册自己 ⇒ `dameng://…` **走不通**，必须把驱动**直接**喂给 `Db::builder()`：

```toml
[dependencies]
toasty = { version = "0.11", default-features = false, features = ["jiff", "rust_decimal"] }
toasty-driver-dameng = { path = "../dameng/crates/toasty-driver-dameng" }
dm-core = { path = "../dameng/crates/dm-core" }   # 构造 DmConfig
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust,no_run
use dm_core::DmConfig;
use toasty_driver_dameng::DamengDriver;

#[derive(Debug, toasty::Model)]
struct Note {
    #[key]
    #[auto]
    id: i64,
    title: String,
    #[unique]
    slug: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = DmConfig::from_connection_string(
        "DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA",
    )?;
    let driver = DamengDriver::from_config(cfg).await?;
    let mut db = toasty::Db::builder()
        .models(toasty::models!(crate::*))
        .build(driver)
        .await?;

    db.push_schema().await?; // 裸 CREATE TABLE，没有 IF NOT EXISTS
    let note = toasty::create!(Note { title: "达梦", slug: "dm" })
        .exec(&mut db)
        .await?;
    assert!(note.id > 0); // 自增主键由 @@IDENTITY 回填（达梦没有 RETURNING）
    Ok(())
}
```

完整可运行示例：[`crates/toasty-driver-dameng/examples/toasty_crud.rs`](crates/toasty-driver-dameng/examples/toasty_crud.rs)
（建表 → 插入并回填主键 → 唯一键查询 → 更新 → 事务提交/回滚 → 删除）。未配 `DM_TEST_DSN`
时它打印一行「跳过」并以退出码 0 结束，所以 CI 里跑也不会红：

```text
$env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
cargo +stable-x86_64-pc-windows-msvc run -p toasty-driver-dameng --example toasty_crud
```

**用之前必须知道的限制**（与 `docs/设计方案.md` §9.7 逐条对应）：

| 限制 | 说明 |
|---|---|
| MSRV | **Rust ≥ 1.95** + edition 2024（Toasty 的硬约束，不是本项目的取舍） |
| URL scheme | `dameng://…` 走不通，只能 `Db::builder().build(driver)` |
| 两层连接池 | `dm-pool` 的 `PoolConfig::max_size`（物理连接上限，默认 10）与 Toasty 自带 `deadpool` 的应用侧上限（`Db::builder().max_pool_size()`）**相乘**要落在服务端预算内（`MAX_SESSIONS=1500`）。驱动**故意不实现** `Driver::max_connections()`——两处都设会互相打架，物理上限**只**在 `PoolConfig` 上设 |
| 日志安全 | Toasty 把 `Value::Bytes` 渲染成 `<N bytes>`（不泄露内容）；但**默认不打印参数值**，要看就开 `Db::builder().log_statement_params(true)` |
| JSON / `#[document]` | `stmt::Type::Bytes` **没有 JSON 表示** ⇒ 二进制列不能进文档 / JSON 嵌入 |
| `Vec<scalar>` 里的真 `null` 元素 | 列表原地删除（`remove(v)`）走 `JSON_TABLE … PATH '$'` 的**裸文本**列重建，真 `null` 元素会被当成字符串 `"null"` 写回（与真字符串 `"null"` 无法区分）。官方套件的 `Vec<String>` 用例不含 `null`，故不影响通过率；已在 `docs/设计方案.md` §12.2 第 24 条如实登记 |
| 上游跟踪预算 | **2–5 人日/月**（上游 6 个月 9 次破坏性发布） |

**一致性**：官方一致性套件（`toasty-driver-integration-suite 0.11.0`，挂在 `--features real` 后）
在 DM8 本机跑出 **850 passed / 1 failed（总 851 条 = 99.88%，Round 28 复跑；`cargo fmt --all` 后的最终修订上再复跑一次同为 850/1，日志 `target/m28_suite_final.log`）**；唯一那条已定位到根因，不是未知缺陷：

1. `type_primitives::ty_str` —— 非 BMP 字符（`🦀`）在达梦 ODBC 下不保真（见上「已知边界」第 1 条）。

历史上先后被关掉的红：`upsert_explicit_assignments_override_model_defaults`（Round 18 的
`upsert_targeted_ignore`：达梦没有 `DO NOTHING`，fork 渲染成「命中分支条件恒假的 MERGE」+ 驱动按受影响行数把「冲突被忽略」返回成空结果），
以及 `Vec<scalar>` 一族 3 条（Round 20 的 `vec_scalar`/`bind_list_param`/`native_array_set_predicates` 三件套：
存储 = `CLOB` 里的 JSON 数组文本，集合谓词 = `JSON_CONTAINS`/`JSON_OVERLAPS`/`JSON_LENGTH`，
编解码在 `crates/toasty-driver-dameng/src/list_json.rs`）——`type_net::network_address_collection_round_trip`
与两条带 `Vec<String> tags` 的 `crud_upsert` 用例因此转绿，`crud_upsert` 子集**已全绿**；随后 **Round 21**（`vec_pop`/`vec_remove_at`：`CASE` 夹空列表 + `JSON_REMOVE`）与 **Round 23**（`vec_remove`：`MERGE … USING(<派生表重建>)`）又把数组原地删除三件套补齐（新跑 4 + 1 条全绿）；**Round 25** 则把 `BigDecimal` 打通（`crates/dm-odbc` 的 `parse_decimal` 改 `Decimal::from_str_exact` + 装不下时按原文回文本，顺带修掉「超出 96 位尾数被静默舍成约 29 位有效数字」的缺陷；驱动 `stmt::Type::BigDecimal` 吃 `Text`/`Decimal`/`Double`/`Real`），新跑 2 条全绿。

通过率推进过程（每步都被真机用例钉住）：**156/639 → 621/174 → 720/75 → 769/18 → 778/8 → 779/7 → 782/4 → 783/3 → 784/2 → 791/5 → 796/4（总 800，Round 18）→ 815/1（总 816，Round 20）→ 819/1（总 820，Round 21）→ 820/1（总 821，Round 23）→ 822/1（总 823，Round 25）→ 844/1（总 845，Round 27）→ 850/1（总 851，Round 28）**。

```text
# ⚠️ 必须先设 DSN：套件生成的用例全部要连真库，没设会满屏假失败（driver_suite.rs:58 直接 panic）
$env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --features real --test driver_suite -- --test-threads=4
```

端到端回归（六步 CRUD + 自增回填）：`cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --test toasty_real -- --include-ignored --test-threads=1`。

## 可观测性（`tracing` + 慢查询告警）

span 树固定五段：`borrow → connect → exec → fetch → recycle`。诊断日志里的 SQL 一律先过**指纹**
（`dm_query::Fingerprint`：字面量归一化成 `?`、大小写与空白折叠），所以日志里不会出现你的数据；
要看驱动真正发出的**原始** SQL，得显式开 `DM_TRACE_SQL=1`（只用于排查，见 `docs/设计方案.md` §12.2 第 21 条）。

**慢查询告警是内建的**：耗时 ≥ `DmConfig::slow_query_threshold`（默认 **1 s**）时打一条 `WARN`，
带 `op`/`sql_fp`/`rows`/`affected`/`ms`/`threshold_ms` 六个字段（真机原文）：

```text
WARN 慢查询：耗时 53 ms ≥ 阈值 1 ms op="query" sql_fp=b328140e294b455e select rownum as n from (select id from dm_q_real_seed where id >= ?) a, … rows=100000 ms=53 threshold_ms=1
```

调阈值或关掉：`DmConfigBuilder::slow_query_threshold(Duration::from_millis(200))`、
`without_slow_query_log()`，或环境变量 `DM_SLOW_QUERY_MS`（`0` = 显式关闭）。
池侧的原子计数走 `PoolGuard::conn_stats()` → `ConnStatsSnapshot`（`ops` / `rows` / `errors` /
`total_nanos` / `slow_ops`）。

**语句缓存是内建的**：每条物理连接一份 LRU 预处理语句表（键是 SQL 文本本身，精确匹配），
`dm-query` 的 `query`/`execute` 命中时省掉一次 `SQLPrepare`。容量由
`DmConfigBuilder::pool_stmt_cache_size(n)`（默认 **16**）或环境变量 `DM_STMT_CACHE_SIZE` 决定，
`0`（或 `without_stmt_cache()`）表示关闭；连接复位会清空缓存，执行失败的语句不进缓存。
观测口是 `PoolGuard::stmt_cache_stats()`（`hits` / `misses` / `evictions` / `size` / `capacity`）。
真机实测（`cargo bench -p dameng --bench latency` 的 D/E 口径，同一条 `insert … values (?, ?)`
各 1000 轮）：缓存开 **114.2 µs** vs 关 **161.5 µs** ⇒ **省 47.3 µs ≈ 29.3%**（`hits=1099, misses=1`）。
**只有「还能再执行」的语句会进缓存**：达梦上结果集被取尽之后，那条语句句柄就再也执行不了了
（`24000/-70027 无效的游标状态`，真机七场景见 `crates/dm-odbc/examples/cursor_reuse_probe.rs`），
所以**查询语句不跨调用复用**（每次重新 `prepare`，但执行仍走 `SQLExecute`），DML/DDL 形态照常复用。
细节、「为什么不在驱动层」与准入判据见 `docs/设计方案.md` §5.4 末、§7 补注 17、能力表 `CAP-C-11`。

## 代码结构

依赖方向严格单向：`dm-error ← dm-core ← {dm-odbc, dm-dpi} ← dm-pool ← dm-query ← {dameng, toasty-driver-dameng}`。

| crate | 职责 | `unsafe` | 关键依赖 |
|---|---|---|---|
| `dm-error` | `DmError` / `DmErrorKind` / `DiagRecord` / 分类判定 / 已实测错误码目录 | 否 | `thiserror` |
| `dm-core` | `DmConfig`、连接串解析、`DmValue`、`DataType`、`ColumnMeta`、能力表、`Backend` trait | 否 | `chrono`(opt)、`rust_decimal`(opt) |
| `dm-dpi` | **默认后端**：直连 DPI（`dmdpi.dll`/`libdmdpi.so`）——动态加载、登录、会话字符集、绑定、数组绑定、结果集、LOB、诊断；`Backend` 实现 | ✅ 受控（FFI 全在 `ffi.rs`/`handle.rs`） | `dm-core`（无第三方） |
| `dm-odbc` | `OdbcConnection` / `OdbcStatement` / 绑定 / 取值 / 诊断 / LOB；`Backend` 实现（可回退后端） | ✅ 受控 | `odbc-api`、`dm-core` |
| `dm-pool` | `DmPool`、借还、探活、复位、回收任务、`PoolStats`、`Router`、阻塞桥接、`DefaultBackend` 别名 | 否 | `dm-odbc`/`dm-dpi`（二选一）、`tokio` |
| `dm-query` | `PooledConn`、`Row`/`RowSet`/`RowStream`、`Args`/`IntoArgs`、`FromRow`、`Transaction`、批处理 | 否 | `dm-core`、`dm-pool` |
| `dm-macros` | `#[derive(FromRow)]` | 否 | `syn`/`quote`/`proc-macro2` |
| `dm-migrate` | 迁移脚本执行 + 版本记录表 `DM_SCHEMA_HISTORY` | 否 | `dm-query` |
| `toasty-driver-dameng` | Toasty `Driver`/`Connection` 实现 + `Capability::DAMENG` | 否 | `dm-query`、`toasty-core`、`toasty-sql`(fork) |
| `toasty-sql` | **vendored fork**（上游 `toasty-sql` 0.11.0，tag `toasty-v0.11.0`）：crate 内 `Flavor::Dameng` 的 DM 方言渲染；`publish = false`，不发 crates.io | 否 | `toasty-core` |
| **`dameng`** | **门面**：`prelude` + 常用类型 re-export + `raw::*` 下层入口 | 否 | 全部 |

**CI 强校验**：`dm-error` / `dm-core` 两个底层 crate **不得依赖 `tokio`、不得依赖 `odbc-api`、不得依赖任何第三方数据库驱动 crate**。

## 三条实测约束（用之前务必知道）

1. **达梦不支持 ODBC 异步**（`SQL_ATTR_ASYNC_ENABLE` 报 `HYC00/-70054`）⇒ 所有连接都占住一个线程，
   **连接池大小就是并发上限**。
2. **字符集**：库默认 `GB18030`，文本一律按宽字符（`SQL_C_WCHAR`/UTF-16）传递；绝不能启用
   `odbc-api` 的 `narrow` feature（那条路是坏的）。
3. **`varchar(n)` 按字节计量**（服务端 `LENGTH_IN_CHAR=0`）⇒ `varchar(100)` 只能存 50 个汉字，
   要按字符数约束请用 `varchar(100 char)` 或 `CLOB`。

## 已知边界与如实登记的缺口

- **非 BMP 字符（增补平面，如 emoji）在达梦 ODBC 下不保真**，且与尺寸无关。绕过办法：
  改存 UTF-8 字节进 `VARBINARY`/`BLOB`。
- ~~**大 `BLOB` 写走「预分配满长度缓冲」**（`SQLPutData` 延迟输入未实测、未实现）⇒ 内存峰值随值放大。~~
  **Round 14 已补**：达梦**接受**延迟输入（探针 [`crates/dm-odbc/examples/putdata_probe.rs`](crates/dm-odbc/examples/putdata_probe.rs) 8 个用例实测会回
  `SQL_NEED_DATA`），分块写在 `Backend::execute_lob` + `LobSource`（`LobBytes` 内存值 / `LobReader` 真流）+
  `PooledConn::execute_lob` 上落地：绑定用 `SQL_DATA_AT_EXEC`/`SQL_LEN_DATA_AT_EXEC`，执行走
  `SQLParamData` → `SQLPutData` 循环，每趟 ≤ `LOB_WRITE_CHUNK_BYTES`（64 KiB）⇒ **内存峰值不随值放大**。
  真机 3 MiB BLOB 与 720 000 字节 CLOB 往返逐字节（逐字符）一致；预分配缓冲（`LobWriter`）退为降级路径。
  一个容易踩的坑（已在代码里用类型消灭）：延迟输入的 `ParameterValuePtr` 是驱动要原样还回来的 **token**，
  所以喂数据器不能移动——`bind_params_with_lob` 返回 `Box<LobFeeder>`。
- **批量写增速随编译档位变化**：release 档 1000 行约 **13×–15×**（未优化档 32×–41×）。
  「33×」只在未优化档成立；10000 行那格的倍数**跨运行不可复现**（双峰 7.3×–13.3×），不作为结论。
- **中途违约是语句级错误**：达梦在 `SQLExecute` 直接返 `-6602`，拿不到行下标，且服务端**一行都不落地**（原子），
  所以 `BatchOutcome.failed_at` 在达梦上恒为 `None`。
- ~~`dhat` / RSS 曲线**未做**（要挂全局分配器＝加依赖）；分块读/写都只做了功能性验证（峰值口径见上一条与本节末的 LOB 两条）。~~
  **Round 16 已采到「不放大」这一半**：测试进程里挂一个**无依赖**的 `GlobalAlloc` 计数分配器
  （[`crates/dm-query/tests/meter/mod.rs`](crates/dm-query/tests/meter/mod.rs)，只数**我们自己**的分配——驱动 DLL 内部的 `malloc` 在它的 CRT 里，看不见），
  门禁内离线判据 [`crates/dm-query/tests/alloc_budget.rs`](crates/dm-query/tests/alloc_budget.rs)（4 条：2 条判据 + 2 条仪器自检，各带阳性对照或量级断言），真机四条在
  [`crates/dm-query/tests/query_real.rs`](crates/dm-query/tests/query_real.rs) 末尾（`real_*_memory_does_not_follow_*`，`#[ignore]`）。真机读数（日志 `target/m16_real_budget.log`）：
  **行流** 1 万 → 10 万行峰值恒 **24.9 KiB**、取完净增 **0**；**分块读** 1 MiB → 4 MiB 峰值 **25.4 → 25.6 KiB**（684 趟）；
  **分块写** 1 MiB → 4 MiB 峰值 **65.6 → 65.3 KiB**（＝一趟 `LOB_WRITE_CHUNK_BYTES`）。三个阳性对照都在：
  全量物化 1 万行 `RowSet` **2442.6 KiB**、攒整值读 4 MiB **6160.0 KiB**、窗口内建 4 MiB 源 **4096.0 KiB**。
  **Round 17 把进程 RSS 那一半也补上了**：探针 `meter::rss_kib()` 走 `K32GetProcessMemoryInfo`（Windows）/
  `/proc/self/statm`（Linux）——**整进程口径，驱动 DLL 内部的 `malloc` 也看得见**。真机判据
  `real_row_stream_rss_does_not_follow_the_row_count`（日志 `target/m17_real_rss2.log`）：热身 100 行后基线 **19 668 KiB**
  ⇒ 1 万行后 **20 180 KiB** ⇒ 10 万行后 **20 204 KiB**（**行数 ×10 只多 24 KiB**）；阳性对照（`RowSet` 全量物化 10 万行）
  **+24 860 KiB**。⚠️ 对照必须让 `RowSet` **活过读数**，否则临时值当场 drop、大块页立刻还回操作系统（实测只涨 432 KiB，假绿）。
  **仍未做**：100 万行那一档、驱动 DLL 内部按请求的分配曲线（RSS 只给总量）。
- **`dm-query` 侧的 `LobRef`/`LobStream`/`LobWriter` 已补齐**（Round 13）：驱动层的游标状态搬到
  `OdbcStatement` 上（`dm_odbc::lob::LobState`），`Backend` 加三个带默认实现（返 `Unsupported`）的分块读方法
  （`lob_start`/`lob_next_row`/`lob_chunk` + `LobChunk { More, Null, Done }`），`dm-query` 侧给出与 `RowStream`
  同构的 `PooledConn::lob_stream(..) -> LobStream`，`LobWriter` 由 `dm-query` 重导出。
  **大 LOB 列必须在查询时走非绑定路径**（同一列不能混用 `SQLBindCol` 与 `SQLGetData`），
  且 `read_chunk` 把「一行里哪一列」的选择交给 `LobRef`；离线判据
  [`crates/dm-query/tests/lob.rs`](crates/dm-query/tests/lob.rs)（9 条），真机判据
  [`crates/dm-query/tests/query_real.rs`](crates/dm-query/tests/query_real.rs) 的 `real_lob_stream_reads_a_one_mib_clob_in_windows`
  （1.2 MB CLOB 分块读逐字节一致，单趟 ≤ 6144 字节 ⇒ 不是整值读）。
- ⭐ **保存点回滚会多一次往返**：达梦 ODBC 驱动会把 `ROLLBACK TO SAVEPOINT` 误判成「事务已结束」，
  于是紧随其后的 `commit()` **返回成功却什么都不做**（整个事务丢）。中间件在
  `Transaction::rollback_to` 之后补发一条无害语句（`select 1 from dual`）绕开它；
  如果绕过 `Transaction` 自己用裸 SQL 发 `ROLLBACK TO SAVEPOINT`，仍会踩到这个相邻关系
  （驱动层探针 R0–R21 与机制见 `docs/设计方案.md` §12.2「M7 实测发现」第 1 条）。
- **Toasty 集成（M8）已收口：上游套件用作门控的能力位全部翻真**（Round 26/27 起连 `#[document]` 集合字段也不再是 `false`——文档列 = `CLOB` 里的规范 JSON 文本，读取按**叶子类型**渲染成 `JSON_VALUE` + `CAST`/`HEXTORAW`/`CASE` 的达梦形状，见 `docs/toasty-fork.md` 第 25 条与设计方案 §12.2 第 27 条 / §14 V-30；顺带修掉两个真缺陷：绑定时间参数的 ODBC 精度声明与驱动侧的微秒截断）：`Vec<…>` 集合字段（`vec_scalar`）**Round 20 已落地**——存储 = `CLOB` 里的规范化 JSON 数组文本，集合谓词由 fork 渲染成 `JSON_CONTAINS` / `JSON_OVERLAPS` / `JSON_LENGTH`，`Value::List` 的编解码在驱动侧 `crates/toasty-driver-dameng/src/list_json.rs`（`vec_scalar` / `bind_list_param` / `native_array_set_predicates` 三件套同时翻真）；**Round 18** 落地的 `upsert_targeted_ignore`（定向忽略 upsert）——达梦没有 `DO NOTHING`，fork 渲染成「命中分支条件恒假（`WHERE 1 = 0`）的 `MERGE INTO`」，驱动再按受影响行数把「冲突被忽略」返回成**空结果**（`or_ignore()` 要的正是 `None`；真机探针第 16/17 节逐条验证，见 `docs/toasty-fork.md` 第 18/19/20 条与设计方案 §12.2 第 22/23 条）。**Round 21 起** `vec_pop` / `vec_remove_at`（列表原地删除）也已落地 —— fork 渲染成 `CASE`（空列表直接回原列）+ `JSON_REMOVE(列, '$[' || (JSON_LENGTH(列) - 1) || ']')` 与 `JSON_REMOVE(列, '$[' || idx || ']')`，真机探针第 18/19 节逐条验证，新跑 4 条套件用例全绿；**Round 23 起** `vec_remove`（删掉每个匹配元素）也已落地 —— 达梦既没有可用的过滤路径（`'$[*]?(@ != …)'` 判 `[HY000/-3102]`），`SET` 里的相关子查询重建又被 `[HY000/-4083]` 拒，可行形状是把重建放进 `MERGE … USING(<派生表重建>)`：`LEFT JOIN JSON_TABLE(src.列, '$[*]' COLUMNS (…)) jt ON 1 = 1` + `LISTAGG(CASE WHEN jt."v" <> ? THEN JSON_QUOTE(jt."v") END, …)`（`LEFT JOIN` 是必需的——内连接语义下「元素全被删掉」的行在派生表里没有分组，`MERGE` 会根本不碰它，见 `docs/toasty-fork.md` 第 21/22 条与设计方案 §12.2 第 24 条）；**Round 24** 把 `crates/toasty-driver-dameng/src/capability.rs` 里 6 处 `(待实测)` 全部真机结案（探针第 23 节，日志 `target/m24_probe23c.log`）——`VARCHAR` 的**真上限是 32767 字节**（旧值 8188 只是默认 8 KiB 页下的**行内**上限，`VARCHAR(32768)` 才报 `-6121`）⇒ `StorageTypes::varchar` 由 `Some(8188)` 改为 `Some(32767)`，其余 5 项声明（`default_string_type` / `max_identifier_length` / `max_auto_increment_integer_width` / `sql_nulls_first_on_asc` / `schema_mutations` + `cte_with_update`）经实测**维持原值**（用例数不变，逐条证据见 `docs/toasty-fork.md` 第 23 条）。一致性套件 **850 passed / 1 failed**（总 **851** 条 = 99.88%，Round 28 复跑，日志 `target/m28_suite2.log`；Round 25 的 `bigdecimal_implemented`、Round 26/27 的 `document_collections` 与 Round 28 的 `upsert_unique` / `upsert_branch_assignments` 分别新跑 2 条 / 22 条 / 8 条用例转绿，Round 28 另有 2 条 `not(…)` 负向用例不再运行），唯一那条红是达梦 ODBC 驱动的非 BMP 缺陷。upsert 四位（`upsert_primary_key` / `upsert_unique` / `upsert_branch_assignments` / `upsert_targeted_ignore`）现已全部为 `true`：`incoming()` 渲染成 `MERGE` 的 `USING` 源列 `s."s_N"`，驱动的 `RETURNING` 模拟在存在冲突目标时**按冲突目标列回查**（唯一键冲突时主键是原值，按主键回查会落空 —— 见设计方案 §12.2 第 28 条）。已实现的部分：上游 `toasty-sql` 0.11.0 已 vendored 为 `crates/toasty-sql/`（`publish = false`），DM 方言由 crate 内 `Flavor::Dameng` 表达，**方言身份**仍复用 `Dialect::Mysql`（`Capability.sql`，见设计方案 §9.1.1/§9.8）；upsert 渲染成 `MERGE INTO`，达梦没有的 `INSERT … RETURNING` 由驱动侧「逐行写入 + 回查」模拟（**回查键：有 upsert 冲突目标时用冲突目标列，否则用主键**；`upsert_primary_key` / `returning_from_insert` 均已置 `true`，取舍见设计方案 §12.2 第 22 条与第 28 条）。
- **L2 契约测试（CT-01…CT-12）的实际覆盖**：逐条 `文件:行` 映射见 [`docs/ct-mapping.md`](docs/ct-mapping.md) —— **覆盖 12 条**、**部分覆盖 0 条**、**无对应用例 0 条**。Round 10 补齐四处：**CT-02**（离线断言渲染不含 `DSN=` + 真机「只用结构化字段建连并下语句」）、**CT-03**（把本来就有、只是没登记的真机 8 KiB / 10 MiB 证据写进表）、**CT-10**（守卫搬一份到离线：`size_of::<SqlChar>() == 2` + `odbc_sys::WChar` 宽度 + UTF-16 往返）、**CT-11**（补「含 NUL」「超 4 KiB」两条离线断言）；**CT-12 的判据已改写**——原文点名的 `TruncationInfo` 是 `odbc-api` 的 `BinColumn` 才有的形状，本项目走自实现分块 `SQLGetData`，改成断言 `crates/dm-odbc/src/result.rs` 的 `written_units` 指标语义。**Round 12 补齐 CT-09**：离线释放顺序断言 [`crates/dm-pool/tests/drop_order.rs`](crates/dm-pool/tests/drop_order.rs)（4 条，钉住「语句释放早于连接的 `reset` / `close` / `drop`」），判据同时按实现口径改写（`SQLFreeHandle` 的调用序在 `odbc-api` 的 `StatementImpl` 内部，我们只对「不自留句柄 + 语句先于连接」负责）。另外，「设计方案 §10.1 的 cdylib 假 ODBC + JSON 剧本引擎」**在本仓库里不存在**——假后端都在 `dm-core::Backend` trait 层（`dm-pool/src/testing.rs`、`dm-query/tests/common/mod.rs`），唯一贴 ODBC 句柄的是 `dm-odbc/src/diag.rs:264` 的 `FakeHandle`；文档已就地更正。
- **麒麟（aarch64）实机未验（AC-08 / R-07）**：交叉编译这半已过（`cargo check --workspace --all-targets`（含 `--all-features`）在 `aarch64-unknown-linux-gnu` 与 `x86_64-unknown-linux-gnu` 上均 exit 0，日志见 `target/cross_*.log`），且正是它揪出 `odbc-api` 的宽字符开关缺陷（**必须显式开 `wide`**，否则非 Windows 上 `SqlChar = u8`，`crates/dm-odbc/src/diag.rs` 的窄字符守门直接编译失败）；剩下的是**运行期**——麒麟真机跑通与 unixODBC 的 `SQLWCHAR` 宽度确认（见设计方案 §11.3）。

## 测试与门禁

**离线**（不需要数据库，适合日常与 CI）：

```powershell
# `--exclude` 不能省：`--all-features` 会打开 toasty-driver-dameng 的 `real`，
# 把 851 条真机用例拉进离线门禁（没设 DSN 时立即满屏失败，看起来像满屏回归）
cargo +stable-x86_64-pc-windows-msvc test --workspace --all-features --exclude toasty-driver-dameng
cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng
cargo +stable-x86_64-pc-windows-msvc clippy --workspace --all-targets --all-features -- -D warnings
cargo +stable-x86_64-pc-windows-msvc fmt --all --check
# 文档链接门禁（docs.rs 用的是同一套检查；Round 11 起加入）
$env:RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links'
cargo +stable-x86_64-pc-windows-msvc doc --workspace --all-features --no-deps
Remove-Item Env:\RUSTDOCFLAGS
```

上面这些（外加 feature 矩阵、交叉编译与真机组）已落成两个脚本，一条命令跑全：

```powershell
pwsh scripts/gates.ps1 -SkipReal          # 离线 + 交叉（CI 的默认形态）
# 只有 Windows PowerShell 5.1（本机没有 pwsh）且策略为 Restricted 时改用：
# powershell -ExecutionPolicy Bypass -File scripts\gates.ps1 -SkipReal
pwsh scripts/gates.ps1 -Only fmt,clippy   # 只跑名字匹配的阶段
$env:DM_TEST_DSN='DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
pwsh scripts/gates.ps1                    # 再追加真机组：官方一致性套件 / toasty_real / dm-odbc / dm-query / dm-migrate / dm-pool / 示例
```

> `scripts/gates.ps1` 必须以 **UTF-8 with BOM** 保存：Windows PowerShell 5.1 会把无 BOM 的 UTF-8 当 ANSI（本机 CP936）解码，中文被打碎后脚本直接语法错误（实测踩过）。`pwsh` 7.x 不受影响；`scripts/gates.sh` 是 bash 脚本，反而**不能**带 BOM。
>
> 官方一致性套件那一段**按设计不会全绿**：唯一红的是已登记在册的非 BMP 文本缺陷（`type_primitives::ty_str`，见 [`docs/toasty-fork.md`](docs/toasty-fork.md)「真机实测发现」第 16 条）。所以脚本对这个阶段不用「非零即红」，而是按「**失败条数恰为 1、且失败名在白名单里**」判 PASS，并把放行的那一条打印出来；要全绿（审计/发布）加 `-StrictSuite` / `--strict-suite`。
>
> 真机阶段的 `--test-threads=1` 不可省：这些用例共用固定表名（`DM_Q_REAL_SEED`、迁移记录表…），并行会互相 `DROP`/建表 ⇒ 假失败。实测（Round 30 首跑漏了它）`dm-query` 红 17 条、`dm-migrate` 红 2 条、`dm-pool` 的会话数被别的用例抬高；补齐后全部回绿。

麒麟（或任意 Linux）侧对应 [`scripts/gates.sh`](scripts/gates.sh)（开关 `--skip-real` / `--skip-cross` / `--skip-soak` / `--coverage` / `--strict-suite`）。两个脚本逐阶段对齐：每个阶段是一条真命令，非零退出码即判红，日志落 `target/gates/NN-<阶段>.log`，末尾打印汇总并点名失败阶段；阶段清单与「设计方案 §10.5 CI 流水线」的对应关系见 [`docs/设计方案.md`](docs/设计方案.md) §10.5。

**覆盖率**（AC-01 判据：四个 L1 库 ≥ 85%；工具 `cargo-llvm-cov 0.9.1`）：

```powershell
# 实测（行覆盖，Round 15 clean 后复采）：dm-error 95.28% / dm-core 91.91% / dm-pool 89.47% / dm-query 90.30%
# dm-migrate 68.69% → 计入真机套件 87.65%；dm-odbc 41.15% → 计入真机 73.41%（纯 FFI 转发行按判据排除）
# 注：Round 13/14 扩码后 dm-query 曾掉到 84.86%（低于判据），Round 15 补离线用例后回到 90.30%（详见 docs/acceptance-report.md §2.5）
foreach ($p in 'dm-error','dm-core','dm-pool','dm-query','dm-migrate','dm-odbc') {
  cargo +stable-x86_64-pc-windows-msvc llvm-cov clean --workspace   # 必须：否则会合并上一次构建的 profraw，数字失真
  cargo +stable-x86_64-pc-windows-msvc llvm-cov -p $p --all-features
}
```

**真机**：所有需要数据库的用例都标了 `#[ignore]`，并且只在设了 `DM_TEST_DSN` 时才跑；没设就跳过（不是失败）。

```powershell
$env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'

# 驱动层：连接/绑定/取值/事务/LOB/错误码（含错误码目录的离线守门）
cargo test -p dm-odbc --test integration -- --include-ignored --test-threads=1

# 查询层：RowSet/RowStream/事务/批量写（含 1000 行批插计时）/LOB 分块读写（Round 13 读、Round 14 写）
cargo test -p dm-query --test query_real -- --include-ignored --test-threads=1

# 内存预算（Round 16）：行流 / 分块读 / 分块写的峰值不随规模放大（`--test-threads=1` 必须）
cargo test -p dm-query --test query_real -- --ignored --test-threads=1 --nocapture memory_does_not_follow

# 迁移：脚本切分（注释/字符串内分号/PL 块）+ 版本记录表 + apply_pending
cargo test -p dm-migrate --test migrate_real -- --include-ignored --test-threads=1

# 连接池：并发借还、超时、回收
cargo test -p dm-pool --test g4_real -- --include-ignored --test-threads=1
```

最近一次**全量真机复跑**（Round 29，`document_collections` / upsert 两个能力位落地之后的当前修订）：`dm-odbc` **74 / 0**、`dm-query` **22 / 0**、`dm-migrate` **4 / 0**、`dm-pool` **5 / 0**、`toasty_real` **1 / 0**、`examples/toasty_crud` **exit 0**（日志 `target/m29_real_*.log`）；其中 `dm-odbc` 那 74 条是 Round 27「时间参数 ODBC 精度声明」改动的真机回归证据。

**性能基准**（自己当 main，不依赖 `criterion`）：

```powershell
# 批量写增速（AC-12：数组绑定 vs 逐行；1000 行 release 档 14.7×–15.5×）
cargo +stable-x86_64-pc-windows-msvc bench -p dm-query --bench batch_insert
# 额外诊断模式（探究「大 PARAMSET_SIZE 是否另有开销」的那批对照，保留作脚手架）
cargo +stable-x86_64-pc-windows-msvc bench -p dm-query --bench batch_insert -- -- diag

# 自研开销（离线、假后端，不含驱动与网络；判据 P50 < 50 µs）
cargo +stable-x86_64-pc-windows-msvc bench -p dm-query --bench overhead

# 真机 select 1 延迟（判据 P99 < 2 ms）+ 裸驱动地板价对照（需 DM_TEST_DSN）
cargo +stable-x86_64-pc-windows-msvc bench -p dameng --bench latency
```

两个带判据的基准默认**只打印判定**；加 `--strict` 才把「判定不过」变成退出码 1（便于入 CI）。

**错误码采集入口**（只打印不断言，用于发现达梦新错误码后决定是否登记）：

```powershell
cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example errors_probe
```

## 文档

- [`docs/达梦中间件使用手册.md`](docs/达梦中间件使用手册.md) —— **使用者手册**（26 章 + 附录，新手与老手分开读）：5 分钟跑通、配置/连接池/查询/事务/批处理/大字段/流式/迁移/错误与重试/读写分离/可观测性/性能调优/测试与门禁/常见坑与 FAQ。文中代码在 [`crates/dameng/examples/manual_snippets.rs`](crates/dameng/examples/manual_snippets.rs) 里编译验证，门禁 `clippy --workspace --all-targets --all-features` 会顺带盯住。
- [`docs/设计方案.md`](docs/设计方案.md) —— 唯一的设计真相来源：需求、实测、架构、冻结 API、里程碑与门禁记录。
- [`docs/ct-mapping.md`](docs/ct-mapping.md) —— AC-02 的 `CT-01…CT-12` ↔ 真实测试用例映射表（含「文档声称有、代码里没有」的六条分歧与缺口清单）。
- [`docs/acceptance-report.md`](docs/acceptance-report.md) —— 验收报告：逐条 AC 的判定、证据与如实登记的缺口（含 AC-01 的逐 crate 覆盖率）。
- [`docs/功能清单与测试结果.md`](docs/功能清单与测试结果.md) —— 中间件支持的功能逐项清单 + 每一项的测试证据与结果（Round 37），含「明确不支持」清单。
- [`docs/测试报告-dbtesta-dbtestb.md`](docs/测试报告-dbtesta-dbtestb.md) —— 针对 `DBTESTA`/`DBTESTB` 两个模式的完整测试报告（模式、限定名、模型表名带前缀的实测记录）。
- [`docs/性能测试报告.md`](docs/性能测试报告.md) —— 性能实测报告：100 字段表 1000 万行、20 字段含随机 BLOB 表 10 万行的插入/读取/随机增删改，含复现命令与 `crates/dm-query/examples/perf_bench.rs` 基准程序。
- [`docs/errors-catalog.json`](docs/errors-catalog.json) —— 已实测登记的达梦错误码目录（由 `dm-error` 的
  `catalog::render_json()` 生成，**禁止手改**，CI 逐字节比对）。

## 许可

MIT OR Apache-2.0。
