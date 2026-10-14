//! `data:` URI 方案的解析与解码。
//!
//! 解析 `data:` URI 的媒体类型、base64 标记与负载，并按
//! 目标响应类型分别走 base64 或百分号解码分支。
//! 提供识别（是否为 data URI）、拆解、解码与反向构造。
//!
//! 同时支持 base64 编码与百分号编码（纯文本）的负载。
//! 本模块是 **纯领域逻辑** —— 无 IO，无框架依赖。

/// 解析 `data:` URI 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataUriParts {
    /// 媒体类型 / MIME 类型（例如 `"text/plain"`、`"image/png"`）。
    /// 若未指定则为空字符串（RFC 2397 默认：`text/plain;charset=US-ASCII`）。
    pub media_type: String,
    /// 负载是否为 base64 编码（存在 `;base64` 参数）。
    pub is_base64: bool,
    /// 原始负载字符串（解码之前）。
    pub raw_payload: String,
}

/// 解析或解码 data URI 时可能发生的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataUriError {
    /// URI 不以 `data:` 开头。
    NotaDataUri,
    /// URI 缺少逗号分隔符。
    MissingComma,
    /// base64 解码失败（字符非法或长度错误）。
    InvalidBase64(String),
    /// 百分号解码失败（转义序列格式错误）。
    InvalidPercentEncoding(String),
}

impl std::fmt::Display for DataUriError {
    /// 将每种解码/解析错误渲染为一句人类可读的英文说明。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotaDataUri => write!(f, "URI does not start with 'data:'"),
            Self::MissingComma => write!(f, "data URI missing ',' separator"),
            Self::InvalidBase64(msg) => write!(f, "invalid base64: {msg}"),
            Self::InvalidPercentEncoding(msg) => write!(f, "invalid percent-encoding: {msg}"),
        }
    }
}

impl std::error::Error for DataUriError {}

/// 若给定的 URL 字符串是 `data:` URI 则返回 `true`。
///
/// 忽略前导空白后检查是否以 `data:` 方案开头。
pub fn is_data_uri(url: &str) -> bool {
    url.trim_start().starts_with("data:")
}

/// 将 `data:` URI 解析为其组成部分，但不解码负载。
///
/// 拆解规则等价于正则 `/^data:(.*?)(;base64)?,(.*)$/`：
/// 逗号之前为元数据（媒体类型 + 可选 `;base64` 后缀），之后为负载。
///
/// # 示例
/// ```
/// use cesium_resource::data_uri::parse_data_uri;
/// let parts = parse_data_uri("data:text/plain;base64,SGVsbG8=").unwrap();
/// assert_eq!(parts.media_type, "text/plain");
/// assert!(parts.is_base64);
/// assert_eq!(parts.raw_payload, "SGVsbG8=");
/// ```
pub fn parse_data_uri(uri: &str) -> Result<DataUriParts, DataUriError> {
    let uri = uri.trim_start();
    if !uri.starts_with("data:") {
        return Err(DataUriError::NotaDataUri);
    }

    let after_scheme = &uri[5..]; // 跳过 "data:"

    // 找到分隔元数据与负载的逗号。
    let comma_pos = after_scheme
        .find(',')
        .ok_or(DataUriError::MissingComma)?;

    let metadata = &after_scheme[..comma_pos];
    let payload = &after_scheme[comma_pos + 1..];

    // 检查元数据中是否有 ;base64 后缀。
    let (media_type, is_base64) = match metadata.strip_suffix(";base64") {
        Some(stripped) => (stripped, true),
        None => (metadata, false),
    };

    Ok(DataUriParts {
        media_type: media_type.to_string(),
        is_base64,
        raw_payload: payload.to_string(),
    })
}

/// 解码 `data:` URI 并以原始字节形式返回负载。
///
/// 同时处理 base64 与百分号编码的负载。对于百分号编码的
/// 负载，解码后的字节是百分号解码字符串的 UTF-8 表示。
///
/// 对应 arraybuffer/blob 响应分支：按字节返回解码后的负载。
pub fn decode_data_uri_bytes(uri: &str) -> Result<Vec<u8>, DataUriError> {
    let parts = parse_data_uri(uri)?;
    // base64 负载直接解字节；否则先百分号解码再取 UTF-8 字节
    if parts.is_base64 {
        base64_decode(&parts.raw_payload)
    } else {
        Ok(percent_decode(&parts.raw_payload).into_bytes())
    }
}

/// 解码 `data:` URI 并以 UTF-8 字符串形式返回负载。
///
/// 对于 base64 负载，解码后的字节按 UTF-8 解释（有损）。
/// 对于百分号编码的负载，结果为百分号解码后的字符串。
///
/// 对应 text 响应分支：以 UTF-8 字符串返回解码后的负载。
pub fn decode_data_uri_text(uri: &str) -> Result<String, DataUriError> {
    let parts = parse_data_uri(uri)?;
    if parts.is_base64 {
        let bytes = base64_decode(&parts.raw_payload)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        Ok(percent_decode(&parts.raw_payload))
    }
}

/// 从 data URI 中提取媒体类型，若不是 data URI 则返回 `None`。
///
/// 当媒体类型字段为空时返回默认的 `"text/plain"`
/// （RFC 2397 将 `text/plain;charset=US-ASCII` 指定为默认值）。
pub fn media_type_of(uri: &str) -> Option<String> {
    parse_data_uri(uri).ok().map(|parts| {
        // 媒体类型缺省时回退到 RFC 2397 默认的 text/plain
        if parts.media_type.is_empty() {
            "text/plain".to_string()
        } else {
            parts.media_type
        }
    })
}

// ── 百分号解码 ──────────────────────────────────────────────────────────────

/// 解码字符串中的百分号编码字符（`%XX`）。
///
/// 语义对齐 Web 标准的 `decodeURIComponent`。与之不同的是，非法的
/// 转义序列会被原样透传而非抛错（增强对现实中格式错误的 data URI 的健壮性）。
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        // 遇到 '%' 且后接两个字符时，尝试把其后两位解析为十六进制字节
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(byte) = hex_pair_to_byte(bytes[i + 1], bytes[i + 2]) {
                result.push(byte);
                i += 3; // 消费掉 "%XX" 三字符
                continue;
            }
        }
        // 非转义序列（或非法转义）原样透传当前字节
        result.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&result).into_owned()
}

/// 对 URI 组件进行百分号编码。
///
/// 语义对齐 Web 标准的 `encodeURIComponent`。未保留字符
/// （`A-Z a-z 0-9 - _ . ! ~ * ' ( )`）原样透传；其余一切
/// 均被百分号编码。
pub fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            // 未保留字符集原样保留
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~'
            | b'*' | b'\'' | b'(' | b')' => {
                result.push(byte as char);
            }
            // 其余字节一律编码为 %XX（大写十六进制）
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

/// 将一对十六进制字符转换为字节值。
fn hex_pair_to_byte(hi: u8, lo: u8) -> Option<u8> {
    // 高 4 位来自第一个字符，低 4 位来自第二个字符
    let h = hex_val(hi)?;
    let l = hex_val(lo)?;
    Some((h << 4) | l)
}

/// 将单个十六进制 ASCII 字符转换为其 4 位值。
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

// ── Base64 解码 ─────────────────────────────────────────────────────────

/// 极简 base64 解码器（对齐浏览器的 `atob` 语义）。
///
/// 处理标准 base64 字母表（`A-Z a-z 0-9 + /`）与 `=` 填充。
/// 空白字符会被静默跳过（沿用宽松解析行为）。
///
/// 输入格式错误时返回 [`DataUriError::InvalidBase64`]。
pub fn base64_decode(input: &str) -> Result<Vec<u8>, DataUriError> {
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();

    if bytes.is_empty() {
        return Ok(Vec::new());
    }

    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);

    // 按每 4 个字符一组（一个 base64 "quantum"）迭代解码
    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            return Err(DataUriError::InvalidBase64(
                "incomplete base64 quantum".to_string(),
            ));
        }

        // 统计填充
        let padding = chunk.iter().filter(|&&b| b == b'=').count();

        // 把 4 个 6 位字符拼进一个 24 位缓冲区（高位在前）
        let mut buf: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            // '=' 视为 0 值，其余按 base64 字母表映射为 6 位值
            let v = if b == b'=' {
                0
            } else {
                base64_char_value(b).ok_or_else(|| {
                    DataUriError::InvalidBase64(format!(
                        "invalid character '{}' at position {}",
                        b as char, i
                    ))
                })?
            };
            buf |= (v as u32) << (18 - 6 * i);
        }

        // 依填充数量决定本组实际输出 1~3 个字节
        output.push((buf >> 16) as u8);
        if padding < 2 {
            output.push((buf >> 8) as u8);
        }
        if padding < 1 {
            output.push(buf as u8);
        }
    }

    Ok(output)
}

/// 将字节编码为 base64 字符串（标准字母表，带填充）。
///
/// 用于往返测试以及以编程方式构造 data URI
/// （例如在测试 fixture 或离线资源生成器中）。
pub fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);

    // 每 3 字节一组，拼成一个 24 位值后按 6 位切分为 4 个 base64 字符
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        // 前两个字符始终输出；不足 3 字节时用 '=' 填充尾部
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

/// 将 base64 字符转换为其 6 位值。
fn base64_char_value(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

// ── Data URI 构造辅助函数 ────────────────────────────────────────────

/// 由媒体类型与原始字节构造一个 `data:` URI（base64 编码）。
///
/// 适用于内联嵌入小资源（例如把测试 fixture 或离线生成的
/// 二进制负载直接编码进 URI，免去外部文件依赖）。
pub fn build_data_uri_base64(media_type: &str, data: &[u8]) -> String {
    format!("data:{};base64,{}", media_type, base64_encode(data))
}

/// 由媒体类型与纯文本构造一个 `data:` URI（百分号编码）。
pub fn build_data_uri_text(media_type: &str, text: &str) -> String {
    format!("data:{},{}", media_type, percent_encode(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_data_uri ─────────────────────────────────────────────────────

    #[test]
    fn recognizes_data_uris() {
        assert!(is_data_uri("data:text/plain,hello"));
        assert!(is_data_uri("data:;base64,AAA="));
        assert!(is_data_uri("data:,minimal"));
        assert!(!is_data_uri("https://example.com"));
        assert!(!is_data_uri("blob:https://example.com/uuid"));
        assert!(!is_data_uri(""));
    }

    // ── parse_data_uri ──────────────────────────────────────────────────

    #[test]
    fn parses_plain_text_data_uri() {
        let parts = parse_data_uri("data:text/plain,Hello%20World").unwrap();
        assert_eq!(parts.media_type, "text/plain");
        assert!(!parts.is_base64);
        assert_eq!(parts.raw_payload, "Hello%20World");
    }

    #[test]
    fn parses_base64_data_uri() {
        let parts = parse_data_uri("data:image/png;base64,iVBOR...").unwrap();
        assert_eq!(parts.media_type, "image/png");
        assert!(parts.is_base64);
        assert_eq!(parts.raw_payload, "iVBOR...");
    }

    #[test]
    fn parses_empty_media_type() {
        let parts = parse_data_uri("data:,hello").unwrap();
        assert_eq!(parts.media_type, "");
        assert!(!parts.is_base64);
        assert_eq!(parts.raw_payload, "hello");
    }

    #[test]
    fn rejects_non_data_uri() {
        assert_eq!(
            parse_data_uri("https://example.com"),
            Err(DataUriError::NotaDataUri)
        );
    }

    #[test]
    fn rejects_missing_comma() {
        assert_eq!(
            parse_data_uri("data:text/plain"),
            Err(DataUriError::MissingComma)
        );
    }

    // ── decode ──────────────────────────────────────────────────────────

    #[test]
    fn decodes_percent_encoded_text() {
        let text = decode_data_uri_text("data:text/plain,A%20brief%20note").unwrap();
        assert_eq!(text, "A brief note");
    }

    #[test]
    fn decodes_base64_text() {
        // base64("Hello") = "SGVsbG8="
        let text = decode_data_uri_text("data:text/plain;base64,SGVsbG8=").unwrap();
        assert_eq!(text, "Hello");
    }

    #[test]
    fn decodes_base64_bytes() {
        // base64("abc") = "YWJj"
        let bytes = decode_data_uri_bytes("data:application/octet-stream;base64,YWJj").unwrap();
        assert_eq!(bytes, vec![97, 98, 99]);
    }

    #[test]
    fn decodes_percent_encoded_bytes() {
        let bytes = decode_data_uri_bytes("data:text/plain,Hi%21").unwrap();
        assert_eq!(bytes, b"Hi!".to_vec());
    }

    // ── media_type_of ───────────────────────────────────────────────────

    #[test]
    fn media_type_extraction() {
        assert_eq!(
            media_type_of("data:application/json;base64,e30="),
            Some("application/json".to_string())
        );
        assert_eq!(
            media_type_of("data:,bare"),
            Some("text/plain".to_string()) // RFC 2397 默认
        );
        assert_eq!(media_type_of("https://example.com"), None);
    }

    // ── 百分号编码/解码 ───────────────────────────────────────────

    #[test]
    fn percent_encode_special_chars() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_encode("a/b?c=d&e"), "a%2Fb%3Fc%3Dd%26e");
        assert_eq!(percent_encode("unreserved-_.!~*'()"), "unreserved-_.!~*'()");
    }

    #[test]
    fn percent_decode_roundtrip() {
        let original = "hello world & goodbye/special=chars";
        let encoded = percent_encode(original);
        let decoded = percent_decode(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn percent_decode_passes_through_invalid() {
        // 非法的百分号序列会原样透传
        assert_eq!(percent_decode("%ZZ%"), "%ZZ%");
        assert_eq!(percent_decode("100%"), "100%");
    }

    // ── base64 ──────────────────────────────────────────────────────────

    #[test]
    fn base64_roundtrip() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data.to_vec());
    }

    #[test]
    fn base64_handles_padding() {
        // 1 字节 → 2 个填充字符
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a".to_vec());
        // 2 字节 → 1 个填充字符
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_decode("YWI=").unwrap(), b"ab".to_vec());
        // 3 字节 → 无填充
        assert_eq!(base64_encode(b"abc"), "YWJj");
        assert_eq!(base64_decode("YWJj").unwrap(), b"abc".to_vec());
    }

    #[test]
    fn base64_rejects_invalid_char() {
        assert!(base64_decode("!!!!").is_err());
    }

    #[test]
    fn base64_empty_input() {
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
        assert_eq!(base64_encode(b""), "");
    }

    // ── 构造辅助函数 ───────────────────────────────────────────────────

    #[test]
    fn build_base64_data_uri() {
        let uri = build_data_uri_base64("image/png", b"abc");
        assert_eq!(uri, "data:image/png;base64,YWJj");
        // 往返
        let bytes = decode_data_uri_bytes(&uri).unwrap();
        assert_eq!(bytes, b"abc".to_vec());
    }

    #[test]
    fn build_text_data_uri() {
        let uri = build_data_uri_text("text/plain", "hello world");
        assert_eq!(uri, "data:text/plain,hello%20world");
        let text = decode_data_uri_text(&uri).unwrap();
        assert_eq!(text, "hello world");
    }

    // ── 查询参数多值支持 ─────────────────────────────────

    #[test]
    fn data_uri_with_query_like_payload() {
        // Data URI 的负载中可以包含 '?' 和 '&' —— 这些不是
        // 查询参数，而是数据的一部分。
        let parts = parse_data_uri("data:application/x-www-form-urlencoded,a=1&b=2").unwrap();
        assert_eq!(parts.raw_payload, "a=1&b=2");
        assert_eq!(parts.media_type, "application/x-www-form-urlencoded");
    }
}
