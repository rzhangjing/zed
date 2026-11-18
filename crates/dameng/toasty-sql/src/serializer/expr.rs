use toasty_core::{schema::db::ColumnId, stmt::ResolvedRef};

use super::statement::Predicate;
use super::{ColumnAlias, Comma, Delimited, Ident, ToSql};

use crate::{serializer::Flavor, stmt};

impl ToSql for &stmt::Expr {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match self {
            stmt::Expr::And(expr) => {
                fmt!(f, Delimited(expr.operands.iter().map(AndOperand), " AND "));
            }
            stmt::Expr::Between(expr) => {
                fmt!(f, expr.expr " BETWEEN " expr.low " AND " expr.high);
            }
            stmt::Expr::BinaryOp(expr) => {
                assert!(!expr.lhs.is_value_null());
                assert!(!expr.rhs.is_value_null());

                fmt!(f, expr.lhs " " expr.op " " expr.rhs);
            }
            stmt::Expr::Exists(expr) => {
                f.depth += 1;
                fmt!(f, "EXISTS (" expr.subquery ")");
                f.depth -= 1;
            }
            stmt::Expr::Func(stmt::ExprFunc::Count(func)) => match (&func.arg, &func.filter) {
                (None, None) => fmt!(f, "COUNT(*)"),
                // Mysql does not support filters, so translate it to an expression.
                // 达梦同样没有 `FILTER (WHERE …)`，走同一条 CASE 改写。
                (None, Some(expr))
                    if f.serializer.is_mysql()
                        || matches!(f.serializer.dialect, Flavor::Dameng) =>
                {
                    fmt!(f, "COUNT(CASE WHEN " Predicate(expr) " THEN 1 END)")
                }
                (None, Some(expr)) => fmt!(f, "COUNT(*) FILTER (WHERE " expr ")"),
                _ => todo!("func={func:#?}"),
            },
            stmt::Expr::Func(stmt::ExprFunc::LastInsertId(_)) => {
                fmt!(f, "LAST_INSERT_ID()")
            }
            stmt::Expr::Func(stmt::ExprFunc::JsonExtract(func)) => {
                serialize_json_extract(f, func);
            }
            stmt::Expr::Project(project)
                if let stmt::Expr::Incoming(incoming) = project.base.as_ref() =>
            {
                let stmt::ExprIncoming::Table(table) = incoming else {
                    panic!("incoming projection was not lowered")
                };
                let [column] = project.projection.as_slice() else {
                    panic!("lowered incoming projection must reference one column")
                };
                let column = ColumnId {
                    table: *table,
                    index: *column,
                };
                match f.serializer.dialect {
                    Flavor::Postgresql => {
                        fmt!(f, "excluded." f.serializer.column_name(column));
                    }
                    // 达梦没有 `excluded`：`WHEN MATCHED` 分支里「本次要写进去的新值」只能
                    // 引用 `MERGE INTO … USING (SELECT … AS "s_<ordinal>" …) AS s` 里的源列。
                    // 那个 ordinal 是**插入列列表里的位置**（`dameng_merge_into` 建立映射，
                    // 渲染 `WHEN MATCHED` 分支期间挂在 formatter 上）。
                    Flavor::Dameng => {
                        let source = f.dameng_merge_source.as_ref().unwrap_or_else(|| {
                            panic!(
                                "dameng incoming projection must be rendered inside a MERGE INTO \
                                 matched clause; column={column:?}"
                            )
                        });
                        let ordinal = source
                            .iter()
                            .position(|candidate| candidate == &column)
                            .unwrap_or_else(|| {
                                panic!(
                                    "dameng MERGE source row has no column {column:?}; \
                                     columns={source:?}"
                                )
                            });
                        // 别名必须与 `statement.rs` 里的 `SourceAlias` **逐字符同形**：小写的
                        // `"s_<ordinal>"`。不能用 `Ident`——它把标识符折成大写（`"S_1"`），
                        // 与 `USING` 子查询里的别名对不上（golden 当场抓到过）。
                        f.dst.push_str("s.\"s_");
                        f.dst.push_str(&ordinal.to_string());
                        f.dst.push('"');
                    }
                    _ => panic!(
                        "incoming projections are only rendered for PostgreSQL (excluded.) \
                         and Dameng (MERGE source); column={column:?}"
                    ),
                }
            }
            stmt::Expr::Incoming(_) => panic!("incoming row must be projected"),
            stmt::Expr::IsSuperset(e) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, e.lhs.as_ref() " @> " e.rhs.as_ref()),
                // The rhs Value::List is bound as one JSON string. MySQL's
                // `JSON_CONTAINS(target, candidate)` matches when every
                // element of `candidate` appears in `target`.
                Flavor::Mysql | Flavor::MariaDb => {
                    fmt!(f, "JSON_CONTAINS(" e.lhs.as_ref() ", " e.rhs.as_ref() ")")
                }
                // SQLite has no direct superset operator; emulate via
                // `NOT EXISTS (rhs element with no match in lhs)`.
                Flavor::Sqlite => fmt!(
                    f,
                    "NOT EXISTS (SELECT 1 FROM json_each(" e.rhs.as_ref()
                    ") AS r WHERE r.value NOT IN (SELECT l.value FROM json_each("
                    e.lhs.as_ref() ") AS l))"
                ),
                // 达梦**有** MySQL 那一族的 JSON 判定函数（T8.12 真机实测，
                // `crates/toasty-driver-dameng/tests/engine_probe.rs` 第 17 节）：
                // `JSON_CONTAINS(target, candidate)` 语义与 MySQL 一致（candidate 的
                // **每个**元素都在 target 里 ⇒ 探针 17ap），第二个参数可以是绑定参数
                // （探针 17ak），`CLOB` 与 `VARCHAR(8188)` 两种列上都能用（探针 17ar）。
                // 与 MySQL 分支共用同一形状。
                Flavor::Dameng => {
                    fmt!(f, "JSON_CONTAINS(" e.lhs.as_ref() ", " e.rhs.as_ref() ")")
                }
            },
            stmt::Expr::Intersects(e) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, e.lhs.as_ref() " && " e.rhs.as_ref()),
                Flavor::Mysql | Flavor::MariaDb => {
                    fmt!(f, "JSON_OVERLAPS(" e.lhs.as_ref() ", " e.rhs.as_ref() ")")
                }
                Flavor::Sqlite => fmt!(
                    f,
                    "EXISTS (SELECT 1 FROM json_each(" e.rhs.as_ref()
                    ") AS r WHERE r.value IN (SELECT l.value FROM json_each("
                    e.lhs.as_ref() ") AS l))"
                ),
                // 同上：达梦的 `JSON_OVERLAPS` 与 MySQL 同形（真机探针 17t/17al）。
                Flavor::Dameng => {
                    fmt!(f, "JSON_OVERLAPS(" e.lhs.as_ref() ", " e.rhs.as_ref() ")")
                }
            },
            stmt::Expr::Length(e) => match f.serializer.dialect {
                Flavor::Postgresql => fmt!(f, "cardinality(" e.expr.as_ref() ")"),
                Flavor::Mysql | Flavor::MariaDb => fmt!(f, "JSON_LENGTH(" e.expr.as_ref() ")"),
                Flavor::Sqlite => fmt!(f, "json_array_length(" e.expr.as_ref() ")"),
                // 达梦的 `JSON_LENGTH` 真机可用（探针 17r，`["admin","verified"]` → 2）。
                Flavor::Dameng => fmt!(f, "JSON_LENGTH(" e.expr.as_ref() ")"),
            },
            stmt::Expr::Ident(name) => {
                fmt!(f, Ident(name));
            }
            stmt::Expr::InList(expr) => {
                fmt!(f, expr.expr " IN " expr.list);
            }
            stmt::Expr::AnyOp(expr) => match f.serializer.dialect {
                // `value = ANY(col)` — PostgreSQL's array membership operator.
                // Drives `Path::contains` for native-array columns and the
                // IN-list rewrite.
                Flavor::Postgresql => {
                    fmt!(f, expr.lhs " " expr.op " ANY(" expr.rhs ")");
                }
                // MySQL's `value MEMBER OF (json_array)` (8.0.17+). Only the
                // equality form makes sense; `Path::contains` is the only
                // current emitter and the lowering pass never produces
                // ANY on MySQL since `predicate_match_any` is false.
                Flavor::Mysql if matches!(expr.op, stmt::BinaryOp::Eq) => {
                    fmt!(f, expr.lhs " MEMBER OF (" expr.rhs ")");
                }
                Flavor::Mysql => unreachable!("AnyOp with non-Eq operator on MySQL: {expr:?}"),
                // MariaDB lacks MEMBER OF. JSON_ARRAY preserves the scalar's
                // JSON type and makes string membership case-sensitive.
                Flavor::MariaDb if matches!(expr.op, stmt::BinaryOp::Eq) => {
                    fmt!(f, "JSON_CONTAINS(" expr.rhs ", JSON_ARRAY(" expr.lhs "))");
                }
                Flavor::MariaDb => {
                    unreachable!("AnyOp with non-Eq operator on MariaDB: {expr:?}")
                }
                // SQLite renders `value = ANY(col)` (i.e. `Path::contains`)
                // as `value IN (SELECT value FROM json_each(col))`.
                Flavor::Sqlite if matches!(expr.op, stmt::BinaryOp::Eq) => {
                    fmt!(
                        f,
                        expr.lhs " IN (SELECT value FROM json_each(" expr.rhs "))"
                    );
                }
                Flavor::Sqlite => {
                    unreachable!("AnyOp with non-Eq operator on SQLite: {expr:?}")
                }
                // `Path::contains` 的达梦形状：达梦没有 `MEMBER OF`（探针 17m 语法错误），
                // 但 `JSON_CONTAINS(col, JSON_ARRAY(?))` 与 MariaDB 分支同义，且真机
                // 可用（探针 17am 命中、17an 大小写敏感 —— 官方套件
                // `vec_string_contains_filter` 明确要求「服务端默认排序规则不许渗进来」）。
                Flavor::Dameng if matches!(expr.op, stmt::BinaryOp::Eq) => {
                    fmt!(f, "JSON_CONTAINS(" expr.rhs ", JSON_ARRAY(" expr.lhs "))")
                }
                Flavor::Dameng => {
                    unreachable!("AnyOp with non-Eq operator on Dameng: {expr:?}")
                }
            },
            stmt::Expr::AllOp(expr) => {
                fmt!(f, expr.lhs " " expr.op " ALL(" expr.rhs ")");
            }
            stmt::Expr::InSubquery(expr) => {
                let op = if expr.negated { " NOT IN (" } else { " IN (" };
                fmt!(f, expr.expr op expr.query ")");
            }
            stmt::Expr::IsNull(expr) => {
                let op = if expr.negated {
                    " IS NOT NULL"
                } else {
                    " IS NULL"
                };
                fmt!(f, expr.expr op);
            }
            stmt::Expr::Like(expr) => {
                let op = if expr.case_insensitive
                    && matches!(f.serializer.dialect, Flavor::Postgresql)
                {
                    " ILIKE "
                } else {
                    " LIKE "
                };
                fmt!(f, expr.expr op expr.pattern);
                if let Some(escape) = expr.escape {
                    let escape = if f.serializer.is_mysql() && escape == '\\' {
                        stmt::Value::String("\\\\".to_string())
                    } else {
                        stmt::Value::String(escape.to_string())
                    };
                    let escape = &escape;
                    fmt!(f, " ESCAPE " escape);
                }
            }
            stmt::Expr::StartsWith(expr) => {
                match f.serializer.dialect {
                    // PostgreSQL's `^@` prefix-match operator; prefix is bound
                    // as a plain string parameter.
                    Flavor::Postgresql => {
                        fmt!(f, expr.expr " ^@ " expr.prefix);
                    }
                    // SQLite GLOB is case-sensitive.  extract_params has already
                    // escaped GLOB metacharacters and appended `*` to the prefix
                    // parameter, so we only need to emit the right operator.
                    Flavor::Sqlite => {
                        fmt!(f, expr.expr " GLOB " expr.prefix);
                    }
                    // MySQL LIKE is case-insensitive by default; casting the
                    // column side to BINARY forces a case-sensitive byte
                    // comparison.  extract_params has escaped `%`/`_`/`!` and
                    // appended `%` to the prefix parameter.
                    Flavor::Mysql | Flavor::MariaDb => {
                        fmt!(f, "BINARY " expr.expr " LIKE " expr.prefix " ESCAPE '!'");
                    }
                    // 达梦没有 `^@`/`GLOB`/`BINARY … LIKE` 这类前缀算子，只有 `LIKE`。
                    // 引擎侧靠 `binary_like_starts_with = true` 把前缀转义
                    // （`%`/`_`/`!` → `!x`）并追加 `%`，所以这里只差 `LIKE` 本身；
                    // **不要**渲染 `BINARY`（达梦不认这个转换）。
                    Flavor::Dameng => {
                        fmt!(f, expr.expr " LIKE " expr.prefix " ESCAPE '!'");
                    }
                }
            }
            stmt::Expr::Not(expr) => {
                // `Predicate`：达梦下 `NOT <值表达式>` 也要补 ` = 1`（见其文档）。
                fmt!(f, "NOT (" Predicate(&expr.expr) ")");
            }
            stmt::Expr::Or(expr) => {
                fmt!(f, Delimited(expr.operands.iter().map(Predicate), " OR "));
            }
            stmt::Expr::Record(expr) => {
                let fields = Comma(expr.fields.iter());
                fmt!(f, "(" fields ")");
            }
            stmt::Expr::Reference(expr_reference @ stmt::ExprReference::Column(expr_column)) => {
                if f.alias {
                    let depth = f.depth - expr_column.nesting;

                    match f.cx.resolve_expr_reference(expr_reference) {
                        ResolvedRef::Column(column) => {
                            let name = Ident(&column.name);
                            fmt!(f, "tbl_" depth "_" expr_column.table "." name)
                        }
                        ResolvedRef::Cte { .. } | ResolvedRef::Derived(_) => {
                            fmt!(f, "tbl_" depth "_" expr_column.table "." ColumnAlias(expr_column.column))
                        }
                        ResolvedRef::Model(model) => {
                            panic!("Model references cannot be serialized to SQL; model={model:?}")
                        }
                        ResolvedRef::Field(field) => {
                            panic!("Field references cannot be serialized to SQL; field={field:?}")
                        }
                    }
                } else {
                    let column =
                        f.cx.resolve_expr_reference(expr_reference)
                            .as_column_unwrap();
                    if matches!(f.serializer.dialect, Flavor::Postgresql)
                        && expr_column.nesting == 0
                        && f.assignment_table == Some(column.id.table)
                    {
                        fmt!(f, f.serializer.table_name(column.id.table) "." Ident(&column.name))
                    } else {
                        fmt!(f, Ident(&column.name))
                    }
                }
            }
            stmt::Expr::Stmt(expr) => {
                let stmt = &*expr.stmt;
                fmt!(f, "(" stmt ")");
            }
            stmt::Expr::List(expr) => {
                let items = Comma(expr.items.iter());
                fmt!(f, "(" items ")");
            }
            stmt::Expr::Value(expr) => expr.to_sql(f),
            // Schema-fixed leaf rendered inline as a SQL literal.  Reuses
            // the same `Value::to_sql` path the inline DDL serializer uses,
            // which escapes `String` defensively.
            stmt::Expr::Static(expr) => expr.to_sql(f),
            stmt::Expr::Arg(arg) => {
                // Pre-extracted bind parameter placeholder — render as a
                // positional parameter. The arg position is 0-based; the
                // placeholder is 1-based.
                f.arg_positions.push(arg.position);
                let placeholder = super::Placeholder(arg.position + 1);
                fmt!(f, placeholder);
            }
            stmt::Expr::Default => match f.serializer.dialect {
                Flavor::Postgresql | Flavor::Mysql | Flavor::MariaDb | Flavor::Dameng => {
                    fmt!(f, "DEFAULT")
                }
                // SQLite does not support the DEFAULT keyword but NULL acts similarly.
                Flavor::Sqlite => fmt!(f, "NULL"),
            },
            _ => todo!("expr={:#?}", self),
        }
    }
}

/// A single operand of an `AND` chain.
///
/// `OR` binds looser than `AND` in SQL, so an `Or` operand must be
/// parenthesized: `a AND (b OR c)` would otherwise serialize as
/// `a AND b OR c`, which parses as `(a AND b) OR c` and silently changes the
/// query's meaning. Operands of other kinds bind at least as tightly as `AND`
/// (comparisons, `IS NULL`, `NOT (..)`, nested `AND`), so they need no parens.
struct AndOperand<'a>(&'a stmt::Expr);

impl ToSql for AndOperand<'_> {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        if matches!(self.0, stmt::Expr::Or(_)) {
            fmt!(f, "(" self.0 ")");
        } else {
            // 达梦：`AND` 的操作数也是布尔条件位置，值表达式要补 ` = 1`
            // （`WHERE <布尔列> AND …` 真机报 `[HY000/-4104]`）。
            fmt!(f, Predicate(self.0));
        }
    }
}

/// Serializes a document path extraction per dialect: `json_extract(col,
/// '$.a.b')` on SQLite, `CAST(JSON_UNQUOTE(JSON_EXTRACT(col, '$.a.b')) AS ...)`
/// on MySQL, and `(col->'a'->>'b')::cast` on PostgreSQL — the latter two unwrap
/// the leaf to text and cast it to match the bound parameter's type.
fn serialize_json_extract(f: &mut super::Formatter<'_>, func: &stmt::FuncJsonExtract) {
    match f.serializer.dialect {
        Flavor::Sqlite => {
            // SQLite's `json_extract` returns SQL-native scalars (unquoted text,
            // integers, reals), so a path read compares directly against a bound
            // parameter with no cast. The path is a single-quoted JSONPath like
            // `$.a.b`.
            fmt!(
                f,
                "json_extract(" func.base.as_ref() ", '$"
                Delimited(func.path.iter().map(|key| (".", key.as_str())), "")
                "')"
            );
        }
        Flavor::Mysql | Flavor::MariaDb => serialize_mysql_json_extract(f, func),
        Flavor::Dameng => serialize_dameng_json_extract(f, func),
        Flavor::Postgresql => {
            // Descend with `->`, take the leaf as text with `->>`, then cast the
            // text to the leaf type so it compares against a bound parameter.
            let (leaf, parents) = func
                .path
                .split_last()
                .expect("json extract path has at least one key");
            fmt!(
                f,
                "(" func.base.as_ref()
                Delimited(parents.iter().map(|key| ("->'", key.as_str(), "'")), "")
                "->>'" leaf.as_str() "')"
                pg_json_cast(&func.ty).map(|cast| ("::", cast))
            );
        }
    }
}

/// Serializes a Dameng document path read.
///
/// Dameng has no JSON column type: `#[document]` values live in a `CLOB` as text
/// (the driver binds them through [`toasty_sql::json`]), and the leaf comes back
/// with `JSON_VALUE(col, '$.a.b')`, which yields a **SQL scalar as text** — a
/// JSON string arrives unquoted, a number or boolean as its bare literal. So
/// each leaf type needs the conversion that makes it compare against the bound
/// parameter; all of the choices below are measured on the real server (probe
/// sections 25/26/27, see `docs/toasty-fork.md` item 25):
///
/// * integers: a bare text leaf never equals a bound `BIGINT` (`25v` only
///   matched with an explicit cast) -> `CAST(... AS BIGINT)`;
/// * floats: `CAST(... AS DOUBLE)`;
/// * booleans: `CAST('true' AS BIT/INT)` fails with `[22018/-6111]`, but
///   `CASE WHEN ... = 'true' THEN 1 ELSE 0 END` equals a bound bool parameter
///   and works in filters as well as projections (`26c`);
/// * UUIDs: the parameter is bound as its 16 bytes (`default_uuid_type = Blob`),
///   so strip the hyphens from the canonical text and `HEXTORAW` it back
///   (`27h`; the other direction, `RAWTOHEX(?)`, does not match — `26i2`);
/// * temporal: the JSON codec writes ISO 8601 (`2024-01-02T03:04:05.123456Z`), which
///   Dameng's `CAST(… AS TIMESTAMP(6))` rejects with `[22007/-6118]` -> swap the `T`
///   for a space and drop the trailing `Z` first, which yields Dameng's own
///   `YYYY-MM-DD HH24:MI:SS.FF6` and compares microsecond-exactly against a bound
///   parameter (probes 28/29; the *parameter* side also has to report ODBC precision 6
///   or Dameng truncates it to whole seconds);
/// * strings, network addresses, `Decimal`/`BigDecimal` and `Zoned`: the bare
///   text already compares correctly (`25t`/`25u` for strings, `26j`/`26k`/`27f`
///   for the numerics, which Dameng compares numerically).
fn serialize_dameng_json_extract(f: &mut super::Formatter<'_>, func: &stmt::FuncJsonExtract) {
    use crate::stmt::Type;

    match &func.ty {
        Type::Bool => {
            fmt!(f, "CASE WHEN ");
            dameng_json_value(f, func);
            fmt!(f, " = 'true' THEN 1 ELSE 0 END");
        }
        Type::Uuid => {
            fmt!(f, "HEXTORAW(REPLACE(");
            dameng_json_value(f, func);
            fmt!(f, ", '-', ''))");
        }
        Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::U8
        | Type::U16
        | Type::U32
        | Type::U64 => {
            fmt!(f, "CAST(");
            dameng_json_value(f, func);
            fmt!(f, " AS BIGINT)");
        }
        Type::F32 | Type::F64 => {
            fmt!(f, "CAST(");
            dameng_json_value(f, func);
            fmt!(f, " AS DOUBLE)");
        }
        #[cfg(feature = "jiff")]
        Type::Timestamp | Type::DateTime => {
            // 文档里存的是 `toasty_sql::json` 的 ISO 8601 存储文本 ——
            // `2024-01-02T03:04:05.123456Z`（`Timestamp` 带尾 `Z`、`DateTime` 不带，
            // 两者都是固定 6 位小数秒，见 `Value::document_storage_text`）。达梦的
            // `CAST(… AS TIMESTAMP(6))` **不认这个形态**：真机回 `[22007/-6118]
            // 非法的时间日期类型数据`（官方套件
            // `type_document::struct_embed_filter_temporal`）。把 `T` 换成空格、抹掉
            // 尾 `Z` 之后正好是达梦自己的 `YYYY-MM-DD HH24:MI:SS.FF6`；第 28/29 节
            // 探针钉住了「这样比与绑定参数逐微秒相等」。
            fmt!(f, "CAST(REPLACE(REPLACE(");
            dameng_json_value(f, func);
            fmt!(f, ", 'T', ' '), 'Z', '') AS TIMESTAMP(6))");
        }
        #[cfg(feature = "jiff")]
        Type::Date => {
            fmt!(f, "CAST(");
            dameng_json_value(f, func);
            fmt!(f, " AS DATE)");
        }
        #[cfg(feature = "jiff")]
        Type::Time => {
            fmt!(f, "CAST(");
            dameng_json_value(f, func);
            fmt!(f, " AS TIME(6))");
        }
        _ => dameng_json_value(f, func),
    }
}

/// The bare `JSON_VALUE(col, '$.a.b')` every Dameng path read is built on. The
/// path is the same single-quoted `$.a.b` JSONPath the other dialects use.
fn dameng_json_value(f: &mut super::Formatter<'_>, func: &stmt::FuncJsonExtract) {
    fmt!(
        f,
        "JSON_VALUE(" func.base.as_ref() ", '$"
        Delimited(func.path.iter().map(|key| (".", key.as_str())), "")
        "')"
    );
}

/// Serializes a MySQL document path read. `JSON_EXTRACT` yields a *JSON-typed*
/// value, which compares against a bound SQL parameter only by luck — a JSON
/// string (e.g. an ISO timestamp) never equals a native `DATETIME`, and JSON
/// string comparison is `utf8mb4_bin` (case-sensitive), unlike a `VARCHAR`
/// column. So unwrap the leaf to text with `JSON_UNQUOTE` and `CAST` it to the
/// leaf's SQL type (`CHAR` for strings, to recover a `VARCHAR`-matching
/// collation — see [`mysql_json_cast`]), mirroring the `->>`-plus-cast
/// PostgreSQL path. Booleans are the exception: the unquoted text
/// `'true'`/`'false'` casts to `0`, so cast the *bare* JSON boolean to
/// `UNSIGNED` instead (`true` -> 1, `false` -> 0), matching a bound bool param.
fn serialize_mysql_json_extract(f: &mut super::Formatter<'_>, func: &stmt::FuncJsonExtract) {
    if matches!(func.ty, stmt::Type::Bool) {
        fmt!(f, "CAST(");
        mysql_json_extract(f, func);
        fmt!(f, " AS UNSIGNED)");
    } else if let Some(cast) = mysql_json_cast(&func.ty) {
        fmt!(f, "CAST(JSON_UNQUOTE(");
        mysql_json_extract(f, func);
        fmt!(f, ") AS " cast ")");
    } else {
        fmt!(f, "JSON_UNQUOTE(");
        mysql_json_extract(f, func);
        fmt!(f, ")");
    }
}

/// Emits the bare `JSON_EXTRACT(col, '$.a.b')` every MySQL path read is built
/// on, with the path as a single-quoted JSONPath argument.
fn mysql_json_extract(f: &mut super::Formatter<'_>, func: &stmt::FuncJsonExtract) {
    fmt!(
        f,
        "JSON_EXTRACT(" func.base.as_ref() ", '$"
        Delimited(func.path.iter().map(|key| (".", key.as_str())), "")
        "')"
    );
}

/// The MySQL `CAST(... AS <type>)` target wrapped around a
/// `JSON_UNQUOTE(JSON_EXTRACT(...))` text extraction so it compares against a
/// bound parameter of the leaf type. Mirrors [`pg_json_cast`]; `None` leaves the
/// extraction as bare unquoted text (only floats, which compare via numeric
/// coercion). `Bool` is absent because it is cast separately — see
/// [`serialize_mysql_json_extract`].
///
/// `String`/`Uuid` cast to `CHAR` not for the type but for the *collation*:
/// `JSON_UNQUOTE` yields `utf8mb4_bin` (case-sensitive), while `CAST(... AS
/// CHAR)` adopts the connection's default collation — the same one a bound
/// literal and a `VARCHAR` column use — so a string filter on a document leaf
/// matches the case sensitivity of a plain column. `AS CHAR` (no length) does
/// not truncate, and inheriting the server default keeps it portable across
/// server collation configs rather than hardcoding a collation name.
///
/// The temporal targets carry `(6)` precision: a bare `CAST(... AS DATETIME)`
/// truncates to whole seconds, which would drop the microseconds the JSON codec
/// writes and break an equality filter on a sub-second value.
fn mysql_json_cast(ty: &stmt::Type) -> Option<&'static str> {
    use crate::stmt::Type;
    Some(match ty {
        Type::String | Type::Uuid => "CHAR",
        Type::I8 | Type::I16 | Type::I32 | Type::I64 => "SIGNED",
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => "UNSIGNED",
        #[cfg(feature = "rust_decimal")]
        Type::Decimal => "DECIMAL(65, 30)",
        #[cfg(feature = "bigdecimal")]
        Type::BigDecimal => "DECIMAL(65, 30)",
        #[cfg(feature = "jiff")]
        Type::Timestamp => "DATETIME(6)",
        #[cfg(feature = "jiff")]
        Type::Date => "DATE",
        #[cfg(feature = "jiff")]
        Type::Time => "TIME(6)",
        #[cfg(feature = "jiff")]
        Type::DateTime => "DATETIME(6)",
        _ => return None,
    })
}

/// The PostgreSQL cast applied to a `->>'` text extraction so it compares
/// against a bound parameter of the leaf type. `String` (and any non-scalar
/// leaf) needs no cast — `->>` already yields text.
///
/// Every scalar a `#[document]` leaf can hold must appear here: an unlisted
/// scalar falls through to `None` and renders as an *uncast* text extraction,
/// which PostgreSQL then refuses to compare against a typed parameter
/// (`operator does not exist: text = ...`). The temporal casts pair with the
/// microsecond-truncated text the JSON codec writes (see `toasty_sql::json`),
/// so the extracted value parses cleanly into the SQL temporal type. Network
/// addresses likewise cast from their canonical text forms. `Zoned` is
/// intentionally absent: it is rejected at schema-build because jiff renders
/// it with an RFC 9557 `[IANA]` annotation that no PostgreSQL cast can parse.
fn pg_json_cast(ty: &stmt::Type) -> Option<&'static str> {
    use crate::stmt::Type;
    Some(match ty {
        Type::Bool => "boolean",
        Type::I8 | Type::I16 | Type::I32 | Type::I64 => "bigint",
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => "bigint",
        Type::F32 | Type::F64 => "double precision",
        Type::Uuid => "uuid",
        #[cfg(feature = "rust_decimal")]
        Type::Decimal => "numeric",
        #[cfg(feature = "bigdecimal")]
        Type::BigDecimal => "numeric",
        #[cfg(feature = "jiff")]
        Type::Timestamp => "timestamptz",
        #[cfg(feature = "jiff")]
        Type::Date => "date",
        #[cfg(feature = "jiff")]
        Type::Time => "time",
        #[cfg(feature = "jiff")]
        Type::DateTime => "timestamp",
        #[cfg(feature = "net")]
        Type::Cidr => "cidr",
        #[cfg(feature = "net")]
        Type::Inet => "inet",
        #[cfg(feature = "net")]
        Type::MacAddr => "macaddr",
        #[cfg(feature = "net")]
        Type::MacAddr8 => "macaddr8",
        _ => return None,
    })
}

impl ToSql for &stmt::BinaryOp {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        f.dst.push_str(match self {
            stmt::BinaryOp::Eq => "=",
            stmt::BinaryOp::Gt => ">",
            stmt::BinaryOp::Ge => ">=",
            stmt::BinaryOp::Lt => "<",
            stmt::BinaryOp::Le => "<=",
            stmt::BinaryOp::Ne => "<>",
            stmt::BinaryOp::Add => "+",
            stmt::BinaryOp::Sub => "-",
        })
    }
}
