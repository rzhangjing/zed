//! TweenCollection - 通过缓动函数实现属性动画。
//!
//! 管理一组标量属性的补间动画，逐帧根据缓动函数求值并回调更新。

// 历史遗留的移植风格债（deferred.md #18）；在 M13 lint 清理、或本文件在其里程碑被重写时重新审视
#![allow(clippy::derivable_impls, clippy::type_complexity, clippy::too_many_arguments, clippy::ptr_eq)]
use std::collections::HashMap;

/// 用于补间（tween）动画的缓动函数。
///
/// 每个家族（二次/三次/四次/五次/正弦/指数/圆形/弹性/回弹/弹跳）都提供
/// In（缓入）、Out（缓出）、InOut（先缓入后缓出）三种对称形态，LinearNone 为匀速。
/// 约定输入 k ∈ [0, 1] 为归一化进度，输出为同区间的缓动值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EasingFunction {
    // 匀速：输出恒等于输入
    LinearNone,
    // 二次（t^2）家族
    QuadraticIn,
    QuadraticOut,
    QuadraticInOut,
    // 三次（t^3）家族
    CubicIn,
    CubicOut,
    CubicInOut,
    // 四次（t^4）家族
    QuarticIn,
    QuarticOut,
    QuarticInOut,
    // 五次（t^5）家族
    QuinticIn,
    QuinticOut,
    QuinticInOut,
    // 正弦缓动家族
    SinusoidalIn,
    SinusoidalOut,
    SinusoidalInOut,
    // 指数缓动家族
    ExponentialIn,
    ExponentialOut,
    ExponentialInOut,
    // 圆形缓动家族
    CircularIn,
    CircularOut,
    CircularInOut,
    // 弹性缓动家族（末端振荡）
    ElasticIn,
    ElasticOut,
    ElasticInOut,
    // 回弹缓动家族（越过后回归）
    BackIn,
    BackOut,
    BackInOut,
    // 弹跳缓动家族（落地反弹分段）
    BounceIn,
    BounceOut,
    BounceInOut,
}

impl Default for EasingFunction {
    /// 默认缓动为匀速（LinearNone）。
    fn default() -> Self {
        Self::LinearNone
    }
}

impl EasingFunction {
    /// 在时间 k（0..1）处求缓动函数的值。
    /// 采经典缓动公式（Penner 系），输入为归一化进度、输出为缓动值。
    pub fn evaluate(&self, k: f64) -> f64 {
        use std::f64::consts::PI;
        match self {
            Self::LinearNone => k,
            Self::QuadraticIn => k * k,
            Self::QuadraticOut => k * (2.0 - k),
            Self::QuadraticInOut => {
                if k < 0.5 { 2.0 * k * k } else { -1.0 + (4.0 - 2.0 * k) * k }
            }
            Self::CubicIn => k * k * k,
            Self::CubicOut => { let f = k - 1.0; f * f * f + 1.0 }
            Self::CubicInOut => {
                if k < 0.5 { 4.0 * k * k * k } else { let f = 2.0 * k - 2.0; 0.5 * f * f * f + 1.0 }
            }
            Self::QuarticIn => k * k * k * k,
            Self::QuarticOut => { let f = k - 1.0; 1.0 - f * f * f * f }
            Self::QuarticInOut => {
                if k < 0.5 { 8.0 * k * k * k * k } else { let f = k - 1.0; 1.0 - 8.0 * f * f * f * f }
            }
            Self::QuinticIn => k * k * k * k * k,
            Self::QuinticOut => { let f = k - 1.0; f * f * f * f * f + 1.0 }
            Self::QuinticInOut => {
                if k < 0.5 { 16.0 * k * k * k * k * k } else { let f = 2.0 * k - 2.0; 0.5 * f * f * f * f * f + 1.0 }
            }
            // 正弦：用 1-cos / sin 的 quarter-cycle 逼近
            Self::SinusoidalIn => 1.0 - (k * PI / 2.0).cos(),
            Self::SinusoidalOut => (k * PI / 2.0).sin(),
            Self::SinusoidalInOut => 0.5 * (1.0 - (PI * k).cos()),
            // 指数：以 2 的幂次逼近端点，仅在 k==0/1 时精确到达
            Self::ExponentialIn => {
                if k == 0.0 { 0.0 } else { 2.0_f64.powf(10.0 * (k - 1.0)) }
            }
            Self::ExponentialOut => {
                if k == 1.0 { 1.0 } else { 1.0 - 2.0_f64.powf(-10.0 * k) }
            }
            Self::ExponentialInOut => {
                if k == 0.0 { return 0.0; }
                if k == 1.0 { return 1.0; }
                if k < 0.5 { 0.5 * 2.0_f64.powf(20.0 * k - 10.0) }
                else { 1.0 - 0.5 * 2.0_f64.powf(-20.0 * k + 10.0) }
            }
            // 圆形：基于单位圆弧 sqrt(1-t^2)
            Self::CircularIn => 1.0 - (1.0 - k * k).sqrt(),
            Self::CircularOut => (1.0 - (k - 1.0) * (k - 1.0)).sqrt(),
            Self::CircularInOut => {
                if k < 0.5 { 0.5 * (1.0 - (1.0 - 4.0 * k * k).sqrt()) }
                else { 0.5 * ((1.0 - (-2.0 * k + 2.0) * (-2.0 * k + 2.0)).sqrt() + 1.0) }
            }
            // 弹性：指数衰减叠加正弦振荡，末端反复过冲
            Self::ElasticIn => {
                if k == 0.0 { return 0.0; }
                if k == 1.0 { return 1.0; }
                -(2.0_f64.powf(10.0 * k - 10.0) * ((k * 10.0 - 10.75) * (2.0 * PI / 3.0)).sin())
            }
            Self::ElasticOut => {
                if k == 0.0 { return 0.0; }
                if k == 1.0 { return 1.0; }
                2.0_f64.powf(-10.0 * k) * ((k * 10.0 - 0.75) * (2.0 * PI / 3.0)).sin() + 1.0
            }
            Self::ElasticInOut => {
                if k == 0.0 { return 0.0; }
                if k == 1.0 { return 1.0; }
                let c5 = (2.0 * PI) / 4.5;
                if k < 0.5 {
                    -(2.0_f64.powf(20.0 * k - 10.0) * ((20.0 * k - 11.125) * c5).sin()) / 2.0
                } else {
                    (2.0_f64.powf(-20.0 * k + 10.0) * ((20.0 * k - 11.125) * c5).sin()) / 2.0 + 1.0
                }
            }
            // 回弹：先越过终点（由 c1 控制过冲量）再回归
            Self::BackIn => {
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                c3 * k * k * k - c1 * k * k
            }
            Self::BackOut => {
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                let f = k - 1.0;
                1.0 + c3 * f * f * f + c1 * f * f
            }
            Self::BackInOut => {
                let c1 = 1.70158;
                let c2 = c1 * 1.525;
                if k < 0.5 {
                    ((2.0 * k) * (2.0 * k) * ((c2 + 1.0) * 2.0 * k - c2)) / 2.0
                } else {
                    ((2.0 * k - 2.0) * (2.0 * k - 2.0) * ((c2 + 1.0) * (k * 2.0 - 2.0) + c2) + 2.0) / 2.0
                }
            }
            // 弹跳：由 bounce_out 的镜像/堆叠组合出三种对称形态
            Self::BounceIn => 1.0 - Self::bounce_out(1.0 - k),
            Self::BounceOut => Self::bounce_out(k),
            Self::BounceInOut => {
                if k < 0.5 { (1.0 - Self::bounce_out(1.0 - 2.0 * k)) / 2.0 }
                else { (1.0 + Self::bounce_out(2.0 * k - 1.0)) / 2.0 }
            }
        }
    }

    /// 弹跳缓出的基础曲线：用分段二次多项式模拟多次衰减的落地反弹。
    fn bounce_out(k: f64) -> f64 {
        let n1 = 7.5625;
        let d1 = 2.75;
        if k < 1.0 / d1 {
            n1 * k * k
        } else if k < 2.0 / d1 {
            let f = k - 1.5 / d1;
            n1 * f * f + 0.75
        } else if k < 2.5 / d1 {
            let f = k - 2.25 / d1;
            n1 * f * f + 0.9375
        } else {
            let f = k - 2.625 / d1;
            n1 * f * f + 0.984375
        }
    }
}

/// 单个补间动画。
///
/// 描述一组命名标量属性从起始值集到结束值集的插值过程，含时长/延迟/缓动与回调。
pub struct Tween {
    /// 属性名 → 起始值的映射。
    start_object: HashMap<String, f64>,
    /// 属性名 → 结束值的映射。
    stop_object: HashMap<String, f64>,
    /// 补间持续时长（秒）。
    duration: f64,
    /// 开始前的延迟时长（秒）。
    delay: f64,
    /// 采用的缓动函数。
    easing_function: EasingFunction,
    /// 每帧插值后调用的更新回调（传入当前插值结果）。
    update_callback: Option<Box<dyn FnMut(&HashMap<String, f64>)>>,
    /// 补间正常完成时调用的回调。
    complete_callback: Option<Box<dyn FnMut()>>,
    /// 补间被取消时调用的回调。
    cancel_callback: Option<Box<dyn FnMut()>>,
    /// 首次更新时记录的实际起始时间（秒），未开始时为 None。
    start_time: Option<f64>,
    /// 额外重复次数（INFINITY 表示无限循环）。
    repeat: f64,
    /// 已完成的重复计数。
    repeat_count: f64,
}

impl Tween {
    /// 返回起始值映射的引用。
    pub fn start_object(&self) -> &HashMap<String, f64> { &self.start_object }
    /// 返回结束值映射的引用。
    pub fn stop_object(&self) -> &HashMap<String, f64> { &self.stop_object }
    /// 返回补间持续时长（秒）。
    pub fn duration(&self) -> f64 { self.duration }
    /// 返回开始前的延迟时长（秒）。
    pub fn delay(&self) -> f64 { self.delay }
    /// 返回采用的缓动函数。
    pub fn easing_function(&self) -> EasingFunction { self.easing_function }

    /// 计算在给定经过时间（自补间开始的秒数）处的插值。
    fn compute_values(&self, elapsed: f64) -> HashMap<String, f64> {
        let mut result = HashMap::new();
        // 归一化进度 t = elapsed / duration，夹到 [0,1]；零时长直接取 1
        let t = if self.duration > 0.0 {
            (elapsed / self.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // 对进度施加缓动函数得到 eased
        let eased = self.easing_function.evaluate(t);
        // 逐属性在起止值间按 eased 线性混合（结束值缺失时沿用起始值）
        for (key, &start_val) in &self.start_object {
            let stop_val = self.stop_object.get(key).copied().unwrap_or(start_val);
            result.insert(key.clone(), start_val + (stop_val - start_val) * eased);
        }
        result
    }
}

/// 添加补间的选项。
///
/// 聚合构造一个 [`Tween`] 所需的全部参数（含可选的三种回调与重复次数）。
pub struct TweenOptions {
    /// 属性名 → 起始值的映射。
    pub start_object: HashMap<String, f64>,
    /// 属性名 → 结束值的映射。
    pub stop_object: HashMap<String, f64>,
    /// 补间持续时长（秒）。
    pub duration: f64,
    /// 开始前的延迟时长（秒）。
    pub delay: f64,
    /// 采用的缓动函数。
    pub easing_function: EasingFunction,
    /// 每帧插值后的更新回调。
    pub update: Option<Box<dyn FnMut(&HashMap<String, f64>)>>,
    /// 完成时回调。
    pub complete: Option<Box<dyn FnMut()>>,
    /// 取消时回调。
    pub cancel: Option<Box<dyn FnMut()>>,
    /// 额外重复次数。
    pub repeat: f64,
}

impl TweenOptions {
    /// 以必填参数创建选项，其余字段取默认值（无延迟/匀速/无回调/不重复）。
    pub fn new(start_object: HashMap<String, f64>, stop_object: HashMap<String, f64>, duration: f64) -> Self {
        Self {
            start_object,
            stop_object,
            duration,
            delay: 0.0,
            easing_function: EasingFunction::LinearNone,
            update: None,
            complete: None,
            cancel: None,
            repeat: 0.0,
        }
    }
}

/// 一组补间动画。
///
/// 持有多个 [`Tween`]，按时间推进求值，自动处理延迟、重复、完成与取消。
pub struct TweenCollection {
    /// 当前持有的补间列表（以 swap_remove 维护，顺序不保证）。
    tweens: Vec<Tween>,
}

impl TweenCollection {
    /// 创建一个空的补间集合。
    pub fn new() -> Self {
        Self { tweens: Vec::new() }
    }

    /// 返回当前补间数量。
    pub fn len(&self) -> usize { self.tweens.len() }
    /// 若集合中没有任何补间则返回 true。
    pub fn is_empty(&self) -> bool { self.tweens.is_empty() }

    /// 添加一个补间。若 duration == 0，则立即调用 complete 并不添加。
    /// 返回已添加补间的索引，若 duration 为 0 则返回 None。
    pub fn add(&mut self, mut options: TweenOptions) -> Option<usize> {
        // 零时长补间：瞬间完成，直接回调 complete 且不入集合
        if options.duration == 0.0 {
            if let Some(ref mut complete) = options.complete {
                complete();
            }
            return None;
        }

        // 由选项各字段构造补间，start_time 待首帧 update 时确定
        let tween = Tween {
            start_object: options.start_object,
            stop_object: options.stop_object,
            duration: options.duration,
            delay: options.delay,
            easing_function: options.easing_function,
            update_callback: options.update,
            complete_callback: options.complete,
            cancel_callback: options.cancel,
            start_time: None,
            repeat: options.repeat,
            repeat_count: 0.0,
        };
        self.tweens.push(tween);
        Some(self.tweens.len() - 1)
    }

    /// 添加一个对单个标量属性做动画的补间。
    ///
    /// 将单一属性封装名为 "value" 的起止映射，并通过 update 回调写回目标对象。
    pub fn add_property(
        &mut self,
        start_value: f64,
        stop_value: f64,
        duration: f64,
        delay: f64,
        easing_function: EasingFunction,
        object: std::rc::Rc<std::cell::RefCell<HashMap<String, f64>>>,
        property: String,
    ) -> Option<usize> {
        // 克隆对象句柄与属性名供闭包捕获
        let obj = object.clone();
        let prop = property.clone();
        let update = move |values: &HashMap<String, f64>| {
            if let Some(&v) = values.get("value") {
                obj.borrow_mut().insert(prop.clone(), v);
            }
        };
        // 起止值都以键 "value" 装入映射
        let mut start = HashMap::new();
        start.insert("value".to_string(), start_value);
        let mut stop = HashMap::new();
        stop.insert("value".to_string(), stop_value);

        let options = TweenOptions {
            start_object: start,
            stop_object: stop,
            duration,
            delay,
            easing_function,
            update: Some(Box::new(update)),
            complete: None,
            cancel: None,
            repeat: 0.0,
        };
        self.add(options)
    }

    /// 添加一个对颜色 uniform 的 alpha 做动画的补间。
    ///
    /// 将 alpha 的插值广播写入每个颜色键对应的 `<key>.alpha` uniform。
    pub fn add_alpha(
        &mut self,
        duration: f64,
        start_value: f64,
        stop_value: f64,
        uniforms: std::rc::Rc<std::cell::RefCell<HashMap<String, f64>>>,
        color_keys: Vec<String>,
    ) -> Option<usize> {
        // 克隆 uniform 句柄与颜色键列表供闭包捕获
        let u = uniforms.clone();
        let keys = color_keys.clone();
        let update = move |values: &HashMap<String, f64>| {
            if let Some(&alpha) = values.get("alpha") {
                let mut map = u.borrow_mut();
                // 将同一 alpha 值广播写入每个颜色键
                for key in &keys {
                    map.insert(format!("{}.alpha", key), alpha);
                }
            }
        };
        let mut start = HashMap::new();
        start.insert("alpha".to_string(), start_value);
        let mut stop = HashMap::new();
        stop.insert("alpha".to_string(), stop_value);

        let options = TweenOptions {
            start_object: start,
            stop_object: stop,
            duration,
            delay: 0.0,
            easing_function: EasingFunction::LinearNone,
            update: Some(Box::new(update)),
            complete: None,
            cancel: None,
            repeat: 0.0,
        };
        self.add(options)
    }

    /// 添加一个递增 offset uniform 的补间。
    ///
    /// 以当前 offset 为起点、+1 为终点做无限重复的线性递增（用于纹理滚动）。
    pub fn add_offset_increment(
        &mut self,
        duration: f64,
        uniforms: std::rc::Rc<std::cell::RefCell<HashMap<String, f64>>>,
    ) -> Option<usize> {
        // 读取当前 offset 作为动画起点（缺失时视为 0）
        let current = uniforms.borrow().get("offset").copied().unwrap_or(0.0);
        let u = uniforms.clone();
        let update = move |values: &HashMap<String, f64>| {
            if let Some(&v) = values.get("value") {
                u.borrow_mut().insert("offset".to_string(), v);
            }
        };
        let mut start = HashMap::new();
        start.insert("value".to_string(), current);
        let mut stop = HashMap::new();
        stop.insert("value".to_string(), current + 1.0);

        let options = TweenOptions {
            start_object: start,
            stop_object: stop,
            duration,
            delay: 0.0,
            easing_function: EasingFunction::LinearNone,
            update: Some(Box::new(update)),
            complete: None,
            cancel: None,
            repeat: f64::INFINITY,
        };
        self.add(options)
    }

    /// 按索引移除一个补间，并调用其 cancel 回调。
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.tweens.len() {
            return false;
        }
        // swap_remove 用末元素填补空位，O(1) 但改变顺序
        let mut tween = self.tweens.swap_remove(index);
        if let Some(ref mut cancel) = tween.cancel_callback {
            cancel();
        }
        true
    }

    /// 通过搜索匹配的补间来移除一个补间。
    /// 若找到并移除则返回 true。
    pub fn remove_tween(&mut self, tween_ptr: *const Tween) -> bool {
        if let Some(pos) = self.tweens.iter().position(|t| t as *const Tween == tween_ptr) {
            self.remove(pos)
        } else {
            false
        }
    }

    /// 移除所有补间，对每个调用 cancel。
    pub fn remove_all(&mut self) {
        // 逐个取出并触发各自的 cancel 回调
        for tween in self.tweens.drain(..) {
            let mut tween = tween;
            if let Some(ref mut cancel) = tween.cancel_callback {
                cancel();
            }
        }
    }

    /// 若集合在给定索引处包含补间则返回 true。
    pub fn contains(&self, index: usize) -> bool {
        // 以索引是否落在长度范围内判定存在性
        index < self.tweens.len()
    }

    /// 按索引获取对某个补间的引用。
    pub fn get(&self, index: usize) -> Option<&Tween> {
        // 越界时返回 None
        self.tweens.get(index)
    }

    /// 取消一个补间（移除它并调用 cancel 回调）。
    pub fn cancel_tween(&mut self, index: usize) -> bool {
        self.remove(index)
    }

    /// 将所有补间更新到给定时间（秒）。
    /// 已完成的补间会从集合中移除。
    pub fn update(&mut self, time: f64) {
        let mut i = 0;
        while i < self.tweens.len() {
            // 首次见到该补间时记下起始时间；elapsed 扣除启动时刻与延迟
            let start_time = self.tweens[i].start_time.get_or_insert(time);
            let elapsed = time - *start_time - self.tweens[i].delay;

            // 仍在延迟窗口内：本帧不处理，直接下一个
            if elapsed < 0.0 {
                i += 1;
                continue;
            }

            let duration = self.tweens[i].duration;
            // 按经过时间插值并触发 update 回调
            let values = self.tweens[i].compute_values(elapsed);

            if let Some(ref mut update_cb) = self.tweens[i].update_callback {
                update_cb(&values);
            }

            if elapsed >= duration {
                // 补间已完成
                // 读取重复设置以决定是重启还是真正完成
                let repeat = self.tweens[i].repeat;
                let repeat_count = self.tweens[i].repeat_count;

                if repeat_count < repeat {
                    // 重新开始
                    self.tweens[i].repeat_count += 1.0;
                    self.tweens[i].start_time = Some(time);
                    i += 1;
                } else {
                    // 完成并移除
                    let mut tween = self.tweens.swap_remove(i);
                    if let Some(ref mut complete_cb) = tween.complete_callback {
                        complete_cb();
                    }
                    // 不递增 i，因为 swap_remove 已移来一个元素
                }
            } else {
                i += 1;
            }
        }
    }
}

impl Default for TweenCollection {
    /// 默认构造一个空集合。
    fn default() -> Self {
        Self::new()
    }
}
