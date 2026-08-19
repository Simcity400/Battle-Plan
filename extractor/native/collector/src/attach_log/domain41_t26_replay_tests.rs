use super::*;
use std::path::PathBuf;

const T26_PAYLOAD_BYTES: usize = 423_701_454;
const T26_DATA_RECORDS: usize = 291;

fn t26_stream() -> PathBuf {
    if let Some(explicit) = std::env::var_os("BATTLE_PLAN_T26_STREAM").or_else(|| std::env::var_os("HOI4_T26_STREAM")) {
        let path = PathBuf::from(explicit);
        assert!(
            path.is_file(),
            "BATTLE_PLAN_T26_STREAM names no file: {}",
            path.display()
        );
        return path;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runs")
        .join("live_unlock")
        .join("t26")
        .join("collector_stream.bin");
    assert!(
        path.is_file(),
        "t26's collector stream was not found at {} — set BATTLE_PLAN_T26_STREAM to it",
        path.display()
    );
    path
}

fn u16le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn i32le(bytes: &[u8], at: usize) -> i32 {
    u32le(bytes, at) as i32
}

fn u64le(bytes: &[u8], at: usize) -> u64 {
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(out)
}

struct DecodedRecord {
    read: CompactProjectsRead,
    elapsed_wall_ms: u64,
    encoder_owned: Vec<u8>,
}

fn take_string(payload: &[u8], cursor: &mut usize) -> String {
    let len = u16le(payload, *cursor) as usize;
    *cursor += 2;
    let out = String::from_utf8_lossy(&payload[*cursor..*cursor + len]).into_owned();
    *cursor += len;
    out
}

fn decode_projects_payload(payload: &[u8]) -> DecodedRecord {
    const HEADER_LEN: usize = 152;
    let elapsed_wall_ms = u64le(payload, 0);
    let status_flags = u32le(payload, 16);
    let facility_count = u32le(payload, 20) as usize;
    let project_count = u32le(payload, 24) as usize;
    let scientist_count = u32le(payload, 28) as usize;
    let header_extension = u32le(payload, 52);
    assert_eq!(
        header_extension, COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1,
        "this decoder is written for the current D41 header extension only"
    );
    let project_state_count = u32le(payload, 136) as usize;
    let breakthrough_count = u32le(payload, 140) as usize;

    let mut cursor = HEADER_LEN;
    let mut facilities = Vec::with_capacity(facility_count);
    for _ in 0..facility_count {
        let flags = u32le(payload, cursor + 8);
        facilities.push(ProjectFacilityRow {
            id: u32le(payload, cursor),
            province: u32le(payload, cursor + 4),
            building: None,
            active_project_handle: u32le(payload, cursor + 12),
            active: flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_ACTIVE != 0,
            scientist_char_id: (flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_SCIENTIST != 0)
                .then(|| u32le(payload, cursor + 16)),
            unread_prototype_rewards: u32le(payload, cursor + 20),
            deleted: flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_DELETED != 0,
        });
        cursor += 32;
    }
    let mut facility_has_building = Vec::with_capacity(facility_count);
    for index in 0..facility_count {
        let flags = u32le(payload, HEADER_LEN + index * 32 + 8);
        facility_has_building.push(flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_BUILDING != 0);
    }

    let mut projects = Vec::with_capacity(project_count);
    for _ in 0..project_count {
        let flags = u32le(payload, cursor + 16);
        projects.push(ProjectRow {
            id: u32le(payload, cursor),
            static_def_handle: u32le(payload, cursor + 4),
            iterations: u32le(payload, cursor + 8),
            worked_at_facility: (flags & COMPACT_PROJECT_ROW_FLAG_WORKED_AT_FACILITY != 0)
                .then(|| u32le(payload, cursor + 12)),
            deleted: flags & COMPACT_PROJECT_ROW_FLAG_DELETED != 0,
        });
        cursor += 24;
    }

    let mut scientists = Vec::with_capacity(scientist_count);
    let mut scientist_flags = Vec::with_capacity(scientist_count);
    for _ in 0..scientist_count {
        let flags = u32le(payload, cursor + 4);
        let deleted = flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DELETED != 0;
        let mut skills = BTreeMap::new();
        if !deleted {
            for (index, key) in [
                "specialization_nuclear",
                "specialization_naval",
                "specialization_air",
                "specialization_land",
            ]
            .into_iter()
            .enumerate()
            {
                skills.insert(key, i32le(payload, cursor + 12 + index * 4));
            }
        }
        scientists.push(ProjectScientistRow {
            char_id: u32le(payload, cursor),
            name_token: None,
            display_name: None,
            skills,
            trait_count: (flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_TRAIT_COUNT != 0)
                .then(|| u32le(payload, cursor + 8)),
            deleted,
        });
        scientist_flags.push(flags);
        cursor += 32;
    }

    let mut project_states = Vec::with_capacity(project_state_count);
    for _ in 0..project_state_count {
        project_states.push(ProjectStateRow {
            id: u32le(payload, cursor),
            static_def_handle: u32le(payload, cursor + 4),
            current_state_offset: u32le(payload, cursor + 8),
            current_state_enum: u32le(payload, cursor + 12),
            prototype_state_enum: u32le(payload, cursor + 16),
            simple_state_1_enum: u32le(payload, cursor + 20),
            simple_state_2_enum: u32le(payload, cursor + 24),
            simple_state_3_enum: u32le(payload, cursor + 28),
            prototype_iterations: u32le(payload, cursor + 32),
            phase_progress_raw: u32le(payload, cursor + 36),
            phase_target_raw: u32le(payload, cursor + 40),
            state_flags: u32le(payload, cursor + 44),
            deleted: false,
        });
        cursor += 48;
    }

    let mut breakthrough = Vec::with_capacity(breakthrough_count);
    for _ in 0..breakthrough_count {
        let flags = u32le(payload, cursor + 12);
        let deleted = flags & COMPACT_PROJECT_BREAKTHROUGH_ROW_FLAG_DELETED != 0;
        let specialization_code =
            if deleted || flags & COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_PRESENT == 0 {
                0
            } else {
                (flags >> COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_SHIFT) & 0b111
            };
        breakthrough.push(ProjectBreakthroughRow {
            tag_db_id: u32le(payload, cursor),
            program_status_index: flags >> COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_INDEX_SHIFT,
            specialization_handle: u32le(payload, cursor + 4),
            points_raw: u32le(payload, cursor + 8),
            specialization_code,
            deleted,
        });
        cursor += 16;
    }

    for (index, facility) in facilities.iter_mut().enumerate() {
        let value = take_string(payload, &mut cursor);
        if facility_has_building[index] {
            facility.building = Some(value);
        }
    }
    for (index, scientist) in scientists.iter_mut().enumerate() {
        let name_token = take_string(payload, &mut cursor);
        let display_name = take_string(payload, &mut cursor);
        if scientist_flags[index] & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_NAME_TOKEN != 0 {
            scientist.name_token = Some(name_token);
        }
        if scientist_flags[index] & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DISPLAY_NAME != 0 {
            scientist.display_name = Some(display_name);
        }
    }

    let mut lifecycle = Vec::new();
    let mut lifecycle_countries = 0u32;
    let mut lifecycle_aborted_countries = 0u32;
    let mut owner_progress = Vec::new();
    let mut owner_progress_countries = 0u32;
    let mut owner_progress_aborted_countries = 0u32;
    let mut reward_history = Vec::new();
    let mut reward_history_countries = 0u32;
    let mut reward_history_aborted_countries = 0u32;
    while cursor + 8 <= payload.len() {
        let magic = &payload[cursor..cursor + 4];
        let header_len = u16le(payload, cursor + 6) as usize;
        let row_count = u32le(payload, cursor + 8) as usize;
        let base = cursor + header_len;
        if magic == COMPACT_PROJECT_LIFECYCLE_TRAILER_MAGIC {
            lifecycle_countries = u32le(payload, cursor + 12);
            lifecycle_aborted_countries = u32le(payload, cursor + 16);
            for index in 0..row_count {
                let at = base + index * 12;
                let flags = u32le(payload, at + 8);
                lifecycle.push(ProjectLifecycleRow {
                    tag_db_id: u32le(payload, at),
                    local_id: u32le(payload, at + 4),
                    started: flags & COMPACT_PROJECT_LIFECYCLE_ROW_FLAG_STARTED != 0,
                    completed: flags & COMPACT_PROJECT_LIFECYCLE_ROW_FLAG_COMPLETED != 0,
                    deleted: flags & COMPACT_PROJECT_LIFECYCLE_ROW_FLAG_DELETED != 0,
                });
            }
            cursor = base + row_count * 12;
        } else if magic == COMPACT_PROJECT_PROGRESS_TRAILER_MAGIC {
            owner_progress_countries = u32le(payload, cursor + 12);
            owner_progress_aborted_countries = u32le(payload, cursor + 16);
            for index in 0..row_count {
                let at = base + index * 52;
                let flags = u32le(payload, at + 48);
                owner_progress.push(ProjectOwnerProgressRow {
                    tag_db_id: u32le(payload, at),
                    local_id: u32le(payload, at + 4),
                    static_def_handle: u32le(payload, at + 8),
                    current_state_offset: u32le(payload, at + 12),
                    current_state_enum: u32le(payload, at + 16),
                    prototype_state_enum: u32le(payload, at + 20),
                    simple_state_1_enum: u32le(payload, at + 24),
                    simple_state_2_enum: u32le(payload, at + 28),
                    simple_state_3_enum: u32le(payload, at + 32),
                    prototype_iterations: u32le(payload, at + 36),
                    phase_progress_raw: u32le(payload, at + 40),
                    phase_target_raw: u32le(payload, at + 44),
                    state_flags: if flags == COMPACT_PROJECT_STATE_ROW_FLAG_DELETED {
                        0
                    } else {
                        flags
                    },
                    deleted: flags == COMPACT_PROJECT_STATE_ROW_FLAG_DELETED,
                });
            }
            cursor = base + row_count * 52;
        } else if magic == COMPACT_PROJECT_REWARD_HISTORY_TRAILER_MAGIC {
            reward_history_countries = u32le(payload, cursor + 12);
            reward_history_aborted_countries = u32le(payload, cursor + 16);
            for index in 0..row_count {
                let at = base + index * COMPACT_PROJECT_REWARD_HISTORY_ROW_LEN;
                let flags = u32le(payload, at + 52);
                reward_history.push(ProjectRewardHistoryRow {
                    tag_db_id: u32le(payload, at),
                    local_id: u32le(payload, at + 4),
                    history_index: u32le(payload, at + 8),
                    prototype_reward_handle: u32le(payload, at + 12),
                    option_handle: u32le(payload, at + 16),
                    province: i32le(payload, at + 20),
                    progress_raw: u32le(payload, at + 24),
                    character_ref_type: u32le(payload, at + 28),
                    character_ref_id: u32le(payload, at + 32),
                    country_ref: u32le(payload, at + 36),
                    scientist_ref: u32le(payload, at + 40),
                    state_ref: u32le(payload, at + 44),
                    date_raw: i32le(payload, at + 48),
                    country_present: flags & COMPACT_PROJECT_REWARD_HISTORY_COUNTRY_PRESENT != 0,
                    scientist_present: flags & COMPACT_PROJECT_REWARD_HISTORY_SCIENTIST_PRESENT
                        != 0,
                    state_present: flags & COMPACT_PROJECT_REWARD_HISTORY_STATE_PRESENT != 0,
                    deleted: flags == COMPACT_PROJECT_REWARD_HISTORY_DELETED,
                });
            }
            cursor = base + row_count * COMPACT_PROJECT_REWARD_HISTORY_ROW_LEN;
        } else {
            break;
        }
    }

    let mut root_reasons = ProjectRootReasonCounters::default();
    root_reasons.invalid_facility_root_read_fail = u32le(payload, 88);
    root_reasons.invalid_facility_slot_read_fail = u32le(payload, 92);
    root_reasons.invalid_facility_candidate_vtable_read_fail = u32le(payload, 96);
    root_reasons.invalid_facility_candidate_sentinel_after_pin = u32le(payload, 100);
    root_reasons.invalid_facility_candidate_fields_invalid = u32le(payload, 104);
    root_reasons.invalid_project_pool_root_read_fail = u32le(payload, 108);
    root_reasons.invalid_project_pool_vtable_read_fail = u32le(payload, 112);
    root_reasons.invalid_project_pool_first_project_read_fail = u32le(payload, 116);
    root_reasons.invalid_project_candidate_vtable_read_fail = u32le(payload, 120);
    root_reasons.invalid_project_candidate_sentinel_after_pin = u32le(payload, 124);
    root_reasons.invalid_project_candidate_fields_invalid = u32le(payload, 128);
    root_reasons.invalid_project_short_array = u32le(payload, 132);

    let read = CompactProjectsRead {
        phase: Domain41PhaseTimes::default(),
        facilities,
        projects,
        scientists,
        project_states,
        breakthrough,
        d18_authority: None,
        read_time_us: 0,
        changed_rows: u32le(payload, 12),
        status_flags,
        observed_facility_rows: u32le(payload, 32),
        observed_project_rows: u32le(payload, 36),
        observed_scientist_rows: u32le(payload, 40),
        observed_project_state_rows: u32le(payload, 144),
        observed_breakthrough_rows: u32le(payload, 148),
        scan_ms: u64le(payload, 56),
        bridge_rows_supplied: u32le(payload, 44),
        source_kind: u32le(payload, 48),
        invalid_program_status_countries: u32le(payload, 64),
        invalid_facility_countries: u32le(payload, 68),
        invalid_project_pool_countries: u32le(payload, 72),
        neutral_program_status_countries: u32le(payload, 76),
        neutral_facility_countries: u32le(payload, 80),
        neutral_project_pool_countries: u32le(payload, 84),
        invalid_breakthrough_countries: 0,
        breakthrough_specializations_resolved: true,
        facility_row_shortfall: 0,
        scientist_row_shortfall: 0,
        root_reasons,
        lifecycle,
        lifecycle_countries,
        lifecycle_aborted_countries,
        owner_progress,
        owner_progress_countries,
        owner_progress_aborted_countries,
        reward_history,
        reward_history_countries,
        reward_history_aborted_countries,
    };
    DecodedRecord {
        read,
        elapsed_wall_ms,
        encoder_owned: payload[..cursor].to_vec(),
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct FoldedProjects {
    facilities: BTreeMap<u32, ProjectFacilityRow>,
    scientists: BTreeMap<u32, ProjectScientistRow>,
    breakthroughs: BTreeMap<(u32, u32, u32), ProjectBreakthroughRow>,
    lifecycle: BTreeMap<(u32, u32), ProjectLifecycleRow>,
    owner_progress: BTreeMap<(u32, u32), ProjectOwnerProgressRow>,
    reward_history: BTreeMap<(u32, u32, u32), ProjectRewardHistoryRow>,
}

impl FoldedProjects {
    fn apply(&mut self, emitted: &CompactProjectsRead) {
        let delta = emitted.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        if !delta {
            *self = FoldedProjects::default();
        }
        for row in &emitted.facilities {
            if row.deleted {
                self.facilities.remove(&row.id);
            } else {
                self.facilities.insert(row.id, row.clone());
            }
        }
        for row in &emitted.scientists {
            if row.deleted {
                self.scientists.remove(&row.char_id);
            } else {
                self.scientists.insert(row.char_id, row.clone());
            }
        }
        for row in &emitted.breakthrough {
            let key = (
                row.tag_db_id,
                row.program_status_index,
                row.specialization_handle,
            );
            if row.deleted {
                self.breakthroughs.remove(&key);
            } else {
                self.breakthroughs.insert(key, *row);
            }
        }
        for row in &emitted.lifecycle {
            let key = (row.tag_db_id, row.local_id);
            if row.deleted {
                self.lifecycle.remove(&key);
            } else {
                self.lifecycle.insert(key, *row);
            }
        }
        for row in &emitted.owner_progress {
            let key = (row.tag_db_id, row.local_id);
            if row.deleted {
                self.owner_progress.remove(&key);
            } else {
                self.owner_progress.insert(key, row.clone());
            }
        }
        for row in &emitted.reward_history {
            let key = (row.tag_db_id, row.local_id, row.history_index);
            if row.deleted {
                self.reward_history.remove(&key);
            } else {
                self.reward_history.insert(key, *row);
            }
        }
    }
}

struct ReplayOutcome {
    records: usize,
    gap_forced_keyframes: usize,
    whole_bytes: usize,
    delta_bytes: usize,
    delta_records: usize,
    keyframe_records: usize,
    keyframe_bytes: usize,
    patch_bytes: usize,
    changed_rows_total: u64,
}

fn replay_t26(path: &std::path::Path) -> ReplayOutcome {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).expect("t26 stream opens");
    let total = file.metadata().expect("t26 stream metadata").len();
    let mut header = [0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
    let mut offset = 0u64;

    let mut tracker = CompactProjectsTracker::default();
    let mut whole_state = FoldedProjects::default();
    let mut delta_state = FoldedProjects::default();
    let mut outcome = ReplayOutcome {
        records: 0,
        gap_forced_keyframes: 0,
        whole_bytes: 0,
        delta_bytes: 0,
        delta_records: 0,
        keyframe_records: 0,
        keyframe_bytes: 0,
        patch_bytes: 0,
        changed_rows_total: 0,
    };
    let mut gap_since_last_data = false;

    while offset + compact_contract::COMPACT_RECORD_HEADER_LEN as u64 <= total {
        file.seek(SeekFrom::Start(offset)).expect("seek");
        file.read_exact(&mut header).expect("record header");
        let parsed =
            compact_contract::parse_compact_record_header(&header).expect("t26 framing is valid");
        let payload_len = parsed.payload_len as u64;
        if parsed.domain_id == COMPACT_DOMAIN_PROJECTS {
            if parsed.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP {
                gap_since_last_data = true;
            } else if parsed.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA {
                let mut payload = vec![0u8; parsed.payload_len];
                file.read_exact(&mut payload).expect("record payload");
                outcome.records += 1;
                outcome.whole_bytes += payload.len();

                let decoded = decode_projects_payload(&payload);
                assert_eq!(
                    compact_projects_data_payload(&decoded.read, decoded.elapsed_wall_ms),
                    decoded.encoder_owned,
                    "record {} did not round-trip through the production encoder",
                    outcome.records
                );

                if gap_since_last_data && outcome.records > 1 {
                    tracker.clear();
                    outcome.gap_forced_keyframes += 1;
                }
                gap_since_last_data = false;

                let whole = decode_projects_payload(&payload).read;
                let mut source = decoded.read;
                source.status_flags &= !COMPACT_STATUS_PARTIAL_DATA;
                let emitted =
                    compact_projects_apply_emit_mode(source, D41EmitMode::Delta, &mut tracker);
                let authority_trailer_len = payload.len() - decoded.encoder_owned.len();
                let emitted_len = compact_projects_data_payload(&emitted, decoded.elapsed_wall_ms)
                    .len()
                    + authority_trailer_len;
                if emitted.status_flags & COMPACT_STATUS_DELTA_DATA != 0 {
                    outcome.delta_records += 1;
                    outcome.changed_rows_total += u64::from(emitted.changed_rows);
                    outcome.patch_bytes += emitted_len;
                } else {
                    outcome.keyframe_records += 1;
                    outcome.keyframe_bytes += emitted_len;
                }
                outcome.delta_bytes += emitted_len;

                let mut whole_hour = whole;
                whole_hour.status_flags &= !COMPACT_STATUS_PARTIAL_DATA;
                whole_state.apply(&whole_hour);
                delta_state.apply(&emitted);
                assert!(
                    whole_state == delta_state,
                    "record {} folded to a different world under delta emission",
                    outcome.records
                );
            }
        }
        offset += compact_contract::COMPACT_RECORD_HEADER_LEN as u64 + payload_len;
    }
    outcome
}

#[test]
#[ignore = "capture evidence, Evidence tier only:             scripts/Invoke-CentralRustValidation.ps1 -Mode Evidence             -Workspace Extractor -Package collector -Lib             -FilterExpression 'test(t26_replayed)'             (set BATTLE_PLAN_T26_STREAM if runs/ is not beside the crate)"]
fn t26_replayed_through_the_repaired_emit_path_stops_resending_the_whole_payload() {
    let outcome = replay_t26(&t26_stream());
    assert_eq!(
        outcome.records, T26_DATA_RECORDS,
        "t26 publishes 291 D41 DOMAIN_DATA records"
    );
    assert_eq!(
        outcome.whole_bytes, T26_PAYLOAD_BYTES,
        "t26's D41 payload total is fixed evidence"
    );
    eprintln!(
        "MEASURE d41/t26 records={} whole_bytes={} delta_bytes={} ratio={:.4} \
         keyframes={} (gap_forced={}) keyframe_bytes={} deltas={} patch_bytes={}          patch_bytes_per_record={} changed_rows_total={}",
        outcome.records,
        outcome.whole_bytes,
        outcome.delta_bytes,
        outcome.delta_bytes as f64 / outcome.whole_bytes as f64,
        outcome.keyframe_records,
        outcome.gap_forced_keyframes,
        outcome.keyframe_bytes,
        outcome.delta_records,
        outcome.patch_bytes,
        outcome.patch_bytes / outcome.delta_records.max(1),
        outcome.changed_rows_total,
    );
    assert_eq!(
        outcome.keyframe_records,
        outcome.gap_forced_keyframes + 1,
        "exactly the first record and one record per own gap may be a keyframe"
    );
    assert!(
        outcome.delta_records > 150,
        "most of t26's D41 records must patch: {} of {}",
        outcome.delta_records,
        outcome.records
    );
    assert!(
        outcome.delta_bytes * 2 < outcome.whole_bytes,
        "delta emission must at least halve D41's payload: {} vs {}",
        outcome.delta_bytes,
        outcome.whole_bytes
    );
    let patch_per_record = outcome.patch_bytes / outcome.delta_records.max(1);
    assert!(
        patch_per_record < 10_000,
        "a steady-state D41 patch must stay a few hundred bytes, got {patch_per_record}"
    );
}
