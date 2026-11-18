use super::{Flavor, Ident, Period, ToSql, dameng_table_name};

use crate::stmt;

impl ToSql for &stmt::Name {
    fn to_sql(self, f: &mut super::Formatter<'_>) {
        match f.serializer.dialect {
            // 达梦：`Name` 承载的正是「可能带模式前缀的对象名」（表名、索引名…），
            // 而达梦的模式是大小写敏感对象（`SET SCHEMA dbtest_lower` 会被折成
            // `DBTEST_LOWER` 而报 `-2103`）。所以整串交给 `dameng_table_name`：
            // 模式段逐字保留、其余折大写，规则与 DML/DDL 里其余表名一致。
            // 顺带覆盖「单段但名字里带点」的写法（`Name(vec!["DBTESTA.T"])`），
            // 不会渲染成一个带点的标识符。
            Flavor::Dameng => f.dst.push_str(&dameng_table_name(&self.0.join("."))),
            _ => {
                let parts = Period(self.0.iter().map(Ident));
                fmt!(f, parts);
            }
        }
    }
}
