use super::*;

#[test]
fn unavailable_passive_chain_returns_a_named_refusal_without_grid_bytes() {
    let memory = map_passive_carriers_live_lane::memory::SliceMemory::new(0x1_4000_0000);
    let mut tracker = CompactXmap07SnowMudTracker::default();
    let mut generations = 0u64;

    let read = read_compact_xmap07_snow_mud_from_memory(
        &memory,
        0,
        44_000_000,
        &mut tracker,
        &mut generations,
    );

    // kills: converting an unavailable anchor into an empty or zero-filled success payload.
    assert!(read.snapshot.is_refusal());
    assert!(read.snapshot.grid.is_empty());
    assert_eq!(read.snapshot.width, 0);
    assert_eq!(read.snapshot.height, 0);
    assert_eq!(
        read.refusal.as_ref().map(|refusal| refusal.reason),
        Some("map_anchor.game_root_unavailable")
    );
    assert_eq!(tracker.epoch_key, None);
    assert_eq!(tracker.generation, 0);
    assert_eq!(tracker.capture_ordinal, 0);
}

#[test]
fn tracker_ordinals_are_one_based_and_anchor_changes_open_a_fresh_generation() {
    let mut tracker = CompactXmap07SnowMudTracker::default();
    let mut generations = 0u64;

    let first = advance_compact_xmap07_snow_mud_tracker(
        &mut tracker,
        &mut generations,
        0x2_0003_0000,
        44_000_000,
    );
    let second = advance_compact_xmap07_snow_mud_tracker(
        &mut tracker,
        &mut generations,
        0x2_0003_0000,
        44_000_001,
    );
    let replacement = advance_compact_xmap07_snow_mud_tracker(
        &mut tracker,
        &mut generations,
        0x2_0004_0000,
        44_000_002,
    );

    // kills: making ordinals zero-based or carrying an ordinal across an anchor replacement.
    assert_eq!(first, (1, 1, 44_000_000));
    assert_eq!(second, (1, 2, 44_000_000));
    assert_eq!(replacement, (2, 1, 44_000_002));
}

#[test]
fn the_wire_baseline_survives_a_rejected_bracket_that_wipes_the_epoch_tracker() {
    // THE MEASUREMENT THIS EXISTS TO PROTECT. On the sealed `captures/oz_tip`
    // the epoch tracker was blanket-reset 86 times in 470 hours. If the wire
    // baseline were wiped alongside it, each of those hours would re-send the
    // whole 2.88 MB grid -- 248 MB against the 15.3 MB the whole delta stream
    // costs.
    //
    // kills: putting `xmap07_snow_mud_wire_baseline` on the blanket reset, or
    // dropping its reinstatement after `*trackers = fresh`.
    let mut trackers = CompactPerfGateTrackers::default();
    let grid = vec![7u8; 64];
    let sha = [0x33; 32];
    trackers
        .xmap07_snow_mud_wire_baseline
        .stage(grid.clone(), sha, true, 60_830_824);
    trackers.xmap07_snow_mud_wire_baseline.commit_pin();
    let mut generations = trackers.xmap07_snow_mud_generation_counter;
    advance_compact_xmap07_snow_mud_tracker(
        &mut trackers.xmap07_snow_mud,
        &mut generations,
        0x2_0003_0000,
        44_000_000,
    );
    trackers.xmap07_snow_mud_generation_counter = generations;
    assert_eq!(trackers.xmap07_snow_mud.capture_ordinal, 1);
    assert_eq!(trackers.xmap07_snow_mud.generation, 1);

    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);

    // The EPOCH tracker is wiped: that is the honest reset the carrier reports.
    assert_eq!(trackers.xmap07_snow_mud.epoch_key, None);
    assert_eq!(trackers.xmap07_snow_mud.capture_ordinal, 0);
    // The WIRE baseline is not: it names what the fold holds, and the fold's
    // copy did not move.
    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .committed_baseline()
            .map(|(bytes, digest)| (bytes.to_vec(), digest)),
        Some((grid, sha))
    );
}

#[test]
fn a_rejected_bracket_drops_this_frames_staged_baseline_but_not_the_committed_one() {
    // kills: committing the baseline at emission instead of at the frame seal.
    // The record this frame encoded is about to become a DOMAIN_GAP, so the
    // fold never receives it; a baseline advanced there would sit one grid
    // ahead of the fold and every later delta would name a baseline the fold
    // cannot produce.
    let mut trackers = CompactPerfGateTrackers::default();
    let committed = vec![1u8; 64];
    let committed_sha = [0x44; 32];
    trackers
        .xmap07_snow_mud_wire_baseline
        .stage(committed.clone(), committed_sha, true, 60_830_824);
    trackers.xmap07_snow_mud_wire_baseline.commit_pin();

    // This frame encodes a NEW grid and stages it...
    trackers
        .xmap07_snow_mud_wire_baseline
        .stage(vec![2u8; 64], [0x55; 32], false, 60_830_824 + 1);

    // ...and then its record is replaced with a gap.
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers, &mut Vec::new());

    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .committed_baseline()
            .map(|(bytes, digest)| (bytes.to_vec(), digest)),
        Some((committed, committed_sha))
    );
    // The cadence mark is rolled back too, so the next keyframe is measured
    // from the keyframe the fold actually holds and not from a gapped hour.
    //
    // IT ROLLS BACK TO THE COMMITTED KEYFRAME'S HOUR, NOT TO `None`. The gapped
    // frame staged a non-keyframe, which never moved the mark; what the undo
    // restores is the hour of the keyframe committed above, which is exactly
    // the keyframe the fold still holds. (Codex review round 2, finding 1: this
    // asserted `None` and would have failed the gate.) The `Some(None)` case --
    // a domain whose FIRST keyframe is rolled back -- is the test below.
    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .last_keyframe_hour_index,
        compact_contract::calendar::hour_index_from_raw(60_830_824)
    );
}

#[test]
fn a_rejected_bracket_on_a_cold_domain_rolls_the_cadence_mark_back_to_none() {
    // kills: staging the undo as an hour instead of as "whatever the frame was
    // entered with". `None` IS a real staged value here -- a domain that had
    // never keyframed before this frame must roll back to having no mark at
    // all, not to hour zero and not to the hour it just staged. Rolling back to
    // hour zero would leave `keyframe_due` measuring elapsed time from 1850,
    // so the cadence would never fire again; rolling back to the staged hour
    // would measure from a keyframe the fold never received.
    let mut trackers = CompactPerfGateTrackers::default();
    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .last_keyframe_hour_index,
        None,
        "the domain starts cold"
    );

    // The domain's FIRST keyframe, staged and not yet committed...
    trackers
        .xmap07_snow_mud_wire_baseline
        .stage(vec![3u8; 64], [0x66; 32], true, 60_830_824);
    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .last_keyframe_hour_index,
        compact_contract::calendar::hour_index_from_raw(60_830_824),
        "staging a keyframe moves the mark to this hour"
    );

    // ...and then its record is replaced with a gap.
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers, &mut Vec::new());

    assert!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .committed_baseline()
            .is_none(),
        "nothing was ever committed"
    );
    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .last_keyframe_hour_index,
        None,
        "and the mark goes back to having no keyframe at all"
    );
}

#[test]
fn a_surviving_frame_commits_its_staged_baseline_and_advances_the_cadence() {
    // kills: rolling the staged baseline back on every frame, which would pin
    // the producer to its first keyframe forever and make every later hour a
    // full-grid keyframe again.
    let mut trackers = CompactPerfGateTrackers::default();
    trackers
        .xmap07_snow_mud_wire_baseline
        .stage(vec![1u8; 64], [0x44; 32], true, 60_830_824);
    commit_delta_pin_transactions(&mut trackers, &mut Vec::new());
    let first_keyframe_hour = compact_contract::calendar::hour_index_from_raw(60_830_824);
    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .last_keyframe_hour_index,
        first_keyframe_hour
    );

    trackers
        .xmap07_snow_mud_wire_baseline
        .stage(vec![2u8; 64], [0x55; 32], false, 60_830_824 + 1);
    commit_delta_pin_transactions(&mut trackers, &mut Vec::new());

    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .committed_baseline()
            .map(|(bytes, digest)| (bytes.to_vec(), digest)),
        Some((vec![2u8; 64], [0x55; 32]))
    );
    // A DELTA DOES NOT MOVE THE MARK. The cadence is measured from the last
    // KEYFRAME, so an intervening delta hour leaves it exactly where it was.
    assert_eq!(
        trackers
            .xmap07_snow_mud_wire_baseline
            .last_keyframe_hour_index,
        first_keyframe_hour
    );
}

#[test]
fn the_cadence_forces_a_keyframe_and_a_cold_baseline_demands_one() {
    // kills: removing the keyframe cadence, which is the only bound on how long
    // a consumer that lost a record has to wait before d100 decodes again.
    let mut baseline = CompactXmap07SnowMudWireBaseline::default();
    assert!(
        baseline.keyframe_due(60_830_824),
        "a cold baseline must keyframe"
    );

    baseline.stage(vec![1u8; 64], [0x44; 32], true, 60_830_824);
    baseline.commit_pin();
    assert!(!baseline.keyframe_due(60_830_824 + 1));

    // THE CADENCE IS GAME HOURS, NOT EMISSIONS. Staging a hundred delta hours
    // without the clock moving must NOT make it due -- that is precisely the
    // record-counting defect this lane removed.
    for _ in 0..(XMAP07_SNOW_MUD_KEYFRAME_CADENCE_HOURS + 36) {
        baseline.stage(vec![1u8; 64], [0x44; 32], false, 60_830_824 + 1);
        baseline.commit_pin();
    }
    assert!(
        !baseline.keyframe_due(60_830_824 + 1),
        "emissions must not consume the cadence; only elapsed hours do"
    );

    let cadence = XMAP07_SNOW_MUD_KEYFRAME_CADENCE_HOURS as i32;
    assert!(
        !baseline.keyframe_due(60_830_824 + cadence - 1),
        "one hour short of the cadence is still inside it"
    );
    assert!(
        baseline.keyframe_due(60_830_824 + cadence),
        "the first grid at or after {XMAP07_SNOW_MUD_KEYFRAME_CADENCE_HOURS}          game hours must keyframe"
    );
    assert!(
        baseline.keyframe_due(60_830_824 + cadence + 6),
        "a grid arriving after a quiet gap must keyframe"
    );
    assert!(
        baseline.keyframe_due(60_830_824 - 10),
        "an hour that regresses below the keyframe must keyframe"
    );
}

#[test]
fn every_reset_opens_a_genuinely_fresh_generation_rather_than_generation_one() {
    // THE DEFECT THIS PINS. On the sealed `captures/oz_tip` the d100 tracker was
    // blanket-reset 86 times in 470 hours, and every single record still carried
    // `anchor_generation = 1` -- because the reset zeroed the counter and
    // `saturating_add(1).max(1)` then resolved to 1 again. The world fold spells
    // that field as `hook_epoch: "anchor-N"`, so the capture declared ONE epoch
    // while `capture_frontier_ordinal` restarted inside it 86 times: 428
    // duplicate ordinals and 85 backwards steps.
    //
    // kills: moving the generation counter back inside
    // `CompactXmap07SnowMudTracker`, where the blanket reset can reach it.
    let mut trackers = CompactPerfGateTrackers::default();
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE].into_iter().collect();

    let mut generations = Vec::new();
    for hour in 0..4i32 {
        let mut counter = trackers.xmap07_snow_mud_generation_counter;
        let (generation, ordinal, _) = advance_compact_xmap07_snow_mud_tracker(
            &mut trackers.xmap07_snow_mud,
            &mut counter,
            0x2_0003_0000,
            44_000_000 + hour,
        );
        trackers.xmap07_snow_mud_generation_counter = counter;
        assert_eq!(ordinal, 1, "each reset restarts the ordinal, as it should");
        generations.push(generation);
        reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    }

    // Four resets, four DISTINCT generations -- not four ones.
    assert_eq!(generations, vec![1, 2, 3, 4]);
}

#[test]
fn staging_a_baseline_twice_still_rolls_back_to_the_frames_entry() {
    // kills: recording the cadence undo on every `stage` call instead of once
    // per frame. A second stage would overwrite the undo with the first call's
    // already-advanced value, and the roll-back would leave the cadence measured
    // from a keyframe the fold never received.
    let mut baseline = CompactXmap07SnowMudWireBaseline::default();
    baseline.stage(vec![1u8; 64], [0x44; 32], true, 60_830_824);
    baseline.commit_pin();
    baseline.stage(vec![2u8; 64], [0x55; 32], false, 60_830_824 + 1);
    baseline.commit_pin();
    let entry_mark = baseline.last_keyframe_hour_index;
    assert_eq!(
        entry_mark,
        compact_contract::calendar::hour_index_from_raw(60_830_824)
    );

    // Two stages inside ONE frame -- the second a KEYFRAME, so the mark really
    // does move and the roll-back has something to undo -- then a roll-back.
    baseline.stage(vec![3u8; 64], [0x66; 32], false, 60_830_824 + 2);
    baseline.stage(vec![4u8; 64], [0x77; 32], true, 60_830_824 + 2);
    baseline.roll_back_uncommitted_pin();

    assert_eq!(
        baseline.last_keyframe_hour_index, entry_mark,
        "the frame was entered on the first keyframe's hour, so the roll-back          must land there"
    );
    assert_eq!(
        baseline
            .committed_baseline()
            .map(|(bytes, digest)| (bytes.to_vec(), digest)),
        Some((vec![2u8; 64], [0x55; 32]))
    );
}

#[test]
fn each_wire_body_kind_unit_has_exactly_one_writer() {
    // THE DEFECT THIS PINS, found by the live capture `captures/pb_perf`: the
    // body-kind units were emitted BOTH by the producer (on d100's perf record)
    // and by `domain_data_authority` (on the data record). Both families land in
    // the same `data_units_total` bucket, so the sealed manifest reported 722
    // body-kind units for 361 records -- 656 no-change + 54 delta + 12 keyframe,
    // every one counted twice -- which reads as a producer emitting two bodies
    // per hour. `grid_changed` + `grid_unchanged` summed to exactly 361, which is
    // how the doubling was spotted.
    //
    // kills: re-adding a decoder-side copy of any `xmap07_snow_mud_wire_*` unit.
    let authority = include_str!("../../../recorder_host/src/domain_data_authority.rs");
    for unit in [
        "xmap07_snow_mud_wire_keyframe",
        "xmap07_snow_mud_wire_delta",
        "xmap07_snow_mud_wire_no_change",
    ] {
        assert!(
            !authority.contains(&format!("\"{unit}\"")),
            "{unit} must have exactly one writer (the producer);              domain_data_authority.rs must not emit it too"
        );
        assert!(
            super::tests::collector_source_lf().contains(&format!("\"{unit}\"")),
            "{unit} must still be emitted by the producer"
        );
    }
}

/// One clean d100 read whose grid is `grid`, for the finalize tests below.
fn xsm2_read(grid: Vec<u8>, hour: i32, ordinal: u64) -> CompactXmap07SnowMudRead {
    use sha2::{Digest, Sha256};
    let grid_sha256: [u8; 32] = Sha256::digest(&grid).into();
    CompactXmap07SnowMudRead {
        snapshot: compact_contract::xmap07_snow_mud::Xmap07SnowMudSnapshot {
            read_time_us: 100,
            status_flags: 0,
            source_sequence: 0,
            anchor_generation: 1,
            capture_ordinal: ordinal,
            first_capture_hour_raw: hour,
            game_hour_raw: hour,
            source_present: true,
            width: 32,
            height: 32,
            fade_f32_bits: 0.25f32.to_bits(),
            grid_sha256,
            grid,
        },
        payload: Vec::new(),
        changed_rows: 0,
        emitted_grid_bytes: 0,
        diagnostic_units: Vec::new(),
        refusal: None,
    }
}

fn unit_names(read: &CompactXmap07SnowMudRead) -> Vec<&'static str> {
    read.diagnostic_units.iter().map(|u| u.name).collect()
}

#[test]
fn a_cadence_keyframe_of_an_unchanged_grid_is_not_reported_as_a_change() {
    // THE DEFECT THIS PINS (Codex review round 1, finding 2). The change signal
    // used to be "every body except NoChange", so a CADENCE keyframe -- which
    // re-sends a grid precisely because 64 hours have passed, not because
    // anything moved -- was reported as changed. On the live capture
    // `captures/pb_perf` 3 of the 6 keyframes carried the same digest as the
    // hour before and all six incremented grid_changed, changed_rows and the
    // whole-grid changed_unit_count.
    //
    // kills: deriving grid_changed from the wire body instead of the digest.
    let grid = vec![9u8; 32 * 32 * 4];
    let mut baseline = CompactXmap07SnowMudWireBaseline::default();
    let mut ledger = compact_contract::domain_outcome::DomainOutcomeLedger::default();

    // THE HOURS HERE ARE IN `raw_date_in_range`. They used to be 44_000_000,
    // which is BELOW the calendar's 1850 floor, so `hour_index_from_raw`
    // returned None and the cadence fell through to its fail-safe -- the test
    // would have passed without ever exercising the elapsed-hour path.
    let base_hour = 60_830_824i32;

    // Hour 1: first observation. No baseline, so this is a keyframe AND a change.
    let mut first = xsm2_read(grid.clone(), base_hour, 1);
    finalize_compact_xmap07_snow_mud_read(&mut first, &mut baseline, &mut ledger);
    baseline.commit_pin();
    assert_eq!(first.changed_rows, 1);
    assert!(unit_names(&first).contains(&"xmap07_snow_mud_grid_changed"));
    assert!(unit_names(&first).contains(&"xmap07_snow_mud_wire_keyframe"));

    // The cadence is due because 100 GAME HOURS have passed, and for no other
    // reason. This used to stage 64 unchanged grids to drive a record counter;
    // that loop is gone with the counter.
    assert!(!baseline.keyframe_due(base_hour + 1));
    assert!(baseline.keyframe_due(base_hour + 100));

    // Hour N: the cadence forces a keyframe, but the grid is identical.
    let mut cadence = xsm2_read(grid.clone(), base_hour + 100, 2);
    finalize_compact_xmap07_snow_mud_read(&mut cadence, &mut baseline, &mut ledger);
    let names = unit_names(&cadence);
    assert!(
        names.contains(&"xmap07_snow_mud_wire_keyframe"),
        "the cadence still forces a keyframe body: {names:?}"
    );
    assert!(
        names.contains(&"xmap07_snow_mud_grid_unchanged"),
        "but the GRID did not change: {names:?}"
    );
    assert_eq!(
        cadence.changed_rows, 0,
        "a re-sent identical grid is not a changed row"
    );
}

#[test]
fn an_encode_failure_publishes_no_diagnostics_for_the_body_it_failed_to_encode() {
    // kills: pushing the body-kind and change units before encoding (Codex
    // review round 1, finding 3). An unchanged grid selects NoChange; if the
    // encode then fails, the hour is emitted as a REFUSAL keyframe carrying no
    // grid, and units saying wire_no_change / grid_unchanged would describe a
    // record that was never written.
    let grid = vec![9u8; 32 * 32 * 4];
    let mut baseline = CompactXmap07SnowMudWireBaseline::default();
    let mut ledger = compact_contract::domain_outcome::DomainOutcomeLedger::default();

    let mut first = xsm2_read(grid.clone(), 44_000_000, 1);
    finalize_compact_xmap07_snow_mud_read(&mut first, &mut baseline, &mut ledger);
    baseline.commit_pin();

    // Same grid (so NoChange is selected) but a fade the encoder must reject.
    let mut broken = xsm2_read(grid, 44_000_001, 2);
    broken.snapshot.fade_f32_bits = f32::NAN.to_bits();
    finalize_compact_xmap07_snow_mud_read(&mut broken, &mut baseline, &mut ledger);

    let names = unit_names(&broken);
    assert!(
        names.contains(&"xmap07_snow_mud_payload_encode_refused"),
        "{names:?}"
    );
    assert!(
        !names.contains(&"xmap07_snow_mud_wire_no_change"),
        "the no-change body was never emitted: {names:?}"
    );
    assert!(
        !names.contains(&"xmap07_snow_mud_grid_unchanged"),
        "a refusal observed no grid, so it is neither changed nor unchanged: {names:?}"
    );
    assert!(names.contains(&"xmap07_snow_mud_grid_unobserved"), "{names:?}");
    assert!(broken.snapshot.is_refusal());
    assert_eq!(broken.changed_rows, 0);
    assert_eq!(broken.emitted_grid_bytes, 0);

    // AND THE BASELINE DID NOT MOVE: a failed encode put no grid on the wire.
    baseline.commit_pin();
    assert_eq!(
        baseline.committed_baseline().map(|(_, sha)| sha),
        Some(first.snapshot.grid_sha256)
    );
}
