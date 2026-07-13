//! Fold decoded compact records into per-game-hour world-state snapshots shaped
//! like `replay_interface/documents/REPLAY_SCHEMA.md`.
//!
//! The recorder already packs the raw compact stream for audit; this module
//! additionally maintains a running world state, applies each DOMAIN_DATA record
//! (full-replace for full domains, delta-apply for delta domains), and on each
//! FRAME_END emits one snapshot for that game hour. `finish()` yields the
//! per-hour snapshot JSON bytes ready to pack as `role: "snapshot"` entries.
//!
//! TAG resolution: compact records key country rows by numeric `tag_db_id`. The
//! `country_identity` domain (id 18) carries a per-frame `tag_db_id -> TAG` map;
//! when present we re-key `countries` and every country-valued field (state
//! owner/controller, province controller, division owner, and the map layer
//! mirrors) by TAG and set `tag_db_id_index.resolved = true`. Per REPLAY_SCHEMA
//! §0.2 a TAG is never fabricated: any tag_db_id with no mapping keeps its
//! stringified numeric key, `resolved` stays false, and a note records the gap.
//! Without any domain-18 record we fall back to numeric keys throughout with
//! `resolved = false`, the fail-closed choice; raw ids are preserved, never
//! guessed strings.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use air_loss_stats_live_lane as airloss61;
use air_wings_variants_lifecycle_live_lane::domain46_air_ace_roster_live as ace46;
use army_battleplans_depth_live_lane as battleplans54;
use capitulation_war_state_live as capwar60;
use characters_leaders_cabinet_live_lane as domain47;
use compact_contract::calendar::{date_from_raw, hour_index_from_raw};
use compact_contract::{
    compact_status_is_failure, parse_compact_record_header, COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_FAILURE_MASK, COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_TRANSITION_BASELINE_RESET,
};
use construction_buildings_state_damage_live_lane as domain48;
use diplomacy_intel_operatives_live as diplomacy53;
use hub_motorization_live_lane as hub62;
use land_combat_detail_events_live_lane as land_detail55;
use naval_combat_live_lane as naval4950;
use naval_missions_status_live_lane as naval56;
use serde_json::{json, Map, Value};
use session_multiplayer_metadata_live as session57;

/// Env-gated, zero-overhead-when-off phase profiler for the fold hot path.
/// Set `FOLD_PROFILE=1` to accumulate per-phase nanoseconds and print a
/// breakdown from the `stream_fold` tool at exit. Never affects folded bytes,
/// so it is safe to leave compiled in (all `Instant` calls are behind the flag).
pub mod foldprof {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    pub static APPLY_NS: AtomicU64 = AtomicU64::new(0);
    pub static BUILD_NS: AtomicU64 = AtomicU64::new(0);
    pub static SERIALIZE_NS: AtomicU64 = AtomicU64::new(0);
    pub static WRITE_NS: AtomicU64 = AtomicU64::new(0);
    pub static SNAP_BYTES: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn enabled() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("FOLD_PROFILE").is_some())
    }

    /// Time `f` into `counter` only when profiling is enabled.
    #[inline]
    pub fn timed<T>(counter: &AtomicU64, f: impl FnOnce() -> T) -> T {
        if enabled() {
            let t = Instant::now();
            let out = f();
            counter.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            out
        } else {
            f()
        }
    }

    pub fn add_bytes(n: usize) {
        if enabled() {
            SNAP_BYTES.fetch_add(n as u64, Ordering::Relaxed);
        }
    }

    /// Human-readable breakdown, or `None` when profiling was never enabled.
    pub fn report() -> Option<String> {
        if !enabled() {
            return None;
        }
        let ms = |c: &AtomicU64| c.load(Ordering::Relaxed) as f64 / 1e6;
        Some(format!(
            "fold phase breakdown (ms): apply_domain_data={:.1} build_snapshot={:.1} serialize={:.1} write={:.1} | snapshot_bytes={}",
            ms(&APPLY_NS),
            ms(&BUILD_NS),
            ms(&SERIALIZE_NS),
            ms(&WRITE_NS),
            SNAP_BYTES.load(Ordering::Relaxed),
        ))
    }
}

const SCHEMA_VERSION: &str = "1";

/// The canonical world-state snapshot schema version stamped into every folded
/// hour (`schema_version` field). Exposed so a stream consumer (e.g. the
/// `replay_decoder` facade) can report it as session metadata without folding a
/// snapshot first.
pub fn schema_version() -> &'static str {
    SCHEMA_VERSION
}

const FUEL_SCALE: f64 = 32768.0;
const PRODUCTION_SCALE: f64 = 100_000.0;
const TECH_PROGRESS_SCALE: f64 = 1000.0;
const TECH_LIVE_SCALE: f64 = 100_000.0;
const FIXED_SCALE: f64 = 100_000.0;

// W1 domain 63 country_core_scalars_live: fixed 48-byte per-country row.
const COUNTRY_CORE_ROW_LEN: usize = 48;
const COUNTRY_CORE_FLAG_MANPOWER_POOL: u32 = 1 << 0;
const COUNTRY_CORE_FLAG_CONVOYS_DESTROYED: u32 = 1 << 1;
const COUNTRY_CORE_FLAG_DIVISIONS_LOST: u32 = 1 << 2;
const COUNTRY_CORE_FLAG_EXPERIENCE: u32 = 1 << 3;
// Experience-status channels are raw signed int32 CFixedPoint (value / 32768).
const COUNTRY_CORE_EXPERIENCE_SCALE: f64 = 32768.0;
const COUNTRY_CORE_MAX_MANPOWER_POOL: u32 = 2_000_000_000;
const COUNTRY_CORE_MAX_CONVOYS_DESTROYED: u32 = 100_000_000;
const COUNTRY_CORE_MAX_DIVISIONS_LOST: u32 = 1_000_000;

// W3 domain 64 delayed_events_live: fixed 20-byte per-event row.
const DELAYED_EVENT_ROW_LEN: usize = 20;
const DELAYED_EVENT_FLAG_DUE_REMAINING_HOURS: u32 = 1 << 0;
const DELAYED_EVENT_FLAG_ORIGINATOR: u32 = 1 << 1;
const DELAYED_EVENT_KNOWN_FLAGS: u32 =
    DELAYED_EVENT_FLAG_DUE_REMAINING_HOURS | DELAYED_EVENT_FLAG_ORIGINATOR;
// Sane ceiling on the remaining-hours countdown (~57 years of 30-day-month
// hours), matching the collector-side DOMAIN51_MAX_DELAYED_EVENT_DUE_HOURS.
const DELAYED_EVENT_MAX_DUE_REMAINING_HOURS: u32 = 500_000;
const DELAYED_EVENT_DOMAIN_ID: u16 = 64;

// W3 domain 65 special_project_programs_live: fixed 16-byte per-facility row,
// preceded by a 24-byte header (elapsed_wall_ms u64, row_count u32 x2,
// status_flags u32, dropped_ambiguous u32).
const SPECIAL_PROJECT_PROGRAM_ROW_LEN: usize = 16;
const SPECIAL_PROJECT_PROGRAM_HEADER_LEN: usize = 24;
const SPECIAL_PROJECT_PROGRAM_DOMAIN_ID: u16 = 65;

// W4 domain 66 occupation_state_extras_live: fixed 24-byte per-occupied-state
// row (controller_tag u32, occupied_tag u32, state_id u32, flags u32,
// occupation_law_token_id u32, garrison_manpower_source_tag_db_id u32),
// preceded by a 20-byte header (elapsed_wall_ms u64, row_count u32 x2,
// status_flags u32).
const OCCUPATION_STATE_EXTRAS_ROW_LEN: usize = 24;
const OCCUPATION_STATE_EXTRAS_HEADER_LEN: usize = 20;
const OCCUPATION_STATE_EXTRAS_FLAG_LAW: u32 = 1 << 0;
const OCCUPATION_STATE_EXTRAS_FLAG_GARRISON_SOURCE: u32 = 1 << 1;
const OCCUPATION_STATE_EXTRAS_KNOWN_FLAGS: u32 =
    OCCUPATION_STATE_EXTRAS_FLAG_LAW | OCCUPATION_STATE_EXTRAS_FLAG_GARRISON_SOURCE;
const OCCUPATION_STATE_EXTRAS_DOMAIN_ID: u16 = 66;

// W5 domain 67 country_dynamic_modifiers_live: 20-byte header
// (elapsed_wall_ms u64, country_count u32, total_id_count u32, status_flags u32),
// then country_count fixed 12-byte rows (tag_db_id u32, id_count u32,
// id_blob_offset u32), then a single concatenated id blob where each id is a
// u16 length prefix + utf8 bytes.
const COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN: usize = 20;
const COUNTRY_DYNAMIC_MODIFIERS_ROW_LEN: usize = 12;
const COUNTRY_DYNAMIC_MODIFIERS_DOMAIN_ID: u16 = 67;
// Fail-closed ceiling on a modifier-id string length (matches the collector
// DOMAIN67_MAX_DYNAMIC_MODIFIER_ID_LEN). A longer length in the blob is a torn
// payload -> the whole frame folds closed (prior map retained).
const COUNTRY_DYNAMIC_MODIFIER_MAX_ID_LEN: usize = 128;

// W6 domain 68 session_header_live: 16-byte header (elapsed_wall_ms u64,
// field_presence_flags u32, status_flags u32), then a fixed scalar block of
// 5 x u32 (player_tag_db_id, dlcs, start_date_raw, save_version, session), then
// three length-prefixed utf8 strings (ideology, cosmetic_tag, version), each a
// u16 length + bytes. A field is meaningful only when its presence bit is set.
const SESSION_HEADER_HEADER_LEN: usize = 16;
const SESSION_HEADER_SCALAR_BLOCK_LEN: usize = 20;
const SESSION_HEADER_DOMAIN_ID: u16 = 68;
const SESSION_HEADER_FLAG_PLAYER: u32 = 1 << 0;
const SESSION_HEADER_FLAG_DLCS: u32 = 1 << 1;
const SESSION_HEADER_FLAG_START_DATE: u32 = 1 << 2;
const SESSION_HEADER_FLAG_SAVE_VERSION: u32 = 1 << 3;
const SESSION_HEADER_FLAG_SESSION: u32 = 1 << 4;
const SESSION_HEADER_FLAG_IDEOLOGY: u32 = 1 << 5;
const SESSION_HEADER_FLAG_COSMETIC_TAG: u32 = 1 << 6;
const SESSION_HEADER_FLAG_VERSION: u32 = 1 << 7;
const SESSION_HEADER_KNOWN_FLAGS: u32 = SESSION_HEADER_FLAG_PLAYER
    | SESSION_HEADER_FLAG_DLCS
    | SESSION_HEADER_FLAG_START_DATE
    | SESSION_HEADER_FLAG_SAVE_VERSION
    | SESSION_HEADER_FLAG_SESSION
    | SESSION_HEADER_FLAG_IDEOLOGY
    | SESSION_HEADER_FLAG_COSMETIC_TAG
    | SESSION_HEADER_FLAG_VERSION;
// Fail-closed ceiling on a session-header token/banner length. Matches the
// collector's DOMAIN68_MAX_TOKEN_LEN / DOMAIN68_MAX_VERSION_LEN (128): a longer
// string is impossible writer output and folds closed (prior header retained).
const SESSION_HEADER_MAX_STRING_LEN: usize = 128;
// Fail-closed ceiling on the player tag_db_id (matches the collector guard).
const SESSION_HEADER_MAX_PLAYER_TAG_DB_ID: u32 = 2_048;

// W6 domain 69 tech_sharing_groups_live: 16-byte header (elapsed_wall_ms u64,
// group_count u32, status_flags u32), then group_count fixed 12-byte rows
// (id_token u32, member_count u32, member_blob_offset u32 in u32 units), then a
// single concatenated blob of u32 tag_db_ids.
const TECH_SHARING_GROUPS_HEADER_LEN: usize = 16;
const TECH_SHARING_GROUPS_ROW_LEN: usize = 12;
const TECH_SHARING_GROUPS_DOMAIN_ID: u16 = 69;
// Fail-closed ceiling on the number of groups. Tied to the collector's own
// vector-size ceiling (build_profile DOMAIN69_TECH_SHARING_MAX_GROUPS): the
// reader now walks the tech-sharing std::vector's active size [0..size), not a
// fixed 9 slots, so a save with MORE than 9 groups emits >9 rows and this host
// cap MUST match or a clean >9 frame would be rejected outright (regression:
// re-gate 019f5467 BLOCKER — the old fixed-9 cap silently truncated, the new
// reader emits the full set). A payload beyond this ceiling is a torn/crafted
// record.
const TECH_SHARING_GROUP_MAX_GROUPS: usize =
    compact_contract::build_profile::DOMAIN69_TECH_SHARING_MAX_GROUPS as usize;
// Fail-closed ceiling on a group's member count (matches the collector guard).
const TECH_SHARING_GROUP_MAX_MEMBERS: u32 = 512;
// Fail-closed ceiling on a member tag_db_id (matches the collector guard).
const TECH_SHARING_GROUP_MAX_MEMBER_TAG_DB_ID: u32 = 2_048;

// E4: division row grew 92 -> 104 to carry strategic_redeployment,
// current_manpower, and cumulative_losses (must stay in lockstep with the
// collector and recorder-decode row lengths).
const DIVISION_ROW_LEN: usize = 104;
/// Wave-4 disrupted-supply presence (row +88). A present 0 is valid.
const DIVISION_FLAG_DISRUPTED_SUPPLY: u32 = 1 << 17;
const DIVISION_FLAG_STRATEGIC_REDEPLOYMENT: u32 = 1 << 18;
const DIVISION_FLAG_CURRENT_MANPOWER: u32 = 1 << 19;
const DIVISION_FLAG_CUMULATIVE_LOSSES: u32 = 1 << 20;
/// Wave-4 supply_gain presence (row +76, packed by the collector at flag
/// bit 10 — already in the row, now folded into the D20 division snapshot).
const DIVISION_FLAG_SUPPLY_GAIN: u32 = 1 << 10;
const DIVISION_FLAG_DELETED: u32 = 1 << 12;
// Wave-3 primary-order linkage flags (mutually exclusive; ORDER_ARMY carries
// the COrderInstance+0x244 instance id in the row tail u32 at +84).
const DIVISION_FLAG_ORDER_ARMY: u32 = 1 << 14;
const DIVISION_FLAG_ORDER_ARMY_GROUP: u32 = 1 << 15;
const DIVISION_FLAG_ORDER_UNRESOLVED: u32 = 1 << 16;
// Same ceiling as the collector read and the summary decoder.
const DIVISION_MAX_ORDER_INSTANCE_ID: u32 = 100_000_000;
const DIVISION_MAX_CURRENT_MANPOWER: u32 = 10_000_000;
const DIVISION_MAX_CUMULATIVE_LOSSES: u32 = 100_000_000;
const LAND_COMBAT_ROW_LEN: usize = 40;
const LAND_COMBAT_PARTICIPANT_ROW_LEN: usize = 24;
const MAP_STATE_ROW_LEN: usize = 16;
const MAP_PROVINCE_ROW_LEN: usize = 16;
const OCCUPATION_HEADER_LEN: usize = 52;
const OCCUPATION_ROW_LEN: usize = 64;
const MAP_DETAIL_HEADER_LEN: usize = 48;
const MAP_DETAIL_STATE_ROW_LEN: usize = 28;
const MAP_DETAIL_PROVINCE_ROW_LEN: usize = 20;
const MAP_DETAIL_ENTRY_ROW_LEN: usize = 12;
const SUPPLY_LOGISTICS_HEADER_LEN: usize = 48;
const SUPPLY_LOGISTICS_ROW_LEN: usize = 40;
const MARKET_TRADE_HEADER_LEN: usize = 48;
const MARKET_TRADE_ROW_LEN: usize = 48;
const MARKET_TRADE_CONTRACT_TYPE: u32 = 81;
const MARKET_TRADE_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;

const OCC_ROW_FLAG_OWNER: u32 = 1;
const OCC_ROW_FLAG_CONTROLLER: u32 = 1 << 1;
const OCC_ROW_FLAG_RESISTANCE: u32 = 1 << 2;
const OCC_ROW_FLAG_COMPLIANCE: u32 = 1 << 3;
const OCC_ROW_FLAG_OCCUPIED_STATE: u32 = 1 << 4;
const OCC_ROW_FLAG_OCCUPIED_TAG_DERIVED: u32 = 1 << 5;
const OCC_ROW_FLAG_OCCUPATION_LAW_TOKEN: u32 = 1 << 6;
const OCC_ROW_FLAG_GARRISON_TEMPLATE: u32 = 1 << 7;
const OCC_ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE: u32 = 1 << 8;
const OCC_ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE: u32 = 1 << 9;
const OCC_ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE: u32 = 1 << 10;
const OCC_ROW_FLAG_RESISTANCE_SPEED: u32 = 1 << 11;
const OCC_ROW_FLAG_COMPLIANCE_SPEED: u32 = 1 << 12;
const OCC_ROW_FLAG_OCCUPATION_STATUS_ROOT: u32 = 1 << 13;
const OCC_ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN: u32 = 1 << 14;
const OCC_ROW_FLAG_GARRISON_SCALARS: u32 = 1 << 15;
const OCC_ROW_KNOWN_FLAGS: u32 = OCC_ROW_FLAG_OWNER
    | OCC_ROW_FLAG_CONTROLLER
    | OCC_ROW_FLAG_RESISTANCE
    | OCC_ROW_FLAG_COMPLIANCE
    | OCC_ROW_FLAG_OCCUPIED_STATE
    | OCC_ROW_FLAG_OCCUPIED_TAG_DERIVED
    | OCC_ROW_FLAG_OCCUPATION_LAW_TOKEN
    | OCC_ROW_FLAG_GARRISON_TEMPLATE
    | OCC_ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
    | OCC_ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
    | OCC_ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
    | OCC_ROW_FLAG_RESISTANCE_SPEED
    | OCC_ROW_FLAG_COMPLIANCE_SPEED
    | OCC_ROW_FLAG_OCCUPATION_STATUS_ROOT
    | OCC_ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN
    | OCC_ROW_FLAG_GARRISON_SCALARS;

const SUPPLY_ROW_KIND_PROVINCE_LOGISTICS: u32 = 1;
const SUPPLY_ROW_KIND_DIVISION_SUPPLY: u32 = 2;
const SUPPLY_ROW_KIND_COMPUTED_BADGE: u32 = 3;
const SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT: u32 = 4;
const SUPPLY_ROW_FLAG_RAIL: u32 = 1;
const SUPPLY_ROW_FLAG_SUPPLY_NODE: u32 = 1 << 1;
const SUPPLY_ROW_FLAG_NAVAL_SUPPLY_HUB: u32 = 1 << 2;
const SUPPLY_ROW_FLAG_INFRASTRUCTURE: u32 = 1 << 3;
const SUPPLY_ROW_FLAG_LIVE_BUILDING_VECTOR: u32 = 1 << 4;
const SUPPLY_ROW_FLAG_DIVISION_CURRENT_SUPPLY: u32 = 1 << 5;
const SUPPLY_ROW_FLAG_DIVISION_MAX_SUPPLY: u32 = 1 << 6;
const SUPPLY_ROW_FLAG_DIVISION_SUPPLY_GAIN: u32 = 1 << 7;
const SUPPLY_ROW_FLAG_DIVISION_OWNER: u32 = 1 << 8;
const SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL: u32 = 1 << 9;
const SUPPLY_ROW_FLAG_ATTRITION_WALL: u32 = 1 << 10;
const SUPPLY_ROW_FLAG_PROXY_VALUE: u32 = 1 << 11;
const SUPPLY_ROW_FLAG_DIVISION_SUPPLY_DEFICIT: u32 = 1 << 12;
const SUPPLY_ROW_FLAG_DIVISION_FUEL: u32 = 1 << 13;
const SUPPLY_ROW_FLAG_MOTORIZATION_WALL: u32 = 1 << 14;
const SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL: u32 = 1 << 15;
const SUPPLY_ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED: u32 = 1 << 16;
const SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY: u32 = 1 << 17;
const SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY: u32 = 1 << 18;
const SUPPLY_ROW_ID_DELETED: u32 = 1 << 31;
const SUPPLY_MAX_PROVINCE_ID: u32 = 13_413;
const SUPPLY_MAX_STATE_ID: u32 = 1_081;
const SUPPLY_MAX_DIVISION_ID: u32 = i32::MAX as u32;
const SUPPLY_MAX_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
const SUPPLY_ROW_KNOWN_FLAGS: u32 = SUPPLY_ROW_FLAG_RAIL
    | SUPPLY_ROW_FLAG_SUPPLY_NODE
    | SUPPLY_ROW_FLAG_NAVAL_SUPPLY_HUB
    | SUPPLY_ROW_FLAG_INFRASTRUCTURE
    | SUPPLY_ROW_FLAG_LIVE_BUILDING_VECTOR
    | SUPPLY_ROW_FLAG_DIVISION_CURRENT_SUPPLY
    | SUPPLY_ROW_FLAG_DIVISION_MAX_SUPPLY
    | SUPPLY_ROW_FLAG_DIVISION_SUPPLY_GAIN
    | SUPPLY_ROW_FLAG_DIVISION_OWNER
    | SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL
    | SUPPLY_ROW_FLAG_ATTRITION_WALL
    | SUPPLY_ROW_FLAG_PROXY_VALUE
    | SUPPLY_ROW_FLAG_DIVISION_SUPPLY_DEFICIT
    | SUPPLY_ROW_FLAG_DIVISION_FUEL
    | SUPPLY_ROW_FLAG_MOTORIZATION_WALL
    | SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL
    | SUPPLY_ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED
    | SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY
    | SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY;

const MARKET_TRADE_FLAG_BUYER_PTR_VALID: u32 = 1;
const MARKET_TRADE_FLAG_EQUIPMENT_SUBBUF_VALID: u32 = 1 << 1;
const MARKET_TRADE_FLAG_DELIVERY_SUBBUF_VALID: u32 = 1 << 2;
const MARKET_TRADE_FLAG_VARIANT_VALID: u32 = 1 << 3;
const MARKET_TRADE_FLAG_AMOUNT_VALID: u32 = 1 << 4;
const MARKET_TRADE_KNOWN_FLAGS: u32 = MARKET_TRADE_FLAG_BUYER_PTR_VALID
    | MARKET_TRADE_FLAG_EQUIPMENT_SUBBUF_VALID
    | MARKET_TRADE_FLAG_DELIVERY_SUBBUF_VALID
    | MARKET_TRADE_FLAG_VARIANT_VALID
    | MARKET_TRADE_FLAG_AMOUNT_VALID;
const MARKET_TRADE_DETAIL_TRAILER_MAGIC: &[u8; 4] = b"MTD1";
const MARKET_TRADE_DETAIL_TRAILER_VERSION: u16 = 1;
const MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN: usize = 32;
const MARKET_TRADE_DETAIL_ROW_LEN: usize = 64;
const MARKET_TRADE_DETAIL_FLAG_PRICE_VALID: u32 = 1;
const MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID: u32 = 1 << 1;
const MARKET_TRADE_DETAIL_FLAG_SPEED_VALID: u32 = 1 << 2;
const MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID: u32 = 1 << 3;
const MARKET_TRADE_DETAIL_FLAG_DAYS_VALID: u32 = 1 << 4;
const MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID: u32 = 1 << 5;
const MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID: u32 = 1 << 6;
const MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID: u32 = 1 << 7;
const MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID: u32 = 1 << 8;
const MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID: u32 = 1 << 9;
const MARKET_TRADE_DETAIL_FLAGS_KNOWN: u32 = MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
    | MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
    | MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
    | MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID
    | MARKET_TRADE_DETAIL_FLAG_DAYS_VALID
    | MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
    | MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
    | MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID
    | MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
    | MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID;
const MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC: &[u8; 4] = b"MTRF";
const MARKET_TRADE_RESOURCE_FLOW_TRAILER_VERSION: u16 = 1;
const MARKET_TRADE_RESOURCE_FLOW_TRAILER_HEADER_LEN: usize = 32;
const MARKET_TRADE_RESOURCE_FLOW_ROW_LEN: usize = 72;
const MARKET_TRADE_RESOURCE_FLOW_KIND_EXPORTED: u32 = 3;
const MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK: u32 = 0x7F;
const MARKET_TRADE_RESOURCE_NAMES: [&str; 7] = [
    "oil",
    "aluminium",
    "rubber",
    "tungsten",
    "steel",
    "chromium",
    "coal",
];

const DIVISIONS_DOMAIN_ID: u16 = 20;
const WEATHER_DOMAIN_ID: u16 = 24;
const DIVISION_EQUIPMENT_DOMAIN_ID: u16 = 26;
const ARMY_ORDER_DOMAIN_ID: u16 = 27;
const OOB_COMMAND_DOMAIN_ID: u16 = 28;
const SHIPS_DOMAIN_ID: u16 = 29;
const AIR_WINGS_DOMAIN_ID: u16 = 30;
const AIR_REGION_ACTIVITY_DOMAIN_ID: u16 = 31;
const AIR_WING_LOADOUTS_DOMAIN_ID: u16 = 32;
const AIR_VARIANT_DEFS_DOMAIN_ID: u16 = 33;
const AIR_BASE_STATUS_DOMAIN_ID: u16 = 34;
const AIR_VARIANT_STATS_DOMAIN_ID: u16 = 35;
const AIR_VARIANT_RESERVES_DOMAIN_ID: u16 = 36;
const STOCKPILE_VARIANT_DEFS_DOMAIN_ID: u16 = 37;
const AIR_SUPERIORITY_INPUTS_DOMAIN_ID: u16 = 38;
const AIR_VARIANT_UPGRADES_DOMAIN_ID: u16 = 39;
const STOCKPILE_VARIANT_UPGRADES_DOMAIN_ID: u16 = 40;
const SPECIAL_PROJECTS_DOMAIN_ID: u16 = 41;
const AIR_COMBAT_LOSSES_DOMAIN_ID: u16 = 42;
const AIR_LOSS_STATS_DOMAIN_ID: u16 = airloss61::DOMAIN_ID;
const HUB_MOTORIZATION_DOMAIN_ID: u16 = hub62::DOMAIN_ID;
const FOCUS_RESEARCH_DECISIONS_DEPTH_DOMAIN_ID: u16 = 45;
const AIR_VARIANT_ARCHETYPES_DOMAIN_ID: u16 = 46;
const CHARACTERS_LEADERS_CABINET_DOMAIN_ID: u16 = 47;
const CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID: u16 = 48;
const SCRIPTED_STATE_DOMAIN_ID: u16 = 51;
const DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID: u16 = diplomacy53::DOMAIN_ID;
const CAPITULATION_WAR_STATE_DOMAIN_ID: u16 = capwar60::DOMAIN_ID;
const ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID: u16 = battleplans54::DOMAIN_ID;
const NAVAL_MISSIONS_STATUS_DOMAIN_ID: u16 = 56;
const NAVAL_COMBAT_LIFECYCLE_DOMAIN_ID: u16 = naval4950::DOMAIN_NAVAL_COMBAT_LIFECYCLE;
const NAVAL_COMBAT_MEMBERS_DOMAIN_ID: u16 = naval4950::DOMAIN_NAVAL_COMBAT_MEMBERS;
const SESSION_MULTIPLAYER_METADATA_DOMAIN_ID: u16 = session57::DOMAIN_ID;

fn compact_expected_partial_tolerance_applies(domain_id: u16, status_flags: u32) -> bool {
    compact_contract::expected_partial_tolerance_applies(domain_id, status_flags).is_some()
}

fn compact_failure_flags_after_expected_partial_tolerance(
    domain_id: u16,
    status_flags: u32,
) -> u32 {
    compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
        domain_id,
        status_flags,
    )
}

/// Layer-2 sparse merge (d53): replace the existing element with the same key,
/// else append. Preserves insertion order — existing keys update in place, new
/// keys append — so on a freshly-CLEARED vec (a clean d53 frame) it is
/// byte-identical to `push`. On a tolerated-PARTIAL frame the vec is NOT cleared,
/// so an element the frame omits is retained instead of fabricated as a deletion.
fn domain53_upsert_by_key<T, K: PartialEq>(vec: &mut Vec<T>, row: T, key: impl Fn(&T) -> K) {
    let k = key(&row);
    if let Some(slot) = vec.iter_mut().find(|existing| key(existing) == k) {
        *slot = row;
    } else {
        vec.push(row);
    }
}

fn domain53_remove_by_key<T, K: PartialEq>(vec: &mut Vec<T>, key_value: K, key: impl Fn(&T) -> K) {
    vec.retain(|row| key(row) != key_value);
}

fn domain53_row_deleted(value: u32) -> bool {
    value & diplomacy53::compact::ROW_ID_DELETED != 0
}

fn domain53_row_key(value: u32) -> u32 {
    value & !diplomacy53::compact::ROW_ID_DELETED
}

fn domain53_war_key(first: u32, second: u32) -> (u32, u32) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn domain53_projected_status_flags(status_flags: u32) -> u32 {
    status_flags & !COMPACT_STATUS_DELTA_DATA
}

const DOMAIN47_R2_KEY: &str = "_r2_characters_leaders_cabinet_live";
/// Wave-4 ace lifecycle: per-country ace-roster sidecar (domain 46 `ACE1`
/// trailer; full-replace each trailer-bearing hour).
const DOMAIN46_ACE_R2_KEY: &str = "_r2_air_ace_roster_live";
const CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY: &str =
    "construction_buildings_state_damage_live";
const DOMAIN53_EXTRA_KEY: &str = "domain53_extra";
const DOMAIN53_R2_KEY: &str = "_r2_diplomacy_intel_operatives_live";
const DOMAIN57_R2_KEY: &str = "_r2_session_multiplayer_metadata_live";
const DOMAIN57_ROOT_PLAYER_COUNTRY: u32 = 1 << 0;
const DOMAIN57_ROOT_PAUSE_CLOCK: u32 = 1 << 1;

const ROW_FLAG_DELETED_LOW_BIT: u32 = 1;
const AIR_STRENGTH_DELTA_ROW_FLAG_DELETED: u32 = 1 << 2;

const WEATHER_HEADER_LEN: usize = 48;
const WEATHER_REGION_ROW_LEN: usize = 32;
const WEATHER_PROVINCE_ROW_LEN: usize = 32;
const WEATHER_REGION_ROWS: usize = 305;
const WEATHER_MIN_PROVINCE_ROWS: usize = 10_000;
const WEATHER_MAX_PROVINCE_ROWS: usize = 65_536;
const WEATHER_MAX_REGION_ID: u32 = 304;
const WEATHER_MAX_PHENOMENON: u32 = 16;
const WEATHER_MAX_TEMPERATURE_RAW: i32 = 10_000_000;
const WEATHER_MAX_TEMP_CHANGE_RAW: i32 = 10_000_000;
const WEATHER_MAX_PRECIP_RAW: i32 = 100_000_000;
const WEATHER_MAX_ACTIVE_MODIFIERS: u32 = 4_096;
const WEATHER_REGION_RAIN_LIGHT: u32 = 1;
const WEATHER_REGION_RAIN_HEAVY: u32 = 1 << 1;
const WEATHER_REGION_SNOW: u32 = 1 << 2;
const WEATHER_REGION_BLIZZARD: u32 = 1 << 3;
const WEATHER_REGION_ARCTIC_WATER: u32 = 1 << 4;
const WEATHER_REGION_ACTIVE_SIGNAL: u32 = 1 << 5;
const WEATHER_REGION_PHENOMENON_ACTIVE: u32 = 1 << 6;
const WEATHER_REGION_SANDSTORM: u32 = 1 << 7;
const WEATHER_REGION_KNOWN_FLAGS: u32 = WEATHER_REGION_RAIN_LIGHT
    | WEATHER_REGION_RAIN_HEAVY
    | WEATHER_REGION_SNOW
    | WEATHER_REGION_BLIZZARD
    | WEATHER_REGION_ARCTIC_WATER
    | WEATHER_REGION_ACTIVE_SIGNAL
    | WEATHER_REGION_PHENOMENON_ACTIVE
    | WEATHER_REGION_SANDSTORM;
const WEATHER_PROVINCE_TEMP_PRESENT: u32 = 1;
const WEATHER_PROVINCE_TEMP_CHANGE_PRESENT: u32 = 1 << 1;
const WEATHER_PROVINCE_WATER_PRESENT: u32 = 1 << 2;
const WEATHER_PROVINCE_SNOW_PRESENT: u32 = 1 << 3;
const WEATHER_PROVINCE_MUD_PRESENT: u32 = 1 << 4;
const WEATHER_PROVINCE_MUD_ACTIVE: u32 = 1 << 5;
const WEATHER_PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT: u32 = 1 << 6;
const WEATHER_PROVINCE_STRUCT_VALIDATED: u32 = 1 << 7;
const WEATHER_PROVINCE_MAP_RECOLOR: u32 = 1 << 8;
const WEATHER_PROVINCE_DYNAMIC_LUT: u32 = 1 << 9;
const WEATHER_PROVINCE_KNOWN_FLAGS: u32 = WEATHER_PROVINCE_TEMP_PRESENT
    | WEATHER_PROVINCE_TEMP_CHANGE_PRESENT
    | WEATHER_PROVINCE_WATER_PRESENT
    | WEATHER_PROVINCE_SNOW_PRESENT
    | WEATHER_PROVINCE_MUD_PRESENT
    | WEATHER_PROVINCE_MUD_ACTIVE
    | WEATHER_PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT
    | WEATHER_PROVINCE_STRUCT_VALIDATED
    | WEATHER_PROVINCE_MAP_RECOLOR
    | WEATHER_PROVINCE_DYNAMIC_LUT;

const DIVISION_EQUIPMENT_HEADER_LEN: usize = 40;
const DIVISION_EQUIPMENT_ROOT_ROW_LEN: usize = 24;
const DIVISION_EQUIPMENT_ENTRY_ROW_LEN: usize = 16;
const DIVISION_EQUIPMENT_ROOT_FLAG_TEMPLATE_PRESENT: u32 = 1;
const DIVISION_EQUIPMENT_ROOT_FLAG_DELETED: u32 = 2;
const DIVISION_EQUIPMENT_ROOT_KNOWN_FLAGS: u32 =
    DIVISION_EQUIPMENT_ROOT_FLAG_TEMPLATE_PRESENT | DIVISION_EQUIPMENT_ROOT_FLAG_DELETED;

const SPECIAL_PROJECTS_HEADER_BASE_LEN: usize = 64;
const SPECIAL_PROJECTS_HEADER_ROOT_COUNTERS_LEN: usize = 24;
const SPECIAL_PROJECTS_HEADER_REASON_COUNTERS_LEN: usize = 48;
const SPECIAL_PROJECTS_HEADER_PROGRESS_LEN: usize = 16;
const SPECIAL_PROJECTS_FACILITY_ROW_LEN: usize = 32;
const SPECIAL_PROJECTS_PROJECT_ROW_LEN: usize = 24;
const SPECIAL_PROJECTS_SCIENTIST_ROW_LEN: usize = 32;
const SPECIAL_PROJECTS_PROJECT_STATE_ROW_LEN: usize = 48;
const SPECIAL_PROJECTS_BREAKTHROUGH_ROW_LEN: usize = 16;
const SPECIAL_PROJECTS_FACILITY_ACTIVE_PROJECT: u32 = 1;
const SPECIAL_PROJECTS_FACILITY_SCIENTIST: u32 = 1 << 1;
const SPECIAL_PROJECTS_FACILITY_BUILDING: u32 = 1 << 2;
const SPECIAL_PROJECTS_FACILITY_DELETED: u32 = 1 << 31;
const SPECIAL_PROJECTS_FACILITY_KNOWN_FLAGS: u32 = SPECIAL_PROJECTS_FACILITY_ACTIVE_PROJECT
    | SPECIAL_PROJECTS_FACILITY_SCIENTIST
    | SPECIAL_PROJECTS_FACILITY_BUILDING
    | SPECIAL_PROJECTS_FACILITY_DELETED;
const SPECIAL_PROJECTS_PROJECT_WORKED_FACILITY: u32 = 1;
const SPECIAL_PROJECTS_PROJECT_DELETED: u32 = 1 << 31;
const SPECIAL_PROJECTS_SCIENTIST_NAME_TOKEN: u32 = 1;
const SPECIAL_PROJECTS_SCIENTIST_DISPLAY_NAME: u32 = 1 << 1;
const SPECIAL_PROJECTS_SCIENTIST_TRAIT_COUNT: u32 = 1 << 2;
const SPECIAL_PROJECTS_SCIENTIST_SKILLS: u32 = 1 << 3;
const SPECIAL_PROJECTS_SCIENTIST_DELETED: u32 = 1 << 31;
const SPECIAL_PROJECTS_SCIENTIST_KNOWN_FLAGS: u32 = SPECIAL_PROJECTS_SCIENTIST_NAME_TOKEN
    | SPECIAL_PROJECTS_SCIENTIST_DISPLAY_NAME
    | SPECIAL_PROJECTS_SCIENTIST_TRAIT_COUNT
    | SPECIAL_PROJECTS_SCIENTIST_SKILLS
    | SPECIAL_PROJECTS_SCIENTIST_DELETED;
const SPECIAL_PROJECTS_PROJECT_STATE_COMPLETED: u32 = 1;
const SPECIAL_PROJECTS_PROJECT_STATE_DELETED: u32 = 1 << 31;
const SPECIAL_PROJECTS_BREAKTHROUGH_DELETED: u32 = 1 << 31;

const FRD_HEADER_LEN_V1: usize = 48;
const FRD_HEADER_LEN: usize = 56;
const FRD_SCHEMA_MAGIC: &[u8; 4] = b"FRD2";
const FRD_SCHEMA_VERSION: u16 = 2;
const FRD_FOCUS_ROW_FIXED_LEN: usize = 36;
const FRD_RESEARCH_ROW_LEN: usize = 80;
const FRD_DECISION_STATUS_ROW_LEN: usize = 28;
const FRD_DECISION_EVENT_ROW_FIXED_LEN: usize = 28;
const FRD_DECISION_TARGET_ROW_FIXED_LEN: usize = 16;
const FRD_BADGE_ROW_FIXED_LEN: usize = 20;
const FRD_FOCUS_COMPLETED_COUNT_INCREASED: u32 = 1;
const FRD_FOCUS_CURRENT_STARTED: u32 = 2;
const FRD_FOCUS_STOPPED: u32 = 3;
const FRD_FOCUS_CHANGED: u32 = 4;
const FRD_FOCUS_CONTINUOUS_KIND: u32 = 2;
const FRD_FOCUS_FLAG_COMPLETED_TOKEN_INFERRED: u32 = 1;
const FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT: u32 = 1 << 1;
const FRD_RESEARCH_FLAG_ETA_AVAILABLE: u32 = 1 << 2;
const FRD_RESEARCH_FLAG_BASE_PRESENT: u32 = 1 << 5;
const FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED: u32 = 1;
const FRD_DECISION_TARGET_FLAG_COUNTRY_TAG: u32 = 1;
const FRD_DECISION_TARGET_FLAG_RECORD_0X20: u32 = 1 << 1;
const FRD_DECISION_TARGET_COUNTRY_FLAGS: u32 =
    FRD_DECISION_TARGET_FLAG_COUNTRY_TAG | FRD_DECISION_TARGET_FLAG_RECORD_0X20;
const FRD_MAX_TARGET_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;

const SCRIPTED_STATE_HEADER_LEN: usize = 64;
// Wire schema version for domain 51 (header byte 52). Bumped to 2 with the
// measured delayed-event row family + due-remaining-hours; the fold rejects a
// payload whose version it does not understand rather than misparsing it.
const SCRIPTED_STATE_SCHEMA_VERSION: u32 = 2;
const SCRIPTED_STATE_ROW_LEN: usize = 48;
const SCRIPTED_STATE_STATUS_FLAG_NAMES_PARTIAL: u32 =
    compact_contract::DOMAIN51_STATUS_FLAG_NAMES_PARTIAL;
const SCRIPTED_STATE_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED: u32 =
    compact_contract::DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED;
const SCRIPTED_STATE_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED: u32 =
    compact_contract::DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;
const SCRIPTED_STATE_OPAQUE_STATUS_MASK: u32 = SCRIPTED_STATE_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
    | SCRIPTED_STATE_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;
const SCRIPTED_STATE_SCOPE_GLOBAL: u32 = 1;
const SCRIPTED_STATE_SCOPE_COUNTRY: u32 = 2;
const SCRIPTED_STATE_SCOPE_STATE: u32 = 3;
const SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64: u32 = 1;
const SCRIPTED_STATE_VALUE_FLAG_NAMED: u32 = 2;
const SCRIPTED_STATE_VALUE_FLAG_TOKEN_ONLY: u32 = 3;
const SCRIPTED_STATE_VALUE_DELAYED_EVENT_OPAQUE: u32 = 4;
const SCRIPTED_STATE_VALUE_SCRIPTED_GUI_OPAQUE: u32 = 5;
// Wave-3 measured pending delayed-event row (owner + event id + scope +
// random seed; due-date absent, a documented residual). Same ceilings as the
// collector emit and the payload gate.
const SCRIPTED_STATE_VALUE_DELAYED_EVENT: u32 = 6;
const SCRIPTED_STATE_VALUE_KIND_DELETED: u32 = 1 << 31;
const SCRIPTED_STATE_MIN_DELAYED_EVENT_ID: u32 = 100_000;
const SCRIPTED_STATE_MAX_DELAYED_EVENT_ID: u32 = 100_000 * 42_949;
const SCRIPTED_STATE_ROW_FLAG_KEY_PRESENT: u32 = 1;
const SCRIPTED_STATE_ROW_FLAG_TOKEN_PRESENT: u32 = 1 << 1;
const SCRIPTED_STATE_ROW_FLAG_VALUE_PRESENT: u32 = 1 << 2;
const SCRIPTED_STATE_ROW_FLAG_DATE_PRESENT: u32 = 1 << 3;
const SCRIPTED_STATE_ROW_FLAG_DELETED: u32 = 1 << 4;
// Wave-3 delayed-event due-remaining-hours present (rides reserved row u32 at
// base+40). Same ceiling as the collector emit + payload gate.
const SCRIPTED_STATE_ROW_FLAG_DELAYED_DUE_PRESENT: u32 = 1 << 5;
const SCRIPTED_STATE_MAX_DELAYED_EVENT_DUE_HOURS: u32 =
    compact_contract::build_profile::DOMAIN51_MAX_DELAYED_EVENT_DUE_HOURS;
const SCRIPTED_STATE_ROW_KNOWN_FLAGS: u32 = SCRIPTED_STATE_ROW_FLAG_KEY_PRESENT
    | SCRIPTED_STATE_ROW_FLAG_TOKEN_PRESENT
    | SCRIPTED_STATE_ROW_FLAG_VALUE_PRESENT
    | SCRIPTED_STATE_ROW_FLAG_DATE_PRESENT
    | SCRIPTED_STATE_ROW_FLAG_DELETED
    | SCRIPTED_STATE_ROW_FLAG_DELAYED_DUE_PRESENT;

const ARMY_ORDER_HEADER_LEN: usize = 40;
const ARMY_ORDER_ROW_LEN: usize = 48;
const ARMY_ORDER_TARGET_ROW_LEN: usize = 8;
const OOB_COMMAND_HEADER_LEN: usize = 48;
const OOB_COMMAND_ROW_LEN: usize = 80;
const OOB_COMMAND_MEMBER_ROW_LEN: usize = 8;
const SHIP_HEADER_LEN: usize = 40;
const SHIP_FIXED_ROW_LEN: usize = 40;
const AIR_WING_HEADER_LEN: usize = 36;
const AIR_WING_ROW_LEN: usize = 40;
const AIR_REGION_ACTIVITY_HEADER_LEN: usize = 36;
const AIR_REGION_ACTIVITY_ROW_LEN: usize = 32;
const AIR_WING_LOADOUT_HEADER_LEN: usize = 44;
const AIR_WING_LOADOUT_ROW_LEN: usize = 32;
const AIR_WING_LOADOUT_ENTRY_ROW_LEN: usize = 16;
const VARIANT_DEF_HEADER_LEN: usize = 44;
const VARIANT_DEF_ROW_LEN: usize = 24;
const VARIANT_DEF_MODULE_ROW_LEN: usize = 8;
const AIR_BASE_STATUS_HEADER_LEN: usize = 36;
const AIR_BASE_STATUS_ROW_LEN: usize = 32;
const AIR_VARIANT_STATS_HEADER_LEN: usize = 36;
const AIR_VARIANT_STATS_ROW_LEN: usize = 64;
const AIR_VARIANT_RESERVE_HEADER_LEN: usize = 48;
const AIR_VARIANT_RESERVE_ROW_LEN: usize = 24;
const AIR_SUPERIORITY_INPUT_HEADER_LEN: usize = 52;
const AIR_SUPERIORITY_INPUT_ROW_LEN: usize = 48;
const VARIANT_UPGRADE_HEADER_LEN: usize = 44;
const VARIANT_UPGRADE_ROW_LEN: usize = 16;
const VARIANT_UPGRADE_ENTRY_ROW_LEN: usize = 8;
const AIR_COMBAT_LOSS_HEADER_LEN: usize = 48;
// Row grew from 68 to 76 bytes with the wave-3 per-wing cumulative scalars
// (air_accidents u32 @68, experience_raw i32 @72).
const AIR_COMBAT_LOSS_ROW_LEN: usize = 76;
// Wing experience is cumulative XP, fixed-x100000 (not a 0..1 ratio).
const AIR_WING_MAX_EXPERIENCE_RAW: i32 = 1_000_000_000;
// Cumulative per-wing accident counter: keep the fold's bound consistent with the
// collector (MAX_AIR_WING_PLANES) and the summary decoder (COMPACT_AIR_WING_MAX_PLANES).
const AIR_WING_MAX_ACCIDENTS: u32 = 10_000_000;
const AIR_VARIANT_ARCHETYPE_HEADER_LEN: usize = 48;
const AIR_VARIANT_ARCHETYPE_ROW_LEN: usize = 32;

const OOB_KIND_ARMY: u32 = 1;
const OOB_KIND_ARMY_GROUP: u32 = 2;
const OOB_KIND_FLEET: u32 = 3;
const OOB_KIND_TASK_FORCE: u32 = 4;

const ARMY_ORDER_ROW_FLAG_TARGET_STATES: u32 = 1 << 1;
const ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
const ARMY_ORDER_KNOWN_FLAGS: u32 = ROW_FLAG_DELETED_LOW_BIT
    | ARMY_ORDER_ROW_FLAG_TARGET_STATES
    | ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT;
const OOB_ROW_FLAG_PARENT: u32 = 1 << 1;
const OOB_ROW_FLAG_OWNER: u32 = 1 << 2;
const OOB_ROW_FLAG_LEADER: u32 = 1 << 3;
const OOB_ROW_FLAG_LEADER_SKILL: u32 = 1 << 4;
const OOB_ROW_FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
const OOB_ROW_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
const OOB_ROW_FLAG_LOCATION: u32 = 1 << 7;
const OOB_KNOWN_FLAGS: u32 = ROW_FLAG_DELETED_LOW_BIT
    | OOB_ROW_FLAG_PARENT
    | OOB_ROW_FLAG_OWNER
    | OOB_ROW_FLAG_LEADER
    | OOB_ROW_FLAG_LEADER_SKILL
    | OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
    | OOB_ROW_FLAG_MISSING_MEMBER_COUNT
    | OOB_ROW_FLAG_LOCATION;
const SHIP_ROW_FLAG_TASK_FORCE: u32 = 1 << 1;
const SHIP_ROW_FLAG_DEFINITION: u32 = 1 << 2;
const SHIP_ROW_FLAG_OWNER: u32 = 1 << 3;
const SHIP_KNOWN_FLAGS: u32 = ROW_FLAG_DELETED_LOW_BIT
    | SHIP_ROW_FLAG_TASK_FORCE
    | SHIP_ROW_FLAG_DEFINITION
    | SHIP_ROW_FLAG_OWNER;
const AIR_WING_ROW_FLAG_MISSION_REGION: u32 = 1 << 1;
const AIR_WING_KNOWN_FLAGS: u32 = ROW_FLAG_DELETED_LOW_BIT | AIR_WING_ROW_FLAG_MISSION_REGION;
const AIR_STRENGTH_DELTA_ROW_FLAG_LOSS: u32 = 1;
const AIR_STRENGTH_DELTA_ROW_FLAG_REINFORCEMENT: u32 = 1 << 1;
const AIR_STRENGTH_DELTA_ROW_FLAG_MISSION_REGION: u32 = 1 << 3;
const AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK: u32 = 1 << 4;
const AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS: u32 = 1 << 5;
const AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE: u32 = 1 << 6;
const AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALAR_CHANGE: u32 = 1 << 7;
const AIR_STRENGTH_DELTA_KNOWN_FLAGS: u32 = AIR_STRENGTH_DELTA_ROW_FLAG_LOSS
    | AIR_STRENGTH_DELTA_ROW_FLAG_REINFORCEMENT
    | AIR_STRENGTH_DELTA_ROW_FLAG_DELETED
    | AIR_STRENGTH_DELTA_ROW_FLAG_MISSION_REGION
    | AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK
    | AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS
    | AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE
    | AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALAR_CHANGE;
const VARIANT_DEF_ROW_FLAG_DISPLAY_NAME: u32 = 1;
const VARIANT_DEF_ROW_FLAG_VARIANT_NAME: u32 = 1 << 1;
const VARIANT_DEF_ROW_FLAG_MODULES: u32 = 1 << 2;
const VARIANT_DEF_KNOWN_FLAGS: u32 = VARIANT_DEF_ROW_FLAG_DISPLAY_NAME
    | VARIANT_DEF_ROW_FLAG_VARIANT_NAME
    | VARIANT_DEF_ROW_FLAG_MODULES;
const AIR_VARIANT_ARCHETYPE_ROW_FLAG_ARCHETYPE: u32 = 1 << 1;
const AIR_VARIANT_ARCHETYPE_ROW_FLAG_BASE_AIRFRAME: u32 = 1 << 2;
const AIR_VARIANT_ARCHETYPE_ROW_FLAG_ROLE: u32 = 1 << 3;
const AIR_VARIANT_ARCHETYPE_ROW_FLAG_AIR_SUPERIORITY_WEIGHT: u32 = 1 << 4;
const AIR_VARIANT_ARCHETYPE_KNOWN_FLAGS: u32 = ROW_FLAG_DELETED_LOW_BIT
    | AIR_VARIANT_ARCHETYPE_ROW_FLAG_ARCHETYPE
    | AIR_VARIANT_ARCHETYPE_ROW_FLAG_BASE_AIRFRAME
    | AIR_VARIANT_ARCHETYPE_ROW_FLAG_ROLE
    | AIR_VARIANT_ARCHETYPE_ROW_FLAG_AIR_SUPERIORITY_WEIGHT;

/// The set of domains this fold understands and folds into world state.
pub const FOLDED_DOMAIN_IDS: &[u16] = &[
    10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35,
    36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 60, 61,
    62, 63, 64, 65, 66, 67, 68, 69,
];

/// Metadata for a folded snapshot spilled to disk by [`WorldFold::new_file_backed`].
pub struct FoldedSnapshotFile {
    pub hour_index: i64,
    pub date: String,
    pub path: PathBuf,
    pub byte_len: usize,
    /// Canonical SHA-256 (lowercase hex) of the spilled snapshot bytes, computed
    /// during the write itself so downstream consumers never have to re-read the
    /// file to hash it. `None` only for snapshots constructed outside the fold
    /// (e.g. unit-test fixtures).
    pub sha256: Option<String>,
}

/// A closed game-hour handed from the (sequential) fold thread to the background
/// emit worker: the fold builds the canonical snapshot `Value`, the worker
/// serializes + writes + hashes it. `Value` is `Send`, so ownership moves with no
/// copy; the fold thread proceeds to the next hour immediately.
struct SnapshotEmitJob {
    hour_index: i64,
    date: String,
    path: PathBuf,
    snapshot: Value,
}

/// Shared result sink for the emit worker. The worker pushes one
/// [`FoldedSnapshotFile`] per successfully written hour (or records the first
/// error). Ordering is re-established by the fold thread at finalize, so the
/// worker never needs to preserve receive order.
#[derive(Default)]
struct SnapshotEmitResults {
    files: Vec<FoldedSnapshotFile>,
    error: Option<String>,
}

/// Background snapshot emitter. Moves serialize + write + per-hour SHA-256 off
/// the fold's critical path so they overlap the next hour's `build_snapshot`
/// (the dominant, inherently-sequential fold cost). A bounded channel provides
/// backpressure so peak memory stays O(a small constant × one hour's snapshot),
/// independent of stream length — the same bounded-memory contract the inline
/// path guaranteed, just with a small fixed number of in-flight hours.
struct SnapshotEmitPipeline {
    tx: Option<std::sync::mpsc::SyncSender<SnapshotEmitJob>>,
    handle: Option<std::thread::JoinHandle<()>>,
    results: std::sync::Arc<std::sync::Mutex<SnapshotEmitResults>>,
}

/// In-flight hours the emit channel may hold before the fold thread blocks. A
/// single worker keeps up with the fold thread comfortably (emit is a small
/// fraction of `build_snapshot`), so this only caps the worst case. Two is
/// enough to fully decouple producer and consumer while keeping peak memory low.
const SNAPSHOT_EMIT_QUEUE_DEPTH: usize = 2;

impl SnapshotEmitPipeline {
    fn spawn() -> SnapshotEmitPipeline {
        let (tx, rx) = std::sync::mpsc::sync_channel::<SnapshotEmitJob>(SNAPSHOT_EMIT_QUEUE_DEPTH);
        let results = std::sync::Arc::new(std::sync::Mutex::new(SnapshotEmitResults::default()));
        let worker_results = std::sync::Arc::clone(&results);
        let handle = std::thread::Builder::new()
            .name("world-fold-emit".to_string())
            .spawn(move || {
                for job in rx {
                    // Once any hour has failed, keep draining the channel (so the
                    // fold thread never deadlocks on a bounded send) but skip work.
                    if worker_results.lock().unwrap().error.is_some() {
                        continue;
                    }
                    match serialize_write_hash_snapshot(&job.path, &job.snapshot) {
                        Ok((byte_len, sha256)) => {
                            worker_results
                                .lock()
                                .unwrap()
                                .files
                                .push(FoldedSnapshotFile {
                                    hour_index: job.hour_index,
                                    date: job.date,
                                    path: job.path,
                                    byte_len,
                                    sha256: Some(sha256),
                                });
                        }
                        Err(err) => {
                            let mut guard = worker_results.lock().unwrap();
                            if guard.error.is_none() {
                                guard.error = Some(format!(
                                    "write folded snapshot hour_index {} failed: {err}",
                                    job.hour_index
                                ));
                            }
                        }
                    }
                }
            })
            .expect("spawn world-fold emit worker");
        SnapshotEmitPipeline {
            tx: Some(tx),
            handle: Some(handle),
            results,
        }
    }

    /// Hand a closed hour to the worker. Blocks only if the bounded channel is
    /// full (backpressure). Records a fatal error if the worker has hung up.
    fn submit(&mut self, job: SnapshotEmitJob) {
        if let Some(tx) = &self.tx {
            if let Err(err) = tx.send(job) {
                let mut guard = self.results.lock().unwrap();
                if guard.error.is_none() {
                    guard.error = Some(format!("snapshot emit worker unavailable: {err}"));
                }
            }
        }
    }

    /// Close the channel, join the worker, and return the spilled files in
    /// deterministic emit order (ascending `hour_index`) or the first error.
    fn finish(mut self) -> Result<Vec<FoldedSnapshotFile>, String> {
        drop(self.tx.take());
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| "snapshot emit worker panicked".to_string())?;
        }
        let mut results = std::sync::Mutex::into_inner(
            std::sync::Arc::try_unwrap(self.results)
                .map_err(|_| "snapshot emit results still shared".to_string())?,
        )
        .map_err(|_| "snapshot emit results lock poisoned".to_string())?;
        if let Some(err) = results.error.take() {
            return Err(err);
        }
        results.files.sort_by_key(|f| f.hour_index);
        Ok(results.files)
    }
}

/// Serialize a canonical snapshot `Value` straight into its spill file while
/// SHA-256-hashing the exact bytes written — one streaming pass, no intermediate
/// `Vec` and no re-read. `serde_json::to_writer` emits byte-for-byte the same
/// stream as `serde_json::to_vec`, so the hash and the file are identical to the
/// old inline path (G1 byte-identity preserved).
fn serialize_write_hash_snapshot(
    path: &std::path::Path,
    snapshot: &Value,
) -> std::io::Result<(usize, String)> {
    use std::io::Write as _;
    foldprof::timed(&foldprof::SERIALIZE_NS, || {
        let file = fs::File::create(path)?;
        let mut writer = SnapshotHashingWriter {
            inner: std::io::BufWriter::with_capacity(1 << 20, file),
            hasher: snapshot::sha256::Sha256::new(),
            len: 0,
        };
        serde_json::to_writer(&mut writer, snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        writer.inner.flush()?;
        foldprof::add_bytes(writer.len);
        let sha = snapshot::sha256::to_hex(&writer.hasher.finalize());
        Ok((writer.len, sha))
    })
}

/// A `Write` adapter that tees to the underlying file writer while updating a
/// running SHA-256 and byte count.
struct SnapshotHashingWriter<W: std::io::Write> {
    inner: W,
    hasher: snapshot::sha256::Sha256,
    len: usize,
}

impl<W: std::io::Write> std::io::Write for SnapshotHashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.len += n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct SnapshotFileSink {
    dir: PathBuf,
    pipeline: SnapshotEmitPipeline,
    /// Emit-stride ownership for the parallel fold: this sink builds + spills
    /// only closed hours whose emission ordinal `o` satisfies
    /// `o % stride == stride_offset`. The fold's STATE transitions run for every
    /// hour regardless (`build_snapshot` is `&self`-pure, so skipping it cannot
    /// perturb any later hour). The sequential fold uses stride 1 / offset 0
    /// (owns every hour) — behavior unchanged.
    stride: u64,
    stride_offset: u64,
}

/// A running world-state accumulator. Feed every H4CR record; snapshots are
/// captured on FRAME_END and returned by [`WorldFold::finish`].
pub struct WorldFold {
    game_id: String,
    /// countries keyed by stringified tag_db_id (see module note on TAG resolution)
    countries: BTreeMap<String, Map<String, Value>>,
    /// states keyed by state-id string
    states: BTreeMap<String, Map<String, Value>>,
    /// provinces keyed by province-id string
    provinces: BTreeMap<String, Map<String, Value>>,
    /// D22-owned state fields keyed by state-id string.
    map_control_states: BTreeMap<String, Map<String, Value>>,
    /// D22-owned province fields keyed by province-id string.
    map_control_provinces: BTreeMap<String, Map<String, Value>>,
    /// divisions keyed by div_uid ("div_id:div_type"), carried across hours
    divisions: BTreeMap<String, Map<String, Value>>,
    /// D26-owned equipment rows keyed only by owner/div_id. They are joined to
    /// D20 divisions at snapshot build only when that key is unique.
    division_equipment: BTreeMap<DivisionEquipmentKey, Vec<DivisionEquipmentEntryState>>,
    division_equipment_touched_this_hour: BTreeSet<DivisionEquipmentKey>,
    /// D26 keys invalidated by D20 division deletes this hour. They are pruned
    /// at hour close so equipment cannot survive a deleted division and later
    /// attach to a reused owner/div_id with a different D20 div_type.
    division_equipment_deleted_division_keys_this_hour: BTreeSet<DivisionEquipmentKey>,
    /// Active land combats keyed by combat_id; a proven domain 21 snapshot
    /// full-replaces this set. Failed or absent snapshots never imply empty.
    combats: BTreeMap<u32, CombatState>,
    /// Domain 55 event annotations for the current hour only.
    combat_detail_events_this_hour: BTreeMap<(u32, u64), CombatDetailEnrichment>,
    /// D23 per-province detail used to rebuild state-folded province buildings
    /// and the renderer mirror without exposing non-schema province fields.
    map_detail_province_state_ids: BTreeMap<String, u32>,
    map_detail_province_buildings: BTreeMap<String, Map<String, Value>>,
    map_detail_live: Option<Value>,
    /// D48 full-frame construction/building/resource damage extras for the
    /// current hour only. D23 owns canonical state resources/buildings fields.
    domain48_known_this_hour: bool,
    domain48_unknown_this_hour: bool,
    /// D48 per-hour PROJECTED row state. Full-replaced by baselines, patched by
    /// deltas, sparse-merged by tolerated-partial frames, retained across unknown
    /// hours, and rendered only for proven hours. This is what the hour's snapshot
    /// reflects (so a tolerated-partial hour still shows its sparse-merged rows).
    domain48_state: Option<Domain48FoldState>,
    /// D48 DELTA BASELINE. The row state the next delta frame is applied onto.
    /// Advanced ONLY on clean (baseline) and delta frames — a tolerated-partial
    /// frame NEVER mutates it, mirroring the collector's `tracker.last_*` which is
    /// only advanced under the same conditions (collector `lib.rs` emit-mode:
    /// partial returns early without advancing the tracker). Keeping this separate
    /// from `domain48_state` prevents a tolerated-partial frame's transient rows
    /// (phantom rows / reverted values) from leaking into the accumulator the next
    /// delta is diffed against — matching d53's two-accumulator design.
    domain48_delta_state: Option<Domain48FoldState>,
    /// D53 diplomacy/intel/operative validated slice, projected only for the
    /// current hour under 53-owned keys.
    domain53_known_this_hour: bool,
    domain53_country_overlays_this_hour: BTreeMap<u32, Domain53CountryOverlay>,
    domain53_world_overlay_this_hour: Domain53WorldOverlay,
    domain53_delta_country_overlays: BTreeMap<u32, Domain53CountryOverlay>,
    domain53_delta_world_overlay: Domain53WorldOverlay,
    /// D60 capitulation/war state. Per-country capitulation is a PERSISTENT
    /// full-replace map (cosmetic-tag pattern: last proven frame wins; a
    /// country absent from the map gets NO capitulation key — absent != false).
    /// Wars are a current-hour overlay: `diplomacy_world.wars` is null until a
    /// proven D60 frame lands this hour (a proven frame with zero wars is a
    /// genuine empty object, distinct from unknown).
    capitulation_by_db_id: BTreeMap<u32, capwar60::compact::CountryCapitulationRow>,
    capitulation_war_state_known_this_hour: bool,
    war_state_wars_this_hour: Vec<capwar60::compact::WarRow>,
    /// D43 world-scoped market contracts; full-replaced by each successful frame.
    market_trade_contracts: Option<Vec<Value>>,
    /// D43 validated exported-resource slice; full-replaced only by valid MTRF
    /// trailers. Countries with zero exported resources remain absent.
    market_trade_resource_exported: BTreeMap<u32, [i64; 7]>,
    /// D44 live supply proxy sidecar rendered from the row-state accumulator.
    supply: Option<Map<String, Value>>,
    /// D44 row-state accumulator. Full-replaced by baselines, patched by deltas,
    /// and retained on failure/partial/source-unavailable frames.
    supply_logistics: Option<SupplyLogisticsState>,
    /// D24 R2 weather overlay projected under `map._r2_weather_live`.
    weather_live: Option<WeatherLiveState>,
    /// D41 top-level special-projects snapshot.
    special_projects: Option<SpecialProjectsState>,
    /// D45 focus/research/decision depth annotations. D12 owns first-class
    /// focus fields; D45 can fill them only when absent while also writing R2
    /// sidecars. Event rows and badges are per-hour provenance; live counters
    /// are carried until replaced.
    frd_focus_metrics: BTreeMap<u32, FrdFocusMetricsState>,
    frd_research_live: BTreeMap<FrdResearchKey, FrdResearchLiveState>,
    frd_decision_status: BTreeMap<u32, FrdDecisionStatusState>,
    frd_decision_targets: BTreeMap<u32, Vec<FrdDecisionTargetState>>,
    frd_decision_targets_unknown: BTreeSet<u32>,
    frd_focus_events_this_hour: BTreeMap<u32, Vec<FrdFocusDepthEventState>>,
    frd_decision_events_this_hour: BTreeMap<u32, Vec<FrdDecisionEventState>>,
    frd_depth_badges_this_hour: Vec<FrdDepthBadgeState>,
    /// D51 scripted state accumulators. Country/state scoped leaves are stored
    /// in the shared `countries`/`states` objects and cleared by D51 only.
    scripted_world_flags: BTreeMap<String, ScriptFlagState>,
    scripted_world_variables: BTreeMap<String, f64>,
    scripted_world_token_only_flags: BTreeMap<u32, ScriptFlagState>,
    scripted_world_opaque_badges: Vec<ScriptedOpaqueBadgeState>,
    /// Wave-3 measured pending delayed-event rows (owner + event id + scope +
    /// random seed; due-date absent). Replaced wholesale on every full D51
    /// snapshot; keyed by (owner, event_id, scope_random) so repeats collapse.
    scripted_world_delayed_events: BTreeMap<(u32, u32, u64), DelayedEventFoldState>,
    /// Chunk2 order-of-battle/design/air accumulators. These remain domain-owned
    /// internally and are merged into shared snapshot sections only at build time.
    army_orders: BTreeMap<ArmyOrderKey, ArmyOrderState>,
    oob_commands: BTreeMap<OobCommandKey, OobCommandState>,
    ships: BTreeMap<u32, ShipState>,
    /// D49/D50 naval combat lifecycle + members, keyed by (combat_id,
    /// generation). Cumulative delta state carried across hours; emitted under
    /// the top-level `naval_combats` snapshot section.
    naval_combats: BTreeMap<(u32, u64), NavalCombatFoldState>,
    /// True only after a successful D49 record in the current hour. The
    /// cumulative accumulator survives failures/missing frames for later
    /// deltas, but battle existence is not current truth without this witness.
    naval_combat_lifecycle_known_this_hour: bool,
    /// True only after a successful D50 record in the current hour. Member rows
    /// and task-force-id arrays are omitted without this witness.
    naval_combat_members_known_this_hour: bool,
    /// D56 R2-projected naval task-force status annotations. D28 owns OOB
    /// existence; D56 only annotates validated task-force status under
    /// `order_of_battle.extra.naval_missions_status_live`.
    naval_missions_status: BTreeMap<u32, NavalMissionTaskForceFoldRow>,
    /// D56 E10/R6 naval supremacy rows. Stored as an ORDER-PRESERVING occurrence
    /// list, not a map: the game's per-strategic-region naval-dominance array
    /// legitimately holds multiple entries for the same (region, controller)
    /// with distinct supremacy values (validated live at D04 1944 — region 29
    /// dominance tags [2,22,39,22] — and in the acceptance fixture). Keying by
    /// (region, controller) would collapse those occurrences last-write-wins and
    /// silently drop real data, so every decoded row is retained in emit order.
    naval_supremacy: Vec<NavalSupremacyFoldRow>,
    /// D57 validated partial session metadata sidecar for the current hour.
    session_multiplayer_metadata_this_hour: Option<Domain57FoldState>,
    /// W6 D68 session header (persistent last-value-wins). The last successfully
    /// folded save/session header (player/ideology/cosmetic_tag/dlcs/start_date/
    /// save_version/session/version). Fields absent from the record stay at
    /// their previous value (static fields never change; the mutable ones are
    /// change-detected downstream). A failed frame retains the prior header.
    session_header: Option<SessionHeaderFoldState>,
    /// W6 D69 tech-sharing groups (persistent last-value-wins). The last proven
    /// complete group set (all decoded slots). Retained on a failed/partial frame.
    tech_sharing_groups: Option<Vec<TechSharingGroupFoldState>>,
    /// D54 R2-projected active army battleplan depth annotations. D27 owns
    /// `battle_plan_orders`; D54 writes only its named extra bucket.
    army_battleplans_depth: BTreeMap<ArmyBattleplanDepthKey, ArmyBattleplanDepthOrder>,
    army_battleplans_depth_unknown_this_hour: bool,
    army_battleplans_depth_omitted_non_exact_this_hour: u64,
    army_battleplans_depth_path_omitted_this_hour: u64,
    air_wings: BTreeMap<AirWingKey, AirWingState>,
    air_wing_loadouts: BTreeMap<AirWingKey, AirWingLoadoutState>,
    /// D32 loadout keys touched this hour or invalidated by a D30 delete. At
    /// hour close these are checked against D30-owned wing existence and pruned
    /// once if proven orphaned, so stale loadouts cannot resurrect later wings.
    air_wing_loadout_orphan_check_keys_this_hour: BTreeSet<AirWingKey>,
    air_region_activity: BTreeMap<AirRegionActivityKey, AirRegionActivityState>,
    air_base_status: BTreeMap<u32, AirBaseStatusState>,
    air_variant_defs: BTreeMap<u32, VariantDefState>,
    air_variant_stats: BTreeMap<u32, AirVariantStatsState>,
    air_variant_upgrades: BTreeMap<u32, Vec<VariantUpgradeState>>,
    air_variant_archetypes: BTreeMap<u32, AirVariantArchetypeState>,
    /// Wave-4 ace lifecycle (domain-46 `ACE1` trailer): full ace roster keyed
    /// by (owner tag_db_id, ace_id); FULL-REPLACE only on a clean complete
    /// frame, retained on partial/source-unavailable/failed frames.
    air_aces: BTreeMap<(u32, u32), ace46::AirAceRosterRow>,
    /// True only for a game-hour whose D46 record folded successfully AND
    /// applied a clean (non-partial, non-source-unavailable) ace trailer.
    /// Gates the D30 snapshot `air_wings[*].ace` join and sidecar emission so
    /// a hour that could-not-read does not surface stale ace annotations as
    /// this hour's ground truth (Codex DEFECT: stale ace usable after D46
    /// failure).
    air_aces_fresh_this_hour: bool,
    stockpile_variant_defs: BTreeMap<u32, VariantDefState>,
    stockpile_variant_upgrades: BTreeMap<u32, Vec<VariantUpgradeState>>,
    air_variant_reserves: BTreeMap<AirVariantReserveKey, AirVariantReserveState>,
    air_superiority_inputs: BTreeMap<AirSuperiorityInputKey, AirSuperiorityInputState>,
    air_combat_losses_this_hour: Option<Vec<AirCombatLossState>>,
    /// D61 per-country cumulative air-loss statistics from statistic id 23.
    /// Full-replaced on clean frames and retained on non-clean frames.
    air_loss_stats: BTreeMap<u32, airloss61::AirLossStatRow>,
    /// D62 per-country motorized supply-node memberships from CCountrySupplySystem.
    /// Full-replaced on clean frames and retained on non-clean frames.
    hub_motorization: BTreeMap<(u32, u32), hub62::HubMotorizationRow>,
    /// Persistent per-wing cumulative scalars (wave-3: air_accidents,
    /// experience), keyed by globally-unique wing_id. Updated by every
    /// scalar-bearing domain-42 row; deleted rows remove their wing.
    air_wing_cumulative: BTreeMap<u32, AirWingCumulativeScalars>,
    /// Successful stateful fold domains. Used to distinguish absent-never-seen
    /// from a measured empty baseline when building schema containers.
    successful_fold_domains_seen: BTreeSet<u16>,
    /// Domains that were present this hour but unknown/fail-closed. Their
    /// accumulators are retained, but owned leaves emit null for this snapshot.
    fold_domains_unknown_this_hour: BTreeSet<u16>,
    /// Whether this hour has a proven active-combat snapshot. Without one,
    /// `combats` is emitted as null even though the carried state is retained
    /// internally for the next proven full snapshot.
    land_combats_known_this_hour: bool,
    land_combats_unknown_note_this_hour: Option<String>,
    /// tag_db_id -> TAG string from the country_identity domain (18); carried
    /// across hours and full-replaced whenever a domain-18 record arrives. Used
    /// at snapshot-build time to re-key country-valued fields by TAG.
    tag_by_db_id: BTreeMap<u32, String>,
    /// E13 measured-country observability: true only when a PROVEN domain-18
    /// record landed THIS hour, i.e. `tag_by_db_id` is the freshly-measured
    /// country set for this frame (not a carried-over roster). Reset each hour;
    /// set in `fold_country_identity` on success. Lets downstream distinguish
    /// "country absent from a domain = no data for it" (the country IS in the
    /// measured set) from "country absent = not resolved this frame" (not in the
    /// measured set — e.g. the D01/D02/D03 dynamic observers the country-roots
    /// resolver excludes, a systemic ~356/437 bound shared by all
    /// country-rooted domains). When this is false, the measured set is UNKNOWN
    /// this hour, never falsely reported as empty or complete.
    country_identity_known_this_hour: bool,
    /// Wave-4 cosmetic tag: tag_db_id -> cosmetic override. `Some(tag)` = an
    /// active cosmetic tag; `None` = validated no-override (folds `null`). A
    /// country whose cosmetic field was UNREADABLE (absent) is NOT in this map
    /// at all (folds absent, never null/empty). Full-replaced with the D18 map.
    cosmetic_by_db_id: BTreeMap<u32, Option<String>>,
    /// domains that fed at least one row this hour (ordered, deduped)
    domains_this_hour: Vec<u16>,
    /// decoded domain data records received this hour that are not folded into
    /// the snapshot shape yet.
    unfolded_domains_this_hour: BTreeSet<u16>,
    /// per-hour domain problems that should be machine-visible in provenance.
    domain_problems_this_hour: Vec<DomainProblem>,
    /// per-hour fail-closed notes (decode issues that skipped a domain fold)
    notes_this_hour: Vec<String>,
    /// Delta-capable domains that have successfully applied a full baseline.
    delta_baselines_seen: BTreeSet<u16>,
    /// Domain 20 was explicitly unknown for this hour; keep the accumulator for
    /// future deltas, but emit null for the affected snapshot.
    divisions_unknown_this_hour: bool,
    /// Domain 22 was explicitly unknown for this hour; keep the accumulator for
    /// future deltas, but emit null for the affected snapshot.
    map_control_unknown_this_hour: bool,
    /// Domain 52 was explicitly unknown for this hour; keep the accumulator for
    /// future deltas, but emit null for affected occupation fields.
    occupation_unknown_this_hour: bool,
    /// Domain 23 was explicitly unknown for this hour; keep the accumulator for
    /// future deltas, but emit null for affected map-detail fields.
    map_detail_unknown_this_hour: bool,
    /// Domain 44 was explicitly unknown for this hour; keep the accumulator, but
    /// emit null for affected supply fields.
    supply_unknown_this_hour: bool,
    /// Domain 43 was explicitly unknown for this hour; keep the accumulator, but
    /// emit null for trade contract fields.
    market_trade_unknown_this_hour: bool,
    /// DOMAIN_DATA records dropped because their hour was already finalized.
    late_records_dropped_since_last_snapshot: BTreeMap<i64, usize>,
    /// finished snapshots: (hour_index, date, json_bytes)
    out: Vec<(i64, String, Vec<u8>)>,
    /// Optional production sink for large snapshot JSON bytes. When present,
    /// snapshots are written to disk at hour close and `out` stays empty.
    file_sink: Option<SnapshotFileSink>,
    file_sink_error: Option<String>,
    /// hours already emitted, to reject a duplicate FRAME_END for a game hour
    /// (the game campaign hour counts are small enough that this remains bounded).
    emitted_hours: BTreeSet<i64>,
}

#[derive(Default, Clone)]
struct Domain53CountryOverlay {
    country: Option<diplomacy53::compact::CountryDiplomacyRow>,
    opinions: Vec<diplomacy53::compact::OpinionRow>,
    relation_flags: Vec<diplomacy53::compact::RelationFlagRow>,
    agency: Option<diplomacy53::compact::AgencyRow>,
    agency_capacity: Option<diplomacy53::compact::AgencyCapacityClusterRow>,
    operatives: Vec<diplomacy53::compact::OperativeRow>,
    cryptology: Vec<diplomacy53::compact::CryptologyRow>,
    state_events: Vec<diplomacy53::compact::StateEventRow>,
}

impl Domain53CountryOverlay {
    fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.opinions.is_empty()
            && self.relation_flags.is_empty()
            && self.agency.is_none()
            && self.agency_capacity.is_none()
            && self.operatives.is_empty()
            && self.cryptology.is_empty()
            && self.state_events.is_empty()
    }
}

#[derive(Default, Clone)]
struct Domain53WorldOverlay {
    status_flags: u32,
    wars: Vec<diplomacy53::compact::WarRelationRow>,
    subjects: Vec<diplomacy53::compact::SubjectRow>,
    /// Wave-5 running-operation instances (world-scoped provisional sidecar).
    operation_instances: Vec<diplomacy53::compact::OperationInstanceRow>,
    incoming_actions: Vec<diplomacy53::compact::IncomingActionQueueRow>,
    intel_network_graphs: Vec<diplomacy53::compact::IntelNetworkGraphRow>,
    factions: Vec<diplomacy53::compact::FactionRow>,
    faction_roster_status: FactionRosterStatus,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
enum FactionRosterStatus {
    /// No FACS trailer/completeness witness. This includes the current
    /// collector's unreadable-system/header/slot path, which emits no trailer.
    #[default]
    Unavailable,
    /// A roster with an explicit completeness witness. The current wire can
    /// produce only a non-empty value; a future zero-row FACS witness can use
    /// the same state without conflating it with Unavailable.
    Measured { is_empty: bool },
}

impl FactionRosterStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Measured { is_empty: true } => "measured_empty",
            Self::Measured { is_empty: false } => "measured_nonempty",
        }
    }
}

#[derive(Default, Clone)]
struct Domain48FoldState {
    source_flags: u32,
    construction: BTreeMap<Domain48ConstructionKey, domain48::ConstructionQueueRow>,
    buildings: BTreeMap<Domain48BuildingKey, domain48::BuildingRow>,
    resources: BTreeMap<Domain48ResourceKey, domain48::StateResourceRow>,
}

type Domain48ConstructionKey = (u32, u32);
type Domain48BuildingKey = (u32, u32, u32, u32);
type Domain48ResourceKey = (u32, u32);

#[derive(Default, Clone)]
struct Domain57FoldState {
    players_by_country: Option<Vec<Domain57PlayerCountryProjection>>,
    pause_is_paused: Option<bool>,
}

#[derive(Clone)]
struct Domain57PlayerCountryProjection {
    country_tag: String,
    tag_db_id: u32,
    human_marker: bool,
    valid_from_hour_raw: i32,
}

/// W6 D68 session-header fold state (persistent last-value-wins). Each field is
/// `None` until a record carries it; once set, a record that omits the field
/// leaves the prior value in place (static fields never change; the mutable
/// player/ideology/cosmetic_tag are refreshed whenever a record carries them).
#[derive(Default, Clone)]
struct SessionHeaderFoldState {
    player_tag_db_id: Option<u32>,
    dlcs: Option<u32>,
    start_date_raw: Option<u32>,
    save_version: Option<u32>,
    session: Option<u32>,
    ideology: Option<String>,
    cosmetic_tag: Option<String>,
    version: Option<String>,
}

/// W6 D69 one tech-sharing group's folded membership: the group id token and
/// the (sorted) member tag_db_id set. The static group id STRING is not decoded
/// (the replay resolves the token via installed content).
#[derive(Clone)]
struct TechSharingGroupFoldState {
    id_token: u32,
    member_tag_db_ids: Vec<u32>,
}

impl WorldFold {
    pub fn new(game_id: impl Into<String>) -> Self {
        WorldFold {
            game_id: game_id.into(),
            countries: BTreeMap::new(),
            states: BTreeMap::new(),
            provinces: BTreeMap::new(),
            map_control_states: BTreeMap::new(),
            map_control_provinces: BTreeMap::new(),
            divisions: BTreeMap::new(),
            division_equipment: BTreeMap::new(),
            division_equipment_touched_this_hour: BTreeSet::new(),
            division_equipment_deleted_division_keys_this_hour: BTreeSet::new(),
            combats: BTreeMap::new(),
            combat_detail_events_this_hour: BTreeMap::new(),
            map_detail_province_state_ids: BTreeMap::new(),
            map_detail_province_buildings: BTreeMap::new(),
            map_detail_live: None,
            domain48_delta_state: None,
            domain48_known_this_hour: false,
            domain48_unknown_this_hour: false,
            domain48_state: None,
            domain53_known_this_hour: false,
            domain53_country_overlays_this_hour: BTreeMap::new(),
            domain53_world_overlay_this_hour: Domain53WorldOverlay::default(),
            domain53_delta_country_overlays: BTreeMap::new(),
            domain53_delta_world_overlay: Domain53WorldOverlay::default(),
            capitulation_by_db_id: BTreeMap::new(),
            capitulation_war_state_known_this_hour: false,
            war_state_wars_this_hour: Vec::new(),
            market_trade_contracts: None,
            market_trade_resource_exported: BTreeMap::new(),
            supply: None,
            supply_logistics: None,
            weather_live: None,
            special_projects: None,
            frd_focus_metrics: BTreeMap::new(),
            frd_research_live: BTreeMap::new(),
            frd_decision_status: BTreeMap::new(),
            frd_decision_targets: BTreeMap::new(),
            frd_decision_targets_unknown: BTreeSet::new(),
            frd_focus_events_this_hour: BTreeMap::new(),
            frd_decision_events_this_hour: BTreeMap::new(),
            frd_depth_badges_this_hour: Vec::new(),
            scripted_world_flags: BTreeMap::new(),
            scripted_world_variables: BTreeMap::new(),
            scripted_world_token_only_flags: BTreeMap::new(),
            scripted_world_opaque_badges: Vec::new(),
            scripted_world_delayed_events: BTreeMap::new(),
            army_orders: BTreeMap::new(),
            oob_commands: BTreeMap::new(),
            ships: BTreeMap::new(),
            naval_combats: BTreeMap::new(),
            naval_combat_lifecycle_known_this_hour: false,
            naval_combat_members_known_this_hour: false,
            naval_missions_status: BTreeMap::new(),
            naval_supremacy: Vec::new(),
            session_multiplayer_metadata_this_hour: None,
            session_header: None,
            tech_sharing_groups: None,
            army_battleplans_depth: BTreeMap::new(),
            army_battleplans_depth_unknown_this_hour: false,
            army_battleplans_depth_omitted_non_exact_this_hour: 0,
            army_battleplans_depth_path_omitted_this_hour: 0,
            air_wings: BTreeMap::new(),
            air_aces: BTreeMap::new(),
            air_aces_fresh_this_hour: false,
            air_wing_loadouts: BTreeMap::new(),
            air_wing_loadout_orphan_check_keys_this_hour: BTreeSet::new(),
            air_region_activity: BTreeMap::new(),
            air_base_status: BTreeMap::new(),
            air_variant_defs: BTreeMap::new(),
            air_variant_stats: BTreeMap::new(),
            air_variant_upgrades: BTreeMap::new(),
            air_variant_archetypes: BTreeMap::new(),
            stockpile_variant_defs: BTreeMap::new(),
            stockpile_variant_upgrades: BTreeMap::new(),
            air_variant_reserves: BTreeMap::new(),
            air_superiority_inputs: BTreeMap::new(),
            air_combat_losses_this_hour: None,
            air_loss_stats: BTreeMap::new(),
            hub_motorization: BTreeMap::new(),
            air_wing_cumulative: BTreeMap::new(),
            successful_fold_domains_seen: BTreeSet::new(),
            fold_domains_unknown_this_hour: BTreeSet::new(),
            land_combats_known_this_hour: false,
            land_combats_unknown_note_this_hour: None,
            tag_by_db_id: BTreeMap::new(),
            country_identity_known_this_hour: false,
            cosmetic_by_db_id: BTreeMap::new(),
            domains_this_hour: Vec::new(),
            unfolded_domains_this_hour: BTreeSet::new(),
            domain_problems_this_hour: Vec::new(),
            notes_this_hour: Vec::new(),
            delta_baselines_seen: BTreeSet::new(),
            divisions_unknown_this_hour: false,
            map_control_unknown_this_hour: false,
            occupation_unknown_this_hour: false,
            map_detail_unknown_this_hour: false,
            supply_unknown_this_hour: false,
            market_trade_unknown_this_hour: false,
            late_records_dropped_since_last_snapshot: BTreeMap::new(),
            out: Vec::new(),
            file_sink: None,
            file_sink_error: None,
            emitted_hours: BTreeSet::new(),
        }
    }

    /// Construct a fold that spills each produced snapshot to `snapshot_dir`
    /// immediately instead of retaining every snapshot JSON byte in memory.
    pub fn new_file_backed(
        game_id: impl Into<String>,
        snapshot_dir: impl Into<PathBuf>,
    ) -> std::io::Result<Self> {
        let dir = snapshot_dir.into();
        fs::create_dir_all(&dir)?;
        let mut fold = Self::new(game_id);
        fold.file_sink = Some(SnapshotFileSink {
            dir,
            pipeline: SnapshotEmitPipeline::spawn(),
            stride: 1,
            stride_offset: 0,
        });
        Ok(fold)
    }

    /// Construct a file-backed fold that OWNS only every `stride`-th closed hour
    /// (those with emission ordinal `% stride == stride_offset`), building and
    /// spilling snapshots for owned hours only. All fold STATE transitions still
    /// run for every hour, so N strided folds over the same stream produce, in
    /// union, byte-identical snapshots to one sequential fold — this is the unit
    /// of the parallel post-capture fold (`fold_compact_stream_file_parallel`).
    pub fn new_file_backed_strided(
        game_id: impl Into<String>,
        snapshot_dir: impl Into<PathBuf>,
        stride: u64,
        stride_offset: u64,
    ) -> std::io::Result<Self> {
        assert!(stride >= 1, "stride must be >= 1");
        assert!(
            stride_offset < stride,
            "stride_offset must be < stride (got {stride_offset} >= {stride})"
        );
        let mut fold = Self::new_file_backed(game_id, snapshot_dir)?;
        if let Some(sink) = &mut fold.file_sink {
            sink.stride = stride;
            sink.stride_offset = stride_offset;
        }
        Ok(fold)
    }

    /// Walk a whole compact stream, folding every DOMAIN_DATA record and
    /// snapshotting on every FRAME_END. Malformed framing stops the walk (the
    /// recorder's `summarize_compact_stream` already reports framing errors).
    pub fn ingest_stream(&mut self, raw: &[u8]) {
        let mut offset = 0usize;
        while offset < raw.len() {
            let remaining = raw.len() - offset;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break;
            }
            // The single H4CR framing primitive (compact_contract). Any malformed
            // header stops the walk, exactly as before.
            let header = match parse_compact_record_header(&raw[offset..]) {
                Ok(h) => h,
                Err(_) => break,
            };
            let record_len = header.record_len();
            if record_len > remaining {
                break;
            }
            let payload = &raw[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];
            self.ingest_record(
                header.game_hour_raw,
                header.domain_id,
                header.record_type,
                payload,
                header.record_flags,
            );
            offset += record_len;
        }
    }

    /// Fold a single already-framed compact record. This is the exact per-record
    /// dispatch [`ingest_stream`](Self::ingest_stream) performs, factored out so a
    /// streaming reader (the `fold_check` fold-equivalence gate tool) can drive the
    /// identical production fold without buffering the whole stream in memory.
    ///
    /// Records other than `DOMAIN_DATA` / `FRAME_END` (e.g. `DOMAIN_PERF` and the
    /// RC read-probe) carry no folded state and are ignored here, exactly as the
    /// whole-buffer `ingest_stream` loop ignores them. Keeping this the single
    /// dispatch point is what lets the gate tool certify that an optimized
    /// collector folds byte-identically: there is exactly one fold.
    pub fn ingest_record(
        &mut self,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
        record_flags: u32,
    ) {
        if record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            if self.record_is_for_finalized_hour(game_hour_raw) {
                self.record_late_domain_data(game_hour_raw);
            } else {
                foldprof::timed(&foldprof::APPLY_NS, || {
                    self.apply_domain_data(domain_id, payload, record_flags)
                });
            }
        } else if record_type == COMPACT_RECORD_TYPE_FRAME_END {
            self.finish_hour(game_hour_raw);
        }
    }

    /// Drain the finished in-memory snapshots accumulated so far, transferring
    /// ownership to the caller (in-memory fold only). This lets a streaming
    /// consumer hash each hour as its `FRAME_END` closes and then release the
    /// bytes, so a multi-GB stream never forces every per-hour snapshot to be
    /// resident at once (unlike [`finish`](Self::finish), which returns them all).
    /// A file-backed fold spills to disk instead and leaves this empty.
    pub fn take_finished(&mut self) -> Vec<(i64, String, Vec<u8>)> {
        std::mem::take(&mut self.out)
    }

    /// Number of distinct game-hours whose snapshot has been emitted so far
    /// (in-memory or spilled to disk). Authoritative in both fold modes.
    pub fn emitted_hours_count(&self) -> usize {
        self.emitted_hours.len()
    }

    fn note(&mut self, msg: impl Into<String>) {
        self.notes_this_hour.push(msg.into());
    }

    fn mark_domain(&mut self, domain_id: u16) {
        if !self.domains_this_hour.contains(&domain_id) {
            self.domains_this_hour.push(domain_id);
        }
    }

    fn record_domain_problem(&mut self, domain_id: u16, problem: &'static str) {
        if !self
            .domain_problems_this_hour
            .iter()
            .any(|entry| entry.domain_id == domain_id && entry.problem == problem)
        {
            self.domain_problems_this_hour
                .push(DomainProblem { domain_id, problem });
        }
    }

    fn mark_fold_domain_success(&mut self, domain_id: u16) {
        self.successful_fold_domains_seen.insert(domain_id);
    }

    fn mark_fold_domain_unknown(
        &mut self,
        domain_id: u16,
        problem: &'static str,
        note: impl Into<String>,
    ) {
        self.fold_domains_unknown_this_hour.insert(domain_id);
        self.record_domain_problem(domain_id, problem);
        self.note(note);
    }

    fn fold_domain_unknown(&self, domain_id: u16) -> bool {
        self.fold_domains_unknown_this_hour.contains(&domain_id)
    }

    fn fold_domain_seen(&self, domain_id: u16) -> bool {
        self.successful_fold_domains_seen.contains(&domain_id)
    }

    fn handle_compact_failure(
        &mut self,
        domain_id: u16,
        domain_name: &'static str,
        status_flags: u32,
    ) -> bool {
        let failure_flags =
            compact_failure_flags_after_expected_partial_tolerance(domain_id, status_flags);
        if failure_flags != 0 {
            self.mark_fold_domain_unknown(
                domain_id,
                "compact_status_failure",
                format!("{domain_name} state unknown this hour; accumulator retained"),
            );
            true
        } else {
            if compact_expected_partial_tolerance_applies(domain_id, status_flags) {
                if let Some(tolerance) = compact_contract::expected_partial_tolerance(domain_id) {
                    self.record_domain_problem(domain_id, "partial_data");
                    self.note(format!(
                        "{domain_name} expected partial tolerance applied: {}",
                        tolerance.reason
                    ));
                }
            }
            false
        }
    }

    fn reject_delta_before_baseline(
        &mut self,
        domain_id: u16,
        domain_name: &'static str,
        status_flags: u32,
    ) -> bool {
        if (status_flags & COMPACT_STATUS_DELTA_DATA) != 0
            && !self.delta_baselines_seen.contains(&domain_id)
        {
            self.mark_fold_domain_unknown(
                domain_id,
                "delta_before_baseline",
                format!(
                    "{domain_name} delta_before_baseline; owned snapshot leaf unknown this hour"
                ),
            );
            true
        } else {
            false
        }
    }

    fn replace_or_sparse_after_first(&mut self, domain_id: u16, status_flags: u32) -> bool {
        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        if compact_expected_partial_tolerance_applies(domain_id, status_flags) {
            // Expected-partial frames carry valid rows plus known omissions. Treat them as
            // sparse observations so an omitted row is never fabricated as a deletion.
            return true;
        }
        if delta || self.delta_baselines_seen.contains(&domain_id) {
            true
        } else {
            self.delta_baselines_seen.insert(domain_id);
            false
        }
    }

    fn implicit_delta_baseline_or_sparse(&mut self, domain_id: u16) -> bool {
        if self.delta_baselines_seen.insert(domain_id) {
            false
        } else {
            true
        }
    }

    fn record_is_for_finalized_hour(&self, game_hour_raw: i32) -> bool {
        hour_index_from_raw(game_hour_raw)
            .map(|hour_index| self.emitted_hours.contains(&hour_index))
            .unwrap_or(false)
    }

    fn record_late_domain_data(&mut self, game_hour_raw: i32) {
        if let Some(hour_index) = hour_index_from_raw(game_hour_raw) {
            *self
                .late_records_dropped_since_last_snapshot
                .entry(hour_index)
                .or_default() += 1;
        }
    }

    fn reset_hour_state(&mut self) {
        // E13: the measured-country set is per-frame; a new hour has not yet
        // measured it. `tag_by_db_id` itself deliberately PERSISTS (D18 full-
        // replaces it) so TAG re-keying still works when D18 is absent, but the
        // KNOWN flag resets so `measured_countries.known` reflects this hour.
        self.country_identity_known_this_hour = false;
        self.domains_this_hour.clear();
        self.unfolded_domains_this_hour.clear();
        self.domain_problems_this_hour.clear();
        self.notes_this_hour.clear();
        self.combat_detail_events_this_hour.clear();
        self.air_combat_losses_this_hour = None;
        self.air_wing_loadout_orphan_check_keys_this_hour.clear();
        self.division_equipment_touched_this_hour.clear();
        self.division_equipment_deleted_division_keys_this_hour
            .clear();
        self.frd_focus_events_this_hour.clear();
        self.frd_decision_events_this_hour.clear();
        self.frd_depth_badges_this_hour.clear();
        self.frd_decision_targets_unknown.clear();
        self.fold_domains_unknown_this_hour.clear();
        self.land_combats_known_this_hour = false;
        self.land_combats_unknown_note_this_hour = None;
        self.divisions_unknown_this_hour = false;
        self.map_control_unknown_this_hour = false;
        self.occupation_unknown_this_hour = false;
        self.map_detail_unknown_this_hour = false;
        self.domain48_known_this_hour = false;
        self.domain48_unknown_this_hour = false;
        self.domain53_known_this_hour = false;
        self.domain53_country_overlays_this_hour.clear();
        self.domain53_world_overlay_this_hour = Domain53WorldOverlay::default();
        // D60: wars are hour-scoped; the capitulation map deliberately
        // PERSISTS across hours (full-replaced by each proven frame).
        self.capitulation_war_state_known_this_hour = false;
        self.war_state_wars_this_hour.clear();
        self.supply_unknown_this_hour = false;
        self.market_trade_unknown_this_hour = false;
        self.session_multiplayer_metadata_this_hour = None;
        self.army_battleplans_depth_unknown_this_hour = false;
        self.army_battleplans_depth_omitted_non_exact_this_hour = 0;
        self.army_battleplans_depth_path_omitted_this_hour = 0;
        self.naval_combat_lifecycle_known_this_hour = false;
        self.naval_combat_members_known_this_hour = false;
        self.air_aces_fresh_this_hour = false;
    }

    /// Mutable country object for a tag_db_id, created empty on first use.
    fn country_mut(&mut self, tag_db_id: u32) -> &mut Map<String, Value> {
        self.countries.entry(tag_db_id.to_string()).or_default()
    }

    /// Resolve a numeric tag_db_id key ("47") to its TAG string ("GER"). Falls
    /// back to the numeric key (recording the gap in `unresolved`) when the id is
    /// not a parseable u32 or has no country_identity mapping. Never fabricates.
    fn resolve_country_key(
        &self,
        numeric_key: &str,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) -> String {
        match numeric_key.parse::<u32>() {
            Ok(id) => match self.tag_by_db_id.get(&id) {
                Some(tag) => tag.clone(),
                None => {
                    unresolved.insert(id);
                    numeric_key.to_string()
                }
            },
            // A non-numeric key is already a TAG (or otherwise resolved); pass through.
            Err(_) => numeric_key.to_string(),
        }
    }

    /// E13: the measured-country roster for this frame — the set of countries
    /// the capture RESOLVED (domain 18's full-replace map). `known` is false
    /// when no proven domain-18 record landed this hour (the measured set is
    /// UNKNOWN, not empty). When known, `count`/`tags`/`tag_db_ids` enumerate
    /// the measured population so downstream can tell a country that was
    /// measured-but-has-no-data-in-a-domain from one that was NOT resolved this
    /// frame (absent from `tag_db_ids`).
    fn build_measured_countries(&self) -> Value {
        if !self.country_identity_known_this_hour {
            return json!({
                "known": false,
                "source": "country_identity",
                "reason": "no proven country_identity (domain 18) record this frame; \
                           the measured-country set is unknown (not empty)",
            });
        }
        let tag_db_ids: Vec<u32> = self.tag_by_db_id.keys().copied().collect();
        let tags: Vec<&str> = self.tag_by_db_id.values().map(String::as_str).collect();
        json!({
            "known": true,
            "source": "country_identity",
            "count": tag_db_ids.len(),
            "tag_db_ids": tag_db_ids,
            "tags": tags,
            "note": "the countries resolved this frame (systemic country-roots bound; \
                     dynamic observers e.g. D01/D02/D03 are excluded). A country ABSENT \
                     from a domain's rows but PRESENT here has no data for that domain; \
                     a country absent here was not resolved this frame.",
        })
    }

    /// Rewrite a country-valued JSON string field ("47" -> "GER") in place. Null
    /// or non-string values are left untouched; unmapped ids stay numeric and are
    /// recorded in `unresolved`.
    fn resolve_country_value(
        &self,
        value: Option<&mut Value>,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) {
        if let Some(Value::String(s)) = value {
            let resolved = self.resolve_country_key(s, unresolved);
            *s = resolved;
        }
    }

    fn resolve_country_fields(
        &self,
        country: &mut Map<String, Value>,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) {
        if let Some(Value::Object(diplomacy)) = country.get_mut("diplomacy") {
            if let Some(Value::Array(lend_lease)) = diplomacy.get_mut("lend_lease") {
                self.resolve_contract_list(lend_lease, unresolved);
            }
        }
    }

    fn resolve_contract_list(
        &self,
        contracts: &mut [Value],
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) {
        for contract in contracts {
            if let Value::Object(obj) = contract {
                self.resolve_contract_fields(obj, unresolved);
            }
        }
    }

    fn resolve_contract_fields(
        &self,
        contract: &mut Map<String, Value>,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) {
        for key in ["seller", "buyer", "counterpart", "delivery_country"] {
            self.resolve_country_value(contract.get_mut(key), unresolved);
        }
    }

    fn project_market_trade_lend_lease(
        &self,
        countries: &mut Map<String, Value>,
        contracts: &[Value],
        unknown: bool,
        unresolved: &mut BTreeSet<u32>,
    ) {
        for contract in contracts {
            let Value::Object(contract_obj) = contract else {
                continue;
            };
            let Some(seller) = contract_obj.get("seller").and_then(Value::as_str) else {
                continue;
            };
            let Some(buyer) = contract_obj.get("buyer").and_then(Value::as_str) else {
                continue;
            };
            for (country_ref, counterpart, direction) in
                [(seller, buyer, "outgoing"), (buyer, seller, "incoming")]
            {
                let country_key = self.resolve_country_key(country_ref, unresolved);
                let country = countries
                    .entry(country_key)
                    .or_insert_with(|| Value::Object(Map::new()));
                if !country.is_object() {
                    *country = Value::Object(Map::new());
                }
                let Value::Object(country) = country else {
                    unreachable!("country projection entry was just made object");
                };
                let diplomacy = ensure_json_object_mut(country, "diplomacy");
                if unknown {
                    diplomacy.insert("lend_lease".into(), Value::Null);
                    continue;
                }
                let mut projection = contract_obj.clone();
                projection.insert("direction".into(), json!(direction));
                projection.insert("counterpart".into(), json!(counterpart));
                projection.insert("r2_market_trade_projection".into(), json!(true));
                ensure_json_array_mut(diplomacy, "lend_lease").push(Value::Object(projection));
            }
        }
    }

    fn project_market_trade_resource_flow(
        &self,
        countries: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        for (tag_db_id, values_raw) in &self.market_trade_resource_exported {
            let country = ensure_country_projection_object(countries, self, *tag_db_id, unresolved);
            let market = ensure_json_object_mut(country, "r2_market_trade_live");
            let resource_flow = ensure_json_object_mut(market, "resource_flow");
            let mut exported = Map::new();
            let mut exported_raw = Map::new();
            for (index, resource_name) in MARKET_TRADE_RESOURCE_NAMES.iter().enumerate() {
                let value_raw = values_raw[index];
                if value_raw != 0 {
                    exported.insert(
                        (*resource_name).into(),
                        json!(value_raw as f64 / FIXED_SCALE),
                    );
                    exported_raw.insert((*resource_name).into(), json!(value_raw));
                }
            }
            resource_flow.insert("exported".into(), Value::Object(exported));
            resource_flow.insert("exported_raw".into(), Value::Object(exported_raw));
            resource_flow.insert("scale_raw".into(), json!(FIXED_SCALE as u32));
            resource_flow.insert("source".into(), json!("CCountryResources+0x5B0"));
            resource_flow.insert("confidence".into(), json!("validated_slice"));
        }
    }

    fn resolve_occupation_fields(
        &self,
        state: &mut Map<String, Value>,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) {
        if let Some(Value::Object(occupation)) = state.get_mut("occupation") {
            for key in ["occupier_tag", "occupied_country_tag"] {
                self.resolve_country_value(occupation.get_mut(key), unresolved);
            }
        }
    }

    fn resolve_supply_fields(
        &self,
        supply: &mut Map<String, Value>,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) {
        let Some(Value::Object(by_province)) = supply.get_mut("by_province") else {
            return;
        };
        for province in by_province.values_mut() {
            let Value::Object(province) = province else {
                continue;
            };
            let Some(Value::Array(divisions)) = province.get_mut("divisions") else {
                continue;
            };
            for division in divisions {
                if let Value::Object(division) = division {
                    self.resolve_country_value(division.get_mut("owner"), unresolved);
                }
            }
        }
    }

    fn build_divisions(&self, unresolved: &mut BTreeSet<u32>) -> (Vec<Value>, Vec<String>) {
        let mut division_objects = self.divisions.clone();
        let mut join_index: BTreeMap<DivisionEquipmentKey, Vec<String>> = BTreeMap::new();
        for (uid, division) in &division_objects {
            if let Some(key) = division_equipment_join_key(division) {
                join_index.entry(key).or_default().push(uid.clone());
            }
        }

        let mut unmatched_touched = 0usize;
        let mut ambiguous_touched = 0usize;
        if self.fold_domain_unknown(DIVISION_EQUIPMENT_DOMAIN_ID) {
            if self.fold_domain_seen(DIVISION_EQUIPMENT_DOMAIN_ID)
                || !self.division_equipment.is_empty()
            {
                for division in division_objects.values_mut() {
                    division.insert("equipment".into(), Value::Null);
                }
            }
        } else {
            for (key, equipment) in &self.division_equipment {
                match join_index.get(key).map(Vec::as_slice) {
                    Some([uid]) => {
                        if let Some(division) = division_objects.get_mut(uid) {
                            division.insert(
                                "equipment".into(),
                                Value::Array(
                                    equipment
                                        .iter()
                                        .map(DivisionEquipmentEntryState::to_json)
                                        .collect(),
                                ),
                            );
                        }
                    }
                    Some(matches) if matches.len() > 1 => {
                        if self.division_equipment_touched_this_hour.contains(key) {
                            ambiguous_touched = ambiguous_touched.saturating_add(1);
                        }
                    }
                    _ => {
                        if self.division_equipment_touched_this_hour.contains(key) {
                            unmatched_touched = unmatched_touched.saturating_add(1);
                        }
                    }
                }
            }
        }

        let mut notes = Vec::new();
        if unmatched_touched > 0 || ambiguous_touched > 0 {
            notes.push(format!(
                "division_equipment_live dropped {unmatched_touched} unmatched and {ambiguous_touched} ambiguous owner/division join row(s)"
            ));
        }

        let divisions = division_objects
            .values_mut()
            .map(|out| {
                self.resolve_country_value(out.get_mut("owner"), unresolved);
                Value::Object(out.clone())
            })
            .collect();
        (divisions, notes)
    }

    fn project_frd_annotations(
        &self,
        countries: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        for (tag_db_id, metrics) in &self.frd_focus_metrics {
            let country = ensure_country_projection_object(countries, self, *tag_db_id, unresolved);
            let focus = ensure_json_object_mut(country, "focus");
            // D12 owns the first-class focus fields. D45 provides an absent-only
            // fallback for depth-only snapshots and always mirrors the value in
            // R2 sidecars so ownership remains explicit.
            if let Some(current) = &metrics.current {
                focus.entry("current").or_insert_with(|| json!(current));
                focus.insert("_r2_depth_current".into(), json!(current));
            }
            if let Some(current_continuous) = &metrics.current_continuous {
                focus
                    .entry("current_continuous")
                    .or_insert_with(|| json!(current_continuous));
                focus.insert(
                    "_r2_depth_current_continuous".into(),
                    json!(current_continuous),
                );
            }
            if let Some(completed_count) = metrics.completed_count {
                focus
                    .entry("completed_count")
                    .or_insert_with(|| json!(completed_count));
                focus.insert("_r2_depth_completed_count".into(), json!(completed_count));
            }
            if !metrics.completed_tokens.is_empty() {
                focus.insert(
                    "_r2_depth_completed".into(),
                    Value::Array(
                        metrics
                            .completed_tokens
                            .iter()
                            .map(|token| json!(token))
                            .collect(),
                    ),
                );
            }
        }

        for (tag_db_id, events) in &self.frd_focus_events_this_hour {
            let country = ensure_country_projection_object(countries, self, *tag_db_id, unresolved);
            ensure_json_object_mut(country, "focus").insert(
                "_r2_depth_events".into(),
                Value::Array(
                    events
                        .iter()
                        .map(FrdFocusDepthEventState::to_json)
                        .collect(),
                ),
            );
        }

        let mut research_by_tag: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
        for (key, row) in &self.frd_research_live {
            research_by_tag
                .entry(key.tag_db_id)
                .or_default()
                .push(row.to_json());
        }
        for (tag_db_id, rows) in research_by_tag {
            let country = ensure_country_projection_object(countries, self, tag_db_id, unresolved);
            ensure_json_object_mut(country, "research")
                .insert("_r2_depth_in_progress_live".into(), Value::Array(rows));
        }

        for (tag_db_id, status) in &self.frd_decision_status {
            let country = ensure_country_projection_object(countries, self, *tag_db_id, unresolved);
            ensure_json_object_mut(country, "decisions")
                .insert("_r2_depth_status".into(), status.to_json());
        }
        for (tag_db_id, targets) in &self.frd_decision_targets {
            let country = ensure_country_projection_object(countries, self, *tag_db_id, unresolved);
            ensure_json_object_mut(country, "decisions").insert(
                "_r2_depth_targets".into(),
                Value::Array(
                    targets
                        .iter()
                        .map(|row| row.to_json(self, unresolved))
                        .collect(),
                ),
            );
        }
        for tag_db_id in &self.frd_decision_targets_unknown {
            let country = ensure_country_projection_object(countries, self, *tag_db_id, unresolved);
            ensure_json_object_mut(country, "decisions")
                .insert("_r2_depth_targets".into(), Value::Null);
        }
        for (tag_db_id, events) in &self.frd_decision_events_this_hour {
            let country = ensure_country_projection_object(countries, self, *tag_db_id, unresolved);
            ensure_json_object_mut(country, "decisions").insert(
                "_r2_depth_events".into(),
                Value::Array(events.iter().map(FrdDecisionEventState::to_json).collect()),
            );
        }
    }

    fn build_world_value(&self) -> Value {
        if self.fold_domain_unknown(SCRIPTED_STATE_DOMAIN_ID) {
            if self.fold_domain_seen(SCRIPTED_STATE_DOMAIN_ID)
                || !self.scripted_world_flags.is_empty()
                || !self.scripted_world_variables.is_empty()
                || !self.scripted_world_token_only_flags.is_empty()
                || !self.scripted_world_opaque_badges.is_empty()
                || !self.scripted_world_delayed_events.is_empty()
            {
                return json!({
                    "flags": Value::Null,
                    "variables": Value::Null,
                    "_r2_token_only_flags": Value::Null,
                    "_r2_scripted_state_badges": Value::Null,
                    "_r2_delayed_events": Value::Null,
                });
            }
            return json!({});
        }

        let mut world = Map::new();
        if self.fold_domain_seen(SCRIPTED_STATE_DOMAIN_ID) || !self.scripted_world_flags.is_empty()
        {
            world.insert(
                "flags".into(),
                Value::Object(script_flag_map_json(&self.scripted_world_flags)),
            );
        }
        if self.fold_domain_seen(SCRIPTED_STATE_DOMAIN_ID)
            || !self.scripted_world_variables.is_empty()
        {
            world.insert(
                "variables".into(),
                Value::Object(f64_map_json(&self.scripted_world_variables)),
            );
        }
        if !self.scripted_world_token_only_flags.is_empty() {
            world.insert(
                "_r2_token_only_flags".into(),
                Value::Object(token_flag_map_json(&self.scripted_world_token_only_flags)),
            );
        }
        if !self.scripted_world_opaque_badges.is_empty() {
            world.insert(
                "_r2_scripted_state_badges".into(),
                Value::Array(
                    self.scripted_world_opaque_badges
                        .iter()
                        .map(ScriptedOpaqueBadgeState::to_json)
                        .collect(),
                ),
            );
        }
        if !self.scripted_world_delayed_events.is_empty() {
            world.insert(
                "_r2_delayed_events".into(),
                Value::Array(
                    self.scripted_world_delayed_events
                        .values()
                        .map(DelayedEventFoldState::to_json)
                        .collect(),
                ),
            );
        }
        Value::Object(world)
    }

    fn build_special_projects_value(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        if self.fold_domain_unknown(SPECIAL_PROJECTS_DOMAIN_ID) {
            return Value::Null;
        }
        self.special_projects
            .as_ref()
            .map(|projects| projects.to_json(self, unresolved))
            .unwrap_or(Value::Null)
    }

    fn build_domain57_session_sidecar(&self) -> Option<Value> {
        let state = self.session_multiplayer_metadata_this_hour.as_ref()?;
        let players_by_country = if let Some(rows) = &state.players_by_country {
            let mut rows = rows.clone();
            rows.sort_by(|a, b| {
                a.country_tag
                    .cmp(&b.country_tag)
                    .then(a.tag_db_id.cmp(&b.tag_db_id))
            });
            json!({
                "state": "known",
                "rows": rows
                    .iter()
                    .map(|row| {
                        json!({
                            "country_tag": row.country_tag.as_str(),
                            "tag_db_id": row.tag_db_id,
                            "is_human_controlled": true,
                            "human_marker": row.human_marker,
                            "human_name": { "state": "not_sourced" },
                            "player_key_hash": { "state": "not_sourced" },
                            "session_slot": { "state": "not_sourced" },
                            "host": { "state": "not_sourced" },
                            "connected": { "state": "not_sourced" },
                            "valid_from_hour_raw": row.valid_from_hour_raw,
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        } else {
            json!({
                "state": "unavailable",
                "rows": [],
            })
        };
        let pause = match state.pause_is_paused {
            Some(is_paused) => json!({
                "state": "known",
                "is_paused": is_paused,
            }),
            None => json!({
                "state": "unavailable",
                "is_paused": Value::Null,
            }),
        };
        Some(json!({
            "players_by_country": players_by_country,
            "pause": pause,
        }))
    }

    // W6 D68: build the top-level `session_header` value from the carried
    // last-value-wins header. Returns None (attach nothing) until at least one
    // header field has ever been folded. player is emitted with both its
    // tag_db_id and resolved TAG (an unmapped id stays numeric and is recorded
    // in `unresolved`). start_date carries the raw hour-count; the replay derives
    // the calendar date with the shared codec.
    fn build_session_header_value(
        &self,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) -> Option<Value> {
        let header = self.session_header.as_ref()?;
        let player = match header.player_tag_db_id {
            Some(id) => {
                let tag = self.resolve_country_key(&id.to_string(), unresolved);
                json!({ "tag_db_id": id, "country_tag": tag })
            }
            None => Value::Null,
        };
        Some(json!({
            "player": player,
            "ideology": header.ideology.clone().map(Value::String).unwrap_or(Value::Null),
            "cosmetic_tag": header.cosmetic_tag.clone().map(Value::String).unwrap_or(Value::Null),
            "dlcs": header.dlcs.map(|v| json!(v)).unwrap_or(Value::Null),
            "start_date_raw": header.start_date_raw.map(|v| json!(v)).unwrap_or(Value::Null),
            "save_version": header.save_version.map(|v| json!(v)).unwrap_or(Value::Null),
            "session": header.session.map(|v| json!(v)).unwrap_or(Value::Null),
            "version": header.version.clone().map(Value::String).unwrap_or(Value::Null),
        }))
    }

    // W6 D69: build the top-level `tech_sharing_group` value from the carried
    // group set. Returns None until a proven frame has folded (a proven-empty
    // frame folds Some([])). Each group carries its id token and its member
    // set (both tag_db_ids and resolved TAGs; unmapped ids stay numeric and are
    // recorded in `unresolved`).
    fn build_tech_sharing_groups_value(
        &self,
        unresolved: &mut std::collections::BTreeSet<u32>,
    ) -> Option<Value> {
        let groups = self.tech_sharing_groups.as_ref()?;
        let rows: Vec<Value> = groups
            .iter()
            .map(|group| {
                let member_tags: Vec<String> = group
                    .member_tag_db_ids
                    .iter()
                    .map(|id| self.resolve_country_key(&id.to_string(), unresolved))
                    .collect();
                json!({
                    "id_token": group.id_token,
                    "member_tag_db_ids": group.member_tag_db_ids,
                    "members": member_tags,
                })
            })
            .collect();
        Some(json!({
            "count": rows.len(),
            "groups": rows,
        }))
    }

    fn apply_domain_data(&mut self, domain_id: u16, payload: &[u8], record_flags: u32) {
        let result = match domain_id {
            10 => self.fold_fuel(payload),
            11 => self.fold_production(payload),
            12 => self.fold_focus(payload),
            13 => self.fold_tech(payload),
            14 => self.fold_politics(payload),
            15 => self.fold_politics_residuals(payload),
            16 => self.fold_decisions(payload),
            17 => self.fold_diplomacy(payload),
            18 => self.fold_country_identity(payload),
            20 => self.fold_divisions(payload),
            21 => self.fold_land_combats(payload),
            22 => self.fold_map_control(payload),
            23 => self.fold_map_detail(payload),
            24 => self.fold_weather(payload),
            26 => self.fold_division_equipment(payload),
            27 => self.fold_army_orders(payload),
            28 => self.fold_oob_command(payload),
            29 => self.fold_ships(payload),
            30 => self.fold_air_wings(payload),
            31 => self.fold_air_region_activity(payload),
            32 => self.fold_air_wing_loadouts(payload),
            33 => self.fold_air_variant_defs(payload, AIR_VARIANT_DEFS_DOMAIN_ID),
            34 => self.fold_air_base_status(payload),
            35 => self.fold_air_variant_stats(payload),
            36 => self.fold_air_variant_reserves(payload),
            37 => self.fold_air_variant_defs(payload, STOCKPILE_VARIANT_DEFS_DOMAIN_ID),
            38 => self.fold_air_superiority_inputs(payload),
            39 => self.fold_variant_upgrades(payload, AIR_VARIANT_UPGRADES_DOMAIN_ID),
            40 => self.fold_variant_upgrades(payload, STOCKPILE_VARIANT_UPGRADES_DOMAIN_ID),
            41 => self.fold_special_projects(payload),
            42 => self.fold_air_combat_losses(payload),
            61 => self.fold_air_loss_stats(payload),
            62 => self.fold_hub_motorization(payload),
            43 => self.fold_market_trade(payload),
            44 => self.fold_supply_logistics_attrition(payload),
            45 => self.fold_focus_research_decisions_depth(payload, record_flags),
            46 => self.fold_air_variant_archetypes(payload),
            47 => self.fold_characters_leaders_cabinet_live(payload),
            48 => self.fold_construction_buildings_state_damage(payload),
            51 => self.fold_scripted_state(payload),
            52 => self.fold_occupation_resistance_compliance(payload),
            53 => self.fold_diplomacy_intel_operatives(payload),
            54 => self.fold_army_battleplans_depth(payload),
            55 => self.fold_land_combat_detail_events(payload),
            49 => self.fold_naval_combat_lifecycle(payload),
            50 => self.fold_naval_combat_members(payload),
            56 => self.fold_naval_missions_status(payload),
            57 => self.fold_session_multiplayer_metadata(payload),
            63 => self.fold_country_core_scalars(payload),
            64 => self.fold_delayed_events(payload),
            65 => self.fold_special_project_programs(payload),
            66 => self.fold_occupation_state_extras(payload),
            67 => self.fold_country_dynamic_modifiers(payload),
            68 => self.fold_session_header(payload),
            69 => self.fold_tech_sharing_groups(payload),
            60 => self.fold_capitulation_war_state(payload),
            _ => {
                self.unfolded_domains_this_hour.insert(domain_id);
                return;
            }
        };
        match result {
            Ok(()) => self.mark_domain(domain_id),
            Err(err) => self.note(format!("domain {domain_id} fold skipped: {err}")),
        }
    }

    fn finish_hour(&mut self, game_hour_raw: i32) {
        let Some(hour_index) = hour_index_from_raw(game_hour_raw) else {
            // fail closed: an out-of-range game hour cannot key a snapshot
            self.note(format!(
                "frame_end skipped: game_hour_raw {game_hour_raw} out of range"
            ));
            self.reset_hour_state();
            return;
        };
        if self.emitted_hours.contains(&hour_index) {
            self.note(format!("frame_end duplicate for hour_index {hour_index}"));
            self.reset_hour_state();
            return;
        }
        let date = date_from_raw(game_hour_raw).unwrap_or_default();
        // Hour STATE transitions run unconditionally (also under a stride that
        // does not own this hour): the prunes/prepare mutate accumulators that
        // later hours' folds depend on. `build_snapshot` is `&self`-pure, so it
        // (and emission) is the only part a non-owning strided fold may skip.
        self.prune_division_equipment_for_deleted_divisions_current_hour();
        self.prune_air_wing_loadout_orphans_for_current_hour();
        self.prepare_domain48_for_snapshot();
        // Emission ordinal of this hour: how many hours closed before it. The
        // insert below is unconditional, so the ordinal sequence is identical
        // across all strided folds of the same stream (determinism anchor).
        let ordinal = self.emitted_hours.len() as u64;
        if let Some(sink) = &self.file_sink {
            let owns_hour = ordinal % sink.stride == sink.stride_offset;
            let path = sink.dir.join(format!("{hour_index:012}.snapshot.json"));
            if owns_hour {
                let snapshot = foldprof::timed(&foldprof::BUILD_NS, || {
                    self.build_snapshot(hour_index, &date)
                });
                // Hand the owned snapshot `Value` to the background emit worker,
                // which serializes + writes + hashes it off the critical path
                // (overlapping the next hour's build). Ownership moves — no copy.
                self.file_sink
                    .as_mut()
                    .expect("file sink present")
                    .pipeline
                    .submit(SnapshotEmitJob {
                        hour_index,
                        date,
                        path,
                        snapshot,
                    });
            }
        } else {
            // In-memory fold: build + serialize inline as before, retain bytes.
            let snapshot = foldprof::timed(&foldprof::BUILD_NS, || {
                self.build_snapshot(hour_index, &date)
            });
            let bytes = foldprof::timed(&foldprof::SERIALIZE_NS, || {
                serde_json::to_vec(&snapshot).expect("snapshot serializes")
            });
            foldprof::add_bytes(bytes.len());
            self.out.push((hour_index, date, bytes));
        }
        self.emitted_hours.insert(hour_index);
        self.late_records_dropped_since_last_snapshot.clear();
        self.reset_hour_state();
    }

    fn prepare_domain48_for_snapshot(&mut self) {
        if !self.domain48_known_this_hour {
            self.clear_domain48_extras();
        }
        if self.domain48_unknown_this_hour {
            self.note(
                "construction_buildings_state_damage_live unknown this hour; domain extras omitted",
            );
        }
    }

    fn prune_division_equipment_for_deleted_divisions_current_hour(&mut self) {
        if self
            .division_equipment_deleted_division_keys_this_hour
            .is_empty()
            || !self.domains_this_hour.contains(&DIVISIONS_DOMAIN_ID)
            || self.divisions_unknown_this_hour
        {
            return;
        }

        let mut dropped = 0usize;
        for key in self
            .division_equipment_deleted_division_keys_this_hour
            .iter()
            .cloned()
            .collect::<Vec<_>>()
        {
            if self.division_equipment.remove(&key).is_some() {
                dropped = dropped.saturating_add(1);
            }
        }
        if dropped > 0 {
            self.note(format!(
                "division_equipment_live pruned {dropped} row(s) after D20 division deletion"
            ));
        }
    }

    fn prune_air_wing_loadout_orphans_for_current_hour(&mut self) {
        if self.air_wing_loadout_orphan_check_keys_this_hour.is_empty()
            || !self.fold_domain_seen(AIR_WINGS_DOMAIN_ID)
            || self.fold_domain_unknown(AIR_WINGS_DOMAIN_ID)
        {
            return;
        }

        let mut dropped = 0usize;
        for key in self
            .air_wing_loadout_orphan_check_keys_this_hour
            .iter()
            .cloned()
            .collect::<Vec<_>>()
        {
            if self.air_wings.contains_key(&key) {
                continue;
            }
            if self.air_wing_loadouts.remove(&key).is_some() {
                dropped = dropped.saturating_add(1);
            }
        }
        if dropped > 0 {
            self.note(format!(
                "air_wing_loadouts dropped {dropped} orphan row(s) without a D30 air_wings owner row"
            ));
        }
    }

    fn build_snapshot(&self, hour_index: i64, date: &str) -> Value {
        // Track every tag_db_id we key or reference this hour that has no TAG
        // mapping, so we never claim full resolution when a gap exists.
        let mut unresolved: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        let have_map = !self.tag_by_db_id.is_empty();

        // Countries: re-key numeric-id keys by TAG when resolvable. A country whose
        // id has no mapping keeps its numeric key (never fabricate a TAG, §0.2).
        let mut countries = Map::new();
        for (numeric_key, obj) in &self.countries {
            let key = self.resolve_country_key(numeric_key, &mut unresolved);
            let mut out = obj.clone();
            self.resolve_country_fields(&mut out, &mut unresolved);
            if self.fold_domain_unknown(SCRIPTED_STATE_DOMAIN_ID) {
                mask_scripted_country_extra(&mut out);
            }
            // Wave-4 cosmetic tag (D18): attach countries[*].cosmetic_tag from
            // the D18 cosmetic map. Present in the map => Some(tag)->string,
            // None->null; a country ABSENT from the map (unreadable cosmetic
            // field, or no D18 record yet) gets NO cosmetic_tag key at all
            // (absent != null != empty).
            if let Ok(id) = numeric_key.parse::<u32>() {
                if let Some(cosmetic) = self.cosmetic_by_db_id.get(&id) {
                    out.insert(
                        "cosmetic_tag".into(),
                        match cosmetic {
                            Some(tag) => json!(tag),
                            None => Value::Null,
                        },
                    );
                }
            }
            countries.insert(key, Value::Object(out));
        }
        if !self.fold_domain_unknown(FOCUS_RESEARCH_DECISIONS_DEPTH_DOMAIN_ID) {
            self.project_frd_annotations(&mut countries, &mut unresolved);
        }
        if self.domain53_known_this_hour {
            self.project_domain53_country_overlays(&mut countries, &mut unresolved);
            self.project_domain53_faction_memberships(&mut countries, &mut unresolved);
        }
        // D60 capitulation attaches from its persistent map (last proven frame
        // wins), creating country objects for capitulated minors if needed.
        self.project_capitulation_country_state(&mut countries, &mut unresolved);
        self.project_market_trade_resource_flow(&mut countries, &mut unresolved);

        // States: merge sibling-domain fields with D22-owned map-control fields.
        // Unknown domains mask only their own fields, not the whole shared object.
        let mut states = Map::new();
        let state_keys: BTreeSet<String> = self
            .states
            .keys()
            .chain(self.map_control_states.keys())
            .cloned()
            .collect();
        for sid in state_keys {
            let mut out = self.states.get(&sid).cloned().unwrap_or_default();
            if let Some(map_control) = self.map_control_states.get(&sid) {
                for key in ["owner", "controller"] {
                    if let Some(value) = map_control.get(key) {
                        out.insert(
                            key.into(),
                            if self.map_control_unknown_this_hour {
                                Value::Null
                            } else {
                                value.clone()
                            },
                        );
                    }
                }
            }
            if !self.map_control_unknown_this_hour {
                self.resolve_country_value(out.get_mut("owner"), &mut unresolved);
                self.resolve_country_value(out.get_mut("controller"), &mut unresolved);
            }
            if self.occupation_unknown_this_hour {
                if out.contains_key("occupation") {
                    out.insert("occupation".into(), Value::Null);
                }
                if out.contains_key("occupation_law") {
                    out.insert("occupation_law".into(), Value::Null);
                }
            } else {
                self.resolve_occupation_fields(&mut out, &mut unresolved);
            }
            if self.map_detail_unknown_this_hour {
                for key in ["resources", "buildings", "province_buildings"] {
                    if out.contains_key(key) {
                        out.insert(key.into(), Value::Null);
                    }
                }
            }
            if self.fold_domain_unknown(SCRIPTED_STATE_DOMAIN_ID) {
                if out.contains_key("_r2_scripted_state") {
                    out.insert("_r2_scripted_state".into(), Value::Null);
                }
            }
            states.insert(sid, Value::Object(out));
        }

        // Provinces: merge sibling-domain fields with D22-owned map-control fields.
        let mut provinces = Map::new();
        let province_keys: BTreeSet<String> = self
            .provinces
            .keys()
            .chain(self.map_control_provinces.keys())
            .cloned()
            .collect();
        for pid in province_keys {
            let mut out = self.provinces.get(&pid).cloned().unwrap_or_default();
            if let Some(map_control) = self.map_control_provinces.get(&pid) {
                for key in ["controller", "state_id"] {
                    if let Some(value) = map_control.get(key) {
                        out.insert(
                            key.into(),
                            if self.map_control_unknown_this_hour {
                                Value::Null
                            } else {
                                value.clone()
                            },
                        );
                    }
                }
            }
            if !self.map_control_unknown_this_hour {
                self.resolve_country_value(out.get_mut("controller"), &mut unresolved);
            }
            provinces.insert(pid, Value::Object(out));
        }

        // Divisions: merge D26 equipment only when owner/div_id joins exactly
        // one live D20 division, then re-key owner country value by TAG.
        let (divisions, division_equipment_notes) = self.build_divisions(&mut unresolved);
        let (combats, dropped_detail_event_keys) = self.build_combats();

        // map layer mirrors (raster-renderer convenience, §8), built from the
        // already-resolved state/province objects so they carry TAG values too.
        let mut owner_by_state = Map::new();
        if !self.map_control_unknown_this_hour {
            for (sid, obj) in states.iter() {
                if let Some(owner) = obj.get("owner") {
                    owner_by_state.insert(sid.clone(), owner.clone());
                }
            }
        }
        let mut control_by_province = Map::new();
        if !self.map_control_unknown_this_hour {
            for (pid, obj) in provinces.iter() {
                if let Some(controller) = obj.get("controller") {
                    control_by_province.insert(pid.clone(), controller.clone());
                }
            }
        }

        let mut diplomacy_world = Map::new();
        // D60: `wars` is null (unknown) until a proven capitulation_war_state
        // frame lands this hour; a proven frame with zero wars is a genuine
        // empty object. Never fabricate emptiness from absence.
        if self.capitulation_war_state_known_this_hour {
            diplomacy_world.insert("wars".into(), self.build_war_state_wars(&mut unresolved));
        } else {
            diplomacy_world.insert("wars".into(), Value::Null);
        }
        diplomacy_world.insert("factions".into(), json!({}));
        diplomacy_world.insert("subjects".into(), json!([]));
        if self.domain53_known_this_hour {
            self.project_domain53_diplomacy_world(&mut diplomacy_world, &mut unresolved);
        }
        if self.market_trade_unknown_this_hour {
            diplomacy_world.insert("trade_contracts".into(), Value::Null);
            if let Some(contracts) = &self.market_trade_contracts {
                self.project_market_trade_lend_lease(
                    &mut countries,
                    contracts,
                    true,
                    &mut unresolved,
                );
            }
        } else if let Some(contracts) = &self.market_trade_contracts {
            let mut contracts = contracts.clone();
            self.resolve_contract_list(&mut contracts, &mut unresolved);
            if !contracts.is_empty() {
                self.project_market_trade_lend_lease(
                    &mut countries,
                    &contracts,
                    false,
                    &mut unresolved,
                );
                diplomacy_world.insert("trade_contracts".into(), Value::Array(contracts));
            }
        }

        let mut supply_value = if self.supply_unknown_this_hour {
            json!({
                "by_province": Value::Null,
                "source": Value::Null,
                "confidence": Value::Null,
                "r2_logistics_attrition": Value::Null,
            })
        } else if let Some(supply) = &self.supply {
            let mut out = supply.clone();
            self.resolve_supply_fields(&mut out, &mut unresolved);
            Value::Object(out)
        } else {
            json!({})
        };
        if let Value::Object(supply) = &mut supply_value {
            self.insert_hub_motorization(supply, &mut unresolved);
        }

        let mut domains: Vec<Value> = self
            .domains_this_hour
            .iter()
            .map(|id| json!(compact_contract::wire_name_for_id(*id).unwrap_or("unknown")))
            .collect();
        domains.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));

        let mut notes = self.notes_this_hour.clone();
        notes.extend(division_equipment_notes);
        if self.divisions_unknown_this_hour {
            notes.push("divisions state unknown this hour; accumulator retained for the next valid baseline/delta".to_string());
        }
        if self.map_control_unknown_this_hour {
            notes.push("map_control state unknown this hour; accumulator retained for the next valid baseline/delta".to_string());
        }
        if self.occupation_unknown_this_hour {
            notes.push("occupation_resistance_compliance_live state unknown this hour; accumulator retained for the next valid baseline/delta".to_string());
        }
        if self.map_detail_unknown_this_hour {
            notes.push("map_detail state unknown this hour; accumulator retained for the next valid baseline/delta".to_string());
        }
        if self.supply_unknown_this_hour {
            notes.push("supply_logistics_attrition_live state unknown this hour; accumulator retained for the next valid frame".to_string());
        }
        if self.market_trade_unknown_this_hour {
            notes.push("market_trade_live state unknown this hour; accumulator retained for the next valid frame".to_string());
        }
        if let Some(orphan_rows) = self
            .naval_missions_status_orphan_rows()
            .filter(|count| *count > 0)
        {
            notes.push(format!(
                "naval_missions_status_live kept {orphan_rows} task-force status row(s) without a D28 OOB task-force owner row under order_of_battle.extra"
            ));
        }
        if let Some(note) = &self.land_combats_unknown_note_this_hour {
            notes.push(note.clone());
        }
        if !dropped_detail_event_keys.is_empty() {
            let keys: Vec<String> = dropped_detail_event_keys
                .iter()
                .map(|(combat_id, generation)| format!("{combat_id}:{generation}"))
                .collect();
            notes.push(format!(
                "land_combat_detail_events_live dropped for unknown active combat key(s): {}",
                keys.join(",")
            ));
        }
        if !self.late_records_dropped_since_last_snapshot.is_empty() {
            let drops: Vec<String> = self
                .late_records_dropped_since_last_snapshot
                .iter()
                .map(|(hour_index, count)| format!("{hour_index}:{count}"))
                .collect();
            notes.push(format!(
                "late domain data records dropped for already-finalized hour(s): {}",
                drops.join(",")
            ));
        }

        let mut domain_problems: Vec<Value> = self
            .domain_problems_this_hour
            .iter()
            .map(|problem| {
                json!({
                    "domain_id": problem.domain_id,
                    "domain": domain_name(problem.domain_id),
                    "problem": problem.problem,
                })
            })
            .collect();
        let mut push_problem = |domain_id: u16, problem: &'static str| {
            if !domain_problems.iter().any(|entry| {
                entry["domain_id"] == json!(domain_id) && entry["problem"] == json!(problem)
            }) {
                domain_problems.push(json!({
                    "domain_id": domain_id,
                    "domain": domain_name(domain_id),
                    "problem": problem,
                }));
            }
        };
        if self.domain53_known_this_hour {
            if self.domain53_world_overlay_this_hour.faction_roster_status
                == FactionRosterStatus::Unavailable
            {
                push_problem(
                    DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
                    "faction_roster_unavailable",
                );
            }
            if !self.domain53_faction_member_conflicts().is_empty() {
                push_problem(
                    DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
                    "faction_member_conflict",
                );
            }
        }
        if !self.naval_combat_lifecycle_known_this_hour
            && self.fold_domain_seen(NAVAL_COMBAT_LIFECYCLE_DOMAIN_ID)
            && !self
                .domains_this_hour
                .contains(&NAVAL_COMBAT_LIFECYCLE_DOMAIN_ID)
        {
            push_problem(
                NAVAL_COMBAT_LIFECYCLE_DOMAIN_ID,
                "domain_missing_current_hour",
            );
        }
        if self.naval_combat_lifecycle_known_this_hour
            && !self.naval_combats.is_empty()
            && !self.naval_combat_members_known_this_hour
            && !self
                .domains_this_hour
                .contains(&NAVAL_COMBAT_MEMBERS_DOMAIN_ID)
        {
            push_problem(
                NAVAL_COMBAT_MEMBERS_DOMAIN_ID,
                "domain_missing_current_hour",
            );
        }
        let unfolded_domains: Vec<Value> = self
            .unfolded_domains_this_hour
            .iter()
            .map(|domain_id| domain_descriptor_json(*domain_id))
            .collect();
        let late_records_dropped: Vec<Value> = self
            .late_records_dropped_since_last_snapshot
            .iter()
            .map(|(hour_index, count)| {
                json!({
                    "hour_index": hour_index,
                    "count": count,
                })
            })
            .collect();
        let mut collector = Map::new();
        collector.insert("captured".into(), json!(true));
        collector.insert("backend".into(), json!("in_process_collector"));
        collector.insert("domains".into(), Value::Array(domains));
        collector.insert("domain_problems".into(), Value::Array(domain_problems));
        collector.insert("unfolded_domains".into(), Value::Array(unfolded_domains));
        collector.insert(
            "late_records_dropped".into(),
            Value::Array(late_records_dropped),
        );
        if !self.frd_depth_badges_this_hour.is_empty() {
            collector.insert(
                "_r2_depth_badges".into(),
                Value::Array(
                    self.frd_depth_badges_this_hour
                        .iter()
                        .map(FrdDepthBadgeState::to_json)
                        .collect(),
                ),
            );
        }

        let divisions_value = if self.divisions_unknown_this_hour {
            Value::Null
        } else {
            Value::Array(divisions)
        };
        let states_value = Value::Object(states);
        let provinces_value = Value::Object(provinces);
        let mut map = Map::new();
        map.insert(
            "owner_by_state".into(),
            if self.map_control_unknown_this_hour {
                Value::Null
            } else {
                Value::Object(owner_by_state)
            },
        );
        map.insert(
            "control_by_province".into(),
            if self.map_control_unknown_this_hour {
                Value::Null
            } else {
                Value::Object(control_by_province)
            },
        );
        if self.map_detail_unknown_this_hour {
            if self.map_detail_live.is_some() {
                map.insert("map_detail_live".into(), Value::Null);
            }
        } else if let Some(layer) = &self.map_detail_live {
            map.insert("map_detail_live".into(), layer.clone());
        };
        if self.fold_domain_unknown(WEATHER_DOMAIN_ID) {
            if self.weather_live.is_some() || self.fold_domain_seen(WEATHER_DOMAIN_ID) {
                map.insert("_r2_weather_live".into(), Value::Null);
            }
        } else if let Some(weather) = &self.weather_live {
            map.insert("_r2_weather_live".into(), weather.to_json());
        }
        let map_value = Value::Object(map);
        let naval_combats_value = self.build_naval_combats(&mut unresolved);
        let order_of_battle_value = self.build_order_of_battle(&mut unresolved);
        let designs_value = self.build_designs(&mut unresolved);
        let naval_air_value = self.build_naval_air(&mut unresolved);
        let strategic_air_value = self.build_strategic_air(&mut unresolved);
        let world_value = self.build_world_value();
        let special_projects_value = self.build_special_projects_value(&mut unresolved);
        let session_multiplayer_metadata_value = self.build_domain57_session_sidecar();
        // W6: top-level session header (d68) + tech-sharing groups (d69). Both
        // can reference country tag_db_ids, so build them BEFORE the resolved
        // gate below so any unmapped id is counted.
        let session_header_value = self.build_session_header_value(&mut unresolved);
        let tech_sharing_groups_value = self.build_tech_sharing_groups_value(&mut unresolved);

        // resolved only when we have a map AND every keyed/referenced id
        // resolved. Keep this after all section builders: OOB, designs, naval
        // air, strategic air, and special projects can all add country refs.
        let resolved = have_map && unresolved.is_empty();
        let tag_db_id_index = if !have_map {
            json!({
                "resolved": false,
                "key": "tag_db_id",
                "reason": "no country_identity (domain 18) tag_db_id -> TAG map yet",
            })
        } else if resolved {
            json!({ "resolved": true, "key": "tag" })
        } else {
            let gaps: Vec<String> = unresolved.iter().map(u32::to_string).collect();
            json!({
                "resolved": false,
                "key": "tag",
                "reason": format!(
                    "country_identity map present but {} tag_db_id(s) unmapped; those keys stay numeric",
                    gaps.len()
                ),
                "unresolved_tag_db_ids": gaps,
            })
        };
        if have_map && !unresolved.is_empty() {
            let gaps: Vec<String> = unresolved.iter().map(u32::to_string).collect();
            notes.push(format!(
                "tag_db_id -> TAG unresolved for {}: {}",
                gaps.len(),
                gaps.join(",")
            ));
        }

        // E13 measured-country observability: the set of countries the capture
        // RESOLVED this frame (domain 18's full-replace roster). Downstream uses
        // it to distinguish "country absent from a domain = no data for it" (the
        // country IS measured) from "country absent = not resolved this frame"
        // (not in the measured set — the systemic country-roots bound excludes
        // dynamic observers like D01/D02/D03). `known: false` when no proven
        // domain-18 record landed this hour — the measured set is UNKNOWN, never
        // reported as an empty/complete set.
        let measured_countries = self.build_measured_countries();

        let mut snapshot = json!({
            "schema_version": SCHEMA_VERSION,
            "game_id": self.game_id,
            "date": date,
            "hour_index": hour_index,
            "save_version": Value::Null,
            "provenance": {
                "collector": Value::Object(collector),
                "mod": Value::Null,
                "notes": notes,
            },
            "tag_db_id_index": tag_db_id_index,
            "measured_countries": measured_countries,
            "countries": Value::Object(countries),
            "states": states_value,
            "provinces": provinces_value,
            "divisions": divisions_value,
            "combats": combats,
            "diplomacy_world": Value::Object(diplomacy_world),
            "map": map_value,
            "designs": designs_value,
            "order_of_battle": order_of_battle_value,
            "naval_combats": naval_combats_value,
            "naval_air": naval_air_value,
            "supply": supply_value,
            "strategic_air": strategic_air_value,
            "strategic_operatives": {},
            "raids": {},
            "world_events": {},
            "special_projects": special_projects_value,
            "world": world_value,
            "events": [],
        });
        if let Some(value) = session_multiplayer_metadata_value {
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert(DOMAIN57_R2_KEY.into(), value);
            }
        }
        // W6: attach the top-level session header (d68) and tech-sharing groups
        // (d69) only once their domains have folded (absent != null != empty).
        if let Some(value) = session_header_value {
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("session_header".into(), value);
            }
        }
        if let Some(value) = tech_sharing_groups_value {
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("tech_sharing_group".into(), value);
            }
        }
        snapshot
    }

    fn build_combats(&self) -> (Value, Vec<(u32, u64)>) {
        if !self.land_combats_known_this_hour {
            let dropped = self
                .combat_detail_events_this_hour
                .keys()
                .copied()
                .collect();
            return (Value::Null, dropped);
        }
        let mut out = Vec::with_capacity(self.combats.len());
        for combat in self.combats.values() {
            let mut obj = Map::new();
            obj.insert("combat_id".into(), json!(combat.combat_id));
            obj.insert("kind".into(), json!("land"));
            obj.insert("location_province".into(), option_u32_json(combat.location));
            obj.insert("attackers".into(), option_u32_vec_json(&combat.attackers));
            obj.insert("defenders".into(), option_u32_vec_json(&combat.defenders));
            obj.insert("combat_width".into(), Value::Null);
            obj.insert("terrain".into(), Value::Null);
            obj.insert("progress".into(), Value::Null);
            obj.insert("modifiers".into(), Value::Null);

            if let Some(detail) = self
                .combat_detail_events_this_hour
                .get(&(combat.combat_id, combat.generation))
            {
                let mut detail_obj = Map::new();
                if let Some(phase_event) = &detail.phase_event {
                    detail_obj.insert("phase_event".into(), json!(phase_event));
                }
                if let Some(generation) = detail.generation {
                    detail_obj.insert("generation".into(), json!(generation));
                } else {
                    detail_obj.insert("generation".into(), json!(combat.generation));
                }
                if let Some(location) = detail.location {
                    detail_obj.insert("location_province".into(), json!(location));
                }
                if let Some(day) = detail.day {
                    detail_obj.insert("age_days".into(), json!(day));
                }
                if let Some(duration) = detail.duration {
                    detail_obj.insert("duration_days".into(), json!(duration));
                }
                if !detail.participant_events.is_empty() {
                    detail_obj.insert(
                        "participant_events".into(),
                        Value::Array(detail.participant_events.clone()),
                    );
                }
                // E2: per-side CCombatSide detail is ALL-OR-NOTHING per combat.
                // If EITHER side failed its read (None), emit NEITHER side's
                // detail — a consumer must never see one-sided detail and read
                // the missing side as zero/complete. An explicit
                // `side_detail_completeness` badge records why detail is
                // present or absent so absence is never ambiguous.
                match (&detail.attacker_detail, &detail.defender_detail) {
                    (Some(attacker), Some(defender)) => {
                        detail_obj.insert(
                            "attacker".into(),
                            Self::attach_side_attribution(
                                Self::side_detail_json(attacker),
                                &detail.attacker_attribution,
                            ),
                        );
                        detail_obj.insert(
                            "defender".into(),
                            Self::attach_side_attribution(
                                Self::side_detail_json(defender),
                                &detail.defender_attribution,
                            ),
                        );
                        // If either side's stable count triple was inconsistent
                        // (a mid-combat transient), badge the pair complete-but-
                        // counts-incomplete rather than fully authoritative.
                        let counts_incomplete =
                            attacker.counts_incomplete || defender.counts_incomplete;
                        detail_obj.insert(
                            "side_detail_completeness".into(),
                            json!(if counts_incomplete {
                                "both_counts_incomplete"
                            } else {
                                "both"
                            }),
                        );
                    }
                    (Some(_), None) => {
                        detail_obj.insert(
                            "side_detail_completeness".into(),
                            json!("attacker_only_suppressed"),
                        );
                    }
                    (None, Some(_)) => {
                        detail_obj.insert(
                            "side_detail_completeness".into(),
                            json!("defender_only_suppressed"),
                        );
                    }
                    (None, None) => {}
                }
                if detail.attacker_detail.is_none() {
                    if let Some(value) =
                        Self::attribution_only_side_json(&detail.attacker_attribution)
                    {
                        detail_obj.insert("attacker".into(), value);
                    }
                }
                if detail.defender_detail.is_none() {
                    if let Some(value) =
                        Self::attribution_only_side_json(&detail.defender_attribution)
                    {
                        detail_obj.insert("defender".into(), value);
                    }
                }
                if !detail_obj.is_empty() {
                    obj.insert(
                        "land_combat_detail_events_live".into(),
                        Value::Object(detail_obj),
                    );
                }
            }

            out.push(Value::Object(obj));
        }
        let dropped = self
            .combat_detail_events_this_hour
            .keys()
            .copied()
            .filter(|(combat_id, generation)| {
                !self
                    .combats
                    .get(combat_id)
                    .is_some_and(|combat| combat.generation == *generation)
            })
            .collect();
        (Value::Array(out), dropped)
    }

    fn country_ref_json(&self, tag_db_id: u32, unresolved: &mut BTreeSet<u32>) -> Value {
        if tag_db_id == 0 {
            Value::Null
        } else {
            json!(self.resolve_country_key(&tag_db_id.to_string(), unresolved))
        }
    }

    fn project_domain53_country_overlays(
        &self,
        countries: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        for (tag_db_id, overlay) in &self.domain53_country_overlays_this_hour {
            if overlay.is_empty() {
                continue;
            }
            let key = self.resolve_country_key(&tag_db_id.to_string(), unresolved);
            let entry = countries
                .entry(key)
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(country) = entry.as_object_mut() else {
                continue;
            };

            if overlay.country.is_some()
                || !overlay.opinions.is_empty()
                || !overlay.relation_flags.is_empty()
            {
                let diplomacy = ensure_json_object_mut(country, "diplomacy");
                diplomacy.insert(
                    DOMAIN53_EXTRA_KEY.into(),
                    Value::Object(self.domain53_diplomacy_extra(overlay, unresolved)),
                );
                // Promote the relation kinds that already have canonical schema
                // homes. Only positively observed rows are emitted: a tolerated
                // partial D53 frame may omit rows, so an absent list must never be
                // fabricated as a measured empty list.
                for (flag_kind, schema_key) in [
                    (diplomacy53::compact::REL_KIND_GUARANTEE, "guarantees"),
                    (
                        diplomacy53::compact::REL_KIND_NON_AGGRESSION_PACT,
                        "non_aggression",
                    ),
                    (diplomacy53::compact::REL_KIND_EMBARGO, "embargoes"),
                ] {
                    let targets = overlay
                        .relation_flags
                        .iter()
                        .filter(|row| row.flag_kind == flag_kind)
                        .map(|row| self.country_ref_json(row.target_tag_db_id, unresolved))
                        .collect::<Vec<_>>();
                    if !targets.is_empty() {
                        diplomacy.insert(schema_key.into(), Value::Array(targets));
                    }
                }
            }
            if overlay.agency.is_some()
                || overlay.agency_capacity.is_some()
                || !overlay.operatives.is_empty()
                || !overlay.cryptology.is_empty()
            {
                let intel = ensure_json_object_mut(country, "intel");
                intel.insert(
                    DOMAIN53_EXTRA_KEY.into(),
                    Value::Object(self.domain53_intel_extra(overlay, unresolved)),
                );
            }
            if !overlay.state_events.is_empty() {
                let extra = ensure_json_object_mut(country, "extra");
                extra.insert(
                    DOMAIN53_R2_KEY.into(),
                    Value::Object(self.domain53_state_events_extra(overlay, unresolved)),
                );
            }
        }
    }

    fn project_domain53_faction_memberships(
        &self,
        countries: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        if self.domain53_world_overlay_this_hour.faction_roster_status
            == FactionRosterStatus::Unavailable
        {
            return;
        }
        let conflicts = self.domain53_faction_member_conflicts();
        for faction in &self.domain53_world_overlay_this_hour.factions {
            let leader = self.country_ref_json(faction.leader_tag_db_id, unresolved);
            for member_tag_db_id in &faction.member_tag_db_ids {
                if conflicts.contains(member_tag_db_id) {
                    continue;
                }
                let key = self.resolve_country_key(&member_tag_db_id.to_string(), unresolved);
                let country = countries
                    .entry(key)
                    .or_insert_with(|| Value::Object(Map::new()));
                let Some(country) = country.as_object_mut() else {
                    continue;
                };
                let politics = ensure_json_object_mut(country, "politics");
                politics.insert(
                    "faction".into(),
                    json!({
                        // D53 carries only name_hash. A hash is not faction_name,
                        // so the name leaf stays absent until the wire carries text.
                        "leader": leader.clone(),
                        "is_leader": *member_tag_db_id == faction.leader_tag_db_id,
                    }),
                );
            }
        }
    }

    fn domain53_faction_member_conflicts(&self) -> BTreeSet<u32> {
        if self.domain53_world_overlay_this_hour.faction_roster_status
            == FactionRosterStatus::Unavailable
        {
            return BTreeSet::new();
        }
        let mut faction_by_member = BTreeMap::new();
        let mut conflicts = BTreeSet::new();
        for faction in &self.domain53_world_overlay_this_hour.factions {
            for member_tag_db_id in &faction.member_tag_db_ids {
                if faction_by_member
                    .insert(*member_tag_db_id, faction.faction_id)
                    .is_some_and(|prior_faction_id| prior_faction_id != faction.faction_id)
                {
                    conflicts.insert(*member_tag_db_id);
                }
            }
        }
        conflicts
    }

    fn domain53_diplomacy_extra(
        &self,
        overlay: &Domain53CountryOverlay,
        unresolved: &mut BTreeSet<u32>,
    ) -> Map<String, Value> {
        let mut out = Map::new();
        if let Some(row) = &overlay.country {
            out.insert("faction_group".into(), json!(row.faction_group));
            out.insert("at_war_count".into(), json!(row.at_war_count));
            out.insert("casualties".into(), json!(row.casualties));
        }
        if !overlay.opinions.is_empty() {
            out.insert(
                "opinion_rows".into(),
                Value::Array(
                    overlay
                        .opinions
                        .iter()
                        .map(|row| {
                            json!({
                                "target": self.country_ref_json(row.target_tag_db_id, unresolved),
                                "target_tag_db_id": row.target_tag_db_id,
                                "opinion": row.opinion,
                            })
                        })
                        .collect(),
                ),
            );
        }
        if !overlay.relation_flags.is_empty() {
            out.insert(
                "relation_flags".into(),
                Value::Array(
                    overlay
                        .relation_flags
                        .iter()
                        .map(|row| {
                            json!({
                                "target": self.country_ref_json(row.target_tag_db_id, unresolved),
                                "target_tag_db_id": row.target_tag_db_id,
                                "flag_kind": row.flag_kind,
                                "flag_name": domain53_relation_flag_name(row.flag_kind),
                                "directed": row.directed,
                            })
                        })
                        .collect(),
                ),
            );
        }
        out
    }

    fn domain53_intel_extra(
        &self,
        overlay: &Domain53CountryOverlay,
        unresolved: &mut BTreeSet<u32>,
    ) -> Map<String, Value> {
        let mut out = Map::new();
        if let Some(row) = &overlay.agency {
            out.insert(
                "agency_scalars".into(),
                json!({
                    "max_operatives_unresolved": row.max_operatives,
                    "usable_slots_unresolved": row.usable_slots,
                    "operative_count": row.operative_count,
                    "cryptology_target_count": row.cryptology_target_count,
                    "defense_raw": row.defense_raw,
                    "upgrade_progress_raw": row.upgrade_progress_raw,
                    "name_hash": row.name_hash,
                    "icon_hash": row.icon_hash,
                }),
            );
        }
        if let Some(row) = &overlay.agency_capacity {
            out.insert(
                "raw_capacity_cluster".into(),
                json!({
                    "value_0xe4": row.value_0xe4,
                    "value_0xf0": row.value_0xf0,
                    "value_0xf4": row.value_0xf4,
                    "proof_hash": row.proof_hash,
                    "semantic_status": "raw_offsets_unresolved",
                }),
            );
        }
        if !overlay.operatives.is_empty() {
            out.insert(
                "operative_lifecycle_rows".into(),
                Value::Array(
                    overlay
                        .operatives
                        .iter()
                        .map(|row| {
                            json!({
                                "op_id": row.op_id,
                                "detected": row.detected,
                                "captured_unvalidated": row.captured,
                                "skill": row.skill,
                                "experience_raw": row.experience_raw,
                                "name_hash": row.name_hash,
                                "state_hash": row.state_hash,
                                "lifecycle_flag": row.lifecycle_flag,
                            })
                        })
                        .collect(),
                ),
            );
        }
        if !overlay.cryptology.is_empty() {
            out.insert(
                "cryptology_targets".into(),
                Value::Array(
                    overlay
                        .cryptology
                        .iter()
                        .map(|row| {
                            json!({
                                "target": self.country_ref_json(row.target_tag_db_id, unresolved),
                                "target_tag_db_id": row.target_tag_db_id,
                                "amount_raw": row.amount_raw,
                                "decrypting": row.decrypting,
                            })
                        })
                        .collect(),
                ),
            );
        }
        out
    }

    fn domain53_state_events_extra(
        &self,
        overlay: &Domain53CountryOverlay,
        unresolved: &mut BTreeSet<u32>,
    ) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert(
            "state_events".into(),
            Value::Array(
                overlay
                    .state_events
                    .iter()
                    .map(|row| {
                        json!({
                            "target": row.target_tag_db_id.map(|tag| self.country_ref_json(tag, unresolved)),
                            "target_tag_db_id": row.target_tag_db_id,
                            "event_kind": row.event_kind,
                            "source_kind": row.source_kind,
                            "object_hash": row.object_hash,
                            "value_hash": row.value_hash,
                            "value_raw": row.value_raw,
                        })
                    })
                    .collect(),
            ),
        );
        out
    }

    fn project_domain53_diplomacy_world(
        &self,
        diplomacy_world: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        let mut extra = Map::new();
        extra.insert(
            "status_flags".into(),
            json!(self.domain53_world_overlay_this_hour.status_flags),
        );
        let mut omitted_unresolved = vec![
            json!("operation_detail_fixture_blocked"),
            json!("spy_network_intel_map_percentages_fixture_blocked"),
            json!("war_lifecycle_semantics_unvalidated"),
            json!("agency_capacity_semantics_unresolved"),
            json!("operative_full_semantics_unvalidated"),
        ];
        if self.domain53_world_overlay_this_hour.status_flags
            & diplomacy53::compact::STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED
            != 0
        {
            omitted_unresolved.push(json!("incoming_action_queues_fixture_blocked"));
        }
        if self.domain53_world_overlay_this_hour.status_flags
            & diplomacy53::compact::STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED
            != 0
        {
            omitted_unresolved.push(json!("intel_network_graph_fixture_blocked"));
        }
        extra.insert(
            "omitted_unresolved".into(),
            Value::Array(omitted_unresolved),
        );
        if !self.domain53_world_overlay_this_hour.wars.is_empty() {
            extra.insert(
                "war_relation_rows".into(),
                Value::Array(
                    self.domain53_world_overlay_this_hour
                        .wars
                        .iter()
                        .map(|row| {
                            json!({
                                "first": self.country_ref_json(row.first_tag_db_id, unresolved),
                                "second": self.country_ref_json(row.second_tag_db_id, unresolved),
                                "first_tag_db_id": row.first_tag_db_id,
                                "second_tag_db_id": row.second_tag_db_id,
                                "first_casualties": row.first_casualties,
                                "second_casualties": row.second_casualties,
                                "lifecycle_unvalidated": true,
                            })
                        })
                        .collect(),
                ),
            );
        }
        if !self.domain53_world_overlay_this_hour.subjects.is_empty() {
            extra.insert(
                "subject_autonomy_rows".into(),
                Value::Array(
                    self.domain53_world_overlay_this_hour
                        .subjects
                        .iter()
                        .map(|row| {
                            let start_date = compact_contract::calendar::date_from_raw(
                                row.start_date_raw,
                            )
                            .map(Value::from)
                            .unwrap_or(Value::Null);
                            json!({
                                "overlord": self.country_ref_json(row.overlord_tag_db_id, unresolved),
                                "subject": self.country_ref_json(row.subject_tag_db_id, unresolved),
                                "overlord_tag_db_id": row.overlord_tag_db_id,
                                "subject_tag_db_id": row.subject_tag_db_id,
                                "autonomy_state_hash": row.autonomy_state_hash,
                                // ratio has no melt oracle — carried UNVALIDATED.
                                "autonomy_ratio_raw": row.autonomy_ratio_raw,
                                "start_date": start_date,
                                "start_date_raw": row.start_date_raw,
                            })
                        })
                        .collect(),
                ),
            );
        }
        let faction_member_conflicts = self.domain53_faction_member_conflicts();
        extra.insert(
            "faction_roster_status".into(),
            json!(if faction_member_conflicts.is_empty() {
                self.domain53_world_overlay_this_hour
                    .faction_roster_status
                    .as_str()
            } else {
                "ambiguous_member_exclusivity"
            }),
        );
        if !faction_member_conflicts.is_empty() {
            extra.insert(
                "faction_member_conflict_tag_db_ids".into(),
                Value::Array(
                    faction_member_conflicts
                        .iter()
                        .copied()
                        .map(Value::from)
                        .collect(),
                ),
            );
        }
        if self.domain53_world_overlay_this_hour.faction_roster_status
            != FactionRosterStatus::Unavailable
            && faction_member_conflicts.is_empty()
        {
            extra.insert(
                "faction_rows".into(),
                Value::Array(
                    self.domain53_world_overlay_this_hour
                        .factions
                        .iter()
                        .map(|row| {
                            json!({
                                "faction_id": row.faction_id,
                                "faction_type": row.faction_type,
                                "name_hash": row.name_hash,
                                "leader": self.country_ref_json(row.leader_tag_db_id, unresolved),
                                "leader_tag_db_id": row.leader_tag_db_id,
                                "member_tag_db_ids": row.member_tag_db_ids.clone(),
                                "members": Value::Array(
                                    row.member_tag_db_ids
                                        .iter()
                                        .map(|tag| self.country_ref_json(*tag, unresolved))
                                        .collect(),
                                ),
                                "member_count": row.member_tag_db_ids.len(),
                            })
                        })
                        .collect(),
                ),
            );
        }
        if !self
            .domain53_world_overlay_this_hour
            .incoming_actions
            .is_empty()
        {
            extra.insert(
                "incoming_diplomatic_action_queue_rows".into(),
                Value::Array(
                    self.domain53_world_overlay_this_hour
                        .incoming_actions
                        .iter()
                        .map(|row| {
                            json!({
                                "recipient": self.country_ref_json(row.recipient_tag_db_id, unresolved),
                                "actor": self.country_ref_json(row.actor_tag_db_id, unresolved),
                                "recipient_tag_db_id": row.recipient_tag_db_id,
                                "actor_tag_db_id": row.actor_tag_db_id,
                                "action_id": row.action_id,
                                "action_type": row.action_type,
                                "date_hours": row.date_hours,
                                "last_command_date_hours": row.last_command_date_hours,
                            })
                        })
                        .collect(),
                ),
            );
        }
        if !self
            .domain53_world_overlay_this_hour
            .intel_network_graphs
            .is_empty()
        {
            extra.insert(
                "intel_network_graph_rows".into(),
                Value::Array(
                    self.domain53_world_overlay_this_hour
                        .intel_network_graphs
                        .iter()
                        .map(|row| {
                            json!({
                                "strategic_operative_index": row.strategic_operative_index,
                                "network_slot": row.network_slot,
                                "target": self.country_ref_json(row.target_tag_db_id, unresolved),
                                "target_tag_db_id": row.target_tag_db_id,
                                "state_ids": row.state_ids.clone(),
                                "vertex_count": row.state_ids.len(),
                            })
                        })
                        .collect(),
                ),
            );
        }
        // Wave-5 running-operation instances (world-scoped provisional
        // sidecar). Each row: {instance_id, def_ref_ok, associated_country}
        // where the country is badged PROVISIONAL — semantics are
        // associated-country; target-vs-operative-location is undistinguished
        // (all 8 coincided on D04). The owner (+0x10) is REJECTED and absent.
        //
        // BLOCKED-ADDITIVE (build 870ebf): this emission path is NOT reachable
        // live yet. The D53 base extract is fail-closed on 870ebf (its live
        // root offsets are unvalidated, so build_domain53_payload emits an
        // empty SOURCE_UNAVAILABLE record — see DOMAIN_REGISTRY.md D53). The
        // operations wiring is complete, unit-proven, and rides the validated
        // discovery cache, but this fold branch stays dark until the d53 base
        // root source-availability is resolved. Do NOT read a green live proof
        // of operations here as validation until that lands; the unit tests +
        // the offset-verify (8/8 targets) are the current proof.
        if !self
            .domain53_world_overlay_this_hour
            .operation_instances
            .is_empty()
        {
            extra.insert(
                "operation_instance_rows".into(),
                Value::Array(
                    self.domain53_world_overlay_this_hour
                        .operation_instances
                        .iter()
                        .map(|row| {
                            json!({
                                "instance_id": row.instance_id,
                                "def_ref_ok": row.def_ref_ok,
                                "associated_country": row
                                    .associated_country_tag_db_id
                                    .map(|tag| self.country_ref_json(tag, unresolved))
                                    .unwrap_or(Value::Null),
                                "associated_country_tag_db_id": row.associated_country_tag_db_id,
                                "associated_country_semantics": "provisional_target_vs_location_undistinguished",
                            })
                        })
                        .collect(),
                ),
            );
        }
        diplomacy_world.insert(DOMAIN53_EXTRA_KEY.into(), Value::Object(extra));
    }

    fn build_order_of_battle(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        let mut out = Map::new();
        let mut by_country: BTreeMap<String, OobCountryCounts> = BTreeMap::new();

        if self.fold_domain_unknown(ARMY_ORDER_DOMAIN_ID) {
            out.insert("battle_plan_orders".into(), Value::Null);
        } else if self.fold_domain_seen(ARMY_ORDER_DOMAIN_ID) {
            out.insert(
                "battle_plan_orders".into(),
                Value::Array(
                    self.army_orders
                        .values()
                        .map(|order| {
                            json!({
                                "army": order.army_id,
                                "order_instance": {
                                    "type": order.order_type,
                                    "states": order.target_states,
                                    "_r2_army_order_capture": {
                                        "generation": order.generation,
                                        "slot_index": order.slot_index,
                                        "target_state_declared_count": order.target_state_declared_count,
                                        "target_status_code": order.target_status_code,
                                    },
                                },
                            })
                        })
                        .collect(),
                ),
            );
        }

        if self.fold_domain_unknown(OOB_COMMAND_DOMAIN_ID) {
            for key in ["army_groups", "armies", "fleets"] {
                out.insert(key.into(), Value::Null);
            }
        } else if self.fold_domain_seen(OOB_COMMAND_DOMAIN_ID) {
            let mut army_groups = Vec::new();
            let mut armies = Vec::new();
            let mut fleets: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
            let mut task_forces_by_fleet: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
            for row in self.oob_commands.values() {
                let owner = self.country_ref_json(row.owner_tag_db_id, unresolved);
                if let Some(owner_key) = owner.as_str() {
                    let counts = by_country.entry(owner_key.to_string()).or_default();
                    match row.kind {
                        OOB_KIND_ARMY => counts.armies = counts.armies.saturating_add(1),
                        OOB_KIND_ARMY_GROUP => {
                            counts.army_groups = counts.army_groups.saturating_add(1)
                        }
                        OOB_KIND_FLEET => counts.fleets = counts.fleets.saturating_add(1),
                        OOB_KIND_TASK_FORCE => {
                            counts.task_forces = counts.task_forces.saturating_add(1)
                        }
                        _ => {}
                    }
                }
                let capture = json!({
                    "kind": row.kind,
                    "parent_status": row.parent_status,
                    "member_status": row.member_status,
                    "owner_status": row.owner_status,
                    "leader_status": row.leader_status,
                    "leader_trait_status": row.leader_trait_status,
                    "logical_member_count": row.logical_member_count,
                    "member_count_engine": row.member_count_engine,
                    "missing_member_count": row.missing_member_count,
                    "leader_trait_count": row.leader_trait_count,
                    "location_province_id": row.location_province_id,
                });
                match row.kind {
                    OOB_KIND_ARMY_GROUP => army_groups.push(json!({
                        "id": row.object_id,
                        "owner": owner,
                        "leader": row.leader_id,
                        "leader_skill": row.leader_skill,
                        "army_refs": row.members,
                        "_r2_oob_command_capture": capture,
                    })),
                    OOB_KIND_ARMY => armies.push(json!({
                        "id": row.object_id,
                        "owner": owner,
                        "leader": row.leader_id,
                        "leader_skill": row.leader_skill,
                        "members": row.members,
                        "member_count": row.logical_member_count,
                        "field_marshal_group": row.parent_id,
                        "_r2_oob_command_capture": capture,
                    })),
                    OOB_KIND_FLEET => {
                        let mut fleet = Map::new();
                        fleet.insert("id".into(), json!(row.object_id));
                        fleet.insert("owner".into(), owner);
                        fleet.insert(
                            "leader".into(),
                            row.leader_id.map_or(Value::Null, |id| json!(id)),
                        );
                        fleet.insert("_r2_oob_command_capture".into(), capture);
                        fleets.insert(row.object_id, fleet);
                    }
                    OOB_KIND_TASK_FORCE => {
                        let mut task_force = Map::new();
                        task_force.insert("id".into(), json!(row.object_id));
                        task_force.insert("owner".into(), owner);
                        task_force.insert(
                            "location_province".into(),
                            row.location_province_id.map_or(Value::Null, |id| json!(id)),
                        );
                        if self.fold_domain_unknown(SHIPS_DOMAIN_ID) {
                            task_force.insert("ships".into(), Value::Null);
                        } else if self.fold_domain_seen(SHIPS_DOMAIN_ID) {
                            task_force.insert(
                                "ships".into(),
                                Value::Array(
                                    self.ships
                                        .values()
                                        .filter(|ship| ship.task_force_id == Some(row.object_id))
                                        .map(|ship| self.ship_json(ship, unresolved))
                                        .collect(),
                                ),
                            );
                        }
                        task_force.insert("_r2_oob_command_capture".into(), capture);
                        task_forces_by_fleet
                            .entry(row.parent_id.unwrap_or(0))
                            .or_default()
                            .push(Value::Object(task_force));
                    }
                    _ => {}
                }
            }
            for (fleet_id, task_forces) in task_forces_by_fleet {
                if fleet_id == 0 {
                    continue;
                }
                fleets
                    .entry(fleet_id)
                    .or_insert_with(|| {
                        let mut fleet = Map::new();
                        fleet.insert("id".into(), json!(fleet_id));
                        fleet
                    })
                    .insert("task_forces".into(), Value::Array(task_forces));
            }
            out.insert("army_groups".into(), Value::Array(army_groups));
            out.insert("armies".into(), Value::Array(armies));
            out.insert(
                "fleets".into(),
                Value::Array(fleets.into_values().map(Value::Object).collect()),
            );
        }

        if self.fold_domain_unknown(SHIPS_DOMAIN_ID) {
            out.insert("ships".into(), Value::Null);
        } else if self.fold_domain_seen(SHIPS_DOMAIN_ID) {
            let mut ships = Vec::new();
            for ship in self.ships.values() {
                if let Some(owner) = self.ship_owner_key(ship, unresolved) {
                    let counts = by_country.entry(owner).or_default();
                    counts.ships = counts.ships.saturating_add(1);
                }
                ships.push(self.ship_json(ship, unresolved));
            }
            out.insert("ships".into(), Value::Array(ships));
        }

        if self.fold_domain_unknown(AIR_WINGS_DOMAIN_ID) {
            out.insert("air_wings".into(), Value::Null);
        } else if self.fold_domain_seen(AIR_WINGS_DOMAIN_ID) {
            // Wave-4 ace lifecycle: (owner, wing) -> ace join from the
            // domain-46 ace roster. Built ONLY when this hour applied a clean
            // ace trailer (air_aces_fresh_this_hour) — a retained/stale
            // roster from a partial/unavailable/failed D46 hour must not be
            // surfaced as this hour's wing ace (Codex DEFECT: stale ace usable
            // after D46 failure). An ace resolves onto a wing only when
            // exactly one roster ace claims it (ambiguity drops the
            // annotation, fail closed); owner match is enforced by the key.
            let mut ace_by_wing: BTreeMap<(u32, u32), Option<&ace46::AirAceRosterRow>> =
                BTreeMap::new();
            if self.air_aces_fresh_this_hour {
                for ((tag_db_id, _ace_id), ace) in &self.air_aces {
                    if let Some(wing_id) = ace.wing_id {
                        ace_by_wing
                            .entry((*tag_db_id, wing_id))
                            .and_modify(|slot| *slot = None)
                            .or_insert(Some(ace));
                    }
                }
            }
            let mut air_wings = Vec::new();
            for key in self.air_wings.keys() {
                let mut wing = Map::new();
                wing.insert("id".into(), json!(key.wing_id));
                wing.insert(
                    "owner".into(),
                    self.country_ref_json(key.owner_tag_db_id, unresolved),
                );
                let base = self
                    .air_wings
                    .get(key)
                    .expect("iterating D30-owned air_wings keys");
                wing.insert("count".into(), json!(base.count));
                wing.insert("mission_type".into(), json!(base.mission_mask));
                wing.insert(
                    "strategic_region".into(),
                    base.mission_region.map_or(Value::Null, |id| json!(id)),
                );
                wing.insert("effective_planes".into(), json!(base.effective_planes));
                wing.insert(
                    "_r2_air_wing_capture".into(),
                    json!({
                        "generation": base.generation,
                        "mission_mask": base.mission_mask,
                    }),
                );
                if let Some(Some(ace)) = ace_by_wing.get(&(key.owner_tag_db_id, key.wing_id)) {
                    wing.insert(
                        "ace".into(),
                        json!({
                            "id": ace.ace_id,
                            "name": ace.name,
                            "surname": ace.surname,
                            "callsign": ace.callsign,
                            "alive": ace.alive,
                        }),
                    );
                }
                if let Some(owner) = self.air_wing_owner_key(key, unresolved) {
                    let counts = by_country.entry(owner).or_default();
                    counts.air_wings = counts.air_wings.saturating_add(1);
                    counts.planes = counts.planes.saturating_add(base.count as u64);
                }
                if self.fold_domain_unknown(AIR_WING_LOADOUTS_DOMAIN_ID) {
                    wing.insert("equipment".into(), Value::Null);
                    wing.insert("_r2_air_wing_loadout".into(), Value::Null);
                } else if let Some(loadout) = self.air_wing_loadouts.get(key) {
                    wing.insert(
                        "air_base".into(),
                        loadout.air_base_id.map_or(Value::Null, |id| json!(id)),
                    );
                    wing.insert(
                        "equipment".into(),
                        Value::Array(
                            loadout
                                .equipment
                                .iter()
                                .map(|entry| {
                                    json!([
                                        entry.variant_id,
                                        Value::Null,
                                        entry.amount_raw as f64 / FIXED_SCALE
                                    ])
                                })
                                .collect(),
                        ),
                    );
                    wing.insert(
                        "_r2_air_wing_loadout".into(),
                        json!({
                            "air_base_id": loadout.air_base_id,
                            "equipment": loadout.equipment.iter().map(|entry| {
                                json!({
                                    "variant_id": entry.variant_id,
                                    "amount_raw": entry.amount_raw,
                                    "amount": entry.amount_raw as f64 / FIXED_SCALE,
                                })
                            }).collect::<Vec<_>>(),
                        }),
                    );
                }
                air_wings.push(Value::Object(wing));
            }
            out.insert("air_wings".into(), Value::Array(air_wings));
        }

        if !by_country.is_empty() {
            out.insert(
                "by_country".into(),
                Value::Object(
                    by_country
                        .into_iter()
                        .map(|(country, counts)| (country, counts.to_json()))
                        .collect(),
                ),
            );
        }

        if self.fold_domain_unknown(NAVAL_MISSIONS_STATUS_DOMAIN_ID) {
            ensure_json_object_mut(&mut out, "extra")
                .insert("naval_missions_status_live".into(), Value::Null);
        } else if self.fold_domain_seen(NAVAL_MISSIONS_STATUS_DOMAIN_ID) {
            let naval_status = self.build_naval_missions_status_extra(unresolved);
            ensure_json_object_mut(&mut out, "extra")
                .insert("naval_missions_status_live".into(), naval_status);
        }

        if self.fold_domain_unknown(ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID)
            || self.fold_domain_seen(ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID)
        {
            ensure_json_object_mut(&mut out, "extra").insert(
                "army_battleplans_depth_live".into(),
                self.army_battleplans_depth_extra_json(),
            );
        }

        Value::Object(out)
    }

    fn army_battleplans_depth_extra_json(&self) -> Value {
        let mut orders = self
            .army_battleplans_depth
            .values()
            .map(|order| {
                json!({
                    "army_id": order.key.army_id,
                    "generation": order.key.generation,
                    "slot_index": order.slot_index,
                    "order_type": order.order_type,
                    "target_states": order.target_states,
                    "path_provinces": order.path_provinces,
                    "target_declared_count": order.target_declared_count,
                    "target_status": order.target_status,
                    "geometry_status": order.geometry_status,
                    "execution_status": order.execution_status,
                    "assigned_units_status": order.assigned_units_status,
                    "order_linkage_status": order.order_linkage_status,
                    "stance_status": order.stance_status,
                    "aggressiveness_status": order.aggressiveness_status,
                })
            })
            .collect::<Vec<_>>();
        orders.sort_by(|a, b| {
            let ak = (
                a.get("army_id").and_then(Value::as_u64).unwrap_or(0),
                a.get("generation").and_then(Value::as_u64).unwrap_or(0),
                a.get("slot_index").and_then(Value::as_u64).unwrap_or(0),
            );
            let bk = (
                b.get("army_id").and_then(Value::as_u64).unwrap_or(0),
                b.get("generation").and_then(Value::as_u64).unwrap_or(0),
                b.get("slot_index").and_then(Value::as_u64).unwrap_or(0),
            );
            ak.cmp(&bk)
        });
        let mut provenance_notes = Vec::new();
        if self.army_battleplans_depth_omitted_non_exact_this_hour != 0 {
            provenance_notes.push(format!(
                "{} non-exact army battleplan depth row(s) omitted this hour",
                self.army_battleplans_depth_omitted_non_exact_this_hour
            ));
        }
        if self.army_battleplans_depth_path_omitted_this_hour != 0 {
            provenance_notes.push(format!(
                "{} invalid/overflow path geometry row(s) omitted this hour",
                self.army_battleplans_depth_path_omitted_this_hour
            ));
        }
        json!({
            "known": !self.army_battleplans_depth_unknown_this_hour
                && self.fold_domain_seen(ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID),
            "baseline_seen": self
                .delta_baselines_seen
                .contains(&ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID),
            "omitted_non_exact_rows_this_hour": self.army_battleplans_depth_omitted_non_exact_this_hour,
            "path_geometry_omitted_rows_this_hour": self.army_battleplans_depth_path_omitted_this_hour,
            "provenance_notes": provenance_notes,
            "orders": orders,
        })
    }

    fn build_naval_missions_status_extra(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        let oob_join_checked = self.naval_missions_status_oob_join_checked();
        let orphan_rows = self.naval_missions_status_orphan_rows().unwrap_or(0);
        let mut rows = Vec::with_capacity(self.naval_missions_status.len());
        for row in self.naval_missions_status.values() {
            let oob_command_task_force_seen =
                match self.naval_missions_status_oob_task_force_seen(row.task_force_id) {
                    Some(seen) => json!(seen),
                    None => Value::Null,
                };
            let (ship_count, dominant_hull_class) =
                self.naval_missions_status_ship_summary(row.task_force_id);
            rows.push(json!({
                "task_force_id": row.task_force_id,
                "fleet_id": row.fleet_id,
                "owner": self.country_ref_json(row.owner_tag_db_id, unresolved),
                "owner_tag": self.country_ref_json(row.owner_tag_db_id, unresolved),
                "owner_tag_db_id": row.owner_tag_db_id,
                "location_province_id": option_u32_json(row.location_province_id),
                "location_province": option_u32_json(row.location_province_id),
                "strategic_region_id": option_u32_json(row.strategic_region_id),
                "ship_count": ship_count,
                "dominant_hull_class": dominant_hull_class,
                "mission_enum": option_u32_json(row.mission_enum),
                "mission_state": option_u32_json(row.mission_state),
                "home_base_province_id": option_u32_json(row.home_base_province_id),
                "repair_base_province_id": option_u32_json(row.repair_base_province_id),
                "repair_progress_milli": option_u32_json(row.repair_progress_milli),
                "retreat_state": option_u32_json(row.retreat_state),
                "sunk_convoys": option_u32_json(row.sunk_convoys),
                "convoy_route_context_id": option_u32_json(row.convoy_route_context_id),
                "fixture_required": row.fixture_required,
                "active_battle_fixture_required": row.active_battle_fixture_required,
                "source_status_flags": row.source_status,
                "oob_command_task_force_seen": oob_command_task_force_seen,
            }));
        }
        rows.sort_by(|a, b| {
            let ak = (
                a.get("owner_tag_db_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                a.get("fleet_id").and_then(Value::as_u64).unwrap_or(0),
                a.get("task_force_id").and_then(Value::as_u64).unwrap_or(0),
            );
            let bk = (
                b.get("owner_tag_db_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                b.get("fleet_id").and_then(Value::as_u64).unwrap_or(0),
                b.get("task_force_id").and_then(Value::as_u64).unwrap_or(0),
            );
            ak.cmp(&bk)
        });
        json!({
            "source": "C",
            "confidence": "validated_slice",
            "projection": "R2 order_of_battle.extra annotation",
            "task_force_existence_owner_domain": "oob_command_live",
            "ship_existence_owner_domain": "ships",
            "task_force_count": rows.len(),
            "validated_location_rows": rows.iter().filter(|row| row.get("location_province_id") != Some(&Value::Null)).count(),
            "derived_region_rows": rows.iter().filter(|row| row.get("strategic_region_id") != Some(&Value::Null)).count(),
            "ship_summary_rows": rows.iter().filter(|row| row.get("ship_count") != Some(&Value::Null)).count(),
            "dominant_hull_rows": rows.iter().filter(|row| row.get("dominant_hull_class") != Some(&Value::Null)).count(),
            "fixture_required_rows": rows.iter().filter(|row| {
                row.get("fixture_required").and_then(Value::as_bool).unwrap_or(false)
                    || row.get("active_battle_fixture_required").and_then(Value::as_bool).unwrap_or(false)
            }).count(),
            "convoy_sunk_count_rows": rows.iter().filter(|row| row.get("sunk_convoys") != Some(&Value::Null)).count(),
            "oob_command_join_checked": oob_join_checked,
            "oob_command_orphan_rows": orphan_rows,
            "task_forces": rows,
        })
    }

    fn naval_missions_status_ship_summary(&self, task_force_id: u32) -> (Value, Value) {
        if self.fold_domain_unknown(SHIPS_DOMAIN_ID) || !self.fold_domain_seen(SHIPS_DOMAIN_ID) {
            return (Value::Null, Value::Null);
        }

        let mut ship_count = 0u64;
        let mut by_definition: BTreeMap<&str, u64> = BTreeMap::new();
        for ship in self
            .ships
            .values()
            .filter(|ship| ship.task_force_id == Some(task_force_id))
        {
            ship_count = ship_count.saturating_add(1);
            if let Some(definition) = ship.definition.as_deref() {
                *by_definition.entry(definition).or_default() += 1;
            }
        }

        let mut dominant: Option<(&str, u64)> = None;
        for (definition, count) in by_definition {
            match dominant {
                None => dominant = Some((definition, count)),
                Some((best_definition, best_count))
                    if count > best_count
                        || (count == best_count && definition < best_definition) =>
                {
                    dominant = Some((definition, count));
                }
                _ => {}
            }
        }

        (
            json!(ship_count),
            dominant.map_or(Value::Null, |(definition, _count)| json!(definition)),
        )
    }

    fn naval_missions_status_oob_join_checked(&self) -> bool {
        self.fold_domain_seen(OOB_COMMAND_DOMAIN_ID)
            && !self.fold_domain_unknown(OOB_COMMAND_DOMAIN_ID)
    }

    fn naval_missions_status_oob_task_force_seen(&self, task_force_id: u32) -> Option<bool> {
        if !self.naval_missions_status_oob_join_checked() {
            return None;
        }
        Some(self.oob_commands.contains_key(&OobCommandKey {
            kind: OOB_KIND_TASK_FORCE,
            object_id: task_force_id,
        }))
    }

    fn naval_missions_status_orphan_rows(&self) -> Option<usize> {
        if !self.fold_domain_seen(NAVAL_MISSIONS_STATUS_DOMAIN_ID)
            || self.fold_domain_unknown(NAVAL_MISSIONS_STATUS_DOMAIN_ID)
            || !self.naval_missions_status_oob_join_checked()
        {
            return None;
        }
        Some(
            self.naval_missions_status
                .keys()
                .filter(|task_force_id| {
                    self.naval_missions_status_oob_task_force_seen(**task_force_id) == Some(false)
                })
                .count(),
        )
    }

    fn ship_owner_key(&self, ship: &ShipState, unresolved: &mut BTreeSet<u32>) -> Option<String> {
        (ship.owner_tag_db_id != 0)
            .then(|| self.resolve_country_key(&ship.owner_tag_db_id.to_string(), unresolved))
    }

    fn air_wing_owner_key(
        &self,
        key: &AirWingKey,
        unresolved: &mut BTreeSet<u32>,
    ) -> Option<String> {
        (key.owner_tag_db_id != 0)
            .then(|| self.resolve_country_key(&key.owner_tag_db_id.to_string(), unresolved))
    }

    fn ship_json(&self, ship: &ShipState, unresolved: &mut BTreeSet<u32>) -> Value {
        json!({
            "id": ship.instance_id,
            "task_force": ship.task_force_id,
            "definition": ship.definition,
            "category": ship.category_enum,
            "strength": ship.strength_raw as f64 / FIXED_SCALE,
            "organisation": ship.organisation_raw as f64 / FIXED_SCALE,
            "experience": ship.experience_raw as f64 / FIXED_SCALE,
            "sunk_convoys": ship.sunk_convoys,
            "_r2_ship_owner": {
                "owner_tag_db_id": ship.owner_tag_db_id,
                "owner": self.country_ref_json(ship.owner_tag_db_id, unresolved),
            },
        })
    }

    fn build_designs(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        let mut out = Map::new();
        let mut live_variant_ids: BTreeSet<u32> = BTreeSet::new();
        live_variant_ids.extend(self.air_variant_defs.keys().copied());
        live_variant_ids.extend(self.air_variant_stats.keys().copied());
        live_variant_ids.extend(self.air_variant_upgrades.keys().copied());
        live_variant_ids.extend(self.air_variant_archetypes.keys().copied());
        if !live_variant_ids.is_empty()
            || self.fold_domain_unknown(AIR_VARIANT_DEFS_DOMAIN_ID)
            || self.fold_domain_unknown(AIR_VARIANT_STATS_DOMAIN_ID)
            || self.fold_domain_unknown(AIR_VARIANT_UPGRADES_DOMAIN_ID)
            || self.fold_domain_unknown(AIR_VARIANT_ARCHETYPES_DOMAIN_ID)
        {
            if live_variant_ids.is_empty() {
                out.insert("equipment_variants_live".into(), Value::Null);
            } else {
                let mut variants = Map::new();
                for variant_id in live_variant_ids {
                    let mut variant = Map::new();
                    variant.insert("variant_id".into(), json!(variant_id));
                    variant.insert("source".into(), json!("C"));
                    if self.fold_domain_unknown(AIR_VARIANT_DEFS_DOMAIN_ID) {
                        variant.insert("live_definition".into(), Value::Null);
                    } else if let Some(definition) = self.air_variant_defs.get(&variant_id) {
                        variant.insert("live_definition".into(), definition.to_json());
                    }
                    if self.fold_domain_unknown(AIR_VARIANT_STATS_DOMAIN_ID) {
                        variant.insert("live_stats".into(), Value::Null);
                    } else if let Some(stats) = self.air_variant_stats.get(&variant_id) {
                        variant.insert("live_stats".into(), stats.to_json());
                    }
                    if self.fold_domain_unknown(AIR_VARIANT_UPGRADES_DOMAIN_ID) {
                        variant.insert("upgrade_levels".into(), Value::Null);
                    } else if let Some(upgrades) = self.air_variant_upgrades.get(&variant_id) {
                        variant.insert(
                            "upgrade_levels".into(),
                            Value::Array(
                                upgrades.iter().map(VariantUpgradeState::to_json).collect(),
                            ),
                        );
                    }
                    if self.fold_domain_unknown(AIR_VARIANT_ARCHETYPES_DOMAIN_ID) {
                        variant.insert("archetype".into(), Value::Null);
                        variant.insert("_r2_air_variant_archetypes".into(), Value::Null);
                    } else if let Some(archetype) = self.air_variant_archetypes.get(&variant_id) {
                        variant.insert(
                            "archetype".into(),
                            archetype
                                .archetype_token
                                .as_ref()
                                .map(|value| json!(value))
                                .unwrap_or(Value::Null),
                        );
                        variant.insert("_r2_air_variant_archetypes".into(), archetype.to_json());
                    }
                    variants.insert(variant_id.to_string(), Value::Object(variant));
                }
                out.insert("equipment_variants_live".into(), Value::Object(variants));
            }
        }

        if self.fold_domain_unknown(STOCKPILE_VARIANT_DEFS_DOMAIN_ID) {
            out.insert("_r2_stockpile_variant_defs".into(), Value::Null);
        } else if self.fold_domain_seen(STOCKPILE_VARIANT_DEFS_DOMAIN_ID) {
            out.insert(
                "_r2_stockpile_variant_defs".into(),
                Value::Object(
                    self.stockpile_variant_defs
                        .iter()
                        .map(|(variant_id, definition)| {
                            (variant_id.to_string(), definition.to_json())
                        })
                        .collect(),
                ),
            );
        }
        if self.fold_domain_unknown(STOCKPILE_VARIANT_UPGRADES_DOMAIN_ID) {
            out.insert("_r2_stockpile_variant_upgrades".into(), Value::Null);
        } else if self.fold_domain_seen(STOCKPILE_VARIANT_UPGRADES_DOMAIN_ID) {
            out.insert(
                "_r2_stockpile_variant_upgrades".into(),
                Value::Object(
                    self.stockpile_variant_upgrades
                        .iter()
                        .map(|(variant_id, upgrades)| {
                            (
                                variant_id.to_string(),
                                Value::Array(
                                    upgrades.iter().map(VariantUpgradeState::to_json).collect(),
                                ),
                            )
                        })
                        .collect(),
                ),
            );
        }
        if self.fold_domain_unknown(AIR_VARIANT_RESERVES_DOMAIN_ID) {
            out.insert("_r2_air_variant_reserves".into(), Value::Null);
        } else if self.fold_domain_seen(AIR_VARIANT_RESERVES_DOMAIN_ID) {
            out.insert(
                "_r2_air_variant_reserves".into(),
                Value::Array(
                    self.air_variant_reserves
                        .values()
                        .map(|reserve| {
                            json!({
                                "owner": self.country_ref_json(reserve.tag_db_id, unresolved),
                                "tag_db_id": reserve.tag_db_id,
                                "variant_id": reserve.variant_id,
                                "amount": reserve.amount_raw as f64 / FIXED_SCALE,
                                "amount_raw": reserve.amount_raw,
                                "scale_raw": reserve.scale_raw,
                            })
                        })
                        .collect(),
                ),
            );
        }
        Value::Object(out)
    }

    // Naval combat (domains 49/50) snapshot section: the current active set of
    // battles keyed by "combat_id:generation", each with its promoted location
    // /region, side counts + positioning, and validated member roster. Carried
    // cumulative state; emitted every hour (empty object once all battles end).
    fn build_naval_combats(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        if !self.naval_combat_lifecycle_known_this_hour {
            return Value::Null;
        }
        let mut battles = Map::new();
        for combat in self.naval_combats.values() {
            let mut obj = Map::new();
            obj.insert("combat_id".into(), json!(combat.combat_id));
            obj.insert("naval_combat_id".into(), json!(combat.combat_id));
            obj.insert("generation".into(), json!(combat.generation));
            obj.insert("kind".into(), json!("naval"));
            obj.insert(
                "location_province".into(),
                option_u32_json(combat.location_id),
            );
            obj.insert("region_id".into(), option_u32_json(combat.region_id));
            obj.insert(
                "attacker_member_count".into(),
                option_u32_json(combat.attacker_member_count),
            );
            obj.insert(
                "defender_member_count".into(),
                option_u32_json(combat.defender_member_count),
            );
            obj.insert(
                "attacker_positioning".into(),
                combat
                    .attacker_positioning_milli
                    .map(|milli| json!(f64::from(milli) / 1000.0))
                    .unwrap_or(Value::Null),
            );
            obj.insert(
                "defender_positioning".into(),
                combat
                    .defender_positioning_milli
                    .map(|milli| json!(f64::from(milli) / 1000.0))
                    .unwrap_or(Value::Null),
            );
            if self.naval_combat_members_known_this_hour {
                let mut attacker_task_force_ids = BTreeSet::new();
                let mut defender_task_force_ids = BTreeSet::new();
                let members = combat
                    .members
                    .values()
                    .map(|member| {
                        if let Some(task_force_id) = member.task_force_id {
                            match member.side.name() {
                                "attacker" => {
                                    attacker_task_force_ids.insert(task_force_id);
                                }
                                "defender" => {
                                    defender_task_force_ids.insert(task_force_id);
                                }
                                _ => {}
                            }
                        }
                        if let Some(owner) = member.owner_tag_db_id {
                            // Record the owner id for tag-resolution accounting; the
                            // ref itself is emitted numeric (no country re-key here).
                            let _ = self.country_ref_json(owner, unresolved);
                        }
                        json!({
                            "side": member.side.name(),
                            "group_index": member.group_index,
                            "member_index": member.member_index,
                            "is_convoy": member.is_convoy,
                            "ship_instance_id": option_u32_json(member.ship_instance_id),
                            "cship_join_validated": member.cship_join_validated,
                            "owner_tag_db_id": option_u32_json(member.owner_tag_db_id),
                            "task_force_id": option_u32_json(member.task_force_id),
                            "fleet_id": option_u32_json(member.fleet_id),
                            "battle_slot_index": member.battle_slot_index_raw,
                            "state_bitmask": option_u32_json(member.state_bitmask),
                            "strength": member.strength_raw.map(|v| f64::from(v) / FIXED_SCALE).map(|v| json!(v)).unwrap_or(Value::Null),
                            "organisation": member.organisation_raw.map(|v| f64::from(v) / FIXED_SCALE).map(|v| json!(v)).unwrap_or(Value::Null),
                            "sunk_convoys": option_u32_json(member.sunk_convoys),
                        })
                    })
                    .collect::<Vec<_>>();
                obj.insert(
                    "attacker_task_force_ids".into(),
                    Value::Array(
                        attacker_task_force_ids
                            .into_iter()
                            .map(Value::from)
                            .collect(),
                    ),
                );
                obj.insert(
                    "defender_task_force_ids".into(),
                    Value::Array(
                        defender_task_force_ids
                            .into_iter()
                            .map(Value::from)
                            .collect(),
                    ),
                );
                obj.insert("members".into(), Value::Array(members));
            }
            battles.insert(
                format!("{}:{}", combat.combat_id, combat.generation),
                Value::Object(obj),
            );
        }
        Value::Object(battles)
    }

    fn build_naval_air(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        let mut out = Map::new();
        self.insert_naval_supremacy(&mut out, unresolved);
        self.insert_air_region_activity(&mut out, unresolved);
        self.insert_air_superiority_inputs(&mut out, unresolved);
        self.insert_air_combat_losses(&mut out, unresolved);
        self.insert_air_loss_stats(&mut out, unresolved);
        Value::Object(out)
    }

    fn build_strategic_air(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        let mut out = Map::new();
        if self.fold_domain_unknown(AIR_BASE_STATUS_DOMAIN_ID) {
            out.insert("air_bases".into(), Value::Null);
        } else if self.fold_domain_seen(AIR_BASE_STATUS_DOMAIN_ID) {
            out.insert(
                "air_bases".into(),
                Value::Array(
                    self.air_base_status
                        .values()
                        .map(|base| {
                            json!({
                                "id": base.air_base_id,
                                "state": base.state_id,
                                "capacity": base.capacity,
                                "level": base.building_level,
                                "_r2_air_base_status": {
                                    "building_level": base.building_level,
                                    "based_wing_count": base.based_wing_count,
                                    "based_plane_count": base.based_plane_count,
                                },
                            })
                        })
                        .collect(),
                ),
            );
        }
        self.insert_air_region_activity(&mut out, unresolved);
        self.insert_air_superiority_inputs(&mut out, unresolved);
        self.insert_air_combat_losses(&mut out, unresolved);
        self.insert_air_loss_stats(&mut out, unresolved);
        Value::Object(out)
    }

    fn insert_air_region_activity(
        &self,
        out: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        if self.fold_domain_unknown(AIR_REGION_ACTIVITY_DOMAIN_ID) {
            out.insert("_r2_air_region_activity".into(), Value::Null);
        } else if self.fold_domain_seen(AIR_REGION_ACTIVITY_DOMAIN_ID) {
            out.insert(
                "_r2_air_region_activity".into(),
                Value::Array(
                    self.air_region_activity
                        .values()
                        .map(|row| {
                            json!({
                                "region_id": row.region_id,
                                "owner": self.country_ref_json(row.owner_tag_db_id, unresolved),
                                "owner_tag_db_id": row.owner_tag_db_id,
                                "mission_mask": row.mission_mask,
                                "wing_count": row.wing_count,
                                "plane_count": row.plane_count,
                                "effective_planes": row.effective_planes,
                            })
                        })
                        .collect(),
                ),
            );
        }
    }

    /// D56 dominance values are CFixedPoint (`raw / 100_000`) naval-dominance
    /// points, not a normalized 0..1 fraction. The reader's live rows decode to
    /// values such as 679.628, and the FILE define
    /// `NAVAL_REGION_TRANSPARENCY_RANGE = 2000.0` consumes those point units.
    /// Only `individual_ratio` is a normalized ratio.
    fn insert_naval_supremacy(&self, out: &mut Map<String, Value>, unresolved: &mut BTreeSet<u32>) {
        if self.fold_domain_unknown(NAVAL_MISSIONS_STATUS_DOMAIN_ID) {
            out.insert("_r2_naval_supremacy".into(), Value::Null);
        } else if self.fold_domain_seen(NAVAL_MISSIONS_STATUS_DOMAIN_ID) {
            // Stable sort by (region, controller); repeated keys retain their
            // emit order so distinct occurrences stay deterministic.
            let mut rows = self
                .naval_supremacy
                .iter()
                .map(|row| {
                    json!({
                        "region_id": row.region_id,
                        "controller": self.country_ref_json(row.controller_tag_db_id, unresolved),
                        "controller_tag_db_id": row.controller_tag_db_id,
                        "current": row.current_raw as f64 / FIXED_SCALE,
                        "target": row.target_raw as f64 / FIXED_SCALE,
                        "base_target": row.base_target_raw as f64 / FIXED_SCALE,
                        "previous": row.previous_raw as f64 / FIXED_SCALE,
                        "decline_from": row.decline_from_raw as f64 / FIXED_SCALE,
                        "individual_ratio": row.individual_ratio_raw as f64 / FIXED_SCALE,
                        "current_raw": row.current_raw,
                        "target_raw": row.target_raw,
                        "base_target_raw": row.base_target_raw,
                        "previous_raw": row.previous_raw,
                        "decline_from_raw": row.decline_from_raw,
                        "individual_ratio_raw": row.individual_ratio_raw,
                    })
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| {
                let ak = (
                    a.get("region_id").and_then(Value::as_u64).unwrap_or(0),
                    a.get("controller_tag_db_id")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                let bk = (
                    b.get("region_id").and_then(Value::as_u64).unwrap_or(0),
                    b.get("controller_tag_db_id")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                ak.cmp(&bk)
            });
            out.insert(
                "_r2_naval_supremacy".into(),
                json!({
                    "source": "C",
                    "confidence": "validated_slice",
                    "projection": "R6 per-strategic-region naval supremacy",
                    "scale_raw": FIXED_SCALE as u32,
                    "row_count": rows.len(),
                    "rows": rows,
                }),
            );
        }
    }

    fn insert_air_superiority_inputs(
        &self,
        out: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        if self.fold_domain_unknown(AIR_SUPERIORITY_INPUTS_DOMAIN_ID) {
            out.insert("_r2_air_superiority_inputs".into(), Value::Null);
        } else if self.fold_domain_seen(AIR_SUPERIORITY_INPUTS_DOMAIN_ID) {
            out.insert(
                "_r2_air_superiority_inputs".into(),
                Value::Array(
                    self.air_superiority_inputs
                        .values()
                        .map(|row| {
                            json!({
                                "region_id": row.region_id,
                                "owner": self.country_ref_json(row.owner_tag_db_id, unresolved),
                                "owner_tag_db_id": row.owner_tag_db_id,
                                "mission_mask": row.mission_mask,
                                "variant_id": row.variant_id,
                                "equipment_token_raw": row.equipment_token_raw,
                                "equipment_type_token_raw": row.equipment_type_token_raw,
                                "interface_type_raw": row.interface_type_raw,
                                "wing_count": row.wing_count,
                                "amount": row.amount_raw as f64 / FIXED_SCALE,
                                "amount_raw": row.amount_raw,
                                "scale_raw": row.scale_raw,
                            })
                        })
                        .collect(),
                ),
            );
        }
    }

    fn insert_air_combat_losses(
        &self,
        out: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        if self.fold_domain_unknown(AIR_COMBAT_LOSSES_DOMAIN_ID) {
            out.insert("_r2_air_combat_losses_live".into(), Value::Null);
            out.insert("_r2_air_wing_cumulative_scalars".into(), Value::Null);
        } else if let Some(losses) = &self.air_combat_losses_this_hour {
            out.insert(
                "_r2_air_combat_losses_live".into(),
                Value::Array(
                    losses
                        .iter()
                        .map(|row| row.to_json(self, unresolved))
                        .collect(),
                ),
            );
            out.insert(
                "_r2_air_wing_cumulative_scalars".into(),
                Value::Array(
                    self.air_wing_cumulative
                        .iter()
                        .map(|(wing_id, state)| {
                            json!({
                                "wing_id": wing_id,
                                "owner": self
                                    .country_ref_json(state.owner_tag_db_id, unresolved),
                                "owner_tag_db_id": state.owner_tag_db_id,
                                "air_accidents": state.air_accidents,
                                "experience": state.experience_raw as f64 / FIXED_SCALE,
                                "experience_raw": state.experience_raw,
                            })
                        })
                        .collect(),
                ),
            );
        }
    }

    fn insert_air_loss_stats(&self, out: &mut Map<String, Value>, unresolved: &mut BTreeSet<u32>) {
        if self.fold_domain_unknown(AIR_LOSS_STATS_DOMAIN_ID) {
            out.insert("_r2_air_loss_stats_live".into(), Value::Null);
        } else if self.fold_domain_seen(AIR_LOSS_STATS_DOMAIN_ID) {
            out.insert(
                "_r2_air_loss_stats_live".into(),
                Value::Array(
                    self.air_loss_stats
                        .values()
                        .map(|row| {
                            let mut channels = Map::new();
                            let mut raw = Map::new();
                            for (index, (_, label)) in airloss61::PINNED_CHANNELS.iter().enumerate()
                            {
                                channels.insert(
                                    (*label).into(),
                                    json!(
                                        row.channels[index] as f64 / airloss61::FIELD_SCALE as f64
                                    ),
                                );
                                raw.insert((*label).into(), json!(row.channels[index]));
                            }
                            json!({
                                "country": self.country_ref_json(row.owner_tag_db_id, unresolved),
                                "owner_tag_db_id": row.owner_tag_db_id,
                                "country_idx": row.country_idx,
                                "channels": channels,
                                "raw": raw,
                                "scale_raw": airloss61::FIELD_SCALE,
                            })
                        })
                        .collect(),
                ),
            );
        }
    }

    fn insert_hub_motorization(
        &self,
        supply: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        if self.fold_domain_unknown(HUB_MOTORIZATION_DOMAIN_ID) {
            supply.insert("_r2_hub_motorization_live".into(), Value::Null);
        } else if self.fold_domain_seen(HUB_MOTORIZATION_DOMAIN_ID) {
            supply.insert(
                "_r2_hub_motorization_live".into(),
                Value::Array(
                    self.hub_motorization
                        .values()
                        .map(|row| {
                            json!({
                                "country": self.country_ref_json(row.owner_tag_db_id, unresolved),
                                "owner_tag_db_id": row.owner_tag_db_id,
                                "province_id": row.province_id,
                                "state_id": row.state_id,
                                "record_index": row.record_index,
                                "map_size": row.map_size,
                                "node_motorized": true,
                                "scope": "node_membership",
                            })
                        })
                        .collect(),
                ),
            );
        }
    }

    // --- DOMAIN 10 fuel_live (full each frame) ----------------------------
    fn fold_fuel(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 24, "fuel header")?;
        let row_count = read_u32(payload, 8) as usize;
        let expected = 24 + row_count * 40;
        if payload.len() != expected {
            return Err(format!("fuel len {} != {expected}", payload.len()));
        }
        for i in 0..row_count {
            let base = 24 + i * 40;
            let tag_db_id = read_u32(payload, base);
            if tag_db_id == 0 {
                return Err("fuel row with zero tag_db_id".into());
            }
            let current = read_i64(payload, base + 8) as f64 / FUEL_SCALE;
            let max = read_i64(payload, base + 16) as f64 / FUEL_SCALE;
            let gain = read_i64(payload, base + 24) as f64 / FUEL_SCALE;
            let cost = read_i64(payload, base + 32) as f64 / FUEL_SCALE;
            let country = self.country_mut(tag_db_id);
            country.insert(
                "fuel".into(),
                json!({ "current": current, "max": max, "gain": gain, "cost": cost }),
            );
        }
        Ok(())
    }

    // --- DOMAIN 63 country_core_scalars_live (full each frame) ---------
    // W1: per-country core economy/counters — manpower_pool, convoys_destroyed,
    // divisions_lost, and the 7-channel experience_status. Each field is
    // presence-flagged: a clear flag folds `null` (absent != 0); a set flag on
    // a validated value folds the value. Full-replace semantics (each frame the
    // collector re-reads every country).
    fn fold_country_core_scalars(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 20, "country_core_scalars header")?;
        let row_count = read_u32(payload, 8) as usize;
        let expected = 20 + row_count * COUNTRY_CORE_ROW_LEN;
        if payload.len() != expected {
            return Err(format!(
                "country_core_scalars len {} != {expected}",
                payload.len()
            ));
        }
        for i in 0..row_count {
            let base = 20 + i * COUNTRY_CORE_ROW_LEN;
            let tag_db_id = read_u32(payload, base);
            if tag_db_id == 0 {
                return Err("country_core_scalars row with zero tag_db_id".into());
            }
            let flags = read_u32(payload, base + 4);
            let manpower_pool = read_u32(payload, base + 8);
            let convoys_destroyed = read_u32(payload, base + 12);
            let divisions_lost = read_u32(payload, base + 16);
            let exp_army = read_i32(payload, base + 20);
            let exp_army_daily = read_i32(payload, base + 24);
            let exp_navy = read_i32(payload, base + 28);
            let exp_navy_daily = read_i32(payload, base + 32);
            let exp_air = read_i32(payload, base + 36);
            let exp_air_daily = read_i32(payload, base + 40);
            let exp_num_armies = read_i32(payload, base + 44);

            let has_manpower = flags & COUNTRY_CORE_FLAG_MANPOWER_POOL != 0;
            let has_convoys = flags & COUNTRY_CORE_FLAG_CONVOYS_DESTROYED != 0;
            let has_divisions_lost = flags & COUNTRY_CORE_FLAG_DIVISIONS_LOST != 0;
            let has_experience = flags & COUNTRY_CORE_FLAG_EXPERIENCE != 0;

            // Validate presence/range: a present value must be in range; an
            // absent field must pack a zero (fail closed on a malformed row).
            validate_optional_u32_field(
                has_manpower,
                manpower_pool,
                0,
                COUNTRY_CORE_MAX_MANPOWER_POOL,
                "country_manpower_pool",
            )?;
            validate_optional_u32_field(
                has_convoys,
                convoys_destroyed,
                0,
                COUNTRY_CORE_MAX_CONVOYS_DESTROYED,
                "convoys_destroyed",
            )?;
            validate_optional_u32_field(
                has_divisions_lost,
                divisions_lost,
                0,
                COUNTRY_CORE_MAX_DIVISIONS_LOST,
                "divisions_lost",
            )?;
            // Experience channels only carry meaning when the flag is set; when
            // absent they must be zero-packed.
            if !has_experience
                && (exp_army != 0
                    || exp_army_daily != 0
                    || exp_navy != 0
                    || exp_navy_daily != 0
                    || exp_air != 0
                    || exp_air_daily != 0
                    || exp_num_armies != 0)
            {
                return Err("country_core_scalars experience present without flag".into());
            }

            let country = self.country_mut(tag_db_id);
            country.insert(
                "manpower_pool".into(),
                if has_manpower {
                    json!(manpower_pool)
                } else {
                    Value::Null
                },
            );
            country.insert(
                "convoys_destroyed".into(),
                if has_convoys {
                    json!(convoys_destroyed)
                } else {
                    Value::Null
                },
            );
            country.insert(
                "divisions_lost".into(),
                if has_divisions_lost {
                    json!(divisions_lost)
                } else {
                    Value::Null
                },
            );
            country.insert(
                "experience_status".into(),
                if has_experience {
                    let s = COUNTRY_CORE_EXPERIENCE_SCALE;
                    json!({
                        "army_experience": exp_army as f64 / s,
                        "army_experience_daily": exp_army_daily as f64 / s,
                        "navy_experience": exp_navy as f64 / s,
                        "navy_experience_daily": exp_navy_daily as f64 / s,
                        "air_experience": exp_air as f64 / s,
                        "air_experience_daily": exp_air_daily as f64 / s,
                        "num_armies_for_training": exp_num_armies as f64 / s,
                    })
                } else {
                    Value::Null
                },
            );
        }
        Ok(())
    }

    // --- W3 DOMAIN 64 delayed_events_live (full replace) ----------------
    // Per-country pending scripted delayed events, rooted at CCountry+0x1290.
    // Each row: {owner tag_db_id, event_id, originator?, due_remaining_hours?}.
    // due_remaining_hours is a RELATIVE countdown (hours) — the replay derives
    // the absolute due date = current game clock + remaining. Full-replace: the
    // per-country `delayed_events` array is rebuilt every frame (a country with
    // no rows this frame gets an empty array, distinguishing "measured, none"
    // from a failed frame which retains the prior map via handle_compact_failure).
    fn fold_delayed_events(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 20, "delayed_events header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let expected = 20 + row_count * DELAYED_EVENT_ROW_LEN;
        if payload.len() != expected {
            return Err(format!(
                "delayed_events len {} != {expected}",
                payload.len()
            ));
        }
        if self.handle_compact_failure(DELAYED_EVENT_DOMAIN_ID, "delayed_events", status_flags) {
            return Ok(());
        }
        // Group rows by owner (a country can have multiple pending events).
        let mut per_owner: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
        for i in 0..row_count {
            let base = 20 + i * DELAYED_EVENT_ROW_LEN;
            let owner_tag_db_id = read_u32(payload, base);
            let event_id = read_u32(payload, base + 4);
            let flags = read_u32(payload, base + 8);
            let originator = read_u32(payload, base + 12);
            let due_remaining_hours = read_u32(payload, base + 16);
            if owner_tag_db_id == 0 {
                return Err("delayed_events row with zero owner tag_db_id".into());
            }
            if event_id == 0 {
                return Err("delayed_events row with zero event_id".into());
            }
            if flags & !DELAYED_EVENT_KNOWN_FLAGS != 0 {
                return Err(format!("delayed_events row {i} unknown flags {flags:#x}"));
            }
            let has_due = flags & DELAYED_EVENT_FLAG_DUE_REMAINING_HOURS != 0;
            let has_originator = flags & DELAYED_EVENT_FLAG_ORIGINATOR != 0;
            // An absent field must pack a zero (fail closed on a malformed row).
            if !has_due && due_remaining_hours != 0 {
                return Err(format!(
                    "delayed_events row {i} due_remaining_hours without flag"
                ));
            }
            if has_due && due_remaining_hours > DELAYED_EVENT_MAX_DUE_REMAINING_HOURS {
                return Err(format!(
                    "delayed_events row {i} due_remaining_hours {due_remaining_hours} out of range"
                ));
            }
            if !has_originator && originator != 0 {
                return Err(format!("delayed_events row {i} originator without flag"));
            }
            per_owner.entry(owner_tag_db_id).or_default().push(json!({
                "event_id": event_id,
                "due_remaining_hours": if has_due {
                    json!(due_remaining_hours)
                } else {
                    Value::Null
                },
                "originator_tag_db_id": if has_originator {
                    json!(originator)
                } else {
                    Value::Null
                },
            }));
        }
        // Full-replace: clear every known country's list, then set the measured
        // lists. Clearing first ensures a country whose last event fired (no row
        // this frame) shows an empty list, not a stale one.
        let known: Vec<String> = self.countries.keys().cloned().collect();
        for key in known {
            if let Ok(id) = key.parse::<u32>() {
                self.country_mut(id)
                    .insert("delayed_events".into(), Value::Array(Vec::new()));
            }
        }
        for (owner, events) in per_owner {
            self.country_mut(owner)
                .insert("delayed_events".into(), Value::Array(events));
        }
        Ok(())
    }

    // --- W5 DOMAIN 67 country_dynamic_modifiers_live (full replace) -----
    // Per-country OWN active dynamic-modifier id SET, rooted at the inline
    // CDynamicModifierContainer (CCountry+0xE58). The wire payload is a fixed
    // 20-byte header, then country_count fixed 12-byte rows
    // (tag_db_id, id_count, id_blob_offset), then a single concatenated id blob
    // where each id is a u16 length prefix + utf8 bytes. Each country's ids are
    // `id_count` consecutive blob entries starting at `id_blob_offset`. Emits
    // `countries.<TAG>.dynamic_modifier = [ids...]` (SORTED for a stable
    // multiset comparison against the melt). Full-replace: every known country's
    // list is cleared first so a country whose last modifier expired shows an
    // empty list, not a stale one. Any bounds/utf8/torn failure returns Err so
    // the whole frame folds closed (prior map retained) — no partial rows.
    fn fold_country_dynamic_modifiers(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN,
            "country_dynamic_modifiers header",
        )?;
        let country_count = read_u32(payload, 8) as usize;
        let total_id_count = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        if self.handle_compact_failure(
            COUNTRY_DYNAMIC_MODIFIERS_DOMAIN_ID,
            "country_dynamic_modifiers",
            status_flags,
        ) {
            return Ok(());
        }
        let table_len = COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN
            + country_count
                .checked_mul(COUNTRY_DYNAMIC_MODIFIERS_ROW_LEN)
                .ok_or("country_dynamic_modifiers row table overflow")?;
        need(payload, table_len, "country_dynamic_modifiers row table")?;

        // Guard the declared id count BEFORE allocating: each on-wire id needs at
        // least 3 bytes (u16 length prefix + >=1 byte), so total_id_count can
        // never exceed the blob's byte budget. This bounds the with_capacity
        // allocation so a crafted header cannot request a huge allocation.
        let blob_budget = payload.len().saturating_sub(table_len);
        if total_id_count > blob_budget / 3 {
            return Err(format!(
                "country_dynamic_modifiers total_id_count {total_id_count} exceeds blob budget {}",
                blob_budget / 3
            ));
        }

        // First pass: parse the id blob into a flat Vec<String>, validating
        // every length prefix stays in bounds. The blob starts after the row
        // table and must contain exactly total_id_count entries that consume the
        // remainder of the payload exactly (no trailing/short bytes).
        let mut ids: Vec<String> = Vec::with_capacity(total_id_count);
        let mut cursor = table_len;
        for _ in 0..total_id_count {
            if cursor + 2 > payload.len() {
                return Err("country_dynamic_modifiers id blob truncated (length prefix)".into());
            }
            let len = read_u16(payload, cursor) as usize;
            cursor += 2;
            if len == 0 || len > COUNTRY_DYNAMIC_MODIFIER_MAX_ID_LEN {
                return Err(format!(
                    "country_dynamic_modifiers id length {len} out of range"
                ));
            }
            if cursor + len > payload.len() {
                return Err("country_dynamic_modifiers id blob truncated (bytes)".into());
            }
            let id = std::str::from_utf8(&payload[cursor..cursor + len])
                .map_err(|_| "country_dynamic_modifiers id not utf8".to_string())?;
            ids.push(id.to_string());
            cursor += len;
        }
        if cursor != payload.len() {
            return Err(format!(
                "country_dynamic_modifiers trailing bytes: consumed {cursor} of {}",
                payload.len()
            ));
        }

        // Second pass: assemble each country's id set from the blob slice its
        // row references. The collector writes CONTIGUOUS, non-overlapping
        // windows in row order, so we require each row's id_blob_offset to equal
        // a running cursor and the final cursor to cover the id vector EXACTLY
        // (no holes, no overlaps, no orphan ids). This rejects a crafted payload
        // whose windows would double-count or ignore parsed ids.
        let mut per_country: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        let mut expected_offset = 0usize;
        for i in 0..country_count {
            let base = COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN + i * COUNTRY_DYNAMIC_MODIFIERS_ROW_LEN;
            let tag_db_id = read_u32(payload, base);
            let id_count = read_u32(payload, base + 4) as usize;
            let id_blob_offset = read_u32(payload, base + 8) as usize;
            if tag_db_id == 0 {
                return Err("country_dynamic_modifiers row with zero tag_db_id".into());
            }
            if id_count == 0 {
                return Err(format!(
                    "country_dynamic_modifiers row {i} has zero id_count (empty countries emit no row)"
                ));
            }
            if id_blob_offset != expected_offset {
                return Err(format!(
                    "country_dynamic_modifiers row {i} id_blob_offset {id_blob_offset} != expected {expected_offset} (non-contiguous)"
                ));
            }
            let end = id_blob_offset
                .checked_add(id_count)
                .ok_or("country_dynamic_modifiers id window overflow")?;
            if end > ids.len() {
                return Err(format!(
                    "country_dynamic_modifiers row {i} id window [{id_blob_offset},{end}) exceeds {} ids",
                    ids.len()
                ));
            }
            if per_country.contains_key(&tag_db_id) {
                return Err(format!(
                    "country_dynamic_modifiers duplicate tag_db_id {tag_db_id}"
                ));
            }
            let mut set: Vec<String> = ids[id_blob_offset..end].to_vec();
            // Sort for a stable, order-independent multiset representation.
            set.sort();
            per_country.insert(tag_db_id, set);
            expected_offset = end;
        }
        // Every parsed id must belong to exactly one country window.
        if expected_offset != ids.len() {
            return Err(format!(
                "country_dynamic_modifiers windows cover {expected_offset} of {} ids (orphan ids)",
                ids.len()
            ));
        }

        // Full-replace: clear every known country's list, then set the measured
        // sets. A country with no row this frame shows an empty list.
        let known: Vec<String> = self.countries.keys().cloned().collect();
        for key in known {
            if let Ok(id) = key.parse::<u32>() {
                self.country_mut(id)
                    .insert("dynamic_modifier".into(), Value::Array(Vec::new()));
            }
        }
        for (tag_db_id, set) in per_country {
            let arr = set.into_iter().map(Value::String).collect();
            self.country_mut(tag_db_id)
                .insert("dynamic_modifier".into(), Value::Array(arr));
        }
        Ok(())
    }

    // --- W6 DOMAIN 68 session_header_live (persistent last-value-wins) ---
    // TOP-LEVEL save/session header. Each present field UPDATES the carried
    // header; an absent field leaves the prior value in place. A hard-failure
    // frame retains the whole prior header. The wire payload is a 16-byte header
    // (elapsed_wall_ms u64, presence flags u32, status_flags u32), a fixed 20-byte
    // scalar block (5 x u32), then three length-prefixed utf8 strings (ideology,
    // cosmetic_tag, version). A field's value is meaningful only when its
    // presence bit is set; every parse failure returns Err (frame folds closed,
    // prior header retained) so a torn payload never overwrites good state.
    fn fold_session_header(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, SESSION_HEADER_HEADER_LEN, "session_header header")?;
        let presence = read_u32(payload, 8);
        let status_flags = read_u32(payload, 12);
        if self.handle_compact_failure(SESSION_HEADER_DOMAIN_ID, "session_header", status_flags) {
            return Ok(());
        }
        if presence & !SESSION_HEADER_KNOWN_FLAGS != 0 {
            return Err(format!(
                "session_header unknown presence flags {presence:#x}"
            ));
        }
        need(
            payload,
            SESSION_HEADER_HEADER_LEN + SESSION_HEADER_SCALAR_BLOCK_LEN,
            "session_header scalar block",
        )?;
        let scalars_base = SESSION_HEADER_HEADER_LEN;
        let player_tag_db_id = read_u32(payload, scalars_base);
        let dlcs = read_u32(payload, scalars_base + 4);
        let start_date_raw = read_u32(payload, scalars_base + 8);
        let save_version = read_u32(payload, scalars_base + 12);
        let session = read_u32(payload, scalars_base + 16);

        let has_player = presence & SESSION_HEADER_FLAG_PLAYER != 0;
        let has_dlcs = presence & SESSION_HEADER_FLAG_DLCS != 0;
        let has_start_date = presence & SESSION_HEADER_FLAG_START_DATE != 0;
        let has_save_version = presence & SESSION_HEADER_FLAG_SAVE_VERSION != 0;
        let has_session = presence & SESSION_HEADER_FLAG_SESSION != 0;
        let has_ideology = presence & SESSION_HEADER_FLAG_IDEOLOGY != 0;
        let has_cosmetic = presence & SESSION_HEADER_FLAG_COSMETIC_TAG != 0;
        let has_version = presence & SESSION_HEADER_FLAG_VERSION != 0;

        // An absent scalar must pack a zero (fail closed on a malformed row).
        if !has_player && player_tag_db_id != 0 {
            return Err("session_header player_tag_db_id without flag".into());
        }
        if has_player
            && (player_tag_db_id == 0 || player_tag_db_id > SESSION_HEADER_MAX_PLAYER_TAG_DB_ID)
        {
            return Err(format!(
                "session_header player_tag_db_id {player_tag_db_id} out of range"
            ));
        }
        if !has_dlcs && dlcs != 0 {
            return Err("session_header dlcs without flag".into());
        }
        if !has_start_date && start_date_raw != 0 {
            return Err("session_header start_date without flag".into());
        }
        if !has_save_version && save_version != 0 {
            return Err("session_header save_version without flag".into());
        }
        if !has_session && session != 0 {
            return Err("session_header session without flag".into());
        }

        // Three length-prefixed utf8 strings: ideology, cosmetic_tag, version.
        // Each is present in the blob (u16 len + bytes) regardless of its flag;
        // an absent field must pack len==0.
        let mut cursor = SESSION_HEADER_HEADER_LEN + SESSION_HEADER_SCALAR_BLOCK_LEN;
        let mut read_token = |flag_present: bool, name: &str| -> Result<Option<String>, String> {
            if cursor + 2 > payload.len() {
                return Err(format!("session_header {name} length prefix truncated"));
            }
            let len = read_u16(payload, cursor) as usize;
            cursor += 2;
            if len > SESSION_HEADER_MAX_STRING_LEN {
                return Err(format!("session_header {name} length {len} out of range"));
            }
            if cursor + len > payload.len() {
                return Err(format!("session_header {name} bytes truncated"));
            }
            let value = if len == 0 {
                None
            } else {
                Some(
                    std::str::from_utf8(&payload[cursor..cursor + len])
                        .map_err(|_| format!("session_header {name} not utf8"))?
                        .to_string(),
                )
            };
            cursor += len;
            if !flag_present && value.is_some() {
                return Err(format!("session_header {name} bytes without flag"));
            }
            if flag_present && value.is_none() {
                return Err(format!("session_header {name} flag set but empty"));
            }
            Ok(value)
        };
        let ideology = read_token(has_ideology, "ideology")?;
        let cosmetic_tag = read_token(has_cosmetic, "cosmetic_tag")?;
        let version = read_token(has_version, "version")?;
        if cursor != payload.len() {
            return Err(format!(
                "session_header trailing bytes: consumed {cursor} of {}",
                payload.len()
            ));
        }

        // Merge present fields into the carried header (last-value-wins). Static
        // fields never change; player/ideology/cosmetic_tag refresh on change.
        let header = self.session_header.get_or_insert_with(Default::default);
        if has_player {
            header.player_tag_db_id = Some(player_tag_db_id);
        }
        if has_dlcs {
            header.dlcs = Some(dlcs);
        }
        if has_start_date {
            header.start_date_raw = Some(start_date_raw);
        }
        if has_save_version {
            header.save_version = Some(save_version);
        }
        if has_session {
            header.session = Some(session);
        }
        if has_ideology {
            header.ideology = ideology;
        }
        if has_cosmetic {
            header.cosmetic_tag = cosmetic_tag;
        }
        if has_version {
            header.version = version;
        }
        self.mark_fold_domain_success(SESSION_HEADER_DOMAIN_ID);
        Ok(())
    }

    // --- W6 DOMAIN 69 tech_sharing_groups_live (persistent last-value-wins) --
    // TOP-LEVEL research-sharing groups. A proven frame FULL-REPLACES the carried
    // group set; a hard-failure frame retains the prior set. Wire payload: 16-byte
    // header (elapsed_wall_ms u64, group_count u32, status_flags u32), then
    // group_count fixed 12-byte rows (id_token u32, member_count u32,
    // member_blob_offset u32 in u32 units), then a single concatenated blob of
    // u32 tag_db_ids. Windows must be contiguous and cover the blob exactly. Any
    // bounds/torn failure returns Err (frame folds closed, prior set retained).
    fn fold_tech_sharing_groups(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            TECH_SHARING_GROUPS_HEADER_LEN,
            "tech_sharing_groups header",
        )?;
        let group_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 12);
        if self.handle_compact_failure(
            TECH_SHARING_GROUPS_DOMAIN_ID,
            "tech_sharing_groups",
            status_flags,
        ) {
            return Ok(());
        }
        if group_count > TECH_SHARING_GROUP_MAX_GROUPS {
            return Err(format!(
                "tech_sharing_groups group_count {group_count} exceeds {TECH_SHARING_GROUP_MAX_GROUPS} fixed slots"
            ));
        }
        let table_len = TECH_SHARING_GROUPS_HEADER_LEN
            + group_count
                .checked_mul(TECH_SHARING_GROUPS_ROW_LEN)
                .ok_or("tech_sharing_groups row table overflow")?;
        need(payload, table_len, "tech_sharing_groups row table")?;

        // The member blob is the payload remainder: a flat array of u32 tag_db_ids.
        let blob = &payload[table_len..];
        if blob.len() % 4 != 0 {
            return Err(format!(
                "tech_sharing_groups member blob {} bytes not u32-aligned",
                blob.len()
            ));
        }
        let member_total = blob.len() / 4;

        let mut groups: Vec<TechSharingGroupFoldState> = Vec::with_capacity(group_count);
        let mut expected_offset = 0usize;
        for i in 0..group_count {
            let base = TECH_SHARING_GROUPS_HEADER_LEN + i * TECH_SHARING_GROUPS_ROW_LEN;
            let id_token = read_u32(payload, base);
            let member_count = read_u32(payload, base + 4) as usize;
            let member_blob_offset = read_u32(payload, base + 8) as usize;
            if id_token == 0 {
                return Err(format!("tech_sharing_groups row {i} zero id_token"));
            }
            if member_count == 0 {
                return Err(format!("tech_sharing_groups row {i} zero member_count"));
            }
            if member_count > TECH_SHARING_GROUP_MAX_MEMBERS as usize {
                return Err(format!(
                    "tech_sharing_groups row {i} member_count {member_count} out of range"
                ));
            }
            if member_blob_offset != expected_offset {
                return Err(format!(
                    "tech_sharing_groups row {i} member_blob_offset {member_blob_offset} != expected {expected_offset} (non-contiguous)"
                ));
            }
            let end = member_blob_offset
                .checked_add(member_count)
                .ok_or("tech_sharing_groups member window overflow")?;
            if end > member_total {
                return Err(format!(
                    "tech_sharing_groups row {i} member window [{member_blob_offset},{end}) exceeds {member_total} members"
                ));
            }
            let mut member_tag_db_ids: Vec<u32> = (member_blob_offset..end)
                .map(|idx| read_u32(blob, idx * 4))
                .collect();
            for tag in &member_tag_db_ids {
                if *tag == 0 || *tag > TECH_SHARING_GROUP_MAX_MEMBER_TAG_DB_ID {
                    return Err(format!(
                        "tech_sharing_groups row {i} member tag_db_id {tag} out of range"
                    ));
                }
            }
            // Sort for a stable, order-independent multiset representation.
            member_tag_db_ids.sort_unstable();
            groups.push(TechSharingGroupFoldState {
                id_token,
                member_tag_db_ids,
            });
            expected_offset = end;
        }
        if expected_offset != member_total {
            return Err(format!(
                "tech_sharing_groups windows cover {expected_offset} of {member_total} members (orphans)"
            ));
        }

        // A proven frame with zero groups is a genuine empty set (distinct from
        // unknown). Full-replace the carried set.
        self.tech_sharing_groups = Some(groups);
        self.mark_fold_domain_success(TECH_SHARING_GROUPS_DOMAIN_ID);
        Ok(())
    }

    // --- W3 DOMAIN 65 special_project_programs_live (full replace) ------
    // Per-country OWNER-JOINED special-project program facilities, rooted at
    // CCountry+0xFA8 -> CProgramStatus -> CProgram. Each row:
    // {owner tag_db_id, program_id, province, unread_prototype_rewards}. The
    // collector fail-closed drops facilities whose CProgram pointer is reachable
    // from >1 country (aliased); `dropped_ambiguous` counts them and, when > 0,
    // PARTIAL_DATA is set (an expected-partial tolerance for d65). Full-replace
    // of the per-country `special_project_programs` array each frame.
    fn fold_special_project_programs(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            SPECIAL_PROJECT_PROGRAM_HEADER_LEN,
            "special_project_programs header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let dropped_ambiguous = read_u32(payload, 20);
        let expected =
            SPECIAL_PROJECT_PROGRAM_HEADER_LEN + row_count * SPECIAL_PROJECT_PROGRAM_ROW_LEN;
        if payload.len() != expected {
            return Err(format!(
                "special_project_programs len {} != {expected}",
                payload.len()
            ));
        }
        if self.handle_compact_failure(
            SPECIAL_PROJECT_PROGRAM_DOMAIN_ID,
            "special_project_programs",
            status_flags,
        ) {
            return Ok(());
        }
        if compact_expected_partial_tolerance_applies(
            SPECIAL_PROJECT_PROGRAM_DOMAIN_ID,
            status_flags,
        ) {
            // Dropped-ambiguous facilities make the owner-joined set incomplete
            // this frame. Without deletion evidence, replacing would fabricate
            // disappearance of the aliased facilities and merging would fabricate
            // persistence; keep the d65 leaf unknown for this hour.
            self.fold_domains_unknown_this_hour
                .insert(SPECIAL_PROJECT_PROGRAM_DOMAIN_ID);
            self.note(
                "special_project_programs state unknown this hour; ambiguous owner-joins dropped"
                    .to_string(),
            );
            return Ok(());
        }
        let mut per_owner: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
        let mut seen: BTreeSet<(u32, u32)> = BTreeSet::new();
        for i in 0..row_count {
            let base = SPECIAL_PROJECT_PROGRAM_HEADER_LEN + i * SPECIAL_PROJECT_PROGRAM_ROW_LEN;
            let owner_tag_db_id = read_u32(payload, base);
            let program_id = read_u32(payload, base + 4);
            let province = read_u32(payload, base + 8);
            let unread_prototype_rewards = read_u32(payload, base + 12);
            if owner_tag_db_id == 0 {
                return Err("special_project_programs row with zero owner tag_db_id".into());
            }
            if program_id == 0 {
                return Err("special_project_programs row with zero program_id".into());
            }
            // (owner, program_id) is the join key and must be unique (fail closed).
            if !seen.insert((owner_tag_db_id, program_id)) {
                return Err(format!(
                    "special_project_programs duplicate (owner {owner_tag_db_id}, program_id {program_id})"
                ));
            }
            per_owner.entry(owner_tag_db_id).or_default().push(json!({
                "program_id": program_id,
                "province": province,
                "unread_prototype_rewards": unread_prototype_rewards,
            }));
        }
        // dropped_ambiguous is carried as a per-frame diagnostic note (the count
        // of facilities that could not be safely owner-attributed this frame).
        if dropped_ambiguous > 0 {
            self.note(format!(
                "special_project_programs dropped {dropped_ambiguous} ambiguous (multiply-owned) facilities"
            ));
        }
        let known: Vec<String> = self.countries.keys().cloned().collect();
        for key in known {
            if let Ok(id) = key.parse::<u32>() {
                self.country_mut(id)
                    .insert("special_project_programs".into(), Value::Array(Vec::new()));
            }
        }
        for (owner, programs) in per_owner {
            self.country_mut(owner)
                .insert("special_project_programs".into(), Value::Array(programs));
        }
        Ok(())
    }

    // --- W4 DOMAIN 66 occupation_state_extras_live (full replace) --------
    // Per-occupied-state occupation extras, keyed by (controller, occupied,
    // state_id): occupation_law_token_id (FNV of the live/effective law token)
    // and garrison_manpower_source_tag_db_id (supplying-country tag). Both are
    // placed under the state's `occupation` object (same per-state home the
    // committed d52 lane uses), each present only when its collector presence
    // bit is set. Full-replace: every state's two d66 keys are cleared first so
    // a state that lost its occupation this frame does not keep a stale token
    // (a failed frame instead retains the prior map via handle_compact_failure).
    fn fold_occupation_state_extras(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            OCCUPATION_STATE_EXTRAS_HEADER_LEN,
            "occupation_state_extras header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let expected =
            OCCUPATION_STATE_EXTRAS_HEADER_LEN + row_count * OCCUPATION_STATE_EXTRAS_ROW_LEN;
        if payload.len() != expected {
            return Err(format!(
                "occupation_state_extras len {} != {expected}",
                payload.len()
            ));
        }
        if self.handle_compact_failure(
            OCCUPATION_STATE_EXTRAS_DOMAIN_ID,
            "occupation_state_extras",
            status_flags,
        ) {
            return Ok(());
        }
        // Decode + validate every row before mutating state (fail closed).
        struct Extra {
            state_id: u32,
            controller_tag_db_id: u32,
            occupied_tag_db_id: u32,
            occupation_law_token_id: Option<u32>,
            garrison_manpower_source_tag_db_id: Option<u32>,
        }
        let mut extras: Vec<Extra> = Vec::with_capacity(row_count);
        let mut seen: BTreeSet<(u32, u32, u32)> = BTreeSet::new();
        for i in 0..row_count {
            let base = OCCUPATION_STATE_EXTRAS_HEADER_LEN + i * OCCUPATION_STATE_EXTRAS_ROW_LEN;
            let controller_tag_db_id = read_u32(payload, base);
            let occupied_tag_db_id = read_u32(payload, base + 4);
            let state_id = read_u32(payload, base + 8);
            let flags = read_u32(payload, base + 12);
            let occupation_law_token_id = read_u32(payload, base + 16);
            let garrison_manpower_source_tag_db_id = read_u32(payload, base + 20);
            if controller_tag_db_id == 0 {
                return Err("occupation_state_extras row with zero controller tag".into());
            }
            if occupied_tag_db_id == 0 {
                return Err("occupation_state_extras row with zero occupied tag".into());
            }
            if state_id == 0 {
                return Err("occupation_state_extras row with zero state_id".into());
            }
            if flags & !OCCUPATION_STATE_EXTRAS_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "occupation_state_extras row {i} unknown flags {flags:#x}"
                ));
            }
            let has_law = flags & OCCUPATION_STATE_EXTRAS_FLAG_LAW != 0;
            let has_source = flags & OCCUPATION_STATE_EXTRAS_FLAG_GARRISON_SOURCE != 0;
            // An absent field must pack a zero (fail closed on a malformed row).
            if !has_law && occupation_law_token_id != 0 {
                return Err(format!(
                    "occupation_state_extras row {i} law token without flag"
                ));
            }
            if has_law && occupation_law_token_id == 0 {
                return Err(format!(
                    "occupation_state_extras row {i} law flag with zero token"
                ));
            }
            if !has_source && garrison_manpower_source_tag_db_id != 0 {
                return Err(format!(
                    "occupation_state_extras row {i} garrison source without flag"
                ));
            }
            if has_source && garrison_manpower_source_tag_db_id == 0 {
                return Err(format!(
                    "occupation_state_extras row {i} garrison flag with zero tag"
                ));
            }
            // A row must carry at least one datum (the key alone is not data).
            if !has_law && !has_source {
                return Err(format!("occupation_state_extras row {i} carries no field"));
            }
            // (controller, occupied, state) is the join key and must be unique.
            if !seen.insert((controller_tag_db_id, occupied_tag_db_id, state_id)) {
                return Err(format!(
                    "occupation_state_extras duplicate (controller {controller_tag_db_id}, occupied {occupied_tag_db_id}, state {state_id})"
                ));
            }
            extras.push(Extra {
                state_id,
                controller_tag_db_id,
                occupied_tag_db_id,
                occupation_law_token_id: has_law.then_some(occupation_law_token_id),
                garrison_manpower_source_tag_db_id: has_source
                    .then_some(garrison_manpower_source_tag_db_id),
            });
        }
        // Full-replace: clear the two d66 keys from every state's occupation
        // object, then set the measured ones.
        for state in self.states.values_mut() {
            if let Some(Value::Object(occupation)) = state.get_mut("occupation") {
                occupation.remove("occupation_law_token_id");
                occupation.remove("occupation_law_token_encoding");
                occupation.remove("occupation_controller_tag_db_id");
                occupation.remove("garrison_manpower_source_tag_db_id");
            }
        }
        for extra in extras {
            let state = self.states.entry(extra.state_id.to_string()).or_default();
            let occupation = match state
                .entry("occupation".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
            {
                Value::Object(map) => map,
                other => {
                    *other = Value::Object(Map::new());
                    match other {
                        Value::Object(map) => map,
                        _ => unreachable!(),
                    }
                }
            };
            occupation.insert(
                "occupation_controller_tag_db_id".into(),
                json!(extra.controller_tag_db_id),
            );
            // occupied_tag is carried for parity with the melt grouping; kept as
            // a diagnostic under the same object.
            occupation.insert("occupied_tag_db_id".into(), json!(extra.occupied_tag_db_id));
            if let Some(token) = extra.occupation_law_token_id {
                occupation.insert("occupation_law_token_id".into(), json!(token));
                occupation.insert(
                    "occupation_law_token_encoding".into(),
                    json!("fnv1a32_of_law_token"),
                );
            }
            if let Some(source_tag) = extra.garrison_manpower_source_tag_db_id {
                occupation.insert(
                    "garrison_manpower_source_tag_db_id".into(),
                    json!(source_tag),
                );
            }
        }
        Ok(())
    }

    // --- DOMAIN 11 production_live (full each frame) -----------------------
    fn fold_production(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 40, "production header")?;
        let country_count = read_u32(payload, 8) as usize;
        let stockpile_count = read_u32(payload, 24) as usize;
        let military_count = read_u32(payload, 28) as usize;
        let naval_count = read_u32(payload, 32) as usize;
        let construction_count = read_u32(payload, 36) as usize;
        let line_count = military_count + naval_count;
        let base_len = 40 + stockpile_count * 16 + line_count * 32 + construction_count * 32;
        // Wave-4 production licensing: base payload is self-delimiting to
        // base_len; any trailing bytes are the `PLIC` license trailer.
        if payload.len() < base_len {
            return Err(format!(
                "production len {} < base {base_len}",
                payload.len()
            ));
        }
        let _ = country_count;

        // clear any prior production block for every country we touch this frame
        // by building fresh per-tag accumulators; full-replace semantics.
        let mut per_tag: BTreeMap<u32, ProductionAccum> = BTreeMap::new();

        let mut cursor = 40usize;
        for _ in 0..stockpile_count {
            let tag = read_u32(payload, cursor);
            let variant_id = read_u32(payload, cursor + 4);
            let amount = read_i64(payload, cursor + 8) as f64 / PRODUCTION_SCALE;
            per_tag.entry(tag).or_default().stockpile.push(json!({
                "id": variant_id.to_string(),
                "amount": amount,
            }));
            cursor += 16;
        }
        for _ in 0..line_count {
            let tag = read_u32(payload, cursor);
            let kind = read_u32(payload, cursor + 4);
            let line_id = read_i32(payload, cursor + 8);
            let active_factories = read_i32(payload, cursor + 12);
            let cost = read_i32(payload, cursor + 16) as f64 / PRODUCTION_SCALE;
            let speed = read_i32(payload, cursor + 20) as f64 / PRODUCTION_SCALE;
            let produced = read_i32(payload, cursor + 24) as f64 / PRODUCTION_SCALE;
            let amount = read_i32(payload, cursor + 28);
            let row = json!({
                "id": line_id,
                "active_factories": active_factories,
                "cost": cost,
                "speed": speed,
                "produced": produced,
                "amount": amount,
            });
            let accum = per_tag.entry(tag).or_default();
            if kind == 2 {
                accum.naval_lines.push(row);
            } else {
                accum.military_lines.push(row);
            }
            cursor += 32;
        }
        for _ in 0..construction_count {
            let tag = read_u32(payload, cursor);
            let line_id = read_u32(payload, cursor + 4);
            let template_id = read_u32(payload, cursor + 8);
            let active_factories = read_i32(payload, cursor + 12);
            let cost = read_i32(payload, cursor + 16) as f64 / PRODUCTION_SCALE;
            let produced = read_i32(payload, cursor + 20) as f64 / PRODUCTION_SCALE;
            let priority = read_u32(payload, cursor + 24);
            let mut row = Map::new();
            row.insert("id".into(), json!(line_id.to_string()));
            if template_id != u32::MAX {
                row.insert("template_id".into(), json!(template_id));
            }
            row.insert("active_factories".into(), json!(active_factories));
            row.insert("cost".into(), json!(cost));
            row.insert("produced".into(), json!(produced));
            row.insert("priority".into(), json!(priority));
            per_tag
                .entry(tag)
                .or_default()
                .construction_queue
                .push(Value::Object(row));
            cursor += 32;
        }

        // Wave-4 production licensing: parse the optional `PLIC` trailer. A
        // country present in the trailer gets its measured license set
        // (proven-empty => `[]`); a country absent from the trailer keeps
        // `licenses = None` (unreadable/not measured, folded as null — absent
        // != empty).
        debug_assert_eq!(cursor, base_len);
        if cursor != payload.len() {
            let licenses_by_tag = parse_production_license_trailer(payload, cursor)?;
            for (tag, ids) in licenses_by_tag {
                per_tag.entry(tag).or_default().licenses = Some(ids);
            }
        }

        for (tag, accum) in per_tag {
            let country = self.country_mut(tag);
            let licenses_value = match accum.licenses {
                Some(ids) => {
                    Value::Array(ids.into_iter().map(|id| json!(id.to_string())).collect())
                }
                None => Value::Null,
            };
            country.insert(
                "production".into(),
                json!({
                    "stockpile": accum.stockpile,
                    "military_lines": accum.military_lines,
                    "naval_lines": accum.naval_lines,
                    "construction_queue": accum.construction_queue,
                    "production_licenses": licenses_value,
                }),
            );
        }
        Ok(())
    }

    // --- DOMAIN 12 focus_live (full each frame) ---------------------------
    fn fold_focus(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 28, "focus header")?;
        let country_count = read_u32(payload, 8) as usize;
        let completed_token_count = read_u32(payload, 24) as usize;
        let mut cursor = 28usize;
        // collect completed tokens per tag first pass is not possible (rows come
        // after country rows), so read countries then completed tokens.
        let mut per_tag: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
        for _ in 0..country_count {
            let tag = read_u32(payload, cursor);
            let flags = read_u32(payload, cursor + 4);
            let completed_count = read_u32(payload, cursor + 8);
            let progress_raw = read_i32(payload, cursor + 12);
            cursor += 16;
            let focus_tree = read_string(payload, &mut cursor, "focus tree")?;
            let current = read_string(payload, &mut cursor, "focus current")?;
            let current_continuous = read_string(payload, &mut cursor, "focus current continuous")?;
            let mut obj = Map::new();
            obj.insert("focus_tree".into(), string_or_null(&focus_tree));
            obj.insert(
                "completed_count".into(),
                if completed_count == u32::MAX {
                    Value::Null
                } else {
                    json!(completed_count)
                },
            );
            obj.insert("current".into(), string_or_null(&current));
            obj.insert(
                "current_continuous".into(),
                string_or_null(&current_continuous),
            );
            // progress present only when flag bit 4 is set
            if flags & (1 << 4) != 0 {
                obj.insert("progress".into(), json!(progress_raw as f64 / FIXED_SCALE));
            } else {
                obj.insert("progress".into(), Value::Null);
            }
            obj.insert("completed".into(), json!([]));
            per_tag.insert(tag, obj);
        }
        for _ in 0..completed_token_count {
            let tag = read_u32(payload, cursor);
            cursor += 4;
            let token = read_string(payload, &mut cursor, "focus completed token")?;
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Array(list)) = obj.get_mut("completed") {
                    list.push(json!(token));
                }
            }
        }
        for (tag, obj) in per_tag {
            self.country_mut(tag)
                .insert("focus".into(), Value::Object(obj));
        }
        Ok(())
    }

    // --- DOMAIN 13 tech_live (full each frame) ----------------------------
    fn fold_tech(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 32, "tech header")?;
        let country_count = read_u32(payload, 8) as usize;
        let active_count = read_u32(payload, 28) as usize;
        let expected = 32 + country_count * 16 + active_count * 40;
        if payload.len() != expected {
            return Err(format!("tech len {} != {expected}", payload.len()));
        }
        let mut cursor = 32usize;
        let mut per_tag: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
        for _ in 0..country_count {
            let tag = read_u32(payload, cursor);
            let completed = read_u32(payload, cursor + 8);
            let slots = read_u32(payload, cursor + 12);
            cursor += 16;
            let mut obj = Map::new();
            obj.insert("tech_count".into(), json!(completed));
            obj.insert("research_slots".into(), json!(slots));
            obj.insert("in_progress_live".into(), json!([]));
            per_tag.insert(tag, obj);
        }
        for _ in 0..active_count {
            let tag = read_u32(payload, cursor);
            let slot_index = read_u32(payload, cursor + 4);
            let tech_handle = read_u32(payload, cursor + 8);
            let tech_def_index = read_u32(payload, cursor + 12);
            let progress = read_i32(payload, cursor + 16) as f64 / TECH_PROGRESS_SCALE;
            let live_progress = read_i64(payload, cursor + 24) as f64 / TECH_LIVE_SCALE;
            let base_required = read_i64(payload, cursor + 32) as f64 / TECH_LIVE_SCALE;
            cursor += 40;
            let slot = json!({
                "slot_index": slot_index,
                "tech_handle": tech_handle,
                "tech_def_index": tech_def_index,
                "progress": progress,
                "live_progress": live_progress,
                "base_required_points": base_required,
            });
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Array(list)) = obj.get_mut("in_progress_live") {
                    list.push(slot);
                }
            }
        }
        for (tag, obj) in per_tag {
            self.country_mut(tag)
                .insert("research".into(), Value::Object(obj));
        }
        Ok(())
    }

    // --- DOMAIN 14 politics_live (full each frame) ------------------------
    fn fold_politics(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 40, "politics header")?;
        let country_count = read_u32(payload, 8) as usize;
        let popularity_count = read_u32(payload, 24) as usize;
        let law_count = read_u32(payload, 28) as usize;
        let advisor_count = read_u32(payload, 32) as usize;
        let spirit_count = read_u32(payload, 36) as usize;
        let mut cursor = 40usize;
        let mut per_tag: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
        for _ in 0..country_count {
            need(payload, cursor + 28, "politics country row")?;
            let tag = read_u32(payload, cursor);
            cursor += 28;
            let ruling_party = read_string(payload, &mut cursor, "ruling_party")?;
            let leader_name = read_string(payload, &mut cursor, "leader_name")?;
            let leader_token = read_string(payload, &mut cursor, "leader_token")?;
            let leader_ideology = read_string(payload, &mut cursor, "leader_ideology")?;
            let mut obj = Map::new();
            obj.insert("ruling_party".into(), string_or_null(&ruling_party));
            obj.insert("leader_name".into(), string_or_null(&leader_name));
            obj.insert("leader_token".into(), string_or_null(&leader_token));
            obj.insert("leader_ideology".into(), string_or_null(&leader_ideology));
            obj.insert("party_popularities".into(), json!({}));
            obj.insert("laws".into(), json!({}));
            obj.insert("appointed_advisors".into(), json!([]));
            obj.insert("national_spirits".into(), json!([]));
            per_tag.insert(tag, obj);
        }
        for _ in 0..popularity_count {
            let tag = read_u32(payload, cursor);
            cursor += 4;
            let ideology = read_string(payload, &mut cursor, "popularity ideology")?;
            let value = read_i32(payload, cursor) as f64 / FIXED_SCALE;
            cursor += 4;
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Object(pop)) = obj.get_mut("party_popularities") {
                    pop.insert(ideology, json!(value));
                }
            }
        }
        for _ in 0..law_count {
            let tag = read_u32(payload, cursor);
            cursor += 4;
            let group = read_string(payload, &mut cursor, "law group")?;
            let token = read_string(payload, &mut cursor, "law token")?;
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Object(laws)) = obj.get_mut("laws") {
                    laws.insert(group, json!(token));
                }
            }
        }
        for _ in 0..advisor_count {
            let tag = read_u32(payload, cursor);
            cursor += 4;
            let _group = read_string(payload, &mut cursor, "advisor group")?;
            let token = read_string(payload, &mut cursor, "advisor token")?;
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Array(list)) = obj.get_mut("appointed_advisors") {
                    list.push(json!(token));
                }
            }
        }
        for _ in 0..spirit_count {
            let tag = read_u32(payload, cursor);
            cursor += 4;
            let token = read_string(payload, &mut cursor, "spirit token")?;
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Array(list)) = obj.get_mut("national_spirits") {
                    list.push(json!(token));
                }
            }
        }
        for (tag, obj) in per_tag {
            merge_into(self.country_mut(tag), "politics", obj);
        }
        Ok(())
    }

    // --- DOMAIN 15 politics_residuals (full each frame) -------------------
    fn fold_politics_residuals(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 24, "politics residuals header")?;
        let country_count = read_u32(payload, 8) as usize;
        let advisor_detail_count = read_u32(payload, 20) as usize;
        let mut cursor = 24usize;
        let mut per_tag: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
        for _ in 0..country_count {
            need(payload, cursor + 16, "politics residuals country row")?;
            let tag = read_u32(payload, cursor);
            let _flags = read_u32(payload, cursor + 4);
            let elections_allowed = read_u32(payload, cursor + 8);
            cursor += 16;
            let last_election = read_string(payload, &mut cursor, "last_election")?;
            let mut obj = Map::new();
            obj.insert("last_election".into(), string_or_null(&last_election));
            obj.insert("elections_allowed".into(), json!(elections_allowed != 0));
            obj.insert("appointed_advisor_details".into(), json!([]));
            per_tag.insert(tag, obj);
        }
        for _ in 0..advisor_detail_count {
            let tag = read_u32(payload, cursor);
            cursor += 4;
            let idea_token = read_string(payload, &mut cursor, "advisor idea_token")?;
            let slot = read_string(payload, &mut cursor, "advisor slot")?;
            let ledger = read_string(payload, &mut cursor, "advisor ledger")?;
            let character = read_string(payload, &mut cursor, "advisor character")?;
            let detail = json!({
                "idea_token": idea_token,
                "slot": slot,
                "ledger": ledger,
                "character": character,
            });
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Array(list)) = obj.get_mut("appointed_advisor_details") {
                    list.push(detail);
                }
            }
        }
        for (tag, obj) in per_tag {
            merge_into(self.country_mut(tag), "politics", obj);
        }
        Ok(())
    }

    // --- DOMAIN 16 decisions_live (full each frame) -----------------------
    fn fold_decisions(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 24, "decisions header")?;
        let country_count = read_u32(payload, 8) as usize;
        let timed_count = read_u32(payload, 20) as usize;
        let mut cursor = 24usize;
        let mut per_tag: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
        for _ in 0..country_count {
            need(payload, cursor + 32, "decisions country row")?;
            let tag = read_u32(payload, cursor);
            let _flags = read_u32(payload, cursor + 4);
            let active_timed = read_u32(payload, cursor + 8);
            let active_targeted = read_u32(payload, cursor + 12);
            let active_targeted_timed = read_u32(payload, cursor + 16);
            cursor += 32;
            let mut obj = Map::new();
            obj.insert(
                "status_present".into(),
                json!({
                    "active_timed": active_timed,
                    "active_targeted": active_targeted,
                    "active_targeted_timed": active_targeted_timed,
                }),
            );
            obj.insert("timed_live".into(), json!([]));
            per_tag.insert(tag, obj);
        }
        for _ in 0..timed_count {
            need(payload, cursor + 12, "decisions timed row")?;
            let tag = read_u32(payload, cursor);
            let days = read_i32(payload, cursor + 4);
            let state = read_i32(payload, cursor + 8);
            cursor += 12;
            let decision = read_string(payload, &mut cursor, "timed decision_id")?;
            let row = json!({ "decision": decision, "days": days, "state": state });
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Array(list)) = obj.get_mut("timed_live") {
                    list.push(row);
                }
            }
        }
        for (tag, obj) in per_tag {
            self.country_mut(tag)
                .insert("decisions".into(), Value::Object(obj));
        }
        Ok(())
    }

    // --- DOMAIN 17 diplomacy_live (full each frame) -----------------------
    fn fold_diplomacy(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 24, "diplomacy header")?;
        // G1-residue gate finding 3 (2026-07-12): this fold previously never
        // read its status word, so a d17 record carrying real failure bits
        // (NO_SCALAR_SOURCE / WORKER_PANIC / PARTIAL_DATA) folded its rows as
        // authoritative — incomplete diplomacy silently replaced good state.
        // Gate on the failure mask; the informational WAR_SEGMENT_ABSENT bit
        // (1<<13, O7) is NOT in the mask and folds normally on its own.
        let status_flags = read_u32(payload, 16);
        if compact_status_is_failure(status_flags) {
            self.note(format!(
                "diplomacy failure status {:#x}; diplomacy state unchanged this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        let country_count = read_u32(payload, 8) as usize;
        let relation_count = read_u32(payload, 20) as usize;
        let mut cursor = 24usize;
        let mut per_tag: BTreeMap<u32, Map<String, Value>> = BTreeMap::new();
        for _ in 0..country_count {
            need(payload, cursor + 24, "diplomacy country row")?;
            let tag = read_u32(payload, cursor);
            let _flags = read_u32(payload, cursor + 4);
            let faction_group = read_i32(payload, cursor + 8);
            let at_war_count = read_u32(payload, cursor + 12);
            let casualties = read_i64(payload, cursor + 16);
            cursor += 24;
            let mut obj = Map::new();
            obj.insert("faction_group".into(), json!(faction_group));
            obj.insert("at_war_count".into(), json!(at_war_count));
            obj.insert("casualties".into(), json!(casualties));
            obj.insert("at_war_with".into(), json!([]));
            per_tag.insert(tag, obj);
        }
        for _ in 0..relation_count {
            need(payload, cursor + 16, "diplomacy at-war row")?;
            let tag = read_u32(payload, cursor);
            let token_len = read_u32(payload, cursor + 4) as usize;
            let enemy = ascii8(&payload[cursor + 8..cursor + 16], token_len);
            cursor += 16;
            if let Some(obj) = per_tag.get_mut(&tag) {
                if let Some(Value::Array(list)) = obj.get_mut("at_war_with") {
                    list.push(json!(enemy));
                }
            }
        }
        for (tag, obj) in per_tag {
            self.country_mut(tag)
                .insert("diplomacy".into(), Value::Object(obj));
        }
        Ok(())
    }

    // --- DOMAIN 18 country_identity (full each frame) ----------------------
    // Header: wall(u64) row_count(u32) changed(u32) status(u32).
    // Rows: tag_db_id(u32) then a u16-length-prefixed UTF-8 TAG string.
    fn fold_country_identity(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 20, "country_identity header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        if compact_status_is_failure(status_flags) {
            self.note(format!(
                "country_identity failure status {:#x}; tag_db_id map unchanged this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        let mut cursor = 20usize;
        let mut fresh: BTreeMap<u32, String> = BTreeMap::new();
        let mut fresh_cosmetic: BTreeMap<u32, Option<String>> = BTreeMap::new();
        for _ in 0..row_count {
            need(payload, cursor + 4, "country_identity row tag_db_id")?;
            let tag_db_id = read_u32(payload, cursor);
            cursor += 4;
            let tag = read_string(payload, &mut cursor, "country_identity tag")?;
            if tag_db_id == 0 {
                return Err("country_identity row with zero tag_db_id".into());
            }
            if tag.is_empty() {
                return Err(format!("country_identity row {tag_db_id} with empty tag"));
            }
            fresh.insert(tag_db_id, tag);
            // Wave-4 cosmetic tag: 1-byte marker then the string only when
            // present. absent(0) => not in the map (folds absent); null(1) =>
            // map[id] = None (folds null); present(2) => map[id] = Some(tag).
            need(payload, cursor + 1, "country_identity cosmetic marker")?;
            let marker = payload[cursor];
            cursor += 1;
            match marker {
                0 => {}
                1 => {
                    fresh_cosmetic.insert(tag_db_id, None);
                }
                2 => {
                    let cosmetic = read_string(payload, &mut cursor, "country_identity cosmetic")?;
                    if cosmetic.is_empty() {
                        return Err(format!(
                            "country_identity row {tag_db_id} present cosmetic marker with empty string"
                        ));
                    }
                    fresh_cosmetic.insert(tag_db_id, Some(cosmetic));
                }
                other => {
                    return Err(format!(
                        "country_identity row {tag_db_id} invalid cosmetic marker {other}"
                    ));
                }
            }
        }
        if cursor != payload.len() {
            return Err(format!(
                "country_identity len {} != consumed {cursor}",
                payload.len()
            ));
        }
        // full-replace: the domain re-sends the complete live-country map each frame.
        self.tag_by_db_id = fresh;
        self.cosmetic_by_db_id = fresh_cosmetic;
        // E13: a proven domain-18 record measured the country set THIS hour, so
        // `tag_by_db_id` is now the freshly-measured roster (not carried over).
        self.country_identity_known_this_hour = true;
        Ok(())
    }

    // --- DOMAIN 27 army_orders (full baseline + explicit delta) -----------
    fn fold_army_orders(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, ARMY_ORDER_HEADER_LEN, "army_orders header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let target_state_count = read_u32(payload, 32) as usize;
        if self.handle_compact_failure(ARMY_ORDER_DOMAIN_ID, "army_orders", status_flags) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(ARMY_ORDER_DOMAIN_ID, "army_orders", status_flags) {
            return Ok(());
        }
        let rows_end =
            ARMY_ORDER_HEADER_LEN.saturating_add(row_count.saturating_mul(ARMY_ORDER_ROW_LEN));
        let expected_len =
            rows_end.saturating_add(target_state_count.saturating_mul(ARMY_ORDER_TARGET_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "army_orders len {} != {expected_len}",
                payload.len()
            ));
        }
        let mut rows = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let base = ARMY_ORDER_HEADER_LEN + index * ARMY_ORDER_ROW_LEN;
            let flags = read_u32(payload, base + 12);
            if flags & !ARMY_ORDER_KNOWN_FLAGS != 0 {
                return Err("army_orders row has unknown flags".into());
            }
            let target_first = read_u32(payload, base + 24) as usize;
            let target_count = read_u32(payload, base + 28) as usize;
            if flags & ARMY_ORDER_ROW_FLAG_TARGET_STATES == 0
                && (target_first != 0 || target_count != 0)
            {
                return Err("army_orders target range present without flag".into());
            }
            let target_end = target_first
                .checked_add(target_count)
                .ok_or_else(|| "army_orders target range overflow".to_string())?;
            if target_end > target_state_count {
                return Err(format!(
                    "army_orders target range {target_first}+{target_count} exceeds {target_state_count}"
                ));
            }
            let reserved0 = read_u32(payload, base + 40);
            let reserved1 = read_u32(payload, base + 44);
            if reserved0 != 0 || reserved1 != 0 {
                return Err("army_orders row reserved field is nonzero".into());
            }
            rows.push(ArmyOrderRawRow {
                army_id: read_u32(payload, base),
                generation: read_u64(payload, base + 4),
                flags,
                slot_index: read_u32(payload, base + 16),
                order_type: read_i32(payload, base + 20),
                target_first,
                target_count,
                target_state_declared_count: read_u32(payload, base + 32),
                target_status_code: read_u32(payload, base + 36),
            });
        }
        let mut targets = Vec::with_capacity(target_state_count);
        for index in 0..target_state_count {
            let base = rows_end + index * ARMY_ORDER_TARGET_ROW_LEN;
            let state_id = read_u32(payload, base);
            let reserved = read_u32(payload, base + 4);
            if reserved != 0 {
                return Err("army_orders target reserved field is nonzero".into());
            }
            targets.push(state_id);
        }
        let sparse = self.replace_or_sparse_after_first(ARMY_ORDER_DOMAIN_ID, status_flags);
        if !sparse {
            self.army_orders.clear();
        }
        for row in rows {
            let key = ArmyOrderKey {
                army_id: row.army_id,
                generation: row.generation,
            };
            if row.flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.army_orders.remove(&key);
                continue;
            }
            let target_end = row.target_first + row.target_count;
            self.army_orders.insert(
                key,
                ArmyOrderState {
                    army_id: row.army_id,
                    generation: row.generation,
                    slot_index: row.slot_index,
                    order_type: row.order_type,
                    target_states: targets[row.target_first..target_end].to_vec(),
                    target_state_declared_count: row.target_state_declared_count,
                    target_status_code: row.target_status_code,
                },
            );
        }
        self.mark_fold_domain_success(ARMY_ORDER_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 28 oob_command (full baseline + explicit delta) -----------
    fn fold_oob_command(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, OOB_COMMAND_HEADER_LEN, "oob_command header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let member_count = read_u32(payload, 32) as usize;
        if self.handle_compact_failure(OOB_COMMAND_DOMAIN_ID, "oob_command", status_flags) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(OOB_COMMAND_DOMAIN_ID, "oob_command", status_flags) {
            return Ok(());
        }
        let rows_end =
            OOB_COMMAND_HEADER_LEN.saturating_add(row_count.saturating_mul(OOB_COMMAND_ROW_LEN));
        let expected_len =
            rows_end.saturating_add(member_count.saturating_mul(OOB_COMMAND_MEMBER_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "oob_command len {} != {expected_len}",
                payload.len()
            ));
        }
        let mut rows = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let base = OOB_COMMAND_HEADER_LEN + index * OOB_COMMAND_ROW_LEN;
            let kind = read_u32(payload, base);
            if !matches!(
                kind,
                OOB_KIND_ARMY | OOB_KIND_ARMY_GROUP | OOB_KIND_FLEET | OOB_KIND_TASK_FORCE
            ) {
                return Err("oob_command row has invalid kind".into());
            }
            let object_id = read_u32(payload, base + 4);
            if object_id == 0 {
                return Err("oob_command row has zero object_id".into());
            }
            let flags = read_u32(payload, base + 8);
            if flags & !OOB_KNOWN_FLAGS != 0 {
                return Err("oob_command row has unknown flags".into());
            }
            let member_first = read_u32(payload, base + 28) as usize;
            let row_member_count = read_u32(payload, base + 32) as usize;
            let member_end = member_first
                .checked_add(row_member_count)
                .ok_or_else(|| "oob_command member range overflow".to_string())?;
            if member_end > member_count {
                return Err(format!(
                    "oob_command member range {member_first}+{row_member_count} exceeds {member_count}"
                ));
            }
            let leader_skill = read_u32(payload, base + 24);
            if flags & OOB_ROW_FLAG_LEADER_SKILL == 0 && leader_skill != 0 {
                return Err("oob_command leader_skill present without flag".into());
            }
            let location = read_u32(payload, base + 72);
            if flags & OOB_ROW_FLAG_LOCATION == 0 && location != 0 {
                return Err("oob_command location present without flag".into());
            }
            let reserved = read_u32(payload, base + 76);
            if reserved != 0 {
                return Err("oob_command row reserved field is nonzero".into());
            }
            rows.push(OobCommandRawRow {
                kind,
                object_id,
                flags,
                parent_id: read_u32(payload, base + 12),
                owner_tag_db_id: read_u32(payload, base + 16),
                leader_id: read_u32(payload, base + 20),
                leader_skill,
                member_first,
                row_member_count,
                logical_member_count: read_u32(payload, base + 36),
                member_count_engine: read_u32(payload, base + 40),
                missing_member_count: read_u32(payload, base + 44),
                leader_trait_count: read_u32(payload, base + 48),
                parent_status: read_u32(payload, base + 52),
                member_status: read_u32(payload, base + 56),
                owner_status: read_u32(payload, base + 60),
                leader_status: read_u32(payload, base + 64),
                leader_trait_status: read_u32(payload, base + 68),
                location_province_id: location,
            });
        }
        let mut members = Vec::with_capacity(member_count);
        for index in 0..member_count {
            let base = rows_end + index * OOB_COMMAND_MEMBER_ROW_LEN;
            let member_id = read_u32(payload, base);
            let reserved = read_u32(payload, base + 4);
            if reserved != 0 {
                return Err("oob_command member reserved field is nonzero".into());
            }
            members.push(member_id);
        }
        let sparse = self.replace_or_sparse_after_first(OOB_COMMAND_DOMAIN_ID, status_flags);
        if !sparse {
            self.oob_commands.clear();
        }
        for row in rows {
            let key = OobCommandKey {
                kind: row.kind,
                object_id: row.object_id,
            };
            if row.flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.oob_commands.remove(&key);
                continue;
            }
            let member_end = row.member_first + row.row_member_count;
            self.oob_commands.insert(
                key,
                OobCommandState {
                    kind: row.kind,
                    object_id: row.object_id,
                    parent_id: (row.flags & OOB_ROW_FLAG_PARENT != 0).then_some(row.parent_id),
                    owner_tag_db_id: if row.flags & OOB_ROW_FLAG_OWNER != 0 {
                        row.owner_tag_db_id
                    } else {
                        0
                    },
                    leader_id: (row.flags & OOB_ROW_FLAG_LEADER != 0).then_some(row.leader_id),
                    leader_skill: (row.flags & OOB_ROW_FLAG_LEADER_SKILL != 0)
                        .then_some(row.leader_skill),
                    members: members[row.member_first..member_end].to_vec(),
                    logical_member_count: row.logical_member_count,
                    member_count_engine: (row.flags & OOB_ROW_FLAG_MEMBER_COUNT_ENGINE != 0)
                        .then_some(row.member_count_engine),
                    missing_member_count: (row.flags & OOB_ROW_FLAG_MISSING_MEMBER_COUNT != 0)
                        .then_some(row.missing_member_count),
                    leader_trait_count: row.leader_trait_count,
                    parent_status: row.parent_status,
                    member_status: row.member_status,
                    owner_status: row.owner_status,
                    leader_status: row.leader_status,
                    leader_trait_status: row.leader_trait_status,
                    location_province_id: (row.flags & OOB_ROW_FLAG_LOCATION != 0)
                        .then_some(row.location_province_id),
                },
            );
        }
        self.mark_fold_domain_success(OOB_COMMAND_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 29 ships (collector sparse after first, flag not set) -----
    fn fold_ships(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, SHIP_HEADER_LEN, "ships header")?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let observed_ship_rows = read_u32(payload, 20) as usize;
        let deleted_rows = read_u32(payload, 28) as usize;
        let scale_raw = read_u32(payload, 32);
        let reserved0 = read_u32(payload, 36);
        ensure_count_at_most(
            "ships",
            "changed_rows",
            changed_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "ships",
            "deleted_rows",
            deleted_rows,
            "row_count",
            row_count,
        )?;
        ensure_row_count_with_deletes(
            "ships",
            row_count,
            "observed_ship_rows",
            observed_ship_rows,
            deleted_rows,
        )?;
        ensure_fixed_scale("ships", "scale_raw", scale_raw)?;
        if reserved0 != 0 {
            return Err("ships header reserved field is nonzero".into());
        }
        if self.handle_compact_failure(SHIPS_DOMAIN_ID, "ships", status_flags) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(SHIPS_DOMAIN_ID, "ships", status_flags) {
            return Ok(());
        }
        let mut rows = Vec::with_capacity(row_count);
        let mut cursor = SHIP_HEADER_LEN;
        for _ in 0..row_count {
            need(payload, cursor + SHIP_FIXED_ROW_LEN, "ships fixed row")?;
            let flags = read_u32(payload, cursor + 4);
            if flags & !SHIP_KNOWN_FLAGS != 0 {
                return Err("ships row has unknown flags".into());
            }
            let reserved = read_u32(payload, cursor + 36);
            if reserved != 0 {
                return Err("ships row reserved field is nonzero".into());
            }
            let row = ShipRawRow {
                instance_id: read_u32(payload, cursor),
                flags,
                task_force_id: read_u32(payload, cursor + 8),
                category_enum: read_u32(payload, cursor + 12),
                strength_raw: read_u32(payload, cursor + 16),
                organisation_raw: read_u32(payload, cursor + 20),
                experience_raw: read_u32(payload, cursor + 24),
                sunk_convoys: read_u32(payload, cursor + 28),
                owner_tag_db_id: read_u32(payload, cursor + 32),
            };
            cursor += SHIP_FIXED_ROW_LEN;
            let value = read_string(payload, &mut cursor, "ships definition")?;
            if value.is_empty() && flags & SHIP_ROW_FLAG_DEFINITION != 0 {
                return Err("ships definition flag with empty string".into());
            }
            if !value.is_empty() && flags & SHIP_ROW_FLAG_DEFINITION == 0 {
                return Err("ships definition string without flag".into());
            }
            rows.push((row, value));
        }
        if cursor != payload.len() {
            return Err("ships payload has trailing bytes".into());
        }
        let sparse = self.replace_or_sparse_after_first(SHIPS_DOMAIN_ID, status_flags);
        if !sparse {
            self.ships.clear();
        }
        for (row, definition) in rows {
            if row.flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.ships.remove(&row.instance_id);
                continue;
            }
            self.ships.insert(
                row.instance_id,
                ShipState {
                    instance_id: row.instance_id,
                    task_force_id: (row.flags & SHIP_ROW_FLAG_TASK_FORCE != 0)
                        .then_some(row.task_force_id),
                    category_enum: row.category_enum,
                    strength_raw: row.strength_raw,
                    organisation_raw: row.organisation_raw,
                    experience_raw: row.experience_raw,
                    sunk_convoys: row.sunk_convoys,
                    owner_tag_db_id: if row.flags & SHIP_ROW_FLAG_OWNER != 0 {
                        row.owner_tag_db_id
                    } else {
                        0
                    },
                    definition: if row.flags & SHIP_ROW_FLAG_DEFINITION != 0 {
                        Some(definition)
                    } else {
                        None
                    },
                },
            );
        }
        self.mark_fold_domain_success(SHIPS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 30 air_wings (collector sparse after first, flag not set) --
    fn fold_air_wings(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, AIR_WING_HEADER_LEN, "air_wings header")?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let observed_air_wing_rows = read_u32(payload, 20) as usize;
        let deleted_rows = read_u32(payload, 24) as usize;
        let scale_raw = read_u32(payload, 28);
        let reserved0 = read_u32(payload, 32);
        ensure_count_at_most(
            "air_wings",
            "changed_rows",
            changed_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "air_wings",
            "deleted_rows",
            deleted_rows,
            "row_count",
            row_count,
        )?;
        ensure_row_count_with_deletes(
            "air_wings",
            row_count,
            "observed_air_wing_rows",
            observed_air_wing_rows,
            deleted_rows,
        )?;
        ensure_fixed_scale("air_wings", "scale_raw", scale_raw)?;
        if reserved0 != 0 {
            return Err("air_wings header reserved field is nonzero".into());
        }
        if self.handle_compact_failure(AIR_WINGS_DOMAIN_ID, "air_wings", status_flags) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(AIR_WINGS_DOMAIN_ID, "air_wings", status_flags) {
            return Ok(());
        }
        let expected_len =
            AIR_WING_HEADER_LEN.saturating_add(row_count.saturating_mul(AIR_WING_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!("air_wings len {} != {expected_len}", payload.len()));
        }
        let sparse = self.replace_or_sparse_after_first(AIR_WINGS_DOMAIN_ID, status_flags);
        if !sparse {
            self.air_wings.clear();
        }
        for index in 0..row_count {
            let base = AIR_WING_HEADER_LEN + index * AIR_WING_ROW_LEN;
            let owner_tag_db_id = read_u32(payload, base);
            let wing_id = read_u32(payload, base + 4);
            let generation = read_u64(payload, base + 8);
            let flags = read_u32(payload, base + 16);
            if flags & !AIR_WING_KNOWN_FLAGS != 0 {
                return Err("air_wings row has unknown flags".into());
            }
            let reserved = read_u32(payload, base + 36);
            if reserved != 0 {
                return Err("air_wings row reserved field is nonzero".into());
            }
            let key = AirWingKey {
                owner_tag_db_id,
                wing_id,
                generation,
            };
            if flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.air_wings.remove(&key);
                self.air_wing_loadout_orphan_check_keys_this_hour
                    .insert(key);
                continue;
            }
            let mission_region_raw = read_u32(payload, base + 28);
            self.air_wings.insert(
                key,
                AirWingState {
                    generation,
                    count: read_u32(payload, base + 20),
                    mission_mask: read_u32(payload, base + 24),
                    mission_region: (flags & AIR_WING_ROW_FLAG_MISSION_REGION != 0)
                        .then_some(mission_region_raw),
                    effective_planes: read_u32(payload, base + 32),
                },
            );
        }
        self.mark_fold_domain_success(AIR_WINGS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 31 air_region_activity (collector sparse after first) ------
    fn fold_air_region_activity(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_REGION_ACTIVITY_HEADER_LEN,
            "air_region_activity header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let observed_bucket_rows = read_u32(payload, 24) as usize;
        let deleted_rows = read_u32(payload, 28) as usize;
        let reserved0 = read_u32(payload, 32);
        ensure_count_at_most(
            "air_region_activity",
            "changed_rows",
            changed_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "air_region_activity",
            "deleted_rows",
            deleted_rows,
            "row_count",
            row_count,
        )?;
        ensure_row_count_with_deletes(
            "air_region_activity",
            row_count,
            "observed_bucket_rows",
            observed_bucket_rows,
            deleted_rows,
        )?;
        if reserved0 != 0 {
            return Err("air_region_activity header reserved field is nonzero".into());
        }
        if self.handle_compact_failure(
            AIR_REGION_ACTIVITY_DOMAIN_ID,
            "air_region_activity",
            status_flags,
        ) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            AIR_REGION_ACTIVITY_DOMAIN_ID,
            "air_region_activity",
            status_flags,
        ) {
            return Ok(());
        }
        let expected_len = AIR_REGION_ACTIVITY_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_REGION_ACTIVITY_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "air_region_activity len {} != {expected_len}",
                payload.len()
            ));
        }
        let sparse =
            self.replace_or_sparse_after_first(AIR_REGION_ACTIVITY_DOMAIN_ID, status_flags);
        if !sparse {
            self.air_region_activity.clear();
        }
        for index in 0..row_count {
            let base = AIR_REGION_ACTIVITY_HEADER_LEN + index * AIR_REGION_ACTIVITY_ROW_LEN;
            let region_id = read_u32(payload, base);
            let owner_tag_db_id = read_u32(payload, base + 4);
            let mission_mask = read_u32(payload, base + 8);
            let flags = read_u32(payload, base + 12);
            if flags & !ROW_FLAG_DELETED_LOW_BIT != 0 {
                return Err("air_region_activity row has unknown flags".into());
            }
            let reserved = read_u32(payload, base + 28);
            if reserved != 0 {
                return Err("air_region_activity row reserved field is nonzero".into());
            }
            let key = AirRegionActivityKey {
                region_id,
                owner_tag_db_id,
                mission_mask,
            };
            if flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.air_region_activity.remove(&key);
                continue;
            }
            self.air_region_activity.insert(
                key,
                AirRegionActivityState {
                    region_id,
                    owner_tag_db_id,
                    mission_mask,
                    wing_count: read_u32(payload, base + 16),
                    plane_count: read_u32(payload, base + 20),
                    effective_planes: read_u32(payload, base + 24),
                },
            );
        }
        self.mark_fold_domain_success(AIR_REGION_ACTIVITY_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 32 air_wing_loadouts (collector sparse after first) --------
    fn fold_air_wing_loadouts(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_WING_LOADOUT_HEADER_LEN,
            "air_wing_loadouts header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let observed_air_wing_rows = read_u32(payload, 20) as usize;
        let observed_equipment_rows = read_u32(payload, 24) as usize;
        let equipment_count = read_u32(payload, 28) as usize;
        let deleted_rows = read_u32(payload, 32) as usize;
        let scale_raw = read_u32(payload, 36);
        let reserved0 = read_u32(payload, 40);
        ensure_count_at_most(
            "air_wing_loadouts",
            "changed_rows",
            changed_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "air_wing_loadouts",
            "deleted_rows",
            deleted_rows,
            "row_count",
            row_count,
        )?;
        ensure_row_count_with_deletes(
            "air_wing_loadouts",
            row_count,
            "observed_air_wing_rows",
            observed_air_wing_rows,
            deleted_rows,
        )?;
        if equipment_count > observed_equipment_rows
            && status_flags & COMPACT_STATUS_DELTA_DATA == 0
        {
            return Err(format!(
                "air_wing_loadouts emitted_equipment_rows {equipment_count} > observed_equipment_rows {observed_equipment_rows}"
            ));
        }
        ensure_fixed_scale("air_wing_loadouts", "scale_raw", scale_raw)?;
        if reserved0 != 0 {
            return Err("air_wing_loadouts header reserved field is nonzero".into());
        }
        if self.handle_compact_failure(
            AIR_WING_LOADOUTS_DOMAIN_ID,
            "air_wing_loadouts",
            status_flags,
        ) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            AIR_WING_LOADOUTS_DOMAIN_ID,
            "air_wing_loadouts",
            status_flags,
        ) {
            return Ok(());
        }
        let rows_end = AIR_WING_LOADOUT_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_WING_LOADOUT_ROW_LEN));
        let expected_len =
            rows_end.saturating_add(equipment_count.saturating_mul(AIR_WING_LOADOUT_ENTRY_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "air_wing_loadouts len {} != {expected_len}",
                payload.len()
            ));
        }
        let mut rows = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let base = AIR_WING_LOADOUT_HEADER_LEN + index * AIR_WING_LOADOUT_ROW_LEN;
            let owner_tag_db_id = read_u32(payload, base);
            let wing_id = read_u32(payload, base + 4);
            let generation = read_u64(payload, base + 8);
            let flags = read_u32(payload, base + 16);
            if flags & !ROW_FLAG_DELETED_LOW_BIT != 0 {
                return Err("air_wing_loadouts row has unknown flags".into());
            }
            let entry_first = read_u32(payload, base + 24) as usize;
            let entry_count = read_u32(payload, base + 28) as usize;
            let entry_end = entry_first
                .checked_add(entry_count)
                .ok_or_else(|| "air_wing_loadouts equipment range overflow".to_string())?;
            if entry_end > equipment_count {
                return Err(format!(
                    "air_wing_loadouts equipment range {entry_first}+{entry_count} exceeds {equipment_count}"
                ));
            }
            rows.push(AirWingLoadoutRawRow {
                owner_tag_db_id,
                wing_id,
                generation,
                flags,
                air_base_id: read_u32(payload, base + 20),
                entry_first,
                entry_count,
            });
        }
        let mut equipment = Vec::with_capacity(equipment_count);
        for index in 0..equipment_count {
            let base = rows_end + index * AIR_WING_LOADOUT_ENTRY_ROW_LEN;
            let variant_id = read_u32(payload, base);
            let reserved = read_u32(payload, base + 4);
            if reserved != 0 {
                return Err("air_wing_loadouts equipment reserved field is nonzero".into());
            }
            equipment.push(AirWingLoadoutEntryState {
                variant_id,
                amount_raw: read_i64(payload, base + 8),
            });
        }
        let sparse = self.replace_or_sparse_after_first(AIR_WING_LOADOUTS_DOMAIN_ID, status_flags);
        if !sparse {
            self.air_wing_loadouts.clear();
        }
        for row in rows {
            let key = AirWingKey {
                owner_tag_db_id: row.owner_tag_db_id,
                wing_id: row.wing_id,
                generation: row.generation,
            };
            if row.flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.air_wing_loadouts.remove(&key);
                continue;
            }
            let entry_end = row.entry_first + row.entry_count;
            self.air_wing_loadout_orphan_check_keys_this_hour
                .insert(key.clone());
            self.air_wing_loadouts.insert(
                key,
                AirWingLoadoutState {
                    air_base_id: (row.air_base_id != 0).then_some(row.air_base_id),
                    equipment: equipment[row.entry_first..entry_end].to_vec(),
                },
            );
        }
        self.mark_fold_domain_success(AIR_WING_LOADOUTS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAINS 33/37 variant definitions (catalog upsert) ---------------
    fn fold_air_variant_defs(&mut self, payload: &[u8], domain_id: u16) -> Result<(), String> {
        let domain_name = if domain_id == AIR_VARIANT_DEFS_DOMAIN_ID {
            "air_variant_defs"
        } else {
            "stockpile_variant_defs"
        };
        need(payload, VARIANT_DEF_HEADER_LEN, "variant defs header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let emitted_module_rows = read_u32(payload, 28) as usize;
        let reserved0 = read_u32(payload, 36);
        let reserved1 = read_u32(payload, 40);
        if reserved0 != 0 || reserved1 != 0 {
            return Err(format!("{domain_name} header reserved field is nonzero"));
        }
        if self.handle_compact_failure(domain_id, domain_name, status_flags) {
            return Ok(());
        }
        let rows_end =
            VARIANT_DEF_HEADER_LEN.saturating_add(row_count.saturating_mul(VARIANT_DEF_ROW_LEN));
        need(payload, rows_end, "variant defs rows")?;
        let mut rows = Vec::with_capacity(row_count);
        let mut declared_modules = 0usize;
        for index in 0..row_count {
            let base = VARIANT_DEF_HEADER_LEN + index * VARIANT_DEF_ROW_LEN;
            let variant_id = read_u32(payload, base);
            if variant_id == 0 {
                return Err(format!("{domain_name} row has zero variant_id"));
            }
            let flags = read_u32(payload, base + 4);
            if flags & !VARIANT_DEF_KNOWN_FLAGS != 0 {
                return Err(format!("{domain_name} row has unknown flags"));
            }
            let module_first = read_u32(payload, base + 8) as usize;
            let module_count = read_u32(payload, base + 12) as usize;
            if module_count == 0 && flags & VARIANT_DEF_ROW_FLAG_MODULES != 0 {
                return Err(format!("{domain_name} modules flag without modules"));
            }
            if module_count > 0 && flags & VARIANT_DEF_ROW_FLAG_MODULES == 0 {
                return Err(format!("{domain_name} modules without modules flag"));
            }
            let module_end = module_first
                .checked_add(module_count)
                .ok_or_else(|| format!("{domain_name} module range overflow"))?;
            if module_end > emitted_module_rows {
                return Err(format!(
                    "{domain_name} module range {module_first}+{module_count} exceeds {emitted_module_rows}"
                ));
            }
            let row_reserved0 = read_u32(payload, base + 16);
            let row_reserved1 = read_u32(payload, base + 20);
            if row_reserved0 != 0 || row_reserved1 != 0 {
                return Err(format!("{domain_name} row reserved field is nonzero"));
            }
            declared_modules = declared_modules.saturating_add(module_count);
            rows.push(VariantDefRawRow {
                variant_id,
                flags,
                module_first,
                module_count,
                display_name: None,
                variant_name: None,
            });
        }
        if declared_modules != emitted_module_rows {
            return Err(format!(
                "{domain_name} emitted_module_rows {emitted_module_rows} != declared {declared_modules}"
            ));
        }
        let mut cursor = rows_end;
        for row in &mut rows {
            let display_name = read_string(payload, &mut cursor, "variant display name")?;
            if display_name.is_empty() && row.flags & VARIANT_DEF_ROW_FLAG_DISPLAY_NAME != 0 {
                return Err(format!("{domain_name} display-name flag with empty string"));
            }
            if !display_name.is_empty() && row.flags & VARIANT_DEF_ROW_FLAG_DISPLAY_NAME == 0 {
                return Err(format!("{domain_name} display name without flag"));
            }
            row.display_name =
                (row.flags & VARIANT_DEF_ROW_FLAG_DISPLAY_NAME != 0).then_some(display_name);

            let variant_name = read_string(payload, &mut cursor, "variant name")?;
            if variant_name.is_empty() && row.flags & VARIANT_DEF_ROW_FLAG_VARIANT_NAME != 0 {
                return Err(format!("{domain_name} variant-name flag with empty string"));
            }
            if !variant_name.is_empty() && row.flags & VARIANT_DEF_ROW_FLAG_VARIANT_NAME == 0 {
                return Err(format!("{domain_name} variant name without flag"));
            }
            row.variant_name =
                (row.flags & VARIANT_DEF_ROW_FLAG_VARIANT_NAME != 0).then_some(variant_name);
        }
        let mut modules = Vec::with_capacity(emitted_module_rows);
        for _ in 0..emitted_module_rows {
            need(
                payload,
                cursor + VARIANT_DEF_MODULE_ROW_LEN,
                "variant def module",
            )?;
            let slot_handle = read_u32(payload, cursor);
            let reserved = read_u32(payload, cursor + 4);
            cursor += VARIANT_DEF_MODULE_ROW_LEN;
            if slot_handle == 0 {
                return Err(format!("{domain_name} module row has zero slot_handle"));
            }
            if reserved != 0 {
                return Err(format!(
                    "{domain_name} module row reserved field is nonzero"
                ));
            }
            let token = read_string(payload, &mut cursor, "variant def module token")?;
            if token.is_empty() {
                return Err(format!("{domain_name} module row has empty token"));
            }
            modules.push(VariantDefModuleState { slot_handle, token });
        }
        if cursor != payload.len() {
            return Err(format!("{domain_name} payload has trailing bytes"));
        }
        let target = if domain_id == AIR_VARIANT_DEFS_DOMAIN_ID {
            &mut self.air_variant_defs
        } else {
            &mut self.stockpile_variant_defs
        };
        for row in rows {
            let module_end = row.module_first + row.module_count;
            target.insert(
                row.variant_id,
                VariantDefState {
                    variant_id: row.variant_id,
                    display_name: row.display_name,
                    variant_name: row.variant_name,
                    modules: modules[row.module_first..module_end].to_vec(),
                },
            );
        }
        self.mark_fold_domain_success(domain_id);
        Ok(())
    }

    // --- DOMAIN 34 air_base_status (collector sparse after first) ----------
    fn fold_air_base_status(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_BASE_STATUS_HEADER_LEN,
            "air_base_status header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let reserved = read_u32(payload, 32);
        if reserved != 0 {
            return Err("air_base_status header reserved field is nonzero".into());
        }
        if self.handle_compact_failure(AIR_BASE_STATUS_DOMAIN_ID, "air_base_status", status_flags) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            AIR_BASE_STATUS_DOMAIN_ID,
            "air_base_status",
            status_flags,
        ) {
            return Ok(());
        }
        let expected_len = AIR_BASE_STATUS_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_BASE_STATUS_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "air_base_status len {} != {expected_len}",
                payload.len()
            ));
        }
        let sparse = self.replace_or_sparse_after_first(AIR_BASE_STATUS_DOMAIN_ID, status_flags);
        if !sparse {
            self.air_base_status.clear();
        }
        for index in 0..row_count {
            let base = AIR_BASE_STATUS_HEADER_LEN + index * AIR_BASE_STATUS_ROW_LEN;
            let air_base_id = read_u32(payload, base);
            let state_id = read_u32(payload, base + 4);
            let flags = read_u32(payload, base + 8);
            if flags & !ROW_FLAG_DELETED_LOW_BIT != 0 {
                return Err("air_base_status row has unknown flags".into());
            }
            let row_reserved = read_u32(payload, base + 28);
            if row_reserved != 0 {
                return Err("air_base_status row reserved field is nonzero".into());
            }
            if flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.air_base_status.remove(&air_base_id);
                continue;
            }
            self.air_base_status.insert(
                air_base_id,
                AirBaseStatusState {
                    air_base_id,
                    state_id,
                    capacity: read_u32(payload, base + 12),
                    building_level: read_u32(payload, base + 16),
                    based_wing_count: read_u32(payload, base + 20),
                    based_plane_count: read_u32(payload, base + 24),
                },
            );
        }
        self.mark_fold_domain_success(AIR_BASE_STATUS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 35 air_variant_stats (catalog upsert) ----------------------
    fn fold_air_variant_stats(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_VARIANT_STATS_HEADER_LEN,
            "air_variant_stats header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let fixed_scale_raw = read_u32(payload, 28);
        let range_scale_raw = read_u32(payload, 32);
        if fixed_scale_raw == 0 || range_scale_raw == 0 {
            return Err("air_variant_stats scale fields must be nonzero".into());
        }
        if self.handle_compact_failure(
            AIR_VARIANT_STATS_DOMAIN_ID,
            "air_variant_stats",
            status_flags,
        ) {
            return Ok(());
        }
        let expected_len = AIR_VARIANT_STATS_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_VARIANT_STATS_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "air_variant_stats len {} != {expected_len}",
                payload.len()
            ));
        }
        for index in 0..row_count {
            let base = AIR_VARIANT_STATS_HEADER_LEN + index * AIR_VARIANT_STATS_ROW_LEN;
            let variant_id = read_u32(payload, base);
            if variant_id == 0 {
                return Err("air_variant_stats row has zero variant_id".into());
            }
            let reserved = read_u32(payload, base + 60);
            if reserved != 0 {
                return Err("air_variant_stats row reserved field is nonzero".into());
            }
            self.air_variant_stats.insert(
                variant_id,
                AirVariantStatsState {
                    variant_id,
                    flags: read_u32(payload, base + 4),
                    equipment_token_raw: read_u32(payload, base + 8),
                    equipment_type_token_raw: read_u32(payload, base + 12),
                    interface_type_raw: read_u32(payload, base + 16),
                    speed_cache_raw: read_i32(payload, base + 20),
                    range_raw: read_i32(payload, base + 24),
                    air_attack_raw: read_i32(payload, base + 28),
                    agility_raw: read_i32(payload, base + 32),
                    max_speed_raw: read_i32(payload, base + 36),
                    reliability_raw: read_i32(payload, base + 40),
                    air_defence_raw: read_i32(payload, base + 44),
                    thrust_raw: read_i32(payload, base + 48),
                    fuel_consumption_raw: read_i32(payload, base + 52),
                    build_cost_ic_raw: read_i32(payload, base + 56),
                    fixed_scale_raw,
                    range_scale_raw,
                },
            );
        }
        self.mark_fold_domain_success(AIR_VARIANT_STATS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 36 air_variant_reserves (collector sparse after first) -----
    fn fold_air_variant_reserves(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_VARIANT_RESERVE_HEADER_LEN,
            "air_variant_reserves header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let observed_country_rows = read_u32(payload, 20) as usize;
        let observed_stockpile_rows = read_u32(payload, 24) as usize;
        let matched_stockpile_rows = read_u32(payload, 32) as usize;
        let invalid_country_rows = read_u32(payload, 36) as usize;
        let deleted_rows = read_u32(payload, 40) as usize;
        let scale_raw = read_u32(payload, 44);
        ensure_count_at_most(
            "air_variant_reserves",
            "changed_rows",
            changed_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "air_variant_reserves",
            "matched_stockpile_rows",
            matched_stockpile_rows,
            "observed_stockpile_rows",
            observed_stockpile_rows,
        )?;
        ensure_count_at_most(
            "air_variant_reserves",
            "invalid_country_rows",
            invalid_country_rows,
            "observed_country_rows",
            observed_country_rows,
        )?;
        ensure_count_at_most(
            "air_variant_reserves",
            "deleted_rows",
            deleted_rows,
            "row_count",
            row_count,
        )?;
        ensure_fixed_scale("air_variant_reserves", "scale_raw", scale_raw)?;
        if self.handle_compact_failure(
            AIR_VARIANT_RESERVES_DOMAIN_ID,
            "air_variant_reserves",
            status_flags,
        ) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            AIR_VARIANT_RESERVES_DOMAIN_ID,
            "air_variant_reserves",
            status_flags,
        ) {
            return Ok(());
        }
        let expected_len = AIR_VARIANT_RESERVE_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_VARIANT_RESERVE_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "air_variant_reserves len {} != {expected_len}",
                payload.len()
            ));
        }
        let sparse =
            self.replace_or_sparse_after_first(AIR_VARIANT_RESERVES_DOMAIN_ID, status_flags);
        if !sparse {
            self.air_variant_reserves.clear();
        }
        for index in 0..row_count {
            let base = AIR_VARIANT_RESERVE_HEADER_LEN + index * AIR_VARIANT_RESERVE_ROW_LEN;
            let tag_db_id = read_u32(payload, base);
            let variant_id = read_u32(payload, base + 4);
            let flags = read_u32(payload, base + 8);
            if flags & !ROW_FLAG_DELETED_LOW_BIT != 0 {
                return Err("air_variant_reserves row has unknown flags".into());
            }
            let reserved = read_u32(payload, base + 12);
            if reserved != 0 {
                return Err("air_variant_reserves row reserved field is nonzero".into());
            }
            let key = AirVariantReserveKey {
                tag_db_id,
                variant_id,
            };
            if flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.air_variant_reserves.remove(&key);
                continue;
            }
            self.air_variant_reserves.insert(
                key,
                AirVariantReserveState {
                    tag_db_id,
                    variant_id,
                    amount_raw: read_i64(payload, base + 16),
                    scale_raw,
                },
            );
        }
        self.mark_fold_domain_success(AIR_VARIANT_RESERVES_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 38 air_superiority_inputs (collector sparse after first) ---
    fn fold_air_superiority_inputs(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_SUPERIORITY_INPUT_HEADER_LEN,
            "air_superiority_inputs header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let observed_air_wing_rows = read_u32(payload, 20) as usize;
        let observed_loadout_rows = read_u32(payload, 24) as usize;
        let observed_equipment_rows = read_u32(payload, 28) as usize;
        let invalid_variant_rows = read_u32(payload, 40) as usize;
        let deleted_rows = read_u32(payload, 44) as usize;
        let scale_raw = read_u32(payload, 48);
        ensure_count_at_most(
            "air_superiority_inputs",
            "changed_rows",
            changed_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "air_superiority_inputs",
            "observed_loadout_rows",
            observed_loadout_rows,
            "observed_air_wing_rows",
            observed_air_wing_rows,
        )?;
        ensure_count_at_most(
            "air_superiority_inputs",
            "invalid_variant_rows",
            invalid_variant_rows,
            "observed_equipment_rows",
            observed_equipment_rows,
        )?;
        ensure_count_at_most(
            "air_superiority_inputs",
            "deleted_rows",
            deleted_rows,
            "row_count",
            row_count,
        )?;
        ensure_row_count_with_deletes(
            "air_superiority_inputs",
            row_count,
            "observed_equipment_rows",
            observed_equipment_rows,
            deleted_rows,
        )?;
        ensure_fixed_scale("air_superiority_inputs", "scale_raw", scale_raw)?;
        if self.handle_compact_failure(
            AIR_SUPERIORITY_INPUTS_DOMAIN_ID,
            "air_superiority_inputs",
            status_flags,
        ) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            AIR_SUPERIORITY_INPUTS_DOMAIN_ID,
            "air_superiority_inputs",
            status_flags,
        ) {
            return Ok(());
        }
        let expected_len = AIR_SUPERIORITY_INPUT_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_SUPERIORITY_INPUT_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "air_superiority_inputs len {} != {expected_len}",
                payload.len()
            ));
        }
        let sparse =
            self.replace_or_sparse_after_first(AIR_SUPERIORITY_INPUTS_DOMAIN_ID, status_flags);
        if !sparse {
            self.air_superiority_inputs.clear();
        }
        for index in 0..row_count {
            let base = AIR_SUPERIORITY_INPUT_HEADER_LEN + index * AIR_SUPERIORITY_INPUT_ROW_LEN;
            let region_id = read_u32(payload, base);
            let owner_tag_db_id = read_u32(payload, base + 4);
            let mission_mask = read_u32(payload, base + 8);
            let variant_id = read_u32(payload, base + 12);
            let flags = read_u32(payload, base + 16);
            if flags & !ROW_FLAG_DELETED_LOW_BIT != 0 {
                return Err("air_superiority_inputs row has unknown flags".into());
            }
            let reserved = read_u32(payload, base + 36);
            if reserved != 0 {
                return Err("air_superiority_inputs row reserved field is nonzero".into());
            }
            let key = AirSuperiorityInputKey {
                region_id,
                owner_tag_db_id,
                mission_mask,
                variant_id,
            };
            if flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.air_superiority_inputs.remove(&key);
                continue;
            }
            self.air_superiority_inputs.insert(
                key,
                AirSuperiorityInputState {
                    region_id,
                    owner_tag_db_id,
                    mission_mask,
                    variant_id,
                    equipment_token_raw: read_u32(payload, base + 20),
                    equipment_type_token_raw: read_u32(payload, base + 24),
                    interface_type_raw: read_u32(payload, base + 28),
                    wing_count: read_u32(payload, base + 32),
                    amount_raw: read_i64(payload, base + 40),
                    scale_raw,
                },
            );
        }
        self.mark_fold_domain_success(AIR_SUPERIORITY_INPUTS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAINS 39/40 variant upgrades (catalog upsert) ------------------
    fn fold_variant_upgrades(&mut self, payload: &[u8], domain_id: u16) -> Result<(), String> {
        let domain_name = if domain_id == AIR_VARIANT_UPGRADES_DOMAIN_ID {
            "air_variant_upgrades"
        } else {
            "stockpile_variant_upgrades"
        };
        need(
            payload,
            VARIANT_UPGRADE_HEADER_LEN,
            "variant upgrades header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let emitted_upgrade_rows = read_u32(payload, 28) as usize;
        let reserved0 = read_u32(payload, 36);
        let reserved1 = read_u32(payload, 40);
        if reserved0 != 0 || reserved1 != 0 {
            return Err(format!("{domain_name} header reserved field is nonzero"));
        }
        if self.handle_compact_failure(domain_id, domain_name, status_flags) {
            return Ok(());
        }
        let rows_end = VARIANT_UPGRADE_HEADER_LEN
            .saturating_add(row_count.saturating_mul(VARIANT_UPGRADE_ROW_LEN));
        let expected_len = rows_end
            .saturating_add(emitted_upgrade_rows.saturating_mul(VARIANT_UPGRADE_ENTRY_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "{domain_name} len {} != {expected_len}",
                payload.len()
            ));
        }
        let mut rows = Vec::with_capacity(row_count);
        let mut declared_upgrades = 0usize;
        for index in 0..row_count {
            let base = VARIANT_UPGRADE_HEADER_LEN + index * VARIANT_UPGRADE_ROW_LEN;
            let variant_id = read_u32(payload, base);
            if variant_id == 0 {
                return Err(format!("{domain_name} row has zero variant_id"));
            }
            let flags = read_u32(payload, base + 4);
            if flags != 0 {
                return Err(format!("{domain_name} row has unknown flags"));
            }
            let upgrade_first = read_u32(payload, base + 8) as usize;
            let upgrade_count = read_u32(payload, base + 12) as usize;
            let upgrade_end = upgrade_first
                .checked_add(upgrade_count)
                .ok_or_else(|| format!("{domain_name} upgrade range overflow"))?;
            if upgrade_end > emitted_upgrade_rows {
                return Err(format!(
                    "{domain_name} upgrade range {upgrade_first}+{upgrade_count} exceeds {emitted_upgrade_rows}"
                ));
            }
            declared_upgrades = declared_upgrades.saturating_add(upgrade_count);
            rows.push((variant_id, upgrade_first, upgrade_count));
        }
        if declared_upgrades != emitted_upgrade_rows {
            return Err(format!(
                "{domain_name} emitted_upgrade_rows {emitted_upgrade_rows} != declared {declared_upgrades}"
            ));
        }
        let mut upgrades = Vec::with_capacity(emitted_upgrade_rows);
        for index in 0..emitted_upgrade_rows {
            let base = rows_end + index * VARIANT_UPGRADE_ENTRY_ROW_LEN;
            upgrades.push(VariantUpgradeState {
                upgrade_index: read_u32(payload, base),
                level: read_u32(payload, base + 4),
            });
        }
        let target = if domain_id == AIR_VARIANT_UPGRADES_DOMAIN_ID {
            &mut self.air_variant_upgrades
        } else {
            &mut self.stockpile_variant_upgrades
        };
        for (variant_id, upgrade_first, upgrade_count) in rows {
            let upgrade_end = upgrade_first + upgrade_count;
            target.insert(variant_id, upgrades[upgrade_first..upgrade_end].to_vec());
        }
        self.mark_fold_domain_success(domain_id);
        Ok(())
    }

    // --- DOMAIN 42 air_combat_losses (per-hour R2 sidecar, no events[]) ---
    fn fold_air_combat_losses(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_COMBAT_LOSS_HEADER_LEN,
            "air_combat_losses header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let loss_rows = read_u32(payload, 28) as usize;
        let reinforcement_rows = read_u32(payload, 32) as usize;
        let deleted_rows = read_u32(payload, 36) as usize;
        let scale_raw = read_u32(payload, 40);
        let reserved = read_u32(payload, 44);
        if changed_rows != row_count {
            return Err("air_combat_losses changed_rows != row_count".into());
        }
        ensure_count_at_most(
            "air_combat_losses",
            "loss_rows",
            loss_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "air_combat_losses",
            "reinforcement_rows",
            reinforcement_rows,
            "row_count",
            row_count,
        )?;
        ensure_count_at_most(
            "air_combat_losses",
            "deleted_rows",
            deleted_rows,
            "row_count",
            row_count,
        )?;
        ensure_fixed_scale("air_combat_losses", "scale_raw", scale_raw)?;
        if reserved != 0 {
            return Err("air_combat_losses header reserved field is nonzero".into());
        }
        if self.handle_compact_failure(
            AIR_COMBAT_LOSSES_DOMAIN_ID,
            "air_combat_losses",
            status_flags,
        ) {
            return Ok(());
        }
        let expected_len = AIR_COMBAT_LOSS_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_COMBAT_LOSS_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "air_combat_losses len {} != {expected_len}",
                payload.len()
            ));
        }
        let mut rows = Vec::with_capacity(row_count);
        // Frame-scope identity: a wing_id may carry at most one live
        // (non-deleted) scalar row per payload; a deleted row plus one live
        // row is a wing recreated at a new body address (deletion applies
        // first regardless of payload order). Two live rows for one wing_id
        // are a torn/stale identity and fail the fold closed.
        let mut live_wing_rows_this_payload: BTreeSet<u32> = BTreeSet::new();
        let mut deleted_wing_rows_this_payload: BTreeSet<u32> = BTreeSet::new();
        let mut cumulative_upserts: Vec<(u32, AirWingCumulativeScalars)> = Vec::new();
        for index in 0..row_count {
            let base = AIR_COMBAT_LOSS_HEADER_LEN + index * AIR_COMBAT_LOSS_ROW_LEN;
            let flags = read_u32(payload, base + 16);
            if flags & !AIR_STRENGTH_DELTA_KNOWN_FLAGS != 0 {
                return Err("air_combat_losses row has unknown flags".into());
            }
            let reserved = read_u32(payload, base + 40);
            if reserved != 0 {
                return Err("air_combat_losses row reserved field is nonzero".into());
            }
            let row = AirCombatLossState {
                owner_tag_db_id: read_u32(payload, base),
                wing_id: read_u32(payload, base + 4),
                generation: read_u64(payload, base + 8),
                flags,
                mission_mask: read_u32(payload, base + 20),
                mission_region: if flags & AIR_STRENGTH_DELTA_ROW_FLAG_MISSION_REGION != 0 {
                    Some(read_u32(payload, base + 24))
                } else {
                    None
                },
                old_count: read_u32(payload, base + 28),
                new_count: read_u32(payload, base + 32),
                count_delta: read_i32(payload, base + 36),
                old_equipment_amount_sum: read_i64(payload, base + 44),
                new_equipment_amount_sum: read_i64(payload, base + 52),
                equipment_amount_delta: read_i64(payload, base + 60),
                air_accidents: read_u32(payload, base + 68),
                experience_raw: read_i32(payload, base + 72),
                scale_raw,
            };
            let deleted = flags & AIR_STRENGTH_DELTA_ROW_FLAG_DELETED != 0;
            let baseline = flags & AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE != 0;
            let wing_scalars = flags & AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS != 0;
            let event = flags
                & (AIR_STRENGTH_DELTA_ROW_FLAG_LOSS
                    | AIR_STRENGTH_DELTA_ROW_FLAG_REINFORCEMENT
                    | AIR_STRENGTH_DELTA_ROW_FLAG_DELETED)
                != 0;
            if baseline && event {
                return Err("air_combat_losses baseline row carries event flags".into());
            }
            if !wing_scalars && (row.air_accidents != 0 || row.experience_raw != 0) {
                return Err(
                    "air_combat_losses row has wing scalar values without measured flag".into(),
                );
            }
            if wing_scalars && !(0..=AIR_WING_MAX_EXPERIENCE_RAW).contains(&row.experience_raw) {
                return Err("air_combat_losses row has invalid experience_raw".into());
            }
            if wing_scalars && row.air_accidents > AIR_WING_MAX_ACCIDENTS {
                return Err("air_combat_losses row has invalid air_accidents".into());
            }
            if deleted {
                if !deleted_wing_rows_this_payload.insert(row.wing_id) {
                    return Err(format!(
                        "air_combat_losses wing_id {} deleted twice in one payload",
                        row.wing_id
                    ));
                }
            } else {
                if !live_wing_rows_this_payload.insert(row.wing_id) {
                    return Err(format!(
                        "air_combat_losses wing_id {} appears twice in one payload",
                        row.wing_id
                    ));
                }
                if wing_scalars {
                    cumulative_upserts.push((
                        row.wing_id,
                        AirWingCumulativeScalars {
                            owner_tag_db_id: row.owner_tag_db_id,
                            air_accidents: row.air_accidents,
                            experience_raw: row.experience_raw,
                        },
                    ));
                }
            }
            // The hourly event array keeps its historical meaning: actual
            // loss/reinforcement/delete events only. Baseline and scalar-only
            // rows feed the persistent cumulative map below.
            if event {
                rows.push(row);
            }
        }
        // Deletions first, then live upserts, so a wing deleted and recreated
        // in the same payload survives regardless of row ordering.
        for wing_id in &deleted_wing_rows_this_payload {
            self.air_wing_cumulative.remove(wing_id);
        }
        for (wing_id, state) in cumulative_upserts {
            self.air_wing_cumulative.insert(wing_id, state);
        }
        self.air_combat_losses_this_hour = Some(rows);
        self.mark_fold_domain_success(AIR_COMBAT_LOSSES_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 61 air_loss_stats_live (per-country cumulative stat-23) ----
    fn fold_air_loss_stats(&mut self, payload: &[u8]) -> Result<(), String> {
        let (header, rows) =
            airloss61::decode_payload_rows(payload).map_err(|errors| errors.join("; "))?;
        if self.handle_compact_failure(
            AIR_LOSS_STATS_DOMAIN_ID,
            "air_loss_stats_live",
            header.status_flags,
        ) {
            return Ok(());
        }
        let mut fresh = BTreeMap::new();
        for row in rows {
            if row.owner_tag_db_id != row.country_idx {
                return Err("air_loss_stats_live owner_tag_db_id != country_idx".into());
            }
            if fresh.insert(row.owner_tag_db_id, row).is_some() {
                return Err(format!(
                    "air_loss_stats_live duplicate country row {}",
                    row.owner_tag_db_id
                ));
            }
        }
        self.air_loss_stats = fresh;
        self.mark_fold_domain_success(AIR_LOSS_STATS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 62 hub_motorization_live (supply-node membership) ----------
    fn fold_hub_motorization(&mut self, payload: &[u8]) -> Result<(), String> {
        let (header, rows) = match hub62::decode_payload_rows(payload) {
            Ok(decoded) => decoded,
            Err(errors) => {
                self.mark_fold_domain_unknown(
                    HUB_MOTORIZATION_DOMAIN_ID,
                    "payload_decode_failed",
                    "hub_motorization_live state unknown this hour; accumulator retained",
                );
                return Err(errors.join("; "));
            }
        };
        if self.handle_compact_failure(
            HUB_MOTORIZATION_DOMAIN_ID,
            "hub_motorization_live",
            header.status_flags,
        ) {
            return Ok(());
        }
        let mut fresh = BTreeMap::new();
        let mut map_entries = 0u64;
        for row in rows {
            if row.map_size == 0 {
                self.mark_fold_domain_unknown(
                    HUB_MOTORIZATION_DOMAIN_ID,
                    "zero_map_size_row",
                    "hub_motorization_live state unknown this hour; accumulator retained",
                );
                return Err("hub_motorization_live row has zero map_size".into());
            }
            map_entries = map_entries.saturating_add(u64::from(row.map_size));
            if fresh
                .insert((row.owner_tag_db_id, row.province_id), row)
                .is_some()
            {
                self.mark_fold_domain_unknown(
                    HUB_MOTORIZATION_DOMAIN_ID,
                    "duplicate_country_province_row",
                    "hub_motorization_live state unknown this hour; accumulator retained",
                );
                return Err(format!(
                    "hub_motorization_live duplicate country/province row {}:{}",
                    row.owner_tag_db_id, row.province_id
                ));
            }
        }
        if map_entries != u64::from(header.total_map_entries) {
            self.mark_fold_domain_unknown(
                HUB_MOTORIZATION_DOMAIN_ID,
                "map_entry_total_mismatch",
                "hub_motorization_live state unknown this hour; accumulator retained",
            );
            return Err(format!(
                "hub_motorization_live map_entries {map_entries} != header {}",
                header.total_map_entries
            ));
        }
        self.hub_motorization = fresh;
        self.mark_fold_domain_success(HUB_MOTORIZATION_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 46 air_variant_archetypes (implicit first baseline) --------
    fn fold_air_variant_archetypes(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            AIR_VARIANT_ARCHETYPE_HEADER_LEN,
            "air_variant_archetypes header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let classified_rows = read_u32(payload, 24) as usize;
        let unresolved_rows = read_u32(payload, 28) as usize;
        let static_catalog_hash = read_u32(payload, 32);
        let fixed_scale_raw = read_u32(payload, 36);
        let reserved0 = read_u32(payload, 40);
        let reserved1 = read_u32(payload, 44);
        if classified_rows.saturating_add(unresolved_rows) != row_count {
            return Err("air_variant_archetypes classified+unresolved != row_count".into());
        }
        if fixed_scale_raw == 0 {
            return Err("air_variant_archetypes fixed scale is zero".into());
        }
        if reserved0 != 0 || reserved1 != 0 {
            return Err("air_variant_archetypes header reserved field is nonzero".into());
        }
        if self.handle_compact_failure(
            AIR_VARIANT_ARCHETYPES_DOMAIN_ID,
            "air_variant_archetypes",
            status_flags,
        ) {
            return Ok(());
        }
        let rows_end = AIR_VARIANT_ARCHETYPE_HEADER_LEN
            .saturating_add(row_count.saturating_mul(AIR_VARIANT_ARCHETYPE_ROW_LEN));
        need(payload, rows_end, "air_variant_archetypes rows")?;
        let mut rows = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let base = AIR_VARIANT_ARCHETYPE_HEADER_LEN + index * AIR_VARIANT_ARCHETYPE_ROW_LEN;
            let variant_id = read_u32(payload, base);
            if variant_id == 0 {
                return Err("air_variant_archetypes row has zero variant_id".into());
            }
            let flags = read_u32(payload, base + 4);
            if flags & !AIR_VARIANT_ARCHETYPE_KNOWN_FLAGS != 0 {
                return Err("air_variant_archetypes row has unknown flags".into());
            }
            let reserved = read_u32(payload, base + 28);
            if reserved != 0 {
                return Err("air_variant_archetypes row reserved field is nonzero".into());
            }
            rows.push(AirVariantArchetypeRawRow {
                variant_id,
                flags,
                equipment_token_raw: read_u32(payload, base + 8),
                equipment_type_token_raw: read_u32(payload, base + 12),
                interface_type_raw: read_u32(payload, base + 16),
                air_superiority_weight_raw: read_i32(payload, base + 20),
                source_mask: read_u32(payload, base + 24),
                archetype_token: None,
                base_airframe_token: None,
                role_token: None,
            });
        }
        let mut cursor = rows_end;
        for row in &mut rows {
            let archetype_token = read_string(payload, &mut cursor, "air archetype token")?;
            let base_airframe_token = read_string(payload, &mut cursor, "air base airframe token")?;
            let role_token = read_string(payload, &mut cursor, "air role token")?;
            row.archetype_token = validate_optional_flagged_string(
                archetype_token,
                row.flags,
                AIR_VARIANT_ARCHETYPE_ROW_FLAG_ARCHETYPE,
                "air_variant_archetypes archetype",
            )?;
            row.base_airframe_token = validate_optional_flagged_string(
                base_airframe_token,
                row.flags,
                AIR_VARIANT_ARCHETYPE_ROW_FLAG_BASE_AIRFRAME,
                "air_variant_archetypes base_airframe",
            )?;
            row.role_token = validate_optional_flagged_string(
                role_token,
                row.flags,
                AIR_VARIANT_ARCHETYPE_ROW_FLAG_ROLE,
                "air_variant_archetypes role",
            )?;
        }
        // Wave-4 ace lifecycle: bytes after the self-delimiting base payload
        // are the `ACE1` ace-roster trailer; anything else fails the record
        // exactly as before. Parsed first (strict), applied after the base
        // archetype fold below.
        let ace_frame = if cursor != payload.len() {
            Some(
                ace46::decode_trailer_rows(payload, cursor)
                    .map_err(|error| format!("air_variant_archetypes ace trailer: {error}"))?,
            )
        } else {
            None
        };
        // D46's lane always sets DELTA_DATA, but the first successful frame is a
        // complete implicit baseline; later frames are sparse upsert/deletes.
        let sparse = self.implicit_delta_baseline_or_sparse(AIR_VARIANT_ARCHETYPES_DOMAIN_ID);
        if !sparse {
            self.air_variant_archetypes.clear();
        }
        for row in rows {
            if row.flags & ROW_FLAG_DELETED_LOW_BIT != 0 {
                self.air_variant_archetypes.remove(&row.variant_id);
                continue;
            }
            self.air_variant_archetypes.insert(
                row.variant_id,
                AirVariantArchetypeState {
                    variant_id: row.variant_id,
                    equipment_token_raw: row.equipment_token_raw,
                    equipment_type_token_raw: row.equipment_type_token_raw,
                    interface_type_raw: row.interface_type_raw,
                    air_superiority_weight_raw: if row.flags
                        & AIR_VARIANT_ARCHETYPE_ROW_FLAG_AIR_SUPERIORITY_WEIGHT
                        != 0
                    {
                        Some(row.air_superiority_weight_raw)
                    } else {
                        None
                    },
                    source_mask: row.source_mask,
                    archetype_token: row.archetype_token,
                    base_airframe_token: row.base_airframe_token,
                    role_token: row.role_token,
                    static_catalog_hash,
                    fixed_scale_raw,
                },
            );
        }
        if let Some(frame) = ace_frame {
            self.apply_air_ace_roster(frame);
        }
        self.mark_fold_domain_success(AIR_VARIANT_ARCHETYPES_DOMAIN_ID);
        Ok(())
    }

    /// Apply a decoded `ACE1` ace-roster trailer.
    ///
    /// FULL-REPLACE of the ace map + per-country `_r2_air_ace_roster_live`
    /// sidecars ONLY on a clean, complete frame. A read failure must never be
    /// written into the replay as ground-truth absence, so:
    /// - `SOURCE_UNAVAILABLE` (country roots missing entirely) => RETAIN prior
    ///   ace state, badge the domain problem, do not touch the map/sidecars;
    /// - `PARTIAL_DATA` (roster incomplete — at least one existing country/ace
    ///   could not be read this frame) => RETAIN prior state too. A partial
    ///   frame's full-replace set is missing real rows; clearing+replacing
    ///   would wipe the un-read country/ace for this hour and flicker it back
    ///   next clean frame (the Codex/coordinator BLOCKER).
    /// - `WING_ASSIGNMENT_BLOCKED` alone is an assignment-only anomaly
    ///   (identity is complete) => still REPLACE, with the badge recorded in
    ///   every country sidecar.
    ///
    /// Only a clean apply sets `air_aces_fresh_this_hour`, which gates the
    /// D30 snapshot `air_wings[*].ace` join so a retained (stale) roster is
    /// never surfaced as this hour's ground truth.
    fn apply_air_ace_roster(&mut self, frame: ace46::AceTrailerFrame) {
        let status_flags = frame.header.status_flags;
        if status_flags & ace46::ACE_STATUS_SOURCE_UNAVAILABLE != 0 {
            self.record_domain_problem(
                AIR_VARIANT_ARCHETYPES_DOMAIN_ID,
                "ace_roster_source_unavailable",
            );
            self.note(
                "air_ace_roster country roots unavailable this hour; ace state retained from prior valid hour",
            );
            return;
        }
        if status_flags & ace46::ACE_STATUS_PARTIAL_DATA != 0 {
            // Incomplete roster: some country/ace that DOES exist could not be
            // read. Retain the prior complete roster rather than wiping the
            // missing rows for this hour (fail closed: absent != could-not-read).
            self.record_domain_problem(AIR_VARIANT_ARCHETYPES_DOMAIN_ID, "ace_roster_partial");
            self.note(format!(
                "air_ace_roster incomplete this hour ({} row(s) unreadable); ace state retained from prior valid hour, not full-replaced",
                frame.header.skipped_rows
            ));
            return;
        }
        // Clean, complete frame: full-replace map + sidecars.
        self.air_aces.clear();
        for country in self.countries.values_mut() {
            if let Some(Value::Object(extra)) = country.get_mut("extra") {
                extra.remove(DOMAIN46_ACE_R2_KEY);
                if extra.is_empty() {
                    country.remove("extra");
                }
            }
        }
        let assignment_blocked = status_flags & ace46::ACE_STATUS_WING_ASSIGNMENT_BLOCKED != 0;
        for row in frame.rows {
            let tag_db_id = row.tag_db_id;
            let sidecar = {
                let extra = ensure_json_object_mut(self.country_mut(tag_db_id), "extra");
                ensure_json_object_mut(extra, DOMAIN46_ACE_R2_KEY)
            };
            if assignment_blocked {
                sidecar.insert("wing_assignment_blocked".into(), json!(true));
            }
            ensure_json_array_mut(sidecar, "aces").push(json!({
                "ace_id": row.ace_id,
                "country_ace_index": row.country_ace_index,
                "name": row.name,
                "surname": row.surname,
                "callsign": row.callsign,
                "alive": row.alive,
                "wing_id": row.wing_id,
                "killer_ace_id": row.killer_ace_id,
                "killer_country_tag_db_id": row.killer_country_tag_db_id,
            }));
            self.air_aces.insert((tag_db_id, row.ace_id), row);
        }
        self.air_aces_fresh_this_hour = true;
    }

    // --- DOMAIN 47 characters_leaders_cabinet_live (full each frame) ------
    fn fold_characters_leaders_cabinet_live(&mut self, payload: &[u8]) -> Result<(), String> {
        let decoded = domain47::decode_payload_rows(0, payload)
            .map_err(|errors| format!("characters_leaders_cabinet_live: {}", errors.join("; ")))?;
        if compact_status_is_failure(decoded.status_flags) {
            self.mark_fold_domain_unknown(
                CHARACTERS_LEADERS_CABINET_DOMAIN_ID,
                "failure_status",
                format!(
                    "characters_leaders_cabinet_live failure status {:#x}; character/leader/cabinet state retained from prior valid hour",
                    decoded.status_flags & COMPACT_STATUS_FAILURE_MASK
                ),
            );
            return Ok(());
        }
        if (decoded.status_flags & COMPACT_STATUS_DELTA_DATA) != 0 {
            self.mark_fold_domain_unknown(
                CHARACTERS_LEADERS_CABINET_DOMAIN_ID,
                "delta_unsupported",
                "characters_leaders_cabinet_live DELTA_DATA is unsupported; character/leader/cabinet state retained from prior valid hour",
            );
            return Ok(());
        }
        if decoded.rows.is_empty() {
            self.mark_fold_domain_unknown(
                CHARACTERS_LEADERS_CABINET_DOMAIN_ID,
                "empty_implausible",
                "characters_leaders_cabinet_live emitted zero rows without failure status; a loaded campaign with zero live characters is implausible, so prior character/leader/cabinet state was retained",
            );
            return Ok(());
        }

        // Excluded fail-closed scope (wave-3 870ebf narrowed it): advisor
        // roster, appointed advisor, and unit-leader trait rows are now
        // validated and fold below (into the domain-47 sidecar only; domain 14
        // remains the owner of politics.appointed_advisors). Portrait rows
        // stay blocked pending a validated static portrait feed.
        let excluded_rows = decoded
            .rows
            .iter()
            .filter(|row| matches!(row, domain47::DecodedDomain47Row::CharacterPortrait { .. }))
            .count();
        if excluded_rows != 0 {
            self.mark_fold_domain_unknown(
                CHARACTERS_LEADERS_CABINET_DOMAIN_ID,
                "excluded_rows_present",
                format!(
                    "characters_leaders_cabinet_live carried {excluded_rows} excluded portrait row(s); prior character/leader/cabinet state retained"
                ),
            );
            return Ok(());
        }
        // Trait rows carry only unit_leader_id; join them to their owning
        // country through this payload's own full-snapshot UnitLeader rows.
        // An orphan trait row means a torn/inconsistent snapshot: retain the
        // prior hour instead of guessing an owner (fail closed).
        let mut trait_leader_owner = BTreeMap::new();
        for row in &decoded.rows {
            if let domain47::DecodedDomain47Row::UnitLeader {
                country_tag_db_id,
                unit_leader_id,
                ..
            } = row
            {
                trait_leader_owner.insert(*unit_leader_id, *country_tag_db_id);
            }
        }
        let orphan_trait_rows = decoded
            .rows
            .iter()
            .filter(|row| {
                matches!(
                    row,
                    domain47::DecodedDomain47Row::UnitLeaderTrait { unit_leader_id, .. }
                        if !trait_leader_owner.contains_key(unit_leader_id)
                )
            })
            .count();
        if orphan_trait_rows != 0 {
            self.mark_fold_domain_unknown(
                CHARACTERS_LEADERS_CABINET_DOMAIN_ID,
                "orphan_trait_rows",
                format!(
                    "characters_leaders_cabinet_live carried {orphan_trait_rows} trait row(s) without a matching unit-leader row; prior character/leader/cabinet state retained"
                ),
            );
            return Ok(());
        }

        self.clear_domain47_country_fields();
        for row in decoded.rows {
            match row {
                domain47::DecodedDomain47Row::Character {
                    country_tag_db_id,
                    char_id,
                    name,
                    token,
                } => {
                    let sidecar = self.domain47_sidecar_mut(country_tag_db_id);
                    ensure_json_array_mut(sidecar, "characters").push(json!({
                        "char_id": char_id,
                        "name": name,
                        "token": token,
                    }));
                }
                domain47::DecodedDomain47Row::UnitLeader {
                    country_tag_db_id,
                    unit_leader_id,
                    char_id,
                    leader_kind,
                    skill,
                    experience_raw,
                    attack,
                    defense,
                    planning,
                    logistics,
                    maneuvering,
                    coordination,
                    token,
                    name,
                } => {
                    let sidecar = self.domain47_sidecar_mut(country_tag_db_id);
                    ensure_json_array_mut(sidecar, "unit_leaders").push(json!({
                        "unit_leader_id": unit_leader_id,
                        "char_id": char_id,
                        "leader_kind": domain47_unit_leader_kind(leader_kind),
                        "leader_kind_raw": leader_kind,
                        "skill": skill,
                        "experience_raw": experience_raw,
                        "attack": attack,
                        "defense": defense,
                        "planning": planning,
                        "logistics": logistics,
                        "maneuvering": maneuvering,
                        "coordination": coordination,
                        "token": token,
                        "name": name,
                    }));
                }
                domain47::DecodedDomain47Row::CountryLeaderRole {
                    country_tag_db_id,
                    char_id,
                    name,
                    token,
                } => {
                    let sidecar = self.domain47_sidecar_mut(country_tag_db_id);
                    ensure_json_array_mut(sidecar, "country_leader_roles").push(json!({
                        "char_id": char_id,
                        "name": name,
                        "token": token,
                    }));
                }
                domain47::DecodedDomain47Row::ActiveRuler {
                    country_tag_db_id,
                    char_id,
                    ruling_party,
                    name,
                    token,
                    leader_vec_size,
                    ambiguous,
                } => {
                    let commanders =
                        ensure_json_object_mut(self.country_mut(country_tag_db_id), "commanders");
                    commanders.insert(
                        "leader".into(),
                        json!({
                            "char_id": char_id,
                            "name": name.clone(),
                            "token": token.clone(),
                            "ideology_group": ruling_party.clone(),
                            "source": "C",
                            "ambiguous": ambiguous,
                        }),
                    );
                    let sidecar = self.domain47_sidecar_mut(country_tag_db_id);
                    ensure_json_array_mut(sidecar, "active_rulers").push(json!({
                        "char_id": char_id,
                        "ruling_party": ruling_party,
                        "name": name,
                        "token": token,
                        "leader_vec_size": leader_vec_size,
                        "ambiguous": ambiguous,
                    }));
                }
                domain47::DecodedDomain47Row::UnitLeaderTrait {
                    unit_leader_id,
                    ordinal,
                    trait_token,
                } => {
                    let country_tag_db_id = *trait_leader_owner
                        .get(&unit_leader_id)
                        .expect("orphan trait rows are rejected before the fold mutates state");
                    let sidecar = self.domain47_sidecar_mut(country_tag_db_id);
                    ensure_json_array_mut(sidecar, "unit_leader_traits").push(json!({
                        "unit_leader_id": unit_leader_id,
                        "ordinal": ordinal,
                        "trait_token": trait_token,
                    }));
                }
                domain47::DecodedDomain47Row::AdvisorRoster {
                    country_tag_db_id,
                    char_id,
                    slot,
                    ledger,
                    idea_token,
                } => {
                    let sidecar = self.domain47_sidecar_mut(country_tag_db_id);
                    ensure_json_array_mut(sidecar, "advisor_roster").push(json!({
                        "char_id": char_id,
                        "slot": slot,
                        "ledger": ledger,
                        "idea_token": idea_token,
                    }));
                }
                domain47::DecodedDomain47Row::AppointedAdvisor {
                    country_tag_db_id,
                    char_id,
                    slot,
                    ledger,
                    idea_token,
                } => {
                    let sidecar = self.domain47_sidecar_mut(country_tag_db_id);
                    ensure_json_array_mut(sidecar, "appointed_advisors").push(json!({
                        "char_id": char_id,
                        "slot": slot,
                        "ledger": ledger,
                        "idea_token": idea_token,
                    }));
                }
                domain47::DecodedDomain47Row::CharacterPortrait { .. } => unreachable!(
                    "excluded domain47 portrait rows are rejected before the fold mutates state"
                ),
            }
        }

        self.mark_fold_domain_success(CHARACTERS_LEADERS_CABINET_DOMAIN_ID);
        Ok(())
    }

    fn clear_domain47_country_fields(&mut self) {
        for country in self.countries.values_mut() {
            if let Some(Value::Object(commanders)) = country.get_mut("commanders") {
                commanders.remove("leader");
                if commanders.is_empty() {
                    country.remove("commanders");
                }
            }
            if let Some(Value::Object(extra)) = country.get_mut("extra") {
                extra.remove(DOMAIN47_R2_KEY);
                if extra.is_empty() {
                    country.remove("extra");
                }
            }
        }
    }

    fn domain47_sidecar_mut(&mut self, tag_db_id: u32) -> &mut Map<String, Value> {
        let extra = ensure_json_object_mut(self.country_mut(tag_db_id), "extra");
        ensure_json_object_mut(extra, DOMAIN47_R2_KEY)
    }

    // --- DOMAIN 52 occupation_resistance_compliance_live (baseline + delta) -
    fn fold_occupation_resistance_compliance(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, OCCUPATION_HEADER_LEN, "occupation header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let expected =
            OCCUPATION_HEADER_LEN.saturating_add(row_count.saturating_mul(OCCUPATION_ROW_LEN));
        if payload.len() != expected {
            return Err(format!("occupation len {} != {expected}", payload.len()));
        }
        if compact_status_is_failure(status_flags) {
            self.occupation_unknown_this_hour = true;
            self.note(format!(
                "occupation_resistance_compliance_live failure status {:#x}; occupation fields unknown this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        if delta && !self.delta_baselines_seen.contains(&52) {
            self.occupation_unknown_this_hour = true;
            self.record_domain_problem(52, "delta_before_baseline");
            self.note(
                "occupation_resistance_compliance_live delta_before_baseline; occupation fields unknown this hour",
            );
            return Ok(());
        }
        if !delta {
            for state in self.states.values_mut() {
                state.remove("occupation");
                state.remove("occupation_law");
            }
        }

        let mut cursor = OCCUPATION_HEADER_LEN;
        for row_index in 0..row_count {
            let state_id = read_u32(payload, cursor);
            let owner_tag_db_id = read_u32(payload, cursor + 4);
            let controller_tag_db_id = read_u32(payload, cursor + 8);
            let occupied_country_tag_db_id = read_u32(payload, cursor + 12);
            let resistance_raw = read_i32(payload, cursor + 16);
            let compliance_raw = read_i32(payload, cursor + 20);
            let resistance_speed_raw = read_i32(payload, cursor + 24);
            let compliance_speed_raw = read_i32(payload, cursor + 28);
            let garrison_manpower_need = read_i32(payload, cursor + 32);
            let garrison_strength_ratio_raw = read_i32(payload, cursor + 36);
            let garrison_required_raw = read_i32(payload, cursor + 40);
            let occupation_law_token_id = read_u32(payload, cursor + 44);
            let garrison_template_id = read_u32(payload, cursor + 48);
            let flags = read_u32(payload, cursor + 52);
            let reserved1 = read_u32(payload, cursor + 56);
            let reserved2 = read_u32(payload, cursor + 60);
            cursor += OCCUPATION_ROW_LEN;

            if state_id == 0 {
                return Err(format!("occupation row {row_index} has zero state_id"));
            }
            if flags & !OCC_ROW_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "occupation row {row_index} has unknown flags {flags:#x}"
                ));
            }
            if reserved1 != 0 || reserved2 != 0 {
                return Err(format!(
                    "occupation row {row_index} reserved field is nonzero"
                ));
            }
            if flags & OCC_ROW_FLAG_OWNER != 0 && owner_tag_db_id == 0 {
                return Err(format!(
                    "occupation row {row_index} has owner flag without tag"
                ));
            }
            if flags & OCC_ROW_FLAG_CONTROLLER != 0 && controller_tag_db_id == 0 {
                return Err(format!(
                    "occupation row {row_index} has controller flag without tag"
                ));
            }
            if flags & OCC_ROW_FLAG_OCCUPIED_TAG_DERIVED != 0 && occupied_country_tag_db_id == 0 {
                return Err(format!(
                    "occupation row {row_index} has occupied-country flag without tag"
                ));
            }

            let mut occupation = Map::new();
            if flags & OCC_ROW_FLAG_RESISTANCE != 0 {
                occupation.insert(
                    "resistance".into(),
                    json!(resistance_raw as f64 / FIXED_SCALE),
                );
            }
            if flags & OCC_ROW_FLAG_COMPLIANCE != 0 {
                occupation.insert(
                    "compliance".into(),
                    json!(compliance_raw as f64 / FIXED_SCALE),
                );
            }
            if flags & OCC_ROW_FLAG_RESISTANCE_SPEED != 0 {
                occupation.insert(
                    "resistance_speed".into(),
                    json!(resistance_speed_raw as f64 / FIXED_SCALE),
                );
            }
            if flags & OCC_ROW_FLAG_COMPLIANCE_SPEED != 0 {
                occupation.insert(
                    "compliance_speed".into(),
                    json!(compliance_speed_raw as f64 / FIXED_SCALE),
                );
            }
            if flags & OCC_ROW_FLAG_OCCUPIED_STATE != 0 {
                if flags & OCC_ROW_FLAG_CONTROLLER != 0 {
                    occupation.insert(
                        "occupier_tag".into(),
                        json!(controller_tag_db_id.to_string()),
                    );
                }
                if flags & OCC_ROW_FLAG_OCCUPIED_TAG_DERIVED != 0 {
                    occupation.insert(
                        "occupied_country_tag".into(),
                        json!(occupied_country_tag_db_id.to_string()),
                    );
                } else if flags & OCC_ROW_FLAG_OWNER != 0 {
                    occupation.insert(
                        "occupied_country_tag".into(),
                        json!(owner_tag_db_id.to_string()),
                    );
                }
            }

            let mut extra = Map::new();
            if flags & OCC_ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE != 0 {
                extra.insert("resistance_speed_status".into(), json!("unavailable"));
            }
            if flags & OCC_ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0 {
                if flags & OCC_ROW_FLAG_GARRISON_SCALARS != 0 {
                    return Err(format!(
                        "occupation row {row_index} marks garrison scalars both measured and unavailable"
                    ));
                }
                extra.insert("garrison_signals_status".into(), json!("unavailable"));
            } else if flags & OCC_ROW_FLAG_GARRISON_SCALARS != 0 {
                // army_manpower_need is a plain manpower count; the other two
                // scalars are fixed-x100000 (wave-3 crack, CStateGarrisonData
                // +0x88/+0xF0/+0xF8).
                extra.insert(
                    "garrison_army_manpower_need".into(),
                    json!(garrison_manpower_need),
                );
                extra.insert(
                    "garrison_strength_ratio".into(),
                    json!(garrison_strength_ratio_raw as f64 / FIXED_SCALE),
                );
                extra.insert(
                    "garrison_required".into(),
                    json!(garrison_required_raw as f64 / FIXED_SCALE),
                );
            } else if garrison_manpower_need != 0
                || garrison_strength_ratio_raw != 0
                || garrison_required_raw != 0
            {
                // Values without a measured badge are torn/foreign data:
                // fail the fold closed rather than emit them.
                return Err(format!(
                    "occupation row {row_index} has garrison scalar values without measured flag"
                ));
            }
            if flags & OCC_ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE != 0 {
                extra.insert("occupation_law_status".into(), json!("unavailable"));
            }
            if flags & OCC_ROW_FLAG_OCCUPATION_LAW_TOKEN != 0 {
                extra.insert(
                    "occupation_law_token_id".into(),
                    json!(occupation_law_token_id),
                );
                extra.insert(
                    "occupation_law_token_encoding".into(),
                    json!("fnv1a32_name_hash"),
                );
            }
            if flags & OCC_ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN != 0 {
                extra.insert(
                    "occupation_law_token_source".into(),
                    json!("controller_country_default"),
                );
            }
            if flags & OCC_ROW_FLAG_GARRISON_TEMPLATE != 0 {
                extra.insert("garrison_template_id".into(), json!(garrison_template_id));
            }
            if flags & OCC_ROW_FLAG_OCCUPATION_STATUS_ROOT != 0 {
                extra.insert("occupation_status_root".into(), json!(true));
            }
            if !extra.is_empty() {
                occupation.insert(
                    "_r2_occupation_resistance_compliance_live".into(),
                    Value::Object(extra),
                );
            }

            let state = self.states.entry(state_id.to_string()).or_default();
            state.insert("occupation_law".into(), Value::Null);
            if occupation.is_empty() {
                state.remove("occupation");
            } else {
                state.insert("occupation".into(), Value::Object(occupation));
            }
        }
        if !delta {
            self.delta_baselines_seen.insert(52);
        }
        Ok(())
    }

    // --- DOMAIN 24 weather_live (baseline + delta, R2 map sidecar) --------
    fn fold_weather(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, WEATHER_HEADER_LEN, "weather header")?;
        let region_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let province_count = read_u32(payload, 20) as usize;
        let observed_region_rows = read_u32(payload, 24) as usize;
        let observed_province_rows = read_u32(payload, 28) as usize;
        let scale_raw = read_u32(payload, 32);
        let active_region_count = read_u32(payload, 36) as usize;
        let reserved0 = read_u32(payload, 40);
        let reserved1 = read_u32(payload, 44);
        if self.handle_compact_failure(WEATHER_DOMAIN_ID, "weather_live", status_flags) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(WEATHER_DOMAIN_ID, "weather_live", status_flags) {
            return Ok(());
        }

        let emitted_rows = region_count.saturating_add(province_count);
        let expected_len = WEATHER_HEADER_LEN
            .saturating_add(region_count.saturating_mul(WEATHER_REGION_ROW_LEN))
            .saturating_add(province_count.saturating_mul(WEATHER_PROVINCE_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!("weather len {} != {expected_len}", payload.len()));
        }
        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        ensure_fixed_scale("weather", "scale_raw", scale_raw)?;
        if observed_region_rows != WEATHER_REGION_ROWS {
            return Err(format!(
                "weather observed_region_rows {observed_region_rows} != {WEATHER_REGION_ROWS}"
            ));
        }
        if observed_province_rows != 0
            && !(WEATHER_MIN_PROVINCE_ROWS..=WEATHER_MAX_PROVINCE_ROWS)
                .contains(&observed_province_rows)
        {
            return Err(format!(
                "weather observed_province_rows {observed_province_rows} outside {WEATHER_MIN_PROVINCE_ROWS}..={WEATHER_MAX_PROVINCE_ROWS}"
            ));
        }
        if !delta && region_count != WEATHER_REGION_ROWS {
            return Err(format!(
                "weather full snapshot region_count {region_count} != {WEATHER_REGION_ROWS}"
            ));
        }
        if !delta && observed_province_rows != 0 && province_count != observed_province_rows {
            return Err(format!(
                "weather full snapshot province_count {province_count} != observed_province_rows {observed_province_rows}"
            ));
        }
        if region_count > observed_region_rows {
            return Err(format!(
                "weather region_count {region_count} > observed_region_rows {observed_region_rows}"
            ));
        }
        if observed_province_rows == 0 && province_count != 0 {
            return Err(
                "weather province_count emitted with zero observed_province_rows".to_string(),
            );
        }
        if province_count > observed_province_rows {
            return Err(format!(
                "weather province_count {province_count} > observed_province_rows {observed_province_rows}"
            ));
        }
        if changed_rows > emitted_rows {
            return Err(format!(
                "weather changed_rows {changed_rows} > emitted_rows {emitted_rows}"
            ));
        }
        if delta && changed_rows != emitted_rows {
            return Err(format!(
                "weather delta changed_rows {changed_rows} != emitted_rows {emitted_rows}"
            ));
        }
        if active_region_count > region_count {
            return Err(format!(
                "weather active_region_count {active_region_count} > region_count {region_count}"
            ));
        }
        if reserved0 != 0 || reserved1 != 0 {
            return Err(format!(
                "weather reserved fields are nonzero ({reserved0}, {reserved1})"
            ));
        }

        let mut regions = Vec::with_capacity(region_count);
        let mut cursor = WEATHER_HEADER_LEN;
        let mut seen_regions = BTreeSet::new();
        let mut computed_active_regions = 0usize;
        for row_index in 0..region_count {
            let region_id = read_u32(payload, cursor);
            let flags = read_u32(payload, cursor + 4);
            let temperature_raw = read_i32(payload, cursor + 8);
            let phenomenon = read_u32(payload, cursor + 12);
            let next_weather_change_raw = read_u32(payload, cursor + 16);
            let active_modifier_count = read_u32(payload, cursor + 20);
            let intensity = read_u32(payload, cursor + 24);
            let row_reserved = read_u32(payload, cursor + 28);
            cursor += WEATHER_REGION_ROW_LEN;
            if region_id > WEATHER_MAX_REGION_ID || !seen_regions.insert(region_id) {
                return Err(format!(
                    "weather region row {row_index} has invalid/duplicate region_id {region_id}"
                ));
            }
            if flags & !WEATHER_REGION_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "weather region row {row_index} has unknown flags {flags}"
                ));
            }
            if !(-WEATHER_MAX_TEMPERATURE_RAW..=WEATHER_MAX_TEMPERATURE_RAW)
                .contains(&temperature_raw)
            {
                return Err(format!(
                    "weather region row {row_index} has invalid temperature_raw {temperature_raw}"
                ));
            }
            if phenomenon > WEATHER_MAX_PHENOMENON {
                return Err(format!(
                    "weather region row {row_index} has invalid phenomenon {phenomenon}"
                ));
            }
            if active_modifier_count > WEATHER_MAX_ACTIVE_MODIFIERS {
                return Err(format!(
                    "weather region row {row_index} has invalid active_modifier_count {active_modifier_count}"
                ));
            }
            if flags & WEATHER_REGION_ACTIVE_SIGNAL != 0 {
                computed_active_regions = computed_active_regions.saturating_add(1);
            }
            if row_reserved != 0 {
                return Err(format!(
                    "weather region row {row_index} reserved field is {row_reserved}"
                ));
            }
            regions.push(WeatherRegionState {
                region_id,
                flags,
                temperature_raw,
                phenomenon,
                next_weather_change_raw,
                active_modifier_count,
                intensity,
                scale_raw,
            });
        }
        if computed_active_regions != active_region_count {
            return Err(format!(
                "weather active_region_count {active_region_count} != computed {computed_active_regions}"
            ));
        }

        let mut provinces = Vec::with_capacity(province_count);
        let mut seen_provinces = BTreeSet::new();
        for row_index in 0..province_count {
            let province_id = read_u32(payload, cursor);
            let flags = read_u32(payload, cursor + 4);
            let temperature_raw = read_i32(payload, cursor + 8);
            let temperature_change_raw = read_i32(payload, cursor + 12);
            let water_raw = read_i32(payload, cursor + 16);
            let snow_raw = read_i32(payload, cursor + 20);
            let custom_modifier_count = read_u32(payload, cursor + 24);
            let row_reserved = read_u32(payload, cursor + 28);
            cursor += WEATHER_PROVINCE_ROW_LEN;
            if province_id == 0 || !seen_provinces.insert(province_id) {
                return Err(format!(
                    "weather province row {row_index} has invalid/duplicate province_id {province_id}"
                ));
            }
            if flags & !WEATHER_PROVINCE_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "weather province row {row_index} has unknown flags {flags}"
                ));
            }
            let temp_present = flags & WEATHER_PROVINCE_TEMP_PRESENT != 0;
            let temp_change_present = flags & WEATHER_PROVINCE_TEMP_CHANGE_PRESENT != 0;
            let water_present = flags & WEATHER_PROVINCE_WATER_PRESENT != 0;
            let snow_present = flags & WEATHER_PROVINCE_SNOW_PRESENT != 0;
            let mud_present = flags & WEATHER_PROVINCE_MUD_PRESENT != 0;
            let mud_active = flags & WEATHER_PROVINCE_MUD_ACTIVE != 0;
            let custom_present = flags & WEATHER_PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT != 0;
            let map_recolor = flags & WEATHER_PROVINCE_MAP_RECOLOR != 0;
            let dynamic_lut = flags & WEATHER_PROVINCE_DYNAMIC_LUT != 0;
            if mud_active && !mud_present {
                return Err(format!(
                    "weather province row {row_index} has mud_active without mud_present"
                ));
            }
            if map_recolor != (temp_present || water_present || snow_present) {
                return Err(format!(
                    "weather province row {row_index} has inconsistent map recolor flag"
                ));
            }
            if dynamic_lut != (snow_present || mud_present) {
                return Err(format!(
                    "weather province row {row_index} has inconsistent dynamic LUT flag"
                ));
            }
            validate_weather_i32_optional(
                row_index,
                temp_present,
                temperature_raw,
                -WEATHER_MAX_TEMPERATURE_RAW,
                WEATHER_MAX_TEMPERATURE_RAW,
                "temperature_raw",
            )?;
            validate_weather_i32_optional(
                row_index,
                temp_change_present,
                temperature_change_raw,
                -WEATHER_MAX_TEMP_CHANGE_RAW,
                WEATHER_MAX_TEMP_CHANGE_RAW,
                "temperature_change_raw",
            )?;
            validate_weather_i32_optional(
                row_index,
                water_present,
                water_raw,
                0,
                WEATHER_MAX_PRECIP_RAW,
                "water_raw",
            )?;
            validate_weather_i32_optional(
                row_index,
                snow_present,
                snow_raw,
                0,
                WEATHER_MAX_PRECIP_RAW,
                "snow_raw",
            )?;
            if custom_present {
                if custom_modifier_count > WEATHER_MAX_ACTIVE_MODIFIERS {
                    return Err(format!(
                        "weather province row {row_index} has invalid custom_modifier_count {custom_modifier_count}"
                    ));
                }
            } else if custom_modifier_count != 0 {
                return Err(format!(
                    "weather province row {row_index} has custom_modifier_count without presence flag"
                ));
            }
            if row_reserved != 0 {
                return Err(format!(
                    "weather province row {row_index} reserved field is {row_reserved}"
                ));
            }
            provinces.push(WeatherProvinceState {
                province_id,
                flags,
                temperature_raw,
                temperature_change_raw,
                water_raw,
                snow_raw,
                custom_modifier_count,
                scale_raw,
            });
        }

        if !delta {
            self.weather_live = Some(WeatherLiveState::default());
            self.delta_baselines_seen.insert(WEATHER_DOMAIN_ID);
        }
        let weather = self
            .weather_live
            .get_or_insert_with(WeatherLiveState::default);
        weather.active_region_count = active_region_count as u32;
        weather.observed_region_rows = observed_region_rows as u32;
        weather.observed_province_rows = observed_province_rows as u32;
        weather.scale_raw = scale_raw;
        for row in regions {
            weather.regions.insert(row.region_id, row);
        }
        for row in provinces {
            weather.provinces.insert(row.province_id, row);
        }
        self.mark_fold_domain_success(WEATHER_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 26 division_equipment (baseline + delta) ------------------
    fn fold_division_equipment(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            DIVISION_EQUIPMENT_HEADER_LEN,
            "division_equipment header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let changed_rows = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 16);
        let scale_raw = read_u32(payload, 20);
        let root_rows = read_u32(payload, 24) as usize;
        let observed_division_rows = read_u32(payload, 28) as usize;
        let equipment_entry_count = read_u32(payload, 32) as usize;
        let observed_equipment_rows = read_u32(payload, 36) as usize;
        let failure_flags_after_tolerance = compact_failure_flags_after_expected_partial_tolerance(
            DIVISION_EQUIPMENT_DOMAIN_ID,
            status_flags,
        );
        let tolerance_applies =
            compact_expected_partial_tolerance_applies(DIVISION_EQUIPMENT_DOMAIN_ID, status_flags);
        let sparse_partial = row_count > 0
            && (failure_flags_after_tolerance == COMPACT_STATUS_PARTIAL_DATA || tolerance_applies);
        if row_count == 0 && tolerance_applies {
            self.mark_fold_domain_unknown(
                DIVISION_EQUIPMENT_DOMAIN_ID,
                "partial_data_rowless",
                "division_equipment rowless partial data; equipment unknown this hour",
            );
            return Ok(());
        }
        if sparse_partial {
            self.record_domain_problem(DIVISION_EQUIPMENT_DOMAIN_ID, "partial_data");
            self.note(
                "division_equipment partial data; sparse rows applied and omitted rows retained",
            );
        } else if self.handle_compact_failure(
            DIVISION_EQUIPMENT_DOMAIN_ID,
            "division_equipment",
            status_flags,
        ) {
            return Ok(());
        }
        if !sparse_partial
            && self.reject_delta_before_baseline(
                DIVISION_EQUIPMENT_DOMAIN_ID,
                "division_equipment",
                status_flags,
            )
        {
            return Ok(());
        }
        ensure_fixed_scale("division_equipment", "scale_raw", scale_raw)?;
        let expected_len = DIVISION_EQUIPMENT_HEADER_LEN
            .saturating_add(row_count.saturating_mul(DIVISION_EQUIPMENT_ROOT_ROW_LEN))
            .saturating_add(equipment_entry_count.saturating_mul(DIVISION_EQUIPMENT_ENTRY_ROW_LEN));
        if payload.len() != expected_len {
            return Err(format!(
                "division_equipment len {} != {expected_len}",
                payload.len()
            ));
        }
        if !sparse_partial && row_count != root_rows {
            return Err(format!(
                "division_equipment row_count {row_count} != root_rows {root_rows}"
            ));
        }
        if changed_rows > row_count {
            return Err(format!(
                "division_equipment changed_rows {changed_rows} > row_count {row_count}"
            ));
        }
        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        if delta && changed_rows != row_count {
            return Err(format!(
                "division_equipment delta changed_rows {changed_rows} != row_count {row_count}"
            ));
        }
        if observed_division_rows > root_rows {
            return Err(format!(
                "division_equipment observed_division_rows {observed_division_rows} > root_rows {root_rows}"
            ));
        }
        if !delta && !sparse_partial && root_rows != observed_division_rows {
            return Err(format!(
                "division_equipment full root_rows {root_rows} != observed_division_rows {observed_division_rows}"
            ));
        }
        if !delta && !sparse_partial && equipment_entry_count != observed_equipment_rows {
            return Err(format!(
                "division_equipment full equipment_entry_count {equipment_entry_count} != observed_equipment_rows {observed_equipment_rows}"
            ));
        }
        if equipment_entry_count > observed_equipment_rows {
            return Err(format!(
                "division_equipment emitted entries {equipment_entry_count} > observed_equipment_rows {observed_equipment_rows}"
            ));
        }

        let mut roots = Vec::with_capacity(row_count);
        let mut root_key_counts: BTreeMap<DivisionEquipmentKey, usize> = BTreeMap::new();
        let mut declared_entries = 0usize;
        let mut cursor = DIVISION_EQUIPMENT_HEADER_LEN;
        for row_index in 0..row_count {
            let owner_tag_db_id = read_u32(payload, cursor);
            let div_id = read_u32(payload, cursor + 4);
            let flags = read_u32(payload, cursor + 8);
            let template_id = read_u32(payload, cursor + 12);
            let entry_first = read_u32(payload, cursor + 16) as usize;
            let entry_count = read_u32(payload, cursor + 20) as usize;
            cursor += DIVISION_EQUIPMENT_ROOT_ROW_LEN;
            if owner_tag_db_id == 0 {
                return Err(format!(
                    "division_equipment row {row_index} has zero owner_tag_db_id"
                ));
            }
            if div_id == 0 {
                return Err(format!(
                    "division_equipment row {row_index} has invalid div_id {div_id}"
                ));
            }
            let key = DivisionEquipmentKey {
                owner_tag_db_id,
                div_id,
            };
            *root_key_counts.entry(key).or_default() += 1;
            if flags & !DIVISION_EQUIPMENT_ROOT_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "division_equipment row {row_index} has unknown flags {flags}"
                ));
            }
            let deleted = flags & DIVISION_EQUIPMENT_ROOT_FLAG_DELETED != 0;
            let has_template = flags & DIVISION_EQUIPMENT_ROOT_FLAG_TEMPLATE_PRESENT != 0;
            if deleted {
                if flags != DIVISION_EQUIPMENT_ROOT_FLAG_DELETED
                    || template_id != 0
                    || entry_count != 0
                    || entry_first > equipment_entry_count
                {
                    return Err(format!(
                        "division_equipment delete row {row_index} has non-delete payload"
                    ));
                }
            } else {
                if !has_template && template_id != 0 {
                    return Err(format!(
                        "division_equipment row {row_index} has template_id without flag"
                    ));
                }
                if entry_first.saturating_add(entry_count) > equipment_entry_count {
                    return Err(format!(
                        "division_equipment row {row_index} entry range {entry_first}+{entry_count} exceeds {equipment_entry_count}"
                    ));
                }
            }
            declared_entries = declared_entries.saturating_add(entry_count);
            roots.push(DivisionEquipmentRootRow {
                key,
                entry_first,
                entry_count,
                deleted,
            });
        }
        if declared_entries != equipment_entry_count {
            return Err(format!(
                "division_equipment entry_count {equipment_entry_count} != declared {declared_entries}"
            ));
        }

        let mut entries = Vec::with_capacity(equipment_entry_count);
        for entry_index in 0..equipment_entry_count {
            let variant_id = read_u32(payload, cursor);
            let reserved = read_u32(payload, cursor + 4);
            let amount_raw = read_i64(payload, cursor + 8);
            cursor += DIVISION_EQUIPMENT_ENTRY_ROW_LEN;
            if variant_id == 0 || variant_id > i32::MAX as u32 {
                return Err(format!(
                    "division_equipment entry row {entry_index} has invalid variant_id {variant_id}"
                ));
            }
            if amount_raw <= 0 || amount_raw > 1_000_000_000_000_000 {
                return Err(format!(
                    "division_equipment entry row {entry_index} has invalid amount_raw {amount_raw}"
                ));
            }
            if reserved != 0 {
                return Err(format!(
                    "division_equipment entry row {entry_index} reserved field is nonzero"
                ));
            }
            entries.push((variant_id, amount_raw));
        }

        if !delta && !sparse_partial {
            self.division_equipment.clear();
            self.delta_baselines_seen
                .insert(DIVISION_EQUIPMENT_DOMAIN_ID);
        } else if sparse_partial {
            self.delta_baselines_seen
                .insert(DIVISION_EQUIPMENT_DOMAIN_ID);
        }
        let duplicate_keys = root_key_counts
            .iter()
            .filter_map(|(key, count)| (*count > 1).then_some(*key))
            .collect::<BTreeSet<_>>();
        let duplicate_rows = duplicate_keys
            .iter()
            .filter_map(|key| root_key_counts.get(key))
            .copied()
            .sum::<usize>();
        for key in &duplicate_keys {
            self.division_equipment_touched_this_hour.insert(*key);
            self.division_equipment.remove(key);
        }
        if duplicate_rows > 0 {
            self.note(format!(
                "division_equipment_live dropped {duplicate_rows} duplicate-key row(s); prior equipment for {key_count} ambiguous key(s) cleared",
                key_count = duplicate_keys.len()
            ));
        }
        let mut seen_entries = BTreeSet::new();
        for root in roots {
            if duplicate_keys.contains(&root.key) {
                continue;
            }
            self.division_equipment_touched_this_hour.insert(root.key);
            if root.deleted {
                self.division_equipment.remove(&root.key);
                continue;
            }
            let end = root.entry_first + root.entry_count;
            let equipment: Vec<DivisionEquipmentEntryState> = entries[root.entry_first..end]
                .iter()
                .map(|(variant_id, amount_raw)| {
                    if !seen_entries.insert((root.key.owner_tag_db_id, root.key.div_id, *variant_id))
                    {
                        return Err(format!(
                            "division_equipment row owner={} div_id={} duplicates variant {variant_id}",
                            root.key.owner_tag_db_id, root.key.div_id
                        ));
                    }
                    Ok(DivisionEquipmentEntryState {
                        variant_id: *variant_id,
                        amount_raw: *amount_raw,
                        scale_raw,
                    })
                })
                .collect::<Result<_, String>>()?;
            self.division_equipment.insert(root.key, equipment);
        }
        self.mark_fold_domain_success(DIVISION_EQUIPMENT_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 41 special_projects (full replace) -----------------------
    fn fold_special_projects(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            SPECIAL_PROJECTS_HEADER_BASE_LEN,
            "special_projects header",
        )?;
        let status_flags = read_u32(payload, 16);
        let delta = status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        let facility_count = read_u32(payload, 20) as usize;
        let project_count = read_u32(payload, 24) as usize;
        let scientist_count = read_u32(payload, 28) as usize;
        let active_facility_count = read_u32(payload, 32) as usize;
        let active_project_count = read_u32(payload, 36) as usize;
        let active_scientist_count = read_u32(payload, 40) as usize;
        let source_kind = read_u32(payload, 48);
        let header_extension = read_u32(payload, 52);
        let mut header_len = SPECIAL_PROJECTS_HEADER_BASE_LEN;
        if header_extension >= 1 {
            header_len = header_len.saturating_add(SPECIAL_PROJECTS_HEADER_ROOT_COUNTERS_LEN);
        }
        if header_extension >= 2 {
            header_len = header_len.saturating_add(SPECIAL_PROJECTS_HEADER_REASON_COUNTERS_LEN);
        }
        let mut project_state_count = 0usize;
        let mut breakthrough_count = 0usize;
        if header_extension >= 3 {
            need(
                payload,
                header_len + SPECIAL_PROJECTS_HEADER_PROGRESS_LEN,
                "special_projects progress extension",
            )?;
            project_state_count = read_u32(payload, header_len) as usize;
            breakthrough_count = read_u32(payload, header_len + 4) as usize;
            header_len = header_len.saturating_add(SPECIAL_PROJECTS_HEADER_PROGRESS_LEN);
        }
        if self.handle_compact_failure(SPECIAL_PROJECTS_DOMAIN_ID, "special_projects", status_flags)
        {
            return Ok(());
        }
        if compact_expected_partial_tolerance_applies(SPECIAL_PROJECTS_DOMAIN_ID, status_flags) {
            // D41 partial conditions are not row-complete:
            // - read_compact_projects marks partial for invalid program/facility/project-pool
            //   roots, so facilities/projects and derived project_states may omit live rows.
            // - scientist/project_state row read shortfalls also mark partial after pinning.
            // - breakthrough progress partials can omit rows and D41 has no tombstones.
            // Without deletion evidence, merging would fabricate persistence; replacing would
            // fabricate disappearance. Keep the D41 leaf unknown for this hour.
            self.fold_domains_unknown_this_hour
                .insert(SPECIAL_PROJECTS_DOMAIN_ID);
            self.note(
                "special_projects state unknown this hour; expected partial can omit current rows"
                    .to_string(),
            );
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            SPECIAL_PROJECTS_DOMAIN_ID,
            "special_projects",
            status_flags,
        ) {
            return Ok(());
        }
        if !delta
            && (facility_count > active_facility_count
                || project_count > active_project_count
                || scientist_count > active_scientist_count)
        {
            return Err("special_projects emitted row count exceeds active count".to_string());
        }

        let facilities_start = header_len;
        let projects_start = facilities_start
            .saturating_add(facility_count.saturating_mul(SPECIAL_PROJECTS_FACILITY_ROW_LEN));
        let scientists_start = projects_start
            .saturating_add(project_count.saturating_mul(SPECIAL_PROJECTS_PROJECT_ROW_LEN));
        let project_states_start = scientists_start
            .saturating_add(scientist_count.saturating_mul(SPECIAL_PROJECTS_SCIENTIST_ROW_LEN));
        let breakthroughs_start = project_states_start.saturating_add(
            project_state_count.saturating_mul(SPECIAL_PROJECTS_PROJECT_STATE_ROW_LEN),
        );
        let string_start = breakthroughs_start.saturating_add(
            breakthrough_count.saturating_mul(SPECIAL_PROJECTS_BREAKTHROUGH_ROW_LEN),
        );
        need(payload, string_start, "special_projects rows")?;

        let mut facilities = if delta {
            self.special_projects
                .as_ref()
                .map(|state| state.facilities.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut seen_facility_rows = BTreeSet::new();
        let mut facility_string_targets = Vec::with_capacity(facility_count);
        for index in 0..facility_count {
            let base = facilities_start + index * SPECIAL_PROJECTS_FACILITY_ROW_LEN;
            let id = read_u32(payload, base);
            let province = read_u32(payload, base + 4);
            let flags = read_u32(payload, base + 8);
            let active_project_handle = read_u32(payload, base + 12);
            let scientist_char_id = read_u32(payload, base + 16);
            let unread_rewards = read_u32(payload, base + 20);
            let reserved0 = read_u32(payload, base + 24);
            let reserved1 = read_u32(payload, base + 28);
            if id == 0 || !seen_facility_rows.insert(id) {
                return Err(format!("special_projects duplicate/zero facility id {id}"));
            }
            if flags & !SPECIAL_PROJECTS_FACILITY_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "special_projects facility row {index} unknown flags {flags}"
                ));
            }
            let deleted = flags & SPECIAL_PROJECTS_FACILITY_DELETED != 0;
            if deleted {
                if flags != SPECIAL_PROJECTS_FACILITY_DELETED {
                    return Err(format!(
                        "special_projects facility deleted row {index} has live flags"
                    ));
                }
                if province != 0
                    || active_project_handle != 0
                    || scientist_char_id != 0
                    || unread_rewards != 0
                    || reserved0 != 0
                    || reserved1 != 0
                {
                    return Err(format!(
                        "special_projects facility deleted row {index} has live fields"
                    ));
                }
                facilities.remove(&id);
                facility_string_targets.push((id, flags, true));
                continue;
            }
            if flags & SPECIAL_PROJECTS_FACILITY_ACTIVE_PROJECT == 0 && active_project_handle != 0 {
                return Err(format!(
                    "special_projects facility row {index} active_project handle without flag"
                ));
            }
            if flags & SPECIAL_PROJECTS_FACILITY_SCIENTIST == 0 && scientist_char_id != 0 {
                return Err(format!(
                    "special_projects facility row {index} scientist id without flag"
                ));
            }
            if reserved0 != 0 || reserved1 != 0 {
                return Err(format!(
                    "special_projects facility row {index} reserved fields are nonzero"
                ));
            }
            facilities.insert(
                id,
                SpecialProjectFacilityState {
                    id,
                    province,
                    active_project_handle: (flags & SPECIAL_PROJECTS_FACILITY_ACTIVE_PROJECT != 0)
                        .then_some(active_project_handle),
                    scientist_char_id: (flags & SPECIAL_PROJECTS_FACILITY_SCIENTIST != 0)
                        .then_some(scientist_char_id),
                    unread_rewards,
                    building: None,
                },
            );
            facility_string_targets.push((id, flags, false));
        }

        let mut projects = if delta {
            self.special_projects
                .as_ref()
                .map(|state| state.projects.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut seen_project_rows = BTreeSet::new();
        for index in 0..project_count {
            let base = projects_start + index * SPECIAL_PROJECTS_PROJECT_ROW_LEN;
            let id = read_u32(payload, base);
            let static_def_handle = read_u32(payload, base + 4);
            let iterations = read_u32(payload, base + 8);
            let worked_at_facility = read_u32(payload, base + 12);
            let flags = read_u32(payload, base + 16);
            let reserved = read_u32(payload, base + 20);
            if id == 0 || !seen_project_rows.insert(id) {
                return Err(format!("special_projects duplicate/zero project id {id}"));
            }
            if flags
                & !(SPECIAL_PROJECTS_PROJECT_WORKED_FACILITY | SPECIAL_PROJECTS_PROJECT_DELETED)
                != 0
            {
                return Err(format!(
                    "special_projects project row {index} unknown flags {flags}"
                ));
            }
            let deleted = flags & SPECIAL_PROJECTS_PROJECT_DELETED != 0;
            if deleted {
                if flags != SPECIAL_PROJECTS_PROJECT_DELETED {
                    return Err(format!(
                        "special_projects project deleted row {index} has live flags"
                    ));
                }
                if static_def_handle != 0
                    || iterations != 0
                    || worked_at_facility != 0
                    || reserved != 0
                {
                    return Err(format!(
                        "special_projects project deleted row {index} has live fields"
                    ));
                }
                projects.remove(&id);
                continue;
            }
            if flags & SPECIAL_PROJECTS_PROJECT_WORKED_FACILITY == 0 && worked_at_facility != 0 {
                return Err(format!(
                    "special_projects project row {index} worked_at_facility without flag"
                ));
            }
            if reserved != 0 {
                return Err(format!(
                    "special_projects project row {index} reserved field is nonzero"
                ));
            }
            projects.insert(
                id,
                SpecialProjectState {
                    id,
                    static_def_handle,
                    iterations,
                    worked_at_facility: (flags & SPECIAL_PROJECTS_PROJECT_WORKED_FACILITY != 0)
                        .then_some(worked_at_facility),
                },
            );
        }

        let mut scientists = if delta {
            self.special_projects
                .as_ref()
                .map(|state| state.scientists.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut seen_scientist_rows = BTreeSet::new();
        let mut scientist_string_targets = Vec::with_capacity(scientist_count);
        for index in 0..scientist_count {
            let base = scientists_start + index * SPECIAL_PROJECTS_SCIENTIST_ROW_LEN;
            let char_id = read_u32(payload, base);
            let flags = read_u32(payload, base + 4);
            let trait_count = read_u32(payload, base + 8);
            let skill_values = [
                read_i32(payload, base + 12),
                read_i32(payload, base + 16),
                read_i32(payload, base + 20),
                read_i32(payload, base + 24),
            ];
            let reserved = read_u32(payload, base + 28);
            if char_id == 0 || !seen_scientist_rows.insert(char_id) {
                return Err(format!(
                    "special_projects duplicate/zero scientist id {char_id}"
                ));
            }
            if flags & !SPECIAL_PROJECTS_SCIENTIST_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "special_projects scientist row {index} unknown flags {flags}"
                ));
            }
            let deleted = flags & SPECIAL_PROJECTS_SCIENTIST_DELETED != 0;
            if deleted {
                if flags != SPECIAL_PROJECTS_SCIENTIST_DELETED {
                    return Err(format!(
                        "special_projects scientist deleted row {index} has live flags"
                    ));
                }
                if trait_count != 0 || skill_values != [0, 0, 0, 0] || reserved != 0 {
                    return Err(format!(
                        "special_projects scientist deleted row {index} has live fields"
                    ));
                }
                scientists.remove(&char_id);
                scientist_string_targets.push((char_id, flags, true));
                continue;
            }
            if flags & SPECIAL_PROJECTS_SCIENTIST_TRAIT_COUNT == 0 && trait_count != 0 {
                return Err(format!(
                    "special_projects scientist row {index} trait_count without flag"
                ));
            }
            if flags & SPECIAL_PROJECTS_SCIENTIST_SKILLS == 0 && skill_values != [0, 0, 0, 0] {
                return Err(format!(
                    "special_projects scientist row {index} skill values without flag"
                ));
            }
            if reserved != 0 {
                return Err(format!(
                    "special_projects scientist row {index} reserved field is nonzero"
                ));
            }
            scientists.insert(
                char_id,
                SpecialProjectScientistState {
                    char_id,
                    trait_count: (flags & SPECIAL_PROJECTS_SCIENTIST_TRAIT_COUNT != 0)
                        .then_some(trait_count),
                    skill_values: (flags & SPECIAL_PROJECTS_SCIENTIST_SKILLS != 0)
                        .then_some(skill_values),
                    name_token: None,
                    display_name: None,
                },
            );
            scientist_string_targets.push((char_id, flags, false));
        }

        let mut project_states = if delta {
            self.special_projects
                .as_ref()
                .map(|state| state.project_states.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut seen_project_state_rows = BTreeSet::new();
        for index in 0..project_state_count {
            let base = project_states_start + index * SPECIAL_PROJECTS_PROJECT_STATE_ROW_LEN;
            let id = read_u32(payload, base);
            let flags = read_u32(payload, base + 44);
            if id == 0 || !seen_project_state_rows.insert(id) {
                return Err(format!(
                    "special_projects duplicate/zero project state id {id}"
                ));
            }
            if flags
                & !(SPECIAL_PROJECTS_PROJECT_STATE_COMPLETED
                    | SPECIAL_PROJECTS_PROJECT_STATE_DELETED)
                != 0
            {
                return Err(format!(
                    "special_projects project state row {index} unknown flags {flags}"
                ));
            }
            let deleted = flags & SPECIAL_PROJECTS_PROJECT_STATE_DELETED != 0;
            if deleted {
                if flags != SPECIAL_PROJECTS_PROJECT_STATE_DELETED {
                    return Err(format!(
                        "special_projects project state deleted row {index} has live flags"
                    ));
                }
                let live_fields = read_u32(payload, base + 4) != 0
                    || read_u32(payload, base + 8) != 0
                    || read_u32(payload, base + 12) != 0
                    || read_u32(payload, base + 16) != 0
                    || read_u32(payload, base + 20) != 0
                    || read_u32(payload, base + 24) != 0
                    || read_u32(payload, base + 28) != 0
                    || read_u32(payload, base + 32) != 0
                    || read_u32(payload, base + 36) != 0
                    || read_u32(payload, base + 40) != 0;
                if live_fields {
                    return Err(format!(
                        "special_projects project state deleted row {index} has live fields"
                    ));
                }
                project_states.remove(&id);
                continue;
            }
            project_states.insert(
                id,
                SpecialProjectProgressState {
                    id,
                    static_def_handle: read_u32(payload, base + 4),
                    current_state_offset: read_u32(payload, base + 8),
                    current_state_enum: read_u32(payload, base + 12),
                    prototype_state_enum: read_u32(payload, base + 16),
                    simple_state_1_enum: read_u32(payload, base + 20),
                    simple_state_2_enum: read_u32(payload, base + 24),
                    simple_state_3_enum: read_u32(payload, base + 28),
                    prototype_iterations: read_u32(payload, base + 32),
                    phase_progress_raw: read_u32(payload, base + 36),
                    phase_target_raw: read_u32(payload, base + 40),
                    completed: (header_extension >= 4)
                        .then_some(flags & SPECIAL_PROJECTS_PROJECT_STATE_COMPLETED != 0),
                },
            );
        }

        let mut breakthroughs = if delta {
            self.special_projects
                .as_ref()
                .map(|state| state.breakthroughs.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut seen_breakthrough_rows = BTreeSet::new();
        for index in 0..breakthrough_count {
            let base = breakthroughs_start + index * SPECIAL_PROJECTS_BREAKTHROUGH_ROW_LEN;
            let tag_db_id = read_u32(payload, base);
            let specialization_handle = read_u32(payload, base + 4);
            let points_raw = read_u32(payload, base + 8);
            let flags = read_u32(payload, base + 12);
            if tag_db_id == 0
                || specialization_handle == 0
                || !seen_breakthrough_rows.insert((tag_db_id, specialization_handle))
            {
                return Err(format!(
                    "special_projects duplicate/zero breakthrough key {tag_db_id}/{specialization_handle}"
                ));
            }
            if flags & !SPECIAL_PROJECTS_BREAKTHROUGH_DELETED != 0 {
                return Err(format!(
                    "special_projects breakthrough row {index} unknown flags {flags:#x}"
                ));
            }
            let deleted = flags & SPECIAL_PROJECTS_BREAKTHROUGH_DELETED != 0;
            if deleted {
                if points_raw != 0 {
                    return Err(format!(
                        "special_projects breakthrough deleted row {index} has live fields"
                    ));
                }
                breakthroughs.remove(&(tag_db_id, specialization_handle));
                continue;
            }
            breakthroughs.insert(
                (tag_db_id, specialization_handle),
                SpecialProjectBreakthroughState {
                    tag_db_id,
                    specialization_handle,
                    points_raw,
                },
            );
        }

        let mut cursor = string_start;
        for (facility_id, flags, deleted) in facility_string_targets {
            let building = read_string(payload, &mut cursor, "special_projects facility building")?;
            if deleted {
                if !building.is_empty() {
                    return Err(
                        "special_projects facility deleted row has live building string"
                            .to_string(),
                    );
                }
                continue;
            }
            if building.is_empty() && flags & SPECIAL_PROJECTS_FACILITY_BUILDING != 0 {
                return Err(
                    "special_projects facility row has building flag with empty string".to_string(),
                );
            }
            if !building.is_empty() && flags & SPECIAL_PROJECTS_FACILITY_BUILDING == 0 {
                return Err(
                    "special_projects facility row has building string without flag".to_string(),
                );
            }
            if !building.is_empty() {
                if let Some(facility) = facilities.get_mut(&facility_id) {
                    facility.building = Some(building);
                }
            }
        }
        for (scientist_id, flags, deleted) in scientist_string_targets {
            let name_token = read_string(payload, &mut cursor, "special_projects scientist token")?;
            let display_name =
                read_string(payload, &mut cursor, "special_projects scientist display")?;
            if deleted {
                if !name_token.is_empty() || !display_name.is_empty() {
                    return Err(
                        "special_projects scientist deleted row has live string fields".to_string(),
                    );
                }
                continue;
            }
            if name_token.is_empty() && flags & SPECIAL_PROJECTS_SCIENTIST_NAME_TOKEN != 0 {
                return Err(
                    "special_projects scientist row has token flag with empty string".to_string(),
                );
            }
            if !name_token.is_empty() && flags & SPECIAL_PROJECTS_SCIENTIST_NAME_TOKEN == 0 {
                return Err(
                    "special_projects scientist row has token string without flag".to_string(),
                );
            }
            if display_name.is_empty() && flags & SPECIAL_PROJECTS_SCIENTIST_DISPLAY_NAME != 0 {
                return Err(
                    "special_projects scientist row has display flag with empty string".to_string(),
                );
            }
            if !display_name.is_empty() && flags & SPECIAL_PROJECTS_SCIENTIST_DISPLAY_NAME == 0 {
                return Err(
                    "special_projects scientist row has display string without flag".to_string(),
                );
            }
            if let Some(scientist) = scientists.get_mut(&scientist_id) {
                if !name_token.is_empty() {
                    scientist.name_token = Some(name_token);
                }
                if !display_name.is_empty() {
                    scientist.display_name = Some(display_name);
                }
            }
        }
        // Wave-5 project lifecycle: any bytes after the self-delimiting base
        // payload are the `PLCF` lifecycle trailer; anything else is a
        // trailing-byte error exactly as before.
        let previous_lifecycle = if delta {
            self.special_projects
                .as_ref()
                .and_then(|state| state.lifecycle.clone())
        } else {
            None
        };
        let lifecycle = if cursor != payload.len() {
            let trailer = parse_project_lifecycle_trailer(payload, cursor)?;
            if delta {
                apply_project_lifecycle_delta(previous_lifecycle, trailer)?
            } else {
                if let Some(trailer) = trailer.as_ref() {
                    if trailer.rows.iter().any(|row| row.deleted) {
                        return Err(
                            "special_projects lifecycle baseline carries deleted rows".to_string()
                        );
                    }
                }
                trailer
            }
        } else if delta {
            previous_lifecycle
        } else {
            // No trailer this frame: lifecycle not measured (absent, never
            // "all-empty").
            None
        };

        let source = match source_kind {
            1 => "scan".to_string(),
            2 => "bridge".to_string(),
            other => format!("unknown:{other}"),
        };
        self.special_projects = Some(SpecialProjectsState {
            facilities,
            projects,
            scientists,
            project_states,
            breakthroughs,
            count: active_project_count as u32,
            source,
            header_extension,
            lifecycle,
        });
        if !delta {
            self.delta_baselines_seen.insert(SPECIAL_PROJECTS_DOMAIN_ID);
        }
        self.mark_fold_domain_success(SPECIAL_PROJECTS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 45 focus/research/decisions depth ------------------------
    fn fold_focus_research_decisions_depth(
        &mut self,
        payload: &[u8],
        record_flags: u32,
    ) -> Result<(), String> {
        let parsed = match parse_frd_depth_payload(payload, record_flags) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.record_domain_problem(
                    FOCUS_RESEARCH_DECISIONS_DEPTH_DOMAIN_ID,
                    "malformed_payload",
                );
                return Err(err);
            }
        };

        if parsed.depth_status_flags == 0 {
            self.note(
                "focus_research_decisions_depth depth_status_flags was zero; folded rows remain row-derived only".to_string(),
            );
        }
        if parsed.decision_target_omitted_rows > 0 {
            self.record_domain_problem(FOCUS_RESEARCH_DECISIONS_DEPTH_DOMAIN_ID, "partial_data");
            self.note(format!(
                "focus_research_decisions_depth kept valid decision rows but omitted {} unreadable targeted-timed country target row(s)",
                parsed.decision_target_omitted_rows
            ));
        }

        for row in parsed.focus_rows {
            let metrics = self.frd_focus_metrics.entry(row.tag_db_id).or_default();
            if row.kind == FRD_FOCUS_COMPLETED_COUNT_INCREASED && row.current_completed != u32::MAX
            {
                metrics.completed_count = Some(row.current_completed);
            }
            if row.kind == FRD_FOCUS_COMPLETED_COUNT_INCREASED && !row.completed_token.is_empty() {
                push_unique_string(&mut metrics.completed_tokens, row.completed_token.clone());
            }
            if matches!(row.kind, FRD_FOCUS_CURRENT_STARTED | FRD_FOCUS_CHANGED)
                && !row.to_token.is_empty()
            {
                if row.focus_kind == FRD_FOCUS_CONTINUOUS_KIND {
                    metrics.current_continuous = Some(row.to_token.clone());
                } else {
                    metrics.current = Some(row.to_token.clone());
                }
            }
            if row.kind == FRD_FOCUS_STOPPED {
                if row.focus_kind == FRD_FOCUS_CONTINUOUS_KIND {
                    metrics.current_continuous = None;
                } else {
                    metrics.current = None;
                }
            }
            self.frd_focus_events_this_hour
                .entry(row.tag_db_id)
                .or_default()
                .push(row);
        }

        for row in parsed.research_rows {
            self.frd_research_live.insert(
                FrdResearchKey {
                    tag_db_id: row.tag_db_id,
                    slot_index: row.slot_index,
                },
                row,
            );
        }
        let mut decision_target_rows_by_tag = BTreeMap::<u32, u32>::new();
        for row in &parsed.decision_target_rows {
            let count = decision_target_rows_by_tag
                .entry(row.tag_db_id)
                .or_default();
            *count = count.saturating_add(1);
        }
        let mut observed_decision_status_tags = BTreeSet::new();
        let mut omitted_decision_target_tags = BTreeSet::new();
        for row in parsed.decision_status_rows {
            if parsed.decision_target_omitted_rows > 0
                && row.targeted_timed_count
                    > decision_target_rows_by_tag
                        .get(&row.tag_db_id)
                        .copied()
                        .unwrap_or(0)
            {
                omitted_decision_target_tags.insert(row.tag_db_id);
            }
            observed_decision_status_tags.insert(row.tag_db_id);
            self.frd_decision_status.insert(row.tag_db_id, row);
        }
        for tag_db_id in observed_decision_status_tags {
            self.frd_decision_targets.remove(&tag_db_id);
            self.frd_decision_targets_unknown.remove(&tag_db_id);
            if omitted_decision_target_tags.contains(&tag_db_id) {
                self.frd_decision_targets_unknown.insert(tag_db_id);
            }
        }
        for row in parsed.decision_target_rows {
            if omitted_decision_target_tags.contains(&row.tag_db_id) {
                continue;
            }
            self.frd_decision_targets
                .entry(row.tag_db_id)
                .or_default()
                .push(row);
        }
        for row in parsed.decision_event_rows {
            self.frd_decision_events_this_hour
                .entry(row.tag_db_id)
                .or_default()
                .push(row);
        }
        self.frd_depth_badges_this_hour = parsed.badges;
        self.mark_fold_domain_success(FOCUS_RESEARCH_DECISIONS_DEPTH_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 51 scripted state flags/variables (baseline + delta) ------
    fn fold_scripted_state(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, SCRIPTED_STATE_HEADER_LEN, "scripted_state header")?;
        let row_count = read_u32(payload, 8) as usize;
        let key_bytes_len = read_u32(payload, 12) as usize;
        let status_flags = read_u32(payload, 20);
        let fixed_scale = read_u32(payload, 48);
        let schema_version = read_u32(payload, 52);
        if schema_version != SCRIPTED_STATE_SCHEMA_VERSION {
            return Err(format!(
                "scripted_state schema version {schema_version} != {SCRIPTED_STATE_SCHEMA_VERSION}"
            ));
        }
        let is_delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        let hard_failure = (status_flags
            & (COMPACT_STATUS_FAILURE_MASK & !SCRIPTED_STATE_OPAQUE_STATUS_MASK))
            != 0
            || (status_flags & (COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA))
                != 0;
        if hard_failure {
            self.mark_fold_domain_unknown(
                SCRIPTED_STATE_DOMAIN_ID,
                "compact_status_failure",
                "scripted_state partial/source failure; owned snapshot leaves unknown this hour",
            );
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            SCRIPTED_STATE_DOMAIN_ID,
            "scripted_state",
            status_flags,
        ) {
            return Ok(());
        }

        let rows_len = row_count.saturating_mul(SCRIPTED_STATE_ROW_LEN);
        let key_start = SCRIPTED_STATE_HEADER_LEN.saturating_add(rows_len);
        let expected_len = key_start.saturating_add(key_bytes_len);
        if payload.len() != expected_len {
            return Err(format!(
                "scripted_state len {} != {expected_len}",
                payload.len()
            ));
        }
        ensure_fixed_scale("scripted_state", "fixed_scale", fixed_scale)?;

        let key_table = &payload[key_start..expected_len];
        let mut rows = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let base = SCRIPTED_STATE_HEADER_LEN + index * SCRIPTED_STATE_ROW_LEN;
            let scope_kind = read_u32(payload, base);
            let owner_id = read_u32(payload, base + 4);
            let raw_value_kind = read_u32(payload, base + 8);
            let value_kind = raw_value_kind & !SCRIPTED_STATE_VALUE_KIND_DELETED;
            let flags = read_u32(payload, base + 12);
            if flags & !SCRIPTED_STATE_ROW_KNOWN_FLAGS != 0 {
                return Err(format!("scripted_state row {index} unknown flags {flags}"));
            }
            let key = if flags & SCRIPTED_STATE_ROW_FLAG_KEY_PRESENT != 0 {
                let key_offset = read_u32(payload, base + 16) as usize;
                let key_len = read_u32(payload, base + 20) as usize;
                let Some(bytes) = key_table.get(key_offset..key_offset.saturating_add(key_len))
                else {
                    return Err(format!(
                        "scripted_state row {index} key range out of bounds"
                    ));
                };
                Some(
                    std::str::from_utf8(bytes)
                        .map_err(|e| format!("scripted_state row {index} key utf8: {e}"))?
                        .to_string(),
                )
            } else {
                if read_u32(payload, base + 20) != 0 {
                    return Err(format!(
                        "scripted_state row {index} has key_len without key flag"
                    ));
                }
                None
            };
            // Reserved slot 0 (base+40) carries the delayed-event due hours
            // when the due flag is set; otherwise it must be 0. Slot 1
            // (base+44) is still fully reserved and must be 0 (mirrors the
            // payload gate so the fold enforces the reserved contract even if
            // it ever ingests without the gate).
            let has_delayed_due = flags & SCRIPTED_STATE_ROW_FLAG_DELAYED_DUE_PRESENT != 0;
            let reserved0 = read_u32(payload, base + 40);
            let reserved1 = read_u32(payload, base + 44);
            let token_id_raw = read_u32(payload, base + 24);
            let date_hour_raw = read_i32(payload, base + 28);
            let value_raw = read_i64(payload, base + 32);
            if !has_delayed_due && reserved0 != 0 {
                return Err(format!(
                    "scripted_state row {index} has due hours without the due flag"
                ));
            }
            if reserved1 != 0 {
                return Err(format!(
                    "scripted_state row {index} reserved field is nonzero"
                ));
            }
            let deleted = raw_value_kind & SCRIPTED_STATE_VALUE_KIND_DELETED != 0
                || flags & SCRIPTED_STATE_ROW_FLAG_DELETED != 0;
            if deleted {
                let allowed_flags = match value_kind {
                    SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64 | SCRIPTED_STATE_VALUE_FLAG_NAMED => {
                        SCRIPTED_STATE_ROW_FLAG_KEY_PRESENT | SCRIPTED_STATE_ROW_FLAG_DELETED
                    }
                    SCRIPTED_STATE_VALUE_FLAG_TOKEN_ONLY => {
                        SCRIPTED_STATE_ROW_FLAG_TOKEN_PRESENT | SCRIPTED_STATE_ROW_FLAG_DELETED
                    }
                    _ => 0,
                };
                if flags & !allowed_flags != 0 {
                    return Err(format!(
                        "scripted_state tombstone row {index} has live flags"
                    ));
                }
                if matches!(
                    value_kind,
                    SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64 | SCRIPTED_STATE_VALUE_FLAG_NAMED
                ) && token_id_raw != 0
                {
                    return Err(format!(
                        "scripted_state tombstone row {index} has live token_id"
                    ));
                }
                if date_hour_raw != 0 || value_raw != 0 || reserved0 != 0 || reserved1 != 0 {
                    return Err(format!(
                        "scripted_state tombstone row {index} has live fields"
                    ));
                }
            }
            let row = ScriptedStateRow {
                scope_kind,
                owner_id,
                value_kind,
                key,
                token_id: (flags & SCRIPTED_STATE_ROW_FLAG_TOKEN_PRESENT != 0)
                    .then_some(token_id_raw),
                date_hour_raw: (flags & SCRIPTED_STATE_ROW_FLAG_DATE_PRESENT != 0)
                    .then_some(date_hour_raw),
                value_raw: (flags & SCRIPTED_STATE_ROW_FLAG_VALUE_PRESENT != 0)
                    .then_some(value_raw),
                deleted,
                delayed_due_hours: has_delayed_due.then_some(reserved0),
            };
            if raw_value_kind & SCRIPTED_STATE_VALUE_KIND_DELETED != 0 && !is_delta {
                return Err(format!(
                    "scripted_state row {index} tombstone appears outside delta frame"
                ));
            }
            validate_scripted_state_row(&row)?;
            rows.push(row);
        }

        if !is_delta {
            self.clear_scripted_state_accumulators();
            self.delta_baselines_seen.insert(SCRIPTED_STATE_DOMAIN_ID);
        } else {
            self.scripted_world_opaque_badges.clear();
            self.scripted_world_delayed_events.clear();
        }
        for row in rows {
            self.apply_scripted_state_row(&row, fixed_scale)?;
        }
        if status_flags & SCRIPTED_STATE_STATUS_FLAG_NAMES_PARTIAL != 0 {
            self.note("scripted_state folded with token-only flag names partial".to_string());
        }
        if status_flags & SCRIPTED_STATE_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED != 0 {
            self.note(
                "scripted_state delayed-event root unresolved; opaque badge rows preserved"
                    .to_string(),
            );
        }
        if status_flags & SCRIPTED_STATE_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED != 0 {
            self.note(
                "scripted_state scripted-GUI root unresolved; opaque badge rows preserved"
                    .to_string(),
            );
        }
        self.mark_fold_domain_success(SCRIPTED_STATE_DOMAIN_ID);
        Ok(())
    }

    fn clear_scripted_state_accumulators(&mut self) {
        self.scripted_world_flags.clear();
        self.scripted_world_variables.clear();
        self.scripted_world_token_only_flags.clear();
        self.scripted_world_opaque_badges.clear();
        self.scripted_world_delayed_events.clear();
        for country in self.countries.values_mut() {
            clear_scripted_country_extra(country);
        }
        for state in self.states.values_mut() {
            state.remove("_r2_scripted_state");
        }
    }

    fn apply_scripted_state_row(
        &mut self,
        row: &ScriptedStateRow,
        fixed_scale: u32,
    ) -> Result<(), String> {
        match row.value_kind {
            SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64 => {
                let key = row.key.as_ref().expect("validated scripted variable key");
                if row.deleted {
                    match row.scope_kind {
                        SCRIPTED_STATE_SCOPE_GLOBAL => {
                            self.scripted_world_variables.remove(key);
                        }
                        SCRIPTED_STATE_SCOPE_COUNTRY => {
                            clear_country_extra_key(
                                self.country_mut(row.owner_id),
                                "_r2_script_variables",
                                key,
                            );
                        }
                        SCRIPTED_STATE_SCOPE_STATE => {
                            clear_state_scripted_key(
                                self.states.entry(row.owner_id.to_string()).or_default(),
                                "variables",
                                key,
                            );
                        }
                        _ => unreachable!("validated scripted variable scope"),
                    }
                    return Ok(());
                }
                let scale = fixed_scale as f64;
                let value =
                    row.value_raw.expect("validated scripted variable value") as f64 / scale;
                match row.scope_kind {
                    SCRIPTED_STATE_SCOPE_GLOBAL => {
                        self.scripted_world_variables.insert(key.clone(), value);
                    }
                    SCRIPTED_STATE_SCOPE_COUNTRY => {
                        let extra = ensure_json_object_mut(self.country_mut(row.owner_id), "extra");
                        ensure_json_object_mut(extra, "_r2_script_variables")
                            .insert(key.clone(), json!(value));
                    }
                    SCRIPTED_STATE_SCOPE_STATE => {
                        let scope = ensure_json_object_mut(
                            self.states.entry(row.owner_id.to_string()).or_default(),
                            "_r2_scripted_state",
                        );
                        ensure_json_object_mut(scope, "variables")
                            .insert(key.clone(), json!(value));
                    }
                    _ => unreachable!("validated scripted variable scope"),
                }
            }
            SCRIPTED_STATE_VALUE_FLAG_NAMED => {
                let key = row.key.as_ref().expect("validated scripted flag key");
                if row.deleted {
                    match row.scope_kind {
                        SCRIPTED_STATE_SCOPE_GLOBAL => {
                            self.scripted_world_flags.remove(key);
                        }
                        SCRIPTED_STATE_SCOPE_COUNTRY => {
                            clear_country_extra_key(
                                self.country_mut(row.owner_id),
                                "script_flags",
                                key,
                            );
                        }
                        SCRIPTED_STATE_SCOPE_STATE => {
                            clear_state_scripted_key(
                                self.states.entry(row.owner_id.to_string()).or_default(),
                                "flags",
                                key,
                            );
                        }
                        _ => unreachable!("validated scripted flag scope"),
                    }
                    return Ok(());
                }
                let flag = ScriptFlagState {
                    value_raw: row.value_raw,
                    date_hour_raw: row.date_hour_raw,
                };
                match row.scope_kind {
                    SCRIPTED_STATE_SCOPE_GLOBAL => {
                        self.scripted_world_flags.insert(key.clone(), flag);
                    }
                    SCRIPTED_STATE_SCOPE_COUNTRY => {
                        let extra = ensure_json_object_mut(self.country_mut(row.owner_id), "extra");
                        ensure_json_object_mut(extra, "script_flags")
                            .insert(key.clone(), flag.to_json());
                    }
                    SCRIPTED_STATE_SCOPE_STATE => {
                        let scope = ensure_json_object_mut(
                            self.states.entry(row.owner_id.to_string()).or_default(),
                            "_r2_scripted_state",
                        );
                        ensure_json_object_mut(scope, "flags").insert(key.clone(), flag.to_json());
                    }
                    _ => unreachable!("validated scripted flag scope"),
                }
            }
            SCRIPTED_STATE_VALUE_FLAG_TOKEN_ONLY => {
                let token_id = row.token_id.expect("validated scripted token flag token");
                let flag = ScriptFlagState {
                    value_raw: row.value_raw,
                    date_hour_raw: row.date_hour_raw,
                };
                if row.deleted {
                    match row.scope_kind {
                        SCRIPTED_STATE_SCOPE_GLOBAL => {
                            self.scripted_world_token_only_flags.remove(&token_id);
                        }
                        SCRIPTED_STATE_SCOPE_COUNTRY => {
                            clear_country_extra_u32_key(
                                self.country_mut(row.owner_id),
                                "_r2_token_only_flags",
                                token_id,
                            );
                        }
                        SCRIPTED_STATE_SCOPE_STATE => {
                            clear_state_scripted_u32_key(
                                self.states.entry(row.owner_id.to_string()).or_default(),
                                "token_only_flags",
                                token_id,
                            );
                        }
                        _ => unreachable!("validated scripted token scope"),
                    }
                    return Ok(());
                }
                match row.scope_kind {
                    SCRIPTED_STATE_SCOPE_GLOBAL => {
                        self.scripted_world_token_only_flags.insert(token_id, flag);
                    }
                    SCRIPTED_STATE_SCOPE_COUNTRY => {
                        let extra = ensure_json_object_mut(self.country_mut(row.owner_id), "extra");
                        ensure_json_object_mut(extra, "_r2_token_only_flags")
                            .insert(token_id.to_string(), flag.to_json());
                    }
                    SCRIPTED_STATE_SCOPE_STATE => {
                        let scope = ensure_json_object_mut(
                            self.states.entry(row.owner_id.to_string()).or_default(),
                            "_r2_scripted_state",
                        );
                        ensure_json_object_mut(scope, "token_only_flags")
                            .insert(token_id.to_string(), flag.to_json());
                    }
                    _ => unreachable!("validated scripted token scope"),
                }
            }
            SCRIPTED_STATE_VALUE_DELAYED_EVENT_OPAQUE
            | SCRIPTED_STATE_VALUE_SCRIPTED_GUI_OPAQUE => {
                self.scripted_world_opaque_badges
                    .push(ScriptedOpaqueBadgeState {
                        value_kind: row.value_kind,
                        scope_kind: row.scope_kind,
                        owner_id: row.owner_id,
                        key: row.key.clone(),
                    });
            }
            SCRIPTED_STATE_VALUE_DELAYED_EVENT => {
                // owner_id = owner/scope tag_db_id; token_id = combined event
                // id; date_hour_raw = scope tag_db_id; value_raw = scope.random
                // seed pair. The due-date is intentionally absent. All three
                // present-flags are enforced by validate_scripted_state_row so
                // the seed is never a fabricated 0.
                let combined_event_id = row
                    .token_id
                    .expect("validated delayed-event event id token");
                let scope_tag_db_id =
                    row.date_hour_raw
                        .expect("validated delayed-event scope tag") as u32;
                let scope_random = row
                    .value_raw
                    .expect("validated delayed-event scope random seed")
                    as u64;
                self.scripted_world_delayed_events.insert(
                    (row.owner_id, combined_event_id, scope_random),
                    DelayedEventFoldState {
                        owner_tag_db_id: row.owner_id,
                        combined_event_id,
                        scope_tag_db_id,
                        scope_random,
                        // Ancillary: absent stays absent (badged, never 0).
                        due_remaining_hours: row.delayed_due_hours,
                    },
                );
            }
            _ => unreachable!("validated scripted value kind"),
        }
        Ok(())
    }

    // --- DOMAIN 20 divisions_live (baseline + delta) ----------------------
    fn fold_divisions(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 32, "divisions header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let expected = 32 + row_count * DIVISION_ROW_LEN;
        if payload.len() != expected {
            return Err(format!("divisions len {} != {expected}", payload.len()));
        }
        if compact_status_is_failure(status_flags) {
            self.divisions_unknown_this_hour = true;
            self.note(format!(
                "divisions failure status {:#x}; division set unknown this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        if delta && !self.delta_baselines_seen.contains(&20) {
            self.divisions_unknown_this_hour = true;
            self.record_domain_problem(20, "delta_before_baseline");
            self.note(
                "divisions delta_before_baseline; division set unknown this hour".to_string(),
            );
            return Ok(());
        }
        if !delta {
            // full baseline replaces the whole division set
            self.divisions.clear();
        }
        let mut cursor = 32usize;
        for _ in 0..row_count {
            let div_id = read_u32(payload, cursor);
            let div_type = read_u32(payload, cursor + 4);
            let flags = read_u32(payload, cursor + 8);
            let uid = format!("{div_id}:{div_type}");
            let order_instance_id = read_u32(payload, cursor + 84);
            let strategic_redeployment_raw = read_u32(payload, cursor + 92);
            let current_manpower = read_u32(payload, cursor + 96);
            let cumulative_losses = read_u32(payload, cursor + 100);
            let has_order_army = flags & DIVISION_FLAG_ORDER_ARMY != 0;
            let has_order_group = flags & DIVISION_FLAG_ORDER_ARMY_GROUP != 0;
            let has_order_unresolved = flags & DIVISION_FLAG_ORDER_UNRESOLVED != 0;
            let has_strategic_redeployment = flags & DIVISION_FLAG_STRATEGIC_REDEPLOYMENT != 0;
            let has_current_manpower = flags & DIVISION_FLAG_CURRENT_MANPOWER != 0;
            let has_cumulative_losses = flags & DIVISION_FLAG_CUMULATIVE_LOSSES != 0;
            // Validate the order-link flags/tail for EVERY row (including
            // deletes) BEFORE acting on the delete, so a malformed deleted row
            // carrying an order flag or a nonzero tail id fails the whole fold
            // closed instead of silently deleting.
            if (has_order_army as u32 + has_order_group as u32 + has_order_unresolved as u32) > 1 {
                return Err("divisions row has conflicting order-link flags".into());
            }
            if has_order_army {
                if order_instance_id == 0 || order_instance_id > DIVISION_MAX_ORDER_INSTANCE_ID {
                    return Err(format!(
                        "divisions row has invalid order instance id {order_instance_id}"
                    ));
                }
            } else if order_instance_id != 0 {
                return Err("divisions row has order instance id without army-order flag".into());
            }
            validate_optional_u32_field(
                has_strategic_redeployment,
                strategic_redeployment_raw,
                0,
                1,
                "strategic_redeployment_raw",
            )?;
            validate_optional_u32_field(
                has_current_manpower,
                current_manpower,
                0,
                DIVISION_MAX_CURRENT_MANPOWER,
                "current_manpower",
            )?;
            validate_optional_u32_field(
                has_cumulative_losses,
                cumulative_losses,
                0,
                DIVISION_MAX_CUMULATIVE_LOSSES,
                "cumulative_losses",
            )?;
            if flags & DIVISION_FLAG_DELETED != 0 {
                // A deleted row's only permitted flag is DELETED itself.
                if flags != DIVISION_FLAG_DELETED {
                    return Err("divisions delete row carries non-delete flags".into());
                }
                if let Some(key) = self
                    .divisions
                    .get(&uid)
                    .and_then(division_equipment_join_key)
                {
                    self.division_equipment_deleted_division_keys_this_hour
                        .insert(key);
                }
                self.divisions.remove(&uid);
                cursor += DIVISION_ROW_LEN;
                continue;
            }
            let location = read_u32(payload, cursor + 12);
            let move_target = read_u32(payload, cursor + 16);
            let movement_progress = read_u32(payload, cursor + 24);
            let organisation_raw = read_u32(payload, cursor + 32);
            let strength_raw = read_u32(payload, cursor + 40);
            let template_id = read_u32(payload, cursor + 52);
            let owner = read_u32(payload, cursor + 56);
            let army_id = read_u32(payload, cursor + 60);
            let experience_raw = read_u32(payload, cursor + 64);
            let current_supply_raw = read_u32(payload, cursor + 72);
            let supply_gain_raw = read_u32(payload, cursor + 76);
            let fuel_raw = read_u32(payload, cursor + 80);
            let disrupted_supply_raw = read_u32(payload, cursor + 88);

            let moving = flags & 1 != 0;
            let has_target = flags & (1 << 2) != 0;
            let in_combat = flags & (1 << 4) != 0;
            let has_owner = flags & (1 << 5) != 0;
            let has_army = flags & (1 << 6) != 0;
            let has_experience = flags & (1 << 7) != 0;
            let has_current_supply = flags & (1 << 9) != 0;
            let has_supply_gain = flags & DIVISION_FLAG_SUPPLY_GAIN != 0;
            let has_fuel = flags & (1 << 11) != 0;
            let has_disrupted_supply = flags & DIVISION_FLAG_DISRUPTED_SUPPLY != 0;
            let has_template = flags & DIVISION_FLAG_TEMPLATE != 0;

            let mut obj = Map::new();
            obj.insert("div_uid".into(), json!(uid.clone()));
            obj.insert("div_id".into(), json!(div_id));
            obj.insert("div_type".into(), json!(div_type));
            obj.insert(
                "owner".into(),
                if has_owner {
                    json!(owner.to_string())
                } else {
                    Value::Null
                },
            );
            obj.insert("location_province".into(), json!(location));
            obj.insert(
                "template_id".into(),
                if has_template {
                    json!(template_id)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "organisation".into(),
                json!(organisation_raw as f64 / FIXED_SCALE),
            );
            obj.insert("strength".into(), json!(strength_raw as f64 / FIXED_SCALE));
            obj.insert("moving".into(), json!(moving));
            obj.insert(
                "move_target_province".into(),
                if has_target {
                    json!(move_target)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "move_progress".into(),
                if moving {
                    json!(movement_progress)
                } else {
                    Value::Null
                },
            );
            obj.insert("in_combat".into(), json!(in_combat));
            obj.insert(
                "experience".into(),
                if has_experience {
                    json!(experience_raw as f64 / FIXED_SCALE)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "supply_status".into(),
                if has_current_supply {
                    json!(current_supply_raw as f64 / FIXED_SCALE)
                } else {
                    Value::Null
                },
            );
            // Wave-4: supply_gain (already in the D20 row, now folded) and
            // disrupted_supply (new). Null when the field was absent
            // (unreadable); a genuine 0.0 is present and kept.
            obj.insert(
                "supply_gain".into(),
                if has_supply_gain {
                    json!(supply_gain_raw as f64 / FIXED_SCALE)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "disrupted_supply".into(),
                if has_disrupted_supply {
                    json!(disrupted_supply_raw as f64 / FIXED_SCALE)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "strategic_redeployment".into(),
                if has_strategic_redeployment {
                    json!(strategic_redeployment_raw != 0)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "current_manpower".into(),
                if has_current_manpower {
                    json!(current_manpower)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "cumulative_losses".into(),
                if has_cumulative_losses {
                    json!(cumulative_losses)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "fuel".into(),
                if has_fuel {
                    json!(fuel_raw as f64 / FIXED_SCALE)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "army_id".into(),
                if has_army {
                    json!(army_id)
                } else {
                    Value::Null
                },
            );
            // Wave-3 primary-order linkage: instance id only for validated
            // army orders; army-group / unresolved are distinct badges; both
            // flags absent means the division has no primary order (null).
            obj.insert(
                "order_instance_id".into(),
                if has_order_army {
                    json!(order_instance_id)
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "order_link".into(),
                if has_order_army {
                    json!("army")
                } else if has_order_group {
                    json!("army_group")
                } else if has_order_unresolved {
                    json!("unresolved")
                } else {
                    Value::Null
                },
            );
            self.divisions.insert(uid, obj);
            cursor += DIVISION_ROW_LEN;
        }
        if !delta {
            self.delta_baselines_seen.insert(20);
        }
        Ok(())
    }

    // --- DOMAIN 21 land_combats_live (full active snapshot) ---------------
    fn fold_land_combats(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 32, "land combats header")?;
        let combat_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let participant_count = read_u32(payload, 20) as usize;
        let reserved = read_u32(payload, 28);
        if reserved != 0 {
            return Err("land combats header reserved field is nonzero".into());
        }
        let expected = 32usize
            .saturating_add(combat_count.saturating_mul(LAND_COMBAT_ROW_LEN))
            .saturating_add(participant_count.saturating_mul(LAND_COMBAT_PARTICIPANT_ROW_LEN));
        if payload.len() != expected {
            return Err(format!("land combats len {} != {expected}", payload.len()));
        }
        if compact_status_is_failure(status_flags) {
            self.land_combats_known_this_hour = false;
            let failure_flags = status_flags & COMPACT_STATUS_FAILURE_MASK;
            self.land_combats_unknown_note_this_hour = Some(format!(
                "land_combats_live failure status {failure_flags:#x}; combats unknown this hour"
            ));
            return Ok(());
        }

        let mut cursor = 32usize;
        let mut rows: BTreeMap<(u32, u64), CombatFoldRow> = BTreeMap::new();
        let mut seen_ids = std::collections::BTreeSet::new();
        let mut declared_participants = 0usize;
        for row_index in 0..combat_count {
            let combat_id = read_u32(payload, cursor);
            let generation = read_u64(payload, cursor + 4);
            let flags = read_u32(payload, cursor + 12);
            let location = read_u32(payload, cursor + 16);
            let day = read_u32(payload, cursor + 20);
            let duration = read_u32(payload, cursor + 24);
            let attacker_count = read_u32(payload, cursor + 28) as usize;
            let defender_count = read_u32(payload, cursor + 32) as usize;
            let row_reserved = read_u32(payload, cursor + 36);
            cursor += LAND_COMBAT_ROW_LEN;

            if combat_id == 0 {
                return Err(format!("land combat row {row_index} has zero combat_id"));
            }
            if !seen_ids.insert(combat_id) {
                return Err(format!(
                    "land combat row {row_index} duplicates combat_id {combat_id}"
                ));
            }
            if flags & !0b1_1111 != 0 {
                return Err(format!(
                    "land combat row {row_index} has unknown flags {flags}"
                ));
            }
            if flags & 1 != 0 && location == 0 {
                return Err(format!(
                    "land combat row {row_index} has invalid location {location}"
                ));
            }
            if flags & 1 == 0 && location != 0 {
                return Err(format!(
                    "land combat row {row_index} has location without flag"
                ));
            }
            if flags & (1 << 1) == 0 && day != 0 {
                return Err(format!("land combat row {row_index} has day without flag"));
            }
            if flags & (1 << 2) == 0 && duration != 0 {
                return Err(format!(
                    "land combat row {row_index} has duration without flag"
                ));
            }
            if flags & (1 << 3) == 0 && attacker_count != 0 {
                return Err(format!(
                    "land combat row {row_index} has attackers without exact flag"
                ));
            }
            if flags & (1 << 4) == 0 && defender_count != 0 {
                return Err(format!(
                    "land combat row {row_index} has defenders without exact flag"
                ));
            }
            if row_reserved != 0 {
                return Err(format!(
                    "land combat row {row_index} reserved field is nonzero"
                ));
            }
            declared_participants =
                declared_participants.saturating_add(attacker_count.saturating_add(defender_count));
            rows.insert(
                (combat_id, generation),
                CombatFoldRow {
                    state: CombatState {
                        combat_id,
                        generation,
                        location: (flags & 1 != 0).then_some(location),
                        attackers: (flags & (1 << 3) != 0)
                            .then(|| Vec::with_capacity(attacker_count)),
                        defenders: (flags & (1 << 4) != 0)
                            .then(|| Vec::with_capacity(defender_count)),
                    },
                    attacker_count,
                    defender_count,
                },
            );
        }
        if declared_participants != participant_count {
            return Err(format!(
                "land combat participant_count {participant_count} != declared {declared_participants}"
            ));
        }

        let mut seen_participants = std::collections::BTreeSet::new();
        for row_index in 0..participant_count {
            let combat_id = read_u32(payload, cursor);
            let generation = read_u64(payload, cursor + 4);
            let side = read_u32(payload, cursor + 12);
            let division_id = read_u32(payload, cursor + 16);
            let row_reserved = read_u32(payload, cursor + 20);
            cursor += LAND_COMBAT_PARTICIPANT_ROW_LEN;
            let Some(row) = rows.get_mut(&(combat_id, generation)) else {
                return Err(format!(
                    "land combat participant row {row_index} references unknown combat {combat_id}:{generation}"
                ));
            };
            if division_id == 0 {
                return Err(format!(
                    "land combat participant row {row_index} has zero division_id"
                ));
            }
            if !seen_participants.insert((combat_id, generation, side, division_id)) {
                return Err(format!(
                    "land combat participant row {row_index} duplicates participant"
                ));
            }
            match side {
                1 => match row.state.attackers.as_mut() {
                    Some(list) => list.push(division_id),
                    None => {
                        return Err(format!(
                        "land combat participant row {row_index} has attacker without exact flag"
                    ))
                    }
                },
                2 => match row.state.defenders.as_mut() {
                    Some(list) => list.push(division_id),
                    None => {
                        return Err(format!(
                        "land combat participant row {row_index} has defender without exact flag"
                    ))
                    }
                },
                _ => {
                    return Err(format!(
                        "land combat participant row {row_index} has invalid side {side}"
                    ))
                }
            }
            if row_reserved != 0 {
                return Err(format!(
                    "land combat participant row {row_index} reserved field is nonzero"
                ));
            }
        }

        let mut fresh = BTreeMap::new();
        for row in rows.into_values() {
            if row
                .state
                .attackers
                .as_ref()
                .is_some_and(|list| list.len() != row.attacker_count)
            {
                return Err(format!(
                    "land combat {} attacker count mismatch",
                    row.state.combat_id
                ));
            }
            if row
                .state
                .defenders
                .as_ref()
                .is_some_and(|list| list.len() != row.defender_count)
            {
                return Err(format!(
                    "land combat {} defender count mismatch",
                    row.state.combat_id
                ));
            }
            fresh.insert(row.state.combat_id, row.state);
        }
        // Full active snapshot: a missing combat has disappeared by this hour.
        self.combats = fresh;
        self.land_combats_known_this_hour = true;
        self.land_combats_unknown_note_this_hour = None;
        Ok(())
    }

    // --- DOMAIN 55 land_combat_detail_events_live (current-hour events) ----
    fn fold_land_combat_detail_events(&mut self, payload: &[u8]) -> Result<(), String> {
        let decoded = land_detail55::decode_detail_events_payload(0, payload)
            .map_err(|errors| errors.join("; "))?;
        let baseline_reset = decoded.status_flags & COMPACT_STATUS_TRANSITION_BASELINE_RESET != 0;
        if baseline_reset {
            self.combat_detail_events_this_hour.clear();
            self.note(format!(
                "land_combat_detail_events_live transition baseline reset after an unknown gap; detail events may be missing for this hour"
            ));
        }
        let failure_flags = decoded.status_flags & COMPACT_STATUS_FAILURE_MASK;
        let tolerated_rebaseline_failure =
            baseline_reset && failure_flags == COMPACT_STATUS_PARTIAL_DATA;
        if compact_status_is_failure(decoded.status_flags) && !tolerated_rebaseline_failure {
            self.combat_detail_events_this_hour.clear();
            self.note(format!(
                "land_combat_detail_events_live failure status {failure_flags:#x}; detail enrichment unknown this hour"
            ));
            return Ok(());
        }
        for row in decoded.rows {
            if baseline_reset {
                continue;
            }
            let Some(event_name) = detail_event_kind_name(row.event_kind) else {
                continue;
            };
            let entry = self
                .combat_detail_events_this_hour
                .entry((row.combat_id, row.generation))
                .or_default();
            match row.event_kind {
                land_detail55::EVENT_PHASE_STARTED
                | land_detail55::EVENT_PHASE_UPDATED
                | land_detail55::EVENT_PHASE_ENDED => {
                    entry.phase_event = Some(event_name.to_string());
                    entry.generation = Some(row.generation);
                    if row.location.is_some() {
                        entry.location = row.location;
                    }
                    if row.day.is_some() {
                        entry.day = row.day;
                    }
                    if row.duration.is_some() {
                        entry.duration = row.duration;
                    }
                }
                land_detail55::EVENT_PARTICIPANT_JOINED
                | land_detail55::EVENT_PARTICIPANT_LEFT_UNCLASSIFIED => {
                    if let (Some(side), Some(division_id)) = (row.side, row.division_id) {
                        let mut obj = Map::new();
                        obj.insert("event".into(), json!(event_name));
                        obj.insert("generation".into(), json!(row.generation));
                        obj.insert(
                            "side".into(),
                            json!(match side {
                                land_detail55::CombatSide::Attacker => "attacker",
                                land_detail55::CombatSide::Defender => "defender",
                            }),
                        );
                        obj.insert("division_id".into(), json!(division_id));
                        obj.insert("old_side_count".into(), json!(row.old_side_count));
                        obj.insert("new_side_count".into(), json!(row.new_side_count));
                        if row.event_kind == land_detail55::EVENT_PARTICIPANT_LEFT_UNCLASSIFIED {
                            obj.insert("semantic".into(), json!("left_unclassified"));
                        }
                        entry.participant_events.push(Value::Object(obj));
                    }
                }
                _ => {}
            }
        }
        // E2: per-side current-state detail scalars, keyed by
        // (combat_id, generation) exactly like the event rows.
        for row in decoded.side_detail_rows {
            let entry = self
                .combat_detail_events_this_hour
                .entry((row.combat_id, row.generation))
                .or_default();
            match row.side {
                land_detail55::CombatSide::Attacker => entry.attacker_detail = Some(row.detail),
                land_detail55::CombatSide::Defender => entry.defender_detail = Some(row.detail),
            }
        }
        for row in decoded.attribution_rows {
            let entry = self
                .combat_detail_events_this_hour
                .entry((row.combat_id, row.generation))
                .or_default();
            match row.side {
                land_detail55::CombatSide::Attacker => entry.attacker_attribution.push(row.entry),
                land_detail55::CombatSide::Defender => entry.defender_attribution.push(row.entry),
            }
        }
        Ok(())
    }

    /// JSON for one side's CCombatSide detail. Damage raws are i64 ×1e5;
    /// tactic and losses are DELIBERATELY absent (unresolved / not
    /// disambiguable on this build — see the lane badges).
    fn side_detail_json(detail: &land_detail55::SideDetail) -> Value {
        let scale = land_detail55::SIDE_DETAIL_FIXED_SCALE as f64;
        json!({
            "units": detail.units,
            "committed": detail.committed,
            "reserve": detail.reserve,
            "air_kills": detail.air_kills,
            "org_loss_summary_index": detail.org_loss_summary_index,
            "num_org_losses": detail.num_org_losses,
            "anti_air_attack": detail.anti_air_attack_raw as f64 / scale,
            "air_damage_str": detail.air_damage_str_raw as f64 / scale,
            "air_damage_org": detail.air_damage_org_raw as f64 / scale,
            "ground_damage_str": detail.ground_damage_str_raw as f64 / scale,
            "ground_damage_org": detail.ground_damage_org_raw as f64 / scale,
            "prevented_damage_str": detail.prevented_damage_str_raw as f64 / scale,
            "prevented_damage_org": detail.prevented_damage_org_raw as f64 / scale,
            "anti_air_attack_raw": detail.anti_air_attack_raw,
            "air_damage_str_raw": detail.air_damage_str_raw,
            "air_damage_org_raw": detail.air_damage_org_raw,
            "ground_damage_str_raw": detail.ground_damage_str_raw,
            "ground_damage_org_raw": detail.ground_damage_org_raw,
            "prevented_damage_str_raw": detail.prevented_damage_str_raw,
            "prevented_damage_org_raw": detail.prevented_damage_org_raw,
            // Unresolved / omitted fields, badged so a consumer never treats
            // their absence as zero.
            "tactic": Value::Null,
            "tactic_status": "unresolved_pointer_table_backed",
            "losses_status": "omitted_all_zero_oracle",
            // True when committed+reserve != units on a STABLE read (a genuine
            // mid-combat transient — the counts are momentarily inconsistent,
            // not authoritative). The damage/kill accumulators are still valid.
            "counts_incomplete": detail.counts_incomplete,
        })
    }

    fn attach_side_attribution(
        mut side_json: Value,
        rows: &[land_detail55::AttributionEntry],
    ) -> Value {
        if let Some(obj) = side_json.as_object_mut() {
            if !rows.is_empty() {
                obj.insert("damage_attribution".into(), Self::attribution_json(rows));
            }
        }
        side_json
    }

    fn attribution_only_side_json(rows: &[land_detail55::AttributionEntry]) -> Option<Value> {
        (!rows.is_empty()).then(|| {
            let mut obj = Map::new();
            obj.insert("damage_attribution".into(), Self::attribution_json(rows));
            Value::Object(obj)
        })
    }

    fn attribution_json(rows: &[land_detail55::AttributionEntry]) -> Value {
        let scale = land_detail55::ATTRIBUTION_FIXED_SCALE as f64;
        Value::Array(
            rows.iter()
                .map(|row| {
                    json!({
                        "dealer_tag_db_id": row.dealer_tag_db_id,
                        "victim_tag_db_id": row.victim_tag_db_id,
                        "damage": row.damage_raw as f64 / scale,
                        "damage_raw": row.damage_raw,
                    })
                })
                .collect(),
        )
    }

    // --- DOMAIN 49 naval combat lifecycle (cumulative delta) --------------
    fn fold_naval_combat_lifecycle(&mut self, payload: &[u8]) -> Result<(), String> {
        // A later failed/undecodable record in the same hour overrides an
        // earlier success as the current-frame witness without clearing state.
        self.naval_combat_lifecycle_known_this_hour = false;
        let (status_flags, rows) =
            naval4950::decode_lifecycle_rows(payload).map_err(|errors| errors.join("; "))?;
        // handle_compact_failure strips domain 49's lane-local bit 11 before the
        // failure-mask check, so the location-candidate badge is never confused
        // with COMPACT_STATUS_TRANSITION_BASELINE_RESET. On a real failure the
        // rows are empty; retain the carried state for later deltas, but do not
        // emit it as current battle existence.
        if self.handle_compact_failure(
            NAVAL_COMBAT_LIFECYCLE_DOMAIN_ID,
            "naval_combat_lifecycle_live",
            status_flags,
        ) {
            return Ok(());
        }
        // Cumulative delta: the collector emits every active row on the first
        // frame (empty previous baseline) and only changes thereafter, so no
        // separate baseline record exists to gate on.
        for row in rows {
            let key = (row.combat_id, row.generation);
            if row.deleted {
                self.naval_combats.remove(&key);
                continue;
            }
            let entry = self
                .naval_combats
                .entry(key)
                .or_insert_with(|| NavalCombatFoldState {
                    combat_id: row.combat_id,
                    generation: row.generation,
                    location_id: None,
                    region_id: None,
                    attacker_member_count: None,
                    defender_member_count: None,
                    attacker_positioning_milli: None,
                    defender_positioning_milli: None,
                    members: BTreeMap::new(),
                });
            entry.location_id = row.location_id;
            entry.region_id = row.region_id;
            entry.attacker_member_count = row.attacker_member_count;
            entry.defender_member_count = row.defender_member_count;
            entry.attacker_positioning_milli = row.attacker_positioning_milli;
            entry.defender_positioning_milli = row.defender_positioning_milli;
        }
        self.naval_combat_lifecycle_known_this_hour = true;
        self.mark_fold_domain_success(NAVAL_COMBAT_LIFECYCLE_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 50 naval combat members (cumulative delta) ----------------
    fn fold_naval_combat_members(&mut self, payload: &[u8]) -> Result<(), String> {
        // A later failed/undecodable record in the same hour overrides an
        // earlier success as the current-frame witness without clearing state.
        self.naval_combat_members_known_this_hour = false;
        let (status_flags, rows) =
            naval4950::decode_member_rows(payload).map_err(|errors| errors.join("; "))?;
        if self.handle_compact_failure(
            NAVAL_COMBAT_MEMBERS_DOMAIN_ID,
            "naval_combat_members_live",
            status_flags,
        ) {
            return Ok(());
        }
        // Validate the whole current D50 delta before mutating any member
        // accumulator. A live row without a current/carried D49 parent is a
        // torn ordering, not a partial roster that may be published. Deletes
        // for already-removed combats remain valid idempotent no-ops.
        if rows.iter().any(|row| {
            !row.deleted
                && !self
                    .naval_combats
                    .contains_key(&(row.combat_id, row.generation))
        }) {
            self.mark_fold_domain_unknown(
                NAVAL_COMBAT_MEMBERS_DOMAIN_ID,
                "orphan_member_reference",
                "naval_combat_members_live referenced an unknown active combat; member leaves unknown this hour and accumulator retained",
            );
            return Ok(());
        }
        for row in rows {
            let combat_key = (row.combat_id, row.generation);
            let member_key = (row.side.wire_id(), row.group_index, row.member_index);
            let Some(combat) = self.naval_combats.get_mut(&combat_key) else {
                // A member for a combat/generation with no active lifecycle row
                // is a torn stream ordering; skip rather than fabricate a
                // parentless combat (delete rows for already-removed combats
                // land here too and are correctly no-ops).
                continue;
            };
            if row.deleted {
                combat.members.remove(&member_key);
            } else {
                combat.members.insert(member_key, row);
            }
        }
        self.naval_combat_members_known_this_hour = true;
        self.mark_fold_domain_success(NAVAL_COMBAT_MEMBERS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 48 construction/building/resource damage live sidecar -----
    fn fold_construction_buildings_state_damage(&mut self, payload: &[u8]) -> Result<(), String> {
        let decoded = match domain48::decoder_gate::decode_domain48_compact(payload) {
            Ok(decoded) => decoded,
            Err(err) => {
                self.clear_domain48_extras();
                self.domain48_known_this_hour = false;
                self.domain48_unknown_this_hour = true;
                self.record_domain_problem(
                    CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
                    "decode_failed",
                );
                return Err(err);
            }
        };
        let status_flags = decoded.frame.status_flags;
        let failure_flags = compact_failure_flags_after_expected_partial_tolerance(
            CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
            status_flags,
        );
        let expected_partial = compact_expected_partial_tolerance_applies(
            CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
            status_flags,
        );
        if failure_flags != 0 {
            self.clear_domain48_extras();
            self.domain48_known_this_hour = false;
            self.domain48_unknown_this_hour = true;
            self.mark_fold_domain_unknown(
                CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
                "compact_status_failure",
                format!(
                    "construction_buildings_state_damage_live failure status {:#x}; domain extras unknown this hour",
                    status_flags & COMPACT_STATUS_FAILURE_MASK
                ),
            );
            return Ok(());
        }
        if !decoded.gate.ok {
            self.clear_domain48_extras();
            self.domain48_known_this_hour = false;
            self.domain48_unknown_this_hour = true;
            self.mark_fold_domain_unknown(
                CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
                "lane_gate_failed",
                format!(
                    "construction_buildings_state_damage_live gate failed: {}",
                    decoded.gate.errors.join("; ")
                ),
            );
            return Ok(());
        }

        let delta = status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        if delta
            && !self
                .delta_baselines_seen
                .contains(&CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID)
        {
            self.clear_domain48_extras();
            self.domain48_known_this_hour = false;
            self.domain48_unknown_this_hour = true;
            self.record_domain_problem(
                CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
                "delta_before_baseline",
            );
            self.note(
                "construction_buildings_state_damage_live delta_before_baseline; domain extras unknown this hour",
            );
            return Ok(());
        }

        if expected_partial {
            if let Some(tolerance) = compact_contract::expected_partial_tolerance(
                CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
            ) {
                self.record_domain_problem(
                    CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID,
                    "partial_data",
                );
                self.note(format!(
                    "construction_buildings_state_damage_live expected partial tolerance applied: {}",
                    tolerance.reason
                ));
            }
        }

        let frame = decoded.frame;
        // Seed the working state (mirrors d53's clean/delta/partial split):
        // - DELTA frame: seed from the DELTA BASELINE (`domain48_delta_state`), NOT
        //   the projected state. The collector computed this delta against its
        //   `tracker.last_*` (which a tolerated-partial frame never advanced), so
        //   applying it onto that same baseline is the only correct fold. Seeding
        //   from `domain48_state` (which a partial may have sparse-merged into)
        //   would re-introduce the partial's transient phantom/reverted rows.
        // - PARTIAL frame: seed from the projected state and sparse-merge (retain
        //   omitted rows), preserving the tolerance's original intent for this
        //   hour's snapshot.
        // - CLEAN frame: full replace from empty.
        let mut state = if delta {
            self.domain48_delta_state.clone().unwrap_or_default()
        } else if expected_partial {
            self.domain48_state.clone().unwrap_or_default()
        } else {
            Domain48FoldState::default()
        };
        state.source_flags = frame.source_flags;
        for row in frame.construction_rows {
            let key = domain48_construction_key(&row);
            if domain48::construction_row_deleted_id(&row).is_some() {
                state.construction.remove(&key);
            } else {
                state.construction.insert(key, row);
            }
        }
        for row in frame.building_rows {
            let key = domain48_building_key(&row);
            if domain48::building_row_deleted_id(&row).is_some() {
                state.buildings.remove(&key);
            } else {
                state.buildings.insert(key, row);
            }
        }
        for row in frame.resource_rows {
            let key = domain48_resource_identity_key(&row);
            if domain48::resource_row_deleted_id(&row).is_some() {
                state.resources.remove(&key);
            } else {
                state.resources.insert(key, row);
            }
        }

        let target_state_unavailable =
            state.source_flags & domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE != 0;
        let mut construction_rows = state
            .construction
            .values()
            .cloned()
            .collect::<Vec<domain48::ConstructionQueueRow>>();
        construction_rows.sort_by_key(|row| (row.tag_db_id, row.priority, row.line_id));
        let frame = domain48::LaneFrame {
            game_hour_raw: 0,
            status_flags,
            source_flags: state.source_flags,
            observed_country_roots: 0,
            observed_state_roots: 0,
            observed_province_roots: 0,
            construction_rows,
            building_rows: state.buildings.values().cloned().collect(),
            resource_rows: state.resources.values().cloned().collect(),
        };
        self.clear_domain48_extras();

        let mut construction_by_country: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
        for row in frame.construction_rows {
            let mut obj = Map::new();
            obj.insert("id".into(), json!(row.line_id));
            obj.insert("template_id".into(), option_u32_json(row.template_id));
            obj.insert("active_factories".into(), json!(row.active_factories));
            obj.insert("cost_raw".into(), json!(row.cost_raw));
            obj.insert("produced_raw".into(), json!(row.produced_raw));
            obj.insert("priority".into(), json!(row.priority));
            // W4: raw hour-count of the line's created_date (replay renders
            // raw->Y.M.D.H with the shared date codec). Absent when the read
            // failed/torn — omit the key rather than emit a fabricated 0.
            if let Some(created_date_raw) = row.created_date_raw {
                obj.insert("created_date_raw".into(), json!(created_date_raw));
            }
            if !target_state_unavailable {
                obj.insert(
                    "target_state_id".into(),
                    option_u32_json(row.target_state_id),
                );
            }
            construction_by_country
                .entry(row.tag_db_id)
                .or_default()
                .push(Value::Object(obj));
        }
        for (tag_db_id, rows) in construction_by_country {
            let country = self.country_mut(tag_db_id);
            let production = ensure_json_object_mut(country, "production");
            let extra =
                ensure_json_object_mut(production, CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY);
            domain48_upsert_construction_queue(extra, rows, expected_partial);
            extra.insert("source".into(), json!("C"));
            if target_state_unavailable {
                extra.insert("target_state_available".into(), json!(false));
            }
        }

        let mut state_buildings: BTreeMap<u32, Vec<domain48::BuildingRow>> = BTreeMap::new();
        let mut province_buildings: BTreeMap<u32, Vec<domain48::BuildingRow>> = BTreeMap::new();
        for row in frame.building_rows {
            match row.scope {
                domain48::SCOPE_STATE => state_buildings.entry(row.state_id).or_default().push(row),
                domain48::SCOPE_PROVINCE => province_buildings
                    .entry(row.location_id)
                    .or_default()
                    .push(row),
                other => {
                    self.clear_domain48_extras();
                    self.domain48_known_this_hour = false;
                    self.domain48_unknown_this_hour = true;
                    return Err(format!("domain48 building row has invalid scope {other}"));
                }
            }
        }
        for (state_id, rows) in state_buildings {
            let state = self.states.entry(state_id.to_string()).or_default();
            let extra =
                ensure_json_object_mut(state, CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY);
            domain48_upsert_building_rows_by_type(extra, "state_buildings", rows, expected_partial);
            extra.insert("source".into(), json!("C"));
        }
        for (province_id, rows) in province_buildings {
            let state_id = rows.first().map(|row| row.state_id).unwrap_or_default();
            let province = self.provinces.entry(province_id.to_string()).or_default();
            let extra =
                ensure_json_object_mut(province, CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY);
            extra.insert("state_id".into(), json!(state_id));
            domain48_upsert_building_rows_by_type(extra, "buildings", rows, expected_partial);
            extra.insert("source".into(), json!("C"));
        }

        for row in frame.resource_rows {
            let state = self.states.entry(row.state_id.to_string()).or_default();
            let extra =
                ensure_json_object_mut(state, CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY);
            let resources = ensure_json_object_mut(extra, "resource_amount_raw_by_id");
            resources.insert(
                domain48_resource_key(row.resource_id),
                json!(row.amount_raw),
            );
            extra.insert("source".into(), json!("C"));
        }

        // Advance the DELTA BASELINE only on clean/delta frames — a
        // tolerated-partial frame must NEVER mutate it (it is exactly the
        // collector's `tracker.last_*` advance condition). The projected state
        // always reflects this frame (including a partial's sparse merge).
        if !expected_partial {
            self.domain48_delta_state = Some(state.clone());
        }
        self.domain48_state = Some(state);
        self.domain48_known_this_hour = true;
        self.domain48_unknown_this_hour = false;
        if !delta && !expected_partial {
            self.delta_baselines_seen
                .insert(CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID);
        }
        self.mark_fold_domain_success(CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID);
        Ok(())
    }

    fn clear_domain48_extras(&mut self) {
        for country in self.countries.values_mut() {
            let remove_production =
                if let Some(Value::Object(production)) = country.get_mut("production") {
                    production.remove(CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY);
                    production.is_empty()
                } else {
                    false
                };
            if remove_production {
                country.remove("production");
            }
        }
        for state in self.states.values_mut() {
            state.remove(CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY);
        }
        for province in self.provinces.values_mut() {
            province.remove(CONSTRUCTION_BUILDINGS_STATE_DAMAGE_EXTRA_KEY);
        }
    }

    // --- DOMAIN 53 diplomacy_intel_operatives_live (validated R2 slice) ---
    fn fold_diplomacy_intel_operatives(&mut self, payload: &[u8]) -> Result<(), String> {
        // A decode failure is a d53 rejection like any other: it must set
        // known=false so a stale success earlier this hour is NOT projected
        // (Codex). Do NOT clear the accumulator — a later good frame this hour can
        // still build on it.
        let decoded = match diplomacy53::compact::decode_snapshot_payload(0, payload) {
            Ok(decoded) => decoded,
            Err(errors) => {
                let err = errors.join("; ");
                self.domain53_known_this_hour = false;
                self.mark_fold_domain_unknown(
                    DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
                    "decode_failed",
                    format!(
                        "diplomacy_intel_operatives_live decode failed: {err}; domain53 extras omitted"
                    ),
                );
                return Err(err);
            }
        };
        let status_flags = decoded.status_flags;
        // FACS is emitted only when the producer has a complete non-empty
        // roster. With the current wire contract, no FACS rows therefore means
        // unavailable, never measured-empty.
        let faction_roster_status = if decoded.factions.is_empty() {
            FactionRosterStatus::Unavailable
        } else {
            FactionRosterStatus::Measured { is_empty: false }
        };
        // Layer-2 (D53_LAYER2_SPEC.md): a tolerated-PARTIAL d53 frame carries
        // valid rows plus known per-row omissions. SPARSE-MERGE it into the hour
        // (retain rows it omits) instead of clearing; a CLEAN frame is
        // authoritative and full-replaces (byte-identical to the prior behavior).
        // `is_partial` is only ever true here for the expected-partial-tolerated
        // PARTIAL bit — a genuine failure bit is rejected by the gate below.
        let is_partial = (status_flags & COMPACT_STATUS_PARTIAL_DATA) != 0;
        let is_delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

        // Gates run FIRST, without mutating the accumulator. EVERY rejection sets
        // `domain53_known_this_hour = false` — projection gates purely on that
        // flag, so a failed frame (even after an earlier success this hour) must
        // NOT publish the retained rows (Codex). The overlays are deliberately
        // RETAINED (not cleared) on failure so a later good frame this hour can
        // still build on them; only a CLEAN success below clears+replaces.
        let residual_failure_flags = compact_failure_flags_after_expected_partial_tolerance(
            DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
            status_flags,
        );
        if residual_failure_flags != 0 {
            self.domain53_known_this_hour = false;
            self.mark_fold_domain_unknown(
                DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
                "compact_status_failure",
                format!(
                    "diplomacy_intel_operatives_live failure status {residual_failure_flags:#x}; domain53 extras omitted",
                ),
            );
            return Err("compact_status_failure".to_string());
        }
        // DELTA|PARTIAL is ACCEPTED (lane/d53-partial-delta). It was previously
        // rejected here, which — combined with the collector re-emitting the full
        // read on every partial frame — is why the d53 delta path never fired in
        // deep war (400/400 frames partial). A DELTA|PARTIAL frame is a delta
        // diffed against the last authoritative (non-partial) full read, with the
        // collector's baseline pinned across the partial. It flows into the
        // `is_delta` branch below, which seeds the hour from the fold's mirror
        // delta baseline (`domain53_delta_*`) and applies the upserts/tombstones;
        // the baseline-advance guard at the end of this function keeps that
        // baseline pinned on partial frames (`if !is_partial`), exactly mirroring
        // the collector. `reject_delta_before_baseline` below still requires a
        // prior clean baseline, so a DELTA|PARTIAL frame can never apply against
        // an empty baseline.
        if self.reject_delta_before_baseline(
            DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
            "diplomacy_intel_operatives_live",
            status_flags,
        ) {
            self.domain53_known_this_hour = false;
            return Err("delta_before_baseline".to_string());
        }
        if decoded
            .wars
            .iter()
            .any(|row| !domain53_row_deleted(row.first_tag_db_id))
            && status_flags & diplomacy53::compact::STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED == 0
        {
            self.domain53_known_this_hour = false;
            self.record_domain_problem(
                DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
                "war_lifecycle_badge_missing",
            );
            return Err(
                "diplomacy_intel_operatives_live war rows lack lifecycle-unvalidated badge"
                    .to_string(),
            );
        }
        if !decoded.operation_anchors.is_empty() || !decoded.intel_map.is_empty() {
            self.domain53_known_this_hour = false;
            self.record_domain_problem(
                DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID,
                "excluded_rows_present",
            );
            return Err(
                "diplomacy_intel_operatives_live emitted fixture-blocked operation/intel-map rows"
                    .to_string(),
            );
        }

        // All gates passed. Now mutate the accumulator: a CLEAN frame is
        // authoritative (clear + full-replace, byte-identical to the prior
        // behavior); a tolerated-PARTIAL frame keeps the accumulator and merges.
        // The clear happens HERE (not at the top) so a failed frame above never
        // wipes a good accumulator before returning (Codex).
        if !is_partial && !is_delta {
            self.domain53_country_overlays_this_hour.clear();
            self.domain53_world_overlay_this_hour = Domain53WorldOverlay::default();
            self.delta_baselines_seen
                .insert(DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID);
        } else if is_delta {
            self.domain53_country_overlays_this_hour = self.domain53_delta_country_overlays.clone();
            self.domain53_world_overlay_this_hour = self.domain53_delta_world_overlay.clone();
            self.domain53_world_overlay_this_hour
                .operation_instances
                .clear();
            self.domain53_world_overlay_this_hour
                .incoming_actions
                .clear();
            self.domain53_world_overlay_this_hour
                .intel_network_graphs
                .clear();
            self.domain53_world_overlay_this_hour.factions.clear();
            self.domain53_world_overlay_this_hour.faction_roster_status =
                FactionRosterStatus::Unavailable;
            // A DELTA|PARTIAL frame (lane/d53-partial-delta) is still a partial
            // read: record the same non-fatal partial-data problem the legacy
            // full-emit partial frame records, so the folded snapshot's
            // provenance stays byte-identical between full and reduced emission.
            if is_partial {
                self.record_domain_problem(DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID, "partial_data");
            }
        } else {
            // Tolerated per-row skips: record the (non-fatal) partial-data problem
            // for manifest visibility, mirroring the generic expected-partial
            // tolerance path other domains use.
            self.record_domain_problem(DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID, "partial_data");
        }

        // Populate via keyed UPSERT. On a CLEAN frame the overlays were just
        // cleared, so upsert == push (byte-identical to the prior behavior). On a
        // tolerated-PARTIAL frame the overlays are NOT cleared, so each present
        // row updates its keyed slot and any row the frame omits is RETAINED from
        // earlier in the hour (never fabricated as a deletion). `country`/`agency`
        // /`agency_capacity` are per-country Option leaves: set for the countries
        // present this frame, retained for those absent.
        for row in decoded.countries {
            let tag_db_id = domain53_row_key(row.tag_db_id);
            let overlay = self
                .domain53_country_overlays_this_hour
                .entry(tag_db_id)
                .or_default();
            if domain53_row_deleted(row.tag_db_id) {
                overlay.country = None;
            } else {
                overlay.country = Some(row);
            }
        }
        for row in decoded.opinions {
            let src_tag_db_id = domain53_row_key(row.src_tag_db_id);
            let overlay = self
                .domain53_country_overlays_this_hour
                .entry(src_tag_db_id)
                .or_default();
            let key = (src_tag_db_id, row.target_tag_db_id);
            if domain53_row_deleted(row.src_tag_db_id) {
                domain53_remove_by_key(&mut overlay.opinions, key, |r| {
                    (r.src_tag_db_id, r.target_tag_db_id)
                });
            } else {
                domain53_upsert_by_key(&mut overlay.opinions, row, |r| {
                    (r.src_tag_db_id, r.target_tag_db_id)
                });
            }
        }
        for row in decoded.relation_flags {
            let src_tag_db_id = domain53_row_key(row.src_tag_db_id);
            let overlay = self
                .domain53_country_overlays_this_hour
                .entry(src_tag_db_id)
                .or_default();
            let key = (src_tag_db_id, row.target_tag_db_id, row.flag_kind);
            if domain53_row_deleted(row.src_tag_db_id) {
                domain53_remove_by_key(&mut overlay.relation_flags, key, |r| {
                    (r.src_tag_db_id, r.target_tag_db_id, r.flag_kind)
                });
            } else {
                domain53_upsert_by_key(&mut overlay.relation_flags, row, |r| {
                    (r.src_tag_db_id, r.target_tag_db_id, r.flag_kind)
                });
            }
        }
        for row in decoded.agencies {
            let owner_tag_db_id = domain53_row_key(row.owner_tag_db_id);
            let overlay = self
                .domain53_country_overlays_this_hour
                .entry(owner_tag_db_id)
                .or_default();
            if domain53_row_deleted(row.owner_tag_db_id) {
                overlay.agency = None;
            } else {
                overlay.agency = Some(row);
            }
        }
        for row in decoded.agency_capacity_clusters {
            let owner_tag_db_id = domain53_row_key(row.owner_tag_db_id);
            let overlay = self
                .domain53_country_overlays_this_hour
                .entry(owner_tag_db_id)
                .or_default();
            if domain53_row_deleted(row.owner_tag_db_id) {
                overlay.agency_capacity = None;
            } else {
                overlay.agency_capacity = Some(row);
            }
        }
        for row in decoded.operatives {
            let op_id = domain53_row_key(row.op_id);
            if domain53_row_deleted(row.op_id) {
                for overlay in self.domain53_country_overlays_this_hour.values_mut() {
                    domain53_remove_by_key(&mut overlay.operatives, op_id, |r| r.op_id);
                }
            } else {
                // Operatives are GLOBALLY unique by op_id, but stored per-owner. A
                // delta that changes an operative's owner (A -> B) upserts under B;
                // the collector keys operatives globally and emits an owner change as
                // a plain upsert (no tombstone under A). Without dropping the stale
                // copy under A the operative would appear under BOTH owners in delta
                // mode, whereas full emission rebuilds hourly and never duplicates
                // (adversarial review Finding 2). RELOCATE: scan every OTHER owner's
                // overlay and remove this op_id before upserting under the new owner.
                let new_owner = row.owner_tag_db_id;
                for (owner_tag, overlay) in self.domain53_country_overlays_this_hour.iter_mut() {
                    if *owner_tag != new_owner {
                        domain53_remove_by_key(&mut overlay.operatives, op_id, |r| r.op_id);
                    }
                }
                let overlay = self
                    .domain53_country_overlays_this_hour
                    .entry(new_owner)
                    .or_default();
                domain53_upsert_by_key(&mut overlay.operatives, row, |r| r.op_id);
            }
        }
        for row in decoded.cryptology {
            let owner_tag_db_id = domain53_row_key(row.owner_tag_db_id);
            let overlay = self
                .domain53_country_overlays_this_hour
                .entry(owner_tag_db_id)
                .or_default();
            if domain53_row_deleted(row.owner_tag_db_id) {
                domain53_remove_by_key(&mut overlay.cryptology, row.target_tag_db_id, |r| {
                    r.target_tag_db_id
                });
            } else {
                domain53_upsert_by_key(&mut overlay.cryptology, row, |r| r.target_tag_db_id);
            }
        }
        for row in decoded.state_events {
            let owner_tag_db_id = domain53_row_key(row.owner_tag_db_id);
            let overlay = self
                .domain53_country_overlays_this_hour
                .entry(owner_tag_db_id)
                .or_default();
            // State-event identity = which object + which signal (NOT the value),
            // so a value change updates the same slot instead of accumulating.
            let key = (
                row.target_tag_db_id,
                row.event_kind,
                row.source_kind,
                row.object_hash,
            );
            if domain53_row_deleted(row.owner_tag_db_id) {
                domain53_remove_by_key(&mut overlay.state_events, key, |r| {
                    (
                        r.target_tag_db_id,
                        r.event_kind,
                        r.source_kind,
                        r.object_hash,
                    )
                });
            } else {
                domain53_upsert_by_key(&mut overlay.state_events, row, |r| {
                    (
                        r.target_tag_db_id,
                        r.event_kind,
                        r.source_kind,
                        r.object_hash,
                    )
                });
            }
        }
        if is_partial || is_delta {
            // Sparse-merge the world-scoped families by their stable keys into the
            // hour's accumulator (retain any this frame omits); update the status.
            let world = &mut self.domain53_world_overlay_this_hour;
            world.status_flags = domain53_projected_status_flags(status_flags);
            world.faction_roster_status = faction_roster_status;
            for row in decoded.wars {
                let key =
                    domain53_war_key(domain53_row_key(row.first_tag_db_id), row.second_tag_db_id);
                if domain53_row_deleted(row.first_tag_db_id) {
                    domain53_remove_by_key(&mut world.wars, key, |r| {
                        domain53_war_key(r.first_tag_db_id, r.second_tag_db_id)
                    });
                } else {
                    domain53_upsert_by_key(&mut world.wars, row, |r| {
                        domain53_war_key(r.first_tag_db_id, r.second_tag_db_id)
                    });
                }
            }
            for row in decoded.subjects {
                let key = (
                    domain53_row_key(row.overlord_tag_db_id),
                    row.subject_tag_db_id,
                );
                if domain53_row_deleted(row.overlord_tag_db_id) {
                    domain53_remove_by_key(&mut world.subjects, key, |r| {
                        (r.overlord_tag_db_id, r.subject_tag_db_id)
                    });
                } else {
                    domain53_upsert_by_key(&mut world.subjects, row, |r| {
                        (r.overlord_tag_db_id, r.subject_tag_db_id)
                    });
                }
            }
            for row in decoded.operation_instances {
                domain53_upsert_by_key(&mut world.operation_instances, row, |r| r.instance_id);
            }
            for row in decoded.incoming_actions {
                domain53_upsert_by_key(&mut world.incoming_actions, row, |r| r.action_id);
            }
            for row in decoded.intel_network_graphs {
                // Key is (strategic_operative_index, network_slot) — the compact
                // validator's unique key. Target is NOT part of the key: a slot
                // that RETARGETS must UPDATE in place, not append a stale duplicate
                // alongside the old target (Codex).
                domain53_upsert_by_key(&mut world.intel_network_graphs, row, |r| {
                    (r.strategic_operative_index, r.network_slot)
                });
            }
            for row in decoded.factions {
                domain53_upsert_by_key(&mut world.factions, row, |r| r.faction_id);
            }
        } else {
            // CLEAN frame: authoritative full-replace (byte-identical).
            self.domain53_world_overlay_this_hour = Domain53WorldOverlay {
                status_flags: domain53_projected_status_flags(status_flags),
                wars: decoded.wars,
                subjects: decoded.subjects,
                // Wave-5 running-operation instances (world-scoped: the +0x10
                // owner is rejected, so instances are not per-country keyed).
                operation_instances: decoded.operation_instances,
                incoming_actions: decoded.incoming_actions,
                intel_network_graphs: decoded.intel_network_graphs,
                factions: decoded.factions,
                faction_roster_status,
            };
        }
        self.domain53_country_overlays_this_hour
            .retain(|_, overlay| !overlay.is_empty());
        // ESTABLISH-FROM-FIRST-PARTIAL (lane/d53-partial-delta). Deep-war captures
        // can be all-partial from the very first d53 frame (measured 400/400
        // partial), so if the delta baseline were only ever set by a clean frame it
        // would never be established there and delta would stay disabled. The
        // collector emits the FIRST frame in full and seeds its baseline from it
        // regardless of the partial bit; mirror that here: a partial FULL frame
        // (is_partial && !is_delta) that is the first d53 frame establishes the
        // baseline and marks it seen, so the following DELTA|PARTIAL frames pass
        // `reject_delta_before_baseline` and seed from it. This is backward-safe:
        // a legacy recording emits no DELTA|PARTIAL, so a baseline seeded from its
        // first partial-full frame is either never consumed OR is overwritten by
        // the next clean-full frame before any pure-delta consumes it.
        let establishing_first_baseline = is_partial
            && !is_delta
            && !self
                .delta_baselines_seen
                .contains(&DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID);
        // Advance the delta baseline on a non-partial frame (clean or pure delta),
        // OR on the first-baseline-establishing partial-full frame. A subsequent
        // DELTA|PARTIAL frame leaves it PINNED (mirroring the collector's
        // `last_emitted` guard) so a row torn on a partial frame is never lost from
        // the baseline. (Was `!is_partial || is_delta`; `|| is_delta` only ever
        // fired for pure-delta frames, which are also `!is_partial`.)
        if !is_partial || establishing_first_baseline {
            self.domain53_delta_country_overlays = self.domain53_country_overlays_this_hour.clone();
            self.domain53_delta_world_overlay = self.domain53_world_overlay_this_hour.clone();
        }
        if establishing_first_baseline {
            self.delta_baselines_seen
                .insert(DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID);
        }
        self.domain53_known_this_hour = true;
        self.mark_fold_domain_success(DIPLOMACY_INTEL_OPERATIVES_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 60 capitulation_war_state_live (E1 validated slice) -------
    fn fold_capitulation_war_state(&mut self, payload: &[u8]) -> Result<(), String> {
        // Strict lane decode: row-level validation, exact-end framing, and the
        // mandatory war running-accumulator badge are all enforced inside.
        let decoded = capwar60::compact::decode_snapshot_payload(0, payload)
            .map_err(|errors| errors.join("; "))?;
        let status_flags = decoded.status_flags;
        self.capitulation_war_state_known_this_hour = false;
        self.war_state_wars_this_hour.clear();

        // INVARIANT: domain 60 is a FULL-REPLACE domain with NO expected-partial
        // tolerance (asserted in tests). It must NEVER rebuild the persistent
        // capitulation map from a non-clean frame — a single torn country read
        // on a live game would otherwise wipe the whole political-map
        // capitulation state to "nobody capitulated" until the next clean frame.
        //
        // The collector's per-row-skip fail-closed path emits an EMPTY payload
        // with `status_flags | COMPACT_STATUS_PARTIAL_DATA`. PARTIAL_DATA is
        // presently a member of COMPACT_STATUS_FAILURE_MASK, so
        // `compact_status_is_failure` ALREADY returns true for that frame and
        // the map is retained. The explicit `|| PARTIAL_DATA` term below is
        // DEFENSIVE HARDENING: it makes the retain-on-partial intent local and
        // self-evident so this full-replace domain stays correct even if
        // PARTIAL_DATA were ever demoted out of the failure mask (a plausible
        // future refactor — PARTIAL is arguably informational for
        // partial-tolerant domains, but for a no-tolerance full-replace domain
        // it must ALWAYS mean "don't full-replace"). Retain on ANY non-clean
        // frame regardless of which flag the source set.
        let non_clean = compact_status_is_failure(status_flags)
            || (status_flags & COMPACT_STATUS_PARTIAL_DATA) != 0;
        if non_clean {
            self.mark_fold_domain_unknown(
                CAPITULATION_WAR_STATE_DOMAIN_ID,
                "compact_status_non_clean",
                format!(
                    "capitulation_war_state_live non-clean status {status_flags:#x}; wars unknown this hour, last proven capitulation map RETAINED (not wiped)"
                ),
            );
            return Err("compact_status_non_clean".to_string());
        }

        // STRUCTURAL guard (closes the one residual an adversarial review found:
        // a clean-status but EMPTY payload would otherwise full-replace to
        // empty and wipe the map). A genuinely clean, complete D60 frame ALWAYS
        // carries the resolved CCountry set (the frame only reaches "clean"
        // after resolving hundreds of live country roots); an empty country set
        // on a clean status is implausible — the collector never emits it on
        // success, only on the fail-closed paths which carry PARTIAL_DATA. Treat
        // clean+empty as non-authoritative and RETAIN, rather than trusting a
        // status field alone. (If a future map with a genuinely empty world ever
        // exists, it would arrive with countries anyway; zero countries is the
        // signal, not a legitimate state.)
        if decoded.countries.is_empty() {
            self.mark_fold_domain_unknown(
                CAPITULATION_WAR_STATE_DOMAIN_ID,
                "clean_but_empty_country_set",
                "capitulation_war_state_live clean status but zero country rows; \
                 last proven capitulation map RETAINED (not wiped)",
            );
            return Err("clean_but_empty_country_set".to_string());
        }

        // Full-replace: each proven (clean) frame re-sends the complete country
        // set. A country absent from the fresh map loses its capitulation key
        // (absent != false), matching the live read's validated-absence rule.
        let mut fresh = BTreeMap::new();
        for row in decoded.countries {
            fresh.insert(row.tag_db_id, row);
        }
        self.capitulation_by_db_id = fresh;
        self.war_state_wars_this_hour = decoded.wars;
        self.capitulation_war_state_known_this_hour = true;
        self.mark_fold_domain_success(CAPITULATION_WAR_STATE_DOMAIN_ID);
        Ok(())
    }

    /// Attach `countries[*].capitulation` from the persistent D60 map. Creates
    /// the country object when no other domain contributed one this hour (a
    /// capitulated minor must never be dropped for lacking economy rows).
    fn project_capitulation_country_state(
        &self,
        countries: &mut Map<String, Value>,
        unresolved: &mut BTreeSet<u32>,
    ) {
        for (tag_db_id, row) in &self.capitulation_by_db_id {
            let key = self.resolve_country_key(&tag_db_id.to_string(), unresolved);
            let entry = countries
                .entry(key)
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(country) = entry.as_object_mut() else {
                continue;
            };
            country.insert("capitulation".into(), capitulation_country_json(row));
        }
    }

    /// Build the §7b `diplomacy_world.wars` object from the current-hour D60
    /// war overlay: keyed `"FIRST|SECOND"` with resolved TAGs where available.
    fn build_war_state_wars(&self, unresolved: &mut BTreeSet<u32>) -> Value {
        let mut wars = Map::new();
        for row in &self.war_state_wars_this_hour {
            let first = self.resolve_country_key(&row.first_tag_db_id.to_string(), unresolved);
            let second = self.resolve_country_key(&row.second_tag_db_id.to_string(), unresolved);
            wars.insert(
                format!("{first}|{second}"),
                json!({
                    "first": first,
                    "second": second,
                    "first_tag_db_id": row.first_tag_db_id,
                    "second_tag_db_id": row.second_tag_db_id,
                    // Decode guarantees an in-window date; a None here would be
                    // a logic error, surfaced as null rather than a fabricated
                    // date (fail closed).
                    "start_date": compact_contract::calendar::date_from_raw(row.start_date_raw)
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                    "start_date_raw": row.start_date_raw,
                    "first_casualties": row.first_casualties,
                    "second_casualties": row.second_casualties,
                    // Casualty semantics (per-war, explicit so a consumer never
                    // reads an ACTIVE war's live total as a save-snapshot value):
                    // the +0x40/+0x48 fields are LIVE RUNNING ACCUMULATORS. For a
                    // frozen war (combat ended) they equal the serialized save
                    // snapshot byte-for-byte (validated: GER-LUX 1110/948). For an
                    // active war they legitimately EXCEED the save snapshot and
                    // keep climbing each tick. Frozen-vs-active is NOT
                    // distinguishable from live memory alone on this build (no
                    // in-object "war frozen" flag was cracked), so the field is
                    // conservatively "live_running_accumulator" for every row —
                    // treat every value as a lower bound that is exact only if the
                    // war is known (from elsewhere) to be frozen.
                    "casualties_semantics": "live_running_accumulator",
                    "casualties_is_save_snapshot": false,
                }),
            );
        }
        Value::Object(wars)
    }

    // --- DOMAIN 54 army_battleplans_depth_live (R2 OOB annotation) -------
    fn fold_army_battleplans_depth(&mut self, payload: &[u8]) -> Result<(), String> {
        let decoded_summary = battleplans54::decode_depth_payload(0, payload);
        if !decoded_summary.parse_errors.is_empty() {
            return Err(decoded_summary.parse_errors.join("; "));
        }
        let status_flags = decoded_summary.totals.status_flags_or;
        let hard_failure_mask = COMPACT_STATUS_FAILURE_MASK & !COMPACT_STATUS_PARTIAL_DATA;
        if (status_flags & hard_failure_mask) != 0 {
            self.army_battleplans_depth_unknown_this_hour = true;
            self.mark_fold_domain_unknown(
                ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID,
                "compact_status_failure",
                format!(
                    "army_battleplans_depth_live failure status {:#x}; battleplan depth state unknown this hour",
                    status_flags & hard_failure_mask
                ),
            );
            return Ok(());
        }
        if (status_flags & COMPACT_STATUS_DELTA_DATA) != 0
            && !self
                .delta_baselines_seen
                .contains(&ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID)
        {
            self.army_battleplans_depth_unknown_this_hour = true;
            self.mark_fold_domain_unknown(
                ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID,
                "delta_before_baseline",
                "army_battleplans_depth_live delta arrived before a full baseline; rows ignored this hour",
            );
            return Ok(());
        }

        let decoded = decode_army_battleplans_depth_payload(payload)?;
        self.army_battleplans_depth_omitted_non_exact_this_hour = decoded.omitted_non_exact_rows;
        self.army_battleplans_depth_path_omitted_this_hour = decoded.path_geometry_omitted_rows;
        if !decoded.is_delta {
            self.army_battleplans_depth.clear();
            self.delta_baselines_seen
                .insert(ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID);
        }

        for row in decoded.rows {
            if row.deleted {
                self.army_battleplans_depth.remove(&row.order.key);
            } else {
                self.army_battleplans_depth
                    .insert(row.order.key.clone(), row.order);
            }
        }

        self.army_battleplans_depth_unknown_this_hour = false;
        self.mark_fold_domain_success(ARMY_BATTLEPLANS_DEPTH_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 57 session_multiplayer_metadata_live (R2 sidecar) --------
    fn fold_session_multiplayer_metadata(&mut self, payload: &[u8]) -> Result<(), String> {
        let gate = session57::gate_payload(payload);
        if !gate.ok {
            return Err(gate.errors.join("; "));
        }

        need(payload, session57::PAYLOAD_HEADER_LEN, "domain57 header")?;
        let status_flags = read_u32(payload, 20);
        let source_flags = read_u32(payload, 24);
        let player_count = read_u32(payload, 28) as usize;
        let event_count = read_u32(payload, 32) as usize;
        let string_bytes = read_u32(payload, 36) as usize;
        let root_evidence_mask = read_u32(payload, 44);
        let player_bytes = player_count
            .checked_mul(session57::PLAYER_COUNTRY_ROW_LEN)
            .ok_or("domain57 player row byte overflow")?;
        let event_bytes = event_count
            .checked_mul(session57::SESSION_EVENT_ROW_LEN)
            .ok_or("domain57 event row byte overflow")?;
        let strings_start = session57::PAYLOAD_HEADER_LEN
            .checked_add(player_bytes)
            .and_then(|value| value.checked_add(event_bytes))
            .ok_or("domain57 strings offset overflow")?;
        let expected_len = strings_start
            .checked_add(string_bytes)
            .ok_or("domain57 payload len overflow")?;
        if expected_len != payload.len() {
            return Err(format!("domain57 len {} != {expected_len}", payload.len()));
        }
        let strings = &payload[strings_start..];

        let mut player_rows = Vec::new();
        let mut cursor = session57::PAYLOAD_HEADER_LEN;
        for row_index in 0..player_count {
            let row = &payload[cursor..cursor + session57::PLAYER_COUNTRY_ROW_LEN];
            cursor += session57::PLAYER_COUNTRY_ROW_LEN;
            let country_tag = d57_string_ref(strings, read_u32(row, 16), read_u16(row, 14))?;
            let tag_db_id = read_u32(row, 20);
            let flags = read_u32(row, 28);
            let valid_from_hour_raw = read_i32(row, 32);
            if flags & session57::PLAYER_FLAG_HUMAN == 0 {
                self.record_domain_problem(
                    SESSION_MULTIPLAYER_METADATA_DOMAIN_ID,
                    "non_human_player_row_ignored",
                );
                continue;
            }
            if !valid_domain57_country_tag(&country_tag) {
                return Err(format!("domain57 player row {row_index} invalid TAG"));
            }
            if tag_db_id == 0
                || tag_db_id > compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
            {
                return Err(format!(
                    "domain57 player row {row_index} invalid tag_db_id {tag_db_id}"
                ));
            }
            // The compact row does not carry the raw +0x388 status byte; this
            // is only the normalized nonzero-human marker for the partial slice.
            player_rows.push(Domain57PlayerCountryProjection {
                country_tag,
                tag_db_id,
                human_marker: true,
                valid_from_hour_raw,
            });
        }

        let mut pause_is_paused = None;
        for _ in 0..event_count {
            let row = &payload[cursor..cursor + session57::SESSION_EVENT_ROW_LEN];
            cursor += session57::SESSION_EVENT_ROW_LEN;
            let raw_kind = read_u16(row, 0);
            let Some(kind) = session57::SessionEventKind::from_raw(raw_kind) else {
                continue;
            };
            if kind != session57::SessionEventKind::Pause {
                continue;
            }
            let value_i32 = read_i32(row, 32);
            if value_i32 != 0 && value_i32 != 1 {
                return Err(format!("domain57 pause value {value_i32} is not 0/1"));
            }
            pause_is_paused = Some(value_i32 != 0);
        }

        let hard_failure = status_flags
            & (session57::STATUS_WORKER_PANIC | session57::STATUS_BRIDGE_UNAVAILABLE)
            != 0;
        let source_root_validated =
            (source_flags & session57::SOURCE_FLAG_SESSION_ROOT_VALIDATED) != 0;
        let player_root_validated =
            source_root_validated && (root_evidence_mask & DOMAIN57_ROOT_PLAYER_COUNTRY) != 0;
        let pause_root_validated =
            source_root_validated && (root_evidence_mask & DOMAIN57_ROOT_PAUSE_CLOCK) != 0;

        let players_known = !hard_failure && player_root_validated && !player_rows.is_empty();
        let pause_known = !hard_failure && pause_root_validated && pause_is_paused.is_some();
        self.session_multiplayer_metadata_this_hour = Some(Domain57FoldState {
            players_by_country: players_known.then_some(player_rows),
            pause_is_paused: if pause_known { pause_is_paused } else { None },
        });

        if status_flags & session57::STATUS_SOURCE_UNAVAILABLE != 0 {
            self.record_domain_problem(
                SESSION_MULTIPLAYER_METADATA_DOMAIN_ID,
                "source_unavailable",
            );
        }
        if status_flags & session57::STATUS_PARTIAL_DATA != 0 {
            self.record_domain_problem(SESSION_MULTIPLAYER_METADATA_DOMAIN_ID, "partial_data");
        }
        if !players_known {
            self.record_domain_problem(
                SESSION_MULTIPLAYER_METADATA_DOMAIN_ID,
                "players_by_country_unavailable",
            );
        }
        if !pause_known {
            self.record_domain_problem(SESSION_MULTIPLAYER_METADATA_DOMAIN_ID, "pause_unavailable");
        }

        if players_known || pause_known {
            self.mark_fold_domain_success(SESSION_MULTIPLAYER_METADATA_DOMAIN_ID);
        } else {
            self.mark_fold_domain_unknown(
                SESSION_MULTIPLAYER_METADATA_DOMAIN_ID,
                "source_unavailable",
                "session_multiplayer_metadata_live unavailable this hour",
            );
        }
        Ok(())
    }

    // --- DOMAIN 56 naval_missions_status_live (R2 OOB annotation) --------
    fn fold_naval_missions_status(&mut self, payload: &[u8]) -> Result<(), String> {
        let decoded =
            naval56::decode_status_payload(0, payload).map_err(|errors| errors.join("; "))?;
        let status_flags = decoded.header.status_flags;
        if self.handle_compact_failure(
            NAVAL_MISSIONS_STATUS_DOMAIN_ID,
            "naval_missions_status_live",
            status_flags,
        ) {
            return Ok(());
        }
        if self.reject_delta_before_baseline(
            NAVAL_MISSIONS_STATUS_DOMAIN_ID,
            "naval_missions_status_live",
            status_flags,
        ) {
            return Ok(());
        }

        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        // D56 currently emits full-replace frames and has no deletion tombstone
        // contract. A valid non-delta frame is therefore the only frame type that
        // may clear stale task-force status rows.
        if !delta {
            self.naval_missions_status.clear();
        }

        let naval56::DecodedNavalMissionStatusPayload {
            rows,
            naval_supremacy_rows,
            ..
        } = decoded;

        for row in rows {
            self.naval_missions_status.insert(
                row.task_force_id,
                NavalMissionTaskForceFoldRow {
                    task_force_id: row.task_force_id,
                    fleet_id: row.fleet_id,
                    owner_tag_db_id: row.owner_tag_db_id,
                    location_province_id: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_LOCATION,
                        row.location_province_id,
                    ),
                    strategic_region_id: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_REGION,
                        row.strategic_region_id,
                    ),
                    mission_enum: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_MISSION,
                        row.mission_enum,
                    ),
                    mission_state: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_MISSION_STATE,
                        row.mission_state,
                    ),
                    home_base_province_id: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_HOME_BASE,
                        row.home_base_province_id,
                    ),
                    repair_base_province_id: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_REPAIR_BASE,
                        row.repair_base_province_id,
                    ),
                    repair_progress_milli: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_REPAIR_PROGRESS,
                        row.repair_progress_milli,
                    ),
                    retreat_state: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_RETREAT_STATE,
                        row.retreat_state,
                    ),
                    sunk_convoys: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_SUNK_CONVOY_COUNT,
                        row.sunk_convoys,
                    ),
                    convoy_route_context_id: option_flag(
                        row.flags,
                        naval56::compact::FLAG_HAS_CONVOY_ROUTE_CONTEXT,
                        row.convoy_route_context_id,
                    ),
                    fixture_required: row.flags & naval56::compact::FLAG_FIXTURE_REQUIRED != 0,
                    active_battle_fixture_required: row.flags
                        & naval56::compact::FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED
                        != 0,
                    source_status: row.source_status,
                },
            );
        }
        // The supremacy sidecar is an ORDER-PRESERVING occurrence list, not a
        // keyed map, so an accidental delta frame could double-append it (a map
        // would merely upsert). D56 supremacy is always a full snapshot, so
        // full-replace it on every successful frame regardless of the delta flag
        // — this stays correct even if a delta frame ever slips through, and the
        // task-force map above keeps its own delta/clear semantics.
        self.naval_supremacy.clear();
        for row in naval_supremacy_rows {
            // Occurrence list: retain every decoded row in emit order. The
            // game's naval-dominance array can carry several distinct entries
            // for the same (region, controller); collapsing them would lose data.
            self.naval_supremacy.push(NavalSupremacyFoldRow {
                region_id: row.region_id,
                controller_tag_db_id: row.controller_tag_db_id,
                current_raw: row.current_raw,
                target_raw: row.target_raw,
                base_target_raw: row.base_target_raw,
                previous_raw: row.previous_raw,
                decline_from_raw: row.decline_from_raw,
                individual_ratio_raw: row.individual_ratio_raw,
            });
        }

        if !delta {
            self.delta_baselines_seen
                .insert(NAVAL_MISSIONS_STATUS_DOMAIN_ID);
        }
        self.mark_fold_domain_success(NAVAL_MISSIONS_STATUS_DOMAIN_ID);
        Ok(())
    }

    // --- DOMAIN 22 map_control_live (baseline + delta) --------------------
    fn fold_map_control(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, 32, "map control header")?;
        let state_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let province_count = read_u32(payload, 20) as usize;
        let expected = 32 + state_count * MAP_STATE_ROW_LEN + province_count * MAP_PROVINCE_ROW_LEN;
        if payload.len() != expected {
            return Err(format!("map control len {} != {expected}", payload.len()));
        }
        if compact_status_is_failure(status_flags) {
            self.map_control_unknown_this_hour = true;
            self.note(format!(
                "map_control failure status {:#x}; map control state unknown this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        if delta && !self.delta_baselines_seen.contains(&22) {
            self.map_control_unknown_this_hour = true;
            self.record_domain_problem(22, "delta_before_baseline");
            self.note("map_control delta_before_baseline; map control state unknown this hour");
            return Ok(());
        }
        if !delta {
            self.map_control_states.clear();
            self.map_control_provinces.clear();
        }
        let mut cursor = 32usize;
        for _ in 0..state_count {
            let state_id = read_u32(payload, cursor);
            let owner = read_u32(payload, cursor + 4);
            let controller = read_u32(payload, cursor + 8);
            let flags = read_u32(payload, cursor + 12);
            cursor += MAP_STATE_ROW_LEN;
            let obj = self
                .map_control_states
                .entry(state_id.to_string())
                .or_default();
            obj.insert(
                "owner".into(),
                if flags & 1 != 0 {
                    json!(owner.to_string())
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "controller".into(),
                if flags & (1 << 1) != 0 {
                    json!(controller.to_string())
                } else {
                    Value::Null
                },
            );
        }
        for _ in 0..province_count {
            let province_id = read_u32(payload, cursor);
            let controller = read_u32(payload, cursor + 4);
            let state_id = read_u32(payload, cursor + 8);
            let flags = read_u32(payload, cursor + 12);
            cursor += MAP_PROVINCE_ROW_LEN;
            let obj = self
                .map_control_provinces
                .entry(province_id.to_string())
                .or_default();
            obj.insert(
                "controller".into(),
                if flags & 1 != 0 {
                    json!(controller.to_string())
                } else {
                    Value::Null
                },
            );
            obj.insert(
                "state_id".into(),
                if flags & (1 << 1) != 0 {
                    json!(state_id)
                } else {
                    Value::Null
                },
            );
        }
        if !delta {
            self.delta_baselines_seen.insert(22);
        }
        Ok(())
    }

    // --- DOMAIN 23 map_detail_live (baseline + delta) ---------------------
    fn fold_map_detail(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, MAP_DETAIL_HEADER_LEN, "map detail header")?;
        let state_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let province_count = read_u32(payload, 20) as usize;
        let resource_count = read_u32(payload, 32) as usize;
        let state_building_count = read_u32(payload, 36) as usize;
        let province_building_count = read_u32(payload, 40) as usize;
        let reserved = read_u32(payload, 44);
        let expected = MAP_DETAIL_HEADER_LEN
            .saturating_add(state_count.saturating_mul(MAP_DETAIL_STATE_ROW_LEN))
            .saturating_add(province_count.saturating_mul(MAP_DETAIL_PROVINCE_ROW_LEN))
            .saturating_add(resource_count.saturating_mul(MAP_DETAIL_ENTRY_ROW_LEN))
            .saturating_add(state_building_count.saturating_mul(MAP_DETAIL_ENTRY_ROW_LEN))
            .saturating_add(province_building_count.saturating_mul(MAP_DETAIL_ENTRY_ROW_LEN));
        if payload.len() != expected {
            return Err(format!("map detail len {} != {expected}", payload.len()));
        }
        if reserved != 0 {
            return Err("map detail header reserved field is nonzero".into());
        }
        if compact_status_is_failure(status_flags) {
            self.map_detail_unknown_this_hour = true;
            self.note(format!(
                "map_detail failure status {:#x}; map detail fields unknown this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
        if delta && !self.delta_baselines_seen.contains(&23) {
            self.map_detail_unknown_this_hour = true;
            self.record_domain_problem(23, "delta_before_baseline");
            self.note("map_detail delta_before_baseline; map detail fields unknown this hour");
            return Ok(());
        }

        let mut cursor = MAP_DETAIL_HEADER_LEN;
        let mut state_rows = Vec::with_capacity(state_count);
        for row_index in 0..state_count {
            let row = MapDetailStateRow {
                state_id: read_u32(payload, cursor),
                flags: read_u32(payload, cursor + 4),
                resource_first: read_u32(payload, cursor + 8) as usize,
                resource_len: read_u32(payload, cursor + 12) as usize,
                building_first: read_u32(payload, cursor + 16) as usize,
                building_len: read_u32(payload, cursor + 20) as usize,
            };
            let row_reserved = read_u32(payload, cursor + 24);
            cursor += MAP_DETAIL_STATE_ROW_LEN;
            if row.state_id == 0 {
                return Err(format!(
                    "map detail state row {row_index} has zero state_id"
                ));
            }
            if row.flags & !0b11 != 0 {
                return Err(format!(
                    "map detail state row {row_index} has unknown flags {}",
                    row.flags
                ));
            }
            if row_reserved != 0 {
                return Err(format!(
                    "map detail state row {row_index} reserved field is nonzero"
                ));
            }
            validate_map_detail_range(
                row.flags & 1 != 0,
                row.resource_first,
                row.resource_len,
                resource_count,
                "map detail state resource",
            )?;
            validate_map_detail_range(
                row.flags & (1 << 1) != 0,
                row.building_first,
                row.building_len,
                state_building_count,
                "map detail state building",
            )?;
            state_rows.push(row);
        }

        let mut province_rows = Vec::with_capacity(province_count);
        for row_index in 0..province_count {
            let row = MapDetailProvinceRow {
                province_id: read_u32(payload, cursor),
                state_id: read_u32(payload, cursor + 4),
                flags: read_u32(payload, cursor + 8),
                building_first: read_u32(payload, cursor + 12) as usize,
                building_len: read_u32(payload, cursor + 16) as usize,
            };
            cursor += MAP_DETAIL_PROVINCE_ROW_LEN;
            if row.province_id == 0 {
                return Err(format!(
                    "map detail province row {row_index} has zero province_id"
                ));
            }
            if row.flags & !0b11 != 0 {
                return Err(format!(
                    "map detail province row {row_index} has unknown flags {}",
                    row.flags
                ));
            }
            if row.flags & 1 != 0 && row.state_id == 0 {
                return Err(format!(
                    "map detail province row {row_index} has state flag without state"
                ));
            }
            if row.flags & 1 == 0 && row.state_id != 0 {
                return Err(format!(
                    "map detail province row {row_index} has state without flag"
                ));
            }
            validate_map_detail_range(
                row.flags & (1 << 1) != 0,
                row.building_first,
                row.building_len,
                province_building_count,
                "map detail province building",
            )?;
            province_rows.push(row);
        }

        let mut resources = Vec::with_capacity(resource_count);
        for _ in 0..resource_count {
            let entry = MapDetailI32Entry {
                type_id: read_u32(payload, cursor),
                amount_raw: read_i32(payload, cursor + 4),
            };
            let entry_reserved = read_u32(payload, cursor + 8);
            cursor += MAP_DETAIL_ENTRY_ROW_LEN;
            if entry.type_id == 0 || entry.type_id == u32::MAX {
                return Err("map detail resource entry has invalid type_id".into());
            }
            if entry_reserved != 0 {
                return Err("map detail resource entry reserved field is nonzero".into());
            }
            resources.push(entry);
        }
        let mut state_buildings = Vec::with_capacity(state_building_count);
        for _ in 0..state_building_count {
            state_buildings.push(read_map_detail_building_entry(payload, &mut cursor)?);
        }
        let mut province_buildings = Vec::with_capacity(province_building_count);
        for _ in 0..province_building_count {
            province_buildings.push(read_map_detail_building_entry(payload, &mut cursor)?);
        }

        if !delta {
            for state in self.states.values_mut() {
                state.remove("resources");
                state.remove("buildings");
                state.remove("province_buildings");
            }
            self.map_detail_province_state_ids.clear();
            self.map_detail_province_buildings.clear();
            self.map_detail_live = None;
        }

        for row in state_rows {
            let state = self.states.entry(row.state_id.to_string()).or_default();
            if row.flags & 1 != 0 {
                state.insert(
                    "resources".into(),
                    Value::Object(map_detail_resource_map(
                        &resources[row.resource_first..row.resource_first + row.resource_len],
                    )),
                );
            }
            if row.flags & (1 << 1) != 0 {
                state.insert(
                    "buildings".into(),
                    Value::Object(map_detail_building_map(
                        &state_buildings[row.building_first..row.building_first + row.building_len],
                    )),
                );
            }
        }
        for row in province_rows {
            if row.flags & 1 != 0 {
                self.map_detail_province_state_ids
                    .insert(row.province_id.to_string(), row.state_id);
            }
            if row.flags & (1 << 1) != 0 {
                self.map_detail_province_buildings.insert(
                    row.province_id.to_string(),
                    map_detail_building_map(
                        &province_buildings
                            [row.building_first..row.building_first + row.building_len],
                    ),
                );
            }
        }
        self.recompute_map_detail_province_building_totals();
        self.rebuild_map_detail_layer();

        if !delta {
            self.delta_baselines_seen.insert(23);
        }
        Ok(())
    }

    fn recompute_map_detail_province_building_totals(&mut self) {
        for state in self.states.values_mut() {
            state.remove("province_buildings");
        }
        let mut totals: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        for (province_id, buildings) in &self.map_detail_province_buildings {
            let Some(state_id) = self.map_detail_province_state_ids.get(province_id) else {
                continue;
            };
            let state_totals = totals.entry(state_id.to_string()).or_default();
            for (name, level) in buildings {
                let add = level.as_u64().unwrap_or(0);
                let current = state_totals.get(name).and_then(Value::as_u64).unwrap_or(0);
                state_totals.insert(name.clone(), json!(current.saturating_add(add)));
            }
        }
        for (state_id, buildings) in totals {
            self.states
                .entry(state_id)
                .or_default()
                .insert("province_buildings".into(), Value::Object(buildings));
        }
    }

    fn rebuild_map_detail_layer(&mut self) {
        let mut resources_by_state = Map::new();
        let mut buildings_by_state = Map::new();
        for (state_id, state) in &self.states {
            if let Some(resources) = state.get("resources") {
                resources_by_state.insert(state_id.clone(), resources.clone());
            }
            if let Some(buildings) = state.get("buildings") {
                buildings_by_state.insert(state_id.clone(), buildings.clone());
            }
        }
        let mut province_buildings_by_province = Map::new();
        for (province_id, buildings) in &self.map_detail_province_buildings {
            province_buildings_by_province
                .insert(province_id.clone(), Value::Object(buildings.clone()));
        }
        self.map_detail_live = Some(json!({
            "resources_by_state": resources_by_state,
            "buildings_by_state": buildings_by_state,
            "province_buildings_by_province": province_buildings_by_province,
        }));
    }

    // --- DOMAIN 43 market_trade_live (full current-contract snapshot) ------
    fn fold_market_trade(&mut self, payload: &[u8]) -> Result<(), String> {
        need(payload, MARKET_TRADE_HEADER_LEN, "market trade header")?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let scale_raw = read_u32(payload, 44);
        let core_end =
            MARKET_TRADE_HEADER_LEN.saturating_add(row_count.saturating_mul(MARKET_TRADE_ROW_LEN));
        if payload.len() < core_end {
            return Err(format!("market trade len {} < {core_end}", payload.len()));
        }
        // G1-residue gate finding 1 (2026-07-12): PARTIAL was previously
        // excluded from this gate and the frame folded "with omissions" — but
        // this fold is a FULL REPLACEMENT, so folding an admittedly-incomplete
        // contract set silently DELETES every unobserved live contract (the
        // collector's first-hold PARTIAL holds cached pointers while new
        // contracts exist outside the observed set). PARTIAL is unknown:
        // retain the prior contract state for the hour.
        if status_flags & COMPACT_STATUS_FAILURE_MASK != 0 {
            self.market_trade_unknown_this_hour = true;
            self.note(format!(
                "market_trade_live failure status {:#x}; trade contracts unknown this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        if scale_raw != FIXED_SCALE as u32 {
            return Err(format!(
                "market trade scale_raw {scale_raw} != {}",
                FIXED_SCALE as u32
            ));
        }
        let trailers = if payload.len() > core_end {
            parse_market_trade_trailers(payload, core_end)?
        } else {
            MarketTradeTrailersFold::default()
        };
        let detail_rows = trailers.detail_rows;

        let mut contracts = Vec::with_capacity(row_count);
        let mut seen_contracts = BTreeSet::new();
        let mut seen_contract_ids = BTreeSet::new();
        let mut cursor = MARKET_TRADE_HEADER_LEN;
        for row_index in 0..row_count {
            let contract_id = read_u32(payload, cursor);
            let contract_type = read_u32(payload, cursor + 4);
            let flags = read_u32(payload, cursor + 8);
            let seller_tag_db_id = read_u32(payload, cursor + 12);
            let buyer_tag_db_id = read_u32(payload, cursor + 16);
            let equipment_variant_id = read_u32(payload, cursor + 20);
            let amount_raw = read_i64(payload, cursor + 24);
            let equipment_subbuf_status = read_u32(payload, cursor + 32);
            let delivery_subbuf_status = read_u32(payload, cursor + 36);
            let buyer_ptr_status = read_u32(payload, cursor + 40);
            let reserved = read_u32(payload, cursor + 44);
            cursor += MARKET_TRADE_ROW_LEN;

            if contract_id == 0 {
                return Err(format!("market trade row {row_index} has zero contract_id"));
            }
            if !seen_contracts.insert((contract_id, contract_type)) {
                return Err(format!(
                    "market trade row {row_index} duplicates contract identity {contract_id}:{contract_type}"
                ));
            }
            seen_contract_ids.insert(contract_id);
            if contract_type != MARKET_TRADE_CONTRACT_TYPE {
                return Err(format!(
                    "market trade row {row_index} has unexpected contract_type {contract_type}"
                ));
            }
            if seller_tag_db_id == 0 || buyer_tag_db_id == 0 {
                return Err(format!(
                    "market trade row {row_index} has zero seller/buyer tag"
                ));
            }
            if flags & !MARKET_TRADE_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "market trade row {row_index} has unknown flags {flags:#x}"
                ));
            }
            if reserved != 0 {
                return Err(format!(
                    "market trade row {row_index} reserved field is nonzero"
                ));
            }

            let mut contract = Map::new();
            contract.insert(
                "contract_id".into(),
                json!(format!("{contract_id}:{contract_type}")),
            );
            contract.insert("seller".into(), json!(seller_tag_db_id.to_string()));
            contract.insert("buyer".into(), json!(buyer_tag_db_id.to_string()));
            contract.insert("speed".into(), Value::Null);
            if flags & MARKET_TRADE_FLAG_VARIANT_VALID != 0 {
                let mut equipment = Map::new();
                equipment.insert("variant_id".into(), json!(equipment_variant_id));
                equipment.insert(
                    "amount".into(),
                    if flags & MARKET_TRADE_FLAG_AMOUNT_VALID != 0 {
                        json!(amount_raw as f64 / FIXED_SCALE)
                    } else {
                        Value::Null
                    },
                );
                contract.insert("equipments".into(), json!([Value::Object(equipment)]));
            } else {
                contract.insert("equipments".into(), Value::Null);
            }
            contract.insert("prices".into(), Value::Null);
            contract.insert("delivery_country".into(), Value::Null);
            contract.insert("convoys".into(), Value::Null);
            contract.insert("delivery_efficiency".into(), Value::Null);
            contract.insert("days".into(), Value::Null);
            contract.insert("delivery_state".into(), Value::Null);
            contract.insert("completed".into(), Value::Null);
            contract.insert(
                "r2_market_trade_live".into(),
                json!({
                    "contract_type": contract_type,
                    "flags": flags,
                    "buyer_ptr_status": buyer_ptr_status,
                    "equipment_subbuf_status": equipment_subbuf_status,
                    "delivery_subbuf_status": delivery_subbuf_status,
                }),
            );
            if let Some(detail) = detail_rows.get(&contract_id).copied() {
                apply_market_trade_detail_to_contract(&mut contract, detail);
            }
            contracts.push(Value::Object(contract));
        }
        for contract_id in detail_rows.keys() {
            if !seen_contract_ids.contains(contract_id) {
                return Err(format!(
                    "market trade MTD1 references missing contract_id {contract_id}"
                ));
            }
        }

        self.market_trade_contracts = Some(contracts);
        if let Some(resource_exported) = trailers.resource_exported {
            self.market_trade_resource_exported = resource_exported;
        }
        Ok(())
    }

    // --- DOMAIN 43 MTD1 purchase-contract detail trailer ------------------

    fn validate_market_trade_detail_i64(
        row_index: usize,
        name: &str,
        value: i64,
        present: bool,
        min: i64,
        max: i64,
    ) -> Result<(), String> {
        if present {
            if !(min..=max).contains(&value) {
                return Err(format!(
                    "market trade MTD1 row {row_index} has invalid {name} {value}"
                ));
            }
        } else if value != 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} has {name} without flag"
            ));
        }
        Ok(())
    }

    fn validate_market_trade_detail_i32(
        row_index: usize,
        name: &str,
        value: i32,
        present: bool,
        min: i32,
        max: i32,
    ) -> Result<(), String> {
        if present {
            if !(min..=max).contains(&value) {
                return Err(format!(
                    "market trade MTD1 row {row_index} has invalid {name} {value}"
                ));
            }
        } else if value != 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} has {name} without flag"
            ));
        }
        Ok(())
    }

    // --- DOMAIN 44 supply_logistics_attrition_live (baseline + delta) ------
    fn fold_supply_logistics_attrition(&mut self, payload: &[u8]) -> Result<(), String> {
        need(
            payload,
            SUPPLY_LOGISTICS_HEADER_LEN,
            "supply logistics header",
        )?;
        let row_count = read_u32(payload, 8) as usize;
        let status_flags = read_u32(payload, 16);
        let delta = status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        let scale_raw = read_u32(payload, 20);
        let province_rows = read_u32(payload, 24) as usize;
        let division_rows = read_u32(payload, 28) as usize;
        let badge_rows = read_u32(payload, 32) as usize;
        let observed_province_roots = read_u32(payload, 36);
        let observed_division_roots = read_u32(payload, 40);
        let csupply_input_rows = read_u32(payload, 44) as usize;
        let expected = SUPPLY_LOGISTICS_HEADER_LEN
            .saturating_add(row_count.saturating_mul(SUPPLY_LOGISTICS_ROW_LEN));
        if payload.len() != expected {
            return Err(format!(
                "supply logistics len {} != {expected}",
                payload.len()
            ));
        }
        if compact_status_is_failure(status_flags) {
            self.supply_unknown_this_hour = true;
            self.note(format!(
                "supply_logistics_attrition_live failure status {:#x}; supply sidecar unknown this hour",
                status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        if scale_raw != FIXED_SCALE as u32 {
            return Err(format!(
                "supply logistics scale_raw {scale_raw} != {}",
                FIXED_SCALE as u32
            ));
        }
        if delta && !self.delta_baselines_seen.contains(&44) {
            self.supply_unknown_this_hour = true;
            self.record_domain_problem(44, "delta_before_baseline");
            self.note(
                "supply_logistics_attrition_live delta_before_baseline; supply sidecar unknown this hour",
            );
            return Ok(());
        }
        if province_rows
            .saturating_add(division_rows)
            .saturating_add(badge_rows)
            .saturating_add(csupply_input_rows)
            != row_count
        {
            return Err("supply logistics header row counters do not sum to row_count".into());
        }

        let mut state = if delta {
            self.supply_logistics.clone().unwrap_or_default()
        } else {
            SupplyLogisticsState::default()
        };
        state.observed_province_roots = observed_province_roots;
        state.observed_division_roots = observed_division_roots;
        let mut seen_provinces = BTreeSet::new();
        let mut seen_divisions = BTreeSet::new();
        let mut seen_csupply_input_provinces = BTreeSet::new();
        let mut seen_badge_flags = BTreeSet::new();
        let mut cursor = SUPPLY_LOGISTICS_HEADER_LEN;
        for row_index in 0..row_count {
            let row_kind = read_u32(payload, cursor);
            let entity_id = read_u32(payload, cursor + 4);
            let join_id = read_u32(payload, cursor + 8);
            let owner_tag_db_id = read_u32(payload, cursor + 12);
            let flags = read_u32(payload, cursor + 16);
            let value_a_raw = read_i32(payload, cursor + 20);
            let value_b_raw = read_i32(payload, cursor + 24);
            let value_c_raw = read_i32(payload, cursor + 28);
            let value_d_raw = read_i32(payload, cursor + 32);
            let aux_raw = read_i32(payload, cursor + 36);
            cursor += SUPPLY_LOGISTICS_ROW_LEN;

            if flags & !SUPPLY_ROW_KNOWN_FLAGS != 0 {
                return Err(format!(
                    "supply logistics row {row_index} has unknown flags {flags:#x}"
                ));
            }
            match row_kind {
                SUPPLY_ROW_KIND_PROVINCE_LOGISTICS => {
                    let deleted_id = supply_logistics_deleted_id(entity_id);
                    let province_id = deleted_id.unwrap_or(entity_id);
                    if province_id == 0 || province_id > SUPPLY_MAX_PROVINCE_ID {
                        return Err(format!(
                            "supply province row {row_index} has invalid province id {province_id}"
                        ));
                    }
                    if !seen_provinces.insert(province_id) {
                        return Err(format!(
                            "supply province row {row_index} duplicates province id {province_id}"
                        ));
                    }
                    if deleted_id.is_some() {
                        validate_supply_logistics_tombstone(
                            "province",
                            row_index,
                            join_id,
                            owner_tag_db_id,
                            flags,
                            [value_a_raw, value_b_raw, value_c_raw, value_d_raw, aux_raw],
                        )?;
                        state.provinces.remove(&province_id);
                        continue;
                    }
                    if join_id == 0 || join_id > SUPPLY_MAX_STATE_ID {
                        return Err(format!(
                            "supply province row {row_index} has invalid state id {join_id}"
                        ));
                    }
                    if owner_tag_db_id != 0 {
                        return Err(format!(
                            "supply province row {row_index} has nonzero owner_tag_db_id"
                        ));
                    }
                    if flags & SUPPLY_ROW_FLAG_LIVE_BUILDING_VECTOR == 0 {
                        return Err(format!(
                            "supply province row {row_index} missing live building vector flag"
                        ));
                    }
                    state.provinces.insert(
                        province_id,
                        SupplyProvinceLogisticsState {
                            province_id,
                            state_id: join_id,
                            flags,
                            rail: value_a_raw,
                            supply_node: value_b_raw,
                            naval_supply_hub: value_c_raw,
                            infrastructure: value_d_raw,
                            building_count: aux_raw,
                        },
                    );
                }
                SUPPLY_ROW_KIND_DIVISION_SUPPLY => {
                    let deleted_id = supply_logistics_deleted_id(entity_id);
                    let division_id = deleted_id.unwrap_or(entity_id);
                    if division_id == 0 || division_id > SUPPLY_MAX_DIVISION_ID {
                        return Err(format!(
                            "supply division row {row_index} has invalid raw division id {division_id}"
                        ));
                    }
                    if !seen_divisions.insert(division_id) {
                        return Err(format!(
                            "supply division row {row_index} duplicates raw division id {division_id}"
                        ));
                    }
                    if deleted_id.is_some() {
                        validate_supply_logistics_tombstone(
                            "division",
                            row_index,
                            join_id,
                            owner_tag_db_id,
                            flags,
                            [value_a_raw, value_b_raw, value_c_raw, value_d_raw, aux_raw],
                        )?;
                        state.divisions.remove(&division_id);
                        continue;
                    }
                    if join_id == 0 || join_id > SUPPLY_MAX_PROVINCE_ID {
                        return Err(format!(
                            "supply division row {row_index} has invalid province id {join_id}"
                        ));
                    }
                    if flags & SUPPLY_ROW_FLAG_PROXY_VALUE == 0 {
                        return Err(format!(
                            "supply division row {row_index} missing proxy flag"
                        ));
                    }
                    let owner_tag_db_id = if flags & SUPPLY_ROW_FLAG_DIVISION_OWNER != 0 {
                        if owner_tag_db_id == 0 || owner_tag_db_id > SUPPLY_MAX_TAG_DB_ID {
                            return Err(format!(
                                    "supply division row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                                ));
                        }
                        Some(owner_tag_db_id)
                    } else {
                        if owner_tag_db_id != 0 {
                            return Err(format!(
                                "supply division row {row_index} has owner without owner flag"
                            ));
                        }
                        None
                    };
                    state.divisions.insert(
                        division_id,
                        SupplyDivisionState {
                            raw_division_id: division_id,
                            province_id: join_id,
                            owner_tag_db_id,
                            flags,
                            current_supply_raw: value_a_raw,
                            max_supply_raw: value_b_raw,
                            supply_gain_raw: value_c_raw,
                            supply_deficit_raw: value_d_raw,
                            fuel_raw: aux_raw,
                        },
                    );
                }
                SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT => {
                    let deleted_id = supply_logistics_deleted_id(entity_id);
                    let province_id = deleted_id.unwrap_or(entity_id);
                    if province_id == 0 || province_id > SUPPLY_MAX_PROVINCE_ID {
                        return Err(format!(
                            "supply csupply row {row_index} has invalid province id {province_id}"
                        ));
                    }
                    if !seen_csupply_input_provinces.insert(province_id) {
                        return Err(format!(
                            "supply csupply row {row_index} duplicates province id {province_id}"
                        ));
                    }
                    if deleted_id.is_some() {
                        validate_supply_logistics_tombstone(
                            "csupply",
                            row_index,
                            join_id,
                            owner_tag_db_id,
                            flags,
                            [value_a_raw, value_b_raw, value_c_raw, value_d_raw, aux_raw],
                        )?;
                        state.csupply_inputs.remove(&province_id);
                        continue;
                    }
                    let expected_flags =
                        SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY | SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY;
                    if entity_id == 0 || join_id != 0 || owner_tag_db_id != 0 {
                        return Err(format!(
                            "supply csupply row {row_index} has invalid identity fields"
                        ));
                    }
                    if flags != expected_flags {
                        return Err(format!(
                            "supply csupply row {row_index} has invalid flags {flags:#x}"
                        ));
                    }
                    if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
                        return Err(format!(
                            "supply csupply row {row_index} has nonzero reserved value fields"
                        ));
                    }
                    state.csupply_inputs.insert(province_id, value_a_raw);
                }
                SUPPLY_ROW_KIND_COMPUTED_BADGE => {
                    if entity_id != 0 || join_id != 0 || owner_tag_db_id != 0 {
                        return Err(format!(
                            "supply badge row {row_index} has nonzero identity fields"
                        ));
                    }
                    let badge_flags = flags
                        & (SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL
                            | SUPPLY_ROW_FLAG_ATTRITION_WALL
                            | SUPPLY_ROW_FLAG_MOTORIZATION_WALL
                            | SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL
                            | SUPPLY_ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED);
                    if badge_flags.count_ones() != 1 || flags != badge_flags {
                        return Err(format!(
                            "supply badge row {row_index} must carry exactly one badge flag"
                        ));
                    }
                    if !seen_badge_flags.insert(badge_flags) {
                        return Err(format!(
                            "supply badge row {row_index} duplicates badge flag {badge_flags:#x}"
                        ));
                    }
                    if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
                        return Err(format!(
                            "supply badge row {row_index} has nonzero reserved value fields"
                        ));
                    }
                    state.badges.insert(badge_flags, value_a_raw);
                }
                other => {
                    return Err(format!(
                        "supply row {row_index} has unknown row_kind {other}"
                    ));
                }
            }
        }
        let required_badge_flags = SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL
            | SUPPLY_ROW_FLAG_ATTRITION_WALL
            | SUPPLY_ROW_FLAG_MOTORIZATION_WALL
            | SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL
            | SUPPLY_ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED;
        let badge_flags_seen = state
            .badges
            .keys()
            .copied()
            .fold(0u32, |seen, flags| seen | flags);
        if (badge_flags_seen & required_badge_flags) != required_badge_flags {
            self.supply_unknown_this_hour = true;
            self.record_domain_problem(44, "missing_required_badge_rows");
            self.note(
                "supply_logistics_attrition_live missing required computed badge rows; supply sidecar unknown this hour",
            );
            return Ok(());
        }

        self.supply = Some(render_supply_logistics_state(&state));
        self.supply_logistics = Some(state);
        if !delta {
            self.delta_baselines_seen.insert(44);
        }
        self.mark_fold_domain_success(44);
        Ok(())
    }

    /// Consume the fold and return every captured snapshot in emit order.
    pub fn finish(self) -> Vec<(i64, String, Vec<u8>)> {
        self.out
    }

    /// Consume a file-backed fold and return every spilled snapshot in emit order.
    /// Closes the emit channel and joins the background worker so every hour is
    /// durably written (and hashed) before returning.
    pub fn finish_file_backed(self) -> Result<Vec<FoldedSnapshotFile>, String> {
        if let Some(err) = self.file_sink_error {
            return Err(err);
        }
        let sink = self
            .file_sink
            .ok_or_else(|| "world fold was not file-backed".to_string())?;
        sink.pipeline.finish()
    }
}

const DIVISION_FLAG_TEMPLATE: u32 = 1 << 13;

#[derive(Default)]
struct ProductionAccum {
    stockpile: Vec<Value>,
    military_lines: Vec<Value>,
    naval_lines: Vec<Value>,
    construction_queue: Vec<Value>,
    /// Wave-4 production licensing: `Some(ids)` when this country carried a
    /// measured license set in the `PLIC` trailer (proven-empty is `Some([])`);
    /// `None` when no trailer row covered this country (unreadable/not measured
    /// — absent != empty). Only countries WITH a trailer row get an entry.
    licenses: Option<Vec<u32>>,
}

#[derive(Clone)]
struct CombatState {
    combat_id: u32,
    generation: u64,
    location: Option<u32>,
    attackers: Option<Vec<u32>>,
    defenders: Option<Vec<u32>>,
}

struct CombatFoldRow {
    state: CombatState,
    attacker_count: usize,
    defender_count: usize,
}

#[derive(Clone, Default)]
struct SupplyLogisticsState {
    provinces: BTreeMap<u32, SupplyProvinceLogisticsState>,
    divisions: BTreeMap<u32, SupplyDivisionState>,
    csupply_inputs: BTreeMap<u32, i32>,
    badges: BTreeMap<u32, i32>,
    observed_province_roots: u32,
    observed_division_roots: u32,
}

#[derive(Clone)]
struct SupplyProvinceLogisticsState {
    province_id: u32,
    state_id: u32,
    flags: u32,
    rail: i32,
    supply_node: i32,
    naval_supply_hub: i32,
    infrastructure: i32,
    building_count: i32,
}

#[derive(Clone)]
struct SupplyDivisionState {
    raw_division_id: u32,
    province_id: u32,
    owner_tag_db_id: Option<u32>,
    flags: u32,
    current_supply_raw: i32,
    max_supply_raw: i32,
    supply_gain_raw: i32,
    supply_deficit_raw: i32,
    fuel_raw: i32,
}

// Keyed by (army, generation) only: generation is globally unique per order
// instance (collector tracker counter), while slot_index is a mutable attribute.
// Including the slot in the key let a same-order slot change strand a stale twin
// under the old key with no tombstone able to remove it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ArmyOrderKey {
    army_id: u32,
    generation: u64,
}

struct ArmyOrderRawRow {
    army_id: u32,
    generation: u64,
    flags: u32,
    slot_index: u32,
    order_type: i32,
    target_first: usize,
    target_count: usize,
    target_state_declared_count: u32,
    target_status_code: u32,
}

struct ArmyOrderState {
    army_id: u32,
    generation: u64,
    slot_index: u32,
    order_type: i32,
    target_states: Vec<u32>,
    target_state_declared_count: u32,
    target_status_code: u32,
}

// Same identity rationale as ArmyOrderKey: (army, generation) is the stable
// order identity; slot_index is a mutable attribute kept on the order value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ArmyBattleplanDepthKey {
    army_id: u32,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ArmyBattleplanDepthOrder {
    key: ArmyBattleplanDepthKey,
    slot_index: u32,
    order_type: i32,
    target_states: Vec<u32>,
    path_provinces: Vec<u32>,
    target_declared_count: Option<u32>,
    target_status: &'static str,
    geometry_status: &'static str,
    execution_status: &'static str,
    assigned_units_status: &'static str,
    order_linkage_status: &'static str,
    stance_status: &'static str,
    aggressiveness_status: &'static str,
}

struct DecodedArmyBattleplansDepthPayload {
    is_delta: bool,
    omitted_non_exact_rows: u64,
    path_geometry_omitted_rows: u64,
    rows: Vec<DecodedArmyBattleplansDepthRow>,
}

struct DecodedArmyBattleplansDepthRow {
    deleted: bool,
    order: ArmyBattleplanDepthOrder,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OobCommandKey {
    kind: u32,
    object_id: u32,
}

struct OobCommandRawRow {
    kind: u32,
    object_id: u32,
    flags: u32,
    parent_id: u32,
    owner_tag_db_id: u32,
    leader_id: u32,
    leader_skill: u32,
    member_first: usize,
    row_member_count: usize,
    logical_member_count: u32,
    member_count_engine: u32,
    missing_member_count: u32,
    leader_trait_count: u32,
    parent_status: u32,
    member_status: u32,
    owner_status: u32,
    leader_status: u32,
    leader_trait_status: u32,
    location_province_id: u32,
}

struct OobCommandState {
    kind: u32,
    object_id: u32,
    parent_id: Option<u32>,
    owner_tag_db_id: u32,
    leader_id: Option<u32>,
    leader_skill: Option<u32>,
    members: Vec<u32>,
    logical_member_count: u32,
    member_count_engine: Option<u32>,
    missing_member_count: Option<u32>,
    leader_trait_count: u32,
    parent_status: u32,
    member_status: u32,
    owner_status: u32,
    leader_status: u32,
    leader_trait_status: u32,
    location_province_id: Option<u32>,
}

#[derive(Default)]
struct OobCountryCounts {
    armies: u64,
    army_groups: u64,
    divisions_in_armies: u64,
    fleets: u64,
    task_forces: u64,
    ships: u64,
    air_wings: u64,
    planes: u64,
}

impl OobCountryCounts {
    fn to_json(self) -> Value {
        json!({
            "armies": self.armies,
            "army_groups": self.army_groups,
            "divisions_in_armies": self.divisions_in_armies,
            "fleets": self.fleets,
            "task_forces": self.task_forces,
            "ships": self.ships,
            "air_wings": self.air_wings,
            "planes": self.planes,
        })
    }
}

struct NavalMissionTaskForceFoldRow {
    task_force_id: u32,
    fleet_id: u32,
    owner_tag_db_id: u32,
    location_province_id: Option<u32>,
    strategic_region_id: Option<u32>,
    mission_enum: Option<u32>,
    mission_state: Option<u32>,
    home_base_province_id: Option<u32>,
    repair_base_province_id: Option<u32>,
    repair_progress_milli: Option<u32>,
    retreat_state: Option<u32>,
    sunk_convoys: Option<u32>,
    convoy_route_context_id: Option<u32>,
    fixture_required: bool,
    active_battle_fixture_required: bool,
    source_status: u32,
}

struct NavalSupremacyFoldRow {
    region_id: u32,
    controller_tag_db_id: u32,
    current_raw: u32,
    target_raw: u32,
    base_target_raw: u32,
    previous_raw: u32,
    decline_from_raw: u32,
    individual_ratio_raw: u32,
}

struct ShipRawRow {
    instance_id: u32,
    flags: u32,
    task_force_id: u32,
    category_enum: u32,
    strength_raw: u32,
    organisation_raw: u32,
    experience_raw: u32,
    sunk_convoys: u32,
    owner_tag_db_id: u32,
}

struct ShipState {
    instance_id: u32,
    task_force_id: Option<u32>,
    category_enum: u32,
    strength_raw: u32,
    organisation_raw: u32,
    experience_raw: u32,
    sunk_convoys: u32,
    owner_tag_db_id: u32,
    definition: Option<String>,
}

// Domains 49/50 (naval combat lifecycle + members). Persistent cumulative
// delta state keyed by (combat_id, generation); a body-pointer change starts a
// new generation on the collector side, so a distinct key is a distinct battle
// epoch. Lifecycle deletes drop the whole epoch (and its members); member
// deletes drop a single member.
#[derive(Clone)]
struct NavalCombatFoldState {
    combat_id: u32,
    generation: u64,
    location_id: Option<u32>,
    region_id: Option<u32>,
    attacker_member_count: Option<u32>,
    defender_member_count: Option<u32>,
    attacker_positioning_milli: Option<i32>,
    defender_positioning_milli: Option<i32>,
    members: BTreeMap<(u32, u32, u32), naval4950::MemberRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AirWingKey {
    owner_tag_db_id: u32,
    wing_id: u32,
    generation: u64,
}

struct AirWingState {
    generation: u64,
    count: u32,
    mission_mask: u32,
    mission_region: Option<u32>,
    effective_planes: u32,
}

#[derive(Clone)]
struct AirWingLoadoutEntryState {
    variant_id: u32,
    amount_raw: i64,
}

struct AirWingLoadoutRawRow {
    owner_tag_db_id: u32,
    wing_id: u32,
    generation: u64,
    flags: u32,
    air_base_id: u32,
    entry_first: usize,
    entry_count: usize,
}

struct AirWingLoadoutState {
    air_base_id: Option<u32>,
    equipment: Vec<AirWingLoadoutEntryState>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AirRegionActivityKey {
    region_id: u32,
    owner_tag_db_id: u32,
    mission_mask: u32,
}

struct AirRegionActivityState {
    region_id: u32,
    owner_tag_db_id: u32,
    mission_mask: u32,
    wing_count: u32,
    plane_count: u32,
    effective_planes: u32,
}

struct AirBaseStatusState {
    air_base_id: u32,
    state_id: u32,
    capacity: u32,
    building_level: u32,
    based_wing_count: u32,
    based_plane_count: u32,
}

struct VariantDefRawRow {
    variant_id: u32,
    flags: u32,
    module_first: usize,
    module_count: usize,
    display_name: Option<String>,
    variant_name: Option<String>,
}

#[derive(Clone)]
struct VariantDefModuleState {
    slot_handle: u32,
    token: String,
}

struct VariantDefState {
    variant_id: u32,
    display_name: Option<String>,
    variant_name: Option<String>,
    modules: Vec<VariantDefModuleState>,
}

impl VariantDefState {
    fn to_json(&self) -> Value {
        json!({
            "variant_id": self.variant_id,
            "display_name": self.display_name,
            "variant_name": self.variant_name,
            "modules": self.modules.iter().map(|module| {
                json!({
                    "slot_handle": module.slot_handle,
                    "token": module.token,
                })
            }).collect::<Vec<_>>(),
            "_r2_variant_def_capture": {
                "live_variant_id": self.variant_id,
            },
        })
    }
}

struct AirVariantStatsState {
    variant_id: u32,
    flags: u32,
    equipment_token_raw: u32,
    equipment_type_token_raw: u32,
    interface_type_raw: u32,
    speed_cache_raw: i32,
    range_raw: i32,
    air_attack_raw: i32,
    agility_raw: i32,
    max_speed_raw: i32,
    reliability_raw: i32,
    air_defence_raw: i32,
    thrust_raw: i32,
    fuel_consumption_raw: i32,
    build_cost_ic_raw: i32,
    fixed_scale_raw: u32,
    range_scale_raw: u32,
}

impl AirVariantStatsState {
    fn to_json(&self) -> Value {
        let fixed = self.fixed_scale_raw as f64;
        let range = self.range_scale_raw as f64;
        json!({
            "variant_id": self.variant_id,
            "speed_cache": self.speed_cache_raw as f64 / fixed,
            "range": self.range_raw as f64 / range,
            "air_attack": self.air_attack_raw as f64 / fixed,
            "agility": self.agility_raw as f64 / fixed,
            "max_speed": self.max_speed_raw as f64 / fixed,
            "reliability": self.reliability_raw as f64 / fixed,
            "air_defence": self.air_defence_raw as f64 / fixed,
            "thrust": self.thrust_raw as f64 / fixed,
            "fuel_consumption": self.fuel_consumption_raw as f64 / fixed,
            "build_cost_ic": self.build_cost_ic_raw as f64 / fixed,
            "_r2_air_variant_stats": {
                "flags": self.flags,
                "equipment_token_raw": self.equipment_token_raw,
                "equipment_type_token_raw": self.equipment_type_token_raw,
                "interface_type_raw": self.interface_type_raw,
                "speed_cache_raw": self.speed_cache_raw,
                "range_raw": self.range_raw,
                "air_attack_raw": self.air_attack_raw,
                "agility_raw": self.agility_raw,
                "max_speed_raw": self.max_speed_raw,
                "reliability_raw": self.reliability_raw,
                "air_defence_raw": self.air_defence_raw,
                "thrust_raw": self.thrust_raw,
                "fuel_consumption_raw": self.fuel_consumption_raw,
                "build_cost_ic_raw": self.build_cost_ic_raw,
                "fixed_scale_raw": self.fixed_scale_raw,
                "range_scale_raw": self.range_scale_raw,
            },
        })
    }
}

#[derive(Clone)]
struct VariantUpgradeState {
    upgrade_index: u32,
    level: u32,
}

impl VariantUpgradeState {
    fn to_json(&self) -> Value {
        json!({
            "upgrade_index": self.upgrade_index,
            "level": self.level,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AirVariantReserveKey {
    tag_db_id: u32,
    variant_id: u32,
}

struct AirVariantReserveState {
    tag_db_id: u32,
    variant_id: u32,
    amount_raw: i64,
    scale_raw: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AirSuperiorityInputKey {
    region_id: u32,
    owner_tag_db_id: u32,
    mission_mask: u32,
    variant_id: u32,
}

struct AirSuperiorityInputState {
    region_id: u32,
    owner_tag_db_id: u32,
    mission_mask: u32,
    variant_id: u32,
    equipment_token_raw: u32,
    equipment_type_token_raw: u32,
    interface_type_raw: u32,
    wing_count: u32,
    amount_raw: i64,
    scale_raw: u32,
}

struct AirCombatLossState {
    owner_tag_db_id: u32,
    wing_id: u32,
    generation: u64,
    flags: u32,
    mission_mask: u32,
    mission_region: Option<u32>,
    old_count: u32,
    new_count: u32,
    count_delta: i32,
    old_equipment_amount_sum: i64,
    new_equipment_amount_sum: i64,
    equipment_amount_delta: i64,
    air_accidents: u32,
    experience_raw: i32,
    scale_raw: u32,
}

impl AirCombatLossState {
    fn to_json(&self, fold: &WorldFold, unresolved: &mut BTreeSet<u32>) -> Value {
        let mut value = json!({
            "owner": fold.country_ref_json(self.owner_tag_db_id, unresolved),
            "owner_tag_db_id": self.owner_tag_db_id,
            "wing_id": self.wing_id,
            "generation": self.generation,
            "mission_mask": self.mission_mask,
            "mission_region": self.mission_region,
            "old_count": self.old_count,
            "new_count": self.new_count,
            "count_delta": self.count_delta,
            "old_equipment_amount_sum": self.old_equipment_amount_sum as f64 / self.scale_raw as f64,
            "new_equipment_amount_sum": self.new_equipment_amount_sum as f64 / self.scale_raw as f64,
            "equipment_amount_delta": self.equipment_amount_delta as f64 / self.scale_raw as f64,
            "old_equipment_amount_sum_raw": self.old_equipment_amount_sum,
            "new_equipment_amount_sum_raw": self.new_equipment_amount_sum,
            "equipment_amount_delta_raw": self.equipment_amount_delta,
            "flags": self.flags,
            "scale_raw": self.scale_raw,
        });
        if self.flags & AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS != 0 {
            if let Value::Object(map) = &mut value {
                map.insert("air_accidents".into(), json!(self.air_accidents));
                map.insert(
                    "experience".into(),
                    json!(self.experience_raw as f64 / FIXED_SCALE),
                );
                map.insert("experience_raw".into(), json!(self.experience_raw));
            }
        }
        value
    }
}

/// Persistent per-wing cumulative scalar state (wave-3 crack: CAirWing+0x74
/// air_accidents, CAirWing+0x208 experience fixed-x100000).
struct AirWingCumulativeScalars {
    owner_tag_db_id: u32,
    air_accidents: u32,
    experience_raw: i32,
}

struct AirVariantArchetypeRawRow {
    variant_id: u32,
    flags: u32,
    equipment_token_raw: u32,
    equipment_type_token_raw: u32,
    interface_type_raw: u32,
    air_superiority_weight_raw: i32,
    source_mask: u32,
    archetype_token: Option<String>,
    base_airframe_token: Option<String>,
    role_token: Option<String>,
}

struct AirVariantArchetypeState {
    variant_id: u32,
    equipment_token_raw: u32,
    equipment_type_token_raw: u32,
    interface_type_raw: u32,
    air_superiority_weight_raw: Option<i32>,
    source_mask: u32,
    archetype_token: Option<String>,
    base_airframe_token: Option<String>,
    role_token: Option<String>,
    static_catalog_hash: u32,
    fixed_scale_raw: u32,
}

impl AirVariantArchetypeState {
    fn to_json(&self) -> Value {
        json!({
            "variant_id": self.variant_id,
            "archetype_token": self.archetype_token,
            "base_airframe_token": self.base_airframe_token,
            "role_token": self.role_token,
            "air_superiority_weight": self
                .air_superiority_weight_raw
                .map(|value| value as f64 / self.fixed_scale_raw as f64),
            "equipment_token_raw": self.equipment_token_raw,
            "equipment_type_token_raw": self.equipment_type_token_raw,
            "interface_type_raw": self.interface_type_raw,
            "air_superiority_weight_raw": self.air_superiority_weight_raw,
            "source_mask": self.source_mask,
            "static_catalog_hash": self.static_catalog_hash,
            "fixed_scale_raw": self.fixed_scale_raw,
        })
    }
}

#[derive(Clone, Default)]
struct CombatDetailEnrichment {
    phase_event: Option<String>,
    generation: Option<u64>,
    location: Option<u32>,
    day: Option<u32>,
    duration: Option<u32>,
    participant_events: Vec<Value>,
    /// E2: per-side CCombatSide current-state detail scalars. Absent side =
    /// no detail row emitted this hour (fail closed; never zero-filled).
    attacker_detail: Option<land_detail55::SideDetail>,
    defender_detail: Option<land_detail55::SideDetail>,
    attacker_attribution: Vec<land_detail55::AttributionEntry>,
    defender_attribution: Vec<land_detail55::AttributionEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct DivisionEquipmentKey {
    owner_tag_db_id: u32,
    div_id: u32,
}

struct DivisionEquipmentRootRow {
    key: DivisionEquipmentKey,
    entry_first: usize,
    entry_count: usize,
    deleted: bool,
}

struct DivisionEquipmentEntryState {
    variant_id: u32,
    amount_raw: i64,
    scale_raw: u32,
}

impl DivisionEquipmentEntryState {
    fn to_json(&self) -> Value {
        json!({
            "equip_id": self.variant_id,
            "amount": self.amount_raw as f64 / self.scale_raw as f64,
            "amount_raw": self.amount_raw,
            "archetype": Value::Null,
            "equip_type": Value::Null,
            "_r2_variant_id": self.variant_id,
        })
    }
}

#[derive(Default)]
struct WeatherLiveState {
    regions: BTreeMap<u32, WeatherRegionState>,
    provinces: BTreeMap<u32, WeatherProvinceState>,
    active_region_count: u32,
    observed_region_rows: u32,
    observed_province_rows: u32,
    scale_raw: u32,
}

impl WeatherLiveState {
    fn to_json(&self) -> Value {
        let mut regions = Map::new();
        for (id, region) in &self.regions {
            regions.insert(id.to_string(), region.to_json());
        }
        let mut provinces = Map::new();
        for (id, province) in &self.provinces {
            provinces.insert(id.to_string(), province.to_json());
        }
        json!({
            "source": "weather_live",
            "active_region_count": self.active_region_count,
            "observed_region_rows": self.observed_region_rows,
            "observed_province_rows": self.observed_province_rows,
            "scale_raw": self.scale_raw,
            "regions": regions,
            "provinces": provinces,
        })
    }
}

struct WeatherRegionState {
    region_id: u32,
    flags: u32,
    temperature_raw: i32,
    phenomenon: u32,
    next_weather_change_raw: u32,
    active_modifier_count: u32,
    intensity: u32,
    scale_raw: u32,
}

impl WeatherRegionState {
    fn to_json(&self) -> Value {
        json!({
            "region_id": self.region_id,
            "temperature": self.temperature_raw as f64 / self.scale_raw as f64,
            "temperature_raw": self.temperature_raw,
            "phenomenon": self.phenomenon,
            "next_weather_change_raw": self.next_weather_change_raw,
            "active_modifier_count": self.active_modifier_count,
            "intensity": self.intensity,
            "rain_light": self.flags & WEATHER_REGION_RAIN_LIGHT != 0,
            "rain_heavy": self.flags & WEATHER_REGION_RAIN_HEAVY != 0,
            "snow": self.flags & WEATHER_REGION_SNOW != 0,
            "blizzard": self.flags & WEATHER_REGION_BLIZZARD != 0,
            "arctic_water": self.flags & WEATHER_REGION_ARCTIC_WATER != 0,
            "active_signal": self.flags & WEATHER_REGION_ACTIVE_SIGNAL != 0,
            "phenomenon_active": self.flags & WEATHER_REGION_PHENOMENON_ACTIVE != 0,
            "sandstorm": self.flags & WEATHER_REGION_SANDSTORM != 0,
        })
    }
}

struct WeatherProvinceState {
    province_id: u32,
    flags: u32,
    temperature_raw: i32,
    temperature_change_raw: i32,
    water_raw: i32,
    snow_raw: i32,
    custom_modifier_count: u32,
    scale_raw: u32,
}

impl WeatherProvinceState {
    fn to_json(&self) -> Value {
        json!({
            "province_id": self.province_id,
            "temperature": optional_scaled_i32(self.flags, WEATHER_PROVINCE_TEMP_PRESENT, self.temperature_raw, self.scale_raw),
            "temperature_raw": optional_i32(self.flags, WEATHER_PROVINCE_TEMP_PRESENT, self.temperature_raw),
            "temperature_change": optional_scaled_i32(self.flags, WEATHER_PROVINCE_TEMP_CHANGE_PRESENT, self.temperature_change_raw, self.scale_raw),
            "temperature_change_raw": optional_i32(self.flags, WEATHER_PROVINCE_TEMP_CHANGE_PRESENT, self.temperature_change_raw),
            "water": optional_scaled_i32(self.flags, WEATHER_PROVINCE_WATER_PRESENT, self.water_raw, self.scale_raw),
            "water_raw": optional_i32(self.flags, WEATHER_PROVINCE_WATER_PRESENT, self.water_raw),
            "snow": optional_scaled_i32(self.flags, WEATHER_PROVINCE_SNOW_PRESENT, self.snow_raw, self.scale_raw),
            "snow_raw": optional_i32(self.flags, WEATHER_PROVINCE_SNOW_PRESENT, self.snow_raw),
            "mud_present": self.flags & WEATHER_PROVINCE_MUD_PRESENT != 0,
            "mud_active": if self.flags & WEATHER_PROVINCE_MUD_PRESENT != 0 {
                json!(self.flags & WEATHER_PROVINCE_MUD_ACTIVE != 0)
            } else {
                Value::Null
            },
            "custom_modifier_count": if self.flags & WEATHER_PROVINCE_CUSTOM_MODIFIER_COUNT_PRESENT != 0 {
                json!(self.custom_modifier_count)
            } else {
                Value::Null
            },
            "struct_validated": self.flags & WEATHER_PROVINCE_STRUCT_VALIDATED != 0,
            "map_recolor": self.flags & WEATHER_PROVINCE_MAP_RECOLOR != 0,
            "dynamic_lut": self.flags & WEATHER_PROVINCE_DYNAMIC_LUT != 0,
        })
    }
}

#[derive(Clone)]
struct SpecialProjectsState {
    facilities: BTreeMap<u32, SpecialProjectFacilityState>,
    projects: BTreeMap<u32, SpecialProjectState>,
    scientists: BTreeMap<u32, SpecialProjectScientistState>,
    project_states: BTreeMap<u32, SpecialProjectProgressState>,
    breakthroughs: BTreeMap<(u32, u32), SpecialProjectBreakthroughState>,
    count: u32,
    source: String,
    header_extension: u32,
    /// Wave-5 owner-joined lifecycle: `Some` when the `PLCF` trailer was
    /// present this frame (per-project started/completed keyed by
    /// (tag_db_id, local_id)); `None` when not measured (absent, never empty).
    lifecycle: Option<ProjectLifecycleFold>,
}

/// Wave-5 project lifecycle fold payload: per-project started/completed rows
/// plus the country counters (validated + fail-closed-aborted).
#[derive(Clone)]
struct ProjectLifecycleFold {
    rows: Vec<ProjectLifecycleFoldRow>,
    countries: u32,
    aborted_countries: u32,
}

#[derive(Clone)]
struct ProjectLifecycleFoldRow {
    tag_db_id: u32,
    local_id: u32,
    started: bool,
    completed: bool,
    deleted: bool,
}

impl SpecialProjectsState {
    fn to_json(&self, fold: &WorldFold, unresolved: &mut BTreeSet<u32>) -> Value {
        let mut facilities = Map::new();
        for (id, facility) in &self.facilities {
            facilities.insert(id.to_string(), facility.to_json());
        }
        let mut projects = Map::new();
        for (id, project) in &self.projects {
            projects.insert(id.to_string(), project.to_json());
        }
        let mut scientists = Map::new();
        for (id, scientist) in &self.scientists {
            scientists.insert(id.to_string(), scientist.to_json());
        }
        let mut project_states = Map::new();
        for (id, state) in &self.project_states {
            project_states.insert(id.to_string(), state.to_json());
        }
        // Wave-5 owner-joined lifecycle: per-country started/completed, keyed
        // by resolved country ref -> [{local_id, started, completed}]. `null`
        // when the lifecycle trailer was not measured this frame (absent !=
        // empty).
        let project_lifecycle = match &self.lifecycle {
            Some(lifecycle) => {
                let mut by_country: BTreeMap<u32, Vec<&ProjectLifecycleFoldRow>> = BTreeMap::new();
                for row in &lifecycle.rows {
                    by_country.entry(row.tag_db_id).or_default().push(row);
                }
                let mut countries = Map::new();
                for (tag_db_id, rows) in by_country {
                    let key = fold.resolve_country_key(&tag_db_id.to_string(), unresolved);
                    let entries: Vec<Value> = rows
                        .iter()
                        .map(|row| {
                            json!({
                                "local_id": row.local_id,
                                "started": row.started,
                                "completed": row.completed,
                            })
                        })
                        .collect();
                    countries.insert(key, Value::Array(entries));
                }
                json!({
                    "countries": countries,
                    "row_count": lifecycle.rows.len(),
                    "started_count": lifecycle.rows.iter().filter(|r| r.started).count(),
                    "completed_count": lifecycle.rows.iter().filter(|r| r.completed).count(),
                    "walked_countries": lifecycle.countries,
                    "aborted_countries": lifecycle.aborted_countries,
                })
            }
            None => Value::Null,
        };
        json!({
            "count": self.count,
            "source": self.source,
            "facilities": facilities,
            "projects": projects,
            "scientists": scientists,
            "project_lifecycle": project_lifecycle,
            "_r2_header_extension": self.header_extension,
            "_r2_project_states": project_states,
            "_r2_breakthroughs": self.breakthroughs.values().map(|row| row.to_json(fold, unresolved)).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone)]
struct SpecialProjectFacilityState {
    id: u32,
    province: u32,
    active_project_handle: Option<u32>,
    scientist_char_id: Option<u32>,
    unread_rewards: u32,
    building: Option<String>,
}

impl SpecialProjectFacilityState {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "province": self.province,
            "active_project": Value::Null,
            "scientist_char_id": self.scientist_char_id,
            "unread_rewards": self.unread_rewards,
            "building": self.building,
            "_r2_active_project_handle": self.active_project_handle,
        })
    }
}

#[derive(Clone)]
struct SpecialProjectState {
    id: u32,
    static_def_handle: u32,
    iterations: u32,
    worked_at_facility: Option<u32>,
}

impl SpecialProjectState {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "static_def": Value::Null,
            "iterations": self.iterations,
            "worked_at_facility": self.worked_at_facility,
            "_r2_static_def_handle": self.static_def_handle,
        })
    }
}

#[derive(Clone)]
struct SpecialProjectScientistState {
    char_id: u32,
    trait_count: Option<u32>,
    skill_values: Option<[i32; 4]>,
    name_token: Option<String>,
    display_name: Option<String>,
}

impl SpecialProjectScientistState {
    fn to_json(&self) -> Value {
        json!({
            "char_id": self.char_id,
            "trait_count": self.trait_count,
            "name_token": self.name_token,
            "display_name": self.display_name,
            "_r2_skill_values": self.skill_values,
        })
    }
}

#[derive(Clone)]
struct SpecialProjectProgressState {
    id: u32,
    static_def_handle: u32,
    current_state_offset: u32,
    current_state_enum: u32,
    prototype_state_enum: u32,
    simple_state_1_enum: u32,
    simple_state_2_enum: u32,
    simple_state_3_enum: u32,
    prototype_iterations: u32,
    phase_progress_raw: u32,
    phase_target_raw: u32,
    completed: Option<bool>,
}

impl SpecialProjectProgressState {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "static_def": Value::Null,
            "current_state": Value::Null,
            "completed": self.completed,
            "phase_progress_raw": self.phase_progress_raw,
            "phase_target_raw": self.phase_target_raw,
            "_r2_static_def_handle": self.static_def_handle,
            "_r2_current_state_offset": self.current_state_offset,
            "_r2_current_state_enum": self.current_state_enum,
            "_r2_prototype_state_enum": self.prototype_state_enum,
            "_r2_simple_state_1_enum": self.simple_state_1_enum,
            "_r2_simple_state_2_enum": self.simple_state_2_enum,
            "_r2_simple_state_3_enum": self.simple_state_3_enum,
            "_r2_prototype_iterations": self.prototype_iterations,
        })
    }
}

#[derive(Clone)]
struct SpecialProjectBreakthroughState {
    tag_db_id: u32,
    specialization_handle: u32,
    points_raw: u32,
}

impl SpecialProjectBreakthroughState {
    fn to_json(&self, fold: &WorldFold, unresolved: &mut BTreeSet<u32>) -> Value {
        json!({
            "country": fold.country_ref_json(self.tag_db_id, unresolved),
            "tag_db_id": self.tag_db_id,
            "specialization": Value::Null,
            "points_raw": self.points_raw,
            "_r2_specialization_handle": self.specialization_handle,
        })
    }
}

#[derive(Default)]
struct FrdFocusMetricsState {
    current: Option<String>,
    current_continuous: Option<String>,
    completed_count: Option<u32>,
    completed_tokens: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FrdResearchKey {
    tag_db_id: u32,
    slot_index: u32,
}

struct FrdFocusDepthEventState {
    tag_db_id: u32,
    kind: u32,
    focus_kind: u32,
    flags: u32,
    previous_completed: u32,
    current_completed: u32,
    previous_progress: i32,
    current_progress: i32,
    completed_delta: u32,
    from_token: String,
    to_token: String,
    completed_token: String,
}

impl FrdFocusDepthEventState {
    fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "focus_kind": self.focus_kind,
            "previous_completed": self.previous_completed,
            "current_completed": self.current_completed,
            "previous_progress": self.previous_progress,
            "current_progress": self.current_progress,
            "completed_delta": self.completed_delta,
            "from_token": self.from_token,
            "to_token": self.to_token,
            "completed_token": self.completed_token,
            "completed_token_exact": self.flags & FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT != 0,
            "completed_token_inferred": self.flags & FRD_FOCUS_FLAG_COMPLETED_TOKEN_INFERRED != 0,
        })
    }
}

struct FrdResearchLiveState {
    tag_db_id: u32,
    slot_index: u32,
    flags: u32,
    hour_delta: u32,
    previous_handle: u32,
    current_handle: u32,
    previous_def: u32,
    current_def: u32,
    previous_progress_raw: i64,
    current_progress_raw: i64,
    delta_raw: i64,
    speed_raw: i64,
    base_raw: Option<i64>,
    eta_hours: Option<i64>,
}

impl FrdResearchLiveState {
    fn to_json(&self) -> Value {
        json!({
            "slot_index": self.slot_index,
            "current_handle": self.current_handle,
            "current_def": self.current_def,
            "previous_handle": self.previous_handle,
            "previous_def": self.previous_def,
            "previous_progress_raw": self.previous_progress_raw,
            "current_progress_raw": self.current_progress_raw,
            "delta_raw": self.delta_raw,
            "speed_raw": self.speed_raw,
            "base_raw": self.base_raw,
            "eta_hours": self.eta_hours,
            "hour_delta": self.hour_delta,
            "flags": self.flags,
        })
    }
}

#[derive(Clone, Copy)]
struct MarketTradeDetailFoldRow {
    contract_id: u32,
    flags: u32,
    price_raw: i64,
    amount_raw: i64,
    speed_raw: i32,
    completed_raw: i32,
    days: i32,
    delivery_country_tag_db_id: u32,
    convoys_assigned: i32,
    convoys_total: i32,
    efficiency_raw: i32,
    factory_cic_progress_raw: i32,
    equipment_transfer_progress_raw: i32,
}

#[derive(Default)]
struct MarketTradeTrailersFold {
    detail_rows: BTreeMap<u32, MarketTradeDetailFoldRow>,
    resource_exported: Option<BTreeMap<u32, [i64; 7]>>,
}

struct FrdDecisionStatusState {
    tag_db_id: u32,
    active_timed_count: u32,
    targeted_count: u32,
    targeted_timed_count: u32,
    decisions_taken_bucket_count: u32,
    re_enable_count: u32,
    timed_live_count: u32,
}

impl FrdDecisionStatusState {
    fn to_json(&self) -> Value {
        json!({
            "active_timed_count": self.active_timed_count,
            "targeted_count": self.targeted_count,
            "targeted_timed_count": self.targeted_timed_count,
            "decisions_taken_bucket_count": self.decisions_taken_bucket_count,
            "re_enable_count": self.re_enable_count,
            "timed_live_count": self.timed_live_count,
        })
    }
}

struct FrdDecisionEventState {
    tag_db_id: u32,
    kind: u32,
    previous_days: i32,
    current_days: i32,
    previous_state: i32,
    current_state: i32,
    decision: String,
}

impl FrdDecisionEventState {
    fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "previous_days": self.previous_days,
            "current_days": self.current_days,
            "previous_state": self.previous_state,
            "current_state": self.current_state,
            "decision": self.decision,
        })
    }
}

#[derive(Clone)]
struct FrdDecisionTargetState {
    tag_db_id: u32,
    target_tag_db_id: u32,
    flags: u32,
    source_kind: u32,
    decision: String,
}

impl FrdDecisionTargetState {
    fn to_json(&self, fold: &WorldFold, unresolved: &mut BTreeSet<u32>) -> Value {
        json!({
            "decision": self.decision,
            "target": fold.country_ref_json(self.target_tag_db_id, unresolved),
            "target_tag_db_id": self.target_tag_db_id,
            "flags": self.flags,
            "source_kind": self.source_kind,
            "source": "CDecisionStatus+0xE8 record+0x20",
        })
    }
}

struct FrdDepthBadgeState {
    kind: u32,
    flags: u32,
    source_domain_id: u32,
    affected_count: u32,
    evidence_code: u32,
    detail: String,
}

impl FrdDepthBadgeState {
    fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "flags": self.flags,
            "source_domain_id": self.source_domain_id,
            "affected_count": self.affected_count,
            "evidence_code": self.evidence_code,
            "detail": self.detail,
        })
    }
}

struct FrdDepthFrame {
    depth_status_flags: u32,
    focus_rows: Vec<FrdFocusDepthEventState>,
    research_rows: Vec<FrdResearchLiveState>,
    decision_status_rows: Vec<FrdDecisionStatusState>,
    decision_event_rows: Vec<FrdDecisionEventState>,
    decision_target_rows: Vec<FrdDecisionTargetState>,
    decision_target_omitted_rows: u32,
    badges: Vec<FrdDepthBadgeState>,
}

#[derive(Clone)]
struct ScriptFlagState {
    value_raw: Option<i64>,
    date_hour_raw: Option<i32>,
}

impl ScriptFlagState {
    fn to_json(&self) -> Value {
        json!({
            "value_raw": self.value_raw,
            "date_hour_raw": self.date_hour_raw,
        })
    }
}

struct ScriptedOpaqueBadgeState {
    value_kind: u32,
    scope_kind: u32,
    owner_id: u32,
    key: Option<String>,
}

impl ScriptedOpaqueBadgeState {
    fn to_json(&self) -> Value {
        json!({
            "value_kind": self.value_kind,
            "scope_kind": self.scope_kind,
            "owner_id": self.owner_id,
            "key": self.key,
        })
    }
}

/// Wave-3 measured pending delayed-event fold state.
struct DelayedEventFoldState {
    owner_tag_db_id: u32,
    combined_event_id: u32,
    scope_tag_db_id: u32,
    scope_random: u64,
    /// CDelayedEvent+0xC4 total remaining hours (months*720 + days*24 +
    /// hours). None = the field read was torn/out-of-bounds and badged absent.
    due_remaining_hours: Option<u32>,
}

impl DelayedEventFoldState {
    fn to_json(&self) -> Value {
        // Decompose the remaining-hours countdown into 30-day months / days /
        // hours for readability when present; null when the read was absent.
        let (due_months, due_days, due_hours) = match self.due_remaining_hours {
            Some(total) => (
                json!(total / 720),
                json!((total % 720) / 24),
                json!(total % 24),
            ),
            None => (Value::Null, Value::Null, Value::Null),
        };
        json!({
            "owner_tag_db_id": self.owner_tag_db_id,
            "event_id": self.combined_event_id,
            "event_namespace_index": self.combined_event_id / 100_000,
            "event_index": self.combined_event_id % 100_000,
            "scope_tag_db_id": self.scope_tag_db_id,
            "scope_random_seed": self.scope_random.to_string(),
            // Validated remaining-hours countdown (CDelayedEvent+0xC4, 17/17
            // oracle). Absent (null) only on a torn/out-of-bounds read.
            "due_remaining_hours": self
                .due_remaining_hours
                .map(|h| json!(h))
                .unwrap_or(Value::Null),
            "due_remaining_months": due_months,
            "due_remaining_days": due_days,
            "due_remaining_hours_component": due_hours,
        })
    }
}

struct ScriptedStateRow {
    scope_kind: u32,
    owner_id: u32,
    value_kind: u32,
    key: Option<String>,
    token_id: Option<u32>,
    date_hour_raw: Option<i32>,
    value_raw: Option<i64>,
    deleted: bool,
    delayed_due_hours: Option<u32>,
}

#[derive(Clone)]
struct DomainProblem {
    domain_id: u16,
    problem: &'static str,
}

struct MapDetailStateRow {
    state_id: u32,
    flags: u32,
    resource_first: usize,
    resource_len: usize,
    building_first: usize,
    building_len: usize,
}

struct MapDetailProvinceRow {
    province_id: u32,
    state_id: u32,
    flags: u32,
    building_first: usize,
    building_len: usize,
}

struct MapDetailI32Entry {
    type_id: u32,
    amount_raw: i32,
}

struct MapDetailU32Entry {
    type_id: u32,
    level: u32,
}

fn decode_army_battleplans_depth_payload(
    payload: &[u8],
) -> Result<DecodedArmyBattleplansDepthPayload, String> {
    need(
        payload,
        battleplans54::HEADER_LEN,
        "army_battleplans_depth_live header",
    )?;
    let row_count = read_u32(payload, 8) as usize;
    let status_flags = read_u32(payload, 16);
    let target_count = read_u32(payload, 28) as usize;
    let omitted_non_exact_rows = read_u32(payload, 40) as u64;
    let path_geometry_omitted_rows = read_u32(payload, 44) as u64;
    let path_count = read_u32(payload, 48) as usize;
    let rows_end =
        battleplans54::HEADER_LEN.saturating_add(row_count.saturating_mul(battleplans54::ROW_LEN));
    let target_table_start = rows_end;
    let path_table_start = target_table_start
        .saturating_add(target_count.saturating_mul(battleplans54::TARGET_ROW_LEN));
    let expected_len = path_table_start
        .saturating_add(path_count.saturating_mul(battleplans54::PATH_GEOMETRY_ROW_LEN));
    if payload.len() != expected_len {
        return Err(format!(
            "army_battleplans_depth_live len {} != {expected_len}",
            payload.len()
        ));
    }

    let mut rows = Vec::with_capacity(row_count);
    let mut declared_targets = 0usize;
    let mut declared_paths = 0usize;
    for row_index in 0..row_count {
        let base = battleplans54::HEADER_LEN + row_index * battleplans54::ROW_LEN;
        let army_id = read_u32(payload, base);
        let generation = read_u64(payload, base + 4);
        let flags = read_u32(payload, base + 12);
        let slot_index = read_u32(payload, base + 16);
        let order_type = read_i32(payload, base + 20);
        let child_first = read_u32(payload, base + 24) as usize;
        let child_count = read_u32(payload, base + 28) as usize;
        let target_declared = read_u32(payload, base + 32);
        let target_status = read_u32(payload, base + 36);
        let geometry_status = read_u32(payload, base + 40);
        let execution_status = read_u32(payload, base + 44);
        let assigned_units_status = read_u32(payload, base + 48);
        let order_linkage_status = read_u32(payload, base + 52);
        let stance_status = read_u32(payload, base + 56);
        let aggressiveness_status = read_u32(payload, base + 60);

        let deleted = flags & battleplans54::FLAG_DELETED != 0;
        let has_targets = flags & battleplans54::FLAG_TARGET_STATES != 0;
        let has_path = flags & battleplans54::FLAG_PATH_GEOMETRY != 0;
        if has_targets && has_path {
            return Err(format!(
                "army_battleplans_depth_live row {row_index} has both target and path children"
            ));
        }
        let target_states = if has_targets {
            let end = child_first
                .checked_add(child_count)
                .ok_or_else(|| "army_battleplans_depth_live target range overflow".to_string())?;
            if end > target_count {
                return Err(format!(
                    "army_battleplans_depth_live row {row_index} target range exceeds child table"
                ));
            }
            declared_targets = declared_targets.saturating_add(child_count);
            read_depth_child_table(
                payload,
                target_table_start,
                child_first,
                child_count,
                battleplans54::TARGET_ROW_LEN,
            )
        } else {
            Vec::new()
        };
        let path_provinces = if has_path {
            let end = child_first
                .checked_add(child_count)
                .ok_or_else(|| "army_battleplans_depth_live path range overflow".to_string())?;
            if end > path_count {
                return Err(format!(
                    "army_battleplans_depth_live row {row_index} path range exceeds child table"
                ));
            }
            declared_paths = declared_paths.saturating_add(child_count);
            read_depth_child_table(
                payload,
                path_table_start,
                child_first,
                child_count,
                battleplans54::PATH_GEOMETRY_ROW_LEN,
            )
        } else {
            Vec::new()
        };
        let target_declared_count =
            (flags & battleplans54::FLAG_TARGET_STATE_COUNT != 0).then_some(target_declared);

        rows.push(DecodedArmyBattleplansDepthRow {
            deleted,
            order: ArmyBattleplanDepthOrder {
                key: ArmyBattleplanDepthKey {
                    army_id,
                    generation,
                },
                slot_index,
                order_type,
                target_states,
                path_provinces,
                target_declared_count,
                target_status: battleplans_depth_status_name(target_status),
                geometry_status: battleplans_depth_status_name(geometry_status),
                execution_status: battleplans_depth_status_name(execution_status),
                assigned_units_status: battleplans_depth_status_name(assigned_units_status),
                order_linkage_status: battleplans_depth_status_name(order_linkage_status),
                stance_status: battleplans_depth_status_name(stance_status),
                aggressiveness_status: battleplans_depth_status_name(aggressiveness_status),
            },
        });
    }
    if declared_targets != target_count {
        return Err("army_battleplans_depth_live target child count mismatch".into());
    }
    if declared_paths != path_count {
        return Err("army_battleplans_depth_live path child count mismatch".into());
    }
    Ok(DecodedArmyBattleplansDepthPayload {
        is_delta: status_flags & COMPACT_STATUS_DELTA_DATA != 0,
        omitted_non_exact_rows,
        path_geometry_omitted_rows,
        rows,
    })
}

fn read_depth_child_table(
    payload: &[u8],
    table_start: usize,
    first: usize,
    count: usize,
    row_len: usize,
) -> Vec<u32> {
    (0..count)
        .map(|index| read_u32(payload, table_start + (first + index) * row_len))
        .collect()
}

fn battleplans_depth_status_name(status: u32) -> &'static str {
    match status {
        battleplans54::STATUS_EXACT => "exact",
        battleplans54::STATUS_UNKNOWN => "unknown",
        battleplans54::STATUS_UNSUPPORTED => "unsupported",
        battleplans54::STATUS_OVERFLOW => "overflow",
        battleplans54::STATUS_VALIDATION_FAILED => "validation_failed",
        battleplans54::STATUS_FIXTURE_REQUIRED => "fixture_required",
        battleplans54::STATUS_COLD_PREFLIGHT_ONLY => "cold_preflight_only",
        _ => "none",
    }
}

/// Merge `add` into `country[key]`, preserving keys already written by another
/// domain (politics_live and politics_residuals both write `politics`).
fn merge_into(country: &mut Map<String, Value>, key: &str, add: Map<String, Value>) {
    match country.get_mut(key) {
        Some(Value::Object(existing)) => {
            for (k, v) in add {
                existing.insert(k, v);
            }
        }
        _ => {
            country.insert(key.into(), Value::Object(add));
        }
    }
}

fn string_or_null(s: &str) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        json!(s)
    }
}

fn option_u32_json(value: Option<u32>) -> Value {
    value.map(|item| json!(item)).unwrap_or(Value::Null)
}

fn domain48_upsert_construction_queue(
    extra: &mut Map<String, Value>,
    rows: Vec<Value>,
    expected_partial: bool,
) {
    if !expected_partial {
        extra.insert("construction_queue".into(), Value::Array(rows));
        return;
    }

    // D48 compact ConstructionQueueRow carries line_id, emitted as `id`, from
    // the live construction line. On tolerated partial frames this is the stable
    // child key: upsert emitted lines and retain non-emitted siblings.
    let mut pending = rows
        .into_iter()
        .filter_map(|row| domain48_construction_queue_key(&row).map(|key| (key, row)))
        .collect::<BTreeMap<_, _>>();
    let mut merged = match extra.remove("construction_queue") {
        Some(Value::Array(existing)) => existing,
        _ => Vec::new(),
    };
    for row in &mut merged {
        if let Some(key) = domain48_construction_queue_key(row) {
            if let Some(update) = pending.remove(&key) {
                *row = update;
            }
        }
    }
    merged.extend(pending.into_values());
    extra.insert("construction_queue".into(), Value::Array(merged));
}

fn domain48_construction_queue_key(row: &Value) -> Option<u32> {
    let id = row.as_object()?.get("id")?;
    id.as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| id.as_str()?.parse::<u32>().ok())
}

fn domain48_upsert_building_rows_by_type(
    extra: &mut Map<String, Value>,
    key: &str,
    rows: Vec<domain48::BuildingRow>,
    expected_partial: bool,
) {
    let Value::Object(updates) = domain48_building_rows_by_type(rows) else {
        return;
    };
    if !expected_partial {
        extra.insert(key.into(), Value::Object(updates));
        return;
    }

    // D48 building rows are scoped by their owning container already
    // (state_id or province location_id), so type_id is the stable child key.
    let existing = ensure_json_object_mut(extra, key);
    for (type_key, detail) in updates {
        existing.insert(type_key, detail);
    }
}

fn domain48_building_rows_by_type(rows: Vec<domain48::BuildingRow>) -> Value {
    let mut obj = Map::new();
    for row in rows {
        let mut detail = Map::new();
        detail.insert("level".into(), json!(row.level));
        detail.insert("healthy_level".into(), option_u32_json(row.healthy_level));
        detail.insert(
            "damaged".into(),
            json!(row.healthy_level.is_some_and(|healthy| healthy < row.level)),
        );
        obj.insert(domain48_type_key(row.type_id), Value::Object(detail));
    }
    Value::Object(obj)
}

fn domain48_type_key(type_id: u32) -> String {
    format!("type_0x{type_id:08x}")
}

fn domain48_resource_key(resource_id: u32) -> String {
    format!("tid_0x{resource_id:08x}")
}

fn domain48_construction_key(row: &domain48::ConstructionQueueRow) -> Domain48ConstructionKey {
    (row.tag_db_id, row.line_id & !domain48::ROW_ID_DELETED)
}

fn domain48_building_key(row: &domain48::BuildingRow) -> Domain48BuildingKey {
    (
        row.scope,
        row.location_id,
        row.state_id,
        row.type_id & !domain48::ROW_ID_DELETED,
    )
}

fn domain48_resource_identity_key(row: &domain48::StateResourceRow) -> Domain48ResourceKey {
    (row.state_id, row.resource_id & !domain48::ROW_ID_DELETED)
}

fn option_flag(flags: u32, flag: u32, value: u32) -> Option<u32> {
    (flags & flag != 0).then_some(value)
}

/// D60 per-country capitulation JSON. Date raws use the engine hour-counter
/// encoding; the engine null sentinel (and only it — decode range-gates
/// everything else) renders as JSON null. Legitimacy is fixed-point x 1e5.
fn capitulation_country_json(row: &capwar60::compact::CountryCapitulationRow) -> Value {
    let date_or_null = |raw: i32| -> Value {
        compact_contract::calendar::date_from_raw(raw)
            .map(Value::from)
            .unwrap_or(Value::Null)
    };
    json!({
        "capitulated": row.capitulated,
        "capitulated_date": date_or_null(row.capitulated_date_raw),
        "last_surrender_date": date_or_null(row.last_surrender_date_raw),
        "faction_join_date": date_or_null(row.faction_join_date_raw),
        "legitimacy": (f64::from(row.legitimacy_raw))
            / f64::from(capwar60::compact::FIXED_SCALE_RAW),
        "legitimacy_raw": row.legitimacy_raw,
        "exile_army_leaders": row.exile_army_leaders,
    })
}

fn domain53_relation_flag_name(flag_kind: u16) -> &'static str {
    match flag_kind {
        diplomacy53::compact::REL_KIND_NON_AGGRESSION_PACT => "non_aggression_pact",
        diplomacy53::compact::REL_KIND_PUPPET => "subject_relation",
        diplomacy53::compact::REL_KIND_EMBARGO => "embargo",
        diplomacy53::compact::REL_KIND_MILITARY_ACCESS => "military_access",
        diplomacy53::compact::REL_KIND_AIR_BASE_ACCESS => "air_base_access",
        diplomacy53::compact::REL_KIND_DOCKING_RIGHTS => "docking_rights",
        diplomacy53::compact::REL_KIND_NAVAL_BLOCKADE => "naval_blockade",
        diplomacy53::compact::REL_KIND_MARKET_ACCESS_RIGHTS => "market_access_rights",
        diplomacy53::compact::REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION => {
            "equipment_purchase_contract_relation"
        }
        diplomacy53::compact::REL_KIND_GUARANTEE => "guarantee",
        diplomacy53::compact::REL_KIND_IMPROVE_RELATIONS => "improve_relations",
        diplomacy53::compact::REL_KIND_ATTACHE => "attache",
        diplomacy53::compact::REL_KIND_LICENCE_PRODUCTION => "licence_production",
        _ => "unknown",
    }
}

fn option_u32_vec_json(value: &Option<Vec<u32>>) -> Value {
    match value {
        Some(items) => Value::Array(items.iter().copied().map(|item| json!(item)).collect()),
        None => Value::Null,
    }
}

fn scaled_value_or_null(flags: u32, flag: u32, raw: i32) -> Value {
    if flags & flag != 0 {
        json!(raw as f64 / FIXED_SCALE)
    } else {
        Value::Null
    }
}

fn supply_logistics_deleted_id(entity_id: u32) -> Option<u32> {
    (entity_id & SUPPLY_ROW_ID_DELETED != 0).then_some(entity_id & !SUPPLY_ROW_ID_DELETED)
}

fn validate_supply_logistics_tombstone(
    family: &str,
    row_index: usize,
    join_id: u32,
    owner_tag_db_id: u32,
    flags: u32,
    values: [i32; 5],
) -> Result<(), String> {
    if join_id != 0 || owner_tag_db_id != 0 {
        return Err(format!(
            "supply {family} tombstone row {row_index} has live identity fields"
        ));
    }
    if flags != 0 || values.iter().any(|value| *value != 0) {
        return Err(format!(
            "supply {family} tombstone row {row_index} has live fields"
        ));
    }
    Ok(())
}

fn render_supply_logistics_state(state: &SupplyLogisticsState) -> Map<String, Value> {
    let mut by_province = Map::new();
    for row in state.provinces.values() {
        let province = ensure_json_object_mut(&mut by_province, &row.province_id.to_string());
        ensure_supply_province_defaults(province);
        let mut buildings = Map::new();
        for (flag, key, value) in [
            (SUPPLY_ROW_FLAG_RAIL, "rail_way", row.rail),
            (SUPPLY_ROW_FLAG_SUPPLY_NODE, "supply_node", row.supply_node),
            (
                SUPPLY_ROW_FLAG_NAVAL_SUPPLY_HUB,
                "naval_supply_hub",
                row.naval_supply_hub,
            ),
            (
                SUPPLY_ROW_FLAG_INFRASTRUCTURE,
                "infrastructure",
                row.infrastructure,
            ),
        ] {
            if row.flags & flag != 0 {
                buildings.insert(key.into(), json!(value));
            }
        }
        province.insert("logistics_buildings".into(), Value::Object(buildings));
        province.insert("state_id".into(), json!(row.state_id));
        if row.building_count != 0 {
            province.insert("logistics_building_count".into(), json!(row.building_count));
        }
    }

    for row in state.divisions.values() {
        let province = ensure_json_object_mut(&mut by_province, &row.province_id.to_string());
        ensure_supply_province_defaults(province);
        let mut division = Map::new();
        division.insert("raw_division_id".into(), json!(row.raw_division_id));
        division.insert(
            "owner".into(),
            row.owner_tag_db_id
                .map(|owner| json!(owner.to_string()))
                .unwrap_or(Value::Null),
        );
        division.insert(
            "current_supply".into(),
            scaled_value_or_null(
                row.flags,
                SUPPLY_ROW_FLAG_DIVISION_CURRENT_SUPPLY,
                row.current_supply_raw,
            ),
        );
        division.insert(
            "max_supply".into(),
            scaled_value_or_null(
                row.flags,
                SUPPLY_ROW_FLAG_DIVISION_MAX_SUPPLY,
                row.max_supply_raw,
            ),
        );
        division.insert(
            "supply_gain".into(),
            scaled_value_or_null(
                row.flags,
                SUPPLY_ROW_FLAG_DIVISION_SUPPLY_GAIN,
                row.supply_gain_raw,
            ),
        );
        division.insert(
            "supply_deficit".into(),
            scaled_value_or_null(
                row.flags,
                SUPPLY_ROW_FLAG_DIVISION_SUPPLY_DEFICIT,
                row.supply_deficit_raw,
            ),
        );
        division.insert(
            "fuel".into(),
            scaled_value_or_null(row.flags, SUPPLY_ROW_FLAG_DIVISION_FUEL, row.fuel_raw),
        );
        division.insert(
            "r2_logistics_attrition_live".into(),
            json!({
                "raw_division_id_source": "compact_row_entity_id",
                "proxy_value": true,
                "flags": row.flags,
            }),
        );
        ensure_json_array_mut(province, "divisions").push(Value::Object(division));
    }

    for (province_id, raw) in &state.csupply_inputs {
        let province = ensure_json_object_mut(&mut by_province, &province_id.to_string());
        ensure_supply_province_defaults(province);
        province.insert("csupply_input".into(), json!(*raw as f64 / FIXED_SCALE));
    }

    let badges = state
        .badges
        .iter()
        .map(|(flags, code)| {
            let badge = if flags & SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL != 0 {
                "exact_supply_wall"
            } else if flags & SUPPLY_ROW_FLAG_ATTRITION_WALL != 0 {
                "attrition_wall"
            } else if flags & SUPPLY_ROW_FLAG_MOTORIZATION_WALL != 0 {
                "motorization_wall"
            } else if flags & SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL != 0 {
                "convoy_pressure_wall"
            } else {
                "csupply_candidate_rejected"
            };
            json!({ "badge": badge, "code": code })
        })
        .collect::<Vec<_>>();

    let mut supply = Map::new();
    supply.insert("by_province".into(), Value::Object(by_province));
    supply.insert("source".into(), json!("M"));
    supply.insert("confidence".into(), json!("division_province_proxy"));
    supply.insert(
        "r2_logistics_attrition".into(),
        json!({
            "badges": badges,
            "observed_province_roots": state.observed_province_roots,
            "observed_division_roots": state.observed_division_roots,
        }),
    );
    supply
}

fn domain47_unit_leader_kind(kind: u8) -> &'static str {
    match kind {
        domain47::UNIT_LEADER_ARMY => "army",
        domain47::UNIT_LEADER_NAVY => "navy",
        _ => "unknown",
    }
}

fn ensure_json_object_mut<'a>(
    obj: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !matches!(obj.get(key), Some(Value::Object(_))) {
        obj.insert(key.into(), Value::Object(Map::new()));
    }
    match obj.get_mut(key) {
        Some(Value::Object(map)) => map,
        _ => unreachable!("object was just inserted"),
    }
}

fn ensure_json_array_mut<'a>(obj: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    if !matches!(obj.get(key), Some(Value::Array(_))) {
        obj.insert(key.into(), Value::Array(Vec::new()));
    }
    match obj.get_mut(key) {
        Some(Value::Array(list)) => list,
        _ => unreachable!("array was just inserted"),
    }
}

fn division_equipment_join_key(division: &Map<String, Value>) -> Option<DivisionEquipmentKey> {
    let div_id = division.get("div_id")?.as_u64()? as u32;
    let owner = division.get("owner")?.as_str()?.parse::<u32>().ok()?;
    Some(DivisionEquipmentKey {
        owner_tag_db_id: owner,
        div_id,
    })
}

fn ensure_country_projection_object<'a>(
    countries: &'a mut Map<String, Value>,
    fold: &WorldFold,
    tag_db_id: u32,
    unresolved: &mut BTreeSet<u32>,
) -> &'a mut Map<String, Value> {
    let key = fold.resolve_country_key(&tag_db_id.to_string(), unresolved);
    let value = countries
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    match value {
        Value::Object(country) => country,
        _ => unreachable!("country projection was normalized to object"),
    }
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn mask_scripted_country_extra(country: &mut Map<String, Value>) {
    let Some(Value::Object(extra)) = country.get_mut("extra") else {
        return;
    };
    for key in [
        "script_flags",
        "_r2_script_variables",
        "_r2_token_only_flags",
    ] {
        if extra.contains_key(key) {
            extra.insert(key.into(), Value::Null);
        }
    }
}

fn clear_scripted_country_extra(country: &mut Map<String, Value>) {
    let Some(Value::Object(extra)) = country.get_mut("extra") else {
        return;
    };
    for key in [
        "script_flags",
        "_r2_script_variables",
        "_r2_token_only_flags",
    ] {
        extra.remove(key);
    }
    if extra.is_empty() {
        country.remove("extra");
    }
}

fn clear_country_extra_key(country: &mut Map<String, Value>, section: &str, key: &str) {
    let Some(Value::Object(extra)) = country.get_mut("extra") else {
        return;
    };
    if let Some(Value::Object(section_obj)) = extra.get_mut(section) {
        section_obj.remove(key);
    }
}

fn clear_country_extra_u32_key(country: &mut Map<String, Value>, section: &str, key: u32) {
    clear_country_extra_key(country, section, &key.to_string());
}

fn clear_state_scripted_key(state: &mut Map<String, Value>, section: &str, key: &str) {
    let Some(Value::Object(scripted)) = state.get_mut("_r2_scripted_state") else {
        return;
    };
    if let Some(Value::Object(section_obj)) = scripted.get_mut(section) {
        section_obj.remove(key);
    }
}

fn clear_state_scripted_u32_key(state: &mut Map<String, Value>, section: &str, key: u32) {
    clear_state_scripted_key(state, section, &key.to_string());
}

fn script_flag_map_json(values: &BTreeMap<String, ScriptFlagState>) -> Map<String, Value> {
    values
        .iter()
        .map(|(key, flag)| (key.clone(), flag.to_json()))
        .collect()
}

fn f64_map_json(values: &BTreeMap<String, f64>) -> Map<String, Value> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), json!(value)))
        .collect()
}

fn token_flag_map_json(values: &BTreeMap<u32, ScriptFlagState>) -> Map<String, Value> {
    values
        .iter()
        .map(|(key, flag)| (key.to_string(), flag.to_json()))
        .collect()
}

fn optional_scaled_i32(flags: u32, bit: u32, raw: i32, scale_raw: u32) -> Value {
    if flags & bit != 0 {
        json!(raw as f64 / scale_raw as f64)
    } else {
        Value::Null
    }
}

fn optional_i32(flags: u32, bit: u32, raw: i32) -> Value {
    if flags & bit != 0 {
        json!(raw)
    } else {
        Value::Null
    }
}

fn validate_weather_i32_optional(
    row_index: usize,
    present: bool,
    raw: i32,
    min: i32,
    max: i32,
    label: &str,
) -> Result<(), String> {
    if present {
        if !(min..=max).contains(&raw) {
            return Err(format!(
                "weather province row {row_index} has invalid {label} {raw}"
            ));
        }
    } else if raw != 0 {
        return Err(format!(
            "weather province row {row_index} has {label} without presence flag"
        ));
    }
    Ok(())
}

/// Wave-5 project lifecycle: parse the domain-41 `PLCF` trailer into
/// per-project started/completed rows keyed by (tag_db_id, local_id). Strict:
/// the trailer must consume the payload to its exact end, flags are STARTED
/// bit0 / COMPLETED bit1 (higher bits fail), and (tag, local_id) must be
/// unique (fail closed). Returns `Some(fold)` on success.
fn parse_project_lifecycle_trailer(
    payload: &[u8],
    trailer_offset: usize,
) -> Result<Option<ProjectLifecycleFold>, String> {
    const MAGIC: &[u8; 4] = b"PLCF";
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 12;
    const FLAG_STARTED: u32 = 1;
    const FLAG_COMPLETED: u32 = 1 << 1;
    const FLAG_DELETED: u32 = 1 << 31;
    let trailer = payload
        .get(trailer_offset..)
        .ok_or_else(|| "project lifecycle trailer offset beyond payload".to_string())?;
    if trailer.len() < HEADER_LEN {
        return Err(format!(
            "project lifecycle trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
    }
    if &trailer[0..4] != MAGIC {
        return Err("project lifecycle trailer magic is invalid".to_string());
    }
    let version = read_u16(trailer, 4);
    let header_len = read_u16(trailer, 6) as usize;
    let row_count = read_u32(trailer, 8) as usize;
    let countries = read_u32(trailer, 12);
    let aborted_countries = read_u32(trailer, 16);
    if version != 1 || header_len != HEADER_LEN {
        return Err(format!(
            "project lifecycle trailer header shape invalid (version {version}, header_len {header_len})"
        ));
    }
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if trailer.len() != expected {
        return Err(format!(
            "project lifecycle trailer len {} != {expected}",
            trailer.len()
        ));
    }
    let mut cursor = HEADER_LEN;
    let mut rows = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    for _ in 0..row_count {
        let tag_db_id = read_u32(trailer, cursor);
        let local_id = read_u32(trailer, cursor + 4);
        let flags = read_u32(trailer, cursor + 8);
        cursor += ROW_LEN;
        if tag_db_id == 0 {
            return Err("project lifecycle row has zero tag_db_id".to_string());
        }
        if local_id == 0 {
            return Err("project lifecycle row has zero local_id".to_string());
        }
        if flags & !(FLAG_STARTED | FLAG_COMPLETED | FLAG_DELETED) != 0 {
            return Err(format!(
                "project lifecycle row has unknown flags {flags:#x}"
            ));
        }
        let deleted = flags & FLAG_DELETED != 0;
        if deleted && flags != FLAG_DELETED {
            return Err("project lifecycle deleted row has live flags".to_string());
        }
        if !seen.insert((tag_db_id, local_id)) {
            return Err(format!(
                "project lifecycle trailer duplicates (tag {tag_db_id}, local_id {local_id})"
            ));
        }
        rows.push(ProjectLifecycleFoldRow {
            tag_db_id,
            local_id,
            started: flags & FLAG_STARTED != 0,
            completed: flags & FLAG_COMPLETED != 0,
            deleted,
        });
    }
    Ok(Some(ProjectLifecycleFold {
        rows,
        countries,
        aborted_countries,
    }))
}

fn apply_project_lifecycle_delta(
    previous: Option<ProjectLifecycleFold>,
    delta: Option<ProjectLifecycleFold>,
) -> Result<Option<ProjectLifecycleFold>, String> {
    let Some(delta) = delta else {
        return Ok(previous);
    };
    let mut rows_by_key = BTreeMap::new();
    if let Some(previous) = previous {
        for row in previous.rows {
            rows_by_key.insert((row.tag_db_id, row.local_id), row);
        }
    }
    for row in delta.rows {
        let key = (row.tag_db_id, row.local_id);
        if row.deleted {
            rows_by_key.remove(&key);
        } else {
            rows_by_key.insert(key, row);
        }
    }
    Ok(Some(ProjectLifecycleFold {
        rows: rows_by_key.into_values().collect(),
        countries: delta.countries,
        aborted_countries: delta.aborted_countries,
    }))
}

/// Wave-4 production licensing: parse the domain-11 `PLIC` license trailer
/// into {tag_db_id -> sorted, deduped license variant db ids}. Strict: the
/// trailer must consume the payload to its exact end, ids are gated 1..2^24,
/// and per-country/country dupes fail the whole trailer (fail closed).
fn parse_production_license_trailer(
    payload: &[u8],
    trailer_offset: usize,
) -> Result<BTreeMap<u32, Vec<u32>>, String> {
    const MAGIC: &[u8; 4] = b"PLIC";
    const HEADER_LEN: usize = 20;
    let trailer = payload
        .get(trailer_offset..)
        .ok_or_else(|| "production license trailer offset beyond payload".to_string())?;
    if trailer.len() < HEADER_LEN {
        return Err(format!(
            "production license trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
    }
    if &trailer[0..4] != MAGIC {
        return Err("production license trailer magic is invalid".to_string());
    }
    let version = read_u16(trailer, 4);
    let header_len = read_u16(trailer, 6) as usize;
    let country_rows = read_u32(trailer, 8) as usize;
    let id_rows = read_u32(trailer, 12) as usize;
    let reserved = read_u32(trailer, 16);
    if version != 1 || header_len != HEADER_LEN || reserved != 0 {
        return Err(format!(
            "production license trailer header shape invalid (version {version}, header_len {header_len}, reserved {reserved})"
        ));
    }
    let expected = HEADER_LEN
        .saturating_add(country_rows.saturating_mul(8))
        .saturating_add(id_rows.saturating_mul(4));
    if trailer.len() != expected {
        return Err(format!(
            "production license trailer len {} != {expected}",
            trailer.len()
        ));
    }
    let mut cursor = HEADER_LEN;
    let mut counts: Vec<(u32, usize)> = Vec::with_capacity(country_rows);
    let mut seen_country = BTreeSet::new();
    let mut total_ids = 0usize;
    for _ in 0..country_rows {
        let tag_db_id = read_u32(trailer, cursor);
        let count = read_u32(trailer, cursor + 4) as usize;
        cursor += 8;
        if tag_db_id == 0 {
            return Err("production license country row has zero tag_db_id".to_string());
        }
        if !seen_country.insert(tag_db_id) {
            return Err(format!(
                "production license trailer duplicates tag_db_id {tag_db_id}"
            ));
        }
        total_ids = total_ids.saturating_add(count);
        counts.push((tag_db_id, count));
    }
    if total_ids != id_rows {
        return Err(format!(
            "production license id total {total_ids} != id_rows {id_rows}"
        ));
    }
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (tag_db_id, count) in counts {
        let mut ids = Vec::with_capacity(count);
        let mut seen_ids = BTreeSet::new();
        for _ in 0..count {
            let variant_id = read_u32(trailer, cursor);
            cursor += 4;
            if variant_id == 0 || variant_id >= (1 << 24) {
                return Err(format!(
                    "production license id {variant_id} for tag {tag_db_id} out of range"
                ));
            }
            if !seen_ids.insert(variant_id) {
                return Err(format!(
                    "production license tag {tag_db_id} duplicates variant_id {variant_id}"
                ));
            }
            ids.push(variant_id);
        }
        ids.sort_unstable();
        out.insert(tag_db_id, ids);
    }
    Ok(out)
}

fn parse_market_trade_trailers(
    payload: &[u8],
    mut trailer_offset: usize,
) -> Result<MarketTradeTrailersFold, String> {
    let mut out = MarketTradeTrailersFold::default();
    let mut detail_seen = false;
    while trailer_offset < payload.len() {
        need(payload, trailer_offset + 4, "market trade trailer magic")?;
        let magic = &payload[trailer_offset..trailer_offset + 4];
        if magic == MARKET_TRADE_DETAIL_TRAILER_MAGIC {
            if detail_seen {
                return Err("market trade has duplicate MTD1 detail trailer".to_string());
            }
            let (detail_rows, next_offset) =
                parse_market_trade_detail_trailer(payload, trailer_offset)?;
            out.detail_rows = detail_rows;
            detail_seen = true;
            trailer_offset = next_offset;
        } else if magic == MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC {
            if out.resource_exported.is_some() {
                return Err("market trade has duplicate MTRF resource-flow trailer".to_string());
            }
            let (resource_exported, next_offset) =
                parse_market_trade_resource_flow_trailer(payload, trailer_offset)?;
            out.resource_exported = Some(resource_exported);
            trailer_offset = next_offset;
        } else {
            return Err("market trade trailing bytes are not a known D43 trailer".to_string());
        }
    }
    Ok(out)
}

fn parse_market_trade_detail_trailer(
    payload: &[u8],
    trailer_offset: usize,
) -> Result<(BTreeMap<u32, MarketTradeDetailFoldRow>, usize), String> {
    if payload.get(trailer_offset..trailer_offset.saturating_add(4))
        != Some(MARKET_TRADE_DETAIL_TRAILER_MAGIC.as_slice())
    {
        return Err("market trade trailing bytes are not MTD1 detail trailer".to_string());
    }
    need(
        payload,
        trailer_offset + MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN,
        "market trade MTD1 header",
    )?;
    let version = read_u16(payload, trailer_offset + 4);
    let header_len = read_u16(payload, trailer_offset + 6) as usize;
    let row_len = read_u16(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16(payload, trailer_offset + 10);
    let row_count = read_u32(payload, trailer_offset + 12) as usize;
    if version != MARKET_TRADE_DETAIL_TRAILER_VERSION
        || header_len != MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN
        || row_len != MARKET_TRADE_DETAIL_ROW_LEN
        || reserved0 != 0
    {
        return Err("market trade MTD1 header shape is invalid".to_string());
    }
    let expected = trailer_offset
        .saturating_add(header_len)
        .saturating_add(row_count.saturating_mul(row_len));
    if payload.len() < expected {
        return Err(format!(
            "market trade MTD1 len {} < {expected}",
            payload.len()
        ));
    }

    let mut rows = BTreeMap::new();
    let mut cursor = trailer_offset + header_len;
    for row_index in 0..row_count {
        let row = MarketTradeDetailFoldRow {
            contract_id: read_u32(payload, cursor),
            flags: read_u32(payload, cursor + 4),
            price_raw: read_i64(payload, cursor + 8),
            amount_raw: read_i64(payload, cursor + 16),
            speed_raw: read_i32(payload, cursor + 24),
            completed_raw: read_i32(payload, cursor + 28),
            days: read_i32(payload, cursor + 32),
            delivery_country_tag_db_id: read_u32(payload, cursor + 36),
            convoys_assigned: read_i32(payload, cursor + 40),
            convoys_total: read_i32(payload, cursor + 44),
            efficiency_raw: read_i32(payload, cursor + 48),
            factory_cic_progress_raw: read_i32(payload, cursor + 52),
            equipment_transfer_progress_raw: read_i32(payload, cursor + 56),
        };
        let reserved = read_u32(payload, cursor + 60);
        cursor += row_len;

        if row.contract_id == 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} has zero contract_id"
            ));
        }
        if row.flags == 0 || row.flags & !MARKET_TRADE_DETAIL_FLAGS_KNOWN != 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} has invalid flags {:#x}",
                row.flags
            ));
        }
        if reserved != 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} reserved is nonzero"
            ));
        }
        WorldFold::validate_market_trade_detail_i64(
            row_index,
            "price_raw",
            row.price_raw,
            row.flags & MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0,
            1,
            MARKET_TRADE_MAX_AMOUNT_RAW,
        )?;
        WorldFold::validate_market_trade_detail_i64(
            row_index,
            "amount_raw",
            row.amount_raw,
            row.flags & MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID != 0,
            1,
            MARKET_TRADE_MAX_AMOUNT_RAW,
        )?;
        WorldFold::validate_market_trade_detail_i32(
            row_index,
            "speed_raw",
            row.speed_raw,
            row.flags & MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0,
            0,
            1_000_000_000,
        )?;
        WorldFold::validate_market_trade_detail_i32(
            row_index,
            "completed_raw",
            row.completed_raw,
            row.flags & MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID != 0,
            0,
            1,
        )?;
        WorldFold::validate_market_trade_detail_i32(
            row_index,
            "days",
            row.days,
            row.flags & MARKET_TRADE_DETAIL_FLAG_DAYS_VALID != 0,
            -1,
            100_000,
        )?;
        if row.flags & MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0 {
            if row.delivery_country_tag_db_id == 0
                || row.delivery_country_tag_db_id
                    > compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
            {
                return Err(format!(
                    "market trade MTD1 row {row_index} has invalid delivery_country_tag_db_id {}",
                    row.delivery_country_tag_db_id
                ));
            }
        } else if row.delivery_country_tag_db_id != 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} has delivery_country_tag_db_id without flag"
            ));
        }
        if row.flags & MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID != 0 {
            if row.convoys_assigned < 0
                || row.convoys_total < 0
                || row.convoys_assigned > row.convoys_total
            {
                return Err(format!(
                    "market trade MTD1 row {row_index} has invalid convoy counts {}/{}",
                    row.convoys_assigned, row.convoys_total
                ));
            }
        } else if row.convoys_assigned != 0 || row.convoys_total != 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} has convoy counts without flag"
            ));
        }
        WorldFold::validate_market_trade_detail_i32(
            row_index,
            "efficiency_raw",
            row.efficiency_raw,
            row.flags & MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID != 0,
            0,
            FIXED_SCALE as i32,
        )?;
        WorldFold::validate_market_trade_detail_i32(
            row_index,
            "factory_cic_progress_raw",
            row.factory_cic_progress_raw,
            row.flags & MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID != 0,
            0,
            FIXED_SCALE as i32,
        )?;
        WorldFold::validate_market_trade_detail_i32(
            row_index,
            "equipment_transfer_progress_raw",
            row.equipment_transfer_progress_raw,
            row.flags & MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID != 0,
            0,
            FIXED_SCALE as i32,
        )?;
        if rows.insert(row.contract_id, row).is_some() {
            return Err(format!(
                "market trade MTD1 row {row_index} duplicates contract_id {}",
                row.contract_id
            ));
        }
    }
    Ok((rows, expected))
}

fn parse_market_trade_resource_flow_trailer(
    payload: &[u8],
    trailer_offset: usize,
) -> Result<(BTreeMap<u32, [i64; 7]>, usize), String> {
    if payload.get(trailer_offset..trailer_offset.saturating_add(4))
        != Some(MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC.as_slice())
    {
        return Err("market trade trailing bytes are not MTRF resource-flow trailer".to_string());
    }
    need(
        payload,
        trailer_offset + MARKET_TRADE_RESOURCE_FLOW_TRAILER_HEADER_LEN,
        "market trade MTRF header",
    )?;
    let version = read_u16(payload, trailer_offset + 4);
    let header_len = read_u16(payload, trailer_offset + 6) as usize;
    let row_len = read_u16(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16(payload, trailer_offset + 10);
    let row_count = read_u32(payload, trailer_offset + 12) as usize;
    let exported_count = read_u32(payload, trailer_offset + 16) as usize;
    let scale_raw = read_u32(payload, trailer_offset + 20);
    let resource_mask = read_u32(payload, trailer_offset + 24);
    let reserved1 = read_u32(payload, trailer_offset + 28);
    if version != MARKET_TRADE_RESOURCE_FLOW_TRAILER_VERSION
        || header_len != MARKET_TRADE_RESOURCE_FLOW_TRAILER_HEADER_LEN
        || row_len != MARKET_TRADE_RESOURCE_FLOW_ROW_LEN
        || reserved0 != 0
        || reserved1 != 0
    {
        return Err("market trade MTRF header shape is invalid".to_string());
    }
    if exported_count != row_count {
        return Err(format!(
            "market trade MTRF exported_count {exported_count} != row_count {row_count}"
        ));
    }
    if scale_raw != FIXED_SCALE as u32 {
        return Err(format!(
            "market trade MTRF scale_raw {scale_raw} != {}",
            FIXED_SCALE as u32
        ));
    }
    if resource_mask != MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK {
        return Err(format!(
            "market trade MTRF resource_mask {resource_mask:#x} != {:#x}",
            MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK
        ));
    }
    let expected = trailer_offset
        .saturating_add(header_len)
        .saturating_add(row_count.saturating_mul(row_len));
    if payload.len() < expected {
        return Err(format!(
            "market trade MTRF len {} < {expected}",
            payload.len()
        ));
    }

    let mut rows = BTreeMap::new();
    let mut cursor = trailer_offset + header_len;
    for row_index in 0..row_count {
        let tag_db_id = read_u32(payload, cursor);
        let flow_kind = read_u32(payload, cursor + 4);
        let row_resource_mask = read_u32(payload, cursor + 8);
        let reserved = read_u32(payload, cursor + 12);
        let mut values_raw = [0i64; 7];
        for (resource_index, value) in values_raw.iter_mut().enumerate() {
            *value = read_i64(payload, cursor + 16 + resource_index * 8);
        }
        cursor += row_len;

        if tag_db_id == 0
            || tag_db_id > compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
        {
            return Err(format!(
                "market trade MTRF row {row_index} has invalid tag_db_id {tag_db_id}"
            ));
        }
        if flow_kind != MARKET_TRADE_RESOURCE_FLOW_KIND_EXPORTED {
            return Err(format!(
                "market trade MTRF row {row_index} has unsupported flow_kind {flow_kind}"
            ));
        }
        if row_resource_mask != MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK {
            return Err(format!(
                "market trade MTRF row {row_index} has resource_mask {row_resource_mask:#x}"
            ));
        }
        if reserved != 0 {
            return Err(format!(
                "market trade MTRF row {row_index} reserved field is nonzero"
            ));
        }
        if values_raw.iter().all(|value| *value == 0) {
            return Err(format!(
                "market trade MTRF row {row_index} has all-zero exported resources"
            ));
        }
        if rows.insert(tag_db_id, values_raw).is_some() {
            return Err(format!(
                "market trade MTRF row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
    }
    Ok((rows, expected))
}

fn apply_market_trade_detail_to_contract(
    contract: &mut Map<String, Value>,
    detail: MarketTradeDetailFoldRow,
) {
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0 {
        contract.insert(
            "prices".into(),
            json!([{
                "amount": detail.price_raw as f64 / FIXED_SCALE,
                "amount_raw": detail.price_raw,
            }]),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0 {
        contract.insert("speed".into(), json!(detail.speed_raw as f64 / FIXED_SCALE));
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0 {
        contract.insert(
            "delivery_country".into(),
            json!(detail.delivery_country_tag_db_id.to_string()),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID != 0 {
        contract.insert(
            "convoys".into(),
            json!({
                "assigned": detail.convoys_assigned,
                "total": detail.convoys_total,
            }),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID != 0 {
        contract.insert(
            "delivery_efficiency".into(),
            json!(detail.efficiency_raw as f64 / FIXED_SCALE),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_DAYS_VALID != 0 {
        contract.insert("days".into(), json!(detail.days));
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID != 0 {
        contract.insert("completed".into(), json!(detail.completed_raw != 0));
    }

    let r2 = ensure_json_object_mut(contract, "r2_market_trade_live");
    r2.insert("detail_flags".into(), json!(detail.flags));
    r2.insert("amount_raw".into(), json!(detail.amount_raw));
    r2.insert("speed_raw".into(), json!(detail.speed_raw));
    r2.insert("completed_raw".into(), json!(detail.completed_raw));
    r2.insert("efficiency_raw".into(), json!(detail.efficiency_raw));
    r2.insert(
        "factory_cic_progress_raw".into(),
        json!(detail.factory_cic_progress_raw),
    );
    r2.insert(
        "equipment_transfer_progress_raw".into(),
        json!(detail.equipment_transfer_progress_raw),
    );
}

fn parse_frd_depth_payload(payload: &[u8], record_flags: u32) -> Result<FrdDepthFrame, String> {
    let header = parse_frd_depth_header(payload, record_flags)?;
    let decision_target_count = header.decision_target_count;
    let decision_target_omitted_rows = header.decision_target_omitted_rows;
    let depth_status_flags = header.depth_status_flags;
    let focus_count = header.focus_count;
    let research_count = header.research_count;
    let decision_status_count = header.decision_status_count;
    let decision_event_count = header.decision_event_count;
    let observed_focus_count = header.observed_focus_count;
    let observed_research_count = header.observed_research_count;
    let observed_decision_status_count = header.observed_decision_status_count;
    let observed_decision_event_count = header.observed_decision_event_count;
    let badge_count = header.badge_count;
    ensure_count_at_most(
        "focus_research_decisions_depth",
        "focus_count",
        focus_count,
        "observed_focus_count",
        observed_focus_count,
    )?;
    ensure_count_at_most(
        "focus_research_decisions_depth",
        "research_count",
        research_count,
        "observed_research_count",
        observed_research_count,
    )?;
    ensure_count_at_most(
        "focus_research_decisions_depth",
        "decision_status_count",
        decision_status_count,
        "observed_decision_status_count",
        observed_decision_status_count,
    )?;
    ensure_count_at_most(
        "focus_research_decisions_depth",
        "decision_event_count",
        decision_event_count,
        "observed_decision_event_count",
        observed_decision_event_count,
    )?;

    let mut cursor = header.header_len;
    let mut focus_rows = Vec::with_capacity(focus_count);
    for _ in 0..focus_count {
        need(payload, cursor + FRD_FOCUS_ROW_FIXED_LEN, "frd focus row")?;
        let tag_db_id = read_u32(payload, cursor);
        let kind = read_u32(payload, cursor + 4);
        let focus_kind = read_u32(payload, cursor + 8);
        let flags = read_u32(payload, cursor + 12);
        let previous_completed = read_u32(payload, cursor + 16);
        let current_completed = read_u32(payload, cursor + 20);
        let previous_progress = read_i32(payload, cursor + 24);
        let current_progress = read_i32(payload, cursor + 28);
        let completed_delta = read_u32(payload, cursor + 32);
        cursor += FRD_FOCUS_ROW_FIXED_LEN;
        let from_token = read_string(payload, &mut cursor, "frd focus from_token")?;
        let to_token = read_string(payload, &mut cursor, "frd focus to_token")?;
        let completed_token = read_string(payload, &mut cursor, "frd focus completed_token")?;
        focus_rows.push(FrdFocusDepthEventState {
            tag_db_id,
            kind,
            focus_kind,
            flags,
            previous_completed,
            current_completed,
            previous_progress,
            current_progress,
            completed_delta,
            from_token,
            to_token,
            completed_token,
        });
    }

    let mut research_rows = Vec::with_capacity(research_count);
    for _ in 0..research_count {
        need(payload, cursor + FRD_RESEARCH_ROW_LEN, "frd research row")?;
        let tag_db_id = read_u32(payload, cursor);
        let slot_index = read_u32(payload, cursor + 4);
        let flags = read_u32(payload, cursor + 8);
        research_rows.push(FrdResearchLiveState {
            tag_db_id,
            slot_index,
            flags,
            hour_delta: read_u32(payload, cursor + 12),
            previous_handle: read_u32(payload, cursor + 16),
            current_handle: read_u32(payload, cursor + 20),
            previous_def: read_u32(payload, cursor + 24),
            current_def: read_u32(payload, cursor + 28),
            previous_progress_raw: read_i64(payload, cursor + 32),
            current_progress_raw: read_i64(payload, cursor + 40),
            delta_raw: read_i64(payload, cursor + 48),
            speed_raw: read_i64(payload, cursor + 56),
            base_raw: (flags & FRD_RESEARCH_FLAG_BASE_PRESENT != 0)
                .then_some(read_i64(payload, cursor + 64)),
            eta_hours: (flags & FRD_RESEARCH_FLAG_ETA_AVAILABLE != 0)
                .then_some(read_i64(payload, cursor + 72)),
        });
        cursor += FRD_RESEARCH_ROW_LEN;
    }

    let mut decision_status_rows = Vec::with_capacity(decision_status_count);
    for _ in 0..decision_status_count {
        need(
            payload,
            cursor + FRD_DECISION_STATUS_ROW_LEN,
            "frd decision status row",
        )?;
        decision_status_rows.push(FrdDecisionStatusState {
            tag_db_id: read_u32(payload, cursor),
            active_timed_count: read_u32(payload, cursor + 4),
            targeted_count: read_u32(payload, cursor + 8),
            targeted_timed_count: read_u32(payload, cursor + 12),
            decisions_taken_bucket_count: read_u32(payload, cursor + 16),
            re_enable_count: read_u32(payload, cursor + 20),
            timed_live_count: read_u32(payload, cursor + 24),
        });
        cursor += FRD_DECISION_STATUS_ROW_LEN;
    }

    let mut decision_event_rows = Vec::with_capacity(decision_event_count);
    for _ in 0..decision_event_count {
        need(
            payload,
            cursor + FRD_DECISION_EVENT_ROW_FIXED_LEN,
            "frd decision event row",
        )?;
        let tag_db_id = read_u32(payload, cursor);
        let kind = read_u32(payload, cursor + 4);
        let previous_days = read_i32(payload, cursor + 12);
        let current_days = read_i32(payload, cursor + 16);
        let previous_state = read_i32(payload, cursor + 20);
        let current_state = read_i32(payload, cursor + 24);
        cursor += FRD_DECISION_EVENT_ROW_FIXED_LEN;
        let decision = read_string(payload, &mut cursor, "frd decision event token")?;
        decision_event_rows.push(FrdDecisionEventState {
            tag_db_id,
            kind,
            previous_days,
            current_days,
            previous_state,
            current_state,
            decision,
        });
    }

    let targeted_timed_status_count_by_tag: BTreeMap<u32, u32> = decision_status_rows
        .iter()
        .map(|row| (row.tag_db_id, row.targeted_timed_count))
        .collect();
    let mut decision_target_rows_by_tag: BTreeMap<u32, u32> = BTreeMap::new();
    let mut seen_decision_targets = BTreeSet::new();
    let mut decision_target_rows = Vec::with_capacity(decision_target_count);
    for row_index in 0..decision_target_count {
        need(
            payload,
            cursor + FRD_DECISION_TARGET_ROW_FIXED_LEN,
            "frd decision target row",
        )?;
        let tag_db_id = read_u32(payload, cursor);
        let target_tag_db_id = read_u32(payload, cursor + 4);
        let flags = read_u32(payload, cursor + 8);
        let source_kind = read_u32(payload, cursor + 12);
        cursor += FRD_DECISION_TARGET_ROW_FIXED_LEN;
        let decision = read_string(payload, &mut cursor, "frd decision target token")?;
        if tag_db_id == 0 || tag_db_id > FRD_MAX_TARGET_TAG_DB_ID {
            return Err(format!(
                "frd decision target row {row_index} has invalid owner tag_db_id {tag_db_id}"
            ));
        }
        if flags != FRD_DECISION_TARGET_COUNTRY_FLAGS {
            return Err(format!(
                "frd decision target row {row_index} has non-country flags {flags:#x}"
            ));
        }
        if target_tag_db_id == 0 || target_tag_db_id > FRD_MAX_TARGET_TAG_DB_ID {
            return Err(format!(
                "frd decision target row {row_index} has invalid country target id {target_tag_db_id}"
            ));
        }
        if source_kind != FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED {
            return Err(format!(
                "frd decision target row {row_index} has invalid source_kind {source_kind}"
            ));
        }
        if decision.is_empty() {
            return Err(format!(
                "frd decision target row {row_index} has no decision token"
            ));
        }
        if !seen_decision_targets.insert((
            tag_db_id,
            target_tag_db_id,
            source_kind,
            decision.clone(),
        )) {
            return Err(format!(
                "frd decision target row {row_index} duplicates owner/target/source key"
            ));
        }
        let count = decision_target_rows_by_tag.entry(tag_db_id).or_insert(0);
        *count = count.saturating_add(1);
        decision_target_rows.push(FrdDecisionTargetState {
            tag_db_id,
            target_tag_db_id,
            flags,
            source_kind,
            decision,
        });
    }
    for (tag_db_id, target_rows) in decision_target_rows_by_tag {
        match targeted_timed_status_count_by_tag.get(&tag_db_id).copied() {
            Some(status_count) if target_rows <= status_count => {}
            Some(status_count) => {
                return Err(format!(
                    "frd decision target rows {target_rows} exceed targeted_timed_count {status_count} for tag_db_id {tag_db_id}"
                ));
            }
            None => {
                return Err(format!(
                    "frd decision target rows exist without a decision status row for tag_db_id {tag_db_id}"
                ));
            }
        }
    }

    let mut badges = Vec::with_capacity(badge_count);
    for _ in 0..badge_count {
        need(payload, cursor + FRD_BADGE_ROW_FIXED_LEN, "frd badge row")?;
        let kind = read_u32(payload, cursor);
        let flags = read_u32(payload, cursor + 4);
        let source_domain_id = read_u32(payload, cursor + 8);
        let affected_count = read_u32(payload, cursor + 12);
        let evidence_code = read_u32(payload, cursor + 16);
        cursor += FRD_BADGE_ROW_FIXED_LEN;
        let detail = read_string(payload, &mut cursor, "frd badge detail")?;
        badges.push(FrdDepthBadgeState {
            kind,
            flags,
            source_domain_id,
            affected_count,
            evidence_code,
            detail,
        });
    }
    if cursor != payload.len() {
        return Err(format!(
            "focus_research_decisions_depth payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    Ok(FrdDepthFrame {
        depth_status_flags,
        focus_rows,
        research_rows,
        decision_status_rows,
        decision_event_rows,
        decision_target_rows,
        decision_target_omitted_rows,
        badges,
    })
}

struct FrdDepthHeader {
    header_len: usize,
    decision_target_count: usize,
    decision_target_omitted_rows: u32,
    depth_status_flags: u32,
    focus_count: usize,
    research_count: usize,
    decision_status_count: usize,
    decision_event_count: usize,
    observed_focus_count: usize,
    observed_research_count: usize,
    observed_decision_status_count: usize,
    observed_decision_event_count: usize,
    badge_count: usize,
}

fn parse_frd_depth_header(payload: &[u8], record_flags: u32) -> Result<FrdDepthHeader, String> {
    need(
        payload,
        FRD_HEADER_LEN_V1,
        "focus_research_decisions_depth header",
    )?;
    if record_flags & COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 != 0 {
        need(
            payload,
            FRD_HEADER_LEN,
            "focus_research_decisions_depth v2 header",
        )?;
        if payload.get(0..4) != Some(FRD_SCHEMA_MAGIC.as_slice()) {
            return Err(
                "focus_research_decisions_depth v2 header is missing FRD2 sanity marker"
                    .to_string(),
            );
        }
        let version = read_u16(payload, 4);
        let header_len = read_u16(payload, 6) as usize;
        if version != FRD_SCHEMA_VERSION || header_len != FRD_HEADER_LEN {
            return Err(format!(
                "focus_research_decisions_depth v2 header has version {version} and len {header_len}"
            ));
        }
        return Ok(FrdDepthHeader {
            header_len,
            depth_status_flags: read_u32(payload, 8),
            focus_count: read_u32(payload, 12) as usize,
            research_count: read_u32(payload, 16) as usize,
            decision_status_count: read_u32(payload, 20) as usize,
            decision_event_count: read_u32(payload, 24) as usize,
            decision_target_count: read_u32(payload, 28) as usize,
            observed_focus_count: read_u32(payload, 32) as usize,
            observed_research_count: read_u32(payload, 36) as usize,
            observed_decision_status_count: read_u32(payload, 40) as usize,
            observed_decision_event_count: read_u32(payload, 44) as usize,
            badge_count: read_u32(payload, 48) as usize,
            decision_target_omitted_rows: read_u32(payload, 52),
        });
    }
    Ok(FrdDepthHeader {
        header_len: FRD_HEADER_LEN_V1,
        decision_target_count: 0,
        decision_target_omitted_rows: 0,
        depth_status_flags: read_u32(payload, 8),
        focus_count: read_u32(payload, 12) as usize,
        research_count: read_u32(payload, 16) as usize,
        decision_status_count: read_u32(payload, 20) as usize,
        decision_event_count: read_u32(payload, 24) as usize,
        observed_focus_count: read_u32(payload, 28) as usize,
        observed_research_count: read_u32(payload, 32) as usize,
        observed_decision_status_count: read_u32(payload, 36) as usize,
        observed_decision_event_count: read_u32(payload, 40) as usize,
        badge_count: read_u32(payload, 44) as usize,
    })
}

fn validate_scripted_state_row(row: &ScriptedStateRow) -> Result<(), String> {
    // Only delayed-event rows may carry due-remaining-hours.
    if row.delayed_due_hours.is_some() && row.value_kind != SCRIPTED_STATE_VALUE_DELAYED_EVENT {
        return Err("scripted_state non-delayed-event row carries due hours".to_string());
    }
    match row.scope_kind {
        SCRIPTED_STATE_SCOPE_GLOBAL => {
            if row.owner_id != 0 {
                return Err("scripted_state global row has nonzero owner_id".to_string());
            }
        }
        SCRIPTED_STATE_SCOPE_COUNTRY | SCRIPTED_STATE_SCOPE_STATE => {
            if row.owner_id == 0 {
                return Err("scripted_state scoped row has zero owner_id".to_string());
            }
        }
        other => return Err(format!("scripted_state unsupported scope {other}")),
    }
    if row.deleted {
        match row.value_kind {
            SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64 | SCRIPTED_STATE_VALUE_FLAG_NAMED => {
                if row.key.as_ref().map_or(true, String::is_empty) {
                    return Err("scripted_state tombstone row missing key".to_string());
                }
                if row.token_id.is_some()
                    || row.date_hour_raw.is_some()
                    || row.value_raw.is_some()
                    || row.delayed_due_hours.is_some()
                {
                    return Err("scripted_state tombstone row has live fields".to_string());
                }
            }
            SCRIPTED_STATE_VALUE_FLAG_TOKEN_ONLY => {
                if row.key.is_some()
                    || row.token_id.is_none()
                    || row.date_hour_raw.is_some()
                    || row.value_raw.is_some()
                    || row.delayed_due_hours.is_some()
                {
                    return Err("scripted_state token tombstone row has live fields".to_string());
                }
            }
            _ => return Err("scripted_state unsupported tombstone value_kind".to_string()),
        }
        return Ok(());
    }
    match row.value_kind {
        SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64 => {
            if row.key.as_ref().map_or(true, String::is_empty) {
                return Err("scripted_state variable row missing key".to_string());
            }
            if !row.deleted && row.value_raw.is_none() {
                return Err("scripted_state variable row missing value".to_string());
            }
        }
        SCRIPTED_STATE_VALUE_FLAG_NAMED => {
            if row.key.as_ref().map_or(true, String::is_empty) {
                return Err("scripted_state named flag row missing key".to_string());
            }
        }
        SCRIPTED_STATE_VALUE_FLAG_TOKEN_ONLY => {
            if row.token_id.is_none() {
                return Err("scripted_state token flag row missing token_id".to_string());
            }
        }
        SCRIPTED_STATE_VALUE_DELAYED_EVENT_OPAQUE | SCRIPTED_STATE_VALUE_SCRIPTED_GUI_OPAQUE => {}
        SCRIPTED_STATE_VALUE_DELAYED_EVENT => {
            // Country-scoped only; carries event id token + scope tag date +
            // scope random seed (value), no string key. Every bound mirrors
            // the collector emit + payload gate so the three layers agree.
            if row.scope_kind != SCRIPTED_STATE_SCOPE_COUNTRY {
                return Err("scripted_state delayed-event row must be country-scoped".to_string());
            }
            // Owner (== scope) tag must be a valid country tag_db_id. The scope
            // outer-match already rejects owner_id == 0 for country scope.
            if row.owner_id > compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID {
                return Err("scripted_state delayed-event row owner tag out of range".to_string());
            }
            match row.token_id {
                Some(id)
                    if (SCRIPTED_STATE_MIN_DELAYED_EVENT_ID
                        ..=SCRIPTED_STATE_MAX_DELAYED_EVENT_ID)
                        .contains(&id) => {}
                _ => {
                    return Err("scripted_state delayed-event row has invalid event id".to_string())
                }
            }
            // Scope tag_db_id (date_hour_raw slot): present, in country range,
            // and equal to the owner tag (scope == owner is a contract, the
            // same CDelayedEvent+0x18 field per WAVE3_CRACKS_870EBF_D04.md §5).
            match row.date_hour_raw {
                Some(scope_tag)
                    if scope_tag > 0
                        && (scope_tag as u32)
                            <= compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
                        && scope_tag as u32 == row.owner_id => {}
                _ => {
                    return Err(
                        "scripted_state delayed-event row scope tag invalid or != owner"
                            .to_string(),
                    )
                }
            }
            if row.value_raw.is_none() {
                return Err(
                    "scripted_state delayed-event row missing scope random seed".to_string()
                );
            }
            if row.key.is_some() {
                return Err("scripted_state delayed-event row must not carry a key".to_string());
            }
            // Due-remaining-hours is ancillary (may be absent), but when
            // present it must be within the shared ceiling.
            if let Some(hours) = row.delayed_due_hours {
                if hours > SCRIPTED_STATE_MAX_DELAYED_EVENT_DUE_HOURS {
                    return Err(
                        "scripted_state delayed-event row due hours out of range".to_string()
                    );
                }
            }
        }
        other => return Err(format!("scripted_state unsupported value_kind {other}")),
    }
    Ok(())
}

fn ensure_supply_province_defaults(province: &mut Map<String, Value>) {
    province.entry("current").or_insert(Value::Null);
    province.entry("capacity").or_insert(Value::Null);
    province.entry("gain").or_insert(Value::Null);
    province
        .entry("divisions")
        .or_insert_with(|| Value::Array(Vec::new()));
}

fn validate_map_detail_range(
    present: bool,
    first: usize,
    len: usize,
    total: usize,
    label: &str,
) -> Result<(), String> {
    if !present {
        if first != 0 || len != 0 {
            return Err(format!("{label} range present without flag"));
        }
    } else if first.saturating_add(len) > total {
        return Err(format!("{label} range exceeds entry table"));
    }
    Ok(())
}

fn validate_optional_flagged_string(
    value: String,
    flags: u32,
    flag: u32,
    label: &str,
) -> Result<Option<String>, String> {
    if value.is_empty() && flags & flag != 0 {
        return Err(format!("{label} flag with empty string"));
    }
    if !value.is_empty() && flags & flag == 0 {
        return Err(format!("{label} string without flag"));
    }
    Ok((flags & flag != 0).then_some(value))
}

fn read_map_detail_building_entry(
    payload: &[u8],
    cursor: &mut usize,
) -> Result<MapDetailU32Entry, String> {
    let entry = MapDetailU32Entry {
        type_id: read_u32(payload, *cursor),
        level: read_u32(payload, *cursor + 4),
    };
    let reserved = read_u32(payload, *cursor + 8);
    *cursor += MAP_DETAIL_ENTRY_ROW_LEN;
    if entry.type_id == 0 {
        return Err("map detail building entry has zero type_id".into());
    }
    if reserved != 0 {
        return Err("map detail building entry reserved field is nonzero".into());
    }
    Ok(entry)
}

fn map_detail_resource_map(entries: &[MapDetailI32Entry]) -> Map<String, Value> {
    let mut out = Map::new();
    for entry in entries {
        out.insert(
            map_detail_resource_name(entry.type_id),
            json!(entry.amount_raw as f64 / FIXED_SCALE),
        );
    }
    out
}

fn map_detail_building_map(entries: &[MapDetailU32Entry]) -> Map<String, Value> {
    let mut out = Map::new();
    for entry in entries {
        out.insert(map_detail_building_name(entry.type_id), json!(entry.level));
    }
    out
}

fn map_detail_resource_name(resource_id: u32) -> String {
    match resource_id {
        0x3E03 => "steel".into(),
        0x3E05 => "rubber".into(),
        0x3E06 => "tungsten".into(),
        0x3E07 => "chromium".into(),
        0x4E1F => "aluminium".into(),
        0x4E25 => "coal".into(),
        0x4E37 => "oil".into(),
        other => format!("tid_0x{other:X}"),
    }
}

fn map_detail_building_name(type_id: u32) -> String {
    match type_id {
        0x27D2 => "naval_supply_hub".into(),
        0x2FAB => "infrastructure".into(),
        0x2FB6 => "air_base".into(),
        0x31F6 => "naval_base".into(),
        0x32B4 => "nuclear_reactor".into(),
        0x3C8F => "fuel_silo".into(),
        0x4BF6 => "dockyard".into(),
        0x4CBE => "supply_node".into(),
        0x4CC1 => "rail_way".into(),
        0x4E22 => "arms_factory".into(),
        0x4E2D => "industrial_complex".into(),
        0x4E44 => "synthetic_refinery".into(),
        0x5781 => "bunker".into(),
        0x5782 => "coastal_bunker".into(),
        0x6D5B => "anti_air_building".into(),
        0x6DEF => "radar_station".into(),
        other => format!("type_0x{other:04X}"),
    }
}

fn domain_name(domain_id: u16) -> &'static str {
    compact_contract::wire_name_for_id(domain_id).unwrap_or("unknown")
}

fn domain_descriptor_json(domain_id: u16) -> Value {
    json!({
        "domain_id": domain_id,
        "domain": domain_name(domain_id),
    })
}

fn d57_string_ref(strings: &[u8], off: u32, len: u16) -> Result<String, String> {
    let start = off as usize;
    let len = len as usize;
    let end = start
        .checked_add(len)
        .ok_or("domain57 string ref overflow")?;
    let bytes = strings
        .get(start..end)
        .ok_or("domain57 string ref out of bounds")?;
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|err| format!("domain57 string ref invalid utf-8: {err}"))
}

fn valid_domain57_country_tag(tag: &str) -> bool {
    tag.len() == 3 && tag != "---" && tag.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn detail_event_kind_name(event_kind: u32) -> Option<&'static str> {
    match event_kind {
        land_detail55::EVENT_PHASE_STARTED => Some("started"),
        land_detail55::EVENT_PHASE_UPDATED => Some("updated"),
        land_detail55::EVENT_PHASE_ENDED => Some("ended"),
        land_detail55::EVENT_PARTICIPANT_JOINED => Some("participant_joined"),
        land_detail55::EVENT_PARTICIPANT_LEFT_UNCLASSIFIED => Some("participant_left_unclassified"),
        _ => None,
    }
}

/// Decode a fixed 8-byte ascii token (diplomacy at-war enemy tag), trimmed to
/// `len` and to the first NUL. Non-utf8 bytes fail closed to an empty string.
fn ascii8(bytes: &[u8], len: usize) -> String {
    let take = len.min(bytes.len());
    let slice = &bytes[..take];
    let end = slice.iter().position(|b| *b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).to_string()
}

fn need(payload: &[u8], at_least: usize, label: &str) -> Result<(), String> {
    if payload.len() < at_least {
        Err(format!(
            "{label}: need {at_least} bytes, have {}",
            payload.len()
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_u32_field(
    present: bool,
    value: u32,
    min_value: u32,
    max_value: u32,
    field: &str,
) -> Result<(), String> {
    if present {
        if value < min_value || value > max_value {
            return Err(format!("{field} value {value} out of range"));
        }
    } else if value != 0 {
        return Err(format!("{field} present value without flag"));
    }
    Ok(())
}

fn ensure_count_at_most(
    domain: &str,
    field: &str,
    value: usize,
    max_field: &str,
    max: usize,
) -> Result<(), String> {
    if value > max {
        Err(format!("{domain} {field} {value} > {max_field} {max}"))
    } else {
        Ok(())
    }
}

fn ensure_row_count_with_deletes(
    domain: &str,
    row_count: usize,
    observed_field: &str,
    observed_rows: usize,
    deleted_rows: usize,
) -> Result<(), String> {
    let allowed = observed_rows.saturating_add(deleted_rows);
    if row_count > allowed {
        Err(format!(
            "{domain} row_count {row_count} > {observed_field} {observed_rows} + deleted_rows {deleted_rows}"
        ))
    } else {
        Ok(())
    }
}

fn ensure_fixed_scale(domain: &str, field: &str, value: u32) -> Result<(), String> {
    let expected = FIXED_SCALE as u32;
    if value != expected {
        Err(format!("{domain} {field} {value} != {expected}"))
    } else {
        Ok(())
    }
}

/// Read a length-prefixed (u16) UTF-8 string, advancing the cursor. Mirrors the
/// recorder's `read_compact_string`.
fn read_string(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    need(bytes, *cursor + 2, label)?;
    let len = read_u16(bytes, *cursor) as usize;
    *cursor += 2;
    need(bytes, *cursor + len, label)?;
    let start = *cursor;
    *cursor += len;
    std::str::from_utf8(&bytes[start..start + len])
        .map(str::to_string)
        .map_err(|e| format!("{label}: invalid utf-8: {e}"))
}

fn read_u16(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}
fn read_u32(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}
fn read_u64(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        bytes[off],
        bytes[off + 1],
        bytes[off + 2],
        bytes[off + 3],
        bytes[off + 4],
        bytes[off + 5],
        bytes[off + 6],
        bytes[off + 7],
    ])
}
fn read_i32(bytes: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}
fn read_i64(bytes: &[u8], off: usize) -> i64 {
    i64::from_le_bytes([
        bytes[off],
        bytes[off + 1],
        bytes[off + 2],
        bytes[off + 3],
        bytes[off + 4],
        bytes[off + 5],
        bytes[off + 6],
        bytes[off + 7],
    ])
}

#[cfg(test)]
mod tests;
