//! COperationInstance, the persistent espionage-operation object.
//!
//! The pinned executable's RTTI identifies the primary vtable. Fields come from
//! `compact_contract/src/build_profile.rs:515-540` and the old D53
//! `extractor.rs::read_operations`; they still need hourly consistency and save
//! validation. The old reader derived phase/progress/status values from these
//! fields, so its fixed-five progress output is not a native stored field.

/// COperationInstance primary vtable; build_profile.rs:515 and pinned a729 RTTI confirm the class, subobject offset zero.
pub const VTABLE_RVA: usize = 0x029a_76c0;
/// COperationInstance packed u64 reference at +8; read_operations reads low u32 type and high u32 instance id.
pub const HANDLE: usize = 0x08;
/// COperationInstance reference type 4713; build_profile.rs:518 and read_operations authenticate the packed handle.
pub const REFERENCE_TYPE: u32 = 4_713;
/// COperationInstance preparation-current raw i64; read_operations reads +30 without proving an independent display scale.
pub const PREPARATION_CURRENT: usize = 0x30;
/// COperationInstance preparation-required raw i64; read_operations compares +38 against the current amount before deriving progress.
pub const PREPARATION_REQUIRED: usize = 0x38;
/// COperationInstance terminal byte; read_operations at extractor.rs:7898 accepts zero or one at +42.
pub const TERMINAL: usize = 0x42;
/// COperationInstance COperation definition pointer; read_operations authenticates the +48 pointee's vtable.
pub const DEFINITION: usize = 0x48;
/// COperationInstance owning CCountry pointer; read_operations joins +50 through CCountry's raw tag id.
pub const OWNER_COUNTRY: usize = 0x50;
/// COperationInstance target-country raw u32 tag id; read_operations reads +58 and joins the country table.
pub const TARGET_COUNTRY_ID: usize = 0x58;
/// COperationInstance optional target CProvince pointer; read_operation_target_state authenticates +60 before following CProvince::STATE.
pub const TARGET_PROVINCE: usize = 0x60;
/// COperationInstance start-hour raw u32; read_operations reads +70, retaining engine hours rather than derived elapsed days.
pub const START_HOUR: usize = 0x70;
/// COperationInstance duration raw u32 days; read_operations reads +80, with zero interpreted as preparation rather than running.
pub const DURATION_DAYS: usize = 0x80;
/// COperationInstance assigned-operative record-array pointer; read_operation_assigned_operatives reads +E0 with 0x38-byte elements.
pub const ASSIGNED_OPERATIVE_DATA: usize = 0xe0;
/// COperationInstance assigned-operative u32 count; read_operation_assigned_operatives reads +EC, not an inferred capacity.
pub const ASSIGNED_OPERATIVE_COUNT: usize = 0xec;
/// COperationInstance phase-count raw u32; read_operations reads +104 and derives a phase index from duration and engine date.
pub const PHASE_COUNT: usize = 0x104;
/// COperationInstance inline resource map; operation_is_prepared at extractor.rs:8889 uses a header rooted at +110.
pub const RESOURCE_MAP: usize = 0x110;
/// COperationInstance requirement-record array pointer; operation_is_prepared reads +158 with 0x20-byte elements.
pub const REQUIREMENT_DATA: usize = 0x158;
/// COperationInstance requirement-record u32 count; operation_is_prepared reads +164 before walking optional and required resources.
pub const REQUIREMENT_COUNT: usize = 0x164;
