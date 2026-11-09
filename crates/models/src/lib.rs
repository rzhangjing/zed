//! Burn demo CLI: a small CNN + Transformer hybrid trained on deterministic
//! synthetic 8x8 images.
//!
//! The implementation lives in the root workspace's `models_core` crate, which
//! the Model Test window also calls in-process. This crate is only the `clap`
//! shell plus the re-exports its own tests use:
//!
//! ```text
//! models train --epochs 2 --seed 42   # train, print loss/accuracy, save checkpoint
//! models test                         # load checkpoint, print accuracy + confusion matrix
//! models cluster --algo all           # pure-Rust clustering demos, comparison table
//! ```

pub mod cluster {
    //! Re-export of [`models_core::cluster`] for the `models` CLI and its tests.
    pub use models_core::cluster::*;
}
pub mod data {
    pub use models_core::data::*;
}
pub mod model {
    pub use models_core::model::*;
}
pub mod train {
    pub use models_core::train::*;
}

pub use models_core::{
    Algo, Batch, CnnTransformer, ClusterSummary, Dataset, EvalResult, ModelsError, Outcome,
    TestArgs, TrainArgs, TrainSummary,
};
// `train` is the module above; the function is `models_core::train`, re-exported
// only through `models::train::train` so the two namespaces stay unambiguous.
pub use models_core::{evaluate, run_cluster};

use std::sync::atomic::AtomicBool;

/// Run `train` with a never-set cancellation token and print every progress line.
pub fn run_train(args: &TrainArgs) -> Result<TrainSummary, ModelsError> {
    let cancel = AtomicBool::new(false);
    models_core::train(args, &mut |line| println!("{line}"), &cancel)
}

/// Run `evaluate` and print every progress line.
pub fn run_evaluate(args: &TestArgs) -> Result<EvalResult, ModelsError> {
    models_core::evaluate(args, &mut |line| println!("{line}"))
}

/// Run one clustering algorithm (or all) and return the printable report.
///
/// Progress lines are suppressed: the CLI prints `summary.rendered`, which is
/// exactly the old `models cluster` payload (the comparison table followed by
/// each algorithm's multi-line summary).
pub fn run_cluster_cli(
    algo: Option<&str>,
    points: usize,
    clusters: usize,
    seed: u64,
) -> Result<ClusterSummary, ModelsError> {
    let cancel = AtomicBool::new(false);
    models_core::run_cluster(algo, points, clusters, seed, &mut |_| {}, &cancel)
}
