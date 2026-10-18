//! 事务：`begin` / `commit` / `rollback` 与 `autocommit` 的交互（§6.6）。
//!
//! 三条纪律要在这里守住：
//! 1. `begin()` 之外的语句各自成事务（`autocommit=ON` 是建连默认）；
//! 2. **事务内不允许留活游标**——`h.try_exec` 里的语句用完即关，所以这里天然满足；
//! 3. 回滚之后连接必须还能用（达梦对「`autocommit=ON` 时回滚」有自己的说法，不能因此把连接搞坏）。

use crate::common::{T_TXN, dm_test};

dm_test!(fn txn_autocommit_insert_is_visible_without_commit(h) {
    h.apply_schema();

    h.try_exec(&format!("insert into {T_TXN} (ID, V) values (1, 11)"));
    h.expect_scalar(
        "autocommit 下插入立即可见",
        &format!("select count(*) from {T_TXN}"),
        "1",
    );
});

dm_test!(fn txn_rollback_discards_and_commit_persists(h) {
    h.apply_schema();
    h.try_exec(&format!("insert into {T_TXN} (ID, V) values (1, 11)"));

    if let Err(err) = h.conn.begin() {
        return h.record("begin", &err);
    }
    h.try_exec(&format!("insert into {T_TXN} (ID, V) values (2, 22)"));
    h.expect_scalar(
        "事务内同一连接看得见自己的写入",
        &format!("select count(*) from {T_TXN}"),
        "2",
    );

    if let Err(err) = h.conn.rollback() {
        return h.record("rollback", &err);
    }
    h.expect_scalar(
        "回滚只丢掉事务里的那一行",
        &format!("select count(*) from {T_TXN}"),
        "1",
    );
    h.expect_scalar(
        "回滚后剩余的是事务之前那一行",
        &format!("select V from {T_TXN} where ID = 1"),
        "11",
    );

    if let Err(err) = h.conn.begin() {
        return h.record("第二次 begin", &err);
    }
    h.try_exec(&format!("insert into {T_TXN} (ID, V) values (3, 33)"));
    if let Err(err) = h.conn.commit() {
        return h.record("commit", &err);
    }
    h.expect_scalar(
        "提交后两行都在",
        &format!("select count(*) from {T_TXN}"),
        "2",
    );
    h.expect_scalar(
        "提交的是事务里那一行",
        &format!("select V from {T_TXN} where ID = 3"),
        "33",
    );
});

dm_test!(fn txn_manual_autocommit_toggle(h) {
    h.apply_schema();

    if let Err(err) = h.conn.set_autocommit(false) {
        return h.record("set_autocommit(false)", &err);
    }
    h.try_exec(&format!("insert into {T_TXN} (ID, V) values (1, 11)"));
    h.expect_scalar(
        "关掉 autocommit 后同一连接仍看得见",
        &format!("select count(*) from {T_TXN}"),
        "1",
    );

    if let Err(err) = h.conn.rollback() {
        return h.record("手动模式下 rollback", &err);
    }
    h.expect_scalar(
        "手动模式下回滚生效",
        &format!("select count(*) from {T_TXN}"),
        "0",
    );

    if let Err(err) = h.conn.set_autocommit(true) {
        return h.record("set_autocommit(true)", &err);
    }
    h.try_exec(&format!("insert into {T_TXN} (ID, V) values (2, 22)"));
    h.expect_scalar(
        "恢复 autocommit 后插入立即生效",
        &format!("select count(*) from {T_TXN}"),
        "1",
    );
});

dm_test!(fn txn_rollback_with_autocommit_on_keeps_connection_usable(h) {
    h.apply_schema();

    // 这条不强制「回滚必须成功」：达梦完全可能回一个「没有活动事务」的错误。
    // 要守的是**连接不能因此坏掉**——错误发生了也照样能继续查询。
    match h.conn.rollback() {
        Ok(()) => h.check("autocommit 下 rollback 返回成功", true),
        Err(err) => {
            crate::common::note_error(&err);
            eprintln!(
                "[note] autocommit=ON 时 rollback 报错（不判失败）：{}",
                crate::common::describe(&err)
            );
        }
    }

    match h.scalar("select 1 from dual") {
        Ok(got) => h.eq("回滚之后连接仍然可用", got.as_deref(), Some("1")),
        Err(err) => h.record("回滚之后的查询", &err),
    }

    // 再走一遍完整事务，确认状态机没被上一次回滚卡住。
    if let Err(err) = h.conn.begin() {
        return h.record("回滚之后 begin", &err);
    }
    h.try_exec(&format!("insert into {T_TXN} (ID, V) values (9, 99)"));
    if let Err(err) = h.conn.commit() {
        return h.record("回滚之后 commit", &err);
    }
    h.expect_scalar(
        "回滚之后事务照常工作",
        &format!("select count(*) from {T_TXN}"),
        "1",
    );
});
