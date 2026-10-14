//! 从 dynamic_globe 黄金路径（M1.5）中抽取出的基础球体模块。
//!
//! 包含 `BaseSphereMarker` 组件、全球合成 resource/辅助函数，以及
//! 用于非-LOD 回退球体的图像工具。由薄壳 `dynamic_globe.rs` 黄金路径共享
//! （冻结的 `dynamic_globe_legacy.rs` A/B 分支已于 2026-09-27 退役，
//! G4 已证明 shell/legacy 像素中性）—— 逻辑逐字节一致，仅模块边界有别。
//!
//! 在 `dynamic_globe.rs` 中的原始位置：
//! - `BaseSphereMarker`：L1904-1905
//! - `COMPOSITE_TILE`/`COMPOSITE_SIZE`：L1907-1908
//! - `BaseSphereComposite`：L1910-1914
//! - `box_downsample`：L1995-2017
//! - `ocean_block`：L2021-2027
//! - `make_clamped_image`：L2031-2062

// 冻结的 legacy 黄金路径风格债；本地 allow 以满足严格的 CI clippy 门
#![allow(clippy::type_complexity)]

use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use std::sync::{mpsc, Mutex};

use crate::globe_textures::build_mip_chain;

// ── 组件 ────────────────────────────────────────────────────────────

/// 非-LOD 基础球体的标记，使合成系统能找到它的材质，
/// 并将烘焙好的全球纹理贴覆其上。
///
/// Original: `dynamic_globe.rs:1904-1905` (逐字节保留).
#[derive(Component)]
pub struct BaseSphereMarker;

// ── 常量 ────────────────────────────────────────────────────────────

/// 合成图中的单瓦片块尺寸（128 px）。
pub const COMPOSITE_TILE: u32 = 128;
/// 合成纹理尺寸：z=3 → 8×8 瓦片 × 128 px = 1024 px。
pub const COMPOSITE_SIZE: u32 = 8 * COMPOSITE_TILE;

// ── 资源 ─────────────────────────────────────────────────────────────

/// 追踪异步全球合成烘焙状态。
///
/// Original: `dynamic_globe.rs:1910-1914` (逐字节保留).
#[derive(Resource, Default)]
pub struct BaseSphereComposite {
    pub rx: Option<Mutex<mpsc::Receiver<(Vec<u8>, u32)>>>,
    pub done: bool,
}

// ── 辅助函数 ──────────────────────────────────────────────────────────

/// 将一个宽度为 `w` 的 RGBA 瓦片（target 的 2 的幂次倍数）
/// 箱式降采样到 `target` × `target`。
///
/// Original: `dynamic_globe.rs:1995-2017` (逐字节保留).
pub fn box_downsample(src: &[u8], w: u32, target: u32) -> Vec<u8> {
    let factor = (w / target).max(1);
    let mut out = vec![0u8; (target * target * 4) as usize];
    for y in 0..target {
        for x in 0..target {
            let mut acc = [0u32; 4];
            for dy in 0..factor {
                for dx in 0..factor {
                    let i = (((y * factor + dy) * w + (x * factor + dx)) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += src[i + c] as u32;
                    }
                }
            }
            let n = factor * factor;
            let o = ((y * target + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    out
}

/// 接近 Bing 影像水面像素的原始（unlit）深海蓝，用于那些瓦片
/// 无影像的合成块。
///
/// Original: `dynamic_globe.rs:2021-2027` (逐字节保留).
pub fn ocean_block() -> Vec<u8> {
    let mut v = Vec::with_capacity((COMPOSITE_TILE * COMPOSITE_TILE * 4) as usize);
    for _ in 0..COMPOSITE_TILE * COMPOSITE_TILE {
        v.extend_from_slice(&[12, 28, 44, 255]);
    }
    v
}

/// 与 `make_image` 类似，但在边缘处 clamp：合成图是一张单独的
/// 全球 Mercator 图像，而非重复瓦片。
///
/// Original: `dynamic_globe.rs:2031-2062` (逐字节保留).
/// sRGB: `Rgba8UnormSrgb` (硬约束).
pub fn make_clamped_image(
    images: &mut Assets<Image>,
    data: Vec<u8>,
    width: u32,
    height: u32,
    levels: u32,
) -> Handle<Image> {
    let base_len = (width * height * 4) as usize;
    let mut img = Image::new(
        bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data[..base_len].to_vec(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    img.data = data;
    img.texture_descriptor.mip_level_count = levels;
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..default()
    });
    images.add(img)
}

/// 在后台线程上由 8×8 基础层块拼装出 1024×1024 的全球合成图，
/// 返回通道接收端。
///
/// 原始：`dynamic_globe.rs:1974-1990`（`base_sphere_composite_system` 中
/// spawn 线程的那部分）。抽取出来，使薄壳与 legacy 共享同一套烘焙逻辑。
pub fn spawn_composite_bake(blocks: Vec<(u32, u32, Vec<u8>)>) -> Mutex<mpsc::Receiver<(Vec<u8>, u32)>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut comp = vec![0u8; (COMPOSITE_SIZE * COMPOSITE_SIZE * 4) as usize];
        for (bx, by, blk) in blocks {
            let x0 = bx * COMPOSITE_TILE;
            let y0 = by * COMPOSITE_TILE;
            for row in 0..COMPOSITE_TILE {
                let src = (row * COMPOSITE_TILE * 4) as usize;
                let dst = ((y0 + row) * COMPOSITE_SIZE * 4 + x0 * 4) as usize;
                comp[dst..dst + (COMPOSITE_TILE * 4) as usize]
                    .copy_from_slice(&blk[src..src + (COMPOSITE_TILE * 4) as usize]);
            }
        }
        let (chain, levels) = build_mip_chain(comp, COMPOSITE_SIZE, COMPOSITE_SIZE);
        let _ = tx.send((chain, levels));
    });
    Mutex::new(rx)
}
