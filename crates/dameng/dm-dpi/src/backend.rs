//! [`Backend`] 的 DPI 实现：直连 `dmdpi` 共享库，不经过 ODBC 层。
//!
//! # 与 `dm-odbc` 的对等关系
//!
//! 两者实现同一个 [`Backend`] trait，因此 `dm-pool`/`dm-query`/`dm-migrate`/
//! `toasty-driver-dameng` 一行都不用改就能跑在 DPI 上。差别只在「怎么跟驱动说话」：
//!
//! | 关注点 | `dm-odbc` | 本实现 |
//! |---|---|---|
//! | 字符集 | 连接串 + `unicode()` 自检 | 建连前 `DSQL_ATTR_LOCAL_CODE = UTF8` |
//! | 默认模式 | `SET SCHEMA`（连接串 `SCHEMA=` 被驱动忽略） | 同（DPI 的 `CURRENT_SCHEMA` 属性只读） |
//! | 错误消息 | 驱动给什么算什么（GB18030） | 建连前 `DSQL_ATTR_LANG_ID = EN`，ASCII |
//! | 参数 | 数组绑定（`PARAMSET_SIZE`） | 逐行 `dpi_bind_param2`（数组绑定待补） |
//! | LOB 读 | 分块 `SQLGetData` | 问长度 + 一次读完（见 [`read_long`]） |
//! | LOB 写 | `SQL_DATA_AT_EXEC` + `SQLPutData` | `DSQL_DATA_AT_EXEC` + `dpi_put_data` |
//!
//! # 三条实测约束（都来自 `crates/dm-dpi/examples/` 里的探针）
//!
//! 1. **列数据是消费式的**：`dpi_get_data` 在同一行里对同一列只能读一次，第二次返回
//!    `DSQL_NO_DATA`。因此取数必须**一次选对 C 类型**，不能「先试小的再换大的」。
//! 2. **文本走本地字符集 + `LOCAL_CODE=UTF8`**：窄字节路径在设了 `LOCAL_CODE` 之后
//!    是正确的 UTF-8；`DSQL_C_WCHAR` 反而只在我们没设字符集时才对。
//! 3. **`fraction` 是纳秒**：`dpi_timestamp_t.fraction = 123456000` 对应 `.123456`。

use std::sync::Arc;

use chrono::{Datelike, Timelike};
use dm_core::backend::{
    Backend, ExecOutcome, FetchState, LobChunk, LobParam, LobSource, ParamArrayOutcome, RowBuf,
};
use dm_core::datatype::DataType;
use dm_core::{ColumnMeta, DmConfig, DmValue};
use dm_error::{DiagRecord, DmError, DmErrorKind, Stage};

use crate::error::DpiError;
use crate::ffi::{SLength, attr, autocommit, codeset, ctype, indicator, param, sqltype};
use crate::handle::{Conn, Dpi, Env, Stmt, Timestamp};

/// 一次 [`Backend::fetch`] 取多少行。
///
/// 取 1 行与 `dm-odbc` 的节奏一致（`RowStream` 逐行往上交），也让「行内哪一列读失败」
/// 的定位与 ODBC 路径完全一样。批量取数（`ROW_ARRAY_SIZE` + `dpi_bind_col`）留待性能优化。
const FETCH_ROWS: usize = 1;

/// 一次 `dpi_get_data` 能交付的字节上限（实测 65535/65536）。
const MAX_GET_DATA: usize = 64 * 1024;
/// 单列最多愿意为「一次读完」开多大的缓冲（防止元数据给出荒唐长度时把内存打爆）。
const MAX_LONG_BYTES: usize = 256 * 1024 * 1024;

/// DPI 后端。每个实例持有一条物理连接。
pub struct DpiBackend {
    /// 运行时（函数表）。进程内是同一个 `&'static Api`。
    dpi: Dpi,
    /// 环境：建连前设字符集/语言，必须比连接活得久。
    env: Env,
    /// 物理连接。
    conn: Conn,
    /// 参数缓冲：DPI 只存指针，值的字节必须活到 `exec` 之后。
    arena: ParamArena,
}

impl DpiBackend {
    /// 建连。
    ///
    /// 顺序**不能变**：设字符集/语言 → 登录 → `SET SCHEMA` → `init_sql`。
    /// 字符集必须在登录握手前设（登录后设不生效，实测 `[-70023]`）。
    ///
    /// # Errors
    ///
    /// 共享库加载失败（`DriverLoad`）、登录失败（`Connect`）、`SET SCHEMA`/`init_sql`
    /// 失败（`Config`）。
    fn open(cfg: &DmConfig) -> Result<Self, DmError> {
        let dpi = Dpi::open().map_err(|e| to_dm(e, Stage::DriverLoad))?;
        let env = dpi.env().map_err(|e| to_dm(e, Stage::Connect))?;
        env.set_local_code(codeset::UTF8)
            .map_err(|e| to_dm(e, Stage::Connect))?;
        env.set_attr_i32(attr::LANG_ID, 1)
            .map_err(|e| to_dm(e, Stage::Connect))?;

        let conn = env.connect().map_err(|e| to_dm(e, Stage::Connect))?;
        let server = format!("{}:{}", cfg.server, cfg.port);
        conn.login(&server, &cfg.user, cfg.password.expose())
            .map_err(|e| {
                DmError::at(
                    DmErrorKind::Connect {
                        source: e.to_string(),
                    },
                    Stage::Connect,
                )
            })?;

        if let Some(schema) = cfg.schema.as_deref() {
            let sql = format!("SET SCHEMA \"{}\"", schema.replace('"', "\"\""));
            conn.exec_direct(&sql).map_err(|e| {
                DmError::at(
                    DmErrorKind::Config(format!("默认模式 {schema:?} 设置失败：{e}")),
                    Stage::Config,
                )
            })?;
        }

        for sql in &cfg.init_sql {
            conn.exec_direct(sql).map_err(|e| {
                DmError::at(
                    DmErrorKind::Config(format!("init_sql 执行失败：{e}")),
                    Stage::Config,
                )
            })?;
        }

        Ok(Self {
            dpi,
            env,
            conn,
            arena: ParamArena::default(),
        })
    }

    /// 执行完一条语句后的收尾：有结果集就描述列，否则读影响行数。
    fn finish(&mut self, stmt: &mut DpiStmt) -> Result<ExecOutcome, DmError> {
        let count = stmt
            .inner
            .column_count()
            .map_err(|e| to_dm(e, Stage::Execute))?;
        if count == 0 {
            let rows = stmt
                .inner
                .affected_rows()
                .map_err(|e| to_dm(e, Stage::Execute))?;
            return Ok(ExecOutcome::Affected { rows: Some(rows) });
        }
        let columns = self.describe(stmt)?;
        // 达梦取尽结果集后语句句柄就报废（`-70027` 一族），查询语句不进缓存。
        stmt.produced_result_set = true;
        Ok(ExecOutcome::ResultSet { columns })
    }

    /// 取（并缓存）列元数据。
    fn describe(&mut self, stmt: &mut DpiStmt) -> Result<Arc<[ColumnMeta]>, DmError> {
        if let Some(columns) = &stmt.columns {
            return Ok(Arc::clone(columns));
        }
        let descs = stmt.inner.columns().map_err(|e| to_dm(e, Stage::Fetch))?;
        let mut columns = Vec::with_capacity(descs.len());
        for desc in descs {
            columns.push(column_meta(&desc));
        }
        let columns: Arc<[ColumnMeta]> = columns.into();
        stmt.columns = Some(Arc::clone(&columns));
        Ok(columns)
    }
}

impl Backend for DpiBackend {
    type Stmt = DpiStmt;

    fn name(&self) -> &'static str {
        "dm-dpi"
    }

    fn connect(cfg: &DmConfig) -> Result<Self, DmError> {
        Self::open(cfg)
    }

    /// 探活：读 `DSQL_ATTR_CONNECTION_DEAD`，**不发 SQL**（与 ODBC 侧同口径）。
    fn ping(&mut self) -> Result<bool, DmError> {
        match self.conn.get_attr_i32(attr::CONNECTION_DEAD) {
            Ok(dead) => Ok(dead == 0),
            // 属性读不出来不等于连接已死：交给调用方按「未知」处理（返回 false 会让池
            // 丢弃一条可能健康的连接，代价更大）。
            Err(err) => {
                tracing::debug!(%err, "读取 CONNECTION_DEAD 失败，探活按连接仍可用处理");
                Ok(true)
            }
        }
    }

    fn prepare(&mut self, sql: &str) -> Result<Self::Stmt, DmError> {
        let inner = self.conn.stmt().map_err(|e| to_dm(e, Stage::Prepare))?;
        inner.prepare(sql).map_err(|e| to_dm(e, Stage::Prepare))?;
        Ok(DpiStmt {
            inner,
            prepared: true,
            produced_result_set: false,
            executed: false,
            columns: None,
            lob_column: None,
        })
    }

    fn bind_params(&mut self, stmt: &mut Self::Stmt, args: &[DmValue]) -> Result<(), DmError> {
        self.arena.clear();
        // 先把所有参数的字节追加进缓冲区，再统一取地址绑定：`Vec` 在追加期间可能搬移，
        // 绑早了会拿到失效指针。
        let mut encoded = Vec::with_capacity(args.len());
        for value in args {
            encoded.push(self.arena.encode(value)?);
        }
        // 文本、时间与十进制的「类型 + 长度」以服务端参数描述为准：`dmPython` 也是
        // 这么拿的（`var.c:327` 的 `sql_type` / `prec` / `scale` 全部来自
        // `dpi_desc_param`）。用自己猜的 `VARCHAR` + 精度 0 绑文本会 `String truncated`，
        // 绑长数值会 `Data out of range`；`#[document]` 里的时间字段在库里是 JSON 文本，
        // 只有照描述符（`VARCHAR`）绑才能和文本形态对上（官方套件
        // `type_document::struct_embed_filter_temporal`）。
        //
        // 但**数值参数不能照抄描述符的类型**：像 `JSON_CONTAINS(col, JSON_ARRAY(?))`
        // 这种位置，描述符会报成 `VARCHAR(8188)`，照抄就把整数当成字符串交给达梦，
        // `JSON_ARRAY(?)` 于是构造出 `["1"]` 而不是 `[1]`，与库里的 `[1, 2]` 对不上
        // ⇒ 官方套件 `embed_enum_collection::vec_enum_contains_and_len` 少两行
        // （Round 41 实测：`dtype=VARCHAR` 时命中字符串目标，`dtype=INTEGER` 时命中
        // 数字目标）。整数与浮点保留自己的 SQL 类型，其余按描述符走。
        for (index, param) in encoded.iter_mut().enumerate() {
            let position = u16::try_from(index + 1).unwrap_or(u16::MAX);
            let keep_numeric = matches!(
                param.ctype,
                ctype::STINYINT
                    | ctype::SSHORT
                    | ctype::SLONG
                    | ctype::SBIGINT
                    | ctype::FLOAT
                    | ctype::DOUBLE
            );
            if keep_numeric {
                continue;
            }
            if let Ok(desc) = stmt.inner.param_desc(position) {
                param.dtype = desc.sql_type;
                param.precision = desc.precision;
                param.scale = desc.scale;
            }
        }
        for (index, param) in encoded.into_iter().enumerate() {
            let index = u16::try_from(index + 1).map_err(|_| {
                DmError::at(
                    DmErrorKind::Config(format!("参数个数 {} 超出 u16", args.len())),
                    Stage::Bind,
                )
            })?;
            self.arena.bind(&stmt.inner, index, param)?;
        }
        Ok(())
    }

    fn execute(&mut self, stmt: &mut Self::Stmt, sql: &str) -> Result<ExecOutcome, DmError> {
        if !stmt.prepared {
            stmt.inner
                .prepare(sql)
                .map_err(|e| to_dm(e, Stage::Prepare))?;
            stmt.prepared = true;
        }
        // 重新执行前必须关游标：达梦对「游标还开着又 exec」报 `-70027` 一族。
        // 但**刚 prepare 的语句绝不能先关**：实测会让 dpi_exec 静默不生效。
        if stmt.executed {
            stmt.inner.close_cursor();
        }
        stmt.columns = None;
        stmt.inner.exec().map_err(|e| to_dm(e, Stage::Execute))?;
        stmt.executed = true;
        self.finish(stmt)
    }

    fn statement_reusable(&self, stmt: &Self::Stmt) -> bool {
        !stmt.produced_result_set
    }

    fn param_count(&mut self, stmt: &Self::Stmt) -> Option<u16> {
        stmt.inner.param_count().ok()
    }

    /// 批量写：**暂用默认的逐行循环**。
    ///
    /// DPI 的数组绑定已经探明可用（`PARAMSET_SIZE` + `dpi_bind_param2`，实测 28.9 万行/秒，
    /// 比逐行快 37–50 倍），但列式缓冲要按每列的类型固定 C 类型，实现放在下一步；
    /// 语义与 ODBC 路径一致（行级失败记 `failed_at`），只是慢。
    /// 列式参数数组：一次 `dpi_exec` 打完一整批。
    ///
    /// 实测（`target/m38_probe2.log`）：500 行 / 1.726 ms ≈ **289 670 行/秒**，逐行
    /// `exec` 的对照组是 7 675 行/秒 ⇒ 约 37 倍。这里也是唯一能对齐 `dm-odbc` 批量
    /// 失败口径的地方：达梦对数组绑定里的约束违约返回**语句级**错误（`-6602`），整批
    /// 不落地；逐行循环则会留下前面已提交的行，语义就不一样了。
    fn execute_param_array(
        &mut self,
        stmt: &mut Self::Stmt,
        columns: &[Vec<DmValue>],
    ) -> Result<ParamArrayOutcome, DmError> {
        let rows = columns.iter().map(Vec::len).min().unwrap_or(0);
        if rows == 0 || columns.is_empty() {
            return Ok(ParamArrayOutcome::default());
        }
        if !stmt.prepared {
            return Err(DmError::at(
                DmErrorKind::Config("execute_param_array 之前必须先 prepare".to_string()),
                Stage::Bind,
            ));
        }
        stmt.inner.close_cursor();
        stmt.executed = false;
        stmt.columns = None;

        // 1. 每列编码成**定长数组**。
        //
        // `dpi_bind_param2` 的 `buf_len` 是**单个元素**的字节数（`dmPython/var.c:104`
        // 的 `bufferSize = type->size`），不是总字节数——传总数会让驱动按错误步长
        // 越界读，进程直接死（实测如此，见 `target/m38_probe2.log` 的前一版）。
        let mut buffers = Vec::with_capacity(columns.len());
        for column in columns {
            buffers.push(ColumnBuffer::build(column, rows)?);
        }

        stmt.inner
            .set_attr_i32(attr::PARAMSET_SIZE, i32::try_from(rows).unwrap_or(i32::MAX))
            .map_err(|e| to_dm(e, Stage::Bind))?;
        stmt.inner
            .unbind_params()
            .map_err(|e| to_dm(e, Stage::Bind))?;

        // 2. 逐列绑定。缓冲/指示符/实际长度数组都活到 `exec` 之后（`buffers` 在本函数
        //    末尾才释放），驱动只按 `PARAMSET_SIZE` 读，不持有指针。
        for (index, buffer) in buffers.iter_mut().enumerate() {
            let param = u16::try_from(index + 1).unwrap_or(u16::MAX);
            // SAFETY: 指针来自 `buffer` 的三个 `Vec`，长度与 `PARAMSET_SIZE` 一致。
            let result = unsafe {
                stmt.inner.bind_param2(
                    param,
                    param::INPUT,
                    buffer.ctype,
                    buffer.dtype,
                    0,
                    0,
                    buffer.data.as_mut_ptr().cast(),
                    SLength::try_from(buffer.stride).unwrap_or(0),
                    buffer.inds.as_mut_ptr(),
                    buffer.acts.as_mut_ptr(),
                )
            };
            result.map_err(|e| to_dm(e, Stage::Bind))?;
        }

        let executed = stmt.inner.exec();

        // 3. 收尾：**无论成败**都要解绑并还原 `PARAMSET_SIZE`，否则同一条语句后面
        //    跑单行会带着数组绑定状态（`real_statement_reuse_after_array_binding_*`）。
        let _ = stmt.inner.unbind_params();
        let _ = stmt.inner.set_attr_i32(attr::PARAMSET_SIZE, 1);
        executed.map_err(|e| to_dm(e, Stage::Execute))?;

        stmt.executed = true;
        let affected = stmt
            .inner
            .affected_rows()
            .map_err(|e| to_dm(e, Stage::Execute))?;
        Ok(ParamArrayOutcome {
            affected,
            executed: rows,
            failed_at: None,
        })
    }

    fn columns(&mut self, stmt: &Self::Stmt) -> Result<Arc<[ColumnMeta]>, DmError> {
        match &stmt.columns {
            Some(columns) => Ok(Arc::clone(columns)),
            None => Err(DmError::at(
                DmErrorKind::Config("语句还没产出结果集：先 execute，再取列元数据".to_string()),
                Stage::Fetch,
            )),
        }
    }

    fn fetch(&mut self, stmt: &mut Self::Stmt, into: &mut RowBuf) -> Result<FetchState, DmError> {
        let columns = self.describe(stmt)?;
        if columns.len() != into.columns() {
            return Err(DmError::at(
                DmErrorKind::Decode {
                    column: Some("<结果集>".to_string()),
                },
                Stage::Fetch,
            ));
        }
        // 注意：**不能**清空 `into`——`RowBuf` 跨多次 `fetch` 累积行，
        // 清空会把上一趟取到的行一起丢掉（表现为「查询零行」）。
        let mut rows = 0;
        while rows < FETCH_ROWS {
            if !stmt.inner.fetch().map_err(|e| to_dm(e, Stage::Fetch))? {
                break;
            }
            let mut row = Vec::with_capacity(columns.len());
            for (index, meta) in columns.iter().enumerate() {
                let index = u16::try_from(index + 1).unwrap_or(u16::MAX);
                row.push(read_value(&stmt.inner, index, meta)?);
            }
            into.push_row(&row)?;
            rows += 1;
        }
        if rows == 0 {
            return Ok(FetchState::Exhausted);
        }
        Ok(FetchState::Rows { rows })
    }

    fn close_cursor(&mut self, stmt: &mut Self::Stmt) -> Result<(), DmError> {
        if stmt.executed {
            stmt.inner.close_cursor();
            stmt.executed = false;
        }
        Ok(())
    }

    fn lob_start(&mut self, stmt: &mut Self::Stmt, column: usize) -> Result<(), DmError> {
        stmt.lob_column = Some(column);
        Ok(())
    }

    fn lob_next_row(&mut self, stmt: &mut Self::Stmt) -> Result<bool, DmError> {
        stmt.inner.fetch().map_err(|e| to_dm(e, Stage::Fetch))
    }

    /// 取一趟 LOB。
    ///
    /// 本实现**一趟交完**：先问长度，再一次性读进 `out`。`dpi_get_data` 的分块模式对文本
    /// 列会在 64 KiB 边界多塞 NUL 终止符（实测），整块读没有这个问题；代价是峰值内存等于
    /// 值本身，`RowBuf` 路径本来也是这样。
    fn lob_chunk(&mut self, stmt: &mut Self::Stmt, out: &mut Vec<u8>) -> Result<LobChunk, DmError> {
        let Some(column) = stmt.lob_column else {
            return Err(DmError::at(
                DmErrorKind::Config("lob_chunk 之前必须先调用 lob_start".to_string()),
                Stage::Fetch,
            ));
        };
        let columns = self.describe(stmt)?;
        let meta = columns.get(column).ok_or_else(|| {
            DmError::at(
                DmErrorKind::Config(format!("LOB 列下标 {column} 超出结果集列数")),
                Stage::Fetch,
            )
        })?;
        let index = u16::try_from(column + 1).unwrap_or(u16::MAX);
        let value_ctype = match meta.data_type.category() {
            dm_core::datatype::TypeCategory::Binary => ctype::BINARY,
            _ => ctype::CHAR,
        };
        match read_long(&stmt.inner, index, value_ctype).map_err(|e| to_dm(e, Stage::Fetch))? {
            None => Ok(LobChunk::Null),
            Some(bytes) => {
                out.extend_from_slice(&bytes);
                Ok(LobChunk::Done)
            }
        }
    }

    /// 执行一条「其中一处 `?` 由 [`LobSource`] 提供」的语句。
    ///
    /// 走的正是探针验证过的路径：`DSQL_DATA_AT_EXEC` 占位 → `dpi_exec` 报
    /// `DSQL_NEED_DATA` → 逐个 `dpi_param_data`/`dpi_put_data` → 执行完毕。
    /// **每个参数只 `put_data` 一次**（分多次交给驱动会把后续字节塞进下一个参数，
    /// 实测如此），所以源会先被读成完整一段。
    fn execute_lob(
        &mut self,
        stmt: &mut Self::Stmt,
        sql: &str,
        args: &[DmValue],
        lob: &mut LobParam<&mut dyn LobSource>,
    ) -> Result<ExecOutcome, DmError> {
        if !stmt.prepared {
            stmt.inner
                .prepare(sql)
                .map_err(|e| to_dm(e, Stage::Prepare))?;
            stmt.prepared = true;
        }
        stmt.inner.close_cursor();
        stmt.columns = None;
        self.arena.clear();

        // 先把普通参数按顺序绑好，LOB 那一处用延迟输入占位。
        let mut encoded = Vec::with_capacity(args.len() + 1);
        let mut arg_index = 0;
        for column in 0..=args.len() {
            if column == lob.column {
                encoded.push(None);
            } else {
                let Some(value) = args.get(arg_index) else {
                    return Err(DmError::at(
                        DmErrorKind::Config(format!(
                            "execute_lob：语句有 {} 个参数，但 args 只有 {} 个",
                            args.len() + 1,
                            args.len()
                        )),
                        Stage::Bind,
                    ));
                };
                arg_index += 1;
                encoded.push(Some(self.arena.encode(value)?));
            }
        }

        // 延迟输入占位的指示符与缓冲区：长度用 `size_hint`，未知则报 0 让驱动来问。
        let hint = lob.source.size_hint().unwrap_or(0);
        let (lob_ctype, lob_dtype) = match lob.kind {
            dm_core::backend::LobKind::Binary => (ctype::BINARY, sqltype::BLOB),
            dm_core::backend::LobKind::Text => (ctype::CHAR, sqltype::CLOB),
        };
        let slot = self.arena.reserve(1);
        let lob_ind = self.arena.push_indicator(indicator::DATA_AT_EXEC);
        let lob_act = self
            .arena
            .push_indicator(SLength::try_from(hint).unwrap_or(0));

        for (index, encoded) in encoded.into_iter().enumerate() {
            let index = u16::try_from(index + 1).map_err(|_| {
                DmError::at(
                    DmErrorKind::Config("参数个数超出 u16".to_string()),
                    Stage::Bind,
                )
            })?;
            match encoded {
                Some(param) => self.arena.bind(&stmt.inner, index, param)?,
                None => {
                    let (buf, ind, act) = self.arena.triple(slot, lob_ind, lob_act);
                    // SAFETY: 指针来自本结构体自有的缓冲区，绑定期与执行期都有效；
                    // 指示符是 DATA_AT_EXEC，驱动此时不会去读 buf 的内容。
                    unsafe {
                        stmt.inner
                            .bind_param2(
                                index,
                                param::INPUT,
                                lob_ctype,
                                lob_dtype,
                                0,
                                0,
                                buf,
                                0,
                                ind,
                                act,
                            )
                            .map_err(|e| to_dm(e, Stage::Bind))?;
                    }
                }
            }
        }

        let deferred = stmt
            .inner
            .exec_deferred()
            .map_err(|e| to_dm(e, Stage::Execute))?;
        if deferred {
            // 先把源读成完整一段：DPI 侧一次 put_data 交完整参数才是对的。
            let mut bytes: Vec<u8> = Vec::with_capacity(hint);
            while lob
                .source
                .next_chunk(&mut bytes)
                .map_err(|e| DmError::at(DmErrorKind::Config(e.to_string()), Stage::Execute))?
            {
            }
            while stmt
                .inner
                .param_data()
                .map_err(|e| to_dm(e, Stage::Execute))?
                .is_some()
            {
                stmt.inner
                    .put_data(&bytes)
                    .map_err(|e| to_dm(e, Stage::Execute))?;
            }
        }
        stmt.executed = true;
        self.finish(stmt)
    }

    fn begin(&mut self) -> Result<(), DmError> {
        // DPI 没有显式 BEGIN：关掉自动提交即进入事务（与 ODBC 的 `SQL_AUTOCOMMIT_OFF` 同义）。
        self.conn
            .set_attr_i32(attr::AUTOCOMMIT, autocommit::OFF)
            .map_err(|e| to_dm(e, Stage::Execute))
    }

    fn commit(&mut self) -> Result<(), DmError> {
        self.conn.commit().map_err(|e| to_dm(e, Stage::Execute))?;
        self.conn
            .set_attr_i32(attr::AUTOCOMMIT, autocommit::ON)
            .map_err(|e| to_dm(e, Stage::Reset))
    }

    fn rollback(&mut self) -> Result<(), DmError> {
        self.conn.rollback().map_err(|e| to_dm(e, Stage::Execute))?;
        self.conn
            .set_attr_i32(attr::AUTOCOMMIT, autocommit::ON)
            .map_err(|e| to_dm(e, Stage::Reset))
    }

    fn reset(&mut self) -> Result<(), DmError> {
        self.conn.rollback().map_err(|e| to_dm(e, Stage::Reset))?;
        self.conn
            .set_attr_i32(attr::AUTOCOMMIT, autocommit::ON)
            .map_err(|e| to_dm(e, Stage::Reset))
    }

    fn close(self) {
        let Self {
            dpi: _dpi,
            env: _env,
            conn,
            arena: _arena,
        } = self;
        if let Err(err) = conn.logout() {
            tracing::debug!(%err, "dpi_logout 失败（连接可能已断）");
        }
    }
}

/// 一条 DPI 语句 + 它的缓存状态。
pub struct DpiStmt {
    /// 底层语句句柄。
    inner: Stmt,
    /// 是否已 `prepare`（`Backend::execute` 允许「未 prepare 直接执行」）。
    prepared: bool,
    /// 本语句是否产出过结果集（产出过的不能进语句缓存）。
    produced_result_set: bool,
    /// 是否已经执行过至少一次。
    ///
    /// 真机实测：对**刚 `prepare` 还没执行**的语句调 `dpi_close_cursor` 会让后面的
    /// `dpi_exec` 静默不生效（DDL 不建表、DML 不改行、查询零行），所以只有执行过的
    /// 语句才需要「执行前先关游标」。
    executed: bool,
    /// 列元数据缓存（`Backend::columns` 的签名只给 `&Self::Stmt`，问不了驱动）。
    columns: Option<Arc<[ColumnMeta]>>,
    /// 分块 LOB 读的当前列（0 起算）。
    lob_column: Option<usize>,
}

impl std::fmt::Debug for DpiStmt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DpiStmt")
            .field("prepared", &self.prepared)
            .field("produced_result_set", &self.produced_result_set)
            .field("columns", &self.columns.as_ref().map(|c| c.len()))
            .field("lob_column", &self.lob_column)
            .finish()
    }
}

/// 手写 `Debug`：`Dpi` / `Env` / `Conn` 都不实现 `Debug`（里面是裸句柄），只印后端的
/// 身份与状态摘要——`DmPool` 之类的容器 `derive(Debug)` 时需要它。
impl std::fmt::Debug for DpiBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DpiBackend")
            .field("backend", &"dm-dpi")
            .finish_non_exhaustive()
    }
}

// SAFETY: 语句句柄只在 `Mutex` 保护下使用（`Backend` 的契约）；`Stmt` 自己已经是 `Send`。
unsafe impl Send for DpiStmt {}

// ---------------------------------------------------------------------------
// 参数缓冲
// ---------------------------------------------------------------------------

/// 一个待绑定参数在缓冲区里的位置与 C/服务端类型。
#[derive(Debug, Clone, Copy)]
struct Encoded {
    /// 值在 [`ParamArena::buf`] 里的起始偏移。
    offset: usize,
    /// 字节数（NULL 时为 0）。
    len: usize,
    /// DPI 的 C 类型。
    ctype: i16,
    /// DPI 的服务端类型。
    dtype: i16,
    /// 精度：来自 `dpi_desc_param`，编码时先用 0。
    precision: u64,
    /// 小数位：来自 `dpi_desc_param`，编码时先用 0。
    scale: i16,
    /// 是否 SQL NULL。
    is_null: bool,
}

/// 参数缓冲区：一次 `bind_params` 的全部字节与指示符。
///
/// DPI 只记住指针，值的字节必须活到 `execute` 之后，所以由后端自己持有而不是借用
/// 调用方的 `DmValue`（那批值在 `execute` 返回后就没人保证了）。
#[derive(Debug, Default)]
struct ParamArena {
    /// 所有参数的字节，紧密排列。
    buf: Vec<u8>,
    /// 指示符（每参数一个）。
    inds: Vec<SLength>,
    /// 实际长度（每参数一个，`dpi_bind_param2` 要求）。
    acts: Vec<SLength>,
}

impl ParamArena {
    fn clear(&mut self) {
        self.buf.clear();
        self.inds.clear();
        self.acts.clear();
    }

    /// 追加一段字节，返回偏移。
    fn push(&mut self, bytes: &[u8]) -> usize {
        let offset = self.buf.len();
        self.buf.extend_from_slice(bytes);
        // 至少留 1 字节：驱动不接受空缓冲指针（即使指示符是 NULL）。
        if bytes.is_empty() {
            self.buf.push(0);
        }
        offset
    }

    /// 预留一个 1 字节的占位槽，返回偏移。
    fn reserve(&mut self, bytes: usize) -> usize {
        let offset = self.buf.len();
        self.buf.resize(offset + bytes.max(1), 0);
        offset
    }

    fn push_indicator(&mut self, value: SLength) -> usize {
        self.inds.push(value);
        self.acts.push(0);
        self.inds.len() - 1
    }

    /// 取（缓冲区指针, 指示符指针, 实际长度指针）。
    fn triple(
        &mut self,
        offset: usize,
        ind: usize,
        act: usize,
    ) -> (*mut std::ffi::c_void, *mut SLength, *mut SLength) {
        let buf = self.buf[offset..].as_mut_ptr().cast();
        let ind = &raw mut self.inds[ind];
        let act = &raw mut self.acts[act];
        (buf, ind, act)
    }

    /// 把第 `index` 个参数绑上。
    fn bind(&mut self, stmt: &Stmt, index: u16, param: Encoded) -> Result<(), DmError> {
        let ind = self.push_indicator(if param.is_null {
            indicator::NULL_DATA
        } else {
            SLength::try_from(param.len).unwrap_or(0)
        });
        let act = self.push_indicator(SLength::try_from(param.len).unwrap_or(0));
        let (buf, ind_ptr, act_ptr) = self.triple(param.offset, ind, act);
        // SAFETY: 缓冲区与指示符都在本结构体里，生命周期覆盖到 execute 之后；
        // 偏移来自 `encode`，长度与内容匹配。
        unsafe {
            stmt.bind_param2(
                index,
                param::INPUT,
                param.ctype,
                param.dtype,
                param.precision,
                param.scale,
                buf,
                SLength::try_from(param.len).unwrap_or(0),
                ind_ptr,
                act_ptr,
            )
            .map_err(|e| to_dm(e, Stage::Bind))
        }
    }

    /// 把 `value` 编码成字节并登记类型。
    fn encode(&mut self, value: &DmValue) -> Result<Encoded, DmError> {
        let (ctype, dtype, bytes, is_null) = encode_bytes(value)?;
        let offset = self.push(&bytes);
        Ok(Encoded {
            offset,
            len: bytes.len(),
            ctype,
            dtype,
            precision: 0,
            scale: 0,
            is_null,
        })
    }
}

/// 把一个值编码成「C 类型 / SQL 类型 / 字节 / 是否 NULL」。
///
/// 单行绑定（[`ParamArena`]）与数组绑定（[`ColumnBuffer`]）共用这一份映射，免得两条
/// 路的类型口径各写一遍再悄悄漂移。
fn encode_bytes(value: &DmValue) -> Result<(i16, i16, Vec<u8>, bool), DmError> {
    let out = match value {
        DmValue::Null => (ctype::CHAR, sqltype::VARCHAR, Vec::new()),
        DmValue::Bool(v) => (ctype::STINYINT, sqltype::BIT, vec![u8::from(*v)]),
        DmValue::TinyInt(v) => (ctype::STINYINT, sqltype::TINYINT, v.to_le_bytes().to_vec()),
        DmValue::SmallInt(v) => (ctype::SSHORT, sqltype::SMALLINT, v.to_le_bytes().to_vec()),
        DmValue::Int(v) => (ctype::SLONG, sqltype::INT, v.to_le_bytes().to_vec()),
        DmValue::BigInt(v) => (ctype::SBIGINT, sqltype::BIGINT, v.to_le_bytes().to_vec()),
        DmValue::Real(v) => (ctype::FLOAT, sqltype::FLOAT, v.to_le_bytes().to_vec()),
        DmValue::Double(v) => (ctype::DOUBLE, sqltype::DOUBLE, v.to_le_bytes().to_vec()),
        // 十进制按文本交付：DPI 的 `dpi_numeric_t` 是打包 BCD，服务端对
        // 「字符型参数 + DEC 目标类型」本来就做隐式转换，没必要自己打包。
        DmValue::Decimal(v) => (ctype::CHAR, sqltype::DEC, v.to_string().into_bytes()),
        DmValue::Text(v) => (ctype::CHAR, sqltype::VARCHAR, v.as_bytes().to_vec()),
        DmValue::Bytes(v) => (ctype::BINARY, sqltype::VARBINARY, v.clone()),
        DmValue::Date(v) => (
            ctype::DATE,
            sqltype::DATE,
            dpi_date(v.year(), v.month(), v.day()),
        ),
        // `dpi_time_t` 没有小数秒字段（`DPI.h`），走 C 类型会把 `TIME(6)` 的微秒
        // 静默抹掉（实测写成 `.000000`）；按文本绑给 `TIME` 列能原样保留精度。
        DmValue::Time(v) => (
            ctype::CHAR,
            sqltype::TIME,
            format!("{}", v.format("%H:%M:%S%.6f")).into_bytes(),
        ),
        DmValue::Timestamp(v) => (
            ctype::TIMESTAMP,
            sqltype::TIMESTAMP,
            dpi_timestamp(
                v.date().year(),
                v.date().month(),
                v.date().day(),
                v.hour(),
                v.minute(),
                v.second(),
                v.nanosecond(),
            ),
        ),
        DmValue::Interval(_) => {
            return Err(DmError::at(
                DmErrorKind::Unsupported {
                    feature: "dm-dpi 的参数绑定暂不支持 INTERVAL".to_string(),
                },
                Stage::Bind,
            ));
        }
        DmValue::Unknown(bytes) => (ctype::BINARY, sqltype::VARBINARY, bytes.clone()),
        _ => {
            return Err(DmError::at(
                DmErrorKind::Unsupported {
                    feature: "dm-dpi 的参数绑定暂不支持该值类型".to_string(),
                },
                Stage::Bind,
            ));
        }
    };
    let is_null = matches!(value, DmValue::Null);
    Ok((out.0, out.1, out.2, is_null))
}

/// 一个参数的**定长数组**：数组绑定要求每行在缓冲里占同样长的槽位。
struct ColumnBuffer {
    ctype: i16,
    dtype: i16,
    /// 单个元素的字节数（喂给 `dpi_bind_param2` 的 `buf_len`）。
    stride: usize,
    data: Vec<u8>,
    inds: Vec<SLength>,
    acts: Vec<SLength>,
}

impl ColumnBuffer {
    fn build(column: &[DmValue], rows: usize) -> Result<Self, DmError> {
        let mut encoded = Vec::with_capacity(rows);
        for value in column.iter().take(rows) {
            encoded.push(encode_bytes(value)?);
        }
        // 类型取第一个**非 NULL** 值；全是 NULL 就按文本给（`NULL_DATA` 下驱动不看类型）。
        let (ctype, dtype) = encoded
            .iter()
            .find(|(_, _, _, is_null)| !*is_null)
            .map(|(ctype, dtype, _, _)| (*ctype, *dtype))
            .unwrap_or((ctype::CHAR, sqltype::VARCHAR));
        let stride = encoded
            .iter()
            .map(|(_, _, bytes, _)| bytes.len())
            .max()
            .unwrap_or(0)
            .max(1);
        let mut data = vec![0u8; stride * rows];
        let mut inds = vec![0 as SLength; rows];
        let mut acts = vec![0 as SLength; rows];
        for (index, (_, _, bytes, is_null)) in encoded.into_iter().enumerate() {
            let offset = index * stride;
            data[offset..offset + bytes.len()].copy_from_slice(&bytes);
            let len = SLength::try_from(bytes.len()).unwrap_or(0);
            acts[index] = len;
            inds[index] = if is_null { indicator::NULL_DATA } else { len };
        }
        Ok(Self {
            ctype,
            dtype,
            stride,
            data,
            inds,
            acts,
        })
    }
}

// ---------------------------------------------------------------------------
// 值编解码
// ---------------------------------------------------------------------------

/// `dpi_date_t`（6 字节）。
#[repr(C)]
struct DpiDate {
    year: i16,
    month: u16,
    day: u16,
}

fn as_bytes<T>(value: &T) -> Vec<u8> {
    // SAFETY: `T` 都是 `#[repr(C)]` 的 POD 结构，按字节视图读不会越界。
    unsafe {
        std::slice::from_raw_parts(std::ptr::from_ref(value).cast::<u8>(), size_of::<T>()).to_vec()
    }
}

fn dpi_date(year: i32, month: u32, day: u32) -> Vec<u8> {
    as_bytes(&DpiDate {
        year: i16::try_from(year).unwrap_or(0),
        month: u16::try_from(month).unwrap_or(0),
        day: u16::try_from(day).unwrap_or(0),
    })
}

#[allow(clippy::too_many_arguments)]
fn dpi_timestamp(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    nanosecond: u32,
) -> Vec<u8> {
    as_bytes(&Timestamp {
        year: i16::try_from(year).unwrap_or(0),
        month: u16::try_from(month).unwrap_or(0),
        day: u16::try_from(day).unwrap_or(0),
        hour: u16::try_from(hour).unwrap_or(0),
        minute: u16::try_from(minute).unwrap_or(0),
        second: u16::try_from(second).unwrap_or(0),
        // 实测：`fraction` 是**纳秒**（`.123456` → `123456000`）。
        fraction: nanosecond,
    })
}

/// 把一列读成 [`DmValue`]。
fn read_value(stmt: &Stmt, index: u16, meta: &ColumnMeta) -> Result<DmValue, DmError> {
    use DataType as T;
    let column = Some(meta.name.clone());
    let decode = |err: DpiError| decode_error(column.clone(), err);
    match meta.data_type {
        T::Bit => Ok(bool_value(stmt.column_i64(index).map_err(decode)?)),
        T::TinyInt => Ok(opt(stmt.column_i64(index).map_err(decode)?, |v| {
            DmValue::TinyInt(v as i8)
        })),
        T::SmallInt => Ok(opt(stmt.column_i64(index).map_err(decode)?, |v| {
            DmValue::SmallInt(v as i16)
        })),
        T::Integer => Ok(opt(stmt.column_i64(index).map_err(decode)?, |v| {
            DmValue::Int(v as i32)
        })),
        T::BigInt => Ok(opt(
            stmt.column_i64(index).map_err(decode)?,
            DmValue::BigInt,
        )),
        T::Real => Ok(opt(stmt.column_f64(index).map_err(decode)?, |v| {
            DmValue::Real(v as f32)
        })),
        T::Float | T::Double => Ok(opt(
            stmt.column_f64(index).map_err(decode)?,
            DmValue::Double,
        )),
        T::Decimal => {
            let text = read_text(stmt, index, meta)?;
            match text {
                None => Ok(DmValue::Null),
                // 与 `dm-odbc` 的 `decimal_text_value` 同口径：`from_str_exact` 装得下
                // 就精确还原，装不下（`DECIMAL(38,20)` 这种 96 位尾数放不下的值）就
                // **按原文文本交回**。用 `FromStr` 会四舍五入到 28 位还把结果当成
                // 精确值——官方套件的 `ty_bigdecimal_as_numeric_fixed_precision` 就是
                // 这么读到 `scale=11` 的。
                Some(text) => {
                    let trimmed = text.trim();
                    match rust_decimal::Decimal::from_str_exact(trimmed) {
                        Ok(value) => Ok(DmValue::Decimal(value)),
                        Err(err) => {
                            tracing::debug!(
                                text = trimmed,
                                %err,
                                "DECIMAL 的文本形态超出 rust_decimal，原样按文本返回"
                            );
                            Ok(DmValue::Text(trimmed.to_owned()))
                        }
                    }
                }
            }
        }
        T::Date => match stmt.column_length(index, ctype::DATE).map_err(decode)? {
            None => Ok(DmValue::Null),
            Some(_) => {
                let mut buf = [0u8; size_of::<DpiDate>()];
                let n = stmt
                    .read_raw(index, ctype::DATE, &mut buf)
                    .map_err(decode)?
                    .unwrap_or(0);
                if n == 0 {
                    return Ok(DmValue::Null);
                }
                let year = i16::from_le_bytes([buf[0], buf[1]]);
                let month = u16::from_le_bytes([buf[2], buf[3]]);
                let day = u16::from_le_bytes([buf[4], buf[5]]);
                let date = chrono::NaiveDate::from_ymd_opt(
                    i32::from(year),
                    u32::from(month),
                    u32::from(day),
                )
                .ok_or_else(|| {
                    decode_error(
                        column.clone(),
                        format_args!("DATE 值不合法：{year}-{month}-{day}"),
                    )
                })?;
                Ok(DmValue::Date(date))
            }
        },
        // `dpi_time_t` 只有「时/分/秒」三个字段（`DPI.h` 没有小数秒），而达梦的
        // `TIME(6)` 精确到微秒。走 C 类型会静默丢小数位（实测 `03:04:05.123456` →
        // `03:04:05`），所以 TIME 列按文本读再自己解析，精度与库内一致。
        T::Time => match read_text(stmt, index, meta)? {
            None => Ok(DmValue::Null),
            Some(text) => parse_time_text(&text).map(DmValue::Time).ok_or_else(|| {
                decode_error(column.clone(), format_args!("TIME 文本无法解析：{text:?}"))
            }),
        },
        T::Timestamp => match stmt.column_timestamp(index).map_err(decode)? {
            None => Ok(DmValue::Null),
            Some(ts) => {
                let date = chrono::NaiveDate::from_ymd_opt(
                    i32::from(ts.year),
                    u32::from(ts.month),
                    u32::from(ts.day),
                );
                let time = chrono::NaiveTime::from_hms_nano_opt(
                    u32::from(ts.hour),
                    u32::from(ts.minute),
                    u32::from(ts.second),
                    ts.fraction,
                );
                match (date, time) {
                    (Some(date), Some(time)) => Ok(DmValue::Timestamp(date.and_time(time))),
                    _ => Err(decode_error(
                        column.clone(),
                        format_args!("TIMESTAMP 值不合法：{ts:?}"),
                    )),
                }
            }
        },
        T::Char | T::VarChar | T::WChar | T::WVarChar => {
            Ok(opt(read_text(stmt, index, meta)?, DmValue::Text))
        }
        T::LongVarChar | T::WLongVarChar => {
            let bytes = read_long(stmt, index, ctype::CHAR).map_err(decode)?;
            Ok(match bytes {
                None => DmValue::Null,
                Some(bytes) => DmValue::Text(String::from_utf8_lossy(&bytes).into_owned()),
            })
        }
        T::Binary | T::VarBinary => Ok(opt(
            stmt.column_binary(index).map_err(decode)?,
            DmValue::Bytes,
        )),
        T::LongVarBinary => {
            let bytes = read_long(stmt, index, ctype::BINARY).map_err(decode)?;
            Ok(match bytes {
                None => DmValue::Null,
                Some(bytes) => DmValue::Bytes(bytes),
            })
        }
        DataType::Unknown(_) => {
            let bytes = read_long(stmt, index, ctype::BINARY).map_err(decode)?;
            Ok(match bytes {
                None => DmValue::Null,
                Some(bytes) => DmValue::Unknown(bytes),
            })
        }
        _ => Err(DmError::at(
            DmErrorKind::Unsupported {
                feature: format!("dm-dpi 暂不支持解码 {:?}", meta.data_type),
            },
            Stage::Fetch,
        )),
    }
}

fn opt<T>(value: Option<T>, f: impl FnOnce(T) -> DmValue) -> DmValue {
    match value {
        None => DmValue::Null,
        Some(v) => f(v),
    }
}

/// 造一个「这一列解不出来」的错误。
///
/// `DmErrorKind::Decode` 只带列名（错误变体的集合是冻结的），细节写在 `tracing` 里：
/// 排查时需要知道是哪一边对不上（驱动返回值 vs 我们的期望）。
fn decode_error(column: Option<String>, detail: impl std::fmt::Display) -> DmError {
    tracing::debug!(column = ?column, %detail, "DPI 取数解码失败");
    DmError::at(DmErrorKind::Decode { column }, Stage::Fetch)
}

fn bool_value(value: Option<i64>) -> DmValue {
    match value {
        None => DmValue::Null,
        Some(0) => DmValue::Bool(false),
        Some(_) => DmValue::Bool(true),
    }
}

/// 读一个短文本列（`CHAR`/`VARCHAR`）。
fn read_text(stmt: &Stmt, index: u16, meta: &ColumnMeta) -> Result<Option<String>, DmError> {
    let _ = meta;
    // 先问长度：短列也走同一条路，免得「按声明长度开缓冲」在 UTF-8 与字符数之间算错。
    match read_long(stmt, index, ctype::CHAR).map_err(|e| to_dm(e, Stage::Fetch))? {
        None => Ok(None),
        Some(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
    }
}

/// 读任意长度的列内容。
///
/// 实测口径：
/// - `dpi_get_data(…, NULL, 0)` 只问长度、**不消费**列数据；
/// - 长度已知时一次给足缓冲就能一次读完（`> 64 KiB` 也一样）；
/// - 但分块模式（每次 64 KiB）对文本列会在边界多塞 NUL，所以**只在必要时**分块，
///   并且丢掉每块尾部的 NUL——文本列里真正的 NUL 本来就不该出现。
fn read_long(stmt: &Stmt, index: u16, value_ctype: i16) -> Result<Option<Vec<u8>>, DpiError> {
    let Some(raw_len) = stmt.column_length(index, value_ctype)? else {
        return Ok(None);
    };
    let hint = usize::try_from(raw_len.max(0)).unwrap_or(0);
    // 文本列的「长度」是**库内字符集**（GB18030）的字节数，转成客户端字符集（UTF-8）
    // 之后会变长（最多 4 字节/字符），所以按 4 倍开缓冲：一次 `dpi_get_data` 给足缓冲
    // 就能一次读完，避开分块模式在块尾塞 NUL 的毛病。
    let cap = if value_ctype == ctype::CHAR {
        hint.saturating_mul(4)
    } else {
        hint
    };
    let cap = cap.clamp(1024, MAX_LONG_BYTES);
    let mut buf = vec![0u8; cap];
    let got = stmt
        .read_raw(index, value_ctype, &mut buf)?
        .unwrap_or(0)
        .min(buf.len());
    if got < cap {
        buf.truncate(got);
        return Ok(Some(buf));
    }
    // 缓冲被填满：值可能更长，继续按块取。文本列每块尾部的 NUL 是驱动补的终结符，
    // 只在「整块被填满」时丢掉最后一个 0（真值里的 NUL 本来也表达不了）。
    buf.truncate(got);
    loop {
        let mut chunk = vec![0u8; MAX_GET_DATA];
        match stmt.read_chunk(index, value_ctype, &mut chunk)? {
            crate::handle::ReadChunk::Data(n) if n > 0 => {
                let mut n = n.min(chunk.len());
                if value_ctype == ctype::CHAR && n == chunk.len() && chunk[n - 1] == 0 {
                    n -= 1;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            _ => break,
        }
    }
    Ok(Some(buf))
}

/// 解析 `HH:MM:SS[.ffffff]`（达梦 TIME 的文本形态）。
fn parse_time_text(text: &str) -> Option<chrono::NaiveTime> {
    let text = text.trim();
    let mut parts = text.split(':');
    let hour: u32 = parts.next()?.trim().parse().ok()?;
    let minute: u32 = parts.next()?.trim().parse().ok()?;
    let rest = parts.next()?.trim();
    let (second, nanos) = match rest.split_once('.') {
        None => (rest.parse::<u32>().ok()?, 0u32),
        Some((sec, frac)) => {
            let sec: u32 = sec.parse().ok()?;
            let frac: String = frac.chars().take(9).collect();
            let digits = frac.len() as u32;
            let value: u32 = frac.parse().ok()?;
            let nanos = value * 10u32.pow(9 - digits);
            (sec, nanos)
        }
    };
    chrono::NaiveTime::from_hms_nano_opt(hour, minute, second, nanos)
}

/// 从 `ColumnDesc` 造 [`ColumnMeta`]。
fn column_meta(desc: &crate::handle::ColumnDesc) -> ColumnMeta {
    use dm_core::datatype::TypeCategory;
    let data_type = data_type_of(desc.sql_type);
    let mut meta = ColumnMeta::new(desc.name.clone(), data_type).with_nullable(desc.nullable);
    match meta.data_type.category() {
        TypeCategory::Text | TypeCategory::Binary => {
            let size = usize::try_from(desc.size).unwrap_or(0);
            if size > 0 && size < usize::MAX / 2 {
                meta = meta.with_column_size(size);
            }
            // 达梦的 `VARCHAR(n)` 默认按字节计量（`LENGTH_IN_CHAR=0`），这里按字节报。
            meta = meta.with_length_in_char(false);
        }
        TypeCategory::Decimal => {
            let precision = usize::try_from(desc.size).unwrap_or(0).clamp(1, 38);
            let scale = usize::try_from(desc.decimals.max(0))
                .unwrap_or(0)
                .min(precision);
            meta = meta.with_precision_scale(precision, scale);
        }
        TypeCategory::Float | TypeCategory::Integer | TypeCategory::Boolean => {}
        _ => {}
    }
    meta
}

/// DPI 服务端类型码 → [`DataType`]。
fn data_type_of(sql_type: i16) -> DataType {
    match sql_type {
        sqltype::CHAR => DataType::Char,
        sqltype::VARCHAR => DataType::VarChar,
        sqltype::BIT | sqltype::BOOLEAN => DataType::Bit,
        sqltype::TINYINT => DataType::TinyInt,
        sqltype::SMALLINT => DataType::SmallInt,
        sqltype::INT => DataType::Integer,
        sqltype::BIGINT => DataType::BigInt,
        sqltype::DEC => DataType::Decimal,
        sqltype::FLOAT => DataType::Float,
        sqltype::DOUBLE => DataType::Double,
        sqltype::DATE => DataType::Date,
        sqltype::TIME => DataType::Time,
        sqltype::TIMESTAMP => DataType::Timestamp,
        sqltype::BINARY => DataType::Binary,
        sqltype::VARBINARY => DataType::VarBinary,
        sqltype::BLOB => DataType::LongVarBinary,
        sqltype::CLOB => DataType::LongVarChar,
        other => DataType::Unknown(other),
    }
}

/// 把 DPI 错误映射成 [`DmError`]。
fn to_dm(err: DpiError, stage: Stage) -> DmError {
    // DPI 不带 SQLSTATE（要另外问诊断字段），统一按 `HY000` 填；分类走
    // `DmErrorKind::classify_server`，它**以原生码为准**（设计方案 §6.8），所以 `-2007`
    // 仍是 `Syntax`、`-2106`/`-2111` 仍是 `ObjectNotFound`——与 `dm-odbc` 的分类结果对齐。
    // 上层依赖这一点（例如 `Transaction::release_savepoint` 只把 `Syntax` 翻成
    // `Unsupported`），不能一律报 `Server`。
    let kind = DmErrorKind::classify_server(*b"HY000", err.code, &err.message);
    let mut out = DmError::at(kind, stage);
    // 诊断记录必须带上：调用方要能看见服务端原话（`-2007` 之类），也正是
    // `RELEASE SAVEPOINT` 用例断言「原始诊断保留」的依据。
    out.push_diag(DiagRecord::new(*b"HY000", err.code, err.message));
    out
}
