//! Integration tests for the pure-Rust clustering bridge.
//!
//! Four properties, one cloud per test:
//!
//! 1. every available algorithm finds at least two of the three well-separated blobs;
//! 2. both K-Means implementations score above 0.5 silhouette;
//! 3. the synthetic cloud itself is deterministic in `(points, clusters, seed)`;
//! 4. two runs with the same seed agree on cluster and noise counts, and the
//!    comparison table lists all eight algorithms one line at a time.
//!
//! `Algo::Kmeans` is the nightly-only `kmeans` crate; without the
//! `nightly-kmeans` feature (the root workspace's default) its row is a
//! documented error, so the checks skip it unless the feature is on.

use std::sync::atomic::AtomicBool;

use models_core::cluster::{self, Algo};

const TEST_POINTS: usize = 150;
const TEST_SEED: u64 = 7;

fn kmeans_available() -> bool {
    cfg!(feature = "nightly-kmeans")
}

#[test]
fn every_available_algorithm_finds_at_least_two_of_three_blobs() {
    for algo in Algo::ALL {
        if algo == Algo::Kmeans && !kmeans_available() {
            continue;
        }
        let outcome = cluster::execute(algo, TEST_POINTS, 3, TEST_SEED)
            .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));

        assert_eq!(outcome.points, TEST_POINTS, "{}", algo.name());
        assert!(
            outcome.clusters >= 2,
            "{} found only {} clusters (noise {})",
            algo.name(),
            outcome.clusters,
            outcome.noise
        );
        assert!(
            outcome.silhouette.is_finite(),
            "{} produced a non-finite silhouette",
            algo.name()
        );
        assert!(
            outcome.groups.iter().all(|group| group.count > 0),
            "{} returned an empty cluster",
            algo.name()
        );
    }
}

#[test]
fn both_kmeans_implementations_score_above_half_silhouette() {
    let mut algorithms = vec![Algo::LinfaKmeans];
    if kmeans_available() {
        algorithms.push(Algo::Kmeans);
    }
    for algo in algorithms {
        let outcome = cluster::execute(algo, 600, 3, 42)
            .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));
        assert!(
            outcome.silhouette > 0.5,
            "{} silhouette {:.3} should beat 0.5",
            algo.name(),
            outcome.silhouette
        );
    }
}

#[test]
fn same_seed_produces_the_same_cluster_counts() {
    for algo in Algo::ALL {
        if algo == Algo::Kmeans && !kmeans_available() {
            continue;
        }
        let first = cluster::execute(algo, TEST_POINTS, 3, TEST_SEED)
            .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));
        let second = cluster::execute(algo, TEST_POINTS, 3, TEST_SEED)
            .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));
        assert_eq!(
            first.clusters,
            second.clusters,
            "{} cluster count changed between identical runs",
            algo.name()
        );
        assert_eq!(
            first.noise,
            second.noise,
            "{} noise count changed between identical runs",
            algo.name()
        );
    }
}

#[test]
fn the_cloud_is_deterministic_and_comparable() {
    let first = cluster::synthetic(TEST_POINTS, 3, TEST_SEED);
    let second = cluster::synthetic(TEST_POINTS, 3, TEST_SEED);
    assert_eq!(first.points, second.points);
    assert_eq!(first.truth, second.truth);
    assert_eq!(first.outliers, second.outliers);
    assert_eq!(first.len(), TEST_POINTS);

    let other = cluster::synthetic(TEST_POINTS, 3, TEST_SEED + 1);
    assert_ne!(first.points, other.points);

    // The in-process entry point reports exactly one progress line per
    // algorithm, and the rendered report names every algorithm.
    let cancel = AtomicBool::new(false);
    let mut lines = Vec::new();
    let summary = cluster::run_cluster(
        Some("all"),
        TEST_POINTS,
        3,
        TEST_SEED,
        &mut |line| lines.push(line.to_owned()),
        &cancel,
    )
    .expect("all algorithms report a table");
    for algo in Algo::ALL {
        assert!(
            summary.rendered.contains(algo.name()),
            "missing {}",
            algo.name()
        );
        assert!(
            lines.iter().any(|line| line.starts_with(algo.name())),
            "missing progress line for {}",
            algo.name()
        );
        assert!(
            summary.table.contains(algo.name()),
            "missing table row for {}",
            algo.name()
        );
    }
    assert_eq!(
        summary.failures.len(),
        if kmeans_available() { 0 } else { 1 }
    );
}

#[test]
fn a_cancelled_cluster_run_stops_before_the_table() {
    let cancel = AtomicBool::new(true);
    let error = cluster::run_cluster(Some("all"), TEST_POINTS, 3, TEST_SEED, &mut |_| {}, &cancel)
        .expect_err("a pre-set token must cancel the table");
    assert!(error.is_cancelled());
}
