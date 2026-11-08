//! Thin bridge to the root-workspace [`clustering`] crate.
//!
//! The algorithms, the synthetic cloud, the silhouette implementation and the
//! rendering live in `crates/fundamental/clustering` (a member of the repository
//! workspace). This module only re-exports that surface for the `models` binary
//! and its tests, and keeps the `models cluster` CLI contract unchanged.
//!
//! `crates/models` is a separate cargo workspace, so it cannot use
//! `workspace.dependencies`; the path dependency in `Cargo.toml` is the one
//! edge between the two workspaces. It enables `nightly-kmeans` because this
//! workspace's `.cargo/config.toml` opts into `RUSTC_BOOTSTRAP=1` for the
//! nightly-only `kmeans` 2.0.2 crate.

pub use clustering::{
    Algo, Group, Outcome, SyntheticData, execute, run_all, silhouette, synthetic, table_header,
};

/// Runs one algorithm and returns the printable summary.
///
/// Same signature the Model Test window and the `models cluster` subcommand
/// have always used.
pub fn run(algo: Algo, points: usize, clusters: usize, seed: u64) -> Result<String, String> {
    clustering::run(algo, points, clusters, seed)
}
