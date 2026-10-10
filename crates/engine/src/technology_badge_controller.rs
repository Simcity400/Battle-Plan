//! Technology badge controller with a separate source-owner argument.

/// Badge controller producer.
pub const UPDATE_RVA: usize = 0x01bc_1a00;
/// Exact producer-entry sentinel.
pub const UPDATE_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x20\x55\x56\x57\x41\x54\x41\x55\x41\x56\x41\x57";
