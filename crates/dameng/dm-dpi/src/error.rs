//! DPI 调用层面的错误。
//!
//! 这一层刻意**不**用 `dm_error::DmError`：DPI 的错误是「返回码 + 诊断记录」的二元组，
//! 而 `DmError` 需要「阶段 + 分类」。两者在 [`crate::backend`] 里才合并
//! （届时 `DpiError` → `DmErrorKind::Server` / `Stage::*`）。
//!
//! 这样拆的好处是：`dm-dpi` 的底层探测（`examples/dpi_smoke.rs`）不需要构造
//! 完整的领域错误，而真正的分类逻辑只有一处、可以被单测覆盖。

use std::fmt;

/// 一次 DPI 调用失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpiError {
    /// 达梦返回的错误码（`dpi_get_diag_rec` 的 `err_code`）。
    ///
    /// 不是驱动报的错时为 `0`，例如共享库加载失败、需要的导出符号不存在。
    pub code: i32,
    /// 诊断消息。多条诊断记录用「；」连接。
    pub message: String,
}

impl DpiError {
    /// 造一个不属于任何句柄的错误（共享库加载、符号缺失、参数不合法）。
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 造一个「本端用法不对」的错误。这类错误没有达梦错误码。
    #[must_use]
    pub fn misuse(message: impl Into<String>) -> Self {
        Self::new(0, message)
    }
}

impl fmt::Display for DpiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.code == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "[{}] {}", self.code, self.message)
        }
    }
}

impl std::error::Error for DpiError {}

/// 「共享库没找到 / 找不到某个导出符号」。
///
/// 单独一个类型是因为它的处置方式和 SQL 错误完全不同：SQL 错误该重试就重试，
/// 而库加载失败只能是部署问题，且值得把「试过哪些路径」一次性告诉用户。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    /// 依次尝试过的库路径，以及各自失败的原因。
    pub tried: Vec<(String, String)>,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "加载达梦 DPI 共享库失败：没找到可用的 dmdpi 动态库。\
             可以设置环境变量 DM_DPI_LIB 指向完整路径。尝试过："
        )?;
        for (path, why) in &self.tried {
            write!(f, "\n  - {path}：{why}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoadError {}
