//! Railway visual aggregate row.
//! This is render-derived; no hourly simulation lifetime is established.

/// Consumer of an array of aggregate rows.
pub const CONSUMER_RVA: usize = 0x0164_aa70;
/// Exact consumer-entry instruction sentinel.
pub const CONSUMER_SENTINEL: [u8; 16] = [
    0x88, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// Aggregate row stride.
pub const STRIDE: usize = 0x30;
/// Selected arrow pointer written during consumption.
pub const ARROW: usize = 0;
/// Ordered province-ID array pointer.
pub const PROVINCES: usize = 8;
/// Ordered province count, i32.
pub const PROVINCE_COUNT: usize = 0x14;
/// Province-ID element stride, u32.
pub const PROVINCE_STRIDE: usize = 4;
/// Effective level, i32.
pub const EFFECTIVE_LEVEL: usize = 0x20;
/// Target level, i32.
pub const TARGET_LEVEL: usize = 0x24;
/// Cooldown selector, i32.
pub const COOLDOWN_SELECTOR: usize = 0x28;
/// Flags, u32.
pub const FLAGS: usize = 0x2c;
/// Damage-eligible flag, checked against positive damage.
pub const DAMAGE_FLAG: u32 = 8;
