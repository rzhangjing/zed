// Empty on purpose. The upstream `msvc_spectre_libs` exposes no public API
// (its `src/lib.rs` is empty too); nothing in the dependency graph ever
// `use`s it. All of its behavior lives in `build.rs`, which this stub no-ops.
