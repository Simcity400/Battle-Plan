use super::*;

const D41: u16 = COMPACT_DOMAIN_PROJECTS;
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

fn read_of(
    facilities: Vec<ProjectFacilityRow>,
    owner_progress_rows: Vec<ProjectOwnerProgressRow>,
    status_flags: u32,
) -> CompactProjectsRead {
    let observed_facility_rows = facilities.len() as u32;
    let observed_project_rows = owner_progress_rows.len() as u32;
    CompactProjectsRead {
        phase: Domain41PhaseTimes::default(),
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
        facility_row_shortfall: 0,
        scientist_row_shortfall: 0,
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

fn settle_frame(trackers: &mut CompactPerfGateTrackers, frame_rejected_domains: &BTreeSet<u16>) {
    if !frame_rejected_domains.is_empty() {
        reset_trackers_after_rejected_bracket(trackers, frame_rejected_domains);
    }
    commit_delta_pin_transactions(trackers);
}

struct Emitted {
    is_delta: bool,
    facility_ids: Vec<u32>,
    owner_progress_ids: Vec<u32>,
    payload_len: usize,
}

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
        owner_progress_ids: emitted
            .owner_progress
            .iter()
            .map(|row| row.local_id)
            .collect(),
        payload_len: payload.len(),
    };
    settle_frame(trackers, frame_rejected_domains);
    out
}

fn run_gapped_frame(trackers: &mut CompactPerfGateTrackers, read: CompactProjectsRead) {
    let _ = run_frame(trackers, read, &rejected(&[D41]));
}

#[test]
fn a_post_own_gap_d41_record_is_a_patch_instead_of_a_keyframe() {
    let mut trackers = CompactPerfGateTrackers::default();

    let seed = run_frame(&mut trackers, steady_world(64), &BTreeSet::new());
    assert!(
        !seed.is_delta,
        "the first record of the stream is a keyframe"
    );
    assert_eq!(seed.facility_ids.len(), 64, "a keyframe carries every row");

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
    let after_gap =
        compact_projects_apply_emit_mode(changed_world(), D41EmitMode::Delta, &mut gapped.projects);

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

#[test]
fn an_unrelated_rejection_does_not_roll_d41_back() {
    let mut trackers = CompactPerfGateTrackers::default();
    run_frame(&mut trackers, steady_world(4), &BTreeSet::new());

    let mut moved = steady_world(4);
    moved.owner_progress[0] = owner_progress(1, 900);
    let published = run_frame(&mut trackers, moved, &rejected(&[UNRELATED]));
    assert_eq!(
        published.owner_progress_ids,
        vec![1],
        "the moved row is emitted on the frame that published it"
    );

    let reverted = run_frame(&mut trackers, steady_world(4), &BTreeSet::new());
    assert_eq!(
        reverted.owner_progress_ids,
        vec![1],
        "the reversion must be emitted; a rewound pin would call it unchanged"
    );
}

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

#[test]
fn a_double_rejection_still_restores_exactly_one_frame_back() {
    let mut gapped = CompactPerfGateTrackers::default();
    run_frame(&mut gapped, steady_world(16), &BTreeSet::new());
    for phase in [600u32, 700, 800] {
        let mut moving = steady_world(16);
        moving.owner_progress[0] = owner_progress(1, phase);
        run_gapped_frame(&mut gapped, moving);
    }
    let resumed = compact_projects_apply_emit_mode(
        steady_world(16),
        D41EmitMode::Delta,
        &mut gapped.projects,
    );

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
    assert_eq!(
        tracker.facilities.rows.len(),
        1,
        "the frame opens with one row"
    );

    tracker.stage_pin_advance(pin_of(2));
    tracker.specialization_dictionary.scan_cursor = 4_096;
    tracker.specialization_dictionary.scanned = true;
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
        tracker
            .specialization_dictionary
            .by_handle
            .get(&77)
            .map(String::as_str),
        Some("specialization_nuclear"),
        "a handle resolved during the frame stays resolved across the rollback"
    );
    assert!(
        tracker.specialization_dictionary.covered_handles.contains(&77),
        "including its census coverage, without which `classify_project_breakthrough_         specializations` would zero the row's specialization code"
    );
}

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
        run_gapped_frame(&mut gapped, steady_world(128));
        let resumed = run_frame(&mut gapped, steady_world(128), &BTreeSet::new());
        if !resumed.is_delta {
            whole_payloads += 1;
        }
        gapped_bytes += resumed.payload_len;
    }

    let mut ungapped = CompactPerfGateTrackers::default();
    let mut ungapped_bytes =
        run_frame(&mut ungapped, steady_world(128), &BTreeSet::new()).payload_len;
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
#[test]
fn d41_never_asks_for_a_baseline_clear() {
    assert!(
        !compact_contract::compact_baseline_clear_admissible(D41),
        "d41 is not on COMPACT_BASELINE_CLEAR_ADMISSIONS and the post-gap pin does not \
         need it: `fold_special_projects` full-replaces on a non-delta record"
    );

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
                let emitted = compact_projects_apply_emit_mode(read, mode, &mut trackers.projects);
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
