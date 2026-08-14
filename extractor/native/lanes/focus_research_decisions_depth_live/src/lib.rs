use std::collections::{BTreeMap, BTreeSet};

use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
    COMPACT_DOMAIN_FRAME, COMPACT_DOMAIN_PERF_BASE_LEN, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
    COMPACT_STATUS_FAILURE_MASK,
};
use recorder_host::domain_data_authority::{summarize_domain_data_payload, DomainDataContext};

pub mod a729_focus_item_producer;
#[cfg(windows)]
pub mod a729_focus_item_hook;
#[cfg(windows)]
mod a729_focus_item_inline;
#[cfg(windows)]
pub mod a729_research_ahead_hook;
pub mod national_focus_presentation;
pub mod xui09_research_contract;

pub const DOMAIN_ID: u16 = 45;
pub const DOMAIN_NAME: &str = "focus_research_decisions_depth_live";

const H4CR_MAGIC: &[u8; 4] = COMPACT_RECORD_MAGIC;
const H4CR_VERSION: u16 = COMPACT_RECORD_VERSION;
const H4CR_HEADER_LEN: usize = COMPACT_RECORD_HEADER_LEN;
const RECORD_TYPE_DOMAIN_PERF: u16 = COMPACT_RECORD_TYPE_DOMAIN_PERF;
const RECORD_TYPE_DOMAIN_DATA: u16 = COMPACT_RECORD_TYPE_DOMAIN_DATA;
const RECORD_TYPE_FRAME_END: u16 = COMPACT_RECORD_TYPE_FRAME_END;
const DOMAIN_FOCUS: u16 = 12;
const DOMAIN_TECH: u16 = 13;
const DOMAIN_DECISIONS: u16 = 16;
const DOMAIN_FRAME: u16 = COMPACT_DOMAIN_FRAME;
const DOMAIN_RC_PROBE: u16 = 250;
const SOURCE_DOMAIN_IDS: &[u16] = &[DOMAIN_FOCUS, DOMAIN_TECH, DOMAIN_DECISIONS];

const DEPTH_HEADER_LEN: usize = 48;
const RESEARCH_ETA_ROW_LEN: usize = 80;
const DECISION_STATUS_ROW_LEN: usize = 28;
const DECISION_TARGET_ROW_FIXED_LEN: usize = 16;
const BADGE_ROW_FIXED_LEN: usize = 20;

const DEPTH_STATUS_FOCUS_PAUSE_BADGED: u32 = 1;
const DEPTH_STATUS_DECISION_TARGETS_BADGED: u32 = 1 << 1;
const DEPTH_STATUS_DIRECT_EVENTS_DERIVED: u32 = 1 << 2;
const DEPTH_STATUS_FOCUS_PAUSE_EXACT: u32 = 1 << 3;
const DEPTH_STATUS_DECISION_TARGETS_EXACT: u32 = 1 << 4;
const DEPTH_STATUS_DECISION_COOLDOWNS_EXACT: u32 = 1 << 5;
const DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED: u32 = 1 << 6;
// Bits 0..=5 are the legacy depth-authority word carried by every schema.
// Bit 6 is a schema-v5-only statement about the independent direct-event
// trailer.  Keep the masks separate: recognizing the v5 bit must never cause
// a v1-v4 encoder to claim that a live direct-hook source was validated.
const DEPTH_STATUS_LEGACY_KNOWN_MASK: u32 = DEPTH_STATUS_FOCUS_PAUSE_BADGED
    | DEPTH_STATUS_DECISION_TARGETS_BADGED
    | DEPTH_STATUS_DIRECT_EVENTS_DERIVED
    | DEPTH_STATUS_FOCUS_PAUSE_EXACT
    | DEPTH_STATUS_DECISION_TARGETS_EXACT
    | DEPTH_STATUS_DECISION_COOLDOWNS_EXACT;
const DEPTH_STATUS_KNOWN_MASK: u32 =
    DEPTH_STATUS_LEGACY_KNOWN_MASK | DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED;

const BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED: u32 = 1;
const BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED: u32 = 2;
const BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE: u32 = 3;
const BADGE_KIND_MAX: u32 = BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE;

const BADGE_FLAG_FAIL_CLOSED: u32 = 1;
const BADGE_FLAG_DERIVED_DETERMINISTIC: u32 = 1 << 1;
const BADGE_FLAG_CURRENT_BUILD_PROBED: u32 = 1 << 2;
const BADGE_FLAG_KNOWN_MASK: u32 =
    BADGE_FLAG_FAIL_CLOSED | BADGE_FLAG_DERIVED_DETERMINISTIC | BADGE_FLAG_CURRENT_BUILD_PROBED;

const BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH: u32 = 1;
const BADGE_EVIDENCE_TIMED_DEFINITION_LAYOUT_REJECTED: u32 = 2;
const BADGE_EVIDENCE_COMPACT_FRAME_DELTA_DERIVED: u32 = 3;

const BADGE_DETAIL_FOCUS_PAUSE_FAIL_CLOSED: &str =
    "focus_pause_flag_fail_closed_no_paused_true_ground_truth";
const BADGE_DETAIL_DECISION_TARGET_FIELDS_FAIL_CLOSED: &str =
    "decision_target_fields_fail_closed_body_layout_unvalidated";
const BADGE_DETAIL_DIRECT_EVENT_HOOKS_UNAVAILABLE: &str =
    "direct_event_hooks_unavailable_transitions_derived_from_compact_frames";

const FOCUS_EVENT_COMPLETED_COUNT_INCREASED: u32 = 1;
const FOCUS_EVENT_CURRENT_STARTED: u32 = 2;
const FOCUS_EVENT_CURRENT_STOPPED: u32 = 3;
const FOCUS_EVENT_CURRENT_CHANGED: u32 = 4;
const FOCUS_EVENT_PROGRESS_REGRESSED: u32 = 5;
const FOCUS_EVENT_INVALID_BOTH_CURRENT_TYPES: u32 = 6;
const FOCUS_EVENT_STATE_SNAPSHOT: u32 = 7;

const FOCUS_KIND_NATIONAL: u32 = 1;
const FOCUS_KIND_CONTINUOUS: u32 = 2;

const FOCUS_FLAG_COMPLETED_TOKEN_INFERRED: u32 = 1;
const FOCUS_FLAG_COMPLETED_TOKEN_EXACT: u32 = 1 << 1;
const FOCUS_FLAG_PAUSED: u32 = 1 << 2;
const FOCUS_FLAG_PAUSED_PRESENT: u32 = 1 << 3;

const RESEARCH_FLAG_SAME_TECH: u32 = 1;
const RESEARCH_FLAG_POSITIVE_SPEED: u32 = 1 << 1;
const RESEARCH_FLAG_ETA_AVAILABLE: u32 = 1 << 2;
const RESEARCH_FLAG_TECH_ROLLOVER: u32 = 1 << 3;
const RESEARCH_FLAG_NEGATIVE_DELTA: u32 = 1 << 4;
const RESEARCH_FLAG_BASE_PRESENT: u32 = 1 << 5;

const DECISION_EVENT_STARTED: u32 = 1;
const DECISION_EVENT_STOPPED: u32 = 2;
const DECISION_EVENT_DAY_DELTA: u32 = 3;
const DECISION_EVENT_STATE_CHANGED: u32 = 4;
const DECISION_EVENT_STATUS_COUNTS_CHANGED: u32 = 5;
const DECISION_EVENT_RE_ENABLE_COOLDOWN_SNAPSHOT: u32 = 6;
const DECISION_EVENT_REMOVE_COOLDOWN_SNAPSHOT: u32 = 7;

const DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED: u32 = 1;
const DECISION_TARGET_FLAG_COUNTRY_TAG: u32 = 1;
const DECISION_TARGET_FLAG_RECORD_0X20: u32 = 1 << 1;
const DECISION_TARGET_FLAG_NUMERIC_ID: u32 = 1 << 2;
const DECISION_TARGET_FLAG_RECORD_0X24: u32 = 1 << 3;
const DECISION_TARGET_FLAG_KNOWN_MASK: u32 = DECISION_TARGET_FLAG_COUNTRY_TAG
    | DECISION_TARGET_FLAG_RECORD_0X20
    | DECISION_TARGET_FLAG_NUMERIC_ID
    | DECISION_TARGET_FLAG_RECORD_0X24;
const DECISION_TARGET_UNKNOWN_ID: u32 = u32::MAX;
/// Was `4096`, a guess at the vanilla-plus-DLC country count (vanilla defines
/// 439 tags). ENGINE-STRUCTURAL now: a country database id is admissible iff
/// `id < *(int32*)(db + 0x0C)` and that size is a signed `int`.
const DECISION_TARGET_MAX_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
/// The numeric decision target is a STATE identity, not an opaque number:
/// `TargetedDecisionTarget::Numeric` becomes `panel::Target::State(state_id)`
/// in `collector/src/a34_a40_evidence.rs`. So this was a state ceiling under a
/// role name -- `1_000_000` -- and a name-based census could not see it. It is
/// now the state id space's STRUCTURAL width
/// (`claim_39c989ce13504504353c`; `runs/ia_province_caps/SWEEP.md`).
const DECISION_TARGET_MAX_NUMERIC_ID: u32 = (i32::MAX as u32) - 1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DepthGateSummary {
    pub data_records: u64,
    pub perf_records: u64,
    pub frame_end_records: u64,
    pub certified_hours: u64,
    pub first_game_hour_raw: Option<i32>,
    pub last_game_hour_raw: Option<i32>,
    pub game_hours_advanced: i64,
    pub commanded_game_hour_advancements: i64,
    pub required_closed_frames: u64,
    pub status_flags_or: u32,
    pub fatal_status_flags: u32,
    pub focus_event_rows: u64,
    pub focus_completed_exact_rows: u64,
    pub focus_completed_inferred_rows: u64,
    pub focus_completed_unresolved_rows: u64,
    pub research_eta_rows: u64,
    pub decision_status_rows: u64,
    pub decision_event_rows: u64,
    pub decision_target_rows: u64,
    pub decision_country_target_rows: u64,
    pub decision_numeric_target_rows: u64,
    pub badge_rows: u64,
    pub focus_pause_badge_rows: u64,
    pub decision_target_badge_rows: u64,
    pub direct_event_badge_rows: u64,
    pub parse_errors: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DepthProof {
    pub depth_stream: Vec<u8>,
    pub source_bytes: usize,
    pub source_records: u64,
    pub source_frames: u64,
    pub first_game_hour_raw: Option<i32>,
    pub last_game_hour_raw: Option<i32>,
    pub game_hours_advanced: i32,
    pub skipped_game_hours: u32,
    pub focus_data_records: u64,
    pub tech_data_records: u64,
    pub decisions_data_records: u64,
    pub source_focus_rows: u64,
    pub source_research_slot_rows: u64,
    pub source_decision_country_rows: u64,
    pub source_timed_decision_rows: u64,
    pub source_targeted_timed_decision_rows: u64,
    pub depth_data_records: u64,
    pub focus_event_rows: u64,
    pub focus_completed_exact_rows: u64,
    pub focus_completed_inferred_rows: u64,
    pub focus_completed_unresolved_rows: u64,
    pub research_eta_rows: u64,
    pub decision_status_rows: u64,
    pub decision_event_rows: u64,
    pub decision_target_rows: u64,
    pub decision_country_target_rows: u64,
    pub decision_numeric_target_rows: u64,
    pub badge_rows: u64,
    pub focus_pause_badge_rows: u64,
    pub decision_target_badge_rows: u64,
    pub direct_event_badge_rows: u64,
    pub parse_errors: Vec<String>,
    pub depth_parse_errors: Vec<String>,
    sample_events: Vec<String>,
}

impl DepthProof {
    pub fn manifest_json(&self, source_stream: &str) -> String {
        let parse_errors = json_string_array(&self.parse_errors);
        let depth_parse_errors = json_string_array(&self.depth_parse_errors);
        format!(
            concat!(
                "{{\n",
                "  \"lane\": \"{lane}\",\n",
                "  \"provisional_domain_id\": {domain_id},\n",
                "  \"source_stream\": {source_stream},\n",
                "  \"source_bytes\": {source_bytes},\n",
                "  \"source_records\": {source_records},\n",
                "  \"source_frames\": {source_frames},\n",
                "  \"first_game_hour_raw\": {first_hour},\n",
                "  \"last_game_hour_raw\": {last_hour},\n",
                "  \"game_hours_advanced\": {advanced},\n",
                "  \"skipped_game_hours\": {skipped},\n",
                "  \"base_domain_data_records\": {{\n",
                "    \"focus_live\": {focus_records},\n",
                "    \"tech_live\": {tech_records},\n",
                "    \"decisions_live\": {decision_records}\n",
                "  }},\n",
                "  \"base_payload_rows\": {{\n",
                "    \"focus_country_rows\": {source_focus_rows},\n",
                "    \"active_research_slot_rows\": {source_research_slot_rows},\n",
                "    \"decision_country_rows\": {source_decision_country_rows},\n",
                "    \"timed_decision_rows\": {source_timed_decision_rows},\n",
                "    \"targeted_timed_decision_rows\": {source_targeted_timed_decision_rows}\n",
                "  }},\n",
                "  \"depth_output\": {{\n",
                "    \"bytes\": {depth_bytes},\n",
                "    \"data_records\": {depth_records},\n",
                "    \"focus_event_rows\": {focus_event_rows},\n",
                "    \"focus_completed_exact_rows\": {focus_completed_exact_rows},\n",
                "    \"focus_completed_inferred_rows\": {focus_completed_inferred_rows},\n",
                "    \"focus_completed_unresolved_rows\": {focus_completed_unresolved_rows},\n",
                "    \"research_eta_rows\": {research_eta_rows},\n",
                "    \"decision_status_rows\": {decision_status_rows},\n",
                "    \"decision_event_rows\": {decision_event_rows},\n",
                "    \"decision_target_rows\": {decision_target_rows},\n",
                "    \"decision_country_target_rows\": {decision_country_target_rows},\n",
                "    \"decision_numeric_target_rows\": {decision_numeric_target_rows},\n",
                "    \"badge_rows\": {badge_rows},\n",
                "    \"badge_rows_by_kind\": {{\n",
                "      \"focus_pause_fail_closed\": {focus_pause_badge_rows},\n",
                "      \"decision_target_fields_fail_closed\": {decision_target_badge_rows},\n",
                "      \"direct_event_hooks_unavailable\": {direct_event_badge_rows}\n",
                "    }}\n",
                "  }},\n",
                "  \"gate\": {{\n",
                "    \"source_parse_errors\": {parse_errors},\n",
                "    \"depth_parse_errors\": {depth_parse_errors},\n",
                "    \"ok\": {ok}\n",
                "  }}\n",
                "}}\n"
            ),
            lane = DOMAIN_NAME,
            domain_id = DOMAIN_ID,
            source_stream = json_str(source_stream),
            source_bytes = self.source_bytes,
            source_records = self.source_records,
            source_frames = self.source_frames,
            first_hour = json_i32(self.first_game_hour_raw),
            last_hour = json_i32(self.last_game_hour_raw),
            advanced = self.game_hours_advanced,
            skipped = self.skipped_game_hours,
            focus_records = self.focus_data_records,
            tech_records = self.tech_data_records,
            decision_records = self.decisions_data_records,
            source_focus_rows = self.source_focus_rows,
            source_research_slot_rows = self.source_research_slot_rows,
            source_decision_country_rows = self.source_decision_country_rows,
            source_timed_decision_rows = self.source_timed_decision_rows,
            source_targeted_timed_decision_rows = self.source_targeted_timed_decision_rows,
            depth_bytes = self.depth_stream.len(),
            depth_records = self.depth_data_records,
            focus_event_rows = self.focus_event_rows,
            focus_completed_exact_rows = self.focus_completed_exact_rows,
            focus_completed_inferred_rows = self.focus_completed_inferred_rows,
            focus_completed_unresolved_rows = self.focus_completed_unresolved_rows,
            research_eta_rows = self.research_eta_rows,
            decision_status_rows = self.decision_status_rows,
            decision_event_rows = self.decision_event_rows,
            decision_target_rows = self.decision_target_rows,
            decision_country_target_rows = self.decision_country_target_rows,
            decision_numeric_target_rows = self.decision_numeric_target_rows,
            badge_rows = self.badge_rows,
            focus_pause_badge_rows = self.focus_pause_badge_rows,
            decision_target_badge_rows = self.decision_target_badge_rows,
            direct_event_badge_rows = self.direct_event_badge_rows,
            parse_errors = parse_errors,
            depth_parse_errors = depth_parse_errors,
            ok = self.parse_errors.is_empty() && self.depth_parse_errors.is_empty(),
        )
    }

    pub fn sample_events_jsonl(&self) -> String {
        let mut out = String::new();
        for line in &self.sample_events {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusState {
    pub tag_db_id: u32,
    pub completed_count: Option<u32>,
    pub progress_raw: Option<i32>,
    pub current: Option<String>,
    pub current_continuous: Option<String>,
    pub completed_tokens: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchSlotState {
    pub tag_db_id: u32,
    pub slot_index: u32,
    pub tech_handle_raw: u32,
    pub tech_def_index_raw: u32,
    pub progress_raw: u32,
    pub live_progress_raw: Option<i64>,
    pub base_required_points_raw: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionCountryState {
    pub tag_db_id: u32,
    pub active_timed_count: u32,
    pub targeted_count: u32,
    pub targeted_timed_count: u32,
    pub decisions_taken_bucket_count: u32,
    pub re_enable_count: u32,
    pub timed_live: Vec<TimedDecisionState>,
    pub targeted_timed_live: Vec<TargetedTimedDecisionState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedDecisionState {
    pub tag_db_id: u32,
    pub decision: String,
    pub days: i32,
    pub state: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedTimedDecisionState {
    pub tag_db_id: u32,
    pub decision: String,
    pub target_tag_db_id: Option<u32>,
    pub target_numeric_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusEventRow {
    pub tag_db_id: u32,
    pub kind: u32,
    pub focus_kind: u32,
    pub flags: u32,
    pub previous_completed_count: u32,
    pub current_completed_count: u32,
    pub previous_progress_raw: i32,
    pub current_progress_raw: i32,
    pub completed_delta: u32,
    pub from_token: Option<String>,
    pub to_token: Option<String>,
    pub completed_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchEtaRow {
    pub tag_db_id: u32,
    pub slot_index: u32,
    pub flags: u32,
    pub hour_delta: u32,
    pub previous_tech_handle: u32,
    pub current_tech_handle: u32,
    pub previous_tech_def_index: u32,
    pub current_tech_def_index: u32,
    pub previous_live_progress_raw: i64,
    pub current_live_progress_raw: i64,
    pub delta_live_progress_raw: i64,
    pub speed_raw_per_hour: i64,
    pub base_required_points_raw: i64,
    pub eta_hours: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionStatusRow {
    pub tag_db_id: u32,
    pub active_timed_count: u32,
    pub targeted_count: u32,
    pub targeted_timed_count: u32,
    pub decisions_taken_bucket_count: u32,
    pub re_enable_count: u32,
    pub timed_live_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionEventRow {
    pub tag_db_id: u32,
    pub kind: u32,
    pub flags: u32,
    pub previous_days: i32,
    pub current_days: i32,
    pub previous_state: i32,
    pub current_state: i32,
    pub decision: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionTargetRow {
    pub tag_db_id: u32,
    pub target_id: u32,
    pub flags: u32,
    pub source_kind: u32,
    pub decision: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadgeRow {
    pub kind: u32,
    pub flags: u32,
    pub source_domain_id: u32,
    pub affected_count: u32,
    pub evidence_code: u32,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DepthFrameRows {
    pub status_flags: u32,
    pub focus_events: Vec<FocusEventRow>,
    pub research_eta: Vec<ResearchEtaRow>,
    pub decision_status: Vec<DecisionStatusRow>,
    pub decision_events: Vec<DecisionEventRow>,
    pub decision_targets: Vec<DecisionTargetRow>,
    pub badges: Vec<BadgeRow>,
    pub observed_focus_rows: u32,
    pub observed_research_slot_rows: u32,
    pub observed_decision_rows: u32,
    pub observed_timed_decision_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FocusTrackerState {
    game_hour_raw: i32,
    state: FocusState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResearchTrackerState {
    game_hour_raw: i32,
    state: ResearchSlotState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecisionTrackerState {
    state: DecisionCountryState,
}

#[derive(Default)]
struct Deriver {
    focus_by_tag: BTreeMap<u32, FocusTrackerState>,
    research_by_slot: BTreeMap<(u32, u32), ResearchTrackerState>,
    decision_by_tag: BTreeMap<u32, DecisionTrackerState>,
    pending: DepthFrameRows,
    samples: Vec<String>,
    sample_limit: usize,
}

impl Deriver {
    fn new() -> Self {
        Self {
            sample_limit: 80,
            ..Self::default()
        }
    }

    fn apply_focus_rows(&mut self, game_hour_raw: i32, rows: Vec<FocusState>) {
        self.pending.observed_focus_rows = self
            .pending
            .observed_focus_rows
            .saturating_add(rows.len().min(u32::MAX as usize) as u32);
        for state in rows {
            let tag_db_id = state.tag_db_id;
            if state.current.is_some() && state.current_continuous.is_some() {
                self.push_focus_event(
                    game_hour_raw,
                    FocusEventRow {
                        tag_db_id,
                        kind: FOCUS_EVENT_INVALID_BOTH_CURRENT_TYPES,
                        focus_kind: 0,
                        flags: 0,
                        previous_completed_count: u32::MAX,
                        current_completed_count: state.completed_count.unwrap_or(u32::MAX),
                        previous_progress_raw: 0,
                        current_progress_raw: state.progress_raw.unwrap_or(0),
                        completed_delta: 0,
                        from_token: state.current.clone(),
                        to_token: state.current_continuous.clone(),
                        completed_token: None,
                    },
                );
            }
            if let Some(previous) = self.focus_by_tag.get(&tag_db_id).cloned() {
                self.derive_focus_events(game_hour_raw, &previous.state, &state);
            }
            self.focus_by_tag.insert(
                tag_db_id,
                FocusTrackerState {
                    game_hour_raw,
                    state,
                },
            );
        }
    }

    fn derive_focus_events(
        &mut self,
        game_hour_raw: i32,
        previous: &FocusState,
        current: &FocusState,
    ) {
        let prev_active = active_focus(previous);
        let curr_active = active_focus(current);
        let prev_completed = previous.completed_count.unwrap_or(u32::MAX);
        let curr_completed = current.completed_count.unwrap_or(u32::MAX);
        if prev_completed != u32::MAX
            && curr_completed != u32::MAX
            && curr_completed > prev_completed
        {
            let completed_delta = curr_completed.saturating_sub(prev_completed);
            let exact_completed = current
                .completed_tokens
                .iter()
                .find(|token| !previous.completed_tokens.contains(*token))
                .cloned();
            let inferred_completed = exact_completed
                .clone()
                .or_else(|| prev_active.as_ref().map(|(_, token)| token.clone()));
            let mut flags = 0;
            if exact_completed.is_some() {
                flags |= FOCUS_FLAG_COMPLETED_TOKEN_EXACT;
            } else if inferred_completed.is_some() {
                flags |= FOCUS_FLAG_COMPLETED_TOKEN_INFERRED;
            }
            self.push_focus_event(
                game_hour_raw,
                FocusEventRow {
                    tag_db_id: current.tag_db_id,
                    kind: FOCUS_EVENT_COMPLETED_COUNT_INCREASED,
                    focus_kind: prev_active
                        .as_ref()
                        .map(|(kind, _)| *kind)
                        .or_else(|| curr_active.as_ref().map(|(kind, _)| *kind))
                        .unwrap_or(0),
                    flags,
                    previous_completed_count: prev_completed,
                    current_completed_count: curr_completed,
                    previous_progress_raw: previous.progress_raw.unwrap_or(0),
                    current_progress_raw: current.progress_raw.unwrap_or(0),
                    completed_delta,
                    from_token: prev_active.as_ref().map(|(_, token)| token.clone()),
                    to_token: curr_active.as_ref().map(|(_, token)| token.clone()),
                    completed_token: inferred_completed,
                },
            );
        }

        if prev_active != curr_active {
            let kind = match (&prev_active, &curr_active) {
                (None, Some(_)) => FOCUS_EVENT_CURRENT_STARTED,
                (Some(_), None) => FOCUS_EVENT_CURRENT_STOPPED,
                (Some(_), Some(_)) => FOCUS_EVENT_CURRENT_CHANGED,
                (None, None) => 0,
            };
            if kind != 0 {
                self.push_focus_event(
                    game_hour_raw,
                    FocusEventRow {
                        tag_db_id: current.tag_db_id,
                        kind,
                        focus_kind: curr_active
                            .as_ref()
                            .map(|(kind, _)| *kind)
                            .or_else(|| prev_active.as_ref().map(|(kind, _)| *kind))
                            .unwrap_or(0),
                        flags: 0,
                        previous_completed_count: prev_completed,
                        current_completed_count: curr_completed,
                        previous_progress_raw: previous.progress_raw.unwrap_or(0),
                        current_progress_raw: current.progress_raw.unwrap_or(0),
                        completed_delta: curr_completed.saturating_sub(prev_completed),
                        from_token: prev_active.as_ref().map(|(_, token)| token.clone()),
                        to_token: curr_active.as_ref().map(|(_, token)| token.clone()),
                        completed_token: None,
                    },
                );
            }
        }

        if prev_active.is_some()
            && prev_active == curr_active
            && current.progress_raw.unwrap_or(0) < previous.progress_raw.unwrap_or(0)
        {
            self.push_focus_event(
                game_hour_raw,
                FocusEventRow {
                    tag_db_id: current.tag_db_id,
                    kind: FOCUS_EVENT_PROGRESS_REGRESSED,
                    focus_kind: curr_active.as_ref().map(|(kind, _)| *kind).unwrap_or(0),
                    flags: 0,
                    previous_completed_count: prev_completed,
                    current_completed_count: curr_completed,
                    previous_progress_raw: previous.progress_raw.unwrap_or(0),
                    current_progress_raw: current.progress_raw.unwrap_or(0),
                    completed_delta: 0,
                    from_token: prev_active.as_ref().map(|(_, token)| token.clone()),
                    to_token: curr_active.as_ref().map(|(_, token)| token.clone()),
                    completed_token: None,
                },
            );
        }
    }

    fn push_focus_event(&mut self, game_hour_raw: i32, row: FocusEventRow) {
        self.sample(
            game_hour_raw,
            "focus",
            row.kind,
            row.tag_db_id,
            row.completed_token
                .as_deref()
                .or(row.to_token.as_deref())
                .or(row.from_token.as_deref()),
        );
        self.pending.focus_events.push(row);
    }

    fn apply_research_rows(&mut self, game_hour_raw: i32, rows: Vec<ResearchSlotState>) {
        self.pending.observed_research_slot_rows = self
            .pending
            .observed_research_slot_rows
            .saturating_add(rows.len().min(u32::MAX as usize) as u32);
        for state in rows {
            let key = (state.tag_db_id, state.slot_index);
            if let Some(previous) = self.research_by_slot.get(&key).cloned() {
                if let Some(row) = derive_research_eta_row(
                    previous.game_hour_raw,
                    &previous.state,
                    game_hour_raw,
                    &state,
                ) {
                    self.sample(game_hour_raw, "research", row.flags, row.tag_db_id, None);
                    self.pending.research_eta.push(row);
                }
            }
            self.research_by_slot.insert(
                key,
                ResearchTrackerState {
                    game_hour_raw,
                    state,
                },
            );
        }
    }

    fn apply_decision_rows(&mut self, game_hour_raw: i32, rows: Vec<DecisionCountryState>) {
        self.pending.observed_decision_rows = self
            .pending
            .observed_decision_rows
            .saturating_add(rows.len().min(u32::MAX as usize) as u32);
        self.pending.observed_timed_decision_rows =
            self.pending.observed_timed_decision_rows.saturating_add(
                rows.iter()
                    .map(|row| row.timed_live.len())
                    .sum::<usize>()
                    .min(u32::MAX as usize) as u32,
            );
        for state in rows {
            let status = DecisionStatusRow {
                tag_db_id: state.tag_db_id,
                active_timed_count: state.active_timed_count,
                targeted_count: state.targeted_count,
                targeted_timed_count: state.targeted_timed_count,
                decisions_taken_bucket_count: state.decisions_taken_bucket_count,
                re_enable_count: state.re_enable_count,
                timed_live_count: state.timed_live.len().min(u32::MAX as usize) as u32,
            };
            self.pending.decision_status.push(status);
            for targeted in &state.targeted_timed_live {
                if let Some(target_tag_db_id) = targeted.target_tag_db_id {
                    self.pending.decision_targets.push(DecisionTargetRow {
                        tag_db_id: state.tag_db_id,
                        target_id: target_tag_db_id,
                        flags: DECISION_TARGET_FLAG_COUNTRY_TAG | DECISION_TARGET_FLAG_RECORD_0X20,
                        source_kind: DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED,
                        decision: Some(targeted.decision.clone()),
                    });
                } else if let Some(target_numeric_id) = targeted.target_numeric_id {
                    self.pending.decision_targets.push(DecisionTargetRow {
                        tag_db_id: state.tag_db_id,
                        target_id: target_numeric_id,
                        flags: DECISION_TARGET_FLAG_NUMERIC_ID | DECISION_TARGET_FLAG_RECORD_0X24,
                        source_kind: DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED,
                        decision: Some(targeted.decision.clone()),
                    });
                }
            }
            if let Some(previous) = self.decision_by_tag.get(&state.tag_db_id).cloned() {
                self.derive_decision_events(game_hour_raw, &previous.state, &state);
            }
            self.decision_by_tag
                .insert(state.tag_db_id, DecisionTrackerState { state });
        }
    }

    fn derive_decision_events(
        &mut self,
        game_hour_raw: i32,
        previous: &DecisionCountryState,
        current: &DecisionCountryState,
    ) {
        if decision_counts(previous) != decision_counts(current) {
            self.push_decision_event(
                game_hour_raw,
                DecisionEventRow {
                    tag_db_id: current.tag_db_id,
                    kind: DECISION_EVENT_STATUS_COUNTS_CHANGED,
                    flags: 0,
                    previous_days: 0,
                    current_days: 0,
                    previous_state: 0,
                    current_state: 0,
                    decision: None,
                },
            );
        }

        let previous_timed = timed_decision_map(previous);
        let current_timed = timed_decision_map(current);
        for (decision, curr) in &current_timed {
            match previous_timed.get(decision) {
                None => self.push_decision_event(
                    game_hour_raw,
                    DecisionEventRow {
                        tag_db_id: current.tag_db_id,
                        kind: DECISION_EVENT_STARTED,
                        flags: 0,
                        previous_days: -1,
                        current_days: curr.days,
                        previous_state: -1,
                        current_state: curr.state,
                        decision: Some(decision.clone()),
                    },
                ),
                Some(prev) => {
                    if prev.days != curr.days {
                        self.push_decision_event(
                            game_hour_raw,
                            DecisionEventRow {
                                tag_db_id: current.tag_db_id,
                                kind: DECISION_EVENT_DAY_DELTA,
                                flags: 0,
                                previous_days: prev.days,
                                current_days: curr.days,
                                previous_state: prev.state,
                                current_state: curr.state,
                                decision: Some(decision.clone()),
                            },
                        );
                    }
                    if prev.state != curr.state {
                        self.push_decision_event(
                            game_hour_raw,
                            DecisionEventRow {
                                tag_db_id: current.tag_db_id,
                                kind: DECISION_EVENT_STATE_CHANGED,
                                flags: 0,
                                previous_days: prev.days,
                                current_days: curr.days,
                                previous_state: prev.state,
                                current_state: curr.state,
                                decision: Some(decision.clone()),
                            },
                        );
                    }
                }
            }
        }
        for (decision, prev) in &previous_timed {
            if !current_timed.contains_key(decision) {
                self.push_decision_event(
                    game_hour_raw,
                    DecisionEventRow {
                        tag_db_id: current.tag_db_id,
                        kind: DECISION_EVENT_STOPPED,
                        flags: 0,
                        previous_days: prev.days,
                        current_days: -1,
                        previous_state: prev.state,
                        current_state: -1,
                        decision: Some(decision.clone()),
                    },
                );
            }
        }
    }

    fn push_decision_event(&mut self, game_hour_raw: i32, row: DecisionEventRow) {
        self.sample(
            game_hour_raw,
            "decision",
            row.kind,
            row.tag_db_id,
            row.decision.as_deref(),
        );
        self.pending.decision_events.push(row);
    }

    fn take_pending(&mut self) -> DepthFrameRows {
        let mut rows = std::mem::take(&mut self.pending);
        apply_required_badges(&mut rows);
        rows
    }

    fn sample(
        &mut self,
        game_hour_raw: i32,
        family: &str,
        kind: u32,
        tag_db_id: u32,
        token: Option<&str>,
    ) {
        if self.samples.len() >= self.sample_limit {
            return;
        }
        self.samples.push(format!(
            "{{\"game_hour_raw\":{game_hour_raw},\"family\":{},\"kind\":{kind},\"tag_db_id\":{tag_db_id},\"token\":{}}}",
            json_str(family),
            token.map(json_str).unwrap_or_else(|| "null".to_string())
        ));
    }
}

fn apply_required_badges(rows: &mut DepthFrameRows) {
    // This helper builds the legacy derived/snapshot proof and its explicit
    // unavailable badges.  Only the collector's authenticated schema-v5 path
    // may set DIRECT_HOOK_SOURCE_VALIDATED and remove the direct-hook badge.
    rows.status_flags |= DEPTH_STATUS_LEGACY_KNOWN_MASK;
    rows.status_flags &= !DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED;
    push_badge_if_missing(
        rows,
        BadgeRow {
            kind: BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED,
            flags: BADGE_FLAG_FAIL_CLOSED | BADGE_FLAG_CURRENT_BUILD_PROBED,
            source_domain_id: DOMAIN_FOCUS as u32,
            affected_count: rows.observed_focus_rows,
            evidence_code: BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH,
            detail: Some(BADGE_DETAIL_FOCUS_PAUSE_FAIL_CLOSED.to_string()),
        },
    );
    push_badge_if_missing(
        rows,
        BadgeRow {
            kind: BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED,
            flags: BADGE_FLAG_FAIL_CLOSED | BADGE_FLAG_CURRENT_BUILD_PROBED,
            source_domain_id: DOMAIN_DECISIONS as u32,
            affected_count: decision_target_body_count(rows),
            evidence_code: BADGE_EVIDENCE_TIMED_DEFINITION_LAYOUT_REJECTED,
            detail: Some(BADGE_DETAIL_DECISION_TARGET_FIELDS_FAIL_CLOSED.to_string()),
        },
    );
    push_badge_if_missing(
        rows,
        BadgeRow {
            kind: BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE,
            flags: BADGE_FLAG_FAIL_CLOSED | BADGE_FLAG_DERIVED_DETERMINISTIC,
            source_domain_id: DOMAIN_FRAME as u32,
            affected_count: rows
                .focus_events
                .len()
                .saturating_add(rows.decision_events.len())
                .min(u32::MAX as usize) as u32,
            evidence_code: BADGE_EVIDENCE_COMPACT_FRAME_DELTA_DERIVED,
            detail: Some(BADGE_DETAIL_DIRECT_EVENT_HOOKS_UNAVAILABLE.to_string()),
        },
    );
}

fn push_badge_if_missing(rows: &mut DepthFrameRows, badge: BadgeRow) {
    if !rows.badges.iter().any(|row| row.kind == badge.kind) {
        rows.badges.push(badge);
    }
}

fn decision_target_body_count(rows: &DepthFrameRows) -> u32 {
    rows.decision_status.iter().fold(0u32, |total, row| {
        total
            .saturating_add(row.targeted_count)
            .saturating_add(row.targeted_timed_count)
            .saturating_add(row.decisions_taken_bucket_count)
            .saturating_add(row.re_enable_count)
    })
}

#[derive(Debug, Default)]
struct AuthoritativeStreamValidation {
    record_count: u64,
    owned_perf_records: u64,
    frame_end_records: u64,
    frame_hours: Vec<i32>,
    perf_status_by_hour: BTreeMap<i32, u32>,
    perf_status_by_hour_domain: BTreeMap<(i32, u16), u32>,
    data_status_by_hour_domain: BTreeMap<(i32, u16), Option<u32>>,
    frame_status_by_hour: BTreeMap<i32, u32>,
    status_flags_or: u32,
    fatal_status_flags: u32,
    errors: Vec<String>,
}

fn validate_authoritative_stream(
    raw: &[u8],
    owned_domains: &[u16],
    require_owned_schema_v2: bool,
) -> AuthoritativeStreamValidation {
    let mut result = AuthoritativeStreamValidation::default();
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut final_record_is_certified_frame_end = false;
    let mut data_counts = BTreeMap::<(i32, u16), u32>::new();
    let mut perf_counts = BTreeMap::<(i32, u16), u32>::new();
    let mut observed_perf_counts_by_hour = BTreeMap::<i32, u64>::new();

    if raw.is_empty() {
        result.errors.push("stream is empty".to_string());
    }

    while offset < raw.len() {
        final_record_is_certified_frame_end = false;
        let record = match parse_h4cr_record(raw, offset) {
            Ok(record) => record,
            Err(error) => {
                result.errors.push(error);
                break;
            }
        };
        result.record_count = result.record_count.saturating_add(1);

        match previous_sequence {
            None if record.sequence != 1 => result.errors.push(format!(
                "offset {offset}: sequence {} does not start at 1 (stream prefix is missing)",
                record.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if record.sequence != expected => result.errors.push(format!(
                    "offset {offset}: sequence discontinuity {previous} -> {} (expected {expected})",
                    record.sequence
                )),
                None => result.errors.push(format!(
                    "offset {offset}: sequence space exhausted after {previous}; wrapping is forbidden"
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(record.sequence);

        let semantics_ok = match validate_compact_record_semantics(
            record.domain_id,
            record.record_type,
            record.record_flags,
            record.payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                result.errors.push(format!("offset {offset}: {error}"));
                false
            }
        };

        match record.record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                if record.domain_id != DOMAIN_RC_PROBE {
                    *observed_perf_counts_by_hour
                        .entry(record.game_hour_raw)
                        .or_default() += 1;
                }
                if let Some(open) = open_frame_hour {
                    if record.game_hour_raw != open {
                        result.errors.push(format!(
                            "offset {offset}: DOMAIN_PERF hour {} does not match open DATA frame hour {open}",
                            record.game_hour_raw
                        ));
                    }
                } else {
                    validate_next_source_hour(
                        &mut result.errors,
                        last_closed_hour,
                        record.game_hour_raw,
                        "DOMAIN_PERF",
                        offset,
                    );
                    match pending_perf_hour {
                        Some(pending) if pending != record.game_hour_raw => {
                            result.errors.push(format!(
                                "offset {offset}: DOMAIN_PERF hour {} conflicts with pending frame hour {pending}",
                                record.game_hour_raw
                            ));
                        }
                        None => pending_perf_hour = Some(record.game_hour_raw),
                        _ => {}
                    }
                }

                if owned_domains.contains(&record.domain_id) {
                    result.owned_perf_records = result.owned_perf_records.saturating_add(1);
                    let count = perf_counts
                        .entry((record.game_hour_raw, record.domain_id))
                        .or_default();
                    *count = count.saturating_add(1);
                    if semantics_ok {
                        let status_flags = read_u32_le(record.payload, 32);
                        result.status_flags_or |= status_flags;
                        *result
                            .perf_status_by_hour
                            .entry(record.game_hour_raw)
                            .or_default() |= status_flags;
                        result
                            .perf_status_by_hour_domain
                            .insert((record.game_hour_raw, record.domain_id), status_flags);
                        let unknown = compact_status_unknown_flags_v1(status_flags);
                        if unknown != 0 {
                            result.errors.push(format!(
                                "offset {offset}: domain {} PERF carries unknown v1 status flags {unknown:#010x}",
                                record.domain_id
                            ));
                        }
                        let fatal = compact_status_failure_flags_after_expected_partial_tolerance(
                            record.domain_id,
                            status_flags,
                        );
                        result.fatal_status_flags |= fatal;
                        if fatal != 0 {
                            result.errors.push(format!(
                                "offset {offset}: domain {} PERF carries fatal status flags {fatal:#010x} after canonical lane-local normalization",
                                record.domain_id
                            ));
                        }
                    }
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
                if let Some(open) = open_frame_hour {
                    if record.game_hour_raw != open {
                        result.errors.push(format!(
                            "offset {offset}: DOMAIN_DATA hour {} encountered before FRAME_END closed open frame hour {open}",
                            record.game_hour_raw
                        ));
                    }
                } else {
                    validate_next_source_hour(
                        &mut result.errors,
                        last_closed_hour,
                        record.game_hour_raw,
                        "DOMAIN_DATA",
                        offset,
                    );
                    if let Some(pending) = pending_perf_hour {
                        if pending != record.game_hour_raw {
                            result.errors.push(format!(
                                "offset {offset}: DOMAIN_DATA hour {} conflicts with pending PERF hour {pending}",
                                record.game_hour_raw
                            ));
                        }
                    }
                    open_frame_hour = Some(record.game_hour_raw);
                }
                if owned_domains.contains(&record.domain_id) {
                    let count = data_counts
                        .entry((record.game_hour_raw, record.domain_id))
                        .or_default();
                    *count = count.saturating_add(1);
                    if require_owned_schema_v2
                        && record.record_flags
                            & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
                            == 0
                    {
                        result.errors.push(format!(
                            "offset {offset}: production-owned domain {} DATA is missing the canonical schema-v2 record flag",
                            record.domain_id
                        ));
                    }
                    if semantics_ok {
                        if record.domain_id == DOMAIN_ID
                            && record.record_flags
                                & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
                                == 0
                        {
                            // Unflagged D45 is the lane-local v1 proof wire,
                            // whose first word is decision_target_count. The
                            // shared production decoder's legacy v1 predates
                            // those target rows, so validate this proof-only
                            // form with the lane decoder. Production FRD2 must
                            // always take the shared flag-aware path below.
                            let local = decode_depth_payload(offset, record.payload);
                            if local.parse_errors.is_empty() {
                                result
                                    .data_status_by_hour_domain
                                    .insert((record.game_hour_raw, record.domain_id), None);
                            } else {
                                result.errors.extend(local.parse_errors);
                            }
                        } else {
                            match summarize_domain_data_payload(
                            record.domain_id,
                            record.payload,
                            DomainDataContext {
                                stream_offset: offset,
                                record_flags: record.record_flags,
                                sequence: record.sequence,
                                header_hour_raw: record.game_hour_raw,
                                session_header_identity: None,
                            },
                            ) {
                                Ok(authority) => {
                                    result.data_status_by_hour_domain.insert(
                                        (record.game_hour_raw, record.domain_id),
                                        authority.status_flags,
                                    );
                                }
                                Err(error)
                                    if record.domain_id == DOMAIN_DECISIONS
                                        && error.errors.len() == 1
                                        && error.errors[0]
                                            .contains("decisions data payload has")
                                        && error.errors[0].contains("trailing byte(s)")
                                        && decode_decisions_payload(record.payload).is_ok() =>
                                {
                                    // D16's lane-local target extension is newer
                                    // than the base shared D16 authority. Accept
                                    // only the shared decoder's sole trailing-byte
                                    // diagnostic after the extension decoder has
                                    // independently consumed the entire suffix.
                                    result.data_status_by_hour_domain.insert(
                                        (record.game_hour_raw, record.domain_id),
                                        error.partial_authority.status_flags,
                                    );
                                }
                                Err(error) => result.errors.push(format!(
                                    "offset {offset}: domain {} DATA failed shared authority decode: {error}",
                                    record.domain_id
                                )),
                            }
                        }
                    }
                }
            }
            RECORD_TYPE_FRAME_END => {
                result.frame_end_records = result.frame_end_records.saturating_add(1);
                let mut certified = semantics_ok;
                match open_frame_hour {
                    Some(open) if open == record.game_hour_raw => {}
                    Some(open) => {
                        certified = false;
                        result.errors.push(format!(
                            "offset {offset}: FRAME_END hour {} does not close open DATA frame hour {open}",
                            record.game_hour_raw
                        ));
                    }
                    None => {
                        certified = false;
                        result.errors.push(format!(
                            "offset {offset}: FRAME_END hour {} has no open DATA frame",
                            record.game_hour_raw
                        ));
                    }
                }
                if pending_perf_hour.is_some_and(|pending| pending != record.game_hour_raw) {
                    certified = false;
                    result.errors.push(format!(
                        "offset {offset}: FRAME_END hour {} conflicts with pending PERF hour {}",
                        record.game_hour_raw,
                        pending_perf_hour.expect("checked Some")
                    ));
                }
                if !next_source_hour_is_valid(last_closed_hour, record.game_hour_raw) {
                    certified = false;
                    validate_next_source_hour(
                        &mut result.errors,
                        last_closed_hour,
                        record.game_hour_raw,
                        "FRAME_END",
                        offset,
                    );
                }

                for domain_id in owned_domains {
                    let data_count = data_counts
                        .get(&(record.game_hour_raw, *domain_id))
                        .copied()
                        .unwrap_or(0);
                    let perf_count = perf_counts
                        .get(&(record.game_hour_raw, *domain_id))
                        .copied()
                        .unwrap_or(0);
                    if data_count != 1 {
                        certified = false;
                        result.errors.push(format!(
                            "game_hour_raw {}: owned domain {domain_id} DATA record count {data_count} != 1",
                            record.game_hour_raw
                        ));
                    }
                    if perf_count != 1 {
                        certified = false;
                        result.errors.push(format!(
                            "game_hour_raw {}: owned domain {domain_id} PERF record count {perf_count} != 1",
                            record.game_hour_raw
                        ));
                    }
                    if data_count == 1 && perf_count == 1 {
                        if let Some(Some(data_status)) = result
                            .data_status_by_hour_domain
                            .get(&(record.game_hour_raw, *domain_id))
                            .copied()
                        {
                            let perf_status = result
                                .perf_status_by_hour_domain
                                .get(&(record.game_hour_raw, *domain_id))
                                .copied();
                            if perf_status != Some(data_status) {
                                certified = false;
                                result.errors.push(format!(
                                    "game_hour_raw {}: owned domain {domain_id} DATA status {data_status:#010x} does not match PERF status {:?}",
                                    record.game_hour_raw,
                                    perf_status.map(|value| format!("{value:#010x}"))
                                ));
                            }
                            let unknown = compact_status_unknown_flags_v1(data_status);
                            if unknown != 0 {
                                certified = false;
                                result.errors.push(format!(
                                    "game_hour_raw {}: owned domain {domain_id} DATA carries unknown v1 status flags {unknown:#010x}",
                                    record.game_hour_raw
                                ));
                            }
                            let fatal =
                                compact_status_failure_flags_after_expected_partial_tolerance(
                                    *domain_id,
                                    data_status,
                                );
                            if fatal != 0 {
                                certified = false;
                                result.errors.push(format!(
                                    "game_hour_raw {}: owned domain {domain_id} DATA carries fatal status flags {fatal:#010x}",
                                    record.game_hour_raw
                                ));
                            }
                        } else if require_owned_schema_v2 {
                            certified = false;
                            result.errors.push(format!(
                                "game_hour_raw {}: production-owned domain {domain_id} DATA lacks compact status authority; DATA/PERF status equality cannot be certified",
                                record.game_hour_raw
                            ));
                        }
                    }
                }

                if record.payload.len() == COMPACT_FRAME_END_PAYLOAD_LEN {
                    let emitted_domains = read_u32_le(record.payload, 16);
                    let scheduled_domains = read_u32_le(record.payload, 20);
                    let frame_status = read_u32_le(record.payload, 24);
                    let reserved = read_u32_le(record.payload, 28);
                    let observed_perf_records = observed_perf_counts_by_hour
                        .get(&record.game_hour_raw)
                        .copied()
                        .unwrap_or(0);
                    if u64::from(emitted_domains) != observed_perf_records {
                        certified = false;
                        result.errors.push(format!(
                            "offset {offset}: FRAME_END emitted_domains {emitted_domains} != independently observed DOMAIN_PERF count {observed_perf_records}"
                        ));
                    }
                    if scheduled_domains != emitted_domains {
                        certified = false;
                        result.errors.push(format!(
                            "offset {offset}: FRAME_END scheduled_domains {scheduled_domains} != emitted_domains {emitted_domains}"
                        ));
                    }
                    if reserved != 0 {
                        certified = false;
                        result.errors.push(format!(
                            "offset {offset}: FRAME_END reserved word is nonzero ({reserved:#010x})"
                        ));
                    }
                    let unknown = compact_status_unknown_flags_v1(frame_status);
                    if unknown != 0 {
                        certified = false;
                        result.errors.push(format!(
                            "offset {offset}: FRAME_END status contains unknown v1 bits {unknown:#010x}"
                        ));
                    }
                    result.status_flags_or |= frame_status;
                    result
                        .frame_status_by_hour
                        .insert(record.game_hour_raw, frame_status);
                    let fatal = frame_status & COMPACT_STATUS_FAILURE_MASK;
                    result.fatal_status_flags |= fatal;
                    if fatal != 0 {
                        certified = false;
                        result.errors.push(format!(
                            "offset {offset}: FRAME_END carries fatal global status flags {fatal:#010x}"
                        ));
                    }
                }

                if open_frame_hour == Some(record.game_hour_raw) {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(record.game_hour_raw);
                    result.frame_hours.push(record.game_hour_raw);
                }
                final_record_is_certified_frame_end = certified;
            }
            _ => {}
        }

        offset = record.next_offset;
    }

    if let Some(open) = open_frame_hour {
        result
            .errors
            .push(format!("stream ended with open DATA frame hour {open}"));
    } else if let Some(pending) = pending_perf_hour {
        result.errors.push(format!(
            "stream ended with pending PERF frame hour {pending} without DATA/FRAME_END"
        ));
    }
    if result.frame_hours.is_empty() {
        result
            .errors
            .push("stream contains no certified frame hours".to_string());
    }
    if !final_record_is_certified_frame_end {
        result.errors.push(
            "stream is not closed by a final canonical FRAME_END with exactly one owned DATA+PERF pair"
                .to_string(),
        );
    }
    result
}

fn next_source_hour_is_valid(last_closed_hour: Option<i32>, game_hour_raw: i32) -> bool {
    match last_closed_hour {
        None => true,
        Some(closed) => closed.checked_add(1) == Some(game_hour_raw),
    }
}

fn validate_next_source_hour(
    errors: &mut Vec<String>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    record_label: &str,
    offset: usize,
) {
    let Some(closed) = last_closed_hour else {
        return;
    };
    match closed.checked_add(1) {
        Some(expected) if game_hour_raw == expected => {}
        Some(expected) => errors.push(format!(
            "offset {offset}: {record_label} hour {game_hour_raw} follows closed frame hour {closed}; expected consecutive hour {expected}"
        )),
        None => errors.push(format!(
            "offset {offset}: closed frame hour {closed} exhausted; another record is forbidden"
        )),
    }
}

pub fn derive_depth_stream(raw: &[u8]) -> Result<DepthProof, String> {
    let source_validation = validate_authoritative_stream(raw, SOURCE_DOMAIN_IDS, false);
    if !source_validation.errors.is_empty() {
        return Err(format!(
            "source stream failed authoritative focus/research/decisions validation: {}",
            source_validation.errors.join("; ")
        ));
    }
    let mut proof = DepthProof {
        source_bytes: raw.len(),
        source_records: source_validation.record_count,
        ..DepthProof::default()
    };
    let mut deriver = Deriver::new();
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut depth_sequence = 1u64;
    let mut frame_hours = Vec::new();

    while offset < raw.len() {
        let record = match parse_h4cr_record(raw, offset) {
            Ok(record) => record,
            Err(err) => {
                proof.parse_errors.push(err);
                break;
            }
        };
        if let Some(previous) = previous_sequence {
            debug_assert_eq!(previous.checked_add(1), Some(record.sequence));
        } else {
            debug_assert_eq!(record.sequence, 1);
        }
        previous_sequence = Some(record.sequence);

        match (record.domain_id, record.record_type) {
            (DOMAIN_FOCUS, RECORD_TYPE_DOMAIN_DATA) => match decode_focus_payload(record.payload) {
                Ok(rows) => {
                    proof.focus_data_records = proof.focus_data_records.saturating_add(1);
                    proof.source_focus_rows =
                        proof.source_focus_rows.saturating_add(rows.len() as u64);
                    deriver.apply_focus_rows(record.game_hour_raw, rows);
                }
                Err(err) => proof
                    .parse_errors
                    .push(format!("offset {offset}: focus_live decode: {err}")),
            },
            (DOMAIN_TECH, RECORD_TYPE_DOMAIN_DATA) => match decode_tech_payload(record.payload) {
                Ok(rows) => {
                    proof.tech_data_records = proof.tech_data_records.saturating_add(1);
                    proof.source_research_slot_rows = proof
                        .source_research_slot_rows
                        .saturating_add(rows.len() as u64);
                    deriver.apply_research_rows(record.game_hour_raw, rows);
                }
                Err(err) => proof
                    .parse_errors
                    .push(format!("offset {offset}: tech_live decode: {err}")),
            },
            (DOMAIN_DECISIONS, RECORD_TYPE_DOMAIN_DATA) => {
                match decode_decisions_payload(record.payload) {
                    Ok(rows) => {
                        proof.decisions_data_records =
                            proof.decisions_data_records.saturating_add(1);
                        proof.source_decision_country_rows = proof
                            .source_decision_country_rows
                            .saturating_add(rows.len() as u64);
                        proof.source_timed_decision_rows =
                            proof.source_timed_decision_rows.saturating_add(
                                rows.iter()
                                    .map(|row| row.timed_live.len() as u64)
                                    .sum::<u64>(),
                            );
                        proof.source_targeted_timed_decision_rows =
                            proof.source_targeted_timed_decision_rows.saturating_add(
                                rows.iter()
                                    .map(|row| row.targeted_timed_live.len() as u64)
                                    .sum::<u64>(),
                            );
                        deriver.apply_decision_rows(record.game_hour_raw, rows);
                    }
                    Err(err) => proof
                        .parse_errors
                        .push(format!("offset {offset}: decisions_live decode: {err}")),
                }
            }
            (DOMAIN_FRAME, RECORD_TYPE_FRAME_END) => {
                proof.source_frames = proof.source_frames.saturating_add(1);
                frame_hours.push(record.game_hour_raw);
                let rows = deriver.take_pending();
                proof.focus_event_rows = proof
                    .focus_event_rows
                    .saturating_add(rows.focus_events.len() as u64);
                for row in &rows.focus_events {
                    if row.kind == FOCUS_EVENT_COMPLETED_COUNT_INCREASED {
                        if row.flags & FOCUS_FLAG_COMPLETED_TOKEN_EXACT != 0 {
                            proof.focus_completed_exact_rows =
                                proof.focus_completed_exact_rows.saturating_add(1);
                        } else if row.flags & FOCUS_FLAG_COMPLETED_TOKEN_INFERRED != 0 {
                            proof.focus_completed_inferred_rows =
                                proof.focus_completed_inferred_rows.saturating_add(1);
                        } else {
                            proof.focus_completed_unresolved_rows =
                                proof.focus_completed_unresolved_rows.saturating_add(1);
                        }
                    }
                }
                proof.research_eta_rows = proof
                    .research_eta_rows
                    .saturating_add(rows.research_eta.len() as u64);
                proof.decision_status_rows = proof
                    .decision_status_rows
                    .saturating_add(rows.decision_status.len() as u64);
                proof.decision_event_rows = proof
                    .decision_event_rows
                    .saturating_add(rows.decision_events.len() as u64);
                proof.decision_target_rows = proof
                    .decision_target_rows
                    .saturating_add(rows.decision_targets.len() as u64);
                proof.decision_country_target_rows =
                    proof.decision_country_target_rows.saturating_add(
                        rows.decision_targets
                            .iter()
                            .filter(|row| row.flags & DECISION_TARGET_FLAG_COUNTRY_TAG != 0)
                            .count() as u64,
                    );
                proof.decision_numeric_target_rows =
                    proof.decision_numeric_target_rows.saturating_add(
                        rows.decision_targets
                            .iter()
                            .filter(|row| row.flags & DECISION_TARGET_FLAG_NUMERIC_ID != 0)
                            .count() as u64,
                    );
                proof.badge_rows = proof.badge_rows.saturating_add(rows.badges.len() as u64);
                let perf_status = source_validation
                    .perf_status_by_hour
                    .get(&record.game_hour_raw)
                    .copied()
                    .unwrap_or(0);
                let payload = encode_depth_payload_v3(&rows, perf_status);
                let perf_payload = canonical_perf_payload(perf_status);
                append_h4cr_record(
                    &mut proof.depth_stream,
                    depth_sequence,
                    record.game_hour_raw,
                    DOMAIN_ID,
                    RECORD_TYPE_DOMAIN_PERF,
                    0,
                    &perf_payload,
                );
                depth_sequence = depth_sequence
                    .checked_add(1)
                    .ok_or_else(|| "derived depth stream sequence space exhausted".to_string())?;
                append_h4cr_record(
                    &mut proof.depth_stream,
                    depth_sequence,
                    record.game_hour_raw,
                    DOMAIN_ID,
                    RECORD_TYPE_DOMAIN_DATA,
                    compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                    &payload,
                );
                depth_sequence = depth_sequence
                    .checked_add(1)
                    .ok_or_else(|| "derived depth stream sequence space exhausted".to_string())?;
                let frame_status = source_validation
                    .frame_status_by_hour
                    .get(&record.game_hour_raw)
                    .copied()
                    .unwrap_or(0);
                let frame_payload = canonical_frame_end_payload(1, frame_status);
                append_h4cr_record(
                    &mut proof.depth_stream,
                    depth_sequence,
                    record.game_hour_raw,
                    DOMAIN_FRAME,
                    RECORD_TYPE_FRAME_END,
                    0,
                    &frame_payload,
                );
                depth_sequence = depth_sequence
                    .checked_add(1)
                    .ok_or_else(|| "derived depth stream sequence space exhausted".to_string())?;
                proof.depth_data_records = proof.depth_data_records.saturating_add(1);
            }
            _ => {}
        }
        offset = record.next_offset;
    }

    if !proof.parse_errors.is_empty() {
        return Err(format!(
            "source stream owned payload validation failed: {}",
            proof.parse_errors.join("; ")
        ));
    }

    frame_hours.sort_unstable();
    frame_hours.dedup();
    proof.first_game_hour_raw = frame_hours.first().copied();
    proof.last_game_hour_raw = frame_hours.last().copied();
    proof.game_hours_advanced = match (proof.first_game_hour_raw, proof.last_game_hour_raw) {
        (Some(first), Some(last)) => last.saturating_sub(first),
        _ => 0,
    };
    proof.skipped_game_hours = skipped_hours(&frame_hours);
    // Derivation uses the structural summary; the production gate requires an
    // independently supplied positive duration target at its public boundary.
    let depth_gate = summarize_depth_stream(&proof.depth_stream);
    proof.focus_pause_badge_rows = depth_gate.focus_pause_badge_rows;
    proof.decision_target_rows = depth_gate.decision_target_rows;
    proof.decision_country_target_rows = depth_gate.decision_country_target_rows;
    proof.decision_numeric_target_rows = depth_gate.decision_numeric_target_rows;
    proof.decision_target_badge_rows = depth_gate.decision_target_badge_rows;
    proof.direct_event_badge_rows = depth_gate.direct_event_badge_rows;
    proof.depth_parse_errors = depth_gate.parse_errors;
    proof.sample_events = deriver.samples;
    Ok(proof)
}

pub fn encode_depth_payload(rows: &DepthFrameRows) -> Vec<u8> {
    let mut normalized = rows.clone();
    apply_required_badges(&mut normalized);
    let rows = &normalized;
    // Lane-local proof format only. The shared H4CR D45 wire payload is
    // collector-encoded, selected by the H4CR record-header schema-v2 flag, and
    // redundantly sanity-checked with its FRD2 header by recorder_host.
    let mut payload = Vec::new();
    payload.extend_from_slice(
        &(rows.decision_targets.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&rows.status_flags.to_le_bytes());
    payload
        .extend_from_slice(&(rows.focus_events.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(rows.research_eta.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(
        &(rows.decision_status.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(rows.decision_events.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(&rows.observed_focus_rows.to_le_bytes());
    payload.extend_from_slice(&rows.observed_research_slot_rows.to_le_bytes());
    payload.extend_from_slice(&rows.observed_decision_rows.to_le_bytes());
    payload.extend_from_slice(&rows.observed_timed_decision_rows.to_le_bytes());
    payload.extend_from_slice(&(rows.badges.len().min(u32::MAX as usize) as u32).to_le_bytes());

    for row in &rows.focus_events {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.kind.to_le_bytes());
        payload.extend_from_slice(&row.focus_kind.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.previous_completed_count.to_le_bytes());
        payload.extend_from_slice(&row.current_completed_count.to_le_bytes());
        payload.extend_from_slice(&row.previous_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.current_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.completed_delta.to_le_bytes());
        append_compact_string(&mut payload, row.from_token.as_deref());
        append_compact_string(&mut payload, row.to_token.as_deref());
        append_compact_string(&mut payload, row.completed_token.as_deref());
    }

    for row in &rows.research_eta {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.slot_index.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.hour_delta.to_le_bytes());
        payload.extend_from_slice(&row.previous_tech_handle.to_le_bytes());
        payload.extend_from_slice(&row.current_tech_handle.to_le_bytes());
        payload.extend_from_slice(&row.previous_tech_def_index.to_le_bytes());
        payload.extend_from_slice(&row.current_tech_def_index.to_le_bytes());
        payload.extend_from_slice(&row.previous_live_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.current_live_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.delta_live_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.speed_raw_per_hour.to_le_bytes());
        payload.extend_from_slice(&row.base_required_points_raw.to_le_bytes());
        payload.extend_from_slice(&row.eta_hours.to_le_bytes());
    }

    for row in &rows.decision_status {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.active_timed_count.to_le_bytes());
        payload.extend_from_slice(&row.targeted_count.to_le_bytes());
        payload.extend_from_slice(&row.targeted_timed_count.to_le_bytes());
        payload.extend_from_slice(&row.decisions_taken_bucket_count.to_le_bytes());
        payload.extend_from_slice(&row.re_enable_count.to_le_bytes());
        payload.extend_from_slice(&row.timed_live_count.to_le_bytes());
    }

    for row in &rows.decision_events {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.kind.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.previous_days.to_le_bytes());
        payload.extend_from_slice(&row.current_days.to_le_bytes());
        payload.extend_from_slice(&row.previous_state.to_le_bytes());
        payload.extend_from_slice(&row.current_state.to_le_bytes());
        append_compact_string(&mut payload, row.decision.as_deref());
    }
    for row in &rows.decision_targets {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.target_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.source_kind.to_le_bytes());
        append_compact_string(&mut payload, row.decision.as_deref());
    }
    for row in &rows.badges {
        payload.extend_from_slice(&row.kind.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.source_domain_id.to_le_bytes());
        payload.extend_from_slice(&row.affected_count.to_le_bytes());
        payload.extend_from_slice(&row.evidence_code.to_le_bytes());
        append_compact_string(&mut payload, row.detail.as_deref());
    }
    payload
}

fn encode_depth_payload_v2(rows: &DepthFrameRows) -> Vec<u8> {
    let v1 = encode_depth_payload(rows);
    let mut v2 = Vec::with_capacity(v1.len().saturating_add(8));
    v2.extend_from_slice(b"FRD2");
    v2.extend_from_slice(&2u16.to_le_bytes());
    v2.extend_from_slice(&56u16.to_le_bytes());
    v2.extend_from_slice(&v1[8..28]);
    v2.extend_from_slice(&v1[0..4]);
    v2.extend_from_slice(&v1[28..48]);
    v2.extend_from_slice(&0u32.to_le_bytes());
    v2.extend_from_slice(&v1[48..]);
    v2
}

fn encode_depth_payload_v3(rows: &DepthFrameRows, raw_status_flags: u32) -> Vec<u8> {
    let mut v3 = encode_depth_payload_v2(rows);
    v3[4..6].copy_from_slice(&3u16.to_le_bytes());
    v3[6..8].copy_from_slice(&60u16.to_le_bytes());
    let row_bytes = v3.split_off(56);
    v3.extend_from_slice(&raw_status_flags.to_le_bytes());
    v3.extend_from_slice(&row_bytes);
    v3
}

pub fn decode_depth_payload(offset: usize, payload: &[u8]) -> DepthGateSummary {
    let mut summary = DepthGateSummary::default();
    summary.data_records = 1;
    if payload.len() < DEPTH_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: {DOMAIN_NAME} payload is {} byte(s), expected at least {DEPTH_HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }
    let decision_target_count = read_u32_le(payload, 0) as usize;
    let schema_flags = read_u32_le(payload, 4);
    let status_flags = read_u32_le(payload, 8);
    let focus_count = read_u32_le(payload, 12) as usize;
    let research_count = read_u32_le(payload, 16) as usize;
    let decision_status_count = read_u32_le(payload, 20) as usize;
    let decision_event_count = read_u32_le(payload, 24) as usize;
    let observed_focus_rows = read_u32_le(payload, 28);
    let _observed_decision_rows = read_u32_le(payload, 36);
    let badge_count = read_u32_le(payload, 44) as usize;
    if schema_flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: {DOMAIN_NAME} header schema_flags must be zero, got {schema_flags}"
        ));
    }
    if status_flags & !DEPTH_STATUS_KNOWN_MASK != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: {DOMAIN_NAME} status_flags has unknown bits: {status_flags}"
        ));
    }
    let exact_status = DEPTH_STATUS_FOCUS_PAUSE_EXACT
        | DEPTH_STATUS_DECISION_TARGETS_EXACT
        | DEPTH_STATUS_DECISION_COOLDOWNS_EXACT;
    let legacy_status = DEPTH_STATUS_FOCUS_PAUSE_BADGED | DEPTH_STATUS_DECISION_TARGETS_BADGED;
    if status_flags & DEPTH_STATUS_DIRECT_EVENTS_DERIVED == 0
        || (status_flags & exact_status != exact_status
            && status_flags & legacy_status != legacy_status)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: {DOMAIN_NAME} status_flags lack a complete exact or legacy authority set"
        ));
    }

    summary.focus_event_rows = focus_count as u64;
    summary.research_eta_rows = research_count as u64;
    summary.decision_status_rows = decision_status_count as u64;
    summary.decision_event_rows = decision_event_count as u64;
    summary.decision_target_rows = decision_target_count as u64;
    summary.badge_rows = badge_count as u64;

    let mut cursor = DEPTH_HEADER_LEN;
    let mut seen_focus = BTreeSet::new();
    for row_index in 0..focus_count {
        if let Err(err) = require_remaining(payload, cursor, 36, "focus depth row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return summary;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let kind = read_u32_le(payload, cursor + 4);
        let focus_kind = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        let previous_completed = read_u32_le(payload, cursor + 16);
        let current_completed = read_u32_le(payload, cursor + 20);
        let previous_progress = read_i32_le(payload, cursor + 24);
        let current_progress = read_i32_le(payload, cursor + 28);
        let completed_delta = read_u32_le(payload, cursor + 32);
        cursor += 36;
        let from_len = read_compact_string_len(payload, &mut cursor, "focus depth from token")
            .unwrap_or_else(|err| {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                0
            });
        let to_len = read_compact_string_len(payload, &mut cursor, "focus depth to token")
            .unwrap_or_else(|err| {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                0
            });
        let completed_len =
            read_compact_string_len(payload, &mut cursor, "focus depth completed token")
                .unwrap_or_else(|err| {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    0
                });
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has zero tag_db_id"
            ));
        }
        if kind == 0 || kind > FOCUS_EVENT_STATE_SNAPSHOT {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has invalid kind {kind}"
            ));
        }
        if focus_kind > FOCUS_KIND_CONTINUOUS {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has invalid focus_kind {focus_kind}"
            ));
        }
        if flags
            & !(FOCUS_FLAG_COMPLETED_TOKEN_INFERRED
                | FOCUS_FLAG_COMPLETED_TOKEN_EXACT
                | FOCUS_FLAG_PAUSED
                | FOCUS_FLAG_PAUSED_PRESENT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has unknown flags {flags}"
            ));
        }
        if kind == FOCUS_EVENT_STATE_SNAPSHOT {
            if flags & FOCUS_FLAG_PAUSED_PRESENT == 0
                || flags & (FOCUS_FLAG_COMPLETED_TOKEN_INFERRED | FOCUS_FLAG_COMPLETED_TOKEN_EXACT)
                    != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus state snapshot row {row_index} has invalid pause flags {flags}"
                ));
            }
        } else if flags & (FOCUS_FLAG_PAUSED | FOCUS_FLAG_PAUSED_PRESENT) != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: non-snapshot focus row {row_index} carries pause flags {flags}"
            ));
        }
        if kind == FOCUS_EVENT_COMPLETED_COUNT_INCREASED {
            if flags & FOCUS_FLAG_COMPLETED_TOKEN_EXACT != 0
                && flags & FOCUS_FLAG_COMPLETED_TOKEN_INFERRED != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus completed row {row_index} has both exact and inferred completed-token flags"
                ));
            }
            if flags & FOCUS_FLAG_COMPLETED_TOKEN_EXACT != 0 {
                summary.focus_completed_exact_rows =
                    summary.focus_completed_exact_rows.saturating_add(1);
            } else if flags & FOCUS_FLAG_COMPLETED_TOKEN_INFERRED != 0 {
                summary.focus_completed_inferred_rows =
                    summary.focus_completed_inferred_rows.saturating_add(1);
            } else {
                summary.focus_completed_unresolved_rows =
                    summary.focus_completed_unresolved_rows.saturating_add(1);
            }
            if current_completed != u32::MAX
                && previous_completed != u32::MAX
                && current_completed <= previous_completed
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus completed row {row_index} did not increase completed_count"
                ));
            }
            if completed_delta == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus completed row {row_index} has zero completed_delta"
                ));
            }
        }
        if kind == FOCUS_EVENT_PROGRESS_REGRESSED && current_progress >= previous_progress {
            summary.parse_errors.push(format!(
                "offset {offset}: focus progress-regression row {row_index} did not regress"
            ));
        }
        if flags & FOCUS_FLAG_COMPLETED_TOKEN_EXACT != 0 && completed_len == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus row {row_index} has exact completed-token flag without token"
            ));
        }
        // A completed-count-increase row is legitimately token-less when the
        // encoder could not resolve a focus token: completed_count ticked up
        // while the country had no active national/continuous focus at either
        // bracketing sample (focus_kind == 0) and completed_tokens surfaced no
        // new token, so neither the EXACT nor INFERRED completed-token flag is
        // set. This is the `focus_completed_unresolved_rows` state, not
        // corruption: the completion delta and counts are still carried, so
        // accepting it loses no data and fabricates no token. An EXACT/INFERRED
        // completed row without a token stays a hard error, and kinds 2..=5
        // always carry >=1 token, so this exemption stays tight and fail-closed.
        let unresolved_completion = kind == FOCUS_EVENT_COMPLETED_COUNT_INCREASED
            && flags & (FOCUS_FLAG_COMPLETED_TOKEN_EXACT | FOCUS_FLAG_COMPLETED_TOKEN_INFERRED)
                == 0;
        if from_len == 0
            && to_len == 0
            && completed_len == 0
            && kind != FOCUS_EVENT_INVALID_BOTH_CURRENT_TYPES
            && kind != FOCUS_EVENT_STATE_SNAPSHOT
            && !unresolved_completion
        {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has no token context"
            ));
        }
        if !seen_focus.insert((
            tag_db_id,
            kind,
            previous_completed,
            current_completed,
            completed_delta,
        )) {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} duplicates focus transition key"
            ));
        }
    }

    let mut seen_research = BTreeSet::new();
    for row_index in 0..research_count {
        if let Err(err) =
            require_remaining(payload, cursor, RESEARCH_ETA_ROW_LEN, "research ETA row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return summary;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let slot_index = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let hour_delta = read_u32_le(payload, cursor + 12);
        let previous_handle = read_u32_le(payload, cursor + 16);
        let current_handle = read_u32_le(payload, cursor + 20);
        let previous_def = read_u32_le(payload, cursor + 24);
        let current_def = read_u32_le(payload, cursor + 28);
        let previous_progress = read_i64_le(payload, cursor + 32);
        let current_progress = read_i64_le(payload, cursor + 40);
        let delta = read_i64_le(payload, cursor + 48);
        let speed = read_i64_le(payload, cursor + 56);
        let base = read_i64_le(payload, cursor + 64);
        let eta = read_i64_le(payload, cursor + 72);
        cursor += RESEARCH_ETA_ROW_LEN;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has zero tag_db_id"
            ));
        }
        if slot_index > 32 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has invalid slot_index {slot_index}"
            ));
        }
        if flags
            & !(RESEARCH_FLAG_SAME_TECH
                | RESEARCH_FLAG_POSITIVE_SPEED
                | RESEARCH_FLAG_ETA_AVAILABLE
                | RESEARCH_FLAG_TECH_ROLLOVER
                | RESEARCH_FLAG_NEGATIVE_DELTA
                | RESEARCH_FLAG_BASE_PRESENT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has unknown flags {flags}"
            ));
        }
        if hour_delta == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has zero hour_delta"
            ));
        }
        let same_tech = flags & RESEARCH_FLAG_SAME_TECH != 0;
        if same_tech && (previous_handle != current_handle || previous_def != current_def) {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has same-tech flag but different tech ids"
            ));
        }
        if !same_tech && flags & RESEARCH_FLAG_TECH_ROLLOVER == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} changed tech without rollover flag"
            ));
        }
        if delta != current_progress.saturating_sub(previous_progress) {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} delta does not match progress"
            ));
        }
        if flags & RESEARCH_FLAG_POSITIVE_SPEED != 0 && speed <= 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has positive-speed flag without positive speed"
            ));
        }
        if flags & RESEARCH_FLAG_NEGATIVE_DELTA != 0 && delta >= 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has negative-delta flag without negative delta"
            ));
        }
        if flags & RESEARCH_FLAG_BASE_PRESENT != 0 && base <= 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has base-present flag without positive base"
            ));
        }
        if flags & RESEARCH_FLAG_ETA_AVAILABLE != 0 && eta < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has negative eta"
            ));
        }
        if !seen_research.insert((
            tag_db_id,
            slot_index,
            current_handle,
            current_def,
            current_progress,
        )) {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} duplicates slot/progress key"
            ));
        }
    }

    let mut seen_decision_status = BTreeSet::new();
    let mut targeted_timed_status_count_by_tag = BTreeMap::new();
    let mut expected_decision_badge_affected_count = 0u32;
    for row_index in 0..decision_status_count {
        if let Err(err) = require_remaining(
            payload,
            cursor,
            DECISION_STATUS_ROW_LEN,
            "decision status row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return summary;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let counts = [
            read_u32_le(payload, cursor + 4),
            read_u32_le(payload, cursor + 8),
            read_u32_le(payload, cursor + 12),
            read_u32_le(payload, cursor + 16),
            read_u32_le(payload, cursor + 20),
            read_u32_le(payload, cursor + 24),
        ];
        cursor += DECISION_STATUS_ROW_LEN;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision status row {row_index} has zero tag_db_id"
            ));
        }
        if counts.iter().any(|count| *count > 4096) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision status row {row_index} has impossible count"
            ));
        }
        if !seen_decision_status.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision status row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        targeted_timed_status_count_by_tag.insert(tag_db_id, counts[2]);
        expected_decision_badge_affected_count = expected_decision_badge_affected_count
            .saturating_add(counts[1])
            .saturating_add(counts[2])
            .saturating_add(counts[3])
            .saturating_add(counts[4]);
    }

    for row_index in 0..decision_event_count {
        if let Err(err) = require_remaining(payload, cursor, 28, "decision event row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return summary;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let kind = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let previous_days = read_i32_le(payload, cursor + 12);
        let current_days = read_i32_le(payload, cursor + 16);
        let previous_state = read_i32_le(payload, cursor + 20);
        let current_state = read_i32_le(payload, cursor + 24);
        cursor += 28;
        let decision_len = read_compact_string_len(payload, &mut cursor, "decision event token")
            .unwrap_or_else(|err| {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                0
            });
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has zero tag_db_id"
            ));
        }
        if !(DECISION_EVENT_STARTED..=DECISION_EVENT_REMOVE_COOLDOWN_SNAPSHOT).contains(&kind) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has invalid kind {kind}"
            ));
        }
        if flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has unknown flags {flags}"
            ));
        }
        if kind != DECISION_EVENT_STATUS_COUNTS_CHANGED && decision_len == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has no decision token"
            ));
        }
        if kind == DECISION_EVENT_DAY_DELTA && previous_days == current_days {
            summary.parse_errors.push(format!(
                "offset {offset}: decision day-delta row {row_index} did not change days"
            ));
        }
        if kind == DECISION_EVENT_STATE_CHANGED && previous_state == current_state {
            summary.parse_errors.push(format!(
                "offset {offset}: decision state-change row {row_index} did not change state"
            ));
        }
    }

    let mut decision_target_rows_by_tag: BTreeMap<u32, u32> = BTreeMap::new();
    let mut seen_decision_targets = BTreeSet::new();
    for row_index in 0..decision_target_count {
        if let Err(err) = require_remaining(
            payload,
            cursor,
            DECISION_TARGET_ROW_FIXED_LEN,
            "decision target row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return summary;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let target_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let source_kind = read_u32_le(payload, cursor + 12);
        cursor += DECISION_TARGET_ROW_FIXED_LEN;
        let decision = read_compact_string(payload, &mut cursor, "decision target token")
            .unwrap_or_else(|err| {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                String::new()
            });

        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has zero tag_db_id"
            ));
        }
        if flags & !DECISION_TARGET_FLAG_KNOWN_MASK != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has unknown flags {flags}"
            ));
        }
        let has_country_flag = flags & DECISION_TARGET_FLAG_COUNTRY_TAG != 0;
        let has_numeric_flag = flags & DECISION_TARGET_FLAG_NUMERIC_ID != 0;
        let has_record_0x20 = flags & DECISION_TARGET_FLAG_RECORD_0X20 != 0;
        let has_record_0x24 = flags & DECISION_TARGET_FLAG_RECORD_0X24 != 0;
        if has_country_flag == has_numeric_flag {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} must have exactly one target kind flag"
            ));
        }
        if has_country_flag && (!has_record_0x20 || has_record_0x24) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} is missing required country/record+0x20 flags"
            ));
        }
        if has_numeric_flag && (!has_record_0x24 || has_record_0x20) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} is missing required numeric/record+0x24 flags"
            ));
        }
        if has_country_flag
            && (target_id == 0
                || target_id == DECISION_TARGET_UNKNOWN_ID
                || target_id > DECISION_TARGET_MAX_TAG_DB_ID)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has invalid country target id {target_id}"
            ));
        }
        if has_numeric_flag
            && (target_id == 0
                || target_id == DECISION_TARGET_UNKNOWN_ID
                || target_id > DECISION_TARGET_MAX_NUMERIC_ID)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has invalid numeric target id {target_id}"
            ));
        }
        if source_kind != DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has invalid source_kind {source_kind}"
            ));
        }
        if decision.is_empty() {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has no decision token"
            ));
        }
        if !seen_decision_targets.insert((tag_db_id, target_id, source_kind, decision.clone())) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} duplicates owner/target/decision key"
            ));
        }
        if has_country_flag {
            summary.decision_country_target_rows =
                summary.decision_country_target_rows.saturating_add(1);
        }
        if has_numeric_flag {
            summary.decision_numeric_target_rows =
                summary.decision_numeric_target_rows.saturating_add(1);
        }
        let count = decision_target_rows_by_tag.entry(tag_db_id).or_insert(0);
        *count = count.saturating_add(1);
    }
    for (tag_db_id, target_rows) in decision_target_rows_by_tag {
        match targeted_timed_status_count_by_tag.get(&tag_db_id).copied() {
            Some(status_count) if target_rows <= status_count => {}
            Some(status_count) => summary.parse_errors.push(format!(
                "offset {offset}: decision target rows {target_rows} exceed targeted_timed_count {status_count} for tag_db_id {tag_db_id}"
            )),
            None => summary.parse_errors.push(format!(
                "offset {offset}: decision target rows exist without a decision status row for tag_db_id {tag_db_id}"
            )),
        }
    }

    let mut badge_kinds = BTreeSet::new();
    for row_index in 0..badge_count {
        if let Err(err) = require_remaining(payload, cursor, BADGE_ROW_FIXED_LEN, "badge row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return summary;
        }
        let kind = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let source_domain_id = read_u32_le(payload, cursor + 8);
        let affected_count = read_u32_le(payload, cursor + 12);
        let evidence_code = read_u32_le(payload, cursor + 16);
        cursor += BADGE_ROW_FIXED_LEN;
        let detail =
            read_compact_string(payload, &mut cursor, "badge detail").unwrap_or_else(|err| {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                String::new()
            });
        if kind == 0 || kind > BADGE_KIND_MAX {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} has invalid kind {kind}"
            ));
        }
        if flags == 0 || flags & !BADGE_FLAG_KNOWN_MASK != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} has invalid flags {flags}"
            ));
        }
        if detail.is_empty() {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} has no detail token"
            ));
        }
        if !badge_kinds.insert(kind) {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} duplicates kind {kind}"
            ));
        }
        match kind {
            BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED => {
                if source_domain_id != DOMAIN_FOCUS as u32
                    || evidence_code != BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH
                    || flags & BADGE_FLAG_FAIL_CLOSED == 0
                    || flags & BADGE_FLAG_CURRENT_BUILD_PROBED == 0
                    || detail != BADGE_DETAIL_FOCUS_PAUSE_FAIL_CLOSED
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: focus pause badge row {row_index} has inconsistent provenance"
                    ));
                }
                summary.focus_pause_badge_rows = summary.focus_pause_badge_rows.saturating_add(1);
                if affected_count != observed_focus_rows {
                    summary.parse_errors.push(format!(
                        "offset {offset}: focus pause badge row {row_index} affected_count {affected_count} != observed focus rows {observed_focus_rows}"
                    ));
                }
            }
            BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED => {
                if source_domain_id != DOMAIN_DECISIONS as u32
                    || evidence_code != BADGE_EVIDENCE_TIMED_DEFINITION_LAYOUT_REJECTED
                    || flags & BADGE_FLAG_FAIL_CLOSED == 0
                    || flags & BADGE_FLAG_CURRENT_BUILD_PROBED == 0
                    || detail != BADGE_DETAIL_DECISION_TARGET_FIELDS_FAIL_CLOSED
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: decision target badge row {row_index} has inconsistent provenance"
                    ));
                }
                summary.decision_target_badge_rows =
                    summary.decision_target_badge_rows.saturating_add(1);
                if affected_count != expected_decision_badge_affected_count {
                    summary.parse_errors.push(format!(
                        "offset {offset}: decision target badge row {row_index} affected_count {affected_count} != decision target-body authority {expected_decision_badge_affected_count}"
                    ));
                }
            }
            BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE => {
                if source_domain_id != DOMAIN_FRAME as u32
                    || evidence_code != BADGE_EVIDENCE_COMPACT_FRAME_DELTA_DERIVED
                    || flags & BADGE_FLAG_FAIL_CLOSED == 0
                    || flags & BADGE_FLAG_DERIVED_DETERMINISTIC == 0
                    || detail != BADGE_DETAIL_DIRECT_EVENT_HOOKS_UNAVAILABLE
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: direct event badge row {row_index} has inconsistent provenance"
                    ));
                }
                summary.direct_event_badge_rows = summary.direct_event_badge_rows.saturating_add(1);
                let expected_direct_event_count = focus_count
                    .saturating_add(decision_event_count)
                    .min(u32::MAX as usize)
                    as u32;
                if affected_count != expected_direct_event_count {
                    summary.parse_errors.push(format!(
                        "offset {offset}: direct event badge row {row_index} affected_count {affected_count} != derived event rows {expected_direct_event_count}"
                    ));
                }
            }
            _ => {}
        }
    }
    for (flag, exact_flag, kind, name) in [
        (
            DEPTH_STATUS_FOCUS_PAUSE_BADGED,
            DEPTH_STATUS_FOCUS_PAUSE_EXACT,
            BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED,
            "focus pause",
        ),
        (
            DEPTH_STATUS_DECISION_TARGETS_BADGED,
            DEPTH_STATUS_DECISION_TARGETS_EXACT,
            BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED,
            "decision target fields",
        ),
        (
            DEPTH_STATUS_DIRECT_EVENTS_DERIVED,
            DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED,
            BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE,
            "direct event hooks",
        ),
    ] {
        if status_flags & exact_flag == 0
            && status_flags & flag != 0
            && !badge_kinds.contains(&kind)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: {DOMAIN_NAME} status flag for {name} lacks badge row"
            ));
        }
        if status_flags & exact_flag == 0 && !badge_kinds.contains(&kind) {
            summary.parse_errors.push(format!(
                "offset {offset}: {DOMAIN_NAME} missing required {name} badge row"
            ));
        }
    }

    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: {DOMAIN_NAME} payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    summary
}

fn decode_depth_record_payload(
    offset: usize,
    record_flags: u32,
    payload: &[u8],
) -> DepthGateSummary {
    if record_flags & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 == 0 {
        return decode_depth_payload(offset, payload);
    }
    let normalized = match normalize_depth_versioned_payload(payload) {
        Ok(normalized) => normalized,
        Err(error) => {
            let mut summary = DepthGateSummary {
                data_records: 1,
                ..DepthGateSummary::default()
            };
            summary
                .parse_errors
                .push(format!("offset {offset}: {error}"));
            return summary;
        }
    };
    decode_depth_payload(offset, &normalized)
}

fn normalize_depth_versioned_payload(payload: &[u8]) -> Result<Vec<u8>, String> {
    const V2_HEADER_LEN: usize = 56;
    const V3_HEADER_LEN: usize = 60;
    const V5_HEADER_LEN: usize = 64;
    if payload.len() < V2_HEADER_LEN {
        return Err(format!(
            "{DOMAIN_NAME} versioned payload is {} byte(s), expected at least {V2_HEADER_LEN}",
            payload.len()
        ));
    }
    if payload.get(0..4) != Some(b"FRD2".as_slice()) {
        return Err(format!(
            "{DOMAIN_NAME} versioned payload is missing FRD2 sanity marker"
        ));
    }
    let schema_version = read_u16_le(payload, 4);
    let header_len = read_u16_le(payload, 6) as usize;
    if !matches!(
        (schema_version, header_len),
        (2, V2_HEADER_LEN)
            | (3, V3_HEADER_LEN)
            | (4, V3_HEADER_LEN)
            | (5, V5_HEADER_LEN)
    ) {
        return Err(format!(
            "{DOMAIN_NAME} versioned header has version {schema_version} and length {header_len}"
        ));
    }
    if payload.len() < header_len {
        return Err(format!(
            "{DOMAIN_NAME} schema-v{schema_version} payload is {} byte(s), expected at least {header_len}",
            payload.len()
        ));
    }

    // V2/V3 share the legacy 16-byte decision-target prefix. V4/V5 extend that
    // row by 12 bytes and V5 appends the pointer-free direct-transition
    // trailer. Rebuild only the legacy D45 view consumed by this lane-local
    // proof; the recorder remains the canonical detailed/v5 fold authority.
    let mut normalized = Vec::with_capacity(
        payload
            .len()
            .saturating_sub(header_len.saturating_sub(DEPTH_HEADER_LEN)),
    );
    normalized.extend_from_slice(&payload[28..32]); // decision_target_count
    normalized.extend_from_slice(&0u32.to_le_bytes()); // v1 reserved schema word
    normalized.extend_from_slice(&payload[8..28]); // status + four row counts
    normalized.extend_from_slice(&payload[32..52]); // observed counts + badge count
    if schema_version <= 3 {
        normalized.extend_from_slice(&payload[header_len..]);
    } else {
        normalize_v4_v5_depth_rows(payload, header_len, schema_version, &mut normalized)?;
    }
    let depth_status = read_u32_le(payload, 8);
    if schema_version == 5
        && depth_status & DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED == 0
    {
        return Err(format!(
            "{DOMAIN_NAME} schema-v5 payload lacks validated direct-hook source"
        ));
    }
    if schema_version < 5 && depth_status & DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED != 0 {
        return Err(format!(
            "{DOMAIN_NAME} legacy payload claims validated direct-hook source"
        ));
    }
    Ok(normalized)
}

fn normalize_v4_v5_depth_rows(
    payload: &[u8],
    header_len: usize,
    schema_version: u16,
    normalized: &mut Vec<u8>,
) -> Result<(), String> {
    let focus_count = read_u32_le(payload, 12) as usize;
    let research_count = read_u32_le(payload, 16) as usize;
    let decision_status_count = read_u32_le(payload, 20) as usize;
    let decision_event_count = read_u32_le(payload, 24) as usize;
    let decision_target_count = read_u32_le(payload, 28) as usize;
    let badge_count = read_u32_le(payload, 48) as usize;
    let direct_count = if schema_version == 5 {
        read_u32_le(payload, 60) as usize
    } else {
        0
    };
    let mut cursor = header_len;

    for _ in 0..focus_count {
        let start = cursor;
        require_remaining(payload, cursor, 36, "v4/v5 focus row")?;
        cursor += 36;
        for label in ["focus from", "focus to", "focus completed"] {
            read_compact_string_len(payload, &mut cursor, label)?;
        }
        normalized.extend_from_slice(&payload[start..cursor]);
    }
    let fixed_len = research_count
        .checked_mul(RESEARCH_ETA_ROW_LEN)
        .and_then(|len| len.checked_add(decision_status_count.checked_mul(DECISION_STATUS_ROW_LEN)?))
        .ok_or_else(|| "v4/v5 fixed row length overflow".to_string())?;
    require_remaining(payload, cursor, fixed_len, "v4/v5 fixed rows")?;
    normalized.extend_from_slice(&payload[cursor..cursor + fixed_len]);
    cursor += fixed_len;
    for _ in 0..decision_event_count {
        let start = cursor;
        require_remaining(payload, cursor, 28, "v4/v5 decision event row")?;
        cursor += 28;
        read_compact_string_len(payload, &mut cursor, "v4/v5 decision event token")?;
        normalized.extend_from_slice(&payload[start..cursor]);
    }
    for _ in 0..decision_target_count {
        require_remaining(payload, cursor, 28, "v4/v5 decision target row")?;
        normalized.extend_from_slice(&payload[cursor..cursor + DECISION_TARGET_ROW_FIXED_LEN]);
        cursor += 28;
        let string_start = cursor;
        read_compact_string_len(payload, &mut cursor, "v4/v5 decision target token")?;
        normalized.extend_from_slice(&payload[string_start..cursor]);
    }
    for _ in 0..badge_count {
        let start = cursor;
        require_remaining(payload, cursor, BADGE_ROW_FIXED_LEN, "v4/v5 badge row")?;
        cursor += BADGE_ROW_FIXED_LEN;
        read_compact_string_len(payload, &mut cursor, "v4/v5 badge detail")?;
        normalized.extend_from_slice(&payload[start..cursor]);
    }
    if schema_version == 5 {
        let trailer = payload
            .get(cursor..)
            .ok_or_else(|| "v5 direct trailer offset overflow".to_string())?;
        let sequence_start = if direct_count == 0 {
            // The hook epoch and sequence cursor survive logical-stream
            // rollover. An empty first drain therefore establishes the new
            // stream's process-wide baseline instead of implying sequence 1.
            32
        } else {
            require_remaining(
                trailer,
                direct_frd_transitions_live::D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN,
                direct_frd_transitions_live::D45_V5_DIRECT_EVENT_ROW_LEN,
                "v5 direct trailer first row",
            )?;
            direct_frd_transitions_live::D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN
        };
        require_remaining(
            trailer,
            sequence_start,
            8,
            "v5 direct trailer sequence baseline",
        )?;
        let expected_first = u64::from_le_bytes(
            trailer[sequence_start..sequence_start + 8]
                .try_into()
                .expect("fixed v5 direct sequence"),
        );
        let rows = direct_frd_transitions_live::decode_trailer(trailer, expected_first)
            .map_err(|error| format!("v5 direct trailer rejected: {error}"))?;
        if rows.len() != direct_count {
            return Err(format!(
                "v5 direct trailer row count {} != header {direct_count}",
                rows.len()
            ));
        }
        cursor = payload.len();
    }
    if cursor != payload.len() {
        return Err(format!(
            "v4/v5 payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    Ok(())
}

/// Gate a production D45 stream for an explicit commanded number of game-hour
/// advancements. Because both endpoints must be closed, `N` advancements
/// require exactly `N + 1` consecutive FRAME_ENDs.
pub fn gate_existing_depth_stream(
    raw: &[u8],
    commanded_game_hour_advancements: i64,
) -> DepthGateSummary {
    let mut summary = summarize_depth_stream(raw);
    summary.commanded_game_hour_advancements = commanded_game_hour_advancements;
    summary.required_closed_frames = commanded_game_hour_advancements
        .checked_add(1)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(u64::MAX);
    if commanded_game_hour_advancements <= 0 {
        summary.parse_errors.push(format!(
            "commanded game-hour advancements must be explicitly positive, got {commanded_game_hour_advancements}"
        ));
    }
    let observed_closed_frames = summary.frame_end_records;
    if summary.game_hours_advanced != commanded_game_hour_advancements
        || observed_closed_frames != summary.required_closed_frames
    {
        summary.parse_errors.push(format!(
            "production proof duration is {} advancement(s) across {} observed closed frame(s), expected exactly {} advancement(s) / {} closed frame(s)",
            summary.game_hours_advanced,
            observed_closed_frames,
            commanded_game_hour_advancements,
            summary.required_closed_frames
        ));
    }
    summary
}

fn summarize_depth_stream(raw: &[u8]) -> DepthGateSummary {
    let validation = validate_authoritative_stream(raw, &[DOMAIN_ID], true);
    let first_game_hour_raw = validation.frame_hours.first().copied();
    let last_game_hour_raw = validation.frame_hours.last().copied();
    let game_hours_advanced = first_game_hour_raw
        .zip(last_game_hour_raw)
        .map(|(first, last)| i64::from(last) - i64::from(first))
        .unwrap_or(-1);
    let mut offset = 0usize;
    let mut summary = DepthGateSummary {
        perf_records: validation.owned_perf_records,
        frame_end_records: validation.frame_end_records,
        certified_hours: if validation.errors.is_empty() {
            validation.frame_hours.len() as u64
        } else {
            0
        },
        first_game_hour_raw,
        last_game_hour_raw,
        game_hours_advanced,
        status_flags_or: validation.status_flags_or,
        fatal_status_flags: validation.fatal_status_flags,
        parse_errors: validation.errors,
        ..DepthGateSummary::default()
    };
    while offset < raw.len() {
        match parse_h4cr_record(raw, offset) {
            Ok(record) => {
                if record.domain_id == DOMAIN_ID && record.record_type == RECORD_TYPE_DOMAIN_DATA {
                    merge_depth_gate(
                        &mut summary,
                        decode_depth_record_payload(offset, record.record_flags, record.payload),
                    );
                }
                offset = record.next_offset;
            }
            Err(_) => break,
        }
    }
    summary
}

fn derive_research_eta_row(
    previous_hour: i32,
    previous: &ResearchSlotState,
    current_hour: i32,
    current: &ResearchSlotState,
) -> Option<ResearchEtaRow> {
    let hour_delta_i32 = current_hour.saturating_sub(previous_hour);
    if hour_delta_i32 <= 0 {
        return None;
    }
    let hour_delta = hour_delta_i32 as u32;
    let same_tech = previous.tech_handle_raw == current.tech_handle_raw
        && previous.tech_def_index_raw == current.tech_def_index_raw;
    let prev_progress = previous.live_progress_raw.unwrap_or(0);
    let curr_progress = current.live_progress_raw.unwrap_or(0);
    let delta = curr_progress.saturating_sub(prev_progress);
    let mut flags = 0u32;
    if same_tech {
        flags |= RESEARCH_FLAG_SAME_TECH;
    } else {
        flags |= RESEARCH_FLAG_TECH_ROLLOVER;
    }
    if delta > 0 {
        flags |= RESEARCH_FLAG_POSITIVE_SPEED;
    }
    if delta < 0 {
        flags |= RESEARCH_FLAG_NEGATIVE_DELTA;
    }
    let base = current.base_required_points_raw.unwrap_or(0);
    if base > 0 {
        flags |= RESEARCH_FLAG_BASE_PRESENT;
    }
    let speed = if hour_delta > 0 {
        delta.checked_div(hour_delta as i64).unwrap_or(0)
    } else {
        0
    };
    let eta = if same_tech && speed > 0 && base > curr_progress {
        flags |= RESEARCH_FLAG_ETA_AVAILABLE;
        ceil_div_i64(base.saturating_sub(curr_progress), speed)
    } else if same_tech && base > 0 && curr_progress >= base {
        flags |= RESEARCH_FLAG_ETA_AVAILABLE;
        0
    } else {
        -1
    };
    if same_tech && previous.live_progress_raw.is_none() && current.live_progress_raw.is_none() {
        return None;
    }
    Some(ResearchEtaRow {
        tag_db_id: current.tag_db_id,
        slot_index: current.slot_index,
        flags,
        hour_delta,
        previous_tech_handle: previous.tech_handle_raw,
        current_tech_handle: current.tech_handle_raw,
        previous_tech_def_index: previous.tech_def_index_raw,
        current_tech_def_index: current.tech_def_index_raw,
        previous_live_progress_raw: prev_progress,
        current_live_progress_raw: curr_progress,
        delta_live_progress_raw: delta,
        speed_raw_per_hour: speed,
        base_required_points_raw: base,
        eta_hours: eta,
    })
}

fn decode_focus_payload(payload: &[u8]) -> Result<Vec<FocusState>, String> {
    if payload.len() < 28 {
        return Err(format!(
            "payload is {} byte(s), expected at least 28",
            payload.len()
        ));
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let completed_token_count = read_u32_le(payload, 24) as usize;
    let mut cursor = 28usize;
    let mut rows = Vec::with_capacity(country_count);
    for _ in 0..country_count {
        require_remaining(payload, cursor, 16, "focus country row")?;
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let completed_count_raw = read_u32_le(payload, cursor + 8);
        let progress_raw = read_i32_le(payload, cursor + 12);
        cursor += 16;
        let _focus_tree = read_compact_string(payload, &mut cursor, "focus tree")?;
        let current = read_compact_string(payload, &mut cursor, "focus current")?;
        let current_continuous =
            read_compact_string(payload, &mut cursor, "focus current continuous")?;
        rows.push(FocusState {
            tag_db_id,
            completed_count: if flags & (1 << 1) != 0 {
                Some(completed_count_raw)
            } else {
                None
            },
            progress_raw: if flags & (1 << 4) != 0 {
                Some(progress_raw)
            } else {
                None
            },
            current: if flags & (1 << 2) != 0 && !current.is_empty() {
                Some(current)
            } else {
                None
            },
            current_continuous: if flags & (1 << 3) != 0 && !current_continuous.is_empty() {
                Some(current_continuous)
            } else {
                None
            },
            completed_tokens: Vec::new(),
        });
    }
    let mut completed_by_tag: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for _ in 0..completed_token_count {
        require_remaining(payload, cursor, 4, "focus completed row")?;
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        let token = read_compact_string(payload, &mut cursor, "focus completed token")?;
        completed_by_tag.entry(tag_db_id).or_default().push(token);
    }
    if cursor != payload.len() {
        return Err(format!(
            "focus payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    for row in &mut rows {
        if let Some(tokens) = completed_by_tag.remove(&row.tag_db_id) {
            row.completed_tokens = tokens;
        }
    }
    Ok(rows)
}

fn decode_tech_payload(payload: &[u8]) -> Result<Vec<ResearchSlotState>, String> {
    if payload.len() < 32 {
        return Err(format!(
            "payload is {} byte(s), expected at least 32",
            payload.len()
        ));
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let active_count = read_u32_le(payload, 28) as usize;
    let mut cursor = 32usize.saturating_add(country_count.saturating_mul(16));
    require_remaining(
        payload,
        cursor,
        active_count.saturating_mul(40),
        "tech active rows",
    )?;
    let mut rows = Vec::with_capacity(active_count);
    for _ in 0..active_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let slot_index = read_u32_le(payload, cursor + 4);
        let tech_handle_raw = read_u32_le(payload, cursor + 8);
        let tech_def_index_raw = read_u32_le(payload, cursor + 12);
        let progress_raw = read_u32_le(payload, cursor + 16);
        let flags = read_u32_le(payload, cursor + 20);
        let live_progress_raw = read_i64_le(payload, cursor + 24);
        let base_required_points_raw = read_i64_le(payload, cursor + 32);
        cursor += 40;
        rows.push(ResearchSlotState {
            tag_db_id,
            slot_index,
            tech_handle_raw,
            tech_def_index_raw,
            progress_raw,
            live_progress_raw: if flags & 1 != 0 {
                Some(live_progress_raw)
            } else {
                None
            },
            base_required_points_raw: if flags & (1 << 1) != 0 {
                Some(base_required_points_raw)
            } else {
                None
            },
        });
    }
    if cursor != payload.len() {
        return Err(format!(
            "tech payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    Ok(rows)
}

fn decode_decisions_payload(payload: &[u8]) -> Result<Vec<DecisionCountryState>, String> {
    if payload.len() < 24 {
        return Err(format!(
            "payload is {} byte(s), expected at least 24",
            payload.len()
        ));
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let timed_count = read_u32_le(payload, 20) as usize;
    let mut cursor = 24usize;
    let mut rows = Vec::with_capacity(country_count);
    for _ in 0..country_count {
        require_remaining(payload, cursor, 32, "decision country row")?;
        let country_flags = read_u32_le(payload, cursor + 4);
        if country_flags & !0b11 != 0 {
            return Err(format!(
                "decision country row has unknown flags {country_flags}"
            ));
        }
        rows.push(DecisionCountryState {
            tag_db_id: read_u32_le(payload, cursor),
            active_timed_count: read_u32_le(payload, cursor + 8),
            targeted_count: read_u32_le(payload, cursor + 12),
            targeted_timed_count: read_u32_le(payload, cursor + 16),
            decisions_taken_bucket_count: read_u32_le(payload, cursor + 20),
            re_enable_count: read_u32_le(payload, cursor + 24),
            timed_live: Vec::new(),
            targeted_timed_live: Vec::new(),
        });
        cursor += 32;
    }
    let mut timed_by_tag: BTreeMap<u32, Vec<TimedDecisionState>> = BTreeMap::new();
    for _ in 0..timed_count {
        require_remaining(payload, cursor, 12, "decision timed row")?;
        let tag_db_id = read_u32_le(payload, cursor);
        let days = read_i32_le(payload, cursor + 4);
        let state = read_i32_le(payload, cursor + 8);
        cursor += 12;
        let decision = read_compact_string(payload, &mut cursor, "decision timed token")?;
        timed_by_tag
            .entry(tag_db_id)
            .or_default()
            .push(TimedDecisionState {
                tag_db_id,
                decision,
                days,
                state,
            });
    }
    let mut targeted_timed_by_tag: BTreeMap<u32, Vec<TargetedTimedDecisionState>> = BTreeMap::new();
    if cursor < payload.len() {
        require_remaining(
            payload,
            cursor,
            4,
            "targeted timed decision extension header",
        )?;
        let targeted_timed_count = read_u32_le(payload, cursor) as usize;
        cursor += 4;
        for _ in 0..targeted_timed_count {
            require_remaining(payload, cursor, 12, "targeted timed decision row")?;
            let tag_db_id = read_u32_le(payload, cursor);
            let target_id = read_u32_le(payload, cursor + 4);
            let flags = read_u32_le(payload, cursor + 8);
            cursor += 12;
            let decision =
                read_compact_string(payload, &mut cursor, "targeted timed decision token")?;
            if decision.is_empty() {
                return Err("targeted timed decision row has no decision token".to_string());
            }
            if flags & !DECISION_TARGET_FLAG_KNOWN_MASK != 0 {
                return Err(format!(
                    "targeted timed decision row has unknown target flags {flags}"
                ));
            }
            let has_country_flag = flags & DECISION_TARGET_FLAG_COUNTRY_TAG != 0;
            let has_record_0x20 = flags & DECISION_TARGET_FLAG_RECORD_0X20 != 0;
            let has_numeric_flag = flags & DECISION_TARGET_FLAG_NUMERIC_ID != 0;
            let has_record_0x24 = flags & DECISION_TARGET_FLAG_RECORD_0X24 != 0;
            if has_country_flag == has_numeric_flag {
                return Err(
                    "targeted timed decision row must have exactly one target kind flag"
                        .to_string(),
                );
            }
            if has_country_flag != has_record_0x20 {
                return Err(
                    "targeted timed decision row has inconsistent country/record+0x20 flags"
                        .to_string(),
                );
            }
            if has_numeric_flag != has_record_0x24 {
                return Err(
                    "targeted timed decision row has inconsistent numeric/record+0x24 flags"
                        .to_string(),
                );
            }
            if has_country_flag
                && (target_id == 0
                    || target_id == DECISION_TARGET_UNKNOWN_ID
                    || target_id > DECISION_TARGET_MAX_TAG_DB_ID)
            {
                return Err(format!(
                    "targeted timed decision row has invalid country target id {target_id}"
                ));
            }
            if has_numeric_flag
                && (target_id == 0
                    || target_id == DECISION_TARGET_UNKNOWN_ID
                    || target_id > DECISION_TARGET_MAX_NUMERIC_ID)
            {
                return Err(format!(
                    "targeted timed decision row has invalid numeric target id {target_id}"
                ));
            }
            targeted_timed_by_tag
                .entry(tag_db_id)
                .or_default()
                .push(TargetedTimedDecisionState {
                    tag_db_id,
                    decision,
                    target_tag_db_id: if has_country_flag {
                        Some(target_id)
                    } else {
                        None
                    },
                    target_numeric_id: if has_numeric_flag {
                        Some(target_id)
                    } else {
                        None
                    },
                });
        }
    }
    if cursor != payload.len() {
        return Err(format!(
            "decisions payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    for row in &mut rows {
        if let Some(timed) = timed_by_tag.remove(&row.tag_db_id) {
            row.timed_live = timed;
        }
        if let Some(targeted_timed) = targeted_timed_by_tag.remove(&row.tag_db_id) {
            row.targeted_timed_live = targeted_timed;
        }
    }
    if !timed_by_tag.is_empty() || !targeted_timed_by_tag.is_empty() {
        return Err("decisions payload contains rows for an undeclared country".to_string());
    }
    Ok(rows)
}

struct H4crRecord<'a> {
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
    payload: &'a [u8],
    next_offset: usize,
}

fn parse_h4cr_record(raw: &[u8], offset: usize) -> Result<H4crRecord<'_>, String> {
    let remaining = raw.len().saturating_sub(offset);
    let header = parse_compact_record_header(&raw[offset..]).map_err(|err| match err {
        RecordFramingError::TruncatedHeader(_) => {
            format!("offset {offset}: truncated H4CR header with {remaining} byte(s)")
        }
        RecordFramingError::BadMagic => format!("offset {offset}: invalid H4CR magic"),
        RecordFramingError::UnsupportedVersion(version) => {
            format!("offset {offset}: unsupported H4CR version {version}")
        }
        RecordFramingError::UnsupportedHeaderLen(header_len) => {
            format!("offset {offset}: unsupported H4CR header length {header_len}")
        }
        RecordFramingError::PayloadTooLarge(_) => format!("offset {offset}: {err}"),
    })?;
    let payload_len = header.payload_len;
    let record_len = header.record_len();
    if record_len > remaining {
        return Err(format!(
            "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
        ));
    }
    let payload_start = offset + H4CR_HEADER_LEN;
    let payload_end = payload_start + payload_len;
    Ok(H4crRecord {
        sequence: header.sequence,
        game_hour_raw: header.game_hour_raw,
        domain_id: header.domain_id,
        record_type: header.record_type,
        record_flags: header.record_flags,
        payload: &raw[payload_start..payload_end],
        next_offset: payload_end,
    })
}

fn canonical_perf_payload(status_flags: u32) -> Vec<u8> {
    let mut payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
    payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
    payload
}

fn canonical_frame_end_payload(emitted_domains: u32, status_flags: u32) -> Vec<u8> {
    let mut payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
    payload[16..20].copy_from_slice(&emitted_domains.to_le_bytes());
    payload[20..24].copy_from_slice(&emitted_domains.to_le_bytes());
    payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
    payload
}

fn append_h4cr_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(H4CR_MAGIC);
    out.extend_from_slice(&H4CR_VERSION.to_le_bytes());
    out.extend_from_slice(&(H4CR_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(payload);
}

fn merge_depth_gate(out: &mut DepthGateSummary, next: DepthGateSummary) {
    out.data_records = out.data_records.saturating_add(next.data_records);
    out.focus_event_rows = out.focus_event_rows.saturating_add(next.focus_event_rows);
    out.focus_completed_exact_rows = out
        .focus_completed_exact_rows
        .saturating_add(next.focus_completed_exact_rows);
    out.focus_completed_inferred_rows = out
        .focus_completed_inferred_rows
        .saturating_add(next.focus_completed_inferred_rows);
    out.focus_completed_unresolved_rows = out
        .focus_completed_unresolved_rows
        .saturating_add(next.focus_completed_unresolved_rows);
    out.research_eta_rows = out.research_eta_rows.saturating_add(next.research_eta_rows);
    out.decision_status_rows = out
        .decision_status_rows
        .saturating_add(next.decision_status_rows);
    out.decision_event_rows = out
        .decision_event_rows
        .saturating_add(next.decision_event_rows);
    out.decision_target_rows = out
        .decision_target_rows
        .saturating_add(next.decision_target_rows);
    out.decision_country_target_rows = out
        .decision_country_target_rows
        .saturating_add(next.decision_country_target_rows);
    out.decision_numeric_target_rows = out
        .decision_numeric_target_rows
        .saturating_add(next.decision_numeric_target_rows);
    out.badge_rows = out.badge_rows.saturating_add(next.badge_rows);
    out.focus_pause_badge_rows = out
        .focus_pause_badge_rows
        .saturating_add(next.focus_pause_badge_rows);
    out.decision_target_badge_rows = out
        .decision_target_badge_rows
        .saturating_add(next.decision_target_badge_rows);
    out.direct_event_badge_rows = out
        .direct_event_badge_rows
        .saturating_add(next.direct_event_badge_rows);
    out.parse_errors.extend(next.parse_errors);
}

fn active_focus(row: &FocusState) -> Option<(u32, String)> {
    row.current
        .as_ref()
        .map(|token| (FOCUS_KIND_NATIONAL, token.clone()))
        .or_else(|| {
            row.current_continuous
                .as_ref()
                .map(|token| (FOCUS_KIND_CONTINUOUS, token.clone()))
        })
}

fn decision_counts(row: &DecisionCountryState) -> [u32; 6] {
    [
        row.active_timed_count,
        row.targeted_count,
        row.targeted_timed_count,
        row.decisions_taken_bucket_count,
        row.re_enable_count,
        row.timed_live.len().min(u32::MAX as usize) as u32,
    ]
}

fn timed_decision_map(row: &DecisionCountryState) -> BTreeMap<String, TimedDecisionState> {
    row.timed_live
        .iter()
        .map(|timed| (timed.decision.clone(), timed.clone()))
        .collect()
}

fn skipped_hours(hours: &[i32]) -> u32 {
    if hours.len() < 2 {
        return 0;
    }
    let mut skipped = 0u32;
    for pair in hours.windows(2) {
        let delta = pair[1].saturating_sub(pair[0]);
        if delta > 1 {
            skipped = skipped.saturating_add((delta - 1) as u32);
        }
    }
    skipped
}

fn ceil_div_i64(numerator: i64, denominator: i64) -> i64 {
    if denominator <= 0 {
        return -1;
    }
    numerator
        .saturating_add(denominator.saturating_sub(1))
        .saturating_div(denominator)
}

fn append_compact_string(out: &mut Vec<u8>, value: Option<&str>) {
    let bytes = value.unwrap_or("").as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..len]);
}

fn read_compact_string(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    require_remaining(bytes, *cursor, 2, label)?;
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    require_remaining(bytes, *cursor, len, label)?;
    let start = *cursor;
    let end = start + len;
    *cursor = end;
    std::str::from_utf8(&bytes[start..end])
        .map(|value| value.to_string())
        .map_err(|err| format!("{label}: invalid utf-8: {err}"))
}

fn read_compact_string_len(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    require_remaining(bytes, *cursor, 2, label)?;
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    require_remaining(bytes, *cursor, len, label)?;
    *cursor += len;
    Ok(len)
}

fn require_remaining(
    bytes: &[u8],
    cursor: usize,
    needed: usize,
    label: &str,
) -> Result<(), String> {
    if bytes.len().saturating_sub(cursor) < needed {
        Err(format!(
            "{label}: need {needed} byte(s), have {}",
            bytes.len().saturating_sub(cursor)
        ))
    } else {
        Ok(())
    }
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32 range"))
}

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("i64 range"))
}

fn json_i32(value: Option<i32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_string_array(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| json_str(value))
        .collect::<Vec<_>>();
    format!("[{}]", items.join(", "))
}

fn json_str(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    /// D45's numeric decision target is a STATE identity, and it carries the
    /// state id space's shared width.
    ///
    /// # Why this pin lives HERE and not in the shared agreement fixture
    ///
    /// `wire_format/tests/producer_consumer_admission_agreement.rs` asserts
    /// that "every consumer applies the shared envelope" over a hand-maintained
    /// list, and `wire_format` does not depend on the lane crates -- so this
    /// constant CANNOT appear in that list. That gap is not academic: it is
    /// exactly why a `1_000_000` ceiling on a state identity survived every
    /// gate and fourteen adversarial review rounds. A list bounded by a crate's
    /// dependency graph cannot honour a claim about every consumer.
    ///
    /// # Why it is a state identity at all
    ///
    /// `TargetedDecisionTarget::Numeric(state_id)` is converted to
    /// `panel::Target::State(state_id)` in
    /// `collector/src/a34_a40_evidence.rs`. The role name "numeric" is what hid
    /// it from a census scoped by the names `MAX_STATE_ID`/`MAX_PROVINCE_ID`.
    ///
    /// kills: reverting `DECISION_TARGET_MAX_NUMERIC_ID` to `1_000_000`,
    /// `65_535`, `70_000` or any other content-scale number; and widening it
    /// past the structural edge, since `2_147_483_647` is asserted refused as a
    /// LITERAL rather than as `MAX + 1`.
    #[test]
    fn the_numeric_decision_target_is_a_state_identity_at_the_shared_width() {
        assert_eq!(
            super::DECISION_TARGET_MAX_NUMERIC_ID,
            compact_contract::build_profile::MAX_STATE_ID,
            "the numeric decision target is a STATE id and must carry the state \
             id space's width, not a private ceiling"
        );
        assert_eq!(super::DECISION_TARGET_MAX_NUMERIC_ID, 2_147_483_646);
        for admitted in [1u32, 65_535, 70_000, 1_000_000, 1_000_001] {
            assert!(
                admitted <= super::DECISION_TARGET_MAX_NUMERIC_ID,
                "state id {admitted} must be admitted as a decision target"
            );
        }
        assert!(2_147_483_647 > super::DECISION_TARGET_MAX_NUMERIC_ID);
    }

    use super::*;

    fn empty_focus_payload() -> Vec<u8> {
        let mut payload = vec![0u8; 28];
        payload[20..24].copy_from_slice(&100_000u32.to_le_bytes());
        payload
    }

    fn empty_tech_payload() -> Vec<u8> {
        let mut payload = vec![0u8; 32];
        payload[20..24].copy_from_slice(&1_000u32.to_le_bytes());
        payload[24..28].copy_from_slice(&100_000u32.to_le_bytes());
        payload
    }

    fn empty_decisions_payload() -> Vec<u8> {
        vec![0u8; 24]
    }

    fn append_source_frame(
        stream: &mut Vec<u8>,
        sequence: &mut u64,
        game_hour_raw: i32,
        focus: &[u8],
        tech: &[u8],
        decisions: &[u8],
    ) {
        append_source_frame_with_status(
            stream,
            sequence,
            game_hour_raw,
            focus,
            tech,
            decisions,
            [0; 3],
            0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn append_source_frame_with_status(
        stream: &mut Vec<u8>,
        sequence: &mut u64,
        game_hour_raw: i32,
        focus: &[u8],
        tech: &[u8],
        decisions: &[u8],
        perf_status: [u32; 3],
        frame_status: u32,
    ) {
        for ((domain_id, payload), status) in [
            (DOMAIN_FOCUS, focus),
            (DOMAIN_TECH, tech),
            (DOMAIN_DECISIONS, decisions),
        ]
        .into_iter()
        .zip(perf_status)
        {
            append_h4cr_record(
                stream,
                *sequence,
                game_hour_raw,
                domain_id,
                RECORD_TYPE_DOMAIN_PERF,
                0,
                &canonical_perf_payload(status),
            );
            *sequence += 1;
            append_h4cr_record(
                stream,
                *sequence,
                game_hour_raw,
                domain_id,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                payload,
            );
            *sequence += 1;
        }
        append_h4cr_record(
            stream,
            *sequence,
            game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(3, frame_status),
        );
        *sequence += 1;
    }

    fn append_depth_frame(
        stream: &mut Vec<u8>,
        sequence: &mut u64,
        game_hour_raw: i32,
        rows: &DepthFrameRows,
        perf_status: u32,
    ) {
        append_h4cr_record(
            stream,
            *sequence,
            game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(perf_status),
        );
        *sequence += 1;
        append_h4cr_record(
            stream,
            *sequence,
            game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &encode_depth_payload_v3(rows, perf_status),
        );
        *sequence += 1;
        append_h4cr_record(
            stream,
            *sequence,
            game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(1, 0),
        );
        *sequence += 1;
    }

    fn append_depth_frame_v2(
        stream: &mut Vec<u8>,
        sequence: &mut u64,
        game_hour_raw: i32,
        rows: &DepthFrameRows,
    ) {
        append_h4cr_record(
            stream,
            *sequence,
            game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(0),
        );
        *sequence += 1;
        append_h4cr_record(
            stream,
            *sequence,
            game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &encode_depth_payload_v2(rows),
        );
        *sequence += 1;
        append_h4cr_record(
            stream,
            *sequence,
            game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(1, 0),
        );
        *sequence += 1;
    }

    #[test]
    fn depth_payload_round_trip_gates_transition_rows() {
        let rows = DepthFrameRows {
            focus_events: vec![FocusEventRow {
                tag_db_id: 47,
                kind: FOCUS_EVENT_COMPLETED_COUNT_INCREASED,
                focus_kind: FOCUS_KIND_NATIONAL,
                flags: FOCUS_FLAG_COMPLETED_TOKEN_INFERRED,
                previous_completed_count: 2,
                current_completed_count: 3,
                previous_progress_raw: 6_900_000,
                current_progress_raw: 0,
                completed_delta: 1,
                from_token: Some("POR_old_focus".to_string()),
                to_token: None,
                completed_token: Some("POR_old_focus".to_string()),
            }],
            research_eta: vec![ResearchEtaRow {
                tag_db_id: 47,
                slot_index: 1,
                flags: RESEARCH_FLAG_SAME_TECH
                    | RESEARCH_FLAG_POSITIVE_SPEED
                    | RESEARCH_FLAG_ETA_AVAILABLE
                    | RESEARCH_FLAG_BASE_PRESENT,
                hour_delta: 24,
                previous_tech_handle: 700,
                current_tech_handle: 700,
                previous_tech_def_index: 700,
                current_tech_def_index: 700,
                previous_live_progress_raw: 100_000,
                current_live_progress_raw: 220_000,
                delta_live_progress_raw: 120_000,
                speed_raw_per_hour: 5_000,
                base_required_points_raw: 1_000_000,
                eta_hours: 156,
            }],
            decision_status: vec![DecisionStatusRow {
                tag_db_id: 47,
                active_timed_count: 1,
                targeted_count: 2,
                targeted_timed_count: 1,
                decisions_taken_bucket_count: 0,
                re_enable_count: 1,
                timed_live_count: 1,
            }],
            decision_events: vec![DecisionEventRow {
                tag_db_id: 47,
                kind: DECISION_EVENT_DAY_DELTA,
                flags: 0,
                previous_days: 44,
                current_days: 43,
                previous_state: 0,
                current_state: 0,
                decision: Some("POR_countdown".to_string()),
            }],
            decision_targets: vec![DecisionTargetRow {
                tag_db_id: 47,
                target_id: 21,
                flags: DECISION_TARGET_FLAG_COUNTRY_TAG | DECISION_TARGET_FLAG_RECORD_0X20,
                source_kind: DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED,
                decision: Some("POR_targeted_timed_country".to_string()),
            }],
            observed_focus_rows: 1,
            observed_research_slot_rows: 1,
            observed_decision_rows: 1,
            observed_timed_decision_rows: 1,
            ..DepthFrameRows::default()
        };
        let payload = encode_depth_payload(&rows);
        let summary = decode_depth_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.focus_event_rows, 1);
        assert_eq!(summary.research_eta_rows, 1);
        assert_eq!(summary.decision_status_rows, 1);
        assert_eq!(summary.decision_event_rows, 1);
        assert_eq!(summary.decision_target_rows, 1);
        assert_eq!(summary.decision_country_target_rows, 1);
        assert_eq!(summary.decision_numeric_target_rows, 0);
        assert_eq!(summary.badge_rows, 3);
        assert_eq!(summary.focus_pause_badge_rows, 1);
        assert_eq!(summary.decision_target_badge_rows, 1);
        assert_eq!(summary.direct_event_badge_rows, 1);
    }

    #[test]
    fn derives_depth_rows_from_base_compact_stream() {
        let mut stream = Vec::new();
        let mut seq = 1u64;

        let focus_1 = focus_payload(FocusState {
            tag_db_id: 47,
            completed_count: Some(2),
            progress_raw: Some(6_800_000),
            current: Some("POR_old_focus".to_string()),
            current_continuous: None,
            completed_tokens: Vec::new(),
        });
        let tech_1 = tech_payload(ResearchSlotState {
            tag_db_id: 47,
            slot_index: 0,
            tech_handle_raw: 700,
            tech_def_index_raw: 700,
            progress_raw: 100,
            live_progress_raw: Some(100_000),
            base_required_points_raw: Some(1_000_000),
        });
        let decisions_1 = decisions_payload(DecisionCountryState {
            tag_db_id: 47,
            active_timed_count: 1,
            targeted_count: 0,
            targeted_timed_count: 0,
            decisions_taken_bucket_count: 0,
            re_enable_count: 0,
            timed_live: vec![TimedDecisionState {
                tag_db_id: 47,
                decision: "POR_countdown".to_string(),
                days: 44,
                state: 0,
            }],
            targeted_timed_live: Vec::new(),
        });
        append_source_frame(&mut stream, &mut seq, 100, &focus_1, &tech_1, &decisions_1);

        let focus_2 = focus_payload(FocusState {
            tag_db_id: 47,
            completed_count: Some(3),
            progress_raw: None,
            current: None,
            current_continuous: None,
            completed_tokens: Vec::new(),
        });
        let tech_2 = tech_payload(ResearchSlotState {
            tag_db_id: 47,
            slot_index: 0,
            tech_handle_raw: 700,
            tech_def_index_raw: 700,
            progress_raw: 200,
            live_progress_raw: Some(220_000),
            base_required_points_raw: Some(1_000_000),
        });
        let decisions_2 = decisions_payload(DecisionCountryState {
            tag_db_id: 47,
            active_timed_count: 1,
            targeted_count: 1,
            targeted_timed_count: 0,
            decisions_taken_bucket_count: 0,
            re_enable_count: 0,
            timed_live: vec![TimedDecisionState {
                tag_db_id: 47,
                decision: "POR_countdown".to_string(),
                days: 43,
                state: 0,
            }],
            targeted_timed_live: Vec::new(),
        });
        append_source_frame(&mut stream, &mut seq, 101, &focus_2, &tech_2, &decisions_2);

        let proof = derive_depth_stream(&stream).expect("derive proof");
        assert!(proof.parse_errors.is_empty(), "{:?}", proof.parse_errors);
        assert!(
            proof.depth_parse_errors.is_empty(),
            "{:?}",
            proof.depth_parse_errors
        );
        assert_eq!(proof.source_frames, 2);
        assert_eq!(proof.skipped_game_hours, 0);
        assert_eq!(proof.focus_event_rows, 2);
        assert_eq!(proof.research_eta_rows, 1);
        assert_eq!(proof.decision_event_rows, 2);
        assert_eq!(proof.decision_status_rows, 2);
        assert_eq!(proof.decision_target_rows, 0);
        assert_eq!(proof.decision_country_target_rows, 0);
        assert_eq!(proof.decision_numeric_target_rows, 0);
        assert_eq!(proof.badge_rows, 6);
        assert_eq!(proof.focus_pause_badge_rows, 2);
        assert_eq!(proof.decision_target_badge_rows, 2);
        assert_eq!(proof.direct_event_badge_rows, 2);
        let gate = gate_existing_depth_stream(&proof.depth_stream, 1);
        assert!(gate.parse_errors.is_empty(), "{:?}", gate.parse_errors);
        assert_eq!(gate.data_records, 2);
        assert_eq!(gate.perf_records, 2);
        assert_eq!(gate.frame_end_records, 2);
        assert_eq!(gate.certified_hours, 2);
    }

    #[test]
    fn authoritative_source_rejects_empty_and_unrelated_only_streams() {
        let empty_error = derive_depth_stream(&[]).expect_err("empty source must fail");
        assert!(empty_error.contains("stream is empty"), "{empty_error}");

        let mut unrelated = Vec::new();
        append_h4cr_record(
            &mut unrelated,
            1,
            100,
            77,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(0),
        );
        append_h4cr_record(
            &mut unrelated,
            2,
            100,
            77,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            b"opaque-unowned-payload",
        );
        append_h4cr_record(
            &mut unrelated,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(1, 0),
        );
        let error = derive_depth_stream(&unrelated).expect_err("unrelated-only source must fail");
        assert!(
            error.contains("owned domain 12 DATA record count 0"),
            "{error}"
        );
        assert!(
            error.contains("owned domain 16 PERF record count 0"),
            "{error}"
        );
    }

    #[test]
    fn authoritative_source_rejects_sequence_prefix_gap_and_wrap() {
        let mut source = Vec::new();
        let mut sequence = 1;
        append_source_frame(
            &mut source,
            &mut sequence,
            100,
            &empty_focus_payload(),
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );

        let mut non_one = source.clone();
        non_one[8..16].copy_from_slice(&2u64.to_le_bytes());
        let error = derive_depth_stream(&non_one).expect_err("non-1 prefix must fail");
        assert!(error.contains("does not start at 1"), "{error}");

        let mut gap = source.clone();
        let second_header = COMPACT_RECORD_HEADER_LEN + COMPACT_DOMAIN_PERF_BASE_LEN;
        gap[second_header + 8..second_header + 16].copy_from_slice(&9u64.to_le_bytes());
        let error = derive_depth_stream(&gap).expect_err("sequence gap must fail");
        assert!(error.contains("sequence discontinuity"), "{error}");

        let mut wrapping = Vec::new();
        append_h4cr_record(
            &mut wrapping,
            1,
            100,
            77,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(0),
        );
        append_h4cr_record(
            &mut wrapping,
            u64::MAX,
            100,
            77,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            b"opaque",
        );
        append_h4cr_record(
            &mut wrapping,
            0,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(1, 0),
        );
        let error = derive_depth_stream(&wrapping).expect_err("sequence wrap must fail");
        assert!(error.contains("sequence space exhausted"), "{error}");
    }

    #[test]
    fn authoritative_source_rejects_illegal_flags_and_malformed_perf() {
        let mut source = Vec::new();
        let mut sequence = 1;
        append_source_frame(
            &mut source,
            &mut sequence,
            100,
            &empty_focus_payload(),
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        source[28..32].copy_from_slice(&1u32.to_le_bytes());
        let error = derive_depth_stream(&source).expect_err("illegal PERF flags must fail");
        assert!(error.contains("rejected bits"), "{error}");

        let mut malformed = Vec::new();
        append_h4cr_record(
            &mut malformed,
            1,
            100,
            DOMAIN_FOCUS,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0u8; COMPACT_DOMAIN_PERF_BASE_LEN - 1],
        );
        let error = derive_depth_stream(&malformed).expect_err("short PERF must fail");
        assert!(error.contains("DOMAIN_PERF payload length"), "{error}");

        let mut malformed_data = Vec::new();
        let mut sequence = 1;
        append_source_frame(
            &mut malformed_data,
            &mut sequence,
            100,
            &[],
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        let error = derive_depth_stream(&malformed_data).expect_err("bad owned DATA must fail");
        assert!(
            error.contains("DATA failed shared authority decode"),
            "{error}"
        );
        assert!(error.contains("expected at least 28"), "{error}");
    }

    #[test]
    fn authoritative_source_rejects_missing_duplicate_and_wrong_hour_owned_pairs() {
        let focus = empty_focus_payload();
        let tech = empty_tech_payload();
        let decisions = empty_decisions_payload();

        let mut missing = Vec::new();
        let mut sequence = 1;
        for (domain_id, payload) in [
            (DOMAIN_FOCUS, focus.as_slice()),
            (DOMAIN_TECH, tech.as_slice()),
            (DOMAIN_DECISIONS, decisions.as_slice()),
        ] {
            if domain_id != DOMAIN_DECISIONS {
                append_h4cr_record(
                    &mut missing,
                    sequence,
                    100,
                    domain_id,
                    RECORD_TYPE_DOMAIN_PERF,
                    0,
                    &canonical_perf_payload(0),
                );
                sequence += 1;
            }
            append_h4cr_record(
                &mut missing,
                sequence,
                100,
                domain_id,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                payload,
            );
            sequence += 1;
        }
        append_h4cr_record(
            &mut missing,
            sequence,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(2, 0),
        );
        let error = derive_depth_stream(&missing).expect_err("missing owned PERF must fail");
        assert!(
            error.contains("owned domain 16 PERF record count 0"),
            "{error}"
        );

        let mut duplicate = Vec::new();
        let mut sequence = 1;
        for (domain_id, payload) in [
            (DOMAIN_FOCUS, focus.as_slice()),
            (DOMAIN_TECH, tech.as_slice()),
            (DOMAIN_DECISIONS, decisions.as_slice()),
        ] {
            append_h4cr_record(
                &mut duplicate,
                sequence,
                100,
                domain_id,
                RECORD_TYPE_DOMAIN_PERF,
                0,
                &canonical_perf_payload(0),
            );
            sequence += 1;
            append_h4cr_record(
                &mut duplicate,
                sequence,
                100,
                domain_id,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                payload,
            );
            sequence += 1;
            if domain_id == DOMAIN_FOCUS {
                append_h4cr_record(
                    &mut duplicate,
                    sequence,
                    100,
                    domain_id,
                    RECORD_TYPE_DOMAIN_DATA,
                    0,
                    payload,
                );
                sequence += 1;
            }
        }
        append_h4cr_record(
            &mut duplicate,
            sequence,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(3, 0),
        );
        let error = derive_depth_stream(&duplicate).expect_err("duplicate DATA must fail");
        assert!(
            error.contains("owned domain 12 DATA record count 2"),
            "{error}"
        );

        let mut wrong_hour = Vec::new();
        append_h4cr_record(
            &mut wrong_hour,
            1,
            99,
            DOMAIN_FOCUS,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(0),
        );
        append_h4cr_record(
            &mut wrong_hour,
            2,
            100,
            DOMAIN_FOCUS,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus,
        );
        let error = derive_depth_stream(&wrong_hour).expect_err("wrong-hour pair must fail");
        assert!(
            error.contains("conflicts with pending PERF hour 99"),
            "{error}"
        );
    }

    #[test]
    fn authoritative_source_rejects_bare_unclosed_skipped_and_trailing_frames() {
        let mut bare = Vec::new();
        append_h4cr_record(
            &mut bare,
            1,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(0, 0),
        );
        let error = derive_depth_stream(&bare).expect_err("bare FRAME_END must fail");
        assert!(error.contains("has no open DATA frame"), "{error}");

        let mut canonical = Vec::new();
        let mut sequence = 1;
        append_source_frame(
            &mut canonical,
            &mut sequence,
            100,
            &empty_focus_payload(),
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        let mut unclosed = canonical.clone();
        unclosed
            .truncate(unclosed.len() - COMPACT_RECORD_HEADER_LEN - COMPACT_FRAME_END_PAYLOAD_LEN);
        let error = derive_depth_stream(&unclosed).expect_err("unclosed frame must fail");
        assert!(
            error.contains("stream ended with open DATA frame"),
            "{error}"
        );

        let mut skipped = canonical.clone();
        append_source_frame(
            &mut skipped,
            &mut sequence,
            102,
            &empty_focus_payload(),
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        let error = derive_depth_stream(&skipped).expect_err("skipped frame hour must fail");
        assert!(error.contains("expected consecutive hour 101"), "{error}");

        let mut reordered = canonical.clone();
        let mut reordered_sequence = sequence - 7;
        append_source_frame(
            &mut reordered,
            &mut reordered_sequence,
            99,
            &empty_focus_payload(),
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        let error = derive_depth_stream(&reordered).expect_err("reordered frame must fail");
        assert!(error.contains("expected consecutive hour 101"), "{error}");

        let mut trailing = canonical;
        trailing.push(0);
        let error = derive_depth_stream(&trailing).expect_err("trailing byte must fail");
        assert!(error.contains("truncated H4CR header"), "{error}");
        assert!(
            error.contains("not closed by a final canonical FRAME_END"),
            "{error}"
        );
    }

    #[test]
    fn depth_gate_requires_certified_d45_pairs_and_normalizes_fatal_status() {
        let mut clean = Vec::new();
        let mut sequence = 1;
        append_depth_frame(
            &mut clean,
            &mut sequence,
            100,
            &DepthFrameRows::default(),
            0,
        );
        append_depth_frame(
            &mut clean,
            &mut sequence,
            101,
            &DepthFrameRows::default(),
            0,
        );
        let clean_summary = gate_existing_depth_stream(&clean, 1);
        assert!(
            clean_summary.parse_errors.is_empty(),
            "{:?}",
            clean_summary.parse_errors
        );
        assert_eq!(clean_summary.certified_hours, 2);
        assert_eq!(clean_summary.fatal_status_flags, 0);

        let mut fatal = Vec::new();
        let mut sequence = 1;
        append_depth_frame(
            &mut fatal,
            &mut sequence,
            100,
            &DepthFrameRows::default(),
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
        );
        append_depth_frame(
            &mut fatal,
            &mut sequence,
            101,
            &DepthFrameRows::default(),
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
        );
        let fatal_summary = gate_existing_depth_stream(&fatal, 1);
        assert_ne!(fatal_summary.fatal_status_flags, 0);
        assert!(fatal_summary
            .parse_errors
            .iter()
            .any(|error| error.contains("fatal status flags")));

        let mut data_only = Vec::new();
        append_h4cr_record(
            &mut data_only,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_depth_payload(&DepthFrameRows::default()),
        );
        append_h4cr_record(
            &mut data_only,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(0, 0),
        );
        let summary = gate_existing_depth_stream(&data_only, 1);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("owned domain 45 PERF record count 0")));
    }

    #[test]
    fn production_duration_requires_positive_advancements_plus_one_closed_frames() {
        let mut one_frame = Vec::new();
        let mut sequence = 1;
        append_depth_frame(
            &mut one_frame,
            &mut sequence,
            200,
            &DepthFrameRows::default(),
            0,
        );

        let nonpositive = gate_existing_depth_stream(&one_frame, 0);
        assert!(nonpositive.parse_errors.iter().any(|error| {
            error.contains("commanded game-hour advancements must be explicitly positive")
        }));

        let short = gate_existing_depth_stream(&one_frame, 1);
        assert!(short.parse_errors.iter().any(|error| {
            error.contains("0 advancement(s) across 1 observed closed frame(s)")
                && error.contains("1 advancement(s) / 2 closed frame(s)")
        }));

        append_depth_frame(
            &mut one_frame,
            &mut sequence,
            201,
            &DepthFrameRows::default(),
            0,
        );
        let exact = gate_existing_depth_stream(&one_frame, 1);
        assert!(!exact
            .parse_errors
            .iter()
            .any(|error| error.contains("production proof duration")));
        assert!(exact.parse_errors.is_empty(), "{:?}", exact.parse_errors);
        assert_eq!(exact.game_hours_advanced, 1);
        assert_eq!(exact.certified_hours, 2);
        assert_eq!(exact.required_closed_frames, 2);

        append_depth_frame(
            &mut one_frame,
            &mut sequence,
            202,
            &DepthFrameRows::default(),
            0,
        );
        let overlong = gate_existing_depth_stream(&one_frame, 1);
        assert!(overlong.parse_errors.iter().any(|error| {
            error.contains("2 advancement(s) across 3 observed closed frame(s)")
                && error.contains("expected exactly 1 advancement(s) / 2 closed frame(s)")
        }));
    }

    #[test]
    fn frame_end_authority_is_bound_to_observed_perf_and_canonical_fields() {
        let cases: [(u32, u32, u32, u32, &str); 5] = [
            (2, 2, 0, 0, "independently observed DOMAIN_PERF count 1"),
            (1, 2, 0, 0, "scheduled_domains 2 != emitted_domains 1"),
            (1, 1, 0, 7, "reserved word is nonzero"),
            (1, 1, 1u32 << 31, 0, "unknown v1 bits"),
            (
                1,
                1,
                compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
                0,
                "fatal global status flags",
            ),
        ];
        for (emitted, scheduled, status, reserved, expected) in cases {
            let mut stream = Vec::new();
            append_h4cr_record(
                &mut stream,
                1,
                300,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                0,
                &canonical_perf_payload(0),
            );
            append_h4cr_record(
                &mut stream,
                2,
                300,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                &encode_depth_payload(&DepthFrameRows::default()),
            );
            let mut frame = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
            frame[16..20].copy_from_slice(&emitted.to_le_bytes());
            frame[20..24].copy_from_slice(&scheduled.to_le_bytes());
            frame[24..28].copy_from_slice(&status.to_le_bytes());
            frame[28..32].copy_from_slice(&reserved.to_le_bytes());
            append_h4cr_record(
                &mut stream,
                3,
                300,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                0,
                &frame,
            );

            let summary = summarize_depth_stream(&stream);
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "expected {expected:?}, errors={:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn production_gate_rejects_statusless_frd2_v2_through_shared_authority() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_depth_frame_v2(&mut stream, &mut sequence, 400, &DepthFrameRows::default());
        append_depth_frame_v2(&mut stream, &mut sequence, 401, &DepthFrameRows::default());
        let summary = gate_existing_depth_stream(&stream, 1);
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("production-owned domain 45 DATA lacks compact status authority")
        }));
        assert_eq!(summary.data_records, 2);
        assert_eq!(summary.certified_hours, 0);
    }

    #[test]
    fn frd2_v3_exposes_raw_status_without_reinterpreting_depth_status() {
        let raw_status = compact_contract::COMPACT_STATUS_DELTA_DATA;
        let payload = encode_depth_payload_v3(&DepthFrameRows::default(), raw_status);
        assert_eq!(&payload[0..4], b"FRD2");
        assert_eq!(read_u16_le(&payload, 4), 3);
        assert_eq!(read_u16_le(&payload, 6), 60);
        assert_eq!(read_u32_le(&payload, 8), DEPTH_STATUS_LEGACY_KNOWN_MASK);
        assert_eq!(
            read_u32_le(&payload, 8) & DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED,
            0,
            "FRD2 v3 must not inherit the schema-v5 direct-hook authority bit"
        );
        assert_eq!(read_u32_le(&payload, 56), raw_status);

        for expected in [0, raw_status] {
            let payload = encode_depth_payload_v3(&DepthFrameRows::default(), expected);
            let authority = summarize_domain_data_payload(
                DOMAIN_ID,
                &payload,
                DomainDataContext {
                    stream_offset: 0,
                    record_flags: compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                    sequence: 1,
                    header_hour_raw: 400,
                    session_header_identity: None,
                },
            )
            .expect("FRD2 v3 shared authority");
            assert_eq!(authority.status_flags, Some(expected));
        }
    }

    #[test]
    fn frd2_v5_normalizer_accepts_empty_process_continuous_sequence_baseline() {
        let batch = direct_frd_transitions_live::DirectEventBatch {
            hook_epoch: 7,
            drain_sequence: 4,
            installed_root_mask:
                direct_frd_transitions_live::D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK,
            observed_kind_mask: 0,
            sequence_cursor: 3,
            producer_complete: true,
            evidence_fixture: false,
            rows: Vec::new(),
        };
        let trailer =
            direct_frd_transitions_live::encode_authoritative_trailer(&batch).unwrap();
        let mut payload = vec![0u8; 64];
        payload[0..4].copy_from_slice(b"FRD2");
        payload[4..6].copy_from_slice(&5u16.to_le_bytes());
        payload[6..8].copy_from_slice(&64u16.to_le_bytes());
        payload[8..12]
            .copy_from_slice(&DEPTH_STATUS_DIRECT_HOOK_SOURCE_VALIDATED.to_le_bytes());
        payload.extend_from_slice(&trailer);

        let normalized = normalize_depth_versioned_payload(&payload)
            .expect("empty process-continuous sequence establishes the lane baseline");
        assert_eq!(normalized.len(), DEPTH_HEADER_LEN);
    }

    #[test]
    fn production_gate_rejects_frd2_v3_data_perf_status_mismatch() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_h4cr_record(
            &mut stream,
            sequence,
            400,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(0),
        );
        sequence += 1;
        append_h4cr_record(
            &mut stream,
            sequence,
            400,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &encode_depth_payload_v3(
                &DepthFrameRows::default(),
                compact_contract::COMPACT_STATUS_DELTA_DATA,
            ),
        );
        sequence += 1;
        append_h4cr_record(
            &mut stream,
            sequence,
            400,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(1, 0),
        );
        sequence += 1;
        append_depth_frame(
            &mut stream,
            &mut sequence,
            401,
            &DepthFrameRows::default(),
            0,
        );

        let summary = gate_existing_depth_stream(&stream, 1);
        let expected_data_status = format!(
            "DATA status {:#010x}",
            compact_contract::COMPACT_STATUS_DELTA_DATA
        );
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains(&expected_data_status)
                && error.contains("does not match PERF status Some(\"0x00000000\")")
        }));
        assert_eq!(summary.certified_hours, 0);
    }

    #[test]
    fn versioned_lane_decoder_rejects_mismatched_and_truncated_v3_headers() {
        let payload = encode_depth_payload_v3(&DepthFrameRows::default(), 0);
        let truncated = decode_depth_record_payload(
            0,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &payload[..56],
        );
        assert!(truncated
            .parse_errors
            .iter()
            .any(|error| error.contains("schema-v3 payload is 56 byte(s), expected at least 60")));

        let mut mismatched = payload;
        mismatched[4..6].copy_from_slice(&2u16.to_le_bytes());
        let summary = decode_depth_record_payload(
            0,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &mismatched,
        );
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| { error.contains("versioned header has version 2 and length 60") }));
    }

    #[test]
    fn production_gate_rejects_legacy_lane_local_payload_wire() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        for hour in [400, 401] {
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                0,
                &canonical_perf_payload(0),
            );
            sequence += 1;
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                &encode_depth_payload(&DepthFrameRows::default()),
            );
            sequence += 1;
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                0,
                &canonical_frame_end_payload(1, 0),
            );
            sequence += 1;
        }

        let summary = gate_existing_depth_stream(&stream, 1);
        assert_eq!(summary.certified_hours, 0);
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("production-owned domain 45 DATA")
                && error.contains("missing the canonical schema-v2 record flag")
        }));
    }

    #[test]
    fn authoritative_gate_rejects_unknown_perf_status_and_cross_hour_probe() {
        let mut unknown_perf = Vec::new();
        append_h4cr_record(
            &mut unknown_perf,
            1,
            500,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(1u32 << 31),
        );
        append_h4cr_record(
            &mut unknown_perf,
            2,
            500,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_depth_payload(&DepthFrameRows::default()),
        );
        append_h4cr_record(
            &mut unknown_perf,
            3,
            500,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &canonical_frame_end_payload(1, 0),
        );
        let unknown_summary = summarize_depth_stream(&unknown_perf);
        assert!(unknown_summary
            .parse_errors
            .iter()
            .any(|error| error.contains("PERF carries unknown v1 status flags")));

        let mut cross_hour_probe = Vec::new();
        append_h4cr_record(
            &mut cross_hour_probe,
            1,
            999,
            DOMAIN_RC_PROBE,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(0),
        );
        append_h4cr_record(
            &mut cross_hour_probe,
            2,
            500,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &canonical_perf_payload(0),
        );
        let probe_summary = summarize_depth_stream(&cross_hour_probe);
        assert!(probe_summary.parse_errors.iter().any(|error| {
            error.contains("DOMAIN_PERF hour 500 conflicts with pending frame hour 999")
        }));
    }

    #[test]
    fn depth_badges_must_exactly_cover_their_authoritative_populations() {
        let mut rows = DepthFrameRows {
            focus_events: vec![FocusEventRow {
                tag_db_id: 47,
                kind: FOCUS_EVENT_CURRENT_STARTED,
                focus_kind: FOCUS_KIND_NATIONAL,
                flags: 0,
                previous_completed_count: 0,
                current_completed_count: 0,
                previous_progress_raw: 0,
                current_progress_raw: 1,
                completed_delta: 0,
                from_token: None,
                to_token: Some("POR_focus".to_string()),
                completed_token: None,
            }],
            decision_status: vec![DecisionStatusRow {
                tag_db_id: 47,
                active_timed_count: 0,
                targeted_count: 1,
                targeted_timed_count: 0,
                decisions_taken_bucket_count: 0,
                re_enable_count: 0,
                timed_live_count: 0,
            }],
            decision_events: vec![DecisionEventRow {
                tag_db_id: 47,
                kind: DECISION_EVENT_STATUS_COUNTS_CHANGED,
                flags: 0,
                previous_days: 0,
                current_days: 0,
                previous_state: 0,
                current_state: 0,
                decision: None,
            }],
            observed_focus_rows: 1,
            observed_decision_rows: 1,
            ..DepthFrameRows::default()
        };
        apply_required_badges(&mut rows);
        for badge in &mut rows.badges {
            badge.affected_count = 0;
        }
        let summary = decode_depth_payload(0, &encode_depth_payload(&rows));
        for expected in [
            "affected_count 0 != observed focus rows 1",
            "affected_count 0 != decision target-body authority 1",
            "affected_count 0 != derived event rows 2",
        ] {
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "expected {expected:?}, errors={:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn source_row_flags_are_rejected_by_shared_authority() {
        let mut focus = focus_payload(FocusState {
            tag_db_id: 47,
            completed_count: Some(1),
            progress_raw: Some(1),
            current: Some("POR_focus".to_string()),
            current_continuous: None,
            completed_tokens: Vec::new(),
        });
        let flags = read_u32_le(&focus, 32) | (1u32 << 30);
        focus[32..36].copy_from_slice(&flags.to_le_bytes());
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_source_frame(
            &mut stream,
            &mut sequence,
            600,
            &focus,
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        let error = derive_depth_stream(&stream).expect_err("unknown source row flags must fail");
        assert!(error.contains("shared authority decode"), "{error}");

        let mut status_mismatch_focus = empty_focus_payload();
        status_mismatch_focus[16..20]
            .copy_from_slice(&compact_contract::COMPACT_STATUS_DELTA_DATA.to_le_bytes());
        let mut mismatch_stream = Vec::new();
        let mut mismatch_sequence = 1;
        append_source_frame(
            &mut mismatch_stream,
            &mut mismatch_sequence,
            601,
            &status_mismatch_focus,
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        let error = derive_depth_stream(&mismatch_stream)
            .expect_err("source DATA/PERF status mismatch must fail");
        assert!(
            error.contains(&format!(
                "DATA status {:#010x}",
                compact_contract::COMPACT_STATUS_DELTA_DATA
            )),
            "{error}"
        );
        assert!(error.contains("does not match PERF status"), "{error}");
    }

    #[test]
    fn derivation_propagates_nonfatal_perf_and_frame_status() {
        let mut source = Vec::new();
        let mut sequence = 1;
        let mut focus = empty_focus_payload();
        focus[16..20].copy_from_slice(&compact_contract::COMPACT_STATUS_DELTA_DATA.to_le_bytes());
        append_source_frame_with_status(
            &mut source,
            &mut sequence,
            100,
            &focus,
            &empty_tech_payload(),
            &empty_decisions_payload(),
            [compact_contract::COMPACT_STATUS_DELTA_DATA, 0, 0],
            compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE,
        );
        let proof = derive_depth_stream(&source).expect("nonfatal status source must derive");
        let summary = summarize_depth_stream(&proof.depth_stream);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_ne!(
            summary.status_flags_or & compact_contract::COMPACT_STATUS_DELTA_DATA,
            0
        );
        assert_ne!(
            summary.status_flags_or & compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE,
            0
        );
        assert_eq!(summary.fatal_status_flags, 0);

        let mut fatal_source = Vec::new();
        let mut sequence = 1;
        append_source_frame_with_status(
            &mut fatal_source,
            &mut sequence,
            100,
            &empty_focus_payload(),
            &empty_tech_payload(),
            &empty_decisions_payload(),
            [compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE, 0, 0],
            0,
        );
        let error = derive_depth_stream(&fatal_source).expect_err("fatal source status must fail");
        assert!(
            error.contains("domain 12 PERF carries fatal status"),
            "{error}"
        );
    }

    #[test]
    fn unresolved_focus_completion_gates_clean_and_counts_unresolved() {
        // Emission/gate invariant regression for the 2026-07-09 speed-5
        // production stream-corruption bug. A country's completed-focus count
        // advances (43 -> 44) while it has no active national or continuous
        // focus at either bracketing sample. The encoder can name no token, so
        // it emits a COMPLETED_COUNT_INCREASED row with from/to/completed all
        // empty -- the focus_completed_unresolved_rows state. The depth gate
        // must accept it (no data loss, no fabricated token) rather than
        // reporting "focus depth row N has no token context", which is what made
        // the production compact stream NOT DECODABLE.
        let mut stream = Vec::new();
        let mut sequence = 1;
        let focus_1 = focus_payload(FocusState {
            tag_db_id: 8,
            completed_count: Some(43),
            progress_raw: None,
            current: None,
            current_continuous: None,
            completed_tokens: Vec::new(),
        });
        append_source_frame(
            &mut stream,
            &mut sequence,
            100,
            &focus_1,
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );
        let focus_2 = focus_payload(FocusState {
            tag_db_id: 8,
            completed_count: Some(44),
            progress_raw: None,
            current: None,
            current_continuous: None,
            completed_tokens: Vec::new(),
        });
        append_source_frame(
            &mut stream,
            &mut sequence,
            101,
            &focus_2,
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );

        let proof = derive_depth_stream(&stream).expect("derive proof");
        assert!(proof.parse_errors.is_empty(), "{:?}", proof.parse_errors);
        assert!(
            proof.depth_parse_errors.is_empty(),
            "{:?}",
            proof.depth_parse_errors
        );
        // The completion event is retained (not dropped) and classified as
        // unresolved (neither exact nor inferred token).
        assert_eq!(proof.focus_event_rows, 1);
        assert_eq!(proof.focus_completed_unresolved_rows, 1);
        assert_eq!(proof.focus_completed_exact_rows, 0);
        assert_eq!(proof.focus_completed_inferred_rows, 0);
    }

    #[test]
    fn tokenless_non_completion_row_still_fails_closed() {
        // Corruption guard: the token-less exemption is scoped to
        // COMPLETED_COUNT_INCREASED. A CURRENT_STARTED row (kind 2) with no
        // tokens cannot be produced by the encoder and must still be rejected.
        let rows = DepthFrameRows {
            status_flags: 0,
            focus_events: vec![FocusEventRow {
                tag_db_id: 8,
                kind: FOCUS_EVENT_CURRENT_STARTED,
                focus_kind: FOCUS_KIND_NATIONAL,
                flags: 0,
                previous_completed_count: 10,
                current_completed_count: 10,
                previous_progress_raw: 0,
                current_progress_raw: 0,
                completed_delta: 0,
                from_token: None,
                to_token: None,
                completed_token: None,
            }],
            observed_focus_rows: 1,
            ..DepthFrameRows::default()
        };
        let payload = encode_depth_payload(&rows);
        let summary = decode_depth_payload(0, &payload);
        assert!(
            summary
                .parse_errors
                .iter()
                .any(|err| err.contains("focus depth row 0 has no token context")),
            "expected fail-closed on token-less non-completion, got {:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn tokenless_inferred_completion_row_still_fails_closed() {
        // Corruption guard: an INFERRED-flagged completion with no token bytes is
        // inconsistent (the encoder sets INFERRED only when it attaches a token)
        // and must stay a hard error, not be swallowed by the exemption.
        let rows = DepthFrameRows {
            status_flags: 0,
            focus_events: vec![FocusEventRow {
                tag_db_id: 8,
                kind: FOCUS_EVENT_COMPLETED_COUNT_INCREASED,
                focus_kind: 0,
                flags: FOCUS_FLAG_COMPLETED_TOKEN_INFERRED,
                previous_completed_count: 43,
                current_completed_count: 44,
                previous_progress_raw: 0,
                current_progress_raw: 0,
                completed_delta: 1,
                from_token: None,
                to_token: None,
                completed_token: None,
            }],
            observed_focus_rows: 1,
            ..DepthFrameRows::default()
        };
        let payload = encode_depth_payload(&rows);
        let summary = decode_depth_payload(0, &payload);
        assert!(
            summary
                .parse_errors
                .iter()
                .any(|err| err.contains("focus depth row 0 has no token context")),
            "expected fail-closed on inferred-flag token-less completion, got {:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn derives_targeted_timed_target_rows_from_extended_decisions_payload() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        let decisions = decisions_payload(DecisionCountryState {
            tag_db_id: 47,
            active_timed_count: 0,
            targeted_count: 0,
            targeted_timed_count: 2,
            decisions_taken_bucket_count: 0,
            re_enable_count: 0,
            timed_live: Vec::new(),
            targeted_timed_live: vec![
                TargetedTimedDecisionState {
                    tag_db_id: 47,
                    decision: "POR_targeted_country_decision".to_string(),
                    target_tag_db_id: Some(21),
                    target_numeric_id: None,
                },
                TargetedTimedDecisionState {
                    tag_db_id: 47,
                    decision: "POR_second_country_target_decision".to_string(),
                    target_tag_db_id: Some(22),
                    target_numeric_id: None,
                },
            ],
        });
        append_source_frame(
            &mut stream,
            &mut sequence,
            200,
            &empty_focus_payload(),
            &empty_tech_payload(),
            &decisions,
        );

        let proof = derive_depth_stream(&stream).expect("derive proof");
        assert!(proof.parse_errors.is_empty(), "{:?}", proof.parse_errors);
        assert!(
            proof.depth_parse_errors.is_empty(),
            "{:?}",
            proof.depth_parse_errors
        );
        assert_eq!(proof.source_targeted_timed_decision_rows, 2);
        assert_eq!(proof.decision_status_rows, 1);
        assert_eq!(proof.decision_target_rows, 2);
        assert_eq!(proof.decision_country_target_rows, 2);
        assert_eq!(proof.decision_numeric_target_rows, 0);
        assert_eq!(proof.decision_target_badge_rows, 1);
    }

    #[test]
    fn derives_exact_focus_completion_from_completed_token_delta() {
        let mut stream = Vec::new();
        let mut seq = 1u64;

        let focus_1 = focus_payload(FocusState {
            tag_db_id: 47,
            completed_count: Some(1),
            progress_raw: Some(6_800_000),
            current: Some("POR_old_focus".to_string()),
            current_continuous: None,
            completed_tokens: vec!["POR_prior_focus".to_string()],
        });
        append_source_frame(
            &mut stream,
            &mut seq,
            100,
            &focus_1,
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );

        let focus_2 = focus_payload(FocusState {
            tag_db_id: 47,
            completed_count: Some(2),
            progress_raw: None,
            current: None,
            current_continuous: None,
            completed_tokens: vec![
                "POR_prior_focus".to_string(),
                "POR_newly_completed_focus".to_string(),
            ],
        });
        append_source_frame(
            &mut stream,
            &mut seq,
            101,
            &focus_2,
            &empty_tech_payload(),
            &empty_decisions_payload(),
        );

        let proof = derive_depth_stream(&stream).expect("derive proof");
        assert!(proof.parse_errors.is_empty(), "{:?}", proof.parse_errors);
        assert!(
            proof.depth_parse_errors.is_empty(),
            "{:?}",
            proof.depth_parse_errors
        );
        assert_eq!(proof.source_frames, 2);
        assert_eq!(proof.focus_event_rows, 2);
        assert_eq!(proof.focus_completed_exact_rows, 1);
        assert_eq!(proof.focus_completed_inferred_rows, 0);
        assert_eq!(proof.focus_completed_unresolved_rows, 0);
    }

    #[test]
    fn depth_gate_rejects_bad_research_delta() {
        let rows = DepthFrameRows {
            research_eta: vec![ResearchEtaRow {
                tag_db_id: 47,
                slot_index: 0,
                flags: RESEARCH_FLAG_SAME_TECH,
                hour_delta: 1,
                previous_tech_handle: 700,
                current_tech_handle: 700,
                previous_tech_def_index: 700,
                current_tech_def_index: 700,
                previous_live_progress_raw: 100,
                current_live_progress_raw: 90,
                delta_live_progress_raw: 1,
                speed_raw_per_hour: 1,
                base_required_points_raw: 0,
                eta_hours: -1,
            }],
            ..DepthFrameRows::default()
        };
        let payload = encode_depth_payload(&rows);
        let summary = decode_depth_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("delta does not match progress")));
    }

    #[test]
    fn depth_gate_rejects_bad_decision_country_target_row() {
        let rows = DepthFrameRows {
            decision_status: vec![DecisionStatusRow {
                tag_db_id: 47,
                active_timed_count: 0,
                targeted_count: 0,
                targeted_timed_count: 1,
                decisions_taken_bucket_count: 0,
                re_enable_count: 0,
                timed_live_count: 0,
            }],
            decision_targets: vec![DecisionTargetRow {
                tag_db_id: 47,
                target_id: DECISION_TARGET_UNKNOWN_ID,
                flags: DECISION_TARGET_FLAG_COUNTRY_TAG,
                source_kind: DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED,
                decision: Some("POR_targeted_country_decision".to_string()),
            }],
            observed_decision_rows: 1,
            ..DepthFrameRows::default()
        };
        let summary = decode_depth_payload(13, &encode_depth_payload(&rows));
        assert!(summary.parse_errors.iter().any(|err| {
            err.contains("decision target row 0") && err.contains("invalid country target id")
        }));
        assert!(summary.parse_errors.iter().any(|err| {
            err.contains("decision target row 0")
                && err.contains("missing required country/record+0x20 flags")
        }));
    }

    #[test]
    fn decisions_payload_rejects_bad_target_extension_flags() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b11u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&21u32.to_le_bytes());
        payload.extend_from_slice(&DECISION_TARGET_FLAG_COUNTRY_TAG.to_le_bytes());
        append_compact_string(&mut payload, Some("POR_targeted_country_decision"));

        let err = decode_decisions_payload(&payload).expect_err("bad target flags rejected");
        assert!(err.contains("inconsistent country/record+0x20 flags"));

        payload[68..72].copy_from_slice(&0u32.to_le_bytes());
        let err = decode_decisions_payload(&payload).expect_err("missing target kind rejected");
        assert!(err.contains("exactly one target kind flag"));
    }

    #[test]
    fn depth_gate_requires_badges_for_fail_closed_status_flags() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&DEPTH_STATUS_FOCUS_PAUSE_BADGED.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        let summary = decode_depth_payload(11, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("focus pause") && err.contains("lacks badge")));
    }

    #[test]
    fn depth_gate_rejects_omitted_required_status_flags() {
        let mut payload = encode_depth_payload(&DepthFrameRows::default());
        payload[8..12].copy_from_slice(&0u32.to_le_bytes());
        let summary = decode_depth_payload(17, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("lack a complete exact or legacy authority set")));
    }

    #[test]
    fn depth_gate_validates_badge_detail_tokens() {
        let rows = DepthFrameRows {
            badges: vec![BadgeRow {
                kind: BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED,
                flags: BADGE_FLAG_FAIL_CLOSED | BADGE_FLAG_CURRENT_BUILD_PROBED,
                source_domain_id: DOMAIN_FOCUS as u32,
                affected_count: 0,
                evidence_code: BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH,
                detail: Some("wrong_focus_pause_badge_detail".to_string()),
            }],
            ..DepthFrameRows::default()
        };
        let payload = encode_depth_payload(&rows);
        let summary = decode_depth_payload(19, &payload);
        assert!(summary.parse_errors.iter().any(
            |err| err.contains("focus pause badge") && err.contains("inconsistent provenance")
        ));
    }

    fn focus_payload(row: FocusState) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&100_000u32.to_le_bytes());
        payload.extend_from_slice(&(row.completed_tokens.len() as u32).to_le_bytes());
        let mut flags = 1u32;
        if row.completed_count.is_some() {
            flags |= 1 << 1;
        }
        if row.current.is_some() {
            flags |= 1 << 2;
        }
        if row.current_continuous.is_some() {
            flags |= 1 << 3;
        }
        if row.progress_raw.is_some() {
            flags |= 1 << 4;
        }
        if !row.completed_tokens.is_empty() {
            flags |= 1 << 5;
        }
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.completed_count.unwrap_or(u32::MAX).to_le_bytes());
        payload.extend_from_slice(&row.progress_raw.unwrap_or(0).to_le_bytes());
        append_compact_string(&mut payload, Some("portugal_focus"));
        append_compact_string(&mut payload, row.current.as_deref());
        append_compact_string(&mut payload, row.current_continuous.as_deref());
        for token in row.completed_tokens {
            payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
            append_compact_string(&mut payload, Some(&token));
        }
        payload
    }

    fn tech_payload(row: ResearchSlotState) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1_000u32.to_le_bytes());
        payload.extend_from_slice(&100_000u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&0b111u32.to_le_bytes());
        payload.extend_from_slice(&10u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.slot_index.to_le_bytes());
        payload.extend_from_slice(&row.tech_handle_raw.to_le_bytes());
        payload.extend_from_slice(&row.tech_def_index_raw.to_le_bytes());
        payload.extend_from_slice(&row.progress_raw.to_le_bytes());
        let mut flags = 0u32;
        if row.live_progress_raw.is_some() {
            flags |= 1;
        }
        if row.base_required_points_raw.is_some() {
            flags |= 1 << 1;
        }
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.live_progress_raw.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.base_required_points_raw.unwrap_or(0).to_le_bytes());
        payload
    }

    fn decisions_payload(row: DecisionCountryState) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&(row.timed_live.len() as u32).to_le_bytes());
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&0b11u32.to_le_bytes());
        payload.extend_from_slice(&row.active_timed_count.to_le_bytes());
        payload.extend_from_slice(&row.targeted_count.to_le_bytes());
        payload.extend_from_slice(&row.targeted_timed_count.to_le_bytes());
        payload.extend_from_slice(&row.decisions_taken_bucket_count.to_le_bytes());
        payload.extend_from_slice(&row.re_enable_count.to_le_bytes());
        payload.extend_from_slice(&(row.timed_live.len() as u32).to_le_bytes());
        for timed in row.timed_live {
            payload.extend_from_slice(&timed.tag_db_id.to_le_bytes());
            payload.extend_from_slice(&timed.days.to_le_bytes());
            payload.extend_from_slice(&timed.state.to_le_bytes());
            append_compact_string(&mut payload, Some(&timed.decision));
        }
        if !row.targeted_timed_live.is_empty() {
            payload.extend_from_slice(&(row.targeted_timed_live.len() as u32).to_le_bytes());
            for targeted in row.targeted_timed_live {
                payload.extend_from_slice(&targeted.tag_db_id.to_le_bytes());
                let (target_id, flags) = if let Some(target_tag_db_id) = targeted.target_tag_db_id {
                    (
                        target_tag_db_id,
                        DECISION_TARGET_FLAG_COUNTRY_TAG | DECISION_TARGET_FLAG_RECORD_0X20,
                    )
                } else if let Some(target_numeric_id) = targeted.target_numeric_id {
                    (
                        target_numeric_id,
                        DECISION_TARGET_FLAG_NUMERIC_ID | DECISION_TARGET_FLAG_RECORD_0X24,
                    )
                } else {
                    (DECISION_TARGET_UNKNOWN_ID, 0)
                };
                payload.extend_from_slice(&target_id.to_le_bytes());
                payload.extend_from_slice(&flags.to_le_bytes());
                append_compact_string(&mut payload, Some(&targeted.decision));
            }
        }
        payload
    }
}
