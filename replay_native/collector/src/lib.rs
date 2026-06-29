#![deny(unsafe_op_in_unsafe_fn)]

use core::{ptr, slice};

pub const ABI_VERSION: u32 = 1;
pub const COLLECTOR_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COLLECTOR_PROFILE_ID: &str = "vanilla_latest_2026_06_28";

const HEARTBEAT_JSON: &[u8] = br#"{"type":"heartbeat","backend":"in_process_collector","collector_version":"0.1.0","abi_version":1,"domains":[],"quality_status":"ok"}"#;

#[no_mangle]
pub extern "C" fn battle-plan_collector_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn battle-plan_collector_heartbeat_len() -> usize {
    HEARTBEAT_JSON.len()
}

/// Copy the static heartbeat JSON into `out`.
///
/// Returns the full required byte length even when `cap` is too small. The bytes
/// are not NUL-terminated; callers should use the returned length contract.
#[no_mangle]
pub unsafe extern "C" fn battle-plan_collector_heartbeat_json(out: *mut u8, cap: usize) -> usize {
    if !out.is_null() && cap != 0 {
        let n = HEARTBEAT_JSON.len().min(cap);
        unsafe {
            ptr::copy_nonoverlapping(HEARTBEAT_JSON.as_ptr(), out, n);
        }
    }
    HEARTBEAT_JSON.len()
}

pub fn heartbeat_json_for_tests() -> &'static [u8] {
    HEARTBEAT_JSON
}

/// Test helper mirroring the C ABI with a Rust slice.
pub fn copy_heartbeat_json(out: &mut [u8]) -> usize {
    let n = HEARTBEAT_JSON.len().min(out.len());
    out[..n].copy_from_slice(&HEARTBEAT_JSON[..n]);
    HEARTBEAT_JSON.len()
}

/// Validate that a raw pointer/length pair can be viewed as a caller-owned byte
/// slice. This is not exported yet; it is the shape future transport functions
/// will use for frame buffers.
unsafe fn _caller_buffer<'a>(ptr: *mut u8, len: usize) -> Option<&'a mut [u8]> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { slice::from_raw_parts_mut(ptr, len) })
}

#[cfg(windows)]
mod attach_log {
    use core::ffi::c_void;
    use core::ptr;
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::fs::{read_to_string, OpenOptions};
    use std::io::Write;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    const DLL_PROCESS_ATTACH: u32 = 1;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;
    type Hmodule = *mut c_void;
    type Lpvoid = *mut c_void;
    type ThreadStart = Option<unsafe extern "system" fn(Lpvoid) -> Dword>;

    static mut DLL_MODULE: Hmodule = ptr::null_mut();

    const MEM_COMMIT: Dword = 0x1000;
    const PAGE_NOACCESS: Dword = 0x01;
    const PAGE_GUARD: Dword = 0x100;
    const GENERIC_WRITE: Dword = 0x40000000;
    const OPEN_EXISTING: Dword = 3;
    const FILE_ATTRIBUTE_NORMAL: Dword = 0x80;
    const C_GAME_STATE_DAILY_UPDATE_OFFSET: usize = 0x1D4540;
    const CG_STATE_DATE_OFFSET: usize = 0x33048C0;
    const COUNTRY_DATABASE_OFFSET: usize = 0x33063F8;
    const GET_LONG_STRING_OFFSET: usize = 0x5269B0;
    const COUNTRY_VTABLE_RVA: usize = 0x27C0E80;
    const POLITICAL_STATUS_VTABLE_RVA: usize = 0x29299C8;
    const POLITICAL_PARTY_VTABLE_RVA: usize = 0x2976EE0;
    const IDEA_VTABLE_RVA_PRIMARY: usize = 0x295A8C0;
    const IDEA_VTABLE_RVA_SECONDARY: usize = 0x295A910;
    const IDEOLOGY_GROUP_VTABLE_RVA: usize = 0x29162E0;
    const GS_CURRENT_DATE_OFFSET: usize = 0x468;
    const GS_START_DATE_OFFSET: usize = 0x4A8;
    const DATE_EPOCH_CONST: i32 = 43_799_999;
    const HEAP_LO: usize = 0x10000000000;
    const HEAP_HI: usize = 0x7FF000000000;
    const OBJ_TAG_DB_ID: usize = 0x08;
    const OBJ_NAME: usize = 0x10;
    const OBJ_TECH_STATUS: usize = 0x0F60;
    const OBJ_POLITICAL_STATUS: usize = 0x0F90;
    const COUNTRY_RECORD_OBJECT_PTR: usize = 0x08;
    const COUNTRY_RECORD_POLITICAL_POWER: usize = 0x20;
    const POLITICAL_STATUS_PARTIES_DATA: usize = 0x20;
    const POLITICAL_STATUS_PARTIES_SIZE: usize = 0x28;
    const POLITICAL_STATUS_PARTIES_CAP: usize = 0x2C;
    const POLITICAL_STATUS_IDEAS_DATA: usize = 0x50;
    const POLITICAL_STATUS_IDEAS_CAP: usize = 0x58;
    const POLITICAL_STATUS_IDEAS_SIZE: usize = 0x5C;
    const POLITICAL_STATUS_COUNTRY_BACKLINK: usize = 0x0C8;
    const POLITICAL_STATUS_RULING_PARTY: usize = 0x0D0;
    const POLITICAL_STATUS_LAST_ELECTION: usize = 0x0A0;
    const POLITICAL_STATUS_POLITICAL_POWER: usize = 0x0E0;
    const POLITICAL_STATUS_ELECTION_FREQUENCY: usize = 0x0E8;
    const POLITICAL_STATUS_ELECTIONS_ALLOWED: usize = 0x0EC;
    const POLITICAL_PARTY_IDEOLOGY_GROUP: usize = 0x18;
    const POLITICAL_PARTY_POPULARITY: usize = 0x88;
    const IDEOLOGY_GROUP_ID: usize = 0x18;
    const IDEA_ID: usize = 0x18;
    const STATE_VTABLE_RVA: usize = 0x2910920;
    const STATE_OFF_SID: usize = 0x58;
    const STATE_OFF_OWNER: usize = 0xC8;
    const STATE_OFF_CTRL: usize = 0xCC;
    const CGAMESTATE_COUNTRY_TABLE: usize = 0x310;
    const CGAMESTATE_CPROVINCE_ARRAY: usize = 0x2B0;
    const CGAMESTATE_CPROVINCE_COUNT: usize = 0x2BC;
    const CPROVINCE_VTABLE_RVA: usize = 0x294AE78;
    const CPROVINCE_OFF_ID: usize = 0xA4;
    const CPROVINCE_OFF_STATE: usize = 0xC0;
    const CPROVINCE_OFF_CONTROLLER: usize = 0x188;
    const OBJ_TEMPLATES_LOCKED: usize = 0x01B4;
    const OBJ_COMMAND_POWER: usize = 0x01F0;
    const OBJ_MANPOWER_POOL: usize = 0x0338;
    const OBJ_PROD_MANAGER: usize = 0x0F68;
    const OBJ_FUEL_STATUS: usize = 0x1580;
    const OBJ_ORIGINAL_CAPITAL: usize = 0x101C;
    const OBJ_STABILITY: usize = 0x10D0;
    const OBJ_WAR_SUPPORT: usize = 0x10D8;
    const OBJ_CONVOYS: usize = 0x1280;
    const OBJ_ACE_COUNT: usize = 0x12E4;
    const OBJ_ORIGINAL_TAG: usize = 0x130C;
    const OBJ_NUM_SHIPS: usize = 0x1330;
    const OBJ_CONVOYS_DESTROYED: usize = 0x1334;
    const OBJ_FOCUS_TREE: usize = 0x1370;
    const OBJ_FOCUS_STATUS: usize = 0x1380;
    const OBJ_COSMETIC_TAG: usize = 0x1488;
    const OBJ_RESERVED_DYNAMIC_COUNTRY: usize = 0x145D;
    const OBJ_ACCIDENTS_SCORE: usize = 0x14E8;
    const OBJ_DECISION_STATUS: usize = 0x0FA0;
    const DECISION_STATUS_OWNER: usize = 0x08;
    const DECISION_STATUS_RE_ENABLE: usize = 0x40;
    const DECISION_STATUS_TARGETED: usize = 0x88;
    const DECISION_STATUS_ACTIVE_TIMED: usize = 0xA0;
    const DECISION_STATUS_TARGETED_TIMED: usize = 0xE8;
    const DECISION_STATUS_RE_ENABLE_ALT: usize = 0x130;
    const DECISION_STATUS_VTABLE_RVA: usize = 0x27C50E0;
    const TIMED_DECISION_VTABLE_RVA: usize = 0x27C4FC0;
    const TIMED_DECISION_DEF: usize = 0x10;
    const TIMED_DECISION_DAYS: usize = 0x18;
    const TIMED_DECISION_STATE: usize = 0x1C;
    const DECISION_DEF_NAME: usize = 0x120;
    const DECISION_STATE_ACTIVE: i32 = 0;
    const MAX_DECISION_STATUS_LIST: u32 = 4096;
    const MAX_TIMED_DECISIONS: usize = 256;
    const MAX_DECISION_DAYS: i32 = 100_000;
    const FOCUS_TREE_ID: usize = 0x08;
    const FOCUS_STATUS_CURRENT: usize = 0x10;
    const FOCUS_STATUS_CONTINUOUS: usize = 0x18;
    const FOCUS_STATUS_PROGRESS: usize = 0x38;
    const FOCUS_STATUS_COMPLETED_DATA: usize = 0x40;
    const FOCUS_STATUS_COMPLETED_CAP: usize = 0x48;
    const FOCUS_STATUS_COMPLETED_SIZE: usize = 0x4C;
    const FOCUS_OBJECT_ID: usize = 0x18;
    const MAX_FOCUS_COUNT: u32 = 4000;
    const MAX_FOCUS_ID_LEN: usize = 128;
    const TECH_STATUS_VTABLE_RVA: usize = 0x294E288;
    const TECH_STATUS_RESEARCHED_DATA: usize = 0x40;
    const TECH_STATUS_RESEARCHED_CAP: usize = 0x48;
    const TECH_STATUS_RESEARCHED_SIZE: usize = 0x4C;
    const TECH_STATUS_RESEARCH_SLOTS_DATA: usize = 0xA0;
    const TECH_STATUS_RESEARCH_SLOTS_CAP: usize = 0xA8;
    const TECH_STATUS_RESEARCH_SLOT_COUNT: usize = 0xAC;
    const TECH_STATUS_COUNTRY_BACKLINK: usize = 0x130;
    const RESEARCH_SLOT_VTABLE_RVA: usize = 0x294E238;
    const RESEARCH_SLOT_TECH_HANDLE: usize = 0x08;
    const RESEARCH_SLOT_OWNER: usize = 0x10;
    const RESEARCH_SLOT_TECH_PTR: usize = 0x18;
    const RESEARCH_SLOT_PROGRESS: usize = 0x38;
    const RESEARCH_SLOT_INDEX: usize = 0x40;
    const RESEARCH_SLOT_IDLE_HANDLE: u32 = 0x2A4E;
    const TECHNOLOGY_VTABLE_RVA_PRIMARY: usize = 0x294E218;
    const TECHNOLOGY_VTABLE_RVA_SECONDARY: usize = 0x294E1C8;
    const TECHNOLOGY_DEF_INDEX: usize = 0x08;
    const MAX_COMPLETED_TECH_COUNT: u32 = 2000;
    const MAX_RESEARCH_SLOT_COUNT: u32 = 32;
    const FUEL_STATUS_CURRENT: usize = 0x08;
    const FUEL_STATUS_MAX: usize = 0x10;
    const FUEL_STATUS_GAIN: usize = 0x18;
    const FUEL_STATUS_COST: usize = 0x38;
    const FUEL_SCALE: f64 = 32768.0;
    const MAX_FUEL_MEASURE: f64 = 1_000_000_000.0;
    const VARIANT_VTABLE_RVA: usize = 0x292B218;
    const VARIANT_TYPE: u32 = 70;
    const STOCK_BEGIN: usize = 0x220;
    const STOCK_CAP: usize = 0x228;
    const STOCK_SIZE: usize = 0x22C;
    const STOCK_STRIDE: usize = 0x10;
    const STOCK_ENTRY_DEF: usize = 0x00;
    const STOCK_ENTRY_AMOUNT: usize = 0x08;
    const VARIANT_DEF_TYPE: usize = 0x08;
    const VARIANT_DEF_ID: usize = 0x0C;
    const LINE_BEGIN: usize = 0x58;
    const LINE_CAP: usize = 0x60;
    const LINE_SIZE: usize = 0x64;
    const LINE_STRIDE: usize = 0x08;
    const LINE_TYPE: usize = 0x08;
    const LINE_ID: usize = 0x0C;
    const LINE_ACTIVE_FACTORIES: usize = 0x18;
    const LINE_COST: usize = 0x28;
    const LINE_SPEED: usize = 0x30;
    const LINE_PRODUCED: usize = 0x38;
    const LINE_AMOUNT: usize = 0x48;
    const LINE_TYPE_MILITARY: u32 = 56;
    const LINE_TYPE_NAVAL: u32 = 57;
    const MAX_PRODUCTION_VECTOR_ENTRIES: u32 = 4096;
    const MAX_PRODUCTION_STOCKPILE_AMOUNT: f64 = 1_000_000_000_000.0;
    const MAX_PRODUCTION_LINE_FIXED: f64 = 1_000_000_000.0;
    const MAX_POLITICAL_PARTIES: u32 = 16;
    const MAX_ACTIVE_IDEAS: u32 = 400;
    const MAX_POLITICS_TOKEN_LEN: usize = 160;
    const MAX_ADVISOR_TOKEN_FILTER_TOKENS: usize = 8192;
    const MAX_ADVISOR_TOKEN_FILTER_PAYLOAD_BYTES: usize = 512 * 1024;
    const MAX_IDEA_GROUP_FILTER_ROWS: usize = 8192;
    const MAX_IDEA_GROUP_FILTER_PAYLOAD_BYTES: usize = 512 * 1024;
    const MIN_USABLE_COUNTRY_OBJECT_ROWS: usize = 40;
    const COUNTRY_DATABASE_OBJECT_ARRAY_SEARCH_SPAN: usize = 0x1000;
    const MAX_COUNTRY_OBJECT_BRIDGE_ROWS: usize = 512;
    const MAX_MAP_CONTROL_STATE_ROOT_ROWS: usize = 4096;
    const MAX_MAP_CONTROL_STATE_ROOT_PAYLOAD_BYTES: usize = 131_072;
    const MAX_MAP_CONTROL_STATE_ID: u32 = 65_535;
    const MAX_MAP_CONTROL_PROVINCE_BRIDGE_PAYLOAD_BYTES: usize = 512;
    const MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS: usize = 1024;
    const MAX_MAP_CONTROL_CONTROLLER_BRIDGE_PAYLOAD_BYTES: usize = 32_768;
    const MAX_MAP_CONTROL_PROVINCE_COUNT: u32 = 65_536;
    const MAX_MAP_CONTROL_PROVINCE_STRIDE: u32 = 64;
    const MAP_CONTROL_POINTER_WIDTH: u32 = 8;
    const MIN_USABLE_STATE_ROWS: usize = 1_000;
    const MIN_USABLE_PROVINCE_ROWS: usize = 10_000;
    const DIVISION_TYPE: u32 = 4713;
    const DIVISION_VTABLE_RVA: usize = 0x2933F00;
    const DIVISION_MOVE_ORDER_VTABLE_RVA: usize = 0x294AE78;
    const DIVISION_OFF_ID_TYPE: usize = 0x08;
    const DIVISION_OFF_ID_ID: usize = 0x0C;
    const DIVISION_OFF_LOCATION: usize = 0x18;
    const DIVISION_OFF_COMBAT_REF_COUNT: usize = 0x1A4;
    const DIVISION_OFF_MOVE_ORDER: usize = 0x1E8;
    const DIVISION_OFF_MOVE_PROGRESS: usize = 0x230;
    const DIVISION_OFF_STRENGTH: usize = 0x410;
    const DIVISION_OFF_ORGANISATION: usize = 0x418;
    const DIVISION_MOVE_ORDER_OFF_SOURCE: usize = 0xA4;
    const MAX_DIVISION_BODY_BRIDGE_ROWS: usize = 20_000;
    const MAX_DIVISION_BODY_BRIDGE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
    const MAX_DIVISION_BODY_BRIDGE_DIV_ID: u32 = i32::MAX as u32;
    const MAX_DIVISION_PROVINCE_ID: u32 = 1_000_000;
    const MAX_DIVISION_COMBAT_REF_COUNT: u32 = 1_000;
    const MAX_DIVISION_ORGANISATION_RAW: u32 = 100_000_000;
    const MAX_DIVISION_STRENGTH_RAW: u32 = 1_000_000_000;
    const DIVISION_DELTA_BUCKET_RAW: u32 = 1_000;
    const ARMY_VTABLE_RVA: usize = 0x292BEC0;
    const ARMY_TYPE: u32 = 53;
    const ARMY_OFF_HANDLE: usize = 0x08;
    const ARMY_OFF_ORDERS_VEC: usize = 0x98;
    const ORDER_VTABLE_RVA: usize = 0x295FA08;
    const ORDER_OFF_TYPE: usize = 0x30;
    const ORDER_OFF_ARMY_BACKREF: usize = 0x38;
    const ORDER_OFF_TYPE5_STATES_BEGIN: usize = 0xE0;
    const ORDER_OFF_TYPE5_STATES_SIZE: usize = 0xE8;
    const ORDER_TYPE5_OFFENSIVE: i32 = 5;
    const MAX_ARMY_ID: u32 = i32::MAX as u32;
    const MAX_ARMY_BODY_BRIDGE_ROWS: usize = 20_000;
    const MAX_ARMY_BODY_BRIDGE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
    const MAX_ARMY_ORDERS_PER_ARMY: u32 = 64;
    const MAX_ARMY_ORDER_TARGET_STATES: u32 = 5_000;
    const MAX_ARMY_ORDER_INLINE_TARGET_STATES: u32 = 256;
    const MAX_ARMY_ORDER_STATE_ID: u32 = 65_535;
    const ARMY_ORDER_SAMPLE_MIN_MS: u64 = 250;
    const LAND_COMBAT_MANAGER_OFFSET: usize = 0x260;
    const LAND_COMBAT_MANAGER_DATA: usize = 0x08;
    const LAND_COMBAT_MANAGER_COUNT: usize = 0x14;
    const LAND_COMBAT_MANAGER_BODY_DELTA: usize = 0x10;
    const LAND_COMBAT_VTABLE_RVA: usize = 0x2980CB8;
    const LAND_COMBAT_SIDE_VTABLE_RVA: usize = 0x2980AC0;
    const LAND_COMBAT_REGISTRY_VTABLE_RVA: usize = 0x2980BD8;
    const LAND_COMBAT_FRONT_VTABLE_RVA: usize = 0x294AE78;
    const LAND_COMBAT_TYPE: u32 = 62;
    const LAND_COMBAT_OFF_TYPE: usize = 0x08;
    const LAND_COMBAT_OFF_ID: usize = 0x0C;
    const LAND_COMBAT_OFF_ATTACKER_SIDE: usize = 0x18;
    const LAND_COMBAT_OFF_FRONT: usize = 0x28;
    const LAND_COMBAT_OFF_DURATION: usize = 0x30;
    const LAND_COMBAT_OFF_DAY: usize = 0x34;
    const LAND_COMBAT_FRONT_OFF_PROVINCE: usize = 0xA4;
    const LAND_COMBAT_SIDE_OFF_REGISTRY: usize = 0x18;
    const LAND_COMBAT_SIDE_OFF_DATA: usize = 0x20;
    const LAND_COMBAT_SIDE_OFF_CAPACITY: usize = 0x28;
    const LAND_COMBAT_SIDE_OFF_SIZE: usize = 0x2C;
    const LAND_COMBAT_REGISTRY_OFF_HANDLE: usize = 0x18;
    const LAND_COMBAT_REGISTRY_OFF_ATTACKER_SIDE: usize = 0x28;
    const LAND_COMBAT_REGISTRY_OFF_DEFENDER_SIDE: usize = 0x30;
    const LAND_COMBAT_REGISTRY_OFF_FRONT: usize = 0x38;
    const LAND_COMBAT_SIDE_UNIT_ALT_TYPE: u32 = 51;
    const MAX_LAND_COMBATS: u32 = 4_000;
    const MAX_LAND_COMBAT_SIDE_UNITS: u32 = 128;
    const MAX_LAND_COMBAT_AGE_DAYS: u32 = 100_000;
    const MAX_LAND_COMBAT_PROVINCE_ID: u32 = 1_000_000;
    const NAVAL_COMBAT_VTABLE_RVA: usize = 0x29B6660;
    const NAVAL_COMBAT_SIDE_VTABLE_RVA: usize = 0x29BC908;
    const NAVAL_COMBAT_REGISTRY_VTABLE_RVA: usize = 0x29B6588;
    const NAVAL_COMBAT_MEMBER_VTABLE_RVA: usize = 0x29F9608;
    const NAVAL_COMBAT_TYPE: u32 = 4713;
    const NAVAL_COMBAT_OFF_TYPEID: usize = 0x08;
    const NAVAL_COMBAT_OFF_ATTACKER_SIDE: usize = 0x18;
    const NAVAL_COMBAT_OFF_DEFENDER_SIDE: usize = 0x20;
    const NAVAL_COMBAT_OFF_FRONT: usize = 0x28;
    const NAVAL_COMBAT_SIDE_OFF_REGISTRY: usize = 0x18;
    const NAVAL_COMBAT_SIDE_OFF_COUNTRIES_DATA: usize = 0x68;
    const NAVAL_COMBAT_SIDE_OFF_COUNTRIES_SIZE: usize = 0x74;
    const NAVAL_COMBAT_SIDE_OFF_FEX_DATA: usize = 0xC8;
    const NAVAL_COMBAT_SIDE_OFF_FEX_SIZE: usize = 0xD4;
    const NAVAL_COMBAT_SIDE_OFF_LEADER: usize = 0x110;
    const NAVAL_COMBAT_SIDE_OFF_POSITIONING: usize = 0x288;
    const NAVAL_COMBAT_REGISTRY_OFF_HANDLE: usize = 0x18;
    const NAVAL_COMBAT_REGISTRY_OFF_ATTACKER_SIDE: usize = 0x28;
    const NAVAL_COMBAT_REGISTRY_OFF_DEFENDER_SIDE: usize = 0x30;
    const NAVAL_COMBAT_REGISTRY_OFF_FRONT: usize = 0x38;
    const NAVAL_COMBAT_FEX_OFF_MEMBERS_DATA: usize = 0x08;
    const NAVAL_COMBAT_FEX_OFF_MEMBERS_PACKED: usize = 0x10;
    const NAVAL_COMBAT_MEMBER_OFF_TYPEID: usize = 0x08;
    const NAVAL_COMBAT_MEMBER_OFF_OWNER_TAG: usize = 0x20;
    const NAVAL_COMBAT_MEMBER_OFF_MAX_STRENGTH: usize = 0x30;
    const NAVAL_COMBAT_MEMBER_OFF_NAME: usize = 0x48;
    const NAVAL_COMBAT_MEMBER_OFF_TYPE_DISPLAY: usize = 0x68;
    const NAVAL_COMBAT_MEMBER_OFF_ARCHETYPE: usize = 0x88;
    const NAVAL_COMBAT_MEMBER_OFF_STATE: usize = 0xE0;
    const NAVAL_COMBAT_MEMBER_OFF_ENGAGEMENT_WEIGHT: usize = 0xF8;
    const NAVAL_COMBAT_MEMBER_OFF_SLOT: usize = 0x120;
    const NAVAL_LEADER_NAME: usize = 0x20;
    const MAX_NAVAL_COMBAT_ID: u32 = i32::MAX as u32;
    const MAX_NAVAL_COMBAT_BODY_BRIDGE_ROWS: usize = 4_000;
    const MAX_NAVAL_COMBAT_BODY_BRIDGE_PAYLOAD_BYTES: usize = 512 * 1024;
    const MAX_NAVAL_COMBAT_SIDE_COUNTRIES: u32 = 64;
    const MAX_NAVAL_COMBAT_SIDE_GROUPS: u32 = 256;
    const MAX_NAVAL_COMBAT_SHIPS_PER_GROUP: u32 = 1_000;
    const MAX_NAVAL_COMBAT_SIDE_SHIPS: u32 = 20_000;
    const MAX_NAVAL_COMMANDER_NAME_LEN: usize = 128;
    const NAVAL_COMBAT_SHIP_TYPE_TAG: u32 = 51;
    const MAX_NAVAL_COMBAT_SHIP_INSTANCE_ID: u32 = 4_000_000;
    const MAX_NAVAL_COMBAT_MEMBER_STRING_LEN: usize = 128;
    const MAX_NAVAL_COMBAT_MEMBER_MAX_STRENGTH_RAW: u32 = 1_000_000_000;
    const MAX_NAVAL_COMBAT_MEMBER_ENGAGEMENT_WEIGHT_RAW: u32 = 10_000_000;
    const AIR_WING_VTABLE_RVA: usize = 0x29540C0;
    const AIR_WING_POOL_VTABLE_RVA: usize = 0x2954118;
    const AIR_WING_OWNER_COUNTRY_VTABLE_RVA: usize = 0x2932248;
    const AIR_WING_REGION_VTABLE_RVA: usize = 0x29468B8;
    const AIR_WING_REGION_ALT_VTABLE_RVA: usize = 0x2946908;
    const AIR_WING_TYPE: u32 = 69;
    const AIR_WING_OFF_TYPE: usize = 0x08;
    const AIR_WING_OFF_ID: usize = 0x0C;
    const AIR_WING_OFF_POOL: usize = 0x28;
    const AIR_WING_OFF_COUNT: usize = 0x6C;
    const AIR_WING_OFF_MISSION_MASK: usize = 0x90;
    const AIR_WING_OFF_MISSION_REGION: usize = 0xB0;
    const AIR_WING_OFF_EFFECTIVE_PLANES: usize = 0x160;
    const AIR_WING_POOL_OFF_COUNTRY: usize = 0x40;
    const AIR_WING_COUNTRY_OFF_TAG: usize = 0x90;
    const AIR_WING_REGION_OFF_ID: usize = 0x58;
    const MAX_AIR_WING_ID: u32 = i32::MAX as u32;
    const MAX_AIR_WING_BODY_BRIDGE_ROWS: usize = 20_000;
    const MAX_AIR_WING_BODY_BRIDGE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
    const MAX_AIR_WING_PLANES: u32 = 10_000_000;
    const MAX_AIR_WING_MISSION_REGION_ID: u32 = 65_535;
    const AIR_WING_SAMPLE_MIN_MS: u64 = 250;
    const PROJECT_FACILITY_VTABLE_RVA: usize = 0x294AB40;
    const PROJECT_CPROJECT_VTABLE_RVA: usize = 0x295C190;
    const PROJECT_CHARACTER_VTABLE_RVA: usize = 0x2957D40;
    const PROJECT_SCIENTIST_VTABLE_RVA: usize = 0x2957D98;
    const PROJECT_FACILITY_TYPE: u32 = 83;
    const PROJECT_PROJECT_TYPE: u32 = 86;
    const PROJECT_CHARACTER_TYPE: u32 = 73;
    const PROJECT_FACILITY_OFF_TYPE: usize = 0x08;
    const PROJECT_FACILITY_OFF_ID: usize = 0x0C;
    const PROJECT_FACILITY_OFF_PROVINCE: usize = 0x1C;
    const PROJECT_FACILITY_OFF_BUILDING: usize = 0x20;
    const PROJECT_FACILITY_OFF_ACTIVE_HANDLE: usize = 0x50;
    const PROJECT_FACILITY_OFF_ACTIVE_FLAG: usize = 0x54;
    const PROJECT_FACILITY_OFF_SCIENTIST_TYPE: usize = 0x58;
    const PROJECT_FACILITY_OFF_SCIENTIST_ID: usize = 0x5C;
    const PROJECT_FACILITY_OFF_UNREAD_REWARDS: usize = 0x178;
    const PROJECT_PROJECT_OFF_TYPE: usize = 0x08;
    const PROJECT_PROJECT_OFF_ID: usize = 0x0C;
    const PROJECT_PROJECT_OFF_DEF_HANDLE: usize = 0x18;
    const PROJECT_PROJECT_OFF_OWNER_TAG: usize = 0x60;
    const PROJECT_PROJECT_OFF_OWNER_FACILITY: usize = 0x64;
    const PROJECT_PROJECT_OFF_ITERATIONS: usize = 0x74;
    const PROJECT_CHARACTER_OFF_TYPE: usize = 0x08;
    const PROJECT_CHARACTER_OFF_ID: usize = 0x0C;
    const PROJECT_CHARACTER_OFF_DISPLAY: usize = 0x28;
    const PROJECT_CHARACTER_OFF_TOKEN: usize = 0x48;
    const PROJECT_CHARACTER_OFF_SCIENTIST_PTR: usize = 0xC0;
    const PROJECT_SCIENTIST_OFF_TRAIT_COUNT: usize = 0x38;
    const PROJECT_SCIENTIST_OFF_SKILLS_PTR: usize = 0x110;
    const PROJECT_SKILL_STRIDE: usize = 0x20;
    const PROJECT_SKILL_LEVEL_OFF: usize = 0x18;
    const PROJECT_SKILL_HEADER_BASE: u16 = 0x56F7;
    const MAX_PROJECT_FACILITIES: usize = 4_096;
    const MAX_PROJECT_ACTIVE_PROJECTS: usize = 4_096;
    const MAX_PROJECT_PROJECT_PINS: usize = 20_000;
    const MAX_PROJECT_SCIENTISTS: usize = 4_096;
    const MAX_PROJECT_ID: u32 = 10_000_000;
    const MAX_PROJECT_PROVINCE_ID: u32 = 1_000_000;
    const MAX_PROJECT_HANDLE: u32 = 10_000_000;
    const MAX_PROJECT_ITERATIONS: u32 = 1_000_000;
    const MAX_PROJECT_TRAITS: u32 = 256;
    const MAX_PROJECT_SKILL_LEVEL: i32 = 1_000;
    const MAX_PROJECT_BUILDING_LEN: usize = 96;
    const MAX_PROJECT_SCIENTIST_TOKEN_LEN: usize = 160;
    const MAX_PROJECT_SCIENTIST_DISPLAY_LEN: usize = 160;
    const FP_SCALE: f64 = 100000.0;
    const MAX_COUNTRY_ACE_COUNT: u32 = 100_000;
    const MAX_COUNTRY_CONVOYS_DESTROYED: u32 = 100_000_000;
    const MAX_COUNTRY_ACCIDENTS_SCORE: f64 = 10.0;
    const MAX_COUNTRY_COSMETIC_TAG_LEN: usize = 96;
    const DAILY_UPDATE_SENTINEL: [u8; 16] = [
        0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D,
        0xAC,
    ];
    const GET_LONG_STRING_SENTINEL: [u8; 16] = [
        0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x57,
    ];
    static FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: Lpvoid,
        allocation_base: Lpvoid,
        allocation_protect: Dword,
        partition_id: u16,
        region_size: usize,
        state: Dword,
        protect: Dword,
        type_: Dword,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CloseHandle(handle: Handle) -> Bool;
        fn CreateFileW(
            filename: *const u16,
            desired_access: Dword,
            share_mode: Dword,
            security_attributes: Lpvoid,
            creation_disposition: Dword,
            flags_and_attributes: Dword,
            template_file: Handle,
        ) -> Handle;
        fn CreateThread(
            attrs: Lpvoid,
            stack_size: usize,
            start: ThreadStart,
            param: Lpvoid,
            flags: Dword,
            thread_id: *mut Dword,
        ) -> Handle;
        fn DisableThreadLibraryCalls(module: Hmodule) -> Bool;
        fn GetCurrentProcessId() -> Dword;
        fn GetModuleFileNameW(module: Hmodule, filename: *mut u16, size: Dword) -> Dword;
        fn GetModuleHandleW(module_name: *const u16) -> Hmodule;
        fn GetTickCount64() -> u64;
        fn VirtualQuery(
            address: *const c_void,
            buffer: *mut MemoryBasicInformation,
            length: usize,
        ) -> usize;
        fn WriteFile(
            file: Handle,
            buffer: *const c_void,
            bytes_to_write: Dword,
            bytes_written: *mut Dword,
            overlapped: Lpvoid,
        ) -> Bool;
    }

    pub unsafe fn dll_main(module: Hmodule, reason: Dword) -> Bool {
        if reason == DLL_PROCESS_ATTACH {
            unsafe {
                DLL_MODULE = module;
                DisableThreadLibraryCalls(module);
                let handle = CreateThread(
                    ptr::null_mut(),
                    0,
                    Some(attach_worker),
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                );
                if !handle.is_null() {
                    CloseHandle(handle);
                }
            }
        }
        1
    }

    unsafe extern "system" fn attach_worker(_param: Lpvoid) -> Dword {
        write_attach_heartbeat();
        0
    }

    fn write_attach_heartbeat() {
        let Some(mut path) = module_path() else {
            return;
        };
        let control = if is_hoi4_process() {
            collector_control(&path)
        } else {
            None
        };
        let state = if is_hoi4_process() {
            Some(CollectorState::resolve(control.as_ref()))
        } else {
            None
        };
        let pid = unsafe { GetCurrentProcessId() };
        let sidecar_log = control
            .as_ref()
            .map(|control| control.sidecar_log)
            .unwrap_or(true);
        path.set_file_name("collector_attach.jsonl");
        let mut file = if sidecar_log {
            OpenOptions::new().create(true).append(true).open(path).ok()
        } else {
            None
        };
        let mut pipe = open_pipe(
            pid,
            control
                .as_ref()
                .and_then(|control| control.pipe_token.as_deref()),
        );
        if let Some(control) = control {
            let worker_started = Instant::now();
            let deadline = worker_started + Duration::from_millis(control.duration_ms as u64);
            let mut session_sequence = 1u64;
            let mut last_scheduled_game_hour: Option<i32> = None;
            let mut previous_division_movement: BTreeMap<u32, DivisionMovementSample> =
                BTreeMap::new();
            let mut land_combat_tracker = LandCombatTracker::default();
            let mut army_order_tracker = ArmyOrderTracker::default();
            let mut last_army_order_sample_elapsed_ms: Option<u64> = None;
            let mut naval_combat_tracker = NavalCombatTracker::default();
            let mut naval_combat_member_tracker = NavalCombatMemberTracker::default();
            let mut air_wing_tracker = AirWingTracker::default();
            let mut last_air_wing_sample_elapsed_ms: Option<u64> = None;
            while Instant::now() <= deadline {
                let elapsed_wall_ms = duration_millis_u64(worker_started.elapsed());
                let capture_boundary = capture_boundary_for_frame(
                    state.as_ref(),
                    &control,
                    &mut last_scheduled_game_hour,
                );
                let line = build_frame_line(
                    pid,
                    state.as_ref(),
                    Some(&control),
                    Some(session_sequence),
                    Some(elapsed_wall_ms),
                    Some(capture_boundary),
                );
                write_sinks(file.as_mut(), pipe, line.as_bytes());
                session_sequence = session_sequence.saturating_add(1);
                if let Some(line) = build_clock_raw_domain_delta_line(
                    pid,
                    state.as_ref(),
                    &control,
                    Some(session_sequence),
                    Some(elapsed_wall_ms),
                    capture_boundary,
                ) {
                    write_sinks(file.as_mut(), pipe, line.as_bytes());
                    session_sequence = session_sequence.saturating_add(1);
                }
                for line in build_division_raw_domain_delta_lines(
                    pid,
                    state.as_ref(),
                    &control,
                    &mut previous_division_movement,
                    session_sequence,
                    Some(elapsed_wall_ms),
                    capture_boundary,
                ) {
                    write_sinks(file.as_mut(), pipe, line.as_bytes());
                    session_sequence = session_sequence.saturating_add(1);
                }
                for line in build_land_combat_raw_domain_delta_lines(
                    pid,
                    state.as_ref(),
                    &control,
                    &mut land_combat_tracker,
                    session_sequence,
                    Some(elapsed_wall_ms),
                    capture_boundary,
                ) {
                    write_sinks(file.as_mut(), pipe, line.as_bytes());
                    session_sequence = session_sequence.saturating_add(1);
                }
                let sample_army_orders = last_army_order_sample_elapsed_ms
                    .map(|last| elapsed_wall_ms.saturating_sub(last) >= ARMY_ORDER_SAMPLE_MIN_MS)
                    .unwrap_or(true);
                if sample_army_orders {
                    for line in build_army_order_raw_domain_delta_lines(
                        pid,
                        state.as_ref(),
                        &control,
                        &mut army_order_tracker,
                        session_sequence,
                        Some(elapsed_wall_ms),
                        capture_boundary,
                    ) {
                        write_sinks(file.as_mut(), pipe, line.as_bytes());
                        session_sequence = session_sequence.saturating_add(1);
                    }
                    last_army_order_sample_elapsed_ms = Some(elapsed_wall_ms);
                }
                for line in build_naval_combat_raw_domain_delta_lines(
                    pid,
                    state.as_ref(),
                    &control,
                    &mut naval_combat_tracker,
                    &mut naval_combat_member_tracker,
                    session_sequence,
                    Some(elapsed_wall_ms),
                    capture_boundary,
                ) {
                    write_sinks(file.as_mut(), pipe, line.as_bytes());
                    session_sequence = session_sequence.saturating_add(1);
                }
                let sample_air_wings = last_air_wing_sample_elapsed_ms
                    .map(|last| elapsed_wall_ms.saturating_sub(last) >= AIR_WING_SAMPLE_MIN_MS)
                    .unwrap_or(true);
                if sample_air_wings {
                    for line in build_air_wing_raw_domain_delta_lines(
                        pid,
                        state.as_ref(),
                        &control,
                        &mut air_wing_tracker,
                        session_sequence,
                        Some(elapsed_wall_ms),
                        capture_boundary,
                    ) {
                        write_sinks(file.as_mut(), pipe, line.as_bytes());
                        session_sequence = session_sequence.saturating_add(1);
                    }
                    last_air_wing_sample_elapsed_ms = Some(elapsed_wall_ms);
                }
                thread::sleep(Duration::from_millis(control.sample_ms as u64));
            }
        } else {
            let line = build_frame_line(pid, state.as_ref(), None, None, None, None);
            write_sinks(file.as_mut(), pipe, line.as_bytes());
        }
        if let Some(handle) = pipe.take() {
            unsafe {
                CloseHandle(handle);
            }
        }
    }

    struct CollectorControl {
        duration_ms: u32,
        sample_ms: u32,
        country_object_scan: bool,
        special_projects_scan: bool,
        country_object_bridge: Vec<CountryObjectBridgeEntry>,
        country_object_bridge_valid: bool,
        country_record_bridge: Vec<CountryObjectBridgeEntry>,
        country_record_bridge_valid: bool,
        division_body_bridge: Vec<DivisionBodyBridgeEntry>,
        division_body_bridge_valid: bool,
        army_body_bridge: Vec<ArmyBodyBridgeEntry>,
        army_body_bridge_valid: bool,
        naval_combat_body_bridge: Vec<NavalCombatBodyBridgeEntry>,
        naval_combat_body_bridge_valid: bool,
        air_wing_body_bridge: Vec<AirWingBodyBridgeEntry>,
        air_wing_body_bridge_valid: bool,
        map_control: MapControlContract,
        advisor_token_filter: BTreeMap<String, AdvisorTokenInfo>,
        advisor_token_filter_valid: bool,
        idea_group_filter: BTreeMap<String, String>,
        idea_group_filter_valid: bool,
        scheduled_snapshot_step_hours: u32,
        pipe_token: Option<String>,
        session_id: Option<String>,
        sidecar_log: bool,
    }

    #[derive(Clone, Copy)]
    struct CountryObjectBridgeEntry {
        tag_db_id: u32,
        object_ptr: usize,
    }

    #[derive(Clone, Copy)]
    struct DivisionBodyBridgeEntry {
        div_id: u32,
        body_ptr: usize,
    }

    #[derive(Clone, Copy)]
    struct ArmyBodyBridgeEntry {
        army_id: u32,
        body_ptr: usize,
    }

    #[derive(Clone, Copy)]
    struct NavalCombatBodyBridgeEntry {
        combat_id: u32,
        body_ptr: usize,
    }

    #[derive(Clone, Copy)]
    struct AirWingBodyBridgeEntry {
        owner_tag_db_id: u32,
        wing_id: u32,
        body_ptr: usize,
    }

    #[derive(Clone, PartialEq, Eq)]
    struct AdvisorTokenInfo {
        slot: Option<String>,
        ledger: Option<String>,
        character: Option<String>,
    }

    #[derive(Clone)]
    struct MapControlContract {
        state_roots: Vec<MapControlStateRootEntry>,
        state_roots_valid: bool,
        province_bridge: Option<MapControlProvinceBridge>,
        province_bridge_valid: bool,
        controller_bridge: Vec<MapControlControllerBridgeEntry>,
        controller_bridge_valid: bool,
    }

    impl MapControlContract {
        fn empty() -> Self {
            Self {
                state_roots: Vec::new(),
                state_roots_valid: true,
                province_bridge: None,
                province_bridge_valid: true,
                controller_bridge: Vec::new(),
                controller_bridge_valid: true,
            }
        }

        fn state_roots_well_formed(&self) -> bool {
            if self.state_roots.len() > MAX_MAP_CONTROL_STATE_ROOT_ROWS {
                return false;
            }
            let mut seen_state_ids: BTreeMap<u32, ()> = BTreeMap::new();
            let mut seen_state_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
            for row in &self.state_roots {
                if row.state_id == 0 || row.state_id > MAX_MAP_CONTROL_STATE_ID {
                    return false;
                }
                if !is_map_control_heap_pointer(row.state_ptr) {
                    return false;
                }
                if seen_state_ids.insert(row.state_id, ()).is_some()
                    || seen_state_ptrs.insert(row.state_ptr, ()).is_some()
                {
                    return false;
                }
            }
            true
        }

        fn province_bridge_well_formed(&self) -> bool {
            match self.province_bridge.as_ref() {
                Some(bridge) => validate_map_control_province_bridge(bridge),
                None => true,
            }
        }

        fn controller_bridge_well_formed(&self) -> bool {
            if self.controller_bridge.len() > MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS {
                return false;
            }
            let mut seen_tag_ids: BTreeMap<u32, ()> = BTreeMap::new();
            let mut seen_controller_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
            for row in &self.controller_bridge {
                if row.tag_db_id == 0 || row.tag_db_id > 1024 {
                    return false;
                }
                if !is_map_control_heap_pointer(row.controller_ptr) {
                    return false;
                }
                if seen_tag_ids.insert(row.tag_db_id, ()).is_some()
                    || seen_controller_ptrs
                        .insert(row.controller_ptr, ())
                        .is_some()
                {
                    return false;
                }
            }
            true
        }
    }

    #[derive(Clone, Copy)]
    struct MapControlStateRootEntry {
        state_id: u32,
        state_ptr: usize,
    }

    #[derive(Clone, Copy)]
    struct MapControlProvinceBridge {
        province_array_ptr: usize,
        province_count: u32,
        province_stride: u32,
        controller_ptr_offset: u32,
    }

    #[derive(Clone, Copy)]
    struct MapControlControllerBridgeEntry {
        tag_db_id: u32,
        controller_ptr: usize,
    }

    fn collector_control(module_path: &PathBuf) -> Option<CollectorControl> {
        let mut path = module_path.clone();
        path.set_file_name("collector_control.json");
        let text = read_to_string(path).ok()?;
        if !text.contains("\"mode\"") || !text.contains("hourly_poll") {
            return None;
        }
        let duration_ms = parse_json_u32(&text, "duration_ms").unwrap_or(5_000);
        let sample_ms = parse_json_u32(&text, "sample_ms").unwrap_or(50);
        let scheduled_snapshot_step_hours =
            parse_json_u32(&text, "scheduled_snapshot_step_hours").unwrap_or(1);
        let country_object_scan = parse_json_bool(&text, "country_object_scan").unwrap_or(false);
        let special_projects_scan =
            parse_json_bool(&text, "special_projects_scan").unwrap_or(false);
        let (country_object_bridge, country_object_bridge_valid) =
            match parse_json_string(&text, "country_object_bridge") {
                Some(value) => match parse_country_object_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (country_record_bridge, country_record_bridge_valid) =
            match parse_json_string(&text, "country_record_bridge") {
                Some(value) => match parse_country_object_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (division_body_bridge, division_body_bridge_valid) =
            match parse_json_string(&text, "division_body_bridge") {
                Some(value) => match parse_division_body_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (army_body_bridge, army_body_bridge_valid) =
            match parse_json_string(&text, "army_body_bridge") {
                Some(value) => match parse_army_body_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (naval_combat_body_bridge, naval_combat_body_bridge_valid) =
            match parse_json_string(&text, "naval_combat_body_bridge") {
                Some(value) => match parse_naval_combat_body_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (air_wing_body_bridge, air_wing_body_bridge_valid) =
            match parse_json_string(&text, "air_wing_body_bridge") {
                Some(value) => match parse_air_wing_body_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (advisor_token_filter, advisor_token_filter_valid) =
            match parse_json_string(&text, "advisor_token_filter") {
                Some(value) => match parse_advisor_token_filter(&value) {
                    Some(tokens) => (tokens, true),
                    None => (BTreeMap::new(), false),
                },
                None => (BTreeMap::new(), true),
            };
        let (idea_group_filter, idea_group_filter_valid) =
            match parse_json_string(&text, "idea_group_filter") {
                Some(value) => match parse_idea_group_filter(&value) {
                    Some(rows) => (rows, true),
                    None => (BTreeMap::new(), false),
                },
                None => (BTreeMap::new(), true),
            };
        let map_control = parse_map_control_contract(&text);
        let pipe_token = parse_json_string(&text, "pipe_token").and_then(clean_pipe_token);
        let session_id = parse_json_string(&text, "session_id").and_then(clean_session_id);
        let sidecar_log = parse_json_bool(&text, "sidecar_log").unwrap_or(false);
        Some(CollectorControl {
            duration_ms: duration_ms.clamp(100, 30_000),
            sample_ms: sample_ms.clamp(10, 5_000),
            country_object_scan,
            special_projects_scan,
            country_object_bridge,
            country_object_bridge_valid,
            country_record_bridge,
            country_record_bridge_valid,
            division_body_bridge,
            division_body_bridge_valid,
            army_body_bridge,
            army_body_bridge_valid,
            naval_combat_body_bridge,
            naval_combat_body_bridge_valid,
            air_wing_body_bridge,
            air_wing_body_bridge_valid,
            map_control,
            advisor_token_filter,
            advisor_token_filter_valid,
            idea_group_filter,
            idea_group_filter_valid,
            scheduled_snapshot_step_hours: scheduled_snapshot_step_hours.clamp(1, 24),
            pipe_token,
            session_id,
            sidecar_log,
        })
    }

    fn parse_json_u32(text: &str, key: &str) -> Option<u32> {
        let marker = format!("\"{key}\"");
        let start = text.find(&marker)?;
        let rest = &text[start + marker.len()..];
        let colon = rest.find(':')?;
        let digits: String = rest[colon + 1..]
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok()
    }

    fn parse_json_bool(text: &str, key: &str) -> Option<bool> {
        let marker = format!("\"{key}\"");
        let start = text.find(&marker)?;
        let rest = &text[start + marker.len()..];
        let colon = rest.find(':')?;
        let value: String = rest[colon + 1..]
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    fn parse_json_string(text: &str, key: &str) -> Option<String> {
        let marker = format!("\"{key}\"");
        let start = text.find(&marker)?;
        let rest = &text[start + marker.len()..];
        let colon = rest.find(':')?;
        let value = rest[colon + 1..].trim_start();
        let value = value.strip_prefix('"')?;
        let end = value.find('"')?;
        Some(value[..end].to_string())
    }

    fn clean_pipe_token(token: String) -> Option<String> {
        if token.is_empty() || token.len() > 64 {
            return None;
        }
        if token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            Some(token)
        } else {
            None
        }
    }

    fn clean_session_id(session_id: String) -> Option<String> {
        if session_id.is_empty() || session_id.len() > 96 {
            return None;
        }
        if session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            Some(session_id)
        } else {
            None
        }
    }

    fn parse_country_object_bridge(value: &str) -> Option<Vec<CountryObjectBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > 16_384 {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_tag_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_objects: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (tag_raw, ptr_raw) = raw.split_once(':')?;
            let tag_db_id = tag_raw.trim().parse::<u32>().ok()?;
            if tag_db_id == 0 || tag_db_id > 1024 {
                return None;
            }
            let object_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if object_ptr == 0 || object_ptr % 8 != 0 {
                return None;
            }
            if seen_tag_ids.insert(tag_db_id, ()).is_some()
                || seen_objects.insert(object_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(CountryObjectBridgeEntry {
                tag_db_id,
                object_ptr,
            });
            if entries.len() > MAX_COUNTRY_OBJECT_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn parse_division_body_bridge(value: &str) -> Option<Vec<DivisionBodyBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_DIVISION_BODY_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_div_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (div_raw, ptr_raw) = raw.split_once(':')?;
            let div_id = parse_bridge_u32(div_raw.trim())?;
            if div_id == 0 || div_id > MAX_DIVISION_BODY_BRIDGE_DIV_ID {
                return None;
            }
            let body_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(body_ptr) {
                return None;
            }
            if seen_div_ids.insert(div_id, ()).is_some()
                || seen_body_ptrs.insert(body_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(DivisionBodyBridgeEntry { div_id, body_ptr });
            if entries.len() > MAX_DIVISION_BODY_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn division_body_bridge_well_formed(rows: &[DivisionBodyBridgeEntry]) -> bool {
        if rows.len() > MAX_DIVISION_BODY_BRIDGE_ROWS {
            return false;
        }
        let mut seen_div_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for row in rows {
            if row.div_id == 0 || row.div_id > MAX_DIVISION_BODY_BRIDGE_DIV_ID {
                return false;
            }
            if !is_map_control_heap_pointer(row.body_ptr) {
                return false;
            }
            if seen_div_ids.insert(row.div_id, ()).is_some()
                || seen_body_ptrs.insert(row.body_ptr, ()).is_some()
            {
                return false;
            }
        }
        true
    }

    fn parse_army_body_bridge(value: &str) -> Option<Vec<ArmyBodyBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_ARMY_BODY_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_army_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (army_raw, ptr_raw) = raw.split_once(':')?;
            let army_id = parse_bridge_u32(army_raw.trim())?;
            if army_id == 0 || army_id > MAX_ARMY_ID {
                return None;
            }
            let body_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(body_ptr) {
                return None;
            }
            if seen_army_ids.insert(army_id, ()).is_some()
                || seen_body_ptrs.insert(body_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(ArmyBodyBridgeEntry { army_id, body_ptr });
            if entries.len() > MAX_ARMY_BODY_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn army_body_bridge_well_formed(rows: &[ArmyBodyBridgeEntry]) -> bool {
        if rows.len() > MAX_ARMY_BODY_BRIDGE_ROWS {
            return false;
        }
        let mut seen_army_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for row in rows {
            if row.army_id == 0 || row.army_id > MAX_ARMY_ID {
                return false;
            }
            if !is_map_control_heap_pointer(row.body_ptr) {
                return false;
            }
            if seen_army_ids.insert(row.army_id, ()).is_some()
                || seen_body_ptrs.insert(row.body_ptr, ()).is_some()
            {
                return false;
            }
        }
        true
    }

    fn parse_naval_combat_body_bridge(value: &str) -> Option<Vec<NavalCombatBodyBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_NAVAL_COMBAT_BODY_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_combat_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (id_raw, ptr_raw) = raw.split_once(':')?;
            let combat_id = parse_bridge_u32(id_raw.trim())?;
            if combat_id == 0 || combat_id > MAX_NAVAL_COMBAT_ID {
                return None;
            }
            let body_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(body_ptr) {
                return None;
            }
            if seen_combat_ids.insert(combat_id, ()).is_some()
                || seen_body_ptrs.insert(body_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(NavalCombatBodyBridgeEntry {
                combat_id,
                body_ptr,
            });
            if entries.len() > MAX_NAVAL_COMBAT_BODY_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn naval_combat_body_bridge_well_formed(rows: &[NavalCombatBodyBridgeEntry]) -> bool {
        if rows.len() > MAX_NAVAL_COMBAT_BODY_BRIDGE_ROWS {
            return false;
        }
        let mut seen_combat_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for row in rows {
            if row.combat_id == 0 || row.combat_id > MAX_NAVAL_COMBAT_ID {
                return false;
            }
            if !is_map_control_heap_pointer(row.body_ptr) {
                return false;
            }
            if seen_combat_ids.insert(row.combat_id, ()).is_some()
                || seen_body_ptrs.insert(row.body_ptr, ()).is_some()
            {
                return false;
            }
        }
        true
    }

    fn parse_air_wing_body_bridge(value: &str) -> Option<Vec<AirWingBodyBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_AIR_WING_BODY_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_keys: BTreeMap<(u32, u32), ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let mut parts = raw.split(':');
            let owner_raw = parts.next()?;
            let wing_raw = parts.next()?;
            let ptr_raw = parts.next()?;
            if parts.next().is_some() {
                return None;
            }
            let owner_tag_db_id = parse_bridge_u32(owner_raw.trim())?;
            if owner_tag_db_id == 0 || owner_tag_db_id > 1024 {
                return None;
            }
            let wing_id = parse_bridge_u32(wing_raw.trim())?;
            if wing_id == 0 || wing_id > MAX_AIR_WING_ID {
                return None;
            }
            let body_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(body_ptr) {
                return None;
            }
            if seen_keys.insert((owner_tag_db_id, wing_id), ()).is_some()
                || seen_body_ptrs.insert(body_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(AirWingBodyBridgeEntry {
                owner_tag_db_id,
                wing_id,
                body_ptr,
            });
            if entries.len() > MAX_AIR_WING_BODY_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn air_wing_body_bridge_well_formed(rows: &[AirWingBodyBridgeEntry]) -> bool {
        if rows.len() > MAX_AIR_WING_BODY_BRIDGE_ROWS {
            return false;
        }
        let mut seen_keys: BTreeMap<(u32, u32), ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for row in rows {
            if row.owner_tag_db_id == 0 || row.owner_tag_db_id > 1024 {
                return false;
            }
            if row.wing_id == 0 || row.wing_id > MAX_AIR_WING_ID {
                return false;
            }
            if !is_map_control_heap_pointer(row.body_ptr) {
                return false;
            }
            if seen_keys
                .insert((row.owner_tag_db_id, row.wing_id), ())
                .is_some()
                || seen_body_ptrs.insert(row.body_ptr, ()).is_some()
            {
                return false;
            }
        }
        true
    }

    fn parse_map_control_contract(text: &str) -> MapControlContract {
        let mut contract = MapControlContract::empty();
        if let Some(value) = parse_json_string(text, "map_control_state_roots") {
            match parse_map_control_state_roots(&value) {
                Some(rows) => contract.state_roots = rows,
                None => contract.state_roots_valid = false,
            }
        }
        if let Some(value) = parse_json_string(text, "map_control_province_bridge") {
            if !value.trim().is_empty() {
                match parse_map_control_province_bridge(&value) {
                    Some(bridge) => contract.province_bridge = Some(bridge),
                    None => contract.province_bridge_valid = false,
                }
            }
        }
        if let Some(value) = parse_json_string(text, "map_control_controller_bridge") {
            match parse_map_control_controller_bridge(&value) {
                Some(rows) => contract.controller_bridge = rows,
                None => contract.controller_bridge_valid = false,
            }
        }
        contract
    }

    fn parse_advisor_token_filter(value: &str) -> Option<BTreeMap<String, AdvisorTokenInfo>> {
        if value.trim().is_empty() {
            return Some(BTreeMap::new());
        }
        if value.len() > MAX_ADVISOR_TOKEN_FILTER_PAYLOAD_BYTES {
            return None;
        }
        let mut tokens = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (token, info) = parse_advisor_token_filter_row(raw)?;
            if !is_script_token(token) {
                return None;
            }
            if let Some(previous) = tokens.insert(token.to_string(), info) {
                if previous != *tokens.get(token)? {
                    return None;
                }
            }
            if tokens.len() > MAX_ADVISOR_TOKEN_FILTER_TOKENS {
                return None;
            }
        }
        Some(tokens)
    }

    fn parse_advisor_token_filter_row(raw: &str) -> Option<(&str, AdvisorTokenInfo)> {
        let Some((token_raw, meta_raw)) = raw.split_once('=') else {
            return Some((
                raw.trim(),
                AdvisorTokenInfo {
                    slot: None,
                    ledger: None,
                    character: None,
                },
            ));
        };
        let token = token_raw.trim();
        let mut parts = meta_raw.split('|');
        let slot = clean_optional_script_token(parts.next().unwrap_or("").trim())?;
        let ledger = clean_optional_script_token(parts.next().unwrap_or("").trim())?;
        let character = clean_optional_script_token(parts.next().unwrap_or("").trim())?;
        if parts.next().is_some() {
            return None;
        }
        Some((
            token,
            AdvisorTokenInfo {
                slot,
                ledger,
                character,
            },
        ))
    }

    fn clean_optional_script_token(value: &str) -> Option<Option<String>> {
        if value.is_empty() {
            return Some(None);
        }
        if is_script_token(value) {
            Some(Some(value.to_string()))
        } else {
            None
        }
    }

    fn parse_idea_group_filter(value: &str) -> Option<BTreeMap<String, String>> {
        if value.trim().is_empty() {
            return Some(BTreeMap::new());
        }
        if value.len() > MAX_IDEA_GROUP_FILTER_PAYLOAD_BYTES {
            return None;
        }
        let mut rows = BTreeMap::new();
        for raw in value.split(';') {
            let (token_raw, group_raw) = raw.split_once('=')?;
            let token = token_raw.trim();
            let group = group_raw.trim();
            if !is_script_token(token) || !is_script_token(group) {
                return None;
            }
            if let Some(previous) = rows.insert(token.to_string(), group.to_string()) {
                if previous != group {
                    return None;
                }
            }
            if rows.len() > MAX_IDEA_GROUP_FILTER_ROWS {
                return None;
            }
        }
        Some(rows)
    }

    fn parse_map_control_state_roots(value: &str) -> Option<Vec<MapControlStateRootEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_MAP_CONTROL_STATE_ROOT_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_state_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_state_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (state_raw, ptr_raw) = raw.split_once(':')?;
            let state_id = parse_bridge_u32(state_raw.trim())?;
            if state_id == 0 || state_id > MAX_MAP_CONTROL_STATE_ID {
                return None;
            }
            let state_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(state_ptr) {
                return None;
            }
            if seen_state_ids.insert(state_id, ()).is_some()
                || seen_state_ptrs.insert(state_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(MapControlStateRootEntry {
                state_id,
                state_ptr,
            });
            if entries.len() > MAX_MAP_CONTROL_STATE_ROOT_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn parse_map_control_province_bridge(value: &str) -> Option<MapControlProvinceBridge> {
        if value.len() > MAX_MAP_CONTROL_PROVINCE_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut province_array_ptr = None;
        let mut province_count = None;
        let mut province_stride = None;
        let mut controller_ptr_offset = None;
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (key, raw_value) = raw.split_once('=')?;
            let key = key.trim();
            let raw_value = raw_value.trim();
            match key {
                "province_array" => {
                    if province_array_ptr.is_some() {
                        return None;
                    }
                    province_array_ptr = Some(parse_bridge_pointer(raw_value)?);
                }
                "province_count" => {
                    if province_count.is_some() {
                        return None;
                    }
                    province_count = Some(parse_bridge_u32(raw_value)?);
                }
                "province_stride" => {
                    if province_stride.is_some() {
                        return None;
                    }
                    province_stride = Some(parse_bridge_u32(raw_value)?);
                }
                "controller_ptr_offset" => {
                    if controller_ptr_offset.is_some() {
                        return None;
                    }
                    controller_ptr_offset = Some(parse_bridge_u32(raw_value)?);
                }
                _ => return None,
            }
        }
        let bridge = MapControlProvinceBridge {
            province_array_ptr: province_array_ptr?,
            province_count: province_count?,
            province_stride: province_stride?,
            controller_ptr_offset: controller_ptr_offset?,
        };
        validate_map_control_province_bridge(&bridge).then_some(bridge)
    }

    fn parse_map_control_controller_bridge(
        value: &str,
    ) -> Option<Vec<MapControlControllerBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_MAP_CONTROL_CONTROLLER_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_tag_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_controller_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (tag_raw, ptr_raw) = raw.split_once(':')?;
            let tag_db_id = parse_bridge_u32(tag_raw.trim())?;
            if tag_db_id == 0 || tag_db_id > 1024 {
                return None;
            }
            let controller_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(controller_ptr) {
                return None;
            }
            if seen_tag_ids.insert(tag_db_id, ()).is_some()
                || seen_controller_ptrs.insert(controller_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(MapControlControllerBridgeEntry {
                tag_db_id,
                controller_ptr,
            });
            if entries.len() > MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn validate_map_control_province_bridge(bridge: &MapControlProvinceBridge) -> bool {
        if !is_map_control_heap_pointer(bridge.province_array_ptr) {
            return false;
        }
        if bridge.province_count == 0 || bridge.province_count > MAX_MAP_CONTROL_PROVINCE_COUNT {
            return false;
        }
        if bridge.province_stride < MAP_CONTROL_POINTER_WIDTH
            || bridge.province_stride > MAX_MAP_CONTROL_PROVINCE_STRIDE
            || bridge.province_stride % MAP_CONTROL_POINTER_WIDTH != 0
        {
            return false;
        }
        if bridge.controller_ptr_offset % MAP_CONTROL_POINTER_WIDTH != 0 {
            return false;
        }
        let Some(controller_end) = bridge
            .controller_ptr_offset
            .checked_add(MAP_CONTROL_POINTER_WIDTH)
        else {
            return false;
        };
        if controller_end > bridge.province_stride {
            return false;
        }
        let Some(span) =
            (bridge.province_count as usize).checked_mul(bridge.province_stride as usize)
        else {
            return false;
        };
        bridge
            .province_array_ptr
            .checked_add(span)
            .is_some_and(|end| end <= HEAP_HI)
    }

    fn is_map_control_heap_pointer(value: usize) -> bool {
        value >= HEAP_LO && value < HEAP_HI && value % (MAP_CONTROL_POINTER_WIDTH as usize) == 0
    }

    fn parse_bridge_u32(value: &str) -> Option<u32> {
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            value.parse::<u32>().ok()
        }
    }

    fn parse_bridge_pointer(value: &str) -> Option<usize> {
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            usize::from_str_radix(hex, 16).ok()
        } else {
            value.parse::<usize>().ok()
        }
    }

    fn build_frame_line(
        pid: Dword,
        state: Option<&CollectorState>,
        control: Option<&CollectorControl>,
        session_sequence: Option<u64>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: Option<&str>,
    ) -> String {
        let tick = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let scheduled_frame = capture_boundary == Some("scheduled_hour_snapshot");
        let include_scheduled_map = scheduled_frame;
        let include_scheduled_country_fuel = scheduled_frame;
        let include_scheduled_country_production = scheduled_frame;
        let include_scheduled_country_focus = scheduled_frame;
        let include_scheduled_country_research = scheduled_frame;
        let include_scheduled_country_politics = scheduled_frame;
        let include_scheduled_country_politics_residuals = scheduled_frame;
        let include_scheduled_country_decisions = scheduled_frame;
        let include_scheduled_projects = scheduled_frame;
        let state_rows = scheduled_state_rows_json(state, control, include_scheduled_map);
        let province_rows = scheduled_province_rows_json(state, control, include_scheduled_map);
        let special_projects = scheduled_special_projects_json(state, include_scheduled_projects);
        let (probe, domains) = root_probe_json(
            state,
            state_rows.as_deref(),
            province_rows.as_deref(),
            special_projects.as_deref(),
            include_scheduled_country_fuel,
            include_scheduled_country_production,
            include_scheduled_country_focus,
            include_scheduled_country_research,
            include_scheduled_country_politics,
            include_scheduled_country_politics_residuals,
            include_scheduled_country_decisions,
            control.map(|control| &control.advisor_token_filter),
            control.map(|control| &control.idea_group_filter),
        );
        let control_meta =
            control_metadata_json(control, session_sequence, elapsed_wall_ms, capture_boundary);
        let quality = if probe.contains("\"status\":\"verified\"") {
            "root_probe_verified"
        } else if probe.contains("\"status\":\"profile_mismatch\"") {
            "profile_mismatch"
        } else {
            "attach_noop"
        };
        let special_projects_field = special_projects
            .map(|rows| format!(",\"special_projects\":{rows}"))
            .unwrap_or_default();
        format!(
            "{{\"type\":\"collector_started\",\"backend\":\"in_process_collector\",\
             \"collector_version\":\"{}\",\"abi_version\":{},\"process_id\":{},\
             \"tick_ms\":{},\"sequence\":{},\"profile_id\":\"{}\",\
             \"domains\":{},{}\"quality_status\":\"{}\",{}{} }}\n",
            super::COLLECTOR_VERSION,
            super::ABI_VERSION,
            pid,
            tick,
            sequence,
            super::COLLECTOR_PROFILE_ID,
            domains,
            control_meta,
            quality,
            probe,
            special_projects_field
        )
    }

    const RAW_SOURCE: &str = "C";
    const RAW_CLOCK_DOMAIN: &str = "clock";
    const RAW_CLOCK_ENTITY_KEY: &str = "clock:game_date";
    const RAW_DIVISIONS_DOMAIN: &str = "divisions";
    const RAW_LAND_COMBATS_DOMAIN: &str = "land_combats";
    const RAW_ARMY_ORDERS_DOMAIN: &str = "army_orders";
    const RAW_NAVAL_COMBATS_DOMAIN: &str = "naval_air_combats";
    const RAW_NAVAL_COMBAT_MEMBERS_DOMAIN: &str = "naval_combat_members";
    const RAW_AIR_WINGS_DOMAIN: &str = "air_wings";
    const RAW_CONFIDENCE_SAMPLED: &str = "sampled";
    const LAND_COMBAT_EVENT_STARTED: &str = "land_combat_started";
    const LAND_COMBAT_EVENT_UPDATED: &str = "land_combat_updated";
    const LAND_COMBAT_EVENT_ENDED: &str = "land_combat_ended";
    const ARMY_ORDER_EVENT_ADDED: &str = "army_order_added";
    const ARMY_ORDER_EVENT_UPDATED: &str = "army_order_updated";
    const ARMY_ORDER_EVENT_REMOVED: &str = "army_order_removed";
    const NAVAL_COMBAT_EVENT_STARTED: &str = "naval_combat_started";
    const NAVAL_COMBAT_EVENT_UPDATED: &str = "naval_combat_updated";
    const NAVAL_COMBAT_EVENT_ENDED: &str = "naval_combat_ended";
    const AIR_WING_EVENT_OBSERVED: &str = "air_wing_observed";
    const AIR_WING_EVENT_UPDATED: &str = "air_wing_updated";
    const AIR_WING_EVENT_LOST: &str = "air_wing_lost";
    const DIVISION_EVENT_MOVEMENT_STARTED: &str = "division_movement_started";
    const DIVISION_EVENT_MOVEMENT_STOPPED: &str = "division_movement_stopped";
    const DIVISION_EVENT_ORG_STRENGTH_DELTA: &str = "division_org_strength_delta";
    const DIVISION_EVENT_COMBAT_REF_CHANGED: &str = "division_combat_ref_changed";
    const RAW_SHORT_STRING_MAX_LENGTH: usize = 128;
    const RAW_STRING_MAX_LENGTH: usize = 512;
    const RAW_JSON_OBJECT_MAX_LENGTH: usize = 4096;

    struct ClockSample {
        date: String,
        hour_index: u64,
        raw_date: i32,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DivisionMovementSample {
        div_id: u32,
        moving: bool,
        raw_movement_progress: Option<u32>,
        progress_bucket: Option<u32>,
        location: u32,
        move_target_province: Option<u32>,
        movement_source_province: Option<u32>,
        raw_organisation: u32,
        organisation_bucket: u32,
        raw_strength: u32,
        strength_bucket: u32,
        combat_ref_count: u32,
        in_combat: bool,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct LandCombatSample {
        combat_id: u32,
        generation: u64,
        body_addr: usize,
        location: Option<u32>,
        day: Option<u32>,
        duration: Option<u32>,
        attacker_divisions: Option<Vec<u32>>,
        defender_divisions: Option<Vec<u32>>,
        attacker_status: &'static str,
        defender_status: &'static str,
    }

    #[derive(Default)]
    struct LandCombatTracker {
        previous: BTreeMap<u32, LandCombatSample>,
        next_generation: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ArmyOrderSample {
        army_id: u32,
        generation: u64,
        order_addr: usize,
        slot_index: u32,
        order_type: i32,
        target_states: Option<Vec<u32>>,
        target_state_count: Option<u32>,
        target_status: &'static str,
    }

    #[derive(Default)]
    struct ArmyOrderTracker {
        previous: BTreeMap<usize, ArmyOrderSample>,
        next_generation: u64,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct NavalCombatSample {
        combat_id: u32,
        generation: u64,
        body_addr: usize,
        location: Option<u32>,
        attacker_tags: Option<Vec<String>>,
        defender_tags: Option<Vec<String>>,
        attacker_ship_count: Option<u32>,
        defender_ship_count: Option<u32>,
        attacker_commander: Option<String>,
        defender_commander: Option<String>,
        attacker_positioning: Option<f64>,
        defender_positioning: Option<f64>,
        attacker_status: &'static str,
        defender_status: &'static str,
    }

    #[derive(Default)]
    struct NavalCombatTracker {
        previous: BTreeMap<u32, NavalCombatSample>,
        next_generation: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct NavalCombatMemberSample {
        member_key: String,
        combat_key: String,
        combat_id: u32,
        generation: u64,
        side: &'static str,
        group_index: u32,
        member_index: u32,
        unit_type_tag: u32,
        ship_instance_id: Option<u32>,
        is_convoy: bool,
        owner: Option<String>,
        name: Option<String>,
        type_display: Option<String>,
        archetype: Option<String>,
        designed_max_strength_raw: Option<u32>,
        battle_slot_index_raw: u32,
        state_bitmask: Option<u32>,
        engagement_weight_raw: Option<u32>,
    }

    #[derive(Default)]
    struct NavalCombatMemberTracker {
        previous: BTreeMap<String, NavalCombatMemberSample>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct AirWingSample {
        owner: String,
        wing_id: u32,
        generation: u64,
        body_addr: usize,
        count: u32,
        mission_mask: u32,
        mission_region: Option<u32>,
        effective_planes: u32,
    }

    #[derive(Default)]
    struct AirWingTracker {
        previous: BTreeMap<(String, u32), AirWingSample>,
        next_generation: u64,
    }

    struct RawDomainDeltaFrame<'a> {
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        date: &'a str,
        hour_index: u64,
        sample_ms: u32,
        duration_ms: Option<u32>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &'a str,
        domain: &'a str,
        entity_key: &'a str,
        confidence: &'a str,
        op: &'a str,
        after_json: &'a str,
        changed_fields: &'a [&'a str],
    }

    struct RawEventFrame<'a> {
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        date: &'a str,
        hour_index: u64,
        sample_ms: u32,
        duration_ms: Option<u32>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &'a str,
        domain: &'a str,
        entity_key: &'a str,
        confidence: &'a str,
        event_type: &'a str,
        subject: &'a str,
        detail_json: &'a str,
    }

    fn build_clock_raw_domain_delta_line(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        session_sequence: Option<u64>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Option<String> {
        let sample = clock_sample_from_state(state)?;
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        clock_raw_domain_delta_frame_line(
            pid,
            sequence,
            session_sequence,
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            &sample,
        )
    }

    fn clock_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        sample: &ClockSample,
    ) -> Option<String> {
        let after_json = format!(
            "{{\"date\":{},\"hour_index\":{},\"raw_date\":{}}}",
            json_str(&sample.date),
            sample.hour_index,
            sample.raw_date
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &sample.date,
            hour_index: sample.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_CLOCK_DOMAIN,
            entity_key: RAW_CLOCK_ENTITY_KEY,
            confidence: RAW_CONFIDENCE_SAMPLED,
            op: "sample",
            after_json: &after_json,
            changed_fields: &["date", "hour_index", "raw_date"],
        })
    }

    fn clock_sample_from_state(state: Option<&CollectorState>) -> Option<ClockSample> {
        let raw_date = current_game_date_raw(state)?;
        let date = decode_datehour(raw_date)?;
        let hour_index = raw_date.checked_sub(DATE_EPOCH_CONST)? as u64;
        Some(ClockSample {
            date,
            hour_index,
            raw_date,
        })
    }

    fn build_division_raw_domain_delta_lines(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        previous: &mut BTreeMap<u32, DivisionMovementSample>,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Vec<String> {
        if !(control.division_body_bridge_valid
            && division_body_bridge_well_formed(&control.division_body_bridge))
        {
            return Vec::new();
        }
        let Some(state) = state else {
            return Vec::new();
        };
        if control.division_body_bridge.is_empty() {
            return Vec::new();
        }
        let Some(clock) = clock_sample_from_state(Some(state)) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        let mut session_sequence = first_session_sequence;
        for entry in &control.division_body_bridge {
            let Some(sample) = sample_division_movement(state.base, entry) else {
                continue;
            };
            let old = previous.get(&sample.div_id).cloned();
            let Some(changed_fields) = track_division_movement_sample(previous, &sample) else {
                continue;
            };
            let tick_ms = unsafe { GetTickCount64() };
            let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            if let Some(line) = division_raw_domain_delta_frame_line(
                pid,
                sequence,
                Some(session_sequence),
                tick_ms,
                control,
                elapsed_wall_ms,
                capture_boundary,
                &clock,
                &sample,
                &changed_fields,
            ) {
                lines.push(line);
                session_sequence = session_sequence.saturating_add(1);
            }
            push_division_event_frames(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                &clock,
                old.as_ref(),
                &sample,
                &changed_fields,
            );
        }
        lines
    }

    fn sample_division_movement(
        module_base: usize,
        entry: &DivisionBodyBridgeEntry,
    ) -> Option<DivisionMovementSample> {
        let body = entry.body_ptr;
        if read_u64(body)? as usize != module_base.checked_add(DIVISION_VTABLE_RVA)? {
            return None;
        }
        if read_u32(body.checked_add(DIVISION_OFF_ID_TYPE)?)? != DIVISION_TYPE {
            return None;
        }
        if read_u32(body.checked_add(DIVISION_OFF_ID_ID)?)? != entry.div_id {
            return None;
        }
        let location = read_u32(body.checked_add(DIVISION_OFF_LOCATION)?)?;
        if !valid_division_province_id(location) {
            return None;
        }
        let combat_ref_count = read_u32(body.checked_add(DIVISION_OFF_COMBAT_REF_COUNT)?)?;
        if combat_ref_count > MAX_DIVISION_COMBAT_REF_COUNT {
            return None;
        }
        let raw_strength = read_division_fixed_raw(
            body.checked_add(DIVISION_OFF_STRENGTH)?,
            MAX_DIVISION_STRENGTH_RAW,
        )?;
        let raw_organisation = read_division_fixed_raw(
            body.checked_add(DIVISION_OFF_ORGANISATION)?,
            MAX_DIVISION_ORGANISATION_RAW,
        )?;
        let raw_progress = read_u32(body.checked_add(DIVISION_OFF_MOVE_PROGRESS)?)?;
        let moving = raw_progress != 0;
        let movement_source_province = if moving {
            read_division_movement_source_province(module_base, body)
        } else {
            None
        };
        Some(DivisionMovementSample {
            div_id: entry.div_id,
            moving,
            raw_movement_progress: moving.then_some(raw_progress),
            progress_bucket: moving.then_some(division_delta_bucket(raw_progress)),
            location,
            move_target_province: moving.then_some(location),
            movement_source_province,
            raw_organisation,
            organisation_bucket: division_delta_bucket(raw_organisation),
            raw_strength,
            strength_bucket: division_delta_bucket(raw_strength),
            combat_ref_count,
            in_combat: combat_ref_count > 0,
        })
    }

    fn read_division_movement_source_province(module_base: usize, body: usize) -> Option<u32> {
        let move_order_ptr = read_u64(body.checked_add(DIVISION_OFF_MOVE_ORDER)?)? as usize;
        if !is_map_control_heap_pointer(move_order_ptr) {
            return None;
        }
        if read_u64(move_order_ptr)? as usize
            != module_base.checked_add(DIVISION_MOVE_ORDER_VTABLE_RVA)?
        {
            return None;
        }
        read_u32(move_order_ptr.checked_add(DIVISION_MOVE_ORDER_OFF_SOURCE)?)
            .filter(|province| valid_division_province_id(*province))
    }

    fn valid_division_province_id(value: u32) -> bool {
        value > 0 && value <= MAX_DIVISION_PROVINCE_ID
    }

    fn read_division_fixed_raw(addr: usize, max_raw: u32) -> Option<u32> {
        read_u32(addr).filter(|value| *value <= max_raw)
    }

    fn division_delta_bucket(raw: u32) -> u32 {
        raw / DIVISION_DELTA_BUCKET_RAW
    }

    fn division_fixed_value(raw: u32) -> f64 {
        raw as f64 / FP_SCALE
    }

    fn track_division_movement_sample(
        previous: &mut BTreeMap<u32, DivisionMovementSample>,
        sample: &DivisionMovementSample,
    ) -> Option<Vec<&'static str>> {
        let changed_fields = match previous.get(&sample.div_id) {
            None if sample.moving => Some(division_movement_all_fields()),
            None => None,
            Some(old) => division_movement_changed_fields(old, sample),
        };
        previous.insert(sample.div_id, sample.clone());
        changed_fields
    }

    fn division_movement_all_fields() -> Vec<&'static str> {
        vec![
            "div_uid",
            "div_id",
            "div_type",
            "moving",
            "movement_progress",
            "progress_bucket",
            "location",
            "move_target_province",
            "movement_source_province",
            "organisation",
            "strength",
            "combat_ref_count",
            "in_combat",
        ]
    }

    fn division_movement_changed_fields(
        old: &DivisionMovementSample,
        new: &DivisionMovementSample,
    ) -> Option<Vec<&'static str>> {
        let mut changed = Vec::new();
        if old.moving != new.moving {
            changed.push("moving");
        }
        if old.progress_bucket != new.progress_bucket {
            changed.push("movement_progress");
            changed.push("progress_bucket");
        }
        if old.location != new.location {
            changed.push("location");
        }
        if old.move_target_province != new.move_target_province {
            changed.push("move_target_province");
        }
        if old.movement_source_province != new.movement_source_province {
            changed.push("movement_source_province");
        }
        if old.organisation_bucket != new.organisation_bucket {
            changed.push("organisation");
        }
        if old.strength_bucket != new.strength_bucket {
            changed.push("strength");
        }
        if old.combat_ref_count != new.combat_ref_count {
            changed.push("combat_ref_count");
        }
        if old.in_combat != new.in_combat {
            changed.push("in_combat");
        }
        (!changed.is_empty()).then_some(changed)
    }

    fn division_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &DivisionMovementSample,
        changed_fields: &[&str],
    ) -> Option<String> {
        let div_uid = format!("{}:{}", sample.div_id, DIVISION_TYPE);
        let entity_key = format!("division:{div_uid}");
        let after_json = format!("{{{}}}", division_sample_fields_json(sample));
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_DIVISIONS_DOMAIN,
            entity_key: &entity_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            op: "sample",
            after_json: &after_json,
            changed_fields,
        })
    }

    fn division_sample_fields_json(sample: &DivisionMovementSample) -> String {
        let div_uid = format!("{}:{}", sample.div_id, DIVISION_TYPE);
        let movement_progress = sample.raw_movement_progress.map(division_fixed_value);
        let organisation = division_fixed_value(sample.raw_organisation);
        let strength = division_fixed_value(sample.raw_strength);
        format!(
            "\"div_uid\":{},\"div_id\":{},\"div_type\":{},\
             \"moving\":{},\"movement_progress\":{},\"progress_bucket\":{},\
             \"location\":{},\"move_target_province\":{},\
             \"movement_source_province\":{},\"organisation\":{},\
             \"strength\":{},\"combat_ref_count\":{},\"in_combat\":{}",
            json_str(&div_uid),
            sample.div_id,
            DIVISION_TYPE,
            sample.moving,
            json_opt_f64(movement_progress),
            json_opt_u32(sample.progress_bucket),
            sample.location,
            json_opt_u32(sample.move_target_province),
            json_opt_u32(sample.movement_source_province),
            json_opt_f64(Some(organisation)),
            json_opt_f64(Some(strength)),
            sample.combat_ref_count,
            sample.in_combat
        )
    }

    fn push_division_event_frames(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        old: Option<&DivisionMovementSample>,
        sample: &DivisionMovementSample,
        changed_fields: &[&str],
    ) {
        let old_moving = old.map(|old| old.moving).unwrap_or(false);
        if !old_moving && sample.moving {
            push_division_event_frame(
                lines,
                session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                sample,
                DIVISION_EVENT_MOVEMENT_STARTED,
                changed_fields,
            );
        } else if old_moving && !sample.moving {
            push_division_event_frame(
                lines,
                session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                sample,
                DIVISION_EVENT_MOVEMENT_STOPPED,
                changed_fields,
            );
        }

        if old.is_none() {
            return;
        }

        let org_strength_fields = changed_fields
            .iter()
            .copied()
            .filter(|field| *field == "organisation" || *field == "strength")
            .collect::<Vec<_>>();
        if !org_strength_fields.is_empty() {
            push_division_event_frame(
                lines,
                session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                sample,
                DIVISION_EVENT_ORG_STRENGTH_DELTA,
                &org_strength_fields,
            );
        }

        let combat_ref_fields = changed_fields
            .iter()
            .copied()
            .filter(|field| *field == "combat_ref_count" || *field == "in_combat")
            .collect::<Vec<_>>();
        if !combat_ref_fields.is_empty() {
            push_division_event_frame(
                lines,
                session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                sample,
                DIVISION_EVENT_COMBAT_REF_CHANGED,
                &combat_ref_fields,
            );
        }
    }

    fn push_division_event_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &DivisionMovementSample,
        event_type: &str,
        changed_fields: &[&str],
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = division_raw_event_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            event_type,
            changed_fields,
        ) {
            lines.push(line);
            *session_sequence = (*session_sequence).saturating_add(1);
        }
    }

    fn division_raw_event_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &DivisionMovementSample,
        event_type: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        let div_uid = format!("{}:{}", sample.div_id, DIVISION_TYPE);
        let entity_key = format!("division:{div_uid}");
        let changed_fields_json = changed_fields
            .iter()
            .map(|field| json_str(field))
            .collect::<Vec<_>>()
            .join(",");
        let detail_json = format!(
            "{{{},\"changed_fields\":[{}]}}",
            division_sample_fields_json(sample),
            changed_fields_json
        );
        raw_event_frame_line(RawEventFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_DIVISIONS_DOMAIN,
            entity_key: &entity_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            event_type,
            subject: "division",
            detail_json: &detail_json,
        })
    }

    fn build_land_combat_raw_domain_delta_lines(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        tracker: &mut LandCombatTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Vec<String> {
        let Some(state) = state else {
            return Vec::new();
        };
        let Some(clock) = clock_sample_from_state(Some(state)) else {
            return Vec::new();
        };
        let current_samples = match sample_land_combats(state) {
            Ok(samples) => samples,
            Err(gap_kind) => {
                let tick_ms = unsafe { GetTickCount64() };
                let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                return land_combat_gap_raw_domain_delta_frame_line(
                    pid,
                    sequence,
                    Some(first_session_sequence),
                    tick_ms,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    &clock,
                    gap_kind,
                )
                .into_iter()
                .collect();
            }
        };
        if current_samples.is_empty() && tracker.previous.is_empty() {
            return Vec::new();
        }

        build_land_combat_raw_lines_from_samples(
            pid,
            control,
            tracker,
            first_session_sequence,
            elapsed_wall_ms,
            capture_boundary,
            &clock,
            current_samples,
        )
    }

    fn build_land_combat_raw_lines_from_samples(
        pid: Dword,
        control: &CollectorControl,
        tracker: &mut LandCombatTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        current_samples: Vec<LandCombatSample>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let mut session_sequence = first_session_sequence;
        let mut current = BTreeMap::new();
        for mut sample in current_samples {
            let old = tracker.previous.get(&sample.combat_id).cloned();
            let continues = land_combat_continues(old.as_ref(), &sample);
            let generation = if continues {
                old.as_ref().map(|old| old.generation).unwrap_or_else(|| {
                    let generation = tracker.next_generation;
                    tracker.next_generation = tracker.next_generation.saturating_add(1);
                    generation
                })
            } else {
                let generation = tracker.next_generation;
                tracker.next_generation = tracker.next_generation.saturating_add(1);
                generation
            };
            if let Some(old_sample) = old.as_ref().filter(|_| !continues) {
                let end_reason = land_combat_split_end_reason(old_sample, &sample);
                push_land_combat_delta_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    old_sample,
                    "delete",
                    &["combat_key"],
                );
                push_land_combat_event_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    old_sample,
                    LAND_COMBAT_EVENT_ENDED,
                    &["combat_key"],
                    Some(end_reason),
                );
            }
            sample.generation = generation;
            let previous_same_generation = old
                .as_ref()
                .filter(|old| old.generation == generation && continues);
            let changed_fields = previous_same_generation
                .and_then(|old| land_combat_changed_fields(old, &sample))
                .unwrap_or_else(land_combat_all_fields);
            if !changed_fields.is_empty() {
                push_land_combat_delta_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    &sample,
                    "sample",
                    &changed_fields,
                );
                let event_type = if previous_same_generation.is_some() {
                    LAND_COMBAT_EVENT_UPDATED
                } else {
                    LAND_COMBAT_EVENT_STARTED
                };
                push_land_combat_event_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    &sample,
                    event_type,
                    &changed_fields,
                    None,
                );
            }
            current.insert(sample.combat_id, sample);
        }

        for old in tracker.previous.values() {
            if current.contains_key(&old.combat_id) {
                continue;
            }
            push_land_combat_delta_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old,
                "delete",
                &["combat_key"],
            );
            push_land_combat_event_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old,
                LAND_COMBAT_EVENT_ENDED,
                &["combat_key"],
                Some("resolved"),
            );
        }

        tracker.previous = current;
        lines
    }

    fn push_land_combat_delta_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &LandCombatSample,
        op: &str,
        changed_fields: &[&str],
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = land_combat_raw_domain_delta_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            op,
            changed_fields,
        ) {
            lines.push(line);
            *session_sequence = session_sequence.saturating_add(1);
        }
    }

    fn push_land_combat_event_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &LandCombatSample,
        event_type: &str,
        changed_fields: &[&str],
        end_reason: Option<&str>,
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = land_combat_raw_event_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            event_type,
            changed_fields,
            end_reason,
        ) {
            lines.push(line);
            *session_sequence = session_sequence.saturating_add(1);
        }
    }

    fn sample_land_combats(state: &CollectorState) -> Result<Vec<LandCombatSample>, &'static str> {
        if !(state.daily_match && state.long_string_match) {
            return Err("state_validation_failed");
        }
        let gamestate = state.date_ptr as usize;
        if !is_map_control_heap_pointer(gamestate) {
            return Err("gamestate_invalid");
        }
        let manager = gamestate.saturating_add(LAND_COMBAT_MANAGER_OFFSET);
        let count = match read_u32(manager.saturating_add(LAND_COMBAT_MANAGER_COUNT)) {
            Some(count) if count <= MAX_LAND_COMBATS => count,
            _ => return Err("manager_count_invalid"),
        };
        if count == 0 {
            return Ok(Vec::new());
        }
        let data = match read_u64(manager.saturating_add(LAND_COMBAT_MANAGER_DATA)) {
            Some(data) if is_map_control_heap_pointer(data as usize) => data as usize,
            _ => return Err("manager_data_invalid"),
        };
        let byte_len = match (count as usize).checked_mul(8) {
            Some(byte_len) => byte_len,
            None => return Err("manager_size_overflow"),
        };
        let raw = match read_bytes(data, byte_len) {
            Some(raw) if raw.len() == byte_len => raw,
            _ => return Err("manager_read_failed"),
        };
        let mut by_id = BTreeMap::new();
        let mut invalid_entries = 0_u32;
        for i in 0..count as usize {
            let Some(raw_ptr) = read_u64_from_slice(&raw, i * 8) else {
                invalid_entries = invalid_entries.saturating_add(1);
                continue;
            };
            if let Some(sample) =
                sample_land_combat_from_manager_entry(state.base, raw_ptr as usize)
            {
                by_id.entry(sample.combat_id).or_insert(sample);
            } else {
                invalid_entries = invalid_entries.saturating_add(1);
            }
        }
        let data2 = read_u64(manager.saturating_add(LAND_COMBAT_MANAGER_DATA));
        let count2 = read_u32(manager.saturating_add(LAND_COMBAT_MANAGER_COUNT));
        if data2 != Some(data as u64) || count2 != Some(count) {
            return Err("manager_changed_during_read");
        }
        land_combat_manager_samples_result(by_id, invalid_entries)
    }

    fn land_combat_manager_samples_result(
        samples: BTreeMap<u32, LandCombatSample>,
        invalid_entries: u32,
    ) -> Result<Vec<LandCombatSample>, &'static str> {
        if samples.is_empty() && invalid_entries > 0 {
            return Err("manager_entry_invalid");
        }
        Ok(samples.into_values().collect())
    }

    fn sample_land_combat_from_manager_entry(
        module_base: usize,
        manager_entry: usize,
    ) -> Option<LandCombatSample> {
        if !is_map_control_heap_pointer(manager_entry) {
            return None;
        }
        let body = manager_entry.checked_add(LAND_COMBAT_MANAGER_BODY_DELTA)?;
        if !is_map_control_heap_pointer(body) {
            return None;
        }
        if read_u64(body)? as usize != module_base.checked_add(LAND_COMBAT_VTABLE_RVA)? {
            return None;
        }
        if read_u32(body.checked_add(LAND_COMBAT_OFF_TYPE)?)? != LAND_COMBAT_TYPE {
            return None;
        }
        let combat_id = read_u32(body.checked_add(LAND_COMBAT_OFF_ID)?)?;
        if combat_id == 0 || combat_id > i32::MAX as u32 {
            return None;
        }
        let front = read_u64(body.checked_add(LAND_COMBAT_OFF_FRONT)?)? as usize;
        let location = Some(read_land_combat_location(module_base, front)?);
        let day = read_u32(body.checked_add(LAND_COMBAT_OFF_DAY)?)
            .filter(|value| *value <= MAX_LAND_COMBAT_AGE_DAYS);
        let duration = read_u32(body.checked_add(LAND_COMBAT_OFF_DURATION)?)
            .filter(|value| *value <= MAX_LAND_COMBAT_AGE_DAYS);
        let (attacker_side, defender_side) =
            land_combat_sides(module_base, body, combat_id, front)?;
        let (attacker_divisions, attacker_status) =
            read_land_combat_side_units(module_base, attacker_side);
        let (defender_divisions, defender_status) =
            read_land_combat_side_units(module_base, defender_side);
        Some(LandCombatSample {
            combat_id,
            generation: 0,
            body_addr: body,
            location,
            day,
            duration,
            attacker_divisions,
            defender_divisions,
            attacker_status,
            defender_status,
        })
    }

    fn read_land_combat_location(module_base: usize, front: usize) -> Option<u32> {
        if !is_map_control_heap_pointer(front) {
            return None;
        }
        if read_u64(front)? as usize != module_base.checked_add(LAND_COMBAT_FRONT_VTABLE_RVA)? {
            return None;
        }
        read_u32(front.checked_add(LAND_COMBAT_FRONT_OFF_PROVINCE)?)
            .filter(|province| *province > 0 && *province <= MAX_LAND_COMBAT_PROVINCE_ID)
    }

    fn land_combat_sides(
        module_base: usize,
        body: usize,
        combat_id: u32,
        front: usize,
    ) -> Option<(usize, usize)> {
        let side_vtable = module_base.checked_add(LAND_COMBAT_SIDE_VTABLE_RVA)?;
        let registry_vtable = module_base.checked_add(LAND_COMBAT_REGISTRY_VTABLE_RVA)?;
        let attacker = read_u64(body.checked_add(LAND_COMBAT_OFF_ATTACKER_SIDE)?)? as usize;
        if !is_map_control_heap_pointer(attacker) || read_u64(attacker)? as usize != side_vtable {
            return None;
        }
        let registry = read_u64(attacker.checked_add(LAND_COMBAT_SIDE_OFF_REGISTRY)?)? as usize;
        if !is_map_control_heap_pointer(registry) || read_u64(registry)? as usize != registry_vtable
        {
            return None;
        }
        let handle = read_u64(registry.checked_add(LAND_COMBAT_REGISTRY_OFF_HANDLE)?)?;
        if (handle & 0xFFFF_FFFF) as u32 != LAND_COMBAT_TYPE
            || ((handle >> 32) & 0xFFFF_FFFF) as u32 != combat_id
        {
            return None;
        }
        if read_u64(registry.checked_add(LAND_COMBAT_REGISTRY_OFF_ATTACKER_SIDE)?)? as usize
            != attacker
        {
            return None;
        }
        if is_map_control_heap_pointer(front)
            && read_u64(registry.checked_add(LAND_COMBAT_REGISTRY_OFF_FRONT)?)? as usize != front
        {
            return None;
        }
        let defender =
            read_u64(registry.checked_add(LAND_COMBAT_REGISTRY_OFF_DEFENDER_SIDE)?)? as usize;
        if !is_map_control_heap_pointer(defender) || read_u64(defender)? as usize != side_vtable {
            return None;
        }
        Some((attacker, defender))
    }

    fn read_land_combat_side_units(
        module_base: usize,
        side: usize,
    ) -> (Option<Vec<u32>>, &'static str) {
        let side_vtable = match module_base.checked_add(LAND_COMBAT_SIDE_VTABLE_RVA) {
            Some(value) => value,
            None => return (None, "validation_failed"),
        };
        if !is_map_control_heap_pointer(side)
            || read_u64(side).map(|v| v as usize) != Some(side_vtable)
        {
            return (None, "validation_failed");
        }
        let data = match read_u64(side.saturating_add(LAND_COMBAT_SIDE_OFF_DATA)) {
            Some(data) => data as usize,
            None => return (None, "validation_failed"),
        };
        let capacity = match read_u32(side.saturating_add(LAND_COMBAT_SIDE_OFF_CAPACITY)) {
            Some(capacity) => capacity,
            None => return (None, "validation_failed"),
        };
        let size = match read_u32(side.saturating_add(LAND_COMBAT_SIDE_OFF_SIZE)) {
            Some(size) => size,
            None => return (None, "validation_failed"),
        };
        if size > capacity || size > MAX_LAND_COMBAT_SIDE_UNITS {
            return (None, "validation_failed");
        }
        if size == 0 {
            return (Some(Vec::new()), "exact");
        }
        if !is_map_control_heap_pointer(data) {
            return (None, "validation_failed");
        }
        let byte_len = match (size as usize).checked_mul(8) {
            Some(byte_len) => byte_len,
            None => return (None, "validation_failed"),
        };
        let raw = match read_bytes(data, byte_len) {
            Some(raw) if raw.len() == byte_len => raw,
            _ => return (None, "validation_failed"),
        };
        let mut ids = Vec::with_capacity(size as usize);
        for i in 0..size as usize {
            let Some(ptr) = read_u64_from_slice(&raw, i * 8) else {
                return (None, "validation_failed");
            };
            let Some(id) = resolve_land_combat_unit_id(module_base, ptr as usize) else {
                return (None, "validation_failed");
            };
            ids.push(id);
        }
        let data2 = read_u64(side.saturating_add(LAND_COMBAT_SIDE_OFF_DATA));
        let capacity2 = read_u32(side.saturating_add(LAND_COMBAT_SIDE_OFF_CAPACITY));
        let size2 = read_u32(side.saturating_add(LAND_COMBAT_SIDE_OFF_SIZE));
        if data2 != Some(data as u64) || capacity2 != Some(capacity) || size2 != Some(size) {
            return (None, "validation_failed");
        }
        ids.sort_unstable();
        ids.dedup();
        (Some(ids), "exact")
    }

    fn resolve_land_combat_unit_id(module_base: usize, ptr: usize) -> Option<u32> {
        resolve_land_combat_unit_id_at(module_base, ptr)
            .or_else(|| resolve_land_combat_unit_id_at(module_base, ptr.checked_add(0x10)?))
    }

    fn resolve_land_combat_unit_id_at(module_base: usize, body: usize) -> Option<u32> {
        if !is_map_control_heap_pointer(body) {
            return None;
        }
        if read_u64(body)? as usize != module_base.checked_add(DIVISION_VTABLE_RVA)? {
            return None;
        }
        let unit_type = read_u32(body.checked_add(DIVISION_OFF_ID_TYPE)?)?;
        if unit_type != DIVISION_TYPE && unit_type != LAND_COMBAT_SIDE_UNIT_ALT_TYPE {
            return None;
        }
        let id = read_u32(body.checked_add(DIVISION_OFF_ID_ID)?)?;
        (id > 0 && id <= MAX_DIVISION_BODY_BRIDGE_DIV_ID).then_some(id)
    }

    fn land_combat_continues(old: Option<&LandCombatSample>, new: &LandCombatSample) -> bool {
        let Some(old) = old else {
            return false;
        };
        if old.body_addr != new.body_addr {
            return false;
        }
        match (old.day, new.day) {
            (Some(old_day), Some(new_day)) => new_day >= old_day,
            _ => true,
        }
    }

    fn land_combat_split_end_reason(
        old: &LandCombatSample,
        new: &LandCombatSample,
    ) -> &'static str {
        if old.body_addr != new.body_addr {
            "body_changed"
        } else {
            "id_reuse"
        }
    }

    fn land_combat_all_fields() -> Vec<&'static str> {
        vec![
            "combat_key",
            "combat_id",
            "generation",
            "location",
            "day",
            "duration",
            "attacker_divisions",
            "defender_divisions",
            "attacker_status",
            "defender_status",
        ]
    }

    fn land_combat_changed_fields(
        old: &LandCombatSample,
        new: &LandCombatSample,
    ) -> Option<Vec<&'static str>> {
        let mut changed = Vec::new();
        if old.location != new.location {
            changed.push("location");
        }
        if old.day != new.day {
            changed.push("day");
        }
        if old.duration != new.duration {
            changed.push("duration");
        }
        if old.attacker_divisions != new.attacker_divisions {
            changed.push("attacker_divisions");
        }
        if old.defender_divisions != new.defender_divisions {
            changed.push("defender_divisions");
        }
        if old.attacker_status != new.attacker_status {
            changed.push("attacker_status");
        }
        if old.defender_status != new.defender_status {
            changed.push("defender_status");
        }
        (!changed.is_empty()).then_some(changed)
    }

    fn land_combat_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &LandCombatSample,
        op: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        let combat_key = format!("land_combat:{}:{}", sample.combat_id, sample.generation);
        let after_json = if op == "delete" {
            "null".to_string()
        } else {
            land_combat_row_json(sample, &combat_key)
        };
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_LAND_COMBATS_DOMAIN,
            entity_key: &combat_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            op,
            after_json: &after_json,
            changed_fields,
        })
    }

    fn land_combat_gap_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        gap_kind: &str,
    ) -> Option<String> {
        if !valid_raw_short_string(gap_kind) || looks_like_hex_pointer(gap_kind) {
            return None;
        }
        let gap_key = format!("land_combats:gap:{gap_kind}");
        let after_json = format!(
            "{{\"gap_key\":{},\"gap_kind\":{},\"status\":\"validation_failed\"}}",
            json_str(&gap_key),
            json_str(gap_kind)
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_LAND_COMBATS_DOMAIN,
            entity_key: &gap_key,
            confidence: "gap",
            op: "sample",
            after_json: &after_json,
            changed_fields: &["gap_key", "gap_kind", "status"],
        })
    }

    fn land_combat_raw_event_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &LandCombatSample,
        event_type: &str,
        changed_fields: &[&str],
        end_reason: Option<&str>,
    ) -> Option<String> {
        if !matches!(
            event_type,
            LAND_COMBAT_EVENT_STARTED | LAND_COMBAT_EVENT_UPDATED | LAND_COMBAT_EVENT_ENDED
        ) {
            return None;
        }
        let combat_key = format!("land_combat:{}:{}", sample.combat_id, sample.generation);
        let detail_json =
            land_combat_event_detail_json(sample, &combat_key, changed_fields, end_reason)?;
        raw_event_frame_line(RawEventFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_LAND_COMBATS_DOMAIN,
            entity_key: &combat_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            event_type,
            subject: "land_combat",
            detail_json: &detail_json,
        })
    }

    fn land_combat_row_json(sample: &LandCombatSample, combat_key: &str) -> String {
        format!(
            "{{\"combat_key\":{},\"combat_id\":{},\"generation\":{},\
             \"location\":{},\"day\":{},\"duration\":{},\
             \"attacker_divisions\":{},\"defender_divisions\":{},\
             \"attacker_status\":{},\"defender_status\":{}}}",
            json_str(combat_key),
            sample.combat_id,
            sample.generation,
            json_opt_u32(sample.location),
            json_opt_u32(sample.day),
            json_opt_u32(sample.duration),
            json_opt_u32_vec(sample.attacker_divisions.as_ref()),
            json_opt_u32_vec(sample.defender_divisions.as_ref()),
            json_str(sample.attacker_status),
            json_str(sample.defender_status)
        )
    }

    fn land_combat_event_detail_json(
        sample: &LandCombatSample,
        combat_key: &str,
        changed_fields: &[&str],
        end_reason: Option<&str>,
    ) -> Option<String> {
        if changed_fields.len() > 256
            || changed_fields
                .iter()
                .any(|field| !valid_raw_short_string(field))
        {
            return None;
        }
        if end_reason
            .map(|reason| !valid_raw_short_string(reason) || looks_like_hex_pointer(reason))
            .unwrap_or(false)
        {
            return None;
        }
        let changed_fields = changed_fields
            .iter()
            .map(|field| json_str(field))
            .collect::<Vec<_>>()
            .join(",");
        let end_reason = end_reason
            .map(|reason| format!(",\"end_reason\":{}", json_str(reason)))
            .unwrap_or_default();
        Some(format!(
            "{{\"combat_key\":{},\"combat_id\":{},\"generation\":{},\
             \"location\":{},\"day\":{},\"duration\":{},\
             \"attacker_divisions\":{},\"defender_divisions\":{},\
             \"attacker_status\":{},\"defender_status\":{},\
             \"changed_fields\":[{}]{} }}",
            json_str(combat_key),
            sample.combat_id,
            sample.generation,
            json_opt_u32(sample.location),
            json_opt_u32(sample.day),
            json_opt_u32(sample.duration),
            json_opt_u32_vec(sample.attacker_divisions.as_ref()),
            json_opt_u32_vec(sample.defender_divisions.as_ref()),
            json_str(sample.attacker_status),
            json_str(sample.defender_status),
            changed_fields,
            end_reason
        ))
    }

    fn build_army_order_raw_domain_delta_lines(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        tracker: &mut ArmyOrderTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Vec<String> {
        let Some(state) = state else {
            return Vec::new();
        };
        let Some(clock) = clock_sample_from_state(Some(state)) else {
            return Vec::new();
        };
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let current_samples = match sample_army_orders(state, control, tracker) {
            Ok(samples) => samples,
            Err(gap_kind) => {
                return army_order_gap_raw_domain_delta_frame_line(
                    pid,
                    sequence,
                    Some(first_session_sequence),
                    tick_ms,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    &clock,
                    gap_kind,
                )
                .into_iter()
                .collect();
            }
        };
        build_army_order_raw_lines_from_samples(
            pid,
            control,
            tracker,
            current_samples,
            first_session_sequence,
            sequence,
            tick_ms,
            elapsed_wall_ms,
            capture_boundary,
            &clock,
        )
    }

    fn build_army_order_raw_lines_from_samples(
        pid: Dword,
        control: &CollectorControl,
        tracker: &mut ArmyOrderTracker,
        current_samples: Vec<ArmyOrderSample>,
        first_session_sequence: u64,
        first_sequence: u64,
        tick_ms: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
    ) -> Vec<String> {
        let previous = std::mem::take(&mut tracker.previous);
        let mut current: BTreeMap<usize, ArmyOrderSample> = BTreeMap::new();
        for mut sample in current_samples {
            if let Some(old) = previous.get(&sample.order_addr) {
                sample.generation = old.generation;
            } else {
                sample.generation = tracker.next_generation;
                tracker.next_generation = tracker.next_generation.saturating_add(1);
            }
            current.insert(sample.order_addr, sample);
        }

        let mut lines = Vec::new();
        let mut sequence = first_sequence;
        let mut session_sequence = first_session_sequence;
        for (order_addr, sample) in &current {
            match previous.get(order_addr) {
                Some(old) => {
                    if let Some(changed_fields) = army_order_changed_fields(old, sample) {
                        push_army_order_delta_frame(
                            &mut lines,
                            pid,
                            sequence,
                            session_sequence,
                            tick_ms,
                            control,
                            elapsed_wall_ms,
                            capture_boundary,
                            clock,
                            sample,
                            "sample",
                            &changed_fields,
                        );
                        sequence = sequence.saturating_add(1);
                        session_sequence = session_sequence.saturating_add(1);
                        push_army_order_event_frame(
                            &mut lines,
                            pid,
                            sequence,
                            session_sequence,
                            tick_ms,
                            control,
                            elapsed_wall_ms,
                            capture_boundary,
                            clock,
                            sample,
                            ARMY_ORDER_EVENT_UPDATED,
                            &changed_fields,
                        );
                        sequence = sequence.saturating_add(1);
                        session_sequence = session_sequence.saturating_add(1);
                    }
                }
                None => {
                    let changed_fields = army_order_all_fields();
                    push_army_order_delta_frame(
                        &mut lines,
                        pid,
                        sequence,
                        session_sequence,
                        tick_ms,
                        control,
                        elapsed_wall_ms,
                        capture_boundary,
                        clock,
                        sample,
                        "sample",
                        &changed_fields,
                    );
                    sequence = sequence.saturating_add(1);
                    session_sequence = session_sequence.saturating_add(1);
                    push_army_order_event_frame(
                        &mut lines,
                        pid,
                        sequence,
                        session_sequence,
                        tick_ms,
                        control,
                        elapsed_wall_ms,
                        capture_boundary,
                        clock,
                        sample,
                        ARMY_ORDER_EVENT_ADDED,
                        &changed_fields,
                    );
                    sequence = sequence.saturating_add(1);
                    session_sequence = session_sequence.saturating_add(1);
                }
            }
        }
        for (order_addr, old_sample) in &previous {
            if current.contains_key(order_addr) {
                continue;
            }
            let changed_fields = ["order_key"];
            push_army_order_delta_frame(
                &mut lines,
                pid,
                sequence,
                session_sequence,
                tick_ms,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old_sample,
                "delete",
                &changed_fields,
            );
            sequence = sequence.saturating_add(1);
            session_sequence = session_sequence.saturating_add(1);
            push_army_order_event_frame(
                &mut lines,
                pid,
                sequence,
                session_sequence,
                tick_ms,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old_sample,
                ARMY_ORDER_EVENT_REMOVED,
                &changed_fields,
            );
            sequence = sequence.saturating_add(1);
            session_sequence = session_sequence.saturating_add(1);
        }
        tracker.previous = current;
        lines
    }

    fn push_army_order_delta_frame(
        lines: &mut Vec<String>,
        pid: Dword,
        sequence: u64,
        session_sequence: u64,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &ArmyOrderSample,
        op: &str,
        changed_fields: &[&str],
    ) {
        if let Some(line) = army_order_raw_domain_delta_frame_line(
            pid,
            sequence,
            Some(session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            op,
            changed_fields,
        ) {
            lines.push(line);
        }
    }

    fn push_army_order_event_frame(
        lines: &mut Vec<String>,
        pid: Dword,
        sequence: u64,
        session_sequence: u64,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &ArmyOrderSample,
        event_type: &str,
        changed_fields: &[&str],
    ) {
        if let Some(line) = army_order_raw_event_frame_line(
            pid,
            sequence,
            Some(session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            event_type,
            changed_fields,
        ) {
            lines.push(line);
        }
    }

    fn sample_army_orders(
        state: &CollectorState,
        control: &CollectorControl,
        tracker: &ArmyOrderTracker,
    ) -> Result<Vec<ArmyOrderSample>, &'static str> {
        let _ = tracker;
        if !(control.army_body_bridge_valid
            && army_body_bridge_well_formed(&control.army_body_bridge))
        {
            return Err("bridge_invalid");
        }
        if control.army_body_bridge.is_empty() {
            return Ok(Vec::new());
        }
        let module_base = state.base;
        let mut out = Vec::new();
        let mut valid_armies = 0u32;
        let mut invalid_entries = 0u32;
        let mut seen_orders: BTreeMap<usize, ()> = BTreeMap::new();
        for entry in &control.army_body_bridge {
            match sample_army_orders_from_bridge_entry(module_base, entry) {
                Some(samples) => {
                    valid_armies = valid_armies.saturating_add(1);
                    for sample in samples {
                        if seen_orders.insert(sample.order_addr, ()).is_none() {
                            out.push(sample);
                        }
                    }
                }
                None => invalid_entries = invalid_entries.saturating_add(1),
            }
        }
        if valid_armies == 0 && invalid_entries > 0 {
            return Err("bridge_entry_invalid");
        }
        Ok(out)
    }

    fn sample_army_orders_from_bridge_entry(
        module_base: usize,
        entry: &ArmyBodyBridgeEntry,
    ) -> Option<Vec<ArmyOrderSample>> {
        let body = entry.body_ptr;
        let army_vtable = module_base.checked_add(ARMY_VTABLE_RVA)?;
        if !is_map_control_heap_pointer(body) || read_u64(body)? as usize != army_vtable {
            return None;
        }
        let handle = read_u64(body.checked_add(ARMY_OFF_HANDLE)?)?;
        if (handle & 0xFFFF_FFFF) as u32 != ARMY_TYPE {
            return None;
        }
        if ((handle >> 32) & 0xFFFF_FFFF) as u32 != entry.army_id {
            return None;
        }
        let orders_begin = read_u64(body.checked_add(ARMY_OFF_ORDERS_VEC)?)? as usize;
        if !is_map_control_heap_pointer(orders_begin) {
            return Some(Vec::new());
        }
        let order_vtable = module_base.checked_add(ORDER_VTABLE_RVA)?;
        let mut out = Vec::new();
        for slot_index in 0..MAX_ARMY_ORDERS_PER_ARMY {
            let order_ptr_addr = orders_begin.checked_add(slot_index as usize * 8)?;
            let order_ptr = read_u64(order_ptr_addr)? as usize;
            if !is_map_control_heap_pointer(order_ptr) {
                break;
            }
            let Some(sample) =
                read_army_order_sample(order_vtable, entry.army_id, body, order_ptr, slot_index)
            else {
                continue;
            };
            out.push(sample);
        }
        Some(out)
    }

    fn read_army_order_sample(
        order_vtable: usize,
        army_id: u32,
        army_body: usize,
        order_ptr: usize,
        slot_index: u32,
    ) -> Option<ArmyOrderSample> {
        if !is_map_control_heap_pointer(order_ptr) || read_u64(order_ptr)? as usize != order_vtable
        {
            return None;
        }
        if read_u64(order_ptr.checked_add(ORDER_OFF_ARMY_BACKREF)?)? as usize != army_body {
            return None;
        }
        let order_type = read_i32(order_ptr.checked_add(ORDER_OFF_TYPE)?)?;
        let (target_states, target_state_count, target_status) =
            read_army_order_target_states(order_ptr, order_type);
        Some(ArmyOrderSample {
            army_id,
            generation: 0,
            order_addr: order_ptr,
            slot_index,
            order_type,
            target_states,
            target_state_count,
            target_status,
        })
    }

    fn read_army_order_target_states(
        order_ptr: usize,
        order_type: i32,
    ) -> (Option<Vec<u32>>, Option<u32>, &'static str) {
        if order_type != ORDER_TYPE5_OFFENSIVE {
            return (None, None, "unsupported");
        }
        let Some(begin_addr) = order_ptr.checked_add(ORDER_OFF_TYPE5_STATES_BEGIN) else {
            return (None, None, "validation_failed");
        };
        let Some(size_addr) = order_ptr.checked_add(ORDER_OFF_TYPE5_STATES_SIZE) else {
            return (None, None, "validation_failed");
        };
        let Some(begin) = read_u64(begin_addr).map(|v| v as usize) else {
            return (None, None, "validation_failed");
        };
        let Some(size) = read_u32(size_addr) else {
            return (None, None, "validation_failed");
        };
        if size == 0 {
            return (Some(Vec::new()), Some(0), "exact");
        }
        if size > MAX_ARMY_ORDER_TARGET_STATES {
            return (None, None, "validation_failed");
        }
        if !is_map_control_heap_pointer(begin) {
            return (None, Some(size), "validation_failed");
        }
        if size > MAX_ARMY_ORDER_INLINE_TARGET_STATES {
            return (None, Some(size), "overflow");
        }
        let byte_len = (size as usize).checked_mul(4).unwrap_or(usize::MAX);
        if byte_len == usize::MAX {
            return (None, Some(size), "validation_failed");
        }
        let Some(raw) = read_bytes(begin, byte_len) else {
            return (None, Some(size), "validation_failed");
        };
        if raw.len() < byte_len {
            return (None, Some(size), "validation_failed");
        }
        let mut states = Vec::new();
        let mut seen: BTreeMap<u32, ()> = BTreeMap::new();
        for i in 0..size as usize {
            let offset = i * 4;
            let state_id = u32::from_le_bytes([
                raw[offset],
                raw[offset + 1],
                raw[offset + 2],
                raw[offset + 3],
            ]);
            if state_id == 0 || state_id > MAX_ARMY_ORDER_STATE_ID {
                return (None, Some(size), "validation_failed");
            }
            if seen.insert(state_id, ()).is_some() {
                return (None, Some(size), "validation_failed");
            }
            states.push(state_id);
        }
        states.sort_unstable();
        (Some(states), Some(size), "exact")
    }

    fn army_order_all_fields() -> Vec<&'static str> {
        vec![
            "order_key",
            "army_id",
            "generation",
            "slot_index",
            "order_type",
            "target_states",
            "target_state_count",
            "target_status",
        ]
    }

    fn army_order_changed_fields(
        old: &ArmyOrderSample,
        new: &ArmyOrderSample,
    ) -> Option<Vec<&'static str>> {
        let mut changed = Vec::new();
        if old.slot_index != new.slot_index {
            changed.push("slot_index");
        }
        if old.order_type != new.order_type {
            changed.push("order_type");
        }
        if old.target_states != new.target_states {
            changed.push("target_states");
        }
        if old.target_state_count != new.target_state_count {
            changed.push("target_state_count");
        }
        if old.target_status != new.target_status {
            changed.push("target_status");
        }
        (!changed.is_empty()).then_some(changed)
    }

    fn army_order_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &ArmyOrderSample,
        op: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        let order_key = format!("army_order:{}:{}", sample.army_id, sample.generation);
        let after_json = if op == "delete" {
            "null".to_string()
        } else {
            army_order_row_json(sample, &order_key)
        };
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_ARMY_ORDERS_DOMAIN,
            entity_key: &order_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            op,
            after_json: &after_json,
            changed_fields,
        })
    }

    fn army_order_gap_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        gap_kind: &str,
    ) -> Option<String> {
        if !valid_raw_short_string(gap_kind) || looks_like_hex_pointer(gap_kind) {
            return None;
        }
        let gap_key = format!("army_orders:gap:{gap_kind}");
        let after_json = format!(
            "{{\"gap_key\":{},\"gap_kind\":{},\"status\":\"validation_failed\"}}",
            json_str(&gap_key),
            json_str(gap_kind)
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_ARMY_ORDERS_DOMAIN,
            entity_key: &gap_key,
            confidence: "gap",
            op: "sample",
            after_json: &after_json,
            changed_fields: &["gap_key", "gap_kind", "status"],
        })
    }

    fn army_order_raw_event_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &ArmyOrderSample,
        event_type: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        if !matches!(
            event_type,
            ARMY_ORDER_EVENT_ADDED | ARMY_ORDER_EVENT_UPDATED | ARMY_ORDER_EVENT_REMOVED
        ) {
            return None;
        }
        let order_key = format!("army_order:{}:{}", sample.army_id, sample.generation);
        let detail_json = army_order_event_detail_json(sample, &order_key, changed_fields)?;
        raw_event_frame_line(RawEventFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_ARMY_ORDERS_DOMAIN,
            entity_key: &order_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            event_type,
            subject: "army_order",
            detail_json: &detail_json,
        })
    }

    fn army_order_row_json(sample: &ArmyOrderSample, order_key: &str) -> String {
        format!(
            "{{\"order_key\":{},\"army_id\":{},\"generation\":{},\
             \"slot_index\":{},\"order_type\":{},\"target_states\":{},\
             \"target_state_count\":{},\"target_status\":{}}}",
            json_str(order_key),
            sample.army_id,
            sample.generation,
            sample.slot_index,
            sample.order_type,
            json_opt_u32_vec(sample.target_states.as_ref()),
            json_opt_u32(sample.target_state_count),
            json_str(sample.target_status)
        )
    }

    fn army_order_event_detail_json(
        sample: &ArmyOrderSample,
        order_key: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        if changed_fields.len() > 256
            || changed_fields
                .iter()
                .any(|field| !valid_raw_short_string(field))
        {
            return None;
        }
        let changed_fields = changed_fields
            .iter()
            .map(|field| json_str(field))
            .collect::<Vec<_>>()
            .join(",");
        Some(format!(
            "{{\"order_key\":{},\"army_id\":{},\"generation\":{},\
             \"slot_index\":{},\"order_type\":{},\"target_states\":{},\
             \"target_state_count\":{},\"target_status\":{},\
             \"changed_fields\":[{}] }}",
            json_str(order_key),
            sample.army_id,
            sample.generation,
            sample.slot_index,
            sample.order_type,
            json_opt_u32_vec(sample.target_states.as_ref()),
            json_opt_u32(sample.target_state_count),
            json_str(sample.target_status),
            changed_fields
        ))
    }

    fn build_naval_combat_raw_domain_delta_lines(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        tracker: &mut NavalCombatTracker,
        member_tracker: &mut NavalCombatMemberTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Vec<String> {
        let Some(state) = state else {
            return Vec::new();
        };
        let Some(clock) = clock_sample_from_state(Some(state)) else {
            return Vec::new();
        };
        let current_samples = match sample_naval_combats(state, control) {
            Ok(samples) => samples,
            Err(gap_kind) => {
                let tick_ms = unsafe { GetTickCount64() };
                let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let mut lines: Vec<String> = naval_combat_gap_raw_domain_delta_frame_line(
                    pid,
                    sequence,
                    Some(first_session_sequence),
                    tick_ms,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    &clock,
                    gap_kind,
                )
                .into_iter()
                .collect();
                if let Some(line) = naval_combat_member_gap_raw_domain_delta_frame_line(
                    pid,
                    FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                    Some(first_session_sequence.saturating_add(lines.len() as u64)),
                    unsafe { GetTickCount64() },
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    &clock,
                    gap_kind,
                ) {
                    lines.push(line);
                }
                return lines;
            }
        };
        if current_samples.is_empty()
            && tracker.previous.is_empty()
            && member_tracker.previous.is_empty()
        {
            return Vec::new();
        }

        let mut lines = build_naval_combat_raw_lines_from_samples(
            pid,
            control,
            tracker,
            first_session_sequence,
            elapsed_wall_ms,
            capture_boundary,
            &clock,
            current_samples,
        );
        let member_start_sequence = first_session_sequence.saturating_add(lines.len() as u64);
        lines.extend(build_naval_combat_member_raw_lines_from_combat_samples(
            pid,
            state,
            control,
            member_tracker,
            member_start_sequence,
            elapsed_wall_ms,
            capture_boundary,
            &clock,
            tracker.previous.values().cloned().collect(),
        ));
        lines
    }

    fn build_naval_combat_raw_lines_from_samples(
        pid: Dword,
        control: &CollectorControl,
        tracker: &mut NavalCombatTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        current_samples: Vec<NavalCombatSample>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let mut session_sequence = first_session_sequence;
        let mut current = BTreeMap::new();
        for mut sample in current_samples {
            let old = tracker.previous.get(&sample.combat_id).cloned();
            let continues = naval_combat_continues(old.as_ref(), &sample);
            let generation = if continues {
                old.as_ref().map(|old| old.generation).unwrap_or_else(|| {
                    let generation = tracker.next_generation;
                    tracker.next_generation = tracker.next_generation.saturating_add(1);
                    generation
                })
            } else {
                let generation = tracker.next_generation;
                tracker.next_generation = tracker.next_generation.saturating_add(1);
                generation
            };
            if let Some(old_sample) = old.as_ref().filter(|_| !continues) {
                let end_reason = naval_combat_split_end_reason(old_sample, &sample);
                push_naval_combat_delta_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    old_sample,
                    "delete",
                    &["combat_key"],
                );
                push_naval_combat_event_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    old_sample,
                    NAVAL_COMBAT_EVENT_ENDED,
                    &["combat_key"],
                    Some(end_reason),
                );
            }
            sample.generation = generation;
            let previous_same_generation = old
                .as_ref()
                .filter(|old| old.generation == generation && continues);
            let changed_fields = previous_same_generation
                .and_then(|old| naval_combat_changed_fields(old, &sample))
                .unwrap_or_else(naval_combat_all_fields);
            if !changed_fields.is_empty() {
                push_naval_combat_delta_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    &sample,
                    "sample",
                    &changed_fields,
                );
                let event_type = if previous_same_generation.is_some() {
                    NAVAL_COMBAT_EVENT_UPDATED
                } else {
                    NAVAL_COMBAT_EVENT_STARTED
                };
                push_naval_combat_event_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    &sample,
                    event_type,
                    &changed_fields,
                    None,
                );
            }
            current.insert(sample.combat_id, sample);
        }

        for old in tracker.previous.values() {
            if current.contains_key(&old.combat_id) {
                continue;
            }
            push_naval_combat_delta_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old,
                "delete",
                &["combat_key"],
            );
            push_naval_combat_event_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old,
                NAVAL_COMBAT_EVENT_ENDED,
                &["combat_key"],
                Some("resolved"),
            );
        }

        tracker.previous = current;
        lines
    }

    fn push_naval_combat_delta_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &NavalCombatSample,
        op: &str,
        changed_fields: &[&str],
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = naval_combat_raw_domain_delta_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            op,
            changed_fields,
        ) {
            lines.push(line);
            *session_sequence = session_sequence.saturating_add(1);
        }
    }

    fn push_naval_combat_event_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &NavalCombatSample,
        event_type: &str,
        changed_fields: &[&str],
        end_reason: Option<&str>,
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = naval_combat_raw_event_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            event_type,
            changed_fields,
            end_reason,
        ) {
            lines.push(line);
            *session_sequence = session_sequence.saturating_add(1);
        }
    }

    fn sample_naval_combats(
        state: &CollectorState,
        control: &CollectorControl,
    ) -> Result<Vec<NavalCombatSample>, &'static str> {
        if !(state.daily_match && state.long_string_match) {
            return Err("state_validation_failed");
        }
        if !(control.naval_combat_body_bridge_valid
            && naval_combat_body_bridge_well_formed(&control.naval_combat_body_bridge))
        {
            return Err("bridge_invalid");
        }
        if control.naval_combat_body_bridge.is_empty() {
            return Ok(Vec::new());
        }
        let tag_table = country_tag_table(state.country_database_ptr as usize)
            .ok_or("tag_table_unavailable")?;
        let mut by_id = BTreeMap::new();
        let mut invalid_entries = 0_u32;
        for entry in &control.naval_combat_body_bridge {
            if let Some(sample) =
                sample_naval_combat_from_bridge_entry(state.base, tag_table, entry)
            {
                by_id.entry(sample.combat_id).or_insert(sample);
            } else {
                invalid_entries = invalid_entries.saturating_add(1);
            }
        }
        if by_id.is_empty() && invalid_entries > 0 {
            return Err("bridge_entry_invalid");
        }
        Ok(by_id.into_values().collect())
    }

    fn sample_naval_combat_from_bridge_entry(
        module_base: usize,
        tag_table: usize,
        entry: &NavalCombatBodyBridgeEntry,
    ) -> Option<NavalCombatSample> {
        let body = entry.body_ptr;
        if !is_map_control_heap_pointer(body) {
            return None;
        }
        if read_u64(body)? as usize != module_base.checked_add(NAVAL_COMBAT_VTABLE_RVA)? {
            return None;
        }
        let handle = read_u64(body.checked_add(NAVAL_COMBAT_OFF_TYPEID)?)?;
        if (handle & 0xFFFF_FFFF) as u32 != NAVAL_COMBAT_TYPE {
            return None;
        }
        let combat_id = ((handle >> 32) & 0xFFFF_FFFF) as u32;
        if combat_id == 0 || combat_id != entry.combat_id || combat_id > MAX_NAVAL_COMBAT_ID {
            return None;
        }
        let front = read_u64(body.checked_add(NAVAL_COMBAT_OFF_FRONT)?)? as usize;
        let location = Some(read_land_combat_location(module_base, front)?);
        let (attacker_side, defender_side) =
            naval_combat_sides(module_base, body, combat_id, front)?;

        let attacker_tags = read_naval_combat_side_tags(tag_table, attacker_side);
        let defender_tags = read_naval_combat_side_tags(tag_table, defender_side);
        let attacker_ship_count = read_naval_combat_side_ship_count(module_base, attacker_side);
        let defender_ship_count = read_naval_combat_side_ship_count(module_base, defender_side);
        let attacker_status = naval_combat_side_status(attacker_tags.as_ref(), attacker_ship_count);
        let defender_status = naval_combat_side_status(defender_tags.as_ref(), defender_ship_count);
        Some(NavalCombatSample {
            combat_id,
            generation: 0,
            body_addr: body,
            location,
            attacker_tags,
            defender_tags,
            attacker_ship_count,
            defender_ship_count,
            attacker_commander: read_naval_combat_commander(attacker_side),
            defender_commander: read_naval_combat_commander(defender_side),
            attacker_positioning: read_naval_combat_positioning(attacker_side),
            defender_positioning: read_naval_combat_positioning(defender_side),
            attacker_status,
            defender_status,
        })
    }

    fn naval_combat_sides(
        module_base: usize,
        body: usize,
        combat_id: u32,
        front: usize,
    ) -> Option<(usize, usize)> {
        let side_vtable = module_base.checked_add(NAVAL_COMBAT_SIDE_VTABLE_RVA)?;
        let registry_vtable = module_base.checked_add(NAVAL_COMBAT_REGISTRY_VTABLE_RVA)?;
        let attacker = read_u64(body.checked_add(NAVAL_COMBAT_OFF_ATTACKER_SIDE)?)? as usize;
        let defender = read_u64(body.checked_add(NAVAL_COMBAT_OFF_DEFENDER_SIDE)?)? as usize;
        if !is_map_control_heap_pointer(attacker) || read_u64(attacker)? as usize != side_vtable {
            return None;
        }
        if !is_map_control_heap_pointer(defender) || read_u64(defender)? as usize != side_vtable {
            return None;
        }
        let registry = read_u64(attacker.checked_add(NAVAL_COMBAT_SIDE_OFF_REGISTRY)?)? as usize;
        if !is_map_control_heap_pointer(registry) || read_u64(registry)? as usize != registry_vtable
        {
            return None;
        }
        let handle = read_u64(registry.checked_add(NAVAL_COMBAT_REGISTRY_OFF_HANDLE)?)?;
        if (handle & 0xFFFF_FFFF) as u32 != NAVAL_COMBAT_TYPE
            || ((handle >> 32) & 0xFFFF_FFFF) as u32 != combat_id
        {
            return None;
        }
        if read_u64(registry.checked_add(NAVAL_COMBAT_REGISTRY_OFF_ATTACKER_SIDE)?)? as usize
            != attacker
        {
            return None;
        }
        if read_u64(registry.checked_add(NAVAL_COMBAT_REGISTRY_OFF_DEFENDER_SIDE)?)? as usize
            != defender
        {
            return None;
        }
        if is_map_control_heap_pointer(front)
            && read_u64(registry.checked_add(NAVAL_COMBAT_REGISTRY_OFF_FRONT)?)? as usize != front
        {
            return None;
        }
        Some((attacker, defender))
    }

    fn read_naval_combat_side_tags(tag_table: usize, side: usize) -> Option<Vec<String>> {
        let size = read_u32(side.checked_add(NAVAL_COMBAT_SIDE_OFF_COUNTRIES_SIZE)?)?;
        if size > MAX_NAVAL_COMBAT_SIDE_COUNTRIES {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }
        let data = read_u64(side.checked_add(NAVAL_COMBAT_SIDE_OFF_COUNTRIES_DATA)?)? as usize;
        if !is_map_control_heap_pointer(data) {
            return None;
        }
        let byte_len = (size as usize).checked_mul(8)?;
        let raw = read_bytes(data, byte_len)?;
        if raw.len() != byte_len {
            return None;
        }
        let mut tags = Vec::with_capacity(size as usize);
        for i in 0..size as usize {
            let handle = read_u64_from_slice(&raw, i * 8)?;
            let tag_db_id = (handle & 0xFFFF_FFFF) as u32;
            tags.push(tag_from_table(tag_table, tag_db_id)?);
        }
        tags.sort();
        tags.dedup();
        Some(tags)
    }

    fn read_naval_combat_side_ship_count(module_base: usize, side: usize) -> Option<u32> {
        let group_count = read_u32(side.checked_add(NAVAL_COMBAT_SIDE_OFF_FEX_SIZE)?)?;
        if group_count > MAX_NAVAL_COMBAT_SIDE_GROUPS {
            return None;
        }
        if group_count == 0 {
            return Some(0);
        }
        let group_data = read_u64(side.checked_add(NAVAL_COMBAT_SIDE_OFF_FEX_DATA)?)? as usize;
        if !is_map_control_heap_pointer(group_data) {
            return None;
        }
        let byte_len = (group_count as usize).checked_mul(8)?;
        let raw = read_bytes(group_data, byte_len)?;
        if raw.len() != byte_len {
            return None;
        }
        let member_vtable = module_base.checked_add(NAVAL_COMBAT_MEMBER_VTABLE_RVA)?;
        let mut total = 0_u32;
        for i in 0..group_count as usize {
            let group = read_u64_from_slice(&raw, i * 8)? as usize;
            if !is_map_control_heap_pointer(group) {
                return None;
            }
            let members_data =
                read_u64(group.checked_add(NAVAL_COMBAT_FEX_OFF_MEMBERS_DATA)?)? as usize;
            let packed = read_u64(group.checked_add(NAVAL_COMBAT_FEX_OFF_MEMBERS_PACKED)?)?;
            let size = ((packed >> 32) & 0xFFFF_FFFF) as u32;
            if size > MAX_NAVAL_COMBAT_SHIPS_PER_GROUP {
                return None;
            }
            total = total.checked_add(size)?;
            if total > MAX_NAVAL_COMBAT_SIDE_SHIPS {
                return None;
            }
            if size == 0 {
                continue;
            }
            if !is_map_control_heap_pointer(members_data) {
                return None;
            }
            let member_bytes = (size as usize).checked_mul(8)?;
            let members = read_bytes(members_data, member_bytes)?;
            if members.len() != member_bytes {
                return None;
            }
            for k in 0..size as usize {
                let member = read_u64_from_slice(&members, k * 8)? as usize;
                if !is_map_control_heap_pointer(member)
                    || read_u64(member)? as usize != member_vtable
                {
                    return None;
                }
            }
        }
        Some(total)
    }

    fn read_naval_combat_commander(side: usize) -> Option<String> {
        let leader = read_u64(side.checked_add(NAVAL_COMBAT_SIDE_OFF_LEADER)?)? as usize;
        if !is_map_control_heap_pointer(leader) {
            return None;
        }
        read_msvc_string(
            leader.checked_add(NAVAL_LEADER_NAME)?,
            MAX_NAVAL_COMMANDER_NAME_LEN,
        )
    }

    fn read_naval_combat_positioning(side: usize) -> Option<f64> {
        read_f32(side.checked_add(NAVAL_COMBAT_SIDE_OFF_POSITIONING)?)
            .map(|value| value as f64)
            .filter(|value| value.is_finite() && (0.0..=10.0).contains(value))
    }

    fn naval_combat_side_status(
        tags: Option<&Vec<String>>,
        ship_count: Option<u32>,
    ) -> &'static str {
        if tags.is_some() && ship_count.is_some() {
            "exact"
        } else {
            "validation_failed"
        }
    }

    fn naval_combat_continues(old: Option<&NavalCombatSample>, new: &NavalCombatSample) -> bool {
        let Some(old) = old else {
            return false;
        };
        old.body_addr == new.body_addr
    }

    fn naval_combat_split_end_reason(
        old: &NavalCombatSample,
        new: &NavalCombatSample,
    ) -> &'static str {
        if old.body_addr != new.body_addr {
            "body_changed"
        } else {
            "id_reuse"
        }
    }

    fn naval_combat_all_fields() -> Vec<&'static str> {
        vec![
            "combat_key",
            "combat_id",
            "generation",
            "location",
            "attacker_tags",
            "defender_tags",
            "attacker_ship_count",
            "defender_ship_count",
            "attacker_commander",
            "defender_commander",
            "attacker_positioning",
            "defender_positioning",
            "attacker_status",
            "defender_status",
        ]
    }

    fn naval_combat_changed_fields(
        old: &NavalCombatSample,
        new: &NavalCombatSample,
    ) -> Option<Vec<&'static str>> {
        let mut changed = Vec::new();
        if old.location != new.location {
            changed.push("location");
        }
        if old.attacker_tags != new.attacker_tags {
            changed.push("attacker_tags");
        }
        if old.defender_tags != new.defender_tags {
            changed.push("defender_tags");
        }
        if old.attacker_ship_count != new.attacker_ship_count {
            changed.push("attacker_ship_count");
        }
        if old.defender_ship_count != new.defender_ship_count {
            changed.push("defender_ship_count");
        }
        if old.attacker_commander != new.attacker_commander {
            changed.push("attacker_commander");
        }
        if old.defender_commander != new.defender_commander {
            changed.push("defender_commander");
        }
        if old.attacker_positioning != new.attacker_positioning {
            changed.push("attacker_positioning");
        }
        if old.defender_positioning != new.defender_positioning {
            changed.push("defender_positioning");
        }
        if old.attacker_status != new.attacker_status {
            changed.push("attacker_status");
        }
        if old.defender_status != new.defender_status {
            changed.push("defender_status");
        }
        (!changed.is_empty()).then_some(changed)
    }

    fn naval_combat_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &NavalCombatSample,
        op: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        let combat_key = format!("naval_combat:{}:{}", sample.combat_id, sample.generation);
        let after_json = if op == "delete" {
            "null".to_string()
        } else {
            naval_combat_row_json(sample, &combat_key)
        };
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_NAVAL_COMBATS_DOMAIN,
            entity_key: &combat_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            op,
            after_json: &after_json,
            changed_fields,
        })
    }

    fn naval_combat_gap_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        gap_kind: &str,
    ) -> Option<String> {
        if !valid_raw_short_string(gap_kind) || looks_like_hex_pointer(gap_kind) {
            return None;
        }
        let gap_key = format!("naval_air_combats:gap:{gap_kind}");
        let after_json = format!(
            "{{\"gap_key\":{},\"gap_kind\":{},\"status\":\"validation_failed\"}}",
            json_str(&gap_key),
            json_str(gap_kind)
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_NAVAL_COMBATS_DOMAIN,
            entity_key: &gap_key,
            confidence: "gap",
            op: "sample",
            after_json: &after_json,
            changed_fields: &["gap_key", "gap_kind", "status"],
        })
    }

    fn naval_combat_raw_event_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &NavalCombatSample,
        event_type: &str,
        changed_fields: &[&str],
        end_reason: Option<&str>,
    ) -> Option<String> {
        if !matches!(
            event_type,
            NAVAL_COMBAT_EVENT_STARTED | NAVAL_COMBAT_EVENT_UPDATED | NAVAL_COMBAT_EVENT_ENDED
        ) {
            return None;
        }
        let combat_key = format!("naval_combat:{}:{}", sample.combat_id, sample.generation);
        let detail_json =
            naval_combat_event_detail_json(sample, &combat_key, changed_fields, end_reason)?;
        raw_event_frame_line(RawEventFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_NAVAL_COMBATS_DOMAIN,
            entity_key: &combat_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            event_type,
            subject: "naval_combat",
            detail_json: &detail_json,
        })
    }

    fn naval_combat_row_json(sample: &NavalCombatSample, combat_key: &str) -> String {
        format!(
            "{{\"combat_key\":{},\"combat_id\":{},\"generation\":{},\
             \"location\":{},\"attacker_tags\":{},\"defender_tags\":{},\
             \"attacker_ship_count\":{},\"defender_ship_count\":{},\
             \"attacker_commander\":{},\"defender_commander\":{},\
             \"attacker_positioning\":{},\"defender_positioning\":{},\
             \"attacker_status\":{},\"defender_status\":{}}}",
            json_str(combat_key),
            sample.combat_id,
            sample.generation,
            json_opt_u32(sample.location),
            json_opt_string_vec(sample.attacker_tags.as_ref()),
            json_opt_string_vec(sample.defender_tags.as_ref()),
            json_opt_u32(sample.attacker_ship_count),
            json_opt_u32(sample.defender_ship_count),
            json_opt_escaped_str(sample.attacker_commander.as_deref()),
            json_opt_escaped_str(sample.defender_commander.as_deref()),
            json_opt_f64(sample.attacker_positioning),
            json_opt_f64(sample.defender_positioning),
            json_str(sample.attacker_status),
            json_str(sample.defender_status)
        )
    }

    fn naval_combat_event_detail_json(
        sample: &NavalCombatSample,
        combat_key: &str,
        changed_fields: &[&str],
        end_reason: Option<&str>,
    ) -> Option<String> {
        if changed_fields.len() > 256
            || changed_fields
                .iter()
                .any(|field| !valid_raw_short_string(field))
        {
            return None;
        }
        if end_reason
            .map(|reason| !valid_raw_short_string(reason) || looks_like_hex_pointer(reason))
            .unwrap_or(false)
        {
            return None;
        }
        let changed_fields = changed_fields
            .iter()
            .map(|field| json_str(field))
            .collect::<Vec<_>>()
            .join(",");
        let end_reason = end_reason
            .map(|reason| format!(",\"end_reason\":{}", json_str(reason)))
            .unwrap_or_default();
        Some(format!(
            "{{\"combat_key\":{},\"combat_id\":{},\"generation\":{},\
             \"location\":{},\"attacker_tags\":{},\"defender_tags\":{},\
             \"attacker_ship_count\":{},\"defender_ship_count\":{},\
             \"attacker_commander\":{},\"defender_commander\":{},\
             \"attacker_positioning\":{},\"defender_positioning\":{},\
             \"attacker_status\":{},\"defender_status\":{},\
             \"changed_fields\":[{}]{} }}",
            json_str(combat_key),
            sample.combat_id,
            sample.generation,
            json_opt_u32(sample.location),
            json_opt_string_vec(sample.attacker_tags.as_ref()),
            json_opt_string_vec(sample.defender_tags.as_ref()),
            json_opt_u32(sample.attacker_ship_count),
            json_opt_u32(sample.defender_ship_count),
            json_opt_escaped_str(sample.attacker_commander.as_deref()),
            json_opt_escaped_str(sample.defender_commander.as_deref()),
            json_opt_f64(sample.attacker_positioning),
            json_opt_f64(sample.defender_positioning),
            json_str(sample.attacker_status),
            json_str(sample.defender_status),
            changed_fields,
            end_reason
        ))
    }

    fn build_naval_combat_member_raw_lines_from_combat_samples(
        pid: Dword,
        state: &CollectorState,
        control: &CollectorControl,
        tracker: &mut NavalCombatMemberTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        combat_samples: Vec<NavalCombatSample>,
    ) -> Vec<String> {
        let tag_table = if combat_samples.is_empty() {
            None
        } else {
            match country_tag_table(state.country_database_ptr as usize) {
                Some(table) => Some(table),
                None => {
                    return naval_combat_member_gap_raw_domain_delta_frame_line(
                        pid,
                        FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                        Some(first_session_sequence),
                        unsafe { GetTickCount64() },
                        control,
                        elapsed_wall_ms,
                        capture_boundary,
                        clock,
                        "tag_table_unavailable",
                    )
                    .into_iter()
                    .collect();
                }
            }
        };
        let mut invalid_combat_rows = 0_u32;
        let mut current = BTreeMap::new();
        for combat in combat_samples {
            let Some(tag_table) = tag_table else {
                continue;
            };
            match sample_naval_combat_members_from_sample(state.base, tag_table, &combat) {
                Some(samples) => {
                    for sample in samples {
                        current.entry(sample.member_key.clone()).or_insert(sample);
                    }
                }
                None => {
                    invalid_combat_rows = invalid_combat_rows.saturating_add(1);
                }
            }
        }
        if current.is_empty() && invalid_combat_rows > 0 {
            return naval_combat_member_gap_raw_domain_delta_frame_line(
                pid,
                FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                Some(first_session_sequence),
                unsafe { GetTickCount64() },
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                "member_entry_invalid",
            )
            .into_iter()
            .collect();
        }

        let mut lines = Vec::new();
        let mut session_sequence = first_session_sequence;
        for sample in current.values() {
            let changed_fields = tracker
                .previous
                .get(&sample.member_key)
                .and_then(|old| naval_combat_member_changed_fields(old, sample))
                .unwrap_or_else(naval_combat_member_all_fields);
            if changed_fields.is_empty() {
                continue;
            }
            push_naval_combat_member_delta_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                sample,
                "sample",
                &changed_fields,
            );
        }
        for old in tracker.previous.values() {
            if current.contains_key(&old.member_key) {
                continue;
            }
            push_naval_combat_member_delta_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old,
                "delete",
                &["member_key"],
            );
        }
        tracker.previous = current;
        lines
    }

    fn push_naval_combat_member_delta_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &NavalCombatMemberSample,
        op: &str,
        changed_fields: &[&str],
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = naval_combat_member_raw_domain_delta_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            op,
            changed_fields,
        ) {
            lines.push(line);
            *session_sequence = session_sequence.saturating_add(1);
        }
    }

    fn sample_naval_combat_members_from_sample(
        module_base: usize,
        tag_table: usize,
        combat: &NavalCombatSample,
    ) -> Option<Vec<NavalCombatMemberSample>> {
        let body = combat.body_addr;
        if !is_map_control_heap_pointer(body) {
            return None;
        }
        if read_u64(body)? as usize != module_base.checked_add(NAVAL_COMBAT_VTABLE_RVA)? {
            return None;
        }
        let handle = read_u64(body.checked_add(NAVAL_COMBAT_OFF_TYPEID)?)?;
        if (handle & 0xFFFF_FFFF) as u32 != NAVAL_COMBAT_TYPE
            || ((handle >> 32) & 0xFFFF_FFFF) as u32 != combat.combat_id
        {
            return None;
        }
        let front = read_u64(body.checked_add(NAVAL_COMBAT_OFF_FRONT)?)? as usize;
        let (attacker_side, defender_side) =
            naval_combat_sides(module_base, body, combat.combat_id, front)?;
        let combat_key = format!("naval_combat:{}:{}", combat.combat_id, combat.generation);
        let mut samples = Vec::new();
        samples.extend(sample_naval_combat_members_from_side(
            module_base,
            tag_table,
            combat,
            &combat_key,
            attacker_side,
            "attacker",
        )?);
        samples.extend(sample_naval_combat_members_from_side(
            module_base,
            tag_table,
            combat,
            &combat_key,
            defender_side,
            "defender",
        )?);
        Some(samples)
    }

    fn sample_naval_combat_members_from_side(
        module_base: usize,
        tag_table: usize,
        combat: &NavalCombatSample,
        combat_key: &str,
        side: usize,
        side_name: &'static str,
    ) -> Option<Vec<NavalCombatMemberSample>> {
        let group_count = read_u32(side.checked_add(NAVAL_COMBAT_SIDE_OFF_FEX_SIZE)?)?;
        if group_count > MAX_NAVAL_COMBAT_SIDE_GROUPS {
            return None;
        }
        if group_count == 0 {
            return Some(Vec::new());
        }
        let group_data = read_u64(side.checked_add(NAVAL_COMBAT_SIDE_OFF_FEX_DATA)?)? as usize;
        if !is_map_control_heap_pointer(group_data) {
            return None;
        }
        let group_bytes = (group_count as usize).checked_mul(8)?;
        let groups = read_bytes(group_data, group_bytes)?;
        if groups.len() != group_bytes {
            return None;
        }
        let member_vtable = module_base.checked_add(NAVAL_COMBAT_MEMBER_VTABLE_RVA)?;
        let mut total = 0_u32;
        let mut samples = Vec::new();
        for group_index in 0..group_count as usize {
            let group = read_u64_from_slice(&groups, group_index * 8)? as usize;
            if !is_map_control_heap_pointer(group) {
                return None;
            }
            let members_data =
                read_u64(group.checked_add(NAVAL_COMBAT_FEX_OFF_MEMBERS_DATA)?)? as usize;
            let packed = read_u64(group.checked_add(NAVAL_COMBAT_FEX_OFF_MEMBERS_PACKED)?)?;
            let capacity = (packed & 0xFFFF_FFFF) as u32;
            let size = ((packed >> 32) & 0xFFFF_FFFF) as u32;
            if size > MAX_NAVAL_COMBAT_SHIPS_PER_GROUP || capacity < size {
                return None;
            }
            total = total.checked_add(size)?;
            if total > MAX_NAVAL_COMBAT_SIDE_SHIPS {
                return None;
            }
            if size == 0 {
                continue;
            }
            if !is_map_control_heap_pointer(members_data) {
                return None;
            }
            let member_bytes = (size as usize).checked_mul(8)?;
            let members = read_bytes(members_data, member_bytes)?;
            if members.len() != member_bytes {
                return None;
            }
            for member_index in 0..size as usize {
                let member = read_u64_from_slice(&members, member_index * 8)? as usize;
                if !is_map_control_heap_pointer(member)
                    || read_u64(member)? as usize != member_vtable
                {
                    return None;
                }
                samples.push(read_naval_combat_member_sample(
                    tag_table,
                    combat,
                    combat_key,
                    side_name,
                    group_index as u32,
                    member_index as u32,
                    member,
                )?);
            }
        }
        Some(samples)
    }

    fn read_naval_combat_member_sample(
        tag_table: usize,
        combat: &NavalCombatSample,
        combat_key: &str,
        side: &'static str,
        group_index: u32,
        member_index: u32,
        member: usize,
    ) -> Option<NavalCombatMemberSample> {
        let packed_id = read_u64(member.checked_add(NAVAL_COMBAT_MEMBER_OFF_TYPEID)?)?;
        let unit_type_tag = (packed_id & 0xFFFF_FFFF) as u32;
        let raw_instance_id = ((packed_id >> 32) & 0xFFFF_FFFF) as u32;
        let is_convoy = unit_type_tag == 0 && raw_instance_id == 0;
        let ship_instance_id = if is_convoy {
            None
        } else {
            if unit_type_tag != NAVAL_COMBAT_SHIP_TYPE_TAG
                || raw_instance_id == 0
                || raw_instance_id > MAX_NAVAL_COMBAT_SHIP_INSTANCE_ID
            {
                return None;
            }
            Some(raw_instance_id)
        };
        let owner_tag_db_id = read_u32(member.checked_add(NAVAL_COMBAT_MEMBER_OFF_OWNER_TAG)?)?;
        let owner = if owner_tag_db_id == 0 {
            None
        } else {
            Some(tag_from_table(tag_table, owner_tag_db_id)?)
        };
        let battle_slot_index_raw = read_u32(member.checked_add(NAVAL_COMBAT_MEMBER_OFF_SLOT)?)?;
        if battle_slot_index_raw > MAX_NAVAL_COMBAT_SIDE_SHIPS {
            return None;
        }
        let member_key = if let Some(instance_id) = ship_instance_id {
            format!("{RAW_NAVAL_COMBAT_MEMBERS_DOMAIN}:{combat_key}:{side}:ship:{instance_id}")
        } else {
            format!(
                "{RAW_NAVAL_COMBAT_MEMBERS_DOMAIN}:{combat_key}:{side}:convoy:{group_index}:{member_index}:{battle_slot_index_raw}"
            )
        };
        let designed_max_strength_raw =
            read_u32(member.checked_add(NAVAL_COMBAT_MEMBER_OFF_MAX_STRENGTH)?)
                .filter(|value| *value <= MAX_NAVAL_COMBAT_MEMBER_MAX_STRENGTH_RAW);
        let engagement_weight_raw =
            read_u32(member.checked_add(NAVAL_COMBAT_MEMBER_OFF_ENGAGEMENT_WEIGHT)?)
                .filter(|value| *value <= MAX_NAVAL_COMBAT_MEMBER_ENGAGEMENT_WEIGHT_RAW);
        Some(NavalCombatMemberSample {
            member_key,
            combat_key: combat_key.to_string(),
            combat_id: combat.combat_id,
            generation: combat.generation,
            side,
            group_index,
            member_index,
            unit_type_tag,
            ship_instance_id,
            is_convoy,
            owner,
            name: read_msvc_string(
                member.checked_add(NAVAL_COMBAT_MEMBER_OFF_NAME)?,
                MAX_NAVAL_COMBAT_MEMBER_STRING_LEN,
            ),
            type_display: read_msvc_string(
                member.checked_add(NAVAL_COMBAT_MEMBER_OFF_TYPE_DISPLAY)?,
                MAX_NAVAL_COMBAT_MEMBER_STRING_LEN,
            ),
            archetype: read_msvc_string(
                member.checked_add(NAVAL_COMBAT_MEMBER_OFF_ARCHETYPE)?,
                MAX_NAVAL_COMBAT_MEMBER_STRING_LEN,
            ),
            designed_max_strength_raw,
            battle_slot_index_raw,
            state_bitmask: read_u8(member.checked_add(NAVAL_COMBAT_MEMBER_OFF_STATE)?)
                .map(|value| value as u32),
            engagement_weight_raw,
        })
    }

    fn naval_combat_member_all_fields() -> Vec<&'static str> {
        vec![
            "member_key",
            "combat_key",
            "combat_id",
            "generation",
            "side",
            "group_index",
            "member_index",
            "unit_type_tag",
            "ship_instance_id",
            "is_convoy",
            "owner",
            "name",
            "type_display",
            "archetype",
            "designed_max_strength_raw",
            "battle_slot_index_raw",
            "state_bitmask",
            "engagement_weight_raw",
        ]
    }

    fn naval_combat_member_changed_fields(
        old: &NavalCombatMemberSample,
        new: &NavalCombatMemberSample,
    ) -> Option<Vec<&'static str>> {
        let mut changed = Vec::new();
        if old.combat_key != new.combat_key {
            changed.push("combat_key");
        }
        if old.combat_id != new.combat_id {
            changed.push("combat_id");
        }
        if old.generation != new.generation {
            changed.push("generation");
        }
        if old.side != new.side {
            changed.push("side");
        }
        if old.group_index != new.group_index {
            changed.push("group_index");
        }
        if old.member_index != new.member_index {
            changed.push("member_index");
        }
        if old.unit_type_tag != new.unit_type_tag {
            changed.push("unit_type_tag");
        }
        if old.ship_instance_id != new.ship_instance_id {
            changed.push("ship_instance_id");
        }
        if old.is_convoy != new.is_convoy {
            changed.push("is_convoy");
        }
        if old.owner != new.owner {
            changed.push("owner");
        }
        if old.name != new.name {
            changed.push("name");
        }
        if old.type_display != new.type_display {
            changed.push("type_display");
        }
        if old.archetype != new.archetype {
            changed.push("archetype");
        }
        if old.designed_max_strength_raw != new.designed_max_strength_raw {
            changed.push("designed_max_strength_raw");
        }
        if old.battle_slot_index_raw != new.battle_slot_index_raw {
            changed.push("battle_slot_index_raw");
        }
        if old.state_bitmask != new.state_bitmask {
            changed.push("state_bitmask");
        }
        if old.engagement_weight_raw != new.engagement_weight_raw {
            changed.push("engagement_weight_raw");
        }
        (!changed.is_empty()).then_some(changed)
    }

    fn naval_combat_member_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &NavalCombatMemberSample,
        op: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        let after_json = if op == "delete" {
            "null".to_string()
        } else {
            naval_combat_member_row_json(sample)
        };
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_NAVAL_COMBAT_MEMBERS_DOMAIN,
            entity_key: &sample.member_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            op,
            after_json: &after_json,
            changed_fields,
        })
    }

    fn naval_combat_member_gap_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        gap_kind: &str,
    ) -> Option<String> {
        if !valid_raw_short_string(gap_kind) || looks_like_hex_pointer(gap_kind) {
            return None;
        }
        let gap_key = format!("{RAW_NAVAL_COMBAT_MEMBERS_DOMAIN}:gap:{gap_kind}");
        let after_json = format!(
            "{{\"gap_key\":{},\"gap_kind\":{},\"status\":\"validation_failed\"}}",
            json_str(&gap_key),
            json_str(gap_kind)
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_NAVAL_COMBAT_MEMBERS_DOMAIN,
            entity_key: &gap_key,
            confidence: "gap",
            op: "sample",
            after_json: &after_json,
            changed_fields: &["gap_key", "gap_kind", "status"],
        })
    }

    fn naval_combat_member_row_json(sample: &NavalCombatMemberSample) -> String {
        format!(
            "{{\"member_key\":{},\"combat_key\":{},\"combat_id\":{},\"generation\":{},\
             \"side\":{},\"group_index\":{},\"member_index\":{},\
             \"unit_type_tag\":{},\"ship_instance_id\":{},\"is_convoy\":{},\
             \"owner\":{},\"name\":{},\"type_display\":{},\"archetype\":{},\
             \"designed_max_strength_raw\":{},\"battle_slot_index_raw\":{},\
             \"state_bitmask\":{},\"engagement_weight_raw\":{}}}",
            json_str(&sample.member_key),
            json_str(&sample.combat_key),
            sample.combat_id,
            sample.generation,
            json_str(sample.side),
            sample.group_index,
            sample.member_index,
            sample.unit_type_tag,
            json_opt_u32(sample.ship_instance_id),
            sample.is_convoy,
            json_opt_escaped_str(sample.owner.as_deref()),
            json_opt_escaped_str(sample.name.as_deref()),
            json_opt_escaped_str(sample.type_display.as_deref()),
            json_opt_escaped_str(sample.archetype.as_deref()),
            json_opt_u32(sample.designed_max_strength_raw),
            sample.battle_slot_index_raw,
            json_opt_u32(sample.state_bitmask),
            json_opt_u32(sample.engagement_weight_raw)
        )
    }

    fn build_air_wing_raw_domain_delta_lines(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        tracker: &mut AirWingTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Vec<String> {
        let Some(state) = state else {
            return Vec::new();
        };
        let Some(clock) = clock_sample_from_state(Some(state)) else {
            return Vec::new();
        };
        let current_samples = match sample_air_wings(state, control) {
            Ok(samples) => samples,
            Err(gap_kind) => {
                let tick_ms = unsafe { GetTickCount64() };
                let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                return air_wing_gap_raw_domain_delta_frame_line(
                    pid,
                    sequence,
                    Some(first_session_sequence),
                    tick_ms,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    &clock,
                    gap_kind,
                )
                .into_iter()
                .collect();
            }
        };
        if current_samples.is_empty() && tracker.previous.is_empty() {
            return Vec::new();
        }

        build_air_wing_raw_lines_from_samples(
            pid,
            control,
            tracker,
            first_session_sequence,
            elapsed_wall_ms,
            capture_boundary,
            &clock,
            current_samples,
        )
    }

    fn build_air_wing_raw_lines_from_samples(
        pid: Dword,
        control: &CollectorControl,
        tracker: &mut AirWingTracker,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        current_samples: Vec<AirWingSample>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let mut session_sequence = first_session_sequence;
        let mut current = BTreeMap::new();
        for mut sample in current_samples {
            let key = (sample.owner.clone(), sample.wing_id);
            let old = tracker.previous.get(&key).cloned();
            let continues = old
                .as_ref()
                .map(|old| old.body_addr == sample.body_addr)
                .unwrap_or(false);
            if let Some(old_sample) = old.as_ref().filter(|_| !continues) {
                push_air_wing_delta_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    old_sample,
                    "delete",
                    &["air_wing_key"],
                );
                push_air_wing_event_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    old_sample,
                    AIR_WING_EVENT_LOST,
                    &["air_wing_key"],
                );
            }
            sample.generation = if continues {
                old.as_ref().map(|old| old.generation).unwrap_or(0)
            } else {
                let generation = tracker.next_generation;
                tracker.next_generation = tracker.next_generation.saturating_add(1);
                generation
            };
            let changed_fields = tracker
                .previous
                .get(&key)
                .filter(|old| old.generation == sample.generation && continues)
                .and_then(|old| air_wing_changed_fields(old, &sample))
                .unwrap_or_else(air_wing_all_fields);
            if !changed_fields.is_empty() {
                push_air_wing_delta_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    &sample,
                    "sample",
                    &changed_fields,
                );
                let event_type = if continues {
                    AIR_WING_EVENT_UPDATED
                } else {
                    AIR_WING_EVENT_OBSERVED
                };
                push_air_wing_event_frame(
                    &mut lines,
                    &mut session_sequence,
                    pid,
                    control,
                    elapsed_wall_ms,
                    capture_boundary,
                    clock,
                    &sample,
                    event_type,
                    &changed_fields,
                );
            }
            current.insert(key, sample);
        }

        for (key, old) in &tracker.previous {
            if current.contains_key(key) {
                continue;
            }
            push_air_wing_delta_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old,
                "delete",
                &["air_wing_key"],
            );
            push_air_wing_event_frame(
                &mut lines,
                &mut session_sequence,
                pid,
                control,
                elapsed_wall_ms,
                capture_boundary,
                clock,
                old,
                AIR_WING_EVENT_LOST,
                &["air_wing_key"],
            );
        }

        tracker.previous = current;
        lines
    }

    fn push_air_wing_delta_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &AirWingSample,
        op: &str,
        changed_fields: &[&str],
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = air_wing_raw_domain_delta_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            op,
            changed_fields,
        ) {
            lines.push(line);
            *session_sequence = session_sequence.saturating_add(1);
        }
    }

    fn push_air_wing_event_frame(
        lines: &mut Vec<String>,
        session_sequence: &mut u64,
        pid: Dword,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &AirWingSample,
        event_type: &str,
        changed_fields: &[&str],
    ) {
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if let Some(line) = air_wing_raw_event_frame_line(
            pid,
            sequence,
            Some(*session_sequence),
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            clock,
            sample,
            event_type,
            changed_fields,
        ) {
            lines.push(line);
            *session_sequence = session_sequence.saturating_add(1);
        }
    }

    fn sample_air_wings(
        state: &CollectorState,
        control: &CollectorControl,
    ) -> Result<Vec<AirWingSample>, &'static str> {
        if !(state.daily_match && state.long_string_match) {
            return Err("state_validation_failed");
        }
        if !(control.air_wing_body_bridge_valid
            && air_wing_body_bridge_well_formed(&control.air_wing_body_bridge))
        {
            return Err("bridge_invalid");
        }
        if control.air_wing_body_bridge.is_empty() {
            return Ok(Vec::new());
        }
        let tag_table = country_tag_table(state.country_database_ptr as usize)
            .ok_or("tag_table_unavailable")?;
        let mut by_key = BTreeMap::new();
        let mut invalid_entries = 0_u32;
        for entry in &control.air_wing_body_bridge {
            if let Some(sample) = sample_air_wing_from_bridge_entry(state.base, tag_table, entry) {
                by_key
                    .entry((sample.owner.clone(), sample.wing_id))
                    .or_insert(sample);
            } else {
                invalid_entries = invalid_entries.saturating_add(1);
            }
        }
        if by_key.is_empty() && invalid_entries > 0 {
            return Err("bridge_entry_invalid");
        }
        Ok(by_key.into_values().collect())
    }

    fn sample_air_wing_from_bridge_entry(
        module_base: usize,
        tag_table: usize,
        entry: &AirWingBodyBridgeEntry,
    ) -> Option<AirWingSample> {
        let body = entry.body_ptr;
        if !is_map_control_heap_pointer(body) {
            return None;
        }
        if read_u64(body)? as usize != module_base.checked_add(AIR_WING_VTABLE_RVA)? {
            return None;
        }
        if read_u32(body.checked_add(AIR_WING_OFF_TYPE)?)? != AIR_WING_TYPE {
            return None;
        }
        let wing_id = read_u32(body.checked_add(AIR_WING_OFF_ID)?)?;
        if wing_id == 0 || wing_id != entry.wing_id || wing_id > MAX_AIR_WING_ID {
            return None;
        }
        let owner_tag_db_id = read_air_wing_owner_tag_db_id(module_base, body)?;
        if owner_tag_db_id != entry.owner_tag_db_id {
            return None;
        }
        let owner = tag_from_table(tag_table, owner_tag_db_id)?;
        let count = read_u32(body.checked_add(AIR_WING_OFF_COUNT)?)?;
        let effective_planes = read_u32(body.checked_add(AIR_WING_OFF_EFFECTIVE_PLANES)?)?;
        if count > MAX_AIR_WING_PLANES || effective_planes > MAX_AIR_WING_PLANES {
            return None;
        }
        Some(AirWingSample {
            owner,
            wing_id,
            generation: 0,
            body_addr: body,
            count,
            mission_mask: read_u32(body.checked_add(AIR_WING_OFF_MISSION_MASK)?)?,
            mission_region: read_air_wing_mission_region(module_base, body),
            effective_planes,
        })
    }

    fn read_air_wing_owner_tag_db_id(module_base: usize, body: usize) -> Option<u32> {
        let pool = read_u64(body.checked_add(AIR_WING_OFF_POOL)?)? as usize;
        if !is_map_control_heap_pointer(pool)
            || read_u64(pool)? as usize != module_base.checked_add(AIR_WING_POOL_VTABLE_RVA)?
        {
            return None;
        }
        let country = read_u64(pool.checked_add(AIR_WING_POOL_OFF_COUNTRY)?)? as usize;
        if !is_map_control_heap_pointer(country)
            || read_u64(country)? as usize
                != module_base.checked_add(AIR_WING_OWNER_COUNTRY_VTABLE_RVA)?
        {
            return None;
        }
        let tag_db_id = read_u32(country.checked_add(AIR_WING_COUNTRY_OFF_TAG)?)?;
        (tag_db_id > 0 && tag_db_id <= 1024).then_some(tag_db_id)
    }

    fn read_air_wing_mission_region(module_base: usize, body: usize) -> Option<u32> {
        let region = read_u64(body.checked_add(AIR_WING_OFF_MISSION_REGION)?)? as usize;
        if !is_map_control_heap_pointer(region) {
            return None;
        }
        if read_u64(region)? as usize != module_base.checked_add(AIR_WING_REGION_VTABLE_RVA)? {
            return None;
        }
        if read_u64(region.checked_add(8)?)? as usize
            != module_base.checked_add(AIR_WING_REGION_ALT_VTABLE_RVA)?
        {
            return None;
        }
        let region_id = read_u32(region.checked_add(AIR_WING_REGION_OFF_ID)?)?;
        (region_id > 0 && region_id <= MAX_AIR_WING_MISSION_REGION_ID).then_some(region_id)
    }

    fn air_wing_all_fields() -> Vec<&'static str> {
        vec![
            "air_wing_key",
            "owner",
            "wing_id",
            "generation",
            "count",
            "mission_mask",
            "mission_region",
            "effective_planes",
        ]
    }

    fn air_wing_changed_fields(
        old: &AirWingSample,
        new: &AirWingSample,
    ) -> Option<Vec<&'static str>> {
        let mut changed = Vec::new();
        if old.owner != new.owner {
            changed.push("owner");
        }
        if old.count != new.count {
            changed.push("count");
        }
        if old.mission_mask != new.mission_mask {
            changed.push("mission_mask");
        }
        if old.mission_region != new.mission_region {
            changed.push("mission_region");
        }
        if old.effective_planes != new.effective_planes {
            changed.push("effective_planes");
        }
        (!changed.is_empty()).then_some(changed)
    }

    fn air_wing_key(sample: &AirWingSample) -> String {
        format!(
            "air_wing:{}:{}:{}",
            sample.owner, sample.wing_id, sample.generation
        )
    }

    fn air_wing_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &AirWingSample,
        op: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        let key = air_wing_key(sample);
        let after_json = if op == "delete" {
            "null".to_string()
        } else {
            air_wing_row_json(sample, &key)
        };
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_AIR_WINGS_DOMAIN,
            entity_key: &key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            op,
            after_json: &after_json,
            changed_fields,
        })
    }

    fn air_wing_gap_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        gap_kind: &str,
    ) -> Option<String> {
        if !valid_raw_short_string(gap_kind) || looks_like_hex_pointer(gap_kind) {
            return None;
        }
        let gap_key = format!("air_wings:gap:{gap_kind}");
        let after_json = format!(
            "{{\"gap_key\":{},\"gap_kind\":{},\"status\":\"validation_failed\"}}",
            json_str(&gap_key),
            json_str(gap_kind)
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_AIR_WINGS_DOMAIN,
            entity_key: &gap_key,
            confidence: "gap",
            op: "sample",
            after_json: &after_json,
            changed_fields: &["gap_key", "gap_kind", "status"],
        })
    }

    fn air_wing_raw_event_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &AirWingSample,
        event_type: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        if !matches!(
            event_type,
            AIR_WING_EVENT_OBSERVED | AIR_WING_EVENT_UPDATED | AIR_WING_EVENT_LOST
        ) {
            return None;
        }
        let key = air_wing_key(sample);
        let detail_json = air_wing_event_detail_json(sample, &key, changed_fields)?;
        raw_event_frame_line(RawEventFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_AIR_WINGS_DOMAIN,
            entity_key: &key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            event_type,
            subject: "air_wing",
            detail_json: &detail_json,
        })
    }

    fn air_wing_row_json(sample: &AirWingSample, key: &str) -> String {
        format!(
            "{{\"air_wing_key\":{},\"owner\":{},\"wing_id\":{},\"generation\":{},\
             \"count\":{},\"mission_mask\":{},\"mission_region\":{},\
             \"effective_planes\":{}}}",
            json_str(key),
            json_str(&sample.owner),
            sample.wing_id,
            sample.generation,
            sample.count,
            sample.mission_mask,
            json_opt_u32(sample.mission_region),
            sample.effective_planes
        )
    }

    fn air_wing_event_detail_json(
        sample: &AirWingSample,
        key: &str,
        changed_fields: &[&str],
    ) -> Option<String> {
        if changed_fields.len() > 256
            || changed_fields
                .iter()
                .any(|field| !valid_raw_short_string(field))
        {
            return None;
        }
        let changed_fields = changed_fields
            .iter()
            .map(|field| json_str(field))
            .collect::<Vec<_>>()
            .join(",");
        Some(format!(
            "{{\"air_wing_key\":{},\"owner\":{},\"wing_id\":{},\"generation\":{},\
             \"count\":{},\"mission_mask\":{},\"mission_region\":{},\
             \"effective_planes\":{},\"changed_fields\":[{}] }}",
            json_str(key),
            json_str(&sample.owner),
            sample.wing_id,
            sample.generation,
            sample.count,
            sample.mission_mask,
            json_opt_u32(sample.mission_region),
            sample.effective_planes,
            changed_fields
        ))
    }

    fn raw_event_frame_line(frame: RawEventFrame<'_>) -> Option<String> {
        if !valid_raw_short_string(frame.date)
            || !valid_raw_short_string(frame.capture_boundary)
            || !valid_raw_domain(frame.domain)
            || !valid_raw_key(frame.entity_key)
            || !valid_raw_confidence(frame.confidence)
            || !valid_raw_short_string(frame.event_type)
            || looks_like_hex_pointer(frame.event_type)
            || !valid_raw_short_string(frame.subject)
            || looks_like_hex_pointer(frame.subject)
            || !valid_raw_after_json(frame.detail_json)
        {
            return None;
        }

        let mut fields = vec![
            "\"type\":\"event\"".to_string(),
            "\"backend\":\"in_process_collector\"".to_string(),
            format!("\"collector_version\":\"{}\"", super::COLLECTOR_VERSION),
            format!("\"abi_version\":{}", super::ABI_VERSION),
            format!("\"process_id\":{}", frame.pid),
            format!("\"tick_ms\":{}", frame.tick_ms),
            format!("\"sequence\":{}", frame.sequence),
            format!("\"profile_id\":\"{}\"", super::COLLECTOR_PROFILE_ID),
            format!("\"date\":{}", json_str(frame.date)),
            format!("\"hour_index\":{}", frame.hour_index),
            format!("\"sample_ms\":{}", frame.sample_ms),
            format!("\"capture_boundary\":{}", json_str(frame.capture_boundary)),
            format!("\"domain\":{}", json_str(frame.domain)),
            format!("\"entity_key\":{}", json_str(frame.entity_key)),
            format!("\"source\":\"{}\"", RAW_SOURCE),
            format!("\"confidence\":{}", json_str(frame.confidence)),
        ];
        if let Some(session_sequence) = frame.session_sequence {
            fields.push(format!("\"session_sequence\":{session_sequence}"));
        }
        if let Some(duration_ms) = frame.duration_ms {
            fields.push(format!("\"duration_ms\":{duration_ms}"));
        }
        if let Some(elapsed_wall_ms) = frame.elapsed_wall_ms {
            fields.push(format!("\"elapsed_wall_ms\":{elapsed_wall_ms}"));
        }
        fields.push(format!(
            "\"payload\":{{\"event_type\":{},\"key\":{},\"subject\":{},\"detail\":{}}}",
            json_str(frame.event_type),
            json_str(frame.entity_key),
            json_str(frame.subject),
            frame.detail_json
        ));
        Some(format!("{{{}}}\n", fields.join(",")))
    }

    fn raw_domain_delta_frame_line(frame: RawDomainDeltaFrame<'_>) -> Option<String> {
        if !valid_raw_short_string(frame.date)
            || !valid_raw_short_string(frame.capture_boundary)
            || !valid_raw_domain(frame.domain)
            || !valid_raw_key(frame.entity_key)
            || !valid_raw_confidence(frame.confidence)
            || !valid_raw_delta_op(frame.op)
            || !valid_raw_after_json(frame.after_json)
            || frame.changed_fields.len() > 256
            || frame
                .changed_fields
                .iter()
                .any(|field| !valid_raw_short_string(field))
        {
            return None;
        }

        let changed_fields = frame
            .changed_fields
            .iter()
            .map(|field| json_str(field))
            .collect::<Vec<_>>()
            .join(",");
        let mut fields = vec![
            "\"type\":\"domain_delta\"".to_string(),
            "\"backend\":\"in_process_collector\"".to_string(),
            format!("\"collector_version\":\"{}\"", super::COLLECTOR_VERSION),
            format!("\"abi_version\":{}", super::ABI_VERSION),
            format!("\"process_id\":{}", frame.pid),
            format!("\"tick_ms\":{}", frame.tick_ms),
            format!("\"sequence\":{}", frame.sequence),
            format!("\"profile_id\":\"{}\"", super::COLLECTOR_PROFILE_ID),
            format!("\"date\":{}", json_str(frame.date)),
            format!("\"hour_index\":{}", frame.hour_index),
            format!("\"sample_ms\":{}", frame.sample_ms),
            format!("\"capture_boundary\":{}", json_str(frame.capture_boundary)),
            format!("\"domain\":{}", json_str(frame.domain)),
            format!("\"entity_key\":{}", json_str(frame.entity_key)),
            format!("\"source\":\"{}\"", RAW_SOURCE),
            format!("\"confidence\":{}", json_str(frame.confidence)),
        ];
        if let Some(session_sequence) = frame.session_sequence {
            fields.push(format!("\"session_sequence\":{session_sequence}"));
        }
        if let Some(duration_ms) = frame.duration_ms {
            fields.push(format!("\"duration_ms\":{duration_ms}"));
        }
        if let Some(elapsed_wall_ms) = frame.elapsed_wall_ms {
            fields.push(format!("\"elapsed_wall_ms\":{elapsed_wall_ms}"));
        }
        fields.push(format!(
            "\"payload\":{{\"op\":{},\"after\":{},\"changed_fields\":[{}]}}",
            json_str(frame.op),
            frame.after_json,
            changed_fields
        ));
        Some(format!("{{{}}}\n", fields.join(",")))
    }

    fn valid_raw_short_string(value: &str) -> bool {
        !value.is_empty() && value.len() <= RAW_SHORT_STRING_MAX_LENGTH
    }

    fn valid_raw_key(value: &str) -> bool {
        !value.is_empty() && value.len() <= RAW_STRING_MAX_LENGTH && !looks_like_hex_pointer(value)
    }

    fn valid_raw_domain(value: &str) -> bool {
        matches!(
            value,
            RAW_CLOCK_DOMAIN
                | RAW_DIVISIONS_DOMAIN
                | RAW_LAND_COMBATS_DOMAIN
                | RAW_ARMY_ORDERS_DOMAIN
                | RAW_NAVAL_COMBATS_DOMAIN
                | RAW_NAVAL_COMBAT_MEMBERS_DOMAIN
                | RAW_AIR_WINGS_DOMAIN
        )
    }

    fn valid_raw_confidence(value: &str) -> bool {
        matches!(value, "exact" | "sampled" | "proxy" | "gap")
    }

    fn valid_raw_delta_op(value: &str) -> bool {
        matches!(value, "upsert" | "delete" | "sample")
    }

    fn looks_like_hex_pointer(value: &str) -> bool {
        let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        else {
            return false;
        };
        !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
    }

    fn valid_raw_after_json(value: &str) -> bool {
        let trimmed = value.trim();
        trimmed == "null"
            || (!trimmed.is_empty()
                && trimmed.len() <= RAW_JSON_OBJECT_MAX_LENGTH
                && trimmed.starts_with('{')
                && trimmed.ends_with('}'))
    }

    fn json_str(value: &str) -> String {
        let mut out = String::with_capacity(value.len() + 2);
        out.push('"');
        for ch in value.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                ch if ch.is_control() => out.push_str(&format!("\\u{:04X}", ch as u32)),
                ch => out.push(ch),
            }
        }
        out.push('"');
        out
    }

    fn control_metadata_json(
        control: Option<&CollectorControl>,
        session_sequence: Option<u64>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: Option<&str>,
    ) -> String {
        let Some(control) = control else {
            return String::new();
        };
        let capture_boundary = capture_boundary.unwrap_or("worker_poll");
        let session = control
            .session_id
            .as_deref()
            .map(|id| format!("\"session_id\":\"{id}\","))
            .unwrap_or_default();
        let session_sequence = session_sequence
            .map(|n| format!("\"session_sequence\":{n},"))
            .unwrap_or_default();
        let elapsed_wall_ms = elapsed_wall_ms
            .map(|n| format!("\"elapsed_wall_ms\":{n},"))
            .unwrap_or_default();
        let division_body_bridge_valid = control.division_body_bridge_valid
            && division_body_bridge_well_formed(&control.division_body_bridge);
        let army_body_bridge_valid = control.army_body_bridge_valid
            && army_body_bridge_well_formed(&control.army_body_bridge);
        let naval_combat_body_bridge_valid = control.naval_combat_body_bridge_valid
            && naval_combat_body_bridge_well_formed(&control.naval_combat_body_bridge);
        let air_wing_body_bridge_valid = control.air_wing_body_bridge_valid
            && air_wing_body_bridge_well_formed(&control.air_wing_body_bridge);
        let map_control_state_roots_valid =
            control.map_control.state_roots_valid && control.map_control.state_roots_well_formed();
        let map_control_province_bridge_valid = control.map_control.province_bridge_valid
            && control.map_control.province_bridge_well_formed();
        let map_control_controller_bridge_valid = control.map_control.controller_bridge_valid
            && control.map_control.controller_bridge_well_formed();
        format!(
            "{session}{session_sequence}{elapsed_wall_ms}\"capture_boundary\":\"{capture_boundary}\",\
             \"duration_ms\":{},\"sample_ms\":{},\
             \"special_projects_scan\":{},\
             \"scheduled_snapshot_step_hours\":{},\
             \"country_object_bridge_rows\":{},\
             \"country_object_bridge_valid\":{},\
             \"country_record_bridge_rows\":{},\
             \"country_record_bridge_valid\":{},\
             \"division_body_bridge_rows\":{},\
             \"division_body_bridge_valid\":{},\
             \"army_body_bridge_rows\":{},\
             \"army_body_bridge_valid\":{},\
             \"naval_combat_body_bridge_rows\":{},\
             \"naval_combat_body_bridge_valid\":{},\
             \"air_wing_body_bridge_rows\":{},\
             \"air_wing_body_bridge_valid\":{},\
             \"advisor_token_filter_tokens\":{},\
             \"advisor_token_filter_valid\":{},\
             \"idea_group_filter_rows\":{},\
             \"idea_group_filter_valid\":{},\
             \"map_control_state_root_rows\":{},\
             \"map_control_state_roots_valid\":{},\
             \"map_control_province_bridge_present\":{},\
             \"map_control_province_bridge_valid\":{},\
             \"map_control_controller_bridge_rows\":{},\
             \"map_control_controller_bridge_valid\":{},",
            control.duration_ms,
            control.sample_ms,
            control.special_projects_scan,
            control.scheduled_snapshot_step_hours,
            control.country_object_bridge.len(),
            control.country_object_bridge_valid,
            control.country_record_bridge.len(),
            control.country_record_bridge_valid,
            control.division_body_bridge.len(),
            division_body_bridge_valid,
            control.army_body_bridge.len(),
            army_body_bridge_valid,
            control.naval_combat_body_bridge.len(),
            naval_combat_body_bridge_valid,
            control.air_wing_body_bridge.len(),
            air_wing_body_bridge_valid,
            control.advisor_token_filter.len(),
            control.advisor_token_filter_valid,
            control.idea_group_filter.len(),
            control.idea_group_filter_valid,
            control.map_control.state_roots.len(),
            map_control_state_roots_valid,
            control.map_control.province_bridge.is_some(),
            map_control_province_bridge_valid,
            control.map_control.controller_bridge.len(),
            map_control_controller_bridge_valid
        )
    }

    fn capture_boundary_for_frame(
        state: Option<&CollectorState>,
        control: &CollectorControl,
        last_scheduled_game_hour: &mut Option<i32>,
    ) -> &'static str {
        let Some(current_raw) = current_game_date_raw(state) else {
            return "worker_poll";
        };
        let step = control.scheduled_snapshot_step_hours.max(1) as i32;
        let should_schedule = match *last_scheduled_game_hour {
            None => true,
            Some(previous_raw) => {
                current_raw != previous_raw && current_raw.saturating_sub(previous_raw) >= step
            }
        };
        if should_schedule {
            *last_scheduled_game_hour = Some(current_raw);
            "scheduled_hour_snapshot"
        } else {
            "worker_poll"
        }
    }

    fn current_game_date_raw(state: Option<&CollectorState>) -> Option<i32> {
        let state = state?;
        if !(state.daily_match && state.long_string_match) {
            return None;
        }
        read_i32(state.date_ptr as usize + GS_CURRENT_DATE_OFFSET)
    }

    fn duration_millis_u64(duration: Duration) -> u64 {
        duration.as_millis().min(u128::from(u64::MAX)) as u64
    }

    struct CollectorState {
        base: usize,
        daily_addr: usize,
        long_string_addr: usize,
        daily_match: bool,
        long_string_match: bool,
        date_ptr: u64,
        country_database_ptr: u64,
        country_identity: CountryIdentityCache,
        special_projects: SpecialProjectsCache,
    }

    impl CollectorState {
        fn resolve(control: Option<&CollectorControl>) -> CollectorState {
            let base = unsafe { GetModuleHandleW(ptr::null()) as usize };
            let daily_addr = base.saturating_add(C_GAME_STATE_DAILY_UPDATE_OFFSET);
            let long_string_addr = base.saturating_add(GET_LONG_STRING_OFFSET);
            let daily_match = base != 0 && sentinel_matches(daily_addr, &DAILY_UPDATE_SENTINEL);
            let long_string_match =
                base != 0 && sentinel_matches(long_string_addr, &GET_LONG_STRING_SENTINEL);
            let date_ptr = if daily_match && long_string_match {
                read_u64(base + CG_STATE_DATE_OFFSET).unwrap_or(0)
            } else {
                0
            };
            let country_database_ptr = if daily_match && long_string_match {
                read_u64(base + COUNTRY_DATABASE_OFFSET).unwrap_or(0)
            } else {
                0
            };
            let country_identity = if daily_match
                && long_string_match
                && control
                    .map(|control| {
                        !control.country_object_bridge_valid || !control.country_record_bridge_valid
                    })
                    .unwrap_or(false)
            {
                CountryIdentityCache::not_attempted("invalid_control_bridge")
            } else if daily_match
                && long_string_match
                && control
                    .map(|control| !control.country_object_bridge.is_empty())
                    .unwrap_or(false)
            {
                let bridge = control
                    .map(|control| control.country_object_bridge.as_slice())
                    .unwrap_or(&[]);
                let record_bridge = control
                    .map(|control| control.country_record_bridge.as_slice())
                    .unwrap_or(&[]);
                CountryIdentityCache::from_control_bridge(
                    base,
                    country_database_ptr as usize,
                    bridge,
                    record_bridge,
                )
            } else if daily_match
                && long_string_match
                && control
                    .map(|control| control.country_object_scan)
                    .unwrap_or(false)
            {
                CountryIdentityCache::resolve(base, country_database_ptr as usize)
            } else if daily_match && long_string_match {
                let gamestate_rows = CountryIdentityCache::from_gamestate_country_table(
                    base,
                    date_ptr as usize,
                    country_database_ptr as usize,
                );
                if gamestate_rows.has_scalar_source() {
                    gamestate_rows
                } else {
                    let database_rows = CountryIdentityCache::from_country_database_root(
                        base,
                        country_database_ptr as usize,
                    );
                    if database_rows.has_scalar_source() {
                        database_rows
                    } else {
                        gamestate_rows
                    }
                }
            } else {
                CountryIdentityCache::not_attempted("profile_mismatch")
            };
            CollectorState {
                base,
                daily_addr,
                long_string_addr,
                daily_match,
                long_string_match,
                date_ptr,
                country_database_ptr,
                country_identity,
                special_projects: if daily_match
                    && long_string_match
                    && control
                        .map(|control| control.special_projects_scan)
                        .unwrap_or(false)
                {
                    SpecialProjectsCache::resolve(base)
                } else {
                    SpecialProjectsCache::not_attempted("disabled")
                },
            }
        }
    }

    #[derive(Clone, Copy)]
    struct ProjectFacilityPin {
        addr: usize,
        facility_id: u32,
        province_id: u32,
    }

    #[derive(Clone, Copy)]
    struct ProjectPin {
        addr: usize,
        project_id: u32,
        static_def_handle: u32,
    }

    #[derive(Clone, Copy)]
    struct ProjectScientistPin {
        addr: usize,
        char_id: u32,
    }

    #[derive(Clone)]
    struct SpecialProjectsCache {
        status: &'static str,
        scan_ms: u64,
        facilities: Vec<ProjectFacilityPin>,
        projects: Vec<ProjectPin>,
        scientists: Vec<ProjectScientistPin>,
        too_many: bool,
    }

    #[derive(Clone)]
    struct ProjectFacilityRow {
        id: u32,
        province: u32,
        building: Option<String>,
        active_project_handle: u32,
        active: bool,
        scientist_char_id: Option<u32>,
        unread_prototype_rewards: u32,
    }

    #[derive(Clone)]
    struct ProjectRow {
        id: u32,
        static_def_handle: u32,
        iterations: u32,
        worked_at_facility: Option<u32>,
    }

    #[derive(Clone)]
    struct ProjectScientistRow {
        char_id: u32,
        name_token: Option<String>,
        display_name: Option<String>,
        skills: BTreeMap<&'static str, i32>,
        trait_count: Option<u32>,
    }

    #[derive(Clone)]
    struct CountryObjectRow {
        tag: String,
        tag_db_id: u32,
        name: String,
        object_ptr: usize,
        record_ptr: Option<usize>,
        original_capital: Option<i32>,
        original_tag: Option<String>,
        templates_locked: Option<bool>,
        reserved_dynamic_country: Option<bool>,
        stability: Option<f64>,
        war_support: Option<f64>,
        political_power: Option<f64>,
        command_power: Option<f64>,
        num_ships: Option<i32>,
        manpower_pool: Option<u32>,
        convoys: Option<f64>,
        misc: Option<CountryMiscRow>,
        fuel: Option<CountryFuelRow>,
        production: Option<CountryProductionRow>,
        focus: Option<CountryFocusRow>,
        research: Option<CountryResearchRow>,
        politics: Option<CountryPoliticsRow>,
        politics_residuals: Option<CountryPoliticsResidualsRow>,
        decisions: Option<CountryDecisionsRow>,
    }

    #[derive(Clone)]
    struct CountryMiscRow {
        convoys_destroyed: Option<u32>,
        accidents_score: Option<f64>,
        ace_count: Option<u32>,
        cosmetic_tag: Option<Option<String>>,
    }

    impl CountryMiscRow {
        fn has_any(&self) -> bool {
            self.convoys_destroyed.is_some()
                || self.accidents_score.is_some()
                || self.ace_count.is_some()
                || self.cosmetic_tag.is_some()
        }
    }

    #[derive(Clone, Copy)]
    struct CountryFuelRow {
        current: f64,
        max: f64,
        gain: f64,
        cost: f64,
    }

    #[derive(Clone)]
    struct CountryProductionRow {
        stockpile: Option<Vec<CountryStockpileRow>>,
        military_lines: Option<Vec<CountryProductionLineRow>>,
        naval_lines: Option<Vec<CountryProductionLineRow>>,
    }

    #[derive(Clone, Copy)]
    struct CountryStockpileRow {
        variant_id: u32,
        amount: f64,
    }

    #[derive(Clone, Copy)]
    struct CountryProductionLineRow {
        id: i32,
        active_factories: i32,
        cost: f64,
        speed: f64,
        produced: f64,
        amount: i32,
    }

    #[derive(Clone)]
    struct CountryFocusRow {
        focus_tree: Option<String>,
        completed: Option<Vec<String>>,
        completed_count: Option<u32>,
        current: Option<String>,
        current_continuous: Option<String>,
        progress: Option<f64>,
    }

    impl CountryFocusRow {
        fn has_any(&self) -> bool {
            self.focus_tree.is_some()
                || self.completed.is_some()
                || self.completed_count.is_some()
                || self.current.is_some()
                || self.current_continuous.is_some()
                || self.progress.is_some()
        }
    }

    #[derive(Clone)]
    struct CountryResearchRow {
        completed_tech_count: Option<u32>,
        research_slot_count: Option<u32>,
        active_research: Option<Vec<CountryResearchSlotRow>>,
    }

    impl CountryResearchRow {
        fn has_any(&self) -> bool {
            self.completed_tech_count.is_some()
                || self.research_slot_count.is_some()
                || self.active_research.is_some()
        }
    }

    #[derive(Clone, Copy)]
    struct CountryResearchSlotRow {
        slot_index: u32,
        tech_handle_raw: u32,
        tech_def_index_raw: u32,
        progress_raw: u32,
    }

    #[derive(Clone)]
    struct CountryPoliticsRow {
        popularities: Option<BTreeMap<String, f64>>,
        ruling_party: Option<String>,
        ideas: Option<Vec<String>>,
        laws: Option<BTreeMap<String, String>>,
        ideas_other: Option<Vec<String>>,
        national_spirits: Option<Vec<String>>,
        idea_count: Option<u32>,
        election_frequency: Option<u32>,
        party_count: Option<u32>,
        low_confidence: Option<bool>,
    }

    impl CountryPoliticsRow {
        fn has_any(&self) -> bool {
            self.popularities.is_some()
                || self.ruling_party.is_some()
                || self.ideas.is_some()
                || self.laws.is_some()
                || self.ideas_other.is_some()
                || self.national_spirits.is_some()
                || self.idea_count.is_some()
                || self.election_frequency.is_some()
                || self.party_count.is_some()
                || self.low_confidence.is_some()
        }
    }

    #[derive(Clone)]
    struct CountryPoliticsResidualsRow {
        last_election: Option<String>,
        elections_allowed: Option<bool>,
        appointed_advisors: Option<Vec<String>>,
        appointed_advisor_details: Option<Vec<AppointedAdvisorRow>>,
        appointed_advisor_slots: Option<BTreeMap<String, Vec<String>>>,
    }

    impl CountryPoliticsResidualsRow {
        fn has_any(&self) -> bool {
            self.last_election.is_some()
                || self.elections_allowed.is_some()
                || self.appointed_advisors.is_some()
                || self.appointed_advisor_details.is_some()
                || self.appointed_advisor_slots.is_some()
        }
    }

    #[derive(Clone)]
    struct AppointedAdvisorRow {
        idea_token: String,
        slot: Option<String>,
        ledger: Option<String>,
        character: Option<String>,
    }

    #[derive(Clone)]
    struct CountryDecisionsRow {
        status_present: Option<BTreeMap<String, u32>>,
        timed_live: Option<Vec<TimedDecisionRow>>,
    }

    impl CountryDecisionsRow {
        fn has_any(&self) -> bool {
            self.status_present.is_some() || self.timed_live.is_some()
        }
    }

    #[derive(Clone)]
    struct TimedDecisionRow {
        decision: String,
        days: i32,
        state: i32,
        state_name: String,
    }

    struct CountryPartyRead {
        popularities: BTreeMap<String, f64>,
        party_ptrs: Vec<usize>,
        party_count: u32,
    }

    struct CountryIdentityCache {
        status: &'static str,
        scan_ms: u64,
        country_vtable: usize,
        bridge_rows_supplied: u32,
        record_rows_supplied: u32,
        rows: Vec<CountryObjectRow>,
        rejected_objects: u32,
        duplicate_tags: u32,
    }

    impl CountryIdentityCache {
        fn not_attempted(status: &'static str) -> CountryIdentityCache {
            CountryIdentityCache {
                status,
                scan_ms: 0,
                country_vtable: 0,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows: Vec::new(),
                rejected_objects: 0,
                duplicate_tags: 0,
            }
        }

        fn resolve(base: usize, country_database_ptr: usize) -> CountryIdentityCache {
            let started = unsafe { GetTickCount64() };
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let country_vtable = base + COUNTRY_VTABLE_RVA;
            let mut rows_by_tag: BTreeMap<String, CountryObjectRow> = BTreeMap::new();
            let mut rejected_objects = 0u32;
            let mut duplicate_tags = 0u32;
            for object_ptr in scan_qword_hits(country_vtable, HEAP_LO, HEAP_HI) {
                let Some(row) = country_object_row(
                    object_ptr,
                    country_vtable,
                    tag_table,
                    None,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    None,
                    None,
                ) else {
                    rejected_objects = rejected_objects.saturating_add(1);
                    continue;
                };
                if rows_by_tag.insert(row.tag.clone(), row).is_some() {
                    duplicate_tags = duplicate_tags.saturating_add(1);
                }
            }
            let scan_ms = unsafe { GetTickCount64() }.saturating_sub(started);
            let rows: Vec<CountryObjectRow> = rows_by_tag.into_values().collect();
            let status = if rows.len() >= MIN_USABLE_COUNTRY_OBJECT_ROWS {
                "verified"
            } else {
                "insufficient_country_objects"
            };
            CountryIdentityCache {
                status,
                scan_ms,
                country_vtable,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows,
                rejected_objects,
                duplicate_tags,
            }
        }

        fn from_control_bridge(
            base: usize,
            country_database_ptr: usize,
            bridge: &[CountryObjectBridgeEntry],
            record_bridge: &[CountryObjectBridgeEntry],
        ) -> CountryIdentityCache {
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let country_vtable = base + COUNTRY_VTABLE_RVA;
            let mut records_by_tag_id: BTreeMap<u32, usize> = BTreeMap::new();
            for entry in record_bridge.iter().take(MAX_COUNTRY_OBJECT_BRIDGE_ROWS) {
                records_by_tag_id.insert(entry.tag_db_id, entry.object_ptr);
            }
            let mut rows_by_tag: BTreeMap<String, CountryObjectRow> = BTreeMap::new();
            let mut rejected_objects = 0u32;
            let mut duplicate_tags = 0u32;
            for entry in bridge.iter().take(MAX_COUNTRY_OBJECT_BRIDGE_ROWS) {
                let Some(row) = country_object_row_expected(
                    entry.object_ptr,
                    country_vtable,
                    tag_table,
                    entry.tag_db_id,
                    records_by_tag_id.get(&entry.tag_db_id).copied(),
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    None,
                    None,
                ) else {
                    rejected_objects = rejected_objects.saturating_add(1);
                    continue;
                };
                if rows_by_tag.insert(row.tag.clone(), row).is_some() {
                    duplicate_tags = duplicate_tags.saturating_add(1);
                }
            }
            let rows: Vec<CountryObjectRow> = rows_by_tag.into_values().collect();
            let status = if rows.len() >= MIN_USABLE_COUNTRY_OBJECT_ROWS {
                "control_bridge_verified"
            } else {
                "insufficient_control_bridge"
            };
            CountryIdentityCache {
                status,
                scan_ms: 0,
                country_vtable,
                bridge_rows_supplied: bridge.len().min(u32::MAX as usize) as u32,
                record_rows_supplied: record_bridge.len().min(u32::MAX as usize) as u32,
                rows,
                rejected_objects,
                duplicate_tags,
            }
        }

        fn from_country_database_root(
            base: usize,
            country_database_ptr: usize,
        ) -> CountryIdentityCache {
            let started = unsafe { GetTickCount64() };
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let Some(tag_row_count) = country_tag_row_count(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let Some(country_vtable) = base.checked_add(COUNTRY_VTABLE_RVA) else {
                return CountryIdentityCache::not_attempted("profile_mismatch");
            };

            let mut seen_bases: BTreeMap<usize, ()> = BTreeMap::new();
            let mut candidates: BTreeMap<usize, Vec<CountryObjectRow>> = BTreeMap::new();
            for off in (0..COUNTRY_DATABASE_OBJECT_ARRAY_SEARCH_SPAN).step_by(8) {
                let Some(field_addr) = country_database_ptr.checked_add(off) else {
                    break;
                };
                for array_base in country_object_array_base_candidates(field_addr) {
                    if seen_bases.insert(array_base, ()).is_some() {
                        continue;
                    }
                    if let Some(rows) = country_rows_from_tag_indexed_array(
                        array_base,
                        country_vtable,
                        tag_table,
                        tag_row_count,
                    ) {
                        candidates.insert(array_base, rows);
                    }
                }
            }

            let scan_ms = unsafe { GetTickCount64() }.saturating_sub(started);
            if candidates.is_empty() {
                return CountryIdentityCache {
                    status: "country_database_root_insufficient",
                    scan_ms,
                    country_vtable,
                    bridge_rows_supplied: 0,
                    record_rows_supplied: 0,
                    rows: Vec::new(),
                    rejected_objects: 0,
                    duplicate_tags: 0,
                };
            }

            let bases: Vec<usize> = candidates.keys().copied().collect();
            let ref_map = country_tag_object_map(
                candidates
                    .get(&bases[0])
                    .expect("non-empty candidate map has first candidate"),
            );
            for base in bases.iter().skip(1) {
                let other_map = country_tag_object_map(
                    candidates
                        .get(base)
                        .expect("candidate key came from candidate map"),
                );
                if !country_tag_object_maps_agree(&ref_map, &other_map) {
                    return CountryIdentityCache {
                        status: "country_database_root_ambiguous",
                        scan_ms,
                        country_vtable,
                        bridge_rows_supplied: 0,
                        record_rows_supplied: 0,
                        rows: Vec::new(),
                        rejected_objects: 0,
                        duplicate_tags: 0,
                    };
                }
            }

            let rows = candidates.remove(&bases[0]).unwrap_or_default();
            CountryIdentityCache {
                status: "country_database_root_verified",
                scan_ms,
                country_vtable,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows,
                rejected_objects: 0,
                duplicate_tags: 0,
            }
        }

        fn from_gamestate_country_table(
            base: usize,
            gamestate_ptr: usize,
            country_database_ptr: usize,
        ) -> CountryIdentityCache {
            let started = unsafe { GetTickCount64() };
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let Some(tag_row_count) = country_tag_row_count(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let Some(country_vtable) = base.checked_add(COUNTRY_VTABLE_RVA) else {
                return CountryIdentityCache::not_attempted("profile_mismatch");
            };
            let Some(country_table) =
                read_u64(gamestate_ptr.saturating_add(CGAMESTATE_COUNTRY_TABLE))
                    .map(|value| value as usize)
            else {
                return CountryIdentityCache::not_attempted("gamestate_country_table_unavailable");
            };
            if !is_map_control_heap_pointer(country_table) {
                return CountryIdentityCache::not_attempted("gamestate_country_table_unavailable");
            }

            let mut rows = Vec::new();
            let mut seen_objects: BTreeMap<usize, ()> = BTreeMap::new();
            let mut rejected_objects = 0u32;
            let mut duplicate_tags = 0u32;
            for tag_db_id in 1..tag_row_count {
                let Some(slot_addr) = country_table.checked_add(tag_db_id as usize * 8) else {
                    break;
                };
                let object_ptr = read_u64(slot_addr).unwrap_or(0) as usize;
                if object_ptr == 0 {
                    continue;
                }
                if !is_map_control_heap_pointer(object_ptr) {
                    rejected_objects = rejected_objects.saturating_add(1);
                    continue;
                }
                let Some(row) = country_object_row_expected(
                    object_ptr,
                    country_vtable,
                    tag_table,
                    tag_db_id,
                    None,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    None,
                    None,
                ) else {
                    rejected_objects = rejected_objects.saturating_add(1);
                    continue;
                };
                if seen_objects.insert(row.object_ptr, ()).is_some() {
                    duplicate_tags = duplicate_tags.saturating_add(1);
                    continue;
                }
                rows.push(row);
            }

            let scan_ms = unsafe { GetTickCount64() }.saturating_sub(started);
            let status = if rows.len() >= MIN_USABLE_COUNTRY_OBJECT_ROWS {
                "gamestate_country_table_verified"
            } else {
                "gamestate_country_table_insufficient"
            };
            CountryIdentityCache {
                status,
                scan_ms,
                country_vtable,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows,
                rejected_objects,
                duplicate_tags,
            }
        }

        fn has_scalar_source(&self) -> bool {
            (self.status == "verified"
                || self.status == "control_bridge_verified"
                || self.status == "country_database_root_verified"
                || self.status == "gamestate_country_table_verified")
                && self.country_vtable != 0
                && !self.rows.is_empty()
        }

        fn current_rows(
            &self,
            country_database_ptr: usize,
            include_scheduled_country_fuel: bool,
            include_scheduled_country_production: bool,
            include_scheduled_country_focus: bool,
            include_scheduled_country_research: bool,
            include_scheduled_country_politics: bool,
            include_scheduled_country_politics_residuals: bool,
            include_scheduled_country_decisions: bool,
            advisor_token_filter: Option<&BTreeMap<String, AdvisorTokenInfo>>,
            idea_group_filter: Option<&BTreeMap<String, String>>,
        ) -> Vec<CountryObjectRow> {
            if !self.has_scalar_source() {
                return Vec::new();
            }
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return Vec::new();
            };
            let mut rows = Vec::new();
            for row in &self.rows {
                if let Some(fresh) = revalidated_country_object_row(
                    row,
                    self.country_vtable,
                    tag_table,
                    include_scheduled_country_fuel,
                    include_scheduled_country_production,
                    include_scheduled_country_focus,
                    include_scheduled_country_research,
                    include_scheduled_country_politics,
                    include_scheduled_country_politics_residuals,
                    include_scheduled_country_decisions,
                    advisor_token_filter,
                    idea_group_filter,
                ) {
                    rows.push(fresh);
                }
            }
            rows
        }

        fn domains_json_from_rows(
            &self,
            current_rows: &[CountryObjectRow],
            has_state_rows: bool,
            has_province_rows: bool,
            has_special_projects: bool,
            include_scheduled_country_fuel: bool,
            include_scheduled_country_production: bool,
            include_scheduled_country_focus: bool,
            include_scheduled_country_research: bool,
            include_scheduled_country_politics: bool,
            include_scheduled_country_politics_residuals: bool,
            include_scheduled_country_decisions: bool,
        ) -> String {
            let mut domains = vec!["\"clock\"", "\"country_identity\""];
            if !current_rows.is_empty() {
                domains.push("\"country_scalars\"");
            }
            if current_rows.iter().any(|row| row.misc.is_some()) {
                domains.push("\"country_misc\"");
            }
            if include_scheduled_country_fuel && current_rows.iter().any(|row| row.fuel.is_some()) {
                domains.push("\"fuel_live\"");
            }
            if include_scheduled_country_production
                && current_rows.iter().any(|row| row.production.is_some())
            {
                domains.push("\"production_live\"");
            }
            if include_scheduled_country_focus && current_rows.iter().any(|row| row.focus.is_some())
            {
                domains.push("\"focus_live\"");
            }
            if include_scheduled_country_research
                && current_rows.iter().any(|row| row.research.is_some())
            {
                domains.push("\"tech_live\"");
            }
            if include_scheduled_country_politics
                && current_rows.iter().any(|row| row.politics.is_some())
            {
                domains.push("\"politics_live\"");
            }
            if include_scheduled_country_politics_residuals
                && current_rows
                    .iter()
                    .any(|row| row.politics_residuals.is_some())
            {
                domains.push("\"politics_residuals\"");
            }
            if include_scheduled_country_decisions
                && current_rows.iter().any(|row| row.decisions.is_some())
            {
                domains.push("\"decisions_live\"");
            }
            if has_state_rows {
                domains.push("\"state_ownership\"");
                domains.push("\"state_control\"");
            }
            if has_province_rows {
                domains.push("\"province_control\"");
            }
            if has_special_projects {
                domains.push("\"projects_live\"");
            }
            format!("[{}]", domains.join(","))
        }

        fn root_json(&self) -> String {
            format!(
                "\"country_object_identity\":{{\"status\":\"{}\",\
                 \"scan_ms\":{},\"bridge_rows_supplied\":{},\
                 \"record_rows_supplied\":{},\
                 \"object_count\":{},\"rejected_objects\":{},\
                 \"duplicate_tags\":{}}}",
                self.status,
                self.scan_ms,
                self.bridge_rows_supplied,
                self.record_rows_supplied,
                self.rows.len(),
                self.rejected_objects,
                self.duplicate_tags
            )
        }

        fn countries_json_from_rows(
            &self,
            country_database_ptr: usize,
            current_rows: &[CountryObjectRow],
            include_scheduled_country_fuel: bool,
            include_scheduled_country_production: bool,
            include_scheduled_country_focus: bool,
            include_scheduled_country_research: bool,
            include_scheduled_country_politics: bool,
            include_scheduled_country_politics_residuals: bool,
            include_scheduled_country_decisions: bool,
        ) -> String {
            if !current_rows.is_empty() {
                let rows: Vec<String> = current_rows
                    .iter()
                    .map(|row| {
                        row_json(
                            row,
                            include_scheduled_country_fuel,
                            include_scheduled_country_production,
                            include_scheduled_country_focus,
                            include_scheduled_country_research,
                            include_scheduled_country_politics,
                            include_scheduled_country_politics_residuals,
                            include_scheduled_country_decisions,
                        )
                    })
                    .collect();
                return format!("{{{}}}", rows.join(","));
            }
            country_rows_from_tag_table_json(country_database_ptr)
        }
    }

    impl SpecialProjectsCache {
        fn not_attempted(status: &'static str) -> Self {
            Self {
                status,
                scan_ms: 0,
                facilities: Vec::new(),
                projects: Vec::new(),
                scientists: Vec::new(),
                too_many: false,
            }
        }

        fn resolve(base: usize) -> Self {
            if base == 0 {
                return Self::not_attempted("profile_mismatch");
            }
            let started = unsafe { GetTickCount64() };
            let hits = scan_special_project_pool_hits(base);
            let scan_ms = unsafe { GetTickCount64() }.saturating_sub(started);
            if hits.too_many {
                return Self {
                    status: "too_many",
                    scan_ms,
                    facilities: Vec::new(),
                    projects: Vec::new(),
                    scientists: Vec::new(),
                    too_many: true,
                };
            }
            let facility_vtable = base.saturating_add(PROJECT_FACILITY_VTABLE_RVA);
            let project_vtable = base.saturating_add(PROJECT_CPROJECT_VTABLE_RVA);
            let character_vtable = base.saturating_add(PROJECT_CHARACTER_VTABLE_RVA);
            let scientist_vtable = base.saturating_add(PROJECT_SCIENTIST_VTABLE_RVA);
            let mut facilities = Vec::new();
            for addr in hits.facilities {
                if let Some(pin) = project_facility_pin(addr, facility_vtable) {
                    facilities.push(pin);
                }
            }
            let mut projects = Vec::new();
            for addr in hits.projects {
                if let Some(pin) = project_pin(addr, project_vtable) {
                    projects.push(pin);
                }
            }
            let mut scientists = Vec::new();
            for addr in hits.scientists {
                if let Some(pin) = project_scientist_pin(addr, character_vtable, scientist_vtable) {
                    scientists.push(pin);
                }
            }
            let status = if facilities.is_empty() && projects.is_empty() && scientists.is_empty() {
                "not_found"
            } else {
                "verified"
            };
            Self {
                status,
                scan_ms,
                facilities,
                projects,
                scientists,
                too_many: false,
            }
        }

        fn rows(
            &self,
            base: usize,
        ) -> (
            Vec<ProjectFacilityRow>,
            Vec<ProjectRow>,
            Vec<ProjectScientistRow>,
        ) {
            if self.status != "verified" || self.too_many {
                return (Vec::new(), Vec::new(), Vec::new());
            }
            let facility_vtable = base.saturating_add(PROJECT_FACILITY_VTABLE_RVA);
            let project_vtable = base.saturating_add(PROJECT_CPROJECT_VTABLE_RVA);
            let character_vtable = base.saturating_add(PROJECT_CHARACTER_VTABLE_RVA);
            let scientist_vtable = base.saturating_add(PROJECT_SCIENTIST_VTABLE_RVA);

            let mut facilities = Vec::new();
            for pin in &self.facilities {
                if let Some(row) = read_project_facility_row(*pin, facility_vtable) {
                    facilities.push(row);
                    if facilities.len() >= MAX_PROJECT_FACILITIES {
                        break;
                    }
                }
            }
            let mut projects = Vec::new();
            for pin in &self.projects {
                if let Some(row) = read_project_row(*pin, project_vtable) {
                    projects.push(row);
                    if projects.len() >= MAX_PROJECT_ACTIVE_PROJECTS {
                        break;
                    }
                }
            }
            let mut scientists = Vec::new();
            for pin in &self.scientists {
                if let Some(row) =
                    read_project_scientist_row(*pin, character_vtable, scientist_vtable)
                {
                    scientists.push(row);
                    if scientists.len() >= MAX_PROJECT_SCIENTISTS {
                        break;
                    }
                }
            }
            (facilities, projects, scientists)
        }

        fn root_json(&self) -> String {
            format!(
                "\"special_projects_identity\":{{\"status\":\"{}\",\
                 \"scan_ms\":{},\"facility_pins\":{},\"project_pins\":{},\
                 \"scientist_pins\":{},\"too_many\":{}}}",
                self.status,
                self.scan_ms,
                self.facilities.len(),
                self.projects.len(),
                self.scientists.len(),
                self.too_many
            )
        }
    }

    struct SpecialProjectPoolHits {
        facilities: Vec<usize>,
        projects: Vec<usize>,
        scientists: Vec<usize>,
        too_many: bool,
    }

    fn scan_special_project_pool_hits(base: usize) -> SpecialProjectPoolHits {
        let facility_vtable = base.saturating_add(PROJECT_FACILITY_VTABLE_RVA) as u64;
        let project_vtable = base.saturating_add(PROJECT_CPROJECT_VTABLE_RVA) as u64;
        let character_vtable = base.saturating_add(PROJECT_CHARACTER_VTABLE_RVA) as u64;
        let mut hits = SpecialProjectPoolHits {
            facilities: Vec::new(),
            projects: Vec::new(),
            scientists: Vec::new(),
            too_many: false,
        };
        let mut addr = HEAP_LO;
        const CHUNK: usize = 16 << 20;
        const OVERLAP: usize = 16;
        while addr < HEAP_HI {
            let Some((region_start, region_size)) = readable_region(addr) else {
                addr = addr.saturating_add(0x1000);
                continue;
            };
            let region_end = region_start.saturating_add(region_size).min(HEAP_HI);
            let mut pos = region_start.max(addr);
            while pos < region_end {
                let want = CHUNK.min(region_end - pos);
                let Some(blob) = read_bytes(pos, want) else {
                    pos = pos.saturating_add(want.max(0x1000));
                    continue;
                };
                let mut i = (8 - (pos & 7)) & 7;
                while i + 8 <= blob.len() {
                    let mut arr = [0u8; 8];
                    arr.copy_from_slice(&blob[i..i + 8]);
                    let value = u64::from_le_bytes(arr);
                    let hit_addr = pos + i;
                    if value == facility_vtable {
                        hits.facilities.push(hit_addr);
                    } else if value == project_vtable {
                        hits.projects.push(hit_addr);
                    } else if value == character_vtable {
                        hits.scientists.push(hit_addr);
                    }
                    if hits.facilities.len() > MAX_PROJECT_FACILITIES
                        || hits.projects.len() > MAX_PROJECT_PROJECT_PINS
                        || hits.scientists.len() > MAX_PROJECT_SCIENTISTS
                    {
                        hits.too_many = true;
                        return hits;
                    }
                    i += 8;
                }
                if pos + blob.len() >= region_end || blob.len() < want {
                    pos = pos.saturating_add(blob.len().max(0x1000));
                } else {
                    pos = pos.saturating_add(blob.len().saturating_sub(OVERLAP));
                }
            }
            addr = region_end.saturating_add(0x1000);
        }
        hits
    }

    fn project_facility_pin(addr: usize, facility_vtable: usize) -> Option<ProjectFacilityPin> {
        if !is_map_control_heap_pointer(addr) || read_u64(addr)? as usize != facility_vtable {
            return None;
        }
        if read_u32(addr + PROJECT_FACILITY_OFF_TYPE)? != PROJECT_FACILITY_TYPE {
            return None;
        }
        let facility_id = read_u32(addr + PROJECT_FACILITY_OFF_ID)?;
        let province_id = read_u32(addr + PROJECT_FACILITY_OFF_PROVINCE)?;
        if facility_id == 0
            || facility_id > MAX_PROJECT_ID
            || province_id == 0
            || province_id > MAX_PROJECT_PROVINCE_ID
        {
            return None;
        }
        Some(ProjectFacilityPin {
            addr,
            facility_id,
            province_id,
        })
    }

    fn project_pin(addr: usize, project_vtable: usize) -> Option<ProjectPin> {
        if !is_map_control_heap_pointer(addr) || read_u64(addr)? as usize != project_vtable {
            return None;
        }
        if read_u32(addr + PROJECT_PROJECT_OFF_TYPE)? != PROJECT_PROJECT_TYPE {
            return None;
        }
        let project_id = read_u32(addr + PROJECT_PROJECT_OFF_ID)?;
        let static_def_handle = read_u32(addr + PROJECT_PROJECT_OFF_DEF_HANDLE)?;
        if project_id == 0
            || project_id > MAX_PROJECT_ID
            || static_def_handle == 0
            || static_def_handle > MAX_PROJECT_HANDLE
        {
            return None;
        }
        Some(ProjectPin {
            addr,
            project_id,
            static_def_handle,
        })
    }

    fn project_scientist_pin(
        addr: usize,
        character_vtable: usize,
        scientist_vtable: usize,
    ) -> Option<ProjectScientistPin> {
        if !is_map_control_heap_pointer(addr) || read_u64(addr)? as usize != character_vtable {
            return None;
        }
        if read_u32(addr + PROJECT_CHARACTER_OFF_TYPE)? != PROJECT_CHARACTER_TYPE {
            return None;
        }
        let scientist_ptr = read_u64(addr + PROJECT_CHARACTER_OFF_SCIENTIST_PTR)? as usize;
        if !is_map_control_heap_pointer(scientist_ptr)
            || read_u64(scientist_ptr)? as usize != scientist_vtable
        {
            return None;
        }
        let char_id = read_u32(addr + PROJECT_CHARACTER_OFF_ID)?;
        if char_id == 0 || char_id > MAX_PROJECT_ID {
            return None;
        }
        Some(ProjectScientistPin { addr, char_id })
    }

    fn write_sinks(file: Option<&mut std::fs::File>, pipe: Option<Handle>, bytes: &[u8]) {
        if let Some(file) = file {
            let _ = file.write_all(bytes);
        }
        if let Some(pipe) = pipe {
            write_handle(pipe, bytes);
        }
    }

    fn module_path() -> Option<PathBuf> {
        let mut buf = [0u16; 32768];
        let len = unsafe {
            let module = DLL_MODULE;
            GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as Dword)
        };
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }

    fn process_path() -> Option<PathBuf> {
        let mut buf = [0u16; 32768];
        let len =
            unsafe { GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), buf.len() as Dword) };
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }

    fn open_pipe(pid: Dword, pipe_token: Option<&str>) -> Option<Handle> {
        let pipe_name = if let Some(token) = pipe_token {
            format!(r"\\.\pipe\battle-plan_collector_{pid}_{token}")
        } else {
            format!(r"\\.\pipe\battle-plan_collector_{pid}")
        };
        let wide: Vec<u16> = OsStr::new(&pipe_name)
            .encode_wide()
            .chain(core::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                0,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if (handle as isize) == -1 {
            None
        } else {
            Some(handle)
        }
    }

    fn write_handle(handle: Handle, bytes: &[u8]) {
        let mut written: Dword = 0;
        unsafe {
            let _ = WriteFile(
                handle,
                bytes.as_ptr() as *const c_void,
                bytes.len().min(Dword::MAX as usize) as Dword,
                &mut written,
                ptr::null_mut(),
            );
        }
    }

    fn scheduled_special_projects_json(
        state: Option<&CollectorState>,
        include_scheduled_projects: bool,
    ) -> Option<String> {
        if !include_scheduled_projects {
            return None;
        }
        let state = state?;
        if !(state.daily_match && state.long_string_match) {
            return None;
        }
        let (facilities, projects, scientists) = state.special_projects.rows(state.base);
        if facilities.is_empty() && projects.is_empty() && scientists.is_empty() {
            return None;
        }
        Some(special_projects_json(&facilities, &projects, &scientists))
    }

    fn read_project_facility_row(
        pin: ProjectFacilityPin,
        facility_vtable: usize,
    ) -> Option<ProjectFacilityRow> {
        let current_pin = project_facility_pin(pin.addr, facility_vtable)?;
        if current_pin.facility_id != pin.facility_id || current_pin.province_id != pin.province_id
        {
            return None;
        }
        let active_project_handle = read_u32(pin.addr + PROJECT_FACILITY_OFF_ACTIVE_HANDLE)
            .filter(|value| *value <= MAX_PROJECT_HANDLE)?;
        let active_flag = read_u32(pin.addr + PROJECT_FACILITY_OFF_ACTIVE_FLAG)?;
        if active_flag > 1 {
            return None;
        }
        let scientist_type = read_u32(pin.addr + PROJECT_FACILITY_OFF_SCIENTIST_TYPE).unwrap_or(0);
        let scientist_id = read_u32(pin.addr + PROJECT_FACILITY_OFF_SCIENTIST_ID)
            .filter(|value| *value <= MAX_PROJECT_ID)?;
        let scientist_char_id = if scientist_id == 0 {
            None
        } else if scientist_type == PROJECT_CHARACTER_TYPE {
            Some(scientist_id)
        } else {
            return None;
        };
        let unread_prototype_rewards = read_u32(pin.addr + PROJECT_FACILITY_OFF_UNREAD_REWARDS)
            .filter(|value| *value <= MAX_PROJECT_ITERATIONS)?;
        let building = read_msvc_string(
            pin.addr + PROJECT_FACILITY_OFF_BUILDING,
            MAX_PROJECT_BUILDING_LEN,
        )
        .filter(|value| is_script_token(value));
        Some(ProjectFacilityRow {
            id: pin.facility_id,
            province: pin.province_id,
            building,
            active_project_handle,
            active: active_flag == 1,
            scientist_char_id,
            unread_prototype_rewards,
        })
    }

    fn read_project_row(pin: ProjectPin, project_vtable: usize) -> Option<ProjectRow> {
        let current_pin = project_pin(pin.addr, project_vtable)?;
        if current_pin.project_id != pin.project_id
            || current_pin.static_def_handle != pin.static_def_handle
        {
            return None;
        }
        let owner_tag = read_u32(pin.addr + PROJECT_PROJECT_OFF_OWNER_TAG).unwrap_or(0);
        let owner_fid = read_u32(pin.addr + PROJECT_PROJECT_OFF_OWNER_FACILITY).unwrap_or(0);
        if owner_tag != PROJECT_FACILITY_TYPE || owner_fid == 0 || owner_fid > MAX_PROJECT_ID {
            return None;
        }
        let iterations = read_u32(pin.addr + PROJECT_PROJECT_OFF_ITERATIONS)
            .filter(|value| *value <= MAX_PROJECT_ITERATIONS)?;
        Some(ProjectRow {
            id: pin.project_id,
            static_def_handle: pin.static_def_handle,
            iterations,
            worked_at_facility: Some(owner_fid),
        })
    }

    fn read_project_scientist_row(
        pin: ProjectScientistPin,
        character_vtable: usize,
        scientist_vtable: usize,
    ) -> Option<ProjectScientistRow> {
        let current_pin = project_scientist_pin(pin.addr, character_vtable, scientist_vtable)?;
        if current_pin.char_id != pin.char_id {
            return None;
        }
        let name_token = read_msvc_string(
            pin.addr + PROJECT_CHARACTER_OFF_TOKEN,
            MAX_PROJECT_SCIENTIST_TOKEN_LEN,
        )
        .filter(|value| is_script_token(value));
        let display_name = read_msvc_string(
            pin.addr + PROJECT_CHARACTER_OFF_DISPLAY,
            MAX_PROJECT_SCIENTIST_DISPLAY_LEN,
        );
        let scientist_ptr = read_u64(pin.addr + PROJECT_CHARACTER_OFF_SCIENTIST_PTR)? as usize;
        let trait_count = read_u32(scientist_ptr + PROJECT_SCIENTIST_OFF_TRAIT_COUNT)
            .filter(|value| *value <= MAX_PROJECT_TRAITS);
        let skills = read_project_scientist_skills(scientist_ptr)?;
        Some(ProjectScientistRow {
            char_id: pin.char_id,
            name_token,
            display_name,
            skills,
            trait_count,
        })
    }

    fn read_project_scientist_skills(scientist_ptr: usize) -> Option<BTreeMap<&'static str, i32>> {
        let skills_ptr = read_u64(scientist_ptr + PROJECT_SCIENTIST_OFF_SKILLS_PTR)? as usize;
        if !is_map_control_heap_pointer(skills_ptr) {
            return None;
        }
        let names = [
            "specialization_nuclear",
            "specialization_naval",
            "specialization_air",
            "specialization_land",
        ];
        let mut skills = BTreeMap::new();
        for (idx, name) in names.iter().enumerate() {
            let rec = skills_ptr.checked_add(idx.checked_mul(PROJECT_SKILL_STRIDE)?)?;
            let expected = PROJECT_SKILL_HEADER_BASE.checked_add(idx as u16)?;
            if read_u16(rec)? != expected {
                return None;
            }
            let level = read_i32(rec + PROJECT_SKILL_LEVEL_OFF)?;
            if !(0..=MAX_PROJECT_SKILL_LEVEL).contains(&level) {
                return None;
            }
            skills.insert(*name, level);
        }
        Some(skills)
    }

    fn special_projects_json(
        facilities: &[ProjectFacilityRow],
        projects: &[ProjectRow],
        scientists: &[ProjectScientistRow],
    ) -> String {
        let facility_rows: Vec<String> = facilities.iter().map(project_facility_json).collect();
        let project_rows: Vec<String> = projects.iter().map(project_row_json).collect();
        let scientist_rows: Vec<String> = scientists.iter().map(project_scientist_json).collect();
        format!(
            "{{\"facilities\":[{}],\"projects\":[{}],\"scientists\":[{}],\
             \"count\":{{\"facilities\":{},\"projects\":{},\"scientists\":{}}},\
             \"source\":\"C\"}}",
            facility_rows.join(","),
            project_rows.join(","),
            scientist_rows.join(","),
            facilities.len(),
            projects.len(),
            scientists.len()
        )
    }

    fn project_facility_json(row: &ProjectFacilityRow) -> String {
        format!(
            "{{\"id\":{},\"province\":{},\"building\":{},\
             \"active_project_handle\":{},\"active\":{},\
             \"scientist_char_id\":{},\"unread_prototype_rewards\":{}}}",
            row.id,
            row.province,
            json_opt_escaped_str(row.building.as_deref()),
            row.active_project_handle,
            row.active,
            json_opt_u32(row.scientist_char_id),
            row.unread_prototype_rewards
        )
    }

    fn project_row_json(row: &ProjectRow) -> String {
        format!(
            "{{\"id\":{},\"static_def_handle\":{},\"iterations\":{},\
             \"worked_at_facility\":{}}}",
            row.id,
            row.static_def_handle,
            row.iterations,
            json_opt_u32(row.worked_at_facility)
        )
    }

    fn project_scientist_json(row: &ProjectScientistRow) -> String {
        let skill_rows: Vec<String> = row
            .skills
            .iter()
            .map(|(key, value)| format!("{}:{}", json_str(key), value))
            .collect();
        format!(
            "{{\"char_id\":{},\"name_token\":{},\"display_name\":{},\
             \"skills\":{{{}}},\"trait_count\":{}}}",
            row.char_id,
            json_opt_escaped_str(row.name_token.as_deref()),
            json_opt_escaped_str(row.display_name.as_deref()),
            skill_rows.join(","),
            json_opt_u32(row.trait_count)
        )
    }

    fn scheduled_state_rows_json(
        state: Option<&CollectorState>,
        control: Option<&CollectorControl>,
        include_scheduled_map: bool,
    ) -> Option<String> {
        if !include_scheduled_map {
            return None;
        }
        let state = state?;
        if !(state.daily_match && state.long_string_match) {
            return None;
        }
        if let Some(rows) = scheduled_cprovince_state_rows_json(state) {
            return Some(rows);
        }
        let control = control?;
        if !(control.map_control.state_roots_valid && control.map_control.state_roots_well_formed())
        {
            return None;
        }
        if control.map_control.state_roots.len() < MIN_USABLE_STATE_ROWS {
            return None;
        }
        let tag_table = country_tag_table(state.country_database_ptr as usize)?;
        let state_vtable = state.base + STATE_VTABLE_RVA;
        let mut rows = Vec::new();
        for entry in &control.map_control.state_roots {
            if read_u64(entry.state_ptr)? as usize != state_vtable {
                return None;
            }
            let sid = read_u32(entry.state_ptr + STATE_OFF_SID)?;
            if sid != entry.state_id {
                return None;
            }
            let owner = read_u32(entry.state_ptr + STATE_OFF_OWNER).unwrap_or(0);
            let controller = read_u32(entry.state_ptr + STATE_OFF_CTRL).unwrap_or(0);
            let owner_tag = if owner == 0 {
                None
            } else {
                tag_from_table(tag_table, owner)
            };
            let controller_tag = if controller == 0 {
                None
            } else {
                tag_from_table(tag_table, controller)
            };
            rows.push(format!(
                "\"{}\":{{\"owner\":{},\"controller\":{}}}",
                entry.state_id,
                json_opt_str(owner_tag.as_deref()),
                json_opt_str(controller_tag.as_deref())
            ));
        }
        if rows.len() >= MIN_USABLE_STATE_ROWS {
            Some(format!("{{{}}}", rows.join(",")))
        } else {
            None
        }
    }

    fn scheduled_cprovince_state_rows_json(state: &CollectorState) -> Option<String> {
        let (province_array, province_count, province_ptrs) = cprovince_array_snapshot(state)?;
        let tag_table = country_tag_table(state.country_database_ptr as usize)?;
        let cprovince_vtable = state.base.checked_add(CPROVINCE_VTABLE_RVA)?;
        let state_vtable = state.base.checked_add(STATE_VTABLE_RVA)?;
        let mut state_ptrs: BTreeMap<u32, usize> = BTreeMap::new();

        for province_id in 1..province_count {
            let province_ptr = cprovince_ptr_from_snapshot(&province_ptrs, province_id)?;
            if province_ptr == 0 {
                continue;
            }
            if !is_map_control_heap_pointer(province_ptr) {
                return None;
            }
            if read_u64(province_ptr)? as usize != cprovince_vtable {
                return None;
            }
            if read_u32(province_ptr.checked_add(CPROVINCE_OFF_ID)?)? != province_id {
                return None;
            }
            let state_ptr = read_u64(province_ptr.checked_add(CPROVINCE_OFF_STATE)?)? as usize;
            if state_ptr == 0 {
                continue;
            }
            if !is_map_control_heap_pointer(state_ptr) {
                return None;
            }
            if read_u64(state_ptr)? as usize != state_vtable {
                return None;
            }
            let state_id = read_u32(state_ptr.checked_add(STATE_OFF_SID)?)?;
            if state_id == 0 || state_id > MAX_MAP_CONTROL_STATE_ID {
                return None;
            }
            if let Some(previous) = state_ptrs.insert(state_id, state_ptr) {
                if previous != state_ptr {
                    return None;
                }
            }
        }

        if state_ptrs.len() < MIN_USABLE_STATE_ROWS {
            return None;
        }
        let mut rows = Vec::new();
        for (state_id, state_ptr) in state_ptrs {
            if read_u64(state_ptr)? as usize != state_vtable {
                return None;
            }
            if read_u32(state_ptr.checked_add(STATE_OFF_SID)?)? != state_id {
                return None;
            }
            let owner = read_u32(state_ptr.checked_add(STATE_OFF_OWNER)?).unwrap_or(0);
            let controller = read_u32(state_ptr.checked_add(STATE_OFF_CTRL)?).unwrap_or(0);
            let owner_tag = if owner == 0 {
                None
            } else {
                tag_from_table(tag_table, owner)
            };
            let controller_tag = if controller == 0 {
                None
            } else {
                tag_from_table(tag_table, controller)
            };
            rows.push(format!(
                "\"{}\":{{\"owner\":{},\"controller\":{}}}",
                state_id,
                json_opt_str(owner_tag.as_deref()),
                json_opt_str(controller_tag.as_deref())
            ));
        }

        if !cprovince_array_still_current(state, province_array, province_count) {
            return None;
        }
        if rows.len() >= MIN_USABLE_STATE_ROWS {
            Some(format!("{{{}}}", rows.join(",")))
        } else {
            None
        }
    }

    fn scheduled_province_rows_json(
        state: Option<&CollectorState>,
        control: Option<&CollectorControl>,
        include_scheduled_map: bool,
    ) -> Option<String> {
        if !include_scheduled_map {
            return None;
        }
        let state = state?;
        if !(state.daily_match && state.long_string_match) {
            return None;
        }
        if let Some(rows) = scheduled_cprovince_rows_json(state) {
            return Some(rows);
        }
        let control = control?;
        if !(control.map_control.province_bridge_valid
            && control.map_control.province_bridge_well_formed()
            && control.map_control.controller_bridge_valid
            && control.map_control.controller_bridge_well_formed())
        {
            return None;
        }
        let bridge = control.map_control.province_bridge?;
        if control.map_control.controller_bridge.is_empty() {
            return None;
        }
        let tag_table = country_tag_table(state.country_database_ptr as usize)?;
        let mut tags_by_controller_ptr: BTreeMap<usize, String> = BTreeMap::new();
        for entry in &control.map_control.controller_bridge {
            tags_by_controller_ptr.insert(
                entry.controller_ptr,
                tag_from_table(tag_table, entry.tag_db_id)?,
            );
        }

        let mut rows = Vec::new();
        for province_id in 1..bridge.province_count {
            let slot_offset =
                (province_id as usize).checked_mul(bridge.province_stride as usize)?;
            let entry_addr = bridge
                .province_array_ptr
                .checked_add(slot_offset)?
                .checked_add(bridge.controller_ptr_offset as usize)?;
            let controller_ptr = read_u64(entry_addr)? as usize;
            if controller_ptr == 0 {
                continue;
            }
            let controller_tag = tags_by_controller_ptr.get(&controller_ptr)?;
            rows.push(format!(
                "\"{}\":{{\"controller\":\"{}\"}}",
                province_id, controller_tag
            ));
        }

        if rows.len() >= MIN_USABLE_PROVINCE_ROWS {
            Some(format!("{{{}}}", rows.join(",")))
        } else {
            None
        }
    }

    fn scheduled_cprovince_rows_json(state: &CollectorState) -> Option<String> {
        let (province_array, province_count, province_ptrs) = cprovince_array_snapshot(state)?;
        let tag_table = country_tag_table(state.country_database_ptr as usize)?;
        let cprovince_vtable = state.base.checked_add(CPROVINCE_VTABLE_RVA)?;
        let mut rows = Vec::new();
        for province_id in 1..province_count {
            let province_ptr = cprovince_ptr_from_snapshot(&province_ptrs, province_id)?;
            if province_ptr == 0 {
                continue;
            }
            if !is_map_control_heap_pointer(province_ptr) {
                return None;
            }
            if read_u64(province_ptr)? as usize != cprovince_vtable {
                return None;
            }
            if read_u32(province_ptr.checked_add(CPROVINCE_OFF_ID)?)? != province_id {
                return None;
            }
            let controller_tag_db_id =
                read_u32(province_ptr.checked_add(CPROVINCE_OFF_CONTROLLER)?).unwrap_or(0);
            if controller_tag_db_id == 0 {
                continue;
            }
            let controller_tag = tag_from_table(tag_table, controller_tag_db_id);
            let state_id = cprovince_state_id(state.base, province_ptr);
            rows.push(cprovince_control_row_json(
                province_id,
                controller_tag.as_deref(),
                controller_tag_db_id,
                state_id,
            ));
        }
        if !cprovince_array_still_current(state, province_array, province_count) {
            return None;
        }
        if rows.len() >= MIN_USABLE_PROVINCE_ROWS {
            Some(format!("{{{}}}", rows.join(",")))
        } else {
            None
        }
    }

    fn cprovince_array_snapshot(state: &CollectorState) -> Option<(usize, u32, Vec<u8>)> {
        let gamestate = state.date_ptr as usize;
        if !is_map_control_heap_pointer(gamestate) {
            return None;
        }
        let province_array = read_u64(gamestate.checked_add(CGAMESTATE_CPROVINCE_ARRAY)?)? as usize;
        let province_count = read_u32(gamestate.checked_add(CGAMESTATE_CPROVINCE_COUNT)?)?;
        if province_count < MIN_USABLE_PROVINCE_ROWS as u32
            || province_count > MAX_MAP_CONTROL_PROVINCE_COUNT
            || !is_map_control_heap_pointer(province_array)
        {
            return None;
        }
        let byte_len = (province_count as usize).checked_mul(8)?;
        let province_ptrs = read_bytes(province_array, byte_len)?;
        if province_ptrs.len() != byte_len {
            return None;
        }
        Some((province_array, province_count, province_ptrs))
    }

    fn cprovince_ptr_from_snapshot(province_ptrs: &[u8], province_id: u32) -> Option<usize> {
        let slot_offset = (province_id as usize).checked_mul(8)?;
        read_u64_from_slice(province_ptrs, slot_offset).map(|ptr| ptr as usize)
    }

    fn cprovince_array_still_current(
        state: &CollectorState,
        province_array: usize,
        province_count: u32,
    ) -> bool {
        let gamestate = state.date_ptr as usize;
        read_u64(gamestate.saturating_add(CGAMESTATE_CPROVINCE_ARRAY)).map(|v| v as usize)
            == Some(province_array)
            && read_u32(gamestate.saturating_add(CGAMESTATE_CPROVINCE_COUNT))
                == Some(province_count)
    }

    fn cprovince_state_id(module_base: usize, province_ptr: usize) -> Option<u32> {
        let state_ptr = read_u64(province_ptr.checked_add(CPROVINCE_OFF_STATE)?)? as usize;
        if !is_map_control_heap_pointer(state_ptr) {
            return None;
        }
        if read_u64(state_ptr)? as usize != module_base.checked_add(STATE_VTABLE_RVA)? {
            return None;
        }
        read_u32(state_ptr.checked_add(STATE_OFF_SID)?)
            .filter(|state_id| *state_id > 0 && *state_id <= MAX_MAP_CONTROL_STATE_ID)
    }

    fn cprovince_control_row_json(
        province_id: u32,
        controller_tag: Option<&str>,
        controller_tag_db_id: u32,
        state_id: Option<u32>,
    ) -> String {
        let controller = json_opt_str(controller_tag);
        let decode_status = if controller_tag.is_some() {
            "decoded"
        } else {
            "tag_table_miss"
        };
        let state_field = state_id
            .map(|state_id| format!(",\"state_id\":{state_id}"))
            .unwrap_or_default();
        format!(
            "\"{}\":{{\"controller\":{},\"controller_tag_db_id\":{},\
             \"controller_decode_status\":\"{}\",\"controller_source\":\"cprovince_static_root\"{}}}",
            province_id, controller, controller_tag_db_id, decode_status, state_field
        )
    }

    fn root_probe_json(
        state: Option<&CollectorState>,
        scheduled_state_rows: Option<&str>,
        scheduled_province_rows: Option<&str>,
        scheduled_special_projects: Option<&str>,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
        include_scheduled_country_focus: bool,
        include_scheduled_country_research: bool,
        include_scheduled_country_politics: bool,
        include_scheduled_country_politics_residuals: bool,
        include_scheduled_country_decisions: bool,
        advisor_token_filter: Option<&BTreeMap<String, AdvisorTokenInfo>>,
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> (String, String) {
        let Some(state) = state else {
            return ("\"root_probe\":null".to_string(), "[]".to_string());
        };
        let base = state.base;
        if state.base == 0 {
            return (
                "\"root_probe\":{\"status\":\"profile_mismatch\",\"reason\":\"no_image_base\"}"
                    .to_string(),
                "[]".to_string(),
            );
        }

        if !(state.daily_match && state.long_string_match) {
            return (
                format!(
                    "\"root_probe\":{{\"status\":\"profile_mismatch\",\
                     \"image_base\":\"0x{base:X}\",\
                     \"daily_update_sentinel_match\":{},\
                     \"long_string_sentinel_match\":{}}}",
                    state.daily_match, state.long_string_match
                ),
                "[]".to_string(),
            );
        }

        let date_ptr = state.date_ptr;
        let country_database_ptr = state.country_database_ptr;
        let country_probe = country_database_probe_json(country_database_ptr as usize);
        let current_rows = state.country_identity.current_rows(
            country_database_ptr as usize,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
            include_scheduled_country_focus,
            include_scheduled_country_research,
            include_scheduled_country_politics,
            include_scheduled_country_politics_residuals,
            include_scheduled_country_decisions,
            advisor_token_filter,
            idea_group_filter,
        );
        let country_rows = state.country_identity.countries_json_from_rows(
            country_database_ptr as usize,
            &current_rows,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
            include_scheduled_country_focus,
            include_scheduled_country_research,
            include_scheduled_country_politics,
            include_scheduled_country_politics_residuals,
            include_scheduled_country_decisions,
        );
        let identity_probe = state.country_identity.root_json();
        let domains = state.country_identity.domains_json_from_rows(
            &current_rows,
            scheduled_state_rows.is_some(),
            scheduled_province_rows.is_some(),
            scheduled_special_projects.is_some(),
            include_scheduled_country_fuel,
            include_scheduled_country_production,
            include_scheduled_country_focus,
            include_scheduled_country_research,
            include_scheduled_country_politics,
            include_scheduled_country_politics_residuals,
            include_scheduled_country_decisions,
        );
        let clock_probe = clock_probe_json(date_ptr as usize);
        let special_projects_probe = state.special_projects.root_json();
        let state_rows = scheduled_state_rows
            .map(|rows| format!(",\"states\":{rows}"))
            .unwrap_or_default();
        let province_rows = scheduled_province_rows
            .map(|rows| format!(",\"provinces\":{rows}"))
            .unwrap_or_default();
        (
            format!(
                "\"root_probe\":{{\"status\":\"verified\",\
                 \"profile\":\"{}\",\
                 \"image_base\":\"0x{base:X}\",\
                 \"daily_update\":\"0x{:X}\",\
                 \"get_long_string\":\"0x{:X}\",\
                 \"daily_update_sentinel_match\":true,\
                 \"long_string_sentinel_match\":true,\
                 \"date_root_ptr\":\"0x{date_ptr:X}\",\
                 \"country_database_root_ptr\":\"0x{country_database_ptr:X}\",\
                 {clock_probe},\
                 {country_probe},\
                 {identity_probe},\
                 {special_projects_probe},\
                 \"domains\":{domains}}},\
                 \"countries\":{country_rows}{state_rows}{province_rows}",
                super::COLLECTOR_PROFILE_ID,
                state.daily_addr,
                state.long_string_addr
            ),
            domains,
        )
    }

    fn is_hoi4_process() -> bool {
        process_path()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
            })
            .as_deref()
            == Some("hoi4.exe")
    }

    fn sentinel_matches(addr: usize, sentinel: &[u8]) -> bool {
        read_bytes(addr, sentinel.len())
            .map(|bytes| bytes == sentinel)
            .unwrap_or(false)
    }

    fn read_u64(addr: usize) -> Option<u64> {
        let bytes = read_bytes(addr, 8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Some(u64::from_le_bytes(arr))
    }

    fn read_u32(addr: usize) -> Option<u32> {
        let bytes = read_bytes(addr, 4)?;
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&bytes);
        Some(u32::from_le_bytes(arr))
    }

    fn read_u16(addr: usize) -> Option<u16> {
        let bytes = read_bytes(addr, 2)?;
        let mut arr = [0u8; 2];
        arr.copy_from_slice(&bytes);
        Some(u16::from_le_bytes(arr))
    }

    fn read_u8(addr: usize) -> Option<u8> {
        read_bytes(addr, 1).and_then(|bytes| bytes.first().copied())
    }

    fn country_database_probe_json(country_database_ptr: usize) -> String {
        if country_database_ptr == 0 {
            return "\"country_tag_count\":null,\"country_active_count\":null,\
                    \"tag_sample\":[]"
                .to_string();
        }
        let tag_table = read_u64(country_database_ptr).unwrap_or(0) as usize;
        let tag_count = read_u32(country_database_ptr + 8).unwrap_or(0);
        let active_count = read_u32(country_database_ptr + 12).unwrap_or(0);
        let sample = tag_sample_json(tag_table, tag_count.min(12));
        format!(
            "\"country_tag_count\":{tag_count},\
             \"country_active_count\":{active_count},\
             \"tag_sample\":{sample}"
        )
    }

    fn country_rows_from_tag_table_json(country_database_ptr: usize) -> String {
        if country_database_ptr == 0 {
            return "{}".to_string();
        }
        let tag_table = read_u64(country_database_ptr).unwrap_or(0) as usize;
        let tag_count = read_u32(country_database_ptr + 8).unwrap_or(0);
        let active_count = read_u32(country_database_ptr + 12).unwrap_or(0);
        if tag_table == 0 || tag_count == 0 || active_count == 0 {
            return "{}".to_string();
        }
        let row_count = tag_count.min(active_count).min(1024);
        let mut rows = Vec::new();
        for i in 0..row_count as usize {
            let Some(tag) = read_small_inline_string(tag_table + i * 32) else {
                continue;
            };
            if is_country_tag(&tag) {
                rows.push(format!(
                    "\"{tag}\":{{\"present\":true,\"identity\":{{\"tag_db_id\":{i}}}}}"
                ));
            }
        }
        format!("{{{}}}", rows.join(","))
    }

    fn country_tag_table(country_database_ptr: usize) -> Option<usize> {
        if country_database_ptr == 0 {
            return None;
        }
        let tag_table = read_u64(country_database_ptr)? as usize;
        let tag_count = read_u32(country_database_ptr + 8)?;
        if tag_table == 0 || tag_count < 40 || tag_count > 1024 {
            return None;
        }
        let prefix = ["---", "GER", "ENG", "SOV"];
        for (i, want) in prefix.iter().enumerate() {
            let got = read_small_inline_string(tag_table + i * 32)?;
            if got != *want {
                return None;
            }
        }
        Some(tag_table)
    }

    fn country_tag_row_count(country_database_ptr: usize) -> Option<u32> {
        if country_database_ptr == 0 {
            return None;
        }
        let tag_count = read_u32(country_database_ptr + 8)?;
        let active_count = read_u32(country_database_ptr + 12)?;
        if active_count == 0 || active_count > tag_count || tag_count > 1024 {
            return None;
        }
        let row_count = tag_count.min(active_count).min(1024);
        if row_count <= MIN_USABLE_COUNTRY_OBJECT_ROWS as u32 {
            return None;
        }
        Some(row_count)
    }

    fn country_object_array_base_candidates(field_addr: usize) -> Vec<usize> {
        let mut candidates = vec![field_addr];
        if let Some(ptr) = read_u64(field_addr).map(|value| value as usize) {
            if ptr > 0 && ptr % 8 == 0 && ptr != field_addr {
                candidates.push(ptr);
            }
        }
        candidates
    }

    fn country_rows_from_tag_indexed_array(
        array_base: usize,
        country_vtable: usize,
        tag_table: usize,
        tag_row_count: u32,
    ) -> Option<Vec<CountryObjectRow>> {
        if !is_map_control_heap_pointer(array_base)
            || tag_row_count <= MIN_USABLE_COUNTRY_OBJECT_ROWS as u32
            || tag_row_count > 1024
        {
            return None;
        }

        let mut rows = Vec::new();
        let mut seen_objects: BTreeMap<usize, ()> = BTreeMap::new();
        let mut holes = 0usize;
        for tag_db_id in 1..tag_row_count {
            let slot_addr = array_base.checked_add(tag_db_id as usize * 8)?;
            let object_ptr = read_u64(slot_addr).unwrap_or(0) as usize;
            if object_ptr == 0 {
                holes += 1;
                if holes >= 16 && tag_db_id as usize > MIN_USABLE_COUNTRY_OBJECT_ROWS {
                    break;
                }
                continue;
            }
            holes = 0;
            if !is_map_control_heap_pointer(object_ptr)
                || seen_objects.insert(object_ptr, ()).is_some()
            {
                return None;
            }
            let row = country_object_row_expected(
                object_ptr,
                country_vtable,
                tag_table,
                tag_db_id,
                None,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                None,
                None,
            )?;
            rows.push(row);
        }

        if rows.len() >= MIN_USABLE_COUNTRY_OBJECT_ROWS {
            Some(rows)
        } else {
            None
        }
    }

    fn country_tag_object_map(rows: &[CountryObjectRow]) -> BTreeMap<u32, usize> {
        rows.iter()
            .map(|row| (row.tag_db_id, row.object_ptr))
            .collect()
    }

    fn country_tag_object_maps_agree(
        left: &BTreeMap<u32, usize>,
        right: &BTreeMap<u32, usize>,
    ) -> bool {
        let mut shared = 0usize;
        for (tag_db_id, object_ptr) in left {
            if let Some(other_object_ptr) = right.get(tag_db_id) {
                shared += 1;
                if object_ptr != other_object_ptr {
                    return false;
                }
            }
        }
        shared >= MIN_USABLE_COUNTRY_OBJECT_ROWS
    }

    fn tag_from_table(tag_table: usize, tag_db_id: u32) -> Option<String> {
        if tag_db_id > 1024 {
            return None;
        }
        read_small_inline_string(tag_table + tag_db_id as usize * 32)
            .filter(|tag| is_country_tag(tag))
    }

    fn country_object_row(
        object_ptr: usize,
        country_vtable: usize,
        tag_table: usize,
        record_ptr: Option<usize>,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
        include_scheduled_country_focus: bool,
        include_scheduled_country_research: bool,
        include_scheduled_country_politics: bool,
        include_scheduled_country_politics_residuals: bool,
        include_scheduled_country_decisions: bool,
        advisor_token_filter: Option<&BTreeMap<String, AdvisorTokenInfo>>,
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> Option<CountryObjectRow> {
        if read_u64(object_ptr)? as usize != country_vtable {
            return None;
        }
        let name = read_msvc_string(object_ptr + OBJ_NAME, 96)?;
        if read_msvc_string(object_ptr + OBJ_NAME, 96)? != name {
            return None;
        }
        let tag_db_id = read_u32(object_ptr + OBJ_TAG_DB_ID)?;
        let tag = tag_from_table(tag_table, tag_db_id)?;
        let original_tag_id = read_u16(object_ptr + OBJ_ORIGINAL_TAG);
        let original_tag = original_tag_id.and_then(|id| {
            if id == 0 {
                None
            } else {
                tag_from_table(tag_table, id as u32)
            }
        });
        Some(CountryObjectRow {
            tag,
            tag_db_id,
            name,
            object_ptr,
            record_ptr,
            original_capital: read_i32(object_ptr + OBJ_ORIGINAL_CAPITAL).and_then(|v| {
                if v > 0 {
                    Some(v)
                } else {
                    None
                }
            }),
            original_tag,
            templates_locked: read_u8(object_ptr + OBJ_TEMPLATES_LOCKED).map(|v| v == 1),
            reserved_dynamic_country: read_u8(object_ptr + OBJ_RESERVED_DYNAMIC_COUNTRY)
                .map(|v| v == 1),
            stability: read_fixed_0_1(object_ptr + OBJ_STABILITY),
            war_support: read_fixed_0_1(object_ptr + OBJ_WAR_SUPPORT),
            political_power: country_object_political_power(object_ptr, country_vtable, record_ptr),
            command_power: read_fixed_nonnegative(object_ptr + OBJ_COMMAND_POWER, 1000.0),
            num_ships: read_i32(object_ptr + OBJ_NUM_SHIPS).and_then(|v| {
                if (0..=100_000).contains(&v) {
                    Some(v)
                } else {
                    None
                }
            }),
            manpower_pool: read_u32(object_ptr + OBJ_MANPOWER_POOL).and_then(|v| {
                if v <= 2_000_000_000 {
                    Some(v)
                } else {
                    None
                }
            }),
            convoys: read_fixed_nonnegative(object_ptr + OBJ_CONVOYS, 1_000_000.0),
            misc: read_country_misc(object_ptr),
            fuel: if include_scheduled_country_fuel {
                read_country_fuel(object_ptr)
            } else {
                None
            },
            production: if include_scheduled_country_production {
                read_country_production(object_ptr, country_vtable)
            } else {
                None
            },
            focus: if include_scheduled_country_focus {
                read_country_focus(object_ptr)
            } else {
                None
            },
            research: if include_scheduled_country_research {
                read_country_research(object_ptr, country_vtable)
            } else {
                None
            },
            politics: if include_scheduled_country_politics {
                read_country_politics(object_ptr, country_vtable, idea_group_filter)
            } else {
                None
            },
            politics_residuals: if include_scheduled_country_politics_residuals {
                read_country_politics_residuals(object_ptr, country_vtable, advisor_token_filter)
            } else {
                None
            },
            decisions: if include_scheduled_country_decisions {
                read_country_decisions(object_ptr, country_vtable)
            } else {
                None
            },
        })
    }

    fn country_object_row_expected(
        object_ptr: usize,
        country_vtable: usize,
        tag_table: usize,
        expected_tag_db_id: u32,
        record_ptr: Option<usize>,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
        include_scheduled_country_focus: bool,
        include_scheduled_country_research: bool,
        include_scheduled_country_politics: bool,
        include_scheduled_country_politics_residuals: bool,
        include_scheduled_country_decisions: bool,
        advisor_token_filter: Option<&BTreeMap<String, AdvisorTokenInfo>>,
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> Option<CountryObjectRow> {
        let row = country_object_row(
            object_ptr,
            country_vtable,
            tag_table,
            record_ptr,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
            include_scheduled_country_focus,
            include_scheduled_country_research,
            include_scheduled_country_politics,
            include_scheduled_country_politics_residuals,
            include_scheduled_country_decisions,
            advisor_token_filter,
            idea_group_filter,
        )?;
        if row.tag_db_id == expected_tag_db_id {
            Some(row)
        } else {
            None
        }
    }

    fn revalidated_country_object_row(
        expected: &CountryObjectRow,
        country_vtable: usize,
        tag_table: usize,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
        include_scheduled_country_focus: bool,
        include_scheduled_country_research: bool,
        include_scheduled_country_politics: bool,
        include_scheduled_country_politics_residuals: bool,
        include_scheduled_country_decisions: bool,
        advisor_token_filter: Option<&BTreeMap<String, AdvisorTokenInfo>>,
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> Option<CountryObjectRow> {
        let row = country_object_row_expected(
            expected.object_ptr,
            country_vtable,
            tag_table,
            expected.tag_db_id,
            expected.record_ptr,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
            include_scheduled_country_focus,
            include_scheduled_country_research,
            include_scheduled_country_politics,
            include_scheduled_country_politics_residuals,
            include_scheduled_country_decisions,
            advisor_token_filter,
            idea_group_filter,
        )?;
        if row.tag == expected.tag && row.name == expected.name {
            Some(row)
        } else {
            None
        }
    }

    fn read_country_misc(object_ptr: usize) -> Option<CountryMiscRow> {
        let row = CountryMiscRow {
            convoys_destroyed: read_u32_stable(object_ptr + OBJ_CONVOYS_DESTROYED)
                .filter(|v| *v <= MAX_COUNTRY_CONVOYS_DESTROYED),
            accidents_score: read_i32_stable(object_ptr + OBJ_ACCIDENTS_SCORE)
                .map(|v| v as f64 / FP_SCALE)
                .filter(|v| v.is_finite() && (0.0..=MAX_COUNTRY_ACCIDENTS_SCORE).contains(v)),
            ace_count: read_u32_stable(object_ptr + OBJ_ACE_COUNT)
                .filter(|v| *v <= MAX_COUNTRY_ACE_COUNT),
            cosmetic_tag: read_stable_msvc_token_nullable(
                object_ptr + OBJ_COSMETIC_TAG,
                MAX_COUNTRY_COSMETIC_TAG_LEN,
            ),
        };
        if row.has_any() {
            Some(row)
        } else {
            None
        }
    }

    fn country_record_political_power(record_ptr: usize, object_ptr: usize) -> Option<f64> {
        if read_u64(record_ptr + COUNTRY_RECORD_OBJECT_PTR)? as usize != object_ptr {
            return None;
        }
        let value = read_i32(record_ptr + COUNTRY_RECORD_POLITICAL_POWER)? as f64 / FP_SCALE;
        if (-1_000_000.0..=1_000_000.0).contains(&value) {
            Some(value)
        } else {
            None
        }
    }

    fn read_country_focus(object_ptr: usize) -> Option<CountryFocusRow> {
        let tree_def = read_u64(object_ptr + OBJ_FOCUS_TREE).unwrap_or(0) as usize;
        let focus_tree = read_focus_id(tree_def, FOCUS_TREE_ID);
        let submgr = read_u64(object_ptr + OBJ_FOCUS_STATUS)? as usize;
        if !is_readable_range(submgr, 0x70) {
            return None;
        }

        let completed_count = read_u32_stable(submgr + FOCUS_STATUS_COMPLETED_SIZE)
            .filter(|value| *value <= MAX_FOCUS_COUNT);
        let completed = completed_count.and_then(|size| read_focus_completed_vector(submgr, size));
        let current_ptr = read_u64(submgr + FOCUS_STATUS_CURRENT).unwrap_or(0) as usize;
        let continuous_ptr = read_u64(submgr + FOCUS_STATUS_CONTINUOUS).unwrap_or(0) as usize;
        let current = read_focus_id(current_ptr, FOCUS_OBJECT_ID);
        let current_continuous = read_focus_id(continuous_ptr, FOCUS_OBJECT_ID);
        let progress = if current.is_some() ^ current_continuous.is_some() {
            read_i32_stable(submgr + FOCUS_STATUS_PROGRESS)
                .map(|value| value as f64 / FP_SCALE)
                .filter(|value| value.is_finite() && (0.0..=10_000.0).contains(value))
        } else {
            None
        };

        let row = CountryFocusRow {
            focus_tree,
            completed,
            completed_count,
            current,
            current_continuous,
            progress,
        };
        if row.has_any() {
            Some(row)
        } else {
            None
        }
    }

    fn read_focus_completed_vector(submgr: usize, size: u32) -> Option<Vec<String>> {
        let cap = read_u32_stable(submgr + FOCUS_STATUS_COMPLETED_CAP)?;
        if cap < size || cap > MAX_FOCUS_COUNT {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }
        let data = read_u64(submgr + FOCUS_STATUS_COMPLETED_DATA)? as usize;
        let bytes = (size as usize).checked_mul(8)?;
        if !is_readable_range(data, bytes) {
            return None;
        }
        let mut out = Vec::new();
        for idx in 0..size as usize {
            let def_ptr = read_u64(data + idx * 8)? as usize;
            let focus_id = read_focus_id(def_ptr, FOCUS_OBJECT_ID)?;
            out.push(focus_id);
        }
        Some(out)
    }

    fn read_focus_id(ptr: usize, id_offset: usize) -> Option<String> {
        if ptr == 0 || !is_readable_range(ptr, id_offset + 0x20) {
            return None;
        }
        let value = read_msvc_string(ptr + id_offset, MAX_FOCUS_ID_LEN)?;
        if is_script_token(&value) {
            Some(value)
        } else {
            None
        }
    }

    fn read_country_research(
        object_ptr: usize,
        country_vtable: usize,
    ) -> Option<CountryResearchRow> {
        let module_base = country_vtable.checked_sub(COUNTRY_VTABLE_RVA)?;
        let status_vtable = module_base.checked_add(TECH_STATUS_VTABLE_RVA)?;
        let slot_vtable = module_base.checked_add(RESEARCH_SLOT_VTABLE_RVA)?;
        let tech_vtables = [
            module_base.checked_add(TECHNOLOGY_VTABLE_RVA_PRIMARY)?,
            module_base.checked_add(TECHNOLOGY_VTABLE_RVA_SECONDARY)?,
        ];
        let status_ptr = read_u64(object_ptr + OBJ_TECH_STATUS)? as usize;
        if !is_readable_range(status_ptr, TECH_STATUS_COUNTRY_BACKLINK + 8) {
            return None;
        }
        if read_u64(status_ptr)? as usize != status_vtable {
            return None;
        }
        if read_u64(status_ptr + TECH_STATUS_COUNTRY_BACKLINK)? as usize != object_ptr {
            return None;
        }

        let completed_tech_count = read_u32_stable(status_ptr + TECH_STATUS_RESEARCHED_SIZE)
            .filter(|count| *count <= MAX_COMPLETED_TECH_COUNT)
            .and_then(|count| {
                let cap = read_u32_stable(status_ptr + TECH_STATUS_RESEARCHED_CAP)?;
                if cap < count || cap > MAX_COMPLETED_TECH_COUNT {
                    return None;
                }
                if count == 0 {
                    return Some(count);
                }
                let data = read_u64(status_ptr + TECH_STATUS_RESEARCHED_DATA)? as usize;
                let bytes = (count as usize).checked_mul(8)?;
                if is_readable_range(data, bytes) {
                    Some(count)
                } else {
                    None
                }
            });
        let research_slot_count = read_u32_stable(status_ptr + TECH_STATUS_RESEARCH_SLOT_COUNT)
            .filter(|count| *count <= MAX_RESEARCH_SLOT_COUNT);
        let active_research = research_slot_count.and_then(|count| {
            read_country_active_research_slots(status_ptr, slot_vtable, tech_vtables, count)
        });

        let row = CountryResearchRow {
            completed_tech_count,
            research_slot_count,
            active_research,
        };
        if row.has_any() {
            Some(row)
        } else {
            None
        }
    }

    fn read_country_active_research_slots(
        status_ptr: usize,
        slot_vtable: usize,
        tech_vtables: [usize; 2],
        slot_count: u32,
    ) -> Option<Vec<CountryResearchSlotRow>> {
        let cap = read_u32_stable(status_ptr + TECH_STATUS_RESEARCH_SLOTS_CAP)?;
        if cap < slot_count || cap > MAX_RESEARCH_SLOT_COUNT {
            return None;
        }
        if slot_count == 0 {
            return Some(Vec::new());
        }
        let data = read_u64_stable(status_ptr + TECH_STATUS_RESEARCH_SLOTS_DATA)? as usize;
        let bytes = (slot_count as usize).checked_mul(8)?;
        if !is_readable_range(data, bytes) {
            return None;
        }

        let mut active = Vec::new();
        let mut seen_slot_indices = 0u64;
        for idx in 0..slot_count as usize {
            let slot_ptr = read_u64(data + idx * 8)? as usize;
            if !is_readable_range(slot_ptr, RESEARCH_SLOT_INDEX + 4) {
                return None;
            }
            if read_u64(slot_ptr)? as usize != slot_vtable {
                return None;
            }
            if read_u64(slot_ptr + RESEARCH_SLOT_OWNER)? as usize != status_ptr {
                return None;
            }
            let slot_index = read_u32_stable(slot_ptr + RESEARCH_SLOT_INDEX)?;
            if slot_index >= slot_count || slot_index >= MAX_RESEARCH_SLOT_COUNT {
                return None;
            }
            let bit = 1u64.checked_shl(slot_index)?;
            if (seen_slot_indices & bit) != 0 {
                return None;
            }
            seen_slot_indices |= bit;

            let handle = read_u32_stable(slot_ptr + RESEARCH_SLOT_TECH_HANDLE)?;
            let tech_ptr = read_u64_stable(slot_ptr + RESEARCH_SLOT_TECH_PTR)? as usize;
            if handle == RESEARCH_SLOT_IDLE_HANDLE || tech_ptr == 0 {
                continue;
            }
            if !is_readable_range(tech_ptr, TECHNOLOGY_DEF_INDEX + 4) {
                continue;
            }
            let tech_vtable = read_u64(tech_ptr)? as usize;
            if tech_vtable != tech_vtables[0] && tech_vtable != tech_vtables[1] {
                continue;
            }
            let def_index = read_u32_stable(tech_ptr + TECHNOLOGY_DEF_INDEX)?;
            if def_index != handle {
                continue;
            }
            let progress = read_u32_stable(slot_ptr + RESEARCH_SLOT_PROGRESS)?;
            active.push(CountryResearchSlotRow {
                slot_index,
                tech_handle_raw: handle,
                tech_def_index_raw: def_index,
                progress_raw: progress,
            });
        }
        active.sort_by_key(|slot| slot.slot_index);
        Some(active)
    }

    fn country_object_political_power(
        object_ptr: usize,
        country_vtable: usize,
        record_ptr: Option<usize>,
    ) -> Option<f64> {
        let module_base = country_vtable.checked_sub(COUNTRY_VTABLE_RVA)?;
        let political_status_vtable = module_base.checked_add(POLITICAL_STATUS_VTABLE_RVA)?;
        if let Some(value) = country_political_status_power(object_ptr, political_status_vtable) {
            return Some(value);
        }
        record_ptr.and_then(|record_ptr| country_record_political_power(record_ptr, object_ptr))
    }

    fn country_political_status_power(
        object_ptr: usize,
        political_status_vtable: usize,
    ) -> Option<f64> {
        let status_ptr = validated_political_status_ptr(object_ptr, political_status_vtable)?;
        let value =
            read_i32_stable(status_ptr + POLITICAL_STATUS_POLITICAL_POWER)? as f64 / FP_SCALE;
        if (-1_000_000.0..=1_000_000.0).contains(&value) {
            Some(value)
        } else {
            None
        }
    }

    fn validated_political_status_ptr(
        object_ptr: usize,
        political_status_vtable: usize,
    ) -> Option<usize> {
        let status_ptr = read_u64(object_ptr + OBJ_POLITICAL_STATUS)? as usize;
        if !is_map_control_heap_pointer(status_ptr) {
            return None;
        }
        if read_u64(status_ptr)? as usize != political_status_vtable {
            return None;
        }
        if read_u64(status_ptr + POLITICAL_STATUS_COUNTRY_BACKLINK)? as usize != object_ptr {
            return None;
        }
        Some(status_ptr)
    }

    fn read_country_politics(
        object_ptr: usize,
        country_vtable: usize,
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> Option<CountryPoliticsRow> {
        let module_base = country_vtable.checked_sub(COUNTRY_VTABLE_RVA)?;
        let status_vtable = module_base.checked_add(POLITICAL_STATUS_VTABLE_RVA)?;
        let party_vtable = module_base.checked_add(POLITICAL_PARTY_VTABLE_RVA)?;
        let ideology_group_vtable = module_base.checked_add(IDEOLOGY_GROUP_VTABLE_RVA)?;
        let idea_vtable_primary = module_base.checked_add(IDEA_VTABLE_RVA_PRIMARY)?;
        let idea_vtable_secondary = module_base.checked_add(IDEA_VTABLE_RVA_SECONDARY)?;
        let status_ptr = validated_political_status_ptr(object_ptr, status_vtable)?;

        let parties = read_political_parties(status_ptr, party_vtable, ideology_group_vtable);
        let ruling_party_ptr =
            read_u64(status_ptr + POLITICAL_STATUS_RULING_PARTY).unwrap_or(0) as usize;
        let ruling_party =
            read_political_party_ideology(ruling_party_ptr, party_vtable, ideology_group_vtable);
        let ideas = read_active_ideas(status_ptr, idea_vtable_primary, idea_vtable_secondary);
        let election_frequency = read_u32_stable(status_ptr + POLITICAL_STATUS_ELECTION_FREQUENCY)
            .filter(|v| (1..=600).contains(v));

        let party_count = parties.as_ref().map(|read| read.party_count);
        let low_confidence = parties.as_ref().map(|read| {
            read.party_count != 4
                || !is_map_control_heap_pointer(ruling_party_ptr)
                || !read.party_ptrs.contains(&ruling_party_ptr)
        });
        let popularities = parties.map(|read| read.popularities);
        let idea_count = ideas.as_ref().map(|values| values.len() as u32);
        let laws = ideas
            .as_ref()
            .and_then(|values| classify_idea_laws(values, idea_group_filter));
        let ideas_other = ideas
            .as_ref()
            .and_then(|values| classify_ideas_other(values, idea_group_filter));
        let national_spirits = ideas
            .as_ref()
            .and_then(|values| classify_national_spirits(values, idea_group_filter));

        let row = CountryPoliticsRow {
            popularities,
            ruling_party,
            ideas,
            laws,
            ideas_other,
            national_spirits,
            idea_count,
            election_frequency,
            party_count,
            low_confidence,
        };
        if row.has_any() {
            Some(row)
        } else {
            None
        }
    }

    fn classify_idea_laws(
        ideas: &[String],
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> Option<BTreeMap<String, String>> {
        let groups = idea_group_filter?;
        if groups.is_empty() {
            return None;
        }
        let mut laws = BTreeMap::new();
        for idea in ideas {
            if let Some(group) = groups.get(idea) {
                if is_law_group(group) {
                    laws.insert(group.clone(), idea.clone());
                }
            }
        }
        if laws.is_empty() {
            None
        } else {
            Some(laws)
        }
    }

    fn classify_ideas_other(
        ideas: &[String],
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> Option<Vec<String>> {
        let groups = idea_group_filter?;
        if groups.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for idea in ideas {
            if groups
                .get(idea)
                .map(|group| !is_law_group(group))
                .unwrap_or(true)
            {
                out.push(idea.clone());
            }
        }
        Some(out)
    }

    fn is_law_group(group: &str) -> bool {
        matches!(group, "economy" | "trade_laws" | "mobilization_laws")
    }

    fn classify_national_spirits(
        ideas: &[String],
        idea_group_filter: Option<&BTreeMap<String, String>>,
    ) -> Option<Vec<String>> {
        let groups = idea_group_filter?;
        if groups.is_empty() {
            return None;
        }
        let out: Vec<String> = ideas
            .iter()
            .filter(|idea| {
                groups
                    .get(*idea)
                    .map(|group| group == "country")
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        Some(out)
    }

    fn read_country_politics_residuals(
        object_ptr: usize,
        country_vtable: usize,
        advisor_token_filter: Option<&BTreeMap<String, AdvisorTokenInfo>>,
    ) -> Option<CountryPoliticsResidualsRow> {
        let module_base = country_vtable.checked_sub(COUNTRY_VTABLE_RVA)?;
        let status_vtable = module_base.checked_add(POLITICAL_STATUS_VTABLE_RVA)?;
        let idea_vtable_primary = module_base.checked_add(IDEA_VTABLE_RVA_PRIMARY)?;
        let idea_vtable_secondary = module_base.checked_add(IDEA_VTABLE_RVA_SECONDARY)?;
        let status_ptr = validated_political_status_ptr(object_ptr, status_vtable)?;

        let last_election =
            read_i32_stable(status_ptr + POLITICAL_STATUS_LAST_ELECTION).and_then(decode_datehour);
        let elections_allowed = read_u8_stable(status_ptr + POLITICAL_STATUS_ELECTIONS_ALLOWED)
            .and_then(|value| {
                if value == 0 {
                    Some(false)
                } else if value == 1 {
                    Some(true)
                } else {
                    None
                }
            });
        let (appointed_advisors, appointed_advisor_details, appointed_advisor_slots) =
            match advisor_token_filter {
                Some(tokens) if !tokens.is_empty() => {
                    match read_active_ideas(status_ptr, idea_vtable_primary, idea_vtable_secondary)
                    {
                        Some(ideas) => {
                            let details = appointed_advisor_rows(&ideas, tokens);
                            let advisors = details
                                .iter()
                                .map(|row| row.idea_token.clone())
                                .collect::<Vec<String>>();
                            let slots = appointed_advisor_slots_from_rows(&details);
                            (Some(advisors), Some(details), Some(slots))
                        }
                        None => (None, None, None),
                    }
                }
                _ => (None, None, None),
            };

        let row = CountryPoliticsResidualsRow {
            last_election,
            elections_allowed,
            appointed_advisors,
            appointed_advisor_details,
            appointed_advisor_slots,
        };
        if row.has_any() {
            Some(row)
        } else {
            None
        }
    }

    fn appointed_advisor_rows(
        ideas: &[String],
        advisor_token_filter: &BTreeMap<String, AdvisorTokenInfo>,
    ) -> Vec<AppointedAdvisorRow> {
        let mut rows = Vec::new();
        for idea in ideas {
            if let Some(info) = advisor_token_filter.get(idea) {
                rows.push(AppointedAdvisorRow {
                    idea_token: idea.clone(),
                    slot: info.slot.clone(),
                    ledger: info.ledger.clone(),
                    character: info.character.clone(),
                });
            }
        }
        rows
    }

    fn appointed_advisor_slots_from_rows(
        rows: &[AppointedAdvisorRow],
    ) -> BTreeMap<String, Vec<String>> {
        let mut slots: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in rows {
            if let Some(slot) = row.slot.as_ref() {
                slots
                    .entry(slot.clone())
                    .or_default()
                    .push(row.idea_token.clone());
            }
        }
        slots
    }

    fn read_country_decisions(
        object_ptr: usize,
        country_vtable: usize,
    ) -> Option<CountryDecisionsRow> {
        let module_base = country_vtable.checked_sub(COUNTRY_VTABLE_RVA)?;
        let status_vtable = module_base.checked_add(DECISION_STATUS_VTABLE_RVA)?;
        let timed_decision_vtable = module_base.checked_add(TIMED_DECISION_VTABLE_RVA)?;
        let status_ptr = read_u64(object_ptr + OBJ_DECISION_STATUS)? as usize;
        if !is_map_control_heap_pointer(status_ptr) {
            return None;
        }
        if read_u64(status_ptr)? as usize != status_vtable {
            return None;
        }
        if read_u64(status_ptr + DECISION_STATUS_OWNER)? as usize != object_ptr {
            return None;
        }

        let status_present = decision_status_present(status_ptr);
        let timed_live = read_active_timed_decisions(status_ptr, timed_decision_vtable);
        let row = CountryDecisionsRow {
            status_present,
            timed_live,
        };
        if row.has_any() {
            Some(row)
        } else {
            None
        }
    }

    fn decision_status_present(status_ptr: usize) -> Option<BTreeMap<String, u32>> {
        let mut present = BTreeMap::new();
        for (key, offset) in [
            ("active_timed_decision", DECISION_STATUS_ACTIVE_TIMED),
            ("active_targeted_decision", DECISION_STATUS_TARGETED),
            (
                "active_targeted_timed_decision",
                DECISION_STATUS_TARGETED_TIMED,
            ),
            ("decisions_taken_bucket", DECISION_STATUS_RE_ENABLE),
            ("decision_to_re_enable", DECISION_STATUS_RE_ENABLE_ALT),
        ] {
            if let Some(size) = decision_vector_size(status_ptr + offset) {
                if size > 0 {
                    present.insert(key.to_string(), size);
                }
            }
        }
        if present.is_empty() {
            None
        } else {
            Some(present)
        }
    }

    fn decision_vector_size(header_addr: usize) -> Option<u32> {
        let (data, size, cap) = read_stable_vector_header(header_addr, 0, 0x0C, 0x08)?;
        if size > cap || cap > MAX_DECISION_STATUS_LIST {
            return None;
        }
        if size == 0 {
            return Some(0);
        }
        if is_map_control_heap_pointer(data) {
            Some(size)
        } else {
            None
        }
    }

    fn read_active_timed_decisions(
        status_ptr: usize,
        timed_decision_vtable: usize,
    ) -> Option<Vec<TimedDecisionRow>> {
        let header_addr = status_ptr.checked_add(DECISION_STATUS_ACTIVE_TIMED)?;
        let (data, size, cap) = read_stable_vector_header(header_addr, 0, 0x0C, 0x08)?;
        if size > cap || cap > MAX_DECISION_STATUS_LIST || size as usize > MAX_TIMED_DECISIONS {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }
        if !is_map_control_heap_pointer(data) {
            return None;
        }
        let mut rows = Vec::new();
        for index in 0..size as usize {
            let rec_ptr = read_u64(data + index * 8)? as usize;
            let row = read_timed_decision_record(rec_ptr, timed_decision_vtable)?;
            if row.state == DECISION_STATE_ACTIVE {
                rows.push(row);
            }
        }
        Some(rows)
    }

    fn read_timed_decision_record(
        rec_ptr: usize,
        timed_decision_vtable: usize,
    ) -> Option<TimedDecisionRow> {
        if !is_map_control_heap_pointer(rec_ptr) {
            return None;
        }
        if read_u64(rec_ptr)? as usize != timed_decision_vtable {
            return None;
        }
        let def_ptr = read_u64(rec_ptr + TIMED_DECISION_DEF)? as usize;
        if !is_map_control_heap_pointer(def_ptr) {
            return None;
        }
        let decision = read_msvc_string(def_ptr + DECISION_DEF_NAME, MAX_POLITICS_TOKEN_LEN)?;
        if !is_script_token(&decision) {
            return None;
        }
        let days = read_i32_stable(rec_ptr + TIMED_DECISION_DAYS)?;
        if !(0..=MAX_DECISION_DAYS).contains(&days) {
            return None;
        }
        let state = read_i32_stable(rec_ptr + TIMED_DECISION_STATE)?;
        if !(0..=16).contains(&state) {
            return None;
        }
        Some(TimedDecisionRow {
            decision,
            days,
            state,
            state_name: decision_state_name(state).to_string(),
        })
    }

    fn decision_state_name(state: i32) -> &'static str {
        match state {
            0 => "active",
            4 => "re_enable_cooldown",
            _ => "unknown",
        }
    }

    fn read_stable_vector_header(
        base: usize,
        data_offset: usize,
        size_offset: usize,
        cap_offset: usize,
    ) -> Option<(usize, u32, u32)> {
        let first = (
            read_u64(base + data_offset)? as usize,
            read_u32(base + size_offset)?,
            read_u32(base + cap_offset)?,
        );
        let second = (
            read_u64(base + data_offset)? as usize,
            read_u32(base + size_offset)?,
            read_u32(base + cap_offset)?,
        );
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_political_parties(
        status_ptr: usize,
        party_vtable: usize,
        ideology_group_vtable: usize,
    ) -> Option<CountryPartyRead> {
        let (data, size, cap) = read_stable_vector_header(
            status_ptr,
            POLITICAL_STATUS_PARTIES_DATA,
            POLITICAL_STATUS_PARTIES_SIZE,
            POLITICAL_STATUS_PARTIES_CAP,
        )?;
        if !is_map_control_heap_pointer(data)
            || size == 0
            || size > MAX_POLITICAL_PARTIES
            || size > cap
        {
            return None;
        }
        let mut observed = BTreeMap::new();
        let mut party_ptrs = Vec::new();
        for index in 0..size as usize {
            let party_ptr = read_u64(data + index * 8)? as usize;
            if !is_map_control_heap_pointer(party_ptr) {
                return None;
            }
            let ideology =
                read_political_party_ideology(party_ptr, party_vtable, ideology_group_vtable)?;
            if observed.contains_key(&ideology) {
                return None;
            }
            let popularity =
                read_i32_stable(party_ptr + POLITICAL_PARTY_POPULARITY)? as f64 / FP_SCALE;
            if !popularity.is_finite() || !(0.0..=100.0001).contains(&popularity) {
                return None;
            }
            observed.insert(ideology, popularity);
            party_ptrs.push(party_ptr);
        }

        let mut popularities = BTreeMap::new();
        for ideology in ["democratic", "communism", "fascism", "neutrality"] {
            popularities.insert(
                ideology.to_string(),
                *observed.get(ideology).unwrap_or(&0.0),
            );
        }
        Some(CountryPartyRead {
            popularities,
            party_ptrs,
            party_count: size,
        })
    }

    fn read_political_party_ideology(
        party_ptr: usize,
        party_vtable: usize,
        ideology_group_vtable: usize,
    ) -> Option<String> {
        if !is_map_control_heap_pointer(party_ptr) || read_u64(party_ptr)? as usize != party_vtable
        {
            return None;
        }
        let group_ptr = read_u64(party_ptr + POLITICAL_PARTY_IDEOLOGY_GROUP)? as usize;
        if !is_map_control_heap_pointer(group_ptr)
            || read_u64(group_ptr)? as usize != ideology_group_vtable
        {
            return None;
        }
        let ideology = read_msvc_string(group_ptr + IDEOLOGY_GROUP_ID, MAX_POLITICS_TOKEN_LEN)?;
        if is_ideology_group_token(&ideology) {
            Some(ideology)
        } else {
            None
        }
    }

    fn read_active_ideas(
        status_ptr: usize,
        idea_vtable_primary: usize,
        idea_vtable_secondary: usize,
    ) -> Option<Vec<String>> {
        let (data, size, cap) = read_stable_vector_header(
            status_ptr,
            POLITICAL_STATUS_IDEAS_DATA,
            POLITICAL_STATUS_IDEAS_SIZE,
            POLITICAL_STATUS_IDEAS_CAP,
        )?;
        if size > cap || size > MAX_ACTIVE_IDEAS {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }
        if !is_map_control_heap_pointer(data) {
            return None;
        }
        let mut ideas = Vec::new();
        for index in 0..size as usize {
            let idea_ptr = read_u64(data + index * 8)? as usize;
            if !is_map_control_heap_pointer(idea_ptr) {
                return None;
            }
            let vtable = read_u64(idea_ptr)? as usize;
            if vtable != idea_vtable_primary && vtable != idea_vtable_secondary {
                return None;
            }
            let idea = read_msvc_string(idea_ptr + IDEA_ID, MAX_POLITICS_TOKEN_LEN)?;
            if !is_script_token(&idea) {
                return None;
            }
            ideas.push(idea);
        }
        Some(ideas)
    }

    fn is_ideology_group_token(value: &str) -> bool {
        matches!(value, "democratic" | "communism" | "fascism" | "neutrality")
    }

    fn is_script_token(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= MAX_POLITICS_TOKEN_LEN
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || byte == b'_'
                    || byte == b'-'
                    || byte == b'.'
                    || byte == b':'
            })
    }

    fn read_country_fuel(object_ptr: usize) -> Option<CountryFuelRow> {
        let fuel_status = read_u64(object_ptr + OBJ_FUEL_STATUS)? as usize;
        if !is_map_control_heap_pointer(fuel_status) {
            return None;
        }
        let current = read_i64(fuel_status + FUEL_STATUS_CURRENT)? as f64 / FUEL_SCALE;
        let max = read_i64(fuel_status + FUEL_STATUS_MAX)? as f64 / FUEL_SCALE;
        let gain = read_i64(fuel_status + FUEL_STATUS_GAIN)? as f64 / FUEL_SCALE;
        let cost = read_i64(fuel_status + FUEL_STATUS_COST)? as f64 / FUEL_SCALE;
        if valid_nonnegative_fuel_measure(current)
            && valid_nonnegative_fuel_measure(max)
            && valid_signed_fuel_measure(gain)
            && valid_signed_fuel_measure(cost)
        {
            Some(CountryFuelRow {
                current,
                max,
                gain,
                cost,
            })
        } else {
            None
        }
    }

    fn valid_nonnegative_fuel_measure(value: f64) -> bool {
        value.is_finite() && (0.0..=MAX_FUEL_MEASURE).contains(&value)
    }

    fn valid_signed_fuel_measure(value: f64) -> bool {
        value.is_finite() && value.abs() <= MAX_FUEL_MEASURE
    }

    fn read_country_production(
        object_ptr: usize,
        country_vtable: usize,
    ) -> Option<CountryProductionRow> {
        let base = country_vtable.checked_sub(COUNTRY_VTABLE_RVA)?;
        let variant_vtable = base.checked_add(VARIANT_VTABLE_RVA)?;
        let manager = read_u64(object_ptr + OBJ_PROD_MANAGER)? as usize;
        if !is_map_control_heap_pointer(manager) {
            return None;
        }
        let stockpile = read_production_stockpile(manager, variant_vtable);
        let lines = read_production_lines(manager);
        if stockpile.is_none() && lines.is_none() {
            return None;
        }
        let (military_lines, naval_lines) = match lines {
            Some((military_lines, naval_lines)) => (Some(military_lines), Some(naval_lines)),
            None => (None, None),
        };
        Some(CountryProductionRow {
            stockpile,
            military_lines,
            naval_lines,
        })
    }

    fn read_production_stockpile(
        manager: usize,
        variant_vtable: usize,
    ) -> Option<Vec<CountryStockpileRow>> {
        let begin = read_u64(manager + STOCK_BEGIN)? as usize;
        let cap = read_u32(manager + STOCK_CAP)?;
        let size = read_u32(manager + STOCK_SIZE)?;
        if size > cap || cap > MAX_PRODUCTION_VECTOR_ENTRIES {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }
        if !is_map_control_heap_pointer(begin) {
            return None;
        }
        let byte_len = (size as usize).checked_mul(STOCK_STRIDE)?;
        let raw = read_bytes(begin, byte_len)?;
        let mut rows_by_variant: BTreeMap<u32, f64> = BTreeMap::new();
        for k in (0..byte_len).step_by(STOCK_STRIDE) {
            let def_ptr = read_u64_from_slice(&raw, k + STOCK_ENTRY_DEF)? as usize;
            let amount_raw = read_i64_from_slice(&raw, k + STOCK_ENTRY_AMOUNT)?;
            if !is_map_control_heap_pointer(def_ptr) {
                continue;
            }
            let def_vtable = read_u64(def_ptr)? as usize;
            let def_type = read_u32(def_ptr + VARIANT_DEF_TYPE)?;
            if def_vtable != variant_vtable || def_type != VARIANT_TYPE {
                return None;
            }
            let variant_id = read_u32(def_ptr + VARIANT_DEF_ID)?;
            let amount = amount_raw as f64 / FP_SCALE;
            if !amount.is_finite() || amount.abs() > MAX_PRODUCTION_STOCKPILE_AMOUNT {
                return None;
            }
            if rows_by_variant.insert(variant_id, amount).is_some() {
                return None;
            }
        }
        Some(
            rows_by_variant
                .into_iter()
                .map(|(variant_id, amount)| CountryStockpileRow { variant_id, amount })
                .collect(),
        )
    }

    fn read_production_lines(
        manager: usize,
    ) -> Option<(Vec<CountryProductionLineRow>, Vec<CountryProductionLineRow>)> {
        let begin = read_u64(manager + LINE_BEGIN)? as usize;
        let cap = read_u32(manager + LINE_CAP)?;
        let size = read_u32(manager + LINE_SIZE)?;
        if size > cap || cap > MAX_PRODUCTION_VECTOR_ENTRIES {
            return None;
        }
        if size == 0 {
            return Some((Vec::new(), Vec::new()));
        }
        if !is_map_control_heap_pointer(begin) {
            return None;
        }
        let byte_len = (size as usize).checked_mul(LINE_STRIDE)?;
        let raw = read_bytes(begin, byte_len)?;
        let mut military = Vec::new();
        let mut naval = Vec::new();
        for k in (0..byte_len).step_by(LINE_STRIDE) {
            let line_ptr = read_u64_from_slice(&raw, k)? as usize;
            if !is_map_control_heap_pointer(line_ptr) {
                continue;
            }
            let line_type = read_u32(line_ptr + LINE_TYPE)?;
            if line_type != LINE_TYPE_MILITARY && line_type != LINE_TYPE_NAVAL {
                continue;
            }
            let active_factories = read_i32(line_ptr + LINE_ACTIVE_FACTORIES)?;
            if !(0..=100_000).contains(&active_factories) {
                continue;
            }
            let Some(cost) = read_production_fixed(line_ptr + LINE_COST) else {
                continue;
            };
            let Some(speed) = read_production_fixed(line_ptr + LINE_SPEED) else {
                continue;
            };
            let Some(produced) = read_production_fixed(line_ptr + LINE_PRODUCED) else {
                continue;
            };
            let line = CountryProductionLineRow {
                id: read_i32(line_ptr + LINE_ID)?,
                active_factories,
                cost,
                speed,
                produced,
                amount: read_i32(line_ptr + LINE_AMOUNT)?,
            };
            if line_type == LINE_TYPE_MILITARY {
                military.push(line);
            } else {
                naval.push(line);
            }
        }
        Some((military, naval))
    }

    fn read_production_fixed(addr: usize) -> Option<f64> {
        read_i32(addr)
            .map(|v| v as f64 / FP_SCALE)
            .filter(|v| v.is_finite() && v.abs() <= MAX_PRODUCTION_LINE_FIXED)
    }

    fn row_json(
        row: &CountryObjectRow,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
        include_scheduled_country_focus: bool,
        include_scheduled_country_research: bool,
        include_scheduled_country_politics: bool,
        include_scheduled_country_politics_residuals: bool,
        include_scheduled_country_decisions: bool,
    ) -> String {
        let original_capital = json_opt_i32(row.original_capital);
        let original_tag = json_opt_str(row.original_tag.as_deref());
        let templates_locked = json_opt_bool(row.templates_locked);
        let reserved_dynamic_country = json_opt_bool(row.reserved_dynamic_country);
        let stability = json_opt_f64(row.stability);
        let war_support = json_opt_f64(row.war_support);
        let political_power = json_opt_f64(row.political_power);
        let command_power = json_opt_f64(row.command_power);
        let num_ships = json_opt_i32(row.num_ships);
        let manpower_pool = json_opt_u32(row.manpower_pool);
        let convoys = json_opt_f64(row.convoys);
        let misc = row.misc.as_ref().map(country_misc_json).unwrap_or_default();
        let fuel = if include_scheduled_country_fuel {
            row.fuel
                .map(|fuel| {
                    format!(
                        ",\"fuel\":{{\"current\":{},\"max\":{},\"gain\":{},\"cost\":{}}}",
                        json_opt_f64(Some(fuel.current)),
                        json_opt_f64(Some(fuel.max)),
                        json_opt_f64(Some(fuel.gain)),
                        json_opt_f64(Some(fuel.cost))
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let production = if include_scheduled_country_production {
            row.production
                .as_ref()
                .map(country_production_json)
                .unwrap_or_default()
        } else {
            String::new()
        };
        let focus = if include_scheduled_country_focus {
            row.focus
                .as_ref()
                .map(country_focus_json)
                .unwrap_or_default()
        } else {
            String::new()
        };
        let research = if include_scheduled_country_research {
            row.research
                .as_ref()
                .map(country_research_json)
                .unwrap_or_default()
        } else {
            String::new()
        };
        let politics = country_politics_json(
            if include_scheduled_country_politics {
                row.politics.as_ref()
            } else {
                None
            },
            if include_scheduled_country_politics_residuals {
                row.politics_residuals.as_ref()
            } else {
                None
            },
        );
        let decisions = if include_scheduled_country_decisions {
            row.decisions
                .as_ref()
                .map(country_decisions_json)
                .unwrap_or_default()
        } else {
            String::new()
        };
        format!(
            "\"{}\":{{\"present\":true,\
             \"collector_object\":\"0x{:X}\",\
             \"identity\":{{\"tag_db_id\":{},\
             \"original_capital\":{original_capital},\
             \"original_tag\":{original_tag},\
             \"reserved_dynamic_country\":{reserved_dynamic_country},\
             \"templates_locked\":{templates_locked}}},\
             \"scalars\":{{\"stability\":{stability},\
             \"war_support\":{war_support},\
             \"political_power\":{political_power},\
             \"command_power\":{command_power},\
             \"num_ships\":{num_ships},\
             \"manpower_pool\":{manpower_pool},\
             \"convoys\":{convoys}}}{misc}{fuel}{production}{focus}{research}{politics}{decisions}}}",
            row.tag, row.object_ptr, row.tag_db_id
        )
    }

    fn country_research_json(research: &CountryResearchRow) -> String {
        let mut fields = Vec::new();
        if let Some(completed_tech_count) = research.completed_tech_count {
            fields.push(format!("\"completed_tech_count\":{completed_tech_count}"));
        }
        if let Some(research_slot_count) = research.research_slot_count {
            fields.push(format!("\"research_slot_count\":{research_slot_count}"));
        }
        if let Some(active_research) = research.active_research.as_ref() {
            let rows = active_research
                .iter()
                .map(|slot| {
                    format!(
                        "{{\"slot_index\":{},\"tech_handle_raw\":{},\
                         \"tech_def_index_raw\":{},\"progress_raw\":{}}}",
                        slot.slot_index,
                        slot.tech_handle_raw,
                        slot.tech_def_index_raw,
                        slot.progress_raw
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            fields.push(format!("\"active_research\":[{rows}]"));
        }
        if fields.is_empty() {
            String::new()
        } else {
            format!(",\"research\":{{{}}}", fields.join(","))
        }
    }

    fn country_focus_json(focus: &CountryFocusRow) -> String {
        let mut fields = Vec::new();
        if let Some(focus_tree) = focus.focus_tree.as_ref() {
            fields.push(format!("\"focus_tree\":{}", json_str(focus_tree)));
        }
        if let Some(completed) = focus.completed.as_ref() {
            let parts: Vec<String> = completed.iter().map(|token| json_str(token)).collect();
            fields.push(format!("\"completed\":[{}]", parts.join(",")));
        }
        if let Some(completed_count) = focus.completed_count {
            fields.push(format!("\"completed_count\":{completed_count}"));
        }
        if let Some(current) = focus.current.as_ref() {
            fields.push(format!("\"current\":{}", json_str(current)));
        }
        if let Some(current_continuous) = focus.current_continuous.as_ref() {
            fields.push(format!(
                "\"current_continuous\":{}",
                json_str(current_continuous)
            ));
        }
        if let Some(progress) = focus.progress {
            fields.push(format!("\"progress\":{}", json_opt_f64(Some(progress))));
        }
        if fields.is_empty() {
            String::new()
        } else {
            format!(",\"focus\":{{{}}}", fields.join(","))
        }
    }

    fn country_misc_json(misc: &CountryMiscRow) -> String {
        let mut fields = Vec::new();
        if let Some(value) = misc.convoys_destroyed {
            fields.push(format!("\"convoys_destroyed\":{value}"));
        }
        if let Some(value) = misc.accidents_score {
            fields.push(format!("\"accidents_score\":{}", json_opt_f64(Some(value))));
        }
        if let Some(value) = misc.ace_count {
            fields.push(format!("\"ace_count\":{value}"));
        }
        if let Some(value) = &misc.cosmetic_tag {
            fields.push(format!(
                "\"cosmetic_tag\":{}",
                json_opt_str(value.as_deref())
            ));
        }
        if fields.is_empty() {
            String::new()
        } else {
            format!(",\"misc\":{{{}}}", fields.join(","))
        }
    }

    fn country_production_json(production: &CountryProductionRow) -> String {
        let mut fields = Vec::new();
        if let Some(stockpile) = production.stockpile.as_deref() {
            let stockpile_total: f64 = stockpile.iter().map(|row| row.amount).sum();
            fields.push(format!(
                "\"stockpile\":{}",
                production_stockpile_json(stockpile)
            ));
            fields.push(format!(
                "\"stockpile_total\":{}",
                json_opt_f64(Some(stockpile_total))
            ));
        }
        if let Some(military_lines) = production.military_lines.as_deref() {
            let military_factories: i32 = military_lines
                .iter()
                .map(|line| line.active_factories)
                .sum();
            fields.push(format!(
                "\"military_lines\":{}",
                production_lines_json(military_lines)
            ));
            fields.push(format!("\"military_factories\":{military_factories}"));
        }
        if let Some(naval_lines) = production.naval_lines.as_deref() {
            let naval_factories: i32 = naval_lines.iter().map(|line| line.active_factories).sum();
            fields.push(format!(
                "\"naval_lines\":{}",
                production_lines_json(naval_lines)
            ));
            fields.push(format!("\"naval_factories\":{naval_factories}"));
        }
        format!(",\"production\":{{{}}}", fields.join(","))
    }

    fn production_stockpile_json(rows: &[CountryStockpileRow]) -> String {
        let parts: Vec<String> = rows
            .iter()
            .map(|row| format!("\"{}\":{}", row.variant_id, json_opt_f64(Some(row.amount))))
            .collect();
        format!("{{{}}}", parts.join(","))
    }

    fn production_lines_json(rows: &[CountryProductionLineRow]) -> String {
        let parts: Vec<String> = rows
            .iter()
            .map(|line| {
                format!(
                    "{{\"id\":{},\"active_factories\":{},\
                     \"cost\":{},\"speed\":{},\"produced\":{},\"amount\":{}}}",
                    line.id,
                    line.active_factories,
                    json_opt_f64(Some(line.cost)),
                    json_opt_f64(Some(line.speed)),
                    json_opt_f64(Some(line.produced)),
                    line.amount
                )
            })
            .collect();
        format!("[{}]", parts.join(","))
    }

    fn country_politics_json(
        politics: Option<&CountryPoliticsRow>,
        residuals: Option<&CountryPoliticsResidualsRow>,
    ) -> String {
        let mut fields = Vec::new();
        if let Some(politics) = politics {
            if let Some(popularities) = politics.popularities.as_ref() {
                let parts: Vec<String> = popularities
                    .iter()
                    .map(|(ideology, value)| {
                        format!("{}:{}", json_str(ideology), json_opt_f64(Some(*value)))
                    })
                    .collect();
                fields.push(format!("\"popularities\":{{{}}}", parts.join(",")));
            }
            if let Some(ruling_party) = politics.ruling_party.as_ref() {
                fields.push(format!("\"ruling_party\":{}", json_str(ruling_party)));
            }
            if let Some(ideas) = politics.ideas.as_ref() {
                let parts: Vec<String> = ideas.iter().map(|idea| json_str(idea)).collect();
                fields.push(format!("\"ideas\":[{}]", parts.join(",")));
            }
            if let Some(laws) = politics.laws.as_ref() {
                let parts: Vec<String> = laws
                    .iter()
                    .map(|(group, token)| format!("{}:{}", json_str(group), json_str(token)))
                    .collect();
                fields.push(format!("\"laws\":{{{}}}", parts.join(",")));
            }
            if let Some(ideas_other) = politics.ideas_other.as_ref() {
                let parts: Vec<String> = ideas_other.iter().map(|idea| json_str(idea)).collect();
                fields.push(format!("\"ideas_other\":[{}]", parts.join(",")));
            }
            if let Some(national_spirits) = politics.national_spirits.as_ref() {
                let parts: Vec<String> =
                    national_spirits.iter().map(|idea| json_str(idea)).collect();
                fields.push(format!("\"national_spirits\":[{}]", parts.join(",")));
            }
            if let Some(idea_count) = politics.idea_count {
                fields.push(format!("\"idea_count\":{idea_count}"));
            }
            if let Some(election_frequency) = politics.election_frequency {
                fields.push(format!("\"election_frequency\":{election_frequency}"));
            }
            if let Some(party_count) = politics.party_count {
                fields.push(format!("\"party_count\":{party_count}"));
            }
            if let Some(low_confidence) = politics.low_confidence {
                fields.push(format!("\"low_confidence\":{low_confidence}"));
            }
        }
        if let Some(residuals) = residuals {
            if let Some(last_election) = residuals.last_election.as_ref() {
                fields.push(format!("\"last_election\":{}", json_str(last_election)));
            }
            if let Some(elections_allowed) = residuals.elections_allowed {
                fields.push(format!("\"elections_allowed\":{elections_allowed}"));
            }
            if let Some(appointed_advisors) = residuals.appointed_advisors.as_ref() {
                let parts: Vec<String> = appointed_advisors
                    .iter()
                    .map(|token| json_str(token))
                    .collect();
                fields.push(format!("\"appointed_advisors\":[{}]", parts.join(",")));
            }
            if let Some(details) = residuals.appointed_advisor_details.as_ref() {
                let parts: Vec<String> = details
                    .iter()
                    .map(|row| {
                        format!(
                            "{{\"idea_token\":{},\"slot\":{},\"ledger\":{},\"character\":{}}}",
                            json_str(&row.idea_token),
                            json_opt_str(row.slot.as_deref()),
                            json_opt_str(row.ledger.as_deref()),
                            json_opt_str(row.character.as_deref())
                        )
                    })
                    .collect();
                fields.push(format!(
                    "\"appointed_advisor_details\":[{}]",
                    parts.join(",")
                ));
            }
            if let Some(slots) = residuals.appointed_advisor_slots.as_ref() {
                let parts: Vec<String> = slots
                    .iter()
                    .map(|(slot, tokens)| {
                        let tokens_json: Vec<String> =
                            tokens.iter().map(|token| json_str(token)).collect();
                        format!("{}:[{}]", json_str(slot), tokens_json.join(","))
                    })
                    .collect();
                fields.push(format!(
                    "\"appointed_advisor_slots\":{{{}}}",
                    parts.join(",")
                ));
            }
        }
        if fields.is_empty() {
            String::new()
        } else {
            format!(",\"politics\":{{{}}}", fields.join(","))
        }
    }

    fn country_decisions_json(decisions: &CountryDecisionsRow) -> String {
        let mut fields = Vec::new();
        if let Some(status_present) = decisions.status_present.as_ref() {
            let parts: Vec<String> = status_present
                .iter()
                .map(|(key, count)| format!("{}:{}", json_str(key), count))
                .collect();
            fields.push(format!("\"status_present\":{{{}}}", parts.join(",")));
        }
        if let Some(timed_live) = decisions.timed_live.as_ref() {
            let parts: Vec<String> = timed_live
                .iter()
                .map(|row| {
                    format!(
                        "{{\"decision\":{},\"days\":{},\"state\":{},\"state_name\":{}}}",
                        json_str(&row.decision),
                        row.days,
                        row.state,
                        json_str(&row.state_name)
                    )
                })
                .collect();
            fields.push(format!("\"timed_live\":[{}]", parts.join(",")));
        }
        if fields.is_empty() {
            String::new()
        } else {
            format!(",\"decisions\":{{{}}}", fields.join(","))
        }
    }

    fn clock_probe_json(gamestate_ptr: usize) -> String {
        if gamestate_ptr == 0 {
            return "\"current_game_date\":null,\"current_game_date_raw\":null,\
                    \"start_date\":null,\"start_date_raw\":null"
                .to_string();
        }
        let current_raw = read_i32(gamestate_ptr + GS_CURRENT_DATE_OFFSET);
        let start_raw = read_i32(gamestate_ptr + GS_START_DATE_OFFSET);
        let current = current_raw.and_then(decode_datehour);
        let start = start_raw.and_then(decode_datehour);
        format!(
            "\"current_game_date\":{},\
             \"current_game_date_raw\":{},\
             \"start_date\":{},\
             \"start_date_raw\":{}",
            json_opt_str(current.as_deref()),
            json_opt_i32(current_raw),
            json_opt_str(start.as_deref()),
            json_opt_i32(start_raw)
        )
    }

    fn read_i32(addr: usize) -> Option<i32> {
        read_u32(addr).map(|v| v as i32)
    }

    fn read_f32(addr: usize) -> Option<f32> {
        let bytes = read_bytes(addr, 4)?;
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&bytes);
        Some(f32::from_le_bytes(arr))
    }

    fn read_i32_stable(addr: usize) -> Option<i32> {
        let first = read_i32(addr)?;
        let second = read_i32(addr)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_u8_stable(addr: usize) -> Option<u8> {
        let first = read_u8(addr)?;
        let second = read_u8(addr)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_u32_stable(addr: usize) -> Option<u32> {
        let first = read_u32(addr)?;
        let second = read_u32(addr)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_u64_stable(addr: usize) -> Option<u64> {
        let first = read_u64(addr)?;
        let second = read_u64(addr)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_i64(addr: usize) -> Option<i64> {
        let bytes = read_bytes(addr, 8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Some(i64::from_le_bytes(arr))
    }

    fn read_u64_from_slice(bytes: &[u8], offset: usize) -> Option<u64> {
        let end = offset.checked_add(8)?;
        let slice = bytes.get(offset..end)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(slice);
        Some(u64::from_le_bytes(arr))
    }

    fn read_i64_from_slice(bytes: &[u8], offset: usize) -> Option<i64> {
        let end = offset.checked_add(8)?;
        let slice = bytes.get(offset..end)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(slice);
        Some(i64::from_le_bytes(arr))
    }

    fn read_fixed_0_1(addr: usize) -> Option<f64> {
        read_i32(addr)
            .map(|v| v as f64 / FP_SCALE)
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }

    fn read_fixed_nonnegative(addr: usize, max_value: f64) -> Option<f64> {
        read_i32(addr)
            .map(|v| v as f64 / FP_SCALE)
            .filter(|v| v.is_finite() && *v >= 0.0 && *v <= max_value)
    }

    fn decode_datehour(stored: i32) -> Option<String> {
        let min = (1850 * 365 * 24) + DATE_EPOCH_CONST;
        let max = (2200 * 365 * 24) + DATE_EPOCH_CONST;
        if stored < min || stored > max {
            return None;
        }
        let hrs = stored - DATE_EPOCH_CONST;
        let hour = hrs % 24;
        let rem_days = hrs / 24;
        let year = rem_days / 365;
        let mut doy = rem_days - year * 365;
        let month_lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut month = 1;
        for len in month_lengths {
            if doy < len {
                break;
            }
            doy -= len;
            month += 1;
        }
        Some(format!("{year}.{month}.{}.{hour}", doy + 1))
    }

    fn json_opt_str(value: Option<&str>) -> String {
        match value {
            Some(v) => format!("\"{v}\""),
            None => "null".to_string(),
        }
    }

    fn json_opt_escaped_str(value: Option<&str>) -> String {
        match value {
            Some(v) => json_str(v),
            None => "null".to_string(),
        }
    }

    fn json_opt_i32(value: Option<i32>) -> String {
        match value {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        }
    }

    fn json_opt_u32(value: Option<u32>) -> String {
        match value {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        }
    }

    fn json_opt_u32_vec(value: Option<&Vec<u32>>) -> String {
        match value {
            Some(values) => {
                let items = values
                    .iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[{items}]")
            }
            None => "null".to_string(),
        }
    }

    fn json_opt_string_vec(value: Option<&Vec<String>>) -> String {
        match value {
            Some(values) => {
                let items = values
                    .iter()
                    .map(|value| json_str(value))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[{items}]")
            }
            None => "null".to_string(),
        }
    }

    fn json_opt_bool(value: Option<bool>) -> String {
        match value {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        }
    }

    fn json_opt_f64(value: Option<f64>) -> String {
        match value {
            Some(v) if v.is_finite() => v.to_string(),
            _ => "null".to_string(),
        }
    }

    fn tag_sample_json(tag_table: usize, count: u32) -> String {
        if tag_table == 0 || count == 0 {
            return "[]".to_string();
        }
        let mut tags = Vec::new();
        for i in 0..count as usize {
            if let Some(tag) = read_small_inline_string(tag_table + i * 32) {
                tags.push(format!("\"{tag}\""));
            }
        }
        format!("[{}]", tags.join(","))
    }

    fn read_small_inline_string(addr: usize) -> Option<String> {
        let bytes = read_bytes(addr, 32)?;
        let len = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
        if len == 0 || len > 7 {
            return None;
        }
        let raw = &bytes[..len];
        if !raw
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'-')
        {
            return None;
        }
        String::from_utf8(raw.to_vec()).ok()
    }

    fn read_msvc_string(addr: usize, max_len: usize) -> Option<String> {
        let size = read_u64(addr + 0x10)? as usize;
        let cap = read_u64(addr + 0x18)? as usize;
        if size == 0 || size > max_len || cap < size || cap > (1 << 20) {
            return None;
        }
        let chars_addr = if cap < 16 {
            addr
        } else {
            read_u64(addr)? as usize
        };
        let bytes = read_bytes(chars_addr, size + 1)?;
        if bytes.get(size).copied() != Some(0) {
            return None;
        }
        let raw = &bytes[..size];
        if !raw.iter().all(|b| (32..127).contains(b)) {
            return None;
        }
        String::from_utf8(raw.to_vec()).ok()
    }

    fn read_stable_msvc_token_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {
        let first = read_msvc_token_nullable(addr, max_len)?;
        let second = read_msvc_token_nullable(addr, max_len)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_msvc_token_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {
        let size = read_u64(addr + 0x10)? as usize;
        let cap = read_u64(addr + 0x18)? as usize;
        if size > max_len || cap < size || cap > (1 << 20) {
            return None;
        }
        if size == 0 {
            return if cap < 16 { Some(None) } else { None };
        }
        let chars_addr = if cap < 16 {
            addr
        } else {
            read_u64(addr)? as usize
        };
        let bytes = read_bytes(chars_addr, size + 1)?;
        if bytes.get(size).copied() != Some(0) {
            return None;
        }
        let raw = &bytes[..size];
        if !raw.iter().all(|b| is_cosmetic_token_byte(*b)) {
            return None;
        }
        Some(Some(String::from_utf8(raw.to_vec()).ok()?))
    }

    fn is_cosmetic_token_byte(value: u8) -> bool {
        value.is_ascii_alphanumeric() || value == b'_' || value == b'-'
    }

    fn is_country_tag(tag: &str) -> bool {
        tag.len() == 3
            && tag
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && tag.bytes().any(|b| b.is_ascii_alphabetic())
    }

    fn read_bytes(addr: usize, len: usize) -> Option<Vec<u8>> {
        if len == 0 || !is_readable_range(addr, len) {
            return None;
        }
        let mut out = vec![0u8; len];
        unsafe {
            ptr::copy_nonoverlapping(addr as *const u8, out.as_mut_ptr(), len);
        }
        Some(out)
    }

    fn scan_qword_hits(needle: usize, lo: usize, hi: usize) -> Vec<usize> {
        let needle_bytes = (needle as u64).to_le_bytes();
        let mut hits = Vec::new();
        let mut addr = lo;
        const CHUNK: usize = 16 << 20;
        const OVERLAP: usize = 16;
        while addr < hi {
            let Some((region_start, region_size)) = readable_region(addr) else {
                addr = addr.saturating_add(0x1000);
                continue;
            };
            let region_end = region_start.saturating_add(region_size).min(hi);
            let mut pos = region_start.max(addr);
            while pos < region_end {
                let want = CHUNK.min(region_end - pos);
                let Some(blob) = read_bytes(pos, want) else {
                    pos = pos.saturating_add(want.max(0x1000));
                    continue;
                };
                let mut i = 0usize;
                while i + 8 <= blob.len() {
                    if blob[i..i + 8] == needle_bytes[..] && (pos + i) % 8 == 0 {
                        hits.push(pos + i);
                    }
                    i += 1;
                }
                if pos + blob.len() >= region_end || blob.len() < want {
                    pos = pos.saturating_add(blob.len().max(0x1000));
                } else {
                    pos = pos.saturating_add(blob.len().saturating_sub(OVERLAP));
                }
            }
            addr = region_end.saturating_add(0x1000);
        }
        hits
    }

    fn readable_region(addr: usize) -> Option<(usize, usize)> {
        let mut mbi = MemoryBasicInformation {
            base_address: ptr::null_mut(),
            allocation_base: ptr::null_mut(),
            allocation_protect: 0,
            partition_id: 0,
            region_size: 0,
            state: 0,
            protect: 0,
            type_: 0,
        };
        let ok = unsafe {
            VirtualQuery(
                addr as *const c_void,
                &mut mbi,
                core::mem::size_of::<MemoryBasicInformation>(),
            )
        };
        if ok == 0 || mbi.state != MEM_COMMIT {
            return None;
        }
        if (mbi.protect & (PAGE_NOACCESS | PAGE_GUARD)) != 0 {
            return None;
        }
        Some((mbi.base_address as usize, mbi.region_size))
    }

    fn is_readable_range(addr: usize, len: usize) -> bool {
        let Some((region_start, region_size)) = readable_region(addr) else {
            return false;
        };
        let region_end = region_start.saturating_add(region_size);
        addr >= region_start && addr.saturating_add(len) <= region_end
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

        fn raw_test_control() -> CollectorControl {
            CollectorControl {
                duration_ms: 8000,
                sample_ms: 10,
                country_object_scan: false,
                special_projects_scan: false,
                country_object_bridge: Vec::new(),
                country_object_bridge_valid: true,
                country_record_bridge: Vec::new(),
                country_record_bridge_valid: true,
                division_body_bridge: Vec::new(),
                division_body_bridge_valid: true,
                army_body_bridge: Vec::new(),
                army_body_bridge_valid: true,
                naval_combat_body_bridge: Vec::new(),
                naval_combat_body_bridge_valid: true,
                air_wing_body_bridge: Vec::new(),
                air_wing_body_bridge_valid: true,
                map_control: MapControlContract::empty(),
                advisor_token_filter: BTreeMap::new(),
                advisor_token_filter_valid: true,
                idea_group_filter: BTreeMap::new(),
                idea_group_filter_valid: true,
                scheduled_snapshot_step_hours: 1,
                pipe_token: Some("abc".to_string()),
                session_id: Some("run_1".to_string()),
                sidecar_log: false,
            }
        }

        fn stationary_division_sample() -> DivisionMovementSample {
            let raw_organisation = 5_000_000;
            let raw_strength = 100_000;
            DivisionMovementSample {
                div_id: 7,
                moving: false,
                raw_movement_progress: None,
                progress_bucket: None,
                location: 10,
                move_target_province: None,
                movement_source_province: None,
                raw_organisation,
                organisation_bucket: division_delta_bucket(raw_organisation),
                raw_strength,
                strength_bucket: division_delta_bucket(raw_strength),
                combat_ref_count: 0,
                in_combat: false,
            }
        }

        fn test_country_row(tag: &str, tag_db_id: u32, object_ptr: usize) -> CountryObjectRow {
            CountryObjectRow {
                tag: tag.to_string(),
                tag_db_id,
                name: tag.to_string(),
                object_ptr,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: Some(0.5),
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: None,
                focus: None,
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: None,
            }
        }

        #[test]
        fn collector_control_accepts_sanitized_session_id() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"pipe_token":"abc-123","session_id":"run_1-abc","sidecar_log":false}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert_eq!(parsed.duration_ms, 8000);
            assert_eq!(parsed.sample_ms, 10);
            assert_eq!(parsed.pipe_token.as_deref(), Some("abc-123"));
            assert_eq!(parsed.session_id.as_deref(), Some("run_1-abc"));
            assert!(parsed.country_object_bridge.is_empty());
            assert!(parsed.country_object_bridge_valid);
            assert!(parsed.country_record_bridge.is_empty());
            assert!(parsed.country_record_bridge_valid);
            assert!(parsed.division_body_bridge.is_empty());
            assert!(parsed.division_body_bridge_valid);
            assert!(parsed.army_body_bridge.is_empty());
            assert!(parsed.army_body_bridge_valid);
            assert!(parsed.naval_combat_body_bridge.is_empty());
            assert!(parsed.naval_combat_body_bridge_valid);
            assert!(parsed.map_control.state_roots.is_empty());
            assert!(parsed.map_control.state_roots_valid);
            assert!(parsed.map_control.province_bridge.is_none());
            assert!(parsed.map_control.province_bridge_valid);
            assert!(parsed.map_control.controller_bridge.is_empty());
            assert!(parsed.map_control.controller_bridge_valid);
            assert!(parsed.advisor_token_filter.is_empty());
            assert!(parsed.advisor_token_filter_valid);
            assert!(parsed.idea_group_filter.is_empty());
            assert!(parsed.idea_group_filter_valid);
            assert!(!parsed.special_projects_scan);
            assert!(!parsed.sidecar_log);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_special_projects_scan_flag() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_projects_scan_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"special_projects_scan":true}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.special_projects_scan);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_advisor_token_filter() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_advisor_filter_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"advisor_token_filter":"POR_salazar;ENG_maxwell_aitken;POR_salazar"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.advisor_token_filter_valid);
            assert_eq!(parsed.advisor_token_filter.len(), 2);
            assert!(parsed.advisor_token_filter.contains_key("POR_salazar"));
            assert!(parsed
                .advisor_token_filter
                .contains_key("ENG_maxwell_aitken"));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_advisor_token_filter_metadata() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_advisor_filter_meta_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"advisor_token_filter":"POR_salazar=political_advisor|political|POR_antonio_de_oliveira_salazar;ENG_maxwell_aitken=high_command||ENG_maxwell_aitken"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.advisor_token_filter_valid);
            assert_eq!(parsed.advisor_token_filter.len(), 2);
            let salazar = parsed.advisor_token_filter.get("POR_salazar").unwrap();
            assert_eq!(salazar.slot.as_deref(), Some("political_advisor"));
            assert_eq!(salazar.ledger.as_deref(), Some("political"));
            assert_eq!(
                salazar.character.as_deref(),
                Some("POR_antonio_de_oliveira_salazar")
            );
            let aitken = parsed
                .advisor_token_filter
                .get("ENG_maxwell_aitken")
                .unwrap();
            assert_eq!(aitken.slot.as_deref(), Some("high_command"));
            assert_eq!(aitken.ledger.as_deref(), None);
            assert_eq!(aitken.character.as_deref(), Some("ENG_maxwell_aitken"));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_idea_group_filter() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_idea_group_filter_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"idea_group_filter":"war_economy=economy;free_trade=trade_laws;volunteer_only=mobilization_laws"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.idea_group_filter_valid);
            assert_eq!(parsed.idea_group_filter.len(), 3);
            assert_eq!(
                parsed
                    .idea_group_filter
                    .get("war_economy")
                    .map(String::as_str),
                Some("economy")
            );
            assert_eq!(
                parsed
                    .idea_group_filter
                    .get("free_trade")
                    .map(String::as_str),
                Some("trade_laws")
            );
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_country_object_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_scan":false,"country_object_bridge":"1:0x1000;2:8192"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(!parsed.country_object_scan);
            assert!(parsed.country_object_bridge_valid);
            assert_eq!(parsed.country_object_bridge.len(), 2);
            assert_eq!(parsed.country_object_bridge[0].tag_db_id, 1);
            assert_eq!(parsed.country_object_bridge[0].object_ptr, 0x1000);
            assert_eq!(parsed.country_object_bridge[1].tag_db_id, 2);
            assert_eq!(parsed.country_object_bridge[1].object_ptr, 8192);
            assert!(parsed.country_record_bridge.is_empty());
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_country_record_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_record_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1000","country_record_bridge":"1:0x3000"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_record_bridge_valid);
            assert_eq!(parsed.country_record_bridge.len(), 1);
            assert_eq!(parsed.country_record_bridge[0].tag_db_id, 1);
            assert_eq!(parsed.country_record_bridge[0].object_ptr, 0x3000);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_division_body_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_division_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"division_body_bridge":"7:0x{:x};8:{}"}}"#,
                    HEAP_LO,
                    HEAP_LO + 0x100
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.division_body_bridge_valid);
            assert_eq!(parsed.division_body_bridge.len(), 2);
            assert_eq!(parsed.division_body_bridge[0].div_id, 7);
            assert_eq!(parsed.division_body_bridge[0].body_ptr, HEAP_LO);
            assert_eq!(parsed.division_body_bridge[1].div_id, 8);
            assert_eq!(parsed.division_body_bridge[1].body_ptr, HEAP_LO + 0x100);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_army_body_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_army_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"army_body_bridge":"17:0x{:x};18:{}"}}"#,
                    HEAP_LO,
                    HEAP_LO + 0x100
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.army_body_bridge_valid);
            assert_eq!(parsed.army_body_bridge.len(), 2);
            assert_eq!(parsed.army_body_bridge[0].army_id, 17);
            assert_eq!(parsed.army_body_bridge[0].body_ptr, HEAP_LO);
            assert_eq!(parsed.army_body_bridge[1].army_id, 18);
            assert_eq!(parsed.army_body_bridge[1].body_ptr, HEAP_LO + 0x100);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_naval_combat_body_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_naval_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"naval_combat_body_bridge":"271:0x{:x};272:{}"}}"#,
                    HEAP_LO,
                    HEAP_LO + 0x100
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.naval_combat_body_bridge_valid);
            assert_eq!(parsed.naval_combat_body_bridge.len(), 2);
            assert_eq!(parsed.naval_combat_body_bridge[0].combat_id, 271);
            assert_eq!(parsed.naval_combat_body_bridge[0].body_ptr, HEAP_LO);
            assert_eq!(parsed.naval_combat_body_bridge[1].combat_id, 272);
            assert_eq!(parsed.naval_combat_body_bridge[1].body_ptr, HEAP_LO + 0x100);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_air_wing_body_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_air_wing_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"air_wing_body_bridge":"1:3:0x{:x};2:4:{}"}}"#,
                    HEAP_LO,
                    HEAP_LO + 0x100
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.air_wing_body_bridge_valid);
            assert_eq!(parsed.air_wing_body_bridge.len(), 2);
            assert_eq!(parsed.air_wing_body_bridge[0].owner_tag_db_id, 1);
            assert_eq!(parsed.air_wing_body_bridge[0].wing_id, 3);
            assert_eq!(parsed.air_wing_body_bridge[0].body_ptr, HEAP_LO);
            assert_eq!(parsed.air_wing_body_bridge[1].owner_tag_db_id, 2);
            assert_eq!(parsed.air_wing_body_bridge[1].wing_id, 4);
            assert_eq!(parsed.air_wing_body_bridge[1].body_ptr, HEAP_LO + 0x100);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_map_control_contract() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_map_control_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            let state_roots = format!("1:0x{:x};1081:0x{:x}", HEAP_LO, HEAP_LO + 0x100);
            let province_bridge = format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=0",
                HEAP_LO + 0x1000
            );
            let controller_bridge =
                format!("1:0x{:x};2:0x{:x}", HEAP_LO + 0x2000, HEAP_LO + 0x3000);
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"map_control_state_roots":"{}","map_control_province_bridge":"{}","map_control_controller_bridge":"{}"}}"#,
                    state_roots, province_bridge, controller_bridge
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.map_control.state_roots_valid);
            assert_eq!(parsed.map_control.state_roots.len(), 2);
            assert_eq!(parsed.map_control.state_roots[0].state_id, 1);
            assert_eq!(parsed.map_control.state_roots[0].state_ptr, HEAP_LO);
            assert_eq!(parsed.map_control.state_roots[1].state_id, 1081);
            assert_eq!(parsed.map_control.state_roots[1].state_ptr, HEAP_LO + 0x100);
            assert!(parsed.map_control.province_bridge_valid);
            let bridge = parsed.map_control.province_bridge.unwrap();
            assert_eq!(bridge.province_array_ptr, HEAP_LO + 0x1000);
            assert_eq!(bridge.province_count, 13232);
            assert_eq!(bridge.province_stride, 8);
            assert_eq!(bridge.controller_ptr_offset, 0);
            assert!(parsed.map_control.controller_bridge_valid);
            assert_eq!(parsed.map_control.controller_bridge.len(), 2);
            assert_eq!(parsed.map_control.controller_bridge[0].tag_db_id, 1);
            assert_eq!(
                parsed.map_control.controller_bridge[0].controller_ptr,
                HEAP_LO + 0x2000
            );
            assert_eq!(parsed.map_control.controller_bridge[1].tag_db_id, 2);
            assert_eq!(
                parsed.map_control.controller_bridge[1].controller_ptr,
                HEAP_LO + 0x3000
            );
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_country_object_bridge_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1001"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_object_bridge.is_empty());
            assert!(!parsed.country_object_bridge_valid);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_advisor_token_filter_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_advisor_filter_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"advisor_token_filter":"POR_salazar;bad token!"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.advisor_token_filter.is_empty());
            assert!(!parsed.advisor_token_filter_valid);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_idea_group_filter_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_idea_group_filter_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"idea_group_filter":"war_economy=economy;bad token!=trade_laws"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.idea_group_filter.is_empty());
            assert!(!parsed.idea_group_filter_valid);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_division_body_bridge_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_division_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1000","division_body_bridge":"7:0x1001"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_object_bridge_valid);
            assert!(!parsed.country_object_bridge.is_empty());
            assert!(parsed.division_body_bridge.is_empty());
            assert!(!parsed.division_body_bridge_valid);
            let frame = build_frame_line(
                1234,
                None,
                Some(&parsed),
                Some(7),
                Some(125),
                Some("worker_poll"),
            );
            assert!(frame.contains("\"division_body_bridge_rows\":0"));
            assert!(frame.contains("\"division_body_bridge_valid\":false"));
            assert!(!frame.contains("0x1001"));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_army_body_bridge_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_army_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"army_body_bridge":"17:0x1001"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.army_body_bridge.is_empty());
            assert!(!parsed.army_body_bridge_valid);
            let frame = build_frame_line(
                1234,
                None,
                Some(&parsed),
                Some(7),
                Some(125),
                Some("worker_poll"),
            );
            assert!(frame.contains("\"army_body_bridge_rows\":0"));
            assert!(frame.contains("\"army_body_bridge_valid\":false"));
            assert!(!frame.contains("0x1001"));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_naval_combat_body_bridge_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_naval_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"naval_combat_body_bridge":"271:0x1001"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.naval_combat_body_bridge.is_empty());
            assert!(!parsed.naval_combat_body_bridge_valid);
            let frame = build_frame_line(
                1234,
                None,
                Some(&parsed),
                Some(7),
                Some(125),
                Some("worker_poll"),
            );
            assert!(frame.contains("\"naval_combat_body_bridge_rows\":0"));
            assert!(frame.contains("\"naval_combat_body_bridge_valid\":false"));
            assert!(!frame.contains("0x1001"));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_air_wing_body_bridge_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_air_wing_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"air_wing_body_bridge":"1:3:0x1001"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.air_wing_body_bridge.is_empty());
            assert!(!parsed.air_wing_body_bridge_valid);
            let frame = build_frame_line(
                1234,
                None,
                Some(&parsed),
                Some(7),
                Some(125),
                Some("worker_poll"),
            );
            assert!(frame.contains("\"air_wing_body_bridge_rows\":0"));
            assert!(frame.contains("\"air_wing_body_bridge_valid\":false"));
            assert!(!frame.contains("0x1001"));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_map_control_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_map_control_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            let state_roots = format!("1:0x{:x}", HEAP_LO + 1);
            let province_bridge = format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=8",
                HEAP_LO + 0x1000
            );
            let controller_bridge =
                format!("1:0x{:x};1:0x{:x}", HEAP_LO + 0x2000, HEAP_LO + 0x3000);
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1000","map_control_state_roots":"{}","map_control_province_bridge":"{}","map_control_controller_bridge":"{}"}}"#,
                    state_roots, province_bridge, controller_bridge
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_object_bridge_valid);
            assert!(!parsed.country_object_bridge.is_empty());
            assert!(parsed.map_control.state_roots.is_empty());
            assert!(!parsed.map_control.state_roots_valid);
            assert!(parsed.map_control.province_bridge.is_none());
            assert!(!parsed.map_control.province_bridge_valid);
            assert!(parsed.map_control.controller_bridge.is_empty());
            assert!(!parsed.map_control.controller_bridge_valid);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn country_object_bridge_parser_rejects_duplicates() {
            assert!(parse_country_object_bridge("1:0x1000;1:0x2000").is_none());
            assert!(parse_country_object_bridge("1:0x1000;2:0x1000").is_none());
        }

        #[test]
        fn division_body_bridge_parser_rejects_duplicates_ranges_and_size() {
            assert!(parse_division_body_bridge("").unwrap().is_empty());
            assert!(
                parse_division_body_bridge(&format!("1:0x{:x};2:{}", HEAP_LO, HEAP_LO + 8))
                    .is_some()
            );
            assert!(parse_division_body_bridge(&format!(
                "1:0x{:x};1:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(
                parse_division_body_bridge(&format!("1:0x{:x};2:0x{:x}", HEAP_LO, HEAP_LO))
                    .is_none()
            );
            assert!(parse_division_body_bridge(&format!("0:0x{:x}", HEAP_LO)).is_none());
            assert!(parse_division_body_bridge(&format!(
                "{}:0x{:x}",
                i32::MAX as u32 + 1,
                HEAP_LO
            ))
            .is_none());
            assert!(parse_division_body_bridge("1:0x1000").is_none());
            assert!(parse_division_body_bridge(&format!("1:0x{:x}", HEAP_LO + 1)).is_none());
            assert!(parse_division_body_bridge(
                &"x".repeat(MAX_DIVISION_BODY_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_DIVISION_BODY_BRIDGE_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_division_body_bridge(&too_many).is_none());
        }

        #[test]
        fn army_body_bridge_parser_rejects_duplicates_ranges_and_size() {
            assert!(parse_army_body_bridge("").unwrap().is_empty());
            assert!(
                parse_army_body_bridge(&format!("1:0x{:x};2:{}", HEAP_LO, HEAP_LO + 8)).is_some()
            );
            assert!(
                parse_army_body_bridge(&format!("1:0x{:x};1:0x{:x}", HEAP_LO, HEAP_LO + 8))
                    .is_none()
            );
            assert!(
                parse_army_body_bridge(&format!("1:0x{:x};2:0x{:x}", HEAP_LO, HEAP_LO)).is_none()
            );
            assert!(parse_army_body_bridge(&format!("0:0x{:x}", HEAP_LO)).is_none());
            assert!(
                parse_army_body_bridge(&format!("{}:0x{:x}", MAX_ARMY_ID + 1, HEAP_LO)).is_none()
            );
            assert!(parse_army_body_bridge("1:0x1000").is_none());
            assert!(parse_army_body_bridge(&format!("1:0x{:x}", HEAP_LO + 1)).is_none());
            assert!(
                parse_army_body_bridge(&"x".repeat(MAX_ARMY_BODY_BRIDGE_PAYLOAD_BYTES + 1))
                    .is_none()
            );
            let too_many = (1..=MAX_ARMY_BODY_BRIDGE_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_army_body_bridge(&too_many).is_none());
        }

        #[test]
        fn naval_combat_body_bridge_parser_rejects_duplicates_ranges_and_size() {
            assert!(parse_naval_combat_body_bridge("").unwrap().is_empty());
            assert!(parse_naval_combat_body_bridge(&format!(
                "1:0x{:x};2:{}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_some());
            assert!(parse_naval_combat_body_bridge(&format!(
                "1:0x{:x};1:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(parse_naval_combat_body_bridge(&format!(
                "1:0x{:x};2:0x{:x}",
                HEAP_LO, HEAP_LO
            ))
            .is_none());
            assert!(parse_naval_combat_body_bridge(&format!("0:0x{:x}", HEAP_LO)).is_none());
            assert!(parse_naval_combat_body_bridge(&format!(
                "{}:0x{:x}",
                MAX_NAVAL_COMBAT_ID + 1,
                HEAP_LO
            ))
            .is_none());
            assert!(parse_naval_combat_body_bridge("1:0x1000").is_none());
            assert!(parse_naval_combat_body_bridge(&format!("1:0x{:x}", HEAP_LO + 1)).is_none());
            assert!(parse_naval_combat_body_bridge(
                &"x".repeat(MAX_NAVAL_COMBAT_BODY_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_NAVAL_COMBAT_BODY_BRIDGE_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_naval_combat_body_bridge(&too_many).is_none());
        }

        #[test]
        fn air_wing_body_bridge_parser_rejects_duplicates_ranges_and_size() {
            assert!(parse_air_wing_body_bridge("").unwrap().is_empty());
            assert!(parse_air_wing_body_bridge(&format!(
                "1:3:0x{:x};2:4:{}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_some());
            assert!(parse_air_wing_body_bridge(&format!(
                "1:3:0x{:x};1:3:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(parse_air_wing_body_bridge(&format!(
                "1:3:0x{:x};2:4:0x{:x}",
                HEAP_LO, HEAP_LO
            ))
            .is_none());
            assert!(parse_air_wing_body_bridge(&format!("0:3:0x{:x}", HEAP_LO)).is_none());
            assert!(parse_air_wing_body_bridge(&format!("1:0:0x{:x}", HEAP_LO)).is_none());
            assert!(parse_air_wing_body_bridge(&format!("1025:3:0x{:x}", HEAP_LO)).is_none());
            assert!(parse_air_wing_body_bridge(&format!(
                "1:{}:0x{:x}",
                MAX_AIR_WING_ID + 1,
                HEAP_LO
            ))
            .is_none());
            assert!(parse_air_wing_body_bridge("1:3:0x1000").is_none());
            assert!(parse_air_wing_body_bridge(&format!("1:3:0x{:x}", HEAP_LO + 1)).is_none());
            assert!(parse_air_wing_body_bridge("1:3").is_none());
            assert!(parse_air_wing_body_bridge("1:3:0x10000000000:extra").is_none());
            assert!(parse_air_wing_body_bridge(
                &"x".repeat(MAX_AIR_WING_BODY_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_AIR_WING_BODY_BRIDGE_ROWS + 1)
                .map(|i| format!("1:{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_air_wing_body_bridge(&too_many).is_none());
        }

        #[test]
        fn map_control_state_roots_reject_duplicates_ranges_and_size() {
            assert!(parse_map_control_state_roots(&format!(
                "1:0x{:x};1:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(
                parse_map_control_state_roots(&format!("1:0x{:x};2:0x{:x}", HEAP_LO, HEAP_LO))
                    .is_none()
            );
            assert!(parse_map_control_state_roots("0:0x10000000000").is_none());
            assert!(parse_map_control_state_roots("1:0x1000").is_none());
            assert!(parse_map_control_state_roots(
                &"x".repeat(MAX_MAP_CONTROL_STATE_ROOT_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_MAP_CONTROL_STATE_ROOT_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_map_control_state_roots(&too_many).is_none());
        }

        #[test]
        fn map_control_province_bridge_rejects_bad_fields() {
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=0",
                HEAP_LO
            ))
            .is_some());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_count=13233;province_stride=8;controller_ptr_offset=0",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count={};province_stride=8;controller_ptr_offset=0",
                HEAP_LO,
                MAX_MAP_CONTROL_PROVINCE_COUNT + 1
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=4;controller_ptr_offset=0",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=8",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(
                "province_array=0x1000;province_count=13232;province_stride=8;controller_ptr_offset=0"
            )
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=0;extra=1",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(
                &"x".repeat(MAX_MAP_CONTROL_PROVINCE_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
        }

        #[test]
        fn map_control_controller_bridge_rejects_duplicates_ranges_and_size() {
            assert!(parse_map_control_controller_bridge(&format!(
                "1:0x{:x};1:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(parse_map_control_controller_bridge(&format!(
                "1:0x{:x};2:0x{:x}",
                HEAP_LO, HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_controller_bridge("0:0x10000000000").is_none());
            assert!(parse_map_control_controller_bridge("1:0x1000").is_none());
            assert!(parse_map_control_controller_bridge(
                &"x".repeat(MAX_MAP_CONTROL_CONTROLLER_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_map_control_controller_bridge(&too_many).is_none());
        }

        #[test]
        fn collector_control_rejects_unsafe_session_id() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_session_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"session_id":"bad path"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert_eq!(parsed.session_id, None);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn raw_clock_delta_frame_includes_bounded_contract_envelope() {
            let control = raw_test_control();
            let hour_index = ((1936 * 365) * 24 + 12) as u64;
            let sample = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index,
                raw_date: DATE_EPOCH_CONST + hour_index as i32,
            };

            let frame = clock_raw_domain_delta_frame_line(
                1234,
                11,
                Some(5),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &sample,
            )
            .unwrap();

            assert!(frame.ends_with('\n'));
            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"backend\":\"in_process_collector\""));
            assert!(frame.contains("\"collector_version\":\"0.1.0\""));
            assert!(frame.contains("\"abi_version\":1"));
            assert!(frame.contains("\"process_id\":1234"));
            assert!(frame.contains("\"tick_ms\":123456"));
            assert!(frame.contains("\"sequence\":11"));
            assert!(frame.contains("\"profile_id\":\"vanilla_latest_2026_06_28\""));
            assert!(frame.contains("\"session_sequence\":5"));
            assert!(frame.contains("\"duration_ms\":8000"));
            assert!(frame.contains("\"elapsed_wall_ms\":125"));
            assert!(frame.contains("\"date\":\"1936.1.1.12\""));
            assert!(frame.contains(&format!("\"hour_index\":{hour_index}")));
            assert!(frame.contains("\"sample_ms\":10"));
            assert!(frame.contains("\"capture_boundary\":\"worker_poll\""));
            assert!(frame.contains("\"domain\":\"clock\""));
            assert!(frame.contains("\"entity_key\":\"clock:game_date\""));
            assert!(frame.contains("\"source\":\"C\""));
            assert!(frame.contains("\"confidence\":\"sampled\""));
            assert!(frame.contains("\"payload\":{\"op\":\"sample\""));
            assert!(frame.contains("\"after\":{\"date\":\"1936.1.1.12\""));
            assert!(frame.contains(&format!(
                "\"raw_date\":{}",
                DATE_EPOCH_CONST + hour_index as i32
            )));
            assert!(frame.contains("\"changed_fields\":[\"date\",\"hour_index\",\"raw_date\"]"));
            assert!(!frame.contains("\"domains\""));
            assert!(!frame.contains("\"countries\""));
            assert!(!frame.contains("\"states\""));
            assert!(!frame.contains("\"provinces\""));
            assert!(!frame.contains("\"divisions\""));
        }

        #[test]
        fn raw_domain_delta_builder_rejects_unbounded_envelope_values() {
            let too_long = "x".repeat(RAW_SHORT_STRING_MAX_LENGTH + 1);
            let changed_fields = ["date"];

            let rejected_boundary = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: &too_long,
                domain: "clock",
                entity_key: "clock:game_date",
                confidence: "sampled",
                op: "sample",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_boundary.is_none());

            let rejected_pointer_key = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "clock",
                entity_key: "0x1234",
                confidence: "sampled",
                op: "sample",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_pointer_key.is_none());

            let accepted_divisions_domain = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "divisions",
                entity_key: "clock:game_date",
                confidence: "sampled",
                op: "sample",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(accepted_divisions_domain.is_some());

            let accepted_land_combats_domain = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "land_combats",
                entity_key: "land_combat:1:0",
                confidence: "sampled",
                op: "sample",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(accepted_land_combats_domain.is_some());

            let accepted_delete_null_after = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "land_combats",
                entity_key: "land_combat:1:0",
                confidence: "sampled",
                op: "delete",
                after_json: "null",
                changed_fields: &changed_fields,
            });
            assert!(accepted_delete_null_after.is_some());

            let rejected_unknown_domain = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "countries",
                entity_key: "clock:game_date",
                confidence: "sampled",
                op: "sample",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_unknown_domain.is_none());

            let rejected_confidence = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "clock",
                entity_key: "clock:game_date",
                confidence: "certain",
                op: "sample",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_confidence.is_none());

            let rejected_op = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "clock",
                entity_key: "clock:game_date",
                confidence: "sampled",
                op: "patch",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_op.is_none());

            let rejected_after = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "clock",
                entity_key: "clock:game_date",
                confidence: "sampled",
                op: "sample",
                after_json: "[]",
                changed_fields: &changed_fields,
            });
            assert!(rejected_after.is_none());
        }

        #[test]
        fn division_movement_tracker_emits_only_meaningful_changes() {
            let mut previous = BTreeMap::new();
            let stationary = stationary_division_sample();
            assert!(track_division_movement_sample(&mut previous, &stationary).is_none());

            let same_stationary_bucket = DivisionMovementSample {
                raw_organisation: 5_000_999,
                organisation_bucket: division_delta_bucket(5_000_999),
                raw_strength: 100_999,
                strength_bucket: division_delta_bucket(100_999),
                ..stationary.clone()
            };
            assert!(
                track_division_movement_sample(&mut previous, &same_stationary_bucket).is_none()
            );

            let org_bucket_changed = DivisionMovementSample {
                raw_organisation: 5_001_000,
                organisation_bucket: division_delta_bucket(5_001_000),
                ..same_stationary_bucket.clone()
            };
            let org_fields =
                track_division_movement_sample(&mut previous, &org_bucket_changed).unwrap();
            assert_eq!(org_fields, vec!["organisation"]);

            let strength_bucket_changed = DivisionMovementSample {
                raw_strength: 102_000,
                strength_bucket: division_delta_bucket(102_000),
                ..org_bucket_changed.clone()
            };
            let strength_fields =
                track_division_movement_sample(&mut previous, &strength_bucket_changed).unwrap();
            assert_eq!(strength_fields, vec!["strength"]);

            let combat_started = DivisionMovementSample {
                combat_ref_count: 1,
                in_combat: true,
                ..strength_bucket_changed.clone()
            };
            let combat_fields =
                track_division_movement_sample(&mut previous, &combat_started).unwrap();
            assert_eq!(combat_fields, vec!["combat_ref_count", "in_combat"]);

            let combat_ref_incremented = DivisionMovementSample {
                combat_ref_count: 2,
                ..combat_started.clone()
            };
            let combat_ref_fields =
                track_division_movement_sample(&mut previous, &combat_ref_incremented).unwrap();
            assert_eq!(combat_ref_fields, vec!["combat_ref_count"]);

            let moving_bucket_one = DivisionMovementSample {
                moving: true,
                raw_movement_progress: Some(1500),
                progress_bucket: Some(division_delta_bucket(1500)),
                move_target_province: Some(10),
                movement_source_province: Some(5),
                ..combat_ref_incremented.clone()
            };
            let start_fields =
                track_division_movement_sample(&mut previous, &moving_bucket_one).unwrap();
            assert!(start_fields.contains(&"moving"));
            assert!(start_fields.contains(&"movement_progress"));
            assert!(start_fields.contains(&"progress_bucket"));
            assert!(start_fields.contains(&"move_target_province"));
            assert!(start_fields.contains(&"movement_source_province"));

            let same_bucket = DivisionMovementSample {
                raw_movement_progress: Some(1900),
                progress_bucket: Some(division_delta_bucket(1900)),
                ..moving_bucket_one.clone()
            };
            assert!(track_division_movement_sample(&mut previous, &same_bucket).is_none());

            let next_bucket = DivisionMovementSample {
                raw_movement_progress: Some(2500),
                progress_bucket: Some(division_delta_bucket(2500)),
                ..same_bucket.clone()
            };
            let bucket_fields =
                track_division_movement_sample(&mut previous, &next_bucket).unwrap();
            assert_eq!(bucket_fields, vec!["movement_progress", "progress_bucket"]);

            let moved_location = DivisionMovementSample {
                location: 11,
                move_target_province: Some(11),
                ..next_bucket.clone()
            };
            let location_fields =
                track_division_movement_sample(&mut previous, &moved_location).unwrap();
            assert_eq!(location_fields, vec!["location", "move_target_province"]);

            let stopped = DivisionMovementSample {
                moving: false,
                raw_movement_progress: None,
                progress_bucket: None,
                location: 11,
                move_target_province: None,
                movement_source_province: None,
                ..moved_location.clone()
            };
            let stop_fields = track_division_movement_sample(&mut previous, &stopped).unwrap();
            assert!(stop_fields.contains(&"moving"));
            assert!(stop_fields.contains(&"movement_progress"));
            assert!(stop_fields.contains(&"progress_bucket"));
            assert!(stop_fields.contains(&"move_target_province"));
            assert!(stop_fields.contains(&"movement_source_province"));

            let mut fresh = BTreeMap::new();
            let first_moving =
                track_division_movement_sample(&mut fresh, &moving_bucket_one).unwrap();
            assert_eq!(first_moving, division_movement_all_fields());
        }

        #[test]
        fn division_movement_frame_builder_emits_sanitized_raw_delta() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = DivisionMovementSample {
                div_id: 7,
                moving: true,
                raw_movement_progress: Some(12000),
                progress_bucket: Some(division_delta_bucket(12000)),
                location: 11597,
                move_target_province: Some(11597),
                movement_source_province: Some(11333),
                raw_organisation: 7_654_321,
                organisation_bucket: division_delta_bucket(7_654_321),
                raw_strength: 98_765,
                strength_bucket: division_delta_bucket(98_765),
                combat_ref_count: 2,
                in_combat: true,
            };
            let changed_fields = division_movement_all_fields();

            let frame = division_raw_domain_delta_frame_line(
                1234,
                11,
                Some(5),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                &changed_fields,
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"domain\":\"divisions\""));
            assert!(frame.contains("\"entity_key\":\"division:7:4713\""));
            assert!(frame.contains("\"payload\":{\"op\":\"sample\""));
            assert!(frame.contains("\"div_uid\":\"7:4713\""));
            assert!(frame.contains("\"div_id\":7"));
            assert!(frame.contains("\"div_type\":4713"));
            assert!(frame.contains("\"moving\":true"));
            assert!(frame.contains("\"movement_progress\":0.12"));
            assert!(frame.contains("\"progress_bucket\":12"));
            assert!(frame.contains("\"location\":11597"));
            assert!(frame.contains("\"move_target_province\":11597"));
            assert!(frame.contains("\"movement_source_province\":11333"));
            assert!(frame.contains("\"organisation\":76.54321"));
            assert!(frame.contains("\"strength\":0.98765"));
            assert!(frame.contains("\"combat_ref_count\":2"));
            assert!(frame.contains("\"in_combat\":true"));
            assert!(frame.contains("\"changed_fields\":[\"div_uid\",\"div_id\",\"div_type\",\"moving\",\"movement_progress\",\"progress_bucket\",\"location\",\"move_target_province\",\"movement_source_province\",\"organisation\",\"strength\",\"combat_ref_count\",\"in_combat\"]"));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("tag_db_id"));
            assert!(!frame.contains("\"owner\""));
            assert!(!frame.contains("0x"));
        }

        #[test]
        fn division_event_frame_builder_emits_sanitized_raw_event() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = DivisionMovementSample {
                div_id: 7,
                moving: true,
                raw_movement_progress: Some(12000),
                progress_bucket: Some(division_delta_bucket(12000)),
                location: 11597,
                move_target_province: Some(11597),
                movement_source_province: Some(11333),
                raw_organisation: 7_654_321,
                organisation_bucket: division_delta_bucket(7_654_321),
                raw_strength: 98_765,
                strength_bucket: division_delta_bucket(98_765),
                combat_ref_count: 2,
                in_combat: true,
            };

            let frame = division_raw_event_frame_line(
                1234,
                12,
                Some(6),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                DIVISION_EVENT_ORG_STRENGTH_DELTA,
                &["organisation", "strength"],
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"event\""));
            assert!(frame.contains("\"domain\":\"divisions\""));
            assert!(frame.contains("\"entity_key\":\"division:7:4713\""));
            assert!(frame.contains("\"event_type\":\"division_org_strength_delta\""));
            assert!(frame.contains("\"subject\":\"division\""));
            assert!(frame.contains("\"div_uid\":\"7:4713\""));
            assert!(frame.contains("\"organisation\":76.54321"));
            assert!(frame.contains("\"strength\":0.98765"));
            assert!(frame.contains("\"changed_fields\":[\"organisation\",\"strength\"]"));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("tag_db_id"));
            assert!(!frame.contains("\"owner\""));
            assert!(!frame.contains("0x"));
        }

        #[test]
        fn first_seen_division_only_emits_movement_start_event() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = DivisionMovementSample {
                moving: true,
                raw_movement_progress: Some(12000),
                progress_bucket: Some(division_delta_bucket(12000)),
                move_target_province: Some(11597),
                movement_source_province: Some(11333),
                combat_ref_count: 2,
                in_combat: true,
                ..stationary_division_sample()
            };
            let mut lines = Vec::new();
            let mut session_sequence = 1;

            push_division_event_frames(
                &mut lines,
                &mut session_sequence,
                1234,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                None,
                &sample,
                &division_movement_all_fields(),
            );

            assert_eq!(lines.len(), 1);
            assert!(lines[0].contains("\"event_type\":\"division_movement_started\""));
            assert!(!lines[0].contains("\"event_type\":\"division_org_strength_delta\""));
            assert!(!lines[0].contains("\"event_type\":\"division_combat_ref_changed\""));
        }

        #[test]
        fn land_combat_frame_builder_emits_sanitized_raw_delta() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = LandCombatSample {
                combat_id: 271,
                generation: 2,
                body_addr: HEAP_LO + 0x12000,
                location: Some(3986),
                day: Some(4618),
                duration: Some(17),
                attacker_divisions: Some(vec![1053, 2837]),
                defender_divisions: Some(vec![465]),
                attacker_status: "exact",
                defender_status: "exact",
            };
            let changed_fields = land_combat_all_fields();

            let frame = land_combat_raw_domain_delta_frame_line(
                1234,
                22,
                Some(9),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                "sample",
                &changed_fields,
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"domain\":\"land_combats\""));
            assert!(frame.contains("\"entity_key\":\"land_combat:271:2\""));
            assert!(frame.contains("\"payload\":{\"op\":\"sample\""));
            assert!(frame.contains("\"combat_key\":\"land_combat:271:2\""));
            assert!(frame.contains("\"combat_id\":271"));
            assert!(frame.contains("\"generation\":2"));
            assert!(frame.contains("\"location\":3986"));
            assert!(frame.contains("\"day\":4618"));
            assert!(frame.contains("\"duration\":17"));
            assert!(frame.contains("\"attacker_divisions\":[1053,2837]"));
            assert!(frame.contains("\"defender_divisions\":[465]"));
            assert!(frame.contains("\"attacker_status\":\"exact\""));
            assert!(frame.contains("\"defender_status\":\"exact\""));
            assert!(frame.contains("\"changed_fields\":[\"combat_key\",\"combat_id\",\"generation\",\"location\",\"day\",\"duration\",\"attacker_divisions\",\"defender_divisions\",\"attacker_status\",\"defender_status\"]"));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("manager"));
            assert!(!frame.contains("0x"));
        }

        #[test]
        fn land_combat_event_builder_emits_sanitized_lifecycle_event() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = LandCombatSample {
                combat_id: 271,
                generation: 2,
                body_addr: HEAP_LO + 0x12000,
                location: Some(3986),
                day: Some(4618),
                duration: Some(17),
                attacker_divisions: Some(vec![1053, 2837]),
                defender_divisions: Some(vec![465]),
                attacker_status: "exact",
                defender_status: "exact",
            };
            let changed_fields = land_combat_all_fields();

            let frame = land_combat_raw_event_frame_line(
                1234,
                25,
                Some(12),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                LAND_COMBAT_EVENT_STARTED,
                &changed_fields,
                None,
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"event\""));
            assert!(frame.contains("\"domain\":\"land_combats\""));
            assert!(frame.contains("\"entity_key\":\"land_combat:271:2\""));
            assert!(frame.contains("\"event_type\":\"land_combat_started\""));
            assert!(frame.contains("\"subject\":\"land_combat\""));
            assert!(frame.contains("\"detail\":{\"combat_key\":\"land_combat:271:2\""));
            assert!(frame.contains("\"changed_fields\":[\"combat_key\",\"combat_id\",\"generation\",\"location\",\"day\",\"duration\",\"attacker_divisions\",\"defender_divisions\",\"attacker_status\",\"defender_status\"]"));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("tag_db_id"));
            assert!(!frame.contains("0x"));
        }

        #[test]
        fn army_order_frame_builders_emit_sanitized_bridge_rows() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = ArmyOrderSample {
                army_id: 17,
                generation: 4,
                order_addr: HEAP_LO + 0x33000,
                slot_index: 2,
                order_type: ORDER_TYPE5_OFFENSIVE,
                target_states: Some(vec![107, 108]),
                target_state_count: Some(2),
                target_status: "exact",
            };
            let changed_fields = army_order_all_fields();

            let frame = army_order_raw_domain_delta_frame_line(
                1234,
                42,
                Some(9),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                "sample",
                &changed_fields,
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"domain\":\"army_orders\""));
            assert!(frame.contains("\"entity_key\":\"army_order:17:4\""));
            assert!(frame.contains("\"order_key\":\"army_order:17:4\""));
            assert!(frame.contains("\"army_id\":17"));
            assert!(frame.contains("\"slot_index\":2"));
            assert!(frame.contains("\"order_type\":5"));
            assert!(frame.contains("\"target_states\":[107,108]"));
            assert!(frame.contains("\"target_state_count\":2"));
            assert!(frame.contains("\"target_status\":\"exact\""));
            assert!(frame.contains("\"changed_fields\":[\"order_key\",\"army_id\",\"generation\",\"slot_index\",\"order_type\",\"target_states\",\"target_state_count\",\"target_status\"]"));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("order_addr"));
            assert!(!frame.contains("tag_db_id"));
            assert!(!frame.contains("0x"));

            let event = army_order_raw_event_frame_line(
                1234,
                43,
                Some(10),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                ARMY_ORDER_EVENT_ADDED,
                &changed_fields,
            )
            .unwrap();

            assert!(event.contains("\"type\":\"event\""));
            assert!(event.contains("\"domain\":\"army_orders\""));
            assert!(event.contains("\"event_type\":\"army_order_added\""));
            assert!(event.contains("\"subject\":\"army_order\""));
            assert!(event.contains("\"detail\":{\"order_key\":\"army_order:17:4\""));
            assert!(!event.contains("body_ptr"));
            assert!(!event.contains("body_addr"));
            assert!(!event.contains("order_addr"));
            assert!(!event.contains("tag_db_id"));
            assert!(!event.contains("0x"));
        }

        #[test]
        fn naval_combat_frame_builders_emit_sanitized_bridge_rows() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = NavalCombatSample {
                combat_id: 241671,
                generation: 3,
                body_addr: HEAP_LO + 0x22000,
                location: Some(3825),
                attacker_tags: Some(vec!["FRA".to_string()]),
                defender_tags: Some(vec!["JAP".to_string()]),
                attacker_ship_count: Some(12),
                defender_ship_count: Some(13),
                attacker_commander: Some("Amedee Nivelle".to_string()),
                defender_commander: None,
                attacker_positioning: Some(1.0),
                defender_positioning: Some(0.97),
                attacker_status: "exact",
                defender_status: "exact",
            };
            let changed_fields = naval_combat_all_fields();

            let frame = naval_combat_raw_domain_delta_frame_line(
                1234,
                42,
                Some(9),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                "sample",
                &changed_fields,
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"domain\":\"naval_air_combats\""));
            assert!(frame.contains("\"entity_key\":\"naval_combat:241671:3\""));
            assert!(frame.contains("\"combat_key\":\"naval_combat:241671:3\""));
            assert!(frame.contains("\"combat_id\":241671"));
            assert!(frame.contains("\"location\":3825"));
            assert!(frame.contains("\"attacker_tags\":[\"FRA\"]"));
            assert!(frame.contains("\"defender_tags\":[\"JAP\"]"));
            assert!(frame.contains("\"attacker_ship_count\":12"));
            assert!(frame.contains("\"defender_ship_count\":13"));
            assert!(frame.contains("\"attacker_commander\":\"Amedee Nivelle\""));
            assert!(frame.contains("\"defender_commander\":null"));
            assert!(frame.contains("\"attacker_positioning\":1"));
            assert!(frame.contains("\"defender_positioning\":0.97"));
            assert!(frame.contains("\"attacker_status\":\"exact\""));
            assert!(frame.contains("\"defender_status\":\"exact\""));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("tag_db_id"));
            assert!(!frame.contains("0x"));

            let event = naval_combat_raw_event_frame_line(
                1234,
                43,
                Some(10),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                NAVAL_COMBAT_EVENT_STARTED,
                &changed_fields,
                None,
            )
            .unwrap();
            assert!(event.contains("\"type\":\"event\""));
            assert!(event.contains("\"domain\":\"naval_air_combats\""));
            assert!(event.contains("\"event_type\":\"naval_combat_started\""));
            assert!(event.contains("\"subject\":\"naval_combat\""));
            assert!(event.contains("\"detail\":{\"combat_key\":\"naval_combat:241671:3\""));
            assert!(!event.contains("body_ptr"));
            assert!(!event.contains("body_addr"));
            assert!(!event.contains("tag_db_id"));
            assert!(!event.contains("0x"));

            let delete_frame = naval_combat_raw_domain_delta_frame_line(
                1234,
                44,
                Some(11),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                "delete",
                &["combat_key"],
            )
            .unwrap();
            assert!(delete_frame.contains("\"domain\":\"naval_air_combats\""));
            assert!(delete_frame.contains("\"payload\":{\"op\":\"delete\",\"after\":null"));
            assert!(!delete_frame.contains("body_addr"));
            assert!(!delete_frame.contains("0x"));
        }

        #[test]
        fn naval_combat_member_frame_builder_emits_sanitized_rows() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = NavalCombatMemberSample {
                member_key: "naval_combat_members:naval_combat:241671:3:attacker:ship:3458"
                    .to_string(),
                combat_key: "naval_combat:241671:3".to_string(),
                combat_id: 241671,
                generation: 3,
                side: "attacker",
                group_index: 0,
                member_index: 2,
                unit_type_tag: 51,
                ship_instance_id: Some(3458),
                is_convoy: false,
                owner: Some("FRA".to_string()),
                name: Some("Sidi-Ferruch".to_string()),
                type_display: Some("Cruiser Submarine".to_string()),
                archetype: Some("submarine".to_string()),
                designed_max_strength_raw: Some(49_000_000),
                battle_slot_index_raw: 4,
                state_bitmask: Some(4),
                engagement_weight_raw: Some(100_000),
            };
            let changed_fields = naval_combat_member_all_fields();

            let frame = naval_combat_member_raw_domain_delta_frame_line(
                1234,
                42,
                Some(9),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                "sample",
                &changed_fields,
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"domain\":\"naval_combat_members\""));
            assert!(frame.contains(
                "\"entity_key\":\"naval_combat_members:naval_combat:241671:3:attacker:ship:3458\""
            ));
            assert!(frame.contains("\"combat_key\":\"naval_combat:241671:3\""));
            assert!(frame.contains("\"side\":\"attacker\""));
            assert!(frame.contains("\"ship_instance_id\":3458"));
            assert!(frame.contains("\"is_convoy\":false"));
            assert!(frame.contains("\"owner\":\"FRA\""));
            assert!(frame.contains("\"name\":\"Sidi-Ferruch\""));
            assert!(frame.contains("\"type_display\":\"Cruiser Submarine\""));
            assert!(frame.contains("\"archetype\":\"submarine\""));
            assert!(frame.contains("\"designed_max_strength_raw\":49000000"));
            assert!(frame.contains("\"battle_slot_index_raw\":4"));
            assert!(frame.contains("\"state_bitmask\":4"));
            assert!(frame.contains("\"engagement_weight_raw\":100000"));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("member_addr"));
            assert!(!frame.contains("tag_db_id"));
            assert!(!frame.contains("0x"));

            let delete_frame = naval_combat_member_raw_domain_delta_frame_line(
                1234,
                43,
                Some(10),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                "delete",
                &["member_key"],
            )
            .unwrap();
            assert!(delete_frame.contains("\"domain\":\"naval_combat_members\""));
            assert!(delete_frame.contains("\"payload\":{\"op\":\"delete\",\"after\":null"));
            assert!(!delete_frame.contains("0x"));
        }

        #[test]
        fn land_combat_manager_keeps_valid_rows_when_some_slots_fail() {
            let sample = LandCombatSample {
                combat_id: 271,
                generation: 0,
                body_addr: HEAP_LO + 0x12000,
                location: Some(3986),
                day: Some(4618),
                duration: Some(17),
                attacker_divisions: Some(vec![1053, 2837]),
                defender_divisions: Some(vec![465]),
                attacker_status: "exact",
                defender_status: "exact",
            };
            let mut samples = BTreeMap::new();
            samples.insert(sample.combat_id, sample);

            let kept = land_combat_manager_samples_result(samples, 3).unwrap();

            assert_eq!(kept.len(), 1);
            assert_eq!(kept[0].combat_id, 271);
        }

        #[test]
        fn land_combat_manager_reports_gap_when_only_invalid_slots_remain() {
            let samples = BTreeMap::new();

            let err = land_combat_manager_samples_result(samples, 3).unwrap_err();

            assert_eq!(err, "manager_entry_invalid");
        }

        #[test]
        fn land_combat_tracker_emits_start_update_and_end_events() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let mut tracker = LandCombatTracker::default();
            let sample = LandCombatSample {
                combat_id: 271,
                generation: 0,
                body_addr: HEAP_LO + 0x12000,
                location: Some(3986),
                day: Some(4618),
                duration: Some(17),
                attacker_divisions: Some(vec![1053]),
                defender_divisions: Some(vec![465]),
                attacker_status: "exact",
                defender_status: "exact",
            };

            let started = build_land_combat_raw_lines_from_samples(
                1234,
                &control,
                &mut tracker,
                10,
                Some(125),
                "worker_poll",
                &clock,
                vec![sample.clone()],
            );
            assert_eq!(started.len(), 2);
            assert!(started[0].contains("\"type\":\"domain_delta\""));
            assert!(started[1].contains("\"event_type\":\"land_combat_started\""));
            assert!(started[1].contains("\"entity_key\":\"land_combat:271:0\""));

            let mut updated_sample = sample.clone();
            updated_sample.duration = Some(18);
            updated_sample.attacker_divisions = Some(vec![1053, 2837]);
            let updated = build_land_combat_raw_lines_from_samples(
                1234,
                &control,
                &mut tracker,
                20,
                Some(225),
                "worker_poll",
                &clock,
                vec![updated_sample],
            );
            assert_eq!(updated.len(), 2);
            assert!(updated[0].contains("\"changed_fields\":[\"duration\",\"attacker_divisions\"]"));
            assert!(updated[1].contains("\"event_type\":\"land_combat_updated\""));
            assert!(updated[1].contains("\"changed_fields\":[\"duration\",\"attacker_divisions\"]"));

            let ended = build_land_combat_raw_lines_from_samples(
                1234,
                &control,
                &mut tracker,
                30,
                Some(325),
                "worker_poll",
                &clock,
                Vec::new(),
            );
            assert_eq!(ended.len(), 2);
            assert!(ended[0].contains("\"payload\":{\"op\":\"delete\",\"after\":null"));
            assert!(ended[1].contains("\"event_type\":\"land_combat_ended\""));
            assert!(ended[1].contains("\"end_reason\":\"resolved\""));
        }

        #[test]
        fn land_combat_tracker_splits_generation_on_body_change() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let mut tracker = LandCombatTracker::default();
            let sample = LandCombatSample {
                combat_id: 271,
                generation: 0,
                body_addr: HEAP_LO + 0x12000,
                location: Some(3986),
                day: Some(10),
                duration: Some(10),
                attacker_divisions: Some(vec![1053]),
                defender_divisions: Some(vec![465]),
                attacker_status: "exact",
                defender_status: "exact",
            };
            let _ = build_land_combat_raw_lines_from_samples(
                1234,
                &control,
                &mut tracker,
                10,
                Some(125),
                "worker_poll",
                &clock,
                vec![sample.clone()],
            );

            let mut reused = sample;
            reused.body_addr = HEAP_LO + 0x22000;
            reused.day = Some(1);
            let split = build_land_combat_raw_lines_from_samples(
                1234,
                &control,
                &mut tracker,
                20,
                Some(225),
                "worker_poll",
                &clock,
                vec![reused],
            );

            assert_eq!(split.len(), 4);
            assert!(split[0].contains("\"entity_key\":\"land_combat:271:0\""));
            assert!(split[0].contains("\"payload\":{\"op\":\"delete\",\"after\":null"));
            assert!(split[1].contains("\"event_type\":\"land_combat_ended\""));
            assert!(split[1].contains("\"end_reason\":\"body_changed\""));
            assert!(split[2].contains("\"entity_key\":\"land_combat:271:1\""));
            assert!(split[3].contains("\"event_type\":\"land_combat_started\""));
            assert!(split[3].contains("\"entity_key\":\"land_combat:271:1\""));
        }

        #[test]
        fn land_combat_delete_and_gap_frames_have_explicit_contracts() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = LandCombatSample {
                combat_id: 271,
                generation: 2,
                body_addr: HEAP_LO + 0x12000,
                location: Some(3986),
                day: Some(4618),
                duration: Some(17),
                attacker_divisions: Some(vec![1053, 2837]),
                defender_divisions: Some(vec![465]),
                attacker_status: "exact",
                defender_status: "exact",
            };

            let delete_frame = land_combat_raw_domain_delta_frame_line(
                1234,
                23,
                Some(10),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                "delete",
                &["combat_key"],
            )
            .unwrap();
            assert!(delete_frame.contains("\"domain\":\"land_combats\""));
            assert!(delete_frame.contains("\"entity_key\":\"land_combat:271:2\""));
            assert!(delete_frame.contains("\"payload\":{\"op\":\"delete\",\"after\":null"));
            assert!(delete_frame.contains("\"changed_fields\":[\"combat_key\"]"));
            assert!(!delete_frame.contains("\"attacker_divisions\""));
            assert!(!delete_frame.contains("body_addr"));
            assert!(!delete_frame.contains("0x"));

            let gap_frame = land_combat_gap_raw_domain_delta_frame_line(
                1234,
                24,
                Some(11),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                "manager_read_failed",
            )
            .unwrap();
            assert!(gap_frame.contains("\"domain\":\"land_combats\""));
            assert!(gap_frame.contains("\"entity_key\":\"land_combats:gap:manager_read_failed\""));
            assert!(gap_frame.contains("\"confidence\":\"gap\""));
            assert!(gap_frame.contains("\"gap_kind\":\"manager_read_failed\""));
            assert!(gap_frame.contains("\"status\":\"validation_failed\""));
            assert!(!gap_frame.contains("body_addr"));
            assert!(!gap_frame.contains("0x"));
        }

        #[test]
        fn raw_clock_delta_does_not_pollute_snapshot_domain_claims() {
            let control = raw_test_control();
            let sample = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };

            let raw = clock_raw_domain_delta_frame_line(
                1234,
                11,
                Some(5),
                123456,
                &control,
                Some(125),
                "scheduled_hour_snapshot",
                &sample,
            )
            .unwrap();
            assert!(raw.contains("\"domain\":\"clock\""));
            assert!(!raw.contains("\"domains\""));
            assert!(!raw.contains("\"root_probe\""));

            let snapshot = build_frame_line(
                1234,
                None,
                Some(&control),
                Some(7),
                Some(125),
                Some("scheduled_hour_snapshot"),
            );

            assert!(snapshot.contains("\"type\":\"collector_started\""));
            assert!(snapshot.contains("\"domains\":[]"));
            assert!(!snapshot.contains("\"domain\":\"clock\""));
            assert!(!snapshot.contains("\"payload\""));
        }

        #[test]
        fn worker_frame_without_state_includes_session_metadata_and_no_domains() {
            let control = CollectorControl {
                duration_ms: 8000,
                sample_ms: 10,
                country_object_scan: false,
                special_projects_scan: false,
                country_object_bridge: Vec::new(),
                country_object_bridge_valid: true,
                country_record_bridge: Vec::new(),
                country_record_bridge_valid: true,
                division_body_bridge: Vec::new(),
                division_body_bridge_valid: true,
                army_body_bridge: Vec::new(),
                army_body_bridge_valid: true,
                naval_combat_body_bridge: Vec::new(),
                naval_combat_body_bridge_valid: true,
                air_wing_body_bridge: Vec::new(),
                air_wing_body_bridge_valid: true,
                map_control: MapControlContract::empty(),
                advisor_token_filter: BTreeMap::new(),
                advisor_token_filter_valid: true,
                idea_group_filter: BTreeMap::new(),
                idea_group_filter_valid: true,
                scheduled_snapshot_step_hours: 1,
                pipe_token: Some("abc".to_string()),
                session_id: Some("run_1".to_string()),
                sidecar_log: false,
            };

            let frame = build_frame_line(
                1234,
                None,
                Some(&control),
                Some(7),
                Some(125),
                Some("worker_poll"),
            );

            assert!(frame.contains("\"type\":\"collector_started\""));
            assert!(frame.contains("\"session_id\":\"run_1\""));
            assert!(frame.contains("\"session_sequence\":7"));
            assert!(frame.contains("\"elapsed_wall_ms\":125"));
            assert!(frame.contains("\"capture_boundary\":\"worker_poll\""));
            assert!(frame.contains("\"duration_ms\":8000"));
            assert!(frame.contains("\"sample_ms\":10"));
            assert!(frame.contains("\"special_projects_scan\":false"));
            assert!(frame.contains("\"scheduled_snapshot_step_hours\":1"));
            assert!(frame.contains("\"map_control_state_root_rows\":0"));
            assert!(frame.contains("\"map_control_state_roots_valid\":true"));
            assert!(frame.contains("\"map_control_province_bridge_present\":false"));
            assert!(frame.contains("\"map_control_province_bridge_valid\":true"));
            assert!(frame.contains("\"map_control_controller_bridge_rows\":0"));
            assert!(frame.contains("\"map_control_controller_bridge_valid\":true"));
            assert!(frame.contains("\"division_body_bridge_rows\":0"));
            assert!(frame.contains("\"division_body_bridge_valid\":true"));
            assert!(frame.contains("\"profile_id\":\"vanilla_latest_2026_06_28\""));
            assert!(frame.contains("\"domains\":[]"));
            assert!(frame.contains("\"root_probe\":null"));
        }

        #[test]
        fn domain_list_claims_province_control_only_with_province_rows() {
            let cache = CountryIdentityCache::not_attempted("test");

            let without_provinces = cache.domains_json_from_rows(
                &[],
                true,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
            );
            assert!(without_provinces.contains("\"state_ownership\""));
            assert!(without_provinces.contains("\"state_control\""));
            assert!(!without_provinces.contains("\"province_control\""));

            let with_provinces = cache.domains_json_from_rows(
                &[],
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
            );
            assert!(!with_provinces.contains("\"state_control\""));
            assert!(with_provinces.contains("\"province_control\""));
        }

        #[test]
        fn domain_list_claims_projects_live_only_with_special_projects() {
            let cache = CountryIdentityCache::not_attempted("test");

            let without_projects = cache.domains_json_from_rows(
                &[],
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
            );
            assert!(!without_projects.contains("\"projects_live\""));

            let with_projects = cache.domains_json_from_rows(
                &[],
                false,
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
            );
            assert!(with_projects.contains("\"projects_live\""));
        }

        #[test]
        fn special_projects_json_emits_top_level_collector_catalog() {
            let mut skills = BTreeMap::new();
            skills.insert("specialization_air", 2);
            skills.insert("specialization_land", 3);
            let json = special_projects_json(
                &[ProjectFacilityRow {
                    id: 1,
                    province: 118,
                    building: Some("facility_alpha".to_string()),
                    active_project_handle: 9001,
                    active: true,
                    scientist_char_id: Some(44),
                    unread_prototype_rewards: 0,
                }],
                &[ProjectRow {
                    id: 77,
                    static_def_handle: 9001,
                    iterations: 2,
                    worked_at_facility: Some(1),
                }],
                &[ProjectScientistRow {
                    char_id: 44,
                    name_token: Some("POR_scientist_a".to_string()),
                    display_name: Some("Ada \"Ace\"".to_string()),
                    skills,
                    trait_count: Some(1),
                }],
            );

            assert!(json.contains("\"facilities\":["));
            assert!(json.contains("\"active_project_handle\":9001"));
            assert!(json.contains("\"projects\":["));
            assert!(json.contains("\"worked_at_facility\":1"));
            assert!(json.contains("\"scientists\":["));
            assert!(json.contains("\"display_name\":\"Ada \\\"Ace\\\"\""));
            assert!(json.contains("\"count\":{\"facilities\":1,\"projects\":1,\"scientists\":1}"));
            assert!(json.contains("\"source\":\"C\""));
        }

        #[test]
        fn country_database_root_status_enables_country_scalars() {
            let row = test_country_row("POR", 47, HEAP_LO + 0x4700);
            let cache = CountryIdentityCache {
                status: "country_database_root_verified",
                scan_ms: 1,
                country_vtable: HEAP_LO + 0x1000,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows: vec![row.clone()],
                rejected_objects: 0,
                duplicate_tags: 0,
            };

            assert!(cache.has_scalar_source());
            assert!(cache
                .domains_json_from_rows(
                    &[row],
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false
                )
                .contains("\"country_scalars\""));
            assert!(cache
                .root_json()
                .contains("\"status\":\"country_database_root_verified\""));

            let gamestate_cache = CountryIdentityCache {
                status: "gamestate_country_table_verified",
                scan_ms: 1,
                country_vtable: HEAP_LO + 0x1000,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows: vec![test_country_row("GER", 1, HEAP_LO + 0x100)],
                rejected_objects: 0,
                duplicate_tags: 0,
            };
            assert!(gamestate_cache.has_scalar_source());
            assert!(gamestate_cache
                .domains_json_from_rows(
                    &gamestate_cache.rows,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false
                )
                .contains("\"country_scalars\""));
        }

        #[test]
        fn country_tag_object_maps_require_shared_agreement() {
            let mut left = BTreeMap::new();
            let mut right = BTreeMap::new();
            for tag_db_id in 1..=MIN_USABLE_COUNTRY_OBJECT_ROWS as u32 {
                left.insert(tag_db_id, HEAP_LO + tag_db_id as usize * 0x100);
                right.insert(tag_db_id, HEAP_LO + tag_db_id as usize * 0x100);
            }
            assert!(country_tag_object_maps_agree(&left, &right));

            right.insert(7, HEAP_LO + 0xDEAD);
            assert!(!country_tag_object_maps_agree(&left, &right));

            right.clear();
            for tag_db_id in 1..MIN_USABLE_COUNTRY_OBJECT_ROWS as u32 {
                right.insert(tag_db_id, HEAP_LO + tag_db_id as usize * 0x100);
            }
            assert!(!country_tag_object_maps_agree(&left, &right));
        }

        #[test]
        fn cprovince_row_json_preserves_raw_controller_id() {
            let decoded = cprovince_control_row_json(11, Some("POR"), 47, Some(112));
            assert!(decoded.contains("\"11\":{\"controller\":\"POR\""));
            assert!(decoded.contains("\"controller_tag_db_id\":47"));
            assert!(decoded.contains("\"controller_decode_status\":\"decoded\""));
            assert!(decoded.contains("\"controller_source\":\"cprovince_static_root\""));
            assert!(decoded.contains("\"state_id\":112"));

            let raw_only = cprovince_control_row_json(12, None, 454, None);
            assert!(raw_only.contains("\"12\":{\"controller\":null"));
            assert!(raw_only.contains("\"controller_tag_db_id\":454"));
            assert!(raw_only.contains("\"controller_decode_status\":\"tag_table_miss\""));
            assert!(!raw_only.contains("\"state_id\""));
        }

        #[test]
        fn country_object_row_json_includes_tag_db_id() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: Some(0x5000),
                original_capital: Some(112),
                original_tag: None,
                templates_locked: Some(false),
                reserved_dynamic_country: Some(false),
                stability: Some(0.61),
                war_support: Some(0.42),
                political_power: Some(7.25),
                command_power: Some(12.5),
                num_ships: Some(7),
                manpower_pool: Some(123456),
                convoys: Some(99.0),
                misc: None,
                fuel: None,
                production: None,
                focus: None,
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: None,
            };

            let json = row_json(&row, false, false, false, false, false, false, false);

            assert!(json.contains("\"POR\""));
            assert!(json.contains("\"tag_db_id\":47"));
            assert!(json.contains("\"stability\":0.61"));
            assert!(json.contains("\"political_power\":7.25"));
            assert!(json.contains("\"manpower_pool\":123456"));
            assert!(!json.contains("\"fuel\""));
        }

        #[test]
        fn country_object_row_json_includes_country_misc_fields() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: Some(0x5000),
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: Some(CountryMiscRow {
                    convoys_destroyed: Some(12),
                    accidents_score: Some(0.5),
                    ace_count: Some(3),
                    cosmetic_tag: Some(None),
                }),
                fuel: None,
                production: None,
                focus: None,
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: None,
            };

            let json = row_json(&row, false, false, false, false, false, false, false);

            assert!(json.contains("\"misc\""));
            assert!(json.contains("\"convoys_destroyed\":12"));
            assert!(json.contains("\"accidents_score\":0.5"));
            assert!(json.contains("\"ace_count\":3"));
            assert!(json.contains("\"cosmetic_tag\":null"));
            let cache = CountryIdentityCache::not_attempted("test");
            let domains = cache.domains_json_from_rows(
                &[row],
                false, // state rows
                false, // province rows
                false, // special-project rows
                false, // fuel
                false, // production
                false, // focus
                false, // research
                false, // politics
                false, // politics residuals
                false, // decisions
            );
            assert!(domains.contains("\"country_misc\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_fuel() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: Some(CountryFuelRow {
                    current: 123.5,
                    max: 456.25,
                    gain: 7.5,
                    cost: -2.25,
                }),
                production: None,
                focus: None,
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: None,
            };

            let scheduled = row_json(&row, true, false, false, false, false, false, false);
            let worker = row_json(&row, false, false, false, false, false, false, false);

            assert!(scheduled.contains(
                "\"fuel\":{\"current\":123.5,\"max\":456.25,\"gain\":7.5,\"cost\":-2.25}"
            ));
            assert!(!worker.contains("\"fuel\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_focus() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: None,
                focus: Some(CountryFocusRow {
                    focus_tree: Some("portugal_focus".to_string()),
                    completed: Some(vec![
                        "POR_estado_novo".to_string(),
                        "POR_army_reorganization".to_string(),
                    ]),
                    completed_count: Some(2),
                    current: Some("POR_industrial_focus".to_string()),
                    current_continuous: Some("continuous_naval_production".to_string()),
                    progress: Some(12.5),
                }),
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: None,
            };

            let scheduled = row_json(&row, false, false, true, false, false, false, false);
            let worker = row_json(&row, false, false, false, false, false, false, false);

            assert!(scheduled.contains("\"focus\""));
            assert!(scheduled.contains("\"focus_tree\":\"portugal_focus\""));
            assert!(scheduled
                .contains("\"completed\":[\"POR_estado_novo\",\"POR_army_reorganization\"]"));
            assert!(scheduled.contains("\"completed_count\":2"));
            assert!(scheduled.contains("\"current\":\"POR_industrial_focus\""));
            assert!(scheduled.contains("\"current_continuous\":\"continuous_naval_production\""));
            assert!(scheduled.contains("\"progress\":12.5"));
            assert!(!worker.contains("\"focus\""));
            let cache = CountryIdentityCache::not_attempted("test");
            let domains = cache.domains_json_from_rows(
                &[row],
                false, // state rows
                false, // province rows
                false, // special-project rows
                false, // fuel
                false, // production
                true,  // focus
                false, // research
                false, // politics
                false, // politics residuals
                false, // decisions
            );
            assert!(domains.contains("\"focus_live\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_research() {
            let mut row = test_country_row("POR", 47, 0x1234);
            row.research = Some(CountryResearchRow {
                completed_tech_count: Some(123),
                research_slot_count: Some(5),
                active_research: Some(vec![CountryResearchSlotRow {
                    slot_index: 0,
                    tech_handle_raw: 0x9020,
                    tech_def_index_raw: 0x9020,
                    progress_raw: 117_000,
                }]),
            });

            let scheduled = row_json(&row, false, false, false, true, false, false, false);
            let worker = row_json(&row, false, false, false, false, false, false, false);

            assert!(scheduled.contains("\"research\""));
            assert!(scheduled.contains("\"completed_tech_count\":123"));
            assert!(scheduled.contains("\"research_slot_count\":5"));
            assert!(scheduled.contains("\"active_research\":[{\"slot_index\":0"));
            assert!(scheduled.contains("\"tech_handle_raw\":36896"));
            assert!(scheduled.contains("\"progress_raw\":117000"));
            assert!(!worker.contains("\"research\""));

            let cache = CountryIdentityCache::not_attempted("test");
            let domains = cache.domains_json_from_rows(
                &[row],
                false,
                false,
                false,
                false,
                false,
                false,
                true,
                false,
                false,
                false,
            );
            assert!(domains.contains("\"tech_live\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_politics() {
            let mut popularities = BTreeMap::new();
            popularities.insert("democratic".to_string(), 72.5);
            popularities.insert("communism".to_string(), 0.0);
            popularities.insert("fascism".to_string(), 12.5);
            popularities.insert("neutrality".to_string(), 15.0);
            let mut laws = BTreeMap::new();
            laws.insert("economy".to_string(), "war_economy".to_string());
            laws.insert("trade_laws".to_string(), "free_trade".to_string());
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: None,
                focus: None,
                research: None,
                politics: Some(CountryPoliticsRow {
                    popularities: Some(popularities),
                    ruling_party: Some("neutrality".to_string()),
                    ideas: Some(vec![
                        "war_economy".to_string(),
                        "free_trade".to_string(),
                        "sour_loser".to_string(),
                    ]),
                    laws: Some(laws),
                    ideas_other: Some(vec!["sour_loser".to_string()]),
                    national_spirits: Some(vec!["sour_loser".to_string()]),
                    idea_count: Some(3),
                    election_frequency: Some(48),
                    party_count: Some(4),
                    low_confidence: Some(false),
                }),
                politics_residuals: None,
                decisions: None,
            };

            let scheduled = row_json(&row, false, false, false, false, true, false, false);
            let worker = row_json(&row, false, false, false, false, false, false, false);

            assert!(scheduled.contains("\"politics\""));
            assert!(scheduled.contains("\"ruling_party\":\"neutrality\""));
            assert!(scheduled.contains("\"war_economy\""));
            assert!(scheduled.contains("\"laws\":{\"economy\":\"war_economy\""));
            assert!(scheduled.contains("\"trade_laws\":\"free_trade\""));
            assert!(scheduled.contains("\"ideas_other\":[\"sour_loser\"]"));
            assert!(scheduled.contains("\"national_spirits\":[\"sour_loser\"]"));
            assert!(scheduled.contains("\"party_count\":4"));
            assert!(!worker.contains("\"politics\""));
            let cache = CountryIdentityCache::not_attempted("test");
            let domains = cache.domains_json_from_rows(
                &[row],
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                true,
                false,
                false,
            );
            assert!(domains.contains("\"politics_live\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_politics_residuals() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: None,
                focus: None,
                research: None,
                politics: None,
                politics_residuals: Some(CountryPoliticsResidualsRow {
                    last_election: Some("1933.7.1.1".to_string()),
                    elections_allowed: Some(false),
                    appointed_advisors: Some(vec![
                        "ENG_maxwell_aitken".to_string(),
                        "POR_salazar".to_string(),
                    ]),
                    appointed_advisor_details: Some(vec![AppointedAdvisorRow {
                        idea_token: "POR_salazar".to_string(),
                        slot: Some("political_advisor".to_string()),
                        ledger: Some("political".to_string()),
                        character: Some("POR_antonio_de_oliveira_salazar".to_string()),
                    }]),
                    appointed_advisor_slots: Some({
                        let mut slots = BTreeMap::new();
                        slots.insert(
                            "political_advisor".to_string(),
                            vec!["POR_salazar".to_string()],
                        );
                        slots
                    }),
                }),
                decisions: None,
            };

            let scheduled = row_json(&row, false, false, false, false, false, true, false);
            let worker = row_json(&row, false, false, false, false, false, false, false);

            assert!(scheduled.contains("\"politics\""));
            assert!(scheduled.contains("\"last_election\":\"1933.7.1.1\""));
            assert!(scheduled.contains("\"elections_allowed\":false"));
            assert!(scheduled
                .contains("\"appointed_advisors\":[\"ENG_maxwell_aitken\",\"POR_salazar\"]"));
            assert!(scheduled.contains("\"appointed_advisor_details\""));
            assert!(scheduled.contains("\"idea_token\":\"POR_salazar\""));
            assert!(scheduled.contains("\"slot\":\"political_advisor\""));
            assert!(scheduled.contains("\"ledger\":\"political\""));
            assert!(scheduled.contains("\"character\":\"POR_antonio_de_oliveira_salazar\""));
            assert!(scheduled
                .contains("\"appointed_advisor_slots\":{\"political_advisor\":[\"POR_salazar\"]}"));
            assert!(!worker.contains("\"politics\""));
            let cache = CountryIdentityCache::not_attempted("test");
            let domains = cache.domains_json_from_rows(
                &[row],
                false, // state rows
                false, // province rows
                false, // special-project rows
                false, // fuel
                false, // production
                false, // focus
                false, // research
                false, // politics
                true,  // politics residuals
                false, // decisions
            );
            assert!(domains.contains("\"politics_residuals\""));
            assert!(!domains.contains("\"politics_live\""));
        }

        #[test]
        fn country_object_row_json_preserves_empty_appointed_advisors() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: None,
                focus: None,
                research: None,
                politics: None,
                politics_residuals: Some(CountryPoliticsResidualsRow {
                    last_election: None,
                    elections_allowed: None,
                    appointed_advisors: Some(Vec::new()),
                    appointed_advisor_details: Some(Vec::new()),
                    appointed_advisor_slots: Some(BTreeMap::new()),
                }),
                decisions: None,
            };

            let scheduled = row_json(&row, false, false, false, false, false, true, false);

            assert!(scheduled.contains("\"appointed_advisors\":[]"));
            assert!(scheduled.contains("\"appointed_advisor_details\":[]"));
            assert!(scheduled.contains("\"appointed_advisor_slots\":{}"));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_decisions() {
            let mut status_present = BTreeMap::new();
            status_present.insert("active_timed_decision".to_string(), 1);
            status_present.insert("decision_to_re_enable".to_string(), 2);
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: None,
                focus: None,
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: Some(CountryDecisionsRow {
                    status_present: Some(status_present),
                    timed_live: Some(vec![TimedDecisionRow {
                        decision: "POR_countdown_mission".to_string(),
                        days: 43,
                        state: 0,
                        state_name: "active".to_string(),
                    }]),
                }),
            };

            let scheduled = row_json(&row, false, false, false, false, false, false, true);
            let worker = row_json(&row, false, false, false, false, false, false, false);

            assert!(scheduled.contains("\"decisions\""));
            assert!(scheduled.contains("\"status_present\""));
            assert!(scheduled.contains("\"active_timed_decision\":1"));
            assert!(scheduled.contains("\"decision\":\"POR_countdown_mission\""));
            assert!(scheduled.contains("\"days\":43"));
            assert!(scheduled.contains("\"state_name\":\"active\""));
            assert!(!worker.contains("\"decisions\""));
            let cache = CountryIdentityCache::not_attempted("test");
            let domains = cache.domains_json_from_rows(
                &[row],
                false, // state rows
                false, // province rows
                false, // special-project rows
                false, // fuel
                false, // production
                false, // focus
                false, // research
                false, // politics
                false, // politics residuals
                true,  // decisions
            );
            assert!(domains.contains("\"decisions_live\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_production() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: Some(CountryProductionRow {
                    stockpile: Some(vec![
                        CountryStockpileRow {
                            variant_id: 70,
                            amount: 123.5,
                        },
                        CountryStockpileRow {
                            variant_id: 71,
                            amount: -0.25,
                        },
                    ]),
                    military_lines: Some(vec![CountryProductionLineRow {
                        id: 349,
                        active_factories: 3,
                        cost: 105.0,
                        speed: 31.05,
                        produced: 34.4,
                        amount: -1,
                    }]),
                    naval_lines: Some(Vec::new()),
                }),
                focus: None,
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: None,
            };

            let scheduled = row_json(&row, false, true, false, false, false, false, false);
            let worker = row_json(&row, false, false, false, false, false, false, false);

            assert!(scheduled.contains("\"production\""));
            assert!(scheduled.contains("\"stockpile\":{\"70\":123.5,\"71\":-0.25}"));
            assert!(scheduled.contains("\"stockpile_total\":123.25"));
            assert!(scheduled.contains("\"military_factories\":3"));
            assert!(scheduled.contains("\"naval_factories\":0"));
            assert!(scheduled.contains("\"amount\":-1"));
            assert!(!worker.contains("\"production\""));
        }

        #[test]
        fn country_object_row_json_omits_unavailable_production_leaves() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: Some(CountryProductionRow {
                    stockpile: None,
                    military_lines: Some(vec![CountryProductionLineRow {
                        id: 349,
                        active_factories: 3,
                        cost: 105.0,
                        speed: 31.05,
                        produced: 34.4,
                        amount: -1,
                    }]),
                    naval_lines: Some(Vec::new()),
                }),
                focus: None,
                research: None,
                politics: None,
                politics_residuals: None,
                decisions: None,
            };

            let scheduled = row_json(&row, false, true, false, false, false, false, false);

            assert!(scheduled.contains("\"production\""));
            assert!(!scheduled.contains("\"stockpile\""));
            assert!(!scheduled.contains("\"stockpile_total\""));
            assert!(scheduled.contains("\"military_lines\""));
            assert!(scheduled.contains("\"military_factories\":3"));
            assert!(scheduled.contains("\"naval_lines\":[]"));
            assert!(scheduled.contains("\"naval_factories\":0"));
        }
    }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: *mut core::ffi::c_void,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    unsafe { attach_log::dll_main(module, reason) }
}

#[cfg(windows)]
mod hook_proc {
    use core::ffi::c_void;
    use core::ptr;

    #[link(name = "user32")]
    extern "system" {
        fn CallNextHookEx(hook: *mut c_void, code: i32, wparam: usize, lparam: isize) -> isize;
    }

    pub unsafe fn get_msg_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn Hoi4ReplayGetMsgProc(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    unsafe { hook_proc::get_msg_proc(code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_exports_expected_metadata() {
        let json = core::str::from_utf8(heartbeat_json_for_tests()).unwrap();
        assert!(json.contains("\"type\":\"heartbeat\""));
        assert!(json.contains("\"backend\":\"in_process_collector\""));
        assert!(json.contains("\"collector_version\":\"0.1.0\""));
        assert_eq!(battle-plan_collector_abi_version(), ABI_VERSION);
        assert_eq!(
            battle-plan_collector_heartbeat_len(),
            heartbeat_json_for_tests().len()
        );
    }

    #[test]
    fn heartbeat_copy_reports_required_length() {
        let mut buf = [0u8; 16];
        let required = copy_heartbeat_json(&mut buf);
        assert_eq!(required, heartbeat_json_for_tests().len());
        assert_eq!(&buf, &heartbeat_json_for_tests()[..16]);
    }

    #[test]
    fn heartbeat_abi_accepts_null_for_length_probe() {
        let required = unsafe { battle-plan_collector_heartbeat_json(ptr::null_mut(), 0) };
        assert_eq!(required, heartbeat_json_for_tests().len());
    }
}
