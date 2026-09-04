use super::*;

use super::lock_domain41_diagnostics as lock_diagnostics;

#[allow(clippy::too_many_arguments)]
fn raw_history_row(
    history_vtable: usize,
    context_vtable: usize,
    country: (u8, u32),
    scientist: (u8, u32),
    state: (u8, u32),
) -> ObjectSnapshot<PROJECT_HISTORY_ROW_STRIDE> {
    let mut buf = [0u8; PROJECT_HISTORY_ROW_STRIDE];
    buf[0..8].copy_from_slice(&(history_vtable as u64).to_le_bytes());
    let at = PROJECT_HISTORY_OFF_CONTEXT;
    buf[at..at + 8].copy_from_slice(&(context_vtable as u64).to_le_bytes());
    let at = PROJECT_HISTORY_OFF_PROTOTYPE_REWARD;
    buf[at..at + 4].copy_from_slice(&0x4C17u32.to_le_bytes());
    let at = PROJECT_HISTORY_OFF_OPTION;
    buf[at..at + 4].copy_from_slice(&0x4C17u32.to_le_bytes());
    let at = PROJECT_HISTORY_OFF_PROVINCE;
    buf[at..at + 4].copy_from_slice(&(-1i32).to_le_bytes());
    let at = PROJECT_HISTORY_OFF_DATE_RAW;
    buf[at..at + 4].copy_from_slice(&0x29C7_7F8i32.to_le_bytes());
    for (flag_off, ref_off, (flag, raw)) in [
        (
            PROJECT_HISTORY_OFF_COUNTRY_PRESENT,
            PROJECT_HISTORY_OFF_COUNTRY_REF,
            country,
        ),
        (
            PROJECT_HISTORY_OFF_SCIENTIST_PRESENT,
            PROJECT_HISTORY_OFF_SCIENTIST_REF,
            scientist,
        ),
        (
            PROJECT_HISTORY_OFF_STATE_PRESENT,
            PROJECT_HISTORY_OFF_STATE_REF,
            state,
        ),
    ] {
        buf[flag_off] = flag;
        buf[ref_off..ref_off + 4].copy_from_slice(&raw.to_le_bytes());
    }
    ObjectSnapshot { buf }
}

const COUNTRIES: u32 = 436;
const PROJECTS_PER_COUNTRY: u32 = 49;
const FACILITIES: u32 = 27;
const SCIENTISTS: u32 = 153;

fn facility_row(id: u32) -> ProjectFacilityRow {
    ProjectFacilityRow {
        id,
        province: 1000 + id,
        building: None,
        active_project_handle: id,
        active: id % 3 == 0,
        scientist_char_id: None,
        unread_prototype_rewards: id % 7,
        deleted: false,
    }
}

fn scientist_row(char_id: u32) -> ProjectScientistRow {
    ProjectScientistRow {
        char_id,
        name_token: None,
        display_name: None,
        skills: BTreeMap::new(),
        trait_count: Some(char_id % 5),
        deleted: false,
    }
}

fn owner_progress_row(tag_db_id: u32, local_id: u32) -> ProjectOwnerProgressRow {
    ProjectOwnerProgressRow {
        tag_db_id,
        local_id,
        static_def_handle: local_id % 97,
        current_state_offset: 0,
        current_state_enum: 1,
        prototype_state_enum: 0,
        simple_state_1_enum: 0,
        simple_state_2_enum: 0,
        simple_state_3_enum: 0,
        prototype_iterations: local_id % 11,
        phase_progress_raw: local_id * 13,
        phase_target_raw: 1_000_000,
        state_flags: 0,
        deleted: false,
    }
}

fn lifecycle_row(tag_db_id: u32, local_id: u32) -> ProjectLifecycleRow {
    ProjectLifecycleRow {
        tag_db_id,
        local_id,
        started: local_id % 17 == 0,
        completed: local_id % 41 == 0,
        deleted: false,
    }
}

fn reward_row(
    tag_db_id: u32,
    local_id: u32,
    history_index: u32,
    country_present: bool,
    country_raw: u32,
) -> ProjectRewardHistoryRow {
    ProjectRewardHistoryRow {
        tag_db_id,
        local_id,
        history_index,
        prototype_reward_handle: 0x4C17,
        option_handle: 0x4C17,
        province: -1,
        progress_raw: 0,
        character_ref_type: 0,
        character_ref_id: 0,
        country_ref: project_history_optional_ref(country_present, country_raw),
        scientist_ref: 0,
        state_ref: 0,
        date_raw: 0x29C7_7F8,
        country_present,
        scientist_present: false,
        state_present: false,
        deleted: false,
    }
}

fn t26_scale_read(status_flags: u32) -> CompactProjectsRead {
    let facilities: Vec<_> = (1..=FACILITIES).map(facility_row).collect();
    let scientists: Vec<_> = (1..=SCIENTISTS).map(scientist_row).collect();
    let mut owner_progress = Vec::new();
    let mut lifecycle = Vec::new();
    for tag in 1..=COUNTRIES {
        for local in 1..=PROJECTS_PER_COUNTRY {
            owner_progress.push(owner_progress_row(tag, local));
            lifecycle.push(lifecycle_row(tag, local));
        }
    }
    let reward_history = vec![
        reward_row(47, 2297, 0, true, 47),
        reward_row(47, 2297, 1, false, 0xDEAD_BEEF),
        reward_row(68, 3288, 6, false, 0x0BAD_F00D),
    ];
    let observed_facility_rows = facilities.len() as u32;
    let observed_scientist_rows = scientists.len() as u32;
    let observed_project_rows = owner_progress.len() as u32;
    let changed_rows = observed_facility_rows.saturating_add(observed_scientist_rows);
    CompactProjectsRead {
        omissions: None,
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
        invalid_project_pool_countries: 0,
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
        lifecycle,
        lifecycle_countries: COUNTRIES,
        lifecycle_aborted_countries: if status_flags & COMPACT_STATUS_PARTIAL_DATA != 0 {
            1
        } else {
            0
        },
        owner_progress,
        owner_progress_countries: COUNTRIES,
        owner_progress_aborted_countries: if status_flags & COMPACT_STATUS_PARTIAL_DATA != 0 {
            1
        } else {
            0
        },
        reward_history,
        reward_history_countries: COUNTRIES,
        reward_history_aborted_countries: if status_flags & COMPACT_STATUS_PARTIAL_DATA != 0 {
            1
        } else {
            0
        },
    }
}

fn emit(tracker: &mut CompactProjectsTracker, status_flags: u32) -> (CompactProjectsRead, Vec<u8>) {
    let read =
        compact_projects_apply_emit_mode(t26_scale_read(status_flags), D41EmitMode::Delta, tracker);
    let payload = compact_projects_data_payload(&read, 1);
    (read, payload)
}

fn payload_status(payload: &[u8]) -> u32 {
    u32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]])
}

#[test]
fn a_clean_unchanged_d41_read_emits_a_patch_not_a_whole_snapshot() {
    let mut tracker = CompactProjectsTracker::default();

    let (baseline, keyframe) = emit(&mut tracker, 0);
    assert_eq!(
        keyframe_status_has_delta(&keyframe),
        false,
        "the seeding hour must be a keyframe, not a patch"
    );
    assert!(
        keyframe.len() > 400_000,
        "a t26-scale keyframe should be hundreds of KB, got {}",
        keyframe.len()
    );
    assert_eq!(
        baseline.owner_progress.len(),
        (COUNTRIES * PROJECTS_PER_COUNTRY) as usize,
        "the seeding hour carries every row"
    );

    let (delta, patch) = emit(&mut tracker, 0);
    assert_ne!(
        payload_status(&patch) & COMPACT_STATUS_DELTA_DATA,
        0,
        "an unchanged clean read must announce itself as a patch"
    );
    assert!(
        delta.owner_progress.is_empty() && delta.lifecycle.is_empty(),
        "nothing changed, so no owner-progress or lifecycle row may be carried \
         (got {} / {})",
        delta.owner_progress.len(),
        delta.lifecycle.len()
    );
    assert!(
        patch.len() * 100 < keyframe.len(),
        "an unchanged hour must cost less than 1% of a keyframe: patch {} vs \
         keyframe {}",
        patch.len(),
        keyframe.len()
    );

    let mut partial_tracker = CompactProjectsTracker::default();
    let (_, first) = emit(&mut partial_tracker, COMPACT_STATUS_PARTIAL_DATA);
    let (_, second) = emit(&mut partial_tracker, COMPACT_STATUS_PARTIAL_DATA);
    assert_eq!(
        payload_status(&second) & COMPACT_STATUS_DELTA_DATA,
        0,
        "a PARTIAL read must not take the delta path in `Delta` mode"
    );
    assert_eq!(
        first.len(),
        second.len(),
        "t26's signature: identical whole payloads hour after hour"
    );
    assert!(
        second.len() * 100 > keyframe.len(),
        "the partial hour costs a whole payload, which is the regression"
    );
}

fn keyframe_status_has_delta(payload: &[u8]) -> bool {
    payload_status(payload) & COMPACT_STATUS_DELTA_DATA != 0
}

#[test]
fn the_optional_discriminator_still_has_a_boolean_domain() {
    let _guard = lock_diagnostics();
    assert_eq!(project_history_presence(0), Some(false));
    assert_eq!(project_history_presence(1), Some(true));
    for junk in [2u8, 3, 0x0D, 0x7F, 0x80, 0xCD, 0xFF] {
        assert_eq!(
            project_history_presence(junk),
            None,
            "discriminator byte {junk:#04x} is not a boolean and must refuse the country"
        );
    }
}

#[test]
fn a_refused_discriminator_latches_the_byte_that_refused_it() {
    let _guard = lock_diagnostics();
    reset_project_lifecycle_failure_diagnostics();
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_PRESENCE_BYTE.load(Ordering::Relaxed),
        0,
        "the reset must clear the sample with the rest of the failure words"
    );
    DOMAIN41_LIFECYCLE_CURRENT_STAGE.store(513, Ordering::Relaxed);
    assert_eq!(project_history_presence(0xCD), None);
    retain_first_project_lifecycle_failure(47);
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_STAGE.load(Ordering::Relaxed),
        513,
        "the sample is only readable under a presence stage"
    );
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_PRESENCE_BYTE.load(Ordering::Relaxed),
        0xCD,
        "the refusing byte must reach the PERF channel"
    );
    assert_eq!(project_history_presence(1), Some(true));
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_PRESENCE_BYTE.load(Ordering::Relaxed),
        0xCD,
        "an accepted byte is not a failure and must not touch the sample"
    );
    reset_project_lifecycle_failure_diagnostics();
}

#[test]
fn the_retained_failure_words_all_describe_the_same_refusal() {
    let _guard = lock_diagnostics();
    reset_project_lifecycle_failure_diagnostics();

    DOMAIN41_LIFECYCLE_CURRENT_STAGE.store(513, Ordering::Relaxed);
    DOMAIN41_LIFECYCLE_CURRENT_LOCAL_ID.store(2297, Ordering::Relaxed);
    DOMAIN41_LIFECYCLE_CURRENT_HISTORY_INDEX.store(1, Ordering::Relaxed);
    assert_eq!(project_history_presence(0xCD), None);
    retain_first_project_lifecycle_failure(47);

    DOMAIN41_LIFECYCLE_CURRENT_STAGE.store(515, Ordering::Relaxed);
    DOMAIN41_LIFECYCLE_CURRENT_LOCAL_ID.store(3288, Ordering::Relaxed);
    DOMAIN41_LIFECYCLE_CURRENT_HISTORY_INDEX.store(6, Ordering::Relaxed);
    assert_eq!(project_history_presence(0xEE), None);
    retain_first_project_lifecycle_failure(68);

    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_TAG_DB_ID.load(Ordering::Relaxed),
        47,
        "the FIRST refusal is the retained one"
    );
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_STAGE.load(Ordering::Relaxed),
        513
    );
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_LOCAL_ID.load(Ordering::Relaxed),
        2297
    );
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_HISTORY_INDEX.load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        DOMAIN41_LIFECYCLE_FAILURE_PRESENCE_BYTE.load(Ordering::Relaxed),
        0xCD,
        "the sample must describe country 47's refusal, not country 68's"
    );
    reset_project_lifecycle_failure_diagnostics();
}

#[test]
fn the_reader_canonicalises_every_absent_optional_it_decodes() {
    let _guard = lock_diagnostics();
    const HISTORY_VT: usize = 0x1_4296_C220;
    const CONTEXT_VT: usize = 0x1_4296_C1D0;

    let stale = raw_history_row(
        HISTORY_VT,
        CONTEXT_VT,
        (0, 0xDEAD_BEEF),
        (0, 0x0BAD_F00D),
        (0, 0xABCD_0001),
    );
    let decoded = project_reward_history_row_from_snapshot(&stale, 2297, 1, HISTORY_VT, CONTEXT_VT)
        .expect("an absent optional with a stale payload must decode, not refuse the country");
    assert!(!decoded.country_present);
    assert!(!decoded.scientist_present);
    assert!(!decoded.state_present);
    assert_eq!(decoded.country_ref, 0);
    assert_eq!(decoded.scientist_ref, 0);
    assert_eq!(decoded.state_ref, 0);

    let present = raw_history_row(HISTORY_VT, CONTEXT_VT, (1, 47), (1, 1484), (1, 900));
    let decoded =
        project_reward_history_row_from_snapshot(&present, 2297, 0, HISTORY_VT, CONTEXT_VT)
            .expect("a present optional must decode");
    assert_eq!(decoded.country_ref, 47);
    assert_eq!(decoded.scientist_ref, 1484);
    assert_eq!(decoded.state_ref, 900);

    reset_project_lifecycle_failure_diagnostics();
    let junk = raw_history_row(HISTORY_VT, CONTEXT_VT, (0xCD, 0), (0, 0), (0, 0));
    assert!(
        project_reward_history_row_from_snapshot(&junk, 2297, 1, HISTORY_VT, CONTEXT_VT).is_none(),
        "a non-boolean discriminator must still refuse"
    );
    assert_eq!(
        DOMAIN41_LIFECYCLE_CURRENT_STAGE.load(Ordering::Relaxed),
        513
    );
    assert_eq!(
        DOMAIN41_LIFECYCLE_CURRENT_PRESENCE_BYTE.load(Ordering::Relaxed),
        0xCD
    );
    reset_project_lifecycle_failure_diagnostics();
}

#[test]
fn an_absent_optional_reaches_the_wire_as_a_zero_ref() {
    assert_eq!(project_history_optional_ref(true, 0xDEAD_BEEF), 0xDEAD_BEEF);
    assert_eq!(project_history_optional_ref(false, 0xDEAD_BEEF), 0);
    assert_eq!(project_history_optional_ref(false, 0), 0);

    let read = t26_scale_read(0);
    let payload = compact_projects_data_payload(&read, 1);
    let trailer_at = payload
        .windows(4)
        .position(|w| w == COMPACT_PROJECT_REWARD_HISTORY_TRAILER_MAGIC)
        .expect("PRHF trailer must be present");
    let row_count = u32::from_le_bytes([
        payload[trailer_at + 8],
        payload[trailer_at + 9],
        payload[trailer_at + 10],
        payload[trailer_at + 11],
    ]) as usize;
    assert_eq!(
        row_count, 3,
        "the fixture carries three reward-history rows"
    );

    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = COMPACT_PROJECT_REWARD_HISTORY_ROW_LEN;
    for row_index in 0..row_count {
        let at = trailer_at + HEADER_LEN + row_index * ROW_LEN;
        let country_ref = u32::from_le_bytes([
            payload[at + 36],
            payload[at + 37],
            payload[at + 38],
            payload[at + 39],
        ]);
        let flags = u32::from_le_bytes([
            payload[at + 52],
            payload[at + 53],
            payload[at + 54],
            payload[at + 55],
        ]);
        let present = flags & COMPACT_PROJECT_REWARD_HISTORY_COUNTRY_PRESENT != 0;
        if !present {
            assert_eq!(
                country_ref, 0,
                "row {row_index} has a country ref under a clear presence flag, which \
                 `parse_project_reward_history_segment` rejects for the whole segment"
            );
        }
    }
}

/// cargo test -p collector --release -- --ignored --nocapture \
#[test]
#[ignore = "wall-clock measurement, not an assertion"]
fn measure_d41_keyed_diff_cost_per_frame() {
    for scale in [1u32, 2, 4, 8] {
        let countries = COUNTRIES;
        let per_country = PROJECTS_PER_COUNTRY * scale;
        let mut tracker = CompactProjectsTracker::default();
        let build = || {
            let mut read = t26_scale_read(0);
            read.owner_progress.clear();
            read.lifecycle.clear();
            for tag in 1..=countries {
                for local in 1..=per_country {
                    read.owner_progress.push(owner_progress_row(tag, local));
                    read.lifecycle.push(lifecycle_row(tag, local));
                }
            }
            read
        };
        let _ = compact_projects_apply_emit_mode(build(), D41EmitMode::Delta, &mut tracker);
        let rows = (countries * per_country) as usize;
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let read = build();
            let started = std::time::Instant::now();
            let emitted = compact_projects_apply_emit_mode(read, D41EmitMode::Delta, &mut tracker);
            let elapsed = started.elapsed();
            assert!(emitted.owner_progress.is_empty());
            best = best.min(elapsed);
        }
        println!(
            "d41 keyed diff: {rows} owner-progress + {rows} lifecycle rows -> \
             {:.3} ms/frame (best of 5)",
            best.as_secs_f64() * 1e3
        );
    }
}

#[test]
#[ignore = "wall-clock measurement, not an assertion"]
fn measure_d41_whole_payload_versus_delta_path() {
    let mut partial_tracker = CompactProjectsTracker::default();
    let mut best_a = std::time::Duration::MAX;
    let mut bytes_a = 0usize;
    for _ in 0..5 {
        let read = t26_scale_read(COMPACT_STATUS_PARTIAL_DATA);
        let started = std::time::Instant::now();
        let emitted =
            compact_projects_apply_emit_mode(read, D41EmitMode::Delta, &mut partial_tracker);
        let payload = compact_projects_data_payload(&emitted, 1);
        let elapsed = started.elapsed();
        bytes_a = payload.len();
        best_a = best_a.min(elapsed);
    }

    let mut clean_tracker = CompactProjectsTracker::default();
    let seed =
        compact_projects_apply_emit_mode(t26_scale_read(0), D41EmitMode::Delta, &mut clean_tracker);
    let seed_bytes = compact_projects_data_payload(&seed, 1).len();
    let mut best_b = std::time::Duration::MAX;
    let mut bytes_b = 0usize;
    for _ in 0..5 {
        let read = t26_scale_read(0);
        let started = std::time::Instant::now();
        let emitted =
            compact_projects_apply_emit_mode(read, D41EmitMode::Delta, &mut clean_tracker);
        let payload = compact_projects_data_payload(&emitted, 1);
        let elapsed = started.elapsed();
        bytes_b = payload.len();
        best_b = best_b.min(elapsed);
    }

    println!(
        "d41 arm A (today, PARTIAL -> whole payload):  {:.3} ms/frame, {bytes_a} bytes",
        best_a.as_secs_f64() * 1e3
    );
    println!(
        "d41 arm B (fixed, clean -> diff + patch):     {:.3} ms/frame, {bytes_b} bytes \
         (seed keyframe {seed_bytes} bytes)",
        best_b.as_secs_f64() * 1e3
    );
    println!(
        "d41 delta: {:+.3} ms/frame, {:+} bytes/frame",
        (best_b.as_secs_f64() - best_a.as_secs_f64()) * 1e3,
        bytes_b as i64 - bytes_a as i64
    );
}

#[test]
#[ignore = "wall-clock measurement, not an assertion"]
fn measure_d41_byte_equality_prefilter_versus_keyed_diff() {
    let read = t26_scale_read(0);
    let a = compact_projects_data_payload(&read, 1);
    let b = compact_projects_data_payload(&read, 1);
    assert_eq!(a.len(), b.len());

    let mut best_cmp = std::time::Duration::MAX;
    for _ in 0..20 {
        let started = std::time::Instant::now();
        let equal = a == b;
        let elapsed = started.elapsed();
        assert!(equal);
        best_cmp = best_cmp.min(elapsed);
    }

    let mut tracker = CompactProjectsTracker::default();
    let _ = compact_projects_apply_emit_mode(t26_scale_read(0), D41EmitMode::Delta, &mut tracker);
    let mut best_diff = std::time::Duration::MAX;
    for _ in 0..5 {
        let r = t26_scale_read(0);
        let started = std::time::Instant::now();
        let _ = compact_projects_apply_emit_mode(r, D41EmitMode::Delta, &mut tracker);
        best_diff = best_diff.min(started.elapsed());
    }

    println!(
        "d41 byte-equality compare over {} B: {:.3} ms (best of 20)",
        a.len(),
        best_cmp.as_secs_f64() * 1e3
    );
    println!(
        "d41 keyed diff it would skip:        {:.3} ms (best of 5)",
        best_diff.as_secs_f64() * 1e3
    );
    println!(
        "d41 pre-filter is {:.0}x cheaper than the diff",
        best_diff.as_secs_f64() / best_cmp.as_secs_f64().max(1e-9)
    );
}

#[test]
fn a_stale_word_behind_a_clear_flag_does_not_manufacture_a_delta() {
    let mut tracker = CompactProjectsTracker::default();
    let mut first = t26_scale_read(0);
    first.reward_history = vec![reward_row(47, 2297, 1, false, 0xDEAD_BEEF)];
    let _ = compact_projects_apply_emit_mode(first, D41EmitMode::Delta, &mut tracker);

    let mut second = t26_scale_read(0);
    second.reward_history = vec![reward_row(47, 2297, 1, false, 0x0BAD_F00D)];
    let emitted = compact_projects_apply_emit_mode(second, D41EmitMode::Delta, &mut tracker);
    assert!(
        emitted.reward_history.is_empty(),
        "an absent optional's residue is not state, so it cannot change: {:?}",
        emitted.reward_history
    );
}
