//! Rectangle —— 由西、南、东、北定义的一个二维区域。
//!
//! 实现包含
//! `intersection`/`union`/`center`/
//! `contains`/`subsection` 中对反经线（IDL）穿越的处理逻辑，以及
//! `fromCartographicArray`/`fromCartesianArray` 的"最小外接矩形"逻辑。
//!
//! 矩形以弧度存储四个边界；因经度环绕，东经允许小于西经，表示该矩形
//! 跨过反经线（如太平洋区域）。所有涉及经度的集合运算（交/并/包含/中心）
//! 都需先处理这一环绕，方法是将相关经度同侧平移 2π 后再比较。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::bounding::BoundingSphere;
use crate::cartographic::Cartographic;
use crate::ellipsoid::{self, Ellipsoid};
use crate::math_utils::{self, EPSILON14, PI_OVER_TWO, TWO_PI};
use crate::transforms;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// 由经度/纬度边界（以弧度计）定义的二维区域。
///
/// 四个字段均为弧度制；当 `east < west` 时，矩形跨反经线（IDL）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rectangle {
    /// 最西侧经度，弧度制 [-PI, PI]。
    pub west: f64,
    /// 最南侧纬度，弧度制 [-PI/2, PI/2]。
    pub south: f64,
    /// 最东侧经度，弧度制 [-PI, PI]。
    pub east: f64,
    /// 最北侧纬度，弧度制 [-PI/2, PI/2]。
    pub north: f64,
}

impl Rectangle {
    /// 可能存在的最大矩形（覆盖全球经/纬度范围）。
    pub const MAX_VALUE: Self = Self {
        west: -PI,
        south: -PI_OVER_TWO,
        east: PI,
        north: PI_OVER_TWO,
    };

    /// 空的（全零）矩形，等价于默认构造。
    ///
    /// 四边均为 0，代表一个退化为原点的零面积矩形。
    pub const EMPTY: Self = Self {
        west: 0.0,
        south: 0.0,
        east: 0.0,
        north: 0.0,
    };

    /// 将该对象打包进数组时所使用的元素个数。
    ///
    /// 固定为 4，对应 west/south/east/north 四个边界值。
    pub const PACKED_LENGTH: usize = 4;

    /// 由弧度创建一个新 Rectangle。
    ///
    /// # 参数
    /// - `west`/`south`/`east`/`north`：四边边界（弧度）；经度允许东小于西以表示跨反经线。
    pub fn new(west: f64, south: f64, east: f64, north: f64) -> Self {
        Self {
            west,
            south,
            east,
            north,
        }
    }

    /// 给定以度为单位的边界经纬度创建矩形。
    ///
    /// # 参数
    /// - `west`/`south`/`east`/`north`：以度计的边界，内部统一转成弧度存储。
    pub fn from_degrees(west: f64, south: f64, east: f64, north: f64) -> Self {
        Self {
            west: math_utils::to_radians(west),
            south: math_utils::to_radians(south),
            east: math_utils::to_radians(east),
            north: math_utils::to_radians(north),
        }
    }

    /// 由弧度为单位的边界经纬度创建矩形（等价于 [`Rectangle::new`]）。
    pub fn from_radians(west: f64, south: f64, east: f64, north: f64) -> Self {
        Self::new(west, south, east, north)
    }

    /// 将给定的实例依次写入数组的连续四个位置。
    ///
    /// # 参数
    /// - `array`：目标可写数组。
    /// - `starting_index`：写入起始下标，依次存放 west/south/east/north。
    pub fn pack_into(&self, array: &mut [f64], starting_index: usize) {
        array[starting_index] = self.west;
        array[starting_index + 1] = self.south;
        array[starting_index + 2] = self.east;
        array[starting_index + 3] = self.north;
    }

    /// 将该矩形打包进一个新的 `[f64; 4]`（`[west, south, east, north]`）。
    ///
    /// 与 [`Rectangle::pack_into`] 采用相同的字段顺序，便于序列化/反序列化对齐。
    pub fn pack(&self) -> [f64; 4] {
        [self.west, self.south, self.east, self.north]
    }

    /// 从打包数组的连续四个位置读回一个矩形实例。
    ///
    /// # 参数
    /// - `array`：源数组。
    /// - `starting_index`：读取起始下标，依次为 west/south/east/north。
    pub fn unpack(array: &[f64], starting_index: usize) -> Self {
        Self {
            west: array[starting_index],
            south: array[starting_index + 1],
            east: array[starting_index + 2],
            north: array[starting_index + 3],
        }
    }

    /// 以弧度计算矩形的宽度。
    ///
    /// 若东经小于西经（跨反经线），将东经加 2π 后再相减，得到正向跨幅。
    pub fn width(&self) -> f64 {
        let mut east = self.east;
        let west = self.west;
        if east < west {
            east += TWO_PI;
        }
        east - west
    }

    /// 计算矩形的高度（纬度跨幅，弧度制）。
    pub fn height(&self) -> f64 {
        self.north - self.south
    }

    /// 创建能包围给定数组中所有位置的最小可能 Rectangle。
    ///
    /// 同时统计“常规区间”与“跨反经线（IDL）区间”两套 west/east，
    /// 最后取跨幅更小的一套，以保证经度环绕时得到真正最小外接矩形。
    pub fn from_cartographic_array(cartographics: &[Cartographic]) -> Self {
        let mut west = f64::MAX;
        let mut east = f64::MIN;
        let mut west_over_idl = f64::MAX;
        let mut east_over_idl = f64::MIN;
        let mut south = f64::MAX;
        let mut north = f64::MIN;

        for position in cartographics {
            // 常规区间：直接累取经/纬度的 min/max。
            west = west.min(position.longitude);
            east = east.max(position.longitude);
            south = south.min(position.latitude);
            north = north.max(position.latitude);

            // 跨 IDL 区间：把负经度折算到 [0, 2π) 再取 min/max。
            let lon_adjusted = if position.longitude >= 0.0 {
                position.longitude
            } else {
                position.longitude + TWO_PI
            };
            west_over_idl = west_over_idl.min(lon_adjusted);
            east_over_idl = east_over_idl.max(lon_adjusted);
        }

        // 两套区间取跨幅更小者；若选跨 IDL 方案则需把超过 π 的经度折回。
        if east - west > east_over_idl - west_over_idl {
            // 跨 IDL 方案跨幅更小，改用它并折回超过 π 的经度。
            west = west_over_idl;
            east = east_over_idl;

            if east > PI {
                east -= TWO_PI;
            }
            if west > PI {
                west -= TWO_PI;
            }
        }

        Self {
            west,
            south,
            east,
            north,
        }
    }

    /// 创建能包围给定的笛卡尔位置数组中所有位置的最小可能 Rectangle。
    ///
    /// 先将每个笛卡尔点投影为测绘坐标，再按与 [`Rectangle::from_cartographic_array`]
    /// 相同的双区间策略求最小外接矩形。
    pub fn from_cartesian_array(cartesians: &[DVec3], ellipsoid: &Ellipsoid) -> Self {
        let mut west = f64::MAX;
        let mut east = f64::MIN;
        let mut west_over_idl = f64::MAX;
        let mut east_over_idl = f64::MIN;
        let mut south = f64::MAX;
        let mut north = f64::MIN;

        for cartesian in cartesians {
            // 先把笛卡尔点投影回椭球面的经纬度，再做与经纬度版相同的统计。
            let position = ellipsoid
                .cartesian_to_cartographic(*cartesian)
                .expect("cartesian must not be at the center of the ellipsoid");
            west = west.min(position.longitude);
            east = east.max(position.longitude);
            south = south.min(position.latitude);
            north = north.max(position.latitude);

            let lon_adjusted = if position.longitude >= 0.0 {
                position.longitude
            } else {
                position.longitude + TWO_PI
            };
            west_over_idl = west_over_idl.min(lon_adjusted);
            east_over_idl = east_over_idl.max(lon_adjusted);
        }

        if east - west > east_over_idl - west_over_idl {
            west = west_over_idl;
            east = east_over_idl;

            if east > PI {
                east -= TWO_PI;
            }
            if west > PI {
                west -= TWO_PI;
            }
        }

        Self {
            west,
            south,
            east,
            north,
        }
    }

    /// 由包围球创建一个矩形，忽略高度。
    ///
    /// 以球心处的东/北单位方向乘以半径得到四个极值点，连同球心一起交给
    /// [`Rectangle::from_cartesian_array`] 求外接矩形；球心为原点时返回最大矩形。
    /// 以地心单位向量与旋转矩阵推导四极值点，再投影为经纬度求外接。
    ///
    /// north/east 先归一化再乘以半径，得到沿当地北/东方向偏移的四个候选点。
    pub fn from_bounding_sphere(bounding_sphere: &BoundingSphere, ellipsoid: &Ellipsoid) -> Self {
        let center = bounding_sphere.center;
        let radius = bounding_sphere.radius;

        if center == DVec3::ZERO {
            return Self::MAX_VALUE;
        }

        let from_enu = transforms::east_north_up_to_fixed_frame(center, ellipsoid);
        // Matrix4.multiplyByPointAsVector：仅应用线性（旋转）部分。
        let east = ellipsoid::normalize_cartesian3(from_enu.transform_vector3(DVec3::X));
        let north = ellipsoid::normalize_cartesian3(from_enu.transform_vector3(DVec3::Y));

        let north = north * radius;
        let east = east * radius;
        let south = -north;
        let west = -east;

        let positions = [
            center + north,
            center + west,
            center + south,
            center + east,
            center,
        ];
        Self::from_cartesian_array(&positions, ellipsoid)
    }

    /// 检查矩形的各属性，若它们不在有效范围内则返回错误。
    ///
    /// 纬度需落在 [-π/2, π/2]，经度需落在 [-π, π]；任一越界即返回 `Err`。
    pub fn validate(&self) -> Result<(), String> {
        let north = self.north;
        if !(north >= -PI_OVER_TWO) || !(north <= PI_OVER_TWO) {
            // 采用双重否定比较，以便同时拦截 NaN（NaN 使两个比较均为假）。
            return Err("north must be in the interval [-Pi/2, Pi/2].".to_string());
        }

        let south = self.south;
        if !(south >= -PI_OVER_TWO) || !(south <= PI_OVER_TWO) {
            return Err("south must be in the interval [-Pi/2, Pi/2].".to_string());
        }

        let west = self.west;
        if !(west >= -PI) || !(west <= PI) {
            return Err("west must be in the interval [-Pi, Pi].".to_string());
        }

        let east = self.east;
        if !(east >= -PI) || !(east <= PI) {
            return Err("east must be in the interval [-Pi, Pi].".to_string());
        }

        Ok(())
    }

    /// 计算矩形的西南角（作为高度为 0 的测绘坐标）。
    ///
    /// 经纬度分别取自 west 与 south。
    pub fn southwest(&self) -> Cartographic {
        Cartographic::from_radians(self.west, self.south, 0.0)
    }

    /// 计算矩形的西北角（作为高度为 0 的测绘坐标）。
    ///
    /// 经纬度分别取自 west 与 north。
    pub fn northwest(&self) -> Cartographic {
        Cartographic::from_radians(self.west, self.north, 0.0)
    }

    /// 计算矩形的东北角（作为高度为 0 的测绘坐标）。
    ///
    /// 经纬度分别取自 east 与 north。
    pub fn northeast(&self) -> Cartographic {
        Cartographic::from_radians(self.east, self.north, 0.0)
    }

    /// 计算矩形的东南角（作为高度为 0 的测绘坐标）。
    ///
    /// 经纬度分别取自 east 与 south。
    pub fn southeast(&self) -> Cartographic {
        Cartographic::from_radians(self.east, self.south, 0.0)
    }

    /// 计算矩形的中心。
    ///
    /// 先将东经归到西经同侧（跨反经线时加 2π）取中点，再用
    /// `negative_pi_to_pi` 将经度折回 [-π, π]；纬度取南、北均值。
    pub fn center(&self) -> Cartographic {
        let mut east = self.east;
        let west = self.west;

        if east < west {
            east += TWO_PI;
        }

        let longitude = math_utils::negative_pi_to_pi((west + east) * 0.5);
        let latitude = (self.south + self.north) * 0.5;

        Cartographic::from_radians(longitude, latitude, 0.0)
    }

    /// 计算两个矩形的交集，考虑经度在反经线处的环绕。
    ///
    /// 先把两侧矩形调整到同一经度基准（按需加 2π 处理跨 IDL），再取 west
    /// 的最大值与 east 的最小值；若经度或纬度区间不相交则返回 None。
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let mut rectangle_east = self.east;
        let mut rectangle_west = self.west;

        let mut other_rectangle_east = other.east;
        let mut other_rectangle_west = other.west;

        if rectangle_east < rectangle_west && other_rectangle_east > 0.0 {
            rectangle_east += TWO_PI;
        } else if other_rectangle_east < other_rectangle_west && rectangle_east > 0.0 {
            other_rectangle_east += TWO_PI;
        }

        if rectangle_east < rectangle_west && other_rectangle_west < 0.0 {
            // 本矩形跨 IDL 且对方偏西：将对方西经提升 2π。
            other_rectangle_west += TWO_PI;
        } else if other_rectangle_east < other_rectangle_west && rectangle_west < 0.0 {
            rectangle_west += TWO_PI;
        }

        // 先求经度交集并折回 [-π, π]，再判是否空交。
        let west = math_utils::negative_pi_to_pi(rectangle_west.max(other_rectangle_west));
        let east = math_utils::negative_pi_to_pi(rectangle_east.min(other_rectangle_east));

        if (self.west < self.east || other.west < other.east) && east <= west {
            return None;
        }

        // 纬度区间交集：取较大的 south 与较小的 north。
        let south = self.south.max(other.south);
        let north = self.north.min(other.north);

        if south >= north {
            return None;
        }

        Some(Self {
            west,
            south,
            east,
            north,
        })
    }

    /// 计算两个矩形的简单交集，忽略反经线
    /// （可用于投影坐标）。
    ///
    /// 直接对四边取 max/min，不做任何经度环绕修正，因而适用于已投影的平面坐标。
    /// 若经度或纬度区间完全错开（west≥east 或 south≥north）则返回 None。
    pub fn simple_intersection(&self, other: &Self) -> Option<Self> {
        let west = self.west.max(other.west);
        let south = self.south.max(other.south);
        let east = self.east.min(other.east);
        let north = self.north.min(other.north);

        if south >= north || west >= east {
            return None;
        }

        Some(Self {
            west,
            south,
            east,
            north,
        })
    }

    /// 计算作为两个矩形并集的矩形，考虑经度在反经线处的环绕。
    ///
    /// 与交集类似地先对齐经度基准，再取 west 最小、east 最大、south 最小、north 最大。
    pub fn union(&self, other: &Self) -> Self {
        let mut rectangle_east = self.east;
        let mut rectangle_west = self.west;

        let mut other_rectangle_east = other.east;
        let mut other_rectangle_west = other.west;

        if rectangle_east < rectangle_west && other_rectangle_east > 0.0 {
            rectangle_east += TWO_PI;
        } else if other_rectangle_east < other_rectangle_west && rectangle_east > 0.0 {
            other_rectangle_east += TWO_PI;
        }

        if rectangle_east < rectangle_west && other_rectangle_west < 0.0 {
            other_rectangle_west += TWO_PI;
        } else if other_rectangle_east < other_rectangle_west && rectangle_west < 0.0 {
            rectangle_west += TWO_PI;
        }

        let west = math_utils::negative_pi_to_pi(rectangle_west.min(other_rectangle_west));
        let east = math_utils::negative_pi_to_pi(rectangle_east.max(other_rectangle_east));

        // 纬度并集：取更低的 south 与更高的 north。
        Self {
            west,
            south: self.south.min(other.south),
            east,
            north: self.north.max(other.north),
        }
    }

    /// 通过不断放大本矩形直到其包含给定的测绘坐标，从而计算出一个矩形。
    ///
    /// 仅按经度/纬度取 min/max 扩张；给定点的高度被忽略。
    ///
    /// 返回新矩形，原矩形不变；若点已在范围内则边界不变。
    pub fn expand(&self, cartographic: &Cartographic) -> Self {
        Self {
            west: self.west.min(cartographic.longitude),
            south: self.south.min(cartographic.latitude),
            east: self.east.max(cartographic.longitude),
            north: self.north.max(cartographic.latitude),
        }
    }

    /// 若测绘位置（经度/纬度，弧度制）位于矩形上或其内部则返回 true，
    /// 否则返回 false。
    ///
    /// 跨反经线（east<west）时将 east 与待测经度同侧平移 2π 后再比较；
    /// 经度边界使用 epsilon 容差以包含落在边上的点。
    pub fn contains(&self, longitude: f64, latitude: f64) -> bool {
        let mut longitude = longitude;

        let west = self.west;
        let mut east = self.east;

        // 跨反经线时本矩形实际横跨 ±π，需将 west 折回、east 提升以保持次序。
        if east < west {
            east += TWO_PI;
            if longitude < 0.0 {
                longitude += TWO_PI;
            }
        }
        (longitude > west
            || math_utils::equals_epsilon(longitude, west, EPSILON14, EPSILON14))
            && (longitude < east
                || math_utils::equals_epsilon(longitude, east, EPSILON14, EPSILON14))
            && latitude >= self.south
            && latitude <= self.north
    }

    /// 对矩形进行采样，使其包含一组适合传给
    /// `BoundingSphere.fromPoints` 的笛卡尔点。采样对于覆盖极点或
    /// 跨越赤道的矩形而言是必要的。
    ///
    /// 先取四个角点，再根据矩形靠近北极/南极/跨赤道选一条中间纬线沿经度采样，
    /// 以保证包围球能正确包围球面上的实际弯曲区域。
    pub fn subsample(&self, ellipsoid: &Ellipsoid, surface_height: f64) -> Vec<DVec3> {
        let mut result = Vec::new();

        let north = self.north;
        let south = self.south;
        let east = self.east;
        let west = self.west;

        // 依次取四个角，按绕边界一圈的顺序写入结果。
        let mut lla = Cartographic::from_radians(west, north, surface_height);
        result.push(ellipsoid.cartographic_to_cartesian(&lla));

        lla.longitude = east;
        result.push(ellipsoid.cartographic_to_cartesian(&lla));

        lla.latitude = south;
        result.push(ellipsoid.cartographic_to_cartesian(&lla));

        lla.longitude = west;
        result.push(ellipsoid.cartographic_to_cartesian(&lla));

        // 选一条靠近极点或赤道的中间纬线，沿经度分 7 段采样落入矩形内的点。
        if north < 0.0 {
            lla.latitude = north;
        } else if south > 0.0 {
            lla.latitude = south;
        } else {
            lla.latitude = 0.0;
        }

        for i in 1..8 {
            // 沿选定的中间纬线把经度从 -π 逐步推进 7 段，仅保留落在矩形内的点。
            lla.longitude = -PI + i as f64 * math_utils::PI_OVER_TWO;
            if self.contains(lla.longitude, lla.latitude) {
                result.push(ellipsoid.cartographic_to_cartesian(&lla));
            }
        }

        // 若采样纬线为赤道，额外补上西、东两端点，避免包围球低估经度跨幅。
        if lla.latitude == 0.0 {
            lla.longitude = west;
            result.push(ellipsoid.cartographic_to_cartesian(&lla));
            lla.longitude = east;
            result.push(ellipsoid.cartographic_to_cartesian(&lla));
        }
        result
    }

    /// 由 [0.0, 1.0] 范围内的归一化坐标计算矩形的一个子区域。
    ///
    /// 四个 lerp 参数分别确定子矩形四边占全矩形的比例；任一越界或次序颠倒均返回 `Err`。
    pub fn subsection(
        &self,
        west_lerp: f64,
        south_lerp: f64,
        east_lerp: f64,
        north_lerp: f64,
    ) -> Result<Self, String> {
        if !(west_lerp >= 0.0) || !(west_lerp <= 1.0) {
            return Err("westLerp must be in the range [0.0, 1.0].".to_string());
        }
        if !(south_lerp >= 0.0) || !(south_lerp <= 1.0) {
            return Err("southLerp must be in the range [0.0, 1.0].".to_string());
        }
        if !(east_lerp >= 0.0) || !(east_lerp <= 1.0) {
            return Err("eastLerp must be in the range [0.0, 1.0].".to_string());
        }
        if !(north_lerp >= 0.0) || !(north_lerp <= 1.0) {
            return Err("northLerp must be in the range [0.0, 1.0].".to_string());
        }
        if !(west_lerp <= east_lerp) {
            return Err("westLerp must be less than or equal to eastLerp.".to_string());
        }
        if !(south_lerp <= north_lerp) {
            return Err("southLerp must be less than or equal to northLerp.".to_string());
        }

        // 不使用 lerp：当起止值相同而 t 变化时 lerp 会引入浮点误差。
        // 跨反经线时宽度需加 2π 修正，再按比例展开并折回 [-π, π]。
        // 本函数不使用 lerp，因为当起始值和结束值相同但 t 变化时，
        // lerp 会有浮点精度问题。
        let (mut west, mut east) = if self.west <= self.east {
            let width = self.east - self.west;
            (self.west + west_lerp * width, self.west + east_lerp * width)
        } else {
            let width = TWO_PI + self.east - self.west;
            (
                math_utils::negative_pi_to_pi(self.west + west_lerp * width),
                math_utils::negative_pi_to_pi(self.west + east_lerp * width),
            )
        };
        let height = self.north - self.south;
        let mut south = self.south + south_lerp * height;
        let mut north = self.south + north_lerp * height;

        // 修复 t = 1 时的浮点精度问题：直接把对应边钉到全矩形的实际边界。
        if west_lerp == 1.0 {
            west = self.east;
        }
        if east_lerp == 1.0 {
            east = self.east;
        }
        if south_lerp == 1.0 {
            south = self.north;
        }
        if north_lerp == 1.0 {
            north = self.north;
        }

        Ok(Self {
            west,
            south,
            east,
            north,
        })
    }

    /// 将该矩形细分为一个小矩形网格。
    ///
    /// # 参数
    /// - `x_segments`/`y_segments`：横向与纵向的分段数，共生成二者乘个小矩形。
    pub fn subdivide(&self, x_segments: u32, y_segments: u32) -> Vec<Self> {
        let mut result = Vec::with_capacity((x_segments * y_segments) as usize);
        // 按分段数将宽/高均分为 dx/dy，逐行逐列生成对齐的小矩形。
        let width = self.width();
        let height = self.height();
        let dx = width / x_segments as f64;
        let dy = height / y_segments as f64;

        for j in 0..y_segments {
            for i in 0..x_segments {
                let west = self.west + dx * i as f64;
                let south = self.south + dy * j as f64;
                result.push(Self {
                    west,
                    south,
                    east: west + dx,
                    north: south + dy,
                });
            }
        }
        result
    }

    /// 判断本矩形是否在 epsilon 容差内等于另一个矩形。
    ///
    /// 四个边界各自差的绝对值均不超过 epsilon 时视为相等。
    pub fn equals_epsilon(&self, other: &Self, epsilon: f64) -> bool {
        (self.west - other.west).abs() <= epsilon
            && (self.south - other.south).abs() <= epsilon
            && (self.east - other.east).abs() <= epsilon
            && (self.north - other.north).abs() <= epsilon
    }
}

impl Default for Rectangle {
    /// 默认矩形为空矩形（全零），与 [`Rectangle::EMPTY`] 一致。
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 验证以度创建的全球矩形等于最大矩形。
    fn test_from_degrees() {
        let r = Rectangle::from_degrees(-180.0, -90.0, 180.0, 90.0);
        assert!(r.equals_epsilon(&Rectangle::MAX_VALUE, 1e-10));
    }

    #[test]
    /// 验证矩形宽度与高度按弧度计算正确。
    fn test_width_height() {
        let r = Rectangle::from_degrees(-90.0, -45.0, 90.0, 45.0);
        assert!((r.width() - PI).abs() < 1e-10);
        assert!((r.height() - PI / 2.0).abs() < 1e-10);
    }

    #[test]
    /// 验证内部点被包含、外部点不被包含。
    fn test_contains() {
        let r = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
        assert!(r.contains(0.0, 0.0));
        assert!(!r.contains(math_utils::to_radians(20.0), 0.0));
    }

    #[test]
    /// 验证两矩形交集的西南角为重叠区域的角点。
    fn test_intersection() {
        let a = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
        let b = Rectangle::from_degrees(0.0, 0.0, 20.0, 20.0);
        let inter = a.intersection(&b).unwrap();
        assert!((inter.west - 0.0).abs() < 1e-10);
        assert!((inter.south - 0.0).abs() < 1e-10);
    }

    #[test]
    /// 验证两矩形并集覆盖两侧的全部经度范围。
    fn test_union() {
        let a = Rectangle::from_degrees(-10.0, -10.0, 10.0, 10.0);
        let b = Rectangle::from_degrees(0.0, 0.0, 20.0, 20.0);
        let u = a.union(&b);
        assert!((u.west - math_utils::to_radians(-10.0)).abs() < 1e-10);
        assert!((u.east - math_utils::to_radians(20.0)).abs() < 1e-10);
    }

    #[test]
    /// 验证对称于原点的矩形其中心落在经/纬 0 处。
    fn test_center() {
        let r = Rectangle::from_degrees(-90.0, -45.0, 90.0, 45.0);
        let c = r.center();
        assert!(c.longitude.abs() < 1e-10);
        assert!(c.latitude.abs() < 1e-10);
    }
}
