//! 构建脚本：只做一件事——在非 Windows 上把 `libdl` 加到链接行。
//!
//! DPI 的共享库是**运行时**用 `LoadLibraryW` / `dlopen` 加载的（见 `src/sys.rs`），
//! 所以构建期不需要 `dmdpi.lib` 或 `libdmdpi.so`：没有装达梦客户端的机器上
//! `cargo check` / `cargo test` 照样能过，只有在真正建连时才会报「库加载失败」。
//!
//! 唯一的例外是 Unix 上 `dlopen` / `dlsym` 符号本身要在链接期解析：
//! glibc 2.34 之后它们在 libc 里，之前（以及 musl 的某些版本）在 libdl 里，
//! 显式带上 `-ldl` 两种都能过。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DM_DPI_LIB");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if !matches!(target_os.as_str(), "windows" | "macos" | "ios" | "wasi") {
        println!("cargo:rustc-link-lib=dl");
    }
}
