//! `Capability::DAMENG` —— 达梦在 Toasty 里的能力声明（T8.4）
//!
//! 设计依据：`docs/设计方案.md` §9.1.1（T8.0 结案）、§9.3 / §9.3.1（字段清单）、
//! §9.4（方言渲染规则）、§4.4（长度语义）、§8.3（服务端字符集）。
//!
//! **字段逐个显式赋值，不用 `..Capability::MYSQL`**，两个理由：
//!
//! 1. 每个取值都要有据可查——继承会掩盖决策（T8.4 的验收判据就是「字段取值有断言」）；
//! 2. 上游升级时新增字段会**编译报错**，而不是被静默继承一个我们没审过的值。
//!
//! 注释里的 **(实测)** 表示已有真机依据；**(待实测)** 表示必须在 T8.9/T8.10 之前用真机确认，
//! 已集中登记在 `docs/设计方案.md` §14 与 §9.4 第 12/13 条附近的待办里。
//!
//! ## 两个反直觉但关键的决定
//!
//! - **`sql: Some(Dialect::Mysql)`（不是 `Sqlite`）**：两个理由。①`returning_from_update = false`
//!   的驱动要靠上游自带的那条 MySQL 专用回查（`toasty/src/engine/exec/exec_statement.rs:268-297`
//!   的 `process_stmt_update_with_returning_on_mysql`，守卫就是 `sql == Some(Dialect::Mysql)`）；
//!   ②达梦的占位符就是 `?`，与 MySQL 同侧。达梦没有 `RETURNING`（§9.4 第 6 条），
//!   insert 侧我们改为**自己声明 `returning_from_insert = true` 并在驱动里模拟**（见下），
//!   所以规划期不再需要 `verify_insert_returning_without_capability()` 那条窄路。
//!   详见 §9.1.1 与 §12.3（T8.3）。
//! - **`vec_scalar` 走「`CLOB` 里的 JSON 数组文本 + MySQL 那一族 JSON 判定函数」（T8.12）**：
//!   达梦无原生数组（真机探针 17l 语法错误）、无 `MEMBER OF`/`SUBMULTISET`（17m/17n），
//!   但 `JSON_CONTAINS`（17s，且**能吃绑定参数**：17ak/17am）、`JSON_OVERLAPS`（17t/17al）、
//!   `JSON_LENGTH`（17r）都真机可用，文本比较又是大小写敏感的（17h/17i/17an）⇒
//!   集合谓词由 fork 渲染成这三个函数（`crates/toasty-sql/src/serializer/expr.rs`），
//!   列表值的编解码在驱动里（`src/list_json.rs`）。
//!   `document_collections` 仍为 `false`：`#[document]` 集合没做。

use toasty_core::driver::{Capability, Dialect, SchemaMutations, SqlPlaceholder, StorageTypes};
use toasty_core::schema::db;

/// 达梦的 `StorageTypes`。
///
/// `StorageTypes` 是上游类型，加不了固有 `const`，所以另起一个常量名。
/// 取值与上游 `StorageTypes::POSTGRESQL` 同族（`Numeric(None)` + `Timestamp(6)`/`Time(6)`/
/// `DateTime(6)`，见 `toasty-core-0.11.0/src/driver/capability.rs:1081-1091`）：
/// 两者都是「有原生 DECIMAL/时间类型、但没有原生 UUID」的库。
pub const DAMENG_STORAGE_TYPES: StorageTypes = StorageTypes {
    // 索引安全优先：达梦的索引键长上限约为半个数据页，而 `CLOB` **根本不能**作主键或建索引。
    // 上游 MySQL 出于同样的理由（utf8mb4 索引前缀）选了 `VarChar(191)`；达梦 `VARCHAR(n)`
    // 按**字节**计量（§4.4，LENGTH_IN_CHAR=0），服务端字符集实测为 GB18030（§8.3），
    // 255 字节 ≈ 127 个汉字。需要更长的字段请用 `#[field(size = n)]` 显式声明。
    //
    // Round 24 实测（`tests/engine_probe.rs` 第 23 节，日志 `target/m24_probe23c.log`；对应 §14 V-21）：
    // ① `VARCHAR(255)` 作**主键**建表成功（23g）；
    // ② 中文值走键：84 个汉字 = 252 字节插入成功、按该主键等值查回命中（23g2/23g3）；
    // ③ 128 个汉字 = 384 字节被拒 `[HY000/-6169] 列[s]长度超出定义`（23g4）⇒ 确认**按字节**计量。
    // ⇒ 255 落在索引键长上限内，`String` 字段可以直接作主键。
    default_string_type: db::Type::VarChar(255),

    // 达梦 `VARCHAR` 声明长度的上限（按字节计量）。必须与 `native_varchar: true` 同真，
    // 否则 `Capability::validate()` 报 `invalid_driver_configuration`。
    //
    // 先前填的 8188 是 DM8 默认 8 KiB 页下的**行内**上限，不是语法上限。Round 24 实测（探针第 23 节；
    // 日志 `target/m24_probe23c.log`；对应 §14 V-22）：
    // ① `VARCHAR(8189)` 建表成功（23j）、`VARCHAR(32767)` 也成功（23j2）；
    // ② `VARCHAR(32768)` 被拒 `[HY000/-6121] 数据精度超出范围`（23j3）⇒ **真上限 = 32767 字节**；
    // ③ 往 `VARCHAR(32767)` 里插入 9000 字节成功（23j4），`LENGTH()` 回读 9000（23j5）⇒ 不是「只准声明」；
    // ⇒ 这里取 `Some(32767)`，让 `#[field(size = n)]` 能声明到真上限。
    // ⚠️ 两个不能忘的代价：① 大 `VARCHAR` 仍受**索引键长**约束（约半个数据页，超了不能建索引，
    // 见 `docs/toasty-fork.md` 第 10 条；默认的 `VarChar(255)` 正是为此选的）；② 一行的多个大列会
    // 撞上单行长度上限。
    varchar: Some(32767),

    // 达梦没有 UUID 类型（§9.4 相关条目）：按**规范的 36 字符文本**（`xxxxxxxx-xxxx-…`）存。
    //
    // 依据（两条，都是实测/上游源码）：
    // 1. 真机实测（T8.9，官方一致性套件 `type_primitives::ty_uuid`）用 `Blob` 时会失败——
    //    上游把 UUID 当主键，达梦回 `[HY000/-3207] 试图在BLOB/CLOB/用户自定义数据类型列[id]上建索引`；
    //    而套件自己的期望表只认 `Uuid | Blob | Text | VarChar(..)` 四种（`type_primitives.rs:462-467`），
    //    定长 `Binary(16)` 会让套件 `todo!` 直接 panic。
    // 2. 上游 `StorageTypes::MYSQL`（`toasty-core-0.11.0/src/driver/capability.rs:1118`）在
    //    「没有原生 UUID 类型」时选的正是 `VarChar(36)`；SQLite 选 `Blob`、PG 选 `Uuid`、DynamoDB 选 `Text`。
    //    `VARCHAR(36)` 在达梦可建索引（不像 BLOB/CLOB），代价是每个键 36 字节而非 16 字节。
    default_uuid_type: db::Type::VarChar(36),

    // `BLOB`。
    default_bytes_type: db::Type::Blob,

    // 达梦有原生 `DECIMAL`，但**必须显式给精度/标度**才能保精度往返。
    //
    // 真机实测（T8.9，`crates/toasty-driver-dameng/tests/engine_probe.rs` 第 4 节）：
    // - 无约束的 `NUMBER` 经 DM ODBC 回来是 **DOUBLE**（`1.5` → `Double(1.5)`，
    //   `@@IDENTITY` 同）⇒ 有效数字超过 ~15 位的值会**静默丢精度**
    //   （官方套件 `type_decimal::ty_decimal` 的 `99999999999999999999.99999999`
    //   回来变成 `100000000000000000000`）；
    // - 而 `DECIMAL(38, 10)` 回来是 **`Decimal`**（精确），所以带精度的 DECIMAL 是可用路径。
    //
    // ⇒ 默认用 `Numeric(Some((38, 18)))`（达梦 DECIMAL 的精度上限 38；标度取 18 以覆盖
    //   官方套件里 `3.141592653589792238` 这类 18 位小数的往返）。超过标度的值由达梦报错
    //   （`-6111` 转换失败），**不会**静默四舍五入。
    default_decimal_type: db::Type::Numeric(Some((38, 18))),

    // `BigDecimal` 按**文本**存。
    //
    // 真机依据（T8.9 官方套件 `type_bigdecimal`）：
    // - `bigdecimal_implemented = false` 只影响**声明了** `requires(bigdecimal_implemented)`
    //   的用例（它们会被跳过），**不会**让引擎在规划期拒绝 `BigDecimal` 字段——
    //   `type_bigdecimal::ty_bigdecimal` / `ty_bigdecimal_as_text` 两条用例**没有**
    //   `requires`，必然要跑；
    // - 走 `Numeric((38, 18))` 时值会以 `Value::BigDecimal` 直达我们的编解码层（T8.14 起
    //   这条路已实现）；任意精度那条套件用例（40 位有效数字）在达梦 `DECIMAL(p,s)`
    //   （p ≤ 38）上仍放不下，由 `decimal_arbitrary_precision = false` 继续门控；
    // - 存 `Text` 时达梦侧落 `VARCHAR(8188)`（见 fork `serializer/ty.rs` 的 `db::Type::Text`
    //   臂），文本往返**逐字符精确**——正是 `BigDecimal` 要的语义。
    //
    // ⇒ 与 `default_zoned_type`/网络地址族同样的「无原生类型 ⇒ 规范文本」策略；
    //   值层的 `Value::BigDecimal` 自 T8.14 起已实现（`bigdecimal_implemented = true`）。
    default_bigdecimal_type: db::Type::Text,

    // 达梦原生时间类型，微秒精度上限 6 位。
    default_timestamp_type: db::Type::Timestamp(6),

    // 带时区的「瞬时」不能落进 `db::Type` 的无时区 `Timestamp`（会丢时区）——
    // 上游 PG/MySQL 都选择按 ISO-8601 文本存，达梦同样处理。
    default_zoned_type: db::Type::Text,

    default_date_type: db::Type::Date,
    default_time_type: db::Type::Time(6),
    default_datetime_type: db::Type::DateTime(6),

    // 网络地址族达梦没有原生类型，按规范文本存（与 PG 之外的三个后端一致）。
    default_cidr_type: db::Type::Text,
    default_inet_type: db::Type::Text,
    default_macaddr_type: db::Type::Text,
    default_macaddr8_type: db::Type::Text,

    // 达梦没有无符号整数，`BIGINT` 是有符号 64 位 ⇒ 上界 i64::MAX，与 SQLite/PG 一致。
    max_unsigned_integer: Some(i64::MAX as u64),
};

/// 达梦的 `Capability`。
///
/// 用法：`fn capability(&self) -> &'static Capability { &capability::DAMENG }`。
pub const DAMENG: Capability = Capability {
    driver_name: "dameng",

    // ⚠️ 见文件头：故意复用 MySQL 的**方言身份**（不是「说 MySQL 的 SQL」，
    // 而是「让规划器走那条唯一按方言身份分支的路径」）。
    sql: Some(Dialect::Mysql),
    sql_placeholder: Some(SqlPlaceholder::QuestionMark),
    storage_types: DAMENG_STORAGE_TYPES,

    // 达梦支持 `ALTER TABLE … MODIFY <col> <type>`。Round 24 实测（探针第 23 节；对应 §14 V-28）：
    // ① `MODIFY "s" VARCHAR(50)` 只改类型成功（23p）；
    // ② 一条里同时改类型 + `DEFAULT` + `NULL` 也成功（23q/23r），复查 `USER_TAB_COLUMNS`
    //    三样都生效（23s：`VARCHAR` / 70 / `Y` / 默认值 `'y'`）⇒ 达梦**能**一条改多属性。
    // `alter_column_type: true` 据此确认（因此不会走 SQLite 式整表重建）。
    // `alter_column_properties_atomic` 仍取 `false`：合并形态要 fork 的 `AlterColumn` 渲染臂支持
    // （当前只有逐属性 `changes.split()` 这条路有真机/迁移覆盖），翻转的收益只是少打几条语句、
    // 风险落在迁移路径上 ⇒ 先保守，等有迁移用例覆盖再翻（证据已备：23q/23r）。
    schema_mutations: SchemaMutations {
        alter_column_type: true,
        alter_column_properties_atomic: false,
    },

    // 达梦接受顶层 `WITH … UPDATE`（Round 24 探针 23t：影响 0 行；23u 的 `WITH … SELECT` 同样
    // OK），但不支持数据修改型 CTE（23v 的 `WITH … MERGE`、23w 的 `WITH x AS (UPDATE …
    // RETURNING …) SELECT …` 都报语法错误——后者还撞上「达梦没有 `RETURNING`」，§9.4 第 6 条）。
    // 引擎要的是把 `UPDATE` 放进 CTE 的那种形态（PG 式），达梦给不了 ⇒ `false` 是正解；
    // 顶层 `WITH … UPDATE` 能用也不影响正确性，只是少一种优选形态。
    cte_with_update: false,

    // §4.6 实测：`SELECT … FOR UPDATE` 可用。
    select_for_update: true,

    // 达梦没有 `INSERT/UPDATE … RETURNING`（§9.4 第 6 条）。
    //
    // `returning_from_insert` **声明 true 的是「驱动会替上游把 RETURNING 模拟出来」**，
    // 不是「达梦有 RETURNING」：`DamengConnection::exec(Operation::Insert)` 会把
    // `RETURNING` 从语句上摘下来执行写入，再按主键 `SELECT` 回查（T8.3，见 §12.2 第 22 条；
    // 多行 `VALUES` 会**逐行写、逐行回查**，因为自增主键只能按行 `@@IDENTITY`，且无 `ORDER BY`
    // 的 `OR` 链行序不可依赖）。
    // 之所以必须声明 true：上游只在 `returning_from_insert == false` 且形状**恰好**是
    // 「MySQL 的 `LAST_INSERT_ID` 单自增主键」时才接受没有 RETURNING 的计划
    // （`toasty-0.11.0/src/engine/plan/statement.rs:1030-1104` 的
    // `verify_insert_returning_without_capability`），upsert 拿不到那条捷径。
    returning_from_insert: true,
    // UPDATE 仍走上游自带的 MySQL 形态回查（`exec_statement.rs:268-297`），无需在此声明。
    returning_from_update: false,

    // upsert：达梦用 `MERGE INTO … WHEN MATCHED / WHEN NOT MATCHED`（§4.6 实测）。
    //
    // T8.3：`Flavor::Dameng` 的 MERGE 渲染已落地（`statement.rs` 的 `dameng_merge_into`），
    // 覆盖「按<冲突目标列> upsert + 共享赋值」这一形状；自增主键带显式值时驱动会用
    // `SET IDENTITY_INSERT "<表>" ON/OFF` 包一层（真机探针 15k–15p 实测）。
    //
    // `upsert_targeted_ignore`（`ON CONFLICT (<列>) DO NOTHING`）**已置真**：达梦没有
    // `DO NOTHING`，但 `WHEN MATCHED` 分支必须存在，渲染成「把第一列赋成它自己」的
    // 空更新即可（`crates/toasty-sql/src/serializer/statement.rs` 的
    // `UpsertAction::Ignore` 臂，真机探针 15f 验证过可达），语义与 `DO NOTHING` 等价
    // ——命中行保持原值、未命中行插入。命中空更新时驱动回查到的就是那一行的原值
    // （引擎要的正是它）。
    //
    // Round 28 把最后两项一起置真：
    //   - `upsert_unique`：`ON (...)` 用的就是 `upsert.target` 的列，唯一键目标与主键目标
    //     在渲染上同形；真正的缺口在**驱动**——命中冲突时保留的是已有行的主键，而写入语句
    //     里带的是本次源行的主键（`upsert_by_email` 那种新生成的 UUID）⇒ `insert_and_reload`
    //     原来按主键回查会 0 行。现在 upsert 一律按 `upsert.target` 的列回查（主键目标与旧
    //     行为等价，真机探针 31 实测唯一键/复合唯一键两种形状）。
    //   - `upsert_branch_assignments`：引擎对 SQL 驱动会把 create 分支折进插入源、把 update
    //     分支覆盖到 `shared` 再下发，所以 `dameng_merge_into` 照旧只渲染「源值 + shared」；
    //     唯一要补的是 `upsert.update(...)` 里的**新值引用**（`stmt::Expr::Incoming`）——
    //     达梦没有 `excluded`，渲染成 `USING` 源列 `s."s_<ordinal>"`（token 由插入列列表的
    //     位置决定，探针 31 实测）。
    upsert_primary_key: true,
    upsert_unique: true,
    upsert_branch_assignments: true,
    upsert_targeted_ignore: true,

    // 达梦支持主键列上的 `!=`（只有 DynamoDB 不支持）。
    primary_key_ne_predicate: true,

    // `IDENTITY(1,1)`（§4.6 实测；`GENERATED … AS IDENTITY` 报 -2007）。
    auto_increment: true,
    // 达梦的 `IDENTITY` 列只能是整数类型；8 字节对齐 i64 主键（i32 主键会选 Integer(4)）。
    //
    // Round 24 实测（探针第 23 节；对应 §14 V-24）：`TINYINT`/`SMALLINT` + `IDENTITY(1,1)` 被拒
    // `[HY000/-2713] 非法IDENTITY列类型`（23k/23l），`INT`/`BIGINT` 通过（23m/23n：插一行
    // `DEFAULT VALUES` 后回读 1）⇒ 达梦要求自增列宽度 **≥ 4 字节**。
    // 本字段只能表达**上限**（上游 0.11.0 字段文档：cap the storage type selected from the Rust
    // field type）⇒ `Some(8)` 等于「不因自增而缩窄 Rust 侧宽度」；代价是窄整数自增键
    // （`i8`/`i16`）会生成达梦拒绝的 DDL，而本字段没有表达下限的位置 —— 如实登记为限制
    // （官方一致性套件与示例都用 8 字节主键，未覆盖该形状）。
    max_auto_increment_integer_width: Some(8),

    // 达梦标识符长度上限（默认 128 字节）。给 `Some(n)` 后超长自动索引名会被截断 + 哈希后缀。
    //
    // Round 24 实测（探针第 23 节；对应 §14 V-23）：128 字节列名建表成功（23e）；129 字节列名
    // 被拒 `[HY000/-2051] 第 1 行, 第 178 列["]附近出现错误: 标示符长度非法`（23f）
    // ⇒ `Some(128)` 与真机一致。未做 `SQLGetInfo(SQL_MAX_IDENTIFIER_LEN)` 交叉核对：该值随
    // 建库参数可能变化，换库后若出现建表失败，先量这一项。
    max_identifier_length: Some(128),

    // 达梦有原生 `VARCHAR(n)`。必须与 `storage_types.varchar.is_some()` 一致（`validate()` 会查）。
    native_varchar: true,

    // 达梦没有 `JSON`/`JSONB` 列类型（有 `JSON` 函数族但没有原生列类型）。
    native_json: false,
    native_jsonb: false,

    // 达梦有原生 `TIMESTAMP(p)` / `DATE` / `TIME(p)` / `DATETIME(p)`（§9.4 相关条目）。
    native_timestamp: true,
    native_date: true,
    native_time: true,
    native_datetime: true,

    // 网络地址族无原生类型。
    native_cidr: false,
    native_inet: false,
    native_macaddr: false,
    native_macaddr8: false,

    // 达梦没有 `ENUM` 类型：与 SQLite 同路，落成 `VARCHAR(n)` + `CHECK` 约束（下游 `column_def` 分支）。
    // named enum 需要 `CREATE TYPE`，达梦没有 ⇒ false。
    native_enum: false,
    named_enum_types: false,

    // 达梦有原生 `DECIMAL`。
    native_decimal: true,
    // T8.14：`BigDecimal` 的编解码**已实现**（值层按十进制文本走，见 §7 与 `value.rs`）⇒ true。
    //
    // 实现与依据：
    // 1. **编码**：`V::BigDecimal` → `DmValue::Text(v.to_string())`。值不经过 `f64`、也不截到
    //    `rust_decimal` 的 96 位；文本参数由达梦的 ODBC 驱动转进数值列（真机探针第 24 节：
    //    38 位整数部分的 `123456789012345678.12345678901234567890` 与 `3.14159265358979323846`
    //    都插得进 `DECIMAL(38,20)`）。
    // 2. **解码**：dm-odbc 的 `read_decimal` 对 `precision > 28` 的列本来就走文本路径
    //    （`SQL_C_NUMERIC` 会溢出），现在把「文本装不进 `rust_decimal`」**如实交回文本**
    //    而不是报 `Decode`（`result.rs::decimal_text_value`）⇒ `T::BigDecimal` 分支再从文本
    //    还原成任意精度十进制（38 位一位不丢）。
    // 3. 声明 true 后**新增**的套件用例只有 `type_bigdecimal::ty_bigdecimal_as_numeric_fixed_precision`
    //    （`requires(and(native_decimal, bigdecimal_implemented))`）；任意精度那条仍由
    //    `decimal_arbitrary_precision = false` 门控（达梦 `DECIMAL(p,s)` 的 p ≤ 38，
    //    放不下套件给的 40 位有效数字）。
    bigdecimal_implemented: true,
    // ⚠️ 这个标志必须与 `default_decimal_type` **配对**（上游 `validate()` 不查这一对，
    // 所以只能靠人盯）：`false` 的语义是「DECIMAL 必须显式给 (p,s)」，与
    // `default_decimal_type = Numeric(Some((38, 18)))` 一致。
    //
    // 真机实测（T8.9 官方套件 + `engine_probe.rs` 第 4 节）：达梦无约束的 `NUMBER` 经 ODBC
    // 回来是 DOUBLE ⇒ 「任意精度」在 ODBC 这条路上**拿不到**，所以诚实地声明 `false`
    // （引擎会在需要任意精度时按 unsupported 处理，而不是给一个悄悄丢精度的结果）。
    decimal_arbitrary_precision: false,

    // 所有 SQL 后端都是 true（索引键条件只有 DynamoDB 才用）。
    index_or_predicate: true,

    // 达梦没有 `^@`（PG 前缀算子）与 `GLOB`，但**有 `LIKE`**，所以前缀匹配走
    // 「引擎把字面前缀转成 `LIKE` 模式 + 转义通配符」这条路（`crates/toasty/src/engine/bind/extract.rs:80-99`
    // 只在 `glob_starts_with || binary_like_starts_with` 为真时才做这次重写），
    // 再由 fork 的渲染臂把 `StartsWith` 渲染成 `<expr> LIKE <pattern> ESCAPE '!'`
    // （§9.8）。⇒ 必须让 `binary_like_starts_with` 为真。
    // `native_starts_with` 本身引擎不读，但 `validate()` 要求
    // `binary_like_starts_with ⇒ native_starts_with`，故一并置真（它是「语义上支持前缀匹配」的声明）。
    // ⚠️ 绝不能让 MySQL 的 `BINARY col LIKE ?` 写法继承进来：达梦不认 `BINARY` 前缀
    // （我们渲染的是普通 `LIKE`）。
    native_starts_with: true,
    glob_starts_with: false,
    binary_like_starts_with: true,
    native_like: true,

    // 达梦没有 `ILIKE`：`.ilike()` 会被 query-verify 以 `unsupported_feature` 拒绝
    // （上游明确不模拟 ILIKE —— 静默退化成大小写敏感的 `LIKE` 比报错更糟）。
    native_ilike: false,

    // SQL 驱动集合：无索引全表读走 `QuerySql`，排序在 SQL 内完成 ⇒ 两者都 true。
    scan: true,
    scan_supports_sort: true,

    // 池由我们自己的 `dm-pool` 管（§9.6），不需要 Toasty 的池测试套件。
    test_connection_pool: false,

    // 达梦没有 `BEGIN IMMEDIATE/EXCLUSIVE`（§6.6）⇒ 非 `Default` 事务模式返回 `unsupported_feature`。
    transaction_lock_mode: false,

    // SQL 后端按游标反向翻页是平凡的（同一 `ORDER BY` 反向 + 严格不等），达梦同理。
    backward_pagination: true,

    // ⚠️ 游标分页的正确性依赖这个值（上游 0.11.0 字段文档：Cursor pagination uses this to
    // generate predicates that match the backend's native ORDER BY behavior；DESC 取相反落点）。
    //
    // Round 24 实测（探针第 23 节；对应 §14 V-26）：
    // ① 达梦**原生**升序把 NULL 排在最前（23a：`select 1 … union all select null … order by 1`
    //    首行是 `Null`），与 PG 相反；
    // ② 显式 `NULLS LAST`（ASC）与 `NULLS FIRST`（DESC）都被接受并生效（23c/23d）。
    // ⇒ 声明取 `false`（= 对引擎承诺「按 `false` 生成的游标谓词是对的」），而把达梦扳成这个语义的
    // 是 fork：`crates/toasty-sql/src/serializer/statement.rs` 的 `OrderByExpr` 达梦分支对每条排序
    // 列显式补 `NULLS LAST`（ASC）/ `NULLS FIRST`（DESC）（`docs/toasty-fork.md` 第 17 条，探针
    // 第 14 节）。⚠️ 这两处必须**成对**改：只翻声明不改渲染（或反之）会让游标分页漏行。
    sql_nulls_first_on_asc: false,

    // SQL 后端对键类型没有 DynamoDB 那种限制。
    bool_key_type: true,

    // `Vec<scalar>`（T8.12）在达梦上落成 `CLOB` 里的一段规范化 JSON 数组文本 ⇒ 列表参数
    // 必须**整体**绑成一个值（`Value::List` → JSON 文本，见 `src/value.rs`），否则引擎会把
    // 它摊成 `Expr::List` 的逐元素行值，`JSON_CONTAINS` 与 `||` 拼接都会拿到错误形状。
    // 与上游 SQLite 同侧（`toasty-core-0.11.0/src/driver/capability.rs` 的 `SQLITE`）。
    bind_list_param: true,

    // `expr <op> ANY(<array>)`：**保持 false**。这一位只管「把 `IN (...)` 改写成
    // `= ANY(数组)`」那条优化（`toasty/src/engine/lower.rs:1969-1975` 的
    // `supports_any_rewrite()`）；达梦没有数组类型 ⇒ 不做这个改写，`IN (...)` 原样渲染。
    // `Path::contains` 产生的 `AnyOp` 是另一条路，由 fork 渲染成 `JSON_CONTAINS`。
    predicate_match_any: false,

    // 无原生数组列（探针 17l：`VARCHAR(50) ARRAY` 语法错误）——`vec_scalar` 走 JSON 文本，
    // 所以这两位**没有**绑定关系（上游 SQLite 同样 `native_array: false` + `vec_scalar: true`）。
    native_array: false,
    // T8.12：`Vec<scalar>` 已落地（`CLOB` 里的 JSON 数组文本 + `JSON_CONTAINS`/`JSON_OVERLAPS`/
    // `JSON_LENGTH` + `src/list_json.rs` 的编解码），真机套件见 `docs/acceptance-report.md`。
    vec_scalar: true,
    unique_list_index: false,
    // T8.14：`#[document]`（单文档列）与 `#[document]` 集合字段都已落地：文档存
    // `CLOB` 里的 JSON 文本（值层走 `toasty_sql::json` 的规范编解码），叶子读取由 fork
    // 渲染成达梦的 `JSON_VALUE(列, '$.a.b')` 并按叶子类型补转换（整数 `CAST(… AS BIGINT)`、
    // 浮点 `CAST(… AS DOUBLE)`、布尔 `CASE WHEN … = 'true' THEN 1 ELSE 0 END`、UUID
    // `HEXTORAW(REPLACE(…, '-', ''))`、时间 `CAST(… AS TIMESTAMP(6))`）——形状由真机探针
    // 第 25/26/27 节钉住（`tests/engine_probe.rs`），golden 见
    // `crates/toasty-sql/tests/serialize_dameng.rs`。`native_json` 仍为 `false`：达梦没有
    // JSON 列型，文档落 `CLOB` 文本——这个组合上游 DynamoDB 也是（`document_collections:
    // true` + `native_json: false`）。
    document_collections: true,

    // 集合谓词（`IsSuperset`/`Intersects`）在达梦上渲染成 `JSON_CONTAINS`/`JSON_OVERLAPS`，
    // 两者都接受任意 rhs 表达式（含绑定参数，探针 17ak/17al）⇒ 与上游 SQLite 一样置 true。
    native_array_set_predicates: true,

    // T8.13：数组原地删除。达梦没有数组，但 `CLOB` 里的 JSON 数组文本能原地改：
    // `pop` = `CASE`(空列表直接回原列) + `JSON_REMOVE(列, '$[' || (JSON_LENGTH(列) - 1) || ']')`、
    // `remove_at` = `JSON_REMOVE(列, '$[' || idx || ']')`（越界路径是 no-op，探针 18d）——
    // 这两条**已真机验证**（探针 19a–19o）。依据：真机探针第 18/19 节（`tests/engine_probe.rs`）。
    vec_pop: true,
    vec_remove_at: true,
    // `remove(v)`（删掉**每一个**匹配元素）：普通 `UPDATE … SET` 由 fork 渲染成
    // `MERGE INTO t USING (<重建派生表>) s ON (主键) WHEN MATCHED THEN UPDATE SET t.列 = s.重建`
    // —— 重建若写在 `SET` 的相关子查询里会被达梦判 `[HY000/-4083] 引用列未找到`
    // （探针 19q/19s/19y/19z2/19z4、22f），而 `MERGE … USING` 里的派生表不受这个限制
    // （探针 22g/22h：普通与绑定参数都写出正确的 `["a","b","c"]`；边界 22i/22j）。
    // **没覆盖的形状**（upsert 的 `WHEN MATCHED` 分支、一次 `UPDATE` 里混写多种赋值）
    // 由 `serialize_remove` 的达梦臂如实 `unimplemented!`——宁可炸也不静默写空。
    // 依据见 `docs/设计方案.md` §12.2 第 24 条、`docs/toasty-fork.md` 第 22 条。
    vec_remove: true,
};

/// 直接喂给 `Driver::capability()` 的 `&'static` 视图。
pub const fn capability() -> &'static Capability {
    &DAMENG
}

#[cfg(test)]
// 这些断言的对象是 `const DAMENG` 的字段 ⇒ 值在编译期就可知，`clippy` 的
// `assertions_on_constants` 在这里是误报：常量**正是**我们要逐字段钉死的东西。
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    /// `db::Schema` 构建期会调用 `validate()`：不自洽的能力表必须在**测试**里就炸掉。
    #[test]
    fn dameng_capability_is_self_consistent() {
        DAMENG
            .validate()
            .expect("Capability::DAMENG 必须通过 validate()");
    }

    #[test]
    fn dameng_identity_reuses_mysql_for_insert_id_path() {
        // 见文件头：这一条是「fork 面只压到 toasty-sql」的前提，也是自增主键回查的前提。
        assert_eq!(DAMENG.sql, Some(Dialect::Mysql));
        assert_eq!(DAMENG.sql_placeholder, Some(SqlPlaceholder::QuestionMark));
        // 达梦**没有** `RETURNING`：这里声明 true 的含义是「驱动会模拟」
        // （`DamengConnection` 把 RETURNING 摘下来执行写入、再按主键回查，T8.3）。
        assert!(
            DAMENG.returning_from_insert,
            "声明 true 表示驱动模拟 RETURNING；upsert 只能靠这条路径拿到回写行"
        );
        assert!(DAMENG.auto_increment, "达梦有 IDENTITY");
    }

    /// `native_varchar` 与 `storage_types.varchar` 必须同真同假（`validate()` 的一部分，
    /// 单独断言是为了在失败时给出比 `invalid_driver_configuration` 更直白的信号）。
    #[test]
    fn varchar_support_is_declared_consistently() {
        assert_eq!(
            DAMENG.native_varchar,
            DAMENG.storage_types.varchar.is_some()
        );
        // 索引安全：默认字符串类型必须是 VARCHAR 而不是 CLOB/TEXT（CLOB 不能建索引/主键）。
        assert!(
            matches!(
                DAMENG.storage_types.default_string_type,
                db::Type::VarChar(_)
            ),
            "default_string_type 必须是 VarChar(n)，否则 String 主键/索引列会落成 CLOB"
        );
        // Round 24 实测（探针第 23 节；§14 V-22）：声明上限 32767 字节（`32768` 被拒
        // `[HY000/-6121] 数据精度超出范围`），不是默认页下 8188 的行内上限 ⇒ 钉住这个值，
        // 换库 / 换页大小 / 换 `LENGTH_IN_CHAR` 之后必须重新量。
        assert_eq!(
            DAMENG.storage_types.varchar,
            Some(32_767),
            "V-22：真机上限 32767 字节（23j2 建表成功、23j3 报 -6121）"
        );
    }

    /// 集合一族逐项对账：**没实现也没渲染**的必须显式 `false`（供应商字段「诚实拒绝」），
    /// **已落地**的（T8.12 的 `vec_scalar` 三件套）必须 `true`——两边都钉死，不吃默认值。
    #[test]
    fn collection_capabilities_match_what_is_rendered() {
        assert!(
            !DAMENG.native_array,
            "达梦无原生数组列（探针 17l 语法错误）"
        );
        assert!(
            DAMENG.vec_scalar && DAMENG.bind_list_param && DAMENG.native_array_set_predicates,
            "T8.12：`CLOB` JSON 文本 + JSON_CONTAINS/JSON_OVERLAPS/JSON_LENGTH + list_json 编解码"
        );
        assert!(
            !DAMENG.predicate_match_any,
            "`IN (...)` 不做 `= ANY(数组)` 改写"
        );
        assert!(
            DAMENG.document_collections,
            "T8.14：`#[document]` 落 `CLOB` JSON 文本 + `JSON_VALUE` 叶子提取（探针第 25/26/27 节真机定界）"
        );
        assert!(!DAMENG.unique_list_index);
        assert!(
            DAMENG.vec_pop && DAMENG.vec_remove_at,
            "T8.13：`JSON_REMOVE` + `JSON_LENGTH` 夹空列表（探针 19a–19o 真机验证）"
        );
        assert!(
            DAMENG.vec_remove,
            "T8.13：`remove(v)` 走 `MERGE … USING(派生表重建)`（探针 22g/22h 真机验证）；\
             未覆盖的形状由 `serialize_remove` 如实 unimplemented"
        );
        assert!(!DAMENG.native_json && !DAMENG.native_jsonb);
    }

    /// 前缀匹配：达梦没有 `^@`/`GLOB`，但能用 `LIKE`。
    /// 引擎只在 `binary_like_starts_with` 为真时才把字面前缀转成 `LIKE` 模式
    /// （`crates/toasty/src/engine/bind/extract.rs:80-99`），fork 的渲染臂再把
    /// `StartsWith` 渲染成 `<expr> LIKE <pattern> ESCAPE '!'` ⇒ 这一项**必须**为真。
    /// `validate()` 要求 `binary_like_starts_with ⇒ native_starts_with`，所以两者同真。
    /// `glob_starts_with` 保持 false（我们没有 GLOB，也不该让引擎把 glob 语义放进来）。
    #[test]
    fn starts_with_goes_through_like() {
        assert!(DAMENG.native_starts_with);
        assert!(DAMENG.binary_like_starts_with);
        assert!(!DAMENG.glob_starts_with);
        assert!(DAMENG.native_like, "降级路径必须有 LIKE 可用");
        assert!(!DAMENG.native_ilike);
    }

    /// upsert 的四项能力位必须与「已落地的渲染形状」逐项对齐（这是本文件唯一的自证）：
    /// 开了位却渲染不出来 ⇒ 引擎把计划送下来，达梦收到语法错误；没开位而渲染其实能做
    /// ⇒ 官方套件里那些用例被规划期无辜拒掉（Round 18 就是这么发现
    /// `upsert_targeted_ignore` 的渲染早就落地的）。
    #[test]
    fn upsert_capabilities_match_what_is_rendered() {
        assert!(
            DAMENG.upsert_primary_key,
            "T8.3：MERGE INTO 渲染随本项一起落地"
        );
        assert!(
            DAMENG.upsert_targeted_ignore,
            "Round 18：`DO NOTHING` 渲染成「把第一列赋成它自己」的空更新（探针 15f）"
        );
        assert!(
            DAMENG.upsert_unique,
            "Round 28：`ON (...)` 用的就是 `upsert.target` 的列，唯一键目标与主键目标同形；\
             驱动侧改成按冲突目标列回查（探针 31 实测唯一键与复合唯一键）"
        );
        assert!(
            DAMENG.upsert_branch_assignments,
            "Round 28：引擎对 SQL 驱动已把 create/update 分支折进源值与 `shared`，\
             新值引用渲染成 MERGE 的 `USING` 源列 `s.\"s_<ordinal>\"`（探针 31 实测）"
        );
    }

    /// decimal 三项必须自洽（上游 `validate()` **不查**这一对，只有这条单测能盯住）：
    /// 「DECIMAL 必须显式给 (p,s)」（`decimal_arbitrary_precision = false`）⇔
    /// 默认十进制类型带显式精度/标度。真机依据见 `DAMENG_STORAGE_TYPES` 的注释
    /// （无约束 `NUMBER` 经 ODBC 只能拿到 DOUBLE）。
    #[test]
    fn decimal_precision_flags_are_paired() {
        assert!(
            !DAMENG.decimal_arbitrary_precision,
            "无约束 NUMBER 经 ODBC 回来是 DOUBLE ⇒ 不能声明任意精度"
        );
        assert!(
            matches!(
                DAMENG.storage_types.default_decimal_type,
                db::Type::Numeric(Some(_))
            ),
            "decimal_arbitrary_precision = false 时默认类型必须显式带 (p, s)"
        );
        assert!(DAMENG.native_decimal, "达梦有原生 DECIMAL");
        assert!(
            DAMENG.bigdecimal_implemented,
            "T8.14：`V::BigDecimal` ⇄ 十进制文本（`DECIMAL(38,20)` 的 38 位不丢）——\
             解码侧靠 dm-odbc 的文本回退（`result.rs::decimal_text_value`，探针第 24 节）"
        );
    }
}
