//! Integration tests for the Burn demo library.
//!
//! All tests run on the pure-Rust CPU backend and stay well under a second each
//! once the crate is compiled:
//!
//! 1. `forward_shape` — the model maps `[2, 1, 8, 8]` to `[2, 4]`.
//! 2. `synthetic_data_is_deterministic` — same seed, same bytes; different seed, different data.
//! 3. `checkpoint_round_trip_preserves_parameters` — save/load reproduces the record exactly.
//! 4. `single_training_step` — one `TrainStep` forward/backward/optimize cycle runs.
//! 5. `cancelled_training_returns_immediately` — a pre-set token stops before any work.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use burn::optim::AdamConfig;
use burn::prelude::*;
use burn::train::TrainStep;

use models_core::data::{self, Dataset};
use models_core::model::CnnTransformer;
use models_core::train::{
    DEFAULT_CHECKPOINT, TrainArgs, batch, image_tensor, inference_device, label_tensor,
    training_device, train,
};

fn tmp_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).expect("create target tmp dir");
    dir
}

fn fixed_images(device: &Device) -> Tensor<4> {
    let pixels: Vec<f32> = (0..2 * data::CHANNELS * data::IMAGE_SIZE * data::IMAGE_SIZE)
        .map(|i| (i % 17) as f32 / 16.0)
        .collect();
    Tensor::<4>::from_data(
        TensorData::new(
            pixels,
            [2, data::CHANNELS, data::IMAGE_SIZE, data::IMAGE_SIZE],
        ),
        device,
    )
}

fn slice_images(dataset: &Dataset, device: &Device, start: usize, end: usize) -> Tensor<4> {
    image_tensor(dataset, device).slice([
        start..end,
        0..data::CHANNELS,
        0..data::IMAGE_SIZE,
        0..data::IMAGE_SIZE,
    ])
}

#[test]
fn forward_shape() {
    let device = inference_device();
    let model = CnnTransformer::new(&device);

    let logits = model.forward(fixed_images(&device));

    assert_eq!(logits.dims(), [2, data::NUM_CLASSES]);
    assert!(
        model.num_params() > 0,
        "the model must own trainable parameters"
    );
}

#[test]
fn synthetic_data_is_deterministic() {
    let first = data::train_dataset(7);
    let second = data::train_dataset(7);
    assert_eq!(first, second, "same seed must produce the same split");

    // The split is balanced and has the documented size.
    assert_eq!(first.len(), data::TRAIN_SAMPLES);
    assert_eq!(
        first.class_histogram(),
        [data::TRAIN_SAMPLES / data::NUM_CLASSES; data::NUM_CLASSES]
    );

    // A different seed produces different pixels.
    let other = data::train_dataset(8);
    assert_ne!(first.labels, vec![0i64; first.len()]);
    assert_ne!(first.images, other.images, "different seeds must differ");

    // Train and test splits are disjoint streams of the same seed.
    let test = data::test_dataset(7);
    assert_eq!(test.len(), data::TEST_SAMPLES);
    let pixels = data::CHANNELS * data::IMAGE_SIZE * data::IMAGE_SIZE;
    assert_ne!(first.images[..pixels], test.images[..pixels]);
    assert_eq!(test, data::test_dataset(7));
}

#[test]
fn checkpoint_round_trip_preserves_parameters() {
    let device = inference_device();
    let model = CnnTransformer::new(&device);
    // The workspace's own `target/` tmp dir, never the repository tree or the
    // system temp directory.
    let dir = tmp_dir();
    let original = dir.join(format!(
        "models-core-roundtrip-original-{}.bpk",
        std::process::id()
    ));
    let reloaded = dir.join(format!(
        "models-core-roundtrip-reloaded-{}.bpk",
        std::process::id()
    ));

    model.clone().into_record().save(&original).expect("save");
    let loaded = CnnTransformer::new(&device)
        .try_load_file(&original)
        .expect("load");
    loaded.clone().into_record().save(&reloaded).expect("re-save");

    // A fresh model has different parameters, so identical records mean the
    // checkpoint actually carried every parameter across.
    let before = std::fs::read(&original).expect("read original");
    let after = std::fs::read(&reloaded).expect("read reloaded");
    assert!(!before.is_empty());
    assert_eq!(before, after, "record bytes must survive save -> load");

    // ... and the forward pass agrees.
    let input = fixed_images(&device);
    let a = model
        .forward(input.clone())
        .into_data()
        .try_to_vec::<f32>()
        .expect("logits");
    let b = loaded
        .forward(input)
        .into_data()
        .try_to_vec::<f32>()
        .expect("logits");
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert!((x - y).abs() < 1e-6, "logits differ: {x} vs {y}");
    }

    let _ = std::fs::remove_file(&original);
    let _ = std::fs::remove_file(&reloaded);
}

#[test]
fn single_training_step() {
    let device = training_device();
    let mut model = CnnTransformer::new(&device);
    let dataset = data::train_dataset(3);

    let images = slice_images(&dataset, &device, 0, 8);
    let labels = label_tensor(&dataset, &device).slice([0..8]);

    let output = model.step(models_core::Batch {
        images,
        targets: labels,
    });
    let loss = output.item.loss.clone().into_scalar::<f32>();
    assert!(loss.is_finite(), "loss must be finite, got {loss}");
    assert!(loss > 0.0, "cross entropy starts above zero");

    let mut optim = AdamConfig::new().init();
    model = model.optimize(&mut optim, 3e-3.into(), output.grads);

    let eval = models_core::train::evaluate_dataset(&model, &dataset, &device, 64);
    assert_eq!(eval.samples, dataset.len());
    assert!((0.0..=1.0).contains(&eval.accuracy));
    let total: usize = eval.confusion.iter().flatten().sum();
    assert_eq!(total, dataset.len(), "confusion matrix must cover the split");
}

#[test]
fn batch_helper_covers_every_sample_once() {
    let dataset = data::train_dataset(11);
    let ranges = dataset.batches(32);
    let covered: usize = ranges.iter().map(|(start, end)| end - start).sum();
    assert_eq!(covered, dataset.len());
    assert!(ranges.iter().all(|(start, end)| start < end));

    let device = inference_device();
    let images = image_tensor(&dataset, &device);
    let labels = label_tensor(&dataset, &device);
    let first = batch(&images, &labels, ranges[0].0, ranges[0].1);
    assert_eq!(first.images.dims(), [32, 1, 8, 8]);
    assert_eq!(first.targets.dims(), [32]);
}

#[test]
fn cancelled_training_returns_immediately() {
    let cancel = AtomicBool::new(true);
    let mut lines = Vec::new();
    let args = TrainArgs::new(2, 32, 42, 3e-3, DEFAULT_CHECKPOINT);
    let error = train(&args, &mut |line| lines.push(line.to_owned()), &cancel)
        .expect_err("a pre-set token must cancel before training starts");
    assert!(error.is_cancelled());
    assert!(lines.is_empty(), "cancelled before the first progress line");
}

#[test]
fn cancelling_mid_training_stops_before_the_checkpoint() {
    let dir = tmp_dir();
    let checkpoint = dir.join(format!("models-core-cancel-{}.bpk", std::process::id()));
    let _ = std::fs::remove_file(&checkpoint);

    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker_checkpoint = checkpoint.clone();
    let worker = std::thread::spawn(move || {
        let args = TrainArgs::new(100, 32, 42, 3e-3, worker_checkpoint);
        let mut lines = Vec::new();
        let result = train(&args, &mut |line| lines.push(line.to_owned()), &worker_cancel);
        (result, lines)
    });

    // Let the worker enter training, then flip the token from this thread.
    std::thread::sleep(Duration::from_millis(150));
    cancel.store(true, Ordering::SeqCst);

    let (result, lines) = worker.join().expect("training thread panicked");
    let error = result.expect_err("a cancelled run must return Err");
    assert!(error.is_cancelled(), "{error}");
    assert!(
        !checkpoint.exists(),
        "a cancelled run must not write a checkpoint"
    );
    assert!(
        lines.iter().any(|line| line.contains("parameters")),
        "the run must have started before it was cancelled"
    );
}
