# clustering — pure-Rust clustering demos

A member of the repository workspace (`crates/fundamental/clustering`). It hosts
the algorithm implementations shared by the `crates/models` CLI and the Model
Test window:

- **K-Means** — `linfa-clustering` and `smartcore`, plus the `kmeans` crate behind
  the `nightly-kmeans` feature.
- **DBSCAN** — `linfa-clustering` and `smartcore`.
- **OPTICS** — `linfa-clustering` (the reachability ordering is converted to
  DBSCAN-style labels by thresholding `eps`).
- **GMM** — `linfa-clustering`'s `GaussianMixtureModel`.
- **HDBSCAN** — the `hdbscan` crate.

Every algorithm consumes the same deterministic synthetic cloud
(`synthetic(points, clusters, seed)`): Gaussian blobs on a circle plus roughly 5%
far-away outliers, then a seeded shuffle. The public entry points are:

```rust
use clustering::{Algo, run, run_all};

let summary = run(Algo::LinfaDbscan, 600, 3, 42)?; // printable multi-line summary
let table = run_all(600, 3, 42);                    // comparison table + summaries
```

`silhouette` is implemented locally (Euclidean distance, O(n²) with a 1000-point
deterministic sample cap) so the score is comparable across libraries.

## `kmeans` 2.0.2 is nightly-only

Version 2.0.2 of the `kmeans` crate starts its lib with
`#![feature(portable_simd)]` and `extern crate test`; the repository pins the
stable toolchain in `rust-toolchain.toml`. The demo is therefore behind the
`nightly-kmeans` feature:

```toml
[dependencies]
kmeans = { workspace = true, optional = true }

[features]
nightly-kmeans = ["dep:kmeans"]
```

The default root-workspace gates (`cargo check -p clustering`, `cargo test -p
clustering`) do not compile that feature, so `kmeans` stays out of the stable
build. `crates/models` is a separate workspace with its own
`.cargo/config.toml`; it sets `RUSTC_BOOTSTRAP=1` and enables `nightly-kmeans`,
so `models cluster --algo kmeans` works there. `run(Algo::Kmeans, ..)` returns a
clear error when the feature is off.

## Tests

```powershell
cargo test -p clustering --locked
```

Covers the synthetic cloud's determinism, the silhouette implementation, and one
labelled run per available algorithm on the same 120-point cloud.
