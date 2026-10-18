//! 跨平台的动态库加载（Windows 的 `LoadLibraryW` / Unix 的 `dlopen`）。
//!
//! 为什么不用 `#[link]` 静态链 `dmdpi.lib`：
//!
//! * `dmdpi.lib` / `libdmdpi.so` 只在**装了达梦客户端**的机器上有，而本 workspace 的
//!   离线门禁（`cargo clippy --workspace --all-targets`、`cargo test --workspace`）
//!   必须在没有达梦的机器上也能过。静态链会让门禁直接链接失败。
//! * 交叉编译（aarch64-unknown-linux-gnu）时更不可能有目标平台的库文件。
//! * 运行时加载能把「没装客户端」变成一个清晰的错误，而不是一个链接期的天书。
//!
//! 代价是每个导出符号都要走一次 `GetProcAddress` / `dlsym`——这发生在
//! **进程内第一次**建连时（`OnceLock`），之后就是一个函数指针调用，不进入热路径。

use std::ffi::c_void;

/// 加载成功的共享库句柄。故意不实现 `Drop`：DPI 的函数指针在整个进程生命周期内
/// 都指向这份库，卸载它会让所有已解析的函数指针变成野指针。
#[derive(Debug)]
pub(crate) struct Lib {
    raw: *mut c_void,
}

// `LoadLibraryW` / `dlopen` 返回的句柄可以跨线程使用（库本身是线程安全的），
// 而这里只把它当作不透明句柄传给 `GetProcAddress` / `dlsym`。
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

impl Lib {
    /// 按路径打开共享库。失败时返回一条可直接展示给用户的原因。
    pub(crate) fn open(path: &str) -> Result<Self, String> {
        platform::open(path).map(|raw| Self { raw })
    }

    /// 取一个导出符号的地址。
    pub(crate) fn symbol(&self, name: &str) -> Option<*mut c_void> {
        platform::symbol(self.raw, name)
    }

    /// 取一个导出符号并转换成函数指针。
    ///
    /// # Safety
    ///
    /// 调用方必须保证 `T` 与该符号在 DPI 头文件里的真实签名**逐参数一致**。
    /// 签名错了不会有编译期报错，只会在调用时把栈踩坏——所以这里的每个 `T`
    /// 都必须能从 `D:\dmdbms\include\DPI.h` 逐字对出来。
    pub(crate) unsafe fn function<T: Copy>(&self, name: &str) -> Result<T, String> {
        let ptr = self
            .symbol(name)
            .ok_or_else(|| format!("导出符号不存在：{name}（DPI 版本不匹配？）"))?;
        debug_assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<*mut c_void>(),
            "函数指针与 void* 的宽度必须一致"
        );
        // SAFETY: 指针非空，且调用方保证 `T` 与真实签名一致（见函数文档）。
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) })
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn GetLastError() -> u32;
    }

    /// `LOAD_WITH_ALTERED_SEARCH_PATH`：让 dll 的**依赖**按 dll 自身所在目录去找。
    ///
    /// 这是必需的：`dmdpi.dll` 依赖同目录下的其它达梦 dll，用普通 `LoadLibraryW`
    /// 会去按进程的 PATH 找依赖，缺依赖时报 `GetLastError=126`
    /// （ERROR_MOD_NOT_FOUND，看起来像「dll 不存在」）。不要求用户改 PATH。
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

    pub(super) fn open(path: &str) -> Result<*mut c_void, String> {
        let wide: Vec<u16> = OsStr::new(path).encode_wide().chain([0u16]).collect();
        // SAFETY: `wide` 以 NUL 结尾，且在调用期间一直存活；`file` 传 NULL 表示按文件名解释。
        let raw = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        };
        if raw.is_null() {
            // SAFETY: `GetLastError` 无参数、无副作用。
            let code = unsafe { GetLastError() };
            Err(format!("LoadLibraryExW 失败（GetLastError={code}）"))
        } else {
            Ok(raw)
        }
    }

    pub(super) fn symbol(module: *mut c_void, name: &str) -> Option<*mut c_void> {
        let mut c_name = Vec::with_capacity(name.len() + 1);
        c_name.extend_from_slice(name.as_bytes());
        c_name.push(0);
        // SAFETY: `c_name` 是 NUL 结尾的 ASCII 字节串；`module` 由 `open` 返回且仍有效。
        let ptr = unsafe { GetProcAddress(module, c_name.as_ptr()) };
        (!ptr.is_null()).then_some(ptr)
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::{CString, c_char, c_int, c_void};

    /// POSIX：立即解析所有符号。用 `RTLD_NOW` 是因为我们本来就逐个取符号，
    /// 延迟解析只会把「符号缺失」推后到更难定位的地方。
    const RTLD_NOW: c_int = 2;

    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *mut c_char;
    }

    pub(super) fn open(path: &str) -> Result<*mut c_void, String> {
        let c_path = CString::new(path).map_err(|_| "库路径里含 NUL 字节".to_owned())?;
        // SAFETY: `c_path` 是有效的 NUL 结尾字符串。
        let raw = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW) };
        if raw.is_null() {
            Err(last_error())
        } else {
            Ok(raw)
        }
    }

    pub(super) fn symbol(handle: *mut c_void, name: &str) -> Option<*mut c_void> {
        let c_name = CString::new(name).ok()?;
        // SAFETY: `c_name` 有效；`handle` 由 `open` 返回且仍有效。
        let ptr = unsafe { dlsym(handle, c_name.as_ptr()) };
        (!ptr.is_null()).then_some(ptr)
    }

    fn last_error() -> String {
        // SAFETY: `dlerror` 返回库内部的静态字符串（可能为 NULL）。
        let raw = unsafe { dlerror() };
        if raw.is_null() {
            return "dlopen 失败，且 dlerror 没有给出原因".to_owned();
        }
        // SAFETY: 非空且以 NUL 结尾，只在本次调用内读取。
        unsafe { std::ffi::CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned()
    }
}
