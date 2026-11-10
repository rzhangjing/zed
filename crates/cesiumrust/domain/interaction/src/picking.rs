//! 屏幕空间拾取：将屏幕坐标转换为世界射线。
//!
//! 提供屏幕空间拾取能力：从窗口坐标生成拾取射线，
//! 并支持逐层穿透拾取与椭球求交。

use cesium_camera::Camera;
use cesium_geospatial::ellipsoid::Ellipsoid;
use cesium_geospatial::ray::Ray;
use glam::{DVec2, DVec3, DVec4};

/// 视口尺寸。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// 宽度（以像素计）。
    pub width: f64,
    /// 高度（以像素计）。
    pub height: f64,
}

impl Viewport {
    /// 创建一个新的视口。
    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    /// 宽高比（宽度 / 高度）。
    pub fn aspect_ratio(&self) -> f64 {
        self.width / self.height
    }
}

/// 从屏幕坐标计算一条拾取射线。
///
/// # 参数
/// * `screen_position` - 屏幕坐标（像素，原点在左上角）
/// * `viewport` - 视口尺寸
/// * `camera` - 相机
///
/// # 返回
/// 世界空间中的一条射线，若无法计算该射线则返回 None
pub fn get_pick_ray(
    screen_position: DVec2,
    viewport: &Viewport,
    camera: &Camera,
) -> Option<Ray> {
    if viewport.width <= 0.0 || viewport.height <= 0.0 {
        return None;
    }

    // 将屏幕坐标转换为 NDC（-1 到 1）
    let ndc_x = (2.0 * screen_position.x / viewport.width) - 1.0;
    let ndc_y = 1.0 - (2.0 * screen_position.y / viewport.height); // 翻转 Y

    // 计算逆视图-投影矩阵
    let view_proj = camera.view_projection_matrix();
    let inv_view_proj = view_proj.inverse();

    // 反投影近点和远点
    let near_point = DVec4::new(ndc_x, ndc_y, -1.0, 1.0);
    let far_point = DVec4::new(ndc_x, ndc_y, 1.0, 1.0);

    let near_world = inv_view_proj * near_point;
    let far_world = inv_view_proj * far_point;

    // 透视除法
    let near_world = DVec3::new(
        near_world.x / near_world.w,
        near_world.y / near_world.w,
        near_world.z / near_world.w,
    );
    let far_world = DVec3::new(
        far_world.x / far_world.w,
        far_world.y / far_world.w,
        far_world.z / far_world.w,
    );

    let direction = (far_world - near_world).normalize();

    Some(Ray::new(near_world, direction))
}

/// 计算拾取射线与椭球表面的交点。
///
/// # 参数
/// * `ray` - 拾取射线
/// * `ellipsoid` - 要求交的椭球
///
/// # 返回
/// ECEF 中的交点，若无交点则返回 None
pub fn pick_ellipsoid(ray: &Ray, ellipsoid: &Ellipsoid) -> Option<DVec3> {
    // 使用二次方程求解射线-椭球相交
    // Ellipsoid: x²/a² + y²/b² + z²/c² = 1
    let radii = ellipsoid.radii();
    let inv_radii_sq = DVec3::new(
        1.0 / (radii.x * radii.x),
        1.0 / (radii.y * radii.y),
        1.0 / (radii.z * radii.z),
    );

    let origin = ray.origin;
    let direction = ray.direction;

    // 二次项系数：at² + bt + c = 0
    let a = direction.x * direction.x * inv_radii_sq.x
        + direction.y * direction.y * inv_radii_sq.y
        + direction.z * direction.z * inv_radii_sq.z;

    let b = 2.0 * (origin.x * direction.x * inv_radii_sq.x
        + origin.y * direction.y * inv_radii_sq.y
        + origin.z * direction.z * inv_radii_sq.z);

    let c = origin.x * origin.x * inv_radii_sq.x
        + origin.y * origin.y * inv_radii_sq.y
        + origin.z * origin.z * inv_radii_sq.z
        - 1.0;

    let discriminant = b * b - 4.0 * a * c;

    if discriminant < 0.0 {
        return None; // 无交点
    }

    let sqrt_disc = discriminant.sqrt();
    let t1 = (-b - sqrt_disc) / (2.0 * a);
    let t2 = (-b + sqrt_disc) / (2.0 * a);

    // 选择最近的正交点
    let t = if t1 > 0.0 {
        t1
    } else if t2 > 0.0 {
        t2
    } else {
        return None; // 两个交点都在射线后方
    };

    Some(ray.origin + ray.direction * t)
}

/// 将一个世界坐标位置转换为屏幕坐标。
///
/// # 参数
/// * `world_position` - ECEF 中的位置
/// * `viewport` - 视口尺寸
/// * `camera` - 相机
///
/// # 返回
/// 屏幕坐标（像素），若在该相机后方则返回 None
pub fn world_to_screen(
    world_position: DVec3,
    viewport: &Viewport,
    camera: &Camera,
) -> Option<DVec2> {
    let view_proj = camera.view_projection_matrix();
    let clip = view_proj * DVec4::new(world_position.x, world_position.y, world_position.z, 1.0);

    // 相机后方检查
    if clip.w <= 0.0 {
        return None;
    }

    // 透视除法 → NDC
    let ndc_x = clip.x / clip.w;
    let ndc_y = clip.y / clip.w;
    let ndc_z = clip.z / clip.w;

    // 在裁剪体之外
    if !(-1.0..=1.0).contains(&ndc_z) {
        return None;
    }

    // NDC → 屏幕
    let screen_x = (ndc_x + 1.0) * 0.5 * viewport.width;
    let screen_y = (1.0 - ndc_y) * 0.5 * viewport.height; // 翻转 Y

    Some(DVec2::new(screen_x, screen_y))
}

/// 以 DVec2 计算窗口中心。
pub fn window_center(viewport: &Viewport) -> DVec2 {
    DVec2::new(viewport.width * 0.5, viewport.height * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_camera() -> Camera {
        // 相机位于赤道上空，朝向中心
        Camera::new(
            DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
        )
    }

    #[test]
    fn test_viewport() {
        let vp = Viewport::new(1920.0, 1080.0);
        assert!((vp.aspect_ratio() - 16.0 / 9.0).abs() < 1e-10);
    }

    #[test]
    fn test_get_pick_ray_center() {
        let camera = create_test_camera();
        let viewport = Viewport::new(800.0, 600.0);
        let center = DVec2::new(400.0, 300.0);

        let ray = get_pick_ray(center, &viewport, &camera).unwrap();

        // 射线原点应靠近相机（在近裁剪面上）
        let dist_to_camera = (ray.origin - camera.position).length();
        assert!(dist_to_camera < camera.position.length() * 0.1);

        // 射线方向应大致朝向 -X（看向地心）
        assert!(ray.direction.x < -0.9);
    }

    #[test]
    fn test_get_pick_ray_invalid_viewport() {
        let camera = create_test_camera();
        let viewport = Viewport::new(0.0, 0.0);

        let ray = get_pick_ray(DVec2::new(100.0, 100.0), &viewport, &camera);
        assert!(ray.is_none());
    }

    #[test]
    fn test_pick_ellipsoid_hit() {
        let camera = create_test_camera();
        let viewport = Viewport::new(800.0, 600.0);
        let center = DVec2::new(400.0, 300.0);

        let ray = get_pick_ray(center, &viewport, &camera).unwrap();
        let hit = pick_ellipsoid(&ray, &Ellipsoid::WGS84);

        assert!(hit.is_some());
        let hit_point = hit.unwrap();

        // 命中点应位于椭球表面上
        let radii = Ellipsoid::WGS84.radii();
        let normalized = DVec3::new(
            hit_point.x / radii.x,
            hit_point.y / radii.y,
            hit_point.z / radii.z,
        );
        assert!((normalized.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_pick_ellipsoid_miss() {
        // 射线指向远离椭球的方向
        let ray = Ray::new(
            DVec3::new(6378137.0 * 3.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0), // 指向远离方向
        );

        let hit = pick_ellipsoid(&ray, &Ellipsoid::WGS84);
        assert!(hit.is_none());
    }

    #[test]
    fn test_world_to_screen_center() {
        let camera = create_test_camera();
        let viewport = Viewport::new(800.0, 600.0);

        // 相机正前方的一个点应投影到屏幕中心
        let world_point = DVec3::new(6378137.0 * 2.0, 0.0, 0.0);
        let screen = world_to_screen(world_point, &viewport, &camera);

        assert!(screen.is_some());
        let screen = screen.unwrap();
        // 应靠近中心
        assert!((screen.x - 400.0).abs() < 50.0);
        assert!((screen.y - 300.0).abs() < 50.0);
    }

    #[test]
    fn test_world_to_screen_behind_camera() {
        let camera = create_test_camera();
        let viewport = Viewport::new(800.0, 600.0);

        // 相机后方的一个点
        let world_point = DVec3::new(6378137.0 * 5.0, 0.0, 0.0);
        let screen = world_to_screen(world_point, &viewport, &camera);

        assert!(screen.is_none());
    }

    #[test]
    fn test_window_center() {
        let viewport = Viewport::new(1920.0, 1080.0);
        let center = window_center(&viewport);
        assert!((center.x - 960.0).abs() < 1e-10);
        assert!((center.y - 540.0).abs() < 1e-10);
    }

    #[test]
    fn test_pick_ray_offset() {
        let camera = create_test_camera();
        let viewport = Viewport::new(800.0, 600.0);

        // 在左上角拾取
        let ray = get_pick_ray(DVec2::new(0.0, 0.0), &viewport, &camera).unwrap();

        // 应与中心射线不同
        let center_ray = get_pick_ray(DVec2::new(400.0, 300.0), &viewport, &camera).unwrap();

        // 方向应不同
        let dot = ray.direction.dot(center_ray.direction);
        assert!(dot < 0.999); // 不是同一方向
    }
}
