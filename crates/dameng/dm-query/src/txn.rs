//! 事务外观：`Transaction`（设计方案 §6.6 / §6.12）。
//!
//! # 为什么 `Transaction` 是「吃掉连接」的
//!
//! `PooledConn::begin(self) -> Transaction` 把连接**所有权**交给事务对象。这样
//! 「事务之外还能用这条连接跑语句」在**类型层面**就不可能——不必靠约定，也不必靠
//! 运行期状态检查。事务结束时 `commit`/`rollback` 同样吃掉 `self`；想接着用连接就
//! 用 [`Transaction::conn`] 借回可变引用，或者干脆重新借一条。
//!
//! # 连接为什么在事务结束后必须过一遍 `reset()`
//!
//! 达梦的「开始事务」就是 `SQL_ATTR_AUTOCOMMIT = OFF`（`dm-odbc` 的 `begin` 文档写明），
//! 而 `commit`/`rollback` **不会**把它扳回 `ON`。所以事务结束后的连接带着
//! `autocommit = OFF` 回到池里是危险的：下一位借出方跑的每条语句都会落进一个没人提交的
//! 隐式事务（真机 T3.7 就因此撞上「断开时事务还开着 ⇒ 释放 DBC 句柄报 -1」那个坑）。
//!
//! `Backend` 抽象里唯一可移植的还原入口是 `reset()`（= 回滚 + `autocommit = ON`），
//! 因此本类型在 [`Drop`] 里**无条件**把连接标脏，让池去跑复位。代价是每次事务多两次
//! 往返（一次空回滚 + 一次设属性），换来的是「绝不让半配置的连接回到池里」。

use dm_core::{Backend, DAMENG};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
use dm_pool::DefaultBackend;

use crate::args::IntoArgs;
use crate::conn::PooledConn;
use crate::rowset::BatchOutcome;
use crate::rowset::RowSet;

/// 隔离级别就是 `dm-core` 的那一个（`dm-core::capability::IsolationLevel`）——
/// 冻结项里 `dm-query::IsolationLevel` 与 `DAMENG.isolation_levels` 必须是同一个类型，
/// 否则「声明支持的档位」和「能传进来的档位」会各说各话。
pub use dm_core::IsolationLevel;
/// 短别名，写签名时少打几个字。
pub use dm_core::IsolationLevel as Level;

/// 隔离级别 → 设置语句。
///
/// 取值文本直接用 `dm-core` 的 `as_str()`（`"READ COMMITTED"` 等），避免两处各写一份
/// 字符串而在改名时只改一处。
pub(crate) fn isolation_sql(level: IsolationLevel) -> String {
    format!("SET TRANSACTION ISOLATION LEVEL {}", level.as_str())
}

/// 一个正在进行的事务。
///
/// **不实现 `Clone`/`Copy`**：事务不能被复制，否则「谁负责提交」就没有答案了。
#[derive(Debug)]
pub struct Transaction<B: Backend = DefaultBackend> {
    conn: PooledConn<B>,
}

impl<B: Backend> Transaction<B> {
    pub(crate) fn new(conn: PooledConn<B>) -> Self {
        Self { conn }
    }

    /// 事务里那条连接（继续跑语句用；不要把它借出去活过事务）。
    pub fn conn(&mut self) -> &mut PooledConn<B> {
        &mut self.conn
    }

    /// 事务内查询（语义同 [`PooledConn::query`]）。
    ///
    /// # Errors
    ///
    /// 同 [`PooledConn::query`]。
    pub async fn query(&mut self, sql: &str, args: impl IntoArgs) -> Result<RowSet, DmError> {
        self.conn.query(sql, args).await
    }

    /// 事务内执行（语义同 [`PooledConn::execute`]）。
    ///
    /// # Errors
    ///
    /// 同 [`PooledConn::execute`]。
    pub async fn execute(&mut self, sql: &str, args: impl IntoArgs) -> Result<u64, DmError> {
        self.conn.execute(sql, args).await
    }

    /// 事务内批量执行（语义同 [`PooledConn::execute_many`]）。
    ///
    /// # Errors
    ///
    /// 同 [`PooledConn::execute_many`]。
    pub async fn execute_many(
        &mut self,
        sql: &str,
        params: impl IntoIterator<Item = impl IntoArgs>,
    ) -> Result<BatchOutcome, DmError> {
        self.conn.execute_many(sql, params).await
    }

    /// 设保存点。
    ///
    /// 名字会被拼进 SQL，所以这里先做**标识符白名单校验**（`[A-Za-z_][A-Za-z0-9_]*`，
    /// 最长 128 字节）——不校验就等于把 SQL 注入的口子开在参数名上。
    ///
    /// # Errors
    ///
    /// 名字非法 → `Config`；能力表未声明保存点 → `Unsupported`；服务端拒绝 → 原样上报。
    pub async fn savepoint(&mut self, name: &str) -> Result<(), DmError> {
        let name = checked_savepoint_name(name)?;
        self.conn.execute(&format!("SAVEPOINT {name}"), ()).await?;
        Ok(())
    }

    /// 回滚到保存点（保存点保留，可继续回滚到它）。
    ///
    /// # 达梦实测：`ROLLBACK TO SAVEPOINT` 之后**必须**再走一条语句
    ///
    /// DM8 的 ODBC 驱动在收到 `ROLLBACK TO SAVEPOINT` 之后，会把「事务还开着」
    /// 这件事判成已结束；紧接着的 `SQLEndTran(SQL_COMMIT)` 于是**返回成功却什么都不做**。
    /// 真机实测（驱动层探针 `crates/dm-odbc/examples/savepoint_probe.rs` R0–R21，与
    /// `crates/dm-query/tests/query_real.rs` 的两条 `real_savepoint_commit_*` 用例）：
    ///
    /// - `rollback_to` 之后**紧接** `commit`：独立会话看不到保存点之前的行，
    ///   而提交方会话自己仍看得见 ⇒ 整个事务没落地，连接还带着一个开着的事务；
    /// - 两者之间多跑**任意一条**语句（哪怕是一条注定失败的 `RELEASE SAVEPOINT`）就完全正常；
    /// - 把最后一条语句换成 `SAVEPOINT sp2`（不是 `ROLLBACK` 开头）也正常 ⇒ 驱动是按
    ///   **语句文本前缀**把 `ROLLBACK TO SAVEPOINT` 误判成「事务结束」的；
    /// - 若这次事务里**根本没有发生过** `SQL_ATTR_AUTOCOMMIT` 的 ON→OFF 状态变更
    ///   （连接早就是 autocommit OFF），同一个序列**不会**出问题 ⇒ 触发条件是驱动客户端
    ///   那份「显式事务模式」标志被这次误判清掉；
    /// - 标志被清掉后再调一次 `SQLEndTran(SQL_COMMIT)` 就能真正提交，调
    ///   `set_autocommit(true)` 也**会提交**（ODBC 规范如此）——后者意味着一旦放任不管，
    ///   连接回池复位时的 `set_autocommit(true)` 有可能把本该回滚的写入提交掉。
    ///
    /// 所以这里在回滚之后补一条无害语句，让驱动重新确认事务边界。代价是保存点回滚
    /// 多一次往返；保存点本身是低频路径，这个代价换「`commit` 不静默丢事务」是划算的。
    ///
    /// # Errors
    ///
    /// 同 [`Transaction::savepoint`]。
    pub async fn rollback_to(&mut self, name: &str) -> Result<(), DmError> {
        let name = checked_savepoint_name(name)?;
        let rolled = self
            .conn
            .execute(&format!("ROLLBACK TO SAVEPOINT {name}"), ())
            .await;
        // 成功失败都补：语句失败时驱动同样可能已经把「事务开着」的标志清掉，
        // 而失败的原因（未知保存点、断连）由 `rolled` 那条错误如实上报。
        let rearm = self.conn.execute(REARM_SQL, ()).await;
        rolled?;
        rearm?;
        Ok(())
    }

    /// 释放保存点（释放之后就不能再回滚到它了）。
    ///
    /// # 达梦实测：DM8 **没有** `RELEASE SAVEPOINT` 语法
    ///
    /// 2026 年在 DM8（`localhost:5236`，SYSDBA）上实测：`RELEASE SAVEPOINT sp1`
    /// 被服务端以 `HY000 / 52000 / -2007`「第 1 行…\[SAVEPOINT\]附近出现错误: 语法分析出错」
    /// 拒绝；而 `SAVEPOINT sp1` 与 `ROLLBACK TO SAVEPOINT sp1` **都成功**。达梦社区
    /// 也有一模一样的未解答提问（<https://eco.dameng.com/community/question/1bba7cc5a092b0572f4807e460995aff>）。
    ///
    /// 所以这里仍然把语句发出去（万一将来的版本支持，代码不用改），但**把语法拒绝
    /// 翻成 [`DmErrorKind::Unsupported`]**：调用方写的是可移植代码，需要知道「本机没有
    /// 这个能力」，而不是去猜 `-2007` 是不是自己的 SQL 写错了。原始诊断（含原生码）
    /// 会原样保留。
    ///
    /// 达梦上保存点不需要显式释放：事务 `COMMIT`/`ROLLBACK` 时自动释放；只想撤销
    /// 建立之后的影响就用 [`Transaction::rollback_to`]。
    ///
    /// # Errors
    ///
    /// 名字非法 → `Config`；能力表未声明保存点 → `Unsupported`；服务端不支持该语法 →
    /// `Unsupported`（见上）；其它服务端错误原样上报。
    pub async fn release_savepoint(&mut self, name: &str) -> Result<(), DmError> {
        let name = checked_savepoint_name(name)?;
        match self
            .conn
            .execute(&format!("RELEASE SAVEPOINT {name}"), ())
            .await
        {
            Ok(_) => Ok(()),
            Err(err) if matches!(err.kind, DmErrorKind::Syntax) => {
                Err(unsupported_release_savepoint(name, err))
            }
            Err(err) => Err(err),
        }
    }

    /// 提交。
    ///
    /// # Errors
    ///
    /// 服务端拒绝提交时原样上报；此时连接已按脏连接处理（不会带着半开事务回池）。
    pub async fn commit(mut self) -> Result<(), DmError> {
        let result = self.conn.guard_mut().with_conn(|raw| raw.commit()).await;
        self.conn.guard_mut().mark_dirty();
        result
    }

    /// 回滚。
    ///
    /// # Errors
    ///
    /// 服务端拒绝回滚时原样上报（多半意味着连接已经断了）。
    pub async fn rollback(mut self) -> Result<(), DmError> {
        let result = self.conn.guard_mut().with_conn(|raw| raw.rollback()).await;
        self.conn.guard_mut().mark_dirty();
        result
    }
}

impl<B: Backend> Drop for Transaction<B> {
    fn drop(&mut self) {
        // 没提交也没回滚就丢了：**不在这里回滚**（drop 可能跑在 tokio 工作线程上，
        // 同步调 ODBC 会阻塞运行时的其他任务，§5.8）。标脏之后由池在专用阻塞线程上
        // 回滚 + 还原 autocommit。
        self.conn.guard_mut().mark_dirty();
        tracing::debug!(
            conn_id = self.conn.id(),
            "事务对象被丢弃：连接已标脏，将由池回滚未提交事务"
        );
    }
}

/// 保存点名白名单校验。
fn checked_savepoint_name(name: &str) -> Result<&str, DmError> {
    if !DAMENG.transaction_savepoint {
        return Err(DmError::at(
            DmErrorKind::Unsupported {
                feature: "保存点（SAVEPOINT）".to_owned(),
            },
            Stage::Other,
        ));
    }
    let valid_len = !name.is_empty() && name.len() <= 128;
    let mut chars = name.chars();
    let head_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let tail_ok = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid_len && head_ok && tail_ok {
        return Ok(name);
    }
    Err(DmError::at(
        DmErrorKind::Config(format!(
            "保存点名 {name:?} 不是合法的标识符：只允许 [A-Za-z_][A-Za-z0-9_]*，且不超过 128 字节\
             （名字会被拼进 SQL，必须白名单校验）"
        )),
        Stage::Config,
    ))
}

/// 保存点回滚之后用来「让驱动重新确认事务边界」的无害语句。
///
/// `select 1 from dual` 的形状在达梦上恒可用、不碰任何用户对象、不产生副作用；
/// 它在 `execute` 眼里是「带结果集、影响行数为 0」的语句，结果集会被丢掉。
/// 详细理由见 [`Transaction::rollback_to`]。
const REARM_SQL: &str = "select 1 from dual";

/// 把「服务端不认 `RELEASE SAVEPOINT`」这个语法拒绝翻成明确的 `Unsupported`。
///
/// 只改 `kind`、**保留原始诊断**（`-2007` 那些 `DiagRecord` 是证据，丢掉了调用方就只能
/// 看到一句我们的话），再补一条说明「该怎么办」。`context`（阶段、指纹）也一并保留，
/// 于是日志里仍然能看到这句话是被 prepare 阶段拒的。
fn unsupported_release_savepoint(name: &str, mut err: DmError) -> DmError {
    err.kind = DmErrorKind::Unsupported {
        feature: "RELEASE SAVEPOINT（达梦 DM8 不支持该语法）".to_owned(),
    };
    err.push_diag(DiagRecord::native(
        0,
        format!(
            "`RELEASE SAVEPOINT {name}` 被服务端以语法错拒绝（DM8 实测 -2007）。\
             保存点不需要显式释放：事务 COMMIT/ROLLBACK 时自动释放；\
             只想撤销它建立之后的影响，用 `rollback_to({name:?})`。"
        ),
    ));
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_sql_matches_dm_core_names() {
        assert_eq!(
            isolation_sql(IsolationLevel::ReadCommitted),
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"
        );
        assert_eq!(
            isolation_sql(IsolationLevel::Serializable),
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE"
        );
    }

    #[test]
    fn savepoint_names_are_whitelisted() {
        assert_eq!(checked_savepoint_name("sp_1").unwrap(), "sp_1");
        assert_eq!(checked_savepoint_name("_x").unwrap(), "_x");

        for bad in ["1sp", "sp-1", "sp; drop table t", "", "s p", "中"] {
            let err = checked_savepoint_name(bad).unwrap_err();
            assert_eq!(err.context.stage, Stage::Config, "名字 {bad:?} 应被拒绝");
            assert!(matches!(err.kind, DmErrorKind::Config(_)));
        }
    }

    #[test]
    fn over_long_savepoint_name_is_rejected() {
        let long = "a".repeat(129);
        assert!(checked_savepoint_name(&long).is_err());
        let ok = "a".repeat(128);
        assert!(checked_savepoint_name(&ok).is_ok());
    }
}
