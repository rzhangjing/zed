//! M3.4 —— 批量截图脚本（`CESIUM_SCREENSHOT_SCRIPT`）。
//!
//! 将单次截图 `CESIUM_SCREENSHOT_AT_FRAME` 测试框架扩展为
//! 有序的多视角捕获。一个 TOML 脚本列出 `[[shot]]` 条目，每个
//! 携带要捕获的帧、该帧应用的确定性相机姿态
//!（`pos`/`quat`/`fov_y`）以及输出 `name`。
//!
//! 相机姿态 schema 与单次截图测试框架已写入的 `.camera.json` **逐字节一致**
//!（`{"pos":[x,y,z],"quat":[x,y,z,w],`
//! "fov_y":<deg>}`），因此脚本条目可与捕获的元数据往返。
//!
//! # 确定性
//!
//! 姿态在 `PostUpdate`（orbit 相机的 `Update` 之后）应用，因此无论
//! orbit 状态如何，脚本化变换在该帧渲染中生效 —— 不与 `orbit_camera`
//! 内部耦合。每个截图还会发出一个 `.meta.json`，盖上帧 / 姿态 / env / git SHA
//! 戳记，用于 `pixel_diff` 门槛。
//!
//! 本模块是纯解析 + 调度（无 Bevy、无 GPU），因此完全
//! 可无头单元测试。

use serde::Deserialize;
use std::path::Path;

/// 确定性相机姿态。对应 `.camera.json` schema：
/// `pos = [x, y, z]`、`quat = [x, y, z, w]`、`fov_y` 以**度**为单位。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CameraPose {
    /// 相机平移（渲染单位；1 渲染单位 = 6378137 米）。
    pub pos: [f64; 3],
    /// 相机朝向，四元数 `[x, y, z, w]`。
    pub quat: [f64; 4],
    /// 垂直视场角（度，可选；缺省时保持当前值）。
    #[serde(default)]
    pub fov_y: Option<f64>,
}

/// 截图脚本的 `[meta]` 块（可选，仅作描述）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ScriptMeta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// 一个 `[[shot]]` 条目：在 `frame`、从此姿态、捕获到 `name`。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ShotEntry {
    /// 要捕获的帧索引（从 1 开始，与测试框架计数器一致）。
    pub frame: u32,
    /// 输出基础名（无扩展名）→ 捕获输出目录下的 `{name}.png` / `{name}.camera.json`
    /// / `{name}.meta.json`。
    pub name: String,
    /// 相机平移 `[x, y, z]`。
    pub pos: [f64; 3],
    /// 相机朝向四元数 `[x, y, z, w]`。
    pub quat: [f64; 4],
    /// 垂直 FOV（度，可选）。
    #[serde(default)]
    pub fov_y: Option<f64>,
}

impl ShotEntry {
    /// 将条目姿态作为 [`CameraPose`]（固定相机 schema），因此脚本
    /// 条目可通过与 `FIXED_CAMERA` 相同的路径重新应用。
    #[allow(dead_code)]
    pub fn pose(&self) -> CameraPose {
        CameraPose {
            pos: self.pos,
            quat: self.quat,
            fov_y: self.fov_y,
        }
    }
}

/// 已解析的截图脚本：可选 `[meta]` + 有序 `[[shot]]` 列表。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ShotScript {
    #[serde(default)]
    pub meta: Option<ScriptMeta>,
    #[serde(default)]
    pub shot: Vec<ShotEntry>,
}

impl ShotScript {
    /// 从 TOML 字符串解析脚本。
    ///
    /// # 错误
    /// TOML 格式错误时返回人类可读的消息。
    pub fn from_toml_str(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("parse screenshot script: {e}"))
    }

    /// 从磁盘读取并解析脚本。
    ///
    /// # 错误
    /// IO 或解析失败时返回人类可读的消息。
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_toml_str(&text)
    }

    /// 按 `frame` 升序排列截图以实现确定性调度（乱序编写的脚本
    /// 仍会按帧顺序触发）。
    pub fn sorted(mut self) -> Self {
        self.shot.sort_by_key(|e| e.frame);
        self
    }
}

/// 纯调度辅助函数：给定按帧排序的截图、下一个未触发索引 `cursor` 和
/// 当前 `frame`，返回下一个到期截图的索引
///（`frame >= shot.frame`），若尚无到期则返回 `None`。
///
/// 使用 `>=`（而非 `==`），因此恰好被跳过帧的截图仍会
/// 触发且仅触发一次 —— 无论是否掉帧，每个条目都会被捕获。
pub fn next_due(shots: &[ShotEntry], cursor: usize, frame: u32) -> Option<usize> {
    shots
        .iter()
        .enumerate()
        .skip(cursor)
        .find(|(_, e)| frame >= e.frame)
        .map(|(i, _)| i)
}

/// 为单个捕获截图构建 `.meta.json` 载荷：触发的帧、脚本姿态、
/// git SHA 以及离线/env 快照（`env`），以便审阅者
/// 可复现精确的捕获。绝不会失败（美化打印一个
/// `serde_json` 值）。
pub fn metadata_json(
    entry: &ShotEntry,
    captured_frame: u32,
    git_sha: &str,
    env: &serde_json::Value,
) -> String {
    let meta = serde_json::json!({
        "name": entry.name,
        "scripted_frame": entry.frame,
        "captured_frame": captured_frame,
        "pos": entry.pos,
        "quat": entry.quat,
        "fov_y": entry.fov_y,
        "git_sha": git_sha,
        "env": env,
    });
    serde_json::to_string_pretty(&meta).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[meta]
name = "baseline_v0"
description = "v0 six-view baseline"

[[shot]]
frame = 90
name = "globe_default"
pos = [0.0, 0.0, 3.0]
quat = [0.0, 0.0, 0.0, 1.0]
fov_y = 60.0

[[shot]]
frame = 120
name = "pole_north"
pos = [0.0, 0.0, 3.0]
quat = [0.7071, 0.0, 0.0, 0.7071]
"#;

    #[test]
    fn parses_sample_script() {
        let s = ShotScript::from_toml_str(SAMPLE).unwrap();
        assert_eq!(s.meta.as_ref().and_then(|m| m.name.clone()).as_deref(), Some("baseline_v0"));
        assert_eq!(s.shot.len(), 2);
        assert_eq!(s.shot[0].name, "globe_default");
        assert_eq!(s.shot[0].frame, 90);
        assert_eq!(s.shot[0].pos, [0.0, 0.0, 3.0]);
        assert_eq!(s.shot[0].quat, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.shot[0].fov_y, Some(60.0));
        // fov_y 是可选的。
        assert_eq!(s.shot[1].fov_y, None);
    }

    #[test]
    fn pose_round_trips_entry() {
        let s = ShotScript::from_toml_str(SAMPLE).unwrap();
        let p = s.shot[0].pose();
        assert_eq!(p.pos, s.shot[0].pos);
        assert_eq!(p.quat, s.shot[0].quat);
        assert_eq!(p.fov_y, s.shot[0].fov_y);
    }

    #[test]
    fn sorted_orders_by_frame() {
        let s = ShotScript::from_toml_str(SAMPLE).unwrap().sorted();
        assert_eq!(s.shot[0].frame, 90);
        assert_eq!(s.shot[1].frame, 120);
    }

    #[test]
    fn rejects_malformed_toml() {
        let err = ShotScript::from_toml_str("[[shot]]\nframe = \"not-a-number\"\n").unwrap_err();
        assert!(err.contains("parse screenshot script"));
    }

    #[test]
    fn empty_script_has_no_shots() {
        let s = ShotScript::from_toml_str("").unwrap();
        assert!(s.shot.is_empty());
        assert!(s.meta.is_none());
    }

    #[test]
    fn next_due_fires_each_entry_once_in_order() {
        let s = ShotScript::from_toml_str(SAMPLE).unwrap().sorted();
        // 帧 90 之前：无到期。
        assert_eq!(next_due(&s.shot, 0, 89), None);
        // 帧 90：条目 0 到期。
        assert_eq!(next_due(&s.shot, 0, 90), Some(0));
        // 触发 0 后（cursor=1），帧 91 到 120 之前无到期。
        assert_eq!(next_due(&s.shot, 1, 91), None);
        assert_eq!(next_due(&s.shot, 1, 120), Some(1));
        // 两者均已触发（cursor=2）：无剩余。
        assert_eq!(next_due(&s.shot, 2, 999), None);
    }

    #[test]
    fn next_due_fires_skipped_frame_late() {
        let s = ShotScript::from_toml_str(SAMPLE).unwrap().sorted();
        // 若帧 90 被错过，在帧 95 条目 0 仍会触发（>= 语义）。
        assert_eq!(next_due(&s.shot, 0, 95), Some(0));
    }

    #[test]
    fn metadata_contains_frame_sha_and_env() {
        let s = ShotScript::from_toml_str(SAMPLE).unwrap();
        let env = serde_json::json!({"strict_offline": true, "offline_imagery_root": "x"});
        let meta = metadata_json(&s.shot[0], 90, "abc1234", &env);
        assert!(meta.contains("\"captured_frame\": 90"));
        assert!(meta.contains("\"git_sha\": \"abc1234\""));
        assert!(meta.contains("\"strict_offline\": true"));
        assert!(meta.contains("globe_default"));
        // 必须是有效 JSON。
        assert!(serde_json::from_str::<serde_json::Value>(&meta).is_ok());
    }

    // 保护已签入的 v0 基线免遭意外损坏：真实的
    // `specs/scripts/baseline_v0.toml` 必须解析为恰好六个
    // M0.1 视角，按帧升序，且四元数已归一化。
    #[test]
    fn baseline_v0_toml_is_valid_six_view_script() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../specs/scripts/baseline_v0.toml");
        let s = ShotScript::from_file(&path)
            .unwrap_or_else(|e| panic!("baseline_v0.toml must parse: {e}"));
        assert_eq!(
            s.meta.as_ref().and_then(|m| m.name.clone()).as_deref(),
            Some("baseline_v0")
        );
        let names: Vec<&str> = s.shot.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "globe_default",
                "pole_north",
                "pole_south",
                "zoom_mid",
                "zoom_close",
                "terrain_off"
            ]
        );
        // 帧严格升序（确定性调度顺序）。
        assert!(s.shot.windows(2).all(|w| w[0].frame < w[1].frame));
        // 单位四元数（有效旋转）+ 与 CAMERA_FOV_Y 一致的 60° FOV。
        for e in &s.shot {
            let [x, y, z, w] = e.quat;
            let norm = (x * x + y * y + z * z + w * w).sqrt();
            assert!((norm - 1.0).abs() < 1e-3, "{}: quat not unit ({norm})", e.name);
            assert_eq!(e.fov_y, Some(60.0));
        }
    }
}
