//! 冒烟：连得上、能下一条语句、探活不跑 SQL、建表脚本幂等。

use dm_core::DmValue;

use crate::common::{Harness, T_ERR, T_LOB, T_STR, T_TXN, T_TYPES, cell_text, dm_test};

/// `select count(*)` 的文本结果（`None` 表示连这个表都查不到）。
fn row_count(h: &mut Harness, table: &str) -> Option<String> {
    h.try_rows(&format!("select count(*) from {table}"))
        .and_then(|rows| rows.first().and_then(|r| cell_text(r, 0)))
}

dm_test!(fn smoke_connects_and_reports_the_server_character_set(h) {
    // §5.5 步骤⑥：建连时做字符集自检；§8.3 的判据是「`unicode()` 落在声明支持集里」。
    let unicode = h.conn.server_unicode();
    h.check("服务端字符集自检有结果", unicode.is_some());
    h.check_that(
        unicode.is_some_and(|u| dm_odbc::SUPPORTED_SERVER_UNICODE.contains(&u)),
        || {
            format!(
                "unicode() = {unicode:?} 不在声明支持集 {:?} 里",
                dm_odbc::SUPPORTED_SERVER_UNICODE
            )
        },
    );
});

dm_test!(fn smoke_ping_does_not_run_sql(h) {
    // 探活走 `SQL_ATTR_CONNECTION_DEAD`，不跑 `select 1`（§4.2：一次往返 29–34 µs，
    // 探活不该花这个钱，也不该在服务端留下语句痕迹）。
    match h.conn.ping() {
        Ok(alive) => h.eq("刚建好的连接是活的", alive, true),
        Err(err) => h.record("ping", &err),
    }
});

dm_test!(fn smoke_select_one_from_dual(h) {
    let Some(rows) = h.try_rows("select 1 from dual") else {
        return;
    };
    h.eq("一行一列", rows.len(), 1);
    h.eq(
        "值是 1",
        rows.first().and_then(|r| cell_text(r, 0)),
        Some("1".to_owned()),
    );
});

dm_test!(fn smoke_ddl_is_affected_not_a_result_set(h) {
    // T3.3 的核心口径：产出形态**问驱动**（`SQLNumResultCols`），不看 SQL 文本。
    // DDL 必须是 `Affected`，否则取值层会去 `fetch` 一张没有游标的表。
    h.try_exec("drop table if exists DM_T_SMOKE_DDL");
    if let Some(outcome) = h.try_exec("create table DM_T_SMOKE_DDL (ID INT)") {
        h.eq("create table 不是结果集", outcome.has_result_set(), false);
        h.check("create table 归入 Affected 分支", outcome.is_affected());
    }
    if let Some(outcome) = h.try_exec("insert into DM_T_SMOKE_DDL values (1)") {
        h.eq("insert 不是结果集", outcome.has_result_set(), false);
        h.eq("insert 影响 1 行", outcome.affected_rows(), Some(1));
    }
    h.try_exec("drop table DM_T_SMOKE_DDL");
});

dm_test!(fn smoke_schema_fixture_is_idempotent(h) {
    // §10.3 / T3.7 的验收项之一：`schema.sql` 幂等可重建。
    //
    // 「幂等」有两种读法，弱的那种（连跑两次都不报错）**可以被静默 no-op 骗过去**，
    // 所以这里断言强的那种：第二次应用必须**真的把表丢掉再建**。办法是在第二次应用之前
    // 塞一行进去，应用之后那一行必须不见——只有真重建才会消失。顺带把 5 张夹具表全部点名。
    h.apply_schema_strict();

    h.try_exec_params(
        &format!("insert into {T_STR} (ID, S) values (?, ?)"),
        &[DmValue::Int(1), DmValue::Text("见证行".to_owned())],
    );
    let witness = row_count(h, T_STR);
    h.eq("先把见证行写进去", witness, Some("1".to_owned()));

    // 第二次应用：走严格版，任何一条 DDL 失败都作为用例失败上报。
    h.apply_schema_strict();

    for table in [T_TYPES, T_STR, T_LOB, T_TXN, T_ERR] {
        let count = row_count(h, table);
        h.eq(
            &format!("{table} 重建后是空表"),
            count,
            Some("0".to_owned()),
        );
    }
});
