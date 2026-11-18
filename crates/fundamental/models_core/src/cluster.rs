//! Thin bridge to the root-workspace [`clustering`] crate.
//!
//! The algorithms, the synthetic cloud, the silhouette implementation and the
//! rendering live in `crates/fundamental/clustering` (a member of the repository
//! workspace). This module only re-exports that surface and adds the
//! in-process, cancellable entry point the Model Test window and the `models
//! cluster` CLI share.
//!
//! `nightly-kmeans` is off unless this crate's matching feature is enabled:
//! `kmeans` 2.0.2 needs nightly features, so only the separate `crates/models`
//! workspace (which sets `RUSTC_BOOTSTRAP=1`) turns it on. Without the feature
//! [`clustering::execute`] returns a documented error for
//! `Algo::Kmeans` and the comparison table prints that row as an error instead
//! of aborting the other algorithms.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::train::ModelsError;

pub use clustering::{
    Algo, Group, Outcome, SyntheticData, execute, run, run_all, silhouette, synthetic,
    table_header,
};

/// The result of one [`run_cluster`] call.
#[derive(Debug, Clone)]
pub struct ClusterSummary {
    /// Algorithms that completed successfully, in run order.
    pub outcomes: Vec<Outcome>,
    /// Algorithms that failed, with the crate's error message.
    pub failures: Vec<(Algo, String)>,
    /// Header plus one line per algorithm (exactly what was sent to progress).
    pub table: String,
    /// Full printable report: the table followed by every successful
    /// algorithm's multi-line summary (the `models cluster` payload).
    pub rendered: String,
    /// Wall-clock time of the whole call.
    pub elapsed: Duration,
}

/// Run one clustering algorithm, or all eight, reporting one line per algorithm.
///
/// `algo` is `None` or `Some("all")` for the comparison table; any other value
/// is a [`Algo::parse`] name. The callback is invoked once for the table header
/// (when running all) and once per algorithm; cancellation is checked between
/// algorithms, so a `Stop` never waits for a whole table.
pub fn run_cluster(
    algo: Option<&str>,
    points: usize,
    clusters: usize,
    seed: u64,
    progress: &mut dyn FnMut(&str),
    cancel: &AtomicBool,
) -> Result<ClusterSummary, ModelsError> {
    let started = Instant::now();
    let run_all = algo.is_none_or(|value| value.eq_ignore_ascii_case("all"));
    let selected: Vec<Algo> = if run_all {
        Algo::ALL.to_vec()
    } else {
        let value = algo.expect("only `None` means all");
        match Algo::parse(value) {
            Some(algo) => vec![algo],
            None => {
                return Err(ModelsError::new(format!(
                    "unknown clustering algorithm `{value}`; expected one of \
                     kmeans, linfa-kmeans, linfa-dbscan, linfa-optics, linfa-gmm, \
                     smartcore-kmeans, smartcore-dbscan, hdbscan, all"
                )));
            }
        }
    };

    let mut table = String::new();
    if run_all {
        let header = format!(
            "synthetic data: {points} points, {clusters} requested clusters, seed {seed}\n\n{}",
            table_header()
        );
        for line in header.lines() {
            progress(line);
        }
        table.push_str(&header);
        table.push('\n');
    }

    let mut outcomes = Vec::new();
    let mut failures = Vec::new();
    for algo in selected {
        if cancel.load(Ordering::Relaxed) {
            return Err(ModelsError::cancelled());
        }
        match clustering::execute(algo, points, clusters, seed) {
            Ok(outcome) => {
                let row = outcome.table_row();
                progress(&row);
                table.push_str(&row);
                table.push('\n');
                outcomes.push(outcome);
            }
            Err(reason) => {
                let row = format!("{:<18} error: {reason}", algo.name());
                progress(&row);
                table.push_str(&row);
                table.push('\n');
                failures.push((algo, reason));
            }
        }
    }

    let mut rendered = table.trim_end().to_owned();
    if run_all {
        rendered.push_str("\n\n");
        for outcome in &outcomes {
            rendered.push_str(&outcome.render());
            rendered.push_str("\n\n");
        }
        rendered = rendered.trim_end().to_owned();
    } else if let Some(outcome) = outcomes.first() {
        rendered = outcome.render();
    }

    Ok(ClusterSummary {
        outcomes,
        failures,
        table,
        rendered,
        elapsed: started.elapsed(),
    })
}
