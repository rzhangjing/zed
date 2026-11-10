//! Zed's adapter onto the RustGIS library layers (L0-L5).
//!
//! RustGIS is copied into `crates/rustgis` verbatim (0.6.0, Apache-2.0; see
//! `upstream.sha256`), keeps its own workspace root, and is never edited here. This crate is the
//! single bridge between that tree and Zed: everything Zed needs from RustGIS goes through the
//! types below, so Zed itself never names a `rustgis-*` layer and the layer rules of the copied
//! tree (`cargo xtask layers`, L6 = egui/wgpu/winit) stay intact. `rustgis-ui-egui` is
//! deliberately not a dependency.
//!
//! # What this wraps
//!
//! | Layer | Crate | Reached through |
//! |---|---|---|
//! | L0 | `rustgis-geom` | [`MapEngine::center_on`], [`CameraStatus`] |
//! | L1 | `rustgis-data` | [`MapEngine::load_geojson`], [`LayerInfo`] |
//! | L2 | `rustgis-map` | [`MapEngine`]'s camera, layers, tiles |
//! | L2 | `rustgis-analysis` | [`MapEngine::command`] (`gp.*`), [`capabilities`] |
//! | L2 | `rustgis-tiles` | [`MapEngine::tile_layers`] |
//! | L2 | `rustgis-keychain` | [`capabilities`] |
//! | L3 | `rustgis-render` | [`MapEngine::render_rgba`], [`MapEngine::render_png`] |
//! | L3 | `rustgis-plugins` | [`capabilities`] |
//! | L3 | `rustgis-geolibre` | [`capabilities`] |
//! | L4 | `rustgis-engine` | [`MapEngine::command`] (`Session::run`) |
//! | L5 | `rustgis-assistant` | [`capabilities`] |
//!
//! # Threading
//!
//! [`MapEngine`] owns a `rustgis_engine::Session`, which is not `Send`; keep it on one thread.
//! [`spawn_engine`] does exactly that: a thread owns the engine and takes [`EngineCommand`]s, so a
//! host can draw a map without drawing on its UI thread.
//!
//! # Interaction and the two qualities
//!
//! A full 1200×800 frame of the sample project is expensive (hundreds of milliseconds in a
//! `cargo build` debug profile): the basemap outline, four layers and label placement all run
//! per frame. While a pointer gesture is in progress a host should draw [`Quality::Preview`]
//! frames instead — a narrower canvas, no labels, no antialiasing ([`MapEngine`] turns
//! `Map::view::fast` on for the duration of that frame only, and restores it) — and settle to a
//! [`Quality::Full`] frame once the gesture stops. Interaction never goes through
//! [`MapEngine::command`]: panning and zooming are [`MapEngine::pan`], [`MapEngine::zoom_at`],
//! [`MapEngine::zoom_center`] and [`MapEngine::zoom_world`], which only touch the camera.
//!
//! `command` is for data and edit commands, and logs its `Session::run` time (and the undo depth
//! before/after) under `[rustgis]`; the navigation commands in RustGIS's registry (`map.zoomIn`,
//! …) are `view()` commands, so they take no undo snapshot, but they still run the session's
//! bookkeeping (log entry, registry lookup) and are not an interaction path.
//!
//! # Panics
//!
//! RustGIS catches escaped panics inside `Session::run`, and the renderer validates sizes. This
//! adapter adds no `unwrap`/`expect`/`panic!` of its own: every fallible path returns
//! `Result<_, String>` so a bad frame degrades into an error a window can show.

use std::sync::Arc;
use std::time::Instant;

use rustgis_engine::{Session, project::Project};
use rustgis_map::Camera;
use rustgis_render::View;

/// The window's default frame size, in logical pixels (`Help > RustGIS`).
pub const DEFAULT_WIDTH: u32 = 1200;
pub const DEFAULT_HEIGHT: u32 = 800;

/// The largest side the renderer accepts (`View::pixel_size` clamps to the same).
const MAX_SIDE: u32 = 16_384;

/// How a frame is drawn.
///
/// [`Quality::Full`] is the settled view: the whole canvas, labels when [`MapEngine::set_labels`]
/// has them on, antialiasing as the map asks. [`Quality::Preview`] is the frame an interaction
/// shows: the canvas is divided by `divisor` (2 = half), labels are skipped and antialiasing is
/// off for that frame only. Since the camera's extent is kept, the smaller frame stretches back
/// over the whole view without moving the map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// The settled view, at the requested size.
    #[default]
    Full,
    /// A pointer gesture's frame, at `width / divisor` × `height / divisor`.
    Preview {
        /// 2 halves each side (a quarter of the pixels); a host may raise it when even that frame
        /// misses its frame budget. Clamped to `2..=8`.
        divisor: u8,
    },
}

impl Quality {
    /// Half resolution, no labels, no antialiasing.
    pub const PREVIEW: Quality = Quality::Preview { divisor: 2 };

    /// The canvas divisor: 1 for [`Quality::Full`], `2..=8` for a preview.
    pub fn divisor(self) -> u8 {
        match self {
            Quality::Full => 1,
            Quality::Preview { divisor } => divisor.clamp(2, 8),
        }
    }

    /// Whether this is an interaction preview.
    pub fn is_preview(self) -> bool {
        self.divisor() > 1
    }
}


/// A camera position in the words a status line shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraStatus {
    /// Longitude and latitude of the view centre, in degrees.
    pub center_lon: f64,
    pub center_lat: f64,
    /// Map units (Web Mercator metres) per logical pixel.
    pub res: f64,
    /// The scale denominator at that resolution, for "1:N".
    pub scale: f64,
    pub bearing: f64,
    pub pitch: f64,
}

impl CameraStatus {
    fn of(camera: &Camera) -> Self {
        let center = camera.center_lonlat();
        CameraStatus {
            center_lon: center.x,
            center_lat: center.y,
            res: camera.res,
            scale: camera.scale(),
            bearing: camera.bearing,
            pitch: camera.pitch,
        }
    }

    /// "lat, lon" in degrees. Five decimals is about a metre at the equator, which is finer than
    /// a pan can be judged by.
    pub fn center_text(&self) -> String {
        format!("{:.5}, {:.5}", self.center_lat, self.center_lon)
    }

    /// The scale as "1:N", rounded to something readable.
    pub fn scale_text(&self) -> String {
        let n = self.scale;
        if !n.is_finite() || n <= 0.0 {
            return "1:?".to_string();
        }
        if n >= 1_000_000.0 {
            format!("1:{:.1}M", n / 1_000_000.0)
        } else if n >= 1_000.0 {
            format!("1:{:.0}k", n / 1_000.0)
        } else {
            format!("1:{n:.0}")
        }
    }
}

/// One layer of the active map, for a Layers list or a status line.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerInfo {
    pub id: u32,
    pub name: String,
    pub visible: bool,
    /// L1's geometry type name (`Point`, `Line`, `Polygon`, ...).
    pub geometry_type: String,
    pub features: usize,
    /// Whether the layer draws labels.
    pub labels: bool,
}

/// One command the session can run, for a command list or a picker.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandInfo {
    pub id: String,
    pub label: String,
    /// Where the RustGIS ribbon puts it (`Home › Navigate`, ...).
    pub location: String,
    /// Whether running it changes the project (and takes an undo step).
    pub mutates: bool,
}

/// A notice RustGIS raised: a read report's loss, a command's warning, an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// The pixels of one frame, ready to hand to a UI's image surface.
///
/// The bytes are **straight (non-premultiplied) RGBA**, row-major, top-left first: `tiny-skia`
/// keeps its pixmaps premultiplied and [`MapEngine::render_rgba`] undoes that, because most UI
/// texturing paths treat alpha as ordinary coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// Whether every pixel is fully transparent, i.e. nothing was drawn.
    pub fn is_blank(&self) -> bool {
        self.rgba.chunks_exact(4).all(|px| px[3] == 0)
    }

    /// How many pixels differ from `other` (`None` when the sizes differ). A host uses it to skip
    /// a texture upload when a pan changed nothing.
    pub fn different_pixels(&self, other: &Frame) -> Option<usize> {
        if (self.width, self.height) != (other.width, other.height) {
            return None;
        }
        Some(
            self.rgba
                .chunks_exact(4)
                .zip(other.rgba.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count(),
        )
    }
}

/// A frame already in the byte order GPUI's atlas wants (BGRA), so the UI thread does no
/// per-pixel work to hand a frame to the renderer.
///
/// GPUI's image atlas is BGRA (`gpui_wgpu` swizzles RGBA uploads); converting on the engine
/// thread, in the same pass that undoes `tiny-skia`'s premultiplication, keeps the 3.75 MiB
/// channel swap off the UI thread entirely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BgraFrame {
    pub width: u32,
    pub height: u32,
    /// Straight (non-premultiplied) BGRA, row-major, top-left first.
    pub bgra: Vec<u8>,
}

impl BgraFrame {
    /// Whether every pixel is fully transparent, i.e. nothing was drawn.
    pub fn is_blank(&self) -> bool {
        self.bgra.chunks_exact(4).all(|px| px[3] == 0)
    }

    /// How many pixels differ from `other` (`None` when the sizes differ).
    pub fn different_pixels(&self, other: &BgraFrame) -> Option<usize> {
        if (self.width, self.height) != (other.width, other.height) {
            return None;
        }
        Some(
            self.bgra
                .chunks_exact(4)
                .zip(other.bgra.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count(),
        )
    }
}

/// A GPU scene the renderer built for one view: what [`gpu::GpuMap`] uploads and draws.
pub struct GpuFrame {
    pub view: View,
    pub scene: rustgis_render::gpu_scene::GpuScene,
}

/// The inputs a frame is a function of: a cache hit can hand back the same pixels.
#[derive(Clone, PartialEq)]
struct RenderKey {
    width: u32,
    height: u32,
    quality: Quality,
    labels: bool,
    /// Bumped whenever the map's content changes (a layer added, a command run).
    revision: u64,
    camera: Camera,
    /// Whether the cached bytes are BGRA ([`BgraFrame`]) or RGBA ([`Frame`]).
    bgra: bool,
}

/// The last frame drawn, so a duplicate request (a resize that lands on the same size, a settle
/// after the camera has not moved) costs a copy instead of a render.
struct CachedFrame {
    key: RenderKey,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}


/// A headless RustGIS session plus the view size its frames are drawn for.
///
/// Construction costs what parsing the sample project costs (~4 MB of embedded GeoJSON), so build
/// one and reuse it rather than one per frame.
pub struct MapEngine {
    session: Session,
    width: f64,
    height: f64,
    /// Draw the map's labels into a [`Quality::Full`] frame.
    labels: bool,
    /// How far the caller has read `Session`'s notices.
    notice_mark: u64,
    /// Bumped when the map's content changes, so a cached frame is never handed out for it.
    revision: u64,
    /// The last frame drawn, for duplicate requests.
    cached: Option<CachedFrame>,
}

impl Default for MapEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MapEngine {
    /// The "World Atlas" sample project: Natural Earth countries by population, lakes, rivers and
    /// cities, with the basemap, labels and camera the RustGIS app opens with.
    pub fn new() -> Self {
        Self::from_project(rustgis_engine::sample::world_project())
    }

    /// A session over an arbitrary project.
    pub fn from_project(project: Project) -> Self {
        MapEngine {
            session: Session::new(project),
            width: DEFAULT_WIDTH as f64,
            height: DEFAULT_HEIGHT as f64,
            labels: true,
            notice_mark: 0,
            revision: 0,
            cached: None,
        }
    }

    /// Whether labels are drawn into a [`Quality::Full`] frame ([`MapEngine::render_rgba`]).
    /// Turning them off (or on) invalidates the frame cache; previews never draw labels.
    pub fn set_labels(&mut self, on: bool) {
        if self.labels != on {
            self.labels = on;
            self.invalidate();
        }
    }

    pub fn labels(&self) -> bool {
        self.labels
    }

    /// The size the next frame is drawn for, in logical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width.max(1.0) as u32, self.height.max(1.0) as u32)
    }

    /// Set the frame size. The camera keeps its resolution, so a resize shows more or less ground
    /// rather than zooming.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.clamp(1, MAX_SIDE) as f64;
        self.height = height.clamp(1, MAX_SIDE) as f64;
    }

    /// Read GeoJSON from memory into the active map as a new layer named `name`.
    ///
    /// Returns the new layer's id; the layer is not added if the bytes don't parse. Whatever the
    /// reader noticed but didn't drop is in [`MapEngine::take_notices`].
    pub fn load_geojson_named(&mut self, bytes: &[u8], name: &str) -> Result<u32, String> {
        let text =
            std::str::from_utf8(bytes).map_err(|e| format!("the file is not UTF-8 text: {e}"))?;
        let (dataset, _report) = rustgis_data::io::read_geojson(text, name)
            .map_err(|e| format!("could not read GeoJSON: {e}"))?;
        if dataset.features.is_empty() {
            return Err(format!("{name}: the GeoJSON holds no features"));
        }
        let id = self.session.map_mut().add_dataset(dataset);
        self.invalidate();
        Ok(id)
    }

    /// [`MapEngine::load_geojson_named`] under the name `"Layer"`, dropping the new layer's id:
    /// the shape the window uses.
    pub fn load_geojson(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.load_geojson_named(bytes, "Layer").map(|_| ())
    }

    /// Draw the map at the current size, as straight RGBA (see [`Frame`]): a settled
    /// [`Quality::Full`] frame.
    ///
    /// With [`MapEngine::set_labels`] on (the default) this draws through
    /// `rustgis_render::render_placed`, which is `render` plus the label layout and rasterised
    /// text, minus the scale bar and credits `render_png` adds. With it off it is
    /// `rustgis_render::render` alone: faster, and the frame then holds no text.
    pub fn render_rgba(&mut self, width: u32, height: u32) -> Result<Frame, String> {
        self.render_rgba_quality(width, height, Quality::Full)
    }

    /// [`MapEngine::render_rgba`] at an explicit [`Quality`]. A preview frame is smaller than
    /// `width` × `height`: `width / divisor` × `height / divisor`.
    pub fn render_rgba_quality(
        &mut self,
        width: u32,
        height: u32,
        quality: Quality,
    ) -> Result<Frame, String> {
        let (width, height, bytes) = self.draw(width, height, quality, false)?;
        Ok(Frame {
            width,
            height,
            rgba: bytes,
        })
    }

    /// [`MapEngine::render_rgba`] in GPUI's byte order: the UI thread can hand these bytes to
    /// `RenderImage` without touching a pixel.
    pub fn render_bgra(&mut self, width: u32, height: u32) -> Result<BgraFrame, String> {
        self.render_bgra_quality(width, height, Quality::Full)
    }

    /// [`MapEngine::render_bgra`] at an explicit [`Quality`].
    pub fn render_bgra_quality(
        &mut self,
        width: u32,
        height: u32,
        quality: Quality,
    ) -> Result<BgraFrame, String> {
        let (width, height, bytes) = self.draw(width, height, quality, true)?;
        Ok(BgraFrame {
            width,
            height,
            bgra: bytes,
        })
    }

    /// Drop the cached frame: the map or the way it is drawn changed.
    fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.cached = None;
    }

    /// Draw, or return the last frame when every input is unchanged.
    fn draw(
        &mut self,
        width: u32,
        height: u32,
        quality: Quality,
        bgra: bool,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        self.resize(width, height);
        let key = RenderKey {
            width: self.width as u32,
            height: self.height as u32,
            quality,
            labels: self.labels,
            revision: self.revision,
            camera: self.session.map().camera,
            bgra,
        };
        if let Some(cached) = &self.cached
            && cached.key == key
        {
            log::debug!(
                "[rustgis] frame cache hit: {}x{} {:?}",
                cached.width,
                cached.height,
                quality
            );
            return Ok((cached.width, cached.height, cached.bytes.clone()));
        }

        let divisor = f64::from(quality.divisor());
        let canvas_width = (self.width / divisor).round().max(1.0) as u32;
        let canvas_height = (self.height / divisor).round().max(1.0) as u32;
        // The same ground in fewer pixels: the camera's resolution grows with the divisor, so the
        // cursor-anchored zoom and a later full frame agree on where the map is.
        let mut camera = self.session.map().camera;
        if divisor != 1.0 {
            camera.res *= divisor;
            camera.clamp();
        }
        let view = View {
            width: canvas_width as f64,
            height: canvas_height as f64,
            pixels_per_point: 1.0,
            camera,
            sky: false,
        };
        let labels = self.labels && !quality.is_preview();
        let preview = quality.is_preview();
        let started = Instant::now();
        let pixmap = {
            let map = self.session.map_mut();
            // `fast` is session-only (serde-skipped) and restored below: antialiasing is most of
            // a preview frame's cost in an unoptimised build, and a moving map hides the jaggies.
            let fast = map.view.fast;
            if preview {
                map.view.fast = true;
            }
            let rendered = if labels {
                rustgis_render::render_placed(map, &view)
                    .map(|placed| placed.pixmap)
                    .map_err(|e| format!("the map could not be drawn: {e}"))
            } else {
                rustgis_render::render(map, &view)
                    .map(|rendered| rendered.pixmap)
                    .ok_or_else(|| "the renderer rejected this frame".to_string())
            };
            map.view.fast = fast;
            rendered
        }?;
        let bytes = frame_bytes(&pixmap, bgra)?;
        let elapsed = started.elapsed();
        log::info!(
            "[rustgis] frame {canvas_width}x{canvas_height} {quality:?} labels={labels} in {:.1} ms",
            elapsed.as_secs_f64() * 1000.0
        );
        self.cached = Some(CachedFrame {
            key,
            width: canvas_width,
            height: canvas_height,
            bytes: bytes.clone(),
        });
        Ok((canvas_width, canvas_height, bytes))
    }

    /// Draw the map with labels, a scale bar and the tile credits, as PNG bytes: what RustGIS's
    /// own `share.exportMap` writes.
    pub fn render_png(&mut self, width: u32, height: u32) -> Result<Vec<u8>, String> {
        self.resize(width, height);
        let view = View {
            width: self.width,
            height: self.height,
            pixels_per_point: 1.0,
            camera: self.session.map().camera,
            sky: false,
        };
        rustgis_render::render_png(self.session.map_mut(), &view)
    }

    /// The GPU scene for `width`×`height` logical points at `dpi` device pixels per point: what
    /// [`gpu::GpuMap`] draws into the map window's surface.
    ///
    /// `None` when this view must be drawn by the CPU renderer (the globe, tilted pictures, the
    /// effect-heavy frames `Backend::Gpu` declines): the caller falls back for that frame.
    pub fn gpu_scene(&mut self, width: u32, height: u32, dpi: f32) -> Option<GpuFrame> {
        self.resize(width, height);
        let view = self.view(width, height, dpi);
        // `set_labels(false)` silences labels for the frame only: `label_display` is saved state.
        let labels = self.labels;
        let map = self.session.map_mut();
        let paused = map.label_display.paused;
        if !labels {
            map.label_display.paused = true;
        }
        let output = rustgis_render::render_backend(map, &view, rustgis_render::Backend::Gpu);
        map.label_display.paused = paused;
        match output {
            Some(rustgis_render::RenderOutput::Gpu(scene)) => Some(GpuFrame { view, scene }),
            _ => None,
        }
    }

    /// The view a frame at `width`×`height` logical points, `dpi` device pixels per point, shows.
    pub(crate) fn view(&self, width: u32, height: u32, dpi: f32) -> View {
        let dpi = if dpi.is_finite() && dpi > 0.0 { dpi.clamp(0.1, 8.0) } else { 1.0 };
        View {
            width: f64::from(width.clamp(1, MAX_SIDE)),
            height: f64::from(height.clamp(1, MAX_SIDE)),
            pixels_per_point: f64::from(dpi),
            camera: self.session.map().camera,
            sky: false,
        }
    }

    /// The active map, for the GPU presenter (`GpuMap::prepare` keeps it for tile placement).
    pub(crate) fn map_mut(&mut self) -> &mut rustgis_map::Map {
        self.session.map_mut()
    }

    /// Move the map by a screen offset, as dragging it does (`Camera::pan_px`). Pure camera: no
    /// `Session::run`, no undo snapshot.
    pub fn pan(&mut self, dx_px: f64, dy_px: f64) {
        if !dx_px.is_finite() || !dy_px.is_finite() {
            return;
        }
        self.camera_mut().pan_px(dx_px, dy_px);
    }

    /// Zoom by `factor` (> 1 zooms in) about the pixel (`x_px`, `y_px`) of the current frame.
    /// Pure camera: no `Session::run`, no undo snapshot.
    pub fn zoom_at(&mut self, x_px: f64, y_px: f64, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let (w, h) = (self.width, self.height);
        self.camera_mut().zoom_at(factor, x_px, y_px, w, h);
    }

    /// Zoom by `factor` about the view centre: what the `Zoom In`/`Zoom Out` buttons and the
    /// `+`/`-` keys do. Pure camera, so it is not a command and takes no undo step.
    pub fn zoom_center(&mut self, factor: f64) {
        let (w, h) = (self.width, self.height);
        self.zoom_at(w / 2.0, h / 2.0, factor);
    }

    /// Fit the World Atlas view (the extent RustGIS's `map.zoomWorld` uses) into the current
    /// frame size. Pure camera, so it is not a command and takes no undo step.
    pub fn zoom_world(&mut self) {
        let extent = rustgis_geom::Extent::new(-170.0, -58.0, 180.0, 78.0);
        let (w, h) = (self.width, self.height);
        let map = self.session.map_mut();
        if map.projection.is_globe() {
            rustgis_map::Globe::fit(&mut map.camera, &extent, w, h);
        } else {
            map.fit_lonlat(&extent, w, h);
        }
    }


    /// Point the camera at a lon/lat centre, keeping the current zoom.
    pub fn center_on(&mut self, lon: f64, lat: f64) -> Result<(), String> {
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Err(format!("{lat}, {lon} is not a longitude/latitude pair"));
        }
        let coord = rustgis_geom::lonlat_to_merc(rustgis_geom::Coord::new(lon, lat));
        let camera = self.camera_mut();
        camera.center = coord;
        camera.clamp();
        Ok(())
    }

    /// The camera, as a status line shows it.
    pub fn camera_status(&self) -> CameraStatus {
        CameraStatus::of(&self.session.map().camera)
    }

    /// How many layers the active map has, without building [`LayerInfo`]s for them.
    pub fn layer_count(&self) -> usize {
        self.session.map().layers.len()
    }

    /// The active map's layers, **top of the drawing order first**: the renderer draws the last
    /// layer first, so the list is what a Layers pane shows top-down.
    pub fn layers(&self) -> Vec<LayerInfo> {        self.session
            .map()
            .layers
            .iter()
            .map(|layer| LayerInfo {
                id: layer.id,
                name: layer.name.clone(),
                visible: layer.visible,
                geometry_type: format!("{:?}", layer.dataset.geometry_type),
                features: layer.dataset.features.len(),
                labels: layer.labels.enabled,
            })
            .collect()
    }

    /// The active map's web tile layers (basemaps and overlays), by name.
    pub fn tile_layers(&self) -> Vec<String> {
        self.session.map().tiles.iter().map(|tile| tile.name.clone()).collect()
    }

    /// Every command the session's registry holds, in registry order.
    pub fn commands(&self) -> Vec<CommandInfo> {
        self.session
            .registry
            .commands
            .iter()
            .map(|spec| CommandInfo {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                location: spec.location.to_string(),
                mutates: spec.mutates,
            })
            .collect()
    }

    /// The commands whose id starts with `prefix` (`gp.`, `plugin.`, ...).
    pub fn commands_with_prefix(&self, prefix: &str) -> Vec<CommandInfo> {
        self.commands()
            .into_iter()
            .filter(|command| command.id.starts_with(prefix))
            .collect()
    }

    /// Run a RustGIS command by id (`Session::run`), returning its JSON result.
    ///
    /// This is the **data/edit** path, not an interaction path: it logs how long `Session::run`
    /// took and the undo depth before and after (`[rustgis]`), so a mutating command's project
    /// snapshot is visible in the log. Navigation commands (`map.zoomIn`, `map.zoomOut`,
    /// `map.zoomWorld`, …) are `view()` commands — `mutates: false`, no undo snapshot — but they
    /// still do session bookkeeping, so the window uses the camera methods instead.
    pub fn command(
        &mut self,
        id: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let undo_before = self.session.undo_depth();
        let started = Instant::now();
        let result = self.session.run(id, &params);
        let elapsed = started.elapsed();
        let undo_after = self.session.undo_depth();
        // A command can change layers, styles, the projection or the camera; a frame cached from
        // before it is not this map any more.
        self.invalidate();
        let millis = elapsed.as_secs_f64() * 1000.0;
        match &result {
            Ok(_) => log::info!(
                "[rustgis] Session::run({id}) {millis:.3} ms, undo {undo_before} -> {undo_after} \
                 (snapshot: {})",
                undo_after > undo_before
            ),
            Err(error) => log::warn!(
                "[rustgis] Session::run({id}) failed after {millis:.3} ms, undo {undo_before} -> \
                 {undo_after}: {error}"
            ),
        }
        result
    }

    /// The notices RustGIS raised since the last call: data-loss warnings, errors, adjustments.
    /// A host that shows them satisfies RustGIS's "never silent" rule.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        let mark = self.session.notice_mark();
        let from = std::mem::replace(&mut self.notice_mark, mark);
        self.session
            .notices_since(from)
            .iter()
            .map(|notice| Notice {
                text: notice.text.clone(),
                error: notice.error,
            })
            .collect()
    }

    /// What this build can call, per layer (see [`capabilities`]).
    pub fn capabilities(&self) -> Vec<Capability> {
        capabilities::of_engine(self)
    }

    fn camera_mut(&mut self) -> &mut Camera {
        &mut self.session.map_mut().camera
    }
}

/// Turn a `tiny-skia` pixmap into straight RGBA, or BGRA when `bgra` is set.
fn frame_bytes(pixmap: &rustgis_render::Pixmap, bgra: bool) -> Result<Vec<u8>, String> {
    let (width, height) = (pixmap.width(), pixmap.height());
    let mut bytes = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        if bgra {
            bytes.extend_from_slice(&[color.blue(), color.green(), color.red(), color.alpha()]);
        } else {
            bytes.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
    }
    let expected = (width as usize) * (height as usize) * 4;
    if bytes.len() != expected {
        return Err(format!(
            "the renderer returned {} bytes for a {width}x{height} frame",
            bytes.len()
        ));
    }
    Ok(bytes)
}


/// L0-L5 availability, for a window that wants to say what it can do.
pub mod capabilities;

/// Running the engine on its own thread, for hosts whose UI thread must never block.
pub mod worker;

/// The map's label text layer (rasterised placed labels over the GPU frame).
pub mod labels;

/// GPU rendering of the map (the map's own wgpu surface; see `gpu::GpuMap`).
pub mod gpu;

/// The map's native child window, for hosts where a wgpu surface is a window surface.
#[cfg(target_os = "windows")]
pub mod win;

/// The map's wgpu surface on that child window (no read-back: the GPU presents the frame).
#[cfg(target_os = "windows")]
pub mod present;

pub use capabilities::Capability;
pub use worker::{EngineCommand, EngineEvent, EngineFrame, EngineHandle, spawn_engine};

/// A `Frame`'s bytes as an `image::RgbaImage`, for hosts that built their texture through `image`.
pub fn frame_to_rgba_image(frame: &Frame) -> Option<image::RgbaImage> {
    image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
}

/// Encode a frame as PNG, for headless proofs and for "save this view".
pub fn frame_to_png(frame: &Frame) -> Result<Vec<u8>, String> {
    use image::ImageEncoder as _;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &frame.rgba,
            frame.width,
            frame.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("could not encode the frame as PNG: {e}"))?;
    Ok(png)
}

/// Decode an image file (PNG/JPEG/WebP) into straight RGBA, for "open this picture".
pub fn decode_image(bytes: &[u8]) -> Result<Frame, String> {
    let decoded = image::load_from_memory(bytes).map_err(|e| format!("not an image: {e}"))?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(Frame {
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

/// A frame a UI shares with whoever else is drawing it, so a redraw does not copy it.
pub type SharedFrame = Arc<Frame>;
