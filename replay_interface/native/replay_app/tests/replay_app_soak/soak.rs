

use replay_core::{open_recording, ReplaySource, WorldTimeline};
use crate::soak_support::{
    assert_seek_eq_play, env_usize, fixture_path, live_bytes, Lcg, SyntheticSource, INDEX_COPIES,
};


const LEAK_CEILING_BYTES: usize = 1 << 20;

const F2_CEILING_BYTES: usize = 64 << 10;


fn scrub_and_measure(hour_count: usize, seeks: usize) -> (usize, usize) {
    let source =
        Box::new(SyntheticSource::new(hour_count, "soak_synthetic", 0)) as Box<dyn ReplaySource>;
    let timeline = WorldTimeline::open(source).expect("open synthetic timeline");
    let first = timeline.first_hour();
    let span = (timeline.last_hour() - first).max(1);

    let mut rng = Lcg::new(0xC0FFEE ^ hour_count as u64);
    let mut errors = 0usize;
    for i in 0..seeks {
        let h = first + (rng.next_u64() as i64).rem_euclid(span + 1);
        match (timeline.seek(h), timeline.play_to(h)) {
            (Ok(s), Ok(p)) => {
                if s.hour_index() != p.hour_index() || s.raw_for_consumer_tests() != p.raw_for_consumer_tests() {
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
fn soak_leakfree_invariant_and_f2_ceiling() {
    let _measurement = crate::soak_support::MEASUREMENT_LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sweeps = env_usize("BATTLE_PLAN_SOAK_SWEEPS", 2000);

    let path = fixture_path("fx_owner_controller_flip.battleplan");
    assert!(
        path.exists(),
        "missing committed fixture: {}",
        path.display()
    );

    let source = open_recording(&path).expect("open committed fixture");
    let timeline = WorldTimeline::open(source).expect("open timeline");
    let hours: Vec<i64> = timeline.hours().to_vec();
    assert!(hours.len() >= 2, "fixture must have a scrubbable span");

    for &h in &hours {
        assert_seek_eq_play(&timeline, h);
    }
    let baseline = live_bytes();

    let mut errors: usize = 0;
    let mut rng = Lcg::new(0x5EED_1234);
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

    // Release the fixture timeline before the F2 measurements. The F2 gate is
    drop(timeline);
    drop(hours);

    let seeks = env_usize("BATTLE_PLAN_SOAK_SCRUB", 20_000);
    let hours_long = env_usize("BATTLE_PLAN_SOAK_HOURS_LONG", 26_280);
    let hours_short = 24usize;

    let (live_short, index_short) = scrub_and_measure(hours_short, seeks);
    let (live_long, index_long) = scrub_and_measure(hours_long, seeks);

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
