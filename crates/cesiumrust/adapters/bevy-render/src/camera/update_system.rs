//! 相机更新系统：将领域相机状态同步到 Bevy 的 Transform 与 Projection。

use bevy::prelude::*;
use bevy::render::camera::Projection;

use crate::camera::components::CesiumCamera;
use crate::METERS_PER_RENDER_UNIT;

/// 逐帧相机更新：读取领域 Camera 并将其
/// view/projection 状态应用到 Bevy 的 Transform 和 Projection 组件。
///
/// # 参数
/// - `cameras`：携带领域相机、变换与投影的查询
pub fn camera_update_system(
    mut cameras: Query<(&CesiumCamera, &mut Transform, &mut Projection)>,
) {
    for (cesium_cam, mut transform, mut projection) in cameras.iter_mut() {
        let cam = &cesium_cam.camera;

        // 世界以米为单位，渲染空间以 render unit（1 = 地球半径）表示，故需缩放。
        let scale = 1.0 / METERS_PER_RENDER_UNIT;
        let position_f32 = cam.position.as_vec3() * scale as f32;

        // Bevy 以 -Z 为前方，故相机朝向取方向的反向，与 up 一同确定姿态。
        let forward = -cam.direction.as_vec3();
        let up = cam.up.as_vec3();

        *transform = Transform::from_translation(position_f32)
            .looking_to(forward, up);

        // 按透视/正交两种投影分支，near/far 同样换算到 render unit。
        match &cam.frustum {
            cesium_camera::Frustum::Perspective(f) => {
                // 透视：直接搬运 fov 与宽高比，深度范围乘缩放。
                *projection = Projection::Perspective(PerspectiveProjection {
                    fov: f.fov as f32,
                    aspect_ratio: f.aspect_ratio as f32,
                    near: f.near as f32 * scale as f32,
                    far: f.far as f32 * scale as f32,
                });
            }
            cesium_camera::Frustum::Orthographic(f) => {
                // 正交：以半宽/半高构造视锥矩形区域。
                let hw = (f.width * 0.5 * scale) as f32;
                let hh = (f.height() * 0.5 * scale) as f32;
                *projection = Projection::Orthographic(OrthographicProjection {
                    near: f.near as f32 * scale as f32,
                    far: f.far as f32 * scale as f32,
                    scale: 1.0,
                    area: Rect::new(-hw, -hh, hw, hh),
                    ..OrthographicProjection::default_3d()
                });
            }
        }
    }
}
