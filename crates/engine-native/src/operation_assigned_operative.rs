//! Assigned-operative array element.

/// Assigned-operative record stride.
pub const STRIDE: usize = 0x38;
/// Packed full reference with low u32 type and high u32 ID.
pub const HANDLE: usize = 0x00;
/// Low u32 reference type shared with operation-instance references.
pub const REFERENCE_TYPE: u32 = 4_713;
/// Raw assigned-operative selection word.
#[cfg(test)]
pub const SELECTION: usize = 0x08;
/// Assigned-slot writer omitting both-zero handles, zero selection and null optional values.
pub const WRITER_RVA: usize = 0x01c7_dac0;

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
    fn writer_rva_matches_supported_operation_layout() {
        assert_eq!(WRITER_RVA, 0x1C7DAC0);
        assert_eq!((STRIDE, HANDLE, SELECTION), (0x38, 0x00, 0x08));
    }
}
