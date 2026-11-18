# ew_gis — native wgpu 3D globe + 3D-plane 2D map

A from-scratch GIS renderer built directly on **wgpu 29** and **winit 0.30** — no Bevy,
no egui, no ECS. It is the first milestone of the new EW GIS: 2D and 3D map display
with an architecture designed for very large plotting workloads and three-axis
rotated icons.

## What is implemented

| Area | Details |
| --- | --- |
| 3D globe | WGS84 ellipsoid, WebMercator imagery tiles, quadtree LOD (screen-space error), frustum + horizon culling, skirts, polar cap; one merged indexed draw |
| 2D map | Plate Carrée plane rendered as a **3D scene** (perspective camera, z = 0 plane), instanced tile quads, one draw; `Tab` switches 2D↔3D preserving center/zoom |
| Tiles | Imagery providers `xyz` / `tms` / `wms` / `arcgis` / `single`, then local directory, then procedural fallback; 512² RGBA8 `texture_2d_array` with 64 layers (1 world base + 63 LRU); async fetch/decode workers; ≤4 uploads/frame |
| Vector import | Shapefile (`.shp`) and GeoJSON → the existing batched plot pipeline (points, polylines, polygon fills); one shared palette; startup import with skipped/hole counters |
| Icons | 1024 instanced textured quads with per-instance quaternion (`T·R·S`), three-axis rotation demo; drawn on the 2D plane and on the WGS84 surface with a tangent frame, one draw each |
| Vector plotting | Batched points (SDF discs), polylines (screen-space ribbons), polygon fills, circles and sectors on both 2D and 3D; 3 draws total, geometry rebuilt only when the mode/ellipsoid changes or the document is edited |
| Vector tiles | MVT (`.mvt`) tiles from HTTP templates or local directories, decoded with geozero, merged into the same plot batches; visible 2D tiles / 3D LOD leaves drive loading, with LRU eviction and a 50k-shape cap |
| Plot editing | `PlotDocument` with stable per-shape ids, selection highlight, click/drag to select and move, `Delete`/`Backspace`, `Ctrl+Z` undo (whole-document snapshots, depth 64); CPU screen-space picking, no GPU readback |
| Screenshot | `EW_GIS_SCREENSHOT=<path>` renders offscreen and writes a PNG |
| HUD | Window title: mode, icon count, plot selection, FPS, frame time, tile stats |
| Zed host | `Help > ew-gis` embeds the renderer in a GPUI window (or `ZED_EW_GIS_OPEN=1` at startup): a render thread owns a headless viewer and the window presents its readback frames as `RenderImage`s; no separate `ew_gis.exe` process |

## Run

```powershell
# default 2D plane, 1024 rotating icons
cargo run -p ew_gis

# 3D globe
cargo run -p ew_gis -- --mode 3d

# offscreen screenshot (waits for visible tiles)
$env:EW_GIS_SCREENSHOT='target\ew-gis-3d.png'
cargo run -p ew_gis -- --frames 30 --mode 3d

# from Zed: Help > ew-gis opens the viewer in a GPUI window of the editor process
# (ZED_EW_GIS_OPEN=1 opens it at startup instead of clicking the menu)
$env:ZED_EW_GIS_OPEN='1'
```

### Controls

| Input | 2D | 3D |
| --- | --- | --- |
| Left drag (empty space) | Pan | Orbit (yaw/pitch) |
| Left click on a plot shape | Select + start dragging that shape | Select + start dragging that shape |
| Left drag after a shape is grabbed | Move the shape | Move the shape |
| `Delete` / `Backspace` | Delete the selected shape | Delete the selected shape |
| `Ctrl+Z` | Undo the last edit (drag or delete) | Undo the last edit |
| `Esc` | Clear the selection (press again to quit) | Clear the selection (again to quit) |
| Wheel | Zoom (camera height) | Zoom (distance) |
| `Tab` | Switch to 3D | Switch to 2D |

Picking is CPU screen-space: layers are tried in the order **point > polyline >
polygon > sector**, and inside a layer the nearest shape wins. A point is picked
within `max(size_px / 2, 7 px)`, a line within `width_px / 2 + 5 px`, an area when
the cursor is inside its projected outer ring or within `width_px / 2 + 5 px` of
its edge. On the globe, shapes on the far side of the Earth are skipped by the
same horizon test the LOD uses.

### Scripted input (screenshots and regression)

```powershell
# press the left button on the polygon at 620,570 px and drag it to 760,640
cargo run -p ew_gis -- --mode 2d --frames 8 --click-at 620,570 --drag-to 760,640 `
  --screenshot target\ew_gis_drag.png

# same gesture, but the press pixel is the projected centre of shape 201
cargo run -p ew_gis -- --mode 3d --zoom 8 --frames 14 --click-plot 201 --drag-by 220,-140 `
  --screenshot target\ew_gis_drag_3d.png

# document-level edits (no mouse): delete the 200 km circle, then undo it
cargo run -p ew_gis -- --mode 2d --select-plot 202 --delete-plot --undo-plot `
  --frames 6 --screenshot target\ew_gis_undo.png
```

`--click-at` / `--click-plot` go through the real `MouseInput`/`CursorMoved`
handlers (press, one intermediate move, release), so the acceptance screenshots
exercise the same path a user does. `--select-plot N` / `--move-plot DX,DY` /
`--delete-plot` / `--undo-plot` are the deterministic document-level equivalents;
`N` is the flattened shape index (points 0..199, then polyline 200, polygon 201,
circle 202, sector 203). `--pick-probe X,Y` logs what a click would hit and
`--plot-at N` moves the scripted actions off frame 2.

### Imagery providers

`--imagery` (or `EW_GIS_IMAGERY`) selects the scheme; the fallback chain is
still **online imagery → local tile directory → procedural**. `--online` enables
the fetch, `--offline` keeps local + procedural, `--procedural` forces the
generator. `--online/--offline/--procedural` are independent of the kind, so
`--online --imagery wms ...` fetches WMS and `--offline` never touches the
network.

| Kind | URL rule |
| --- | --- |
| `xyz` (default) | template with `{z}`, `{x}`, `{y}`, `{s}` (`{s}` cycles `a`/`b`/`c`) |
| `tms` | same template, tile row flipped: `{z}/{x}/{2^z-1-y}` |
| `wms` | `GetMap` with `SERVICE=WMS`, `VERSION`, `REQUEST=GetMap`, `LAYERS`, `STYLES`, `FORMAT`, `TRANSPARENT=TRUE`, `WIDTH=512`, `HEIGHT=512`, `BBOX` in `EPSG:3857` metres, and `CRS` (1.3.0) or `SRS` (1.1.1) |
| `arcgis` | `{base}/tile/{z}/{y}/{x}` (row before column) |
| `single` | one world image read/downloaded at startup, resized to the 512² layer-0 base map and cropped per tile (equirectangular) |
| `local` | no provider: local `EW_GIS_TILE_DIR` tree only (sets the offline source) |

The WMS bbox uses `x_min = x/2^z·2πR - πR`, `x_max = (x+1)/2^z·2πR - πR`,
`y_max = πR - y/2^z·2πR`, `y_min = πR - (y+1)/2^z·2πR` with `R = 6378137 m`.
`Single` has no per-tile URL; if the image cannot be read/downloaded the world
layer falls back to the procedural map. Tile crops are still resized to 512² so
they fit the atlas layer format.

| Flag | Environment | Meaning |
| --- | --- | --- |
| `--imagery KIND` | `EW_GIS_IMAGERY` | `xyz`\|`tms`\|`wms`\|`arcgis`\|`single`\|`local` |
| `--tile-url` | `EW_GIS_TILE_URL` | XYZ template |
| `--tms-url` | `EW_GIS_TMS_URL` | TMS template |
| `--arcgis-url` | `EW_GIS_ARCGIS_URL` | ArcGIS MapServer base |
| `--wms-url` / `--wms-layers` | `EW_GIS_WMS_URL` / `EW_GIS_WMS_LAYERS` | WMS endpoint + `LAYERS` |
| `--wms-styles` / `--wms-format` | `EW_GIS_WMS_STYLES` / `EW_GIS_WMS_FORMAT` | `STYLES`, `FORMAT` (default `image/png`) |
| `--wms-version` / `--wms-crs` | `EW_GIS_WMS_VERSION` / `EW_GIS_WMS_CRS` | `VERSION` (default `1.3.0`), `CRS`/`SRS` (default `EPSG:3857`) |
| `--single-image` | `EW_GIS_SINGLE_IMAGE` | path or `http(s)` URL of the single world image |
| | `EW_GIS_TILE_DIR` / `EW_GIS_TILE_TEMPLATE` | local tile tree |
| | `EW_GIS_GLOBE_MAX_Z` | quadtree max level (default 5) |
| | `EW_GIS_GLOBE_TARGET_PX` | screen-space error target (default 320 px) |
| | `EW_GIS_GLOBE_ELLIPSOID=sphere` | use a sphere instead of WGS84 |
| | `EW_GIS_DUMP_WORLD` | dump the layer-0 world base map |

```powershell
# NASA GIBS WMS, WebMercator bbox per tile
cargo run -p ew_gis -- --online --imagery wms `
  --wms-url https://gibs.earthdata.nasa.gov/wms/epsg3857/best/wms.cgi `
  --wms-layers BlueMarble_ShadedRelief_Bathymetry --mode 2d

# one equirectangular world image as the base map (tiles are cropped from it)
cargo run -p ew_gis -- --imagery single --single-image target\ew_gis_blue_marble_world.jpg
```

### Vector data import

`--geojson PATH` and `--shape PATH.shp` are repeatable and merge into the plot
document; `--no-demo` drops the 200-point / line / polygon / sector demo shapes
so only the imported data is visible. A missing or broken file is logged and
skipped, never fatal.

```powershell
# fixtures used by the acceptance screenshots
cargo run -p ew_gis --example vector_fixtures

# GeoJSON: 3 points + 1 line + 1 polygon, 2D and 3D
cargo run -p ew_gis -- --mode 2d --geojson target\ew_gis_east_asia.geojson `
  --no-demo --zoom 4 --frames 10 --screenshot target\ew-gis-vector-2d.png
cargo run -p ew_gis -- --mode 3d --geojson target\ew_gis_east_asia.geojson `
  --no-demo --zoom 4 --frames 10 --screenshot target\ew-gis-vector-3d.png

# the same data as three shapefiles (points, polylines, polygons)
cargo run -p ew_gis -- --mode 2d --shape target\ew_gis_east_asia_points.shp `
  --shape target\ew_gis_east_asia_lines.shp `
  --shape target\ew_gis_east_asia_polygons.shp --no-demo --zoom 4 `
  --frames 10 --screenshot target\ew-gis-vector-shapefile-2d.png
```

| Item | Behaviour |
| --- | --- |
| Shapefile | Point/PointM/PointZ, MultiPoint (+M/Z), Polyline (+M/Z), Polygon (+M/Z); `.dbf` attributes ignored |
| GeoJSON | FeatureCollection / Feature / bare Geometry, including GeometryCollection; Point, MultiPoint, LineString, MultiLineString, Polygon, MultiPolygon |
| Style | points 6 px, lines 2 px, polygon fill α 0.30 + 2 px outline α 0.95, 8-colour shared palette cycling per feature |
| Holes | interior rings are counted (`holes dropped` in the log) and not rendered |
| Skip | invalid/empty geometries, null features, unreadable records and anything beyond the cap are counted (`skipped` / `capped`), never abort the import |
| Cap | 50,000 shapes per file (`MAX_FEATURES`), bounding startup and first-frame cost |
| Projection | `.prj` is read but **not** applied: coordinates are assumed WGS84 (non-WGS84 is warned about, not reprojected) |

The startup log reports the per-file and total counts, e.g.

```
vector target/ew_gis_east_asia.geojson: 5 shapes (3 points / 1 lines / 1 polygons), 0 skipped, 0 holes dropped, 0 capped, 0.2 ms
vector import: 1 file(s), 5 shapes (3 points / 1 lines / 1 polygons), 0 skipped, 0 holes dropped, 0 capped; demo shapes off; total plot shapes 5; 0.5 ms
```

## Performance (measured, 1600×900 unless noted)

| Scenario | Frame time | Draw calls | Geometry |
| --- | --- | --- | --- |
| 2D, 1024 icons, 15 tiles | 16.6 ms / 60 FPS | 2 | one instanced tile pass + one icon pass |
| 3D quadtree | 16.4 ms / 61 FPS | 2 | 59 tiles, 21.7k vertices + surface icons |
| 2D + plot | 16.79 ms / 59.6 FPS | 5 | 570 fill triangles, 127 line instances, 200 points |
| 3D + plot | 16.81 ms / 59.5 FPS | 5 | 193 line instances, surface-tessellated fills |
| 2D plot edit (select/drag) | 16.4 ms / 65 FPS | 5 | rebuild 0.18–0.33 ms (tessellation 0.10–0.21 ms) |
| 3D plot edit (select/drag) | 16.4 ms / 61 FPS | 5 | rebuild 0.22–0.74 ms (tessellation 0.19–0.65 ms) |
| 3D 1080p | 16.63 ms / 60.1 FPS | 1 | 62 tiles, 22.8k vertices, rebuild 2.17 ms |
| 3D 4K | 16.68 ms / 59.9 FPS | 1 | LOD geometry fits the frame budget |
| 2D + 11.5k imported shapes | 16.00 ms / 62.5 FPS | 5 | 10,000 points + 4,000 line segments + 500 polygons: import 64.9 ms, rebuild 2.66 ms (tessellation 2.31 ms) |
| 2D WMS (NASA GIBS) | 16.64 ms / 60.1 FPS | 5 | 6 live WMS 512² tiles, all exact, ~7 s warmup on this network |
| 2D/3D single-image base | 16.2–16.5 ms / 61 FPS | 5 | 2048×1024 source decoded to the 512² world map in 0.87 s, 9/59 crops |

An edit (selection change, drag step, delete, undo) rebuilds the three batches
on the CPU and re-uploads them. Highlighting never adds a draw call: a selected
area re-colours its fill and outline, a selected line gains a translucent glow
pass plus a crisp stroke, and a selected point marker gains a halo instance, all
inside the existing three buffers.

## Architecture

```
src/
  lib.rs                 # the library: every module below is public
  viewer.rs              # windowless scene/input/renderer (headless GPU, offscreen frames)
  app.rs                 # winit ApplicationHandler around a Viewer: window, HUD, scripted runs
  main.rs                # `cargo run -p ew_gis`: Config + app::run
  data/                  # Shapefile / GeoJSON import into DemoShapes
  gpu/                   # device/surface or headless device, depth/offscreen frame, pipelines, WGSL
  math/                  # WGS84/ECEF (+inverse, ray-ellipsoid), plane mapping, cameras, LOD
  tiles/                 # imagery providers (xyz/tms/wms/arcgis/single), workers, LRU uploads
  scene/                 # plane tiles, globe mesh, icons, plot document + picking
  render/                # per-pass recorders (tile, globe, plot, icon)
  screenshot.rs          # offscreen readback → RGBA frame → PNG
examples/
  vector_fixtures.rs     # writes target/ew_gis_east_asia.{geojson,shp}
```

### Embedding the renderer

`ew_gis` builds both a library and the standalone binary. A host that has its own
window (Zed's GPUI `Help > ew-gis`) links the library and never starts `winit`:

```rust
use ew_gis::viewer::{Viewer, ViewerOptions};

// Blocking: creates a headless wgpu device. Call it on a worker thread.
let mut viewer = Viewer::new(ViewerOptions {
    width: 1280,
    height: 720,
    ..ViewerOptions::default()
})?;

// One frame: update the scene, render offscreen, copy the pixels back.
let frame = viewer.render_rgba()?;  // frame.bytes is tightly packed RGBA8

viewer.mouse_move(x, y);
viewer.mouse_down(x, y);
viewer.mouse_up();
viewer.wheel(1.0);
viewer.key_down(ew_gis::viewer::ViewerKey::Tab);   // 2D ↔ 3D
viewer.resize(width, height);
```

`render_rgba` blocks on the GPU readback, so it must not run on a UI executor.
`Viewer::with_gpu` accepts a surface-backed `GpuContext` instead of creating a
headless one; that is what the standalone host does, and `render_to_view` then
records straight into its swapchain texture without a readback.

### Design notes

- **2D is a 3D scene**: the map is a `z = 0` plane in a perspective camera. Icons are
  ordinary 3D quads with quaternions, so three-axis rotation needs no separate 2D
  path; a future 2D/3D switch only changes the camera and tile geometry.
- **f64 CPU / f32 GPU**: ECEF, camera and LOD are f64; vertices drop to f32 once.
  Render origin is currently the Earth center (~0.5 m f32 quantization, ~0.35 px at
  2 km altitude); moving the origin to the camera is the next precision step.
- **One draw per layer family**: visible globe tiles are merged into one dynamic
  vertex/index buffer; 2D tiles are instanced quads; icons are instanced quads.
- **No per-frame allocation** in the steady state: buffers are persistent; geometry
  is rebuilt only when the visible tile set or layer assignment changes.
- **Quadtree** (`math/lod.rs`): 4×4 root at z=2, screen-space error target, frustum
  and horizon culling, 0.85 hysteresis, near/high-level retention under the 62-tile
  budget (one spare layer avoids LRU ping-pong).
- **Plot document** (`scene/plot.rs`): authoring shapes keep their batched form;
  a parallel id table gives every shape a stable `ShapeId` (never reused), and the
  selection plus a bounded undo stack of whole-document snapshots live next to it.
  A drag records the shape as it was at press time and re-applies the *total*
  geographic delta, so a long gesture cannot drift and a press that never moved is
  not an undo step.
- **Picking** (`scene/pick.rs`): inverse view-projection in `f64` for both modes,
  screen-space hit tests per layer, horizon/front-face rejection on the globe, and
  a drag delta from the surface point under the press to the one under the cursor
  (`2D`: ray ∩ z=0 plane, `3D`: ray ∩ WGS84). The 2D result equals the classic
  `dx · meters_per_pixel` mapping, which the tests cross-check.

## Known limitations

- **Online endpoints**: from this development network the public OSM XYZ,
  OpenTopoMap and ArcGIS REST tile hosts time out, while WMS does work — the live
  online run uses NASA GIBS WMS (and WMS 1.1.1/1.3.0 URL rules are unit-tested).
  XYZ/TMS/ArcGIS requests therefore fall back to local/procedural here; their URL
  construction is covered by tests and any reachable endpoint works through the
  same fetch path.
- **Shapefile projection**: `.prj` is only inspected. Coordinates are assumed
  WGS84 and are **not reprojected**; a non-WGS84 `.prj` is logged as a warning.
  The `.dbf` attributes are ignored (no attribute-driven styling or labels yet).
- **Polygon holes**: interior rings are counted and logged, not rendered; only the
  outer ring becomes a fill.
- **MVT tile limitations**: polygon holes are counted and logged but not rendered
  (the fill batch has no hole support); latitude is linearly interpolated within a
  tile (exact Mercator inverse is a one-line change for high latitudes); a change
  in the visible tile set rebuilds the merged plot document, which clears plot
  selection/undo; failed tiles are not retried for the session.
- **No per-feature attribute model** yet: imported static data and MVT features are
  merged into the batched plot document (50,000-shape cap after static data).
- **Single imagery** is treated as an equirectangular world image (the same
  simplification Cesium's single-tile provider makes); a Mercator world image
  would be stretched at high latitudes.
- Polar cap reuses the 512² equirectangular world layer; it is hole-free but low
  resolution, and the WebMercator top row still converges near the pole.
- No MSAA/mipmaps yet; no reverse-Z (the ellipsoid is a single shell).
- 3D icon overlay is implemented (surface tangent frame + three-axis local rotation);
  labels are the next milestone.
- Plot picking is layer-priority, not z-aware: with 200 demo markers layered over the
  areas, a click within 7 px of a marker selects the marker even if it sits on top of
  a polygon. Zoom in (or use `--select-plot`) to reach the area underneath.
- Picking projects to the reference surface and ignores the ≤ 420 m plot layer lift;
  at the demo zoom that is far below one pixel.
- The globe camera uses the local north tangent as its up vector; `pitch_deg == 0`
  (which used to produce a degenerate `look_at`) is covered by a unit test.
- No cache eviction test under sustained flight; current steady state has zero
  pending uploads and no per-frame evictions.

## Screenshots (acceptance)

| File | Content |
| --- | --- |
| `target/ew-gis-imagery-wms-2d.png` | live NASA GIBS WMS tiles, 2D plane |
| `target/ew-gis-imagery-single-2d.png` | Blue Marble world image as the single provider, 2D |
| `target/ew-gis-imagery-single-3d.png` | the same single image on the WGS84 globe |
| `target/ew-gis-vector-2d.png` | imported GeoJSON (3 points, 1 line, 1 polygon), 2D |
| `target/ew-gis-vector-3d.png` | the same GeoJSON on the globe |
| `target/ew-gis-vector-shapefile-2d.png` | the same data read from three `.shp` files |
| `target/ew-gis-vector-large-2d.png` | 10k points / 1k lines / 500 polygons import |
| `target/ew-gis-mvt-2d.png` | MVT vector tiles (15 tiles, 45 features) on the 2D plane |
| `target/ew-gis-mvt-3d.png` | the same MVT layer on the 3D globe |
| `target/ew-gis-nogrid-2d.png` / `-3d.png` | no graticule and no tile-border grid |

## Roadmap

1. Camera-follow render origin + reverse-Z (near-ground precision).
2. Label atlas and icon picking/editing.
3. `.shp` reprojection (`.prj`), polygon holes, and MVT feature attributes.
4. Feed the plot document from the shared tactical model; add vertex handles,
   rotation and per-shape style editing.
4. Multi-select, rubber-band selection and redo.
5. Zed host polish: a status/HUD line, a 2D/3D control and picking inside the
   GPUI window (the frame stream and input forwarding are in place).
