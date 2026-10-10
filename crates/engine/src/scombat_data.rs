//! Retained combat-terminal history record from a reference-counted pool.
//! Country and orders-group histories retain pool indices for serialization.
//! These are ended-battle aggregates rather than per-tick lines or stable
//! live-combat references. No hourly history walk is installed.

/// Record vtable.
pub const VTABLE_RVA: usize = 0x0294_6fb8;
/// Copied record extent.
pub const SIZE: usize = 0x448;
/// First side manpower loss, i32; presentation selects attacker/defender using LAYOUT_SIDE.
pub const ATTACKER_MANPOWER_LOSS: usize = 0x10;
/// Second side manpower loss, i32; presentation selects attacker/defender using LAYOUT_SIDE.
pub const DEFENDER_MANPOWER_LOSS: usize = 0x220;
/// Embedded end-date object.
pub const END_DATE: usize = 0x428;
/// Raw end date, i32, used for elapsed-time text.
pub const END_DATE_RAW: usize = 0x430;
/// Second date subobject; exact meaning remains unestablished.
pub const LOCATION_DATE_OBJECT: usize = 0x438;
/// Province location, u32.
pub const LOCATION: usize = 0x440;
/// Winner-side byte, retained without enum reinterpretation.
pub const WINNER_SIDE: usize = 0x444;
/// Winter byte, retained without Boolean normalization.
pub const WINTER: usize = 0x445;
/// Result-selector byte.
pub const RESULT_SELECTOR: usize = 0x446;
/// Layout-side byte, retained without enum reinterpretation.
pub const LAYOUT_SIDE: usize = 0x447;
/// Overrun result selector.
pub const RESULT_OVERRUN: u8 = 0;
/// Normal result selector.
pub const RESULT_NORMAL: u8 = 1;

/// Normal-terminal writer.
pub const NORMAL_WRITER_RVA: usize = 0x00cc_c7f0;
/// Overrun-terminal writer.
pub const OVERRUN_WRITER_RVA: usize = 0x00cc_c100;
/// Record allocator.
pub const RECORD_ALLOCATOR_RVA: usize = 0x00cc_9f10;
/// Exact normal-terminal entry sentinel.
pub const NORMAL_WRITER_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55,
];
/// Exact overrun-terminal entry sentinel.
pub const OVERRUN_WRITER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
/// Exact allocator-entry sentinel.
pub const RECORD_ALLOCATOR_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x57, 0x41, 0x55, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x63, 0x05, 0xec, 0x9c, 0x65, 0x02,
];
