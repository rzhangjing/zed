//! 材质装配：Fabric 模板 -> GLSL 着色器源码 + uniforms。
//!
//! 构造流水线依次涉及以下阶段：材质初始化、方法定义生成、uniform
//! 与子材质创建、token 替换与计数，以及半透明性判定。
//!
//! 领域层生成完整的 GLSL `czm_getMaterial` 着色器源码及
//! uniform 簿记；随后由渲染适配器负责将该源码翻译为目标着色语言。

use crate::cache::CachedMaterial;
use crate::error::MaterialError;
use crate::fabric::FabricTemplate;
use crate::translucent::TranslucentSpec;
use crate::uniform::UniformValue;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

/// 用于为匿名材质类型生成 GUID 的计数器。
/// 每次自增产出一个唯一的 `material-<hex>` 类型名。
static GUID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 基于自增计数器生成一个匿名的材质 GUID 类型名。
fn create_guid() -> String {
    let n = GUID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("material-{:016x}", n)
}

/// 构造 [`Material`] 的选项。
///
/// 对应材质构造函数的输入对象（`{ strict, translucent, fabric, count }`）。
/// 其中 `count` 成员是内部的一个共享重命名计数器，改为以 `&mut usize`
/// 的形式在构建函数间传递。
#[derive(Debug, Clone, Default)]
pub struct MaterialOptions {
    /// 为 `true` 时，未使用的 uniforms / channels / 子材质会报错。
    pub strict: bool,
    /// 显式的半透明覆盖，强制材质为或不透明。
    ///
    /// 领域层仅支持布尔形式；内置材质所使用的函数形式由
    /// [`TranslucentSpec`] 承载。
    pub translucent: Option<bool>,
    /// Fabric 模板，材质描述的来源。
    pub fabric: FabricTemplate,
}

/// 一个已构造的 Fabric 材质。
///
/// [`Material::shader_source`] 是完整装配好的 GLSL（子材质函数前置，
/// uniforms 重命名为唯一 id）。子材质以嵌套 [`Material`] 的形式保留，
/// 使其 uniform 值可单独寻址。
#[derive(Debug, Clone)]
pub struct Material {
    /// 材质类型名（匿名为 GUID）。
    type_name: String,
    /// 装配完成的 GLSL 着色器源码。
    shader_source: String,
    /// 以原始（Fabric）名称为键的公开 uniform 值。
    uniforms: BTreeMap<String, UniformValue>,
    /// 以 Fabric 名称为键的子材质。
    materials: BTreeMap<String, Material>,
    /// 该材质自身解析后的半透明性规格（在材质初始化末尾确定的值）。
    own_translucent: Option<TranslucentSpec>,
    /// 重命名后（着色器）uniform id -> 原始（Fabric）uniform id。
    uniform_bindings: BTreeMap<String, String>,
}

impl Material {
    /// 材质类型名（当 Fabric 没有 `type` 时为一个 GUID）。
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// 装配好的 GLSL 着色器源码。
    pub fn shader_source(&self) -> &str {
        &self.shader_source
    }

    /// 以原始 Fabric 名称为键的公开 uniform 值。
    pub fn uniforms(&self) -> &BTreeMap<String, UniformValue> {
        &self.uniforms
    }

    /// 对公开 uniform 值的可变访问。
    pub fn uniforms_mut(&mut self) -> &mut BTreeMap<String, UniformValue> {
        &mut self.uniforms
    }

    /// 以 Fabric 名称为键的子材质。
    pub fn materials(&self) -> &BTreeMap<String, Material> {
        &self.materials
    }

    /// 重命名后（着色器）uniform id -> 原始 Fabric uniform id。
    pub fn uniform_bindings(&self) -> &BTreeMap<String, String> {
        &self.uniform_bindings
    }

    /// 该材质（及其所有子材质）是否半透明。
    ///
    /// 将本材质自身的规则与所有后代材质的规则做 AND 运算：
    /// 只有当材质及其全部后代都可保持不透明时，整体才判为不透明。
    pub fn is_translucent(&self) -> bool {
        // 自身无规则时保守视为半透明（true）
        let own = self
            .own_translucent
            .as_ref()
            .map(|spec| spec.evaluate(&self.uniforms))
            .unwrap_or(true);
        own && self.materials.values().all(Material::is_translucent)
    }

    /// 展平的映射：着色器（重命名后）uniform id -> 当前值，覆盖本材质
    /// 及所有子材质。便于按着色器名称绑定 uniform 的渲染适配器使用。
    pub fn shader_uniforms(&self) -> BTreeMap<String, UniformValue> {
        let mut out = BTreeMap::new();
        self.collect_shader_uniforms(&mut out);
        out
    }

    /// 递归收集本材质及子材质的重命名后 uniform id 到当前值的映射。
    fn collect_shader_uniforms(&self, out: &mut BTreeMap<String, UniformValue>) {
        for (renamed, original) in &self.uniform_bindings {
            if let Some(value) = self.uniforms.get(original) {
                out.insert(renamed.clone(), value.clone());
            }
        }
        for sub in self.materials.values() {
            sub.collect_shader_uniforms(out);
        }
    }

    /// 本材质及其子材质中所有纹理（`sampler2D` / `samplerCube`）uniform
    /// 的名称，以原始 Fabric 名称为键。
    pub fn texture_uniforms(&self) -> BTreeMap<String, UniformValue> {
        let mut out = BTreeMap::new();
        self.collect_texture_uniforms(&mut out);
        out
    }

    /// 递归收集本材质及子材质中所有纹理（sampler2D/samplerCube）uniform。
    fn collect_texture_uniforms(&self, out: &mut BTreeMap<String, UniformValue>) {
        for (name, value) in &self.uniforms {
            if matches!(
                value,
                UniformValue::Sampler2D(_) | UniformValue::SamplerCube(_)
            ) {
                out.insert(name.clone(), value.clone());
            }
        }
        for sub in self.materials.values() {
            sub.collect_texture_uniforms(out);
        }
    }
}

/// 从 Fabric 模板构建一个 [`Material`]。
///
/// 这是材质缓存使用的顶层入口。它返回材质，以及收集到的
/// 半透明规则数量（自身 + 后代），父材质需要
/// 该数量来计算其默认半透明性。
pub(crate) fn build_material(
    fabric: FabricTemplate,
    strict: bool,
    options_translucent: Option<bool>,
    cache: &HashMap<String, CachedMaterial>,
    count: &mut usize,
) -> Result<(Material, usize), MaterialError> {
    // 模板已从原始 fabric 选项深拷贝而来，此处直接接管该所有权
    let mut template = fabric;

    // 材质类型名：优先用模板的 type，缺失时生成一个 GUID
    let type_name = template.type_name.clone().unwrap_or_else(create_guid);

    // 缓存合并：基于已存储的模板构建（用户优先）。
    let cached = cache.get(&type_name);
    let cached_translucent: Option<TranslucentSpec> = if let Some(cached_material) = cached {
        template.merge_over(&cached_material.fabric);
        cached_material.translucent.clone()
    } else {
        None
    };

    // 校验模板结构错误（source/components 互斥、uniform 与子材质命名冲突）
    template.validate()?;

    // 从 source 或 components 生成 czm_getMaterial 方法体
    let mut shader_source = String::new();
    create_method_definition(&template, &mut shader_source);

    // 处理并绑定模板声明的每个 uniform
    let mut uniforms = BTreeMap::new();
    let mut uniform_bindings = BTreeMap::new();
    create_uniforms(
        &template,
        strict,
        &mut shader_source,
        &mut uniforms,
        &mut uniform_bindings,
        count,
    )?;

    // 递归构建子材质并将其源码拼接进父材质
    let mut materials = BTreeMap::new();
    let mut sub_translucent_count = 0usize;
    create_sub_materials(
        &template,
        strict,
        cache,
        count,
        &mut shader_source,
        &mut materials,
        &mut sub_translucent_count,
    )?;

    // 解析半透明性，优先级：显式 options.translucent > 缓存 > 默认值
    //（无任何半透明规则时默认为 Always，否则默认不确定）
    let default_translucent = if sub_translucent_count == 0 {
        Some(TranslucentSpec::Always)
    } else {
        None
    };
    // 将布尔覆盖映射为 Always/Never，再按优先级链依次回退
    let resolved = options_translucent
        .map(|b| {
            if b {
                TranslucentSpec::Always
            } else {
                TranslucentSpec::Never
            }
        })
        .or(cached_translucent)
        .or(default_translucent);
    // 本材质自身贡献一个半透明规则（仅当 resolved 存在时）
    let own_count = usize::from(resolved.is_some());

    Ok((
        Material {
            type_name,
            shader_source,
            uniforms,
            materials,
            own_translucent: resolved,
            uniform_bindings,
        },
        sub_translucent_count + own_count,
    ))
}

/// `isMaterialFused`：某分量表达式是否引用了任何子材质？
/// 只要表达式的文本包含任一子材质 id，即视为发生了融合。
fn is_material_fused(component_expr: &str, materials: &BTreeMap<String, FabricTemplate>) -> bool {
    materials
        .keys()
        .any(|id| component_expr.contains(id.as_str()))
}

/// `createMethodDefinition`：从 `source` 或 `components` 构建 `czm_getMaterial` 函数体。
fn create_method_definition(template: &FabricTemplate, shader_source: &mut String) {
    // 若提供自定义 source，则逐字发出并提前返回
    if let Some(source) = &template.source {
        shader_source.push_str(source);
        shader_source.push('\n');
        return;
    }

    // 否则生成标准的 czm_getMaterial 函数骨架（先取默认材质）
    shader_source
        .push_str("czm_material czm_getMaterial(czm_materialInput materialInput)\n{\n");
    shader_source.push_str("czm_material material = czm_getDefaultMaterial(materialInput);\n");

    if let Some(components) = &template.components {
        // 是否含子材质决定了 diffuse/emission 是否需要融合判定
        let is_multi_material = !template.materials.is_empty();
        for (component, expr) in components.iter() {
            if component == "diffuse" || component == "emission" {
                // 融合：表达式引用了子材质时直接使用，否则套 gamma 校正
                let is_fusion = is_multi_material && is_material_fused(expr, &template.materials);
                let component_source = if is_fusion {
                    expr.to_string()
                } else {
                    format!("czm_gammaCorrect({})", expr)
                };
                // 注意换行符前的尾随空格：diffuse/emission/alpha 分量发出
                // `material.<c> = <src>; \n`（分号后带一个空格）。
                shader_source.push_str(&format!("material.{} = {}; \n", component, component_source));
            } else if component == "alpha" {
                // alpha 分量不做 gamma 校正，但同样保留尾随空格
                shader_source.push_str(&format!("material.alpha = {}; \n", expr));
            } else {
                // 其余分量（specular/shininess/normal）使用无尾随空格的标准赋值
                shader_source.push_str(&format!("material.{} = {};\n", component, expr));
            }
        }
    }

    // 函数体以返回装配好的 material 结束
    shader_source.push_str("return material;\n}\n");
}

/// `createUniforms`：处理模板中声明的每个 uniform。
fn create_uniforms(
    template: &FabricTemplate,
    strict: bool,
    shader_source: &mut String,
    uniforms: &mut BTreeMap<String, UniformValue>,
    bindings: &mut BTreeMap<String, String>,
    count: &mut usize,
) -> Result<(), MaterialError> {
    // 在一个可增长的副本上操作，以便我们能动态添加 `<image>Dimensions`
    // uniform（模板声明的 uniforms 会因此被扩充）。
    let mut all_uniforms = template.uniforms.clone();
    let ids: Vec<String> = template.uniforms.keys().cloned().collect();
    let mut processed = HashSet::new();
    // 逐个处理已声明的 uniform（固定为快照的键列表，避免遍历时修改）
    for id in ids {
        create_uniform(
            &id,
            strict,
            shader_source,
            &mut all_uniforms,
            uniforms,
            bindings,
            count,
            &mut processed,
        )?;
    }
    Ok(())
}

/// `createUniform`：声明、重命名并绑定单个 uniform。
#[allow(clippy::too_many_arguments)]
fn create_uniform(
    uniform_id: &str,
    strict: bool,
    shader_source: &mut String,
    all_uniforms: &mut BTreeMap<String, UniformValue>,
    uniforms: &mut BTreeMap<String, UniformValue>,
    bindings: &mut BTreeMap<String, String>,
    count: &mut usize,
    processed: &mut HashSet<String>,
) -> Result<(), MaterialError> {
    // 去重：同一 uniform 只处理一次（避免 Dimensions 递归时重复处理）
    if !processed.insert(uniform_id.to_string()) {
        return Ok(());
    }

    // 取出 uniform 值（缺失即为非法），并推断其 GLSL 类型
    let uniform_value = all_uniforms
        .get(uniform_id)
        .cloned()
        .ok_or_else(|| MaterialError::InvalidUniformType {
            uniform: uniform_id.to_string(),
        })?;
    let uniform_type = uniform_value.glsl_type();

    if uniform_type == "channels" {
        // channels 是一种文本替换，而非真正的 uniform。
        let channels_str = match &uniform_value {
            UniformValue::Channels(s) => s.clone(),
            _ => unreachable!("glsl_type() reported channels for a non-Channels value"),
        };
        let replaced = replace_token(shader_source, uniform_id, &channels_str, false);
        // strict 下若源码中无任何替换发生，则视为未使用的 channels
        if replaced == 0 && strict {
            return Err(MaterialError::StrictUnusedChannels {
                uniform: uniform_id.to_string(),
            });
        }
        return Ok(());
    }

    // WebGL 无法在 GLSL 中查询纹理尺寸，因此当源码使用纹理时，需创建一个
    // 配套的 `<image>Dimensions` ivec3 uniform 来显式传入尺寸。
    if uniform_type == "sampler2D" {
        let dims_name = format!("{}Dimensions", uniform_id);
        if get_number_of_tokens(shader_source, &dims_name) > 0 {
            all_uniforms.insert(dims_name.clone(), UniformValue::IVec3([1, 1, 0]));
            create_uniform(
                &dims_name,
                strict,
                shader_source,
                all_uniforms,
                uniforms,
                bindings,
                count,
                processed,
            )?;
        }
    }

    // 若源码尚未声明，则前置该声明。
    if !has_uniform_declaration(shader_source, uniform_type, uniform_id) {
        let declaration = format!("uniform {} {};", uniform_type, uniform_id);
        shader_source.insert_str(0, &declaration);
    }

    // 重命名为唯一 id：`<id>_<count++>`。
    let new_id = format!("{}_{}", uniform_id, *count);
    *count += 1;
    let replaced = replace_token(shader_source, uniform_id, &new_id, true);
    // 计数恰为一，意味着只有声明被重命名，即该 uniform 被声明却在函数体中从未使用。
    if replaced == 1 && strict {
        return Err(MaterialError::StrictUnusedUniform {
            uniform: uniform_id.to_string(),
        });
    }

    uniforms.insert(uniform_id.to_string(), uniform_value);
    bindings.insert(new_id, uniform_id.to_string());
    Ok(())
}

/// `createSubMaterials`：递归构建子材质，并将其着色器源码与方法调用拼接进父材质。
#[allow(clippy::too_many_arguments)]
fn create_sub_materials(
    template: &FabricTemplate,
    strict: bool,
    cache: &HashMap<String, CachedMaterial>,
    count: &mut usize,
    shader_source: &mut String,
    materials: &mut BTreeMap<String, Material>,
    sub_translucent_count: &mut usize,
) -> Result<(), MaterialError> {
    for (sub_id, sub_template) in &template.materials {
        // 递归构造子材质（子材质没有 options.translucent）
        let (mut sub_material, sub_count) =
            build_material(sub_template.clone(), strict, None, cache, count)?;
        // 累加子材质及其后代的半透明规则数
        *sub_translucent_count += sub_count;

        // 将子材质的 czm_getMaterial 重命名为唯一的方法名
        let new_method_name = format!("czm_getMaterial_{}", *count);
        *count += 1;
        replace_token(
            &mut sub_material.shader_source,
            "czm_getMaterial",
            &new_method_name,
            true,
        );

        // 将子材质的源码前置到父源码之前（保证方法先定义后调用）
        let sub_source = std::mem::take(&mut sub_material.shader_source);
        let parent_source = std::mem::take(shader_source);
        *shader_source = sub_source + &parent_source;

        // 将每个子材质 id 替换为一次对新方法名的调用，并传入 materialInput
        let method_call = format!("{}(materialInput)", new_method_name);
        let replaced = replace_token(shader_source, sub_id, &method_call, true);
        if replaced == 0 && strict {
            return Err(MaterialError::StrictUnusedMaterial { id: sub_id.clone() });
        }

        materials.insert(sub_id.clone(), sub_material);
    }
    Ok(())
}

/// 将 `source` 中独立出现的 `token` 替换为 `new_token`，返回替换次数。
///
/// 独立出现是指其前面不是单词字符（当 `exclude_period` 为 true 时前面也
/// 不能是点号）且后面不是单词字符。此判定等价于按正则
/// `([\w.])?token([\w])?` 做字节级匹配（当 `exclude_period` 为 false
/// 时，前缀字符类中去掉 `.`）。
pub(crate) fn replace_token(
    source: &mut String,
    token: &str,
    new_token: &str,
    exclude_period: bool,
) -> usize {
    let bytes = source.as_bytes();
    let token_bytes = token.as_bytes();
    let tlen = token_bytes.len();
    let n = bytes.len();
    if tlen == 0 || tlen > n {
        return 0;
    }

    let mut result: Vec<u8> = Vec::with_capacity(n);
    let mut count = 0usize;
    let mut i = 0usize;
    while i < n {
        if i + tlen <= n && &bytes[i..i + tlen] == token_bytes {
            let prefix_ok = if i == 0 {
                true
            } else {
                let prev = bytes[i - 1];
                if exclude_period {
                    !is_word_byte(prev) && prev != b'.'
                } else {
                    !is_word_byte(prev)
                }
            };
            let suffix_ok = i + tlen >= n || !is_word_byte(bytes[i + tlen]);
            if prefix_ok && suffix_ok {
                result.extend_from_slice(new_token.as_bytes());
                count += 1;
                i += tlen;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }

    // SAFETY：源码本是有效的 UTF-8；我们逐字节原样复制了除 ASCII token 范围
    // 外的每个字节，那些范围被替换为 ASCII 的 `new_token`。
    *source = String::from_utf8(result).expect("replace_token preserves UTF-8 validity");
    count
}

/// 统计 `token` 独立出现的次数而不修改源码。
/// 实现上复用 [`replace_token`]，将 token 原地替换为自身以触发计数。
fn get_number_of_tokens(source: &mut String, token: &str) -> usize {
    replace_token(source, token, token, true)
}

/// 判断字节是否为单词字符（字母、数字或下划线）。
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 测试源码是否已包含带灵活空白的 `uniform <type> <id> ;`，等价于
/// 子串正则 `uniform\s+<type>\s+<id>\s*;` 的匹配。
fn has_uniform_declaration(source: &str, uniform_type: &str, uniform_id: &str) -> bool {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let kw = b"uniform";
    let type_bytes = uniform_type.as_bytes();
    let id_bytes = uniform_id.as_bytes();

    let mut i = 0usize;
    // 逐字节扫描：匹配 `uniform` 关键字后依次跳过空白、匹配类型、
    // 跳过空白、匹配 id、跳过空白，最后要求紧跟分号
    while i + kw.len() <= n {
        if &bytes[i..i + kw.len()] == kw {
            let mut j = i + kw.len();
            let s1 = j;
            while j < n && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j > s1 && j + type_bytes.len() <= n && &bytes[j..j + type_bytes.len()] == type_bytes
            {
                let mut k = j + type_bytes.len();
                let s2 = k;
                while k < n && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k > s2 && k + id_bytes.len() <= n && &bytes[k..k + id_bytes.len()] == id_bytes {
                    let mut m = k + id_bytes.len();
                    while m < n && bytes[m].is_ascii_whitespace() {
                        m += 1;
                    }
                    if m < n && bytes[m] == b';' {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MaterialSystem;

    fn build(fabric: &str) -> Material {
        let template = FabricTemplate::from_json_str(fabric).unwrap();
        let system = MaterialSystem::with_builtin_materials();
        system
            .build(template, false, None)
            .expect("material should build")
    }

    // Color 类型：验证 color uniform 声明、唯一重命名与 gamma 校正后的 diffuse/alpha
    #[test]
    fn test_color_material_shader() {
        let m = build(r#"{"type": "Color"}"#);
        assert_eq!(m.type_name(), "Color");
        let src = m.shader_source();
        // color uniform 被声明并以唯一后缀重命名。
        assert!(src.contains("uniform vec4 color_"));
        // diffuse 经过 gamma 校正（Color 不是多材质融合）。
        assert!(src.contains("material.diffuse = czm_gammaCorrect(color_"));
        assert!(src.contains(".rgb); \n"));
        // alpha 赋值带有尾随空格。
        assert!(src.contains("material.alpha = color_"));
        assert!(src.contains(".a; \n"));
        assert!(src.contains("return material;"));
        // 该 uniform 值为内置默认值。
        assert_eq!(
            m.uniforms().get("color"),
            Some(&UniformValue::Vec4([1.0, 0.0, 0.0, 0.5]))
        );
    }

    // 默认 Color 的 alpha=0.5，应判为半透明
    #[test]
    fn test_color_material_translucency() {
        // 默认 Color 的 alpha 为 0.5 -> 半透明。
        let m = build(r#"{"type": "Color"}"#);
        assert!(m.is_translucent());
    }

    // 显式传入 alpha=1.0 应覆盖为不透明
    #[test]
    fn test_color_material_opaque_override() {
        let m = build(r#"{"type": "Color", "uniforms": {"color": {"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0}}}"#);
        assert!(!m.is_translucent());
    }

    // Checkerboard 的两个颜色 uniform 均应被声明并重命名
    #[test]
    fn test_checkerboard_renames_both_colors() {
        let m = build(r#"{"type": "Checkerboard"}"#);
        let src = m.shader_source();
        assert!(src.contains("uniform vec4 lightColor_"));
        assert!(src.contains("uniform vec4 darkColor_"));
        // 源码使用重命名后的 uniforms。
        assert!(src.contains("lightColor_"));
        assert!(src.contains("darkColor_"));
        // Checkerboard 的 GLSL 源码已内联嵌入。
        assert!(src.contains("czm_getMaterial"));
        // 默认 light/dark 颜色的 alpha 为 0.5 -> 半透明。
        assert!(m.is_translucent());
    }

    // Image 类型的纹理与 repeat uniform 均应绑定
    #[test]
    fn test_image_material_texture_and_repeat() {
        let m = build(r#"{"type": "Image"}"#);
        let src = m.shader_source();
        assert!(src.contains("uniform sampler2D image_"));
        assert!(src.contains("uniform vec2 repeat_"));
        assert!(src.contains("texture(image_"));
        assert!(src.contains("fract(repeat_"));
        // 默认 image uniform 是哨兵 id。
        assert_eq!(
            m.uniforms().get("image"),
            Some(&UniformValue::Sampler2D(
                crate::uniform::DEFAULT_IMAGE_ID.to_string()
            ))
        );
        assert!(m.texture_uniforms().contains_key("image"));
    }

    // channels token 应被文本替换且不作为真正 uniform 保留
    #[test]
    fn test_diffuse_map_channels_substitution() {
        let m = build(r#"{"type": "DiffuseMap"}"#);
        let src = m.shader_source();
        // `channels` token 被文本替换为 `rgb`。
        assert!(src.contains(".rgb"));
        assert!(!src.contains(".channels"));
        // channels 不是真正的 uniform。
        assert!(!m.uniforms().contains_key("channels"));
        assert!(m.uniforms().contains_key("image"));
        assert!(m.uniforms().contains_key("repeat"));
        // DiffuseMap 从不半透明。
        assert!(!m.is_translucent());
    }

    // BumpMap 使用 imageDimensions，应自动生成配套的 ivec3 uniform
    #[test]
    fn test_bump_map_auto_dimensions_uniform() {
        let m = build(r#"{"type": "BumpMap"}"#);
        // BumpMap 的 GLSL 使用 imageDimensions，因此配套的 uniform 存在。
        let dims = m.uniforms().get("imageDimensions");
        assert_eq!(dims, Some(&UniformValue::IVec3([1, 1, 0])));
        assert!(m.shader_source().contains("uniform ivec3 imageDimensions_"));
        assert!(!m.is_translucent());
    }

    // 无 type 的自定义 components 应获得 GUID 类型名
    #[test]
    fn test_custom_components_material_gets_guid_type() {
        let m = build(r#"{"components": {"diffuse": "vec3(1.0)", "alpha": "0.5"}}"#);
        assert!(m.type_name().starts_with("material-"));
        let src = m.shader_source();
        assert!(src.contains("material.diffuse = czm_gammaCorrect(vec3(1.0)); \n"));
        assert!(src.contains("material.alpha = 0.5; \n"));
    }

    // 自定义 source 应逐字发出并带尾随换行
    #[test]
    fn test_custom_source_material() {
        let src = "czm_material czm_getMaterial(czm_materialInput materialInput)\n{\n  czm_material m = czm_getDefaultMaterial(materialInput);\n  m.diffuse = vec3(0.5);\n  return m;\n}";
        let json = format!(r#"{{"source": {:?}}}"#, src);
        let m = build(&json);
        // 源码逐字发出，带一个尾随换行。
        assert!(m.shader_source().starts_with(src));
        assert!(m.shader_source().ends_with('\n'));
    }

    // 子材质应被重命名、前置源码并替换为方法调用
    #[test]
    fn test_sub_material_composition() {
        // 一个将 Color 子材质融合进其 diffuse 的父材质。
        let m = build(
            r#"{
                "materials": {
                    "base": {"type": "Color"}
                },
                "components": {
                    "diffuse": "base.diffuse",
                    "alpha": "base.alpha"
                }
            }"#,
        );
        let src = m.shader_source();
        // 子材质的 czm_getMaterial 被重命名。
        assert!(src.contains("czm_getMaterial_"));
        // 子材质 id 被替换为一个方法调用。
        assert!(src.contains("(materialInput).diffuse"));
        assert!(!src.contains("base.diffuse"));
        // 子材质存在。
        assert!(m.materials().contains_key("base"));
        // 融合后的 diffuse 不会被 czm_gammaCorrect 包裹。
        assert!(src.contains("material.diffuse = czm_getMaterial_"));
    }

    // 子材质的半透明性应传播到父材质
    #[test]
    fn test_sub_material_translucency_propagates() {
        // 父材质默认不透明，但 Color 子材质（alpha 0.5）
        // 使整体变为半透明。
        let m = build(
            r#"{
                "materials": {"base": {"type": "Color"}},
                "components": {"diffuse": "base.diffuse", "alpha": "base.alpha"}
            }"#,
        );
        assert!(m.is_translucent());
    }

    // strict 下未被使用的 uniform 应报错
    #[test]
    fn test_strict_unused_uniform_errors() {
        let template = FabricTemplate::from_json_str(
            r#"{"uniforms": {"unused": 1.0}, "components": {"diffuse": "vec3(1.0)"}}"#,
        )
        .unwrap();
        let system = MaterialSystem::with_builtin_materials();
        let err = system
            .build(template, true, None)
            .expect_err("strict build should fail");
        assert!(matches!(
            err,
            MaterialError::StrictUnusedUniform { ref uniform } if uniform == "unused"
        ));
    }

    // strict 下未被使用的 channels 应报错
    #[test]
    fn test_strict_unused_channels_errors() {
        let template = FabricTemplate::from_json_str(
            r#"{"uniforms": {"channels": "rgb"}, "components": {"diffuse": "vec3(1.0)"}}"#,
        )
        .unwrap();
        let system = MaterialSystem::with_builtin_materials();
        let err = system
            .build(template, true, None)
            .expect_err("strict build should fail");
        assert!(matches!(
            err,
            MaterialError::StrictUnusedChannels { ref uniform } if uniform == "channels"
        ));
    }

    // token 替换的词边界判定：点号前缀与大小写不应误匹配
    #[test]
    fn test_replace_token_boundaries() {
        // 替换 `color` 时不得影响 `lightColor`（大小写），且
        // （在 exclude_period = true 的点号前缀下）替换 `diffuse` 时不得影响
        // `material.diffuse`。
        let mut s = "material.diffuse = lightColor.rgb;".to_string();
        let n = replace_token(&mut s, "diffuse", "diffuse_0", true);
        assert_eq!(n, 0);
        assert_eq!(s, "material.diffuse = lightColor.rgb;");

        let mut s2 = "vec3 color = color.rgb;".to_string();
        let n2 = replace_token(&mut s2, "color", "color_0", true);
        // 两处独立的 `color` 都被替换；`color.rgb` 的
        // `color` 是独立的（后缀为 `.`），`vec3 color` 中的 `color` 亦然。
        assert_eq!(n2, 2);
        assert_eq!(s2, "vec3 color_0 = color_0.rgb;");
    }

    // exclude_period=false 时应允许点号前缀（channels 替换）
    #[test]
    fn test_replace_token_period_allowed_for_channels() {
        let mut s = "texture(image, st).channels".to_string();
        let n = replace_token(&mut s, "channels", "rgb", false);
        assert_eq!(n, 1);
        assert_eq!(s, "texture(image, st).rgb");
    }

    // shader_uniforms 应展平包含子材质重命名后的 uniform
    #[test]
    fn test_shader_uniforms_flattened() {
        let m = build(
            r#"{
                "materials": {"base": {"type": "Color"}},
                "components": {"diffuse": "base.diffuse", "alpha": "base.alpha"}
            }"#,
        );
        let flat = m.shader_uniforms();
        // 包含子材质重命名后的 color uniform。
        assert!(flat.values().any(|v| matches!(v, UniformValue::Vec4(_))));
    }
}
