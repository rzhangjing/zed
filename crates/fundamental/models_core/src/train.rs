//! Training / evaluation loop, cancellation and checkpoint handling.
//!
//! Everything here is callable from any thread: the inputs are plain data, the
//! progress callback runs on the calling thread, and the only shared mutable
//! state is the caller's `AtomicBool` cancellation token. The training loop
//! checks it before every epoch and before every mini-batch, so a cancelled run
//! returns [`ModelsError::cancelled`] instead of writing a half-trained
//! checkpoint.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use burn::optim::AdamConfig;
use burn::prelude::*;
use burn::train::TrainOutput;
use burn::train::TrainStep;

use crate::data::{self, Dataset};
use crate::model::{Batch, CnnTransformer, IMAGE_SIZE, NUM_CLASSES};

/// Default checkpoint written by `train` and read by `test`.
///
/// Burn 0.22 dropped the MessagePack (`.mpk`) recorders in favour of
/// burnpack, so the checkpoint is a burnpack file (`.bpk`) holding the same
/// module record (parameter path + `ParamId` + tensor data).
pub const DEFAULT_CHECKPOINT: &str = "artifacts/model.bpk";

/// Arguments of the `train` entry point.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrainArgs {
    pub epochs: usize,
    pub batch_size: usize,
    pub seed: u64,
    pub learning_rate: f64,
    pub checkpoint: PathBuf,
}

impl TrainArgs {
    /// Build arguments with the demo defaults for everything but the five
    /// parameters listed here.
    pub fn new(
        epochs: usize,
        batch_size: usize,
        seed: u64,
        learning_rate: f64,
        checkpoint: impl Into<PathBuf>,
    ) -> Self {
        Self {
            epochs,
            batch_size,
            seed,
            learning_rate,
            checkpoint: checkpoint.into(),
        }
    }
}

impl Default for TrainArgs {
    fn default() -> Self {
        Self {
            epochs: 3,
            batch_size: 32,
            seed: 42,
            learning_rate: 3e-3,
            checkpoint: PathBuf::from(DEFAULT_CHECKPOINT),
        }
    }
}

/// Arguments of the `evaluate` entry point (the CLI's `test` subcommand).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TestArgs {
    pub seed: u64,
    pub checkpoint: PathBuf,
}

impl TestArgs {
    pub fn new(seed: u64, checkpoint: impl Into<PathBuf>) -> Self {
        Self {
            seed,
            checkpoint: checkpoint.into(),
        }
    }
}

impl Default for TestArgs {
    fn default() -> Self {
        Self {
            seed: 42,
            checkpoint: PathBuf::from(DEFAULT_CHECKPOINT),
        }
    }
}

/// Result of a completed training run.
#[derive(Debug, Clone)]
pub struct TrainSummary {
    /// Epochs actually run.
    pub epochs: usize,
    /// Test accuracy after the last epoch, in `0..=1`.
    pub final_accuracy: f32,
    /// Full evaluation of the test split after training.
    pub final_eval: EvalResult,
    /// Where the checkpoint was written.
    pub checkpoint: PathBuf,
    /// Size of the checkpoint on disk.
    pub checkpoint_bytes: u64,
    /// Wall-clock time of the whole run.
    pub elapsed: Duration,
}

/// Accuracy and confusion matrix of one evaluation pass.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvalResult {
    /// Fraction of correctly classified samples in `0..=1`.
    pub accuracy: f32,
    /// Number of correctly classified samples.
    pub correct: usize,
    /// `confusion[true][predicted]`
    pub confusion: [[usize; NUM_CLASSES]; NUM_CLASSES],
    /// Number of evaluated samples.
    pub samples: usize,
}

impl EvalResult {
    /// Render the confusion matrix as plain text (one line per true class).
    pub fn confusion_text(&self) -> String {
        let mut out = String::new();
        out.push_str("confusion matrix (rows = true class, columns = predicted class)\n");
        out.push_str("        pred0  pred1  pred2  pred3   total   recall\n");
        for (true_class, row) in self.confusion.iter().enumerate() {
            let total: usize = row.iter().sum();
            let correct = row[true_class];
            let recall = if total == 0 {
                0.0
            } else {
                correct as f32 / total as f32
            };
            out.push_str(&format!(
                "true{true_class}  {:>5}  {:>5}  {:>5}  {:>5}   {:>5}  {:>5.1}%\n",
                row[0],
                row[1],
                row[2],
                row[3],
                total,
                recall * 100.0
            ));
        }
        out
    }

    /// The same text as [`Self::confusion_text`], one `String` per line, so a
    /// log ring can show one row per entry.
    pub fn confusion_lines(&self) -> impl Iterator<Item = String> + '_ {
        self.confusion_text()
            .lines()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// A training/evaluation failure that is safe to move across threads.
///
/// Burn's own error types are not all `Send`, so they are rendered into a
/// message here; callers only need the text (the window logs it, the CLI prints
/// it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsError {
    message: String,
    cancelled: bool,
}

impl ModelsError {
    /// A regular failure with a human-readable message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cancelled: false,
        }
    }

    /// The run was stopped through its cancellation token.
    pub fn cancelled() -> Self {
        Self {
            message: "the run was cancelled".to_owned(),
            cancelled: true,
        }
    }

    /// Whether this error is the result of cancellation rather than a failure.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// The message shown to the user.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ModelsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ModelsError {}

impl From<String> for ModelsError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for ModelsError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<std::io::Error> for ModelsError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

/// `Err(cancelled)` when the caller asked to stop, `Ok(())` otherwise.
pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), ModelsError> {
    if cancel.load(Ordering::Relaxed) {
        Err(ModelsError::cancelled())
    } else {
        Ok(())
    }
}

/// Device used for training: the pure-Rust NdArray CPU backend with autodiff.
///
/// Burn 0.22 made `burn-ndarray` a (deprecated but working) backend that is
/// reached through burn's `ndarray` feature: no GPU / wgpu / CubeCL backend is
/// compiled in.
#[allow(deprecated)]
pub fn training_device() -> Device {
    Device::ndarray().autodiff()
}

/// Device used for inference: the same CPU backend without the autodiff decorator.
#[allow(deprecated)]
pub fn inference_device() -> Device {
    Device::ndarray()
}

/// Build the `[n, 1, 8, 8]` image tensor for a whole split.
pub fn image_tensor(dataset: &Dataset, device: &Device) -> Tensor<4> {
    let shape = [dataset.len(), data::CHANNELS, IMAGE_SIZE, IMAGE_SIZE];
    Tensor::<4>::from_data(TensorData::new(dataset.images.clone(), shape), device)
}

/// Build the `[n]` integer label tensor for a whole split.
pub fn label_tensor(dataset: &Dataset, device: &Device) -> Tensor<1, Int> {
    Tensor::<1, Int>::from_data(
        TensorData::new(dataset.labels.clone(), [dataset.len()]),
        device,
    )
}

/// Slice `start..end` out of the pre-built split tensors.
pub fn batch(images: &Tensor<4>, labels: &Tensor<1, Int>, start: usize, end: usize) -> Batch {
    Batch {
        images: images.clone().slice([
            start..end,
            0..data::CHANNELS,
            0..IMAGE_SIZE,
            0..IMAGE_SIZE,
        ]),
        targets: labels.clone().slice([start..end]),
    }
}

/// Run `model` over a whole split and collect accuracy / confusion.
///
/// Predictions are taken to the host once per batch and reduced in plain Rust:
/// with 8x8 images and four classes this is cheaper than building an argmax
/// graph, and it keeps the confusion matrix independent of the tensor API.
pub fn evaluate_dataset(
    model: &CnnTransformer,
    dataset: &Dataset,
    device: &Device,
    batch_size: usize,
) -> EvalResult {
    let images = image_tensor(dataset, device);
    let mut confusion = [[0usize; NUM_CLASSES]; NUM_CLASSES];
    let mut correct = 0usize;

    for (start, end) in dataset.batches(batch_size) {
        let logits = model.forward(images.clone().slice([
            start..end,
            0..data::CHANNELS,
            0..IMAGE_SIZE,
            0..IMAGE_SIZE,
        ]));
        let values = logits
            .into_data()
            .try_to_vec::<f32>()
            .expect("logits are f32");
        for row in 0..(end - start) {
            let scores = &values[row * NUM_CLASSES..(row + 1) * NUM_CLASSES];
            let mut predicted = 0usize;
            for class in 1..NUM_CLASSES {
                if scores[class] > scores[predicted] {
                    predicted = class;
                }
            }
            let actual = dataset.labels[start + row] as usize;
            confusion[actual][predicted] += 1;
            if actual == predicted {
                correct += 1;
            }
        }
    }

    let samples = dataset.len();
    let accuracy = if samples == 0 {
        0.0
    } else {
        correct as f32 / samples as f32
    };
    EvalResult {
        accuracy,
        correct,
        confusion,
        samples,
    }
}

/// Train the model and write a checkpoint.
///
/// `progress` receives one line per event (device summary, every epoch line,
/// checkpoint path, final accuracy); `cancel` is checked before every epoch and
/// every mini-batch. A cancelled run returns [`ModelsError::cancelled`] without
/// writing the checkpoint.
pub fn train(
    args: &TrainArgs,
    progress: &mut dyn FnMut(&str),
    cancel: &AtomicBool,
) -> Result<TrainSummary, ModelsError> {
    check_cancel(cancel)?;
    let started = Instant::now();
    let device = training_device();
    // Seed Burn's device RNG so parameter initialisation is reproducible too;
    // the synthetic data is seeded separately inside `data`.
    device.seed(args.seed);
    let train_set = data::train_dataset(args.seed);
    let test_set = data::test_dataset(args.seed);

    let train_images = image_tensor(&train_set, &device);
    let train_labels = label_tensor(&train_set, &device);

    let mut model = CnnTransformer::new(&device);
    let mut optim = AdamConfig::new().init();

    progress(&format!("device           : {device:?}"));
    progress(&format!("parameters       : {}", model.num_params()));
    for (layer, params) in model.layer_summary() {
        progress(&format!("  {layer:<26} {params:>7}"));
    }
    progress(&format!(
        "data             : train {} / test {} samples, {} classes, seed {}",
        train_set.len(),
        test_set.len(),
        NUM_CLASSES,
        args.seed
    ));
    progress(&format!(
        "optimizer        : Adam(lr={}), batch size {}, {} epoch(s)",
        args.learning_rate, args.batch_size, args.epochs
    ));

    for epoch in 1..=args.epochs {
        check_cancel(cancel)?;
        let epoch_started = Instant::now();
        let mut loss_sum = 0.0f64;
        let mut batches = 0usize;

        for (start, end) in train_set.batches(args.batch_size) {
            check_cancel(cancel)?;
            let batch = batch(&train_images, &train_labels, start, end);
            let TrainOutput { grads, item } = model.step(batch);
            loss_sum += item.loss.clone().into_scalar::<f32>() as f64;
            batches += 1;
            model = model.optimize(&mut optim, args.learning_rate.into(), grads);
        }

        let eval = evaluate_dataset(&model, &test_set, &device, 64);
        progress(&format!(
            "epoch {}/{} | train loss {:.4} | test accuracy {:5.2}% | {:.1}s",
            epoch,
            args.epochs,
            loss_sum / batches.max(1) as f64,
            eval.accuracy * 100.0,
            epoch_started.elapsed().as_secs_f32()
        ));
    }

    check_cancel(cancel)?;
    if let Some(parent) = args.checkpoint.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    model
        .clone()
        .into_record()
        .save(&args.checkpoint)
        .map_err(|error| {
            ModelsError::new(format!(
                "could not save checkpoint {}: {error}",
                args.checkpoint.display()
            ))
        })?;
    let checkpoint_bytes = fs::metadata(&args.checkpoint)?.len();
    progress(&format!(
        "checkpoint       : {} ({:.1} KiB)",
        args.checkpoint.display(),
        checkpoint_bytes as f64 / 1024.0
    ));
    progress(&format!(
        "total time       : {:.1}s",
        started.elapsed().as_secs_f32()
    ));

    let final_eval = evaluate_dataset(&model, &test_set, &device, 64);
    progress(&format!(
        "final test accuracy: {:.2}%",
        final_eval.accuracy * 100.0
    ));

    Ok(TrainSummary {
        epochs: args.epochs,
        final_accuracy: final_eval.accuracy,
        final_eval,
        checkpoint: args.checkpoint.clone(),
        checkpoint_bytes,
        elapsed: started.elapsed(),
    })
}

/// Load a checkpoint and report accuracy plus the confusion matrix.
///
/// This is the `test` subcommand's implementation and the Model Test window's
/// `Test` button; every line it would have printed is passed to `progress`.
pub fn evaluate(
    args: &TestArgs,
    progress: &mut dyn FnMut(&str),
) -> Result<EvalResult, ModelsError> {
    if !args.checkpoint.exists() {
        return Err(ModelsError::new(format!(
            "checkpoint {} not found; run `models train` first",
            args.checkpoint.display()
        )));
    }

    let started = Instant::now();
    let device = inference_device();
    let test_set = data::test_dataset(args.seed);
    let model = CnnTransformer::new(&device)
        .try_load_file(&args.checkpoint)
        .map_err(|error| {
            ModelsError::new(format!(
                "could not load checkpoint {}: {error}",
                args.checkpoint.display()
            ))
        })?;

    progress(&format!("device           : {device:?}"));
    progress(&format!(
        "checkpoint       : {}",
        args.checkpoint.display()
    ));
    progress(&format!("parameters       : {}", model.num_params()));
    progress(&format!(
        "test set         : {} samples, seed {}",
        test_set.len(),
        args.seed
    ));

    let eval = evaluate_dataset(&model, &test_set, &device, 64);
    progress(&format!(
        "test accuracy    : {:.2}% ({} / {})",
        eval.accuracy * 100.0,
        eval.correct,
        eval.samples
    ));
    for line in eval.confusion_lines() {
        progress(&line);
    }
    progress(&format!(
        "eval time        : {:.2}s",
        started.elapsed().as_secs_f32()
    ));
    Ok(eval)
}
