# `rustgis_zed` — Zed's adapter onto the RustGIS library layers

RustGIS 0.6.0 lives in `crates/rustgis`, copied in whole and **kept byte-identical** to
upstream (`upstream.sha256`, 970 files, verified at every stage of the integration). It keeps its
own `[workspace]` root (`resolver = "3"`, `edition = "2024"`, `rust-version = "1.90"`,
`[workspace.lints]`), so its crates resolve `workspace = true` against that manifest; the root
`Cargo.toml` therefore `exclude`s `crates/rustgis` and only `crates/rustgis-zed` (this crate) is a
member of Zed's workspace.

**Nothing in this crate may edit a file under `crates/rustgis`.** All adaptation lives here. Zed
itself depends on `rustgis_zed` alone and never names a `rustgis-*` layer, so the layering of the
copied tree (`cargo xtask layers`; L6 = egui/wgpu/winit) stays intact, and `rustgis-ui-egui` is
deliberately not a dependency.

## What it exposes

`MapEngine` — a headless `rustgis_engine::Session` plus the frame size:

| method | RustGIS path |
|---|---|
| `MapEngine::new()` | `rustgis_engine::sample::world_project()` (World Atlas) |
| `load_geojson` / `load_geojson_named` | `rustgis_data::io::read_geojson` → `Map::add_dataset` |
| `render_rgba(w, h)` | `rustgis_render::render_placed` (labels) or `rustgis_render::render` |
| `render_png(w, h)` | `rustgis_render::render_png` (labels, scale bar, credits) |
| `pan(dx, dy)` | `Camera::pan_px` |
| `zoom_at(x, y, factor)` | `Camera::zoom_at` |
| `center_on(lon, lat)` | `rustgis_geom::lonlat_to_merc` + `Camera` |
| `command(id, params)` | `Session::run(id, &params)` |
| `layers()` / `tile_layers()` / `commands()` | `Map` and `Session::registry` |
| `take_notices()` | `Session::notice_mark` / `notices_since` |
| `capabilities()` | `capabilities::of_engine`, L0–L5 availability |

`worker` moves the engine onto its own thread (`spawn_engine` → `EngineCommand` in,
`EngineEvent` out), because `Session` is not `Send` and a frame can take tens of milliseconds: a
UI thread must never draw one.

`Frame` is **straight (non-premultiplied) RGBA**, row-major, top-left first — `tiny-skia` keeps
its pixmaps premultiplied and `render_rgba` undoes that once, so a host's texture path treats
alpha as ordinary coverage.

## Building

```powershell
cargo check -p rustgis_zed --all-targets
cargo test -p rustgis_zed
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

so a clean build needs no network and the embedded sample data is pinned by this repository.

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
