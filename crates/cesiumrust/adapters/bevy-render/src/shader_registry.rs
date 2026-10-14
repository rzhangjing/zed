//! 无头安好的 shader / 内部资产注册助手。
//!
//! Bevy 的 [`load_internal_asset!`](bevy::asset::load_internal_asset) 宏与
//! [`MaterialPlugin`](bevy::pbr::MaterialPlugin) 都引用资产后端
//! 资源（`Assets<Shader>` / `AssetServer`），而这些资源在一个
//! 无头 `MinimalPlugins` app（无 `AssetPlugin`、无 `RenderPlugin`）下**缺失**。在其上调用
//! 会 panic —— 这就是在
//! `docs/deviations.md#dev-005` 与 `docs/deferred.md#6` 中跟踪的根因。
//!
//! 本模块将注册包装在可用性守卫中，因此插件在无头/CI 环境下降级为
//! 空操作（而非 panic），而当资产后端存在时行为与原始
//! Bevy 调用**完全一致**（常规 GPU app 路径 —— 同一
//! `include_str!` 嵌入的源插入到同一个 `AssetId`，因此像素中性）。
//!
//! 被本适配器中每个嵌入的 WGSL 复用。当前仅剩两个消费者：
//! `shaders/fabric_material.wgsl`（M5.1，`fabric_material.rs:627`）与
//! `src/atmosphere/sky_atmosphere.wgsl`（M5-C，`atmosphere/mod.rs:30`）。
//!
//! 2026-09 范围裁剪（P0）删除了后处理/观感子树后，`fxaa.wgsl`、`ao.wgsl`、
//! `clipping.wgsl`、`ibl.wgsl`、`panorama.wgsl`、`oit_*.wgsl`、`cloud*.wgsl`、
//! `split.wgsl`、`pass_through.wgsl` 已**不再被任何 Rust 代码 `include_str!`**，
//! 并已随 P0/P1 清理一并移出仓库（git 历史可追溯；登记条目已标为「已废止」，
//! 见 `docs/SCOPE_2D3D_IMAGERY_PLOT.md` §3.1.1/§3.2.1）。
//!
//! # 为何是资产后端，而非渲染 app？
//! Bevy 0.15 *已经*守卫了 `MaterialPlugin::build` 与
//! `RenderAssetPlugin::build` 两者的 `RenderApp` 子 app 部分
//! （`if let Some(render_app) = app.get_sub_app_mut(RenderApp) { .. }`），因此
//! 缺失的渲染 app 是一个静默空操作，而非 panic。**唯一**未受守卫的
//! 无头隐患是：
//! 1. `load_internal_asset!` → `World::resource_mut::<Assets<Shader>>()`; and
//! 2. `MaterialPlugin::build` → `init_asset::<M>()` → `World::resource::<AssetServer>()`.
//!
//! 两者都由下方的谓词覆盖。

use bevy::asset::AssetServer;
use bevy::prelude::*;
use bevy::render::render_resource::Shader;

/// Bevy 的资产后端（[`AssetServer`]）是否存在。
///
/// [`AssetPlugin`](bevy::asset::AssetPlugin) 会插入 `AssetServer`；
/// `MinimalPlugins` 不会。`MaterialPlugin::build` 内部调用
/// `init_asset::<M>()`，它引用 `AssetServer` 并在其缺失时 panic
/// —— 因此调用方**必须**基于此谓词门控 `MaterialPlugin` 注册
/// 以保持无头安好。
pub fn asset_backend_available(app: &App) -> bool {
    app.world().contains_resource::<AssetServer>()
}

/// 嵌入 shader 存储（[`Assets<Shader>`]）是否存在。
///
/// 这正是 `load_internal_asset!` 引用的那个资源；基于它守卫
/// 能使 shader 插入无头安好。它由 `init_asset::<Shader>()`
/// （经由渲染栈）插入，因此即使在一个无渲染但启用资产的 app
/// 中存在 `AssetServer`，它也可能缺失。
pub fn shader_assets_available(app: &App) -> bool {
    app.world().contains_resource::<Assets<Shader>>()
}

/// 针对一个嵌入 WGSL shader 的 [`load_internal_asset!`](bevy::asset::load_internal_asset)
/// 无头安好等价物。
///
/// **仅当** shader 存储存在时，将 `Shader::from_wgsl(source, path)` 插入
/// [`Assets<Shader>`] 的 `handle` 下，返回 `Some(handle)`。当其不存在时
/// （无头 `MinimalPlugins`），这是一个返回 `None` 的空操作而非
/// 在缺失资源上 panic。
///
/// 在 GPU 路径上（资产后端存在）这与原始宏逐字节等价：两者
/// 都编译同一个 `include_str!` 嵌入的 `source` 并插入到同一个
/// [`AssetId`]，因此渲染输出不变。
///
/// # 示例（可跨 M5 WGSL 新增复用）
/// ```text
/// crate::shader_registry::try_load_internal_shader(
///     app,
///     MY_SHADER_HANDLE,
///     include_str!("../shaders/my_shader.wgsl"),
///     std::path::Path::new(file!())
///         .parent()
///         .unwrap()
///         .join("../shaders/my_shader.wgsl")
///         .to_string_lossy(),
/// );
/// ```
pub fn try_load_internal_shader(
    app: &mut App,
    handle: Handle<Shader>,
    source: impl Into<std::borrow::Cow<'static, str>>,
    path: impl Into<String>,
) -> Option<Handle<Shader>> {
    if !shader_assets_available(app) {
        return None;
    }
    let shader = Shader::from_wgsl(source, path);
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(handle.id(), shader);
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个 `MinimalPlugins` app 没有 `AssetPlugin`，因此资产后端与
    /// shader 存储都不存在 —— 注册必须降级为一个空操作。
    #[test]
    fn headless_minimal_app_has_no_asset_backend() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        assert!(!asset_backend_available(&app));
        assert!(!shader_assets_available(&app));

        let handle: Handle<Shader> = Handle::weak_from_u128(0xDEAD_BEEF);
        // 绝不能 panic；因为 Assets<Shader> 缺失而返回 None。
        let out = try_load_internal_shader(&mut app, handle, "// empty wgsl", "test.wgsl");
        assert!(out.is_none());
    }
}
