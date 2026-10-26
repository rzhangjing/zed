//! 简单基准：`cargo run --release --example bench`

use std::time::Instant;

use ew_operate_record::{OperationLog, Options, Query, Recorder};

const TOTAL: usize = 20_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join("ew_operate_record_bench");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;
    let database = directory.join("bench.db");

    let recorder = Recorder::open_with(
        &database,
        Options::default().queue_capacity(65_536).batch_size(1_024),
    )?;

    let started = Instant::now();
    for index in 0..TOTAL {
        recorder.log(
            OperationLog::new(format!("u-{}", index % 50), "删除订单")
                .target(format!("order:{index}"))
                .result("成功")
                .detail("客户重复下单，已退款 12.5 元"),
        )?;
    }
    recorder.flush()?;
    let elapsed = started.elapsed();
    println!(
        "写入 {TOTAL} 条: {:?}，{:.0} 条/秒",
        elapsed,
        TOTAL as f64 / elapsed.as_secs_f64()
    );

    let query_started = Instant::now();
    let entries = recorder.query(&Query::new().user("u-7").limit(100))?;
    println!(
        "按操作人查询 100 条: {:?}（命中 {} 条，本页 {} 条）",
        query_started.elapsed(),
        recorder.count(&Query::new().user("u-7"))?,
        entries.len()
    );

    recorder.checkpoint()?;
    let size = std::fs::metadata(&database)?.len()
        + std::fs::metadata(format!("{}-wal", database.display())).map_or(0, |meta| meta.len());
    println!(
        "数据库 {:.1} MiB，平均 {:.0} 字节/条",
        size as f64 / 1_048_576.0,
        size as f64 / TOTAL as f64
    );
    println!("统计: {:?}", recorder.stats());

    recorder.close()?;
    Ok(())
}
