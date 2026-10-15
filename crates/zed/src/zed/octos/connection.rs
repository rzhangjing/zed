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
use futures::StreamExt as _;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, ReadHalf, WriteHalf};

/// Where a reply to a host request is delivered once it arrives.
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

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

        tokio::spawn(write_frames(writer, outgoing_rx));
        tokio::spawn(read_frames(reader, pending.clone(), incoming));

        let connection = Self {
            outgoing,
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
        };
        (connection, incoming_rx)
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

async fn write_frames(mut writer: WriteHalf<DuplexStream>, mut frames: mpsc::UnboundedReceiver<Value>) {
    while let Some(frame) = frames.next().await {
        if let Err(error) = writer.write_all(format!("{frame}\n").as_bytes()).await {
            log::warn!("[octos] could not write to the kernel: {error}");
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
                            return;
                        }
                    }
                }
            }
            Ok(None) => return,
            Err(error) => {
                log::warn!("[octos] could not read from the kernel: {error}");
                return;
            }
        }
    }
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
}
