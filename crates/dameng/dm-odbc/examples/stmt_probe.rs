//! T3.3 真机验收探针：语句对象（预处理 / 一次性下发 / 重复执行 / `-70027` 零出现）。
//!
//! 跑法（环境变量口径与 T1.2 的 spike 一致）：
//!
//! ```text
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example stmt_probe
//! ```
//!
//! 验收口径直接来自 §12.2 T3.3：
//!
//! - 「**同一语句连续两次执行不出状态错**」——用例 2/3/4 反复执行同一个语句句柄。
//! - 「**`create table` 后不 fetch**」——用例 1 只断言产出形态是影响行数，绝不 `fetch`。
//! - §6.8：全程 **`-70027` 计数必须为 0**——探针逐条统计并最后打印。
//!
//! 与 `connect_probe` 一样**故意用 `println!` 而不是断言**：要的是「真机上到底发生了什么」，
//! 断言会把没预料到的真实行为变成中断而不是信息。逐行看输出，别看退出码。

use dm_core::DmConfig;
use dm_error::{DmError, DmErrorKind};
use dm_odbc::OdbcConnection;

/// §6.8 表末行：达梦「无效的游标状态」。本探针的硬指标是它一次都不出现。
const INVALID_CURSOR: i32 = dm_error::codes::INVALID_CURSOR;

/// 探针用的表名：带固定后缀，跑完就删（`schema.sql` 之前不许留垃圾）。
const PROBE_TABLE: &str = "DMD_ODBC_PROBE_STMT";

fn base() -> DmConfig {
    DmConfig::from_env().expect("环境变量口径不合法（DM_SERVER/DM_USER/DM_PASSWORD…）")
}

fn describe(err: &DmError) -> String {
    let mut out = format!("{:?} @ {:?}", err.kind, err.context.stage);
    for (i, d) in err.diags().iter().enumerate() {
        out.push_str(&format!(
            "\n        diag[{i}] state={:?} native={} msg={}",
            d.sqlstate_str(),
            d.native_code,
            d.message
        ));
    }
    out
}

/// 探针的记账本：失败次数 + `-70027` 出现次数。
#[derive(Default)]
struct Tally {
    failures: usize,
    invalid_cursor: usize,
    checks: usize,
}

impl Tally {
    /// 记一次「期望成功」的调用。
    fn ok<T>(&mut self, what: &str, result: Result<T, DmError>) -> Option<T> {
        self.checks += 1;
        match result {
            Ok(value) => {
                println!("  OK  {what}");
                Some(value)
            }
            Err(err) => {
                self.failures += 1;
                self.note_cursor(&err);
                println!("  FAIL {what}：{}", describe(&err));
                None
            }
        }
    }

    /// 记一次「期望失败」的调用：`pred` 为真才算通过。
    fn expect_err<T: std::fmt::Debug>(
        &mut self,
        what: &str,
        result: Result<T, DmError>,
        pred: impl Fn(&DmError) -> bool,
    ) {
        self.checks += 1;
        match result {
            Ok(value) => {
                self.failures += 1;
                println!("  FAIL {what}：本应失败，却成功返回 {value:?}");
            }
            Err(err) => {
                self.note_cursor(&err);
                if pred(&err) {
                    println!("  OK  {what}：按预期失败 → {:?}", err.kind);
                } else {
                    self.failures += 1;
                    println!("  FAIL {what}：失败原因不是预期的那种 → {}", describe(&err));
                }
            }
        }
    }

    /// §6.8 硬指标：任何一条诊断里出现 `-70027` 都要记一笔。
    fn note_cursor(&mut self, err: &DmError) {
        // `server_code()` 返回的是 `(sqlstate, code)` 整个三元组的前两项，只要码。
        if err.kind.server_code().map(|(_, code)| code) == Some(INVALID_CURSOR)
            || err.diags().iter().any(|d| d.native_code == INVALID_CURSOR)
        {
            self.invalid_cursor += 1;
            println!("        ⚠ 出现了 -70027 无效的游标状态");
        }
    }

    fn check(&mut self, what: &str, condition: bool) {
        self.checks += 1;
        if condition {
            println!("  OK  {what}");
        } else {
            self.failures += 1;
            println!("  FAIL {what}");
        }
    }
}

fn main() {
    let mut t = Tally::default();

    let mut conn = match OdbcConnection::connect(&base()) {
        Ok(c) => c,
        Err(err) => {
            println!("无法建连，探针中止：{}", describe(&err));
            std::process::exit(2);
        }
    };

    // 收尾用：不管中间怎么错，最后都把探针表删掉。
    let _ = conn.rollback();

    // ------------------------------------------------------------------
    println!("=== 用例 1：create table（无结果集，绝不 fetch）===");
    {
        let mut stmt = match conn.allocate_statement() {
            Ok(s) => s,
            Err(err) => {
                println!("分配语句失败，探针中止：{}", describe(&err));
                std::process::exit(2);
            }
        };
        // 先清场：上次跑崩了可能留下残表。这一步「表不存在」是正常情况，不记账。
        match stmt.exec_direct(&format!("drop table {PROBE_TABLE}")) {
            Ok(_) => println!("  --  残表存在，已清掉"),
            Err(err) => println!(
                "  --  清场 drop 失败（表本来就不存在，正常）：{:?}",
                err.kind
            ),
        }

        let outcome = t.ok(
            "exec_direct(create table)",
            stmt.exec_direct(&format!(
                "create table {PROBE_TABLE} (id int primary key, name varchar(64), amount decimal(10,2))"
            )),
        );
        if let Some(outcome) = outcome {
            // 这就是 DPI 时代漏判的地方：`create table` 必须报「影响行数」，不是结果集。
            t.check(
                "create table 被判为「无结果集」（DPI 曾在此漏判并换来 -70027）",
                !outcome.has_result_set(),
            );
            t.check(
                "create table 的 columns() 为 None",
                outcome.columns().is_none(),
            );
            // **绝不 fetch**：本探针只在 `has_result_set()` 为真时才 fetch，而这里为假。
            println!("  --  未对 create table 调用 fetch（结构上不可能：outcome 不是结果集）");
            t.check(
                "create table 的 kind 已变为 Direct",
                stmt.kind() == dm_odbc::StmtKind::Direct,
            );
        }
        drop(stmt);
    }

    // ------------------------------------------------------------------
    println!("=== 用例 2：同一预处理语句连续执行两次（核心验收项）===");
    {
        let mut stmt = conn.allocate_statement().expect("分配语句");
        t.ok(
            "prepare(insert … values (?, ?, ?))",
            stmt.prepare(&format!(
                "insert into {PROBE_TABLE} (id, name, amount) values (?, ?, ?)"
            )),
        );
        // T3.4 之前还不能绑定，所以这里只能验「个数对不对」——驱动的回答，不是我们数文本里的 `?`。
        t.check("param_count == 3", stmt.param_count() == 3);

        // 第一次执行：参数没绑。**实测**达梦回的是 `07002 / -70039 绑定的参数个数不正确`
        // （不是 ODBC 规范里的 `SQL_NEED_DATA`），所以它落回 `Server`——按 dm-error 的规矩，
        // 未登记的码一律原样保留，不猜语义。这里要验的是「码是 -70039 而不是 -70027」。
        t.expect_err(
            "exec() 未绑定参数 → 07002/-70039 绑定的参数个数不正确",
            stmt.exec(),
            |e| matches!(&e.kind, DmErrorKind::Server { code: -70039, .. }),
        );
        t.check(
            "未绑定时 kind 仍是 Prepared（可继续用）",
            stmt.kind().is_prepared(),
        );

        // 同一语句连续执行两次：这是 §12.2 点名要验的「不出状态错」。
        // 参数仍未绑，所以两次都会失败——要验的是**第二次失败的原因与第一次相同**，
        // 而且都不是 -70027。
        let first = stmt.exec().err().map(|e| format!("{:?}", e.kind));
        let second = stmt.exec().err().map(|e| format!("{:?}", e.kind));
        println!("  第一次 exec 的 kind：{first:?}");
        println!("  第二次 exec 的 kind：{second:?}");
        t.checks += 1;
        if first.is_some() && first == second {
            println!("  OK  连续两次执行给出同一个错因（没有累积成状态错）");
        } else {
            t.failures += 1;
            println!("  FAIL 连续两次执行的错因不一致");
        }
        drop(stmt);
    }

    // ------------------------------------------------------------------
    println!("=== 用例 3：select 连续执行两次 + 结果集不消费就重执行 ===");
    {
        let mut stmt = conn.allocate_statement().expect("分配语句");
        t.ok(
            "prepare(select id, name, amount from … order by id)",
            stmt.prepare(&format!(
                "select id, name, amount from {PROBE_TABLE} order by id"
            )),
        );
        t.check("select 的 param_count == 0", stmt.param_count() == 0);

        // 3.1 第一次执行并**故意不消费结果集**。
        if let Some(outcome) = t.ok("第一次 exec()", stmt.exec()) {
            t.check("select 被判为结果集", outcome.has_result_set());
            t.check("列数为 3", outcome.columns() == Some(3));
            t.check("执行后 is_cursor_open()", stmt.is_cursor_open());
        }
        // 3.2 结果集一行没取，直接再执行 —— 这就是真实世界触发 -70027 的场景。
        //     我们的 exec() 会先自动关游标，所以这里应该干干净净地成功。
        if let Some(outcome) = t.ok("未消费结果集就第二次 exec()（自动关游标）", stmt.exec())
        {
            t.check("第二次仍是结果集，列数 3", outcome.columns() == Some(3));
        }
        // 3.3 再来一次，确认不是「第二次侥幸」。
        t.ok("第三次 exec()", stmt.exec());

        // 3.4 显式关游标后再执行。
        t.ok("显式 close_cursor()", stmt.close_cursor());
        t.check("关游标后 is_cursor_open() 为假", !stmt.is_cursor_open());
        t.ok("关游标后再 exec()", stmt.exec());
        // 3.5 重复关游标（对没有游标的句柄是空操作）。
        t.ok("重复 close_cursor()", stmt.close_cursor());

        // 3.6 exec_direct 也是同一条语句句柄：先 prepare 过，再一次性下发。
        let outcome = t.ok(
            "同一句柄上 exec_direct(select count(*))",
            stmt.exec_direct(&format!("select count(*) from {PROBE_TABLE}")),
        );
        if let Some(outcome) = outcome {
            t.check("exec_direct 的 select 是结果集", outcome.has_result_set());
        }
        t.check(
            "exec_direct 后 kind 变回 Direct",
            stmt.kind() == dm_odbc::StmtKind::Direct,
        );

        // 3.7 在 Direct 状态下调 exec() 必须被拒（而不是撞出一个服务端状态错）。
        t.expect_err(
            "Direct 状态下 exec() → 本地拒绝（Config）",
            stmt.exec(),
            |e| matches!(e.kind, DmErrorKind::Config(_)),
        );
        drop(stmt);
    }

    // ------------------------------------------------------------------
    println!("=== 用例 4：DDL/DML 的影响行数 ===");
    {
        // 直接下发，不绑定参数：T3.4 之前只能这样造数据。
        let mut stmt = conn.allocate_statement().expect("分配语句");
        for id in 1..=3 {
            let outcome = t.ok(
                &format!("exec_direct(insert id={id})"),
                stmt.exec_direct(&format!(
                    "insert into {PROBE_TABLE} (id, name, amount) values ({id}, 'n{id}', {id}.25)"
                )),
            );
            if let Some(outcome) = outcome {
                t.check(
                    &format!(
                        "insert id={id} 的影响行数为 1（实际 {:?}）",
                        outcome.affected_rows()
                    ),
                    outcome.affected_rows() == Some(1),
                );
                t.check("insert 不是结果集", outcome.is_affected());
            }
        }
        let outcome = t.ok(
            "exec_direct(update 三行)",
            stmt.exec_direct(&format!("update {PROBE_TABLE} set amount = amount + 1")),
        );
        if let Some(outcome) = outcome {
            println!("  --  update 的影响行数：{:?}", outcome.affected_rows());
        }
        let outcome = t.ok(
            "exec_direct(select count(*))",
            stmt.exec_direct(&format!("select count(*) from {PROBE_TABLE}")),
        );
        if let Some(outcome) = outcome {
            t.check(
                "select 的 affected_rows() 为 None",
                outcome.affected_rows().is_none(),
            );
        }
        drop(stmt);
    }

    // ------------------------------------------------------------------
    println!("=== 用例 5：本该被拒的用法 ===");
    {
        let mut stmt = conn.allocate_statement().expect("分配语句");
        // 5.1 全新句柄上调 exec()：本地就拒绝，不能拿服务端的「状态不对」搪塞。
        t.expect_err("Fresh 状态下 exec() → Config", stmt.exec(), |e| {
            matches!(e.kind, DmErrorKind::Config(_))
        });

        // 5.2 带 `?` 的 exec_direct：本层不接参数。**实测**达梦回 `HY000 / -6804 需要更多的参数`
        //     而**不是** `SQL_NEED_DATA`，所以同样落回 `Server`——但码与消息都把原因说清楚了。
        t.expect_err(
            "exec_direct(带 ? 的语句) → HY000/-6804 需要更多的参数",
            stmt.exec_direct("select ? from dual"),
            |e| matches!(&e.kind, DmErrorKind::Server { code: -6804, .. }),
        );

        // 5.3 语法错误：必须分类成 Syntax（`-2007`），不是笼统的 Server。
        t.expect_err(
            "exec_direct(语法错误) → Syntax",
            stmt.exec_direct("seelct 1 from dual"),
            |e| matches!(e.kind, DmErrorKind::Syntax),
        );

        // 5.4 表不存在：必须分类成 ObjectNotFound（`-2111`）。
        t.expect_err(
            "exec_direct(表不存在) → ObjectNotFound",
            stmt.exec_direct("select 1 from DMD_ODBC_NO_SUCH_TABLE_XYZ"),
            |e| matches!(e.kind, DmErrorKind::ObjectNotFound),
        );

        // 5.5 语法错误之后同一句柄还能继续用（错误不得污染句柄状态）。
        t.ok(
            "语法错误之后同一句柄仍可 exec_direct",
            stmt.exec_direct("select 1 from dual"),
        );
        drop(stmt);
    }

    // ------------------------------------------------------------------
    println!("=== 用例 6：清场 ===");
    {
        let mut stmt = conn.allocate_statement().expect("分配语句");
        t.ok(
            "drop table（清场）",
            stmt.exec_direct(&format!("drop table {PROBE_TABLE}")),
        );
        drop(stmt);
    }
    let _ = conn.rollback();

    // ------------------------------------------------------------------
    println!();
    println!("=== 汇总 ===");
    println!("  检查项：{}", t.checks);
    println!("  失败项：{}", t.failures);
    println!(
        "  -70027（无效的游标状态）出现次数：{}  ← §6.8 硬指标，必须为 0",
        t.invalid_cursor
    );
    if t.failures == 0 && t.invalid_cursor == 0 {
        println!("结论：T3.3 真机验收通过");
    } else {
        println!("结论：未通过");
    }
}
