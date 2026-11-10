use bevy::prelude::*;
use cesium_camera::Frustum;
use cesium_scene_mode::SceneMode;

use crate::camera::components::{ActiveMorph, CesiumCamera};

/// 场景模式切换系统：键盘快捷键与渐变动画。
///
/// 按 `2` 切 2D，`3` 切 3D，`C` 切 Columbus View。
/// 渐变在各模式间平滑动画。
///
/// # 参数
/// - `cameras`：Cesium 相机组件（可写）
/// - `morph`：当前渐变状态（可写）
/// - `keys`：按键输入状态
/// - `time`：帧时钟（提供 dt 推进渐变）
pub fn scene_mode_system(
    mut cameras: Query<&mut CesiumCamera>,
    mut morph: ResMut<ActiveMorph>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    // 本帧时长（f32→f64），用于推进渐变。
    let dt = time.delta_secs() as f64;

    // --- 检测模式切换键 ---
    let target_mode = if keys.just_pressed(KeyCode::Digit2) {
        Some(SceneMode::Scene2D)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(SceneMode::Scene3D)
    } else if keys.just_pressed(KeyCode::KeyC) {
        Some(SceneMode::ColumbusView)
    } else {
        None
    };

    // 若按下切换键且当前非渐变中，则启动 2 秒渐变到新目标。
    if let Some(target) = target_mode {
        for cesium_cam in cameras.iter() {
            let current = cesium_cam.scene_mode;
            if current != target && current != SceneMode::Morphing {
                morph.state.start_morph(current, target, 2.0);
            }
        }
    }

    // --- 推进渐变 ---
    // 推进渐变，并检测本帧是否刚好完成（active 由真变假）。
    let was_active = morph.state.active;
    morph.state.update(dt);
    let just_finished = was_active && !morph.state.active;

    for mut cesium_cam in cameras.iter_mut() {
        if morph.state.active {
            // 渐变中：相机置为 Morphing，并按进度插值投影。
            cesium_cam.scene_mode = SceneMode::Morphing;
            cesium_cam.camera.mode = cesium_camera::SceneMode::Morphing;

            let t = morph.state.progress;
            let from_2d_or_cv = matches!(morph.state.from, SceneMode::Scene2D | SceneMode::ColumbusView);
            let to_2d_or_cv = matches!(morph.state.to, SceneMode::Scene2D | SceneMode::ColumbusView);

            // 根据起/终点是否为平面模式，选择 2D→3D 或 3D→2D 插值方向。
            if from_2d_or_cv && morph.state.to == SceneMode::Scene3D {
                update_projection_for_morph(&mut cesium_cam, t, true);
            } else if morph.state.from == SceneMode::Scene3D && to_2d_or_cv {
                update_projection_for_morph(&mut cesium_cam, t, false);
            }
        } else if just_finished {
            // 刚完成：锁定到目标模式并应用其最终投影。
            let target = morph.state.to;
            cesium_cam.scene_mode = target;
            apply_mode_projection(&mut cesium_cam, target);
        }
    }
}

/// 按目标模式为相机设置固定投影（透视/正交）。
///
/// # 参数
/// - `cesium_cam`：待修改的相机
/// - `mode`：目标场景模式
fn apply_mode_projection(cesium_cam: &mut CesiumCamera, mode: SceneMode) {
    let cam = &mut cesium_cam.camera;
    // 逐模式分派：3D/CV 用透视，2D 用正交（覆盖全地球周长）。
    match mode {
        SceneMode::Scene3D => {
            cam.mode = cesium_camera::SceneMode::Scene3D;
            cam.frustum = Frustum::Perspective(cesium_geospatial::PerspectiveFrustum::new(
                std::f64::consts::FRAC_PI_3,
                16.0 / 9.0,
                1.0,
                500_000_000.0,
            ));
        }
        SceneMode::Scene2D => {
            cam.mode = cesium_camera::SceneMode::Scene2D;
            cam.frustum = Frustum::Orthographic(cesium_geospatial::OrthographicFrustum::new(
                6378137.0 * std::f64::consts::TAU,
                16.0 / 9.0,
                1.0,
                500_000_000.0,
            ));
        }
        SceneMode::ColumbusView => {
            cam.mode = cesium_camera::SceneMode::ColumbusView;
            cam.frustum = Frustum::Perspective(cesium_geospatial::PerspectiveFrustum::new(
                std::f64::consts::FRAC_PI_3,
                16.0 / 9.0,
                1.0,
                500_000_000.0,
            ));
        }
        SceneMode::Morphing => {
            cam.mode = cesium_camera::SceneMode::Morphing;
        }
    }
}

/// 在渐变期间按进度 t 插值相机 FOV（在 3D 透视与近似正交间平滑过渡）。
///
/// # 参数
/// - `cesium_cam`：待修改的相机
/// - `t`：渐变进度 [0,1]
/// - `from_2d_to_3d`：方向为真表示 2D→3D，否则 3D→2D
fn update_projection_for_morph(cesium_cam: &mut CesiumCamera, t: f64, from_2d_to_3d: bool) {
    // 目标 3D 透视视场角。
    let perspective_fov = std::f64::consts::FRAC_PI_3;

    // 2D→3D：从近似零 FOV 渐入到全透视 FOV；反之那么渐出。
    if from_2d_to_3d {
        let fov = perspective_fov * t + (1.0 - t) * 0.01;
        cesium_cam.camera.frustum = Frustum::Perspective(
            cesium_geospatial::PerspectiveFrustum::new(fov, 16.0 / 9.0, 1.0, 500_000_000.0),
        );
    } else {
        let fov = perspective_fov * (1.0 - t) + t * 0.01;
        cesium_cam.camera.frustum = Frustum::Perspective(
            cesium_geospatial::PerspectiveFrustum::new(fov, 16.0 / 9.0, 1.0, 500_000_000.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_scene_mode::SceneMode;

    #[test]
    /// 验证 3D 模式应用透视锥台。
    fn test_apply_mode_projection_3d() {
        let mut cc = CesiumCamera::default();
        apply_mode_projection(&mut cc, SceneMode::Scene3D);
        assert_eq!(cc.camera.mode, cesium_camera::SceneMode::Scene3D);
        assert!(matches!(cc.camera.frustum, Frustum::Perspective(_)));
    }

    #[test]
    /// 验证 2D 模式应用正交锥台。
    fn test_apply_mode_projection_2d() {
        let mut cc = CesiumCamera::default();
        apply_mode_projection(&mut cc, SceneMode::Scene2D);
        assert_eq!(cc.camera.mode, cesium_camera::SceneMode::Scene2D);
        assert!(matches!(cc.camera.frustum, Frustum::Orthographic(_)));
    }

    #[test]
    /// 验证默认渐变状态未激活且进度为 1。
    fn test_morph_state_default_inactive() {
        let morph = ActiveMorph::default();
        assert!(!morph.state.active);
        assert!((morph.state.progress - 1.0).abs() < 1e-10);
    }
}
