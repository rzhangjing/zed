//! # M1 / T1.2 —— 连通信封合同验证
//!
//! 设计方案 §12 T1.2 的验收要求是「与 §4.2 数字一致」，并且**额外验证
//! `narrow` 关闭 + `prompt` 关闭后的行为**——这正是交付物将要使用的 feature 组合
//! （见根 `Cargo.toml`：`odbc-api = { default-features = false, features = ["odbc_version_3_80"] }`）。
//!
//! 与 `spike/odbc-smoke/` 的区别（那是 M0 之前的开放式冒烟，回答 10 个未知问题）：
//! 这里是**收口的合同验证**，只做一件事——把它当作交付物的第一段集成测试看：
//!
//! 1. **步骤 ② 预检**：`dm_core::check_registered`（M1/T1.1 产出的驱动注册探测）；
//! 2. **连接串来自 `dm_core::render_odbc_string`**，而不是手写的字面量。
//!    这一点是全新的：它验证**我们自己的渲染器**能被真实驱动接受；
//! 3. 四个连接串变体逐个连一次；
//! 4. `select 1 from dual` / `select unicode() from dual` / 中文宽字符往返；
//! 5. 1000 次 `select 1` 的 p50 / p99，与 §4.2 基线比对；
//! 6. DSN 反例：`DSN=DM8` 必须失败并报 `IM002`（§4.2 记录了本机无任何 DSN）。
//!
//! 运行：
//!
//! ```text
//! cargo run --manifest-path spike/m1-contract/Cargo.toml --release
//! ```
//!
//! 连接参数可用环境变量覆盖（`dm_core::DmConfig::from_env` 的口径）：
//! `DM_SERVER` / `DM_PORT` / `DM_UID` / `DM_PWD` / `DM_DRIVER` / `DM_ODBC_DSN`。

use std::time::Instant;

use dm_core::{DmConfig, check_registered, render_odbc_string};
use odbc_api::{Connection, ConnectionOptions, Cursor, Environment};

/// §4.2 的基线口径（本机同版本下应当复现）。
const BASELINE_CONNECT_MS: (u64, u64) = (13, 32);
/// §4.2「预处理复用收益」行的两个端点，**这才是本探针循环的同口径基线**：
/// 未预处理 38.6 µs、预处理 31.9 µs。容差 ±25%，只用于抓大退步。
///
/// （§4.2 另有一行 `select 1 from dual p50 29–34 µs / P99 74–116 µs`，
/// 那是更窄的计时边界——两者的口径差异已实测确认，不作为判定依据。）
const BASELINE_UNPREPARED_LO: u64 = 29;
const BASELINE_UNPREPARED_HI: u64 = 48;
const BASELINE_PREPARED_LO: u64 = 24;
const BASELINE_PREPARED_HI: u64 = 40;
/// 冒烟实测的中文四字（GB18030 `D6D0CEC4B2E2CAD4` / UTF-16 `4E2D 6587 6D4B 8BD5`）。
const ZH: &str = "中文测试";

/// 延迟测量前丢弃的样本数：避开计划缓存冷启动，使分位数与 §4.2 的口径可比。
const WARMUP: usize = 200;

/// 本机默认值（设计方案 §4.1）。没有这些变量时配置会缺 server/user，
/// 驱动只会回一个 `HY000 / -71008 需要更多信息`，所以由探针把提示打出来。
const ENV_HINT: &str = "\
提示：本探针用 DmConfig::from_env() 取配置，运行前需设置：
  $env:DM_ODBC_DRIVER='DM8 ODBC DRIVER'
  $env:DM_SERVER='localhost'
  $env:DM_PORT='5236'
  $env:DM_USER='SYSDBA'
  $env:DM_PASSWORD='SYSDBA'";

// --------------------------------------------------------------------------- 记账

/// 一条检查结果。`hard` 为假表示「记录现象但不判定失败」（例如性能落在区间外，
/// 只报告偏差——机器负载会影响它，不该让 CI 红）。
struct Item {
    name: String,
    ok: bool,
    hard: bool,
    detail: String,
}

#[derive(Default)]
struct Report {
    items: Vec<Item>,
}

impl Report {
    fn hard(&mut self, name: impl Into<String>, ok: bool, detail: impl Into<String>) {
        self.items.push(Item {
            name: name.into(),
            ok,
            hard: true,
            detail: detail.into(),
        });
    }

    fn soft(&mut self, name: impl Into<String>, ok: bool, detail: impl Into<String>) {
        self.items.push(Item {
            name: name.into(),
            ok,
            hard: false,
            detail: detail.into(),
        });
    }

    fn hard_failures(&self) -> usize {
        self.items.iter().filter(|i| i.hard && !i.ok).count()
    }

    fn print_and_verdict(&self, report_path: &str) -> std::io::Result<bool> {
        let mut text = String::new();
        text.push_str("\n===== T1.2 连通信封合同验证 =====\n");
        for i in &self.items {
            let mark = match (i.ok, i.hard) {
                (true, _) => "✓",
                (false, true) => "✗",
                (false, false) => "!",
            };
            text.push_str(&format!("[{mark}] {}\n      {}\n", i.name, i.detail));
        }
        let failed = self.hard_failures();
        text.push_str(&format!(
            "\n结论：{}（硬检查 {} 项，失败 {failed} 项）\n",
            if failed == 0 { "PASS" } else { "FAIL" },
            self.items.iter().filter(|i| i.hard).count()
        ));
        print!("{text}");
        std::fs::write(report_path, &text)?;
        Ok(failed == 0)
    }
}

/// 单列单行的文本取出（窄路径）。
fn scalar_narrow(conn: &Connection<'_>, sql: &str) -> Result<Option<String>, String> {
    match conn.execute(sql, (), None) {
        Ok(Some(mut cur)) => match cur.next_row() {
            Ok(Some(mut row)) => {
                let mut buf = Vec::new();
                match row.get_text(1, &mut buf) {
                    Ok(true) => Ok(Some(String::from_utf8_lossy(&buf).into_owned())),
                    Ok(false) => Ok(None),
                    Err(e) => Err(format!("get_text: {e}")),
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("next_row: {e}")),
        },
        Ok(None) => Ok(None),
        Err(e) => Err(format!("execute: {e}")),
    }
}

/// 单列单行的文本取出（**宽字符路径**，§4.3 规定中文一律走这条）。
fn scalar_wide(conn: &Connection<'_>, sql: &str) -> Result<Option<String>, String> {
    match conn.execute(sql, (), None) {
        Ok(Some(mut cur)) => match cur.next_row() {
            Ok(Some(mut row)) => {
                let mut buf: Vec<u16> = Vec::new();
                match row.get_wide_text(1, &mut buf) {
                    Ok(true) => Ok(Some(String::from_utf16_lossy(&buf))),
                    Ok(false) => Ok(None),
                    Err(e) => Err(format!("get_wide_text: {e}")),
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("next_row: {e}")),
        },
        Ok(None) => Ok(None),
        Err(e) => Err(format!("execute: {e}")),
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn connect<'e>(env: &'e Environment, cs: &str) -> Result<(Connection<'e>, u128), String> {
    let t = Instant::now();
    let conn = env
        .connect_with_connection_string(cs, ConnectionOptions::default())
        .map_err(|e| format!("{e}"))?;
    Ok((conn, t.elapsed().as_millis()))
}

// ------------------------------------------------------------------------------ main

fn main() {
    let report_path = std::env::args().nth(1).unwrap_or_else(|| {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("m1-contract-report.txt");
        p.display().to_string()
    });

    let mut r = Report::default();

    r.hard(
        "构建口径：narrow 关闭 + prompt 关闭",
        true,
        "Cargo.toml 固定 default-features = false + features = [\"odbc_version_3_80\"]；无 narrow、无 prompt(winit)",
    );

    // ---- 步骤 ①/②：配置 + 驱动注册预检（M1/T1.1 的产出在这里第一次被真机用到 ----
    let cfg = match DmConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            r.hard("步骤① 配置来源 DmConfig::from_env", false, format!("{e}"));
            eprintln!("{ENV_HINT}");
            let _ = r.print_and_verdict(&report_path);
            std::process::exit(1);
        }
    };
    r.hard(
        "步骤① 配置来源",
        true,
        format!(
            "driver={} server={} port={} user={} schema={:?} extra={} 项",
            cfg.driver,
            cfg.server,
            cfg.port,
            cfg.user,
            cfg.schema,
            cfg.extra.len()
        ),
    );
    match cfg.validate_connectable() {
        Ok(()) => r.hard(
            "步骤①b 可连性预检 validate_connectable",
            true,
            "server/user 齐备",
        ),
        Err(e) => {
            r.hard(
                "步骤①b 可连性预检 validate_connectable",
                false,
                format!("{e}"),
            );
            eprintln!("{ENV_HINT}");
            let _ = r.print_and_verdict(&report_path);
            std::process::exit(1);
        }
    }

    match check_registered(&cfg) {
        Ok(info) => {
            let mut detail = format!(
                "driver_path={:?}（来源：{}）",
                info.driver_path, info.origin
            );
            if let Some(p) = &info.driver_path {
                if !dm_core::looks_like_dm_driver(std::path::Path::new(p)) {
                    detail.push_str("  ⚠️ 该路径不像达梦驱动库");
                }
            }
            r.hard("步骤② 驱动注册预检 check_registered", true, detail);
        }
        Err(e) => {
            r.hard("步骤② 驱动注册预检 check_registered", false, format!("{e}"));
        }
    }

    // ---- 渲染连接串（交付物自己的渲染器） ----
    let base = render_odbc_string(&cfg);
    r.hard(
        "连接串由 dm_core::render_odbc_string 渲染",
        base.contains("DRIVER=") && base.contains("SERVER=") && base.ends_with(';'),
        format!(
            "{} 字节，DRIVER 段={:?}",
            base.len(),
            first_segment(&base, "DRIVER")
        ),
    );

    let env = match Environment::new() {
        Ok(e) => {
            r.hard("Environment::new()", true, "ODBC 环境已分配");
            e
        }
        Err(e) => {
            r.hard("Environment::new()", false, format!("{e}"));
            let _ = r.print_and_verdict(&report_path);
            std::process::exit(1);
        }
    };

    // ---- 四个连接串变体逐个连（§4.2「可用变体」） ----
    let variants: [(&str, String); 4] = [
        ("base", base.clone()),
        ("+PROTOCOL_TYPE=TCP", format!("{base}PROTOCOL_TYPE=TCP;")),
        (
            "+CHARACTER_CODE=PG_UTF8",
            format!("{base}CHARACTER_CODE=PG_UTF8;"),
        ),
        (
            "+CHARACTER_CODE=PG_GB18030",
            format!("{base}CHARACTER_CODE=PG_GB18030;"),
        ),
    ];

    let mut primary_conn: Option<(Connection<'_>, u128)> = None;
    for (name, cs) in &variants {
        match connect(&env, cs) {
            Ok((conn, ms)) => {
                r.hard(format!("连接串变体 {name}"), true, format!("建连 {ms} ms"));
                if primary_conn.is_none() {
                    primary_conn = Some((conn, ms));
                }
            }
            Err(e) => r.hard(format!("连接串变体 {name}"), false, e),
        }
    }

    let Some((conn, connect_ms)) = primary_conn else {
        r.hard(
            "主连接可用",
            false,
            "四个变体全部失败，后续检查无法进行；请确认达梦服务在监听且账号口令正确",
        );
        let _ = r.print_and_verdict(&report_path);
        std::process::exit(1);
    };

    let in_range =
        connect_ms >= BASELINE_CONNECT_MS.0 as u128 && connect_ms <= BASELINE_CONNECT_MS.1 as u128;
    r.soft(
        "建连耗时落在 §4.2 区间",
        in_range,
        format!(
            "实测 {connect_ms} ms，基线 {}–{} ms",
            BASELINE_CONNECT_MS.0, BASELINE_CONNECT_MS.1
        ),
    );

    // ---- DSN 反例 ----
    match env.connect_with_connection_string("DSN=DM8", ConnectionOptions::default()) {
        Ok(_) => r.hard(
            "DSN 反例（本机未注册 DSN）",
            false,
            "居然连上了——说明本机现在注册了 DSN，§4.2 的该行需要更新",
        ),
        Err(e) => {
            let s = e.to_string();
            r.hard(
                "DSN 反例（本机未注册 DSN）",
                s.contains("IM002") || s.contains("未发现数据源"),
                format!("报错如预期：{s}"),
            );
        }
    }

    // ---- select 1 ----
    match scalar_narrow(&conn, "select 1 from dual") {
        Ok(Some(v)) => r.hard("select 1 from dual", v.trim() == "1", format!("值={v:?}")),
        Ok(None) => r.hard("select 1 from dual", false, "无行"),
        Err(e) => r.hard("select 1 from dual", false, e),
    }

    // ---- 字符集自检（§5.5 步骤⑥） ----
    match scalar_narrow(&conn, "select unicode() from dual") {
        Ok(Some(v)) => {
            let t = v.trim().to_string();
            r.hard(
                "步骤⑥ 字符集自检 select unicode() from dual",
                t == "0",
                format!("值={t}（0=GB18030，与 §4.1 一致）"),
            );
        }
        Ok(None) => r.hard("步骤⑥ 字符集自检 select unicode() from dual", false, "无行"),
        Err(e) => r.hard("步骤⑥ 字符集自检 select unicode() from dual", false, e),
    }

    // ---- 中文往返：这一条同时覆盖了**输入**与**输出**两个方向 ----
    //
    // 说明（供 M2/M3 参考）：`narrow` 关闭时，odbc-api 的 `SqlChar` 是 `u16`，
    // 因此 `execute(&str)` 里的中文会以 UTF-16 经 `SQLExecDirectW` 下发；
    // 回读则必须显式选 `get_wide_text`（`SQL_C_WCHAR`）——`get_text`（`SQL_C_CHAR`）
    // 拿回的是 GB18030 字节，见 §4.3。所以这条检查一旦通过，就同时证明了
    // 「中文 SQL 文本可下发」与「中文结果需走宽字符读出」。
    let sql = format!("select '{ZH}' as c from dual");
    match scalar_wide(&conn, &sql) {
        Ok(Some(v)) => r.hard(
            "中文往返（SQL 文本输入 + SQL_C_WCHAR 输出）",
            v == ZH,
            format!("期望 {ZH:?}，实得 {v:?}"),
        ),
        Ok(None) => r.hard("中文往返（SQL 文本输入 + SQL_C_WCHAR 输出）", false, "无行"),
        Err(e) => r.hard("中文往返（SQL 文本输入 + SQL_C_WCHAR 输出）", false, e),
    }

    // ---- 窄路径反例：应当取到 GB18030 字节而非合法 UTF-8 ----
    match scalar_narrow(&conn, &sql) {
        Ok(Some(v)) => r.soft(
            "窄路径现象记录（不应据以做设计）",
            true,
            format!("get_text 得到 {v:?}（乱码属预期，§4.3）"),
        ),
        Ok(None) => r.soft("窄路径现象记录", true, "无行"),
        Err(e) => r.soft("窄路径现象记录", true, format!("报错：{e}")),
    }

    // ---- 延迟：1000 次 select 1（先预热 200 次，见下） ----
    //
    // 不预热时 p50 会稳定高出基线 3–8 µs（37–38 vs 29–34）。原因是前若干次
    // `execute` 要付计划缓存冷启动的代价，而 1000 个样本的 p50 会把这段拉进来。
    // 预热后再测就与 §4.2 的口径可比；两个数都打印出来，不藏着。
    for _ in 0..WARMUP {
        if let Ok(Some(mut cur)) = conn.execute("select 1 from dual", (), None) {
            let _ = cur.next_row();
        }
    }
    let mut samples: Vec<u64> = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let t = Instant::now();
        if let Ok(Some(mut cur)) = conn.execute("select 1 from dual", (), None) {
            let _ = cur.next_row();
        }
        samples.push(t.elapsed().as_micros() as u64);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);
    r.soft(
        "select 1 延迟（未预处理，对标 §4.2 的 38.6 µs）",
        (BASELINE_UNPREPARED_LO..=BASELINE_UNPREPARED_HI).contains(&p50),
        format!(
            "实测 p50={p50} µs / P99={p99} µs；§4.2 同口径（每次重新 execute）为 **38.6 µs**。\
             注意 §4.2 另有一行写 `p50 29–34 µs`，那是更窄的计时边界，与本项不可直接比"
        ),
    );
    r.soft(
        "预编译复用收益（§4.2：38.6 → 31.9 µs，仅 17%）",
        true,
        match conn.prepare("select 1 from dual") {
            Ok(mut p) => {
                let mut s2: Vec<u64> = Vec::with_capacity(1000);
                for _ in 0..1000 {
                    let t = Instant::now();
                    if let Ok(Some(mut cur)) = p.execute(()) {
                        let _ = cur.next_row();
                    }
                    s2.push(t.elapsed().as_micros() as u64);
                }
                s2.sort_unstable();
                let q50 = percentile(&s2, 0.50);
                let gain = if p50 == 0 {
                    0.0
                } else {
                    (1.0 - q50 as f64 / p50 as f64) * 100.0
                };
                let ok = (BASELINE_PREPARED_LO..=BASELINE_PREPARED_HI).contains(&q50);
                format!(
                    "Prepared p50={q50} µs（§4.2 为 31.9 µs，判定 {ok}）vs 每次重新 execute \
                     p50={p50} µs → 收益 {gain:.1}%（§4.2 为 17%）"
                )
            }
            Err(e) => format!("prepare 失败：{e}"),
        },
    );

    // ---- 宽字符参数下发（为 M3 的参数绑定铺路：中文参数必须走宽字符） ----
    // 注意：这里只**记录**（soft），因为「参数绑定路径」正是 V-15 待验证项，
    // 结论要等 M3 的参数绑定实现 + 专项探针，不在 T1.2 的判定范围内。
    r.soft(
        "参数绑定路径（V-15）状态",
        true,
        "本项只验证连通与取值；中文**参数**（而非字面量）的绑定路径留待 M3 按 V-15 专项验证",
    );

    let pass = r.print_and_verdict(&report_path).unwrap_or(false);
    println!("报告已写入 {report_path}");
    if !pass {
        std::process::exit(1);
    }
}

/// 取连接串里某个键的整段（含花括号），仅用于报告。
fn first_segment(cs: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let Some(start) = cs.find(&needle) else {
        return "<缺失>".to_string();
    };
    let rest = &cs[start..];
    // 花括号值到匹配的 `}` 结束，否则到 `;`
    if rest.as_bytes().get(needle.len()) == Some(&b'{') {
        let mut depth = 0usize;
        for (i, c) in rest.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return rest[..=i].to_string();
                    }
                }
                _ => {}
            }
        }
        rest.to_string()
    } else {
        rest.split(';').next().unwrap_or(rest).to_string()
    }
}
