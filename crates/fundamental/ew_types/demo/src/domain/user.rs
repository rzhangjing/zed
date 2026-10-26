//! 用户领域。

use std::time::SystemTime;

/// 用户标识。26 位，全局唯一。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(String);

impl UserId {
    /// 由外部给定的字符串构造。不做格式校验，由上层保证。
    pub fn new(raw: &str) -> Self {
        Self(raw.to_string())
    }

    /// 标识的文本形式。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 用户角色。
///
/// # 流转
/// ```text
/// Guest ──登录──▶ Member ──实名──▶ Verified
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// 未登录访客。
    Guest,
    /// 已登录普通用户。
    Member,
    /// 已实名认证用户。
    Verified,
}

/// 用户档案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// 用户标识。单位：—。范围：26 字符。默认值：无。非空。
    pub id: UserId,
    /// 昵称。单位：—。范围：[1, 32] 字符。默认值：无。非空。
    pub name: String,
    /// 角色。单位：—。范围：见 [`Role`]。默认值：Guest。
    pub role: Role,
}

/// 列表加载状态。每种状态只携带属于它的数据，非法组合无法构造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load<T> {
    /// 尚未发起请求。
    Idle,
    /// 请求中。`started_at` 供超时判断。
    Loading {
        /// 请求发起时刻。单位：UTC。范围：≤ 当前时刻。默认值：无。非空。
        started_at: SystemTime,
    },
    /// 成功且有数据。`items` 保证非空。
    Ready {
        /// 结果集。单位：—。范围：非空。默认值：无。保持服务端顺序。
        items: Vec<T>,
    },
    /// 成功但结果为空。
    Empty {
        /// 触发本次查询的关键词。单位：—。范围：[0, 128] 字符。默认值：无。可为空，为空表示无筛选条件。
        query: String,
    },
    /// 失败。`retryable` 表示重试是否有意义。
    Failed {
        /// 错误描述。单位：—。范围：—。默认值：无。非空。
        error: String,
        /// 是否可重试。单位：—。范围：—。默认值：true。非空。
        retryable: bool,
    },
}
