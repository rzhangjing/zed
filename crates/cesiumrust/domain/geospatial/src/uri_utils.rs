//! URI 与 HTTP 工具函数（查询串互转、响应头解析、文件名/扩展名提取）。

// 遗留的 CesiumJS 移植风格技术债（deferred.md #18）；在 M13 lint-cleanup
// 或本文件在其里程碑被重写时重新审视
#![allow(clippy::manual_pattern_char_comparison)]
use std::collections::HashMap;

/// 将表示 URL 参数的对象转换为查询字符串。
///
/// CesiumJS: `objectToQuery(obj)` → `"key1=value1&key2=value2"`
/// 数组会产生重复的键：`{key: ["a","b"]}` → `"key=a&key=b"`
pub fn object_to_query(obj: &HashMap<String, QueryValue>) -> String {
    let mut parts: Vec<String> = Vec::new();
    // 逐键值对拼接 `key=value`；数组展开为多个重复键。
    for (key, value) in obj {
        match value {
            QueryValue::Single(v) => {
                parts.push(format!("{}={}", key, percent_encode(v)));
            }
            QueryValue::Array(arr) => {
                for item in arr {
                    parts.push(format!("{}={}", key, percent_encode(item)));
                }
            }
        }
    }
    parts.join("&")
}

/// 将查询字符串转换为对象。
///
/// CesiumJS: `queryToObject(queryString)` → `{key1: "value1", key2: ["a","b"]}`
/// 同时支持 `&` 和 `;` 作为分隔符。`+` 被解码为空格。
pub fn query_to_object(query_string: &str) -> HashMap<String, QueryValue> {
    let mut result: HashMap<String, Vec<String>> = HashMap::new();

    if query_string.is_empty() {
        return HashMap::new();
    }

    // 在 & 或 ; 处拆分
    let pairs: Vec<&str> = query_string
        .split(|c| c == '&' || c == ';')
        .filter(|s| !s.is_empty())
        .collect();

    for pair in pairs {
        // 以首个 `=` 划分键与值；无 `=` 时整段视为键、值为空串。
        let (key, value) = if let Some(idx) = pair.find('=') {
            (&pair[..idx], &pair[idx + 1..])
        } else {
            (pair, "")
        };
        let decoded_key = percent_decode(key);
        let decoded_value = percent_decode(value);
        result.entry(decoded_key).or_default().push(decoded_value);
    }

    // 将 Vec<String> 转换为 QueryValue
    // 单元素归为 Single，多元素归为 Array。
    result
        .into_iter()
        .map(|(k, v)| {
            if v.len() == 1 {
                (k, QueryValue::Single(v.into_iter().next().unwrap()))
            } else {
                (k, QueryValue::Array(v))
            }
        })
        .collect()
}

/// 将 HTTP 响应头字符串解析为一个映射。
///
/// CesiumJS: `parseResponseHeaders(headerString)` → `{Date: "...", Server: "..."}`
pub fn parse_response_headers(header_string: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    if header_string.is_empty() {
        return result;
    }

    for line in header_string.split("\r\n") {
        // 仅在含 `: ` 分隔符的行上拆分键与值。
        if let Some(idx) = line.find(": ") {
            let key = &line[..idx];
            let value = &line[idx + 2..];
            result.insert(key.to_string(), value.to_string());
        }
    }
    result
}

/// 从 URI 获取文件名（最后一个路径段，不含查询/片段）。
///
/// CesiumJS: `getFilenameFromUri(uri)`
pub fn get_filename_from_uri(uri: &str) -> String {
    // 移除查询字符串
    let path = uri.split('?').next().unwrap_or(uri);
    // 移除片段
    let path = path.split('#').next().unwrap_or(path);
    // 获取最后一个段
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

/// 从 URI 获取文件扩展名（不含点）。
///
/// CesiumJS: `getExtensionFromUri(uri)` → `"png"` 或 `""`
pub fn get_extension_from_uri(uri: &str) -> String {
    let filename = get_filename_from_uri(uri);
    // 取最后一个点之后的子串作扩展名；无点则为空串。
    if let Some(idx) = filename.rfind('.') {
        filename[idx + 1..].to_string()
    } else {
        String::new()
    }
}

/// 查询参数的值类型。
#[derive(Clone, Debug, PartialEq)]
pub enum QueryValue {
    /// 单个值（键只出现一次）。
    Single(String),
    /// 多个值（同一键重复出现，如 `key=a&key=b`）。
    Array(Vec<String>),
}

/// 对一个字符串进行百分号编码（RFC 3986）。
/// 除未保留字符 A-Z a-z 0-9 - _ . ~ 之外的所有字符都会被编码
fn percent_encode(input: &str) -> String {
    let mut result = String::new();
    // 逐字节：未保留字符直接输出，其余写成 `%XX`（大写十六进制）。
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

/// 解码一个百分号编码的字符串。也将 `+` 转换为空格。
fn percent_decode(input: &str) -> String {
    let input = input.replace('+', " ");
    let mut result = String::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    // 扫描字节流：遇 `%XX` 且能解析为十六进制时还原为一个字节，否则原样拷贝。
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                result.push(byte as char);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}
