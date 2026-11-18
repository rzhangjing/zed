//! Cameras. All world positions are computed in `f64`; only the final
//! relative-to-render-origin positions become `f32` for the GPU.
//!
//! The plane camera is a narrow-FOV perspective camera at `(0, 0, height)` in a
//! render space whose origin is the ground point under the camera.
//!
//! The globe camera orbits the WGS84 ellipsoid. Its eye, target, and
//! view-projection matrix are `f64` ECEF values; the matrix is only converted to
//! `f32` once, after the `proj * view` product has been formed, so the GPU never
//! sees a `view` translation of ~6.4e6 metres combined with a small near-plane
//! distance.

use glam::{DMat4, DVec3, DVec4, Mat4, Vec3};

use crate::math::geo::{EARTH_RADIUS, Ellipsoid, MAX_MERCATOR_LAT};

const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;

/// Closest eye altitude above the ellipsoid (metres). At this altitude the
/// z=16 LOD level is reachable; `f32` vertex positions still quantise to ~0.5 m,
/// i.e. well under one pixel at 1080p.
pub const GLOBE_MIN_ALTITUDE: f64 = 2_000.0;
/// Farthest orbit distance from the Earth centre.
pub const GLOBE_MAX_DISTANCE: f64 = EARTH_RADIUS * 8.0;

#[derive(Clone, Debug)]
pub struct PlaneCamera {
    /// Geographic center of the view (degrees).
    pub center_lon: f64,
    pub center_lat: f64,
    /// Eye altitude above the z=0 plane, in metres.
    pub height: f64,
    pub fov_y_deg: f64,
    pub near: f32,
    pub far: f32,
}

impl Default for PlaneCamera {
    fn default() -> Self {
        Self {
            center_lon: 120.0,
            center_lat: 30.0,
            height: 9_000_000.0,
            fov_y_deg: 30.0,
            near: 500.0,
            far: 200_000_000.0,
        }
    }
}

impl PlaneCamera {
    pub fn clamp_center(&mut self) {
        self.center_lon = self.center_lon.clamp(-180.0, 180.0);
        self.center_lat = self.center_lat.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    }

    pub fn zoom_wheel(&mut self, line_delta: f32) {
        // Scroll up -> smaller altitude -> closer view.
        let factor = (-line_delta * 0.16).exp() as f64;
        self.height = (self.height * factor).clamp(20_000.0, 80_000_000.0);
    }

    /// Pan in screen pixels: dragging the map right moves the camera west.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64, viewport_height: f64) {
        let mpp = self.meters_per_pixel(viewport_height);
        let dlon = dx * mpp / (EARTH_RADIUS * DEG_TO_RAD);
        let dlat = dy * mpp / (EARTH_RADIUS * DEG_TO_RAD);
        self.center_lon -= dlon;
        self.center_lat += dlat;
        self.clamp_center();
    }

    pub fn meters_per_pixel(&self, viewport_height: f64) -> f64 {
        2.0 * self.height * (self.fov_y_deg * DEG_TO_RAD * 0.5).tan() / viewport_height.max(1.0)
    }

    pub fn half_extent(&self, aspect: f64) -> (f64, f64) {
        let half_y = self.height * (self.fov_y_deg * DEG_TO_RAD * 0.5).tan();
        (half_y * aspect.max(0.01), half_y)
    }

    /// Camera position in the local render space (metres).
    pub fn eye(&self) -> [f32; 3] {
        [0.0, 0.0, self.height as f32]
    }

    pub fn view_proj(&self, aspect: f64) -> Mat4 {
        let eye = Vec3::new(0.0, 0.0, self.height as f32);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(
            (self.fov_y_deg as f32).to_radians(),
            aspect.max(0.01) as f32,
            self.near,
            self.far,
        );
        proj * view
    }

    pub fn target_zoom(&self, viewport_height: f64) -> u32 {
        crate::math::geo::zoom_for_height(self.height, viewport_height, self.fov_y_deg)
    }
}

/// One resolved globe camera: `f64` ECEF eye and view-projection plus the
/// `f32` clip-space matrix the uniform buffer needs.
#[derive(Clone, Debug)]
pub struct GlobeView {
    pub eye: [f64; 3],
    /// Eye position relative to the render origin, `f64` (and `f32` for the
    /// fragment shader's view vector).
    pub eye_relative: [f32; 3],
    pub view_proj: DMat4,
}

impl GlobeView {
    /// `f32` view-projection for the GPU uniform. The `f64` product is rounded
    /// once here; no per-vertex `f32` subtraction of huge ECEF values happens.
    pub fn view_proj_f32(&self) -> Mat4 {
        let cols = self.view_proj.to_cols_array().map(|value| value as f32);
        Mat4::from_cols_array(&cols)
    }

    pub fn eye_vec3(&self) -> DVec3 {
        DVec3::from_array(self.eye)
    }

    /// Frustum planes in the same ECEF space as the mesh vertices.
    pub fn frustum(&self) -> Frustum {
        Frustum::from_view_proj(self.view_proj)
    }
}

#[derive(Clone, Debug)]
pub struct GlobeCamera {
    /// Longitude of the sub-camera point (what the view is centered on).
    pub yaw_deg: f64,
    /// Latitude of the sub-camera point.
    pub pitch_deg: f64,
    /// Eye distance from the Earth centre.
    pub distance: f64,
    pub fov_y_deg: f64,
}

impl Default for GlobeCamera {
    fn default() -> Self {
        Self {
            yaw_deg: 110.0,
            pitch_deg: 25.0,
            distance: EARTH_RADIUS * 3.2,
            fov_y_deg: 42.0,
        }
    }
}

impl GlobeCamera {
    pub fn from_plane(plane: &PlaneCamera) -> Self {
        Self {
            yaw_deg: plane.center_lon,
            pitch_deg: plane.center_lat.clamp(-80.0, 80.0),
            distance: (EARTH_RADIUS + plane.height * 1.5)
                .clamp(EARTH_RADIUS + 20_000.0, EARTH_RADIUS * 6.5),
            ..Default::default()
        }
    }

    pub fn apply_to_plane(&self, plane: &mut PlaneCamera) {
        plane.center_lon = self.yaw_deg;
        plane.center_lat = self.pitch_deg;
        plane.height = (self.distance - EARTH_RADIUS).clamp(40_000.0, 60_000_000.0);
        plane.clamp_center();
    }

    pub fn orbit_pixels(&mut self, dx: f64, dy: f64) {
        self.yaw_deg = (self.yaw_deg - dx * 0.16).rem_euclid(360.0);
        if self.yaw_deg > 180.0 {
            self.yaw_deg -= 360.0;
        }
        self.pitch_deg = (self.pitch_deg + dy * 0.16).clamp(-85.0, 85.0);
    }

    pub fn zoom_wheel(&mut self, line_delta: f32) {
        let factor = (-line_delta * 0.12).exp() as f64;
        self.distance =
            (self.distance * factor).clamp(EARTH_RADIUS + GLOBE_MIN_ALTITUDE, GLOBE_MAX_DISTANCE);
    }

    /// Eye altitude above the ellipsoid, in metres.
    pub fn altitude(&self) -> f64 {
        (self.distance - EARTH_RADIUS).max(0.0)
    }

    /// Eye position in ECEF metres: the geodetic surface point under the camera
    /// plus `altitude` along its geodetic normal. The orbit radius is therefore
    /// the `distance` field at the equator and ~21 km smaller over the poles.
    pub fn eye_ecef(&self, ellipsoid: Ellipsoid) -> [f64; 3] {
        let surface = ellipsoid.surface(self.yaw_deg, self.pitch_deg);
        let normal = ellipsoid.normal(self.yaw_deg, self.pitch_deg);
        let altitude = self.altitude();
        [
            surface[0] + normal[0] * altitude,
            surface[1] + normal[1] * altitude,
            surface[2] + normal[2] * altitude,
        ]
    }

    /// Near/far planes for the globe projection.
    ///
    /// Depth choice (recorded per the stage-2 spec): a standard `[0,1]`
    /// projection with `Less` against a `Depth32Float` target. Reverse-Z was
    /// evaluated and is not needed here: the ellipsoid mesh is a single shell
    /// with no coplanar surfaces, and the only geometry that overlaps in depth
    /// is the skirt strip, which is deliberately behind the surface it hides.
    /// With `near = altitude * 0.08` the smallest depth step at the surface is
    /// still `near / altitude²` ≈ 4e-5 per metre at the 2 km floor, far above
    /// `f32` epsilon, so the near ground does not z-fight. Keeping the standard
    /// projection also leaves the 2D tile/icon passes untouched.
    pub fn near_far(&self, ellipsoid: Ellipsoid) -> (f32, f32) {
        let altitude = self.altitude();
        let near = (altitude * 0.08).clamp(20.0, 2_000_000.0) as f32;
        let far = (self.distance + ellipsoid.max_radius() * 2.0) as f32;
        (near, far)
    }

    /// Resolve the camera for one frame: `f64` ECEF eye, view-projection, and
    /// the `f32` values the uniform needs.
    pub fn view(&self, aspect: f64, ellipsoid: Ellipsoid) -> GlobeView {
        let eye = self.eye_ecef(ellipsoid);
        let eye_vec = DVec3::from_array(eye);
        // Screen-up is the local north tangent, not the radial direction: at
        // `pitch_deg == 0` the radial is parallel to the view direction and
        // `look_at` would become degenerate (NaN). North is tangent to the
        // ellipsoid and perpendicular to eye→center for every latitude in the
        // supported ±85° pitch range.
        let lat = self.pitch_deg.to_radians();
        let lon = self.yaw_deg.to_radians();
        let up_hint = DVec3::new(
            -lat.sin() * lon.cos(),
            -lat.sin() * lon.sin(),
            lat.cos(),
        );
        let dir = (DVec3::ZERO - eye_vec).normalize_or_zero();
        let up = (up_hint - dir * up_hint.dot(dir)).normalize_or_zero();
        let up = if up.length_squared() > 0.5 {
            up
        } else {
            // Polar fallback: use the local east tangent.
            let east = DVec3::new(-lon.sin(), lon.cos(), 0.0);
            (east - dir * east.dot(dir)).normalize_or_zero()
        };
        let (near, far) = self.near_far(ellipsoid);
        let view = DMat4::look_at_rh(eye_vec, DVec3::ZERO, up);
        let proj = DMat4::perspective_rh(
            (self.fov_y_deg * DEG_TO_RAD) as f64,
            aspect.max(0.01),
            near as f64,
            far as f64,
        );
        GlobeView {
            eye,
            eye_relative: [eye[0] as f32, eye[1] as f32, eye[2] as f32],
            view_proj: proj * view,
        }
    }
}

/// Six normalized clip-space planes (`n·p + d >= 0` is inside).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [DVec4; 6],
}

impl Frustum {
    /// Gribb/Hartmann plane extraction for a wgpu-style projection
    /// (`z` in `0..1` after the perspective divide).
    pub fn from_view_proj(m: DMat4) -> Self {
        let row = |index: usize| {
            let r = DVec4::new(
                m.x_axis[index],
                m.y_axis[index],
                m.z_axis[index],
                m.w_axis[index],
            );
            r
        };
        let r0 = row(0);
        let r1 = row(1);
        let r2 = row(2);
        let r3 = row(3);
        let raw = [
            r3 + r0, // left
            r3 - r0, // right
            r3 + r1, // bottom
            r3 - r1, // top
            r2,      // near (clip z >= 0)
            r3 - r2, // far (clip z <= w)
        ];
        let mut planes = [DVec4::ZERO; 6];
        for (index, plane) in raw.iter().enumerate() {
            let normal = DVec3::new(plane.x, plane.y, plane.z);
            let length = normal.length().max(1e-30);
            planes[index] = DVec4::new(
                plane.x / length,
                plane.y / length,
                plane.z / length,
                plane.w / length,
            );
        }
        Self { planes }
    }

    /// Conservative sphere test: false only when the sphere is fully outside a
    /// plane.
    pub fn intersects_sphere(&self, center: DVec3, radius: f64) -> bool {
        for plane in &self.planes {
            let normal = DVec3::new(plane.x, plane.y, plane.z);
            if normal.dot(center) + plane.w < -radius {
                return false;
            }
        }
        true
    }
}

/// Horizon (Earth-occlusion) test for the eye. A bounding sphere is rejected
/// only when it lies entirely beyond the tangent plane through the eye, using
/// the *polar* radius so the test never culls something that is visible.
#[derive(Clone, Copy, Debug)]
pub struct HorizonOccluder {
    eye: DVec3,
    eye_length: f64,
    radius_sq: f64,
}

impl HorizonOccluder {
    pub fn new(eye: DVec3, ellipsoid: Ellipsoid) -> Self {
        let radius = ellipsoid.min_radius();
        Self {
            eye,
            eye_length: eye.length().max(1.0),
            radius_sq: radius * radius,
        }
    }

    /// Signed distance from the sphere centre to the horizon plane is
    /// `(C·E - r²) / |E|`; the sphere is hidden when it is farther than its own
    /// radius on the far side.
    pub fn sphere_visible(&self, center: DVec3, radius: f64) -> bool {
        (center.dot(self.eye) - self.radius_sq) / self.eye_length >= -radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::geo::TileId;

    #[test]
    fn plane_half_extent_is_proportional_to_height() {
        let a = PlaneCamera::default();
        let mut b = a.clone();
        b.height *= 2.0;
        let (ax, ay) = a.half_extent(16.0 / 9.0);
        let (bx, by) = b.half_extent(16.0 / 9.0);
        assert!((bx / ax - 2.0).abs() < 1e-9);
        assert!((by / ay - 2.0).abs() < 1e-9);
    }

    #[test]
    fn globe_plane_roundtrip_keeps_center() {
        let mut plane = PlaneCamera::default();
        plane.center_lon = 12.0;
        plane.center_lat = -33.0;
        let globe = GlobeCamera::from_plane(&plane);
        globe.apply_to_plane(&mut plane);
        assert!((plane.center_lon - 12.0).abs() < 1e-9);
        assert!((plane.center_lat + 33.0).abs() < 1e-9);
    }

    #[test]
    fn globe_eye_is_above_the_surface() {
        let camera = GlobeCamera::default();
        let ellipsoid = Ellipsoid::WGS84;
        let eye = camera.eye_ecef(ellipsoid);
        let radius = (eye[0] * eye[0] + eye[1] * eye[1] + eye[2] * eye[2]).sqrt();
        assert!((radius - camera.distance).abs() < 30_000.0);
        assert!(radius > ellipsoid.min_radius());
    }

    #[test]
    fn frustum_keeps_the_globe_and_rejects_the_far_side() {
        let camera = GlobeCamera::default();
        let ellipsoid = Ellipsoid::WGS84;
        let view = camera.view(16.0 / 9.0, ellipsoid);
        let frustum = view.frustum();
        assert!(frustum.intersects_sphere(DVec3::ZERO, ellipsoid.min_radius() * 0.5));
        // The sub-camera surface patch is visible.
        let center = ellipsoid.surface(camera.yaw_deg, camera.pitch_deg);
        assert!(frustum.intersects_sphere(DVec3::from_array(center), 200_000.0));
        // A point far above the camera is outside the view frustum.
        let eye = view.eye_vec3();
        let above = eye + view.eye_vec3().normalize() * 10_000_000.0;
        assert!(!frustum.intersects_sphere(above, 10_000.0));
    }

    #[test]
    fn horizon_occluder_hides_the_far_side() {
        let camera = GlobeCamera::default();
        let ellipsoid = Ellipsoid::WGS84;
        let eye = DVec3::from_array(camera.eye_ecef(ellipsoid));
        let occluder = HorizonOccluder::new(eye, ellipsoid);
        let near = ellipsoid.surface(camera.yaw_deg, camera.pitch_deg);
        assert!(occluder.sphere_visible(DVec3::from_array(near), 100_000.0));
        let far = ellipsoid.surface(camera.yaw_deg + 180.0, -camera.pitch_deg);
        assert!(!occluder.sphere_visible(DVec3::from_array(far), 100_000.0));
        // The same result for a tile right at the antipode of the camera.
        let id = TileId::new(2, 0, 2);
        let b = crate::math::geo::tile_bounds(id);
        let c = ellipsoid.surface(b.center().0, b.center().1);
        assert!(!occluder.sphere_visible(DVec3::from_array(c), 300_000.0));
    }

    #[test]
    fn globe_view_at_the_equator_and_poles_is_finite() {
        // `pitch_deg == 0` used to put the radial up vector parallel to the view
        // direction, producing a NaN `look_at`. The north-tangent up fixes it.
        for pitch in [0.0, 0.5, -0.5, 85.0, -85.0] {
            let camera = GlobeCamera {
                yaw_deg: 0.0,
                pitch_deg: pitch,
                distance: EARTH_RADIUS * 3.0,
                fov_y_deg: 42.0,
            };
            let view = camera.view(16.0 / 9.0, Ellipsoid::WGS84);
            assert!(
                view.view_proj
                    .to_cols_array()
                    .iter()
                    .all(|value| value.is_finite()),
                "view matrix is not finite at pitch {pitch}"
            );
            assert!(view.eye.iter().all(|value| value.is_finite()));
        }
    }
}
