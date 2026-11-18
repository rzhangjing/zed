//! `PooledConn`：池上借出的一条连接 + 查询/执行入口（设计方案 §6.7 / §6.12）。
//!
//! # 为什么 `PooledConn` 在 `dm-query` 而不是 `dm-pool`
//!
//! 它要提供 `query`/`execute`/`fetch_stream`，也就是说它必须知道 `Row`/`RowSet`；
//! 而 `dm-pool` 在依赖图上低于 `dm-query`（`dm-pool ← dm-query`），看不见这些类型。
//! 所以分工是：`dm-pool` 管**连接的生死与复用**（`PoolGuard`），
//! `dm-query` 管**语句的语义**（`PooledConn` 包一层 `PoolGuard`）。
//!
//! # 一次语句执行只桥接一次（§5.4 纪律 1）
//!
//! `prepare → bind → execute → fetch… → close_cursor` 全部写在**同一个**传给
//! [`PoolGuard::with_conn`] 的闭包里。拆成多次桥接的话，每步都要重新进一次阻塞线程、
//! 重新取一次连接槽锁——代价随语句数线性放大，而收益是零。
//!
//! # 无参语句走 `SQLExecDirect`，带参语句走预处理（实测口径）
//!
//! | 情形 | 走的路 | 依据 |
//! |---|---|---|
//! | 无参数 | `prepare` + `exec_direct`（`Backend::execute` 收到非空 `sql`） | 真机 T3.7 里 DDL/无参 DML/查询**全部**是 `SQLExecDirect` 跑通的 |
//! | 有参数 | `prepare` + `bind_params` + `exec`（`execute` 收到空 `sql`） | `SQLExecDirect` 不接受参数（`dm-odbc` 会翻成 `Unsupported`），带参只能走预处理 |
//!
//! 代价说明：无参路径多花一次 `SQLPrepare`（`Backend` 只提供「`prepare` 拿到语句句柄」
//! 这一条路，没有「只分配句柄」的入口）。这是**有意换来的确定性**——真正跑通过的是
//! `SQLExecDirect`，而不是「预处理过的 DDL 再 `SQLExecute`」那条没验证的路径。

use std::sync::Arc;
use std::time::{Duration, Instant};

use dm_core::{
    Backend, ColumnMeta, DmValue, ExecOutcome, FetchState, LobParam, LobSource, ParamArrayOutcome,
    RowBuf,
};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
use dm_pool::DefaultBackend;
use dm_pool::{DmPool, PoolGuard};

use crate::args::IntoArgs;
use crate::batch;
use crate::fingerprint::Fingerprint;
use crate::lob::{LobRef, LobStream};
use crate::row::Row;
use crate::rowset::{BatchOutcome, RowSet};
use crate::stream::RowStream;
use crate::txn::{IsolationLevel, Transaction};

/// `fetch_stream` 每批取多少行。
///
/// 太小 → 跨线程桥接次数多；太大 → 首批延迟变高、内存占用变大。
/// 64 是「一次桥接大约对应几百微秒」量级下的折中（§4.2 实测单次往返 p50 29–34 µs）。
pub(crate) const STREAM_CHUNK_ROWS: usize = 64;

/// 慢查询告警的门槛判断与 `warn!`（设计方案 §6.9 / AC-09）。
///
/// # 判据（冻结）
///
/// - 阈值取自 `DmConfig::slow_query_threshold`；[`Duration::ZERO`] = 关闭；
/// - 只在语句**成功**时打这条日志——失败有自己的错误路径，混在一起会让慢查询
///   淹没在错误里；
/// - 字段：`op`（`query`/`execute`/`execute_many`/`fetch_stream`）、`sql_fp`
///   （[`Fingerprint`] 的 `Display`，**不含字面量也不含参数值**）、`rows`
///   （本次取到的行数）、`affected`（影响行数，查询为 `None`）、`ms`、
///   `threshold_ms`。
///
/// 返回值只给单测用：`true` = 这条语句够慢。它**不代表**日志真被谁看见了——
/// 没有装 subscriber 时 `warn!` 是空操作。调用处只看副作用，所以**不加**
/// `#[must_use]`（加了会让每个调用点都挂一条 unused_must_use）。
pub(crate) fn log_slow_query(
    op: &'static str,
    sql_fp: &Fingerprint,
    threshold: Duration,
    elapsed: Duration,
    rows: usize,
    affected: Option<u64>,
) -> bool {
    if threshold.is_zero() || elapsed < threshold {
        return false;
    }
    let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    let threshold_ms = u64::try_from(threshold.as_millis()).unwrap_or(u64::MAX);
    tracing::warn!(
        op,
        sql_fp = %sql_fp,
        rows,
        affected,
        ms,
        threshold_ms,
        "慢查询：耗时 {ms} ms ≥ 阈值 {threshold_ms} ms"
    );
    true
}

/// 从池里借出的一条连接。
///
/// `Drop` 即归还（由内部 `PoolGuard` 完成）：**不要**在持有它的时候去等一个
/// 长时间的操作，否则池里会少一条可用连接。要长期占用就显式 `begin()` 开事务。
#[derive(Debug)]
pub struct PooledConn<B: Backend = DefaultBackend> {
    guard: PoolGuard<B>,
}

impl<B: Backend> PooledConn<B> {
    /// 包一条已借出的连接。
    #[must_use]
    pub fn new(guard: PoolGuard<B>) -> Self {
        Self { guard }
    }

    /// 从池里借一条。
    ///
    /// # Errors
    ///
    /// 池已关闭、超时、或建连失败（与 [`DmPool::acquire`] 一致）。
    pub async fn acquire(pool: &DmPool<B>) -> Result<Self, DmError> {
        Ok(Self::new(pool.acquire().await?))
    }

    /// 拆出内部句柄（想直接操作池层时用）。
    #[must_use]
    pub fn into_guard(self) -> PoolGuard<B> {
        self.guard
    }

    /// 只读句柄。
    #[must_use]
    pub fn guard(&self) -> &PoolGuard<B> {
        &self.guard
    }

    /// 可变句柄（`mark_dirty`/`mark_poisoned`/`with_conn` 等）。
    pub fn guard_mut(&mut self) -> &mut PoolGuard<B> {
        &mut self.guard
    }

    /// 连接编号（池内唯一，日志/指标用）。
    #[must_use]
    pub fn id(&self) -> u64 {
        self.guard.id()
    }

    /// 这条连接的累计统计。
    #[must_use]
    pub fn conn_stats(&self) -> dm_pool::ConnStatsSnapshot {
        self.guard.conn_stats()
    }

    // -----------------------------------------------------------------------
    // 查询
    // -----------------------------------------------------------------------

    /// 执行一条语句并**物化**结果。
    ///
    /// DDL/DML 不报错：此时 `rows` 为空、`columns` 为空、`affected` 带影响行数
    /// （驱动没报就是 `None`）。判据见 [`RowSet`] 的模块文档。
    ///
    /// 语句走**每条连接一份的预处理语句缓存**（`pool.stmt_cache_size`，默认 16 条，
    /// `0` = 关闭）。缓存命中与否不影响语义——每次调用都真的执行一遍，只是复用语句句柄。
    ///
    /// 注意**查询语句本身不跨调用复用**：达梦上结果集被取尽之后句柄就再也执行不了了
    /// （`24000/-70027`，见 `crates/dm-odbc/examples/cursor_reuse_probe.rs`），所以
    /// [`Backend::statement_reusable`] 对结果集语句回 `false`，本条查询下次会重新
    /// `prepare`——但执行仍走已预处理的计划（`SQLExecute`），不会退回 `SQLExecDirect`
    /// 的双份解析。
    ///
    /// # Errors
    ///
    /// 预处理/绑定/执行/取数的任何失败都原样上报；连接是否可复用由 `dm-pool`
    /// 依 `is_connection_lost()` 判定（断连类错误会直接销毁连接）。
    pub async fn query(&mut self, sql: &str, args: impl IntoArgs) -> Result<RowSet, DmError> {
        let args = args.into_args().into_values();
        let fp = Fingerprint::of(sql);
        // 阈值在进阻塞闭包之前读一次（闭包里读池配置会多一次锁，§5.8）。
        let threshold = self.guard.config().slow_query_threshold;
        tracing::debug!(sql_fp = %fp, params = args.len(), "query");
        let started = Instant::now();
        // 语句走连接的语句缓存（CAP-C-11）：同一条 SQL 第二次执行省掉一次 `SQLPrepare`。
        let out = self
            .guard
            .with_cached_stmt(sql.to_owned(), move |raw, stmt, _sql| {
                query_with_stmt(raw, stmt, &args)
            })
            .await?;
        log_slow_query(
            "query",
            &fp,
            threshold,
            started.elapsed(),
            out.len(),
            out.affected(),
        );
        Ok(out)
    }

    /// 取第一行（`query` 的便捷包装）。
    ///
    /// # Errors
    ///
    /// 同 [`PooledConn::query`]。
    pub async fn query_one(
        &mut self,
        sql: &str,
        args: impl IntoArgs,
    ) -> Result<Option<Row>, DmError> {
        Ok(self.query(sql, args).await?.rows().first().cloned())
    }

    /// 执行一条语句，返回**受影响行数**。
    ///
    /// 语义（冻结）：`affected` 是 DDL/DML 的账；驱动没报影响行数时为 `0`。
    /// 若这条语句其实产出了结果集（例如误用 `execute` 跑 `SELECT`），结果集会被
    /// **丢弃**并返回 `0`——想拿结果请用 [`PooledConn::query`]。
    ///
    /// 与 [`PooledConn::query`] 共用同一条连接的预处理语句缓存。
    ///
    /// # Errors
    ///
    /// 同 [`PooledConn::query`]。
    pub async fn execute(&mut self, sql: &str, args: impl IntoArgs) -> Result<u64, DmError> {
        let args = args.into_args().into_values();
        let fp = Fingerprint::of(sql);
        let threshold = self.guard.config().slow_query_threshold;
        tracing::debug!(sql_fp = %fp, params = args.len(), "execute");
        let started = Instant::now();
        let affected = self
            .guard
            .with_cached_stmt(sql.to_owned(), move |raw, stmt, _sql| {
                execute_with_stmt(raw, stmt, &args)
            })
            .await?;
        log_slow_query(
            "execute",
            &fp,
            threshold,
            started.elapsed(),
            0,
            Some(affected),
        );
        Ok(affected)
    }

    /// 同一条 SQL 跑多组参数（**部分成功不当整体失败**）。
    ///
    /// 返回 [`BatchOutcome`]：跑到第几条、累计影响多少行、第一条失败的下标。
    /// 只有「连语句都没建成/第一组参数都没绑上」这类**致命**错误才返回 `Err`。
    ///
    /// # 默认走列式参数数组（§4.7 / §6.12 第 7 条）
    ///
    /// 只要「多组参数 + 语句带 `?`」，这条路径就交给
    /// [`Backend::execute_param_array`]——一次 `SQLBindParameter` 绑 `N` 行、一次
    /// `SQLExecute` 打过去，而不是逐行往返。覆盖了数组绑定的后端（`dm-odbc`）因此
    /// 拿到 §4.7 的数量级；没覆盖的后端走 trait 默认实现（逐行 `bind_params` +
    /// `execute`），语义完全一致、只是慢。
    ///
    /// 不走数组绑定的三种情形（都仍然正确）：
    ///
    /// | 情形 | 走什么 | 为什么 |
    /// |---|---|---|
    /// | 0 组参数 | 什么都不执行 | 空集合是合法用法，不是错误 |
    /// | 语句没有 `?` | 逐组 `execute` | 没有参数要绑，「数组」退化成把同一条语句重复打 `N` 次 |
    /// | 每组参数个数与 `?` 不符 | `Err(Config)` | 没法转置；静默错位比报错危险得多 |
    ///
    /// # 记账口径（冻结）
    ///
    /// `executed` 是**已经成功生效**的组数（**不含**失败那一组），`affected` 是这些组
    /// 累计的影响行数，`failed_at` 是第一个失败组的下标。已经生效的组**不**被抹掉
    /// ——批量写要在显式事务里才有「整批回滚」的语义（§6.7），这层只如实记账。
    ///
    /// # Errors
    ///
    /// 预处理失败、参数形状不合法、以及语句级（而不是行级）的执行失败。
    pub async fn execute_many(
        &mut self,
        sql: &str,
        params: impl IntoIterator<Item = impl IntoArgs>,
    ) -> Result<BatchOutcome, DmError> {
        let params: Vec<Vec<DmValue>> = params
            .into_iter()
            .map(|p| p.into_args().into_values())
            .collect();
        let sql = sql.to_owned();
        let fp = Fingerprint::of(&sql);
        let threshold = self.guard.config().slow_query_threshold;
        tracing::debug!(sql_fp = %fp, batches = params.len(), "execute_many");
        let started = Instant::now();
        let out = self
            .guard
            .with_conn(move |raw| execute_many_impl(raw, &sql, &params))
            .await?;
        log_slow_query(
            "execute_many",
            &fp,
            threshold,
            started.elapsed(),
            0,
            Some(out.affected),
        );
        Ok(out)
    }

    /// 流式取数：返回一个**分批**拉行的 [`RowStream`]。
    ///
    /// 首批（含列元数据）在调用时就已经取好，所以 `prepare`/`execute` 阶段的错误
    /// 会在这里直接返回 `Err`，而不是等到第一次 `poll_next`。
    ///
    /// # 为什么它按值拿走 `self`
    ///
    /// 流必须**拥有**这条连接：每一批取数都是一个跨越阻塞线程的 future，而这个
    /// future 要被存进流自身。Rust 不允许把「借用了自身字段的 future」存进自身
    /// （自引用），所以这里让流拿走连接的所有权。流结束时连接由 [`RowStream`] 归还池；
    /// 想在中途把连接拿回来接着用，用 [`RowStream::into_conn`]。
    ///
    /// # Errors
    ///
    /// 同 [`PooledConn::query`]。
    pub async fn fetch_stream(
        self,
        sql: &str,
        args: impl IntoArgs,
    ) -> Result<RowStream<B>, DmError> {
        let args = args.into_args().into_values();
        let sql = sql.to_owned();
        tracing::debug!(
            sql_fp = %Fingerprint::of(&sql),
            params = args.len(),
            "fetch_stream"
        );
        RowStream::start(self, sql, args).await
    }

    /// 开一条**分块读**大 LOB 的流（冻结面 §6.12 的 `LobRef`/`LobStream`）。
    ///
    /// 与 [`fetch_stream`](Self::fetch_stream) 的区别是「取不取整行」：那个把每一行物化成
    /// [`Row`](crate::Row)（`get::<Vec<u8>>()` 就是一份完整副本），这个只把游标推进行
    /// （[`LobStream::next_row`]），值由 [`LobStream::read_chunk`] 按 ≤ 4 KiB 的窗口一块块取。
    /// 想知道哈希、想直接写文件、想边读边解析时用它。
    ///
    /// `lob` 指出要读哪一列（`LobRef::index` 或 `LobRef::name`，名字大小写不敏感）。
    /// **同一列不要同时走 `Row` 与 `LobStream`**：ODBC 不允许同一列混用 `SQLBindCol` 与
    /// `SQLGetData`（§6.5），所以调用方应当把大 LOB 列**单独**取一次。
    ///
    /// # 为什么要 `self`
    ///
    /// 同 [`fetch_stream`](Self::fetch_stream)：流必须拥有这条连接。详见
    /// [`LobStream`](crate::LobStream) 的「连接的所有权」（它比 `RowStream` 还多一条：
    /// 即使某一趟被取消，连接也仍然能由 `into_conn` 还回调用方）。
    ///
    /// # Errors
    ///
    /// 同 [`PooledConn::query`]；语句不是查询时额外报 [`DmErrorKind::Config`]。
    pub async fn lob_stream(
        self,
        sql: &str,
        args: impl IntoArgs,
        lob: LobRef,
    ) -> Result<LobStream<B>, DmError> {
        crate::lob::open(self, sql, args, lob).await
    }

    /// 执行一条写语句，其中**一处** `?` 的值由 [`LobSource`] 分块交付
    /// （`SQL_DATA_AT_EXEC` + `SQLPutData`；分块读的写侧对偶，§6.5 的「大 BLOB 写」）。
    ///
    /// 与 [`execute`](Self::execute) 的区别只有一个：那个要求整个值先在内存里
    /// （[`DmValue::Bytes`](dm_core::DmValue)/`Text`），这个把数据源一趟一趟交给驱动。
    /// 写 10 MiB 的 BLOB 时，前者的峰值是「值 × 份数」，后者是「一块 64 KiB + 源自己的
    /// 缓冲」——[`LOB_WRITE_CHUNK_BYTES`](dm_core::LOB_WRITE_CHUNK_BYTES)。
    ///
    /// `lob` 指出这一处 LOB 占第几个 `?`（0 起算，见 [`LobParam::new`]）：**它不放进
    /// `args`**，所以语句的 `?` 个数必须是 `args.len() + 1`。LOB 没有
    /// [`DmValue`](dm_core::DmValue) 形态——它是流，不是值；硬塞进 `args` 就只能先读成
    /// 完整字节，那正是这个方法要消灭的事。
    ///
    /// # 为什么源必须 owned
    ///
    /// 执行要穿过 `dm-pool` 的语句缓存闭包，而那个闭包必须 `Send + 'static`（§5.4）：
    /// 借来的 `&mut dyn LobSource` 捕获不进去。所以 `lob` 收的是 `S: LobSource + 'static`，
    /// 由本方法把它搬进闭包、在闭包内重借成 `&mut dyn LobSource` 视图。
    ///
    /// # 与语句缓存共用一条语句
    ///
    /// 同 [`execute`](Self::execute)：命中的语句省掉一次 `SQLPrepare`。延迟绑定每次都会先
    /// `reset_parameters` 再重绑，所以复用不会把上一次的喂数据状态带进来；而**喂到一半
    /// 失败**时语句是被丢弃的（缓存只在成功后放回），停在中途的 `SQL_NEED_DATA` 语句
    /// 不会留给下一次执行。
    ///
    /// # Errors
    ///
    /// - 语句没预处理、`?` 个数与 `args.len() + 1` 不符、`lob.column` 越界 ⇒
    ///   [`DmErrorKind::Config`] @ [`Stage::Bind`]；
    /// - 数据源自己读不出来、或文本源不是合法 UTF-8 ⇒ [`DmErrorKind::Encode`]
    ///   @ [`Stage::Execute`]——**不是**连接故障，连接照常回池复用；
    /// - 后端没有这个能力（假后端、非 ODBC 后端）⇒ [`DmErrorKind::Unsupported`]。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// # async fn example(conn: &mut dm_query::PooledConn) -> Result<(), dm_query::DmError> {
    /// use dm_query::{LobBytes, LobKind, LobParam};
    ///
    /// // `insert into t (id, body) values (?, ?)`：第 1 个 `?` 是 LOB。
    /// let lob = LobParam::new(1, LobKind::Binary, LobBytes::new(vec![0u8; 8 * 1024 * 1024]));
    /// let affected = conn.execute_lob("insert into t (id, body) values (?, ?)", (7i32,), lob).await?;
    /// assert_eq!(affected, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn execute_lob<S>(
        &mut self,
        sql: &str,
        args: impl IntoArgs,
        lob: LobParam<S>,
    ) -> Result<u64, DmError>
    where
        S: LobSource + 'static,
    {
        let args = args.into_args().into_values();
        let fp = Fingerprint::of(sql);
        let threshold = self.guard.config().slow_query_threshold;
        tracing::debug!(
            sql_fp = %fp,
            params = args.len(),
            column = lob.column,
            kind = ?lob.kind,
            "execute_lob"
        );
        let started = Instant::now();
        let affected = self
            .guard
            .with_cached_stmt(sql.to_owned(), move |raw, stmt, _sql| {
                // owned 的源在这里借成**动态**视图：`Backend::execute_lob` 收的是
                // 「借一处 LOB」（`&mut LobParam<&mut dyn LobSource>`），先显式退化成 trait 对象。
                let mut lob = lob;
                let source: &mut dyn LobSource = &mut lob.source;
                let mut view = LobParam {
                    column: lob.column,
                    kind: lob.kind,
                    source,
                };
                let outcome = raw.execute_lob(stmt, "", &args, &mut view)?;
                match outcome {
                    ExecOutcome::Affected { rows } => Ok(rows.unwrap_or(0)),
                    // 同 `execute_with_stmt`：用 `execute_lob` 跑了查询就按 0 行记（结果集
                    // 丢弃）。不在这里把「查询被丢掉」升级成错误——本方法的契约就是「只要
                    // 影响行数」。
                    ExecOutcome::ResultSet { .. } => Ok(0),
                    other => Err(unknown_outcome(other)),
                }
            })
            .await?;
        log_slow_query(
            "execute_lob",
            &fp,
            threshold,
            started.elapsed(),
            0,
            Some(affected),
        );
        Ok(affected)
    }

    // -----------------------------------------------------------------------
    // 事务
    // -----------------------------------------------------------------------

    /// 开事务（DM 默认隔离级别）。
    ///
    /// 连接被 `Transaction` 独占；`Transaction` drop 时若没提交/回滚，连接会被
    /// 标记为脏、由池回滚（**不会**把开着的事务留给下一个借出方）。
    ///
    /// # Errors
    ///
    /// 与 [`Backend::begin`] 一致。
    pub async fn begin(self) -> Result<Transaction<B>, DmError> {
        self.begin_with(IsolationLevel::ReadCommitted).await
    }

    /// 开事务并设置隔离级别。
    ///
    /// 隔离级别用 SQL 下发（`SET TRANSACTION ISOLATION LEVEL …`）而不是
    /// `SQL_ATTR_TXN_ISOLATION`：后者要下探 `odbc-sys`，而 `dm-core` 不依赖
    /// `odbc-api`、`Backend` 抽象里也没有这个口子（§6.6 的 4 档取值见
    /// [`IsolationLevel::as_str`]）。这条 SQL 的执行**必须在任何语句之前**，
    /// 否则达梦只会把它应用到下一个事务上。
    ///
    /// # Errors
    ///
    /// - `BEGIN` 失败；
    /// - 隔离级别语句被服务端拒绝（会把事务回滚掉再返回错误，不留下半个事务）。
    pub async fn begin_with(mut self, level: IsolationLevel) -> Result<Transaction<B>, DmError> {
        self.guard.with_conn(|raw| raw.begin()).await?;
        let sql = crate::txn::isolation_sql(level);
        if let Err(err) = self.execute(&sql, ()).await {
            // 隔离级别没设上就不该留一个半开的事务：回滚后再把错误交给调用方。
            let _ = self.guard.with_conn(|raw| raw.rollback()).await;
            return Err(err);
        }
        Ok(Transaction::new(self))
    }
}

// ---------------------------------------------------------------------------
// 闭包内实现（全部是「一次桥接」里的同步代码）
// ---------------------------------------------------------------------------

/// 在**已经预处理好**的语句上执行（参数非空先绑定）。
///
/// 两条路都调 `Backend::execute(stmt, "")`——语句已经准备好了，空 SQL 正是
/// 「用预处理好的语句执行」的契约。**不要把 SQL 文本再传一遍**：那会走
/// `SQLExecDirect`，等于把刚做的 `SQLPrepare` 白扔掉，语句缓存命中也就省不下
/// 服务端那次编译（§3.1 的 ②−① 正是这一项）。
fn execute_prepared<B: Backend>(
    raw: &mut B,
    stmt: &mut B::Stmt,
    args: &[DmValue],
) -> Result<ExecOutcome, DmError> {
    if !args.is_empty() {
        raw.bind_params(stmt, args)?;
    }
    raw.execute(stmt, "")
}

/// 取数循环：`max_rows == None` 表示取到尽。
///
/// 返回 `(本批行, 是否已取尽)`。批间复用同一个 `buf`（§5.4 纪律 3）。
pub(crate) fn fetch_rows<B: Backend>(
    raw: &mut B,
    stmt: &mut B::Stmt,
    columns: &Arc<[ColumnMeta]>,
    buf: &mut RowBuf,
    max_rows: Option<usize>,
) -> Result<(Vec<Row>, bool), DmError> {
    let mut rows = Vec::new();
    loop {
        // 每批前清空：`fetch` 的落点是复用的缓冲，追加语义会让行越攒越多。
        buf.clear();
        match raw.fetch(stmt, buf)? {
            FetchState::Rows { .. } => {
                rows.reserve(buf.len());
                for values in buf.rows() {
                    rows.push(Row::from_slice(values, columns));
                }
                if let Some(max) = max_rows
                    && rows.len() >= max
                {
                    return Ok((rows, false));
                }
            }
            FetchState::Exhausted => return Ok((rows, true)),
            other => return Err(unknown_fetch_state(other)),
        }
    }
}

/// 在**已经预处理好**的语句上执行并把结果物化（`query` 的缓存路径）。
pub(crate) fn query_with_stmt<B: Backend>(
    raw: &mut B,
    stmt: &mut B::Stmt,
    args: &[DmValue],
) -> Result<RowSet, DmError> {
    let outcome = execute_prepared(raw, stmt, args)?;
    match outcome {
        ExecOutcome::ResultSet { columns } => {
            let mut buf = RowBuf::with_capacity(columns.len(), STREAM_CHUNK_ROWS);
            let (rows, _) = fetch_rows(raw, stmt, &columns, &mut buf, None)?;
            raw.close_cursor(stmt)?;
            Ok(RowSet::result_set(columns, rows))
        }
        ExecOutcome::Affected { rows } => {
            raw.close_cursor(stmt)?;
            Ok(RowSet::affected_only(rows))
        }
        other => Err(unknown_outcome(other)),
    }
}

/// 在**已经预处理好**的语句上执行，只取影响行数（`execute` 的缓存路径）。
pub(crate) fn execute_with_stmt<B: Backend>(
    raw: &mut B,
    stmt: &mut B::Stmt,
    args: &[DmValue],
) -> Result<u64, DmError> {
    let outcome = execute_prepared(raw, stmt, args)?;
    let affected = match outcome {
        ExecOutcome::Affected { rows } => rows.unwrap_or(0),
        // 用 `execute` 跑了查询：结果集丢掉、按 0 行处理（想拿结果请用 `query`）。
        // 不在这里把「查询被丢掉」升级成错误——`execute` 的契约就是「只要影响行数」。
        ExecOutcome::ResultSet { .. } => 0,
        other => return Err(unknown_outcome(other)),
    };
    raw.close_cursor(stmt)?;
    Ok(affected)
}

/// 批量执行：**默认走列式参数数组**（§4.7 / §6.12 第 7 条）。
///
/// 流程：建一次语句 → 把行优先的参数组转置成列式（[`crate::batch`]，形状校验都在
/// 那里）→ 一次 [`Backend::execute_param_array`] → 映射成 [`BatchOutcome`]。
///
/// # 为什么这一层没有「逐行循环」的分支
///
/// 早先的实现自带一个逐组循环。那个循环现在**只**活在
/// [`Backend::execute_param_array`] 的默认实现里——把慢的那条路放在 trait 默认实现上
/// 有一个实打实的好处：它同时是「所有假后端照常能跑」与「真实后端覆盖即加速」的
/// 唯一出处，两条路不会各写一遍然后悄悄漂移。本层只负责两件事：空批次不执行，
/// 以及把 `ParamArrayOutcome` 记成 `BatchOutcome`。
pub(crate) fn execute_many_impl<B: Backend>(
    raw: &mut B,
    sql: &str,
    params: &[Vec<DmValue>],
) -> Result<BatchOutcome, DmError> {
    // 语句只建一次：数组绑定与默认的逐行实现都复用同一条预处理语句。
    let mut stmt = raw.prepare(sql)?;
    let columns = batch::collect(params, raw.param_count(&stmt))?;

    // 空批次：一条语句都不执行。先于一切绑定返回，免得为了「0 行」去调一次
    // `SQLSetStmtAttr(PARAMSET_SIZE=0)` —— 那个取值在 ODBC 里本来就是非法的。
    if columns.is_empty() {
        raw.close_cursor(&mut stmt)?;
        return Ok(BatchOutcome::default());
    }

    // 没有 `?` 的语句：没有参数可绑，数组绑定在这里没有意义（列式布局的宽度是 0，
    // 设 `PARAMSET_SIZE` 反而会要求驱动去支持「零参数数组」这种没人实现的东西）。
    // 语义是「把同一条语句重复打 N 次」，直接照做。
    if columns.width() == 0 {
        let mut outcome = BatchOutcome::default();
        for index in 0..columns.rows() {
            // 语句在 :466 已经预处理过，所以每趟传空 SQL 走 `SQLExecute`；
            // 传文本会退回 `SQLExecDirect`，把 N−1 次准备好的计划全白扔。
            match raw.execute(&mut stmt, "") {
                Ok(ExecOutcome::Affected { rows }) => {
                    outcome.affected += rows.unwrap_or(0);
                    outcome.executed += 1;
                }
                Ok(ExecOutcome::ResultSet { .. }) => {
                    raw.close_cursor(&mut stmt)?;
                    outcome.executed += 1;
                }
                Ok(_) | Err(_) => {
                    outcome.failed_at = Some(index);
                    break;
                }
            }
        }
        raw.close_cursor(&mut stmt)?;
        return Ok(outcome);
    }

    let summary = raw.execute_param_array(&mut stmt, columns.as_columns())?;
    let outcome = batch_outcome(&summary);
    raw.close_cursor(&mut stmt)?;
    Ok(outcome)
}

/// 把一次参数数组执行的结果映射成 [`BatchOutcome`]（记账口径的唯一出处）。
///
/// # 为什么成功组数取 `ParamArrayOutcome::executed` 而不是 `failed_at`
///
/// 驱动完全可以「把失败那行也报成已处理」甚至「在失败行之后继续填成功状态」。
/// 用下标减出来的成功行数在「第 500 行违约」这种场景里会正好多算一行，而
/// `executed` 是后端按状态数组逐个判出来的（见 `dm-odbc` 的 `is_param_ok`）。
fn batch_outcome(summary: &ParamArrayOutcome) -> BatchOutcome {
    let mut outcome = BatchOutcome {
        affected: summary.affected,
        executed: summary.executed,
        failed_at: summary.failed_at,
    };
    // 有的语句（例如批量 `INSERT … SELECT`）驱动不给逐行影响行数：此时按
    // 「一个成功组一行」记，至少让账与 `executed` 自洽，而不是报 0 行。
    if outcome.affected == 0 && outcome.executed > 0 {
        outcome.affected = outcome.executed as u64;
    }
    outcome
}

fn unknown_outcome<O: std::fmt::Debug>(outcome: O) -> DmError {
    let mut err = DmError::at(
        DmErrorKind::Unsupported {
            feature: format!("执行结果形态未知：{outcome:?}"),
        },
        Stage::Execute,
    );
    err.push_diag(DiagRecord::native(
        0,
        "后端返回了 `ExecOutcome` 当前变体之外的形态（枚举是 non_exhaustive）：\
         本层不知道该怎么处置，按不支持上报",
    ));
    err
}

fn unknown_fetch_state<O: std::fmt::Debug>(state: O) -> DmError {
    let mut err = DmError::at(
        DmErrorKind::Unsupported {
            feature: format!("取数状态未知：{state:?}"),
        },
        Stage::Fetch,
    );
    err.push_diag(DiagRecord::native(
        0,
        "后端返回了 `FetchState` 当前变体之外的形态（枚举是 non_exhaustive）",
    ));
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 慢查询门槛的三个判据（§6.9 / AC-09）：阈值 0 = 关闭、`< 阈值` 不报、
    /// `>= 阈值` 报（边界必须是「等于也算慢」，否则 1 s 阈值永远差一纳秒）。
    ///
    /// 这里只测**判据与字段**：`tracing` 没装 subscriber 时 `warn!` 是空操作，
    /// 所以断言的是返回值（它同时是「有没有跨过门槛」的唯一出口）。
    #[test]
    fn slow_query_gate_respects_threshold_and_disable() {
        let fp = Fingerprint::of("select * from t where id = 42");

        assert!(
            !log_slow_query(
                "query",
                &fp,
                Duration::ZERO,
                Duration::from_secs(9),
                10,
                None
            ),
            "阈值 0 = 关闭：再慢也不报"
        );
        assert!(
            !log_slow_query(
                "query",
                &fp,
                Duration::from_millis(50),
                Duration::from_millis(49),
                10,
                None
            ),
            "没到阈值不报"
        );
        assert!(
            log_slow_query(
                "query",
                &fp,
                Duration::from_millis(50),
                Duration::from_millis(50),
                10,
                None
            ),
            "恰好等于阈值要报（判据是 >=）"
        );
        assert!(
            log_slow_query(
                "execute",
                &fp,
                Duration::from_secs(1),
                Duration::from_secs(2),
                0,
                Some(7)
            ),
            "超过阈值要报"
        );
    }
}
