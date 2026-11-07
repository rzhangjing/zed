//! CLI entry point: `models train` / `models test`.

use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use models::train::{self, DEFAULT_CHECKPOINT};

#[derive(Parser, Debug)]
#[command(
    name = "models",
    about = "Burn CNN + Transformer demo on synthetic 8x8 images",
    long_about = "A CNN front-end (2x Conv2d + ReLU + MaxPool) followed by one Transformer \
encoder layer, trained on deterministic synthetic 8x8 images (no dataset download)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Train the model and write a checkpoint.
    Train {
        /// Number of epochs.
        #[arg(long, default_value_t = 3)]
        epochs: usize,
        /// Mini-batch size.
        #[arg(long, default_value_t = 32)]
        batch_size: usize,
        /// Seed for the synthetic data and the initial parameters.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Adam learning rate.
        #[arg(long, default_value_t = 3e-3)]
        learning_rate: f64,
        /// Where to write the checkpoint.
        #[arg(long, default_value = DEFAULT_CHECKPOINT)]
        checkpoint: PathBuf,
    },
    /// Load a checkpoint and print test accuracy and the confusion matrix.
    Test {
        /// Seed for the synthetic test split (must match the training seed).
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Checkpoint to load.
        #[arg(long, default_value = DEFAULT_CHECKPOINT)]
        checkpoint: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Train {
            epochs,
            batch_size,
            seed,
            learning_rate,
            checkpoint,
        } => train::train(&train::TrainArgs {
            epochs,
            batch_size,
            seed,
            learning_rate,
            checkpoint,
        }),
        Command::Test { seed, checkpoint } => {
            train::test(&train::TestArgs { seed, checkpoint })
        }
    }
}
