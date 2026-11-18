//! Background tile fetch/decode workers.
//!
//! The main thread never touches the network or the image decoder. It only
//! queues `TileRequest`s, drains ready RGBA buffers, allocates atlas layers, and
//! uploads at most four layers per frame.

use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::math::geo::TileId;
use crate::tiles::provider::{Provider, TileOrigin};

#[derive(Clone, Copy, Debug)]
pub struct TileRequest {
    pub id: TileId,
}

pub struct TileReady {
    pub id: TileId,
    pub rgba: Vec<u8>,
    pub origin: TileOrigin,
}

pub struct TileWorker {
    request_tx: Sender<TileRequest>,
    ready_rx: Receiver<TileReady>,
    threads: Vec<thread::JoinHandle<()>>,
    pub completed: u64,
}

impl TileWorker {
    pub fn spawn(provider: Arc<Provider>, thread_count: usize) -> Self {
        let (request_tx, request_rx) = unbounded::<TileRequest>();
        let (ready_tx, ready_rx) = unbounded::<TileReady>();
        let thread_count = thread_count.clamp(1, 8);
        let mut threads = Vec::with_capacity(thread_count);
        for index in 0..thread_count {
            let request_rx = request_rx.clone();
            let ready_tx = ready_tx.clone();
            let provider = Arc::clone(&provider);
            threads.push(
                thread::Builder::new()
                    .name(format!("ew-gis-tile-{index}"))
                    .spawn(move || {
                        while let Ok(request) = request_rx.recv() {
                            let image = provider.load(request.id);
                            let _ = ready_tx.send(TileReady {
                                id: request.id,
                                rgba: image.rgba,
                                origin: image.origin,
                            });
                        }
                    })
                    .expect("spawn tile worker"),
            );
        }
        Self {
            request_tx,
            ready_rx,
            threads,
            completed: 0,
        }
    }

    pub fn request(&self, id: TileId) {
        if self.request_tx.send(TileRequest { id }).is_err() {
            log::warn!("tile worker channel closed while requesting {:?}", id);
        }
    }

    pub fn try_recv(&mut self) -> Option<TileReady> {
        match self.ready_rx.try_recv() {
            Ok(ready) => {
                self.completed += 1;
                Some(ready)
            }
            Err(_) => None,
        }
    }
}

impl Drop for TileWorker {
    fn drop(&mut self) {
        // Dropping the sender lets every worker exit its recv loop. The handles
        // are intentionally not joined here: shutdown must not stall the event
        // loop on a slow HTTP timeout.
        let _ = &self.threads;
    }
}
