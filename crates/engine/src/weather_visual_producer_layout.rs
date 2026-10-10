//! Strategic-region weather presentation producer, separate from
//! simulation weather. No recorded table or hook is enabled.

/// Region pointer, followed to its ID at +0x58.
pub const REGION: usize = 0x30;
/// Primary/fading entity-pair array pointer.
pub const ENTITY_PAIRS: usize = 0x48;
/// Entity-pair capacity, u32.
pub const PAIR_CAPACITY: usize = 0x50;
/// Entity-pair count, u32.
pub const PAIR_COUNT: usize = 0x54;
/// Weather code, u32, retained at full width.
pub const WEATHER_CODE: usize = 0x60;
/// Visual producer entry.
pub const WEATHER_VISUAL_PRODUCER_RVA: u32 = 0x0166_EC50;
/// Exact visual-producer entry sentinel.
pub const WEATHER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x50, 0x10, 0x55, 0x53, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57,
];
