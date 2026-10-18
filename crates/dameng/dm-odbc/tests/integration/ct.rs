//! 契约测试 CT-xx 的**真机侧**证据（设计方案 §10.2）。
//!
//! 离线能验的一半尽量留在各 crate 的单测里（例如 `dm-core/src/connstr.rs` 的
//! `ct02_structured_render_carries_no_dsn`）；这里只放**必须连库**才成立的那一半，免得把
//! 「需要达梦」的判据伪装成离线门禁。

use dm_core::DmConfig;
use dm_odbc::OdbcConnection;

use crate::common::dm_test;

// CT-02：**不依赖任何 DSN** 也能建连。
//
// 做法是把夹具配置**只用结构化字段**重新表达一遍，再连一次：
//
// * 先把「夹具自己不含 `DSN=`」断言出来——夹具是用户手写的整串（`DM_TEST_DSN`），
//   若它自带 DSN，整组真机用例都会在「其实靠 DSN」的情况下显得通过；
// * 重建时**不带 `extra`**（`DSN` 只可能住在 `extra` 里），于是这一次建连不可能用到 DSN；
// * 连上之后必须真的能下一条语句：建连成功却立刻失效的连接没有证明力。
//
// 写成 `//` 而不是 `///`：`dm_test!` 展开后是一个宏调用，rustdoc 不为宏调用生成文档，
// `///` 会触发 `unused_doc_comments`，而门禁是 `-D warnings`。
dm_test!(fn ct02_connects_from_structured_fields_without_a_dsn(h) {
    let fixture = h.conn.config().clone();
    let fixture_str = fixture.to_connection_string();
    h.check_that(
        !fixture_str.to_ascii_uppercase().contains("DSN="),
        || format!("夹具自带 DSN，这条用例会变成假通过：{fixture_str}"),
    );

    let mut builder = DmConfig::builder()
        .driver(fixture.driver.clone())
        .server(fixture.server.clone())
        .port(fixture.port)
        .user(fixture.user.clone())
        .password(fixture.password.expose())
        .connection_timeout(fixture.connection_timeout)
        .login_timeout(fixture.login_timeout);
    if let Some(schema) = fixture.schema.clone() {
        builder = builder.schema(schema);
    }
    if let Some(code) = fixture.character_code.clone() {
        builder = builder.character_code(code);
    }
    if let Some(mode) = fixture.compatible_mode {
        builder = builder.compatible_mode(mode);
    }

    let fresh = match builder.build() {
        Ok(cfg) => cfg,
        Err(err) => {
            h.record("用结构化字段重建配置", &err);
            return;
        }
    };
    h.check(
        "重建的配置没有任何 extra（DSN 只可能住在 extra）",
        fresh.extra.is_empty(),
    );
    let fresh_str = fresh.to_connection_string();
    h.check_that(
        !fresh_str.to_ascii_uppercase().contains("DSN="),
        || format!("重建的配置渲染出了 DSN：{fresh_str}"),
    );

    match OdbcConnection::connect(&fresh) {
        // 之后 `h.conn` 就是那条「无 DSN」的连接——后续断言跑在它上面。
        Ok(conn) => h.conn = conn,
        Err(err) => {
            h.record("无 DSN 建连（只用结构化字段）", &err);
            return;
        }
    }

    let Some(rows) = h.try_rows("select 1 from dual") else {
        return;
    };
    h.eq("无 DSN 的连接能下语句并取回一行", rows.len(), 1);
});
