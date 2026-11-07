# crates/models — Burn CNN + Transformer 训练/测试 demo

一个**独立的 cargo workspace**（根 `Cargo.toml` 的 `exclude` 中有 `"crates/models"`，不是根
workspace 的 member，也不使用根 `workspace.dependencies`）。用 Burn 在纯 CPU 上训练一个
「CNN 前端 + 1 层 Transformer 编码器」的 4 类 8×8 图像分类器，不需要下载任何数据集。

## 结构

```
crates/models/
├── .cargo/config.toml     # 本 crate 自己的 vendored-sources（指向 ./vendor）+ offline
├── .gitignore             # /target/ /vendor/ /artifacts/
├── Cargo.toml             # burn 0.22（core+train+dataset+autodiff+vision+store+ndarray）
├── Cargo.lock             # 离线解析用的锁文件（vendored 依赖与之一一对应）
├── artifacts/model.bpk    # train 产出的 checkpoint（gitignore）
├── vendor/                # 本 crate 专属的 vendored 依赖（gitignore，见下）
├── src/
│   ├── data.rs            # 确定性合成数据（4 类图案 + Box-Muller 噪声）
│   ├── model.rs           # CnnTransformer + TrainStep 实现
│   ├── train.rs           # 训练/评估循环、checkpoint 读写、混淆矩阵
│   ├── lib.rs
│   └── main.rs            # clap CLI：train / test
└── tests/demo.rs          # 前向形状 / 数据确定性 / checkpoint 往返 / 单步训练
```

## 依赖与离线

- Burn **0.22.1**，后端 `burn-ndarray` **0.22.1**（NDArray CPU，纯 Rust，无 GPU/wgpu/CubeCL 后端依赖）。
- 根 workspace 用 `vendor/` 做 crates-io 源替换，里面没有 Burn。因此本 crate 自带
  `vendor/`（`cargo vendor` 生成，约 **856 MB / 751 个 crate**，含 Burn 全特性锁文件里的
  可选后端源码，但**不会参与编译**），并在 `.cargo/config.toml` 里做源替换：

  ```toml
  [source.crates-io]
  replace-with = "vendored-sources"
  [source.vendored-sources]
  directory = "vendor"
  ```

  之后 `CARGO_NET_OFFLINE=true` 下 `cargo metadata/test/run` 全部可用。`vendor/`、`target/`、
  `artifacts/` 都已 gitignore，不入库。
- 重新生成 vendor（需要联网，例如 `Cargo.toml` 改版本后）：

  ```powershell
  # 在仓库外的临时目录里 cargo vendor 后复制回来，或临时移除 .cargo/config.toml 的源替换再 vendor
  ```

## 用法（cwd = `crates/models`）

```powershell
$env:CARGO_NET_OFFLINE='true'
cargo test --offline                                # 6 个测试
cargo run --offline -- train --epochs 2 --seed 42   # 训练并写出 artifacts/model.bpk
cargo run --offline -- test                         # 加载 checkpoint，打印 accuracy + 混淆矩阵
cargo run --offline -- --help
```

`train` 可调：`--epochs`（默认 3）、`--batch-size`（默认 32）、`--seed`（默认 42）、
`--learning-rate`（默认 3e-3）、`--checkpoint`。`--seed` 同时决定合成数据和 Burn 设备 RNG
（参数初始化），因此**同 seed 完全可复现**。

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
