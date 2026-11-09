# crates/models — Burn CNN + Transformer 训练/测试 demo 的 CLI 壳

一个**独立的 cargo workspace**（根 `Cargo.toml` 的 `exclude` 中有 `"crates/models"`，不是根
workspace 的 member），只保留 `clap` CLI、本 crate 自己的 `Cargo.lock` / `vendor/`，以及
`RUSTC_BOOTSTRAP=1` 的 nightly-kmeans 开关。**模型、数据、训练/评估循环、checkpoint 与聚类
封装都在根 workspace 的 `crates/fundamental/models-core`**（package `models_core`）里；
本 crate 的 `lib.rs` 只是对它的薄封装，`main.rs` 只做参数解析。

这样只有一套逻辑：`models train|test|cluster`、Model Test 窗口（`crates/framework/zed/src/zed/model_test.rs`，
现在**进程内**调用 `models_core::train/evaluate/run_cluster`）和根 workspace 的测试都走
`models_core`。

聚类算法本身住在根 workspace 的 `crates/fundamental/clustering`（K-Means / DBSCAN /
OPTICS / GMM / HDBSCAN 与轮廓系数），`models_core` 通过根 `[workspace.dependencies]` 的
path 依赖调用它；本 crate 打开 `models_core/nightly-kmeans`，因此这里的
`models cluster --algo all` 仍是 8 个算法全部可用。

## 结构

```
crates/models/
├── .cargo/config.toml     # 本 crate 自己的 vendored-sources（指向 ./vendor）+ offline + RUSTC_BOOTSTRAP
├── .gitignore             # /target/ /vendor/ /artifacts/
├── Cargo.toml             # 只依赖 models_core（features = ["nightly-kmeans"]）+ clap
├── Cargo.lock             # 本 workspace 的离线锁文件（vendored 依赖与之一一对应）
├── artifacts/model.bpk    # train 产出的 checkpoint（gitignore，与 Model Test 窗口共用）
├── vendor/                # 本 crate 专属的 vendored 依赖（gitignore，见下）
├── src/
│   ├── lib.rs             # re-export models_core：data/model/train/cluster + run_* 辅助
│   └── main.rs            # clap CLI：train / test / cluster
└── tests/cluster.rs       # 各算法簇数 / 轮廓 / 同 seed 稳定（4 个测试）
```

真正的实现与测试在 `crates/fundamental/models-core`：

```
crates/fundamental/models-core/src/
├── data.rs                # 确定性合成数据（4 类图案 + Box-Muller 噪声）
├── model.rs               # CnnTransformer + TrainStep 实现
├── train.rs               # train/evaluate + 进度回调 + AtomicBool 取消 + checkpoint
├── cluster.rs             # run_cluster：每算法一行进度、取消、表格/完整报告
└── lib.rs
crates/fundamental/models-core/tests/
├── demo.rs                # 前向形状 / 数据确定性 / checkpoint 往返 / 单步训练 / 取消
└── cluster.rs             # 各算法簇数 / 轮廓 / 同 seed 稳定 / 8 行进度表
```

## 依赖与离线

- `models_core` 通过根 `[workspace.dependencies]` 使用 Burn **0.22.1** + `burn-ndarray`
  **0.22.1**（NDArray CPU，纯 Rust，无 GPU/wgpu/CubeCL 后端被编译），以及根表里的
  `clustering`、`serde`、`rand`、`ndarray`。
- 根 workspace 的 `vendor/` 现在也含 Burn 的依赖（从本 crate 的 `vendor/` 补缺复制）；
  本 crate 仍保留自己的 `vendor/`（约 **862 MB / 773 个 crate**，含 Burn 全特性锁文件里的
  可选后端源码，但**不会参与编译**），以便这个独立 workspace 有自己的锁文件和
  `RUSTC_BOOTSTRAP` 开关。
- `kmeans` 2.0.2 上游是 nightly-only（`#![feature(portable_simd)]`）。本 crate 的
  `.cargo/config.toml` 通过 `[env] RUSTC_BOOTSTRAP = "1"` 让稳定的 1.98.1 编译它；
  `models_core/nightly-kmeans` 由本 crate 打开，根 workspace 的默认门禁不编译它。

## 用法（cwd = `crates/models`）

```powershell
$env:CARGO_NET_OFFLINE='true'
cargo test --offline                                # 4 个聚类集成测试（模型测试在 models_core）
cargo run --offline -- train --epochs 2 --seed 42   # 训练并写出 artifacts/model.bpk
cargo run --offline -- test                         # 加载 checkpoint，打印 accuracy + 混淆矩阵
cargo run --offline -- cluster --algo all           # 8 个聚类算法对比表 + 每算法摘要
cargo run --offline -- cluster --algo linfa-dbscan  # 单个算法
cargo run --offline -- --help
```

`train` 可调：`--epochs`（默认 3）、`--batch-size`（默认 32）、`--seed`（默认 42）、
`--learning-rate`（默认 3e-3）、`--checkpoint`。`--seed` 同时决定合成数据和 Burn 设备 RNG
（参数初始化），因此**同 seed 完全可复现**。


`cluster` 可调：`--algo`（默认 `all`）、`--points`（默认 600）、`--clusters`（默认 3）、
`--seed`（默认 42）。算法名：`kmeans`、`linfa-kmeans`、`linfa-dbscan`、`linfa-optics`、
`linfa-gmm`、`smartcore-kmeans`、`smartcore-dbscan`、`hdbscan`、`all`。

## 模型

输入 `[batch, 1, 8, 8]`，共 **3540** 个参数：

| 层 | 输出 | 参数 |
| --- | --- | --- |
| Conv2d 1→8, 3×3, Same + ReLU | `[b, 8, 8, 8]` | 80 |
| MaxPool2d 2×2 | `[b, 8, 4, 4]` | 0 |
| Conv2d 8→16, 3×3, Same + ReLU | `[b, 16, 4, 4]` | 1168 |
| MaxPool2d 2×2 | `[b, 16, 2, 2]` | 0 |
| reshape/permute | `[b, 4, 16]`（4 个 token，d_model=16） | 0 |
| MultiHeadAttention（4 heads）+ 残差 + LayerNorm | `[b, 4, 16]` | 1088 |
| FFN 16→32→16 + ReLU + 残差 + LayerNorm | `[b, 4, 16]` | 1072 |
| mean-pool over tokens | `[b, 16]` | 0 |
| Linear 16→4 | `[b, 4]` logits | 68 |

## 关于 checkpoint 文件名

Burn 0.22 **移除了** MessagePack（`.mpk`）recorder（`NamedMpkFileRecorder` 等），`burn::store`
现在只有 burnpack / SafeTensors。因此 checkpoint 保存为 **`artifacts/model.bpk`**（burnpack
格式，内容是 module record：参数路径 + `ParamId` + 张量数据），而不是原计划的 `model.mpk`。
`Module::into_record().save()` / `Module::try_load_file()` 往返字节级一致（见测试）。
