//! Writes the `assets` directory the admin demo loads at runtime.
//!
//! Topcoat's own flow is `cargo topcoat asset bundle` / `cargo topcoat dev`,
//! which builds the app's binary and bundles the assets declared in it. Zed
//! embeds this demo instead, so the declarations end up in `zed.exe`, and the
//! bundle has to be generated from a binary that carries them — either
//! `zed.exe` or the standalone `admin.exe` (the asset ids are content
//! addressed, so both produce the same manifest).
//!
//! ```text
//! cargo run -p admin --features bundle-tool --bin admin-bundle-assets -- \
//!     <binary> [out-dir]
//! ```
//!
//! `out-dir` defaults to `assets` next to `binary`, which is where
//! `AssetBundle::load` looks (`<exe dir>/assets`). Remote assets (fonts, icons)
//! are downloaded into the shared Topcoat cache on first use.

use std::{
    path::{Path, PathBuf},
    process::exit,
};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(binary) = args.next().map(PathBuf::from) else {
        eprintln!(
            "usage: admin-bundle-assets <binary> [out-dir]\n\
             out-dir defaults to `assets` next to the binary"
        );
        exit(2);
    };
    let out_dir = args.next().map(PathBuf::from).unwrap_or_else(|| {
        binary
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("assets")
    });

    let bytes = match std::fs::read(&binary) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("cannot read {}: {error}", binary.display());
            exit(1);
        }
    };

    // The bundler reuses downloads (fonts, icons) from a cache. Cargo's build
    // scripts derive it from `OUT_DIR`; this tool runs outside cargo, so it
    // names the same `<target>/topcoat/cache/asset` directory explicitly.
    // `ADMIN_ASSETS_CACHE` overrides it.
    let cache_dir = std::env::var_os("ADMIN_ASSETS_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_cache_dir(&binary));

    let config = topcoat_asset::BundlerConfig::new().cache_dir(cache_dir);
    let bundler = topcoat_asset::Bundler::new(&config);
    match bundler.bundle(&bytes, &out_dir) {
        Ok(()) => println!("bundled assets into {}", out_dir.display()),
        Err(error) => {
            eprintln!("failed to bundle assets: {error}");
            exit(1);
        }
    }
}

/// `<target>/topcoat/cache/asset`, derived from a binary in `<target>/<profile>`.
fn default_cache_dir(binary: &Path) -> PathBuf {
    let dir = binary.parent().unwrap_or_else(|| Path::new("."));
    let target = match dir.file_name().and_then(|name| name.to_str()) {
        Some("debug" | "release") => dir.parent().unwrap_or(dir).to_path_buf(),
        _ => dir.to_path_buf(),
    };
    target.join("topcoat").join("cache").join("asset")
}
