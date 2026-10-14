# CesiumRust

A Rust rewrite of [CesiumJS](https://cesium.com) — an open-source 3D globe and geospatial visualization engine.

Uses **hexagonal architecture** (ports & adapters) to separate pure domain logic from framework and IO concerns. The rendering layer is built on [Bevy](https://bevyengine.org), a data-driven game engine for Rust.

> **Architecture deep-dive:** See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for a comprehensive 3000+ word architecture document covering design decisions, module reference, dependency graph, data flow, and contributing guide.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  APPLICATION                         │
│         cesium-app (Bevy App assembly)               │
├─────────────────────────────────────────────────────┤
│                     PORTS                            │
│   driving (app APIs)    ·    driven (IO contracts)   │
├──────────────────────┬──────────────────────────────┤
│      ADAPTERS        │           ADAPTERS            │
│   bevy-render        │  plot-bevy · pipeline         │
│                      │  network · decoders           │
├──────────────────────┴──────────────────────────────┤
│                     DOMAIN                           │
│   18 crates  ·  pure Rust  ·  framework-free         │
│   Bit-exact fidelity with CesiumJS                   │
└─────────────────────────────────────────────────────┘
```

| Layer | Crates | Description |
|-------|--------|-------------|
| **Domain** | 18 | Pure Rust logic — geometry, time, camera, imagery, globe, quadtree, plot, datasource, material, atmosphere, shadow, scene-mode, KML, GPX, animation, event, interaction, resource. No framework dependency. |
| **Ports** | 2 | Trait contracts. `driving` defines application-facing APIs; `driven` defines IO interfaces that adapters implement. |
| **Adapters** | 5 | `bevy-render` (Bevy ECS + wgpu rendering), `plot-bevy` (2D/3D plotting overlay), `pipeline` (gated fetch/decode/evict core), `network` (ureq HTTP client), `decoders` (image/gzip decoding). |
| **Application** | 1 | `cesium-app` — assembles the Bevy app, wires adapters to domain, launches the interactive 3D globe viewer. |

> **Scope note (2026-09):** the project targets *efficient 2D/3D globe imagery display plus 2D/3D
> navigation plotting*. The 3D Tiles / terrain / glTF / styling chains and the CesiumJS
> Entity/DataSource visualization route were removed as out of scope; see
> [docs/SCOPE_2D3D_IMAGERY_PLOT.md](docs/SCOPE_2D3D_IMAGERY_PLOT.md) for the rationale,
> the full deletion record and the retained ("reserve") capabilities.

## Plugins

The `cesium-bevy-render` crate exports 7 plugins at its root (plus `headless::CesiumHeadlessPlugin`).
`cesium-app` registers only part of them — the rest are exported for embedders. "Registered?" below is
verified against `application/cesium-app/src`:

| Plugin | Registered by `cesium-app`? | Purpose |
|--------|------------------------------|---------|
| `CesiumCorePlugin` | ✅ always (`main.rs:451`) | Initialize GlobeConfig, RenderScale, TileLoadStats, AnimationClock, scene lighting |
| `CesiumAtmospherePlugin` | ✅ gated `CESIUM_ENABLE_SKYDOME` (`main.rs:510`) | Sky atmosphere, celestial bodies |
| `CesiumShadowPlugin` | ✅ only under `LightingMode::DayNight` (`main.rs:499`) | Shadow maps |
| `CesiumMaterialPlugin` | ⚙️ only via `MaterialShowcasePlugin` (opt-in, `material_showcase.rs:50`) | Material system resources |
| `FabricMaterialPlugin` | ⚙️ only via `MaterialShowcasePlugin` (opt-in) | Fabric shader material management |
| `CesiumCameraPlugin` | ❌ exported but **not** registered — the app drives `CameraControlPort` + `camera_control_port_system` directly (`main.rs:568-569`) | Camera orbit/flight controllers, scene mode switching |
| `CesiumPipelinePlugin` | ❌ exported but **not** registered — `dynamic_globe` uses `cesium_pipeline::DefaultBudget` directly (`dynamic_globe.rs:46`) | Gated tile pipeline core (`cesium-pipeline`) |

`headless::CesiumHeadlessPlugin` is added only in headless mode (`main.rs:675`).

### Local Plugins

| Plugin | Purpose |
|--------|---------|
| `OrbitCameraPlugin` | Mouse orbit/zoom with grab-the-globe tracking |
| `StarfieldPlugin` | Procedural starfield background |
| `AtmosphereGlowPlugin` | Atmospheric limb glow around globe edge |
| `DynamicGlobePlugin` | View-dependent LOD tiles + Bing Maps imagery (**primary render path**) |
| `BaseSpherePlugin` | Static base sphere + polar caps safety net |
| `Map2dPlugin` | 2D map mode + mode-switch button (windowed only) |
| `MaterialShowcasePlugin` | Fabric material showcase (opt-in) |
| `PerfTracePlugin` | CSV perf trace + camera-script replay (opt-in) |
| `CesiumPlotBridgePlugin` | 2D/3D plotting overlay bridge (`cesium-plot-bevy`, windowed only) |

## Getting Started

```bash
# Run the interactive 3D globe viewer
cargo run

# Run the full test suite
cargo test --workspace

# Check compilation (fast, no binary)
cargo check
```

**Prerequisites:** Rust 1.80+, system dependencies for Bevy (libudev, alsa, vulkan). On Ubuntu:

```bash
sudo apt install libudev-dev libasound2-dev libxkbcommon-dev vulkan-validationlayers
```

## Controls

| Action | Input |
|--------|-------|
| Orbit / rotate globe | Left mouse drag |
| Tilt (change viewing angle, 3D only) | **Middle mouse drag** (vertical tilts pitch, horizontal turns heading) |
| Zoom in/out (change view height) | Mouse wheel, or **right mouse drag** (vertical) |
| Switch 2D / 3D map mode | On-screen mode selector (windowed only) |

> Zoom follows CesiumJS's default `ScreenSpaceCameraController.zoomEventTypes`
> (`[RIGHT_DRAG, WHEEL, PINCH]`): dragging **up** zooms in (approaches the surface), dragging **down** zooms out —
> the same direction as the wheel. This deliberately inverts CesiumJS's `handleZoom` (`approachingSurface = diff > 0`,
> where dragging *down* approaches), because the wheel and the "pull closer by dragging up" reading are the ones that
> feel right here. The target height is anchored to the surface distance at the moment the button went down and follows
> `height_at_press × e^(total_dy / window_height · ZOOM_DRAG_RANGE)`, clamped by `[min_distance, max_distance]`: a
> full-window drag is ≈4.5×, and the vertical `total_dy` is the **cursor's displacement in window logical pixels**
> (`zoom_drag_advance`), not raw device counts — the latter scales with mouse DPI, so the same hand movement would zoom
> by an arbitrary factor and one drag could throw the globe out to `max_distance`. A single drag is capped at
> `e^ZOOM_DRAG_MAX_SPAN ≈ 9.5×` (the cursor cannot leave the window anyway), so travelling further means releasing and
> dragging again, or using the wheel (1.43× per notch); when no `Window` is available the raw-motion fallback is
> rate-limited to 144 px/frame so an input burst cannot teleport the camera. When the cursor leaves and re-enters the
> window (winit drops the position on `CursorLeft`) the two input paths are stitched by re-anchoring, so the height
> stays continuous instead of jumping. While the right button is down the height
> tracks the cursor directly; the easing that the wheel uses (it keeps discrete notches from reading as tile jitter)
> would only show up as drag lag. The gesture is reversible and — unlike a
> per-frame multiplicative step — the feel does not change with the frame rate (the earlier per-frame form reached 50%
> per frame and felt far too fast). See
> `application/cesium-app/src/orbit_camera.rs::zoom_drag_target_surface` and `::zoom_drag_advance`.
> Rotation stays on the left button (`rotateEventTypes = LEFT_DRAG`); while the plot bridge is drawing it
> holds `PlotInputCapture`, and both gestures yield to it.
> Tilt follows the first entry of CesiumJS's `tiltEventTypes` (`[MIDDLE_DRAG, PINCH, {LEFT_DRAG, CTRL},
> {RIGHT_DRAG, CTRL}]` → `tilt3D` → `tilt3DOnEllipsoid`): the pivot is the surface point under the centre of
> the canvas and the camera orbits it, leaving the geocentric orbit state (and therefore zoom, LOD and the
> inertial spin) untouched. The visible difference from "looking at the geocentre" is carried in a single
> extra quaternion (`OrbitTilt::aim`). Rate and mixing match `rotate3D` (`clamp(rho − 1, 1/5000, 1.77)` ×
> `min(±pixel / window size, maximumMovementRatio)`), including the "cannot tilt further down in the wrong
> direction" guard — dragging **up** raises the view. See
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §Controls.
> There are currently **no keyboard bindings** in `cesium-app` (`KeyCode::` has zero matches).
> The former `F1`/`F2`/`F3` debug overlays (bounding volumes, tile stats, LOD debug) were removed
> together with the widget/debug modules in the P0/P1 scope cut — see
> [docs/SCOPE_2D3D_IMAGERY_PLOT.md](docs/SCOPE_2D3D_IMAGERY_PLOT.md) §3.1.1/§3.2.1.

## Project Structure

```
cesiumrust/
├── Cargo.toml                 # Workspace root (29 crates)
├── domain/                    # Domain layer — 18 crates
│   ├── geospatial/            # Cartesian, cartographic, geographic math (f64)
│   ├── time/                  # Julian date, TAI, UTC
│   ├── camera/                # Camera controllers, frustum culling
│   ├── event/                 # Event system
│   ├── resource/              # Async resource loading
│   ├── imagery/               # Imagery layers, tile requests, alpha blending
│   ├── globe/                 # Ellipsoid globe surface, lighting params
│   ├── quadtree/              # Quadtree tile subdivision
│   ├── plot/                  # 2D/3D navigation plotting model (geometry, documents)
│   ├── datasource/            # Generic data source / entity model
│   ├── material/              # Material system, shader abstractions
│   ├── atmosphere/            # Star sphere, celestial + scattering math
│   ├── shadow/                # Shadow maps
│   ├── scene-mode/            # 2D / 2.5D / 3D scene modes
│   ├── interaction/           # Mouse/keyboard/touch input
│   ├── kml/                   # KML parsing
│   ├── gpx/                   # GPX parsing
│   └── animation/             # Keyframe animation, splines
├── ports/
│   ├── driving/               # Application API trait contracts
│   └── driven/                # IO trait contracts (renderer, network, loader)
├── adapters/
│   ├── bevy-render/           # Bevy ECS + wgpu rendering adapter
│   ├── plot-bevy/             # 2D/3D plotting overlay adapter (panel, UI, view sync)
│   ├── pipeline/              # Gated fetch/decode/evict tile pipeline core
│   ├── decoders/              # Image / gzip decoders
│   └── network/               # HTTP client (ureq) adapter
├── application/
│   └── cesium-app/            # Bevy App — main binary entry point (dynamic_globe = primary render path)
├── docs/                      # ARCHITECTURE.md, MAPPING.md, SCOPE_2D3D_IMAGERY_PLOT.md, …
└── specs/                     # Integration tests ported from CesiumJS Specs
```

## Verification

The domain layer maintains **bit-exact fidelity** with CesiumJS. Integration tests in `specs/` are ported directly from the CesiumJS Specs test suite and verify identical numerical output at every step.

Key design decisions enabling bit-exact fidelity:
- **f64 precision** throughout the domain layer (f32 only at GPU boundary)
- **Standard IEEE-754 rounding** (no FMA/fast-math)
- **Matching algorithm implementations** (same math, same order of operations)

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Hexagonal architecture | Test domain logic without GPU; swap rendering backends |
| Bevy ECS for rendering | Natural fit for thousands of tile entities |
| f64 domain / f32 GPU | Sub-mm precision at Earth scale vs GPU hardware limits |
| Hybrid ECS + traits | ECS for tile rendering, traits for IO decoupling |
| Bit-exact CesiumJS fidelity | Domain tests ported from CesiumJS Specs |
