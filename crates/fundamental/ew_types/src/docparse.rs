//! 注释解析：把 `///` 文本按规范句式拆成结构化字段。
//!
//! 规范要求字段注释固定五段：
//! `{含义}。单位：{单位}。范围：{范围}。默认值：{默认值}。{精度}。{可空说明}。`
//!
//! 解析不出来就是"未声明"，用 `Option::None` 表达，绝不猜。

use crate::model::Transition;

/// 类型级注释的解析结果。
#[derive(Debug, Default, Clone)]
pub struct Doc {
    /// 首行摘要。
    pub summary: String,
    /// 摘要之后、第一个 `#` 段之前的正文行。
    pub body: Vec<String>,
    /// `# 不变量` 段。
    pub invariants: Vec<String>,
    /// `# 流转` 段的原始行。
    pub transition_lines: Vec<String>,
    /// 状态流转解析结果。
    pub transitions: Vec<Transition>,
    /// `# 示例` 段的代码。
    pub example: Option<String>,
    /// `# 错误` 段。
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Invariants,
    Transitions,
    Example,
    Errors,
}

/// 解析一段 `///` 文本。
pub fn parse_doc(lines: &[String]) -> Doc {
    let mut doc = Doc::default();
    let mut section = Section::None;
    let mut summary_done = false;
    let mut example_buf: Vec<String> = Vec::new();

    for raw in lines {
        let line = raw.trim_end().to_string();
        let trimmed = line.trim();

        // 段标题：以 "# " 开头且不是代码块
        if let Some(title) = trimmed.strip_prefix("# ") {
            match title.trim() {
                "不变量" => {
                    section = Section::Invariants;
                    continue;
                }
                "流转" => {
                    section = Section::Transitions;
                    continue;
                }
                "示例" => {
                    section = Section::Example;
                    continue;
                }
                t if t.starts_with("错误") || t.starts_with("Errors") => {
                    section = Section::Errors;
                    continue;
                }
                _ => {
                    // 其他标准段（Panics / Safety）原样忽略，不进正文
                    section = Section::None;
                    continue;
                }
            }
        }

        match section {
            Section::Invariants => {
                if let Some(item) = strip_bullet(trimmed) {
                    if !item.is_empty() {
                        doc.invariants.push(item.to_string());
                    }
                }
            }
            Section::Transitions => {
                doc.transition_lines.push(trimmed.to_string());
            }
            Section::Example => {
                if trimmed.starts_with("```") {
                    continue;
                }
                example_buf.push(line.clone());
            }
            Section::Errors => {
                if let Some(item) = strip_bullet(trimmed) {
                    if !item.is_empty() {
                        doc.errors.push(item.to_string());
                    }
                } else if !trimmed.is_empty() {
                    doc.errors.push(trimmed.to_string());
                }
            }
            Section::None => {
                if trimmed.is_empty() {
                    if !summary_done && !doc.summary.is_empty() {
                        summary_done = true;
                    }
                    continue;
                }
                if !summary_done && doc.summary.is_empty() {
                    doc.summary = trimmed.to_string();
                } else {
                    summary_done = true;
                    doc.body.push(trimmed.to_string());
                }
            }
        }
    }

    let example = example_buf.join("\n").trim().to_string();
    if !example.is_empty() {
        doc.example = Some(example);
    }
    doc.transitions = parse_transitions(&doc.transition_lines);
    doc
}

fn strip_bullet(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")) {
        return Some(rest.trim());
    }
    // 有序列表 "1. xxx"
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && s[i..].starts_with(". ") {
        return Some(s[i + 2..].trim());
    }
    None
}

/// 支持两种写法：
/// - `- Pending -> Paid：支付成功`（推荐，分支随意写）
/// - `Pending ──支付成功──▶ Paid ──发货──▶ Shipped`（仅直链）
pub fn parse_transitions(lines: &[String]) -> Vec<Transition> {
    let mut out = Vec::new();
    for line in lines {
        let l = line.trim();
        if l.is_empty() || l.starts_with("```") {
            continue;
        }
        if l.contains("->") {
            if let Some(t) = parse_arrow_line(l) {
                out.push(t);
            }
            continue;
        }
        if l.contains('▶') {
            out.extend(parse_chain_line(l));
        }
    }
    out
}

fn parse_arrow_line(line: &str) -> Option<Transition> {
    let l = strip_bullet(line).unwrap_or(line).trim();
    let (left, right) = l.split_once("->")?;
    let from = left.trim().to_string();
    let right = right.trim();
    // 触发条件用全角或半角冒号分隔
    let (to, via) = match right.split_once('：').or_else(|| right.split_once(':')) {
        Some((t, v)) => (t.trim().to_string(), v.trim().to_string()),
        None => (right.to_string(), String::new()),
    };
    if from.is_empty() || to.is_empty() {
        return None;
    }
    Some(Transition { from, to, via })
}

fn parse_chain_line(line: &str) -> Vec<Transition> {
    let mut out = Vec::new();
    let parts: Vec<&str> = line.split('▶').collect();
    for i in 0..parts.len().saturating_sub(1) {
        let left = parts[i];
        let right = parts[i + 1];
        let lsegs: Vec<&str> = left.split("──").collect();
        let from = lsegs.first().map(|s| s.trim()).unwrap_or("");
        let via = lsegs
            .get(1)
            .map(|s| s.trim_matches(|c| c == '─' || c == ' ').to_string())
            .unwrap_or_default();
        let to = right.split("──").next().unwrap_or("").trim();
        if !from.is_empty() && !to.is_empty() {
            out.push(Transition {
                from: from.to_string(),
                to: to.to_string(),
                via,
            });
        }
    }
    out
}

/// 字段注释解析结果。
#[derive(Debug, Default, Clone)]
pub struct FieldParts {
    pub meaning: String,
    pub unit: Option<String>,
    pub range: Option<String>,
    pub precision: Option<String>,
    pub default: Option<String>,
    pub nullable: bool,
    /// 五段之外的自由说明，如"保持下单顺序"。
    pub note: Option<String>,
    /// 三个必写段是否出现过（用于合规检查）。
    pub has_unit: bool,
    pub has_range: bool,
    pub has_default: bool,
}

/// 解析字段注释。
///
/// 分段规则：遇到第一个关键字段（单位 / 范围 / 默认值 / 精度）之前的内容都算"含义"，
/// 之后的非关键内容归到 note，避免把"非空""两位小数"混进含义。
pub fn parse_field_doc(text: &str) -> FieldParts {
    let mut parts = FieldParts::default();
    let mut meaning: Vec<String> = Vec::new();
    let mut note: Vec<String> = Vec::new();
    let mut tail = false;

    for seg in text.split('。') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        if let Some(v) = strip_key(seg, &["单位：", "单位:", "单位"]) {
            parts.has_unit = true;
            parts.unit = non_empty(v);
            tail = true;
            continue;
        }
        if let Some(v) = strip_key(seg, &["范围：", "范围:"]) {
            parts.has_range = true;
            parts.range = non_empty(v);
            tail = true;
            continue;
        }
        if let Some(v) = strip_key(seg, &["默认值：", "默认值:", "默认值"]) {
            parts.has_default = true;
            parts.default = non_empty(v);
            tail = true;
            continue;
        }
        if let Some(v) = strip_key(seg, &["精度：", "精度:"]) {
            parts.precision = non_empty(v);
            tail = true;
            continue;
        }

        if !tail {
            meaning.push(seg.to_string());
            continue;
        }

        if seg.contains("可为空") || seg.contains("为空表示") {
            parts.nullable = true;
            note.push(seg.to_string());
        } else if seg == "非空" {
            // 已由 nullable=false 表达，不重复记
        } else if seg.contains("小数") {
            if parts.precision.is_none() {
                parts.precision = Some(seg.to_string());
            }
        } else {
            note.push(seg.to_string());
        }
    }

    parts.meaning = meaning.join("。");
    if !note.is_empty() {
        parts.note = Some(note.join("。"));
    }
    parts
}

fn strip_key<'a>(seg: &'a str, keys: &[&str]) -> Option<&'a str> {
    for k in keys {
        if let Some(rest) = seg.strip_prefix(k) {
            return Some(rest.trim());
        }
    }
    None
}

/// `—` 表示"已声明但无值"，与"未声明"（None）区分；`无` 作为字面值保留。
fn non_empty(v: &str) -> Option<String> {
    let v = v.trim().trim_end_matches('。').trim();
    if v.is_empty() || v == "—" || v == "-" {
        None
    } else {
        Some(v.to_string())
    }
}
