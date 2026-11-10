//! # M11.4 (#83) — `e2e_render`: end-to-end headless render tests
//!
//! These tests are the executable form of the M11.3 headless capture path
//! (`cesium_bevy_render::headless`). Each test **spawns the real `cesium-app`
//! binary** with `CESIUM_HEADLESS=1` + the offline determinism env family
//! (`OFFLINE_*_ROOT` + `STRICT_OFFLINE` + `FIXED_TIME`) + a single deterministic
//! camera pose (`FIXED_CAMERA`), lets it render N frames to an offscreen target,
//! capture a PNG, and exit cleanly — then asserts on the artefact.
//!
//! ## Why self-consistency instead of golden-image matching
//!
//! The committed `specs/baselines/v0/*.png` were captured **online** (Bing
//! imagery) at **1920×1080** (DPI-dependent, see `DEFER-M0-DPI`), whereas the
//! headless offscreen target is a fixed **1280×720** LDR sRGB buffer fed from
//! the **offline** fixtures in `specs/fixtures/`. Dimensions and content
//! therefore cannot match those baselines directly. Instead this suite proves the
//! three properties that actually matter for a render regression gate:
//!
//! 1. **Validity** — the binary produces a decodable, non-empty, correctly-sized
//!    PNG that contains real scene content (a lit globe, not a black frame).
//! 2. **Determinism** — the *same* view rendered twice is pixel-identical
//!    (PSNR ≥ 45 dB, the project golden standard), proving `offline + FIXED_TIME`
//!    removes run-to-run variance.
//! 3. **View-distinctness** — *different* camera poses produce *different*
//!    renders, proving the capture is driven by real scene state (not a static
//!    stub).
//!
//! Each view is a separate `#[test]` that invokes the binary exactly once
//! (determinism invokes it twice), reusing the single-view headless branch in
//! `main.rs` unchanged.
//!
//! ## GPU / sandbox note
//!
//! These tests require a GPU (wgpu adapter) and spawn a child process. They are
//! `#[ignore]`d so the **mandatory PR gate** (`cesiumrust-ci.yml`, whose `test`
//! job is a plain `ubuntu-latest` runner with no GPU / no `xvfb` / no llvmpipe)
//! does **not** collect them — matching that workflow's own header intent
//! ("不含 e2e 渲染测试") and keeping the PR gate fast and adapter-free. On the
//! reference machine (RTX 3080 / Vulkan) run them with
//! `cargo test -p cesium-app -- --ignored`; on GPU-less CI the
//! `.github/workflows/cesiumrust-e2e.yml` nightly/dispatch workflow runs them
//! under `xvfb-run` + `llvmpipe` via its `cargo test --workspace -- --ignored`
//! step (see `docs/deferred.md#75`, FIX-CI-GPU). `#[ignore]` here is a
//! **routing** mechanism, not an `#[ignore]`-escape: the suite still executes on
//! any device-provisioned runner, only it is steered out of the hardware-less
//! mandatory gate into the hardware-provisioned non-blocking one.

use assert_cmd::Command as AssertCommand;
use pixel_diff::compare_images;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;

/// Headless offscreen resolution (fixed by `headless::DEFAULT_HEADLESS_*`).
const W: u32 = 1280;
const H: u32 = 720;

/// Frames rendered before capture. Enough for the base sphere + first offline
/// LOD ring to settle deterministically; small enough to keep tests fast.
const FRAMES: usize = 90;

/// Per-capture wall-clock budget. The reference GPU finishes in a few seconds;
/// the generous ceiling absorbs cold shader compilation + `llvmpipe` on CI.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(240);

/// Golden PSNR floor (dB) for the determinism assertion.
const DETERMINISM_PSNR_DB: f64 = 45.0;

/// A deterministic camera pose (render units; 1 unit = 6_378_137 m). Poses are
/// the exact transforms from `specs/scripts/baseline_v0.toml`.
struct View {
    name: &'static str,
    pos: [f64; 3],
    quat: [f64; 4], // [x, y, z, w]
}

/// Default globe view (lon 0, lat ~23, dist 3.0) — the pristine v0 anchor.
fn globe_default() -> View {
    View {
        name: "globe_default",
        pos: [2.763183, 0.0, 1.168255],
        quat: [0.390699, 0.390699, 0.589368, 0.589368],
    }
}

/// North-pole view (lat 80) — a distinctly different orientation.
fn pole_north() -> View {
    View {
        name: "pole_north",
        pos: [0.520945, 0.0, 2.954423],
        quat: [0.061628, 0.061628, 0.704416, 0.704416],
    }
}

/// Workspace root (`cesiumrust/`) from this crate's manifest dir
/// (`application/cesium-app`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// An `assert_cmd` command bound to this package's built `cesium-app` binary.
/// `CARGO_BIN_EXE_<name>` is set by cargo for integration tests in the same
/// package and points at the freshly-built executable, so the binary is always
/// up to date and no path guessing is needed.
fn app_command() -> AssertCommand {
    AssertCommand::from_std(StdCommand::new(env!("CARGO_BIN_EXE_cesium-app")))
}

/// Writes a `FIXED_CAMERA` TOML (`[camera]` pos/quat/fov_y) into `dir`.
fn write_camera_toml(dir: &Path, view: &View) -> PathBuf {
    let path = dir.join(format!("{}_camera.toml", view.name));
    let toml = format!(
        "[camera]\npos = [{}, {}, {}]\nquat = [{}, {}, {}, {}]\nfov_y = 60.0\n",
        view.pos[0],
        view.pos[1],
        view.pos[2],
        view.quat[0],
        view.quat[1],
        view.quat[2],
        view.quat[3],
    );
    std::fs::write(&path, toml).expect("write FIXED_CAMERA toml");
    path
}

/// Runs one headless capture of `view` and returns the (still-live) temp dir +
/// the PNG path. The temp dir must be bound by the caller so the artefact is not
/// deleted before it is inspected.
///
/// `hdr` selects the `CESIUM_HEADLESS_HDR` offscreen format (Rgba16Float);
/// `false` keeps the default LDR sRGB path.
fn run_capture(view: &View, hdr: bool) -> (tempfile::TempDir, PathBuf) {
    let root = workspace_root();
    let tmp = tempfile::Builder::new()
        .prefix(&format!("e2e_render_{}_", view.name))
        .tempdir()
        .expect("create temp dir");
    let out_png = tmp.path().join(format!("{}.png", view.name));
    let cam = write_camera_toml(tmp.path(), view);

    let mut cmd = app_command();
    cmd.env("CESIUM_HEADLESS", "1")
        .env("CESIUM_HEADLESS_FRAMES", FRAMES.to_string())
        .env("CESIUM_HEADLESS_OUTPUT", &out_png)
        .env(
            "OFFLINE_IMAGERY_ROOT",
            root.join("specs/fixtures/offline-imagery"),
        )
        .env(
            "OFFLINE_TERRAIN_ROOT",
            root.join("specs/fixtures/offline-terrain"),
        )
        .env("STRICT_OFFLINE", "1")
        .env("FIXED_TIME", "1")
        .env("FIXED_CAMERA", &cam)
        .env("RUST_LOG", "warn")
        .timeout(CAPTURE_TIMEOUT);

    if hdr {
        cmd.env("CESIUM_HEADLESS_HDR", "1");
    } else {
        // Ensure an inherited HDR flag from the parent env cannot leak in.
        cmd.env_remove("CESIUM_HEADLESS_HDR");
    }

    cmd.assert().success();
    (tmp, out_png)
}

/// Decodes the PNG and returns `(width, height, mean_rgb, max_channel,
/// distinct_channel_values)`.
fn png_stats(path: &Path) -> (u32, u32, f64, u8, usize) {
    let img = image::open(path)
        .unwrap_or_else(|e| panic!("PNG at {path:?} must decode: {e}"))
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    let mut sum = 0u64;
    let mut n = 0u64;
    let mut max_channel = 0u8;
    let mut seen = std::collections::HashSet::new();
    for p in img.pixels() {
        for &c in &p.0[..3] {
            sum += c as u64;
            n += 1;
            if c > max_channel {
                max_channel = c;
            }
            seen.insert(c);
        }
    }
    (w, h, sum as f64 / n as f64, max_channel, seen.len())
}

/// Asserts the artefact is a valid, non-empty, correctly-sized PNG with real
/// scene content (a lit globe: bright pixels present + non-uniform).
fn assert_valid_capture(path: &Path, view_name: &str) {
    assert!(path.exists(), "{view_name}: PNG was not produced at {path:?}");
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    assert!(len > 1024, "{view_name}: PNG suspiciously small ({len} bytes)");

    let (w, h, mean, maxc, distinct) = png_stats(path);
    assert_eq!(
        (w, h),
        (W, H),
        "{view_name}: headless capture must be {W}x{H}"
    );
    // A black/empty frame has max channel 0 and a single distinct value. The
    // offline globe render has a bright sunlit disc (centre ≈ 238,255,255 on the
    // reference machine) against black space, so both must be well above zero.
    assert!(
        maxc > 100,
        "{view_name}: frame looks black (max channel {maxc}, mean {mean:.1}) — globe did not render"
    );
    assert!(
        distinct > 16,
        "{view_name}: frame is near-uniform ({distinct} distinct channel values) — no scene content"
    );
    eprintln!(
        "[e2e_render/{view_name}] {w}x{h} bytes={len} mean_rgb={mean:.1} max={maxc} distinct={distinct}"
    );
}

// ── Validity: one test per view, each invokes the binary once ─────────────

#[test]
#[ignore = "requires a GPU (wgpu adapter); routed to cesiumrust-e2e.yml (xvfb + llvmpipe, --ignored). See docs/deferred.md#75"]
fn e2e_globe_default_produces_valid_png() {
    let view = globe_default();
    let (_tmp, png) = run_capture(&view, false);
    assert_valid_capture(&png, view.name);
}

#[test]
#[ignore = "requires a GPU (wgpu adapter); routed to cesiumrust-e2e.yml (xvfb + llvmpipe, --ignored). See docs/deferred.md#75"]
fn e2e_pole_north_produces_valid_png() {
    let view = pole_north();
    let (_tmp, png) = run_capture(&view, false);
    assert_valid_capture(&png, view.name);
}

// ── Determinism: same view twice must be pixel-identical (offline+FIXED_TIME) ──

#[test]
#[ignore = "requires a GPU (wgpu adapter); routed to cesiumrust-e2e.yml (xvfb + llvmpipe, --ignored). See docs/deferred.md#75"]
fn e2e_capture_is_deterministic() {
    let view = globe_default();
    let (_t1, png1) = run_capture(&view, false);
    let (_t2, png2) = run_capture(&view, false);

    let a = image::open(&png1).expect("first capture decodes");
    let b = image::open(&png2).expect("second capture decodes");
    let m = compare_images(&a, &b, DETERMINISM_PSNR_DB)
        .expect("same view twice must have identical dimensions");

    eprintln!(
        "[e2e_render/determinism] psnr={:.3} dB ssim={:.6} mae={:.6} (identical => psnr=inf)",
        m.psnr_db, m.ssim, m.mean_abs_err
    );
    assert!(
        m.pass,
        "headless capture is NOT deterministic: PSNR {:.3} dB < {DETERMINISM_PSNR_DB} dB floor \
         (offline + FIXED_TIME must give pixel-stable frames)",
        m.psnr_db
    );
}

// ── View-distinctness: different poses must produce different renders ──────

#[test]
#[ignore = "requires a GPU (wgpu adapter); routed to cesiumrust-e2e.yml (xvfb + llvmpipe, --ignored). See docs/deferred.md#75"]
fn e2e_distinct_views_produce_distinct_renders() {
    let gd = globe_default();
    let pn = pole_north();
    let (_t1, png1) = run_capture(&gd, false);
    let (_t2, png2) = run_capture(&pn, false);

    let a = image::open(&png1).expect("globe_default decodes");
    let b = image::open(&png2).expect("pole_north decodes");
    let m = compare_images(&a, &b, DETERMINISM_PSNR_DB)
        .expect("both captures are 1280x720");

    eprintln!(
        "[e2e_render/distinctness] globe_default vs pole_north psnr={:.3} dB ssim={:.6}",
        m.psnr_db, m.ssim
    );
    // Different camera poses must NOT be identical: PSNR is finite and well
    // below the determinism floor, proving the render is driven by real scene
    // state rather than a static frame.
    assert!(
        m.psnr_db.is_finite() && m.psnr_db < DETERMINISM_PSNR_DB,
        "distinct views rendered identically (psnr={:.3}) — camera pose is not driving the capture",
        m.psnr_db
    );
}

// ── HDR option: CESIUM_HEADLESS_HDR=1 selects the Rgba16Float offscreen path ──

#[test]
#[ignore = "requires a GPU (wgpu adapter); routed to cesiumrust-e2e.yml (xvfb + llvmpipe, --ignored). See docs/deferred.md#75"]
fn e2e_hdr_capture_produces_valid_png() {
    let view = globe_default();
    // Exercises the M11.4 HDR (Rgba16Float) offscreen target end-to-end on the
    // GPU: the half-float buffer is read back and tonemapped to a decodable PNG.
    let (_tmp, png) = run_capture(&view, true);
    assert_valid_capture(&png, &format!("{}[hdr]", view.name));
}
