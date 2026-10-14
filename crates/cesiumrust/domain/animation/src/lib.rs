//! cesium-animation：时间动态动画与插值。
//!
//! Domain 层 - 纯 Rust，f64 精度。
//!
//! 提供四类能力：多项式插值算法（Hermite/Lagrange/线性）、
//! 参数化样条曲线求值、时间轴播放控制，以及属性补间动画与缓动函数。
//! 各子模块相互独立，统一由本 crate 门面再导出公共类型。

pub mod interpolation;
pub mod spline;
pub mod timeline;
pub mod tween;

pub use interpolation::{
    catmull_rom, catmull_rom_vec3, hermite, hermite_vec3, interpolate, lagrange_interpolate,
    lagrange_interpolate_vec3, lerp, lerp_vec3, slerp_vec3, InterpolationType, SamplePoint,
};
pub use spline::{
    tridiagonal_solve, CatmullRomSpline, ConstantSpline, HermiteSpline, LinearSpline,
    MorphWeightSpline, QuaternionSpline, ScalarSpline, Spline, SteppedSpline,
};
pub use timeline::{
    AnimationController, SpeedPreset, TimelineConfig, TimelineState,
};
pub use tween::{EasingFunction, Tween, TweenCollection, TweenOptions};
