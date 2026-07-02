//! Per-build offset/sentinel profile for the supported hoi4.exe build.
//!
//! Offsets here are rediscovered per build from live and save-derived
//! invariants; they are never carried across builds blindly. If either
//! sentinel fails to match at attach time the collector must fail closed:
//! every root derived from this profile is invalid for that process.
//!
//! Lane crates must import these values instead of keeping local copies;
//! lane-local sentinel/offset drift is what previously blocked the domain 55
//! live root probe.

/// Human-readable id for the currently supported build profile.
pub const COLLECTOR_PROFILE_ID: &str = "vanilla_latest_2026_06_30";

/// First 16 bytes of the daily-update tick function prologue.
pub const DAILY_UPDATE_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D, 0xAC,
];

/// First 16 bytes of the long-string accessor prologue.
pub const GET_LONG_STRING_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];

/// Module RVA of the daily-update tick function (sentinel anchor).
pub const C_GAME_STATE_DAILY_UPDATE_OFFSET: usize = 0x1D4390;

/// Module RVA of the long-string accessor (sentinel anchor).
pub const GET_LONG_STRING_OFFSET: usize = 0x526A40;

/// Module RVA of the static current-date pointer slot.
pub const CG_STATE_DATE_OFFSET: usize = 0x3316E90;

/// Module RVA of the static country database pointer slot.
pub const COUNTRY_DATABASE_OFFSET: usize = 0x33189C8;

/// Maximum compact country/tag-db id accepted by live row schemas.
///
/// Country table traversal may retain a wider structural guard, but emitted
/// compact owner ids must share this cap with lane validators to avoid encoder
/// panics on high sparse slots.
pub const MAX_COMPACT_OWNER_TAG_DB_ID: u32 = 2_048;

/// Domain 54 army/order battleplan offsets for the supported profile.
///
/// Current-build provenance is documented in
/// `extractor/documents/archive/native_lanes/army_oob_orders_battleplans_live/ARMY_BATTLEPLANS_DEPTH_REGISTRATION_PLAN.md`.
pub const DOMAIN54_ARMY_VTABLE_RVA: usize = 0x293BC60;
pub const DOMAIN54_ORDER_VTABLE_RVA: usize = 0x296FAE8;
pub const DOMAIN54_ARMY_OFF_ORDERS_VEC: usize = 0x98;
pub const DOMAIN54_ORDER_OFF_TYPE: usize = 0x30;
pub const DOMAIN54_ORDER_OFF_ARMY_BACKREF: usize = 0x38;
pub const DOMAIN54_ORDER_OFF_PATH_BEGIN: usize = 0x70;
pub const DOMAIN54_ORDER_OFF_PATH_SIZE: usize = 0x78;
pub const DOMAIN54_ORDER_OFF_TYPE5_STATES_BEGIN: usize = 0xE0;
pub const DOMAIN54_ORDER_OFF_TYPE5_STATES_SIZE: usize = 0xE8;

/// Short build proof id retained by the domain 48 proof binaries.
pub const DOMAIN48_BUILD_PROFILE: &str = "870ebf";

/// Conservative process heap range used by domain 48 pointer validation.
pub const DOMAIN48_HEAP_LO: usize = 0x10000000000;
pub const DOMAIN48_HEAP_HI: usize = 0x7FF000000000;
pub const DOMAIN48_POINTER_WIDTH: usize = 8;

/// Domain 47 validated live character/leader roots for the supported profile.
pub const DOMAIN47_CCHARACTER_VTABLE_RVA: usize = 0x2967E30;
pub const DOMAIN47_CARMY_LEADER_VTABLE_RVA: usize = 0x293F538;
pub const DOMAIN47_CNAVY_LEADER_VTABLE_RVA: usize = 0x293F650;
pub const DOMAIN47_CCOUNTRY_LEADER_VTABLE_RVA: usize = 0x296A720;

/// Domain 47 historical roots that stay blocked until revalidated in the
/// shared collector. Advisor/appointee/portrait rows and unit-leader trait
/// token rows must not be emitted from these constants alone.
pub const DOMAIN47_CADVISOR_RTTI_RVA_BLOCKED_PENDING_REVALIDATION: usize = 0x2990E60;
pub const DOMAIN47_CUNIT_LEADER_TRAIT_VTABLE_RVA_BLOCKED_PENDING_REVALIDATION: usize = 0x291F3C0;

/// Domain 53 validated diplomacy/intel/operative live roots for the supported
/// profile. These constants are intentionally raw current-build evidence; lane
/// code must keep unresolved semantics badged instead of renaming fields early.
pub const DOMAIN53_CGAMESTATE_COUNTRY_TABLE: usize = 0x310;
pub const DOMAIN53_CGAMESTATE_COUNTRY_COUNT: usize = 0x31C;
pub const DOMAIN53_COUNTRY_VTABLE_RVA: usize = 0x27D0E90;
pub const DOMAIN53_OBJ_TAG_DB_ID: usize = 0x08;
pub const DOMAIN53_COUNTRY_FACTION_GROUP: usize = 0x598;
pub const DOMAIN53_RELATION_MANAGER: usize = 0xF88;
pub const DOMAIN53_RELATION_TABLE: usize = 0x08;
pub const DOMAIN53_RELATION_OPINION: usize = 0x300;
pub const DOMAIN53_RELATION_SCAN_BYTES: usize = 0x340;
pub const DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA: usize = 0x2949D98;
pub const DOMAIN53_REL_SUBJECT_VTABLE_RVA: usize = 0x2949F18;
pub const DOMAIN53_REL_EMBARGO_VTABLE_RVA: usize = 0x294A070;
pub const DOMAIN53_REL_MILITARY_ACCESS_VTABLE_RVA: usize = 0x2949918;
pub const DOMAIN53_REL_AIR_BASE_ACCESS_VTABLE_RVA: usize = 0x2A0A9D8;
pub const DOMAIN53_REL_DOCKING_RIGHTS_VTABLE_RVA: usize = 0x2A0ABC8;
pub const DOMAIN53_REL_NAVAL_BLOCKADE_VTABLE_RVA: usize = 0x2A0AD70;
pub const DOMAIN53_REL_MARKET_ACCESS_RIGHTS_VTABLE_RVA: usize = 0x2A0AF20;
pub const DOMAIN53_REL_EQUIPMENT_PURCHASE_RELATION_VTABLE_RVA: usize = 0x2A0AE58;
pub const DOMAIN53_REL_GUARANTEE_VTABLE_RVA: usize = 0x2949CD8;
pub const DOMAIN53_REL_IMPROVE_RELATIONS_VTABLE_RVA: usize = 0x2949A98;
pub const DOMAIN53_REL_ATTACHE_VTABLE_RVA: usize = 0x2949B58;
pub const DOMAIN53_REL_LICENCE_PRODUCTION_VTABLE_RVA: usize = 0x2949858;
pub const DOMAIN53_SUBJECT_OVERLORD: usize = 0x10;
pub const DOMAIN53_SUBJECT_SUBJECT: usize = 0x14;
pub const DOMAIN53_SUBJECT_STATE_DB: usize = 0x58;
pub const DOMAIN53_SUBJECT_STATE_NAME: usize = 0x08;
pub const DOMAIN53_SUBJECT_RATIO: usize = 0x60;
pub const DOMAIN53_COUNTRY_AGENCY_PTR: usize = 0xFC0;
pub const DOMAIN53_AGENCY_VTABLE_RVA: usize = 0x296B338;
pub const DOMAIN53_AGENCY_NAME: usize = 0x80;
pub const DOMAIN53_AGENCY_ICON: usize = 0xA0;
pub const DOMAIN53_AGENCY_UPGRADE_PROGRESS: usize = 0xC8;
pub const DOMAIN53_AGENCY_CAPACITY_E4: usize = 0xE4;
pub const DOMAIN53_AGENCY_CAPACITY_F0: usize = 0xF0;
pub const DOMAIN53_AGENCY_CAPACITY_F4: usize = 0xF4;
pub const DOMAIN53_AGENCY_CRYPTO_MGR: usize = 0x120;
pub const DOMAIN53_AGENCY_DEFENSE: usize = 0x128;
pub const DOMAIN53_CRYPTOLOGY_VTABLE_RVA: usize = 0x29A6540;
pub const DOMAIN53_CRYPTOLOGY_RECORD_VTABLE_RVA: usize = 0x29A64F0;
pub const DOMAIN53_CRYPTOLOGY_MGR_DATA: usize = 0x28;
pub const DOMAIN53_CRYPTOLOGY_MGR_SIZE: usize = 0x30;
pub const DOMAIN53_CRYPTOLOGY_MGR_CAP: usize = 0x34;
pub const DOMAIN53_CRYPTOLOGY_RECORD_STRIDE: usize = 0x38;
pub const DOMAIN53_CRYPTOLOGY_RECORD_TARGET: usize = 0x08;
pub const DOMAIN53_CRYPTOLOGY_RECORD_DECRYPTING: usize = 0x12;
pub const DOMAIN53_CRYPTOLOGY_RECORD_AMOUNT: usize = 0x18;
pub const DOMAIN53_OPERATIVE_HANDLE_TYPE: u32 = 4_713;
pub const DOMAIN53_OPERATIVE_VTABLE_RVA: usize = 0x293F790;
pub const DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE: usize = 0x08;
pub const DOMAIN53_OPERATIVE_NAME: usize = 0x18;
pub const DOMAIN53_OPERATIVE_NAME_DUPLICATE: usize = 0x38;
pub const DOMAIN53_OPERATIVE_DETECTED: usize = 0x68;
pub const DOMAIN53_OPERATIVE_LIFECYCLE_FLAG: usize = 0x110;
pub const DOMAIN53_OPERATIVE_OWNER: usize = 0x118;
pub const DOMAIN53_OPERATIVE_LEADER_FROM_IDENTITY_DELTA: isize = -0x08;
pub const DOMAIN53_OPERATIVE_SKILL_SUBOBJECT: usize = 0xE60;
pub const DOMAIN53_OPERATIVE_EXPERIENCE_RAW: usize = 0xE68;
pub const DOMAIN53_OPERATIVE_SKILL_LEVEL: usize = 0x1B8;
pub const DOMAIN53_WAR_FIRST_TAG: usize = 0x00;
pub const DOMAIN53_WAR_SECOND_TAG: usize = 0x04;
pub const DOMAIN53_WAR_DATE_VTABLE: usize = 0x08;
pub const DOMAIN53_WAR_FIRST_CASUALTIES: usize = 0x40;
pub const DOMAIN53_WAR_SECOND_CASUALTIES: usize = 0x48;
pub const DOMAIN53_WAR_ECHO_FIRST: usize = 0x68;
pub const DOMAIN53_WAR_ECHO_SECOND: usize = 0x6C;
pub const DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA: usize = 0x2702190;
pub const DOMAIN53_WAR_GAME_DATE_VTABLE_RVA: usize = 0x2702388;
pub const DOMAIN53_HEAP_LO: usize = 0x10000000000;
pub const DOMAIN53_HEAP_HI: usize = 0x7FF000000000;
pub const DOMAIN53_MAX_COUNTRY_TABLE_SLOTS: u32 = 4_096;
pub const DOMAIN53_MAX_VECTOR_ROWS: u32 = 8_192;

/// Domain 43 market-trade purchase-contract layout for the supported 870ebf
/// profile. Provenance: MTD1 lane validation in
/// `extractor/native/lanes/market_trade_live/SLICE_PACKET.md` matched 63
/// same-tick purchase-contract detail rows on the current build. The price
/// root remains the shared equipment-subbuffer root at `+0x38`; these offsets
/// cover the purchase-contract object and detail fields only.
pub const DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA: usize = 0x2954C70;
pub const DOMAIN43_PURCHASE_CONTRACT_ID_TYPE: usize = 0x08;
pub const DOMAIN43_PURCHASE_CONTRACT_ID_ID: usize = 0x0C;
pub const DOMAIN43_PURCHASE_CONTRACT_EQUIPMENT_SUBBUF: usize = 0x38;
pub const DOMAIN43_PURCHASE_CONTRACT_SELLER_TAG_DB_ID: usize = 0x58;
pub const DOMAIN43_PURCHASE_CONTRACT_BUYER_TAG_DB_ID: usize = 0x5C;
pub const DOMAIN43_PURCHASE_CONTRACT_DELIVERY_COUNTRY_TAG_DB_ID: usize = 0x5C;
pub const DOMAIN43_PURCHASE_CONTRACT_DELIVERY_SUBBUF: usize = 0x80;
pub const DOMAIN43_PURCHASE_CONTRACT_SPEED_RAW: usize = 0xC0;
pub const DOMAIN43_PURCHASE_CONTRACT_CONVOYS_ASSIGNED: usize = 0x118;
pub const DOMAIN43_PURCHASE_CONTRACT_CONVOYS_TOTAL: usize = 0x11C;
pub const DOMAIN43_PURCHASE_CONTRACT_BUYER_PTR: usize = 0x128;
pub const DOMAIN43_PURCHASE_CONTRACT_EFFICIENCY_RAW: usize = 0x1C0;
pub const DOMAIN43_PURCHASE_CONTRACT_DAYS: usize = 0x1D0;
pub const DOMAIN43_PURCHASE_CONTRACT_FACTORY_CIC_PROGRESS_RAW: usize = 0x1E0;
pub const DOMAIN43_PURCHASE_CONTRACT_EQUIPMENT_TRANSFER_PROGRESS_RAW: usize = 0x1E8;
pub const DOMAIN43_PURCHASE_CONTRACT_COMPLETED_RAW: usize = 0x25C;
pub const DOMAIN43_PURCHASE_CONTRACT_TYPE: u32 = 81;

/// Domain 45 targeted-timed decision country-target layout for the supported
/// 870ebf profile. Provenance: focus/research/decisions depth lane validation
/// in `extractor/native/lanes/focus_research_decisions_depth_live/SLICE_PACKET.md`
/// matched 1061 save/live country-target rows on the current build. Numeric
/// targets, province targets, cooldowns, and body fields remain excluded.
pub const DOMAIN45_DECISION_STATUS_TARGETED_TIMED: usize = 0xE8;
pub const DOMAIN45_TARGETED_TIMED_DECISION_VTABLE_RVA: usize = 0x27D5038;
pub const DOMAIN45_TARGETED_TIMED_DECISION_DEF: usize = 0x10;
pub const DOMAIN45_TARGETED_TIMED_DECISION_COUNTRY_TARGET: usize = 0x20;

/// Domain 48 state/province/building offsets for the supported profile.
pub const STATE_VTABLE_RVA: usize = 0x29206C0;
pub const STATE_OFF_SID: usize = 0x58;
pub const STATE_OFF_DEF: usize = 0x30;
pub const STATE_OFF_BUILDINGS_VEC: usize = 0x158;
pub const STATE_DEF_OFF_RESOURCES_PTR: usize = 0xF8;
pub const STATE_DEF_OFF_RESOURCES_SIZE: usize = 0x100;
pub const STATE_DEF_OFF_RESOURCES_CAP: usize = 0x104;
pub const STATE_RESOURCE_STRIDE: usize = 0x10;
pub const STATE_RESOURCE_NO_ID: u32 = u32::MAX;
pub const STATE_RESOURCE_MAX_ROWS: u32 = 16;
pub const STATE_RESOURCE_MAX_ABS_RAW: i32 = 100_000_000;

pub const CPROVINCE_VTABLE_RVA: usize = 0x295AF68;
pub const CPROVINCE_OFF_ID: usize = 0xA4;
pub const CPROVINCE_OFF_STATE: usize = 0xC0;
pub const CPROVINCE_OFF_BUILDINGS_VEC: usize = 0x1C8;

pub const CBUILDING_VTABLE_RVA: usize = 0x2973B88;
pub const CBUILDING_OFF_TYPE: usize = 0x08;
pub const CBUILDING_OFF_LEVEL: usize = 0x40;
pub const CBUILDING_OFF_HEALTHY_LEVEL: usize = 0x42;
pub const BUILDING_VECTOR_MAX: u32 = 64;
pub const BUILDING_LEVEL_SANE_MAX: u32 = 1_000;

/// Domain 48 production/construction queue offsets for the supported profile.
pub const OBJ_PROD_MANAGER: usize = 0x0F68;
pub const CONSTRUCTION_BEGIN: usize = 0x70;
pub const CONSTRUCTION_CAP: usize = 0x78;
pub const CONSTRUCTION_SIZE: usize = 0x7C;
pub const CONSTRUCTION_STRIDE: usize = 0x08;
pub const LINE_TYPE: usize = 0x08;
pub const LINE_ID: usize = 0x0C;
pub const LINE_ACTIVE_FACTORIES: usize = 0x18;
pub const LINE_COST: usize = 0x28;
pub const LINE_PRODUCED: usize = 0x38;
pub const LINE_CONSTRUCTION_SUB: usize = 0x70;
pub const LINE_TYPE_CONSTRUCTION: u32 = 59;
pub const CONSTRUCTION_TEMPLATE_ID: usize = 0x08;
pub const CONSTRUCTION_TARGET_LOCATION_PTR: usize = 0x1D8;
pub const TARGET_LOCATION_ID: usize = 0x6C;
pub const MAX_CONSTRUCTION_TEMPLATE_ID: u32 = 10_000_000;
pub const MAX_PRODUCTION_VECTOR_ENTRIES: u32 = 4096;
pub const MAX_PRODUCTION_LINE_FIXED_RAW: i32 = 2_000_000_000;
pub const MAX_ACTIVE_FACTORIES: i32 = 100_000;

pub const SCOPE_STATE: u32 = 1;
pub const SCOPE_PROVINCE: u32 = 2;
pub const MAX_STATE_ID: u32 = 65_535;
pub const MAX_PROVINCE_ID: u32 = 65_535;
