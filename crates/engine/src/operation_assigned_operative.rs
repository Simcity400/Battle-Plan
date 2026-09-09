//! Assigned-operative array element; its engine class name is unproved.

/// COperationInstance assigned-operative record size; build_profile.rs:531 and read_operation_assigned_operatives advance 0x38 bytes.
pub const STRIDE: usize = 0x38;
/// Assigned-operative packed u64 reference at row+0; read_operation_assigned_operatives extracts low u32 type and high u32 id.
pub const HANDLE: usize = 0x00;
/// Assigned-operative low-u32 reference type; the old reader compares it to COperationInstance's 4713 without proving a distinct type.
pub const REFERENCE_TYPE: u32 = 4_713;
