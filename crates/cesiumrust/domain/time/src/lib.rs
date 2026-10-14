//! cesium-time：JulianDate、Clock、TimeInterval
//! 领域层 - 纯 Rust，无框架依赖。
//!
//! 提供儒略日时间戳、格里高利历日期、时间区间及其集合，
//! 以及时钟推进与动画枚举等纯领域类型。

pub mod julian_date;
pub mod gregorian_date;
pub mod time_interval;
pub mod time_interval_collection;
pub mod clock;

pub use julian_date::JulianDate;
pub use julian_date::TimeStandard;
pub use gregorian_date::GregorianDate;
pub use gregorian_date::{is_leap_year, days_in_month};
pub use time_interval::TimeInterval;
pub use time_interval_collection::{TimeIntervalCollection, TimeIntervalData, FromIso8601Options};
pub use clock::{Clock, ClockOptions, ClockRange, ClockStep};
