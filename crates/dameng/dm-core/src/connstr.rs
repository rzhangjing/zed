//! # 连接串的渲染与解析
//!
//! 支持两种入口，见设计方案 §6.1：
//!
//! 1. **ODBC 连接串**：`KEY=VALUE;KEY=VALUE`，这是达梦 ODBC 驱动真正吃的东西。
//!    `DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=…`
//! 2. **我们自己的 URL 形式**：`dm://user:pass@host:port/schema?k=v`。
//!    这是本项目定义的**便利语法**，不是任何上游 crate 的语法。
//!
//! ## 往返约定
//!
//! [`render_odbc_string`] 只渲染「ODBC 连接串本身能表达」的字段；`charset_check`、
//! `length_in_char`、`init_sql`、`pool` 属于中间件自身的行为开关，不进连接串。
//! 因此往返恒等式 `parse(render(c)) == c` 成立于：这些字段保持默认值的配置。
//! 测试里的 `carried_config()` 正是这个子集。
//!
//! ## `extra` 的优先级
//!
//! [`DmConfig::extra`] 是逃生舱：渲染时**只要 `extra` 里出现了某个键，结构化字段就跳过**
//! 该键，由 `extra` 原样输出。这样用户总能用一个未识别的原始键值覆盖我们的映射，
//! 例如把 `RW_SEPARATE=ON2`（结构化字段是 `Option<bool>`，表达不了 `ON2`）原样透传。

use std::time::Duration;

use dm_error::{DmError, DmErrorKind, Stage};

use crate::config::{CompatMode, DmConfig, SecretString};

/// 渲染连接串时使用的结构化键（顺序即输出顺序）。
pub const STRUCTURED_KEYS: &[&str] = &[
    "DRIVER",
    "SERVER",
    "TCP_PORT",
    "UID",
    "PWD",
    "SCHEMA",
    "CHARACTER_CODE",
    "COMPATIBLE_MODE",
    "RW_SEPARATE",
    "CONNECTION_TIMEOUT",
    "LOGIN_TIMEOUT",
];

/// 解析时会被映射进结构化字段的**同义键**（同一份配置的另一种写法）。
///
/// 这些键不出现在渲染结果里：解析成结构化字段后，渲染统一用 [`STRUCTURED_KEYS`] 的名字。
pub const ALIAS_KEYS: &[&str] = &["SERVERNAME", "HOST", "PORT", "USER", "USERNAME", "PASSWORD"];

/// 结构化键 + 同义键的并集。
///
/// 这两个集合里的键**不允许**出现在 [`DmConfig::extra`] 中：否则渲染时 `extra` 会覆盖
/// 结构化字段，解析回来却落进结构化字段，往返就断了。
pub const RESERVED_KEYS: &[&str] = &[
    "DRIVER",
    "SERVER",
    "TCP_PORT",
    "UID",
    "PWD",
    "SCHEMA",
    "CHARACTER_CODE",
    "COMPATIBLE_MODE",
    "RW_SEPARATE",
    "CONNECTION_TIMEOUT",
    "LOGIN_TIMEOUT",
    "SERVERNAME",
    "HOST",
    "PORT",
    "USER",
    "USERNAME",
    "PASSWORD",
];

fn cs_err(msg: impl Into<String>) -> DmError {
    DmError::at(DmErrorKind::Config(msg.into()), Stage::Config)
}

// ---------------------------------------------------------------------------
// ODBC 连接串 —— 渲染
// ---------------------------------------------------------------------------

/// ODBC 的转义规则：值里含有 `;`、`{`、`}` 或首尾空白时，必须用 `{…}` 包起来，
/// 且内部的 `}` 要写成 `}}`。
fn needs_brace(v: &str) -> bool {
    v.contains(';')
        || v.contains('{')
        || v.contains('}')
        || v.starts_with(char::is_whitespace)
        || v.ends_with(char::is_whitespace)
        || v.contains('\n')
        || v.contains('\r')
}

/// 追加一对 `KEY=VALUE;`。`force_brace` 用于驱动名这类习惯上总带花括号的键。
fn push_pair(out: &mut String, key: &str, value: &str, force_brace: bool) {
    out.push_str(key);
    out.push('=');
    if force_brace || needs_brace(value) {
        out.push('{');
        for c in value.chars() {
            if c == '}' {
                out.push('}');
            }
            out.push(c);
        }
        out.push('}');
    } else {
        out.push_str(value);
    }
    out.push(';');
}

/// 把配置渲染成达梦 ODBC 能直接使用的连接串。
///
/// 输出总是以 `;` 结尾；`extra` 中的键值对排在最后（因此同名时优先生效，
/// 不过 [`RESERVED_KEYS`] 里的键根本不会被结构化部分重复输出）。
pub fn render_odbc_string(cfg: &DmConfig) -> String {
    let skip = |k: &str| cfg.extra.contains_key(k);
    let mut out = String::with_capacity(160);

    if !skip("DRIVER") {
        push_pair(&mut out, "DRIVER", &cfg.driver, true);
    }
    if !skip("SERVER") {
        push_pair(&mut out, "SERVER", &cfg.server, false);
    }
    if !skip("TCP_PORT") {
        push_pair(&mut out, "TCP_PORT", &cfg.port.to_string(), false);
    }
    if !skip("UID") {
        push_pair(&mut out, "UID", &cfg.user, false);
    }
    if !skip("PWD") {
        push_pair(&mut out, "PWD", cfg.password.expose(), false);
    }
    if let Some(v) = &cfg.schema
        && !skip("SCHEMA")
    {
        // 仍然照常写进去：达梦驱动会忽略它，但其它 ODBC 网关（以及未来版本）可能认；
        // 真正生效靠 `dm_odbc` 建连时的 `SET SCHEMA`（见 `DmConfig::schema` 的文档）。
        push_pair(&mut out, "SCHEMA", v, false);
    }
    if let Some(v) = &cfg.character_code
        && !skip("CHARACTER_CODE")
    {
        push_pair(&mut out, "CHARACTER_CODE", v, false);
    }
    if let Some(v) = cfg.compatible_mode
        && !skip("COMPATIBLE_MODE")
    {
        push_pair(&mut out, "COMPATIBLE_MODE", v.as_str(), false);
    }
    if let Some(v) = cfg.rw_separate
        && !skip("RW_SEPARATE")
    {
        push_pair(&mut out, "RW_SEPARATE", if v { "ON" } else { "OFF" }, false);
    }
    if !skip("CONNECTION_TIMEOUT") {
        push_pair(
            &mut out,
            "CONNECTION_TIMEOUT",
            &cfg.connection_timeout.as_secs().to_string(),
            false,
        );
    }
    if !skip("LOGIN_TIMEOUT") {
        push_pair(
            &mut out,
            "LOGIN_TIMEOUT",
            &cfg.login_timeout.as_secs().to_string(),
            false,
        );
    }

    for (k, v) in &cfg.extra {
        push_pair(&mut out, k, v, false);
    }

    out
}

// ---------------------------------------------------------------------------
// ODBC 连接串 —— 解析
// ---------------------------------------------------------------------------

/// 把连接串切成 `(大写键, 值)` 序列。值两侧的花括号会被剥掉，`}}` 还原成 `}`。
fn parse_pairs(s: &str) -> Result<Vec<(String, String)>, DmError> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();

    loop {
        // 跳过分隔符与空白
        while let Some(&(_, c)) = it.peek() {
            if c == ';' || c.is_whitespace() {
                it.next();
            } else {
                break;
            }
        }
        let Some(&(start, _)) = it.peek() else { break };

        // 读键，直到 '='
        let mut key_end = None;
        for (i, c) in it.by_ref() {
            if c == '=' {
                key_end = Some(i);
                break;
            }
            if c == ';' {
                break;
            }
        }
        let Some(ke) = key_end else {
            return Err(cs_err(format!(
                "连接串片段 `{}` 缺少 '='",
                &s[start..s.len().min(start + 24)]
            )));
        };
        let key = s[start..ke].trim().to_ascii_uppercase();
        if key.is_empty() {
            return Err(cs_err("连接串中出现空键名"));
        }

        // 读值
        let mut value = String::new();
        if let Some(&(_, '{')) = it.peek() {
            it.next(); // 吃掉 '{'
            let mut closed = false;
            while let Some((_, c)) = it.next() {
                if c == '}' {
                    if let Some(&(_, '}')) = it.peek() {
                        it.next();
                        value.push('}');
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    value.push(c);
                }
            }
            if !closed {
                return Err(cs_err(format!("键 {key} 的花括号没有闭合")));
            }
        } else {
            for (_, c) in it.by_ref() {
                if c == ';' {
                    break;
                }
                value.push(c);
            }
            value = value.trim().to_string();
        }

        out.push((key, value));
    }

    Ok(out)
}

/// `RW_SEPARATE` 的可表达取值。`ON2`（只连事务一致性备库）返回 `None`，
/// 由调用方保留进 `extra` 原样透传。
fn parse_rw_separate(v: &str) -> Option<bool> {
    match v.trim().to_ascii_uppercase().as_str() {
        "ON" | "1" | "TRUE" | "YES" => Some(true),
        "OFF" | "0" | "FALSE" | "NO" => Some(false),
        _ => None,
    }
}

/// 把一个键值对应用到配置上。
fn apply_pair(cfg: &mut DmConfig, key: &str, value: &str) -> Result<(), DmError> {
    // 解析不进结构化字段的取值，一律留在 extra 里原样透传（而不是丢弃）。
    let keep_raw = |cfg: &mut DmConfig| {
        cfg.extra.insert(key.to_string(), value.to_string());
    };

    match key {
        "DRIVER" => cfg.driver = value.to_string(),
        "SERVER" | "SERVERNAME" | "HOST" => cfg.server = value.to_string(),
        "TCP_PORT" | "PORT" => {
            cfg.port = value
                .trim()
                .parse::<u16>()
                .map_err(|_| cs_err(format!("端口 `{value}` 不是合法的 16 位端口号")))?;
        }
        "UID" | "USER" | "USERNAME" => cfg.user = value.to_string(),
        "PWD" | "PASSWORD" => cfg.password = SecretString::new(value),
        "SCHEMA" => cfg.schema = Some(value.to_string()),
        "CHARACTER_CODE" => cfg.character_code = Some(value.to_string()),
        "COMPATIBLE_MODE" => match CompatMode::parse(value) {
            Some(m) => cfg.compatible_mode = Some(m),
            None => keep_raw(cfg),
        },
        "RW_SEPARATE" => match parse_rw_separate(value) {
            Some(b) => cfg.rw_separate = Some(b),
            None => keep_raw(cfg),
        },
        "CONNECTION_TIMEOUT" => match value.trim().parse::<u64>() {
            Ok(n) => cfg.connection_timeout = Duration::from_secs(n),
            Err(_) => keep_raw(cfg),
        },
        "LOGIN_TIMEOUT" => match value.trim().parse::<u64>() {
            Ok(n) => cfg.login_timeout = Duration::from_secs(n),
            Err(_) => keep_raw(cfg),
        },
        _ => keep_raw(cfg),
    }
    Ok(())
}

/// 解析 ODBC 连接串。
///
/// 只做**结构**校验（花括号闭合、端口是不是数字）；「用户名非空」「`max_size` 不为 0」
/// 这类**语义**校验在 [`DmConfig::validate`] 里。未识别的键保留进 [`DmConfig::extra`]。
pub fn parse_odbc_string(s: &str) -> Result<DmConfig, DmError> {
    let mut cfg = DmConfig::default();
    for (k, v) in parse_pairs(s)? {
        apply_pair(&mut cfg, &k, &v)?;
    }
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// 百分号编码（URL 形式用）
// ---------------------------------------------------------------------------

/// 编码一个 URL 组件：只保留 `A-Za-z0-9-._~`，其余按 UTF-8 字节转义。
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 解码一个 URL 组件。
fn decode_component(s: &str) -> Result<String, DmError> {
    let bytes_in = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes_in.len());
    let mut i = 0;
    while i < bytes_in.len() {
        if bytes_in[i] == b'%' {
            if i + 2 >= bytes_in.len() {
                return Err(cs_err(format!("URL 组件 `{s}` 里的 `%` 转义不完整")));
            }
            let hex = std::str::from_utf8(&bytes_in[i + 1..i + 3])
                .map_err(|_| cs_err(format!("URL 组件 `{s}` 含非法转义")))?;
            let b = u8::from_str_radix(hex, 16)
                .map_err(|_| cs_err(format!("URL 组件 `{s}` 含非法十六进制转义 `%{hex}`")))?;
            out.push(b);
            i += 3;
        } else {
            out.push(bytes_in[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| cs_err(format!("URL 组件 `{s}` 解码后不是合法 UTF-8")))
}

// ---------------------------------------------------------------------------
// URL 形式
// ---------------------------------------------------------------------------

/// `dm://` URL 里被识别为结构化字段的查询参数键（其余进 [`DmConfig::extra`]）。
pub const URL_PARAM_KEYS: &[&str] = &[
    "driver",
    "character_code",
    "compatible_mode",
    "rw_separate",
    "connection_timeout",
    "login_timeout",
];

/// 渲染成 `dm://user:pass@host:port/schema?k=v` 形式。
///
/// 这是本项目自定义的便利语法（**不是**任何上游 crate 的语法）。
/// 所有组件都做百分号编码，因此口令里含 `@`、`:`、`/` 也不会破坏语法。
pub fn render_dm_url(cfg: &DmConfig) -> String {
    let mut out = String::with_capacity(160);
    out.push_str("dm://");
    out.push_str(&encode_component(&cfg.user));
    out.push(':');
    out.push_str(&encode_component(cfg.password.expose()));
    out.push('@');
    out.push_str(&encode_component(&cfg.server));
    out.push(':');
    out.push_str(&cfg.port.to_string());
    out.push('/');
    if let Some(v) = &cfg.schema {
        out.push_str(&encode_component(v));
    }

    let mut params: Vec<(String, String)> = vec![("driver".into(), cfg.driver.clone())];
    if let Some(v) = &cfg.character_code {
        params.push(("character_code".into(), v.clone()));
    }
    if let Some(v) = cfg.compatible_mode {
        params.push(("compatible_mode".into(), v.as_str().to_string()));
    }
    if let Some(v) = cfg.rw_separate {
        params.push((
            "rw_separate".into(),
            if v { "on" } else { "off" }.to_string(),
        ));
    }
    params.push((
        "connection_timeout".into(),
        cfg.connection_timeout.as_secs().to_string(),
    ));
    params.push((
        "login_timeout".into(),
        cfg.login_timeout.as_secs().to_string(),
    ));
    for (k, v) in &cfg.extra {
        params.push((k.to_ascii_lowercase(), v.clone()));
    }

    let mut first = true;
    for (k, v) in params {
        out.push(if first { '?' } else { '&' });
        first = false;
        out.push_str(&k);
        out.push('=');
        out.push_str(&encode_component(&v));
    }

    out
}

/// 解析 `dm://` URL。未识别的查询参数保留进 [`DmConfig::extra`]（键转大写）。
pub fn parse_dm_url(s: &str) -> Result<DmConfig, DmError> {
    let rest = s
        .trim()
        .strip_prefix("dm://")
        .ok_or_else(|| cs_err("URL 形式的连接串必须以 `dm://` 开头"))?;

    // 先切出查询串
    let (authority_and_path, query) = match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };

    // userinfo@hostport/schema
    let (userinfo, host_and_path) = match authority_and_path.find('@') {
        Some(i) => (Some(&authority_and_path[..i]), &authority_and_path[i + 1..]),
        None => (None, authority_and_path),
    };

    let mut cfg = DmConfig::default();

    if let Some(ui) = userinfo {
        let (u, p) = match ui.find(':') {
            Some(i) => (&ui[..i], &ui[i + 1..]),
            None => (ui, ""),
        };
        cfg.user = decode_component(u)?;
        cfg.password = SecretString::new(decode_component(p)?);
    }

    let (hostport, path) = match host_and_path.find('/') {
        Some(i) => (&host_and_path[..i], &host_and_path[i + 1..]),
        None => (host_and_path, ""),
    };

    // 主机与端口：从最后一个 ':' 切（主机里的 ':' 已被编码，所以不会歧义）
    match hostport.rfind(':') {
        Some(i) => {
            cfg.server = decode_component(&hostport[..i])?;
            let p = &hostport[i + 1..];
            if !p.is_empty() {
                cfg.port = p
                    .parse::<u16>()
                    .map_err(|_| cs_err(format!("URL 里的端口 `{p}` 不是合法的 16 位端口号")))?;
            }
        }
        None => cfg.server = decode_component(hostport)?,
    }

    if !path.is_empty() {
        cfg.schema = Some(decode_component(path)?);
    }

    if let Some(q) = query {
        for part in q.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = match part.find('=') {
                Some(i) => (&part[..i], &part[i + 1..]),
                None => (part, ""),
            };
            let value = decode_component(v)?;
            // 只有两个分支会「拿走」value，其余只借用；用 clone 让 value 在所有分支后仍然可用。
            let mut handled = true;
            match k.to_ascii_lowercase().as_str() {
                "driver" => cfg.driver = value.clone(),
                "character_code" => cfg.character_code = Some(value.clone()),
                "compatible_mode" => match CompatMode::parse(&value) {
                    Some(m) => cfg.compatible_mode = Some(m),
                    None => handled = false,
                },
                "rw_separate" => match parse_rw_separate(&value) {
                    Some(b) => cfg.rw_separate = Some(b),
                    None => handled = false,
                },
                "connection_timeout" => match value.trim().parse::<u64>() {
                    Ok(n) => cfg.connection_timeout = Duration::from_secs(n),
                    Err(_) => handled = false,
                },
                "login_timeout" => match value.trim().parse::<u64>() {
                    Ok(n) => cfg.login_timeout = Duration::from_secs(n),
                    Err(_) => handled = false,
                },
                _ => handled = false,
            }
            if !handled {
                cfg.extra.insert(k.to_ascii_uppercase(), value);
            }
        }
    }

    Ok(cfg)
}

/// 按前缀自动分派：`dm://` 走 URL 解析，其余按 ODBC 连接串解析。
pub fn parse(s: &str) -> Result<DmConfig, DmError> {
    if s.trim_start().starts_with("dm://") {
        parse_dm_url(s)
    } else {
        parse_odbc_string(s)
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use super::*;
    use crate::config::{CharsetCheck, DEFAULT_SLOW_QUERY_THRESHOLD, PoolConfig};

    const BASIC: &str =
        "DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA";

    // ---------------- 解析 ----------------

    #[test]
    fn parses_the_evidenced_baseline_connection_string() {
        let c = parse_odbc_string(BASIC).unwrap();
        assert_eq!(c.driver, "DM8 ODBC DRIVER");
        assert_eq!(c.server, "localhost");
        assert_eq!(c.port, 5236);
        assert_eq!(c.user, "SYSDBA");
        assert_eq!(c.password.expose(), "SYSDBA");
        // 未出现的字段保持默认
        assert_eq!(c.charset_check, CharsetCheck::Error);
        assert!(c.schema.is_none());
    }

    #[test]
    fn parse_accepts_trailing_semicolon_and_whitespace_and_empty_pairs() {
        for s in [
            "SERVER=h;UID=u;",
            "  SERVER=h ; UID=u ",
            ";;SERVER=h;;UID=u;;",
            "SERVER=h;\nUID=u;\n",
        ] {
            let c = parse_odbc_string(s).unwrap_or_else(|e| panic!("解析 `{s}` 失败：{e}"));
            assert_eq!(c.server, "h", "输入 {s:?}");
            assert_eq!(c.user, "u", "输入 {s:?}");
        }
    }

    #[test]
    fn keys_are_case_insensitive_and_normalised_to_upper() {
        let c = parse_odbc_string("server=host1;uid=u1;pwd=p1;tcp_port=1111").unwrap();
        assert_eq!(c.server, "host1");
        assert_eq!(c.user, "u1");
        assert_eq!(c.password.expose(), "p1");
        assert_eq!(c.port, 1111);
    }

    #[test]
    fn alias_keys_are_accepted() {
        let c = parse_odbc_string("SERVERNAME=a;HOST=b;USER=u;PASSWORD=p;PORT=999").unwrap();
        // 后面的同义词覆盖前面的
        assert_eq!(c.server, "b");
        assert_eq!(c.user, "u");
        assert_eq!(c.password.expose(), "p");
        assert_eq!(c.port, 999);
    }

    #[test]
    fn braced_values_protect_semicolons_and_braces() {
        // `{weird}}name}` 里 `}}` 是一个字面量 `}`，最后一个 `}` 才是结束符
        let c = parse_odbc_string("PWD={a;b};SERVER={weird}}name}").unwrap();
        assert_eq!(c.password.expose(), "a;b");
        assert_eq!(c.server, "weird}name");
    }

    #[test]
    fn braced_values_are_not_trimmed_but_unbraced_are() {
        let c = parse_odbc_string("UID={  spaced  };SERVER=  trimmed  ").unwrap();
        assert_eq!(c.user, "  spaced  ");
        assert_eq!(c.server, "trimmed");
    }

    #[test]
    fn empty_braced_value_is_allowed() {
        let c = parse_odbc_string("PWD={};UID=u").unwrap();
        assert_eq!(c.password.expose(), "");
        assert!(c.password.is_empty());
    }

    #[test]
    fn unknown_keys_land_in_extra_with_uppercase_names() {
        let c = parse_odbc_string("SERVER=h;PROTOCOL_TYPE=TCP;schema=SYSDBA").unwrap();
        assert_eq!(c.schema.as_deref(), Some("SYSDBA"), "已知键不该进 extra");
        assert_eq!(
            c.extra.get("PROTOCOL_TYPE").map(String::as_str),
            Some("TCP")
        );
        assert!(!c.extra.contains_key("SCHEMA"));
        assert!(!c.extra.contains_key("SERVER"));
    }

    #[test]
    fn dsn_key_is_preserved_via_extra() {
        // DmConfig 没有 dsn 字段（设计方案 §6.1），DSN 走逃生舱原样保留
        let c = parse_odbc_string("DSN=DM8;UID=u").unwrap();
        assert_eq!(c.extra.get("DSN").map(String::as_str), Some("DM8"));
        assert!(render_odbc_string(&c).contains("DSN=DM8;"));
    }

    #[test]
    fn compatible_mode_and_rw_separate_parse_into_structured_fields() {
        let c = parse_odbc_string("COMPATIBLE_MODE=ORACLE;RW_SEPARATE=ON").unwrap();
        assert_eq!(c.compatible_mode, Some(CompatMode::Oracle));
        assert_eq!(c.rw_separate, Some(true));

        let c2 = parse_odbc_string("COMPATIBLE_MODE=0;RW_SEPARATE=OFF").unwrap();
        assert_eq!(c2.compatible_mode, Some(CompatMode::Dm));
        assert_eq!(c2.rw_separate, Some(false));
    }

    #[test]
    fn rw_separate_on2_is_passed_through_verbatim() {
        // ON2（只连事务一致性备库）表达不进 Option<bool>，必须原样保留而不是丢失
        let c = parse_odbc_string("RW_SEPARATE=ON2").unwrap();
        assert_eq!(c.rw_separate, None);
        assert_eq!(c.extra.get("RW_SEPARATE").map(String::as_str), Some("ON2"));
        assert!(render_odbc_string(&c).contains("RW_SEPARATE=ON2;"));
        // 结构化部分不应再输出一个 RW_SEPARATE
        assert_eq!(render_odbc_string(&c).matches("RW_SEPARATE=").count(), 1);
    }

    #[test]
    fn unparsable_values_are_kept_raw_not_dropped() {
        let c = parse_odbc_string("COMPATIBLE_MODE=mysql;CONNECTION_TIMEOUT=abc;LOGIN_TIMEOUT=xyz")
            .unwrap();
        assert_eq!(c.compatible_mode, None);
        assert_eq!(
            c.extra.get("COMPATIBLE_MODE").map(String::as_str),
            Some("mysql")
        );
        assert_eq!(
            c.extra.get("CONNECTION_TIMEOUT").map(String::as_str),
            Some("abc")
        );
        assert_eq!(
            c.extra.get("LOGIN_TIMEOUT").map(String::as_str),
            Some("xyz")
        );
        // 超时保持默认
        assert_eq!(c.connection_timeout, Duration::from_secs(30));
    }

    #[test]
    fn timeouts_parse_as_seconds() {
        let c = parse_odbc_string("CONNECTION_TIMEOUT=5;LOGIN_TIMEOUT=7").unwrap();
        assert_eq!(c.connection_timeout, Duration::from_secs(5));
        assert_eq!(c.login_timeout, Duration::from_secs(7));
    }

    // ---------------- 解析错误 ----------------

    #[test]
    fn unclosed_brace_is_a_config_error() {
        let e = parse_odbc_string("PWD={oops;UID=u").unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("没有闭合")));
        assert_eq!(e.context.stage, Stage::Config);
    }

    #[test]
    fn fragment_without_equals_is_a_config_error() {
        let e = parse_odbc_string("SERVER=h;GARBAGE").unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("缺少 '='")));
    }

    #[test]
    fn empty_key_is_a_config_error() {
        let e = parse_odbc_string("=value").unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("空键名")));
    }

    #[test]
    fn bad_port_is_a_config_error() {
        for s in [
            "TCP_PORT=abc",
            "TCP_PORT=99999",
            "TCP_PORT=-1",
            "TCP_PORT=1.5",
        ] {
            let e = parse_odbc_string(s).unwrap_err();
            assert!(
                matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("端口")),
                "输入 {s} 应报端口错误"
            );
        }
    }

    #[test]
    fn empty_input_yields_defaults_without_error() {
        let c = parse_odbc_string("").unwrap();
        assert_eq!(c, DmConfig::default());
    }

    // ---------------- 渲染 ----------------

    /// 构造一个只改若干字段的配置。直接 `let mut c = DmConfig::default();`
    /// 再逐字段赋值会触发 clippy 的 `field_reassign_with_default`。
    fn cfg_with(f: impl FnOnce(&mut DmConfig)) -> DmConfig {
        let mut c = DmConfig::default();
        f(&mut c);
        c
    }

    #[test]
    fn render_emits_the_expected_shape() {
        let cfg = cfg_with(|c| {
            c.server = "localhost".into();
            c.user = "SYSDBA".into();
            c.password = SecretString::new("SYSDBA");
        });
        let s = render_odbc_string(&cfg);
        assert!(s.starts_with("DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA;CONNECTION_TIMEOUT=30;LOGIN_TIMEOUT=30;"), "实际：{s}");
        assert!(s.ends_with(';'));
    }

    #[test]
    fn render_braces_values_that_need_it() {
        let cfg = cfg_with(|c| {
            c.password = SecretString::new("p;a}s{s");
            c.server = "host".to_string();
        });
        let s = render_odbc_string(&cfg);
        // `}` 必须写成 `}}`
        assert!(s.contains("PWD={p;a}}s{s};"), "实际：{s}");
        // 解析回来必须一致
        assert_eq!(parse_odbc_string(&s).unwrap(), cfg);
    }

    #[test]
    fn render_always_braces_the_driver() {
        let cfg = cfg_with(|c| c.driver = "NoSpaces".into());
        assert!(render_odbc_string(&cfg).starts_with("DRIVER={NoSpaces};"));
    }

    #[test]
    fn render_skips_keys_present_in_extra() {
        let cfg = cfg_with(|c| {
            c.server = "structured".into();
            c.extra.insert("SERVER".into(), "raw".into());
        });
        let s = render_odbc_string(&cfg);
        assert!(s.contains("SERVER=raw;"));
        assert!(!s.contains("SERVER=structured;"));
    }

    #[test]
    fn render_then_parse_is_identity_for_the_baseline() {
        let c = parse_odbc_string(BASIC).unwrap();
        let again = parse_odbc_string(&render_odbc_string(&c)).unwrap();
        assert_eq!(again, c);
    }

    /// CT-02（离线侧）：**只用结构化字段**的配置渲染出来的连接串不带 `DSN`。
    ///
    /// 「无 DSN 连接串可用」这条契约能离线验的那一半就是它：渲染结果里一旦出现 `DSN=`，
    /// 驱动管理器就会绕过 `DRIVER`/`SERVER`/`UID` 去查数据源，我们写进连接串的字段全部失效。
    ///
    /// `DSN` 只允许作为**用户显式要求**的逃生舱出现（另见 `dsn_key_is_preserved_via_extra`），
    /// 而 [`DmConfig::validate_connectable`] 明确允许「有 `DSN`、没 `server`」——
    /// 那是达梦 ODBC 的合法用法，不是我们的依赖。真机侧另有
    /// `crates/dm-odbc/tests/integration/ct.rs` 用结构化字段直接建连。
    #[test]
    fn ct02_structured_render_carries_no_dsn() {
        let cfg = parse_odbc_string(BASIC).unwrap();
        let rendered = render_odbc_string(&cfg);
        assert!(
            rendered.starts_with(
                "DRIVER={DM8 ODBC DRIVER};SERVER=localhost;TCP_PORT=5236;UID=SYSDBA;PWD=SYSDBA;"
            ),
            "结构化字段必须逐个落到连接串里：{rendered}"
        );
        assert!(
            !rendered.to_ascii_uppercase().contains("DSN="),
            "结构化配置不该渲染出 DSN：{rendered}"
        );
        // 配置本身也真的没有 DSN——否则上面那句只能证明「渲染器恰好没输出」。
        assert!(
            cfg.extra.is_empty(),
            "BASIC 不该产生 extra：{:?}",
            cfg.extra
        );
        assert!(
            !cfg.to_connection_string()
                .to_ascii_uppercase()
                .contains("DSN="),
            "`DmConfig::to_connection_string` 与 `render_odbc_string` 必须是同一件事"
        );
    }

    // ---------------- URL 形式 ----------------

    #[test]
    fn url_parse_basic() {
        let c = parse_dm_url("dm://SYSDBA:SYSDBA@localhost:5236/SYSDBA").unwrap();
        assert_eq!(c.user, "SYSDBA");
        assert_eq!(c.password.expose(), "SYSDBA");
        assert_eq!(c.server, "localhost");
        assert_eq!(c.port, 5236);
        assert_eq!(c.schema.as_deref(), Some("SYSDBA"));
        assert_eq!(c.driver, "DM8 ODBC DRIVER", "URL 里没给 driver 时走默认");
    }

    #[test]
    fn url_parse_without_port_schema_or_userinfo() {
        let c = parse_dm_url("dm://myhost").unwrap();
        assert_eq!(c.server, "myhost");
        assert_eq!(c.port, 5236, "缺省端口应回落到 5236");
        assert!(c.schema.is_none());
        assert_eq!(c.user, "");
    }

    #[test]
    fn url_parse_query_params() {
        let c = parse_dm_url(
            "dm://u:p@h:1111/s?driver=MyDriver&character_code=PG_UTF8&compatible_mode=oracle&rw_separate=on&connection_timeout=4&login_timeout=6",
        )
        .unwrap();
        assert_eq!(c.driver, "MyDriver");
        assert_eq!(c.character_code.as_deref(), Some("PG_UTF8"));
        assert_eq!(c.compatible_mode, Some(CompatMode::Oracle));
        assert_eq!(c.rw_separate, Some(true));
        assert_eq!(c.connection_timeout, Duration::from_secs(4));
        assert_eq!(c.login_timeout, Duration::from_secs(6));
    }

    #[test]
    fn url_unknown_query_params_go_to_extra() {
        let c = parse_dm_url("dm://u:p@h:1111/s?MP_LOGIN=1&schema_conflict=zzz").unwrap();
        assert_eq!(c.extra.get("MP_LOGIN").map(String::as_str), Some("1"));
        assert_eq!(
            c.extra.get("SCHEMA_CONFLICT").map(String::as_str),
            Some("zzz")
        );
    }

    #[test]
    fn url_requires_the_scheme() {
        let e = parse_dm_url("http://x").unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("dm://")));
    }

    #[test]
    fn url_bad_port_is_a_config_error() {
        let e = parse_dm_url("dm://u:p@h:notaport/s").unwrap_err();
        assert!(matches!(e.kind, DmErrorKind::Config(ref m) if m.contains("端口")));
    }

    #[test]
    fn percent_encoding_roundtrips_all_unsafe_characters() {
        let c = DmConfig {
            server: "h".into(),
            user: "u@s:e/r?t#f".into(),
            password: SecretString::new("p;a}s{s%20 @:/?#"),
            schema: Some("sch ema/带中文".into()),
            ..DmConfig::default()
        };
        let url = render_dm_url(&c);
        let back = parse_dm_url(&url).unwrap();
        assert_eq!(back.user, c.user);
        assert_eq!(back.password, c.password);
        assert_eq!(back.schema, c.schema);
        assert_eq!(back.server, c.server);
    }

    #[test]
    fn percent_decode_rejects_malformed_input() {
        assert!(parse_dm_url("dm://u:p@h/%ZZ").is_err());
        assert!(parse_dm_url("dm://u:p@h/%A").is_err());
        // 单独出现 '%' 且后面不足两位
        assert!(decode_component("%").is_err());
        assert!(decode_component("%4").is_err());
    }

    #[test]
    fn percent_decode_rejects_invalid_utf8() {
        // 0xFF 不是合法 UTF-8 起始字节
        assert!(decode_component("%FF").is_err());
    }

    #[test]
    fn encode_keeps_unreserved_characters_verbatim() {
        assert_eq!(encode_component("aZ0-._~"), "aZ0-._~");
        assert_eq!(encode_component(" "), "%20");
        assert_eq!(encode_component("中"), "%E4%B8%AD");
        assert_eq!(encode_component("%"), "%25");
    }

    // ---------------- 分派 ----------------

    #[test]
    fn parse_dispatches_by_scheme() {
        let a = parse("dm://u:p@h:1111/s").unwrap();
        assert_eq!(a.schema.as_deref(), Some("s"));

        let b = parse(BASIC).unwrap();
        assert_eq!(b.server, "localhost");

        // 前导空白不影响分派
        let c = parse("  dm://u:p@h:1111/s").unwrap();
        assert_eq!(c.schema.as_deref(), Some("s"));
    }

    // ---------------- 性能特性往返（proptest） ----------------

    fn safe_text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            proptest::char::any().prop_filter("去掉控制字符", |c| !c.is_control()),
            0..16,
        )
        .prop_map(|v| v.into_iter().collect::<String>())
    }

    fn safe_nonempty() -> impl Strategy<Value = String> {
        safe_text().prop_filter("非空", |s| !s.trim().is_empty())
    }

    fn extra_map() -> impl Strategy<Value = BTreeMap<String, String>> {
        prop::collection::btree_map("[A-Z][A-Z0-9_]{0,12}", safe_text(), 0..4).prop_map(|m| {
            m.into_iter()
                .filter(|(k, _)| !RESERVED_KEYS.contains(&k.as_str()))
                .collect::<BTreeMap<_, _>>()
        })
    }

    fn compat() -> impl Strategy<Value = CompatMode> {
        prop::sample::select(vec![CompatMode::Dm, CompatMode::Oracle])
    }

    /// 生成「ODBC 连接串能完整表达」的那部分配置。
    ///
    /// `charset_check`、`length_in_char`、`init_sql`、`pool` 固定在默认值上——
    /// 它们不进连接串，因此往返恒等式只在这个子集上成立（见模块文档）。
    fn carried_config() -> impl Strategy<Value = DmConfig> {
        (
            safe_nonempty(),
            safe_text(),
            1u16..=u16::MAX,
            safe_text(),
            safe_text(),
            prop::option::of(safe_nonempty()),
            prop::option::of(safe_nonempty()),
            prop::option::of(compat()),
            prop::option::of(any::<bool>()),
            1u64..86_400,
            1u64..86_400,
            extra_map(),
        )
            .prop_map(
                |(
                    driver,
                    server,
                    port,
                    user,
                    password,
                    schema,
                    character_code,
                    compatible_mode,
                    rw_separate,
                    ct,
                    lt,
                    extra,
                )| {
                    DmConfig {
                        driver,
                        server,
                        port,
                        user,
                        password: SecretString::new(password),
                        schema,
                        character_code,
                        compatible_mode,
                        rw_separate,
                        connection_timeout: Duration::from_secs(ct),
                        login_timeout: Duration::from_secs(lt),
                        init_sql: Vec::new(),
                        charset_check: CharsetCheck::default(),
                        length_in_char: None,
                        slow_query_threshold: DEFAULT_SLOW_QUERY_THRESHOLD,
                        extra,
                        pool: PoolConfig::default(),
                    }
                },
            )
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        /// 核心恒等式：`parse(render(c)) == c`
        #[test]
        fn odbc_string_roundtrip(c in carried_config()) {
            let s = render_odbc_string(&c);
            let back = parse_odbc_string(&s).expect("渲染出来的连接串必须能被自己解析");
            prop_assert_eq!(back, c, "连接串 = {}", s);
        }

        #[test]
        fn dm_url_roundtrip(c in carried_config()) {
            let u = render_dm_url(&c);
            let back = parse_dm_url(&u).expect("渲染出来的 URL 必须能被自己解析");
            prop_assert_eq!(back, c, "URL = {}", u);
        }

        #[test]
        fn parse_dispatches_both_forms(c in carried_config()) {
            let via_odbc = parse(&render_odbc_string(&c)).unwrap();
            let via_url = parse(&render_dm_url(&c)).unwrap();
            prop_assert_eq!(&via_odbc, &c);
            prop_assert_eq!(&via_url, &c);
        }

        /// 渲染必须把 `extra` 完整带出去（逃生舱不能丢数据）
        #[test]
        fn render_preserves_every_extra_entry(c in carried_config()) {
            let s = render_odbc_string(&c);
            let back = parse_odbc_string(&s).unwrap();
            prop_assert_eq!(back.extra, c.extra);
        }
    }
}
