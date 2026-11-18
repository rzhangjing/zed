//! 参数绑定（T3.4，设计方案 §6.3）。
//!
//! 这一层只做一件事：把 [`DmValue`] 变成「驱动能读的字节 + 指标」，再逐个
//! `SQLBindParameter` 到语句句柄上。取值在 T3.5（`result.rs`）。
//!
//! # 为什么绕过 `odbc-api` 直接调裸 FFI
//!
//! `odbc-api` 给字符串/二进制参数传的指标指针是 **NULL**（`Vec<T>: CData` 的
//! `indicator_ptr()` 返回空指针），意思是「长度等于 `buffer_length`，且不是 NULL」。
//! 那正好把 §6.3 纪律①要区分的东西糊掉了：
//!
//! * **空串与 NULL 分不开**。NULL 要靠 `indicator = SQL_NULL_DATA(-1)` 表达，
//!   空串靠 `indicator = 0` 表达；指标指针为 NULL 时两者都只能靠「有没有数据」猜。
//! * **长度不精确**。文本参数的长度必须是**字节数**（UTF-16 下的 `字符数 × 2`），
//!   而 `buffer_length` 只是缓冲容量；缓冲按容量报，驱动的截断判断就会错位。
//!
//! 所以本模块自己调 `SQLBindParameter`（`odbc-api` 也是这么调的，见其
//! `handles/statement.rs` 的 `bind_parameter`）。裸调用换来的代价是诊断要自己收——
//! 交给了 [`crate::diag::sql_ok`]，判断规矩仍然只有一处。
//!
//! # D8 五条纪律的落点（§6.3）
//!
//! 1. 字符/二进制参数**一定**传指标指针，且值是**字节长度** → [`bind_params`] 里
//!    `base.add(meta.ind_off)`，绝不为这两类传空指针。
//! 2. [`DmValue::Null`] → 指标 = `SQL_NULL_DATA(-1)`，且**不写值缓冲**（只占位，见
//!    [`encode_one`]）。
//! 3. 字节长度与缓冲长度**来自同一变量**（`meta.buf_len`），不各自算一遍。
//! 4. 编码完就 `debug_assert!(ind == buf_len)`——同一份数据量两遍，不一致立刻炸。
//! 5. **稳态零分配**：[`ParamArena`] 的 `arena` 缓冲跨次数复用；`bind_params` 只
//!    `clear()`（保容量）之后写，不在循环里 `Vec::new`。
//!
//! # D16：不依赖 `describe_param`
//!
//! §4.4 实测达梦的 `SQLDescribeParam` 不可用（返回 `8188`），因此**不**靠它决定
//! 每个 `?` 该用什么类型——类型完全由**调用方给的 `DmValue`** 决定。数值超长时不
//! 静默截断，交给服务端报 `22001/-70005`，那条码在 `dm-error` 里已分类为
//! [`DmErrorKind::Encode`](dm_error::DmErrorKind::Encode)。
//!
//! # 一次 `bind_params` 的三趟
//!
//! 1. **问类型序列**（[`type_of`]，纯函数）：与上一趟的类型序列比对，只在「个数或类型
//!    变了」时 `SQLFreeStmt(SQL_RESET_PARAMS)`（§6.3）。复位之后所有参数都会重绑，
//!    所以不会留下指向旧缓冲的悬挂绑定。
//! 2. **一遍写入**：按 `[8 字节对齐的值缓冲][8 字节指标]` 铺进 `arena`，同时产出
//!    [`ParamMeta`]。写入前先 `reserve` 一个上界（[`upper_bound`]），保证这一趟
//!    不会重分配——重分配会让已经算好的偏移与即将交给驱动的指针分家。
//! 3. **逐个绑定**：拿 `arena` 的基址加偏移，交给 `SQLBindParameter`。
//!
//! 绑定之后**不得**再让 `arena` 重分配或 `clear` 而不重绑：驱动存的是裸指针。
//! `ParamArena` 用 `Vec<u8>` 装数据，所以把 `ParamArena` 结构体本身搬来搬去是安全的
//! （堆上的那块不会跟着动），会失效的只有「`clear()` 之后不重绑就直接执行」。

use std::fmt;
use std::mem::{align_of, size_of};

use dm_core::{DmValue, LobKind, LobParam, LobSource};
use dm_error::{DmError, DmErrorKind, Stage};
use odbc_api::handles::{SqlResult, Statement};
use odbc_sys::{
    CDataType, HStmt, IS_POINTER, IS_UINTEGER, Len, NULL_DATA, ParamType, Pointer,
    SQLBindParameter, SQLSetStmtAttrW, SqlDataType, SqlReturn, StatementAttribute, ULen,
};

use crate::diag;
use crate::lob::LobFeeder;
use crate::stmt::OdbcStatement;

/// ODBC `SQL_MAX_NUMERIC_LEN`：`SQL_NUMERIC_STRUCT.value` 的字节数。
const NUMERIC_LEN: usize = 16;

/// 每个参数的固定开销上界：`[值缓冲前的对齐填充(≤7)] + [值缓冲后的对齐填充(≤7)] + [指标(8)]`
/// = 22，再留 2 字节余量取 24。
const PER_PARAM_SLACK: usize = 24;

/// 定长类型（整数/浮点/日期时间/DECIMAL 结构体）在预留时的单参数上界。
///
/// 最大的一个是 `SQL_NUMERIC_STRUCT` 的 19 字节，`Timestamp` 是 16；取 24 把两者
/// 都盖住并留余量。**这个数字比实际大的代价只是多留几字节容量**（一次性的），
/// 比实际小的代价是写入那趟真的重分配、偏移与交给驱动的指针分家——所以宁可大。
const FIXED_PAYLOAD_ESTIMATE: usize = 24;

// ---------------------------------------------------------------------------
// ODBC 结构体（按字节铺进 arena）
// ---------------------------------------------------------------------------

/// ODBC `SQL_NUMERIC_STRUCT`。
///
/// 字段顺序与 `sqlext.h` 一致：`precision`(1) + `scale`(1) + `sign`(1) +
/// `val[16]`，全部 1 字节对齐，所以**没有填充字节**，`size_of` 就是 19。
///
/// `val` 是**绝对值**的小端字节串（最低位在前，高位补 0）；正负由 `sign` 单独表达
/// （1 = 正，0 = 负）。这正是 ODBC 与「带符号整数尾数」的区别所在，也是最容易写错的
/// 一步：把 `-1.5` 的 `-15` 直接按二进制补码塞进去，服务端会读成一个巨大的正数。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct NumericStruct {
    /// 有效十进制位数（总位数，不是小数位数）。
    pub(crate) precision: u8,
    /// 小数位数。
    pub(crate) scale: i8,
    /// 1 = 正，0 = 负。
    pub(crate) sign: u8,
    /// 绝对值的小端字节串。
    pub(crate) val: [u8; NUMERIC_LEN],
}

// ---------------------------------------------------------------------------
// 参数元数据与复用缓冲
// ---------------------------------------------------------------------------

/// 一个已绑定参数在 [`ParamArena`] 里的位置，以及交给 `SQLBindParameter` 的元数据。
///
/// 设计方案 §6.3 把这块描述成 `(ctype, dtype, column_size, buf, ind)` 五元组；这里
/// 把 `buf`/`ind` 拆成「偏移 + 长度」（`buf_off`/`buf_len`/`ind_off`）正是为了**不存裸指针**：
/// 存指针的话，`ParamArena` 一被搬动（或者以后换成 `SmallVec`）就全悬了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamMeta {
    /// `SQLBindParameter` 的 `ValueType`：缓冲区里装的是哪种 C 类型。
    pub ctype: CDataType,
    /// `SQLBindParameter` 的 `ParameterType`：这个 `?` 在 SQL 里是什么类型。
    pub dtype: SqlDataType,
    /// 精度/长度。文本是**字符数**，二进制是字节数，DECIMAL 是有效位数；定长类型为 `0`。
    pub column_size: usize,
    /// 小数位数（只有 DECIMAL 非 0）。
    pub decimal_digits: i16,
    /// 值缓冲在 arena 里的偏移（8 字节对齐）。
    pub buf_off: usize,
    /// 值缓冲的字节长度，同时也是指标值（见 [`ParamMeta::ind`]）。
    pub buf_len: usize,
    /// 指标值：`SQL_NULL_DATA(-1)` 表示 NULL，否则等于 `buf_len`（D8 纪律②③）。
    pub ind: isize,
    /// 指标在 arena 里的偏移（8 字节对齐）。
    ///
    /// `SQLBindParameter` 要的是指标的**地址**而不是值，所以位置也得存。
    pub ind_off: usize,
}

impl ParamMeta {
    /// 这个参数绑的是 NULL。
    pub fn is_null(&self) -> bool {
        self.ind == NULL_DATA
    }
}

/// 跨语句复用的一块参数缓冲。
///
/// 用法：`let mut arena = ParamArena::new();` 然后每次执行前
/// [`bind_params`]`(&mut stmt, &args, &mut arena)`。第二趟起 `arena` 不再分配
/// （D8 纪律⑤）。
///
/// **不要**在绑定之后、执行之前调用 [`ParamArena::clear`]：驱动句柄上存的是指向
/// 本缓冲的裸指针，清掉之后它们就指向旧内容了（清空只减长度不缩容量，指针本身
/// 不悬，但内容已经不是当初绑定的那份）。
#[derive(Debug, Default)]
pub struct ParamArena {
    /// 上一次绑定的元数据。两个用途：给调用方看（`metas()`），以及判断下次要不要
    /// `SQL_RESET_PARAMS`（类型序列变了就得复位）。
    metas: Vec<ParamMeta>,
    /// 值缓冲与指标混排的字节池。
    arena: Vec<u8>,
}

impl ParamArena {
    /// 新建一块空缓冲。
    pub fn new() -> Self {
        Self::default()
    }

    /// 预分配：`params` 个参数、`bytes` 字节的值缓冲。
    ///
    /// 池化场景下值得一调——第一次绑定就把容量铺好，之后整条链路上没有分配。
    pub fn with_capacity(params: usize, bytes: usize) -> Self {
        Self {
            metas: Vec::with_capacity(params),
            arena: Vec::with_capacity(bytes),
        }
    }

    /// 清空上一次的绑定记录与缓冲内容（**保留容量**）。
    ///
    /// [`bind_params`] 自己会先调它，调用方通常不需要手动调。
    pub fn clear(&mut self) {
        self.metas.clear();
        self.arena.clear();
    }

    /// 上次绑定的参数个数。
    pub fn len(&self) -> usize {
        self.metas.len()
    }

    /// 上次没绑过任何参数。
    pub fn is_empty(&self) -> bool {
        self.metas.is_empty()
    }

    /// 上次绑定的参数元数据。
    pub fn metas(&self) -> &[ParamMeta] {
        &self.metas
    }

    /// 值缓冲区的容量（字节）。稳态下这个数字应当不再增长。
    pub fn capacity(&self) -> usize {
        self.arena.capacity()
    }

    /// 上次绑定实际占用的字节数。
    pub fn byte_len(&self) -> usize {
        self.arena.len()
    }
}

impl fmt::Display for ParamArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ParamArena({} 个参数, 占用 {} 字节 / 容量 {} 字节)",
            self.metas.len(),
            self.arena.len(),
            self.arena.capacity()
        )
    }
}

// ---------------------------------------------------------------------------
// T6.4：列式参数数组绑定（§4.7）
// ---------------------------------------------------------------------------

/// 一列（一个 `?`）在数组绑定里的布局。
///
/// 列式布局的关键量是 [`ParamArrayColumnPlan::stride`]：它同时是
/// `SQLBindParameter` 的 `BufferLength`（**数组元素的步长**，不是整块缓冲的长度）。
/// 驱动算第 `r` 个元素的地址就是 `base + r * stride`，所以步长必须 ≥ 任何元素的
/// 实际字节数——短元素补零、长元素决定步长。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamArrayColumnPlan {
    /// `SQLBindParameter` 的 `ValueType`。
    pub ctype: CDataType,
    /// `SQLBindParameter` 的 `ParameterType`。
    pub dtype: SqlDataType,
    /// `ColumnSize`：文本是**字符数**（取该列最大值），二进制是字节数。
    pub column_size: usize,
    /// `DecimalDigits`。
    pub decimal_digits: i16,
    /// 值数组在 `ParamArrayArena::arena` 里的起始偏移。
    pub buf_off: usize,
    /// 元素步长（= `BufferLength`），8 字节对齐。
    pub stride: usize,
    /// 指标数组的起始偏移（`[Len; N]`，8 字节对齐）。
    pub ind_off: usize,
}

/// 列式参数数组的复用缓冲（[`ParamArena`] 的数组版）。
///
/// # 为什么单独一块缓冲，而不是复用 [`ParamArena`]
///
/// [`ParamArena`] 的形状是「一个参数一组值」，数组绑定要的是「一个参数 `N` 个值」。
/// 两者的容量规划、对齐与释放时机都不同（数组这块在 `SQLExecute` 返回之后才允许动，
/// 而单行那块每次绑定都要重写），混在一起只会让「什么时候能改」变成需要记住的约定。
/// 分开之后各自 `clear()` 都不影响对方。
///
/// # 内存泄漏风险
///
/// 驱动在 `SQLBindParameter` 上存的是本缓冲的**裸指针**（不是副本），所以绑定之后、
/// 执行之前**不能**动这块缓冲。`bind_param_array` 一次调用里完成「铺缓冲 + 绑定」，
/// 调用方拿不到中间态，这条纪律就编进了 API 形状。
#[derive(Debug, Default)]
pub struct ParamArrayArena {
    /// 每列的布局（按参数序号）。
    plans: Vec<ParamArrayColumnPlan>,
    /// 列式值缓冲与指标数组混排的字节池。
    ///
    /// `Box<[u8]>` 而不是 `Vec<u8>`，且**慢速增长**（见 [`ParamArrayArena::ensure`]）。
    arena: Box<[u8]>,
    /// 本趟绑定的行数（= `SQL_ATTR_PARAMSET_SIZE`）。只有 `arena.len()` 是 2 的幂，
    /// 所以行数必须单独记——不能拿 `status.len()` 当行数用（那会多执行出一堆零值行）。
    rows: usize,
    /// `SQL_ATTR_PARAMS_PROCESSED_PTR` 指向它：执行后驱动把「实际处理了几个参数集」
    /// 写进来。`Box` 是为了让地址在对象的整个生命周期里稳定——驱动存的是它的地址。
    processed: Box<Len>,
    /// `SQL_ATTR_PARAM_STATUS_PTR` 指向它：每行的 `SQL_PARAM_*` 状态。
    status: Box<[u16]>,
    /// 逐元素编码时的临时缓冲（含编码时的对齐填充），跨调用复用。
    scratch: Vec<u8>,
}

impl ParamArrayArena {
    /// 新建一块空缓冲。
    pub fn new() -> Self {
        Self {
            // `processed` 初始化为 `SQL_PARAM_UNUSED`（3）：**不是** 0。
            // 万一驱动没写这个槽位（某些版本在纯成功路径上不更新），0 会被读成
            // 「一行都没处理」，而实际全都插进去了——那是最危险的一个方向。
            processed: Box::new(PARAM_UNUSED as Len),
            ..Self::default()
        }
    }

    /// 本趟绑定的参数（列）个数。
    pub fn column_count(&self) -> usize {
        self.plans.len()
    }

    /// 本趟绑定的行数（参数集个数）。
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// 本趟每一列的布局。
    pub fn plans(&self) -> &[ParamArrayColumnPlan] {
        &self.plans
    }

    /// 值缓冲区的长度（字节）。
    pub fn byte_len(&self) -> usize {
        self.arena.len()
    }

    /// 每行的 `SQL_PARAM_*` 状态（下标即参数集下标，有效范围 `0..rows`）。
    pub fn status(&self) -> &[u16] {
        &self.status[..self.rows.min(self.status.len())]
    }

    /// 驱动实际处理的参数集个数（`SQL_ATTR_PARAMS_PROCESSED_PTR`）。
    pub fn processed(&self) -> usize {
        (*self.processed).max(0) as usize
    }

    /// 状态数组里第一个 `SQL_PARAM_ERROR` 的下标；`None` = 没有任何一行被标为失败。
    pub fn first_param_error(&self) -> Option<usize> {
        self.status().iter().position(|&code| code == PARAM_ERROR)
    }

    /// 状态数组里**被驱动标记为成功**的行数。
    ///
    /// 只数 `[PARAM_SUCCESS, PARAM_SUCCESS_WITH_INFO, PARAM_DIAG_UNAVAILABLE]`，
    /// 并把扫描范围限定在第一个失败行之前——驱动可以在失败行之后继续填
    /// `SQL_PARAM_SUCCESS`（那一行没执行），数进来就是凭空多记。
    pub fn succeeded(&self) -> usize {
        let status = self.status();
        let upto = self.first_param_error().unwrap_or(status.len());
        status[..upto]
            .iter()
            .filter(|&&code| is_param_ok(code))
            .count()
    }

    /// `SQL_ATTR_PARAMS_PROCESSED_PTR` 要传给驱动的地址。
    pub fn processed_ptr(&self) -> *mut Len {
        // 用 `as_ref` 拿地址而不是 `&*self.processed`：`processed` 是 `Box`，
        // 指针在对象存活期间稳定（驱动会在 `SQLExecute` 里写它）。
        (&*self.processed) as *const Len as *mut Len
    }

    /// 复位这一趟的记账（保留已分配容量，避免每次重新分配）。
    pub fn clear(&mut self) {
        self.plans.clear();
        self.rows = 0;
    }

    /// 保证 `arena` 至少能容纳 `bytes` 字节，且基址按 8 字节对齐；同时保证
    /// `plans` 能装下 `columns` 个元素（绑定循环里**不能再**让 `plans` 重新分配）。
    ///
    /// 用 **2 的幂** 增长而不是「精确大小」：`SQLBindParameter` 交给驱动的是基址的
    /// 裸指针，所以这块内存在一次绑定到 `SQLExecute` 返回之间**绝不能**再分配。
    /// 提前铺够比在绑定中途扩容安全得多；代价只是某个行数区间多占一倍内存。
    fn ensure(&mut self, bytes: usize, columns: usize) {
        self.plans.reserve(columns);
        if self.arena.len() >= bytes && is_aligned8(self.arena.as_ptr()) {
            return;
        }
        let want = bytes.next_power_of_two().max(64);
        // `Box<[u8]>` 的元素对齐是 1，所以它**不保证** 8 字节对齐；而指标数组与
        // `SQL_NUMERIC_STRUCT` 等结构体都要求 8 字节对齐。多要 8 字节当垫片，
        // 再把**基址**向上取整到 8 的倍数（`align8` 只对齐长度，这里要对齐地址）。
        //
        // 新的分配里每一块都是 8 的倍数长，所以只要基址对齐，后续所有
        // `align8(偏移)` 算出来的列内偏移也一定落在 8 的倍数地址上。
        let raw = vec![0u8; want + 8].into_boxed_slice();
        let lead = align8(raw.as_ptr() as usize) - raw.as_ptr() as usize;
        self.arena = raw[lead..lead + want].to_vec().into_boxed_slice();
        debug_assert!(is_aligned8(self.arena.as_ptr()), "基址必须 8 字节对齐");
    }

    /// 保证状态数组能容纳 `n` 行（2 的幂增长，同 [`ParamArrayArena::ensure`]）。
    fn ensure_status(&mut self, n: usize) {
        if self.status.len() >= n {
            return;
        }
        self.status = vec![PARAM_UNUSED; n.next_power_of_two().max(64)].into_boxed_slice();
    }
}

/// 指针是否按 `Len`（= `isize`）对齐。
///
/// 不用 `ptr::is_aligned_to`：那是 `#![feature(pointer_is_aligned_to)]`，在钉死的
/// stable 工具链（1.95）上编不过。也不用 `% align`——clippy 的
/// `manual_is_multiple_of` 会要求 `is_multiple_of`，而它同样还没 stable。
/// 位掩码是第三种等价写法，且**只对 2 的幂成立**（对齐要求永远满足这一点）。
fn is_aligned8(ptr: *const u8) -> bool {
    ptr as usize & (align_of::<Len>() - 1) == 0
}

impl fmt::Display for ParamArrayArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ParamArrayArena({} 列 × {} 行 / 缓冲 {} 字节)",
            self.plans.len(),
            self.rows,
            self.arena.len()
        )
    }
}

/// ODBC `SQL_PARAM_SUCCESS`。
const PARAM_SUCCESS: u16 = 0;
/// ODBC `SQL_PARAM_SUCCESS_WITH_INFO`。
const PARAM_SUCCESS_WITH_INFO: u16 = 1;
/// ODBC `SQL_PARAM_ERROR`：这一行执行失败。**唯一**能判定行级失败的取值。
pub const PARAM_ERROR: u16 = 2;
/// ODBC `SQL_PARAM_UNUSED`。
const PARAM_UNUSED: u16 = 3;
/// ODBC `SQL_PARAM_DIAG_UNAVAILABLE`。
const PARAM_DIAG_UNAVAILABLE: u16 = 4;

/// 一行参数集在状态数组里的取值是否算「成功」——判定只有这一处。
///
/// 三个取值都算成功：`SUCCESS` 是本行成功；`SUCCESS_WITH_INFO` 是本行成功但另有告警
/// （达梦在「第 `i` 行违反约束」时对**其他**行就会这么报）；`DIAG_UNAVAILABLE` 是
/// 「驱动不提供逐行状态」（规范允许），把它当失败会让一个根本不报状态的驱动
/// 把整批都判成失败。
fn is_param_ok(status: u16) -> bool {
    matches!(
        status,
        PARAM_SUCCESS | PARAM_SUCCESS_WITH_INFO | PARAM_DIAG_UNAVAILABLE
    )
}

/// 每行最多能绑多少个参数集：`SQL_ATTR_PARAMSET_SIZE` 是 `SQLULEN`，但参数序号与
/// 状态数组下标都按 `u16` 走，所以 65535 是本实现的上限（一行语句 6 万个 `?` 已经
/// 远超实用范围）。
const MAX_ROWS: usize = u16::MAX as usize;

/// 一次 [`bind_param_array`] 的结果概要。
///
/// 只带「这一趟绑了几行、驱动处理后说了什么」——把 ODBC 细节留在本模块里，
/// `Backend` 层只需要这三个数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamArraySummary {
    /// 绑进参数数组的行数（= `SQL_ATTR_PARAMSET_SIZE`）。
    pub rows: usize,
    /// `SQLExecute` 之后驱动实际处理的参数集个数（`SQL_ATTR_PARAMS_PROCESSED_PTR`）。
    pub processed: usize,
    /// 状态数组里**被驱动标记为成功**的行数（`SQL_PARAM_SUCCESS` /
    /// `SQL_PARAM_SUCCESS_WITH_INFO` / `SQL_PARAM_DIAG_UNAVAILABLE`）。
    ///
    /// 这不是 `processed`：驱动可以把失败那行也报进 `processed`（它「处理」过了，
    /// 只是没成功）。用 `processed` 记账会把失败那行算成成功行，在「第 500 行违约」
    /// 的场景里直接多记一行——正是 §6.7 的账最容易错的地方。
    pub succeeded: usize,
    /// 状态数组里第一个 `SQL_PARAM_ERROR` 的下标；`None` = 没有任何一行被标为失败。
    pub failed_at: Option<usize>,
    /// `SQLRowCount` 给出的影响行数（驱动没报时为 `None`）。
    pub affected: Option<u64>,
}

impl ParamArraySummary {
    /// 成功执行的参数集个数。
    pub fn executed(&self) -> usize {
        self.succeeded
    }
}

/// 校验列式形状，返回行数。
///
/// 形状错误必须在**碰 ODBC 之前**拒绝：一次数组绑定会下发 `N` 行，形状错了以后
/// 报出来的服务端错误（主键冲突、类型不符）会让人往完全错误的方向查。这里给的是
/// 「第几列传了几个」这种能直接对上调用代码的信息。
fn array_shape(columns: &[Vec<DmValue>]) -> Result<usize, DmError> {
    let Some(first) = columns.first() else {
        return Err(DmError::at(
            DmErrorKind::Config(
                "参数数组是空的：列数为 0 表示「语句没有 `?`」，但批量写至少要指向一列（哪怕每列只有 1 个元素）"
                    .to_owned(),
            ),
            Stage::Bind,
        ));
    };
    let rows = first.len();
    if let Some((index, column)) = columns
        .iter()
        .enumerate()
        .find(|(_, column)| column.len() != rows)
    {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "参数数组各列长度不一致：第 0 列有 {rows} 个元素，第 {index} 列有 {} 个——\
                 列式数组的每一列都必须是同一批参数的同一个 `?`",
                column.len()
            )),
            Stage::Bind,
        ));
    }
    if rows == 0 {
        return Err(DmError::at(
            DmErrorKind::Config(
                "参数数组有 0 行：空批次没有可执行内容。批量写的契约是「至少一组参数」，\
                 调用方应当在收集参数时先判空"
                    .to_owned(),
            ),
            Stage::Bind,
        ));
    }
    if rows > MAX_ROWS {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "参数数组有 {rows} 行，超过本实现的上限 {MAX_ROWS}：请拆成多批"
            )),
            Stage::Bind,
        ));
    }
    Ok(rows)
}

/// 把列式参数绑成 ODBC 参数数组（T6.4，§4.7）。
///
/// `columns[p][r]` 是第 `r` 组参数的第 `p` 个值。调用前 `stmt` 必须已经
/// [`OdbcStatement::prepare`]，且 `columns.len()` 等于语句的 `?` 个数。
///
/// # 与 [`bind_params`] 的形状差异（以及为什么必须这样）
///
/// 单行绑定的 `BufferLength` 是**那个值**的长度；数组绑定的 `BufferLength` 是
/// **元素步长**，驱动按下标 `r` 算地址 `base + r × BufferLength`。所以：
///
/// * 每列先求「本列元素的最大字节数」，再按它铺 `N` 个槽位；短元素补零。
///   步长取最大值是唯一正确选择——任何更小的步长都会让相邻元素互相覆盖。
/// * 指标是**每个元素一个** `Len`（`SQL_NULL_DATA` 表达 NULL），不是一个参数一个。
/// * `ColumnSize` 取该列的最大字符数/字节数，与步长相自洽（`D16`：不查
///   `SQLDescribeParam`，类型仍完全由 `DmValue` 决定）。
///
/// # 绑定前后的纪律（语句缓存下最容易踩的坑）
///
/// 数组绑定会改**语句句柄**上的三个属性（`PARAMSET_SIZE`/`PARAM_STATUS_PTR`/
/// `PARAMS_PROCESSED_PTR`）。语句是池化复用的，所以 `SQLExecute` 之后必须
/// [`reset_param_array_attrs`] 复位，否则下一条复用这条语句的单行 `execute` 会带着
/// 「一次跑 N 行」的设置执行——那是本项目最隐蔽的一类错。
///
/// 本函数**只**负责铺缓冲与绑定；复位与执行在
/// [`OdbcStatement::exec_param_array`]（那里的清理是结构上保证的）。
pub fn bind_param_array(
    stmt: &mut OdbcStatement<'_>,
    columns: &[Vec<DmValue>],
    arena: &mut ParamArrayArena,
) -> Result<(), DmError> {
    if !stmt.kind().is_prepared() {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "bind_param_array 只能用于预处理过的语句，当前状态是 {:?}：请先 prepare(sql)",
                stmt.kind()
            )),
            Stage::Bind,
        ));
    }

    let rows = array_shape(columns)?;
    let expected = usize::from(stmt.param_count());
    if columns.len() != expected {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "参数数组的列数不符：语句有 {expected} 个 `?` 占位符，本次传了 {} 列",
                columns.len()
            )),
            Stage::Bind,
        ));
    }
    for (index, column) in columns.iter().enumerate() {
        if column.len() != rows {
            return Err(DmError::at(
                DmErrorKind::Config(format!(
                    "第 {index} 列有 {} 个元素，其他列有 {rows} 个——列式数组要求每列等长",
                    column.len()
                )),
                Stage::Bind,
            ));
        }
    }

    // 上一趟的指针已经交给驱动；重新铺缓冲前先让驱动松开它们（也顺便把
    // `PARAMSET_SIZE` 归位）。不先复位就直接改缓冲内容 = 驱动仍指着一块会被覆盖的内存。
    reset_param_array_attrs(stmt)?;

    arena.clear();
    gather_plans(columns, rows, arena)?;
    let plans = arena.plans.clone();
    write_columns(columns, rows, &plans, arena)?;
    bind_columns(stmt, arena)
}

/// 第一趟：问类型、定步长、铺出每列的值缓冲与指标数组（**不写值**）。
///
/// 先布置、后写值，是因为步长取决于「本列最长元素」——不先量一遍就不知道往哪写。
/// 量长度这一步复用 [`ParamArrayArena::scratch`]（`encode_one` 往哪写都行，
/// 它返回的 `buf_len`/`column_size` 才是我们要的）。
fn gather_plans(
    columns: &[Vec<DmValue>],
    rows: usize,
    arena: &mut ParamArrayArena,
) -> Result<(), DmError> {
    let mut max_len: Vec<usize> = Vec::with_capacity(columns.len());
    let mut column_size: Vec<usize> = Vec::with_capacity(columns.len());
    // 类型取**本列第一个**元素的映射。一列里的值必须是同一种 `DmValue`，否则
    // 驱动按一个类型解释两种布局（INT 的 4 字节被当 VARCHAR 读），结果不可预期。
    // 形状错误在这里变成一条能直接对上调用代码的错误。
    let mut kinds: Vec<(CDataType, SqlDataType, i16)> = Vec::with_capacity(columns.len());

    for column in columns {
        let mut widest = 0usize;
        let mut widest_chars = 0usize;
        let mut kind: Option<(CDataType, SqlDataType, i16)> = None;
        for value in column {
            arena.scratch.clear();
            let (spec, _off) = encode_one(&mut arena.scratch, value)?;
            // 元素步长的下界就是「最长元素的字节数」，取不到比它更小的合法值。
            widest = widest.max(spec.buf_len);
            widest_chars = widest_chars.max(spec.column_size);
            match kind {
                None => kind = Some((spec.ctype, spec.dtype, spec.decimal_digits)),
                Some(seen) if (seen.0, seen.1) != (spec.ctype, spec.dtype) => {
                    return Err(DmError::at(
                        DmErrorKind::Config(format!(
                            "同一列里出现了两种参数类型：{:?} 与 {:?}——数组绑定按列解释缓冲，\
                             一列里混类型会让驱动按同一个类型读两种布局",
                            (seen.0, seen.1),
                            (spec.ctype, spec.dtype)
                        )),
                        Stage::Bind,
                    ));
                }
                Some(_) => {}
            }
        }
        // 空串/NULL 列：`widest == 0`。步长为 0 会让驱动把同一个地址算给所有元素，
        // 所以给一个最小步长；反正这一列没有任何字节要读。
        max_len.push(widest.max(1));
        column_size.push(widest_chars);
        // 形状校验已经保证 `columns` 非空，而每列至少一个元素（`rows >= 1`）。
        let seen = kind.expect("array_shape 已保证每列至少一个元素");
        kinds.push(seen);
    }

    // 先铺到上界，再回填精确偏移：上界 = Σ(每列 N × 步长 + 对齐余量) +
    // Σ(每列指标数组 N × 8 + 对齐余量) + 整体余量。提前铺够是硬要求——绑定之后
    // 驱动握着裸指针，中途不能再分配。
    let upper: usize = max_len
        .iter()
        .map(|len| rows.saturating_mul(*len) + 16)
        .sum::<usize>()
        + columns.len() * (rows.saturating_mul(size_of::<Len>()) + 16)
        + 64;
    arena.ensure_status(rows);
    arena.ensure(upper, columns.len());
    arena.rows = rows;
    arena.status.fill(PARAM_UNUSED);

    let mut data_end = 0usize;
    for (index, stride) in max_len.iter().enumerate() {
        let stride = *stride;
        let buf_off = align8(data_end);
        let ind_off = align8(buf_off + stride * rows);
        data_end = ind_off + size_of::<Len>() * rows;
        debug_assert!(data_end <= arena.arena.len(), "上界估算必须覆盖真实布局");

        arena.plans.push(ParamArrayColumnPlan {
            ctype: kinds[index].0,
            dtype: kinds[index].1,
            column_size: column_size[index],
            decimal_digits: kinds[index].2,
            buf_off,
            stride,
            ind_off,
        });
    }

    // 值缓冲必须清零：短元素补零，而 `SQL_NULL_DATA` 的元素**完全不写**值缓冲。
    // 复用 `arena.arena` 时上一趟的内容还在，不清零就会把上一批的数据当成这一批的。
    arena.arena.fill(0);
    Ok(())
}

/// 第二趟：把每个元素编码进它自己的槽位，并写下指标。
///
/// 因为值缓冲已经在第一趟按最终尺寸铺好并清零，这一趟**不碰布局**，只写内容。
/// 传进来的 `plans`/`arena` 是分开的可变借用（调用点先 clone 一份布局），
/// 免得在遍历布局的同时又要可变地写缓冲。
fn write_columns(
    columns: &[Vec<DmValue>],
    rows: usize,
    plans: &[ParamArrayColumnPlan],
    arena: &mut ParamArrayArena,
) -> Result<(), DmError> {
    let base = arena.arena.as_mut_ptr() as usize;
    // 布局是 `Copy` 的小数组，先拷成局部量：循环里要同时「读布局」与「可变写缓冲」，
    // 直接借 `arena.plans` 会与 `arena.arena` 的可变借用冲突。列数很少，代价可忽略。
    let plans: Vec<ParamArrayColumnPlan> = plans.to_vec();
    // 编码临时缓冲：`encode_one` 与最终缓冲必须分开——写进去的是它给的字节数。
    let mut scratch: Vec<u8> = Vec::new();

    for (col, column) in columns.iter().enumerate() {
        let plan = plans[col];
        for (row, value) in column.iter().enumerate().take(rows) {
            scratch.clear();
            let (spec, _off) = encode_one(&mut scratch, value)?;
            debug_assert!(
                spec.buf_len <= plan.stride,
                "步长必须 ≥ 任何元素的实际字节数（第一趟量过）"
            );
            let value_start = base + plan.buf_off + row * plan.stride;
            let ind_at = base + plan.ind_off + row * size_of::<Len>();
            // SAFETY: `plan.buf_off + row * stride + spec.buf_len <= arena.arena.len()`：
            // `stride` 是这一列的最大元素长度、`row < rows`，而第一趟按 `rows * stride`
            // 给这一列铺了空间，并保证指标区紧随其后且不重叠。`base` 是本函数刚刚取的
            // 可变指针，本函数持有 `&mut arena`（无别名读写）；写入量由 `spec.buf_len`
            // 界定，不会越出该元素的步长槽位。
            unsafe {
                std::ptr::copy_nonoverlapping(
                    scratch.as_ptr(),
                    value_start as *mut u8,
                    spec.buf_len,
                );
                // `SQL_NULL_DATA(-1)` 与元素字节长度共用这支指标，写进 `[Len; rows]`
                // 的第 `row` 格。
                std::ptr::write(ind_at as *mut Len, spec.ind);
            }
        }
    }
    Ok(())
}

/// 第三趟：每列一次 `SQLBindParameter`，并设好数组相关的三个语句属性。
fn bind_columns(stmt: &mut OdbcStatement<'_>, arena: &ParamArrayArena) -> Result<(), DmError> {
    let rows = arena.rows();
    let columns = arena.column_count();
    set_paramset_size(stmt, rows)?;

    // 状态数组与已处理行数数组：**必须**在 `SQLExecute` 之前设好，否则行级失败
    // 完全看不到（`sql_ok` 会把 `SUCCESS_WITH_INFO` 当成功，只留一条 warn）。
    let status_ptr = arena.status.as_ptr() as Pointer;
    set_stmt_attr(
        stmt,
        StatementAttribute::ParamStatusPtr,
        status_ptr,
        IS_POINTER,
        "SQL_ATTR_PARAM_STATUS_PTR",
    )?;
    let processed_ptr = arena.processed_ptr() as Pointer;
    set_stmt_attr(
        stmt,
        StatementAttribute::ParamsProcessedPtr,
        processed_ptr,
        IS_POINTER,
        "SQL_ATTR_PARAMS_PROCESSED_PTR",
    )?;

    let hstmt: HStmt = stmt.statement().as_sys();
    let base = arena.arena.as_ptr() as usize;

    for (index, plan) in arena.plans.iter().enumerate() {
        let Ok(number) = u16::try_from(index + 1) else {
            return Err(DmError::at(
                DmErrorKind::Config(format!("参数序号 {} 超过 ODBC 的 u16 上限", index + 1)),
                Stage::Bind,
            ));
        };

        // SAFETY: 与 `bind_params` 同一套推理。`arena.arena` 在这次绑定到
        // `SQLExecute` 返回之间不再改变（`ParamArrayArena::ensure` 已经把容量铺到
        // 上界，且数组绑定的调用方拿不到中间态）；`plan.buf_off`/`plan.ind_off` 都在
        // 缓冲范围内且按 8 字节对齐；`hstmt` 是本函数借用的语句句柄，整个循环有效。
        // 驱动会**写**指标数组（每行一个 `Len`），那块内存由 `arena` 独占。
        let ret = unsafe {
            SQLBindParameter(
                hstmt,
                number,
                ParamType::Input,
                plan.ctype,
                plan.dtype,
                plan.column_size as ULen,
                plan.decimal_digits,
                (base + plan.buf_off) as Pointer,
                plan.stride as Len,
                (base + plan.ind_off) as *mut Len,
            )
        };
        diag::sql_ok(
            fold_sql_return(ret, "SQLBindParameter"),
            stmt.statement(),
            Stage::Bind,
            "SQLBindParameter",
        )?;
    }
    debug_assert_eq!(columns, arena.column_count(), "每一列都必须绑过一次");
    debug_assert!(rows > 0, "0 行不该走到绑定（array_shape 已拒绝）");
    Ok(())
}

/// 把数组绑定用过的语句属性全部复位（`PARAMSET_SIZE` 回 1、两个指针置空）。
///
/// **这是语句缓存下的必需步骤**，不是清理癖好：语句句柄在池里复用，属性是**句柄级**
/// 状态而不是调用级状态。不复位的话，下一次 `SQLExecute` 会按上一次的 `PARAMSET_SIZE`
/// 执行——一条本该插一行的语句插了 1000 行（或者反过来读越界缓冲）。
///
/// 指针置空（NULL）是 ODBC 规范定义的「不再需要驱动回报状态」的写法，所以这里
/// 传空指针而不是留着旧地址（旧地址在下一轮 `clear()` 之后就是悬的）。
pub fn reset_param_array_attrs(stmt: &mut OdbcStatement<'_>) -> Result<(), DmError> {
    set_paramset_size(stmt, 1)?;
    set_stmt_attr(
        stmt,
        StatementAttribute::ParamStatusPtr,
        std::ptr::null_mut(),
        IS_POINTER,
        "SQL_ATTR_PARAM_STATUS_PTR",
    )?;
    set_stmt_attr(
        stmt,
        StatementAttribute::ParamsProcessedPtr,
        std::ptr::null_mut(),
        IS_POINTER,
        "SQL_ATTR_PARAMS_PROCESSED_PTR",
    )?;
    // 参数缓冲的归属也交还给「没人绑过」：下一趟单行 `bind_params` 一定会重绑，
    // 不会以为句柄上还留着有效的绑定。
    stmt.reset_parameters()
}

/// `SQLSetStmtAttr(SQL_ATTR_PARAMSET_SIZE, n)`。
fn set_paramset_size(stmt: &mut OdbcStatement<'_>, rows: usize) -> Result<(), DmError> {
    set_stmt_attr(
        stmt,
        StatementAttribute::ParamsetSize,
        rows as Pointer,
        IS_UINTEGER,
        "SQL_ATTR_PARAMSET_SIZE",
    )
}

/// 设一个语句属性，失败走统一诊断口径。
fn set_stmt_attr(
    stmt: &mut OdbcStatement<'_>,
    attr: StatementAttribute,
    value: Pointer,
    length: i32,
    function: &'static str,
) -> Result<(), DmError> {
    let hstmt: HStmt = stmt.statement().as_sys();
    // SAFETY: `hstmt` 来自本函数借用的语句句柄，调用期间有效。`value` 的合法性由
    // 调用点保证：要么是一个 `SQL_IS_*` 编码的整数（按 ODBC 规范直接塞进指针），
    // 要么是 `arena` 里一块活到 `SQLExecute` 之后的缓冲地址。
    let ret = unsafe { SQLSetStmtAttrW(hstmt, attr, value, length) };
    diag::sql_ok(
        fold_sql_return(ret, function),
        stmt.statement(),
        Stage::Bind,
        function,
    )
}

/// 把裸 `SqlReturn` 折成 `SqlResult<()>`（与 [`bind_params`] 同一份折叠）。
fn fold_sql_return(ret: SqlReturn, function: &'static str) -> SqlResult<()> {
    diag::fold_return(ret, function)
}

// ---------------------------------------------------------------------------
// 规划：DmValue → ODBC 类型
// ---------------------------------------------------------------------------

/// 一个 `DmValue` 对应的 `(ValueType, ParameterType)`。
///
/// **这是全模块唯一一处类型映射**：绑定前的「要不要复位」判断与真正编码都走它，
/// 免得两处各写一份、日后改一处漏一处。
///
/// 类型不匹配（比如把字符串塞进 INT 列）**不在这里拦**：那要看目标列，而 §4.4 实测
/// `SQLDescribeParam` 不可用，看不到目标列。让服务端报错比在这里猜一个更可靠，也
/// 符合 §5.10「未声明支持即拒绝，不静默降级」的反面——这里不是「不支持」，是
/// 「该由服务端判」。
fn type_of(value: &DmValue) -> Result<(CDataType, SqlDataType), DmError> {
    Ok(match value {
        // NULL 没有类型。挑文本路径是因为它对目标列的适配面最宽（塞进 INT 列的 NULL
        // 由服务端转换），而且我们无论如何都要给一个合法缓冲与指标。
        DmValue::Null => (CDataType::WChar, SqlDataType::VARCHAR),
        DmValue::Bool(_) => (CDataType::Bit, SqlDataType::EXT_BIT),
        DmValue::TinyInt(_) => (CDataType::STinyInt, SqlDataType::EXT_TINY_INT),
        DmValue::SmallInt(_) => (CDataType::SShort, SqlDataType::SMALLINT),
        DmValue::Int(_) => (CDataType::SLong, SqlDataType::INTEGER),
        DmValue::BigInt(_) => (CDataType::SBigInt, SqlDataType::EXT_BIG_INT),
        DmValue::Real(_) => (CDataType::Float, SqlDataType::REAL),
        DmValue::Double(_) => (CDataType::Double, SqlDataType::DOUBLE),
        DmValue::Decimal(_) => (CDataType::Numeric, SqlDataType::DECIMAL),
        DmValue::Text(_) => (CDataType::WChar, SqlDataType::VARCHAR),
        DmValue::Bytes(_) => (CDataType::Binary, SqlDataType::EXT_VAR_BINARY),
        DmValue::Date(_) => (CDataType::TypeDate, SqlDataType::DATE),
        // 时间参数**不用** `SQL_C_TYPE_TIME`：ODBC 的 `SQL_TIME_STRUCT` 只有时/分/秒，
        // **没有小数秒**，而达梦的 `TIME(6)` 有。按 `SQL_TIMESTAMP_STRUCT` 编码
        // （见 [`encode_one`] 的 `DmValue::Time` 臂），目标 SQL 类型仍诚实报 `TIME`。
        DmValue::Time(_) => (CDataType::TypeTimestamp, SqlDataType::TIME),
        DmValue::Timestamp(_) => (CDataType::TypeTimestamp, SqlDataType::TIMESTAMP),
        DmValue::Interval(_) => {
            return Err(unsupported(
                "INTERVAL 参数（§5.10 的 interval_type 还是 ⚠️ 待验证项，v1.0 不声明支持）",
            ));
        }
        DmValue::Unknown(_) => {
            return Err(unsupported(
                "未知类型码（SQL_UNKNOWN_TYPE）参数：请先用 CAST 明确目标类型再绑定",
            ));
        }
        // `DmValue` 是 `#[non_exhaustive]` 的，所以必须留一条兜底臂。
        // 兜底的方向是**拒绝**而不是猜一个类型：猜错会变成「静默写坏数据」。
        _ => return Err(unsupported("未识别的 DmValue 变体")),
    })
}

/// 「本层不声明支持」的统一错误。
fn unsupported(what: &str) -> DmError {
    DmError::at(
        DmErrorKind::Unsupported {
            feature: format!("绑定 {what}"),
        },
        Stage::Bind,
    )
}

// ---------------------------------------------------------------------------
// 编码：写进 arena
// ---------------------------------------------------------------------------

/// 编码一个参数时算出来的、`SQLBindParameter` 需要的东西（除偏移外）。
struct ParamSpec {
    ctype: CDataType,
    dtype: SqlDataType,
    column_size: usize,
    decimal_digits: i16,
    buf_len: usize,
    ind: isize,
}

/// 把 `value` 编码进 `out`，返回元数据与值缓冲的偏移。
///
/// 只负责写字节：**不分配**（调用方已 `reserve`）、不碰句柄、不认识诊断。
fn encode_one(out: &mut Vec<u8>, value: &DmValue) -> Result<(ParamSpec, usize), DmError> {
    let (ctype, dtype) = type_of(value)?;

    // 值缓冲 8 字节对齐：整数与指标本身宽 8，错位读写是未定义行为。
    let buf_off = align8(out.len());
    out.resize(buf_off, 0);

    let (column_size, decimal_digits, buf_len, ind) = match value {
        // D8 纪律②：NULL 只占位，不写值。指标是 -1，服务端不会去读这 2 个字节。
        //
        // `column_size` 给 **1** 而不是 0：占位缓冲就是 1 个 UTF-16 码元，两者自洽；
        // 而 `ColumnSize = 0` 在 ODBC 里是「长度未知」的退化写法，有的驱动会拿它去
        // 算内部缓冲大小。指标既然是 `SQL_NULL_DATA`，服务端本来就不会读这块。
        DmValue::Null => {
            out.extend_from_slice(&[0u8; 2]);
            (1usize, 0i16, 2usize, NULL_DATA)
        }
        DmValue::Bool(v) => {
            out.push(u8::from(*v));
            (1, 0, 1, 1)
        }
        DmValue::TinyInt(v) => {
            out.extend_from_slice(&v.to_ne_bytes());
            (3, 0, size_of::<i8>(), size_of::<i8>() as isize)
        }
        DmValue::SmallInt(v) => {
            out.extend_from_slice(&v.to_ne_bytes());
            (5, 0, size_of::<i16>(), size_of::<i16>() as isize)
        }
        DmValue::Int(v) => {
            out.extend_from_slice(&v.to_ne_bytes());
            (10, 0, size_of::<i32>(), size_of::<i32>() as isize)
        }
        DmValue::BigInt(v) => {
            out.extend_from_slice(&v.to_ne_bytes());
            (19, 0, size_of::<i64>(), size_of::<i64>() as isize)
        }
        DmValue::Real(v) => {
            out.extend_from_slice(&v.to_ne_bytes());
            (7, 0, size_of::<f32>(), size_of::<f32>() as isize)
        }
        DmValue::Double(v) => {
            out.extend_from_slice(&v.to_ne_bytes());
            (15, 0, size_of::<f64>(), size_of::<f64>() as isize)
        }
        DmValue::Decimal(v) => {
            let numeric = pack_numeric(v);
            // SAFETY: `NumericStruct` 是 `#[repr(C)]` 且字段全是整数，没有指针也没有
            // 填充字节，按字节读它是定义良好的。
            let bytes = unsafe { pod_bytes(&numeric) };
            out.extend_from_slice(bytes);
            (
                usize::from(numeric.precision),
                i16::from(numeric.scale),
                bytes.len(),
                bytes.len() as isize,
            )
        }
        DmValue::Text(s) => {
            // 字符数（`column_size` 要的）与 UTF-16 码元数（字节数要的）不是一回事：
            // 一个增补平面字符是 1 个 `char`、2 个码元、4 个字节。所以要分别数，
            // 而且字节数**以实际写进去的为准**（写多少报多少，见下面 D8 纪律④）。
            let chars = s.chars().count();
            let start = out.len();
            out.extend(s.encode_utf16().flat_map(u16::to_ne_bytes));
            let bytes = out.len() - start;
            (chars, 0, bytes, bytes as isize)
        }
        DmValue::Bytes(b) => {
            out.extend_from_slice(b);
            (b.len(), 0, b.len(), b.len() as isize)
        }
        DmValue::Date(d) => {
            use chrono::Datelike;
            let raw = odbc_sys::Date {
                year: d.year() as i16,
                month: d.month() as u16,
                day: d.day() as u16,
            };
            // SAFETY: `odbc_sys::Date` 是 `#[repr(C)]` 的三个整数，无指针无填充。
            let bytes = unsafe { pod_bytes(&raw) };
            out.extend_from_slice(bytes);
            (0, 0, bytes.len(), bytes.len() as isize)
        }
        DmValue::Time(t) => {
            use chrono::Timelike;
            // **不用 `odbc_sys::Time`**（= `SQL_TIME_STRUCT`）：它只有时/分/秒三个字段，
            // **没有小数秒**，达梦 `TIME(6)` 的微秒在这一步被整段丢掉——官方套件
            // `type_jiff::ty_time`（真机 `left: 23:59:59` / `right: 23:59:59.999999`）与
            // `ty_time_precision_2`（`left: 14:30:45` / `right: 14:30:45.12`）就是这一处。
            //
            // 改按 `SQL_TIMESTAMP_STRUCT` 编码，`ValueType` 报 `SQL_C_TYPE_TIMESTAMP`，
            // `ParameterType` 仍报 `SQL_TIME`（目标列确实是 TIME）。ODBC 的转换规则里
            // `SQL_C_TYPE_TIMESTAMP → SQL_TYPE_TIME` 正是「丢日期、留小数秒」，与
            // 「事务里只关心时分秒」的语义一致。日期填 **1970-01-01**，转换时被丢掉。
            let raw = odbc_sys::Timestamp {
                year: 1970,
                month: 1,
                day: 1,
                hour: t.hour() as u16,
                minute: t.minute() as u16,
                second: t.second() as u16,
                // 与 `DmValue::Timestamp` 同规：`fraction` 的单位是**纳秒**。
                fraction: t.nanosecond(),
            };
            // SAFETY: `odbc_sys::Timestamp` 是 `#[repr(C)]` 的整数与一个 u32，无指针无填充。
            let bytes = unsafe { pod_bytes(&raw) };
            out.extend_from_slice(bytes);
            // `ColumnSize` 报**字符串形态长度**（`HH:MM:SS.FFFFFF` = 15），`DecimalDigits`
            // 报小数秒位数 6：ODBC 规范里日期时间型就是这么表达的。之前报 `(0, 0)`，
            // 达梦按「精度 0」处理，`TIME(6)` 的微秒在**参数**这一侧被截掉（套件
            // `type_jiff::ty_time` 的另一半，见第 28 节探针）。
            (15, 6, bytes.len(), bytes.len() as isize)
        }
        DmValue::Timestamp(ts) => {
            use chrono::{Datelike, Timelike};
            let raw = odbc_sys::Timestamp {
                year: ts.year() as i16,
                month: ts.month() as u16,
                day: ts.day() as u16,
                hour: ts.hour() as u16,
                minute: ts.minute() as u16,
                second: ts.second() as u16,
                // `SQL_TIMESTAMP_STRUCT.fraction` 的单位是**纳秒**（ODBC 规范），
                // 不是微秒。真机实测（T3.5）：按微秒填会让服务端少存三个数量级。
                fraction: ts.nanosecond(),
            };
            // SAFETY: 同上，`odbc_sys::Timestamp` 是 `#[repr(C)]` 的整数与一个 u32。
            let bytes = unsafe { pod_bytes(&raw) };
            out.extend_from_slice(bytes);
            // `ColumnSize` = `YYYY-MM-DD HH:MM:SS.FFFFFF` 的字符数（26），`DecimalDigits` = 6。
            //
            // 之前报 `(0, 0)`（= ODBC 里「长度未知/精度 0」）时，达梦把**绑定参数**按整秒
            // 处理：探针第 27 节 27b2 里 `TO_CHAR(?, '…FF6')` 回 `2024-01-02 03:04:05.000000`，
            // 于是 `CAST(文档叶子 AS TIMESTAMP(6)) = ?` 恒不匹配（27c/27e2）。写路径（目标列
            // 是 `TIMESTAMP(6)`）不受影响，只有「参数 vs 列/表达式」的比较会丢微秒。
            (26, 6, bytes.len(), bytes.len() as isize)
        }
        // `type_of` 已经把这些挡掉了，走到这里说明两个 match 的覆盖面分了家。
        DmValue::Interval(_) | DmValue::Unknown(_) => {
            return Err(unsupported("该类型的参数"));
        }
        _ => return Err(unsupported("未识别的 DmValue 变体")),
    };

    // D8 纪律④：同一份数据量两遍（写进去多少 vs 报给驱动多少），不一致立刻炸。
    debug_assert_eq!(
        out.len() - buf_off,
        buf_len,
        "编码出的字节数与报给驱动的不一致"
    );
    debug_assert!(
        ind == NULL_DATA || ind as usize == buf_len,
        "指标必须要么是 SQL_NULL_DATA，要么等于字节长度"
    );

    Ok((
        ParamSpec {
            ctype,
            dtype,
            column_size,
            decimal_digits,
            buf_len,
            ind,
        },
        buf_off,
    ))
}

/// 按 8 字节向上对齐。
fn align8(n: usize) -> usize {
    (n + 7) & !7
}

/// 把一个 `#[repr(C)]` POD 结构体看成字节切片。
///
/// # Safety
///
/// `T` 必须是 `#[repr(C)]`、字段全是整数、且没有指针成员与填充字节的 POD。否则把
/// 它的字节当值用（或者反过来）是未定义行为。调用点必须能自己说清为什么满足。
unsafe fn pod_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: 由调用方保证 `T` 是 POD；`size_of::<T>()` 覆盖整个值，返回的切片不越界，
    // 存活期跟着 `value` 走。
    unsafe { std::slice::from_raw_parts(std::ptr::from_ref(value).cast::<u8>(), size_of::<T>()) }
}

/// 把 `rust_decimal::Decimal` 打成 ODBC 的 `SQL_NUMERIC_STRUCT`。
///
/// 三个容易错的点，都在这里一次说清：
///
/// * **val 是绝对值**。ODBC 的 `val` 不存补码，`-1.5` 要写成 `sign = 0` + `val = 15`。
///   直接塞 `-15` 的补码，服务端会读成一个巨大的正数。
/// * **val 是小端**，高位补 0。
/// * **precision 不能小于 scale**。`0.05` 的尾数是 `5`（1 位），但小数位是 2 位；
///   报 `precision = 1` 有的驱动会直接拒。取两者较大值。
fn pack_numeric(value: &rust_decimal::Decimal) -> NumericStruct {
    let mantissa = value.mantissa();
    let scale = value.scale();
    let magnitude = mantissa.unsigned_abs();

    let digits = if magnitude == 0 {
        1
    } else {
        magnitude.ilog10() + 1
    };
    let precision = digits.max(scale).min(u32::from(u8::MAX)) as u8;

    NumericStruct {
        precision,
        scale: scale as i8,
        sign: u8::from(mantissa >= 0),
        val: magnitude.to_le_bytes(),
    }
}

/// 第一个参数之前先 `reserve` 这么多字节，保证写入那趟不重分配。
///
/// 是**上界**不是精确值，取宽一点没关系（多留的容量一次就够了，之后不再长）：
///
/// * 文本：UTF-16 码元数 ≤ UTF-8 字节数，所以 `len * 2` 一定够；
/// * 二进制/未知码：就是 `len`；
/// * 其余（含 `Timestamp` 的 16 字节与 `SQL_NUMERIC_STRUCT` 的 19 字节）：
///   [`FIXED_PAYLOAD_ESTIMATE`] 封顶。
///
/// 每参数还要加 [`PER_PARAM_SLACK`]（对齐填充 + 指标），末尾再留一份给
/// **第一个参数之前的对齐填充**与总和上的零头。
fn upper_bound(args: &[DmValue]) -> usize {
    let payload: usize = args
        .iter()
        .map(|v| match v {
            DmValue::Text(s) => s.len().saturating_mul(2),
            DmValue::Bytes(b) | DmValue::Unknown(b) => b.len(),
            _ => FIXED_PAYLOAD_ESTIMATE,
        })
        .sum();
    payload + (args.len() + 1) * PER_PARAM_SLACK
}

// ---------------------------------------------------------------------------
// 绑定
// ---------------------------------------------------------------------------

/// 把 `args` 绑到 `stmt` 的 `?` 上（§6.3）。
///
/// `arena` 由调用方持有并跨次复用；绑定后的裸指针指向 `arena` 的堆缓冲，所以在本次
/// 绑定与下一次绑定之间，**不要**动 `arena`。
///
/// 调用前 `stmt` 必须已经 [`OdbcStatement::prepare`]：只有预处理过的语句才有确定的
/// `?` 个数。个数对不上会**提前拒绝**（而不是把 `07002/-70039` 留给服务端）——
/// 那是调用方用法错误，越早说越省事。
pub fn bind_params(
    stmt: &mut OdbcStatement<'_>,
    args: &[DmValue],
    arena: &mut ParamArena,
) -> Result<(), DmError> {
    if !stmt.kind().is_prepared() {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "bind_params 只能用于预处理过的语句，当前状态是 {:?}：\
                 请先 prepare(sql)，或者用 exec_direct(sql) 一次性下发",
                stmt.kind()
            )),
            Stage::Bind,
        ));
    }

    let expected = usize::from(stmt.param_count());
    if args.len() != expected {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "参数个数不符：语句有 {expected} 个 `?` 占位符，本次传了 {} 个",
                args.len()
            )),
            Stage::Bind,
        ));
    }

    // §6.3：只有参数个数或类型变了才复位。类型序列靠 `type_of` 比，不靠猜。
    // 注意这一步在真正写入之前——不支持的类型会在这里就报错，不会写坏 arena。
    let mut needs_reset = arena.metas.len() != args.len();
    if !needs_reset {
        for (meta, arg) in arena.metas.iter().zip(args) {
            let (ctype, dtype) = type_of(arg)?;
            if meta.ctype != ctype || meta.dtype != dtype {
                needs_reset = true;
                break;
            }
        }
    }
    if needs_reset {
        stmt.reset_parameters()?;
        arena.clear();
    }

    arena.metas.clear();
    arena.arena.clear();
    arena.arena.reserve(upper_bound(args));

    for arg in args {
        push_param(arena, arg)?;
    }

    // SAFETY: `arena.arena` 在上面的 `reserve` 之后到本函数返回为止不再改变长度，
    // 所以 `base` 到 `base + arena.arena.len()` 一直是这块分配的有效范围，且
    // `buf_off`/`ind_off` 都落在其中（写入那趟刚把它们铺出来）。
    let base = arena.arena.as_mut_ptr();

    for (index, meta) in arena.metas.iter().enumerate() {
        let number = slot_number(index + 1)?;
        let meta = *meta;
        // SAFETY: 见上面 `base` 的说明。
        unsafe { bind_one(stmt, number, meta, base)? };
    }

    Ok(())
}

/// 把 `args` 绑到 `stmt` 的 `?` 上，并把 `lob.column` 那一处绑成**延迟输入**参数
/// （§6.5 的大 LOB 写，Round 14；机制见 [`crate::lob::LobFeeder`]）。
///
/// # 槽位怎么排
///
/// [`LobParam::column`] 是 **0 起算的 `?` 下标**，它**不消耗** `args` 里的值：写
/// `insert into t (id, body) values (?, ?)` 时传 `args = [id]` 加 `lob.column = 1`。
/// 于是「`?` 总数 = `args.len() + 1`」，多一个少一个都在这里提前拒绝（而不是把
/// `07002/-70039` 留给服务端）。
///
/// # 为什么每次都复位参数
///
/// 延迟输入是**语句句柄级**状态，而且这一轮绑进去的 token 指向本次调用栈上的
/// [`LobFeeder`]。上一轮的绑定必须整体丢掉，否则某个参数位可能还挂着上一轮的 token
/// ——那是个已经失效的地址。所以这里**不做** [`bind_params`] 那套「类型序列没变就不
/// 复位」的优化：延迟输入的复用本来就不该跨次保留。
///
/// # 返回值
///
/// 返回的 [`LobFeeder`] 必须在**同一次执行**里交给
/// [`OdbcStatement::exec_delayed`]，并且不能移动——它自己的地址就是驱动要还回来的 token。
///
/// 所以这里包了一层 [`Box`]：本地变量一定会在 `Ok(feeder)` 时被搬到调用方的栈帧上，
/// 按值返回就等于把 token 指到一个**已经失效的地址**（真机上表现为 `SQLParamData`
/// 还回来的 token 与我们对不上，随后语句停在 `SQL_NEED_DATA`、析构时
/// `SQLFreeHandle` 失败并把 panic 抛进池里）。装箱之后地址固定，这个坑从类型上消失。
pub fn bind_params_with_lob<'l>(
    stmt: &mut OdbcStatement<'_>,
    args: &[DmValue],
    lob: &'l mut LobParam<&mut dyn LobSource>,
    arena: &mut ParamArena,
) -> Result<Box<LobFeeder<'l>>, DmError> {
    if !stmt.kind().is_prepared() {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "bind_params_with_lob 只能用于预处理过的语句，当前状态是 {:?}：\
                 请先 prepare(sql)",
                stmt.kind()
            )),
            Stage::Bind,
        ));
    }

    let expected = usize::from(stmt.param_count());
    if args.len() + 1 != expected {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "参数个数不符：语句有 {expected} 个 `?` 占位符，本次传了 {} 个立即参数 + 1 处分块 LOB",
                args.len()
            )),
            Stage::Bind,
        ));
    }
    if lob.column >= expected {
        return Err(DmError::at(
            DmErrorKind::Config(format!(
                "分块 LOB 的列下标 {} 越界：语句只有 {expected} 个 `?`（合法下标 0..{expected}）",
                lob.column
            )),
            Stage::Bind,
        ));
    }

    stmt.reset_parameters()?;
    arena.clear();
    arena.arena.reserve(upper_bound(args));

    // `metas` 与「槽位」不再一一对应（LOB 那处不占 arena），所以记一张槽位 → 账目 的表。
    let mut meta_of_slot = vec![usize::MAX; expected];
    for (index, arg) in args.iter().enumerate() {
        // 第 `lob.column` 个 `?` 是 LOB，它之后的立即参数整体后移一位。
        let slot = if index < lob.column { index } else { index + 1 };
        meta_of_slot[slot] = arena.metas.len();
        push_param(arena, arg)?;
    }

    // 绑定之前先建 feeder：token 就是它的地址。`&mut *lob.source` 是一次重借
    // （feeder 借用 `lob` 里的源，而不拥有它）。装箱的理由见上面的「返回值」一段：
    // 按值返回会把 token 指到调用方的栈帧之外。
    let mut feeder = Box::new(LobFeeder::new(lob.kind, &mut *lob.source)?);

    // SAFETY: `arena.arena` 在 `reserve` 之后到本函数返回为止不再改变长度；下面每个
    // `meta_of_slot` 里的下标都来自刚才那趟 `push_param`（都在 `arena.metas` 范围内）。
    let base = arena.arena.as_mut_ptr();

    for (slot, meta_index) in meta_of_slot.iter().enumerate() {
        let number = slot_number(slot + 1)?;
        if slot == lob.column {
            bind_lob_param(stmt, number, &mut feeder)?;
        } else {
            let meta = arena.metas[*meta_index];
            // SAFETY: 见上面 `base` 的说明。
            unsafe { bind_one(stmt, number, meta, base)? };
        }
    }

    Ok(feeder)
}

/// `?` 序号（1 起算）换算成 ODBC 的 `u16`；超上限是调用方的用法错误。
fn slot_number(slot: usize) -> Result<u16, DmError> {
    u16::try_from(slot).map_err(|_| {
        DmError::at(
            DmErrorKind::Config(format!("参数序号 {slot} 超过 ODBC 的 u16 上限")),
            Stage::Bind,
        )
    })
}

/// 把一个立即参数编码进 `arena` 并记账。
///
/// 值缓冲与指标格都 append 进 `arena.arena`，偏移记进 `arena.metas`——**不存裸指针**
/// （`ParamArena` 一被搬动就全悬了，见 [`ParamMeta`] 的文档）。
fn push_param(arena: &mut ParamArena, arg: &DmValue) -> Result<(), DmError> {
    let (spec, buf_off) = encode_one(&mut arena.arena, arg)?;

    let ind_off = align8(arena.arena.len());
    arena.arena.resize(ind_off + size_of::<Len>(), 0);
    let ind_bytes = spec.ind.to_ne_bytes();
    arena.arena[ind_off..ind_off + ind_bytes.len()].copy_from_slice(&ind_bytes);

    arena.metas.push(ParamMeta {
        ctype: spec.ctype,
        dtype: spec.dtype,
        column_size: spec.column_size,
        decimal_digits: spec.decimal_digits,
        buf_off,
        buf_len: spec.buf_len,
        ind: spec.ind,
        ind_off,
    });
    Ok(())
}

/// 把一个立即参数绑到 `number` 号（1 起算）的 `?` 上。
///
/// # Safety
///
/// `base` 必须是 `arena.arena.as_mut_ptr()`，且**本次调用期间 `arena.arena` 不得重新
/// 分配**（`meta.buf_off`/`meta.ind_off` 是相对它的偏移）。句柄 `hstmt` 来自
/// `stmt.statement()`，调用方持有 `&mut stmt`，句柄在调用期间有效。驱动在
/// `ParamType::Input` 上只**读**值缓冲、**写**指标缓冲——写指标是规范允许的（服务端
/// 可以回报实际长度），而那块内存由 `arena` 独占。
unsafe fn bind_one(
    stmt: &mut OdbcStatement<'_>,
    number: u16,
    meta: ParamMeta,
    base: *mut u8,
) -> Result<(), DmError> {
    // SAFETY: 由调用方保证（见函数文档）。
    let ret = unsafe {
        SQLBindParameter(
            stmt.statement().as_sys(),
            number,
            ParamType::Input,
            meta.ctype,
            meta.dtype,
            meta.column_size as ULen,
            meta.decimal_digits,
            base.add(meta.buf_off) as Pointer,
            meta.buf_len as Len,
            base.add(meta.ind_off) as *mut Len,
        )
    };

    diag::sql_ok(
        fold_sql_return(ret, "SQLBindParameter"),
        stmt.statement(),
        Stage::Bind,
        "SQLBindParameter",
    )
}

/// 把一处延迟输入参数绑到 `number` 号（1 起算）的 `?` 上。
///
/// 与 [`bind_one`] 的三处差别，每一处都有实测依据（`examples/putdata_probe.rs`，Round 14）：
///
/// * `ParameterValuePtr` 不是数据地址，而是驱动要**原样还回来**的 token（feeder 的地址）；
/// * `BufferLength` 给 `0`：延迟输入没有客户端缓冲；
/// * 指标格指向 feeder 自己的 `indicator`（`SQL_LEN_DATA_AT_EXEC(n)` 或 `SQL_DATA_AT_EXEC`），
///   驱动据此决定要不要回 `SQL_NEED_DATA`——达梦会回（探针 8 个用例全部如此）。
fn bind_lob_param(
    stmt: &mut OdbcStatement<'_>,
    number: u16,
    feeder: &mut LobFeeder<'_>,
) -> Result<(), DmError> {
    let (ctype, dtype) = match feeder.kind() {
        LobKind::Binary => (CDataType::Binary, SqlDataType::EXT_VAR_BINARY),
        LobKind::Text => (CDataType::WChar, SqlDataType::VARCHAR),
    };
    let column_size = feeder.column_size();
    let token = feeder.token();
    let indicator = feeder.indicator_ptr();

    // SAFETY: `token` 是上面那个 `feeder` 的地址、`indicator` 是它内部一个稳定的 `Len`
    // 格子；调用方保证 feeder 在整次执行期间不移动，两者都活过本次绑定。
    let ret = unsafe {
        SQLBindParameter(
            stmt.statement().as_sys(),
            number,
            ParamType::Input,
            ctype,
            dtype,
            column_size,
            0,
            token,
            0,
            indicator,
        )
    };

    diag::sql_ok(
        fold_sql_return(ret, "SQLBindParameter"),
        stmt.statement(),
        Stage::Bind,
        "SQLBindParameter",
    )
}

// ---------------------------------------------------------------------------
// 纯逻辑测试（不连库）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(value: &DmValue) -> (ParamSpec, usize, Vec<u8>) {
        let mut out = Vec::new();
        let (spec, off) = encode_one(&mut out, value).expect("应当能编码");
        (spec, off, out)
    }

    #[test]
    fn maps_every_supported_value_to_an_odbc_type() {
        let cases: Vec<(DmValue, CDataType, SqlDataType)> = vec![
            (DmValue::Null, CDataType::WChar, SqlDataType::VARCHAR),
            (DmValue::Bool(true), CDataType::Bit, SqlDataType::EXT_BIT),
            (
                DmValue::TinyInt(1),
                CDataType::STinyInt,
                SqlDataType::EXT_TINY_INT,
            ),
            (
                DmValue::SmallInt(1),
                CDataType::SShort,
                SqlDataType::SMALLINT,
            ),
            (DmValue::Int(1), CDataType::SLong, SqlDataType::INTEGER),
            (
                DmValue::BigInt(1),
                CDataType::SBigInt,
                SqlDataType::EXT_BIG_INT,
            ),
            (DmValue::Real(1.0), CDataType::Float, SqlDataType::REAL),
            (DmValue::Double(1.0), CDataType::Double, SqlDataType::DOUBLE),
            (
                DmValue::Decimal(rust_decimal::Decimal::new(15, 1)),
                CDataType::Numeric,
                SqlDataType::DECIMAL,
            ),
            (
                DmValue::Text("x".into()),
                CDataType::WChar,
                SqlDataType::VARCHAR,
            ),
            (
                DmValue::Bytes(vec![1]),
                CDataType::Binary,
                SqlDataType::EXT_VAR_BINARY,
            ),
            (
                DmValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap()),
                CDataType::TypeDate,
                SqlDataType::DATE,
            ),
            (
                DmValue::Time(chrono::NaiveTime::from_hms_opt(1, 2, 3).unwrap()),
                CDataType::TypeTimestamp,
                SqlDataType::TIME,
            ),
            (
                DmValue::Timestamp(
                    chrono::NaiveDate::from_ymd_opt(2024, 3, 5)
                        .unwrap()
                        .and_hms_opt(1, 2, 3)
                        .unwrap(),
                ),
                CDataType::TypeTimestamp,
                SqlDataType::TIMESTAMP,
            ),
        ];
        for (value, ctype, dtype) in cases {
            assert_eq!(type_of(&value).unwrap(), (ctype, dtype), "{value:?}");
        }
    }

    #[test]
    fn null_binds_a_placeholder_with_the_null_indicator() {
        let (spec, _off, bytes) = enc(&DmValue::Null);
        assert_eq!(spec.ind, NULL_DATA);
        assert_eq!(spec.buf_len, 2, "占位缓冲仍然是 2 字节，只是不写值");
        assert_eq!(
            spec.column_size, 1,
            "占位缓冲是 1 个字符，`ColumnSize` 必须与之自洽而不是 0"
        );
        assert_eq!(bytes, vec![0, 0]);
        assert_eq!(spec.dtype, SqlDataType::VARCHAR);
    }

    #[test]
    fn empty_text_is_zero_length_not_null() {
        // §6.4：NULL 与空串必须严格区分。指标 0 vs -1 就是这条界线。
        let (spec, _off, bytes) = enc(&DmValue::Text(String::new()));
        assert_eq!(spec.ind, 0);
        assert_ne!(spec.ind, NULL_DATA);
        assert_eq!(spec.buf_len, 0);
        assert_eq!(spec.column_size, 0);
        assert!(bytes.is_empty());
    }

    #[test]
    fn text_is_utf16_with_char_count_as_column_size() {
        let (spec, _off, bytes) = enc(&DmValue::Text("中文ab".into()));
        assert_eq!(spec.column_size, 4, "字符数");
        assert_eq!(spec.buf_len, 8, "UTF-16 字节数");
        assert_eq!(bytes.len(), 8);
        assert_eq!(
            u16::from_ne_bytes([bytes[0], bytes[1]]),
            '中' as u16,
            "按 UTF-16 码元写，不是 UTF-8 字节"
        );
    }

    #[test]
    fn supplementary_chars_count_as_one_char_but_four_bytes() {
        // U+1D11E 是增补平面：1 个 char、2 个 UTF-16 码元、4 个字节。
        // 用 `s.len()`（UTF-8 字节 = 4）当字符数就会和缓冲长度错位。
        let (spec, _off, bytes) = enc(&DmValue::Text("\u{1D11E}".into()));
        assert_eq!(spec.column_size, 1);
        assert_eq!(spec.buf_len, 4);
        assert_eq!(bytes.len(), 4);
    }

    #[test]
    fn fixed_width_values_report_their_exact_size() {
        for (value, want) in [
            (DmValue::Bool(true), 1usize),
            (DmValue::TinyInt(-1), 1),
            (DmValue::SmallInt(-1), 2),
            (DmValue::Int(-1), 4),
            (DmValue::BigInt(-1), 8),
            (DmValue::Real(1.5), 4),
            (DmValue::Double(1.5), 8),
        ] {
            let (spec, _off, bytes) = enc(&value);
            assert_eq!(spec.buf_len, want, "{value:?}");
            assert_eq!(bytes.len(), want, "{value:?}");
            assert_eq!(spec.ind as usize, want, "D8 纪律④：ind == buf_len");
        }
    }

    /// CT-11（第 3 类）：**含 NUL** 的文本必须整段写下去。
    ///
    /// 我们给字符参数传的是显式指标（§6.3 纪律①），所以嵌入的 NUL 只是缓冲里的一个码元。
    /// 一旦有人「顺手」按 C 字符串的口径收尾（`CString` / 在 NUL 处截断），长度会静默变短，
    /// 而驱动**不会**报错——它会老老实实按短长度写进库里。
    #[test]
    fn ct11_text_with_embedded_nul_keeps_its_true_length() {
        let (spec, _off, bytes) = enc(&DmValue::Text("a\0b".into()));
        assert_eq!(spec.column_size, 3, "NUL 也占一个字符位");
        assert_eq!(spec.buf_len, 6, "UTF-16 字节数 = 3 个码元 × 2");
        assert_eq!(bytes.len(), 6, "整段写下去，不在第一个 NUL 处截断");
        assert_eq!(
            u16::from_ne_bytes([bytes[2], bytes[3]]),
            0,
            "中间的 NUL 码元"
        );
        assert_eq!(
            u16::from_ne_bytes([bytes[4], bytes[5]]),
            u16::from(b'b'),
            "NUL 之后的数据仍在缓冲里"
        );
        assert_eq!(spec.ind as usize, spec.buf_len, "D8 纪律④：ind == buf_len");
    }

    /// CT-11（第 4 类）：**超 4 KiB** 的值不能被静默截断。
    ///
    /// 4 KiB 是二进制**取数**路径的分块大小（§6.6）；**参数**侧没有这个上限，值缓冲按真实
    /// 长度铺进 arena。文本按 UTF-16 字节算、二进制按字节算，两条都要整段落下去。
    #[test]
    fn ct11_values_over_four_kib_are_not_truncated() {
        // 4097 个码元 × 2 = 8194 字节 > 4096。
        let big_text = "中".repeat(4097);
        let (spec, _off, bytes) = enc(&DmValue::Text(big_text));
        assert_eq!(spec.column_size, 4097, "字符数");
        assert_eq!(spec.buf_len, 8194, "UTF-16 字节数");
        assert_eq!(bytes.len(), 8194, "超 4 KiB 的文本必须整段写下去");
        assert_eq!(spec.ind as usize, spec.buf_len, "D8 纪律④：ind == buf_len");

        let big_bin = vec![0xABu8; 8192];
        let (spec, _off, bytes) = enc(&DmValue::Bytes(big_bin.clone()));
        assert_eq!(spec.buf_len, 8192, "二进制按字节计数");
        assert_eq!(
            bytes, big_bin,
            "超 4 KiB 的二进制必须整段写下去（无 NUL 收尾）"
        );
        assert_eq!(spec.ind as usize, spec.buf_len, "D8 纪律④：ind == buf_len");
    }

    #[test]
    fn date_time_timestamp_use_the_odbc_structs() {
        let (d_spec, _, d) = enc(&DmValue::Date(
            chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap(),
        ));
        assert_eq!(d_spec.buf_len, size_of::<odbc_sys::Date>());
        assert_eq!(d, vec![0xE8, 0x07, 0x03, 0x00, 0x05, 0x00], "2024-03-05");

        // 时间参数按 `SQL_TIMESTAMP_STRUCT` 编码：`SQL_TIME_STRUCT` 没有小数秒字段，
        // 会把 `TIME(6)` 的微秒整段丢掉（套件 `type_jiff::ty_time`）。
        let (t_spec, _, t) = enc(&DmValue::Time(
            chrono::NaiveTime::from_hms_micro_opt(1, 2, 3, 4).unwrap(),
        ));
        assert_eq!(t_spec.buf_len, size_of::<odbc_sys::Timestamp>());
        assert_eq!(t.len(), 16);
        assert_eq!(
            t[0..2],
            1970i16.to_ne_bytes(),
            "日期填 1970-01-01，转换时被丢掉"
        );
        assert_eq!(t[2..4], 1u16.to_ne_bytes(), "月");
        assert_eq!(t[4..6], 1u16.to_ne_bytes(), "日");
        assert_eq!(t[6..8], 1u16.to_ne_bytes(), "时");
        assert_eq!(t[8..10], 2u16.to_ne_bytes(), "分");
        assert_eq!(t[10..12], 3u16.to_ne_bytes(), "秒");
        // 与 Timestamp 同规：fraction 的单位是**纳秒**，4 微秒 ⇒ 4000。
        assert_eq!(u32::from_ne_bytes(t[12..16].try_into().unwrap()), 4_000);

        let (ts_spec, _, ts) = enc(&DmValue::Timestamp(
            chrono::NaiveDate::from_ymd_opt(2024, 3, 5)
                .unwrap()
                .and_hms_micro_opt(1, 2, 3, 4)
                .unwrap(),
        ));
        assert_eq!(ts_spec.buf_len, size_of::<odbc_sys::Timestamp>());
        assert_eq!(ts.len(), 16);
        // fraction 是**纳秒**（ODBC 规范与 T3.5 真机实测），4 微秒 ⇒ 4000。
        assert_eq!(u32::from_ne_bytes(ts[12..16].try_into().unwrap()), 4_000);
    }

    #[test]
    fn decimal_packs_absolute_magnitude_with_a_separate_sign() {
        // -1.5：尾数 -15、scale 1 ⇒ val = 15（绝对值），sign = 0（负）。
        let (spec, _off, bytes) = enc(&DmValue::Decimal(rust_decimal::Decimal::new(-15, 1)));
        assert_eq!(spec.column_size, 2, "有效位数");
        assert_eq!(spec.decimal_digits, 1, "小数位数");
        assert_eq!(bytes.len(), 19, "SQL_NUMERIC_STRUCT 无填充");
        assert_eq!(bytes[0], 2, "precision");
        assert_eq!(bytes[1] as i8, 1, "scale");
        assert_eq!(bytes[2], 0, "sign：负");
        assert_eq!(bytes[3], 15, "val 起点是绝对值");
        assert_eq!(&bytes[4..19], &[0u8; 15], "高位补 0");
    }

    #[test]
    fn decimal_precision_never_drops_below_scale() {
        // 0.05：尾数 5（1 位），scale 2。precision 报 1 有些驱动会拒。
        let (spec, _off, bytes) = enc(&DmValue::Decimal(rust_decimal::Decimal::new(5, 2)));
        assert_eq!(bytes[0], 2, "precision 取 max(有效位数, 小数位数)");
        assert_eq!(spec.column_size, 2);
        assert_eq!(spec.decimal_digits, 2);
    }

    #[test]
    fn layout_keeps_every_buffer_eight_byte_aligned() {
        let mut arena = ParamArena::new();
        arena
            .arena
            .reserve(upper_bound(&[DmValue::Text("a".into()), DmValue::Int(7)]));

        let (_, first) = encode_one(&mut arena.arena, &DmValue::Text("a".into())).unwrap();
        let ind_first = align8(arena.arena.len());
        arena.arena.resize(ind_first + 8, 0);
        let (_, second) = encode_one(&mut arena.arena, &DmValue::Int(7)).unwrap();

        assert_eq!(first, 0);
        assert_eq!(ind_first, 8, "值缓冲占 2 字节，指标对齐到 8");
        assert_eq!(second % 8, 0, "第二个参数的值缓冲也要对齐");
        assert!(second >= ind_first + 8);
    }

    #[test]
    fn binding_an_interval_or_unknown_type_is_refused() {
        for value in [
            DmValue::Unknown(vec![1, 2, 3]),
            DmValue::Interval(dm_core::DmInterval::year_to_month(1, 2, 1)),
        ] {
            let err = type_of(&value).expect_err("应当拒绝");
            assert!(
                matches!(err.kind, DmErrorKind::Unsupported { .. }),
                "{value:?} ⇒ {:?}",
                err.kind
            );
            assert_eq!(err.context.stage, Stage::Bind);
        }
    }

    #[test]
    fn reusing_the_arena_does_not_grow_it() {
        // D8 纪律⑤：稳态零分配。第二趟起容量不该再长。
        let args = vec![
            DmValue::Int(1),
            DmValue::Text("达梦".into()),
            DmValue::Bytes(vec![9; 64]),
        ];

        let mut arena = ParamArena::new();
        let build = |arena: &mut ParamArena| {
            arena.clear();
            arena.arena.reserve(upper_bound(&args));
            for arg in &args {
                let (spec, buf_off) = encode_one(&mut arena.arena, arg).unwrap();
                let ind_off = align8(arena.arena.len());
                arena.arena.resize(ind_off + 8, 0);
                let bytes = spec.ind.to_ne_bytes();
                arena.arena[ind_off..ind_off + 8].copy_from_slice(&bytes);
                arena.metas.push(ParamMeta {
                    ctype: spec.ctype,
                    dtype: spec.dtype,
                    column_size: spec.column_size,
                    decimal_digits: spec.decimal_digits,
                    buf_off,
                    buf_len: spec.buf_len,
                    ind: spec.ind,
                    ind_off,
                });
            }
        };

        build(&mut arena);
        let capacity = arena.capacity();
        build(&mut arena);
        assert_eq!(arena.capacity(), capacity, "第二趟不该再分配");
        assert_eq!(arena.len(), 3);
        assert_eq!(arena.metas()[1].column_size, 2, "「达梦」是两个字符");
    }

    #[test]
    fn capacity_estimate_is_a_true_upper_bound() {
        // 上界估错的后果是写入那趟真的重分配，偏移与指针分家。所以这里钉住它。
        let samples = vec![
            DmValue::Null,
            DmValue::Bool(true),
            DmValue::BigInt(i64::MIN),
            DmValue::Double(1.0),
            DmValue::Decimal(rust_decimal::Decimal::new(-123456, 3)),
            DmValue::Text("中文abc\u{1D11E}".into()),
            DmValue::Bytes(vec![7; 300]),
            DmValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap()),
            DmValue::Timestamp(
                chrono::NaiveDate::from_ymd_opt(2024, 3, 5)
                    .unwrap()
                    .and_hms_micro_opt(1, 2, 3, 4)
                    .unwrap(),
            ),
        ];
        let claim = upper_bound(&samples);

        let mut out = Vec::with_capacity(claim);
        let capacity = out.capacity();
        let mut used = 0usize;
        for value in &samples {
            let _encoded = encode_one(&mut out, value).unwrap();
            let ind_off = align8(out.len());
            out.resize(ind_off + 8, 0);
            used = out.len();
        }
        assert!(used <= capacity, "用了 {used} 字节，只预留了 {capacity}");
        assert_eq!(out.capacity(), capacity, "预留够了就不该再分配");
    }
}
