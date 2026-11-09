//! Burn demo as a library: a small CNN + Transformer hybrid trained on
//! deterministic synthetic 8x8 images, plus the pure-Rust clustering demos.
//!
//! This crate is the single implementation behind two front-ends:
//!
//! * `crates/models` — the standalone CLI (`models train|test|cluster`), kept as
//!   a thin wrapper so its own workspace and Burn vendor tree keep working;
//! * `crates/framework/zed/src/zed/model_test.rs` — the Model Test window, which
//!   now calls [`train`], [`evaluate`] and [`run_cluster`] **in process** on a
//!   background executor instead of spawning `models.exe`.
//!
//! The API is thread-safe in the usual Rust sense: it owns no global state, the
//! inputs are plain data, and the only cross-thread object is the
//! `Arc<AtomicBool>` cancellation token. Progress is reported through a callback
//! that runs on the calling thread; the caller decides which thread that is.
//!
//! ```no_run
//! use std::sync::atomic::AtomicBool;
//! use std::sync::Arc;
//! use models_core::{TrainArgs, train};
//!
//! let cancel = Arc::new(AtomicBool::new(false));
//! let mut progress = |line: &str| println!("{line}");
//! let summary = train(
//!     &TrainArgs::new(2, 32, 42, 3e-3, "artifacts/model.bpk"),
//!     &mut progress,
//!     &cancel,
//! )?;
//! println!("final accuracy: {:.2}%", summary.final_accuracy * 100.0);
//! # Ok::<(), models_core::ModelsError>(())
//! ```
//!
//! Layout:
//!
//! - [`data`]: seeded synthetic data (no downloads, no files).
//! - [`model`]: the `CnnTransformer` module and its `TrainStep` implementation.
//! - [`train`]: training / evaluation loops, cancellation and checkpoints.
//! - [`cluster`]: K-Means / DBSCAN / OPTICS / GMM / HDBSCAN demos over one cloud.

pub mod cluster;
pub mod data;
pub mod model;
pub mod train;

pub use cluster::{Algo, ClusterSummary, Outcome, run_cluster};
pub use data::{Dataset, IMAGE_SIZE, NUM_CLASSES};
pub use model::{Batch, CnnTransformer};
pub use train::{
    DEFAULT_CHECKPOINT, EvalResult, ModelsError, TestArgs, TrainArgs, TrainSummary, evaluate,
    train,
};
