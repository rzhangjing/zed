//! Fabric 材质的半透明规格。
//!
//! 半透明性可存储为布尔值，或作为材质当前 uniform 值的
//! 函数（例如 `material.uniforms.color.alpha < 1.0`）。本模块用一个
//! 单一的、数据驱动的 enum 捕获完整的内置函数形状，使领域层保持
//! 无闭包且可序列化。

use crate::uniform::UniformValue;
use std::collections::BTreeMap;

/// 材质如何判断自身是否半透明。
///
/// 每个材质缓存条目的 `translucent` 成员可取以下形态：
/// - `translucent: true`  -> [`TranslucentSpec::Always`]
/// - `translucent: false` -> [`TranslucentSpec::Never`]
/// - `translucent: function (material) { return <uniform>.alpha < 1.0 || ... }`
///   -> [`TranslucentSpec::AnyAlphaLt1`]，列出被检查的 uniform 名。
///
/// 对于 color（`vec4`）uniform，该函数读取 alpha 分量；对于
/// `float` uniform（如 Grid 的 `cellAlpha`）则读取标量本身。两种
/// 情况都由 [`UniformValue::alpha_or_scalar`] 处理。
#[derive(Debug, Clone, PartialEq)]
pub enum TranslucentSpec {
    /// 始终半透明（`translucent: true`）。
    Always,
    /// 从不半透明（`translucent: false`）。
    Never,
    /// 当任一命名 uniform 的 alpha/标量 `< 1.0` 时半透明。
    AnyAlphaLt1(Vec<&'static str>),
}

impl TranslucentSpec {
    /// 针对材质当前的 uniform 值求值该规格。
    ///
    /// 缺失或没有 alpha/标量分量的 uniform 贡献 `false`（它无法
    /// 使材质半透明），即只读取实际存在的 uniform。
    pub fn evaluate(&self, uniforms: &BTreeMap<String, UniformValue>) -> bool {
        match self {
            TranslucentSpec::Always => true,
            TranslucentSpec::Never => false,
            TranslucentSpec::AnyAlphaLt1(names) => names.iter().any(|name| {
                // 任一命名 uniform 的 alpha/标量 < 1.0 即判为半透明
                uniforms
                    .get(*name)
                    .and_then(UniformValue::alpha_or_scalar)
                    .map(|v| v < 1.0)
                    .unwrap_or(false)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniforms(pairs: &[(&str, UniformValue)]) -> BTreeMap<String, UniformValue> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_always_never() {
        let empty = uniforms(&[]);
        assert!(TranslucentSpec::Always.evaluate(&empty));
        assert!(!TranslucentSpec::Never.evaluate(&empty));
    }

    #[test]
    fn test_any_alpha_lt1_color() {
        let spec = TranslucentSpec::AnyAlphaLt1(vec!["color"]);
        assert!(spec.evaluate(&uniforms(&[(
            "color",
            UniformValue::Vec4([1.0, 0.0, 0.0, 0.5])
        )])));
        assert!(!spec.evaluate(&uniforms(&[(
            "color",
            UniformValue::Vec4([1.0, 0.0, 0.0, 1.0])
        )])));
    }

    #[test]
    fn test_any_alpha_lt1_scalar() {
        // Grid：cellAlpha 是一个 float uniform。
        let spec = TranslucentSpec::AnyAlphaLt1(vec!["color", "cellAlpha"]);
        assert!(spec.evaluate(&uniforms(&[
            ("color", UniformValue::Vec4([0.0, 1.0, 0.0, 1.0])),
            ("cellAlpha", UniformValue::Float(0.1)),
        ])));
        assert!(!spec.evaluate(&uniforms(&[
            ("color", UniformValue::Vec4([0.0, 1.0, 0.0, 1.0])),
            ("cellAlpha", UniformValue::Float(1.0)),
        ])));
    }

    #[test]
    fn test_any_alpha_lt1_multiple_names_or_semantics() {
        // Stripe：evenColor.alpha < 1.0 || oddColor.alpha < 1.0
        let spec = TranslucentSpec::AnyAlphaLt1(vec!["evenColor", "oddColor"]);
        assert!(spec.evaluate(&uniforms(&[
            ("evenColor", UniformValue::Vec4([1.0, 1.0, 1.0, 1.0])),
            ("oddColor", UniformValue::Vec4([0.0, 0.0, 1.0, 0.5])),
        ])));
        assert!(!spec.evaluate(&uniforms(&[
            ("evenColor", UniformValue::Vec4([1.0, 1.0, 1.0, 1.0])),
            ("oddColor", UniformValue::Vec4([0.0, 0.0, 1.0, 1.0])),
        ])));
    }

    #[test]
    fn test_missing_uniform_is_not_translucent() {
        let spec = TranslucentSpec::AnyAlphaLt1(vec!["color"]);
        assert!(!spec.evaluate(&uniforms(&[])));
    }
}
