//! Training / evaluation loop and checkpoint handling.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

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

/// Arguments of the `train` subcommand.
#[derive(Debug, Clone)]
pub struct TrainArgs {
    pub epochs: usize,
    pub batch_size: usize,
    pub seed: u64,
    pub learning_rate: f64,
    pub checkpoint: PathBuf,
}

/// Arguments of the `test` subcommand.
#[derive(Debug, Clone)]
pub struct TestArgs {
    pub seed: u64,
    pub checkpoint: PathBuf,
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

/// Accuracy and confusion matrix of one evaluation pass.
#[derive(Debug, Clone)]
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
    /// Render the confusion matrix as plain text.
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
}

/// Run `model` over a whole split and collect accuracy / confusion.
///
/// Predictions are taken to the host once per batch and reduced in plain Rust:
/// with 8x8 images and four classes this is cheaper than building an argmax
/// graph, and it keeps the confusion matrix independent of the tensor API.
pub fn evaluate(
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
pub fn train(args: &TrainArgs) -> Result<(), Box<dyn Error>> {
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

    println!("device           : {device:?}");
    println!("parameters       : {}", model.num_params());
    for (layer, params) in model.layer_summary() {
        println!("  {layer:<26} {params:>7}");
    }
    println!(
        "data             : train {} / test {} samples, {} classes, seed {}",
        train_set.len(),
        test_set.len(),
        NUM_CLASSES,
        args.seed
    );
    println!(
        "optimizer        : Adam(lr={}), batch size {}, {} epoch(s)",
        args.learning_rate, args.batch_size, args.epochs
    );

    let started = Instant::now();
    for epoch in 1..=args.epochs {
        let epoch_started = Instant::now();
        let mut loss_sum = 0.0f64;
        let mut batches = 0usize;

        for (start, end) in train_set.batches(args.batch_size) {
            let batch = batch(&train_images, &train_labels, start, end);
            let TrainOutput { grads, item } = model.step(batch);
            loss_sum += item.loss.clone().into_scalar::<f32>() as f64;
            batches += 1;
            model = model.optimize(&mut optim, args.learning_rate.into(), grads);
        }

        let eval = evaluate(&model, &test_set, &device, 64);
        println!(
            "epoch {}/{} | train loss {:.4} | test accuracy {:5.2}% | {:.1}s",
            epoch,
            args.epochs,
            loss_sum / batches.max(1) as f64,
            eval.accuracy * 100.0,
            epoch_started.elapsed().as_secs_f32()
        );
    }

    if let Some(parent) = args.checkpoint.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    model.clone().into_record().save(&args.checkpoint)?;
    println!(
        "checkpoint       : {} ({:.1} KiB)",
        args.checkpoint.display(),
        fs::metadata(&args.checkpoint)?.len() as f64 / 1024.0
    );
    println!("total time       : {:.1}s", started.elapsed().as_secs_f32());

    let eval = evaluate(&model, &test_set, &device, 64);
    println!("final test accuracy: {:.2}%", eval.accuracy * 100.0);
    Ok(())
}

/// Load a checkpoint and report accuracy plus the confusion matrix.
pub fn test(args: &TestArgs) -> Result<(), Box<dyn Error>> {
    if !args.checkpoint.exists() {
        return Err(format!(
            "checkpoint {} not found; run `models train` first",
            args.checkpoint.display()
        )
        .into());
    }

    let device = inference_device();
    let test_set = data::test_dataset(args.seed);
    let model = CnnTransformer::new(&device).try_load_file(&args.checkpoint)?;

    println!("device           : {device:?}");
    println!("checkpoint       : {}", args.checkpoint.display());
    println!("parameters       : {}", model.num_params());
    println!(
        "test set         : {} samples, seed {}",
        test_set.len(),
        args.seed
    );

    let started = Instant::now();
    let eval = evaluate(&model, &test_set, &device, 64);
    println!(
        "test accuracy    : {:.2}% ({} / {})",
        eval.accuracy * 100.0,
        eval.correct,
        eval.samples
    );
    println!();
    print!("{}", eval.confusion_text());
    println!("eval time        : {:.2}s", started.elapsed().as_secs_f32());
    Ok(())
}
