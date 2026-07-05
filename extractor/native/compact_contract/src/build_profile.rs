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
/// Current game-hour counter inside CGameState, used as the paused-frame gate.
pub const CGAMESTATE_CURRENT_HOUR: usize = 0x468;

/// Module RVA of the static country database pointer slot.
pub const COUNTRY_DATABASE_OFFSET: usize = 0x33189C8;

/// Maximum compact country/tag-db id accepted by live row schemas.
///
/// Country table traversal may retain a wider structural guard, but emitted
/// compact owner ids must share this cap with lane validators to avoid encoder
/// panics on high sparse slots.
/// Existing 1024/4096 tag/table caps are deliberate-or-unaudited; harmonizing them requires a contract-change patch with boundary fixtures.
pub const MAX_COMPACT_OWNER_TAG_DB_ID: u32 = 2_048;

/// Domain 57 session multiplayer metadata roots for supported build 870ebf.
///
/// Provenance: 2026-07-02 passive live probe validation
/// (`live_probe_session.py`) passed the strict SP oracle twice with Ireland as
/// human-controlled TAG `IRE` at tag-db index 29. The values are current-build
/// sentinels only; if the profile sentinel/build SHA changes, these offsets must
/// be revalidated before domain 57 is scheduled.
pub const DOMAIN57_CGAMESTATE_SINGLETON_RVA: usize = 0x3316E90;
pub const DOMAIN57_CGAMESTATE_PLAYER_COUNTRY_COUNT: usize = 0x394;
pub const DOMAIN57_CGAMESTATE_COUNTRY_COUNT: usize = DOMAIN57_CGAMESTATE_PLAYER_COUNTRY_COUNT;
pub const DOMAIN57_CGAMESTATE_HUMAN_STATUS_ARRAY: usize = 0x388;
pub const DOMAIN57_CGAMESTATE_COUNTRY_HANDLE_ARRAY: usize = 0x310;
pub const DOMAIN57_COUNTRY_HANDLE_SLOT_INDEX: usize = 0x08;
pub const DOMAIN57_COUNTRY_HANDLE_LOCALIZED_NAME_UNINTEGRATED: usize = 0x10;
pub const DOMAIN57_COUNTRY_HANDLE_LOCALIZED_NAME: usize =
    DOMAIN57_COUNTRY_HANDLE_LOCALIZED_NAME_UNINTEGRATED;
pub const DOMAIN57_CGAMESTATE_COUNTRY_TAG_RECORDS: usize = 0x358;
pub const DOMAIN57_CGAMESTATE_SLOT_TAG_ARRAY: usize = DOMAIN57_CGAMESTATE_COUNTRY_TAG_RECORDS;
pub const DOMAIN57_COUNTRY_TAG_RECORD_STRIDE: usize = 0x20;
pub const DOMAIN57_COUNTRY_TAG_RECORD_TAG: usize = 0x00;

/// Domain 57 pause-clock roots for supported build 870ebf.
///
/// Provenance: the same 2026-07-02 passive probe validated both clock singleton
/// slots and required the clock object's vtable identity before trusting the
/// pause bool at `+0x6C1`. Keep the vtable check in readers; the bool offset is
/// not valid without the `module+0x2952510` identity match.
pub const DOMAIN57_CLOCK_SINGLETON_A_RVA: usize = 0x33172E0;
pub const DOMAIN57_CLOCK_SINGLETON_B_RVA: usize = 0x33172E8;
pub const DOMAIN57_CLOCK_SINGLETON_SLOT_A_RVA: usize = DOMAIN57_CLOCK_SINGLETON_A_RVA;
pub const DOMAIN57_CLOCK_SINGLETON_SLOT_B_RVA: usize = DOMAIN57_CLOCK_SINGLETON_B_RVA;
pub const DOMAIN57_CLOCK_VTABLE_RVA: usize = 0x2952510;
pub const DOMAIN57_CLOCK_IS_PAUSED: usize = 0x6C1;

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
/// COrderInstance save-identity instance id (u32), validated live on build
/// 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad against
/// the same-tick save oracle: 430/430 enumerated army orders matched the
/// save's (army_id, slot) -> instance_id map at +0x244 (next-best offset
/// 22/430). Evidence: `extractor/documents/WAVE3_CRACKS_870EBF_D04.md` §2,
/// `extractor/native/runs/wave3_division_order_link_870ebf/`.
pub const DOMAIN54_ORDER_OFF_INSTANCE_ID: usize = 0x244;
/// CDivision -> the division's current/primary assigned COrderInstance
/// pointer, validated in the same wave-3 evidence: 2035 divisions point at an
/// enumerated army order (2031/2035 in the division's oracle scheduled-order
/// set; 4 = order advanced past the snapshot), ~1067 point at an army-GROUP
/// COrderInstance (back-ref `.?AVCArmyGroup@@`), 65 null.
pub const DOMAIN20_DIVISION_OFF_PRIMARY_ORDER: usize = 0xC8;

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

/// Domain 47 advisor/trait roots, revalidated live on supported build
/// 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad
/// (paused D04 save 1944.2.11.18, 2026-07-03; evidence in
/// `extractor/documents/WAVE3_CRACKS_870EBF_D04.md` §1 and
/// `extractor/native/runs/wave3_d47_advisor_trait_870ebf/`).
///
/// `.?AVCAdvisor@@` vtable: 10,576 live instances (~1005 dead-pointer objects
/// carry no idea token and are excluded); advisor idea-token oracle 4874/4876
/// (2 diffs are byte-identical UTF-8/latin-1 mojibake), slot oracle 9547/9547.
/// The historical RVA never moved and is now validated.
pub const DOMAIN47_CADVISOR_VTABLE_RVA: usize = 0x2990E60;
/// `.?AVCUnitLeaderTrait@@` vtable: 194 live instances; per-leader assigned
/// trait multiset 1193/1193 exact keyed by unit_leader_id. CORRECTED from the
/// stale historical `0x291F3C0` during the same wave-3 revalidation.
pub const DOMAIN47_CUNIT_LEADER_TRAIT_VTABLE_RVA: usize = 0x292F160;

/// Domain 51 pending delayed-event roots, validated live on build
/// 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad (paused
/// D04, re-probe 2026-07-04: 22/22 CDelayedEvent instances, 22/22 CEvent RTTI,
/// 22/22 valid combined ids, 22/22 owner==owner-copy; oracle join 17/17 via
/// scope tag + random seed pair). Evidence: WAVE3_CRACKS_870EBF_D04.md §5,
/// `extractor/native/runs/wave3_delayed_event_870ebf/`.
pub const DOMAIN51_CDELAYED_EVENT_VTABLE_RVA: usize = 0x27D0FB8;
/// CDelayedEvent+0x08 -> CEvent (`.?AVCEvent@@`).
pub const DOMAIN51_CDELAYED_EVENT_CEVENT_OFF: usize = 0x08;
/// CEvent+0x0C = combined event id (namespace*100000 + index).
pub const DOMAIN51_CEVENT_COMBINED_ID_OFF: usize = 0x0C;
/// CDelayedEvent+0x18 = owner/originator tag_db_id (also the scope tag_db_id;
/// second copy at +0xC0).
pub const DOMAIN51_CDELAYED_EVENT_OWNER_TAG_OFF: usize = 0x18;
pub const DOMAIN51_CDELAYED_EVENT_OWNER_TAG_COPY_OFF: usize = 0xC0;
/// CDelayedEvent scope.random seed pair (+0x1C low32, +0x20 high32).
pub const DOMAIN51_CDELAYED_EVENT_SCOPE_RANDOM_A_OFF: usize = 0x1C;
pub const DOMAIN51_CDELAYED_EVENT_SCOPE_RANDOM_B_OFF: usize = 0x20;
/// CDelayedEvent+0xC4 (u32) = total remaining hours until the event fires =
/// months*720 + days*24 + hours (30-day months). Validated 17/17 exact vs the
/// same-tick save oracle (orchestrator-confirmed 2026-07-04; live re-probe
/// 22/22 in bounds), all decoy encodings (decomposed d/h/m, absolute
/// timestamp) ruled out. This resolves the wave-3 due-date RESIDUAL.
pub const DOMAIN51_CDELAYED_EVENT_DUE_REMAINING_HOURS_OFF: usize = 0xC4;
/// Sane ceiling on the remaining-hours countdown (~57 years of 30-day-month
/// hours). A value beyond this is treated as a torn read.
pub const DOMAIN51_MAX_DELAYED_EVENT_DUE_HOURS: u32 = 500_000;

// --- W3 domain 64 delayed_events_live: ROOTED per-country list root ----------
// The per-CDelayedEvent field offsets above (CEVENT_OFF, COMBINED_ID_OFF,
// OWNER_TAG_COPY_OFF as the originator, SCOPE_RANDOM, DUE_REMAINING_HOURS_OFF,
// VTABLE_RVA) are shared with the d51 diagnostic path. What d64 adds is the
// ROOTED list root that replaces the d51 heap scan (agent D6, D04, 13/13
// countries, 17/17 events, ZERO mismatch vs melt oracle; no heap scan):
//   CCountry+0x1290 = std::vector<CDelayedEvent*> begin
//   CCountry+0x1298 = packed u64 (low32 = count)
/// CCountry+0x1290: begin pointer of the per-country pending delayed-event
/// vector<CDelayedEvent*>.
pub const DOMAIN64_COUNTRY_DELAYED_EVENT_VECTOR_BEGIN_OFF: usize = 0x1290;
/// CCountry+0x1298: packed u64; low32 = count of pending delayed events.
pub const DOMAIN64_COUNTRY_DELAYED_EVENT_VECTOR_PACKED_OFF: usize = 0x1298;
/// Fail-closed ceiling on the per-country pending-event count (a low32 beyond
/// this is a torn/aliased read). D04's busiest country has 2; 4096 is safe.
pub const DOMAIN64_MAX_DELAYED_EVENTS_PER_COUNTRY: u32 = 4096;

// --- W5 domain 67 country_dynamic_modifiers_live: ROOTED per-country walk -----
// The country's OWN active dynamic-modifier definition ids (agent D2, D04 melt
// oracle: 6/6 countries exact multiset + 159/47/144 database-wide). ROOTED walk
// of the inline CDynamicModifierContainer at CCountry+0xE58 (its own vtable is
// mb+0x27BFDE8; a +0x08 back-ref u64 that MUST == the owning CCountry object
// pointer hard-guards ownership — opinion/relation modifiers between OTHER
// countries live in different containers and would fail this guard). The entry
// array pointer is at container+0x28; the u32 count is at container+0x34 (packed
// as one u64 with capacity at +0x30). Each entry is 0x40 bytes; entry+0x18 ->
// CDynamicModifier (vtable mb+0x27BFD48), whose modifier DEFINITION id is the
// MSVC std::string at +0x28. The CDynamicModifier is a SHARED/interned def per
// modifier-id (fine: membership is per-country via the guarded entry array; the
// id token is the def's name = the melt `modifier="..."`). Per-instance
// value/expiry is UNRESOLVED-BY-ABSENCE in D04 and is not emitted. 390/437
// countries have count==0 (the array is never walked when count==0).
/// CCountry+0xE58: inline CDynamicModifierContainer (the container object begins
/// here; its own vtable is DOMAIN67_DYNAMIC_MODIFIER_CONTAINER_VTABLE_RVA).
pub const DOMAIN67_COUNTRY_DYNAMIC_MODIFIER_CONTAINER_OFF: usize = 0xE58;
/// CDynamicModifierContainer vtable RVA (mb-relative); the inline container's
/// first qword must equal mb+this.
pub const DOMAIN67_DYNAMIC_MODIFIER_CONTAINER_VTABLE_RVA: usize = 0x27BFDE8;
/// container+0x08: back-ref to the owning CCountry (hard ownership guard).
pub const DOMAIN67_DYNAMIC_MODIFIER_CONTAINER_BACKREF_OFF: usize = 0x08;
/// container+0x28: u64 pointer to the SDynamicModifierEntry[] array.
pub const DOMAIN67_DYNAMIC_MODIFIER_CONTAINER_ENTRY_ARRAY_OFF: usize = 0x28;
/// container+0x34: u32 entry count (capacity is the sibling u32 at +0x30).
pub const DOMAIN67_DYNAMIC_MODIFIER_CONTAINER_COUNT_OFF: usize = 0x34;
/// SDynamicModifierEntry stride.
pub const DOMAIN67_DYNAMIC_MODIFIER_ENTRY_STRIDE: usize = 0x40;
/// entry+0x18: u64 pointer to the shared CDynamicModifier definition object.
pub const DOMAIN67_DYNAMIC_MODIFIER_ENTRY_DEF_PTR_OFF: usize = 0x18;
/// CDynamicModifier vtable RVA (mb-relative); each def object's first qword
/// must equal mb+this before its id string is read.
pub const DOMAIN67_DYNAMIC_MODIFIER_DEF_VTABLE_RVA: usize = 0x27BFD48;
/// CDynamicModifier+0x28: MSVC std::string = the modifier DEFINITION id.
pub const DOMAIN67_DYNAMIC_MODIFIER_DEF_ID_OFF: usize = 0x28;
/// Fail-closed ceiling on the per-country dynamic-modifier count (a u32 beyond
/// this is a torn/aliased read). D04's busiest country has 10; 4096 is safe.
pub const DOMAIN67_MAX_DYNAMIC_MODIFIERS_PER_COUNTRY: u32 = 4096;
/// Maximum modifier-id string length accepted (a longer MSVC string is a torn
/// read and drops that entry).
pub const DOMAIN67_MAX_DYNAMIC_MODIFIER_ID_LEN: usize = 128;

// --- W3 domain 65 special_project_programs_live: ROOTED owner-joined walk -----
// CCountry+0xFA8 -> CProgramStatus, an array of up to 3 records (stride 0x160).
// Each record's own vtable must == the CProgramStatus primary vtable
// (mb+0x295AC88); a record whose vtable differs ends the record walk. Each
// record: +0x20 = std::vector<CProgram*> begin, +0x28 = packed u64
// (low32 = count, high32 = capacity). Each element -> CProgram
// (vtable mb+0x295AC30): program_id @+0x0C, province @+0x174,
// unread_prototype_rewards @+0x178. Validated 26/26 vs melt (agent D6).
/// CCountry+0xFA8: pointer to the CProgramStatus record array.
pub const DOMAIN65_COUNTRY_PROGRAM_STATUS_OFF: usize = 0xFA8;
/// CProgramStatus primary vtable RVA (each record's own vtable must match).
pub const DOMAIN65_PROGRAM_STATUS_VTABLE_RVA: usize = 0x295AC88;
/// CProgram facility vtable RVA.
pub const DOMAIN65_PROGRAM_VTABLE_RVA: usize = 0x295AC30;
/// CProgramStatus record stride (array-of-records).
pub const DOMAIN65_PROGRAM_STATUS_RECORD_STRIDE: usize = 0x160;
/// Maximum CProgramStatus records walked per country (air/army/naval).
pub const DOMAIN65_MAX_PROGRAM_STATUS_RECORDS: usize = 3;
/// CProgramStatus record +0x20: vector<CProgram*> begin.
pub const DOMAIN65_PROGRAM_STATUS_RECORD_VECTOR_BEGIN_OFF: usize = 0x20;
/// CProgramStatus record +0x28: packed u64 (low32 = count, high32 = capacity).
pub const DOMAIN65_PROGRAM_STATUS_RECORD_PACKED_OFF: usize = 0x28;
/// CProgram +0x0C: program_id (owner-join key; == melt program.id).
pub const DOMAIN65_PROGRAM_ID_OFF: usize = 0x0C;
/// CProgram +0x174: province (independent 2nd join key).
pub const DOMAIN65_PROGRAM_PROVINCE_OFF: usize = 0x174;
/// CProgram +0x178: unread_prototype_rewards (the reward/output scalar).
pub const DOMAIN65_PROGRAM_UNREAD_REWARDS_OFF: usize = 0x178;
/// Fail-closed ceiling on a record's CProgram count (a low32 beyond this, or a
/// low32 > capacity, is a torn/aliased read that drops the record).
pub const DOMAIN65_MAX_PROGRAMS_PER_RECORD: u32 = 4096;

// --- W4 domain 66 occupation_state_extras_live -------------------------------
// A NEW rooted per-occupied-state domain that re-walks the occupation chain in
// the MAIN collector from validated country roots, keeping the forbidden d52
// occupation_resistance_compliance_live lane byte-identical. All RVAs/offsets
// are build 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad
// and mirror the same-build values used by the committed d52 lane (agent D5,
// validated vs the D04 melt oracle).
//
// Chain (per validated CCountry): +0xFD0 -> CCountryOccupationStatus
// (vtable mb+0x296DD50, secondary vtable mb+0x296DD88 at +0x18) -> +0x48
// occupation-data container -> (bounded qword scan) CCountryOccupationData
// (vtable mb+0x296DD00; +0x08 back-ref to the status ptr; +0x10 occupied
// tag_db_id) -> +0xB0 law hashmap AND +0xD8 garrison-list container ->
// (bounded qword scan) CStateGarrisonData (vtable mb+0x296DCB0; +0x58 back-ref
// to its CCountryOccupationData; +0x50 -> CState) -> +0x70 manpower-pool sub.
/// CCountry+0xFD0: pointer to CCountryOccupationStatus.
pub const DOMAIN66_COUNTRY_OCCUPATION_STATUS_OFF: usize = 0xFD0;
/// CCountryOccupationStatus primary vtable RVA.
pub const DOMAIN66_OCCUPATION_STATUS_VTABLE_RVA: usize = 0x296DD50;
/// CCountryOccupationStatus secondary vtable RVA (stored at status+0x18).
pub const DOMAIN66_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA: usize = 0x296DD88;
/// Offset within CCountryOccupationStatus holding the secondary vtable.
pub const DOMAIN66_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF: usize = 0x18;
/// CCountryOccupationStatus+0x48: occupation-data container pointer.
pub const DOMAIN66_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF: usize = 0x48;
/// CCountryOccupationData primary vtable RVA.
pub const DOMAIN66_OCCUPATION_DATA_VTABLE_RVA: usize = 0x296DD00;
/// CCountryOccupationData+0x08: back-ref to its parent CCountryOccupationStatus.
pub const DOMAIN66_OCCUPATION_DATA_PARENT_STATUS_OFF: usize = 0x08;
/// CCountryOccupationData+0x10: occupied-country tag_db_id.
pub const DOMAIN66_OCCUPATION_DATA_OCCUPIED_TAG_OFF: usize = 0x10;
/// CCountryOccupationData+0xB0: PDX open-addressing occupation-law hashmap
/// header. +0xB0 u64 bucket_data ptr, +0xB8 u32 size, +0xBC u32 capacity.
/// Buckets are 0x18 bytes: state_id u32 @+0x08, law* u64 @+0x10 (0 == empty).
pub const DOMAIN66_OCCUPATION_DATA_LAW_MAP_OFF: usize = 0xB0;
/// Byte offset (from the law-map header) to the u32 hashmap size.
pub const DOMAIN66_LAW_MAP_SIZE_OFF: usize = 0x08;
/// Byte offset (from the law-map header) to the u32 hashmap capacity.
pub const DOMAIN66_LAW_MAP_CAP_OFF: usize = 0x0C;
/// Occupation-law hashmap bucket stride.
pub const DOMAIN66_LAW_MAP_BUCKET_STRIDE: usize = 0x18;
/// Bucket +0x08: state_id (join key).
pub const DOMAIN66_LAW_MAP_BUCKET_STATE_ID_OFF: usize = 0x08;
/// Bucket +0x10: CBuildingOccupationLaw* (0 == empty/tombstone slot). The
/// negative control (reading the law ptr at bucket+0x00 instead) gave 0/1155
/// valid-law hits, proving +0x10 is the true slot.
pub const DOMAIN66_LAW_MAP_BUCKET_LAW_PTR_OFF: usize = 0x10;
/// CCountryOccupationData+0xD8: garrison-list container pointer.
pub const DOMAIN66_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF: usize = 0xD8;
/// CBuildingOccupationLaw vtable RVA (each accepted law ptr must match).
pub const DOMAIN66_OCCUPATION_LAW_VTABLE_RVA: usize = 0x2928218;
/// CBuildingOccupationLaw+0x10: PDX (SSO) law-token string.
pub const DOMAIN66_OCCUPATION_LAW_NAME_OFF: usize = 0x10;
/// CStateGarrisonData vtable RVA.
pub const DOMAIN66_STATE_GARRISON_DATA_VTABLE_RVA: usize = 0x296DCB0;
/// CStateGarrisonData+0x50: CState pointer (state-root join key).
pub const DOMAIN66_STATE_GARRISON_DATA_STATE_OFF: usize = 0x50;
/// CStateGarrisonData+0x58: back-ref to the CCountryOccupationData it belongs to.
pub const DOMAIN66_STATE_GARRISON_DATA_OCCUPATION_DATA_OFF: usize = 0x58;
/// CStateGarrisonData+0x70: manpower-pool sub-map pointer. +0x00 u32 supplying
/// tag_db_id, +0x04 u32 manpower value (value redundant with committed
/// army_manpower_need; only the supplying tag is emitted by d66).
pub const DOMAIN66_STATE_GARRISON_DATA_MANPOWER_POOL_OFF: usize = 0x70;
/// Manpower-pool sub +0x00: supplying-country tag_db_id.
pub const DOMAIN66_MANPOWER_POOL_SOURCE_TAG_OFF: usize = 0x00;
/// CState+0x58: state id (validated against this frame's state roots).
pub const DOMAIN66_STATE_ID_OFF: usize = 0x58;
/// CState vtable RVA (state-root validation).
pub const DOMAIN66_STATE_VTABLE_RVA: usize = 0x29206C0;
/// Bounded qword-scan length for the occupation-data and garrison-list
/// containers (matches the committed d52 walk).
pub const DOMAIN66_GARRISON_CONTAINER_SCAN_LEN: usize = 0x300;
/// Blob length read from each candidate CStateGarrisonData.
pub const DOMAIN66_STATE_GARRISON_READ_LEN: usize = 0x100;
/// Fail-closed ceiling on the occupation-law hashmap capacity (a cap beyond
/// this is a torn/aliased read and the whole map is skipped).
pub const DOMAIN66_MAX_LAW_MAP_CAP: u32 = 65536;
/// Maximum plausible state_id accepted from a law bucket (D5 guard: (0,70000)).
pub const DOMAIN66_MAX_STATE_ID: u32 = 70000;
/// Maximum plausible occupied/supplying tag_db_id (matches the d52 guard).
pub const DOMAIN66_MAX_TAG_DB_ID: u32 = 1024;
/// Fail-closed ceiling on the CCountryOccupationData records walked per country
/// (bounds the bounded container scan's accepted-record count).
pub const DOMAIN66_MAX_OCCUPATION_DATA_RECORDS: usize = 4096;

// --- W6 domain 68 session_header_live: TOP-LEVEL session header --------------
// A NEW rooted top-level (non-per-country) domain that snapshots the mostly-
// static save/session header off the CGameState singleton (agent D3, all fields
// live-validated vs the D04 melt oracle at discovery). One record per frame,
// last-value-wins in the fold (static fields never change; player/ideology/
// cosmetic_tag are change-detected downstream from the last emitted value).
// Root: gs = u64[module_base + DOMAIN57_CGAMESTATE_SINGLETON_RVA (0x3316E90)].
//
// player:       CHuman(gs+0xA8) +0x70 = u32 controlled-country tag_db_id
//               (melt player="D04" == tag_db_id 368).
// ideology:     CIdeologyGroup(gs+0xC8) +0x18 = inline MSVC std::string
//               (melt ideology=fascism).
// cosmetic_tag: gs+0x70 = inline MSVC std::string (melt "FRA_VICHY").
// dlcs:         gs+0xD4 = u32 bitmask (melt 257535 = 0x3EDFF).
// start_date:   gs+0x4A8 = u32 native hour-count (melt 1936.1.1.12 == 60759371;
//               the replay derives the calendar date with the shared codec).
// save_version: gs+0x58 = u32 (melt 33). Provisional (small int, single save).
// session:      gs+0x490 = u32 (melt 43695). Provisional (weak int, single save).
// version:      module-static .rdata NUL-terminated ascii @ RVA 0x2701108
//               ("Operation Postern v1.19.2.0.a729"); best-effort build banner,
//               NOT a save-header field (the melt "(d245)" checksum suffix is
//               assembled separately). Guarded by module-bounds + prefix check.
/// CHuman pointer offset within CGameState.
pub const DOMAIN68_CGAMESTATE_HUMAN_OFF: usize = 0xA8;
/// CHuman+0x70: controlled-country tag_db_id (u32).
pub const DOMAIN68_CHUMAN_PLAYER_TAG_OFF: usize = 0x70;
/// CIdeologyGroup pointer offset within CGameState.
pub const DOMAIN68_CGAMESTATE_IDEOLOGY_GROUP_OFF: usize = 0xC8;
/// CIdeologyGroup+0x18: inline MSVC std::string ideology-group token.
pub const DOMAIN68_IDEOLOGY_GROUP_TOKEN_OFF: usize = 0x18;
/// CGameState+0x70: inline MSVC std::string player/host cosmetic tag.
pub const DOMAIN68_CGAMESTATE_COSMETIC_TAG_OFF: usize = 0x70;
/// CGameState+0xD4: u32 enabled-DLC bitmask.
pub const DOMAIN68_CGAMESTATE_DLCS_OFF: usize = 0xD4;
/// CGameState+0x4A8: u32 campaign start_date native hour-count.
pub const DOMAIN68_CGAMESTATE_START_DATE_OFF: usize = 0x4A8;
/// CGameState+0x58: u32 save_version.
pub const DOMAIN68_CGAMESTATE_SAVE_VERSION_OFF: usize = 0x58;
/// CGameState+0x490: u32 session id.
pub const DOMAIN68_CGAMESTATE_SESSION_OFF: usize = 0x490;
/// Module .rdata RVA of the build banner cstr ("Operation Postern v...").
pub const DOMAIN68_VERSION_BANNER_RVA: usize = 0x2701108;
/// Maximum bytes scanned for the version banner NUL terminator.
pub const DOMAIN68_MAX_VERSION_LEN: usize = 128;
/// Expected ASCII prefix of the version banner (guards a bad RVA / patch drift;
/// a banner not starting with this is treated as unavailable, fail-closed).
pub const DOMAIN68_VERSION_BANNER_PREFIX: &str = "Operation Postern";
/// Maximum accepted ideology/cosmetic MSVC-string length (a longer read is torn
/// and the field is badged absent).
pub const DOMAIN68_MAX_TOKEN_LEN: usize = 128;
/// Fail-closed ceiling on the player tag_db_id (a value beyond this is a torn
/// read; the player field is badged absent).
pub const DOMAIN68_MAX_PLAYER_TAG_DB_ID: u32 = 2_048;

// --- W6 domain 69 tech_sharing_groups_live: TOP-LEVEL research-sharing groups -
// A NEW rooted top-level domain (agent D8, 9/9 groups + membership multisets
// validated vs the D04 melt oracle at discovery). Root:
// gs = u64[module_base + 0x3316E90]; CGameState+0x3A0 -> tech-sharing manager.
// The manager holds 9 inline pointer slots @ +0x00..+0x40 (stride 8), each ->
// CTechnologySharingGroup (vtable RVA 0x270B720). Per group:
//   +0x08 = u32 group id token (a FNV/db token; the group's static id string
//           lives behind the +0x48 template ptr and is NOT decoded here),
//   +0x18 = begin ptr of the member array (u32 tag_db_ids, stride 4),
//   +0x20 = u32 member count.
// A full-heap vtable census found EXACTLY 9 such objects == melt's 9 groups.
/// CGameState+0x3A0: pointer to the tech-sharing manager.
pub const DOMAIN69_CGAMESTATE_TECH_SHARING_MANAGER_OFF: usize = 0x3A0;
/// CTechnologySharingGroup vtable RVA (each of the 9 slot pointers must match).
pub const DOMAIN69_TECH_SHARING_GROUP_VTABLE_RVA: usize = 0x270B720;
/// Number of inline group pointer slots in the manager (fixed at 9).
pub const DOMAIN69_TECH_SHARING_GROUP_SLOTS: usize = 9;
/// Byte offset of the first group pointer slot within the manager.
pub const DOMAIN69_TECH_SHARING_MANAGER_SLOTS_OFF: usize = 0x00;
/// Group +0x08: u32 group id token.
pub const DOMAIN69_TECH_SHARING_GROUP_ID_TOKEN_OFF: usize = 0x08;
/// Group +0x18: begin ptr of the member tag_db_id array (u32, stride 4).
pub const DOMAIN69_TECH_SHARING_GROUP_MEMBERS_BEGIN_OFF: usize = 0x18;
/// Group +0x20: u32 member count.
pub const DOMAIN69_TECH_SHARING_GROUP_MEMBERS_COUNT_OFF: usize = 0x20;
/// Fail-closed ceiling on a group's member count (a count beyond this is a torn
/// read; that group is skipped).
pub const DOMAIN69_MAX_MEMBERS_PER_GROUP: u32 = 512;
/// Fail-closed ceiling on a member tag_db_id.
pub const DOMAIN69_MAX_MEMBER_TAG_DB_ID: u32 = 2_048;

/// Domain 53 diplomacy/intel/operative live roots for supported build
/// 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad.
///
/// Provenance: 2026-07-03 live re-validation artifacts under
/// `extractor/native/runs/domain53_revalidation_870ebf_20260703/`.
/// Country, agency, cryptology, operative lifecycle, relation/faction
/// structure, war row-shape roots, and incoming diplomatic action queues are
/// current-build evidence. Operation detail and spy-network/intel-map
/// percentages remain unvalidated and must stay fail-closed. See the lane
/// `REVALIDATION_870EBF.md` for the constant-by-constant audit table. These
/// constants are raw layout evidence; lane code must keep unresolved semantics
/// badged instead of renaming fields early.
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
/// Pointer at `CRelationStatus+0x288` to the heap-allocated `CSubjectRelation`.
/// The committed inline-window vtable scan never matches (the CSubjectRelation
/// vtable is NOT embedded inline in the relation object), so subjects must be
/// reached through this pointer hop. Live-validated 38/38 pairs on 870ebf.
pub const DOMAIN53_RELATION_SUBJECT_PTR: usize = 0x288;
/// `CSubjectRelation+0x20` i32 engine-hour subject start_date. Emit RAW; the
/// replay layer derives the display date (raw = the game calendar hours + a fixed
/// epoch offset; deltas match the melt to the hour). Live-validated 38/38.
pub const DOMAIN53_SUBJECT_START_DATE: usize = 0x20;

/// CFactionSystem: `CGameState+0x3F8 -> CFactionSystem`. The faction pointer
/// array (`CFaction*[]`) begins at `+0x20`; `+0x28` packs the element count as
/// two u32 fields (size lo32, capacity hi32). Per faction: id/type at `+0x08`
/// (id = HI32, type = LO32, real factions carry type == 88), display name is an
/// MSVC std::string at `+0x38`, members are `CCountry*[]` at `+0x58` with a u32
/// count at `+0x64`, and the leader is `members[0]`. Live-validated 6/6 with
/// exact member multisets + leaders (GER/ENG/SOV/JAP/CHI/ETH) on 870ebf.
pub const DOMAIN53_CGAMESTATE_FACTION_SYSTEM: usize = 0x3F8;
pub const DOMAIN53_FACTION_ARRAY: usize = 0x20;
pub const DOMAIN53_FACTION_COUNT: usize = 0x28;
pub const DOMAIN53_FACTION_ID: usize = 0x08;
pub const DOMAIN53_FACTION_NAME: usize = 0x38;
pub const DOMAIN53_FACTION_MEMBERS: usize = 0x58;
pub const DOMAIN53_FACTION_MEMBERS_SIZE: usize = 0x64;
/// Real factions carry this LO32 type discriminator; the array tail past the
/// live element count holds nulls/garbage with other type values, so gate on it.
pub const DOMAIN53_FACTION_TYPE_EXPECTED: u32 = 88;
/// Upper sanity bound on the faction vector size/capacity (guards a torn count).
pub const DOMAIN53_FACTION_MAX_SLOTS: u32 = 512;
/// Upper sanity bound on a faction's member count.
pub const DOMAIN53_FACTION_MAX_MEMBERS: u32 = 256;
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
// Wave-5 running-operations (validated live 870ebf, paused D04, melt oracle;
// runs/attemptcrack_operations_intel/EVIDENCE_v2 + wave5_independent_verify:
// exactly 8 COperationInstance, target@+0x50 8/8 exact SOV/DEN/ITA/POR/BRA/
// ENG/CHL/PER). COperationInstance heap object (vtable RVA below), handle u64
// @+0x08 with id=HI32 / type=LO32 MUST==4713; operation-def ptr @+0x48 ->
// COperation (vtable RVA below, def_ref vtable-check); associated country ptr
// @+0x50 -> CCountry (resolve to tag_db_id at +0x08). The +0x10 owner field is
// REJECTED (independent re-read: low byte=1 on all 8, polluted high bytes) and
// is NOT read.
pub const DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA: usize = 0x29A76C0;
pub const DOMAIN53_OPERATION_DEF_VTABLE_RVA: usize = 0x2928C98;
pub const DOMAIN53_OPERATION_INSTANCE_HANDLE: usize = 0x08;
pub const DOMAIN53_OPERATION_INSTANCE_HANDLE_TYPE: u32 = 4_713;
pub const DOMAIN53_OPERATION_INSTANCE_DEF: usize = 0x48;
pub const DOMAIN53_OPERATION_INSTANCE_TARGET_COUNTRY: usize = 0x50;
// C3 incoming diplomatic-action queue (validated live 870ebf, paused D04,
// melt oracle: 5/5 populated recipients, 7/7 actions, 0 false positives).
// CCountry+0xFF8 -> CIncomingDiplomaticActionQueue; queue+0x08 points to the
// CIncomingDiplomaticAction* array and queue+0x10 packs size low32/cap high32.
// queue+0x20 low32 must equal the recipient/owner tag_db_id on populated
// queues. Each action has vtable RVA below, handle @+0x08 (id=HI32,
// type=LO32 MUST==4713), and body @+0x18 with actor/recipient/date fields.
pub const DOMAIN53_INCOMING_ACTION_QUEUE: usize = 0xFF8;
pub const DOMAIN53_INCOMING_ACTION_QUEUE_DATA: usize = 0x08;
pub const DOMAIN53_INCOMING_ACTION_QUEUE_PACKED_SIZE_CAP: usize = 0x10;
pub const DOMAIN53_INCOMING_ACTION_QUEUE_OWNER_GUARD: usize = 0x20;
pub const DOMAIN53_INCOMING_ACTION_VTABLE_RVA: usize = 0x2A209A0;
pub const DOMAIN53_INCOMING_ACTION_HANDLE: usize = 0x08;
pub const DOMAIN53_INCOMING_ACTION_HANDLE_TYPE: u32 = 4_713;
pub const DOMAIN53_INCOMING_ACTION_BODY: usize = 0x18;
pub const DOMAIN53_INCOMING_ACTION_BODY_TYPE: usize = 0x10;
pub const DOMAIN53_INCOMING_ACTION_ACTOR: usize = 0x14;
pub const DOMAIN53_INCOMING_ACTION_ORIGINAL_ACTOR: usize = 0x18;
pub const DOMAIN53_INCOMING_ACTION_RECIPIENT: usize = 0x1C;
pub const DOMAIN53_INCOMING_ACTION_ORIGINAL_RECIPIENT: usize = 0x20;
pub const DOMAIN53_INCOMING_ACTION_DATE: usize = 0x30;
pub const DOMAIN53_INCOMING_ACTION_LAST_COMMAND_DATE: usize = 0x48;
pub const DOMAIN53_MAX_INCOMING_ACTION_ROWS: u32 = 4_096;

// C4 intel-network graph (validated live 870ebf, paused D04,
// crackqueue2_c4_intel). The serialized save's strategic_operatives block is
// backed by CStrategicOperative objects, each owning a vector of
// CCountryIntelNetwork pointers. Graph vertices carry the covered state_id set.
pub const DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA: usize = 0x298B448;
pub const DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA: usize = 0x298AB48;
pub const DOMAIN53_STRATEGIC_OPERATIVE_INDEX: usize = 0x08;
pub const DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_VECTOR: usize = 0x10;
pub const DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_CAPACITY: usize = 0x18;
pub const DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_SIZE: usize = 0x1C;
pub const DOMAIN53_COUNTRY_INTEL_NETWORK_TARGET: usize = 0x08;
pub const DOMAIN53_COUNTRY_INTEL_NETWORK_GRAPH: usize = 0x18;
pub const DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_DATA: usize = 0x38;
pub const DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_COUNT: usize = 0x40;
pub const DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE: usize = 0x10;
pub const DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STATE_ID: usize = 0x00;
pub const DOMAIN53_MAX_INTEL_NETWORKS: u32 = 4_096;
pub const DOMAIN53_MAX_INTEL_NETWORK_STATES: u32 = 2_048;
pub const DOMAIN53_HEAP_LO: usize = 0x10000000000;
pub const DOMAIN53_HEAP_HI: usize = 0x7FF000000000;
pub const DOMAIN53_MAX_COUNTRY_TABLE_SLOTS: u32 = 4_096;
pub const DOMAIN53_MAX_VECTOR_ROWS: u32 = 8_192;

/// Domain 60 capitulation/war-state offsets for the supported 870ebf profile.
///
/// Provenance: FINAL_SWEEP_RESULTS_2026-07-04.md §T1 — the per-country
/// capitulation block hangs off the CCountry diplomacy sub-object
/// (`D = u64[CCountry+0xF88]`, the same pointer as DOMAIN53_RELATION_MANAGER).
/// `capitulated` u8 @ D+0x2E0 was orchestrator-verified live: exactly the
/// melt's 15 capitulated tags across all 437 live countries, 0 mismatches.
/// Sibling fields validated in the same sweep (dates 0-mismatch over 437,
/// legitimacy 8/8 exact, exile counts hit all 8 exile countries). Evidence:
/// `extractor/native/runs/finalsweep_t1_capitulation/EVIDENCE.md`.
pub const DOMAIN60_DIPLOMACY_SUB_OBJECT: usize = DOMAIN53_RELATION_MANAGER;
pub const DOMAIN60_DIPLOMACY_LEGITIMACY: usize = 0x1B0;
pub const DOMAIN60_DIPLOMACY_EXILE_ARMY_LEADERS: usize = 0x1B8;
pub const DOMAIN60_DIPLOMACY_LAST_SURRENDER_DATE: usize = 0x2B8;
pub const DOMAIN60_DIPLOMACY_FACTION_JOIN_DATE: usize = 0x2D0;
pub const DOMAIN60_DIPLOMACY_CAPITULATED: usize = 0x2E0;
pub const DOMAIN60_DIPLOMACY_CAPITULATED_DATE: usize = 0x2F0;
/// War-row start-date VALUE (i32 engine hour counter inside the inline date
/// object whose vtable sits at war-row+0x08). Validated 2026-07-04 during E1
/// wiring: exact match vs melt `war_relation.start_date` for ALL 958/958 melt
/// pairs across both date-object kinds (289 CGregorianDate + 669 CGameDate);
/// every other candidate offset in row+0x0C..row+0x38 scored 0/958. Evidence:
/// `extractor/native/runs/e1_wire_capitulation_war/EVIDENCE.md`.
pub const DOMAIN60_WAR_START_DATE: usize = 0x10;

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
/// W4 (2026-07-05, agent D5): CBuildingProductionLine+0x58 = u32 native
/// hour-count of the construction line's created_date (same epoch encoding as
/// the CGameState date word: gate 1944.2.11.18 == raw 60830441; the replay
/// derives the calendar date raw->Y.M.D.H with the shared date codec). D5 live
/// validation: 105/105 construction lines across GER/ENG/SOV/USA/ITA, 0 wrong,
/// each keyed by unique line_id; entropy 136 distinct dates over 147 lines
/// (rejects the "last-updated" hypothesis). +0x10 holds a constant that does
/// NOT track the date and was rejected; +0x58 was the ONLY [0,0xC0) offset
/// matching all lines. Melt anchor: line 966792 line_id 18354
/// created_date=1943.9.23.4 -> live +0x58 == 60827043.
pub const LINE_CREATED_DATE: usize = 0x58;
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
