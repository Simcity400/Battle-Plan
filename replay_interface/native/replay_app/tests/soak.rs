//! F3 — long-run soak harness (headless data-path).
//!
//! A scripted play + scrub loop over recordings, asserting the timeline data
//! path is leak-free and its working set stays bounded. This is the harness
//! that VALIDATES the F2 memory ceiling (snapshot spine + LRU): it measures,
//! and if the current (pre-F2) code grew unboundedly it would fail rather
//! than fake a pass.
//!
//! Design (why headless): per the plan, F3 drives "the timeline + the data
//! path, not necessarily D3D". D3D device-removed is a swapchain concern with
//! no window in CI; this harness exercises everything a renderer consumes —
//! `replay_core`'s `WorldTimeline` (ingest → cache/LRU → typed views) — and
//! proves it does not leak or grow under a multi-thousand-iteration
//! seek/play/scrub script. `replay_core` is READ-ONLY here (consumed only).
//!
//! What is gated (numeric, no-eyeball):
//!   1. No panic / zero accumulated errors across the whole script.
//!   2. A4 invariant `seek(h) == play_to(h)` holds on every probed hour,
//!      throughout the run (content equality of the resolved frames).
//!   3. Bounded memory: net live-heap bytes (custom counting allocator) after
//!      N iterations does not exceed the post-warmup baseline by more than a
//!      documented ceiling — a genuine per-iteration leak would blow past it.
//!   4. F2 ceiling hook: post-scrub working set over a MULTI-YEAR (26k-hour)
//!      timeline is within a small ceiling of the SAME scrub over a 24-hour
//!      timeline — proving the working set is bounded by the frame cache, not
//!      by timeline length.
//!
//! Iteration counts are CI-fast by default and overridable via env for a real
//! long soak:
//!   BATTLE_PLAN_SOAK_SWEEPS      full fwd+back sweeps of the fixture   (default 2000)
//!   BATTLE_PLAN_SOAK_SCRUB       random seeks per synthetic timeline   (default 20000)
//!   BATTLE_PLAN_SOAK_HOURS_LONG  hours in the multi-year timeline      (default 26280)

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use replay_core::{
    open_recording, RawFrame, ReplaySource, SourceError, WorldFrame, WorldTimeline,
};
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Memory measurement: a counting global allocator.
//
// Net-live bytes = sum(alloc.size) - sum(dealloc.size). Sampling it at loop
// boundaries (when the harness holds no frame Arcs) reflects exactly what the
// timeline is retaining: the fixed-size structures plus the ≤16-entry frame
// cache. A leak accumulates here monotonically; a bounded design stays flat.
// `realloc` routes through the default GlobalAlloc impl (alloc + dealloc), so
// it is accounted without an explicit override.
// ---------------------------------------------------------------------------

struct CountingAlloc;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
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
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn live_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Documented ceilings (see LANE_REPORT.md for the derivation).
// ---------------------------------------------------------------------------

/// Post-warmup live-heap growth allowed across the whole seek/play/scrub
/// script. The cache is capped at 16 frames and the harness holds no frames at
/// a sample point, so the true steady-state delta is ~0; this 1 MiB budget
/// absorbs allocator fragmentation and any concurrently-running sibling test
/// while still catching a real leak (even 64 B/iter × 20k iters = 1.25 MiB).
const LEAK_CEILING_BYTES: usize = 1 << 20; // 1 MiB

/// F2 ceiling: how much MORE *frame working set* a 26,280-hour timeline may
/// retain vs a 24-hour timeline after the same scrub, once the legitimately
/// O(length) hour-index vectors are subtracted out. Both timelines are bounded
/// by the same 16-frame LRU cap, so this residual must be ~0; 64 KiB absorbs
/// allocator fragmentation while staying far below any real per-hour retention.
const F2_CEILING_BYTES: usize = 64 << 10; // 64 KiB

/// Number of O(length) hour-index vectors held live during a scrub: the
/// `ReplaySource`'s own `hours` vector, plus `WorldTimeline::open`'s defensive
/// `to_vec()` copy of it. Both are `i64`-per-hour index spines (NOT frame
/// payloads) and scale with timeline length by design; the F2 gate subtracts
/// them so it measures only the LRU-capped frame cache.
const INDEX_COPIES: usize = 2;

fn env_usize(key: &str, default: usize) -> usize {
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
// exactly what the F2 ceiling gate is measuring.
// ---------------------------------------------------------------------------

struct SyntheticSource {
    game_id: String,
    hours: Vec<i64>,
}

impl SyntheticSource {
    fn new(hour_count: usize) -> SyntheticSource {
        let base: i64 = 16_959_360; // matches the fixture epoch style
        let hours: Vec<i64> = (0..hour_count as i64).map(|i| base + i).collect();
        SyntheticSource {
            game_id: "soak_synthetic".to_string(),
            hours,
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

fn assert_seek_eq_play(tl: &WorldTimeline, hour: i64) {
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

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(name)
}

// ---------------------------------------------------------------------------
// Gate 1+2+3: real production ingest path (committed .battleplan fixture),
// long fwd/back sweep script, seek==play throughout, bounded live heap.
// ---------------------------------------------------------------------------

#[test]
fn soak_real_fixture_leakfree_and_invariant() {
    let sweeps = env_usize("BATTLE_PLAN_SOAK_SWEEPS", 2000);

    let path = fixture_path("fx_owner_controller_flip.battleplan");
    assert!(path.exists(), "missing committed fixture: {}", path.display());

    let source = open_recording(&path).expect("open committed fixture");
    let timeline = WorldTimeline::open(source).expect("open timeline");
    let hours: Vec<i64> = timeline.hours().to_vec();
    assert!(hours.len() >= 2, "fixture must have a scrubbable span");

    // Warmup: one full sweep fills the (≤16-entry) cache to steady state.
    for &h in &hours {
        assert_seek_eq_play(&timeline, h);
    }
    let baseline = live_bytes();

    // Scripted loop: forward sweep + backward sweep, plus midpoint random
    // jumps, many thousands of times. Track errors instead of unwrapping the
    // scrub calls so "zero accumulated errors" is an explicit gate.
    let mut errors: usize = 0;
    let mut rng = Lcg::new(0x5EED_1234);
    // Checkpoints to confirm the live heap is flat over time, not just at the
    // endpoints (a slow leak would show rising checkpoints).
    let checkpoint_every = (sweeps / 4).max(1);
    let mut checkpoints: Vec<usize> = Vec::new();

    for sweep in 0..sweeps {
        for &h in &hours {
            if timeline.seek(h).is_err() {
                errors += 1;
            }
            assert_seek_eq_play(&timeline, h);
        }
        for &h in hours.iter().rev() {
            if timeline.play_to(h).is_err() {
                errors += 1;
            }
        }
        // A couple of arbitrary in-range jumps (scrub, not linear play).
        for _ in 0..3 {
            let span = (timeline.last_hour() - timeline.first_hour()).max(1);
            let h = timeline.first_hour() + (rng.next_u64() as i64).rem_euclid(span + 1);
            if timeline.seek(h).is_err() {
                errors += 1;
            }
        }
        if sweep % checkpoint_every == 0 {
            checkpoints.push(live_bytes());
        }
    }
    checkpoints.push(live_bytes());

    let end = live_bytes();
    let growth = end.saturating_sub(baseline);

    assert_eq!(errors, 0, "scrub script accumulated {errors} errors");
    assert!(
        growth <= LEAK_CEILING_BYTES,
        "live heap grew {growth} B over {sweeps} sweeps (baseline {baseline} B, \
         end {end} B); exceeds {LEAK_CEILING_BYTES} B ceiling — probable leak. \
         checkpoints={checkpoints:?}"
    );

    // Trend gate: the last checkpoint must not tower over the first (a leak
    // that stayed under the absolute ceiling but is clearly monotonic).
    if checkpoints.len() >= 2 {
        let first = checkpoints[0];
        let last = *checkpoints.last().unwrap();
        let trend = last.saturating_sub(first);
        assert!(
            trend <= LEAK_CEILING_BYTES,
            "live-heap checkpoints trend upward by {trend} B: {checkpoints:?}"
        );
    }

    eprintln!(
        "[soak] real-fixture: sweeps={sweeps} hours={} baseline={baseline}B end={end}B \
         growth={growth}B errors=0",
        hours.len()
    );
}

// ---------------------------------------------------------------------------
// Gate 4 (F2 ceiling hook): random-access scrub over a 24-hour vs a
// multi-year (26,280-hour) synthetic timeline. Post-scrub working set must be
// bounded by the frame cache, NOT by timeline length. This is the measurement
// F2 (snapshot spine + LRU) must satisfy; on today's fixed 16-frame cache it
// already holds, and this test will keep F2 honest.
// ---------------------------------------------------------------------------

fn scrub_and_measure(hour_count: usize, seeks: usize) -> (usize, usize) {
    let source = Box::new(SyntheticSource::new(hour_count)) as Box<dyn ReplaySource>;
    let timeline = WorldTimeline::open(source).expect("open synthetic timeline");
    let first = timeline.first_hour();
    let span = (timeline.last_hour() - first).max(1);

    let mut rng = Lcg::new(0xC0FFEE ^ hour_count as u64);
    let mut errors = 0usize;
    for i in 0..seeks {
        let h = first + (rng.next_u64() as i64).rem_euclid(span + 1);
        match (timeline.seek(h), timeline.play_to(h)) {
            (Ok(s), Ok(p)) => {
                if s.hour_index() != p.hour_index() || s.raw() != p.raw() {
                    errors += 1;
                }
            }
            _ => errors += 1,
        }
        // Occasionally verify equality against the exact-hour view too.
        if i % 997 == 0 {
            assert_seek_eq_play(&timeline, h);
        }
    }
    assert_eq!(errors, 0, "synthetic scrub accumulated {errors} errors");
    // At this sample point the harness holds no frame Arcs; live == fixed
    // structures + the O(length) index vectors + the ≤16-entry frame cache.
    // The caller subtracts the index cost to isolate the cache.
    let live = live_bytes();
    let index_bytes = INDEX_COPIES * hour_count * std::mem::size_of::<i64>();
    drop(timeline);
    (live, index_bytes)
}

#[test]
fn soak_f2_working_set_bounded_by_cache_not_length() {
    let seeks = env_usize("BATTLE_PLAN_SOAK_SCRUB", 20_000);
    let hours_long = env_usize("BATTLE_PLAN_SOAK_HOURS_LONG", 26_280); // 3 years hourly
    let hours_short = 24usize;

    // Measure short first, then long, in the same process.
    let (live_short, index_short) = scrub_and_measure(hours_short, seeks);
    let (live_long, index_long) = scrub_and_measure(hours_long, seeks);

    // The hour-index vectors legitimately scale with timeline length (the
    // snapshot spine); they are O(hours) by construction and are NOT the cached
    // frame working set. Subtract them so the gate targets frame retention only.
    let cache_short = live_short.saturating_sub(index_short);
    let cache_long = live_long.saturating_sub(index_long);
    let extra = cache_long.saturating_sub(cache_short);

    eprintln!(
        "[soak] F2: seeks={seeks} short_hours={hours_short} long_hours={hours_long} \
         live_short={live_short}B live_long={live_long}B index_short={index_short}B \
         index_long={index_long}B cache_short={cache_short}B cache_long={cache_long}B \
         cache_extra={extra}B"
    );

    assert!(
        extra <= F2_CEILING_BYTES,
        "working set grew {extra} B when the timeline went from {hours_short} to \
         {hours_long} hours (cache_short={cache_short}B cache_long={cache_long}B); \
         a bounded (LRU) working set must not scale with timeline length. This is \
         the F2 ceiling — if it fires on post-F2 code the LRU is not capping."
    );
}

// ---------------------------------------------------------------------------
// Tiny deterministic PRNG (xorshift-based LCG). No external dep; the scrub
// script must be reproducible so a failure is re-runnable.
// ---------------------------------------------------------------------------

struct Lcg {
    state: u64,
}
impl Lcg {
    fn new(seed: u64) -> Lcg {
        Lcg {
            state: seed | 1, // never zero
        }
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}
