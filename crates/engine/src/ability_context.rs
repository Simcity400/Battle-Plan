//! Context object supplied to the command-ability invalid-context probe.
//! Its concrete type is unknown.

/// Invalid-context byte probe; observers retain its context argument and low-byte result.
pub const INVALID_PROBE_RVA: usize = 0x00b5_8ca0;
/// Entry sentinel covering the flag and conditional target-pointer accesses.
pub const INVALID_PROBE_SENTINEL: [u8; 16] = [
    0x80, 0xb9, 0xfa, 0x03, 0x00, 0x00, 0x00, 0x74, 0x0c, 0x48, 0x8b, 0x89, 0x40, 0x02, 0x00, 0x00,
];
/// Byte compared with zero; its full meaning is unknown.
pub const PROBE_FLAG: usize = 0x3fa;
/// Pointer loaded by the conditional path; its target type is unknown.
pub const PROBE_TARGET: usize = 0x240;
