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
}

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
