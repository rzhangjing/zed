//! 最小 MCP 服务端：stdio + JSON-RPC 2.0，只实现 tools 能力。
//!
//! 每次调用重新扫描，保证模型看到的永远是当前源码。

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::api::{self, Format};

pub fn serve(root: PathBuf) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(resp) = handle(&req, &root) {
            writeln!(stdout, "{}", resp)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle(req: &Value, root: &Path) -> Option<Value> {
    let method = req.get("method")?.as_str()?;
    let id = req.get("id").cloned();

    if matches!(method, "notifications/initialized" | "initialized") {
        return None;
    }

    let outcome: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "ew-types", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call_tool(req, root),
        other => Err((-32601, format!("未知方法：{}", other))),
    };

    let id = id?; // 通知不需要回复
    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }),
    })
}

fn call_tool(req: &Value, root: &Path) -> Result<Value, (i64, String)> {
    let params = req.get("params").cloned().unwrap_or(json!({}));
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "缺少 tools/call.params.name".to_string()))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let extracted = api::load(root);
    let cards = &extracted.cards;
    let fmt = args
        .get("format")
        .and_then(|v| v.as_str())
        .and_then(Format::parse)
        .unwrap_or(Format::Json);

    let text = match name {
        "ls" => {
            let module = args.get("module").and_then(|v| v.as_str());
            api::ls(cards, module, fmt, &extracted.errors)
        }
        "get" => {
            let n = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "get 需要 name".to_string()))?;
            match api::get(cards, n, fmt) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(text_result(&e, true));
                }
            }
        }
        "search" => {
            let kw = args
                .get("keyword")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "search 需要 keyword".to_string()))?;
            api::search(cards, kw)
        }
        "refs" => {
            let ty = args
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "refs 需要 type".to_string()))?;
            api::refs(cards, ty)
        }
        "check" => api::check_cmd(cards, &extracted.errors, true).0,
        other => return Err((-32602, format!("未知工具：{}", other))),
    };

    Ok(text_result(&text, false))
}

fn text_result(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "ls",
            "description": "列出可用的类型索引（名字、类别、模块、一句话摘要）。不确定有什么类型时先调这个。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "module": { "type": "string", "description": "只列某个模块，如 domain::order" },
                    "format": { "type": "string", "enum": ["json", "compact"], "default": "json" }
                }
            }
        },
        {
            "name": "get",
            "description": "取单个类型的字段级定义：字段含义、单位、范围、默认值、不变量、构造入口、枚举变体与流转。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "类型名，如 Order；重名时用 domain::order::Order" },
                    "format": { "type": "string", "enum": ["json", "compact"], "default": "json" }
                },
                "required": ["name"]
            }
        },
        {
            "name": "search",
            "description": "按名字、模块或中文摘要模糊搜索类型。",
            "inputSchema": {
                "type": "object",
                "properties": { "keyword": { "type": "string" } },
                "required": ["keyword"]
            }
        },
        {
            "name": "refs",
            "description": "反查：哪些类型的字段或构造器用到了这个类型。改字段前先查影响面。",
            "inputSchema": {
                "type": "object",
                "properties": { "type": { "type": "string" } },
                "required": ["type"]
            }
        },
        {
            "name": "check",
            "description": "检查注释是否合规：缺单位、缺范围、缺默认值、变体缺注释等，返回违规清单。",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}
