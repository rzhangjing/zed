//! ew-types —— 把 Rust 结构与枚举定义变成大模型可直接消费的类型卡。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ew_types::api::{self, Format};
use ew_types::mcp;

const USAGE: &str = "\
ew-types —— Rust 类型卡工具

用法:
  ew-types [--root DIR] [--format json|compact] <命令> [参数]

命令:
  ls [--module M]     列出类型索引
  get <NAME>          取单个类型卡（字段级定义）
  search <KEYWORD>    按名字 / 模块 / 摘要模糊搜索
  refs <TYPE>         反查谁引用了这个类型
  check [--json]      注释合规检查（CI 用，有错误时退出码 1）
  schema              打印卡片 JSON Schema
  mcp                 以 MCP 服务运行（stdio）

全局选项:
  --root DIR          扫描目录，默认向上查找最近的 Cargo.toml
  --format json|compact  输出格式，默认 json；compact 为省 token 的纯文本
  -h, --help          显示本帮助
";

fn main() -> ExitCode {
    let mut positional: Vec<String> = Vec::new();
    let mut root: Option<PathBuf> = None;
    let mut format = Format::Json;
    let mut json_out = false;
    let mut module: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                if let Some(v) = args.next() {
                    root = Some(PathBuf::from(v));
                }
            }
            "--format" => {
                if let Some(v) = args.next().and_then(|s| Format::parse(&s)) {
                    format = v;
                }
            }
            "--module" => module = args.next(),
            "--json" => json_out = true,
            "--compact" => format = Format::Compact,
            "-h" | "--help" => {
                print!("{}", USAGE);
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--root=") => {
                root = Some(PathBuf::from(other.trim_start_matches("--root=")));
            }
            other if other.starts_with("--format=") => {
                if let Some(v) = Format::parse(other.trim_start_matches("--format=")) {
                    format = v;
                }
            }
            other => positional.push(other.to_string()),
        }
    }

    let Some(cmd) = positional.first().cloned() else {
        print!("{}", USAGE);
        return ExitCode::SUCCESS;
    };

    let root = resolve_root(root);
    if !root.exists() {
        eprintln!("扫描目录不存在：{}", root.display());
        return ExitCode::from(2);
    }

    match cmd.as_str() {
        "ls" => {
            let r = api::load(&root);
            print!("{}", api::ls(&r.cards, module.as_deref(), format, &r.errors));
            ExitCode::SUCCESS
        }
        "get" => {
            let Some(name) = positional.get(1) else {
                eprintln!("用法：ew-types get <NAME>");
                return ExitCode::from(2);
            };
            let r = api::load(&root);
            match api::get(&r.cards, name, format) {
                Ok(text) => {
                    println!("{}", text);
                    ExitCode::SUCCESS
                }
                Err(msg) => {
                    eprintln!("{}", msg);
                    ExitCode::from(2)
                }
            }
        }
        "search" => {
            let Some(kw) = positional.get(1) else {
                eprintln!("用法：ew-types search <KEYWORD>");
                return ExitCode::from(2);
            };
            let r = api::load(&root);
            println!("{}", api::search(&r.cards, kw));
            ExitCode::SUCCESS
        }
        "refs" => {
            let Some(ty) = positional.get(1) else {
                eprintln!("用法：ew-types refs <TYPE>");
                return ExitCode::from(2);
            };
            let r = api::load(&root);
            println!("{}", api::refs(&r.cards, ty));
            ExitCode::SUCCESS
        }
        "check" => {
            let r = api::load(&root);
            let (text, errors) = api::check_cmd(&r.cards, &r.errors, json_out);
            print!("{}", text);
            if errors > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        "schema" => {
            print!("{}", include_str!("../schema/card.schema.json"));
            ExitCode::SUCCESS
        }
        "mcp" => match mcp::serve(root) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("MCP 服务退出：{}", e);
                ExitCode::from(1)
            }
        },
        other => {
            eprintln!("未知命令：{}\n", other);
            print!("{}", USAGE);
            ExitCode::from(2)
        }
    }
}

/// 优先用显式 `--root`，否则从当前目录向上找最近的 `Cargo.toml`。
fn resolve_root(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut dir: &Path = &cwd;
    loop {
        if dir.join("Cargo.toml").exists() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    cwd
}
