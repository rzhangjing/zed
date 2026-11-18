//! 句柄封装：把 [`crate::ffi`] 的裸函数指针包成带 RAII 与诊断的安全接口。
//!
//! 三层结构，与 DPI 的句柄层次一一对应：
//!
//! * [`Dpi`] —— 进程级：加载共享库、`dpi_module_init`，并持有 [`ffi::Api`]；
//! * [`Env`] —— 环境：设客户端字符集这类**建连之前**必须就位的属性；
//! * [`Conn`] —— 连接：登录、提交/回滚、模式、语句；
//! * [`Stmt`] —— 语句：预处理、绑定、执行、取列描述、取行、取列值。
//!
//! 生命周期**不**用借用表达，而是用引用计数：`Stmt` 持有 `Arc<ConnGuard>`、
//! `Conn` 持有 `Arc<EnvGuard>`，因此子句柄天然比父句柄活得久（父句柄的释放推迟到
//! 最后一个子句柄消失），又不需要把生命周期参数传染到 `Backend::Stmt` 上
//! （那里要求 `type Stmt: Send + 'static`，见 `dm-core` 的 `Backend`）。
//!
//! 这一层只做「机制」，不做「策略」：不判字符集、不做类型映射、不缓存语句。
//! 那些属于上层的 `Backend` 实现。

use std::ffi::CString;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use crate::error::DpiError;
use crate::ffi::{
    self, Api, Handle, Ret, SLength, attr, codeset, ctype, handle_type, indicator, ret,
};

/// DPI 运行时。整个进程只需要一个，建议放在 `OnceLock` 里复用。
#[derive(Clone, Copy)]
pub struct Dpi {
    api: &'static Api,
}

impl Dpi {
    /// 加载 `dmdpi` 共享库并初始化运行时。
    ///
    /// 库的查找顺序见 [`ffi::api`]；没装达梦客户端时这里就会失败，而且错误里
    /// 会列出所有试过的路径。
    ///
    /// # Errors
    ///
    /// 共享库找不到、导出符号缺失、`dpi_module_init` 返回失败。
    pub fn open() -> Result<Self, DpiError> {
        let api = ffi::api().map_err(|e| DpiError::new(0, e.to_string()))?;
        // SAFETY: 无参数；`dpi_module_init` 允许被重复调用（内部做引用计数）。
        let code = unsafe { (api.dpi_module_init)() };
        if !ret::is_success(code) {
            return Err(DpiError::new(
                i32::from(code),
                "dpi_module_init 失败：DPI 运行时没有初始化成功",
            ));
        }
        Ok(Self { api })
    }

    /// 函数表。给上层实现 `Backend` 时直接用，避免重复解析。
    #[must_use]
    pub fn api(&self) -> &'static Api {
        self.api
    }

    /// 分配一个环境句柄。
    ///
    /// # Errors
    ///
    /// `dpi_alloc_env` 失败。
    pub fn env(&self) -> Result<Env, DpiError> {
        let mut raw: Handle = ptr::null_mut();
        // SAFETY: `raw` 是有效的出参地址。
        let code = unsafe { (self.api.dpi_alloc_env)(&raw mut raw) };
        check(self.api, handle_type::ENV, raw, code, "dpi_alloc_env")?;
        if raw.is_null() {
            return Err(DpiError::misuse("dpi_alloc_env 返回成功但句柄为空"));
        }
        Ok(Env {
            raw,
            api: self.api,
            guard: Arc::new(EnvGuard { raw, api: self.api }),
        })
    }
}

/// 环境句柄的**所有权**持有者：最后一个引用消失时才 `dpi_free_env`。
pub(crate) struct EnvGuard {
    raw: Handle,
    api: &'static Api,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: 这是最后一个引用（`Arc` 只在 `Env`/`ConnGuard` 里），之后不再使用。
        unsafe {
            (self.api.dpi_free_env)(self.raw);
        }
    }
}

// SAFETY: 与 `Env` 同理——句柄本身可在 `Mutex` 保护下跨线程移动，不能并发使用。
unsafe impl Send for EnvGuard {}
unsafe impl Sync for EnvGuard {}

/// 环境句柄。建连**之前**的属性（客户端字符集、错误消息语言）在这里设。
///
/// 句柄本身由 [`EnvGuard`] 持有，`Env` 只是它的句柄视图；克隆 `Env` 不会多开环境。
pub struct Env {
    raw: Handle,
    api: &'static Api,
    guard: Arc<EnvGuard>,
}

// SAFETY: `Env` 只含裸句柄与函数表指针，DPI 的句柄可以在线程间移动（不能并发使用，
// 但 `Backend` 契约本来就用 `Mutex` 串行化，见 `dm-core` 的 `Backend` 文档）。
unsafe impl Send for Env {}

impl Env {
    /// 设一个 `sdint4` 型环境属性。
    ///
    /// **数值型属性的传法是「把值本身当指针」且 `val_len = 0`**，不是传 `&value`。
    /// 依据是达梦随驱动发布的 dmPython 源码：`dmPython/Connection.c:754`
    /// `dpi_set_con_attr(self->hcon, *attr_id, (dpointer)numValue, val_len)`
    /// （`val_len` 默认 0）。按「传地址 + 长度 4」的写法，本机实测一律报
    /// `[-70023] 无效的参数值`。
    ///
    /// # Errors
    ///
    /// `dpi_set_env_attr` 失败。
    pub fn set_attr_i32(&self, attr_id: i32, value: i32) -> Result<(), DpiError> {
        // SAFETY: 数值型属性按 DPI 约定把值当指针传、长度 0；句柄由 `Dpi::env` 保证有效。
        let code = unsafe {
            (self.api.dpi_set_env_attr)(self.raw, attr_id, value as usize as *mut c_void, 0)
        };
        check(
            self.api,
            handle_type::ENV,
            self.raw,
            code,
            "dpi_set_env_attr",
        )
    }

    /// 直接设一个环境属性：原样把指针和长度交给驱动。
    ///
    /// 存在的意义是**探测**：DPI 各属性到底要 `sdint4` 还是字符串、`val_len` 该给
    /// 几，各版本不完全一致，只能实测（见 `examples/dpi_smoke.rs`）。
    ///
    /// # Safety
    ///
    /// `val` 必须在调用期间有效且与 `val_len` 匹配；类型错了驱动会按错的理解读。
    ///
    /// # Errors
    ///
    /// `dpi_set_env_attr` 失败。
    pub unsafe fn set_attr_raw(
        &self,
        attr_id: i32,
        val: *mut std::ffi::c_void,
        val_len: i32,
    ) -> Result<(), DpiError> {
        // SAFETY: 由调用方保证指针与长度一致（见本函数的 `# Safety`）。
        let code = unsafe { (self.api.dpi_set_env_attr)(self.raw, attr_id, val, val_len) };
        check(
            self.api,
            handle_type::ENV,
            self.raw,
            code,
            "dpi_set_env_attr",
        )
    }

    /// 设客户端字符集（见 [`codeset`]）。
    ///
    /// **必须在 `connect` 之前**调用：字符集在登录握手时确定，之后再设不生效。
    ///
    /// # Errors
    ///
    /// 属性设置失败。
    pub fn set_local_code(&self, charset: i32) -> Result<(), DpiError> {
        self.set_attr_i32(attr::LOCAL_CODE, charset)
    }

    /// 分配一个连接句柄（还没登录）。
    ///
    /// # Errors
    ///
    /// `dpi_alloc_con` 失败。
    pub fn connect(&self) -> Result<Conn, DpiError> {
        let mut raw: Handle = ptr::null_mut();
        // SAFETY: `raw` 是有效出参地址；环境句柄有效。
        let code = unsafe { (self.api.dpi_alloc_con)(self.raw, &raw mut raw) };
        check(self.api, handle_type::CON, raw, code, "dpi_alloc_con")?;
        if raw.is_null() {
            return Err(DpiError::misuse("dpi_alloc_con 返回成功但句柄为空"));
        }
        let guard = Arc::new(ConnGuard {
            raw,
            api: self.api,
            _env: Arc::clone(&self.guard),
        });
        Ok(Conn {
            raw,
            api: self.api,
            guard,
        })
    }
}

/// 连接句柄的**所有权**持有者：最后一个引用（含所有语句）消失时才 `dpi_free_con`。
pub(crate) struct ConnGuard {
    raw: Handle,
    api: &'static Api,
    /// 保证环境句柄比连接活得久。
    _env: Arc<EnvGuard>,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        // SAFETY: 这是最后一个引用，之后不再使用。
        unsafe {
            (self.api.dpi_free_con)(self.raw);
        }
    }
}

// SAFETY: 同 [`EnvGuard`]：句柄可在 `Mutex` 保护下跨线程移动，不能并发使用。
unsafe impl Send for ConnGuard {}
unsafe impl Sync for ConnGuard {}

/// 连接句柄。
pub struct Conn {
    raw: Handle,
    api: &'static Api,
    guard: Arc<ConnGuard>,
}

// SAFETY: 同 [`Env`]：句柄可在 `Mutex` 保护下跨线程移动。
unsafe impl Send for Conn {}

impl Conn {
    /// 登录。`server` 形如 `127.0.0.1:5236`（省略端口时用 DPI 默认的 5236）。
    ///
    /// # Errors
    ///
    /// 认证失败、网络不通、服务端拒绝。
    pub fn login(&self, server: &str, user: &str, password: &str) -> Result<(), DpiError> {
        let server = cstr(server)?;
        let user = cstr(user)?;
        let password = cstr(password)?;
        // SAFETY: 三个指针都指向 NUL 结尾的 C 字符串，调用期间一直存活。
        let code = unsafe {
            (self.api.dpi_login)(self.raw, server.as_ptr(), user.as_ptr(), password.as_ptr())
        };
        check(self.api, handle_type::CON, self.raw, code, "dpi_login")
    }

    /// 登出。
    ///
    /// # Errors
    ///
    /// `dpi_logout` 失败。
    pub fn logout(&self) -> Result<(), DpiError> {
        // SAFETY: 连接句柄有效。
        let code = unsafe { (self.api.dpi_logout)(self.raw) };
        check(self.api, handle_type::CON, self.raw, code, "dpi_logout")
    }

    /// 设一个 `sdint4` 型连接属性（自动提交、超时、包大小）。
    ///
    /// 数值型属性的传法与 [`Env::set_attr_i32`] 一致：**值本身当指针、`val_len = 0`**。
    /// dmPython 里 `DSQL_ATTR_AUTOCOMMIT` 等属性走的正是这条路径
    /// （`dmPython/Connection.c:1529`）。
    ///
    /// # Errors
    ///
    /// `dpi_set_con_attr` 失败。
    pub fn set_attr_i32(&self, attr_id: i32, value: i32) -> Result<(), DpiError> {
        // SAFETY: 数值型属性按 DPI 约定把值当指针传、长度 0；句柄有效。
        let code = unsafe {
            (self.api.dpi_set_con_attr)(self.raw, attr_id, value as usize as *mut c_void, 0)
        };
        check(
            self.api,
            handle_type::CON,
            self.raw,
            code,
            "dpi_set_con_attr",
        )
    }

    /// 设一个字符串型连接属性（当前模式、应用名）。
    ///
    /// # Errors
    ///
    /// 字符串含 NUL 字节，或 `dpi_set_con_attr` 失败。
    pub fn set_attr_str(&self, attr_id: i32, value: &str) -> Result<(), DpiError> {
        let value = cstr(value)?;
        let len = i32::try_from(value.as_bytes().len())
            .map_err(|_| DpiError::misuse("属性值过长（超过 i32 能表达的长度）"))?;
        // SAFETY: 指针指向 NUL 结尾字符串，`len` 与内容一致；句柄有效。
        let code = unsafe {
            (self.api.dpi_set_con_attr)(
                self.raw,
                attr_id,
                value.as_ptr().cast_mut().cast::<std::ffi::c_void>(),
                len,
            )
        };
        check(
            self.api,
            handle_type::CON,
            self.raw,
            code,
            "dpi_set_con_attr",
        )
    }

    /// 切换当前模式。
    ///
    /// **本机实测 `DSQL_ATTR_CURRENT_SCHEMA` 是只读属性**：`dpi_set_con_attr`
    /// 返回 `[-70042] 只读属性`（日志 `target/m39_dpi_smoke.log` E 段）。
    /// 切模式要用 SQL [`Conn::exec_direct`] 发 `SET SCHEMA "名字"`，
    /// 与 ODBC 后端 Round 35 的做法一致。这个函数留着是为了**将来驱动版本
    /// 支持时可切换**，当前调用会如实返回错误。
    ///
    /// # Errors
    ///
    /// 当前驱动版本一律失败（只读属性）。
    pub fn set_current_schema(&self, schema: &str) -> Result<(), DpiError> {
        self.set_attr_str(attr::CURRENT_SCHEMA, schema)
    }

    /// 读一个字符串型连接属性。
    ///
    /// # Errors
    ///
    /// `dpi_get_con_attr` 失败。
    pub fn get_attr_str(&self, attr_id: i32) -> Result<String, DpiError> {
        let mut buf = [0u8; 512];
        let mut len: i32 = 0;
        // SAFETY: 缓冲区容量与 `buf_len` 一致；`len` 是有效出参。
        let code = unsafe {
            (self.api.dpi_get_con_attr)(
                self.raw,
                attr_id,
                buf.as_mut_ptr().cast::<std::ffi::c_void>(),
                i32::try_from(buf.len()).unwrap_or(512),
                &raw mut len,
            )
        };
        check(
            self.api,
            handle_type::CON,
            self.raw,
            code,
            "dpi_get_con_attr",
        )?;
        let len = usize::try_from(len.max(0)).unwrap_or(0).min(buf.len());
        Ok(String::from_utf8_lossy(&buf[..len])
            .trim_end_matches('\0')
            .to_owned())
    }

    /// 读一个 `sdint4` 型连接属性（客户端字符集、自动提交）。
    ///
    /// 读的口径与 dmPython 一致：缓冲区给 `i32`、`buf_len = 0`，驱动按属性类型写回
    /// `i32`（`dmPython/Environment.c:186`
    /// `dpi_get_con_attr(con, DSQL_ATTR_LOCAL_CODE, &codeId, 0, &len)`）。
    ///
    /// # Errors
    ///
    /// `dpi_get_con_attr` 失败。
    pub fn get_attr_i32(&self, attr_id: i32) -> Result<i32, DpiError> {
        let mut value: i32 = 0;
        let mut len: i32 = 0;
        // SAFETY: 缓冲区是 4 字节 `i32`，`buf_len = 0` 是数值属性的读法；`len` 是有效出参。
        let code = unsafe {
            (self.api.dpi_get_con_attr)(
                self.raw,
                attr_id,
                (&raw mut value).cast::<std::ffi::c_void>(),
                0,
                &raw mut len,
            )
        };
        check(
            self.api,
            handle_type::CON,
            self.raw,
            code,
            "dpi_get_con_attr",
        )?;
        Ok(value)
    }

    /// 提交。
    ///
    /// # Errors
    ///
    /// `dpi_commit` 失败。
    pub fn commit(&self) -> Result<(), DpiError> {
        // SAFETY: 连接句柄有效。
        let code = unsafe { (self.api.dpi_commit)(self.raw) };
        check(self.api, handle_type::CON, self.raw, code, "dpi_commit")
    }

    /// 回滚。
    ///
    /// # Errors
    ///
    /// `dpi_rollback` 失败。
    pub fn rollback(&self) -> Result<(), DpiError> {
        // SAFETY: 连接句柄有效。
        let code = unsafe { (self.api.dpi_rollback)(self.raw) };
        check(self.api, handle_type::CON, self.raw, code, "dpi_rollback")
    }

    /// 分配一个语句句柄。
    ///
    /// # Errors
    ///
    /// `dpi_alloc_stmt` 失败。
    pub fn stmt(&self) -> Result<Stmt, DpiError> {
        let mut raw: Handle = ptr::null_mut();
        // SAFETY: `raw` 是有效出参；连接句柄有效。
        let code = unsafe { (self.api.dpi_alloc_stmt)(self.raw, &raw mut raw) };
        check(self.api, handle_type::STMT, raw, code, "dpi_alloc_stmt")?;
        if raw.is_null() {
            return Err(DpiError::misuse("dpi_alloc_stmt 返回成功但句柄为空"));
        }
        Ok(Stmt {
            raw,
            api: self.api,
            _guard: Arc::clone(&self.guard),
        })
    }

    /// 直接执行一条 SQL（内部临时分配一个语句句柄），返回影响行数。
    ///
    /// # Errors
    ///
    /// 语句分配失败或 SQL 执行失败。
    pub fn exec_direct(&self, sql: &str) -> Result<u64, DpiError> {
        let stmt = self.stmt()?;
        stmt.exec_direct(sql)?;
        stmt.affected_rows()
    }
}

/// 一条结果列的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDesc {
    /// 列序号，**1 起**（DPI 的口径）。
    pub index: u16,
    /// 列名（本地字符集，这里按 UTF-8 解）。
    pub name: String,
    /// [`crate::ffi::sqltype`] 里的服务端类型。
    pub sql_type: i16,
    /// 服务端声明的最大长度（字节）。
    pub size: u64,
    /// 小数位。
    pub decimals: i16,
    /// 是否允许 NULL。
    pub nullable: bool,
}

/// 一个**参数**在服务端眼中的类型与精度（`dpi_desc_param` 的出参）。
///
/// 绑参数时把 [`crate::ffi::sqltype`] 的取值换成这里的 `sql_type`，并带上 `precision`
/// / `scale`——自己按 Rust 类型猜的版本会在文本列上触发 `[-70005] String truncated`、
/// 在定长数值列上触发 `[-70012] Data out of range`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParamDesc {
    /// [`crate::ffi::sqltype`] 里的服务端类型。
    pub sql_type: i16,
    /// 精度（数值型是总位数，字符型是长度）。
    pub precision: u64,
    /// 小数位。
    pub scale: i16,
    /// 是否允许 NULL（`1` 允许）。
    pub nullable: i16,
}

/// DPI 的时间戳结构（与 `dpi_timestamp_t` 内存布局一致）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Timestamp {
    /// 年。
    pub year: i16,
    /// 月。
    pub month: u16,
    /// 日。
    pub day: u16,
    /// 时。
    pub hour: u16,
    /// 分。
    pub minute: u16,
    /// 秒。
    pub second: u16,
    /// 微秒（0..=999_999）。
    pub fraction: u32,
}

/// 语句句柄。
pub struct Stmt {
    raw: Handle,
    api: &'static Api,
    /// 保证连接句柄比语句活得久（语句释放要先于连接，见 CT-09）。
    _guard: Arc<ConnGuard>,
}

// SAFETY: 同 [`Env`]。
unsafe impl Send for Stmt {}

impl Stmt {
    /// 设一个 `sdint4` 型语句属性（`DSQL_ATTR_PARAMSET_SIZE`、`DSQL_ATTR_ROW_ARRAY_SIZE`、
    /// 查询超时等）。
    ///
    /// 传法与连接属性一致：**值本身当指针、`val_len = 0`**。dmPython 的
    /// `executemany` 就是先 `dpi_set_stmt_attr(stmt, DSQL_ATTR_PARAMSET_SIZE,
    /// (dpointer)n, 0)`（`dmPython/Cursor.c:1269`），再把参数缓冲区按 `n` 个元素
    /// 一起绑定（`dmPython/var.c:327` 的 `dpi_bind_param2`）。
    ///
    /// # Errors
    ///
    /// `dpi_set_stmt_attr` 失败。
    pub fn set_attr_i32(&self, attr_id: i32, value: i32) -> Result<(), DpiError> {
        // SAFETY: 数值型属性按 DPI 约定把值当指针传、长度 0；语句句柄有效。
        let code = unsafe {
            (self.api.dpi_set_stmt_attr)(self.raw, attr_id, value as usize as *mut c_void, 0)
        };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_set_stmt_attr",
        )
    }

    /// 直接执行一条 SQL（不需要预处理）。
    ///
    /// # Errors
    ///
    /// SQL 语法错误、权限不足、对象不存在等。
    pub fn exec_direct(&self, sql: &str) -> Result<(), DpiError> {
        let sql = cstr(sql)?;
        // SAFETY: SQL 是 NUL 结尾字符串；语句句柄有效。
        let code = unsafe { (self.api.dpi_exec_direct)(self.raw, sql.as_ptr()) };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_exec_direct",
        )
    }

    /// 预处理一条带参数的 SQL。
    ///
    /// # Errors
    ///
    /// SQL 语法错误、对象不存在等。
    pub fn prepare(&self, sql: &str) -> Result<(), DpiError> {
        let sql = cstr(sql)?;
        // SAFETY: SQL 是 NUL 结尾字符串；语句句柄有效。
        let code = unsafe { (self.api.dpi_prepare)(self.raw, sql.as_ptr()) };
        check(self.api, handle_type::STMT, self.raw, code, "dpi_prepare")
    }

    /// 执行已预处理的语句。
    ///
    /// # Errors
    ///
    /// 执行失败（约束冲突、类型不匹配等）。
    pub fn exec(&self) -> Result<(), DpiError> {
        self.exec_deferred().map(|_| ())
    }

    /// 执行已预处理的语句，并报告驱动是否在等「延迟输入」数据。
    ///
    /// 返回 `Ok(true)` 表示参数里有 `DSQL_DATA_AT_EXEC`（大对象流式写），
    /// 此时要循环 [`Self::param_data`] + [`Self::put_data`] 直到它返回 `Ok(false)`。
    /// 返回 `Ok(false)` 表示语句已经执行完毕。
    ///
    /// # Errors
    ///
    /// 执行失败（约束冲突、类型不匹配等）。
    pub fn exec_deferred(&self) -> Result<bool, DpiError> {
        // SAFETY: 语句句柄有效。
        let code = unsafe { (self.api.dpi_exec)(self.raw) };
        if code == ret::NEED_DATA {
            return Ok(true);
        }
        check(self.api, handle_type::STMT, self.raw, code, "dpi_exec")?;
        Ok(false)
    }

    /// 关闭游标。语句句柄仍可复用（重新 `prepare` 或 `exec_direct`）。
    pub fn close_cursor(&self) {
        // SAFETY: 语句句柄有效；关闭游标失败没有补救动作，也不影响后续复用。
        unsafe {
            (self.api.dpi_close_cursor)(self.raw);
        }
    }

    /// 结果集的列数。
    ///
    /// # Errors
    ///
    /// `dpi_number_columns` 失败。
    pub fn column_count(&self) -> Result<u16, DpiError> {
        let mut count: i16 = 0;
        // SAFETY: `count` 是有效出参；语句句柄有效。
        let code = unsafe { (self.api.dpi_number_columns)(self.raw, &raw mut count) };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_number_columns",
        )?;
        Ok(u16::try_from(count.max(0)).unwrap_or(0))
    }

    /// 参数个数。
    ///
    /// # Errors
    ///
    /// `dpi_number_params` 失败。
    pub fn param_count(&self) -> Result<u16, DpiError> {
        let mut count: i16 = 0;
        // SAFETY: `count` 是有效出参；语句句柄有效。
        let code = unsafe { (self.api.dpi_number_params)(self.raw, &raw mut count) };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_number_params",
        )?;
        Ok(u16::try_from(count.max(0)).unwrap_or(0))
    }

    /// 第 `index` 个参数（**1 起**）在服务端眼中的类型与精度。
    ///
    /// 绑参数时要用这里的 `sql_type` / `precision` / `scale`，而不是自己按 Rust 类型
    /// 猜一个（`dmPython` 也是这么做的：`var.c:327` 的三个实参全部来自
    /// `dpi_desc_param`）。实测差别：文本列猜 `VARCHAR` 且精度给 0 会得到
    /// `[-70005] String truncated`；定长数值列精度给 0 会得到 `[-70012] Data out of
    /// range`。
    ///
    /// # Errors
    ///
    /// `dpi_desc_param` 失败（例如语句还没 prepare、或序号越界）。
    pub fn param_desc(&self, index: u16) -> Result<ParamDesc, DpiError> {
        let mut sql_type: i16 = 0;
        let mut precision: u64 = 0;
        let mut scale: i16 = 0;
        let mut nullable: i16 = 0;
        // SAFETY: 四个出参都是本栈帧上的有效变量；语句句柄有效。
        let code = unsafe {
            (self.api.dpi_desc_param)(
                self.raw,
                index,
                &raw mut sql_type,
                &raw mut precision,
                &raw mut scale,
                &raw mut nullable,
            )
        };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_desc_param",
        )?;
        Ok(ParamDesc {
            sql_type,
            precision,
            scale,
            nullable,
        })
    }

    /// 上一条语句影响的行数（DML）。    ///
    /// # Errors
    ///
    /// `dpi_row_count` 失败。
    pub fn affected_rows(&self) -> Result<u64, DpiError> {
        let mut rows: i64 = 0;
        // SAFETY: `rows` 是有效出参；语句句柄有效。
        let code = unsafe { (self.api.dpi_row_count)(self.raw, &raw mut rows) };
        check(self.api, handle_type::STMT, self.raw, code, "dpi_row_count")?;
        Ok(u64::try_from(rows.max(0)).unwrap_or(0))
    }

    /// 绑定一个参数并给出实际长度数组（dmPython 用的就是这个版本）。
    ///
    /// 数组绑定（`DSQL_ATTR_PARAMSET_SIZE = n > 1`）时 `buf` 要装下 n 个元素、
    /// `ind` / `act_len` 都要指向长度为 n 的数组：单指针的 [`Self::bind_param`]
    /// 在 `n > 1` 时会把进程打崩（本机实测，`examples/dpi_probe2.rs` F 段）。
    ///
    /// # Safety
    ///
    /// 同 [`Self::bind_param`]：指针都被存进驱动，必须活到 `exec` 之后；
    /// `act_len` 也必须是 n 个 `slength` 的可写数组。
    ///
    /// # Errors
    ///
    /// `dpi_bind_param2` 失败（参数下标越界、类型不兼容等）。
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn bind_param2(
        &self,
        index: u16,
        param_type: i16,
        value_ctype: i16,
        dtype: i16,
        precision: u64,
        scale: i16,
        buf: *mut std::ffi::c_void,
        buf_len: SLength,
        ind: *mut SLength,
        act_len: *mut SLength,
    ) -> Result<(), DpiError> {
        // SAFETY: 由调用方保证指针与类型/长度一致（见本函数的 `# Safety`）。
        let code = unsafe {
            (self.api.dpi_bind_param2)(
                self.raw,
                index,
                param_type,
                value_ctype,
                dtype,
                precision,
                scale,
                buf,
                buf_len,
                ind,
                act_len,
            )
        };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_bind_param2",
        )
    }

    /// 绑定一个参数。
    ///
    /// 参数下标 `index` **1 起**，与 DPI 的口径一致。
    ///
    /// # Safety
    ///
    /// 两个裸指针都会被**存进驱动**，直到 `exec` 返回前都可能被读：
    ///
    /// * `buf` 至少要能容纳 `buf_len` 字节（NULL 参数可以给空指针 + 长度 0）；
    /// * `ind` 指向的指示符必须活到 `exec` 之后（非 NULL 时是字节数，
    ///   [`indicator::NULL_DATA`] 表示这个参数是 NULL）。
    ///
    /// 调用方还要保证 `ctype` / `dtype` 与 `buf` 里真实的内存布局一致——写错了
    /// 驱动会照着错的理解去读，后果是脏数据或崩溃，而不是一个错误码。
    ///
    /// # Errors
    ///
    /// `dpi_bind_param` 失败（参数下标越界、类型不兼容等）。
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn bind_param(
        &self,
        index: u16,
        param_type: i16,
        value_ctype: i16,
        dtype: i16,
        precision: u64,
        scale: i16,
        buf: *mut std::ffi::c_void,
        buf_len: SLength,
        ind: *mut SLength,
    ) -> Result<(), DpiError> {
        // SAFETY: 由调用方保证指针与类型一致（见本函数的 `# Safety`）。
        let code = unsafe {
            (self.api.dpi_bind_param)(
                self.raw,
                index,
                param_type,
                value_ctype,
                dtype,
                precision,
                scale,
                buf,
                buf_len,
                ind,
            )
        };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_bind_param",
        )
    }

    /// 读一个 LOB 定位器指向的数据总长度。
    ///
    /// 定位器来自「把结果列按 [`ctype::LOB_HANDLE`] 绑进 8 字节槽后 `fetch`」，
    /// 槽里就是定位器句柄（`DSQL_C_LOB_HANDLE`）。有了定位器就能**按偏移任意读**，
    /// 不受 `dpi_get_data` 单次 64 KB 和字符边界截断的限制。
    ///
    /// # Safety
    ///
    /// `locator` 必须是本语句 `fetch` 之后从绑定槽里取出的定位器句柄，且语句仍然存活。
    ///
    /// # Errors
    ///
    /// `dpi_lob_get_length` 失败。
    pub unsafe fn lob_length(&self, locator: Handle) -> Result<SLength, DpiError> {
        let mut len: SLength = 0;
        // SAFETY: `len` 是有效出参；定位器由调用方保证有效。
        let code = unsafe { (self.api.dpi_lob_get_length)(locator, &raw mut len) };
        check(
            self.api,
            handle_type::LOB_LOCATOR,
            locator,
            code,
            "dpi_lob_get_length",
        )?;
        Ok(len)
    }

    /// 从 LOB 定位器的 `offset`（0 起）读最多 `buf.len()` 字节。
    ///
    /// `value_ctype` 给 [`ctype::CHAR`]（文本，按客户端字符集转换）或
    /// [`ctype::BINARY`]（二进制原始字节）。返回实际读到的字节数。
    ///
    /// # Safety
    ///
    /// 同 [`Self::lob_length`]；`buf` 必须至少 `buf.len()` 字节。
    ///
    /// # Errors
    ///
    /// `dpi_lob_read3` 失败。
    pub unsafe fn lob_read(
        &self,
        locator: Handle,
        offset: u64,
        value_ctype: i16,
        buf: &mut [u8],
    ) -> Result<usize, DpiError> {
        let mut got: SLength = 0;
        let mut got_bytes: SLength = 0;
        // SAFETY: 缓冲区容量与传进去的长度一致；定位器由调用方保证有效。
        let code = unsafe {
            (self.api.dpi_lob_read3)(
                locator,
                offset,
                value_ctype,
                SLength::try_from(buf.len()).unwrap_or(0),
                buf.as_mut_ptr().cast(),
                SLength::try_from(buf.len()).unwrap_or(0),
                &raw mut got,
                &raw mut got_bytes,
            )
        };
        check(
            self.api,
            handle_type::LOB_LOCATOR,
            locator,
            code,
            "dpi_lob_read3",
        )?;
        Ok(usize::try_from(got_bytes.max(0))
            .unwrap_or(0)
            .min(buf.len()))
    }

    /// 把一个结果列绑到调用方的缓冲区（`index` **1 起**）。
    ///
    /// 长列（`CLOB`/`BLOB`）走「缓冲区里放指针」模式：`value_ctype` 给
    /// [`ctype::NCHAR`]（文本）或 [`ctype::BINARY`]（二进制），`buf` 只给 8 字节。
    /// `dpi_fetch` 之后缓冲里是**数据指针**，`ind` 里是**字节长度**
    /// （`dmPython/vlong.c:95-121`、`:219-225`：`cType = DSQL_C_NCHAR`、
    /// 元素长度 `sizeof(sdint8)`）。值短到能塞进 8 字节时 `ind` 就是真实长度，
    /// 缓冲里直接是数据本身。
    ///
    /// # Safety
    ///
    /// 三个指针都会被存进驱动：`ind` 要活到下一次 `fetch` 之后；`buf` 指向的
    /// 指针由驱动写入，解引用前必须自己确认长度可信。
    ///
    /// # Errors
    ///
    /// `dpi_bind_col` 失败（列号越界等）。
    pub unsafe fn bind_col(
        &self,
        index: u16,
        value_ctype: i16,
        buf: *mut std::ffi::c_void,
        buf_len: SLength,
        ind: *mut SLength,
    ) -> Result<(), DpiError> {
        // SAFETY: 由调用方保证指针与类型一致（见本函数的 `# Safety`）。
        let code =
            unsafe { (self.api.dpi_bind_col)(self.raw, index, value_ctype, buf, buf_len, ind) };
        check(self.api, handle_type::STMT, self.raw, code, "dpi_bind_col")
    }

    /// 同 [`Self::bind_col`]，但多给一个实际长度数组（`dmPython` 用的版本）。
    ///
    /// # Safety
    ///
    /// 同 [`Self::bind_col`]；`act_len` 也要活到下一次 `fetch` 之后。
    ///
    /// # Errors
    ///
    /// `dpi_bind_col2` 失败（列号越界等）。
    pub unsafe fn bind_col2(
        &self,
        index: u16,
        value_ctype: i16,
        buf: *mut std::ffi::c_void,
        buf_len: SLength,
        ind: *mut SLength,
        act_len: *mut SLength,
    ) -> Result<(), DpiError> {
        // SAFETY: 由调用方保证指针与类型一致（见本函数的 `# Safety`）。
        let code = unsafe {
            (self.api.dpi_bind_col2)(self.raw, index, value_ctype, buf, buf_len, ind, act_len)
        };
        check(self.api, handle_type::STMT, self.raw, code, "dpi_bind_col2")
    }

    /// 解除全部参数绑定。语句复用前必须调，否则上一轮的值会被带到下一轮。
    ///
    /// # Errors
    ///
    /// `dpi_unbind_params` 失败。
    pub fn unbind_params(&self) -> Result<(), DpiError> {
        // SAFETY: 语句句柄有效。
        let code = unsafe { (self.api.dpi_unbind_params)(self.raw) };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_unbind_params",
        )
    }

    /// 取全部列描述。
    ///
    /// # Errors
    ///
    /// 列数或某一列的 `dpi_desc_column` 失败。
    pub fn columns(&self) -> Result<Vec<ColumnDesc>, DpiError> {
        let count = self.column_count()?;
        let mut out = Vec::with_capacity(usize::from(count));
        for index in 1..=count {
            out.push(self.column(index)?);
        }
        Ok(out)
    }

    /// 取第 `index` 列（**1 起**）的描述。
    ///
    /// # Errors
    ///
    /// `dpi_desc_column` 失败。
    pub fn column(&self, index: u16) -> Result<ColumnDesc, DpiError> {
        let mut name = [0u8; 512];
        let mut name_len: i16 = 0;
        let mut sql_type: i16 = 0;
        let mut size: u64 = 0;
        let mut decimals: i16 = 0;
        let mut nullable: i16 = 0;
        // SAFETY: 所有出参都是本栈帧内的有效地址；缓冲区容量与 `name_buf_len` 一致。
        let code = unsafe {
            (self.api.dpi_desc_column)(
                self.raw,
                i16::try_from(index).unwrap_or(i16::MAX),
                name.as_mut_ptr(),
                i16::try_from(name.len()).unwrap_or(512),
                &raw mut name_len,
                &raw mut sql_type,
                &raw mut size,
                &raw mut decimals,
                &raw mut nullable,
            )
        };
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_desc_column",
        )?;
        let len = usize::try_from(name_len.max(0))
            .unwrap_or(0)
            .min(name.len());
        Ok(ColumnDesc {
            index,
            name: String::from_utf8_lossy(&name[..len]).into_owned(),
            sql_type,
            size,
            decimals,
            nullable: nullable != 0,
        })
    }

    /// 取下一行。返回 `false` 表示结果集已取完。
    ///
    /// # Errors
    ///
    /// `dpi_fetch` 报错（例如结果集已被关闭）。
    pub fn fetch(&self) -> Result<bool, DpiError> {
        let mut rows: u64 = 0;
        // SAFETY: `rows` 是有效出参；语句句柄有效。
        let code = unsafe { (self.api.dpi_fetch)(self.raw, &raw mut rows) };
        if ret::is_success(code) {
            Ok(true)
        } else if code == ret::NO_DATA {
            Ok(false)
        } else {
            Err(diag(
                self.api,
                handle_type::STMT,
                self.raw,
                code,
                "dpi_fetch",
            ))
        }
    }

    /// 以 `i64` 读第 `index` 列（**1 起**）。NULL 返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn column_i64(&self, index: u16) -> Result<Option<i64>, DpiError> {
        let mut value: i64 = 0;
        let ind = self.read_fixed(index, ctype::SBIGINT, (&raw mut value).cast(), 8)?;
        Ok(ind.map(|_| value))
    }

    /// 以 `f64` 读第 `index` 列。NULL 返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn column_f64(&self, index: u16) -> Result<Option<f64>, DpiError> {
        let mut value: f64 = 0.0;
        let ind = self.read_fixed(index, ctype::DOUBLE, (&raw mut value).cast(), 8)?;
        Ok(ind.map(|_| value))
    }

    /// 以 **UTF-16** 读第 `index` 列。NULL 返回 `Ok(None)`。
    ///
    /// 这是文本的**默认**读法。为什么不用窄字符（`DSQL_C_CHAR`）：
    /// 本机实测（`examples/dpi_smoke.rs`，日志 `target/m39_dpi_smoke.log`）表明
    /// 数据库字符集是 GB18030，而 DPI 的窄字节路径**不做转换**——把 UTF-8 字节
    /// 直接当 GB18030 存进去，往返字节虽然一致，服务端语义已经错了
    /// （`lengthb()` 与宽字符读回来都对不上）。走 `DSQL_C_WCHAR` 则由驱动按
    /// 客户端/服务端字符集做正解转换，与 ODBC 后端的行为一致。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn column_text(&self, index: u16) -> Result<Option<String>, DpiError> {
        let mut units = vec![0u16; 32 * 1024];
        let Some(bytes) = self.read_wide(index, &mut units)? else {
            return Ok(None);
        };
        Ok(Some(String::from_utf16_lossy(&units[..bytes / 2])))
    }

    /// 以本地字符集的十六进制字节读文本，**只用于诊断**（字符集设错时看驱动吐了什么）。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn column_text_local(&self, index: u16) -> Result<Option<String>, DpiError> {
        Ok(self
            .column_bytes(index)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// 以本地字符集的**原始字节**读第 `index` 列。NULL 返回 `Ok(None)`。
    ///
    /// 与 [`Self::column_text`] 的区别是**不走 UTF-16**——字符集设错时，
    /// 这里能看出驱动到底吐了什么字节（探针 `examples/dpi_smoke.rs` 就靠它）。
    /// 二进制列（`VARBINARY`/`BLOB` 前缀）请走 [`Self::column_binary`]。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn column_bytes(&self, index: u16) -> Result<Option<Vec<u8>>, DpiError> {
        let mut buf = vec![0u8; 64 * 1024];
        let Some(len) = self.read_variable(index, ctype::CHAR, &mut buf)? else {
            return Ok(None);
        };
        Ok(Some(buf[..len].to_vec()))
    }

    /// 以 `DSQL_C_BINARY` 读二进制字段（`VARBINARY` 之类，不含大对象）。NULL 返回 `Ok(None)`。
    ///
    /// 大对象（`BLOB`/`CLOB`）要用定位器 `dpi_lob_read*`，不走这里。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn column_binary(&self, index: u16) -> Result<Option<Vec<u8>>, DpiError> {
        let mut buf = vec![0u8; 64 * 1024];
        let Some(len) = self.read_variable(index, ctype::BINARY, &mut buf)? else {
            return Ok(None);
        };
        Ok(Some(buf[..len].to_vec()))
    }

    /// 以 UTF-16 读第 `index` 列，返回**字节数**（`units` 前 `bytes / 2` 个单元有效）。
    ///
    /// 宽字符路径由驱动按客户端/服务端字符集做转换，不依赖本地代码页。
    fn read_wide(&self, index: u16, units: &mut [u16]) -> Result<Option<usize>, DpiError> {
        let buf_len = units.len() * 2;
        let ind = self.read_fixed(index, ctype::WCHAR, units.as_mut_ptr().cast(), buf_len)?;
        Ok(ind.map(|bytes| usize::try_from(bytes.max(0)).unwrap_or(0).min(buf_len)))
    }

    /// 用 NULL 缓冲问第 `index` 列的**总长度**（不消耗数据）。
    ///
    /// 与 ODBC 的 `SQLGetData(..., NULL, 0, &len)` 同义：驱动只说长度。有了它就能
    /// 一次分配够大的缓冲、一次读完，避开 `dpi_get_data` 单次 64 KB 的分块与分块
    /// 边界的字符截断（`examples/dpi_lob_probe.rs` C/G 段实测过那个坑）。
    ///
    /// 返回 `Ok(None)` 表示该列是 NULL。
    ///
    /// # Errors
    ///
    /// 列不存在或 `dpi_get_data` 失败。
    pub fn column_length(&self, index: u16, value_ctype: i16) -> Result<Option<SLength>, DpiError> {
        let mut ind: SLength = 0;
        // SAFETY: 传 NULL 缓冲与 0 长度是「只要长度」的约定用法；句柄有效。
        let code = unsafe {
            (self.api.dpi_get_data)(
                self.raw,
                index,
                value_ctype,
                std::ptr::null_mut(),
                0,
                &raw mut ind,
            )
        };
        if ind == indicator::NULL_DATA {
            return Ok(None);
        }
        if code == ret::NO_DATA {
            return Ok(Some(0));
        }
        check(
            self.api,
            handle_type::STMT,
            self.raw,
            code,
            "dpi_get_data(问长度)",
        )?;
        Ok(Some(ind))
    }

    /// 以调用方指定的 C 类型把第 `index` 列读进 `buf`，返回写入的字节数。
    ///
    /// 这是 [`Self::column_text`] 一类便捷方法的通用版本：把 `value_ctype` 交给
    /// 调用方决定，方便先探清楚某个类型该怎么读（`examples/dpi_type_probe.rs`）。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn read_raw(
        &self,
        index: u16,
        value_ctype: i16,
        buf: &mut [u8],
    ) -> Result<Option<usize>, DpiError> {
        self.read_variable(index, value_ctype, buf)
    }

    /// 分块读变长类型：一次最多把 `buf.len()` 字节读进 `buf`，直到 [`ReadChunk::End`]。
    ///
    /// 本机实测（`examples/dpi_lob_probe.rs` C 段）：`dpi_get_data` 单次最多返回
    /// 65535/65536 字节，更大的 `CLOB`/`BLOB` 必须反复调用同一列才能读全。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn read_chunk(
        &self,
        index: u16,
        value_ctype: i16,
        buf: &mut [u8],
    ) -> Result<ReadChunk, DpiError> {
        let mut ind: SLength = 0;
        // SAFETY: 缓冲区容量与字节长度一致；句柄有效。
        let code = unsafe {
            (self.api.dpi_get_data)(
                self.raw,
                index,
                value_ctype,
                buf.as_mut_ptr().cast(),
                SLength::try_from(buf.len()).unwrap_or(0),
                &raw mut ind,
            )
        };
        if code == ret::NO_DATA {
            return Ok(ReadChunk::End);
        }
        if ind == indicator::NULL_DATA {
            return Ok(ReadChunk::Null);
        }
        check(self.api, handle_type::STMT, self.raw, code, "dpi_get_data")?;
        let len = usize::try_from(ind.max(0)).unwrap_or(0).min(buf.len());
        if len == 0 {
            Ok(ReadChunk::End)
        } else {
            Ok(ReadChunk::Data(len))
        }
    }

    /// 同 [`Self::read_chunk`]，但走 `dpi_get_data2`（多一个实际长度出参）。
    ///
    /// 本机实测 `dpi_get_data` 在 64 KB 边界会把多字节字符截断并塞一个 NUL
    /// （`examples/dpi_lob_probe.rs` C 段），这里试另一条路径。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data2` 失败。
    pub fn read_chunk2(
        &self,
        index: u16,
        value_ctype: i16,
        buf: &mut [u8],
    ) -> Result<ReadChunk, DpiError> {
        let mut ind: SLength = 0;
        let mut act: SLength = 0;
        // SAFETY: 缓冲区容量与字节长度一致；句柄有效。
        let code = unsafe {
            (self.api.dpi_get_data2)(
                self.raw,
                index,
                value_ctype,
                buf.as_mut_ptr().cast(),
                SLength::try_from(buf.len()).unwrap_or(0),
                &raw mut ind,
                &raw mut act,
            )
        };
        if code == ret::NO_DATA {
            return Ok(ReadChunk::End);
        }
        if ind == indicator::NULL_DATA {
            return Ok(ReadChunk::Null);
        }
        check(self.api, handle_type::STMT, self.raw, code, "dpi_get_data2")?;
        let len = usize::try_from(ind.max(0)).unwrap_or(0).min(buf.len());
        if len == 0 {
            Ok(ReadChunk::End)
        } else {
            Ok(ReadChunk::Data(len))
        }
    }

    /// 以「缓冲区里放指针」的方式读第 `index` 列，返回 `(指示符, 缓冲区 8 字节原值)`。
    /// 达梦的大对象列（`CLOB`/`BLOB`）与 `dmPython` 的 `LONG_STRING`/`LONG_BINARY`
    /// 用的是这一套：绑定/取值缓冲只需 8 字节（`sizeof(sdint8)`），驱动把**数据指针**
    /// 写进这 8 字节、把**数据长度**写进指示符
    /// （`dmPython/vlong.c:165`、`:219-225`）。指示符大于 8 就说明走的是指针模式。
    ///
    /// # Safety
    ///
    /// 返回的指针属于驱动的语句缓冲，只在**本行还没被下一条 `dpi_fetch` 覆盖**期间有效；
    /// 解引用它之前必须自己确认指示符长度可信、且语句句柄仍然存活。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn read_pointer(
        &self,
        index: u16,
        value_ctype: i16,
    ) -> Result<Option<(SLength, [u8; 8])>, DpiError> {
        let mut slot = [0u8; 8];
        let Some(ind) = self.read_variable(index, value_ctype, &mut slot)? else {
            return Ok(None);
        };
        Ok(Some((SLength::try_from(ind).unwrap_or(0), slot)))
    }

    /// 取下一个「延迟输入」参数的数据指针。
    ///
    /// 返回 `Some(dst)` 表示驱动还要数据（`DSQL_NEED_DATA`），`dst` 是它要填的那个
    /// 参数缓冲的地址，此时用 [`Self::put_data`] 交付；返回 `None`（`DSQL_SUCCESS`）
    /// 表示整条语句已经执行完毕。
    ///
    /// 每个 `DSQL_DATA_AT_EXEC` 参数只会要**一次**：把该参数的完整数据一次性交完
    /// （`dmPython` 就是这么做的），不要自己切块——切块会被驱动当成下一个参数。
    ///
    /// # Errors
    ///
    /// `dpi_param_data` 返回了非 `NEED_DATA`/`SUCCESS` 的码。
    pub fn param_data(&self) -> Result<Option<*mut std::ffi::c_void>, DpiError> {
        let mut dst: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `dst` 是有效出参；语句句柄有效。
        let code = unsafe { (self.api.dpi_param_data)(self.raw, &raw mut dst) };
        if code == ret::NEED_DATA {
            Ok(Some(dst))
        } else if ret::is_success(code) {
            Ok(None)
        } else {
            Err(diag(
                self.api,
                handle_type::STMT,
                self.raw,
                code,
                "dpi_param_data",
            ))
        }
    }

    /// 交付一段延迟输入数据（一次可以给任意长度，`dmPython` 就是整块给）。
    ///
    /// # Errors
    ///
    /// `dpi_put_data` 失败。
    pub fn put_data(&self, bytes: &[u8]) -> Result<(), DpiError> {
        // SAFETY: `bytes` 在调用期间有效；长度与切片一致。
        let code = unsafe {
            (self.api.dpi_put_data)(
                self.raw,
                bytes.as_ptr().cast_mut().cast::<std::ffi::c_void>(),
                SLength::try_from(bytes.len()).unwrap_or(0),
            )
        };
        check(self.api, handle_type::STMT, self.raw, code, "dpi_put_data")
    }

    /// 以 `DSQL_C_TIMESTAMP` 读第 `index` 列。NULL 返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// 列不存在、`dpi_get_data` 失败。
    pub fn column_timestamp(&self, index: u16) -> Result<Option<Timestamp>, DpiError> {
        let mut value = Timestamp::default();
        let size = std::mem::size_of::<Timestamp>();
        let ind = self.read_fixed(index, ctype::TIMESTAMP, (&raw mut value).cast(), size)?;
        Ok(ind.map(|_| value))
    }

    /// 读定长类型。返回 `None` 表示该列是 NULL。
    fn read_fixed(
        &self,
        index: u16,
        value_ctype: i16,
        buf: *mut std::ffi::c_void,
        buf_len: usize,
    ) -> Result<Option<SLength>, DpiError> {
        let mut ind: SLength = 0;
        // SAFETY: `buf` 指向调用方给的、至少 `buf_len` 字节的缓冲区；句柄有效。
        let code = unsafe {
            (self.api.dpi_get_data)(
                self.raw,
                index,
                value_ctype,
                buf,
                SLength::try_from(buf_len).unwrap_or(0),
                &raw mut ind,
            )
        };
        if ind == indicator::NULL_DATA {
            return Ok(None);
        }
        check(self.api, handle_type::STMT, self.raw, code, "dpi_get_data")?;
        Ok(Some(ind))
    }

    /// 读变长类型。返回 `None` 表示该列是 NULL，否则返回写入的字节数。
    fn read_variable(
        &self,
        index: u16,
        value_ctype: i16,
        buf: &mut [u8],
    ) -> Result<Option<usize>, DpiError> {
        let mut ind: SLength = 0;
        // SAFETY: 缓冲区容量与字节长度一致；句柄有效。
        let code = unsafe {
            (self.api.dpi_get_data)(
                self.raw,
                index,
                value_ctype,
                buf.as_mut_ptr().cast(),
                SLength::try_from(buf.len()).unwrap_or(0),
                &raw mut ind,
            )
        };
        if ind == indicator::NULL_DATA {
            return Ok(None);
        }
        check(self.api, handle_type::STMT, self.raw, code, "dpi_get_data")?;
        Ok(Some(
            usize::try_from(ind.max(0)).unwrap_or(0).min(buf.len()),
        ))
    }
}

/// [`Stmt::read_chunk`] 的返回值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadChunk {
    /// 该列是 NULL。
    Null,
    /// 读到了这么多字节（已写进调用方给的缓冲区）。
    Data(usize),
    /// 没有更多数据了。
    End,
}

impl Drop for Stmt {
    fn drop(&mut self) {
        // SAFETY: 句柄由本结构体独占；`guard` 保证连接句柄此刻仍然有效。
        unsafe {
            (self.api.dpi_free_stmt)(self.raw);
        }
    }
}

/// 把 `&str` 转成 NUL 结尾的 C 字符串。
fn cstr(value: &str) -> Result<CString, DpiError> {
    CString::new(value).map_err(|_| DpiError::misuse("字符串里含 NUL 字节，DPI 无法接收"))
}

/// 返回码检查：成功通过，否则从该句柄抽诊断记录拼一条错误。
fn check(api: &Api, hndl_type: i16, hndl: Handle, code: Ret, what: &str) -> Result<(), DpiError> {
    if ret::is_success(code) {
        Ok(())
    } else {
        Err(diag(api, hndl_type, hndl, code, what))
    }
}

/// 从句柄上取诊断记录（最多 4 条），拼成一条可读的消息。
fn diag(api: &Api, hndl_type: i16, hndl: Handle, code: Ret, what: &str) -> DpiError {
    let mut messages: Vec<String> = Vec::new();
    let mut first_code: i32 = 0;

    for rec in 1..=4i16 {
        let mut buf = [0u8; 1024];
        let mut msg_len: i16 = 0;
        let mut err_code: i32 = 0;
        // SAFETY: 缓冲区容量与 `buf_sz` 一致；其余都是有效出参。
        let got = unsafe {
            (api.dpi_get_diag_rec)(
                hndl_type,
                hndl,
                rec,
                &raw mut err_code,
                buf.as_mut_ptr(),
                i16::try_from(buf.len()).unwrap_or(1024),
                &raw mut msg_len,
            )
        };
        if !ret::is_success(got) {
            break;
        }
        if rec == 1 {
            first_code = err_code;
        }
        let len = usize::try_from(msg_len.max(0)).unwrap_or(0).min(buf.len());
        if len == 0 {
            break;
        }
        // 客户端字符集没设成 UTF-8 时，诊断消息是 GBK 字节：这里保留原始字节的
        // 十六进制，避免「乱码看一眼就过去了」。
        let text = match std::str::from_utf8(&buf[..len]) {
            Ok(text) => text.trim().to_owned(),
            Err(_) => format!("<非 UTF-8 字节 {:02X?}>", &buf[..len]),
        };
        messages.push(text);
    }

    let message = if messages.is_empty() {
        format!("{what} 失败（返回码 {code}），且该句柄没有诊断记录")
    } else {
        format!("{what} 失败：{}", messages.join("；"))
    };
    DpiError {
        code: if first_code == 0 {
            i32::from(code)
        } else {
            first_code
        },
        message,
    }
}

/// 便捷常量：`PG_UTF8`，供 `Env::set_local_code` 使用。
pub const UTF8: i32 = codeset::UTF8;
