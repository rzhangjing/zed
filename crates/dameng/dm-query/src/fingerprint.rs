//! SQL 指纹：进日志的 SQL 去掉字面量、**不含参数值**（设计方案 §6.9 / T5.4）。
//!
//! # 为什么要指纹
//!
//! 日志里出现原始 SQL 会带来两个问题：一是把业务数据（订单号、身份证号）写进日志，
//! 二是同一个语句每次执行的字面量都不同，无法聚合统计。指纹做的是：
//!
//! ```text
//!   SELECT * FROM t WHERE id = 42 AND name = 'it''s'
//!   → select * from t where id = ? and name = ?
//! ```
//!
//! 归一化后：字面量变成 `?`，未加引号的标识符与关键字折叠成小写（`SELECT`/`select`
//! 得到同一个指纹），空白折叠成单个空格，注释丢弃。**加了双引号的标识符保持原样**
//! （它们在达梦里是大小写敏感的，不能折叠，否则两个不同的表会被算成一个指纹）。
//!
//! 参数值本身**从不进入**指纹——它们是 `?` 占位符或绑定值，与形状无关。

use std::fmt;

/// 归一化结果的长度上限（超长的 SQL 只保留前一段，避免日志被撑爆）。
const MAX_NORMALIZED_CHARS: usize = 1024;

/// 日志里展示归一化 SQL 的截断长度。
const DISPLAY_CHARS: usize = 96;

/// 把一个 SQL 归一化成「形状」（去掉字面量、注释，折叠大小写与空白）。
///
/// 这个函数是**幂等**的：`normalize(&normalize(sql)) == normalize(sql)`。
#[must_use]
pub fn normalize(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    // 空白**延后兑现**：只有真的来了下一个 token 才补一个空格，所以串尾/串首不留空格，
    // 也保证 `normalize` 幂等（不会越归一化越多空格）。
    let mut pending_space = false;
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        match ch {
            // 字符串字面量：'...'，内部 '' 表示一个单引号。
            '\'' => {
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\'' {
                        if chars.get(i + 1) == Some(&'\'') {
                            i += 2; // 转义的单引号
                            continue;
                        }
                        i += 1; // 收尾引号
                        break;
                    }
                    i += 1;
                }
                push_token(&mut out, &mut pending_space, "?");
            }
            // 双引号标识符：保持原样（达梦里大小写敏感）。
            '"' => {
                let mut token = String::from('"');
                i += 1;
                while i < chars.len() {
                    let c = chars[i];
                    token.push(c);
                    i += 1;
                    if c == '"' {
                        if chars.get(i) == Some(&'"') {
                            token.push('"');
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
                push_token(&mut out, &mut pending_space, &token);
            }
            // 行注释：-- 直到行尾。
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                pending_space = true;
            }
            // 块注释：/* ... */。
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() {
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                pending_space = true;
            }
            // 数字字面量（含 0x 十六进制与 1.5e-3 这类科学计数法）。
            c if c.is_ascii_digit() => {
                i = skip_number(&chars, i);
                push_token(&mut out, &mut pending_space, "?");
            }
            // 标识符/关键字：折叠成小写。
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let start = i;
                while i < chars.len() {
                    let c = chars[i];
                    if c.is_alphanumeric() || c == '_' || c == '$' || c == '#' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let token: String = chars[start..i]
                    .iter()
                    .flat_map(|c| c.to_lowercase())
                    .collect();
                push_token(&mut out, &mut pending_space, &token);
            }
            c if c.is_whitespace() => {
                i += 1;
                pending_space = true;
            }
            c => {
                i += 1;
                push_token(&mut out, &mut pending_space, &c.to_string());
            }
        }

        if out.chars().count() >= MAX_NORMALIZED_CHARS {
            return out.chars().take(MAX_NORMALIZED_CHARS).collect();
        }
    }

    out
}

/// 跳过一段数字字面量，返回下一个待处理下标。
fn skip_number(chars: &[char], mut i: usize) -> usize {
    if chars[i] == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
        i += 2;
        while i < chars.len() && chars[i].is_ascii_hexdigit() {
            i += 1;
        }
        return i;
    }
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if chars.get(i) == Some(&'.') {
        // 只在点号后面还跟着数字时才算小数（`1.` 后的标识符不从点号开始）。
        if chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    if matches!(chars.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        if matches!(chars.get(j), Some('+' | '-')) {
            j += 1;
        }
        if chars.get(j).is_some_and(char::is_ascii_digit) {
            i = j;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}

/// 追加一个 token：先把「待兑现的空白」补上（若串还不为空），再写 token。
fn push_token(out: &mut String, pending_space: &mut bool, token: &str) {
    if token.is_empty() {
        return;
    }
    if *pending_space {
        if !out.is_empty() {
            out.push(' ');
        }
        *pending_space = false;
    }
    out.push_str(token);
}

/// 一段 SQL 的指纹：归一化文本 + 64 位哈希。
///
/// `Debug`/`Display` 都只给出「哈希 + 截断后的归一化 SQL」，可以安全进日志。
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    normalized: String,
    hash: u64,
}

impl Fingerprint {
    /// 计算一段 SQL 的指纹。
    #[must_use]
    pub fn of(sql: &str) -> Self {
        let normalized = normalize(sql);
        let hash = fnv1a64(normalized.as_bytes());
        Self { normalized, hash }
    }

    /// 归一化后的 SQL（字面量已变成 `?`，标识符已折叠成小写）。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    /// 64 位 FNV-1a 哈希。
    #[must_use]
    pub const fn hash(&self) -> u64 {
        self.hash
    }

    /// 16 位小写十六进制哈希（定长，适合做指标标签）。
    #[must_use]
    pub fn hex(&self) -> String {
        format!("{:016x}", self.hash)
    }

    /// 日志用的一行摘要：`<hex> <归一化 SQL（截断）>`。
    #[must_use]
    pub fn summary(&self) -> String {
        let mut text: String = self.normalized.chars().take(DISPLAY_CHARS).collect();
        if self.normalized.chars().count() > DISPLAY_CHARS {
            text.push('…');
        }
        format!("{} {}", self.hex(), text)
    }

    /// 归一化文本（所有权移交，供需要长期保存的调用方使用）。
    #[must_use]
    pub fn into_normalized(self) -> String {
        self.normalized
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fingerprint")
            .field("hash", &self.hex())
            .field("sql", &self.normalized)
            .finish()
    }
}

/// FNV-1a 64 位（不需要外部依赖，且跨进程稳定）。
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_literals_and_folds_case() {
        let a = Fingerprint::of("SELECT * FROM t WHERE id = 42 AND name = 'x'");
        let b = Fingerprint::of("select * from t where id = 7 and name = 'y'");
        assert_eq!(a.as_str(), "select * from t where id = ? and name = ?");
        assert_eq!(a.hash(), b.hash(), "同一形状必须同指纹");
    }

    #[test]
    fn keeps_quoted_identifiers_verbatim() {
        let a = Fingerprint::of(r#"select "CaseSensitive" from t"#);
        let b = Fingerprint::of(r#"select "casesensitive" from t"#);
        assert_eq!(a.as_str(), r#"select "CaseSensitive" from t"#);
        assert_ne!(a.hash(), b.hash(), "双引号标识符大小写敏感，不能折叠");
    }

    #[test]
    fn drops_comments() {
        let sql = "select 1 -- 尾注释\nfrom dual /* 块\n注释 */ where 2 = 2";
        assert_eq!(normalize(sql), "select ? from dual where ? = ?");
    }

    #[test]
    fn handles_numbers_and_escapes() {
        assert_eq!(
            normalize("select 1.5e-3, 0x1F, 42, 3."),
            "select ?, ?, ?, ?."
        );
        assert_eq!(normalize("select 'it''s' from dual"), "select ? from dual");
        assert_eq!(normalize("select 'unterminated"), "select ?");
    }

    #[test]
    fn collapses_whitespace_and_is_idempotent() {
        let once = normalize("  select\n\t*\r\nfrom   dual  ");
        assert_eq!(once, "select * from dual");
        assert_eq!(normalize(&once), once, "归一化必须幂等");
    }

    #[test]
    fn parameters_stay_as_placeholders() {
        assert_eq!(
            normalize("select * from t where a = ? and b = ?"),
            "select * from t where a = ? and b = ?"
        );
    }

    #[test]
    fn summary_is_bounded_and_hex_is_fixed_width() {
        let long = format!("select {} from dual", "a, ".repeat(200));
        let fp = Fingerprint::of(&long);
        assert_eq!(fp.hex().len(), 16);
        assert!(fp.summary().chars().count() <= DISPLAY_CHARS + 17 + 1);
        assert!(fp.summary().contains('…'));
    }

    #[test]
    fn distinct_shapes_have_distinct_hashes() {
        assert_ne!(
            Fingerprint::of("select a from t").hash(),
            Fingerprint::of("select b from t").hash()
        );
    }

    #[test]
    fn escaped_double_quotes_stay_inside_a_quoted_identifier() {
        // 达梦里 `"a""b"` 是一个标识符，里面的 `""` 是一个双引号。
        assert_eq!(
            normalize(r#"select "a""b" from t"#),
            r#"select "a""b" from t"#
        );
    }

    #[test]
    fn scientific_notation_and_hex_are_single_placeholders() {
        assert_eq!(normalize("select 1e5, 1E+7, 1e-2"), "select ?, ?, ?");
        assert_eq!(normalize("select 0XAB, 0x1f"), "select ?, ?");
    }

    #[test]
    fn an_over_long_sql_is_cut_at_the_cap() {
        let sql = "a ".repeat(2000);
        let out = normalize(&sql);
        assert_eq!(
            out.chars().count(),
            MAX_NORMALIZED_CHARS,
            "超长 SQL 必须截到长度上限，避免日志被撑爆"
        );
        assert!(out.starts_with("a a a "), "截断只砍尾巴：{out:?}");
    }

    #[test]
    fn display_debug_and_into_normalized_agree_with_the_shape() {
        let fp = Fingerprint::of("select 1 from dual");
        assert_eq!(fp.as_str(), "select ? from dual");
        assert_eq!(fp.hex().len(), 16, "哈希定长 16 位十六进制");
        assert_eq!(
            fp.summary(),
            format!("{} select ? from dual", fp.hex()),
            "摘要 = 定长哈希 + 截断的归一化 SQL"
        );
        assert_eq!(fp.to_string(), fp.summary(), "Display 就是日志摘要");

        let debug = format!("{fp:?}");
        assert!(debug.contains("Fingerprint"), "Debug 要能看出类型：{debug}");
        assert!(debug.contains(&fp.hex()), "Debug 里要有哈希：{debug}");

        assert_eq!(fp.into_normalized(), "select ? from dual");
    }
}
