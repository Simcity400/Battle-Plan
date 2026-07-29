use std::collections::{BTreeMap, BTreeSet};

use compact_contract::{parse_compact_record_header, RecordFramingError};

#[cfg(windows)]
pub mod a729_hook;
#[cfg(windows)]
pub mod terminal_coordinator;
#[cfg(windows)]
pub mod x2_carriers;

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const DOMAIN_LAND_COMBAT_DETAIL_EVENTS: u16 = 55;
pub const DOMAIN_LAND_COMBAT_DETAIL_EVENTS_NAME: &str = "land_combat_detail_events_live";

// Schema v2 (E2) appends a per-side combat DETAIL row family AFTER the event
// rows, extending the header with a side-detail count. v1 payloads (no side
// detail) remain decodable: a v1 header omits the trailing count field, so the
// version gate selects the header/parse shape (never a payload-byte sniff).
pub const SCHEMA_VERSION: u16 = 8;
pub const SCHEMA_VERSION_V1_NO_SIDE_DETAIL: u16 = 1;
pub const SCHEMA_VERSION_V2_SIDE_DETAIL: u16 = 2;
pub const SCHEMA_VERSION_V3_ATTRIBUTION: u16 = 3;
pub const SCHEMA_VERSION_V4_TACTIC_PROGRESS: u16 = 4;
pub const SCHEMA_VERSION_V5_RESERVE_MEMBERS: u16 = 5;
pub const SCHEMA_VERSION_V6_TERMINAL_RECORDS: u16 = 6;
pub const SCHEMA_VERSION_V7_WIDTH_LOSS: u16 = 7;
/// v1 header (event rows only). Kept for decoding older streams.
pub const PAYLOAD_HEADER_LEN_V1: usize = 28;
/// v2 header adds a u32 side_detail_row_count after the event-row count block.
pub const PAYLOAD_HEADER_LEN_V2: usize = 32;
/// v3/v4 header adds a u32 attribution_row_count after the side-detail count.
/// In v4 the existing string_count word describes the canonical tactic string
/// table appended after all fixed rows.
pub const PAYLOAD_HEADER_LEN_V4: usize = 36;
/// v5 appends the exact reserve-member row count after attribution_row_count.
pub const PAYLOAD_HEADER_LEN_V5: usize = 40;
/// v6 appends the authenticated terminal-record row count.
pub const PAYLOAD_HEADER_LEN_V6: usize = 44;
/// v7 appends exact-call width and current-epoch loss observation counts. v8
/// assigns the former trailing reserved word to authenticated combat-panel
/// side observations, retaining the same 52-byte header.
pub const PAYLOAD_HEADER_LEN: usize = 52;
pub const DETAIL_EVENT_ROW_LEN: usize = 52;

// --- Per-side combat DETAIL row (E2, CCombatSide scalars) -----------------
//
// Current-state (NOT delta): one row per active combat per side, emitted every
// frame for every live combat, so the fold always has the latest per-side
// scalars regardless of whether the participant/phase delta changed this hour.
// Offsets validated 50/50 on both sides vs the melt (FINAL_SWEEP §T2). Damage
// fields are i64 fixed-point ×100000; counts are u32. Schema v4 additionally
// carries the exact progress accumulator and the validated tactic identity.
// `losses` remains undisambiguated on the all-zero-loss oracle and is omitted.
/// Side-detail row wire length. Layout (all little-endian):
///   combat_id u32 | generation u64 | side u32 | flags u32           (20)
///   units u32 | committed u32 | reserve u32 | air_kills u32
///     | org_loss_summary_index u32 | num_org_losses u32             (+24 = 44)
///   7 × i64 damage raws: anti_air, air_str, air_org, ground_str,
///     ground_org, prevented_str, prevented_org                      (+56 = 100)
pub const SIDE_DETAIL_ROW_LEN_V3: usize = 100;
/// v4 adds progress_raw i32, tactic_id i32, tactic_token_index u32, and
/// tactic_name_index u32. String indices address the canonical payload table.
pub const SIDE_DETAIL_ROW_LEN: usize = 116;
pub const SIDE_DETAIL_FIXED_SCALE: i64 = 100_000;
pub const MAX_TACTIC_STRING_COUNT: usize = 16_000;
pub const MAX_TACTIC_STRING_LEN: usize = 256;
pub const TACTIC_STRING_INDEX_NONE: u32 = u32::MAX;

pub const SIDE_DETAIL_FLAG_ATTACKER: u32 = 1 << 0;
pub const SIDE_DETAIL_FLAG_DEFENDER: u32 = 1 << 1;
/// The CCombatSide vtable + registry back-ref revalidated for this side.
pub const SIDE_DETAIL_FLAG_SIDE_VALIDATED: u32 = 1 << 2;
/// tactic is unresolved (pointer/table-backed) — never emitted as a value.
pub const SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED: u32 = 1 << 3;
/// losses is not disambiguable on this save (all-zero oracle) — omitted.
pub const SIDE_DETAIL_FLAG_LOSSES_OMITTED: u32 = 1 << 4;
/// The count triple was read STABLY (each field double-read agreed) but the
/// `committed + reserve == units` invariant did not hold — i.e. a genuine
/// mid-combat transient (a division joining/leaving between the engine's own
/// bookkeeping), NOT a torn read. The row is retained (dropping it would lose
/// the side exactly at the interesting transition) but badged so a consumer
/// treats the count triple as momentarily inconsistent, not authoritative.
pub const SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE: u32 = 1 << 5;
pub const SIDE_DETAIL_FLAG_PROGRESS_PRESENT: u32 = 1 << 6;
pub const SIDE_DETAIL_FLAG_TACTIC_KNOWN: u32 = 1 << 7;
pub const SIDE_DETAIL_FLAG_TACTIC_PRESENT: u32 = 1 << 8;
/// Schema v5 proves that the reserve-member vector was read stably. When this
/// bit is absent, reserve_member_rows are unknown rather than known-empty.
pub const SIDE_DETAIL_FLAG_RESERVE_MEMBERS_KNOWN: u32 = 1 << 9;
pub const SIDE_DETAIL_KNOWN_FLAG_MASK: u32 = SIDE_DETAIL_FLAG_ATTACKER
    | SIDE_DETAIL_FLAG_DEFENDER
    | SIDE_DETAIL_FLAG_SIDE_VALIDATED
    | SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED
    | SIDE_DETAIL_FLAG_LOSSES_OMITTED
    | SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE
    | SIDE_DETAIL_FLAG_PROGRESS_PRESENT
    | SIDE_DETAIL_FLAG_TACTIC_KNOWN
    | SIDE_DETAIL_FLAG_TACTIC_PRESENT
    | SIDE_DETAIL_FLAG_RESERVE_MEMBERS_KNOWN;

/// Sanity ceiling on a fixed-point damage accumulator (±~9.2e12 raw = ±92M
/// after the /1e5 scale — generous for a single combat, still rejects garbage).
pub const MAX_SIDE_DETAIL_DAMAGE_RAW: i64 = 9_200_000_000_000;
pub const MAX_SIDE_DETAIL_COUNT: u32 = 100_000;

pub const ATTRIBUTION_ROW_LEN: usize = 40;
pub const ATTRIBUTION_FIXED_SCALE: u64 = 100_000;
pub const ATTRIBUTION_FLAG_ATTACKER: u32 = 1 << 0;
pub const ATTRIBUTION_FLAG_DEFENDER: u32 = 1 << 1;
pub const ATTRIBUTION_FLAG_SIDE_VALIDATED: u32 = 1 << 2;
pub const ATTRIBUTION_FLAG_DAMAGE_ROW_VALIDATED: u32 = 1 << 3;
pub const ATTRIBUTION_FLAG_DAMAGE_FIXED_POINT: u32 = 1 << 4;
pub const ATTRIBUTION_KNOWN_FLAG_MASK: u32 = ATTRIBUTION_FLAG_ATTACKER
    | ATTRIBUTION_FLAG_DEFENDER
    | ATTRIBUTION_FLAG_SIDE_VALIDATED
    | ATTRIBUTION_FLAG_DAMAGE_ROW_VALIDATED
    | ATTRIBUTION_FLAG_DAMAGE_FIXED_POINT;
pub const MAX_ATTRIBUTION_DAMAGE_RAW: u64 = MAX_SIDE_DETAIL_DAMAGE_RAW as u64;
pub const MAX_COUNTRY_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;

pub const STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
pub const STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const STATUS_TRANSITION_BASELINE_RESET: u32 =
    compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET;
pub const FAILURE_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_FAILURE_MASK;
/// D55 uses only the shared compact-status vocabulary (bits 0..=13); it has no
/// lane-local aliases. Unknown bits must not silently become non-fatal badges.
pub const D55_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

pub const EVENT_PHASE_STARTED: u32 = 1;
pub const EVENT_PHASE_UPDATED: u32 = 2;
pub const EVENT_PHASE_ENDED: u32 = 3;
pub const EVENT_PARTICIPANT_JOINED: u32 = 4;
pub const EVENT_PARTICIPANT_LEFT_UNCLASSIFIED: u32 = 5;

pub const SIDE_NONE: u32 = 0;
pub const SIDE_ATTACKER: u32 = 1;
pub const SIDE_DEFENDER: u32 = 2;

pub const FLAG_LOCATION_PRESENT: u32 = 1 << 0;
pub const FLAG_DAY_PRESENT: u32 = 1 << 1;
pub const FLAG_DURATION_PRESENT: u32 = 1 << 2;
pub const FLAG_SIDE_PRESENT: u32 = 1 << 3;
pub const FLAG_DIVISION_PRESENT: u32 = 1 << 4;
pub const FLAG_FROM_VALIDATED_DOMAIN21_ROOT: u32 = 1 << 5;
pub const FLAG_SEMANTIC_UNCLASSIFIED: u32 = 1 << 6;
pub const KNOWN_FLAG_MASK: u32 = FLAG_LOCATION_PRESENT
    | FLAG_DAY_PRESENT
    | FLAG_DURATION_PRESENT
    | FLAG_SIDE_PRESENT
    | FLAG_DIVISION_PRESENT
    | FLAG_FROM_VALIDATED_DOMAIN21_ROOT
    | FLAG_SEMANTIC_UNCLASSIFIED;

pub const MAX_COMBAT_ID: u32 = 0x7fff_ffff;
pub const MAX_DIVISION_ID: u32 = 0x7fff_ffff;
pub const MAX_LOCATION_ID: u32 = 1_000_000;
pub const MAX_AGE_DAYS: u32 = 100_000;
pub const MAX_SIDE_UNITS: usize = 128;
/// The shared collector admits at most 4,000 land combats. In the most
/// adversarial one-hour transition, both 128-entry side vectors can be wholly
/// replaced (four participant events per side slot) plus one phase event per
/// combat. Bound decoded allocations to that production topology.
pub const MAX_DETAIL_EVENT_ROWS: usize = 4_000 * (4 * MAX_SIDE_UNITS + 1);
/// Two sides per combat; bound the side-detail family the same as MAX_LAND_COMBATS.
pub const MAX_LAND_COMBATS_SIDE_DETAIL: usize = 8_000;
pub const MAX_LAND_COMBATS_ATTRIBUTION: usize = 32_000;
pub const MAX_LAND_COMBATS_RESERVE_MEMBERS: usize = 8_000 * MAX_SIDE_UNITS;
pub const MAX_LAND_COMBAT_TERMINAL_ROWS: usize = 8_000;
pub const MAX_LAND_COMBAT_WIDTH_ROWS: usize = 8_000 * MAX_SIDE_UNITS;
pub const MAX_LAND_COMBAT_LOSS_ROWS: usize = 8_000 * MAX_SIDE_UNITS;
pub const MAX_LAND_COMBAT_PANEL_ROWS: usize = 8_000;

/// One pointer-free observation sealed by the exact land-combat side panel
/// materializer at 0x1415C9860. The nested 0x1415BBF00 appends prove the exact
/// displayed modifier set/order, and 0x14129E190 supplies the selected leader.
pub const PANEL_OBSERVATION_ROW_LEN: usize = 40;
pub const PANEL_FLAG_OUTER_SENTINEL: u32 = 1 << 0;
pub const PANEL_FLAG_MODIFIER_SENTINEL: u32 = 1 << 1;
pub const PANEL_FLAG_LEADER_SENTINEL: u32 = 1 << 2;
pub const PANEL_FLAG_ACTIVE_EPOCH_BOUND: u32 = 1 << 3;
pub const PANEL_FLAG_COMPLETE_SCOPE: u32 = 1 << 4;
pub const PANEL_FLAG_LEADER_IDENTITY: u32 = 1 << 5;
pub const PANEL_FLAG_ENGINE_ORDER: u32 = 1 << 6;
pub const PANEL_REQUIRED_FLAG_MASK: u32 = PANEL_FLAG_OUTER_SENTINEL
    | PANEL_FLAG_MODIFIER_SENTINEL
    | PANEL_FLAG_LEADER_SENTINEL
    | PANEL_FLAG_ACTIVE_EPOCH_BOUND
    | PANEL_FLAG_COMPLETE_SCOPE
    | PANEL_FLAG_LEADER_IDENTITY
    | PANEL_FLAG_ENGINE_ORDER;
pub const PANEL_MODIFIER_KNOWN_MASK: u32 = 0x03ff;
/// Exact append order in 0x1415C9860, expressed as tooltip enum ids.
pub const PANEL_MODIFIER_CANONICAL_ORDER: [u8; 10] = [8, 2, 1, 0, 3, 4, 5, 6, 7, 9];

/// One row per exact `wrapper+0x170` return in an authenticated synchronous
/// `0x141299610` call. The final producer return is repeated for a mechanical
/// grouped sum check. No process pointer crosses this boundary.
pub const WIDTH_OBSERVATION_ROW_LEN: usize = 56;
pub const WIDTH_FLAG_PRODUCER_SENTINEL: u32 = 1 << 0;
pub const WIDTH_FLAG_LEAF_SENTINEL: u32 = 1 << 1;
pub const WIDTH_FLAG_ACTIVE_EPOCH_BOUND: u32 = 1 << 2;
pub const WIDTH_FLAG_WRAPPER_IDENTITY: u32 = 1 << 3;
pub const WIDTH_FLAG_FINAL_SUM_EXACT: u32 = 1 << 4;
pub const WIDTH_REQUIRED_FLAG_MASK: u32 = WIDTH_FLAG_PRODUCER_SENTINEL
    | WIDTH_FLAG_LEAF_SENTINEL
    | WIDTH_FLAG_ACTIVE_EPOCH_BOUND
    | WIDTH_FLAG_WRAPPER_IDENTITY
    | WIDTH_FLAG_FINAL_SUM_EXACT;

/// One row per changed equipment identity in one exact current-epoch writer
/// observation. Manpower before/after is repeated so each grouped row remains
/// independently auditable. The carrier remains inactive pending X2-LOSS.
pub const CURRENT_LOSS_OBSERVATION_ROW_LEN: usize = 72;
pub const CURRENT_LOSS_FLAG_WRITER_SENTINEL: u32 = 1 << 0;
pub const CURRENT_LOSS_FLAG_ACTIVE_EPOCH_BOUND: u32 = 1 << 1;
pub const CURRENT_LOSS_FLAG_TWO_HOUR_CONTROL: u32 = 1 << 2;
pub const CURRENT_LOSS_FLAG_MANPOWER_DELTA: u32 = 1 << 3;
pub const CURRENT_LOSS_FLAG_EQUIPMENT_DELTA: u32 = 1 << 4;
pub const CURRENT_LOSS_FLAG_EQUIPMENT_IDENTITY: u32 = 1 << 5;
/// The synchronous update came from the active division-damage producer at
/// `hoi4.exe+0x0C7BFC0`.
pub const CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE: u32 = 1 << 6;
/// The synchronous update came from the participant-lifecycle producer at
/// `hoi4.exe+0x1299850`.
pub const CURRENT_LOSS_FLAG_SOURCE_PARTICIPANT_LIFECYCLE: u32 = 1 << 7;
pub const CURRENT_LOSS_SOURCE_FLAG_MASK: u32 =
    CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE | CURRENT_LOSS_FLAG_SOURCE_PARTICIPANT_LIFECYCLE;
pub const CURRENT_LOSS_REQUIRED_FLAG_MASK: u32 = CURRENT_LOSS_FLAG_WRITER_SENTINEL
    | CURRENT_LOSS_FLAG_ACTIVE_EPOCH_BOUND
    | CURRENT_LOSS_FLAG_TWO_HOUR_CONTROL
    | CURRENT_LOSS_FLAG_MANPOWER_DELTA
    | CURRENT_LOSS_FLAG_EQUIPMENT_DELTA
    | CURRENT_LOSS_FLAG_EQUIPMENT_IDENTITY;

/// Pointer-free projection of the engine's exact 0x448-byte terminal record.
/// The hook retains the raw bytes only process-locally, then projects these
/// proven scalar offsets before crossing the replay boundary.
pub const TERMINAL_RECORD_ROW_LEN: usize = 48;
pub const TERMINAL_WRITER_KIND_OVERRUN: u32 = 1;
pub const TERMINAL_WRITER_KIND_NORMAL: u32 = 2;

/// Exact reserve identity, read from CCombatSide's reserve vector (+0xE0/+0xEC).
/// Layout: combat_id u32 | generation u64 | side u32 | flags u32 |
/// ordinal u32 | division_id u32 | reserved u32.
pub const RESERVE_MEMBER_ROW_LEN: usize = 32;
pub const RESERVE_MEMBER_FLAG_ATTACKER: u32 = 1 << 0;
pub const RESERVE_MEMBER_FLAG_DEFENDER: u32 = 1 << 1;
pub const RESERVE_MEMBER_FLAG_SIDE_VALIDATED: u32 = 1 << 2;
pub const RESERVE_MEMBER_FLAG_VECTOR_STABLE: u32 = 1 << 3;
pub const RESERVE_MEMBER_KNOWN_FLAG_MASK: u32 = RESERVE_MEMBER_FLAG_ATTACKER
    | RESERVE_MEMBER_FLAG_DEFENDER
    | RESERVE_MEMBER_FLAG_SIDE_VALIDATED
    | RESERVE_MEMBER_FLAG_VECTOR_STABLE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CombatSide {
    Attacker,
    Defender,
}

impl CombatSide {
    pub fn wire_id(self) -> u32 {
        match self {
            Self::Attacker => SIDE_ATTACKER,
            Self::Defender => SIDE_DEFENDER,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TacticIdentity {
    pub id: i32,
    pub token: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideDetail {
    /// Total units on the side (side+0x2C = committed + reserve). 100/100.
    pub units: u32,
    /// Committed (front-line) count (side+0xD4). 100/100, values 1..6.
    pub committed: u32,
    /// Reserve count (side+0xEC). 100/100.
    pub reserve: u32,
    /// Air-to-ground kills (side+0x140). u32.
    pub air_kills: u32,
    /// Org-loss summary index (side+0x1A8). u32.
    pub org_loss_summary_index: u32,
    /// Number of org losses = combat day count (side+0x1AC). u32.
    pub num_org_losses: u32,
    /// i64 ×1e5 damage accumulators (side+0x138..+0x170).
    pub anti_air_attack_raw: i64,
    pub air_damage_str_raw: i64,
    pub air_damage_org_raw: i64,
    pub ground_damage_str_raw: i64,
    pub ground_damage_org_raw: i64,
    pub prevented_damage_str_raw: i64,
    pub prevented_damage_org_raw: i64,
    /// Exact stored side progress: i32[CCombatSide+0x418] / 100000.
    /// None exists only when decoding legacy v2/v3 streams.
    pub progress_raw: Option<i32>,
    /// Exact runtime CCombatTactic identity. None with tactic_known=true means
    /// the stable side tactic pointer was null; false is legacy/unavailable.
    pub tactic: Option<TacticIdentity>,
    pub tactic_known: bool,
    /// True when the stably-read count triple violated committed+reserve==units
    /// (a genuine mid-combat transient). The row is still emitted, badged.
    pub counts_incomplete: bool,
}

/// A per-side current-state detail row for one combat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideDetailRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub detail: SideDetail,
    /// Whether the matching reserve-member vector passed the stable container
    /// read. False is unavailable, not known-empty.
    pub reserve_members_known: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AttributionEntry {
    pub dealer_tag_db_id: u32,
    pub victim_tag_db_id: u32,
    pub damage_raw: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributionRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub entry: AttributionEntry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveMemberRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    /// Stable engine-vector order. Consumers must not infer battle position.
    pub ordinal: u32,
    pub division_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalCombatRecordRow {
    pub combat_id: u32,
    pub generation: u64,
    pub record_index: u32,
    pub writer_kind: u32,
    /// Exact eight-byte scalar passed as record+0x10 / record+0x220 to the
    /// combat-log manpower localizer. No display conversion is applied.
    pub attacker_manpower_loss_raw: i64,
    pub defender_manpower_loss_raw: i64,
    pub end_date_raw: u32,
    pub location: u32,
    pub winner_side_raw: u8,
    pub winter_raw: u8,
    pub result_selector_raw: u8,
    pub layout_side_raw: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthObservationRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub capture_sequence: u64,
    pub ordinal: u32,
    pub wrapper_count: u32,
    pub division_id: u32,
    pub control_flags: u32,
    pub wrapper_width_raw: i64,
    pub final_width_raw: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentLossObservationRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub capture_sequence: u64,
    pub ordinal: u32,
    pub equipment_count: u32,
    pub equipment_id: u32,
    pub control_flags: u32,
    pub manpower_before_raw: i64,
    pub manpower_after_raw: i64,
    pub equipment_before_raw: i64,
    pub equipment_after_raw: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelObservationRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub capture_sequence: u64,
    /// Exact `CArmyLeader+0x0C`; zero is an authenticated no-leader result.
    pub leader_id: u32,
    /// Tooltip enum presence bits 0..=9. Order is the pinned engine order.
    pub modifier_bits: u32,
    pub control_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCombatSnapshot {
    pub combat_id: u32,
    pub generation: u64,
    pub location: Option<u32>,
    pub day: Option<u32>,
    pub duration: Option<u32>,
    pub attacker_divisions: Vec<u32>,
    pub defender_divisions: Vec<u32>,
    /// E2: per-side CCombatSide detail scalars. `None` when the side object
    /// failed revalidation (fail closed — absent, never zero-filled).
    pub attacker_detail: Option<SideDetail>,
    pub defender_detail: Option<SideDetail>,
    pub attacker_attribution: Vec<AttributionEntry>,
    pub defender_attribution: Vec<AttributionEntry>,
    /// Exact reserve identities from the side reserve vector. None means the
    /// vector did not pass the stable container/partition validation.
    pub attacker_reserve_divisions: Option<Vec<u32>>,
    pub defender_reserve_divisions: Option<Vec<u32>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailEventRow {
    pub combat_id: u32,
    pub generation: u64,
    pub event_kind: u32,
    pub side: Option<CombatSide>,
    pub division_id: Option<u32>,
    pub location: Option<u32>,
    pub day: Option<u32>,
    pub duration: Option<u32>,
    pub old_side_count: u32,
    pub new_side_count: u32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateSummary {
    pub record_count: u64,
    pub row_count: u64,
    pub phase_started_rows: u64,
    pub phase_updated_rows: u64,
    pub phase_ended_rows: u64,
    pub participant_joined_rows: u64,
    pub participant_left_unclassified_rows: u64,
    pub unclassified_rows: u64,
    pub side_detail_rows: u64,
    pub attribution_rows: u64,
    pub reserve_member_rows: u64,
    pub terminal_record_rows: u64,
    pub width_observation_rows: u64,
    pub current_loss_observation_rows: u64,
    pub panel_observation_rows: u64,
    pub domains_seen: BTreeSet<u16>,
    pub status_flags_or: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateResult {
    pub ok: bool,
    pub errors: Vec<String>,
    pub summary: GateSummary,
}

#[derive(Debug, Default)]
struct GateBoundaryState {
    open_hour: Option<i32>,
    pending_perf_hour: Option<i32>,
    first_closed_hour: Option<i32>,
    last_closed_hour: Option<i32>,
    closed_hour_count: u64,
    last_record_type: Option<u16>,
    d55_data_count: u32,
    d55_perf_count: u32,
    d55_data_authority: Option<D55DataAuthority>,
    d55_perf_authority: Option<D55PerfAuthority>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct D55DataAuthority {
    elapsed_wall_ms: u64,
    status_flags: u32,
    row_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct D55PerfAuthority {
    elapsed_wall_ms: u64,
    row_count: u32,
    changed_rows: u32,
    unit_count: u32,
    changed_unit_count: u32,
    status_flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum D55RecordAuthority {
    Data(D55DataAuthority),
    Perf(D55PerfAuthority),
}

impl GateBoundaryState {
    fn observe(
        &mut self,
        offset: usize,
        header: &compact_contract::CompactRecordHeader,
        record_authority: Option<D55RecordAuthority>,
        errors: &mut Vec<String>,
    ) {
        self.last_record_type = Some(header.record_type);
        match header.record_type {
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if let Some(open) = self.open_hour {
                    if open != header.game_hour_raw {
                        errors.push(format!(
                            "offset {offset}: DOMAIN_PERF hour {} interleaves open hour {open}",
                            header.game_hour_raw
                        ));
                        return;
                    }
                } else {
                    if let Some(last_closed) = self.last_closed_hour {
                        let Some(expected) = last_closed.checked_add(1) else {
                            errors.push(format!(
                                "offset {offset}: closed-hour overflow after {last_closed}"
                            ));
                            return;
                        };
                        if header.game_hour_raw != expected {
                            errors.push(format!(
                                "offset {offset}: non-consecutive PERF hour {}; expected {expected} after closed hour {last_closed}",
                                header.game_hour_raw
                            ));
                            return;
                        }
                    }
                    match self.pending_perf_hour {
                        Some(pending) if pending != header.game_hour_raw => {
                            errors.push(format!(
                                "offset {offset}: PERF hour {} conflicts with pending hour {pending}",
                                header.game_hour_raw
                            ));
                            return;
                        }
                        Some(_) => {}
                        None => self.pending_perf_hour = Some(header.game_hour_raw),
                    }
                }
                if header.domain_id == DOMAIN_LAND_COMBAT_DETAIL_EVENTS {
                    self.d55_perf_count = self.d55_perf_count.saturating_add(1);
                    if self.d55_perf_authority.is_none() {
                        self.d55_perf_authority = match record_authority {
                            Some(D55RecordAuthority::Perf(authority)) => Some(authority),
                            _ => None,
                        };
                    }
                    if self.d55_perf_count > 1 {
                        errors.push(format!(
                            "offset {offset}: duplicate domain-55 PERF for hour {}",
                            header.game_hour_raw
                        ));
                    }
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                if let Some(open) = self.open_hour {
                    if open != header.game_hour_raw {
                        errors.push(format!(
                            "offset {offset}: DOMAIN_DATA hour {} interleaves open hour {open}",
                            header.game_hour_raw
                        ));
                        return;
                    }
                } else {
                    if let Some(pending) = self.pending_perf_hour {
                        if pending != header.game_hour_raw {
                            errors.push(format!(
                                "offset {offset}: DATA hour {} conflicts with pending PERF hour {pending}",
                                header.game_hour_raw
                            ));
                            return;
                        }
                    }
                    if let Some(last_closed) = self.last_closed_hour {
                        let Some(expected) = last_closed.checked_add(1) else {
                            errors.push(format!(
                                "offset {offset}: closed-hour overflow after {last_closed}"
                            ));
                            return;
                        };
                        if header.game_hour_raw != expected {
                            errors.push(format!(
                                "offset {offset}: non-consecutive DATA hour {}; expected {expected} after closed hour {last_closed}",
                                header.game_hour_raw
                            ));
                            return;
                        }
                    }
                    self.open_hour = Some(header.game_hour_raw);
                }
                if header.domain_id == DOMAIN_LAND_COMBAT_DETAIL_EVENTS {
                    self.d55_data_count = self.d55_data_count.saturating_add(1);
                    if self.d55_data_authority.is_none() {
                        self.d55_data_authority = match record_authority {
                            Some(D55RecordAuthority::Data(authority)) => Some(authority),
                            _ => None,
                        };
                    }
                    if self.d55_data_count > 1 {
                        errors.push(format!(
                            "offset {offset}: duplicate domain-55 DATA for hour {}",
                            header.game_hour_raw
                        ));
                    }
                }
            }
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END => {
                match self.open_hour {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        errors.push(format!(
                            "offset {offset}: FRAME_END hour {} does not match open hour {open}",
                            header.game_hour_raw
                        ));
                        return;
                    }
                    None => {
                        errors.push(format!(
                            "offset {offset}: bare FRAME_END for hour {} without DOMAIN_DATA",
                            header.game_hour_raw
                        ));
                        return;
                    }
                }
                if self.d55_data_count != 1 || self.d55_perf_count != 1 {
                    errors.push(format!(
                        "offset {offset}: hour {} has domain-55 DATA/PERF counts {}/{}; expected exactly 1/1",
                        header.game_hour_raw, self.d55_data_count, self.d55_perf_count
                    ));
                }
                if let (Some(data), Some(perf)) = (self.d55_data_authority, self.d55_perf_authority)
                {
                    if data.status_flags != perf.status_flags {
                        errors.push(format!(
                            "offset {offset}: hour {} domain-55 DATA status {:#010x} does not match PERF status {:#010x}",
                            header.game_hour_raw,
                            data.status_flags,
                            perf.status_flags,
                        ));
                    }
                    if data.elapsed_wall_ms != perf.elapsed_wall_ms {
                        errors.push(format!(
                            "offset {offset}: hour {} domain-55 DATA elapsed_wall_ms {} does not match PERF elapsed_wall_ms {}",
                            header.game_hour_raw,
                            data.elapsed_wall_ms,
                            perf.elapsed_wall_ms,
                        ));
                    }
                    for (label, perf_count) in [
                        ("row_count", perf.row_count),
                        ("changed_rows", perf.changed_rows),
                        ("unit_count", perf.unit_count),
                        ("changed_unit_count", perf.changed_unit_count),
                    ] {
                        if perf_count != data.row_count {
                            errors.push(format!(
                                "offset {offset}: hour {} domain-55 PERF {label} {perf_count} does not match decoded DATA row count {}",
                                header.game_hour_raw,
                                data.row_count,
                            ));
                        }
                    }
                } else if self.d55_data_count == 1 && self.d55_perf_count == 1 {
                    errors.push(format!(
                        "offset {offset}: hour {} lacks decoded domain-55 DATA/PERF authority for exact status/count binding",
                            header.game_hour_raw
                    ));
                }
                self.open_hour = None;
                self.pending_perf_hour = None;
                self.first_closed_hour.get_or_insert(header.game_hour_raw);
                self.last_closed_hour = Some(header.game_hour_raw);
                self.closed_hour_count = self.closed_hour_count.saturating_add(1);
                self.d55_data_count = 0;
                self.d55_perf_count = 0;
                self.d55_data_authority = None;
                self.d55_perf_authority = None;
            }
            _ => {}
        }
    }

    fn finish(&self, errors: &mut Vec<String>) {
        if let Some(open) = self.open_hour {
            errors.push(format!("domain stream leaves hour {open} open at EOF"));
        }
        if let Some(pending) = self.pending_perf_hour {
            errors.push(format!(
                "domain stream leaves PERF-only hour {pending} pending at EOF"
            ));
        }
        if self.last_closed_hour.is_none() {
            errors.push("domain stream certifies no closed hour".to_string());
        }
        if self.last_record_type != Some(compact_contract::COMPACT_RECORD_TYPE_FRAME_END) {
            errors.push("final record is not canonical FRAME_END".to_string());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureFrame {
    pub compact_stream: Vec<u8>,
    pub rows: Vec<DetailEventRow>,
    /// E2: per-side current-state detail rows (one per active combat per side).
    pub side_detail_rows: Vec<SideDetailRow>,
    /// E5: per-side current-state damage attribution rows.
    pub attribution_rows: Vec<AttributionRow>,
    /// X2b: exact reserve division identities in stable engine-vector order.
    pub reserve_member_rows: Vec<ReserveMemberRow>,
    /// X2 terminal records produced before active-combat deletion.
    pub terminal_record_rows: Vec<TerminalCombatRecordRow>,
    /// Dormant X2 carrier transport. Empty until the exact width controls pass.
    pub width_observation_rows: Vec<WidthObservationRow>,
    /// Dormant X2 carrier transport. Empty until the exact loss controls pass.
    pub current_loss_observation_rows: Vec<CurrentLossObservationRow>,
    pub panel_observation_rows: Vec<PanelObservationRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LandCombatDetailTracker {
    previous: Vec<LandCombatSnapshot>,
    pending_rebaseline: bool,
    last_frame_rebaselined: bool,
}

impl LandCombatDetailTracker {
    pub fn reset_transition_baseline(&mut self) {
        self.previous.clear();
        self.pending_rebaseline = true;
        self.last_frame_rebaselined = false;
    }

    pub fn last_frame_rebaselined(&self) -> bool {
        self.last_frame_rebaselined
    }
}

pub trait LandCombatDetailMemory {
    fn land_combat_snapshots(&self) -> Result<Vec<LandCombatSnapshot>, Vec<String>>;
}

pub fn collect_transition_capture_frame<M: LandCombatDetailMemory>(
    memory: &M,
    tracker: &mut LandCombatDetailTracker,
    sequence: u64,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> Result<CaptureFrame, Vec<String>> {
    let current = memory.land_combat_snapshots()?;
    if tracker.pending_rebaseline {
        // The transition BASELINE reset zeroes the event-delta history so no
        // synthetic join/leave events are fabricated across the gap. Per-side
        // detail is CURRENT-STATE, not a delta, so it is still emitted on the
        // rebaseline frame — the fold needs the live scalars every hour.
        let side_detail_rows = side_detail_rows_from_snapshots(&current);
        let attribution_rows = attribution_rows_from_snapshots(&current);
        let reserve_member_rows = reserve_member_rows_from_snapshots(&current);
        let payload = encode_detail_events_payload_v8(
            elapsed_wall_ms,
            STATUS_DELTA_DATA | STATUS_PARTIAL_DATA | STATUS_TRANSITION_BASELINE_RESET,
            &[],
            &side_detail_rows,
            &attribution_rows,
            &reserve_member_rows,
            &[],
            &[],
            &[],
            &[],
        );
        let mut compact_stream = Vec::new();
        append_compact_record(
            &mut compact_stream,
            sequence,
            game_hour_raw,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        tracker.previous = current;
        tracker.pending_rebaseline = false;
        tracker.last_frame_rebaselined = true;
        return Ok(CaptureFrame {
            compact_stream,
            rows: Vec::new(),
            side_detail_rows,
            attribution_rows,
            reserve_member_rows,
            terminal_record_rows: Vec::new(),
            width_observation_rows: Vec::new(),
            current_loss_observation_rows: Vec::new(),
            panel_observation_rows: Vec::new(),
        });
    }
    let frame = build_transition_capture_frame(
        tracker.previous.as_slice(),
        current.as_slice(),
        sequence,
        game_hour_raw,
        elapsed_wall_ms,
    )?;
    tracker.previous = current;
    tracker.last_frame_rebaselined = false;
    Ok(frame)
}

/// Build the per-side current-state detail rows for every active combat that
/// carries validated CCombatSide detail. A side whose detail failed
/// revalidation contributes NO row (fail closed; absent, not zero).
pub fn side_detail_rows_from_snapshots(current: &[LandCombatSnapshot]) -> Vec<SideDetailRow> {
    let mut rows = Vec::new();
    for snapshot in current {
        if let Some(detail) = snapshot.attacker_detail.clone() {
            rows.push(SideDetailRow {
                combat_id: snapshot.combat_id,
                generation: snapshot.generation,
                side: CombatSide::Attacker,
                detail,
                reserve_members_known: snapshot.attacker_reserve_divisions.is_some(),
            });
        }
        if let Some(detail) = snapshot.defender_detail.clone() {
            rows.push(SideDetailRow {
                combat_id: snapshot.combat_id,
                generation: snapshot.generation,
                side: CombatSide::Defender,
                detail,
                reserve_members_known: snapshot.defender_reserve_divisions.is_some(),
            });
        }
    }
    rows
}

pub fn attribution_rows_from_snapshots(current: &[LandCombatSnapshot]) -> Vec<AttributionRow> {
    let mut rows = Vec::new();
    for snapshot in current {
        for entry in &snapshot.attacker_attribution {
            rows.push(AttributionRow {
                combat_id: snapshot.combat_id,
                generation: snapshot.generation,
                side: CombatSide::Attacker,
                entry: *entry,
            });
        }
        for entry in &snapshot.defender_attribution {
            rows.push(AttributionRow {
                combat_id: snapshot.combat_id,
                generation: snapshot.generation,
                side: CombatSide::Defender,
                entry: *entry,
            });
        }
    }
    rows
}

pub fn reserve_member_rows_from_snapshots(current: &[LandCombatSnapshot]) -> Vec<ReserveMemberRow> {
    let mut rows = Vec::new();
    for snapshot in current {
        for (side, members) in [
            (
                CombatSide::Attacker,
                snapshot.attacker_reserve_divisions.as_ref(),
            ),
            (
                CombatSide::Defender,
                snapshot.defender_reserve_divisions.as_ref(),
            ),
        ] {
            if let Some(members) = members {
                for (ordinal, division_id) in members.iter().copied().enumerate() {
                    rows.push(ReserveMemberRow {
                        combat_id: snapshot.combat_id,
                        generation: snapshot.generation,
                        side,
                        ordinal: ordinal as u32,
                        division_id,
                    });
                }
            }
        }
    }
    rows
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDetailEventsPayload {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub rows: Vec<DetailEventRow>,
    /// E2: per-side current-state detail rows (empty for v1 streams).
    pub side_detail_rows: Vec<SideDetailRow>,
    /// E5: per-side current-state damage attribution rows (empty before v3).
    pub attribution_rows: Vec<AttributionRow>,
    /// X2b: exact reserve identities (empty before v5).
    pub reserve_member_rows: Vec<ReserveMemberRow>,
    /// X2 authenticated terminal record scalars (empty before v6).
    pub terminal_record_rows: Vec<TerminalCombatRecordRow>,
    /// X2 exact-call width rows (empty before v7).
    pub width_observation_rows: Vec<WidthObservationRow>,
    /// X2 exact current-epoch loss rows (empty before v7).
    pub current_loss_observation_rows: Vec<CurrentLossObservationRow>,
    /// Authenticated panel-selected leader and displayed modifiers (v8).
    pub panel_observation_rows: Vec<PanelObservationRow>,
}

pub fn build_transition_capture_frame(
    previous: &[LandCombatSnapshot],
    current: &[LandCombatSnapshot],
    sequence: u64,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> Result<CaptureFrame, Vec<String>> {
    let rows = build_detail_event_rows(previous, current)?;
    let side_detail_rows = side_detail_rows_from_snapshots(current);
    let attribution_rows = attribution_rows_from_snapshots(current);
    let reserve_member_rows = reserve_member_rows_from_snapshots(current);
    let payload = encode_detail_events_payload_v8(
        elapsed_wall_ms,
        STATUS_DELTA_DATA,
        &rows,
        &side_detail_rows,
        &attribution_rows,
        &reserve_member_rows,
        &[],
        &[],
        &[],
        &[],
    );
    let mut compact_stream = Vec::new();
    append_compact_record(
        &mut compact_stream,
        sequence,
        game_hour_raw,
        DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &payload,
    );
    Ok(CaptureFrame {
        compact_stream,
        rows,
        side_detail_rows,
        attribution_rows,
        reserve_member_rows,
        terminal_record_rows: Vec::new(),
        width_observation_rows: Vec::new(),
        current_loss_observation_rows: Vec::new(),
        panel_observation_rows: Vec::new(),
    })
}

pub fn build_detail_event_rows(
    previous: &[LandCombatSnapshot],
    current: &[LandCombatSnapshot],
) -> Result<Vec<DetailEventRow>, Vec<String>> {
    let mut errors = Vec::new();
    for row in previous.iter().chain(current.iter()) {
        if let Err(err) = validate_snapshot(row) {
            errors.push(err);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let previous_by_key = previous
        .iter()
        .map(|row| ((row.combat_id, row.generation), row))
        .collect::<BTreeMap<_, _>>();
    let current_by_key = current
        .iter()
        .map(|row| ((row.combat_id, row.generation), row))
        .collect::<BTreeMap<_, _>>();

    let mut rows = Vec::new();
    for (key, now) in &current_by_key {
        match previous_by_key.get(key) {
            None => rows.push(phase_row(EVENT_PHASE_STARTED, now)),
            Some(old) if phase_changed(old, now) => rows.push(phase_row(EVENT_PHASE_UPDATED, now)),
            Some(_) => {}
        }
        if let Some(old) = previous_by_key.get(key).copied() {
            push_side_delta_rows(
                &mut rows,
                old.attacker_divisions.as_slice(),
                &now.attacker_divisions,
                now,
                CombatSide::Attacker,
            );
            push_side_delta_rows(
                &mut rows,
                old.defender_divisions.as_slice(),
                &now.defender_divisions,
                now,
                CombatSide::Defender,
            );
        }
    }
    for (key, old) in &previous_by_key {
        if !current_by_key.contains_key(key) {
            rows.push(phase_row(EVENT_PHASE_ENDED, old));
        }
    }
    Ok(rows)
}

/// v1 encoder (event rows only, no side detail). Retained for the legacy wire
/// shape and older tests; writes SCHEMA_VERSION_V1 and the 28-byte header.
pub fn encode_detail_events_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PAYLOAD_HEADER_LEN_V1 + rows.len() * DETAIL_EVENT_ROW_LEN);
    payload.extend_from_slice(&SCHEMA_VERSION_V1_NO_SIDE_DETAIL.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    append_event_rows(&mut payload, rows);
    payload
}

/// v2 encoder (E2): event rows PLUS a per-side detail row family. The 32-byte
/// header appends a u32 side_detail_row_count after the event-row count block;
/// the side-detail rows follow the event rows.
pub fn encode_detail_events_payload_v2(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        PAYLOAD_HEADER_LEN_V2
            + rows.len() * DETAIL_EVENT_ROW_LEN
            + side_detail_rows.len() * SIDE_DETAIL_ROW_LEN_V3,
    );
    payload.extend_from_slice(&SCHEMA_VERSION_V2_SIDE_DETAIL.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // string_count
    payload
        .extend_from_slice(&(side_detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
    append_event_rows(&mut payload, rows);
    for row in side_detail_rows {
        append_side_detail_row_legacy(&mut payload, row);
    }
    payload
}

/// v3 compatibility encoder: attribution rows are present, while side detail
/// retains the legacy 100-byte shape and has no tactic string table.
pub fn encode_detail_events_payload_v3(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
    attribution_rows: &[AttributionRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        PAYLOAD_HEADER_LEN_V4
            + rows.len() * DETAIL_EVENT_ROW_LEN
            + side_detail_rows.len() * SIDE_DETAIL_ROW_LEN_V3
            + attribution_rows.len() * ATTRIBUTION_ROW_LEN,
    );
    payload.extend_from_slice(&SCHEMA_VERSION_V3_ATTRIBUTION.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // string_count
    payload
        .extend_from_slice(&(side_detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(attribution_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
    append_event_rows(&mut payload, rows);
    for row in side_detail_rows {
        append_side_detail_row_legacy(&mut payload, row);
    }
    for row in attribution_rows {
        append_attribution_row(&mut payload, row);
    }
    payload
}

/// v4 production encoder: exact per-side progress and tactic identity, with a
/// canonical UTF-8 tactic string table, plus attribution rows.
pub fn encode_detail_events_payload_v4(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
    attribution_rows: &[AttributionRow],
) -> Vec<u8> {
    let strings = canonical_tactic_strings(side_detail_rows);
    let string_indices = strings
        .iter()
        .enumerate()
        .map(|(index, value)| (value.as_str(), index as u32))
        .collect::<BTreeMap<_, _>>();
    let string_bytes = strings.iter().map(|s| 4 + s.len()).sum::<usize>();
    let mut payload = Vec::with_capacity(
        PAYLOAD_HEADER_LEN_V4
            + rows.len() * DETAIL_EVENT_ROW_LEN
            + side_detail_rows.len() * SIDE_DETAIL_ROW_LEN
            + attribution_rows.len() * ATTRIBUTION_ROW_LEN
            + string_bytes,
    );
    payload.extend_from_slice(&SCHEMA_VERSION_V4_TACTIC_PROGRESS.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&(strings.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(side_detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(attribution_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
    append_event_rows(&mut payload, rows);
    for row in side_detail_rows {
        append_side_detail_row(&mut payload, row, &string_indices, false);
    }
    for row in attribution_rows {
        append_attribution_row(&mut payload, row);
    }
    for value in strings {
        payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }
    payload
}

/// v5 production encoder: v4 tactic/progress + exact reserve identities.
pub fn encode_detail_events_payload_v5(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
    attribution_rows: &[AttributionRow],
    reserve_member_rows: &[ReserveMemberRow],
) -> Vec<u8> {
    let strings = canonical_tactic_strings(side_detail_rows);
    let string_indices = strings
        .iter()
        .enumerate()
        .map(|(index, value)| (value.as_str(), index as u32))
        .collect::<BTreeMap<_, _>>();
    let string_bytes = strings.iter().map(|s| 4 + s.len()).sum::<usize>();
    let mut payload = Vec::with_capacity(
        PAYLOAD_HEADER_LEN_V5
            + rows.len() * DETAIL_EVENT_ROW_LEN
            + side_detail_rows.len() * SIDE_DETAIL_ROW_LEN
            + attribution_rows.len() * ATTRIBUTION_ROW_LEN
            + reserve_member_rows.len() * RESERVE_MEMBER_ROW_LEN
            + string_bytes,
    );
    payload.extend_from_slice(&SCHEMA_VERSION_V5_RESERVE_MEMBERS.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&(strings.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(side_detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(attribution_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(
        &(reserve_member_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(&0u32.to_le_bytes());
    append_event_rows(&mut payload, rows);
    for row in side_detail_rows {
        append_side_detail_row(&mut payload, row, &string_indices, true);
    }
    for row in attribution_rows {
        append_attribution_row(&mut payload, row);
    }
    for row in reserve_member_rows {
        append_reserve_member_row(&mut payload, row);
    }
    for value in strings {
        payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }
    payload
}

/// v6 production encoder: v5 plus the authenticated, pointer-free projection
/// of terminal combat-log records. Terminal rows precede the tactic strings.
pub fn encode_detail_events_payload_v6(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
    attribution_rows: &[AttributionRow],
    reserve_member_rows: &[ReserveMemberRow],
    terminal_record_rows: &[TerminalCombatRecordRow],
) -> Vec<u8> {
    let strings = canonical_tactic_strings(side_detail_rows);
    let string_indices = strings
        .iter()
        .enumerate()
        .map(|(index, value)| (value.as_str(), index as u32))
        .collect::<BTreeMap<_, _>>();
    let string_bytes = strings.iter().map(|s| 4 + s.len()).sum::<usize>();
    let mut payload = Vec::with_capacity(
        PAYLOAD_HEADER_LEN_V6
            + rows.len() * DETAIL_EVENT_ROW_LEN
            + side_detail_rows.len() * SIDE_DETAIL_ROW_LEN
            + attribution_rows.len() * ATTRIBUTION_ROW_LEN
            + reserve_member_rows.len() * RESERVE_MEMBER_ROW_LEN
            + terminal_record_rows.len() * TERMINAL_RECORD_ROW_LEN
            + string_bytes,
    );
    payload.extend_from_slice(&SCHEMA_VERSION_V6_TERMINAL_RECORDS.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&(strings.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(side_detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(attribution_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(
        &(reserve_member_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(terminal_record_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(&0u32.to_le_bytes());
    append_event_rows(&mut payload, rows);
    for row in side_detail_rows {
        append_side_detail_row(&mut payload, row, &string_indices, true);
    }
    for row in attribution_rows {
        append_attribution_row(&mut payload, row);
    }
    for row in reserve_member_rows {
        append_reserve_member_row(&mut payload, row);
    }
    for row in terminal_record_rows {
        append_terminal_record_row(&mut payload, row);
    }
    for value in strings {
        payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }
    payload
}

/// Current production encoder: v6 plus the pointer-free X2 width/loss and
/// authenticated panel-observation transports. Counts are explicit header
/// fields; no row shape is inferred from payload remainder bytes.
pub fn encode_detail_events_payload_v8(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
    attribution_rows: &[AttributionRow],
    reserve_member_rows: &[ReserveMemberRow],
    terminal_record_rows: &[TerminalCombatRecordRow],
    width_observation_rows: &[WidthObservationRow],
    current_loss_observation_rows: &[CurrentLossObservationRow],
    panel_observation_rows: &[PanelObservationRow],
) -> Vec<u8> {
    let strings = canonical_tactic_strings(side_detail_rows);
    let string_indices = strings
        .iter()
        .enumerate()
        .map(|(index, value)| (value.as_str(), index as u32))
        .collect::<BTreeMap<_, _>>();
    let string_bytes = strings.iter().map(|s| 4 + s.len()).sum::<usize>();
    let mut payload = Vec::with_capacity(
        PAYLOAD_HEADER_LEN
            + rows.len() * DETAIL_EVENT_ROW_LEN
            + side_detail_rows.len() * SIDE_DETAIL_ROW_LEN
            + attribution_rows.len() * ATTRIBUTION_ROW_LEN
            + reserve_member_rows.len() * RESERVE_MEMBER_ROW_LEN
            + terminal_record_rows.len() * TERMINAL_RECORD_ROW_LEN
            + width_observation_rows.len() * WIDTH_OBSERVATION_ROW_LEN
            + current_loss_observation_rows.len() * CURRENT_LOSS_OBSERVATION_ROW_LEN
            + panel_observation_rows.len() * PANEL_OBSERVATION_ROW_LEN
            + string_bytes,
    );
    payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&(strings.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(side_detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(attribution_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(
        &(reserve_member_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(terminal_record_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(width_observation_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(current_loss_observation_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(panel_observation_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    append_event_rows(&mut payload, rows);
    for row in side_detail_rows {
        append_side_detail_row(&mut payload, row, &string_indices, true);
    }
    for row in attribution_rows {
        append_attribution_row(&mut payload, row);
    }
    for row in reserve_member_rows {
        append_reserve_member_row(&mut payload, row);
    }
    for row in terminal_record_rows {
        append_terminal_record_row(&mut payload, row);
    }
    for row in width_observation_rows {
        append_width_observation_row(&mut payload, row);
    }
    for row in current_loss_observation_rows {
        append_current_loss_observation_row(&mut payload, row);
    }
    for row in panel_observation_rows {
        append_panel_observation_row(&mut payload, row);
    }
    for value in strings {
        payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }
    payload
}

fn canonical_tactic_strings(side_detail_rows: &[SideDetailRow]) -> Vec<String> {
    let mut strings = BTreeSet::new();
    for row in side_detail_rows {
        if let Some(tactic) = &row.detail.tactic {
            assert!(!tactic.token.is_empty() && tactic.token.len() <= MAX_TACTIC_STRING_LEN);
            assert!(!tactic.name.is_empty() && tactic.name.len() <= MAX_TACTIC_STRING_LEN);
            strings.insert(tactic.token.clone());
            strings.insert(tactic.name.clone());
        }
    }
    assert!(strings.len() <= MAX_TACTIC_STRING_COUNT);
    strings.into_iter().collect()
}

fn append_event_rows(payload: &mut Vec<u8>, rows: &[DetailEventRow]) {
    for row in rows {
        validate_row(row).expect("invalid detail event row");
        payload.extend_from_slice(&row.combat_id.to_le_bytes());
        payload.extend_from_slice(&row.generation.to_le_bytes());
        payload.extend_from_slice(&row.event_kind.to_le_bytes());
        payload.extend_from_slice(
            &row.side
                .map(CombatSide::wire_id)
                .unwrap_or(SIDE_NONE)
                .to_le_bytes(),
        );
        payload.extend_from_slice(&row.division_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.location.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.day.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.duration.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.old_side_count.to_le_bytes());
        payload.extend_from_slice(&row.new_side_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
}

fn append_side_detail_row_legacy(payload: &mut Vec<u8>, row: &SideDetailRow) {
    let mut flags = SIDE_DETAIL_FLAG_SIDE_VALIDATED
        | SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED
        | SIDE_DETAIL_FLAG_LOSSES_OMITTED;
    flags |= match row.side {
        CombatSide::Attacker => SIDE_DETAIL_FLAG_ATTACKER,
        CombatSide::Defender => SIDE_DETAIL_FLAG_DEFENDER,
    };
    if row.detail.counts_incomplete {
        flags |= SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE;
    }
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&row.detail.units.to_le_bytes());
    payload.extend_from_slice(&row.detail.committed.to_le_bytes());
    payload.extend_from_slice(&row.detail.reserve.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_kills.to_le_bytes());
    payload.extend_from_slice(&row.detail.org_loss_summary_index.to_le_bytes());
    payload.extend_from_slice(&row.detail.num_org_losses.to_le_bytes());
    payload.extend_from_slice(&row.detail.anti_air_attack_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_damage_org_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.ground_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.ground_damage_org_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.prevented_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.prevented_damage_org_raw.to_le_bytes());
}

/// Encode one v4 side-detail row (SIDE_DETAIL_ROW_LEN bytes).
fn append_side_detail_row(
    payload: &mut Vec<u8>,
    row: &SideDetailRow,
    string_indices: &BTreeMap<&str, u32>,
    emit_reserve_authority: bool,
) {
    assert!(
        row.detail.progress_raw.is_some(),
        "schema v4 requires exact side progress"
    );
    assert!(
        row.detail.tactic_known,
        "schema v4 requires authoritative tactic presence/absence"
    );
    let mut flags = SIDE_DETAIL_FLAG_SIDE_VALIDATED | SIDE_DETAIL_FLAG_LOSSES_OMITTED;
    flags |= match row.side {
        CombatSide::Attacker => SIDE_DETAIL_FLAG_ATTACKER,
        CombatSide::Defender => SIDE_DETAIL_FLAG_DEFENDER,
    };
    if row.detail.counts_incomplete {
        flags |= SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE;
    }
    if row.detail.progress_raw.is_some() {
        flags |= SIDE_DETAIL_FLAG_PROGRESS_PRESENT;
    }
    if row.detail.tactic_known {
        flags |= SIDE_DETAIL_FLAG_TACTIC_KNOWN;
    } else {
        flags |= SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED;
    }
    if row.detail.tactic.is_some() {
        flags |= SIDE_DETAIL_FLAG_TACTIC_PRESENT;
    }
    if emit_reserve_authority && row.reserve_members_known {
        flags |= SIDE_DETAIL_FLAG_RESERVE_MEMBERS_KNOWN;
    }
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&row.detail.units.to_le_bytes());
    payload.extend_from_slice(&row.detail.committed.to_le_bytes());
    payload.extend_from_slice(&row.detail.reserve.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_kills.to_le_bytes());
    payload.extend_from_slice(&row.detail.org_loss_summary_index.to_le_bytes());
    payload.extend_from_slice(&row.detail.num_org_losses.to_le_bytes());
    payload.extend_from_slice(&row.detail.anti_air_attack_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_damage_org_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.ground_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.ground_damage_org_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.prevented_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.prevented_damage_org_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.progress_raw.unwrap_or(0).to_le_bytes());
    if let Some(tactic) = &row.detail.tactic {
        payload.extend_from_slice(&tactic.id.to_le_bytes());
        payload.extend_from_slice(&string_indices[tactic.token.as_str()].to_le_bytes());
        payload.extend_from_slice(&string_indices[tactic.name.as_str()].to_le_bytes());
    } else {
        payload.extend_from_slice(&0i32.to_le_bytes());
        payload.extend_from_slice(&TACTIC_STRING_INDEX_NONE.to_le_bytes());
        payload.extend_from_slice(&TACTIC_STRING_INDEX_NONE.to_le_bytes());
    }
}

fn append_attribution_row(payload: &mut Vec<u8>, row: &AttributionRow) {
    validate_attribution_entry(row.entry).expect("invalid attribution row");
    let mut flags = ATTRIBUTION_FLAG_SIDE_VALIDATED
        | ATTRIBUTION_FLAG_DAMAGE_ROW_VALIDATED
        | ATTRIBUTION_FLAG_DAMAGE_FIXED_POINT;
    flags |= match row.side {
        CombatSide::Attacker => ATTRIBUTION_FLAG_ATTACKER,
        CombatSide::Defender => ATTRIBUTION_FLAG_DEFENDER,
    };
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&row.entry.dealer_tag_db_id.to_le_bytes());
    payload.extend_from_slice(&row.entry.victim_tag_db_id.to_le_bytes());
    payload.extend_from_slice(&row.entry.damage_raw.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
}

fn append_reserve_member_row(payload: &mut Vec<u8>, row: &ReserveMemberRow) {
    assert!(row.combat_id > 0 && row.combat_id <= MAX_COMBAT_ID);
    assert!(row.generation > 0);
    assert!(row.division_id > 0 && row.division_id <= MAX_DIVISION_ID);
    assert!(row.ordinal < MAX_SIDE_UNITS as u32);
    let flags = RESERVE_MEMBER_FLAG_SIDE_VALIDATED
        | RESERVE_MEMBER_FLAG_VECTOR_STABLE
        | match row.side {
            CombatSide::Attacker => RESERVE_MEMBER_FLAG_ATTACKER,
            CombatSide::Defender => RESERVE_MEMBER_FLAG_DEFENDER,
        };
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&row.ordinal.to_le_bytes());
    payload.extend_from_slice(&row.division_id.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
}

fn append_terminal_record_row(payload: &mut Vec<u8>, row: &TerminalCombatRecordRow) {
    validate_terminal_record_row(row).expect("invalid terminal combat record row");
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.record_index.to_le_bytes());
    payload.extend_from_slice(&row.writer_kind.to_le_bytes());
    payload.extend_from_slice(&row.attacker_manpower_loss_raw.to_le_bytes());
    payload.extend_from_slice(&row.defender_manpower_loss_raw.to_le_bytes());
    payload.extend_from_slice(&row.end_date_raw.to_le_bytes());
    payload.extend_from_slice(&row.location.to_le_bytes());
    payload.push(row.winner_side_raw);
    payload.push(row.winter_raw);
    payload.push(row.result_selector_raw);
    payload.push(row.layout_side_raw);
}

fn append_width_observation_row(payload: &mut Vec<u8>, row: &WidthObservationRow) {
    validate_width_observation_row(row).expect("invalid width observation row");
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&row.capture_sequence.to_le_bytes());
    payload.extend_from_slice(&row.ordinal.to_le_bytes());
    payload.extend_from_slice(&row.wrapper_count.to_le_bytes());
    payload.extend_from_slice(&row.division_id.to_le_bytes());
    payload.extend_from_slice(&row.control_flags.to_le_bytes());
    payload.extend_from_slice(&row.wrapper_width_raw.to_le_bytes());
    payload.extend_from_slice(&row.final_width_raw.to_le_bytes());
}

fn append_current_loss_observation_row(payload: &mut Vec<u8>, row: &CurrentLossObservationRow) {
    validate_current_loss_observation_row(row).expect("invalid current loss observation row");
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&row.capture_sequence.to_le_bytes());
    payload.extend_from_slice(&row.ordinal.to_le_bytes());
    payload.extend_from_slice(&row.equipment_count.to_le_bytes());
    payload.extend_from_slice(&row.equipment_id.to_le_bytes());
    payload.extend_from_slice(&row.control_flags.to_le_bytes());
    payload.extend_from_slice(&row.manpower_before_raw.to_le_bytes());
    payload.extend_from_slice(&row.manpower_after_raw.to_le_bytes());
    payload.extend_from_slice(&row.equipment_before_raw.to_le_bytes());
    payload.extend_from_slice(&row.equipment_after_raw.to_le_bytes());
}

fn append_panel_observation_row(payload: &mut Vec<u8>, row: &PanelObservationRow) {
    validate_panel_observation_row(row).expect("invalid panel observation row");
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&row.capture_sequence.to_le_bytes());
    payload.extend_from_slice(&row.leader_id.to_le_bytes());
    payload.extend_from_slice(&row.modifier_bits.to_le_bytes());
    payload.extend_from_slice(&row.control_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
}

fn validate_width_observation_row(row: &WidthObservationRow) -> Result<(), String> {
    if row.combat_id == 0
        || row.combat_id > MAX_COMBAT_ID
        || row.generation == 0
        || row.capture_sequence == 0
        || row.wrapper_count == 0
        || row.wrapper_count as usize > MAX_SIDE_UNITS
        || row.ordinal >= row.wrapper_count
        || row.division_id == 0
        || row.division_id > MAX_DIVISION_ID
        || row.control_flags != WIDTH_REQUIRED_FLAG_MASK
    {
        return Err("width observation identity/control field invalid".into());
    }
    Ok(())
}

fn validate_current_loss_observation_row(row: &CurrentLossObservationRow) -> Result<(), String> {
    if row.combat_id == 0
        || row.combat_id > MAX_COMBAT_ID
        || row.generation == 0
        || row.capture_sequence == 0
        || row.equipment_count == 0
        || row.equipment_count as usize > MAX_SIDE_UNITS
        || row.ordinal >= row.equipment_count
        || row.equipment_id == 0
        || row.control_flags & CURRENT_LOSS_REQUIRED_FLAG_MASK != CURRENT_LOSS_REQUIRED_FLAG_MASK
        || row.control_flags & !(CURRENT_LOSS_REQUIRED_FLAG_MASK | CURRENT_LOSS_SOURCE_FLAG_MASK)
            != 0
        || (row.control_flags & CURRENT_LOSS_SOURCE_FLAG_MASK).count_ones() != 1
        || row.manpower_before_raw == row.manpower_after_raw
        || row.equipment_before_raw == row.equipment_after_raw
    {
        return Err("current-loss observation identity/control/delta field invalid".into());
    }
    Ok(())
}

fn validate_panel_observation_row(row: &PanelObservationRow) -> Result<(), String> {
    if row.combat_id == 0
        || row.combat_id > MAX_COMBAT_ID
        || row.generation == 0
        || row.capture_sequence == 0
        || row.leader_id > MAX_DIVISION_ID
        || row.modifier_bits & !PANEL_MODIFIER_KNOWN_MASK != 0
        || row.control_flags != PANEL_REQUIRED_FLAG_MASK
    {
        return Err("panel observation identity/control field invalid".into());
    }
    Ok(())
}

fn validate_terminal_record_row(row: &TerminalCombatRecordRow) -> Result<(), String> {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID || row.generation == 0 {
        return Err("terminal combat identity out of range".into());
    }
    if row.location > MAX_LOCATION_ID {
        return Err("terminal combat location out of range".into());
    }
    if row.winner_side_raw > 1 || row.winter_raw > 1 || row.layout_side_raw > 1 {
        return Err("terminal combat boolean discriminator out of range".into());
    }
    match row.writer_kind {
        TERMINAL_WRITER_KIND_OVERRUN if row.result_selector_raw == 0 => Ok(()),
        TERMINAL_WRITER_KIND_NORMAL if row.result_selector_raw == 1 => Ok(()),
        TERMINAL_WRITER_KIND_OVERRUN | TERMINAL_WRITER_KIND_NORMAL => {
            Err("terminal writer/result discriminator mismatch".into())
        }
        _ => Err("terminal writer kind unknown".into()),
    }
}

/// Validate the compact stream's framing, per-hour D55 authority, payload
/// joins, and terminal boundary. This is the structural gate; callers making a
/// production-duration claim must use [`gate_stream_for_product`].
pub fn gate_stream(data: &[u8]) -> GateResult {
    gate_stream_internal(data, None)
}

/// Validate a product capture for an explicitly requested number of game-hour
/// advancements. Because both endpoints must be closed, `N` advancements
/// require at least `N + 1` consecutive authoritative FRAME_END records.
pub fn gate_stream_for_product(data: &[u8], minimum_game_hour_advancements: i64) -> GateResult {
    gate_stream_internal(data, Some(minimum_game_hour_advancements))
}

fn gate_stream_internal(data: &[u8], minimum_game_hour_advancements: Option<i64>) -> GateResult {
    let mut errors = Vec::new();
    let mut summary = GateSummary::default();
    let mut cursor = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut boundary = GateBoundaryState::default();
    while cursor < data.len() {
        let header = match parse_compact_record_header(&data[cursor..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                errors.push(format!("offset {cursor}: truncated compact record header"));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                errors.push(format!("offset {cursor}: bad compact record magic"));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                errors.push(format!(
                    "offset {cursor}: unsupported compact version {version}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                errors.push(format!(
                    "offset {cursor}: compact header_len {header_len} != {COMPACT_RECORD_HEADER_LEN}"
                ));
                break;
            }
            Err(err @ RecordFramingError::PayloadTooLarge(_)) => {
                errors.push(format!("offset {cursor}: {err}"));
                break;
            }
        };
        match previous_sequence {
            None if header.sequence != 1 => errors.push(format!(
                "offset {cursor}: sequence {} does not start at 1 (stream prefix is missing)",
                header.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if header.sequence != expected => errors.push(format!(
                    "offset {cursor}: sequence discontinuity {previous} -> {} (expected {expected})",
                    header.sequence
                )),
                None => errors.push(format!(
                    "offset {cursor}: sequence space exhausted after {previous}; got {} (wrapping is not allowed)",
                    header.sequence
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(header.sequence);
        let payload_start = cursor + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start.saturating_add(header.payload_len);
        if payload_end > data.len() {
            errors.push(format!("offset {cursor}: payload overruns stream"));
            break;
        }
        let payload = &data[payload_start..payload_end];
        let semantic_result = compact_contract::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        );
        if let Err(error) = &semantic_result {
            errors.push(format!("offset {cursor}: {error}"));
        }
        summary.record_count = summary.record_count.saturating_add(1);
        summary.domains_seen.insert(header.domain_id);
        let data_authority = if semantic_result.is_ok()
            && header.domain_id == DOMAIN_LAND_COMBAT_DETAIL_EVENTS
            && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            decode_detail_events_payload_into_summary(cursor, payload, &mut summary, &mut errors)
        } else {
            None
        };
        if semantic_result.is_ok() {
            let record_authority = match (header.domain_id, header.record_type) {
                (DOMAIN_LAND_COMBAT_DETAIL_EVENTS, COMPACT_RECORD_TYPE_DOMAIN_DATA) => {
                    data_authority.map(D55RecordAuthority::Data)
                }
                (
                    DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
                    compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                ) => Some(D55RecordAuthority::Perf(D55PerfAuthority {
                    elapsed_wall_ms: read_u64(payload, 0),
                    row_count: read_u32(payload, 16),
                    changed_rows: read_u32(payload, 20),
                    unit_count: read_u32(payload, 24),
                    changed_unit_count: read_u32(payload, 28),
                    status_flags: read_u32(payload, 32),
                })),
                _ => None,
            };
            boundary.observe(cursor, &header, record_authority, &mut errors);
            match header.record_type {
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF
                    if header.domain_id == DOMAIN_LAND_COMBAT_DETAIL_EVENTS =>
                {
                    record_gate_status(cursor, read_u32(payload, 32), &mut summary, &mut errors);
                }
                compact_contract::COMPACT_RECORD_TYPE_FRAME_END => {
                    record_gate_status(cursor, read_u32(payload, 24), &mut summary, &mut errors);
                    validate_frame_end_authority(cursor, payload, &mut errors);
                }
                _ => {}
            }
        }
        cursor = payload_end;
    }
    boundary.finish(&mut errors);
    if let Some(minimum) = minimum_game_hour_advancements {
        validate_product_duration(&boundary, minimum, &mut errors);
    }
    let fatal_status =
        compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            summary.status_flags_or,
        );
    if fatal_status != 0 {
        errors.push(format!(
            "land combat detail events status has failure flags {}",
            fatal_status
        ));
    }
    GateResult {
        ok: errors.is_empty(),
        errors,
        summary,
    }
}

fn validate_frame_end_authority(offset: usize, payload: &[u8], errors: &mut Vec<String>) {
    let emitted_domains = read_u32(payload, 16);
    let scheduled_domains = read_u32(payload, 20);
    let reserved = read_u32(payload, 28);
    if emitted_domains == 0 || scheduled_domains == 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END has zero emitted/scheduled domain authority ({emitted_domains}/{scheduled_domains})"
        ));
    }
    if emitted_domains != scheduled_domains {
        errors.push(format!(
            "offset {offset}: FRAME_END emitted domain count {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END reserved payload word is nonzero"
        ));
    }
}

fn validate_product_duration(
    boundary: &GateBoundaryState,
    minimum_game_hour_advancements: i64,
    errors: &mut Vec<String>,
) {
    if minimum_game_hour_advancements <= 0 {
        errors.push(format!(
            "minimum game-hour advancements must be explicitly positive, got {minimum_game_hour_advancements}"
        ));
        return;
    }
    let observed_advancements = match (boundary.first_closed_hour, boundary.last_closed_hour) {
        (Some(first), Some(last)) => i64::from(last) - i64::from(first),
        _ => -1,
    };
    let observed_closed_frames = i64::try_from(boundary.closed_hour_count).unwrap_or(i64::MAX);
    let required_closed_frames = minimum_game_hour_advancements
        .checked_add(1)
        .unwrap_or(i64::MAX);
    if observed_advancements < minimum_game_hour_advancements
        || observed_closed_frames < required_closed_frames
    {
        errors.push(format!(
            "production proof duration is {observed_advancements} advancement(s) across {observed_closed_frames} closed frame(s), below required {minimum_game_hour_advancements} advancement(s) / {required_closed_frames} closed frame(s)"
        ));
    }
}

fn record_gate_status(
    offset: usize,
    status_flags: u32,
    summary: &mut GateSummary,
    errors: &mut Vec<String>,
) {
    summary.status_flags_or |= status_flags;
    if status_flags & !D55_KNOWN_STATUS_MASK != 0 {
        errors.push(format!(
            "offset {offset}: compact status has unknown bits {:#010x}",
            status_flags & !D55_KNOWN_STATUS_MASK
        ));
    }
}

pub fn result_json(gate: &GateResult) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"ok\": {},\n",
            "  \"errors\": {},\n",
            "  \"record_count\": {},\n",
            "  \"row_count\": {},\n",
            "  \"phase_started_rows\": {},\n",
            "  \"phase_updated_rows\": {},\n",
            "  \"phase_ended_rows\": {},\n",
            "  \"participant_joined_rows\": {},\n",
            "  \"participant_left_unclassified_rows\": {},\n",
            "  \"unclassified_rows\": {},\n",
            "  \"side_detail_rows\": {},\n",
            "  \"attribution_rows\": {},\n",
            "  \"reserve_member_rows\": {},\n",
            "  \"terminal_record_rows\": {},\n",
            "  \"width_observation_rows\": {},\n",
            "  \"current_loss_observation_rows\": {},\n",
            "  \"panel_observation_rows\": {},\n",
            "  \"status_flags_or\": {}\n",
            "}}\n"
        ),
        gate.ok,
        json_string_array(&gate.errors),
        gate.summary.record_count,
        gate.summary.row_count,
        gate.summary.phase_started_rows,
        gate.summary.phase_updated_rows,
        gate.summary.phase_ended_rows,
        gate.summary.participant_joined_rows,
        gate.summary.participant_left_unclassified_rows,
        gate.summary.unclassified_rows,
        gate.summary.side_detail_rows,
        gate.summary.attribution_rows,
        gate.summary.reserve_member_rows,
        gate.summary.terminal_record_rows,
        gate.summary.width_observation_rows,
        gate.summary.current_loss_observation_rows,
        gate.summary.panel_observation_rows,
        gate.summary.status_flags_or
    )
}

pub fn decode_detail_events_payload(
    offset: usize,
    payload: &[u8],
) -> Result<DecodedDetailEventsPayload, Vec<String>> {
    let mut errors = Vec::new();
    // The version selects the header/parse shape (framing-level, never a
    // payload-byte sniff). v1 = events only, v2 = side detail, v3 = side detail
    // plus damage attribution, v4 = exact side progress and tactic identity,
    // v5 = exact reserve-member identities, v6 = terminal records, v7 =
    // pointer-free X2 width/current-loss transports, v8 = exact panel rows.
    if payload.len() < 4 {
        return Err(vec![format!("offset {offset}: detail payload too short")]);
    }
    let version = read_u16(payload, 0);
    let (
        header_len,
        side_detail_row_count,
        attribution_row_count,
        reserve_member_row_count,
        terminal_record_row_count,
        width_observation_row_count,
        current_loss_observation_row_count,
        panel_observation_row_count,
        side_detail_row_len,
    ) = match version {
        SCHEMA_VERSION_V1_NO_SIDE_DETAIL => (
            PAYLOAD_HEADER_LEN_V1,
            0usize,
            0usize,
            0usize,
            0usize,
            0usize,
            0usize,
            0usize,
            0usize,
        ),
        SCHEMA_VERSION_V2_SIDE_DETAIL => {
            if payload.len() < PAYLOAD_HEADER_LEN_V2 {
                return Err(vec![format!(
                    "offset {offset}: detail v2 payload too short"
                )]);
            }
            (
                PAYLOAD_HEADER_LEN_V2,
                read_u32(payload, 24) as usize,
                0usize,
                0usize,
                0usize,
                0usize,
                0usize,
                0usize,
                SIDE_DETAIL_ROW_LEN_V3,
            )
        }
        SCHEMA_VERSION_V3_ATTRIBUTION => {
            if payload.len() < PAYLOAD_HEADER_LEN_V4 {
                return Err(vec![format!(
                    "offset {offset}: detail v3 payload too short"
                )]);
            }
            (
                PAYLOAD_HEADER_LEN_V4,
                read_u32(payload, 24) as usize,
                read_u32(payload, 28) as usize,
                0usize,
                0usize,
                0usize,
                0usize,
                0usize,
                SIDE_DETAIL_ROW_LEN_V3,
            )
        }
        SCHEMA_VERSION_V4_TACTIC_PROGRESS => {
            if payload.len() < PAYLOAD_HEADER_LEN_V4 {
                return Err(vec![format!(
                    "offset {offset}: detail v4 payload too short"
                )]);
            }
            (
                PAYLOAD_HEADER_LEN_V4,
                read_u32(payload, 24) as usize,
                read_u32(payload, 28) as usize,
                0usize,
                0usize,
                0usize,
                0usize,
                0usize,
                SIDE_DETAIL_ROW_LEN,
            )
        }
        SCHEMA_VERSION_V5_RESERVE_MEMBERS => {
            if payload.len() < PAYLOAD_HEADER_LEN_V5 {
                return Err(vec![format!(
                    "offset {offset}: detail v5 payload too short"
                )]);
            }
            (
                PAYLOAD_HEADER_LEN_V5,
                read_u32(payload, 24) as usize,
                read_u32(payload, 28) as usize,
                read_u32(payload, 32) as usize,
                0usize,
                0usize,
                0usize,
                0usize,
                SIDE_DETAIL_ROW_LEN,
            )
        }
        SCHEMA_VERSION_V6_TERMINAL_RECORDS => {
            if payload.len() < PAYLOAD_HEADER_LEN_V6 {
                return Err(vec![format!(
                    "offset {offset}: detail v6 payload too short"
                )]);
            }
            (
                PAYLOAD_HEADER_LEN_V6,
                read_u32(payload, 24) as usize,
                read_u32(payload, 28) as usize,
                read_u32(payload, 32) as usize,
                read_u32(payload, 36) as usize,
                0usize,
                0usize,
                0usize,
                SIDE_DETAIL_ROW_LEN,
            )
        }
        SCHEMA_VERSION_V7_WIDTH_LOSS => {
            if payload.len() < PAYLOAD_HEADER_LEN {
                return Err(vec![format!(
                    "offset {offset}: detail v7 payload too short"
                )]);
            }
            (
                PAYLOAD_HEADER_LEN,
                read_u32(payload, 24) as usize,
                read_u32(payload, 28) as usize,
                read_u32(payload, 32) as usize,
                read_u32(payload, 36) as usize,
                read_u32(payload, 40) as usize,
                read_u32(payload, 44) as usize,
                0usize,
                SIDE_DETAIL_ROW_LEN,
            )
        }
        SCHEMA_VERSION => {
            if payload.len() < PAYLOAD_HEADER_LEN {
                return Err(vec![format!(
                    "offset {offset}: detail v8 payload too short"
                )]);
            }
            (
                PAYLOAD_HEADER_LEN,
                read_u32(payload, 24) as usize,
                read_u32(payload, 28) as usize,
                read_u32(payload, 32) as usize,
                read_u32(payload, 36) as usize,
                read_u32(payload, 40) as usize,
                read_u32(payload, 44) as usize,
                read_u32(payload, 48) as usize,
                SIDE_DETAIL_ROW_LEN,
            )
        }
        other => {
            return Err(vec![format!(
                "offset {offset}: detail schema version {other} unsupported"
            )]);
        }
    };
    if payload.len() < header_len {
        return Err(vec![format!("offset {offset}: detail payload too short")]);
    }
    let row_len = read_u16(payload, 2) as usize;
    let elapsed_wall_ms = read_u64(payload, 4);
    let status_flags = read_u32(payload, 12);
    let row_count = read_u32(payload, 16) as usize;
    let string_count = read_u32(payload, 20) as usize;
    // v8 assigns the final v7 reserved word to panel_observation_row_count.
    let reserved = read_u32(payload, header_len - 4);
    if row_len != DETAIL_EVENT_ROW_LEN {
        errors.push(format!(
            "offset {offset}: detail row_len {row_len} != {DETAIL_EVENT_ROW_LEN}"
        ));
    }
    if status_flags & !D55_KNOWN_STATUS_MASK != 0 {
        errors.push(format!(
            "offset {offset}: detail status has unknown bits {:#010x}",
            status_flags & !D55_KNOWN_STATUS_MASK
        ));
    }
    if status_flags & STATUS_DELTA_DATA == 0 {
        errors.push(format!(
            "offset {offset}: detail status lacks canonical DELTA_DATA"
        ));
    }
    if status_flags & STATUS_TRANSITION_BASELINE_RESET != 0
        && status_flags & STATUS_PARTIAL_DATA == 0
    {
        errors.push(format!(
            "offset {offset}: transition baseline reset is not badged PARTIAL_DATA"
        ));
    }
    if (!matches!(
        version,
        SCHEMA_VERSION_V4_TACTIC_PROGRESS
            | SCHEMA_VERSION_V5_RESERVE_MEMBERS
            | SCHEMA_VERSION_V6_TERMINAL_RECORDS
            | SCHEMA_VERSION_V7_WIDTH_LOSS
            | SCHEMA_VERSION
    ) && string_count != 0)
        || (version != SCHEMA_VERSION && reserved != 0)
    {
        errors.push(format!(
            "offset {offset}: detail payload header reserved/string fields nonzero"
        ));
    }
    if string_count > MAX_TACTIC_STRING_COUNT {
        errors.push(format!(
            "offset {offset}: tactic string_count {string_count} exceeds max {MAX_TACTIC_STRING_COUNT}"
        ));
    }
    if row_count > MAX_DETAIL_EVENT_ROWS {
        errors.push(format!(
            "offset {offset}: detail row_count {row_count} exceeds max {MAX_DETAIL_EVENT_ROWS}"
        ));
    }
    if side_detail_row_count > MAX_LAND_COMBATS_SIDE_DETAIL {
        errors.push(format!(
            "offset {offset}: side_detail_row_count {side_detail_row_count} exceeds max"
        ));
    }
    if attribution_row_count > MAX_LAND_COMBATS_ATTRIBUTION {
        errors.push(format!(
            "offset {offset}: attribution_row_count {attribution_row_count} exceeds max"
        ));
    }
    if reserve_member_row_count > MAX_LAND_COMBATS_RESERVE_MEMBERS {
        errors.push(format!(
            "offset {offset}: reserve_member_row_count {reserve_member_row_count} exceeds max"
        ));
    }
    if terminal_record_row_count > MAX_LAND_COMBAT_TERMINAL_ROWS {
        errors.push(format!(
            "offset {offset}: terminal_record_row_count {terminal_record_row_count} exceeds max"
        ));
    }
    if width_observation_row_count > MAX_LAND_COMBAT_WIDTH_ROWS {
        errors.push(format!(
            "offset {offset}: width_observation_row_count {width_observation_row_count} exceeds max"
        ));
    }
    if current_loss_observation_row_count > MAX_LAND_COMBAT_LOSS_ROWS {
        errors.push(format!(
            "offset {offset}: current_loss_observation_row_count {current_loss_observation_row_count} exceeds max"
        ));
    }
    if panel_observation_row_count > MAX_LAND_COMBAT_PANEL_ROWS {
        errors.push(format!(
            "offset {offset}: panel_observation_row_count {panel_observation_row_count} exceeds max"
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let fixed_len = header_len
        .saturating_add(row_count.saturating_mul(row_len))
        .saturating_add(side_detail_row_count.saturating_mul(side_detail_row_len))
        .saturating_add(attribution_row_count.saturating_mul(ATTRIBUTION_ROW_LEN))
        .saturating_add(reserve_member_row_count.saturating_mul(RESERVE_MEMBER_ROW_LEN))
        .saturating_add(terminal_record_row_count.saturating_mul(TERMINAL_RECORD_ROW_LEN))
        .saturating_add(width_observation_row_count.saturating_mul(WIDTH_OBSERVATION_ROW_LEN))
        .saturating_add(
            current_loss_observation_row_count.saturating_mul(CURRENT_LOSS_OBSERVATION_ROW_LEN),
        )
        .saturating_add(panel_observation_row_count.saturating_mul(PANEL_OBSERVATION_ROW_LEN));
    if payload.len() < fixed_len {
        errors.push(format!(
            "offset {offset}: detail payload len {} shorter than fixed rows {fixed_len}",
            payload.len()
        ));
        return Err(errors);
    }
    let mut strings = Vec::with_capacity(string_count);
    let mut string_cursor = fixed_len;
    for string_index in 0..string_count {
        if string_cursor.saturating_add(4) > payload.len() {
            return Err(vec![format!(
                "offset {offset}: tactic string {string_index} length overruns payload"
            )]);
        }
        let len = read_u32(payload, string_cursor) as usize;
        string_cursor += 4;
        if len == 0
            || len > MAX_TACTIC_STRING_LEN
            || string_cursor.saturating_add(len) > payload.len()
        {
            return Err(vec![format!(
                "offset {offset}: tactic string {string_index} invalid length {len}"
            )]);
        }
        let bytes = &payload[string_cursor..string_cursor + len];
        let value = std::str::from_utf8(bytes)
            .map_err(|_| {
                vec![format!(
                    "offset {offset}: tactic string {string_index} is not UTF-8"
                )]
            })?
            .to_string();
        if value.chars().any(char::is_control) {
            return Err(vec![format!(
                "offset {offset}: tactic string {string_index} contains control characters"
            )]);
        }
        if strings
            .last()
            .is_some_and(|previous: &String| previous >= &value)
        {
            return Err(vec![format!(
                "offset {offset}: tactic string table is not strictly canonical"
            )]);
        }
        strings.push(value);
        string_cursor += len;
    }
    if string_cursor != payload.len() {
        return Err(vec![format!(
            "offset {offset}: detail payload has {} trailing bytes after tactic strings",
            payload.len() - string_cursor
        )]);
    }
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(row_count);
    let mut cursor = header_len;
    for row_index in 0..row_count {
        let row = DecodedRow {
            combat_id: read_u32(payload, cursor),
            generation: read_u64(payload, cursor + 4),
            event_kind: read_u32(payload, cursor + 12),
            side: read_u32(payload, cursor + 16),
            division_id: read_u32(payload, cursor + 20),
            flags: read_u32(payload, cursor + 24),
            location: read_u32(payload, cursor + 28),
            day: read_u32(payload, cursor + 32),
            duration: read_u32(payload, cursor + 36),
            old_side_count: read_u32(payload, cursor + 40),
            new_side_count: read_u32(payload, cursor + 44),
            reserved: read_u32(payload, cursor + 48),
        };
        cursor += row_len;
        validate_decoded_row(offset, row_index, &row, &mut seen, &mut errors);
        rows.push(DetailEventRow {
            combat_id: row.combat_id,
            generation: row.generation,
            event_kind: row.event_kind,
            side: match row.side {
                SIDE_ATTACKER => Some(CombatSide::Attacker),
                SIDE_DEFENDER => Some(CombatSide::Defender),
                _ => None,
            },
            division_id: if row.flags & FLAG_DIVISION_PRESENT != 0 {
                Some(row.division_id)
            } else {
                None
            },
            location: if row.flags & FLAG_LOCATION_PRESENT != 0 {
                Some(row.location)
            } else {
                None
            },
            day: if row.flags & FLAG_DAY_PRESENT != 0 {
                Some(row.day)
            } else {
                None
            },
            duration: if row.flags & FLAG_DURATION_PRESENT != 0 {
                Some(row.duration)
            } else {
                None
            },
            old_side_count: row.old_side_count,
            new_side_count: row.new_side_count,
            flags: row.flags,
        });
    }

    // v2 per-side detail family (empty for v1).
    let mut side_detail_rows = Vec::with_capacity(side_detail_row_count);
    let mut seen_side = BTreeSet::new();
    for row_index in 0..side_detail_row_count {
        match decode_side_detail_row(
            offset,
            row_index,
            payload,
            cursor,
            version,
            &strings,
            &mut seen_side,
        ) {
            Ok(row) => side_detail_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += side_detail_row_len;
    }

    let mut attribution_rows = Vec::with_capacity(attribution_row_count);
    let mut seen_attribution = BTreeSet::new();
    for row_index in 0..attribution_row_count {
        match decode_attribution_row(offset, row_index, payload, cursor, &mut seen_attribution) {
            Ok(row) => attribution_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += ATTRIBUTION_ROW_LEN;
    }

    let mut reserve_member_rows = Vec::with_capacity(reserve_member_row_count);
    let mut seen_reserve = BTreeSet::new();
    for row_index in 0..reserve_member_row_count {
        match decode_reserve_member_row(offset, row_index, payload, cursor, &mut seen_reserve) {
            Ok(row) => reserve_member_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += RESERVE_MEMBER_ROW_LEN;
    }

    let mut terminal_record_rows = Vec::with_capacity(terminal_record_row_count);
    let mut seen_terminal = BTreeSet::new();
    for row_index in 0..terminal_record_row_count {
        match decode_terminal_record_row(offset, row_index, payload, cursor, &mut seen_terminal) {
            Ok(row) => terminal_record_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += TERMINAL_RECORD_ROW_LEN;
    }

    let mut width_observation_rows = Vec::with_capacity(width_observation_row_count);
    for row_index in 0..width_observation_row_count {
        match decode_width_observation_row(offset, row_index, payload, cursor) {
            Ok(row) => width_observation_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += WIDTH_OBSERVATION_ROW_LEN;
    }

    let mut current_loss_observation_rows = Vec::with_capacity(current_loss_observation_row_count);
    for row_index in 0..current_loss_observation_row_count {
        match decode_current_loss_observation_row(offset, row_index, payload, cursor) {
            Ok(row) => current_loss_observation_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += CURRENT_LOSS_OBSERVATION_ROW_LEN;
    }
    let mut panel_observation_rows = Vec::with_capacity(panel_observation_row_count);
    let mut seen_panel = BTreeSet::new();
    for row_index in 0..panel_observation_row_count {
        match decode_panel_observation_row(offset, row_index, payload, cursor, &mut seen_panel) {
            Ok(row) => panel_observation_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += PANEL_OBSERVATION_ROW_LEN;
    }

    validate_width_observation_groups(offset, &width_observation_rows, &mut errors);
    validate_current_loss_observation_groups(offset, &current_loss_observation_rows, &mut errors);

    validate_payload_referential_consistency(
        offset,
        &rows,
        &side_detail_rows,
        &attribution_rows,
        &reserve_member_rows,
        &terminal_record_rows,
        &width_observation_rows,
        &current_loss_observation_rows,
        &panel_observation_rows,
        &mut errors,
    );

    if errors.is_empty() {
        Ok(DecodedDetailEventsPayload {
            elapsed_wall_ms,
            status_flags,
            rows,
            side_detail_rows,
            attribution_rows,
            reserve_member_rows,
            terminal_record_rows,
            width_observation_rows,
            current_loss_observation_rows,
            panel_observation_rows,
        })
    } else {
        Err(errors)
    }
}

fn decode_panel_observation_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u64, CombatSide)>,
) -> Result<PanelObservationRow, String> {
    let side = decode_observation_side(read_u32(payload, cursor + 12))
        .ok_or_else(|| format!("offset {offset}: panel row {row_index}: side invalid"))?;
    let row = PanelObservationRow {
        combat_id: read_u32(payload, cursor),
        generation: read_u64(payload, cursor + 4),
        side,
        capture_sequence: read_u64(payload, cursor + 16),
        leader_id: read_u32(payload, cursor + 24),
        modifier_bits: read_u32(payload, cursor + 28),
        control_flags: read_u32(payload, cursor + 32),
    };
    let reserved = read_u32(payload, cursor + 36);
    validate_panel_observation_row(&row)
        .map_err(|error| format!("offset {offset}: panel row {row_index}: {error}"))?;
    if reserved != 0 || !seen.insert((row.combat_id, row.generation, row.side)) {
        return Err(format!(
            "offset {offset}: panel row {row_index}: reserved nonzero or duplicate side"
        ));
    }
    Ok(row)
}

fn decode_width_observation_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
) -> Result<WidthObservationRow, String> {
    let side = decode_observation_side(read_u32(payload, cursor + 12))
        .ok_or_else(|| format!("offset {offset}: width row {row_index}: side invalid"))?;
    let row = WidthObservationRow {
        combat_id: read_u32(payload, cursor),
        generation: read_u64(payload, cursor + 4),
        side,
        capture_sequence: read_u64(payload, cursor + 16),
        ordinal: read_u32(payload, cursor + 24),
        wrapper_count: read_u32(payload, cursor + 28),
        division_id: read_u32(payload, cursor + 32),
        control_flags: read_u32(payload, cursor + 36),
        wrapper_width_raw: read_i64(payload, cursor + 40),
        final_width_raw: read_i64(payload, cursor + 48),
    };
    validate_width_observation_row(&row)
        .map_err(|error| format!("offset {offset}: width row {row_index}: {error}"))?;
    Ok(row)
}

fn decode_current_loss_observation_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
) -> Result<CurrentLossObservationRow, String> {
    let side = decode_observation_side(read_u32(payload, cursor + 12))
        .ok_or_else(|| format!("offset {offset}: loss row {row_index}: side invalid"))?;
    let row = CurrentLossObservationRow {
        combat_id: read_u32(payload, cursor),
        generation: read_u64(payload, cursor + 4),
        side,
        capture_sequence: read_u64(payload, cursor + 16),
        ordinal: read_u32(payload, cursor + 24),
        equipment_count: read_u32(payload, cursor + 28),
        equipment_id: read_u32(payload, cursor + 32),
        control_flags: read_u32(payload, cursor + 36),
        manpower_before_raw: read_i64(payload, cursor + 40),
        manpower_after_raw: read_i64(payload, cursor + 48),
        equipment_before_raw: read_i64(payload, cursor + 56),
        equipment_after_raw: read_i64(payload, cursor + 64),
    };
    validate_current_loss_observation_row(&row)
        .map_err(|error| format!("offset {offset}: loss row {row_index}: {error}"))?;
    Ok(row)
}

fn decode_observation_side(value: u32) -> Option<CombatSide> {
    match value {
        SIDE_ATTACKER => Some(CombatSide::Attacker),
        SIDE_DEFENDER => Some(CombatSide::Defender),
        _ => None,
    }
}

fn validate_width_observation_groups(
    offset: usize,
    rows: &[WidthObservationRow],
    errors: &mut Vec<String>,
) {
    let mut groups: BTreeMap<(u32, u64, u32, u64), Vec<&WidthObservationRow>> = BTreeMap::new();
    let mut sequence_owner = BTreeMap::new();
    for row in rows {
        let owner = (row.combat_id, row.generation, row.side.wire_id());
        if sequence_owner
            .insert(row.capture_sequence, owner)
            .is_some_and(|previous| previous != owner)
        {
            errors.push(format!(
                "offset {offset}: width capture sequence {} is reused across active identities",
                row.capture_sequence
            ));
        }
        groups
            .entry((
                row.combat_id,
                row.generation,
                row.side.wire_id(),
                row.capture_sequence,
            ))
            .or_default()
            .push(row);
    }
    for (key, group) in groups {
        let expected = group[0].wrapper_count as usize;
        let final_raw = group[0].final_width_raw;
        let ordinals = group.iter().map(|row| row.ordinal).collect::<BTreeSet<_>>();
        let divisions = group
            .iter()
            .map(|row| row.division_id)
            .collect::<BTreeSet<_>>();
        let sum = group
            .iter()
            .try_fold(0i64, |sum, row| sum.checked_add(row.wrapper_width_raw));
        if group.len() != expected
            || ordinals.len() != expected
            || !ordinals.iter().copied().eq(0..expected as u32)
            || divisions.len() != expected
            || group.iter().any(|row| {
                row.wrapper_count as usize != expected || row.final_width_raw != final_raw
            })
            || sum != Some(final_raw)
        {
            errors.push(format!(
                "offset {offset}: width observation group {:?} is not complete/order-unique/exact-sum",
                key
            ));
        }
    }
}

fn validate_current_loss_observation_groups(
    offset: usize,
    rows: &[CurrentLossObservationRow],
    errors: &mut Vec<String>,
) {
    let mut groups: BTreeMap<(u32, u64, u32, u64), Vec<&CurrentLossObservationRow>> =
        BTreeMap::new();
    let mut sequence_owner = BTreeMap::new();
    for row in rows {
        let owner = (row.combat_id, row.generation, row.side.wire_id());
        if sequence_owner
            .insert(row.capture_sequence, owner)
            .is_some_and(|previous| previous != owner)
        {
            errors.push(format!(
                "offset {offset}: loss capture sequence {} is reused across active identities",
                row.capture_sequence
            ));
        }
        groups
            .entry((
                row.combat_id,
                row.generation,
                row.side.wire_id(),
                row.capture_sequence,
            ))
            .or_default()
            .push(row);
    }
    for (key, group) in groups {
        let expected = group[0].equipment_count as usize;
        let manpower = (group[0].manpower_before_raw, group[0].manpower_after_raw);
        let source_flags = group[0].control_flags & CURRENT_LOSS_SOURCE_FLAG_MASK;
        let ordinals = group.iter().map(|row| row.ordinal).collect::<BTreeSet<_>>();
        let equipment = group
            .iter()
            .map(|row| row.equipment_id)
            .collect::<BTreeSet<_>>();
        if group.len() != expected
            || ordinals.len() != expected
            || !ordinals.iter().copied().eq(0..expected as u32)
            || equipment.len() != expected
            || group.iter().any(|row| {
                row.equipment_count as usize != expected
                    || (row.manpower_before_raw, row.manpower_after_raw) != manpower
                    || row.control_flags & CURRENT_LOSS_SOURCE_FLAG_MASK != source_flags
            })
        {
            errors.push(format!(
                "offset {offset}: current-loss observation group {:?} is not complete/order-unique/consistent",
                key
            ));
        }
    }
}

fn decode_terminal_record_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u64)>,
) -> Result<TerminalCombatRecordRow, String> {
    let row = TerminalCombatRecordRow {
        combat_id: read_u32(payload, cursor),
        generation: read_u64(payload, cursor + 4),
        record_index: read_u32(payload, cursor + 12),
        writer_kind: read_u32(payload, cursor + 16),
        attacker_manpower_loss_raw: read_i64(payload, cursor + 20),
        defender_manpower_loss_raw: read_i64(payload, cursor + 28),
        end_date_raw: read_u32(payload, cursor + 36),
        location: read_u32(payload, cursor + 40),
        winner_side_raw: payload[cursor + 44],
        winter_raw: payload[cursor + 45],
        result_selector_raw: payload[cursor + 46],
        layout_side_raw: payload[cursor + 47],
    };
    validate_terminal_record_row(&row)
        .map_err(|error| format!("offset {offset}: terminal row {row_index}: {error}"))?;
    if !seen.insert((row.combat_id, row.generation)) {
        return Err(format!(
            "offset {offset}: terminal row {row_index} duplicates combat {}:{} (record {})",
            row.combat_id, row.generation, row.record_index
        ));
    }
    Ok(row)
}

fn decode_side_detail_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    schema_version: u16,
    strings: &[String],
    seen: &mut BTreeSet<(u32, u64, u32)>,
) -> Result<SideDetailRow, String> {
    let combat_id = read_u32(payload, cursor);
    let generation = read_u64(payload, cursor + 4);
    let side_wire = read_u32(payload, cursor + 12);
    let flags = read_u32(payload, cursor + 16);
    let units = read_u32(payload, cursor + 20);
    let committed = read_u32(payload, cursor + 24);
    let reserve = read_u32(payload, cursor + 28);
    let air_kills = read_u32(payload, cursor + 32);
    let org_loss_summary_index = read_u32(payload, cursor + 36);
    let num_org_losses = read_u32(payload, cursor + 40);
    let anti_air_attack_raw = read_i64(payload, cursor + 44);
    let air_damage_str_raw = read_i64(payload, cursor + 52);
    let air_damage_org_raw = read_i64(payload, cursor + 60);
    let ground_damage_str_raw = read_i64(payload, cursor + 68);
    let ground_damage_org_raw = read_i64(payload, cursor + 76);
    let prevented_damage_str_raw = read_i64(payload, cursor + 84);
    let prevented_damage_org_raw = read_i64(payload, cursor + 92);
    let has_tactic_progress = matches!(
        schema_version,
        SCHEMA_VERSION_V4_TACTIC_PROGRESS
            | SCHEMA_VERSION_V5_RESERVE_MEMBERS
            | SCHEMA_VERSION_V6_TERMINAL_RECORDS
            | SCHEMA_VERSION_V7_WIDTH_LOSS
            | SCHEMA_VERSION
    );
    let progress_raw = has_tactic_progress.then(|| read_i32(payload, cursor + 100));
    let tactic_id = has_tactic_progress.then(|| read_i32(payload, cursor + 104));
    let tactic_token_index = has_tactic_progress
        .then(|| read_u32(payload, cursor + 108))
        .unwrap_or(TACTIC_STRING_INDEX_NONE);
    let tactic_name_index = has_tactic_progress
        .then(|| read_u32(payload, cursor + 112))
        .unwrap_or(TACTIC_STRING_INDEX_NONE);

    if combat_id == 0 || combat_id > MAX_COMBAT_ID {
        return Err(format!(
            "offset {offset}: side detail row {row_index} invalid combat_id"
        ));
    }
    if generation == 0 {
        return Err(format!(
            "offset {offset}: side detail row {row_index} has zero generation"
        ));
    }
    if flags & !SIDE_DETAIL_KNOWN_FLAG_MASK != 0 {
        return Err(format!(
            "offset {offset}: side detail row {row_index} unknown flags {}",
            flags & !SIDE_DETAIL_KNOWN_FLAG_MASK
        ));
    }
    if flags & SIDE_DETAIL_FLAG_SIDE_VALIDATED == 0 {
        return Err(format!(
            "offset {offset}: side detail row {row_index} lacks side-validated flag"
        ));
    }
    if has_tactic_progress {
        if flags & SIDE_DETAIL_FLAG_PROGRESS_PRESENT == 0 {
            return Err(format!(
                "offset {offset}: side detail row {row_index} lacks exact progress"
            ));
        }
        if flags & SIDE_DETAIL_FLAG_TACTIC_KNOWN == 0
            || flags & SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED != 0
        {
            return Err(format!(
                "offset {offset}: side detail row {row_index} tactic is not authoritative"
            ));
        }
    }
    let side = match side_wire {
        SIDE_ATTACKER => {
            if flags & (SIDE_DETAIL_FLAG_ATTACKER | SIDE_DETAIL_FLAG_DEFENDER)
                != SIDE_DETAIL_FLAG_ATTACKER
            {
                return Err(format!(
                    "offset {offset}: side detail row {row_index} attacker side/flag mismatch"
                ));
            }
            CombatSide::Attacker
        }
        SIDE_DEFENDER => {
            if flags & (SIDE_DETAIL_FLAG_ATTACKER | SIDE_DETAIL_FLAG_DEFENDER)
                != SIDE_DETAIL_FLAG_DEFENDER
            {
                return Err(format!(
                    "offset {offset}: side detail row {row_index} defender side/flag mismatch"
                ));
            }
            CombatSide::Defender
        }
        other => {
            return Err(format!(
                "offset {offset}: side detail row {row_index} invalid side {other}"
            ));
        }
    };
    for (label, value) in [
        ("units", units),
        ("committed", committed),
        ("reserve", reserve),
        ("air_kills", air_kills),
        ("org_loss_summary_index", org_loss_summary_index),
        ("num_org_losses", num_org_losses),
    ] {
        if value > MAX_SIDE_DETAIL_COUNT {
            return Err(format!(
                "offset {offset}: side detail row {row_index} {label} {value} exceeds max"
            ));
        }
    }
    // The committed+reserve==units invariant is REQUIRED unless the row is
    // explicitly badged COUNTS_INCOMPLETE (a genuine mid-combat transient the
    // collector observed with a stable double-read). Without the badge, a
    // violation is a corrupt/torn payload and is rejected.
    let counts_incomplete = flags & SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE != 0;
    if !counts_incomplete && committed.saturating_add(reserve) != units {
        return Err(format!(
            "offset {offset}: side detail row {row_index} committed+reserve != units without incomplete badge"
        ));
    }
    for (label, raw) in [
        ("anti_air_attack", anti_air_attack_raw),
        ("air_damage_str", air_damage_str_raw),
        ("air_damage_org", air_damage_org_raw),
        ("ground_damage_str", ground_damage_str_raw),
        ("ground_damage_org", ground_damage_org_raw),
        ("prevented_damage_str", prevented_damage_str_raw),
        ("prevented_damage_org", prevented_damage_org_raw),
    ] {
        if !(0..=MAX_SIDE_DETAIL_DAMAGE_RAW).contains(&raw) {
            return Err(format!(
                "offset {offset}: side detail row {row_index} {label} raw {raw} out of range"
            ));
        }
    }
    if !seen.insert((combat_id, generation, side_wire)) {
        return Err(format!(
            "offset {offset}: side detail row {row_index} duplicates (combat,generation,side)"
        ));
    }
    let tactic_present = flags & SIDE_DETAIL_FLAG_TACTIC_PRESENT != 0;
    let tactic = if tactic_present {
        let token = strings.get(tactic_token_index as usize).ok_or_else(|| {
            format!("offset {offset}: side detail row {row_index} tactic token index invalid")
        })?;
        let name = strings.get(tactic_name_index as usize).ok_or_else(|| {
            format!("offset {offset}: side detail row {row_index} tactic name index invalid")
        })?;
        Some(TacticIdentity {
            id: tactic_id.unwrap_or_default(),
            token: token.clone(),
            name: name.clone(),
        })
    } else {
        if tactic_token_index != TACTIC_STRING_INDEX_NONE
            || tactic_name_index != TACTIC_STRING_INDEX_NONE
            || tactic_id.unwrap_or_default() != 0
        {
            return Err(format!(
                "offset {offset}: side detail row {row_index} absent tactic has noncanonical payload"
            ));
        }
        None
    };
    Ok(SideDetailRow {
        combat_id,
        generation,
        side,
        reserve_members_known: flags & SIDE_DETAIL_FLAG_RESERVE_MEMBERS_KNOWN != 0,
        detail: SideDetail {
            units,
            committed,
            reserve,
            air_kills,
            org_loss_summary_index,
            num_org_losses,
            anti_air_attack_raw,
            air_damage_str_raw,
            air_damage_org_raw,
            ground_damage_str_raw,
            ground_damage_org_raw,
            prevented_damage_str_raw,
            prevented_damage_org_raw,
            progress_raw,
            tactic,
            tactic_known: has_tactic_progress,
            counts_incomplete,
        },
    })
}

#[derive(Debug)]
struct ParticipantDeltaSet {
    old_count: u32,
    new_count: u32,
    joined: BTreeSet<u32>,
    left: BTreeSet<u32>,
}

fn validate_payload_referential_consistency(
    offset: usize,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
    attribution_rows: &[AttributionRow],
    reserve_member_rows: &[ReserveMemberRow],
    terminal_record_rows: &[TerminalCombatRecordRow],
    width_observation_rows: &[WidthObservationRow],
    current_loss_observation_rows: &[CurrentLossObservationRow],
    panel_observation_rows: &[PanelObservationRow],
    errors: &mut Vec<String>,
) {
    let mut metadata_by_combat = BTreeMap::new();
    let mut phase_by_combat = BTreeMap::new();
    let mut participant_sets: BTreeMap<(u32, u64, CombatSide), ParticipantDeltaSet> =
        BTreeMap::new();
    let mut participant_identity = BTreeSet::new();
    let side_details = side_detail_rows
        .iter()
        .map(|row| ((row.combat_id, row.generation, row.side), row))
        .collect::<BTreeMap<_, _>>();

    for (row_index, row) in rows.iter().enumerate() {
        let combat_key = (row.combat_id, row.generation);
        let metadata = (row.location, row.day, row.duration);
        match metadata_by_combat.get(&combat_key) {
            Some(previous) if *previous != metadata => errors.push(format!(
                "offset {offset}: detail row {row_index} disagrees on combat {}:{} location/day/duration",
                row.combat_id, row.generation
            )),
            Some(_) => {}
            None => {
                metadata_by_combat.insert(combat_key, metadata);
            }
        }

        if matches!(
            row.event_kind,
            EVENT_PHASE_STARTED | EVENT_PHASE_UPDATED | EVENT_PHASE_ENDED
        ) {
            if let Some(previous_kind) = phase_by_combat.insert(combat_key, row.event_kind) {
                errors.push(format!(
                    "offset {offset}: combat {}:{} has multiple phase events ({previous_kind} and {}) in one payload",
                    row.combat_id, row.generation, row.event_kind
                ));
            }
            continue;
        }

        if !matches!(
            row.event_kind,
            EVENT_PARTICIPANT_JOINED | EVENT_PARTICIPANT_LEFT_UNCLASSIFIED
        ) {
            continue;
        }
        let (Some(side), Some(division_id)) = (row.side, row.division_id) else {
            continue;
        };
        if !participant_identity.insert((row.combat_id, row.generation, division_id)) {
            errors.push(format!(
                "offset {offset}: division {division_id} has multiple participant events for combat {}:{} in one payload",
                row.combat_id, row.generation
            ));
        }
        let key = (row.combat_id, row.generation, side);
        let set = participant_sets
            .entry(key)
            .or_insert_with(|| ParticipantDeltaSet {
                old_count: row.old_side_count,
                new_count: row.new_side_count,
                joined: BTreeSet::new(),
                left: BTreeSet::new(),
            });
        if set.old_count != row.old_side_count || set.new_count != row.new_side_count {
            errors.push(format!(
                "offset {offset}: participant rows for combat {}:{} {} disagree on old/new side counts",
                row.combat_id,
                row.generation,
                match side {
                    CombatSide::Attacker => "attacker",
                    CombatSide::Defender => "defender",
                }
            ));
        }
        if row.event_kind == EVENT_PARTICIPANT_JOINED {
            set.joined.insert(division_id);
        } else {
            set.left.insert(division_id);
        }
    }

    for ((combat_id, generation, side), set) in participant_sets {
        let expected = i64::from(set.old_count) + set.joined.len() as i64 - set.left.len() as i64;
        if expected != i64::from(set.new_count) {
            errors.push(format!(
                "offset {offset}: participant delta for combat {combat_id}:{generation} {} is partial: old {} + {} joined - {} left != new {}",
                match side {
                    CombatSide::Attacker => "attacker",
                    CombatSide::Defender => "defender",
                },
                set.old_count,
                set.joined.len(),
                set.left.len(),
                set.new_count
            ));
        }
        if let Some(detail) = side_details.get(&(combat_id, generation, side)) {
            if !detail.detail.counts_incomplete && detail.detail.units != set.new_count {
                errors.push(format!(
                    "offset {offset}: participant delta new count {} disagrees with side-detail units {} for combat {combat_id}:{generation}",
                    set.new_count, detail.detail.units
                ));
            }
        }
    }

    for row in attribution_rows {
        if !side_details.contains_key(&(row.combat_id, row.generation, row.side)) {
            errors.push(format!(
                "offset {offset}: attribution for combat {}:{} has no matching side-detail row",
                row.combat_id, row.generation
            ));
        }
    }

    let mut reserve_counts = BTreeMap::<(u32, u64, CombatSide), usize>::new();
    let mut reserve_identities = BTreeMap::<(u32, u64, CombatSide), BTreeSet<u32>>::new();
    for row in reserve_member_rows {
        let key = (row.combat_id, row.generation, row.side);
        match side_details.get(&key) {
            None => errors.push(format!(
                "offset {offset}: reserve member for combat {}:{} has no matching side-detail row",
                row.combat_id, row.generation
            )),
            Some(detail) if !detail.reserve_members_known => errors.push(format!(
                "offset {offset}: reserve member for combat {}:{} is present without reserve-vector authority",
                row.combat_id, row.generation
            )),
            Some(_) => {}
        }
        let count = reserve_counts.entry(key).or_default();
        if row.ordinal as usize != *count {
            errors.push(format!(
                "offset {offset}: reserve member ordinal {} is not contiguous expected {} for combat {}:{}",
                row.ordinal, *count, row.combat_id, row.generation
            ));
        }
        *count += 1;
        if !reserve_identities
            .entry(key)
            .or_default()
            .insert(row.division_id)
        {
            errors.push(format!(
                "offset {offset}: reserve division {} repeats for combat {}:{}",
                row.division_id, row.combat_id, row.generation
            ));
        }
    }
    for (key, row) in &side_details {
        if row.detail.counts_incomplete || !row.reserve_members_known {
            continue;
        }
        let actual = reserve_counts.get(key).copied().unwrap_or(0);
        if actual != row.detail.reserve as usize {
            errors.push(format!(
                "offset {offset}: reserve member count {actual} disagrees with side-detail reserve {} for combat {}:{}",
                row.detail.reserve, key.0, key.1
            ));
        }
    }
    for row in terminal_record_rows {
        match phase_by_combat.get(&(row.combat_id, row.generation)) {
            Some(&EVENT_PHASE_ENDED) => {}
            _ => errors.push(format!(
                "offset {offset}: terminal record {} for combat {}:{} lacks the matching ended phase",
                row.record_index, row.combat_id, row.generation
            )),
        }
    }
    for row in width_observation_rows {
        match side_details.get(&(row.combat_id, row.generation, row.side)) {
            Some(detail)
                if detail.detail.counts_incomplete
                    || detail.detail.committed == row.wrapper_count => {}
            Some(detail) => errors.push(format!(
                "offset {offset}: width wrapper count {} disagrees with side-detail committed {} for combat {}:{}",
                row.wrapper_count, detail.detail.committed, row.combat_id, row.generation
            )),
            None => errors.push(format!(
                "offset {offset}: width observation for combat {}:{} has no matching side-detail row",
                row.combat_id, row.generation
            )),
        }
    }
    for row in current_loss_observation_rows {
        if !side_details.contains_key(&(row.combat_id, row.generation, row.side)) {
            errors.push(format!(
                "offset {offset}: current-loss observation for combat {}:{} has no matching side-detail row",
                row.combat_id, row.generation
            ));
        }
    }
    for row in panel_observation_rows {
        if !side_details.contains_key(&(row.combat_id, row.generation, row.side)) {
            errors.push(format!(
                "offset {offset}: panel observation for combat {}:{} has no matching side-detail row",
                row.combat_id, row.generation
            ));
        }
    }
}

fn decode_reserve_member_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u64, u32, u32, u32)>,
) -> Result<ReserveMemberRow, String> {
    let combat_id = read_u32(payload, cursor);
    let generation = read_u64(payload, cursor + 4);
    let side_wire = read_u32(payload, cursor + 12);
    let flags = read_u32(payload, cursor + 16);
    let ordinal = read_u32(payload, cursor + 20);
    let division_id = read_u32(payload, cursor + 24);
    let reserved = read_u32(payload, cursor + 28);
    if combat_id == 0 || combat_id > MAX_COMBAT_ID || generation == 0 {
        return Err(format!(
            "offset {offset}: reserve member row {row_index} invalid combat identity"
        ));
    }
    if division_id == 0 || division_id > MAX_DIVISION_ID || ordinal >= MAX_SIDE_UNITS as u32 {
        return Err(format!(
            "offset {offset}: reserve member row {row_index} invalid ordinal/division"
        ));
    }
    if reserved != 0 || flags & !RESERVE_MEMBER_KNOWN_FLAG_MASK != 0 {
        return Err(format!(
            "offset {offset}: reserve member row {row_index} invalid flags/reserved"
        ));
    }
    if flags & (RESERVE_MEMBER_FLAG_SIDE_VALIDATED | RESERVE_MEMBER_FLAG_VECTOR_STABLE)
        != RESERVE_MEMBER_FLAG_SIDE_VALIDATED | RESERVE_MEMBER_FLAG_VECTOR_STABLE
    {
        return Err(format!(
            "offset {offset}: reserve member row {row_index} lacks validation flags"
        ));
    }
    let side = match side_wire {
        SIDE_ATTACKER
            if flags & (RESERVE_MEMBER_FLAG_ATTACKER | RESERVE_MEMBER_FLAG_DEFENDER)
                == RESERVE_MEMBER_FLAG_ATTACKER =>
        {
            CombatSide::Attacker
        }
        SIDE_DEFENDER
            if flags & (RESERVE_MEMBER_FLAG_ATTACKER | RESERVE_MEMBER_FLAG_DEFENDER)
                == RESERVE_MEMBER_FLAG_DEFENDER =>
        {
            CombatSide::Defender
        }
        _ => {
            return Err(format!(
                "offset {offset}: reserve member row {row_index} side/flag mismatch"
            ));
        }
    };
    if !seen.insert((combat_id, generation, side_wire, ordinal, division_id)) {
        return Err(format!(
            "offset {offset}: reserve member row {row_index} duplicates identity"
        ));
    }
    // An ID may appear only once per side even if a corrupt payload changes its ordinal.
    if seen
        .iter()
        .filter(|(c, g, s, _, d)| {
            *c == combat_id && *g == generation && *s == side_wire && *d == division_id
        })
        .count()
        != 1
    {
        return Err(format!(
            "offset {offset}: reserve member row {row_index} repeats division_id"
        ));
    }
    Ok(ReserveMemberRow {
        combat_id,
        generation,
        side,
        ordinal,
        division_id,
    })
}

fn decode_attribution_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u64, u32, u32, u32)>,
) -> Result<AttributionRow, String> {
    let combat_id = read_u32(payload, cursor);
    let generation = read_u64(payload, cursor + 4);
    let side_wire = read_u32(payload, cursor + 12);
    let flags = read_u32(payload, cursor + 16);
    let dealer_tag_db_id = read_u32(payload, cursor + 20);
    let victim_tag_db_id = read_u32(payload, cursor + 24);
    let damage_raw = read_u64(payload, cursor + 28);
    let reserved = read_u32(payload, cursor + 36);

    if combat_id == 0 || combat_id > MAX_COMBAT_ID {
        return Err(format!(
            "offset {offset}: attribution row {row_index} invalid combat_id"
        ));
    }
    if generation == 0 {
        return Err(format!(
            "offset {offset}: attribution row {row_index} has zero generation"
        ));
    }
    if reserved != 0 {
        return Err(format!(
            "offset {offset}: attribution row {row_index} reserved field is nonzero"
        ));
    }
    if flags & !ATTRIBUTION_KNOWN_FLAG_MASK != 0 {
        return Err(format!(
            "offset {offset}: attribution row {row_index} unknown flags {}",
            flags & !ATTRIBUTION_KNOWN_FLAG_MASK
        ));
    }
    if flags & ATTRIBUTION_FLAG_SIDE_VALIDATED == 0
        || flags & ATTRIBUTION_FLAG_DAMAGE_ROW_VALIDATED == 0
        || flags & ATTRIBUTION_FLAG_DAMAGE_FIXED_POINT == 0
    {
        return Err(format!(
            "offset {offset}: attribution row {row_index} lacks validation flags"
        ));
    }
    let side = match side_wire {
        SIDE_ATTACKER => {
            if flags & (ATTRIBUTION_FLAG_ATTACKER | ATTRIBUTION_FLAG_DEFENDER)
                != ATTRIBUTION_FLAG_ATTACKER
            {
                return Err(format!(
                    "offset {offset}: attribution row {row_index} attacker side/flag mismatch"
                ));
            }
            CombatSide::Attacker
        }
        SIDE_DEFENDER => {
            if flags & (ATTRIBUTION_FLAG_ATTACKER | ATTRIBUTION_FLAG_DEFENDER)
                != ATTRIBUTION_FLAG_DEFENDER
            {
                return Err(format!(
                    "offset {offset}: attribution row {row_index} defender side/flag mismatch"
                ));
            }
            CombatSide::Defender
        }
        other => {
            return Err(format!(
                "offset {offset}: attribution row {row_index} invalid side {other}"
            ));
        }
    };
    let entry = AttributionEntry {
        dealer_tag_db_id,
        victim_tag_db_id,
        damage_raw,
    };
    validate_attribution_entry(entry)
        .map_err(|err| format!("offset {offset}: attribution row {row_index} {err}"))?;
    if !seen.insert((
        combat_id,
        generation,
        side_wire,
        dealer_tag_db_id,
        victim_tag_db_id,
    )) {
        return Err(format!(
            "offset {offset}: attribution row {row_index} duplicates (combat,generation,side,dealer,victim)"
        ));
    }
    Ok(AttributionRow {
        combat_id,
        generation,
        side,
        entry,
    })
}

fn decode_detail_events_payload_into_summary(
    offset: usize,
    payload: &[u8],
    summary: &mut GateSummary,
    errors: &mut Vec<String>,
) -> Option<D55DataAuthority> {
    if payload.len() >= 16 {
        summary.status_flags_or |= read_u32(payload, 12);
    }
    let decoded = match decode_detail_events_payload(offset, payload) {
        Ok(decoded) => decoded,
        Err(mut decode_errors) => {
            errors.append(&mut decode_errors);
            return None;
        }
    };
    let total_row_count = decoded
        .rows
        .len()
        .saturating_add(decoded.side_detail_rows.len())
        .saturating_add(decoded.attribution_rows.len())
        .saturating_add(decoded.reserve_member_rows.len())
        .saturating_add(decoded.terminal_record_rows.len())
        .saturating_add(decoded.width_observation_rows.len())
        .saturating_add(decoded.current_loss_observation_rows.len())
        .saturating_add(decoded.panel_observation_rows.len());
    let authority = D55DataAuthority {
        elapsed_wall_ms: decoded.elapsed_wall_ms,
        status_flags: decoded.status_flags,
        row_count: u32::try_from(total_row_count).expect("validated D55 row ceilings fit u32"),
    };
    summary.row_count = summary.row_count.saturating_add(decoded.rows.len() as u64);
    summary.side_detail_rows = summary
        .side_detail_rows
        .saturating_add(decoded.side_detail_rows.len() as u64);
    summary.attribution_rows = summary
        .attribution_rows
        .saturating_add(decoded.attribution_rows.len() as u64);
    summary.reserve_member_rows = summary
        .reserve_member_rows
        .saturating_add(decoded.reserve_member_rows.len() as u64);
    summary.terminal_record_rows = summary
        .terminal_record_rows
        .saturating_add(decoded.terminal_record_rows.len() as u64);
    summary.width_observation_rows = summary
        .width_observation_rows
        .saturating_add(decoded.width_observation_rows.len() as u64);
    summary.current_loss_observation_rows = summary
        .current_loss_observation_rows
        .saturating_add(decoded.current_loss_observation_rows.len() as u64);
    summary.panel_observation_rows = summary
        .panel_observation_rows
        .saturating_add(decoded.panel_observation_rows.len() as u64);
    for row in decoded.rows {
        match row.event_kind {
            EVENT_PHASE_STARTED => summary.phase_started_rows += 1,
            EVENT_PHASE_UPDATED => summary.phase_updated_rows += 1,
            EVENT_PHASE_ENDED => summary.phase_ended_rows += 1,
            EVENT_PARTICIPANT_JOINED => summary.participant_joined_rows += 1,
            EVENT_PARTICIPANT_LEFT_UNCLASSIFIED => {
                summary.participant_left_unclassified_rows += 1;
                summary.unclassified_rows += 1;
            }
            _ => {}
        }
    }
    Some(authority)
}

#[derive(Debug)]
struct DecodedRow {
    combat_id: u32,
    generation: u64,
    event_kind: u32,
    side: u32,
    division_id: u32,
    flags: u32,
    location: u32,
    day: u32,
    duration: u32,
    old_side_count: u32,
    new_side_count: u32,
    reserved: u32,
}

fn validate_decoded_row(
    offset: usize,
    row_index: usize,
    row: &DecodedRow,
    seen: &mut BTreeSet<(u32, u64, u32, u32, u32)>,
    errors: &mut Vec<String>,
) {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid combat_id"
        ));
    }
    if row.generation == 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has zero generation"
        ));
    }
    if !matches!(
        row.event_kind,
        EVENT_PHASE_STARTED
            | EVENT_PHASE_UPDATED
            | EVENT_PHASE_ENDED
            | EVENT_PARTICIPANT_JOINED
            | EVENT_PARTICIPANT_LEFT_UNCLASSIFIED
    ) {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid event_kind {}",
            row.event_kind
        ));
    }
    if row.flags & !KNOWN_FLAG_MASK != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has unknown flags {}",
            row.flags
        ));
    }
    if row.flags & FLAG_FROM_VALIDATED_DOMAIN21_ROOT == 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} lacks validated-root provenance"
        ));
    }
    if row.side != SIDE_NONE && row.side != SIDE_ATTACKER && row.side != SIDE_DEFENDER {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid side {}",
            row.side
        ));
    }
    let side_present = row.flags & FLAG_SIDE_PRESENT != 0;
    if !side_present && row.side != SIDE_NONE {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has side without flag"
        ));
    }
    if side_present && !matches!(row.side, SIDE_ATTACKER | SIDE_DEFENDER) {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has side flag without a canonical side"
        ));
    }
    let division_present = row.flags & FLAG_DIVISION_PRESENT != 0;
    if !division_present && row.division_id != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has division without flag"
        ));
    }
    if division_present && (row.division_id == 0 || row.division_id > MAX_DIVISION_ID) {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid flagged division"
        ));
    }
    if row.flags & FLAG_LOCATION_PRESENT != 0
        && (row.location == 0 || row.location > MAX_LOCATION_ID)
    {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid location"
        ));
    }
    if row.flags & FLAG_LOCATION_PRESENT == 0 && row.location != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has location without flag"
        ));
    }
    if row.flags & FLAG_DAY_PRESENT != 0 && row.day > MAX_AGE_DAYS {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid day"
        ));
    }
    if row.flags & FLAG_DAY_PRESENT == 0 && row.day != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has day without flag"
        ));
    }
    if row.flags & FLAG_DURATION_PRESENT != 0 && row.duration > MAX_AGE_DAYS {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid duration"
        ));
    }
    if row.flags & FLAG_DURATION_PRESENT == 0 && row.duration != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has duration without flag"
        ));
    }
    if row.reserved != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} reserved field is nonzero"
        ));
    }
    if row.old_side_count > MAX_SIDE_UNITS as u32 || row.new_side_count > MAX_SIDE_UNITS as u32 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} side counts exceed max"
        ));
    }
    match row.event_kind {
        EVENT_PHASE_STARTED | EVENT_PHASE_UPDATED | EVENT_PHASE_ENDED => {
            if side_present || division_present || row.side != SIDE_NONE || row.division_id != 0 {
                errors.push(format!(
                    "offset {offset}: phase row {row_index} carries participant identity"
                ));
            }
            if row.flags & FLAG_SEMANTIC_UNCLASSIFIED != 0 {
                errors.push(format!(
                    "offset {offset}: phase row {row_index} carries participant classification"
                ));
            }
            if row.old_side_count != 0 {
                errors.push(format!(
                    "offset {offset}: phase row {row_index} has nonzero old_side_count"
                ));
            }
        }
        EVENT_PARTICIPANT_JOINED => {
            if !side_present || !division_present {
                errors.push(format!(
                    "offset {offset}: participant-joined row {row_index} lacks side/division identity"
                ));
            }
            if row.flags & FLAG_SEMANTIC_UNCLASSIFIED != 0 {
                errors.push(format!(
                    "offset {offset}: participant-joined row {row_index} is incorrectly unclassified"
                ));
            }
            if row.new_side_count <= row.old_side_count {
                errors.push(format!(
                    "offset {offset}: participant-joined row {row_index} has non-increasing side counts {} -> {}",
                    row.old_side_count, row.new_side_count
                ));
            }
        }
        EVENT_PARTICIPANT_LEFT_UNCLASSIFIED => {
            if !side_present || !division_present {
                errors.push(format!(
                    "offset {offset}: participant-left row {row_index} lacks side/division identity"
                ));
            }
            if row.flags & FLAG_SEMANTIC_UNCLASSIFIED == 0 {
                errors.push(format!(
                    "offset {offset}: participant-left row {row_index} lacks unclassified semantic badge"
                ));
            }
            if row.new_side_count >= row.old_side_count {
                errors.push(format!(
                    "offset {offset}: participant-left row {row_index} has non-decreasing side counts {} -> {}",
                    row.old_side_count, row.new_side_count
                ));
            }
        }
        _ => {}
    }
    if !seen.insert((
        row.combat_id,
        row.generation,
        row.event_kind,
        row.side,
        row.division_id,
    )) {
        errors.push(format!(
            "offset {offset}: detail row {row_index} duplicates event key"
        ));
    }
}

fn push_side_delta_rows(
    rows: &mut Vec<DetailEventRow>,
    old_units: &[u32],
    new_units: &[u32],
    now: &LandCombatSnapshot,
    side: CombatSide,
) {
    let old = old_units.iter().copied().collect::<BTreeSet<_>>();
    let new = new_units.iter().copied().collect::<BTreeSet<_>>();
    for division_id in new.difference(&old) {
        rows.push(participant_row(
            EVENT_PARTICIPANT_JOINED,
            now,
            side,
            *division_id,
            old.len() as u32,
            new.len() as u32,
        ));
    }
    for division_id in old.difference(&new) {
        rows.push(participant_row(
            EVENT_PARTICIPANT_LEFT_UNCLASSIFIED,
            now,
            side,
            *division_id,
            old.len() as u32,
            new.len() as u32,
        ));
    }
}

fn phase_changed(old: &LandCombatSnapshot, now: &LandCombatSnapshot) -> bool {
    old.location != now.location
        || old.day != now.day
        || old.duration != now.duration
        || old.attacker_divisions.len() != now.attacker_divisions.len()
        || old.defender_divisions.len() != now.defender_divisions.len()
}

fn phase_row(event_kind: u32, snapshot: &LandCombatSnapshot) -> DetailEventRow {
    let mut flags = FLAG_FROM_VALIDATED_DOMAIN21_ROOT;
    if snapshot.location.is_some() {
        flags |= FLAG_LOCATION_PRESENT;
    }
    if snapshot.day.is_some() {
        flags |= FLAG_DAY_PRESENT;
    }
    if snapshot.duration.is_some() {
        flags |= FLAG_DURATION_PRESENT;
    }
    DetailEventRow {
        combat_id: snapshot.combat_id,
        generation: snapshot.generation,
        event_kind,
        side: None,
        division_id: None,
        location: snapshot.location,
        day: snapshot.day,
        duration: snapshot.duration,
        old_side_count: 0,
        new_side_count: snapshot
            .attacker_divisions
            .len()
            .saturating_add(snapshot.defender_divisions.len())
            .min(u32::MAX as usize) as u32,
        flags,
    }
}

fn participant_row(
    event_kind: u32,
    snapshot: &LandCombatSnapshot,
    side: CombatSide,
    division_id: u32,
    old_side_count: u32,
    new_side_count: u32,
) -> DetailEventRow {
    let mut flags = FLAG_FROM_VALIDATED_DOMAIN21_ROOT | FLAG_SIDE_PRESENT | FLAG_DIVISION_PRESENT;
    if event_kind == EVENT_PARTICIPANT_LEFT_UNCLASSIFIED {
        flags |= FLAG_SEMANTIC_UNCLASSIFIED;
    }
    if snapshot.location.is_some() {
        flags |= FLAG_LOCATION_PRESENT;
    }
    if snapshot.day.is_some() {
        flags |= FLAG_DAY_PRESENT;
    }
    if snapshot.duration.is_some() {
        flags |= FLAG_DURATION_PRESENT;
    }
    DetailEventRow {
        combat_id: snapshot.combat_id,
        generation: snapshot.generation,
        event_kind,
        side: Some(side),
        division_id: Some(division_id),
        location: snapshot.location,
        day: snapshot.day,
        duration: snapshot.duration,
        old_side_count,
        new_side_count,
        flags,
    }
}

fn validate_snapshot(row: &LandCombatSnapshot) -> Result<(), String> {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        return Err(format!("invalid combat_id {}", row.combat_id));
    }
    if row.generation == 0 {
        return Err(format!("combat {} has zero generation", row.combat_id));
    }
    for (label, units) in [
        ("attacker", row.attacker_divisions.as_slice()),
        ("defender", row.defender_divisions.as_slice()),
    ] {
        if units.len() > MAX_SIDE_UNITS {
            return Err(format!("combat {} {label} units exceed max", row.combat_id));
        }
        let mut seen = BTreeSet::new();
        for unit in units {
            if *unit == 0 || *unit > MAX_DIVISION_ID {
                return Err(format!(
                    "combat {} {label} has invalid division id {unit}",
                    row.combat_id
                ));
            }
            if !seen.insert(*unit) {
                return Err(format!(
                    "combat {} {label} duplicates division id {unit}",
                    row.combat_id
                ));
            }
        }
    }
    let attacker_ids = row
        .attacker_divisions
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if let Some(division_id) = row
        .defender_divisions
        .iter()
        .find(|division_id| attacker_ids.contains(division_id))
    {
        return Err(format!(
            "combat {} assigns division {division_id} to both sides",
            row.combat_id
        ));
    }
    if row
        .location
        .is_some_and(|value| value == 0 || value > MAX_LOCATION_ID)
    {
        return Err(format!("combat {} invalid location", row.combat_id));
    }
    if row.day.is_some_and(|value| value > MAX_AGE_DAYS) {
        return Err(format!("combat {} invalid day", row.combat_id));
    }
    if row.duration.is_some_and(|value| value > MAX_AGE_DAYS) {
        return Err(format!("combat {} invalid duration", row.combat_id));
    }
    for (label, entries) in [
        ("attacker", row.attacker_attribution.as_slice()),
        ("defender", row.defender_attribution.as_slice()),
    ] {
        let mut seen = BTreeSet::new();
        for entry in entries {
            validate_attribution_entry(*entry)
                .map_err(|err| format!("combat {} {label} attribution {err}", row.combat_id))?;
            if !seen.insert((entry.dealer_tag_db_id, entry.victim_tag_db_id)) {
                return Err(format!(
                    "combat {} {label} duplicates attribution pair",
                    row.combat_id
                ));
            }
        }
    }
    Ok(())
}

fn validate_row(row: &DetailEventRow) -> Result<(), String> {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        return Err("invalid combat id".to_string());
    }
    if row.generation == 0 {
        return Err("invalid zero generation".to_string());
    }
    if let Some(division_id) = row.division_id {
        if division_id == 0 || division_id > MAX_DIVISION_ID {
            return Err("invalid division id".to_string());
        }
    }
    Ok(())
}

fn validate_attribution_entry(entry: AttributionEntry) -> Result<(), String> {
    if entry.dealer_tag_db_id == 0 || entry.dealer_tag_db_id > MAX_COUNTRY_TAG_DB_ID {
        return Err("has invalid dealer_tag_db_id".to_string());
    }
    if entry.victim_tag_db_id == 0 || entry.victim_tag_db_id > MAX_COUNTRY_TAG_DB_ID {
        return Err("has invalid victim_tag_db_id".to_string());
    }
    if entry.dealer_tag_db_id == entry.victim_tag_db_id {
        return Err("has identical dealer and victim".to_string());
    }
    // SDamageDone publishes a validated dealer/victim relationship before its
    // cumulative damage necessarily becomes nonzero. A stable zero is therefore
    // measured zero, not an unavailable row. This was observed directly on the
    // exact-a729 CStatsObserver path (runtime job
    // 20260724T141552Z-observe-runtime-72087eab).
    if entry.damage_raw > MAX_ATTRIBUTION_DAMAGE_RAW {
        return Err("has invalid damage_raw".to_string());
    }
    Ok(())
}

fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(payload);
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn read_i64(data: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn json_str(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization failed")
}

fn json_string_array(values: &[String]) -> String {
    let parts = values
        .iter()
        .map(|value| json_str(value))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perf_payload() -> Vec<u8> {
        perf_payload_with_status(STATUS_DELTA_DATA)
    }

    fn perf_payload_with_status(status_flags: u32) -> Vec<u8> {
        perf_payload_with_authority(1, status_flags, 0)
    }

    fn perf_payload_with_authority(
        elapsed_wall_ms: u64,
        status_flags: u32,
        row_count: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[0..8].copy_from_slice(&elapsed_wall_ms.to_le_bytes());
        for offset in [16usize, 20, 24, 28] {
            payload[offset..offset + 4].copy_from_slice(&row_count.to_le_bytes());
        }
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn payload_authority(payload: &[u8]) -> (u64, u32, u32) {
        assert!(payload.len() >= PAYLOAD_HEADER_LEN_V1);
        let version = read_u16(payload, 0);
        let event_rows = read_u32(payload, 16);
        let side_detail_rows = if version >= SCHEMA_VERSION_V2_SIDE_DETAIL {
            read_u32(payload, 24)
        } else {
            0
        };
        let attribution_rows =
            if version >= SCHEMA_VERSION_V3_ATTRIBUTION && payload.len() >= PAYLOAD_HEADER_LEN_V4 {
                read_u32(payload, 28)
            } else {
                0
            };
        let reserve_member_rows = if version >= SCHEMA_VERSION_V5_RESERVE_MEMBERS
            && payload.len() >= PAYLOAD_HEADER_LEN_V5
        {
            read_u32(payload, 32)
        } else {
            0
        };
        let terminal_record_rows = if version >= SCHEMA_VERSION_V6_TERMINAL_RECORDS
            && payload.len() >= PAYLOAD_HEADER_LEN_V6
        {
            read_u32(payload, 36)
        } else {
            0
        };
        let width_observation_rows =
            if version >= SCHEMA_VERSION_V7_WIDTH_LOSS && payload.len() >= PAYLOAD_HEADER_LEN {
                read_u32(payload, 40)
            } else {
                0
            };
        let current_loss_observation_rows =
            if version >= SCHEMA_VERSION_V7_WIDTH_LOSS && payload.len() >= PAYLOAD_HEADER_LEN {
                read_u32(payload, 44)
            } else {
                0
            };
        let panel_observation_rows =
            if version >= SCHEMA_VERSION && payload.len() >= PAYLOAD_HEADER_LEN {
                read_u32(payload, 48)
            } else {
                0
            };
        (
            read_u64(payload, 4),
            read_u32(payload, 12),
            event_rows
                .saturating_add(side_detail_rows)
                .saturating_add(attribution_rows)
                .saturating_add(reserve_member_rows)
                .saturating_add(terminal_record_rows)
                .saturating_add(width_observation_rows)
                .saturating_add(current_loss_observation_rows)
                .saturating_add(panel_observation_rows),
        )
    }

    fn perf_payload_with_diagnostic_unit() -> Vec<u8> {
        let name = b"read_us";
        let mut payload = perf_payload();
        payload.extend_from_slice(compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&17u64.to_le_bytes());
        payload.extend_from_slice(name);
        payload
    }

    fn append_frame_end(stream: &mut Vec<u8>, sequence: u64, hour: i32) {
        append_frame_end_with_status(stream, sequence, hour, 0, 0);
    }

    fn append_frame_end_with_status(
        stream: &mut Vec<u8>,
        sequence: u64,
        hour: i32,
        status_flags: u32,
        reserved: u32,
    ) {
        let mut payload = [0u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&1u32.to_le_bytes());
        payload[20..24].copy_from_slice(&1u32.to_le_bytes());
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload[28..32].copy_from_slice(&reserved.to_le_bytes());
        append_compact_record(
            stream,
            sequence,
            hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &payload,
        );
    }

    fn certify_lane_data_record(mut stream: Vec<u8>) -> Vec<u8> {
        let header = parse_compact_record_header(&stream).expect("lane DATA header");
        let perf_sequence = header.sequence.checked_add(1).expect("test sequence");
        let payload = &stream[COMPACT_RECORD_HEADER_LEN..];
        let (elapsed_wall_ms, data_status, row_count) = payload_authority(payload);
        append_compact_record(
            &mut stream,
            perf_sequence,
            header.game_hour_raw,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload_with_authority(elapsed_wall_ms, data_status, row_count),
        );
        append_frame_end(
            &mut stream,
            perf_sequence.checked_add(1).expect("test sequence"),
            header.game_hour_raw,
        );
        stream
    }

    fn canonical_empty_d55_payload() -> Vec<u8> {
        encode_detail_events_payload_v4(1, STATUS_DELTA_DATA, &[], &[], &[])
    }

    fn append_lane_hour(stream: &mut Vec<u8>, first_sequence: u64, hour: i32, payload: &[u8]) {
        let (elapsed_wall_ms, data_status, row_count) = payload_authority(payload);
        append_compact_record(
            stream,
            first_sequence,
            hour,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            payload,
        );
        append_compact_record(
            stream,
            first_sequence + 1,
            hour,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload_with_authority(elapsed_wall_ms, data_status, row_count),
        );
        append_frame_end(stream, first_sequence + 2, hour);
    }

    fn gate_d55_payload(payload: &[u8]) -> GateResult {
        let mut stream = Vec::new();
        append_lane_hour(&mut stream, 1, 100, payload);
        gate_stream(&stream)
    }

    fn participant_event(
        event_kind: u32,
        side: CombatSide,
        division_id: u32,
        old_side_count: u32,
        new_side_count: u32,
    ) -> DetailEventRow {
        let mut flags = FLAG_FROM_VALIDATED_DOMAIN21_ROOT
            | FLAG_SIDE_PRESENT
            | FLAG_DIVISION_PRESENT
            | FLAG_LOCATION_PRESENT
            | FLAG_DAY_PRESENT
            | FLAG_DURATION_PRESENT;
        if event_kind == EVENT_PARTICIPANT_LEFT_UNCLASSIFIED {
            flags |= FLAG_SEMANTIC_UNCLASSIFIED;
        }
        DetailEventRow {
            combat_id: 42,
            generation: 7,
            event_kind,
            side: Some(side),
            division_id: Some(division_id),
            location: Some(1000),
            day: Some(3),
            duration: Some(3),
            old_side_count,
            new_side_count,
            flags,
        }
    }
    fn gate_sequences(sequences: &[u64]) -> GateResult {
        let mut stream = Vec::new();
        let payload = perf_payload();
        for &sequence in sequences {
            append_compact_record(
                &mut stream,
                sequence,
                100,
                DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &payload,
            );
        }
        gate_stream(&stream)
    }

    #[test]
    fn compact_gate_rejects_missing_sequence_prefix() {
        let gate = gate_sequences(&[2]);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("sequence 2 does not start at 1")));
    }

    #[test]
    fn compact_gate_rejects_sequence_gap() {
        let gate = gate_sequences(&[1, 3]);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 3 (expected 2)")));
    }

    #[test]
    fn compact_gate_rejects_repeated_sequence() {
        let gate = gate_sequences(&[1, 1]);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 1 (expected 2)")));
    }

    #[test]
    fn compact_gate_rejects_sequence_regression() {
        let gate = gate_sequences(&[1, 2, 1]);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 2 -> 1 (expected 3)")));
    }

    #[test]
    fn compact_gate_rejects_sequence_wrap_after_max() {
        let gate = gate_sequences(&[u64::MAX, 0]);

        assert!(!gate.ok);
        assert!(gate.errors.iter().any(|error| {
            error.contains("sequence space exhausted after 18446744073709551615")
                && error.contains("wrapping is not allowed")
        }));
    }

    #[test]
    fn compact_gate_accepts_canonical_perf_base_and_announced_pdu1_trailer_semantics() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(),
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_diagnostic_unit(),
        );

        let gate = gate_stream(&stream);
        assert!(gate
            .errors
            .iter()
            .all(|error| !error.contains("DOMAIN_PERF payload")));
        assert!(!gate.ok, "PERF-only stream must not certify");
    }

    #[test]
    fn compact_gate_rejects_missing_announced_perf_trailer() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload(),
        );

        let gate = gate_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("diagnostic trailer header is truncated")));
    }

    #[test]
    fn compact_gate_rejects_unannounced_perf_trailer() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload_with_diagnostic_unit(),
        );

        let gate = gate_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("unannounced trailer byte(s)")));
    }

    #[test]
    fn compact_gate_rejects_malformed_perf_trailer() {
        let mut payload = perf_payload_with_diagnostic_unit();
        payload[compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );

        let gate = gate_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("diagnostic trailer magic is invalid")));
    }

    #[test]
    fn record_flag_policy_accepts_canonical_schema_and_perf_flags() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            45,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_diagnostic_unit(),
        );

        let gate = gate_stream(&stream);
        assert!(gate
            .errors
            .iter()
            .all(|error| !error.contains("record flags")));
    }

    #[test]
    fn record_flag_policy_rejects_wrong_placement_and_unknown_bits() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            0,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            1 << 29,
            &[],
        );

        let gate = gate_stream(&stream);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("record flags 0x80000000")));
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("record flags 0x20000000")));
    }

    #[test]
    fn compact_gate_accepts_one_data_and_perf_in_a_canonical_closed_hour() {
        let gate = gate_d55_payload(&canonical_empty_d55_payload());
        assert!(gate.ok, "{:?}", gate.errors);
    }

    #[test]
    fn product_gate_requires_explicit_positive_duration_and_inclusive_closed_frames() {
        let payload = canonical_empty_d55_payload();
        let mut stream = Vec::new();
        append_lane_hour(&mut stream, 1, 100, &payload);
        append_lane_hour(&mut stream, 4, 101, &payload);

        let exact = gate_stream_for_product(&stream, 1);
        assert!(exact.ok, "{:?}", exact.errors);

        let too_long = gate_stream_for_product(&stream, 2);
        assert!(!too_long.ok);
        assert!(too_long.errors.iter().any(|error| {
            error.contains("1 advancement(s) across 2 closed frame(s)")
                && error.contains("required 2 advancement(s) / 3 closed frame(s)")
        }));

        let non_positive = gate_stream_for_product(&stream, 0);
        assert!(!non_positive.ok);
        assert!(non_positive
            .errors
            .iter()
            .any(|error| error.contains("must be explicitly positive")));
    }

    #[test]
    fn compact_gate_rejects_empty_and_unrelated_only_streams() {
        let empty = gate_stream(&[]);
        assert!(!empty.ok);
        assert!(empty
            .errors
            .iter()
            .any(|error| error.contains("certifies no closed hour")));

        let mut unrelated = Vec::new();
        append_compact_record(
            &mut unrelated,
            1,
            100,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            b"opaque-unowned-payload",
        );
        append_compact_record(
            &mut unrelated,
            2,
            100,
            18,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(),
        );
        append_frame_end(&mut unrelated, 3, 100);
        let gate = gate_stream(&unrelated);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("DATA/PERF counts 0/0")));
        assert!(gate
            .errors
            .iter()
            .all(|error| !error.contains("opaque-unowned")));
    }

    #[test]
    fn compact_gate_rejects_missing_or_duplicate_d55_records() {
        let payload = canonical_empty_d55_payload();

        let mut missing_perf = Vec::new();
        append_compact_record(
            &mut missing_perf,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_frame_end(&mut missing_perf, 2, 100);
        let gate = gate_stream(&missing_perf);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("DATA/PERF counts 1/0")));

        let mut missing_data = Vec::new();
        append_compact_record(
            &mut missing_data,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(),
        );
        append_compact_record(
            &mut missing_data,
            2,
            100,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            b"opaque",
        );
        append_frame_end(&mut missing_data, 3, 100);
        let gate = gate_stream(&missing_data);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("DATA/PERF counts 0/1")));

        let mut duplicate_data = Vec::new();
        append_compact_record(
            &mut duplicate_data,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_compact_record(
            &mut duplicate_data,
            2,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_compact_record(
            &mut duplicate_data,
            3,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(),
        );
        append_frame_end(&mut duplicate_data, 4, 100);
        let gate = gate_stream(&duplicate_data);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("duplicate domain-55 DATA")));

        let mut duplicate_perf = Vec::new();
        append_compact_record(
            &mut duplicate_perf,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        for sequence in [2, 3] {
            append_compact_record(
                &mut duplicate_perf,
                sequence,
                100,
                DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &perf_payload(),
            );
        }
        append_frame_end(&mut duplicate_perf, 4, 100);
        let gate = gate_stream(&duplicate_perf);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("duplicate domain-55 PERF")));
    }

    #[test]
    fn compact_gate_rejects_bare_skipped_reordered_and_trailing_boundaries() {
        let mut bare = Vec::new();
        append_frame_end(&mut bare, 1, 100);
        assert!(gate_stream(&bare)
            .errors
            .iter()
            .any(|error| error.contains("bare FRAME_END")));

        let payload = canonical_empty_d55_payload();
        let mut skipped = Vec::new();
        append_lane_hour(&mut skipped, 1, 100, &payload);
        append_lane_hour(&mut skipped, 4, 102, &payload);
        assert!(gate_stream(&skipped)
            .errors
            .iter()
            .any(|error| error.contains("non-consecutive DATA hour 102")));

        let mut reordered = Vec::new();
        append_compact_record(
            &mut reordered,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_compact_record(
            &mut reordered,
            2,
            101,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            b"opaque",
        );
        assert!(gate_stream(&reordered)
            .errors
            .iter()
            .any(|error| error.contains("interleaves open hour 100")));

        let mut trailing = Vec::new();
        append_lane_hour(&mut trailing, 1, 100, &payload);
        append_compact_record(
            &mut trailing,
            4,
            101,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(),
        );
        let gate = gate_stream(&trailing);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("PERF-only hour 101")));
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("final record is not canonical FRAME_END")));
    }

    #[test]
    fn compact_gate_rejects_noncanonical_and_fatal_d55_status() {
        let unknown =
            encode_detail_events_payload_v4(1, STATUS_DELTA_DATA | (1 << 31), &[], &[], &[]);
        assert!(gate_d55_payload(&unknown)
            .errors
            .iter()
            .any(|error| error.contains("unknown bits")));

        let fatal = encode_detail_events_payload_v4(
            1,
            STATUS_DELTA_DATA | STATUS_SOURCE_UNAVAILABLE,
            &[],
            &[],
            &[],
        );
        assert!(gate_d55_payload(&fatal)
            .errors
            .iter()
            .any(|error| error.contains("failure flags")));

        let payload = canonical_empty_d55_payload();
        let mut perf_fatal = Vec::new();
        append_compact_record(
            &mut perf_fatal,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_compact_record(
            &mut perf_fatal,
            2,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload_with_status(STATUS_SOURCE_UNAVAILABLE),
        );
        append_frame_end(&mut perf_fatal, 3, 100);
        assert!(gate_stream(&perf_fatal)
            .errors
            .iter()
            .any(|error| error.contains("failure flags")));

        let mut frame_fatal = Vec::new();
        append_compact_record(
            &mut frame_fatal,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_compact_record(
            &mut frame_fatal,
            2,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(),
        );
        append_frame_end_with_status(&mut frame_fatal, 3, 100, STATUS_SOURCE_UNAVAILABLE, 0);
        assert!(gate_stream(&frame_fatal)
            .errors
            .iter()
            .any(|error| error.contains("failure flags")));

        let mut frame_reserved = frame_fatal;
        let frame_offset = frame_reserved.len() - compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN;
        frame_reserved[frame_offset + 24..frame_offset + 28].copy_from_slice(&0u32.to_le_bytes());
        frame_reserved[frame_offset + 28..frame_offset + 32].copy_from_slice(&1u32.to_le_bytes());
        assert!(gate_stream(&frame_reserved)
            .errors
            .iter()
            .any(|error| error.contains("reserved payload word")));

        let mut zero_authority = Vec::new();
        append_compact_record(
            &mut zero_authority,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_compact_record(
            &mut zero_authority,
            2,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(),
        );
        let frame_payload = [0u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN];
        append_compact_record(
            &mut zero_authority,
            3,
            100,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_payload,
        );
        assert!(gate_stream(&zero_authority)
            .errors
            .iter()
            .any(|error| error.contains("zero emitted/scheduled domain authority")));

        let mut mismatched_authority = frame_payload;
        mismatched_authority[16..20].copy_from_slice(&1u32.to_le_bytes());
        mismatched_authority[20..24].copy_from_slice(&2u32.to_le_bytes());
        let frame_start = zero_authority.len() - compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN;
        zero_authority[frame_start..].copy_from_slice(&mismatched_authority);
        assert!(gate_stream(&zero_authority)
            .errors
            .iter()
            .any(|error| error.contains("does not match scheduled domain count")));

        let mismatched_perf = perf_payload_with_status(STATUS_PARTIAL_DATA);
        let mut status_mismatch = Vec::new();
        append_compact_record(
            &mut status_mismatch,
            1,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_compact_record(
            &mut status_mismatch,
            2,
            100,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &mismatched_perf,
        );
        append_frame_end(&mut status_mismatch, 3, 100);
        assert!(gate_stream(&status_mismatch)
            .errors
            .iter()
            .any(|error| error.contains("DATA status") && error.contains("does not match PERF")));
    }

    #[test]
    fn compact_gate_binds_perf_timestamp_and_all_four_counts_to_decoded_data() {
        let payload = canonical_empty_d55_payload();
        let mut canonical = Vec::new();
        append_lane_hour(&mut canonical, 1, 100, &payload);
        assert!(gate_stream(&canonical).ok);

        let data_header = parse_compact_record_header(&canonical).expect("DATA header");
        let perf_offset = COMPACT_RECORD_HEADER_LEN + data_header.payload_len;
        let perf_payload = perf_offset + COMPACT_RECORD_HEADER_LEN;

        let mut elapsed_mismatch = canonical.clone();
        elapsed_mismatch[perf_payload..perf_payload + 8].copy_from_slice(&2u64.to_le_bytes());
        assert!(gate_stream(&elapsed_mismatch)
            .errors
            .iter()
            .any(|error| error.contains("elapsed_wall_ms")));

        for (offset, label) in [
            (16usize, "row_count"),
            (20, "changed_rows"),
            (24, "unit_count"),
            (28, "changed_unit_count"),
        ] {
            let mut mismatch = canonical.clone();
            mismatch[perf_payload + offset..perf_payload + offset + 4]
                .copy_from_slice(&1u32.to_le_bytes());
            let gate = gate_stream(&mismatch);
            assert!(!gate.ok);
            assert!(gate
                .errors
                .iter()
                .any(|error| error.contains(label) && error.contains("decoded DATA row count")));
        }
    }

    #[test]
    fn decoder_rejects_event_row_count_above_production_topology_before_allocation() {
        let mut payload = canonical_empty_d55_payload();
        payload[16..20].copy_from_slice(&((MAX_DETAIL_EVENT_ROWS as u32) + 1).to_le_bytes());
        let errors = decode_detail_events_payload(0, &payload).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("detail row_count") && error.contains("exceeds max")));
    }

    #[test]
    fn participant_delta_sets_must_be_complete_and_referentially_consistent() {
        let partial_join =
            participant_event(EVENT_PARTICIPANT_JOINED, CombatSide::Attacker, 1002, 1, 3);
        let payload =
            encode_detail_events_payload_v4(1, STATUS_DELTA_DATA, &[partial_join], &[], &[]);
        assert!(gate_d55_payload(&payload)
            .errors
            .iter()
            .any(|error| error.contains("participant delta") && error.contains("partial")));

        let joined = participant_event(EVENT_PARTICIPANT_JOINED, CombatSide::Attacker, 1002, 1, 1);
        let left = participant_event(
            EVENT_PARTICIPANT_LEFT_UNCLASSIFIED,
            CombatSide::Attacker,
            1002,
            1,
            1,
        );
        let payload =
            encode_detail_events_payload_v4(1, STATUS_DELTA_DATA, &[joined, left], &[], &[]);
        assert!(gate_d55_payload(&payload)
            .errors
            .iter()
            .any(|error| error.contains("multiple participant events")));

        let mut missing_identity =
            participant_event(EVENT_PARTICIPANT_JOINED, CombatSide::Defender, 2002, 1, 2);
        missing_identity.flags &= !FLAG_DIVISION_PRESENT;
        let payload =
            encode_detail_events_payload_v4(1, STATUS_DELTA_DATA, &[missing_identity], &[], &[]);
        assert!(gate_d55_payload(&payload).errors.iter().any(|error| {
            error.contains("division without flag")
                || error.contains("lacks side/division identity")
        }));

        let mut missing_side =
            participant_event(EVENT_PARTICIPANT_JOINED, CombatSide::Defender, 2002, 1, 2);
        missing_side.flags &= !FLAG_SIDE_PRESENT;
        let payload =
            encode_detail_events_payload_v4(1, STATUS_DELTA_DATA, &[missing_side], &[], &[]);
        assert!(gate_d55_payload(&payload).errors.iter().any(|error| {
            error.contains("side without flag") || error.contains("lacks side/division identity")
        }));

        let wrong_join_direction =
            participant_event(EVENT_PARTICIPANT_JOINED, CombatSide::Attacker, 1002, 2, 1);
        let payload = encode_detail_events_payload_v4(
            1,
            STATUS_DELTA_DATA,
            &[wrong_join_direction],
            &[],
            &[],
        );
        assert!(gate_d55_payload(&payload)
            .errors
            .iter()
            .any(|error| error.contains("participant-joined") && error.contains("non-increasing")));

        let wrong_leave_direction = participant_event(
            EVENT_PARTICIPANT_LEFT_UNCLASSIFIED,
            CombatSide::Defender,
            2002,
            1,
            2,
        );
        let payload = encode_detail_events_payload_v4(
            1,
            STATUS_DELTA_DATA,
            &[wrong_leave_direction],
            &[],
            &[],
        );
        assert!(gate_d55_payload(&payload)
            .errors
            .iter()
            .any(|error| error.contains("participant-left") && error.contains("non-decreasing")));
    }

    struct TestMemory {
        snapshots: Vec<LandCombatSnapshot>,
    }

    impl LandCombatDetailMemory for TestMemory {
        fn land_combat_snapshots(&self) -> Result<Vec<LandCombatSnapshot>, Vec<String>> {
            Ok(self.snapshots.clone())
        }
    }

    #[test]
    fn transition_emits_phase_and_participant_events() {
        let old = [snapshot(42, 7, 1000, &[1001, 1002], &[2001])];
        let new = [snapshot(42, 7, 1000, &[1001], &[2001, 2002])];
        let rows = build_detail_event_rows(&old, &new).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|row| row.event_kind == EVENT_PHASE_UPDATED));
        assert!(rows
            .iter()
            .any(|row| row.event_kind == EVENT_PARTICIPANT_JOINED));
        assert!(rows
            .iter()
            .any(|row| row.event_kind == EVENT_PARTICIPANT_LEFT_UNCLASSIFIED));
    }

    #[test]
    fn compact_gate_accepts_transition_stream() {
        let start = [snapshot(42, 7, 1000, &[1001, 1002], &[2001])];
        let update = [snapshot(42, 7, 1000, &[1001], &[2001, 2002])];
        let frame1 = build_transition_capture_frame(&[], &start, 1, 60_787_539, 1).unwrap();
        let frame2 = build_transition_capture_frame(&start, &update, 4, 60_787_540, 2).unwrap();
        let frame3 = build_transition_capture_frame(&update, &[], 7, 60_787_541, 3).unwrap();
        let mut stream = certify_lane_data_record(frame1.compact_stream);
        stream.extend_from_slice(&certify_lane_data_record(frame2.compact_stream));
        stream.extend_from_slice(&certify_lane_data_record(frame3.compact_stream));
        let gate = gate_stream(&stream);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.phase_started_rows, 1);
        assert_eq!(gate.summary.phase_updated_rows, 1);
        assert_eq!(gate.summary.phase_ended_rows, 1);
        assert_eq!(gate.summary.participant_joined_rows, 1);
        assert_eq!(gate.summary.participant_left_unclassified_rows, 1);
    }

    #[test]
    fn reset_transition_baseline_rebaselines_without_synthetic_events() {
        let mut tracker = LandCombatDetailTracker::default();
        let first = TestMemory {
            snapshots: vec![snapshot(42, 7, 1000, &[1001], &[2001])],
        };
        let changed_after_gap = TestMemory {
            snapshots: vec![snapshot(42, 7, 1000, &[1001, 1002], &[2001, 2002])],
        };

        let first_frame =
            collect_transition_capture_frame(&first, &mut tracker, 1, 60_787_539, 1).unwrap();
        assert!(!first_frame.rows.is_empty());

        tracker.reset_transition_baseline();
        let reset_frame =
            collect_transition_capture_frame(&changed_after_gap, &mut tracker, 2, 60_787_541, 2)
                .unwrap();
        assert!(reset_frame.rows.is_empty());
        assert!(tracker.last_frame_rebaselined());
        let payload = &reset_frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let decoded = decode_detail_events_payload(COMPACT_RECORD_HEADER_LEN, payload).unwrap();
        assert_eq!(decoded.rows.len(), 0);
        assert_ne!(decoded.status_flags & STATUS_TRANSITION_BASELINE_RESET, 0);
        assert_ne!(decoded.status_flags & STATUS_PARTIAL_DATA, 0);
    }

    fn snapshot(
        combat_id: u32,
        generation: u64,
        location: u32,
        attackers: &[u32],
        defenders: &[u32],
    ) -> LandCombatSnapshot {
        LandCombatSnapshot {
            combat_id,
            generation,
            location: Some(location),
            day: Some(3),
            duration: Some(3),
            attacker_divisions: attackers.to_vec(),
            defender_divisions: defenders.to_vec(),
            attacker_detail: None,
            defender_detail: None,
            attacker_attribution: Vec::new(),
            defender_attribution: Vec::new(),
            attacker_reserve_divisions: None,
            defender_reserve_divisions: None,
        }
    }

    fn sample_side_detail(committed: u32, reserve: u32) -> SideDetail {
        SideDetail {
            units: committed + reserve,
            committed,
            reserve,
            air_kills: 8,
            org_loss_summary_index: 0,
            num_org_losses: 192,
            anti_air_attack_raw: 434_710,
            air_damage_str_raw: 8_961_046,
            air_damage_org_raw: 18_464_611,
            ground_damage_str_raw: 11_248_500,
            ground_damage_org_raw: 15_443_824,
            prevented_damage_str_raw: 111_654,
            prevented_damage_org_raw: 245_433,
            progress_raw: Some(53_000),
            tactic: Some(TacticIdentity {
                id: 7,
                token: "tactic_basic_attack".into(),
                name: "TACTIC_BASIC_ATTACK".into(),
            }),
            tactic_known: true,
            counts_incomplete: false,
        }
    }

    fn snapshot_with_detail(
        combat_id: u32,
        generation: u64,
        attackers: &[u32],
        defenders: &[u32],
        attacker_detail: Option<SideDetail>,
        defender_detail: Option<SideDetail>,
    ) -> LandCombatSnapshot {
        LandCombatSnapshot {
            combat_id,
            generation,
            location: Some(3236),
            day: Some(192),
            duration: Some(192),
            attacker_divisions: attackers.to_vec(),
            defender_divisions: defenders.to_vec(),
            attacker_detail,
            defender_detail,
            attacker_attribution: Vec::new(),
            defender_attribution: Vec::new(),
            attacker_reserve_divisions: None,
            defender_reserve_divisions: None,
        }
    }

    #[test]
    fn side_detail_rows_roundtrip_through_v5_payload() {
        let att = sample_side_detail(2, 2);
        let def = sample_side_detail(2, 0);
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001, 8002, 8003, 8004],
            &[9001, 9002],
            Some(att.clone()),
            Some(def.clone()),
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        assert_eq!(frame.side_detail_rows.len(), 2);
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let decoded = decode_detail_events_payload(COMPACT_RECORD_HEADER_LEN, payload).unwrap();
        assert_eq!(decoded.side_detail_rows.len(), 2);
        let attacker = decoded
            .side_detail_rows
            .iter()
            .find(|row| row.side == CombatSide::Attacker)
            .unwrap();
        assert_eq!(attacker.combat_id, 21381);
        assert_eq!(attacker.detail, att);
        assert_eq!(attacker.detail.units, 4);
        assert_eq!(attacker.detail.committed, 2);
        assert_eq!(attacker.detail.reserve, 2);
        let defender = decoded
            .side_detail_rows
            .iter()
            .find(|row| row.side == CombatSide::Defender)
            .unwrap();
        assert_eq!(defender.detail, def);

        // Gate accepts the v4 stream and counts exact side-detail rows.
        let gate = gate_stream(&certify_lane_data_record(frame.compact_stream));
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.side_detail_rows, 2);
    }

    #[test]
    fn attribution_rows_roundtrip_through_v5_payload() {
        let mut current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001, 8002],
            &[9001],
            Some(sample_side_detail(2, 0)),
            Some(sample_side_detail(1, 0)),
        )];
        current[0].attacker_attribution = vec![AttributionEntry {
            dealer_tag_db_id: 3,
            victim_tag_db_id: 1,
            damage_raw: 1_421_400,
        }];
        current[0].defender_attribution = vec![AttributionEntry {
            dealer_tag_db_id: 1,
            victim_tag_db_id: 3,
            damage_raw: 9_571_110,
        }];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        assert_eq!(frame.attribution_rows.len(), 2);
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        assert_eq!(read_u16(payload, 0), SCHEMA_VERSION);
        assert_eq!(read_u32(payload, 28), 2);
        let decoded = decode_detail_events_payload(COMPACT_RECORD_HEADER_LEN, payload).unwrap();
        assert_eq!(decoded.attribution_rows, frame.attribution_rows);
        let gate = gate_stream(&certify_lane_data_record(frame.compact_stream));
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.attribution_rows, 2);
    }

    #[test]
    fn zero_damage_attribution_is_a_measured_relationship() {
        let mut current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001, 8002],
            &[9001],
            Some(sample_side_detail(2, 0)),
            Some(sample_side_detail(1, 0)),
        )];
        current[0].attacker_attribution = vec![AttributionEntry {
            dealer_tag_db_id: 3,
            victim_tag_db_id: 1,
            damage_raw: 0,
        }];

        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        assert_eq!(frame.attribution_rows.len(), 1);
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let decoded = decode_detail_events_payload(COMPACT_RECORD_HEADER_LEN, payload).unwrap();
        assert_eq!(decoded.attribution_rows, frame.attribution_rows);
        let gate = gate_stream(&certify_lane_data_record(frame.compact_stream));
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.attribution_rows, 1);
    }

    #[test]
    fn reserve_member_rows_roundtrip_with_known_empty_distinguished_from_unknown() {
        let mut current = vec![snapshot_with_detail(
            21_381,
            7,
            &[8001, 8002, 8003, 8004],
            &[9001],
            Some(sample_side_detail(2, 2)),
            Some(sample_side_detail(1, 0)),
        )];
        current[0].attacker_reserve_divisions = Some(vec![8003, 8004]);
        current[0].defender_reserve_divisions = Some(Vec::new());
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        assert_eq!(frame.reserve_member_rows.len(), 2);
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        assert_eq!(read_u16(payload, 0), SCHEMA_VERSION);
        assert_eq!(read_u32(payload, 32), 2);
        let decoded = decode_detail_events_payload(COMPACT_RECORD_HEADER_LEN, payload).unwrap();
        assert_eq!(decoded.reserve_member_rows, frame.reserve_member_rows);
        assert!(decoded
            .side_detail_rows
            .iter()
            .all(|row| row.reserve_members_known));
        let gate = gate_stream(&certify_lane_data_record(frame.compact_stream));
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.reserve_member_rows, 2);

        let mut incomplete = current;
        incomplete[0].attacker_reserve_divisions = Some(vec![8003]);
        let frame = build_transition_capture_frame(&[], &incomplete, 1, 60_787_539, 1).unwrap();
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        assert!(decode_detail_events_payload(0, payload)
            .unwrap_err()
            .iter()
            .any(|error| error.contains("reserve member count 1 disagrees")));
    }

    #[test]
    fn terminal_record_v6_roundtrip_is_ended_keyed_and_discriminator_gated() {
        let ended = DetailEventRow {
            combat_id: 21_381,
            generation: 7,
            event_kind: EVENT_PHASE_ENDED,
            side: None,
            division_id: None,
            location: Some(3_236),
            day: Some(192),
            duration: Some(192),
            old_side_count: 0,
            new_side_count: 0,
            flags: FLAG_LOCATION_PRESENT
                | FLAG_DAY_PRESENT
                | FLAG_DURATION_PRESENT
                | FLAG_FROM_VALIDATED_DOMAIN21_ROOT,
        };
        let terminal = TerminalCombatRecordRow {
            combat_id: 21_381,
            generation: 7,
            record_index: 14,
            writer_kind: TERMINAL_WRITER_KIND_NORMAL,
            attacker_manpower_loss_raw: 1_200,
            defender_manpower_loss_raw: 3_400,
            end_date_raw: 60_787_539,
            location: 3_236,
            winner_side_raw: 1,
            winter_raw: 0,
            result_selector_raw: 1,
            layout_side_raw: 0,
        };
        let mut payload = encode_detail_events_payload_v6(
            5,
            STATUS_DELTA_DATA,
            &[ended],
            &[],
            &[],
            &[],
            &[terminal],
        );
        let decoded = decode_detail_events_payload(0, &payload).unwrap();
        assert_eq!(decoded.terminal_record_rows, vec![terminal]);

        let terminal_start = PAYLOAD_HEADER_LEN_V6 + DETAIL_EVENT_ROW_LEN;
        payload[terminal_start + 46] = 0;
        assert!(decode_detail_events_payload(0, &payload).is_err());
    }

    #[test]
    fn x2_v8_observation_rows_are_pointer_free_grouped_and_exact_sum_gated() {
        let side = SideDetailRow {
            combat_id: 21_381,
            generation: 7,
            side: CombatSide::Attacker,
            detail: sample_side_detail(2, 0),
            reserve_members_known: false,
        };
        let widths = [
            WidthObservationRow {
                combat_id: 21_381,
                generation: 7,
                side: CombatSide::Attacker,
                capture_sequence: 44,
                ordinal: 0,
                wrapper_count: 2,
                division_id: 8001,
                control_flags: WIDTH_REQUIRED_FLAG_MASK,
                wrapper_width_raw: 2_000_000,
                final_width_raw: 4_000_000,
            },
            WidthObservationRow {
                ordinal: 1,
                division_id: 8002,
                ..WidthObservationRow {
                    combat_id: 21_381,
                    generation: 7,
                    side: CombatSide::Attacker,
                    capture_sequence: 44,
                    ordinal: 0,
                    wrapper_count: 2,
                    division_id: 8001,
                    control_flags: WIDTH_REQUIRED_FLAG_MASK,
                    wrapper_width_raw: 2_000_000,
                    final_width_raw: 4_000_000,
                }
            },
        ];
        let losses = [
            CurrentLossObservationRow {
                combat_id: 21_381,
                generation: 7,
                side: CombatSide::Attacker,
                capture_sequence: 45,
                ordinal: 0,
                equipment_count: 2,
                equipment_id: 70,
                control_flags: CURRENT_LOSS_REQUIRED_FLAG_MASK
                    | CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE,
                manpower_before_raw: 100,
                manpower_after_raw: 90,
                equipment_before_raw: 50,
                equipment_after_raw: 48,
            },
            CurrentLossObservationRow {
                ordinal: 1,
                equipment_id: 71,
                equipment_before_raw: 40,
                equipment_after_raw: 39,
                ..CurrentLossObservationRow {
                    combat_id: 21_381,
                    generation: 7,
                    side: CombatSide::Attacker,
                    capture_sequence: 45,
                    ordinal: 0,
                    equipment_count: 2,
                    equipment_id: 70,
                    control_flags: CURRENT_LOSS_REQUIRED_FLAG_MASK
                        | CURRENT_LOSS_FLAG_SOURCE_ACTIVE_DAMAGE,
                    manpower_before_raw: 100,
                    manpower_after_raw: 90,
                    equipment_before_raw: 50,
                    equipment_after_raw: 48,
                }
            },
        ];
        let panel = PanelObservationRow {
            combat_id: 21_381,
            generation: 7,
            side: CombatSide::Attacker,
            capture_sequence: 46,
            leader_id: 912,
            modifier_bits: (1 << 8) | (1 << 2) | (1 << 5),
            control_flags: PANEL_REQUIRED_FLAG_MASK,
        };
        let payload = encode_detail_events_payload_v8(
            5,
            STATUS_DELTA_DATA,
            &[],
            std::slice::from_ref(&side),
            &[],
            &[],
            &[],
            &widths,
            &losses,
            &[panel],
        );
        let decoded = decode_detail_events_payload(0, &payload).unwrap();
        assert_eq!(decoded.width_observation_rows, widths.to_vec());
        assert_eq!(decoded.current_loss_observation_rows, losses.to_vec());
        assert_eq!(decoded.panel_observation_rows, vec![panel]);

        let mut corrupt = payload.clone();
        let width_start = PAYLOAD_HEADER_LEN + SIDE_DETAIL_ROW_LEN;
        corrupt[width_start + 40..width_start + 48].copy_from_slice(&1_999_999i64.to_le_bytes());
        assert!(decode_detail_events_payload(0, &corrupt)
            .unwrap_err()
            .iter()
            .any(|error| error.contains("exact-sum")));

        let loss_start = width_start + widths.len() * WIDTH_OBSERVATION_ROW_LEN;
        let mut source_less = payload.clone();
        source_less[loss_start + 36..loss_start + 40]
            .copy_from_slice(&CURRENT_LOSS_REQUIRED_FLAG_MASK.to_le_bytes());
        assert!(decode_detail_events_payload(0, &source_less)
            .unwrap_err()
            .iter()
            .any(|error| error.contains("identity/control/delta")));

        let mut mixed_source = payload;
        mixed_source[loss_start + CURRENT_LOSS_OBSERVATION_ROW_LEN + 36
            ..loss_start + CURRENT_LOSS_OBSERVATION_ROW_LEN + 40]
            .copy_from_slice(
                &(CURRENT_LOSS_REQUIRED_FLAG_MASK | CURRENT_LOSS_FLAG_SOURCE_PARTICIPANT_LIFECYCLE)
                    .to_le_bytes(),
            );
        assert!(decode_detail_events_payload(0, &mixed_source)
            .unwrap_err()
            .iter()
            .any(|error| error.contains("complete/order-unique/consistent")));

        let panel_start = loss_start + losses.len() * CURRENT_LOSS_OBSERVATION_ROW_LEN;
        let mut bad_panel = encode_detail_events_payload_v8(
            5,
            STATUS_DELTA_DATA,
            &[],
            std::slice::from_ref(&side),
            &[],
            &[],
            &[],
            &widths,
            &losses,
            &[panel],
        );
        bad_panel[panel_start + 32..panel_start + 36].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_detail_events_payload(0, &bad_panel)
            .unwrap_err()
            .iter()
            .any(|error| error.contains("panel row")));
    }

    #[test]
    fn v3_payload_decodes_with_explicit_legacy_tactic_progress_authority() {
        let detail = sample_side_detail(2, 0);
        let rows = vec![SideDetailRow {
            combat_id: 21_381,
            generation: 7,
            side: CombatSide::Attacker,
            detail: detail.clone(),
            reserve_members_known: false,
        }];
        let payload = encode_detail_events_payload_v3(1, STATUS_DELTA_DATA, &[], &rows, &[]);
        assert_eq!(read_u16(&payload, 0), SCHEMA_VERSION_V3_ATTRIBUTION);
        let decoded = decode_detail_events_payload(0, &payload).unwrap();
        let legacy = &decoded.side_detail_rows[0].detail;
        assert_eq!(legacy.ground_damage_str_raw, detail.ground_damage_str_raw);
        assert_eq!(legacy.progress_raw, None);
        assert_eq!(legacy.tactic, None);
        assert!(!legacy.tactic_known);
    }

    #[test]
    fn side_missing_detail_emits_no_row_fail_closed() {
        // A side whose detail failed revalidation (None) must contribute NO
        // row — absent, never zero-filled.
        let current = vec![snapshot_with_detail(
            21453,
            7,
            &[8001],
            &[9001],
            Some(sample_side_detail(1, 0)),
            None,
        )];
        let rows = side_detail_rows_from_snapshots(&current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].side, CombatSide::Attacker);
    }

    #[test]
    fn v1_payload_still_decodes_with_no_side_detail() {
        // The legacy v1 encoder path (event rows only) must still decode.
        let start = [snapshot(42, 7, 1000, &[1001, 1002], &[2001])];
        let update = [snapshot(42, 7, 1000, &[1001], &[2001, 2002])];
        let rows = build_detail_event_rows(&start, &update).unwrap();
        let payload = encode_detail_events_payload(1, STATUS_DELTA_DATA, &rows);
        assert_eq!(read_u16(&payload, 0), SCHEMA_VERSION_V1_NO_SIDE_DETAIL);
        let decoded = decode_detail_events_payload(0, &payload).unwrap();
        assert!(decoded.side_detail_rows.is_empty());
        assert!(decoded.attribution_rows.is_empty());
        assert_eq!(decoded.rows.len(), rows.len());
    }

    #[test]
    fn side_detail_committed_plus_reserve_must_equal_units_without_badge() {
        // An invariant violation WITHOUT the incomplete badge is a corrupt/torn
        // payload and must be rejected.
        let mut bad = sample_side_detail(2, 2);
        bad.units = 5; // committed+reserve = 4 != 5
        bad.counts_incomplete = false;
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001],
            &[9001],
            Some(bad),
            None,
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let err = decode_detail_events_payload(0, payload).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.contains("committed+reserve != units without incomplete badge")));
    }

    #[test]
    fn side_detail_invariant_violation_accepted_when_incomplete_badged() {
        // The SAME violation WITH the incomplete badge (a genuine mid-combat
        // transient the collector observed with a stable double-read) is
        // RETAINED and round-trips — the row is not dropped.
        let mut transient = sample_side_detail(2, 0);
        transient.units = 3; // committed 2 + reserve 0 = 2 != 3
        transient.counts_incomplete = true;
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001],
            &[9001],
            Some(transient),
            None,
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        assert_eq!(frame.side_detail_rows.len(), 1);
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let decoded = decode_detail_events_payload(0, payload).expect("incomplete row decodes");
        assert_eq!(decoded.side_detail_rows.len(), 1);
        assert!(decoded.side_detail_rows[0].detail.counts_incomplete);
        assert_eq!(decoded.side_detail_rows[0].detail.units, 3);
        // The flag round-trips in the wire flags.
        let gate = gate_stream(&certify_lane_data_record(frame.compact_stream));
        assert!(gate.ok, "{:?}", gate.errors);
    }

    #[test]
    fn side_detail_damage_out_of_range_is_rejected() {
        let mut bad = sample_side_detail(1, 0);
        bad.ground_damage_str_raw = MAX_SIDE_DETAIL_DAMAGE_RAW + 1;
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001],
            &[9001],
            Some(bad),
            None,
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        assert!(decode_detail_events_payload(0, payload).is_err());
    }
}
