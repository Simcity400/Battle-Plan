//! What D41 emission costs on t26's REAL read sequence, and what it would cost
//! with the delta path alive.
//!
//! `domain41_presence_regression_tests` proves WHY t26's D41 never patched: one
//! optional-discriminator refusal (stage 513, country 47, project 2297, history
//! row 1, byte-identical in all 291 records) put `PARTIAL_DATA` on every record
//! and sent `compact_projects_apply_emit_mode` down its whole-payload return.
//! It proves that on a synthesised read at t26's scale. This module answers the
//! question that synthesis cannot: **how much of t26's D41 payload was actually
//! new information?**
//!
//! Measured directly from `runs/live_unlock/t26/collector_stream.bin`
//! (2,594,412,495 bytes) by decoding all 291 D41 `DOMAIN_DATA` payloads and
//! comparing each row against the row the previous record carried under the same
//! key:
//!
//! | | |
//! |---|---|
//! | D41 `DOMAIN_DATA` records | 291 |
//! | payload bytes | 423,701,454 |
//! | distinct payload lengths | 3 — 1,456,002 ×227, 1,456,058 ×41, 1,456,114 ×23 |
//! | rows compared across the 290 adjacent pairs | 13,487,987 |
//! | rows whose bytes differ from the previous record | 6,624 (0.049%) |
//! | keys added / removed | 2 / 0 |
//! | adjacent pairs with NO changed row at all | 248 of 290 |
//!
//! The three payload lengths are not a read that flattens variation: the row
//! counts are `fac=27 sci=157 brk=2,980 lif=21,462 own=21,462` in every one of
//! the 291 records, and the only thing that moves is the reward history —
//! 422 rows ×227, 423 ×41, 424 ×23, at `COMPACT_PROJECT_REWARD_HISTORY_ROW_LEN`
//! = 56 bytes each. 1,456,058 − 1,456,002 = 56 and 1,456,114 − 1,456,002 = 112.
//! Every byte of the length variation is accounted for by two reward rows the
//! campaign gained.
//!
//! The remaining cost after the repair is the KEYFRAME, and it is set by the
//! gap structure, not by the change rate. D41 published 353 `DOMAIN_GAP`
//! records against its 291 `DOMAIN_DATA` records (148 backpressure, 170
//! hour-label mismatch, 35 torn), and 91 of the 290 post-first data records had
//! at least one D41 gap since their own previous data record. §2.3 of
//! `DELTA_WIRE_DESIGN.md` forces a keyframe after a rejected bracket for that
//! domain, so those 91 records re-send the whole payload however little changed.
//! The test below models EVERY own gap as a reset, which is the conservative
//! bound: fewer resets can only make the delta stream smaller.
//!
//! # What this module actually runs
//!
//! It walks the stream, decodes each D41 payload back into a
//! `CompactProjectsRead`, and drives the production
//! `compact_projects_apply_emit_mode` over the real sequence. Two things make
//! that trustworthy rather than a second implementation of the wire format:
//!
//! 1. **The decode is checked against the encoder.** Every decoded read is
//!    re-encoded with `compact_projects_data_payload` and asserted BYTE-EQUAL to
//!    the bytes it came from (up to the `PPA1` authority trailer, which the
//!    caller appends outside that function). A decode that misread a field
//!    cannot survive that.
//! 2. **The status is corrected from the stream's own diagnostics, not from a
//!    guess.** t26's D41 `DOMAIN_PERF` records carry
//!    `d41_lifecycle_aborted_countries = 1`,
//!    `d41_owner_progress_aborted_countries = 1`,
//!    `d41_reward_history_aborted_countries = 1` and stage 513 in all 291
//!    records, with `d41_invalid_program_status_countries`,
//!    `d41_invalid_facility_countries`, `d41_invalid_project_pool_countries` and
//!    `d41_invalid_breakthrough_countries` all ZERO in all 291; and the payload
//!    header itself shows `facility_count == observed_facility_rows` (27) and
//!    `scientist_count == observed_scientist_rows` (157), so the row-shortfall
//!    arm is off too. Every `PARTIAL_DATA` source in `read_compact_projects`
//!    other than the repaired stage-513 refusal is therefore measurably absent,
//!    and clearing that one bit is what the repaired reader would have produced.
//!
//! The replay's row content is t26's, minus the one country the old reader
//! aborted (47/2297) — the repaired reader would have carried its lifecycle,
//! progress and reward rows and this stream does not contain them. That makes
//! the row totals a small underestimate and does not affect the ratio.
//!
//! # What it measures, run against the real stream
//!
//! ```text
//! MEASURE d41/t26 records=291 whole_bytes=423701454 delta_bytes=134126662
//!   ratio=0.3166 keyframes=92 (gap_forced=91) keyframe_bytes=133955040
//!   deltas=199 patch_bytes=171622 patch_bytes_per_record=862
//!   changed_rows_total=4850
//! ```
//!
//! 423,701,454 → 134,126,662 bytes, a 289.6 MB saving, and the whole of what is
//! left is keyframes: 133,955,040 of the 134,126,662 bytes are the 92 whole
//! payloads the gap structure forces. A steady-state patch is **862 bytes**
//! against a 1,456,002-byte whole payload — 1/1,689 of it, and 208 of those 862
//! are the fixed `PPA1` authority trailer both sides pay. The lever on the
//! remaining 134 MB is therefore d41's post-gap pin, not its change predicate.
//!
//! Every one of the 291 records is also checked for reconstruction: the world
//! rebuilt from the delta stream is compared, record by record, against the
//! world rebuilt from whole payloads over the same reads, applying the merge
//! rules `world_fold::fold_special_projects` applies (replace on a non-delta
//! record, merge-and-tombstone on a delta one).
//!
//! MUTATIONS, both observed verbatim:
//!
//! * make the shared engine's change predicate always say "unchanged"
//!   (`.map(|old| old != row)` → `.map(|_old| false)` in
//!   `compact_projects_delta_rows`):
//!   "record 7 folded to a different world under delta emission"
//! * make the emit gate refuse to diff a clean read (`0 => true` → `0 => return
//!   read` in `compact_projects_apply_emit_mode`), i.e. reinstate whole-payload
//!   emission:
//!   "assertion `left == right` failed: exactly the first record and one record
//!    per own gap may be a keyframe / left: 291 / right: 92"
//!   with `ratio=0.9999` on the measurement line beside it.

use super::*;
use std::path::PathBuf;

const T26_PAYLOAD_BYTES: usize = 423_701_454;
const T26_DATA_RECORDS: usize = 291;

/// `runs/` is READ-ONLY capture evidence, is not tracked, and is absent from a
/// worktree checkout, so the test that needs it is `#[ignore]`d and this
/// PANICS rather than returning: a capture-evidence test that quietly passes
/// when its evidence is missing certifies nothing, and once invoked it must
/// either measure or fail.
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
    u32::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
    ])
}

fn i32le(bytes: &[u8], at: usize) -> i32 {
    u32le(bytes, at) as i32
}

fn u64le(bytes: &[u8], at: usize) -> u64 {
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(out)
}

/// One D41 record as it sits on the wire, decoded far enough to drive the
/// production emit path.
struct DecodedRecord {
    read: CompactProjectsRead,
    elapsed_wall_ms: u64,
    /// The bytes `compact_projects_data_payload` is responsible for: the whole
    /// payload up to the `PPA1` authority trailer the caller appends after it.
    encoder_owned: Vec<u8>,
}

fn take_string(payload: &[u8], cursor: &mut usize) -> String {
    let len = u16le(payload, *cursor) as usize;
    *cursor += 2;
    let out = String::from_utf8_lossy(&payload[*cursor..*cursor + len]).into_owned();
    *cursor += len;
    out
}

/// Decode one D41 `DOMAIN_DATA` payload into the read the encoder built it from.
///
/// Layout mirrors `compact_projects_data_payload` and the trailer appenders
/// beside it. The caller asserts the round trip, so a drift here is a test
/// failure and not a silently wrong measurement.
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
        let specialization_code = if deleted
            || flags & COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_PRESENT == 0
        {
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
            // `PPA1`, appended by the caller after
            // `compact_projects_data_payload` returns. Everything from here on
            // is outside the encoder this test round-trips.
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
        // The capture's breakthrough rows are decoded with their specialization
        // codes, so this is the same question over the same rows.
        breakthrough_specializations_resolved: true,
        // Lane IJ: no fixture here stages a cache shortfall.
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

/// The consumer's view, rebuilt exactly the way `world_fold::fold_special_
/// projects` rebuilds it: a non-delta record REPLACES the section, a delta
/// record merges carried rows over the retained map and removes tombstoned keys.
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

/// Walk the stream once, driving the production emit path over the real D41
/// sequence. Payloads are decoded one at a time — the whole D41 stream is
/// 424 MB and never needs to be resident.
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

                // §2.3: a rejected bracket for THIS domain drops its pin, so the
                // next read is a keyframe. Modelled for EVERY own gap, which is
                // the conservative bound on the delta stream's size.
                if gap_since_last_data && outcome.records > 1 {
                    tracker.clear();
                    outcome.gap_forced_keyframes += 1;
                }
                gap_since_last_data = false;

                // Both sides are driven from two identical decodes:
                // `CompactProjectsRead` is deliberately not `Clone`.
                let whole = decode_projects_payload(&payload).read;
                let mut source = decoded.read;
                // The repaired reader's status word. See the module header for
                // the per-record diagnostics that make every other PARTIAL
                // source measurably absent in this capture.
                source.status_flags &= !COMPACT_STATUS_PARTIAL_DATA;
                let emitted =
                    compact_projects_apply_emit_mode(source, D41EmitMode::Delta, &mut tracker);
                // The captured `whole_bytes` include the 208-byte `PPA1`
                // authority trailer the caller appends after
                // `compact_projects_data_payload` returns, and a patch record
                // carries it too — so the emitted side is charged the same
                // trailer, taken from the record it was decoded from rather
                // than assumed.
                let authority_trailer_len = payload.len() - decoded.encoder_owned.len();
                let emitted_len =
                    compact_projects_data_payload(&emitted, decoded.elapsed_wall_ms).len()
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

/// THE MEASUREMENT. Drives t26's real D41 sequence through the production emit
/// path and reports what the repaired domain costs.
///
/// This is the test that would have caught the regression: it asserts against
/// the whole-payload total, so a D41 that re-sends everything fails it whatever
/// the reason.
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
    // The keyframes dominate what is left, and they are set by the gap
    // structure rather than by the change rate. Half is a wide guard on a
    // measured 31%; it fails the moment the domain goes back to whole payloads.
    assert!(
        outcome.delta_bytes * 2 < outcome.whole_bytes,
        "delta emission must at least halve D41's payload: {} vs {}",
        outcome.delta_bytes,
        outcome.whole_bytes
    );
    // The steady-state claim, separate from the keyframe budget: a patch was
    // measured at 862 bytes against a 1,456,002-byte whole payload. 10 KB is a
    // wide guard that still fails on any regression to whole-ish patches.
    let patch_per_record = outcome.patch_bytes / outcome.delta_records.max(1);
    assert!(
        patch_per_record < 10_000,
        "a steady-state D41 patch must stay a few hundred bytes, got {patch_per_record}"
    );
}
