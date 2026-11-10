//! Fabric 材质系统产生的错误。
//!
//! 定义 Fabric 材质系统在解析模板与组装材质过程中产生的错误类型，
//! 覆盖模板校验、uniform 创建、子材质创建与按类型查找等失败情形。

use thiserror::Error;

/// 解析 Fabric JSON 或组装材质时抛出的错误。
#[derive(Debug, Error, PartialEq)]
pub enum MaterialError {
    /// `fabric: cannot have source and components in the same template.`
    #[error("fabric: cannot have source and components in the same template.")]
    SourceAndComponents,

    /// `fabric: property name '<property>' is not valid. It should be ...`
    #[error(
        "fabric: property name '{property}' is not valid. It should be {expected}."
    )]
    InvalidPropertyName {
        /// 出问题的属性名。
        property: String,
        /// 有效属性名的逗号分隔列表。
        expected: String,
    },

    /// `fabric: uniforms and materials cannot share the same property '<name>'`
    #[error("fabric: uniforms and materials cannot share the same property '{name}'")]
    DuplicateUniformMaterialName {
        /// 共享的属性名。
        name: String,
    },

    /// `fabric: uniform '<uniform>' has invalid type.`
    #[error("fabric: uniform '{uniform}' has invalid type.")]
    InvalidUniformType {
        /// 其值无法确定类型的 uniform 名。
        uniform: String,
    },

    /// 无法从 JSON 解析 uniform 值。
    #[error("fabric: uniform '{uniform}' has an invalid value: {reason}")]
    InvalidUniformValue {
        /// uniform 名（或匿名值的占位符）。
        uniform: String,
        /// 人类可读的原因。
        reason: String,
    },

    /// `strict: shader source does not use uniform '<uniform>'.`
    #[error("strict: shader source does not use uniform '{uniform}'.")]
    StrictUnusedUniform {
        /// 未被使用的 uniform 名。
        uniform: String,
    },

    /// `strict: shader source does not use channels '<uniform>'.`
    #[error("strict: shader source does not use channels '{uniform}'.")]
    StrictUnusedChannels {
        /// 未被使用的 channels uniform 名。
        uniform: String,
    },

    /// `strict: shader source does not use material '<id>'.`
    #[error("strict: shader source does not use material '{id}'.")]
    StrictUnusedMaterial {
        /// 未被使用的子材质 id。
        id: String,
    },

    /// `material with type '<type>' does not exist.`
    #[error("material with type '{type_name}' does not exist.")]
    UnknownMaterialType {
        /// 所请求的材质类型。
        type_name: String,
    },

    /// 无效的 Fabric JSON 文档。
    #[error("invalid fabric JSON: {0}")]
    Json(String),
}

impl From<serde_json::Error> for MaterialError {
    /// 将底层的 JSON 解析错误包装为 [`MaterialError::Json`]。
    fn from(e: serde_json::Error) -> Self {
        MaterialError::Json(e.to_string())
    }
}
