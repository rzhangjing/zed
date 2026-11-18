//! The windowless EW GIS viewer: scene state, input, and frame rendering.
//!
//! A [`Viewer`] owns everything the standalone `winit` binary and the embedded
//! Zed GPUI window both need:
//!
//! * a GPU device — headless, with no swapchain surface
//!   ([`GpuContext::new_headless`]);
//! * the tile atlas, the 2D plane, the 3D globe, the icon overlay, the vector
//!   plot and the optional MVT overlay;
//! * the renderer, one offscreen color target and the depth buffer;
//! * the 2D/3D cameras and the current [`SceneMode`].
//!
//! It renders in one of two ways:
//!
//! * [`Viewer::render_to_view`] records a frame into a caller-supplied color
//!   view — the standalone window's swapchain texture — without reading it
//!   back;
//! * [`Viewer::render_rgba`] renders into the owned offscreen target and copies
//!   the pixels to the CPU, which is how a host that has no surface of its own
//!   (the Zed window) gets its frames.
//!
//! [`Viewer::update`] is the single per-frame scene update both paths call, so
//! the standalone binary and the embedded window cannot drift apart. Input
//! methods ([`Viewer::mouse_move`], [`Viewer::mouse_down`], [`Viewer::wheel`],
//! [`Viewer::key_down`]) are the same ones the `winit` host forwards to.
//!
//! Nothing here starts `winit` or creates a window; the `winit` dependency is
//! reachable through [`crate::gpu::GpuContext::new`] only, which a host that
//! embeds the viewer never calls.

use std::path::PathBuf;
use std::time::Instant;

use crate::data;
use crate::data::mvt::MvtSource;
use crate::gpu::GpuContext;
use crate::gpu::frame::OffscreenTarget;
use crate::math::camera::{GlobeCamera, PlaneCamera};
use crate::math::geo::TileId;
use crate::render::Renderer;
use crate::scene::SceneMode;
use crate::scene::globe::GlobeScene;
use crate::scene::icons::{ICON_COUNT, IconDemo};
use crate::scene::mvt_layer::{self, MvtLayer};
use crate::scene::pick::{self, ScreenProjection};
use crate::scene::plane::PlaneScene;
use crate::scene::plot::{
    DemoShapes, PlotDocument, PlotScene, PlotStats,
};
use crate::screenshot::{self, FrameRgba};
use crate::tiles::TileSystem;
use crate::tiles::provider::{ImageryOptions, ProviderConfig, TileSource};

const TITLE_PREFIX: &str = "EW GIS";

/// How a [`Viewer`] is built.
///
/// The defaults are the embedded window's: a 1280x720 2D plane with the
/// built-in demo shapes. The standalone host fills every field from its
/// [`crate::app::Config`], so the command line and the environment keep working
/// there.
#[derive(Clone, Debug)]
pub struct ViewerOptions {
    pub width: u32,
    pub height: u32,
    pub mode: SceneMode,
    /// Include the built-in demo shapes (`--no-demo` clears this).
    pub demo: bool,
    /// Imagery overrides from the command line; environment values are applied
    /// first by `ProviderConfig::from_env`.
    pub imagery: ImageryOptions,
    pub tile_source: Option<TileSource>,
    /// Shapefiles imported at startup.
    pub shapefiles: Vec<PathBuf>,
    /// GeoJSON files imported at startup.
    pub geojson: Vec<PathBuf>,
    /// MVT vector tile URL template.
    pub mvt_url: Option<String>,
    /// MVT local tile root.
    pub mvt_dir: Option<PathBuf>,
    pub mvt_max_tiles: usize,
    /// Scripted wheel zoom applied before the first frame, in wheel lines.
    pub zoom: i32,
}

impl Default for ViewerOptions {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            mode: SceneMode::Plane,
            demo: true,
            imagery: ImageryOptions::default(),
            tile_source: None,
            shapefiles: Vec::new(),
            geojson: Vec::new(),
            mvt_url: None,
            mvt_dir: None,
            mvt_max_tiles: mvt_layer::DEFAULT_MAX_TILES,
            zoom: 0,
        }
    }
}

/// A key the viewer understands. The host translates its own key events into
/// these; only the bindings the standalone window documents are here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerKey {
    /// Toggle 2D/3D.
    Tab,
    /// Switch to the 2D plane (no-op when already there).
    Plane,
    /// Switch to the 3D globe (no-op when already there).
    Globe,
    /// Delete/Backspace: delete the selected plot shape.
    Delete,
    /// Ctrl+Z: undo the most recent plot edit.
    Undo,
    /// Esc: clear the plot selection.
    Escape,
}

/// The windowless EW GIS scene.
pub struct Viewer {
    pub gpu: GpuContext,
    pub tiles: TileSystem,
    pub plane: PlaneScene,
    pub globe: GlobeScene,
    pub plot: PlotScene,
    pub icons: IconDemo,
    pub mvt: Option<MvtLayer>,
    pub renderer: Renderer,
    /// Offscreen target: the color attachment for [`Viewer::render_rgba`] and
    /// the depth attachment for both render paths.
    pub target: OffscreenTarget,
    pub plane_camera: PlaneCamera,
    pub globe_camera: GlobeCamera,
    pub mode: SceneMode,
    /// Static plot shapes (demo + imports) the MVT overlay is merged into.
    pub base_shapes: DemoShapes,
    /// Last cursor position, in the viewer's own pixel coordinates.
    cursor: Option<[f32; 2]>,
    /// Left button held: the camera pans/orbits unless a shape is grabbed.
    dragging: bool,
    started: Instant,
    frames: u64,
}

impl Viewer {
    /// Build a headless viewer: no window, no swapchain surface. This is the
    /// constructor an embedding host uses, and the only one allowed to run on a
    /// worker thread.
    ///
    /// Blocking: this creates a wgpu adapter/device and cannot run on a UI
    /// executor thread.
    pub fn new(options: ViewerOptions) -> Result<Self, String> {
        let gpu = GpuContext::new_headless(options.width.max(1), options.height.max(1))
            .map_err(|error| format!("create headless GPU context: {error}"))?;
        Self::with_gpu(gpu, options)
    }

    /// Build a viewer around an existing GPU context. The standalone `winit`
    /// host creates a surface-backed context first and hands it over, so the
    /// scene setup is shared with [`Viewer::new`] instead of duplicated.
    pub fn with_gpu(gpu: GpuContext, options: ViewerOptions) -> Result<Self, String> {
        let width = gpu.width.max(1);
        let height = gpu.height.max(1);

        let mut provider_config = ProviderConfig::from_env();
        // CLI imagery options apply on top of the environment, then an explicit
        // `--online/--offline/--procedural` chooses the network policy.
        options.imagery.apply(&mut provider_config);
        if let Some(source) = options.tile_source {
            provider_config = provider_config.with_source(source);
        }
        // Validate the merged env + CLI imagery only now: a CLI flag may have
        // supplied the URL/layers the environment omitted.
        provider_config.resolve_imagery();

        let tiles = TileSystem::new(&gpu, provider_config);
        let icons = IconDemo::new(&gpu);
        let globe = GlobeScene::new(&gpu);

        // MVT: the URL template and/or the local tile root. Both are optional
        // and an unset pair leaves the layer off entirely.
        let mvt_source = Some(MvtSource::new(
            options.mvt_url.clone().unwrap_or_default(),
            options.mvt_dir.clone().unwrap_or_default(),
        ))
        .filter(MvtSource::is_configured);

        let base_shapes = load_plot_shapes(&options);
        let has_imports = !options.geojson.is_empty() || !options.shapefiles.is_empty();
        // A live MVT overlay needs a document the viewer can replace per tile
        // change, so it takes the explicit-document path even with no imports.
        let plot = if has_imports || !options.demo || mvt_source.is_some() {
            PlotScene::with_document(
                &gpu,
                PlotDocument::new(base_shapes.clone()),
                options.mode,
                globe.ellipsoid(),
            )
        } else {
            // Default run: the built-in demo document, unchanged.
            PlotScene::new(&gpu, options.mode, globe.ellipsoid())
        };
        let mvt = mvt_source.map(|source| {
            let layer = MvtLayer::new(source, options.mvt_max_tiles);
            log::info!(
                "MVT vector layer: {} (max {} resident/visible tiles, {} worker thread(s))",
                layer.source_label(),
                layer.max_tiles(),
                std::thread::available_parallelism()
                    .map(|count| count.get())
                    .unwrap_or(2)
                    .clamp(1, 4)
            );
            layer
        });

        let renderer = Renderer::new(&gpu, &tiles, &icons);
        let target = OffscreenTarget::new(&gpu.device, width, height, gpu.format);

        let mut viewer = Self {
            gpu,
            tiles,
            plane: PlaneScene::default(),
            globe,
            plot,
            icons,
            mvt,
            renderer,
            target,
            plane_camera: PlaneCamera::default(),
            globe_camera: GlobeCamera::default(),
            mode: options.mode,
            base_shapes,
            cursor: None,
            dragging: false,
            started: Instant::now(),
            frames: 0,
        };

        if options.zoom != 0 {
            // Scripted wheel zoom for acceptance runs, applied before the first
            // frame so the screenshot and the picks share one camera state.
            let steps = options.zoom as f32;
            match viewer.mode {
                SceneMode::Plane => viewer.plane_camera.zoom_wheel(steps),
                SceneMode::Globe => viewer.globe_camera.zoom_wheel(steps),
            }
            log::info!(
                "scripted zoom {:+} -> 2D height {:.0} m, 3D distance {:.0} m",
                options.zoom,
                viewer.plane_camera.height,
                viewer.globe_camera.distance
            );
        }

        log::info!(
            "viewer ready: mode {}, {}x{}, icon instances {}, draw calls {} (base + plot fill/line/point + icons); \
             globe LOD {} ({} vertex / {} index buffer slots); vector plot {} fill triangles, {} line segments, {} point markers; \
             plot document: {} shapes",
            viewer.mode.label(),
            viewer.gpu.width,
            viewer.gpu.height,
            ICON_COUNT,
            Renderer::draw_calls(viewer.mode),
            viewer.globe.config.label(),
            GlobeScene::max_vertices(),
            GlobeScene::max_indices(),
            viewer.plot.stats.fill_triangles,
            viewer.plot.stats.segments,
            viewer.plot.stats.points,
            viewer.plot.document().len(),
        );

        Ok(viewer)
    }

    /// Resize the GPU context (surface-sized when there is a surface) and
    /// reallocate the offscreen target.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.gpu.width
            && height == self.gpu.height
            && width == self.target.width
            && height == self.target.height
        {
            return;
        }
        self.gpu.resize(width, height);
        self.target = OffscreenTarget::new(&self.gpu.device, width, height, self.gpu.format);
    }

    /// Switch to `mode`. A no-op when it is already active; the camera state is
    /// carried across so Tab feels like a viewpoint change, not a reset.
    pub fn set_mode(&mut self, mode: SceneMode) {
        if self.mode == mode {
            return;
        }
        // A mode switch cancels the zoom/pan feel of an in-flight shape drag;
        // commit whatever moved so it still becomes exactly one undo step.
        if let Some(id) = self.plot.end_drag() {
            log::info!(
                "plot: drag of shape {} committed by the mode switch",
                id.raw()
            );
        }
        match mode {
            SceneMode::Globe => {
                self.globe_camera = GlobeCamera::from_plane(&self.plane_camera);
            }
            SceneMode::Plane => {
                self.globe_camera.apply_to_plane(&mut self.plane_camera);
            }
        }
        self.mode = mode;
        log::info!(
            "switched to {} (center {:.3}, {:.3})",
            self.mode.label(),
            self.plane_camera.center_lon,
            self.plane_camera.center_lat
        );
    }

    pub fn toggle_mode(&mut self) {
        self.set_mode(self.mode.next());
    }

    pub fn mode(&self) -> SceneMode {
        self.mode
    }

    /// The world → pixel mapping for the active mode, using exactly the camera
    /// state the next frame will render with.
    pub fn projection(&self) -> Option<ScreenProjection> {
        let ellipsoid = self.globe.ellipsoid();
        let screen = [self.gpu.width as f32, self.gpu.height as f32];
        Some(match self.mode {
            SceneMode::Plane => ScreenProjection::plane(
                self.plot.anchor_plane,
                &self.plane_camera,
                self.gpu.aspect(),
                screen,
            ),
            SceneMode::Globe => {
                ScreenProjection::globe(&self.globe_camera, ellipsoid, self.gpu.aspect(), screen)
            }
        })
    }

    /// Cursor motion: a grabbed shape follows the pointer, otherwise the camera
    /// pans/orbits.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        let cursor = [x, y];
        let anchor = self.plot.document().drag_anchor();
        if let Some(anchor) = anchor {
            let delta = self
                .projection()
                .and_then(|projection| projection.drag_delta_from(anchor, cursor));
            if let Some(delta) = delta {
                self.plot.drag_to(delta);
            }
        } else if self.dragging
            && let Some(previous) = self.cursor
        {
            let dx = (x - previous[0]) as f64;
            let dy = (y - previous[1]) as f64;
            match self.mode {
                SceneMode::Plane => {
                    self.plane_camera
                        .pan_pixels(dx, dy, self.gpu.height as f64);
                }
                SceneMode::Globe => {
                    self.globe_camera.orbit_pixels(dx, dy);
                }
            }
        }
        self.cursor = Some(cursor);
    }

    /// Left button down at `(x, y)`: pick a shape and start dragging it;
    /// otherwise behave exactly like the camera drag.
    pub fn mouse_down(&mut self, x: f32, y: f32) {
        self.mouse_move(x, y);
        self.press_plot([x, y]);
        self.dragging = true;
    }

    /// Left button up: finish a shape drag, committing one undo step when it
    /// actually moved.
    pub fn mouse_up(&mut self) {
        self.dragging = false;
        self.release_plot();
    }

    /// Wheel scroll, in lines; positive zooms in.
    pub fn wheel(&mut self, lines: f32) {
        match self.mode {
            SceneMode::Plane => self.plane_camera.zoom_wheel(lines),
            SceneMode::Globe => self.globe_camera.zoom_wheel(lines),
        }
    }

    /// Handle one key. Returns whether it did something: `false` means the host
    /// may apply its own fallback (the standalone window quits on an Esc that
    /// had no selection to clear).
    pub fn key_down(&mut self, key: ViewerKey) -> bool {
        match key {
            ViewerKey::Tab => {
                self.toggle_mode();
                true
            }
            ViewerKey::Plane => {
                self.set_mode(SceneMode::Plane);
                true
            }
            ViewerKey::Globe => {
                self.set_mode(SceneMode::Globe);
                true
            }
            ViewerKey::Delete => {
                self.delete_selected_plot();
                true
            }
            ViewerKey::Undo => {
                self.undo_plot();
                true
            }
            ViewerKey::Escape => self.clear_plot_selection(),
        }
    }

    /// Esc: clear the plot selection. `true` when something was selected.
    pub fn clear_plot_selection(&mut self) -> bool {
        self.plot.select(None)
    }

    /// Update the scene for one frame: upload ready tiles, update the visible
    /// set, the globe mesh, the icon instances, the MVT overlay and the plot
    /// batches, and write the camera uniforms.
    ///
    /// `time` is seconds since the viewer started; it drives the icon animation.
    /// Both render paths call this once per frame.
    pub fn update(&mut self, time: f32) {
        self.frames += 1;
        let gpu = &self.gpu;
        self.tiles.upload_ready(gpu);
        let screen = [gpu.width as f32, gpu.height as f32];
        match self.mode {
            SceneMode::Plane => {
                self.plane.update(
                    &self.plane_camera,
                    &mut self.tiles,
                    gpu.aspect(),
                    gpu.height as f64,
                );
                self.renderer.upload_tiles(gpu, &self.plane);
                self.icons.update(gpu, &self.plane_camera, time);
                self.renderer.set_icon_count(ICON_COUNT);
                let view_proj = self.plane_camera.view_proj(gpu.aspect());
                self.renderer
                    .set_plane_camera(gpu, view_proj, self.plane_camera.eye());
                // MVT: the plane scene publishes its visible tile ids; a changed
                // overlay replaces the document, and `plot.update` below rebuilds
                // the three batches in the same frame.
                if let Some(mvt) = self.mvt.as_mut()
                    && mvt.update(self.plane.visible_ids())
                {
                    sync_mvt_plot(mvt, &self.base_shapes, &mut self.plot);
                }
                // The plot geometry is anchored, not camera relative, so the
                // rebuild is a no-op unless the mode changed; only the shared
                // plot uniform is rewritten per frame.
                self.plot.update(gpu, SceneMode::Plane, self.globe.ellipsoid());
                let plot_camera = self.plot.camera_plane(&self.plane_camera, gpu.aspect(), screen);
                self.renderer.set_plot_camera(gpu, &plot_camera);
            }
            SceneMode::Globe => {
                // The view and the LOD traversal both use the ellipsoid from the
                // scene; the uniform gets the `f64` product rounded to `f32` once
                // (see `GlobeView`), never an `f32` eye/target difference.
                let view = self.globe_camera.view(gpu.aspect(), self.globe.ellipsoid());
                self.globe.update(
                    gpu,
                    &self.globe_camera,
                    &mut self.tiles,
                    gpu.aspect(),
                    gpu.height as f64,
                );
                // Globe-anchored icons: tangent frame + three-axis local rotation.
                self.icons.update_globe(
                    gpu,
                    &self.globe_camera,
                    self.globe.ellipsoid(),
                    gpu.height as f64,
                    time,
                );
                self.renderer.set_icon_count(ICON_COUNT);
                self.renderer.set_globe_camera(
                    gpu,
                    view.view_proj_f32(),
                    view.eye_relative,
                );
                // 3D is optional for MVT: the quadtree leaves are the visible set.
                // Their count is bounded by `--mvt-max-tiles` before requesting.
                if let Some(mvt) = self.mvt.as_mut() {
                    let desired: Vec<TileId> =
                        self.globe.tiles.iter().map(|tile| tile.id).collect();
                    if mvt.update(&desired) {
                        sync_mvt_plot(mvt, &self.base_shapes, &mut self.plot);
                    }
                }
                self.plot.update(gpu, SceneMode::Globe, self.globe.ellipsoid());
                let plot_camera = self.plot.camera_globe(&view, screen);
                self.renderer.set_plot_camera(gpu, &plot_camera);
            }
        }
    }

    /// Render the current scene into `color_view` (a surface or any other
    /// render attachment) using the viewer's depth buffer. Submits the command
    /// buffer; the caller presents.
    pub fn render_to_view(&self, color_view: &wgpu::TextureView) {
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ew_gis frame encoder"),
            });
        self.renderer.render(
            &mut encoder,
            color_view,
            &self.target.depth.view,
            self.mode,
            &self.plot,
            &self.icons,
            &self.globe,
        );
        self.gpu.queue.submit(Some(encoder.finish()));
    }

    /// Advance the scene one frame and render it offscreen, returning the pixels
    /// as tightly packed RGBA8.
    ///
    /// This blocks on the GPU readback (`map_async` + `device.poll`), so call it
    /// from a worker thread, never from a UI executor.
    pub fn render_rgba(&mut self) -> Result<FrameRgba, String> {
        let time = self.started.elapsed().as_secs_f32();
        self.update(time);
        screenshot::render_rgba_into(
            &self.gpu,
            &self.target,
            &self.renderer,
            &self.plot,
            &self.icons,
            &self.globe,
            self.mode,
        )
        .map_err(|error| format!("render offscreen frame: {error}"))
    }

    /// Frames rendered through [`Viewer::update`].
    pub fn frames(&self) -> u64 {
        self.frames
    }

    // ── HUD fragments ────────────────────────────────────────────────────────
    // The standalone window's title bar is the viewer's only status display.
    // They live here (rather than in `app`) because they read the scene state
    // the viewer owns; a host that wants a status line can call `hud` too.

    pub fn plot_stats(&self) -> PlotStats {
        self.plot.stats
    }

    /// HUD fragment for the plot document selection.
    pub fn plot_hud(&self) -> String {
        let document = self.plot.document();
        match (document.selected(), document.selected_ref()) {
            (Some(id), Some(at)) => format!(
                "sel {}#{} @{} undo {}",
                at.kind.label(),
                id.raw(),
                document.ordinal_of(id).unwrap_or(usize::MAX),
                document.undo_depth()
            ),
            _ => format!(
                "sel - | {} shapes | undo {}",
                document.len(),
                document.undo_depth()
            ),
        }
    }

    /// HUD fragment for the MVT overlay (empty when no MVT source is
    /// configured).
    pub fn mvt_hud(&self) -> String {
        let Some(mvt) = self.mvt.as_ref() else {
            return String::new();
        };
        let stats = mvt.stats;
        if stats.failed_tiles > 0 {
            format!(
                " | mvt {}/{}t {}f {}err",
                stats.resident_tiles, stats.desired_tiles, stats.features, stats.failed_tiles
            )
        } else {
            format!(
                " | mvt {}/{}t {}f",
                stats.resident_tiles, stats.desired_tiles, stats.features
            )
        }
    }

    /// Exit-summary fragment for the MVT overlay.
    pub fn mvt_summary(&self) -> String {
        let Some(mvt) = self.mvt.as_ref() else {
            return "mvt: off".to_string();
        };
        let stats = mvt.stats;
        format!(
            "mvt: tiles={} desired={} pending={} failed={} features={} layers={} skipped={} holes={} \
             merged_shapes={} local={} online={} requests={} errors={} evictions={} rebuilds={} \
             merge={:.3} ms update={:.3} ms",
            stats.resident_tiles,
            stats.desired_tiles,
            stats.pending_tiles,
            stats.failed_tiles,
            stats.features,
            stats.layers,
            stats.skipped,
            stats.holes,
            stats.merged_shapes,
            stats.local_tiles,
            stats.online_tiles,
            stats.requests,
            stats.errors,
            stats.evictions,
            stats.rebuilds,
            stats.merge_ms,
            stats.last_update_ms
        )
    }

    /// The one-line status string the standalone window puts in its title bar.
    pub fn hud(&self, fps: f32, frame_ms: f32) -> String {
        let tiles = self.tiles.resident_tiles();
        let layers = self.tiles.used_layers();
        let plot = self.plot_stats();
        let selection = self.plot_hud();
        let mvt = self.mvt_hud();
        match self.mode {
            SceneMode::Plane => format!(
                "{TITLE_PREFIX} | 2D Plane | {} icons | plot {} tri/{} seg/{} pts | {} | {:.1} FPS | {:.2} ms | z{} | {} tiles | {}/64 layers{mvt}",
                ICON_COUNT,
                plot.fill_triangles,
                plot.segments,
                plot.points,
                selection,
                fps,
                frame_ms,
                self.plane.stats.target_z,
                tiles,
                layers
            ),
            SceneMode::Globe => {
                let globe = self.globe.stats;
                format!(
                    "{TITLE_PREFIX} | 3D Globe LOD | plot {} tri/{} seg/{} pts | {} | {:.1} FPS | {:.2} ms | {} tiles ({} exact/{} fallback) z{} | {} verts | {}/64 layers | rebuild {:.2} ms{mvt}",
                    plot.fill_triangles,
                    plot.segments,
                    plot.points,
                    selection,
                    fps,
                    frame_ms,
                    globe.visible_tiles,
                    globe.exact_tiles,
                    globe.fallback_tiles,
                    globe.max_depth,
                    globe.vertices,
                    layers,
                    globe.rebuild_ms
                )
            }
        }
    }

    // ── Plot editing internals ───────────────────────────────────────────────

    /// Left-button press: pick a shape and start dragging it; otherwise behave
    /// exactly like the old camera drag.
    fn press_plot(&mut self, cursor: [f32; 2]) {
        let Some(projection) = self.projection() else {
            return;
        };
        let Some(hit) = pick::pick(self.plot.document(), &projection, cursor) else {
            if self.plot.select(None) {
                log::info!("plot: selection cleared (clicked empty space)");
            }
            return;
        };
        let ordinal = self.plot.document().ordinal_of(hit.id);
        self.plot.select(Some(hit.id));
        match projection.pixel_to_lonlat(cursor) {
            Some(anchor) if self.plot.begin_drag(hit.id, anchor) => {
                log::info!(
                    "plot: picked {} #{} (ordinal {}) at {:.1},{:.1} px, {:.1} px from the cursor; \
                     drag from {:.4},{:.4}",
                    hit.kind.label(),
                    hit.id.raw(),
                    ordinal.unwrap_or(usize::MAX),
                    cursor[0],
                    cursor[1],
                    hit.distance,
                    anchor[0],
                    anchor[1]
                );
            }
            _ => log::warn!(
                "plot: selected {} #{} but could not resolve a drag anchor at {:.1},{:.1} px",
                hit.kind.label(),
                hit.id.raw(),
                cursor[0],
                cursor[1]
            ),
        }
    }

    /// Finish a shape drag, committing one undo step when it actually moved.
    fn release_plot(&mut self) {
        let Some(id) = self.plot.end_drag() else {
            return;
        };
        let document = self.plot.document();
        let label = document
            .locate(id)
            .map(|at| at.kind.label())
            .unwrap_or("shape");
        log::info!(
            "plot: drag committed, {} #{} moved; {} edits, {} undo steps",
            label,
            id.raw(),
            document.edits(),
            document.undo_depth()
        );
    }

    fn delete_selected_plot(&mut self) {
        let before = self.plot.document().len();
        match self.plot.delete_selected() {
            Some(id) => log::info!(
                "plot: deleted shape {} ({} -> {} shapes, {} undo steps)",
                id.raw(),
                before,
                self.plot.document().len(),
                self.plot.document().undo_depth()
            ),
            None => log::info!("plot: Delete ignored, no shape is selected"),
        }
    }

    fn undo_plot(&mut self) {
        if self.plot.undo() {
            log::info!(
                "plot: undo -> {} shapes, {} undo steps left",
                self.plot.document().len(),
                self.plot.document().undo_depth()
            );
        } else {
            log::info!("plot: Ctrl+Z ignored, nothing to undo");
        }
    }
}

/// Build the static plot shapes from the built-in demo group plus every
/// GeoJSON/Shapefile import. A missing or broken file is logged and skipped:
/// the map still starts.
///
/// The shapes are kept because the MVT overlay merges into a fresh document
/// whenever its visible tile set changes.
fn load_plot_shapes(options: &ViewerOptions) -> DemoShapes {
    let started = Instant::now();
    let mut shapes = if options.demo {
        DemoShapes::demo()
    } else {
        DemoShapes::empty()
    };
    let mut imported = data::LoadedShapes::default();
    let paths: Vec<&PathBuf> = options
        .geojson
        .iter()
        .chain(options.shapefiles.iter())
        .collect();
    for path in &paths {
        // The loader dispatches on the extension, so `--shape` and `--geojson`
        // both go through one path and share its error text.
        match data::load_file(path) {
            Ok(loaded) => {
                log::info!("vector {}: {}", path.display(), loaded.summary());
                imported.merge(loaded);
            }
            Err(error) => log::error!("vector import failed: {error}"),
        }
    }
    for warning in &imported.warnings {
        log::warn!("vector import: {warning}");
    }
    let imported_features = imported.feature_count();
    let imported_counts = imported.shapes.counts();
    let files = paths.len();
    shapes.points.extend(imported.shapes.points);
    shapes.polylines.extend(imported.shapes.polylines);
    shapes.polygons.extend(imported.shapes.polygons);
    if files > 0 {
        log::info!(
            "vector import: {files} file(s), {imported_features} shapes \
             ({} points / {} lines / {} polygons), {} skipped, {} holes dropped, \
             {} capped; demo shapes {}; total plot shapes {}; {:.1} ms",
            imported_counts[0],
            imported_counts[1],
            imported_counts[2],
            imported.skipped,
            imported.holes,
            imported.capped,
            if options.demo { "on" } else { "off" },
            shapes.counts().iter().sum::<usize>(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    shapes
}

/// Replace the plot document with the static shapes merged with the MVT
/// overlay.
///
/// The document owns selection and undo history, so a visible-tile change resets
/// them (the overlay is read-only data, not an editable document). The merged
/// document goes through the same batched plot path as an import: no extra draw
/// call, and `PlotScene::update` rebuilds the three buffers on the next call.
fn sync_mvt_plot(mvt: &MvtLayer, base: &DemoShapes, plot: &mut PlotScene) {
    let selected = plot.document().selected().is_some();
    let (shapes, dropped) =
        mvt_layer::merge_shapes(base, mvt.merged(), mvt_layer::MAX_MERGED_SHAPES);
    let static_count = mvt_layer::shape_count(base);
    let total = mvt_layer::shape_count(&shapes);
    plot.replace_document(PlotDocument::new(shapes));
    if dropped > 0 {
        log::warn!(
            "MVT: {dropped} overlay shapes dropped at the {} shape cap",
            mvt_layer::MAX_MERGED_SHAPES
        );
    }
    if selected {
        log::info!(
            "MVT: visible tile set changed, plot selection/undo reset by the document rebuild"
        );
    }
    log::info!(
        "MVT overlay: {} resident tiles ({} desired), {} features, {} merged shapes; \
         {} static + overlay = {} plot shapes (overlay v{})",
        mvt.stats.resident_tiles,
        mvt.stats.desired_tiles,
        mvt.stats.features,
        mvt.stats.merged_shapes,
        static_count,
        total,
        mvt.version()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headless viewer builds a real GPU scene and returns a frame of the
    /// requested size with pixels in it.
    ///
    /// Gated behind `--ignored` because it needs a working GPU adapter: a
    /// headless CI machine may have none, and in that case `Viewer::new` returns
    /// an error instead of a viewer (which is exactly the error the embedded
    /// window displays rather than crashing). Run it by hand after touching the
    /// viewer or the readback:
    ///
    /// ```text
    /// cargo test -p ew_gis -- --ignored viewer
    /// ```
    #[test]
    #[ignore = "requires a GPU; run with --ignored, see the doc comment"]
    fn headless_viewer_renders_frames() {
        let mut viewer = match Viewer::new(ViewerOptions {
            width: 640,
            height: 360,
            ..ViewerOptions::default()
        }) {
            Ok(viewer) => viewer,
            Err(error) => {
                // No GPU on this machine: the contract is a clean error, not a
                // panic, so there is nothing else to assert.
                eprintln!("[ew_gis] headless viewer unavailable on this machine: {error}");
                return;
            }
        };

        let frame = viewer.render_rgba().expect("first offscreen frame");
        assert_eq!((frame.width, frame.height), (640, 360));
        assert_eq!(frame.bytes.len(), (640 * 360 * 4) as usize);
        assert!(
            frame.bytes.chunks_exact(4).any(|pixel| pixel[0] > 8
                || pixel[1] > 8
                || pixel[2] > 8),
            "the frame is entirely black: the scene did not reach the offscreen target"
        );

        // A resize reallocates the target, and the next frame follows it.
        viewer.resize(320, 200);
        let frame = viewer.render_rgba().expect("frame after resize");
        assert_eq!((frame.width, frame.height), (320, 200));
        assert_eq!(frame.bytes.len(), (320 * 200 * 4) as usize);
    }

    /// A viewer created headless never has a surface: nothing here may try to
    /// acquire a swapchain texture.
    #[test]
    fn headless_context_has_no_surface() {
        // Construction needs a GPU, so only the shape of the API is checked
        // here: the surface field is the `Option` the app matches on.
        fn surface(ctx: &GpuContext) -> Option<&wgpu::Surface<'static>> {
            ctx.surface.as_ref()
        }
        let _ = surface;
    }
}
