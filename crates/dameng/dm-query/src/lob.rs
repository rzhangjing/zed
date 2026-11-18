//! `LobRef` / `LobStream`：大 LOB 的**分块读**（设计方案 §6.5 / §6.12 冻结面）。
//!
//! # 它解决什么
//!
//! [`Row::get::<Vec<u8>>`](crate::Row::get) 与 `get::<String>` 是**全量载入**：1 MiB 的
//! `CLOB` 在内存里就是一份完整的 `String`。想知道哈希、想直接写进文件、想边读边解析时，
//! 这份完整副本是白花的。`LobStream` 给的是「**游标停在某一行上，按 4 KiB 窗口把这一列
//! 抽出来**」，峰值内存是「一块 + 调用方自己的累积区」，与值的大小无关。
//!
//! ```no_run
//! # async fn demo(conn: dm_query::PooledConn) -> Result<(), dm_error::DmError> {
//! use dm_query::LobRef;
//! let mut stream = conn
//!     .lob_stream("select C from T order by ID", (), LobRef::name("C"))
//!     .await?;
//! while stream.next_row().await? {
//!     let mut value = Vec::new();
//!     loop {
//!         let state = stream.read_chunk(&mut value).await?;
//!         if state.is_done() {
//!             break;
//!         }
//!         // 边读边用：`value` 里目前只有已经取回的部分。
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! # 三条必须知道的约束
//!
//! 1. **同一列不能混用 `SQLBindCol` 与 `SQLGetData`**（§6.5）。走 `LobStream` 的列必须是
//!    「单独拉取」的那一列，不要再指望它出现在 `Row` 里。
//! 2. **同一行内 `SQLGetData` 的列号必须递增**（ODBC 规范）：一次分块读只在下一次
//!    `SQLFetch` 之前有效，且这一列应当是**本行最后被读的列**。
//! 3. **非 BMP 文本内容不保真**（§6.5 那条已实测的驱动缺陷）。分块读走的是同一条
//!    `SQLGetData` 路径，所以同一个缺陷在这里同样成立；`BLOB`/`VARBINARY` 逐字节精确。
//!
//! # 状态为什么挂在语句上
//!
//! 进度（读到哪一列、第几趟、跨趟留下的半个代理对）不是纯函数能算出来的东西，它得有
//! 宿主。宿主是**语句**而不是这个结构体：`dm-pool` 的 `with_conn` 只搬后端与语句，
//! 状态挂在语句里，一次分块读就天然跟着它所属的语句走（详见 `dm_odbc::lob` 的模块头）。
//! 本类型负责的是**另一件事**：把「语句 + 连接」一起端住，让分块读能跨 `await` 用。
//!
//! # 连接的所有权（与 `RowStream` 同构）
//!
//! `LobStream` **拥有** `PooledConn`，每次取一趟都把它借给阻塞线程，回来还是它。
//! 与 [`RowStream`](crate::RowStream) 的差别是：这里没有「在飞的那一趟要被存进自身」
//! 的需求（`read_chunk` 是普通 `async fn`，不需要自引用），所以**连接从不离开这个结构体**
//! 的字段——这带来一条很实际的好处：即使在飞的一趟被 `timeout`/`select!` 取消（future 被
//! 丢弃），字段里的连接对象仍然存在，[`LobStream::into_conn`] 照样能把（已被池判定为
//! 不可复用的）连接句柄还回调用方，不会留下一个「连接去哪了」的黑洞。
//!
//! # 取消语义（D11）
//!
//! 取一趟在飞时被丢弃 ⇒ `dm-pool` 判定「没人要结果了」⇒ 该连接按不可复用处理
//! （除非开了 `reuse_after_cancel`）。流在**两趟之间**被丢弃且结果集没取尽 ⇒ 本类型的
//! `Drop` 主动标脏，让池走一遍复位（关游标/回滚/复原 autocommit）。
//!
//! # `read_chunk` 为什么要「先借一块再追加」
//!
//! `with_conn` 的闭包必须是 `'static`，所以它**不能借用调用方的 `&mut Vec<u8>`**。
//! 每趟因此这样走：闭包内部分配一块（≤ 4 KiB）、取一趟、连同语句一起还回来，
//! 外层再把这 ≤ 4 KiB 追加进调用方的缓冲。代价是每趟一次不超过 4 KiB 的拷贝，
//! 换来的是「调用方的累积区从来不跨线程」——取消最多丢掉**在飞的那一块**，
//! 已经累积的内容永远在调用方自己手里。

use std::sync::Arc;

use dm_core::{Backend, ColumnMeta, DmValue, ExecOutcome, LobChunk};
use dm_error::{DmError, DmErrorKind, Stage};
use dm_pool::DefaultBackend;

use crate::conn::PooledConn;
use crate::fingerprint::Fingerprint;
use crate::row::ColumnIndex;

/// 开流时 `with_conn` 闭包要一次交回的「语句 + 列元数据 + 目标列下标」。
///
/// 单独立个名字是因为这个三元组的书写复杂度已经超过 clippy 的阈值，而且它正是
/// 「语句必须跟着连接一起搬运」这条约束的落点（见模块头）。
type Attached<B> = Result<(<B as Backend>::Stmt, Arc<[ColumnMeta]>, usize), DmError>;

/// 指向结果集里某一列——按序号（0 起）或按列名。
///
/// 名字匹配沿用 [`ColumnIndex`] 的冻结语义：**大小写不敏感**（ASCII 折叠），找不到就是
/// [`DmErrorKind::Decode`]、越界也是 `Decode`——和 `Row::get("C")` 的报错口径一致，
/// 调用方不必为「按名」和「按序号」学两套错误处理。
///
/// ```no_run
/// # use dm_query::LobRef;
/// let by_name = LobRef::name("C");
/// let by_index = LobRef::index(0);
/// # let _ = (by_name, by_index);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LobRef {
    /// 结果集里的列序号，**从 0 起**。
    Index(usize),
    /// 列名（大小写不敏感）。
    Name(String),
}

impl LobRef {
    /// 按序号（0 起）。
    #[must_use]
    pub fn index(index: usize) -> Self {
        Self::Index(index)
    }

    /// 按列名（大小写不敏感）。
    pub fn name(name: impl Into<String>) -> Self {
        Self::Name(name.into())
    }

    /// 序号形态（按名构造时为 `None`）。
    #[must_use]
    pub fn as_index(&self) -> Option<usize> {
        match self {
            Self::Index(index) => Some(*index),
            Self::Name(_) => None,
        }
    }

    /// 列名形态（按序号构造时为 `None`）。
    #[must_use]
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Index(_) => None,
            Self::Name(name) => Some(name),
        }
    }

    /// 在列元数据里定位，返回 0 起的序号。
    ///
    /// 用的是 [`ColumnIndex`] 的实现，而不是这里另写一套字符串比较：口径只有一处，
    /// 「`Row::get` 找得到、`LobRef` 找不到」这种事就不会发生。
    ///
    /// # Errors
    ///
    /// 列名不存在或序号越界 ⇒ [`DmErrorKind::Decode`]（与 `Row::get` 同一口径）。
    pub fn resolve(&self, columns: &[ColumnMeta]) -> Result<usize, DmError> {
        match self {
            Self::Index(index) => index.index(columns),
            Self::Name(name) => name.as_str().index(columns),
        }
    }
}

/// 一条语句上按块读一列的流（冻结面，§6.12）。
///
/// 用法是「`next_row` 定位 → `read_chunk` 取到 `Done` → 再 `next_row`」：
///
/// * [`next_row`](Self::next_row)：把游标推到下一行，**不物化任何列**（`false` = 取尽）；
/// * [`read_chunk`](Self::read_chunk)：把**当前行**那一列的内容再取一块，追加进调用方的
///   `Vec`；返回 [`LobChunk::Done`]/[`LobChunk::Null`] 之后这一行就取完了。
///
/// 想中途把连接拿回来接着用，用 [`into_conn`](Self::into_conn)；直接丢弃也会让连接
/// （带着未取尽的游标）标脏回池，只是要多等一次复位。
///
/// # 字段顺序不是随便排的
///
/// `stmt` 声明在 `conn` **之前**：Rust 按声明顺序析构字段，而「语句必须先于连接释放」
/// 是 CT-09 的硬要求（连接交回池之后可能被别的线程复位甚至物理关闭）。
pub struct LobStream<B: Backend = DefaultBackend> {
    columns: Arc<[ColumnMeta]>,
    /// 目标列在 `columns` 里的下标（0 起），构造时就解析好。
    index: usize,
    /// 语句句柄。一次取数**在飞**时它是 `None`（被 move 进了阻塞闭包），回来再填上。
    ///
    /// 声明在 `conn` 之前 ⇒ 先析构（CT-09）。
    stmt: Option<B::Stmt>,
    /// 连接。**从不离开这个字段**（见模块头的「连接的所有权」）。
    ///
    /// 用 `Option` 只有一个原因：本类型实现了 `Drop`，所以 [`LobStream::into_conn`] 不能
    /// 直接把它 move 出去（`Drop` 类型不允许字段被部分移出），只能 `take` 成 `None`。
    /// 正常流程里它**永远是 `Some`**：`take` 只发生在 `into_conn`（它按值拿走 `self`）。
    conn: Option<PooledConn<B>>,
    /// 是否已经对当前行调用过 `lob_start`（懒开：第一趟 `read_chunk` 才开）。
    started: bool,
    /// 游标是否已经停在某一行上（`next_row` 给过 `true`）。
    positioned: bool,
    /// 当前行这一列是否已经读完。
    drained_row: bool,
    /// 当前行这一列是不是 NULL。
    null: bool,
    /// 当前行已经读出的字节数。
    bytes: usize,
    /// 已经定位过的行数。
    rows: u64,
    /// 结果集已取尽（`next_row` 给过 `false`）：`Drop` 判断「该不该标脏」用。
    eof: bool,
}

impl<B: Backend> std::fmt::Debug for LobStream<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不打印连接与语句句柄：它们对排查没有帮助，而且 `conn` 会顺带把池的信息拉进来。
        f.debug_struct("LobStream")
            .field("column", &self.column_name())
            .field("index", &self.index)
            .field("has_stmt", &self.stmt.is_some())
            .field("positioned", &self.positioned)
            .field("drained_row", &self.drained_row)
            .field("null", &self.null)
            .field("bytes", &self.bytes)
            .field("rows", &self.rows)
            .field("eof", &self.eof)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> LobStream<B> {
    /// 开一条分块读：预处理 + 绑定 + 执行，解析目标列，**不取行**。
    ///
    /// 首批的定位由调用方用 [`next_row`](Self::next_row) 显式发起：分块读的入口只应该
    /// 有一条路径（「先定位、再取块」），隐式取第一行会让「取尽」与「第一行没取到」
    /// 两种情况长得一模一样。
    ///
    /// # Errors
    ///
    /// 语句不是查询（`ExecOutcome::Affected`）⇒ [`DmErrorKind::Config`] @ `Stage::Execute`；
    /// 目标列不存在 ⇒ `Decode`；其余同 [`PooledConn::query`](crate::PooledConn::query)。
    pub(crate) async fn start(
        mut conn: PooledConn<B>,
        sql: String,
        args: Vec<DmValue>,
        lob: LobRef,
    ) -> Result<Self, DmError> {
        let outcome = conn
            .guard_mut()
            .with_conn(move |raw| -> Attached<B> {
                let mut stmt = raw.prepare(&sql)?;
                let outcome = if args.is_empty() {
                    raw.execute(&mut stmt, &sql)?
                } else {
                    raw.bind_params(&mut stmt, &args)?;
                    raw.execute(&mut stmt, "")?
                };
                let columns = match outcome {
                    ExecOutcome::ResultSet { columns } => columns,
                    // 非查询语句没有列可按块读：这不是「空结果」，是用法错误。
                    other => return Err(not_a_result_set(&other)),
                };
                let index = lob.resolve(&columns)?;
                Ok((stmt, columns, index))
            })
            .await;

        // 出错时语句是在闭包里析构的（`SQLFreeHandle` 会顺手关掉游标），连接照常回池。
        let (stmt, columns, index) = outcome?;
        Ok(Self {
            columns,
            index,
            stmt: Some(stmt),
            conn: Some(conn),
            started: false,
            positioned: false,
            drained_row: false,
            null: false,
            bytes: 0,
            rows: 0,
            eof: false,
        })
    }

    /// 结果集的列元数据。
    #[must_use]
    pub fn columns(&self) -> &[ColumnMeta] {
        &self.columns
    }

    /// 目标列在结果集里的下标（0 起）。
    #[must_use]
    pub fn column_index(&self) -> usize {
        self.index
    }

    /// 目标列的元数据。
    #[must_use]
    pub fn column(&self) -> &ColumnMeta {
        &self.columns[self.index]
    }

    /// 目标列的名字。
    fn column_name(&self) -> &str {
        &self.column().name
    }

    /// 当前行这一列已经读出的字节数（文本是 UTF-8 字节）。
    #[must_use]
    pub fn bytes_read(&self) -> usize {
        self.bytes
    }

    /// 当前行这一列是不是 NULL（只在读完之后才有意义）。
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.null
    }

    /// 当前行这一列是否已经读完（`Done`/`Null` 已经出现过）。
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.drained_row
    }

    /// 当前行这一列是不是**空的**（合法但没有内容）——与 NULL 分得开。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.drained_row && !self.null && self.bytes == 0
    }

    /// 结果集是否已经取尽。
    #[must_use]
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// 已经定位过的行数。
    #[must_use]
    pub fn rows_read(&self) -> u64 {
        self.rows
    }

    /// 把游标推进到下一行（`false` = 结果集取尽）。
    ///
    /// 这一趟**不物化任何列**：走 [`PooledConn::fetch_stream`](crate::PooledConn::fetch_stream)
    /// 会把整行取进 `RowBuf`（1 MiB 的 CLOB 就是一份完整副本），而分块读要绕开的正是这件事。
    /// 取尽时顺手关掉游标，归还连接时能少一次复位。
    ///
    /// # Errors
    ///
    /// 驱动取行失败，或池/连接层面失败（这一趟的连接按 D11 处置）。
    pub async fn next_row(&mut self) -> Result<bool, DmError> {
        let Some(handle) = self.stmt.take() else {
            return Err(statement_missing());
        };
        let Some(conn) = self.conn.as_mut() else {
            return Err(connection_missing());
        };
        let outcome = conn
            .guard_mut()
            .with_conn(move |raw| -> Result<(B::Stmt, bool), DmError> {
                let mut handle = handle;
                let more = raw.lob_next_row(&mut handle)?;
                if !more {
                    // 取尽 ⇒ 关游标。`lob_next_row` 内部已经知道驱动把游标关了
                    // （`SQL_NO_DATA`），关一次是幂等的。
                    raw.close_cursor(&mut handle)?;
                }
                Ok((handle, more))
            })
            .await;

        match outcome {
            Ok((handle, more)) => {
                self.stmt = Some(handle);
                // 新的一行：上一行的一切都作废。
                self.started = false;
                self.drained_row = false;
                self.null = false;
                self.bytes = 0;
                self.positioned = more;
                if more {
                    self.rows += 1;
                } else {
                    self.eof = true;
                }
                Ok(more)
            }
            // 池级失败：语句回不来了，这条流到此为止。
            Err(err) => Err(err),
        }
    }

    /// 取一块，把**新增**的内容追加到 `out`（追加语义，不清空 `out`）。
    ///
    /// 第一次调用会先把这一列的读取状态挂到语句上（`Backend::lob_start`）。
    /// 返回 [`LobChunk::More`] 就可以接着取；[`LobChunk::Done`] 表示这一行读完了；
    /// [`LobChunk::Null`] 表示这一列是 NULL（此时 `out` 没有被追加任何东西）。
    ///
    /// # Errors
    ///
    /// * 还没 [`next_row`](Self::next_row) 定位到行 ⇒ [`DmErrorKind::Config`]
    ///   （`SQLGetData` 只对当前行有效，早报比让驱动报一句状态错可信）；
    /// * 列不是 LOB/长值类型 ⇒ `Decode`；
    /// * 驱动取块失败 ⇒ `Decode`/驱动错误（`out` 里已累积的内容不受影响）。
    pub async fn read_chunk(&mut self, out: &mut Vec<u8>) -> Result<LobChunk, DmError> {
        if !self.positioned {
            return Err(DmError::at(
                DmErrorKind::Config(
                    "分块读之前要先 next_row() 把游标定位到一行上：SQLGetData 只对当前行有效"
                        .to_owned(),
                ),
                Stage::Fetch,
            ));
        }
        let Some(handle) = self.stmt.take() else {
            return Err(statement_missing());
        };
        let Some(conn) = self.conn.as_mut() else {
            return Err(connection_missing());
        };

        let index = self.index;
        let lazy_start = !self.started;
        let outcome = conn
            .guard_mut()
            .with_conn(move |raw| -> Result<(B::Stmt, (bool, Chunk)), DmError> {
                let mut handle = handle;
                let mut chunk = Vec::new();
                // 内层闭包把「操作失败」与「池级失败」分开：操作失败时语句句柄仍然还回来，
                // 这条流的状态没有被破坏（`out` 也没被碰过）。
                //
                // 布尔位是「这一列的读取状态**现在**挂在语句上了吗」。`lob_start` 失败时
                // 是 `false`：状态没挂上，下一趟必须重来一次——否则重试会跳过 `lob_start`
                // 直接 `SQLGetData`，驱动只会回一句状态错。
                let (started, value): (bool, Chunk) = if lazy_start {
                    match raw.lob_start(&mut handle, index) {
                        Ok(()) => match raw.lob_chunk(&mut handle, &mut chunk) {
                            Ok(state) => (true, Ok((state, chunk))),
                            Err(err) => (true, Err(err)),
                        },
                        Err(err) => (false, Err(err)),
                    }
                } else {
                    match raw.lob_chunk(&mut handle, &mut chunk) {
                        Ok(state) => (true, Ok((state, chunk))),
                        Err(err) => (true, Err(err)),
                    }
                };
                Ok((handle, (started, value)))
            })
            .await;

        let (handle, (started, value)) = outcome?;
        self.stmt = Some(handle);
        if started {
            self.started = true;
        }
        let (state, chunk) = value?;

        // ≤ 4 KiB 的一次拷贝：调用方的累积区从不跨线程（见模块头）。
        self.bytes += chunk.len();
        out.extend_from_slice(&chunk);
        match state {
            LobChunk::Null => {
                self.null = true;
                self.drained_row = true;
            }
            LobChunk::Done => self.drained_row = true,
            LobChunk::More => {}
        }
        Ok(state)
    }

    /// 把连接还回调用方——包括「还没取尽」和「上一趟被取消」这两种情况。
    ///
    /// 比直接丢弃 `LobStream` 干净：直接丢弃会让连接带着没取尽的游标回池（`Drop` 会标脏，
    /// 池会复位，但要等复位）。连接**从不离开本结构体的字段**，所以这里不需要等任何在飞的
    /// 一趟——被取消的那一趟由 `dm-pool` 自己处置（D11）。
    pub async fn into_conn(mut self) -> PooledConn<B> {
        // 只能在这一刻拿走：`Drop` 类型不允许字段被部分移出，所以这里先取成 `None`
        // （`into_conn` 按值消费 `self`，此后不可能再有人观察这个字段）。
        self.conn
            .take()
            .expect("LobStream 的连接只在 into_conn 里被取走")
    }
}

impl<B: Backend> Drop for LobStream<B> {
    fn drop(&mut self) {
        // 结果集没取尽就丢掉 ⇒ 游标还开着，连接不能原样复用。
        // （在飞的那一趟由 `dm-pool` 依取消语义处置，这里只管「连接在手里」这种。）
        if !self.eof
            && let Some(conn) = &mut self.conn
        {
            conn.guard_mut().mark_dirty();
        }
    }
}

/// 一趟的产物：`(状态, 新增字节)`。
type Chunk = Result<(LobChunk, Vec<u8>), DmError>;

fn not_a_result_set(outcome: &ExecOutcome) -> DmError {
    DmError::at(
        DmErrorKind::Config(format!(
            "lob_stream 需要一条产出结果集的语句，实际拿到的是 {outcome:?}：要影响行数请用 execute"
        )),
        Stage::Execute,
    )
}

fn statement_missing() -> DmError {
    DmError::at(
        DmErrorKind::Config(
            "LobStream 的内部状态损坏：语句句柄缺失（这条流在上一趟失败或取消之后已经不可用）"
                .to_owned(),
        ),
        Stage::Fetch,
    )
}

fn connection_missing() -> DmError {
    DmError::at(
        DmErrorKind::Config("LobStream 的内部状态损坏：连接已经被 into_conn 取走".to_owned()),
        Stage::Fetch,
    )
}

/// 分块读的入口：`PooledConn::lob_stream` 的实现体（放在这里是为了让 `conn.rs` 只多一行）。
pub(crate) async fn open<B: Backend>(
    conn: PooledConn<B>,
    sql: &str,
    args: impl crate::args::IntoArgs,
    lob: LobRef,
) -> Result<LobStream<B>, DmError> {
    let args = args.into_args().into_values();
    let sql = sql.to_owned();
    tracing::debug!(
        sql_fp = %Fingerprint::of(&sql),
        params = args.len(),
        column = ?lob,
        "lob_stream"
    );
    LobStream::start(conn, sql, args, lob).await
}
