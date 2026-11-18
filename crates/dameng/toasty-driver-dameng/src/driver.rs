//! `DamengDriver`：`toasty_core::driver::Driver` 的达梦实现（T8.6）。
//!
//! # 职责与边界
//!
//! 上游的 `Driver` 只做三件事：报告身份（`url`）、报告能力（`capability`）、
//! 打开连接（`connect`）；外加两个可选的维护入口（`generate_migration`、`reset_db`）。
//! **连接池由引擎创建并持有**（`toasty::Db::builder().build(driver)`，
//! `crates/toasty/src/db/builder.rs:290-299`），我们只是把自研的 `dm-pool` 交出去。
//!
//! 因此本类型持有 `Arc<DmPool>`：`connect()` 每次给一个轻量的
//! [`DamengConnection`](crate::conn::DamengConnection)，它按需向池子借/还**真实**连接。
//! `Driver::max_connections()` 保持上游默认 `None`——真正的上限在
//! `dm_core::PoolConfig::max_size` 上，两处都设会互相打架。
//!
//! 池子可以**现成**给进来（[`DamengDriver::new`] / [`DamengDriver::from_config`]），
//! 也可以只给配置、推迟到第一次用池时再建（[`DamengDriver::lazy`]）——上游的
//! `Driver` 是同步构造的（官方一致性套件的 `Setup::driver(&self) -> Box<dyn Driver>`），
//! 而建池必须在 tokio 运行时里异步做，`lazy` 就是为了补上这个缝。
//!
//! # 迁移
//!
//! `generate_migration` 用 `toasty_sql::migration::MigrationStatement::from_diff` 生成
//! DDL 语句，**每条语句用各自的 schema 快照渲染**（`MigrationStatement::schema()`；
//! 改名/重建类操作会在生成过程中改 schema，用同一份快照会渲染错）。
//! 渲染用 vendored fork 的 `Serializer::dameng`（§9.8）。

use std::borrow::Cow;
use std::sync::Arc;

use async_trait::async_trait;
use dm_core::DmConfig;
use dm_error::DmError;
use dm_pool::DmPool;
use toasty_core::driver::{Capability, ConnectContext, Connection, Driver};
use toasty_core::schema::{db::Migration, diff};

use crate::capability;
use crate::conn::DamengConnection;
use crate::error::{to_toasty, to_toasty_with_sql};

use tokio::sync::OnceCell;

/// 达梦的 Toasty 驱动。
///
/// **不是** `Clone`：引擎只需要一份，且池子本身是 `Arc` 共享的。
#[derive(Debug)]
pub struct DamengDriver {
    source: PoolSource,
}

/// 池子的来源。
///
/// 上游的 `Driver` 是**同步**构造的（例如官方一致性套件的
/// `Setup::driver(&self) -> Box<dyn Driver>`），而 [`DmPool::connect`] 是异步的、
/// 且必须在 tokio 运行时里建池。`Lazy` 变体把「建池」推迟到第一次真正用池的
/// 异步入口（`connect` / `reset_db`），从而支持同步构造。
#[derive(Debug)]
enum PoolSource {
    /// 现成的池（`new` / `from_config`）。
    Ready(Arc<DmPool>),
    /// 只存配置，第一次用到时建池（`lazy`）。
    ///
    /// `config` 走 `Box`：`DmConfig` 有近 400 字节，直接内联会让本枚举的大小由它决定
    /// （`clippy::large_enum_variant`），而这个变体本来只在「还没建池」时短暂存在。
    Lazy {
        config: Box<DmConfig>,
        pool: OnceCell<Arc<DmPool>>,
    },
}

impl DamengDriver {
    /// 用现成的连接池构造驱动。
    #[must_use]
    pub fn new(pool: Arc<DmPool>) -> Self {
        Self {
            source: PoolSource::Ready(pool),
        }
    }

    /// 按配置建池并构造驱动（便捷入口）。
    ///
    /// 与 [`DmPool::connect`] 等价，只是顺手包成 `Arc`；连不上会**立刻**报错。
    pub async fn from_config(config: DmConfig) -> Result<Self, DmError> {
        let pool = DmPool::connect(config).await?;
        Ok(Self::new(Arc::new(pool)))
    }

    /// 只存配置、**不建池**地构造驱动；第一次 `connect` / `reset_db` 时才建池。
    ///
    /// 给「构造点是同步的」场合（例如上游一致性套件的 `Setup::driver`）。
    /// 建池失败会在第一次使用时由 `Driver::connect` 以 `driver_operation_failed`
    /// 报出（错误链上保留原始 `DmError`）。
    #[must_use]
    pub fn lazy(config: DmConfig) -> Self {
        Self {
            source: PoolSource::Lazy {
                config: Box::new(config),
                pool: OnceCell::new(),
            },
        }
    }

    /// 确保池已建立并取回它。
    ///
    /// `Ready` 直接克隆 `Arc`；`Lazy` 走一次性初始化（并发调用也只会建一次池）。
    pub async fn ensure_pool(&self) -> Result<Arc<DmPool>, DmError> {
        match &self.source {
            PoolSource::Ready(pool) => Ok(Arc::clone(pool)),
            PoolSource::Lazy { config, pool } => {
                let pool = pool
                    .get_or_try_init(|| async {
                        DmPool::connect(config.as_ref().clone()).await.map(Arc::new)
                    })
                    .await?;
                Ok(Arc::clone(pool))
            }
        }
    }

    /// 取出内部池的引用（给测试与高级用法）；`lazy` 构造且尚未建池时为 `None`。
    #[must_use]
    pub fn pool(&self) -> Option<&Arc<DmPool>> {
        match &self.source {
            PoolSource::Ready(pool) => Some(pool),
            PoolSource::Lazy { .. } => None,
        }
    }

    /// 配置（无论池是否已经建好）。
    #[must_use]
    fn config(&self) -> &DmConfig {
        match &self.source {
            PoolSource::Ready(pool) => pool.config(),
            PoolSource::Lazy { config, .. } => config,
        }
    }

    /// 可打印的 DSN。
    ///
    /// **不能**直接用 `DmConfig::to_url()`：它会带上明文口令（`connstr.rs:381-386`），
    /// 而上游把这个字符串用于日志与错误信息。这里只保留定位信息。
    fn redacted_url(&self) -> String {
        let cfg = self.config();
        let mut out = String::from("dameng://");
        if !cfg.user.is_empty() {
            out.push_str(&cfg.user);
            out.push('@');
        }
        out.push_str(&cfg.server);
        out.push(':');
        out.push_str(&cfg.port.to_string());
        if let Some(schema) = &cfg.schema {
            out.push('/');
            out.push_str(schema);
        }
        out
    }

    /// 把标识符渲染成达梦能安全接受的形式（双引号 + 内部引号加倍）。
    fn quote_identifier(name: &str) -> String {
        let mut out = String::with_capacity(name.len() + 2);
        out.push('"');
        for ch in name.chars() {
            if ch == '"' {
                out.push('"');
            }
            out.push(ch);
        }
        out.push('"');
        out
    }
}

#[async_trait]
impl Driver for DamengDriver {
    fn url(&self) -> Cow<'_, str> {
        Cow::Owned(self.redacted_url())
    }

    fn capability(&self) -> &'static Capability {
        &capability::DAMENG
    }

    async fn connect(&self, cx: &ConnectContext) -> toasty_core::Result<Box<dyn Connection>> {
        let pool = self.ensure_pool().await.map_err(to_toasty)?;
        Ok(Box::new(DamengConnection::new(pool, cx.query_log)))
    }

    fn generate_migration(&self, schema_diff: &diff::Schema<'_>) -> Migration {
        let statements =
            toasty_sql::migration::MigrationStatement::from_diff(schema_diff, &capability::DAMENG);
        let mut sql = Vec::with_capacity(statements.len());
        for statement in &statements {
            // 每条语句配它自己的 schema 快照（见模块文档）。
            let serializer = toasty_sql::Serializer::dameng(statement.schema());
            sql.push(serializer.serialize(statement.statement()));
        }
        Migration::new_sql_with_breakpoints(&sql)
    }

    /// 删掉当前用户下的**所有**表。
    ///
    /// 上游对这个方法的定位是「测试用」：不应用迁移、只把库清干净
    /// （对应 MySQL 驱动的 `DROP DATABASE` + `CREATE DATABASE`）。达梦没有
    /// 等价的「重建模式」入口，所以逐个 `DROP TABLE`；遇到外键引用时退一步用
    /// `CASCADE CONSTRAINTS` 再试一次。
    async fn reset_db(&self) -> toasty_core::Result<()> {
        let pool = self.ensure_pool().await.map_err(to_toasty)?;
        let mut conn = dm_query::PooledConn::acquire(&pool)
            .await
            .map_err(to_toasty)?;

        let rowset = conn
            .query("SELECT TABLE_NAME FROM USER_TABLES", ())
            .await
            .map_err(to_toasty)?;
        let mut names = Vec::with_capacity(rowset.len());
        for row in rowset.rows() {
            let name = row.get::<String>(0).map_err(to_toasty)?;
            // ⚠️ `USER_TABLES` 在 SYSDBA 下**不只**是「我们建的表」：真机跑套件时它里面同时有
            // `##HISTOGRAMS_TABLE` 这类达梦内部表（`DROP` 会被拒：`[42000/-5535] 没有删除表
            // [##HISTOGRAMS_TABLE]权限`）以及 `SYS*` 系统表。reset_db 的语义是「清掉本次测试
            // 建的表」，不是「清库」，所以这两类必须跳过——否则一个测试用例就能把实例删坏。
            if name.starts_with("##") || name.starts_with("SYS") {
                tracing::debug!(table = %name, "reset_db 跳过达梦内部/系统表");
                continue;
            }
            names.push(name);
        }

        for name in names {
            let quoted = Self::quote_identifier(&name);
            let plain = format!("DROP TABLE {quoted}");
            if let Err(err) = conn.execute(&plain, ()).await {
                // `-2106` = 「对象不存在」：说明这张表在列表与 DROP 之间已经没了
                // （并发测试、或它本来就是让 `USER_TABLES` 列出来的视图/同义词）。
                // `reset_db` 的目标状态就是「表不存在」⇒ 这就是成功，别把它升级成失败
                // （官方套件 `infra_reset_db::reset_db_and_recreate` 真机就是这样挂在
                // `[42S02/-2106] 对象不存在（表、列或视图）` 上的）。
                if is_object_missing(&err) {
                    tracing::debug!(table = %name, "DROP TABLE 时对象已不存在（按成功处理）");
                    continue;
                }
                tracing::debug!(table = %name, %err, "DROP TABLE 失败，改用 CASCADE CONSTRAINTS 重试");
                let cascade = format!("DROP TABLE {quoted} CASCADE CONSTRAINTS");
                if let Err(err) = conn.execute(&cascade, ()).await {
                    // 表名与语句一起上报：没有它，`-2106` 这类错误看不出是哪张表。
                    return Err(to_toasty_with_sql(err, &cascade));
                }
            }
        }

        // 记账表可能已经在上面的列表里；如果它是在我们查询之后才出现的，这里兜一次。
        let drop_migrations = "DROP TABLE \"__TOASTY_MIGRATIONS\"";
        if let Err(err) = conn.execute(drop_migrations, ()).await {
            tracing::debug!(%err, "记账表的兜底 DROP 未成功（可能本来就不存在）");
        }

        Ok(())
    }
}

/// 达梦的「对象不存在」错误（`[42S02/-2106]`）。
///
/// `reset_db` 用它把「DROP 的目标已经没了」判成成功（见那里的注释）。
fn is_object_missing(err: &DmError) -> bool {
    matches!(err.server_code_and_message(), Some((-2106, _)))
}
