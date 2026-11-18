//! SQL 脚本的分号切分（设计方案 §6.11 / §12.2 T7.1 / §15.7 CAP-G-01）。
//!
//! # 为什么切分器住在 `dm-migrate` 而不是 `dm-query`
//!
//! 「按分号切一条脚本里的多条语句」是**跑迁移脚本**这件事的性质，不是查询层的能力：
//! `PooledConn::execute` 的契约就是「一条语句」——参数化预处理的桥接路径只能送一条
//! 语句过去（达梦 ODBC 一次 `SQLExecDirect` 也只吃一条）。把切分塞进查询层会让
//! 「什么算一条语句」变成所有调用方都要面对的隐式规则；放在这里，它只被
//! [`crate::Migrator::run_sql_file`] 与 `apply_pending` 使用，语义边界清楚。
//!
//! # 一条语句的边界由什么决定
//!
//! 只有**词法层**的信息能决定边界（我们不做语法分析）：
//!
//! 1. **跳过不该看的地方**：`'…'` / `"…"` 引号串、`--` 行注释、`/* … */` 块注释里的
//!    分号一律不算分隔符（`'a;b'` 是一个值，不是两条语句）；
//! 2. **`BEGIN … END;` 块整体不切**：达梦/`PL-SQL` 里 `BEGIN` 之后的每个 `;` 都是
//!    块内部的语句分隔符，**块只由最后一个 `END;` 收尾**。切错了不是「多跑一条空语句」
//!    这种小事，而是把一个合法块拆成两半、两条都语法错。
//!
//! 第 2 条用「关键字计数器」而不是「正则匹配 `BEGIN.*END`」实现：正则处理不了嵌套
//! （函数体里套 `BEGIN`），也处理不了 `CASE … END`（它同样以 `END` 收尾，若不计入
//! 就会在 `CASE` 的 `END` 上提前把块的深度减到 0，于是在 `CASE` 之后的分号上误切）。
//!
//! # 状态机的两个维度
//!
//! | 维度 | 取值 | 作用 |
//! |---|---|---|
//! | [`SplitState`] | `Normal` / `InSingle` / `InDouble` / `InLineComment` / `InBlockComment` / `InQuoted` | 当前这段字节**属于谁**（代码、字符串、注释、`q'…'` 串） |
//! | `depth` | `usize` | 已经进入了几层「以 `END` 收尾」的复合结构（`BEGIN` / `CASE` / `LOOP`） |
//! | `blocks` | `usize` | 语句体内已经开了几个 `BEGIN`（用来区分 `CASE` 该不该自己加一层，见 [`settle_end`]） |
//!
//! 每扫完一个字节，只有在 `state == Normal` **且** `depth == 0`（**且**没有待定的
//! `END`）时遇到的那个 `;` 才算语句结束。
//!
//! # 已知边界（如实列出，别当已支持）
//!
//! - **`BEGIN` 前面不能出现过别的有效词**才算块开始（前面的空白与注释不算）。
//!   注释是词法透明的：`/* c */ begin … end;` **仍然是块**（与 Oracle 一致）。
//! - **`BEGIN TRANSACTION` 会被当成块**：达梦的 `BEGIN TRANSACTION` / `BEGIN TRAN`
//!   这类事务控制语句也以 `BEGIN` 开头，词法上分不出来。**实测行为**：脚本
//!   `begin transaction; select 1 from dual;` 会整体作为**一条**语句发出，由服务端
//!   报语法错（而不是我们静默切成两条）。迁移脚本里不要用 `BEGIN TRANSACTION`。
//! - **块注释不嵌套**（`/* a /* b */ c */` 在第一个 `*/` 就结束，与 SQL 标准一致）。
//! - `q'…'` 只认**单字符**定界符（`q'[…]'`、`q'{…}'`、`q'(…)'`、`q'<…>'`、`q'X…X'`）；
//!   达梦的 `q'{…}'` 形式里定界符本身也是单字符，所以这条覆盖了实测到的全部形式。
//! - `END` **只在 `depth > 0` 时**才减深度：`select x end from t` 这种把 `END` 当普通
//!   标识符用的句子不会把深度减成负数（减成负数会让后面的每个 `;` 都被当成块内）。
//! - 关键字识别要求「前一个字节不是标识符字符」，所以 `APPEND` / `mybegin` 不会被误认。
//! - 字符串/注释里的**多字节字符按 `char` 处理**（[`push_char_at`]）：曾按字节
//!   `push(b as char)`，实测把 `中文` 拆成 `ä¸­æ\u{96}\u{87}` 发给了服务端。

/// 切分脚本时「当前字节属于谁」。
///
/// 六个状态各自对应一种**需要原样吞掉分号**的区域；回到 [`SplitState::Normal`] 之后
/// 分号才重新有意义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplitState {
    /// 普通代码。
    Normal,
    /// `'…'` 字符串字面量（`''` 是内部转义的引号，不是字符串结束）。
    InSingle,
    /// `"…"` 引号标识符（`""` 同理）。
    InDouble,
    /// `-- …` 行注释，到行尾结束。
    InLineComment,
    /// `/* … */` 块注释，到第一个 `*/` 结束（不嵌套）。
    InBlockComment,
    /// `q'X…X'` 替代引号串，`X` 是单字符定界符（`[`/`{`/`(`/`<` 会配对到对应的闭括号）。
    InQuoted(char),
}

/// 语句**句首**到目前为止长什么样。
///
/// `Head::Other` 时不再把 `begin` 认成块开始：`insert …; begin …` 里那第二个
/// `begin` 是**新语句**（分号已经把 `head` 重置成 [`Head::Start`]），而同一句里
/// 出现过的别的词（`select begin …`）说明这个 `begin` 不是语句头一个词。
///
/// ⚠️ 注释与空白**不改** `head`（词法透明），所以 `/* c */ begin … end;` 仍然是块；
/// 代价是 `begin transaction; select …;` 也会被当成块整体发出——词法上分不出来，
/// 交给服务端报语法错（见模块文档「已知边界」）。
///
/// `EndSeen` 取代了以前那个 `pending_end` 布尔量：`END` 后面可能还有东西
/// （`END IF`、`END LOOP`、`END /* c */`），深度必须等到**下一个有效字符**出现
/// 甚至**一直等到脚本结束**才能结算——否则 `BEGIN IF x THEN … END IF; END;` 会在
/// 第一个 `END` 上就把深度减到 0，然后在 `end if;` 的分号上误切。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Head {
    /// 句首还没出现有效字符。
    Start,
    /// 句首只有空白与左括号。
    Paren,
    /// 句首已经出现过别的内容。
    Other,
    /// 当前语句以 `END` 结尾（块结束），还没结算。
    EndSeen,
}

/// `q'…'` 的闭定界符。
///
/// 括号类开定界符允许按「配对括号」收尾；其余定界符只有一种收尾方式。
/// 两种都收是**放宽**而不是收紧：真写错时服务端照样会以语法错拒绝。
fn closing_delim(open: char) -> [char; 2] {
    match open {
        '[' => [']', '['],
        '{' => ['}', '{'],
        '(' => [')', '('],
        '<' => ['>', '<'],
        other => [other, other],
    }
}

/// 切分结果里的一条语句（已 trim，保证非空）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Statement {
    /// 语句文本，原样保留注释（只去掉了首尾空白）。
    pub(crate) sql: String,
}

/// 按分号把脚本切成语句。
///
/// 返回的每条语句都已**去掉首尾空白**，且**不包含**分隔用的分号；**只有注释或空白的
/// 片段一律丢弃**（`;;`、空脚本、脚本末尾的尾注释都不会产出语句）——它们不是能发给
/// 服务端的语句。跟在真代码后面的注释则原样留在该条语句里。
///
/// 文本不会被改写（注释与换行原样进 `sql`），这样报错时用户能拿原始行号对照。
pub(crate) fn split_statements(script: &str) -> Vec<Statement> {
    let bytes = script.as_bytes();
    let len = bytes.len();

    let mut out: Vec<Statement> = Vec::new();
    let mut code = String::new();
    let mut state = SplitState::Normal;
    // 块深度：`BEGIN` 与 `CASE` 都 +1、`END` -1，所以 `;` 只在深度 0 处切分。
    let mut depth: usize = 0;
    // **语句体内已经开了几个 PL/SQL 块**（`BEGIN` 算）。
    //
    // 为什么不能只靠 `depth` 判断 `CASE` 要不要 +1：`depth` 把 `BEGIN` 与 `CASE`
    // 混在一起数，块体里的裸 `END` 无法区分自己收的是哪一个。**实测的 bug**：
    // `BEGIN CASE … END; END;` 在 `CASE` 的 `END` 处把块深度也减到 0，
    // 于是在 `END;` 的分号上把外层块切开了。
    // 有了这个计数就能区分：`blocks > 0` 时 `CASE` 与 `BEGIN` 共用一个 `END`
    // （不 +1），只有**表达式位置**的 `CASE`（`blocks == 0`）才需要自己 +1。
    let mut blocks: usize = 0;

    // 语句**句首**到目前为止长什么样：只有 [`Head::Start`] / [`Head::Paren`] 时
    // 才可能把 `begin` 认成块开始（见模块文档「已知边界」）；`Head::EndSeen`
    // 表示深度还欠一次「块结束」没结算。
    let mut head = Head::Start;
    // `q'…'` 串当前用的开定界符。单独存一份而不是从缓冲区里反查：定界符可能
    // 是多字节字符，按 `char` 反查既脆又绕。
    let mut quoted_open = '\'';

    // 当前片段里**有没有真代码**：注释与空白不算。只有真代码才值得作为一条语句
    // 交给服务端——整段只有注释的片段（空脚本、脚本末尾的尾注释）必须**丢掉**，
    // 否则会往服务端发一条纯注释的「语句」。
    let mut has_code = false;

    let mut i = 0usize;
    while i < len {
        let b = bytes[i];

        if state == SplitState::Normal {
            match b {
                b';' => {
                    // 先看还欠不欠结算：
                    // - `head == EndSeen`：`END IF` / `END LOOP` 这种「块体收尾词」的
                    //   尾词（`if`/`loop`）不像 `END` 那样会自己结算，欠账会一直挂到
                    //   这个 `;`——**必须在这里结算**，否则外层块的账目永远收不掉。
                    //   （`END` 后面直接跟 `;` 的情况在 `end` 分支里已经结算过了。）
                    // - `depth == 0`：语句已经在块外，欠账早就该结清（防御性）。
                    // 判据用 `blocks`（**还有几个 `BEGIN`/`CASE` 块没收到 `END`**）
                    // 而不是 `depth`：`depth` 只在这里被读来防呆。
                    //
                    // **实测的 bug**：早先 `;` 分支只看 `depth == 0`，`END IF;` 的
                    // `END` 欠账挂到 `end;` 才结算，外层块的那个 `;` 因此被当成语句边界。
                    if head == Head::EndSeen || depth == 0 {
                        settle_end(&mut head, &mut depth, &mut blocks);
                    }
                    if blocks == 0 {
                        push_statement(&mut out, &mut code, &mut has_code);
                        head = Head::Start;
                    } else {
                        // 块内分隔符：原样保留，且句首状态**不清零**
                        // ——外层块还没收尾，后面的语句仍然在块里。
                        code.push(';');
                    }
                    i += 1;
                    continue;
                }
                b'-' if bytes.get(i + 1) == Some(&b'-') => {
                    // 注释不算「有效字符」：`begin -- 说明\n … end;` 仍然是一个块。
                    state = SplitState::InLineComment;
                    code.push_str("--");
                    i += 2;
                    continue;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    state = SplitState::InBlockComment;
                    code.push_str("/*");
                    i += 2;
                    continue;
                }
                b'\'' => {
                    // 字符串是表达式的一部分，会撤销「句首」状态。
                    head = Head::Other;
                    has_code = true;
                    state = SplitState::InSingle;
                    code.push('\'');
                    i += 1;
                    continue;
                }
                b'"' => {
                    head = Head::Other;
                    has_code = true;
                    state = SplitState::InDouble;
                    code.push('"');
                    i += 1;
                    continue;
                }
                b'q' | b'Q'
                    if bytes.get(i + 1) == Some(&b'\'')
                        && !prev_is_ident_byte(bytes, i)
                        && bytes.get(i + 2).is_some() =>
                {
                    let open = bytes[i + 2] as char;
                    head = Head::Other;
                    has_code = true;
                    quoted_open = open;
                    state = SplitState::InQuoted(open);
                    code.push(char::from(b));
                    code.push('\'');
                    code.push(open);
                    i += 3;
                    continue;
                }
                _ if is_blank(b) => {
                    // 空白不改句首状态，也不结算 `END`：
                    // `begin\n  insert …；` 里 `begin` 与 `insert` 之间的换行不算内容。
                    //
                    // 非 ASCII 字节可能是 UTF-8 多字节字符（中文注释/中文字面量）的首字节，
                    // 必须**按整个 `char`** 复制（见 `push_char_at`）。
                    i = push_char_at(bytes, i, &mut code);
                    continue;
                }
                b'(' => {
                    // `( begin … )`：句首的左括号不撤销「句首」。
                    settle_end(&mut head, &mut depth, &mut blocks);
                    if head == Head::Start {
                        head = Head::Paren;
                    }
                    code.push('(');
                    i += 1;
                    continue;
                }
                _ => {}
            }

            // ⚠️ 这里**不能**再无条件结算一次「欠着的 END」：`END IF` / `END LOOP`
            // 的尾词（`if`/`loop`）是块体收尾词，它本身不是有效字符意义上的「内容」，
            // 一旦在这里把欠账结掉，`END IF;` 就会提前把**外层块**收尾
            // （**实测的 bug**：`begin if … end if; end;` 被切成 3 条）。
            // `END` 的结算只发生在两处：`end` 分支里（账上恰好只剩它一个时）和
            // `;` 分支里（`END IF` / `END LOOP` 的尾词之后、语句结束的那个 `;`）。

            // ⚠️ `begin` 的处理必须放在**这里**（有效字符的入口），不能放进上面的
            // `match` 里再 `continue`：`begin` 的游标要推到词尾（`i += 5`），那样下一次
            // 循环就从关键字后面那个字节开始了——如果那正好是个 `;`，`;` 分支会先执行
            // 并且当时那个 `pending_begin` 标记会被顺手清掉，于是「是不是块」的判定
            // **一次都不会跑**，`begin` 既没进缓冲区也没记 `has_code`，整条语句凭空消失
            // （实测：`begin; begin; select 1 from dual;` 只剩 `["select 1 from dual"]`）。
            // 放在这里则关键字、块判定、缓冲区三者一次完成，且**绝不越过关键字**。
            //
            // 前面必须不是标识符字节：`mybegin` 不是关键字。
            //
            // ⚠️ 这里的判据是「**句首**」而不是「缓冲区里还没东西」：`BEGIN BEGIN …`
            // 这种嵌套块里，外层 `begin` 已经写进缓冲区了，但内层 `begin` **仍然是
            // 它自己的句首**——它前面只有空白。所以块开始后必须把句首状态复位成
            // [`Head::Start`]（而不是 `Other`），否则内层块根本不计数。
            //
            // **实测的 bug**：早先这里写 `head = Head::Other` + 条件 `head != Head::Other`，
            // 于是 `BEGIN BEGIN … END; … END;` 里外层块占了深度 1、内层块没数，
            // 内层的 `END;` 就把深度减到 0 并提前切开（切成 3 条）。
            if !prev_is_ident_byte(bytes, i)
                && starts_with_ci(bytes, i, b"begin")
                && !matches!(head, Head::Other | Head::EndSeen)
            {
                let is_block = inspect_begin(bytes, &mut code, &mut has_code, i + 5);
                if is_block {
                    // 块体马上就要来了：句首复位、块深度 +1（内层 `begin` 靠这一行才数得上）。
                    // `begin;` 这种普通语句则两样都不动——`begin;` 后面**紧跟**再来一个
                    // `begin … end;` 时，第二个 `begin` 仍然是语句的第一个词。
                    head = Head::Start;
                    depth += 1;
                    blocks += 1;
                }
                i += 5;
                continue;
            }
            if head != Head::Start {
                head = Head::Other;
            }

            // 关键字只在「前一个字节不是标识符字符」时才认（`APPEND`/`mybegin` 不算）。
            if !prev_is_ident_byte(bytes, i) {
                if starts_with_ci(bytes, i, b"end") {
                    // 先记账、后往前看：`END IF` / `END LOOP` / `END CASE` 是
                    // **块体内部**的收尾，不能减块深度。
                    i += 3;
                    let body_closer = end_is_body_closer(bytes, i);
                    head = Head::EndSeen;
                    code.push_str("end");
                    has_code = true;
                    // `END` 结算的条件：不是块体的收尾词（`END IF` 等），且账上
                    // **恰好还剩一个**待收的块（`blocks == 1`）；
                    //
                    // ⚠️ 但同时必须 `depth == 1`：`blocks == 1` 只说明「块账上还有一个
                    // `BEGIN` 没收」，并不说明**这个 `END` 就是冲它来的**——
                    // 块体里的 `v := case … end` 也会走到这里（`blocks` 仍是 1），
                    // 它收的是**表达式**，`depth` 比 `blocks` 多出来的那 1 就是它的。
                    // **实测的 bug**：只看 `blocks == 1` 时，`BEGIN v := case … end; ` 的
                    // `CASE` 收尾把块账目减掉，那个 `;` 就被当成语句边界（切成 3 条）。
                    if !body_closer && blocks == 1 && depth == 1 {
                        settle_end(&mut head, &mut depth, &mut blocks);
                    }
                    continue;
                }
                if starts_with_ci(bytes, i, b"case") {
                    // `CASE … END` 自成一对，和 `BEGIN … END` 一样要记账：
                    // `depth += 1`（这样它里面的 `;` 不会被当成分隔符），
                    // `blocks += 1`（这样它的 `END` 有机会被 [`settle_end`] 结算，
                    // 因为 `END` 的结算条件是「`blocks == 1` 且 `depth == 1`」）。
                    //
                    // **实测的 bug**：早先写成 `if blocks == 0 { depth += 1 }`
                    // ——句首判断让块体里的 `case` 永远不进这里的 `blocks == 0` 分支，
                    // 于是它的 `END` 把**块**的账目给结算掉了，
                    // `BEGIN v := case … end;` 在那个 `;` 上被切成 3 条。
                    depth += 1;
                    blocks += 1;
                    code.push_str("case");
                    has_code = true;
                    i += 4;
                    continue;
                }
                if starts_with_ci(bytes, i, b"loop") {
                    head = Head::Other;
                    depth += 1;
                    blocks += 1;
                    code.push_str("loop");
                    has_code = true;
                    i += 4;
                    continue;
                }
            }

            head = Head::Other;
            code.push(char::from(b));
            has_code = true;
            i += 1;
            continue;
        }

        // 非 Normal 状态：逐个字符吞掉，只在遇到本状态的收尾标记时切回 Normal。
        //
        // ⚠️ 复制文本一律走 `push_char_at`（**按 `char` 而不是按字节**）：按字节
        // `push(b as char)` 会把中文拆成乱码（见 `push_char_at` 的文档）。
        //
        // 这一整段都**不设 `has_code`**：注释不算「真代码」，而字符串属于表达式的一部分
        // ——所以 `InSingle`/`InDouble`/`InQuoted` 的入口（上面的 `match b` 分支）
        // 已经设过了。
        match state {
            SplitState::InSingle => {
                if b == b'\'' {
                    if bytes.get(i + 1) == Some(&b'\'') {
                        code.push_str("''");
                        i += 2;
                        continue;
                    }
                    state = SplitState::Normal;
                    code.push('\'');
                    i += 1;
                    continue;
                }
                i = push_char_at(bytes, i, &mut code);
            }
            SplitState::InDouble => {
                if b == b'"' {
                    if bytes.get(i + 1) == Some(&b'"') {
                        code.push_str("\"\"");
                        i += 2;
                        continue;
                    }
                    state = SplitState::Normal;
                    code.push('"');
                    i += 1;
                    continue;
                }
                i = push_char_at(bytes, i, &mut code);
            }
            SplitState::InLineComment => {
                i = push_char_at(bytes, i, &mut code);
                if b == b'\n' {
                    state = SplitState::Normal;
                }
            }
            SplitState::InBlockComment => {
                if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    code.push_str("*/");
                    i += 2;
                    state = SplitState::Normal;
                    continue;
                }
                i = push_char_at(bytes, i, &mut code);
            }
            SplitState::InQuoted(_) => {
                // 达梦/PL-SQL 的 `q'…'` 没有转义：定界符本身不能出现在串里。
                let closes = closing_delim(quoted_open);
                if b == closes[0] as u8 || b == closes[1] as u8 {
                    code.push(char::from(b));
                    i += 1;
                    if bytes.get(i) == Some(&b'\'') {
                        code.push('\'');
                        i += 1;
                    }
                    state = SplitState::Normal;
                    continue;
                }
                i = push_char_at(bytes, i, &mut code);
            }
            SplitState::Normal => unreachable!("Normal 分支上面已经处理"),
        }
    }

    // 收尾：脚本末尾没有分号时，最后一段也是完整的一条语句（但只有注释的尾段不算）。
    settle_end(&mut head, &mut depth, &mut blocks);
    push_statement(&mut out, &mut code, &mut has_code);
    out
}

/// 结算「刚才看到的 `END` 是不是块结束」。
///
/// 只在 `depth > 0` 时减深度：`select x end from t` 这种把 `END` 当普通标识符用的
/// 句子不会把深度减成负数（减成负数会让后面每个 `;` 都被当成块内）。
///
/// `blocks` 与 `depth` **一起减**：一个 `END` 要么收掉一个 `BEGIN`，要么收掉一个
/// 表达式 `CASE`；由 `case` 分支决定当初有没有为它加过 `depth`，这里只看 `blocks`
/// 里还有没有块可收（见 `blocks` 的注释）。
fn settle_end(head: &mut Head, depth: &mut usize, blocks: &mut usize) {
    if *head == Head::EndSeen {
        if *depth > 0 {
            *depth -= 1;
        }
        if *blocks > 0 {
            *blocks -= 1;
        }
        *head = Head::Other;
    }
}

/// 从 `at` 起跳过空白与 `--` / `/* */` 注释，返回下一个**有效字符**的位置
/// （可能等于 `bytes.len()`，表示「后面没有有效字符了」）。
///
/// 切分器需要「往前看」的地方只有两处（`begin` 后面是不是 `;`、`end` 后面是不是
/// `if`/`loop`/`case`/`while`），两处都用它，避免注释跳过规则写两遍走偏。
///
/// 未闭合的注释按「跳到脚本末尾」处理：切分器不做 panic（方案 R-13），
/// 这种脚本交给服务端用语法错拒绝。
fn skip_trivia(bytes: &[u8], at: usize) -> usize {
    let mut j = at;
    loop {
        while j < bytes.len() && is_blank(bytes[j]) {
            j += 1;
        }
        if bytes.get(j) == Some(&b'-') && bytes.get(j + 1) == Some(&b'-') {
            j += 2;
            while j < bytes.len() && bytes[j] != b'\n' {
                j += 1;
            }
            continue;
        }
        if bytes.get(j) == Some(&b'/') && bytes.get(j + 1) == Some(&b'*') {
            j += 2;
            while j < bytes.len() && !(bytes[j] == b'*' && bytes.get(j + 1) == Some(&b'/')) {
                j += 1;
            }
            j = (j + 2).min(bytes.len());
            continue;
        }
        return j;
    }
}

/// 当前位置（`END` 之后）起是不是 `IF` / `LOOP` / `CASE` / `WHILE` 这类
/// **块体内部**的收尾词。
///
/// `END IF;` / `END LOOP;` / `END CASE;` 在自己的分号处并不结束外层 `BEGIN` 块，
/// 所以它们**不能**减块深度；只有光秃秃的 `END`（或 `END;` / `END 末尾`）才是块结束。
fn end_is_body_closer(bytes: &[u8], at: usize) -> bool {
    let j = skip_trivia(bytes, at);
    [b"if".as_slice(), b"loop", b"case", b"while"]
        .iter()
        .any(|w| starts_with_ci(bytes, j, w))
}

/// 把 `begin` 关键字写进缓冲区，并判断它是不是「块开始」。
///
/// 判据：`begin` 后面**第一个有效字符**（跳过空白与注释）不能是 `;`，也不能
/// 没有（脚本到此结束）。`begin;` / `begin` 单独一行都是「打个分号就完事」的普通
/// 语句；否则按块处理，并且**必须**等到最后一个 `END;` 才收尾。
///
/// `after_keyword` 是「`begin` 这个词之后」的位置（调用方自己 `+5` 推过去）。
/// **游标推进由调用方负责**：本函数只写缓冲区并返回判定结果，绝不改游标——
/// 早先那版把「推进到下一个有效字符」交给返回值，结果 `begin;` 场景下调用方
/// 直接越过了分号，整条语句收不了尾（见调用点的注释）。
fn inspect_begin(
    bytes: &[u8],
    code: &mut String,
    has_code: &mut bool,
    after_keyword: usize,
) -> bool {
    code.push_str("begin");
    *has_code = true;
    let next = skip_trivia(bytes, after_keyword);
    !matches!(bytes.get(next), Some(&b';') | None)
}

/// 从 `at` 起复制**一个完整的 `char`** 进缓冲区，返回推进后的位置。
///
/// 这是非 `Normal` 状态（字符串、引号标识符、注释、`q'…'` 串）里复制文本的唯一入口。
/// 为什么不能 `code.push(bytes[at] as char)`：那会把一个 UTF-8 汉字拆成 3 个
/// `U+00E4` 这种拉丁字符（**实测到的乱码**：`中文` → `ä¸­æ\u{96}\u{87}`），
/// 发给服务端就是错的文本。UTF-8 是自同步编码，从首字节起 `str::from_utf8` 一定能
/// 切出完整字符；非法序列退回单字节，交给服务端用语法错拒绝，**绝不 panic**（R-13）。
///
/// 调用方仍需自己先用单字节比较判断收尾标记（`'` / `"` / `*` / 定界符）：这些标记
/// 都是 ASCII，而 UTF-8 多字节序列的**每个字节都 ≥ 0x80**，所以「先查标记、再整字复制」
/// 不会漏掉任何多字节字符的收尾。
fn push_char_at(bytes: &[u8], at: usize, code: &mut String) -> usize {
    if bytes[at] < 0x80 {
        code.push(char::from(bytes[at]));
        return at + 1;
    }
    if let Ok(rest) = std::str::from_utf8(&bytes[at..])
        && let Some(ch) = rest.chars().next()
    {
        code.push(ch);
        return at + ch.len_utf8();
    }
    code.push(char::from(bytes[at]));
    at + 1
}

/// 把当前缓冲区作为一条语句收下。
///
/// **只有 `has_code` 为真才收**：整段只有注释或空白的片段（空脚本、脚本末尾的
/// 尾注释）必须丢掉——留着会往服务端发一条纯注释的「语句」，而它一条语句都不是。
fn push_statement(out: &mut Vec<Statement>, code: &mut String, has_code: &mut bool) {
    let trimmed = code.trim();
    if *has_code && !trimmed.is_empty() {
        out.push(Statement {
            sql: trimmed.to_owned(),
        });
    }
    code.clear();
    *has_code = false;
}

/// 当前位置起是否以 `word`（大小写不敏感）开头，且 `word` 之后不是标识符字符。
///
/// 后一个条件很重要：`ending` / `casement` 不能被当成 `end` / `case`。
fn starts_with_ci(bytes: &[u8], at: usize, word: &[u8]) -> bool {
    if at + word.len() > bytes.len() {
        return false;
    }
    let head = &bytes[at..at + word.len()];
    if !head.eq_ignore_ascii_case(word) {
        return false;
    }
    match bytes.get(at + word.len()) {
        Some(&next) => !is_ident_byte(next),
        None => true,
    }
}

/// 前一个字节是否是标识符字符（用于「关键字前面不能贴着标识符」的判断）。
fn prev_is_ident_byte(bytes: &[u8], at: usize) -> bool {
    at > 0 && is_ident_byte(bytes[at - 1])
}

/// 是否是标识符字符（ASCII 字母、数字、下划线，以及 `$`/`#` 这类达梦允许的字符）。
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'#'
}

/// 是否是空白（含制表、换行；以及任何**非 ASCII** 字节——多字节字符在状态机里
/// 只是「普通字节」，把它们当空白处理能保证 `char` 转换不产生半截 UTF-8）。
fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' || b == 0x0b || b == 0x0c || b >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取出切分结果里的字符串，断言时少写一点样板。
    fn parts(script: &str) -> Vec<String> {
        split_statements(script)
            .into_iter()
            .map(|s| s.sql)
            .collect()
    }

    // -----------------------------------------------------------------------
    // T7.1 显式验收点：字符串字面量里的 `;`
    // -----------------------------------------------------------------------

    #[test]
    fn semicolon_inside_single_quoted_string_does_not_split() {
        let got = parts("insert into t values ('a;b');");
        assert_eq!(got, vec!["insert into t values ('a;b')"]);
    }

    #[test]
    fn doubled_quote_escape_stays_inside_the_literal() {
        // `''` 是「字符串里的一个引号」，不是「字符串结束 + 新字符串开始」。
        let got = parts("insert into t values ('it''s; here'); select 1 from dual;");
        assert_eq!(
            got,
            vec!["insert into t values ('it''s; here')", "select 1 from dual"]
        );
    }

    #[test]
    fn empty_string_literal_is_not_a_statement_boundary() {
        let got = parts("insert into t values (''); select 2 from dual;");
        assert_eq!(got, vec!["insert into t values ('')", "select 2 from dual"]);
    }

    #[test]
    fn semicolon_inside_double_quoted_identifier_does_not_split() {
        let got = parts("create table \"odd;name\" (id int);");
        assert_eq!(got, vec!["create table \"odd;name\" (id int)"]);
    }

    // -----------------------------------------------------------------------
    // T7.1 显式验收点：注释里的 `;`
    // -----------------------------------------------------------------------

    #[test]
    fn semicolon_inside_line_comment_does_not_split() {
        let got = parts("select 1 from dual -- 这里有个分号 ; 不算\n; select 2 from dual;");
        assert_eq!(
            got,
            vec![
                "select 1 from dual -- 这里有个分号 ; 不算",
                "select 2 from dual"
            ]
        );
    }

    #[test]
    fn semicolon_inside_block_comment_does_not_split() {
        let got = parts("select 1 /* a ; b */ from dual; select 2 from dual;");
        assert_eq!(
            got,
            vec!["select 1 /* a ; b */ from dual", "select 2 from dual"]
        );
    }

    #[test]
    fn block_comment_is_not_nested() {
        // 标准 SQL 的块注释不嵌套：第一个 `*/` 就收尾，所以后面的 `;` 是分隔符。
        let got = parts("select 1 /* a /* b */ ; select 2 from dual;");
        assert_eq!(got, vec!["select 1 /* a /* b */", "select 2 from dual"]);
    }

    #[test]
    fn comment_markers_inside_string_are_literal_text() {
        let got = parts(
            "insert into t values ('-- 不是注释'); insert into t (c) values ('/* 也不是 */');",
        );
        assert_eq!(
            got,
            vec![
                "insert into t values ('-- 不是注释')",
                "insert into t (c) values ('/* 也不是 */')"
            ]
        );
    }

    #[test]
    fn quote_inside_line_comment_is_literal() {
        // 注释里的引号不会「打开」一个字符串，否则后面的 `;` 会被吞掉。
        let got = parts("select 1 -- it's fine\n; select 2 from dual;");
        assert_eq!(got, vec!["select 1 -- it's fine", "select 2 from dual"]);
    }

    // -----------------------------------------------------------------------
    // T7.1 显式验收点：PL/SQL 块不被误切
    // -----------------------------------------------------------------------

    #[test]
    fn plsql_block_is_kept_as_one_statement() {
        let script = "begin\n  insert into t values (1);\n  insert into t values (2);\nend;\n";
        let got = parts(script);
        assert_eq!(got.len(), 1, "BEGIN…END; 必须整体成一条语句：{got:?}");
        assert!(got[0].starts_with("begin"));
        assert!(got[0].ends_with("end"));
    }

    #[test]
    fn statement_after_a_plsql_block_is_still_split() {
        let script = "begin\n  null;\nend;\nselect 1 from dual;";
        let got = parts(script);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[1], "select 1 from dual");
    }

    #[test]
    fn two_bare_begins_are_two_statements() {
        // 回归：`begin;` 后面紧跟另一个块时，第二个 `begin` 仍然是新语句的第一个词
        // （曾把「是不是块」的状态结算放在分支外，导致第二个 `begin` 被当成普通词）。
        let got = parts("begin; begin; select 1 from dual;");
        let all: Vec<String> = split_statements("begin; begin; select 1 from dual;")
            .into_iter()
            .map(|s| s.sql)
            .collect();
        assert_eq!(
            got,
            vec!["begin", "begin", "select 1 from dual"],
            "all={all:?}"
        );
    }

    #[test]
    fn nested_begin_blocks_are_kept_until_the_outer_end() {
        let script = "begin\n  begin\n    insert into t values (1);\n  end;\n  insert into t values (2);\nend;\n";
        let got = parts(script);
        assert_eq!(got.len(), 1, "嵌套块的内部 END; 不能提前收尾：{got:?}");
    }

    #[test]
    fn end_if_inside_a_block_does_not_close_it_early() {
        // `END IF` 的 `END` **不是**块结束：它的欠账要留到语句末尾那个 `;`
        // 才能结算，否则外层 `BEGIN` 会被提前收尾。
        let script = "begin\n  if 1 = 1 then\n    insert into t values (1);\n  end if;\nend;\nselect 1 from dual;";
        let got = parts(script);
        assert_eq!(got.len(), 2, "END IF 提前收尾了：{got:?}");
        assert_eq!(got[1], "select 1 from dual");
    }

    #[test]
    fn case_expression_end_does_not_end_the_block_early() {
        // `case … end` 也以 `END` 收尾：不计入深度的话，会在 `case` 的 `END`
        // 之后那个分号上误切。
        let script = "begin\n  v := case when a > 1 then 'x;y' else 'z' end;\n  insert into t values (v);\nend;\n";
        let got = parts(script);
        assert_eq!(got.len(), 1, "CASE 的 END 被当成了块结束：{got:?}");
    }

    #[test]
    fn quoted_string_keeps_semicolon_and_looks_like_a_comment() {
        // `q'[…]'` 里的 `;` 不分隔，里面的 `/*` 也不是注释开始。
        let script = "begin\n  insert into t values (q'[a;b /* not a comment */]');\nend;";
        let got = parts(script);
        assert_eq!(got.len(), 1, "{got:?}");
    }

    #[test]
    fn q_quote_with_brace_delimiter() {
        let got = parts("insert into t values (q'{a;b}'); select 1 from dual;");
        assert_eq!(
            got,
            vec!["insert into t values (q'{a;b}')", "select 1 from dual"]
        );
    }

    #[test]
    fn bare_begin_is_not_treated_as_a_block() {
        // `begin;` 后面直接就是分号：它是普通语句，不能进入「等 END」的状态
        // （那样脚本剩下的部分会被整块吞掉，一个语句都发不出去）。
        let got = parts("begin; select 1 from dual;");
        assert_eq!(got, vec!["begin", "select 1 from dual"]);
    }

    /// 注释是**词法透明**的：`/* c */ begin … end;` 里的 `begin` 仍然是块开始
    /// （与 Oracle 一致）。这条同时钉住**已知边界**——`begin transaction; select …;`
    /// 会因为同一个原因被当成块整体发出，由服务端报语法错，而不是我们静默切成两条。
    #[test]
    fn comment_is_lexically_transparent_before_begin() {
        let got = parts("/* 事务 */ begin\n  insert into t values (1);\nend;\nselect 1 from dual;");
        assert_eq!(got.len(), 2, "注释不改变「begin 是第一个词」：{got:?}");
        assert_eq!(got[1], "select 1 from dual");

        // 已知边界（词法上无法区分的后果，如实钉住而不是假装支持）。
        let ambiguous = parts("begin transaction; select 1 from dual;");
        assert_eq!(
            ambiguous.len(),
            1,
            "`begin transaction` 与块在词法上分不出来，只能整体交给服务端：{ambiguous:?}"
        );
    }

    #[test]
    fn end_used_as_a_plain_identifier_does_not_corrupt_depth() {
        // 深度不能减成负数，否则后面每个分号都会被当成「块内」而一个语句都切不出来。
        let got = parts("select a as end from t; select 1 from dual;");
        assert_eq!(got, vec!["select a as end from t", "select 1 from dual"]);
    }

    #[test]
    fn keyword_prefix_inside_a_longer_identifier_is_not_a_keyword() {
        let got = parts("select append, casey, ending from t; select 1 from dual;");
        assert_eq!(
            got,
            vec!["select append, casey, ending from t", "select 1 from dual"]
        );
    }

    #[test]
    fn comment_after_begin_still_counts_as_a_block() {
        // `begin` 与块体之间夹注释不影响判定：注释不是「有效字符」。
        let script = "begin -- 开始\n  insert into t values (1);\nend;\nselect 1 from dual;";
        let got = parts(script);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[1], "select 1 from dual");
    }

    #[test]
    fn block_comment_between_begin_and_body_still_counts_as_a_block() {
        let script = "begin /* 开始 */\n  insert into t values (1);\nend;";
        let got = parts(script);
        assert_eq!(got.len(), 1, "{got:?}");
    }

    // -----------------------------------------------------------------------
    // 空语句 / 末尾无分号
    // -----------------------------------------------------------------------

    #[test]
    fn empty_statements_and_stray_semicolons_are_dropped() {
        let got = parts(";;\n;\nupdate t set v = 1;;\n;");
        assert_eq!(got, vec!["update t set v = 1"]);
    }

    #[test]
    fn trailing_statement_without_semicolon_still_counted() {
        let got = parts("select 1 from dual;\nselect 2 from dual");
        assert_eq!(got, vec!["select 1 from dual", "select 2 from dual"]);
    }

    #[test]
    fn script_without_any_semicolon_is_one_statement() {
        let got = parts("select 1 from dual");
        assert_eq!(got, vec!["select 1 from dual"]);
    }

    /// 多字节字符按**字符**而不是按字节处理：既不能被拆成乱码，也不能因为字节值
    /// 落进 ASCII 区间而误判收尾。真缺陷曾在这里：非 `Normal` 状态用
    /// `code.push(b as char)` 逐字节复制，`中文` 被拆成 `ä¸­æ\u{96}\u{87}` 这样的乱码。
    /// 现在统一走 `push_char_at`，这里用正式断言钉住。
    #[test]
    fn multibyte_literal_survives_verbatim() {
        let got = parts("insert into t values ('中文；分号');");
        assert_eq!(got.len(), 1, "全角分号不是分隔符，必须只有 1 条：{got:?}");
        assert_eq!(got[0], "insert into t values ('中文；分号')");
    }

    /// 多字节字符出现在字符串/注释里时，同一条语句内的文本必须逐字保持原样
    /// （既不能乱码，也不能因为 `is_blank` 把 ≥ 0x80 的字节当空白而吃掉内容）。
    #[test]
    fn multibyte_text_in_literals_and_comments_is_verbatim() {
        let got =
            parts("insert into t values ('中文；分号'); -- 中文注释；\nselect '再见' from dual;");
        assert_eq!(
            got,
            vec![
                "insert into t values ('中文；分号')",
                "-- 中文注释；\nselect '再见' from dual",
            ]
        );
    }

    #[test]
    fn empty_or_whitespace_only_script_yields_nothing() {
        assert!(parts("").is_empty());
        assert!(parts("   \n\t\r\n  ").is_empty());
        assert!(parts("-- 只有注释\n").is_empty());
        assert!(parts("/* 只有块注释 */").is_empty());
    }

    #[test]
    fn statements_are_trimmed_but_inner_text_is_verbatim() {
        // 尾部的纯注释片段（分号之后没有真代码）**被丢弃**：它不是一条能发出去的语句，
        // 留着只会让服务端收到一条纯注释。脚本内部跟着真代码的注释则原样保留。
        let got = parts("  \n  insert into t values (1) -- 行内注释\n  ;\n  -- 尾注释\n");
        assert_eq!(
            got,
            vec!["insert into t values (1) -- 行内注释"],
            "尾部纯注释片段应当被丢弃"
        );
    }

    #[test]
    fn unterminated_constructs_do_not_panic() {
        // 手写脚本出错时不能 panic（不许 `unwrap`/`panic!` 顶实现，见设计方案 R-13），
        // 切不出来就交给服务端用语法错拒绝。
        for bad in [
            "insert into t values ('unterminated",
            "select 1 /* unterminated",
            "q'[unterminated",
            "insert into t values (\"unterminated",
        ] {
            let got = parts(bad);
            assert_eq!(got.len(), 1, "{bad:?} 应当作为一条语句交给服务端");
        }
    }

    /// 全角 `；` 是文本不是分隔符；顺带钉住中文在不带引号/注释的最普通路径下也不乱码。
    #[test]
    fn multibyte_comment_text_is_preserved() {
        let got = parts("insert into t values ('中文；分号'); -- 中文注释；\nselect 1 from dual;");
        assert_eq!(
            got,
            vec![
                "insert into t values ('中文；分号')",
                // 注释挂在**下一段真代码**上（分号切开后注释是紧接着的那条语句的前缀）。
                "-- 中文注释；\nselect 1 from dual"
            ]
        );
    }

    /// 半角 `;` 与全角 `；` 是两回事：只有半角才是分隔符。
    #[test]
    fn full_width_semicolon_is_not_a_separator() {
        let got = parts("insert into t values ('；');");
        assert_eq!(got, vec!["insert into t values ('；')"]);
    }
}
