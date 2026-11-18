//! 中间件行为：写入、查询、翻页、校验、丢弃、关闭、清理。

use std::path::PathBuf;
use std::time::Duration;

use ew_operate_record::{Error, OperationLog, Options, Query, Recorder};
use time::OffsetDateTime;

fn temp_path(name: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join(name);
    (directory, path)
}

#[test]
fn records_and_queries_by_user_and_action() {
    let (_dir, path) = temp_path("ops.db");
    let recorder = Recorder::open(&path).expect("open");

    recorder
        .log(
            OperationLog::new("u-1", "删除订单")
                .with_user_name("李雷")
                .target("order:1")
                .result("成功")
                .detail("订单已退款"),
        )
        .expect("log");
    recorder
        .log(OperationLog::new("u-1", "登录").result("成功"))
        .expect("log");
    recorder
        .log(OperationLog::new("u-2", "删除订单").result("失败"))
        .expect("log");
    recorder.flush().expect("flush");

    let all = recorder.query(&Query::new()).expect("query");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].action, "删除订单", "最新在前");

    let by_user = recorder.query(&Query::new().user("u-1")).expect("query");
    assert_eq!(by_user.len(), 2);
    assert_eq!(by_user[1].user_name.as_deref(), Some("李雷"));

    let by_action = recorder
        .query(&Query::new().action("删除订单"))
        .expect("query");
    assert_eq!(by_action.len(), 2);

    let keyword = recorder
        .query(&Query::new().keyword("退款"))
        .expect("query");
    assert_eq!(keyword.len(), 1);
    assert_eq!(keyword[0].target.as_deref(), Some("order:1"));

    assert_eq!(recorder.count(&Query::new().user("u-2")).expect("count"), 1);

    recorder.close().expect("close");
}

#[test]
fn pagination_walks_backwards_with_timestamp_cursor() {
    let (_dir, path) = temp_path("ops.db");
    let recorder = Recorder::open(&path).expect("open");
    for index in 0..10 {
        recorder
            .log_user("u-1", format!("动作{index}"))
            .expect("log");
    }
    recorder.flush().expect("flush");

    let first = recorder.query(&Query::new().limit(4)).expect("page 1");
    assert_eq!(first.len(), 4);
    assert!(first[0].occurred_at > first[3].occurred_at);

    let second = recorder
        .query(&Query::new().limit(4).before(first[3].occurred_at))
        .expect("page 2");
    assert_eq!(second.len(), 4);
    assert!(second[0].occurred_at < first[3].occurred_at);
    assert!(second[0].occurred_at > second[3].occurred_at);

    let third = recorder
        .query(&Query::new().limit(4).before(second[3].occurred_at))
        .expect("page 3");
    assert_eq!(third.len(), 2);

    let mut times: Vec<_> = first
        .iter()
        .chain(second.iter())
        .chain(third.iter())
        .map(|entry| entry.occurred_at)
        .collect();
    times.sort_unstable();
    times.dedup();
    assert_eq!(times.len(), 10, "分页不能重复或漏行");

    recorder.close().expect("close");
}

#[test]
fn time_range_filters_entries() {
    let (_dir, path) = temp_path("ops.db");
    {
        let recorder = Recorder::open(&path).expect("open");
        recorder
            .log(OperationLog::new("u-1", "新操作"))
            .expect("log");
        recorder.close().expect("close");
    }
    let now = OffsetDateTime::now_utc();
    // 直接写一条 30 天前的记录（记录时间由 log() 打点，测试里用 SQL 造历史数据）。
    let old = i64::try_from((now - time::Duration::days(30)).unix_timestamp_nanos() / 1_000)
        .expect("micros");
    let connection = rusqlite::Connection::open(&path).expect("db");
    connection
        .execute(
            "INSERT INTO operation_log (occurred_at, user_id, action) VALUES (?1, 'u-1', '旧操作')",
            [old],
        )
        .expect("insert old");
    drop(connection);

    let recorder = Recorder::open(&path).expect("reopen");
    let recent = recorder
        .query(&Query::new().time_range(now - time::Duration::days(1), now))
        .expect("query");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].action, "新操作");

    recorder.close().expect("close");
}

#[test]
fn invalid_records_are_rejected_before_queueing() {
    let recorder = Recorder::open_in_memory().expect("open");
    assert!(matches!(
        recorder.log(OperationLog::new("", "登录")),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        recorder.log(OperationLog::new("u-1", " ")),
        Err(Error::Invalid(_))
    ));
    recorder.close().expect("close");
}

#[test]
fn oversized_detail_is_truncated() {
    let recorder = Recorder::open_in_memory().expect("open");
    recorder
        .log(OperationLog::new("u-1", "导入").detail("x".repeat(10_000)))
        .expect("log");
    recorder.flush().expect("flush");

    let entries = recorder.query(&Query::new()).expect("query");
    let detail = entries[0].detail.as_deref().expect("detail");
    assert!(detail.ends_with('…'));
    assert!(detail.chars().count() <= 4097);

    recorder.close().expect("close");
}

#[test]
fn close_flushes_queued_records_and_is_idempotent() {
    let (_dir, path) = temp_path("ops.db");
    let recorder =
        Recorder::open_with(&path, Options::default().queue_capacity(2048)).expect("open");
    for index in 0..500 {
        recorder
            .log_user("u-1", format!("动作{index}"))
            .expect("log");
    }
    recorder.close().expect("close");
    recorder.close().expect("close again");
    assert_eq!(recorder.count(&Query::new()).expect("count"), 500);
    assert_eq!(recorder.stats().written, 500);
}

#[test]
fn queue_full_is_reported_and_counted() {
    let (_dir, path) = temp_path("ops.db");
    let recorder = Recorder::open_with(
        &path,
        Options {
            queue_capacity: 4,
            batch_size: 1,
        },
    )
    .expect("open");

    // 用另一个连接占住写锁，让写入线程阻塞在 INSERT 上。
    let blocker = rusqlite::Connection::open(&path).expect("blocker");
    blocker
        .busy_timeout(Duration::from_millis(200))
        .expect("timeout");
    blocker.execute_batch("BEGIN IMMEDIATE;").expect("lock");

    let mut full = false;
    for index in 0..200 {
        match recorder.log_user("u-1", format!("动作{index}")) {
            Ok(()) => {}
            Err(Error::QueueFull { .. }) => {
                full = true;
                break;
            }
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
    assert!(full, "队列应当被写满");
    assert!(recorder.stats().dropped >= 1);

    blocker.execute_batch("ROLLBACK;").expect("unlock");
    drop(blocker);

    recorder.flush().expect("flush after unlock");
    assert!(
        !recorder
            .query(&Query::new().limit(10))
            .expect("query")
            .is_empty()
    );
    recorder.close().expect("close");
}

#[test]
fn concurrent_logging_keeps_every_record() {
    let (_dir, path) = temp_path("ops.db");
    let recorder = Recorder::open_with(
        &path,
        Options {
            queue_capacity: 8192,
            batch_size: 64,
        },
    )
    .expect("open");

    let mut handles = Vec::new();
    for worker in 0..4 {
        let recorder = recorder.clone();
        handles.push(std::thread::spawn(move || {
            for index in 0..100 {
                recorder
                    .log_user(format!("u-{worker}"), format!("动作{index}"))
                    .expect("log");
            }
        }));
    }
    for handle in handles {
        handle.join().expect("join");
    }
    recorder.flush().expect("flush");

    assert_eq!(recorder.count(&Query::new()).expect("count"), 400);
    assert_eq!(recorder.stats().written, 400);

    recorder.close().expect("close");
}

#[test]
fn prune_removes_only_records_older_than_cutoff() {
    let (_dir, path) = temp_path("ops.db");
    {
        let recorder = Recorder::open(&path).expect("open");
        recorder
            .log(OperationLog::new("u-1", "新操作"))
            .expect("log");
        recorder.close().expect("close");
    }

    let now = OffsetDateTime::now_utc();
    let connection = rusqlite::Connection::open(&path).expect("db");
    for index in 0..3 {
        let old =
            i64::try_from((now - time::Duration::days(30 + index)).unix_timestamp_nanos() / 1_000)
                .expect("micros");
        connection
            .execute(
                "INSERT INTO operation_log (occurred_at, user_id, action) \
                 VALUES (?1, 'u-1', '旧操作')",
                [old],
            )
            .expect("insert old");
    }
    drop(connection);

    let recorder = Recorder::open(&path).expect("reopen");
    let deleted = recorder
        .prune_before(now - time::Duration::days(7))
        .expect("prune");
    assert_eq!(deleted, 3);
    assert_eq!(recorder.count(&Query::new()).expect("count"), 1);

    recorder.close().expect("close");
}

#[test]
fn operations_after_close_are_rejected() {
    let recorder = Recorder::open_in_memory().expect("open");
    recorder.close().expect("close");

    assert!(matches!(
        recorder.log_user("u-1", "登录"),
        Err(Error::Closed)
    ));
    assert!(matches!(recorder.flush(), Err(Error::Closed)));
}

#[test]
fn migrates_legacy_id_schema_to_timestamp_primary_key() {
    let (_dir, path) = temp_path("ops.db");
    {
        let connection = rusqlite::Connection::open(&path).expect("legacy db");
        connection
            .execute_batch(
                "CREATE TABLE operation_log (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    user_id     TEXT NOT NULL,
                    user_name   TEXT,
                    action      TEXT NOT NULL,
                    target      TEXT,
                    result      TEXT,
                    detail      TEXT,
                    occurred_at INTEGER NOT NULL
                );
                CREATE INDEX idx_operation_log_user ON operation_log (user_id, id DESC);
                INSERT INTO operation_log (user_id, action, occurred_at) VALUES
                    ('u-1', '登录', 1761300000000),
                    ('u-1', '删除订单', 1761300000000);
                PRAGMA user_version = 1;",
            )
            .expect("legacy schema");
    }

    let recorder = Recorder::open(&path).expect("open migrated database");
    let entries = recorder.query(&Query::new()).expect("query");
    assert_eq!(entries.len(), 2, "旧库记录必须迁移过来");

    let mut times: Vec<OffsetDateTime> = entries.iter().map(|entry| entry.occurred_at).collect();
    times.sort_unstable();
    times.dedup();
    assert_eq!(times.len(), 2, "同一毫秒的旧记录迁移时顺延");

    recorder.close().expect("close");
}
