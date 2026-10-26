//! 注释合规检查。把定义规范变成可执行的闸门。

use crate::model::Card;

#[derive(Debug, Clone)]
pub struct Violation {
    pub level: &'static str,
    pub rule: &'static str,
    pub target: String,
    pub src: String,
    pub message: String,
}

pub struct Report {
    pub violations: Vec<Violation>,
}

impl Report {
    pub fn errors(&self) -> usize {
        self.violations.iter().filter(|v| v.level == "error").count()
    }
    pub fn warns(&self) -> usize {
        self.violations.iter().filter(|v| v.level == "warn").count()
    }
}

pub fn check(cards: &[Card]) -> Report {
    let mut violations = Vec::new();

    for card in cards {
        if card.doc.is_empty() {
            violations.push(Violation {
                level: "error",
                rule: "TY-01",
                target: card.name.clone(),
                src: card.src.clone(),
                message: "类型缺少文档注释".to_string(),
            });
        }
        if card.doc.contains("TODO") || card.doc.contains("待定") || card.doc.contains("暂时") {
            violations.push(Violation {
                level: "error",
                rule: "TY-02",
                target: card.name.clone(),
                src: card.src.clone(),
                message: "类型注释含 TODO / 待定 / 暂时".to_string(),
            });
        }

        // 缺字段注释只报一次，不叠加三个段缺失；newtype 的内层字段由类型注释说明
        for f in &card.fields {
            if card.kind == "newtype" {
                break;
            }
            let target = format!("{}.{}", card.name, f.name);
            if f.doc.trim().is_empty() {
                violations.push(Violation {
                    level: "error",
                    rule: "FD-01",
                    target,
                    src: card.src.clone(),
                    message: "字段缺少文档注释".to_string(),
                });
                continue;
            }
            for seg in &f.missing {
                let (rule, name) = match seg.as_str() {
                    "unit" => ("FD-02", "单位"),
                    "range" => ("FD-03", "范围"),
                    _ => ("FD-04", "默认值"),
                };
                violations.push(Violation {
                    level: "error",
                    rule,
                    target: target.clone(),
                    src: card.src.clone(),
                    message: format!("字段注释缺「{}」段", name),
                });
            }
        }

        for v in &card.variants {
            if v.doc.trim().is_empty() {
                violations.push(Violation {
                    level: "error",
                    rule: "EN-01",
                    target: format!("{}::{}", card.name, v.name),
                    src: card.src.clone(),
                    message: "枚举变体缺少注释（应写何时进入）".to_string(),
                });
            }
            if v.name == "Other" || v.name == "Unknown" {
                violations.push(Violation {
                    level: "warn",
                    rule: "EN-02",
                    target: format!("{}::{}", card.name, v.name),
                    src: card.src.clone(),
                    message: "存在兜底变体，会导致未知状态被吞掉".to_string(),
                });
            }
        }
    }

    Report { violations }
}

pub fn report_json(report: &Report, parse_errors: &[String]) -> String {
    let items: Vec<_> = report
        .violations
        .iter()
        .map(|v| {
            serde_json::json!({
                "level": v.level,
                "rule": v.rule,
                "target": v.target,
                "src": v.src,
                "message": v.message,
            })
        })
        .collect();
    let value = serde_json::json!({
        "summary": {
            "error": report.errors(),
            "warn": report.warns(),
            "parseErrors": parse_errors.len(),
        },
        "violations": items,
        "parseErrors": parse_errors,
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

pub fn report_text(report: &Report, parse_errors: &[String]) -> String {
    let mut out = String::new();
    for e in parse_errors {
        out.push_str(&format!("[parse] {}\n", e));
    }
    for v in &report.violations {
        out.push_str(&format!(
            "[{}] {} {} {} — {}\n",
            v.level, v.rule, v.target, v.src, v.message
        ));
    }
    out.push_str(&format!(
        "\n合计 {} 个错误，{} 个警告，{} 个解析失败\n",
        report.errors(),
        report.warns(),
        parse_errors.len()
    ));
    out
}
