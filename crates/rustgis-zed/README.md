# `rustgis_zed` — Zed's adapter onto the RustGIS library layers

RustGIS 0.6.0 lives in `crates/rustgis`, copied in whole and **kept byte-identical** to
upstream (`upstream.sha256`, 970 files, verified after every change). It keeps its own
`[workspace]` root (`resolver = "3"`, `edition = "2024"`, `rust-version = "1.90"`,
`[workspace.lints]`), so its crates resolve `workspace = true` against that manifest; the root
`Cargo.toml` therefore `exclude`s `crates/rustgis` and only `crates/rustgis-zed` (this crate) is a
member of Zed's workspace.

**Nothing in this crate may edit a file under `crates/rustgis`.** All adaptation lives here. Zed
itself depends on `rustgis_zed` alone and never names a `rustgis-*` layer, so the layering of the
copied tree (`cargo xtask layers`; L6 = egui/wgpu/winit) stays intact, and `rustgis-ui-egui` is
deliberately not a dependency.

## Architecture

```text
Help > RustGIS
  crates/framework/zed/src/zed/rustgis.rs          GPUI window (UI thread)
    EngineCommand  ────── bounded queue (8) ──────▶  worker thread: MapEngine
    EngineEvent    ◀───── bounded queue (4) ───────    └─ rustgis_engine::Session (not Send)
      Frame(BGRA, quality, elapsed)                         ├─ rustgis_render  (tiny-skia)
      Status(camera, layers, notices)                       ├─ rustgis_map     (layers, camera)
                                                           ├─ rustgis_geom    (projection)
                                                           └─ rustgis_data    (GeoJSON, …)
                                                       crates/rustgis (verbatim upstream 0.6.0)
```

* `MapEngine` — a headless `rustgis_engine::Session` plus the frame size.
* `worker` — moves the engine onto its own thread (`spawn_engine` → `EngineCommand` in,
  `EngineEvent` out), because `Session` is not `Send` and a frame is tens to hundreds of
  milliseconds.
* `Quality` — `Full` (settled; labels when `set_labels` is on) or `Preview { divisor }` (a gesture
  frame: canvas divided by 2–8, labels off, antialiasing off for that frame only).
* `BgraFrame` / `EngineFrame` — straight BGRA in GPUI's atlas order, converted on the engine
  thread in the same pass that undoes `tiny-skia`'s premultiplication.

| method | RustGIS path |
|---|---|
| `MapEngine::new()` | `rustgis_engine::sample::world_project()` (World Atlas) |
| `load_geojson` / `load_geojson_named` | `rustgis_data::io::read_geojson` → `Map::add_dataset` |
| `render_rgba(_quality)` | `rustgis_render::render_placed` (labels, full) or `render` (preview/no labels) |
| `render_bgra(_quality)` | the same frame in GPUI's BGRA order, for the window |
| `render_png(w, h)` | `rustgis_render::render_png` (labels, scale bar, credits) |
| `pan(dx, dy)` | `Camera::pan_px` — **not** `Session::run` |
| `zoom_at(x, y, factor)` / `zoom_center(factor)` | `Camera::zoom_at` — **not** `Session::run` |
| `zoom_world()` | `Map::fit_lonlat` over RustGIS's World extent — **not** `Session::run` |
| `command(id, params)` | `Session::run(id, &params)`, for data/edit commands; logs its time and undo depth |
| `layers()` / `tile_layers()` / `commands()` | `Map` and `Session::registry` |
| `take_notices()` | `Session::notice_mark` / `notices_since` |
| `capabilities()` | `capabilities::of_engine`, L0–L5 availability |

`Frame` is **straight (non-premultiplied) RGBA**; `BgraFrame` is the same bytes in BGRA, ready
for GPUI. Both are row-major, top-left first.

## Interaction and performance

The window keeps **at most one frame in flight**, folds every input event since the last frame
into one `Interact`, and sends at most one interaction frame every 33 ms. A gesture draws a
preview (half canvas, no labels, no antialiasing); 150 ms after the last movement — or right after
mouse-up — one `Settle` draws the full-quality frame. If a preview misses 33 ms (a debug build),
the worker halves its canvas once more; a fast machine stays at half resolution. While a pan is
in flight the displayed frame is shifted by the pointer delta that has not been drawn yet, so
dragging feels immediate even before the next frame lands.

Navigation never goes through `Session::run`:

* `map.zoomIn` / `map.zoomOut` / `map.zoomWorld` are `view()` commands (`mutates: false`), so
  they take **no undo snapshot** — but they still run the session's registry lookup, project-log
  entry and notice handling. The measured cost is ~0.02 ms; it is not the bottleneck. The window
  uses the camera API for buttons, wheel and keys because that is the right path for gestures,
  not because `Session::run` clones the project.
* Real data/edit commands still go through `MapEngine::command` → `Session::run`, and log
  `[rustgis] Session::run(<id>) <ms>, undo <before> -> <after> (snapshot: …)`.

The costs that did matter, measured with `examples/frame_timing.rs` on this machine (median of
5; 1200×800; debug = what `cargo build -p zed` produces):

| measurement | before | after |
|---|---:|---:|
| interaction frame (was: full frame per mouse event) | 611 ms | **24.6 ms** preview (16.3 ms at divisor 4) |
| settled full frame with labels | 611 ms | **132 ms** |
| full frame without labels | 508 ms | 102 ms |
| `render_placed` vs `render` | +~100 ms for label placement | same |
| UI thread per-frame RGBA→BGRA clone+swap | 34 ms | **0.4 ms** (bytes moved, no pixel loop) |
| `Session::run(map.zoomIn)` | 0.02 ms | 0.02 ms (not the cause) |
| camera `pan`/`zoom_at` | ~0 ms | ~0 ms |
| duplicate frame (cache hit) | — | 0.7 ms |

In `--release` the same harness reports **16.2 ms** for a half-canvas preview, **28.3 ms** for a
full frame with labels, and **0.0 ms** of UI-thread pixel work.

The real `target\debug\zed.exe` (isolated `--user-data-dir`, `ZED_RUSTGIS_OPEN=1`, 125% DPI)
logged preview frames of **9.8–23.7 ms** while a drag was in flight and full settles of
**49.6–135 ms** after it, with the status line showing `preview 600×371 · 22 ms` /
`full 1200×742 · 135 ms`. That is the acceptance target (preview ≤ 33 ms, settle ≤ 300 ms) on the
debug profile `cargo build -p zed` produces.

The debug profile optimises the RustGIS render path only (`[profile.dev.package]` in the root
`Cargo.toml`: `rustgis-*`, `tiny-skia`, `lyon*`, `ab_glyph`). With those crates at the default
`opt-level = 0`, even a 300×200 preview costs ~78 ms and no amount of input coalescing can reach
30 fps; the rest of a debug Zed stays unoptimised.

The other per-frame costs, checked rather than assumed:

* **No undo snapshot on zoom.** `map.zoomIn`/`zoomOut`/`zoomWorld` are registered with `.view()`
  (`mutates = false`), so `Session::guarded` takes `snapshot = None`. Logged undo depth is
  unchanged across the call.
* **No tile or layer re-read.** The sample project has no web tile layers (`tile_layers()` is
  empty), so `render_placed`'s tile-wait loop exits immediately; labels are the extra cost
  (~100 ms debug, ~9 ms release). Layers are redrawn, not re-parsed.
* **No projection cache to add.** `rustgis_render` projects each visible feature per frame and
  owns its own per-layer caches; the adapter cannot cache inside it without editing the copied
  tree. What the adapter caches is the finished frame: `RenderKey` is (size, quality, labels,
  content revision, camera, byte order), so a resize that lands on the same size or a settle that
  changed nothing is a 0.7 ms copy instead of a full render.
* **No `Session` call on the UI thread.** The window only converts the BGRA frame into a
  `RenderImage` (a move) and lays out GPUI elements.
* **Bounded queues.** `EngineHandle::send` uses `try_send`; a full queue reports "the RustGIS
  engine is busy" instead of growing, and the event channel blocks the worker if the host stops
  reading.

## Building

```powershell
$env:CARGO_HTTP_CHECK_REVOKE = "false"   # only if cargo must fetch crates; see below
cargo check -p rustgis_zed --all-targets
cargo test  -p rustgis_zed
cargo check -p zed --all-targets
cargo build -p zed
```

`cargo test -p rustgis_zed` writes `target/rustgis-zed-poc.png` (800×600, the sample project) so
you can look at what the window shows without opening one.

### Sample data, and why the build is offline

`crates/rustgis/crates/render/build.rs` downloads five Natural Earth GeoJSON files
(`countries`, `countries_chn`, `cities`, `rivers`, `lakes`) with `curl` into `OUT_DIR` unless
`RUSTGIS_DATA_DIR` is set. The copy under `sample-data/` is checked in, and the root
`.cargo/config.toml` points the build at it:

```toml
[env]
RUSTGIS_DATA_DIR = { value = "crates/rustgis-zed/sample-data", relative = true }
```

so a clean build needs no network and the embedded sample data is pinned by this repository. If
you build the copied tree on its own, set `RUSTGIS_DATA_DIR` to that folder (or to any folder
holding the five files) first.

### Getting the crates first (`CARGO_HTTP_CHECK_REVOKE=false`)

On this machine (Windows + `schannel`), cargo's TLS handshake to `index.crates.io` fails with
`transfer too slow: failed to transfer more than 10 bytes in 30s` on every attempt, because the
certificate-revocation check cannot reach the network (`curl.exe` shows the same failure as
`CRYPT_E_REVOCATION_OFFLINE`). Fetch the new crates once with:

```powershell
$env:CARGO_HTTP_CHECK_REVOKE = "false"
cargo fetch
```

`CARGO_HTTP_MULTIPLEXING=false` is also worth setting if the index stalls. This is an environment
workaround, not a repository setting: it is deliberately **not** in `.cargo/config.toml`.

## Looking at it on the machine

The window opens from `Help > RustGIS`, or at startup with the test hooks:

```powershell
$env:ZED_RUSTGIS_OPEN = "1"
$env:ZED_RUSTGIS_LOAD = "crates/rustgis-zed/sample-data/cities.geojson"
target\debug\zed.exe
```

Use a separate profile (`--user-data-dir`) so the check never touches a real Zed instance. Every
frame logs one `[rustgis]` line with its size, quality and milliseconds; a gesture shows a
`preview` line per frame and one `full` line after it settles:

```text
[rustgis] layout: GPUI viewport 1200.0x800.0, client 1200x800, DWM frame 1201.6x831.2, delta 0.0x0.0
[rustgis] 600x371 Preview { divisor: 2 } frame in 22.5 ms (preview divisor 2)
[rustgis] UI: 600x371 Preview { divisor: 2 } in 0.2 ms (render 22.5 ms)
...
[rustgis] 1200x742 Full frame in 47.5 ms (preview divisor 2)
```

On a >100% DPI display `GetWindowRect` is virtualised while DWM's extended frame is not; the
`client`/`DWM frame`/`delta` line is what makes a layout mismatch visible instead of guessed, and
the window is laid out for the smaller of GPUI's viewport and that client area.

`examples/frame_timing.rs` reproduces the table above without a window:

```powershell
cargo run -p rustgis_zed --example frame_timing
```

## Verifying that `crates/rustgis` is untouched

`upstream.sha256` lists `sha256  relative/path` for all 970 files. After any change, from the
repository root:

```powershell
$changed = 0
Get-Content crates\rustgis-zed\upstream.sha256 | ForEach-Object {
    if ($_ -match '^([0-9a-f]{64})  (.+)$') {
        $path = Join-Path (Get-Location) $Matches[2]
        if (-not (Test-Path -LiteralPath $path) -or
            (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLower() -ne $Matches[1]) {
            "changed: $path"
            $changed++
        }
    }
}
"changed=$changed"   # must be 0
```

`git status --porcelain -- crates/rustgis` must also be empty.

## Upgrading RustGIS

The copy is replaced **whole**, never patched:

1. Note the new upstream release (the tree's `Cargo.toml` version) and check its licence and
   MSRV against `crates/rustgis-zed/Cargo.toml` (`edition`/`rust-version` of the root toolchain).
2. Replace the directory: delete `crates/rustgis` and copy the new release's tree into the same
   place, keeping its top-level `Cargo.toml` (the `[workspace]` root) and `crates/` layout.
3. Regenerate the hash list (paths relative to the repository root, forward slashes, sorted):

   ```powershell
   $root = (Get-Location).Path
   Get-ChildItem crates\rustgis -Recurse -File |
       ForEach-Object {
           "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(),
               ($_.FullName.Substring($root.Length + 1) -replace '\\', '/')
       } |
       Sort-Object |
       Set-Content crates\rustgis-zed\upstream.sha256 -Encoding ascii
   ```

4. Fix this adapter only if the public API it uses moved: `Session`, `Project`, `Map`, `Camera`,
   `View`, `render`/`render_placed`/`render_png`, `read_geojson`. The compiler is the checklist.
5. Re-run the whole gate: `cargo check -p rustgis_zed --all-targets`, `cargo test -p rustgis_zed`,
   `cargo check -p zed --all-targets`, `cargo build -p zed`, then the `changed=0` check above and
   a `ZED_RUSTGIS_OPEN=1` screenshot.

## Dependencies

Path dependencies into `crates/rustgis/crates/`: `geom` (L0), `data` (L1), `map` / `tiles` /
`analysis` / `keychain` (L2), `render` / `plugins` / `geolibre` (L3), `engine` (L4), `assistant`
(L5). They came in in stages, one `cargo check -p rustgis_zed` per stage, so the cost of each is
known:

| stage | added | why |
|---|---|---|
| 1 | `geom`, `data`, `map`, `tiles`, `render`, `engine` | the minimum that draws: `engine` reaches `analysis` and `plugins` in turn |
| 2 | `keychain` | L2 secret store, availability check only |
| 3 | `geolibre` | L3 tool suite, availability check only (needs the `geolibre` program) |
| 4 | `assistant` | L5 agent loop, availability check only (needs provider credentials) |

`rusqlite` is the reason this integration is not a drop-in: RustGIS pins `0.41` /
`libsqlite3-sys 0.39`, `libsqlite3-sys` declares `links = "sqlite3"` (one version only), so the
root workspace moved to `rusqlite 0.41` and `crates/admin`'s `toasty` chain is patched to match
(`tools/toasty-sqlite-patch/`).
