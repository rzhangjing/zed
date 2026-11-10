//! 采样属性的插值算法。
//!
//! 本模块提供采样属性求值所用的三类插值算法（线性 `LinearApproximation`、
//! 拉格朗日多项式 `LagrangePolynomialApproximation`、埃尔米特多项式
//! `HermitePolynomialApproximation`）以及外推类型枚举 `ExtrapolationType`，
//! 供 `SampledProperty` 在离散样本之间构造连续值。
//!
//! 所有算法都在打包的 `f64` 表上操作，与 CesiumJS 完全一致：
//! - `x_table`：自变量值（时间，以秒计），递增顺序。
//! - `y_table`：因变量值；当每个样本有 `y_stride` 个分量时，其布局
//!   为 `{p1, q1, w1, p2, q2, w2, ...}`。

use cesium_geospatial::math_utils::factorial;

/// 决定当查询时间超出可用样本数据边界时，插值结果如何被外推：
/// 不外推、保持端值，或沿首/末段趋势继续外推。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExtrapolationType {
    /// 不进行外推；在样本范围之外的值为 undefined。
    #[default]
    None,
    /// 在样本数据范围之外时使用第一个或最后一个值。
    Hold,
    /// 对外推该值。
    Extrapolate,
}

impl ExtrapolationType {
    /// CesiumJS 所使用的数值（NONE=0、HOLD=1、EXTRAPOLATE=2）。
    pub fn to_u32(self) -> u32 {
        match self {
            ExtrapolationType::None => 0,
            ExtrapolationType::Hold => 1,
            ExtrapolationType::Extrapolate => 2,
        }
    }

    /// 由 CesiumJS 的数值创建外推类型。
    pub fn from_u32(value: u32) -> Self {
        match value {
            1 => ExtrapolationType::Hold,
            2 => ExtrapolationType::Extrapolate,
            _ => ExtrapolationType::None,
        }
    }
}

/// 用于插值打包因变量表的算法。
///
/// 映射到 CesiumJS `InterpolationAlgorithm` 接口，由
/// `LinearApproximation`、`LagrangePolynomialApproximation` 与
/// `HermitePolynomialApproximation` 实现。
pub trait InterpolationAlgorithm: Send + Sync {
    /// 算法类型名（`"Linear"`、`"Lagrange"`、`"Hermite"`）。
    fn name(&self) -> &'static str;

    /// 给定所需次数，返回插值所需的数据点数量。
    ///
    /// `input_order` 是输入的阶数（0 表示仅有数据，
    /// 1 表示数据及其导数，依此类推）。
    fn get_required_data_points(&self, degree: usize, input_order: usize) -> usize;

    /// 此算法是否实现 `interpolate`（即支持
    /// 导数输入/输出）。在 CesiumJS 中仅 Hermite 支持。
    fn supports_derivatives(&self) -> bool {
        false
    }

    /// 使用该算法插值（零阶，无导数）。
    ///
    /// 返回一个包含 `y_stride` 个插值分量的 `Vec`。
    fn interpolate_order_zero(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
    ) -> Vec<f64>;

    /// 使用导数输入与输出进行插值。
    ///
    /// 默认实现（用于那些在 CesiumJS 中未定义
    /// `interpolate` 的算法）回退到零阶插值，并将导数输出
    /// 填充为零。
    fn interpolate(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
        _input_order: usize,
        output_order: usize,
    ) -> Vec<f64> {
        let mut result = self.interpolate_order_zero(x, x_table, y_table, y_stride);
        result.resize(y_stride * (output_order + 1), 0.0);
        result
    }
}

/// 线性插值算法：仅用相邻两个样本构造一次多项式。它恒定需要 2 个
/// 数据点，且不支持导数输入输出，因此是最廉价、最常用的插值方式，
/// 适合样本足够密集、只需分段线性近似的场景。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinearApproximation;

impl InterpolationAlgorithm for LinearApproximation {
    /// 返回算法名 `Linear`。
    fn name(&self) -> &'static str {
        "Linear"
    }

    /// 由于线性插值只能生成一次多项式，
    /// 因此总是返回 2。
    fn get_required_data_points(&self, _degree: usize, _input_order: usize) -> usize {
        2
    }

    /// 在恰好两个样本之间对每个分量做线性插值，返回 `y_stride` 个
    /// 插值分量；调用方须保证 `x_table` 长度为 2 且两点不相等。
    fn interpolate_order_zero(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
    ) -> Vec<f64> {
        debug_assert_eq!(
            x_table.len(),
            2,
            "The xTable provided to the linear interpolator must have exactly two elements."
        );
        debug_assert!(
            y_stride > 0,
            "There must be at least 1 dependent variable for each independent variable."
        );

        let mut result = vec![0.0; y_stride];
        let x0 = x_table[0];
        let x1 = x_table[1];
        debug_assert_ne!(x0, x1, "Divide by zero error: xTable[0] and xTable[1] are equal");

        for i in 0..y_stride {
            let y0 = y_table[i];
            let y1 = y_table[i + y_stride];
            result[i] = ((y1 - y0) * x + x1 * y0 - x0 * y1) / (x1 - x0);
        }
        result
    }
}

/// Lagrange 多项式插值：用全部样本点构造通过它们的低次多项式，
/// 所需数据点为次数加一（至少 2），不支持导数，精度高于线性但
/// 随点数增多可能振荡（龙格现象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LagrangePolynomialApproximation;

impl InterpolationAlgorithm for LagrangePolynomialApproximation {
    /// 返回算法名 `Lagrange`。
    fn name(&self) -> &'static str {
        "Lagrange"
    }

    /// 返回次数加一与 2 的较大者，即构造该次数多项式所需的样本点数。
    fn get_required_data_points(&self, degree: usize, _input_order: usize) -> usize {
        (degree + 1).max(2)
    }

    /// 按定义式累加各基函数的贡献：对每个样本 i 计算其 Lagrange 基
    /// 函数在 x 处的取值，再乘以对应分量累入结果。
    fn interpolate_order_zero(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
    ) -> Vec<f64> {
        // 结果向量按分量累加各 Lagrange 基函数的贡献。
        let mut result = vec![0.0; y_stride];
        let length = x_table.len();

        // 外层遍历每个样本，构造对应的基函数并累加。
        for i in 0..length {
            let mut coefficient = 1.0;
            // 内层连乘 (x - xj)/(xi - xj)，跳过 j == i 的自身项。
            for j in 0..length {
                if j != i {
                    let diff_x = x_table[i] - x_table[j];
                    coefficient *= (x - x_table[j]) / diff_x;
                }
            }
            // 把该基函数系数乘到每个分量上，累入对应结果槽。
            for j in 0..y_stride {
                result[j] += coefficient * y_table[i * y_stride + j];
            }
        }
        result
    }
}

/// Hermite 多项式插值：在差商表中同时利用函数值与导数值，是三种类
/// 型中唯一支持导数输入/输出的算法，可用更少样本达到更高精度，
/// 常用于位置连同速度一起采样的轨迹插值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HermitePolynomialApproximation;

impl InterpolationAlgorithm for HermitePolynomialApproximation {
    /// 返回算法名 `Hermite`。
    fn name(&self) -> &'static str {
        "Hermite"
    }

    /// 按输入阶数折算所需点数：(次数+1)/(输入阶+1)，且至少为 2。
    fn get_required_data_points(&self, degree: usize, input_order: usize) -> usize {
        ((degree + 1) / (input_order + 1)).max(2)
    }

    /// Hermite 是唯一支持导数输入/输出的内置算法，恒返回 true。
    fn supports_derivatives(&self) -> bool {
        true
    }

    /// 零阶求值：先构建各分量的差商表，再沿 Newton 形式累加系数项；
    /// 当相邻 x 相同（重复样本）时改从导数槽取系数并除以阶乘。
    fn interpolate_order_zero(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
    ) -> Vec<f64> {
        let length = x_table.len();
        let mut result = vec![0.0; y_stride];
        if length == 0 || y_stride == 0 {
            return result;
        }

        // coefficients[s][i] 为分量 s 保存差商表的行。
        let mut coefficients: Vec<Vec<Vec<f64>>> =
            (0..y_stride).map(|_| (0..length).map(|_| Vec::new()).collect()).collect();

        let z_indices: Vec<usize> = (0..length).collect();

        let mut highest_non_zero_coef = length - 1;
        for (s, coef_s) in coefficients.iter_mut().enumerate() {
            for &z in &z_indices {
                let index = z * y_stride + s;
                coef_s[0].push(y_table[index]);
            }

            for i in 1..length {
                let mut non_zero_coefficients = false;
                for j in 0..(length - i) {
                    let zj = x_table[z_indices[j]];
                    let zn = x_table[z_indices[j + i]];

                    let numerator;
                    if zn - zj <= 0.0 {
                        let index = z_indices[j] * y_stride + y_stride * i + s;
                        numerator = y_table[index];
                        coef_s[i].push(numerator / factorial(i as u32) as f64);
                    } else {
                        numerator = coef_s[i - 1][j + 1] - coef_s[i - 1][j];
                        coef_s[i].push(numerator / (zn - zj));
                    }
                    non_zero_coefficients = non_zero_coefficients || numerator != 0.0;
                }

                if !non_zero_coefficients {
                    highest_non_zero_coef = i - 1;
                }
            }
        }

        // 在 interpolateOrderZero 中，外层循环只会以 d = 0 运行
        // （在 CesiumJS 中为 `for (d = 0, len = 0; d <= len; d++)`）。
        // 显式的索引循环模仿 CesiumJS 的差商累加，
        // 无法用普通的迭代器 zip 表达。
        #[allow(clippy::needless_range_loop)]
        for i in 0..=highest_non_zero_coef {
            let temp_term =
                calculate_coefficient_term(x, &z_indices, x_table, 0, i, &mut Vec::new());
            for (result_s, coef_s) in result.iter_mut().zip(coefficients.iter()) {
                *result_s += coef_s[i][0] * temp_term;
            }
        }

        result
    }

    /// 带导数的求值：构造含导数条目的 zIndices 与差商系数表，再对
    /// 每个输出阶 d 累加 Newton 项，得到值与各阶导数。
    fn interpolate(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
        input_order: usize,
        output_order: usize,
    ) -> Vec<f64> {
        let result_length = y_stride * (output_order + 1);
        let mut result = vec![0.0; result_length];

        let length = x_table.len();
        // zIndices 数组保存我们所查看范围内 xTable 值
        // 地址的副本。
        let z_len = length * (input_order + 1);
        let mut z_indices = vec![0usize; z_len];
        for i in 0..length {
            for j in 0..(input_order + 1) {
                z_indices[i * (input_order + 1) + j] = i;
            }
        }

        // 差商表按三角数 tmp 展开为一维打包缓冲，逐分量存放。
        let tmp = z_len * (z_len + 1) / 2;
        let mut coefficients = vec![0.0f64; y_stride * tmp];
        // 计算各阶差商系数，并得到最高非零差商阶用于截断累加。
        let highest_non_zero_coef = fill_coefficient_list(
            &mut coefficients,
            &z_indices,
            x_table,
            y_table,
            y_stride,
            input_order,
        );

        // 输出阶上限取最高非零差商阶与 output_order 的较小者。
        let loop_stop = highest_non_zero_coef.min(output_order as isize);
        if loop_stop < 0 {
            return result;
        }
        for d in 0..=loop_stop as usize {
            for i in d..=highest_non_zero_coef as usize {
                let temp_term = calculate_coefficient_term(
                    x,
                    &z_indices,
                    x_table,
                    d,
                    i,
                    &mut Vec::new(),
                );
                let dim_two = row_offset(i, z_len);
                for s in 0..y_stride {
                    let dim_one = s * tmp;
                    let coef = coefficients[dim_one + dim_two];
                    result[s + d * y_stride] += coef * temp_term;
                }
            }
        }

        result
    }
}

/// 差商行 `i` 在打包系数缓冲区内的偏移。
///
/// 行 `i` 保存 `z_len - i` 个条目；CesiumJS 将其计算为
/// `Math.floor((i * (1 - i)) / 2) + zIndicesLength * i`。
fn row_offset(i: usize, z_len: usize) -> usize {
    let signed = (i as isize) * (1 - i as isize) / 2 + (z_len * i) as isize;
    signed as usize
}

/// 填充 Hermite `interpolate` 所需的一维打包差商系数表，逐分量、
/// 逐阶计算差商，返回最高非零差商的阶（用于截断累加）。
fn fill_coefficient_list(
    coefficients: &mut [f64],
    z_indices: &[usize],
    x_table: &[f64],
    y_table: &[f64],
    y_stride: usize,
    input_order: usize,
) -> isize {
    let mut highest_non_zero: isize = -1;
    let z_len = z_indices.len();
    let tmp = z_len * (z_len + 1) / 2;

    // 逐分量填充：dim_one 为该分量在一维缓冲中的起始偏移。
    for s in 0..y_stride {
        let dim_one = s * tmp;

        // 零阶差商即函数值本身，直接取样本对应槽。
        for j in 0..z_len {
            let index = z_indices[j] * y_stride * (input_order + 1) + s;
            coefficients[dim_one + j] = y_table[index];
        }

        for i in 1..z_len {
            let mut coef_index = 0usize;
            let dim_two = row_offset(i, z_len);
            let mut non_zero_coefficients = false;

            for j in 0..(z_len - i) {
                let zj = x_table[z_indices[j]];
                let zn = x_table[z_indices[j + i]];

                let numerator;
                if zn - zj <= 0.0 {
                    let index = z_indices[j] * y_stride * (input_order + 1) + y_stride * i + s;
                    numerator = y_table[index];
                    coefficients[dim_one + dim_two + coef_index] =
                        numerator / factorial(i as u32) as f64;
                    coef_index += 1;
                } else {
                    let dim_two_minus_one = row_offset(i - 1, z_len);
                    numerator = coefficients[dim_one + dim_two_minus_one + j + 1]
                        - coefficients[dim_one + dim_two_minus_one + j];
                    coefficients[dim_one + dim_two + coef_index] = numerator / (zn - zj);
                    coef_index += 1;
                }
                non_zero_coefficients = non_zero_coefficients || numerator != 0.0;
            }

            if non_zero_coefficients {
                highest_non_zero = highest_non_zero.max(i as isize);
            }
        }
    }

    highest_non_zero
}

/// 计算 Newton 形式系数多项式的一项，或它的某个导数。
///
/// 当 `deriv_order == 0` 时，这是对所有非保留的 `i < term_order`
/// 求 `(x - xTable[zIndices[i]])` 的乘积。当 `deriv_order > 0` 时，
/// 它是该乘积的 `deriv_order` 阶导数，通过递归地
/// 对所有逐一保留一个因子的方式求和计算得出。
fn calculate_coefficient_term(
    x: f64,
    z_indices: &[usize],
    x_table: &[f64],
    deriv_order: usize,
    term_order: usize,
    reserved_indices: &mut Vec<usize>,
) -> f64 {
    if deriv_order > 0 {
        let mut result = 0.0;
        for i in 0..term_order {
            if !reserved_indices.contains(&i) {
                reserved_indices.push(i);
                result += calculate_coefficient_term(
                    x,
                    z_indices,
                    x_table,
                    deriv_order - 1,
                    term_order,
                    reserved_indices,
                );
                reserved_indices.pop();
            }
        }
        return result;
    }

    let mut result = 1.0;
    for i in 0..term_order {
        if !reserved_indices.contains(&i) {
            result *= x - x_table[z_indices[i]];
        }
    }
    result
}

/// 内置的插值算法，可按值选择。
///
/// 此枚举分发到 [`LinearApproximation`]、
/// [`LagrangePolynomialApproximation`] 与 [`HermitePolynomialApproximation`]，
/// 并由 `SampledProperty` 用于存储所选的算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InterpolationAlgorithmKind {
    #[default]
    Linear,
    Lagrange,
    Hermite,
}

impl InterpolationAlgorithmKind {
    /// 返回此种类对应的算法对象。
    pub fn algorithm(&self) -> &'static dyn InterpolationAlgorithm {
        match self {
            InterpolationAlgorithmKind::Linear => &LinearApproximation,
            InterpolationAlgorithmKind::Lagrange => &LagrangePolynomialApproximation,
            InterpolationAlgorithmKind::Hermite => &HermitePolynomialApproximation,
        }
    }

    /// 解析 CesiumJS 的算法类型名（`"Linear"`、`"Lagrange"`、
    /// `"Hermite"`）。
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Linear" => Some(InterpolationAlgorithmKind::Linear),
            "Lagrange" => Some(InterpolationAlgorithmKind::Lagrange),
            "Hermite" => Some(InterpolationAlgorithmKind::Hermite),
            _ => None,
        }
    }
}

impl InterpolationAlgorithm for InterpolationAlgorithmKind {
    /// 委托底层算法返回类型名。
    fn name(&self) -> &'static str {
        self.algorithm().name()
    }

    /// 委托底层算法计算所需数据点数量。
    fn get_required_data_points(&self, degree: usize, input_order: usize) -> usize {
        self.algorithm().get_required_data_points(degree, input_order)
    }

    /// 委托底层算法判断是否支持导数。
    fn supports_derivatives(&self) -> bool {
        self.algorithm().supports_derivatives()
    }

    /// 委托底层算法做零阶插值。
    fn interpolate_order_zero(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
    ) -> Vec<f64> {
        self.algorithm()
            .interpolate_order_zero(x, x_table, y_table, y_stride)
    }

    /// 委托底层算法做带导数的插值。
    fn interpolate(
        &self,
        x: f64,
        x_table: &[f64],
        y_table: &[f64],
        y_stride: usize,
        input_order: usize,
        output_order: usize,
    ) -> Vec<f64> {
        self.algorithm()
            .interpolate(x, x_table, y_table, y_stride, input_order, output_order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    /// 验证外推类型与数值编码 (NONE=0/HOLD=1/EXTRAPOLATE=2)
    /// 的双向往返，以及默认值为 None。
    #[test]
    fn test_extrapolation_type_roundtrip() {
        assert_eq!(ExtrapolationType::None.to_u32(), 0);
        assert_eq!(ExtrapolationType::Hold.to_u32(), 1);
        assert_eq!(ExtrapolationType::Extrapolate.to_u32(), 2);
        assert_eq!(ExtrapolationType::from_u32(0), ExtrapolationType::None);
        assert_eq!(ExtrapolationType::from_u32(1), ExtrapolationType::Hold);
        assert_eq!(ExtrapolationType::from_u32(2), ExtrapolationType::Extrapolate);
        assert_eq!(ExtrapolationType::default(), ExtrapolationType::None);
    }

    /// 验证线性算法无论次数/输入阶如何都只需 2 个数据点。
    #[test]
    fn test_linear_required_data_points() {
        assert_eq!(LinearApproximation.get_required_data_points(1, 0), 2);
        assert_eq!(LinearApproximation.get_required_data_points(5, 0), 2);
        assert_eq!(LinearApproximation.get_required_data_points(9, 2), 2);
    }

    /// 验证线性插值在中点命中：对 y=2x+1 的两个分量分别插值，
    /// x=5 处应得 (11, 9)。
    #[test]
    fn test_linear_interpolate_midpoint() {
        // y = 2x + 1，在 x = 0 与 x = 10 处采样（每个样本两个分量）。
        let x_table = [0.0, 10.0];
        let y_table = [1.0, -1.0, 21.0, 19.0];
        let result = LinearApproximation.interpolate_order_zero(5.0, &x_table, &y_table, 2);
        assert!((result[0] - 11.0).abs() < EPS);
        assert!((result[1] - 9.0).abs() < EPS);
    }

    /// 验证线性插值在采样节点上精确：x 取两端点时应原样返回
    /// 对应的 y 值。
    #[test]
    fn test_linear_interpolate_at_nodes() {
        let x_table = [-4.0, 2.0];
        let y_table = [3.0, 7.0];
        let r0 = LinearApproximation.interpolate_order_zero(-4.0, &x_table, &y_table, 1);
        let r1 = LinearApproximation.interpolate_order_zero(2.0, &x_table, &y_table, 1);
        assert!((r0[0] - 3.0).abs() < EPS);
        assert!((r1[0] - 7.0).abs() < EPS);
    }

    /// 验证负的相对 x（距最后一样本的秒数，可为负）仍满足线性公式。
    #[test]
    fn test_linear_negative_x_extrapolates() {
        // xTable 值是相对的（距最后一个样本的秒数）且可能
        // 为负；公式仍须成立。
        let x_table = [-10.0, 0.0];
        let y_table = [0.0, 100.0];
        let result = LinearApproximation.interpolate_order_zero(-5.0, &x_table, &y_table, 1);
        assert!((result[0] - 50.0).abs() < EPS);
    }

    /// 验证 Lagrange 所需点数等于次数加一且下限为 2。
    #[test]
    fn test_lagrange_required_data_points() {
        assert_eq!(LagrangePolynomialApproximation.get_required_data_points(0, 0), 2);
        assert_eq!(LagrangePolynomialApproximation.get_required_data_points(1, 0), 2);
        assert_eq!(LagrangePolynomialApproximation.get_required_data_points(2, 0), 3);
        assert_eq!(LagrangePolynomialApproximation.get_required_data_points(7, 0), 8);
    }

    /// 验证 Lagrange 对二次多项式精确：用三个采样点重建 y=x²-2x+3，
    /// 在多个内部 x 处插值与解析值一致。
    #[test]
    fn test_lagrange_quadratic_exact() {
        // y = x^2 - 2x + 3，在 x = -1、0、2 处采样。
        let x_table = [-1.0, 0.0, 2.0];
        let y_table = [6.0, 3.0, 3.0];
        for &x in &[0.5, 1.0, -0.5, 1.7] {
            let expected = x * x - 2.0 * x + 3.0;
            let result =
                LagrangePolynomialApproximation.interpolate_order_zero(x, &x_table, &y_table, 1);
            assert!(
                (result[0] - expected).abs() < EPS,
                "x={x}: got {}, expected {expected}",
                result[0]
            );
        }
    }

    /// 验证 Lagrange 多分量插值：p=x、q=x³ 两分量共用一张表，在
    /// x=1.5 处分别插出 1.5 与 3.375。
    #[test]
    fn test_lagrange_multi_component() {
        // 两个分量：p = x、q = x^3，在 x = 0、1、2、3 处。
        let x_table = [0.0, 1.0, 2.0, 3.0];
        let y_table = [0.0, 0.0, 1.0, 1.0, 2.0, 8.0, 3.0, 27.0];
        let result =
            LagrangePolynomialApproximation.interpolate_order_zero(1.5, &x_table, &y_table, 2);
        assert!((result[0] - 1.5).abs() < EPS);
        assert!((result[1] - 3.375).abs() < EPS);
    }

    /// 验证 Hermite 所需点数随输入阶数下降：无导数时为次数加一，
    /// 带一阶导数时约减半。
    #[test]
    fn test_hermite_required_data_points() {
        assert_eq!(HermitePolynomialApproximation.get_required_data_points(1, 0), 2);
        assert_eq!(HermitePolynomialApproximation.get_required_data_points(3, 0), 4);
        // 带导数（inputOrder=1）：(degree+1)/2 个点。
        assert_eq!(HermitePolynomialApproximation.get_required_data_points(3, 1), 2);
        assert_eq!(HermitePolynomialApproximation.get_required_data_points(5, 1), 3);
        assert_eq!(HermitePolynomialApproximation.get_required_data_points(0, 0), 2);
    }

    /// 验证 Hermite 零阶在互异点且无导数时等价于多项式插值：对四点
    /// 三次式精确重建。
    #[test]
    fn test_hermite_order_zero_matches_lagrange() {
        // 对于互异的点且无导数时，Hermite 零阶就是
        // Newton 形式的多项式插值：对 4 个点的三次式精确。
        let x_table = [-3.0, -1.0, 1.0, 2.0];
        let f = |x: f64| 2.0 * x * x * x - x * x + 4.0 * x - 7.0;
        let y_table: Vec<f64> = x_table.iter().map(|&x| f(x)).collect();
        for &x in &[-2.0, 0.0, 0.5, 1.5] {
            let result =
                HermitePolynomialApproximation.interpolate_order_zero(x, &x_table, &y_table, 1);
            assert!(
                (result[0] - f(x)).abs() < 1e-9,
                "x={x}: got {}, expected {}",
                result[0],
                f(x)
            );
        }
    }

    /// 验证 Hermite 零阶处理全相等样本：最高非零差商收缩为 0，结果
    /// 恒为该常量。
    #[test]
    fn test_hermite_order_zero_constant_data() {
        // 全相等的样本：highestNonZeroCoef 收缩为 0。
        let x_table = [0.0, 1.0, 2.0];
        let y_table = [5.0, 5.0, 5.0];
        let result =
            HermitePolynomialApproximation.interpolate_order_zero(0.7, &x_table, &y_table, 1);
        assert!((result[0] - 5.0).abs() < EPS);
    }

    /// 验证经典三次 Hermite：f(t)=t³ 于 [0,1] 连同端点导数采样，在
    /// t=0.5 同时插出值 0.125 与导数 0.75。
    #[test]
    fn test_hermite_with_derivatives_cubic() {
        // 经典的三次 Hermite：f(t) = t^3 于 [0, 1]。
        // f(0)=0，f'(0)=0，f(1)=1，f'(1)=3。
        // yTable 每点的布局：[value, derivative]。
        let x_table = [0.0, 1.0];
        let y_table = [0.0, 0.0, 1.0, 3.0];
        let result =
            HermitePolynomialApproximation.interpolate(0.5, &x_table, &y_table, 1, 1, 1);
        // result[0] = f(0.5) = 0.125，result[1] = f'(0.5) = 0.75。
        assert!(
            (result[0] - 0.125).abs() < 1e-9,
            "value: got {}",
            result[0]
        );
        assert!(
            (result[1] - 0.75).abs() < 1e-9,
            "derivative: got {}",
            result[1]
        );
    }

    /// 验证 Hermite 带导数插值在二次函数 f(x)=x² 上于各节点及中点
    /// 都精确重建值与导数。
    #[test]
    fn test_hermite_with_derivatives_at_nodes() {
        let x_table = [-1.0, 2.0];
        // f(x) = x^2：f(-1)=1，f'(-1)=-2，f(2)=4，f'(2)=4。
        let y_table = [1.0, -2.0, 4.0, 4.0];
        for &x in &[-1.0, 0.0, 2.0] {
            let result =
                HermitePolynomialApproximation.interpolate(x, &x_table, &y_table, 1, 1, 1);
            assert!((result[0] - x * x).abs() < 1e-9, "x={x}: got {}", result[0]);
            assert!((result[1] - 2.0 * x).abs() < 1e-9, "x={x}: got {}", result[1]);
        }
    }

    /// 验证 Hermite 三点带导数可精确到五次：f(x)=x⁵-x 连同导数采样，
    /// 六个 z-index 足以覆盖五次多项式。
    #[test]
    fn test_hermite_three_points_with_derivatives() {
        // f(x) = x^5 - x，在 -1、0、1 处连同导数采样。
        let x_table = [-1.0, 0.0, 1.0];
        let f = |x: f64| x.powi(5) - x;
        let df = |x: f64| 5.0 * x.powi(4) - 1.0;
        let mut y_table = Vec::new();
        for &x in &x_table {
            y_table.push(f(x));
            y_table.push(df(x));
        }
        // 3 个点 * 2 个值 = 6 个 z-index；次数至多 5 → 对 x^5 精确。
        let result =
            HermitePolynomialApproximation.interpolate(0.5, &x_table, &y_table, 1, 1, 1);
        assert!((result[0] - f(0.5)).abs() < 1e-9, "got {}", result[0]);
        assert!((result[1] - df(0.5)).abs() < 1e-9, "got {}", result[1]);
    }

    /// 验证算法枚举的分发与名字：三类算法名字、导数支持标志、名字解析
    /// 与默认值都与底层实现一致。
    #[test]
    fn test_kind_dispatch_and_names() {
        assert_eq!(InterpolationAlgorithmKind::Linear.name(), "Linear");
        assert_eq!(InterpolationAlgorithmKind::Lagrange.name(), "Lagrange");
        assert_eq!(InterpolationAlgorithmKind::Hermite.name(), "Hermite");
        assert!(!InterpolationAlgorithmKind::Linear.supports_derivatives());
        assert!(!InterpolationAlgorithmKind::Lagrange.supports_derivatives());
        assert!(InterpolationAlgorithmKind::Hermite.supports_derivatives());
        assert_eq!(
            InterpolationAlgorithmKind::from_name("Hermite"),
            Some(InterpolationAlgorithmKind::Hermite)
        );
        assert_eq!(InterpolationAlgorithmKind::from_name("Bogus"), None);
        assert_eq!(
            InterpolationAlgorithmKind::default(),
            InterpolationAlgorithmKind::Linear
        );
    }

    /// 验证非 Hermite 枚举的 interpolate 回退到零阶并把导数输出填零：
    /// 值正确、二槽为 0.0。
    #[test]
    fn test_kind_interpolate_fallback_zero_fills_derivatives() {
        // 非 Hermite 算法回退到零阶并填充零。
        let x_table = [0.0, 10.0];
        let y_table = [1.0, 21.0];
        let result = InterpolationAlgorithmKind::Linear.interpolate(5.0, &x_table, &y_table, 1, 0, 1);
        assert!((result[0] - 11.0).abs() < EPS);
        assert_eq!(result[1], 0.0);
    }
}
