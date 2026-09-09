//! CFlagManager for country, state and global scripted flags.
//!
//! Sources: archived `lanes/scripted_state_flags_variables_live/
//! scripted_state_domain51_collector.rs:25-30` and `collector/src/attach_log.rs:
//! 106279-106604`. The old collector validates RTTI, then learns the vtable;
//! no fixed vtable RVA or hourly consistency proof is established here.

/// CFlagManager RTTI name; domain51_flag_scope confirms this string before caching the vtable.
pub const RTTI: &str = ".?AVCFlagManager@@";
/// CFlagManager primary vtable pointer; domain51_stable_flag_manager_head reads +0x00.
pub const VTABLE: usize = 0x00;
/// CFlagManager dense flag-row array pointer; the D51 header and row walk use +0x08.
pub const ROWS: usize = 0x08;
/// CFlagManager packed u64 slot/entry counts; domain51_stable_flag_header reads +0x10.
pub const PACKED_SLOT_ENTRY_COUNT: usize = 0x10;
/// CFlagManager u32 slot count; domain51_flag_manager_head_from_samples takes the low word at +0x10.
pub const SLOT_COUNT: usize = 0x10;
/// CFlagManager u32 entry count; domain51_flag_manager_head_from_samples takes the high word at +0x14.
pub const ENTRY_COUNT: usize = 0x14;
/// CFlagManager +0x18 token-record-vtable candidate; old resolve_compact_air_token_record_vtable uses it without RTTI proof.
pub const TOKEN_RECORD_VTABLE_CANDIDATE: usize = 0x18;
