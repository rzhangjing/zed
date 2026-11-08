//! Integration tests for the pure-Rust clustering demos.
//!
//! Four properties, one cloud per test:
//!
//! 1. every algorithm finds at least two of the three well-separated blobs;
//! 2. both K-Means implementations score above 0.5 silhouette;
//! 3. the synthetic cloud itself is deterministic in `(points, clusters, seed)`;
//! 4. two runs with the same seed agree on cluster and noise counts.

use models::cluster::{self, Algo};

const TEST_POINTS: usize = 150;
const TEST_SEED: u64 = 7;

#[test]
fn every_algorithm_finds_at_least_two_of_three_blobs() {
    for algo in Algo::ALL {
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
    for algo in [Algo::Kmeans, Algo::LinfaKmeans] {
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

    // `all` prints a header and one row per algorithm without aborting.
    let report = cluster::run_all(TEST_POINTS, 3, TEST_SEED);
    assert!(report.contains("algorithm"));
    for algo in Algo::ALL {
        assert!(report.contains(algo.name()), "missing {}", algo.name());
    }
}
