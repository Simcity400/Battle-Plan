//! Assigned-operative array element; its engine class name is unproved.

/// COperationInstance assigned-operative record size; build_profile.rs:531 and read_operation_assigned_operatives advance 0x38 bytes.
pub const STRIDE: usize = 0x38;
/// Assigned-operative packed u64 reference at row+0; read_operation_assigned_operatives extracts low u32 type and high u32 id.
pub const HANDLE: usize = 0x00;
/// Assigned-operative low-u32 reference type; the old reader compares it to COperationInstance's 4713 without proving a distinct type.
pub const REFERENCE_TYPE: u32 = 4_713;
/// Assigned-operative raw selection word; 13EDEE0 stores its fourth argument at slot+8.
pub const SELECTION: usize = 0x08;
pub const WRITER_RVA: usize = 0x01bd_1cd0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub operation: u64,
    pub ordinal: u32,
    /// Full native reference; both-zero is an unassigned slot, retained as state.
    pub operative: u64,
    /// Opaque native bits; no inferred enum or display projection.
    pub selection: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proved_writer_rva_matches_ghidra_verified_address() {
        assert_eq!(WRITER_RVA, 0x1BD1CD0);
        assert_eq!((STRIDE, HANDLE, SELECTION), (0x38, 0x00, 0x08));
    }
}
