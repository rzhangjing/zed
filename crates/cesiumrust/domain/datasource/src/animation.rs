//! 动画系统：属性动画、路径动画与路径可视化。
//!
//! 本模块提供三部分能力：以关键帧序列在时间轴上插值出连续位置的
//! `interpolate_position`（支持线性/Hermite/Lagrange 三种算法），驱动播放
//! 进度与循环的 `AnimationClock`，以及沿采样轨迹计算拖尾与前导路径点的
//! `compute_path`——后者把经纬度高程逐个转换到给定椭球下的 Cartesian3。

use cesium_geospatial::{Cartographic, Ellipsoid};

use crate::entity::Entity;
use crate::entity_collection::EntityCollection;
use crate::property::Property;

/// 动画中的一个关键帧。
///
/// 记录某一时刻及其对应的值，位置分量以 [lon_rad, lat_rad, height_m]
/// 表示；关键帧序列按时间递增排列，供插值在其间求值。
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    /// 自历元起以秒计的时间。
    pub time: f64,
    /// 此关键帧处的值（位置以 [lon_rad, lat_rad, height_m] 表示）。
    pub value: [f64; 3],
}

/// 动画的插值算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpolationAlgorithm {
    /// 线性插值。
    #[default]
    Linear,
    /// Hermite（三次）插值。
    Hermite,
    /// Lagrange 多项式插值。
    Lagrange,
}

/// 动画时钟状态。
///
/// 维护起止时间、当前时间与播放速率倍率，并以 tick 按帧推动当前时间；
/// 越界时根据 looping 决定回绕到另一端还是钉在边界并停止播放。
#[derive(Debug, Clone)]
pub struct AnimationClock {
    /// 起始时间（秒）。
    pub start_time: f64,
    /// 停止时间（秒）。
    pub stop_time: f64,
    /// 当前时间（秒）。
    pub current_time: f64,
    /// 播放速率倍率。
    pub multiplier: f64,
    /// 时钟是否正在播放。
    pub playing: bool,
    /// 是否循环。
    pub looping: bool,
}

impl AnimationClock {
    /// 创建新的动画时钟。
    pub fn new(start_time: f64, stop_time: f64) -> Self {
        Self {
            start_time,
            stop_time,
            current_time: start_time,
            multiplier: 1.0,
            playing: false,
            looping: true,
        }
    }

    /// 将时钟推进 delta_time 秒。
    ///
    /// 仅当处于播放态才前进；推进后若越过停止或起始边界，则依
    /// looping 对区间长度取模回绕，或钉在边界并置 playing 为假。
    pub fn tick(&mut self, delta_time: f64) {
        if !self.playing {
            return;
        }

        self.current_time += delta_time * self.multiplier;

        if self.current_time > self.stop_time {
            if self.looping {
                self.current_time = self.start_time
                    + (self.current_time - self.start_time) % (self.stop_time - self.start_time);
            } else {
                self.current_time = self.stop_time;
                self.playing = false;
            }
        } else if self.current_time < self.start_time {
            if self.looping {
                let range = self.stop_time - self.start_time;
                self.current_time = self.stop_time - (self.start_time - self.current_time) % range;
            } else {
                self.current_time = self.start_time;
                self.playing = false;
            }
        }
    }

    /// 归一化的进度（0.0 到 1.0）。
    ///
    /// 以当前时间在起止区间内的占比计算；当区间退化（长度近零）
    /// 时直接返回 0.0 以避免除零。
    pub fn progress(&self) -> f64 {
        if (self.stop_time - self.start_time).abs() < f64::EPSILON {
            return 0.0;
        }
        (self.current_time - self.start_time) / (self.stop_time - self.start_time)
    }

    /// 重置到起点。
    pub fn reset(&mut self) {
        self.current_time = self.start_time;
    }

    /// 定位到特定时间。
    ///
    /// 将目标时间钉到 [start_time, stop_time] 区间内写入当前时间，
    /// 不改变播放态。
    pub fn seek(&mut self, time: f64) {
        self.current_time = time.clamp(self.start_time, self.stop_time);
    }
}

/// 从关键帧在给定的时间处插值出一个位置。
///
/// 先处理空序列与单帧的退化情形，再定位相邻关键帧并对区间外的
/// 时间钳到端值；随后按所选算法在相邻帧间求值：线性直接比例插值，
/// Hermite 用零切线的 smoothstep，Lagrange 取至多四个邻点构造基函数。
pub fn interpolate_position(
    keyframes: &[Keyframe],
    time: f64,
    algorithm: InterpolationAlgorithm,
) -> Option<[f64; 3]> {
    if keyframes.is_empty() {
        return None;
    }
    if keyframes.len() == 1 {
        return Some(keyframes[0].value);
    }

    // 查找相邻的关键帧
    let mut prev_idx = 0;
    for (i, kf) in keyframes.iter().enumerate() {
        if kf.time > time {
            break;
        }
        prev_idx = i;
    }

    // 在第一个之前或正好在第一个
    if time <= keyframes[0].time {
        return Some(keyframes[0].value);
    }
    // 在最后一个之后或正好在最后一个
    if time >= keyframes[keyframes.len() - 1].time {
        return Some(keyframes[keyframes.len() - 1].value);
    }

    let next_idx = (prev_idx + 1).min(keyframes.len() - 1);
    // 取相邻两帧及其时间差，时间差为零时直接回退到前帧值。
    let prev = &keyframes[prev_idx];
    let next = &keyframes[next_idx];

    let dt = next.time - prev.time;
    if dt.abs() < f64::EPSILON {
        return Some(prev.value);
    }

    // 归一化局部参数 t∈[0,1]，表示查询时刻在 prev→next 区间的位置。
    let t = (time - prev.time) / dt;

    // 按所选算法在相邻帧间求值。
    match algorithm {
        InterpolationAlgorithm::Linear => {
            Some([
                prev.value[0] + t * (next.value[0] - prev.value[0]),
                prev.value[1] + t * (next.value[1] - prev.value[1]),
                prev.value[2] + t * (next.value[2] - prev.value[2]),
            ])
        }
        InterpolationAlgorithm::Hermite => {
            // 零切线的三次 Hermite（smoothstep 平滑）
            let t2 = t * t;
            let t3 = t2 * t;
            let h = 3.0 * t2 - 2.0 * t3; // smoothstep
            // 逐分量用 smoothstep 系数 h 在 prev→next 间插值，起止处切线为零。
            Some([
                prev.value[0] + h * (next.value[0] - prev.value[0]),
                prev.value[1] + h * (next.value[1] - prev.value[1]),
                prev.value[2] + h * (next.value[2] - prev.value[2]),
            ])
        }
        InterpolationAlgorithm::Lagrange => {
            // 为 Lagrange 使用至多 4 个相邻点
            let start = prev_idx.saturating_sub(1);
            let end = (next_idx + 2).min(keyframes.len());
            let points: Vec<&Keyframe> = keyframes[start..end].iter().collect();

            if points.len() < 3 {
                // 回退到线性
                return Some([
                    prev.value[0] + t * (next.value[0] - prev.value[0]),
                    prev.value[1] + t * (next.value[1] - prev.value[1]),
                    prev.value[2] + t * (next.value[2] - prev.value[2]),
                ]);
            }

            // 累加各 Lagrange 基函数乘以对应样本分量的贡献。
            let mut result = [0.0; 3];
            for (i, pi) in points.iter().enumerate() {
                let mut basis = 1.0;
                for (j, pj) in points.iter().enumerate() {
                    if i != j {
                        let denom = pi.time - pj.time;
                        if denom.abs() > f64::EPSILON {
                            basis *= (time - pj.time) / denom;
                        }
                    }
                }
                result[0] += basis * pi.value[0];
                result[1] += basis * pi.value[1];
                result[2] += basis * pi.value[2];
            }
            Some(result)
        }
    }
}

/// Cartesian3 中的一个路径拖尾点。
#[derive(Debug, Clone)]
pub struct PathPoint {
    /// 以 Cartesian3 [x, y, z] 表示的位置。
    pub position: [f64; 3],
    /// 此点处的时间。
    pub time: f64,
}

/// 计算给定时间处实体的拖尾/前导路径。
///
/// 静态（常量位置）实体只产出一个点；采样位置则先在拖尾区间
/// `[time - trail_time, time]`、再在前导区间 `[time, time + lead_time]` 上
/// 按 resolution 步长线性插值，逐点转到椭球直角坐标后返回。
pub fn compute_path(
    entity: &Entity,
    time: f64,
    lead_time: f64,
    trail_time: f64,
    resolution: f64,
    ellipsoid: &Ellipsoid,
) -> Vec<PathPoint> {
    let mut path = Vec::new();

    // 实体无采样序列时无法构轨，静态或空位置分支处理。

    // 从实体获取位置样本
    let samples = match &entity.position {
        Property::Sampled(s) => s,
        Property::Constant(pos) => {
            // 静态实体 - 无路径
            let cart = ellipsoid.cartographic_to_cartesian(
                &Cartographic::from_radians(pos[0], pos[1], pos[2]),
            );
            path.push(PathPoint {
                position: [cart.x, cart.y, cart.z],
                time,
            });
            return path;
        }
        Property::Undefined => return path,
    };

    if samples.is_empty() {
        return path;
    }

    // 由样本计算关键帧
    let keyframes: Vec<Keyframe> = samples
        .iter()
        .map(|(t, pos)| Keyframe { time: *t, value: *pos })
        .collect();

    // 拖尾：从 (time - trail_time) 到 time
    // 以 resolution 为步长逐时刻插值并转到直角坐标。
    let trail_start = time - trail_time;
    let mut t = trail_start;
    while t <= time {
        if let Some(pos) = interpolate_position(&keyframes, t, InterpolationAlgorithm::Linear) {
            let cart = ellipsoid.cartographic_to_cartesian(
                &Cartographic::from_radians(pos[0], pos[1], pos[2]),
            );
            path.push(PathPoint {
                position: [cart.x, cart.y, cart.z],
                time: t,
            });
        }
        t += resolution;
    }

    // 前导：从 time 到 (time + lead_time)
    let lead_end = time + lead_time;
    t = time + resolution;
    while t <= lead_end {
        if let Some(pos) = interpolate_position(&keyframes, t, InterpolationAlgorithm::Linear) {
            let cart = ellipsoid.cartographic_to_cartesian(
                &Cartographic::from_radians(pos[0], pos[1], pos[2]),
            );
            path.push(PathPoint {
                position: [cart.x, cart.y, cart.z],
                time: t,
            });
        }
        t += resolution;
    }

    path
}

/// 更新所有带 path 图形的实体，计算它们的拖尾/前导路径。
///
/// 先过滤出可见且含 path 图形的实体，逐项读取前导/拖尾时长与分辨率
/// 后调用 compute_path，仅保留非空结果并以实体 id 连同路径点集返回。
pub fn update_all_paths(
    entities: &EntityCollection,
    time: f64,
    ellipsoid: &Ellipsoid,
) -> Vec<(String, Vec<PathPoint>)> {
    entities
        .values()
        .filter(|e| e.show && e.path.is_some())
        .filter_map(|entity| {
            let path_graphics = entity.path.as_ref().unwrap();
            let lead_time = path_graphics.lead_time.get_value(time).copied().unwrap_or(0.0);
            let trail_time = path_graphics.trail_time.get_value(time).copied().unwrap_or(0.0);
            let resolution = path_graphics.resolution.get_value(time).copied().unwrap_or(60.0);

            let path = compute_path(entity, time, lead_time, trail_time, resolution, ellipsoid);
            if path.is_empty() {
                None
            } else {
                Some((entity.id.clone(), path))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::PathGraphics;

    /// 验证播放态时钟逐帧推进：倍率为 1 时每次 tick 10 秒，
    /// 当前时间依次累加到 10 与 20。
    #[test]
    fn test_animation_clock_tick() {
        let mut clock = AnimationClock::new(0.0, 100.0);
        clock.playing = true;
        clock.multiplier = 1.0;

        clock.tick(10.0);
        assert!((clock.current_time - 10.0).abs() < 1e-10);

        clock.tick(10.0);
        assert!((clock.current_time - 20.0).abs() < 1e-10);
    }

    /// 验证循环回绕：超过停止时间时按区间长度取模，110 秒回绕
    /// 到起点后的 10 秒处。
    #[test]
    fn test_animation_clock_loop() {
        let mut clock = AnimationClock::new(0.0, 100.0);
        clock.playing = true;
        clock.looping = true;

        clock.tick(110.0);
        assert!((clock.current_time - 10.0).abs() < 1e-10);
    }

    /// 验证非循环时不取模：越过停止边界后钉在 100 秒并自动
    /// 停止播放。
    #[test]
    fn test_animation_clock_no_loop() {
        let mut clock = AnimationClock::new(0.0, 100.0);
        clock.playing = true;
        clock.looping = false;

        clock.tick(110.0);
        assert!((clock.current_time - 100.0).abs() < 1e-10);
        assert!(!clock.playing);
    }

    /// 验证进度归一化：当前时间处于起止中点时 progress 返回 0.5。
    /// 确保进度仅依赖时间位置而与倍率/播放态无关。
    #[test]
    fn test_animation_clock_progress() {
        let clock = AnimationClock {
            start_time: 0.0,
            stop_time: 100.0,
            current_time: 50.0,
            multiplier: 1.0,
            playing: true,
            looping: true,
        };
        assert!((clock.progress() - 0.5).abs() < 1e-10);
    }

    /// 验证线性插值：两帧之间取中点时刻 5 秒，各分量按半程比例
    /// 处于起点与终点之间。
    #[test]
    fn test_interpolate_linear() {
        let keyframes = vec![
            Keyframe { time: 0.0, value: [0.0, 0.0, 0.0] },
            Keyframe { time: 10.0, value: [10.0, 20.0, 30.0] },
        ];

        let pos = interpolate_position(&keyframes, 5.0, InterpolationAlgorithm::Linear).unwrap();
        assert!((pos[0] - 5.0).abs() < 1e-10);
        assert!((pos[1] - 10.0).abs() < 1e-10);
        assert!((pos[2] - 15.0).abs() < 1e-10);
    }

    /// 验证 Hermite(smoothstep) 插值：在中点处对称性仍给出 0.5 比例，
    /// 因此首分量为 5。
    #[test]
    fn test_interpolate_hermite() {
        let keyframes = vec![
            Keyframe { time: 0.0, value: [0.0, 0.0, 0.0] },
            Keyframe { time: 10.0, value: [10.0, 10.0, 10.0] },
        ];

        // 在中点处，Hermite（smoothstep）应给出 0.5
        let pos = interpolate_position(&keyframes, 5.0, InterpolationAlgorithm::Hermite).unwrap();
        assert!((pos[0] - 5.0).abs() < 1e-10);
    }

    /// 验证区间外钳位：时间早于首帧返回首值，晚于末帧返回末值，
    /// 实现两端的外推钉定。
    #[test]
    fn test_interpolate_boundaries() {
        let keyframes = vec![
            Keyframe { time: 0.0, value: [1.0, 2.0, 3.0] },
            Keyframe { time: 10.0, value: [10.0, 20.0, 30.0] },
        ];

        // 在开始之前
        let pos = interpolate_position(&keyframes, -5.0, InterpolationAlgorithm::Linear).unwrap();
        assert_eq!(pos, [1.0, 2.0, 3.0]);

        // 在结束之后
        let pos = interpolate_position(&keyframes, 15.0, InterpolationAlgorithm::Linear).unwrap();
        assert_eq!(pos, [10.0, 20.0, 30.0]);
    }

    /// 验证单帧退化：只有一个关键帧时无论查询时间都直接返回该帧值。
    /// 确保不会因无法构成区间而报错。
    #[test]
    fn test_interpolate_single_keyframe() {
        let keyframes = vec![Keyframe { time: 0.0, value: [5.0, 5.0, 5.0] }];
        let pos = interpolate_position(&keyframes, 100.0, InterpolationAlgorithm::Linear).unwrap();
        assert_eq!(pos, [5.0, 5.0, 5.0]);
    }

    /// 验证采样实体路径计算：拖尾加前导共应产生至少 4 个路径点。
    /// 位置样本经插值后逐个转到直角坐标。
    #[test]
    fn test_compute_path_sampled() {
        let mut entity = Entity::new("sat");
        entity.position = Property::Sampled(vec![
            (0.0, [0.0, 0.0, 0.0]),
            (60.0, [0.01, 0.01, 100.0]),
            (120.0, [0.02, 0.02, 200.0]),
        ]);
        entity.path = Some(PathGraphics {
            lead_time: Property::Constant(60.0),
            trail_time: Property::Constant(60.0),
            resolution: Property::Constant(30.0),
            ..Default::default()
        });

        let ellipsoid = Ellipsoid::WGS84;
        let path = compute_path(&entity, 60.0, 60.0, 60.0, 30.0, &ellipsoid);

        // 应有拖尾 (0-60) + 前导 (60-120) 的点
        assert!(path.len() >= 4);
    }

    /// 验证静态实体路径：常量位置无拖尾/前导，只产出一个当前点。
    #[test]
    fn test_compute_path_static() {
        let entity = Entity::new("static").with_position(0.0, 0.0, 1000.0);
        let ellipsoid = Ellipsoid::WGS84;

        let path = compute_path(&entity, 0.0, 60.0, 60.0, 30.0, &ellipsoid);
        assert_eq!(path.len(), 1); // 静态实体只有一个点
    }

    /// 验证批量更新：仅带 path 图形且可见的实体参算，因此两个实体
    /// 中只有 sat-1 返回非空路径。
    #[test]
    fn test_update_all_paths() {
        let mut entities = EntityCollection::new();

        let mut sat = Entity::new("sat-1");
        sat.position = Property::Sampled(vec![
            (0.0, [0.0, 0.0, 0.0]),
            (120.0, [0.02, 0.02, 200.0]),
        ]);
        sat.path = Some(PathGraphics {
            lead_time: Property::Constant(60.0),
            trail_time: Property::Constant(60.0),
            resolution: Property::Constant(30.0),
            ..Default::default()
        });
        entities.add(sat);

        // 没有 path 的实体
        entities.add(Entity::new("no-path").with_position(0.0, 0.0, 0.0));

        let ellipsoid = Ellipsoid::WGS84;
        let paths = update_all_paths(&entities, 60.0, &ellipsoid);

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "sat-1");
        assert!(!paths[0].1.is_empty());
    }

    /// 验证 seek 钉位：目标时间超出起止时被钉到区间边界，中点则
    /// 原样写入。
    #[test]
    fn test_clock_seek() {
        let mut clock = AnimationClock::new(0.0, 100.0);
        clock.seek(50.0);
        assert!((clock.current_time - 50.0).abs() < 1e-10);

        clock.seek(200.0);
        assert!((clock.current_time - 100.0).abs() < 1e-10);

        clock.seek(-10.0);
        assert!((clock.current_time - 0.0).abs() < 1e-10);
    }

    /// 验证倍率作用：multiplier=2 时 tick 10 秒实际前进 20 秒。
    /// 确保推进量按倍率缩放。
    #[test]
    fn test_clock_multiplier() {
        let mut clock = AnimationClock::new(0.0, 100.0);
        clock.playing = true;
        clock.multiplier = 2.0;

        clock.tick(10.0);
        assert!((clock.current_time - 20.0).abs() < 1e-10);
    }
}
