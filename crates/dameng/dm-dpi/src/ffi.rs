//! 达梦 DPI（Dependency Programming Interface）的原始 FFI 绑定。
//!
//! 常量与原型逐条对自 `D:\dmdbms\include\DPI.h` / `DPIext.h` / `DPItypes.h`
//! （DM8 服务端 build `1-3-140-2024.05.27-229704-20093-ENT`，头文件随客户端分发）。
//! 只声明本项目用得到的那部分——DPI 一共导出 101 个函数，全量声明只会增加
//! 「签名抄错」的面积。
//!
//! # 这里为什么全是 `unsafe`
//!
//! FFI 调用本身就不安全：参数类型错了 C 侧不会拦，直接在调用点踩栈。所以本模块
//! 把「不安全」全部收在 `unsafe extern` 声明和 [`Api`] 的函数指针字段里，
//! 由上层的 [`crate::handle`] 封装成带 RAII 和诊断的安全接口。
//!
//! # 句柄宽度
//!
//! DPI 的句柄统一是 `void*`（`dhenv` / `dhcon` / `dhstmt` / `dhloblctr` 都是），
//! 只有句柄**类型**（`DSQL_HANDLE_*`）用 `sdint2` 区分。`slength` / `ulength`
//! 在 64 位平台上分别是 `sdint8` / `udint8`（`DPItypes.h` 的 `DM64` 分支），
//! 本 crate 只支持 64 位目标（`i64` / `u64` 直接对应）。

use std::ffi::c_char;
use std::ffi::c_void;
use std::sync::OnceLock;

use crate::error::LoadError;
use crate::sys::Lib;

/// DPI 句柄：`dhenv` / `dhcon` / `dhstmt` / `dhloblctr` 统一是 `void*`。
pub type Handle = *mut c_void;

/// `DPIRETURN`（`sdint2`）。
pub type Ret = i16;

/// `slength`（64 位下的 `sdint8`）。
pub type SLength = i64;

/// `ulength`（64 位下的 `udint8`）。
pub type ULength = u64;

#[allow(missing_docs)]
pub mod ret {
    use super::Ret;

    /// 成功。
    pub const SUCCESS: Ret = 0;
    /// 成功但有附带信息（例如被截断）。
    pub const SUCCESS_WITH_INFO: Ret = 1;
    /// 没有更多数据（`dpi_fetch` 走到结果集末尾）。
    pub const NO_DATA: Ret = 100;
    /// 失败。
    pub const ERROR: Ret = -1;
    /// 需要调用方用 `dpi_put_data` 送长数据。
    pub const NEED_DATA: Ret = 99;

    /// 是一类「成功」：`SUCCESS` 或 `SUCCESS_WITH_INFO`。
    #[must_use]
    pub fn is_success(code: Ret) -> bool {
        code == SUCCESS || code == SUCCESS_WITH_INFO
    }
}

#[allow(missing_docs)]
pub mod indicator {
    use super::SLength;

    /// 该列 / 该参数是 NULL。
    pub const NULL_DATA: SLength = -1;
    /// 长数据在执行期才开始送。
    pub const DATA_AT_EXEC: SLength = -2;
    /// 字符串以 NUL 结尾，长度未知。
    pub const NTS: SLength = -3;
}

#[allow(missing_docs)]
pub mod handle_type {
    use super::Ret;

    pub const ENV: Ret = 1;
    pub const CON: Ret = 2;
    pub const STMT: Ret = 3;
    pub const DESC: Ret = 4;
    pub const LOB_LOCATOR: Ret = 5;
}

#[allow(missing_docs)]
pub mod ctype {
    use super::Ret;

    /// 16 位有符号整数。**必须用它绑 `SMALLINT`**：拿 `SLONG`（4 字节）配 2 字节
    /// 缓冲，驱动会按 4 字节去读，读到相邻字节凑成的垃圾值 ⇒ `[-70012] Data out of
    /// range`（官方套件的 `type_primitives::ty_i16` 就是这么红的）。
    pub const SSHORT: Ret = 1;
    pub const USHORT: Ret = 2;
    pub const SLONG: Ret = 3;
    pub const ULONG: Ret = 4;
    pub const FLOAT: Ret = 5;
    pub const DOUBLE: Ret = 6;
    pub const BIT: Ret = 7;
    pub const STINYINT: Ret = 8;
    pub const UTINYINT: Ret = 9;
    pub const SBIGINT: Ret = 10;
    pub const UBIGINT: Ret = 11;
    pub const BINARY: Ret = 12;
    pub const DATE: Ret = 13;
    pub const TIME: Ret = 14;
    pub const TIMESTAMP: Ret = 15;
    /// 宽字符（UTF-16LE）：Windows 上的天然选择，且不受客户端字符集设置影响。
    pub const WCHAR: Ret = 1001;
    /// 本地字符集字节串，**不以 NUL 结尾**，长度由指示符给出。
    pub const CHAR: Ret = 1003;
    /// 本地字符集字节串，**以 NUL 结尾**（`DSQL_C_NCHAR = 0`）。
    ///
    /// 长列指针模式用的就是它（`dmPython/vlong.c:95`）。
    pub const NCHAR: Ret = 0;
    pub const LOB_HANDLE: Ret = 999;
}

#[allow(missing_docs)]
pub mod sqltype {
    use super::Ret;

    pub const CHAR: Ret = 1;
    pub const VARCHAR: Ret = 2;
    pub const BIT: Ret = 3;
    pub const TINYINT: Ret = 5;
    pub const SMALLINT: Ret = 6;
    pub const INT: Ret = 7;
    pub const BIGINT: Ret = 8;
    pub const DEC: Ret = 9;
    pub const FLOAT: Ret = 10;
    pub const DOUBLE: Ret = 11;
    pub const BLOB: Ret = 12;
    pub const BOOLEAN: Ret = 13;
    pub const DATE: Ret = 14;
    pub const TIME: Ret = 15;
    pub const TIMESTAMP: Ret = 16;
    pub const BINARY: Ret = 17;
    pub const VARBINARY: Ret = 18;
    pub const CLOB: Ret = 19;
    pub const RSET: Ret = 119;
    pub const BFILE: Ret = 1000;
}

#[allow(missing_docs)]
pub mod attr {
    /// 环境：客户端本地字符集（取值见 [`super::codeset`]）。
    pub const LOCAL_CODE: i32 = 12345;
    /// 环境：错误消息语言。
    pub const LANG_ID: i32 = 12346;
    /// 连接：自动提交。
    pub const AUTOCOMMIT: i32 = 102;
    /// 连接：登录超时（秒）。
    pub const LOGIN_TIMEOUT: i32 = 103;
    /// 连接：网络包大小。
    pub const PACKET_SIZE: i32 = 112;
    /// 连接：连接超时（秒）。
    pub const CONNECTION_TIMEOUT: i32 = 113;
    /// 连接：当前模式。**这就是 ODBC 层 `SCHEMA=` 关键字想做而做不到的事。**
    pub const CURRENT_SCHEMA: i32 = 12354;
    /// 连接：登录用户名（只读）。
    pub const LOGIN_USER: i32 = 12352;
    /// 连接：服务端版本（只读）。
    pub const SERVER_VERSION: i32 = 12400;
    /// 连接：应用名。
    pub const APP_NAME: i32 = 12357;
    /// 连接：连接是否已失效（0 = 仍可用）。探活读它，不发 SQL。
    pub const CONNECTION_DEAD: i32 = 1209;
    /// 语句：一次执行携带的参数组数（数组绑定，dmPython 的 `executemany` 用它）。
    pub const PARAMSET_SIZE: i32 = 22;
    /// 语句：一次 `dpi_fetch` 取多少行。
    pub const ROW_ARRAY_SIZE: i32 = 27;
}

#[allow(missing_docs)]
pub mod autocommit {
    /// 关闭自动提交。
    pub const OFF: i32 = 0;
    /// 打开自动提交（DPI 的默认值）。
    pub const ON: i32 = 1;
}

#[allow(missing_docs)]
pub mod param {
    use super::Ret;

    pub const INPUT: Ret = 1;
    pub const INPUT_OUTPUT: Ret = 2;
    pub const OUTPUT: Ret = 4;
}

#[allow(missing_docs)]
pub mod codeset {
    pub const UTF8: i32 = 1;
    pub const GBK: i32 = 2;
    pub const GB18030: i32 = 10;
}

/// DPI 导出的函数表。
///
/// 字段名就是导出符号名，解析时逐个 `GetProcAddress` / `dlsym`。
/// 调用这些字段仍然需要 `unsafe`：签名正确性由本文件保证，但句柄有效性得调用方保证。
pub struct Api {
    /// `dpi_module_init`：初始化 DPI 运行时（进程内调用一次即可）。
    pub dpi_module_init: unsafe extern "C" fn() -> Ret,
    /// 分配环境句柄。
    pub dpi_alloc_env: unsafe extern "C" fn(*mut Handle) -> Ret,
    /// 分配连接句柄。
    pub dpi_alloc_con: unsafe extern "C" fn(Handle, *mut Handle) -> Ret,
    /// 分配语句句柄。
    pub dpi_alloc_stmt: unsafe extern "C" fn(Handle, *mut Handle) -> Ret,
    /// 释放环境句柄。
    pub dpi_free_env: unsafe extern "C" fn(Handle) -> Ret,
    /// 释放连接句柄。
    pub dpi_free_con: unsafe extern "C" fn(Handle) -> Ret,
    /// 释放语句句柄。
    pub dpi_free_stmt: unsafe extern "C" fn(Handle) -> Ret,
    /// 设置环境属性。
    pub dpi_set_env_attr: unsafe extern "C" fn(Handle, i32, *mut c_void, i32) -> Ret,
    /// 设置连接属性。
    pub dpi_set_con_attr: unsafe extern "C" fn(Handle, i32, *mut c_void, i32) -> Ret,
    /// 设置语句属性（`PARAMSET_SIZE`、`ROW_ARRAY_SIZE` 等）。
    pub dpi_set_stmt_attr: unsafe extern "C" fn(Handle, i32, *mut c_void, i32) -> Ret,
    /// 读连接属性。
    pub dpi_get_con_attr: unsafe extern "C" fn(Handle, i32, *mut c_void, i32, *mut i32) -> Ret,
    /// 登录。
    pub dpi_login: unsafe extern "C" fn(Handle, *const c_char, *const c_char, *const c_char) -> Ret,
    /// 登出。
    pub dpi_logout: unsafe extern "C" fn(Handle) -> Ret,
    /// 提交。
    pub dpi_commit: unsafe extern "C" fn(Handle) -> Ret,
    /// 回滚。
    pub dpi_rollback: unsafe extern "C" fn(Handle) -> Ret,
    /// 预处理语句。
    pub dpi_prepare: unsafe extern "C" fn(Handle, *const c_char) -> Ret,
    /// 执行预处理 / 直接执行过的语句。
    pub dpi_exec: unsafe extern "C" fn(Handle) -> Ret,
    /// 直接执行一条 SQL。
    pub dpi_exec_direct: unsafe extern "C" fn(Handle, *const c_char) -> Ret,
    /// 关闭游标（释放结果集，语句本身还能复用）。
    pub dpi_close_cursor: unsafe extern "C" fn(Handle) -> Ret,
    /// 结果集列数。
    pub dpi_number_columns: unsafe extern "C" fn(Handle, *mut i16) -> Ret,
    /// 参数个数。
    pub dpi_number_params: unsafe extern "C" fn(Handle, *mut i16) -> Ret,
    /// 描述第 `iparam` 个参数（**1 起**）：服务端类型 / 精度 / 小数位 / 可空。
    ///
    /// `dmPython` 就是这么绑参数的：`var.c:327` 传给 `dpi_bind_param2` 的
    /// `sql_type`、`prec`、`scale` 全部来自 `dpi_desc_param`，不是自己猜。猜错的后果
    /// 实测有两种：文本列报 `[-70005] String truncated`，定长数值列报 `[-70012]
    /// Data out of range`。
    #[allow(clippy::type_complexity)]
    pub dpi_desc_param:
        unsafe extern "C" fn(Handle, u16, *mut i16, *mut u64, *mut i16, *mut i16) -> Ret,
    /// 影响行数。
    pub dpi_row_count: unsafe extern "C" fn(Handle, *mut i64) -> Ret,
    /// 描述第 `icol` 列（**1 起**）。
    #[allow(clippy::type_complexity)]
    pub dpi_desc_column: unsafe extern "C" fn(
        Handle,
        i16,
        *mut u8,
        i16,
        *mut i16,
        *mut i16,
        *mut u64,
        *mut i16,
        *mut i16,
    ) -> Ret,
    /// 绑定一个参数并同时给出实际长度数组（**1 起**）。
    ///
    /// 与 `dpi_bind_param` 的区别只是多一个 `act_len_ptr`。dmPython 只调这个版本
    /// （`dmPython/var.c:327`），数组绑定（`PARAMSET_SIZE > 1`）时两个指针都要指向
    /// 长度为 n 的数组；用单指针版本配数组绑定会把进程打崩（实测）。
    #[allow(clippy::type_complexity)]
    pub dpi_bind_param2: unsafe extern "C" fn(
        Handle,
        u16,
        i16,
        i16,
        i16,
        u64,
        i16,
        *mut c_void,
        i64,
        *mut i64,
        *mut i64,
    ) -> Ret,
    /// 绑定一个参数（**1 起**）。
    #[allow(clippy::type_complexity)]
    pub dpi_bind_param: unsafe extern "C" fn(
        Handle,
        u16,
        i16,
        i16,
        i16,
        u64,
        i16,
        *mut c_void,
        i64,
        *mut i64,
    ) -> Ret,
    /// 解除全部参数绑定。
    pub dpi_unbind_params: unsafe extern "C" fn(Handle) -> Ret,
    /// 取下一个「延迟输入」参数的数据指针（`DSQL_NEED_DATA` 循环的第一步）。
    pub dpi_param_data: unsafe extern "C" fn(Handle, *mut *mut c_void) -> Ret,
    /// 交付一段延迟输入数据。
    pub dpi_put_data: unsafe extern "C" fn(Handle, *mut c_void, SLength) -> Ret,
    /// 在语句上分配一个 LOB 定位器（读大对象用）。
    pub dpi_alloc_lob_locator: unsafe extern "C" fn(Handle, *mut Handle) -> Ret,
    /// 释放 LOB 定位器。
    pub dpi_free_lob_locator: unsafe extern "C" fn(Handle) -> Ret,
    /// 读 LOB 的长度。
    pub dpi_lob_get_length: unsafe extern "C" fn(Handle, *mut SLength) -> Ret,
    /// 按偏移读一段 LOB（`udint8` 偏移版，随机访问）。
    #[allow(clippy::type_complexity)]
    pub dpi_lob_read3: unsafe extern "C" fn(
        Handle,
        u64,
        i16,
        SLength,
        *mut c_void,
        SLength,
        *mut SLength,
        *mut SLength,
    ) -> Ret,
    /// 按偏移写一段 LOB（`udint8` 偏移版）。
    #[allow(clippy::type_complexity)]
    pub dpi_lob_write2: unsafe extern "C" fn(Handle, u64, i16, *mut c_void, u64, *mut u64) -> Ret,
    /// 取一行。
    pub dpi_fetch: unsafe extern "C" fn(Handle, *mut u64) -> Ret,
    /// 把一个结果列绑到调用方的缓冲区（**1 起**）。
    ///
    /// 长列（`CLOB`/`BLOB`）的「缓冲区里放指针」模式就走这里：缓冲只给 8 字节，
    /// 驱动把数据指针写进这 8 字节、把长度写进 `ind`
    /// （`dmPython/vlong.c:95-121`：`cType = DSQL_C_NCHAR` / `DSQL_C_BINARY`，
    /// `sizeof(sdint8)` 元素长度）。
    pub dpi_bind_col: unsafe extern "C" fn(Handle, u16, i16, *mut c_void, i64, *mut i64) -> Ret,
    /// 绑定结果列并同时给出实际长度数组（`dmPython` 用的版本）。
    #[allow(clippy::type_complexity)]
    pub dpi_bind_col2:
        unsafe extern "C" fn(Handle, u16, i16, *mut c_void, i64, *mut i64, *mut i64) -> Ret,
    /// 读第 `icol` 列（**1 起**）的数据。
    pub dpi_get_data: unsafe extern "C" fn(Handle, u16, i16, *mut c_void, i64, *mut i64) -> Ret,
    /// 带实际长度的读列（`dpi_get_data2`）。
    #[allow(clippy::type_complexity)]
    pub dpi_get_data2:
        unsafe extern "C" fn(Handle, u16, i16, *mut c_void, i64, *mut i64, *mut i64) -> Ret,
    /// 取第 `rec_num` 条（**1 起**）诊断记录。
    #[allow(clippy::type_complexity)]
    pub dpi_get_diag_rec:
        unsafe extern "C" fn(i16, Handle, i16, *mut i32, *mut u8, i16, *mut i16) -> Ret,
}

impl Api {
    /// 从已打开的共享库里解析出整张函数表。
    ///
    /// # Safety
    ///
    /// 调用方必须保证 `lib` 在返回的 `Api` 使用期间一直有效（本 crate 的做法是
    /// 永不卸载库，见 `crate::sys`）。
    unsafe fn resolve(lib: &Lib) -> Result<Self, String> {
        Ok(Self {
            // SAFETY: 每个字段的类型都在本文件里与 DPI.h 的原型逐参数对齐；
            // 符号缺失时 `function` 返回 Err 而不是空指针。
            dpi_module_init: unsafe { lib.function("dpi_module_init") }?,
            dpi_alloc_env: unsafe { lib.function("dpi_alloc_env") }?,
            dpi_alloc_con: unsafe { lib.function("dpi_alloc_con") }?,
            dpi_alloc_stmt: unsafe { lib.function("dpi_alloc_stmt") }?,
            dpi_free_env: unsafe { lib.function("dpi_free_env") }?,
            dpi_free_con: unsafe { lib.function("dpi_free_con") }?,
            dpi_free_stmt: unsafe { lib.function("dpi_free_stmt") }?,
            dpi_set_env_attr: unsafe { lib.function("dpi_set_env_attr") }?,
            dpi_set_con_attr: unsafe { lib.function("dpi_set_con_attr") }?,
            dpi_set_stmt_attr: unsafe { lib.function("dpi_set_stmt_attr") }?,
            dpi_get_con_attr: unsafe { lib.function("dpi_get_con_attr") }?,
            dpi_login: unsafe { lib.function("dpi_login") }?,
            dpi_logout: unsafe { lib.function("dpi_logout") }?,
            dpi_commit: unsafe { lib.function("dpi_commit") }?,
            dpi_rollback: unsafe { lib.function("dpi_rollback") }?,
            dpi_prepare: unsafe { lib.function("dpi_prepare") }?,
            dpi_exec: unsafe { lib.function("dpi_exec") }?,
            dpi_exec_direct: unsafe { lib.function("dpi_exec_direct") }?,
            dpi_close_cursor: unsafe { lib.function("dpi_close_cursor") }?,
            dpi_number_columns: unsafe { lib.function("dpi_number_columns") }?,
            dpi_number_params: unsafe { lib.function("dpi_number_params") }?,
            dpi_desc_param: unsafe { lib.function("dpi_desc_param") }?,
            dpi_row_count: unsafe { lib.function("dpi_row_count") }?,
            dpi_desc_column: unsafe { lib.function("dpi_desc_column") }?,
            dpi_bind_param: unsafe { lib.function("dpi_bind_param") }?,
            dpi_bind_param2: unsafe { lib.function("dpi_bind_param2") }?,
            dpi_unbind_params: unsafe { lib.function("dpi_unbind_params") }?,
            dpi_param_data: unsafe { lib.function("dpi_param_data") }?,
            dpi_put_data: unsafe { lib.function("dpi_put_data") }?,
            dpi_alloc_lob_locator: unsafe { lib.function("dpi_alloc_lob_locator") }?,
            dpi_free_lob_locator: unsafe { lib.function("dpi_free_lob_locator") }?,
            dpi_lob_get_length: unsafe { lib.function("dpi_lob_get_length") }?,
            dpi_lob_read3: unsafe { lib.function("dpi_lob_read3") }?,
            dpi_lob_write2: unsafe { lib.function("dpi_lob_write2") }?,
            dpi_fetch: unsafe { lib.function("dpi_fetch") }?,
            dpi_bind_col: unsafe { lib.function("dpi_bind_col") }?,
            dpi_bind_col2: unsafe { lib.function("dpi_bind_col2") }?,
            dpi_get_data: unsafe { lib.function("dpi_get_data") }?,
            dpi_get_data2: unsafe { lib.function("dpi_get_data2") }?,
            dpi_get_diag_rec: unsafe { lib.function("dpi_get_diag_rec") }?,
        })
    }
}

/// 共享库的默认文件名。
const LIB_FILE: &str = if cfg!(windows) {
    "dmdpi.dll"
} else {
    "libdmdpi.so"
};

/// 按优先级列出候选库路径。
///
/// 顺序很重要：显式配置永远第一（`DM_DPI_LIB`），其次是 `DM_HOME/bin`，
/// 然后是这台机器上实测过的安装位置，最后退回「让系统自己按名字找」
/// （Windows 会搜 PATH，Unix 会搜 `LD_LIBRARY_PATH` / `ld.so.cache`）。
fn candidates() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(explicit) = std::env::var_os("DM_DPI_LIB").filter(|v| !v.is_empty()) {
        out.push(explicit.to_string_lossy().into_owned());
    }
    if let Some(home) = std::env::var_os("DM_HOME").filter(|v| !v.is_empty()) {
        out.push(
            std::path::PathBuf::from(home)
                .join("bin")
                .join(LIB_FILE)
                .to_string_lossy()
                .into_owned(),
        );
    }
    if cfg!(windows) {
        out.push(format!(r"D:\dmdbms\bin\{LIB_FILE}"));
    } else {
        out.push(format!("/opt/dmdbms/bin/{LIB_FILE}"));
    }
    out.push(LIB_FILE.to_owned());
    out
}

/// 加载 DPI 共享库并解析函数表（进程内只做一次）。
///
/// # Errors
///
/// 所有候选路径都失败时返回 [`LoadError`]，里面带着每条路径各自的失败原因。
pub fn api() -> Result<&'static Api, LoadError> {
    static API: OnceLock<Result<Api, LoadError>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

fn load() -> Result<Api, LoadError> {
    let mut tried = Vec::new();
    for path in candidates() {
        match Lib::open(&path) {
            Ok(lib) => {
                // SAFETY: 库句柄永不卸载（`Lib` 没有 Drop），所以函数指针一直有效。
                match unsafe { Api::resolve(&lib) } {
                    Ok(api) => {
                        // 库句柄必须比函数指针活得久：放进永不析构的 static（Rust 不运行
                        // 静态项的 Drop），别把它卸载掉。
                        static LOADED: OnceLock<Lib> = OnceLock::new();
                        let _ = LOADED.set(lib);
                        return Ok(api);
                    }
                    Err(why) => tried.push((path, why)),
                }
            }
            Err(why) => tried.push((path, why)),
        }
    }
    Err(LoadError { tried })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_list_puts_plain_name_last() {
        let list = candidates();
        assert!(!list.is_empty());
        assert_eq!(list.last().map(String::as_str), Some(LIB_FILE));
    }

    #[test]
    fn success_predicate_covers_both_codes() {
        assert!(ret::is_success(ret::SUCCESS));
        assert!(ret::is_success(ret::SUCCESS_WITH_INFO));
        assert!(!ret::is_success(ret::NO_DATA));
        assert!(!ret::is_success(ret::ERROR));
    }
}
