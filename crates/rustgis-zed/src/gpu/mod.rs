//! GPU rendering of a RustGIS map into a wgpu render pass.
//!
//! [`GpuMap`] is a port of RustGIS 0.6.0's egui map backend
//! (`crates/ui-egui/src/mapgpu.rs`, Apache-2.0; see the attribution in `mapgpu.rs`) to wgpu 29
//! and to a host without egui. It takes the UI-independent [`rustgis_render::gpu_scene::GpuScene`]
//! the CPU renderer produces with `Backend::Gpu`, uploads it, and draws it into an offscreen
//! 4× MSAA target; [`GpuMap::present`] then blits the resolved frame into a surface texture (the
//! map's own child window in `crate::win`) or [`GpuMap::render`] reads it back for a test.
//!
//! The scene is drawn at **device pixels** (`view.width * dpi`), so a 1200×800 logical map at
//! 125 % scaling is 1500×1000 real pixels: vector edges stay sharp however the window is sized,
//! which is what the CPU "small frame stretched over the window" path cannot do.
//!
//! [`GpuMap::set_overlay`] adds a premultiplied RGBA layer over the map (the placed label text,
//! rasterised on the CPU at the same device resolution), drawn in the scene's own pass with the
//! renderer's premultiplied-alpha blend — the "GPU map + text layer" the egui app draws too.

mod mapgpu;

pub use mapgpu::GpuMap;
