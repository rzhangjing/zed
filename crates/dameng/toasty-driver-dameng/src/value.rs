//! 值编解码：`stmt::Value` ↔ `dm_core::DmValue`（设计方案 §9.4.1，T8.6）。
//!
//! # 三条纪律
//!
//! 1. **不做有损转换**：编码/解码失败一律报错（`unsupported_feature` 或
//!    `serialization_failure`），**绝不**静默丢精度、塞默认值或截断。
//! 2. **达梦没有的语义不进 SQL**：DM 无原生 UUID / JSON **列型** / 数组 / 文档列型 /
//!    网络 / MAC ⇒ 这些变体落到**有依据的文本载体**上（UUID 是 BLOB 16 字节、
//!    `Vec<scalar>` 与 `#[document]` 是 `CLOB` 里的一段 JSON 文本、`BigDecimal` 是十进制
//!    文本），或者如实报 `unsupported_feature`——绝不留一个「以为落库成功」的静默降级
//!    （`Record`/`SparseRecord`、`Interval` 就是后者）。
//! 3. **解码由 `ret: Option<Vec<stmt::Type>>`（或 ODBC 元数据推断）驱动，不由 DM 的
//!    列类型反推**：引擎已经知道它要什么类型，驱动只负责按那个类型还原。
//!
//! # 时间：两套类型系统的桥
//!
//! `stmt::Value` 的时间载体是 **`jiff`**（`Timestamp`/`Zoned`/`Date`/`Time`/`DateTime`），
//! `DmValue` 的是 **`chrono`**。桥接规则（§9.4.1）：
//!
//! | `stmt` 侧 | `DmValue` 侧 | 说明 |
//! |---|---|---|
//! | `Timestamp`（瞬时，UTC） | `Timestamp(NaiveDateTime)` | 一律按 UTC 归一 |
//! | `DateTime`（民用日期时间） | `Timestamp(NaiveDateTime)` | 落 `DATETIME(p)` 列 |
//! | `Zoned`（带时区） | `Text`（RFC 3339） | DM 无带时区类型；正常路径引擎已 cast 成 `String` |
//!
//! `chrono` 的可表示范围比 `jiff` 窄，越界时报 `serialization_failure` 而不是取回 1970。

use dm_core::DmValue;
use toasty_core::{driver::operation::TypedValue, stmt};

/// 把一次执行的绑定参数整批编成 `DmValue`（顺序与 `Expr::Arg(n)` 一致）。
///
/// # Errors
///
/// 任一参数的类型达梦放不下时返回 `unsupported_feature`。
pub(crate) fn encode_params(params: &[TypedValue]) -> Result<Vec<DmValue>, toasty_core::Error> {
    params.iter().map(|p| encode_value(&p.value)).collect()
}

/// 单个 `stmt::Value` → `DmValue`。
///
/// `TypedValue.ty`（`db::Type`）**不参与**编码决策：值自己的变体已经足够决定参数缓冲
/// 里的形态，而 `db::Type` 只描述列该怎么建（DDL 侧，见 fork 的 `ty.rs`）。
///
/// # Errors
///
/// 达梦无对应表示时返回 `unsupported_feature`。
pub(crate) fn encode_value(value: &stmt::Value) -> Result<DmValue, toasty_core::Error> {
    use stmt::Value as V;

    Ok(match value {
        V::Null => DmValue::Null,
        V::Bool(v) => DmValue::Bool(*v),

        V::I8(v) => DmValue::TinyInt(*v),
        V::I16(v) => DmValue::SmallInt(*v),
        V::I32(v) => DmValue::Int(*v),
        V::I64(v) => DmValue::BigInt(*v),

        // 无符号按「宽一级的有符号」编码（§9.4 规则 14，DM 无 UNSIGNED）。
        V::U8(v) => DmValue::SmallInt(i16::from(*v)),
        V::U16(v) => DmValue::Int(i32::from(*v)),
        V::U32(v) => DmValue::BigInt(i64::from(*v)),
        V::U64(v) => match i64::try_from(*v) {
            Ok(v) => DmValue::BigInt(v),
            Err(_) => {
                return Err(unsupported(
                    "u64 超出达梦 BIGINT 上限（DM 无无符号 64 位整数列）",
                ));
            }
        },

        V::F32(v) => DmValue::Real(*v),
        V::F64(v) => DmValue::Double(*v),

        V::String(v) => DmValue::Text(v.clone()),
        // 原始字节直进参数缓冲：没有 base64、没有 hex（§9.4.1）。
        V::Bytes(v) => DmValue::Bytes(v.clone()),
        // `native_uuid = false` + `default_uuid_type = Blob` ⇒ UUID 落 BLOB（16 字节大端）。
        V::Uuid(v) => DmValue::Bytes(v.as_bytes().to_vec()),

        V::Decimal(v) => DmValue::Decimal(*v),

        V::Timestamp(v) => DmValue::Timestamp(timestamp_to_chrono(*v)?),
        V::DateTime(v) => DmValue::Timestamp(datetime_to_chrono(*v)?),
        V::Date(v) => DmValue::Date(date_to_chrono(*v)?),
        V::Time(v) => DmValue::Time(time_to_chrono(*v)?),
        V::Zoned(v) => DmValue::Text(v.to_string()),

        // `bigdecimal`（任意精度）：达梦侧没有对应列型，`StorageTypes::default_bigdecimal_type`
        // 是 `Text`、显式 `numeric(p,s)` 列则靠**文本参数**——达梦的 ODBC 驱动把
        // `SQL_C_WCHAR` 转成数值列（真机探针第 24 节）。`Display` 不丢位（38 位尾数原样），
        // 不经过 `f64`、也不截到 `rust_decimal` 的 96 位。
        V::BigDecimal(v) => DmValue::Text(v.to_string()),

        // 网络地址族：`net` feature 打开时才有。达梦无原生类型（`native_cidr/inet/
        // macaddr/macaddr8 = false`），`Capability` 把四个默认存储类型都指向 `Text`
        // ⇒ 按各自规范的文本形态编码，解码侧再按 `stmt::Type` 还原。
        V::Cidr(v) => DmValue::Text(v.to_string()),
        V::Inet(v) => DmValue::Text(v.to_string()),
        V::MacAddr(v) => DmValue::Text(v.to_string()),
        V::MacAddr8(v) => DmValue::Text(v.to_string()),
        // `Vec<scalar>`（T8.12）：达梦没有数组列，列表落在 `CLOB` 里的一段 JSON 数组文本
        // （`crate::list_json`）。**标量元素**走这条（T8.12 起被真机套件钉住的规范形态）；
        // 含文档/嵌套列表的元素（T8.15 的 `Vec<#[document]>` 集合）改走 fork 的
        // `toasty_sql::json`——那是上游 MySQL/SQLite/PostgreSQL 文档列的同一套编解码。
        V::List(items) => DmValue::Text(list_json_text(value, items)?),
        // `#[document]`（T8.15）：整篇文档落 `CLOB` 里的一段 JSON 文本，编码与上游文档列
        // 一致（`toasty_sql::json`；对象里值为 `Null` 的键会被省略，见 `json.rs`）。
        V::Object(_) => DmValue::Text(document_json_text(value)?),
        // 结构性记录（`Record`/`SparseRecord`）在文档列上**不会**出现：文档列的值以
        // `Value::Object`（或对象列表）的形态到达驱动（`toasty_sql::json` 的文档）。
        // 真出现说明上层绕过了文档 lowering ⇒ 如实报不支持，不猜一个 JSON 形状。
        V::Record(_) | V::SparseRecord(_) => {
            return Err(unsupported(
                "记录值（`Record`/`SparseRecord`）：文档列的值以 `Object` 形态到达驱动，\
                 出现记录值说明上层绕过了文档 lowering",
            ));
        }
    })
}

/// 结果列的值 → `stmt::Value`，目标类型由引擎给出（`QuerySql.ret`）。
///
/// # Errors
///
/// 达梦给回来的值无法按目标类型还原时返回 `serialization_failure`。
pub(crate) fn decode(value: &DmValue, ty: &stmt::Type) -> Result<stmt::Value, toasty_core::Error> {
    use stmt::{Type as T, Value as V};

    // `NULL` 对任何目标类型都合法。
    if matches!(value, DmValue::Null) {
        return Ok(V::Null);
    }

    Ok(match ty {
        // 达梦没有布尔列类型（`BOOL`/`BOOLEAN` 建表报 `[22019/-6119] 无效的数据类型`，
        // 见 fork 的 `ty.rs`：我们渲染成 `BIT`）。真机上 `BIT` 列会以 `DmValue::Bool`
        // 回来；而**布尔表达式**（比较、`EXISTS`、`LIKE` …）达梦不接受直接当投影列，
        // fork 会包成 `CASE WHEN … THEN 1 ELSE 0 END`（`Projected`），于是这里拿到的是
        // 整数 0/1。⇒ 整数变体按 SQL 语义读成布尔（0 = false，非 0 = true）。
        T::Bool => V::Bool(match value {
            DmValue::Bool(v) => *v,
            DmValue::TinyInt(v) => *v != 0,
            DmValue::SmallInt(v) => *v != 0,
            DmValue::Int(v) => *v != 0,
            DmValue::BigInt(v) => *v != 0,
            other => return Err(mismatch(other, ty)),
        }),

        T::I8 => V::I8(
            i8::try_from(as_i64(value).ok_or_else(|| mismatch(value, ty))?)
                .map_err(|_| mismatch(value, ty))?,
        ),
        T::I16 => V::I16(
            i16::try_from(as_i64(value).ok_or_else(|| mismatch(value, ty))?)
                .map_err(|_| mismatch(value, ty))?,
        ),
        T::I32 => V::I32(
            i32::try_from(as_i64(value).ok_or_else(|| mismatch(value, ty))?)
                .map_err(|_| mismatch(value, ty))?,
        ),
        T::I64 => V::I64(as_i64(value).ok_or_else(|| mismatch(value, ty))?),

        T::U8 => V::U8(
            u8::try_from(as_i64(value).ok_or_else(|| mismatch(value, ty))?)
                .map_err(|_| mismatch(value, ty))?,
        ),
        T::U16 => V::U16(
            u16::try_from(as_i64(value).ok_or_else(|| mismatch(value, ty))?)
                .map_err(|_| mismatch(value, ty))?,
        ),
        T::U32 => V::U32(
            u32::try_from(as_i64(value).ok_or_else(|| mismatch(value, ty))?)
                .map_err(|_| mismatch(value, ty))?,
        ),
        T::U64 => V::U64(
            u64::try_from(as_i64(value).ok_or_else(|| mismatch(value, ty))?)
                .map_err(|_| mismatch(value, ty))?,
        ),

        T::F32 => V::F32(match value {
            DmValue::Real(v) => *v,
            DmValue::Double(v) => *v as f32,
            other => as_i64(other)
                .map(|v| v as f32)
                .ok_or_else(|| mismatch(other, ty))?,
        }),
        T::F64 => V::F64(match value {
            DmValue::Double(v) => *v,
            DmValue::Real(v) => f64::from(*v),
            other => as_i64(other)
                .map(|v| v as f64)
                .ok_or_else(|| mismatch(other, ty))?,
        }),

        T::String => match value {
            DmValue::Text(v) => V::String(v.clone()),
            other => return Err(mismatch(other, ty)),
        },
        T::Bytes => match value {
            DmValue::Bytes(v) => V::Bytes(v.clone()),
            other => return Err(mismatch(other, ty)),
        },
        T::Uuid => V::Uuid(match value {
            DmValue::Bytes(v) => uuid::Uuid::from_slice(v).map_err(|_| mismatch(value, ty))?,
            // 兜底：某些列型会把 UUID 当文本给回来（例如手工建的 VARCHAR(36)）。
            DmValue::Text(v) => v.parse::<uuid::Uuid>().map_err(|_| mismatch(value, ty))?,
            other => return Err(mismatch(other, ty)),
        }),
        // 达梦的 `NUMBER`（= `db::Type::Numeric(None)`，`decimal_arbitrary_precision = true`）
        // 经 ODBC **只能拿到 DOUBLE**：真机实测 `SELECT "n"` / `SELECT CAST(1.5 AS NUMBER)`
        // 都回 `Double(1.5)`，而显式 `DECIMAL(38,10)` 回 `Decimal(2.25)`。既然服务端已经
        // 把值交给了我们一个 f64，这里就照实还原成 `Decimal`（rust_decimal 的 f64 转换，
        // 保 28 位有效数字）；拒绝它只会让 `NUMBER` 列彻底不可用。⚠️ >15 位有效数字的
        // 精度在 ODBC 边界上已经丢了，这与本驱动的解码无关（§9.4.1「已知边界」）。
        T::Decimal => match value {
            DmValue::Decimal(v) => V::Decimal(*v),
            DmValue::Double(v) => {
                V::Decimal(rust_decimal::Decimal::try_from(*v).map_err(|_| mismatch(value, ty))?)
            }
            DmValue::Real(v) => V::Decimal(
                rust_decimal::Decimal::try_from(f64::from(*v)).map_err(|_| mismatch(value, ty))?,
            ),
            // dm-odbc 只在**精确**解析装不下时才给文本（`decimal_text_value` + `from_str_exact`）。
            // 目标是 `rust_decimal` 时按它的语义解析（超位会四舍五入），与改前 `FromStr` 的行为
            // 一致——想一位不丢就得声明 `BigDecimal` 字段（下面那一臂按原文精确解析）。
            DmValue::Text(v) => V::Decimal(
                v.trim()
                    .parse::<rust_decimal::Decimal>()
                    .map_err(|_| mismatch(value, ty))?,
            ),
            other => return Err(mismatch(other, ty)),
        },

        // 任意精度十进制（`bigdecimal_implemented = true`，T8.14）。值可能来自三种载体：
        // 超出 `rust_decimal` 的列走 dm-odbc 的文本路径（`DmValue::Text`，见
        // `result.rs::decimal_text_value`）、`NUMBER` 列经 ODBC 只给 f64（`Double`）、
        // 28 位以内的 `DECIMAL` 是 `Decimal`。三者都**按十进制字符串**转，避免再丢一次精度。
        T::BigDecimal => V::BigDecimal(match value {
            DmValue::Text(v) => v
                .trim()
                .parse::<bigdecimal::BigDecimal>()
                .map_err(|_| mismatch(value, ty))?,
            DmValue::Decimal(v) => v
                .to_string()
                .parse::<bigdecimal::BigDecimal>()
                .map_err(|_| mismatch(value, ty))?,
            DmValue::Double(v) => v
                .to_string()
                .parse::<bigdecimal::BigDecimal>()
                .map_err(|_| mismatch(value, ty))?,
            DmValue::Real(v) => f64::from(*v)
                .to_string()
                .parse::<bigdecimal::BigDecimal>()
                .map_err(|_| mismatch(value, ty))?,
            other => return Err(mismatch(other, ty)),
        }),

        T::Timestamp => V::Timestamp(chrono_to_timestamp(match value {
            DmValue::Timestamp(v) => *v,
            DmValue::Date(v) => v.and_hms_opt(0, 0, 0).ok_or_else(|| mismatch(value, ty))?,
            other => return Err(mismatch(other, ty)),
        })?),
        T::DateTime => V::DateTime(chrono_to_datetime(match value {
            DmValue::Timestamp(v) => *v,
            DmValue::Date(v) => v.and_hms_opt(0, 0, 0).ok_or_else(|| mismatch(value, ty))?,
            other => return Err(mismatch(other, ty)),
        })),
        T::Date => V::Date(match value {
            DmValue::Date(v) => chrono_to_date(*v),
            DmValue::Timestamp(v) => chrono_to_date(v.date()),
            other => return Err(mismatch(other, ty)),
        }),
        T::Time => V::Time(match value {
            DmValue::Time(v) => chrono_to_time(*v),
            DmValue::Timestamp(v) => chrono_to_time(v.time()),
            other => return Err(mismatch(other, ty)),
        }),

        // `Vec<scalar>`（T8.12）：列是 `CLOB`（fork 的 `ty.rs` 给 `db::Type::List` 选的列型），
        // ODBC 把 `CLOB` 交给 `DmValue::Text` ⇒ 这里按元素类型解开 JSON 数组文本。
        // ⚠️ 只有 `vec_scalar` 能力位为真时才可能走到这里（见 capability.rs）。
        //
        // `Vec<#[document]>`（T8.15 的 `document_collections`）元素是**对象**：交给 fork 的
        // `toasty_sql::json`（与上游文档列同源），因为 `crate::list_json` 只管标量。
        T::List(elem) if matches!(**elem, T::Object) => match value {
            DmValue::Text(v) => document_json_value(v, ty)?,
            other => return Err(mismatch(other, ty)),
        },
        T::List(_) => match value {
            DmValue::Text(v) => V::List(crate::list_json::decode(v, ty)?),
            other => return Err(mismatch(other, ty)),
        },

        // `#[document]`（T8.15）：单篇文档列同样是 `CLOB` 里的一段 JSON 文本。解码是
        // **结构性**的——字符串留字符串、数字按整数贴合度选宽度、对象按键序成
        // `Value::Object`；引擎随后把它抬成有类型的记录（`toasty_sql::json` 的文档）。
        T::Object => match value {
            DmValue::Text(v) => document_json_value(v, ty)?,
            other => return Err(mismatch(other, ty)),
        },

        // `Zoned` 在 DM 上没有落点：`StorageTypes::default_zoned_type = Text`，正常路径引擎
        // 已经把它 cast 成 `String`（§9.4.1）。真走到这里说明上层绕过了 cast，如实报不支持。
        T::Zoned => {
            return Err(unsupported(
                "Zoned 结果列：达梦无带时区类型，引擎应先把该列 cast 成 String",
            ));
        }

        other => {
            return Err(unsupported(format!(
                "结果列类型 {other:?}：达梦无原生对应（记录/文档/数组/网络/MAC/枚举别名等）"
            )));
        }
    })
}

/// `RawSqlRet::Infer` 用：按 `DmValue` 自己的变体挑最接近的 `stmt::Value`。
///
/// `dm-query` 已经按 ODBC 元数据把裸缓冲解成了 `DmValue`，所以这里就是「用元数据推断」。
///
/// # Errors
///
/// `DmValue::Interval`（DM 的 `INTERVAL`）与 `DmValue::Unknown` 在 `stmt::Value` 里
/// 没有对应变体——前者报不支持，后者按原始字节给 `Bytes`。
pub(crate) fn infer(value: &DmValue) -> Result<stmt::Value, toasty_core::Error> {
    use stmt::Value as V;

    Ok(match value {
        DmValue::Null => V::Null,
        // 达梦的布尔列是 `BIT`（`Flavor::Dameng` 的 `Type::Boolean` 渲染臂），而在
        // 「按元数据推断存储值」这条路上，上游对**非 PostgreSQL** 后端统一期望整数值
        // （`raw_sql::query_infers_storage_values`：`is_postgresql(&db)` 才要 `Bool(true)`，
        // 其余要 `Value::I64(1)`，MySQL/SQLite 的 BOOLEAN 也是整数存储）。
        // 需要真正的布尔值时由调用方用 `ret = Types([…Bool…])` 声明，那时走 `decode`，
        // 回的仍是 `Value::Bool` ✓。
        DmValue::Bool(v) => V::I64(i64::from(*v)),
        DmValue::TinyInt(v) => V::I8(*v),
        DmValue::SmallInt(v) => V::I16(*v),
        DmValue::Int(v) => V::I32(*v),
        DmValue::BigInt(v) => V::I64(*v),
        DmValue::Real(v) => V::F32(*v),
        DmValue::Double(v) => V::F64(*v),
        DmValue::Decimal(v) => V::Decimal(*v),
        DmValue::Text(v) => V::String(v.clone()),
        DmValue::Bytes(v) => V::Bytes(v.clone()),
        DmValue::Date(v) => V::Date(chrono_to_date(*v)),
        DmValue::Time(v) => V::Time(chrono_to_time(*v)),
        DmValue::Timestamp(v) => V::Timestamp(chrono_to_timestamp(*v)?),
        DmValue::Interval(_) => {
            return Err(unsupported("INTERVAL 结果列：stmt::Value 没有区间变体"));
        }
        DmValue::Unknown(v) => V::Bytes(v.clone()),
        // `DmValue` 是 `#[non_exhaustive]`：上游将来加变体时，这里**报不支持**而不是猜一个
        // 有损的映射（本项目的原则：绝不静默降级）。
        other => {
            return Err(unsupported(format!(
                "未知的 DmValue 变体 {}（dm-core 与驱动版本不匹配？）",
                variant_name(other)
            )));
        }
    })
}

// ---------------------------------------------------------------------------
// 文档 / 结构化列表的 JSON 文本（T8.15，`document_collections`）
// ---------------------------------------------------------------------------

/// `Value::List` → 列里的 JSON 文本：标量元素走 `crate::list_json`（T8.12 起被真机套件
/// 钉住的规范形态，改动它等于改动已验证的 `Vec<scalar>` 语义），只要有一个元素是对象
/// 或嵌套列表，就整段交给 fork 的 [`toasty_sql::json`]（上游文档列的同一套编解码）。
fn list_json_text(
    value: &stmt::Value,
    items: &[stmt::Value],
) -> Result<String, toasty_core::Error> {
    if items
        .iter()
        .any(|item| matches!(item, stmt::Value::Object(_) | stmt::Value::List(_)))
    {
        document_json_text(value)
    } else {
        crate::list_json::encode(items)
    }
}

/// 文档值（`Value::Object`，或元素是对象的列表）→ JSON 文本。
///
/// 用的是 vendored fork 的 `toasty_sql::json`：上游 MySQL/SQLite/PostgreSQL 的
/// `#[document]` 列走同一套编解码（对象里值为 `Null` 的键省略、UUID/十进制/时间按
/// 「文档存储文本」写出），所以达梦的 `CLOB` 文档列与它们**逐字节同构**。
fn document_json_text(value: &stmt::Value) -> Result<String, toasty_core::Error> {
    toasty_sql::json::to_string(value).map_err(|e| {
        toasty_core::Error::serialization_failure(format!(
            "文档值编成 JSON 失败：{e}（`#[document]` 列在达梦上是一段 CLOB JSON 文本）"
        ))
    })
}

/// `toasty_sql::json` 写出的 JSON 文本 → `Value`（`ty` 是 `Object` 或 `List(Object)`）。
///
/// 解码是**结构性**的：字符串留字符串、数字按整数贴合度选宽度、对象按键序成
/// `Value::Object`；引擎随后把它抬成有类型的记录（见 `toasty_sql::json` 的模块文档）。
fn document_json_value(text: &str, ty: &stmt::Type) -> Result<stmt::Value, toasty_core::Error> {
    toasty_sql::json::from_str(text, ty).map_err(|e| {
        toasty_core::Error::serialization_failure(format!(
            "文档列文本解成 {ty:?} 失败：{e}（列内容不是 `toasty_sql::json` 写出的 JSON 文本）"
        ))
    })
}

// ---------------------------------------------------------------------------
// 数值：从任意整数变体取出 i64
// ---------------------------------------------------------------------------

fn as_i64(value: &DmValue) -> Option<i64> {
    match value {
        DmValue::TinyInt(v) => Some(i64::from(*v)),
        DmValue::SmallInt(v) => Some(i64::from(*v)),
        DmValue::Int(v) => Some(i64::from(*v)),
        DmValue::BigInt(v) => Some(*v),
        // 达梦的 `@@IDENTITY` 与 `NUMBER` 列走 DECIMAL 回来：整数标度才能取。
        DmValue::Decimal(d) => {
            // `to_i64` 来自 `num_traits::ToPrimitive`（`rust_decimal` 的 prelude 重导出）。
            use rust_decimal::prelude::ToPrimitive as _;
            d.to_i64()
        }
        // 极少数列型（如手工建的 VARCHAR）会把整数以文本给回来——**不**在这里接受：
        // 「按 `ret` 解码、类型不符就报错、绝不静默转换」是本驱动的原则（§9.4.1），
        // 文本→整数属于静默转换，真需要时由引擎显式 `stmt::Type::cast` 完成。
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// jiff ↔ chrono
// ---------------------------------------------------------------------------

/// 亚微秒一律**截断**（不是四舍五入）。
///
/// 达梦的时间列只有微秒（`TIMESTAMP(6)` / `TIME(6)`），但**绑定参数**上的亚微秒会被
/// 达梦**四舍五入**：探针第 30 节把 `.123456789` 绑进去，`TO_CHAR(?)` 回 `.123457`
/// （30f；`CAST(? AS TIMESTAMP(6))` 也一样，30j）。而文档列的存储文本是 JSON 编解码器
/// 按 `RoundMode::Trunc` **截断**写进去的（`.123456`）⇒ 按原始纳秒值过滤命中不了
/// （30n 0 行 / 30o 用微秒参数 1 行）。上游套件的契约写得很明白 —— 「JSON 编解码器写入
/// 时截断，以匹配各驱动**截断式**的参数绑定，于是按原始纳秒值等值过滤仍能命中被截断
/// 存储的行」（`type_document::struct_embed_timestamp_truncates_to_micros` 的文档注释）
/// ⇒ 驱动这边也截断。写入与过滤走同一条编码，普通时间列同样自洽（不会出现「写入按
/// 达梦四舍五入、过滤按我们截断」的两套精度）。
fn truncate_to_micros(nanos: u32) -> u32 {
    nanos / 1_000 * 1_000
}

fn timestamp_to_chrono(t: jiff::Timestamp) -> Result<chrono::NaiveDateTime, toasty_core::Error> {
    chrono::DateTime::from_timestamp(
        t.as_second(),
        truncate_to_micros(u32::try_from(t.subsec_nanosecond()).unwrap_or(0)),
    )
    .map(|dt| dt.naive_utc())
    .ok_or_else(|| {
        toasty_core::Error::serialization_failure(
            "jiff Timestamp 超出 chrono 可表示范围（±26 万年）",
        )
    })
}

/// `chrono::NaiveDateTime`（约定为 UTC）→ `jiff::Timestamp`，**保留亚秒**。
///
/// 不用 `chrono` 的 `timestamp_nanos_opt()`：那只看 1677–2262 年这一段；这里用
/// 「整秒 × 1e9 + 纳秒」自己算，再交给 `jiff::Timestamp::from_nanosecond`（i128），
/// 只有超出 jiff 的 ±9999 年才报错。
///
/// # Errors
///
/// 时间戳超出 jiff 可表示范围时返回 `serialization_failure`（**不**静默截断到 epoch）。
fn chrono_to_timestamp(t: chrono::NaiveDateTime) -> Result<jiff::Timestamp, toasty_core::Error> {
    use chrono::Timelike as _;

    let dt = t.and_utc();
    let nanos = i128::from(dt.timestamp()) * 1_000_000_000 + i128::from(dt.nanosecond());
    jiff::Timestamp::from_nanosecond(nanos).map_err(|_| {
        toasty_core::Error::serialization_failure("chrono 时间戳超出 jiff 可表示范围（±9999 年）")
    })
}

fn chrono_to_datetime(t: chrono::NaiveDateTime) -> jiff::civil::DateTime {
    jiff::civil::DateTime::from_parts(chrono_to_date(t.date()), chrono_to_time(t.time()))
}

fn date_to_chrono(d: jiff::civil::Date) -> Result<chrono::NaiveDate, toasty_core::Error> {
    chrono::NaiveDate::from_ymd_opt(
        i32::from(d.year()),
        u32::from(d.month() as u8),
        u32::from(d.day() as u8),
    )
    .ok_or_else(|| toasty_core::Error::serialization_failure("jiff Date 不是合法公历日期"))
}

fn chrono_to_date(d: chrono::NaiveDate) -> jiff::civil::Date {
    use chrono::Datelike as _;
    jiff::civil::Date::new(d.year() as i16, d.month() as i8, d.day() as i8)
        .unwrap_or(jiff::civil::Date::ZERO)
}

fn time_to_chrono(t: jiff::civil::Time) -> Result<chrono::NaiveTime, toasty_core::Error> {
    chrono::NaiveTime::from_hms_nano_opt(
        u32::from(t.hour() as u8),
        u32::from(t.minute() as u8),
        u32::from(t.second() as u8),
        truncate_to_micros(u32::try_from(t.subsec_nanosecond()).unwrap_or(0)),
    )
    .ok_or_else(|| toasty_core::Error::serialization_failure("jiff Time 不是合法时刻"))
}

fn chrono_to_time(t: chrono::NaiveTime) -> jiff::civil::Time {
    use chrono::Timelike as _;
    jiff::civil::Time::new(
        t.hour() as i8,
        t.minute() as i8,
        t.second() as i8,
        t.nanosecond() as i32,
    )
    .unwrap_or_else(|_| jiff::civil::time(0, 0, 0, 0))
}

fn datetime_to_chrono(
    dt: jiff::civil::DateTime,
) -> Result<chrono::NaiveDateTime, toasty_core::Error> {
    Ok(date_to_chrono(dt.date())?.and_time(time_to_chrono(dt.time())?))
}

// ---------------------------------------------------------------------------
// 错误构造
// ---------------------------------------------------------------------------

fn unsupported(what: impl AsRef<str>) -> toasty_core::Error {
    toasty_core::Error::unsupported_feature(format!("达梦驱动不支持 {}", what.as_ref()))
}

fn mismatch(value: &DmValue, ty: &stmt::Type) -> toasty_core::Error {
    toasty_core::Error::serialization_failure(format!(
        "达梦返回的 {} 无法还原成目标类型 {ty:?}（不做有损转换）",
        variant_name(value)
    ))
}

/// 诊断用：`DmValue` 的变体名（`Display` 会带上具体值，日志里太长）。
fn variant_name(value: &DmValue) -> &'static str {
    match value {
        DmValue::Null => "NULL",
        DmValue::Bool(_) => "BOOL",
        DmValue::TinyInt(_) => "TINYINT",
        DmValue::SmallInt(_) => "SMALLINT",
        DmValue::Int(_) => "INT",
        DmValue::BigInt(_) => "BIGINT",
        DmValue::Real(_) => "REAL",
        DmValue::Double(_) => "DOUBLE",
        DmValue::Decimal(_) => "DECIMAL",
        DmValue::Text(_) => "TEXT",
        DmValue::Bytes(_) => "BYTES",
        DmValue::Date(_) => "DATE",
        DmValue::Time(_) => "TIME",
        DmValue::Timestamp(_) => "TIMESTAMP",
        DmValue::Interval(_) => "INTERVAL",
        DmValue::Unknown(_) => "UNKNOWN",
        // `DmValue` 是 `#[non_exhaustive]`：未知变体如实标出来，别硬套一个已有名字。
        _ => "OTHER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dm_core::DmInterval;

    #[test]
    fn integers_widen_but_never_truncate() {
        assert_eq!(
            encode_value(&stmt::Value::U32(u32::MAX)).unwrap(),
            DmValue::BigInt(4_294_967_295)
        );
        // u64 超过 i64::MAX：没有 DM 列放得下 ⇒ 必须报不支持，而不是截断成负数。
        let err = encode_value(&stmt::Value::U64(u64::MAX)).unwrap_err();
        assert!(err.is_unsupported_feature(), "实际：{err}");
    }

    #[test]
    fn uuid_goes_to_blob_bytes() {
        let id = uuid::Uuid::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        assert_eq!(
            encode_value(&stmt::Value::Uuid(id)).unwrap(),
            DmValue::Bytes(id.as_bytes().to_vec())
        );
    }

    #[test]
    fn round_trips_timestamp_through_chrono() {
        let ts: jiff::Timestamp = "2026-01-02T03:04:05.678901Z".parse().unwrap();
        let encoded = encode_value(&stmt::Value::Timestamp(ts)).unwrap();
        assert!(matches!(encoded, DmValue::Timestamp(_)));
        let decoded = decode(&encoded, &stmt::Type::Timestamp).unwrap();
        assert_eq!(decoded, stmt::Value::Timestamp(ts));
    }

    #[test]
    fn null_decodes_to_null_for_every_target() {
        for ty in [
            stmt::Type::String,
            stmt::Type::I64,
            stmt::Type::Timestamp,
            stmt::Type::Bytes,
        ] {
            assert_eq!(decode(&DmValue::Null, &ty).unwrap(), stmt::Value::Null);
        }
    }

    #[test]
    fn decode_refuses_lossy_conversions() {
        // 文本列给回文本、目标要整数 ⇒ 报错（不猜、不 parse）。
        let err = decode(&DmValue::Text("12".to_owned()), &stmt::Type::I64).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
        // 越界的整数也不许截断。
        let err = decode(&DmValue::BigInt(300), &stmt::Type::I8).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
    }

    #[test]
    fn interval_and_documents_are_refused() {
        let record = stmt::Value::Record(stmt::ValueRecord::from_vec(vec![stmt::Value::I64(1)]));
        let err = encode_value(&record).unwrap_err();
        assert!(err.is_unsupported_feature(), "实际：{err}");
        let err = infer(&DmValue::Interval(DmInterval::default())).unwrap_err();
        assert!(err.is_unsupported_feature(), "实际：{err}");
    }

    /// `Vec<scalar>`（T8.12）：`V::List` ⇄ `CLOB` 里的 JSON 数组文本。
    #[test]
    fn lists_round_trip_through_json_text() {
        let items = vec![
            stmt::Value::String("admin".to_owned()),
            stmt::Value::String("带\"引号\"".to_owned()),
            stmt::Value::Null,
        ];
        let encoded = encode_value(&stmt::Value::List(items.clone())).unwrap();
        assert_eq!(
            encoded,
            DmValue::Text("[\"admin\",\"带\\\"引号\\\"\",null]".to_owned())
        );

        let ty = stmt::Type::List(Box::new(stmt::Type::String));
        assert_eq!(decode(&encoded, &ty).unwrap(), stmt::Value::List(items));

        // 空列表也要能往返（`[]`）。
        let empty = stmt::Value::List(Vec::new());
        let encoded = encode_value(&empty).unwrap();
        assert_eq!(decode(&encoded, &ty).unwrap(), empty);

        // 列表列给回的不是文本 ⇒ 报类型不符，不猜。
        let err = decode(&DmValue::Int(1), &ty).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
    }

    /// `#[document]`（T8.15）：`V::Object`（以及「元素是对象的列表」）⇄ `CLOB` 里的 JSON 文本。
    #[test]
    fn documents_round_trip_through_json_text() {
        let doc = stmt::Value::Object(stmt::ValueObject::from_vec(vec![
            ("s".to_owned(), stmt::Value::String("hello".to_owned())),
            ("n".to_owned(), stmt::Value::I64(42)),
            ("b".to_owned(), stmt::Value::Bool(true)),
            // `Option::None` 的字段编成「缺失的键」，不是显式 `null`。
            ("none".to_owned(), stmt::Value::Null),
        ]));
        let encoded = encode_value(&doc).unwrap();
        assert_eq!(
            encoded,
            DmValue::Text("{\"s\":\"hello\",\"n\":42,\"b\":true}".to_owned())
        );
        // 解码是结构性的：数字按 wire 形态（`I64`）回来、`None` 的键缺席；引擎再抬成记录。
        let decoded = decode(&encoded, &stmt::Type::Object).unwrap();
        assert_eq!(
            decoded,
            stmt::Value::Object(stmt::ValueObject::from_vec(vec![
                ("s".to_owned(), stmt::Value::String("hello".to_owned())),
                ("n".to_owned(), stmt::Value::I64(42)),
                ("b".to_owned(), stmt::Value::Bool(true)),
            ]))
        );

        // 文档集合（`Vec<#[document]>`）：元素是对象 ⇒ 走同一套编解码（不是 `list_json`）。
        let list = stmt::Value::List(vec![stmt::Value::Object(stmt::ValueObject::from_vec(
            vec![("k".to_owned(), stmt::Value::String("deep".to_owned()))],
        ))]);
        let encoded = encode_value(&list).unwrap();
        assert_eq!(encoded, DmValue::Text("[{\"k\":\"deep\"}]".to_owned()));
        let ty = stmt::Type::List(Box::new(stmt::Type::Object));
        assert_eq!(decode(&encoded, &ty).unwrap(), list);

        // 列里不是 JSON 文本 ⇒ 报错，不猜。
        let err = decode(&DmValue::Text("不是 JSON".to_owned()), &stmt::Type::Object).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
    }

    /// `BigDecimal`（T8.14）：文本载体，38 位尾数一位不丢。
    #[test]
    fn bigdecimal_round_trips_through_text_without_losing_digits() {
        let text = "123456789012345678.12345678901234567890";
        let value: bigdecimal::BigDecimal = text.parse().unwrap();
        let encoded = encode_value(&stmt::Value::BigDecimal(value.clone())).unwrap();
        assert_eq!(encoded, DmValue::Text(text.to_owned()));
        assert_eq!(
            decode(&encoded, &stmt::Type::BigDecimal).unwrap(),
            stmt::Value::BigDecimal(value)
        );
        // 28 位以内的 `DECIMAL` 列（ODBC 给 `Decimal`）与 `NUMBER` 列（只给 f64）也要能还原。
        assert_eq!(
            decode(
                &DmValue::Decimal(rust_decimal::Decimal::new(225, 2)),
                &stmt::Type::BigDecimal
            )
            .unwrap(),
            stmt::Value::BigDecimal("2.25".parse().unwrap())
        );
        assert_eq!(
            decode(&DmValue::Double(2.25), &stmt::Type::BigDecimal).unwrap(),
            stmt::Value::BigDecimal("2.25".parse().unwrap())
        );
        // 非十进制文本不许猜。
        let err = decode(&DmValue::Text("不是数".to_owned()), &stmt::Type::BigDecimal).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
    }

    /// 时间值的**参数绑定**截断到微秒（探针第 30 节）。
    ///
    /// 达梦会把绑定参数上的亚微秒四舍五入（30f：`.123456789` → `.123457`），而文档列的
    /// 存储文本是 JSON 编解码器**截断**写进去的（`.123456`）⇒ 不截断就命中不了。
    /// 套件 `type_document::struct_embed_timestamp_truncates_to_micros` 钉住了这条契约。
    #[test]
    fn sub_microsecond_digits_are_truncated_before_binding() {
        use jiff::civil;

        let ts = jiff::Timestamp::from_nanosecond(1_704_164_645_123_456_789).unwrap();
        assert_eq!(
            encode_value(&stmt::Value::Timestamp(ts)).unwrap(),
            DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_micro_opt(3, 4, 5, 123_456)
                    .unwrap()
            )
        );

        // `Time` 同一条规矩（`TIME(6)`），经 `DateTime` 的 `time_to_chrono` 也走这里。
        let time = civil::Time::new(3, 4, 5, 123_456_789).unwrap();
        assert_eq!(
            encode_value(&stmt::Value::Time(time)).unwrap(),
            DmValue::Time(chrono::NaiveTime::from_hms_micro_opt(3, 4, 5, 123_456).unwrap())
        );
        let dt = civil::DateTime::from_parts(
            civil::Date::new(2024, 1, 2).unwrap(),
            civil::Time::new(3, 4, 5, 123_456_789).unwrap(),
        );
        assert_eq!(
            encode_value(&stmt::Value::DateTime(dt)).unwrap(),
            DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_micro_opt(3, 4, 5, 123_456)
                    .unwrap()
            )
        );
    }

    #[test]
    fn infer_maps_native_variants() {
        assert_eq!(infer(&DmValue::Int(-7)).unwrap(), stmt::Value::I32(-7));
        assert_eq!(infer(&DmValue::Null).unwrap(), stmt::Value::Null);
        assert_eq!(
            infer(&DmValue::Unknown(vec![1, 2])).unwrap(),
            stmt::Value::Bytes(vec![1, 2])
        );
    }
}
