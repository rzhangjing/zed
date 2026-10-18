//! 能力声明表（设计方案 §5.10）——把「DM 支持什么」从散落在实现里的 `if`
//! 变成**集中的、可查询的常量表**。
//!
//! ## 硬纪律
//!
//! 调用未声明支持的能力时**立即返回 [`DmErrorKind::Unsupported`]，不静默降级**。
//! 降级的问题不是「结果不对」，而是「结果看起来对」——那会让调用方在
//! 生产环境才发现语义丢了。
//!
//! ## 这张表的性质
//!
//! 它是**实测结论的容器**，不是愿望清单。每一行都对应 §4 的一次实测或
//! §14 的一个待验证项；`⚠️` 的行用 [`SupportTier::BestEffort`] 表达
//! 「能做但不承诺」，用 [`SupportTier::NotSupported`] 表达「明确不做」。
//!
//! ## 与方法 / 与 `COMPATIBLE_MODE` 的关系
//!
//! 表里的方言类取值（占位符、引号、upsert、limit）都是在
//! `COMPATIBLE_MODE=0`（DM 原生模式）下实测的。**开启 Oracle 兼容模式会改变
//! 其中若干行的语义**，所以 [`Capability::compatible_mode`] 是表的一部分：
//! 部署若改了它，必须重新验证这张表，而不是假定表还成立。

use core::fmt;

use crate::config::CompatMode;
use dm_error::{DmError, DmErrorKind, Stage};

/// 参数占位符风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaceholderStyle {
    /// `?`（位置式）。DM 实测只认这个（§4.6）。
    QuestionMark,
    /// `:name` 命名式（**DM 不支持**，列在这里是为了让上层能明确拒绝）。
    NamedColon,
    /// `$1` 编号式（**DM 不支持**）。
    DollarNumbered,
}

impl PlaceholderStyle {
    /// 用于错误消息的稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            PlaceholderStyle::QuestionMark => "?",
            PlaceholderStyle::NamedColon => ":name",
            PlaceholderStyle::DollarNumbered => "$1",
        }
    }
}

/// upsert 语法族。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpsertSyntax {
    /// 完全没有 upsert。
    None,
    /// `MERGE INTO … WHEN MATCHED / WHEN NOT MATCHED`（DM 实测接受，§4.6）。
    MergeInto,
    /// `ON DUPLICATE KEY UPDATE`（**DM 拒绝** `-2007`）。
    OnDuplicateKeyUpdate,
    /// `ON CONFLICT … DO UPDATE`（**DM 拒绝**）。
    OnConflict,
}

impl UpsertSyntax {
    /// 用于错误消息的稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            UpsertSyntax::None => "none",
            UpsertSyntax::MergeInto => "MERGE INTO",
            UpsertSyntax::OnDuplicateKeyUpdate => "ON DUPLICATE KEY UPDATE",
            UpsertSyntax::OnConflict => "ON CONFLICT DO UPDATE",
        }
    }
}

/// 自增列语法族。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AutoIncrementSyntax {
    /// 不支持。
    None,
    /// `IDENTITY(1,1)` 或 `AUTO_INCREMENT`（DM 实测两者都接受，§4.6）。
    IdentityOrAutoIncrement,
    /// `GENERATED … AS IDENTITY`（**DM 拒绝** `-2007`）。
    GeneratedAsIdentity,
}

impl AutoIncrementSyntax {
    /// 用于错误消息的稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            AutoIncrementSyntax::None => "none",
            AutoIncrementSyntax::IdentityOrAutoIncrement => "IDENTITY(1,1) / AUTO_INCREMENT",
            AutoIncrementSyntax::GeneratedAsIdentity => "GENERATED ... AS IDENTITY",
        }
    }
}

/// 分页语法族。DM 同时接受多种（§4.6 实测三种 `LIMIT` 形态全通）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitStyle {
    /// `LIMIT n` / `LIMIT n OFFSET m`。
    LimitOffset,
    /// `LIMIT offset, count`。
    LimitComma,
    /// `TOP n`。
    Top,
    /// `ROWNUM <= n`。
    Rownum,
}

/// `column_size` 的单位语义（§4.4 / D16）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColumnSizeUnit {
    /// 报的是**声明长度**，单位还需按 `LENGTH_IN_CHAR` 解释（DM 实测即此）。
    DeclaredLength,
    /// 报的是内容长度（DM **不是**这样）。
    ContentLength,
}

/// 能力的实现档位（用于 §5.10 里标 `⚠️` 的行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum SupportTier {
    /// ✅ 保证：已实测，进验收标准。
    Guaranteed,
    /// ⚠️ 尽力：路径存在但达梦侧未实测，失败要能优雅退化。
    BestEffort,
    /// ❌ 明确不做：调用即 `Unsupported`。
    NotSupported,
}

impl SupportTier {
    /// 是否可承诺（`Guaranteed` 为真）。
    pub const fn is_guaranteed(self) -> bool {
        matches!(self, SupportTier::Guaranteed)
    }

    /// 用于错误消息的稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            SupportTier::Guaranteed => "guaranteed",
            SupportTier::BestEffort => "best-effort",
            SupportTier::NotSupported => "not-supported",
        }
    }
}

/// DM 的 4 档事务隔离级别（§6.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum IsolationLevel {
    /// `READ UNCOMMITTED`
    ReadUncommitted,
    /// `READ COMMITTED`（DM 默认）
    ReadCommitted,
    /// `REPEATABLE READ`
    RepeatableRead,
    /// `SERIALIZABLE`
    Serializable,
}

impl IsolationLevel {
    /// DM 支持的 4 档，由弱到强。
    pub const ALL: [IsolationLevel; 4] = [
        IsolationLevel::ReadUncommitted,
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Serializable,
    ];

    /// 传给 ODBC `SQL_ATTR_TXN_ISOLATION` 的**原始位值**。
    ///
    /// 这里是裸 `i32` 而不是 `odbc_api` 的枚举，因为 `dm-core` 不许依赖
    /// `odbc-api`（CI 强校验）；`dm-odbc` 负责把返回值转成它的枚举。
    /// 取值来自 ODBC 标准：`READ_UNCOMMITTED=1`、`READ_COMMITTED=2`、
    /// `REPEATABLE_READ=4`、`SERIALIZABLE=8`。
    pub const fn odbc_value(self) -> i32 {
        match self {
            IsolationLevel::ReadUncommitted => 1,
            IsolationLevel::ReadCommitted => 2,
            IsolationLevel::RepeatableRead => 4,
            IsolationLevel::Serializable => 8,
        }
    }

    /// 用于错误消息与日志的稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    }
}

impl fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一个后端的完整能力声明。
///
/// **不是** `Default`：每一条都必须是有人负责的声明，给默认值等于鼓励沉默。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Capability {
    /// 参数占位符风格。
    pub sql_placeholder: PlaceholderStyle,
    /// 标识符引号字符（DM 实测 `SQLGetInfo(SQL_IDENTIFIER_QUOTE_CHAR)` 返回 `"`）。
    pub quote_char: char,
    /// 被引号包起来时标识符是否大小写敏感（DM：是，§4.6）。
    pub quoted_identifier_case_sensitive: bool,
    /// 不加引号时标识符是否折叠成大写（DM：是，§4.6）。
    pub unquoted_identifier_folds_upper: bool,
    /// `INSERT` 能否 `RETURNING`（DM：否）。
    pub returning_from_insert: bool,
    /// `UPDATE`/`DELETE` 能否取受影响行数（DM：能）。
    pub row_count_from_dml: bool,
    /// 是否支持保存点。
    pub transaction_savepoint: bool,
    /// 是否支持设置隔离级别（为真时 [`Capability::isolation_levels`] 必须非空）。
    pub transaction_isolation_level: bool,
    /// 支持的隔离级别集合。
    pub isolation_levels: &'static [IsolationLevel],
    /// 是否支持流式结果（分批取行）。
    pub streaming_result: bool,
    /// 是否支持列式（数组）绑定批量写（DM：能，实测 33×，§4.7）。
    pub bulk_bind_column_wise: bool,
    /// upsert 语法族。
    pub upsert_syntax: UpsertSyntax,
    /// 自增列语法族。
    pub auto_increment_syntax: AutoIncrementSyntax,
    /// 支持的分页形态（可多种并存）。
    pub limit_styles: &'static [LimitStyle],
    /// 驱动是否内建读写分离（DM：`RW_SEPARATE` 可用，**但必须与应用层路由二选一**）。
    pub read_write_splitting_native: bool,
    /// ODBC 异步是否可用（DM：`HYC00 / -70054`，否，§4.5）。
    pub async_odbc: bool,
    /// 大 LOB 是否支持任意偏移随机读（DM 经 ODBC：否，§4.8）。
    pub blob_random_offset_read: bool,
    /// 大 LOB 是否支持流式读（DM 经 ODBC：否，§4.8）。
    pub blob_streaming_read: bool,
    /// `column_size` 的单位语义（§4.4）。
    pub column_size_unit: ColumnSizeUnit,
    /// 是否能取到**参数**的长度元数据（DM：否，`describe_param` 返回 8188，D16）。
    pub param_length_metadata: bool,
    /// INTERVAL 类型的支持档位。
    pub interval_type: SupportTier,
    /// 数组类型的支持档位。
    pub array_type: SupportTier,
    /// 表内方言取值所基于的兼容模式（**部署改了它就必须重新验证整表**）。
    pub compatible_mode: CompatMode,
}

impl Capability {
    /// 约束：读 `…_native` 与"必须二选一"这条规则的运行时守卫。
    pub fn require_native_rw_split_is_off(&self, configured: bool) -> Result<(), DmError> {
        if configured && self.read_write_splitting_native {
            return Err(DmError::at(
                DmErrorKind::Unsupported {
                    feature: "驱动内建 RW_SEPARATE 与应用层读写分离同时开启".to_string(),
                },
                Stage::Connect,
            ));
        }
        Ok(())
    }

    /// 需要「流式结果」时调用。
    pub fn require_streaming(&self) -> Result<(), DmError> {
        self.require(self.streaming_result, "流式结果集")
    }

    /// 需要「保存点」时调用。
    pub fn require_savepoint(&self) -> Result<(), DmError> {
        self.require(self.transaction_savepoint, "事务保存点")
    }

    /// 需要「取受影响行数」时调用。
    pub fn require_row_count(&self) -> Result<(), DmError> {
        self.require(self.row_count_from_dml, "DML 受影响行数")
    }

    /// 需要「`INSERT … RETURNING`」时调用。
    pub fn require_returning(&self) -> Result<(), DmError> {
        self.require(self.returning_from_insert, "INSERT ... RETURNING")
    }

    /// 需要「列式批量绑定」时调用。
    pub fn require_bulk_column_wise(&self) -> Result<(), DmError> {
        self.require(self.bulk_bind_column_wise, "列式（数组）绑定批量写")
    }

    /// 需要「ODBC 异步」时调用。
    pub fn require_async_odbc(&self) -> Result<(), DmError> {
        self.require(self.async_odbc, "ODBC 异步执行")
    }

    /// 需要「LOB 随机偏移读」时调用。
    pub fn require_blob_random_read(&self) -> Result<(), DmError> {
        self.require(self.blob_random_offset_read, "LOB 随机偏移读")
    }

    /// 需要「LOB 流式读」时调用。
    pub fn require_blob_streaming_read(&self) -> Result<(), DmError> {
        self.require(self.blob_streaming_read, "LOB 流式读")
    }

    /// 需要「参数长度元数据」时调用。
    pub fn require_param_length_metadata(&self) -> Result<(), DmError> {
        self.require(
            self.param_length_metadata,
            "参数长度元数据（describe_param）",
        )
    }

    /// 需要 upsert 时调用。
    pub fn require_upsert(&self) -> Result<(), DmError> {
        match self.upsert_syntax {
            UpsertSyntax::None => Err(self.unsupported("upsert")),
            _ => Ok(()),
        }
    }

    fn require(&self, supported: bool, feature: &str) -> Result<(), DmError> {
        if supported {
            Ok(())
        } else {
            Err(self.unsupported(feature))
        }
    }

    fn unsupported(&self, feature: &str) -> DmError {
        DmError::at(
            DmErrorKind::Unsupported {
                feature: feature.to_string(),
            },
            Stage::Other,
        )
    }

    /// 自洽性校验。表写错时**启动即失败**，而不是等到某个调用悄悄降级。
    pub fn validate(&self) -> Result<(), DmError> {
        let bad = |what: &str| {
            DmError::at(
                DmErrorKind::Config(format!("能力表自相矛盾：{what}")),
                Stage::Config,
            )
        };
        if self.transaction_isolation_level && self.isolation_levels.is_empty() {
            return Err(bad("声明支持隔离级别，但 isolation_levels 为空"));
        }
        if !self.transaction_isolation_level && !self.isolation_levels.is_empty() {
            return Err(bad("声明不支持隔离级别，但 isolation_levels 非空"));
        }
        if self.limit_styles.is_empty() {
            return Err(bad("limit_styles 为空——DM 至少支持 LIMIT"));
        }
        // D16：我们之所以依赖「声明长度」，正是因为拿不到参数长度元数据。
        if self.column_size_unit == ColumnSizeUnit::DeclaredLength && self.param_length_metadata {
            return Err(bad(
                "既声明 column_size 是声明长度，又声明能取参数长度元数据——D16 已证伪后者",
            ));
        }
        if self.sql_placeholder != PlaceholderStyle::QuestionMark {
            return Err(bad("DM 只认 `?` 占位符，其他风格必须先用 dm-query 重排"));
        }
        if self.compatible_mode != CompatMode::Dm {
            return Err(bad(
                "方言类取值只在 COMPATIBLE_MODE=0 下实测过；改兼容模式必须重新验证整张表",
            ));
        }
        Ok(())
    }

    /// 把表渲染成多行文本（`dm-core` 的启动日志与 `--explain-capability` 用）。
    pub fn render(&self) -> String {
        use core::fmt::Write as _;
        let mut s = String::new();
        let _ = writeln!(
            s,
            "sql_placeholder          = {}",
            self.sql_placeholder.as_str()
        );
        let _ = writeln!(s, "quote_char               = {:?}", self.quote_char);
        let _ = writeln!(
            s,
            "identifier               = 引号内区分大小写:{}, 不加引号折大写:{}",
            self.quoted_identifier_case_sensitive, self.unquoted_identifier_folds_upper
        );
        let _ = writeln!(
            s,
            "returning_from_insert    = {}",
            self.returning_from_insert
        );
        let _ = writeln!(s, "row_count_from_dml       = {}", self.row_count_from_dml);
        let _ = writeln!(
            s,
            "transaction_savepoint    = {}",
            self.transaction_savepoint
        );
        let levels: Vec<&str> = self.isolation_levels.iter().map(|l| l.as_str()).collect();
        let _ = writeln!(s, "isolation_levels         = {}", levels.join(" | "));
        let _ = writeln!(s, "streaming_result         = {}", self.streaming_result);
        let _ = writeln!(
            s,
            "bulk_bind_column_wise    = {}",
            self.bulk_bind_column_wise
        );
        let _ = writeln!(
            s,
            "upsert_syntax            = {}",
            self.upsert_syntax.as_str()
        );
        let _ = writeln!(
            s,
            "auto_increment_syntax    = {}",
            self.auto_increment_syntax.as_str()
        );
        let _ = writeln!(
            s,
            "limit_styles             = {} 种",
            self.limit_styles.len()
        );
        let _ = writeln!(
            s,
            "rw_split_native          = {}（与应用层路由二选一）",
            self.read_write_splitting_native
        );
        let _ = writeln!(s, "async_odbc               = {}", self.async_odbc);
        let _ = writeln!(
            s,
            "blob_random_offset_read  = {}",
            self.blob_random_offset_read
        );
        let _ = writeln!(s, "blob_streaming_read      = {}", self.blob_streaming_read);
        let _ = writeln!(s, "column_size_unit         = {:?}", self.column_size_unit);
        let _ = writeln!(
            s,
            "param_length_metadata    = {}",
            self.param_length_metadata
        );
        let _ = writeln!(
            s,
            "interval_type            = {}",
            self.interval_type.as_str()
        );
        let _ = writeln!(s, "array_type               = {}", self.array_type.as_str());
        let _ = writeln!(
            s,
            "compatible_mode          = {}",
            match self.compatible_mode {
                CompatMode::Dm => "DM(0)",
                CompatMode::Oracle => "ORACLE(1)",
            }
        );
        s
    }
}

/// 达梦 DM8 的能力表——**由 §4 的实测结论填充**（§5.10 表格的逐行落地）。
pub const DAMENG: Capability = Capability {
    sql_placeholder: PlaceholderStyle::QuestionMark,
    quote_char: '"',
    // §4.6：`"a"` 报 `-2111 无效的列名[a]`（小写列名不存在），加引号时大小写精确匹配。
    quoted_identifier_case_sensitive: true,
    // §4.6：不加引号时 `id` 折叠成 `ID`。
    unquoted_identifier_folds_upper: true,
    // §4.6：达梦无 `RETURNING`；自增走 IDENTITY/AUTO_INCREMENT。
    returning_from_insert: false,
    // §4.2：DML 可用 `dpi_row_count` / ODBC 行数。
    row_count_from_dml: true,
    transaction_savepoint: true,
    transaction_isolation_level: true,
    isolation_levels: &IsolationLevel::ALL,
    // §6.4：分批取行（`SQL_ATTR_ROW_ARRAY_SIZE`）。
    streaming_result: true,
    // §4.7：`ColumnarBulkInserter` 1000 行 3.759 ms vs 逐行 124.160 ms = 33.03×。
    bulk_bind_column_wise: true,
    // §4.6：只有 `MERGE INTO` 能用；`ON DUPLICATE KEY UPDATE` 报 `-2007`。
    upsert_syntax: UpsertSyntax::MergeInto,
    // §4.6：`IDENTITY(1,1)` 与 `AUTO_INCREMENT` 都接受（实测真的自增了 1/2）；
    // `GENERATED … AS IDENTITY` 报 `-2007`。
    auto_increment_syntax: AutoIncrementSyntax::IdentityOrAutoIncrement,
    // §4.6：`LIMIT n` / `LIMIT n OFFSET m` / `LIMIT off,cnt` 三种全通，另有 `TOP` / `ROWNUM`。
    limit_styles: &[
        LimitStyle::LimitOffset,
        LimitStyle::LimitComma,
        LimitStyle::Top,
        LimitStyle::Rownum,
    ],
    // 连接串关键字 `RW_SEPARATE` 可用，但驱动内建路由会与应用层叠加（R-12）⇒ 必须二选一。
    read_write_splitting_native: true,
    // §4.5：`SQLGetInfo(SQL_ASYNC_MODE)=0`；`SQL_ATTR_ASYNC_ENABLE` 开/关都报 `HYC00/-70054`。
    async_odbc: false,
    // §4.8：ODBC 只有顺序 `SQLGetData`，且与 `SQLBindCol` 互斥；无 `dpi_lob_read` 等价物。
    blob_random_offset_read: false,
    blob_streaming_read: false,
    // §4.4：`column_size` 报的是声明长度，单位要按 `LENGTH_IN_CHAR` 解释。
    column_size_unit: ColumnSizeUnit::DeclaredLength,
    // §4.4 / D16：`describe_param` 对 `cast(? as varchar(10))` 返回 `Varchar{length: 8188}`。
    param_length_metadata: false,
    // §14 V-09 未做 ⇒ 不承诺。
    interval_type: SupportTier::BestEffort,
    // 对象/数组/`RECORD` 明确不做（§7 表）。
    array_type: SupportTier::NotSupported,
    // 本机 `dm.ini` 的 `COMPATIBLE_MODE=0`（§4.1）。
    compatible_mode: CompatMode::Dm,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_table_is_self_consistent() {
        DAMENG.validate().expect("§5.10 的能力表必须自洽");
    }

    /// 每一行都必须与 §4 的实测结论一致——这些断言就是「表没被人手滑改掉」的守卫。
    #[test]
    fn shipped_table_matches_the_measured_baseline() {
        // 先绑到局部变量：直接对 `const` 取字段做 `assert!` 会被 clippy 判为
        // `assertions_on_constants`（常量断言），而那并不是我们想表达的语义——
        // 这里要的是「表与实测基线一致」的守卫，不是编译期恒真式。
        let c = DAMENG;
        assert_eq!(c.sql_placeholder, PlaceholderStyle::QuestionMark);
        assert_eq!(c.quote_char, '"');
        assert!(c.quoted_identifier_case_sensitive);
        assert!(c.unquoted_identifier_folds_upper);
        assert!(!c.returning_from_insert, "DM 无 RETURNING");
        assert!(c.row_count_from_dml);
        assert!(c.transaction_savepoint);
        assert!(c.streaming_result);
        assert!(c.bulk_bind_column_wise, "33.03× 已实测");
        assert_eq!(c.upsert_syntax, UpsertSyntax::MergeInto);
        assert_eq!(
            c.auto_increment_syntax,
            AutoIncrementSyntax::IdentityOrAutoIncrement
        );
        assert_eq!(c.limit_styles.len(), 4);
        assert!(c.read_write_splitting_native);
        assert!(!c.async_odbc, "HYC00/-70054");
        assert!(!c.blob_random_offset_read);
        assert!(!c.blob_streaming_read);
        assert_eq!(c.column_size_unit, ColumnSizeUnit::DeclaredLength);
        assert!(!c.param_length_metadata, "describe_param 返回 8188");
        assert_eq!(c.interval_type, SupportTier::BestEffort);
        assert_eq!(c.array_type, SupportTier::NotSupported);
        assert_eq!(c.compatible_mode, CompatMode::Dm);
    }

    #[test]
    fn isolation_levels_are_the_four_odbc_bit_values() {
        assert_eq!(DAMENG.isolation_levels.len(), 4);
        let v: Vec<i32> = DAMENG
            .isolation_levels
            .iter()
            .map(|l| l.odbc_value())
            .collect();
        assert_eq!(v, vec![1, 2, 4, 8], "ODBC 标准位值，不能改");
        assert_eq!(IsolationLevel::ReadCommitted.as_str(), "READ COMMITTED");
        assert_eq!(IsolationLevel::Serializable.to_string(), "SERIALIZABLE");
    }

    /// 未声明支持的能力必须**报错**，不能静默降级。
    #[test]
    fn unsupported_capabilities_are_refused() {
        for (r, want) in [
            (DAMENG.require_async_odbc(), "ODBC 异步执行"),
            (DAMENG.require_blob_random_read(), "LOB 随机偏移读"),
            (DAMENG.require_blob_streaming_read(), "LOB 流式读"),
            (
                DAMENG.require_param_length_metadata(),
                "参数长度元数据（describe_param）",
            ),
            (DAMENG.require_returning(), "INSERT ... RETURNING"),
        ] {
            let e = r.expect_err(want);
            match &e.kind {
                DmErrorKind::Unsupported { feature } => assert_eq!(feature, want),
                other => panic!("期望 Unsupported，实得 {other:?}"),
            }
        }
    }

    /// 已声明支持的能力不报错。
    #[test]
    fn supported_capabilities_pass() {
        DAMENG.require_streaming().unwrap();
        DAMENG.require_savepoint().unwrap();
        DAMENG.require_row_count().unwrap();
        DAMENG.require_bulk_column_wise().unwrap();
        DAMENG.require_upsert().unwrap();
    }

    /// `RW_SEPARATE` 与应用层路由同时开必须被拒绝（R-12）。
    #[test]
    fn native_rw_split_and_app_routing_are_mutually_exclusive() {
        DAMENG.require_native_rw_split_is_off(false).unwrap();
        let e = DAMENG.require_native_rw_split_is_off(true).unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Unsupported { .. }));
        assert!(e.to_string().contains("RW_SEPARATE"));
    }

    /// `validate()` 必须真的能抓到错表。
    #[test]
    fn validate_rejects_inconsistent_tables() {
        let mut c = DAMENG;
        c.isolation_levels = &[];
        assert!(c.validate().is_err(), "声明支持隔离级别却给空表");

        let mut c = DAMENG;
        c.transaction_isolation_level = false;
        assert!(c.validate().is_err(), "声明不支持却给了非空表");

        let mut c = DAMENG;
        c.limit_styles = &[];
        assert!(c.validate().is_err(), "分页风格不能为空");

        let mut c = DAMENG;
        c.param_length_metadata = true;
        assert!(c.validate().is_err(), "与 D16 冲突");

        let mut c = DAMENG;
        c.sql_placeholder = PlaceholderStyle::DollarNumbered;
        assert!(c.validate().is_err(), "DM 只认 ?");

        let mut c = DAMENG;
        c.compatible_mode = CompatMode::Oracle;
        assert!(c.validate().is_err(), "Oracle 兼容模式下整表需重验");
    }

    #[test]
    fn render_mentions_every_fact() {
        let s = DAMENG.render();
        for needle in [
            "sql_placeholder",
            "quote_char",
            "upsert_syntax",
            "MERGE INTO",
            "column_size_unit",
            "param_length_metadata",
            "compatible_mode",
            "DM(0)",
        ] {
            assert!(s.contains(needle), "缺少 {needle}：\n{s}");
        }
    }

    #[test]
    fn tier_ordering_and_names() {
        assert!(SupportTier::Guaranteed.is_guaranteed());
        assert!(!SupportTier::BestEffort.is_guaranteed());
        assert!(SupportTier::Guaranteed < SupportTier::BestEffort);
        assert!(SupportTier::BestEffort < SupportTier::NotSupported);
        assert_eq!(SupportTier::NotSupported.as_str(), "not-supported");
    }

    #[test]
    fn syntax_names_are_the_ones_users_type() {
        assert_eq!(UpsertSyntax::MergeInto.as_str(), "MERGE INTO");
        assert_eq!(
            AutoIncrementSyntax::IdentityOrAutoIncrement.as_str(),
            "IDENTITY(1,1) / AUTO_INCREMENT"
        );
        assert_eq!(PlaceholderStyle::DollarNumbered.as_str(), "$1");
        assert_eq!(UpsertSyntax::None.as_str(), "none");
        assert_eq!(AutoIncrementSyntax::None.as_str(), "none");
    }
}
