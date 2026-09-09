//! `NProject::CProgram`, called a project facility by the old collector.
//!
//! Harvested from `collector/src/attach_log/offsets.rs:1213-1252`,
//! `compact_contract/src/build_profile.rs:235-250`, and the archived collector's
//! `read_project_facility_row` / `decode_special_project_program_snapshots_detailed`.
//! These are retained layout observations; hourly consistency is not yet proved.

/// NProject::CProgram primary vtable; pinned executable RTTI confirms the old facility vtable.
pub const VTABLE_RVA: usize = 0x0295_ac30;
/// CProgram reference type discriminator 83; checked by archived `project_facility_pin`.
pub const REFERENCE_TYPE: u32 = 83;
/// CProgram +0x08 reference type u32; checked by archived `project_facility_pin`.
pub const TYPE: usize = 0x08;
/// CProgram +0x0c local id u32; read by archived project facility and Domain 65 walkers.
pub const ID: usize = 0x0c;
/// CProgram +0x18 owner reference u32; Domain 65 compares this with the +0x170 copy.
pub const OWNER_PRIMARY: usize = 0x18;
/// CProgram +0x1c province reference u32; Domain 65 compares this with the +0x174 copy.
pub const PROVINCE_PRIMARY: usize = 0x1c;
/// CProgram +0x20 MSVC building token string; read by archived `read_project_facility_row`.
pub const BUILDING: usize = 0x20;
/// CProgram +0x50 active project handle u32; read by archived `read_project_facility_row`.
pub const ACTIVE_PROJECT_HANDLE: usize = 0x50;
/// CProgram +0x54 active flag read as u32, value 1 means active in the archived facility reader.
pub const ACTIVE_FLAG: usize = 0x54;
/// CProgram +0x58 scientist character reference type u32; old reader expects CCharacter type 73.
pub const SCIENTIST_REFERENCE_TYPE: usize = 0x58;
/// CProgram +0x5c scientist character reference id u32; read by archived facility reader.
pub const SCIENTIST_REFERENCE_ID: usize = 0x5c;
/// CProgram +0x170 owner reference u32; independently compared with +0x18 by Domain 65.
pub const OWNER_COPY: usize = 0x170;
/// CProgram +0x174 province reference u32; independently compared with +0x1c by Domain 65.
pub const PROVINCE_COPY: usize = 0x174;
pub const UNREAD_PROTOTYPE_REWARDS: usize = 0x178;
