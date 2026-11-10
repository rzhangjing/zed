//! 位置属性：其值为一个世界位置（`Cartesian3`）并带有
//! 相关联参考系的属性。
//!
//! 具体实现包括常量位置 `ConstantPositionProperty`、采样位置
//! `SampledPositionProperty`、组合位置 `CompositePositionProperty`、
//! 时间区间集合位置 `TimeIntervalCollectionPositionProperty` 与回调位置
//! `CallbackPositionProperty`；它们都携带一个与之关联的参考系，
//! 并可在固定（fixed）与惯性（inertial）两系之间求值。

use crate::property_system::interpolation::{ExtrapolationType, InterpolationAlgorithmKind};
use crate::property_system::property::{CompositeProperty, DynProperty, SampledProperty};
use crate::property_system::value::{PackableType, PropertyValue, ReferenceFrame};
use cesium_geospatial::transforms::compute_icrf_to_fixed_matrix;
use cesium_time::{JulianDate, TimeInterval, TimeIntervalCollection, TimeIntervalData};
use glam::DVec3;
use std::any::Any;
use std::sync::Arc;

/// 在给定时间处将一个位置从一个参考系转换到另一个。
///
/// 映射到 `PositionProperty.convertToReferenceFrame`。当两参考系相同
/// 时值原样返回。否则为 `time` 计算 ICRF 到 fixed 的旋转
/// 矩阵；inertial→fixed 乘以该矩阵，
/// fixed→inertial 乘以其转置。
pub fn convert_to_reference_frame(
    time: &JulianDate,
    value: DVec3,
    input_frame: ReferenceFrame,
    output_frame: ReferenceFrame,
) -> Option<DVec3> {
    if input_frame == output_frame {
        return Some(value);
    }

    let julian_date_seconds = time.total_days() * 86400.0;
    let icrf_to_fixed = compute_icrf_to_fixed_matrix(julian_date_seconds)?;
    match input_frame {
        ReferenceFrame::Inertial => Some(icrf_to_fixed * value),
        ReferenceFrame::Fixed => Some(icrf_to_fixed.transpose() * value),
    }
}

/// 将可选位置包装为 `PropertyValue`。
fn position_to_value(position: Option<DVec3>) -> PropertyValue {
    match position {
        Some(p) => PropertyValue::Cartesian3(p),
        None => PropertyValue::Undefined,
    }
}

// ---------------------------------------------------------------------------
// ConstantPositionProperty
// ---------------------------------------------------------------------------

/// 一种位置属性，其值相对于其定义所在的参考系不发生变化；
/// 它是位置属性中最简单的形态，内部固化一个可选位置向量
/// 与一个参考系，任意时刻求值都基于该固定向量做系间转换。
#[derive(Debug, Clone, Default)]
pub struct ConstantPositionProperty {
    /// 已存储的位置向量；为 `None` 时表示该属性未定义值。
    value: Option<DVec3>,
    /// 位置向量所属的参考系（fixed 或惯性），求值时以此为基准转换。
    reference_frame: ReferenceFrame,
}

impl ConstantPositionProperty {
    /// 在 fixed 参考系中创建新的常量位置属性。
    pub fn new(value: DVec3) -> Self {
        Self {
            value: Some(value),
            reference_frame: ReferenceFrame::Fixed,
        }
    }

    /// 在指定参考系中创建新的常量位置属性。
    /// 映射到 `new ConstantPositionProperty(value, referenceFrame)`。
    pub fn with_reference_frame(value: DVec3, reference_frame: ReferenceFrame) -> Self {
        Self {
            value: Some(value),
            reference_frame,
        }
    }

    /// 创建无值的常量位置属性。
    pub fn undefined() -> Self {
        Self {
            value: None,
            reference_frame: ReferenceFrame::Fixed,
        }
    }

    /// 设置值，并可选地设置参考系。
    /// 映射到 `ConstantPositionProperty.prototype.setValue`。
    pub fn set_value(&mut self, value: Option<DVec3>, reference_frame: Option<ReferenceFrame>) {
        self.value = value;
        if let Some(frame) = reference_frame {
            self.reference_frame = frame;
        }
    }

    /// 已存储的值（在此属性的参考系中）。
    pub fn value(&self) -> Option<DVec3> {
        self.value
    }

    /// 在所提供的参考系中获取 `time` 处的位置。
    /// 映射到 `ConstantPositionProperty.prototype.getValueInReferenceFrame`。
    pub fn position_in_reference_frame(
        &self,
        time: &JulianDate,
        reference_frame: ReferenceFrame,
    ) -> Option<DVec3> {
        let value = self.value?;
        convert_to_reference_frame(time, value, self.reference_frame, reference_frame)
    }
}

impl DynProperty for ConstantPositionProperty {
    /// 未定义或位于 fixed 系时为常量；惯性系位置以 fixed 表示会随
    /// 时间旋转，故不为常量。
    fn is_constant(&self) -> bool {
        // 惯性系位置在用 fixed 系表示时会随时间变化，因此仅当未定义
        // 或为 fixed 系时才是常量。
        self.value.is_none() || self.reference_frame == ReferenceFrame::Fixed
    }

    /// 在 fixed 参考系中求值当前位置，包装为 `PropertyValue`。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        position_to_value(self.position_in_reference_frame(time, ReferenceFrame::Fixed))
    }

    /// 返回类型名 `ConstantPositionProperty`。
    fn type_name(&self) -> &'static str {
        "ConstantPositionProperty"
    }

    /// 仅当对方同为常量位置属性且内部值与参考系都相等时判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<ConstantPositionProperty>() {
            Some(o) => self.value == o.value && self.reference_frame == o.reference_frame,
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// 返回此属性定义所在的参考系。
    fn reference_frame(&self) -> Option<ReferenceFrame> {
        Some(self.reference_frame)
    }

    /// 将已存储位置转换到 `frame` 后求值，缺失时返回 `None`。
    fn get_value_in_reference_frame(
        &self,
        time: &JulianDate,
        frame: ReferenceFrame,
    ) -> Option<PropertyValue> {
        self.position_in_reference_frame(time, frame)
            .map(PropertyValue::Cartesian3)
    }
}

// ---------------------------------------------------------------------------
// SampledPositionProperty
// ---------------------------------------------------------------------------

/// 一个同时也是位置属性的 `SampledProperty`：将采样属性按位置
/// 语义包装，额外携带一个参考系与导数个数，求值时委托内部采样
/// 属性插值并做系间转换。
#[derive(Debug, Clone)]
pub struct SampledPositionProperty {
    /// 内部的采样属性，存算样本时间、位置与可选导数。
    property: SampledProperty,
    /// 位置样本所属的参考系，用于插值后的坐标转换。
    reference_frame: ReferenceFrame,
    /// 每个样本随位置一同提供的导数个数（如速度/加速度）。
    number_of_derivatives: usize,
}

impl SampledPositionProperty {
    /// 创建新的采样位置属性。
    ///
    /// 映射到 `new SampledPositionProperty(referenceFrame, numberOfDerivatives)`。
    pub fn new(reference_frame: ReferenceFrame, number_of_derivatives: usize) -> Self {
        let derivative_types = if number_of_derivatives > 0 {
            Some(vec![PackableType::Cartesian3; number_of_derivatives])
        } else {
            None
        };
        Self {
            property: SampledProperty::with_derivative_types(
                PackableType::Cartesian3,
                derivative_types,
            ),
            reference_frame,
            number_of_derivatives,
        }
    }

    /// 在 fixed 参考系中创建新的采样位置属性，不带导数。
    pub fn fixed() -> Self {
        Self::new(ReferenceFrame::Fixed, 0)
    }

    /// 随每个位置一同提供的导数数量。
    /// 映射到 `numberOfDerivatives`。
    pub fn number_of_derivatives(&self) -> usize {
        self.number_of_derivatives
    }

    /// 插值次数。映射到 `interpolationDegree`。
    pub fn interpolation_degree(&self) -> usize {
        self.property.interpolation_degree()
    }

    /// 插值算法。映射到 `interpolationAlgorithm`。
    pub fn interpolation_algorithm(&self) -> InterpolationAlgorithmKind {
        self.property.interpolation_algorithm()
    }

    /// 当前存储的样本数量。
    pub fn sample_count(&self) -> usize {
        self.property.sample_count()
    }

    /// 设置插值位置时所使用的算法与次数。
    /// 映射到 `SampledPositionProperty.prototype.setInterpolationOptions`。
    pub fn set_interpolation_options(
        &mut self,
        algorithm: Option<InterpolationAlgorithmKind>,
        degree: Option<usize>,
    ) {
        self.property.set_interpolation_options(algorithm, degree);
    }

    /// 设置前推外推类型。映射到 `forwardExtrapolationType`。
    pub fn set_forward_extrapolation_type(&mut self, value: ExtrapolationType) {
        self.property.set_forward_extrapolation_type(value);
    }

    /// 设置前推外推时长。
    /// 映射到 `forwardExtrapolationDuration`。
    pub fn set_forward_extrapolation_duration(&mut self, value: f64) {
        self.property.set_forward_extrapolation_duration(value);
    }

    /// 设置后推外推类型。
    /// 映射到 `backwardExtrapolationType`。
    pub fn set_backward_extrapolation_type(&mut self, value: ExtrapolationType) {
        self.property.set_backward_extrapolation_type(value);
    }

    /// 设置后推外推时长。
    /// 映射到 `backwardExtrapolationDuration`。
    pub fn set_backward_extrapolation_duration(&mut self, value: f64) {
        self.property.set_backward_extrapolation_duration(value);
    }

    /// 添加一个新样本。映射到 `SampledPositionProperty.prototype.addSample`。
    pub fn add_sample(&mut self, time: JulianDate, position: DVec3, derivatives: &[DVec3]) {
        let value = PropertyValue::Cartesian3(position);
        let deriv_values: Vec<PropertyValue> = derivatives
            .iter()
            .map(|d| PropertyValue::Cartesian3(*d))
            .collect();
        self.property.add_sample(time, &value, &deriv_values);
    }

    /// 通过并行数组添加多个样本。
    /// 映射到 `SampledPositionProperty.prototype.addSamples`。
    pub fn add_samples(
        &mut self,
        times: &[JulianDate],
        positions: &[DVec3],
        derivatives: Option<&[Vec<DVec3>]>,
    ) {
        let values: Vec<PropertyValue> = positions
            .iter()
            .map(|p| PropertyValue::Cartesian3(*p))
            .collect();
        let deriv_values: Option<Vec<Vec<PropertyValue>>> = derivatives.map(|ds| {
            ds.iter()
                .map(|dv| dv.iter().map(|d| PropertyValue::Cartesian3(*d)).collect())
                .collect()
        });
        self.property
            .add_samples(times, &values, deriv_values.as_deref());
    }

    /// 以单个打包数组添加样本，其中每个样本为一个时间偏移
    /// （相对于 `epoch` 的秒数）后接打包的位置与
    /// 导数。
    /// 映射到 `SampledPositionProperty.prototype.addSamplesPackedArray`。
    pub fn add_samples_packed_array(&mut self, packed_samples: &[f64], epoch: &JulianDate) {
        self.property
            .add_samples_packed_array(packed_samples, epoch);
    }

    /// 若存在则移除给定时间处的样本。
    /// 映射到 `SampledPositionProperty.prototype.removeSample`。
    pub fn remove_sample(&mut self, time: &JulianDate) -> bool {
        self.property.remove_sample(time)
    }

    /// 移除给定时间区间内的所有样本。
    /// 映射到 `SampledPositionProperty.prototype.removeSamples`。
    pub fn remove_samples_interval(&mut self, time_interval: &TimeInterval) {
        self.property.remove_samples_interval(time_interval);
    }

    /// 在所提供的参考系中获取 `time` 处的位置。
    /// 映射到 `SampledPositionProperty.prototype.getValueInReferenceFrame`。
    pub fn position_in_reference_frame(
        &self,
        time: &JulianDate,
        reference_frame: ReferenceFrame,
    ) -> Option<DVec3> {
        match self.property.get_value(time) {
            PropertyValue::Cartesian3(p) => {
                convert_to_reference_frame(time, p, self.reference_frame, reference_frame)
            }
            _ => None,
        }
    }
}

impl DynProperty for SampledPositionProperty {
    /// 直接委托内部采样属性判定常量性。
    fn is_constant(&self) -> bool {
        self.property.is_constant()
    }

    /// 在 fixed 参考系中插值求值当前位置，包装为 `PropertyValue`。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        position_to_value(self.position_in_reference_frame(time, ReferenceFrame::Fixed))
    }

    /// 返回类型名 `SampledPositionProperty`。
    fn type_name(&self) -> &'static str {
        "SampledPositionProperty"
    }

    /// 仅当同为采样位置属性、内部采样集相等且参考系一致时判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<SampledPositionProperty>() {
            Some(o) => {
                self.property.equals(&o.property) && self.reference_frame == o.reference_frame
            }
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// 返回此属性定义所在的参考系。
    fn reference_frame(&self) -> Option<ReferenceFrame> {
        Some(self.reference_frame)
    }

    /// 先按时间插值，再将结果转换到 `frame`；无有效插值时返回 `None`。
    fn get_value_in_reference_frame(
        &self,
        time: &JulianDate,
        frame: ReferenceFrame,
    ) -> Option<PropertyValue> {
        self.position_in_reference_frame(time, frame)
            .map(PropertyValue::Cartesian3)
    }
}

// ---------------------------------------------------------------------------
// CompositePositionProperty
// ---------------------------------------------------------------------------

/// 一个同时也是位置属性的 `CompositeProperty`。
///
/// 每个区间的数据本身就是一个位置属性；求值时定位到包含
/// 该时刻的区间，再委托内部属性的 `getValueInReferenceFrame`。
#[derive(Clone)]
pub struct CompositePositionProperty {
    /// 内部的组合属性，按时间区间存放若个子位置属性。
    composite: CompositeProperty,
    /// 此位置自我呈现时采用的“首选”参考系。
    reference_frame: ReferenceFrame,
}

impl CompositePositionProperty {
    /// 创建新的组合位置属性。
    /// 映射到 `new CompositePositionProperty(referenceFrame)`。
    pub fn new(reference_frame: ReferenceFrame) -> Self {
        Self {
            composite: CompositeProperty::new(),
            reference_frame,
        }
    }

    /// 底层的区间集合。映射到 `intervals`。
    pub fn intervals(&self) -> &TimeIntervalCollection<Arc<dyn DynProperty>> {
        self.composite.intervals()
    }

    /// 添加一个数据为另一个（位置）属性的区间。
    pub fn add_interval(&mut self, interval: TimeInterval, data: Option<Arc<dyn DynProperty>>) {
        self.composite.add_interval(interval, data);
    }

    /// 设置此位置自我呈现的“首选”参考系。
    /// 映射到 `referenceFrame` setter。
    pub fn set_reference_frame(&mut self, frame: ReferenceFrame) {
        self.reference_frame = frame;
    }

    /// 在所提供的参考系中获取 `time` 处的位置。
    /// 映射到 `CompositePositionProperty.prototype.getValueInReferenceFrame`。
    pub fn position_in_reference_frame(
        &self,
        time: &JulianDate,
        reference_frame: ReferenceFrame,
    ) -> Option<DVec3> {
        let inner = self
            .composite
            .intervals()
            .find_data_for_interval_containing_date(time)?;
        match inner.get_value_in_reference_frame(time, reference_frame)? {
            PropertyValue::Cartesian3(p) => Some(p),
            _ => None,
        }
    }
}

impl DynProperty for CompositePositionProperty {
    /// 委托内部组合属性判定常量性（空区间集即为常量）。
    fn is_constant(&self) -> bool {
        self.composite.is_constant()
    }

    /// 定位包含 `time` 的区间并在 fixed 系求值，包装为 `PropertyValue`。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        position_to_value(self.position_in_reference_frame(time, ReferenceFrame::Fixed))
    }

    /// 返回类型名 `CompositePositionProperty`。
    fn type_name(&self) -> &'static str {
        "CompositePositionProperty"
    }

    /// 仅当同为组合位置属性、参考系一致且内部组合相等时判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<CompositePositionProperty>() {
            Some(o) => {
                self.reference_frame == o.reference_frame
                    && self.composite.equals(&o.composite)
            }
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// 返回此属性首选的参考系。
    fn reference_frame(&self) -> Option<ReferenceFrame> {
        Some(self.reference_frame)
    }

    /// 定位区间并委托内部属性在 `frame` 中取值；无匹配区间时返回 `None`。
    fn get_value_in_reference_frame(
        &self,
        time: &JulianDate,
        frame: ReferenceFrame,
    ) -> Option<PropertyValue> {
        self.position_in_reference_frame(time, frame)
            .map(PropertyValue::Cartesian3)
    }
}

// ---------------------------------------------------------------------------
// TimeIntervalCollectionPositionProperty
// ---------------------------------------------------------------------------

/// 比较两个位置数据是否相同，用于区间集合添加/比较时的相等判定。
fn position_same_data(left: &DVec3, right: &DVec3) -> bool {
    *left == *right
}

/// 一个同时也是位置属性的 `TimeIntervalCollectionProperty`：位置数据
/// 直接以 `DVec3` 形式按不重叠时间区间存放，求值时定位到包含时刻的
/// 区间取其常量位置，再做系间转换。
#[derive(Debug, Clone)]
pub struct TimeIntervalCollectionPositionProperty {
    /// 按时间区间存放位置数据的集合，区间之间不允许重叠。
    intervals: TimeIntervalCollection<DVec3>,
    /// 已存位置所属的参考系，用于求值时的坐标转换。
    reference_frame: ReferenceFrame,
}

impl TimeIntervalCollectionPositionProperty {
    /// 创建新的时间区间集合位置属性。
    /// 映射到 `new TimeIntervalCollectionPositionProperty(referenceFrame)`。
    pub fn new(reference_frame: ReferenceFrame) -> Self {
        Self {
            intervals: TimeIntervalCollection::new(),
            reference_frame,
        }
    }

    /// 底层的区间集合。映射到 `intervals`。
    pub fn intervals(&self) -> &TimeIntervalCollection<DVec3> {
        &self.intervals
    }

    /// 添加一个带给定位置数据的区间。
    pub fn add_interval(&mut self, interval: TimeInterval, data: Option<DVec3>) {
        let tid = TimeIntervalData::new(interval, data);
        self.intervals.add_interval(tid, &position_same_data);
    }

    /// 在所提供的参考系中获取 `time` 处的位置。
    ///
    /// 映射到
    /// `TimeIntervalCollectionPositionProperty.prototype.getValueInReferenceFrame`。
    pub fn position_in_reference_frame(
        &self,
        time: &JulianDate,
        reference_frame: ReferenceFrame,
    ) -> Option<DVec3> {
        let position = self.intervals.find_data_for_interval_containing_date(time)?;
        convert_to_reference_frame(time, *position, self.reference_frame, reference_frame)
    }
}

impl DynProperty for TimeIntervalCollectionPositionProperty {
    /// 区间集为空时视为常量（无任何随时间变化的数据）。
    fn is_constant(&self) -> bool {
        self.intervals.is_empty()
    }

    /// 定位包含 `time` 的区间并在 fixed 系求值，包装为 `PropertyValue`。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        position_to_value(self.position_in_reference_frame(time, ReferenceFrame::Fixed))
    }

    /// 返回类型名 `TimeIntervalCollectionPositionProperty`。
    fn type_name(&self) -> &'static str {
        "TimeIntervalCollectionPositionProperty"
    }

    /// 仅当同为区间集合位置属性、区间数据相等且参考系一致时判定相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other
            .as_any()
            .downcast_ref::<TimeIntervalCollectionPositionProperty>()
        {
            Some(o) => {
                self.intervals.equals(&o.intervals, &position_same_data)
                    && self.reference_frame == o.reference_frame
            }
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// 返回此属性定义所在的参考系。
    fn reference_frame(&self) -> Option<ReferenceFrame> {
        Some(self.reference_frame)
    }

    /// 定位区间取位置后转换到 `frame`；无匹配区间时返回 `None`。
    fn get_value_in_reference_frame(
        &self,
        time: &JulianDate,
        frame: ReferenceFrame,
    ) -> Option<PropertyValue> {
        self.position_in_reference_frame(time, frame)
            .map(PropertyValue::Cartesian3)
    }
}

// ---------------------------------------------------------------------------
// CallbackPositionProperty
// ---------------------------------------------------------------------------

/// `CallbackPositionProperty` 使用的回调签名：给定时间，
/// 返回属性参考系中的位置（或 `None`）。
pub type PositionCallbackFn = Arc<dyn Fn(&JulianDate) -> Option<DVec3> + Send + Sync>;

/// 其值由回调函数延迟求值的位置属性：每次求值都调用内部回调得到
/// 参考系中的位置，再按需转换；回调可为任意实现 `Fn` 的闭包。
pub struct CallbackPositionProperty {
    /// 延迟求值的回调，给定时间返回参考系中的位置（或 `None`）。
    callback: PositionCallbackFn,
    /// 标志位：告知调用方此回调是否对所有时间返回相同值。
    is_constant: bool,
    /// 回调返回位置所属的参考系，用于求值时的坐标转换。
    reference_frame: ReferenceFrame,
}

impl CallbackPositionProperty {
    /// 创建新的回调位置属性。
    ///
    /// 映射到 `new CallbackPositionProperty(callback, isConstant, referenceFrame)`。
    pub fn new(
        callback: PositionCallbackFn,
        is_constant: bool,
        reference_frame: ReferenceFrame,
    ) -> Self {
        Self {
            callback,
            is_constant,
            reference_frame,
        }
    }

    /// 替换回调与常量标志。
    /// 映射到 `CallbackPositionProperty.prototype.setCallback`。
    pub fn set_callback(&mut self, callback: PositionCallbackFn, is_constant: bool) {
        self.callback = callback;
        self.is_constant = is_constant;
    }

    /// 在所提供的参考系中获取 `time` 处的位置。
    ///
    /// 映射到 `CallbackPositionProperty.prototype.getValueInReferenceFrame`。
    pub fn position_in_reference_frame(
        &self,
        time: &JulianDate,
        reference_frame: ReferenceFrame,
    ) -> Option<DVec3> {
        let value = (self.callback)(time)?;
        convert_to_reference_frame(time, value, self.reference_frame, reference_frame)
    }
}

impl DynProperty for CallbackPositionProperty {
    /// 直接返回构造/设置时给定的常量标志。
    fn is_constant(&self) -> bool {
        self.is_constant
    }

    /// 调用回调取位置并在 fixed 系求值，包装为 `PropertyValue`。
    fn get_value(&self, time: &JulianDate) -> PropertyValue {
        position_to_value(self.position_in_reference_frame(time, ReferenceFrame::Fixed))
    }

    /// 返回类型名 `CallbackPositionProperty`。
    fn type_name(&self) -> &'static str {
        "CallbackPositionProperty"
    }

    /// 仅当回调指针相等（共享同一 `Arc`）且常量标志与参考系都一致时相等。
    fn equals(&self, other: &dyn DynProperty) -> bool {
        match other.as_any().downcast_ref::<CallbackPositionProperty>() {
            Some(o) => {
                Arc::ptr_eq(&self.callback, &o.callback)
                    && self.is_constant == o.is_constant
                    && self.reference_frame == o.reference_frame
            }
            None => false,
        }
    }

    /// 以 `Any` 引用暴露自身，供向下转型使用。
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// 返回此属性定义所在的参考系。
    fn reference_frame(&self) -> Option<ReferenceFrame> {
        Some(self.reference_frame)
    }

    /// 调用回调取位置后转换到 `frame`；回调返回 `None` 时结果也为 `None`。
    fn get_value_in_reference_frame(
        &self,
        time: &JulianDate,
        frame: ReferenceFrame,
    ) -> Option<PropertyValue> {
        self.position_in_reference_frame(time, frame)
            .map(PropertyValue::Cartesian3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cesium_time::TimeInterval;
    use glam::DVec3;

    /// 构造测试用儒略日：固定 JD 2451545.0 基准，仅变化秒偏移，
    /// 以便在同一基准上比较不同采样时刻。
    fn t(seconds: f64) -> JulianDate {
        JulianDate::new(2451545.0, seconds)
    }

    /// 验证同一参考系之间的转换不改变向量：fixed→fixed 与
    /// inertial→inertial 都应原样返回输入位置。
    #[test]
    fn test_convert_same_frame_returns_unchanged() {
        let v = DVec3::new(1.0, 2.0, 3.0);
        let out = convert_to_reference_frame(&t(0.0), v, ReferenceFrame::Fixed, ReferenceFrame::Fixed);
        assert_eq!(out, Some(v));
        let out = convert_to_reference_frame(
            &t(0.0),
            v,
            ReferenceFrame::Inertial,
            ReferenceFrame::Inertial,
        );
        assert_eq!(out, Some(v));
    }

    /// 验证惯性↔固定双向转换的回环一致性：先转过去再转回来应
    /// 重建原向量，且旋转保持向量长度不变。
    #[test]
    fn test_convert_roundtrip_inertial_fixed() {
        let v = DVec3::new(1_000_000.0, 2_000_000.0, 3_000_000.0);
        let time = t(43200.0);
        let fixed = convert_to_reference_frame(&time, v, ReferenceFrame::Inertial, ReferenceFrame::Fixed)
            .unwrap();
        // 旋转保持长度。
        assert!((fixed.length() - v.length()).abs() < 1e-6);
        let back =
            convert_to_reference_frame(&time, fixed, ReferenceFrame::Fixed, ReferenceFrame::Inertial)
                .unwrap();
        assert!(back.abs_diff_eq(v, 1e-6));
    }

    /// 验证同一惯性向量在不同时刻转到 fixed 系会得到不同结果，
    /// 体现地球自转带来的时变旋转。
    #[test]
    fn test_convert_changes_with_time() {
        // 惯性位置在用 fixed 系表示时会随时间旋转。
        let v = DVec3::new(1_000_000.0, 0.0, 0.0);
        let f1 = convert_to_reference_frame(&t(0.0), v, ReferenceFrame::Inertial, ReferenceFrame::Fixed)
            .unwrap();
        let f2 = convert_to_reference_frame(
            &t(21600.0),
            v,
            ReferenceFrame::Inertial,
            ReferenceFrame::Fixed,
        )
        .unwrap();
        assert!(!f1.abs_diff_eq(f2, 1.0));
    }

    /// 验证 fixed 系常量位置：为常量、参考系为 fixed，且各接口求值
    /// 都返回已存储的向量。
    #[test]
    fn test_constant_position_fixed_frame() {
        let p = DVec3::new(1.0, 2.0, 3.0);
        let prop = ConstantPositionProperty::new(p);
        assert!(prop.is_constant());
        assert_eq!(prop.reference_frame(), Some(ReferenceFrame::Fixed));
        assert_eq!(prop.get_value(&t(0.0)), PropertyValue::Cartesian3(p));
        assert_eq!(
            prop.get_value_in_reference_frame(&t(0.0), ReferenceFrame::Fixed),
            Some(PropertyValue::Cartesian3(p))
        );
    }

    /// 验证惯性系常量位置并非常量（在 fixed 系中旋转），但在其自身
    /// 惯性系中任意时刻都返回已存向量，且可回环转换。
    #[test]
    fn test_constant_position_inertial_frame_not_constant() {
        let p = DVec3::new(1.0, 2.0, 3.0);
        let prop = ConstantPositionProperty::with_reference_frame(p, ReferenceFrame::Inertial);
        // 惯性系位置并非常量（它们在 fixed 系中旋转）。
        assert!(!prop.is_constant());

        // 在其自身参考系中的值，在任意时间都是已存储的值。
        assert_eq!(
            prop.get_value_in_reference_frame(&t(0.0), ReferenceFrame::Inertial),
            Some(PropertyValue::Cartesian3(p))
        );
        assert_eq!(
            prop.get_value_in_reference_frame(&t(1000.0), ReferenceFrame::Inertial),
            Some(PropertyValue::Cartesian3(p))
        );

        // fixed 系的值不同于已存储的惯性值……
        let fixed = prop.get_value(&t(0.0));
        assert_ne!(fixed, PropertyValue::Cartesian3(p));
        // ……并可回环转换回已存储的值。
        match fixed {
            PropertyValue::Cartesian3(fp) => {
                let back = convert_to_reference_frame(
                    &t(0.0),
                    fp,
                    ReferenceFrame::Fixed,
                    ReferenceFrame::Inertial,
                )
                .unwrap();
                assert!(back.abs_diff_eq(p, 1e-12));
            }
            _ => panic!("expected Cartesian3"),
        }
    }

    /// 验证未定义常量位置：仍为常量，求值返回未定义哨兵，参考系
    /// 取值返回 `None`。
    #[test]
    fn test_constant_position_undefined() {
        let prop = ConstantPositionProperty::undefined();
        assert!(prop.is_constant());
        assert_eq!(prop.get_value(&t(0.0)), PropertyValue::Undefined);
        assert_eq!(
            prop.get_value_in_reference_frame(&t(0.0), ReferenceFrame::Fixed),
            None
        );
    }

    /// 验证常量位置相等比较：同值同系相等，参考系不同则不等。
    #[test]
    fn test_constant_position_equals() {
        let p = DVec3::new(1.0, 2.0, 3.0);
        let a = ConstantPositionProperty::new(p);
        let b = ConstantPositionProperty::new(p);
        let c = ConstantPositionProperty::with_reference_frame(p, ReferenceFrame::Inertial);
        assert!(a.equals(&b));
        assert!(!a.equals(&c));
    }

    /// 验证采样位置线性插值：两点间中点取半分，精确采样时刻返回
    /// 精确值。
    #[test]
    fn test_sampled_position_linear_interpolation() {
        let mut prop = SampledPositionProperty::fixed();
        prop.add_sample(t(0.0), DVec3::new(0.0, 0.0, 0.0), &[]);
        prop.add_sample(t(10.0), DVec3::new(10.0, 20.0, 30.0), &[]);
        assert!(!prop.is_constant());

        let mid = prop.position_in_reference_frame(&t(5.0), ReferenceFrame::Fixed).unwrap();
        assert!(mid.abs_diff_eq(DVec3::new(5.0, 10.0, 15.0), 1e-12));

        // 精确的采样时间返回精确的值。
        let at0 = prop.position_in_reference_frame(&t(0.0), ReferenceFrame::Fixed).unwrap();
        assert!(at0.abs_diff_eq(DVec3::ZERO, 1e-12));
    }

    /// 验证惯性系采样位置：自身系插值直接，fixed 系插值需经旋转
    /// 转换且与显式转换一致。
    #[test]
    fn test_sampled_position_inertial_frame() {
        let mut prop = SampledPositionProperty::new(ReferenceFrame::Inertial, 0);
        prop.add_sample(t(0.0), DVec3::new(1.0, 0.0, 0.0), &[]);
        prop.add_sample(t(10.0), DVec3::new(2.0, 0.0, 0.0), &[]);

        // 在其自身（惯性）系中，插值是直接的。
        let inertial = prop
            .position_in_reference_frame(&t(5.0), ReferenceFrame::Inertial)
            .unwrap();
        assert!(inertial.abs_diff_eq(DVec3::new(1.5, 0.0, 0.0), 1e-12));

        // fixed 系的值是旋转后的插值。
        let fixed = prop.position_in_reference_frame(&t(5.0), ReferenceFrame::Fixed).unwrap();
        let expected = convert_to_reference_frame(
            &t(5.0),
            DVec3::new(1.5, 0.0, 0.0),
            ReferenceFrame::Inertial,
            ReferenceFrame::Fixed,
        )
        .unwrap();
        assert!(fixed.abs_diff_eq(expected, 1e-12));
    }

    /// 验证带速度导数的 Hermite 插值可重建三次曲线：以 p(t)=t³ 的
    /// 样本与导数 3t² 插值得到接近理论中点值。
    #[test]
    fn test_sampled_position_with_derivatives() {
        // 使用速度导数的 Hermite 插值可重建一个三次曲线。
        let mut prop = SampledPositionProperty::new(ReferenceFrame::Fixed, 1);
        prop.set_interpolation_options(Some(InterpolationAlgorithmKind::Hermite), Some(1));
        // p(t) = t^3 along x; p'(t) = 3t^2.
        for i in 0..=2 {
            let tf = i as f64;
            prop.add_sample(
                t(tf),
                DVec3::new(tf * tf * tf, 0.0, 0.0),
                &[DVec3::new(3.0 * tf * tf, 0.0, 0.0)],
            );
        }
        let mid = prop.position_in_reference_frame(&t(1.5), ReferenceFrame::Fixed).unwrap();
        assert!((mid.x - 3.375).abs() < 1e-9);
    }

    /// 验证外推类型：默认 NONE 时样本外返回未定义，改为 HOLD 后
    /// 前推保持末值。
    #[test]
    fn test_sampled_position_extrapolation_none() {
        let mut prop = SampledPositionProperty::fixed();
        prop.add_sample(t(0.0), DVec3::new(0.0, 0.0, 0.0), &[]);
        prop.add_sample(t(10.0), DVec3::new(10.0, 0.0, 0.0), &[]);
        // 默认外推为 NONE：在样本之外 → undefined。
        assert_eq!(prop.get_value(&t(20.0)), PropertyValue::Undefined);
        assert_eq!(
            prop.position_in_reference_frame(&t(20.0), ReferenceFrame::Fixed),
            None
        );

        prop.set_forward_extrapolation_type(ExtrapolationType::Hold);
        let held = prop.position_in_reference_frame(&t(20.0), ReferenceFrame::Fixed).unwrap();
        assert!(held.abs_diff_eq(DVec3::new(10.0, 0.0, 0.0), 1e-12));
    }

    /// 验证采样位置相等比较：相同样本与参考系相等，参考系不同则不等。
    #[test]
    fn test_sampled_position_equals() {
        let mut a = SampledPositionProperty::fixed();
        a.add_sample(t(0.0), DVec3::new(1.0, 2.0, 3.0), &[]);
        let mut b = SampledPositionProperty::fixed();
        b.add_sample(t(0.0), DVec3::new(1.0, 2.0, 3.0), &[]);
        let c = SampledPositionProperty::new(ReferenceFrame::Inertial, 0);
        assert!(a.equals(&b));
        assert!(!a.equals(&c));
    }

    /// 验证组合位置委托内部属性：不同区间取到对应常量位置，区间
    /// 之外返回未定义。
    #[test]
    fn test_composite_position_delegates_to_inner() {
        let mut prop = CompositePositionProperty::new(ReferenceFrame::Fixed);
        let p1 = DVec3::new(1.0, 0.0, 0.0);
        let p2 = DVec3::new(2.0, 0.0, 0.0);
        prop.add_interval(
            TimeInterval::new(t(0.0), t(10.0), true, false),
            Some(Arc::new(ConstantPositionProperty::new(p1)) as Arc<dyn DynProperty>),
        );
        prop.add_interval(
            TimeInterval::new(t(10.0), t(20.0), true, true),
            Some(Arc::new(ConstantPositionProperty::new(p2)) as Arc<dyn DynProperty>),
        );
        assert!(!prop.is_constant());

        assert_eq!(prop.get_value(&t(5.0)), PropertyValue::Cartesian3(p1));
        assert_eq!(prop.get_value(&t(15.0)), PropertyValue::Cartesian3(p2));
        // 在所有区间之外 → undefined。
        assert_eq!(prop.get_value(&t(30.0)), PropertyValue::Undefined);
    }

    /// 验证组合位置对内部惯性属性的透传：以惯性系查询时返回内部
    /// 属性已存储的值，系间转换由内部属性自行处理。
    #[test]
    fn test_composite_position_inner_inertial() {
        // 内部属性自行处理其参考系转换：一个惯性内部
        // 属性在为 INERTIAL 查询时返回其已存储的值。
        let mut prop = CompositePositionProperty::new(ReferenceFrame::Fixed);
        let p = DVec3::new(5.0, 6.0, 7.0);
        prop.add_interval(
            TimeInterval::new(t(0.0), t(100.0), true, true),
            Some(
                Arc::new(ConstantPositionProperty::with_reference_frame(
                    p,
                    ReferenceFrame::Inertial,
                )) as Arc<dyn DynProperty>,
            ),
        );
        assert_eq!(
            prop.position_in_reference_frame(&t(50.0), ReferenceFrame::Inertial),
            Some(p)
        );
    }

    /// 验证区间集合位置：空集合为常量，加入区间后非常量，区间内取值、
    /// 区间外返回未定义。
    #[test]
    fn test_tic_position_property() {
        let mut prop = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
        assert!(prop.is_constant()); // 空 → 常量

        let p = DVec3::new(100.0, 200.0, 300.0);
        prop.add_interval(TimeInterval::new(t(0.0), t(10.0), true, true), Some(p));
        assert!(!prop.is_constant());

        assert_eq!(prop.get_value(&t(5.0)), PropertyValue::Cartesian3(p));
        assert_eq!(prop.get_value(&t(11.0)), PropertyValue::Undefined);
    }

    /// 验证惯性系区间集合位置：自身系返回已存值，fixed 系为经旋转的
    /// 转换结果。
    #[test]
    fn test_tic_position_inertial_frame() {
        let mut prop = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Inertial);
        let p = DVec3::new(100.0, 200.0, 300.0);
        prop.add_interval(TimeInterval::new(t(0.0), t(10.0), true, true), Some(p));

        // 自身系：已存储的值。
        assert_eq!(
            prop.position_in_reference_frame(&t(5.0), ReferenceFrame::Inertial),
            Some(p)
        );
        // fixed 系：已旋转。
        let fixed = prop.position_in_reference_frame(&t(5.0), ReferenceFrame::Fixed).unwrap();
        let expected = convert_to_reference_frame(
            &t(5.0),
            p,
            ReferenceFrame::Inertial,
            ReferenceFrame::Fixed,
        )
        .unwrap();
        assert!(fixed.abs_diff_eq(expected, 1e-12));
    }

    /// 验证区间集合位置相等比较：相同区间与参考系相等，参考系不同则不等。
    #[test]
    fn test_tic_position_equals() {
        let mut a = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
        let mut b = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Fixed);
        let p = DVec3::new(1.0, 2.0, 3.0);
        a.add_interval(TimeInterval::new(t(0.0), t(10.0), true, true), Some(p));
        b.add_interval(TimeInterval::new(t(0.0), t(10.0), true, true), Some(p));
        assert!(a.equals(&b));

        let c = TimeIntervalCollectionPositionProperty::new(ReferenceFrame::Inertial);
        assert!(!a.equals(&c));
    }

    /// 验证回调位置属性：按回调返回的日数生成 x 坐标，且标记为非常量。
    #[test]
    fn test_callback_position_property() {
        let callback: PositionCallbackFn = Arc::new(|time: &JulianDate| {
            let d = time.day_number as f64;
            Some(DVec3::new(d, 0.0, 0.0))
        });
        let prop = CallbackPositionProperty::new(callback, false, ReferenceFrame::Fixed);
        assert!(!prop.is_constant());
        assert_eq!(
            prop.get_value(&t(7.0)),
            PropertyValue::Cartesian3(DVec3::new(2451545.0, 0.0, 0.0))
        );
    }

    /// 验证回调返回 `None` 时求值取未定义哨兵，同时保留常量标志。
    #[test]
    fn test_callback_position_none_value() {
        let callback: PositionCallbackFn = Arc::new(|_time: &JulianDate| None);
        let prop = CallbackPositionProperty::new(callback, true, ReferenceFrame::Fixed);
        assert!(prop.is_constant());
        assert_eq!(prop.get_value(&t(0.0)), PropertyValue::Undefined);
    }

    /// 验证惯性系回调位置：自身系返回已算值，fixed 系为经旋转的转换结果。
    #[test]
    fn test_callback_position_inertial() {
        let callback: PositionCallbackFn =
            Arc::new(|_time: &JulianDate| Some(DVec3::new(1.0, 2.0, 3.0)));
        let prop = CallbackPositionProperty::new(callback, true, ReferenceFrame::Inertial);

        assert_eq!(
            prop.position_in_reference_frame(&t(0.0), ReferenceFrame::Inertial),
            Some(DVec3::new(1.0, 2.0, 3.0))
        );
        let fixed = prop.position_in_reference_frame(&t(0.0), ReferenceFrame::Fixed).unwrap();
        let expected = convert_to_reference_frame(
            &t(0.0),
            DVec3::new(1.0, 2.0, 3.0),
            ReferenceFrame::Inertial,
            ReferenceFrame::Fixed,
        )
        .unwrap();
        assert!(fixed.abs_diff_eq(expected, 1e-12));
    }

    /// 验证回调位置相等比较：共享同一回调 `Arc` 且标志/参考系一致则相等，
    /// 常量标志不同则不等。
    #[test]
    fn test_callback_position_equals() {
        let cb: PositionCallbackFn = Arc::new(|_time: &JulianDate| Some(DVec3::ONE));
        let a = CallbackPositionProperty::new(Arc::clone(&cb), true, ReferenceFrame::Fixed);
        let b = CallbackPositionProperty::new(Arc::clone(&cb), true, ReferenceFrame::Fixed);
        let c = CallbackPositionProperty::new(Arc::clone(&cb), false, ReferenceFrame::Fixed);
        assert!(a.equals(&b));
        assert!(!a.equals(&c));
    }
}
