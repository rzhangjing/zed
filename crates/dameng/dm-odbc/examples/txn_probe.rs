//! 事务收尾探针：达梦在什么操作之后**释放连接句柄会失败**。
//!
//! 背景：`txn_rollback_discards_and_commit_persists` 与
//! `txn_rollback_with_autocommit_on_keeps_connection_usable` 两个用例在收尾时
//! panic 在 `odbc-api/src/handles.rs:71`——`SQLFreeHandle(SQL_HANDLE_DBC)` 回了 `-1`。
//! 反汇编式的排查只证明了「是连接句柄释放失败」，不知道**哪个操作把它弄坏的**。
//!
//! 这份探针逐个试：每轮都新建连接 → 做一件事 → 正常 `SQLDisconnect` → 释放句柄。
//! 用 `catch_unwind` 把 `Drop` 里那个 panic 接住，于是「哪轮炸」就是答案。
//!
//! **结论**（已写进设计文档 §7「真机实测补注（M3/T3.7）」第 15 条）：
//! 触发条件是「`autocommit = OFF` **且事务还开着**」——包括 `begin()` 之后只跑了一句
//! `SELECT` 这种隐式开着的情况。修法是 `OdbcConnection` 记 `autocommit` 状态，
//! `disconnect` 时若为 OFF 先 `rollback` 再断开（**回滚**而不是 `SET AUTOCOMMIT ON`：
//! 按 ODBC 规范后者会提交未完成事务，等于替调用方做了「保存」这个破坏性决定）。
//!
//! 跑法：
//! ```text
//! $env:DM_SERVER='localhost'; $env:DM_USER='SYSDBA'; $env:DM_PASSWORD='SYSDBA'
//! cargo run -p dm-odbc --example txn_probe
//! ```
//!
//! **保留**（不删）：9 个场景的正确/失败对照是这条结论的可重跑证据，也是
//! 「为什么必须回滚、不能改成提交」的现场记录。达梦升级后重跑一遍即可复核。

use std::panic::{AssertUnwindSafe, catch_unwind};

use dm_core::DmConfig;
use dm_odbc::OdbcConnection;

fn connect() -> OdbcConnection {
    let cfg = DmConfig::from_env().expect("需要 DM_* 具名环境变量");
    OdbcConnection::connect(&cfg).expect("连接失败")
}

/// 跑一轮：建连 → `body` → `SQLDisconnect` → 释放句柄。返回「这一轮炸了吗」。
fn round(label: &str, body: impl FnOnce(&mut OdbcConnection)) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut conn = connect();
        body(&mut conn);
        match conn.close() {
            Ok(()) => eprintln!("    SQLDisconnect: Ok"),
            Err(err) => eprintln!("    SQLDisconnect: Err({})", err.kind),
        }
    }));
    match outcome {
        Ok(()) => println!("[ok]   {label}"),
        Err(_) => println!("[BOOM] {label}  ← SQLFreeHandle(SQL_HANDLE_DBC) 失败"),
    }
}

fn main() {
    // 基准：什么都不做。
    round("基准（建连即断）", |_| {});

    // 一条普通语句。
    round("执行 select 1", |conn| {
        let mut stmt = conn.allocate_statement().expect("alloc");
        stmt.exec_direct("select 1 from dual").expect("select");
        stmt.close_cursor().expect("close");
    });

    // 手动事务：begin → rollback。
    round("begin → rollback", |conn| {
        conn.begin().expect("begin");
        conn.rollback().expect("rollback");
    });

    // 手动事务：begin → commit。
    round("begin → commit", |conn| {
        conn.begin().expect("begin");
        conn.commit().expect("commit");
    });

    // autocommit=ON 时直接 rollback（达梦可能报「没有活动事务」）。
    round("autocommit=ON → rollback", |conn| match conn.rollback() {
        Ok(()) => eprintln!("    rollback: Ok"),
        Err(err) => eprintln!("    rollback: Err({})", err.kind),
    });

    // autocommit=ON 时直接 commit。
    round("autocommit=ON → commit", |conn| match conn.commit() {
        Ok(()) => eprintln!("    commit: Ok"),
        Err(err) => eprintln!("    commit: Err({})", err.kind),
    });

    // 手动开关 autocommit（`txn_manual_autocommit_toggle` 的路径）。
    round(
        "set_autocommit(false) → rollback → set_autocommit(true)",
        |conn| {
            conn.set_autocommit(false).expect("off");
            conn.rollback().expect("rollback");
            conn.set_autocommit(true).expect("on");
        },
    );

    // 完整事务 + 真实 DML（最接近失败用例的路径）。
    round("begin → insert → commit", |conn| {
        conn.begin().expect("begin");
        let mut stmt = conn.allocate_statement().expect("alloc");
        stmt.exec_direct("select 1 from dual").expect("select");
        stmt.close_cursor().expect("close");
        drop(stmt);
        conn.commit().expect("commit");
    });

    // 事务里写入后又回滚，再继续用。
    round("begin → insert → rollback → select", |conn| {
        conn.begin().expect("begin");
        let mut stmt = conn.allocate_statement().expect("alloc");
        stmt.exec_direct("select 1 from dual").expect("select");
        stmt.close_cursor().expect("close");
        drop(stmt);
        conn.rollback().expect("rollback");
        let mut stmt2 = conn.allocate_statement().expect("alloc");
        stmt2.exec_direct("select 2 from dual").expect("select2");
        stmt2.close_cursor().expect("close2");
        drop(stmt2);
    });

    // ---- 二分：到底要「事务里有语句」「回滚后有语句」还是两者都要 ----

    // A：事务里有语句，回滚后不再用语句。
    round("A: begin → select → rollback", |conn| {
        conn.begin().expect("begin");
        let mut stmt = conn.allocate_statement().expect("alloc");
        stmt.exec_direct("select 1 from dual").expect("select");
        stmt.close_cursor().expect("close");
        drop(stmt);
        conn.rollback().expect("rollback");
    });

    // B：事务里没语句，回滚后用语句。
    round("B: begin → rollback → select", |conn| {
        conn.begin().expect("begin");
        conn.rollback().expect("rollback");
        let mut stmt = conn.allocate_statement().expect("alloc");
        stmt.exec_direct("select 1 from dual").expect("select");
        stmt.close_cursor().expect("close");
        drop(stmt);
    });

    // C：只有回滚后的语句（事务外）。
    round(
        "C: select → rollback → select（autocommit=ON）",
        |conn| {
            let mut stmt = conn.allocate_statement().expect("alloc");
            stmt.exec_direct("select 1 from dual").expect("select");
            stmt.close_cursor().expect("close");
            drop(stmt);
            let _ = conn.rollback();
            let mut stmt2 = conn.allocate_statement().expect("alloc");
            stmt2.exec_direct("select 2 from dual").expect("select2");
            stmt2.close_cursor().expect("close2");
            drop(stmt2);
        },
    );

    // D：事务里有语句 + 提交（对照「回滚」）。
    round("D: begin → select → commit → select", |conn| {
        conn.begin().expect("begin");
        let mut stmt = conn.allocate_statement().expect("alloc");
        stmt.exec_direct("select 1 from dual").expect("select");
        stmt.close_cursor().expect("close");
        drop(stmt);
        conn.commit().expect("commit");
        let mut stmt2 = conn.allocate_statement().expect("alloc");
        stmt2.exec_direct("select 2 from dual").expect("select2");
        stmt2.close_cursor().expect("close2");
        drop(stmt2);
    });

    // ---- 验证假设：坏状态 = 「断开时 autocommit=OFF 且有一条**隐式**事务开着」 ----

    // E：显式事务开着就断开。
    round(
        "E: begin → select → 断开（显式事务开着）",
        |conn| {
            conn.begin().expect("begin");
            let mut stmt = conn.allocate_statement().expect("alloc");
            stmt.exec_direct("select 1 from dual").expect("select");
            stmt.close_cursor().expect("close");
            drop(stmt);
        },
    );

    // F：关掉 autocommit，跑一条语句（开隐式事务），断开。
    round(
        "F: set_autocommit(false) → select → 断开（隐式事务开着）",
        |conn| {
            conn.set_autocommit(false).expect("off");
            let mut stmt = conn.allocate_statement().expect("alloc");
            stmt.exec_direct("select 1 from dual").expect("select");
            stmt.close_cursor().expect("close");
            drop(stmt);
        },
    );

    // G：回滚把隐式事务收掉，再断开。
    round(
        "G: begin → rollback → select → rollback → 断开",
        |conn| {
            conn.begin().expect("begin");
            conn.rollback().expect("rollback");
            let mut stmt = conn.allocate_statement().expect("alloc");
            stmt.exec_direct("select 1 from dual").expect("select");
            stmt.close_cursor().expect("close");
            drop(stmt);
            conn.rollback().expect("rollback again");
        },
    );

    // H：显式事务里跑语句 → 提交 → 再跑语句 → 提交 → 断开。
    round(
        "H: begin → select → commit → select → commit → 断开",
        |conn| {
            conn.begin().expect("begin");
            for _ in 0..2 {
                let mut stmt = conn.allocate_statement().expect("alloc");
                stmt.exec_direct("select 1 from dual").expect("select");
                stmt.close_cursor().expect("close");
                drop(stmt);
                conn.commit().expect("commit");
            }
        },
    );

    // I：关掉 autocommit，**不跑语句**，断开。
    round(
        "I: set_autocommit(false) → 断开（没有隐式事务）",
        |conn| {
            conn.set_autocommit(false).expect("off");
        },
    );
}
