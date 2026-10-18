//! 列元数据探针：真机上 `SQLColAttribute(SQL_DESC_NULLABLE)` 到底给了什么。
//!
//! 背景：`result_column_metadata_reports_nullability_and_precision` 用例最初发现三列
//! （`NOT NULL` 的 `NOT_NULL_COL`、可空的 `NULL_COL`、带精度的 `DEC_COL`）**全被判成可空**。
//! `describe_columns` 的写法是对的（`col_nullability` → `could_be_nullable()`），
//! 所以要分辨是「驱动一律回 SQL_NULLABLE」还是「我们问错了地方」。
//!
//! **结论**（已写进设计文档 §7「真机实测补注（M3/T3.7）」第 16 条）：
//! 真实表上 `NOT NULL` 的列报 `SQL_NO_NULLS`，可空列报 `SQL_NULLABLE`，**但表达式列
//! 一律报 `SQL_NULLABLE`**——所以列可空性只能对真实表列下判断。用例后来改成用真实表列断言。
//!
//! 跑法（PowerShell）：
//! ```text
//! $env:DM_SERVER='localhost'; $env:DM_USER='SYSDBA'; $env:DM_PASSWORD='SYSDBA'
//! cargo run -p dm-odbc --example meta_probe
//! ```
//!
//! **保留**（不删）：它是这条「实测登记」结论的可重跑证据——达梦升级后重跑一遍，
//! 就知道补注 16 是否还成立。只打印，不做断言。

use dm_core::DmConfig;
use dm_odbc::OdbcConnection;
use odbc_api::handles::{SqlResult, Statement as _};

fn main() {
    let cfg = DmConfig::from_env().expect("需要 DM_* 具名环境变量");
    let mut conn = OdbcConnection::connect(&cfg).expect("连接失败");
    println!(
        "连接成功：length_in_char={:?}",
        conn.config().length_in_char
    );

    {
        let mut stmt = conn.allocate_statement().expect("alloc");
        for sql in [
            "DROP TABLE IF EXISTS DM_P_META",
            "CREATE TABLE DM_P_META (ID INT NOT NULL PRIMARY KEY, N INT NOT NULL, \
             U VARCHAR(20), D DECIMAL(10,3), T TIMESTAMP)",
        ] {
            stmt.exec_direct(sql).expect("DDL");
            let _ = stmt.close_cursor();
        }
    }

    let mut stmt = conn.allocate_statement().expect("alloc");
    stmt.exec_direct("SELECT ID, N, U, D, T, N + 1 AS EXPR FROM DM_P_META")
        .expect("select");
    let raw = stmt.statement();
    let cols = match raw.num_result_cols() {
        SqlResult::Success(n) | SqlResult::SuccessWithInfo(n) => n,
        other => panic!("SQLNumResultCols 失败：{other:?}"),
    };
    let cols = u16::try_from(cols.max(0)).expect("列数超界");
    println!("\n列数={cols}");
    for index in 1..=cols {
        let mut buf = Vec::new();
        let name = if raw.col_name(index, &mut buf).is_success() {
            // 驱动的列名是窄字符；直接按字节渲染就够辨认了。
            String::from_utf8_lossy(
                &buf.iter()
                    .map(|c| u8::try_from(*c).unwrap_or(b'?'))
                    .collect::<Vec<u8>>(),
            )
            .trim_end_matches('\0')
            .to_owned()
        } else {
            "<问不出来>".to_owned()
        };
        let nullability = raw.col_nullability(index);
        let flag = match &nullability {
            SqlResult::Success(n) | SqlResult::SuccessWithInfo(n) => {
                format!("could_be_nullable={}", n.could_be_nullable())
            }
            other => format!("{other:?}"),
        };
        let concise = match raw.col_concise_type(index) {
            SqlResult::Success(t) | SqlResult::SuccessWithInfo(t) => format!("{}", t.0),
            other => format!("{other:?}"),
        };
        println!("  [{index}] {name:<6} {nullability:?} => {flag} | concise={concise}");
    }
    let _ = stmt.close_cursor();
    drop(stmt);

    // ---- 二进制列的 `length()` 口径 ----
    // `binary_all_byte_values_round_trip` 实测：往 `VARBINARY(200)` 里写 200 字节，
    // `rawtohex` 给 400 个十六进制字符（=200 字节，正确），但 `length(B)` 回 **400**。
    // 这里把 `length` / `lengthb` / `octet_length` 三个都问一遍，弄清楚它到底在数什么。
    {
        let mut stmt = conn.allocate_statement().expect("alloc");
        for sql in [
            "DROP TABLE IF EXISTS DM_P_BIN",
            "CREATE TABLE DM_P_BIN (ID INT, V3 VARBINARY(300), V200 VARBINARY(200), LB BLOB)",
        ] {
            stmt.exec_direct(sql).expect("DDL");
            let _ = stmt.close_cursor();
        }
        drop(stmt);

        let mut stmt = conn.allocate_statement().expect("alloc");
        stmt.exec_direct("insert into DM_P_BIN (ID, V3) values (1, 0xAABBCC)")
            .expect("insert");
        let _ = stmt.close_cursor();
        drop(stmt);

        {
            let sql = "select length(V3) as LEN, lengthb(V3) as LENB, octet_length(V3) as OCT from DM_P_BIN where ID = 1";
            let mut stmt = conn.allocate_statement().expect("alloc");
            match stmt.exec_direct(sql) {
                Ok(_) => match dm_odbc::describe_columns(&mut stmt, None) {
                    Ok(columns) => {
                        let mut scratch = dm_odbc::FetchScratch::new();
                        let mut buf = dm_core::RowBuf::new(columns.len());
                        let value =
                            match dm_odbc::fetch(&mut stmt, &columns, &mut buf, &mut scratch) {
                                Ok(_) => columns
                                    .iter()
                                    .enumerate()
                                    .map(|(index, column)| {
                                        let text = buf
                                            .row(0)
                                            .and_then(|row| row.get(index))
                                            .map(|v| format!("{v:?}"))
                                            .unwrap_or_else(|| "<无值>".to_owned());
                                        format!("{}={text}", column.name)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("  "),
                                Err(err) => format!("取值失败：{err:?}"),
                            };
                        println!("  3 字节 VARBINARY ⇒ {value}");
                    }
                    Err(err) => println!("  describe 失败：{err:?}"),
                },
                Err(err) => println!("  `{sql}` 失败：{err:?}"),
            }
            let _ = stmt.close_cursor();
            drop(stmt);
        }
    }
    let _ = conn.close();
}

/// 让 `SqlResult` 好读一点（真机上达梦偶尔给 `SuccessWithInfo`）。
trait SqlResultExt<T> {
    fn is_success(&self) -> bool;
}

impl<T> SqlResultExt<T> for SqlResult<T> {
    fn is_success(&self) -> bool {
        matches!(self, SqlResult::Success(_) | SqlResult::SuccessWithInfo(_))
    }
}
