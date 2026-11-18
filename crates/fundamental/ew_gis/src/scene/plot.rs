//! Batched vector plotting: point markers, polylines, polygon fills, circles
//! and sectors, authored in geographic coordinates and drawn both on the 2D
//! plane and on the 3D globe.
//!
//! One geographic description ([`DemoShapes`]) is projected into the render
//! space of the active mode:
//!
//! * `Plane` — equirectangular plane metres relative to a fixed demo anchor,
//!   with `z = 0` for every vertex (the tile plane is exactly coplanar).
//! * `Globe` — absolute WGS84 ECEF on the same ellipsoid the globe mesh uses,
//!   lifted a few hundred metres so the shell never wins the depth test.
//!
//! The overlay is exactly three GPU batches — fill triangles, instanced line
//! segments, instanced point quads — so it costs three draw calls. The geometry
//! is rebuilt only when the scene mode (or the reference ellipsoid) changes or
//! the document is edited (selection, drag, delete, undo); camera motion only
//! rewrites the shared plot uniform, the same policy the globe mesh follows.
//!
//! ## Curvature and the 3D lift
//!
//! A flat triangle cannot hug an ellipsoid. Fills are therefore tessellated in
//! *geographic* space with [`FILL_RADIAL_STEPS`] concentric rings, and long line
//! segments are split every [`MAX_CHORD_M`] metres, so the deepest chord sag for
//! the demo shapes (~250 km radius) stays around 100 m. The three layers are
//! lifted by [`LIFT_FILL_M`] / [`LIFT_LINE_M`] / [`LIFT_POINT_M`] on the globe;
//! the tradeoff is up to that much parallax when orbiting at the closest allowed
//! altitude, where shapes of this size are off screen anyway. A denser,
//! screen-space-adaptive tessellation is deliberately deferred.
//!
//! ## Document, selection and editing
//!
//! [`PlotDocument`] owns the authoring data ([`DemoShapes`]) plus a stable
//! [`ShapeId`] per shape, the current selection and an undo stack. Every edit
//! (drag, delete) is a whole-document snapshot at demo scale, which is cheap
//! and cannot desynchronise the parallel id tables. The render geometry is
//! rebuilt from the document (3 draw calls, ~0.3-0.5 ms for the demo); picking
//! and drag conversion live in [`crate::scene::pick`] and never touch the GPU.

use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::gpu::GpuContext;
use crate::math::camera::{GlobeView, PlaneCamera};
use crate::math::geo::{Ellipsoid, lonlat_to_plane, plane_to_lonlat};
use crate::scene::SceneMode;
use crate::scene::icons::demo_altitude;

/// Geographic centre of the demo group. It sits between the two default views
/// (2D centre 120E 30N, 3D sub-camera 110E 25N) so the shapes are visible in
/// both without moving the camera.
pub const DEMO_CENTER_LON: f64 = 115.0;
pub const DEMO_CENTER_LAT: f64 = 27.5;
/// Number of demo point markers (a 20 x 10 jittered grid).
pub const DEMO_POINT_COUNT: usize = 200;
pub const DEMO_LINE_WIDTH_PX: f32 = 3.0;
pub const DEMO_CIRCLE_RADIUS_M: f64 = 200_000.0;
pub const DEMO_SECTOR_RADIUS_M: f64 = 250_000.0;
pub const DEMO_SECTOR_START_DEG: f64 = 30.0;
pub const DEMO_SECTOR_END_DEG: f64 = 120.0;

/// Full-circle fill tessellation (angular samples).
const CIRCLE_SEGMENTS: usize = 72;
/// Concentric rings per fill fan; keeps the chord sag of a 250 km shape near
/// 100 m instead of ~800 m for a single fan.
const FILL_RADIAL_STEPS: usize = 3;
/// Longest line chord on the globe before it is subdivided.
const MAX_CHORD_M: f64 = 40_000.0;

/// Metric lift of the three plot layers above the ellipsoid surface. Larger
/// than the deepest chord sag, far smaller than the icon overlay's altitude.
const LIFT_FILL_M: f64 = 250.0;
const LIFT_LINE_M: f64 = 320.0;
const LIFT_POINT_M: f64 = 420.0;

/// 2D clip-space depth bias bounds, see [`plane_depth_bias`].
const PLANE_BIAS_MIN: f32 = 2.5e-7;
const PLANE_BIAS_MAX: f32 = 5.0e-7;
const PLANE_BIAS_STEP: f32 = 3.0e-7;
/// 3D clip-space depth bias: the metric lift already separates the layers from
/// the shell, this only absorbs projection rounding.
const GLOBE_BIAS: [f32; 3] = [3.0e-7, 6.0e-7, 9.0e-7];

/// One instanced point marker. `center.w` is the marker diameter in pixels,
/// which keeps it constant on screen at any zoom.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PlotPointInstance {
    pub center: [f32; 4],
    pub color: [f32; 4],
}

/// One instanced screen-space line segment. `a.w` is the half width in pixels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PlotLineInstance {
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub color: [f32; 4],
}

/// One fill vertex: world position plus a per-vertex colour.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PlotVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// Reporting counters for one built frame of plot geometry.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlotStats {
    pub points: usize,
    pub segments: usize,
    pub fill_triangles: usize,
    pub fill_vertices: usize,
    /// Number of completed geometry rebuilds since startup.
    pub rebuilds: u64,
    /// CPU time of the most recent rebuild (tessellation + buffer upload).
    pub rebuild_ms: f64,
    /// The tessellation part of `rebuild_ms` (no buffer allocation/upload).
    pub tessellate_ms: f64,
}

/// Per-frame inputs the three plot pipelines share.
pub struct PlotCamera {
    pub view_proj: Mat4,
    pub eye: [f32; 3],
    /// Viewport size in physical pixels.
    pub screen: [f32; 2],
    /// Clip-space depth bias (fraction of `w`) for fill / line / point.
    pub depth_bias: [f32; 3],
}

/// A point marker authored in geographic coordinates.
#[derive(Clone, Copy, Debug)]
pub struct GeoPoint {
    pub lon: f64,
    pub lat: f64,
    /// Marker diameter in pixels.
    pub size_px: f32,
    pub color: [f32; 4],
}

/// An open polyline authored in geographic coordinates.
#[derive(Clone, Debug)]
pub struct GeoPolyline {
    pub points: Vec<[f64; 2]>,
    pub width_px: f32,
    pub color: [f32; 4],
}

/// A filled polygon with an outline, authored as a geographic ring.
#[derive(Clone, Debug)]
pub struct GeoPolygon {
    pub ring: Vec<[f64; 2]>,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub width_px: f32,
}

/// A circle or sector: a geographic centre, a metric radius and an angular
/// sweep in degrees. `start = 0, end = 360` is a full disc.
#[derive(Clone, Copy, Debug)]
pub struct GeoSector {
    pub lon: f64,
    pub lat: f64,
    pub radius_m: f64,
    pub start_deg: f64,
    pub end_deg: f64,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub width_px: f32,
}

impl GeoSector {
    pub fn is_full_circle(&self) -> bool {
        (self.end_deg - self.start_deg).abs() >= 359.999
    }
}

/// The demo plot group. Kept separate from the GPU build so a future tactical
/// overlay can hand in its own shapes: `PlotGeometry::from_shapes` is the only
/// entry point.
#[derive(Clone, Debug)]
pub struct DemoShapes {
    pub points: Vec<GeoPoint>,
    pub polylines: Vec<GeoPolyline>,
    pub polygons: Vec<GeoPolygon>,
    pub sectors: Vec<GeoSector>,
}

impl DemoShapes {
    pub fn demo() -> Self {
        Self {
            points: demo_points(),
            polylines: vec![demo_polyline()],
            polygons: vec![demo_polygon()],
            sectors: vec![
                // The 200 km circle: a full sweep is the disc case.
                GeoSector {
                    lon: DEMO_CENTER_LON + 4.0,
                    lat: DEMO_CENTER_LAT + 2.7,
                    radius_m: DEMO_CIRCLE_RADIUS_M,
                    start_deg: 0.0,
                    end_deg: 360.0,
                    fill: [0.20, 0.95, 0.75, 0.22],
                    stroke: [0.38, 1.00, 0.86, 0.95],
                    width_px: 2.0,
                },
                // The 30..120 degree sector.
                GeoSector {
                    lon: DEMO_CENTER_LON + 2.2,
                    lat: DEMO_CENTER_LAT - 3.8,
                    radius_m: DEMO_SECTOR_RADIUS_M,
                    start_deg: DEMO_SECTOR_START_DEG,
                    end_deg: DEMO_SECTOR_END_DEG,
                    fill: [0.62, 0.55, 1.00, 0.26],
                    stroke: [0.82, 0.76, 1.00, 0.95],
                    width_px: 2.0,
                },
            ],
        }
    }

    /// No shapes at all. Tests build a document from this and push exactly the
    /// shapes they need.
    pub fn empty() -> Self {
        Self {
            points: Vec::new(),
            polylines: Vec::new(),
            polygons: Vec::new(),
            sectors: Vec::new(),
        }
    }

    /// Shape count per kind, in [`ShapeKind::ALL`] order.
    pub fn counts(&self) -> [usize; ShapeKind::COUNT] {
        [
            self.points.len(),
            self.polylines.len(),
            self.polygons.len(),
            self.sectors.len(),
        ]
    }
}

impl Default for DemoShapes {
    fn default() -> Self {
        Self::empty()
    }
}

/// Stable identity of one plotted shape.
///
/// Ids are allocated once when a document is created and never reused, so a
/// selection or a drag target can never silently move to a different shape when
/// a sibling is deleted or an edit is undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShapeId(u64);

impl ShapeId {
    /// Raw id, for logging and scripted selection.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// The four vector layers, in picking priority order (points, then lines, then
/// areas).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeKind {
    Point,
    Polyline,
    Polygon,
    Sector,
}

impl ShapeKind {
    pub const COUNT: usize = 4;
    /// Priority order: earlier kinds win a tie in picking.
    pub const ALL: [ShapeKind; Self::COUNT] = [
        ShapeKind::Point,
        ShapeKind::Polyline,
        ShapeKind::Polygon,
        ShapeKind::Sector,
    ];

    /// Index into the document's per-kind tables (also the picking priority).
    pub fn slot(self) -> usize {
        match self {
            ShapeKind::Point => 0,
            ShapeKind::Polyline => 1,
            ShapeKind::Polygon => 2,
            ShapeKind::Sector => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Point => "point",
            ShapeKind::Polyline => "polyline",
            ShapeKind::Polygon => "polygon",
            ShapeKind::Sector => "sector",
        }
    }
}

/// Position of a shape inside the authoring vectors of [`DemoShapes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeRef {
    pub kind: ShapeKind,
    pub index: usize,
}

impl ShapeRef {
    pub fn new(kind: ShapeKind, index: usize) -> Self {
        Self { kind, index }
    }
}

/// One shape detached from the document: the payload of a drag or a scripted
/// edit.
#[derive(Clone, Debug)]
pub enum ShapeData {
    Point(GeoPoint),
    Polyline(GeoPolyline),
    Polygon(GeoPolygon),
    Sector(GeoSector),
}

impl ShapeData {
    pub fn kind(&self) -> ShapeKind {
        match self {
            ShapeData::Point(_) => ShapeKind::Point,
            ShapeData::Polyline(_) => ShapeKind::Polyline,
            ShapeData::Polygon(_) => ShapeKind::Polygon,
            ShapeData::Sector(_) => ShapeKind::Sector,
        }
    }

    /// Pan every coordinate of the shape by a geographic delta.
    ///
    /// Points and sector centres move their anchor; polylines and polygons move
    /// every vertex. The longitude is wrapped into `[-180, 180)` at the end only,
    /// so a small drag never changes a coordinate by a whole turn.
    pub fn translate(&mut self, dlon: f64, dlat: f64) {
        match self {
            ShapeData::Point(point) => {
                point.lon = normalize_lon(point.lon + dlon);
                point.lat += dlat;
            }
            ShapeData::Polyline(polyline) => translate_path(&mut polyline.points, dlon, dlat),
            ShapeData::Polygon(polygon) => translate_path(&mut polygon.ring, dlon, dlat),
            ShapeData::Sector(sector) => {
                sector.lon = normalize_lon(sector.lon + dlon);
                sector.lat += dlat;
            }
        }
    }
}

/// Selection highlight colours. The document's own colours are kept in the
/// geometry; a selected shape is re-styled to these instead.
pub const SELECT_STROKE: [f32; 4] = [1.00, 0.88, 0.36, 1.00];
pub const SELECT_FILL: [f32; 4] = [1.00, 0.80, 0.28, 0.42];
pub const SELECT_POINT: [f32; 4] = [1.00, 0.95, 0.66, 1.00];
/// Wide, translucent pass under the crisp highlight (lines and area outlines).
pub const SELECT_GLOW: [f32; 4] = [1.00, 0.82, 0.30, 0.30];
/// Halo ring diameter multiplier for a selected point marker.
const SELECT_POINT_HALO_SCALE: f32 = 2.6;
const SELECT_POINT_SCALE: f32 = 1.35;
const SELECT_WIDTH_BONUS_PX: f32 = 2.5;
const SELECT_GLOW_BONUS_PX: f32 = 6.0;

/// Undo depth. Every entry is one whole-document clone; the demo document is a
/// few hundred kilobytes at most, and the stack is bounded so a long editing
/// session cannot grow without limit.
pub const UNDO_DEPTH: usize = 64;

/// A whole-document snapshot: the undo unit and the drag origin.
#[derive(Clone, Debug)]
pub struct PlotSnapshot {
    shapes: DemoShapes,
    ids: [Vec<ShapeId>; ShapeKind::COUNT],
    selected: Option<ShapeId>,
}

/// In-progress drag: the shape as it was when the button went down, plus the
/// geographic anchor under the cursor at that moment.
#[derive(Clone, Debug)]
struct ShapeDrag {
    id: ShapeId,
    at: ShapeRef,
    origin: ShapeData,
    anchor: [f64; 2],
    delta: [f64; 2],
    moved: bool,
}

/// The editable plot document: authoring shapes, stable ids, selection, undo.
///
/// All edits take effect immediately (the scene rebuilds the next frame); the
/// undo stack only records *committed* edits, so an in-progress drag is not
/// undone mid-gesture and a click that never moved is not an undo step.
#[derive(Clone, Debug)]
pub struct PlotDocument {
    shapes: DemoShapes,
    ids: [Vec<ShapeId>; ShapeKind::COUNT],
    selected: Option<ShapeId>,
    undo: Vec<PlotSnapshot>,
    drag: Option<ShapeDrag>,
    /// Pre-drag snapshot, pushed onto the undo stack only if the drag moved.
    pending_undo: Option<PlotSnapshot>,
    /// Number of committed edits (drags + deletes), for logging and tests.
    edits: u64,
}

impl PlotDocument {
    /// Wrap authoring shapes, assigning one id per shape in [`ShapeKind::ALL`]
    /// order (all points first, then polylines, polygons, sectors). Ids are never
    /// reused, so they stay stable for the life of the document.
    pub fn new(shapes: DemoShapes) -> Self {
        let mut next_id = 0u64;
        let mut ids: [Vec<ShapeId>; ShapeKind::COUNT] = std::array::from_fn(|_| Vec::new());
        for kind in ShapeKind::ALL {
            let count = shapes.counts()[kind.slot()];
            let mut table = Vec::with_capacity(count);
            for _ in 0..count {
                table.push(ShapeId(next_id));
                next_id += 1;
            }
            ids[kind.slot()] = table;
        }
        Self {
            shapes,
            ids,
            selected: None,
            undo: Vec::new(),
            drag: None,
            pending_undo: None,
            edits: 0,
        }
    }

    pub fn demo() -> Self {
        Self::new(DemoShapes::demo())
    }

    pub fn shapes(&self) -> &DemoShapes {
        &self.shapes
    }

    pub fn len(&self) -> usize {
        self.shapes.counts().iter().sum()
    }

    pub fn edits(&self) -> u64 {
        self.edits
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn selected(&self) -> Option<ShapeId> {
        self.selected
    }

    /// Number of shapes per kind, in [`ShapeKind::ALL`] order.
    pub fn counts(&self) -> [usize; ShapeKind::COUNT] {
        self.shapes.counts()
    }

    /// Id of the shape at `index` in a kind's authoring vector.
    pub fn id_at(&self, at: ShapeRef) -> Option<ShapeId> {
        self.ids
            .get(at.kind.slot())
            .and_then(|table| table.get(at.index).copied())
    }

    /// Where a shape id currently lives.
    pub fn locate(&self, id: ShapeId) -> Option<ShapeRef> {
        for kind in ShapeKind::ALL {
            let table = &self.ids[kind.slot()];
            if let Ok(index) = table.binary_search(&id) {
                return Some(ShapeRef::new(kind, index));
            }
        }
        None
    }

    pub fn selected_ref(&self) -> Option<ShapeRef> {
        self.selected.and_then(|id| self.locate(id))
    }

    /// Flattened position of a shape (points, polylines, polygons, sectors), the
    /// index space `--select-plot` speaks.
    pub fn ordinal_of(&self, id: ShapeId) -> Option<usize> {
        let at = self.locate(id)?;
        let mut ordinal = 0;
        for kind in ShapeKind::ALL {
            if kind == at.kind {
                return Some(ordinal + at.index);
            }
            ordinal += self.ids[kind.slot()].len();
        }
        None
    }

    /// Shape id at a flattened position, see [`Self::ordinal_of`].
    pub fn id_by_ordinal(&self, ordinal: usize) -> Option<ShapeId> {
        let mut remaining = ordinal;
        for kind in ShapeKind::ALL {
            let table = &self.ids[kind.slot()];
            if remaining < table.len() {
                return Some(table[remaining]);
            }
            remaining -= table.len();
        }
        None
    }

    /// Clone one shape out of the document.
    pub fn data(&self, id: ShapeId) -> Option<ShapeData> {
        let at = self.locate(id)?;
        Some(match at.kind {
            ShapeKind::Point => ShapeData::Point(self.shapes.points[at.index]),
            ShapeKind::Polyline => ShapeData::Polyline(self.shapes.polylines[at.index].clone()),
            ShapeKind::Polygon => ShapeData::Polygon(self.shapes.polygons[at.index].clone()),
            ShapeKind::Sector => ShapeData::Sector(self.shapes.sectors[at.index]),
        })
    }

    /// Geographic anchor of a shape: the marker position, the sector centre, or
    /// the average of a path's vertices.
    pub fn shape_center(&self, id: ShapeId) -> Option<[f64; 2]> {
        let at = self.locate(id)?;
        Some(match at.kind {
            ShapeKind::Point => {
                let point = &self.shapes.points[at.index];
                [point.lon, point.lat]
            }
            ShapeKind::Polyline => ring_center(&self.shapes.polylines[at.index].points),
            ShapeKind::Polygon => ring_center(&self.shapes.polygons[at.index].ring),
            ShapeKind::Sector => {
                let sector = &self.shapes.sectors[at.index];
                [sector.lon, sector.lat]
            }
        })
    }

    /// Select a shape, or clear the selection with `None`. Returns true when the
    /// selection changed. Selecting an unknown id clears the selection.
    pub fn select(&mut self, id: Option<ShapeId>) -> bool {
        let id = match id {
            Some(id) if self.locate(id).is_some() => Some(id),
            _ => None,
        };
        if self.selected == id {
            return false;
        }
        self.selected = id;
        true
    }

    pub fn snapshot(&self) -> PlotSnapshot {
        PlotSnapshot {
            shapes: self.shapes.clone(),
            ids: self.ids.clone(),
            selected: self.selected,
        }
    }

    pub fn restore(&mut self, snapshot: &PlotSnapshot) {
        self.shapes = snapshot.shapes.clone();
        self.ids = snapshot.ids.clone();
        self.selected = snapshot.selected;
    }

    /// Delete the selected shape. Returns the removed id (for logging) and clears
    /// the selection; pushes one undo step.
    pub fn delete_selected(&mut self) -> Option<ShapeId> {
        let id = self.selected?;
        let at = self.locate(id)?;
        let snapshot = self.snapshot();
        match at.kind {
            ShapeKind::Point => {
                self.shapes.points.remove(at.index);
            }
            ShapeKind::Polyline => {
                self.shapes.polylines.remove(at.index);
            }
            ShapeKind::Polygon => {
                self.shapes.polygons.remove(at.index);
            }
            ShapeKind::Sector => {
                self.shapes.sectors.remove(at.index);
            }
        }
        self.ids[at.kind.slot()].remove(at.index);
        self.selected = None;
        self.push_undo(snapshot);
        self.edits += 1;
        Some(id)
    }

    /// Start dragging `id` from the geographic point under the cursor.
    pub fn begin_drag(&mut self, id: ShapeId, anchor: [f64; 2]) -> bool {
        let Some(at) = self.locate(id) else {
            return false;
        };
        let Some(origin) = self.data(id) else {
            return false;
        };
        self.pending_undo = Some(self.snapshot());
        self.drag = Some(ShapeDrag {
            id,
            at,
            origin,
            anchor,
            delta: [0.0; 2],
            moved: false,
        });
        true
    }

    /// Geographic anchor of the active drag (the point under the press).
    pub fn drag_anchor(&self) -> Option<[f64; 2]> {
        self.drag.as_ref().map(|drag| drag.anchor)
    }

    /// Move the dragged shape to `origin + delta`. Deltas are geographic degrees
    /// relative to the press position, so re-applying a delta is idempotent and
    /// the shape cannot drift over a long gesture.
    pub fn drag_to(&mut self, delta: [f64; 2]) -> bool {
        if self.drag.as_ref().is_none_or(|drag| drag.delta == delta) {
            // No grab, or the pointer did not actually move: no rebuild.
            return false;
        }
        let Some(mut drag) = self.drag.take() else {
            return false;
        };
        let mut changed = false;
        if delta[0].is_finite()
            && delta[1].is_finite()
            && let Some(at) = self.locate(drag.id)
        {
            let mut data = drag.origin.clone();
            data.translate(delta[0], delta[1]);
            drag.at = at;
            drag.delta = delta;
            if delta[0] != 0.0 || delta[1] != 0.0 {
                drag.moved = true;
            }
            self.write(at, data);
            changed = true;
        }
        self.drag = Some(drag);
        changed
    }

    /// Finish the drag. Returns the shape id when the gesture actually moved the
    /// shape (i.e. when one undo step was committed).
    pub fn end_drag(&mut self) -> Option<ShapeId> {
        let drag = self.drag.take()?;
        let snapshot = self.pending_undo.take();
        if !drag.moved {
            return None;
        }
        if let Some(snapshot) = snapshot {
            self.push_undo(snapshot);
        }
        self.edits += 1;
        Some(drag.id)
    }

    /// Undo the most recent committed edit. Returns false when the stack is
    /// empty (the selection is left alone in that case).
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.drag = None;
        self.pending_undo = None;
        self.restore(&snapshot);
        self.edits += 1;
        true
    }

    /// Translate a shape in one shot as a committed edit (scripted edits and
    /// tests; the interactive path uses [`Self::begin_drag`] + [`Self::drag_to`]).
    pub fn translate_shape(&mut self, id: ShapeId, delta: [f64; 2]) -> bool {
        if !self.begin_drag(id, [0.0, 0.0]) {
            return false;
        }
        self.drag_to(delta);
        self.end_drag().is_some()
    }

    fn write(&mut self, at: ShapeRef, data: ShapeData) {
        if data.kind() != at.kind {
            log::warn!(
                "plot document: refusing to write a {} into the {} slot",
                data.kind().label(),
                at.kind.label()
            );
            return;
        }
        match data {
            ShapeData::Point(point) => self.shapes.points[at.index] = point,
            ShapeData::Polyline(polyline) => self.shapes.polylines[at.index] = polyline,
            ShapeData::Polygon(polygon) => self.shapes.polygons[at.index] = polygon,
            ShapeData::Sector(sector) => self.shapes.sectors[at.index] = sector,
        }
    }

    fn push_undo(&mut self, snapshot: PlotSnapshot) {
        if self.undo.len() == UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
    }
}

impl Default for PlotDocument {
    fn default() -> Self {
        Self::demo()
    }
}

fn translate_path(points: &mut [[f64; 2]], dlon: f64, dlat: f64) {
    for point in points {
        point[0] = normalize_lon(point[0] + dlon);
        point[1] += dlat;
    }
}

/// Wrap a longitude into `[-180, 180)`.
fn normalize_lon(lon: f64) -> f64 {
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    // `rem_euclid` can return exactly 360.0 for a tiny negative input.
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    }
}

/// CPU-side tessellation result: three batches, ready for one upload each.
#[derive(Clone, Debug, Default)]
pub struct PlotGeometry {
    pub fill: Vec<PlotVertex>,
    pub lines: Vec<PlotLineInstance>,
    pub points: Vec<PlotPointInstance>,
}

impl PlotGeometry {
    /// The demo batches, without a document. Kept as the shape-only entry point
    /// (acceptance tooling and the tests use it); [`Self::from_document`] is what
    /// the live scene rebuilds through.
    #[allow(dead_code)]
    pub fn demo(mode: SceneMode, ellipsoid: Ellipsoid) -> Self {
        Self::from_shapes(&DemoShapes::demo(), mode, ellipsoid)
    }

    /// Tessellate authoring shapes with no selection highlight.
    #[allow(dead_code)]
    pub fn from_shapes(shapes: &DemoShapes, mode: SceneMode, ellipsoid: Ellipsoid) -> Self {
        Self::from_shapes_selected(shapes, None, mode, ellipsoid)
    }

    /// Tessellate the document, re-styling the selected shape.
    pub fn from_document(document: &PlotDocument, mode: SceneMode, ellipsoid: Ellipsoid) -> Self {
        Self::from_shapes_selected(document.shapes(), document.selected_ref(), mode, ellipsoid)
    }

    /// Tessellate `shapes` into the three batches, with `selected` highlighted.
    ///
    /// Highlighting never adds a draw call: a selected area re-colours its fill
    /// and outline, a selected line gains a wider translucent pass plus a crisp
    /// stroke, and a selected point marker gains a halo instance. All of it stays
    /// inside the same three instance/vertex buffers.
    pub fn from_shapes_selected(
        shapes: &DemoShapes,
        selected: Option<ShapeRef>,
        mode: SceneMode,
        ellipsoid: Ellipsoid,
    ) -> Self {
        let projector = Projector::new(mode, ellipsoid);
        let mut geometry = Self::default();
        let is_selected =
            |kind: ShapeKind, index: usize| selected == Some(ShapeRef::new(kind, index));

        for (index, point) in shapes.points.iter().enumerate() {
            let position = projector.project(point.lon, point.lat, 0.0, 0.0, LIFT_POINT_M);
            if is_selected(ShapeKind::Point, index) {
                // Halo first: the point pipeline blends in instance order, so the
                // bright marker lands on top of its own glow.
                geometry.points.push(PlotPointInstance {
                    center: [
                        position[0],
                        position[1],
                        position[2],
                        point.size_px * SELECT_POINT_HALO_SCALE,
                    ],
                    color: SELECT_GLOW,
                });
                geometry.points.push(PlotPointInstance {
                    center: [
                        position[0],
                        position[1],
                        position[2],
                        point.size_px * SELECT_POINT_SCALE,
                    ],
                    color: SELECT_POINT,
                });
            } else {
                geometry.points.push(PlotPointInstance {
                    center: [position[0], position[1], position[2], point.size_px],
                    color: point.color,
                });
            }
        }

        for (index, polyline) in shapes.polylines.iter().enumerate() {
            let center = ring_center(&polyline.points);
            let local = local_path(&projector, &polyline.points, center);
            if is_selected(ShapeKind::Polyline, index) {
                geometry.push_segments(
                    &projector,
                    center,
                    &local,
                    polyline.width_px + SELECT_GLOW_BONUS_PX,
                    SELECT_GLOW,
                );
                geometry.push_segments(
                    &projector,
                    center,
                    &local,
                    polyline.width_px + SELECT_WIDTH_BONUS_PX,
                    SELECT_STROKE,
                );
            } else {
                geometry.push_segments(
                    &projector,
                    center,
                    &local,
                    polyline.width_px,
                    polyline.color,
                );
            }
        }

        for (index, polygon) in shapes.polygons.iter().enumerate() {
            let center = ring_center(&polygon.ring);
            let local = local_path(&projector, &polygon.ring, center);
            let highlighted = is_selected(ShapeKind::Polygon, index);
            let fill = if highlighted {
                SELECT_FILL
            } else {
                polygon.fill
            };
            geometry.push_fill(&projector, center, &local, true, fill);
            // Closed outline, drawn one layer above the fill.
            let mut outline = local.clone();
            outline.push(local[0]);
            if highlighted {
                geometry.push_segments(
                    &projector,
                    center,
                    &outline,
                    polygon.width_px + SELECT_GLOW_BONUS_PX,
                    SELECT_GLOW,
                );
                geometry.push_segments(
                    &projector,
                    center,
                    &outline,
                    polygon.width_px + SELECT_WIDTH_BONUS_PX,
                    SELECT_STROKE,
                );
            } else {
                geometry.push_segments(
                    &projector,
                    center,
                    &outline,
                    polygon.width_px,
                    polygon.stroke,
                );
            }
        }

        for (index, sector) in shapes.sectors.iter().enumerate() {
            let full = sector.is_full_circle();
            let ring = sector_ring(sector);
            let center = [sector.lon, sector.lat];
            let highlighted = is_selected(ShapeKind::Sector, index);
            let fill = if highlighted {
                SELECT_FILL
            } else {
                sector.fill
            };
            geometry.push_fill(&projector, center, &ring, full, fill);

            // Outline: closed ring for a disc, arc plus both radii for a sector.
            let mut outline: Vec<[f64; 2]> = Vec::with_capacity(ring.len() + 2);
            if !full {
                outline.push([0.0, 0.0]);
            }
            outline.extend_from_slice(&ring);
            if full {
                outline.push(ring[0]);
            } else {
                outline.push([0.0, 0.0]);
            }
            if highlighted {
                geometry.push_segments(
                    &projector,
                    center,
                    &outline,
                    sector.width_px + SELECT_GLOW_BONUS_PX,
                    SELECT_GLOW,
                );
                geometry.push_segments(
                    &projector,
                    center,
                    &outline,
                    sector.width_px + SELECT_WIDTH_BONUS_PX,
                    SELECT_STROKE,
                );
            } else {
                geometry.push_segments(
                    &projector,
                    center,
                    &outline,
                    sector.width_px,
                    sector.stroke,
                );
            }
        }

        geometry
    }

    /// One path as instanced screen-space segments (no closing edge).
    fn push_segments(
        &mut self,
        projector: &Projector,
        center: [f64; 2],
        path: &[[f64; 2]],
        width_px: f32,
        color: [f32; 4],
    ) {
        for pair in path.windows(2) {
            self.push_segment(projector, center, pair[0], pair[1], width_px, color);
        }
    }

    /// A fill fan in shape-local metres, tessellated into concentric rings so
    /// the chord sag stays small on the globe. `full` closes the ring (disc)
    /// instead of leaving the two radial edges of a sector.
    fn push_fill(
        &mut self,
        projector: &Projector,
        center: [f64; 2],
        ring: &[[f64; 2]],
        full: bool,
        color: [f32; 4],
    ) {
        let count = ring.len();
        if count < 3 {
            return;
        }
        let steps = FILL_RADIAL_STEPS.max(1);
        let mut rings: Vec<Vec<[f32; 3]>> = Vec::with_capacity(steps + 1);
        for step in 0..=steps {
            let t = step as f64 / steps as f64;
            rings.push(
                ring.iter()
                    .map(|offset| {
                        projector.project(
                            center[0],
                            center[1],
                            offset[0] * t,
                            offset[1] * t,
                            LIFT_FILL_M,
                        )
                    })
                    .collect(),
            );
        }

        let shared = rings[0][0];
        let span = if full { count } else { count - 1 };
        for index in 0..span {
            let next = (index + 1) % count;
            self.push_triangle(shared, rings[1][index], rings[1][next], color);
            for step in 1..steps {
                let inner = &rings[step];
                let outer = &rings[step + 1];
                self.push_triangle(inner[index], outer[index], outer[next], color);
                self.push_triangle(inner[index], outer[next], inner[next], color);
            }
        }
    }

    /// One screen-space segment. On the globe a long chord is split so the line
    /// does not sink through the ellipsoid between its endpoints.
    fn push_segment(
        &mut self,
        projector: &Projector,
        center: [f64; 2],
        from: [f64; 2],
        to: [f64; 2],
        width_px: f32,
        color: [f32; 4],
    ) {
        let length = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
        let steps = if projector.mode == SceneMode::Globe {
            ((length / MAX_CHORD_M).ceil() as usize).max(1)
        } else {
            1
        };
        let half_width = width_px * 0.5;
        let mut previous = projector.project(center[0], center[1], from[0], from[1], LIFT_LINE_M);
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let east = from[0] + (to[0] - from[0]) * t;
            let north = from[1] + (to[1] - from[1]) * t;
            let current = projector.project(center[0], center[1], east, north, LIFT_LINE_M);
            self.lines.push(PlotLineInstance {
                a: [previous[0], previous[1], previous[2], half_width],
                b: [current[0], current[1], current[2], 0.0],
                color,
            });
            previous = current;
        }
    }

    fn push_triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], color: [f32; 4]) {
        for position in [a, b, c] {
            self.fill.push(PlotVertex { position, color });
        }
    }
}

/// Maps geographic coordinates into the render space of one mode.
///
/// "Local" coordinates are metres east/north of a shape centre. Converting a
/// geographic ring to local offsets and projecting them back is exact in both
/// modes, which is what lets the fills and their outlines share one ring.
#[derive(Clone, Copy, Debug)]
struct Projector {
    mode: SceneMode,
    /// 2D render origin in plane metres.
    anchor: [f64; 2],
    ellipsoid: Ellipsoid,
}

impl Projector {
    fn new(mode: SceneMode, ellipsoid: Ellipsoid) -> Self {
        Self {
            mode,
            anchor: lonlat_to_plane(DEMO_CENTER_LON, DEMO_CENTER_LAT),
            ellipsoid,
        }
    }

    /// Local east/north metres of a geographic point relative to `center`.
    fn local(&self, lon: f64, lat: f64, center: [f64; 2]) -> [f64; 2] {
        match self.mode {
            SceneMode::Plane => {
                let point = lonlat_to_plane(lon, lat);
                let origin = lonlat_to_plane(center[0], center[1]);
                [point[0] - origin[0], point[1] - origin[1]]
            }
            SceneMode::Globe => {
                let (prime_vertical, meridian) = local_radii(self.ellipsoid, center[1]);
                let lat_rad = center[1].to_radians();
                [
                    (lon - center[0]).to_radians() * prime_vertical * lat_rad.cos(),
                    (lat - center[1]).to_radians() * meridian,
                ]
            }
        }
    }

    /// Render-space position of a local east/north offset from a centre.
    fn project(&self, lon: f64, lat: f64, east_m: f64, north_m: f64, lift_m: f64) -> [f32; 3] {
        match self.mode {
            SceneMode::Plane => {
                let origin = lonlat_to_plane(lon, lat);
                [
                    (origin[0] - self.anchor[0] + east_m) as f32,
                    (origin[1] - self.anchor[1] + north_m) as f32,
                    0.0,
                ]
            }
            SceneMode::Globe => {
                let (target_lon, target_lat) =
                    geodetic_offset(self.ellipsoid, lon, lat, east_m, north_m);
                let point = self.ellipsoid.point(target_lon, target_lat, lift_m);
                [point[0] as f32, point[1] as f32, point[2] as f32]
            }
        }
    }
}

/// The ellipsoid's two local curvature radii at a latitude: (prime vertical,
/// meridian). They turn metric east/north offsets into lon/lat degrees.
fn local_radii(ellipsoid: Ellipsoid, lat_deg: f64) -> (f64, f64) {
    let e2 = ellipsoid.e2();
    let sin = lat_deg.to_radians().sin();
    let w = (1.0 - e2 * sin * sin).sqrt();
    (ellipsoid.a / w, ellipsoid.a * (1.0 - e2) / (w * w * w))
}

/// Local east/north metres of a geographic offset from `center` on the 2D plane.
///
/// The plane is `x = R·lon_rad, y = R·lat_rad`, so metric offsets are plain
/// degree offsets; this is the geographic twin of [`Projector::project`]'s plane
/// branch, used by CPU picking to rebuild a sector outline in lon/lat.
pub(crate) fn plane_offset(center: [f64; 2], east_m: f64, north_m: f64) -> (f64, f64) {
    let origin = lonlat_to_plane(center[0], center[1]);
    plane_to_lonlat(origin[0] + east_m, origin[1] + north_m)
}

pub(crate) fn geodetic_offset(
    ellipsoid: Ellipsoid,
    lon: f64,
    lat: f64,
    east_m: f64,
    north_m: f64,
) -> (f64, f64) {
    let (prime_vertical, meridian) = local_radii(ellipsoid, lat);
    let cos_lat = lat.to_radians().cos().max(1.0e-6);
    (
        lon + (east_m / (prime_vertical * cos_lat)).to_degrees(),
        lat + (north_m / meridian).to_degrees(),
    )
}

/// Sampled outline of a circle/sector in local east/north metres: the arc, left
/// open for a full sweep so the ring can be closed by wrapping. Shared by the
/// fill tessellation and by CPU picking (which rebuilds it in lon/lat).
pub(crate) fn sector_ring(sector: &GeoSector) -> Vec<[f64; 2]> {
    let full = sector.is_full_circle();
    let sweep = (sector.end_deg - sector.start_deg).abs();
    let segments = if full {
        CIRCLE_SEGMENTS
    } else {
        ((sweep / 360.0) * CIRCLE_SEGMENTS as f64).ceil().max(8.0) as usize
    };
    arc_offsets(sector.radius_m, sector.start_deg, sector.end_deg, segments)
}

/// Sampled circular arc in local east/north metres. A full sweep omits the
/// duplicate end point so the ring can be closed by wrapping.
fn arc_offsets(radius_m: f64, start_deg: f64, end_deg: f64, segments: usize) -> Vec<[f64; 2]> {
    let sweep = (end_deg - start_deg).to_radians();
    let full = (sweep.abs() - std::f64::consts::TAU).abs() < 1.0e-6;
    let count = if full { segments } else { segments + 1 };
    let segments = segments.max(1) as f64;
    (0..count)
        .map(|index| {
            let angle = start_deg.to_radians() + sweep * (index as f64 / segments);
            [radius_m * angle.cos(), radius_m * angle.sin()]
        })
        .collect()
}

pub(crate) fn ring_center(ring: &[[f64; 2]]) -> [f64; 2] {
    if ring.is_empty() {
        return [0.0, 0.0];
    }
    let mut lon = 0.0;
    let mut lat = 0.0;
    for point in ring {
        lon += point[0];
        lat += point[1];
    }
    let count = ring.len() as f64;
    [lon / count, lat / count]
}

/// Convert a geographic path into local east/north metres around `center`.
fn local_path(projector: &Projector, points: &[[f64; 2]], center: [f64; 2]) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|point| projector.local(point[0], point[1], center))
        .collect()
}

/// Deterministic value in `[0, 1)`; identical on every platform, unlike
/// `f64::sin`-derived hashes.
fn hash01(seed: u32) -> f64 {
    let mut x = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x ^= x >> 15;
    x = x.wrapping_mul(2_246_822_519);
    x ^= x >> 13;
    (x >> 8) as f64 / 16_777_216.0
}

fn demo_points() -> Vec<GeoPoint> {
    const PALETTE: [[f32; 3]; 6] = [
        [0.35, 0.86, 1.00],
        [1.00, 0.72, 0.25],
        [0.55, 1.00, 0.62],
        [1.00, 0.40, 0.55],
        [0.78, 0.68, 1.00],
        [1.00, 0.95, 0.80],
    ];
    const COLS: usize = 20;
    const ROWS: usize = DEMO_POINT_COUNT / COLS;
    let mut points = Vec::with_capacity(DEMO_POINT_COUNT);
    for index in 0..DEMO_POINT_COUNT {
        let col = (index % COLS) as f64;
        let row = (index / COLS) as f64;
        // Grid cell centre, then a deterministic jitter so the batch reads as
        // scattered plot markers rather than a lattice.
        let cell_lon = (col + 0.5) / COLS as f64 - 0.5;
        let cell_lat = (row + 0.5) / ROWS as f64 - 0.5;
        let jitter_lon = hash01(index as u32 * 2 + 1) - 0.5;
        let jitter_lat = hash01(index as u32 * 2 + 2) - 0.5;
        let lon = DEMO_CENTER_LON + cell_lon * 11.0 + jitter_lon * 0.24;
        let lat = DEMO_CENTER_LAT + cell_lat * 8.4 + jitter_lat * 0.20;
        let size_px = 4.0 + hash01(index as u32 * 7 + 3) * 7.0;
        let color = PALETTE[(hash01(index as u32 * 13 + 5) * 6.0) as usize % PALETTE.len()];
        let alpha = 0.80 + 0.20 * hash01(index as u32 * 17 + 7) as f32;
        points.push(GeoPoint {
            lon,
            lat,
            size_px: size_px as f32,
            color: [color[0], color[1], color[2], alpha],
        });
    }
    points
}

fn demo_polyline() -> GeoPolyline {
    const STEPS: usize = 12;
    let mut points = Vec::with_capacity(STEPS);
    for step in 0..STEPS {
        let t = step as f64 / (STEPS - 1) as f64;
        let lon = DEMO_CENTER_LON - 5.6 + 11.2 * t;
        let lat = DEMO_CENTER_LAT + 2.9 + 2.8 * (t * std::f64::consts::PI).sin() - 4.6 * t;
        points.push([lon, lat]);
    }
    GeoPolyline {
        points,
        width_px: DEMO_LINE_WIDTH_PX,
        color: [0.32, 0.96, 1.00, 0.95],
    }
}

fn demo_polygon() -> GeoPolygon {
    const VERTICES: usize = 24;
    let center = [DEMO_CENTER_LON - 3.8, DEMO_CENTER_LAT - 3.3];
    let mut ring = Vec::with_capacity(VERTICES);
    for index in 0..VERTICES {
        let angle = std::f64::consts::TAU * index as f64 / VERTICES as f64;
        // Irregular ring: a lobed radius plus a mild east/west bias.
        let radius = 2.05 * (1.0 + 0.24 * (3.0 * angle).sin() + 0.10 * (5.0 * angle).cos());
        ring.push([
            center[0] + radius * angle.cos() * 1.08,
            center[1] + radius * angle.sin() * 0.88,
        ]);
    }
    GeoPolygon {
        ring,
        fill: [1.00, 0.62, 0.22, 0.30],
        stroke: [1.00, 0.80, 0.45, 0.95],
        width_px: 2.0,
    }
}

/// Owns the three dynamic batches, the editable document and the rebuild policy.
pub struct PlotScene {
    pub fill_buffer: wgpu::Buffer,
    pub line_buffer: wgpu::Buffer,
    pub point_buffer: wgpu::Buffer,
    pub fill_vertex_count: u32,
    pub line_instance_count: u32,
    pub point_instance_count: u32,
    pub stats: PlotStats,
    /// 2D render origin in plane metres; the plot uniform shifts the plane
    /// camera by `anchor - camera ground point`.
    pub anchor_plane: [f64; 2],
    document: PlotDocument,
    built: Option<(SceneMode, Ellipsoid)>,
    /// Set by every document edit; forces the next [`Self::update`] to rebuild.
    dirty: bool,
}

impl PlotScene {
    pub fn new(gpu: &GpuContext, mode: SceneMode, ellipsoid: Ellipsoid) -> Self {
        Self::with_document(gpu, PlotDocument::demo(), mode, ellipsoid)
    }

    /// Build a scene from an already authored document (tests and future
    /// tactical overlays).
    pub fn with_document(
        gpu: &GpuContext,
        document: PlotDocument,
        mode: SceneMode,
        ellipsoid: Ellipsoid,
    ) -> Self {
        let mut scene = Self {
            fill_buffer: dynamic_vertex_buffer(gpu, "ew_gis plot fill vertices", 0),
            line_buffer: dynamic_vertex_buffer(gpu, "ew_gis plot line instances", 0),
            point_buffer: dynamic_vertex_buffer(gpu, "ew_gis plot point instances", 0),
            fill_vertex_count: 0,
            line_instance_count: 0,
            point_instance_count: 0,
            stats: PlotStats::default(),
            anchor_plane: lonlat_to_plane(DEMO_CENTER_LON, DEMO_CENTER_LAT),
            document,
            built: None,
            dirty: true,
        };
        scene.update(gpu, mode, ellipsoid);
        scene
    }

    /// The editable document behind the batches. Read-only: edits go through the
    /// `select` / `drag` / `delete` / `undo` methods so the rebuild is scheduled.
    pub fn document(&self) -> &PlotDocument {
        &self.document
    }

    /// Replace the document with a freshly authored one (a live layer merged new
    /// shapes). The scene keeps its buffers and its rebuild policy: the next
    /// [`Self::update`] rewrites the three batches. Selection and undo history
    /// belong to the document, so the replacement starts with an empty history.
    pub fn replace_document(&mut self, document: PlotDocument) {
        self.document = document;
        self.dirty = true;
    }

    /// Rebuild the CPU tessellation and the three buffers when the mode (or the
    /// reference ellipsoid) changed, or when the document was edited. Returns
    /// true when buffers were rewritten.
    ///
    /// There is no camera argument on purpose: vertices are anchored (2D) or
    /// absolute ECEF (3D), so panning and orbiting never rebuild geometry. The
    /// camera only contributes the per-frame clip matrix built by
    /// [`Self::camera_plane`] / [`Self::camera_globe`].
    pub fn update(&mut self, gpu: &GpuContext, mode: SceneMode, ellipsoid: Ellipsoid) -> bool {
        if self.built == Some((mode, ellipsoid)) && !self.dirty {
            return false;
        }
        let started = std::time::Instant::now();
        let geometry = PlotGeometry::from_document(&self.document, mode, ellipsoid);
        let tessellate_ms = started.elapsed().as_secs_f64() * 1000.0;

        self.fill_buffer = dynamic_vertex_buffer(
            gpu,
            "ew_gis plot fill vertices",
            geometry.fill.len() * size_of::<PlotVertex>(),
        );
        self.line_buffer = dynamic_vertex_buffer(
            gpu,
            "ew_gis plot line instances",
            geometry.lines.len() * size_of::<PlotLineInstance>(),
        );
        self.point_buffer = dynamic_vertex_buffer(
            gpu,
            "ew_gis plot point instances",
            geometry.points.len() * size_of::<PlotPointInstance>(),
        );

        if !geometry.fill.is_empty() {
            gpu.queue
                .write_buffer(&self.fill_buffer, 0, bytemuck::cast_slice(&geometry.fill));
        }
        if !geometry.lines.is_empty() {
            gpu.queue
                .write_buffer(&self.line_buffer, 0, bytemuck::cast_slice(&geometry.lines));
        }
        if !geometry.points.is_empty() {
            gpu.queue.write_buffer(
                &self.point_buffer,
                0,
                bytemuck::cast_slice(&geometry.points),
            );
        }

        self.fill_vertex_count = geometry.fill.len() as u32;
        self.line_instance_count = geometry.lines.len() as u32;
        self.point_instance_count = geometry.points.len() as u32;
        self.stats = PlotStats {
            points: geometry.points.len(),
            segments: geometry.lines.len(),
            fill_triangles: geometry.fill.len() / 3,
            fill_vertices: geometry.fill.len(),
            rebuilds: self.stats.rebuilds + 1,
            rebuild_ms: started.elapsed().as_secs_f64() * 1000.0,
            tessellate_ms,
        };
        self.built = Some((mode, ellipsoid));
        self.dirty = false;

        log::info!(
            "plot geometry rebuilt for {}: {} fill triangles ({} vertices), {} segments, {} points, {:.3} ms (tessellation {:.3} ms)",
            mode.label(),
            self.stats.fill_triangles,
            self.stats.fill_vertices,
            self.stats.segments,
            self.stats.points,
            self.stats.rebuild_ms,
            self.stats.tessellate_ms
        );
        true
    }

    /// Select a shape (or clear with `None`); schedules a rebuild on change.
    pub fn select(&mut self, id: Option<ShapeId>) -> bool {
        let changed = self.document.select(id);
        self.dirty |= changed;
        changed
    }

    /// Delete the selected shape; schedules a rebuild when something was removed.
    pub fn delete_selected(&mut self) -> Option<ShapeId> {
        let removed = self.document.delete_selected();
        self.dirty |= removed.is_some();
        removed
    }

    /// Undo the last committed edit; schedules a rebuild when the stack was not
    /// empty.
    pub fn undo(&mut self) -> bool {
        let undone = self.document.undo();
        self.dirty |= undone;
        undone
    }

    pub fn begin_drag(&mut self, id: ShapeId, anchor: [f64; 2]) -> bool {
        self.document.begin_drag(id, anchor)
    }

    /// Move the grabbed shape; schedules a rebuild when the shape was written.
    pub fn drag_to(&mut self, delta: [f64; 2]) -> bool {
        let changed = self.document.drag_to(delta);
        self.dirty |= changed;
        changed
    }

    /// Release the grab. Returns the shape id when one edit step was committed.
    pub fn end_drag(&mut self) -> Option<ShapeId> {
        self.document.end_drag()
    }

    /// Translate a shape as one committed edit (the scripted `--move-plot` path).
    pub fn translate_shape(&mut self, id: ShapeId, delta: [f64; 2]) -> bool {
        let changed = self.document.translate_shape(id, delta);
        self.dirty |= changed;
        changed
    }

    /// Resolve the shared plot uniform for the 2D plane: the plane camera's
    /// matrix shifted so the anchor-relative vertices land on the same ground
    /// point the camera looks at.
    pub fn camera_plane(&self, camera: &PlaneCamera, aspect: f64, screen: [f32; 2]) -> PlotCamera {
        let ground = lonlat_to_plane(camera.center_lon, camera.center_lat);
        let shift = Vec3::new(
            (self.anchor_plane[0] - ground[0]) as f32,
            (self.anchor_plane[1] - ground[1]) as f32,
            0.0,
        );
        PlotCamera {
            view_proj: camera.view_proj(aspect) * Mat4::from_translation(shift),
            eye: camera.eye(),
            screen,
            depth_bias: plane_depth_bias(camera.height, camera.near),
        }
    }

    /// Resolve the shared plot uniform for the 3D globe. Vertices are already
    /// absolute ECEF, so the globe's clip matrix is used unchanged.
    pub fn camera_globe(&self, view: &GlobeView, screen: [f32; 2]) -> PlotCamera {
        PlotCamera {
            view_proj: view.view_proj_f32(),
            eye: view.eye_relative,
            screen,
            depth_bias: GLOBE_BIAS,
        }
    }
}

/// Depth bias for the 2D plane, in clip space (a fraction of `w`). The plot is
/// exactly coplanar with the tile quads and one f32 depth step at a 9,000 km
/// eye altitude is ~10 km of ground distance, so the separation has to be
/// synthetic. Two bounds set it:
///
/// * at least a few f32 depth ULP, so tile/plot rounding never decides the test
///   (the two paths evaluate the same matrix product in a different order);
/// * at most half the icon overlay's altitude, because the icon pass runs after
///   the fills and does not write depth, and it must not be punched out.
fn plane_depth_bias(height: f64, near: f32) -> [f32; 3] {
    let height = height.max(1.0);
    let clearance = 0.5 * demo_altitude(height as f32) as f64;
    let cap = clearance * near.max(1.0) as f64 / (height * height);
    let base = (cap as f32).clamp(PLANE_BIAS_MIN, PLANE_BIAS_MAX);
    [base, base + PLANE_BIAS_STEP, base + 2.0 * PLANE_BIAS_STEP]
}

fn dynamic_vertex_buffer(gpu: &GpuContext, label: &str, size: usize) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::geo::EARTH_RADIUS;

    #[test]
    fn demo_shapes_match_the_spec() {
        let shapes = DemoShapes::demo();
        assert_eq!(shapes.points.len(), DEMO_POINT_COUNT);
        assert_eq!(shapes.polylines.len(), 1);
        assert!(shapes.polylines[0].points.len() >= 8);
        assert_eq!(shapes.polylines[0].width_px, DEMO_LINE_WIDTH_PX);
        assert_eq!(shapes.polygons.len(), 1);
        assert!(shapes.polygons[0].ring.len() >= 3);
        assert!(shapes.polygons[0].fill[3] < 1.0, "fill must be translucent");

        let circle = shapes
            .sectors
            .iter()
            .find(|sector| sector.is_full_circle())
            .expect("demo circle");
        assert!((circle.radius_m - 200_000.0).abs() < 1.0);

        let sector = shapes
            .sectors
            .iter()
            .find(|sector| !sector.is_full_circle())
            .expect("demo sector");
        assert_eq!(sector.start_deg, 30.0);
        assert_eq!(sector.end_deg, 120.0);
    }

    #[test]
    fn point_markers_all_come_through() {
        for mode in [SceneMode::Plane, SceneMode::Globe] {
            let geometry = PlotGeometry::demo(mode, Ellipsoid::WGS84);
            assert_eq!(geometry.points.len(), DEMO_POINT_COUNT);
            assert!(geometry.points.iter().all(|point| point.center[3] >= 4.0));
            assert!(geometry.points.iter().all(|point| point.color[3] > 0.0));
        }
    }

    #[test]
    fn plane_geometry_is_coplanar() {
        let geometry = PlotGeometry::demo(SceneMode::Plane, Ellipsoid::WGS84);
        assert!(!geometry.fill.is_empty());
        assert!(!geometry.lines.is_empty());
        assert_eq!(geometry.fill.len() % 3, 0);
        assert!(geometry.fill.iter().all(|vertex| vertex.position[2] == 0.0));
        assert!(geometry.points.iter().all(|point| point.center[2] == 0.0));
        // The plane geometry is anchored at the demo centre, so nothing is more
        // than a few degrees away from the origin.
        let limit = EARTH_RADIUS * 0.25;
        assert!(
            geometry
                .points
                .iter()
                .all(|point| point.center[0].abs() < limit as f32
                    && point.center[1].abs() < limit as f32)
        );
    }

    #[test]
    fn globe_geometry_hugs_the_ellipsoid() {
        let ellipsoid = Ellipsoid::WGS84;
        let geometry = PlotGeometry::demo(SceneMode::Globe, ellipsoid);
        let radius = |position: [f32; 3]| {
            ((position[0] * position[0] + position[1] * position[1] + position[2] * position[2])
                as f64)
                .sqrt()
        };
        let low = ellipsoid.min_radius() - 1.0;
        let high = ellipsoid.max_radius() + LIFT_POINT_M + 1.0;
        for vertex in &geometry.fill {
            let r = radius(vertex.position);
            assert!(r > low && r < high, "fill radius {r}");
        }
        for line in &geometry.lines {
            for position in [line.a, line.b] {
                let r = radius([position[0], position[1], position[2]]);
                assert!(r > low && r < high, "line radius {r}");
            }
        }
        for point in &geometry.points {
            let r = radius([point.center[0], point.center[1], point.center[2]]);
            assert!(r > low && r < high, "point radius {r}");
        }
        // Nothing may collapse into the Earth's centre (a sign of a bad
        // projection or a degenerate local frame).
        assert!(
            geometry
                .fill
                .iter()
                .all(|vertex| radius(vertex.position) > 6.0e6)
        );
    }

    #[test]
    fn globe_metric_offsets_keep_their_radius() {
        let projector = Projector::new(SceneMode::Globe, Ellipsoid::WGS84);
        let center = [DEMO_CENTER_LON, DEMO_CENTER_LAT];
        let origin = projector.project(center[0], center[1], 0.0, 0.0, 0.0);
        let origin = [origin[0] as f64, origin[1] as f64, origin[2] as f64];
        let origin_unit = unit(origin);
        for angle in [0.0f64, 30.0, 75.0, 120.0, 200.0, 330.0] {
            let radians: f64 = angle.to_radians();
            let sample = projector.project(
                center[0],
                center[1],
                DEMO_CIRCLE_RADIUS_M * radians.cos(),
                DEMO_CIRCLE_RADIUS_M * radians.sin(),
                0.0,
            );
            let sample = [sample[0] as f64, sample[1] as f64, sample[2] as f64];
            let dot = origin_unit
                .iter()
                .zip(unit(sample).iter())
                .map(|(a, b)| a * b)
                .sum::<f64>()
                .clamp(-1.0, 1.0);
            let distance = dot.acos() * 6_371_000.0;
            let error = (distance - DEMO_CIRCLE_RADIUS_M).abs();
            assert!(
                error < 3_000.0,
                "angle {angle}: {distance} m ({error} m off)"
            );
        }
    }

    #[test]
    fn sector_arc_stays_inside_the_sweep() {
        // The 90 degree sector must not wrap around the circle.
        let ring = arc_offsets(250_000.0, 30.0, 120.0, 18);
        assert_eq!(ring.len(), 19);
        for [east, north] in ring {
            let angle = north.atan2(east).to_degrees();
            assert!((-60.0..=120.0).contains(&angle), "angle {angle}");
        }
        // A full circle omits the duplicate end point.
        let full = arc_offsets(200_000.0, 0.0, 360.0, CIRCLE_SEGMENTS);
        assert_eq!(full.len(), CIRCLE_SEGMENTS);
    }

    #[test]
    fn fills_are_tessellated_at_several_depths() {
        let sources = DemoShapes::demo();
        let geometry = PlotGeometry::from_shapes(&sources, SceneMode::Globe, Ellipsoid::WGS84);
        // Polygon fan + disc + sector, each with concentric rings.
        assert!(
            geometry.fill.len() / 3 > 300,
            "{} triangles",
            geometry.fill.len() / 3
        );
    }

    #[test]
    fn plane_depth_bias_respects_its_bounds() {
        let plane = PlaneCamera::default();
        let bias = plane_depth_bias(plane.height, plane.near);
        assert!(bias[0] >= PLANE_BIAS_MIN && bias[2] <= PLANE_BIAS_MAX + 2.0 * PLANE_BIAS_STEP);
        assert!(bias[0] < bias[1] && bias[1] < bias[2]);
        // At the closest zoom the bias must keep shrinking with the ground
        // distance it fakes, otherwise it would swallow the icon overlay. Past
        // ~10,000 km eye altitude the depth buffer cannot separate the plane
        // from the overlay at all (one f32 step there is hundreds of km of
        // ground distance), so the bound is only meaningful in the normal range.
        for height in [20_000.0, 500_000.0, 9_000_000.0] {
            let bias = plane_depth_bias(height, 500.0);
            let fake_metres = bias[0] as f64 * height * height / 500.0;
            assert!(fake_metres < 0.9 * demo_altitude(height as f32) as f64);
        }
    }

    /// Flattened ids of a document, the ordinal space CLI selection uses.
    fn ordinals(document: &PlotDocument) -> Vec<ShapeId> {
        (0..document.len())
            .map(|ordinal| document.id_by_ordinal(ordinal).expect("ordinal must exist"))
            .collect()
    }

    fn sector_of(document: &PlotDocument, id: ShapeId) -> GeoSector {
        match document.data(id).expect("shape data") {
            ShapeData::Sector(sector) => sector,
            other => panic!("expected a sector, got {:?}", other.kind()),
        }
    }

    fn point_of(document: &PlotDocument, id: ShapeId) -> GeoPoint {
        match document.data(id).expect("shape data") {
            ShapeData::Point(point) => point,
            other => panic!("expected a point, got {:?}", other.kind()),
        }
    }

    #[test]
    fn document_ids_are_unique_and_ordered_by_layer() {
        let document = PlotDocument::demo();
        assert_eq!(document.len(), DEMO_POINT_COUNT + 4);
        let ids = ordinals(&document);
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "ids must be unique");
        // Point, line, polygon, circle, sector: ordinals follow the layer order.
        assert_eq!(document.locate(ids[0]).unwrap().kind, ShapeKind::Point);
        assert_eq!(
            document.locate(ids[DEMO_POINT_COUNT]).unwrap(),
            ShapeRef::new(ShapeKind::Polyline, 0)
        );
        assert_eq!(
            document.locate(ids[DEMO_POINT_COUNT + 1]).unwrap(),
            ShapeRef::new(ShapeKind::Polygon, 0)
        );
        assert_eq!(
            document.locate(ids[DEMO_POINT_COUNT + 2]).unwrap(),
            ShapeRef::new(ShapeKind::Sector, 0)
        );
        assert_eq!(document.ordinal_of(ids[3]), Some(3));
        assert_eq!(document.id_by_ordinal(ids.len()), None);
        assert_eq!(document.counts(), [DEMO_POINT_COUNT, 1, 1, 2]);
    }

    #[test]
    fn deleting_a_shape_keeps_the_other_ids() {
        let mut document = PlotDocument::demo();
        let before = ordinals(&document);
        let victim = document.id_by_ordinal(3).expect("fourth point");
        document.select(Some(victim));
        assert_eq!(document.delete_selected(), Some(victim));
        assert_eq!(document.len(), before.len() - 1);
        assert!(document.data(victim).is_none());
        assert_eq!(document.selected(), None);
        let expected: Vec<ShapeId> = before.into_iter().filter(|id| *id != victim).collect();
        assert_eq!(ordinals(&document), expected);
    }

    #[test]
    fn delete_then_undo_restores_the_polygon() {
        let mut document = PlotDocument::demo();
        let polygon = document
            .id_by_ordinal(DEMO_POINT_COUNT + 1)
            .expect("the demo polygon");
        let original = document.shapes().polygons[0].clone();
        document.select(Some(polygon));
        assert!(document.delete_selected().is_some());
        assert!(document.shapes().polygons.is_empty());
        assert!(document.selected().is_none());
        assert_eq!(document.undo_depth(), 1);

        assert!(document.undo());
        assert_eq!(document.shapes().polygons.len(), 1);
        assert_eq!(document.shapes().polygons[0].ring, original.ring);
        assert_eq!(document.shapes().polygons[0].fill, original.fill);
        // Undo restores the document-wide state, including the selection.
        assert_eq!(document.selected(), Some(polygon));
        assert_eq!(document.ordinal_of(polygon), Some(DEMO_POINT_COUNT + 1));
        // The stack is empty again.
        assert!(!document.undo());
        assert_eq!(document.len(), DEMO_POINT_COUNT + 4);
    }

    #[test]
    fn drag_moves_the_sector_centre_and_is_one_undo_step() {
        let mut document = PlotDocument::demo();
        let id = document
            .id_by_ordinal(DEMO_POINT_COUNT + 3)
            .expect("the demo sector");
        let origin = sector_of(&document, id);
        document.select(Some(id));
        assert!(document.begin_drag(id, [origin.lon, origin.lat]));
        assert!(document.drag_to([0.5, -0.25]));
        // A second move replaces the delta instead of stacking it.
        assert!(document.drag_to([1.0, -0.5]));
        let moved = sector_of(&document, id);
        assert!((moved.lon - (origin.lon + 1.0)).abs() < 1.0e-12);
        assert!((moved.lat - (origin.lat - 0.5)).abs() < 1.0e-12);
        // The radius and the sweep are untouched: only the anchor moves.
        assert_eq!(moved.radius_m, origin.radius_m);
        assert_eq!(moved.start_deg, origin.start_deg);
        assert_eq!(document.end_drag(), Some(id));
        assert_eq!(document.undo_depth(), 1);
        assert_eq!(document.edits(), 1);

        assert!(document.undo());
        let restored = sector_of(&document, id);
        assert_eq!(restored.lon, origin.lon);
        assert_eq!(restored.lat, origin.lat);
    }

    #[test]
    fn a_click_without_movement_is_not_an_undo_step() {
        let mut document = PlotDocument::demo();
        let id = document.id_by_ordinal(0).expect("first point");
        document.select(Some(id));
        assert!(document.begin_drag(id, [0.0, 0.0]));
        assert_eq!(document.end_drag(), None);
        assert_eq!(document.undo_depth(), 0);
        assert_eq!(document.edits(), 0);
        assert!(!document.undo());
    }

    #[test]
    fn drag_moves_every_path_coordinate() {
        let mut document = PlotDocument::demo();
        let polyline = document.id_by_ordinal(DEMO_POINT_COUNT).expect("polyline");
        let polygon = document
            .id_by_ordinal(DEMO_POINT_COUNT + 1)
            .expect("polygon");
        let line_before = document.shapes().polylines[0].points.clone();
        let ring_before = document.shapes().polygons[0].ring.clone();
        assert!(document.translate_shape(polyline, [0.4, 0.6]));
        assert!(document.translate_shape(polygon, [-0.3, 0.2]));

        for (before, after) in line_before
            .iter()
            .zip(&document.shapes().polylines[0].points)
        {
            assert!((after[0] - (before[0] + 0.4)).abs() < 1.0e-12);
            assert!((after[1] - (before[1] + 0.6)).abs() < 1.0e-12);
        }
        for (before, after) in ring_before.iter().zip(&document.shapes().polygons[0].ring) {
            assert!((after[0] - (before[0] - 0.3)).abs() < 1.0e-12);
            assert!((after[1] - (before[1] + 0.2)).abs() < 1.0e-12);
        }
        // Both edits are on the undo stack, in order.
        assert_eq!(document.undo_depth(), 2);
        assert!(document.undo());
        assert_eq!(document.shapes().polygons[0].ring, ring_before);
    }

    #[test]
    fn dragged_shapes_stay_inside_the_valid_longitude_range() {
        let mut shapes = DemoShapes::empty();
        shapes.points.push(GeoPoint {
            lon: 179.0,
            lat: 10.0,
            size_px: 5.0,
            color: [1.0; 4],
        });
        let mut document = PlotDocument::new(shapes);
        let id = document.id_by_ordinal(0).expect("one point");
        assert!(document.translate_shape(id, [3.0, 0.0]));
        let moved = point_of(&document, id);
        assert!(
            (-180.0..180.0).contains(&moved.lon),
            "wrapped longitude {}",
            moved.lon
        );
        assert!((moved.lon + 178.0).abs() < 1.0e-12, "lon {}", moved.lon);
        assert!((moved.lat - 10.0).abs() < 1.0e-12);
    }

    #[test]
    fn selection_is_cleared_by_none_and_unknown_ids() {
        let mut document = PlotDocument::demo();
        assert_eq!(document.selected(), None);
        let id = document
            .id_by_ordinal(DEMO_POINT_COUNT + 1)
            .expect("polygon");
        assert!(document.select(Some(id)));
        assert_eq!(
            document.selected_ref(),
            Some(ShapeRef::new(ShapeKind::Polygon, 0))
        );
        // Selecting the same shape twice is not a change (no needless rebuild).
        assert!(!document.select(Some(id)));
        assert!(document.select(None));
        assert_eq!(document.selected(), None);
        // A stale/unknown id clears rather than pointing at another shape.
        assert!(!document.select(Some(ShapeId(9_999))));
        assert_eq!(document.selected(), None);
    }

    #[test]
    fn geometry_follows_the_document() {
        let mut document = PlotDocument::demo();
        let before = PlotGeometry::from_document(&document, SceneMode::Plane, Ellipsoid::WGS84);
        assert_eq!(before.points.len(), DEMO_POINT_COUNT);

        let id = document.id_by_ordinal(0).expect("first point");
        assert!(document.translate_shape(id, [1.0, 0.0]));
        let after = PlotGeometry::from_document(&document, SceneMode::Plane, Ellipsoid::WGS84);
        let dx = after.points[0].center[0] - before.points[0].center[0];
        let expected = (EARTH_RADIUS * 1.0f64.to_radians()) as f32;
        assert!((dx - expected).abs() < 1.0, "dx {dx} expected {expected}");
        assert_eq!(after.points[0].center[1], before.points[0].center[1]);

        // A deletion shrinks the batch and a selected shape changes the colours.
        document.select(Some(id));
        let highlighted =
            PlotGeometry::from_document(&document, SceneMode::Plane, Ellipsoid::WGS84);
        assert_eq!(highlighted.points.len(), DEMO_POINT_COUNT + 1);
        document.delete_selected();
        let deleted = PlotGeometry::from_document(&document, SceneMode::Plane, Ellipsoid::WGS84);
        assert_eq!(deleted.points.len(), DEMO_POINT_COUNT - 1);
    }

    fn unit(v: [f64; 3]) -> [f64; 3] {
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / length, v[1] / length, v[2] / length]
    }
}
