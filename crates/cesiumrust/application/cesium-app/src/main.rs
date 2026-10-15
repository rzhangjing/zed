//! cesium-app 的 bin target —— 一层壳。
//!
//! 查看器的全部装配都在 lib 根的 [`cesium_app::run`]（原先就是本文件的
//! `fn main`）。留这层壳是为了让 `cargo run -p cesium-app`、
//! `tests/e2e_render.rs` 与 CESIUM_HEADLESS 捕获路径保持原样，同时使
//! `src/*.rs` 里的模块只被编译一次：bin 与 lib 各声明一遍 `mod` 会把
//! 同一批源文件编成两份互不相同的类型。
//!
//! 不直接 `fn main() { cesium_app::run() }` 之外再加逻辑：任何新的启动
//! 行为都属于 lib 的 [`cesium_app::run`]，否则嵌入方又看不到它。

fn main() {
    cesium_app::run()
}
