//! EW GIS: a native `wgpu` 2D map + WGS84 quadtree LOD globe.
//!
//! The crate is a library so that a host can embed its renderer instead of
//! launching the standalone `winit` binary:
//!
//! * [`viewer::Viewer`] owns the GPU device, the tile atlas, the 2D plane, the
//!   3D globe, the vector plot, the MVT overlay and the cameras, and renders
//!   either into a window surface ([`viewer::Viewer::render_to_view`]) or into
//!   an offscreen frame that is read back to the CPU
//!   ([`viewer::Viewer::render_rgba`]). It creates no window and never starts
//!   `winit`.
//! * [`app`] is the standalone host: a `winit` window, input, HUD, scripted
//!   acceptance actions and the screenshot/benchmark exit policy, driving a
//!   [`viewer::Viewer`].
//!
//! `cargo run -p ew_gis` still builds and runs the binary from `main.rs`, which
//! is only the argument/environment handling around [`app::run`].

pub mod app;
pub mod data;
pub mod gpu;
pub mod math;
pub mod render;
pub mod scene;
pub mod screenshot;
pub mod tiles;
pub mod viewer;
