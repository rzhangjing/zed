//! 一条用户操作，以及从数据库读出的记录。

use time::OffsetDateTime;

use crate::error::{Error, Result};

/// 操作人 ID 的最大长度。
pub const MAX_USER_ID: usize = 64;
/// 操作人显示名的最大长度。
pub const MAX_USER_NAME: usize = 64;
/// 动作名的最大长度。
pub const MAX_ACTION: usize = 64;
/// 操作对象的最大长度。
pub const MAX_TARGET: usize = 128;
/// 结果描述的最大长度。
pub const MAX_RESULT: usize = 64;
/// 详情文本的最大长度（超出截断）。
pub const MAX_DETAIL: usize = 4096;

/// 一条待记录的用户操作。
///
/// 只有用户发起的操作才应该构造它；系统自身的动作（定时任务、初始化、内部补偿）
/// 不要调用记录器——这是「只记录用户操作」的边界。
///
/// 发生时间不需要填：`Recorder::log()` 会用当前时间打点，并保证时间戳严格递增。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationLog {
    /// 操作人 ID（登录名、用户 ID）。
    pub user_id: String,
    /// 操作人显示名（可选，便于阅读）。
    pub user_name: Option<String>,
    /// 做了什么，如 `删除订单`、`导出报表`。
    pub action: String,
    /// 操作对象，如 `order:O-88231`。
    pub target: Option<String>,
    /// 结果，如 `成功` / `失败：余额不足`。
    pub result: Option<String>,
    /// 附加信息：参数、变更前后、失败原因（超长自动截断）。
    pub detail: Option<String>,
}

impl OperationLog {
    /// 新建一条操作记录。
    pub fn new(user_id: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            user_name: None,
            action: action.into(),
            target: None,
            result: None,
            detail: None,
        }
    }

    /// 补充操作人显示名。
    #[must_use]
    pub fn with_user_name(mut self, user_name: impl Into<String>) -> Self {
        self.user_name = Some(user_name.into());
        self
    }

    /// 指定操作对象。
    #[must_use]
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// 指定结果。
    #[must_use]
    pub fn result(mut self, result: impl Into<String>) -> Self {
        self.result = Some(result.into());
        self
    }

    /// 指定附加信息。
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// 去空白、校验长度、截断详情。
    pub(crate) fn normalize(mut self) -> Result<Self> {
        self.user_id = self.user_id.trim().to_owned();
        self.action = self.action.trim().to_owned();
        if self.user_id.is_empty() {
            return Err(Error::Invalid("user_id 不能为空".to_owned()));
        }
        if self.action.is_empty() {
            return Err(Error::Invalid("action 不能为空".to_owned()));
        }
        check_len("user_id", &self.user_id, MAX_USER_ID)?;
        check_len("action", &self.action, MAX_ACTION)?;

        self.user_name = normalize_opt(self.user_name, MAX_USER_NAME, "user_name")?;
        self.target = normalize_opt(self.target, MAX_TARGET, "target")?;
        self.result = normalize_opt(self.result, MAX_RESULT, "result")?;
        self.detail = match self.detail {
            Some(detail) => {
                let detail = detail.trim();
                if detail.is_empty() {
                    None
                } else {
                    Some(truncate(detail, MAX_DETAIL))
                }
            }
            None => None,
        };
        Ok(self)
    }
}

/// 入队后的记录：内容 + 记录器打的时间点。
#[derive(Clone, Debug)]
pub(crate) struct PendingLog {
    pub(crate) occurred_at: OffsetDateTime,
    pub(crate) log: OperationLog,
}

/// 从数据库读出的一条记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// 发生时间，也是这条记录的主键。
    pub occurred_at: OffsetDateTime,
    /// 操作人 ID。
    pub user_id: String,
    /// 操作人显示名。
    pub user_name: Option<String>,
    /// 做了什么。
    pub action: String,
    /// 操作对象。
    pub target: Option<String>,
    /// 结果。
    pub result: Option<String>,
    /// 附加信息。
    pub detail: Option<String>,
}

fn check_len(field: &str, value: &str, max: usize) -> Result<()> {
    if value.chars().count() > max {
        return Err(Error::Invalid(format!("{field} 超过 {max} 个字符")));
    }
    Ok(())
}

fn normalize_opt(value: Option<String>, max: usize, field: &str) -> Result<Option<String>> {
    let Some(value) = value else { return Ok(None) };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    check_len(field, value, max)?;
    Ok(Some(value.to_owned()))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_identity_and_action() {
        assert!(OperationLog::new(" ", "登录").normalize().is_err());
        assert!(OperationLog::new("u-1", "  ").normalize().is_err());
    }

    #[test]
    fn trims_and_drops_empty_optionals() {
        let log = OperationLog::new(" u-1 ", " 登录 ")
            .target("   ")
            .result(" 成功 ")
            .normalize()
            .expect("valid");
        assert_eq!(log.user_id, "u-1");
        assert_eq!(log.action, "登录");
        assert_eq!(log.target, None);
        assert_eq!(log.result.as_deref(), Some("成功"));
    }

    #[test]
    fn truncates_oversized_detail() {
        let log = OperationLog::new("u-1", "导出")
            .detail("x".repeat(MAX_DETAIL + 100))
            .normalize()
            .expect("valid");
        let detail = log.detail.expect("detail");
        assert_eq!(detail.chars().count(), MAX_DETAIL + 1);
        assert!(detail.ends_with('…'));
    }
}
