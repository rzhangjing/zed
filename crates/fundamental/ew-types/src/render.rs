//! 输出渲染：JSON（默认）与紧凑文本（备用）。

use crate::model::Card;

pub fn card_json(card: &Card) -> String {
    serde_json::to_string_pretty(card).unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
}

pub fn cards_json(cards: &[Card]) -> String {
    let entries: Vec<_> = cards.iter().map(|c| c.index_entry()).collect();
    let value = serde_json::json!({ "count": entries.len(), "types": entries });
    serde_json::to_string_pretty(&value).unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
}

/// 紧凑文本：给上下文紧张时用，每字段一行，用 ` | ` 分隔，不靠空格对齐（中文是宽字符）。
pub fn card_compact(card: &Card) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} · {} · {} · {}\n",
        card.name, card.kind, card.path, card.src
    ));
    if !card.doc.is_empty() {
        out.push_str(&format!("  {}\n", card.doc));
    }
    if !card.invariants.is_empty() {
        out.push_str(&format!("  inv: {}\n", card.invariants.join(" | ")));
    }
    if let Some(c) = &card.ctor {
        out.push_str(&format!("  ctor: {}{}\n", c.name, c.sig));
    }
    if !card.derives.is_empty() {
        out.push_str(&format!("  derives: {}\n", card.derives.join(", ")));
    }
    if !card.fields.is_empty() {
        out.push_str("  fields:\n");
        for f in &card.fields {
            out.push_str(&format!(
                "    {} | {} | {} | 单位:{} | 范围:{} | 默认值:{}{}{}\n",
                f.name,
                f.ty,
                if f.meaning.is_empty() { "-" } else { &f.meaning },
                opt(&f.unit),
                opt(&f.range),
                opt(&f.default),
                f.precision
                    .as_ref()
                    .map(|p| format!(" | 精度:{}", p))
                    .unwrap_or_default(),
                if f.nullable { " | 可为空" } else { "" },
            ));
        }
    }
    if !card.variants.is_empty() {
        out.push_str("  variants:\n");
        for v in &card.variants {
            let payload = if v.payload.is_empty() {
                String::new()
            } else {
                let names: Vec<String> = v
                    .payload
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.ty))
                    .collect();
                format!(" {{{}}}", names.join(", "))
            };
            out.push_str(&format!(
                "    {}{} | {}\n",
                v.name,
                payload,
                if v.doc.is_empty() { "-" } else { &v.doc }
            ));
        }
    }
    if !card.transitions.is_empty() {
        out.push_str("  transitions:\n");
        for t in &card.transitions {
            out.push_str(&format!("    {} -> {} | {}\n", t.from, t.to, t.via));
        }
    }
    out
}

fn opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("—")
}
