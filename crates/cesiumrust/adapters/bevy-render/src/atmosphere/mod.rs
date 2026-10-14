pub mod celestial_system;
pub mod sky_dome;
pub mod sky_system;

pub use celestial_system::celestial_system;
pub use celestial_system::LightingParams;
pub use sky_dome::{
    sky_dome_gate_enabled, SkyAtmosphereParams, SkyDome, SkyDomeMaterial,
    SKY_ATMOSPHERE_SHADER_HANDLE,
};
pub use sky_system::{sky_dome_setup, sky_system, SkyAtmosphere};

use bevy::prelude::*;

/// 大气子模块聚合：天体光照、天空 dome 与散射系统的插件入口。
pub struct CesiumAtmospherePlugin;

impl Plugin for CesiumAtmospherePlugin {
    /// 注册 dome 着色器与相关资源（无头安全地守卫材质注册），并链式挂载光照/dome/天空系统。
    ///
    /// # 参数
    /// - `app`：Bevy 应用
    fn build(&self, app: &mut App) {
        // 无头安全的内嵌 WGSL 注册。一个裸的 `load_internal_asset!`
        // 会解引用 `Assets<Shader>` 并在 `MinimalPlugins` 下 panic——这一
        // 隐患记录在 docs/deviations.md#dev-005 并由
        // `shader_registry`（M5-A）闭合。`sky_atmosphere.wgsl` 位于本文件旁边
        // （不在 `shaders/` 下），因为它是大气适配层的
        // 私有实现细节，而非可复用的材质库。
        crate::shader_registry::try_load_internal_shader(
            app,
            SKY_ATMOSPHERE_SHADER_HANDLE,
            include_str!("sky_atmosphere.wgsl"),
            std::path::Path::new(file!())
                .parent()
                .unwrap()
                .join("sky_atmosphere.wgsl")
                .to_string_lossy(),
        );

        app.init_resource::<SkyAtmosphere>()
            .init_resource::<LightingParams>()
            // `sky_system` 取 `ResMut<ClearColor>`，因此本插件必须能
            // 独立站立（与 #47 中修复的地形/影像耦合属同一缺陷类）。当资源已
            // 存在时 `init_resource` 是空操作，而 `ClearColor::default()` 是 `Color::BLACK`——正是
            // main.rs L469 所插入的——所以这不会扰动应用自身的值。
            .init_resource::<ClearColor>();

        // `MaterialPlugin::build` 需要*两个* asset-backend 资源，而
        // 只守卫第一个正是把一个 有-asset-但-无-render 的应用
        // 变成一次 panic 的原因：
        //   1. `init_asset::<M>()` 解引用 `AssetServer`——在 `MinimalPlugins`
        //      下缺失（`shader_registry::asset_backend_available`）；以及
        //   2. 它内部添加 `PrepassPipelinePlugin<M>`，其 `build` 调用
        //      `load_internal_asset!` 因而无条件地解引用 `Assets<Shader>`
        //      （bevy_pbr-0.15.3 `prepass/mod.rs` L70）。该
        //      存储由*渲染*栈（而非 `AssetPlugin`）插入，所以
        //      单有 `AssetPlugin` 并不够——这正是 `shader_registry::shader_assets_available`
        //      存在以作守卫的那个资源。
        // 在常规 GPU 路径上两者都成立（`DefaultPlugins` 先运行），所以
        // 材质像以往一样注册。下面的系统取
        // `Option<ResMut<Assets<SkyDomeMaterial>>>`，当任一者不存在时即为空操作。
        let asset_backend = crate::shader_registry::asset_backend_available(app);
        let shader_assets = crate::shader_registry::shader_assets_available(app);
        if asset_backend && shader_assets {
            app.add_plugins(MaterialPlugin::<SkyDomeMaterial>::default());
        } else {
            // 上面的守卫是正确的——没有它 `MaterialPlugin::build` 会在
            // `PrepassPipelinePlugin` 无条件的 `load_internal_asset!` 内
            // panic（DEV-021）——但*静默跳过*对那些在 `DefaultPlugins`
            // 之前挂载本插件的人（specs 集成测试、第三方嵌入）是条死路：
            // dome 根本不出现，也没有任何提示说明原因。除了这条日志之外的
            // 替代结果是一次崩溃而非一片可用的天空，所以说清两个存储中
            // 缺的是哪一个，而不是留给人去猜。
            warn!(
                "[M5-C] SkyDomeMaterial 未注册：缺 AssetServer 或 Assets<Shader>（需 DefaultPlugins 先行）；sky dome 不可见 \
                 (asset_backend_available={asset_backend}, shader_assets_available={shader_assets})",
            );
        }

        // 从本适配层自身对 `CESIUM_ENABLE_SKYDOME` 的读取来播种
        // 运行时的 dome 标志。应用层的
        // `feature_flags::skydome_enabled()` 已经决定了本插件是否被注册
        // （main.rs L513-516）；在此重新读取它，能在插件被直接挂载（测试、
        // 第三方嵌入）而未设置该环境变量时保持资源自洽。
        let dome_enabled = sky_dome_gate_enabled();
        app.world_mut().resource_mut::<SkyAtmosphere>().dome = dome_enabled;

        // 链式排列，使 `celestial_system` 发布的太阳方向在同一帧内被
        // dome 材质拾取，并使 `sky_system` 看到一个已 spawn 的 dome
        // 而非与之竞态。
        app.add_systems(Update, (celestial_system, sky_dome_setup, sky_system).chain());
    }
}
