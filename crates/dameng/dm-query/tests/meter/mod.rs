//! 内存预算的**进程内**判据：一个不引第三方依赖的「计数分配器」。
//!
//! 缺口背景：设计方案 §14 的 G4/G5/G6 出口记录里，「内存纪律」（分块读/写不随值放大）
//! 一直只有功能性验证，写着「未采 `dhat`/RSS」。本机离线、也不该为几个数字引依赖树
//! （同 §12.3 T6.6 不引 `criterion`、M4 不引 `dhat` 的立场），所以这里用
//! **`GlobalAlloc` 薄包装 `System`** 直接量同一件事：
//!
//! * `peak_growth()` —— 窗口内「活字节」比窗口开始时最多多出多少（峰值内存）；
//! * `total_growth()` —— 窗口内新分配的总字节数（含瞬时，用来判断「每批一次」还是「每行一次」）；
//! * `net_growth()` —— 窗口结束时的净活字节变化（正数=有保留/泄漏）。
//!
//! 三条纪律，破一条数字就没意义：
//!
//! 1. **量的是我们自己的分配**。驱动 DLL 内部的 `malloc` 走它自己的 CRT，这里看不见；
//!    真机用例同样如此（见 `docs/acceptance-report.md` §2.5 的说明）。
//! 2. **同一二进制里只能有一个窗口**：计数是全局的，所以测量前先 `serialize()` 拿闸门，
//!    并且**拿闸门之后再 `Window::open()`**；本二进制的每条用例都必须拿闸门。
//! 3. **窗口里只放被测负载**：建运行时、建夹具、灌数据都要在 `open()` 之前做完，
//!    否则量到的是准备工作的内存。
//! 4. **读数会被同进程其他线程扰动**：`peak` 可能**少记**（别的线程正好在我们的窗口里释放了
//!    内存，而峰值是「活字节」的高水位），`total` 只会**多记**（别人的分配也算进来），
//!    `net` 两边都会偏。所以断言要留带宽、以 `total` 为准绳，别断到精确字节——
//!    这个仪器的用途是判「量级」（25 KiB vs 2.4 MiB），KiB 级噪声不影响结论。
//!
//! 用法：`let _gate = serialize(); let rt = runtime(); let m = rt.block_on(workload());`
//! —— 闸门持在**同步**函数里（`block_on` 不是 `.await`，不会踩 `await_holding_lock`）。

#![allow(dead_code)] // 共享给多个测试二进制，每个只用到其中一部分

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 包装 `System`：只多记三个数，分配行为本身一点不改。
struct Counting;

/// 当前活字节数（分配减释放）。
static LIVE: AtomicUsize = AtomicUsize::new(0);
/// 自进程启动以来活字节达到过的最大值。
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// 自进程启动以来新分配的总字节数（含已经释放的）。
static TOTAL: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn note_allocated(size: usize) {
    TOTAL.fetch_add(size, Ordering::Relaxed);
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    // CAS 循环抬峰值：失败说明别的线程刚改过，用观测值重试。
    let mut peak = PEAK.load(Ordering::Relaxed);
    while live > peak {
        match PEAK.compare_exchange_weak(peak, live, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => peak = observed,
        }
    }
}

fn note_freed(size: usize) {
    LIVE.fetch_sub(size, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 转交给 `System`，参数与契约同 `GlobalAlloc::alloc`。
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            note_allocated(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 同上。
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            note_allocated(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_freed(layout.size());
        // SAFETY: 同上；指针与布局由调用方保证仍然有效。
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: 同上。
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            // 只有成功才改账：失败时原块还在调用方手里。
            if new_size >= layout.size() {
                note_allocated(new_size - layout.size());
            } else {
                note_freed(layout.size() - new_size);
            }
        }
        new_ptr
    }
}

/// 全局闸门：计数是全局的，同一二进制里同时只能有一个测量窗口。
///
/// 返回值必须绑到一个变量上（`let _gate = serialize();`）——绑定成 `_` 会立刻解锁。
/// 闸门可以持着穿过 `block_on`：`MutexGuard` 不是 `Send`，但 `new_current_thread`
/// 运行时只在当前线程上跑，不需要 `Send`。
pub fn serialize() -> std::sync::MutexGuard<'static, ()> {
    static GATE: Mutex<()> = Mutex::new(());
    // 别的用例 panic 会毒化锁；这里保护的不是数据，照用。
    GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 只带时间驱动的单线程运行时：夹具是假后端，不做 IO。
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("建测试运行时失败")
}

/// 测量窗口。`open()` 把峰值重置到当前活字节，三个读数都相对窗口开始。
pub struct Window {
    start_live: usize,
    start_total: usize,
}

impl Window {
    /// 开窗。注意：**先在闸门里，再开窗**。
    pub fn open() -> Window {
        let start_live = LIVE.load(Ordering::Relaxed);
        PEAK.store(start_live, Ordering::Relaxed);
        Window {
            start_live,
            start_total: TOTAL.load(Ordering::Relaxed),
        }
    }

    /// 窗口内峰值活字节相对窗口开始的增量。
    pub fn peak_growth(&self) -> usize {
        PEAK.load(Ordering::Relaxed).saturating_sub(self.start_live)
    }

    /// 窗口内新分配的总字节数。
    pub fn total_growth(&self) -> usize {
        TOTAL
            .load(Ordering::Relaxed)
            .saturating_sub(self.start_total)
    }

    /// 净活字节变化：>0 表示窗口结束时比开始时多留了这么多。
    pub fn net_growth(&self) -> isize {
        let live = LIVE.load(Ordering::Relaxed);
        live as isize - self.start_live as isize
    }
}

/// 把字节数写成 KiB，给失败信息用（断言失败才分配，不进窗口）。
pub fn kib(bytes: usize) -> String {
    format!("{:.1} KiB", bytes as f64 / 1024.0)
}

#[test]
fn the_meter_sees_a_known_allocation() {
    let _gate = serialize();
    let window = Window::open();
    let buf: Vec<u8> = vec![0u8; 256 * 1024];
    let peak = window.peak_growth();
    let total = window.total_growth();
    drop(buf);
    let net = window.net_growth();

    // `total` 是单调计数器，只会多记（别的线程的分配也算进来），所以这条可以断严格。
    assert!(total >= 256 * 1024, "总量只记到 {}", kib(total));
    // `peak` 是「活字节」高水位，别的线程在我们的窗口里释放内存会让它少记一截
    // （同二进制的其他用例被闸门挡着，libtest 自己的线程仍会分配/释放），留 8 KiB 带宽。
    assert!(
        peak + 8 * 1024 >= 256 * 1024,
        "峰值只记到 {}（另见 total 与 net）",
        kib(peak)
    );
    // 净值同样受其他线程影响，留 64 KiB 带宽——它要防的是「窗口里留下的东西不释放」。
    assert!(net.abs() < 64 * 1024, "释放后净值偏了 {} 字节", net);
}

/// 进程常驻内存（RSS）当前值，单位 KiB。**这是整进程口径**，与 `Window`（只数我们自己的
/// 分配）互补：**驱动 DLL 内部的 `malloc` 会体现在这里**——这正是「流式 RSS 平坦」
/// 那条判据要的东西。
///
/// * Windows：`K32GetProcessMemoryInfo`（kernel32）读 `WorkingSetSize`，不引任何 crate，
///   手写 `PROCESS_MEMORY_COUNTERS` 的内存布局；
/// * Linux：读 `/proc/self/statm` 第二列（resident pages）× 4 KiB（麒麟 x86_64/aarch64
///   的常规页都是 4 KiB）；
/// * 其他平台返回 `None`——不猜。
pub fn rss_kib() -> Option<usize> {
    platform::rss_kib()
}

#[cfg(windows)]
mod platform {
    /// `PROCESS_MEMORY_COUNTERS`（x64 上是 72 字节：2 个 `DWORD` + 8 个 `SIZE_T`）。
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut core::ffi::c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    pub(super) fn rss_kib() -> Option<usize> {
        // 用结构体更新语法而不是「先 `default()` 再逐字段赋值」：后者会踩 clippy 的
        // `field_reassign_with_default`（`-D warnings` 下是错）。
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            ..Default::default()
        };
        // SAFETY: `GetCurrentProcess()` 返回的伪句柄对本进程始终有效；`counters` 是我们
        // 自己的结构体，`cb` 报的就是它的字节长度，Windows 只会往里写这么多。
        let ok =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        (ok != 0).then_some(counters.working_set_size / 1024)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    pub(super) fn rss_kib() -> Option<usize> {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident_pages: usize = statm.split_whitespace().nth(1)?.parse().ok()?;
        Some(resident_pages * 4)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub(super) fn rss_kib() -> Option<usize> {
        None
    }
}

/// 仪器自检：**RSS 探针必须看得见一块真的落了页的内存**。
///
/// 它比上面那条自检更吃噪声（RSS 是操作系统口径，别的线程也会抬它），所以只断
/// 「涨了至少 8 MiB」这种量级断言，不断精确值。
#[test]
fn the_rss_probe_sees_a_big_allocation() {
    let _gate = serialize();
    let Some(before) = rss_kib() else {
        println!("RSS 探针在本平台不可用（只实现了 Windows 与 Linux），跳过");
        return;
    };
    // Windows 上 `vec![0u8; N]` 是惰性提交的，要真写一遍页才进 RSS。
    let mut buf = vec![0u8; 32 * 1024 * 1024];
    for byte in buf.iter_mut().step_by(4096) {
        *byte = 1;
    }
    let during = rss_kib().expect("RSS 探针刚才还可用了");
    let growth = during.saturating_sub(before);
    assert!(
        growth >= 8 * 1024,
        "32 MiB 的块已经落页，RSS 却只涨了 {growth} KiB（前 {before} / 后 {during}）"
    );
    drop(buf);
    println!("RSS 探针：{before} KiB → {during} KiB（+{growth} KiB）");
}
