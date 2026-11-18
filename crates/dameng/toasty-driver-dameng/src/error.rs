//! 错误映射：`dm-error::DmError` → `toasty_core::Error`（设计方案 §9.1、CAP-I-07、CAP-K-14）。
//!
//! # 为什么必须「保分类」而不是无脑包一层
//!
//! Toasty 的 `Error` 是**不透明结构体**（不是 `pub enum`），调用方只能靠
//! `is_connection_lost()` / `is_unsupported_feature()` / `is_serialization_failure()` /
//! `is_read_only_transaction()` / `is_driver_operation_failed()` 这些谓词分类。
//! 上游文档对驱动作者写得很直白：
//!
//! > Returning a `driver_operation_failed` for a connection-level fault instead of this
//! > variant leaks a dead slot back into the pool.
//!
//! 也就是说，把「连接已断」错报成普通执行失败，**池会把死连接放回空闲集**，
//! 之后每次借出都失败一次，且后台巡检也不会被唤醒。所以这张映射表是硬契约，不是审美问题。
//!
//! # 映射表
//!
//! | `DmErrorKind` | `toasty_core::Error` | 理由 |
//! |---|---|---|
//! | `Io`、`ConnectionLost` | `connection_lost(..)` | 与 [`DmErrorKind::is_connection_lost`] 的判定**逐字对齐**（只有这两个变体为真） |
//! | `Unsupported { feature }` | `unsupported_feature(..)` | 上层据此把「DM 不做」与「暂时失败」分开 |
//! | `Deadlock`、`SerializationFailure` | `serialization_failure(..)` | 上游没有独立的 deadlock 变体；两者调用方语义相同（事务被回滚、可重试） |
//! | `ReadOnlyTransaction` | `read_only_transaction(..)` | 只读事务里写 |
//! | 其余 16 个变体 | `driver_operation_failed(..)` | 不猜语义：原样透传，保留 `DmError` 的诊断链 |
//!
//! ⚠️ 两个**故意不蹭**的上游变体：
//!
//! - `duplicate_key` 在上游**不存在**——唯一键冲突与约束违反都只有
//!   `driver_operation_failed` 可落（`SQLSTATE`/原生码经 cause 链照样保留）。
//! - `condition_failed` 的语义是「条件更新**没匹配到行**」（乐观锁版本不符、
//!   DynamoDB 条件写失败），**不是**唯一键冲突；蹭上去会让上层的重试/报错判断错位。
//!
//! # 诊断链不能被吞
//!
//! 被分类成上面四种时，本模块用 `Error::context` 把**原始 `DmError` 作为 cause** 挂在
//! 分类错误的里面（`context(self, consequent)` 的语义是：`self` 当 cause、`consequent`
//! 当最外层）。这样既满足上游谓词，又让 `Display` 打出
//! `transaction serialization failure: …: 服务端错误 [40001/-…]`，
//! `dm-error` 记录的原生码/`SQLSTATE` 一条都不丢。
//!
//! ```text
//! toasty_core::Error（最外层 = 分类）
//!   └── cause: toasty_core::Error::driver_operation_failed(DmError)
//!         └── source: DmError（Display 里带 SQLSTATE + 原生码 + 消息）
//! ```

use std::fmt;

use dm_error::{DmError, DmErrorKind};

/// 把 `DmError` 的**诊断明细**带进 Toasty 的 cause 链。
///
/// `DmError` 自己的 `Display` 只打 `kind`（例如「SQL 语法错误」），把 `diags` 里的
/// `SQLSTATE`、原生码、服务端消息全留在了 `Debug` 里——而真机排错最需要的正是这几项
/// （首轮 T8.1a 联调就吃了这个亏：`push_schema` 只报「SQL 语法错误」，看不出是哪一条
/// DDL、也看不出服务端到底说了什么）。
///
/// 上游的 `toasty_core::Error` 会把整条 cause 链打出来，所以这里做一个轻量包装：
/// `Display` 给出「kind（阶段 X）；[SQLSTATE/原生码] 服务端消息；语句：…」，
/// `source()` 再指回原始 `DmError`（保留类型化的诊断向量）。
#[derive(Debug)]
struct Detailed {
    /// 原始错误：`Display` 给「kind」，`source()` 让它留在链上。
    err: DmError,
    /// 触发错误的语句（能拿到时才有；DDL/查询路径都带）。
    sql: Option<String>,
}

impl Detailed {
    fn new(err: DmError, sql: Option<&str>) -> Self {
        Self {
            err,
            sql: sql.map(str::to_owned),
        }
    }
}

impl fmt::Display for Detailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}（阶段 {}）", self.err.kind, self.err.context.stage)?;
        for diag in self.err.diags() {
            write!(f, "；{diag}")?;
        }
        if let Some(sql) = &self.sql {
            write!(f, "；语句：{sql}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Detailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.err)
    }
}

/// 把中间件错误映射成 Toasty 错误，**保留分类与诊断链**。
///
/// 这是驱动里唯一的错误出口：`conn.rs`/`driver.rs`/`value.rs` 里的每个
/// `DmError` 都必须经过它，不要各处自己 `Error::driver_operation_failed`。
#[must_use]
pub fn to_toasty(err: DmError) -> toasty_core::Error {
    to_toasty_inner(err, None)
}

/// 同 [`to_toasty`]，但把**触发错误的语句**也带进诊断链。
///
/// 执行/查询路径用这个：达梦的「SQL 语法错误」本身不含语句文本，真机联调时
/// 没有它就等于盲调（T8.1a 的教训）。
#[must_use]
pub fn to_toasty_with_sql(err: DmError, sql: &str) -> toasty_core::Error {
    to_toasty_inner(err, Some(sql))
}

fn to_toasty_inner(err: DmError, sql: Option<&str>) -> toasty_core::Error {
    /// 先把分类算出来，避免在借用 `err.kind` 的同时移动 `err`。
    enum Class {
        Lost,
        Unsupported(String),
        Serialization,
        ReadOnly,
        Other,
    }

    let class = match &err.kind {
        // 与 `DmErrorKind::is_connection_lost()` 的判定保持一致：
        // 这两个变体 ⇒ 连接不可复用 ⇒ 必须让池驱逐。
        DmErrorKind::Io | DmErrorKind::ConnectionLost => Class::Lost,
        DmErrorKind::Unsupported { feature } => Class::Unsupported(feature.clone()),
        // 死锁与序列化失败在上游**共用** `serialization_failure`：`toasty_core::Error` 没有
        // 单独的 deadlock 变体，而两者的调用方语义完全一致（「事务被服务端回滚，可重试」）。
        // 设计方案 §9.5 冻结的就是这个映射。
        DmErrorKind::Deadlock | DmErrorKind::SerializationFailure => Class::Serialization,
        DmErrorKind::ReadOnlyTransaction => Class::ReadOnly,
        _ => Class::Other,
    };

    match class {
        Class::Lost => toasty_core::Error::connection_lost(Detailed::new(err, sql)),
        // 下面三种是「语义可分类」的错误：最外层给上游谓词，cause 里挂原始 `DmError`。
        Class::Unsupported(feature) => {
            classified(err, sql, toasty_core::Error::unsupported_feature(feature))
        }
        Class::Serialization => classified(
            err,
            sql,
            toasty_core::Error::serialization_failure("达梦报告事务被回滚（死锁或序列化失败）"),
        ),
        Class::ReadOnly => classified(
            err,
            sql,
            toasty_core::Error::read_only_transaction("达梦拒绝了只读事务中的写操作"),
        ),
        // 其余全部原样透传：不替调用方猜语义，也不改分类。
        Class::Other => toasty_core::Error::driver_operation_failed(Detailed::new(err, sql)),
    }
}

/// 把 `DmError` 作为 cause 挂到分类错误里面。
///
/// `Error::context` 在「最外层错误已经带 cause」时会 panic，所以这里先造一个
/// 干净的 `driver_operation_failed` 当内层，再挂分类错误当最外层。
fn classified(
    err: DmError,
    sql: Option<&str>,
    classified: toasty_core::Error,
) -> toasty_core::Error {
    toasty_core::Error::driver_operation_failed(Detailed::new(err, sql)).context(classified)
}

/// 「这个 `Operation` 变体达梦不支持」的统一错误（`conn.rs` 用）。
///
/// 故意走 `unsupported_feature` 而不是 `todo!()`：设计方案 CAP-K-26 明确要求
/// 「不支持的结构变更返回 `unsupported_feature`（无 `todo!()`）」——
/// 一个还没实现的变体是**可诊断的能力缺口**，不是一个 panic。
#[must_use]
pub fn unsupported_operation(what: &str) -> toasty_core::Error {
    toasty_core::Error::unsupported_feature(format!("达梦驱动尚未支持的操作：{what}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dm_error::{DiagRecord, ErrorContext, SQLSTATE_LEN, Stage};

    /// 一条带完整诊断链的 `DmError`，用来确认映射不会吞掉服务端三元组。
    fn server_error(code: i32, sqlstate: [u8; SQLSTATE_LEN]) -> DmError {
        let mut err = DmError::new(DmErrorKind::Server {
            sqlstate,
            code,
            message: "语法分析出错".to_string(),
        });
        err.push_diag(DiagRecord::new(sqlstate, code, "语法分析出错"));
        err.context = ErrorContext::new(Stage::Execute);
        err
    }

    #[test]
    fn connection_faults_use_connection_lost() {
        // 与 `DmErrorKind::is_connection_lost()` 完全对齐：只有 Io 与 ConnectionLost。
        for kind in [DmErrorKind::Io, DmErrorKind::ConnectionLost] {
            let mapped = to_toasty(DmError::new(kind));
            assert!(
                mapped.is_connection_lost(),
                "Io/ConnectionLost 必须映射成 connection_lost，否则死连接会漏回池"
            );
            assert!(!mapped.is_driver_operation_failed());
        }
    }

    #[test]
    fn other_kinds_are_driver_operation_failed() {
        for kind in [
            DmErrorKind::Syntax,
            DmErrorKind::ObjectNotFound,
            DmErrorKind::ConstraintViolation,
            DmErrorKind::DuplicateKey,
            DmErrorKind::Timeout,
            DmErrorKind::PoolTimeout,
            DmErrorKind::Encode { column: None },
            DmErrorKind::Decode { column: None },
            DmErrorKind::NullViolation {
                column: "c".to_string(),
            },
            DmErrorKind::DataTypeMismatch {
                expect: "i64".to_string(),
                got: "String".to_string(),
            },
            DmErrorKind::CharsetMismatch {
                configured: 65001,
                actual: 0,
            },
            DmErrorKind::Config("port".to_string()),
            DmErrorKind::Connect {
                source: "refused".to_string(),
            },
            DmErrorKind::DriverNotFound {
                tried: vec!["a".to_string()],
                hint: "装驱动".to_string(),
            },
        ] {
            let mapped = to_toasty(DmError::new(kind));
            assert!(mapped.is_driver_operation_failed());
            assert!(
                !mapped.is_connection_lost(),
                "只有 Io/ConnectionLost 才是连接失效"
            );
        }
    }

    #[test]
    fn classified_kinds_keep_predicate_and_diagnostics() {
        let unsupported = to_toasty(DmError::new(DmErrorKind::Unsupported {
            feature: "SQL_ATTR_ASYNC_ENABLE".to_string(),
        }));
        assert!(unsupported.is_unsupported_feature());
        assert!(unsupported.to_string().contains("SQL_ATTR_ASYNC_ENABLE"));

        let serialization = to_toasty(DmError::new(DmErrorKind::SerializationFailure));
        assert!(serialization.is_serialization_failure());

        // 死锁走同一个上游变体（§9.5 的冻结映射）。
        let deadlock = to_toasty(DmError::new(DmErrorKind::Deadlock));
        assert!(deadlock.is_serialization_failure());

        let read_only = to_toasty(DmError::new(DmErrorKind::ReadOnlyTransaction));
        assert!(read_only.is_read_only_transaction());

        // 分类错误是「最外层」，原始 DmError 作为 cause 仍在链上。
        let mapped = to_toasty(server_error(-2007, *b"42000"));
        assert!(mapped.is_driver_operation_failed());
        let text = mapped.to_string();
        assert!(
            text.contains("-2007") && text.contains("42000"),
            "原生码与 SQLSTATE 必须留在 Display 链上，实际：{text}"
        );
    }

    #[test]
    fn unsupported_operation_is_diagnosable_not_panic() {
        let err = unsupported_operation("Operation::Scan");
        assert!(err.is_unsupported_feature());
        assert!(err.to_string().contains("Operation::Scan"));
    }
}
