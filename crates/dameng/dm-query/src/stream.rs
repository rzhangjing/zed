//! `RowStream`：分批拉行的流（设计方案 §6.7 / §6.12）。
//!
//! # 它为什么必须「自己拿着连接」
//!
//! 朴素做法是「`RowStream` 借用 `PooledConn`，每次 `poll_next` 再借一次」——那要求把
//! 一个**借用了自身字段**的 future 存进自身（自引用），安全 Rust 里做不到。
//!
//! 所以这里换一条路：`RowStream` **拥有** `PooledConn`，并把「一批取数」写成
//! 一个**拥有连接所有权**的 `'static` future。状态机在两个位置之间搬运连接：
//!
//! ```text
//!   Phase::Ready { stmt, conn, buf }  ──poll_next──▶  Phase::Fetching(future)
//!          ▲                                                   │
//!          └──────────────── future 完成，连接与语句句柄还回来 ────┘
//! ```
//!
//! 连接**始终**在其中一个位置里，任何时刻都不会丢；`RowStream::into_conn` 把它还给调用方，
//! 而即使调用方直接丢弃 `RowStream`，连接也会随 `PoolGuard` 的 `Drop` 回到池里。
//!
//! # 取消语义（D11）与它在这里的落法
//!
//! 流在一批取数**在飞**的时候被丢弃 ⇒ `with_conn` 的 oneshot 接收端随之析构 ⇒
//! `dm-pool` 判定「没人要结果了」⇒ 该连接按不可复用处理（除非开了 `reuse_after_cancel`）。
//! 流在**两批之间**（`Phase::Ready`）被丢弃且没取尽 ⇒ 本类型的 `Drop` 主动标脏，
//! 让池走一遍复位（关游标/回滚/复原 autocommit）。
//!
//! # 一行的代价
//!
//! 每批 [`STREAM_CHUNK_ROWS`] 行一次跨线程桥接，不是每行一次。行本身在批边界上
//! 从复用的 `RowBuf` 里搬进 `Row`（一次拷贝），这是「`Stream::Item` 必须自有」的代价。

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use dm_core::{Backend, ColumnMeta, DmValue, ExecOutcome, RowBuf};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};
use dm_pool::DefaultBackend;
use futures_core::Stream;

use crate::conn::{PooledConn, STREAM_CHUNK_ROWS, fetch_rows, log_slow_query};
use crate::fingerprint::Fingerprint;
use crate::row::Row;

/// 连续多少批「一行都没有但也没取尽」就判定后端在空转。
const MAX_EMPTY_BATCHES: u32 = 8;

/// 一批取数的结果（连接与语句句柄都会还回来）。
struct Step<B: Backend> {
    /// 语句句柄。**声明在 `conn` 之前**：析构顺序＝声明顺序，语句必须比连接先释放
    /// （CT-09）。连接交回池之后可能被别的线程复位/物理关闭。
    stmt: Option<B::Stmt>,
    conn: PooledConn<B>,
    buf: Option<RowBuf>,
    columns: Arc<[ColumnMeta]>,
    rows: Result<Vec<Row>, DmError>,
    exhausted: bool,
}

/// 在飞的那一批。
type StepFuture<B> = Pin<Box<dyn Future<Output = Step<B>> + Send>>;

/// 状态机的位置。
enum Phase<B: Backend> {
    /// 连接在手里，下一批还没发起。
    Ready {
        /// 同 [`Step::stmt`]：先于 `conn` 析构。
        stmt: Option<B::Stmt>,
        conn: PooledConn<B>,
        buf: Option<RowBuf>,
    },
    /// 一批取数在飞（连接在那个 future 里）。
    Fetching(StepFuture<B>),
    /// 中间态：`into_conn` 把内容取走时留下的空位，不会在正常流程里被观察到。
    Done,
}

/// 分批拉行的结果流。
///
/// `Item = Result<Row, DmError>`：某一批失败时先吐出那个错误，再结束。
pub struct RowStream<B: Backend = DefaultBackend> {
    columns: Arc<[ColumnMeta]>,
    /// 已取到、尚未吐出的行。
    buffered: VecDeque<Row>,
    phase: Phase<B>,
    /// 已取尽或已出错：之后恒返回 `None`。
    finished: bool,
    /// 正常取尽（用于 `Drop` 判断「该不该标脏」）。
    eof: bool,
    empty_batches: u32,
}

// `RowStream` 里没有任何需要「钉住地址」的东西：在飞的那一批已经被 `Box::pin` 住，
// 语句句柄与连接句柄都只在 `poll_next` 期间被搬来搬去（不做自引用）。所以它整体是
// `Unpin` 的——手动实现即可，`poll_next` 里就能用安全代码拿到 `&mut self`
// （crate 级 `forbid(unsafe_code)`，不能用 `get_unchecked_mut`）。
impl<B: Backend> Unpin for RowStream<B> {}

impl<B: Backend> std::fmt::Debug for RowStream<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowStream")
            .field("columns", &self.columns.len())
            .field("buffered", &self.buffered.len())
            .field(
                "phase",
                &match self.phase {
                    Phase::Ready { .. } => "Ready",
                    Phase::Fetching(_) => "Fetching",
                    Phase::Done => "Done",
                },
            )
            .field("finished", &self.finished)
            .field("eof", &self.eof)
            .finish()
    }
}

impl<B: Backend> RowStream<B> {
    /// 打开一个流：预处理 + 执行 + 取**首批**（列元数据在这一步就确定了）。
    ///
    /// 首批是同步等出来的，所以 `prepare`/`execute` 的错误在这里就以 `Err` 的形式返回，
    /// 而不是变成一个「第一次 `poll_next` 才报错」的流。
    pub(crate) async fn start(
        conn: PooledConn<B>,
        sql: String,
        args: Vec<DmValue>,
    ) -> Result<Self, DmError> {
        let step = first_step(conn, sql, args).await;
        match step.rows {
            Err(err) => Err(err),
            Ok(rows) => Ok(Self {
                columns: step.columns,
                buffered: rows.into(),
                phase: Phase::Ready {
                    stmt: step.stmt,
                    conn: step.conn,
                    buf: step.buf,
                },
                finished: step.exhausted,
                eof: step.exhausted,
                empty_batches: 0,
            }),
        }
    }

    /// 列元数据（首批取回时就已经确定）。
    #[must_use]
    pub fn columns(&self) -> &[ColumnMeta] {
        &self.columns
    }

    /// 把连接还回调用方（在飞的那一批会被等回来）。
    ///
    /// 想「流取到一半就停、然后接着用这条连接」时用它——比直接丢弃 `RowStream` 更干净：
    /// 直接丢弃会让连接带着「没取尽的游标」回池（`Drop` 会标脏，池会复位，但要等复位）。
    pub async fn into_conn(mut self) -> PooledConn<B> {
        match std::mem::replace(&mut self.phase, Phase::Done) {
            Phase::Ready { conn, .. } => conn,
            Phase::Fetching(fut) => fut.await.conn,
            Phase::Done => unreachable!("RowStream 的连接只在 Ready/Fetching 两个位置"),
        }
    }
}

impl<B: Backend> Drop for RowStream<B> {
    fn drop(&mut self) {
        // 只有「连接在手里」这种情况能标脏；在飞的那种由 `dm-pool` 依取消语义处置。
        if let Phase::Ready { conn, .. } = &mut self.phase
            && !self.eof
        {
            conn.guard_mut().mark_dirty();
        }
    }
}

impl<B: Backend> Stream for RowStream<B> {
    type Item = Result<Row, DmError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // `RowStream` 的全部字段都是 `Unpin`（在飞的 future 被 `Box::pin` 住），
        // 所以 `get_mut` 是安全的，不需要 `unsafe`（crate 级 `forbid(unsafe_code)`）。
        let this = self.get_mut();

        loop {
            if let Some(row) = this.buffered.pop_front() {
                return Poll::Ready(Some(Ok(row)));
            }
            if this.finished {
                return Poll::Ready(None);
            }

            match &mut this.phase {
                Phase::Ready { .. } => {
                    let Phase::Ready { stmt, conn, buf } =
                        std::mem::replace(&mut this.phase, Phase::Done)
                    else {
                        unreachable!("刚判过是 Ready")
                    };
                    let columns = Arc::clone(&this.columns);
                    this.phase = Phase::Fetching(Box::pin(next_step(conn, stmt, buf, columns)));
                    // 继续循环：立刻去 poll 刚放进去的那一批。
                }
                Phase::Fetching(fut) => match fut.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(step) => {
                        this.empty_batches = 0;
                        this.eof = step.exhausted;
                        this.phase = Phase::Ready {
                            stmt: step.stmt,
                            conn: step.conn,
                            buf: step.buf,
                        };
                        match step.rows {
                            Err(err) => {
                                this.finished = true;
                                return Poll::Ready(Some(Err(err)));
                            }
                            Ok(rows) => {
                                this.buffered.extend(rows);
                                if step.exhausted {
                                    this.finished = true;
                                } else if this.buffered.is_empty() {
                                    // 空批但没取尽：可能是驱动的行集大小行为，再试一批；
                                    // 一直空转就报错，别把调用方吊死。
                                    this.empty_batches += 1;
                                    if this.empty_batches >= MAX_EMPTY_BATCHES {
                                        this.finished = true;
                                        return Poll::Ready(Some(Err(empty_batch_error())));
                                    }
                                }
                            }
                        }
                    }
                },
                Phase::Done => {
                    this.finished = true;
                    return Poll::Ready(None);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 一批取数（全部在同一个阻塞闭包内完成）
// ---------------------------------------------------------------------------

/// 首批取数的闭包产物：`(语句, 行缓冲, 列元数据, 本批行, 是否取尽)`。
///
/// 写成别名不只是为了绕开 clippy 的 `type_complexity`：这个元组就是「一次取数要带回
/// 哪些东西」的清单，写上名字比数五个位置可读得多（`Option` 表示「这一路可能没有」：
/// 非查询语句没有语句句柄，取尽之后不再留缓冲）。
type FirstBatch<B> = (
    Option<<B as Backend>::Stmt>,
    Option<RowBuf>,
    Arc<[ColumnMeta]>,
    Vec<Row>,
    bool,
);

/// 后续批次的闭包产物：`(语句, 行缓冲, 本批行, 是否取尽)`——列元数据已经在流里，
/// 不必每批再带回来一次。
type NextBatch<B> = (Option<<B as Backend>::Stmt>, Option<RowBuf>, Vec<Row>, bool);

/// 首批：预处理 + 绑定 + 执行 + 取一批。
async fn first_step<B: Backend>(
    mut conn: PooledConn<B>,
    sql: String,
    args: Vec<DmValue>,
) -> Step<B> {
    let fp = Fingerprint::of(&sql);
    let threshold = conn.guard().config().slow_query_threshold;
    let started = std::time::Instant::now();
    let outcome = conn
        .guard_mut()
        .with_conn(move |raw| -> Result<FirstBatch<B>, DmError> {
            let mut stmt = raw.prepare(&sql)?;
            let outcome = if args.is_empty() {
                raw.execute(&mut stmt, &sql)?
            } else {
                raw.bind_params(&mut stmt, &args)?;
                raw.execute(&mut stmt, "")?
            };
            match outcome {
                ExecOutcome::ResultSet { columns } => {
                    let mut buf = RowBuf::with_capacity(columns.len(), STREAM_CHUNK_ROWS);
                    let (rows, exhausted) =
                        fetch_rows(raw, &mut stmt, &columns, &mut buf, Some(STREAM_CHUNK_ROWS))?;
                    if exhausted {
                        raw.close_cursor(&mut stmt)?;
                    }
                    Ok((Some(stmt), Some(buf), columns, rows, exhausted))
                }
                ExecOutcome::Affected { .. } => {
                    // 不是查询：给一个空流（列元数据为空）。要影响行数请用 `execute`。
                    raw.close_cursor(&mut stmt)?;
                    Ok((
                        None,
                        None,
                        Arc::from(Vec::<ColumnMeta>::new()),
                        Vec::new(),
                        true,
                    ))
                }
                other => Err(unknown_outcome(other)),
            }
        })
        .await;

    // 慢查询告警（§6.9）：只按**首批**判一次。首批含预处理+执行+第一次取数，
    // 是流的延迟大头；后续每批只有 `STREAM_CHUNK_ROWS` 行，逐批打会把日志刷满。
    if let Ok((_, _, _, rows, _)) = &outcome {
        log_slow_query(
            "fetch_stream",
            &fp,
            threshold,
            started.elapsed(),
            rows.len(),
            None,
        );
    }

    match outcome {
        Ok((stmt, buf, columns, rows, exhausted)) => Step {
            stmt,
            conn,
            buf,
            columns,
            rows: Ok(rows),
            exhausted,
        },
        Err(err) => Step {
            stmt: None,
            conn,
            buf: None,
            columns: Arc::from(Vec::<ColumnMeta>::new()),
            rows: Err(err),
            exhausted: true,
        },
    }
}

/// 后续批次：只取数（语句已经预处理/执行过了）。
async fn next_step<B: Backend>(
    mut conn: PooledConn<B>,
    stmt: Option<B::Stmt>,
    buf: Option<RowBuf>,
    columns: Arc<[ColumnMeta]>,
) -> Step<B> {
    let mut stmt = stmt;
    let mut buf = buf;
    let inner_columns = Arc::clone(&columns);
    let outcome = conn
        .guard_mut()
        .with_conn(move |raw| -> Result<NextBatch<B>, DmError> {
            let mut handle = match stmt.take() {
                Some(handle) => handle,
                // 不该发生：只有首批会把 `stmt` 留成 `None`（那时流已经结束了）。
                None => {
                    return Err(DmError::at(
                        DmErrorKind::Config("RowStream 的内部状态损坏：语句句柄缺失".to_owned()),
                        Stage::Fetch,
                    ));
                }
            };
            let mut buffer = buf
                .take()
                .unwrap_or_else(|| RowBuf::with_capacity(inner_columns.len(), STREAM_CHUNK_ROWS));
            let (rows, exhausted) = fetch_rows(
                raw,
                &mut handle,
                &inner_columns,
                &mut buffer,
                Some(STREAM_CHUNK_ROWS),
            )?;
            if exhausted {
                raw.close_cursor(&mut handle)?;
            }
            Ok((Some(handle), Some(buffer), rows, exhausted))
        })
        .await;

    match outcome {
        Ok((stmt, buf, rows, exhausted)) => Step {
            stmt,
            conn,
            buf,
            columns,
            rows: Ok(rows),
            exhausted,
        },
        Err(err) => Step {
            stmt: None,
            conn,
            buf: None,
            columns,
            rows: Err(err),
            exhausted: true,
        },
    }
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
        "后端返回了 `ExecOutcome` 当前变体之外的形态（枚举是 non_exhaustive）",
    ));
    err
}

fn empty_batch_error() -> DmError {
    let mut err = DmError::at(DmErrorKind::Decode { column: None }, Stage::Fetch);
    err.push_diag(DiagRecord::native(
        0,
        format!(
            "连续 {MAX_EMPTY_BATCHES} 批取数都返回 0 行且未宣告取尽：驱动/后端在空转，\
             这里主动中止以免调用方被无限吊住"
        ),
    ));
    err
}
