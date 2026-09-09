//! CCountryExperienceStatus from the old country-core scalar walk.
//! The old reader copied i32 channels despite eight-byte spacing and applied /32768.
//! Complete native widths and scale remain unproved; do not repeat its narrowing.

/// CCountryExperienceStatus primary vtable, confirmed by pinned-exe RTTI; old offsets.rs:59.
pub const VTABLE_RVA: usize = 0x0299_11c0;
/// CCountryExperienceStatus army experience; old read_country_experience_status reads EXP_ARMY from offsets.rs:266.
pub const ARMY: usize = 0x10;
/// CCountryExperienceStatus army daily experience; old offsets.rs:267, old read width i32 unproved as native.
pub const ARMY_DAILY: usize = 0x18;
/// CCountryExperienceStatus naval experience; old offsets.rs:268, old read width i32 unproved as native.
pub const NAVY: usize = 0x28;
/// CCountryExperienceStatus naval daily experience; old offsets.rs:269, old read width i32 unproved as native.
pub const NAVY_DAILY: usize = 0x30;
/// CCountryExperienceStatus air experience; old offsets.rs:270, old read width i32 unproved as native.
pub const AIR: usize = 0x40;
/// CCountryExperienceStatus air daily experience; old offsets.rs:271, old read width i32 unproved as native.
pub const AIR_DAILY: usize = 0x48;
/// CCountryExperienceStatus training-army channel; old offsets.rs:272 treated it as i32 fixed point, unproved natively.
pub const ARMIES_FOR_TRAINING: usize = 0x50;
/// CCountryExperienceStatus old scale declaration; offsets.rs:273, retained as unverified interpretation only.
pub const REPORTED_SCALE: i64 = 32_768;
