use std::fmt;

/// A possibly schema-qualified SQL name (e.g. `"users"` or `"public"."users"`).
///
/// Stores each segment as a separate string. Single-segment names are the
/// common case.
///
/// # Example
///
/// ```
/// use toasty_sql::stmt::Name;
///
/// let name = Name::from("users");
/// assert_eq!(name.to_string(), "users");
/// ```
#[derive(Debug, Clone)]
pub struct Name(pub Vec<String>);

impl From<&str> for Name {
    fn from(value: &str) -> Self {
        Self(vec![value.into()])
    }
}

impl From<&String> for Name {
    fn from(value: &String) -> Self {
        Self::from(&value[..])
    }
}

impl Name {
    /// 表名 → 分段名：`DBTESTA.T_ORDER` → `["DBTESTA", "T_ORDER"]`。
    ///
    /// 模型侧 `#[table = "DBTESTA.T_ORDER"]` 会把整串塞进 `db::Table::name`。达梦方言
    /// 只有拆成两段才渲染成 `"DBTESTA"."T_ORDER"`；整串一段会渲染成
    /// `"DBTESTA.T_ORDER"`——那是一个「名字里带点」的表，实际不存在（`-2106`）。
    ///
    /// 只拆第一个 `.`：达梦没有三级对象名。某一侧为空（`DBTESTA.` / `.t`）时保持单段，
    /// 免得渲染出 `""."T"` 这种非法语句。
    #[must_use]
    pub fn from_table_name(name: &str) -> Self {
        match name.split_once('.') {
            Some((schema, table)) if !schema.trim().is_empty() && !table.trim().is_empty() => {
                Self(vec![schema.trim().to_owned(), table.trim().to_owned()])
            }
            _ => Self(vec![name.trim().to_owned()]),
        }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = "";
        for ident in &self.0 {
            write!(f, "{s}{ident}")?;
            s = ", ";
        }

        Ok(())
    }
}
