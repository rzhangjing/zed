//! 后端抽象（设计方案 §5.11）——把「物理连接」这件事与上层的池、查询 API 解耦。
//!
//! ## 为什么现在就抽这一层（v1.0 只有 ODBC 一个实现）
//!
//! 成本≈0，收益是**未来接任意上层框架只需写一个适配器**：那个适配器把框架自己的
//! `Operation` 翻成这里的一组调用，而 L3/L4/L5（`dm-odbc`/`dm-pool`/`dm-query`）
//! 一行都不用改。Toasty 达梦驱动（§9）就是这条约束的第一个受益者。
//!
//! ## 对象安全
//!
//! [`Backend::connect`] 与 [`Backend::close`] 带 `where Self: Sized`，因此
//! **不在 vtable 里**，`dyn Backend<Stmt = _>` 仍然可用（测试里有编译期断言）。
//! 需要「按 trait object 关闭」时走 `Drop`；[`Backend::close`] 只对具体类型可用，
//! 它的价值是**把关闭错误交给调用方**——`Drop` 里不能报告错误。
//!
//! ## 一条不能忘的纪律（§6.4）
//!
//! 「先判语句类别再决定取不取数」在 DPI 时代靠数 `number_columns > 0` 手工判断，
//! 漏判会得到 `-70027 无效的游标状态`。这里把判断**编码进类型**：
//! [`ExecOutcome`] 要么是 `ResultSet`（可以 `fetch`），要么是 `Affected`（不能 `fetch`）。
//! `fetch` 之前不需要再问「这语句有结果集吗」。

use std::sync::Arc;

use crate::column::ColumnMeta;
use crate::config::DmConfig;
use crate::value::DmValue;
use dm_error::DmError;

/// 语句执行后的两种结局。
///
/// **不是 `bool` + `Option<u64>` 的组合**：那样每次调用方都要自己拼出
/// 「有结果集但行数也有值」这种不存在的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExecOutcome {
    /// 有结果集：可以调用 [`Backend::fetch`] 取行。
    ResultSet {
        /// 列元数据（`Arc` 便于结果集与流对象共享，不复制）。
        columns: Arc<[ColumnMeta]>,
    },
    /// 无结果集（DML / DDL / `SET` 等）：**不要**调 `fetch`。
    Affected {
        /// 受影响行数。`None` = 驱动未提供（例如部分 DDL）。
        rows: Option<u64>,
    },
}

impl ExecOutcome {
    /// 是否产出了结果集。
    pub const fn has_result_set(&self) -> bool {
        matches!(self, ExecOutcome::ResultSet { .. })
    }

    /// 受影响行数（仅 `Affected` 有值）。
    pub const fn affected_rows(&self) -> Option<u64> {
        match self {
            ExecOutcome::Affected { rows } => *rows,
            ExecOutcome::ResultSet { .. } => None,
        }
    }

    /// 列元数据（仅 `ResultSet` 有值）。
    pub fn columns(&self) -> Option<&Arc<[ColumnMeta]>> {
        match self {
            ExecOutcome::ResultSet { columns } => Some(columns),
            ExecOutcome::Affected { .. } => None,
        }
    }

    /// 列数（`Affected` 为 0）。
    pub fn column_count(&self) -> usize {
        match self {
            ExecOutcome::ResultSet { columns } => columns.len(),
            ExecOutcome::Affected { .. } => 0,
        }
    }
}

/// 一次 [`Backend::fetch`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchState {
    /// 本次取到 `rows` 行（`rows > 0`），**可能还有更多**。
    Rows {
        /// 本次取到的行数。
        rows: usize,
    },
    /// 结果集已耗尽（本次取到 0 行）。
    Exhausted,
}

impl FetchState {
    /// 本次取到的行数。
    pub const fn rows(self) -> usize {
        match self {
            FetchState::Rows { rows } => rows,
            FetchState::Exhausted => 0,
        }
    }

    /// 是否已耗尽。
    pub const fn is_exhausted(self) -> bool {
        matches!(self, FetchState::Exhausted)
    }
}

/// 分块读一处 LOB 的一趟结果（§6.5 的分块读 / §6.12 的 `LobStream`）。
///
/// # 为什么 `Null` 是独立的一档而不是「零字节的 `Done`」
///
/// ODBC 里 `SQL_NULL_DATA` 与「取到 0 个码元」都会让一趟取数结束，但**语义相反**：
/// 前者是 SQL `NULL`，后者是空串/空二进制。上层（`LobStream` → `bytes_read()`）
/// 必须能分辨，否则会把 `NULL` 写成空值——那是数据语义的改变，不是表示形式的差别。
/// 让空值在**第一趟**就自报家门，调用方不必再问第二个方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobChunk {
    /// 本趟取到了内容，后面还有（可以继续 [`Backend::lob_chunk`]）。
    More,
    /// 这一列是 `NULL`（零趟即结束）。
    Null,
    /// 取完了；已交付的字节是这一列的**全部**内容（可能是空串/空二进制）。
    Done,
}

impl LobChunk {
    /// 这一列是否已经取完（`Null` 与 `Done` 都算）。
    pub const fn is_done(self) -> bool {
        !matches!(self, LobChunk::More)
    }
}

/// 分块 LOB **写**时一趟交给驱动多少字节（Round 14，§6.5 的「大 BLOB 写」）。
///
/// 这是**我们自己的**上限，不是驱动的限制：探针实测（`crates/dm-odbc/examples/putdata_probe.rs`，
/// 本机 DM8 + `DM8 ODBC DRIVER`）一次 `SQLPutData` 交付 102 407 字节照样合法。取 64 KiB
/// 只因为它小、整，而「一条语句写 10 MiB」在这种粒度下也只有 160 趟——趟数从来不是瓶颈，
/// 峰值内存才是。
pub const LOB_WRITE_CHUNK_BYTES: usize = 64 * 1024;

/// 待写 LOB 的**值类型**：决定驱动层把它当二进制还是当文本交付。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobKind {
    /// 二进制（`BLOB`/`IMAGE`）：[`LobSource`] 给出的字节原样交付。
    Binary,
    /// 文本（`CLOB`/`TEXT`）：[`LobSource`] 给出的是 **UTF-8**，由驱动层转成宽字符交付。
    ///
    /// # 为什么源侧说 UTF-8
    ///
    /// Rust 里的文本本来就是 UTF-8。让每个调用方自己转 UTF-16 只是在驱动层少一次转换，
    /// 却把「一个字符被批边界劈开」这件事摊给了所有调用方——那是驱动层该负责的
    /// （§6.5 非 BMP 那条缺陷也说明：码元层面的坑不该外溢到 API 上）。
    Text,
}

/// 一处待写 LOB 的**数据源**（owned，可跨线程搬进 `dm-pool` 的工作闭包）。
///
/// # 语义
///
/// [`next_chunk`](LobSource::next_chunk) 把**下一段**字节**追加**到 `out`（不清空它，
/// 与分块读的追加语义一致），返回 `false` = 源已取尽。一次调用追加的字节数**不得超过**
/// [`LOB_WRITE_CHUNK_BYTES`]：驱动层按这个粒度向服务端交付，源一次给得更多不会更快，
/// 只会让峰值内存变大。
///
/// # 为什么是 owned 而不是收一个 `&mut dyn Read`
///
/// 调用链要穿过 `dm-pool` 的 `with_conn`，它的闭包必须 `Send + 'static`（§5.4）：借来的
/// 读写器过不去。所以源必须自己能被搬进去——这就是本 trait 存在的理由。
pub trait LobSource: Send {
    /// 精确的总字节数（已知时给出）。
    ///
    /// `Some(n)` 让**二进制**列能用 `SQL_LEN_DATA_AT_EXEC(n)` 一开始就告诉驱动「一共 n 字节」；
    /// `None` 退回 `SQL_DATA_AT_EXEC`（驱动逐趟来问）。它**不是**内存预留提示，文本列也不用它
    /// （UTF-8 的字节数与驱动要交付的 UTF-16 字节数不是一回事）。
    fn size_hint(&self) -> Option<usize> {
        None
    }

    /// 追加下一段（`false` = 已取尽）。
    fn next_chunk(&mut self, out: &mut Vec<u8>) -> Result<bool, DmError>;
}

/// 一次分块 LOB 写：写哪一处 `?`、什么类型、数据从哪来。
///
/// `column` 是语句里 `?` 的**下标**（0 起算），也就是 [`Backend::execute_lob`] 的 `args`
/// 里**没有**的那一处：LOB 参数没有 `DmValue` 形态，不进 `args`。所以语句的 `?` 总个数
/// = `args.len() + 1`。
#[derive(Debug)]
pub struct LobParam<S> {
    /// 这一处 LOB 在语句里的 `?` 下标（0 起算）。
    pub column: usize,
    /// 值类型。
    pub kind: LobKind,
    /// 数据源。
    pub source: S,
}

impl<S> LobParam<S> {
    /// 组装一处待写 LOB。
    pub fn new(column: usize, kind: LobKind, source: S) -> Self {
        Self {
            column,
            kind,
            source,
        }
    }
}

/// 把一段**已经在内存里**的字节当 LOB 写（`Vec<u8>`/`&[u8]`/`String` 都收）。
///
/// 存在的理由：多数调用方本来就有完整值（配置里的小文本、刚读出来的图片），让它们为了
/// 走分块路径而手写一个 [`LobSource`] 是没必要的仪式。注意它的**内存曲线与整值绑定一样**
/// （值已经在内存里了）——真正的流式要用 [`LobReader`] 或自己实现 [`LobSource`]。
#[derive(Debug, Clone)]
pub struct LobBytes {
    bytes: Vec<u8>,
    offset: usize,
}

impl LobBytes {
    /// 接管一段字节。
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
            offset: 0,
        }
    }

    /// 总字节数。
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// 是否零字节。
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl LobSource for LobBytes {
    fn size_hint(&self) -> Option<usize> {
        Some(self.bytes.len())
    }

    fn next_chunk(&mut self, out: &mut Vec<u8>) -> Result<bool, DmError> {
        let remaining = self.bytes.len() - self.offset;
        if remaining == 0 {
            return Ok(false);
        }
        let take = remaining.min(LOB_WRITE_CHUNK_BYTES);
        out.extend_from_slice(&self.bytes[self.offset..self.offset + take]);
        self.offset += take;
        Ok(true)
    }
}

/// 从任何 [`std::io::Read`] 流式写入 LOB（文件、网络流、解压流都行）。
///
/// `size_hint` 要用 [`LobReader::with_size_hint`] 明确给：从 `Read` 上问不出长度，而
/// `Some(n)` 能让二进制列走 `SQL_LEN_DATA_AT_EXEC(n)`（少几趟驱动问答）。
#[derive(Debug)]
pub struct LobReader<R> {
    reader: R,
    size_hint: Option<usize>,
}

impl<R> LobReader<R> {
    /// 包裹一个读取器，长度未知（走 `SQL_DATA_AT_EXEC`）。
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            size_hint: None,
        }
    }

    /// 包裹一个读取器，并声明总字节数（走 `SQL_LEN_DATA_AT_EXEC(n)`）。
    pub fn with_size_hint(reader: R, size_hint: usize) -> Self {
        Self {
            reader,
            size_hint: Some(size_hint),
        }
    }

    /// 取回里面的读取器（提前放弃写入时用）。
    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R: std::io::Read + Send> LobSource for LobReader<R> {
    fn size_hint(&self) -> Option<usize> {
        self.size_hint
    }

    fn next_chunk(&mut self, out: &mut Vec<u8>) -> Result<bool, DmError> {
        let start = out.len();
        out.resize(start + LOB_WRITE_CHUNK_BYTES, 0);
        let mut filled = 0;
        while filled < LOB_WRITE_CHUNK_BYTES {
            match self.reader.read(&mut out[start + filled..]) {
                // 读空 = 源结束。**不是**错误：`Read` 的契约就是这么说的。
                Ok(0) => break,
                Ok(n) => filled += n,
                // 被信号打断不算失败（`Read` 文档明确要求重试）。
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    // `out` 已经被 `resize` 撑大过：失败时必须还原，否则调用方会看到一段
                    // 属于别人的零字节尾巴。
                    out.truncate(start);
                    return Err(lob_source_error(&err));
                }
            }
        }
        out.truncate(start + filled);
        Ok(filled > 0)
    }
}

/// 数据源自己读不出来：**是参数编码失败，不是连接故障**。
///
/// # 为什么不是 `DmErrorKind::Io`
///
/// `Io` 的语义是「连接已经不可信」（`is_connection_lost()` 为真，见
/// `crates/dm-error/src/lib.rs:248`、`:305`），池会因此**丢弃**整条连接。但这里连接好好的
/// ——只是调用方给的那段源读不下去了（文件被删、网络流断了）。回池 `reset()`（关游标 +
/// 回滚语句）之后连接仍可复用，所以归 `Encode`（参数编码失败）这一档。
///
/// 原始 IO 错误进诊断链：`Encode` 只有一个 `column` 字段，装不下消息。
fn lob_source_error(err: &std::io::Error) -> DmError {
    DmError::with_diags(
        dm_error::DmErrorKind::Encode { column: None },
        vec![dm_error::DiagRecord::native(
            0,
            format!("读取待写 LOB 数据源失败：{err}"),
        )],
        dm_error::ErrorContext::new(dm_error::Stage::Execute),
    )
}

/// 一次 [`Backend::execute_param_array`] 的账（T6.4，§4.7 数组绑定）。
///
/// # 为什么输入是「列式」的
///
/// 输入是**列式**的（每个 `?` 一列、每列 `N` 个元素），因为 ODBC 数组绑定本身就是
/// 列式布局：第 `p` 个参数的值缓冲是一段连续的「`N` 个同宽元素」，`BufferLength`
/// 就是元素步长。把转置交给调用方，后端就不必再分配一份行优先副本。
///
/// # 为什么行级失败不是 `Err`
///
/// 批量写「跑到第几条出的问题」本身就是**结果**：前面那些行多半已经生效了
/// （`dm-query` 的 `BatchOutcome` 就是照这个口径记账的）。所以行级失败进
/// [`ParamArrayOutcome::failed_at`]，只有**语句级**失败（prepare 没做、execute
/// 拿到了错误码）才返回 `Err`。把行级失败也变成 `Err`，调用方就再也拿不到
/// 「已经成功了几行」这个信息。
///
/// 注意这个类型**故意不叫** `BatchOutcome`：那是 `dm-query` 的公开类型（§6.7），
/// 属于上层契约；这里是后端的能力边界，两者形状像但分层不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParamArrayOutcome {
    /// 累计受影响行数（驱动没报的那些按 0 计）。
    pub affected: u64,
    /// 实际**成功执行**的参数集个数（下标 `0..executed` 都生效了）。
    pub executed: usize,
    /// 第一个失败的参数集下标；`None` = 全部成功。
    pub failed_at: Option<usize>,
}

impl ParamArrayOutcome {
    /// 是否全部成功。
    pub const fn is_complete(&self) -> bool {
        self.failed_at.is_none()
    }
}

/// 取值的落点：一个**行优先**的、可跨批复用的 `DmValue` 缓冲。
///
/// 复用是设计目标之一（§5.4 纪律 3：稳态零分配）——`dm-query` 的
/// `RowStream` 每批调一次 `clear()` 再 `fetch`，`Vec` 的容量一直留着。
#[derive(Debug, Clone, PartialEq)]
pub struct RowBuf {
    /// 调用方**请求**的列数（未做下限修正）。用于暴露「漏判 `ExecOutcome`」。
    requested_columns: usize,
    columns: usize,
    /// 行优先存储，长度恒为 `rows * columns`。
    values: Vec<DmValue>,
}

impl RowBuf {
    /// 为 `columns` 列的结果集建缓冲。
    ///
    /// `columns == 0` 是**调用方的编程错误**（说明没检查 `ExecOutcome`），
    /// 这里退回 1 以免除零，由 [`RowBuf::is_degenerate`] 暴露出来。
    pub fn new(columns: usize) -> Self {
        let requested_columns = columns;
        let columns = columns.max(1);
        Self {
            requested_columns,
            columns,
            values: Vec::new(),
        }
    }

    /// 预留 `columns × rows` 的容量，避免首批取数时扩容。
    pub fn with_capacity(columns: usize, rows: usize) -> Self {
        let requested_columns = columns;
        let columns = columns.max(1);
        Self {
            requested_columns,
            columns,
            values: Vec::with_capacity(columns * rows),
        }
    }

    /// 列数（至少 1）。
    pub const fn columns(&self) -> usize {
        self.columns
    }

    /// 已取到的行数。
    pub fn len(&self) -> usize {
        self.values.len() / self.columns
    }

    /// 是否没有行。
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 是否由 `new(0)` 触发的退化缓冲（调用方漏判了 `ExecOutcome`）。
    pub fn is_degenerate(&self) -> bool {
        self.requested_columns == 0
    }

    /// 清空行、保留容量（跨批复用的关键）。
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// 第 `index` 行。
    pub fn row(&self, index: usize) -> Option<&[DmValue]> {
        let start = index.checked_mul(self.columns)?;
        self.values.get(start..start + self.columns)
    }

    /// 逐行遍历（`dm-query` 的 `RowSet` 用它构造 `Row`）。
    pub fn rows(&self) -> impl Iterator<Item = &[DmValue]> {
        self.values.chunks_exact(self.columns)
    }

    /// 后端写入用：追加一行。宽度必须等于 [`RowBuf::columns`]。
    pub fn push_row(&mut self, row: &[DmValue]) -> Result<(), DmError> {
        if row.len() != self.columns {
            return Err(DmError::at(
                dm_error::DmErrorKind::Decode { column: None },
                dm_error::Stage::Fetch,
            ));
        }
        self.values.extend_from_slice(row);
        Ok(())
    }

    /// 后端写入用：追加 `n` 行 `NULL` 占位，返回它们的可变视图。
    ///
    /// 典型用法是「先开位、再逐列填」——ODBC 的列绑定就是这种形状。
    pub fn extend_nulls(&mut self, n: usize) -> &mut [DmValue] {
        let start = self.values.len();
        self.values.resize(start + n * self.columns, DmValue::Null);
        &mut self.values[start..]
    }

    /// 后端写入用：取第 `row` 行的可变切片。
    pub fn row_mut(&mut self, row: usize) -> Option<&mut [DmValue]> {
        let start = row.checked_mul(self.columns)?;
        self.values.get_mut(start..start + self.columns)
    }
}

/// 一个物理连接后端。
///
/// 实现必须 `Send`（可跨线程移动），**不必 `Sync`**——外层用 `Mutex` 串行化，
/// 这正是 `dm-pool` 的 `ConnInner` 的做法（§5.4）。
pub trait Backend: Send + 'static {
    /// 该后端的语句类型（ODBC 下是持有 `dhstmt` 等价物的对象）。
    type Stmt: Send + 'static;

    /// 后端名称，用于日志与错误消息。
    fn name(&self) -> &'static str {
        "unknown"
    }

    /// 建连。`where Self: Sized` 使它不进 vtable（保对象安全）。
    fn connect(cfg: &DmConfig) -> Result<Self, DmError>
    where
        Self: Sized;

    /// 探活。`Ok(false)` = 连接已死（**不是**错误）。
    ///
    /// ⚠️ ODBC 实现应走 `SQL_ATTR_CONNECTION_DEAD` 而不是 `SELECT 1`：
    /// §4.2 实测一次往返 p50 29–34 µs，探活不该花这个钱，也不该在
    /// 服务端留下语句痕迹。
    fn ping(&mut self) -> Result<bool, DmError>;

    /// 预处理。语句对象可复用，但**每次重新执行前必须关游标**（`-70027` 的教训），
    /// 而且达梦上「结果集取尽的语句」已经不可复用——谁能进语句缓存由
    /// [`statement_reusable`](Self::statement_reusable) 说。
    fn prepare(&mut self, sql: &str) -> Result<Self::Stmt, DmError>;

    /// 绑定参数（D8 纪律见 §6.3）。
    fn bind_params(&mut self, stmt: &mut Self::Stmt, args: &[DmValue]) -> Result<(), DmError>;

    /// 执行。`sql` 用于「未 prepare 直接执行」的路径；已 prepare 的可传空串。
    fn execute(&mut self, stmt: &mut Self::Stmt, sql: &str) -> Result<ExecOutcome, DmError>;

    /// 这条语句**现在**还能不能再执行一次（语句缓存的准入判据，CAP-C-11）。
    ///
    /// 判据在语句刚执行完、结果集还没取的时候问一次就够：`dm-pool` 的缓存只在
    /// 「本次执行成功」后拿它决定要不要把语句放回表里。返回 `false` 的语句一律
    /// 丢弃（句柄随条目 drop），下次同一条 SQL 重新 `prepare`。
    ///
    /// # 默认 `true`
    ///
    /// 抽象层默认认为「已预处理的语句可以反复执行」，这也是 ODBC 规范的说法。
    ///
    /// # 为什么达梦需要 `false`
    ///
    /// 达梦不是这样：**结果集被取尽之后，该语句句柄就再也执行不了了**。真机实测
    /// （`crates/dm-odbc/examples/cursor_reuse_probe.rs`，本机 DM8）：
    ///
    /// | 场景 | 结果 |
    /// |---|---|
    /// | `prepare` + `exec` + 取尽 + `exec` | `24000/-70027 无效的游标状态` |
    /// | 同上，中间显式 `close_cursor()`（返回 `Ok`！） | 仍 `-70027` |
    /// | 同上，第二次改 `exec_direct`（旧路径的形状） | 仍 `-70027` |
    /// | `prepare` + `exec` + **不取**（游标真开着）+ `exec` | 通过 |
    /// | DML（无结果集）：`prepare` + `exec` + `exec` | 通过 |
    ///
    /// 也就是说「取尽」这一步本身就把句柄弄死了，`SQLFreeStmt(SQL_CLOSE)` 与
    /// `SQLExecDirect` 都救不回来（与 `-70027` 的既有教训是同一族驱动怪癖，见
    /// `crates/dm-odbc/examples/t37_bisect.rs`）。因此 `dm-odbc` 对「产出结果集」
    /// 的语句返回 `false`：查询语句不跨调用复用，DML/DDL 照常复用。
    ///
    /// 注意这不影响「本次执行」：缓存未命中时语句本来就是新 `prepare` 的，第一次
    /// 执行（`SQLExecute`）完全正常。
    fn statement_reusable(&self, _stmt: &Self::Stmt) -> bool {
        true
    }

    /// 已预处理语句的 `?` 占位符个数；后端拿不到时返回 `None`。
    ///
    /// # 为什么会需要它
    ///
    /// 批量写要在**碰参数之前**把「第 3 组少了一个参数」报成可行动的用法错误
    /// （`dm-query::batch` 拿它做形状校验）。不查的话只能等 `bind_params` 报
    /// 「参数个数不符」——错误还是错的，但调用方拿不到「第几组」这个定位信息。
    ///
    /// 默认 `None` 是**故意**的：`Backend` 抽象里没有「问占位符个数」这件事，
    /// 假后端不该为了这个可选优化被迫实现一个新的必选方法（那会让所有既有实现
    /// 立刻编不过）。返回 `None` 时形状校验退化成「各列等长」这一条，语义不变。
    fn param_count(&mut self, _stmt: &Self::Stmt) -> Option<u16> {
        None
    }

    /// 批量写：把 `columns` 描述的 `N` 组参数**一次**执行掉（T6.4，§4.7）。
    ///
    /// 输入是列式的：`columns[p][r]` 是第 `r` 组参数的第 `p` 个值，因此
    /// 「组数」= 各列长度（必须一致），「参数个数」= 列数（必须等于语句的 `?` 个数）。
    /// 空批次（`columns` 为空，或每列都是空）是**调用方用法错误**，报
    /// [`DmErrorKind::Config`](dm_error::DmErrorKind::Config)；「语句有 `?` 但一行都没传」同理——真正的
    /// 「零个 `?` 的语句跑 `N` 遍」是 `columns.is_empty() && n > 0`，那是合法的。
    ///
    /// # 默认实现＝逐行循环
    ///
    /// 默认实现就是「每组 `bind_params` + `execute`」，语义与没有数组绑定能力时
    /// **完全一致**：这是给所有假后端（以及未来的非 ODBC 后端）留的台阶——它们
    /// 不必实现这个方法也能通过 `dm-query` 的批量写用例。真后端覆盖它才会快
    /// （ODBC 下走列式数组绑定，§4.7 目标 33×）。
    ///
    /// 失败口径（与覆盖了本方法的真后端**同口径**，两条路不允许漂移）：
    ///
    /// - **行级**失败：某一行的 `execute` 报错（含第 0 行——违约就是违约，与服务端
    ///   在第几行挡下无关）→ 记进 `failed_at` 后停下，前面已生效的照实计数，
    ///   返回 `Ok`。这是 `execute_many` 的契约：部分成功不当整体失败。
    /// - **语句级**失败：`bind_params` 报错（参数形状/类型与语句对不上）→ 立即
    ///   `Err`。连参数都没绑上，没有任何账可记。
    fn execute_param_array(
        &mut self,
        stmt: &mut Self::Stmt,
        columns: &[Vec<DmValue>],
    ) -> Result<ParamArrayOutcome, DmError> {
        // 组数取各列长度的最小值：不一致时**不要**猜一个更长的数然后越界读数，
        // 少跑几组是安全的方向（真正的形状检查在 `dm-query::batch` 里，那里能给出
        // 「第几列是几」的可行动错误）。
        let rows = columns.iter().map(Vec::len).min().unwrap_or(0);
        let mut outcome = ParamArrayOutcome::default();

        // 行缓冲复用：默认实现是「不支持数组绑定的后端」的兜底，它本身就该是慢的
        // （真后端会覆盖本方法），但没必要每组都新建一个 `Vec`。
        let mut args: Vec<DmValue> = Vec::with_capacity(columns.len());
        for index in 0..rows {
            args.clear();
            args.extend(columns.iter().map(|column| column[index].clone()));
            self.bind_params(stmt, &args)?;

            // 行级失败与 ODBC 路径同口径：记下下标并停下，前面已生效的照实计数。
            // 只有「连第一条都没跑成」才是 `Err`（那样的失败没有账可记）。
            let executed = match self.execute(stmt, "") {
                Ok(ExecOutcome::Affected { rows }) => {
                    outcome.affected += rows.unwrap_or(0);
                    true
                }
                // 批量写里出现结果集（例如 `INSERT … RETURNING`）：与 `dm-query`
                // 既有的逐组路径同口径——把游标关掉，算这一条「执行成功」。
                Ok(ExecOutcome::ResultSet { .. }) => {
                    self.close_cursor(stmt)?;
                    true
                }
                // 行的 `execute` 报错**永远是行级失败**（记 `failed_at`），第 0 行也不例外。
                //
                // # 为什么第 0 行不返回 `Err`
                //
                // `execute_many` 的契约是「部分成功不当整体失败」，它的**唯一**错误形态
                // 是「连语句都没建成 / 参数组形状不合法」这类**语句级**故障。第 0 行违约
                // 与第 500 行违约在服务端是同一件事（那一行的约束没过），把它升格成
                // `Err` 会让调用方在「批量写被唯一约束挡住」时拿到 `Err` 而不是
                // `BatchOutcome { failed_at: Some(0) }`——同一类故障出现两种返回形态，
                // 调用方就得多写一个 `match` 分支。
                //
                // 上面 `self.bind_params(...)?` 保留 `?`：绑不上是**语句级**问题
                // （参数形状/类型与语句不符），确实没有账可记。
                Err(_err) => {
                    outcome.failed_at = Some(index);
                    false
                }
            };
            if executed {
                outcome.executed += 1;
            } else {
                break;
            }
        }

        Ok(outcome)
    }

    /// 取列元数据（`ExecOutcome::ResultSet` 已带时可不调用）。
    fn columns(&mut self, stmt: &Self::Stmt) -> Result<Arc<[ColumnMeta]>, DmError>;

    /// 取一批行写入 `into`（`into` 由调用方 `clear()` 后复用）。
    fn fetch(&mut self, stmt: &mut Self::Stmt, into: &mut RowBuf) -> Result<FetchState, DmError>;

    /// 关闭游标。**重新执行同一语句前必须调用**（否则 `-70027 无效的游标状态`）。
    fn close_cursor(&mut self, stmt: &mut Self::Stmt) -> Result<(), DmError>;

    // -----------------------------------------------------------------------
    // 分块 LOB 读（§6.5 的分块读 / §6.12 的 `LobStream`）——三个方法都是**可选能力**
    // -----------------------------------------------------------------------
    //
    // 默认体全部返回 `Unsupported`：这是「有默认实现的新增方法」，不是必须实现的
    // 义务。加进 `Backend` 而不是单开一个 `Lob` trait，理由是**同一个语句句柄上的
    // 状态**——分块读的进度（读到哪一列、第几趟、半个代理对）必须和语句一起被搬运
    // 回执行线程（`dm-pool` 的 `with_conn` 只搬 `Self` 与 `Self::Stmt`），另开 trait
    // 就得再搬一份旁路状态，反而把不变式拆散。

    /// 开始在**当前行**上分块读某一列（`column` 从 0 起算）。
    ///
    /// # 前置条件（调用方负责，后端不校验）
    ///
    /// 1. 游标已经停在某一行上（先 [`Backend::fetch`] 或先 [`Backend::lob_next_row`]）；
    /// 2. 这一列是**这一行最后被读取的列**——ODBC 规定同一行内 `SQLGetData` 的列号必须
    ///    递增，读过高位列之后不能再回头读低位列。只读一列天然满足；要读两列就得
    ///    按列号从小到大读，且不能与 `SQLBindCol` 的列混用。
    ///
    /// 这个方法**不动驱动**：真正的第一趟发生在第一次 [`Backend::lob_chunk`]，
    /// 这样调用方还能先决定要不要读这一列。
    fn lob_start(&mut self, _stmt: &mut Self::Stmt, _column: usize) -> Result<(), DmError> {
        Err(lob_unsupported("lob_start"))
    }

    /// 把游标推进到下一行，**不绑定也不物化任何列**。
    ///
    /// 返回 `false` = 结果集取尽。这正是分块读要的那次 `SQLFetch`：走 [`Backend::fetch`]
    /// 会把整行取进 `RowBuf`（1 MiB 的 CLOB 在内存里就是一份完整副本），而分块读的目的
    /// 恰恰是「不进 `RowBuf`」。
    fn lob_next_row(&mut self, _stmt: &mut Self::Stmt) -> Result<bool, DmError> {
        Err(lob_unsupported("lob_next_row"))
    }

    /// 再取一趟，把**新增**的内容追加到 `out`（追加语义，不清空 `out`）。
    ///
    /// 一趟最多写 `LOB_CHUNK_BYTES` 量级的内容，所以峰值内存是「一块 + 调用方自己的
    /// 累积区」，与值的大小无关。返回 [`LobChunk::More`] 之后可以接着调；
    /// [`LobChunk::Done`]/[`LobChunk::Null`] 之后继续调是无害的空转。
    fn lob_chunk(
        &mut self,
        _stmt: &mut Self::Stmt,
        _out: &mut Vec<u8>,
    ) -> Result<LobChunk, DmError> {
        Err(lob_unsupported("lob_chunk"))
    }

    // -----------------------------------------------------------------------
    // 分块 LOB 写（§6.5 的大 BLOB 写 / §6.12 的 `LobWriter`）——同样可选能力
    // -----------------------------------------------------------------------

    /// 执行一条语句，其中**一处** `?` 的值由 [`LobSource`] 分块交付（ODBC 的
    /// `SQL_DATA_AT_EXEC` + `SQLPutData`）。
    ///
    /// # 参数怎么对位
    ///
    /// `args` 是**除**那处 LOB 之外的全部参数，`lob.column` 指出 LOB 占的是第几个 `?`
    /// （0 起算）。因此语句的 `?` 个数必须是 `args.len() + 1`。把 LOB 从 `args` 里拿掉不是
    /// 省事：LOB 没有 `DmValue` 形态（它是流，不是值），留在 `args` 里就只能退化成「先读成
    /// 完整字节」——那正是这个方法要消灭的事。
    ///
    /// # 失败口径
    ///
    /// - **绑不上/执行不了**：返回 `Err`（语句级），没有账可记。
    /// - **喂到一半源自己失败**：返回 `Err`，并且**这条语句不能再复用**——它停在
    ///   `SQL_NEED_DATA` 中间。实现必须让调用方（`dm-pool`）没法把它放回语句缓存；
    ///   连接本身还是好的（`net` 侧没坏），走 `reset()` 即可。
    /// - 成功时与 [`Backend::execute`] 同口径：`Affected`/`ResultSet` 二选一。
    ///
    /// # 默认实现
    ///
    /// 返回 `Unsupported`。这是「有默认实现的新增方法」，假后端与非 ODBC 后端不必为此改动；
    /// `dm-query` 的 `PooledConn::execute_lob` 会把这份 `Unsupported` 原样交给调用方。
    fn execute_lob(
        &mut self,
        _stmt: &mut Self::Stmt,
        _sql: &str,
        _args: &[DmValue],
        _lob: &mut LobParam<&mut dyn LobSource>,
    ) -> Result<ExecOutcome, DmError> {
        Err(lob_write_unsupported("execute_lob"))
    }

    /// 开启事务。
    fn begin(&mut self) -> Result<(), DmError>;

    /// 提交。
    fn commit(&mut self) -> Result<(), DmError>;

    /// 回滚。
    fn rollback(&mut self) -> Result<(), DmError>;

    /// 回池复位：关游标 → 回滚未提交事务 → 还原 `autocommit=ON` → 清语句缓存
    /// 中带未关闭游标的条目（§5.7 的四步）。
    fn reset(&mut self) -> Result<(), DmError>;

    /// 确认式关闭。**只有具体类型能调**（不进 vtable）。
    ///
    /// 存在的理由是 `Drop` 不能报错：调试期想把「关闭失败」变成可观测事件时用它。
    /// 生产路径走 `Drop` 即可，因为关闭失败通常只意味着连接已断，重连即可。
    fn close(self)
    where
        Self: Sized;
}

/// 可选能力缺失时的统一错误：`Unsupported` @ `Stage::Fetch`，`feature` 里带上方法名。
///
/// 为什么不新开一个错误变体：错误变体的集合是冻结的（§6.2），而「这个后端没有实现某个
/// 可选能力」本来就是 `Unsupported` 这一档的语义。把方法名放进 `feature` 而不是只写
/// 「不支持 LOB」，是因为调用方唯一能采取的行动是「换个后端或改回整值读」，得先知道
/// 自己在哪一步撞上的。
fn lob_unsupported(method: &str) -> DmError {
    DmError::at(
        dm_error::DmErrorKind::Unsupported {
            feature: format!(
                "{method}：分块 LOB 读（Backend::lob_start / lob_next_row / lob_chunk）\
                 在这个后端上没有实现"
            ),
        },
        dm_error::Stage::Fetch,
    )
}

/// 分块 LOB 写缺失时的统一错误：`Unsupported` @ `Stage::Execute`，`feature` 里带上方法名。
///
/// 与读侧分开写是因为 `feature` 文本要指对方向：调用方看到「分块 LOB 读没实现」却是在写
/// 的时候撞上的，会先去查错地方。阶段取 `Execute` 也是同理——这次失败发生在执行期，
/// 不是在取数期。
fn lob_write_unsupported(method: &str) -> DmError {
    DmError::at(
        dm_error::DmErrorKind::Unsupported {
            feature: format!(
                "{method}：分块 LOB 写（Backend::execute_lob + LobSource）在这个后端上没有实现"
            ),
        },
        dm_error::Stage::Execute,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::DataType;
    use dm_error::{DmErrorKind, Stage};

    // ---------------- ExecOutcome ----------------

    fn cols(n: usize) -> Arc<[ColumnMeta]> {
        (0..n)
            .map(|i| ColumnMeta::new(format!("C{i}"), DataType::Integer))
            .collect::<Vec<_>>()
            .into()
    }

    #[test]
    fn result_set_exposes_columns_and_no_affected_count() {
        let o = ExecOutcome::ResultSet { columns: cols(3) };
        assert!(o.has_result_set());
        assert_eq!(o.column_count(), 3);
        assert_eq!(o.affected_rows(), None);
        assert_eq!(o.columns().unwrap().len(), 3);
    }

    #[test]
    fn affected_exposes_count_and_no_columns() {
        let o = ExecOutcome::Affected { rows: Some(7) };
        assert!(!o.has_result_set());
        assert_eq!(o.column_count(), 0);
        assert_eq!(o.affected_rows(), Some(7));
        assert!(o.columns().is_none());

        // DDL 可能拿不到行数——这是合法状态，不是错误。
        let o = ExecOutcome::Affected { rows: None };
        assert_eq!(o.affected_rows(), None);
    }

    // ---------------- FetchState ----------------

    #[test]
    fn fetch_state_reports_rows_and_exhaustion() {
        assert_eq!(FetchState::Rows { rows: 512 }.rows(), 512);
        assert!(!FetchState::Rows { rows: 1 }.is_exhausted());
        assert_eq!(FetchState::Exhausted.rows(), 0);
        assert!(FetchState::Exhausted.is_exhausted());
    }

    // ---------------- RowBuf ----------------

    #[test]
    fn push_row_and_read_back() {
        let mut b = RowBuf::new(2);
        assert!(b.is_empty());
        assert_eq!(b.columns(), 2);
        b.push_row(&[DmValue::Int(1), DmValue::Text("a".into())])
            .unwrap();
        b.push_row(&[DmValue::Int(2), DmValue::Text("b".into())])
            .unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(b.row(0).unwrap()[0], DmValue::Int(1));
        assert_eq!(b.row(1).unwrap()[1], DmValue::Text("b".into()));
        assert!(b.row(2).is_none(), "越界返回 None，不 panic");
        let all: Vec<usize> = b.rows().map(<[DmValue]>::len).collect();
        assert_eq!(all, vec![2, 2]);
    }

    /// 宽度不符必须报错——否则列会错位，那是最难查的一类 bug。
    #[test]
    fn push_row_rejects_wrong_width() {
        let mut b = RowBuf::new(3);
        let e = b.push_row(&[DmValue::Int(1)]).unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Decode { .. }));
        assert_eq!(e.context.stage, Stage::Fetch);
        assert!(b.is_empty(), "失败的 push 不能留下半行");
    }

    #[test]
    fn clear_keeps_capacity_for_reuse() {
        let mut b = RowBuf::with_capacity(4, 512);
        let null_row: Vec<DmValue> = (0..4).map(|_| DmValue::Null).collect();
        for _ in 0..512 {
            b.push_row(&null_row).unwrap();
        }
        let cap = b.values.capacity();
        assert_eq!(b.len(), 512);
        b.clear();
        assert!(b.is_empty());
        assert_eq!(b.columns(), 4, "clear 只清行，不清列数");
        assert_eq!(b.values.capacity(), cap, "容量必须留着（稳态零分配）");
    }

    /// 后端「先开位再填列」的写法。
    #[test]
    fn extend_nulls_gives_a_writable_slab() {
        let mut b = RowBuf::new(2);
        let slab = b.extend_nulls(3);
        assert_eq!(slab.len(), 6);
        assert!(slab.iter().all(DmValue::is_null));
        slab[0] = DmValue::Int(10);
        slab[2] = DmValue::Int(20);
        assert_eq!(b.len(), 3);
        assert_eq!(b.row(0).unwrap()[0], DmValue::Int(10));
        assert_eq!(b.row(1).unwrap()[0], DmValue::Int(20));
        assert!(b.row(1).unwrap()[1].is_null());
    }

    #[test]
    fn row_mut_writes_the_right_slice() {
        let mut b = RowBuf::new(2);
        b.extend_nulls(2);
        b.row_mut(1).unwrap()[1] = DmValue::Bool(true);
        assert_eq!(b.row(1).unwrap()[1], DmValue::Bool(true));
        assert!(b.row(2).is_none());
        assert!(b.row_mut(2).is_none());
    }

    /// 极端：0 列不该让 `len()` 除零 panic。
    #[test]
    fn zero_columns_degrades_instead_of_dividing_by_zero() {
        let b = RowBuf::new(0);
        assert_eq!(b.columns(), 1, "退回 1 列，避免除零");
        assert_eq!(b.len(), 0);
        assert!(b.is_degenerate(), "但必须能被发现");
        assert!(b.row(0).is_none());
        assert!(!RowBuf::new(1).is_degenerate());
    }

    #[test]
    fn large_row_index_does_not_overflow() {
        let b = RowBuf::new(2);
        assert!(b.row(usize::MAX).is_none());
        let mut b = RowBuf::new(2);
        assert!(b.row_mut(usize::MAX).is_none());
    }

    // ---------------- 对象安全 + mock 后端 ----------------

    /// 编译期断言：`Backend` 可以作为 trait object 使用（T2.4 的验收点）。
    fn assert_object_safe(_: &mut dyn Backend<Stmt = MockStmt>) {}

    struct MockStmt {
        sql: String,
        rows: Vec<Vec<DmValue>>,
        cursor_open: bool,
    }

    /// 一个只存在于测试里的后端：它也顺便把「游标纪律」钉成可执行规格。
    struct MockBackend {
        poisoned: bool,
    }

    impl Backend for MockBackend {
        type Stmt = MockStmt;

        fn name(&self) -> &'static str {
            "mock"
        }

        fn connect(_cfg: &DmConfig) -> Result<Self, DmError> {
            Ok(Self { poisoned: false })
        }

        fn ping(&mut self) -> Result<bool, DmError> {
            Ok(!self.poisoned)
        }

        fn prepare(&mut self, sql: &str) -> Result<Self::Stmt, DmError> {
            Ok(MockStmt {
                sql: sql.to_string(),
                rows: Vec::new(),
                cursor_open: false,
            })
        }

        fn bind_params(
            &mut self,
            _stmt: &mut Self::Stmt,
            _args: &[DmValue],
        ) -> Result<(), DmError> {
            Ok(())
        }

        fn execute(&mut self, stmt: &mut Self::Stmt, _sql: &str) -> Result<ExecOutcome, DmError> {
            if stmt
                .sql
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("select")
            {
                stmt.cursor_open = true;
                stmt.rows = vec![
                    vec![DmValue::Int(1), DmValue::Text("中文".into())],
                    vec![DmValue::Int(2), DmValue::Null],
                ];
                Ok(ExecOutcome::ResultSet { columns: cols(2) })
            } else {
                Ok(ExecOutcome::Affected { rows: Some(0) })
            }
        }

        fn columns(&mut self, _stmt: &Self::Stmt) -> Result<Arc<[ColumnMeta]>, DmError> {
            Ok(cols(2))
        }

        fn fetch(
            &mut self,
            stmt: &mut Self::Stmt,
            into: &mut RowBuf,
        ) -> Result<FetchState, DmError> {
            if !stmt.cursor_open {
                // 复刻真实的 `-70027 无效的游标状态`。
                return Err(DmError::at(
                    DmErrorKind::Server {
                        sqlstate: *b"70027",
                        code: -70027,
                        message: "无效的游标状态".to_string(),
                    },
                    Stage::Fetch,
                ));
            }
            let mut n = 0;
            // 必须**按原顺序**写出——倒序会让 `row(0)` 变成最后一行。
            for row in core::mem::take(&mut stmt.rows) {
                into.push_row(&row)?;
                n += 1;
            }
            stmt.cursor_open = false;
            Ok(if n == 0 {
                FetchState::Exhausted
            } else {
                FetchState::Rows { rows: n }
            })
        }

        fn close_cursor(&mut self, stmt: &mut Self::Stmt) -> Result<(), DmError> {
            stmt.cursor_open = false;
            Ok(())
        }

        fn begin(&mut self) -> Result<(), DmError> {
            Ok(())
        }
        fn commit(&mut self) -> Result<(), DmError> {
            Ok(())
        }
        fn rollback(&mut self) -> Result<(), DmError> {
            Ok(())
        }
        fn reset(&mut self) -> Result<(), DmError> {
            Ok(())
        }
        fn close(self) {}
    }

    #[test]
    fn mock_backend_works_through_a_trait_object() {
        let mut b = MockBackend::connect(&DmConfig::default()).unwrap();
        assert_eq!(b.name(), "mock");
        assert!(b.ping().unwrap());

        let mut stmt = b.prepare("select a, b from t").unwrap();
        let out = b.execute(&mut stmt, "").unwrap();
        assert_eq!(out.column_count(), 2);

        let mut buf = RowBuf::with_capacity(2, 512);
        assert_eq!(b.fetch(&mut stmt, &mut buf).unwrap().rows(), 2);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.row(0).unwrap()[1], DmValue::Text("中文".into()));
        assert!(buf.row(1).unwrap()[1].is_null());

        // 游标已关 ⇒ 再 fetch 必报错（这就是 `-70027` 的形状）。
        let e = b.fetch(&mut stmt, &mut buf).unwrap_err();
        assert!(e.to_string().contains("无效的游标状态"));

        // trait object 路径。
        let dyn_backend: &mut dyn Backend<Stmt = MockStmt> = &mut b;
        assert_object_safe(dyn_backend);
        assert_eq!(dyn_backend.name(), "mock");
    }

    /// 非查询语句：不该有人去 `fetch` 它。
    #[test]
    fn affected_statement_is_distinguishable_from_a_result_set() {
        let mut b = MockBackend::connect(&DmConfig::default()).unwrap();
        let mut stmt = b.prepare("create table t(a int)").unwrap();
        let out = b.execute(&mut stmt, "").unwrap();
        assert!(!out.has_result_set());
        assert_eq!(out.affected_rows(), Some(0));
        // 调用方看到 `Affected` 就不会去 fetch，也就不会撞上 `-70027`。
    }

    #[test]
    fn close_is_available_on_the_concrete_type() {
        let b = MockBackend::connect(&DmConfig::default()).unwrap();
        b.close();
    }

    // ---------------- 分块 LOB 读的默认实现与 LobChunk ----------------

    #[test]
    fn lob_chunk_reports_completion() {
        assert!(!LobChunk::More.is_done());
        assert!(LobChunk::Done.is_done());
        // `Null` 也是「结束了」：零趟即终。它单独占一档是为了让上层能分辨
        // 「SQL NULL」与「空串」，见枚举文档。
        assert!(LobChunk::Null.is_done());
    }

    /// 没实现分块读的后端必须**明确拒绝**，而不是「安静地返回零字节」。
    ///
    /// 安静返回零字节是最坏的一种失败：调用方会把一个 1 MiB 的 CLOB 读成空串，
    /// 然后在业务层莫名其妙地丢失数据。三个方法都得拒绝，缺一个就是漏洞。
    #[test]
    fn lob_defaults_are_explicitly_unsupported() {
        let mut b = MockBackend::connect(&DmConfig::default()).unwrap();
        let mut stmt = b.prepare("select a from t").unwrap();
        let mut out = Vec::new();

        for (what, err) in [
            ("lob_start", b.lob_start(&mut stmt, 0).unwrap_err()),
            ("lob_next_row", b.lob_next_row(&mut stmt).unwrap_err()),
            ("lob_chunk", b.lob_chunk(&mut stmt, &mut out).unwrap_err()),
        ] {
            assert_eq!(err.context.stage, Stage::Fetch, "{what}");
            match &err.kind {
                DmErrorKind::Unsupported { feature } => {
                    assert!(feature.contains(what), "{what}: {feature}");
                }
                other => panic!("{what}: 期望 Unsupported，得到 {other:?}"),
            }
        }
        // 三个方法都没往 `out` 里写东西。
        assert!(out.is_empty());
    }

    // ---------------- 分块 LOB 写的默认实现与数据源 ----------------

    /// 没实现分块写的后端必须明确拒绝：悄悄退化成「零字节的 LOB」会让一次导入
    /// 把 10 MiB 的附件写成空文件，而且**不报错**。
    #[test]
    fn execute_lob_default_is_explicitly_unsupported() {
        let mut b = MockBackend::connect(&DmConfig::default()).unwrap();
        let mut stmt = b.prepare("insert into t (b) values (?)").unwrap();
        let mut source = LobBytes::new(vec![1u8, 2, 3]);
        let mut lob = LobParam::new(0, LobKind::Binary, &mut source as &mut dyn LobSource);

        let err = b.execute_lob(&mut stmt, "", &[], &mut lob).unwrap_err();
        assert_eq!(err.context.stage, Stage::Execute);
        match &err.kind {
            DmErrorKind::Unsupported { feature } => {
                assert!(feature.contains("execute_lob"), "{feature}");
                assert!(feature.contains("分块 LOB 写"), "{feature}");
            }
            other => panic!("期望 Unsupported，得到 {other:?}"),
        }
    }

    #[test]
    fn lob_bytes_delivers_in_windows_and_then_stops() {
        let total = LOB_WRITE_CHUNK_BYTES + 3;
        let mut source = LobBytes::new(vec![7u8; total]);
        assert_eq!(source.len(), total);
        assert_eq!(LobSource::size_hint(&source), Some(total));

        let mut out = Vec::new();
        assert!(source.next_chunk(&mut out).unwrap());
        assert_eq!(out.len(), LOB_WRITE_CHUNK_BYTES);

        // 追加语义：第二趟接在后面，不是从头重写。
        assert!(source.next_chunk(&mut out).unwrap());
        assert_eq!(out.len(), total);

        // 取尽之后不再有内容（继续调是无害的空转）。
        assert!(!source.next_chunk(&mut out).unwrap());
        assert_eq!(out.len(), total);
    }

    #[test]
    fn lob_reader_streams_every_byte_exactly_once() {
        let total = LOB_WRITE_CHUNK_BYTES * 2 + 11;
        let reader = std::io::Cursor::new(vec![9u8; total]);
        let mut source = LobReader::new(reader);
        assert_eq!(LobSource::size_hint(&source), None);

        let mut out = Vec::new();
        let mut trips = 0;
        while source.next_chunk(&mut out).unwrap() {
            trips += 1;
            assert!(trips <= 4, "趟数不该超过 4，实际 {trips}");
        }
        assert_eq!(out.len(), total);
        assert!(out.iter().all(|&b| b == 9));

        // 明确长度走的是另一条绑定形状，源这边只是把它报出去。
        let sized = LobReader::with_size_hint(std::io::Cursor::new(vec![0u8; 5]), 5);
        assert_eq!(LobSource::size_hint(&sized), Some(5));
        assert_eq!(sized.size_hint, Some(5));
    }

    /// 读不出来、读一半被删的文件：**连接是好的**，别让池把连接丢掉。
    struct BrokenReader;

    impl std::io::Read for BrokenReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("磁盘炸了"))
        }
    }

    #[test]
    fn lob_source_failure_is_an_encode_error_not_a_lost_connection() {
        let mut source = LobReader::new(BrokenReader);
        let mut out = vec![1u8, 2, 3];
        let err = source.next_chunk(&mut out).unwrap_err();

        assert_eq!(err.context.stage, Stage::Execute);
        assert_eq!(err.kind, DmErrorKind::Encode { column: None });
        // 关键判据：不是 `Io`（那会让 `dm-pool` 丢弃整条连接）。
        assert!(!err.is_connection_lost());
        // 失败时不能把 `resize` 出来的零字节尾巴留给调用方。
        assert_eq!(out, vec![1u8, 2, 3]);
        // 原始 IO 错误进诊断链（`Encode` 装不下消息）。
        let text = err
            .diags()
            .iter()
            .map(|d| d.message.as_str())
            .collect::<String>();
        assert!(text.contains("磁盘炸了"), "{text}");
    }

    // ---------------- execute_param_array 的默认实现 ----------------

    /// 只实现必需方法的后端：它**不**覆盖 [`Backend::execute_param_array`]，
    /// 于是量到的就是 trait 默认实现（逐行 `bind_params` + `execute`）。
    ///
    /// 它是「假后端 / 未来的非 ODBC 后端」的替身：默认体的契约要是错了，所有
    /// 这类后端都会跟着错，所以默认体必须自己被测，而不是只靠 `dm-query` 那层
    /// 的假后端间接覆盖。
    #[derive(Default)]
    struct DefaultArrayBackend {
        binds: usize,
        executes: usize,
        closes: usize,
        /// 第几次 `execute` 失败（0 起算，含 0）。
        fail_execute_at: Option<usize>,
    }

    struct DefaultArrayStmt;

    impl Backend for DefaultArrayBackend {
        type Stmt = DefaultArrayStmt;

        fn name(&self) -> &'static str {
            "default-array"
        }

        fn connect(_cfg: &DmConfig) -> Result<Self, DmError> {
            Ok(Self::default())
        }

        fn ping(&mut self) -> Result<bool, DmError> {
            Ok(true)
        }

        fn prepare(&mut self, _sql: &str) -> Result<Self::Stmt, DmError> {
            Ok(DefaultArrayStmt)
        }

        fn bind_params(
            &mut self,
            _stmt: &mut Self::Stmt,
            _args: &[DmValue],
        ) -> Result<(), DmError> {
            self.binds += 1;
            Ok(())
        }

        fn execute(&mut self, _stmt: &mut Self::Stmt, _sql: &str) -> Result<ExecOutcome, DmError> {
            let nth = self.executes;
            self.executes += 1;
            if self.fail_execute_at == Some(nth) {
                return Err(DmError::at(
                    DmErrorKind::Server {
                        sqlstate: *b"23000",
                        code: -6602,
                        message: "违反唯一性约束".to_string(),
                    },
                    Stage::Execute,
                ));
            }
            Ok(ExecOutcome::Affected { rows: Some(1) })
        }

        fn columns(&mut self, _stmt: &Self::Stmt) -> Result<Arc<[ColumnMeta]>, DmError> {
            Ok(cols(0))
        }

        fn fetch(
            &mut self,
            _stmt: &mut Self::Stmt,
            _into: &mut RowBuf,
        ) -> Result<FetchState, DmError> {
            Ok(FetchState::Exhausted)
        }

        fn close_cursor(&mut self, _stmt: &mut Self::Stmt) -> Result<(), DmError> {
            self.closes += 1;
            Ok(())
        }

        fn begin(&mut self) -> Result<(), DmError> {
            Ok(())
        }

        fn commit(&mut self) -> Result<(), DmError> {
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), DmError> {
            Ok(())
        }

        fn reset(&mut self) -> Result<(), DmError> {
            Ok(())
        }

        fn close(self) {}
    }

    /// 一张 3 行 × 2 列的表（`columns[p][r]`＝第 `r` 行第 `p` 个参数）。
    fn three_by_two() -> Vec<Vec<DmValue>> {
        vec![
            vec![DmValue::Int(1), DmValue::Int(2), DmValue::Int(3)],
            vec![
                DmValue::Text("甲".into()),
                DmValue::Null,
                DmValue::Text("丙".into()),
            ],
        ]
    }

    /// 默认实现＝逐行：3 行就是 3 次 bind + 3 次 execute，账是 3 行 3 组。
    #[test]
    fn default_param_array_binds_and_executes_once_per_row() {
        let mut backend = DefaultArrayBackend::default();
        let mut stmt = backend.prepare("insert into t values (?, ?)").unwrap();

        let outcome = backend
            .execute_param_array(&mut stmt, &three_by_two())
            .unwrap();

        assert_eq!(backend.binds, 3, "每行绑一次参数");
        assert_eq!(backend.executes, 3, "每行执行一次");
        assert_eq!(outcome.executed, 3);
        assert_eq!(outcome.affected, 3, "每行影响 1 行");
        assert_eq!(outcome.failed_at, None);
    }

    /// **第 0 行**失败也是行级失败：返回 `Ok` 并记 `failed_at = Some(0)`。
    ///
    /// 这是与 `execute_many` 契约对齐的关键一条：把「第一条就违约」升格成 `Err`
    /// 会让调用方在「批量写被唯一约束挡住」时拿到两种返回形态。
    #[test]
    fn default_param_array_reports_a_first_row_failure_as_failed_at() {
        let mut backend = DefaultArrayBackend {
            fail_execute_at: Some(0),
            ..Default::default()
        };
        let mut stmt = backend.prepare("insert into t values (?, ?)").unwrap();

        let outcome = backend
            .execute_param_array(&mut stmt, &three_by_two())
            .unwrap();

        assert_eq!(outcome.failed_at, Some(0));
        assert_eq!(outcome.executed, 0, "失败那一组不计入成功");
        assert_eq!(outcome.affected, 0);
        assert_eq!(backend.executes, 1, "失败即停，不继续后面的行");
    }

    /// 中途失败：前面已生效的照实记账，**不**被抹掉。
    #[test]
    fn default_param_array_keeps_the_rows_that_already_landed() {
        let mut backend = DefaultArrayBackend {
            fail_execute_at: Some(2),
            ..Default::default()
        };
        let mut stmt = backend.prepare("insert into t values (?, ?)").unwrap();

        let outcome = backend
            .execute_param_array(&mut stmt, &three_by_two())
            .unwrap();

        assert_eq!(outcome.failed_at, Some(2));
        assert_eq!(outcome.executed, 2);
        assert_eq!(outcome.affected, 2);
    }

    /// 组数取各列长度的**最小值**：列不齐时少跑几组，绝不越界读。
    #[test]
    fn default_param_array_uses_the_shortest_column() {
        let mut backend = DefaultArrayBackend::default();
        let mut stmt = backend.prepare("insert into t values (?, ?)").unwrap();
        let ragged = vec![
            vec![DmValue::Int(1), DmValue::Int(2), DmValue::Int(3)],
            vec![DmValue::Int(10)],
        ];

        let outcome = backend.execute_param_array(&mut stmt, &ragged).unwrap();

        assert_eq!(outcome.executed, 1, "只有第 0 组凑得齐两个参数");
        assert_eq!(backend.executes, 1);
    }

    /// 一列都没有（0 个 `?`）：一组都不执行，账是零。
    #[test]
    fn default_param_array_with_no_columns_executes_nothing() {
        let mut backend = DefaultArrayBackend::default();
        let mut stmt = backend.prepare("create table t(a int)").unwrap();

        let outcome = backend.execute_param_array(&mut stmt, &[]).unwrap();

        assert_eq!(outcome.executed, 0);
        assert_eq!(backend.executes, 0);
        assert_eq!(backend.binds, 0);
    }
}
