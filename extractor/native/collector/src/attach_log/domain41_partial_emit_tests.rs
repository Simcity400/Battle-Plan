use super::*;

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
        phase: Domain41PhaseTimes::default(),
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
        facility_loss: Domain41MaterialiseLoss::default(),
        scientist_loss: Domain41MaterialiseLoss::default(),
        cache_too_many: false,
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

fn sorted(mut hour: ReconstructedHour) -> ReconstructedHour {
    hour.facilities.sort_unstable();
    hour.scientists.sort_unstable();
    hour.owner_progress.sort_unstable();
    hour
}

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

#[test]
fn delta_partial_reconstructs_every_partial_hour_after_a_clean_seed() {
    let mut full_tracker = CompactProjectsTracker::default();
    let mut delta_tracker = CompactProjectsTracker::default();
    let mut baseline: Option<CompactProjectsRead> = None;

    for hour in 0..24u32 {
        let dropped = if (6..10).contains(&hour) {
            (hour as usize - 5) * 7
        } else {
            0
        };
        let partial = hour != 0;
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

#[test]
fn default_delta_mode_still_refuses_to_patch_a_partial_read() {
    let mut tracker = CompactProjectsTracker::default();
    for hour in 0..4u32 {
        let source_len = encoded_len(&hour_read(hour, true, 0));
        let emitted = compact_projects_apply_emit_mode(
            hour_read(hour, true, 0),
            D41EmitMode::Delta,
            &mut tracker,
        );
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

#[test]
fn a_partial_first_record_never_seeds_the_chain() {
    let mut tracker = CompactProjectsTracker::default();

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

    let second = compact_projects_apply_emit_mode(
        hour_read(1, true, 0),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_eq!(second.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert!(!tracker.baseline_seeded);

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

#[test]
fn a_domain_that_empties_out_mid_capture_does_not_re_seed() {
    let mut tracker = CompactProjectsTracker::default();
    let seeded = compact_projects_apply_emit_mode(
        read_with(vec![facility(1, 10, 0)], Vec::new(), Vec::new(), 0),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_eq!(seeded.status_flags & COMPACT_STATUS_DELTA_DATA, 0);

    let emptied = compact_projects_apply_emit_mode(
        read_with(Vec::new(), Vec::new(), Vec::new(), 0),
        D41EmitMode::Delta,
        &mut tracker,
    );
    assert_ne!(emptied.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(
        emptied.facilities.len(),
        1,
        "the removal ships as a tombstone"
    );
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

    let clean = compact_projects_apply_emit_mode(
        hour_read(1, false, 5),
        D41EmitMode::DeltaPartial,
        &mut tracker,
    );
    assert_ne!(clean.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    let tombstones = clean
        .owner_progress
        .iter()
        .filter(|row| row.deleted)
        .count();
    assert_eq!(
        tombstones, 5,
        "a clean read must tombstone what it proves gone"
    );
    assert_eq!(
        tracker.owner_progress.len(),
        395,
        "a clean read advances the pin"
    );

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

#[test]
fn delta_partial_emits_a_small_fraction_of_the_full_payload() {
    let mut full_tracker = CompactProjectsTracker::default();
    let mut delta_tracker = CompactProjectsTracker::default();
    let mut full_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let hours = 24u32;

    for hour in 0..hours {
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
    assert!(
        delta_per_hour * 20 < full_per_hour,
        "at t19 scale a steady-state hour must cost well under a twentieth of a          full payload: {delta_per_hour} vs {full_per_hour}"
    );
}

#[derive(Clone, Default)]
struct Pin {
    facilities: BTreeMap<u32, ProjectFacilityRow>,
    scientists: BTreeMap<u32, ProjectScientistRow>,
    owner_progress: BTreeMap<(u32, u32), ProjectOwnerProgressRow>,
}

impl Pin {
    fn applied(&self, emitted: &CompactProjectsRead, is_delta: bool) -> Pin {
        let mut next = if is_delta {
            self.clone()
        } else {
            Pin::default()
        };
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

#[test]
fn mixed_clean_and_partial_coverage_reconstructs_across_pin_moves() {
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
            specialization_code: 0,
            deleted: false,
        }];
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
        u32::from_le_bytes([
            payload[at],
            payload[at + 1],
            payload[at + 2],
            payload[at + 3],
        ])
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

#[test]
fn an_out_of_order_trailer_walk_is_not_patched() {
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
