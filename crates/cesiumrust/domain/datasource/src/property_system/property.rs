//! `DynProperty` 核心 trait 及其具体实现。
//!
//! 本模块定义属性系统的统一接口 `DynProperty`，并给出若干具体属性
//! 类型：常量属性 `ConstantProperty`、采样属性 `SampledProperty`、
//! 时间区间集合属性 `TimeIntervalCollectionProperty`、组合属性
//! `CompositeProperty` 与回调属性 `CallbackProperty`。
//!
//! 这些类型共同支撑“可随时间求值的值”这一抽象：常量属性不随时间变化，
//! 采样属性按离散时间与插值算法求值，区间集合属性按落入的时间区间取值，
//! 组合属性在每个区间内委托给另一个属性，回调属性则延迟调用用户函数。

use crate::property_system::interpolation::{
    ExtrapolationType, InterpolationAlgorithm, InterpolationAlgorithmKind,
};
use crate::property_system::value::{PackableType, PropertyValue, ReferenceFrame};
use cesium_time::{JulianDate, TimeInterval, TimeIntervalCollection, TimeIntervalData};
use std::any::Any;
use std::sync::Arc;

/// 属性统一接口：表示一个可随时间求值的值。
///
/// 每个实现都需提供：是否为常量、给定时间处的取值、稳定类型名、
/// 相等比较与向下转型能力；位置类与材质类属性还可覆写参考系查询、
/// 按参考系取值以及材质类型查询等默认返回空的可选方法。
pub trait DynProperty: Send + Sync {
    /// 在当前定义下 `get_value` 是否总返回相同结果。映射到 `Property.prototype.isConstant`。
    fn is_constant(&self) -> bool;

    /// 获取所提供时间处该属性的值。
    /// 映射到 `Property.prototype.getValue`。
    fn get_value(&self, time: &JulianDate) -> PropertyValue;

    /// 一个用于向下转型与调试的稳定类型名。
    fn type_name(&self) -> &'static str;

    /// 将此属性与另一个属性比较。映射到 `Property.prototype.equals`。
    fn equals(&self, other: &dyn DynProperty) -> bool;

    /// 支持向下转型为具体类型。
    fn as_any(&self) -> &dyn Any;

    /// 定义位置时所用的参考系。仅对位置属性有效。
    /// 映射到 `PositionProperty.referenceFrame`。
    fn reference_frame(&self) -> Option<ReferenceFrame> {
        None
    }

    /// 在所提供的参考系中获取值。仅对位置属性
    /// 有效。映射到 `PositionProperty.getValueInReferenceFrame`。
    fn get_value_in_reference_frame(
        &self,
        _time: &JulianDate,
        _frame: ReferenceFrame,
    ) -> Option<PropertyValue> {
        None
    }

    /// 获取所提供时间处的材质类型。仅对材质
    /// 属性有效。映射到 `MaterialProperty.getType`。
    fn get_type(&self, _time: &JulianDate) -> Option<String> {
        None
    }
}

/// 比较两个 trait-object 属性是否相等，将 `Arc`
/// 指针相等视为相等。镜像 CesiumJS `Property.equals(left, right)`。
pub fn arc_property_equals(left: &Arc<dyn DynProperty>, right: &Arc<dyn DynProperty>) -> bool {
    // 先看两个 Arc 是否指向同一对象（快路径），否则委托给动态 equals 方法。
    Arc::ptr_eq(left, right) || left.equals(right.as_ref())
}

/// 镜像 CesiumJS `Property.isConstant(property)`：缺失的属性
/// 被视为常量。
pub fn property_is_constant(property: Option<&dyn DynProperty>) -> bool {
    // 缺失属性（None）按约定视为常量；否则委托具体实现判定。
    match property {
        None => true,
        Some(p) => p.is_constant(),
    }
}

/// 镜像 CesiumJS `Property.getValueOrUndefined(property, time)`。
pub fn property_get_value_or_undefined(
    property: Option<&dyn DynProperty>,
    time: &JulianDate,
) -> PropertyValue {
    // 存在则求值，缺失则统一返回未定义哨兵值。
    match property {
        Some(p) => p.get_value(time),
        None => PropertyValue::Undefined,
    }
}

// ---------------------------------------------------------------------------
// ConstantProperty
// ---------------------------------------------------------------------------

/// 其值不随仿真时间变化的属性。
///
/// 构造时固化一个 `PropertyValue`，任意时刻求值都返回该值的克隆；
/// 通过 `set_value` 可整体替换当前值。
#[derive(Debug, Clone)]
pub struct ConstantProperty {
    /// 固定的属性值，不随时间改变。
    value: PropertyValue,
}

impl ConstantProperty {
    /// 创建一个新的常量属性，取给定值。
    pub fn new(value: PropertyValue) -> Self {
        Self { value }
    }

    /// 设置属性的值。
    /// 映射到 `ConstantProperty.prototype.setValue`。
    pub fn set_value(&mut self, value: PropertyValue) {
        self.value = value;
    }

    /// 获取此属性的值。
    /// 映射到 `ConstantProperty.prototype.valueOf`。
    pub fn value(&self) -> &PropertyValue {
        &self.value
    }
}

impl DynProperty for ConstantProperty {
    /// 常量属性始终为常量。
    fn is_constant(&self) -> bool {
        true
    }

    /// 忽略时间参数，直接返回固定值的克隆。
    fn get_value(&self, _time: &JulianDate) -> PropertyValue {
        self.value.clone()
    }

    /// 返回类型名 `ConstantProperty`。
    fn type_name(&self) -> &'static str {
        "ConstantProperty"
    }

    /// 仅当对方同为常量属性且内部值相等时才判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<ConstantProperty>() {
            Some(o) => self.value == o.value,
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// SampledProperty
// ---------------------------------------------------------------------------

/// 在已排序的 `JulianDate` 切片上二分查找。若精确匹配
/// 则返回其索引，否则返回插入点的按位取反值。
fn binary_search_times(times: &[JulianDate], target: &JulianDate) -> isize {
    // 标准二分：low/high 向中间夹收，命中时返回其中点索引。
    let mut low: isize = 0;
    let mut high: isize = times.len() as isize - 1;
    while low <= high {
        let mid = (low + high) / 2;
        match times[mid as usize].cmp(target) {
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Greater => high = mid - 1,
            std::cmp::Ordering::Equal => return mid,
        }
    }
    // 未命中：返回插入点的按位取反值（~low），供调用方区分命中与否。
    !low
}

/// 将新样本合并进已排序的 `times`/`values` 存储，并
/// 保持有序。时间已存在的样本会覆盖已存的值；不存在的时间
/// 则按插入点一次性批量插入尽可能多的连续新样本，减少 splice 次数。
fn merge_new_samples(
    times: &mut Vec<JulianDate>,
    values: &mut Vec<f64>,
    new_times: &[JulianDate],
    new_values: &[f64],
    packed_length: usize,
) {
    let new_count = new_times.len();
    let mut new_data_index = 0usize;

    while new_data_index < new_count {
        // 对当前待插入时间做二分查找，判断其是否已存在于表中。
        let current_time = new_times[new_data_index];
        let search = binary_search_times(times, &current_time);

        if search < 0 {
            // 不存在：尽可能多地插入额外的连续值。
            // 插入点由按位取反的查找结果恢复，值插入位置需乘以打包长度。
            let insert_idx = (!search) as usize;
            let values_insertion_point = insert_idx * packed_length;
            // 下一个已有时间作为本次批量插入的上界。
            let next_time = times.get(insert_idx).copied();

            let mut times_to_insert = Vec::new();
            let mut values_to_insert = Vec::new();
            let mut prev_item: Option<JulianDate> = None;

            while new_data_index < new_count {
                let ct = new_times[new_data_index];
                // 新样本需严格递增；不大于前一个则本轮批量插入结束。
                if let Some(prev) = prev_item {
                    if prev >= ct {
                        break;
                    }
                }
                // 达到下一个已有时间则停，避免跨越现有样本的插入点。
                if let Some(nt) = next_time {
                    if ct >= nt {
                        break;
                    }
                }
                times_to_insert.push(ct);
                let sample_idx = new_data_index;
                new_data_index += 1;
                for i in 0..packed_length {
                    values_to_insert.push(new_values[sample_idx * packed_length + i]);
                }
                prev_item = Some(ct);
            }

            if !times_to_insert.is_empty() {
                values.splice(
                    values_insertion_point..values_insertion_point,
                    values_to_insert.iter().copied(),
                );
                times.splice(insert_idx..insert_idx, times_to_insert.iter().copied());
            }
        } else {
            // 找到精确匹配：覆盖已存的值。
            // 按打包长度逐分量写入对应槽位，不改变时间顺序。
            let idx = search as usize;
            for i in 0..packed_length {
                values[idx * packed_length + i] = new_values[new_data_index * packed_length + i];
            }
            new_data_index += 1;
        }
    }
}

/// 其值在给定时间处由所提供的样本集以及指定的插值
/// 算法与次数插值得到的属性。样本以弧长参数化的时间轴存储，
/// 每个时间对应一段长度为 `packed_length` 的打包数值，可附带导数。
#[derive(Debug, Clone)]
pub struct SampledProperty {
    /// 值的可打包类型，决定打包/解包与插值转换方式。
    property_type: PackableType,
    /// 可选的各阶导数类型集合，用于埃尔米特等带导数的插值。
    derivative_types: Option<Vec<PackableType>>,
    /// 插值次数（参与插值的样本点数减一）。
    interpolation_degree: usize,
    /// 插值算法种类（线性、拉格朗日、埃尔米特等）。
    interpolation_algorithm: InterpolationAlgorithmKind,
    /// 已排序的样本时间序列。
    times: Vec<JulianDate>,
    /// 打包后的样本数值，按 `packed_length` 为每个样本连续存放。
    values: Vec<f64>,
    /// 单个样本（含导数）占用的打包数值长度。
    packed_length: usize,
    /// 参与插值计算时使用的打包长度（可能与存储长度不同，如单位四元数）。
    packed_interpolation_length: usize,
    /// 导数阶数，等于 `derivative_types` 的元素个数。
    input_order: usize,
    /// 前推（超出末尾时间）外推类型。
    forward_extrapolation_type: ExtrapolationType,
    /// 前推外推的有效时长（秒），超出则取值无定义。
    forward_extrapolation_duration: f64,
    /// 后推（早于首个时间）外推类型。
    backward_extrapolation_type: ExtrapolationType,
    /// 后推外推的有效时长（秒），超出则取值无定义。
    backward_extrapolation_duration: f64,
}

impl SampledProperty {
    /// 创建指定类型的新采样属性。
    pub fn new(property_type: PackableType) -> Self {
        Self::with_derivative_types(property_type, None)
    }

    /// 创建带导数信息的新采样属性。
    /// 映射到 `new SampledProperty(type, derivativeTypes)`。
    pub fn with_derivative_types(
        property_type: PackableType,
        derivative_types: Option<Vec<PackableType>>,
    ) -> Self {
        let mut packed_length = property_type.packed_length();
        let mut packed_interpolation_length = property_type.packed_interpolation_length();
        let mut input_order = 0;
        // 若带导数类型，则逐个将导数的打包长度累加到总长，并记录导数阶数。
        if let Some(ref derivs) = derivative_types {
            input_order = derivs.len();
            for d in derivs {
                packed_length += d.packed_length();
                packed_interpolation_length += d.packed_interpolation_length();
            }
        }
        // 主值与导数均入表：初始插值次数为 1、算法为线性，外推均为 None。
        Self {
            property_type,
            derivative_types,
            interpolation_degree: 1,
            interpolation_algorithm: InterpolationAlgorithmKind::Linear,
            times: Vec::new(),
            values: Vec::new(),
            packed_length,
            packed_interpolation_length,
            input_order,
            forward_extrapolation_type: ExtrapolationType::None,
            forward_extrapolation_duration: 0.0,
            backward_extrapolation_type: ExtrapolationType::None,
            backward_extrapolation_duration: 0.0,
        }
    }

    /// 属性的类型。映射到 `SampledProperty.prototype.type`。
    pub fn property_type(&self) -> PackableType {
        self.property_type
    }

    /// 导数类型。映射到 `SampledProperty.prototype.derivativeTypes`。
    pub fn derivative_types(&self) -> Option<&[PackableType]> {
        self.derivative_types.as_deref()
    }

    /// 插值次数。映射到 `interpolationDegree`。
    pub fn interpolation_degree(&self) -> usize {
        self.interpolation_degree
    }

    /// 插值算法。映射到 `interpolationAlgorithm`。
    pub fn interpolation_algorithm(&self) -> InterpolationAlgorithmKind {
        self.interpolation_algorithm
    }

    /// 当前存储的样本数量。
    pub fn sample_count(&self) -> usize {
        // 样本数等于时间轴长度（值数组长度为其 packed_length 倍）。
        self.times.len()
    }

    /// 采样时间。
    pub fn times(&self) -> &[JulianDate] {
        // 暴露内部已排序时间轴的只读切片。
        &self.times
    }

    /// 设置插值时所使用的算法与次数。
    /// 映射到 `SampledProperty.prototype.setInterpolationOptions`。
    pub fn set_interpolation_options(
        &mut self,
        algorithm: Option<InterpolationAlgorithmKind>,
        degree: Option<usize>,
    ) {
        // 两个参数均为可选：仅当传入 Some 时才覆盖对应的算法或次数字段。
        if let Some(alg) = algorithm {
            self.interpolation_algorithm = alg;
        }
        if let Some(deg) = degree {
            self.interpolation_degree = deg;
        }
    }

    /// 设置前推外推类型。映射到 `forwardExtrapolationType`。
    pub fn set_forward_extrapolation_type(&mut self, value: ExtrapolationType) {
        self.forward_extrapolation_type = value;
    }

    /// 设置前推外推时长。映射到
    /// `forwardExtrapolationDuration`。
    pub fn set_forward_extrapolation_duration(&mut self, value: f64) {
        self.forward_extrapolation_duration = value;
    }

    /// 设置后推外推类型。映射到
    /// `backwardExtrapolationType`。
    pub fn set_backward_extrapolation_type(&mut self, value: ExtrapolationType) {
        self.backward_extrapolation_type = value;
    }

    /// 设置后推外推时长。映射到
    /// `backwardExtrapolationDuration`。
    pub fn set_backward_extrapolation_duration(&mut self, value: f64) {
        self.backward_extrapolation_duration = value;
    }

    /// 添加一个新样本。映射到 `SampledProperty.prototype.addSample`。
    pub fn add_sample(
        &mut self,
        time: JulianDate,
        value: &PropertyValue,
        derivatives: &[PropertyValue],
    ) {
        // 先将主值打包，再按导数类型逐个拼接对应阶的导数（缺失记为未定义）。
        let mut new_values = Vec::with_capacity(self.packed_length);
        self.property_type.pack(value, &mut new_values);
        if let Some(ref deriv_types) = self.derivative_types {
            for (i, dt) in deriv_types.iter().enumerate() {
                let d = derivatives.get(i).unwrap_or(&PropertyValue::Undefined);
                dt.pack(d, &mut new_values);
            }
        }
        // 合并进有序存储（新样本可能与已有时间重叠而触发覆盖）。
        merge_new_samples(
            &mut self.times,
            &mut self.values,
            &[time],
            &new_values,
            self.packed_length,
        );
    }

    /// 添加一组样本。映射到 `SampledProperty.prototype.addSamples`。
    pub fn add_samples(
        &mut self,
        times: &[JulianDate],
        values: &[PropertyValue],
        derivative_values: Option<&[Vec<PropertyValue>]>,
    ) {
        // 预分配容量：时间为 N 个，值为 N 乘以单样本打包长度。
        let mut new_times = Vec::with_capacity(times.len());
        let mut new_values = Vec::with_capacity(times.len() * self.packed_length);
        // 逐样本打包主值及其各阶导数（缺失均以未定义补齐）。
        for (i, t) in times.iter().enumerate() {
            new_times.push(*t);
            let v = values.get(i).unwrap_or(&PropertyValue::Undefined);
            self.property_type.pack(v, &mut new_values);
            if let Some(ref deriv_types) = self.derivative_types {
                let empty: Vec<PropertyValue> = Vec::new();
                let derivs = derivative_values
                    .and_then(|dvs| dvs.get(i))
                    .unwrap_or(&empty);
                for (j, dt) in deriv_types.iter().enumerate() {
                    let d = derivs.get(j).unwrap_or(&PropertyValue::Undefined);
                    dt.pack(d, &mut new_values);
                }
            }
        }
        merge_new_samples(
            &mut self.times,
            &mut self.values,
            &new_times,
            &new_values,
            self.packed_length,
        );
    }

    /// 以单个打包数组添加样本，其中每个样本由一个数值时间偏移
    /// （相对于 `epoch` 的秒数）后接打包值（及导数）表示。
    ///
    /// 映射到 `SampledProperty.prototype.addSamplesPackedArray`。
    pub fn add_samples_packed_array(&mut self, packed_samples: &[f64], epoch: &JulianDate) {
        // 每个样本占 stride 个数值：首为相对 epoch 的秒偏移，其后为打包值。
        let stride = 1 + self.packed_length;
        let count = packed_samples.len() / stride;
        let mut new_times = Vec::with_capacity(count);
        let mut new_values = Vec::with_capacity(count * self.packed_length);
        for s in 0..count {
            // 将秒偏移还原为绝对时间，随后拷贝本样本的打包值段。
            let base = s * stride;
            new_times.push(epoch.add_seconds(packed_samples[base]));
            for i in 0..self.packed_length {
                new_values.push(packed_samples[base + 1 + i]);
            }
        }
        merge_new_samples(
            &mut self.times,
            &mut self.values,
            &new_times,
            &new_values,
            self.packed_length,
        );
    }

    /// 获取给定索引处样本的时间。负索引按
    /// 逆序访问样本列表。
    /// 映射到 `SampledProperty.prototype.getSample`。
    pub fn get_sample(&self, index: isize) -> Option<JulianDate> {
        let len = self.times.len();
        // 空表无任何样本，直接返回空。
        if len == 0 {
            return None;
        }
        // 负索引按从尾部倒数的语义折算为正索引。
        let mut idx = index;
        if idx < 0 {
            idx += len as isize;
        }
        // 折算后仍越界则视为无效索引。
        if idx < 0 || idx >= len as isize {
            return None;
        }
        Some(self.times[idx as usize])
    }

    /// 若存在则移除给定时间处的样本。若移除了
    /// 样本则返回 `true`。映射到 `SampledProperty.prototype.removeSample`。
    pub fn remove_sample(&mut self, time: &JulianDate) -> bool {
        // 二分定位：负值表示该时间无对应样本，无需删除。
        let index = binary_search_times(&self.times, time);
        if index < 0 {
            return false;
        }
        self.remove_samples_at(index as usize, 1);
        true
    }

    /// 移除给定时间区间内的所有样本。
    /// 映射到 `SampledProperty.prototype.removeSamples`。
    pub fn remove_samples_interval(&mut self, time_interval: &TimeInterval) {
        // 起点：未命中时取插入点；命中且左开时后移一位以排除该样本。
        let mut start_index = binary_search_times(&self.times, &time_interval.start);
        if start_index < 0 {
            start_index = !start_index;
        } else if !time_interval.is_start_included {
            start_index += 1;
        }
        // 终点：未命中时取插入点；命中且右闭时后移一位以包含该样本。
        let mut stop_index = binary_search_times(&self.times, &time_interval.stop);
        if stop_index < 0 {
            stop_index = !stop_index;
        } else if time_interval.is_stop_included {
            stop_index += 1;
        }
        // 仅当区间非空（stop>start）时才批量删除。
        let start = start_index as usize;
        let stop = stop_index as usize;
        if stop > start {
            self.remove_samples_at(start, stop - start);
        }
    }

    /// 从 `start_index` 起移除 `number_to_remove` 个样本，同步排空
    /// 时间轴与对应的打包数值段。
    fn remove_samples_at(&mut self, start_index: usize, number_to_remove: usize) {
        // 值数组按 packed_length 对齐，故时间区间需乘以打包长度。
        let packed_length = self.packed_length;
        self.times
            .drain(start_index..start_index + number_to_remove);
        self.values.drain(
            start_index * packed_length..(start_index + number_to_remove) * packed_length,
        );
    }
}

impl DynProperty for SampledProperty {
    /// 没有任何样本时视为常量（无需求值）。
    fn is_constant(&self) -> bool {
        self.values.is_empty()
    }

    /// 在给定时间处求值：先二分查找样本时间，命中则直接解包返回；
    /// 否则根据时间落在范围前/后进行外推处理，最终在选取的样本窗口内
    /// 按所配置的插值算法与次数插值得到结果。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        let times = &self.times;
        let times_length = times.len();
        // 无样本时无可求值，直接返回未定义。
        if times_length == 0 {
            return PropertyValue::Undefined;
        }

        let inner_type = self.property_type;
        let values = &self.values;
        // 二分查找：非负为精确命中的索引，负值为按位取反的插入点。
        let mut index = binary_search_times(times, time);

        if index >= 0 {
            // 精确匹配。
            return inner_type.unpack(values, index as usize * self.packed_length);
        }

        // 转换为插入索引。
        index = !index;

        // 时间早于首个样本：按后推外推类型处理（None/超时→未定义，Hold→首值）。
        if index == 0 {
            let start_time = times[0];
            let timeout = self.backward_extrapolation_duration;
            if self.backward_extrapolation_type == ExtrapolationType::None
                || (timeout != 0.0 && start_time.seconds_difference(time) > timeout)
            {
                return PropertyValue::Undefined;
            }
            if self.backward_extrapolation_type == ExtrapolationType::Hold {
                return inner_type.unpack(values, 0);
            }
        }

        // 时间晚于末个样本：钳制到末尾索引并按前推外推类型处理。
        if index as usize >= times_length {
            index = (times_length - 1) as isize;
            let end_time = times[index as usize];
            let timeout = self.forward_extrapolation_duration;
            if self.forward_extrapolation_type == ExtrapolationType::None
                || (timeout != 0.0 && time.seconds_difference(&end_time) > timeout)
            {
                return PropertyValue::Undefined;
            }
            if self.forward_extrapolation_type == ExtrapolationType::Hold {
                return inner_type.unpack(values, index as usize * inner_type.packed_length());
            }
        }

        // 前推/后推为 Extrapolate 时不提前返回，继续走下方插值分支。
        let interpolation_algorithm = self.interpolation_algorithm;
        let packed_interpolation_length = self.packed_interpolation_length;
        let input_order = self.input_order;

        // 依算法与导数阶数计算所需样本点数，并受实际样本总数上限约束。
        let number_of_points = interpolation_algorithm
            .get_required_data_points(self.interpolation_degree, input_order)
            .min(times_length);

        // 有效插值次数小于 1 时无法插值，返回未定义。
        let degree = number_of_points as isize - 1;
        if degree < 1 {
            return PropertyValue::Undefined;
        }
        let degree = degree as usize;

        let mut first_index = 0usize;
        let mut last_index = times_length - 1;
        let points_in_collection = last_index - first_index + 1;

        // 围绕插入点选取长度约为 degree 的样本窗口，并向两端夹紧到合法范围。
        if points_in_collection > degree {
            let mut computed_first = index - (degree as isize / 2) - 1;
            if computed_first < 0 {
                computed_first = 0;
            }
            let mut computed_last = computed_first + degree as isize;
            let last_is = last_index as isize;
            if computed_last > last_is {
                computed_last = last_is;
                computed_first = computed_last - degree as isize;
                if computed_first < 0 {
                    computed_first = 0;
                }
            }
            first_index = computed_first as usize;
            last_index = computed_last as usize;
        }
        let length = last_index - first_index + 1;

        // 构建 x 表（相对于窗口内最后一个样本的秒数）。
        let mut x_table = vec![0.0f64; length];
        for (i, x) in x_table.iter_mut().enumerate() {
            *x = times[first_index + i].seconds_difference(&times[last_index]);
        }

        // 构建 y 表。
        let y_table: Vec<f64> = if !inner_type.uses_interpolation_conversion() {
            let packed_length = self.packed_length;
            let source_start = first_index * packed_length;
            let source_stop = (last_index + 1) * packed_length;
            values[source_start..source_stop].to_vec()
        } else {
            let mut table = vec![0.0f64; length * packed_interpolation_length];
            inner_type.convert_packed_array_for_interpolation(
                values,
                first_index,
                last_index,
                &mut table,
            );
            table
        };

        // 插值自变量 x：目标时间相对窗口末个样本的秒差（末样本为原点）。
        let x = time.seconds_difference(&times[last_index]);
        // 无导数或算法不支持导数时走零阶插值，否则走带导数的插值。
        let interpolation_result = if input_order == 0
            || !interpolation_algorithm.supports_derivatives()
        {
            interpolation_algorithm.interpolate_order_zero(
                x,
                &x_table,
                &y_table,
                packed_interpolation_length,
            )
        } else {
            let y_stride = packed_interpolation_length / (input_order + 1);
            interpolation_algorithm.interpolate(x, &x_table, &y_table, y_stride, input_order, input_order)
        };

        // 若类型需要插值转换（如经纬度→笛卡尔），需将结果还原回存储表示。
        if !inner_type.uses_interpolation_conversion() {
            inner_type.unpack(&interpolation_result, 0)
        } else {
            inner_type.unpack_interpolation_result(
                &interpolation_result,
                values,
                first_index,
                last_index,
            )
        }
    }

    /// 返回类型名 `SampledProperty`。
    fn type_name(&self) -> &'static str {
        "SampledProperty"
    }

    /// 仅当对方同为采样属性，且类型、插值次数/算法、导数类型
    /// 以及全部时间与数值均逐相等时才判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<SampledProperty>() {
            Some(o) => {
                self.property_type == o.property_type
                    && self.interpolation_degree == o.interpolation_degree
                    && self.interpolation_algorithm == o.interpolation_algorithm
                    && self.derivative_types == o.derivative_types
                    && self.times == o.times
                    && self.values == o.values
            }
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// TimeIntervalCollectionProperty
// ---------------------------------------------------------------------------

/// 判断两个属性值是否携带相同数据，用于区间集合的相等比较。
fn value_same_data(a: &PropertyValue, b: &PropertyValue) -> bool {
    a == b
}

/// 由 `TimeIntervalCollection` 定义的属性，其中每个区间的数据
/// 直接存储为该时间处的取值；求值时定位包含该时间的区间并返回其数据。
#[derive(Debug, Clone)]
pub struct TimeIntervalCollectionProperty {
    /// 按时间划分、每段携带一个属性值的区间集合。
    intervals: TimeIntervalCollection<PropertyValue>,
}

impl Default for TimeIntervalCollectionProperty {
    /// 默认构造一个空区间集合的属性。
    fn default() -> Self {
        Self::new()
    }
}

impl TimeIntervalCollectionProperty {
    /// 创建空的区间集合属性。
    pub fn new() -> Self {
        Self {
            intervals: TimeIntervalCollection::new(),
        }
    }

    /// 底层的区间集合。映射到 `intervals`。
    pub fn intervals(&self) -> &TimeIntervalCollection<PropertyValue> {
        &self.intervals
    }

    /// 添加一个带给定值数据的区间。
    pub fn add_interval(&mut self, interval: TimeInterval, data: Option<PropertyValue>) {
        // 将时间与值打包为区间数据，再按相等判断函数插入集合（重叠时合并）。
        let tid = TimeIntervalData::new(interval, data);
        self.intervals.add_interval(tid, &value_same_data);
    }
}

impl DynProperty for TimeIntervalCollectionProperty {
    /// 区间集合为空时视为常量。
    fn is_constant(&self) -> bool {
        self.intervals.is_empty()
    }

    /// 定位包含给定时间的区间，返回其存储值的克隆，否则返回未定义。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        match self.intervals.find_data_for_interval_containing_date(time) {
            Some(v) => v.clone(),
            None => PropertyValue::Undefined,
        }
    }

    /// 返回类型名 `TimeIntervalCollectionProperty`。
    fn type_name(&self) -> &'static str {
        "TimeIntervalCollectionProperty"
    }

    /// 仅当对方同为区间集合属性且底层区间逐相等时判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other
            .as_any()
            .downcast_ref::<TimeIntervalCollectionProperty>()
        {
            Some(o) => self.intervals.equals(&o.intervals, &value_same_data),
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// CompositeProperty
// ---------------------------------------------------------------------------

/// 判断两个 trait-object 属性是否携带相同数据，委托给 `arc_property_equals`。
fn property_same_data(a: &Arc<dyn DynProperty>, b: &Arc<dyn DynProperty>) -> bool {
    arc_property_equals(a, b)
}

/// 由 `TimeIntervalCollection` 定义的属性，其中每个区间的数据
/// 是另一个属性；求值时定位包含给定时间的区间，再委托该内部属性求值。
#[derive(Clone)]
pub struct CompositeProperty {
    /// 按时间划分、每段携带一个子属性的区间集合。
    intervals: TimeIntervalCollection<Arc<dyn DynProperty>>,
}

impl Default for CompositeProperty {
    /// 默认构造一个空区间集合的组合属性。
    fn default() -> Self {
        Self::new()
    }
}

impl CompositeProperty {
    /// 创建空的组合属性。
    pub fn new() -> Self {
        Self {
            intervals: TimeIntervalCollection::new(),
        }
    }

    /// 底层的区间集合。映射到 `intervals`。
    pub fn intervals(&self) -> &TimeIntervalCollection<Arc<dyn DynProperty>> {
        &self.intervals
    }

    /// 添加一个数据为另一个属性的区间。
    pub fn add_interval(&mut self, interval: TimeInterval, data: Option<Arc<dyn DynProperty>>) {
        // 将子属性作为区间数据插入集合，相等判断基于 Arc 指针与其动态 equals。
        let tid = TimeIntervalData::new(interval, data);
        self.intervals.add_interval(tid, &property_same_data);
    }
}

impl DynProperty for CompositeProperty {
    /// 区间集合为空时视为常量。
    fn is_constant(&self) -> bool {
        self.intervals.is_empty()
    }

    /// 定位包含给定时间的区间，若存在则委托内部子属性在该时间处求值。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        match self.intervals.find_data_for_interval_containing_date(time) {
            Some(inner) => inner.get_value(time),
            None => PropertyValue::Undefined,
        }
    }

    /// 返回类型名 `CompositeProperty`。
    fn type_name(&self) -> &'static str {
        "CompositeProperty"
    }

    /// 仅当对方同为组合属性且底层区间逐相等时判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<CompositeProperty>() {
            Some(o) => self.intervals.equals(&o.intervals, &property_same_data),
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// CallbackProperty
// ---------------------------------------------------------------------------

/// `CallbackProperty` 所使用的回调函数类型。
pub type CallbackFn = Arc<dyn Fn(&JulianDate) -> PropertyValue + Send + Sync>;

/// 其值由回调函数延迟求值的属性。每次求值都调用内部回调，
/// 并可由 `is_constant` 标志声明结果是否不随时间变化。
#[derive(Clone)]
pub struct CallbackProperty {
    /// 延迟求值的共享回调，接受时间返回属性值。
    callback: CallbackFn,
    /// 声明该回调是否总返回相同结果（常量标志）。
    is_constant: bool,
}

impl CallbackProperty {
    /// 创建新的回调属性。
    /// 映射到 `new CallbackProperty(callback, isConstant)`。
    pub fn new<F>(callback: F, is_constant: bool) -> Self
    where
        F: Fn(&JulianDate) -> PropertyValue + Send + Sync + 'static,
    {
        // 将用户闭包装入共享指针以便按引用克隆与传借。
        Self {
            callback: Arc::new(callback),
            is_constant,
        }
    }

    /// 从共享回调创建回调属性。
    pub fn from_arc(callback: CallbackFn, is_constant: bool) -> Self {
        Self {
            callback,
            is_constant,
        }
    }

    /// 设置要使用的回调。
    /// 映射到 `CallbackProperty.prototype.setCallback`。
    pub fn set_callback<F>(&mut self, callback: F, is_constant: bool)
    where
        F: Fn(&JulianDate) -> PropertyValue + Send + Sync + 'static,
    {
        // 同时更新回调闭包与常量标志。
        self.callback = Arc::new(callback);
        self.is_constant = is_constant;
    }
}

impl DynProperty for CallbackProperty {
    /// 直接返回构造时给定的常量标志。
    fn is_constant(&self) -> bool {
        self.is_constant
    }

    /// 将给定时间传入回调函数并返回其结果。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        (self.callback)(time)
    }

    /// 返回类型名 `CallbackProperty`。
    fn type_name(&self) -> &'static str {
        "CallbackProperty"
    }

    /// 仅当对方同为回调属性、回调共享同一 `Arc` 且常量标志相同时判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<CallbackProperty>() {
            Some(o) => {
                Arc::ptr_eq(&self.callback, &o.callback)
                    && self.is_constant == o.is_constant
            }
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    // 本测试集覆盖各具体属性类型的求值、插值、外推与相等比较行为。
    use super::*;
    use glam::DVec3;

    /// 基于给定秒偏移构造一个测试用儒略日（固定基准日 2451545.0）。
    fn jd(seconds: f64) -> JulianDate {
        JulianDate::new(2451545.0, seconds)
    }

    /// 验证常量属性始终为常量、任意时刻返回固定值，且相等比较区分同值/异值。
    #[test]
    fn test_constant_property() {
        let p = ConstantProperty::new(PropertyValue::Number(42.0));
        assert!(p.is_constant());
        assert_eq!(p.get_value(&jd(0.0)), PropertyValue::Number(42.0));
        assert_eq!(p.type_name(), "ConstantProperty");

        let q = ConstantProperty::new(PropertyValue::Number(42.0));
        assert!(p.equals(&q));
        let r = ConstantProperty::new(PropertyValue::Number(7.0));
        assert!(!p.equals(&r));
    }

    /// 验证数值型采样属性在两点之间线性插值（中点、四分点均正确）。
    #[test]
    fn test_sampled_property_linear_number() {
        let mut p = SampledProperty::new(PackableType::Number);
        assert!(p.is_constant()); // 尚无样本

        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        assert!(!p.is_constant());
        assert_eq!(p.sample_count(), 2);

        // 精确匹配。
        assert_eq!(p.get_value(&jd(0.0)), PropertyValue::Number(0.0));
        assert_eq!(p.get_value(&jd(10.0)), PropertyValue::Number(100.0));
        // 插值中点。
        assert_eq!(p.get_value(&jd(5.0)), PropertyValue::Number(50.0));
        // 四分之一处。
        assert_eq!(p.get_value(&jd(2.5)), PropertyValue::Number(25.0));
    }

    /// 验证默认外推类型为 None 时，超出样本时间范围的求值返回未定义。
    #[test]
    fn test_sampled_property_out_of_range_none() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        // 默认外推为 NONE。
        assert_eq!(p.get_value(&jd(-1.0)), PropertyValue::Undefined);
        assert_eq!(p.get_value(&jd(11.0)), PropertyValue::Undefined);
    }

    /// 验证 Hold 外推：范围外保持首个/末个样本值不变。
    #[test]
    fn test_sampled_property_hold_extrapolation() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(0.0), &PropertyValue::Number(5.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(15.0), &[]);
        p.set_backward_extrapolation_type(ExtrapolationType::Hold);
        p.set_forward_extrapolation_type(ExtrapolationType::Hold);
        assert_eq!(p.get_value(&jd(-5.0)), PropertyValue::Number(5.0));
        assert_eq!(p.get_value(&jd(20.0)), PropertyValue::Number(15.0));
    }

    /// 验证 Extrapolate 外推：沿样本斜率向范围外延伸求值。
    #[test]
    fn test_sampled_property_extrapolate() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        p.set_forward_extrapolation_type(ExtrapolationType::Extrapolate);
        p.set_backward_extrapolation_type(ExtrapolationType::Extrapolate);
        assert_eq!(p.get_value(&jd(20.0)), PropertyValue::Number(200.0));
        assert_eq!(p.get_value(&jd(-5.0)), PropertyValue::Number(-50.0));
    }

    /// 验证外推时长限制：在时长内保持、超出则返回未定义。
    #[test]
    fn test_sampled_property_extrapolation_duration() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        p.set_forward_extrapolation_type(ExtrapolationType::Hold);
        p.set_forward_extrapolation_duration(5.0);
        // 在时长范围内：保持。
        assert_eq!(p.get_value(&jd(12.0)), PropertyValue::Number(100.0));
        // 超出时长：undefined。
        assert_eq!(p.get_value(&jd(20.0)), PropertyValue::Undefined);
    }

    /// 验证笛卡尔三分量采样属性在各分量上线性插值。
    #[test]
    fn test_sampled_property_cartesian3() {
        let mut p = SampledProperty::new(PackableType::Cartesian3);
        p.add_sample(
            jd(0.0),
            &PropertyValue::Cartesian3(DVec3::new(0.0, 0.0, 0.0)),
            &[],
        );
        p.add_sample(
            jd(10.0),
            &PropertyValue::Cartesian3(DVec3::new(10.0, 20.0, 30.0)),
            &[],
        );
        let mid = p.get_value(&jd(5.0));
        assert_eq!(
            mid,
            PropertyValue::Cartesian3(DVec3::new(5.0, 10.0, 15.0))
        );
    }

    /// 验证四元数采样属性在中点处得到球面插值（绕 Z 轴 45 度）。
    #[test]
    fn test_sampled_property_quaternion_slerp_like() {
        use glam::DQuat;
        use std::f64::consts::FRAC_PI_2;
        let mut p = SampledProperty::new(PackableType::Quaternion);
        p.add_sample(
            jd(0.0),
            &PropertyValue::Quaternion(DQuat::IDENTITY),
            &[],
        );
        p.add_sample(
            jd(10.0),
            &PropertyValue::Quaternion(DQuat::from_rotation_z(FRAC_PI_2)),
            &[],
        );
        // 中点应为绕 Z 轴 45 度的旋转。
        let mid = p.get_value(&jd(5.0));
        if let PropertyValue::Quaternion(q) = mid {
            let expected = DQuat::from_rotation_z(FRAC_PI_2 / 2.0);
            let dot = q.dot(expected).abs();
            assert!((dot - 1.0).abs() < 1e-9, "dot = {dot}");
        } else {
            panic!("expected quaternion");
        }
    }

    /// 验证乱序添加的样本会被合并为按时间排序的内部存储。
    #[test]
    fn test_sampled_property_out_of_order_insertion() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(5.0), &PropertyValue::Number(50.0), &[]);
        assert_eq!(p.sample_count(), 3);
        assert_eq!(p.times()[0], jd(0.0));
        assert_eq!(p.times()[1], jd(5.0));
        assert_eq!(p.times()[2], jd(10.0));
        assert_eq!(p.get_value(&jd(2.5)), PropertyValue::Number(25.0));
    }

    /// 验证向已存在时间添加样本会覆盖旧值而不增加样本数。
    #[test]
    fn test_sampled_property_overwrite_existing() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        // 覆盖 t=10 处的样本。
        p.add_sample(jd(10.0), &PropertyValue::Number(200.0), &[]);
        assert_eq!(p.sample_count(), 2);
        assert_eq!(p.get_value(&jd(10.0)), PropertyValue::Number(200.0));
    }

    /// 验证带一阶导数的埃尔米特插值能精确重现三次多项式。
    #[test]
    fn test_sampled_property_hermite_with_derivatives() {
        // f(t) = t^3 on [0, 1]: f(0)=0, f'(0)=0, f(1)=1, f'(1)=3.
        let mut p = SampledProperty::with_derivative_types(
            PackableType::Number,
            Some(vec![PackableType::Number]),
        );
        p.set_interpolation_options(Some(InterpolationAlgorithmKind::Hermite), Some(3));
        p.add_sample(
            jd(0.0),
            &PropertyValue::Number(0.0),
            &[PropertyValue::Number(0.0)],
        );
        p.add_sample(
            jd(1.0),
            &PropertyValue::Number(1.0),
            &[PropertyValue::Number(3.0)],
        );
        let v = p.get_value(&jd(0.5));
        assert_eq!(v, PropertyValue::Number(0.125));
    }

    /// 验证按时间移除单样本：命中返回 true，重复移除返回 false。
    #[test]
    fn test_sampled_property_remove_sample() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(5.0), &PropertyValue::Number(50.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        assert!(p.remove_sample(&jd(5.0)));
        assert_eq!(p.sample_count(), 2);
        assert!(!p.remove_sample(&jd(5.0)));
    }

    /// 验证按时间区间批量移除闭区间内的全部样本。
    #[test]
    fn test_sampled_property_remove_samples_interval() {
        let mut p = SampledProperty::new(PackableType::Number);
        for i in 0..=10 {
            p.add_sample(jd(i as f64), &PropertyValue::Number(i as f64), &[]);
        }
        let interval = TimeInterval::new(jd(3.0), jd(7.0), true, true);
        p.remove_samples_interval(&interval);
        // 已移除 t=3,4,5,6,7 -> 剩余 6 个样本。
        assert_eq!(p.sample_count(), 6);
    }

    /// 验证 get_sample 支持负索引倒数与越界返回空。
    #[test]
    fn test_sampled_property_get_sample_negative_index() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        p.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        assert_eq!(p.get_sample(-1), Some(jd(10.0)));
        assert_eq!(p.get_sample(0), Some(jd(0.0)));
        assert_eq!(p.get_sample(5), None);
    }

    /// 验证从打包数组（时间偏移+值交替）批量添加样本。
    #[test]
    fn test_sampled_property_add_samples_packed_array() {
        let mut p = SampledProperty::new(PackableType::Number);
        let epoch = jd(0.0);
        // 每个样本：[time_offset, value]。
        let packed = [0.0, 0.0, 10.0, 100.0, 5.0, 50.0];
        p.add_samples_packed_array(&packed, &epoch);
        assert_eq!(p.sample_count(), 3);
        assert_eq!(p.get_value(&jd(2.5)), PropertyValue::Number(25.0));
    }

    /// 验证采样属性相等比较：样本集相同则等，不则不等。
    #[test]
    fn test_sampled_property_equals() {
        let mut a = SampledProperty::new(PackableType::Number);
        a.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        let mut b = SampledProperty::new(PackableType::Number);
        b.add_sample(jd(0.0), &PropertyValue::Number(0.0), &[]);
        assert!(a.equals(&b));

        b.add_sample(jd(10.0), &PropertyValue::Number(100.0), &[]);
        assert!(!a.equals(&b));
    }

    /// 验证只有一个样本时精确匹配生效、但无法插值。
    #[test]
    fn test_sampled_property_single_sample() {
        let mut p = SampledProperty::new(PackableType::Number);
        p.add_sample(jd(5.0), &PropertyValue::Number(42.0), &[]);
        // 精确匹配生效。
        assert_eq!(p.get_value(&jd(5.0)), PropertyValue::Number(42.0));
        // 只有一个样本时无法插值。
        assert_eq!(p.get_value(&jd(6.0)), PropertyValue::Undefined);
    }

    /// 验证区间集合属性按落入区间返回对应值，区间外返回未定义。
    #[test]
    fn test_time_interval_collection_property() {
        let mut p = TimeIntervalCollectionProperty::new();
        assert!(p.is_constant());

        p.add_interval(
            TimeInterval::new(jd(0.0), jd(10.0), true, false),
            Some(PropertyValue::Number(1.0)),
        );
        p.add_interval(
            TimeInterval::new(jd(10.0), jd(20.0), true, true),
            Some(PropertyValue::Number(2.0)),
        );
        assert!(!p.is_constant());

        assert_eq!(p.get_value(&jd(5.0)), PropertyValue::Number(1.0));
        assert_eq!(p.get_value(&jd(15.0)), PropertyValue::Number(2.0));
        assert_eq!(p.get_value(&jd(25.0)), PropertyValue::Undefined);

        let q = TimeIntervalCollectionProperty::new();
        assert!(!p.equals(&q));
    }

    /// 验证组合属性将每个区间委托给内部子属性求值。
    #[test]
    fn test_composite_property() {
        let mut p = CompositeProperty::new();
        assert!(p.is_constant());

        let c1: Arc<dyn DynProperty> = Arc::new(ConstantProperty::new(PropertyValue::Number(1.0)));
        let mut sampled = SampledProperty::new(PackableType::Number);
        sampled.add_sample(jd(10.0), &PropertyValue::Number(10.0), &[]);
        sampled.add_sample(jd(20.0), &PropertyValue::Number(20.0), &[]);
        let c2: Arc<dyn DynProperty> = Arc::new(sampled);

        p.add_interval(
            TimeInterval::new(jd(0.0), jd(10.0), true, false),
            Some(c1),
        );
        p.add_interval(
            TimeInterval::new(jd(10.0), jd(20.0), true, true),
            Some(c2),
        );
        assert!(!p.is_constant());

        assert_eq!(p.get_value(&jd(5.0)), PropertyValue::Number(1.0));
        assert_eq!(p.get_value(&jd(15.0)), PropertyValue::Number(15.0));
        assert_eq!(p.get_value(&jd(25.0)), PropertyValue::Undefined);
    }

    /// 验证回调属性延迟求值，且相等比较基于共享 Arc 指针。
    #[test]
    fn test_callback_property() {
        let p = CallbackProperty::new(|t| PropertyValue::Number(t.day_number as f64), false);
        assert!(!p.is_constant());
        assert_eq!(p.get_value(&jd(7.0)), PropertyValue::Number(2451545.0));

        let q = CallbackProperty::new(|t| PropertyValue::Number(t.day_number as f64), false);
        // 不同的 closure -> 不相等。
        assert!(!p.equals(&q));

        // 相同的 Arc -> 相等。
        let shared: CallbackFn = Arc::new(|t| PropertyValue::Number(t.day_number as f64));
        let r = CallbackProperty::from_arc(Arc::clone(&shared), true);
        let s = CallbackProperty::from_arc(shared, true);
        assert!(r.equals(&s));
    }

    /// 验证辅助函数对缺失属性的处理（视为常量、取值返回未定义）。
    #[test]
    fn test_property_helpers() {
        let c = ConstantProperty::new(PropertyValue::Number(1.0));
        assert!(property_is_constant(None));
        assert!(property_is_constant(Some(&c)));
        assert_eq!(
            property_get_value_or_undefined(None, &jd(0.0)),
            PropertyValue::Undefined
        );
        assert_eq!(
            property_get_value_or_undefined(Some(&c), &jd(0.0)),
            PropertyValue::Number(1.0)
        );
    }

    /// 验证不同类型属性间的相等比较总返回 false。
    #[test]
    fn test_cross_type_equals_false() {
        let c = ConstantProperty::new(PropertyValue::Number(1.0));
        let s = SampledProperty::new(PackableType::Number);
        assert!(!c.equals(&s));
        assert!(!s.equals(&c));
    }
}
