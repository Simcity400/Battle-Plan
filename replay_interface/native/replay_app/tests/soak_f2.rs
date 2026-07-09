//! F2 — memory-ceiling gates for the CONFIGURABLE frame-cache budget.
//!
//! The F3 harness (`tests/soak.rs`) proves the default 16-entry LRU bounds
//! the working set. This file gates the F2 rework proper — the budget knob
//! (`WorldTimeline::open_with_config` / `TimelineConfig.frame_cache_entries`,
//! CLI `--frame-cache`) — with numeric, no-eyeball gates:
//!
//!   (b) CORRECTNESS AT SHRUNKEN BUDGETS: with the cache shrunk to 4, 1 and
//!       even 0 entries, `seek(h) == play_to(h)` holds on every recorded hour
//!       of the committed production-ingest fixture, and frame content is
//!       identical to the default-budget timeline (the budget may only trade
//!       speed, never content).
//!   (a) LONG-TIMELINE BUDGET: after a 20k random-seek scrub of a multi-year
//!       (26,280-hour) synthetic timeline at a 4-entry budget, the frame
//!       working set exceeds the SAME scrub of a 24-hour timeline by at most
//!       a small ceiling (working set bounded by the budget, not by timeline
//!       length) — and the knob is MEMORY-AWARE: the same scrub at 16 entries
//!       retains measurably more than at 4 (the budget actually controls
//!       retention; frames carry a deterministic 16 KiB payload so the signal
//!       dominates allocator noise).
//!   (c) NO UPWARD HEAP TREND: >= 2000 fwd+back sweeps of the committed
//!       fixture at the shrunken 4-entry budget stay leak-free (absolute
//!       growth and checkpoint trend under the same 1 MiB law as F3).
//!
//! Env overrides (same knobs as soak.rs):
//!   BATTLE_PLAN_SOAK_SWEEPS      full fwd+back sweeps of the fixture   (default 2000)
//!   BATTLE_PLAN_SOAK_SCRUB       random seeks per synthetic timeline   (default 20000)
//!   BATTLE_PLAN_SOAK_HOURS_LONG  hours in the multi-year timeline      (default 26280)
//!
//! The allocator, PRNG and synthetic-source scaffolding mirror `tests/soak.rs`
//! (a separate test binary owned by the F3 lane; duplicated here rather than
//! restructuring that deliverable into a shared module).

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use replay_core::{
    open_recording, RawFrame, ReplaySource, SourceError, TimelineConfig, WorldFrame, WorldTimeline,
};
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Counting global allocator (net-live bytes; see soak.rs for the rationale).
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

fn live_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Ceilings. The correctness gate (b) runs as its own #[test]; both
// allocator-sampled gates (a) and (c) run SEQUENTIALLY inside one #[test] so
// they never sample across each other's live synthetic timelines. Gate (b)'s
// concurrent small-fixture caches are the only cross-test noise; the ceilings
// carry headroom for it.
// ---------------------------------------------------------------------------

/// Gate (a): extra frame working set a 26,280-hour timeline may retain over a
/// 24-hour timeline after the same scrub at the same 4-entry budget, once the
/// O(hours) index spines are subtracted. Both caches hold the same number of
/// same-sized frames, so the true residual is ~0; 128 KiB absorbs allocator
/// fragmentation plus gate (b)'s concurrent small caches, while a real
/// per-hour retention (26k hours x even 16 B) would blow past it.
const F2_LENGTH_CEILING_BYTES: usize = 128 << 10; // 128 KiB

/// Gate (a') floor: how much MORE the 16-entry budget must retain than the
/// 4-entry budget on the same long timeline. Expected signal: 12 extra frames
/// x (16 KiB payload + decode overhead) >= ~192 KiB; asserting half of the
/// payload-only figure keeps the gate robust to noise while proving the knob
/// controls retention.
const F2_KNOB_FLOOR_BYTES: usize = 96 << 10; // 96 KiB

/// Gate (c): same leak law as soak.rs (F3): post-warmup growth and checkpoint
/// trend across the whole sweep script must stay under 1 MiB.
const LEAK_CEILING_BYTES: usize = 1 << 20; // 1 MiB

/// O(length) hour-index spines live during a scrub: the source's own `hours`
/// vector + `WorldTimeline::open`'s defensive copy (see the F2 audit notes in
/// replay_core::source / world). Subtracted analytically by the caller.
const INDEX_COPIES: usize = 2;

/// Deterministic per-frame payload (hex, derived from the hour) so cached
/// frames have a known-sized footprint: 16 chars x 1024 = 16 KiB.
const PAD_REPEATS: usize = 1024;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

// ---------------------------------------------------------------------------
// Synthetic in-memory source (soak.rs pattern + the 16 KiB payload). Frames
// are generated on demand; the source itself holds only the hour vector.
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
            game_id: "soak_f2_synthetic".to_string(),
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
        // Known-sized payload: an unknown top-level domain is preserved
        // verbatim in WorldFrame::raw(), so each CACHED frame retains exactly
        // one 16 KiB copy — the measurable unit of the budget gates.
        rest.insert(
            "soak_pad".to_string(),
            Value::String(format!("{hour_index:016x}").repeat(PAD_REPEATS)),
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
// Shared helpers.
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

fn open_fixture_with_cap(cap: usize) -> WorldTimeline {
    let path = fixture_path("fx_owner_controller_flip.battleplan");
    assert!(
        path.exists(),
        "missing committed fixture: {}",
        path.display()
    );
    let source = open_recording(&path).expect("open committed fixture");
    WorldTimeline::open_with_config(
        source,
        TimelineConfig {
            frame_cache_entries: cap,
        },
    )
    .expect("open timeline")
}

struct Lcg {
    state: u64,
}
impl Lcg {
    fn new(seed: u64) -> Lcg {
        Lcg { state: seed | 1 }
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

// ---------------------------------------------------------------------------
// Gate (b): shrunken (and disabled) budgets stay CORRECT on the real
// production ingest path — seek == play_to on every recorded hour, and frame
// content identical to the default-budget timeline.
// ---------------------------------------------------------------------------

#[test]
fn f2_shrunken_cache_seek_equals_play_to() {
    let reference = open_fixture_with_cap(TimelineConfig::default().frame_cache_entries);
    assert_eq!(reference.frame_cache_entries(), 16, "default budget is 16");
    let hours: Vec<i64> = reference.hours().to_vec();
    assert!(hours.len() >= 2, "fixture must have a scrubbable span");

    for cap in [4usize, 1, 0] {
        let timeline = open_fixture_with_cap(cap);
        assert_eq!(timeline.frame_cache_entries(), cap, "opened budget");
        // Forward sweep, backward sweep, then a thrash pattern that
        // alternates ends (guaranteed evictions at every shrunken budget).
        for &h in &hours {
            assert_seek_eq_play(&timeline, h);
        }
        for &h in hours.iter().rev() {
            assert_seek_eq_play(&timeline, h);
        }
        for (i, (&lo, &hi)) in hours.iter().zip(hours.iter().rev()).enumerate() {
            let h = if i % 2 == 0 { lo } else { hi };
            assert_seek_eq_play(&timeline, h);
        }
        // Budget may trade speed only, never content: every frame matches
        // the default-budget timeline byte-for-byte at the raw layer.
        for &h in &hours {
            let a = timeline.seek(h).expect("seek shrunken");
            let b = reference.seek(h).expect("seek reference");
            assert_eq!(a.raw(), b.raw(), "cap {cap} content diverged @ {h}");
            assert_eq!(
                a.owner_by_state(),
                b.owner_by_state(),
                "cap {cap} owner view diverged @ {h}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Gates (a)+(a')+(c): allocator-sampled, run sequentially in ONE test so the
// samples never race each other's live timelines.
// ---------------------------------------------------------------------------

/// Random-scrub a synthetic timeline of `hour_count` hours at `cap`, then
/// sample net-live bytes with no frame Arcs held. Returns (live, index_bytes)
/// where index_bytes is the analytic O(hours) spine cost to subtract.
fn scrub_and_measure(hour_count: usize, seeks: usize, cap: usize) -> (usize, usize) {
    let source = Box::new(SyntheticSource::new(hour_count)) as Box<dyn ReplaySource>;
    let timeline = WorldTimeline::open_with_config(
        source,
        TimelineConfig {
            frame_cache_entries: cap,
        },
    )
    .expect("open synthetic timeline");
    let first = timeline.first_hour();
    let span = (timeline.last_hour() - first).max(1);

    let mut rng = Lcg::new(0xF2_C0FFEE ^ (hour_count as u64) ^ ((cap as u64) << 32));
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
        if i % 997 == 0 {
            assert_seek_eq_play(&timeline, h);
        }
    }
    assert_eq!(errors, 0, "synthetic scrub accumulated {errors} errors");
    let live = live_bytes();
    let index_bytes = INDEX_COPIES * hour_count * std::mem::size_of::<i64>();
    drop(timeline);
    (live, index_bytes)
}

#[test]
fn f2_budget_and_trend_gates() {
    let seeks = env_usize("BATTLE_PLAN_SOAK_SCRUB", 20_000);
    let hours_long = env_usize("BATTLE_PLAN_SOAK_HOURS_LONG", 26_280); // 3 years hourly
    let hours_short = 24usize;
    let sweeps = env_usize("BATTLE_PLAN_SOAK_SWEEPS", 2000);

    // ---- Gate (a): working set at a shrunken 4-entry budget is bounded by
    // the budget, not by timeline length. ----
    let (live_short4, index_short) = scrub_and_measure(hours_short, seeks, 4);
    let (live_long4, index_long) = scrub_and_measure(hours_long, seeks, 4);
    let cache_short4 = live_short4.saturating_sub(index_short);
    let cache_long4 = live_long4.saturating_sub(index_long);
    let extra = cache_long4.saturating_sub(cache_short4);

    eprintln!(
        "[soak_f2] gate(a): seeks={seeks} cap=4 short_hours={hours_short} \
         long_hours={hours_long} live_short={live_short4}B live_long={live_long4}B \
         index_short={index_short}B index_long={index_long}B \
         cache_short={cache_short4}B cache_long={cache_long4}B cache_extra={extra}B"
    );
    assert!(
        extra <= F2_LENGTH_CEILING_BYTES,
        "shrunken-budget working set grew {extra} B going from {hours_short} to \
         {hours_long} hours (cache_short={cache_short4}B cache_long={cache_long4}B); \
         the 4-entry budget must bound the working set independent of length"
    );

    // ---- Gate (a'): the knob is memory-aware — 16 entries retain measurably
    // more than 4 on the same long timeline (12 extra 16 KiB frames). ----
    let (live_long16, _) = scrub_and_measure(hours_long, seeks, 16);
    let cache_long16 = live_long16.saturating_sub(index_long);
    let knob_delta = cache_long16.saturating_sub(cache_long4);
    eprintln!(
        "[soak_f2] gate(a'): cap16_cache={cache_long16}B cap4_cache={cache_long4}B \
         knob_delta={knob_delta}B (floor {F2_KNOB_FLOOR_BYTES}B)"
    );
    assert!(
        knob_delta >= F2_KNOB_FLOOR_BYTES,
        "16-entry budget retained only {knob_delta} B more than the 4-entry budget \
         on the same long timeline (expected >= {F2_KNOB_FLOOR_BYTES} B from 12 extra \
         16 KiB frames); the budget knob does not appear to control retention"
    );

    // ---- Gate (c): no upward heap trend across >= 2000 sweeps at the
    // shrunken budget (same 1 MiB law as the F3 soak). ----
    let timeline = open_fixture_with_cap(4);
    let hours: Vec<i64> = timeline.hours().to_vec();
    for &h in &hours {
        assert_seek_eq_play(&timeline, h); // warmup to steady state
    }
    let baseline = live_bytes();
    let mut rng = Lcg::new(0x5EED_F2F2);
    let checkpoint_every = (sweeps / 4).max(1);
    let mut checkpoints: Vec<usize> = Vec::new();
    let mut errors = 0usize;
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
    assert_eq!(errors, 0, "sweep script accumulated {errors} errors");
    eprintln!(
        "[soak_f2] gate(c): sweeps={sweeps} cap=4 hours={} baseline={baseline}B \
         end={end}B growth={growth}B checkpoints={checkpoints:?}",
        hours.len()
    );
    assert!(
        growth <= LEAK_CEILING_BYTES,
        "live heap grew {growth} B over {sweeps} sweeps at the 4-entry budget \
         (baseline {baseline} B, end {end} B); exceeds {LEAK_CEILING_BYTES} B — \
         probable leak. checkpoints={checkpoints:?}"
    );
    if checkpoints.len() >= 2 {
        let first = checkpoints[0];
        let last = *checkpoints.last().unwrap();
        let trend = last.saturating_sub(first);
        assert!(
            trend <= LEAK_CEILING_BYTES,
            "live-heap checkpoints trend upward by {trend} B: {checkpoints:?}"
        );
    }
}
