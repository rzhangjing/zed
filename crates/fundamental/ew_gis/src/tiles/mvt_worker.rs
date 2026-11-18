//! Background MVT fetch/decode workers.
//!
//! The main thread never performs MVT I/O or protobuf decoding. It queues tile
//! ids, drains decoded [`MvtTile`]s, and keeps the resident/visible set. The
//! worker mirrors the imagery [`crate::tiles::worker::TileWorker`] shape, with
//! two differences driven by the vector path:
//!
//! * a failure is delivered as `Err`, not swallowed, so the layer can count it
//!   and stop re-requesting that tile every frame;
//! * decoding runs inside `catch_unwind`: `geozero`'s command reader indexes the
//!   geometry stream, so a truncated tile from a broken server would otherwise
//!   kill the worker thread and leave its tile pending forever.

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::data::mvt::{MvtError, MvtOrigin, MvtSource, MvtTile, decode_mvt};
use crate::math::geo::TileId;

#[derive(Clone, Copy, Debug)]
pub struct MvtRequest {
    pub id: TileId,
}

/// One finished request: the decoded tile or the error that stopped it.
pub struct MvtReady {
    pub id: TileId,
    /// Where the bytes came from; `None` when the request failed before a
    /// source was resolved.
    pub origin: Option<MvtOrigin>,
    pub tile: Result<MvtTile, MvtError>,
}

pub struct MvtWorker {
    request_tx: Sender<MvtRequest>,
    ready_rx: Receiver<MvtReady>,
    threads: Vec<thread::JoinHandle<()>>,
    /// In-flight ids: the request de-duplication set.
    requested: HashSet<TileId>,
    pub completed: u64,
}

impl MvtWorker {
    pub fn spawn(source: Arc<MvtSource>, thread_count: usize) -> Self {
        let (request_tx, request_rx) = unbounded::<MvtRequest>();
        let (ready_tx, ready_rx) = unbounded::<MvtReady>();
        let thread_count = thread_count.clamp(1, 4);
        let mut threads = Vec::with_capacity(thread_count);
        for index in 0..thread_count {
            let request_rx = request_rx.clone();
            let ready_tx = ready_tx.clone();
            let source = Arc::clone(&source);
            threads.push(
                thread::Builder::new()
                    .name(format!("ew-gis-mvt-{index}"))
                    .spawn(move || {
                        while let Ok(request) = request_rx.recv() {
                            let id = request.id;
                            let decoded =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let (bytes, origin) = source.fetch(id)?;
                                    let tile = decode_mvt(id, &bytes)?;
                                    Ok::<_, MvtError>((tile, origin))
                                }));
                            let ready = match decoded {
                                Ok(Ok((tile, origin))) => MvtReady {
                                    id,
                                    origin: Some(origin),
                                    tile: Ok(tile),
                                },
                                Ok(Err(error)) => MvtReady {
                                    id,
                                    origin: None,
                                    tile: Err(error),
                                },
                                Err(_) => MvtReady {
                                    id,
                                    origin: None,
                                    tile: Err(MvtError::Decode {
                                        z: id.z,
                                        x: id.x,
                                        y: id.y,
                                        message: "MVT decoder panicked on a malformed tile"
                                            .to_string(),
                                    }),
                                },
                            };
                            if ready_tx.send(ready).is_err() {
                                // The layer was dropped: stop this worker.
                                break;
                            }
                        }
                    })
                    .expect("spawn MVT worker"),
            );
        }
        Self {
            request_tx,
            ready_rx,
            threads,
            requested: HashSet::new(),
            completed: 0,
        }
    }

    /// Queue a tile unless it is already in flight. Returns true when queued.
    pub fn request(&mut self, id: TileId) -> bool {
        if !self.requested.insert(id) {
            return false;
        }
        if self.request_tx.send(MvtRequest { id }).is_err() {
            self.requested.remove(&id);
            log::warn!("MVT worker channel closed while requesting {:?}", id);
            return false;
        }
        true
    }

    pub fn try_recv(&mut self) -> Option<MvtReady> {
        match self.ready_rx.try_recv() {
            Ok(ready) => {
                self.requested.remove(&ready.id);
                self.completed += 1;
                Some(ready)
            }
            Err(_) => None,
        }
    }

    pub fn is_in_flight(&self, id: TileId) -> bool {
        self.requested.contains(&id)
    }

    pub fn in_flight(&self) -> usize {
        self.requested.len()
    }
}

impl Drop for MvtWorker {
    fn drop(&mut self) {
        // Dropping the sender lets every worker exit its recv loop. The handles
        // are intentionally not joined: shutdown must not stall the event loop
        // on a slow HTTP timeout.
        let _ = &self.threads;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use crate::data::mvt::MvtOrigin;
    use geozero::mvt::tile;
    use geozero::mvt::{Message, Tile};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ew_gis_mvt_worker_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn tile_bytes() -> Vec<u8> {
        let mut layer = tile::Layer {
            version: 2,
            name: "points".to_string(),
            extent: Some(4096),
            ..Default::default()
        };
        layer.features.push(tile::Feature {
            r#type: Some(tile::GeomType::Point as i32),
            geometry: vec![9, 20, 20],
            ..Default::default()
        });
        Tile {
            layers: vec![layer],
        }
        .encode_to_vec()
    }

    fn wait(worker: &mut MvtWorker, timeout: Duration) -> Option<MvtReady> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(ready) = worker.try_recv() {
                return Some(ready);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn worker_decodes_a_local_tile_and_deduplicates_requests() {
        let dir = temp_dir("local");
        let id = TileId::new(2, 1, 1);
        let path = dir.join("2").join("1").join("1.mvt");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("tile dir");
        std::fs::write(&path, tile_bytes()).expect("write tile");

        let mut worker = MvtWorker::spawn(Arc::new(MvtSource::new("", &dir)), 2);
        assert!(worker.request(id), "first request is queued");
        assert!(!worker.request(id), "the same tile is not queued twice");
        assert_eq!(worker.in_flight(), 1);
        assert!(worker.is_in_flight(id));

        let ready = wait(&mut worker, Duration::from_secs(5)).expect("worker result");
        assert_eq!(ready.id, id);
        assert_eq!(ready.origin, Some(MvtOrigin::Local));
        assert_eq!(worker.in_flight(), 0);
        let tile = ready.tile.expect("decoded tile");
        assert_eq!(tile.shapes.points.len(), 1);
        // A finished request may be re-requested (the layer decides whether it
        // is still desired).
        assert!(worker.request(id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worker_reports_a_missing_tile_instead_of_hanging() {
        let dir = temp_dir("missing");
        let id = TileId::new(3, 2, 1);
        let mut worker = MvtWorker::spawn(Arc::new(MvtSource::new("", &dir)), 1);
        assert!(worker.request(id));
        let ready = wait(&mut worker, Duration::from_secs(5)).expect("worker result");
        assert_eq!(ready.origin, None);
        assert!(ready.tile.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
