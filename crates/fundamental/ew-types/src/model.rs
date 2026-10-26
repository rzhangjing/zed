//! 类型卡的数据模型。字段顺序即 JSON 输出顺序，不要随意调整。

use serde::Serialize;

fn is_false(b: &bool) -> bool {
    !*b
}

/// 一个类型（结构体 / 枚举 / 别名）的完整卡片。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// 类型名，不含路径。
    pub name: String,
    /// `struct` / `enum` / `alias`。
    pub kind: String,
    /// 模块路径，如 `domain::order`。
    pub path: String,
    /// 首行摘要。
    pub doc: String,
    /// `# 不变量` 段，逐条。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invariants: Vec<String>,
    /// 泛型参数名。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub generics: Vec<String>,
    /// 结构体字段。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    /// 枚举变体。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
    /// 枚举状态流转。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<Transition>,
    /// 构造入口。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctor: Option<Ctor>,
    /// derive 列表，原样保留顺序。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub derives: Vec<String>,
    /// 是否 `#[non_exhaustive]`。
    #[serde(skip_serializing_if = "is_false")]
    pub non_exhaustive: bool,
    /// 定义所在位置，`文件:行号`。
    pub src: String,
}

/// 字段或枚举变体载荷字段。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    /// 类型源码原文，已规范化空白。
    #[serde(rename = "type")]
    pub ty: String,
    /// 注释原文。
    pub doc: String,
    /// 注释里的含义段。
    pub meaning: String,
    /// 单位。`null` 表示注释未声明。
    pub unit: Option<String>,
    /// 范围。`null` 表示注释未声明。
    pub range: Option<String>,
    /// 精度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<String>,
    /// 默认值。`null` 表示注释未声明。
    pub default: Option<String>,
    /// 是否可空，来自注释的"可为空"表述。
    pub nullable: bool,
    /// 五段之外的自由说明，如"保持下单顺序"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 注释里缺失的必写段，如 `["unit","range"]`。
    /// 空数组表示三段都写了；`null` 值 + 空数组表示"明确没有"。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

/// 枚举变体。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub name: String,
    pub doc: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub payload: Vec<Field>,
}

/// 状态流转的一条边。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub from: String,
    pub to: String,
    pub via: String,
}

/// 构造入口。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ctor {
    pub name: String,
    /// 参数与返回值，如 `(id, items) -> Result<Order, OrderError>`。
    pub sig: String,
    /// 构造时校验的内容，取自 `# 错误` 段与不变量。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
}

/// 索引条目，只够判断"要不要取这张卡"。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub doc: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub fields: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub variants: usize,
    pub src: String,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Card {
    pub fn index_entry(&self) -> IndexEntry {
        IndexEntry {
            name: self.name.clone(),
            kind: self.kind.clone(),
            path: self.path.clone(),
            doc: self.doc.clone(),
            fields: self.fields.len(),
            variants: self.variants.len(),
            src: self.src.clone(),
        }
    }

    /// 该类型直接引用到的类型名，用于反查。
    pub fn referenced_types(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut push = |ty: &str| {
            collect_idents(ty, &mut out);
        };
        for f in &self.fields {
            push(&f.ty);
        }
        for v in &self.variants {
            for f in &v.payload {
                push(&f.ty);
            }
        }
        if let Some(c) = &self.ctor {
            collect_idents(&c.sig, &mut out);
        }
        out.sort();
        out.dedup();
        out
    }
}

/// 从类型文本里抽出可能引用的类型名：跳过内置标量与关键字。
fn collect_idents(ty: &str, out: &mut Vec<String>) {
    const SKIP: &[&str] = &[
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
        "f32", "f64", "bool", "char", "str", "String", "Vec", "Option", "Result", "Box", "Arc",
        "Rc", "Cow", "HashMap", "BTreeMap", "HashSet", "BTreeSet", "VecDeque", "Duration",
        "Instant", "SystemTime", "Self", "self", "dyn", "impl", "mut", "ref", "and", "where",
        "Result", "Ok", "Err", "Some", "None",
    ];
    let mut cur = String::new();
    for ch in ty.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else {
            if !cur.is_empty() {
                let id = std::mem::take(&mut cur);
                let is_type_like = id.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                if is_type_like && !SKIP.contains(&id.as_str()) {
                    out.push(id);
                }
            }
        }
    }
    if !cur.is_empty() {
        let is_type_like = cur.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        if is_type_like && !SKIP.contains(&cur.as_str()) {
            out.push(cur);
        }
    }
}
