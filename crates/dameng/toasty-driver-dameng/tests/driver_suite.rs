//! T8.9：把达梦驱动接进上游**官方一致性套件**
//! （`toasty-driver-integration-suite` 0.11.0）。
//!
//! 套件本身是 crates.io 上的独立 crate，它用 `#[driver_test(requires = …)]` 标注
//! 每条用例需要哪些 `Capability` 位，再由
//! `toasty_driver_integration_suite::generate_driver_tests!` 一次性展开成
//! 一大组测试函数。驱动侧只需实现两个东西（`crates/toasty-driver-integration-suite/src/setup.rs:3-11`）：
//!
//! ```text
//! fn driver(&self) -> Box<dyn toasty_core::driver::Driver>;
//! async fn delete_table(&self, name: &str);
//! ```
//!
//! # 为什么整个文件挂在 `real` feature 后面
//!
//! 套件生成的用例**必须**连真库（它们不做「没配库就跳过」这一步，上游是在 CI 里
//! 起一个真 MySQL 跑的）。所以这里用 `#![cfg(feature = "real")]` 把整个测试
//! crate 关掉：默认 `cargo test -p toasty-driver-dameng` 不会碰真库，
//! 只有显式开 feature 才会编译并运行：
//!
//! ```text
//! $env:DM_TEST_DSN = 'DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA'
//! cargo +stable-x86_64-pc-windows-msvc test -p toasty-driver-dameng --features real \
//!     --test driver_suite -- --test-threads=1 --nocapture
//! ```
//!
//! # 与上游 MySQL 接入的差异
//!
//! - `Setup::driver()` 是**同步**的，而上游 MySQL 驱动是「存 URL、连的时候才连」；
//!   我们的池是异步建的 ⇒ 用 [`DamengDriver::lazy`]（同步构造、首次 `connect` 建池）。
//! - `delete_table` 不用 `DROP TABLE IF EXISTS`（该语法在达梦上**未实测**），
//!   先查 `USER_TABLES` 确认存在再删；被外键引用时退 `CASCADE CONSTRAINTS`。
#![cfg(feature = "real")]

use dm_core::DmConfig;
use dm_query::PooledConn;
use toasty_driver_dameng::DamengDriver;

/// 真机配置 + 清理入口。
struct DamengSetup {
    config: DmConfig,
}

impl DamengSetup {
    /// 从 `DM_TEST_DSN` / `DM_SERVER` 读配置。
    ///
    /// 与 `tests/toasty_real.rs` 同一约定：没配就**报错**（本套件不做静默跳过——
    /// 整个文件已经由 `real` feature 把门）。
    fn new() -> Self {
        let dsn = std::env::var("DM_TEST_DSN")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let config = if let Some(dsn) = dsn {
            DmConfig::from_connection_string(&dsn).expect("DM_TEST_DSN 不是合法的连接串")
        } else if std::env::var("DM_SERVER").is_ok() {
            DmConfig::from_env().expect("DM_SERVER 环境下无法构造 DmConfig")
        } else {
            panic!("T8.9 需要真机：请设置 DM_TEST_DSN（或 DM_SERVER）后再开 --features real")
        };
        Self { config }
    }
}

#[async_trait::async_trait]
impl toasty_driver_integration_suite::Setup for DamengSetup {
    fn driver(&self) -> Box<dyn toasty_core::driver::Driver> {
        // 同步构造：`Setup::driver()` 不在 async 上下文里，池推迟到第一次
        // `Driver::connect` 时再建（`driver.rs` 的 `PoolSource::Lazy`）。
        Box::new(DamengDriver::lazy(self.config.clone()))
    }

    async fn delete_table(&self, name: &str) {
        let pool = DamengDriver::lazy(self.config.clone())
            .ensure_pool()
            .await
            .expect("连不上达梦（delete_table）");
        let mut conn = PooledConn::acquire(&pool).await.expect("借连接失败");

        // ⚠️ **名字大小写**：套件把模型名 `probe_widgets` 原样传进来，而引擎建表时
        // fork 的渲染臂把标识符折成**大写**（`serializer/ident.rs`）⇒ 库里是
        // `PROBE_WIDGETS`。拿小写名字去查 `USER_TABLES` 永远查不到（用例会留下残表，
        // 下一次 `push_schema` 报 `[42S01/-2124] 对象已存在`）。所以这里统一折成大写。
        let name = name.to_uppercase();

        // 达梦没有把握支持 `DROP TABLE IF EXISTS`（未实测）⇒ 先确认存在。
        let tables = conn
            .query("SELECT TABLE_NAME FROM USER_TABLES", ())
            .await
            .expect("查询 USER_TABLES 失败");
        let exists = tables
            .rows()
            .iter()
            .any(|row| row.get::<String>(0).map(|t| t == name).unwrap_or_default());
        if !exists {
            return;
        }

        let quoted = format!("\"{}\"", name.replace('"', "\"\""));
        if let Err(err) = conn.execute(&format!("DROP TABLE {quoted}"), ()).await {
            tracing::debug!(table = %name, %err, "DROP TABLE 失败，改用 CASCADE CONSTRAINTS 重试");
            conn.execute(&format!("DROP TABLE {quoted} CASCADE CONSTRAINTS"), ())
                .await
                .unwrap_or_else(|err| panic!("delete_table({name}) 两次都失败：{err}"));
        }
    }
}

// 能力位：与 `src/capability.rs` 的 `DAMENG` 逐项一致（套件用它们决定哪些用例该跑）。
//
// ⚠️ **没写出来的位一律当 `true`**（`test_variants.rs:118-122` 的
// `capability_checker`：`None => ThreeValuedBool::True`）⇒ `DAMENG` 里每个 `false`
// 的字段都必须在这里显式写 `false`，否则那条用例会照跑，然后在运行期 panic
// 「Driver does not support required capability: …」。下面按 `capability.rs` 的顺序列全：
// 自增/回查（`returning_from_insert`/`returning_from_update`）、upsert 四项、
// JSON 与网络地址族、枚举、`bigdecimal`、`ILIKE`、集合/数组一族、只读连接池测试、
// `transaction_lock_mode`、无参 DECIMAL 之外的 decimal 位（`native_decimal` 为 true，不列）。
toasty_driver_integration_suite::generate_driver_tests!(
    DamengSetup::new(),
    cte_unreferenced: false,
    // 真机实测：无约束 `NUMBER` 经 ODBC 回来是 DOUBLE（`engine_probe.rs` 第 4 节）⇒
    // 任意精度拿不到；默认十进制类型改为显式 `DECIMAL(38, 18)`（见 `src/capability.rs`）。
    decimal_arbitrary_precision: false,
    native_ilike: false,
    native_json: false,
    native_jsonb: false,
    native_cidr: false,
    native_inet: false,
    native_macaddr: false,
    native_macaddr8: false,
    native_enum: false,
    named_enum_types: false,
    // T8.14：`BigDecimal` 由值层按十进制文本编解码（`V::BigDecimal` ⇄ `DmValue::Text`），
    // 解码侧靠 dm-odbc 对高精度列的原样文本回退（`result.rs::decimal_text_value`）⇒ 开位。
    // 这一位一开，新跑的就是 `type_bigdecimal::ty_bigdecimal_as_numeric_fixed_precision`
    // （`numeric(38,20)`，38 位有效数字）；任意精度那条仍由 `decimal_arbitrary_precision: false`
    // 门控。依据：真机探针第 24 节、`docs/toasty-fork.md` 第 24 条。
    bigdecimal_implemented: true,
    // T8.3：`upsert_primary_key` 已开（达梦 `MERGE INTO` 由 fork 渲染，`src/capability.rs:164`）。
    upsert_primary_key: true,
    // Round 28：冲突目标从主键换成唯一键。`MERGE … ON (唯一列)` 的语义由探针第 31 节
    // 在真机上钉住（首次 upsert 走 INSERT、二次命中 MATCHED 且**不改主键**；复合唯一键同理），
    // 同时驱动侧 `conn.rs::insert_and_reload` 改为按**冲突目标列**回查。
    upsert_unique: true,
    // Round 28：`upsert.update(...)` 的分支赋值。引擎在下发 SQL 前已把 create 分支折进
    // `INSERT` 源、update 分支并进 `shared`，fork 只需把 `update.name(incoming())` 渲染成
    // `USING` 源列（`s."s_<ordinal>"`，见 `serializer/expr.rs` 的 `Expr::Incoming` 达梦臂）。
    upsert_branch_assignments: true,
    // Round 18：`or_ignore()`（`UpsertAction::Ignore`）已开——渲染走「命中就什么都不做」
    // 的 MERGE（`ON (...)` 不含 MATCHED 分支 / 条件恒假），驱动再按受影响行数决定
    // 要不要回查（忽略时返回空结果，`or_ignore()` 才拿得到 `None`）。
    // 依据：探针第 16 节（`tests/engine_probe.rs`）。
    upsert_targeted_ignore: true,
    native_array: false,
    // T8.12：`Vec<scalar>` 落成 `CLOB` 里的 JSON 数组文本，集合谓词渲染成 `JSON_CONTAINS` /
    // `JSON_OVERLAPS` / `JSON_LENGTH`（真机探针第 17 节）⇒ 这三位一起开。
    // `predicate_match_any` 仍为 false：它只管 `IN (...)` → `= ANY(数组)` 那条改写。
    native_array_set_predicates: true,
    vec_scalar: true,
    unique_list_index: false,
    // T8.14：`#[document]`（含集合字段）落 `CLOB` 里的 JSON 文本，叶子读取走达梦的
    // `JSON_VALUE`，按叶子类型补转换（整数 `CAST(… AS BIGINT)`、浮点 `CAST(… AS DOUBLE)`、
    // 布尔 `CASE WHEN … = 'true' THEN 1 ELSE 0 END`、UUID `HEXTORAW(REPLACE(…, '-', ''))`、
    // 时间 `CAST(… AS TIMESTAMP(6))`）。`native_json` 仍为 false（达梦没有 JSON 列型）。
    // 依据：真机探针第 25/26/27 节、`docs/toasty-fork.md` 第 25 条。
    document_collections: true,
    bind_list_param: true,
    predicate_match_any: false,
    // T8.3：达梦没有原生 `RETURNING`（§9.4 第 6 条），这里 `true` 的含义是
    // 「驱动会把它模拟出来」——`DamengConnection` 先摘掉 RETURNING 执行写入，再按主键回查
    // （`src/conn.rs` 的 `insert_and_reload`；多行 `VALUES` 逐行写、逐行回查）。这一位一开，
    // 引擎会把**所有**「结果行不能由常量提取」的插入都规划成需要投影的形状，驱动必须全部兜住
    // （`relation_preload` 的批量插入就是这么暴露出来的）。上游两条负向能力断言用例是
    // 「能力值感知」的（断言 `capability.returning_from_insert == expected`，不写死方向），
    // 所以不构成阻碍；真正的代价与遗留见 `docs/设计方案.md` §12.2 第 22 条。
    returning_from_insert: true,
    returning_from_update: false,
    // T8.13：数组原地删除三件套（真机探针 19a–19o 与 22a–22k 验证）——
    // `vec_pop` = `JSON_LENGTH` 夹空列表 + `JSON_REMOVE`、
    // `vec_remove_at` = `JSON_REMOVE(列, '$[' || idx || ']')`、
    // `vec_remove` = 普通 `UPDATE … SET` 由 fork 渲染成
    // `MERGE INTO … USING(<重建派生表>) … WHEN MATCHED THEN UPDATE SET`。
    // 依据见 `docs/toasty-fork.md` 第 22 条、`docs/设计方案.md` §12.2 第 24 条。
    vec_remove: true,
    vec_pop: true,
    vec_remove_at: true,
    transaction_lock_mode: false,
    test_connection_pool: false,
    glob_starts_with: false,
);
