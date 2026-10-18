//! T3.1 真机验收探针：正确/错误口令、不存在的主机、`charset_check=error` 下的字符集错配。
//!
//! 跑法（环境变量口径与 T1.2 的 spike 一致）：
//!
//! ```text
//! cargo +stable-x86_64-pc-windows-msvc run -p dm-odbc --example connect_probe
//! ```
//!
//! 这里**故意用 `println!` 而不是断言**：T3.1 的验收要的是「真机上到底发生了什么」，
//! 断言会把没预料到的真实行为变成失败而不是信息。逐行看输出，别看退出码。

use std::time::Duration;

use dm_core::{CharsetCheck, DmConfig, SecretString};
use dm_error::DmErrorKind;
use dm_odbc::OdbcConnection;

/// 不存在的目标：TEST-NET-1（RFC 5737），保证不可路由，用来触发连接超时。
const UNROUTABLE_HOST: &str = "192.0.2.1";

fn base() -> DmConfig {
    DmConfig::from_env().expect("环境变量口径不合法（DM_SERVER/DM_USER/DM_PASSWORD…）")
}

fn describe(err: &dm_error::DmError) -> String {
    let mut out = format!("{:?} @ {:?}", err.kind, err.context.stage);
    for (i, d) in err.diags().iter().enumerate() {
        out.push_str(&format!(
            "\n      diag[{i}] state={:?} native={} msg={}",
            d.sqlstate_str(),
            d.native_code,
            d.message
        ));
    }
    out
}

fn main() {
    let mut failures = 0usize;

    // ---- 用例 1：正确口令，建连 + 自检 + init_sql + 探活 + 关闭 ----
    println!("=== 用例 1：正常建连 ===");
    {
        let mut cfg = base();
        cfg.charset_check = CharsetCheck::Error;
        cfg.init_sql = vec!["set schema SYSDBA".to_owned()];
        match OdbcConnection::connect(&cfg) {
            Ok(mut conn) => {
                let stats = conn.stats().clone();
                println!(
                    "  OK  建连耗时 {} µs，unicode={:?}，比对了字符集={}",
                    stats.connect_micros, stats.server_unicode, stats.charset_checked
                );
                println!("  Debug 形态：{conn:?}");
                match conn.ping() {
                    Ok(true) => println!("  OK  探活：连接活着"),
                    Ok(false) => {
                        failures += 1;
                        println!("  FAIL 探活：连接已死");
                    }
                    Err(err) => {
                        failures += 1;
                        println!("  FAIL 探活报错：{}", describe(&err));
                    }
                }
                match conn.rollback() {
                    Ok(()) => println!("  OK  回滚成功"),
                    Err(err) => {
                        failures += 1;
                        println!("  FAIL 回滚报错：{}", describe(&err));
                    }
                }
                match conn.close() {
                    Ok(()) => println!("  OK  显式关闭成功"),
                    Err(err) => {
                        failures += 1;
                        println!("  FAIL 关闭报错：{}", describe(&err));
                    }
                }
            }
            Err(err) => {
                failures += 1;
                println!("  FAIL 正常配置却连不上：{}", describe(&err));
            }
        }
    }

    // ---- 用例 2：错误口令 ----
    println!("=== 用例 2：错误口令 ===");
    {
        let mut cfg = base();
        cfg.password = SecretString::new("definitely-not-the-password");
        cfg.login_timeout = Duration::from_secs(5);
        match OdbcConnection::connect(&cfg) {
            Ok(_) => {
                failures += 1;
                println!("  FAIL 错误口令竟然连上了");
            }
            Err(err) => {
                let is_driver_not_found = matches!(err.kind, DmErrorKind::DriverNotFound { .. });
                println!(
                    "  OK  如期失败（误判成 DriverNotFound = {is_driver_not_found}）\n      {}",
                    describe(&err)
                );
                if is_driver_not_found {
                    failures += 1;
                    println!("  FAIL 口令错被误判成「驱动没装」");
                }
                if err.diags().is_empty() {
                    failures += 1;
                    println!("  FAIL 建连失败却没带诊断");
                }
            }
        }
    }

    // ---- 用例 3：不存在的主机 ----
    println!("=== 用例 3：不可路由主机（{UNROUTABLE_HOST}） ===");
    {
        let mut cfg = base();
        cfg.server = UNROUTABLE_HOST.to_owned();
        cfg.login_timeout = Duration::from_secs(3);
        cfg.connection_timeout = Duration::from_secs(3);
        let started = std::time::Instant::now();
        match OdbcConnection::connect(&cfg) {
            Ok(_) => {
                failures += 1;
                println!("  FAIL 不可路由主机竟然连上了");
            }
            Err(err) => {
                println!(
                    "  OK  如期失败（{} ms）\n      {}",
                    started.elapsed().as_millis(),
                    describe(&err)
                );
            }
        }
    }

    // ---- 用例 4：character_code=PG_UTF8 + 默认的 charset_check=error 必须放行 ----
    //      这是真机证伪出来的回归点：服务端字符集由 dminit 定死，CHARACTER_CODE 只改
    //      驱动侧转换目标，所以 PG_UTF8 下 unicode() 仍是 0，连接完全正常。
    println!("=== 用例 4：character_code=PG_UTF8 + charset_check=error（不得假阳性阻断） ===");
    {
        let mut cfg = base();
        cfg.character_code = Some("PG_UTF8".to_owned());
        cfg.charset_check = CharsetCheck::Error;
        cfg.login_timeout = Duration::from_secs(5);
        let t0 = std::time::Instant::now();
        let result = OdbcConnection::connect(&cfg);
        let elapsed = t0.elapsed();
        println!(
            "  （本次建连 {} ms——驱动注册表已缓存，这是稳态耗时）",
            elapsed.as_millis()
        );
        match result {
            Ok(conn) => {
                let stats = conn.stats();
                if stats.server_unicode == Some(dm_odbc::UNICODE_GB18030) {
                    println!(
                        "  OK  如期放行：PG_UTF8 下服务端 unicode()={:?}（服务端字符集不随客户端关键字变），\
                         比对了字符集={}",
                        stats.server_unicode, stats.charset_checked
                    );
                } else {
                    failures += 1;
                    println!(
                        "  FAIL 服务端 unicode()={:?}，与本模块登记的 GB18030 基线不符",
                        stats.server_unicode
                    );
                }
            }
            Err(err) => {
                failures += 1;
                println!(
                    "  FAIL charset_check=error 假阳性阻断了工作正常的 PG_UTF8 连接：{}",
                    describe(&err)
                );
            }
        }
    }

    // ---- 用例 5：charset_check=warn 放行，且 off 时根本不查 ----
    println!("=== 用例 5：charset_check=warn / off ===");
    {
        let mut cfg = base();
        cfg.character_code = Some("PG_UTF8".to_owned());
        cfg.charset_check = CharsetCheck::Warn;
        cfg.login_timeout = Duration::from_secs(5);
        match OdbcConnection::connect(&cfg) {
            Ok(conn) => println!(
                "  OK  warn 放行（unicode={:?}，比对了字符集={}）",
                conn.server_unicode(),
                conn.stats().charset_checked
            ),
            Err(err) => {
                failures += 1;
                println!("  FAIL charset_check=warn 不该拦：{}", describe(&err));
            }
        }

        let mut cfg = base();
        cfg.charset_check = CharsetCheck::Off;
        cfg.login_timeout = Duration::from_secs(5);
        match OdbcConnection::connect(&cfg) {
            Ok(conn) => {
                let stats = conn.stats();
                if stats.server_unicode.is_none() && !stats.charset_checked {
                    println!("  OK  off 时完全跳过自检（unicode=None，比对了字符集=false）");
                } else {
                    failures += 1;
                    println!(
                        "  FAIL off 却做了自检（unicode={:?}，checked={}）",
                        stats.server_unicode, stats.charset_checked
                    );
                }
            }
            Err(err) => {
                failures += 1;
                println!("  FAIL charset_check=off 不该拦：{}", describe(&err));
            }
        }
    }

    // ---- 用例 6：缺驱动时必须是 DriverNotFound ----
    println!("=== 用例 6：驱动名不存在 → DriverNotFound ===");
    {
        let mut cfg = base();
        cfg.driver = "NO SUCH DM DRIVER 12345".to_owned();
        cfg.login_timeout = Duration::from_secs(5);
        match OdbcConnection::connect(&cfg) {
            Ok(_) => {
                failures += 1;
                println!("  FAIL 不存在的驱动名竟然连上了");
            }
            Err(err) => {
                let mut ok = true;
                match &err.kind {
                    DmErrorKind::DriverNotFound { tried, hint } => {
                        println!(
                            "  OK  DriverNotFound：tried 有 {} 条，hint 非空={}",
                            tried.len(),
                            !hint.is_empty()
                        );
                        // 注册表预检在建连前拦下，没碰 ODBC，所以这里**不该**有诊断；
                        // 有诊断反而说明走错了路径（那是 SQLDriverConnect 才该发生的事）。
                        if !err.diags().is_empty() {
                            println!(
                                "  注意 预检路径竟然带了 {} 条 ODBC 诊断（说明不是预检拦下的）",
                                err.diags().len()
                            );
                        }
                    }
                    other => {
                        ok = false;
                        println!("  FAIL 报的不是 DriverNotFound 而是 {other:?}");
                    }
                }
                if !ok {
                    failures += 1;
                }
            }
        }
    }

    println!("\n合计失败用例：{failures}");
    if failures > 0 {
        std::process::exit(1);
    }
}
