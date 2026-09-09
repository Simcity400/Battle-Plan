//! Anonymous foreign-homebase record in CCountrySupplySystem + 0x1e0.
//!
//! The old reader preserved seven numeric fields as i64. Their display units
//! and the record's C++ type name were not established by this harvest.

/// Foreign-homebase record stride; old attach_log.rs:21475 and D44 walk at 121878-121929.
pub const STRIDE: usize = 0x70;
/// Foreign-homebase u32 province id; old attach_log.rs:121891.
pub const PROVINCE: usize = 0x00;
/// Foreign-homebase supply i64; old attach_log.rs:121923.
pub const SUPPLY: usize = 0x08;
/// Foreign-homebase start i64; old attach_log.rs:121924.
pub const START: usize = 0x10;
/// Foreign-homebase penalty i64; old attach_log.rs:121925.
pub const PENALTY: usize = 0x18;
/// Foreign-homebase added-penalty i64; old attach_log.rs:121926.
pub const ADDED_PENALTY: usize = 0x20;
/// Foreign-homebase source MSVC string; old attach_log.rs:121906-121914 reads a stable string snapshot.
pub const SOURCE: usize = 0x28;
/// Foreign-homebase duration i64; old attach_log.rs:121927.
pub const DURATION: usize = 0x48;
/// Foreign-homebase duplicate u32 province id checked against +0; old attach_log.rs:121896.
pub const PROVINCE_COPY: usize = 0x50;
/// Foreign-homebase hours i64; old attach_log.rs:121928.
pub const HOURS: usize = 0x58;
/// Foreign-homebase decay i64; old attach_log.rs:121929.
pub const DECAY: usize = 0x60;
/// Foreign-homebase base flag u8 in {0,1}; old attach_log.rs:121900-121904.
pub const BASE: usize = 0x68;
