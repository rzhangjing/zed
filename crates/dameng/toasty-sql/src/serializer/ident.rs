use super::{Flavor, Formatter, ToSql};

pub(super) struct Ident<S>(pub(super) S);

impl<S: AsRef<str>> ToSql for Ident<S> {
    fn to_sql(self, f: &mut Formatter<'_>) {
        match f.serializer.dialect {
            Flavor::Mysql | Flavor::MariaDb => {
                f.dst.push('`');
                f.dst.push_str(self.0.as_ref());
                f.dst.push('`');
            }
            // 达梦默认（Oracle 兼容）用 ANSI 双引号标识符。
            //
            // ⚠️ 达梦还把**未加引号**的标识符折叠成**大写**（真机实测：不带引号建的
            // 表在 `USER_TABLES` 里叫 `DM_ENGINE_PROBE2`，而 `SELECT … FROM
            // "dm_engine_probe2"` 报 `[42S02/-2106] 无效的表或视图名`）。用户写在
            // `RawSql` 里的 SQL（官方套件的 `raw_sql` 簇）正是不带引号的 => 我们生成
            // 的所有标识符也必须折成大写，否则「引擎建的表」与「用户手写的表名」在
            // 同一条语句里对不上。PG/SQLite 折成小写、MySQL 不折叠，所以只有达梦要
            // 这一步。
            Flavor::Dameng => {
                f.dst.push('"');
                f.dst.push_str(&self.0.as_ref().to_uppercase());
                f.dst.push('"');
            }
            Flavor::Postgresql | Flavor::Sqlite => {
                f.dst.push('"');
                f.dst.push_str(self.0.as_ref());
                f.dst.push('"');
            }
        }
    }
}

/// 表名标识符。
///
/// 与 [`Ident`] 只有一处差别，且只在达梦方言下生效：表名允许写成「模式名.表名」
/// 形态（模型侧就是 `#[table = "DBTESTA.T_ORDER"]`），这时渲染成**两个**标识符
/// `DBTESTA.T_ORDER`，而不是把整串当一个标识符渲染成 `"DBTESTA.T_ORDER"`
/// （那是另一个对象名，达梦报 `-2106 无效的表或视图名`，Round 36 真机实测）。
///
/// 拆分后的渲染规则（真机证据：`crates/dm-odbc/examples/qualified_probe.rs`，
/// 日志 `target/m36_qualified_probe.log`；结论见 `docs/测试报告-dbtesta-dbtestb.md` §13）：
///
/// - **两段都折大写**，且**优先不带引号**（`DBTESTA.T_ORDER`）。这就是用户手写
///   `S.T` 的形态；达梦把未加引号的标识符折成大写，所以 `dbtesta.t_order` 这种
///   小写输入也能落到 `DBTESTA.T_ORDER` 上（实测：`S.T` 与 `"S"."T"` 对同一批大写
///   对象等价，而 `"S"."T"` 一旦大小写写错就报 `-2106` / `-2103`）。
/// - 只有「裸写不安全」时才加双引号：标识符里有非 `[A-Z0-9_$#]` 字符、以数字开头、
///   或撞上达梦保留字（`ORDER` / `TABLE` …，见 [`DAMENG_RESERVED`]）。
/// - 没有 `.`、或 `.` 某一侧为空 ⇒ 维持改动前的行为（**整串折大写并加引号**），
///   不会渲染出 `""."T"` 这种非法语句。
/// - 其它方言完全保持 [`Ident`] 的行为（不拆分）：本 fork 只服务达梦，不为别的方言
///   引入语义猜测。
pub(super) struct TableIdent<S>(pub(super) S);

impl<S: AsRef<str>> ToSql for TableIdent<S> {
    fn to_sql(self, f: &mut Formatter<'_>) {
        match f.serializer.dialect {
            Flavor::Dameng => f.dst.push_str(&dameng_table_name(self.0.as_ref())),
            _ => Ident(self.0).to_sql(f),
        }
    }
}

/// 把表名渲染成达梦可用的形态：`DBTESTA.T_ORDER` → `DBTESTA.T_ORDER`，
/// `dbtesta.t_order` → `DBTESTA.T_ORDER`，`my schema.t` → `"MY SCHEMA".T`。
///
/// 规则与 [`TableIdent`] 完全一致。单独暴露是因为驱动里还有一处「手写 SQL 片段」
/// 需要同样的渲染：`SET IDENTITY_INSERT <表> ON/OFF`（`toasty-driver-dameng/src/conn.rs`）。
#[must_use]
pub fn dameng_table_name(name: &str) -> String {
    match name.split_once('.') {
        // 只认一段模式前缀；`a.b.c` 会被当成 `a` + `B.C`（达梦没有三级对象名）。
        Some((schema, table)) if !schema.trim().is_empty() && !table.trim().is_empty() => {
            format!(
                "{}.{}",
                render_qualified_part(schema),
                render_qualified_part(table)
            )
        }
        // 不带前缀：与改动前完全一致（整串折大写 + 双引号），既有 golden 不受影响。
        _ => quote_identifier(&name.trim().to_uppercase()),
    }
}

/// 限定名里的一段：折大写；能裸写就裸写，否则加引号。
fn render_qualified_part(raw: &str) -> String {
    let folded = raw.trim().to_uppercase();
    if is_bare_identifier(&folded) {
        folded
    } else {
        quote_identifier(&folded)
    }
}

/// 能不能不加引号直接写：ASCII 字母/下划线开头，其余只含字母数字与 `_ $ #`，
/// 且不是达梦保留字。
fn is_bare_identifier(folded: &str) -> bool {
    let mut chars = folded.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '#') {
        return false;
    }
    !DAMENG_RESERVED.contains(&folded)
}

/// 加 ANSI 双引号；内部的双引号按 SQL 规矩双写。
fn quote_identifier(inner: &str) -> String {
    format!("\"{}\"", inner.replace('"', "\"\""))
}

/// 达梦（Oracle 兼容）保留字里**最可能被当成表名/模式名**的那些。
///
/// 不追求完整清单（完整清单几百个，且随版本变）：这里只挡住「模型里真会有」的常见词，
/// 挡住的方式就是回到加引号的写法——引号里的保留字在任何 SQL 方言里都合法。
const DAMENG_RESERVED: &[&str] = &[
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "BETWEEN",
    "BY",
    "CASE",
    "CHECK",
    "COLUMN",
    "CONNECT",
    "CREATE",
    "CURRENT",
    "DATE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "EXISTS",
    "FOR",
    "FOREIGN",
    "FROM",
    "GRANT",
    "GROUP",
    "HAVING",
    "IN",
    "INDEX",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIKE",
    "NOT",
    "NULL",
    "ON",
    "OR",
    "ORDER",
    "PRIMARY",
    "REFERENCES",
    "RIGHT",
    "ROW",
    "ROWNUM",
    "SELECT",
    "SET",
    "SIZE",
    "TABLE",
    "THEN",
    "TIME",
    "TIMESTAMP",
    "TO",
    "TRIGGER",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USER",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
];
