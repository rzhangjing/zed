//! 结果集：`RowSet` 与 `BatchOutcome`（设计方案 §6.12 冻结项）。
//!
//! # `Affected` 与「零行的结果集」怎么区分
//!
//! 两者都可能是「零行」，所以行数**不是**判据。判据是**列**：
//! `SELECT`（哪怕一行都没有）一定有列，DML/DDL 一定没有列。
//! 于是 `columns().is_empty()` 为真 ⟺ 这条语句走的是 `Affected` 分支。
//!
//! 这个区分必须存在，否则下列两件事无法分辨：
//!
//! - `DELETE FROM t WHERE 1=0`（影响了 0 行，`affected == Some(0)`）；
//! - `SELECT … WHERE 1=0`（返回 0 行，`affected == None`、`columns` 非空）。
//!
//! # 为什么 `affected` 是 `Option<u64>`
//!
//! `None` = **驱动没说**（`ExecOutcome::Affected { rows: None }`），不是「零行」。
//! 把它压成 `0` 会让调用方无法区分「真的没影响任何行」与「驱动没报」——
//! `dm-core::ExecOutcome` 用 `Option` 表达这个区别，这里照搬，不在中间层丢掉。

use std::sync::Arc;

use dm_core::ColumnMeta;

use crate::row::Row;

/// 一次查询的完整结果。
///
/// 全量物化（`RowStream` 才是流式的）。物化的代价是内存，换来的是
/// 「结果集用完即走」——连接不会被结果集吊着，`PoolGuard` 能立刻归还。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowSet {
    columns: Arc<[ColumnMeta]>,
    rows: Vec<Row>,
    affected: Option<u64>,
}

impl RowSet {
    /// 结果集分支。
    #[must_use]
    pub(crate) fn result_set(columns: Arc<[ColumnMeta]>, rows: Vec<Row>) -> Self {
        Self {
            columns,
            rows,
            affected: None,
        }
    }

    /// 影响行数分支（`rows == None` = 驱动没说）。
    #[must_use]
    pub(crate) fn affected_only(affected: Option<u64>) -> Self {
        Self {
            columns: Arc::from(Vec::<ColumnMeta>::new()),
            rows: Vec::new(),
            affected,
        }
    }

    /// 列元数据。
    ///
    /// **空 = 这条语句没有结果集**（见模块文档的判据）。
    #[must_use]
    pub fn columns(&self) -> &[ColumnMeta] {
        &self.columns
    }

    /// 全部行。
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// 取走全部行（想长期持有结果时用它，省一次拷贝）。
    #[must_use]
    pub fn into_rows(self) -> Vec<Row> {
        self.rows
    }

    /// 第一行（`query_one` 这类便捷用法的基础）。
    #[must_use]
    pub fn first(&self) -> Option<&Row> {
        self.rows.first()
    }

    /// 影响行数；`None` = 没有影响行数信息（结果集，或驱动没报）。
    #[must_use]
    pub const fn affected(&self) -> Option<u64> {
        self.affected
    }

    /// 行数（`Affected` 分支恒为 0）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// 是否没有行。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// 批量执行的账：**部分成功不当作整体失败**。
///
/// `execute_many` 的语义是「能跑多少跑多少」——调用方要的是「跑到第几条出的问题，
/// 前面几条生效了没有」，而不是一个把全部信息抹平的 `Err`。所以失败位置在返回值里，
/// 只有**建语句/绑定层的致命错误**（连第一条都没跑成）才返回 `Err`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchOutcome {
    /// 累计受影响行数（驱动没报的那些按 0 计）。
    pub affected: u64,
    /// 实际执行成功的条数。
    pub executed: usize,
    /// 第一条失败的**参数下标**；`None` = 全部成功。
    pub failed_at: Option<usize>,
}

impl BatchOutcome {
    /// 是否全部成功。
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.failed_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dm_core::{DataType, DmValue};

    fn columns(names: &[&str]) -> Arc<[ColumnMeta]> {
        names
            .iter()
            .map(|n| ColumnMeta {
                name: (*n).to_owned(),
                data_type: DataType::VarChar,
                nullable: true,
                precision: None,
                scale: None,
                column_size: None,
                length_in_char: None,
            })
            .collect::<Vec<_>>()
            .into()
    }

    #[test]
    fn result_set_with_zero_rows_is_not_affected() {
        let rs = RowSet::result_set(columns(&["ID"]), Vec::new());
        assert!(rs.is_empty());
        assert_eq!(rs.affected(), None);
        assert!(!rs.columns().is_empty(), "有列 ⇒ 它是结果集，不是 DML");
        assert!(rs.first().is_none());
    }

    #[test]
    fn affected_only_has_no_columns() {
        let rs = RowSet::affected_only(Some(3));
        assert_eq!(rs.affected(), Some(3));
        assert!(rs.columns().is_empty(), "没有列 ⇒ 它是 DML/DDL");
        assert_eq!(rs.len(), 0);
    }

    #[test]
    fn affected_unknown_is_none_not_zero() {
        let rs = RowSet::affected_only(None);
        assert_eq!(rs.affected(), None);
        assert!(rs.columns().is_empty());
    }

    #[test]
    fn rows_are_addressable() {
        let row = Row::new(vec![DmValue::Int(1)], columns(&["ID"]));
        let rs = RowSet::result_set(columns(&["ID"]), vec![row.clone()]);
        assert_eq!(rs.first(), Some(&row));
        assert_eq!(rs.into_rows().len(), 1);
    }

    #[test]
    fn batch_outcome_reports_partial_success() {
        let ok = BatchOutcome {
            affected: 5,
            executed: 5,
            failed_at: None,
        };
        assert!(ok.is_complete());
        let partial = BatchOutcome {
            affected: 2,
            executed: 2,
            failed_at: Some(2),
        };
        assert!(!partial.is_complete());
        assert_eq!(partial.failed_at, Some(2));
    }
}
