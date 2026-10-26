//! 最小示例：`cargo run --example basic`

use ew_operate_record::{OperationLog, Query, Recorder};
use time::format_description::well_known::Rfc3339;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join("ew_operate_record_example");
    std::fs::create_dir_all(&directory)?;
    let database = directory.join("operations.db");
    let _ = std::fs::remove_file(&database);

    let recorder = Recorder::open(&database)?;

    // 用户发起的操作才记录；系统内部动作不要调用 log()
    recorder.log(OperationLog::new("u-1024", "登录").result("成功"))?;
    recorder.log(
        OperationLog::new("u-1024", "删除订单")
            .target("order:O-88231")
            .result("成功")
            .detail("客户重复下单，已退款 12.5 元"),
    )?;
    recorder.log(
        OperationLog::new("u-2048", "导出报表")
            .with_user_name("韩梅梅")
            .target("report:sales-2026-10")
            .result("失败：无权限"),
    )?;

    recorder.flush()?;

    let entries = recorder.query(&Query::new().user("u-1024").limit(10))?;
    println!("u-1024 的最近操作：");
    for entry in &entries {
        println!(
            "{} {} {} {}",
            entry.occurred_at.format(&Rfc3339)?,
            entry.action,
            entry.target.as_deref().unwrap_or("-"),
            entry.result.as_deref().unwrap_or("-"),
        );
    }

    println!("\n统计: {:?}", recorder.stats());
    recorder.close()?;
    println!("数据库: {}", database.display());
    Ok(())
}
