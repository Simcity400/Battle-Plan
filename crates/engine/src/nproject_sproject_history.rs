//! `NProject::SProjectHistory`; archived `project_reward_history_row_from_snapshot`.

/// NProject::SProjectHistory primary vtable; confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x0296_c220;
/// SProjectHistory vector row stride 0x58; archived serializer-owned history walk.
pub const STRIDE: usize = 0x58;
/// SProjectHistory +0x08 prototype reward handle u32; read by archived history row decoder.
pub const PROTOTYPE_REWARD_HANDLE: usize = 0x08;
/// SProjectHistory +0x0c option handle u32; read by archived history row decoder.
pub const OPTION_HANDLE: usize = 0x0c;
/// SProjectHistory +0x10 inline SProjectContext; independently vtable-checked by archived decoder.
pub const CONTEXT: usize = 0x10;
/// SProjectHistory +0x48 raw date i32; read by archived history row decoder.
pub const DATE: usize = 0x48;
