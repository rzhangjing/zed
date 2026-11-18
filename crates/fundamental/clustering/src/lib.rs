//! Pure-Rust clustering demos over one deterministic synthetic point cloud.
//!
//! This crate is a normal member of the repository workspace. It is also the
//! implementation behind the separate `crates/models` demo binary (`models
//! cluster ...`) and, through that binary, the Model Test window's cluster
//! buttons.
//!
//! The algorithms are exercised for real (no mock results):
//!
//! * `linfa-clustering` — K-Means, DBSCAN, OPTICS and a Gaussian mixture model.
//! * `smartcore` — K-Means and DBSCAN on its own `DenseMatrix`.
//! * `hdbscan` — hierarchical density clustering.
//! * `kmeans` — Lloyd's algorithm with k-means++ initialisation. This crate is
//!   nightly-only upstream (2.0.2 ships `#![feature(portable_simd)]`), so its
//!   algorithm is behind the `nightly-kmeans` feature and is off in the default
//!   root-workspace build.
//!
//! All algorithms consume the same [`SyntheticData`]: `clusters` Gaussian blobs
//! arranged on a circle plus a small fraction of far-away outliers. Every run is
//! deterministic in `(points, clusters, seed)`, so the printed summaries are
//! comparable and the tests can assert on cluster counts and silhouette scores.

pub mod cluster;

pub use cluster::{
    Algo, Group, Outcome, SyntheticData, execute, run, run_all, silhouette, synthetic,
    table_header,
};
