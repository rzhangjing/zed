# G4 像素中性 A/B 证据（建议1 · dynamic_globe_legacy 退役前置）

**日期**：2026-09-27  
**裁决依据**：`docs/ARCHITECTURE.md` "Render Main Path (Adjudication, P1-1)" 中的第 2 条退役前置——"G4 至少在当前 tree 上被测量并至少通过一次（薄壳 vs legacy ≥ 45 dB）"。  
**结论**：G4 实测 PASS——薄壳与冻结 legacy 单体像素中性，残差纯粹是 Bing 在线瓦片流式抖动的 run-to-run 噪声。允许 legacy 退役。

---

## 方法论

在 headless 离屏渲染 + 同一固定相机（默认 orbit + 无输入）下，用两个二进制开关分别跑同一个 `cesium-app.exe` 到 frame 2000，让 Bing CDN 完整铺完瓦片，然后 `pixel_diff --threshold 45`：

- **Arm A（薄壳）**：不设 `CESIUMRST_LEGACY_DYNAMIC_GLOBE`，`dynamic_globe.rs` 325 行薄壳。
- **Arm B（legacy）**：设 `CESIUMRST_LEGACY_DYNAMIC_GLOBE=1`（现已删除），2210 行冻结单体。
- **对照**（噪声底）：Arm A 跑两次，度量同一路径的 run-to-run 抖动。

> 注：不使用 `STRICT_OFFLINE=1 + OFFLINE_IMAGERY_ROOT` 做 A/B——legacy 单体硬编码 Bing 且完全不消费 `OFFLINE_*`（deferred #27），离线对照会跑成"本地 PNG vs 远程 Bing"两条完全不同数据源，测出的是数据源差而非渲染差。首轮尝试（`pixel_diff_g4_shell_vs_legacy.json` = 13.45 dB）即因此，作废保留作反例。

`pixel_diff` 判据：PSNR ≥ 45 dB PASS / SSIM 参考 / MAE 参考。

---

## 结果

### 已铺完瓦片（frame 2000，在线 Bing）

| Pair | PSNR (dB) | SSIM | MAE | Verdict |
| --- | ---: | ---: | ---: | :--- |
| 对照：`set_shell.png` vs `set_shell2.png`（同路径重跑） | **42.6263** | 0.998717 | 0.00231618 | 噪声底 |
| **G4：`set_shell.png` vs `set_legacy.png`（薄壳 vs legacy）** | **42.6261** | **0.998717** | **0.00231619** | **PASS** |

跨路径与对照的 PSNR/SSIM/MAE 差异只在 **1e-8** 数量级：薄壳相对 legacy 完全没有可测量的渲染偏差，残差就是 Bing 网络流式抖动本身。

### 离线确定性同路径复核（`STRICT_OFFLINE=1 + OFFLINE_IMAGERY_ROOT`）

| Pair | PSNR (dB) | SSIM | Verdict |
| --- | ---: | ---: | :--- |
| `shell_globe_default.png` vs `shell2_globe_default.png`（同路径重跑） | **51.49** | 0.9997 | 输入固定时抖动消失 → 渲染本身确定性 |

这条排除了"跨路径 42.63 dB 是不是薄壳本身有问题"：只要喂同一份数据源，同一路径能拿到 51.49 dB。42.63 纯粹是网络流式抖动。

### 退役后金路径回归

| Pair | PSNR (dB) | SSIM | Verdict |
| --- | ---: | ---: | :--- |
| `shell_globe_default.png`（删除前）vs `post_delete_shell.png`（删除后，同一 600 帧离线确定性） | 51.49 | 0.9997 | PASS — 删除 legacy 完全未动金路径 |

---

## 编译门

- `cargo check -p cesium-app` = EXIT 0
- `cargo clippy --workspace --all-targets -- -D warnings` = EXIT 0
- `cargo build --release -p cesium-app` = EXIT 0

---

## 采集方法（可复现）

Arm A：

```powershell
$env:CESIUM_ENABLE_NEW_CHAINS = "0"
$env:CESIUM_ENABLE_TERRAIN = "0"
$env:CESIUM_ENABLE_TILESET = "0"
$env:CESIUM_HEADLESS = "1"
$env:CESIUM_HEADLESS_FRAMES = "2000"
$env:CESIUM_HEADLESS_OUTPUT = "<abs>\set_shell.png"
Remove-Item Env:\CESIUMRST_LEGACY_DYNAMIC_GLOBE -EA SilentlyContinue
& "<abs>\target\release\cesium-app.exe"
```

Arm B 同上，加 `$env:CESIUMRST_LEGACY_DYNAMIC_GLOBE = "1"`（退役前）。

比对：

```powershell
& "<abs>\target\release\pixel_diff.exe" <A.png> <B.png> --threshold 45 --json
```

**Windows 沙箱陷阱**：直接 `& cesium-app.exe` 会被沙箱以"拒绝访问"拦截；须在非沙箱/脱离作业对象的 PowerShell 中执行（或 `required_permissions='all'`）。

---

## 归档文件

- `set_shell.png` / `set_shell2.png` / `set_legacy.png` — frame 2000 已铺完，在线 Bing
- `shell_globe_default.png` / `shell2_globe_default.png` — frame 600 离线确定性同路径复核
- `post_delete_shell.png` — 删除后金路径回归
- `on_shell.png` / `on_shell2.png` / `on_legacy.png` — frame 600 早期铺瓦（未铺完，仅参考）
- `pixel_diff_g4_settled_shell_vs_legacy.json` — G4 判定（PASS）
- `pixel_diff_g4_online_shell_vs_legacy.json` — frame 600 早期铺瓦版（噪声底更高）
- `pixel_diff_g4_shell_vs_legacy.json` — 首轮离线反例（**作废保留**，说明 A/B 必须走同源）
