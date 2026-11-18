//! Pure-Rust clustering demos over one deterministic synthetic point cloud.
//!
//! Five crates are exercised for real (no mock results):
//!
//! * `kmeans` — Lloyd's algorithm with k-means++ initialisation.
//! * `linfa-clustering` — K-Means, DBSCAN, OPTICS and a Gaussian mixture model.
//! * `smartcore` — K-Means and DBSCAN on its own `DenseMatrix`.
//! * `hdbscan` — hierarchical density clustering.
//!
//! All algorithms consume the same [`SyntheticData`]: `clusters` Gaussian blobs
//! arranged on a circle plus a small fraction of far-away outliers. Every run is
//! deterministic in `(points, clusters, seed)`, so the printed summaries are
//! comparable and the tests can assert on cluster counts and silhouette scores.
//!
//! The CLI exposes this through `models cluster --algo <name>`; the module also
//! powers the Model Test window's cluster buttons.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use linfa::traits::{Fit, Predict, Transformer};
use linfa_clustering::{Dbscan, GaussianMixtureModel, KMeans, Optics};
use ndarray::Array2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Standard deviation of each Gaussian blob, in data units.
const BLOB_STD: f64 = 0.7;
/// Circle radius the blob centres sit on.
const BLOB_RADIUS: f64 = 12.0;
/// DBSCAN / OPTICS neighbourhood radius for this point cloud.
const DENSITY_EPS: f64 = 2.0;
/// DBSCAN / OPTICS minimum neighbourhood size.
const DENSITY_MIN_POINTS: usize = 5;
/// Maximum number of points the silhouette calculation looks at.
const SILHOUETTE_SAMPLE_LIMIT: usize = 1000;
/// Maximum number of cluster summaries printed per run.
const PRINTED_GROUPS: usize = 4;

/// One supported clustering implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Algo {
    /// The `kmeans` crate's Lloyd implementation.
    Kmeans,
    /// `linfa-clustering`'s K-Means.
    LinfaKmeans,
    /// `linfa-clustering`'s DBSCAN.
    LinfaDbscan,
    /// `linfa-clustering`'s OPTICS.
    LinfaOptics,
    /// `linfa-clustering`'s Gaussian mixture model.
    LinfaGmm,
    /// `smartcore`'s K-Means.
    SmartcoreKmeans,
    /// `smartcore`'s DBSCAN.
    SmartcoreDbscan,
    /// The `hdbscan` crate's HDBSCAN.
    Hdbscan,
}

impl Algo {
    /// Every algorithm, in the order `all` runs them.
    pub const ALL: [Algo; 8] = [
        Algo::Kmeans,
        Algo::LinfaKmeans,
        Algo::LinfaDbscan,
        Algo::LinfaOptics,
        Algo::LinfaGmm,
        Algo::SmartcoreKmeans,
        Algo::SmartcoreDbscan,
        Algo::Hdbscan,
    ];

    /// The stable CLI name (`--algo <name>`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Kmeans => "kmeans",
            Self::LinfaKmeans => "linfa-kmeans",
            Self::LinfaDbscan => "linfa-dbscan",
            Self::LinfaOptics => "linfa-optics",
            Self::LinfaGmm => "linfa-gmm",
            Self::SmartcoreKmeans => "smartcore-kmeans",
            Self::SmartcoreDbscan => "smartcore-dbscan",
            Self::Hdbscan => "hdbscan",
        }
    }

    /// A human-readable label used in the rendered summaries.
    pub fn label(self) -> &'static str {
        match self {
            Self::Kmeans => "K-Means (kmeans crate)",
            Self::LinfaKmeans => "K-Means (linfa)",
            Self::LinfaDbscan => "DBSCAN (linfa)",
            Self::LinfaOptics => "OPTICS (linfa)",
            Self::LinfaGmm => "GMM (linfa)",
            Self::SmartcoreKmeans => "K-Means (smartcore)",
            Self::SmartcoreDbscan => "DBSCAN (smartcore)",
            Self::Hdbscan => "HDBSCAN (hdbscan)",
        }
    }

    /// Parses a CLI name. `all` is handled by the caller.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "kmeans" => Some(Self::Kmeans),
            "linfa-kmeans" => Some(Self::LinfaKmeans),
            "linfa-dbscan" => Some(Self::LinfaDbscan),
            "linfa-optics" => Some(Self::LinfaOptics),
            "linfa-gmm" => Some(Self::LinfaGmm),
            "smartcore-kmeans" => Some(Self::SmartcoreKmeans),
            "smartcore-dbscan" => Some(Self::SmartcoreDbscan),
            "hdbscan" => Some(Self::Hdbscan),
            _ => None,
        }
    }
}

/// The shared synthetic point cloud.
#[derive(Clone, Debug)]
pub struct SyntheticData {
    /// Two-dimensional points, in generation order (then deterministically shuffled).
    pub points: Vec<[f64; 2]>,
    /// The blob a point was generated from; outliers get `usize::MAX`.
    pub truth: Vec<usize>,
    /// `true` for points drawn far away from every blob.
    pub outliers: Vec<bool>,
}

impl SyntheticData {
    /// Number of points.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// `true` when the cloud has no points (never in practice).
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

/// Generates the deterministic point cloud used by every algorithm.
///
/// `clusters` blob centres are placed on a circle of radius [`BLOB_RADIUS`];
/// roughly 5% of `points` are then replaced by far-away outliers. A fixed
/// `seed` fully determines the cloud, and the final shuffle removes any
/// ordering advantage a particular algorithm could exploit.
pub fn synthetic(points: usize, clusters: usize, seed: u64) -> SyntheticData {
    if points == 0 {
        return SyntheticData {
            points: Vec::new(),
            truth: Vec::new(),
            outliers: Vec::new(),
        };
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let cluster_count = clusters.max(1).min(points.max(1));
    let outlier_count = ((points as f64 * 0.05).round() as usize)
        .min(points.saturating_sub(cluster_count))
        .max(1.min(points));
    let inlier_count = points.saturating_sub(outlier_count);

    let centers: Vec<[f64; 2]> = (0..cluster_count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / cluster_count as f64;
            [BLOB_RADIUS * angle.cos(), BLOB_RADIUS * angle.sin()]
        })
        .collect();

    let mut data = SyntheticData {
        points: Vec::with_capacity(points),
        truth: Vec::with_capacity(points),
        outliers: Vec::with_capacity(points),
    };

    for index in 0..inlier_count {
        // Spread the remainder over the first blobs so the counts stay exact.
        let blob = index % cluster_count;
        let center = centers[blob];
        let mut point = [0.0, 0.0];
        for (axis, value) in point.iter_mut().enumerate() {
            *value = center[axis] + box_muller(&mut rng) * BLOB_STD;
        }
        data.points.push(point);
        data.truth.push(blob);
        data.outliers.push(false);
    }

    for _ in 0..outlier_count {
        let angle: f64 = rng.gen_range(0.0..std::f64::consts::TAU);
        let radius = 2.2 * BLOB_RADIUS + rng.gen_range(0.0..0.6) * BLOB_RADIUS;
        data.points
            .push([radius * angle.cos(), radius * angle.sin()]);
        data.truth.push(usize::MAX);
        data.outliers.push(true);
    }

    // Shuffling makes the point order independent of the blob labels.
    for index in (1..data.points.len()).rev() {
        let other = rng.gen_range(0..=index);
        data.points.swap(index, other);
        data.truth.swap(index, other);
        data.outliers.swap(index, other);
    }

    data
}

/// One standard-normal sample (Box-Muller transform).
fn box_muller(rng: &mut StdRng) -> f64 {
    let u1: f64 = rng.gen_range(1e-12..1.0);
    let u2: f64 = rng.gen_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// One cluster's size and centroid.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Label assigned by the algorithm (renumbered to `0..clusters` in output).
    pub label: i32,
    pub count: usize,
    pub centroid: [f64; 2],
}

/// The measured result of one algorithm run.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub algo: Algo,
    pub points: usize,
    pub clusters: usize,
    pub noise: usize,
    pub silhouette: f64,
    pub elapsed: Duration,
    pub groups: Vec<Group>,
}

impl Outcome {
    /// The compact one-line summary used by the `all` table.
    pub fn table_row(&self) -> String {
        format!(
            "{:<18} {:>8} {:>6} {:>10.3} {:>10.1}",
            self.algo.name(),
            self.clusters,
            self.noise,
            self.silhouette,
            self.elapsed.as_secs_f64() * 1000.0
        )
    }

    /// The full multi-line summary printed by `models cluster`.
    pub fn render(&self) -> String {
        let mut out = format!(
            "{} | points={} clusters={} noise={} silhouette={:.3} elapsed={:.1}ms",
            self.algo.label(),
            self.points,
            self.clusters,
            self.noise,
            self.silhouette,
            self.elapsed.as_secs_f64() * 1000.0
        );
        for group in self.groups.iter().take(PRINTED_GROUPS) {
            let _ = write!(
                out,
                "\n  cluster {}: n={} centroid=({:.2}, {:.2})",
                group.label, group.count, group.centroid[0], group.centroid[1]
            );
        }
        if self.groups.len() > PRINTED_GROUPS {
            let _ = write!(
                out,
                "\n  ... {} more clusters",
                self.groups.len() - PRINTED_GROUPS
            );
        }
        out
    }
}

/// The table header for [`run_all`].
pub fn table_header() -> String {
    format!(
        "{:<18} {:>8} {:>6} {:>10} {:>10}",
        "algorithm", "clusters", "noise", "silhouette", "elapsed_ms"
    )
}

/// Runs one algorithm and returns the printable summary.
///
/// This is the interface the Model Test window and the CLI both use.
pub fn run(algo: Algo, points: usize, clusters: usize, seed: u64) -> Result<String, String> {
    Ok(execute(algo, points, clusters, seed)?.render())
}

/// Runs every algorithm and renders the comparison table plus per-algorithm
/// summaries. A failing algorithm is reported in the table instead of aborting
/// the remaining runs.
pub fn run_all(points: usize, clusters: usize, seed: u64) -> String {
    let mut out = format!(
        "synthetic data: {points} points, {clusters} requested clusters, seed {seed}\n\n{}\n",
        table_header()
    );
    let mut summaries = Vec::new();
    for algo in Algo::ALL {
        match execute(algo, points, clusters, seed) {
            Ok(outcome) => {
                out.push_str(&outcome.table_row());
                out.push('\n');
                summaries.push(outcome);
            }
            Err(reason) => {
                let _ = writeln!(out, "{:<18} error: {reason}", algo.name());
            }
        }
    }
    out.push_str("\n");
    for outcome in summaries {
        out.push_str(&outcome.render());
        out.push_str("\n\n");
    }
    out.trim_end().to_owned()
}

/// Runs one algorithm and returns its raw measurements.
///
/// Data generation is deliberately outside the timed region; `elapsed` is the
/// algorithm (plus label post-processing) time.
pub fn execute(
    algo: Algo,
    points: usize,
    clusters: usize,
    seed: u64,
) -> Result<Outcome, String> {
    if points < 2 {
        return Err("at least 2 points are required".to_owned());
    }
    let k = clusters.max(2).min(points);
    let data = synthetic(points, clusters, seed);
    let started = Instant::now();
    let labels = match algo {
        Algo::Kmeans => {
            #[cfg(feature = "nightly-kmeans")]
            {
                run_kmeans_crate(&data, k, seed)?
            }
            #[cfg(not(feature = "nightly-kmeans"))]
            {
                return Err(
                    "the kmeans crate 2.0.2 needs nightly features \
                     (`#![feature(portable_simd)]`); build with `--features nightly-kmeans` \
                     and RUSTC_BOOTSTRAP=1"
                        .to_owned(),
                );
            }
        }
        Algo::LinfaKmeans => run_linfa_kmeans(&data, k, seed)?,
        Algo::LinfaDbscan => run_linfa_dbscan(&data)?,
        Algo::LinfaOptics => run_linfa_optics(&data)?,
        Algo::LinfaGmm => run_linfa_gmm(&data, k, seed)?,
        Algo::SmartcoreKmeans => run_smartcore_kmeans(&data, k, seed)?,
        Algo::SmartcoreDbscan => run_smartcore_dbscan(&data)?,
        Algo::Hdbscan => run_hdbscan(&data)?,
    };
    let elapsed = started.elapsed();

    if labels.len() != data.len() {
        return Err(format!(
            "{} returned {} labels for {} points",
            algo.name(),
            labels.len(),
            data.len()
        ));
    }
    let (groups, noise) = summarize(&labels, &data.points);
    let silhouette = silhouette(&data.points, &labels);
    Ok(Outcome {
        algo,
        points: data.len(),
        clusters: groups.len(),
        noise,
        silhouette,
        elapsed,
        groups,
    })
}

/// `kmeans` crate: Lloyd + k-means++ with a seeded `StdRng`.
///
/// The crate's Lloyd loop has no built-in restart, and a single k-means++ seed
/// can land in a local optimum where one centre covers the far-away outliers and
/// two blobs share a centre (the synthetic cloud deliberately contains both).
/// Five deterministic restarts (seeded from `seed`) keep the best `distsum`,
/// which makes the crate's result comparable to the other K-Means demos while
/// remaining exactly reproducible for a given seed.
///
/// Only available with the `nightly-kmeans` feature: version 2.0.2 upstream
/// declares `#![feature(portable_simd)]` and `extern crate test`, which the
/// pinned stable toolchain rejects.
#[cfg(feature = "nightly-kmeans")]
fn run_kmeans_crate(data: &SyntheticData, k: usize, seed: u64) -> Result<Vec<i32>, String> {
    use kmeans::{EuclideanDistance, KMeans as CrateKMeans, KMeansConfig};

    let flat: Vec<f64> = data
        .points
        .iter()
        .flat_map(|point| [point[0], point[1]])
        .collect();
    let kmean: CrateKMeans<f64, 8, _> =
        CrateKMeans::new(&flat, data.len(), 2, EuclideanDistance);
    let mut best: Option<(f64, Vec<i32>)> = None;
    for attempt in 0..5u64 {
        let config = KMeansConfig::build()
            .random_generator(StdRng::seed_from_u64(seed.wrapping_add(attempt)))
            .build();
        let state = kmean.kmeans_lloyd(k, 100, CrateKMeans::init_kmeanplusplus, &config);
        let labels: Vec<i32> = state
            .assignments
            .iter()
            .map(|&assignment| assignment as i32)
            .collect();
        if best
            .as_ref()
            .is_none_or(|(best_distsum, _)| state.distsum < *best_distsum)
        {
            best = Some((state.distsum, labels));
        }
    }
    Ok(best.expect("the restart loop runs at least once").1)
}

fn dataset(points: &[[f64; 2]]) -> linfa::DatasetBase<Array2<f64>, ndarray::Array1<()>> {
    let mut array = Array2::<f64>::zeros((points.len(), 2));
    for (index, point) in points.iter().enumerate() {
        array[[index, 0]] = point[0];
        array[[index, 1]] = point[1];
    }
    linfa::DatasetBase::from(array)
}

/// `linfa-clustering` K-Means with a seeded RNG.
fn run_linfa_kmeans(data: &SyntheticData, k: usize, seed: u64) -> Result<Vec<i32>, String> {
    let dataset = dataset(&data.points);
    let model = KMeans::params_with_rng(k, StdRng::seed_from_u64(seed))
        .max_n_iterations(300)
        .tolerance(1e-6)
        .fit(&dataset)
        .map_err(|error| format!("linfa KMeans fit failed: {error}"))?;
    Ok(model
        .predict(&dataset)
        .iter()
        .map(|&label| label as i32)
        .collect())
}

/// `linfa-clustering` DBSCAN with the shared epsilon / min-points parameters.
fn run_linfa_dbscan(data: &SyntheticData) -> Result<Vec<i32>, String> {
    let dataset = dataset(&data.points);
    let labeled = Dbscan::params::<f64>(DENSITY_MIN_POINTS)
        .tolerance(DENSITY_EPS)
        .transform(dataset)
        .map_err(|error| format!("linfa DBSCAN failed: {error}"))?;
    Ok(labeled
        .targets()
        .iter()
        .map(|label| label.map_or(-1, |cluster| cluster as i32))
        .collect())
}

/// `linfa-clustering` OPTICS.
///
/// OPTICS itself produces a reachability ordering, not labels, so the ordering
/// is converted to DBSCAN-style labels with the classic reachability threshold
/// extraction: a point whose reachability exceeds `eps` starts a new cluster
/// only when it is also a core point.
fn run_linfa_optics(data: &SyntheticData) -> Result<Vec<i32>, String> {
    let mut array = Array2::<f64>::zeros((data.len(), 2));
    for (index, point) in data.points.iter().enumerate() {
        array[[index, 0]] = point[0];
        array[[index, 1]] = point[1];
    }
    let analysis = Optics::params::<f64>(DENSITY_MIN_POINTS)
        .tolerance(f64::MAX)
        .transform(array.view())
        .map_err(|error| format!("linfa OPTICS failed: {error}"))?;
    let mut labels = vec![-1i32; data.len()];
    let mut cluster = -1i32;
    for sample in analysis.iter() {
        let reachability = (*sample.reachability_distance()).unwrap_or(f64::INFINITY);
        let core = (*sample.core_distance()).unwrap_or(f64::INFINITY);
        let index = sample.index();
        if reachability > DENSITY_EPS {
            if core <= DENSITY_EPS {
                cluster += 1;
                labels[index] = cluster;
            }
        } else if cluster >= 0 {
            labels[index] = cluster;
        }
    }
    Ok(labels)
}

/// `linfa-clustering` Gaussian mixture model.
fn run_linfa_gmm(data: &SyntheticData, k: usize, seed: u64) -> Result<Vec<i32>, String> {
    let dataset = dataset(&data.points);
    let model = GaussianMixtureModel::<f64>::params_with_rng(k, StdRng::seed_from_u64(seed))
        .n_runs(3)
        .max_n_iterations(200)
        .tolerance(1e-4)
        .fit(&dataset)
        .map_err(|error| format!("linfa GMM fit failed: {error}"))?;
    Ok(model
        .predict(&dataset)
        .iter()
        .map(|&label| label as i32)
        .collect())
}

fn dense_matrix(points: &[[f64; 2]]) -> Result<smartcore::linalg::basic::matrix::DenseMatrix<f64>, String> {
    let rows: Vec<Vec<f64>> = points.iter().map(|point| point.to_vec()).collect();
    smartcore::linalg::basic::matrix::DenseMatrix::from_2d_vec(&rows)
        .map_err(|error| format!("smartcore DenseMatrix build failed: {error}"))
}

/// `smartcore` K-Means. The crate's K-Means has no centroid getter, so
/// centroids are recomputed from the returned labels in [`summarize`].
fn run_smartcore_kmeans(data: &SyntheticData, k: usize, seed: u64) -> Result<Vec<i32>, String> {
    use smartcore::cluster::kmeans::{KMeans as SmartKMeans, KMeansParameters};
    use smartcore::linalg::basic::matrix::DenseMatrix;

    let matrix = dense_matrix(&data.points)?;
    let parameters = KMeansParameters {
        k,
        max_iter: 100,
        seed: Some(seed),
    };
    let model = SmartKMeans::<f64, i32, DenseMatrix<f64>, Vec<i32>>::fit(&matrix, parameters)
        .map_err(|error| format!("smartcore KMeans fit failed: {error}"))?;
    model
        .predict(&matrix)
        .map_err(|error| format!("smartcore KMeans predict failed: {error}"))
}

/// `smartcore` DBSCAN.
///
/// `predict` returns `0` for noise and `1..=k` for clusters; the labels are
/// normalised to `-1` / `0..k` so every algorithm's summary is comparable.
fn run_smartcore_dbscan(data: &SyntheticData) -> Result<Vec<i32>, String> {
    use smartcore::cluster::dbscan::{DBSCANParameters, DBSCAN};
    use smartcore::linalg::basic::matrix::DenseMatrix;
    use smartcore::metrics::distance::euclidian::Euclidian;

    let matrix = dense_matrix(&data.points)?;
    let parameters = DBSCANParameters::default()
        .with_eps(DENSITY_EPS)
        .with_min_samples(DENSITY_MIN_POINTS);
    let model = DBSCAN::<f64, i32, DenseMatrix<f64>, Vec<i32>, Euclidian<f64>>::fit(
        &matrix, parameters,
    )
    .map_err(|error| format!("smartcore DBSCAN fit failed: {error}"))?;
    let labels = model
        .predict(&matrix)
        .map_err(|error| format!("smartcore DBSCAN predict failed: {error}"))?;
    Ok(labels
        .into_iter()
        .map(|label| if label == 0 { -1 } else { label - 1 })
        .collect())
}

/// The `hdbscan` crate. Version 0.12 spells the constructor
/// `Hdbscan::default_hyper_params(&data)` + `.cluster()`; there is no
/// `Hdbscan::default().build(..)` in this release.
fn run_hdbscan(data: &SyntheticData) -> Result<Vec<i32>, String> {
    use hdbscan::{Hdbscan, HdbscanHyperParams};

    let rows: Vec<Vec<f64>> = data.points.iter().map(|point| point.to_vec()).collect();
    let hyper_params = HdbscanHyperParams::builder()
        .min_cluster_size(DENSITY_MIN_POINTS)
        .min_samples(DENSITY_MIN_POINTS)
        .build();
    Hdbscan::new(&rows, hyper_params)
        .cluster()
        .map_err(|error| format!("hdbscan failed: {error}"))
}

/// Counts clusters / noise and computes each cluster's centroid.
fn summarize(labels: &[i32], points: &[[f64; 2]]) -> (Vec<Group>, usize) {
    let mut groups: BTreeMap<i32, (usize, [f64; 2])> = BTreeMap::new();
    let mut noise = 0usize;
    for (index, &label) in labels.iter().enumerate() {
        if label < 0 {
            noise += 1;
            continue;
        }
        let point = points[index];
        let entry = groups.entry(label).or_insert((0, [0.0, 0.0]));
        entry.0 += 1;
        entry.1[0] += point[0];
        entry.1[1] += point[1];
    }
    let groups = groups
        .into_iter()
        .map(|(label, (count, sums))| Group {
            label,
            count,
            centroid: [sums[0] / count as f64, sums[1] / count as f64],
        })
        .collect();
    (groups, noise)
}

/// Mean silhouette coefficient over the labelled points.
///
/// `-1` labels are noise and are not scored. The calculation is the textbook
/// one (mean intra-cluster distance `a`, smallest mean distance to another
/// cluster `b`, `s = (b - a) / max(a, b)`); above [`SILHOUETTE_SAMPLE_LIMIT`]
/// points it deterministically samples evenly spaced indices, keeping the
/// O(n²) distance matrix bounded. Returns `0.0` when fewer than two clusters
/// exist or when there is nothing to score.
pub fn silhouette(points: &[[f64; 2]], labels: &[i32]) -> f64 {
    let labelled: Vec<usize> = (0..points.len()).filter(|&index| labels[index] >= 0).collect();
    let mut cluster_sizes: BTreeMap<i32, usize> = BTreeMap::new();
    for &index in &labelled {
        *cluster_sizes.entry(labels[index]).or_insert(0) += 1;
    }
    if cluster_sizes.len() < 2 || labelled.is_empty() {
        return 0.0;
    }

    let stride = (labelled.len() / SILHOUETTE_SAMPLE_LIMIT).max(1);
    let sample: Vec<usize> = labelled.iter().copied().step_by(stride).collect();
    let mut total = 0.0;
    let mut scored = 0usize;

    for &index in &sample {
        let own = labels[index];
        let own_size = cluster_sizes[&own];
        if own_size <= 1 {
            continue;
        }
        let mut sums: BTreeMap<i32, f64> = BTreeMap::new();
        for &other in &labelled {
            if other == index {
                continue;
            }
            let distance = euclidean(points[index], points[other]);
            *sums.entry(labels[other]).or_insert(0.0) += distance;
        }
        let a = sums[&own] / (own_size - 1) as f64;
        let b = sums
            .iter()
            .filter(|(label, _)| **label != own)
            .map(|(label, sum)| sum / cluster_sizes[label] as f64)
            .fold(f64::INFINITY, f64::min);
        if !a.is_finite() || !b.is_finite() {
            continue;
        }
        let denominator = a.max(b);
        if denominator > 0.0 {
            total += (b - a) / denominator;
            scored += 1;
        }
    }

    if scored == 0 {
        0.0
    } else {
        total / scored as f64
    }
}

fn euclidean(left: [f64; 2], right: [f64; 2]) -> f64 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_data_is_deterministic_and_exact() {
        let first = synthetic(200, 3, 7);
        let second = synthetic(200, 3, 7);
        assert_eq!(first.points, second.points);
        assert_eq!(first.truth, second.truth);
        assert_eq!(first.outliers, second.outliers);
        assert_eq!(first.len(), 200);
        assert_eq!(first.outliers.iter().filter(|&&outlier| outlier).count(), 10);
        assert_eq!(first.truth.iter().filter(|&&blob| blob != usize::MAX).count(), 190);

        let other = synthetic(200, 3, 8);
        assert_ne!(first.points, other.points);
    }

    #[test]
    fn silhouette_is_high_for_separated_points() {
        let points = vec![[0.0, 0.0], [1.0, 0.0], [10.0, 0.0], [11.0, 0.0]];
        let score = silhouette(&points, &[0, 0, 1, 1]);
        assert!(score > 0.85, "expected a near-perfect score, got {score}");

        // One cluster, or only noise, cannot be scored.
        assert_eq!(silhouette(&points, &[0, 0, 0, 0]), 0.0);
        assert_eq!(silhouette(&points, &[-1, -1, -1, -1]), 0.0);
    }

    #[test]
    fn every_algorithm_returns_labels_for_the_same_cloud() {
        let data = synthetic(120, 3, 11);
        for algo in Algo::ALL {
            if algo == Algo::Kmeans && !cfg!(feature = "nightly-kmeans") {
                // Upstream `kmeans` 2.0.2 requires nightly features.
                continue;
            }
            let labels = match algo {
                Algo::Kmeans => {
                    #[cfg(feature = "nightly-kmeans")]
                    {
                        run_kmeans_crate(&data, 3, 11)
                    }
                    #[cfg(not(feature = "nightly-kmeans"))]
                    {
                        unreachable!("the loop skips Kmeans without the feature")
                    }
                }
                Algo::LinfaKmeans => run_linfa_kmeans(&data, 3, 11),
                Algo::LinfaDbscan => run_linfa_dbscan(&data),
                Algo::LinfaOptics => run_linfa_optics(&data),
                Algo::LinfaGmm => run_linfa_gmm(&data, 3, 11),
                Algo::SmartcoreKmeans => run_smartcore_kmeans(&data, 3, 11),
                Algo::SmartcoreDbscan => run_smartcore_dbscan(&data),
                Algo::Hdbscan => run_hdbscan(&data),
            }
            .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));
            assert_eq!(labels.len(), data.len(), "{}", algo.name());
            assert!(
                labels.iter().any(|&label| label >= 0),
                "{} labelled every point as noise",
                algo.name()
            );
        }
    }

    #[test]
    fn every_available_algorithm_finds_at_least_two_clusters() {
        for algo in Algo::ALL {
            if algo == Algo::Kmeans && !cfg!(feature = "nightly-kmeans") {
                continue;
            }
            let outcome = execute(algo, 150, 3, 7)
                .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));
            assert_eq!(outcome.points, 150, "{}", algo.name());
            assert!(
                outcome.clusters >= 2,
                "{} found only {} clusters",
                algo.name(),
                outcome.clusters
            );
            assert!(outcome.silhouette.is_finite(), "{}", algo.name());
        }
    }

    #[test]
    fn same_seed_produces_the_same_cluster_and_noise_counts() {
        for algo in Algo::ALL {
            if algo == Algo::Kmeans && !cfg!(feature = "nightly-kmeans") {
                continue;
            }
            let first = execute(algo, 150, 3, 7)
                .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));
            let second = execute(algo, 150, 3, 7)
                .unwrap_or_else(|error| panic!("{} failed: {error}", algo.name()));
            assert_eq!(first.clusters, second.clusters, "{}", algo.name());
            assert_eq!(first.noise, second.noise, "{}", algo.name());
        }
    }

    #[test]
    fn kmeans_silhouette_beats_half() {
        let linfa = execute(Algo::LinfaKmeans, 600, 3, 42).expect("linfa KMeans");
        assert!(
            linfa.silhouette > 0.5,
            "linfa KMeans silhouette {:.3}",
            linfa.silhouette
        );

        #[cfg(feature = "nightly-kmeans")]
        {
            let crate_kmeans = execute(Algo::Kmeans, 600, 3, 42).expect("kmeans crate");
            assert!(
                crate_kmeans.silhouette > 0.5,
                "kmeans crate silhouette {:.3}",
                crate_kmeans.silhouette
            );
        }
    }
}
