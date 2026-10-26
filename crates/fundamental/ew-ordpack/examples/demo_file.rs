//! 单条记录持久化：文件头（magic + 格式版本 + schema 指纹）。
//!
//! 运行：`cargo run --example demo_file`

use std::fs;

use ew_ordpack::{Decode, Encode};

/// 4 字节 magic + 1 字节格式版本 + 8 字节类型指纹。
const MAGIC: &[u8; 4] = b"EWOP";
const HEADER_LEN: usize = 13;

#[derive(Encode, Decode, PartialEq, Debug)]
struct Config {
    id: u64,
    name: String,
    retries: u32,
}

fn save(path: &str, value: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let payload = ew_ordpack::to_vec(value)?;
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(ew_ordpack::FORMAT_VERSION);
    out.extend_from_slice(&Config::FINGERPRINT.to_le_bytes());
    out.extend_from_slice(&payload);
    fs::write(path, out)?;
    Ok(())
}

fn load(path: &str) -> Result<Config, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    if bytes.len() < HEADER_LEN || &bytes[..4] != MAGIC {
        return Err("bad magic or truncated header".into());
    }
    if bytes[4] != ew_ordpack::FORMAT_VERSION {
        return Err("format version mismatch".into());
    }
    let fingerprint = u64::from_le_bytes(bytes[5..HEADER_LEN].try_into()?);
    if fingerprint != Config::FINGERPRINT {
        return Err("schema fingerprint mismatch".into());
    }
    Ok(ew_ordpack::from_slice(&bytes[HEADER_LEN..])?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("ew_ordpack_demo_file.bin");
    let path = path.to_str().expect("temporary path must be UTF-8");

    let config = Config {
        id: 7,
        name: "demo".into(),
        retries: 3,
    };

    save(path, &config)?;
    println!(
        "saved: {} bytes (header {HEADER_LEN} + payload {})",
        fs::metadata(path)?.len(),
        fs::metadata(path)?.len() as usize - HEADER_LEN
    );

    let loaded = load(path)?;
    println!("loaded: {loaded:?}");
    assert_eq!(loaded, config);

    // 篡改指纹：加载时能在解码前发现 schema 不匹配。
    let mut bytes = fs::read(path)?;
    bytes[5] ^= 0xFF;
    fs::write(path, &bytes)?;
    println!("tampered header -> {:?}", load(path).err());

    fs::remove_file(path).ok();
    Ok(())
}
