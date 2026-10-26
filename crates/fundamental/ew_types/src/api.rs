//! 命令行与 MCP 共用的业务逻辑。

use std::path::Path;

use crate::check::{check, report_json, report_text};
use crate::extract::{extract_all, ExtractResult};
use crate::model::Card;
use crate::render::{card_compact, card_json, cards_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Compact,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "json" => Some(Format::Json),
            "compact" => Some(Format::Compact),
            _ => None,
        }
    }
}

pub fn load(root: &Path) -> ExtractResult {
    extract_all(root)
}

/// 列类型索引。
pub fn ls(cards: &[Card], module: Option<&str>, fmt: Format, parse_errors: &[String]) -> String {
    let filtered: Vec<Card> = cards
        .iter()
        .filter(|c| match module {
            Some(m) => c.path == m || c.path.starts_with(&format!("{}::", m)),
            None => true,
        })
        .cloned()
        .collect();

    match fmt {
        Format::Json => {
            if parse_errors.is_empty() {
                cards_json(&filtered)
            } else {
                let mut v: serde_json::Value =
                    serde_json::from_str(&cards_json(&filtered)).unwrap_or_default();
                v["parseErrors"] = serde_json::json!(parse_errors);
                serde_json::to_string_pretty(&v).unwrap_or_default()
            }
        }
        Format::Compact => {
            let mut out = String::new();
            for c in &filtered {
                out.push_str(&format!(
                    "{} | {} | {} | {} | {}\n",
                    c.name, c.kind, c.path, c.doc, c.src
                ));
            }
            out.push_str(&format!("共 {} 个类型\n", filtered.len()));
            out
        }
    }
}

/// 取单个类型卡。名字不唯一时返回候选清单，不猜。
pub fn get(cards: &[Card], name: &str, fmt: Format) -> Result<String, String> {
    let exact: Vec<&Card> = cards.iter().filter(|c| c.name == name).collect();
    let chosen: Vec<&Card> = if exact.is_empty() {
        // 支持 `domain::order::Order` 或大小写不敏感匹配
        let lower = name.to_lowercase();
        let by_path: Vec<&Card> = cards
            .iter()
            .filter(|c| {
                let full = format!("{}::{}", c.path, c.name).to_lowercase();
                full == lower || c.name.to_lowercase() == lower
            })
            .collect();
        if by_path.is_empty() {
            let contains: Vec<&Card> = cards
                .iter()
                .filter(|c| c.name.to_lowercase().contains(&lower))
                .collect();
            contains
        } else {
            by_path
        }
    } else {
        exact
    };

    match chosen.len() {
        0 => Err(format!("未找到类型 `{}`，用 `ew-types ls` 查看全部", name)),
        1 => Ok(match fmt {
            Format::Json => card_json(chosen[0]),
            Format::Compact => card_compact(chosen[0]),
        }),
        _ => {
            let list: Vec<String> = chosen
                .iter()
                .map(|c| format!("  {}::{}", c.path, c.name))
                .collect();
            Err(format!(
                "`{}` 匹配到多个类型，请用全名：\n{}",
                name,
                list.join("\n")
            ))
        }
    }
}

/// 按名字、模块、摘要模糊搜索。
pub fn search(cards: &[Card], keyword: &str) -> String {
    let kw = keyword.to_lowercase();
    let hits: Vec<serde_json::Value> = cards
        .iter()
        .filter(|c| {
            c.name.to_lowercase().contains(&kw)
                || c.path.to_lowercase().contains(&kw)
                || c.doc.to_lowercase().contains(&kw)
        })
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "kind": c.kind,
                "path": c.path,
                "doc": c.doc,
                "src": c.src,
            })
        })
        .collect();
    let value = serde_json::json!({ "query": keyword, "count": hits.len(), "matches": hits });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// 反查：谁在字段或构造器里用到了这个类型。
pub fn refs(cards: &[Card], ty: &str) -> String {
    let mut hits: Vec<serde_json::Value> = Vec::new();
    for c in cards {
        if c.name == ty {
            continue;
        }
        for f in &c.fields {
            if contains_type(&f.ty, ty) {
                hits.push(serde_json::json!({
                    "from": c.name, "path": c.path, "field": f.name, "type": f.ty, "src": c.src
                }));
            }
        }
        for v in &c.variants {
            for f in &v.payload {
                if contains_type(&f.ty, ty) {
                    hits.push(serde_json::json!({
                        "from": format!("{}::{}", c.name, v.name),
                        "path": c.path,
                        "field": f.name,
                        "type": f.ty,
                        "src": c.src
                    }));
                }
            }
        }
        if let Some(ctor) = &c.ctor {
            if contains_type(&ctor.sig, ty) {
                hits.push(serde_json::json!({
                    "from": c.name, "path": c.path, "field": format!("fn {}", ctor.name),
                    "type": ctor.sig, "src": c.src
                }));
            }
        }
    }
    let value = serde_json::json!({
        "type": ty,
        "count": hits.len(),
        "referencedBy": hits,
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

fn contains_type(haystack: &str, needle: &str) -> bool {
    let mut cur = String::new();
    for ch in haystack.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else {
            if cur == needle {
                return true;
            }
            cur.clear();
        }
    }
    cur == needle
}

/// 注释合规检查。返回（输出文本，错误数）。
pub fn check_cmd(cards: &[Card], parse_errors: &[String], json: bool) -> (String, usize) {
    let report = check(cards);
    let errors = report.errors() + parse_errors.len();
    let text = if json {
        report_json(&report, parse_errors)
    } else {
        report_text(&report, parse_errors)
    };
    (text, errors)
}
