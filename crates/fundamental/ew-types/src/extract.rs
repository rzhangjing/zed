//! 从 Rust 源码提取类型卡。用 `syn` 解析，不依赖 nightly 的 rustdoc JSON。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::{Attribute, Fields, ImplItem, Item, Meta, Token, Type};

use crate::docparse::{parse_doc, parse_field_doc};
use crate::model::{Card, Ctor, Field, Variant};

pub struct ExtractResult {
    pub cards: Vec<Card>,
    pub errors: Vec<String>,
}

/// 递归收集 `src/` 下的 `.rs` 文件；没有 `src/` 就扫根目录（跳过 target 与隐藏目录）。
pub fn collect_files(root: &Path) -> Vec<PathBuf> {
    let src = root.join("src");
    let base = if src.is_dir() { src } else { root.to_path_buf() };
    let mut out = Vec::new();
    walk(&base, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name == "target" || name == ".git" || name == "node_modules" {
                continue;
            }
            walk(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

/// 全量提取。
pub fn extract_all(root: &Path) -> ExtractResult {
    let mut cards: Vec<Card> = Vec::new();
    let mut ctors: BTreeMap<String, Vec<Ctor>> = BTreeMap::new();
    let mut errors = Vec::new();

    for file in collect_files(root) {
        let text = match fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                errors.push(format!("{}: 读取失败 {}", file.display(), e));
                continue;
            }
        };
        let ast = match syn::parse_file(&text) {
            Ok(a) => a,
            Err(e) => {
                errors.push(format!("{}: 解析失败 {}", file.display(), e));
                continue;
            }
        };
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let module = module_path(&rel);
        walk_items(&ast.items, &module, &rel, &mut cards, &mut ctors);
    }

    for card in &mut cards {
        if card.ctor.is_none() {
            if let Some(list) = ctors.get(&card.name) {
                card.ctor = list.first().cloned();
            }
        }
    }

    cards.sort_by(|a, b| a.path.cmp(&b.path).then(a.name.cmp(&b.name)));
    ExtractResult { cards, errors }
}

/// `src/domain/order.rs` → `domain::order`
fn module_path(rel: &str) -> String {
    let mut s = rel.to_string();
    if let Some(rest) = s.strip_prefix("src/") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_suffix(".rs") {
        s = rest.to_string();
    }
    let comps: Vec<&str> = s.split('/').filter(|c| !c.is_empty()).collect();
    let comps: Vec<&str> = comps
        .into_iter()
        .filter(|c| *c != "mod" && *c != "lib" && *c != "main")
        .collect();
    comps.join("::")
}

fn walk_items(
    items: &[Item],
    module: &str,
    file: &str,
    cards: &mut Vec<Card>,
    ctors: &mut BTreeMap<String, Vec<Ctor>>,
) {
    for item in items {
        match item {
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    let sub = if module.is_empty() {
                        m.ident.to_string()
                    } else {
                        format!("{}::{}", module, m.ident)
                    };
                    walk_items(inner, &sub, file, cards, ctors);
                }
            }
            Item::Struct(s) => {
                let doc = parse_doc(&doc_lines(&s.attrs));
                let (kind, mut fields) = match &s.fields {
                    Fields::Named(named) => ("struct", fields_of(named.named.iter())),
                    Fields::Unnamed(unnamed) => {
                        let f = fields_of(unnamed.unnamed.iter());
                        if f.len() == 1 {
                            ("newtype", f)
                        } else {
                            ("struct", f)
                        }
                    }
                    Fields::Unit => ("struct", Vec::new()),
                };
                // newtype 的内层字段没有独立注释时，用类型摘要补上它的含义
                if kind == "newtype" {
                    if let Some(f) = fields.first_mut() {
                        if f.doc.trim().is_empty() {
                            f.meaning = doc.summary.clone();
                        }
                    }
                }
                cards.push(Card {
                    name: s.ident.to_string(),
                    kind: kind.to_string(),
                    path: module.to_string(),
                    doc: doc.summary.clone(),
                    invariants: doc.invariants.clone(),
                    generics: generic_names(&s.generics),
                    fields,
                    variants: Vec::new(),
                    transitions: Vec::new(),
                    ctor: None,
                    derives: derives(&s.attrs),
                    non_exhaustive: has_attr(&s.attrs, "non_exhaustive"),
                    src: format!("{}:{}", file, line_of(&s.ident)),
                });
            }
            Item::Enum(e) => {
                let doc = parse_doc(&doc_lines(&e.attrs));
                let variants: Vec<Variant> = e
                    .variants
                    .iter()
                    .map(|v| {
                        let vdoc = parse_doc(&doc_lines(&v.attrs));
                        Variant {
                            name: v.ident.to_string(),
                            doc: vdoc.summary,
                            payload: match &v.fields {
                                Fields::Named(n) => fields_of(n.named.iter()),
                                Fields::Unnamed(u) => fields_of(u.unnamed.iter()),
                                Fields::Unit => Vec::new(),
                            },
                        }
                    })
                    .collect();
                cards.push(Card {
                    name: e.ident.to_string(),
                    kind: "enum".to_string(),
                    path: module.to_string(),
                    doc: doc.summary.clone(),
                    invariants: doc.invariants.clone(),
                    generics: generic_names(&e.generics),
                    fields: Vec::new(),
                    variants,
                    transitions: doc.transitions.clone(),
                    ctor: None,
                    derives: derives(&e.attrs),
                    non_exhaustive: has_attr(&e.attrs, "non_exhaustive"),
                    src: format!("{}:{}", file, line_of(&e.ident)),
                });
            }
            Item::Type(t) => {
                let doc = parse_doc(&doc_lines(&t.attrs));
                cards.push(Card {
                    name: t.ident.to_string(),
                    kind: "alias".to_string(),
                    path: module.to_string(),
                    doc: doc.summary.clone(),
                    invariants: doc.invariants.clone(),
                    generics: generic_names(&t.generics),
                    fields: vec![Field {
                        name: "target".to_string(),
                        ty: ty_string(&t.ty),
                        doc: String::new(),
                        meaning: doc.summary.clone(),
                        unit: None,
                        range: None,
                        precision: None,
                        default: None,
                        nullable: false,
                        note: None,
                        missing: Vec::new(),
                    }],
                    variants: Vec::new(),
                    transitions: Vec::new(),
                    ctor: None,
                    derives: Vec::new(),
                    non_exhaustive: false,
                    src: format!("{}:{}", file, line_of(&t.ident)),
                });
            }
            Item::Impl(i) => {
                let Type::Path(tp) = &*i.self_ty else {
                    continue;
                };
                let Some(seg) = tp.path.segments.last() else {
                    continue;
                };
                let tyname = seg.ident.to_string();
                for it in &i.items {
                    if let ImplItem::Fn(f) = it {
                        let name = f.sig.ident.to_string();
                        if !matches!(name.as_str(), "try_new" | "new" | "create" | "from_parts") {
                            continue;
                        }
                        let sig = format!("({}) -> {}", param_names(&f.sig), return_type(&f.sig))
                            .replace("Self", &tyname);
                        let fdoc = parse_doc(&doc_lines(&f.attrs));
                        let entry = Ctor {
                            name,
                            sig,
                            checks: fdoc.errors.clone(),
                        };
                        ctors.entry(tyname.clone()).or_default().push(entry);
                    }
                }
            }
            _ => {}
        }
    }
}

fn fields_of<'a>(iter: impl Iterator<Item = &'a syn::Field>) -> Vec<Field> {
    let mut out = Vec::new();
    for (idx, f) in iter.enumerate() {
        let name = f
            .ident
            .as_ref()
            .map(|i| i.to_string())
            .unwrap_or_else(|| idx.to_string());
        let raw = doc_lines(&f.attrs).join("。");
        let parts = parse_field_doc(&raw);
        let mut missing = Vec::new();
        if !parts.has_unit {
            missing.push("unit".to_string());
        }
        if !parts.has_range {
            missing.push("range".to_string());
        }
        if !parts.has_default {
            missing.push("default".to_string());
        }
        out.push(Field {
            name,
            ty: ty_string(&f.ty),
            doc: raw,
            meaning: parts.meaning,
            unit: parts.unit,
            range: parts.range,
            precision: parts.precision,
            default: parts.default,
            nullable: parts.nullable,
            note: parts.note.clone(),
            missing,
        });
    }
    out
}

fn param_names(sig: &syn::Signature) -> String {
    let mut out = Vec::new();
    for input in &sig.inputs {
        match input {
            syn::FnArg::Receiver(_) => out.push("self".to_string()),
            syn::FnArg::Typed(pt) => {
                let name = match &*pt.pat {
                    syn::Pat::Ident(pi) => pi.ident.to_string(),
                    _ => "_".to_string(),
                };
                out.push(name);
            }
        }
    }
    out.join(", ")
}

fn return_type(sig: &syn::Signature) -> String {
    match &sig.output {
        syn::ReturnType::Default => "()".to_string(),
        syn::ReturnType::Type(_, ty) => ty_string(ty),
    }
}

fn generic_names(g: &syn::Generics) -> Vec<String> {
    g.params
        .iter()
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn doc_lines(attrs: &[Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for a in attrs {
        if !a.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &a.meta {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                out.push(s.value().trim().to_string());
            }
        }
    }
    out
}

fn derives(attrs: &[Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for a in attrs {
        if !a.path().is_ident("derive") {
            continue;
        }
        let Ok(list) = a.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
        else {
            continue;
        };
        for p in list {
            if let Some(seg) = p.segments.last() {
                out.push(seg.ident.to_string());
            }
        }
    }
    out
}

fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

fn line_of(ident: &syn::Ident) -> usize {
    ident.span().start().line
}

/// 规范化类型文本：收敛空格，去掉尖括号与逗号两侧的空白。
fn ty_string(ty: &Type) -> String {
    let raw = ty.to_token_stream().to_string();
    let mut t = raw;
    while t.contains("  ") {
        t = t.replace("  ", " ");
    }
    for (from, to) in [
        (" <", "<"),
        ("< ", "<"),
        (" >", ">"),
        (" ::", "::"),
        (":: ", "::"),
        (" ,", ","),
        ("& ", "&"),
    ] {
        t = t.replace(from, to);
    }
    t.trim().to_string()
}
