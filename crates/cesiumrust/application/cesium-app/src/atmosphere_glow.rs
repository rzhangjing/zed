//! 大气光晕 —— 地球周围嵌套的半透明蓝色壳层。
//!
//! 模拟从太空可见的大气边缘辉光。每个壳层只渲染 BACK-FACES（背面）
//! 并采用叠加混合，因此它仅以一圈绕边缘的光晕出现。八个
//! 具有递减 alpha 的壳层产生柔和的渐变回落，而非单一硬边。
//!
//! 背面技巧仅在 camera 停留在壳层外部时有效。一旦缩放使 camera
//! 进入壳层内部，远半球会包围视图并在屏幕上刷出大片平蓝区域。因此
//! 壳层随 camera 距离逐淡，并在靠近地表时完全消失（那里本就
//! 看不到边缘光晕）。

use bevy::prelude::*;

use crate::orbit_camera::OrbitState;

/// spawn 大气光晕壳层的插件。
pub struct AtmosphereGlowPlugin;

impl Plugin for AtmosphereGlowPlugin {
    /// 插件装配入口：启动时生成大气壳层，每帧按距离渐淡。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_atmosphere)
            .add_systems(Update, fade_atmosphere_with_distance);
    }
}

/// 每个壳层的基础叠加 alpha，在 camera 远离时恢复。
#[derive(Component)]
struct AtmosphereShell {
    /// 该壳层在未渐淡时的基准叠加 alpha。
    base_alpha: f32,
}

/// 辉光在太空中完全可见（distance >= 3 R），并随 camera 靠近地表
/// （<= 1.5 R）渐淡至零，因此放大时绝不会看到蓝色壳层内部。
fn glow_fade(distance: f32) -> f32 {
    let t = ((distance - 1.5) / 1.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 生成多层向外阶梯的半透明球壳，构成大气边缘的蓝色辉光。
fn setup_atmosphere(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // 地球渲染为单位球（scale = 1/METERS_PER_RENDER_UNIT 应用于一个
    // 半径 ~6378137 m 的 mesh）。地球在渲染单位中的有效半径
    // ≈ 1.0。壳层向外阶梯，alpha 逐半，形成柔和的边缘回落。
    let mesh = meshes.add(
        Sphere::new(1.0)
            .mesh()
            .ico(6)
            .expect("ico subdivision failed"),
    );

    // (scale, 叠加 alpha)：最内层最亮，最外层最暗。八个壳层保持每层
    // alpha 步长足够小，使黑色太空背景下察觉不到环带或硬外轮廓。这叠
    // 结构是一个紧贴边缘的薄而苍白的 rim（约占半径 4%），与参考的
    // CesiumJS 外观一致，而非宽阔饱和的光晕。
    let shells: [(f32, f32); 8] = [
        (1.004, 0.12),
        (1.008, 0.085),
        (1.013, 0.058),
        (1.018, 0.040),
        (1.023, 0.026),
        (1.028, 0.016),
        (1.034, 0.009),
        (1.040, 0.004),
    ];

    for (scale, alpha) in shells {
        commands.spawn((
            AtmosphereShell { base_alpha: alpha },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgba(0.40, 0.68, 1.0, alpha),
                unlit: true,
                alpha_mode: AlphaMode::Add,
                // 只渲染背面：壳层的远半球会以一圈光晕出现在
                // 地球轮廓稍外侧。
                cull_mode: Some(bevy::render::render_resource::Face::Front),
                ..default()
            })),
            Transform::from_scale(Vec3::splat(scale)),
        ));
    }
}

/// 按 camera 距离渐淡因子缩放每个壳层的叠加 alpha，使光晕在 camera
/// 俯入壳层内部后绝不会刷出蓝色区域。
fn fade_atmosphere_with_distance(
    orbit: Res<OrbitState>,
    shells: Query<(&AtmosphereShell, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let fade = glow_fade(orbit.distance);
    for (shell, mat_handle) in &shells {
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = Color::srgba(0.40, 0.68, 1.0, shell.base_alpha * fade);
        }
    }
}
