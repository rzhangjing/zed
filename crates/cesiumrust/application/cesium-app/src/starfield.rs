//! 星野背景 —— 在一个巨大的天球上渲染星星。
//!
//! 使用领域层的 `StarSphere` 内置星表提供亮星，
//! 加上过程化散布的暗星，构成逼真的夜空。
//!
//! 所有恒星都合并到一个 mesh（面向 camera 的 quad）中，使整片天空
//! 只花费一次 draw call —— 每颗恒星 spawn 一个实体会多出
//! ~1500 次 draw call 并拖垮帧率。

use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use cesium_atmosphere::StarSphere;

/// spawn 星野的插件。
pub struct StarfieldPlugin;

impl Plugin for StarfieldPlugin {
    /// 插件装配入口：仅在 Startup 阶段挂载星野生成系统。
    ///
    /// # 参数
    /// - `app`：Bevy 应用。
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_starfield);
    }
}

/// 简单的确定性 hash 伪随机 [0, 1)。
fn hash_rand(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    x ^= x >> 16;
    x = x.wrapping_mul(0x45d9f3b);
    x ^= x >> 16;
    (x & 0x00FF_FFFF) as f32 / 16777216.0
}

/// 追加一个以 `center` 为中心、半尺寸为 `size` 的面向 camera 的 quad
/// （4 顶点 / 2 三角）。quad 平面垂直于径向，因此它总朝向
/// 靠近原点的 camera。
fn push_star_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    center: Vec3,
    size: f32,
) {
    let radial = center.normalize();
    // 垂直于径向的平面内的切标架。
    let up = if radial.dot(Vec3::Z).abs() > 0.95 {
        Vec3::X
    } else {
        Vec3::Z
    };
    let t = radial.cross(up).normalize();
    let b = radial.cross(t).normalize();

    let base = positions.len() as u32;
    let corners = [
        center - t * size - b * size,
        center + t * size - b * size,
        center + t * size + b * size,
        center - t * size + b * size,
    ];
    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (c, u) in corners.iter().zip(uv.iter()) {
        positions.push(c.to_array());
        normals.push(radial.to_array());
        uvs.push(*u);
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// 过程化的 64x64 星点 sprite：明亮的圆形核心带一个向全透明淡出的
/// 柔和高斯晕光，使面向 camera 的 quad 读作圆形光晕而非白色实心方块。
fn make_star_sprite(images: &mut Assets<Image>) -> Handle<Image> {
    const N: u32 = 64;
    let mut data = Vec::with_capacity((N * N * 4) as usize);
    let c = (N as f32 - 1.0) * 0.5;
    for y in 0..N {
        for x in 0..N {
            let d = (((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / c).min(1.0);
            // 锐利核心 + 宽大微弱晕光，钳位到 1。
            let a = (-(d / 0.22).powi(2)).exp() + 0.28 * (-(d / 0.62).powi(2)).exp();
            let a = a.min(1.0);
            data.push(255);
            data.push(255);
            data.push(255);
            data.push((a * 255.0) as u8);
        }
    }
    let mut img = Image::new(
        bevy::render::render_resource::Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    images.add(img)
}

/// 构建整片星野：将亮星与暗星合并到单一 mesh，配以圆形光晕 sprite 材质。
fn setup_starfield(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    // 天球半径：足够大以包裹整个场景。
    let radius = 50.0_f32;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // --- 来自领域星表的亮星 ---
    let star_sphere = StarSphere::with_builtin_catalog();
    for star in star_sphere.visible_stars() {
        // 将 RA/Dec 转换为天球上的 3D 位置
        let ra = star.right_ascension as f32;
        let dec = star.declination as f32;
        let pos = Vec3::new(
            radius * dec.cos() * ra.cos(),
            radius * dec.sin(),
            radius * dec.cos() * ra.sin(),
        );
        // 更亮的星（星等越低）-> 更大的光晕 quad。尺寸设定使最亮的星
        // 在默认视图下仅跨几个像素。
        let half = 0.08 + 0.03 * (6.0 - star.magnitude as f32).max(0.0);
        push_star_quad(&mut positions, &mut normals, &mut uvs, &mut indices, pos, half);
    }

    // --- 过程化暗星（~1500 个随机点）---
    let dim_count = 1500u32;
    for i in 0..dim_count {
        // 通过 hash 在球面上均匀分布
        let theta = hash_rand(i * 3 + 1) * std::f32::consts::TAU; // 方位角
        let phi = (hash_rand(i * 3 + 2) * 2.0 - 1.0).acos(); // 极角
        let pos = Vec3::new(
            radius * phi.sin() * theta.cos(),
            radius * phi.cos(),
            radius * phi.sin() * theta.sin(),
        );
        let half = 0.03 + hash_rand(i * 3 + 3) * 0.06;
        push_star_quad(&mut positions, &mut normals, &mut uvs, &mut indices, pos, half);
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    // 将所有星 quad 合并为单一 TriangleList mesh。
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));

    // 无光照（unlit）白色材质，贴图提供柔和衰减。
    let star_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(make_star_sprite(&mut images)),
        unlit: true,
        // Alpha 混合的圆形光晕；quad 四角完全透明。
        alpha_mode: AlphaMode::Blend,
        cull_mode: None, // 两侧都可见
        ..default()
    });

    // 整个天空：一个实体 / 一次 draw call。
    commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(star_material)));
}
