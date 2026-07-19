//! Shared scaffolding for the soak test binaries (`tests/soak.rs` = F3 long-run
//! harness, `tests/soak_f2.rs` = F2 budget-knob gates).
//!
//! Compiled into each binary via `mod soak_support;` — the mod.rs-in-
//! subdirectory form keeps Cargo from treating this file as a standalone test
//! target. Measurement-critical pieces (the counting allocator, the
//! `INDEX_COPIES` subtraction constant, the synthetic source) live here exactly
//! once so the two binaries cannot drift.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use replay_core::{RawFrame, ReplaySource, SourceError, WorldFrame, WorldTimeline};
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Memory measurement: a counting global allocator.
//
// Net-live bytes = sum(alloc.size) - sum(dealloc.size). Sampling it at loop
// boundaries (when the harness holds no frame Arcs) reflects exactly what the
// timeline is retaining: the fixed-size structures plus the budget-capped
// frame cache. A leak accumulates here monotonically; a bounded design stays
// flat. `realloc` routes through the default GlobalAlloc impl (alloc +
// dealloc), so it is accounted without an explicit override.
// ---------------------------------------------------------------------------

struct CountingAlloc;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

pub fn live_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// Number of O(length) hour-index vectors held live during a scrub: the
/// `ReplaySource`'s own `hours` vector, plus `WorldTimeline::open`'s defensive
/// `to_vec()` copy of it. Both are `i64`-per-hour index spines (NOT frame
/// payloads) and scale with timeline length by design; the F2-style gates
/// subtract them so they measure only the budget-capped frame cache.
pub const INDEX_COPIES: usize = 2;

pub fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

// ---------------------------------------------------------------------------
// Synthetic in-memory ReplaySource: an arbitrarily long hourly timeline.
//
// Frames are generated on demand (the source stores only the hour vector), so
// the source itself never scales its live footprint with the scrub — the ONLY
// thing that can grow during a scrub is the timeline's frame cache, which is
// exactly what the F2-style ceiling gates are measuring.
//
// `pad_repeats` controls an optional deterministic per-frame payload (hex,
// derived from the hour) under an unknown top-level domain ("soak_pad") that
// `WorldFrame::raw()` preserves verbatim: 0 omits the key entirely (the F3
// harness's small frames), 1024 yields 16 chars x 1024 = 16 KiB per cached
// frame — the measurable unit of the F2 budget gates.
// ---------------------------------------------------------------------------

pub struct SyntheticSource {
    game_id: String,
    hours: Vec<i64>,
    pad_repeats: usize,
}

impl SyntheticSource {
    pub fn new(hour_count: usize, game_id: &str, pad_repeats: usize) -> SyntheticSource {
        let base: i64 = 16_959_360; // matches the fixture epoch style
        let hours: Vec<i64> = (0..hour_count as i64).map(|i| base + i).collect();
        SyntheticSource {
            game_id: game_id.to_string(),
            hours,
            pad_repeats,
        }
    }
}

impl ReplaySource for SyntheticSource {
    fn game_id(&self) -> &str {
        &self.game_id
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &[]
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        if self.hours.binary_search(&hour_index).is_err() {
            return Err(SourceError::Open(format!("no such hour {hour_index}")));
        }
        // A small but non-trivial payload whose values are a deterministic
        // function of the hour, so seek==play equality compares real content
        // and the typed-view builders actually run each frame.
        let owner = if (hour_index % 2) == 0 { "GER" } else { "SOV" };
        let mut states = Map::new();
        for s in 0..4u32 {
            states.insert(
                s.to_string(),
                json!({ "owner": owner, "controller": owner }),
            );
        }
        let mut rest = Map::new();
        rest.insert("states".to_string(), Value::Object(states));
        rest.insert(
            "countries".to_string(),
            json!({ "GER": { "politics": { "stability": (hour_index % 100) as f64 / 100.0 } } }),
        );
        if self.pad_repeats > 0 {
            // Known-sized payload: an unknown top-level domain is preserved
            // verbatim in WorldFrame::raw(), so each CACHED frame retains
            // exactly one pad_repeats x 16-char copy.
            rest.insert(
                "soak_pad".to_string(),
                Value::String(format!("{hour_index:016x}").repeat(self.pad_repeats)),
            );
        }
        Ok(RawFrame {
            hour_index,
            date: Some(format!("1936.1.1.{}", hour_index % 24)),
            save_version: None,
            schema_version: Some(json!("1")),
            rest,
        })
    }
}

// ---------------------------------------------------------------------------
// A4 equality (reused invariant): seek(h) and play_to(h) must resolve to
// content-equal frames. We compare the canonical hour, the raw domain map, and
// a representative typed view (owner_by_state) — the same equality A4 gates.
// ---------------------------------------------------------------------------

pub fn assert_seek_eq_play(tl: &WorldTimeline, hour: i64) {
    let s: Arc<WorldFrame> = tl.seek(hour).expect("seek");
    let p: Arc<WorldFrame> = tl.play_to(hour).expect("play_to");
    assert_eq!(s.hour_index(), p.hour_index(), "hour mismatch @ {hour}");
    assert_eq!(s.raw(), p.raw(), "raw-frame mismatch @ {hour}");
    assert_eq!(
        s.owner_by_state(),
        p.owner_by_state(),
        "owner view mismatch @ {hour}"
    );
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(name)
}

// ---------------------------------------------------------------------------
// Tiny deterministic PRNG (xorshift-based LCG). No external dep; the scrub
// scripts must be reproducible so a failure is re-runnable.
// ---------------------------------------------------------------------------

pub struct Lcg {
    state: u64,
}
impl Lcg {
    pub fn new(seed: u64) -> Lcg {
        Lcg {
            state: seed | 1, // never zero
        }
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}
