//! Owner of technology-mode tree views. Construction and owner-slot
//! relationships do not establish an hourly-state walk.

/// Canvas-owner constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00b5_2390;
/// Exact owner-constructor entry sentinel.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x20\x48\x89\x4c\x24\x08\x55\x56\x57\x41\x54\x41";
/// Mode-6 controller pointer.
pub const MODE6_CONTROLLER: usize = 0xf8;
/// Mode-7 controller pointer.
pub const MODE7_CONTROLLER: usize = 0x100;
