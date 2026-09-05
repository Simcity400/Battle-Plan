
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use replay_core::{RawFrame, ReplaySource, SourceError, WorldFrame, WorldTimeline};
use serde_json::{json, Map, Value};


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

pub const INDEX_COPIES: usize = 2;

pub fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

// Frames are generated on demand (the source stores only the hour vector), so

pub struct SyntheticSource {
    game_id: String,
    hours: Vec<i64>,
    pad_repeats: usize,
}

impl SyntheticSource {
    pub fn new(hour_count: usize, game_id: &str, pad_repeats: usize) -> SyntheticSource {
        let base: i64 = 16_959_360;
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
        // The per-id country account: without it the viewer nulls `countries`
        // on every synthetic frame and this payload never reaches the typed
        // view the soak claims to exercise (Codex round 54).

        // The per-id country account: without it the viewer nulls `countries`
        // on every synthetic frame and this payload never reaches the typed
        // view the soak claims to exercise (Codex round 54).
        rest.insert(
            "tag_db_id_index".to_string(),
            json!({ "country_composition_version": 2 }),
        );
        rest.insert(
            "countries".to_string(),
            json!({ "GER": { "_r2_tag_db_id": 1, "politics": { "stability": (hour_index % 100) as f64 / 100.0 } } }),
        );
        if self.pad_repeats > 0 {
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
            rest: rest.into(),
        })
    }
}


pub fn assert_seek_eq_play(tl: &WorldTimeline, hour: i64) {
    let s: Arc<WorldFrame> = tl.seek(hour).expect("seek");
    let p: Arc<WorldFrame> = tl.play_to(hour).expect("play_to");
    assert_eq!(s.hour_index(), p.hour_index(), "hour mismatch @ {hour}");
    assert_eq!(
        s.raw_for_consumer_tests(),
        p.raw_for_consumer_tests(),
        "raw-frame mismatch @ {hour}"
    );
    assert_eq!(
        s.owner_by_state(),
        p.owner_by_state(),
        "owner view mismatch @ {hour}"
    );
    // The TYPED country view, not only the raw frame and the state owners. The
    // synthetic source emits a GER politics payload that the viewer nulls for a
    // fold with no per-id account, so the soak ran for its whole length over a
    // country that was never there and nothing here looked (Codex round 54).
    assert!(
        s.country_identity("GER").is_some(),
        "the synthetic country must be named @ {hour}"
    );
    assert!(
        matches!(s.topbar_scalars("GER"), Some(scalars) if scalars.stability.is_measured()),
        "and carry its measured stability @ {hour}"
    );
    assert_eq!(
        s.topbar_scalars("GER").map(|scalars| scalars.stability.clone()),
        p.topbar_scalars("GER").map(|scalars| scalars.stability.clone()),
        "typed country view mismatch @ {hour}"
    );
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(name)
}


pub struct Lcg {
    state: u64,
}
impl Lcg {
    pub fn new(seed: u64) -> Lcg {
        Lcg {
            state: seed | 1,
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
