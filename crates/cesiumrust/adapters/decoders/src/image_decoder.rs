//! 面向 PNG、JPEG、WebP 格式的图像解码。
//!
//! 使用 `image` crate，它从魔数字节自动检测格式。

use cesium_ports_driven::{DecodedImage, PortError, PortResult};
use image::GenericImageView;

/// 从内存字节解码一张图像（PNG/JPEG/WebP 等）。
///
/// 统一输出为 RGBA8（`channels` 固定为 4）；失败时返回
/// [`PortError::Decode`]。
pub fn decode_image(data: &[u8]) -> PortResult<DecodedImage> {
    // 由魔数字自动检测格式并从字节加载。
    let img = image::load_from_memory(data)
        .map_err(|e| PortError::Decode(format!("failed to decode image: {e}")))?;

    let (width, height) = img.dimensions();
    // 无论源格式如何，都提升到 RGBA，使下游对通道数有一致预期。
    let rgba = img.to_rgba8();

    Ok(DecodedImage {
        width,
        height,
        channels: 4,
        data: rgba.into_raw(),
    })
}
