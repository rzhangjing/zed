//! winit 0.30 application host, frame loop, input routing, HUD, and
//! screenshot/benchmark exit policy.
//!
//! The scene, the cameras, the renderer and the input state are the library's
//! [`Viewer`]; this module is the window around it. Every scene update and
//! render goes through the viewer, so the embedded Zed GPUI window (which uses
//! the same viewer with no surface) cannot drift from this host.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::render::Renderer;
use crate::scene::SceneMode;
use crate::scene::icons::ICON_COUNT;
use crate::scene::mvt_layer;
use crate::scene::pick::{self, ScreenProjection};
use crate::screenshot;
use crate::tiles::provider::{ImageryOptions, ImageryTag, TileSource};
use crate::viewer::{Viewer, ViewerKey, ViewerOptions};

const TITLE_PREFIX: &str = "EW GIS";

#[derive(Clone, Debug)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub frames: u64,
    pub mode: SceneMode,
    pub screenshot: Option<PathBuf>,
    pub tile_source: Option<TileSource>,
    /// Imagery provider overrides from the command line. Environment values are
    /// applied first by `ProviderConfig::from_env`; these win over them.
    pub imagery: ImageryOptions,
    /// Shapefiles imported at startup (`--shape`, repeatable).
    pub shapefiles: Vec<PathBuf>,
    /// GeoJSON files imported at startup (`--geojson`, repeatable).
    pub geojson: Vec<PathBuf>,
    /// Suppress the built-in demo shapes (`--no-demo`).
    pub no_demo: bool,
    /// Automated Tab-equivalent toggle for headless acceptance runs.
    pub toggle_at: u64,
    pub print_help: bool,
    /// Scripted press position in physical pixels (real input path).
    pub click_at: Option<[f64; 2]>,
    /// Scripted press at the projected centre of this document ordinal.
    pub click_plot: Option<usize>,
    /// Scripted absolute release position (a drag when it differs from the press).
    pub drag_to: Option<[f64; 2]>,
    /// Scripted release offset relative to the press position.
    pub drag_by: Option<[f64; 2]>,
    /// Select a shape by document ordinal, then edit it without the mouse.
    pub select_plot: Option<usize>,
    /// Translate the selection by this many pixels (document-level edit).
    pub move_plot: Option<[f64; 2]>,
    pub delete_plot: bool,
    pub undo_plot: bool,
    /// Log what a pick at this pixel would hit (no state change).
    pub pick_probe: Option<[f64; 2]>,
    /// Frame at which the scripted plot actions run.
    pub plot_at: u64,
    /// Scripted wheel zoom, in wheel lines (positive zooms in).
    pub zoom: i32,
    /// MVT vector tile URL template (`--mvt-url` / `EW_GIS_MVT_URL`).
    pub mvt_url: Option<String>,
    /// MVT local tile root (`--mvt-dir` / `EW_GIS_MVT_DIR`).
    pub mvt_dir: Option<PathBuf>,
    /// Resident/visible MVT tile cap (`--mvt-max-tiles`).
    pub mvt_max_tiles: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
            frames: 0,
            mode: SceneMode::Plane,
            screenshot: None,
            tile_source: None,
            imagery: ImageryOptions::default(),
            shapefiles: Vec::new(),
            geojson: Vec::new(),
            no_demo: false,
            toggle_at: 0,
            print_help: false,
            click_at: None,
            click_plot: None,
            drag_to: None,
            drag_by: None,
            select_plot: None,
            move_plot: None,
            delete_plot: false,
            undo_plot: false,
            pick_probe: None,
            plot_at: 2,
            zoom: 0,
            mvt_url: None,
            mvt_dir: None,
            mvt_max_tiles: mvt_layer::DEFAULT_MAX_TILES,
        }
    }
}

impl Config {
    /// Is a pointer gesture scripted (which needs a few frames to complete)?
    fn scripted_gesture(&self) -> bool {
        self.click_at.is_some() || self.click_plot.is_some()
    }

    /// Is any scripted plot action configured?
    fn scripted_plot(&self) -> bool {
        self.scripted_gesture()
            || self.select_plot.is_some()
            || self.move_plot.is_some()
            || self.delete_plot
            || self.undo_plot
            || self.pick_probe.is_some()
    }

    pub fn from_args_and_env() -> Self {
        let mut config = Self::default();
        let args: Vec<String> = std::env::args().skip(1).collect();
        // CLI value for the MVT cap, so a lower `EW_GIS_MVT_MAX_TILES` cannot
        // override an explicit `--mvt-max-tiles`.
        let mut mvt_max_tiles: Option<usize> = None;
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            let next = || args.get(index + 1).map(String::as_str);
            match arg.as_str() {
                "--help" | "-h" => config.print_help = true,
                "--mode" => {
                    if let Some(value) = next() {
                        config.mode = parse_mode(value).unwrap_or(config.mode);
                        index += 1;
                    }
                }
                value if value.starts_with("--mode=") => {
                    config.mode = parse_mode(&value[7..]).unwrap_or(config.mode);
                }
                "--frames" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.frames = value;
                        index += 1;
                    }
                }
                value if value.starts_with("--frames=") => {
                    if let Ok(frames) = value[9..].parse() {
                        config.frames = frames;
                    }
                }
                "--screenshot" => {
                    if let Some(value) = next() {
                        config.screenshot = Some(PathBuf::from(value));
                        index += 1;
                    }
                }
                value if value.starts_with("--screenshot=") => {
                    config.screenshot = Some(PathBuf::from(&value[13..]));
                }
                "--width" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.width = value;
                        index += 1;
                    }
                }
                "--height" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.height = value;
                        index += 1;
                    }
                }
                "--size" => {
                    if let Some(value) = next() {
                        if let Some((width, height)) = parse_size(value) {
                            config.width = width;
                            config.height = height;
                        }
                        index += 1;
                    }
                }
                "--online" => config.tile_source = Some(TileSource::Online),
                "--offline" => config.tile_source = Some(TileSource::Offline),
                "--procedural" => config.tile_source = Some(TileSource::Procedural),
                "--source" => {
                    if let Some(value) = next().and_then(TileSource::parse) {
                        config.tile_source = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--source=") => {
                    config.tile_source = TileSource::parse(&value[9..]);
                }
                "--toggle-at" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.toggle_at = value;
                        index += 1;
                    }
                }
                value if value.starts_with("--toggle-at=") => {
                    if let Ok(frame) = value[12..].parse() {
                        config.toggle_at = frame;
                    }
                }
                "--click-at" => {
                    if let Some(value) = next().and_then(parse_point) {
                        config.click_at = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--click-at=") => {
                    config.click_at = parse_point(&value[11..]);
                }
                "--drag-to" => {
                    if let Some(value) = next().and_then(parse_point) {
                        config.drag_to = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--drag-to=") => {
                    config.drag_to = parse_point(&value[10..]);
                }
                "--drag-by" => {
                    if let Some(value) = next().and_then(parse_point) {
                        config.drag_by = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--drag-by=") => {
                    config.drag_by = parse_point(&value[10..]);
                }
                "--click-plot" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.click_plot = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--click-plot=") => {
                    config.click_plot = value[13..].parse().ok();
                }
                "--select-plot" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.select_plot = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--select-plot=") => {
                    config.select_plot = value[14..].parse().ok();
                }
                "--move-plot" => {
                    if let Some(value) = next().and_then(parse_point) {
                        config.move_plot = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--move-plot=") => {
                    config.move_plot = parse_point(&value[12..]);
                }
                "--delete-plot" => config.delete_plot = true,
                "--undo-plot" => config.undo_plot = true,
                "--pick-probe" => {
                    if let Some(value) = next().and_then(parse_point) {
                        config.pick_probe = Some(value);
                        index += 1;
                    }
                }
                value if value.starts_with("--pick-probe=") => {
                    config.pick_probe = parse_point(&value[13..]);
                }
                "--plot-at" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.plot_at = value;
                        index += 1;
                    }
                }
                value if value.starts_with("--plot-at=") => {
                    if let Ok(frame) = value[10..].parse() {
                        config.plot_at = frame;
                    }
                }
                "--zoom" => {
                    if let Some(value) = next().and_then(|value| value.parse().ok()) {
                        config.zoom = value;
                        index += 1;
                    }
                }
                value if value.starts_with("--zoom=") => {
                    if let Ok(zoom) = value[7..].parse() {
                        config.zoom = zoom;
                    }
                }
                "--no-demo" => config.no_demo = true,
                "--mvt-max-tiles" => {
                    if let Some(value) = next().and_then(|value| value.parse::<usize>().ok()) {
                        mvt_max_tiles = Some(value.max(1));
                        index += 1;
                    }
                }
                value if value.starts_with("--mvt-max-tiles=") => {
                    if let Ok(parsed) = value[16..].parse::<usize>() {
                        mvt_max_tiles = Some(parsed.max(1));
                    }
                }
                // The provider options are shared by the `--flag value` and
                // `--flag=value` spellings, so they live in one helper instead
                // of two match arms each.
                other => {
                    let handled = parse_imagery_flag(&args, &mut index, &mut config.imagery.kind)
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--tile-url",
                            &mut config.imagery.tile_url,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--tms-url",
                            &mut config.imagery.tms_url,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--arcgis-url",
                            &mut config.imagery.arcgis_url,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--wms-url",
                            &mut config.imagery.wms_url,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--wms-layers",
                            &mut config.imagery.wms_layers,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--wms-styles",
                            &mut config.imagery.wms_styles,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--wms-format",
                            &mut config.imagery.wms_format,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--wms-version",
                            &mut config.imagery.wms_version,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--wms-crs",
                            &mut config.imagery.wms_crs,
                        )
                        || parse_string_flag(
                            &args,
                            &mut index,
                            "--single-image",
                            &mut config.imagery.single_image,
                        )
                        || parse_path_flag(&args, &mut index, "--shape", &mut config.shapefiles)
                        || parse_path_flag(&args, &mut index, "--geojson", &mut config.geojson)
                        || parse_string_flag(&args, &mut index, "--mvt-url", &mut config.mvt_url)
                        || parse_opt_path_flag(&args, &mut index, "--mvt-dir", &mut config.mvt_dir);
                    if !handled {
                        log::warn!("ignoring unknown argument {other:?}");
                    }
                }
            }
            index += 1;
        }

        if let Ok(path) = std::env::var("EW_GIS_SCREENSHOT") {
            if !path.trim().is_empty() {
                config.screenshot = Some(PathBuf::from(path));
            }
        }
        if let Ok(mode) = std::env::var("EW_GIS_MODE") {
            if let Some(mode) = parse_mode(&mode) {
                config.mode = mode;
            }
        }
        if let Ok(frames) = std::env::var("EW_GIS_FRAMES") {
            if let Ok(frames) = frames.parse() {
                config.frames = frames;
            }
        }
        // MVT layer: CLI wins, then the environment, then off.
        if config.mvt_url.is_none() {
            config.mvt_url = std::env::var("EW_GIS_MVT_URL")
                .ok()
                .filter(|value| !value.trim().is_empty());
        }
        if config.mvt_dir.is_none() {
            config.mvt_dir = std::env::var("EW_GIS_MVT_DIR")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from);
        }
        config.mvt_max_tiles = mvt_max_tiles
            .or_else(|| {
                std::env::var("EW_GIS_MVT_MAX_TILES")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(mvt_layer::DEFAULT_MAX_TILES)
            .clamp(1, 256);
        // Only set a source when the flag/env actually asked for one, so the
        // source selected by `EW_GIS_TILE_SOURCE` / `EW_GIS_IMAGERY=local`
        // survives. `auto` is the provider default anyway.
        if config.tile_source.is_none() {
            config.tile_source = match std::env::var("EW_GIS_ONLINE").as_deref() {
                Ok("1") | Ok("true") | Ok("yes") => Some(TileSource::Online),
                Ok("0") | Ok("false") | Ok("no") => Some(TileSource::Offline),
                _ => None,
            };
        }
        config.width = config.width.max(320);
        config.height = config.height.max(240);
        if config.screenshot.is_some() && config.frames == 0 {
            // The acceptance command uses `--frames 3`; without it, still
            // produce a screenshot after a short warmup.
            config.frames = 3;
        }
        if config.scripted_gesture() {
            // The gesture needs the press frame, one intermediate move and the
            // release frame, and the screenshot must not fire mid-drag.
            config.frames = config.frames.max(config.plot_at + 4);
        }
        config
    }
}

fn parse_mode(value: &str) -> Option<SceneMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "2d" | "plane" | "map" => Some(SceneMode::Plane),
        "3d" | "globe" | "earth" => Some(SceneMode::Globe),
        _ => None,
    }
}

fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once(['x', 'X'])?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

/// Parse `X,Y` (or `XxY`) into a pixel/degree pair.
fn parse_point(value: &str) -> Option<[f64; 2]> {
    let (x, y) = value.split_once([',', 'x', 'X'])?;
    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
}

/// Consume one string-valued provider option, in either `--flag value` or
/// `--flag=value` form. Returns true when the argument was consumed.
fn parse_string_flag(
    args: &[String],
    index: &mut usize,
    flag: &str,
    target: &mut Option<String>,
) -> bool {
    let Some(arg) = args.get(*index) else {
        return false;
    };
    if arg == flag {
        let Some(value) = args.get(*index + 1) else {
            return false;
        };
        *target = Some(value.clone());
        *index += 1;
        return true;
    }
    match arg
        .strip_prefix(flag)
        .and_then(|rest| rest.strip_prefix('='))
    {
        Some(value) => {
            *target = Some(value.to_string());
            true
        }
        None => false,
    }
}

/// Consume a repeatable path option (`--shape`, `--geojson`).
fn parse_path_flag(
    args: &[String],
    index: &mut usize,
    flag: &str,
    target: &mut Vec<PathBuf>,
) -> bool {
    let mut value: Option<String> = None;
    let Some(arg) = args.get(*index) else {
        return false;
    };
    if arg == flag {
        let Some(raw) = args.get(*index + 1) else {
            return false;
        };
        value = Some(raw.clone());
        *index += 1;
    } else if let Some(raw) = arg
        .strip_prefix(flag)
        .and_then(|rest| rest.strip_prefix('='))
    {
        value = Some(raw.to_string());
    }
    match value {
        Some(value) if !value.trim().is_empty() => {
            target.push(PathBuf::from(value));
            true
        }
        Some(_) => true,
        None => false,
    }
}

/// Consume a single-valued optional path option (`--mvt-dir`).
fn parse_opt_path_flag(
    args: &[String],
    index: &mut usize,
    flag: &str,
    target: &mut Option<PathBuf>,
) -> bool {
    let mut value: Option<String> = None;
    let Some(arg) = args.get(*index) else {
        return false;
    };
    if arg == flag {
        let Some(raw) = args.get(*index + 1) else {
            return false;
        };
        value = Some(raw.clone());
        *index += 1;
    } else if let Some(raw) = arg
        .strip_prefix(flag)
        .and_then(|rest| rest.strip_prefix('='))
    {
        value = Some(raw.to_string());
    }
    match value {
        Some(value) if !value.trim().is_empty() => {
            *target = Some(PathBuf::from(value));
            true
        }
        Some(_) => true,
        None => false,
    }
}

/// Consume `--imagery KIND` / `--imagery=KIND`.
fn parse_imagery_flag(args: &[String], index: &mut usize, target: &mut Option<ImageryTag>) -> bool {
    let mut value: Option<String> = None;
    let Some(arg) = args.get(*index) else {
        return false;
    };
    if arg == "--imagery" {
        let Some(raw) = args.get(*index + 1) else {
            return false;
        };
        value = Some(raw.clone());
        *index += 1;
    } else if let Some(raw) = arg.strip_prefix("--imagery=") {
        value = Some(raw.to_string());
    }
    let Some(value) = value else {
        return false;
    };
    match ImageryTag::parse(&value) {
        Some(tag) => *target = Some(tag),
        None => log::warn!("unknown --imagery value {value:?}; keeping the current imagery"),
    }
    true
}

pub fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    if config.print_help {
        println!(
            "ew_gis stage 2\n\
             \n\
             Usage: cargo run -p ew_gis -- [options]\n\
             \n\
             Options:\n\
               --mode 2d|3d          initial scene (Tab toggles)\n\
               --frames N            exit after N rendered frames (0 = run until close)\n\
               --size WxH            window size (default 1600x900)\n\
               --width W --height H  window size\n\
               --screenshot PATH     render one offscreen PNG after warmup\n\
               --online              online imagery (fallback: local, procedural)\n\
               --offline             local tiles (fallback: procedural)\n\
               --procedural          force procedural tiles\n\
               --source NAME         auto|online|offline|procedural\n\
               --imagery KIND        xyz|tms|wms|arcgis|single|local (default xyz)\n\
               --tile-url TEMPLATE   XYZ template, {{z}}{{x}}{{y}}{{s}}\n\
               --tms-url TEMPLATE    TMS template, y is flipped automatically\n\
               --arcgis-url BASE     ArcGIS MapServer base; tiles are base/tile/z/y/x\n\
               --wms-url URL         WMS endpoint (GetMap per tile)\n\
               --wms-layers LIST     WMS LAYERS (required when --imagery wms)\n\
               --wms-styles LIST     WMS STYLES (default empty)\n\
               --wms-format FMT      WMS FORMAT (default image/png)\n\
               --wms-version V       WMS VERSION (default 1.3.0; 1.1.1 uses SRS)\n\
               --wms-crs CRS         WMS CRS (default EPSG:3857)\n\
               --single-image PATH   one world image used as the layer-0 base map\n\
               --shape PATH.shp      import a shapefile (repeatable)\n\
               --geojson PATH        import a GeoJSON file (repeatable)\n\
               --no-demo             do not add the built-in demo shapes\n\
               --mvt-url TEMPLATE    MVT vector tiles, {{z}}/{{x}}/{{y}}.mvt URL template\n\
               --mvt-dir PATH        MVT tile root on disk, PATH/{{z}}/{{x}}/{{y}}.mvt\n\
               --mvt-max-tiles N     resident/visible MVT tile cap (default 64)\n\
               --toggle-at N         headless test: perform the Tab toggle at frame N\n\
             \n\
             Vector plot selection/editing (scripted, for screenshots and regression):\n\
               --click-at X,Y        press the left button at this pixel (real pick path)\n\
               --click-plot N        press at the projected centre of document shape N\n\
               --drag-to X,Y         release the press here (an absolute drag)\n\
               --drag-by DX,DY       release at press + (DX,DY) pixels\n\
               --select-plot N       select shape N (flattened order: points, lines,\n\
                                     polygons, sectors) without simulating the mouse\n\
               --move-plot DX,DY     translate the selection by this many pixels\n\
               --delete-plot         delete the selection\n\
               --undo-plot           undo the most recent edit\n\
               --pick-probe X,Y      log which shape a pick at this pixel would hit\n\
               --plot-at N           frame the scripted plot actions run on (default 2)\n\
               --zoom N              scripted wheel zoom, in wheel lines (positive zooms in)\n\
             \n\
             Environment:\n\
               EW_GIS_SCREENSHOT     screenshot path (same as --screenshot)\n\
               EW_GIS_MODE           2d|3d\n\
               EW_GIS_FRAMES         frame limit\n\
               EW_GIS_TILE_SOURCE    auto|online|offline|procedural\n\
               EW_GIS_TILE_DIR       local tile root (default crates/fundamental/ew_gis/assets/tiles)\n\
               EW_GIS_TILE_TEMPLATE  local template (default {{z}}/{{x}}/{{y}}.png)\n\
               EW_GIS_TILE_URL       online XYZ template\n\
               EW_GIS_IMAGERY        xyz|tms|wms|arcgis|single|local\n\
               EW_GIS_TMS_URL        TMS template (y flipped)\n\
               EW_GIS_ARCGIS_URL     ArcGIS MapServer base\n\
               EW_GIS_WMS_URL        WMS endpoint\n\
               EW_GIS_WMS_LAYERS     WMS LAYERS\n\
               EW_GIS_WMS_STYLES     WMS STYLES\n\
               EW_GIS_WMS_FORMAT     WMS FORMAT (default image/png)\n\
               EW_GIS_WMS_VERSION    WMS VERSION (default 1.3.0)\n\
               EW_GIS_WMS_CRS        WMS CRS/SRS (default EPSG:3857)\n\
               EW_GIS_SINGLE_IMAGE   single world image (path or http URL)\n\
               EW_GIS_MVT_URL        MVT vector tile URL template ({{z}}/{{x}}/{{y}}.mvt)\n\
               EW_GIS_MVT_DIR        local MVT tile root ({{z}}/{{x}}/{{y}}.mvt)\n\
               EW_GIS_MVT_MAX_TILES  MVT resident/visible tile cap (default 64)\n\
               EW_GIS_GLOBE_ELLIPSOID     wgs84|sphere (3D reference surface)\n\
               EW_GIS_GLOBE_TARGET_PX     screen-space LOD target, pixels (64..1024)\n\
               EW_GIS_GLOBE_MAX_Z         deepest quadtree level (3..18)\n\
             \n\
             Controls: Tab 2D/3D, left drag pan/orbit, click a plot shape to select,\n\
             drag it to move, Delete/Backspace delete, Ctrl+Z undo, wheel zoom,\n\
             Esc clear selection (again to quit)."
        );
        return Ok(());
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(config);
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct FrameStats {
    samples: [f32; 240],
    cursor: usize,
    count: usize,
    total_frames: u64,
    sum_ms: f64,
    min_ms: f64,
    max_ms: f64,
}

impl FrameStats {
    fn new() -> Self {
        Self {
            samples: [0.0; 240],
            cursor: 0,
            count: 0,
            total_frames: 0,
            sum_ms: 0.0,
            min_ms: f64::MAX,
            max_ms: 0.0,
        }
    }

    fn push(&mut self, ms: f32) {
        self.samples[self.cursor] = ms;
        self.cursor = (self.cursor + 1) % self.samples.len();
        self.count = (self.count + 1).min(self.samples.len());
        self.total_frames += 1;
        self.sum_ms += ms as f64;
        self.min_ms = self.min_ms.min(ms as f64);
        self.max_ms = self.max_ms.max(ms as f64);
    }

    fn average_window(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let sum: f32 = self.samples[..self.count].iter().sum();
        sum / self.count as f32
    }

    fn average(&self) -> f64 {
        if self.total_frames == 0 {
            0.0
        } else {
            self.sum_ms / self.total_frames as f64
        }
    }
}

struct App {
    config: Config,
    window: Option<Arc<Window>>,
    /// The scene, the GPU device, the renderer and the cameras. `None` until
    /// `resumed` has built it.
    viewer: Option<Viewer>,
    /// Last cursor position, in physical pixels. Kept here as well as in the
    /// viewer because a winit `MouseInput` event does not carry a position.
    cursor: Option<PhysicalPosition<f64>>,
    modifiers: ModifiersState,
    /// Press position of the scripted gesture, once it has fired.
    script_press: Option<[f64; 2]>,
    frame_index: u64,
    frame_stats: FrameStats,
    last_frame: Instant,
    started: Instant,
    title_timer: Instant,
    log_timer: Instant,
    last_title: String,
    screenshot_taken: bool,
    exit_logged: bool,
}

impl App {
    fn new(config: Config) -> Self {
        let now = Instant::now();
        Self {
            config,
            window: None,
            viewer: None,
            cursor: None,
            modifiers: ModifiersState::empty(),
            script_press: None,
            frame_index: 0,
            frame_stats: FrameStats::new(),
            last_frame: now,
            started: now,
            title_timer: now,
            log_timer: now,
            last_title: format!("{TITLE_PREFIX} | initializing"),
            screenshot_taken: false,
            exit_logged: false,
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.last_title.clone())
                        .with_inner_size(PhysicalSize::new(self.config.width, self.config.height))
                        .with_min_inner_size(PhysicalSize::new(640, 360)),
                )
                .map_err(|error| format!("create window: {error}"))?,
        );
        let size = window.inner_size();
        let gpu = pollster::block_on(crate::gpu::GpuContext::new(window.clone(), size))
            .map_err(|error| format!("create GPU context: {error}"))?;
        let viewer = Viewer::with_gpu(
            gpu,
            ViewerOptions {
                width: size.width.max(1),
                height: size.height.max(1),
                mode: self.config.mode,
                demo: !self.config.no_demo,
                imagery: self.config.imagery.clone(),
                tile_source: self.config.tile_source,
                shapefiles: self.config.shapefiles.clone(),
                geojson: self.config.geojson.clone(),
                mvt_url: self.config.mvt_url.clone(),
                mvt_dir: self.config.mvt_dir.clone(),
                mvt_max_tiles: self.config.mvt_max_tiles,
                zoom: self.config.zoom,
            },
        )
        .map_err(|error| format!("create viewer: {error}"))?;

        log::info!(
            "EW GIS ready: mode {}, {}x{}, icon instances {}, draw calls {} \
             (base + plot fill/line/point + icons); \
             globe LOD {} ({} vertex / {} index buffer slots); \
             vector plot {} fill triangles, {} line segments, {} point markers; \
             plot document: {} shapes ({} points / {} lines / {} polygons / {} sectors), ids 0..{}; \
             controls: Tab switch, drag pan/orbit, click/drag shape, Delete, Ctrl+Z, wheel zoom, Esc",
            viewer.mode.label(),
            viewer.gpu.width,
            viewer.gpu.height,
            ICON_COUNT,
            Renderer::draw_calls(viewer.mode),
            viewer.globe.config.label(),
            crate::scene::globe::GlobeScene::max_vertices(),
            crate::scene::globe::GlobeScene::max_indices(),
            viewer.plot.stats.fill_triangles,
            viewer.plot.stats.segments,
            viewer.plot.stats.points,
            viewer.plot.document().len(),
            viewer.plot.document().counts()[0],
            viewer.plot.document().counts()[1],
            viewer.plot.document().counts()[2],
            viewer.plot.document().counts()[3],
            viewer.plot.document().len().saturating_sub(1)
        );

        self.window = Some(window);
        self.viewer = Some(viewer);
        self.last_frame = Instant::now();
        self.started = Instant::now();
        // Force the first HUD refresh on the second frame, once a frame-time
        // sample exists, so the title carries a real FPS immediately.
        self.title_timer = Instant::now() - Duration::from_millis(600);
        self.log_timer = Instant::now();
        self.frame_index = 0;
        Ok(())
    }

    fn switch_mode(&mut self) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.toggle_mode();
        }
    }

    /// The world → pixel mapping for the active mode, using exactly the camera
    /// state the next frame will render with.
    fn plot_projection(&self) -> Option<ScreenProjection> {
        self.viewer.as_ref().and_then(Viewer::projection)
    }

    fn delete_selected_plot(&mut self) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.key_down(ViewerKey::Delete);
        }
    }

    fn undo_plot(&mut self) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.key_down(ViewerKey::Undo);
        }
    }

    /// Cursor motion: a grabbed shape follows the pointer, otherwise the camera
    /// pans/orbits exactly as before.
    fn pointer_moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor = Some(position);
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.mouse_move(position.x as f32, position.y as f32);
        }
    }

    /// Left button press/release. The press resolves the cursor position first,
    /// so a click that arrives before any `CursorMoved` still picks correctly.
    fn pointer_button(&mut self, state: ElementState) {
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        match state {
            ElementState::Pressed => {
                if let Some(cursor) = self.cursor {
                    viewer.mouse_down(cursor.x as f32, cursor.y as f32);
                }
            }
            ElementState::Released => viewer.mouse_up(),
        }
    }

    /// Scripted plot actions for headless acceptance runs: the document-level
    /// commands run first, then a pointer gesture that drives the *real*
    /// press/move/release handlers.
    fn run_plot_script(&mut self) {
        if !self.config.scripted_plot() {
            return;
        }
        let base = self.config.plot_at.max(1);
        let frame = self.frame_index;
        if frame == base {
            if let Some(probe) = self.config.pick_probe {
                self.script_pick_probe(probe);
            }
            if let Some(ordinal) = self.config.select_plot {
                self.script_select_ordinal(ordinal);
            }
            if let Some(delta) = self.config.move_plot {
                self.script_move_plot(delta);
            }
            if self.config.delete_plot {
                self.delete_selected_plot();
            }
            if self.config.undo_plot {
                self.undo_plot();
            }
            if let Some(press) = self.script_press_pixel() {
                self.script_press = Some(press);
                let position = PhysicalPosition::new(press[0], press[1]);
                self.cursor = Some(position);
                self.pointer_button(ElementState::Pressed);
            }
        } else if let Some(press) = self.script_press {
            let release = self.script_release_pixel(press);
            if frame == base + 1 {
                if let Some(target) = release {
                    // One intermediate move so the incremental drag path (not
                    // just the final jump) is exercised.
                    let mid = [(press[0] + target[0]) * 0.5, (press[1] + target[1]) * 0.5];
                    self.pointer_moved(PhysicalPosition::new(mid[0], mid[1]));
                }
            } else if frame == base + 2 {
                if let Some(target) = release {
                    self.pointer_moved(PhysicalPosition::new(target[0], target[1]));
                }
                self.pointer_button(ElementState::Released);
                self.script_press = None;
            }
        }
    }

    fn script_press_pixel(&self) -> Option<[f64; 2]> {
        if let Some(pixel) = self.config.click_at {
            return Some(pixel);
        }
        let ordinal = self.config.click_plot?;
        let projection = self.plot_projection()?;
        let document = self.viewer.as_ref()?.plot.document();
        let id = document.id_by_ordinal(ordinal)?;
        let screen = pick::shape_screen_position(document, &projection, id)?;
        Some([screen[0] as f64, screen[1] as f64])
    }

    fn script_release_pixel(&self, press: [f64; 2]) -> Option<[f64; 2]> {
        if let Some(target) = self.config.drag_to {
            return Some(target);
        }
        let offset = self.config.drag_by?;
        Some([press[0] + offset[0], press[1] + offset[1]])
    }

    fn script_select_ordinal(&mut self, ordinal: usize) {
        let Some(projection) = self.plot_projection() else {
            return;
        };
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        let Some(id) = viewer.plot.document().id_by_ordinal(ordinal) else {
            log::warn!(
                "plot: --select-plot {ordinal} is out of range ({} shapes)",
                viewer.plot.document().len()
            );
            return;
        };
        let kind = viewer
            .plot
            .document()
            .locate(id)
            .map(|at| at.kind.label())
            .unwrap_or("shape");
        viewer.plot.select(Some(id));
        let screen = pick::shape_screen_position(viewer.plot.document(), &projection, id);
        log::info!(
            "plot: --select-plot {ordinal} selected {} #{} at {:?} px",
            kind,
            id.raw(),
            screen
        );
    }

    fn script_move_plot(&mut self, offset: [f64; 2]) {
        let Some(projection) = self.plot_projection() else {
            return;
        };
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        let plot = &mut viewer.plot;
        let Some(id) = plot.document().selected() else {
            log::warn!("plot: --move-plot needs a selection (use --select-plot)");
            return;
        };
        let Some(center) = pick::shape_screen_position(plot.document(), &projection, id) else {
            log::warn!("plot: --move-plot could not project shape {}", id.raw());
            return;
        };
        let Some(anchor) = projection.pixel_to_lonlat(center) else {
            log::warn!("plot: --move-plot could not resolve the surface point");
            return;
        };
        let target = [center[0] + offset[0] as f32, center[1] + offset[1] as f32];
        let Some(delta) = projection.drag_delta_from(anchor, target) else {
            log::warn!("plot: --move-plot target is off the surface");
            return;
        };
        if plot.translate_shape(id, delta) {
            log::info!(
                "plot: --move-plot moved shape {} by {:+.4},{:+.4} deg",
                id.raw(),
                delta[0],
                delta[1]
            );
        }
    }

    fn script_pick_probe(&self, probe: [f64; 2]) {
        let Some(projection) = self.plot_projection() else {
            return;
        };
        let Some(viewer) = self.viewer.as_ref() else {
            return;
        };
        let plot = &viewer.plot;
        let cursor = [probe[0] as f32, probe[1] as f32];
        match pick::pick(plot.document(), &projection, cursor) {
            Some(hit) => log::info!(
                "plot probe {:.1},{:.1}: hit {} #{} (ordinal {:?}) at {:.2} px",
                probe[0],
                probe[1],
                hit.kind.label(),
                hit.id.raw(),
                plot.document().ordinal_of(hit.id),
                hit.distance
            ),
            None => log::info!("plot probe {:.1},{:.1}: no shape", probe[0], probe[1]),
        }
    }

    fn update_title(&mut self, title: String) {
        if title == self.last_title {
            return;
        }
        if let Some(window) = &self.window {
            window.set_title(&title);
        }
        self.last_title = title;
    }

    fn current_hud(&self, fps: f32, frame_ms: f32) -> String {
        self.viewer
            .as_ref()
            .map(|viewer| viewer.hud(fps, frame_ms))
            .unwrap_or_else(|| format!("{TITLE_PREFIX} | initializing"))
    }

    fn log_summary(&mut self) {
        if self.exit_logged {
            return;
        }
        self.exit_logged = true;
        let avg = self.frame_stats.average();
        let max = self.frame_stats.max_ms;
        let min = if self.frame_stats.total_frames == 0 {
            0.0
        } else {
            self.frame_stats.min_ms
        };
        let Some(viewer) = self.viewer.as_ref() else {
            return;
        };
        let uploads = viewer.tiles.total_uploaded;
        let evictions = viewer.tiles.array.evictions;
        let plot = viewer.plot_stats();
        let document = {
            let document = viewer.plot.document();
            let selected = match (document.selected(), document.selected_ref()) {
                (Some(id), Some(at)) => {
                    format!("{}#{}", at.kind.label(), id.raw())
                }
                _ => "none".to_string(),
            };
            let counts = document.counts();
            format!(
                "shapes={} (points {} lines {} polygons {} sectors {}), selected={}, edits={}, undo={}",
                document.len(),
                counts[0],
                counts[1],
                counts[2],
                counts[3],
                selected,
                document.edits(),
                document.undo_depth()
            )
        };
        let plot_text = format!(
            "plot: fill_triangles={} fill_vertices={} segments={} points={} rebuilds={} last_rebuild={:.3} ms (tessellation {:.3} ms) draw_calls={}; \
             plot document: {document}",
            plot.fill_triangles,
            plot.fill_vertices,
            plot.segments,
            plot.points,
            plot.rebuilds,
            plot.rebuild_ms,
            plot.tessellate_ms,
            Renderer::draw_calls(viewer.mode)
        );
        let mvt_text = viewer.mvt_summary();
        match viewer.mode {
            SceneMode::Plane => {
                let stats = viewer.plane.stats;
                log::info!(
                    "summary: rendered_frames={} samples={} mode={} avg={:.2} ms ({:.1} FPS) min={:.2} ms max={:.2} ms; \
                     draw_calls={}; visible_tiles={} exact={} fallback={} target_z={}; atlas_uploads={} evictions={}; \
                     {plot_text}; {mvt_text}",
                    self.frame_index,
                    self.frame_stats.total_frames,
                    viewer.mode.label(),
                    avg,
                    if avg > 0.0 { 1000.0 / avg } else { 0.0 },
                    min,
                    max,
                    Renderer::draw_calls(viewer.mode),
                    stats.visible_tiles,
                    stats.exact_tiles,
                    stats.fallback_tiles,
                    stats.target_z,
                    uploads,
                    evictions,
                );
            }
            SceneMode::Globe => {
                let globe = viewer.globe.stats;
                let lod = globe.lod;
                log::info!(
                    "summary: rendered_frames={} samples={} mode={} avg={:.2} ms ({:.1} FPS) min={:.2} ms max={:.2} ms; \
                     draw_calls={}; quadtree: visible_tiles={} exact={} fallback={} max_z={} splits={} \
                     frustum_culled={} horizon_culled={} budget_limited={} caps=({},{}); \
                     vertices={} indices={} rebuilds={} last_rebuild={:.2} ms; atlas_uploads={} evictions={}; \
                     {plot_text}; {mvt_text}",
                    self.frame_index,
                    self.frame_stats.total_frames,
                    viewer.mode.label(),
                    avg,
                    if avg > 0.0 { 1000.0 / avg } else { 0.0 },
                    min,
                    max,
                    Renderer::draw_calls(viewer.mode),
                    globe.visible_tiles,
                    globe.exact_tiles,
                    globe.fallback_tiles,
                    globe.max_depth,
                    lod.splits,
                    lod.frustum_culled,
                    lod.horizon_culled,
                    lod.budget_limited,
                    lod.caps[0],
                    lod.caps[1],
                    globe.vertices,
                    globe.indices,
                    globe.rebuilds,
                    globe.rebuild_ms,
                    uploads,
                    evictions,
                );
            }
        }
    }

    fn draw_frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_logged {
            return;
        }
        let Some(window) = self.window.clone() else {
            return;
        };
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame);
        self.last_frame = now;
        self.frame_index += 1;
        if self.frame_index > 1 {
            self.frame_stats.push((dt.as_secs_f64() * 1000.0) as f32);
        }
        let time = now.duration_since(self.started).as_secs_f32();

        if self.config.toggle_at > 0 && self.frame_index == self.config.toggle_at {
            self.switch_mode();
        }

        // Scripted acceptance input runs before the geometry update so an edit
        // and its rebuild land in the same frame.
        self.run_plot_script();

        {
            let Some(viewer) = self.viewer.as_mut() else {
                return;
            };
            viewer.update(time);

            // A headless viewer has no surface. The standalone host always
            // creates one, so a missing surface here is a programming error.
            let Some(surface) = viewer.gpu.surface.as_ref() else {
                log::error!("the standalone host has no surface to present to");
                return;
            };
            let surface_frame = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => frame,
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    log::warn!("suboptimal surface, reconfiguring");
                    viewer.gpu.reconfigure();
                    frame
                }
                wgpu::CurrentSurfaceTexture::Timeout => {
                    log::warn!("surface timeout");
                    return;
                }
                wgpu::CurrentSurfaceTexture::Occluded => {
                    return;
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    log::warn!("surface outdated, reconfiguring");
                    viewer.gpu.reconfigure();
                    return;
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    log::warn!("surface lost, reconfiguring");
                    viewer.gpu.reconfigure();
                    return;
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    log::error!("surface validation error");
                    return;
                }
            };
            let view = surface_frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            viewer.render_to_view(&view);
            window.pre_present_notify();
            surface_frame.present();
        }

        // Screenshot policy: honor the minimum frame count, then wait (briefly)
        // for the visible exact tiles so the PNG is not only root fallback. Both
        // modes share the same gate: the plane waits for its exact grid, the
        // globe waits for the quadtree leaves it requested (otherwise the shot
        // would be the layer-0 world map). The timeout keeps a broken/slow tile
        // source from hanging the run.
        let min_frames_done = self.config.frames == 0 || self.frame_index >= self.config.frames;
        let screenshot_ready = {
            let Some(viewer) = self.viewer.as_ref() else {
                return;
            };
            // A capture with MVT enabled also waits for the visible vector tiles,
            // so the PNG shows the overlay instead of an empty first frame.
            let mvt_ready = viewer.mvt.as_ref().is_none_or(|mvt| mvt.settled());
            (viewer.tiles.ready_for_screenshot() && mvt_ready)
                || now.duration_since(self.started) >= Duration::from_secs(12)
        };
        if self.config.screenshot.is_some()
            && !self.screenshot_taken
            && min_frames_done
            && screenshot_ready
        {
            if let Some(path) = self.config.screenshot.clone()
                && let Some(viewer) = self.viewer.as_ref()
            {
                match screenshot::capture(
                    &viewer.gpu,
                    &viewer.renderer,
                    &viewer.plot,
                    &viewer.icons,
                    &viewer.globe,
                    viewer.mode,
                    &path,
                ) {
                    Ok(path) => {
                        println!("screenshot: {}", path.display());
                        log::info!("screenshot: {}", path.display());
                    }
                    Err(error) => {
                        log::error!("screenshot failed: {error}");
                        eprintln!("screenshot failed: {error}");
                    }
                }
            }
            self.screenshot_taken = true;
        }

        if self.screenshot_taken
            || (self.config.screenshot.is_none()
                && self.config.frames > 0
                && self.frame_index >= self.config.frames)
        {
            self.log_summary();
            event_loop.exit();
            return;
        }

        // HUD once every 500 ms, plus the first few frames so the title has a
        // real FPS reading even during a short `--frames 3` screenshot run.
        let title_elapsed = now.duration_since(self.title_timer);
        if title_elapsed >= Duration::from_millis(500) || self.frame_index <= 3 {
            let avg_ms = self.frame_stats.average_window();
            if avg_ms > 0.0 {
                let fps = 1000.0 / avg_ms;
                let title = self.current_hud(fps, avg_ms);
                self.update_title(title.clone());
                if self.config.frames <= 3 && self.frame_index <= 3 {
                    log::info!("window title: {title:?}");
                }
                self.title_timer = now;
            }
        }
        if now.duration_since(self.log_timer) >= Duration::from_secs(1) {
            let avg_ms = self.frame_stats.average_window();
            let fps = if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 };
            if let Some(viewer) = self.viewer.as_ref() {
                let (layers, pending) = (viewer.tiles.used_layers(), viewer.tiles.pending());
                let mvt_line = viewer
                    .mvt
                    .as_ref()
                    .map(|mvt| {
                        format!(
                            "{}/{}t {}f {}e",
                            mvt.stats.resident_tiles,
                            mvt.stats.desired_tiles,
                            mvt.stats.features,
                            mvt.stats.errors
                        )
                    })
                    .unwrap_or_else(|| "off".to_string());
                match viewer.mode {
                    SceneMode::Plane => log::info!(
                        "fps={:.1} frame={:.2} ms mode={} tiles={} (exact {}, fallback {}) z={} layers={}/64 pending={} mvt={mvt_line} title={:?}",
                        fps,
                        avg_ms,
                        viewer.mode.label(),
                        viewer.plane.stats.visible_tiles,
                        viewer.plane.stats.exact_tiles,
                        viewer.plane.stats.fallback_tiles,
                        viewer.plane.stats.target_z,
                        layers,
                        pending,
                        self.last_title
                    ),
                    SceneMode::Globe => {
                        let globe = viewer.globe.stats;
                        log::info!(
                            "fps={:.1} frame={:.2} ms mode={} tiles={} (exact {}, fallback {}) z={} layers={}/64 pending={} verts={} indices={} rebuild={:.2} ms changed={} draws={} mvt={mvt_line} title={:?}",
                            fps,
                            avg_ms,
                            viewer.mode.label(),
                            globe.visible_tiles,
                            globe.exact_tiles,
                            globe.fallback_tiles,
                            globe.max_depth,
                            layers,
                            pending,
                            globe.vertices,
                            globe.indices,
                            globe.rebuild_ms,
                            globe.geometry_changed,
                            Renderer::draw_calls(viewer.mode),
                            self.last_title
                        );
                    }
                }
            }
            self.log_timer = now;
        }

        // Continuous animation: request the next redraw (VSync paces present).
        window.request_redraw();
    }

    fn handle_key(&mut self, event_loop: &ActiveEventLoop, event: winit::event::KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        let key = match event.physical_key {
            PhysicalKey::Code(KeyCode::Tab) => Some(ViewerKey::Tab),
            PhysicalKey::Code(KeyCode::Digit1) => Some(ViewerKey::Plane),
            PhysicalKey::Code(KeyCode::Digit2) => Some(ViewerKey::Globe),
            PhysicalKey::Code(KeyCode::Delete) | PhysicalKey::Code(KeyCode::Backspace) => {
                Some(ViewerKey::Delete)
            }
            PhysicalKey::Code(KeyCode::KeyZ) if self.modifiers.control_key() => {
                Some(ViewerKey::Undo)
            }
            PhysicalKey::Code(KeyCode::Escape) => Some(ViewerKey::Escape),
            _ => None,
        };
        let Some(key) = key else {
            return;
        };
        let handled = self
            .viewer
            .as_mut()
            .map(|viewer| viewer.key_down(key))
            .unwrap_or(false);
        match key {
            // Esc clears a plot selection first; with nothing selected it quits
            // as before.
            ViewerKey::Escape if !handled => {
                self.log_summary();
                event_loop.exit();
            }
            ViewerKey::Escape => {
                log::info!("plot: selection cleared (Esc)");
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.initialize(event_loop) {
            log::error!("initialization failed: {error}");
            eprintln!("ew_gis initialization failed: {error}");
            event_loop.exit();
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                self.log_summary();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }
                if let Some(viewer) = self.viewer.as_mut() {
                    viewer.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = window.inner_size();
                if let Some(viewer) = self.viewer.as_mut() {
                    viewer.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_key(event_loop, event);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_moved(position);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    self.pointer_button(state);
                    window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => (position.y / 60.0) as f32,
                };
                if let Some(viewer) = self.viewer.as_mut() {
                    viewer.wheel(amount);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.draw_frame(event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.window.is_some() && !self.exit_logged {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}
