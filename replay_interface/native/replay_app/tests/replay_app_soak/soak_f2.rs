

use replay_core::{open_recording, ReplaySource, TimelineConfig, WorldTimeline};
use crate::soak_support::{
    assert_seek_eq_play, env_usize, fixture_path, live_bytes, Lcg, SyntheticSource, INDEX_COPIES,
};


const F2_LENGTH_CEILING_BYTES: usize = 128 << 10;

const F2_KNOB_FLOOR_BYTES: usize = 96 << 10;

const LEAK_CEILING_BYTES: usize = 1 << 20;

const PAD_REPEATS: usize = 1024;

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
            ..TimelineConfig::default()
        },
    )
    .expect("open timeline")
}


fn gate_b_shrunken_cache_seek_equals_play_to() {
    let reference = open_fixture_with_cap(TimelineConfig::default().frame_cache_entries);
    assert_eq!(reference.frame_cache_entries(), 2, "default budget is 2");
    let hours: Vec<i64> = reference.hours().to_vec();
    assert!(hours.len() >= 2, "fixture must have a scrubbable span");

    for cap in [4usize, 1, 0] {
        let timeline = open_fixture_with_cap(cap);
        assert_eq!(timeline.frame_cache_entries(), cap, "opened budget");
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
        for &h in &hours {
            let a = timeline.seek(h).expect("seek shrunken");
            let b = reference.seek(h).expect("seek reference");
            assert_eq!(
                a.raw_for_consumer_tests(),
                b.raw_for_consumer_tests(),
                "cap {cap} content diverged @ {h}"
            );
            assert_eq!(
                a.owner_by_state(),
                b.owner_by_state(),
                "cap {cap} owner view diverged @ {h}"
            );
        }
    }
}


fn scrub_and_measure(hour_count: usize, seeks: usize, cap: usize) -> (usize, usize) {
    let source = Box::new(SyntheticSource::new(
        hour_count,
        "soak_f2_synthetic",
        PAD_REPEATS,
    )) as Box<dyn ReplaySource>;
    let timeline = WorldTimeline::open_with_config(
        source,
        TimelineConfig {
            frame_cache_entries: cap,
            ..TimelineConfig::default()
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
fn f2_correctness_budget_and_trend_gates() {
    gate_b_shrunken_cache_seek_equals_play_to();

    let seeks = env_usize("BATTLE_PLAN_SOAK_SCRUB", 20_000);
    let hours_long = env_usize("BATTLE_PLAN_SOAK_HOURS_LONG", 26_280);
    let hours_short = 24usize;
    let sweeps = env_usize("BATTLE_PLAN_SOAK_SWEEPS", 2000);

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

    let timeline = open_fixture_with_cap(4);
    let hours: Vec<i64> = timeline.hours().to_vec();
    for &h in &hours {
        assert_seek_eq_play(&timeline, h);
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
