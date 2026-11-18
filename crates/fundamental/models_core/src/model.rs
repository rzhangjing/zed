//! `CnnTransformer`: a small CNN front-end followed by one Transformer encoder layer.
//!
//! Burn 0.22 selects the execution backend through [`Device`] instead of a
//! `B: Backend` type parameter, so the model is a plain struct deriving
//! [`Module`] and every layer is a concrete module type.

use burn::nn::attention::{MhaInput, MultiHeadAttention, MultiHeadAttentionConfig};
use burn::nn::conv::{Conv2d, Conv2dConfig};
use burn::nn::loss::CrossEntropyLossConfig;
use burn::nn::pool::{MaxPool2d, MaxPool2dConfig};
use burn::nn::{Dropout, DropoutConfig, LayerNorm, LayerNormConfig, Linear, LinearConfig, PaddingConfig2d, Relu};
use burn::prelude::*;
use burn::train::{ClassificationOutput, TrainOutput, TrainStep};

use crate::data;

/// Input image side (square).
pub const IMAGE_SIZE: usize = data::IMAGE_SIZE;
/// Input channels.
pub const IN_CHANNELS: usize = data::CHANNELS;
/// Output classes.
pub const NUM_CLASSES: usize = data::NUM_CLASSES;
/// Channels produced by the first convolution.
pub const CONV1_CHANNELS: usize = 8;
/// Channels produced by the second convolution.
pub const CONV2_CHANNELS: usize = 16;
/// Transformer width (`d_model`); also the channel count fed to it.
pub const D_MODEL: usize = CONV2_CHANNELS;
/// Attention heads (each head sees `D_MODEL / N_HEADS = 4` features).
pub const N_HEADS: usize = 4;
/// Feed-forward hidden width inside the encoder layer.
pub const D_FF: usize = 32;
/// Spatial positions after the two 2x2 pools: `(8/2/2)^2`.
pub const TOKENS: usize = 4;
/// Dropout probability. Zero keeps training and evaluation identical and the
/// demo reproducible; raise it to regularise.
pub const DROPOUT: f64 = 0.0;

/// One batch of images and their class indices, on the training device.
#[derive(Clone, Debug)]
pub struct Batch {
    /// `[batch, 1, 8, 8]`
    pub images: Tensor<4>,
    /// `[batch]` class indices in `0..NUM_CLASSES`.
    pub targets: Tensor<1, Int>,
}

/// CNN front-end + 1-layer Transformer encoder classifier.
///
/// Shapes: `[batch, 1, 8, 8]` -> conv1 `[batch, 8, 8, 8]` -> pool `[batch, 8, 4, 4]`
/// -> conv2 `[batch, 16, 4, 4]` -> pool `[batch, 16, 2, 2]`
/// -> tokens `[batch, 4, 16]` -> encoder `[batch, 4, 16]`
/// -> mean-pool `[batch, 16]` -> logits `[batch, 4]`.
#[derive(Module, Debug)]
pub struct CnnTransformer {
    conv1: Conv2d,
    relu1: Relu,
    conv2: Conv2d,
    relu2: Relu,
    pool: MaxPool2d,
    attn: MultiHeadAttention,
    norm1: LayerNorm,
    ffn1: Linear,
    ffn2: Linear,
    relu3: Relu,
    dropout: Dropout,
    norm2: LayerNorm,
    classifier: Linear,
}

impl CnnTransformer {
    /// Build the model with freshly initialised parameters on `device`.
    pub fn new(device: &Device) -> Self {
        // `Same` padding keeps the 8x8 grid until the pooled 2x2 token grid.
        let conv1 = Conv2dConfig::new([IN_CHANNELS, CONV1_CHANNELS], [3, 3])
            .with_padding(PaddingConfig2d::Same)
            .init(device);
        let conv2 = Conv2dConfig::new([CONV1_CHANNELS, CONV2_CHANNELS], [3, 3])
            .with_padding(PaddingConfig2d::Same)
            .init(device);

        Self {
            conv1,
            relu1: Relu::new(),
            conv2,
            relu2: Relu::new(),
            pool: MaxPool2dConfig::new([2, 2]).init(),
            attn: MultiHeadAttentionConfig::new(D_MODEL, N_HEADS)
                .with_dropout(DROPOUT)
                .init(device),
            norm1: LayerNormConfig::new(D_MODEL).init(device),
            ffn1: LinearConfig::new(D_MODEL, D_FF).init(device),
            ffn2: LinearConfig::new(D_FF, D_MODEL).init(device),
            relu3: Relu::new(),
            dropout: DropoutConfig::new(DROPOUT).init(),
            norm2: LayerNormConfig::new(D_MODEL).init(device),
            classifier: LinearConfig::new(D_MODEL, NUM_CLASSES).init(device),
        }
    }

    /// CNN front-end: two conv/pool blocks, returned as `[batch, 16, 2, 2]`.
    fn features(&self, input: Tensor<4>) -> Tensor<4> {
        let x = self.conv1.forward(input);
        let x = self.relu1.forward(x);
        let x = self.pool.forward(x);
        let x = self.conv2.forward(x);
        let x = self.relu2.forward(x);
        self.pool.forward(x)
    }

    /// One Transformer encoder layer (post-norm): self-attention residual, then
    /// position-wise feed-forward residual.
    fn encode(&self, tokens: Tensor<3>) -> Tensor<3> {
        let attn = self.attn.forward(MhaInput::self_attn(tokens.clone())).context;
        let x = self.norm1.forward(tokens + attn);

        let ff = self.ffn1.forward(x.clone());
        let ff = self.relu3.forward(ff);
        let ff = self.ffn2.forward(ff);
        let ff = self.dropout.forward(ff);
        self.norm2.forward(x + ff)
    }

    /// Full forward pass: `[batch, 1, 8, 8]` -> logits `[batch, 4]`.
    pub fn forward(&self, input: Tensor<4>) -> Tensor<2> {
        let [batch, _, _, _] = input.dims();

        // [batch, 16, 2, 2] -> [batch, 16, 4] -> [batch, 4, 16]: one token per
        // spatial position, its channel vector as the embedding.
        let x = self.features(input);
        let x = x.reshape([batch, CONV2_CHANNELS, TOKENS]);
        let tokens = x.permute([0, 2, 1]);

        let encoded = self.encode(tokens);
        let pooled = encoded.mean_dim(1).reshape([batch, D_MODEL]);
        self.classifier.forward(pooled)
    }

    /// Forward pass plus cross-entropy loss for one batch.
    pub fn forward_classification(&self, batch: Batch) -> ClassificationOutput {
        let logits = self.forward(batch.images);
        let device = logits.device();
        let loss = CrossEntropyLossConfig::new()
            .init(&device)
            .forward(logits.clone(), batch.targets.clone());
        ClassificationOutput::new(loss, logits, batch.targets)
    }

    /// Parameter count per layer, in forward order (for the training summary).
    pub fn layer_summary(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("conv1 (1->8, 3x3)", self.conv1.num_params()),
            ("pool1 (2x2)", self.pool.num_params()),
            ("conv2 (8->16, 3x3)", self.conv2.num_params()),
            ("pool2 (2x2)", self.pool.num_params()),
            ("mha (d_model=16, heads=4)", self.attn.num_params()),
            ("norm1", self.norm1.num_params()),
            ("ffn (16->32->16)", self.ffn1.num_params() + self.ffn2.num_params()),
            ("norm2", self.norm2.num_params()),
            ("classifier (16->4)", self.classifier.num_params()),
        ]
    }
}

impl TrainStep for CnnTransformer {
    type Input = Batch;
    type Output = ClassificationOutput;

    fn step(&self, batch: Batch) -> TrainOutput<Self::Output> {
        let output = self.forward_classification(batch);
        let grads = output.loss.backward();
        TrainOutput::new(self, grads, output)
    }
}
