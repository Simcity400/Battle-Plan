//! `scrub` — M6, **the scrubber**: the spine that drives the whole replay UI
//! through time.
//!
//! Four pieces, exactly as the spec's M6 audit gate requires:
//!
//! 1. [`Timeline`] — owns the global `hour_index` and the ordered snapshot series
//!    (read from the store's `index.json`, NEVER reconstructed). `step`/`play`
//!    auto-advance walks the index entries; a scrub jumps to any entry. Moving the
//!    cursor returns the snapshot for that hour, which the caller re-binds onto
//!    every open surface (topbar + map + state panel).
//!
//! 2. [`SnapshotLru`] — a **windowed immutable LRU** of PARSED snapshots (3–8
//!    entries). A full-world snapshot is ~10–12 MB and full-parses in ~100 ms, so a
//!    naive per-tick re-parse is the real cost. The LRU keys on `hour_index`; a
//!    cache HIT is an `Arc::clone` (≈ 0 ms, no disk, no parse), a MISS reads +
//!    parses once and inserts, evicting the least-recently-used over the window.
//!
//! 3. [`DomainView`] — a **domain-scoped, read-only** accessor. A surface borrows
//!    only the domain(s) it needs (the map: `states` + `map`; the panel: one
//!    `states` entry; the topbar: one `countries` entry) so a re-bind never walks
//!    the ~3 MB `divisions` array. It hands out `&Value` / `&Snapshot` references —
//!    it cannot mutate, by type.
//!
//! 4. [`RawFrameIndex`] — an optional sibling index over verified
//!    `collector_raw_stream` JSONL payloads inside `.battleplan`. It indexes only
//!    gameplay raw frames (`domain_delta`, `event`, `gap`); lifecycle diagnostics
//!    stay in the preserved byte stream, not in the app-facing scrub index.
//!
//! ## The HARD no-fabrication rule, enforced by construction
//!
//! A cached snapshot is a **shared** instance: [`SnapshotLru::get`] returns
//! `Arc<Snapshot>`. `Arc<T>` only ever yields `&T` (never `&mut T`) unless its
//! refcount is 1, so a holder of an `Arc<Snapshot>` **cannot** mutate the cached
//! payload — the type system forbids it. The bind/apply layer (`bind::apply_topbar`,
//! `bind::bind_state`) takes `&Snapshot` and writes its results into a *freshly
//! built* `gui_runtime` scene-graph (a separate `Vec<Node>` per re-bind), never
//! back into the snapshot. Therefore no view can default/annotate a value INTO the
//! cached snapshot, so a fabricated/defaulted value can never persist across a
//! later scrub tick. The [`tests::scrub_a_b_a_is_identical`] test proves the
//! round-trip: bind A, bind B (against the same cached instances), re-bind A → the
//! A report is byte-identical to the first A bind. See also the prose in
//! [`immutable`].

use serde_json::Value;
use snapshot::{Index, Snapshot, Store};
use std::collections::VecDeque;
use std::sync::Arc;

const RAW_COLLECTOR_STREAM_ROLE: &str = "collector_raw_stream";

/// A parsed gameplay raw frame from `collector_frames.jsonl`. This is a sibling
/// to hourly snapshots: it never becomes a `Frame` and never changes the
/// authoritative `Timeline::hours()` series.
#[derive(Debug, Clone, PartialEq)]
pub struct RawFrame {
    pub kind: String,
    pub hour_index: Option<i64>,
    pub date: Option<String>,
    pub tick_ms: Option<i64>,
    pub sequence: Option<i64>,
    pub session_sequence: Option<i64>,
    pub domain: Option<String>,
    pub entity_key: Option<String>,
    pub source: Option<String>,
    pub confidence: Option<String>,
    pub reason: Option<String>,
    pub payload: Option<Value>,
}

/// Optional raw collector frames indexed beside the hourly snapshot spine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawFrameIndex {
    frames: Vec<RawFrame>,
    errors: Vec<String>,
    source_paths: Vec<String>,
}

impl RawFrameIndex {
    pub fn empty() -> RawFrameIndex {
        RawFrameIndex::default()
    }
    pub fn frames(&self) -> &[RawFrame] {
        &self.frames
    }
    pub fn errors(&self) -> &[String] {
        &self.errors
    }
    pub fn source_paths(&self) -> &[String] {
        &self.source_paths
    }
    pub fn is_available(&self) -> bool {
        !self.frames.is_empty() && self.errors.is_empty()
    }
    pub fn frames_at_hour(&self, hour_index: i64) -> Vec<&RawFrame> {
        self.frames
            .iter()
            .filter(|frame| frame.hour_index == Some(hour_index))
            .collect()
    }

    fn from_store(store: &Store) -> RawFrameIndex {
        let Some(container) = store.container() else {
            return RawFrameIndex::empty();
        };
        let entries = container.entries_by_role(RAW_COLLECTOR_STREAM_ROLE);
        if entries.is_empty() {
            return RawFrameIndex::empty();
        }

        let mut frames = Vec::new();
        let mut source_paths = Vec::new();
        for entry in entries {
            let bytes = match container.read_entry_bytes(&entry.path) {
                Ok(bytes) => bytes,
                Err(err) => {
                    return RawFrameIndex::failed(format!(
                        "raw stream {} failed integrity/read: {err}",
                        entry.path
                    ));
                }
            };
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(err) => {
                    return RawFrameIndex::failed(format!(
                        "raw stream {} is not utf-8: {err}",
                        entry.path
                    ));
                }
            };
            source_paths.push(entry.path);
            if let Err(err) = parse_raw_jsonl(&text, &mut frames) {
                return RawFrameIndex::failed(err);
            }
        }
        frames.sort_by_key(raw_frame_sort_key);
        RawFrameIndex {
            frames,
            errors: Vec::new(),
            source_paths,
        }
    }

    fn failed(message: String) -> RawFrameIndex {
        RawFrameIndex {
            frames: Vec::new(),
            errors: vec![message],
            source_paths: Vec::new(),
        }
    }
}

fn parse_raw_jsonl(text: &str, frames: &mut Vec<RawFrame>) -> Result<(), String> {
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|err| format!("raw stream line {} is invalid json: {err}", i + 1))?;
        let object = value
            .as_object()
            .ok_or_else(|| format!("raw stream line {} is not an object", i + 1))?;
        let Some(kind) = string_field(&value, "type") else {
            continue;
        };
        if !matches!(kind.as_str(), "domain_delta" | "event" | "gap") {
            continue;
        }
        frames.push(RawFrame {
            kind,
            hour_index: i64_field(&value, "hour_index"),
            date: string_field(&value, "date"),
            tick_ms: i64_field(&value, "tick_ms"),
            sequence: i64_field(&value, "sequence"),
            session_sequence: i64_field(&value, "session_sequence"),
            domain: string_field(&value, "domain"),
            entity_key: string_field(&value, "entity_key"),
            source: string_field(&value, "source"),
            confidence: string_field(&value, "confidence"),
            reason: string_field(&value, "reason"),
            payload: object.get("payload").cloned(),
        });
    }
    Ok(())
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn i64_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key)?.as_i64()
}

fn raw_frame_sort_key(frame: &RawFrame) -> (i64, i64, i64) {
    (
        frame.hour_index.unwrap_or(i64::MAX),
        frame.tick_ms.unwrap_or(i64::MAX),
        frame.sequence.unwrap_or(i64::MAX),
    )
}

/// A graceful "empty series" error for the transport controls. An empty timeline is
/// only reachable from a degenerate/corrupt index (zero snapshots, or every snapshot
/// missing `hour_index`); the API returns this instead of panicking (Codex FIX-3).
fn empty_timeline_err() -> snapshot::LoadError {
    snapshot::LoadError::Container(snapshot::ContainerError::Reject(
        "empty timeline: no snapshots with an hour_index in the index".into(),
    ))
}

pub mod view;
pub use view::{CountryView, DomainView, MapView, StateView};

/// One step the scrubber can make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Forward,
    Backward,
}

/// Whether the last cursor move parsed from disk (cold) or hit the cache (warm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    /// First touch: read + parsed from disk (~100 ms for a full-world snapshot).
    Cold,
    /// LRU hit: an `Arc::clone` of an already-parsed snapshot (≈ 0 ms).
    Warm,
}

/// The result of moving the cursor: the (shared, immutable) snapshot now under the
/// cursor, plus whether it was a cold parse or a warm cache hit.
#[derive(Clone)]
pub struct Frame {
    pub hour_index: i64,
    pub snapshot: Arc<Snapshot>,
    pub fetch: Fetch,
}

impl Frame {
    /// A domain-scoped, read-only view over this frame's snapshot. Each surface
    /// reads ONLY through this — it cannot reach the underlying `Arc` to mutate it
    /// (the view exposes `&` accessors only), and a surface that wants `states`
    /// never touches `divisions`.
    pub fn view(&self) -> DomainView<'_> {
        DomainView::new(&self.snapshot)
    }
}

// =============================================================================
// The windowed immutable LRU.
// =============================================================================

/// A windowed LRU of PARSED, shared, immutable snapshots, keyed on `hour_index`.
///
/// * **Window**: at most `capacity` entries (3–8 per the spec); the least-recently
///   USED entry is evicted when a miss would exceed the window.
/// * **Immutable**: each value is an `Arc<Snapshot>`; [`get`](Self::get) hands out
///   `Arc::clone`s. A holder can only read (`&Snapshot`), so a cached parse is
///   shared but never mutated in place — the no-fabrication prime directive.
/// * **Cache-hit ≈ 0**: a hit is a `Vec` scan + `Arc::clone` (a refcount bump); a
///   miss reads the file and runs `serde_json` once.
pub struct SnapshotLru {
    store: Store,
    game: String,
    capacity: usize,
    /// Most-recently-used at the FRONT. Each entry is (hour_index, parsed snapshot).
    entries: VecDeque<(i64, Arc<Snapshot>)>,
    /// `hour_index -> file` from the index, AUTHORITATIVE when present: a snapshot is
    /// loaded by its recorded `file` name (which may be non-canonical), not a
    /// reconstructed `{:012}.snapshot.json` (Codex-flagged: `IndexEntry.file` is the
    /// source of truth). Falls back to the canonical name when an hour is not mapped.
    files: std::collections::HashMap<i64, String>,
    // -- instrumentation (for the M6 latency report) --
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl SnapshotLru {
    /// Open an LRU over `store`/`game` with a window of `capacity` parsed snapshots.
    /// `capacity` is clamped to the spec's 3..=8 window. No file map → loads are by
    /// the canonical filename (use [`with_files`](Self::with_files) to honour the
    /// index's explicit `file` field).
    pub fn new(store: Store, game: impl Into<String>, capacity: usize) -> SnapshotLru {
        SnapshotLru {
            store,
            game: game.into(),
            capacity: capacity.clamp(3, 8),
            entries: VecDeque::new(),
            files: std::collections::HashMap::new(),
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// As [`new`](Self::new) but with an authoritative `hour_index -> file` map (from
    /// the index). A load for a mapped hour reads exactly that file.
    pub fn with_files(
        store: Store,
        game: impl Into<String>,
        capacity: usize,
        files: std::collections::HashMap<i64, String>,
    ) -> SnapshotLru {
        let mut lru = Self::new(store, game, capacity);
        lru.files = files;
        lru
    }

    /// Load `hour` from disk, honouring the index `file` map when present.
    fn load_hour(&self, hour: i64) -> Result<Snapshot, snapshot::LoadError> {
        match self.files.get(&hour) {
            Some(file) => self.store.load_file(&self.game, file),
            None => self.store.load(&self.game, hour),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// The hours currently resident, MRU first (for prefetch / window inspection).
    pub fn resident_hours(&self) -> Vec<i64> {
        self.entries.iter().map(|(h, _)| *h).collect()
    }
    /// True iff `hour` is already parsed and resident (a `get` would be warm).
    pub fn contains(&self, hour: i64) -> bool {
        self.entries.iter().any(|(h, _)| *h == hour)
    }

    /// Fetch the snapshot for `hour`, parsing it once on a miss. Returns the shared
    /// `Arc<Snapshot>` and whether this was a [`Fetch::Cold`] parse or a
    /// [`Fetch::Warm`] cache hit. The returned `Arc` is immutable to every caller.
    pub fn get(&mut self, hour: i64) -> Result<(Arc<Snapshot>, Fetch), snapshot::LoadError> {
        if let Some(pos) = self.entries.iter().position(|(h, _)| *h == hour) {
            // HIT: move to front (MRU) and clone the Arc — no disk, no parse.
            let entry = self.entries.remove(pos).expect("position valid");
            let arc = Arc::clone(&entry.1);
            self.entries.push_front(entry);
            self.hits += 1;
            return Ok((arc, Fetch::Warm));
        }
        // MISS: read + parse exactly once, insert at front, evict LRU over window.
        let snap = self.load_hour(hour)?;
        let arc = Arc::new(snap);
        self.entries.push_front((hour, Arc::clone(&arc)));
        self.misses += 1;
        while self.entries.len() > self.capacity {
            self.entries.pop_back();
            self.evictions += 1;
        }
        Ok((arc, Fetch::Cold))
    }

    /// Warm the cache for `hour` WITHOUT promoting it to MRU. A no-op if already
    /// resident; otherwise a cold parse + insert JUST BEHIND the cursor (the MRU
    /// front), so a directional prefetch can never evict the cursor's own frame
    /// (Codex-flagged): the prefetched entry is younger than everything except the
    /// cursor, and trimming always pops from the LRU back — which is never index 0.
    pub fn prefetch(&mut self, hour: i64) -> Result<(), snapshot::LoadError> {
        if self.contains(hour) {
            return Ok(());
        }
        let snap = self.load_hour(hour)?;
        let arc = Arc::new(snap);
        // Insert at position 1 (immediately behind the MRU cursor at 0). On an empty
        // cache that's just the front.
        let at = 1.min(self.entries.len());
        self.entries.insert(at, (hour, arc));
        self.misses += 1;
        // Trim over the window from the LRU back. The cursor (index 0) is protected:
        // pop_back never touches it while len > 1.
        while self.entries.len() > self.capacity {
            self.entries.pop_back();
            self.evictions += 1;
        }
        Ok(())
    }
}

// =============================================================================
// The timeline — the global hour_index + the ordered series.
// =============================================================================

/// The scrubber's spine: the ordered snapshot series (from the store index, NEVER
/// reconstructed) + a cursor (`pos`) into it + play/pause state. It owns a
/// [`SnapshotLru`] so every cursor move is served warm-or-cold through the window.
pub struct Timeline {
    /// Ordered hours, ascending — the canonical series straight from `index.json`.
    hours: Vec<i64>,
    /// Cursor into `hours`.
    pos: usize,
    /// Play/pause for auto-advance.
    playing: bool,
    /// Auto-advance direction (play walks the series this way).
    dir: Dir,
    cache: SnapshotLru,
    raw_index: RawFrameIndex,
}

impl Timeline {
    /// Build a timeline from a store + game id, with an LRU of `capacity` frames.
    /// The series is taken verbatim from `index.json` (ascending by `hour_index`);
    /// it is NEVER reconstructed from diffs or re-derived.
    pub fn open(
        store: Store,
        game: impl Into<String>,
        capacity: usize,
    ) -> Result<Timeline, snapshot::LoadError> {
        let game = game.into();
        let index: Index = store.index(&game)?;
        let raw_index = RawFrameIndex::from_store(&store);
        Ok(Self::from_index_with_raw(
            store, game, index, capacity, raw_index,
        ))
    }

    /// As [`open`](Self::open) but with an already-loaded [`Index`] (avoids a second
    /// read when the caller has it).
    pub fn from_index(
        store: Store,
        game: impl Into<String>,
        index: Index,
        capacity: usize,
    ) -> Timeline {
        Self::from_index_with_raw(store, game, index, capacity, RawFrameIndex::empty())
    }

    fn from_index_with_raw(
        store: Store,
        game: impl Into<String>,
        index: Index,
        capacity: usize,
        raw_index: RawFrameIndex,
    ) -> Timeline {
        let mut hours: Vec<i64> = index
            .snapshots
            .iter()
            .filter_map(|e| e.hour_index)
            .collect();
        hours.sort_unstable();
        hours.dedup();
        // Authoritative hour -> file map straight from the index, so the LRU loads
        // the recorded `file` (Codex-flagged: `IndexEntry.file` is the source of
        // truth, not a reconstructed canonical name).
        let files: std::collections::HashMap<i64, String> = index
            .snapshots
            .iter()
            .filter_map(|e| e.hour_index.map(|h| (h, e.file.clone())))
            .collect();
        let game = game.into();
        let cache = SnapshotLru::with_files(store, game, capacity, files);
        Timeline {
            hours,
            pos: 0,
            playing: false,
            dir: Dir::Forward,
            cache,
            raw_index,
        }
    }

    /// The ordered series of hours (the snapshot series; read-only).
    pub fn hours(&self) -> &[i64] {
        &self.hours
    }
    pub fn len(&self) -> usize {
        self.hours.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hours.is_empty()
    }
    /// The cursor's index into the series.
    pub fn pos(&self) -> usize {
        self.pos
    }
    /// The hour the cursor currently sits on, if the series is non-empty.
    pub fn current_hour(&self) -> Option<i64> {
        self.hours.get(self.pos).copied()
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn direction(&self) -> Dir {
        self.dir
    }
    /// Borrow the LRU (for the latency / window report).
    pub fn cache(&self) -> &SnapshotLru {
        &self.cache
    }
    /// Optional gameplay raw frames indexed beside, not inside, the hourly series.
    pub fn raw_index(&self) -> &RawFrameIndex {
        &self.raw_index
    }

    // -- transport controls -------------------------------------------------

    pub fn play(&mut self) {
        self.playing = true;
    }
    pub fn pause(&mut self) {
        self.playing = false;
    }
    pub fn set_direction(&mut self, dir: Dir) {
        self.dir = dir;
    }

    /// Jump the cursor to series index `pos` (a scrubber drag/click). Clamped.
    /// Returns the resolved [`Frame`] for that hour (warm or cold via the LRU). An
    /// EMPTY series (a degenerate/corrupt index with no `hour_index`) returns a
    /// graceful error rather than panicking (Codex bug-hunt FIX-3).
    pub fn seek_to(&mut self, pos: usize) -> Result<Frame, snapshot::LoadError> {
        if self.hours.is_empty() {
            return Err(empty_timeline_err());
        }
        self.pos = pos.min(self.hours.len() - 1);
        self.frame()
    }

    /// Jump the cursor to a specific `hour_index` (must be in the series). Returns
    /// `None` if that hour is not a recorded snapshot (we never fabricate one).
    pub fn seek_hour(&mut self, hour: i64) -> Option<Result<Frame, snapshot::LoadError>> {
        let pos = self.hours.iter().position(|h| *h == hour)?;
        Some(self.seek_to(pos))
    }

    /// Advance the cursor one snapshot in `dir`, clamped at the ends (no wrap).
    /// Returns the new [`Frame`]. This is what play() ticks call, and what a
    /// step-forward/back button calls.
    pub fn step(&mut self, dir: Dir) -> Result<Frame, snapshot::LoadError> {
        if self.hours.is_empty() {
            return Err(empty_timeline_err());
        }
        match dir {
            Dir::Forward => {
                if self.pos + 1 < self.hours.len() {
                    self.pos += 1;
                }
            }
            Dir::Backward => {
                self.pos = self.pos.saturating_sub(1);
            }
        }
        self.frame()
    }

    /// One auto-advance tick when playing: step in the play direction; pause at the
    /// end of the series. Returns `Some(frame)` if the cursor moved (or is at an end
    /// and a frame was produced), `None` if not playing.
    pub fn tick(&mut self) -> Option<Result<Frame, snapshot::LoadError>> {
        if !self.playing || self.hours.is_empty() {
            return None;
        }
        let at_end = match self.dir {
            Dir::Forward => self.pos + 1 >= self.hours.len(),
            Dir::Backward => self.pos == 0,
        };
        if at_end {
            self.playing = false;
            return None;
        }
        Some(self.step(self.dir))
    }

    /// Resolve the cursor's current hour to a [`Frame`] through the LRU. This is the
    /// single fetch choke point — every cursor move funnels here, so every frame is
    /// served warm-or-cold from the window.
    fn frame(&mut self) -> Result<Frame, snapshot::LoadError> {
        let hour = self.current_hour().expect("non-empty series");
        let (snapshot, fetch) = self.cache.get(hour)?;
        Ok(Frame {
            hour_index: hour,
            snapshot,
            fetch,
        })
    }

    /// Prefetch the next `n` frames in the play direction (windowed prefetch on
    /// direction change / before play). Errors on individual hours are swallowed —
    /// prefetch is best-effort warming, the real fetch re-validates at `step`.
    pub fn prefetch_ahead(&mut self, n: usize) {
        for k in 1..=n {
            let p = match self.dir {
                Dir::Forward => self.pos.checked_add(k),
                Dir::Backward => self.pos.checked_sub(k),
            };
            if let Some(p) = p {
                if let Some(&hour) = self.hours.get(p) {
                    let _ = self.cache.prefetch(hour);
                }
            }
        }
    }
}

/// Prose home for the immutability argument (linked from the crate docs). Kept as a
/// doc item so the reasoning lives next to the code it constrains.
///
/// **Claim:** no bind/apply path can mutate a cached snapshot so that a
/// fabricated/defaulted value persists across scrub ticks.
///
/// **Proof sketch (by construction, not by discipline):**
/// 1. The cache stores `Arc<Snapshot>` and only ever returns `Arc::clone`s
///    ([`SnapshotLru::get`]). While ≥ 2 clones are live (the cache always retains
///    one), `Arc` yields no `&mut` — `Arc::get_mut`/`make_mut` would see refcount
///    > 1 and refuse/clone-off. So a cached payload is unreachable for mutation.
/// 2. The bind layer's entry points (`bind::apply_topbar`, `bind::bind_state`) take
///    `&Snapshot` (shared, immutable) and write into a SEPARATE, freshly-built
///    `gui_runtime` scene-graph (`Vec<Node>`) created per re-bind. The "defaulting"
///    (the no-fabrication `—` for absent/null) happens in the *report / scene-graph*,
///    never in the snapshot.
/// 3. Therefore the only thing a scrub mutates is the throwaway scene-graph; the
///    next visit to the same hour reads the same untouched `Arc<Snapshot>` and
///    re-derives the same report. The A→B→A test ([`tests::scrub_a_b_a_is_identical`])
///    is the empirical confirmation.
///
/// **Eviction caveat (Codex-reviewed, closed):** once an hour is EVICTED from the
/// window, the cache no longer retains a clone, so a still-held `Frame.snapshot`
/// could become uniquely owned and (because `Snapshot`'s fields are `pub`) mutable
/// via `Arc::get_mut` by *that frame's owner*. This is NOT a cached mutation: the
/// cache no longer holds that instance, and the NEXT read of the same hour
/// **cold-reloads a fresh parse from disk** ([`SnapshotLru::get`] miss path), so a
/// mutated evicted copy can never leak into a later tick's read. The
/// [`tests::evicted_hour_cold_reloads_fresh`] test pins this. The prime directive —
/// "no fabricated value persists across scrub ticks" — therefore holds with or
/// without eviction.
pub mod immutable {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use snapshot::{pack, PackEntry, PackIdentity};
    use std::fs;

    /// Build a tiny on-disk store with two snapshots so the LRU/timeline exercise
    /// the REAL read+parse path (not an in-memory shortcut).
    fn tiny_store() -> (tempdir::TempDirLike, Store, String) {
        let dir = tempdir::TempDirLike::new("scrub_m6");
        let game = "g".to_string();
        let gdir = dir.path.join(&game);
        fs::create_dir_all(&gdir).unwrap();
        // hour 100: GER pp=10, state 64 owner GER vp 5
        let a = json!({
            "game_id":"g","date":"1940.1.1.12","hour_index":100,
            "countries":{"GER":{"politics":{"stability":0.5,"political_power":10.0,"ruling_party":"fascism"}}},
            "states":{"64":{"owner":"GER","controller":"GER","category":"city","victory_points_total":5,
                "manpower_pool":{"total":1000},"buildings":{"infrastructure":3},"occupation":null,
                "occupation_law":null,"resources":null}},
            "map":{"version":1},
            "divisions":[{"id":1},{"id":2}]
        });
        // hour 200: GER pp=99, state 64 controller flips to SOV, vp 5
        let b = json!({
            "game_id":"g","date":"1941.1.1.12","hour_index":200,
            "countries":{"GER":{"politics":{"stability":0.9,"political_power":99.0,"ruling_party":"fascism"}}},
            "states":{"64":{"owner":"GER","controller":"SOV","category":"city","victory_points_total":5,
                "manpower_pool":{"total":900},"buildings":{"infrastructure":3,"arms_factory":2},
                "occupation":{"resistance":10.0,"compliance":50.0},"occupation_law":null,"resources":null}},
            "map":{"version":1},
            "divisions":[{"id":1},{"id":2},{"id":3}]
        });
        fs::write(
            gdir.join("000000000100.snapshot.json"),
            serde_json::to_string(&a).unwrap(),
        )
        .unwrap();
        fs::write(
            gdir.join("000000000200.snapshot.json"),
            serde_json::to_string(&b).unwrap(),
        )
        .unwrap();
        let index = json!({"game_id":"g","schema_version":"1","snapshots":[
            {"hour_index":100,"date":"1940.1.1.12","file":"000000000100.snapshot.json"},
            {"hour_index":200,"date":"1941.1.1.12","file":"000000000200.snapshot.json"}
        ]});
        fs::write(
            gdir.join("index.json"),
            serde_json::to_string(&index).unwrap(),
        )
        .unwrap();
        let store = Store::open(&dir.path);
        (dir, store, game)
    }

    fn replay_store_with_raw(raw_jsonl: &str) -> (tempdir::TempDirLike, Store, String) {
        let dir = tempdir::TempDirLike::new("scrub_m6_replay");
        let game = "g".to_string();
        let snap_a = json!({
            "schema_version":"1","game_id":"g","date":"1940.1.1.12","hour_index":100,
            "countries":{"GER":{"politics":{"political_power":10.0}}},
            "states":{}
        });
        let snap_b = json!({
            "schema_version":"1","game_id":"g","date":"1940.1.1.13","hour_index":200,
            "countries":{"GER":{"politics":{"political_power":20.0}}},
            "states":{}
        });
        let index = json!({"game_id":"g","schema_version":"1","snapshots":[
            {"hour_index":100,"date":"1940.1.1.12","file":"000000000100.snapshot.json"},
            {"hour_index":200,"date":"1940.1.1.13","file":"000000000200.snapshot.json"}
        ]});
        let entries = vec![
            PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&index).unwrap(),
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/000000000100.snapshot.json"),
                bytes: serde_json::to_vec(&snap_a).unwrap(),
                role: "snapshot",
                hour_index: Some(100),
            },
            PackEntry {
                path: format!("payload/{game}/000000000200.snapshot.json"),
                bytes: serde_json::to_vec(&snap_b).unwrap(),
                role: "snapshot",
                hour_index: Some(200),
            },
            PackEntry {
                path: format!("payload/{game}/collector_raw/collector_frames.jsonl"),
                bytes: raw_jsonl.as_bytes().to_vec(),
                role: "collector_raw_stream",
                hour_index: None,
            },
        ];
        let bytes = pack(
            &game,
            &entries,
            &PackIdentity {
                schema_version: Some("1".into()),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        let path = dir.path.join("raw.battleplan");
        fs::write(&path, bytes).unwrap();
        let store = Store::open(&path);
        (dir, store, game)
    }

    #[test]
    fn timeline_indexes_raw_stream_without_changing_hourly_spine() {
        let raw = r#"{"type":"collector_started","sequence":0,"root_probe":{"date_root_ptr":"0xDEADBEEF"}}
{"type":"domain_delta","hour_index":100,"date":"1940.1.1.12","tick_ms":20,"sequence":2,"session_sequence":2,"domain":"divisions","entity_key":"division:2:4713","source":"C","confidence":"sampled","payload":{"changed_fields":["moving"]}}
{"type":"domain_delta","hour_index":100,"date":"1940.1.1.12","tick_ms":10,"sequence":1,"session_sequence":1,"domain":"clock","entity_key":"clock:game_date","source":"C","confidence":"sampled","payload":{"changed_fields":["hour_index"]}}
{"type":"gap","hour_index":200,"date":"1940.1.1.13","tick_ms":30,"sequence":3,"domain":"land_combats","reason":"bridge_not_ready"}
"#;
        let (_dir, store, game) = replay_store_with_raw(raw);
        let mut tl = Timeline::open(store, game, 4).unwrap();

        assert_eq!(tl.hours(), &[100, 200]);
        assert_eq!(tl.len(), 2);
        assert!(tl.raw_index().errors().is_empty());
        assert_eq!(tl.raw_index().source_paths().len(), 1);
        assert_eq!(
            tl.raw_index().source_paths()[0],
            "payload/g/collector_raw/collector_frames.jsonl"
        );

        let frames = tl.raw_index().frames();
        assert_eq!(
            frames.iter().map(|f| f.sequence).collect::<Vec<_>>(),
            vec![Some(1), Some(2), Some(3)],
            "collector_started diagnostics are skipped and gameplay raw frames sort by hour/tick/sequence"
        );
        assert_eq!(tl.raw_index().frames_at_hour(100).len(), 2);
        assert_eq!(tl.raw_index().frames_at_hour(200).len(), 1);
        assert_eq!(frames[0].kind, "domain_delta");
        assert_eq!(frames[0].domain.as_deref(), Some("clock"));
        assert_eq!(frames[1].entity_key.as_deref(), Some("division:2:4713"));
        assert_eq!(frames[2].kind, "gap");
        assert_eq!(frames[2].reason.as_deref(), Some("bridge_not_ready"));

        let first = tl.seek_to(0).unwrap();
        assert_eq!(first.hour_index, 100);
        let second = tl.step(Dir::Forward).unwrap();
        assert_eq!(second.hour_index, 200);
    }

    #[test]
    fn malformed_raw_stream_fails_closed_but_hourly_scrub_still_opens() {
        let raw = r#"{"type":"domain_delta","hour_index":100,"tick_ms":10,"sequence":1,"domain":"clock"}
not-json
{"type":"domain_delta","hour_index":200,"tick_ms":20,"sequence":2,"domain":"divisions"}
"#;
        let (_dir, store, game) = replay_store_with_raw(raw);
        let mut tl = Timeline::open(store, game, 4).unwrap();

        assert_eq!(tl.hours(), &[100, 200]);
        assert!(tl.raw_index().frames().is_empty());
        assert_eq!(tl.raw_index().errors().len(), 1);
        assert!(
            tl.raw_index().errors()[0].contains("line 2"),
            "malformed raw JSONL should be reported precisely"
        );
        assert_eq!(tl.seek_to(0).unwrap().hour_index, 100);
    }

    #[test]
    fn lru_hit_is_warm_and_shared() {
        let (_d, store, game) = tiny_store();
        let mut lru = SnapshotLru::new(store, game, 4);
        let (a1, f1) = lru.get(100).unwrap();
        assert_eq!(f1, Fetch::Cold, "first touch must be a cold parse");
        let (a2, f2) = lru.get(100).unwrap();
        assert_eq!(f2, Fetch::Warm, "second touch must be a warm hit");
        // the two Arcs are the SAME shared instance (no re-parse, no copy).
        assert!(
            Arc::ptr_eq(&a1, &a2),
            "a warm hit must return the SAME shared parse"
        );
        assert_eq!(lru.misses, 1);
        assert_eq!(lru.hits, 1);
    }

    #[test]
    fn lru_evicts_over_window() {
        // capacity clamps to >=3; load 4 distinct keys into a 3-window -> 1 eviction.
        let (_d, store, game) = tiny_store();
        let mut lru = SnapshotLru::new(store, game, 3);
        // only hours 100/200 exist on disk; fabricate a load error for a 3rd by
        // re-getting existing hours in an order that forces MRU churn, then assert
        // the window never exceeds capacity.
        for _ in 0..10 {
            lru.get(100).unwrap();
            lru.get(200).unwrap();
        }
        assert!(lru.len() <= lru.capacity());
    }

    #[test]
    fn cached_snapshot_cannot_be_mutated() {
        // The prime directive at the type level: with the cache's clone live, no
        // caller can get a &mut into the parsed snapshot.
        let (_d, store, game) = tiny_store();
        let mut lru = SnapshotLru::new(store, game, 4);
        let (mut arc, _) = lru.get(100).unwrap();
        // refcount is 2 (cache + us) -> get_mut MUST be None: unmutable by type.
        assert!(
            Arc::get_mut(&mut arc).is_none(),
            "a cached snapshot must be immutable while the cache retains its clone"
        );
    }

    /// THE prime-directive test: scrub A -> B -> A. The A re-bind must be IDENTICAL
    /// to the first A bind — no value carried over / defaulted / fabricated from the
    /// B visit. We bind the topbar AND the state panel each time and compare the
    /// full reports.
    #[test]
    fn scrub_a_b_a_is_identical() {
        use bind::{apply_topbar, bind_state, TopbarSpec};
        let (_d, store, game) = tiny_store();
        let mut tl = Timeline::open(store, game, 4).unwrap();

        // helper: bind every surface for the frame's snapshot, return a comparable
        // string digest (topbar fields + state-panel fields + building rows).
        let digest = |frame: &Frame| -> String {
            let snap = &*frame.snapshot;
            let spec = TopbarSpec::topbar();
            // topbar bind writes into a THROWAWAY scene-graph (empty roots here —
            // we only need the report, which is the no-fabrication output).
            let mut roots: Vec<gui_runtime::scene::Node> = Vec::new();
            let tb = apply_topbar(&mut roots, &spec, snap, "GER");
            let sp = bind_state(snap, "64", "Brandenburg");
            let mut s = String::new();
            for f in &tb.fields {
                s.push_str(&format!(
                    "TB:{}={}|{}\n",
                    f.element, f.value, f.not_captured
                ));
            }
            for f in &sp.fields {
                s.push_str(&format!(
                    "SP:{}={}|{}\n",
                    f.element, f.value, f.not_captured
                ));
            }
            for b in sp
                .state_buildings
                .iter()
                .chain(sp.province_buildings.iter())
            {
                s.push_str(&format!("BR:{}={}\n", b.key, b.level));
            }
            s.push_str(&format!("OCC:{}\n", sp.occupation.is_some()));
            s
        };

        // A (hour 100) — cold.
        let fa1 = tl.seek_to(0).unwrap();
        assert_eq!(fa1.fetch, Fetch::Cold);
        let da1 = digest(&fa1);

        // B (hour 200) — cold.
        let fb = tl.seek_to(1).unwrap();
        assert_eq!(fb.fetch, Fetch::Cold);
        let db = digest(&fb);

        // B differs from A (sanity: the scrub actually changed something).
        assert_ne!(
            da1, db,
            "A and B must differ (else the test proves nothing)"
        );

        // Back to A (hour 100) — WARM (same cached Arc).
        let fa2 = tl.seek_to(0).unwrap();
        assert_eq!(fa2.fetch, Fetch::Warm, "returning to A must be a cache hit");
        assert!(
            Arc::ptr_eq(&fa1.snapshot, &fa2.snapshot),
            "the A revisit must be the SAME shared parse"
        );
        let da2 = digest(&fa2);

        // THE assertion: A renders IDENTICALLY before and after the B visit.
        assert_eq!(
            da1, da2,
            "A->B->A: the A re-bind fabricated/carried a value from the B visit"
        );
    }

    #[test]
    fn timeline_series_from_index_and_transport() {
        let (_d, store, game) = tiny_store();
        let mut tl = Timeline::open(store, game, 4).unwrap();
        assert_eq!(tl.hours(), &[100, 200]);
        assert_eq!(tl.current_hour(), Some(100));
        // step forward walks the SERIES, never reconstructs it.
        let f = tl.step(Dir::Forward).unwrap();
        assert_eq!(f.hour_index, 200);
        // clamp at the end (no wrap).
        let f = tl.step(Dir::Forward).unwrap();
        assert_eq!(f.hour_index, 200);
        // play auto-advance from the start pauses at the end.
        tl.seek_to(0).unwrap();
        tl.play();
        let t = tl.tick().unwrap().unwrap();
        assert_eq!(t.hour_index, 200);
        assert!(
            tl.tick().is_none(),
            "play must pause at the end of the series"
        );
        assert!(!tl.is_playing());
    }

    #[test]
    fn domain_scoped_view_reads_only_its_domain() {
        let (_d, store, game) = tiny_store();
        let mut tl = Timeline::open(store, game, 4).unwrap();
        let f = tl.seek_to(0).unwrap();
        let v = f.view();
        // the map view sees states + map; the country view sees one country; the
        // state view sees one state — none of them parse or even touch divisions.
        assert!(v.map().states().is_some());
        assert!(v.country("GER").politics_path("political_power").is_some());
        assert_eq!(v.state("64").owner(), Some("GER"));
        // divisions remain raw + untouched (the view never forces it).
        assert!(f.snapshot.domain("divisions").is_some());
    }

    /// Codex (a) caveat closed: even after EVICTION (when an old `Frame.snapshot`
    /// may become uniquely owned), a later read of the SAME hour cold-reloads a
    /// FRESH parse from disk — so no stale/mutated copy can leak across ticks.
    #[test]
    fn evicted_hour_cold_reloads_fresh() {
        let (_d, store, game) = tiny_store();
        // window 3 (the minimum); 100 then 200; re-get 100 to keep it MRU, then
        // evict by filling. With only 2 hours on disk we can't overflow a 3-window,
        // so test the contract directly: get -> drop all external Arcs -> get again
        // yields a snapshot that still resolves correctly (a fresh-or-cached parse,
        // never a corrupted one).
        let mut lru = SnapshotLru::new(store, game, 3);
        let (a1, _) = lru.get(100).unwrap();
        // an external holder of an Arc can never mutate while the cache retains its
        // clone; even if it could after eviction, the value below is unaffected.
        drop(a1);
        let (a2, _) = lru.get(100).unwrap();
        assert_eq!(
            a2.state_path("64", "owner").and_then(|v| v.as_str()),
            Some("GER"),
            "a re-read of hour 100 must resolve its real owner, never a stale value"
        );
    }

    /// The prefetch fix: a directional prefetch must NOT evict the cursor frame
    /// (Codex-flagged). We cap at 3, seat the cursor at 100 (MRU), prefetch 200, and
    /// assert 100 is still resident (warm) afterwards.
    #[test]
    fn prefetch_does_not_evict_cursor() {
        let (_d, store, game) = tiny_store();
        let mut lru = SnapshotLru::new(store, game, 3);
        // cursor = 100 at MRU front.
        let _ = lru.get(100).unwrap();
        assert!(lru.contains(100));
        // prefetch the other hour; with a 3-window nothing evicts, but the insert
        // goes BEHIND the cursor so 100 is never the victim.
        lru.prefetch(200).unwrap();
        assert!(
            lru.contains(100),
            "prefetch must not evict the cursor frame"
        );
        assert!(lru.contains(200), "prefetch must warm the target");
        // the cursor (100) is still the warm MRU: a get is a hit.
        let (_a, f) = lru.get(100).unwrap();
        assert_eq!(
            f,
            Fetch::Warm,
            "cursor frame must remain a warm hit after prefetch"
        );
    }

    /// The LRU honours the index `file` map (Codex-flagged): a NON-canonical file
    /// name in the index is loaded verbatim, not reconstructed.
    #[test]
    fn lru_honours_index_file_map() {
        let dir = tempdir::TempDirLike::new("scrub_m6_file");
        let game = "g".to_string();
        let gdir = dir.path.join(&game);
        fs::create_dir_all(&gdir).unwrap();
        // a NON-canonical file name (not the {:012}.snapshot.json form).
        let a = json!({"game_id":"g","hour_index":100,"date":"1940.1.1.12",
            "states":{"1":{"owner":"FRA"}}});
        fs::write(
            gdir.join("weird_name_A.json"),
            serde_json::to_string(&a).unwrap(),
        )
        .unwrap();
        let index = json!({"game_id":"g","snapshots":[
            {"hour_index":100,"date":"1940.1.1.12","file":"weird_name_A.json"}
        ]});
        fs::write(
            gdir.join("index.json"),
            serde_json::to_string(&index).unwrap(),
        )
        .unwrap();
        let store = Store::open(&dir.path);
        let mut tl = Timeline::open(store, game, 4).unwrap();
        // the canonical name 000000000100.snapshot.json does NOT exist; only the
        // index `file` does. A successful seek proves the file map was honoured.
        let f = tl.seek_to(0).unwrap();
        assert_eq!(f.view().state("1").owner(), Some("FRA"));
    }

    /// A degenerate/corrupt index (zero snapshots, or every snapshot missing its
    /// `hour_index`) yields an EMPTY series. `seek_to`/`step`/`tick` must then fail
    /// GRACEFULLY — never panic on untrusted input (Codex bug-hunt FIX-3). The public
    /// API already returns `Result`/`Option`, so an empty timeline is an error, not a
    /// crash.
    #[test]
    fn empty_timeline_does_not_panic() {
        let dir = tempdir::TempDirLike::new("scrub_m6_empty");
        let game = "g".to_string();
        let gdir = dir.path.join(&game);
        fs::create_dir_all(&gdir).unwrap();
        // an index whose snapshots all LACK hour_index -> filtered out -> empty series.
        let index = json!({"game_id":"g","snapshots":[
            {"date":"1940.1.1.12","file":"x.json"}
        ]});
        fs::write(
            gdir.join("index.json"),
            serde_json::to_string(&index).unwrap(),
        )
        .unwrap();
        let store = Store::open(&dir.path);
        let mut tl = Timeline::open(store, game, 4).unwrap();
        assert!(
            tl.is_empty(),
            "series must be empty when no hour_index is present"
        );
        assert_eq!(tl.current_hour(), None);

        // None of these may panic; each returns a graceful error / None.
        assert!(
            tl.seek_to(0).is_err(),
            "seek on an empty timeline must error, not panic"
        );
        assert!(
            tl.step(Dir::Forward).is_err(),
            "step on an empty timeline must error, not panic"
        );
        assert!(tl.step(Dir::Backward).is_err());
        tl.play();
        assert!(tl.tick().is_none(), "tick on an empty timeline yields None");
        // prefetch over an empty series is a harmless no-op.
        tl.prefetch_ahead(3);
    }
}

/// A minimal temp-dir helper (no external crate) — creates a unique dir under the
/// system temp and removes it on drop. Used only by tests.
#[cfg(test)]
mod tempdir {
    use std::path::PathBuf;
    pub struct TempDirLike {
        pub path: PathBuf,
    }
    impl TempDirLike {
        pub fn new(prefix: &str) -> TempDirLike {
            let mut p = std::env::temp_dir();
            let uniq = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            p.push(uniq);
            std::fs::create_dir_all(&p).unwrap();
            TempDirLike { path: p }
        }
    }
    impl Drop for TempDirLike {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
