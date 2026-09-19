// Intentionally empty: the upstream crate's build.rs adds a
// `cargo:rustc-link-search` pointing at the Spectre-mitigated MSVC CRT libs and
// panics (when built with the `error` feature) if they are missing. This stub does
// nothing, so the build falls back to the regular CRT and never panics.
fn main() {}
