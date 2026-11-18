//! CPU screen-space picking and drag conversion for the vector plot.
//!
//! Everything here is pure CPU math: no GPU readback, no per-shape draw, no
//! depth lookups. The same matrices the plot shader receives are reused so what
//! the user clicks is what the GPU drew:
//!
//! * `2D` — [`PlotScene::camera_plane`](crate::scene::plot::PlotScene::camera_plane)
//!   (the plane camera already shifted by `anchor - ground`), world vertices in
//!   anchored plane metres with `z = 0`.
//! * `3D` — `GlobeCamera::view(..)`, world vertices in absolute WGS84 ECEF.
//!
//! ## Hit rules
//!
//! | layer | hit test | slop |
//! | --- | --- | --- |
//! | point | centre distance ≤ `max(size_px / 2, 7 px)` | 7 px floor |
//! | polyline | distance to the nearest projected segment | `width_px / 2 + 5 px` |
//! | polygon / circle / sector | screen-space point-in-polygon of the outer ring, or distance to its edge | `width_px / 2 + 5 px` |
//!
//! Priority is the layer order **point > polyline > polygon > sector**; inside a
//! layer the nearest hit wins and an exact tie goes to the lower index. A
//! selected point is drawn 1.35x larger than its authored size, so the 7 px floor
//! keeps it clickable at every zoom.
//!
//! On the globe a shape is skipped when its own surface point faces away from
//! the eye or fails the horizon test
//! ([`HorizonOccluder`](crate::math::camera::HorizonOccluder)), so markers on the
//! far side of the Earth can never be picked even though they project inside the
//! screen rectangle.
//!
//! Picking projects to the *reference surface* and ignores the plot layer lift
//! (`LIFT_FILL_M` / `LIFT_LINE_M` / `LIFT_POINT_M`, at most 420 m). At the demo
//! zoom that parallax is far below one pixel; it would only matter below a few
//! tens of kilometres of eye altitude, where the demo shapes are off screen.
//!
//! ## Drag conversion
//!
//! A drag applies a *geographic delta* to the shape: the lon/lat under the press
//! and under the current cursor are both resolved from the inverse view-projection
//! (`2D`: ray ∩ `z = 0` plane; `3D`: ray ∩ WGS84 ellipsoid), and their difference
//! moves every coordinate of the shape. On the 2D plane that difference equals
//! the classic `dx · meters_per_pixel` mapping at the view centre — see
//! [`plane_pixel_delta`], which the tests cross-check against the ray path.

use glam::{DMat4, DVec3, DVec4, Mat4, Vec3};

use crate::math::camera::{GlobeCamera, HorizonOccluder, PlaneCamera};
use crate::math::geo::{EARTH_RADIUS, Ellipsoid, lonlat_to_plane, plane_to_lonlat};
use crate::scene::SceneMode;
use crate::scene::plot::{
    PlotDocument, ShapeId, ShapeKind, ShapeRef, geodetic_offset, plane_offset, sector_ring,
};

/// Minimum click radius of a point marker, in physical pixels.
pub const POINT_PICK_MIN_PX: f32 = 7.0;
/// Extra tolerance around a polyline, beyond half its stroke width.
pub const LINE_PICK_SLOP_PX: f32 = 5.0;
/// Extra tolerance around an area outline, beyond half its stroke width.
pub const AREA_PICK_SLOP_PX: f32 = 5.0;

/// One pick result: which shape and how far the cursor was from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlotHit {
    pub id: ShapeId,
    pub kind: ShapeKind,
    /// Index inside the kind's authoring vector.
    pub index: usize,
    /// Screen distance in pixels: centre distance for a point, distance to the
    /// nearest segment/edge for a line or area, `0` when the cursor is inside an
    /// area.
    pub distance: f32,
}

/// A resolved world → pixel mapping for one mode, valid for one frame.
///
/// The matrices are `f64` end to end so a pick at 2 km altitude does not inherit
/// the GPU's `f32` ECEF quantisation (~0.5 m). The projection reproduces the
/// vertex shader's clip→NDC→viewport chain exactly, including the wgpu `z ∈ [0,1]`
/// convention, which is what makes the inverse usable for drags.
#[derive(Clone, Debug)]
pub struct ScreenProjection {
    mode: SceneMode,
    ellipsoid: Ellipsoid,
    view_proj: DMat4,
    inverse: DMat4,
    /// Viewport size in physical pixels.
    screen: [f32; 2],
    /// Eye in the same space as the projected world positions (globe only).
    eye: Option<DVec3>,
    /// 2D render origin in plane metres (the plot vertices are relative to it).
    anchor_plane: [f64; 2],
    occluder: Option<HorizonOccluder>,
}

impl ScreenProjection {
    /// 2D projection from the active plane camera. `anchor_plane` must be the
    /// plot scene's render origin ([`PlotScene::anchor_plane`]).
    ///
    /// [`PlotScene::anchor_plane`]: crate::scene::plot::PlotScene::anchor_plane
    pub fn plane(
        anchor_plane: [f64; 2],
        camera: &PlaneCamera,
        aspect: f64,
        screen: [f32; 2],
    ) -> Self {
        let ground = lonlat_to_plane(camera.center_lon, camera.center_lat);
        let shift = Vec3::new(
            (anchor_plane[0] - ground[0]) as f32,
            (anchor_plane[1] - ground[1]) as f32,
            0.0,
        );
        let view_proj = (camera.view_proj(aspect) * Mat4::from_translation(shift)).as_dmat4();
        Self {
            mode: SceneMode::Plane,
            ellipsoid: Ellipsoid::WGS84,
            view_proj,
            inverse: view_proj.inverse(),
            screen,
            eye: None,
            anchor_plane,
            occluder: None,
        }
    }

    /// 3D projection from the active globe camera.
    pub fn globe(
        camera: &GlobeCamera,
        ellipsoid: Ellipsoid,
        aspect: f64,
        screen: [f32; 2],
    ) -> Self {
        let view = camera.view(aspect, ellipsoid);
        let eye = DVec3::from_array(view.eye);
        Self {
            mode: SceneMode::Globe,
            ellipsoid,
            view_proj: view.view_proj,
            inverse: view.view_proj.inverse(),
            screen,
            eye: Some(eye),
            anchor_plane: [0.0; 2],
            occluder: Some(HorizonOccluder::new(eye, ellipsoid)),
        }
    }

    /// World-space position of a geographic point in the active mode: anchored
    /// plane metres (`z = 0`) or absolute ECEF at the surface.
    pub fn world(&self, lon: f64, lat: f64) -> DVec3 {
        match self.mode {
            SceneMode::Plane => {
                let point = lonlat_to_plane(lon, lat);
                DVec3::new(
                    point[0] - self.anchor_plane[0],
                    point[1] - self.anchor_plane[1],
                    0.0,
                )
            }
            SceneMode::Globe => DVec3::from_array(self.ellipsoid.surface(lon, lat)),
        }
    }

    /// Geographic point → physical pixel, or `None` when it is behind the eye.
    pub fn project_lonlat(&self, lon: f64, lat: f64) -> Option<[f32; 2]> {
        self.project_world(self.world(lon, lat))
    }

    /// World position → physical pixel, or `None` when it is behind the eye.
    pub fn project_world(&self, world: DVec3) -> Option<[f32; 2]> {
        let clip = self.view_proj * DVec4::new(world.x, world.y, world.z, 1.0);
        if clip.w <= 1.0e-6 {
            return None;
        }
        let ndc = DVec3::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);
        Some([
            ((ndc.x * 0.5 + 0.5) * self.screen[0] as f64) as f32,
            ((0.5 - ndc.y * 0.5) * self.screen[1] as f64) as f32,
        ])
    }

    /// Is a surface point facing the eye and above the horizon?
    ///
    /// Always true on the 2D plane. On the globe this is the cheap two-part test
    /// that keeps the far hemisphere unclickable: the local normal must point
    /// towards the eye, and the sphere of `radius_m` around the point must reach
    /// past the horizon plane through the eye.
    pub fn surface_visible(&self, lon: f64, lat: f64, radius_m: f64) -> bool {
        let Some(eye) = self.eye else {
            return true;
        };
        let point = DVec3::from_array(self.ellipsoid.surface(lon, lat));
        let normal = DVec3::from_array(self.ellipsoid.normal(lon, lat));
        if normal.dot(eye - point) <= 0.0 {
            return false;
        }
        match self.occluder {
            Some(occluder) => occluder.sphere_visible(point, radius_m.max(0.0)),
            None => true,
        }
    }

    /// Pixel → geographic point on the reference surface.
    ///
    /// `2D`: the ray's intersection with the anchored `z = 0` plane.
    /// `3D`: the ray's intersection with the WGS84 ellipsoid. Returns `None` for
    /// pixels that miss the surface (beyond the horizon on the globe) or look
    /// away from it.
    pub fn pixel_to_lonlat(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        let (origin, direction) = self.ray(pixel)?;
        match self.mode {
            SceneMode::Plane => {
                if direction.z.abs() < 1.0e-12 {
                    return None;
                }
                let t = -origin.z / direction.z;
                if t <= 0.0 {
                    return None;
                }
                let x = origin.x + direction.x * t + self.anchor_plane[0];
                let y = origin.y + direction.y * t + self.anchor_plane[1];
                let (lon, lat) = plane_to_lonlat(x, y);
                Some([lon, lat])
            }
            SceneMode::Globe => {
                let hit = self
                    .ellipsoid
                    .ray_intersection(origin.to_array(), direction.to_array())?;
                let (lon, lat, _) = self.ellipsoid.to_geodetic(hit);
                Some([lon, lat])
            }
        }
    }

    /// Geographic delta from one pixel to another: the drag transform.
    pub fn drag_delta_from(&self, anchor: [f64; 2], cursor: [f32; 2]) -> Option<[f64; 2]> {
        let end = self.pixel_to_lonlat(cursor)?;
        Some([shortest_lon_delta(anchor[0], end[0]), end[1] - anchor[1]])
    }

    /// Eye ray for a pixel: `(origin_at_near_plane, normalized_direction)`.
    fn ray(&self, pixel: [f32; 2]) -> Option<(DVec3, DVec3)> {
        let width = self.screen[0].max(1.0) as f64;
        let height = self.screen[1].max(1.0) as f64;
        let ndc_x = pixel[0] as f64 / width * 2.0 - 1.0;
        let ndc_y = 1.0 - pixel[1] as f64 / height * 2.0;
        let near = self.inverse * DVec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far = self.inverse * DVec4::new(ndc_x, ndc_y, 1.0, 1.0);
        if near.w.abs() < 1.0e-12 || far.w.abs() < 1.0e-12 {
            return None;
        }
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        let direction = far - near;
        if direction.length_squared() < 1.0e-30 {
            return None;
        }
        Some((near, direction.normalize()))
    }

    /// Geographic lon/lat of a local east/north offset from `center`, matching
    /// the projection the geometry builder uses for fill rings.
    fn offset_lonlat(&self, center: [f64; 2], east_m: f64, north_m: f64) -> (f64, f64) {
        match self.mode {
            SceneMode::Plane => plane_offset(center, east_m, north_m),
            SceneMode::Globe => {
                geodetic_offset(self.ellipsoid, center[0], center[1], east_m, north_m)
            }
        }
    }
}

/// The classic 2D pixel → degree mapping: `dx · meters_per_pixel` at the ground
/// plane, split into lon/lat (screen y grows south).
///
/// [`ScreenProjection::pixel_to_lonlat`] solves the same transform exactly by
/// inverting the view-projection; this linear form stays available (and is
/// tested against it) because it is the formula the map convention documents.
#[allow(dead_code)] // Authoring/diagnostic helper, cross-checked by the tests.
pub fn plane_pixel_delta(camera: &PlaneCamera, viewport_height: f64, dx: f64, dy: f64) -> [f64; 2] {
    let mpp = camera.meters_per_pixel(viewport_height);
    let degrees = mpp / (EARTH_RADIUS * std::f64::consts::PI / 180.0);
    [dx * degrees, -dy * degrees]
}

/// Pick the topmost shape under `cursor` (physical pixels, top-left origin).
///
/// Layer priority is point > polyline > polygon > sector; the nearest shape
/// inside the winning layer wins, an exact tie goes to the lower index.
pub fn pick(
    document: &PlotDocument,
    projection: &ScreenProjection,
    cursor: [f32; 2],
) -> Option<PlotHit> {
    for kind in ShapeKind::ALL {
        let hit = match kind {
            ShapeKind::Point => pick_points(document, projection, cursor),
            ShapeKind::Polyline => pick_polylines(document, projection, cursor),
            ShapeKind::Polygon => pick_areas(document, projection, cursor, ShapeKind::Polygon),
            ShapeKind::Sector => pick_areas(document, projection, cursor, ShapeKind::Sector),
        };
        if hit.is_some() {
            return hit;
        }
    }
    None
}

/// Screen centre of a shape (its anchor point), for scripted clicks/drags.
pub fn shape_screen_position(
    document: &PlotDocument,
    projection: &ScreenProjection,
    id: ShapeId,
) -> Option<[f32; 2]> {
    let center = document.shape_center(id)?;
    projection.project_lonlat(center[0], center[1])
}

fn pick_points(
    document: &PlotDocument,
    projection: &ScreenProjection,
    cursor: [f32; 2],
) -> Option<PlotHit> {
    let mut best: Option<PlotHit> = None;
    for (index, point) in document.shapes().points.iter().enumerate() {
        if !projection.surface_visible(point.lon, point.lat, 0.0) {
            continue;
        }
        let Some(screen) = projection.project_lonlat(point.lon, point.lat) else {
            continue;
        };
        let distance = screen_distance(screen, cursor);
        let radius = (point.size_px * 0.5).max(POINT_PICK_MIN_PX);
        if distance > radius {
            continue;
        }
        let Some(id) = document.id_at(ShapeRef::new(ShapeKind::Point, index)) else {
            continue;
        };
        if best.is_none_or(|hit| distance < hit.distance) {
            best = Some(PlotHit {
                id,
                kind: ShapeKind::Point,
                index,
                distance,
            });
        }
    }
    best
}

fn pick_polylines(
    document: &PlotDocument,
    projection: &ScreenProjection,
    cursor: [f32; 2],
) -> Option<PlotHit> {
    let mut best: Option<PlotHit> = None;
    for (index, polyline) in document.shapes().polylines.iter().enumerate() {
        if polyline.points.len() < 2 {
            continue;
        }
        let projected: Vec<Option<[f32; 2]>> = polyline
            .points
            .iter()
            .map(|point| project_visible(projection, point[0], point[1], 0.0))
            .collect();
        let threshold = polyline.width_px * 0.5 + LINE_PICK_SLOP_PX;
        let mut nearest = f32::INFINITY;
        for pair in projected.windows(2) {
            let (Some(a), Some(b)) = (pair[0], pair[1]) else {
                continue;
            };
            nearest = nearest.min(segment_distance(cursor, a, b));
        }
        if nearest > threshold {
            continue;
        }
        let Some(id) = document.id_at(ShapeRef::new(ShapeKind::Polyline, index)) else {
            continue;
        };
        if best.is_none_or(|hit| nearest < hit.distance) {
            best = Some(PlotHit {
                id,
                kind: ShapeKind::Polyline,
                index,
                distance: nearest,
            });
        }
    }
    best
}

fn pick_areas(
    document: &PlotDocument,
    projection: &ScreenProjection,
    cursor: [f32; 2],
    kind: ShapeKind,
) -> Option<PlotHit> {
    let shapes = document.shapes();
    let count = match kind {
        ShapeKind::Polygon => shapes.polygons.len(),
        ShapeKind::Sector => shapes.sectors.len(),
        _ => return None,
    };
    let mut best: Option<PlotHit> = None;
    for index in 0..count {
        // Build the ring in geographic coordinates: authored vertices for a
        // polygon, the sampled arc plus its centre for a sector.
        let ring: Vec<[f64; 2]> = match kind {
            ShapeKind::Polygon => shapes.polygons[index].ring.clone(),
            ShapeKind::Sector => {
                let sector = &shapes.sectors[index];
                let center = [sector.lon, sector.lat];
                let local = sector_ring(sector);
                let mut ring = Vec::with_capacity(local.len() + 1);
                if !sector.is_full_circle() {
                    ring.push(center);
                }
                ring.extend(local.iter().map(|offset| {
                    let (lon, lat) = projection.offset_lonlat(center, offset[0], offset[1]);
                    [lon, lat]
                }));
                ring
            }
            _ => return None,
        };
        if ring.len() < 3 {
            continue;
        }
        let projected: Vec<[f32; 2]> = ring
            .iter()
            .filter_map(|point| project_visible(projection, point[0], point[1], 0.0))
            .collect();
        if projected.len() < 3 {
            continue;
        }
        let width_px = match kind {
            ShapeKind::Polygon => shapes.polygons[index].width_px,
            ShapeKind::Sector => shapes.sectors[index].width_px,
            _ => 0.0,
        };
        let edge = ring_edge_distance(cursor, &projected);
        let inside = point_in_polygon(cursor, &projected);
        let threshold = width_px * 0.5 + AREA_PICK_SLOP_PX;
        let Some(distance) = (if inside {
            Some(0.0)
        } else if edge <= threshold {
            Some(edge)
        } else {
            None
        }) else {
            continue;
        };
        let Some(id) = document.id_at(ShapeRef::new(kind, index)) else {
            continue;
        };
        if best.is_none_or(|hit| distance < hit.distance) {
            best = Some(PlotHit {
                id,
                kind,
                index,
                distance,
            });
        }
    }
    best
}

fn project_visible(
    projection: &ScreenProjection,
    lon: f64,
    lat: f64,
    radius_m: f64,
) -> Option<[f32; 2]> {
    if !projection.surface_visible(lon, lat, radius_m) {
        return None;
    }
    projection.project_lonlat(lon, lat)
}

fn screen_distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn segment_distance(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length_sq = ab[0] * ab[0] + ab[1] * ab[1];
    if length_sq <= 1.0e-12 {
        return screen_distance(point, a);
    }
    let t = (((point[0] - a[0]) * ab[0] + (point[1] - a[1]) * ab[1]) / length_sq).clamp(0.0, 1.0);
    screen_distance(point, [a[0] + ab[0] * t, a[1] + ab[1] * t])
}

/// Even-odd point-in-polygon test on the projected outer ring.
fn point_in_polygon(point: [f32; 2], ring: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = ring.len() - 1;
    for current in 0..ring.len() {
        let a = ring[current];
        let b = ring[previous];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let crossing = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if point[0] < crossing {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// Distance from `point` to the closest edge of a closed ring.
fn ring_edge_distance(point: [f32; 2], ring: &[[f32; 2]]) -> f32 {
    let mut nearest = f32::INFINITY;
    let mut previous = ring.len() - 1;
    for current in 0..ring.len() {
        nearest = nearest.min(segment_distance(point, ring[previous], ring[current]));
        previous = current;
    }
    nearest
}

/// Wrap `to - from` (degrees) into `[-180, 180)`.
fn shortest_lon_delta(from: f64, to: f64) -> f64 {
    let delta = (to - from + 180.0).rem_euclid(360.0) - 180.0;
    if delta >= 180.0 { delta - 360.0 } else { delta }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::geo::Ellipsoid;
    use crate::scene::plot::{
        DEMO_CENTER_LAT, DEMO_CENTER_LON, DemoShapes, GeoPoint, GeoPolygon, GeoPolyline, GeoSector,
        PlotGeometry, SELECT_FILL, SELECT_POINT,
    };

    const SCREEN: [f32; 2] = [1600.0, 900.0];

    fn plane_projection(height: f64) -> ScreenProjection {
        let camera = PlaneCamera {
            center_lon: DEMO_CENTER_LON,
            center_lat: DEMO_CENTER_LAT,
            height,
            ..PlaneCamera::default()
        };
        ScreenProjection::plane(
            lonlat_to_plane(DEMO_CENTER_LON, DEMO_CENTER_LAT),
            &camera,
            SCREEN[0] as f64 / SCREEN[1] as f64,
            SCREEN,
        )
    }

    fn globe_projection(camera: &GlobeCamera) -> ScreenProjection {
        ScreenProjection::globe(
            camera,
            Ellipsoid::WGS84,
            SCREEN[0] as f64 / SCREEN[1] as f64,
            SCREEN,
        )
    }

    fn document_with(shapes: DemoShapes) -> PlotDocument {
        PlotDocument::new(shapes)
    }

    fn point_at(lon: f64, lat: f64, size_px: f32) -> GeoPoint {
        GeoPoint {
            lon,
            lat,
            size_px,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn plane_projection_matches_the_camera_math() {
        let projection = plane_projection(9_000_000.0);
        let center = projection
            .project_lonlat(DEMO_CENTER_LON, DEMO_CENTER_LAT)
            .expect("center must project");
        assert!((center[0] - 800.0).abs() < 0.01, "x {}", center[0]);
        assert!((center[1] - 450.0).abs() < 0.01, "y {}", center[1]);

        // One degree of longitude east sits `R·1°` metres away and the plane is a
        // linear projection at z = 0, so the pixel offset is exact.
        let east = projection
            .project_lonlat(DEMO_CENTER_LON + 1.0, DEMO_CENTER_LAT)
            .expect("east point must project");
        let mpp = 2.0 * 9_000_000.0 * (30.0f64.to_radians() * 0.5).tan() / 900.0;
        let expected = (EARTH_RADIUS * 1.0f64.to_radians() / mpp) as f32;
        assert!((east[0] - 800.0 - expected).abs() < 0.05, "x {}", east[0]);
        assert!((east[1] - 450.0).abs() < 0.01, "y {}", east[1]);

        // North is up on screen.
        let north = projection
            .project_lonlat(DEMO_CENTER_LON, DEMO_CENTER_LAT + 1.0)
            .expect("north point must project");
        assert!(north[1] < 450.0 - 10.0, "y {}", north[1]);
    }

    #[test]
    fn plane_pixel_delta_agrees_with_inverse_projection() {
        let height = 2_000_000.0;
        let camera = PlaneCamera {
            center_lon: DEMO_CENTER_LON,
            center_lat: DEMO_CENTER_LAT,
            height,
            ..PlaneCamera::default()
        };
        let projection = ScreenProjection::plane(
            lonlat_to_plane(DEMO_CENTER_LON, DEMO_CENTER_LAT),
            &camera,
            SCREEN[0] as f64 / SCREEN[1] as f64,
            SCREEN,
        );
        for (dx, dy) in [(120.0, 0.0), (0.0, 90.0), (-45.0, 60.0)] {
            let from = [800.0, 450.0];
            let to = [from[0] + dx, from[1] + dy];
            let anchor = projection.pixel_to_lonlat(from).expect("press point");
            let ray = projection.drag_delta_from(anchor, to).expect("drag delta");
            let mpp = plane_pixel_delta(&camera, 900.0, dx as f64, dy as f64);
            // The camera matrix is built in f32, the analytic form in f64, so the
            // agreement is ~1e-7 degrees rather than exact.
            assert!(
                (ray[0] - mpp[0]).abs() < 1.0e-6,
                "dlon {} vs {}",
                ray[0],
                mpp[0]
            );
            assert!(
                (ray[1] - mpp[1]).abs() < 1.0e-6,
                "dlat {} vs {}",
                ray[1],
                mpp[1]
            );
        }
        // Sanity: 120 px right and 90 px down is east and south.
        let center = projection.pixel_to_lonlat([800.0, 450.0]).expect("centre");
        let delta = projection
            .drag_delta_from(center, [920.0, 540.0])
            .expect("delta");
        assert!(delta[0] > 0.0 && delta[1] < 0.0, "delta {delta:?}");
    }

    #[test]
    fn globe_projection_centres_the_view_axis_and_hides_the_far_side() {
        // The camera looks at the Earth centre, so the surface point on that axis
        // must land exactly on the screen centre.
        let camera = GlobeCamera::default();
        let ellipsoid = Ellipsoid::WGS84;
        let projection = globe_projection(&camera);
        let eye = camera.eye_ecef(ellipsoid);
        let axis_hit = ellipsoid
            .ray_intersection(eye, [-eye[0], -eye[1], -eye[2]])
            .expect("axis ray hits the ellipsoid");
        let (lon, lat, _) = ellipsoid.to_geodetic(axis_hit);
        let center = projection
            .project_lonlat(lon, lat)
            .expect("axis point must project");
        assert!((center[0] - 800.0).abs() < 0.01, "x {}", center[0]);
        assert!((center[1] - 450.0).abs() < 0.01, "y {}", center[1]);

        // The geodetic sub-camera point is a couple of pixels off that axis on
        // WGS84, because the eye sits on the geodetic normal rather than the
        // radius through the surface point.
        let nadir = projection
            .project_lonlat(camera.yaw_deg, camera.pitch_deg)
            .expect("sub-camera point must project");
        assert!((nadir[0] - 800.0).abs() < 4.0, "x {}", nadir[0]);
        assert!((nadir[1] - 450.0).abs() < 4.0, "y {}", nadir[1]);

        // East is screen right and north is screen up around the sub-camera point.
        let east = projection
            .project_lonlat(camera.yaw_deg + 2.0, camera.pitch_deg)
            .expect("east point must project");
        assert!(east[0] > nadir[0] + 5.0, "x {}", east[0]);
        assert!((east[1] - nadir[1]).abs() < 2.0, "y {}", east[1]);

        assert!(projection.surface_visible(camera.yaw_deg, camera.pitch_deg, 0.0));
        assert!(!projection.surface_visible(camera.yaw_deg + 180.0, -camera.pitch_deg, 0.0));
    }

    #[test]
    fn globe_pixel_to_lonlat_inverts_the_projection() {
        let camera = GlobeCamera::default();
        let projection = globe_projection(&camera);
        for (lon, lat) in [(110.0, 25.0), (118.0, 30.0), (100.0, 10.0)] {
            let pixel = projection.project_lonlat(lon, lat).expect("project");
            let back = projection.pixel_to_lonlat(pixel).expect("unproject");
            assert!((back[0] - lon).abs() < 1.0e-3, "lon {lon} -> {}", back[0]);
            assert!((back[1] - lat).abs() < 1.0e-3, "lat {lat} -> {}", back[1]);
        }
        // Screen down means south on the globe.
        let anchor = projection.pixel_to_lonlat([800.0, 450.0]).expect("centre");
        let delta = projection
            .drag_delta_from(anchor, [800.0, 520.0])
            .expect("delta");
        assert!(delta[1] < 0.0, "dlat {}", delta[1]);
        assert!(delta[0].abs() < 1.0e-6, "dlon {}", delta[0]);
    }

    #[test]
    fn point_pick_uses_a_seven_pixel_floor() {
        let projection = plane_projection(9_000_000.0);
        let mut shapes = DemoShapes::empty();
        shapes
            .points
            .push(point_at(DEMO_CENTER_LON, DEMO_CENTER_LAT, 2.0));
        let document = document_with(shapes);
        let id = document.id_by_ordinal(0).expect("one point");

        let hit = pick(&document, &projection, [806.0, 450.0]).expect("inside the floor");
        assert_eq!(hit.id, id);
        assert_eq!(hit.kind, ShapeKind::Point);
        assert_eq!(hit.distance, 6.0);
        assert!(pick(&document, &projection, [808.0, 450.0]).is_none());

        // A big marker is picked out to its drawn radius.
        let mut shapes = DemoShapes::empty();
        shapes
            .points
            .push(point_at(DEMO_CENTER_LON, DEMO_CENTER_LAT, 30.0));
        let document = document_with(shapes);
        assert!(pick(&document, &projection, [814.0, 450.0]).is_some());
        assert!(pick(&document, &projection, [816.0, 450.0]).is_none());
    }

    #[test]
    fn line_pick_uses_half_width_plus_slop() {
        let projection = plane_projection(9_000_000.0);
        let mut shapes = DemoShapes::empty();
        shapes.polylines.push(GeoPolyline {
            points: vec![
                [DEMO_CENTER_LON - 1.0, DEMO_CENTER_LAT],
                [DEMO_CENTER_LON + 1.0, DEMO_CENTER_LAT],
            ],
            width_px: 4.0,
            color: [1.0, 1.0, 1.0, 1.0],
        });
        let document = document_with(shapes);
        let id = document.id_by_ordinal(0).expect("one line");
        // Half width 2 px + 5 px slop = 7 px.
        let hit = pick(&document, &projection, [800.0, 456.5]).expect("within slop");
        assert_eq!(hit.id, id);
        assert_eq!(hit.kind, ShapeKind::Polyline);
        assert!((hit.distance - 6.5).abs() < 0.01);
        assert!(pick(&document, &projection, [800.0, 458.0]).is_none());
        // Off the end of the segment there is no hit.
        assert!(pick(&document, &projection, [700.0, 450.0]).is_none());
    }

    #[test]
    fn area_pick_inside_and_near_the_edge() {
        let projection = plane_projection(9_000_000.0);
        let mut shapes = DemoShapes::empty();
        let ring: Vec<[f64; 2]> = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
            .iter()
            .map(|(dlon, dlat)| [DEMO_CENTER_LON + dlon, DEMO_CENTER_LAT + dlat])
            .collect();
        shapes.polygons.push(GeoPolygon {
            ring,
            fill: [1.0, 1.0, 1.0, 0.3],
            stroke: [1.0, 1.0, 1.0, 1.0],
            width_px: 2.0,
        });
        let document = document_with(shapes);
        let id = document.id_by_ordinal(0).expect("one polygon");

        let inside = pick(&document, &projection, [800.0, 450.0]).expect("inside");
        assert_eq!(inside.id, id);
        assert_eq!(inside.kind, ShapeKind::Polygon);
        assert_eq!(inside.distance, 0.0);

        // The ring edge of a 1° square is ~10.4 px from the centre, so 6 px past
        // it is still inside the 1 + 5 px slop band.
        let edge = projection
            .project_lonlat(DEMO_CENTER_LON + 0.5, DEMO_CENTER_LAT)
            .expect("edge vertex");
        let just_outside = [edge[0] + 5.0, edge[1]];
        assert!(pick(&document, &projection, just_outside).is_some());
        let well_outside = [edge[0] + 12.0, edge[1]];
        assert!(pick(&document, &projection, well_outside).is_none());
    }

    #[test]
    fn sector_pick_covers_the_wedge_but_not_the_opposite_side() {
        let projection = plane_projection(9_000_000.0);
        let mut shapes = DemoShapes::empty();
        shapes.sectors.push(GeoSector {
            lon: DEMO_CENTER_LON,
            lat: DEMO_CENTER_LAT,
            radius_m: 250_000.0,
            start_deg: 30.0,
            end_deg: 120.0,
            fill: [1.0, 1.0, 1.0, 0.3],
            stroke: [1.0, 1.0, 1.0, 1.0],
            width_px: 2.0,
        });
        let document = document_with(shapes);
        // 60° is inside the 30..120° sweep: 100 km out along that bearing.
        let inside = projection
            .project_lonlat(DEMO_CENTER_LON, DEMO_CENTER_LAT)
            .expect("centre");
        assert!(pick(&document, &projection, inside).is_some());

        // The opposite side of the disc centre (bearing 300°) is outside.
        let (lon, lat) = plane_offset([DEMO_CENTER_LON, DEMO_CENTER_LAT], -100_000.0, -60_000.0);
        let outside = projection.project_lonlat(lon, lat).expect("outside point");
        assert!(pick(&document, &projection, outside).is_none());
    }

    #[test]
    fn priority_is_point_then_line_then_area() {
        let projection = plane_projection(9_000_000.0);
        let center = [DEMO_CENTER_LON, DEMO_CENTER_LAT];
        let mut shapes = DemoShapes::empty();
        shapes.points.push(point_at(center[0], center[1], 6.0));
        shapes.polylines.push(GeoPolyline {
            points: vec![[center[0] - 1.0, center[1]], [center[0] + 1.0, center[1]]],
            width_px: 6.0,
            color: [1.0; 4],
        });
        shapes.polygons.push(GeoPolygon {
            ring: vec![
                [center[0] - 0.5, center[1] - 0.5],
                [center[0] + 0.5, center[1] - 0.5],
                [center[0] + 0.5, center[1] + 0.5],
                [center[0] - 0.5, center[1] + 0.5],
            ],
            fill: [1.0; 4],
            stroke: [1.0; 4],
            width_px: 2.0,
        });
        let document = document_with(shapes);
        let cursor = [800.0, 450.0];
        let hit = pick(&document, &projection, cursor).expect("point wins");
        assert_eq!(hit.kind, ShapeKind::Point);

        // Removing the point hands the click to the line, then to the polygon.
        let mut document = document.clone();
        document.select(document.id_by_ordinal(0));
        document.delete_selected().expect("point deleted");
        let hit = pick(&document, &projection, cursor).expect("line wins");
        assert_eq!(hit.kind, ShapeKind::Polyline);
        let mut document = document.clone();
        let line = document
            .id_by_ordinal(0)
            .expect("line is now the first shape");
        document.select(Some(line));
        document.delete_selected().expect("line deleted");
        let hit = pick(&document, &projection, cursor).expect("polygon wins");
        assert_eq!(hit.kind, ShapeKind::Polygon);
    }

    #[test]
    fn far_side_points_are_not_pickable() {
        let camera = GlobeCamera::default();
        let projection = globe_projection(&camera);
        let mut shapes = DemoShapes::empty();
        // Near the antipode of the sub-camera point: it projects *inside* the
        // screen rectangle (it is beyond the Earth, along the view direction), so
        // only the occlusion test can reject it.
        shapes
            .points
            .push(point_at(camera.yaw_deg + 175.0, 10.0, 40.0));
        let document = document_with(shapes);
        assert!(
            projection
                .project_lonlat(camera.yaw_deg + 175.0, 10.0)
                .is_some(),
            "the far point must project, otherwise this test proves nothing"
        );
        for column in 0..16 {
            for row in 0..9 {
                let cursor = [column as f32 * 100.0 + 50.0, row as f32 * 100.0 + 50.0];
                assert!(
                    pick(&document, &projection, cursor).is_none(),
                    "far-side point picked at {cursor:?}"
                );
            }
        }

        // The same document with a near-side marker *is* pickable there.
        let mut shapes = DemoShapes::empty();
        shapes
            .points
            .push(point_at(camera.yaw_deg, camera.pitch_deg, 40.0));
        let document = document_with(shapes);
        let cursor = projection
            .project_lonlat(camera.yaw_deg, camera.pitch_deg)
            .expect("near point projects");
        assert!(pick(&document, &projection, cursor).is_some());
    }

    #[test]
    fn selection_restyles_geometry_without_extra_draws() {
        let shapes = DemoShapes::demo();
        let selected = ShapeRef::new(ShapeKind::Polygon, 0);
        let plain =
            PlotGeometry::from_shapes_selected(&shapes, None, SceneMode::Plane, Ellipsoid::WGS84);
        let highlighted = PlotGeometry::from_shapes_selected(
            &shapes,
            Some(selected),
            SceneMode::Plane,
            Ellipsoid::WGS84,
        );
        assert!(
            plain
                .fill
                .iter()
                .any(|vertex| vertex.color == shapes.polygons[0].fill)
        );
        assert!(
            highlighted
                .fill
                .iter()
                .any(|vertex| vertex.color == SELECT_FILL)
        );
        assert!(
            !highlighted
                .fill
                .iter()
                .any(|vertex| vertex.color == shapes.polygons[0].fill),
            "the selected fill must be restyled"
        );
        // The highlight adds one glow pass and one crisp pass for the outline.
        assert!(highlighted.lines.len() > plain.lines.len());
        // Points are unaffected by an area selection.
        assert_eq!(highlighted.points.len(), plain.points.len());
    }

    #[test]
    fn selected_point_gains_a_halo_instance() {
        let mut shapes = DemoShapes::empty();
        shapes
            .points
            .push(point_at(DEMO_CENTER_LON, DEMO_CENTER_LAT, 8.0));
        let plain =
            PlotGeometry::from_shapes_selected(&shapes, None, SceneMode::Plane, Ellipsoid::WGS84);
        let highlighted = PlotGeometry::from_shapes_selected(
            &shapes,
            Some(ShapeRef::new(ShapeKind::Point, 0)),
            SceneMode::Plane,
            Ellipsoid::WGS84,
        );
        assert_eq!(plain.points.len(), 1);
        assert_eq!(highlighted.points.len(), 2);
        // Halo first, crisp marker second, both larger than the authored marker.
        assert!(highlighted.points[0].center[3] > highlighted.points[1].center[3]);
        assert!(highlighted.points[1].center[3] > 8.0);
        assert_eq!(highlighted.points[1].color, SELECT_POINT);
    }
}
