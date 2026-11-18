use super::{Flavor, Ident, ToSql};

use toasty_core::schema::db;

impl ToSql for &db::Type {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            // 达梦（本机 DM8）**没有 BOOLEAN 列类型**：`CREATE TABLE … ("b" BOOLEAN)`
            // 直接报 `[22019/-6119] 无效的数据类型`。`BIT` 是真机实测可用的等价物
            // （真机探针：建表通过、`SELECT "b"` 回 `Bool(true)`、绑 `Bool(false)` 可写入）。
            db::Type::Boolean => match f.serializer.dialect {
                Flavor::Dameng => fmt!(f, "BIT"),
                _ => fmt!(f, "BOOLEAN"),
            },
            db::Type::Integer(1..=2) => fmt!(f, "SMALLINT"),
            db::Type::Integer(3..=4) => fmt!(f, "INTEGER"),
            db::Type::Integer(5..=8) => fmt!(f, "BIGINT"),
            db::Type::Integer(_) => todo!(),
            db::Type::UnsignedInteger(size) => {
                match f.serializer.dialect {
                    Flavor::Mysql | Flavor::MariaDb => match size {
                        1 => fmt!(f, "TINYINT UNSIGNED"),
                        2 => fmt!(f, "SMALLINT UNSIGNED"),
                        3..=4 => fmt!(f, "INT UNSIGNED"),
                        5..=8 => fmt!(f, "BIGINT UNSIGNED"),
                        _ => todo!("Unsupported unsigned integer size: {}", size),
                    },
                    // 达梦没有无符号整数类型：宽一级映射到有符号类型
                    // （与 PostgreSQL 同形，见 `docs/设计方案.md` §9.4 规则 14）。
                    Flavor::Postgresql | Flavor::Dameng => {
                        match size {
                            1 => fmt!(f, "SMALLINT"),   // u8 -> SMALLINT (i16)
                            2 => fmt!(f, "INTEGER"),    // u16 -> INTEGER (i32)
                            3..=4 => fmt!(f, "BIGINT"), // u32 -> BIGINT (i64)
                            5..=8 => fmt!(f, "BIGINT"), // u64 -> BIGINT (i64) with capability limits
                            _ => todo!("Unsupported unsigned integer size: {}", size),
                        }
                    }
                    Flavor::Sqlite => {
                        // SQLite uses INTEGER for all integer types
                        fmt!(f, "INTEGER")
                    }
                }
            }
            db::Type::Float(size) => match f.serializer.dialect {
                Flavor::Sqlite => fmt!(f, "REAL"),
                Flavor::Postgresql => {
                    if *size <= 4 {
                        fmt!(f, "REAL")
                    } else {
                        fmt!(f, "DOUBLE PRECISION")
                    }
                }
                Flavor::Mysql | Flavor::MariaDb => {
                    if *size <= 4 {
                        fmt!(f, "FLOAT")
                    } else {
                        fmt!(f, "DOUBLE")
                    }
                }
                Flavor::Dameng => {
                    if *size <= 4 {
                        fmt!(f, "REAL")
                    } else {
                        fmt!(f, "DOUBLE")
                    }
                }
            },
            db::Type::Text => match f.serializer.dialect {
                // 达梦的 `TEXT` 是 `CLOB` 的别名，但 **`CLOB` 不能建索引、不能用在
                // `CHECK`/`=`/`ORDER BY`/`GROUP BY` 里**（真机实测：枚举列渲染成
                // `CLOB NOT NULL CHECK (...)` 直接报 `[22019/-6119] 无效的数据类型`；
                // 给 CLOB 列建索引报 `[HY000/-3207] 试图在BLOB/CLOB/...上建索引`）。
                // 而 `db::Type::Text` 在引擎里主要出现在**枚举列**（`native_enum = false`
                // ⇒ Text + CHECK）与桥接的 JSON/文档列上，恰恰都需要可比较/可索引。
                // ⇒ 达梦侧改用 `VARCHAR(8188)`：默认 8 KiB 页下**行内**放得下、也仍在索引键长
                // 量级内的保守取值（Round 24 实测：`VARCHAR` 的**语法**上限其实是 32767 字节，
                // `VARCHAR(32768)` 才报 `[HY000/-6121] 数据精度超出范围`；`StorageTypes::varchar`
                // 已按真上限改成 `Some(32767)`，见 `docs/设计方案.md` §14 V-22）。
                // 普通字符串走的是 `default_string_type = VarChar(255)`，不受影响。
                // 副作用（已知限制，§9.4/T8.11 记录）：显式声明超过 8188 字节的字段
                // 会被建成 `VARCHAR(8188)`，写入超长字符串由达梦报 `-70005`（**不会静默截断**）。
                Flavor::Dameng => fmt!(f, "VARCHAR(8188)"),
                _ => fmt!(f, "TEXT"),
            },
            db::Type::VarChar(size) => fmt!(f, "VARCHAR(" size ")"),
            db::Type::Uuid => {
                fmt!(
                    f,
                    match f.serializer.dialect {
                        Flavor::Postgresql | Flavor::MariaDb => "UUID",
                        // 达梦无原生 UUID：16 字节定长二进制（`value.rs` 同口径）。
                        Flavor::Dameng => "VARBINARY(16)",
                        _ => todo!("Unsupported type UUID"),
                    }
                );
            }
            db::Type::Numeric(None) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "NUMERIC"),
                // 达梦沿用 Oracle 的 `NUMBER`（无参数 = 最大精度）。渲染成无参 `DECIMAL` 其实
                // **同样可行**（Round 25 探针 24j/24j2/24k/24l2：`DECIMAL` 列在 `USER_TAB_COLUMNS`
                // 里 `DATA_PRECISION`/`DATA_SCALE` 都是 NULL，插 `1.2345` 回读 `Double(1.2345)`，
                // **没有** `(18, 0)` 截断这种事 —— 早先那条注释是错的，已按实测更正），统一写
                // `NUMBER` 只是为了与达梦文档的写法一致。真正要记住的是：**无约束数值列回读只给
                // f64**（24m/24m2：插 38 位值被接受，回读 `Double(1.2345678901234568e17)`）⇒ 精度
                // 靠 `decimal_arbitrary_precision = false` + 显式 `Numeric(Some((p, s)))` 兜住。
                Flavor::Dameng => fmt!(f, "NUMBER"),
                Flavor::Mysql | Flavor::MariaDb => todo!(
                    "MySQL does not support arbitrary-precision NUMERIC; precision and scale must be specified"
                ),
                Flavor::Sqlite => todo!("SQLite does not support NUMERIC type"),
            },
            db::Type::Numeric(Some((precision, scale))) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "NUMERIC(" precision ", " scale ")"),
                Flavor::Mysql | Flavor::MariaDb | Flavor::Dameng => {
                    fmt!(f, "DECIMAL(" precision ", " scale ")")
                }
                Flavor::Sqlite => todo!("SQLite does not support NUMERIC type"),
            },
            db::Type::Binary(size) => match f.serializer.dialect {
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "BINARY(" size ")"),
                Flavor::Dameng => fmt!(f, "VARBINARY(" size ")"),
                _ => todo!("Unsupported fixed size binary type"),
            },
            db::Type::Blob => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "BYTEA"),
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "BLOB"),
                Flavor::Sqlite => fmt!(f, "BLOB"),
                Flavor::Dameng => fmt!(f, "BLOB"),
            },
            db::Type::Timestamp(precision) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "TIMESTAMPTZ(" precision ")"),
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "TIMESTAMP(" precision ")"),
                // 达梦 `TIMESTAMP(n)` 的小数秒精度上限是 6（微秒），
                // 能力表里声明的是 `Timestamp(6)`。
                Flavor::Dameng => fmt!(f, "TIMESTAMP(" precision ")"),
                Flavor::Sqlite => todo!("SQLite does not support Timestamp"),
            },
            db::Type::Date => match f.serializer.dialect {
                Flavor::Postgresql | Flavor::Mysql | Flavor::MariaDb | Flavor::Dameng => {
                    fmt!(f, "DATE")
                }
                Flavor::Sqlite => todo!("SQLite does not support Date"),
            },
            db::Type::Time(precision) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "TIME(" precision ")"),
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "TIME(" precision ")"),
                Flavor::Dameng => fmt!(f, "TIME(" precision ")"),
                Flavor::Sqlite => todo!("SQLite does not support Time"),
            },
            db::Type::DateTime(precision) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "TIMESTAMP(" precision ")"),
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "DATETIME(" precision ")"),
                Flavor::Dameng => fmt!(f, "DATETIME(" precision ")"),
                Flavor::Sqlite => todo!("SQLite does not support DateTime"),
            },
            db::Type::Cidr => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "CIDR"),
                _ => todo!("Only PostgreSQL supports CIDR"),
            },
            db::Type::Inet => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "INET"),
                _ => todo!("Only PostgreSQL supports INET"),
            },
            db::Type::MacAddr => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "MACADDR"),
                _ => todo!("Only PostgreSQL supports MACADDR"),
            },
            db::Type::MacAddr8 => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "MACADDR8"),
                _ => todo!("Only PostgreSQL supports MACADDR8"),
            },
            db::Type::Enum(type_enum) => match f.serializer.dialect {
                // PostgreSQL: reference the named enum type created with CREATE TYPE.
                Flavor::Postgresql => {
                    let name = type_enum
                        .name
                        .as_deref()
                        .expect("PostgreSQL enums require a type name");
                    fmt!(f, Ident(name));
                }
                // MySQL: inline ENUM('label1', 'label2', ...) column type.
                Flavor::Mysql | Flavor::MariaDb => {
                    use toasty_core::stmt::Value;

                    f.dst.push_str("ENUM(");
                    for (i, variant) in type_enum.variants.iter().enumerate() {
                        if i > 0 {
                            f.dst.push_str(", ");
                        }
                        Value::String(variant.name.clone()).to_sql(f);
                    }
                    f.dst.push(')');
                }
                // SQLite: TEXT column (CHECK constraint added in ColumnDef).
                // 达梦无枚举类型，同 SQLite 用「有界文本 + CHECK 约束」（CHECK 由 ColumnDef 追加）。
                // 必须是 `VARCHAR`：`CLOB` 上既建不了 CHECK（`-6119 无效的数据类型`）
                // 也建不了索引（`-3207`）。
                Flavor::Sqlite => fmt!(f, "TEXT"),
                Flavor::Dameng => fmt!(f, "VARCHAR(255)"),
            },
            db::Type::List(elem) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, elem.as_ref() "[]"),
                // MySQL stores `Vec<scalar>` as a JSON document; SQLite uses
                // TEXT (JSON1 functions operate on either, but TEXT is the
                // idiomatic affinity). The element type is tracked by the
                // engine — it doesn't surface in the column DDL.
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "JSON"),
                Flavor::Sqlite => fmt!(f, "TEXT"),
                // 达梦无数组类型；引擎据 `native_array = false` 把它合法化成
                // JSON 文本存进 `CLOB`。
                Flavor::Dameng => fmt!(f, "CLOB"),
            },
            db::Type::Document { binary } => match f.serializer.dialect {
                // `binary` selects `jsonb` over `json` on PostgreSQL; the text
                // encoding (`#[document(text)]`) is not yet wired up.
                Flavor::Postgresql if *binary => fmt!(f, "JSONB"),
                Flavor::Postgresql => fmt!(f, "JSON"),
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "JSON"),
                Flavor::Sqlite => fmt!(f, "TEXT"),
                Flavor::Dameng => fmt!(f, "CLOB"),
            },
            db::Type::Json => match f.serializer.dialect {
                Flavor::Dameng => fmt!(f, "CLOB"),
                _ => fmt!(f, "JSON"),
            },
            db::Type::Jsonb => match f.serializer.dialect {
                Flavor::Dameng => fmt!(f, "CLOB"),
                _ => fmt!(f, "JSONB"),
            },
            db::Type::Custom(custom) => fmt!(f, custom.as_str()),
        }
    }
}
