//! # dm-migrate —— 迁移执行入口
//!
//! 计划提供（设计方案 §6.11）：脚本发现与排序（`V{version}__{name}.sql`）、
//! 版本记录表 `DM_SCHEMA_HISTORY`（用 `SELECT 1 FROM … WHERE 1 = 0` 探存在性
//! ——达梦没有 `CREATE TABLE IF NOT EXISTS`）、校验和、失败即停。
//!
//! # 定位：**不是**完整迁移框架（§6.11）
//!
//! 本 crate 只做「**按序执行已经写好的 SQL 脚本并记账**」这一件事：
//!
//! - **不做 DDL 生成**：没有「从实体反推建表语句」这回事。要那种能力用
//!   Toasty（`toasty-cli` + `__toasty_migrations`），两者**互不替代**：
//!   `dm-migrate` 服务于不用 Toasty 的用户，表名与记录格式都不重叠（§6.11 末段）。
//! - **回滚脚本 `U{version}__{name}.sql` 是可选能力，当前未实现**。本 crate
//!   不假装支持 `down`：`discover` 只认 `V` 前缀，`U` 开头的文件会被**静默忽略**。
//!   想要「可回滚」的用户目前得自己把补偿语句当成一个新版本写进去。
//! - **没有锁**：不做迁移互斥（多个进程同时 `apply_pending` 时，后到的那个会在
//!   主键冲突上失败）。单机单部署下够用，多副本部署请自己在外部串行化。
//!
//! # 记账语义（§6.11）：「失败即停」**不是**「整体事务」
//!
//! [`Migrator::apply_pending`] 逐个脚本处理，**每个脚本与它的版本记录行在同一个
//! 事务里**（要么都生效、要么都不生效）；但只要某个脚本失败，函数立刻返回错误、
//! **已经提交过的前几个脚本不回滚**。这是有意的：迁移的语义是「向前推进」，
//! 把已经跑过的 DDL 回滚掉往往做不到（达梦的 DDL 不支持事务回滚）。
//! 所以返回值与错误信息都在告诉你**推进到了哪一步**，好让你手工收尾。
//!
//! # 连接从哪来
//!
//! [`Migrator`] 对后端泛型（`B: Backend`），内部持 `Arc<DmPool<B>>`：
//!
//! - 每个方法自己 [`PooledConn::acquire`]——**方法之间不持有连接**，所以
//!   `Migrator` 可以长期存活，也不会因为忘了归还而把池饿死；
//! - `Arc` 让 `Migrator` 能 [`Clone`]（迁移入口经常要在几个任务间传来传去）；
//! - 池必须由调用方在 **tokio 运行时上下文**里建好（`DmPool` 的硬约束），
//!   本 crate 不替调用方建池——连接串/池大小是应用的事，不是迁移的事。
//!
//! # 脚本文件是**同步读**的，这是有意的
//!
//! `std::fs::read_to_string` 会阻塞当前线程。迁移脚本是**人写的、一次性、几十 KB**
//! 的文件，为了它引入 `tokio::fs` 会把整个 crate 拖上 `tokio` 的依赖（本项目的
//! 「能不加依赖就不加」）。代价写在明面上：**一次 `apply_pending` 会短时间阻塞
//! 一个 tokio 工作线程**，脚本目录放在本地盘（而不是网络盘）时无感。
//!
//! # 校验和
//!
//! 每个脚本算一个 64 位 FNV-1a 校验和（16 位十六进制小写），进记录表。
//! **自己实现而不是引第三方哈希库**：这里只要「同一内容恒定、内容变了值就变」，
//! 不做密码学用途——FNV-1a 是十几行代码，不值得一个依赖。
//! 校验和的用法是**提醒**而不是**拦截**：脚本内容变了（说明有人改了已应用的迁移）
//! 会在 `tracing::warn!` 里报出来。

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dm_core::{Backend, DmValue};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
use dm_pool::DmPool;
use dm_query::{PooledConn, Transaction};

mod sql;

/// 版本记录表的默认表名（§6.11 / 占位头）。
///
/// 与 Toasty 的 `__toasty_migrations` 刻意不同名：两者的记录格式不兼容，
/// 同名会让人以为可以混用。
pub const HISTORY_TABLE: &str = "DM_SCHEMA_HISTORY";

/// 已应用的一条迁移记录（版本记录表 `DM_SCHEMA_HISTORY` 的一行）。
///
/// 字段类型选得**保守**：`APPLIED_AT` 用 `String` 而不是 `chrono::NaiveDateTime`，
/// 因为「记录表里那一列到底是什么类型」是实现细节（本 crate 建的是
/// `TIMESTAMP(6)`，但用户可能先手工建了表、或换成了 `VARCHAR`），
/// 而 `String` 对 `TIMESTAMP` 与 `VARCHAR` **两种都能解出来**
/// （`ValueRef::as_text` 会把 `Timestamp` 格式化，见 `dm-core/src/value.rs:658`）。
/// 反过来，写死 `NaiveDateTime` 会在「用户表是 `VARCHAR`」时直接报解码错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    /// 版本号（脚本名里 `V` 与 `__` 之间的那一段，原样保留，如 `"1"`、`"20260101"`）。
    pub version: String,
    /// 迁移名（`__` 与 `.sql` 之间的那一段，如 `"create_user"`）。
    pub name: String,
    /// 脚本内容的 64 位 FNV-1a 校验和（16 位十六进制小写）。
    pub checksum: String,
    /// 应用时间。格式由记录表的列类型决定（本 crate 建的表是
    /// `YYYY-MM-DD HH:MM:SS.ffffff`），**不做解析**。
    pub applied_at: String,
}

/// 发现到的一个迁移脚本（`V{version}__{name}.sql`）。
///
/// 它是「磁盘上的一个文件」的投影：`version`/`name` 已经从文件名里解出来了，
/// `checksum` 也已经读出来算好了，所以 `apply_pending` 不用再碰一次文件名解析。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    /// 版本号（`V` 与 `__` 之间，原样保留）。
    pub version: String,
    /// 迁移名（`__` 与 `.sql` 之间）。
    pub name: String,
    /// 脚本路径。
    pub path: PathBuf,
    /// 脚本内容的 64 位 FNV-1a 校验和（16 位十六进制小写）。
    pub checksum: String,
}

/// 迁移版本记录表的列名。
///
/// 集中成常量而不是散在 SQL 字符串里：列名同时出现在 DDL、`INSERT`、`SELECT`
/// 三处，改名时漏掉一处就是运行期才发现的错。
mod cols {
    /// 版本号列（主键）。
    pub(super) const VERSION: &str = "VERSION";
    /// 迁移名列。
    pub(super) const NAME: &str = "NAME";
    /// 校验和列。
    pub(super) const CHECKSUM: &str = "CHECKSUM";
    /// 应用时间列。
    pub(super) const APPLIED_AT: &str = "APPLIED_AT";
}

/// 迁移执行入口（设计方案 §6.11）。
///
/// 泛型参数 `B` 是后端（默认达梦 ODBC）。选泛型而不是写死 `OdbcBackend`：
/// `dm-pool`/`dm-query` 本来就是后端泛型的，写死会让本 crate 无法用假后端做
/// 离线测试（与 `DmPool<B: Backend = OdbcBackend>` 的既有约定一致）。
///
/// # 例子
///
/// ```no_run
/// # use std::path::Path;
/// # use std::sync::Arc;
/// # use dm_migrate::Migrator;
/// # use dm_core::DmConfig;
/// # use dm_pool::{DmPool, ExecutorConfig};
/// # use dm_odbc::OdbcBackend;
/// # async fn demo() -> Result<(), dm_error::DmError> {
/// let cfg = DmConfig::parse("DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA")?;
/// let pool = Arc::new(DmPool::<OdbcBackend>::connect_with(cfg, ExecutorConfig::for_pool(2)).await?);
///
/// let m = Migrator::new(Arc::clone(&pool));
/// m.ensure_table().await?;                       // 幂等
/// m.apply_pending(Path::new("migrations")).await?;
///
/// for rec in m.applied().await? {
///     println!("{} {} {}", rec.version, rec.name, rec.applied_at);
/// }
///
/// pool.close().await?;
/// pool.wait_closed().await;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Migrator<B: Backend = dm_pool::DefaultBackend> {
    pool: Arc<DmPool<B>>,
    history_table: String,
}

impl<B: Backend> fmt::Debug for Migrator<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Migrator")
            .field("history_table", &self.history_table)
            .field("pool_state", &self.pool.state())
            .finish()
    }
}

impl<B: Backend> Migrator<B> {
    /// 用一个**已经建好**的池构造迁移入口（表名取 [`HISTORY_TABLE`]）。
    ///
    /// 池用 `Arc` 传：`Migrator` 自己 `Clone`，且方法之间不持有连接，
    /// 所以同一个池上可以并存多个 `Migrator`（比如读 `applied()` 的同时跑
    /// `apply_pending`——但**不建议**，见 crate 文档「没有锁」那条）。
    #[must_use]
    pub fn new(pool: Arc<DmPool<B>>) -> Self {
        Self {
            pool,
            history_table: HISTORY_TABLE.to_owned(),
        }
    }

    /// 换一个版本记录表名（默认 [`HISTORY_TABLE`]）。
    ///
    /// 名字会被拼进 DDL/DML（表名没法当绑定参数），所以这里**白名单校验**
    /// 标识符（`[A-Za-z_][A-Za-z0-9_]*`，最长 128 字节），校验不过返回
    /// [`DmErrorKind::Config`]——不校验就等于把 SQL 注入的口子开在表名上
    /// （同 `dm-query` 的保存点名处理，见 `crates/dm-query/src/txn.rs:194`）。
    ///
    /// # Errors
    ///
    /// 名字不是合法标识符 → [`DmErrorKind::Config`]。
    pub fn with_history_table(mut self, name: &str) -> Result<Self, DmError> {
        checked_identifier(name, "版本记录表名")?;
        self.history_table = name.to_owned();
        Ok(self)
    }

    /// 当前使用的版本记录表名。
    #[must_use]
    pub fn history_table(&self) -> &str {
        &self.history_table
    }

    /// 底层连接池（想拿 [`dm_pool::PoolStats`] 或关池时用）。
    #[must_use]
    pub fn pool(&self) -> &Arc<DmPool<B>> {
        &self.pool
    }

    // -----------------------------------------------------------------------
    // 发现与排序（纯文件系统操作，不连库）
    // -----------------------------------------------------------------------

    /// 扫描 `dir` 下的 `V{version}__{name}.sql`，按**版本序**返回（不是字典序）。
    ///
    /// # 排序规则（§6.11 / 用户要求「`V10` 要在 `V9` 之后」）
    ///
    /// 版本号按 `.` 切成若干段，**逐段按数值**比较；段数不同时短的前面
    /// （`1.2` < `1.2.3`）。所以 `V9` < `V10`，`V1.9` < `V1.10`。
    /// 版本号相同（重复版本）时报错——静默取一个会让「到底跑了哪个」没有答案。
    ///
    /// 非 `V…__….sql` 形态的文件（含回滚脚本 `U…`、`README.md`、`.bak`）**静默忽略**。
    ///
    /// # Errors
    ///
    /// - 目录读不了 → [`DmErrorKind::Config`]；
    /// - 版本段不是数字、名字为空、或出现重复版本 → [`DmErrorKind::Config`]。
    pub fn discover(dir: &Path) -> Result<Vec<MigrationFile>, DmError> {
        let entries = std::fs::read_dir(dir).map_err(|e| {
            DmError::at(
                DmErrorKind::Config(format!("读迁移目录 {} 失败：{e}", display_path(dir))),
                Stage::Config,
            )
        })?;

        let mut found: Vec<MigrationFile> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| {
                DmError::at(
                    DmErrorKind::Config(format!(
                        "读迁移目录 {} 中的一项失败：{e}",
                        display_path(dir)
                    )),
                    Stage::Config,
                )
            })?;
            // 只要普通文件：目录名叫 `V1__x.sql` 时不该被当成脚本。
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_file() {
                continue;
            }
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue; // 文件名不是 UTF-8：不可能是我们认识的脚本名。
            };
            let Some((version, name)) = parse_script_name(file_name) else {
                tracing::debug!(file = %path.display(), "不是 V{{version}}__{{name}}.sql，已忽略");
                continue;
            };
            let checksum = checksum_of_file(&path)?;
            found.push(MigrationFile {
                version,
                name,
                path,
                checksum,
            });
        }

        // 逐段数值比较；同版本时用名字做稳定的次序，保证结果可复现。
        found.sort_by(|a, b| {
            version_key(&a.version)
                .cmp(&version_key(&b.version))
                .then_with(|| a.name.cmp(&b.name))
        });

        for pair in found.windows(2) {
            let (prev, next) = (&pair[0], &pair[1]);
            if prev.version == next.version {
                return Err(DmError::at(
                    DmErrorKind::Config(format!(
                        "迁移版本 {} 重复：{} 与 {}（版本号必须唯一，否则「跑过哪个」没有答案）",
                        prev.version,
                        display_path(&prev.path),
                        display_path(&next.path)
                    )),
                    Stage::Config,
                ));
            }
        }

        Ok(found)
    }

    /// 「已发现的脚本里哪些还没应用」（`discover` + `applied` 的差集），按版本序。
    ///
    /// 想「先看一眼再决定跑不跑」的调用方用这个；`apply_pending` 内部走的也是同一条路。
    ///
    /// # Errors
    ///
    /// 同 [`Migrator::discover`] 与 [`Migrator::applied`]。
    pub async fn pending(&self, dir: &Path) -> Result<Vec<MigrationFile>, DmError> {
        let all = Self::discover(dir)?;
        let applied = self.applied().await?;
        Ok(all
            .into_iter()
            .filter(|f| !applied.iter().any(|a| a.version == f.version))
            .collect())
    }

    // -----------------------------------------------------------------------
    // 版本记录表
    // -----------------------------------------------------------------------

    /// 建版本记录表（**幂等**：表已存在时成功返回）。
    ///
    /// # 为什么这么建（§6.11 / 占位头）
    ///
    /// 达梦**没有 `CREATE TABLE IF NOT EXISTS`**（M6 实测），所以先探一次：
    /// `SELECT 1 FROM {表} WHERE 1 = 0`。这条语句在表存在时不返回任何行、也不报错；
    /// 表不存在时服务端报 `ObjectNotFound`-类错误（达梦是 `HY000 / -2106
    /// 「无效的表或视图名」`，本项目错误分类翻成 [`DmErrorKind::ObjectNotFound`]）。
    /// 靠**错误分类**而不是靠「有没有行」判断，是因为「有行」根本不会发生。
    ///
    /// 表结构（见下）为什么这么定：
    ///
    /// | 列 | 类型 | 理由 |
    /// |---|---|---|
    /// | `VERSION` | `VARCHAR(64)` | 主键。字符串而不是整数：版本号允许 `1.2.3` 这种分段形式，而且**前导零**（`V007`）在字符串里保真 |
    /// | `NAME` | `VARCHAR(200)` | 校验和/记录展示用。`varchar(n)` 按**字节**计量（库默认 `LENGTH_IN_CHAR=0`），200 字节够放英文名并留出余量 |
    /// | `CHECKSUM` | `VARCHAR(64)` | 现在是 16 位十六进制；留宽是为了以后换算法不用改表 |
    /// | `APPLIED_AT` | `TIMESTAMP(6) DEFAULT SYSDATE` | 由**服务端**填时间：应用进程的时钟可能不对，而且这样 Rust 侧不需要格式化时间、也不需要 `chrono` 依赖 |
    ///
    /// # 关于 DDL 的事务语义
    ///
    /// 达梦的 `CREATE TABLE` 是**隐式提交**的（DDL 不参与事务回滚），所以这里
    /// 不开事务：开了也回不去。与 `apply_pending` 的脚本事务是两回事。
    ///
    /// # Errors
    ///
    /// 探测或建表失败（权限不足、表名非法、连接断了等）时原样上报。
    /// 探测得到的非 `ObjectNotFound` 错误**不会**被吞掉——「探不了」不等于
    /// 「不存在」，硬建表只会把真正的错（比如没权限）盖成一句「表已存在」。
    pub async fn ensure_table(&self) -> Result<(), DmError> {
        let mut conn = self.acquire().await?;
        self.ensure_table_on(&mut conn).await
    }

    /// [`Migrator::ensure_table`] 的「借用已有连接」版本。
    ///
    /// 借用而不是按值拿走：探测用的 `SELECT 1 FROM … WHERE 1 = 0` 与建表语句
    /// 都要走 `&mut PooledConn`，调用方（`apply_pending`）借完还要接着用这条连接
    /// 读一次已应用列表。
    async fn ensure_table_on(&self, conn: &mut PooledConn<B>) -> Result<(), DmError> {
        let probe = format!("SELECT 1 FROM {} WHERE 1 = 0", self.history_table);
        match conn.query(&probe, ()).await {
            Ok(_) => {
                tracing::debug!(table = %self.history_table, "版本记录表已存在");
                Ok(())
            }
            Err(err) if matches!(err.kind, DmErrorKind::ObjectNotFound) => {
                tracing::info!(table = %self.history_table, "版本记录表不存在，开始建表");
                conn.execute(&self.create_table_sql(), ()).await?;
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// 建表 DDL 原文。
    ///
    /// 私有而不是公开：用户问「你到底建了什么表」时，答案应当从
    /// [`Migrator::ensure_table`] 的文档与 `tracing` 日志里拿到；
    /// 把它做成公开 API 等于承诺这段 DDL 的文本形式永不改变。
    fn create_table_sql(&self) -> String {
        format!(
            "CREATE TABLE {} (\
                 {} VARCHAR(64) NOT NULL PRIMARY KEY, \
                 {} VARCHAR(200) NOT NULL, \
                 {} VARCHAR(64) NOT NULL, \
                 {} TIMESTAMP(6) DEFAULT SYSDATE NOT NULL\
             )",
            self.history_table,
            cols::VERSION,
            cols::NAME,
            cols::CHECKSUM,
            cols::APPLIED_AT
        )
    }

    /// 已应用的迁移记录，按**应用顺序**（`APPLIED_AT`，同刻按版本号兜底）。
    ///
    /// 不按版本号排：记录表的用途之一是「这台机器实际是按什么顺序跑过来的」，
    /// 而手工补跑过的脚本很可能版本号与时间不一致。
    ///
    /// # Errors
    ///
    /// 表不存在（先调 [`Migrator::ensure_table`]）或结果集解码失败时上报。
    pub async fn applied(&self) -> Result<Vec<AppliedMigration>, DmError> {
        let mut conn = self.acquire().await?;
        self.applied_on(&mut conn).await
    }

    /// [`Migrator::applied`] 的「借用已有连接」版本。
    async fn applied_on(&self, conn: &mut PooledConn<B>) -> Result<Vec<AppliedMigration>, DmError> {
        let sql = format!(
            "SELECT {}, {}, {}, {} FROM {} ORDER BY {}, {}",
            cols::VERSION,
            cols::NAME,
            cols::CHECKSUM,
            cols::APPLIED_AT,
            self.history_table,
            cols::APPLIED_AT,
            cols::VERSION
        );
        let set = conn.query(&sql, ()).await?;
        let mut out = Vec::with_capacity(set.len());
        for row in set.rows() {
            out.push(AppliedMigration {
                version: row.get::<String>(cols::VERSION)?,
                name: row.get::<String>(cols::NAME)?,
                checksum: row.get::<String>(cols::CHECKSUM)?,
                applied_at: row.get::<String>(cols::APPLIED_AT)?,
            });
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // 执行
    // -----------------------------------------------------------------------

    /// 执行**一个** SQL 脚本（按分号切分后逐条跑）。
    ///
    /// # 事务语义：**逐条自动提交**，不是一个事务
    ///
    /// 这条路径对应「我就想跑一段 SQL」的用法，所以不包事务：每条语句各自提交，
    /// 中途失败时**前面的语句已经生效**。错误信息里带上是第几条语句失败的
    /// （`DiagRecord`），因为脚本里的行号与实际执行位置并不一致。
    /// 要「整脚本原子」请用 [`Migrator::apply_pending`]（它把脚本与记录行放同一个事务）。
    ///
    /// 脚本里的注释原样发给服务端（切分器不改写文本），所以报错行号还能对得上。
    ///
    /// # Errors
    ///
    /// - 文件读不了 → [`DmErrorKind::Config`]；
    /// - 切分出 0 条语句（空文件/只有注释）→ **成功返回**（空脚本不是错误）；
    /// - 语句执行失败 → 原样上报 + 一条「第 N 条语句」诊断。
    pub async fn run_sql_file(&self, path: &Path) -> Result<(), DmError> {
        let mut conn = self.acquire().await?;
        let script = read_script(path)?;
        run_script_on(&mut conn, path, &script).await
    }

    /// 把 `dir` 里**尚未应用**的脚本按版本序逐个执行，返回成功应用的条数。
    ///
    /// # 语义（用户要求 + §6.11）
    ///
    /// - 先 [`Migrator::ensure_table`]（幂等），再 `discover` + `applied` 求差集；
    /// - 每个脚本 + 它的记录行在**同一个事务**里：语句跑完再 `INSERT`，一起 `COMMIT`
    ///   （做不到的只有一件事：脚本里的 DDL 在达梦上会**隐式提交**，所以「脚本失败时
    ///   前半段的 DDL 也回不去」。这是达梦的 DDL 事务语义，不是本 crate 的选择）；
    /// - **失败即停**：某个脚本失败就 `ROLLBACK` 它、立刻返回错误，**已提交的不回滚**；
    /// - 第二次调用返回 `0`（已记录的全部跳过）。
    ///
    /// # 校验和只提醒不拦截
    ///
    /// 已经应用过的脚本内容变了（有人改了历史迁移），会在 `tracing::warn!` 里报出
    /// 版本号与两个校验和，但**不**中止——中止会让「改了注释想重新跑一次」变成
    /// 不可能，而真正的护栏是「已应用的脚本不该改」这条纪律，不是这里的检查。
    ///
    /// # Errors
    ///
    /// - 目录/版本号问题 → 同 [`Migrator::discover`]；
    /// - 建表、读记录失败 → 原样上报；
    /// - 某个脚本失败 → 原样上报 + 带版本/路径/第几条语句的诊断。
    pub async fn apply_pending(&self, dir: &Path) -> Result<usize, DmError> {
        let scripts = Self::discover(dir)?;

        // 先把表准备好，再取一次「已应用」快照：这一步只是为了**日志**（校验和漂移）
        // 与提前暴露「表建不起来」这类错误，真正的跳过判定在每个脚本自己的连接上重做
        // （见 `apply_one`），所以这里读到的快照过期不会造成重复执行。
        let mut probe_conn = self.acquire().await?;
        self.ensure_table_on(&mut probe_conn).await?;
        let snapshot = self.applied_on(&mut probe_conn).await?;
        drop(probe_conn);
        warn_on_checksum_drift(&scripts, &snapshot);

        let mut done = 0usize;
        for script in &scripts {
            // 每个脚本自己借一条连接、自己（在事务开始前）判断是否已应用：
            // 顺序执行时上一次的提交对本次可见，而「判断」与「提交」在同一个
            // 借出周期内，不会被另一个进程插入到中间。
            let mut conn = self.acquire().await?;
            if self
                .applied_on(&mut conn)
                .await?
                .iter()
                .any(|a| a.version == script.version)
            {
                tracing::debug!(
                    version = %script.version,
                    file = %script.path.display(),
                    "已应用，跳过"
                );
                continue;
            }
            self.apply_one(conn, script).await?;
            done += 1;
            tracing::info!(
                version = %script.version,
                name = %script.name,
                file = %script.path.display(),
                "迁移已应用"
            );
        }
        Ok(done)
    }

    /// 执行一个脚本 + 写记录行，两者同一个事务。
    ///
    /// **按值**拿走连接（`Transaction::begin` 的签名就是 `self`）：事务期间这条连接
    /// 被 `Transaction` 独占，所以每个脚本借一条、用完即还，事务的生命周期只在这一个
    /// 脚本内——而不是整轮迁移共用一个越滚越长的事务句柄。
    async fn apply_one(&self, conn: PooledConn<B>, script: &MigrationFile) -> Result<(), DmError> {
        let script_text = read_script(&script.path)?;
        let statements = sql::split_statements(&script_text);

        // 事务对象 `Drop` 时会把连接标脏交给池回滚，所以这里显式拿到两种结局。
        let mut txn = conn.begin().await?;
        match run_script_in_txn(&mut txn, &script.path, &statements).await {
            Ok(()) => {}
            Err(err) => {
                // 回滚失败（多半是连接已经断了）不该盖掉真正的错：把回滚错误当诊断附上。
                if let Err(rb) = txn.rollback().await {
                    return Err(with_diag(
                        err,
                        format!("迁移 {} 失败后回滚也失败：{rb}", script.version),
                    ));
                }
                return Err(err);
            }
        }

        if let Err(err) = self
            .insert_history(&mut txn, script, statements.len())
            .await
        {
            txn.rollback().await?;
            return Err(err);
        }
        txn.commit().await
    }

    /// 插一行版本记录（在传入的事务里）。
    async fn insert_history(
        &self,
        txn: &mut Transaction<B>,
        script: &MigrationFile,
        statement_count: usize,
    ) -> Result<(), DmError> {
        let sql = format!(
            "INSERT INTO {} ({}, {}, {}, {}) VALUES (?, ?, ?, CURRENT_TIMESTAMP)",
            self.history_table,
            cols::VERSION,
            cols::NAME,
            cols::CHECKSUM,
            cols::APPLIED_AT
        );
        let args = vec![
            DmValue::Text(script.version.clone()),
            DmValue::Text(script.name.clone()),
            DmValue::Text(script.checksum.clone()),
        ];
        txn.execute(&sql, args).await.map_err(|err| {
            with_diag(
                err,
                format!(
                    "写版本记录失败（{} {}，{} 条语句已在本事务内执行）",
                    script.version, script.name, statement_count
                ),
            )
        })?;
        Ok(())
    }

    /// 借一条连接（把「借不到」的错误补一句上下文）。
    async fn acquire(&self) -> Result<PooledConn<B>, DmError> {
        PooledConn::acquire(&self.pool).await.map_err(|err| {
            with_diag(
                err,
                format!("从池里借连接失败（池状态 {:?}）", self.pool.state()),
            )
        })
    }
}

// ---------------------------------------------------------------------------
// 内部辅助
// ---------------------------------------------------------------------------

/// 在一个**没有事务**的连接上逐条执行脚本里的语句。
async fn run_script_on<B: Backend>(
    conn: &mut PooledConn<B>,
    path: &Path,
    script: &str,
) -> Result<(), DmError> {
    let statements = sql::split_statements(script);
    if statements.is_empty() {
        tracing::debug!(file = %path.display(), "脚本里没有可执行语句（空文件或只有注释）");
        return Ok(());
    }
    tracing::debug!(
        file = %path.display(),
        statements = statements.len(),
        "开始执行脚本"
    );
    for (i, stmt) in statements.iter().enumerate() {
        if let Err(err) = conn.execute(&stmt.sql, ()).await {
            return Err(statement_error(
                err,
                path,
                i + 1,
                statements.len(),
                &stmt.sql,
            ));
        }
    }
    Ok(())
}

/// 在一个**事务**上逐条执行脚本里的语句。
async fn run_script_in_txn<B: Backend>(
    txn: &mut Transaction<B>,
    path: &Path,
    statements: &[sql::Statement],
) -> Result<(), DmError> {
    if statements.is_empty() {
        return Ok(());
    }
    for (i, stmt) in statements.iter().enumerate() {
        if let Err(err) = txn.execute(&stmt.sql, ()).await {
            return Err(statement_error(
                err,
                path,
                i + 1,
                statements.len(),
                &stmt.sql,
            ));
        }
    }
    Ok(())
}

/// 给「第 N 条语句失败」补上定位信息（脚本路径 + 序号 + 语句头 80 个字符）。
///
/// 只加诊断、**不改 `kind`**：`kind` 是调用方做重试判定的依据，动它等于骗人。
fn statement_error(err: DmError, path: &Path, index: usize, total: usize, sql: &str) -> DmError {
    with_diag(
        err,
        format!(
            "脚本 {} 的第 {index}/{total} 条语句失败，语句开头：{}",
            display_path(path),
            head_of(sql, 80)
        ),
    )
}

/// 附一条诊断记录（`DmError` 的 `DiagRecord` 是只增的，见 `dm-error`）。
fn with_diag(mut err: DmError, message: String) -> DmError {
    err.push_diag(DiagRecord::native(0, message));
    err
}

/// 取字符串的前若干个**字符**（不是字节：按字节切会把多字节汉字切成半截）。
///
/// 只有真的截断了才加省略号：`head_of("select 1")` 就是 `select 1` 本身，
/// 不加尾巴——诊断信息里多一个「（已省略后续内容）」会让人以为后面还有东西。
fn head_of(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('…');
        out.push_str("（已省略后续内容）");
    }
    out
}

/// 读脚本文件（见 crate 文档「同步读是有意的」）。
fn read_script(path: &Path) -> Result<String, DmError> {
    std::fs::read_to_string(path).map_err(|e| {
        DmError::at(
            DmErrorKind::Config(format!("读脚本 {} 失败：{e}", display_path(path))),
            Stage::Config,
        )
    })
}

/// 算文件的 FNV-1a 校验和并读成脚本。
fn checksum_of_file(path: &Path) -> Result<String, DmError> {
    let content = read_script(path)?;
    Ok(fnv1a_hex(content.as_bytes()))
}

/// 64 位 FNV-1a，输出 16 位小写十六进制。
///
/// 选它的理由：常量是标准值（不自己编）、十几行、结果稳定可复现。
/// **不做密码学用途**——这里只要「内容变则值变」。
fn fnv1a_hex(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// 把版本号切成分段数值键（`"1.10"` → `[1, 10]`）。
///
/// 解析不了的那段当 `0` 处理而不是报错：排序是**执行顺序**的事，
/// 而「版本号是不是合法数字」在 `parse_script_name` 里已经拦过了。
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|seg| seg.parse::<u64>().unwrap_or(0))
        .collect()
}

/// 解析 `V{version}__{name}.sql`，不像这个形态就返回 `None`。
///
/// 形态要求（比占位头严格一点，理由是「宽松的解析会把乱七八糟的文件当成迁移」）：
///
/// - 以 `V` 开头、以 `.sql` 结尾（**大小写敏感**：`v1__x.sql` 会被忽略，
///   因为 Windows 的文件名大小写不敏感，宽松匹配会让 `V1__x.SQL` 与 `v1__x.sql`
///   变成两个不同的脚本或一个重复版本，两种都不好）；
/// - `V` 与 `__` 之间是**非空、全数字、以 `.` 分段**的版本号；
/// - `__` 与 `.sql` 之间是非空名字。
fn parse_script_name(file_name: &str) -> Option<(String, String)> {
    let stem = file_name.strip_suffix(".sql")?;
    let rest = stem.strip_prefix('V')?;
    // 用 `split_once` 而不是 `split("__")`：名字里允许再出现 `__`（`V1__a__b.sql`
    // 的名字就是 `a__b`），只有**第一次**出现的 `__` 是分隔符。
    let (version, name) = rest.split_once("__")?;
    if version.is_empty() || name.is_empty() {
        return None;
    }
    if !version.split('.').all(is_decimal_segment) {
        return None;
    }
    Some((version.to_owned(), name.to_owned()))
}

/// 版本号的一段是否全数字（非空）。
///
/// 要求全数字的理由：`V1a__x.sql` 这种名字里的「版本」没法按数值排序，
/// 静默按字符串排会得到 `V10 < V9` 那种反直觉顺序（正是用户点名的坑）。
fn is_decimal_segment(seg: &str) -> bool {
    !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit())
}

/// 校验一个会被拼进 SQL 的标识符：`[A-Za-z_][A-Za-z0-9_]*`，≤ 128 字节。
fn checked_identifier(name: &str, what: &str) -> Result<(), DmError> {
    let valid_len = !name.is_empty() && name.len() <= 128;
    let mut chars = name.chars();
    let head_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let tail_ok = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid_len && head_ok && tail_ok {
        return Ok(());
    }
    Err(DmError::at(
        DmErrorKind::Config(format!(
            "{what} {name:?} 不是合法的标识符：只允许 [A-Za-z_][A-Za-z0-9_]*，且不超过 128 字节\
             （名字会被拼进 SQL，必须白名单校验）"
        )),
        Stage::Config,
    ))
}

/// 已应用的脚本内容与磁盘上不一致时告警（不中止，见 [`Migrator::apply_pending`]）。
fn warn_on_checksum_drift(scripts: &[MigrationFile], applied: &[AppliedMigration]) {
    for script in scripts {
        // 收成 let-chain（edition 2024 支持）：层叠 `if let` + `if` 会被
        // `clippy::collapsible_if` 判为可合并。
        if let Some(rec) = applied.iter().find(|a| a.version == script.version)
            && rec.checksum != script.checksum
        {
            tracing::warn!(
                version = %script.version,
                applied = %rec.checksum,
                on_disk = %script.checksum,
                file = %script.path.display(),
                "已应用的迁移内容与磁盘上不一致：通常意味着有人改了历史脚本。\
                 本次不会重跑（已记录即跳过），但请核对哪一份才是对的"
            );
        }
    }
}

/// 路径显示：非 UTF-8 路径也要能打印出来（`Path::display` 会做有损替换）。
fn display_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个本进程独占、用完即删的临时目录。
    ///
    /// **实测**：这台机器上 `std::env::temp_dir()`（`%TEMP%`）对测试进程**不可写**——
    /// `create_dir_all` 报 `Os { code: 5, kind: PermissionDenied }`。所以按顺序试几个
    /// 候选根目录，谁先成功用谁：cargo 给测试指派的 `CARGO_TARGET_TMPDIR`、工作区的
    /// `target\tmp`、最后才退回 `%TEMP%`。名字里带 `std::process::id()` 保证互不打架，
    /// 调用方负责删（见各用例末尾的 `remove_dir_all`）。
    fn scratch_dir(tag: &str) -> PathBuf {
        let name = format!("dm-migrate-{tag}-{}", std::process::id());
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(dir) = option_env!("CARGO_TARGET_TMPDIR") {
            candidates.push(PathBuf::from(dir));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("target").join("tmp"));
        }
        candidates.push(std::env::temp_dir());

        for base in candidates {
            let dir = base.join(&name);
            let _ = std::fs::remove_dir_all(&dir);
            if std::fs::create_dir_all(&dir).is_ok() {
                return dir;
            }
        }
        // 所有候选都失败才算错：这时候报出来的是真实的环境问题。
        std::env::temp_dir().join(name)
    }

    /// T7.1（用户点名的用例）：`V10` 必须排在 `V9` 之后——字典序会给出反直觉的顺序。
    #[test]
    fn discover_sorts_by_numeric_version_not_lexicographic() {
        let dir = scratch_dir("unit");

        // 刻意乱序写入，且带上必须被忽略的干扰项。
        for name in [
            "V10__tenth.sql",
            "V9__ninth.sql",
            "V1__first.sql",
            "V2.10__two_ten.sql",
            "V2.9__two_nine.sql",
            "README.md",
            "U9__rollback.sql",
            "v1__lowercase.sql",
        ] {
            std::fs::write(dir.join(name), "select 1 from dual;\n").expect("写脚本失败");
        }

        let found = Migrator::<dm_odbc::OdbcBackend>::discover(&dir).expect("扫描目录失败");
        let versions: Vec<&str> = found.iter().map(|f| f.version.as_str()).collect();
        // `V` 前缀大小写敏感（`v1__lowercase.sql` 被忽略）；`U` 与 `README.md` 也被忽略。
        assert_eq!(versions, vec!["1", "2.9", "2.10", "9", "10"], "{found:?}");
        assert!(found.iter().all(|f| f.checksum.len() == 16));
        assert_eq!(found[4].name, "tenth");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 重复版本号必须报错（静默取一个会让「到底跑了哪个」没有答案）。
    #[test]
    fn discover_rejects_duplicate_versions() {
        let dir = scratch_dir("unit-dup");
        for name in ["V1__a.sql", "V1__b.sql"] {
            std::fs::write(dir.join(name), "select 1 from dual;\n").expect("写脚本失败");
        }

        let err = Migrator::<dm_odbc::OdbcBackend>::discover(&dir).expect_err("重复版本必须报错");
        assert!(
            matches!(err.kind, DmErrorKind::Config(_)),
            "应当是 Config 类错误：{err:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 校验和是稳定的（同内容同值），且内容变化时值也变。
    #[test]
    fn checksum_is_stable_and_content_sensitive() {
        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a_hex(b"a"), "af63dc4c8601ec8c");
        assert_ne!(fnv1a_hex(b"select 1"), fnv1a_hex(b"select 2"));
    }

    /// 表名是拼进 SQL 的，必须白名单校验（否则等于把注入点开在表名上）。
    #[test]
    fn history_table_name_is_validated() {
        assert!(checked_identifier("DM_SCHEMA_HISTORY", "表名").is_ok());
        assert!(checked_identifier("_x1", "表名").is_ok());
        for bad in ["", "1abc", "a-b", "a b", "a;drop table t", &"x".repeat(129)] {
            assert!(
                checked_identifier(bad, "表名").is_err(),
                "{bad:?} 不该被当成合法标识符"
            );
        }
    }
}
