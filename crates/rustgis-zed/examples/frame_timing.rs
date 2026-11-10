//! Measures what one frame of the `Help > RustGIS` window costs, layer by layer.
//!
//! The window draws the sample project at 1200×800. While a pointer gesture runs it draws
//! [`Quality::Preview`] frames (half the canvas, labels off, no antialiasing) and settles to a
//! [`Quality::Full`] frame after 150 ms. This harness puts a number on each of those, on the
//! `Session::run` path the window no longer uses for navigation, and on the CPU work the UI thread
//! used to do per frame (the BGRA swap for GPUI's atlas), so a change can be judged against the
//! previous numbers:
//!
//! ```powershell
//! cargo run -p rustgis_zed --example frame_timing
//! cargo run -p rustgis_zed --release --example frame_timing   # the shipped profile
//! ```
//!
//! Every measurement is the median of `RUNS` runs after a warm-up frame, so the first frame's
//! font/glyph cache work is not counted as a steady-state cost. Renders that should miss the
//! frame cache pan the camera by a hair first; the cache-hit row deliberately does not.

use std::time::{Duration, Instant};

use rustgis_zed::{BgraFrame, Frame, MapEngine, Quality};

#[cfg(target_os = "windows")]
use rustgis_zed::gpu::GpuMap;

/// Runs per measurement; the median is reported.
const RUNS: usize = 5;
/// The window's frame size.
const WIDTH: u32 = 1200;
const HEIGHT: u32 = 800;

fn main() {
    install_logger();

    // Fonts, glyphs and the projected geometry caches are built on the first frame; one warm-up
    // frame at the window's size keeps that out of every measurement below.
    let mut engine = MapEngine::new();
    let warm = Instant::now();
    let _ = engine.render_rgba(WIDTH, HEIGHT);
    println!(
        "[rustgis] bench warm-up (first full frame, includes font/geometry caches): {:.1} ms",
        ms(warm.elapsed())
    );

    bench("full 1200x800 labels=on (render_placed)", || {
        nudge(&mut engine);
        engine.render_rgba(WIDTH, HEIGHT)
    });
    engine.set_labels(false);
    bench("full 1200x800 labels=off (render)", || {
        nudge(&mut engine);
        engine.render_rgba(WIDTH, HEIGHT)
    });
    bench("full 1200x800 labels=off, BGRA (what the worker sends)", || {
        nudge(&mut engine);
        engine.render_bgra(WIDTH, HEIGHT)
    });
    bench("preview 1200x800 divisor=2 (600x400, no AA)", || {
        nudge(&mut engine);
        engine.render_bgra_quality(WIDTH, HEIGHT, Quality::Preview { divisor: 2 })
    });
    bench("preview 1200x800 divisor=4 (300x200, no AA)", || {
        nudge(&mut engine);
        engine.render_bgra_quality(WIDTH, HEIGHT, Quality::Preview { divisor: 4 })
    });
    bench("cache hit: same view, repeated full BGRA", || {
        engine.render_bgra(WIDTH, HEIGHT)
    });
    engine.set_labels(true);

    bench("Session::run(map.zoomIn) (not an interaction path)", || {
        engine
            .command("map.zoomIn", serde_json::json!({}))
            .map(|_| ())
    });
    bench("Session::run(map.zoomOut)", || {
        engine
            .command("map.zoomOut", serde_json::json!({}))
            .map(|_| ())
    });
    bench("direct Camera::zoom_center(2.0)", || {
        engine.zoom_center(2.0);
    });
    bench("direct Camera::pan_px(40,0)", || {
        engine.pan(40.0, 0.0);
    });

    let frame: Frame = engine
        .render_rgba(WIDTH, HEIGHT)
        .expect("the sample project renders");
    bench("UI (before): clone RGBA + swap R/B per frame", || {
        let mut bytes = frame.rgba.clone();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        bytes.len()
    });
    let bgra: BgraFrame = engine
        .render_bgra(WIDTH, HEIGHT)
        .expect("the sample project renders");
    bench("UI (after): move BGRA into an image buffer", || {
        let image: Option<image::RgbaImage> =
            image::ImageBuffer::from_raw(bgra.width, bgra.height, bgra.bgra.clone());
        image.map(|image| image.len())
    });
    println!(
        "[rustgis] bench frame bytes: {} KiB RGBA, {} KiB BGRA",
        frame.rgba.len() / 1024,
        bgra.bgra.len() / 1024
    );
    println!(
        "[rustgis] bench preview bytes at divisor 2: {} KiB",
        (WIDTH as usize / 2) * (HEIGHT as usize / 2) * 4 / 1024
    );

    bench_gpu();
}

/// The GPU path, measured offscreen (scene build + upload + 4× MSAA draw + read-back): the
/// window itself presents without the read-back, so these are upper bounds on its frame cost.
#[cfg(target_os = "windows")]
fn bench_gpu() {
    let instance = wgpu30::Instance::new(wgpu30::InstanceDescriptor {
        backends: wgpu30::Backends::all(),
        flags: wgpu30::InstanceFlags::default(),
        backend_options: wgpu30::BackendOptions::default(),
        memory_budget_thresholds: wgpu30::MemoryBudgetThresholds::default(),
        display: None,
    });
    let Some(adapter) = pollster::block_on(instance.request_adapter(&wgpu30::RequestAdapterOptions {
        power_preference: wgpu30::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()
    else {
        println!("[rustgis] bench gpu: no adapter, skipped");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu30::DeviceDescriptor {
        label: Some("rustgis_frame_timing"),
        required_features: wgpu30::Features::empty(),
        required_limits: wgpu30::Limits::downlevel_defaults()
            .using_resolution(adapter.limits())
            .using_alignment(adapter.limits()),
        memory_hints: wgpu30::MemoryHints::MemoryUsage,
        trace: wgpu30::Trace::Off,
        experimental_features: wgpu30::ExperimentalFeatures::disabled(),
    })) else {
        println!("[rustgis] bench gpu: no device, skipped");
        return;
    };
    println!("[rustgis] bench gpu adapter: {}", adapter.get_info().name);

    let mut engine = MapEngine::new();
    let mut scratch = rustgis_map::Map::new("bench");
    let mut map = GpuMap::new(&device, &queue, wgpu30::TextureFormat::Rgba8Unorm, (1500, 928));
    // Warm-up: fonts, geometry caches and the scene's pipelines.
    if let Some(frame) = engine.gpu_scene(WIDTH, HEIGHT - 58, 1.25) {
        map.prepare(&device, &queue, &mut scratch, &frame.view, &frame.scene, 1.25);
        let _ = map.render(&device, &queue);
    }

    bench("gpu scene build 1200x742 @125% (GPU device pixels 1500x928)", || {
        nudge(&mut engine);
        engine
            .gpu_scene(WIDTH, HEIGHT - 58, 1.25)
            .map_or(0, |frame| frame.scene.cmds.len())
    });
    bench("gpu prepare+draw+read-back 1500x928 (no present)", || {
        let Some(frame) = engine.gpu_scene(WIDTH, HEIGHT - 58, 1.25) else {
            return 0;
        };
        if !map.prepare(&device, &queue, &mut scratch, &frame.view, &frame.scene, 1.25) {
            return 0;
        }
        map.render(&device, &queue).len()
    });
    bench("gpu scene build 1200x742 @100% (1200x742)", || {
        nudge(&mut engine);
        engine
            .gpu_scene(WIDTH, HEIGHT - 58, 1.0)
            .map_or(0, |frame| frame.scene.cmds.len())
    });
    bench("gpu prepare+draw+read-back 1200x742 (no present)", || {
        let Some(frame) = engine.gpu_scene(WIDTH, HEIGHT - 58, 1.0) else {
            return 0;
        };
        if !map.prepare(&device, &queue, &mut scratch, &frame.view, &frame.scene, 1.0) {
            return 0;
        }
        map.render(&device, &queue).len()
    });

    // The label layer: what a settle costs to draw (the text raster), what a gesture frame costs
    // for it (a uniform write, no text at all), and what drawing the layer over the map adds.
    let Some(frame) = engine.gpu_scene(WIDTH, HEIGHT - 58, 1.25) else {
        println!("[rustgis] bench labels: no GPU scene, skipped");
        return;
    };
    let (unplaced, placed) = (&frame.scene.unplaced, &frame.scene.labels);
    println!("[rustgis] bench labels: {} placed, {} unplaced", placed.len(), unplaced.len());
    bench("labels: rasterize 1500x928 @125% (the settle's text)", || {
        rustgis_zed::labels::rasterize(unplaced, placed, WIDTH, HEIGHT - 58, 1.25).map_or(0, |(_, _, rgba)| rgba.len())
    });
    bench("labels: rasterize 1200x742 @100%", || {
        rustgis_zed::labels::rasterize(unplaced, placed, WIDTH, HEIGHT - 58, 1.0).map_or(0, |(_, _, rgba)| rgba.len())
    });
    let Some((lw, lh, rgba)) = rustgis_zed::labels::rasterize(unplaced, placed, WIDTH, HEIGHT - 58, 1.25) else {
        println!("[rustgis] bench labels: nothing to draw, skipped");
        return;
    };
    map.set_overlay(&device, &queue, lw, lh, &rgba);
    bench("labels: move the layer's quad (what a gesture frame adds)", || {
        map.set_overlay_rect(&queue, [12.0, -8.0, lw as f32 + 12.0, lh as f32 - 8.0]);
        0
    });
    bench("gpu prepare+draw+read-back 1500x928 with the label layer", || {
        let Some(frame) = engine.gpu_scene(WIDTH, HEIGHT - 58, 1.25) else {
            return 0;
        };
        if !map.prepare(&device, &queue, &mut scratch, &frame.view, &frame.scene, 1.25) {
            return 0;
        }
        map.render(&device, &queue).len()
    });
    map.clear_overlay();
}

#[cfg(not(target_os = "windows"))]
fn bench_gpu() {}

/// Pan by a hair so the frame cache misses: measurements below are render costs, not copies.
fn nudge(engine: &mut MapEngine) {
    engine.pan(0.01, 0.0);
}

/// Times `f` `RUNS` times (plus one warm-up call) and reports the median.
fn bench<T>(label: &str, mut f: impl FnMut() -> T) {
    let _ = f();
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let start = Instant::now();
        let _ = f();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    println!(
        "[rustgis] bench {label}: median {:.2} ms (min {:.2}, max {:.2})",
        ms(median),
        ms(samples[0]),
        ms(samples[samples.len() - 1])
    );
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Prints the adapter's own `[rustgis]` lines, which carry `Session::run` timings and undo depths.
fn install_logger() {
    struct Printer;

    impl log::Log for Printer {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            if record
                .target()
                .starts_with(concat!(env!("CARGO_PKG_NAME"), "::"))
            {
                println!("[log] {}", record.args());
            }
        }

        fn flush(&self) {}
    }

    static PRINTER: Printer = Printer;
    let _ = log::set_logger(&PRINTER);
    log::set_max_level(log::LevelFilter::Info);
}
