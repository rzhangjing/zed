//! cesium-decoders：通用二进制解码器（影像 / gzip）
//!
//! CesiumJS 映射：
//! - 图像解码（PNG/JPEG/WebP）
//! - Gzip 解压缩
//!
//! 2026-09 P1 裁剪：`quantized_mesh_decoder`（quantized-mesh 地形）与
//! `content_type`（b3dm/i3dm/pnts/cmpt 魔数嗅探，本来是到 `cesium-tileset`
//! 的三行桥接）随 `cesium-terrain` / `cesium-tileset` 删除；
//! `draco_decoder` + `draco` feature 随 glTF/3D Tiles 内容链删除。
//! 本 crate 现在只保留与内容格式无关的通用解码器（影像 + gzip），
//! 当前工作区无消费者，属「储备能力」。

pub mod image_decoder;
pub mod gzip_decoder;

use cesium_ports_driven::{DecodedImage, Decoder, PortError, PortResult};

/// [`Decoder`] port 契约的适配器实现：将各二进制解码
/// 任务委派给同 crate 内的具体解码器模块。本身无状态，
/// 因此可作为共享单例注册到依赖注入容器。
pub struct DecoderImpl;

impl Decoder for DecoderImpl {
    /// 解码 Draco 压缩网格为 [`GeometryData`](cesium_geospatial::GeometryData)。
    ///
    /// 2026-09 P1：`draco` feature 已删除（Draco 只服务 3D Tiles / glTF
    /// 压缩内容），因此该方法现在**无条件**返回描述“尚未实现”的
    /// [`PortError::Decode`]，以便上层优雅降级。`_data` 为 Draco 字节流。
    fn decode_draco(&self, _data: &[u8]) -> PortResult<cesium_geospatial::GeometryData> {
        Err(PortError::Decode(
            "Draco decoding not yet implemented".to_string(),
        ))
    }

    /// 将编码图像字节（PNG/JPEG/WebP）解码为 RGBA 的 [`DecodedImage`]。
    /// `data` 为原始文件字节；解码失败时返回 [`PortError::Decode`]。
    fn decode_image(&self, data: &[u8]) -> PortResult<DecodedImage> {
        image_decoder::decode_image(data)
    }

    /// 解压一个 gzip 流。`data` 为压缩字节，返回解压后的原始
    /// 字节；输入非合法 gzip 时返回 [`PortError::Decode`]。
    fn decode_gzip(&self, data: &[u8]) -> PortResult<Vec<u8>> {
        gzip_decoder::decode_gzip(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // 测试依赖：手工用 flate2/image 编码出待解码的输入字节。
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use image::ImageEncoder;
    use std::io::Write;

    /// 验证一张 4×4 RGBA PNG 能被解码出正确的尺寸与通道数。
    #[test]
    fn test_decode_png() {
        let png_data = create_test_png();
        let decoder = DecoderImpl;
        let result = decoder.decode_image(&png_data).unwrap();
        // 解码后应保留原始的 4×4 尺寸与 4 通道 RGBA。
        assert_eq!(result.width, 4);
        assert_eq!(result.height, 4);
        assert_eq!(result.channels, 4);
        assert_eq!(result.data.len(), 4 * 4 * 4);
    }

    /// 验证一张 4×4 JPEG 能被解码；解码器统一输出 RGBA 四通道。
    #[test]
    fn test_decode_jpeg() {
        let jpeg_data = create_test_jpeg();
        let decoder = DecoderImpl;
        let result = decoder.decode_image(&jpeg_data).unwrap();
        // JPEG 虽为 RGB 存储，但解码后提升为 RGBA。
        assert_eq!(result.width, 4);
        assert_eq!(result.height, 4);
        assert_eq!(result.channels, 4);
    }

    /// gzip 压缩→解压往返：输出应与原始字节逐字相等。
    #[test]
    fn test_decode_gzip_roundtrip() {
        let original = b"Hello, quantized mesh! This is test data for gzip roundtrip verification.";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decoder = DecoderImpl;
        let decompressed = decoder.decode_gzip(&compressed).unwrap();
        // 解压后字节应与未压缩前的原文完全一致。
        assert_eq!(decompressed, original);
    }

    /// 空输入的 gzip 往返：解压结果也应为空。
    #[test]
    fn test_decode_gzip_empty() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"").unwrap();
        let compressed = encoder.finish().unwrap();

        let decoder = DecoderImpl;
        let decompressed = decoder.decode_gzip(&compressed).unwrap();
        // 空流解压后仍为空，不报错。
        assert!(decompressed.is_empty());
    }

    /// 未启用 `draco` feature 时，decode_draco 应返回一个“尚未实现”错误。
    #[test]
    fn test_draco_stub() {
        let decoder = DecoderImpl;
        let result = decoder.decode_draco(b"fake draco data");
        // 无论是否启用 feature，无效数据都应报错而非 panic。
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not yet implemented"));
    }

    /// 用逐像素递推的 RGBA 值构造一张 4×4 PNG，供解码测试使用。
    fn create_test_png() -> Vec<u8> {
        let mut pixels = Vec::with_capacity(4 * 4 * 4);
        // 逐像素递推一组 RGBA：透明度固定为 255（不透明）。
        for i in 0..16 {
            let v = (i * 16) as u8;
            pixels.extend_from_slice(&[v, 255 - v, v, 255]);
        }
        let mut buf = Vec::new();
        {
            let encoder = image::codecs::png::PngEncoder::new(&mut buf);
            // 以 RGBA8 布局写入 4×4 图像到内存缓冲。
            encoder
                .write_image(
                    &pixels,
                    4,
                    4,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
        }
        buf
    }

    /// 用逐像素递推的 RGB 值构造一张 4×4 JPEG（质量 90），供解码测试使用。
    fn create_test_jpeg() -> Vec<u8> {
        let mut pixels = Vec::with_capacity(4 * 4 * 3);
        // 逐像素递推一组 RGB（JPEG 无透明通道）。
        for i in 0..16 {
            let v = (i * 16) as u8;
            pixels.extend_from_slice(&[v, 128, 255 - v]);
        }
        let mut buf = Vec::new();
        {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
            // 以 RGB8 布局、质量 90 写入 4×4 图像。
            encoder
                .write_image(
                    &pixels,
                    4,
                    4,
                    image::ExtendedColorType::Rgb8,
                )
                .unwrap();
        }
        buf
    }
}
