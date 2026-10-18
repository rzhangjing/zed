//! # 分块 LOB 读写（M6 / T6.3）
//!
//! 这一层解决的是「**不要为了看一眼就先全量载入**」。[`crate::result::fetch`] 会把每个
//! 值取成一个 [`dm_core::DmValue`]：一行的 1 MiB `CLOB` 在内存里就是一个 1.2 MiB 的
//! `String`（UTF-16 转 UTF-8 之后按内容还可能更大），而调用方也许只想算个哈希或写进
//! 文件。想按需取，就得放弃「一次取整行再交给调用方」的模型，改成「**游标停在这一行上
//! 时，按块把这一列抽出来**」——这正是 [`LobState`] 的语义。
//!
//! ## 两条必须遵守的约束
//!
//! 1. **游标必须停在这一行上**：`SQLGetData` 取的是「当前行」的值，所以一次分块读
//!    只在下一次 `SQLFetch`/重新执行之前有效（[`OdbcStatement::fetch_row`] 自己取一行，
//!    [`LobRead::start`] 则要求调用方已经取好）。
//! 2. **同一行内的 `SQLGetData` 列号必须递增**（ODBC 规范）：一旦对更高的列号调用过
//!    `SQLGetData`，就不能再回头取低位列。所以分块读的那一列必须**在本行最后**读——
//!    要么它是最后一列，要么在它之后不再读任何列。§6.5 的设计约束「同一列不能混用
//!    `SQLBindCol` 与 `SQLGetData`」是同一件事的另一面。
//!
//! ## 状态为什么挂在语句上（`LobState`）
//!
//! 分块读的进度（读到哪一列、第几趟、上一趟留下的半个代理对）**不是**一个纯函数能算
//! 出来的东西，它必须有个宿主。放在语句上有一条别的方案给不了的好处：
//!
//! * `dm-pool` 的 `with_conn` 只把 `Self`（后端）与 `Self::Stmt`（语句）搬回执行线程，
//!   别的旁路状态它不认识。状态挂在语句里 ⇒ **一次分块读天然跟着它所属的语句走**，
//!   上层的 `dm-query::LobStream` 只要搬语句，不必再搬第二份状态，也就不会出现
//!   「语句回了执行线程、进度还留在调用线程」这种要命的错位。
//! * 任何让「当前行」失效的操作（重新预处理、执行、关游标、`SQL_NO_DATA`）都在
//!   `stmt.rs` 里顺手把状态清掉，于是**没有悬空的分块读**。
//!
//! [`LobRead`] 是它的**借用式视图**：老代码（真机用例、`examples/`）仍然按
//! 「`LobRead::start(stmt, i, meta)` ⇒ 反复 `next_chunk`」写，一个字都不用改。
//!
//! ## 与 §6.5「大 LOB 流式读不承诺」的关系
//!
//! §6.5 不承诺的是**内容保真**，不是「能不能分块」：达梦 ODBC 驱动在宽字符上有一条
//! 已实测的缺陷——增补平面字符（代理对）读写都会坏（`🀄` 读回另一个增补平面字符，
//! 见 §6.5 与 [`crate::result`] 的 `read_wide` 文档）。分块读走的是同一条 `SQLGetData`
//! 路径，所以**同一个缺陷在这里同样成立**：非 BMP 文本不要指望内容保真。二进制
//! （`BLOB`/`VARBINARY`）没有这个问题，逐字节精确（有真机用例）。
//!
//! ## 写侧的两种走法（缓冲式与延迟输入）
//!
//! §6.5 给大 LOB 写留了两条路：
//!
//! * **缓冲式**（[`LobWriter`]）：分片收进 `Vec`，最后一次性变成参数值。它的价值在于
//!   「调用方不必一次性构造整个值」，而**不是**省掉这份内存——峰值仍与值同阶。
//! * **延迟输入**（`SQLPutData`）：`SQLBindParameter` 只交出一个 token，执行时驱动回
//!   `SQL_NEED_DATA`，我们用 `SQLParamData` 取回 token、用 `SQLPutData` 把数据一趟一趟
//!   推进去。内存峰值降到一趟窗口（[`dm_core::LOB_WRITE_CHUNK_BYTES`]）。
//!
//! 第二条在 M6 时「没有实现也没有实测」（§14 V-06），Round 14 用
//! `examples/putdata_probe.rs` 探明**达梦的驱动认得 `SQL_NEED_DATA`**（二进制/文本、
//! 已知长度/未知长度四种绑定形状全部可用，回读逐字节一致）之后落地：[`LobFeeder`] 是
//! 驱动那一侧的取数口，[`OdbcStatement::exec_delayed`] 是 `SQL_NEED_DATA` 循环本身。
//!
//! **非 BMP 的警告在写侧同样成立**：文本按 UTF-16 交付，达梦驱动在增补平面字符上的坏
//! 行为（见上一节）不会因为换了走法就消失。二进制（`BLOB`）逐字节精确。

use std::mem::size_of;

use dm_core::{ColumnMeta, DataType, LOB_WRITE_CHUNK_BYTES, LobKind, LobSource};
use dm_error::{DiagRecord, DmError, DmErrorKind, ErrorContext, Stage};
use odbc_api::handles::Statement as _;
use odbc_sys::{
    CDataType, DATA_AT_EXEC, Len, NO_TOTAL, NULL_DATA, Pointer, SQLGetData, SqlReturn, ULen,
    len_data_at_exec,
};

use crate::diag;
use crate::stmt::OdbcStatement;

/// 分块读 LOB 时每趟向驱动申请的窗口大小（字节）。
///
/// 宽字符按 2 字节/码元折算，所以一趟最多 2048 个 UTF-16 码元。这个数字与
/// [`crate::result`] 的 `WIDE_CHUNK_UNITS`/`BINARY_CHUNK_BYTES` 一致：分块读与整值读
/// 面对的是同一个驱动，窗口大小没有理由两套。
pub const LOB_CHUNK_BYTES: usize = 4096;

/// 单个值的兜底上限（码元/字节）。
///
/// 与 `result.rs` 的 `MAX_VALUE_UNITS` 同一条纪律：这不是业务限制，是防坏驱动报出
/// 天文数字的长度指标时我们照着扩容。两个常量分开写是因为那边是私有实现细节；
/// 改一个记得看另一个。
const MAX_LOB_UNITS: usize = 64 * 1024 * 1024;

/// 一趟取数的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkState {
    /// 还有内容，可以继续 [`LobState::next_chunk`]。
    More,
    /// 取完了（或这一列是 NULL）。
    Done,
}

/// 定位在**当前行**上的分块 LOB 读取状态。
///
/// 用法是「开始 → 反复取 → 拼」：
///
/// ```no_run
/// # use dm_odbc::lob::{ChunkState, LobRead};
/// # use dm_odbc::OdbcStatement;
/// # fn demo(stmt: &mut OdbcStatement<'static>, meta: &dm_core::ColumnMeta)
/// #     -> Result<(), dm_error::DmError> {
/// let mut reader = LobRead::start(stmt, 1, meta)?;
/// let mut value = Vec::new();
/// loop {
///     if reader.next_chunk(&mut value)? == ChunkState::Done {
///         break;
///     }
/// }
/// # Ok(()) }
/// ```
///
/// `value` 里始终只有**已经取回的部分**，最大的一块永远不超过
/// `3 × (LOB_CHUNK_BYTES / 2)` 字节（2048 个 UTF-16 码元最多转出 3 字节/字符）。
///
/// 它自己不带 `OdbcStatement` 的引用：状态由语句持有（见模块文档），
/// 所以取一趟要**把状态借出来再放回去**（[`OdbcStatement::take_lob`]）。这不是故意的
/// 别扭，而是「状态必须能跟着语句跨线程搬」的代价——`LobRead` 只是驱动层给老代码留的
/// 便捷视图，`dm-query` 的 `LobStream` 直接把语句搬来搬去。
#[derive(Debug)]
pub struct LobState {
    /// ODBC 列号，**1 起算**。
    column_index: u16,
    column: String,
    /// 文本列走 `SQL_C_WCHAR`（UTF-16），二进制列走 `SQL_C_BINARY`。
    wide: bool,
    /// 一趟的窗口（码元数或字节数）。
    chunk: usize,
    /// 宽字符暂存：第 0 槽留给上一趟留下来的高代理（如果有）。
    wide_buf: Vec<u16>,
    /// 上一趟结尾那个还没配对的**高代理**，留给下一趟的第一枚码元去配。
    ///
    /// 一趟的窗口是任意切的，代理对被切开是常态而不是异常；把一个半个的码元交给
    /// 调用方才是异常（`String::from_utf16` 会直接报错）。
    carry: Option<u16>,
    /// 指标给出的剩余码元/字节数（`None` = 还没跑过，或驱动回了 `SQL_NO_TOTAL`）。
    pending: Option<usize>,
    /// 已经从驱动取出的码元/字节数（`MAX_LOB_UNITS` 兜底用）。
    units: usize,
    /// 已交付给调用方的内容字节数（UTF-8 字节 / 原始字节）。
    emitted: usize,
    null: bool,
    done: bool,
}

impl LobState {
    /// 开始分块读某一列。`column_index` 从 1 起（ODBC 约定）。
    ///
    /// 这一步**不动驱动**：真正的第一趟发生在第一次 [`next_chunk`](Self::next_chunk)。
    /// 这样调用方可以先决定要不要读（比如已经知道这一列是 NULL 就跳过）。
    pub fn new(column_index: u16, meta: &ColumnMeta) -> Result<Self, DmError> {
        let wide = match meta.data_type {
            DataType::Char
            | DataType::VarChar
            | DataType::LongVarChar
            | DataType::WChar
            | DataType::WVarChar
            | DataType::WLongVarChar
            | DataType::Guid => true,
            DataType::Binary | DataType::VarBinary | DataType::LongVarBinary => false,
            other => {
                return Err(decode_error(
                    Some(meta.name.as_str()),
                    format!(
                        "列 {} 不是 LOB/长值类型（{other:?}），分块读没有意义",
                        meta.name
                    ),
                ));
            }
        };

        Ok(Self {
            column_index,
            column: meta.name.clone(),
            wide,
            chunk: if wide {
                LOB_CHUNK_BYTES / size_of::<u16>()
            } else {
                LOB_CHUNK_BYTES
            },
            wide_buf: Vec::new(),
            carry: None,
            pending: None,
            units: 0,
            emitted: 0,
            null: false,
            done: false,
        })
    }

    /// 这一列是 NULL（驱动在某一趟回了 `SQL_NULL_DATA`）。
    pub fn is_null(&self) -> bool {
        self.null
    }

    /// 是否已经取完。
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// 已交付给调用方的内容字节数（文本是 UTF-8 字节）。
    pub fn read_bytes(&self) -> usize {
        self.emitted
    }

    /// 列名（报错与日志用）。
    pub fn column(&self) -> &str {
        &self.column
    }

    /// 再取一趟，把**新增**的内容追加到 `out`（不清空 `out`）。
    ///
    /// 返回 [`ChunkState::Done`] 之后继续调用是无害的空转。
    pub fn next_chunk(
        &mut self,
        stmt: &mut OdbcStatement<'_>,
        out: &mut Vec<u8>,
    ) -> Result<ChunkState, DmError> {
        if self.done {
            return Ok(ChunkState::Done);
        }
        if self.units > MAX_LOB_UNITS {
            return Err(decode_error(
                Some(self.column.as_str()),
                format!("LOB 长度超过 {MAX_LOB_UNITS} 的上限"),
            ));
        }
        if self.wide {
            self.next_wide(stmt, out)
        } else {
            self.next_binary(stmt, out)
        }
    }

    /// 文本列：取一趟 UTF-16 码元 → 转 UTF-8 追加。
    fn next_wide(
        &mut self,
        stmt: &mut OdbcStatement<'_>,
        out: &mut Vec<u8>,
    ) -> Result<ChunkState, DmError> {
        // 有 carry 时它占第 0 槽，本趟向驱动要的窗口从第 1 槽写起。
        let base = usize::from(self.carry.is_some());
        let want = self
            .pending
            .map_or(self.chunk, |units| units.clamp(1, self.chunk));

        self.wide_buf.clear();
        self.wide_buf.resize(base + want, 0);
        if let Some(high) = self.carry {
            self.wide_buf[0] = high;
        }

        let mut indicator: Len = 0;
        // SAFETY: 句柄是本读取器借来的有效语句；`wide_buf` 在 `base` 起有 `want` 个
        // 已初始化的可写码元，长度按字节给出。本层不使用 delayed input 参数，所以
        // `SQLGetData` 不会去解引用任何调用方的参数缓冲。
        let hstmt = stmt.statement().as_sys();
        let ret = unsafe {
            SQLGetData(
                hstmt,
                self.column_index,
                CDataType::WChar,
                self.wide_buf.as_mut_ptr().add(base) as Pointer,
                (want * size_of::<u16>()) as Len,
                &mut indicator,
            )
        };

        if ret == SqlReturn::NO_DATA {
            // 一次都没取到：空值（不是 NULL，NULL 走下面的 `SQL_NULL_DATA`）。
            self.done = true;
            self.wide_buf.clear();
            return self.finish_wide(out);
        }
        let finished = ret == SqlReturn::SUCCESS;
        diag::sql_ok(
            diag::fold_return(ret, "SQLGetData"),
            stmt.statement(),
            Stage::Fetch,
            "SQLGetData",
        )?;

        if indicator == NULL_DATA {
            self.null = true;
            self.done = true;
            self.wide_buf.clear();
            return Ok(ChunkState::Done);
        }
        if indicator < 0 && indicator != NO_TOTAL {
            return Err(decode_error(
                Some(self.column.as_str()),
                format!("文本列返回了无法解释的长度指标 {indicator}"),
            ));
        }

        // 指标在 ODBC 里有两种合法解释（总长度 / 剩余长度），所以只拿它当上界，
        // 真实末端由宽字符 NUL 定——与 [`crate::result`] 的 `read_wide` 同一套办法。
        self.pending = (indicator != NO_TOTAL).then(|| indicator as usize / size_of::<u16>());
        let ceiling = self.pending.map_or(want, |units| units.min(want));
        let written = match self.wide_buf[base..].iter().position(|unit| *unit == 0) {
            Some(position) => position.min(ceiling),
            None => ceiling,
        };
        self.wide_buf.truncate(base + written);
        self.units += written;

        if written == 0 && !finished && base == 0 {
            return Err(decode_error(
                Some(self.column.as_str()),
                "驱动在未取完时写入了 0 个码元，无法推进",
            ));
        }

        // 交付：剥掉结尾那个孤立的高代理，留给下一趟。
        let (text, held) = split_chunk(&self.wide_buf)
            .map_err(|detail| decode_error(Some(self.column.as_str()), detail))?;
        self.carry = held;
        self.emitted += text.len();
        out.extend_from_slice(text.as_bytes());

        if finished {
            self.done = true;
            return self.finish_wide(out);
        }
        if written == 0 {
            return Err(decode_error(
                Some(self.column.as_str()),
                "驱动在未取完时写入了 0 个码元，无法推进",
            ));
        }
        Ok(ChunkState::More)
    }

    /// 收尾时的宽字符检查：值不能以孤立的高代理结束。
    fn finish_wide(&mut self, _out: &mut Vec<u8>) -> Result<ChunkState, DmError> {
        if self.carry.is_some() {
            return Err(decode_error(
                Some(self.column.as_str()),
                "值以孤立的高代理结尾：驱动交回的 UTF-16 是畸形的",
            ));
        }
        Ok(ChunkState::Done)
    }

    /// 二进制列：直接把调用方的 `out` 当暂存（取到多少截断到多少）。
    fn next_binary(
        &mut self,
        stmt: &mut OdbcStatement<'_>,
        out: &mut Vec<u8>,
    ) -> Result<ChunkState, DmError> {
        let start = out.len();
        if start > MAX_LOB_UNITS {
            return Err(decode_error(
                Some(self.column.as_str()),
                format!("LOB 长度超过 {MAX_LOB_UNITS} 的上限"),
            ));
        }
        let want = self
            .pending
            .map_or(self.chunk, |bytes| bytes.clamp(1, self.chunk));
        out.resize(start + want, 0);

        let mut indicator: Len = 0;
        // SAFETY: 同 `next_wide`：从 `start` 起有 `want` 个已初始化的可写字节。
        let hstmt = stmt.statement().as_sys();
        let ret = unsafe {
            SQLGetData(
                hstmt,
                self.column_index,
                CDataType::Binary,
                out.as_mut_ptr().add(start) as Pointer,
                want as Len,
                &mut indicator,
            )
        };

        if ret == SqlReturn::NO_DATA {
            out.truncate(start);
            self.done = true;
            return Ok(ChunkState::Done);
        }
        let finished = ret == SqlReturn::SUCCESS;
        if let Err(err) = diag::sql_ok(
            diag::fold_return(ret, "SQLGetData"),
            stmt.statement(),
            Stage::Fetch,
            "SQLGetData",
        ) {
            out.truncate(start);
            return Err(err);
        }

        if indicator == NULL_DATA {
            out.truncate(start);
            self.null = true;
            self.done = true;
            return Ok(ChunkState::Done);
        }
        if indicator < 0 && indicator != NO_TOTAL {
            out.truncate(start);
            return Err(decode_error(
                Some(self.column.as_str()),
                format!("二进制列返回了无法解释的长度指标 {indicator}"),
            ));
        }

        // 二进制没有「结尾 NUL」可言，只能信指标；`SQL_NO_TOTAL` 时以缓冲被填满为准。
        self.pending = (indicator != NO_TOTAL).then_some(indicator as usize);
        let written = if indicator == NO_TOTAL {
            want
        } else {
            (indicator as usize).min(want)
        };
        out.truncate(start + written);
        self.units += written;
        self.emitted += written;

        if finished {
            self.done = true;
            return Ok(ChunkState::Done);
        }
        if written == 0 {
            return Err(decode_error(
                Some(self.column.as_str()),
                "驱动在未取完时写入了 0 字节，无法推进",
            ));
        }
        Ok(ChunkState::More)
    }
}

/// 定位在**当前行**上的分块 LOB 读取器（[`LobState`] 的借用式视图）。
///
/// 它与 `LobState` 的关系是「视图 : 状态」：状态存在语句上，视图只借语句。老代码用它
/// （真机用例、`examples/`），`dm-query` 的 `LobStream` 则直接搬语句。
#[derive(Debug)]
pub struct LobRead<'a, 'conn> {
    stmt: &'a mut OdbcStatement<'conn>,
}

impl<'a, 'conn> LobRead<'a, 'conn> {
    /// 开始分块读某一列。`column_index` 从 1 起（ODBC 约定）。
    ///
    /// 这一步**不动驱动**：真正的第一趟发生在第一次 [`next_chunk`](Self::next_chunk)。
    /// 这样调用方可以先决定要不要读（比如已经知道这一列是 NULL 就跳过）。
    pub fn start(
        stmt: &'a mut OdbcStatement<'conn>,
        column_index: u16,
        meta: &ColumnMeta,
    ) -> Result<Self, DmError> {
        stmt.start_lob(column_index, meta)?;
        Ok(Self { stmt })
    }

    /// 打开分块读取器，并**自己把游标推进到下一行**（`SQLFetch`）。
    ///
    /// 与 [`start`](Self::start) 的区别只在「谁负责定位」：那个要求调用方已经把游标停在
    /// 某一行上（例如刚用 `result::fetch` 取完一批行），这个自己取一行，适合
    /// 「一条语句只读一列 LOB」这种用法。结果集已取尽时返回 `Ok(None)`。
    ///
    /// 注意：同一行内 `SQLGetData` 的列号必须递增，所以这一列之后的列不要再读
    /// （见模块文档第 2 条）。
    pub fn open(
        stmt: &'a mut OdbcStatement<'conn>,
        column_index: u16,
        meta: &ColumnMeta,
    ) -> Result<Option<Self>, DmError> {
        if !stmt.fetch_row()? {
            return Ok(None);
        }
        Self::start(stmt, column_index, meta).map(Some)
    }

    /// 这一列是 NULL（驱动在某一趟回了 `SQL_NULL_DATA`）。
    pub fn is_null(&self) -> bool {
        self.stmt.lob().is_some_and(LobState::is_null)
    }

    /// 是否已经取完。
    ///
    /// 状态不在语句上时（还没 `start`/`open`）算「取完」：没有状态就没有未交付的内容。
    pub fn is_done(&self) -> bool {
        self.stmt.lob().is_none_or(LobState::is_done)
    }

    /// 已交付给调用方的内容字节数（文本是 UTF-8 字节）。
    pub fn read_bytes(&self) -> usize {
        self.stmt.lob().map_or(0, LobState::read_bytes)
    }

    /// 列名（报错与日志用）。
    pub fn column(&self) -> &str {
        self.stmt.lob().map_or("", LobState::column)
    }

    /// 再取一趟，把**新增**的内容追加到 `out`（不清空 `out`）。
    ///
    /// 返回 [`ChunkState::Done`] 之后继续调用是无害的空转。
    pub fn next_chunk(&mut self, out: &mut Vec<u8>) -> Result<ChunkState, DmError> {
        let mut state = self.stmt.take_lob().ok_or_else(|| {
            decode_error(
                None,
                "分块读还没开始：先调用 LobRead::start/open（或 OdbcStatement::start_lob）",
            )
        })?;
        // 借出来 → 用 → 放回去：`next_chunk` 要用语句句柄发 `SQLGetData`，而状态本身
        // 也存在语句上，两者不能同时可变借用。
        let result = state.next_chunk(self.stmt, out);
        self.stmt.put_lob(state);
        result
    }
}

/// 把一趟码元切成「能安全解码的整码元」+「留给下一趟的高代理」。
///
/// 只可能挂在**结尾**的是高代理：低代理单独出现说明驱动给的就是畸形序列，交给
/// `String::from_utf16` 报错（宁可响亮失败，也不要把半个码元当值返回）。
fn split_chunk(units: &[u16]) -> Result<(String, Option<u16>), String> {
    let mut end = units.len();
    let held = if end > 0 && is_high_surrogate(units[end - 1]) {
        end -= 1;
        Some(units[end])
    } else {
        None
    };
    String::from_utf16(&units[..end])
        .map(|text| (text, held))
        .map_err(|e| format!("UTF-16 解码失败：{e}"))
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

/// 取不出来的值一律是 `Decode { column }` @ `Stage::Fetch`，细节进诊断记录。
///
/// 与 `result.rs` 的同名函数同一形状：这里自己写一份是为了不把 `result.rs` 的实现
/// 细节提升成 crate 级公共接口（`column` 允许 `None`：那几处失败与具体列无关）。
fn decode_error(column: Option<&str>, detail: impl std::fmt::Display) -> DmError {
    DmError::with_diags(
        DmErrorKind::Decode {
            column: column.map(str::to_owned),
        },
        vec![DiagRecord::native(0, detail.to_string())],
        ErrorContext::new(Stage::Fetch),
    )
}

/// 缓冲式 LOB 写入器：把值分片收进来，最后一次性变成参数值。
///
/// 它**不省内存**（值最终还是要在参数缓冲里完整出现一次），省的是「调用方必须先
/// 构造出一个完整的 `Vec<u8>`/`String`」——从文件、从迭代器、从多个来源拼一个大值
/// 时，这个差别就是「多一份拷贝」与「多一份完整拷贝 + 一份中间值」之间的差别。
///
/// 需要「整个值不进参数缓冲」时走延迟输入（[`LobFeeder`] +
/// [`OdbcStatement::exec_delayed`]）；两条路的分工见模块头。
///
/// `dm-query` 会把它重导出（冻结清单 §6.12 把 `LobWriter` 记在 `dm-query` 上）。
/// 定义留在这里是因为它本来就是本层的缓冲式写实现，与 [`LobFeeder`] 共享同一套
/// 「UTF-8 / UTF-16 / 二进制」的口径。
#[derive(Debug, Default)]
pub struct LobWriter {
    bytes: Vec<u8>,
}

impl LobWriter {
    /// 新建一个空写入器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 预留容量（已知大小时可以省掉几次扩容）。
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
        }
    }

    /// 追加一段字节（文本按 UTF-8 追加即可）。
    pub fn write(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// 追加一段文本。
    pub fn write_str(&mut self, text: &str) {
        self.bytes.extend_from_slice(text.as_bytes());
    }

    /// 已收进的字节数。
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// 是否还没收到任何字节。
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// 取走字节（二进制列用）。
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// 取走字节并按键 UTF-8 解释（文本列用）。收进来的不是合法 UTF-8 时返回
    /// `Decode { column: None }`——这一步没有列名可用，错误落在「值」上而不是「列」上。
    pub fn into_text(self) -> Result<String, DmError> {
        String::from_utf8(self.bytes)
            .map_err(|e| decode_error(None, format!("LOB 写入器的内容不是合法 UTF-8：{e}")))
    }
}

// ---------------------------------------------------------------------------
// 写侧：延迟输入（`SQL_NEED_DATA` / `SQLPutData`）
// ---------------------------------------------------------------------------

/// 文本源连续多少趟只给出「半个字符」就认定源头有问题。
///
/// 与 `dm-query::RowStream` 的 `MAX_EMPTY_BATCHES` 同一条纪律：坏源可以一直凑不出一个
/// 完整字符，但执行线程不能一直等——我们自己得有个台阶下。
const MAX_STALLED_TEXT_CHUNKS: usize = 8;

/// 把 [`LobSource`] 包成「驱动来要一趟、我们交一块」的喂数据器。
///
/// # 它既是状态也是 token
///
/// 延迟输入下，`SQLBindParameter` 收的 `ParameterValuePtr` **不是数据指针**，而是一个由
/// 驱动原样还回来的 token（`SQLParamData` 给出）。这里把 `self` 的地址当 token：驱动
/// 还回来时我们就重新拿到了这一处 LOB 的全部进度（读到哪、还剩多少、文本那半个字符）。
/// 所以绑定之后 [`LobFeeder`] **不能被移动**——它只应活在调用方的栈上，一次执行一个。
///
/// # 文本为什么要转 UTF-16
///
/// 达梦的宽字符参数要 UTF-16LE，而 [`LobSource`] 交付的是 UTF-8。转换放在这里，于是
/// 「一个字符被源块边界劈开」与「代理对被交付边界劈开」这两个坑只有这一个文件里有答案：
/// 前者靠 `carry`（不完整的 UTF-8 尾巴留到下一趟），后者靠 `text_end`（切点正好落在
/// 高代理之后时退一个码元）。
///
/// # 为什么要再切一刀
///
/// [`LobSource::next_chunk`] 的契约是「一趟最多追加 [`LOB_WRITE_CHUNK_BYTES`]」，但那是
/// 约定而不是类型保证；[`next_batch`](Self::next_batch) 自己**再按同一上限切片交付**，
/// 于是一次 `SQLPutData` 的长度永远可预期，坏源也没法让我们一口气推一个 GB 上去。
pub struct LobFeeder<'a> {
    /// 二进制还是文本（决定交付前的转换）。
    kind: LobKind,
    /// 数据源。**不拥有**：一次执行借一次，喂完就还。
    source: &'a mut dyn LobSource,
    /// 绑定时交给驱动的指标格（`SQL_LEN_DATA_AT_EXEC(n)` 或 `SQL_DATA_AT_EXEC`）。
    indicator: Len,
    /// 绑定时交给驱动的 `ColumnSize`。
    column_size: ULen,
    /// 二进制路径：源给的、还没交付完的字节。
    pending: Vec<u8>,
    /// `pending` 里已交付到哪。
    pending_pos: usize,
    /// 文本路径：已转成 UTF-16LE、还没交付完的字节。
    utf16: Vec<u8>,
    /// `utf16` 里已交付到哪。
    utf16_pos: usize,
    /// 文本路径：源上一趟末尾那个**不完整**的 UTF-8 字符（接回下一趟开头）。
    carry: Vec<u8>,
    /// 已经交付给驱动的字节数（诊断与测试用）。
    delivered: u64,
}

/// 手写而不是 `derive`：`source` 是 `&mut dyn LobSource`（trait 对象不实现 `Debug`），
/// 而 `#![warn(missing_debug_implementations)]` 要求每个公开类型都有 `Debug`。
/// 打印的是「交付进度」而不是源本身——那才是调试延迟输入时要看的东西。
impl std::fmt::Debug for LobFeeder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LobFeeder")
            .field("kind", &self.kind)
            .field("indicator", &self.indicator)
            .field("column_size", &self.column_size)
            .field("pending", &self.pending.len())
            .field("pending_pos", &self.pending_pos)
            .field("utf16", &self.utf16.len())
            .field("utf16_pos", &self.utf16_pos)
            .field("carry", &self.carry.len())
            .field("delivered", &self.delivered)
            .finish_non_exhaustive()
    }
}

impl<'a> LobFeeder<'a> {
    /// 包住一个数据源。`kind` 决定交付前的转换与绑定形状。
    ///
    /// 二进制源声明的长度超过 `isize` 上限时返回 `Encode { column: None }`：那是坏数据，
    /// 不该换算成一个负数交给驱动。
    pub fn new(kind: LobKind, source: &'a mut dyn LobSource) -> Result<Self, DmError> {
        let (indicator, column_size) = match kind {
            // 文本：`size_hint` 说的是 **UTF-8** 字节数，驱动要的是 UTF-16 字节数，两者
            // 一般不等 ⇒ 一律走「长度未知」。探针 6/7 实测两种形状驱动都收，而未知那条
            // 是流式文本唯一诚实的说法。
            LobKind::Text => (DATA_AT_EXEC, 0),
            LobKind::Binary => match source.size_hint() {
                Some(len) => (
                    len_data_at_exec(Len::try_from(len).map_err(|_| length_too_big(len))?),
                    len,
                ),
                None => (DATA_AT_EXEC, 0),
            },
        };
        Ok(Self {
            kind,
            source,
            indicator,
            column_size,
            pending: Vec::new(),
            pending_pos: 0,
            utf16: Vec::new(),
            utf16_pos: 0,
            carry: Vec::new(),
            delivered: 0,
        })
    }

    /// 值类型。
    pub fn kind(&self) -> LobKind {
        self.kind
    }

    /// 绑定时交给驱动的指标格地址。
    ///
    /// 必须是**稳定**地址：驱动在整次执行期间持有它，并可能回写实际长度。
    pub fn indicator_ptr(&mut self) -> *mut Len {
        &mut self.indicator
    }

    /// 绑定时交给驱动的 `ColumnSize`（文本是码元数、二进制是字节数；未知为 0）。
    pub fn column_size(&self) -> ULen {
        self.column_size
    }

    /// token：驱动会在 `SQLParamData` 里原样还回来（就是 `self` 的地址）。
    pub fn token(&mut self) -> Pointer {
        std::ptr::from_mut(self).cast()
    }

    /// 已经交付给驱动的字节数（诊断与测试用）。
    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    /// 取下**一批**交给驱动；`None` = 源已经取尽。
    ///
    /// 「追加」这件事由源负责（见 [`LobSource::next_chunk`]）；这里只管把源给的字节按
    /// 驱动能吃的粒度切好。
    pub fn next_batch(&mut self) -> Result<Option<&[u8]>, DmError> {
        match self.kind {
            LobKind::Binary => {
                if self.pending_pos >= self.pending.len() {
                    self.pending.clear();
                    self.pending_pos = 0;
                    if !self.source.next_chunk(&mut self.pending)? {
                        return Ok(None);
                    }
                }
                let end = (self.pending_pos + LOB_WRITE_CHUNK_BYTES).min(self.pending.len());
                let batch = &self.pending[self.pending_pos..end];
                self.pending_pos = end;
                self.delivered += batch.len() as u64;
                Ok(Some(batch))
            }
            LobKind::Text => {
                if self.utf16_pos >= self.utf16.len() && !self.refill_text()? {
                    return Ok(None);
                }
                let end = text_end(&self.utf16, self.utf16_pos);
                let batch = &self.utf16[self.utf16_pos..end];
                self.utf16_pos = end;
                self.delivered += batch.len() as u64;
                Ok(Some(batch))
            }
        }
    }

    /// 文本路径的补货：向源要一趟、验 UTF-8、把完整字符转成 UTF-16LE 铺进 `utf16`。
    ///
    /// 返回 `false` = 源取尽且没有残留（真正的结束）；`true` = `utf16` 里至少有一个码元。
    fn refill_text(&mut self) -> Result<bool, DmError> {
        self.utf16.clear();
        self.utf16_pos = 0;
        for _ in 0..MAX_STALLED_TEXT_CHUNKS {
            // 上一趟留下的半个字符接回这一趟的开头（`next_chunk` 是追加语义）。
            let mut buf = std::mem::take(&mut self.carry);
            if !self.source.next_chunk(&mut buf)? {
                if buf.is_empty() {
                    return Ok(false);
                }
                // 源在半路上结束了：最后一个字符只给了一半。少写一个字符比报错更糟。
                return Err(text_truncated(&buf));
            }

            // `error_len() == None` ⇒ 尾巴是个**被劈开**的 UTF-8 序列，留给下一趟；
            // 有长度 ⇒ 真的不是合法 UTF-8，立刻报（不猜、不替换字符）。
            let valid = match std::str::from_utf8(&buf) {
                Ok(_) => buf.len(),
                Err(err) if err.error_len().is_none() => err.valid_up_to(),
                Err(err) => return Err(text_invalid(&err)),
            };
            if valid == 0 {
                self.carry = buf;
                continue;
            }

            self.carry = buf.split_off(valid);
            let text = std::str::from_utf8(&buf).map_err(|err| text_invalid(&err))?;
            self.utf16.reserve(text.len().saturating_mul(2));
            for unit in text.encode_utf16() {
                self.utf16.extend_from_slice(&unit.to_le_bytes());
            }
            if !self.utf16.is_empty() {
                return Ok(true);
            }
        }
        Err(text_stalled())
    }
}

/// 文本交付的切点：一趟不超过 [`LOB_WRITE_CHUNK_BYTES`] 字节，且**不切开代理对**。
///
/// 切点落在「高代理之后、低代理之前」时退回一个码元。`utf16` 只由完整字符拼成，所以
/// 绝不会出现「一个孤立代理占满整趟」导致退回后长度为 0 的情况（一次至少推一个字符）。
fn text_end(buf: &[u8], pos: usize) -> usize {
    let end = (pos + LOB_WRITE_CHUNK_BYTES).min(buf.len());
    if end < buf.len() && end >= pos + 2 {
        let last = u16::from_le_bytes([buf[end - 2], buf[end - 1]]);
        if is_high_surrogate(last) {
            return end - 2;
        }
    }
    end
}

/// 二进制源声明的长度换算不成 `isize`：坏数据，不是坏连接。
fn length_too_big(len: usize) -> DmError {
    DmError::with_diags(
        DmErrorKind::Encode { column: None },
        vec![DiagRecord::native(
            0,
            format!("待写 LOB 声明的长度 {len} 字节超过 isize 上限，无法作为 ODBC 长度指标"),
        )],
        ErrorContext::new(Stage::Execute),
    )
}

/// 文本源给了不是合法 UTF-8 的字节。
///
/// 故意**不是** `DmErrorKind::Io`：那条路会被当成「连接坏了」（
/// `DmErrorKind::is_connection_lost`），池子会白扔一条好连接。这是**参数编码**失败。
fn text_invalid(err: &std::str::Utf8Error) -> DmError {
    DmError::with_diags(
        DmErrorKind::Encode { column: None },
        vec![DiagRecord::native(
            0,
            format!(
                "待写 LOB 文本不是合法 UTF-8（第 {} 字节起）：{err}",
                err.valid_up_to()
            ),
        )],
        ErrorContext::new(Stage::Execute),
    )
}

/// 文本源结束了，但最后一个字符只给了一半。
fn text_truncated(tail: &[u8]) -> DmError {
    DmError::with_diags(
        DmErrorKind::Encode { column: None },
        vec![DiagRecord::native(
            0,
            format!(
                "待写 LOB 文本以不完整的 UTF-8 序列结尾（余下 {} 字节 {tail:02X?}）",
                tail.len()
            ),
        )],
        ErrorContext::new(Stage::Execute),
    )
}

/// 文本源连续 [`MAX_STALLED_TEXT_CHUNKS`] 趟都只给出半个字符。
fn text_stalled() -> DmError {
    DmError::with_diags(
        DmErrorKind::Encode { column: None },
        vec![DiagRecord::native(
            0,
            format!("待写 LOB 文本源连续 {MAX_STALLED_TEXT_CHUNKS} 趟只给出半个字符"),
        )],
        ErrorContext::new(Stage::Execute),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_that_does_not_end_on_a_surrogate_decodes_whole() {
        let units: Vec<u16> = "达梦A".encode_utf16().collect();
        let (text, held) = split_chunk(&units).expect("能解码");
        assert_eq!(text, "达梦A");
        assert_eq!(held, None);
    }

    #[test]
    fn a_split_surrogate_pair_is_held_back_and_rejoined() {
        // 「🀄」= U+1F004 = D83C DC04。一趟只取到高代理。
        let first = [0x0041u16, 0xD83C];
        let (text, held) = split_chunk(&first).expect("前半段本身是合法的（没有孤立低代理）");
        assert_eq!(text, "A");
        assert_eq!(held, Some(0xD83C));

        // 下一趟把留下的高代理放在开头，配上低代理就还原成一个字符。
        let second = [0xD83C, 0xDC04, 0x0042];
        let (text, held) = split_chunk(&second).expect("拼起来是合法的");
        assert_eq!(text, "🀄B");
        assert_eq!(held, None);
    }

    #[test]
    fn a_lone_low_surrogate_is_an_error_not_a_replacement_character() {
        let units = [0xDC04u16];
        let err = split_chunk(&units).expect_err("孤立低代理必须报错");
        assert!(err.contains("UTF-16"), "{err}");
    }

    #[test]
    fn an_empty_chunk_is_fine() {
        let (text, held) = split_chunk(&[]).expect("空趟不是错误");
        assert!(text.is_empty());
        assert_eq!(held, None);
    }

    #[test]
    fn writer_round_trips_text_and_bytes() {
        let mut writer = LobWriter::with_capacity(8);
        writer.write_str("达梦");
        writer.write("🀄".as_bytes());
        let text = writer.into_text().expect("合法 UTF-8");
        assert_eq!(text, "达梦🀄");

        let mut writer = LobWriter::new();
        assert!(writer.is_empty());
        writer.write(&[1, 2, 3]);
        assert_eq!(writer.len(), 3);
        assert_eq!(writer.into_bytes(), vec![1, 2, 3]);
    }

    #[test]
    fn writer_rejects_invalid_utf8_for_text() {
        let mut writer = LobWriter::new();
        writer.write(&[0xE8, 0xBE]); // 「达」的前两字节
        let err = writer.into_text().expect_err("截断的 UTF-8 必须报错");
        assert_eq!(err.context.stage, Stage::Fetch);
        assert!(matches!(err.kind, DmErrorKind::Decode { column: None }));
    }

    #[test]
    fn the_chunk_window_is_four_kibibytes_wide_or_byte_wise() {
        assert_eq!(LOB_CHUNK_BYTES, 4096);
        assert_eq!(LOB_CHUNK_BYTES / size_of::<u16>(), 2048);
    }

    /// 文本列走宽字符窗口、二进制列走字节窗口，且非 LOB 类型要在 `new` 就拒绝。
    ///
    /// 这三条以前只写在代码里没被测过；`LobState` 变成公开类型之后，它们是「上层拿到的
    /// 窗口大小对不对」的唯一防线。
    #[test]
    fn new_picks_the_window_by_type_and_rejects_non_lob_columns() {
        let text = ColumnMeta::new("C", DataType::LongVarChar);
        let state = LobState::new(1, &text).expect("文本列可以分块读");
        assert!(state.wide);
        assert_eq!(state.chunk, LOB_CHUNK_BYTES / size_of::<u16>());
        assert_eq!(state.column(), "C");
        assert!(!state.is_null());
        assert!(!state.is_done());
        assert_eq!(state.read_bytes(), 0);

        let binary = ColumnMeta::new("B", DataType::LongVarBinary);
        let state = LobState::new(2, &binary).expect("二进制列可以分块读");
        assert!(!state.wide);
        assert_eq!(state.chunk, LOB_CHUNK_BYTES);

        let number = ColumnMeta::new("N", DataType::Integer);
        let err = LobState::new(3, &number).expect_err("整数列分块读没有意义");
        assert!(matches!(err.kind, DmErrorKind::Decode { column: Some(_) }));
        assert_eq!(err.context.stage, Stage::Fetch);
    }

    // -----------------------------------------------------------------------
    // 写侧喂数据器（`LobFeeder`）
    // -----------------------------------------------------------------------

    /// 按调用方给的批次逐趟交付的源：用来把「源怎么切」与「驱动怎么要」分开摆布。
    struct Scripted {
        /// 还没交出去的批次（正序）。
        batches: std::collections::VecDeque<Vec<u8>>,
        /// 声明给驱动的总长度（`None` = 长度未知）。
        hint: Option<usize>,
    }

    impl Scripted {
        fn new(batches: Vec<Vec<u8>>, hint: Option<usize>) -> Self {
            Self {
                batches: batches.into(),
                hint,
            }
        }

        /// 把 `bytes` 按固定宽度切开（最后一块可能短）。
        fn sliced(bytes: &[u8], width: usize) -> Self {
            Self::new(
                bytes.chunks(width).map(<[u8]>::to_vec).collect(),
                Some(bytes.len()),
            )
        }
    }

    impl LobSource for Scripted {
        fn size_hint(&self) -> Option<usize> {
            self.hint
        }

        fn next_chunk(&mut self, out: &mut Vec<u8>) -> Result<bool, DmError> {
            match self.batches.pop_front() {
                Some(batch) => {
                    out.extend_from_slice(&batch);
                    Ok(true)
                }
                None => Ok(false),
            }
        }
    }

    /// 把喂数据器交付的全部字节收起来（同时检查每趟都不超过上限）。
    fn drain_feeder(feeder: &mut LobFeeder<'_>) -> Vec<u8> {
        let mut all = Vec::new();
        while let Some(batch) = feeder.next_batch().expect("源不会失败") {
            assert!(
                batch.len() <= LOB_WRITE_CHUNK_BYTES,
                "一趟交付了 {} 字节，超过上限 {LOB_WRITE_CHUNK_BYTES}",
                batch.len()
            );
            all.extend_from_slice(batch);
        }
        all
    }

    /// 把交付出去的 UTF-16LE 字节还原成码元序列（三处文本断言共用）。
    fn utf16_units(bytes: &[u8]) -> Vec<u16> {
        let (pairs, rest) = bytes.as_chunks::<2>();
        assert!(rest.is_empty(), "UTF-16LE 的交付长度必须是偶数");
        pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect()
    }

    #[test]
    fn a_binary_feeder_declares_the_exact_length() {
        let mut source = Scripted::sliced(b"hello dameng", 4);
        let mut feeder = LobFeeder::new(LobKind::Binary, &mut source).expect("长度合法");
        // 绑定形状：`SQL_LEN_DATA_AT_EXEC(n)` + `ColumnSize = n`。
        assert_eq!(feeder.indicator, len_data_at_exec(12));
        assert_eq!(feeder.column_size(), 12);
        assert_eq!(feeder.kind(), LobKind::Binary);
        assert_eq!(drain_feeder(&mut feeder), b"hello dameng");
        assert_eq!(feeder.delivered(), 12);
    }

    #[test]
    fn a_binary_feeder_without_a_hint_falls_back_to_data_at_exec() {
        let mut source = Scripted::new(vec![b"abc".to_vec(), b"de".to_vec()], None);
        let mut feeder = LobFeeder::new(LobKind::Binary, &mut source).expect("未知长度也要能绑");
        assert_eq!(feeder.indicator, DATA_AT_EXEC);
        assert_eq!(feeder.column_size(), 0);
        assert_eq!(drain_feeder(&mut feeder), b"abcde");
    }

    #[test]
    fn a_binary_feeder_never_hands_out_more_than_one_window() {
        // 源故意不守约：一趟给 3 个窗口的量。
        let big = vec![7u8; LOB_WRITE_CHUNK_BYTES * 3 + 11];
        let mut source = Scripted::new(vec![big.clone()], Some(big.len()));
        let mut feeder = LobFeeder::new(LobKind::Binary, &mut source).expect("长度合法");
        let mut windows = 0;
        while let Some(batch) = feeder.next_batch().expect("源不会失败") {
            assert!(batch.len() <= LOB_WRITE_CHUNK_BYTES);
            windows += 1;
        }
        assert_eq!(windows, 4, "3 个满窗 + 一个尾巴");
        assert_eq!(feeder.delivered(), big.len() as u64);
    }

    #[test]
    fn a_text_feeder_always_uses_data_at_exec() {
        // 文本的 `size_hint` 是 UTF-8 字节数，驱动要的是 UTF-16 字节数：不能拿去当长度。
        let text = "达梦".as_bytes().to_vec();
        let mut source = Scripted::new(vec![text.clone()], Some(text.len()));
        let mut feeder = LobFeeder::new(LobKind::Text, &mut source).expect("文本源不需要长度");
        assert_eq!(feeder.indicator, DATA_AT_EXEC);
        assert_eq!(feeder.column_size(), 0);
        assert_eq!(feeder.kind(), LobKind::Text);

        let delivered = drain_feeder(&mut feeder);
        let units = utf16_units(&delivered);
        assert_eq!(String::from_utf16(&units).expect("合法的 UTF-16"), "达梦");
    }

    #[test]
    fn a_text_feeder_carries_a_character_split_across_source_batches() {
        // 「达」= E8 BE BE（3 字节）。切开 1/2 与 2/1，两处都断在字符中间。
        let text = "达A梦";
        let bytes = text.as_bytes();
        let mut source = Scripted::new(
            vec![
                bytes[..2].to_vec(),
                bytes[2..4].to_vec(),
                bytes[4..].to_vec(),
            ],
            None,
        );
        let mut feeder = LobFeeder::new(LobKind::Text, &mut source).expect("文本源不需要长度");
        let delivered = drain_feeder(&mut feeder);
        let units = utf16_units(&delivered);
        assert_eq!(String::from_utf16(&units).expect("合法的 UTF-16"), text);
        assert_eq!(delivered.len(), text.encode_utf16().count() * 2);
    }

    #[test]
    fn a_text_feeder_never_splits_a_surrogate_pair() {
        // 「😀」= U+1F600 = D83D DE00：给它制造一个「切点正好落在高代理之后」的窗口。
        let text = "😀".repeat(40);
        let bytes = text.as_bytes();
        let mut source = Scripted::sliced(bytes, 3); // 源的切点根本不看字符边界
        let mut feeder = LobFeeder::new(LobKind::Text, &mut source).expect("文本源不需要长度");
        let mut delivered = Vec::new();
        while let Some(batch) = feeder.next_batch().expect("源不会失败") {
            assert_eq!(batch.len() % 2, 0, "UTF-16LE 的交付长度必须是偶数");
            if let [.., high, low] = batch
                && (0xD800..=0xDBFF).contains(&u16::from_le_bytes([*high, *low]))
            {
                panic!("一趟以孤立高代理结尾：驱动会把它和下一趟的低代理拼错");
            }
            delivered.extend_from_slice(batch);
        }
        let units = utf16_units(&delivered);
        assert_eq!(String::from_utf16(&units).expect("合法的 UTF-16"), text);
    }

    #[test]
    fn a_text_feeder_rejects_invalid_utf8_as_an_encode_error() {
        let mut source = Scripted::new(vec![vec![0xFF, 0xFE]], None);
        let mut feeder = LobFeeder::new(LobKind::Text, &mut source).expect("文本源不需要长度");
        let err = feeder.next_batch().expect_err("非法 UTF-8 必须报错");
        assert!(matches!(err.kind, DmErrorKind::Encode { column: None }));
        assert_eq!(err.context.stage, Stage::Execute);
        // 连接没问题：归 `Encode` 而不是 `Io`，池照常复用这条连接。
        assert!(!err.is_connection_lost());
    }

    #[test]
    fn a_text_feeder_rejects_a_truncated_trailing_character() {
        // 「达」的前两字节，源就到此为止。
        let mut source = Scripted::new(vec![vec![0xE8, 0xBE]], None);
        let mut feeder = LobFeeder::new(LobKind::Text, &mut source).expect("文本源不需要长度");
        let err = feeder.next_batch().expect_err("截断的 UTF-8 必须报错");
        assert!(matches!(err.kind, DmErrorKind::Encode { column: None }));
        assert!(
            err.diags().iter().any(|d| d.message.contains("不完整")),
            "诊断里要说清是「不完整」而不是「非法」：{:?}",
            err.diags()
        );
    }
}
