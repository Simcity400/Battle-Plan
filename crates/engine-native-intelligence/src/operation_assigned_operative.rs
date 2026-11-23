
use engine_sites::Rva;
pub const STRIDE: usize = 0x38;
pub const HANDLE: usize = 0x00;
pub const REFERENCE_TYPE: u32 = 4_713;
#[cfg(test)]
pub const SELECTION: usize = 0x08;
pub const WRITER_RVA: Rva = Rva(0x01c7_dac0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub operation: u64,
    pub ordinal: u32,
    pub operative: u64,
    pub selection: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_rva_matches_supported_operation_layout() {
        assert_eq!(WRITER_RVA.0, 0x1C7DAC0);
        assert_eq!((STRIDE, HANDLE, SELECTION), (0x38, 0x00, 0x08));
    }
}
