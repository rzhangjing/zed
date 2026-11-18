//! MVT vector layer: visible-tile selection, fetch policy, and the merged
//! shapes handed to the plot document.
//!
//! The layer owns no GPU state. It follows the same request/eviction shape as
//! the imagery atlas ([`crate::tiles::TileSystem`]) but works on tile *ids*
//! supplied by the active scene:
//!
//! ```text
//! desired visible ids -> request missing -> drain decoded tiles
//!                     -> evict tiles that left the view (LRU, max_tiles cap)
//!                     -> merge the desired tiles into one DemoShapes
//! ```
//!
//! The merged [`DemoShapes`] is exactly what [`crate::scene::plot::PlotDocument`]
//! consumes, so the whole vector layer costs no extra draw call: it shares the
//! plot fill/line/point batches with the imported and demo shapes. The merge is
//! rebuilt only when a visible tile arrives or is evicted (a version counter lets
//! the host skip the document rebuild), and the combined shape count is capped
//! at [`MAX_MERGED_SHAPES`].
//!
//! ## Policy
//!
//! * `desired` is truncated to `max_tiles`; the rest is dropped, so a deep zoom
//!   can never queue an unbounded number of tiles.
//! * A failed tile is remembered: it is not re-requested every frame. The error
//!   count is exposed in [`MvtStats`] and logged once per tile.
//! * Tiles that leave the view stay resident while the layer is at or below the
//!   cap (a small cache for short pans) and are evicted least-recently-used
//!   first once the cap is exceeded.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::data::MAX_FEATURES;
use crate::data::mvt::{MvtOrigin, MvtSource, MvtTile};
use crate::math::geo::TileId;
use crate::scene::plot::DemoShapes;
use crate::tiles::mvt_worker::MvtWorker;

/// Default resident/visible tile cap (`--mvt-max-tiles`).
pub const DEFAULT_MAX_TILES: usize = 64;

/// Upper bound on shapes in the merged MVT overlay. Shares the importer cap so
/// the combined plot document stays at the 50k shapes the acceptance run pins.
pub const MAX_MERGED_SHAPES: usize = MAX_FEATURES;

/// Counters for the HUD and the exit summary.
#[derive(Clone, Copy, Debug, Default)]
pub struct MvtStats {
    /// Visible tiles the scene asked for (after the `max_tiles` truncation).
    pub desired_tiles: usize,
    /// Decoded tiles currently held (visible plus the short pan cache).
    pub resident_tiles: usize,
    /// Requests still in flight.
    pub pending_tiles: usize,
    /// Tiles whose fetch/decode failed (not retried for the session).
    pub failed_tiles: usize,
    /// Features and layers in the visible resident tiles.
    pub features: usize,
    pub layers: usize,
    /// Features skipped by the decoder, and holes dropped by it.
    pub skipped: usize,
    pub holes: usize,
    /// Shapes in the merged overlay.
    pub merged_shapes: usize,
    /// Tiles decoded from the local directory / over HTTP.
    pub local_tiles: usize,
    pub online_tiles: usize,
    pub requests: u64,
    pub errors: u64,
    pub evictions: u64,
    /// Number of merged-shape rebuilds.
    pub rebuilds: u64,
    /// CPU time of the most recent merge.
    pub merge_ms: f64,
    /// CPU time of the most recent [`MvtLayer::update`] call.
    pub last_update_ms: f64,
}

pub struct MvtLayer {
    /// Decoded tiles by id. A [`MvtTile`] carries its shapes plus the counters.
    tiles: HashMap<TileId, MvtTile>,
    source: Arc<MvtSource>,
    worker: MvtWorker,
    max_tiles: usize,
    /// Visible tile ids of the last update, in scene order (merge order).
    desired: Vec<TileId>,
    /// Failed tile ids and their error text.
    failed: HashMap<TileId, String>,
    /// Last update tick per resident tile, for the LRU eviction order.
    last_used: HashMap<TileId, u64>,
    tick: u64,
    merged: DemoShapes,
    merged_dirty: bool,
    version: u64,
    requests_total: u64,
    errors_total: u64,
    evictions_total: u64,
    rebuilds_total: u64,
    local_tiles: usize,
    online_tiles: usize,
    merge_ms: f64,
    pub stats: MvtStats,
}

impl MvtLayer {
    pub fn new(source: MvtSource, max_tiles: usize) -> Self {
        let source = Arc::new(source);
        let thread_count = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(2)
            .clamp(1, 4);
        let worker = MvtWorker::spawn(Arc::clone(&source), thread_count);
        Self {
            tiles: HashMap::with_capacity(max_tiles.clamp(1, 256)),
            source,
            worker,
            max_tiles: max_tiles.max(1),
            desired: Vec::with_capacity(max_tiles.clamp(1, 256)),
            failed: HashMap::new(),
            last_used: HashMap::new(),
            tick: 0,
            merged: DemoShapes::empty(),
            merged_dirty: false,
            version: 0,
            requests_total: 0,
            errors_total: 0,
            evictions_total: 0,
            rebuilds_total: 0,
            local_tiles: 0,
            online_tiles: 0,
            merge_ms: 0.0,
            stats: MvtStats::default(),
        }
    }

    pub fn max_tiles(&self) -> usize {
        self.max_tiles
    }

    pub fn source_label(&self) -> String {
        self.source.label()
    }

    /// The merged shapes of the visible tiles, ready for `PlotDocument::new`.
    pub fn merged(&self) -> &DemoShapes {
        &self.merged
    }

    /// Increments whenever the merged shapes change.
    pub fn version(&self) -> u64 {
        self.version
    }

    #[cfg(test)]
    pub fn desired(&self) -> &[TileId] {
        &self.desired
    }

    /// True when every visible tile is either resident or known to have failed.
    /// The screenshot gate waits on this so a capture is not taken mid-load.
    pub fn settled(&self) -> bool {
        self.worker.in_flight() == 0
            && self
                .desired
                .iter()
                .all(|id| self.tiles.contains_key(id) || self.failed.contains_key(id))
    }

    /// Insert an already decoded tile without going through the worker. Test
    /// helper: it is how a seeded cache / a future disk cache would prime the
    /// layer, and it marks the next update for a merge rebuild.
    #[cfg(test)]
    pub fn insert_tile(&mut self, tile: MvtTile) {
        self.last_used.insert(tile.id, self.tick);
        self.tiles.insert(tile.id, tile);
        self.merged_dirty = true;
    }

    /// Update the visible set, queue the missing tiles, drain finished ones,
    /// evict what left the view, and rebuild the merged shapes when anything
    /// visible changed. Returns true when the merged shapes changed.
    pub fn update(&mut self, desired: &[TileId]) -> bool {
        let started = Instant::now();
        self.tick += 1;
        let limit = self.max_tiles;

        self.desired.clear();
        self.desired.extend(desired.iter().copied().take(limit));

        for id in &self.desired {
            let id = *id;
            if self.tiles.contains_key(&id) || self.failed.contains_key(&id) {
                continue;
            }
            if self.worker.is_in_flight(id) {
                continue;
            }
            if self.worker.request(id) {
                self.requests_total += 1;
                if self.requests_total == 1 {
                    log::info!(
                        "MVT layer: requesting {}/{}/{} (source: {})",
                        id.z,
                        id.x,
                        id.y,
                        self.source.label()
                    );
                }
            }
        }

        while let Some(ready) = self.worker.try_recv() {
            match ready.tile {
                Ok(tile) => {
                    let origin = ready
                        .origin
                        .map(|origin| {
                            match origin {
                                MvtOrigin::Local => self.local_tiles += 1,
                                MvtOrigin::Online => self.online_tiles += 1,
                            }
                            origin.label()
                        })
                        .unwrap_or("unknown");
                    log::debug!(
                        "MVT tile {}/{}/{} decoded from {origin}: {} layers, {} features, {} shapes, {} skipped, {} holes",
                        tile.id.z,
                        tile.id.x,
                        tile.id.y,
                        tile.layers,
                        tile.features,
                        tile.shape_count(),
                        tile.skipped,
                        tile.holes
                    );
                    if self.desired.contains(&tile.id) {
                        self.merged_dirty = true;
                    }
                    self.last_used.insert(tile.id, self.tick);
                    self.tiles.insert(tile.id, tile);
                }
                Err(error) => {
                    self.errors_total += 1;
                    log::warn!(
                        "MVT tile {}/{}/{} failed: {error}",
                        ready.id.z,
                        ready.id.x,
                        ready.id.y
                    );
                    self.failed.insert(ready.id, error.to_string());
                }
            }
        }

        self.evict(limit);
        let changed = self.merged_dirty;
        if changed {
            self.rebuild_merged();
        }
        self.refresh_stats(started);
        changed
    }

    /// Drop the least recently used tiles outside the visible set until the
    /// resident count fits the cap; drop visible tiles only if the view alone
    /// exceeds it (the requested set was already truncated, so this is rare).
    fn evict(&mut self, limit: usize) {
        if self.tiles.len() <= limit {
            return;
        }
        let mut candidates: Vec<(u64, TileId)> = self
            .tiles
            .keys()
            .filter(|id| !self.desired.contains(id))
            .map(|id| (self.last_used.get(id).copied().unwrap_or(0), *id))
            .collect();
        candidates.sort_unstable();
        for (_, id) in candidates {
            if self.tiles.len() <= limit {
                break;
            }
            self.remove_tile(id);
        }
        while self.tiles.len() > limit {
            let oldest = self
                .tiles
                .keys()
                .copied()
                .min_by_key(|id| self.last_used.get(id).copied().unwrap_or(0));
            let Some(id) = oldest else { break };
            self.remove_tile(id);
        }
    }

    fn remove_tile(&mut self, id: TileId) {
        if self.tiles.remove(&id).is_none() {
            return;
        }
        self.last_used.remove(&id);
        self.evictions_total += 1;
        if self.desired.contains(&id) {
            self.merged_dirty = true;
        }
        log::debug!("MVT tile {}/{}/{} evicted", id.z, id.x, id.y);
    }

    fn rebuild_merged(&mut self) {
        let started = Instant::now();
        let mut merged = DemoShapes::empty();
        for id in &self.desired {
            if let Some(tile) = self.tiles.get(id) {
                append_shapes(&mut merged, &tile.shapes, MAX_MERGED_SHAPES);
            }
        }
        self.merged = merged;
        self.merged_dirty = false;
        self.version += 1;
        self.rebuilds_total += 1;
        self.merge_ms = started.elapsed().as_secs_f64() * 1000.0;
        let counts = self.merged.counts();
        log::debug!(
            "MVT merged overlay rebuilt: {} shapes ({} points / {} lines / {} polygons) from {} tiles in {:.3} ms",
            counts.iter().sum::<usize>(),
            counts[0],
            counts[1],
            counts[2],
            self.desired
                .iter()
                .filter(|id| self.tiles.contains_key(id))
                .count(),
            self.merge_ms
        );
    }

    fn refresh_stats(&mut self, started: Instant) {
        let mut stats = MvtStats {
            desired_tiles: self.desired.len(),
            resident_tiles: self.tiles.len(),
            pending_tiles: self.worker.in_flight(),
            failed_tiles: self.failed.len(),
            merged_shapes: self.merged.counts().iter().sum(),
            local_tiles: self.local_tiles,
            online_tiles: self.online_tiles,
            requests: self.requests_total,
            errors: self.errors_total,
            evictions: self.evictions_total,
            rebuilds: self.rebuilds_total,
            merge_ms: self.merge_ms,
            ..Default::default()
        };
        for id in &self.desired {
            if let Some(tile) = self.tiles.get(id) {
                stats.features += tile.features;
                stats.layers += tile.layers;
                stats.skipped += tile.skipped;
                stats.holes += tile.holes;
            }
        }
        stats.last_update_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.stats = stats;
    }
}

/// Total shapes of a `DemoShapes` (all kinds).
pub fn shape_count(shapes: &DemoShapes) -> usize {
    shapes.counts().iter().sum()
}

/// Append `source` shapes to `destination`, keeping the combined count at or
/// below `cap` (points first, then lines, then polygons). Returns the number of
/// shapes that did not fit.
pub fn append_shapes(destination: &mut DemoShapes, source: &DemoShapes, cap: usize) -> usize {
    let mut room = cap.saturating_sub(shape_count(destination));
    let mut dropped = 0usize;

    let take = source.points.len().min(room);
    destination.points.extend_from_slice(&source.points[..take]);
    dropped += source.points.len() - take;
    room -= take;

    let take = source.polylines.len().min(room);
    destination
        .polylines
        .extend_from_slice(&source.polylines[..take]);
    dropped += source.polylines.len() - take;
    room -= take;

    let take = source.polygons.len().min(room);
    destination
        .polygons
        .extend_from_slice(&source.polygons[..take]);
    dropped += source.polygons.len() - take;
    room -= take;

    let take = source.sectors.len().min(room);
    destination
        .sectors
        .extend_from_slice(&source.sectors[..take]);
    dropped += source.sectors.len() - take;
    dropped
}

/// Merge the static document shapes with the MVT overlay under one cap. The
/// static shapes are kept whole: if they already fill the cap, no overlay shape
/// is taken and the dropped count is reported to the caller.
pub fn merge_shapes(base: &DemoShapes, overlay: &DemoShapes, cap: usize) -> (DemoShapes, usize) {
    let mut merged = base.clone();
    let dropped = append_shapes(&mut merged, overlay, cap);
    (merged, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::plot::GeoPoint;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ew_gis_mvt_layer_{}_{}_{}",
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

    fn point(lon: f64, lat: f64) -> GeoPoint {
        GeoPoint {
            lon,
            lat,
            size_px: 4.0,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }

    fn tile_with_points(id: TileId, count: usize) -> MvtTile {
        let mut tile = MvtTile::new(id);
        tile.layers = 1;
        tile.features = count;
        for index in 0..count {
            tile.shapes
                .points
                .push(point(id.x as f64 + index as f64 * 0.1, id.y as f64));
        }
        tile
    }

    #[test]
    fn merge_keeps_the_static_shapes_and_caps_the_overlay() {
        let mut base = DemoShapes::empty();
        base.points.push(point(1.0, 2.0));
        base.points.push(point(3.0, 4.0));

        let mut overlay = DemoShapes::empty();
        for index in 0..5 {
            overlay.points.push(point(5.0 + index as f64, 6.0));
        }

        let (merged, dropped) = merge_shapes(&base, &overlay, 4);
        assert_eq!(shape_count(&merged), 4);
        assert_eq!(merged.points.len(), 4);
        assert_eq!(dropped, 3);
        // The static shapes survive the cap.
        assert_eq!(merged.points[0].lon, 1.0);
        assert_eq!(merged.points[1].lon, 3.0);

        // A cap already consumed by the static document drops the whole overlay.
        let (all_static, dropped) = merge_shapes(&base, &overlay, 2);
        assert_eq!(all_static.points.len(), 2);
        assert_eq!(dropped, 5);
    }

    #[test]
    fn append_shapes_fills_points_then_lines_then_polygons() {
        let mut destination = DemoShapes::empty();
        destination.points.push(point(0.0, 0.0));
        let mut source = DemoShapes::empty();
        source.points.push(point(1.0, 1.0));
        source.polylines.push(crate::scene::plot::GeoPolyline {
            points: vec![[2.0, 2.0], [3.0, 3.0]],
            width_px: 1.5,
            color: [1.0, 0.0, 0.0, 1.0],
        });
        let dropped = append_shapes(&mut destination, &source, 2);
        assert_eq!(destination.points.len(), 2);
        assert_eq!(destination.polylines.len(), 0);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn layer_merges_only_the_desired_tiles() {
        let dir = temp_dir("merge");
        let mut layer = MvtLayer::new(MvtSource::new("", &dir), DEFAULT_MAX_TILES);
        let visible = TileId::new(4, 3, 2);
        let cached = TileId::new(4, 4, 2);
        layer.insert_tile(tile_with_points(visible, 2));
        layer.insert_tile(tile_with_points(cached, 3));
        assert_eq!(layer.version(), 0, "inserting alone does not merge");

        let changed = layer.update(&[visible]);
        assert!(changed, "the merged overlay was rebuilt");
        assert_eq!(layer.version(), 1);
        assert_eq!(shape_count(layer.merged()), 2);
        assert_eq!(layer.stats.merged_shapes, 2);
        assert_eq!(layer.stats.features, 2);
        assert_eq!(layer.stats.layers, 1);
        assert_eq!(layer.stats.resident_tiles, 2, "the cache tile stays");
        assert_eq!(layer.stats.desired_tiles, 1);
        assert_eq!(layer.stats.rebuilds, 1);

        // Nothing changed on the next frame: no rebuild.
        assert!(!layer.update(&[visible]));
        assert_eq!(layer.version(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn eviction_drops_tiles_that_left_the_view_over_the_cap() {
        let dir = temp_dir("evict");
        let mut layer = MvtLayer::new(MvtSource::new("", &dir), 1);
        let keep = TileId::new(2, 1, 1);
        let drop = TileId::new(2, 2, 1);
        layer.insert_tile(tile_with_points(keep, 1));
        layer.insert_tile(tile_with_points(drop, 1));

        layer.update(&[keep]);
        assert_eq!(layer.stats.resident_tiles, 1);
        assert_eq!(layer.stats.evictions, 1);
        assert_eq!(shape_count(layer.merged()), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn desired_is_truncated_to_the_cap() {
        let dir = temp_dir("truncate");
        let mut layer = MvtLayer::new(MvtSource::new("", &dir), 2);
        let ids = [
            TileId::new(3, 0, 0),
            TileId::new(3, 1, 0),
            TileId::new(3, 2, 0),
            TileId::new(3, 3, 0),
        ];
        layer.update(&ids);
        assert_eq!(layer.desired().len(), 2);
        assert_eq!(layer.stats.desired_tiles, 2);
        assert_eq!(
            layer.stats.failed_tiles, 0,
            "failures arrive asynchronously"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
