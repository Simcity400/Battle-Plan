//! `domain41_partial_emit_tests`: the losslessness gate for `DeltaPartial`.
//!
//! D41 is the wire's second-largest domain and its delta path has never once
//! fired. Measured over `runs/live_unlock/t19/collector_stream.bin` (4.78 GB,
//! 691 game-hours, 67 domains): 474 D41 `DOMAIN_DATA` records, mean 1.46 MB,
//! 692 MB total; `PARTIAL_DATA` set on 474 of 474 and `DELTA_DATA` on none. The
//! cause was a single country whose FACILITY walk was refused
//! (`invalid_facility_countries = 1` with
//! `invalid_facility_candidate_vtable_read_fail = 1`, byte-identical in every
//! hour — NOT the project pool, whose counters are all 0 in every record),
//! which is enough to put `PARTIAL_DATA` — a member of
//! `COMPACT_STATUS_FAILURE_MASK` — on every record and send
//! `compact_projects_apply_emit_mode` down its failure return before any diff
//! is attempted. A long-window LZMA measurement over the same records puts
//! D41's actual information content at about 0.26 KB per game-hour.
//!
//! That root is repaired: the facility walk is now bounded by the engine's own
//! element count (`PROJECT_PROGRAM_STATUS_OFF_PROGRAMS_COUNT`) instead of
//! guessing the end of the array, so a stale slot past the end can no longer
//! refuse a country. The `fixture` tests for that live beside the other
//! facility-walk tests in `tests.rs`. The `DeltaPartial` gate below is
//! unaffected and still governs the mode, which remains off — with a clean
//! read the ordinary `Delta` path carries the domain, as it does in t21
//! (157 of 256 records delta, keyframe 1,458,417 B vs delta 420 B).
//!
//! These tests pin what `DeltaPartial` must preserve to be enabled: the rows an
//! hour decodes to, exactly, for every hour. Reconstruction here mirrors the
//! shipped d48/d53 fold contract — each hour is rebuilt from the PINNED delta
//! baseline, never from the previous hour's reconstruction, because a partial
//! read never advances that baseline.

use super::*;

/// Rebuilt row set for one hour, in the shape the fold reconstructs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ReconstructedHour {
    facilities: Vec<(u32, u32, u32)>,
    scientists: Vec<(u32, Option<u32>)>,
    owner_progress: Vec<(u32, u32, u32, u32)>,
}

fn facility(id: u32, province: u32, unread: u32) -> ProjectFacilityRow {
    ProjectFacilityRow {
        id,
        province,
        building: None,
        active_project_handle: 0,
        active: false,
        scientist_char_id: None,
        unread_prototype_rewards: unread,
        deleted: false,
    }
}

fn scientist(char_id: u32, trait_count: Option<u32>) -> ProjectScientistRow {
    ProjectScientistRow {
        char_id,
        name_token: None,
        display_name: None,
        skills: BTreeMap::new(),
        trait_count,
        deleted: false,
    }
}

fn owner_progress(tag: u32, local_id: u32, phase: u32, iterations: u32) -> ProjectOwnerProgressRow {
    ProjectOwnerProgressRow {
        tag_db_id: tag,
        local_id,
        static_def_handle: 7,
        current_state_offset: 0,
        current_state_enum: 1,
        prototype_state_enum: 0,
        simple_state_1_enum: 0,
        simple_state_2_enum: 0,
        simple_state_3_enum: 0,
        prototype_iterations: iterations,
        phase_progress_raw: phase,
        phase_target_raw: 1_000_000,
        state_flags: 0,
        deleted: false,
    }
}

fn read_with(
    facilities: Vec<ProjectFacilityRow>,
    scientists: Vec<ProjectScientistRow>,
    owner_progress_rows: Vec<ProjectOwnerProgressRow>,
    status_flags: u32,
) -> CompactProjectsRead {
    let observed_facility_rows = facilities.len() as u32;
    let observed_scientist_rows = scientists.len() as u32;
    let observed_project_rows = owner_progress_rows.len() as u32;
    let changed_rows = observed_facility_rows.saturating_add(observed_scientist_rows);
    CompactProjectsRead {
        facilities,
        projects: Vec::new(),
        scientists,
        project_states: Vec::new(),
        breakthrough: Vec::new(),
        d18_authority: None,
        read_time_us: 1_000,
        changed_rows,
        status_flags,
        observed_facility_rows,
        observed_project_rows,
        observed_scientist_rows,
        observed_project_state_rows: observed_project_rows,
        observed_breakthrough_rows: 0,
        scan_ms: 0,
        bridge_rows_supplied: 0,
        source_kind: 1,
        // t19's exact partial cause: one country's project pool is unreadable,
        // in every hour, and nothing else is wrong with the walk.
        invalid_program_status_countries: 0,
        invalid_facility_countries: 0,
        invalid_project_pool_countries: if status_flags & COMPACT_STATUS_PARTIAL_DATA != 0 {
            1
        } else {
            0
        },
        neutral_program_status_countries: 0,
        neutral_facility_countries: 0,
        neutral_project_pool_countries: 0,
        invalid_breakthrough_countries: 0,
        breakthrough_specializations_resolved: true,
        root_reasons: ProjectRootReasonCounters::default(),
        lifecycle: Vec::new(),
        lifecycle_countries: 0,
        lifecycle_aborted_countries: 0,
        owner_progress: owner_progress_rows,
        owner_progress_countries: 1,
        owner_progress_aborted_countries: 0,
        reward_history: Vec::new(),
        reward_history_countries: 0,
        reward_history_aborted_countries: 0,
    }
}

/// The rows a FULL payload decodes to: exactly what the read carried.
fn reconstruct_full(read: &CompactProjectsRead) -> ReconstructedHour {
    ReconstructedHour {
        facilities: read
            .facilities
            .iter()
            .filter(|row| !row.deleted)
            .map(|row| (row.id, row.province, row.unread_prototype_rewards))
            .collect(),
        scientists: read
            .scientists
            .iter()
            .filter(|row| !row.deleted)
            .map(|row| (row.char_id, row.trait_count))
            .collect(),
        owner_progress: read
            .owner_progress
            .iter()
            .filter(|row| !row.deleted)
            .map(|row| {
                (
                    row.tag_db_id,
                    row.local_id,
                    row.phase_progress_raw,
                    row.prototype_iterations,
                )
            })
            .collect(),
    }
}

/// The rows a DELTA payload decodes to, applied the way the fold applies one:
/// seed from the pinned delta baseline, overwrite carried rows, drop tombstoned
/// keys. `baseline` is the emission that seeded the chain, never the previous
/// hour's reconstruction — a partial read does not advance the pin.
fn reconstruct_delta(
    baseline: &CompactProjectsRead,
    emitted: &CompactProjectsRead,
) -> ReconstructedHour {
    let mut facilities: BTreeMap<u32, ProjectFacilityRow> = baseline
        .facilities
        .iter()
        .map(|row| (row.id, row.clone()))
        .collect();
    let mut scientists: BTreeMap<u32, ProjectScientistRow> = baseline
        .scientists
        .iter()
        .map(|row| (row.char_id, row.clone()))
        .collect();
    let mut owner_progress: BTreeMap<(u32, u32), ProjectOwnerProgressRow> = baseline
        .owner_progress
        .iter()
        .map(|row| ((row.tag_db_id, row.local_id), row.clone()))
        .collect();

    for row in &emitted.facilities {
        if row.deleted {
            facilities.remove(&row.id);
        } else {
            facilities.insert(row.id, row.clone());
        }
    }
    for row in &emitted.scientists {
        if row.deleted {
            scientists.remove(&row.char_id);
        } else {
            scientists.insert(row.char_id, row.clone());
        }
    }
    for row in &emitted.owner_progress {
        let key = (row.tag_db_id, row.local_id);
        if row.deleted {
            owner_progress.remove(&key);
        } else {
            owner_progress.insert(key, row.clone());
        }
    }

    ReconstructedHour {
        facilities: facilities
            .values()
            .map(|row| (row.id, row.province, row.unread_prototype_rewards))
            .collect(),
        scientists: scientists
            .values()
            .map(|row| (row.char_id, row.trait_count))
            .collect(),
        owner_progress: owner_progress
            .values()
            .map(|row| {
                (
                    row.tag_db_id,
                    row.local_id,
                    row.phase_progress_raw,
                    row.prototype_iterations,
                )
            })
            .collect(),
    }
}

/// Sort a full-emission reconstruction into the delta side's key order, so the
/// comparison is about row CONTENT and not about the order a `Vec` happened to
/// arrive in.
fn sorted(mut hour: ReconstructedHour) -> ReconstructedHour {
    hour.facilities.sort_unstable();
    hour.scientists.sort_unstable();
    hour.owner_progress.sort_unstable();
    hour
}

/// t19's shape, small enough to run: one hour's read, with `hour` driving the
/// handful of rows that actually move.
fn hour_read(hour: u32, partial: bool, drop_tail_owner_rows: usize) -> CompactProjectsRead {
    hour_read_scaled(hour, partial, drop_tail_owner_rows, 400)
}

fn hour_read_scaled(
    hour: u32,
    partial: bool,
    drop_tail_owner_rows: usize,
    total_owner_rows: usize,
) -> CompactProjectsRead {
    let facilities = (0..8u32)
        .map(|id| facility(id + 1, 100 + id, hour % 3))
        .collect();
    let scientists = (0..12u32)
        .map(|id| scientist(id + 1, Some((id + hour) % 5)))
        .collect();
    let live_owner_rows = total_owner_rows.saturating_sub(drop_tail_owner_rows);
    let owner_rows = (0..live_owner_rows)
        .map(|index| {
            let local_id = index as u32 + 1;
            // Realistic change rate: a small, moving subset advances each hour
            // while everything else is byte-identical to the hour before.
            let phase = if local_id % 97 == hour % 97 {
                hour * 1_000
            } else {
                local_id * 13
            };
            owner_progress(42, local_id, phase, local_id % 4)
        })
        .collect();
    let status = if partial {
        COMPACT_STATUS_PARTIAL_DATA
    } else {
        0
    };
    read_with(facilities, scientists, owner_rows, status)
}

fn encoded_len(read: &CompactProjectsRead) -> usize {
    compact_projects_data_payload(read, 0).len()
}

/// THE GATE. Every partial hour after a clean seed, encoded under
/// `DeltaPartial`, must decode to exactly the rows full emission carried. If
/// this fails the mode is lossy and must not be enabled.
///
/// Hour 0 is CLEAN because the seed is not part of the extension: a
/// `PARTIAL_DATA` record is discarded whole by `world_fold` (D41 is absent from
/// `COMPACT_EXPECTED_PARTIAL_TOLERANCES`), so a pin taken from one is a baseline
/// the consumer has not got. `a_partial_first_record_never_seeds_the_chain` is
/// that half of the contract; this one is the reconstruction half.
///
/// Production change that makes this fail: dropping the tombstone sweep for
/// partial reads, or advancing the pinned baseline from a partial read. Either
/// one leaves the reconstruction holding rows the hour did not observe.
#[test]
fn delta_partial_reconstructs_every_partial_hour_after_a_clean_seed() {
    let mut full_tracker = CompactProjectsTracker::default();
    let mut delta_tracker = CompactProjectsTracker::default();
    let mut baseline: Option<CompactProjectsRead> = None;

    for hour in 0..24u32 {
        // Hours 6..9 lose a growing tail of owner rows: the partial read is not
        // merely stale, it returns FEWER keys, which is the case a tombstone
        // sweep exists for.
        let dropped = if (6..10).contains(&hour) {
            (hour as usize - 5) * 7
        } else {
            0
        };
        let partial = hour != 0;
        // The same read, built twice: `CompactProjectsRead` is a production
        // type and is deliberately not `Clone`, so the two modes are driven
        // from two identical constructions rather than from a test-only derive.
        let full = compact_projects_apply_emit_mode(
            hour_read(hour, partial, dropped),
            D41EmitMode::Full,
            &mut full_tracker,
        );
        let emitted = compact_projects_apply_emit_mode(
            hour_read(hour, partial, dropped),
            D41EmitMode::DeltaPartial,
            &mut delta_tracker,
        );

        let expected = sorted(reconstruct_full(&full));
        match &baseline {
            None => {
                // The seeding hour is emitted whole and pins the chain.
                assert_eq!(
                    emitted.status_flags & COMPACT_STATUS_DELTA_DATA,
                    0,
                    "hour {hour} seeded the chain and must not be a delta"
                );
                assert_eq!(sorted(reconstruct_full(&emitted)), expected);
                baseline = Some(emitted);
            }
            Some(pinned) => {
                assert_ne!(
                    emitted.status_flags & COMPACT_STATUS_DELTA_DATA,
                    0,
                    "hour {hour} must be a delta once the chain is pinned"
                );
                assert_eq!(
                    sorted(reconstruct_delta(pinned, &emitted)),
                    expected,
                    "hour {hour} did not reconstruct the rows full emission carried"
                );
            }
        }
    }
}

/// `Delta` must be untouched. The mode ships OFF and the default path has to
/// stay byte-for-byte what it was: a partial read emits its whole payload, sets
/// no `DELTA_DATA`, and moves no baseline.
///
/// Production change that makes this fail: letting `DeltaPartial`'s branch fire
/// under the default mode.
#[test]
fn default_delta_mode_still_refuses_to_patch_a_partial_read() {
    let mut tracker = CompactProjectsTracker::default();
    for hour in 0..4u32 {
        let source_len = encoded_len(&hour_read(hour, true, 0));
        let emitted =
            compact_projects_apply_emit_mode(hour_read(hour, true, 0), D41EmitMode::Delta, &mut tracker);
        assert_eq!(
            emitted.status_flags & COMPACT_STATUS_DELTA_DATA,
            0,
            "hour {hour}: default Delta must not patch a partial read"
        );
        assert_eq!(encoded_len(&emitted), source_len);
        assert!(
            !tracker.baseline_seeded,
            "hour {hour}: a refused partial read must not seed the baseline"
        );
    }
}

/// A `PARTIAL_DATA` first record must NOT seed the chain, and the clean hours
/// after it must still be foldable.
///
/// This is the defect the seed guard exists for, and its cost is not the partial
/// hours. D41 is absent from `COMPACT_EXPECTED_PARTIAL_TOLERANCES`, so
/// `world_fold::handle_compact_failure` discards every `PARTIAL_DATA` D41 record
/// whole and never records a delta baseline for the domain. Seeding from one
/// pins the producer to a state the fold has not got; every later clean hour
/// then ships `DELTA_DATA` into `reject_delta_before_baseline` and is marked
/// unknown, and nothing recovers, because `baseline_seeded` stays true and no
/// keyframe is ever emitted again. The CLEAN hours are what is lost.
///
/// The test models the fold's two rules directly rather than describing them:
/// a record with a failure bit is discarded, and a `DELTA_DATA` record before a
/// baseline is discarded.
///
/// MUTATION: revert the seed rule in `compact_projects_apply_emit_mode` —
/// delete the `if baseline && !advance_baseline { return read; }` guard AND
/// restore `if baseline || advance_baseline { *tracker = current; }` — and this
/// fails, observed verbatim:
///   "a record the fold discards must not seed the producer's pin"
/// with `mixed_clean_and_partial_coverage_reconstructs_across_pin_moves` failing
/// beside it:
///   "assertion `left == right` failed: hour 1: the chain starts at the first
///    clean read and only records after it patch / left: true / right: false"
///
/// Deleting the guard ALONE is not observable: the pin condition below it is
/// the load-bearing half, and the guard is the early exit that states the rule
/// at the point of decision and skips a keyed diff whose output is discarded.
#[test]
fn a_partial_first_record_never_seeds_the_chain() {
    let mut tracker = CompactProjectsTracker::default();

    // Hour 0: partial. Emitted whole, pins nothing.
    let first = compact_projects_apply_emit_mode(
        hour_read(0, true, 0),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_eq!(
        first.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "a discarded record must not be dressed as a patch"
    );
    assert!(
        !tracker.baseline_seeded,
        "a record the fold discards must not seed the producer's pin"
    );

    // Hour 1: partial again. Same answer — the chain has still not started.
    let second = compact_projects_apply_emit_mode(
        hour_read(1, true, 0),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_eq!(second.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert!(!tracker.baseline_seeded);

    // Hour 2: clean. This is the first record the fold applies, so it is the
    // keyframe — and it must be a whole payload, or the fold rejects it as
    // `delta_before_baseline` and the hour is lost.
    let third = compact_projects_apply_emit_mode(
        hour_read(2, false, 0),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    let mut fold_has_baseline = false;
    for (label, record) in [("hour 0", &first), ("hour 1", &second), ("hour 2", &third)] {
        let discarded_by_failure =
            record.status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK != 0;
        let is_delta = record.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        if discarded_by_failure {
            continue;
        }
        assert!(
            !(is_delta && !fold_has_baseline),
            "the fold never got a baseline, so {label}'s patch is \
             delta_before_baseline and the clean hour is lost"
        );
        fold_has_baseline = true;
    }
    assert!(
        fold_has_baseline,
        "the clean hour must have reached the fold"
    );
    assert!(tracker.baseline_seeded, "the clean read seeds the chain");

    // And the chain is live from there: the next read patches.
    let fourth = compact_projects_apply_emit_mode(
        hour_read(3, true, 0),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_ne!(
        fourth.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "once a clean read has seeded, later reads patch"
    );
}

/// An ALL-EMPTY first read is still a baseline, and it seeds exactly once.
///
/// The sentinel has to be `baseline_seeded`, not emptiness of the row maps,
/// because an empty row set is a legitimate thing to have pinned and emptiness
/// cannot tell "nothing pinned yet" from "an empty set is pinned". With
/// emptiness as the sentinel every read taken while the domain has no rows
/// re-seeds and re-emits a keyframe, and the producer's seed count drifts out
/// of lockstep with the fold's `delta_baselines_seen`.
///
/// MUTATION: restore `let baseline = tracker.is_empty();` (and the
/// `CompactProjectsTracker::is_empty` helper) and this fails on hour 1 —
/// observed verbatim:
///   "an empty baseline is still a baseline: hour 1 must patch, not re-seed"
#[test]
fn an_all_empty_first_read_seeds_the_baseline_exactly_once() {
    let mut tracker = CompactProjectsTracker::default();
    let first = compact_projects_apply_emit_mode(
        read_with(Vec::new(), Vec::new(), Vec::new(), 0),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_eq!(
        first.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "hour 0 is the keyframe"
    );
    assert!(tracker.baseline_seeded, "hour 0 seeds the pin");

    for hour in 1..4u32 {
        let emitted = compact_projects_apply_emit_mode(
            read_with(Vec::new(), Vec::new(), Vec::new(), 0),
            D41EmitMode::Delta,
            &mut tracker,
        );
        assert_ne!(
            emitted.status_flags & COMPACT_STATUS_DELTA_DATA,
            0,
            "an empty baseline is still a baseline: hour {hour} must patch, not re-seed"
        );
    }
}

/// A domain that empties out MID-CAPTURE must not re-seed either.
///
/// This is the expensive half of the same defect and it is not a first-frame
/// concern: once every row is tombstoned the tracker's maps are empty again, so
/// an emptiness sentinel reads "no baseline" and the next non-empty read emits
/// a whole keyframe — 1.46 MB on d41's t19 measurements — instead of a patch.
///
/// MUTATION: restore `let baseline = tracker.is_empty();` and this fails on the
/// third read — observed verbatim:
///   "a domain that emptied out has a pinned baseline: this must patch"
#[test]
fn a_domain_that_empties_out_mid_capture_does_not_re_seed() {
    let mut tracker = CompactProjectsTracker::default();
    let seeded = compact_projects_apply_emit_mode(
        read_with(vec![facility(1, 10, 0)], Vec::new(), Vec::new(), 0),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_eq!(seeded.status_flags & COMPACT_STATUS_DELTA_DATA, 0);

    // Every row goes away: the patch tombstones it and the tracker's maps are
    // empty once more.
    let emptied = compact_projects_apply_emit_mode(
        read_with(Vec::new(), Vec::new(), Vec::new(), 0),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_ne!(emptied.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(emptied.facilities.len(), 1, "the removal ships as a tombstone");
    assert!(emptied.facilities[0].deleted);
    assert!(tracker.facilities.is_empty(), "the pin is now an empty set");
    assert!(tracker.baseline_seeded, "but it is still a pin");

    let returned = compact_projects_apply_emit_mode(
        read_with(vec![facility(1, 10, 0)], Vec::new(), Vec::new(), 0),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_ne!(
        returned.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "a domain that emptied out has a pinned baseline: this must patch"
    );
}

/// A clean read still advances the pin, and the deletions it proves still ship
/// as tombstones. This is the property that would silently rot if `DeltaPartial`
/// were implemented by suppressing tombstones instead of pinning the baseline.
#[test]
fn a_clean_read_advances_the_pin_and_still_tombstones_what_it_proves_gone() {
    let mut tracker = CompactProjectsTracker::default();
    let seed = compact_projects_apply_emit_mode(
        hour_read(0, false, 0),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_eq!(seed.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    let pinned_after_seed = tracker.owner_progress.len();
    assert_eq!(pinned_after_seed, 400);

    // A clean read missing the last 5 owner rows PROVES they are gone.
    let clean = compact_projects_apply_emit_mode(
        hour_read(1, false, 5),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_ne!(clean.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    let tombstones = clean.owner_progress.iter().filter(|row| row.deleted).count();
    assert_eq!(tombstones, 5, "a clean read must tombstone what it proves gone");
    assert_eq!(
        tracker.owner_progress.len(),
        395,
        "a clean read advances the pin"
    );

    // A partial read after it does NOT advance the pin.
    let partial = compact_projects_apply_emit_mode(
        hour_read(2, true, 0),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_ne!(partial.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(
        tracker.owner_progress.len(),
        395,
        "a partial read must leave the pin exactly where the clean read left it"
    );
}

/// A read that fails for any reason OTHER than incomplete coverage is not a
/// patch candidate under any mode, and must not disturb the pin.
///
/// Production change that makes this fail: widening `DeltaPartial`'s match arm
/// past `PARTIAL_DATA`.
#[test]
fn delta_partial_refuses_every_failure_flag_except_partial_data() {
    for flag in [
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        COMPACT_STATUS_STATE_UNAVAILABLE,
        COMPACT_STATUS_WORKER_PANIC,
        COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE,
    ] {
        let mut tracker = CompactProjectsTracker::default();
        let seed = compact_projects_apply_emit_mode(
            hour_read(0, false, 0),
            D41EmitMode::DeltaPartial,
            &mut tracker,
        );
        assert_eq!(seed.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
        let pinned = tracker.owner_progress.len();

        let mut source = hour_read(1, false, 0);
        source.status_flags |= flag;
        let source_len = encoded_len(&source);
        let emitted =
            compact_projects_apply_emit_mode(source, D41EmitMode::DeltaPartial, &mut tracker);
        assert_eq!(
            emitted.status_flags & COMPACT_STATUS_DELTA_DATA,
            0,
            "flag {flag:#x} must not be patched"
        );
        assert_eq!(encoded_len(&emitted), source_len);
        assert_eq!(
            tracker.owner_progress.len(),
            pinned,
            "flag {flag:#x} must not move the pin"
        );
    }
}

/// The size claim, measured on the encoder rather than asserted. With t19's
/// change rate the patch has to be a small fraction of the whole payload; a
/// regression that quietly re-emits everything would still pass the
/// reconstruction gate above, so the win needs its own assertion.
#[test]
fn delta_partial_emits_a_small_fraction_of_the_full_payload() {
    let mut full_tracker = CompactProjectsTracker::default();
    let mut delta_tracker = CompactProjectsTracker::default();
    let mut full_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let hours = 24u32;

    for hour in 0..hours {
        // Hour 0 is clean: it is the only record that may seed the chain, and a
        // partial one is not allowed to. See `delta_partial_reconstructs_every_
        // partial_hour_after_a_clean_seed`.
        let partial = hour != 0;
        full_bytes += encoded_len(&compact_projects_apply_emit_mode(
            hour_read(hour, partial, 0),
            D41EmitMode::Full,
            &mut full_tracker,
        ));
        delta_bytes += encoded_len(&compact_projects_apply_emit_mode(
            hour_read(hour, partial, 0),
            D41EmitMode::DeltaPartial,
            &mut delta_tracker,
        ));
    }

    assert!(
        delta_bytes * 5 < full_bytes,
        "delta emission must be well under a fifth of full emission: \
         {delta_bytes} vs {full_bytes} over {hours} hours"
    );
}

/// The same measurement at t19's measured D41 scale: 21,511 owner-joined
/// progress rows, which is the row family that dominates its 1.46 MB payload.
/// Reported per steady-state hour — the seeding keyframe is charged separately
/// because a capture pays it once, not hourly.
#[test]
fn delta_partial_measured_at_t19_owner_progress_scale() {
    const ROWS: usize = 21_511;
    const HOURS: u32 = 12;
    let mut full_tracker = CompactProjectsTracker::default();
    let mut delta_tracker = CompactProjectsTracker::default();
    let mut full_bytes = 0usize;
    let mut keyframe_bytes = 0usize;
    let mut steady_delta_bytes = 0usize;

    for hour in 0..HOURS {
        // Hour 0 is the clean seed; see the module's seeding rule.
        let partial = hour != 0;
        full_bytes += encoded_len(&compact_projects_apply_emit_mode(
            hour_read_scaled(hour, partial, 0, ROWS),
            D41EmitMode::Full,
            &mut full_tracker,
        ));
        let emitted = compact_projects_apply_emit_mode(
            hour_read_scaled(hour, partial, 0, ROWS),
            D41EmitMode::DeltaPartial,
            &mut delta_tracker,
        );
        if hour == 0 {
            keyframe_bytes = encoded_len(&emitted);
        } else {
            steady_delta_bytes += encoded_len(&emitted);
        }
    }

    let full_per_hour = full_bytes / HOURS as usize;
    let delta_per_hour = steady_delta_bytes / (HOURS - 1) as usize;
    eprintln!(
        "MEASURE rows={ROWS} hours={HOURS} full_per_hour={full_per_hour}          keyframe={keyframe_bytes} steady_delta_per_hour={delta_per_hour}          ratio={:.5}",
        delta_per_hour as f64 / full_per_hour as f64
    );
    // Measured at this change rate: 23,779 vs 1,119,468 bytes, 2.12%. The
    // guard is set at 5% so it catches a regression to whole-payload emission
    // without pinning the exact synthetic change rate.
    assert!(
        delta_per_hour * 20 < full_per_hour,
        "at t19 scale a steady-state hour must cost well under a twentieth of a          full payload: {delta_per_hour} vs {full_per_hour}"
    );
}

/// The pin's own state, applied the way a consumer's delta-baseline mirror
/// applies a patch. Held as maps rather than as a `CompactProjectsRead` so the
/// expectation is computed from the CONTRACT — complete reads advance the pin,
/// incomplete ones do not — and never read out of the producer's tracker.
#[derive(Clone, Default)]
struct Pin {
    facilities: BTreeMap<u32, ProjectFacilityRow>,
    scientists: BTreeMap<u32, ProjectScientistRow>,
    owner_progress: BTreeMap<(u32, u32), ProjectOwnerProgressRow>,
}

impl Pin {
    fn applied(&self, emitted: &CompactProjectsRead, is_delta: bool) -> Pin {
        let mut next = if is_delta { self.clone() } else { Pin::default() };
        for row in &emitted.facilities {
            if row.deleted {
                next.facilities.remove(&row.id);
            } else {
                next.facilities.insert(row.id, row.clone());
            }
        }
        for row in &emitted.scientists {
            if row.deleted {
                next.scientists.remove(&row.char_id);
            } else {
                next.scientists.insert(row.char_id, row.clone());
            }
        }
        for row in &emitted.owner_progress {
            let key = (row.tag_db_id, row.local_id);
            if row.deleted {
                next.owner_progress.remove(&key);
            } else {
                next.owner_progress.insert(key, row.clone());
            }
        }
        next
    }

    fn hour(&self) -> ReconstructedHour {
        ReconstructedHour {
            facilities: self
                .facilities
                .values()
                .map(|row| (row.id, row.province, row.unread_prototype_rewards))
                .collect(),
            scientists: self
                .scientists
                .values()
                .map(|row| (row.char_id, row.trait_count))
                .collect(),
            owner_progress: self
                .owner_progress
                .values()
                .map(|row| {
                    (
                        row.tag_db_id,
                        row.local_id,
                        row.phase_progress_raw,
                        row.prototype_iterations,
                    )
                })
                .collect(),
        }
    }
}

/// A capture that alternates coverage. This is the sequence the all-partial gate
/// cannot reach: the pin MOVES partway through, so every later patch is measured
/// against a different reference and a reconstruction still using the original
/// baseline would silently diverge. The schedule also drives rows that vanish and
/// come back, and values that revert to exactly what the pin already holds.
///
/// Production change that makes this fail: advancing the pin on an incomplete
/// read, or failing to advance it on a complete one. Either desynchronises the
/// reference and the very next hour reconstructs the wrong rows.
#[test]
fn mixed_clean_and_partial_coverage_reconstructs_across_pin_moves() {
    // (incomplete coverage?, owner rows dropped from the tail)
    let schedule = [
        (true, 0usize),
        (true, 11),
        (false, 4),
        (true, 0),
        (true, 25),
        (false, 0),
        (true, 9),
        (false, 9),
        (true, 0),
    ];
    let mut full_tracker = CompactProjectsTracker::default();
    let mut delta_tracker = CompactProjectsTracker::default();
    let mut pin = Pin::default();
    let mut seeded = false;

    for (hour, (partial, dropped)) in schedule.into_iter().enumerate() {
        let hour = hour as u32;
        let full = compact_projects_apply_emit_mode(
            hour_read(hour, partial, dropped),
            D41EmitMode::Full,
            &mut full_tracker,
        );
        let emitted = compact_projects_apply_emit_mode(
            hour_read(hour, partial, dropped),
            D41EmitMode::DeltaPartial,
            &mut delta_tracker,
        );
        let is_delta = emitted.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        // Only a record the consumer applies may seed, so the leading partial
        // hours are whole payloads that pin nothing. The chain starts at the
        // first CLEAN read and every record after it is a patch.
        assert_eq!(
            is_delta, seeded,
            "hour {hour}: the chain starts at the first clean read and only records after it patch"
        );

        let applied = pin.applied(&emitted, is_delta);
        assert_eq!(
            sorted(applied.hour()),
            sorted(reconstruct_full(&full)),
            "hour {hour} (partial={partial}, dropped={dropped}) reconstructed the wrong rows"
        );

        // The contract: the seeding record and every complete read become the
        // next reference; an incomplete read never does.
        if !partial {
            pin = applied;
            seeded = true;
        }
    }
    assert!(
        seeded,
        "the schedule must contain a clean read, or it proves nothing about the pin"
    );
}

/// The `PPA1` authority word must describe the STATE, not the patch.
///
/// `D41_AUTHORITY_X9_SPECIALIZATIONS` claims that every breakthrough row this
/// record's read observed carried a resolved specialization. It used to be
/// derived with `read.breakthrough.iter().all(..)` inside
/// `append_compact_project_authority_trailer`, which runs AFTER
/// `compact_projects_apply_emit_mode` has replaced that vector with a patch —
/// and `all()` over an empty patch is vacuously true. So a state whose whole
/// payload was refused the claim would be granted it on the very next unchanged
/// hour, and the fold stores the authority whether or not the record re-carried
/// the unresolved row.
///
/// MUTATION: put `read.breakthrough.iter().all(|row| row.deleted || matches!(
/// row.specialization_code, PROJECT_SPECIALIZATION_NUCLEAR | ..LAND))` back in
/// place of `read.breakthrough_specializations_resolved` and this fails —
/// observed verbatim:
///   "an empty patch is not evidence that every breakthrough row resolved"
#[test]
fn an_unresolved_breakthrough_is_not_forgotten_by_the_patch_that_omits_it() {
    fn read_with_unresolved_breakthrough() -> CompactProjectsRead {
        let mut read = read_with(
            vec![facility(1, 10, 0)],
            Vec::new(),
            vec![owner_progress(42, 1, 5, 0)],
            0,
        );
        read.breakthrough = vec![ProjectBreakthroughRow {
            tag_db_id: 42,
            program_status_index: 0,
            specialization_handle: 7,
            points_raw: 3,
            // The unresolved case: the handle did not classify.
            specialization_code: 0,
            deleted: false,
        }];
        // What `read_compact_projects` computes over the WHOLE observed row set.
        read.breakthrough_specializations_resolved = false;
        read.d18_authority = Some(D41CountryRosterAuthority {
            row_count: 1,
            sha256: [0u8; 32],
        });
        read
    }

    fn authority_word(read: &CompactProjectsRead) -> u32 {
        let mut payload = compact_projects_data_payload(read, 0);
        let trailer_start = payload.len();
        append_compact_project_authority_trailer(&mut payload, read, 1, 10);
        assert!(
            payload.len() > trailer_start,
            "the authority trailer must be emitted for this read"
        );
        let at = trailer_start + 44;
        u32::from_le_bytes([payload[at], payload[at + 1], payload[at + 2], payload[at + 3]])
    }

    let mut tracker = CompactProjectsTracker::default();
    let seed = compact_projects_apply_emit_mode(
        read_with_unresolved_breakthrough(),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_eq!(seed.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(
        authority_word(&seed) & D41_AUTHORITY_X9_SPECIALIZATIONS,
        0,
        "the whole payload carrying the unresolved row must be refused the X9 claim"
    );

    // The same read again. Nothing changed, so the patch carries no breakthrough
    // row at all — and must still be refused the claim.
    let patch = compact_projects_apply_emit_mode(
        read_with_unresolved_breakthrough(),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_ne!(
        patch.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "an unchanged read must patch"
    );
    assert!(
        patch.breakthrough.is_empty(),
        "the unchanged breakthrough row must not be re-carried"
    );
    assert_eq!(
        authority_word(&patch) & D41_AUTHORITY_X9_SPECIALIZATIONS,
        0,
        "an empty patch is not evidence that every breakthrough row resolved"
    );

    // And the claim is still GRANTED when the state really does resolve, so the
    // fix is not "never set the bit".
    let mut resolved_tracker = CompactProjectsTracker::default();
    let mut resolved = read_with_unresolved_breakthrough();
    resolved.breakthrough[0].specialization_code = PROJECT_SPECIALIZATION_NUCLEAR;
    resolved.breakthrough_specializations_resolved = true;
    let resolved_seed =
        compact_projects_apply_emit_mode(resolved, D41EmitMode::Delta, &mut resolved_tracker);
    assert_ne!(
        authority_word(&resolved_seed) & D41_AUTHORITY_X9_SPECIALIZATIONS,
        0,
        "a resolved state must still earn the X9 claim"
    );
}

/// A trailer walk that is not key-ascending emits WHOLE and drops the pin.
///
/// `fold_special_projects` keeps PLCF/PPGF/PRHF as `Vec`s: a whole record stores
/// them in walk order, a patch is merged through a `BTreeMap` and comes back in
/// ascending key order. While the walk is ascending the two agree — it is, in
/// every record of t26 — but if it ever were not, the same hour would fold to
/// differently ordered arrays under whole and patch emission and the per-hour
/// snapshot hash would differ for an identical world.
///
/// The predicate has THREE independent clauses, one per trailer, and each is
/// checked here on its own tracker: dropping any one of them must turn a case
/// red. A gate that reversed only owner-progress would let the lifecycle and
/// reward-history clauses be deleted silently.
///
/// MUTATIONS, each observed verbatim:
///
/// * delete the `compact_projects_trailers_are_key_ascending` guard in
///   `compact_projects_apply_emit_mode`:
///   "an out-of-order walk must not be patched"
/// * drop the `read.lifecycle` clause of the predicate:
///   "assertion `left == right` failed: lifecycle: an out-of-order walk must
///    not be patched"
/// * drop the `read.reward_history` clause:
///   "assertion `left == right` failed: reward_history: an out-of-order walk
///    must not be patched"
#[test]
fn an_out_of_order_trailer_walk_is_not_patched() {
    // One case per clause of the predicate: seed ascending, then walk the SAME
    // rows of one trailer family in the other order.
    for (family, reverse) in [
        ("lifecycle", 0usize),
        ("owner_progress", 1),
        ("reward_history", 2),
    ] {
        fn lifecycle_rows(reversed: bool) -> Vec<ProjectLifecycleRow> {
            let mut rows = vec![
                ProjectLifecycleRow {
                    tag_db_id: 42,
                    local_id: 1,
                    started: true,
                    completed: false,
                    deleted: false,
                },
                ProjectLifecycleRow {
                    tag_db_id: 42,
                    local_id: 2,
                    started: true,
                    completed: false,
                    deleted: false,
                },
            ];
            if reversed {
                rows.reverse();
            }
            rows
        }
        fn reward_rows(reversed: bool) -> Vec<ProjectRewardHistoryRow> {
            let row = |history_index: u32| ProjectRewardHistoryRow {
                tag_db_id: 42,
                local_id: 1,
                history_index,
                prototype_reward_handle: 3,
                option_handle: 4,
                province: 5,
                progress_raw: 6,
                character_ref_type: 0,
                character_ref_id: 0,
                country_ref: 0,
                scientist_ref: 0,
                state_ref: 0,
                date_raw: 7,
                country_present: false,
                scientist_present: false,
                state_present: false,
                deleted: false,
            };
            let mut rows = vec![row(0), row(1)];
            if reversed {
                rows.reverse();
            }
            rows
        }
        let read = |reversed: bool| {
            let mut read = read_with(
                Vec::new(),
                Vec::new(),
                if reversed && reverse == 1 {
                    vec![owner_progress(42, 2, 5, 0), owner_progress(42, 1, 5, 0)]
                } else {
                    vec![owner_progress(42, 1, 5, 0), owner_progress(42, 2, 5, 0)]
                },
                0,
            );
            read.lifecycle = lifecycle_rows(reversed && reverse == 0);
            read.lifecycle_countries = 1;
            read.reward_history = reward_rows(reversed && reverse == 2);
            read.reward_history_countries = 1;
            read
        };

        let mut tracker = CompactProjectsTracker::default();
        let seed = compact_projects_apply_emit_mode(read(false), D41EmitMode::Delta, &mut tracker);
        assert_eq!(
            seed.status_flags & COMPACT_STATUS_DELTA_DATA,
            0,
            "{family}: the ascending seed is a keyframe"
        );
        assert!(tracker.baseline_seeded, "{family}: the ascending seed pins");

        let out_of_order =
            compact_projects_apply_emit_mode(read(true), D41EmitMode::Delta, &mut tracker);
        assert_eq!(
            out_of_order.status_flags & COMPACT_STATUS_DELTA_DATA,
            0,
            "{family}: an out-of-order walk must not be patched"
        );
        assert!(
            !tracker.baseline_seeded,
            "{family}: the pin is dropped, so the next record is a keyframe too"
        );
    }

    let mut tracker = CompactProjectsTracker::default();
    let seed = compact_projects_apply_emit_mode(
        read_with(
            Vec::new(),
            Vec::new(),
            vec![owner_progress(42, 1, 5, 0), owner_progress(42, 2, 5, 0)],
            0,
        ),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_eq!(seed.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert!(tracker.baseline_seeded, "the ascending seed pins");

    // Same rows, walked in the other order.
    let reversed = compact_projects_apply_emit_mode(
        read_with(
            Vec::new(),
            Vec::new(),
            vec![owner_progress(42, 2, 5, 0), owner_progress(42, 1, 5, 0)],
            0,
        ),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_eq!(
        reversed.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "an out-of-order walk must not be patched"
    );
    assert_eq!(
        reversed.owner_progress.len(),
        2,
        "the whole payload carries both rows"
    );
    assert!(
        !tracker.baseline_seeded,
        "the pin is dropped, so the next record is a keyframe too"
    );

    // An ascending walk after it starts a fresh chain.
    let next = compact_projects_apply_emit_mode(
        read_with(
            Vec::new(),
            Vec::new(),
            vec![owner_progress(42, 1, 5, 0), owner_progress(42, 2, 5, 0)],
            0,
        ),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_eq!(
        next.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "the record after a dropped pin is a keyframe"
    );
    let patched = compact_projects_apply_emit_mode(
        read_with(
            Vec::new(),
            Vec::new(),
            vec![owner_progress(42, 1, 5, 0), owner_progress(42, 2, 5, 0)],
            0,
        ),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_ne!(
        patched.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "and the chain resumes from there"
    );
}
