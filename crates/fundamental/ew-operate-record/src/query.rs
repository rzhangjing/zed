//! 查询条件。

use time::OffsetDateTime;

/// 查询条件，所有条件之间是「与」关系。
#[derive(Clone, Debug, Default)]
pub struct Query {
    /// 操作人 ID（精确匹配）。
    pub user_id: Option<String>,
    /// 动作名（精确匹配）。
    pub action: Option<String>,
    /// 关键字：动作、对象、详情、操作人（模糊匹配）。
    pub keyword: Option<String>,
    /// 起始时间（含）。
    pub from: Option<OffsetDateTime>,
    /// 结束时间（含）。
    pub to: Option<OffsetDateTime>,
    /// 返回条数，默认 100，最大 1000。
    pub limit: usize,
    /// 游标：返回发生时间早于该时刻的记录（上一页最后一条的时间）。
    pub before_us: Option<i64>,
}

impl Query {
    /// 默认条件：最新在前，取 100 条。
    pub fn new() -> Self {
        Self {
            limit: 100,
            ..Self::default()
        }
    }

    /// 只看某个操作人。
    #[must_use]
    pub fn user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// 只看某个动作。
    #[must_use]
    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// 关键字模糊匹配。
    #[must_use]
    pub fn keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keyword = Some(keyword.into());
        self
    }

    /// 时间范围（含两端）。
    #[must_use]
    pub fn time_range(mut self, from: OffsetDateTime, to: OffsetDateTime) -> Self {
        self.from = Some(from);
        self.to = Some(to);
        self
    }

    /// 返回条数（会自动限制在 1..=1000）。
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// 从某条记录之前继续取（翻页），传入上一页最后一条的 `occurred_at`。
    #[must_use]
    pub fn before(mut self, occurred_at: OffsetDateTime) -> Self {
        self.before_us = Some(to_micros(occurred_at));
        self
    }
}

fn to_micros(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000).unwrap_or_default()
}
