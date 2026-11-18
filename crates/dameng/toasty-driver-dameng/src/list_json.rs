//! `Vec<scalar>` 的 JSON 文本编解码（设计方案 §9.8 第 23 条，T8.12）。
//!
//! # 为什么是 JSON 文本
//!
//! 达梦**没有**原生数组列，也没有 `MEMBER OF` / `SUBMULTISET`（真机探针第 17 节，
//! 17k–17n 全是语法错误）；但它有 MySQL 那一族的 JSON 判定函数，而且**能吃绑定参数**：
//! `JSON_CONTAINS(列, ?)`（17ak）、`JSON_OVERLAPS(列, ?)`（17al）、
//! `JSON_LENGTH(列)`（17r）、`JSON_CONTAINS(列, JSON_ARRAY(?))`（17am），
//! 语义与 MySQL 对齐（17ap/17ao），且大小写不受服务端默认排序规则影响（17h/17i/17an）。
//!
//! ⇒ 列表列在达梦上的存储形态只能是**一段 JSON 数组文本**（fork 给 `db::Type::List`
//! 选的列型是 `CLOB`，`crates/toasty-sql/src/serializer/ty.rs`），元素是 JSON 标量：
//!
//! ```text
//! vec!["admin", "verified"]  ⇄  ["admin","verified"]
//! vec![1i64, 2]              ⇄  [1,2]
//! ```
//!
//! # 纪律
//!
//! 与 `value.rs` 同一条：**不做有损转换**。放不下的东西（裸字节、嵌套集合、
//! `BigDecimal`、NaN/∞）一律报 `unsupported_feature`，JSON 写坏/读坏一律报
//! `serialization_failure`——绝不当成空列表糊过去。
//!
//! 数字**按原文保留**：解码时不去碰 f64，`Decimal` 的有效数字（`rust_decimal` 上限
//! 28–29 位，列宽是 `DECIMAL(38, 18)`）原样搬回。

use std::str::FromStr;

use stmt::{Type as T, Value as V};
use toasty_core::{Error, stmt};

// ---------------------------------------------------------------------------
// 编码：stmt::Value 列表 → JSON 数组文本
// ---------------------------------------------------------------------------

/// `Vec<stmt::Value>` → `["a","b"]` 这样的 JSON 数组文本。
///
/// # Errors
///
/// 元素里有 JSON 放不下的东西（裸字节 / 嵌套集合 / `BigDecimal` / 非有限浮点）时返回
/// `unsupported_feature`。
pub(crate) fn encode(items: &[V]) -> Result<String, Error> {
    let mut out = String::with_capacity(2 + items.len() * 8);
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_scalar(&mut out, item)?;
    }
    out.push(']');
    Ok(out)
}

fn write_scalar(out: &mut String, value: &V) -> Result<(), Error> {
    match value {
        V::Null => out.push_str("null"),
        V::Bool(v) => out.push_str(if *v { "true" } else { "false" }),

        V::I8(v) => out.push_str(&v.to_string()),
        V::I16(v) => out.push_str(&v.to_string()),
        V::I32(v) => out.push_str(&v.to_string()),
        V::I64(v) => out.push_str(&v.to_string()),
        V::U8(v) => out.push_str(&v.to_string()),
        V::U16(v) => out.push_str(&v.to_string()),
        V::U32(v) => out.push_str(&v.to_string()),
        V::U64(v) => out.push_str(&v.to_string()),

        V::F32(v) => write_float(out, f64::from(*v))?,
        V::F64(v) => write_float(out, *v)?,
        // `rust_decimal` 的 `Display` 是定点十进制（不用科学计数法），本身就是合法 JSON 数字。
        V::Decimal(v) => out.push_str(&v.to_string()),

        V::String(v) => write_string(out, v),
        // UUID 的**标量**形态是 BLOB 里的 16 字节，但 JSON 文本里放不下裸字节 ⇒ 用规范文本。
        // 解码侧按 `T::Uuid` 解析回 `V::Uuid`，无损。
        V::Uuid(v) => write_string(out, &v.to_string()),

        V::Timestamp(v) => write_string(out, &v.to_string()),
        V::DateTime(v) => write_string(out, &v.to_string()),
        V::Date(v) => write_string(out, &v.to_string()),
        V::Time(v) => write_string(out, &v.to_string()),
        // `default_zoned_type = Text` 时引擎本应把整列 cast 成 String；真进来了就存 RFC 3339。
        V::Zoned(v) => write_string(out, &v.to_string()),

        // 网络地址族：`native_*= false` + `default_*_type = Text` ⇒ 存各自规范的文本形态。
        V::Cidr(v) => write_string(out, &v.to_string()),
        V::Inet(v) => write_string(out, &v.to_string()),
        V::MacAddr(v) => write_string(out, &v.to_string()),
        V::MacAddr8(v) => write_string(out, &v.to_string()),

        V::Bytes(_) => {
            return Err(unsupported(
                "列表里的 BYTES 元素（JSON 数组文本放不下裸字节；如确需请先用引擎侧 cast 成文本）",
            ));
        }
        V::BigDecimal(_) => {
            return Err(unsupported(
                "列表里的 BigDecimal 元素（达梦驱动只实现 Decimal / rust_decimal）",
            ));
        }
        V::List(_) | V::Record(_) | V::SparseRecord(_) | V::Object(_) => {
            return Err(unsupported(
                "嵌套列表 / 记录 / 文档元素（达梦的列表列是 CLOB 里的**标量** JSON 数组）",
            ));
        }
    }
    Ok(())
}

/// 浮点：必须是有限值（JSON 没有 NaN/±∞），且**一定**带小数点或指数
/// ——否则 `1.0` 会被写成 `1`，达梦的 `JSON_CONTAINS` 就把它当整数比了（探针 17ao）。
fn write_float(out: &mut String, v: f64) -> Result<(), Error> {
    if !v.is_finite() {
        return Err(unsupported("列表里的 NaN / ±∞ 元素（不是合法 JSON 数字）"));
    }

    let text = v.to_string();
    out.push_str(&text);
    if !text.contains(['.', 'e', 'E']) {
        out.push_str(".0");
    }
    Ok(())
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u");
                out.push_str(&format!("{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// 解码：JSON 数组文本 → stmt::Value 列表（元素类型由引擎给出的 `T::List(内层)` 决定）
// ---------------------------------------------------------------------------

/// JSON 数组文本 → `Vec<stmt::Value>`，每个元素按列表列的内层类型还原。
///
/// `ty` 是**列表列**的语句类型（`T::List(内层)`，即 `value.rs::decode` 收到的那一个）；
/// 不是列表类型时直接报不支持，免得静默按错类型解。
///
/// # Errors
///
/// 文本不是合法的「标量 JSON 数组」，或某个元素无法按内层类型还原（含越界整数）时返回
/// `serialization_failure`；内层类型是达梦放不下的类型时返回 `unsupported_feature`。
pub(crate) fn decode(text: &str, ty: &T) -> Result<Vec<V>, Error> {
    let elem = match ty {
        T::List(elem) => elem.as_ref(),
        other => {
            return Err(unsupported(format!(
                "列表列解码要求 `T::List`，实际拿到 {other:?}"
            )));
        }
    };
    let mut parser = Parser::new(text);
    let items = parser.parse_array(elem)?;
    parser.skip_ws();
    if !parser.is_eof() {
        return Err(bad(format!(
            "列表列文本尾部有多余内容（第 {} 字节起：{:?}）",
            parser.pos,
            parser.remaining_head()
        )));
    }
    Ok(items)
}

/// JSON 数组文本里允许出现的**标量**元素。
///
/// 数字保留**原文**（不经过 f64）——`Decimal` 的精度就靠这一点。
#[derive(Debug)]
enum Elem {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
}

fn element_to_value(elem: Elem, ty: &T) -> Result<V, Error> {
    // `NULL` 对任何元素类型都合法（与 `value.rs::decode` 同一条规则）。
    if matches!(elem, Elem::Null) {
        return Ok(V::Null);
    }

    Ok(match (ty, elem) {
        (T::Bool, Elem::Bool(v)) => V::Bool(v),
        (T::Bool, other) => return Err(element_mismatch(&other, ty)),

        (T::I8, Elem::Number(n)) => V::I8(int_in_range::<i8>(&n, ty)?),
        (T::I16, Elem::Number(n)) => V::I16(int_in_range::<i16>(&n, ty)?),
        (T::I32, Elem::Number(n)) => V::I32(int_in_range::<i32>(&n, ty)?),
        (T::I64, Elem::Number(n)) => V::I64(int_in_range::<i64>(&n, ty)?),
        (T::U8, Elem::Number(n)) => V::U8(uint_in_range::<u8>(&n, ty)?),
        (T::U16, Elem::Number(n)) => V::U16(uint_in_range::<u16>(&n, ty)?),
        (T::U32, Elem::Number(n)) => V::U32(uint_in_range::<u32>(&n, ty)?),
        (T::U64, Elem::Number(n)) => V::U64(uint_in_range::<u64>(&n, ty)?),

        // f64 → f32 的收窄与 `value.rs::decode` 的 `T::F32` 分支同一条先例（ODBC 也只给 f64）。
        (T::F32, Elem::Number(n)) => V::F32(parse_f64(&n, ty)? as f32),
        (T::F64, Elem::Number(n)) => V::F64(parse_f64(&n, ty)?),
        // 走原文而不是 f64：`NUMBER` 列（`decimal_arbitrary_precision = true`）的精度靠这里保住。
        (T::Decimal, Elem::Number(n)) => V::Decimal(
            rust_decimal::Decimal::from_str(&n)
                .map_err(|_| bad(format!("列表元素 {n:?} 不是合法的小数")))?,
        ),

        (T::String, Elem::Str(s)) => V::String(s),
        (T::Uuid, Elem::Str(s)) => V::Uuid(
            uuid::Uuid::from_str(&s).map_err(|_| bad(format!("列表元素 {s:?} 不是合法 UUID")))?,
        ),

        (T::Timestamp, Elem::Str(s)) => V::Timestamp(parse_text(&s, ty)?),
        (T::DateTime, Elem::Str(s)) => V::DateTime(parse_text(&s, ty)?),
        (T::Date, Elem::Str(s)) => V::Date(parse_text(&s, ty)?),
        (T::Time, Elem::Str(s)) => V::Time(parse_text(&s, ty)?),
        (T::Zoned, Elem::Str(s)) => V::Zoned(parse_text(&s, ty)?),

        (T::Cidr, Elem::Str(s)) => V::Cidr(parse_text(&s, ty)?),
        (T::Inet, Elem::Str(s)) => V::Inet(parse_text(&s, ty)?),
        (T::MacAddr, Elem::Str(s)) => V::MacAddr(parse_text(&s, ty)?),
        (T::MacAddr8, Elem::Str(s)) => V::MacAddr8(parse_text(&s, ty)?),

        (T::Bytes, _) => {
            return Err(unsupported(
                "列表列的 BYTES 元素（JSON 数组文本里没有裸字节的落点）",
            ));
        }
        (other, elem) => return Err(element_mismatch(&elem, other)),
    })
}

fn element_mismatch(elem: &Elem, ty: &T) -> Error {
    bad(format!(
        "列表元素 {elem:?} 无法还原成元素类型 {ty:?}（不做有损转换）"
    ))
}

fn parse_f64(n: &str, ty: &T) -> Result<f64, Error> {
    n.parse::<f64>()
        .map_err(|_| bad(format!("列表元素 {n:?} 不是合法的浮点数（目标 {ty:?}）")))
}

fn int_in_range<Ty>(n: &str, ty: &T) -> Result<Ty, Error>
where
    Ty: TryFrom<i64>,
{
    let parsed = n
        .parse::<i64>()
        .map_err(|_| bad(format!("列表元素 {n:?} 不是 {ty:?} 能装的整数")))?;
    Ty::try_from(parsed).map_err(|_| {
        bad(format!(
            "列表元素 {n:?} 超出目标类型 {ty:?} 的范围（不截断）"
        ))
    })
}

fn uint_in_range<Ty>(n: &str, ty: &T) -> Result<Ty, Error>
where
    Ty: TryFrom<u64>,
{
    let parsed = n
        .parse::<u64>()
        .map_err(|_| bad(format!("列表元素 {n:?} 不是 {ty:?} 能装的无符号整数")))?;
    Ty::try_from(parsed).map_err(|_| {
        bad(format!(
            "列表元素 {n:?} 超出目标类型 {ty:?} 的范围（不截断）"
        ))
    })
}

/// 按目标类型 `FromStr`：`jiff` 的时间族、`cidr`/`macaddr` 的网络地址族都用它。
fn parse_text<Ty>(s: &str, ty: &T) -> Result<Ty, Error>
where
    Ty: FromStr,
{
    s.parse::<Ty>()
        .map_err(|_| bad(format!("列表元素 {s:?} 不是合法的 {ty:?} 文本形态")))
}

fn unsupported(what: impl AsRef<str>) -> Error {
    Error::unsupported_feature(format!("达梦驱动不支持 {}", what.as_ref()))
}

fn bad(what: impl AsRef<str>) -> Error {
    Error::serialization_failure(format!("达梦列表列编解码：{}", what.as_ref()))
}

// ---------------------------------------------------------------------------
// 最小 JSON 读取器：只认「标量组成的数组」
// ---------------------------------------------------------------------------

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.text.len()
    }

    /// 报错用：从当前位置起最多 32 个字符（报错文本别把整列灌进日志）。
    fn remaining_head(&self) -> &'a str {
        let start = self.pos.min(self.text.len());
        let rest = &self.text[start..];
        match rest.char_indices().nth(32) {
            Some((end, _)) => &rest[..end],
            None => rest,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), Error> {
        match self.peek() {
            Some(b) if b == expected => {
                self.pos += 1;
                Ok(())
            }
            Some(b) => Err(bad(format!(
                "列表文本第 {} 字节期望 {:?}，实际是 {:?}",
                self.pos, expected as char, b as char
            ))),
            None => Err(bad(format!(
                "列表文本在期望 {:?} 处提前结束",
                expected as char
            ))),
        }
    }

    fn parse_array(&mut self, elem: &T) -> Result<Vec<V>, Error> {
        self.skip_ws();
        self.expect(b'[')?;

        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(items);
        }

        loop {
            let element = self.parse_element()?;
            items.push(element_to_value(element, elem)?);

            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                Some(b) => {
                    return Err(bad(format!(
                        "列表文本第 {} 字节期望 ',' 或 ']'，实际是 {:?}",
                        self.pos, b as char
                    )));
                }
                None => return Err(bad("列表文本在数组闭合前结束")),
            }
        }

        Ok(items)
    }

    fn parse_element(&mut self) -> Result<Elem, Error> {
        match self.peek() {
            Some(b'"') => Ok(Elem::Str(self.parse_string()?)),
            Some(b'n') => {
                self.parse_keyword("null")?;
                Ok(Elem::Null)
            }
            Some(b't') => {
                self.parse_keyword("true")?;
                Ok(Elem::Bool(true))
            }
            Some(b'f') => {
                self.parse_keyword("false")?;
                Ok(Elem::Bool(false))
            }
            Some(b) if b == b'-' || b.is_ascii_digit() => Ok(Elem::Number(self.parse_number()?)),
            // 嵌套数组 / 对象：诚实拒绝（列表列只存标量 JSON 数组）。
            Some(b'[') | Some(b'{') => Err(unsupported(
                "列表列里的嵌套数组 / 对象元素（只支持标量 JSON 数组）",
            )),
            Some(b) => Err(bad(format!(
                "列表文本第 {} 字节出现意外字符 {:?}",
                self.pos, b as char
            ))),
            None => Err(bad("列表文本在元素处提前结束")),
        }
    }

    fn parse_keyword(&mut self, keyword: &str) -> Result<(), Error> {
        if self.text[self.pos..].starts_with(keyword) {
            self.pos += keyword.len();
            Ok(())
        } else {
            Err(bad(format!(
                "列表文本第 {} 字节期望字面量 {keyword:?}",
                self.pos
            )))
        }
    }

    /// 数字按原文取出（只做「是不是合法 JSON 数字语法」的检查）。
    fn parse_number(&mut self) -> Result<String, Error> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }

        let digits_start = self.pos;
        while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(bad(format!("列表文本第 {start} 字节起的数字没有整数部分")));
        }

        if self.peek() == Some(b'.') {
            self.pos += 1;
            let frac_start = self.pos;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(bad(format!(
                    "列表文本第 {} 字节的小数点后没有数字",
                    self.pos
                )));
            }
        }

        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == exp_start {
                return Err(bad(format!("列表文本第 {} 字节的指数没有数字", self.pos)));
            }
        }

        Ok(self.text[start..self.pos].to_owned())
    }

    /// 字符串字面量：支持 `\" \\ \/ \b \f \n \r \t` 与 `\uXXXX`（含代理对）。
    fn parse_string(&mut self) -> Result<String, Error> {
        self.expect(b'"')?;
        let mut out = String::new();

        loop {
            let Some(b) = self.peek() else {
                return Err(bad("列表文本里的字符串没有闭合引号"));
            };

            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let Some(esc) = self.peek() else {
                        return Err(bad("列表文本里的转义序列未写完"));
                    };
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.parse_hex4()?;
                            let ch = if (0xd800..0xdc00).contains(&first) {
                                // 高代理：必须紧跟一个低代理。
                                self.expect(b'\\')?;
                                self.expect(b'u')?;
                                let second = self.parse_hex4()?;
                                if !(0xdc00..0xe000).contains(&second) {
                                    return Err(bad(format!(
                                        "列表文本里的代理对不完整（第二个 \\u{second:04x}）"
                                    )));
                                }
                                let combined =
                                    0x1_0000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                                char::from_u32(combined)
                                    .ok_or_else(|| bad("列表文本里的代理对不是合法码点"))?
                            } else {
                                char::from_u32(first).ok_or_else(|| {
                                    bad(format!("列表文本里的 \\u{first:04x} 不是合法码点"))
                                })?
                            };
                            out.push(ch);
                        }
                        other => {
                            return Err(bad(format!("列表文本里的未知转义 \\{}", other as char)));
                        }
                    }
                }
                _ => {
                    // 非 ASCII 走字符边界，避免把多字节序列切坏。
                    let Some(ch) = self.text[self.pos..].chars().next() else {
                        return Err(bad("列表文本里的字符串没有闭合引号"));
                    };
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, Error> {
        let end = self.pos + 4;
        if end > self.text.len() {
            return Err(bad("列表文本里的 \\u 转义不足 4 位"));
        }
        let digits = &self.text[self.pos..end];
        let value = u32::from_str_radix(digits, 16)
            .map_err(|_| bad(format!("\\u{digits} 不是十六进制")))?;
        self.pos = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_ty(elem: T) -> T {
        T::List(Box::new(elem))
    }

    #[test]
    fn strings_round_trip_including_escapes() {
        let items = vec![
            V::String("admin".to_owned()),
            V::String("带中文与引号\"和反斜杠\\".to_owned()),
            V::String("换行\n制表\t".to_owned()),
            V::Null,
        ];
        let text = encode(&items).unwrap();
        assert_eq!(
            text,
            "[\"admin\",\"带中文与引号\\\"和反斜杠\\\\\",\"换行\\n制表\\t\",null]"
        );
        let back = decode(&text, &list_ty(T::String)).unwrap();
        assert_eq!(back, items);
    }

    #[test]
    fn integers_keep_their_json_shape() {
        let items = vec![V::I64(1), V::I8(-8), V::U32(u32::MAX)];
        let text = encode(&items).unwrap();
        assert_eq!(text, "[1,-8,4294967295]");
        assert_eq!(
            decode(&text, &list_ty(T::I64)).unwrap(),
            vec![V::I64(1), V::I64(-8), V::I64(4_294_967_295)]
        );
    }

    #[test]
    fn integers_out_of_range_are_refused_not_truncated() {
        let err = decode("[300]", &list_ty(T::I8)).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
        let err = decode("[-1]", &list_ty(T::U8)).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
    }

    #[test]
    fn floats_always_carry_a_fraction() {
        // 1.0 若写成 `1`，达梦的 JSON_CONTAINS 会按整数比（探针 17ao）⇒ 必须带 `.0`。
        assert_eq!(encode(&[V::F64(1.0)]).unwrap(), "[1.0]");
        assert_eq!(encode(&[V::F32(0.5)]).unwrap(), "[0.5]");
        assert_eq!(
            decode("[1.0]", &list_ty(T::F64)).unwrap(),
            vec![V::F64(1.0)]
        );
    }

    #[test]
    fn non_finite_floats_are_refused() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = encode(&[V::F64(v)]).unwrap_err();
            assert!(err.is_unsupported_feature(), "实际：{err}");
        }
    }

    /// `Decimal` 的原样保留：不进 f64、不丢尾数。
    ///
    /// 有效数字上限是 `rust_decimal` 的 28 位（列宽 `DECIMAL(38, 18)` 更大，但驱动手上只有
    /// 这 28 位）⇒ 这里就用 28 位有效数字的样本钉住「原文搬运」。
    #[test]
    fn decimals_keep_full_precision() {
        let text_in = "12345678901234567890.12345678";
        let d = rust_decimal::Decimal::from_str(text_in).unwrap();
        let text = encode(&[V::Decimal(d)]).unwrap();
        assert_eq!(text, format!("[{text_in}]"));
        assert_eq!(
            decode(&text, &list_ty(T::Decimal)).unwrap(),
            vec![V::Decimal(d)]
        );
    }

    #[test]
    fn uuid_uses_canonical_text() {
        let id = uuid::Uuid::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        let text = encode(&[V::Uuid(id)]).unwrap();
        assert_eq!(text, "[\"01020304-0506-0708-090a-0b0c0d0e0f10\"]");
        assert_eq!(decode(&text, &list_ty(T::Uuid)).unwrap(), vec![V::Uuid(id)]);
    }

    #[test]
    fn time_values_round_trip_as_text() {
        let ts: jiff::Timestamp = "2026-01-02T03:04:05.678901Z".parse().unwrap();
        let d: jiff::civil::Date = "2026-01-02".parse().unwrap();
        let t: jiff::civil::Time = "03:04:05.678".parse().unwrap();
        let z: jiff::Zoned = "2026-01-02T03:04:05-05:00[America/New_York]"
            .parse()
            .unwrap();

        for (value, ty) in [
            (V::Timestamp(ts), T::Timestamp),
            (V::Date(d), T::Date),
            (V::Time(t), T::Time),
            (V::Zoned(z), T::Zoned),
        ] {
            let text = encode(std::slice::from_ref(&value)).unwrap();
            assert_eq!(
                decode(&text, &list_ty(ty.clone())).unwrap(),
                vec![value],
                "目标类型 {ty:?}"
            );
        }
    }

    #[test]
    fn nested_collections_and_bytes_are_refused() {
        let err = encode(&[V::List(vec![V::I64(1)])]).unwrap_err();
        assert!(err.is_unsupported_feature(), "实际：{err}");
        let err = encode(&[V::Bytes(vec![1, 2, 3])]).unwrap_err();
        assert!(err.is_unsupported_feature(), "实际：{err}");
        let err = decode("[[1]]", &list_ty(T::I64)).unwrap_err();
        assert!(err.is_unsupported_feature(), "实际：{err}");
    }

    #[test]
    fn malformed_text_is_refused() {
        for text in ["", "[", "[1", "[1,", "[\"a]", "[1 2]", "[]]", "[01x]"] {
            assert!(
                decode(text, &list_ty(T::I64)).is_err(),
                "应当拒绝：{text:?}"
            );
        }
        // 尾部多余内容也要拒绝，不能只看前缀。
        assert!(decode("[] []", &list_ty(T::I64)).is_err());
    }

    #[test]
    fn empty_list_round_trips() {
        assert_eq!(encode(&[]).unwrap(), "[]");
        assert!(decode("[]", &list_ty(T::String)).unwrap().is_empty());
        assert!(decode(" [ ] ", &list_ty(T::String)).unwrap().is_empty());
    }

    #[test]
    fn unicode_escapes_are_understood() {
        assert_eq!(
            decode("[\"\\u4e2d\\u6587\"]", &list_ty(T::String)).unwrap(),
            vec![V::String("中文".to_owned())]
        );
        // 代理对：U+1F600。
        assert_eq!(
            decode("[\"\\ud83d\\ude00\"]", &list_ty(T::String)).unwrap(),
            vec![V::String("😀".to_owned())]
        );
    }

    #[test]
    fn element_type_mismatch_is_refused() {
        // 字符串元素要还原成整数 ⇒ 报错（不 parse、不猜）。
        let err = decode("[\"12\"]", &list_ty(T::I64)).unwrap_err();
        assert!(err.is_serialization_failure(), "实际：{err}");
        // 嵌套列表元素要还原成字符串 ⇒ 报不支持。
        let err = decode("[[1]]", &list_ty(T::String)).unwrap_err();
        assert!(err.is_unsupported_feature(), "实际：{err}");
    }
}
