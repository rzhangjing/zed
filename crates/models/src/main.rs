//! CLI entry point: `models train` / `models test` / `models cluster`.

use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use models::cluster::{self, Algo};
use models::train::{self, DEFAULT_CHECKPOINT};

#[derive(Parser, Debug)]
#[command(
    name = "models",
    about = "Burn CNN + Transformer demo on synthetic 8x8 images",
    long_about = "A CNN front-end (2x Conv2d + ReLU + MaxPool) followed by one Transformer \
encoder layer, trained on deterministic synthetic 8x8 images (no dataset download), \
plus pure-Rust clustering demos over one deterministic 2-D point cloud."
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
    /// Run the pure-Rust clustering demos on one deterministic point cloud.
    Cluster {
        /// Algorithm name, or `all` for the comparison table.
        #[arg(
            long,
            default_value = "all",
            value_name = "ALGO",
            help = "kmeans | linfa-kmeans | linfa-dbscan | linfa-optics | linfa-gmm | \
smartcore-kmeans | smartcore-dbscan | hdbscan | all"
        )]
        algo: String,
        /// Number of synthetic points (3 Gaussian blobs + outliers).
        #[arg(long, default_value_t = 600)]
        points: usize,
        /// Number of blobs to generate (K-Means/GMM are told this `k`).
        #[arg(long, default_value_t = 3)]
        clusters: usize,
        /// Seed for the point cloud and every seeded algorithm.
        #[arg(long, default_value_t = 42)]
        seed: u64,
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
        Command::Cluster {
            algo,
            points,
            clusters,
            seed,
        } => {
            if algo.eq_ignore_ascii_case("all") {
                println!("{}", cluster::run_all(points, clusters, seed));
                Ok(())
            } else {
                let algo = Algo::parse(&algo).ok_or_else(|| {
                    std::io::Error::other(format!(
                        "unknown clustering algorithm `{algo}`; expected one of \
                         kmeans, linfa-kmeans, linfa-dbscan, linfa-optics, linfa-gmm, \
                         smartcore-kmeans, smartcore-dbscan, hdbscan, all"
                    ))
                })?;
                println!(
                    "{}",
                    cluster::run(algo, points, clusters, seed).map_err(std::io::Error::other)?
                );
                Ok(())
            }
        }
    }
}
