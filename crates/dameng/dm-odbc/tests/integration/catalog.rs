//! 错误码目录的一致性用例（M6 / T6.5，G6 判据「错误码目录入 CI」）。
//!
//! 这个文件**不需要达梦**：它守的是「代码 / JSON / 分类器」三者不漂移，因此跟着
//! `cargo test --workspace --all-features` 一起在 CI 里跑。真机那一半（每个生码都
//! 在 DM8 上真的触发一遍）在 [`crate::errors`] / [`crate::params`] / [`crate::lob`] 里，
//! 目录条目里的 `covered_by` 写的就是那些用例名。
//!
//! ## 改了目录怎么更新 JSON
//!
//! ```text
//! $env:DM_CATALOG_UPDATE='1'; cargo +stable-x86_64-pc-windows-msvc test -p dm-odbc --test integration catalog
//! ```
//!
//! 然后**不带**环境变量再跑一次，确认它是绿的（这一步就是 CI 会做的事）。

use std::path::PathBuf;

use dm_error::catalog::{self, ENTRIES};
use dm_error::{DmErrorKind, SQLSTATE_LEN, codes};

/// 仓库根目录（`crates/dm-odbc/../..`）。
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 把条目里的 `SQLSTATE` 文本变成分类器要的 5 字节。
///
/// 条目里写空串表示「没观测到 `SQLSTATE`」，对应全零（驱动未提供）。
fn sqlstate_bytes(text: &str) -> [u8; SQLSTATE_LEN] {
    if text.is_empty() {
        [0u8; SQLSTATE_LEN]
    } else {
        dm_error::sqlstate_from_str(text)
    }
}

/// `docs/errors-catalog.json` 必须是 [`dm_error::catalog::render_json`] 的原样输出。
///
/// 这是**唯一**允许改动那份 JSON 的途径：它由代码生成，手改必然在下次跑测试时被打回。
#[test]
fn errors_catalog_json_is_in_sync_with_the_module() {
    let path = repo_root().join("docs").join("errors-catalog.json");
    let rendered = catalog::render_json();
    let actual = std::fs::read_to_string(&path);
    if matches!(&actual, Ok(text) if text == &rendered) {
        return;
    }
    if std::env::var_os("DM_CATALOG_UPDATE").is_some() {
        std::fs::write(&path, &rendered).expect("写入 docs/errors-catalog.json");
        println!("已重新生成 {}", path.display());
        return;
    }
    let why = match actual {
        Ok(text) => format!(
            "磁盘上 {} 字节，模块渲染 {} 字节",
            text.len(),
            rendered.len()
        ),
        Err(err) => format!("读不出来：{err}"),
    };
    panic!(
        "docs/errors-catalog.json 与 dm_error::catalog::ENTRIES 不一致（{why}）。\n\
         重新生成：$env:DM_CATALOG_UPDATE='1'; \
         cargo +stable-x86_64-pc-windows-msvc test -p dm-odbc --test integration catalog"
    );
}

/// 每登记一个原生码，目录里就要有一条对应条目——两处清单不许分叉。
#[test]
fn every_registered_code_has_a_catalog_entry() {
    for code in codes::REGISTERED {
        assert!(
            ENTRIES.iter().any(|entry| entry.native_code == code),
            "已登记的原生码 {code} 在目录里没有条目——登记了码就要在 §6.8 的目录里留痕"
        );
    }
}

/// 目录说某个码归到哪一类，分类器就必须**真的**给出那一类。
///
/// 这条把「文档承诺」变成「可执行的断言」：把 `classified` 从 `false` 改成 `true`
/// 而忘了登记进 [`codes`]，测试立刻红——因为分类器那时还只会给它 [`DmErrorKind::Server`]。
#[test]
fn catalog_kinds_match_the_classifier() {
    for entry in ENTRIES {
        let kind = DmErrorKind::classify_server(
            sqlstate_bytes(entry.sqlstate),
            entry.native_code,
            entry.detail,
        );
        assert_eq!(
            catalog::kind_name(&kind),
            entry.kind,
            "{} / {} 的分类与目录不符",
            if entry.sqlstate.is_empty() {
                "(无 SQLSTATE)"
            } else {
                entry.sqlstate
            },
            entry.native_code
        );
    }
}

/// G6 判据「错误码目录覆盖 ≥ 11 类错误」。
///
/// 这里说的是**实测过的生码条目**数（§6.8 那张表的行数），不是 [`DmErrorKind`] 的
/// 变体数——达梦给一个生码、我们故意不分类，也算采集到了一条。
#[test]
fn catalog_covers_at_least_eleven_measured_codes() {
    let measured = ENTRIES.iter().filter(|entry| entry.verified).count();
    assert!(
        measured >= 11,
        "G6 判据要求目录覆盖 ≥ 11 类实测错误，现在只有 {measured}"
    );
}

/// 目录的文本快照：目录一改，`insta` 就给出可读的 diff，方便在评审里逐字看。
///
/// 与上面的「JSON 逐字节一致」是**两道**保险：那一份保证磁盘上的文件与代码同源，
/// 这一份保证代码本身的改动在评审里看得见（`cargo insta review`）。
#[test]
fn errors_catalog_snapshot() {
    insta::assert_snapshot!(catalog::render_json());
}
