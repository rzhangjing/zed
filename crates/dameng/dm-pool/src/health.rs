//! 探活（设计方案 §5.7）。
//!
//! 走 `SQL_ATTR_CONNECTION_DEAD`（`Backend::ping`）而不是 `SELECT 1`：
//! §4.2 实测一次往返 p50 29–34 µs，探活不该花这个钱，也不该在服务端留下语句痕迹。
//!
//! 整个动作在**阻塞线程**上做——ODBC 是同步 API。这里的写法与
//! `PoolGuard::with_conn` 同一套纪律：`take_conn` 把裸连接交给闭包，
//! 闭包结束时**先定终态、再 `put_conn`**；如果期间句柄被 drop 了
//! （`guard_is_gone()`），由闭包认领归还责任把连接还给池，
//! **保证连接不会两头都不管，也不会被归还两次**。

use std::sync::Arc;
use std::time::Instant;

use dm_core::Backend;
use dm_error::{DmError, DmErrorKind, Stage};

use crate::conn::{ConnInner, ConnState, conn_busy_error};
use crate::exec::JobReply;
use crate::inner::PoolInner;

/// 探活。`Ok(true)` 活着，`Ok(false)` 已死（**不是错误**，`Backend::ping` 的契约）。
///
/// 调用方拿到 `Ok(false)` 或 [`DmError::is_connection_lost`] 为真的错误时应当淘汰连接；
/// 其它错误（例如驱动不支持这个属性）不该淘汰连接——那会把一条好连接白白扔掉。
pub(crate) async fn check<B: Backend>(
    pool: &Arc<PoolInner<B>>,
    conn: Arc<ConnInner<B>>,
) -> Result<bool, DmError> {
    let Some(raw) = conn.take_conn() else {
        return Err(conn_busy_error());
    };
    let job_conn = Arc::clone(&conn);
    let job_pool = Arc::clone(pool);

    let rx = pool
        .executor()
        .submit(move |reply: JobReply<Result<bool, DmError>>| {
            let mut raw = raw;
            let outcome = raw.ping();
            job_conn.set_state(ConnState::Idle);
            job_conn.put_conn(raw, Instant::now());
            // 探活不改业务状态，只处理「句柄已经先走了」这一种情况：
            // 那时没人会再调 publish，必须在这里把连接还给池。
            if job_conn.guard_is_gone() && job_conn.claim_publish() {
                job_pool.publish(job_conn, ConnState::Idle);
            }
            let _ = reply.send(outcome);
        })
        .await?;

    match rx.await {
        Ok(outcome) => outcome,
        Err(_) if pool.is_shutting_down() => Err(pool.closed_error()),
        Err(_) => Err(DmError::at(DmErrorKind::Io, Stage::Execute)),
    }
}
