//! Child allocation object reached through CStateGarrisonData's child vector.
//! Its native class and RTTI are not established by the old alert reader.

/// Garrison child embedded equipment allocation pool; old a729_garrison_alert.rs:307-315 reads its 24-byte archetype/amount rows.
pub const EQUIPMENT_POOL: usize = 0x38;
