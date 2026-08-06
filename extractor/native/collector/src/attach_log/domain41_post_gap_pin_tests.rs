//! Gates for D41's POST-GAP TRANSACTIONAL PIN.
//!
//! # The defect
//!
//! `trackers.projects` is on the domain-scoped allowlist, so
//! `reset_trackers_after_rejected_bracket` DEFAULT-CONSTRUCTED it whenever d41's
//! own bracket was rejected. That takes `baseline_seeded` with it, and the next
//! accepted read therefore emits a whole 1.46 MB KEYFRAME instead of a patch.
//!
//! Measured by replaying t26's real D41 sequence through the repaired emit path
//! (`domain41_t26_replay_tests`, `DELTA_WIRE_DESIGN.md` §6.2): 291 records
//! become 92 keyframes and 199 patches, and **91 of those keyframes — carrying
//! 133,955,040 of the 134,126,662 remaining payload bytes — follow an own gap
//! since d41's previous data record**, which is the candidate set this pin
//! removes rather than a per-record proof of causation. A steady-state patch is
//! 862 bytes. Over the same capture 6,624 of 13,487,987 rows (0.049%) differ
//! from the row the previous record carried, so those keyframes deliver
//! essentially nothing.
//!
//! # Why this is a size lever and not a correctness repair
//!
//! Unlike d29 (§7.1), d41 leaks no identity across a gap today.
//! `world_fold::fold_special_projects` branches on `COMPACT_STATUS_DELTA_DATA`
//! itself instead of calling `replace_or_sparse_after_first`: its non-delta arm
//! starts every section from an empty map and overwrites `self.special_projects`
//! whole, so a d41 keyframe REPLACES the folded state. That is also why d41 is
//! NOT on `COMPACT_BASELINE_CLEAR_ADMISSIONS` and must not be added to it —
//! the resync primitive exists for domains whose keyframe MERGES, which d41's
//! does not. `d41_never_asks_for_a_baseline_clear` pins that.
//!
//! # Why a rollback and not a retention
//!
//! The rejected frame's records were replaced with a `DOMAIN_GAP`, so the fold
//! received nothing and holds exactly the pre-frame state. RETAINING the
//! advanced pin would put the producer AHEAD of it: a row first observed on the
//! rejected frame was never delivered, and a retained pin would call it
//! unchanged and suppress it for the rest of the campaign
//! (`a_row_first_observed_on_the_rejected_frame_is_not_suppressed`). Rolling
//! back a pin whose domain PUBLISHED is the mirror-image error and leaves the
//! producer BEHIND the fold
//! (`an_unrelated_rejection_does_not_roll_d41_back`).
//!
//! Every property below is proven as an EQUIVALENCE against a run in which the
//! rejected frame never happened, because that run is by definition what the
//! consumer holds.

use super::*;

const D41: u16 = COMPACT_DOMAIN_PROJECTS;
/// A domain whose bracket is unrelated to d41's, for the controls that must NOT
/// roll the pin back.
const UNRELATED: u16 = COMPACT_DOMAIN_WEATHER;

fn facility(id: u32, unread: u32) -> ProjectFacilityRow {
    ProjectFacilityRow {
        id,
        province: 100 + id,
        building: None,
        active_project_handle: 0,
        active: false,
        scientist_char_id: None,
        unread_prototype_rewards: unread,
        deleted: false,
    }
}

fn owner_progress(local_id: u32, phase: u32) -> ProjectOwnerProgressRow {
    ProjectOwnerProgressRow {
        tag_db_id: 1,
        local_id,
        static_def_handle: 7,
        current_state_offset: 0,
        current_state_enum: 1,
        prototype_state_enum: 0,
        simple_state_1_enum: 0,
        simple_state_2_enum: 0,
        simple_state_3_enum: 0,
        prototype_iterations: 0,
        phase_progress_raw: phase,
        phase_target_raw: 1_000_000,
        state_flags: 0,
        deleted: false,
    }
}

/// A read carrying `facilities` and `owner_progress`, clean unless a status word
/// is supplied.
fn read_of(
    facilities: Vec<ProjectFacilityRow>,
    owner_progress_rows: Vec<ProjectOwnerProgressRow>,
    status_flags: u32,
) -> CompactProjectsRead {
    let observed_facility_rows = facilities.len() as u32;
    let observed_project_rows = owner_progress_rows.len() as u32;
    CompactProjectsRead {
        facilities,
        projects: Vec::new(),
        scientists: Vec::new(),
        project_states: Vec::new(),
        breakthrough: Vec::new(),
        d18_authority: None,
        read_time_us: 1_000,
        changed_rows: observed_facility_rows,
        status_flags,
        observed_facility_rows,
        observed_project_rows,
        observed_scientist_rows: 0,
        observed_project_state_rows: observed_project_rows,
        observed_breakthrough_rows: 0,
        scan_ms: 0,
        bridge_rows_supplied: 0,
        source_kind: 1,
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

/// A steady d41 world: `count` facilities and `count` owner-progress rows.
fn steady_world(count: u32) -> CompactProjectsRead {
    read_of(
        (1..=count).map(|id| facility(id, 0)).collect(),
        (1..=count).map(|id| owner_progress(id, 500)).collect(),
        0,
    )
}

fn rejected(domains: &[u16]) -> BTreeSet<u16> {
    domains.iter().copied().collect()
}

/// The production frame sequence: the reset only when something was rejected,
/// then the unconditional commit. Spelled as the two real functions so this
/// cannot drift into a convenience wrapper the frame builder does not use.
fn settle_frame(trackers: &mut CompactPerfGateTrackers, frame_rejected_domains: &BTreeSet<u16>) {
    if !frame_rejected_domains.is_empty() {
        reset_trackers_after_rejected_bracket(trackers, frame_rejected_domains);
    }
    commit_delta_pin_transactions(trackers);
}

/// What one frame emits: the read as `compact_projects_apply_emit_mode` left it,
/// plus the bytes the wire would carry.
struct Emitted {
    is_delta: bool,
    facility_ids: Vec<u32>,
    owner_progress_ids: Vec<u32>,
    payload_len: usize,
}

/// Run one d41 frame end to end: the reader's emit-mode pass, then the frame
/// seal for whatever the frame's bracket verdict was.
fn run_frame(
    trackers: &mut CompactPerfGateTrackers,
    read: CompactProjectsRead,
    frame_rejected_domains: &BTreeSet<u16>,
) -> Emitted {
    let emitted =
        compact_projects_apply_emit_mode(read, D41EmitMode::Delta, &mut trackers.projects);
    let payload = compact_projects_data_payload(&emitted, 0);
    let out = Emitted {
        is_delta: emitted.status_flags & COMPACT_STATUS_DELTA_DATA != 0,
        facility_ids: emitted.facilities.iter().map(|row| row.id).collect(),
        owner_progress_ids: emitted.owner_progress.iter().map(|row| row.local_id).collect(),
        payload_len: payload.len(),
    };
    settle_frame(trackers, frame_rejected_domains);
    out
}

/// A read the frame then REJECTS: its record is replaced with a `DOMAIN_GAP`, so
/// the fold never sees it and whatever it emitted is discarded.
fn run_gapped_frame(trackers: &mut CompactPerfGateTrackers, read: CompactProjectsRead) {
    let _ = run_frame(trackers, read, &rejected(&[D41]));
}

/// THE LEVER. After d41's own gap the next record is a PATCH, not a keyframe.
///
/// What production change makes this fail (verified): deleting the
/// `rolled_back_projects` carve-out in `reset_trackers_after_rejected_bracket`,
/// so `trackers.projects` takes the blanket `CompactProjectsTracker::default()`
/// again — which is exactly the shipped behaviour this lane changes — or never
/// sealing the transaction in `commit_delta_pin_transactions`, which rewinds
/// past the seed and unseeds the pin.
///
/// What it deliberately does NOT distinguish is rolling back from RETAINING:
/// the gap frames here read an unchanged world, so a retained pin also produces
/// an empty patch. `a_row_first_observed_on_the_rejected_frame_is_not_suppressed`
/// is the test that separates those two.
#[test]
fn a_post_own_gap_d41_record_is_a_patch_instead_of_a_keyframe() {
    let mut trackers = CompactPerfGateTrackers::default();

    let seed = run_frame(&mut trackers, steady_world(64), &BTreeSet::new());
    assert!(!seed.is_delta, "the first record of the stream is a keyframe");
    assert_eq!(seed.facility_ids.len(), 64, "a keyframe carries every row");

    // Three consecutive frames whose d41 bracket was rejected. Each one READ —
    // the reader runs before the verdict is known — and each one's record was
    // replaced with a `DOMAIN_GAP`.
    for _ in 0..3 {
        run_gapped_frame(&mut trackers, steady_world(64));
    }

    let resumed = run_frame(&mut trackers, steady_world(64), &BTreeSet::new());
    assert!(
        resumed.is_delta,
        "d41 resumes from the pin after its own gap instead of re-keyframing"
    );
    assert!(
        resumed.facility_ids.is_empty() && resumed.owner_progress_ids.is_empty(),
        "nothing changed across the gap, so the patch carries no rows: {:?} / {:?}",
        resumed.facility_ids,
        resumed.owner_progress_ids
    );
    assert!(
        resumed.payload_len * 20 < seed.payload_len,
        "the post-gap record must be a patch, not a keyframe: {} vs {} bytes",
        resumed.payload_len,
        seed.payload_len
    );
}

/// THE EQUIVALENCE. A gapped run and a run in which the rejected frames never
/// happened emit the same record, byte-for-byte, on the next accepted frame.
/// That run is by definition what the consumer holds, so this is the whole
/// correctness claim for the rollback.
///
/// What production change makes this fail (all three verified by mutation):
/// retaining the advanced pin instead of rolling it back, so the changed row
/// observed on the gapped frames is suppressed; dropping the
/// `rolled_back_projects` carve-out, so the resumed record is a keyframe;
/// or never sealing the transaction in `commit_delta_pin_transactions`, so the
/// rollback reaches back past the seed.
#[test]
fn a_gapped_run_and_an_ungapped_run_emit_the_same_next_record() {
    fn changed_world() -> CompactProjectsRead {
        let mut read = steady_world(64);
        read.facilities[7] = facility(8, 3);
        read.owner_progress[9] = owner_progress(10, 777);
        read
    }

    let mut gapped = CompactPerfGateTrackers::default();
    run_frame(&mut gapped, steady_world(64), &BTreeSet::new());
    for _ in 0..4 {
        run_gapped_frame(&mut gapped, changed_world());
    }
    let after_gap = compact_projects_apply_emit_mode(
        changed_world(),
        D41EmitMode::Delta,
        &mut gapped.projects,
    );

    let mut ungapped = CompactPerfGateTrackers::default();
    run_frame(&mut ungapped, steady_world(64), &BTreeSet::new());
    let without_gap = compact_projects_apply_emit_mode(
        changed_world(),
        D41EmitMode::Delta,
        &mut ungapped.projects,
    );

    assert_eq!(
        compact_projects_data_payload(&after_gap, 0),
        compact_projects_data_payload(&without_gap, 0),
        "the record after the gap must be the record the consumer would have received \
         had the gapped frames never happened"
    );
    assert!(
        after_gap.status_flags & COMPACT_STATUS_DELTA_DATA != 0,
        "and it is a patch"
    );
}

/// A row first observed on the REJECTED frame was never delivered, so the next
/// accepted record must carry it. This is the difference between rolling back
/// and merely RETAINING the pin, and it is the reason a retention is wrong.
///
/// What production change makes this fail: replacing `roll_back_uncommitted_pin`
/// with a no-op (i.e. keeping the advanced pin), after which row 65 compares
/// equal against a pin the fold never received and is suppressed for good.
#[test]
fn a_row_first_observed_on_the_rejected_frame_is_not_suppressed() {
    let mut trackers = CompactPerfGateTrackers::default();
    run_frame(&mut trackers, steady_world(64), &BTreeSet::new());

    let mut grown = steady_world(64);
    grown.facilities.push(facility(65, 0));
    grown.observed_facility_rows = 65;
    run_gapped_frame(&mut trackers, grown);

    let mut grown_again = steady_world(64);
    grown_again.facilities.push(facility(65, 0));
    grown_again.observed_facility_rows = 65;
    let resumed = run_frame(&mut trackers, grown_again, &BTreeSet::new());

    assert!(resumed.is_delta, "still a patch");
    assert_eq!(
        resumed.facility_ids,
        vec![65],
        "facility 65 was only ever observed on frames the fold did not receive, \
         so the first delivered record must carry it"
    );
}

/// The mirror-image control: d41 PUBLISHED its record, so its pin must stay
/// advanced. Rolling it back because some OTHER domain was rejected leaves the
/// producer BEHIND the fold, and the next time the value returns to what the
/// rewound pin holds the producer compares equal, emits nothing, and the fold
/// keeps the superseded value for the rest of the capture.
///
/// What production change makes this fail: widening the rollback condition from
/// `domain_scoped_tracker_tag_survived(COMPACT_DOMAIN_PROJECTS, ..)` to "any
/// domain was rejected".
#[test]
fn an_unrelated_rejection_does_not_roll_d41_back() {
    let mut trackers = CompactPerfGateTrackers::default();
    run_frame(&mut trackers, steady_world(4), &BTreeSet::new());

    // Value moves to 900 on a frame d41 published and an unrelated domain was
    // rejected. The fold APPLIED this record.
    let mut moved = steady_world(4);
    moved.owner_progress[0] = owner_progress(1, 900);
    let published = run_frame(&mut trackers, moved, &rejected(&[UNRELATED]));
    assert_eq!(
        published.owner_progress_ids,
        vec![1],
        "the moved row is emitted on the frame that published it"
    );

    // …and then returns to its old value. A pin that was wrongly rewound to 500
    // compares EQUAL here and emits nothing, and the fold keeps 900 forever.
    let reverted = run_frame(&mut trackers, steady_world(4), &BTreeSet::new());
    assert_eq!(
        reverted.owner_progress_ids,
        vec![1],
        "the reversion must be emitted; a rewound pin would call it unchanged"
    );
}

/// A rejection during the stream's FIRST record leaves the pin unseeded, so the
/// next record is a keyframe again — not a `DELTA_DATA` patch against a baseline
/// the fold has never received, which `world_fold::reject_delta_before_baseline`
/// would mark the domain-hour unknown for.
///
/// What production change makes this fail: rolling back to `current` rather than
/// to the pre-advance value, or seeding `baseline_seeded` outside the tracker so
/// the rollback cannot take it back.
#[test]
fn a_rejection_during_the_first_record_leaves_the_next_one_a_keyframe() {
    let mut trackers = CompactPerfGateTrackers::default();
    run_gapped_frame(&mut trackers, steady_world(8));
    assert!(
        !trackers.projects.baseline_seeded,
        "the fold received nothing, so the producer holds no baseline"
    );

    let retried = run_frame(&mut trackers, steady_world(8), &BTreeSet::new());
    assert!(
        !retried.is_delta,
        "the first record the fold will actually receive must be the keyframe"
    );
    assert_eq!(retried.facility_ids.len(), 8);
}

/// Two rejections in a row still restore ONE frame back — to the last COMMITTED
/// value, never to an intermediate one.
///
/// What production change makes this fail: failing to clear the slot in
/// `commit_pin`, which lets a later rejection reach back past frames the fold
/// has already applied (verified: five of this module's tests fail, this one
/// among them). The other half of the same rule — a frame that advances TWICE —
/// is unreachable through the frame path, because d41 reads once per frame, so
/// it is gated directly on the tracker in
/// `two_advances_in_one_frame_roll_back_to_the_frames_entry_value`.
#[test]
fn a_double_rejection_still_restores_exactly_one_frame_back() {
    let mut gapped = CompactPerfGateTrackers::default();
    run_frame(&mut gapped, steady_world(16), &BTreeSet::new());
    for phase in [600u32, 700, 800] {
        let mut moving = steady_world(16);
        moving.owner_progress[0] = owner_progress(1, phase);
        run_gapped_frame(&mut gapped, moving);
    }
    let resumed =
        compact_projects_apply_emit_mode(steady_world(16), D41EmitMode::Delta, &mut gapped.projects);

    let mut ungapped = CompactPerfGateTrackers::default();
    run_frame(&mut ungapped, steady_world(16), &BTreeSet::new());
    let control = compact_projects_apply_emit_mode(
        steady_world(16),
        D41EmitMode::Delta,
        &mut ungapped.projects,
    );

    assert_eq!(
        compact_projects_data_payload(&resumed, 0),
        compact_projects_data_payload(&control, 0),
        "three consecutive rejections restore the same state one does: the value the \
         consumer last received"
    );
    assert!(
        resumed.owner_progress.is_empty(),
        "the row went 500 -> 600 -> 700 -> 800 -> 500 across frames the fold never saw, \
         so the delivered record carries no change at all"
    );
}

/// The fill-only-when-empty rule, gated where it lives.
///
/// Production advances d41's pin at most once per frame — one read, one
/// `compact_projects_apply_emit_mode` call — so no frame-level test can reach
/// this branch, and one that claimed to would be asserting a path production
/// never takes. The rule is still load-bearing for anything that later stages a
/// second successor inside one frame: rolling back to the INTERMEDIATE value
/// leaves the producer describing a state the fold never received.
///
/// It also gates the dictionary asymmetry, which is why the second half exists:
/// the SPECIALIZATION DICTIONARY is monotonic and must survive a rollback that
/// the row pin does not. Under a naive `*self = *committed` the scan progress
/// made between the two advances is silently discarded with the rows.
///
/// What production change makes this fail: replacing the
/// `already_staged.unwrap_or_else(..)` in `stage_pin_advance` with an
/// unconditional `Some(Box::new(previous))` (the row half), or dropping the
/// dictionary carry-over from `roll_back_uncommitted_pin` (the second half).
#[test]
fn two_advances_in_one_frame_roll_back_to_the_frames_entry_value() {
    fn pin_of(count: u32) -> CompactProjectsTracker {
        let mut pin = CompactProjectsTracker::default();
        pin.baseline_seeded = true;
        let rows = (1..=count).map(|id| facility(id, 0)).collect::<Vec<_>>();
        let mut scratch = Vec::new();
        pin.facilities
            .rebuild_key_ascending(&rows, |row| row.id, &mut scratch);
        pin
    }

    let mut tracker = CompactProjectsTracker::default();
    tracker.stage_pin_advance(pin_of(1));
    tracker.commit_pin();
    assert_eq!(tracker.facilities.rows.len(), 1, "the frame opens with one row");

    tracker.stage_pin_advance(pin_of(2));
    // Scan progress made BETWEEN the two advances. The dictionary resolves
    // specialization handles out of process-global records that are immutable
    // after game-data load, so this is not a delta baseline and the rollback
    // must not take it.
    tracker.specialization_dictionary.scan_cursor = 4_096;
    tracker.specialization_dictionary.scanned = true;
    // A handle resolved the way production resolves one: covered by the census
    // AND mapped to a token `project_specialization_code` recognises. Either
    // half alone would leave the row unresolved, so the fixture carries both.
    tracker.specialization_dictionary.covered_handles.insert(77);
    tracker
        .specialization_dictionary
        .by_handle
        .insert(77, "specialization_nuclear".to_string());
    tracker.stage_pin_advance(pin_of(3));
    assert_eq!(tracker.facilities.rows.len(), 3);

    tracker.roll_back_uncommitted_pin();
    assert_eq!(
        tracker.facilities.rows.len(),
        1,
        "the rollback restores the value the frame OPENED with, not the intermediate one"
    );
    assert!(tracker.uncommitted_pin.is_none(), "and the slot is spent");
    assert_eq!(
        tracker.specialization_dictionary.scan_cursor, 4_096,
        "the dictionary is monotonic and is NOT rolled back with the rows"
    );
    assert!(tracker.specialization_dictionary.scanned);
    assert_eq!(
        tracker.specialization_dictionary.by_handle.get(&77).map(String::as_str),
        Some("specialization_nuclear"),
        "a handle resolved during the frame stays resolved across the rollback"
    );
    assert!(
        tracker.specialization_dictionary.covered_handles.contains(&77),
        "including its census coverage, without which `classify_project_breakthrough_         specializations` would zero the row's specialization code"
    );
}

/// THE VOLUME CLAIM, stated as bytes. Over a sequence whose every other frame is
/// an own gap, d41 emits exactly ONE whole payload — the stream's seed — and the
/// gapped run's total is within a patch of the ungapped run's.
///
/// What production change makes this fail: the same carve-out deletion as
/// above. Under the shipped behaviour this sequence emits 1 + 12 keyframes
/// instead of 1, which the keyframe count assertion states directly.
#[test]
fn the_post_gap_keyframe_volume_is_gone() {
    const GAPS: usize = 12;
    let mut gapped = CompactPerfGateTrackers::default();
    let mut whole_payloads = 0usize;
    let mut gapped_bytes = 0usize;

    let seed = run_frame(&mut gapped, steady_world(128), &BTreeSet::new());
    assert!(!seed.is_delta);
    whole_payloads += 1;
    gapped_bytes += seed.payload_len;

    for _ in 0..GAPS {
        // The gapped frame publishes nothing at all, so it contributes no bytes.
        run_gapped_frame(&mut gapped, steady_world(128));
        let resumed = run_frame(&mut gapped, steady_world(128), &BTreeSet::new());
        if !resumed.is_delta {
            whole_payloads += 1;
        }
        gapped_bytes += resumed.payload_len;
    }

    let mut ungapped = CompactPerfGateTrackers::default();
    let mut ungapped_bytes = run_frame(&mut ungapped, steady_world(128), &BTreeSet::new()).payload_len;
    for _ in 0..GAPS {
        ungapped_bytes += run_frame(&mut ungapped, steady_world(128), &BTreeSet::new()).payload_len;
    }

    assert_eq!(
        whole_payloads, 1,
        "only the stream's seed is a whole payload; {GAPS} own gaps force none"
    );
    assert_eq!(
        gapped_bytes, ungapped_bytes,
        "a capture with {GAPS} own gaps costs the same bytes as one with none"
    );
    assert!(
        gapped_bytes < seed.payload_len * 2,
        "and the whole sequence still costs less than two keyframes: {gapped_bytes} vs {}",
        seed.payload_len
    );
}

/// CF's seed rule survives the transaction. A `PARTIAL_DATA` record is discarded
/// whole by `world_fold::handle_compact_failure` — d41 has no entry in
/// `COMPACT_EXPECTED_PARTIAL_TOLERANCES` — so it must not seed the pin, and a
/// rejection in the middle of that sequence must not seed it either.
///
/// What production change makes this fail: reverting the
/// `if baseline && !advance_baseline { return read; }` guard in
/// `compact_projects_apply_emit_mode`, or having the rollback restore a
/// `baseline_seeded` the discarded record set.
#[test]
fn a_refused_record_still_never_seeds_the_pin_across_a_rejection() {
    let mut trackers = CompactPerfGateTrackers::default();

    let partial = run_frame(
        &mut trackers,
        read_of(
            (1..=8).map(|id| facility(id, 0)).collect(),
            Vec::new(),
            COMPACT_STATUS_PARTIAL_DATA,
        ),
        &BTreeSet::new(),
    );
    assert!(
        !partial.is_delta,
        "a record the fold discards is emitted whole"
    );
    assert!(
        !trackers.projects.baseline_seeded,
        "a record the fold discards must not seed the producer's pin"
    );

    run_gapped_frame(&mut trackers, steady_world(8));
    assert!(
        !trackers.projects.baseline_seeded,
        "and a rejected frame cannot seed it either"
    );

    let clean = run_frame(&mut trackers, steady_world(8), &BTreeSet::new());
    assert!(
        !clean.is_delta,
        "the first record the fold will apply is the keyframe that seeds the chain"
    );
    assert!(trackers.projects.baseline_seeded);

    let next = run_frame(&mut trackers, steady_world(8), &BTreeSet::new());
    assert!(
        next.is_delta,
        "and every record after it patches, which is what the seed guard is worth"
    );
}

/// D41 MUST NOT ACQUIRE THE RESYNC PRIMITIVE. `COMPACT_STATUS_BASELINE_CLEAR`
/// exists for a domain whose keyframe MERGES and whose producer knows the two
/// sides have parted company. d41's keyframe REPLACES, so the pin above is the
/// whole repair and the bit would be a wire change older readers hard-reject for
/// nothing.
///
/// What production change makes this fail: adding a `CompactBaselineClearAdmission`
/// for domain 41, or setting bit 15 anywhere on d41's emitted status word.
///
/// This is a LAW gate and not a transaction gate: it passes with the pin
/// carve-out reverted, deliberately, because the law it states is about the wire
/// contract rather than about this lane's mechanism.
#[test]
fn d41_never_asks_for_a_baseline_clear() {
    assert!(
        !compact_contract::compact_baseline_clear_admissible(D41),
        "d41 is not on COMPACT_BASELINE_CLEAR_ADMISSIONS and the post-gap pin does not \
         need it: `fold_special_projects` full-replaces on a non-delta record"
    );

    // EVERY emission arm, not only the steady delta/gap one: the three emit
    // modes crossed with a clean read, a `PARTIAL_DATA` read and a hard failure,
    // each settled both as published and as its own gap. A conditional bit-15
    // set on any arm this domain can take is therefore in range.
    let mut statuses = Vec::new();
    for mode in [
        D41EmitMode::Delta,
        D41EmitMode::Full,
        D41EmitMode::DeltaPartial,
    ] {
        for status_flags in [
            0,
            COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
        ] {
            let mut trackers = CompactPerfGateTrackers::default();
            for frame in 0..6 {
                let mut read = steady_world(8);
                read.status_flags = status_flags;
                if status_flags & COMPACT_STATUS_PARTIAL_DATA != 0 {
                    read.invalid_project_pool_countries = 1;
                }
                let emitted =
                    compact_projects_apply_emit_mode(read, mode, &mut trackers.projects);
                statuses.push((mode, status_flags, emitted.status_flags));
                let verdict = if frame % 2 == 1 {
                    rejected(&[D41])
                } else {
                    BTreeSet::new()
                };
                settle_frame(&mut trackers, &verdict);
            }
        }
    }
    assert_eq!(statuses.len(), 54, "every arm is sampled");
    for (mode, input, emitted) in statuses {
        assert_eq!(
            emitted & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
            0,
            "no d41 record may carry the resync bit (mode {mode:?}, input status {input:#x})"
        );
    }
}
