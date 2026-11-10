//! Tests for the GPU map backend: the camera math against the CPU renderer's own transform, a
//! headless device round-trip, and pixel parity between `rustgis_render`'s CPU frame and this
//! module's GPU frame for the same map and view.
//!
//! The parity tests skip themselves when no wgpu adapter exists (a CI box without a GPU); they
//! are the same comparison RustGIS's own `mapgpu` tests make, so a difference here is a real
//! rendering difference, not a font or driver artefact.

use super::*;

use wgpu30 as wgpu;

use std::sync::MutexGuard;

use rustgis_geom::Coord;
use rustgis_render::gpu_scene::{MeshVertex, PointInstance};

fn assert_matches_to_screen(cam: Camera, w: f64, h: f64, p: Coord, origin: [f64; 2]) {
    let m = view_proj(&cam, w, h, origin);
    let (vx, vy) = ((p.x - origin[0]) as f32, (p.y - origin[1]) as f32);
    let cx = f64::from(m[0][0] * vx + m[3][0]);
    let cy = f64::from(m[1][1] * vy + m[3][1]);
    let (gx, gy) = ((cx + 1.0) * 0.5 * w, (1.0 - cy) * 0.5 * h);
    let (ex, ey) = cam.to_screen(p, w, h);
    assert!((gx - ex).abs() < 0.01 && (gy - ey).abs() < 0.01, "gpu ({gx},{gy}) vs cpu ({ex},{ey})");
}

#[test]
fn matches_to_screen() {
    // World view, a city zoom, and a custom-projection's units (e.g. UTM metres).
    assert_matches_to_screen(Camera::world(), 1280.0, 720.0, Coord::new(2_611_702.0, 8_565_456.0), [0.0, 2_000_000.0]);
    let city = Camera { center: Coord::new(2_000_000.0, 6_000_000.0), res: 150.0, bearing: 0.0, pitch: 0.0 };
    assert_matches_to_screen(city, 1280.0, 720.0, Coord::new(2_013_400.0, 5_991_100.0), [2_000_000.0, 6_000_000.0]);
    let utm = Camera { center: Coord::new(500_000.0, 4_600_000.0), res: 25.0, bearing: 0.0, pitch: 0.0 };
    assert_matches_to_screen(utm, 800.0, 600.0, Coord::new(500_312.0, 4_599_511.0), [500_000.0, 4_600_000.0]);
}

#[test]
fn origin_off_center_shifts_clip() {
    // A vertex 100 units right of the origin, origin 1000 units right of the camera:
    // clip.x = (100 + 1000) * 2/(res*w) = 1100 * 0.002.
    let cam = Camera { center: Coord::new(0.0, 0.0), res: 10.0, bearing: 0.0, pitch: 0.0 };
    let m = view_proj(&cam, 100.0, 100.0, [1000.0, 0.0]);
    let clip_x = f64::from(m[0][0] * 100.0 + m[3][0]);
    assert!((clip_x - 2.2).abs() < 1e-5);
}

#[test]
fn marker_displacement_is_pixels_to_clip() {
    // 4 px radius at a 1280x720 viewport: 8 px of NDC width-2 -> 8/1280*2.
    let [dx, dy] = marker_displace([1.0, -1.0], 4.0, [1280.0, 720.0]);
    assert!((dx - 0.00625).abs() < 1e-6);
    assert!(dy > 0.0, "template y is screen-down, clip y is up");
}

#[test]
fn marker_templates_match_cpu_shapes() {
    let t = marker_templates();
    // Circle: 24 fan triangles = 72 vertices filling the slot, on the unit ring or centre.
    assert!(t[1][0].abs() + t[1][1].abs() > 0.9, "circle starts on the unit ring");
    // Square: 4 triangles = 12 vertices, then centre padding.
    let sq = &t[TEMPLATE_VERTS..TEMPLATE_VERTS + 12];
    assert!(sq.iter().any(|p| p[0] == 0.9 && p[1] == 0.9));
    assert!(t[TEMPLATE_VERTS + 12..2 * TEMPLATE_VERTS].iter().all(|p| *p == [0.0; 4]), "padding stays at the centre");
    // Star's top point (y-down): (0, -1.2).
    let star = &t[4 * TEMPLATE_VERTS..5 * TEMPLATE_VERTS];
    assert!(star.iter().any(|p| (p[1] + 1.2).abs() < 1e-6), "star top point");
}

/// The full camera matrix puts a point where `Camera::to_screen` does, turned and tilted, and
/// its clip depth leaves out what is behind the eye.
#[test]
fn view_proj_matches_turned_and_tilted_cameras() {
    let (w, h) = (800.0, 500.0);
    let center = Coord::new(1_000_000.0, 6_000_000.0);
    let origin = [center.x + 1234.5, center.y - 777.0];
    for (bearing, pitch) in [(30.0, 0.0), (-120.0, 0.0), (0.0, 60.0), (45.0, 75.0), (170.0, 85.0)] {
        let cam = Camera { center, res: 300.0, bearing, pitch };
        let m = view_proj(&cam, w, h, origin);
        let clip = |p: Coord| -> [f64; 4] {
            let v = [(p.x - origin[0]) as f32, (p.y - origin[1]) as f32, 0.0, 1.0];
            std::array::from_fn(|r| f64::from((0..4).map(|c| m[c][r] * v[c]).sum::<f32>()))
        };
        for (dx, dy) in [(0.0, 0.0), (40_000.0, 30_000.0), (-60_000.0, 90_000.0), (20_000.0, -50_000.0)] {
            let p = Coord::new(center.x + dx, center.y + dy);
            let c = clip(p);
            match cam.try_to_screen(p, w, h) {
                Some((sx, sy)) => {
                    assert!(c[2] >= 0.0 && c[2] <= c[3], "{bearing}/{pitch}: {c:?}");
                    let (gx, gy) = ((c[0] / c[3] + 1.0) / 2.0 * w, (1.0 - c[1] / c[3]) / 2.0 * h);
                    assert!((gx - sx).abs() < 0.05 && (gy - sy).abs() < 0.05, "{bearing}/{pitch}: ({gx}, {gy}) vs ({sx}, {sy})");
                }
                None => assert!(c[2] < 0.0, "{bearing}/{pitch}: behind the eye, clipped: {c:?}"),
            }
        }
    }
}

#[test]
fn commands_over_device_limits_are_declined() {
    let limits = wgpu::Limits { max_buffer_size: 1024, max_texture_dimension_2d: 2048, ..Default::default() };
    let mesh = |nv: usize, ni: usize| DrawCmd::Mesh {
        origin: [0.0, 0.0],
        vertices: vec![MeshVertex { position: [0.0, 0.0], normal: [0.0, 0.0], color: [0.0; 4], stroke_px: 0.0, _pad: 0.0 }; nv],
        indices: vec![0u32; ni],
    };
    assert!(cmd_fits(&limits, &mesh(4, 6)), "small mesh fits");
    assert!(!cmd_fits(&limits, &mesh(100, 6)), "a vertex buffer over max_buffer_size declines");
    assert!(!cmd_fits(&limits, &mesh(4, 300)), "an index buffer over max_buffer_size declines");
    assert!(
        !cmd_fits(&limits, &DrawCmd::Image { pixmap: rustgis_render::Pixmap::new(2049, 2).expect("pixmap") }),
        "an image over max_texture_dimension_2d declines"
    );
    assert!(
        cmd_fits(&limits, &DrawCmd::Image { pixmap: rustgis_render::Pixmap::new(2, 2).expect("pixmap") }),
        "a valid 2x2 image fits"
    );
}

/// GPU tests run one at a time: some drivers stall when a device is created concurrently.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A headless wgpu device, `None` on a host without any adapter (CI without a GPU skips the
/// GPU tests rather than failing).
fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rustgis_zed_gpu_test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults()
            .using_resolution(adapter.limits())
            .using_alignment(adapter.limits()),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .ok()
}

/// A scene of one triangle, one marker and one CPU image: enough to exercise every upload path
/// that does not need a tile cache.
fn tiny_scene() -> GpuScene {
    let v = |x: f32, y: f32| MeshVertex { position: [x, y], normal: [0.0, 0.0], color: [1.0, 0.0, 0.0, 1.0], stroke_px: 0.0, _pad: 0.0 };
    GpuScene {
        clear: [0.0, 0.0, 0.0, 1.0],
        cmds: vec![
            DrawCmd::Mesh { origin: [0.0, 0.0], vertices: vec![v(0.0, 0.0), v(100.0, 0.0), v(0.0, 100.0)], indices: vec![0, 1, 2] },
            DrawCmd::Points {
                origin: [0.0, 0.0],
                instances: vec![PointInstance { position: [10.0, 10.0], color: [0.0, 1.0, 0.0, 1.0], radius_px: 4.0, shape: 0 }],
            },
            DrawCmd::Image { pixmap: rustgis_render::Pixmap::new(1, 1).expect("pixmap") },
        ],
        labels: vec![],
        unplaced: vec![],
        attribution: vec![],
        tiles_pending: false,
        data_pending: false,
        tile_errors: vec![],
        partial: vec![],
        pending_in_place: vec![],
        tilt: None,
        clips: vec![],
    }
}

#[test]
fn gpu_map_uploads_a_scene_and_reads_it_back() {
    let _serial = one_at_a_time();
    let Some((device, queue)) = headless_device() else { return };
    let mut map = rustgis_map::Map::new("Map");
    let view = rustgis_render::View {
        width: 400.0,
        height: 300.0,
        pixels_per_point: 1.0,
        // One map unit per pixel: the 100-unit triangle is 100 px wide.
        camera: Camera { center: Coord::new(0.0, 0.0), res: 1.0, bearing: 0.0, pitch: 0.0 },
        sky: false,
    };
    let scene = tiny_scene();
    let mut gpu = GpuMap::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, (400, 300));
    assert!(gpu.prepare(&device, &queue, &mut map, &view, &scene, 1.0));
    let pixels = gpu.render(&device, &queue);
    assert_eq!(pixels.len(), 400 * 300 * 4, "a full RGBA frame");
    // The clear is black, the triangle is red: the read-back is not a blank or uninitialised
    // buffer.
    assert!(pixels.chunks_exact(4).any(|px| px[0] > 200 && px[1] < 40), "the red triangle was drawn");
    assert!(pixels.chunks_exact(4).any(|px| px[3] == 255), "opaque pixels came back");
}

/// A 2×2 view whose clear colour is known: read back in both formats, the channels are where the
/// host wants them (GPUI's atlas wants BGRA, so `Bgra8Unorm` needs no per-pixel swap in Zed).
#[test]
fn read_back_is_rgba_or_bgra_as_asked() {
    let _serial = one_at_a_time();
    let Some((device, queue)) = headless_device() else { return };
    let mut map = rustgis_map::Map::new("Map");
    let view = rustgis_render::View { width: 2.0, height: 2.0, pixels_per_point: 1.0, camera: Camera::world(), sky: false };
    let mut scene = tiny_scene();
    scene.clear = [0.25, 0.5, 0.75, 1.0];
    scene.cmds.clear();
    for (format, expected) in [
        (wgpu::TextureFormat::Rgba8Unorm, [64u8, 128, 191, 255]),
        (wgpu::TextureFormat::Bgra8Unorm, [191u8, 128, 64, 255]),
    ] {
        let mut gpu = GpuMap::new(&device, &queue, format, (2, 2));
        assert!(gpu.prepare(&device, &queue, &mut map, &view, &scene, 1.0));
        let pixels = gpu.render(&device, &queue);
        assert_eq!(pixels, expected.repeat(4), "{format:?}");
    }
}

/// A pan reuses the uploaded scene (`prepare_reusing`) and re-renders without a scene rebuild;
/// a resize does not.
#[test]
fn a_pan_reuses_the_scene_at_the_same_size() {
    let _serial = one_at_a_time();
    let Some((device, queue)) = headless_device() else { return };
    let mut map = rustgis_map::Map::new("Map");
    let mut view = rustgis_render::View { width: 320.0, height: 200.0, pixels_per_point: 1.0, camera: Camera::world(), sky: false };
    let mut gpu = GpuMap::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, (320, 200));
    // A CPU image run cannot re-project (the egui map view re-drew those frames too): the reuse
    // path is for pure-vector scenes, so drop the 1×1 image from the test scene.
    let mut scene = tiny_scene();
    scene.cmds.retain(|cmd| !matches!(cmd, DrawCmd::Image { .. }));
    assert!(gpu.prepare(&device, &queue, &mut map, &view, &scene, 1.0));
    let first = gpu.render(&device, &queue);
    assert_eq!(first.len(), 320 * 200 * 4);

    // A zoom within the range a re-projected scene covers: no new scene (a size change is not).
    let mut zoomed = view;
    zoomed.camera.res /= 1.2;
    assert!(gpu.prepare_reusing(&queue, &zoomed, 1.0), "a 1.2x zoom stays within the scene's reach");
    let second = gpu.render(&device, &queue);
    assert_eq!(second.len(), first.len());
    assert_ne!(first, second, "the moved camera re-projects the triangle");

    view.width = 640.0;
    assert!(!gpu.prepare_reusing(&queue, &view, 1.0), "a new size needs a new target");
}

/// The device-pixel size is the logical view times the DPI, so 125 % scaling renders a
/// 1200×800 map at 1500×1000 pixels (the point of the GPU path).
#[test]
fn a_frame_is_drawn_at_device_pixels() {
    let _serial = one_at_a_time();
    let Some((device, queue)) = headless_device() else { return };
    let mut map = rustgis_map::Map::new("Map");
    let view = rustgis_render::View { width: 1200.0, height: 800.0, pixels_per_point: 1.25, camera: Camera::world(), sky: false };
    let mut gpu = GpuMap::new(&device, &queue, wgpu::TextureFormat::Bgra8Unorm, (1200, 800));
    assert!(gpu.prepare(&device, &queue, &mut map, &view, &tiny_scene(), 1.25));
    assert_eq!(gpu.size(), (1500, 1000));
    assert_eq!(gpu.render(&device, &queue).len(), 1500 * 1000 * 4);
}

// ---- CPU vs GPU pixel parity on the world sample ----

fn world_view() -> rustgis_render::View {
    rustgis_render::View {
        width: 640.0,
        height: 400.0,
        pixels_per_point: 1.0,
        camera: Camera::world(),
        sky: false,
    }
}

fn world_map() -> rustgis_map::Map {
    let mut m = rustgis_map::Map::new("Map");
    for (name, text) in [
        ("Countries", rustgis_render::sample_data::COUNTRIES),
        ("Rivers", rustgis_render::sample_data::RIVERS),
        ("Lakes", rustgis_render::sample_data::LAKES),
        ("Cities", rustgis_render::sample_data::CITIES),
    ] {
        if let Some(d) = rustgis_render::world_dataset(name, text) {
            m.add_dataset(d);
        }
    }
    m
}

/// Mean per-byte absolute difference and the fraction of pixels whose worst channel differs by
/// more than 16.
fn diff_stats(a: &[u8], b: &[u8]) -> (f64, f64) {
    assert_eq!(a.len(), b.len());
    let mut sum = 0u64;
    let mut over = 0usize;
    for (px, py) in a.chunks(4).zip(b.chunks(4)) {
        let mut worst = 0u8;
        for c in 0..4 {
            let d = px[c].abs_diff(py[c]);
            sum += u64::from(d);
            worst = worst.max(d);
        }
        if worst > 16 {
            over += 1;
        }
    }
    (sum as f64 / a.len() as f64, over as f64 / (a.len() / 4) as f64)
}

/// Pixels differing by >16 whose 3×3 neighbourhood in the CPU render is FLAT (max − min < 4 per
/// channel). Edge pixels legitimately differ between analytic AA (CPU) and 4× MSAA (GPU); a
/// mismatch in a flat area is a structural bug (wrong colour, missing content, bad z-order).
fn structural_outliers(cpu: &[u8], gpu: &[u8], w: usize, h: usize) -> usize {
    let mut out = 0;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let worst = (0..4).map(|c| cpu[i + c].abs_diff(gpu[i + c])).max().unwrap_or(0);
            if worst <= 16 {
                continue;
            }
            let mut lo = [255u8; 4];
            let mut hi = [0u8; 4];
            for dy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for dx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = (dy * w + dx) * 4;
                    for c in 0..4 {
                        lo[c] = lo[c].min(cpu[j + c]);
                        hi[c] = hi[c].max(cpu[j + c]);
                    }
                }
            }
            if (0..4).all(|c| hi[c] - lo[c] < 4) {
                out += 1;
            }
        }
    }
    out
}

/// Render `map` through the GPU at `view` and return straight premultiplied RGBA, plus the
/// scene's labels, or `None` without an adapter.
fn gpu_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    map: &mut rustgis_map::Map,
    view: &rustgis_render::View,
) -> Option<(Vec<u8>, Vec<rustgis_render::PlacedLabel>)> {
    let Some(rustgis_render::RenderOutput::Gpu(scene)) =
        rustgis_render::render_backend(map, view, rustgis_render::Backend::Gpu)
    else {
        return None;
    };
    let (w, h) = view.pixel_size();
    let mut gpu = GpuMap::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, (w, h));
    if !gpu.prepare(device, queue, map, view, &scene, view.pixels_per_point as f32) {
        return None;
    }
    Some((gpu.render(device, queue), scene.labels))
}

/// Write `name-{cpu,gpu,diff}.png` into `target/rustgis-gpu/` for eyeballing, as RustGIS's
/// `PARITY_DUMP` does.
fn dump(name: &str, cpu: &[u8], gpu: &[u8], w: u32, h: u32) {
    let dir = std::path::Path::new("target/rustgis-gpu");
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let save = |suffix: &str, data: &[u8]| {
        let path = dir.join(format!("{name}-{suffix}.png"));
        let _ = image::save_buffer(&path, data, w, h, image::ColorType::Rgba8);
    };
    save("cpu", cpu);
    save("gpu", gpu);
    let mut diff: Vec<u8> = cpu.iter().zip(gpu).map(|(a, b)| a.abs_diff(*b).saturating_mul(8)).collect();
    for px in diff.chunks_exact_mut(4) {
        px[3] = 255;
    }
    save("diff", &diff);
}

fn assert_parity(name: &str, build: impl Fn() -> rustgis_map::Map, view: &rustgis_render::View) {
    let _serial = one_at_a_time();
    let Some((device, queue)) = headless_device() else { return };
    let gpu = {
        let mut gm = build();
        let Some((pixels, _labels)) = gpu_frame(&device, &queue, &mut gm, view) else { return };
        pixels
    };
    let mut cm = build();
    let cpu = rustgis_render::render(&mut cm, view).expect("the CPU renderer draws the world sample");
    let (w, h) = view.pixel_size();
    dump(name, cpu.pixmap.data(), &gpu, w, h);
    let (mean_raw, over) = diff_stats(cpu.pixmap.data(), &gpu);
    let structural = structural_outliers(cpu.pixmap.data(), &gpu, w as usize, h as usize);
    // Same thresholds RustGIS's own `mapgpu` parity tests hold: the same image overall, and no
    // structural differences away from antialiased edges.
    // Upstream's threshold (mean and no structural mismatch): antialiased edges legitimately
    // differ between analytic AA and 4x MSAA, so the fraction over 16 is reported, not bounded.
    assert!(mean_raw < 1.5 && structural == 0, "{name}: raw mean {mean_raw:.2}/255, {over:.3} pixels over 16, {structural} structural outliers");
}

#[test]
fn parity_world_sample() {
    assert_parity("world-sample", world_map, &world_view());
}

#[test]
fn parity_world_sample_at_125_percent() {
    // What Zed draws on the test machine: 640 logical points at 1.25 device pixels per point.
    let view = rustgis_render::View { pixels_per_point: 1.25, ..world_view() };
    assert_parity("world-sample-125", world_map, &view);
}

#[test]
fn parity_midnight_world_sample() {
    assert_parity(
        "midnight",
        || {
            let mut m = world_map();
            m.basemap = rustgis_map::Basemap::Midnight;
            m
        },
        &world_view(),
    );
}

#[test]
fn parity_turned_world_sample() {
    let view = rustgis_render::View { camera: Camera { bearing: 35.0, ..Camera::world() }, ..world_view() };
    assert_parity("turned", world_map, &view);
}

#[test]
fn parity_markers_all_shapes() {
    use rustgis_data::{Dataset, Geometry, GeometryType};
    use rustgis_map::{Marker, Renderer, Rgba, Symbol};
    use rustgis_render::gpu_scene::DrawCmd as Cmd;
    assert_parity(
        "markers",
        || {
            let mut m = rustgis_map::Map::new("Map");
            for (i, marker) in Marker::ALL.into_iter().enumerate() {
                let mut d = Dataset::new(format!("p{i}"), GeometryType::Point, vec![]);
                d.push(Some(Geometry::Point(Coord::new(i as f64 * 25.0 - 50.0, 20.0))), vec![]);
                let id = m.add_dataset(d);
                if let Some(l) = m.layer_mut(id) {
                    l.renderer = Renderer::Single(Symbol {
                        fill: Rgba::rgb(0xE0, 0x7B, 0x39),
                        stroke: Rgba::rgb(255, 255, 255),
                        stroke_width: 1.5,
                        size: 12.0,
                        marker,
                    });
                }
            }
            let _ = Cmd::Points { origin: [0.0, 0.0], instances: vec![] };
            m
        },
        &world_view(),
    );
}
