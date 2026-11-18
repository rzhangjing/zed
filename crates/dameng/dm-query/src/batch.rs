//! 批量写的参数整理（T6.4，设计方案 §4.7 / §6.7 / §6.12 第 7 条）。
//!
//! 这一模块只做一件事：把调用方给的「一组一组参数」（行优先）**转成列式**
//! （每个 `?` 一列、每列 `N` 个元素），并在碰 ODBC 之前把形状校验做掉。
//!
//! # 为什么转置放在这里，而不是后端
//!
//! ODBC 的数组绑定本来就是列式布局：第 `p` 个参数的缓冲是一段连续的「`N` 个同宽
//! 元素」，`BufferLength` 是元素**步长**。后端如果收行优先的输入，就得自己再分配
//! 一份转置副本——那份副本的寿命必须覆盖整次 `SQLExecute`（驱动存的是裸指针），
//! 于是后端要额外管一块内存，而调用方本来就要把参数收集成 `Vec`。转置放在这里，
//! 后端拿到的就是要绑的东西本身。
//!
//! # 形状的三条硬约束（都在这里拦）
//!
//! 1. **每组的参数个数必须一致**。不一致的输入没法转置（第 `p` 列会缺元素）。
//!    这一条**必然**要拦：不拦的话「第 3 组少了 1 个参数」会变成「第 3 行的第 2 列
//!    用了第 4 组的值」这种静默错位。§6.7 的批量契约里没有「补 NULL」这种默认。
//! 2. **列数必须等于语句的 `?` 个数**。0 个 `?` 与 `N` 组的组合（「把同一条语句
//!    打 N 遍」）也不走列式绑定，由 `conn.rs` 直接循环执行——列式绑定在这里的
//!    宽度是 0，硬套上去就得让驱动去支持「零参数数组」。
//! 3. **空批次（0 组）不执行任何语句**，返回零影响、零执行。调用方给空集合是合法
//!    用法（`WHERE id IN (…)` 先收集后批量），报错会把「恰好没数据」变成故障。
//!
//! 前两条报 [`DmErrorKind::Config`]（调用方用法错误，重试不会好），第三条正常返回。

use dm_core::DmValue;
use dm_error::{DmError, DmErrorKind, Stage};

/// 转置后的列式参数：`columns[p]` 是第 `p` 个 `?` 在全部 `rows` 组里的取值。
///
/// 私有类型：`dm-query` 对外的批量契约是
/// [`BatchOutcome`](crate::rowset::BatchOutcome)，列式只是「怎么把参数交给后端」的
/// 内部形状。公开它会让「后端换了绑定方式」变成一次破坏性变更。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ColumnBatch {
    columns: Vec<Vec<DmValue>>,
    rows: usize,
}

impl ColumnBatch {
    /// 行（参数组）数。
    pub(crate) fn rows(&self) -> usize {
        self.rows
    }

    /// 参数（列）个数，等于语句的 `?` 个数。
    pub(crate) fn width(&self) -> usize {
        self.columns.len()
    }

    /// 是否一组参数都没有（[`BatchOutcome`](crate::rowset::BatchOutcome) 该是零）。
    pub(crate) fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// 交给 [`dm_core::Backend::execute_param_array`] 的列式视图。
    pub(crate) fn as_columns(&self) -> &[Vec<DmValue>] {
        &self.columns
    }
}

/// 把行优先的参数组转成列式，并校验形状（本模块文档第 1、2 条）。
///
/// `params` 是已经收集好的参数组；`param_count` 是语句的 `?` 个数
/// （[`Backend::param_count`](dm_core::Backend::param_count) 拿不到时是 `None`，
/// 那时只校验「各组等长」，与语句的一致性交给后端的绑定检查）。
pub(crate) fn collect(
    params: &[Vec<DmValue>],
    param_count: Option<u16>,
) -> Result<ColumnBatch, DmError> {
    let rows = params.len();

    // 空批次：合法，不执行任何语句。**先判这一步**，因为下面的「宽度一致性」在
    // 0 组时没有样本可比——`expected` 反而成了唯一的信息来源，而 0 组本来就不需要
    // 参数个数对得上（没有语句要执行）。
    if rows == 0 {
        return Ok(ColumnBatch {
            columns: Vec::new(),
            rows: 0,
        });
    }

    // 「语句有几个 `?`」优先信后端；后端答不上来时退化成「以第 0 组的宽度为准」。
    // 退化后仍然能拦住「第 3 组少了一个参数」——那正是这里存在的理由。
    let declared = param_count.map(usize::from);
    let expected = declared.unwrap_or(params[0].len());

    // 形状错误在这里就变成「第几组有几个参数」——让调用方直接对上自己的代码，
    // 而不是等驱动回一句「参数个数不正确」。
    for (index, args) in params.iter().enumerate() {
        if args.len() != expected {
            // 措辞分两种：知道语句宽度时说「语句有 N 个 `?`」；不知道时说
            // 「第 0 组有 N 个」——不能让调用方以为我们查过语句。
            let baseline = match declared {
                Some(_) => format!("语句有 {expected} 个 `?` 占位符"),
                None => format!("第 0 组有 {expected} 个参数"),
            };
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "批量写第 {index} 组参数有 {} 个，但{baseline}：\
                     每组参数个数必须完全一致",
                    args.len()
                )),
                Stage::Bind,
            ));
        }
    }

    // 转置。`expected == 0` 时（语句没有 `?`）没有列要建，`rows` 仍然照实记——
    // 调用方走「逐组执行」那条路，不经过列式绑定。
    let mut columns: Vec<Vec<DmValue>> = (0..expected).map(|_| Vec::with_capacity(rows)).collect();
    for args in params {
        for (index, value) in args.iter().enumerate() {
            columns[index].push(value.clone());
        }
    }

    Ok(ColumnBatch { columns, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ints(values: &[i32]) -> Vec<DmValue> {
        values.iter().map(|&v| DmValue::Int(v)).collect()
    }

    #[test]
    fn 转置成列式且各列等长() {
        let params = vec![ints(&[1, 10]), ints(&[2, 20]), ints(&[3, 30])];
        let batch = collect(&params, Some(2)).expect("形状正确");
        assert_eq!(batch.rows(), 3);
        assert_eq!(batch.width(), 2);
        assert_eq!(
            batch.as_columns(),
            &[
                vec![DmValue::Int(1), DmValue::Int(2), DmValue::Int(3)],
                vec![DmValue::Int(10), DmValue::Int(20), DmValue::Int(30)],
            ]
        );
    }

    #[test]
    fn 空批次既不报错也不建列() {
        let batch = collect(&[], Some(2)).expect("空批次合法");
        assert!(batch.is_empty());
        assert_eq!(batch.rows(), 0);
        assert_eq!(
            batch.width(),
            0,
            "没有参数就没有列，不该凭 param_count 造空列"
        );
    }

    #[test]
    fn 没有占位符的语句可以跑多组() {
        let params = vec![Vec::new(), Vec::new()];
        let batch = collect(&params, Some(0)).expect("0 个 ? 与 0 个参数一致");
        assert_eq!(batch.rows(), 2);
        assert_eq!(batch.width(), 0);
    }

    #[test]
    fn 某组参数个数不符报可行动的错() {
        let params = vec![ints(&[1, 10]), ints(&[2]), ints(&[3, 30])];
        let err = collect(&params, Some(2)).expect_err("第 1 组少一个参数");
        let text = err.to_string();
        assert!(text.contains("第 1 组"), "错误要说清是第几组：{text}");
        assert!(
            text.contains("2 个 `?`"),
            "错误要说清语句有几个占位符：{text}"
        );
    }

    #[test]
    fn 参数个数全多报错() {
        let params = vec![ints(&[1, 10, 100])];
        let err = collect(&params, Some(2)).expect_err("多一个参数");
        assert!(err.to_string().contains("第 0 组"));
    }

    /// 后端答不上来 `?` 个数时（`None`），仍然要拦住「某组宽度与别人不同」。
    #[test]
    fn 不知道占位符个数时以第一组为准仍然拦不一致() {
        let params = vec![ints(&[1, 10]), ints(&[2]), ints(&[3, 30])];
        let err = collect(&params, None).expect_err("第 1 组宽度不同");
        let text = err.to_string();
        assert!(text.contains("第 1 组"), "{text}");
        assert!(
            text.contains("第 0 组有 2 个参数"),
            "不能假装查过语句：{text}"
        );
    }

    /// 后端答不上来且各组本来就等宽：照常转置，一致性留给后端绑定去查。
    #[test]
    fn 不知道占位符个数时等宽输入照常转置() {
        let params = vec![ints(&[1, 10]), ints(&[2, 20])];
        let batch = collect(&params, None).expect("等宽就够转置");
        assert_eq!(batch.rows(), 2);
        assert_eq!(batch.width(), 2);
    }
}
