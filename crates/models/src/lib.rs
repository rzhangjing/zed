//! Burn demo: a small CNN + Transformer hybrid trained on deterministic
//! synthetic 8x8 images.
//!
//! The binary exposes two subcommands through `clap`:
//!
//! ```text
//! models train --epochs 2 --seed 42   # train, print loss/accuracy, save checkpoint
//! models test                         # load checkpoint, print accuracy + confusion matrix
//! ```
//!
//! Layout:
//!
//! - [`data`]: seeded synthetic data (no downloads, no files).
//! - [`model`]: the `CnnTransformer` module and its `TrainStep` implementation.
//! - [`train`]: training / evaluation loops and checkpoint handling.

pub mod data;
pub mod model;
pub mod train;

pub use data::{Dataset, IMAGE_SIZE, NUM_CLASSES};
pub use model::{Batch, CnnTransformer};
pub use train::{EvalResult, TestArgs, TrainArgs};
