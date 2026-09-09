//! Railway visual aggregate row consumed by FUN14164aa70; C++ type is unconfirmed.
//! This row is render-derived and is not a proven hourly simulation table.

/// FUN14164aa70 consumes a CPdxArray of these rows, rail_supply_arrow.rs:11,415-449,636-671.
pub const CONSUMER_RVA: usize = 0x0164_aa70;
/// FUN14164aa70 entry bytes checked by the old hook, rail_supply_arrow.rs:15-17.
pub const CONSUMER_SENTINEL: [u8; 16] = [
    0x88, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// Railway aggregate row stride used by rail_supply_arrow.rs:22,645.
pub const STRIDE: usize = 0x30;
/// Railway aggregate selected arrow pointer written during the consumer, checked by rail_supply_arrow.rs:550.
pub const ARROW: usize = 0;
/// Railway aggregate ordered province-id array pointer, read by rail_supply_arrow.rs:646.
pub const PROVINCES: usize = 8;
/// Railway aggregate ordered province count i32, read by rail_supply_arrow.rs:647.
pub const PROVINCE_COUNT: usize = 0x14;
/// Railway aggregate native province-id u32 element stride, read by rail_supply_arrow.rs:658.
pub const PROVINCE_STRIDE: usize = 4;
/// Railway aggregate effective level i32, read by rail_supply_arrow.rs:668.
pub const EFFECTIVE_LEVEL: usize = 0x20;
/// Railway aggregate target level i32, read by rail_supply_arrow.rs:669.
pub const TARGET_LEVEL: usize = 0x24;
/// Railway aggregate cooldown selector i32, read by rail_supply_arrow.rs:670.
pub const COOLDOWN_SELECTOR: usize = 0x28;
/// Railway aggregate flags u32, read by rail_supply_arrow.rs:671.
pub const FLAGS: usize = 0x2c;
/// Railway aggregate damage-eligible flag checked against positive damage by rail_supply_arrow.rs:552.
pub const DAMAGE_FLAG: u32 = 8;
