//! 使用 flate2 进行 Gzip 解压缩。

use cesium_ports_driven::{PortError, PortResult};
use flate2::read::GzDecoder;
use std::io::Read;

/// 解压一段 gzip 字节流。`data` 为压缩输入；返回解压后的
/// 原始字节。输入非合法 gzip 时映射为 [`PortError::Decode`]。
pub fn decode_gzip(data: &[u8]) -> PortResult<Vec<u8>> {
    // 包装输入为流式 GzDecoder，一次性读到末尾。
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| PortError::Decode(format!("failed to decompress gzip: {e}")))?;
    Ok(decompressed)
}
