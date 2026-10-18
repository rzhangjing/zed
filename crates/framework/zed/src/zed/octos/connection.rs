//! The host side of the OUP connection to the embedded Octos kernel.
//!
//! This is the seam every future Zed-side Octos feature goes through: with the
//! kernel in-process, "calling Octos" means sending newline-delimited JSON-RPC
//! frames over the duplex pipe `octos_cli::embedded::serve_io` serves, and
//! routing what comes back.
//!
//! Boundary (see `.rules`): Octos owns agent execution and the OUP runtime
//! contracts; the host owns the pipe, the session it opens, its own UI, app
//! records, adapters and — importantly — *authorization*. A tool declaration or
//! model text never authorizes access by itself, so any host tool registered
//! over this connection must re-check its own permissions where it runs.
//!
//! Frame shape (from `crates/octos/octos-cli/src/embedded.rs` and the OUP
//! specification): `{"jsonrpc": "2.0", "id": …, "method": …, "params": …}` for
//! requests, a frame with the same `id` for the reply, and frames without an
//! `id` for notifications the kernel initiates.
//!
//! What is deliberately not here yet: `peer/prepare` staging. Host *peers* are
//! not: [`super::peers`] speaks `peer/tools/register`, `peer/tool/call`,
//! `peer/tool/cancel` and `peer/tool/result` over this connection, the way the
//! OUP host-tool contract requires (see `crates/octos/octos-cli/src/peers/host_tools.rs`).

use futures::channel::{mpsc, oneshot};
use futures::future::{Either, select};
use futures::StreamExt as _;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, ReadHalf, WriteHalf};

/// Where a reply to a host request is delivered once it arrives.
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// Why [`Connection::call_with_timeout`] produced no reply frame.
///
/// Both variants are *host-side* facts about the wait, never a claim about what
/// the kernel did with the frame: a timeout does not cancel anything, and a
/// closed connection does not mean the kernel did not act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallFailure {
    /// No reply arrived within the caller's window. The frame was written;
    /// whether the kernel acted on it is unknown from here.
    Timeout { method: String, timeout: Duration },
    /// The connection ended before a reply arrived — the kernel's end of the
    /// pipe broke. Every waiting call is resolved this way (see
    /// [`fail_pending`]).
    Closed { method: String },
}

impl fmt::Display for CallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { method, timeout } => write!(
                f,
                "{method} was not answered within {timeout:?} (the kernel may still be working on it)"
            ),
            Self::Closed { method } => write!(
                f,
                "the kernel connection closed before {method} was answered"
            ),
        }
    }
}

impl std::error::Error for CallFailure {}

/// A host-side OUP connection to the embedded kernel.
///
/// Cheap to clone: every clone shares the writer queue and the pending-request
/// table, so several Zed modules can call the kernel concurrently.
#[derive(Clone)]
pub struct Connection {
    outgoing: mpsc::UnboundedSender<Value>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

impl Connection {
    /// Takes the host end of `pipe` and starts its reader and writer tasks.
    ///
    /// Must be called inside the kernel's Tokio runtime (the tasks are spawned
    /// onto the ambient runtime). Returns the connection plus the stream of
    /// frames the kernel initiates — notifications, and replies to requests
    /// this host did not make.
    pub fn new(pipe: DuplexStream) -> (Self, mpsc::UnboundedReceiver<Value>) {
        let (reader, writer) = tokio::io::split(pipe);
        let (outgoing, outgoing_rx) = mpsc::unbounded();
        let (incoming, incoming_rx) = mpsc::unbounded();
        let pending: Pending = Default::default();

        tokio::spawn(write_frames(writer, outgoing_rx, pending.clone()));
        tokio::spawn(read_frames(reader, pending.clone(), incoming));

        let connection = Self {
            outgoing,
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
        };
        (connection, incoming_rx)
    }

    /// How many calls are waiting for a reply. The regression test for the EOF
    /// cleanup reads this: a dead kernel must leave the table empty, not just
    /// settle the one call that happened to be observed.
    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.pending.lock().expect("pending request table").len()
    }

    /// Sends `method` and resolves with its reply.
    ///
    /// The receiver resolves with the raw reply frame — including one carrying
    /// an `error` object, because deciding what an error means is the caller's
    /// business. It resolves to `Err` only if the connection ended first.
    pub fn call(&self, method: &str, params: Value) -> oneshot::Receiver<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let (reply, reply_rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending request table")
            .insert(id.clone(), reply);

        let frame = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if self.outgoing.unbounded_send(frame).is_err() {
            // The writer is gone: drop the entry so the caller sees the close
            // instead of waiting forever.
            self.pending
                .lock()
                .expect("pending request table")
                .remove(&id);
        }
        reply_rx
    }

    /// [`Self::call`], bounded: resolves `Err` when the wait runs out instead
    /// of hanging on a kernel that never answers.
    ///
    /// The timeout is the caller's, not the kernel's: `Err(CallFailure::Timeout)`
    /// says nothing about whether the frame was acted on, and cancels nothing.
    /// The request's pending entry is deliberately left in place — the reply,
    /// if it ever comes, is delivered to a receiver nobody polls any more, and
    /// [`fail_pending`] clears it when the connection ends.
    pub async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CallFailure> {
        let reply = self.call(method, params);
        match select(reply, smol::Timer::after(timeout)).await {
            Either::Left((Ok(frame), _)) => Ok(frame),
            Either::Left((Err(_), _)) => Err(CallFailure::Closed {
                method: method.to_owned(),
            }),
            Either::Right(_) => Err(CallFailure::Timeout {
                method: method.to_owned(),
                timeout,
            }),
        }
    }

    /// Sends a notification the kernel initiated no request for: no `id`, so
    /// the kernel cannot reply to it. This is how the host answers
    /// `peer/tool/call` (`peer/tool/result`) and refuses a `peer/input`
    /// (`peer/input/reject`) — both are one-way frames by contract.
    ///
    /// Returns whether the frame reached the writer queue; `false` means the
    /// connection is closed, which the caller must treat as "the kernel never
    /// heard this" rather than as a delivery.
    pub fn notify(&self, method: &str, params: Value) -> bool {
        self.outgoing
            .unbounded_send(json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            }))
            .is_ok()
    }
}

async fn write_frames(
    mut writer: WriteHalf<DuplexStream>,
    mut frames: mpsc::UnboundedReceiver<Value>,
    pending: Pending,
) {
    while let Some(frame) = frames.next().await {
        if let Err(error) = writer.write_all(format!("{frame}\n").as_bytes()).await {
            log::warn!("[octos] could not write to the kernel: {error}");
            // A frame that could not be written cannot be answered: nothing
            // may stay waiting on this connection's writer.
            fail_pending(&pending);
            return;
        }
    }

    // Nothing left to send: closing our side is what ends the kernel's read
    // half, so `serve_io` can drain and return.
    let _ = writer.shutdown().await;
}

async fn read_frames(
    reader: ReadHalf<DuplexStream>,
    pending: Pending,
    incoming: mpsc::UnboundedSender<Value>,
) {
    let mut lines = BufReader::new(reader).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                    log::debug!("[octos] unparseable frame: {line}");
                    continue;
                };

                // A reply correlates by id; anything else is kernel-initiated.
                let reply = frame
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| pending.lock().expect("pending request table").remove(id));

                match reply {
                    Some(reply) => {
                        let _ = reply.send(frame);
                    }
                    None => {
                        if incoming.unbounded_send(frame).is_err() {
                            // The host dropped the notification stream. There
                            // is nobody left to route replies to either, so
                            // end the loop; the cleanup below settles every
                            // wait rather than leaving it unresolved.
                            break;
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                log::warn!("[octos] could not read from the kernel: {error}");
                break;
            }
        }
    }

    // EOF or a read failure: the kernel is gone, so no reply that is still
    // expected will ever arrive. Dropping the senders is what settles every
    // waiting `call`/`call_with_timeout` as `Err` instead of leaving the
    // futures unresolved forever (the regression this pins).
    fail_pending(&pending);
}

/// Ends every wait on a connection that can no longer deliver replies.
///
/// Dropping the `oneshot::Sender`s resolves each waiting receiver with `Err`,
/// which is the only honest outcome: the answer is not coming.
fn fail_pending(pending: &Pending) {
    let mut pending = pending.lock().expect("pending request table");
    if pending.is_empty() {
        return;
    }
    log::warn!(
        "[octos] the kernel connection ended with {} request(s) in flight; \
         they will never be answered",
        pending.len()
    );
    pending.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A fake kernel: replies to every request with the method it was asked
    /// for, and pushes one notification. This is the contract the real
    /// `serve_io` implements, without needing a configured Octos profile.
    #[tokio::test]
    async fn correlates_replies_and_forwards_notifications() {
        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, mut incoming) = Connection::new(host);

        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(kernel);
            let mut lines = BufReader::new(reader).lines();
            writer
                .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"session/notify\",\"params\":{}}\n")
                .await
                .expect("write notification");
            while let Ok(Some(line)) = lines.next_line().await {
                let frame: Value = serde_json::from_str(&line).expect("request frame");
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": frame["id"],
                    "result": { "method": frame["method"] },
                });
                writer
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .expect("write reply");
            }
        });

        let reply = connection
            .call("session/open", json!({ "profile_id": "main" }))
            .await
            .expect("a reply");
        assert_eq!(reply["result"]["method"], json!("session/open"));

        let notification = tokio::time::timeout(Duration::from_secs(5), incoming.next())
            .await
            .expect("a notification")
            .expect("stream stays open");
        assert_eq!(notification["method"], json!("session/notify"));
    }

    /// A kernel that dies mid-call must not leave the caller waiting forever.
    ///
    /// The reader observes EOF and drops every pending sender, so the waiting
    /// `call` resolves `Err` and the table is empty — before this, a kernel
    /// crash (no profile, a panic, a closed pipe) left the future unresolved
    /// and the console stuck on "a turn is in flight".
    #[tokio::test]
    async fn eof_fails_every_pending_call() {
        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, _incoming) = Connection::new(host);

        let waiting = connection.call("session/open", json!({ "profile_id": "main" }));
        assert_eq!(
            connection.pending_count(),
            1,
            "the call must be recorded before the kernel dies"
        );

        // The kernel dies: dropping its end is what the reader sees as EOF.
        drop(kernel);

        let reply = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("EOF must settle the call rather than let it hang");
        assert!(reply.is_err(), "a dead kernel cannot answer");
        // The receiver resolving at all proves the sender was dropped; this
        // pins that the whole table was cleared, not just that one entry.
        assert_eq!(connection.pending_count(), 0, "the pending table is cleared");
    }

    /// The bounded call is the caller's own guard against a kernel that accepts
    /// a frame and never answers: it gives up with a readable reason, and a
    /// timeout is never reported as a reply.
    #[tokio::test]
    async fn call_with_timeout_gives_up_on_a_silent_kernel() {
        // The kernel stays alive (and silent): this is a hang, not a crash.
        let (host, _kernel) = tokio::io::duplex(64 * 1024);
        let (connection, _incoming) = Connection::new(host);

        let failure = connection
            .call_with_timeout("turn/start", json!({}), Duration::from_millis(50))
            .await
            .expect_err("a silent kernel cannot answer");
        match &failure {
            CallFailure::Timeout { method, .. } => assert_eq!(method, "turn/start"),
            other => panic!("a live-but-silent kernel is a timeout, not {other:?}"),
        }
        assert!(failure.to_string().contains("turn/start"), "{failure}");
    }

    /// The other half of the same guard: a reply that arrives inside the window
    /// is the reply, and a closed connection is reported as closed rather than
    /// as a timeout.
    #[tokio::test]
    async fn call_with_timeout_reports_the_reply_or_the_close() {
        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, _incoming) = Connection::new(host);

        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(kernel);
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let frame: Value = serde_json::from_str(&line).expect("request frame");
                let reply = json!({ "jsonrpc": "2.0", "id": frame["id"], "result": {} });
                writer
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .expect("write reply");
            }
        });

        let reply = connection
            .call_with_timeout("turn/interrupt", json!({}), Duration::from_secs(5))
            .await
            .expect("a reply inside the window");
        assert_eq!(reply["result"], json!({}));

        // Now drop the kernel and wait for the reader to settle the next call.
        let (host, kernel) = tokio::io::duplex(64 * 1024);
        let (connection, _incoming) = Connection::new(host);
        let waiting = connection.call("turn/interrupt", json!({}));
        drop(kernel);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), waiting)
                .await
                .expect("EOF must settle the wait")
                .is_err()
        );
        let failure = connection
            .call_with_timeout("turn/interrupt", json!({}), Duration::from_secs(5))
            .await
            .expect_err("the connection is gone");
        assert!(
            matches!(failure, CallFailure::Closed { .. }),
            "a closed connection is not a timeout: {failure:?}"
        );
    }
}
