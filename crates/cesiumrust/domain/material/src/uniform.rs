//! Fabric uniform 值与 GLSL 类型推断。
//!
//! 本模块负责 uniform 的取值解析、根据值形状推断
//! GLSL uniform 类型，以及 WebGL uniform 设置器所执行的
//! uniform 值强制转换。

use crate::error::MaterialError;
use serde_json::Value as JsonValue;

/// 默认纹理 uniform 值。
/// 未指定图像时使用的默认纹理标识。
pub const DEFAULT_IMAGE_ID: &str = "czm_defaultImage";

/// 默认立方体贴图纹理 uniform 值。
/// 未指定立方贴图时使用的默认纹理标识。
pub const DEFAULT_CUBEMAP_ID: &str = "czm_defaultCubeMap";

/// cube map uniform 的六个面图像。
/// 表示 GLSL `samplerCube` uniform 所接受的六面图像对象。
#[derive(Debug, Clone, PartialEq)]
pub struct CubeMapFaces {
    /// +X 面的图像标识。
    pub positive_x: String,
    /// -X 面的图像标识。
    pub negative_x: String,
    /// +Y 面的图像标识。
    pub positive_y: String,
    /// -Y 面的图像标识。
    pub negative_y: String,
    /// +Z 面的图像标识。
    pub positive_z: String,
    /// -Z 面的图像标识。
    pub negative_z: String,
}

/// Fabric 材质的 uniform 值。
///
/// uniform 值以原始形式存储（数字、布尔、颜色、
/// 二维/三维/四维向量、图像 URL、通道字符串、以数组表示的矩阵、
/// 立方体贴图面对象），并根据值的形状推断 GLSL uniform 类型。
/// 本 enum 捕获同一组形状，并将推断出的类型显式化。
#[derive(Debug, Clone, PartialEq)]
pub enum UniformValue {
    /// GLSL `float`（JS number）。
    Float(f64),
    /// GLSL `bool`（JS boolean）。
    Bool(bool),
    /// GLSL `vec2`（`Cartesian2`、`{x, y}`，或像 `fadeDirection: {x: true, y: true}`
    /// 这样的双通道布尔向量）。
    Vec2([f64; 2]),
    /// GLSL `vec3`（`Cartesian3` / `{x, y, z}`）。
    Vec3([f64; 3]),
    /// GLSL `vec4`（`Color` / `Cartesian4` / `{x, y, z, w}`）。
    Vec4([f64; 4]),
    /// GLSL `ivec3`（用于自动生成的 `<image>Dimensions` uniform）。
    IVec3([i64; 3]),
    /// GLSL `mat2`（4 个数字的列主序数组）。
    Mat2([f64; 4]),
    /// GLSL `mat3`（9 个数字的列主序数组）。
    Mat3([f64; 9]),
    /// GLSL `mat4`（16 个数字的列主序数组）。
    Mat4([f64; 16]),
    /// GLSL `sampler2D`（图像 URL，或 [`DEFAULT_IMAGE_ID`]）。
    Sampler2D(String),
    /// GLSL `samplerCube`。`None` 表示 [`DEFAULT_CUBEMAP_ID`]。
    SamplerCube(Option<CubeMapFaces>),
    /// 像 `"rgb"` 或 `"a"` 这样的通道 swizzle 字符串。它
    /// 并非真正的 uniform：该 token 会在着色器源码中被
    /// 文本替换（推断为 `channels` 类型）。
    Channels(String),
}

impl UniformValue {
    /// 该值的 GLSL uniform 类型名。
    /// 根据值的形状返回对应的类型名。
    pub fn glsl_type(&self) -> &'static str {
        match self {
            UniformValue::Float(_) => "float",
            UniformValue::Bool(_) => "bool",
            UniformValue::Vec2(_) => "vec2",
            UniformValue::Vec3(_) => "vec3",
            UniformValue::Vec4(_) => "vec4",
            UniformValue::IVec3(_) => "ivec3",
            UniformValue::Mat2(_) => "mat2",
            UniformValue::Mat3(_) => "mat3",
            UniformValue::Mat4(_) => "mat4",
            UniformValue::Sampler2D(_) => "sampler2D",
            UniformValue::SamplerCube(_) => "samplerCube",
            UniformValue::Channels(_) => "channels",
        }
    }

    /// 对于类颜色值返回 alpha 分量，对于 float 返回标量。
    /// 由半透明性求值使用（内置半透明函数中的
    /// `material.uniforms.color.alpha < 1.0` 和 `uniforms.cellAlpha < 1.0`）。
    pub fn alpha_or_scalar(&self) -> Option<f64> {
        // 颜色（vec4）取 alpha 分量，float 取自身，其余无 alpha
        match self {
            UniformValue::Float(f) => Some(*f),
            UniformValue::Vec4(v) => Some(v[3]),
            _ => None,
        }
    }
}

/// 字符串是否为通道 swizzle（`"r"`、`"rgb"`、`"rgba"` 等）。
/// 遵循 `^[rgba]{1,4}$`（忽略大小写）的形状判定。
pub fn is_channel_string(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 4
        && s.chars()
            .all(|c| matches!(c.to_ascii_lowercase(), 'r' | 'g' | 'b' | 'a'))
}

/// 将 JSON 数字或布尔强制转换为 f64（布尔→1.0/0.0）。
fn json_to_f64(v: &JsonValue) -> Option<f64> {
    // 数字直接取 f64；布尔强制为 1.0/0.0；其余不可转换
    match v {
        JsonValue::Number(n) => n.as_f64(),
        JsonValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// 从 JSON 对象中按键取出一个可强制为 f64 的数值分量。
fn take_number(map: &serde_json::Map<String, JsonValue>, key: &str) -> Option<f64> {
    // 按键查找后复用 json_to_f64 的强制转换
    map.get(key).and_then(json_to_f64)
}

/// 将 Fabric JSON uniform 值解析为 [`UniformValue`]。
///
/// 基于值形状的推断规则：
/// - number → `float`
/// - boolean → `bool`
/// - string → 当匹配 `^[rgba]{1,4}$` 时为 `channels`，当为
///   [`DEFAULT_CUBEMAP_ID`] 时为 `samplerCube`，否则为 `sampler2D`
/// - 4/9/16 个数字的数组 → `mat2`/`mat3`/`mat4`
/// - 含 `red/green/blue/alpha` 的对象 → `vec4`（一个 `Color`）
/// - 含六个立方体贴图面键的对象 → `samplerCube`
/// - 含 2/3/4 个属性的对象 → `vec2`/`vec3`/`vec4`（布尔强制转换为
///   1.0/0.0，与 WebGL uniform 设置器一致）
/// - 显式的 `"type"` 成员会覆盖推断出的类型名
pub fn uniform_value_from_json(value: &JsonValue) -> Result<UniformValue, MaterialError> {
    match value {
        // 数字→float，非有限数报错
        JsonValue::Number(n) => Ok(UniformValue::Float(
            n.as_f64().ok_or(MaterialError::InvalidUniformValue {
                uniform: "<number>".to_string(),
                reason: "not a finite number".to_string(),
            })?,
        )),
        // 布尔→bool
        JsonValue::Bool(b) => Ok(UniformValue::Bool(*b)),
        // 字符串：通道 swizzle / 默认立方贴图 / 普通图像 URL
        JsonValue::String(s) => {
            if is_channel_string(s) {
                Ok(UniformValue::Channels(s.clone()))
            } else if s == DEFAULT_CUBEMAP_ID {
                Ok(UniformValue::SamplerCube(None))
            } else {
                Ok(UniformValue::Sampler2D(s.clone()))
            }
        }
        // 数组：逐元素强制为数字，按长度（4/9/16）选择矩阵类型
        JsonValue::Array(arr) => {
            let nums: Option<Vec<f64>> = arr.iter().map(json_to_f64).collect();
            let nums = nums.ok_or(MaterialError::InvalidUniformValue {
                uniform: "<array>".to_string(),
                reason: "array elements must be numbers".to_string(),
            })?;
            match nums.len() {
                // 4/9/16 个元素分别对应 mat2/mat3/mat4，其余非法
                4 => Ok(UniformValue::Mat2([nums[0], nums[1], nums[2], nums[3]])),
                9 => {
                    let mut m = [0.0; 9];
                    m.copy_from_slice(&nums);
                    Ok(UniformValue::Mat3(m))
                }
                16 => {
                    let mut m = [0.0; 16];
                    m.copy_from_slice(&nums);
                    Ok(UniformValue::Mat4(m))
                }
                _ => Err(MaterialError::InvalidUniformValue {
                    uniform: "<array>".to_string(),
                    reason: format!(
                        "matrix arrays must have 4, 9 or 16 elements, got {}",
                        nums.len()
                    ),
                }),
            }
        }
        JsonValue::Object(map) => {
            // 显式类型标注（例如自动生成的
            // `{ type: "ivec3", x: 1, y: 1 }` 尺寸 uniform）。
            if let Some(JsonValue::String(type_name)) = map.get("type") {
                return uniform_value_with_explicit_type(type_name, map);
            }

            // Color：{ red, green, blue, alpha }
            if map.contains_key("red")
                && map.contains_key("green")
                && map.contains_key("blue")
                && map.contains_key("alpha")
            {
                return Ok(UniformValue::Vec4([
                    // red/green/blue 缺失时默认 0.0，alpha 缺失时默认 1.0（不透明）
                    take_number(map, "red").unwrap_or(0.0),
                    take_number(map, "green").unwrap_or(0.0),
                    take_number(map, "blue").unwrap_or(0.0),
                    take_number(map, "alpha").unwrap_or(1.0),
                ]));
            }

            // 立方体贴图面：{ positiveX, negativeX, ..., negativeZ }
            if map.contains_key("positiveX")
                && map.contains_key("negativeX")
                && map.contains_key("positiveY")
                && map.contains_key("negativeY")
                && map.contains_key("positiveZ")
                && map.contains_key("negativeZ")
            {
                // 逐面取图像名，缺失或非字符串时取空串
                let face = |key: &str| -> String {
                    map.get(key)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string()
                };
                return Ok(UniformValue::SamplerCube(Some(CubeMapFaces {
                    positive_x: face("positiveX"),
                    negative_x: face("negativeX"),
                    positive_y: face("positiveY"),
                    negative_y: face("negativeY"),
                    positive_z: face("positiveZ"),
                    negative_z: face("negativeZ"),
                })));
            }

            // 基于属性数量的向量推断（2..=4 个属性）。
            // 分量名按 x/y/z/w 依次取值，缺失则补 0.0
            let num_attributes = map.len();
            let component = |key: &str| -> Option<f64> { take_number(map, key) };
            match num_attributes {
                2 => Ok(UniformValue::Vec2([
                    component("x").unwrap_or(0.0),
                    component("y").unwrap_or(0.0),
                ])),
                3 => Ok(UniformValue::Vec3([
                    component("x").unwrap_or(0.0),
                    component("y").unwrap_or(0.0),
                    component("z").unwrap_or(0.0),
                ])),
                4 => Ok(UniformValue::Vec4([
                    component("x").unwrap_or(0.0),
                    component("y").unwrap_or(0.0),
                    component("z").unwrap_or(0.0),
                    component("w").unwrap_or(0.0),
                ])),
                _ => Err(MaterialError::InvalidUniformValue {
                    uniform: "<object>".to_string(),
                    reason: format!(
                        "cannot infer uniform type from object with {} attributes",
                        num_attributes
                    ),
                }),
            }
        }
        JsonValue::Null => Err(MaterialError::InvalidUniformValue {
            uniform: "<null>".to_string(),
            reason: "null is not a valid uniform value".to_string(),
        }),
    }
}

/// 根据显式 `"type"` 标注从对象字段构造 [`UniformValue`]。
/// 用于含 `type` 成员的结构（如自动生成的尺寸 uniform）。
fn uniform_value_with_explicit_type(
    type_name: &str,
    map: &serde_json::Map<String, JsonValue>,
) -> Result<UniformValue, MaterialError> {
    let num = |key: &str| take_number(map, key).unwrap_or(0.0);
    match type_name {
        // 标量类从 value 字段取值，向量类从 x/y/z/w 分量取值
        "float" => Ok(UniformValue::Float(num("value"))),
        "bool" => Ok(UniformValue::Bool(
            map.get("value").and_then(|v| v.as_bool()).unwrap_or(false),
        )),
        "vec2" => Ok(UniformValue::Vec2([num("x"), num("y")])),
        "vec3" => Ok(UniformValue::Vec3([num("x"), num("y"), num("z")])),
        "vec4" => Ok(UniformValue::Vec4([num("x"), num("y"), num("z"), num("w")])),
        "ivec3" => Ok(UniformValue::IVec3([
            // 向量分量截断取整为整数
            num("x") as i64,
            num("y") as i64,
            num("z") as i64,
        ])),
        "sampler2D" => Ok(UniformValue::Sampler2D(
            // 缺失 value 时回退到默认图像标识
            map.get("value")
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_IMAGE_ID)
                .to_string(),
        )),
        "samplerCube" => Ok(UniformValue::SamplerCube(None)),
        other => Err(MaterialError::InvalidUniformValue {
            uniform: format!("<typed:{other}>"),
            reason: format!("unsupported explicit uniform type '{other}'"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 数字应推断为 float，且 glsl_type 返回 "float"
    #[test]
    fn test_number_is_float() {
        assert_eq!(
            uniform_value_from_json(&json!(0.8)).unwrap(),
            UniformValue::Float(0.8)
        );
        assert_eq!(
            uniform_value_from_json(&json!(0.8)).unwrap().glsl_type(),
            "float"
        );
    }

    // true/false 应推断为 bool
    #[test]
    fn test_bool() {
        assert_eq!(
            uniform_value_from_json(&json!(true)).unwrap(),
            UniformValue::Bool(true)
        );
        assert_eq!(
            uniform_value_from_json(&json!(false)).unwrap().glsl_type(),
            "bool"
        );
    }

    // 符合通道形状的字符串应归为 channels，其余否定
    #[test]
    fn test_channel_strings() {
        assert_eq!(
            uniform_value_from_json(&json!("rgb")).unwrap(),
            UniformValue::Channels("rgb".to_string())
        );
        assert_eq!(
            uniform_value_from_json(&json!("a")).unwrap(),
            UniformValue::Channels("a".to_string())
        );
        assert!(is_channel_string("rgba"));
        assert!(!is_channel_string(""));
        assert!(!is_channel_string("rgbba"));
        assert!(!is_channel_string("qx"));
    }

    // 普通图像 URL 字符串应推断为 sampler2D
    #[test]
    fn test_image_url_is_sampler2d() {
        assert_eq!(
            uniform_value_from_json(&json!("path/to/image.png")).unwrap(),
            UniformValue::Sampler2D("path/to/image.png".to_string())
        );
        assert_eq!(
            uniform_value_from_json(&json!("path/to/image.png"))
                .unwrap()
                .glsl_type(),
            "sampler2D"
        );
    }

    // 默认图像/默认立方贴图标识应各自归位
    #[test]
    fn test_default_ids() {
        assert_eq!(
            uniform_value_from_json(&json!("czm_defaultImage")).unwrap(),
            UniformValue::Sampler2D(DEFAULT_IMAGE_ID.to_string())
        );
        assert_eq!(
            uniform_value_from_json(&json!("czm_defaultCubeMap")).unwrap(),
            UniformValue::SamplerCube(None)
        );
    }

    // 含 red/green/blue/alpha 的对象应作为 Color 解析为 vec4
    #[test]
    fn test_color_object_is_vec4() {
        let v = uniform_value_from_json(&json!({
            "red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 0.5
        }))
        .unwrap();
        assert_eq!(v, UniformValue::Vec4([1.0, 0.0, 0.0, 0.5]));
        assert_eq!(v.glsl_type(), "vec4");
        assert_eq!(v.alpha_or_scalar(), Some(0.5));
    }

    // 含 x/y 的对象应推断为 vec2
    #[test]
    fn test_cartesian2_object_is_vec2() {
        assert_eq!(
            uniform_value_from_json(&json!({"x": 8.0, "y": 8.0})).unwrap(),
            UniformValue::Vec2([8.0, 8.0])
        );
    }

    // 双通道布尔向量应被强制为 1.0/0.0
    #[test]
    fn test_boolean_vector_coercion() {
        // fadeDirection：{ x: true, y: true } → vec2(1.0, 1.0)
        assert_eq!(
            uniform_value_from_json(&json!({"x": true, "y": false})).unwrap(),
            UniformValue::Vec2([1.0, 0.0])
        );
    }

    // 含六个立方贴图面键的对象应解析为 samplerCube
    #[test]
    fn test_cubemap_faces() {
        let v = uniform_value_from_json(&json!({
            "positiveX": "px.png", "negativeX": "nx.png",
            "positiveY": "py.png", "negativeY": "ny.png",
            "positiveZ": "pz.png", "negativeZ": "nz.png"
        }))
        .unwrap();
        match v {
            UniformValue::SamplerCube(Some(faces)) => {
                assert_eq!(faces.positive_x, "px.png");
                assert_eq!(faces.negative_z, "nz.png");
            }
            _ => panic!("expected samplerCube"),
        }
    }

    // 长度为 4/9/16 的数字数组应分别推断为 mat2/mat3/mat4
    #[test]
    fn test_matrix_arrays() {
        assert_eq!(
            uniform_value_from_json(&json!([1.0, 0.0, 0.0, 1.0])).unwrap(),
            UniformValue::Mat2([1.0, 0.0, 0.0, 1.0])
        );
        let mat3: Vec<f64> = vec![1.0; 9];
        assert!(matches!(
            uniform_value_from_json(&JsonValue::Array(
                mat3.iter().map(|v| json!(v)).collect()
            ))
            .unwrap(),
            UniformValue::Mat3(_)
        ));
        let mat4: Vec<f64> = vec![1.0; 16];
        assert!(matches!(
            uniform_value_from_json(&JsonValue::Array(
                mat4.iter().map(|v| json!(v)).collect()
            ))
            .unwrap(),
            UniformValue::Mat4(_)
        ));
        assert!(uniform_value_from_json(&json!([1.0, 2.0, 3.0])).is_err());
    }

    // 显式 type 标注应覆盖推断（ivec3 取整 x/y/z）
    #[test]
    fn test_explicit_ivec3_type() {
        let v = uniform_value_from_json(&json!({"type": "ivec3", "x": 1, "y": 1})).unwrap();
        assert_eq!(v, UniformValue::IVec3([1, 1, 0]));
        assert_eq!(v.glsl_type(), "ivec3");
    }

    // null 与无法推断形状的应对象均应报错
    #[test]
    fn test_invalid_values() {
        assert!(uniform_value_from_json(&JsonValue::Null).is_err());
        // 5 个属性无法推断
        assert!(uniform_value_from_json(&json!({
            "a": 1, "b": 2, "c": 3, "d": 4, "e": 5
        }))
        .is_err());
    }
}
